/**
 * Undo / redo for the desktop shell -- a port of the web app's
 * `src/history.ts` (`SessionHistory`) plus the `App.tsx` wiring that drives
 * it. Read that file first: the four semantics it documents (one entry per
 * gesture, selection travelling WITH history but never touching it, no-op
 * entries skipped, canUndo/canRedo derived on every read) are the contract,
 * and every one of them is kept here.
 *
 * WHY FULL SNAPSHOTS, NOT Deltas -- unchanged from the web: a topology is a
 * few dozen plain-data objects, so a complete copy per entry is a few KB,
 * and the snapshot form is the one that is obviously correct.
 *
 * WHAT CHANGED FOR THE DESKTOP, and why:
 *
 *   - The snapshot is `{ topology, selectedIds, offeredLoad }` rather than
 *     the web's `{ topology, selectedIds, rps, presetId }`. There is no
 *     standalone `rps` in this app: offered load is DERIVED from the client
 *     nodes (see `routes/+page.svelte`), so `currentSnapshot()` recomputes
 *     the sum to keep the shape -- and to keep `snapshotEqual` honest when
 *     a client's rps is the only thing that moved. `presetId` has no home
 *     yet: the active-example badge is page-local state this module cannot
 *     reach, so undoing a preset load restores the diagram but not the
 *     badge (recorded in this task's report, not papered over here).
 *   - The stacks live in plain closure variables; the ONE reactive field is
 *     `version`, which every getter reads so a button re-derives its
 *     disabled state. This is `App.tsx`'s `setHistVersion` (`src/App.tsx`
 *     ~1029) in Svelte form: caching `canUndo` as a boolean is exactly the
 *     stale-button regression Excalidraw shipped, and this file has no way
 *     to express it.
 *   - The receipt ("Undid move") lives HERE instead of in `+page.svelte`
 *     as `toast` does in App.tsx, so it can be unit-tested without mounting
 *     the whole shell. Observable behaviour is identical: a `role="status"`
 *     element for 2200ms, keyed by an incrementing id so consecutive undos
 *     restart the entrance animation.
 *
 * Selecting is NOT this file's business. `topology.svelte.ts` owns
 * selection, selection changes never call in here, and a committed entry
 * merely CAPTURES the selection as it stood -- which is how undo restores
 * it "for free" without a selection ever clearing the redo stack.
 */

import { topologyStore } from './topology.svelte';
import type { Topology } from '$lib/domain';

/** Everything a single history entry restores. */
export interface HistorySnapshot {
  topology: Topology;
  /**
   * Plain ids, copied out of the live `SvelteSet`. An entry must be inert
   * data: if it held the reactive set itself, every later selection change
   * would retroactively rewrite the entry being compared against.
   */
  selectedIds: readonly string[];
  /** Sum of the client nodes' rps -- see `currentSnapshot()`. */
  offeredLoad: number;
}

/**
 * A snapshot plus the name of the edit that produced it, for the receipt
 * ("Undid move"). The snapshot is the state BEFORE that edit.
 */
export interface HistoryEntry extends HistorySnapshot {
  label: string;
}

/** The receipt's payload. `id` keys the DOM node so repeats re-animate. */
export interface HistoryReceipt {
  text: string;
  id: number;
}

/**
 * Stack bound. 50 entries of a ~30-node topology is on the order of 500KB
 * worst case, which is nothing, and 50 deliberate edits is far more than a
 * student walks back one keypress at a time. The bound exists so an
 * hours-long session cannot grow memory without limit, not because entries
 * are expensive.
 */
export const HISTORY_LIMIT = 50;

/**
 * How long a streamed edit must go quiet before it commits, in ms. Slider
 * input events arrive every frame (16ms) and key-repeat nudges every ~35ms,
 * so anything above ~100ms coalesces a gesture; 500ms also absorbs the
 * natural jitter of a student wiggling a control to a value, while staying
 * short enough that two deliberate consecutive edits become two entries.
 */
export const SETTLE_MS = 500;

/** How long the undo/redo receipt stays on screen, in ms. */
export const RECEIPT_MS = 2200;

/* ---------------- equality ---------------- */

/**
 * Structural equality, written out field by field rather than via
 * JSON.stringify so optional fields present-as-undefined and key order can
 * never produce a false difference. `NodeConfig` and the annotation shapes
 * are flat, so `!==` on each value is exact.
 */
function configEqual(a: object, b: object): boolean {
  const ra = a as Record<string, unknown>;
  const rb = b as Record<string, unknown>;
  for (const k of Object.keys(ra)) if (ra[k] !== rb[k]) return false;
  for (const k of Object.keys(rb)) if (!(k in ra)) return false;
  return true;
}

/**
 * Annotations are flat plain-data objects, so per-entry field comparison via
 * configEqual is exact. Absent and empty are the same diagram: a topology
 * that never had the optional field must not differ from one holding [].
 */
function annotationsEqual(
  a: Topology['annotations'],
  b: Topology['annotations'],
): boolean {
  const xs = a ?? [];
  const ys = b ?? [];
  if (xs.length !== ys.length) return false;
  for (let i = 0; i < xs.length; i++) {
    if (!configEqual(xs[i], ys[i])) return false;
  }
  return true;
}

function edgesEqual(a: Topology, b: Topology): boolean {
  if (a.edges.length !== b.edges.length) return false;
  for (let i = 0; i < a.edges.length; i++) {
    const d = a.edges[i];
    const e = b.edges[i];
    if (
      d.id !== e.id ||
      d.from !== e.from ||
      d.to !== e.to ||
      d.weight !== e.weight ||
      d.control !== e.control ||
      // The three declared-but-unset link fields: absent on both sides
      // today, so comparing them costs nothing and stays honest the day a
      // preset starts carrying a latency.
      d.latencyMs !== e.latencyMs ||
      d.bandwidthRps !== e.bandwidthRps ||
      d.lossRate !== e.lossRate
    ) {
      return false;
    }
  }
  return true;
}

function topologyEqual(a: Topology, b: Topology): boolean {
  if (a.nodes.length !== b.nodes.length) return false;
  if (a.edges.length !== b.edges.length) return false;
  if (!annotationsEqual(a.annotations, b.annotations)) return false;
  for (let i = 0; i < a.nodes.length; i++) {
    const m = a.nodes[i];
    const n = b.nodes[i];
    if (
      m.id !== n.id ||
      m.kind !== n.kind ||
      m.label !== n.label ||
      m.x !== n.x ||
      m.y !== n.y ||
      !configEqual(m.config, n.config)
    ) {
      return false;
    }
  }
  return edgesEqual(a, b);
}

function selectionEqual(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const other = new Set(b);
  for (const id of a) if (!other.has(id)) return false;
  return true;
}

function snapshotEqual(a: HistorySnapshot, b: HistorySnapshot): boolean {
  return (
    a.offeredLoad === b.offeredLoad &&
    selectionEqual(a.selectedIds, b.selectedIds) &&
    topologyEqual(a.topology, b.topology)
  );
}

/**
 * Deep copy on the way IN. Entries must be immune to later mutation of
 * whatever they were captured from; the store is mutated in place in a few
 * places (`nodes.length = 0` in tests, SvelteSet selection churn in the
 * canvas), and history must not depend on every caller remembering that.
 *
 * `$state.snapshot` unwraps Svelte's proxies into plain data AND copies it,
 * which is the desktop equivalent of the web's `structuredClone` here. Its
 * `Snapshot<T>` type is structurally looser than `Topology`, hence the cast.
 */
function cloneSnapshot(s: HistorySnapshot): HistorySnapshot {
  return {
    topology: $state.snapshot(s.topology) as Topology,
    selectedIds: [...s.selectedIds],
    offeredLoad: s.offeredLoad,
  };
}

/**
 * The live state, as of NOW, ready to be handed to `commit`/`touch`/`undo`.
 *
 * Called BEFORE the edit it labels, so it is the pre-edit baseline -- the
 * desktop's answer to App.tsx's `snapRef` (which is written in a layout
 * effect and therefore also holds the previous commit's state when a
 * handler runs). The store is the live truth here, so reading it at the top
 * of a handler gives the same value.
 */
export function currentSnapshot(): HistorySnapshot {
  const topology = topologyStore.topology;
  let offeredLoad = 0;
  for (const node of topology.nodes) {
    if (node.kind === 'client') offeredLoad += node.config.rps;
  }
  return {
    topology,
    selectedIds: [...topologyStore.selectedIds],
    offeredLoad,
  };
}

/** Construction options; every default is the constant documented above. */
export interface SessionHistoryOptions {
  limit?: number;
  settleMs?: number;
  receiptMs?: number;
}

/**
 * The public shape of one history. `version` is exposed because it is what
 * the getters depend on for reactivity; nothing else should write it.
 */
export interface SessionHistory {
  version: number;
  receipt: HistoryReceipt | null;
  readonly canUndo: boolean;
  readonly canRedo: boolean;
  /** True between beginGesture and endGesture, so streamed moves stay out. */
  readonly inGesture: boolean;
  readonly undoDepth: number;
  readonly redoDepth: number;
  readonly oldestLabel: string | null;
  commit(label: string, before: HistorySnapshot): void;
  touch(label: string, before: HistorySnapshot): void;
  flushSettling(): void;
  beginGesture(label: string, before: HistorySnapshot): void;
  endGesture(after: HistorySnapshot): void;
  undo(current: HistorySnapshot): HistoryEntry | null;
  redo(current: HistorySnapshot): HistoryEntry | null;
}

/**
 * Build an independent history. Unit tests take their own instance so
 * cases cannot leak entries into each other; the app uses the singleton
 * below.
 */
export function createSessionHistory(options?: SessionHistoryOptions): SessionHistory {
  const limit = options?.limit ?? HISTORY_LIMIT;
  const settleMs = options?.settleMs ?? SETTLE_MS;
  const receiptMs = options?.receiptMs ?? RECEIPT_MS;

  /* The stacks stay OUT of `$state`: they are rewritten wholesale on every
     edit, and proxying a 50-entry array of cloned topologies would cost a
     deep proxy walk per push. `version` is the single signal that says
     "something moved", exactly as `setHistVersion` is in App.tsx. */
  let past: HistoryEntry[] = [];
  let future: HistoryEntry[] = [];

  /** Baseline captured at gesture promotion, committed at gesture end. */
  let gestureBase: HistoryEntry | null = null;

  /** Baseline of a streamed edit, committed once the stream settles. */
  let pending: HistoryEntry | null = null;
  let settleTimer: ReturnType<typeof setTimeout> | null = null;

  let receiptTimer: ReturnType<typeof setTimeout> | null = null;
  let receiptSeq = 0;

  function pushPast(entry: HistoryEntry): void {
    past.push(entry);
    // Bounded from the bottom: the oldest edit falls off, never the newest.
    if (past.length > limit) past.splice(0, past.length - limit);
  }

  function notify(): void {
    history.version += 1;
  }

  function clearSettleTimer(): void {
    if (settleTimer !== null) {
      clearTimeout(settleTimer);
      settleTimer = null;
    }
  }

  /**
   * Raise the receipt naming what just happened. `id` bumps on every
   * message so a second Ctrl+Z re-mounts the element and restarts its
   * entrance animation instead of sitting still on text that appears not to
   * change (see App.tsx's toast comment, `src/App.tsx` ~1086).
   */
  function showReceipt(verb: 'Undid' | 'Redid', label: string): void {
    receiptSeq += 1;
    history.receipt = { text: `${verb} ${label}`, id: receiptSeq };
    if (receiptTimer !== null) clearTimeout(receiptTimer);
    receiptTimer = setTimeout(() => {
      receiptTimer = null;
      history.receipt = null;
    }, receiptMs);
  }

  /** Land the pending streamed edit now, if there is one. */
  function flushSettling(): void {
    clearSettleTimer();
    if (!pending) return;
    const entry = pending;
    pending = null;
    pushPast(entry);
    notify();
  }

  const history: SessionHistory = $state<SessionHistory>({
    version: 0,
    receipt: null,

    /** Derived from the stack on every read; a pending edit is undoable too. */
    get canUndo() {
      void this.version;
      return past.length > 0 || pending !== null;
    },
    get canRedo() {
      void this.version;
      return future.length > 0;
    },
    get inGesture() {
      void this.version;
      return gestureBase !== null;
    },
    get undoDepth() {
      void this.version;
      return past.length;
    },
    get redoDepth() {
      void this.version;
      return future.length;
    },
    /** Oldest surviving entry's label, for asserting the bound drops from
     *  the bottom. */
    get oldestLabel() {
      void this.version;
      return past[0]?.label ?? null;
    },

    /**
     * A discrete edit is about to happen (add, connect, load a design).
     * `before` is the state as it stands RIGHT NOW, pre-edit. Clears redo:
     * a real edit forks the timeline.
     */
    commit(label: string, before: HistorySnapshot): void {
      // An unsettled streamed edit happened first chronologically; land it
      // before this entry so undo unwinds in the order things were done.
      flushSettling();
      pushPast({ label, ...cloneSnapshot(before) });
      future = [];
      notify();
    },

    /**
     * One frame of a streamed edit (slider drag, spinner held down, arrow
     * key-repeat). The FIRST frame captures the pre-edit baseline and clears
     * redo, because the state has genuinely diverged; every further frame
     * only pushes the settle timer back. The entry lands after `settleMs` of
     * quiet -- commit-on-settle rather than commit-on-release because half
     * the streamed paths have no release event to hook, and undo hides the
     * delay by flushing the pending entry first.
     */
    touch(label: string, before: HistorySnapshot): void {
      // A different KIND of streamed edit starts a new entry: a rename
      // followed within the settle window by a slider drag is two edits, and
      // coalescing them would make one Ctrl+Z revert both.
      if (pending && pending.label !== label) flushSettling();
      if (!pending) {
        pending = { label, ...cloneSnapshot(before) };
        future = [];
        notify();
      }
      clearSettleTimer();
      settleTimer = setTimeout(() => flushSettling(), settleMs);
    },

    flushSettling,

    /**
     * A pointer gesture that will stream moves is starting. Captures the
     * baseline once; the per-frame moves then bypass history entirely (they
     * never reach this module at all on the desktop -- the canvas renders
     * them through a drag overlay and writes the topology once, at the end).
     */
    beginGesture(label: string, before: HistorySnapshot): void {
      flushSettling();
      gestureBase = { label, ...cloneSnapshot(before) };
    },

    /**
     * The gesture ended (pointerup, pointercancel, Escape, or a dropped
     * pointer). Commits the baseline as ONE entry, unless the gesture went
     * nowhere: a drag that returned to its origin, or a press that promoted
     * and released in place, must not cost the student an undo step. The
     * comparison deliberately ignores selection: promotion selects the
     * grabbed node, and a selection-only change is not an entry.
     */
    endGesture(after: HistorySnapshot): void {
      const base = gestureBase;
      gestureBase = null;
      if (!base) return;
      if (
        topologyEqual(base.topology, after.topology) &&
        base.offeredLoad === after.offeredLoad
      ) {
        return;
      }
      pushPast(base);
      future = [];
      notify();
    },

    /**
     * Pop back one entry. `current` is the live state, which becomes the
     * redo entry. Entries indistinguishable from the current state are
     * skipped, so a press of Ctrl+Z always changes something visible or
     * does nothing at all (returning null and raising no receipt).
     */
    undo(current: HistorySnapshot): HistoryEntry | null {
      flushSettling();
      let entry = past.pop();
      while (entry && snapshotEqual(entry, current)) entry = past.pop();
      if (!entry) {
        notify();
        return null;
      }
      future.push({ label: entry.label, ...cloneSnapshot(current) });
      notify();
      showReceipt('Undid', entry.label);
      return entry;
    },

    /** Pop forward one entry. Mirror of undo; does NOT clear the future. */
    redo(current: HistorySnapshot): HistoryEntry | null {
      flushSettling();
      let entry = future.pop();
      while (entry && snapshotEqual(entry, current)) entry = future.pop();
      if (!entry) {
        notify();
        return null;
      }
      pushPast({ label: entry.label, ...cloneSnapshot(current) });
      notify();
      showReceipt('Redid', entry.label);
      return entry;
    },
  });

  return history;
}

/**
 * The history the app shares: `+page.svelte` commits and undoes, and
 * `Canvas.svelte` brackets its gestures with it.
 */
export const sessionHistory = createSessionHistory();
