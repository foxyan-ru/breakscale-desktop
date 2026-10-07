import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { makeNode } from '../geometry';
import {
  topologyStore,
  selectOne,
  selectAll,
  clearSelection,
} from '$lib/state/topology.svelte';
import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';
import { buildClipboardText } from '$lib/domain/clipboard';

/** Keep every IPC bridge out of jsdom: the stores import the Tauri command
 *  wrappers transitively, and the canvas fire-and-forget syncs on every
 *  topology write these tests drive. */
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
 * Shared singleton, emptied both ways before every case (see
 * `routes/__tests__/undo-redo.test.ts` for why one direction cannot do
 * it). `undo` flushes a pending `touch` first, so this also clears any
 * settle timer a previous case armed. Nothing is APPLIED while draining,
 * so each case's setup survives into the case itself.
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

async function up(el: Element, x: number, y: number): Promise<void> {
  await fireEvent.pointerUp(el, { ...MOUSE, button: 0, clientX: x, clientY: y });
  await tick();
}

function hitNode(container: HTMLElement, id: string): Element {
  const el = container.querySelector(`[data-hit="node"][data-id="${id}"]`);
  if (!el) throw new Error(`hit target for node ${id} did not render`);
  return el;
}

function hitPort(container: HTMLElement, id: string): Element {
  const el = container.querySelector(`[data-hit="port-out"][data-id="${id}"]`);
  if (!el) throw new Error(`port-out for node ${id} did not render`);
  return el;
}

beforeEach(() => {
  drainHistory();
  // Selection is global store state: a leak from any other file could put
  // this node under selectedNodeId and make the nudge guard swallow the
  // arrow presses this file drives.
  clearSelection();
});

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  if (topologyStore.topology.annotations) topologyStore.topology.annotations.length = 0;
});

/**
 * The discrete (non-gesture) write paths, each carrying the web's exact
 * label straight from App.tsx: a connection (:1556), a whole-selection
 * delete whose undo must bring the EDGES back with the nodes (:1587),
 * paste (:1734) and the arrow-key nudge's coalescing stream (:1156).
 * `undo-shortcut-contract.test.ts` pins the call-site counts; these cases
 * prove the entries actually restore the topology they hold.
 */
describe('discrete canvas edits as history entries', () => {
  it('records a click-through connection as one entry that undo removes', async () => {
    const a = makeNode('client', 0, 0);
    const b = makeNode('service', 400, 0);
    topologyStore.topology.nodes.push(a, b);

    const { container } = render(Canvas);
    await tick();
    const depthBefore = sessionHistory.undoDepth;

    // Arm the connection from A's output port -- selecting is not an edit.
    const port = hitPort(container, a.id);
    await down(port, 10, 10);
    await up(port, 10, 10);
    expect(sessionHistory.undoDepth).toBe(depthBefore);

    // Clicking B completes it: one edge, one entry, label 'connection'.
    const nodeB = hitNode(container, b.id);
    await down(nodeB, 100, 60);
    await up(nodeB, 100, 60);

    expect(topologyStore.topology.edges).toHaveLength(1);
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('connection');
    expect(entry!.topology.edges).toHaveLength(0);
    expect(entry!.topology.nodes).toHaveLength(2);
  });

  it('restores nodes AND edges when a whole-selection delete is undone', async () => {
    const a = makeNode('client', 0, 0);
    const b = makeNode('service', 400, 0);
    topologyStore.topology.nodes.push(a, b);
    topologyStore.topology.edges.push({ id: 'edge-seed', from: a.id, to: b.id, weight: 1 });
    selectAll();

    // Rendered for its window-level keydown listener (Delete).
    render(Canvas);
    await tick();
    const depthBefore = sessionHistory.undoDepth;

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Delete' }));
    await tick();

    // Both the nodes and the edge between them are gone in ONE entry --
    // a two-step undo that only brought the nodes back would leave the
    // student hunting for their connections.
    expect(topologyStore.topology.nodes).toHaveLength(0);
    expect(topologyStore.topology.edges).toHaveLength(0);
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('delete');
    expect(entry!.topology.nodes).toHaveLength(2);
    expect(entry!.topology.edges).toHaveLength(1);
    expect(entry!.topology.edges[0].id).toBe('edge-seed');
  });

  it('records one paste entry that undo takes back to the single node', async () => {
    const a = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(a);
    selectOne(a.id, false);
    const text = buildClipboardText(topologyStore.topology, topologyStore.selectedIds);

    // Rendered for its document-level paste listener.
    render(Canvas);
    await tick();
    const depthBefore = sessionHistory.undoDepth;

    // The handler is a document-level 'paste' listener; jsdom's
    // ClipboardEvent does not reliably carry `clipboardData`, so the
    // payload is hung on the event the way the handler reads it.
    const evt = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(evt, 'clipboardData', {
      value: { getData: (t: string) => (t === 'text/plain' ? text ?? '' : '') },
    });
    document.dispatchEvent(evt);
    await tick();

    expect(topologyStore.topology.nodes).toHaveLength(2);
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    const entry = sessionHistory.undo(currentSnapshot());
    expect(entry?.label).toBe('paste');
    expect(entry!.topology.nodes).toHaveLength(1);
  });

  it('coalesces an arrow-key burst into one move entry, one entry per settled press', async () => {
    const a = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(a);

    const { container } = render(Canvas);
    await tick();
    const el = hitNode(container, a.id);
    const depthBefore = sessionHistory.undoDepth;

    // Two presses in the same frame (key-repeat): streamed like the web's
    // guarded touch('move') at :1156 -- undoable immediately, pushed once.
    await fireEvent.keyDown(el, { key: 'ArrowLeft' });
    await fireEvent.keyDown(el, { key: 'ArrowLeft' });
    const xAfterBurst = topologyStore.topology.nodes[0].x;
    expect(xAfterBurst).toBeLessThan(0);
    expect(sessionHistory.canUndo).toBe(true);
    expect(sessionHistory.undoDepth).toBe(depthBefore);

    sessionHistory.flushSettling();
    expect(sessionHistory.undoDepth).toBe(depthBefore + 1);

    // After the stream settled, the next press is its own entry.
    await fireEvent.keyDown(el, { key: 'ArrowLeft' });
    sessionHistory.flushSettling();
    expect(sessionHistory.undoDepth).toBe(depthBefore + 2);

    // Unwind in reverse order: first the lone press (baseline = where the
    // burst left off), then the burst itself (baseline = the origin).
    const lone = sessionHistory.undo(currentSnapshot());
    expect(lone?.label).toBe('move');
    expect(lone!.topology.nodes[0].x).toBe(xAfterBurst);

    const burst = sessionHistory.undo(currentSnapshot());
    expect(burst?.label).toBe('move');
    expect(burst!.topology.nodes[0].x).toBe(0);
  });
});
