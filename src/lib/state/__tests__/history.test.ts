import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  createSessionHistory,
  currentSnapshot,
  HISTORY_LIMIT,
  RECEIPT_MS,
  SETTLE_MS,
} from '../history.svelte';
import type { HistorySnapshot } from '../history.svelte';
import { topologyStore, clearSelection, setSelection } from '$lib/state/topology.svelte';
import { makeNode } from '$lib/components/canvas/geometry';
import type { Topology } from '$lib/domain';

/** The store pulls in the Tauri command wrappers transitively; keep IPC out
 *  of jsdom even though these tests never reach a sync. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/**
 * Unit tests for the desktop port of `src/history.ts` (SessionHistory).
 *
 * Every case builds its own history via `createSessionHistory()` so entries
 * cannot leak between cases, and hand-builds snapshots rather than driving
 * the store: what is under test is the stack logic itself, not the store.
 * The one exception is `currentSnapshot()`, which has to read the real
 * store to prove the offered-load sum and the selection copy.
 *
 * `topo()` deliberately varies each topology it returns (node id and x ride
 * a counter): two snapshots built from separate calls are then never
 * accidentally "equal", so a test only collapses when it says so.
 */
let seq = 0;
function topo(): Topology {
  seq += 1;
  return { nodes: [makeNode('client', seq, 0)], edges: [] };
}

function snap(t: Topology, ids: readonly string[] = []): HistorySnapshot {
  return { topology: t, selectedIds: ids, offeredLoad: 0 };
}

afterEach(() => {
  vi.useRealTimers();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  clearSelection();
});

describe('commit / undo / redo', () => {
  it('lands one entry per commit and walks back then forward', () => {
    const h = createSessionHistory();
    const before = snap(topo());
    const after = snap(topo());

    h.commit('add', before);

    expect(h.canUndo).toBe(true);
    expect(h.canRedo).toBe(false);
    expect(h.undoDepth).toBe(1);
    expect(h.oldestLabel).toBe('add');

    // The undo's `current` is the post-edit state; the entry it returns
    // holds the pre-edit one.
    const undone = h.undo(after);
    expect(undone?.label).toBe('add');
    expect(undone?.topology.nodes[0].id).toBe(before.topology.nodes[0].id);
    expect(h.canUndo).toBe(false);
    expect(h.canRedo).toBe(true);

    // Redo hands back the state the undo had replaced, and REFILLS past
    // rather than clearing it -- redo must not cost the undo stack.
    const redone = h.redo(before);
    expect(redone?.topology.nodes[0].id).toBe(after.topology.nodes[0].id);
    expect(h.undoDepth).toBe(1);
    expect(h.canRedo).toBe(false);
  });

  it('discards an entry indistinguishable from the current state', () => {
    const h = createSessionHistory();
    const t = topo();
    h.commit('move', snap(t));

    // Nothing actually changed: the press must neither rewind anything nor
    // leave a phantom entry behind (and nothing is pushed to redo, so the
    // skipped undo is invisible to the student).
    expect(h.undo(snap(t))).toBeNull();
    expect(h.canUndo).toBe(false);
    expect(h.canRedo).toBe(false);
  });

  it('clears the redo stack the moment a real edit is committed', () => {
    const h = createSessionHistory();
    h.commit('add', snap(topo()));
    h.undo(snap(topo()));
    expect(h.canRedo).toBe(true);

    h.commit('connect', snap(topo()));

    expect(h.canRedo).toBe(false);
    expect(h.redoDepth).toBe(0);
    // The undo emptied past, so this commit is now the only entry in it.
    expect(h.undoDepth).toBe(1);
    expect(h.oldestLabel).toBe('connect');
  });

  it('restores the selection the entry captured, not whatever is selected now', () => {
    const h = createSessionHistory();
    h.commit('add', snap(topo(), ['alpha']));

    // Selection changed after the commit, with no history call: selection
    // edits neither create entries nor rewrite existing ones.
    const entry = h.undo(snap(topo(), ['beta', 'gamma']));
    expect([...entry!.selectedIds]).toEqual(['alpha']);

    const back = h.redo(snap(topo()));
    expect([...back!.selectedIds]).toEqual(['beta', 'gamma']);
  });

  it('flushes an unsettled streamed edit before the next discrete one', () => {
    const h = createSessionHistory();
    const base = snap(topo());
    h.touch('setting change', base);
    h.commit('connect', snap(topo()));

    // Chronologically the slider ran first, so it must sit BELOW the
    // commit: undo unwinds in the order things were done.
    expect(h.undoDepth).toBe(2);
    expect(h.oldestLabel).toBe('setting change');
  });
});

describe('gesture bracket (beginGesture / endGesture)', () => {
  it('turns a whole drag into one entry and reports the bracket', () => {
    const h = createSessionHistory();
    const base = snap(topo(), ['keep-me']);
    h.beginGesture('move', base);
    expect(h.inGesture).toBe(true);

    h.endGesture(snap(topo()));

    expect(h.inGesture).toBe(false);
    expect(h.undoDepth).toBe(1);
    expect(h.oldestLabel).toBe('move');
    // The baseline held the pre-drag selection, exactly as the web's
    // onMoveStart contract promises.
    expect(h.canRedo).toBe(false);
  });

  it('drops a gesture that moved nothing', () => {
    const h = createSessionHistory();
    const t = topo();

    h.beginGesture('move', snap(t));
    h.endGesture(snap(t));

    expect(h.undoDepth).toBe(0);
    expect(h.canUndo).toBe(false);
  });

  it('ignores a selection-only difference: promoting selects, that is not an entry', () => {
    const h = createSessionHistory();
    const t = topo();

    h.beginGesture('move', snap(t, []));
    h.endGesture(snap(t, ['node-9']));

    expect(h.undoDepth).toBe(0);
  });

  it('replaces a stranded baseline when the next gesture starts', () => {
    const h = createSessionHistory();
    h.beginGesture('move', snap(topo()));
    // Second press supersedes: the first drag never ended (its pointer
    // went away), and must not commit retroactively under the new label.
    h.beginGesture('resize', snap(topo()));
    expect(h.inGesture).toBe(true);

    h.endGesture(snap(topo()));

    expect(h.undoDepth).toBe(1);
    expect(h.oldestLabel).toBe('resize');
  });
});

describe('streamed edits (touch)', () => {
  it('coalesces same-label frames into a single entry', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();

    h.touch('setting change', snap(topo()));
    expect(h.canUndo).toBe(true); // the baseline is undoable at once...
    expect(h.undoDepth).toBe(0); // ...but has not landed yet.

    // More frames inside the settle window only push the timer back.
    h.touch('setting change', snap(topo()));
    vi.advanceTimersByTime(SETTLE_MS - 1);
    expect(h.undoDepth).toBe(0);
    vi.advanceTimersByTime(1);

    expect(h.undoDepth).toBe(1);
    expect(h.redoDepth).toBe(0);
  });

  it('separates two different kinds of edit even inside one settle window', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();

    h.touch('rename', snap(topo()));
    h.touch('setting change', snap(topo()));
    vi.advanceTimersByTime(SETTLE_MS);

    expect(h.undoDepth).toBe(2);
    expect(h.oldestLabel).toBe('rename');
  });

  it('is undone immediately, without waiting for the settle timer', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();
    h.touch('setting change', snap(topo()));

    // No advance: undo flushes the pending entry itself so the student
    // never waits out 500ms to take back one frame of a slider drag.
    const entry = h.undo(snap(topo()));

    expect(entry?.label).toBe('setting change');
    expect(h.canUndo).toBe(false);
    expect(h.canRedo).toBe(true);
  });

  it('clears the redo stack on its first frame only', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();
    h.commit('add', snap(topo()));
    h.undo(snap(topo()));
    expect(h.canRedo).toBe(true);

    h.touch('setting change', snap(topo()));
    expect(h.canRedo).toBe(false);

    vi.advanceTimersByTime(SETTLE_MS);
    // Past was emptied by the undo; the settled frame is its only entry.
    expect(h.undoDepth).toBe(1);
    expect(h.canUndo).toBe(true);
  });
});

describe('capacity', () => {
  it('keeps the newest HISTORY_LIMIT entries and drops from the bottom', () => {
    const h = createSessionHistory();
    for (let i = 0; i < HISTORY_LIMIT + 5; i++) {
      h.commit(`edit-${i}`, snap(topo()));
    }

    expect(h.undoDepth).toBe(HISTORY_LIMIT);
    // The five OLDEST went, not the newest five.
    expect(h.oldestLabel).toBe('edit-5');
    h.undo(snap(topo()));
    expect(h.oldestLabel).toBe('edit-6');
  });
});

describe('receipt', () => {
  it('names the reverted action, bumps its id, then dismisses itself', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();

    h.commit('move', snap(topo()));
    h.undo(snap(topo()));
    expect(h.receipt).toEqual({ text: 'Undid move', id: 1 });

    const firstId = h.receipt!.id;
    // Straight back forward again -- no commit in between, or the redo
    // stack would be the thing that got cleared.
    h.redo(snap(topo()));
    // A repeat re-keys the element so its entrance animation restarts
    // instead of the text silently swapping in place.
    expect(h.receipt).toEqual({ text: 'Redid move', id: firstId + 1 });

    vi.advanceTimersByTime(RECEIPT_MS);
    expect(h.receipt).toBeNull();
  });

  it('raises nothing for an undo that found nothing to undo', () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const h = createSessionHistory();
    expect(h.undo(snap(topo()))).toBeNull();
    expect(h.receipt).toBeNull();
  });
});

describe('currentSnapshot', () => {
  it('sums client rps and copies the selection out of the live set', () => {
    topologyStore.topology.nodes.push(
      { ...makeNode('client', 0, 0), config: { ...makeNode('client', 0, 0).config, rps: 30 } },
      { ...makeNode('client', 80, 0), config: { ...makeNode('client', 80, 0).config, rps: 12 } },
      makeNode('queue', 160, 0),
    );
    const id = topologyStore.topology.nodes[0].id;
    setSelection([id]);

    const s = currentSnapshot();

    expect(s.offeredLoad).toBe(42); // queue contributes nothing
    expect([...s.selectedIds]).toEqual([id]);

    // A plain array, not the live SvelteSet: mutating the store afterwards
    // must not rewrite a snapshot history is already holding.
    topologyStore.selectedIds.clear();
    expect([...s.selectedIds]).toEqual([id]);
  });
});
