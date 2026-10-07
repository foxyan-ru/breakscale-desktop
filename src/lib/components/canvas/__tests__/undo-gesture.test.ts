import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { makeNode } from '../geometry';
import { makeNote } from '$lib/domain/annotations';
import { topologyStore, selectOne, clearSelection } from '$lib/state/topology.svelte';
import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';

/** Keep every IPC bridge out of jsdom: the stores import the Tauri command
 *  wrappers transitively, and the canvas fire-and-forget syncs on drag end. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/** jsdom has no ResizeObserver; Svelte's `bind:clientWidth` creates one
 *  unguarded the moment Canvas renders. Never fires (surface stays 0x0). */
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

/**
 * These tests drive the SHARED history singleton through Canvas's own
 * gesture hooks, so every case starts by emptying both stacks (see
 * `routes/__tests__/undo-redo.test.ts` for why one direction cannot do it).
 * Nothing is APPLIED while draining, so the topology each case sets up
 * survives into the case itself.
 */
function drainHistory(): void {
  while (sessionHistory.canUndo) sessionHistory.undo(currentSnapshot());
  if (sessionHistory.canRedo) {
    sessionHistory.commit('drain', currentSnapshot());
    sessionHistory.undo(currentSnapshot());
  }
  sessionHistory.receipt = null;
}

const MOUSE = { pointerId: 1, pointerType: 'mouse' as const };

async function down(el: Element, x: number, y: number): Promise<void> {
  await fireEvent.pointerDown(el, { ...MOUSE, button: 0, clientX: x, clientY: y });
}

async function move(el: Element, x: number, y: number): Promise<void> {
  await fireEvent.pointerMove(el, { ...MOUSE, clientX: x, clientY: y });
}

async function up(el: Element, x: number, y: number): Promise<void> {
  await fireEvent.pointerUp(el, { ...MOUSE, button: 0, clientX: x, clientY: y });
  await tick();
}

/** Press at `from`, drag to `to`, release -- one pointer id throughout. */
async function drag(el: Element, from: [number, number], to: [number, number]): Promise<void> {
  await down(el, from[0], from[1]);
  await move(el, to[0], to[1]);
  await up(el, to[0], to[1]);
}

function hitNode(container: HTMLElement, id: string): Element {
  const el = container.querySelector(`[data-hit="node"][data-id="${id}"]`);
  if (!el) throw new Error(`hit target for node ${id} did not render`);
  return el;
}

beforeEach(() => {
  drainHistory();
});

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  if (topologyStore.topology.annotations) topologyStore.topology.annotations.length = 0;
});

describe('pointer gestures as history entries', () => {
  it('turns one drag into exactly one move entry bracketed around it', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);

    const { container } = render(Canvas);
    await tick();
    const el = hitNode(container, node.id);
    const depthBefore = sessionHistory.undoDepth;

    await down(el, 100, 60);
    await move(el, 140, 60);
    // Bracketed: promoted above threshold, nothing committed yet.
    expect(sessionHistory.inGesture).toBe(true);

    await up(el, 140, 60);
    expect(sessionHistory.inGesture).toBe(false);
    // The whole drag -- every intermediate frame of it -- is ONE entry.
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);
    expect(topologyStore.topology.nodes[0].x).toBeGreaterThan(0);

    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('move');
    // Baseline was captured BEFORE promote()'s selectOne, so it holds the
    // pre-drag selection (empty), not the node the drag ended up selecting.
    expect([...entry!.selectedIds]).toEqual([]);
    expect(entry!.topology.nodes[0].x).toBe(0);
  });

  it('commits a multi-member group drag as one entry despite its several writes', async () => {
    const a = makeNode('client', 0, 0);
    const b = makeNode('queue', 400, 0);
    const note = makeNote(1000, 500, 'shared');
    topologyStore.topology.nodes.push(a, b);
    topologyStore.topology.annotations = [note];
    selectOne(a.id, false);
    selectOne(b.id, true);
    selectOne(note.id, true);

    const { container } = render(Canvas);
    await tick();
    const el = hitNode(container, a.id);
    const depthBefore = sessionHistory.undoDepth;

    await drag(el, [100, 60], [140, 60]);

    // finishDrag writes annotations AND nodes separately; the student still
    // takes the whole reposition back in one Ctrl+Z.
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('move');
    expect(entry!.topology.nodes.find((n) => n.id === a.id)?.x).toBe(0);
    expect(entry!.topology.annotations?.find((x) => x.id === note.id)?.x).toBe(1000);
  });

  it('spends no entry on a press that never crossed the drag threshold', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);

    const { container } = render(Canvas);
    await tick();
    const el = hitNode(container, node.id);
    const depthBefore = sessionHistory.undoDepth;

    // Down and up in the same spot: a click selects, and selecting is not
    // an edit the student should have to undo.
    await down(el, 100, 60);
    await up(el, 100, 60);

    expect(sessionHistory.inGesture).toBe(false);
    expect(sessionHistory.undoDepth).toBe(depthBefore);
    expect(topologyStore.topology.nodes[0].x).toBe(0);
  });

  it('closes the bracket on Escape without committing a move that never landed', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);

    const { container } = render(Canvas);
    await tick();
    const el = hitNode(container, node.id);
    const depthBefore = sessionHistory.undoDepth;

    await down(el, 100, 60);
    await move(el, 140, 60);
    expect(sessionHistory.inGesture).toBe(true);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await tick();

    // The drag was still in flight (no pointerup), so the topology never
    // moved: the cancelled gesture tidies its baseline away, and a
    // stranded baseline must never survive to corrupt the next gesture.
    expect(sessionHistory.inGesture).toBe(false);
    expect(sessionHistory.undoDepth).toBe(depthBefore);
    expect(topologyStore.topology.nodes[0].x).toBe(0);
  });
});
