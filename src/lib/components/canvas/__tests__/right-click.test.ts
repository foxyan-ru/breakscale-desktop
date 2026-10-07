import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { makeNode } from '../geometry';
import { topologyStore, selectOne, clearSelection } from '$lib/state/topology.svelte';

/** The stores pull in the Tauri command wrappers transitively; keep IPC out
 *  of jsdom even though a right-press never reaches a sync. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/** jsdom has no ResizeObserver, which `bind:clientWidth` creates unguarded
 *  the moment Canvas renders. The stub never fires (surface stays 0x0). */
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

/** The Inspector IS this app's side menu, so a right-press on a node is
 *  what should open it: the press has to land the node in the selection
 *  without collapsing anything else the student had selected. */
async function rightPress(el: Element): Promise<void> {
  await fireEvent.pointerDown(el, { pointerId: 1, pointerType: 'mouse', button: 2, clientX: 100, clientY: 60 });
  await tick();
}

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
});

describe('right-click select-to-open', () => {
  it('selects the node under a right-press so the Inspector opens', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${node.id}"]`);
    expect(el).toBeInTheDocument();

    await rightPress(el!);

    expect(topologyStore.selectedIds.size).toBe(1);
    expect(topologyStore.selectedIds.has(node.id)).toBe(true);

    // No gesture is ever started, so the matching release must be inert:
    // nothing is dragged, clicked or re-selected by it.
    await fireEvent.pointerUp(el!, { pointerId: 1, pointerType: 'mouse', button: 2 });
    await tick();
    expect(topologyStore.selectedIds.size).toBe(1);
    expect(topologyStore.topology.nodes[0]).toMatchObject({ x: 0, y: 0 });
  });

  it('leaves an already-selected multi-selection alone', async () => {
    const a = makeNode('client', 0, 0);
    const b = makeNode('queue', 400, 0);
    topologyStore.topology.nodes.push(a, b);
    selectOne(a.id, false);
    selectOne(b.id, true);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${a.id}"]`);
    expect(el).toBeInTheDocument();

    await rightPress(el!);

    // A right-press must never collapse a selection down to one node.
    expect(topologyStore.selectedIds.size).toBe(2);
    expect(topologyStore.selectedIds.has(a.id)).toBe(true);
    expect(topologyStore.selectedIds.has(b.id)).toBe(true);
  });

  it('changes nothing for a right-press on the background', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);
    selectOne(node.id, false);

    const { container } = render(Canvas);
    await tick();
    const surface = container.querySelector('.cv-surface');
    expect(surface).toBeInTheDocument();

    await rightPress(surface!);

    expect(topologyStore.selectedIds.size).toBe(1);
    expect(topologyStore.selectedIds.has(node.id)).toBe(true);
  });

  it('suppresses the native context menu over the diagram', async () => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);

    const { container } = render(Canvas);
    await tick();
    const surface = container.querySelector('.cv-surface')!;
    const el = container.querySelector(`[data-hit="node"][data-id="${node.id}"]`)!;

    // Dispatched by hand rather than through `fireEvent.contextMenu`, whose
    // return value in @testing-library/svelte is a promise that resolves
    // after the Svelte flush, not the `!event.defaultPrevented` boolean the
    // DOM library returns. `defaultPrevented` on the event itself is the
    // fact under test, and it is synchronous: true only if the handler ran
    // preventDefault during dispatch. `bubbles` mirrors the real event, so
    // a menu on a node is still cancelled by the surface's handler.
    const suppresses = (target: Element): boolean => {
      const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
      target.dispatchEvent(ev);
      return ev.defaultPrevented;
    };

    expect(suppresses(surface)).toBe(true);
    expect(suppresses(el)).toBe(true);
  });
});
