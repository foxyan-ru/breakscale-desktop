<script lang="ts">
  /* ==========================================================================
     The app shell: top bar, left rail, canvas, right panel, bottom strip,
     and every overlay panel, wired to the finished stores/api modules and
     the 19 finished components. See the parent project's `MIGRATION_PLAN.md` and
     `AGENTS.md` for the wider context; see this file's own comments below
     for the integration decisions specific to wiring it all together.

     OVERLAY MODEL. `uiStore.activeView` (`'canvas' | 'glossary' | 'designs'
     | 'settings' | 'sysdesign'`) is the finished store's real enum, and
     Glossary is actually mounted in `+layout.svelte` (see that file for
     why). Three more panels this app needs -- Examples, Shortcuts and
     Challenges -- have no slot in that enum at all (built in parallel, by
     an agent who did not know about the other two). Rather than touch
     `ui.svelte.ts` (finished/do-not-modify), those three live in a small
     local `overlay` field here, and `showOverlay`/`closeOverlay` below keep
     the two mutually exclusive so at most one panel is ever open, matching
     the brief's "show at most one at a time".
     ========================================================================== */

  import { onMount } from 'svelte';

  import {
    topologyStore,
    addNode,
    select,
    updateNodeConfig,
    setTopology,
  } from '$lib/state/topology.svelte';
  import { simulationStore, setRunning } from '$lib/state/simulation.svelte';
  import { settingsStore } from '$lib/state/settings.svelte';
  import { uiStore, pushError } from '$lib/state/ui.svelte';
  import type { ActiveView } from '$lib/state/ui.svelte';

  import { simNew, simReset, simStep } from '$lib/api/sim';
  import { presetsList, presetLoad } from '$lib/api/presets';
  import {
    challengesList,
    challengeStart,
    evaluateChallenge,
  } from '$lib/api/challenges';
  import type {
    Challenge as ChallengeData,
    ChallengeSummary,
    ChallengeResult,
  } from '$lib/api/challenges';
  import { isAppError } from '$lib/api';
  import type { NodeKind, SystemStats, Topology } from '$lib/domain';

  import Canvas from '$lib/components/canvas/Canvas.svelte';
  import { makeNode } from '$lib/components/canvas/geometry';
  import Palette from '$lib/components/palette/Palette.svelte';
  import type { AnnotationTool } from '$lib/components/palette/Palette.svelte';
  import Inspector from '$lib/components/inspector/Inspector.svelte';
  import VendorPanel from '$lib/components/vendor/VendorPanel.svelte';
  import Metrics from '$lib/components/metrics/Metrics.svelte';
  import MainMenu from '$lib/components/shell/MainMenu.svelte';
  import type { MenuItem } from '$lib/components/shell/MainMenu.svelte';
  import TrafficControl from '$lib/components/shell/TrafficControl.svelte';
  import Designs from '$lib/components/shell/Designs.svelte';
  import Examples from '$lib/components/shell/Examples.svelte';
  import type { PresetSummary } from '$lib/components/shell/Examples.svelte';
  import Settings from '$lib/components/shell/Settings.svelte';
  import Shortcuts from '$lib/components/shell/Shortcuts.svelte';
  import Challenges from '$lib/components/challenges/Challenges.svelte';
  import ChallengePanel from '$lib/components/challenges/Challenge.svelte';
  import ArchitectureEditor from '$lib/components/sysdesign/ArchitectureEditor.svelte';
  import LowLevelEditor from '$lib/components/sysdesign/LowLevelEditor.svelte';
  import ExportPanel from '$lib/components/sysdesign/ExportPanel.svelte';

  function describeErr(e: unknown): string {
    if (isAppError(e)) return e.message;
    return e instanceof Error ? e.message : String(e);
  }

  /* ------------------------------------------------------------------ *
   * Overlay model -- see header comment.
   * ------------------------------------------------------------------ */

  type LocalOverlay = 'examples' | 'shortcuts' | 'challenges' | null;
  let overlay: LocalOverlay = $state(null);

  /** The name to suggest in the Designs dialog's "name this design" field --
   *  whatever example or challenge is currently loaded, if any. */
  let loadedName = $state('');

  function showOverlay(kind: ActiveView | 'examples' | 'shortcuts' | 'challenges'): void {
    if (kind === 'examples' || kind === 'shortcuts' || kind === 'challenges') {
      uiStore.activeView = 'canvas';
      overlay = kind;
      if (kind === 'examples') void ensureExamplesLoaded();
      if (kind === 'challenges') void ensureChallengesLoaded();
    } else {
      overlay = null;
      uiStore.activeView = kind;
    }
  }

  function closeOverlay(): void {
    overlay = null;
    uiStore.activeView = 'canvas';
  }

  /* ------------------------------------------------------------------ *
   * Main menu. Labels, icon paths and hints copied verbatim from the web
   * app's own `App.tsx` menu (`src/App.tsx`'s `menuItems`), so the port
   * reads as the same app; "System design" is new (MIGRATION_PLAN.md §8
   * has no web-app precedent to copy from), given a plain icon in the same
   * stroke style as the rest.
   * ------------------------------------------------------------------ */

  let menuOpen = $state(false);

  const menuItems: MenuItem[] = [
    {
      label: 'Your designs',
      icon: 'M4 4a2 2 0 0 1 2-2h7l5 5v13a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2zM13 2v5h5M9 13h6M9 17h6',
      onSelect: () => showOverlay('designs'),
    },
    {
      label: 'Challenges',
      icon: 'M6 9H4.5a2.5 2.5 0 0 1 0-5H6M18 9h1.5a2.5 2.5 0 0 0 0-5H18M4 22h16M10 14.66V17c0 .55-.47.98-.97 1.21C7.85 18.75 7 20.24 7 22M14 14.66V17c0 .55.47.98.97 1.21C16.15 18.75 17 20.24 17 22M18 2H6v7a6 6 0 0 0 12 0z',
      onSelect: () => showOverlay('challenges'),
    },
    {
      label: 'Examples',
      icon: 'M3 4a1 1 0 0 1 1-1h6v7H3zM14 3h6a1 1 0 0 1 1 1v5h-7zM3 13h7v8H4a1 1 0 0 1-1-1zM14 13h7v7a1 1 0 0 1-1 1h-6z',
      onSelect: () => showOverlay('examples'),
    },
    {
      label: 'System design',
      icon: 'M12 2 2 7l10 5 10-5zM2 17l10 5 10-5M2 12l10 5 10-5',
      onSelect: () => showOverlay('sysdesign'),
    },
    {
      label: 'Glossary',
      icon: 'M4 19.5A2.5 2.5 0 0 1 6.5 17H20M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z',
      hint: '?',
      onSelect: () => showOverlay('glossary'),
    },
    {
      label: 'Keyboard shortcuts',
      icon: 'M2 5.5A2.5 2.5 0 0 1 4.5 3h15A2.5 2.5 0 0 1 22 5.5v13a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 18.5zM6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M18 13h.01M9 13h6M7 17h10',
      hint: 'Ctrl+/',
      onSelect: () => showOverlay('shortcuts'),
    },
    {
      label: 'Settings',
      icon: 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z',
      onSelect: () => showOverlay('settings'),
    },
  ];

  /* ------------------------------------------------------------------ *
   * Canvas selection -> Inspector / VendorPanel.
   * ------------------------------------------------------------------ */

  const selectedNode = $derived(
    topologyStore.topology.nodes.find((n) => n.id === topologyStore.selectedNodeId) ?? null,
  );

  /* ------------------------------------------------------------------ *
   * Top bar: offered load, readouts, transport.
   *
   * TrafficControl in `shell/` is a pure view -- every value crosses as a
   * prop and every action as a callback -- so the derivations and the
   * write paths live here, exactly where the web app keeps them in
   * `App.tsx` (lines 2445-2512 and 1810-1840). See that file's comments
   * for WHY each of these is derived rather than stored; the short version
   * is that topology is the single source of truth for the load figure
   * (add/delete paths then cannot forget to reconcile a mirrored copy),
   * and `failuresByReason` is a LIFETIME count that has to be
   * differenced against sim time to become the per-second "Dropped" it
   * claims to be.
   * ------------------------------------------------------------------ */

  /** Shown for the single frame before the first snapshot exists. */
  const EMPTY_SYSTEM: SystemStats = {
    timeMs: 0,
    offeredRps: 0,
    goodputRps: 0,
    errorRate: 0,
    p50: 0,
    p95: 0,
    p99: 0,
    totalRequests: 0,
    totalFailed: 0,
  };

  const clients = $derived(
    topologyStore.topology.nodes.filter((n) => n.kind === 'client'),
  );

  /** The header's offered load: the sum across clients, so multi-client presets add up. */
  const offeredRps = $derived(clients.reduce((sum, c) => sum + c.config.rps, 0));

  const hasTrafficSource = $derived(clients.length > 0);

  const system = $derived<SystemStats>(simulationStore.snapshot?.system ?? EMPTY_SYSTEM);

  const cumulativeLost = $derived(
    Object.values(simulationStore.snapshot?.failuresByReason ?? {}).reduce(
      (sum, v) => (Number.isFinite(v) ? sum + v : sum),
      0,
    ),
  );

  const simTimeMs = $derived(simulationStore.snapshot?.system.timeMs ?? 0);

  /* The backing values for the lost-per-second derivation. Deliberately
     NOT `$state`: the effect below reads `cumulativeLost`/`simTimeMs` and
     must be free to write these without re-triggering itself, and
     `handleReset` has to zero them SYNCHRONOUSLY -- leaving it to the
     effect meant the old run's "Dropped 104k/s" sat on screen next to p99
     0ms until the next snapshot, which is forever while paused. */
  let lostPrev: number | null = null;
  let lostPrevTimeMs = 0;
  let lostRps = $state(0);

  $effect(() => {
    if (!Number.isFinite(simTimeMs)) return;
    const prev = lostPrev;
    const dtMs = simTimeMs - lostPrevTimeMs;

    // First sample, or a reset (sim time or the counter moved backwards):
    // adopt the count as the new baseline and report nothing this frame.
    if (prev === null || dtMs < 0 || cumulativeLost < prev) {
      lostPrev = cumulativeLost;
      lostPrevTimeMs = simTimeMs;
      lostRps = 0;
      return;
    }

    // Sample no faster than 250ms of sim time: below that the divisor is
    // tiny and the quotient is mostly quantisation noise.
    if (dtMs < 250) return;

    const delta = cumulativeLost - prev;
    lostPrev = cumulativeLost;
    lostPrevTimeMs = simTimeMs;
    lostRps = delta > 0 ? (delta * 1000) / dtMs : 0;
  });

  function resetLostRate(): void {
    lostPrev = null;
    lostPrevTimeMs = 0;
    lostRps = 0;
  }

  /**
   * The top-bar slider sets the TOTAL offered load. One client gets the
   * value outright; several are scaled proportionally so a preset's
   * deliberate traffic mix survives the drag, with the remainder placed on
   * the first client so the distributed parts always sum to exactly
   * `next`. Transcribed from `App.tsx`'s `handleRpsChange`.
   *
   * Each write goes through `updateNodeConfig`, which updates the store
   * optimistically and pushes to the Rust engine -- so a multi-client drag
   * costs one IPC per client rather than one per pixel, because
   * TrafficControl commits on pointer-up (see its header comment).
   */
  function handleRpsChange(next: number): void {
    if (clients.length === 0) return;
    if (clients.length === 1) {
      updateNodeConfig(clients[0].id, { rps: next });
      return;
    }
    const total = clients.reduce((s, c) => s + c.config.rps, 0);
    const shares = clients.map((c) =>
      Math.max(0, Math.round(next * (total > 0 ? c.config.rps / total : 1 / clients.length))),
    );
    const spread = shares.reduce((s, v) => s + v, 0);
    shares[0] = Math.max(0, shares[0] + (next - spread));
    clients.forEach((c, i) => updateNodeConfig(c.id, { rps: shares[i] }));
  }

  /* ------------------------------------------------------------------ *
   * Palette -> Canvas.
   *
   * Canvas.svelte takes no props at all (it reads every store directly and
   * owns its own view transform), so "add this kind" has to become a store
   * mutation the canvas will simply render on its next tick, rather than a
   * prop or callback into it.
   * ------------------------------------------------------------------ */

  function handlePaletteAdd(kind: NodeKind): void {
    const n = topologyStore.topology.nodes.length;
    const x = 160 + (n % 5) * 220;
    const y = 160 + Math.floor(n / 5) * 140;
    const node = makeNode(kind, x, y);
    addNode(node);
    select(node.id, null);
  }

  /**
   * Palette's "Annotate" row offers a click-to-arm path for the note/section
   * tools as well as drag-and-drop (`ANN_DND_MIME`, which Canvas.svelte's own
   * drop handler already understands with no wiring needed here). The
   * click-to-arm path has no home, though: the armed tool
   * (`'note' | 'section' | null`) is Canvas.svelte's own local `$state`,
   * never exposed as a prop or export, and Palette's `armedTool` prop has
   * nothing to read it back from either -- so the toolbar row can show
   * "armed" today only via Canvas's OWN 'n'/'B' keyboard shortcut, not via a
   * value this page can see.
   *
   * Canvas.svelte's key handler (its own `onMount`) already arms exactly
   * this way on a plain 'n'/'b' keydown. Synthesizing that same event is a
   * small, honest bridge rather than a hack around a missing API: it reuses
   * Canvas's own documented toggle logic verbatim instead of re-implementing
   * it here against state this file cannot reach. What is genuinely missing
   * is the round trip back (Palette's row never shows `is-armed`, and
   * `armedTool` is therefore never passed below) -- noted in this task's
   * report rather than solved by adding a prop to a finished component.
   */
  function handlePaletteAnnotation(tool: AnnotationTool): void {
    window.dispatchEvent(new KeyboardEvent('keydown', { key: tool === 'note' ? 'n' : 'b' }));
  }

  /* ------------------------------------------------------------------ *
   * Examples.
   * ------------------------------------------------------------------ */

  let examplesList: PresetSummary[] = $state([]);
  let examplesLoading = $state(false);
  let examplesLoaded = $state(false);
  let activePresetId: string | null = $state(null);

  /**
   * Cap how long a preset IPC call may take before its caller treats it as
   * failed. Both commands are synchronous lookups over compile-time
   * embedded data, so a real answer arrives immediately; the only way to
   * take seconds is for the call to never settle at all (a native side
   * that panicked without answering the invoke). Without this, such a call
   * would leave the Examples spinner turning forever -- and in the startup
   * path it would block `sim_new` outright, leaving the engine unloaded.
   * Timing out turns a silent hang into the same visible, retryable error
   * a rejection already produces.
   */
  const PRESET_FETCH_TIMEOUT_MS = 3000;

  function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`${label} did not answer within ${PRESET_FETCH_TIMEOUT_MS}ms.`)),
        PRESET_FETCH_TIMEOUT_MS,
      );
      promise.then(
        (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        (error) => {
          clearTimeout(timer);
          reject(error);
        },
      );
    });
  }

  async function ensureExamplesLoaded(): Promise<void> {
    if (examplesLoaded) return;
    examplesLoading = true;
    try {
      examplesList = await withTimeout(presetsList(), 'presets_list');
      examplesLoaded = true;
    } catch (e) {
      pushError(`Loading examples failed: ${describeErr(e)}`);
    } finally {
      examplesLoading = false;
    }
  }

  async function handleLoadExample(id: string): Promise<void> {
    try {
      const preset = await withTimeout(presetLoad(id), 'preset_load');
      setTopology(preset.topology);
      activePresetId = preset.id;
      loadedName = preset.name;
      activeChallenge = null;
    } catch (e) {
      pushError(`Loading the example failed: ${describeErr(e)}`);
    }
  }

  /* ------------------------------------------------------------------ *
   * Challenges.
   * ------------------------------------------------------------------ */

  let challengeSummaries: ChallengeSummary[] = $state([]);
  let challengesLoading = $state(false);
  let challengesLoaded = $state(false);
  let activeChallenge: ChallengeData | null = $state(null);

  async function ensureChallengesLoaded(): Promise<void> {
    if (challengesLoaded) return;
    challengesLoading = true;
    try {
      challengeSummaries = await challengesList();
      challengesLoaded = true;
    } catch (e) {
      pushError(`Loading challenges failed: ${describeErr(e)}`);
    } finally {
      challengesLoading = false;
    }
  }

  async function handleStartChallenge(id: string): Promise<void> {
    try {
      const result = await challengeStart(id);
      setTopology(result.topology);
      activeChallenge = result.challenge;
      activePresetId = null;
      loadedName = result.challenge.name;
    } catch (e) {
      pushError(`Starting the challenge failed: ${describeErr(e)}`);
    }
  }

  function handleGiveUpChallenge(): void {
    activeChallenge = null;
  }

  const challengeResult = $derived<ChallengeResult | null>(
    activeChallenge && simulationStore.snapshot
      ? evaluateChallenge(activeChallenge, simulationStore.snapshot)
      : null,
  );

  /* ------------------------------------------------------------------ *
   * System design overlay: ArchitectureEditor / LowLevelEditor / ExportPanel
   * bundled into one tabbed view. All three are prop-less and fully
   * self-contained via `sysdesignStore`, so there is nothing to wire beyond
   * the tab chrome and this view's own open/close + focus management --
   * unlike the dialogs in `shell/*.svelte`, these three were built as plain
   * embeddable panels with no `open`/`onClose` of their own.
   * ------------------------------------------------------------------ */

  type SysTab = 'architecture' | 'lowlevel' | 'export';
  let sysTab: SysTab = $state('architecture');
  let sysdesignCardEl: HTMLDivElement | undefined = $state(undefined);

  $effect(() => {
    if (uiStore.activeView !== 'sysdesign') return;
    sysTab = 'architecture';
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const card = sysdesignCardEl;
    card?.focus();
    return () => {
      const active = document.activeElement;
      if (!active || active === document.body || card?.contains(active)) {
        opener?.focus();
      }
    };
  });

  function onSysdesignKeyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeOverlay();
    }
  }

  /* ------------------------------------------------------------------ *
   * Bootstrap: start the simulation engine the moment the page renders.
   *
   * The Rust side already installed a default engine (the first built-in
   * example) in `lib.rs`'s `setup()` before this runs, so commands work
   * and snapshots flow no matter what; what this block adds is the
   * CANVAS's copy of that topology plus the examples list. `topologyStore`
   * initializes with a blank `Topology` (see `state/topology.svelte.ts`),
   * and `presets_list`/`preset_load` (wrapped in `$lib/api/presets.ts`)
   * load exactly as if the reader had opened Examples and picked the top
   * entry, so the instant the window appears canvas and engine agree on a
   * working system with traffic flowing. Falls back to the blank topology
   * the store already holds -- still handed to `sim_new` below -- if the
   * presets command is unreachable or the fetch times out.
   * ------------------------------------------------------------------ */

  function isTypingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }

  /* ------------------------------------------------------------------ *
   * Where the bar actually ends, so everything that must clear it can.
   *
   * `--bar-clear` was a constant (shell.css still declares 80px as the
   * first-paint value). It held while the bar was one row of a known
   * height: 12px offset + 56px island + 12px gap. It stopped holding the
   * moment the bar's contents changed -- the traffic island is far taller
   * than the two text buttons it replaced, and on a narrow window the bar
   * can wrap -- and a constant that has to be re-guessed every time the
   * bar's contents change is a number that will be wrong again. Only the
   * element knows, so it is measured, exactly as `App.tsx:668-718` does.
   *
   * This is also what keeps `.ins-close` clickable: the slots (and the
   * inspector's close button inside the right slot) sit at
   * `top: var(--bar-clear)` under a bar at z-index 40, so an under-measured
   * clearance puts the islands on top of the button and the clicks land on
   * the bar instead.
   * ------------------------------------------------------------------ */

  let barEl: HTMLElement | undefined = $state(undefined);
  let barBottom = $state<number | null>(null);

  /** The gap the bar floats in below, plus its offset, is read from CSS. */
  const BAR_GAP_PX = 12;

  /** Falls back to the stylesheet constant until the first measurement lands. */
  const barClearStyle = $derived(
    barBottom === null ? '' : `--bar-clear: ${barBottom + BAR_GAP_PX}px`,
  );

  function measureBar(): void {
    const el = barEl;
    if (!el) return;
    /* The bar's BOTTOM edge, not its height: the bar floats, so its height
       alone is short by the --sp-3 offset and the panels would start flush
       against it with no gap at all. Reading the bottom of the rect
       includes whatever the offset happens to be. */
    barBottom = Math.round(el.getBoundingClientRect().bottom);
  }

  function onWindowKeyDown(e: KeyboardEvent): void {
    if (isTypingTarget(e.target)) return;

    // Escape: Canvas.svelte owns clearing the canvas selection/tool itself
    // (its own window keydown listener), and every `shell/*.svelte` dialog
    // already closes itself on Escape via its own handler (which calls
    // `stopPropagation`, so it never reaches here). The one overlay this
    // file builds by hand -- the system-design view, which has no such
    // handler of its own -- is the one case left to cover.
    if (e.key === 'Escape' && uiStore.activeView === 'sysdesign') {
      closeOverlay();
      return;
    }

    if (e.ctrlKey && !e.metaKey && !e.altKey && e.key === '/') {
      e.preventDefault();
      showOverlay('shortcuts');
      return;
    }

    if (!e.ctrlKey && !e.metaKey && !e.altKey) {
      if (e.key === '?') {
        e.preventDefault();
        showOverlay('glossary');
        return;
      }
      if (e.key === 'c' || e.key === 'C') {
        e.preventDefault();
        uiStore.library = !uiStore.library;
        return;
      }
      if (e.key === 'm' || e.key === 'M') {
        e.preventDefault();
        uiStore.metrics = !uiStore.metrics;
        return;
      }
      if (e.key === ' ') {
        e.preventDefault();
        setRunning(!simulationStore.running);
        return;
      }
      // Step one tick. Ignored with a modifier held (the block above
      // already guarantees that) so it cannot shadow browser shortcuts
      // like Ctrl/Cmd+S. Mirrors App.tsx; the table in
      // `shell/Shortcuts.svelte` has printed this binding since it was
      // ported, so until now it advertised a key nothing handled.
      if (e.key === 's' || e.key === 'S') {
        e.preventDefault();
        handleStep();
        return;
      }
    }
  }

  onMount(() => {
    window.addEventListener('keydown', onWindowKeyDown);

    // Bar measurement (see the block above `onWindowKeyDown`). jsdom has no
    // ResizeObserver, so the guard keeps the mount path testable; without
    // one, the stylesheet's constant simply stands in.
    measureBar();
    let ro: ResizeObserver | null = null;
    if (barEl && typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(() => measureBar());
      ro.observe(barEl);
    }
    /* A mobile webview's chrome collapsing resizes the VIEWPORT without
       resizing the bar, so the ResizeObserver never fires while everything
       measured against the window shifts underneath it. visualViewport is
       the event that reports it; `resize` covers the rest. */
    const vv = window.visualViewport;
    vv?.addEventListener('resize', measureBar);
    window.addEventListener('resize', measureBar);

    // `src-tauri/src/lib.rs`'s `setup()` installs a default engine (the
    // first built-in example) and starts the tick thread with it BEFORE
    // the webview can invoke anything, so the simulation is already
    // running by the time this component mounts -- mirror that locally so
    // the Play/Pause button reads correctly on first paint instead of
    // claiming paused. The bootstrap below then swaps in its own copy of
    // the same preset, so canvas and engine stay on the same topology.
    simulationStore.running = true;

    void (async () => {
      let topology: Topology = topologyStore.topology;
      try {
        const list = await withTimeout(presetsList(), 'presets_list');
        examplesList = list;
        examplesLoaded = true;
        if (list.length > 0) {
          const preset = await withTimeout(presetLoad(list[0].id), 'preset_load');
          topology = preset.topology;
          activePresetId = preset.id;
          loadedName = preset.name;
        }
      } catch {
        // No examples reachable (command unreachable, or the fetch timed
        // out) -- fall back to the blank topology the store already
        // initializes with; `sim_new` below still runs so the engine holds
        // exactly what the canvas shows. Not surfaced as an error: a blank
        // canvas is a legitimate starting state, not a failure.
      }

      topologyStore.topology = topology;
      try {
        await simNew(topology);
      } catch (e) {
        pushError(`Starting the simulation failed: ${describeErr(e)}`);
      }
    })();

    return () => {
      window.removeEventListener('keydown', onWindowKeyDown);
      ro?.disconnect();
      vv?.removeEventListener('resize', measureBar);
      window.removeEventListener('resize', measureBar);
    };
  });

  /* ------------------------------------------------------------------ *
   * Transport: play/pause, step, reset. The three icon buttons inside
   * TrafficControl call straight into these.
   *
   * Step has no web-app equivalent to copy at THIS level because the web
   * engine is in-process and stepped inline; here it is a command, and
   * `sim_step` performs the same pause-then-advance contract in Rust so
   * the delta cannot race the tick thread. Mirrors `App.tsx`'s
   * `handleStep` ("a step always pauses first -- the same contract a
   * debugger's step button has").
   * ------------------------------------------------------------------ */

  function handleToggleRun(): void {
    setRunning(!simulationStore.running);
  }

  function handleStep(): void {
    // Pause first, locally, so the button reflects the stopped clock at
    // once; the command pauses again on the Rust side before advancing.
    if (simulationStore.running) setRunning(false);
    simStep().catch((e) => pushError(`Stepping the simulation failed: ${describeErr(e)}`));
  }

  function handleReset(): void {
    // Synchronously, not via the derivation effect: Reset must never leave
    // the previous run's Dropped figure standing beside a zeroed clock.
    resetLostRate();
    simReset().catch((e) => pushError(`Resetting the simulation failed: ${describeErr(e)}`));
  }
</script>

<svelte:head>
  <title>Breakscale</title>
</svelte:head>

<div class="app">
  <header class="app-bar" bind:this={barEl}>
    <div class="app-island app-island-brand">
      <div class="app-brand">
        <h1 class="app-title">Breakscale</h1>
        <p class="app-tagline">Build it, load it, watch it break</p>
      </div>
    </div>

    <!-- The web app's `.app-island-load` (App.tsx ~2606): the load slider
         and its readouts, the ONE control a student drives. This island
         used to hold two text buttons (Play/Pause/Reset) instead -- a
         stand-in until the control itself was ported -- which is why the
         bar was 56px tall, why `--bar-clear`'s constant happened to fit,
         and why the transport and every headline number were missing from
         the shell. -->
    <div class="app-island app-island-load">
      <TrafficControl
        rps={offeredRps}
        onRpsChange={handleRpsChange}
        running={simulationStore.running}
        onToggleRun={handleToggleRun}
        onStep={handleStep}
        onReset={handleReset}
        system={system}
        lost={lostRps}
        empty={topologyStore.topology.nodes.length === 0}
        noTrafficSource={!hasTrafficSource}
      />
    </div>

    <div class="app-island app-island-menu">
      <div class="app-menu-wrap">
        <button
          type="button"
          class="btn app-menu-btn"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          onclick={() => (menuOpen = !menuOpen)}
        >
          Menu
        </button>
        <MainMenu open={menuOpen} onClose={() => (menuOpen = false)} items={menuItems} />
      </div>
    </div>
  </header>

  <div
    class="app-body"
    style={barClearStyle}
    class:has-library={uiStore.library}
    class:has-inspector={Boolean(selectedNode)}
    class:has-metrics={uiStore.metrics}
  >
    {#if uiStore.library}
      <nav class="app-slot app-slot-left" aria-label="Components">
        <Palette onAdd={handlePaletteAdd} onAddAnnotation={handlePaletteAnnotation} />
      </nav>
    {/if}

    <main class="app-stage">
      <div class="stage-canvas">
        <Canvas />

        {#if activeChallenge && challengeResult}
          <ChallengePanel
            challenge={activeChallenge}
            result={challengeResult}
            onGiveUp={handleGiveUpChallenge}
          />
        {/if}

        <button
          type="button"
          class="btn btn-icon stage-toggle stage-toggle-library"
          aria-pressed={uiStore.library}
          aria-label={uiStore.library ? 'Hide components' : 'Show components'}
          onclick={() => (uiStore.library = !uiStore.library)}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d={uiStore.library ? 'M15 18l-6-6 6-6' : 'M9 18l6-6-6-6'} />
          </svg>
        </button>

        <button
          type="button"
          class="btn btn-icon stage-toggle stage-toggle-metrics"
          aria-pressed={uiStore.metrics}
          aria-label={uiStore.metrics ? 'Hide charts' : 'Show charts'}
          onclick={() => (uiStore.metrics = !uiStore.metrics)}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d={uiStore.metrics ? 'M6 15l6-6 6 6' : 'M6 9l6 6 6-6'} />
          </svg>
        </button>
      </div>

      {#if uiStore.metrics}
        <section class="app-slot app-slot-bottom" aria-label="Charts">
          <Metrics />
        </section>
      {/if}
    </main>

    {#if selectedNode}
      {@const node = selectedNode}
      <aside class="app-slot app-slot-right" aria-label="Inspector">
        <Inspector />
        {#if settingsStore.vendor !== 'generic'}
          <div class="panel vp-wrap">
            <VendorPanel
              nodeId={node.id}
              kind={node.kind}
              config={node.config}
              onChange={(patch) => updateNodeConfig(node.id, patch)}
            />
          </div>
        {/if}
      </aside>
    {/if}
  </div>
</div>

<Designs open={uiStore.activeView === 'designs'} onClose={closeOverlay} suggestedName={loadedName} />
<Settings open={uiStore.activeView === 'settings'} onClose={closeOverlay} />
<Shortcuts open={overlay === 'shortcuts'} onClose={closeOverlay} />
<Examples
  open={overlay === 'examples'}
  onClose={closeOverlay}
  presets={examplesList}
  loading={examplesLoading}
  activePresetId={activePresetId}
  onLoad={handleLoadExample}
/>
<Challenges
  open={overlay === 'challenges'}
  onClose={closeOverlay}
  challenges={challengeSummaries}
  loading={challengesLoading}
  onStart={handleStartChallenge}
/>

{#if uiStore.activeView === 'sysdesign'}
  <div class="sd-root">
    <div class="sd-scrim" onclick={closeOverlay} aria-hidden="true"></div>
    <div
      bind:this={sysdesignCardEl}
      class="sd-card panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="sd-title"
      tabindex="-1"
      onkeydown={onSysdesignKeyDown}
    >
      <header class="sd-head">
        <h2 id="sd-title" class="sd-title">System design</h2>
        <button type="button" class="btn btn-ghost btn-sm btn-icon" onclick={closeOverlay} aria-label="Close system design">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
            <path d="M18 6 6 18M6 6l12 12" />
          </svg>
        </button>
      </header>

      <div class="sd-tabs" role="tablist" aria-label="System design sections">
        <button type="button" class="btn btn-sm" role="tab" aria-selected={sysTab === 'architecture'} onclick={() => (sysTab = 'architecture')}>
          Architecture
        </button>
        <button type="button" class="btn btn-sm" role="tab" aria-selected={sysTab === 'lowlevel'} onclick={() => (sysTab = 'lowlevel')}>
          Low-level design
        </button>
        <button type="button" class="btn btn-sm" role="tab" aria-selected={sysTab === 'export'} onclick={() => (sysTab = 'export')}>
          Export
        </button>
      </div>

      <div class="sd-body scroll">
        {#if sysTab === 'architecture'}
          <ArchitectureEditor />
        {:else if sysTab === 'lowlevel'}
          <LowLevelEditor />
        {:else}
          <ExportPanel />
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* This file's own layout glue: the pieces `shell/shell.css` does not
     already define because they belong to decisions made here, not to the
     shell's generic grid (rail/stage/strip geometry, panel slots, edge
     toggles -- all already styled by the import in `+layout.svelte`). */

  /* (The old `.app-island-sim` rule lived here. The sim island is gone:
     its transport moved into `shell/TrafficControl.svelte`, and the island
     itself is now shell.css's own `.app-island-load`, which carries the
     sizing. Leaving the rule behind would only earn an unused-selector
     warning from svelte-check.) */

  /* `.ins-panel` (Inspector's own root class) paints its own full card --
     border, background, shadow -- via the shared `.panel` primitive, so
     this slot only supplies the flex sizing shell.css's own
     `.app-slot-right > .ins` rule would have (Inspector's root is
     `.ins-panel`, not `.ins`; see this task's report for the drift). */
  .app-slot-right {
    flex-direction: column;
    gap: var(--sp-3);
    overflow-y: auto;
  }

  .app-slot-right :global(.ins-panel) {
    flex: 1 1 auto;
    min-width: 0;
  }

  /* VendorPanel's own `.vp` styles itself to sit INSIDE another panel's
     padding (a top rule, no border of its own -- see its header comment,
     "meant to be embedded in the ... node inspector"). Inspector is
     finished/do-not-modify, so it cannot literally be injected inside it;
     this gives it an equivalent small card of its own instead. */
  .vp-wrap {
    flex: none;
    padding: var(--sp-4);
  }

  /* -------------------------------------------------------------------
     System design overlay. Same dialog shell shape as `shell/Settings.svelte`
     et al. (scrim + centred card), written fresh here because the three
     sysdesign/* components are plain embeddable panels with no dialog
     chrome of their own to reuse (see this file's script comment).
     ------------------------------------------------------------------- */

  .sd-root {
    position: fixed;
    inset: 0;
    z-index: 500;
    display: grid;
    place-items: center;
    padding: var(--sp-5);
  }

  .sd-scrim {
    position: absolute;
    inset: 0;
    background: var(--scrim);
    cursor: default;
  }

  /* Viewport math instead of `100%`: this card is a `place-items: center`
     (fit-content) grid item, so a percentage max-height has no guaranteed
     definite base. Unclamped, the card would run past the viewport, which
     `html, body { overflow: clip }` makes unreachable. Same number as
     `min(720px, 100%)` when the percentage does resolve. Kept OUTSIDE the
     rule below: the scroll-contract test extracts this rule as selector
     through its first closing brace, and this comment contains braces. */
  .sd-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    width: min(900px, 100%);
    max-height: min(720px, calc(100vh - var(--sp-5) * 2));
    min-height: 0;
    padding: var(--sp-5);
    outline: none;
  }

  .sd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .sd-title {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-med);
    color: var(--text);
  }

  .sd-tabs {
    display: flex;
    gap: var(--sp-2);
    flex: none;
  }

  .sd-tabs .btn[aria-selected='true'] {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent-ink);
  }

  /* The dialog's ONE scroll container: content past the card's max-height
     lands here, and the wheel reaches it because the sysdesign/* roots are
     plain auto-height blocks (a nested `.scroll` that cannot itself overflow
     would stop the wheel via overscroll-behavior: contain). */
  .sd-body {
    min-height: 0;
    overflow-y: auto;
  }
</style>
