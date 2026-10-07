<script lang="ts">
  /* ==========================================================================
     Canvas -- the SVG diagram surface. Svelte reimplementation of
     `src/components/Canvas.tsx` (6295 lines) for the desktop migration (see
     MIGRATION_PLAN.md §9 item 3, "Canvas interaction fidelity").

     This is NOT a line-for-line transliteration of the React version. The
     React file's pointer-capture/synthetic-click-suppression machinery
     (documented in `AGENTS.md`'s "Canvas" section: capture-on-pointerdown
     had to be rebuilt once already because it swallowed the browser's own
     synthesized click) exists specifically to work around React's synthetic
     event system and its interaction with `setPointerCapture`. Svelte has no
     synthetic event layer -- `onpointerdown`/`onpointermove`/`onpointerup`
     here are the browser's own events -- so that whole class of workaround
     does not apply and is not reproduced. What IS kept, because it is
     ordinary correct web-platform practice rather than a React-specific
     hack:

       - `data-hit` + `data-id` hit routing, resolved once per gesture via
         `closest('[data-hit]')` (AGENTS.md: "Hit routing is data-hit +
         data-id resolved in one hitTest").
       - `setPointerCapture` only at gesture PROMOTION (after the drag
         threshold is crossed), wrapped in try/catch.
       - Floating chrome (the note/section text editors, and Minimap.svelte)
         rendered as DOM SIBLINGS of `.cv-surface`, never descendants, so a
         press on them can never reach the surface's pointer router at all --
         not even a `closest()` exclusion is needed structurally, though one
         is kept as belt-and-braces.
       - The PENDING -> ACTIVE promotion state machine (a press is a click
         until it has travelled `dragThresholdFor(pointerType)` screen px,
         at which point it promotes to whatever drag its hit target implies)
         from `pointer-input.ts`.

     SELECTION MODEL. `$lib/state/topology.svelte.ts` now holds
     `selectedIds: SvelteSet<string>` -- node ids, edge ids and annotation
     ids together in one set, matching the web app's `ReadonlySet<string>
     selectedIds` (`src/components/Canvas.tsx`). This is the second
     revision of this file: an earlier version was written against that
     store's prior shape (one selected node id XOR one selected edge id,
     with annotation selection tracked locally because the store had no
     home for it at all) and explicitly could not implement multi-select,
     marquee-select, Ctrl+A or clipboard copy/paste as a result. That
     restriction is gone -- the store was extended for this pass -- so all
     four are implemented here: `selectOne`/`setSelection`/`selectAll`/
     `clearSelection` (topology.svelte.ts) drive the selection, a
     shift/ctrl-drag on empty background starts a marquee
     (`'marquee'` pending-gesture mode below, resolved by rect-intersection
     against nodes/notes and rect-CONTAINMENT against sections, mirroring
     `Canvas.tsx`'s own asymmetry there), native `copy`/`cut`/`paste`
     document events drive the system clipboard via
     `$lib/domain/clipboard.ts` (a new port of `src/clipboard.ts`), and a
     double-click on a node opens a floating rename `<input>` modelled
     exactly on the pre-existing section-label editor below.

     GROUP DRAG, ZOOM-TO-FIT, LABEL DE-CONFLICTION, RIGHT-CLICK. The four
     canvas features of this pass, each ported from `Canvas.tsx`: dragging
     one member of an ALREADY-SELECTED multi-selection now carries the
     whole selection plus every selected annotation
     (`groupDragTargets`/`sharedSnappedDelta` in pointer-input.ts and the
     `groupDelta`/`groupOrigins` state below); the zoom cluster gained a
     fit button, and the % badge became one, both wired to the same `fitTo`
     the Shift+1 / Shift+2 chords call; coincident edge rate labels are
     bucketed and staggered by `labelDyById` (edge-route.ts); and a right
     press on a node selects it so the Inspector -- the app's side menu --
     opens on it.

     See this file's closing comment block for the full, itemised list of
     what is still simplified relative to `Canvas.tsx` / `annotationLayout.ts`
     (alt-drag-duplicate and Ctrl+D remain out of scope; undo/redo lives in
     `$lib/state/history.svelte` + `routes/+page.svelte`, with THIS file
     bracketing pointer gestures and committing each discrete topology edit
     it performs).
     ========================================================================== */

  import { onMount } from 'svelte';
  import {
    topologyStore,
    addNode,
    addEdge,
    moveNode,
    removeNode,
    removeEdge,
    removeSelection,
    renameNode,
    selectOne,
    setSelection,
    selectAll,
    clearSelection,
  } from '$lib/state/topology.svelte';
  import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';
  import { simulationStore } from '$lib/state/simulation.svelte';
  import { settingsStore } from '$lib/state/settings.svelte';
  import { simSetTopology } from '$lib/api/sim';
  import { isAppError } from '$lib/api';
  import { pushError } from '$lib/state/ui.svelte';
  import type {
    FailureKind,
    NodeKind,
    NodeStats,
    SimEdge,
    SimNode,
  } from '$lib/domain';
  import {
    isNote,
    isSection,
    makeNote as makeAnnNote,
    makeSection as makeAnnSection,
    SECTION_TONE_COUNT,
    SECTION_MIN_WIDTH,
    SECTION_MIN_HEIGHT,
  } from '$lib/domain/annotations';
  import type { Annotation, Note, Section } from '$lib/domain/annotations';
  import {
    buildClipboardText,
    parseClipboardText,
    cloneSubgraph,
  } from '$lib/domain/clipboard';

  import Minimap from './Minimap.svelte';

  import {
    // geometry / constants
    NODE_W,
    NODE_H,
    NODE_R,
    RING_GAP,
    HEAD_H,
    PAD_X,
    SEC_HALF,
    SPARK_X,
    SPARK_Y,
    SPARK_W,
    SPARK_H,
    SPARK_LEN,
    STRIP_Y,
    STRIP_H,
    METER_Y,
    METER_H,
    METER_W,
    WARN_AT,
    GRID,
    PORT_R,
    PORT_HIT_R,
    PORT_CY,
    MIN_ZOOM,
    MAX_ZOOM,
    ZOOM_STEP,
    DETAIL_ZOOM,
    MINIMAL_ZOOM,
    GLYPH_PX,
    GLYPH_INK_CENTER,
    HEAD_CENTER_Y,
    NAME_GAP,
    MARK_SIZE,
    MARK_RESERVE,
    clamp,
    snapTo,
    // zoom-to-fit
    fitViewTo,
    makeNode,
    newId,
    // visuals
    KIND_ICON,
    KIND_NAME,
    NODE_DND_MIME,
    stackLayers,
    stackBadge,
    cellStrip,
    // readouts
    readoutFor,
    sourceBacklogs,
    // edge routing
    routeEdge,
    arrowPath,
    previewPath,
    labelDyById,
  } from './geometry';
  import type { Readout } from './geometry';
  import { formatRate, healthOfLoad, toneClass } from './format';
  import type { Health } from './format';
  import { measureText, truncateToWidth, resetTextMetrics } from './text-metrics';
  import type { TextStyle } from './text-metrics';
  import {
    ANN_DND_MIME,
    NEW_NOTE_TEXT,
    NEW_SECTION_W,
    NEW_SECTION_H,
    RESIZE_DIRS,
    handleAnchor,
    layoutNote,
    resizeRect,
  } from './annotation-layout';
  import type { ResizeDir } from './annotation-layout';
  import { NOTE_MIN_WIDTH, NOTE_MAX_WIDTH } from '$lib/domain/annotations';
  import {
    dragThresholdFor,
    pressAction,
    isPalmTouch,
    beginPinch,
    pinchFrame,
    endPointer,
    groupDragTargets,
    sharedSnappedDelta,
  } from './pointer-input';
  import type { PinchState, TouchMap } from './pointer-input';

  import './Canvas.css';

  /* ------------------------------------------------------------------ *
   * Local layout constants not exported by geometry.ts (chrome, not the
   * shared node/edge geometry other panels need).
   * ------------------------------------------------------------------ */

  const VAL_STYLE: TextStyle = { size: 12, weight: 650, family: 'mono' };
  const VAL_PRIMARY_STYLE: TextStyle = { size: 16, weight: 650, family: 'mono' };
  const VAL_SELECTED_STYLE: TextStyle = { size: 22, weight: 650, family: 'mono' };
  const CAP_STYLE: TextStyle = {
    size: 11,
    weight: 650,
    family: 'sans',
    tracking: 0.66,
    uppercase: true,
  };
  const NAME_STYLE: TextStyle = { size: 14, weight: 450, family: 'sans' };
  const BADGE_STYLE: TextStyle = { size: 12, weight: 650, family: 'mono' };
  const SEC_LABEL_STYLE: TextStyle = { size: 12, weight: 550, family: 'sans' };
  const CELL_GAP_W = 3;
  const GLYPH_Y = HEAD_CENTER_Y - GLYPH_INK_CENTER * (GLYPH_PX / 24);
  const GLYPH_SCALE = GLYPH_PX / 24;
  const SEC_LABEL_PAD_X = 10;
  const SEC_LABEL_H = 24;
  const SEC_LABEL_MIN_W = 28;

  /* ------------------------------------------------------------------ *
   * View transform, surface size, gesture state.
   * ------------------------------------------------------------------ */

  interface View {
    x: number;
    y: number;
    k: number;
  }

  let view = $state<View>({ x: 40, y: 40, k: 1 });
  let surfaceW = $state(0);
  let surfaceH = $state(0);
  let hostEl: HTMLDivElement | null = $state(null);
  let surfaceEl: HTMLDivElement | null = $state(null);

  /** Armed annotation tool: 'n' arms note, 'b' arms section, matching the web app. */
  let tool = $state<'note' | 'section' | null>(null);
  /** Two-click wiring: an output port was clicked once, waiting for a target click. */
  let pendingLinkFrom = $state<string | null>(null);
  /**
   * Reactive mirror of `pending?.mode === 'pan'`, for the `grab`/`grabbing`
   * cursor class. `pending` itself is a plain (non-`$state`) object -- see
   * its declaration below -- so the template cannot read it reactively.
   */
  let isPanning = $state(false);

  /** Live overlays for an in-progress drag -- committed to the store on release. */
  let dragOverlay = $state<{ id: string; x: number; y: number } | null>(null);
  /**
   * The other members of an in-progress NODE drag: where each started, and
   * the shared delta every one of them translates by.
   *
   * The grabbed node is in NEITHER -- it keeps using `dragOverlay`, which
   * the render path already special-cases. `groupDelta` is `$state` because
   * `nodePos`/`sectionRect`/`notePos` read it while drawing; `groupOrigins`
   * is a plain Map because only the drag-end commit reads it, and every
   * read path checks `groupDelta` first. Both are cleared the same places
   * `dragOverlay` is.
   */
  let groupOrigins: Map<string, { x: number; y: number }> | null = null;
  let groupDelta = $state<{ dx: number; dy: number } | null>(null);
  let linkPreview = $state<{ from: string; x: number; y: number; over: string | null } | null>(
    null,
  );
  let annMoveOverlay = $state<{ id: string; dx: number; dy: number } | null>(null);
  let annResizeOverlay = $state<{
    id: string;
    rect: { x: number; y: number; w: number; h: number };
  } | null>(null);
  let noteResizeOverlay = $state<{ id: string; x: number; width: number } | null>(null);
  let drawSectionOverlay = $state<{ x: number; y: number; w: number; h: number } | null>(
    null,
  );
  /**
   * Marquee-select, in progress. Resolved into a selection on release by
   * rect-intersection against nodes/notes and rect-containment against
   * sections -- see `finishDrag`'s `'marquee'` case.
   */
  let marqueeOverlay = $state<{ x: number; y: number; w: number; h: number } | null>(null);

  /** Inline editors, floating HTML chrome over the SVG. */
  let noteEditor = $state<{ id: string; draft: string } | null>(null);
  let sectionLabelEditor = $state<{ id: string; draft: string } | null>(null);
  /** Double-click-a-node rename editor; same contract as `sectionLabelEditor`. */
  let nodeRenameEditor = $state<{ id: string; draft: string } | null>(null);
  let noteEditorEl: HTMLTextAreaElement | null = $state(null);
  let sectionLabelEditorEl: HTMLInputElement | null = $state(null);
  let nodeRenameEditorEl: HTMLInputElement | null = $state(null);

  type ResizeHandleDir = ResizeDir;

  interface PendingGesture {
    pointerId: number;
    hitKind: HitKind;
    hitId: string | null;
    hitDir: string | null;
    hitTone: string | null;
    threshold: number;
    screenX: number;
    screenY: number;
    worldX: number;
    worldY: number;
    vx: number;
    vy: number;
    tool: 'note' | 'section' | null;
    /** Modifier keys held at pointerdown. Drives additive click-select and
     *  marquee-vs-pan on empty background; captured once here rather than
     *  re-read at pointerup/move, matching `Canvas.tsx`'s own `p.shift`/
     *  `p.ctrl` (so releasing a modifier mid-gesture does not change its
     *  outcome). */
    shift: boolean;
    ctrl: boolean;
    active: boolean;
    mode:
      | 'pan'
      | 'node'
      | 'link'
      | 'ann'
      | 'ann-resize'
      | 'note-resize'
      | 'draw-section'
      | 'marquee'
      | null;
    grabDx: number;
    grabDy: number;
    annDir: ResizeHandleDir | null;
    annOrigin: { x: number; y: number; w: number; h: number } | null;
  }

  /**
   * Plain (non-`$state`) mutable gesture record, deliberately outside
   * Svelte's reactivity: it is written on every `pointermove` of an active
   * drag, and wrapping that in `$state` would re-run every `$derived`/
   * template read that touches it on every pixel of mouse movement. The
   * handful of fields the template DOES need to see live (`dragOverlay`,
   * `linkPreview`, `isPanning`, ...) are their own `$state` variables,
   * updated explicitly where they change.
   */
  let pending: PendingGesture | null = null;
  let penIsDown = false;
  const touches: TouchMap = new Map();
  let pinch: PinchState | null = null;
  /**
   * Last pointer position in CLIENT coords, remembered for Ctrl+V: the
   * viewport can pan and zoom between the last move and the paste, and
   * converting to world coords at paste time (`toWorld`, in the paste
   * handler) rather than storing world coords here stays honest through
   * both. Plain, not `$state`: nothing reads it reactively.
   */
  let lastClient: { x: number; y: number } | null = null;

  /* ------------------------------------------------------------------ *
   * Derived read models.
   * ------------------------------------------------------------------ */

  const topology = $derived(topologyStore.topology);
  const nodes = $derived(topology.nodes);
  const edges = $derived(topology.edges);
  const annotations = $derived<Annotation[]>(topology.annotations ?? []);
  const sections = $derived(annotations.filter(isSection));
  const notesList = $derived(annotations.filter(isNote));

  const nodeById = $derived(new Map(nodes.map((n) => [n.id, n] as const)));
  const annotationById = $derived(new Map(annotations.map((a) => [a.id, a] as const)));

  /**
   * The one selected id, or null for an empty or multi-selection. Field-for-
   * field port of the web app's own single-selection derivation (used e.g.
   * by its note/section format toolbar): "null for a multi-selection on
   * purpose... with two things selected any single answer would be wrong
   * about one of them" (`Canvas.tsx`). Here it gates the annotation
   * selection chrome (resize handles etc.), which only ever makes sense for
   * exactly one annotation at a time.
   */
  const singleSelectedId = $derived.by(() => {
    if (topologyStore.selectedIds.size !== 1) return null;
    const [id] = topologyStore.selectedIds;
    return id ?? null;
  });
  const selectedAnnotation = $derived(
    singleSelectedId ? annotationById.get(singleSelectedId) ?? null : null,
  );

  const snapshot = $derived(simulationStore.snapshot);
  const statsById = $derived(snapshot?.nodes ?? ({} as Record<string, NodeStats>));
  const backlogs = $derived(sourceBacklogs(topology, statsById));
  const faultByNode = $derived.by(() => {
    const m = new Map<string, FailureKind>();
    for (const f of snapshot?.activeFailures ?? []) m.set(f.nodeId, f.kind);
    return m;
  });

  const detail = $derived<0 | 1 | 2>(
    view.k < MINIMAL_ZOOM ? 0 : view.k < DETAIL_ZOOM ? 1 : 2,
  );
  /**
   * Rate labels appear at 1x and above -- `Canvas.tsx`'s own
   * `const showEdgeLabels = view.k >= 1`, deliberately NOT the node detail
   * budget (`detail === 2`, which starts at 0.7x): an 11px label needs the
   * zoom to read it, while the numbers inside a node are sized by the node.
   */
  const showEdgeLabels = $derived(view.k >= 1);

  /** Per-node sparkline ring buffers, sampled once per incoming snapshot. */
  let sparkHistory = $state<Map<string, number[]>>(new Map());

  $effect(() => {
    const snap = simulationStore.snapshot;
    if (!snap || !settingsStore.sparklines) return;
    const liveIds = new Set(nodes.map((n) => n.id));
    const next = new Map(sparkHistory);
    for (const id of [...next.keys()]) if (!liveIds.has(id)) next.delete(id);
    for (const n of nodes) {
      const stats = snap.nodes[n.id];
      if (!stats) continue;
      const readout = readoutFor(n.kind, stats, n.config, backlogs.get(n.id) ?? 0);
      const arr = (next.get(n.id) ?? []).slice();
      arr.push(readout.spark);
      if (arr.length > SPARK_LEN) arr.splice(0, arr.length - SPARK_LEN);
      next.set(n.id, arr);
    }
    sparkHistory = next;
  });

  /* Drop cached text measurements once the real font stack has settled; the
     10Hz snapshot loop naturally re-renders labels within ~100ms after, so
     no extra reactive plumbing is needed to pick the corrected widths up. */
  onMount(() => {
    if (typeof document === 'undefined' || !document.fonts?.ready) return;
    let live = true;
    document.fonts.ready.then(() => {
      if (live) resetTextMetrics();
    });
    return () => {
      live = false;
    };
  });

  /* ------------------------------------------------------------------ *
   * Coordinate conversion + zoom.
   * ------------------------------------------------------------------ */

  function toWorld(clientX: number, clientY: number): { x: number; y: number } {
    if (!surfaceEl) return { x: 0, y: 0 };
    const r = surfaceEl.getBoundingClientRect();
    return { x: (clientX - r.left - view.x) / view.k, y: (clientY - r.top - view.y) / view.k };
  }

  function zoomAt(px: number, py: number, next: (k: number) => number): void {
    const k = clamp(Math.round(next(view.k) * 1000) / 1000, MIN_ZOOM, MAX_ZOOM);
    if (k === view.k) return;
    const wx = (px - view.x) / view.k;
    const wy = (py - view.y) / view.k;
    view = { k, x: px - wx * k, y: py - wy * k };
  }

  /** Chrome-button zoom: scale about the viewport centre, like the web
   *  app's `zoomCentered` -- the diagram grows where the eye already is. */
  function zoomBy(factor: number): void {
    const el = surfaceEl;
    if (!el) return;
    const r = el.getBoundingClientRect();
    zoomAt(r.width / 2, r.height / 2, (k) => k * factor);
  }

  function gotoWorld(wx: number, wy: number): void {
    view = { ...view, x: surfaceW / 2 - wx * view.k, y: surfaceH / 2 - wy * view.k };
  }

  function snapIf(v: number): number {
    return settingsStore.snapToGrid ? snapTo(v, GRID) : Math.round(v);
  }

  /**
   * The `.stage-safe` sentinel's rect, clamped into the surface, or the
   * surface itself when there is none.
   *
   * shell.css documents that invisible div as the UNCOVERED area (the part
   * of the stage no library rail / inspector / metrics strip covers) and
   * that the canvas measures it whenever it aims the camera -- but
   * `+page.svelte` does not render it yet, and in a headless layout every
   * rect is 0x0. Both cases fall back to the whole surface, which is also
   * the all-panels-closed answer, so a fit never aims outside the element
   * the transform is anchored to.
   */
  function visibleFrame(surface: { x: number; y: number; w: number; h: number }): {
    x: number;
    y: number;
    w: number;
    h: number;
  } {
    const sentinel = typeof document === 'undefined' ? null : document.querySelector('.stage-safe');
    const r = sentinel?.getBoundingClientRect();
    if (!r || r.width === 0 || r.height === 0) return surface;
    const x = Math.max(r.left, surface.x);
    const y = Math.max(r.top, surface.y);
    const w = Math.min(r.right, surface.x + surface.w) - x;
    const h = Math.min(r.bottom, surface.y + surface.h) - y;
    return w <= 0 || h <= 0 ? surface : { x, y, w, h };
  }

  /**
   * Fit a set of nodes into the viewport: the whole diagram for Shift+1 and
   * the cluster's fit button, just the selection for Shift+2.
   *
   * The scale and the centring both use `visibleFrame`, so a fit with
   * panels open frames the diagram inside the uncovered area; the returned
   * offset is surface-relative, which is what the pan/zoom transform reads.
   * The arithmetic is `geometry.ts`'s `fitViewTo` (a port of `Canvas.tsx`'s
   * `fitTo`) so it can be unit tested without a DOM. Null means an empty
   * diagram or an unmeasured viewport -- either way the camera stays where
   * the student put it.
   */
  function fitTo(target: readonly SimNode[]): void {
    const el = surfaceEl;
    if (!el || target.length === 0) return;
    const box = el.getBoundingClientRect();
    const surface = { x: box.x, y: box.y, w: box.width, h: box.height };
    const next = fitViewTo(
      target.map((n) => ({ x: n.x, y: n.y, w: NODE_W, h: NODE_H })),
      surface,
      visibleFrame(surface),
    );
    if (!next) return;
    view = { k: next.k, x: next.x, y: next.y };
  }

  /** Shift+1 and the cluster's fit button. */
  function fitToContent(): void {
    fitTo(nodes);
  }

  onMount(() => {
    const el = surfaceEl;
    if (!el) return;
    function onWheel(e: WheelEvent): void {
      const r = el!.getBoundingClientRect();
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        zoomAt(e.clientX - r.left, e.clientY - r.top, (k) => k * Math.exp(-e.deltaY * 0.0022));
        return;
      }
      e.preventDefault();
      view = { ...view, x: view.x - e.deltaX, y: view.y - e.deltaY };
    }
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  });

  /* ------------------------------------------------------------------ *
   * Annotation mutation helpers.
   *
   * `topology.svelte.ts` (finished, not to be modified) has no annotation
   * mutators -- it was written before `$lib/domain/annotations.ts` existed.
   * These helpers mirror that module's OWN pattern exactly: optimistic
   * local update to `topologyStore.topology`, then a fire-and-forget
   * `simSetTopology` sync routed into `pushError` on failure, never rolled
   * back locally (the next full sync reconciles it). Reading the pattern
   * from `topology.svelte.ts` rather than inventing a new one is
   * deliberate, so a later agent that DOES get to extend that module can
   * lift these functions in verbatim.
   * ------------------------------------------------------------------ */

  function describeErr(e: unknown): string {
    if (isAppError(e)) return e.message;
    return e instanceof Error ? e.message : String(e);
  }

  function commitAnnotations(next: Annotation[]): void {
    const nextTopology = { ...topologyStore.topology, annotations: next };
    topologyStore.topology = nextTopology;
    simSetTopology(nextTopology).catch((e) =>
      pushError(`Updating annotations failed to reach the simulation engine: ${describeErr(e)}`),
    );
  }

  /**
   * Drag-end commit for a group of nodes: ONE topology edit and ONE
   * `simSetTopology`.
   *
   * The solo case keeps using `moveNode` (already a whole-topology push),
   * but calling it N times in a row for an N-member selection would push
   * the topology N times back to back with a different number of moved
   * nodes each time. This mirrors `commitAnnotations`' own pattern --
   * optimistic local update, then fire-and-forget sync into `pushError` --
   * so the engine is never sent a half-moved selection.
   */
  function commitNodeMoves(moves: readonly { id: string; x: number; y: number }[]): void {
    if (moves.length === 0) return;
    const byId = new Map(moves.map((m) => [m.id, m]));
    const nextTopology = {
      ...topologyStore.topology,
      nodes: topologyStore.topology.nodes.map((n) => {
        const m = byId.get(n.id);
        return m ? { ...n, x: m.x, y: m.y } : n;
      }),
    };
    topologyStore.topology = nextTopology;
    simSetTopology(nextTopology).catch((e) =>
      pushError(
        `Moving the selection failed to reach the simulation engine: ${describeErr(e)}`,
      ),
    );
  }

  function createNote(x: number, y: number): string {
    const note = makeAnnNote(snapIf(x), snapIf(y), NEW_NOTE_TEXT);
    // Pre-write baseline, so one Ctrl+Z drops the note back off the canvas.
    // App.tsx commits 'add note' (:1300) in the same before/after order.
    sessionHistory.commit('add note', currentSnapshot());
    commitAnnotations([...annotations, note]);
    return note.id;
  }

  function createSection(x: number, y: number, w: number, h: number): string {
    const section = makeAnnSection(snapIf(x), snapIf(y), Math.round(w), Math.round(h));
    // Same before/after order as App.tsx's 'add section' commit (:1323);
    // every path that makes a section (tool click, section draw, palette
    // drop) funnels through here, so each is exactly one entry.
    sessionHistory.commit('add section', currentSnapshot());
    commitAnnotations([...annotations, section]);
    return section.id;
  }

  function deleteAnnotation(id: string): void {
    // Missing id = nothing to drop; committing anyway would raise a no-op
    // entry that still clears redo (the blanked-note editor can re-fire on
    // an already-gone note).
    if (!annotationById.has(id)) return;
    // ONE entry for the removal, baseline first -- App.tsx's 'delete'
    // commit (:1587) guards and orders the same way.
    sessionHistory.commit('delete', currentSnapshot());
    commitAnnotations(annotations.filter((a) => a.id !== id));
  }

  function commitAnnotationMove(id: string, dx: number, dy: number): void {
    if (dx === 0 && dy === 0) return;
    commitAnnotations(
      annotations.map((a) =>
        a.id === id ? { ...a, x: snapIf(a.x + dx), y: snapIf(a.y + dy) } : a,
      ),
    );
  }

  function commitSectionResize(
    id: string,
    rect: { x: number; y: number; w: number; h: number },
  ): void {
    commitAnnotations(
      annotations.map((a) =>
        a.id === id && isSection(a)
          ? { ...a, x: rect.x, y: rect.y, width: rect.w, height: rect.h }
          : a,
      ),
    );
  }

  function commitNoteResize(id: string, x: number, width: number): void {
    commitAnnotations(
      annotations.map((a) =>
        a.id === id && isNote(a)
          ? { ...a, x, width: clamp(width, NOTE_MIN_WIDTH, NOTE_MAX_WIDTH) }
          : a,
      ),
    );
  }

  function commitNoteText(id: string, text: string): void {
    if (!text.trim()) {
      deleteAnnotation(id);
      return;
    }
    const next = text.slice(0, 2000);
    const target = annotationById.get(id);
    // Closing the editor with the text unchanged must raise NO entry --
    // a phantom commit would clear redo for nothing. App.tsx's
    // handleEditNote guards identically before its 'note edit' commit
    // (:1363), which also sits after every guard and before the write.
    if (!target || !isNote(target) || target.text === next) return;
    sessionHistory.commit('note edit', currentSnapshot());
    commitAnnotations(annotations.map((a) => (a.id === id && isNote(a) ? { ...a, text: next } : a)));
  }

  function commitSectionLabel(id: string, label: string): void {
    const next = label.slice(0, 200);
    const target = annotationById.get(id);
    // Same no-op guard as the note editor above; web commits 'label edit'
    // at :1376 in the same guarded position.
    if (!target || !isSection(target) || target.label === next) return;
    sessionHistory.commit('label edit', currentSnapshot());
    commitAnnotations(annotations.map((a) => (a.id === id && isSection(a) ? { ...a, label: next } : a)));
  }

  function cycleSectionTone(id: string): void {
    // A shade flip is one discrete entry, like App.tsx's 'section shade'
    // commit (:1440).
    sessionHistory.commit('section shade', currentSnapshot());
    commitAnnotations(
      annotations.map((a) =>
        a.id === id && isSection(a) ? { ...a, tone: (a.tone + 1) % SECTION_TONE_COUNT } : a,
      ),
    );
  }

  function openNoteEditor(id: string): void {
    const n = annotationById.get(id);
    noteEditor = { id, draft: n && isNote(n) ? n.text : '' };
    queueMicrotask(() => noteEditorEl?.focus());
  }

  function commitNoteEditor(): void {
    if (!noteEditor) return;
    commitNoteText(noteEditor.id, noteEditor.draft);
    noteEditor = null;
  }

  function openSectionLabelEditor(id: string): void {
    const s = annotationById.get(id);
    sectionLabelEditor = { id, draft: s && isSection(s) ? s.label : '' };
    queueMicrotask(() => sectionLabelEditorEl?.focus());
  }

  function commitSectionLabelEditor(): void {
    if (!sectionLabelEditor) return;
    commitSectionLabel(sectionLabelEditor.id, sectionLabelEditor.draft);
    sectionLabelEditor = null;
  }

  /**
   * Double-click-a-node rename. Same contract as the section label editor
   * above and `Canvas.tsx`'s own `commitRename`/`cancelRename`: Enter or
   * blur commits, Escape cancels, and an empty or unchanged name reverts
   * silently rather than writing a blank label -- a node must always have a
   * name, and clearing the box is not "delete the label".
   */
  function openNodeRenameEditor(id: string): void {
    const n = nodeById.get(id);
    if (!n) return;
    nodeRenameEditor = { id, draft: n.label };
    queueMicrotask(() => nodeRenameEditorEl?.focus());
  }

  function commitNodeRenameEditor(): void {
    if (!nodeRenameEditor) return;
    const { id, draft } = nodeRenameEditor;
    nodeRenameEditor = null;
    const node = nodeById.get(id);
    const label = draft.trim();
    if (!node || label === '' || label === node.label) return;
    // Web fires its rename stream through touch('rename') (:1795) and
    // debounces it into one typed name; this editor writes once on
    // Enter/blur, so the same touch still lands exactly one 'rename'
    // entry (flushed by the next commit or by undo itself).
    sessionHistory.touch('rename', currentSnapshot());
    renameNode(id, label);
  }

  /* ------------------------------------------------------------------ *
   * Hit testing.
   * ------------------------------------------------------------------ */

  type HitKind =
    | 'node'
    | 'port-in'
    | 'port-out'
    | 'edge'
    | 'edge-delete'
    | 'background'
    | 'note'
    | 'section'
    | 'section-resize'
    | 'note-resize'
    | 'section-tone';

  interface Hit {
    kind: HitKind;
    id: string | null;
    dir: string | null;
    tone: string | null;
  }

  const BACKGROUND_HIT: Hit = { kind: 'background', id: null, dir: null, tone: null };

  function hitTest(target: EventTarget | null): Hit {
    const el = target instanceof Element ? target : null;
    const found = el?.closest('[data-hit]');
    if (!found) return BACKGROUND_HIT;
    return {
      kind: (found.getAttribute('data-hit') as HitKind | null) ?? 'background',
      id: found.getAttribute('data-id'),
      dir: found.getAttribute('data-dir'),
      tone: found.getAttribute('data-tone'),
    };
  }

  function isChrome(target: EventTarget | null): boolean {
    const el = target instanceof Element ? target : null;
    return Boolean(el?.closest('button, input, select, textarea, a, [data-chrome]'));
  }

  function canLink(from: string, to: string): boolean {
    if (from === to) return false;
    return !edges.some((e) => e.from === from && e.to === to);
  }

  /* ------------------------------------------------------------------ *
   * Pointer gesture state machine.
   * ------------------------------------------------------------------ */

  function onSurfacePointerDown(e: PointerEvent): void {
    if (isChrome(e.target)) return;
    // RIGHT MOUSE PRESS = select the node under the cursor so the Inspector
    // opens on it: the Inspector IS this app's side menu (+page.svelte keys
    // its `has-inspector` class off `selectedNodeId`). Gated on `mouse` so a
    // pen's barrel button (also button 2) stays inert, exactly as
    // pressAction's own doc comment promises. No gesture is started, and a
    // node already in the selection is left alone so a right-press can
    // never collapse a multi-selection. Background right-press changes no
    // selection at all.
    if (e.pointerType === 'mouse' && e.button === 2) {
      const hit = hitTest(e.target);
      if (hit.kind === 'node' && hit.id && !topologyStore.selectedIds.has(hit.id)) {
        selectOne(hit.id, false);
      }
      return;
    }
    const action = pressAction(e.button, e.pointerType);
    if (action === 'none') return;

    if (e.pointerType === 'touch') {
      if (isPalmTouch(e.pointerType, e.width, e.height, penIsDown)) return;
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (touches.size >= 2) {
        // A second finger takes the surface over: close any undo gesture the
        // first had going (it dropped its topology on release anyway, so the
        // baseline normally discards itself -- what matters is that no
        // baseline is left dangling past this point).
        pending = null;
        closeUndoGesture();
        pinch = beginPinch(touches, view.k);
        return;
      }
    }
    if (e.pointerType === 'pen') penIsDown = true;

    const hit = action === 'pan' ? BACKGROUND_HIT : hitTest(e.target);
    const world = toWorld(e.clientX, e.clientY);

    pending = {
      pointerId: e.pointerId,
      hitKind: hit.kind,
      hitId: hit.id,
      hitDir: hit.dir,
      hitTone: hit.tone,
      threshold: dragThresholdFor(e.pointerType),
      screenX: e.clientX,
      screenY: e.clientY,
      worldX: world.x,
      worldY: world.y,
      vx: view.x,
      vy: view.y,
      tool,
      shift: e.shiftKey,
      ctrl: e.ctrlKey || e.metaKey,
      active: false,
      mode: action === 'pan' ? 'pan' : null,
      grabDx: 0,
      grabDy: 0,
      annDir: null,
      annOrigin: null,
    };
  }

  /**
   * Bracket the pointer gesture's undo entry. Called on EVERY way a gesture
   * can end -- pointerup, pointercancel, Escape (cancelGesture) and a pinch
   * taking the surface over mid-drag -- so a `beginGesture` baseline can
   * never outlive its gesture and silently swallow the next one. A gesture
   * that changed nothing drops out of `endGesture` with nothing pushed, so
   * a plain click costs no undo step. This is the desktop's port of
   * `Canvas.tsx`'s `onMoveEnd`, the single funnel all of those exits used.
   */
  function closeUndoGesture(): void {
    if (sessionHistory.inGesture) sessionHistory.endGesture(currentSnapshot());
  }

  function promote(p: PendingGesture): void {
    try {
      surfaceEl?.setPointerCapture(p.pointerId);
    } catch {
      // Capture is a convenience; the gesture still works without it.
    }
    switch (p.hitKind) {
      case 'node': {
        const node = p.hitId ? nodeById.get(p.hitId) : undefined;
        if (!node) {
          p.mode = 'pan';
          break;
        }
        p.mode = 'node';
        // Baseline FIRST, before the selectOne below: one 'move' entry for
        // the whole drag, holding the PRE-drag selection too so undo puts
        // it back (Canvas.tsx fires onMoveStart at exactly this point --
        // see its comment at ~361).
        sessionHistory.beginGesture('move', currentSnapshot());
        p.grabDx = p.worldX - node.x;
        p.grabDy = p.worldY - node.y;
        dragOverlay = { id: node.id, x: node.x, y: node.y };
        // Press on an ALREADY-SELECTED member of a multi-selection drags
        // the WHOLE selection -- the other selected nodes plus every
        // selected annotation, so a marquee'd cluster moves as one object
        // whichever member was grabbed (`groupDragTargets`, ported from
        // `Canvas.tsx`'s own `sel.has(id) && sel.size > 1` test). Anything
        // else selects and moves only this node, exactly as a plain click
        // would.
        let origins: Map<string, { x: number; y: number }> | null = null;
        const group = groupDragTargets(
          node.id,
          topologyStore.selectedIds,
          nodes.map((n) => n.id),
          annotations.map((a) => a.id),
        );
        if (group) {
          const found = new Map<string, { x: number; y: number }>();
          for (const id of group.nodes) {
            const n = nodeById.get(id);
            if (n) found.set(id, { x: n.x, y: n.y });
          }
          for (const id of group.annotations) {
            const a = annotationById.get(id);
            if (a) found.set(id, { x: a.x, y: a.y });
          }
          origins = found.size > 0 ? found : null;
        }
        if (origins) {
          groupOrigins = origins;
          groupDelta = { dx: 0, dy: 0 };
        } else {
          selectOne(node.id, false);
        }
        break;
      }
      case 'port-out': {
        if (!p.hitId) {
          p.mode = 'pan';
          break;
        }
        p.mode = 'link';
        linkPreview = { from: p.hitId, x: p.worldX, y: p.worldY, over: null };
        break;
      }
      case 'section':
      case 'note': {
        const ann = p.hitId ? annotationById.get(p.hitId) : undefined;
        if (!ann) {
          p.mode = 'pan';
          break;
        }
        p.mode = 'ann';
        // Before selectOne: same pre-drag baseline as the node case
        // (Canvas.tsx's onMoveStart at ~3989).
        sessionHistory.beginGesture('move', currentSnapshot());
        annMoveOverlay = { id: ann.id, dx: 0, dy: 0 };
        selectOne(ann.id, false);
        break;
      }
      case 'section-resize': {
        const ann = p.hitId ? annotationById.get(p.hitId) : undefined;
        if (!ann || !isSection(ann) || !p.hitDir) {
          p.mode = 'pan';
          break;
        }
        p.mode = 'ann-resize';
        // Resizing is its own entry label, matching the 'resize' label the
        // web gives this same promote branch (Canvas.tsx ~4050).
        sessionHistory.beginGesture('resize', currentSnapshot());
        p.annDir = p.hitDir as ResizeHandleDir;
        p.annOrigin = { x: ann.x, y: ann.y, w: ann.width, h: ann.height };
        annResizeOverlay = { id: ann.id, rect: { ...p.annOrigin } };
        break;
      }
      case 'note-resize': {
        const ann = p.hitId ? annotationById.get(p.hitId) : undefined;
        if (!ann || !isNote(ann) || !p.hitDir) {
          p.mode = 'pan';
          break;
        }
        p.mode = 'note-resize';
        // Same 'resize' label the web gives this branch (Canvas.tsx ~4066).
        sessionHistory.beginGesture('resize', currentSnapshot());
        p.annDir = p.hitDir as ResizeHandleDir;
        noteResizeOverlay = { id: ann.id, x: ann.x, width: ann.width };
        break;
      }
      default: {
        if (p.mode === 'pan') break;
        if (p.tool === 'section') {
          p.mode = 'draw-section';
          drawSectionOverlay = { x: p.worldX, y: p.worldY, w: 0, h: 0 };
        } else if (p.shift || p.ctrl) {
          // Empty background: shift or ctrl starts a marquee, a plain drag
          // pans. Panning is the more common intent, so the marquee stays
          // one modifier away -- field-for-field port of `Canvas.tsx`'s own
          // `p.mode = p.shift || p.ctrl ? 'marquee' : 'pan'`.
          p.mode = 'marquee';
          marqueeOverlay = { x: p.worldX, y: p.worldY, w: 0, h: 0 };
        } else {
          p.mode = 'pan';
        }
      }
    }
    isPanning = p.mode === 'pan';
  }

  function applyDrag(p: PendingGesture, e: PointerEvent): void {
    const world = toWorld(e.clientX, e.clientY);
    switch (p.mode) {
      case 'pan':
        view = { ...view, x: p.vx + (e.clientX - p.screenX), y: p.vy + (e.clientY - p.screenY) };
        break;
      case 'node': {
        if (!dragOverlay) break;
        const nx = snapIf(world.x - p.grabDx);
        const ny = snapIf(world.y - p.grabDy);
        dragOverlay = { id: dragOverlay.id, x: nx, y: ny };
        // Every OTHER member of a promoted group translates by this same
        // delta. Both terms go through `snapIf` before the subtraction so
        // the grabbed node's snapped position and the members' shared delta
        // are rounded identically -- the selection keeps its exact shape on
        // the grid instead of each member snapping independently.
        if (groupOrigins && groupDelta) {
          groupDelta = {
            dx: sharedSnappedDelta(world.x - p.grabDx, p.worldX - p.grabDx, snapIf),
            dy: sharedSnappedDelta(world.y - p.grabDy, p.worldY - p.grabDy, snapIf),
          };
        }
        break;
      }
      case 'link': {
        if (!linkPreview) break;
        // `promote()` captured the pointer on the surface element, so once
        // the drag promotes, every move event is retargeted to the surface
        // and `hitTest(e.target)` can never resolve the node/port under the
        // cursor -- `over` would stay null and `finishDrag` would never
        // `addEdge`, breaking drag-to-connect entirely. Ask the browser for
        // the element actually under the pointer instead; the link preview
        // path is `pointer-events: none` (Canvas.css) so it never wins.
        const hit = hitTest(document.elementFromPoint(e.clientX, e.clientY));
        const overId =
          (hit.kind === 'node' || hit.kind === 'port-in') && hit.id && canLink(linkPreview.from, hit.id)
            ? hit.id
            : null;
        linkPreview = { from: linkPreview.from, x: world.x, y: world.y, over: overId };
        break;
      }
      case 'ann': {
        if (!annMoveOverlay) break;
        annMoveOverlay = {
          id: annMoveOverlay.id,
          dx: world.x - p.worldX,
          dy: world.y - p.worldY,
        };
        break;
      }
      case 'ann-resize': {
        if (!annResizeOverlay || !p.annOrigin || !p.annDir) break;
        const place = (v: number) => snapIf(v);
        const rect = resizeRect(
          p.annOrigin,
          p.annDir,
          world.x - p.worldX,
          world.y - p.worldY,
          place,
          SECTION_MIN_WIDTH,
          SECTION_MIN_HEIGHT,
        );
        annResizeOverlay = { id: annResizeOverlay.id, rect };
        break;
      }
      case 'note-resize': {
        if (!noteResizeOverlay || !p.annDir) break;
        const note = annotationById.get(noteResizeOverlay.id);
        if (!note || !isNote(note)) break;
        const dx = world.x - p.worldX;
        if (p.annDir.includes('e')) {
          const width = clamp(note.width + dx, NOTE_MIN_WIDTH, NOTE_MAX_WIDTH);
          noteResizeOverlay = { id: note.id, x: note.x, width };
        } else if (p.annDir.includes('w')) {
          const width = clamp(note.width - dx, NOTE_MIN_WIDTH, NOTE_MAX_WIDTH);
          noteResizeOverlay = { id: note.id, x: note.x + (note.width - width), width };
        }
        break;
      }
      case 'draw-section': {
        if (!drawSectionOverlay) break;
        drawSectionOverlay = {
          x: Math.min(p.worldX, world.x),
          y: Math.min(p.worldY, world.y),
          w: Math.abs(world.x - p.worldX),
          h: Math.abs(world.y - p.worldY),
        };
        break;
      }
      case 'marquee': {
        if (!marqueeOverlay) break;
        marqueeOverlay = {
          x: Math.min(p.worldX, world.x),
          y: Math.min(p.worldY, world.y),
          w: Math.abs(world.x - p.worldX),
          h: Math.abs(world.y - p.worldY),
        };
        break;
      }
    }
  }

  function onSurfacePointerMove(e: PointerEvent): void {
    // Remembered for Ctrl+V, which pastes at the pointer -- see the
    // `lastClient` declaration above for why this stays in client coords.
    lastClient = { x: e.clientX, y: e.clientY };
    if (e.pointerType === 'touch' && touches.has(e.pointerId)) {
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
    }
    if (pinch) {
      const frame = pinchFrame(pinch, touches);
      if (frame) {
        const targetK = clamp(Math.round(frame.k * 1000) / 1000, MIN_ZOOM, MAX_ZOOM);
        view = { k: targetK, x: view.x + frame.dx, y: view.y + frame.dy };
        pinch = { ...pinch, lastMid: frame.mid };
      }
      return;
    }
    if (!pending || pending.pointerId !== e.pointerId) return;
    if (!pending.active) {
      const dx = e.clientX - pending.screenX;
      const dy = e.clientY - pending.screenY;
      if (Math.hypot(dx, dy) < pending.threshold) return;
      pending.active = true;
      promote(pending);
    }
    applyDrag(pending, e);
  }

  function finishDrag(p: PendingGesture): void {
    isPanning = false;
    switch (p.mode) {
      case 'node': {
        const moved = dragOverlay;
        const origins = groupOrigins;
        const delta = groupDelta;
        if (moved && origins && delta) {
          const nodeMoves: { id: string; x: number; y: number }[] = [
            { id: moved.id, x: moved.x, y: moved.y },
          ];
          const annMoves = new Map<string, { x: number; y: number }>();
          for (const [id, o] of origins) {
            const to = { x: o.x + delta.dx, y: o.y + delta.dy };
            if (nodeById.has(id)) nodeMoves.push({ id, x: to.x, y: to.y });
            else annMoves.set(id, to);
          }
          // Annotations are committed FIRST: `commitAnnotations` rewrites
          // the topology wholesale, so the node batch has to be built from
          // the store AFTER that write or one would drop the other's field.
          if (annMoves.size > 0) {
            commitAnnotations(
              annotations.map((a) => {
                const m = annMoves.get(a.id);
                return m ? { ...a, x: m.x, y: m.y } : a;
              }),
            );
          }
          commitNodeMoves(nodeMoves);
        } else if (moved) {
          moveNode(moved.id, moved.x, moved.y);
        }
        groupOrigins = null;
        groupDelta = null;
        dragOverlay = null;
        break;
      }
      case 'link':
        if (linkPreview?.over) {
          // Discrete write outside any bracket (promote does not open one
          // for 'link'), so it commits its own entry -- App.tsx's
          // 'connection' commit at :1556.
          sessionHistory.commit('connection', currentSnapshot());
          addEdge({ id: newId('edge'), from: linkPreview.from, to: linkPreview.over, weight: 1 });
        }
        linkPreview = null;
        break;
      case 'ann':
        if (annMoveOverlay) commitAnnotationMove(annMoveOverlay.id, annMoveOverlay.dx, annMoveOverlay.dy);
        annMoveOverlay = null;
        break;
      case 'ann-resize':
        if (annResizeOverlay) commitSectionResize(annResizeOverlay.id, annResizeOverlay.rect);
        annResizeOverlay = null;
        break;
      case 'note-resize':
        if (noteResizeOverlay) commitNoteResize(noteResizeOverlay.id, noteResizeOverlay.x, noteResizeOverlay.width);
        noteResizeOverlay = null;
        break;
      case 'draw-section':
        if (drawSectionOverlay && drawSectionOverlay.w > 4 && drawSectionOverlay.h > 4) {
          const id = createSection(
            drawSectionOverlay.x,
            drawSectionOverlay.y,
            drawSectionOverlay.w,
            drawSectionOverlay.h,
          );
          selectOne(id, false);
        }
        drawSectionOverlay = null;
        tool = null;
        break;
      case 'marquee': {
        if (marqueeOverlay) {
          const x0 = marqueeOverlay.x;
          const y0 = marqueeOverlay.y;
          const x1 = marqueeOverlay.x + marqueeOverlay.w;
          const y1 = marqueeOverlay.y + marqueeOverlay.h;
          // Intersection for nodes and notes, CONTAINMENT for sections --
          // field-for-field port of `Canvas.tsx`'s own marquee-resolve: "a
          // box that clips a node selects it [intersection]... a section
          // frames the very nodes a marquee inside it sweeps up, so
          // intersection would make it impossible to box-select a section's
          // contents without also grabbing the frame itself. A marquee that
          // swallows the whole frame plainly means it [containment]."
          const next = new Set<string>(p.shift || p.ctrl ? topologyStore.selectedIds : []);
          for (const n of nodes) {
            if (n.x <= x1 && n.x + NODE_W >= x0 && n.y <= y1 && n.y + NODE_H >= y0) {
              next.add(n.id);
            }
          }
          for (const a of annotations) {
            if (isSection(a)) {
              if (a.x >= x0 && a.x + a.width <= x1 && a.y >= y0 && a.y + a.height <= y1) {
                next.add(a.id);
              }
            } else {
              const h = layoutNote(a.text, a.width, a.size, a.font, a.bold, a.italic, a.scale).height;
              if (a.x <= x1 && a.x + a.width >= x0 && a.y <= y1 && a.y + h >= y0) {
                next.add(a.id);
              }
            }
          }
          setSelection(next);
        }
        marqueeOverlay = null;
        break;
      }
    }
  }

  function handleClick(p: PendingGesture): void {
    const additive = p.shift || p.ctrl;
    switch (p.hitKind) {
      case 'node': {
        if (!p.hitId) break;
        if (pendingLinkFrom && canLink(pendingLinkFrom, p.hitId)) {
          // Same 'connection' entry as the drag-release link path above
          // (App.tsx :1556): a click-through connection is one Ctrl+Z step.
          sessionHistory.commit('connection', currentSnapshot());
          addEdge({ id: newId('edge'), from: pendingLinkFrom, to: p.hitId, weight: 1 });
          pendingLinkFrom = null;
          break;
        }
        pendingLinkFrom = null;
        selectOne(p.hitId, additive);
        break;
      }
      case 'port-out':
        if (p.hitId) {
          pendingLinkFrom = p.hitId;
          selectOne(p.hitId, additive);
        }
        break;
      case 'edge':
        pendingLinkFrom = null;
        if (p.hitId) selectOne(p.hitId, additive);
        break;
      case 'edge-delete':
        if (p.hitId) {
          // The X on an edge label routes to web's handleDeleteSelection
          // (:1587, 'delete'), not to a connection-shaped undo.
          sessionHistory.commit('delete', currentSnapshot());
          removeEdge(p.hitId);
        }
        break;
      case 'section':
      case 'note':
        pendingLinkFrom = null;
        if (p.hitId) selectOne(p.hitId, additive);
        break;
      case 'section-tone':
        if (p.hitId) cycleSectionTone(p.hitId);
        break;
      default: {
        pendingLinkFrom = null;
        if (p.tool === 'note') {
          const id = createNote(p.worldX, p.worldY);
          tool = null;
          openNoteEditor(id);
        } else if (p.tool === 'section') {
          const id = createSection(
            p.worldX - NEW_SECTION_W / 2,
            p.worldY - NEW_SECTION_H / 2,
            NEW_SECTION_W,
            NEW_SECTION_H,
          );
          tool = null;
          selectOne(id, false);
        } else if (!additive) {
          // A plain click on empty background clears the selection. An
          // additive (shift/ctrl) click on empty background is a no-op --
          // there is nothing under the pointer to toggle, and the existing
          // selection should survive a mis-aimed modifier-click, matching
          // `Canvas.tsx`'s own `default: if (!additive) clearSelection();`.
          clearSelection();
        }
      }
    }
  }

  function onSurfacePointerUp(e: PointerEvent): void {
    if (e.pointerType === 'touch') pinch = endPointer(touches, pinch, e.pointerId);
    if (e.pointerType === 'pen') penIsDown = false;
    if (!pending || pending.pointerId !== e.pointerId) return;
    const p = pending;
    pending = null;
    try {
      surfaceEl?.releasePointerCapture(e.pointerId);
    } catch {
      // Already released, or never captured (the gesture never promoted).
    }
    if (!p.active) handleClick(p);
    else finishDrag(p);
    // AFTER finishDrag wrote the topology, so endGesture sees the moved
    // result against the promote-time baseline and lands exactly one entry.
    closeUndoGesture();
  }

  function onSurfacePointerCancel(e: PointerEvent): void {
    if (e.pointerType === 'touch') pinch = endPointer(touches, pinch, e.pointerId);
    if (e.pointerType === 'pen') penIsDown = false;
    if (!pending || pending.pointerId !== e.pointerId) return;
    pending = null;
    isPanning = false;
    dragOverlay = null;
    groupOrigins = null;
    groupDelta = null;
    linkPreview = null;
    annMoveOverlay = null;
    annResizeOverlay = null;
    noteResizeOverlay = null;
    drawSectionOverlay = null;
    marqueeOverlay = null;
    // A cancelled gesture discards its overlays without touching topology,
    // so this only ever tidies away a baseline that can no longer move.
    closeUndoGesture();
  }

  /**
   * Suppress the browser's own context menu over the diagram: the press
   * that opened it already did this app's work (a right-press selects the
   * node so the Inspector opens), and a native menu on top of a design tool
   * only offers page-level actions that make no sense here.
   */
  function onSurfaceContextMenu(e: MouseEvent): void {
    e.preventDefault();
  }

  function onSurfaceDblClick(e: MouseEvent): void {
    const hit = hitTest(e.target);
    if (hit.kind === 'note' && hit.id) openNoteEditor(hit.id);
    else if (hit.kind === 'section' && hit.id) openSectionLabelEditor(hit.id);
    else if (hit.kind === 'node' && hit.id) openNodeRenameEditor(hit.id);
  }

  /* ------------------------------------------------------------------ *
   * Keyboard.
   * ------------------------------------------------------------------ */

  function isTypingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }

  function cancelGesture(): void {
    pending = null;
    isPanning = false;
    dragOverlay = null;
    groupOrigins = null;
    groupDelta = null;
    linkPreview = null;
    annMoveOverlay = null;
    annResizeOverlay = null;
    noteResizeOverlay = null;
    drawSectionOverlay = null;
    marqueeOverlay = null;
    // Escape ends the gesture the same way pointerup would, so an
    // interrupted drag still lands as one entry (nothing changed, so the
    // baseline discards itself rather than pushing an empty step).
    closeUndoGesture();
  }

  onMount(() => {
    function onKey(e: KeyboardEvent): void {
      if (isTypingTarget(e.target)) return;

      if (e.key === 'Escape') {
        cancelGesture();
        pendingLinkFrom = null;
        tool = null;
        clearSelection();
        return;
      }

      if (!e.ctrlKey && !e.metaKey && !e.altKey) {
        if (e.key === 'n' || e.key === 'N') {
          e.preventDefault();
          pendingLinkFrom = null;
          tool = tool === 'note' ? null : 'note';
          return;
        }
        if (e.key === 'b' || e.key === 'B') {
          e.preventDefault();
          pendingLinkFrom = null;
          tool = tool === 'section' ? null : 'section';
          return;
        }
        // Shift+1 fits everything, Shift+2 the selection: the pair every
        // canvas app ships, and already promised by Shortcuts.svelte's
        // View list. Positional codes, because Shift+1 is "!" only on some
        // layouts. Shift+2 with nothing selected is deliberately NOT
        // consumed, so it stays the browser's own shortcut.
        if (e.shiftKey && e.code === 'Digit1') {
          e.preventDefault();
          fitToContent();
          return;
        }
        if (e.shiftKey && e.code === 'Digit2') {
          const sel = nodes.filter((n) => topologyStore.selectedIds.has(n.id));
          if (sel.length === 0) return;
          e.preventDefault();
          fitTo(sel);
          return;
        }
      }

      if ((e.key === 'a' || e.key === 'A') && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        selectAll();
        return;
      }

      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (topologyStore.selectedIds.size === 0) return;
        e.preventDefault();
        const nodeIdSet = new Set(nodes.map((n) => n.id));
        const annIdSet = new Set(annotations.map((a) => a.id));
        const nodeIds: string[] = [];
        const edgeIds: string[] = [];
        const annIds: string[] = [];
        for (const id of topologyStore.selectedIds) {
          if (nodeIdSet.has(id)) nodeIds.push(id);
          else if (annIdSet.has(id)) annIds.push(id);
          else edgeIds.push(id);
        }
        // ONE entry for the whole selection, orphaned edges included,
        // after every guard and before either write -- App.tsx's
        // handleDeleteSelection orders it identically (:1587).
        if (nodeIds.length > 0 || edgeIds.length > 0 || annIds.length > 0) {
          sessionHistory.commit('delete', currentSnapshot());
        }
        if (nodeIds.length > 0 || edgeIds.length > 0) removeSelection(nodeIds, edgeIds);
        if (annIds.length > 0) {
          commitAnnotations(annotations.filter((a) => !annIds.includes(a.id)));
          for (const id of annIds) topologyStore.selectedIds.delete(id);
        }
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  function onNodeKeyDown(e: KeyboardEvent, node: SimNode): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      if (pendingLinkFrom && canLink(pendingLinkFrom, node.id)) {
        // Keyboard-confirmed connection: same 'connection' entry as the
        // two pointer paths (App.tsx :1556).
        sessionHistory.commit('connection', currentSnapshot());
        addEdge({ id: newId('edge'), from: pendingLinkFrom, to: node.id, weight: 1 });
        pendingLinkFrom = null;
      } else {
        selectOne(node.id, e.shiftKey || e.ctrlKey || e.metaKey);
      }
      return;
    }
    if (topologyStore.selectedNodeId === node.id) return; // canvas-level nudge not implemented; avoid double-move
    const step = e.shiftKey ? 1 : GRID;
    let dx = 0;
    let dy = 0;
    if (e.key === 'ArrowLeft') dx = -step;
    else if (e.key === 'ArrowRight') dx = step;
    else if (e.key === 'ArrowUp') dy = -step;
    else if (e.key === 'ArrowDown') dy = step;
    else return;
    e.preventDefault();
    e.stopPropagation();
    // Arrow nudges stream on key-repeat; touch coalesces one held burst
    // into a single 'move' entry and a lone press into one entry, exactly
    // App.tsx's guarded touch('move') at :1156 (the guard also keeps this
    // out of an open pointer-gesture bracket, though the two never overlap
    // today).
    if (!sessionHistory.inGesture) sessionHistory.touch('move', currentSnapshot());
    moveNode(node.id, node.x + dx, node.y + dy);
  }

  /* ------------------------------------------------------------------ *
   * System clipboard.
   *
   * Native `copy`/`cut`/`paste` document events, not key bindings: the
   * browser fires these for whatever chord the user's platform and layout
   * assign, focus in a text field keeps its native behaviour via the
   * `isTypingTarget` guard, and no clipboard permission prompt is ever
   * raised. Field-for-field port of `Canvas.tsx`'s own "system clipboard"
   * section.
   *
   * THE PASTE PATH IS UNTRUSTED. Whatever is on the clipboard is parsed and
   * structurally validated exactly the way a saved design would be
   * (`parseClipboardText`, `$lib/domain/clipboard.ts`); anything that does
   * not hold up is ignored without an error, and the event is left
   * unconsumed so the browser can do whatever it would otherwise have done.
   * ------------------------------------------------------------------ */

  /**
   * Append a cloned subgraph in one topology edit and make the clones the
   * new selection -- the one caller is paste, but this mirrors the web
   * app's own `appendClones` (`src/App.tsx`) so a future duplicate feature
   * could reuse it unchanged.
   */
  function appendClones(clones: { nodes: SimNode[]; edges: SimEdge[] }): void {
    const next = {
      ...topology,
      nodes: [...topology.nodes, ...clones.nodes],
      edges: [...topology.edges, ...clones.edges],
    };
    topologyStore.topology = next;
    simSetTopology(next).catch((e) =>
      pushError(`Pasting failed to reach the simulation engine: ${describeErr(e)}`),
    );
    const ids = new Set<string>();
    for (const n of clones.nodes) ids.add(n.id);
    for (const e2 of clones.edges) ids.add(e2.id);
    setSelection(ids);
  }

  function onCopyEvent(e: ClipboardEvent): void {
    if (isTypingTarget(e.target)) return;
    const text = buildClipboardText(topology, topologyStore.selectedIds);
    if (!text || !e.clipboardData) return;
    e.preventDefault();
    e.clipboardData.setData('text/plain', text);
  }

  function onCutEvent(e: ClipboardEvent): void {
    if (isTypingTarget(e.target)) return;
    const text = buildClipboardText(topology, topologyStore.selectedIds);
    if (!text || !e.clipboardData) return;
    e.preventDefault();
    e.clipboardData.setData('text/plain', text);
    // Cut = copy + the same single-edit delete the Delete key performs.
    // Annotations partition into their own bucket: the clipboard payload
    // never carries them (see `$lib/domain/clipboard.ts`), but a selected
    // note or section must still leave the canvas.
    const nodeIdSet = new Set(nodes.map((n) => n.id));
    const annIdSet = new Set(annotations.map((a) => a.id));
    const nodeIds: string[] = [];
    const edgeIds: string[] = [];
    const annIds: string[] = [];
    for (const id of topologyStore.selectedIds) {
      if (nodeIdSet.has(id)) nodeIds.push(id);
      else if (annIdSet.has(id)) annIds.push(id);
      else edgeIds.push(id);
    }
    // Web routes cut through the same handleDeleteSelection as the Delete
    // key (:1587), so cut is 'delete', not a label of its own -- one entry
    // for the whole removed selection, baseline before the writes.
    if (nodeIds.length > 0 || edgeIds.length > 0 || annIds.length > 0) {
      sessionHistory.commit('delete', currentSnapshot());
    }
    if (nodeIds.length > 0 || edgeIds.length > 0) removeSelection(nodeIds, edgeIds);
    if (annIds.length > 0) {
      commitAnnotations(annotations.filter((a) => !annIds.includes(a.id)));
      for (const id of annIds) topologyStore.selectedIds.delete(id);
    }
  }

  function onPasteEvent(e: ClipboardEvent): void {
    if (isTypingTarget(e.target)) return;
    const text = e.clipboardData?.getData('text/plain');
    if (!text) return;
    const sub = parseClipboardText(text);
    if (!sub || sub.nodes.length === 0) return;
    e.preventDefault();
    // At the pointer when it is over the canvas; at the centre of the
    // surface otherwise (a paste right after switching windows, say), so
    // the paste never lands off-screen. `lastClient` is tracked in client
    // coords and converted now, not when it was recorded, since the
    // viewport can pan and zoom in between -- see its declaration above.
    let at: { x: number; y: number };
    if (lastClient) {
      at = toWorld(lastClient.x, lastClient.y);
    } else if (surfaceEl) {
      const r = surfaceEl.getBoundingClientRect();
      at = toWorld(r.left + r.width / 2, r.top + r.height / 2);
    } else {
      at = { x: 0, y: 0 };
    }
    // The bounding-box centre lands on the pointer, moved by a grid-snapped
    // delta so the subgraph's internal offsets survive exactly and
    // grid-aligned content stays aligned.
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const n of sub.nodes) {
      if (n.x < minX) minX = n.x;
      if (n.y < minY) minY = n.y;
      if (n.x + NODE_W > maxX) maxX = n.x + NODE_W;
      if (n.y + NODE_H > maxY) maxY = n.y + NODE_H;
    }
    const dx = Math.round((at.x - (minX + maxX) / 2) / GRID) * GRID;
    const dy = Math.round((at.y - (minY + maxY) / 2) / GRID) * GRID;
    // After every guard and positioned, before the append -- App.tsx's
    // 'paste' commit sits at exactly this point (:1734).
    sessionHistory.commit('paste', currentSnapshot());
    appendClones(cloneSubgraph(sub, topology, dx, dy));
  }

  onMount(() => {
    document.addEventListener('copy', onCopyEvent);
    document.addEventListener('cut', onCutEvent);
    document.addEventListener('paste', onPasteEvent);
    return () => {
      document.removeEventListener('copy', onCopyEvent);
      document.removeEventListener('cut', onCutEvent);
      document.removeEventListener('paste', onPasteEvent);
    };
  });

  /* ------------------------------------------------------------------ *
   * Palette drop.
   * ------------------------------------------------------------------ */

  function onDragOver(e: DragEvent): void {
    const dt = e.dataTransfer;
    if (!dt || (!dt.types.includes(NODE_DND_MIME) && !dt.types.includes(ANN_DND_MIME))) return;
    e.preventDefault();
    dt.dropEffect = 'copy';
  }

  function onDrop(e: DragEvent): void {
    const dt = e.dataTransfer;
    if (!dt) return;

    const ann = dt.getData(ANN_DND_MIME);
    if (ann === 'note' || ann === 'section') {
      e.preventDefault();
      const w = toWorld(e.clientX, e.clientY);
      if (ann === 'note') {
        openNoteEditor(createNote(w.x, w.y));
      } else {
        const id = createSection(w.x - NEW_SECTION_W / 2, w.y - NEW_SECTION_H / 2, NEW_SECTION_W, NEW_SECTION_H);
        selectOne(id, false);
      }
      return;
    }

    const kind = dt.getData(NODE_DND_MIME) as NodeKind;
    if (!kind) return;
    e.preventDefault();
    const w = toWorld(e.clientX, e.clientY);
    const node = makeNode(kind, snapIf(w.x - NODE_W / 2), snapIf(w.y - NODE_H / 2));
    // Same 'add' entry as the +page palette click (App.tsx :1476); the
    // annotation drops above ride on createNote/createSection's own
    // commits instead.
    sessionHistory.commit('add', currentSnapshot());
    addNode(node);
    selectOne(node.id, false);
  }

  /* ------------------------------------------------------------------ *
   * Render-position helpers (apply live drag overlays).
   * ------------------------------------------------------------------ */

  function nodePos(n: SimNode): { x: number; y: number } {
    if (dragOverlay && dragOverlay.id === n.id) return { x: dragOverlay.x, y: dragOverlay.y };
    // A group member of the drag in progress follows the shared delta; the
    // grabbed node is handled by the overlay above and is never in origins.
    const g = groupDelta;
    const o = groupOrigins?.get(n.id);
    if (g && o) return { x: o.x + g.dx, y: o.y + g.dy };
    return { x: n.x, y: n.y };
  }

  function sectionRect(s: Section): { x: number; y: number; w: number; h: number } {
    if (annResizeOverlay && annResizeOverlay.id === s.id) return annResizeOverlay.rect;
    if (annMoveOverlay && annMoveOverlay.id === s.id) {
      return { x: s.x + annMoveOverlay.dx, y: s.y + annMoveOverlay.dy, w: s.width, h: s.height };
    }
    const g = groupDelta;
    const sec = groupOrigins?.get(s.id);
    if (g && sec) return { x: sec.x + g.dx, y: sec.y + g.dy, w: s.width, h: s.height };
    return { x: s.x, y: s.y, w: s.width, h: s.height };
  }

  function notePos(n: Note): { x: number; y: number } {
    if (noteResizeOverlay && noteResizeOverlay.id === n.id) return { x: noteResizeOverlay.x, y: n.y };
    if (annMoveOverlay && annMoveOverlay.id === n.id) return { x: n.x + annMoveOverlay.dx, y: n.y + annMoveOverlay.dy };
    const g = groupDelta;
    const origin = groupOrigins?.get(n.id);
    if (g && origin) return { x: origin.x + g.dx, y: origin.y + g.dy };
    return { x: n.x, y: n.y };
  }

  function noteWidth(n: Note): number {
    if (noteResizeOverlay && noteResizeOverlay.id === n.id) return noteResizeOverlay.width;
    return n.width;
  }

  /* ------------------------------------------------------------------ *
   * Edge geometry helpers.
   * ------------------------------------------------------------------ */

  function edgeRect(id: string): { x: number; y: number; w: number; h: number } | null {
    const n = nodeById.get(id);
    if (!n) return null;
    const pos = nodePos(n);
    return { x: pos.x, y: pos.y, w: NODE_W, h: NODE_H };
  }

  function laneFor(edge: SimEdge): number {
    const hasReverse = edges.some((e) => e.from === edge.to && e.to === edge.from);
    if (!hasReverse) return 0;
    return edge.from < edge.to ? -1 : 1;
  }

  function edgeWidth(f: number): number {
    return f <= 0 ? 1 : clamp(1 + Math.log10(1 + f) * 0.85, 1, 4.5);
  }

  /**
   * Vertical offsets that keep coincident edge rate labels apart, keyed by
   * edge id -- `labelDyById` (edge-route.ts), a port of `Canvas.tsx`'s own
   * derivation. Fed from the SAME rects, lane and live snapshot the
   * template draws with, so the bucketed anchor is the exact point each
   * label renders at; entries are supplied in edge order, the order the
   * stagger itself is assigned in. Empty whenever `showEdgeLabels` is off.
   */
  const edgeLabelDy = $derived.by(() =>
    labelDyById(
      edges.map((edge) => {
        const state = snapshot?.edgeState[edge.id] ?? 'idle';
        return {
          id: edge.id,
          a: edgeRect(edge.from),
          b: edgeRect(edge.to),
          lane: laneFor(edge),
          control: Boolean(edge.control),
          severed: state === 'cut' || state === 'blocked',
          flow: snapshot?.edgeFlow[edge.id] ?? 0,
        };
      }),
      showEdgeLabels,
    ),
  );

  /* ------------------------------------------------------------------ *
   * Node label / cell text fitting (mirrors Canvas.tsx's budget maths).
   * ------------------------------------------------------------------ */

  function cellWidth(value: string, label: string): number {
    const gap = value && label ? CELL_GAP_W : 0;
    return measureText(value, VAL_STYLE) + measureText(label, CAP_STYLE) + gap;
  }

  function fitCellLen(value: string, label: string, budget: number): number | undefined {
    return cellWidth(value, label) <= budget ? undefined : budget;
  }

  function fitPrimaryLen(value: string, label: string, selected: boolean): number | undefined {
    const budget = SPARK_X - PAD_X - 6;
    const vs = selected ? VAL_SELECTED_STYLE : VAL_PRIMARY_STYLE;
    const gap = value && label ? CELL_GAP_W + 1 : 0;
    const w = measureText(value, vs) + measureText(label, CAP_STYLE) + gap;
    return w <= budget ? undefined : budget;
  }

  function truncateLabel(label: string, showHeader: boolean, badgeText: string, hasMark: boolean): string {
    const startX = showHeader ? PAD_X + GLYPH_PX + NAME_GAP : PAD_X;
    const reserve = showHeader
      ? (hasMark ? MARK_RESERVE : 0) + (badgeText ? measureText(badgeText, BADGE_STYLE) + NAME_GAP : 0)
      : 0;
    const avail = NODE_W - PAD_X - startX - reserve;
    return truncateToWidth(label, avail, NAME_STYLE);
  }

  function sectionLabelWidth(label: string): number {
    return Math.max(SEC_LABEL_MIN_W, measureText(label, SEC_LABEL_STYLE) + SEC_LABEL_PAD_X * 2);
  }

  /** Round up to a friendly ceiling so a sparkline's y-domain does not jitter. */
  function niceCeil(v: number): number {
    if (!Number.isFinite(v) || v <= 0) return 1;
    const mag = 10 ** Math.floor(Math.log10(v));
    const n = v / mag;
    const step = n <= 1 ? 1 : n <= 2 ? 2 : n <= 5 ? 5 : 10;
    return step * mag;
  }

  function sparkPoints(data: number[] | undefined, unit: boolean): string {
    if (!data || data.length < 2) return '';
    let max = 0;
    let filled = 0;
    for (const v of data) {
      if (!Number.isFinite(v)) continue;
      filled += 1;
      if (v > max) max = v;
    }
    if (filled < 2) return '';
    const top = unit && max >= 0.1 ? 1 : niceCeil(max * 1.15);
    const n = data.length;
    const out: string[] = [];
    for (let i = n - filled; i < n; i++) {
      const v = data[i];
      if (!Number.isFinite(v)) continue;
      const x = filled > 1 ? ((i - (n - filled)) / (filled - 1)) * SPARK_W : SPARK_W;
      const y = SPARK_H - clamp(v / top, 0, 1) * SPARK_H;
      out.push(`${x.toFixed(1)},${y.toFixed(1)}`);
    }
    return out.join(' ');
  }

  /** Which of the three structural drawings a node gets -- see geometry.ts's NodeStats doc. */
  function structureOf(
    kind: NodeKind,
    unitsLen: number,
    pendingUnits: number,
  ): 'strip' | 'vessel' | 'stack' | 'none' {
    if (kind === 'shard' || kind === 'replica' || kind === 'streambroker') return 'strip';
    if (kind === 'queue') return 'vessel';
    if (unitsLen > 1) return 'stack';
    if (pendingUnits > 0) return 'stack';
    return 'none';
  }

  function nodeAriaLabel(node: SimNode, stats: NodeStats | null, readout: Readout | null, fault: FailureKind | null): string {
    const parts: (string | null)[] = [];
    if (node.label.trim().toLowerCase() !== KIND_NAME[node.kind].toLowerCase()) {
      parts.push(KIND_NAME[node.kind]);
    }
    parts.push(node.label || KIND_NAME[node.kind]);
    if (readout) parts.push(`${readout.primary.value} ${readout.primary.label}`);
    const badge = stackBadge(stats?.instances);
    if (badge) {
      if (node.kind === 'shard' || node.kind === 'streambroker') parts.push(`${stats?.instances} partitions`);
      else if (node.kind === 'replica') parts.push(`primary plus ${(stats?.instances ?? 1) - 1} read replicas`);
      else parts.push(`${stats?.instances} instances`);
    }
    const pendingN = stats?.instancesPending ?? 0;
    if (pendingN > 0) parts.push(`${pendingN} warming up`);
    if (fault) parts.push(`faulted: ${fault}`);
    return parts.filter((x): x is string => Boolean(x)).join(', ');
  }
</script>

<div
  class="cv-host"
  bind:this={hostEl}
  bind:clientWidth={surfaceW}
  bind:clientHeight={surfaceH}
>
  <div
    class="cv-surface"
    class:is-pan={isPanning}
    class:is-tool-note={tool === 'note'}
    class:is-tool-section={tool === 'section'}
    bind:this={surfaceEl}
    role="application"
    aria-label="System design canvas"
    style="--grid-ox:{view.x}px; --grid-oy:{view.y}px; --grid-k:{view.k};"
    onpointerdown={onSurfacePointerDown}
    onpointermove={onSurfacePointerMove}
    onpointerup={onSurfacePointerUp}
    onpointercancel={onSurfacePointerCancel}
    ondblclick={onSurfaceDblClick}
    oncontextmenu={onSurfaceContextMenu}
    ondragover={onDragOver}
    ondrop={onDrop}
  >
    <svg class="cv-svg" width="100%" height="100%" role="presentation">
      <g transform="translate({view.x},{view.y}) scale({view.k})">
        <!-- ==================== Sections (behind everything) ==================== -->
        {#each sections as s (s.id)}
          {@const rect = sectionRect(s)}
          <g
            class="cv-section"
            class:is-selected={topologyStore.selectedIds.has(s.id)}
            data-tone={((s.tone % SECTION_TONE_COUNT) + SECTION_TONE_COUNT) % SECTION_TONE_COUNT}
            style={s.color
              ? `--sec-fill: color-mix(in srgb, ${s.color} 12%, transparent); --sec-line: color-mix(in srgb, ${s.color} 55%, transparent); --sec-ink: ${s.color};`
              : undefined}
          >
            <rect class="cv-section-fill" data-hit="section" data-id={s.id} x={rect.x} y={rect.y} width={rect.w} height={rect.h} rx="8" />
            <rect class="cv-section-line" x={rect.x} y={rect.y} width={rect.w} height={rect.h} rx="8" />
            <rect class="cv-section-hit" data-hit="section" data-id={s.id} x={rect.x} y={rect.y} width={rect.w} height={rect.h} rx="8" />
            <g class="cv-section-label" data-hit="section" data-id={s.id}>
              <rect
                class="cv-section-label-bg"
                x={rect.x}
                y={rect.y - SEC_LABEL_H - 4}
                width={Math.min(sectionLabelWidth(s.label), rect.w)}
                height={SEC_LABEL_H}
                rx="6"
              />
              {#if sectionLabelEditor?.id !== s.id && s.label}
                <text class="cv-section-label-text" x={rect.x + SEC_LABEL_PAD_X} y={rect.y - SEC_LABEL_H / 2 - 4}>{truncateToWidth(s.label, rect.w - SEC_LABEL_PAD_X * 2, SEC_LABEL_STYLE)}</text>
              {/if}
            </g>
          </g>
        {/each}

        <!-- ==================== Edges ==================== -->
        {#each edges as edge (edge.id)}
          {@const a = edgeRect(edge.from)}
          {@const b = edgeRect(edge.to)}
          {#if a && b}
            {@const route = routeEdge(a, b, laneFor(edge))}
            {@const flow = snapshot?.edgeFlow[edge.id] ?? 0}
            {@const state = snapshot?.edgeState[edge.id] ?? 'idle'}
            {@const control = Boolean(edge.control)}
            {@const severed = state === 'cut' || state === 'blocked'}
            {@const width = severed || control ? 1 : edgeWidth(flow)}
            {@const active = !severed && !control && flow > 0.05}
            {@const targetNode = nodeById.get(edge.to)}
            {@const targetStats = targetNode ? statsById[targetNode.id] : undefined}
            {@const targetReadout = targetNode && targetStats ? readoutFor(targetNode.kind, targetStats, targetNode.config, backlogs.get(targetNode.id) ?? 0) : null}
            {@const targetHealth = targetReadout?.health ?? 'ok'}
            {@const selected = topologyStore.selectedIds.has(edge.id)}
            {@const labelDy = edgeLabelDy.get(edge.id) ?? 0}
            <g
              class="cv-edge is-{severed || control ? 'ok' : targetHealth} is-state-{state}"
              class:is-selected={selected}
              class:is-active={active}
              class:is-control={control}
            >
              <path d={route.d} class="cv-edge-hit" data-hit="edge" data-id={edge.id} />
              <path d={route.d} class="cv-edge-line" style="stroke-width:{width}" />
              {#if active}
                <path
                  d={route.d}
                  class="cv-edge-flow"
                  style="stroke-width:{width * 0.75}; --dash-on:3; --dash-off:{clamp(20 - Math.log10(1 + flow) * 4, 7, 20) - 3}; --dash-cycle:{-clamp(20 - Math.log10(1 + flow) * 4, 7, 20)}; animation-duration:{clamp(3.2 / Math.log10(10 + flow), 0.35, 2.4)}s"
                ></path>
              {/if}
              <path d={arrowPath(route.tip, route.dir)} class="cv-edge-arrow" />
              {#if severed}
                <g class="cv-edge-break" transform="translate({route.label.x},{route.label.y})">
                  <rect class="cv-edge-break-gap" x="-7" y="-7" width="14" height="14" />
                  <path class="cv-edge-break-mark" d="M-4.5,-5 L-1.5,0 L-4.5,5 M4.5,-5 L1.5,0 L4.5,5" />
                </g>
              {/if}
              {#if showEdgeLabels && control}
                <text class="cv-edge-label is-control" x={route.label.x} y={route.label.y - 6 + labelDy}>scales</text>
              {/if}
              {#if showEdgeLabels && active}
                <text class="cv-edge-label" x={route.label.x} y={route.label.y - 6 + labelDy}>{formatRate(flow)}</text>
              {/if}
              {#if selected}
                <g class="cv-edge-del" transform="translate({route.label.x},{route.label.y})" data-hit="edge-delete" data-id={edge.id} role="button" aria-label="Delete connection">
                  <rect class="cv-edge-del-hit" x="-22" y="-22" width="44" height="44" />
                  <rect x="-9" y="-9" width="18" height="18" rx="3" />
                  <path d="M-3.5,-3.5 L3.5,3.5 M3.5,-3.5 L-3.5,3.5" />
                </g>
              {/if}
            </g>
          {/if}
        {/each}

        <!-- ==================== Link preview (in-flight wire drag) ==================== -->
        {#if linkPreview}
          {@const srcRect = edgeRect(linkPreview.from)}
          {#if srcRect}
            <path
              class="cv-link-preview"
              class:is-valid={Boolean(linkPreview.over)}
              d={previewPath(srcRect, linkPreview.x, linkPreview.y)}
            />
          {/if}
        {/if}
        {#if pendingLinkFrom}
          {@const srcRect = edgeRect(pendingLinkFrom)}
          {#if srcRect}
            <circle class="cv-link-armed" cx={srcRect.x + srcRect.w} cy={srcRect.y + srcRect.h / 2} r="9" />
          {/if}
        {/if}

        <!-- ==================== Section draw-in-progress ==================== -->
        {#if drawSectionOverlay}
          <rect
            class="cv-section-draw"
            x={drawSectionOverlay.x}
            y={drawSectionOverlay.y}
            width={drawSectionOverlay.w}
            height={drawSectionOverlay.h}
            rx="8"
          />
        {/if}

        <!-- ==================== Marquee-select, in progress ==================== -->
        {#if marqueeOverlay}
          <rect
            class="cv-marquee"
            x={marqueeOverlay.x}
            y={marqueeOverlay.y}
            width={marqueeOverlay.w}
            height={marqueeOverlay.h}
          />
        {/if}

        <!-- ==================== Nodes ==================== -->
        {#each nodes as node (node.id)}
          {@const pos = nodePos(node)}
          {@const stats = statsById[node.id] ?? null}
          {@const readout = stats ? readoutFor(node.kind, stats, node.config, backlogs.get(node.id) ?? 0) : null}
          {@const fault = faultByNode.get(node.id) ?? null}
          {@const health = (fault ? 'danger' : readout ? readout.health : 'ok') as Health}
          {@const units = stats?.perInstance}
          {@const pendingUnits = stats?.instancesPending ?? 0}
          {@const badge = stackBadge(stats?.instances)}
          {@const structure = structureOf(node.kind, units?.length ?? 0, pendingUnits)}
          {@const selected = topologyStore.selectedIds.has(node.id)}
          {@const linking = Boolean(linkPreview) || Boolean(pendingLinkFrom)}
          {@const linkRole = linkPreview?.from === node.id || pendingLinkFrom === node.id
            ? 'source'
            : linking
              ? linkPreview
                ? canLink(linkPreview.from, node.id) ? 'valid' : 'invalid'
                : pendingLinkFrom
                  ? canLink(pendingLinkFrom, node.id) ? 'valid' : 'invalid'
                  : 'none'
              : 'none'}
          {@const linkTarget = linkPreview?.over === node.id}
          {@const load = readout ? clamp(readout.load, 0, 1) : 0}
          {@const meterW = load > 0 ? Math.max(2, load * METER_W) : 0}
          {@const full = detail === 2}
          {@const showHeader = detail >= 1}
          {@const badgeText = badge ? `${badge}${pendingUnits > 0 ? `+${pendingUnits}` : ''}` : ''}
          {@const shownName = truncateLabel(node.label, showHeader, badgeText, Boolean(fault) || health !== 'ok')}
          <g
            class="cv-node is-{health}"
            class:is-selected={selected}
            class:is-losing={readout?.losing}
            class:is-faulted={Boolean(fault)}
            class:is-linking={linking}
            class:has-strip={structure === 'strip'}
            class:is-link-source={linkRole === 'source'}
            class:is-link-valid={linkRole === 'valid'}
            class:is-link-invalid={linkRole === 'invalid'}
            class:is-link-target={linkTarget}
            transform="translate({pos.x},{pos.y})"
            tabindex="0"
            role="button"
            aria-label={nodeAriaLabel(node, stats, readout, fault)}
            aria-pressed={selected}
            data-hit="node"
            data-id={node.id}
            data-kind={node.kind}
            onkeydown={(e) => onNodeKeyDown(e, node)}
          >
            <title>{node.label || KIND_NAME[node.kind]}</title>

            {#if structure === 'stack' && showHeader}
              <g class="cv-units" aria-hidden="true">
                {#each stackLayers(units?.length ?? 1, pendingUnits) as layer (layer.offset + (layer.pending ? 'p' : 'l'))}
                  <rect class={layer.pending ? 'cv-unit is-pending' : 'cv-unit'} x={layer.offset} y={-layer.offset} width={NODE_W} height={NODE_H} rx={NODE_R} ry={NODE_R} />
                {/each}
              </g>
            {/if}

            <rect class="cv-node-body" width={NODE_W} height={NODE_H} rx={NODE_R} ry={NODE_R} />

            {#if selected}
              <rect class="cv-node-ring" x={-RING_GAP} y={-RING_GAP} width={NODE_W + RING_GAP * 2} height={NODE_H + RING_GAP * 2} rx={NODE_R + RING_GAP} ry={NODE_R + RING_GAP} />
            {/if}

            {#if showHeader}
              <line class="cv-node-hair" x1={PAD_X} y1={HEAD_H} x2={NODE_W - PAD_X} y2={HEAD_H} />
              <g class="cv-node-icon" transform="translate({PAD_X},{GLYPH_Y}) scale({GLYPH_SCALE})">
                <g class="cv-glyph" fill="none" stroke="currentColor" stroke-width={(2 * GLYPH_PX) / 24} stroke-linecap="round" stroke-linejoin="round">
                  {#each KIND_ICON[node.kind] as [tag, attrs], i (i)}
                    <svelte:element this={tag} {...attrs} />
                  {/each}
                </g>
              </g>
            {/if}

            <text class="cv-node-name" x={showHeader ? PAD_X + GLYPH_PX + NAME_GAP : PAD_X} y={showHeader ? HEAD_CENTER_Y + 4 : 24}>{shownName}</text>

            {#if showHeader && fault}
              <rect class="cv-node-mark is-fault" x={NODE_W - PAD_X - MARK_SIZE} y={HEAD_CENTER_Y - MARK_SIZE / 2} width={MARK_SIZE} height={MARK_SIZE} />
            {:else if showHeader && health === 'danger'}
              <circle class="cv-node-mark is-danger" cx={NODE_W - PAD_X - MARK_SIZE / 2} cy={HEAD_CENTER_Y} r={MARK_SIZE / 2} />
            {:else if showHeader && health === 'warn'}
              <circle class="cv-node-mark is-warn" cx={NODE_W - PAD_X - MARK_SIZE / 2} cy={HEAD_CENTER_Y} r={MARK_SIZE / 2 - 1} />
            {/if}

            {#if showHeader && badgeText}
              <text class={pendingUnits > 0 ? 'cv-node-badge is-warming' : 'cv-node-badge'} x={NODE_W - PAD_X - (fault || health !== 'ok' ? MARK_RESERVE : 0)} y={HEAD_CENTER_Y} text-anchor="end">{badgeText}</text>
            {/if}

            {#if full && readout}
              <text class="cv-node-primary" x={PAD_X} y="52" textLength={fitPrimaryLen(readout.primary.value, readout.primary.label, selected && structure !== 'strip')} lengthAdjust={fitPrimaryLen(readout.primary.value, readout.primary.label, selected && structure !== 'strip') ? 'spacingAndGlyphs' : undefined}>
                <tspan class="cv-val">{readout.primary.value}</tspan><tspan class="cv-cap" dx="4">{readout.primary.label}</tspan>
              </text>

              {#if structure === 'strip' && (readout.a.value || readout.a.label)}
                <text class="cv-node-sec" x={NODE_W - PAD_X} y="52" text-anchor="end" textLength={fitCellLen(readout.a.value, readout.a.label, 60)} lengthAdjust={fitCellLen(readout.a.value, readout.a.label, 60) ? 'spacingAndGlyphs' : undefined}>
                  <tspan class="cv-val">{readout.a.value}</tspan>{#if readout.a.value && readout.a.label}<tspan class="cv-cap" dx="3">{readout.a.label}</tspan>{:else}<tspan class="cv-cap">{readout.a.label}</tspan>{/if}
                </text>
              {/if}

              {#if structure === 'strip' && units}
                {@const strip = cellStrip(units.length, METER_W)}
                <g class="cv-strip" transform="translate({PAD_X},{STRIP_Y})" aria-hidden="true">
                  {#each units as raw, i (i)}
                    {@const v = clamp(Number.isFinite(raw) ? raw : 0, 0, 1)}
                    {@const fh = v > 0 ? Math.max(1, v * STRIP_H) : 0}
                    {@const leadIndex = node.kind === 'replica' ? 0 : -1}
                    <g>
                      <rect class={i === leadIndex ? 'cv-cell is-lead' : 'cv-cell'} x={strip.x(i)} y="0" width={strip.w} height={STRIP_H} />
                      {#if fh > 0}
                        <rect class="cv-cell-fill {toneClass(healthOfLoad(v)) ?? ''} {i === leadIndex ? 'is-lead' : ''}" x={strip.x(i)} y={STRIP_H - fh} width={strip.w} height={fh} />
                      {/if}
                    </g>
                  {/each}
                  {#if (node.kind === 'replica' ? 0 : -1) >= 0}
                    <rect class="cv-cell-lead-notch" x={strip.x(0)} y={STRIP_H + 1.5} width={strip.w} height="2" />
                  {/if}
                </g>
              {:else if structure === 'vessel' && stats}
                {@const lim = stats.queueLimit > 0 ? stats.queueLimit : 1}
                {@const fill = clamp(stats.queued / lim, 0, 1)}
                {@const fw = fill > 0 ? Math.max(1.5, fill * SPARK_W) : 0}
                <g class={stats.shedRate > 0 ? 'cv-vessel is-shedding' : 'cv-vessel'} transform="translate({SPARK_X},{SPARK_Y + SPARK_H - 10})" aria-hidden="true">
                  <rect class="cv-vessel-track" x="0" y="0" width={SPARK_W} height="10" rx="2" />
                  {#if fw > 0}<rect class="cv-vessel-fill" x="0" y="0" width={fw} height="10" rx="2" />{/if}
                  <rect class="cv-vessel-brim" x={SPARK_W - 1} y="-1" width="1.5" height="12" />
                </g>
              {:else if settingsStore.sparklines}
                {@const pts = sparkPoints(sparkHistory.get(node.id), readout.sparkUnit)}
                <g transform="translate({SPARK_X},{SPARK_Y})">
                  <line class="cv-spark-base" x1="0" y1={SPARK_H + 0.5} x2={SPARK_W} y2={SPARK_H + 0.5} />
                  {#if pts}<polyline class="cv-spark" points={pts} />{/if}
                </g>
              {/if}

              {#if structure !== 'strip'}
                <text class="cv-node-sec" x={PAD_X} y="70" textLength={fitCellLen(readout.a.value, readout.a.label, SEC_HALF)} lengthAdjust={fitCellLen(readout.a.value, readout.a.label, SEC_HALF) ? 'spacingAndGlyphs' : undefined}>
                  <tspan class="cv-val">{readout.a.value}</tspan>{#if readout.a.value && readout.a.label}<tspan class="cv-cap" dx="3">{readout.a.label}</tspan>{:else}<tspan class="cv-cap">{readout.a.label}</tspan>{/if}
                </text>
                <text class="cv-node-sec" x={NODE_W - PAD_X} y="70" text-anchor="end" textLength={fitCellLen(readout.b.value, readout.b.label, SEC_HALF)} lengthAdjust={fitCellLen(readout.b.value, readout.b.label, SEC_HALF) ? 'spacingAndGlyphs' : undefined}>
                  <tspan class="cv-val">{readout.b.value}</tspan>{#if readout.b.value && readout.b.label}<tspan class="cv-cap" dx="3">{readout.b.label}</tspan>{:else}<tspan class="cv-cap">{readout.b.label}</tspan>{/if}
                </text>
              {/if}
            {/if}

            <rect class="cv-meter-track" x={PAD_X} y={METER_Y} width={METER_W} height={METER_H} rx="1.5" />
            {#if meterW > 0}
              <rect class="cv-meter-fill {toneClass(health) ?? ''}" x={PAD_X} y={METER_Y} width={meterW} height={METER_H} rx="1.5" />
            {/if}
            <rect class="cv-meter-tick" x={PAD_X + METER_W * WARN_AT} y={METER_Y - 1} width="1" height={METER_H + 2} />

            <circle class="cv-port-hit" cx="0" cy={PORT_CY} r={PORT_HIT_R} data-hit="port-in" data-id={node.id} />
            <circle class="cv-port cv-port-in" cx="0" cy={PORT_CY} r={PORT_R} />
            <circle class="cv-port-hit" cx={NODE_W} cy={PORT_CY} r={PORT_HIT_R} data-hit="port-out" data-id={node.id} />
            <circle class="cv-port cv-port-out" cx={NODE_W} cy={PORT_CY} r={PORT_R} />
          </g>
        {/each}

        <!-- ==================== Notes (in front of nodes) ==================== -->
        {#each notesList as note (note.id)}
          {@const pos = notePos(note)}
          {@const width = noteWidth(note)}
          {@const layout = layoutNote(note.text, width, note.size, note.font, note.bold, note.italic, note.scale)}
          {@const selected = topologyStore.selectedIds.has(note.id)}
          <g
            class="cv-note is-{note.size}"
            class:is-selected={selected}
            class:is-bold={note.bold}
            class:is-italic={note.italic}
            class:is-underline={note.underline}
            data-font={note.font ?? 'sans'}
            data-tone={note.tone ?? undefined}
            style={note.color ? `color:${note.color}` : undefined}
            transform="translate({pos.x},{pos.y})"
          >
            <rect class="cv-note-hit" data-hit="note" data-id={note.id} x="-6" y="-4" width={width + 12} height={layout.height + 8} />
            {#if noteEditor?.id !== note.id}
              <text class="cv-note-text" x="0" y="0" style={note.scale ? `font-size:${layout.font}px` : undefined}>
                {#each layout.lines as line, i (i)}
                  <tspan x="0" y={layout.baseline + i * layout.lineH}>{line === '' ? ' ' : line}</tspan>
                {/each}
              </text>
            {/if}
          </g>
        {/each}

        <!-- ==================== Annotation selection chrome (top layer) ====================
             Only drawn for a single-selected annotation (`selectedAnnotation`,
             derived above): with several things selected at once, resize
             handles for just one of them would be a lie about which one is
             grabbable, matching the web app's own single-selection gate
             (`Canvas.tsx`: "null for a multi-selection on purpose"). -->
        {#if selectedAnnotation}
          {@const sel = selectedAnnotation}
          {#if isSection(sel)}
            {@const rect = sectionRect(sel)}
            {@const ui = 1 / view.k}
            <g class="cv-ann-sel">
              <rect class="cv-ann-ring" x={rect.x - 3 * ui} y={rect.y - 3 * ui} width={rect.w + 6 * ui} height={rect.h + 6 * ui} rx={8 + 3 * ui} />
              {#each RESIZE_DIRS as dir (dir)}
                {@const a = handleAnchor({ x: rect.x, y: rect.y, w: rect.w, h: rect.h }, dir)}
                {@const hs = 9 * ui}
                {@const hit = 36 * ui}
                <rect class="cv-handle" x={a.x - hs / 2} y={a.y - hs / 2} width={hs} height={hs} rx={2 * ui} />
                <rect class="cv-handle-hit" data-hit="section-resize" data-id={sel.id} data-dir={dir} x={a.x - hit / 2} y={a.y - hit / 2} width={hit} height={hit} />
              {/each}
              <rect
                class="cv-sec-tone-swatch"
                data-hit="section-tone"
                data-id={sel.id}
                role="button"
                aria-label="Change section shade"
                x={rect.x}
                y={rect.y + rect.h + 10 * ui}
                width={15 * ui}
                height={15 * ui}
                rx={3 * ui}
                data-tone={sel.tone}
              />
            </g>
          {:else if isNote(sel)}
            {@const pos = notePos(sel)}
            {@const width = noteWidth(sel)}
            {@const layout = layoutNote(sel.text, width, sel.size, sel.font, sel.bold, sel.italic, sel.scale)}
            {@const ui = 1 / view.k}
            {@const hs = 9 * ui}
            {@const hit = 36 * ui}
            {@const midY = pos.y + layout.height / 2}
            <g class="cv-ann-sel">
              <rect class="cv-ann-ring" x={pos.x - 6} y={pos.y - 4} width={width + 12} height={layout.height + 8} rx="4" />
              <rect class="cv-handle" x={pos.x - 6 - hs / 2} y={midY - hs / 2} width={hs} height={hs} rx={2 * ui} />
              <rect class="cv-handle-hit" data-hit="note-resize" data-id={sel.id} data-dir="w" x={pos.x - 6 - hit / 2} y={midY - hit / 2} width={hit} height={hit} />
              <rect class="cv-handle" x={pos.x + width + 6 - hs / 2} y={midY - hs / 2} width={hs} height={hs} rx={2 * ui} />
              <rect class="cv-handle-hit" data-hit="note-resize" data-id={sel.id} data-dir="e" x={pos.x + width + 6 - hit / 2} y={midY - hit / 2} width={hit} height={hit} />
            </g>
          {/if}
        {/if}
      </g>
    </svg>
  </div>

  <Minimap nodes={topology.nodes} {view} surface={{ width: surfaceW, height: surfaceH }} ongoto={gotoWorld} />

  {#if tool}
    <div class="cv-tool-hint" data-chrome="tool-hint">
      {tool === 'note' ? 'Click the canvas to place a note (Esc to cancel)' : 'Drag on the canvas to frame a section (Esc to cancel)'}
    </div>
  {/if}

  {#if noteEditor}
    {@const editor = noteEditor}
    {@const note = annotationById.get(editor.id)}
    {#if note && isNote(note)}
      {@const pos = notePos(note)}
      <textarea
        bind:this={noteEditorEl}
        class="cv-note-editor"
        data-chrome="note-editor"
        style="left:{view.x + pos.x * view.k}px; top:{view.y + pos.y * view.k}px; width:{noteWidth(note) * view.k}px; font-size:{14 * view.k}px;"
        bind:value={editor.draft}
        onblur={commitNoteEditor}
        onkeydown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            noteEditor = null;
          } else if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            commitNoteEditor();
          }
        }}
      ></textarea>
    {/if}
  {/if}

  {#if sectionLabelEditor}
    {@const editor = sectionLabelEditor}
    {@const sec = annotationById.get(editor.id)}
    {#if sec && isSection(sec)}
      {@const rect = sectionRect(sec)}
      <input
        bind:this={sectionLabelEditorEl}
        class="cv-section-label-editor"
        data-chrome="section-label-editor"
        style="left:{view.x + rect.x * view.k}px; top:{view.y + (rect.y - SEC_LABEL_H - 4) * view.k}px; width:{Math.max(120, rect.w) * view.k}px;"
        bind:value={editor.draft}
        onblur={commitSectionLabelEditor}
        onkeydown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            sectionLabelEditor = null;
          } else if (e.key === 'Enter') {
            e.preventDefault();
            commitSectionLabelEditor();
          }
        }}
      />
    {/if}
  {/if}

  {#if nodeRenameEditor}
    {@const editor = nodeRenameEditor}
    {@const node = nodeById.get(editor.id)}
    {#if node}
      {@const pos = nodePos(node)}
      <input
        bind:this={nodeRenameEditorEl}
        class="cv-node-rename-editor"
        data-chrome="node-rename-editor"
        style="left:{view.x + (pos.x + PAD_X) * view.k}px; top:{view.y + pos.y * view.k}px; width:{(NODE_W - PAD_X * 2) * view.k}px; height:{HEAD_H * view.k}px; font-size:{14 * view.k}px;"
        bind:value={editor.draft}
        onblur={commitNodeRenameEditor}
        onkeydown={(e) => {
          if (e.key === 'Escape') {
            e.preventDefault();
            nodeRenameEditor = null;
          } else if (e.key === 'Enter') {
            e.preventDefault();
            commitNodeRenameEditor();
          }
        }}
      />
    {/if}
  {/if}

  <!-- The zoom cluster. The % badge used to live alone at the bottom LEFT,
       where shell.css could not lift it (it only lifts .cv-ledger/.cv-zoom),
       so the metrics strip covered it; moving it inside the cluster puts
       every zoom affordance where shell.css's offsets already aim.
       The % is now a BUTTON and a labelled fit icon sits beside it, both
       for the same reason the web app gave: somebody who had panned into
       empty space had no visible way back, and the honest answer was a
       keyboard shortcut they had not been told about. Clicking the number
       keeps working because someone who learned to click it should be
       right. -->
  <div class="cv-zoom" data-chrome="zoom">
    <button
      type="button"
      class="btn btn-ghost btn-sm"
      aria-label="Zoom out"
      onclick={() => zoomBy(1 / ZOOM_STEP)}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12h14" /></svg>
    </button>
    <button
      type="button"
      class="btn btn-ghost btn-sm cv-zoom-level"
      aria-label={`Zoom ${Math.round(view.k * 100)} percent. Fit the diagram on screen`}
      onclick={fitToContent}>{Math.round(view.k * 100)}%</button>
    <button
      type="button"
      class="btn btn-ghost btn-sm"
      aria-label="Zoom in"
      onclick={() => zoomBy(ZOOM_STEP)}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
    </button>
    <button
      type="button"
      class="btn btn-ghost btn-sm"
      aria-label="Fit the diagram on screen"
      onclick={fitToContent}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 9V5a1 1 0 0 1 1-1h4M15 4h4a1 1 0 0 1 1 1v4M20 15v4a1 1 0 0 1-1 1h-4M9 20H5a1 1 0 0 1-1-1v-4" /></svg>
    </button>
  </div>
</div>

<!-- ==========================================================================
     KNOWN LIMITATIONS relative to src/components/Canvas.tsx and
     src/components/annotationLayout.ts -- see this migration's "Known
     limitations" section (MIGRATION_PLAN.md / README.md) for the
     canonical copy of this list. Kept here too since this is the file the
     gaps live in.

     SELECTION MODEL
       - Multi-select (shift/ctrl additive click, Ctrl+A select-all),
         marquee-select, node rename (double-click) and system clipboard
         copy/cut/paste are all implemented -- see this file's header
         comment and `$lib/state/topology.svelte.ts`'s own header comment
         for the selection-model revision that made them possible.
       - Alt+drag duplicate-while-dragging is NOT implemented (confirmed
         out of scope).
       - Ctrl+D duplicate is NOT implemented (confirmed out of scope; the
         clipboard's own `cloneSubgraph`/`freshId`, `$lib/domain/clipboard.ts`,
         are written generically enough that a future Ctrl+D could reuse
         them unchanged).
        - Group-drag-move of a multi-selection IS implemented for NODES:
          pressing an already-selected member drags the other selected
          nodes plus every selected annotation together
          (`promote()`'s `'node'` case). Pressing a note or a section
          still selects and moves only that annotation -- the web app
          promotes an annotation press into a group too, and wiring that
          would touch `promote()`'s `'section'`/`'note'` cases.
        - Undo/redo is FULLY wired: `promote()` opens one history entry
          per drag (`beginGesture`, before it changes any selection) and
          every way a gesture can end closes it (`closeUndoGesture`), so a
          move/resize is a single Ctrl+Z step while a click costs nothing;
          every OTHER topology write this file performs carries its own
          `sessionHistory.commit`/`touch` with the web's exact label --
          link creation (`finishDrag` `'link'`, `handleClick` and the
          keyboard Enter, 'connection'), edge delete and Delete/Backspace
          and cut ('delete'), note/section creation ('add note'/'add
          section', covering tool click, section draw and palette drop),
          tone cycling ('section shade'), the note/section editors ('note
          edit'/'label edit'), paste ('paste'), palette drop ('add'),
          double-click rename (`touch('rename')`) and the arrow-key nudge
          (`touch('move')`) -- mirroring App.tsx's own commit sites. The
          history itself, the header buttons and Ctrl+Z / Ctrl+Shift+Z /
          Ctrl+Y live in `$lib/state/history.svelte` and
          `routes/+page.svelte`.
        - Zoom-to-fit IS implemented: the cluster's fit button and the %
          button beside it, plus Shift+1 (everything) / Shift+2 (the
          selection), matching what Shortcuts.svelte's View list already
          documents. Ctrl+= / Ctrl+- / Ctrl+0 remain unimplemented even
          though that same dialog lists them.

     PALETTE-DROPPED NODE DEFAULTS
       - A node dropped from the palette gets one generic `NodeConfig`
         (`geometry.ts`'s `defaultNodeConfig`), not the web app's 34
         kind-tuned defaults from `src/sim/presets.ts`'s `defaultConfig`
         (out of this task's scope; ports separately per MIGRATION_PLAN.md
         §5). A freshly dropped autoscaler/breaker/shard/etc. needs its
         config tuned by hand in the Inspector before it behaves
         realistically; it will not crash or misrender in the meantime.

     EDGE LABELS
       - Colliding rate labels ARE de-conflicted: `labelDyById`
         (edge-route.ts) buckets anchors on a 16px grid and staggers the
         second and later label landing in a bucket 10px further down,
         exactly as `Canvas.tsx` did. The break mark and delete button keep
         the un-staggered anchor, so a label's box can overlap them.

     ANNOTATIONS (vs. annotationLayout.ts / Canvas.tsx's annotation chrome)
       - Section resize supports all 8 compass handles (the pure
         `resizeRect`/`handleAnchor` math is faithfully ported and does the
         real work here), but the section SHADE PICKER is a single swatch
         that cycles through the 13 tones on click, not the original's row
         of 13 individually-clickable swatches.
       - Note resize supports only the two SIDE handles (width reflow).
         The original's four CORNER handles, which scale font size and
         width together while preserving line breaks, are not implemented;
         a note can be repositioned and reflowed but not scaled up/down as
         a unit. `NOTE_MIN_SCALE`/`NOTE_MAX_SCALE`/the `scale` field are
         still respected for ANY note that already carries a `scale` (e.g.
         loaded from a saved/shared design) -- only the interactive corner
         drag that WRITES a new scale is missing.
       - The note editor is a plain HTML `<textarea>` overlay: commits on
         blur or Ctrl/Cmd+Enter, cancels on Escape. The original's
         bold/italic/underline/font/tone toolbar and Tab-to-indent
         (`applyTab` in annotation-layout.ts, ported but unused here) are
         not wired to any UI control in this pass -- an imported design
         that already has `bold`/`italic`/`font`/`tone`/`underline` set
         still renders them correctly; there is just no in-app way yet to
         SET those fields from a fresh note.
       - Dragging a node into a section's bounds does not highlight the
         section as a drop target (`sectionAtPoint`'s highlight in the web
         app); sections remain purely spatial (unaffected functionally --
         a node dropped inside a section's bounds still reads as "in" it
         the same way, there is just no highlight while dragging).

     RENDERING
       - Per-node sparklines are sampled into a ring buffer owned by this
         component (mirroring the web app's own architecture note that
         "the ring buffer lives in the UI"), capped at `SPARK_LEN` (60)
         samples and reset when a node is removed. There is no persistence
         across a full page reload (matching the web app: the buffer is
         never saved).
       - `readoutFor` (geometry.ts) is a faithful field-for-field port of
         all 33 per-kind cases. `sourceBacklogs` is faithfully ported too,
         but its `pullsFromQueues`/`buffersForConsumers` kind sets are
         HARDCODED (`worker`+`transcoder` pull, `queue` buffers) rather than
         read from a live `behaviourFor()` lookup, because that lookup is a
         TS simulation-engine concept and the desktop app's engine runs in
         Rust with no equivalent exposed to the frontend. See the doc
         comment on `BACKLOG_CONSUMER_KINDS`/`BACKLOG_BUFFER_KINDS` in
         geometry.ts.
        - Zoom fit (`fitTo`/`fitToContent`, Shift+1 / Shift+2 / the cluster
          buttons) is implemented, but it measures NODES only -- exactly as
          the web app's `fitTo(nodes)` does, so a note or section outside
          the node bounding box can still be framed off screen. It also
          frames the `.stage-safe` sentinel when that div exists
          (`visibleFrame`), which `+page.svelte` does not render yet; with
          no sentinel the whole surface is used. The view still starts at a
          fixed `{x:40,y:40,k:1}` and is otherwise only moved by the user
          (pan/zoom/pinch), the Minimap's `ongoto`, or a fit.
       - Background grid lines (app.css's `--grid-line`/`--grid-major`
         tokens) are painted via a CSS `background-image` on `.cv-surface`
       in Canvas.css rather than as SVG rules; visually equivalent, cheaper
       to keep in sync with pan/zoom via CSS custom properties.
   ========================================================================== -->
