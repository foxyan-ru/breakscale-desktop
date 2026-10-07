import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Inspector from '../Inspector.svelte';
import { topologyStore, select, clearSelection } from '$lib/state/topology.svelte';
import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';
import { makeNode } from '$lib/components/canvas/geometry';
import type { SimNode } from '$lib/domain';

/** Keep every IPC bridge out of jsdom: the store's config writes push to
 *  Rust over the same wrappers. Mirrors `number-commit.test.ts`. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/** Emptied both ways before every case; `undo` also flushes a pending
 *  settle timer, so no timer from a previous case can land mid-test. */
function drainHistory(): void {
  while (sessionHistory.canUndo) sessionHistory.undo(currentSnapshot());
  if (sessionHistory.canRedo) {
    sessionHistory.commit('drain', currentSnapshot());
    sessionHistory.undo(currentSnapshot());
  }
  sessionHistory.receipt = null;
}

/** The number input for `capacity`, the field these tests drive. */
function capacityInput(container: Element): HTMLInputElement {
  const input = container.querySelector('#ins-capacity');
  if (!input) throw new Error('capacity field did not render');
  return input as HTMLInputElement;
}

function capacity(): number {
  const n = topologyStore.topology.nodes[0];
  return n ? n.config.capacity : NaN;
}

describe('inspector knob changes as history entries', () => {
  let node: SimNode;

  beforeEach(() => {
    drainHistory();
    node = makeNode('service', 0, 0);
    topologyStore.topology.nodes.push(node);
    select(node.id, null);
  });

  afterEach(() => {
    clearSelection();
    topologyStore.topology.nodes.length = 0;
  });

  it('coalesces successive knob changes into one setting change entry', async () => {
    const { container } = render(Inspector);
    await tick();
    const depthBefore = sessionHistory.undoDepth;
    const original = capacity();

    // Two commits inside one settle window (a drag released twice, or two
    // knobs turned back to back): ONE pending entry, like App.tsx's
    // touch('setting change') at :1751.
    await fireEvent.change(capacityInput(container), { target: { value: '20' } });
    await fireEvent.change(capacityInput(container), { target: { value: '30' } });
    await tick();

    expect(sessionHistory.canUndo).toBe(true);
    expect(sessionHistory.undoDepth).toBe(depthBefore);

    sessionHistory.flushSettling();
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    // The baseline was taken before the FIRST frame, so undo lands on the
    // value the panel started from, not the intermediate one.
    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('setting change');
    expect(entry!.topology.nodes[0].config.capacity).toBe(original);
  });

  it('a knob change after the stream settled is a second entry', async () => {
    const { container } = render(Inspector);
    await tick();
    const depthBefore = sessionHistory.undoDepth;

    await fireEvent.change(capacityInput(container), { target: { value: '20' } });
    sessionHistory.flushSettling();
    await fireEvent.change(capacityInput(container), { target: { value: '30' } });
    sessionHistory.flushSettling();

    // Two deliberate edits are two Ctrl+Z steps: the settle window must
    // not swallow a later change into the earlier entry.
    expect(sessionHistory.undoDepth).toBe(depthBefore + 2);
  });
});
