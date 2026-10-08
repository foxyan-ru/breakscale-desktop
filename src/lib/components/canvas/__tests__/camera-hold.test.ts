import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { makeNode } from '../geometry';
import { topologyStore, clearSelection } from '$lib/state/topology.svelte';

/* Keep every IPC bridge out of jsdom, same guard group-drag.test.ts uses. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}
globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
});

/**
 * Upstream commit `254a1b51` ("hold the camera still when a component
 * lands") fixed a bug in a reactive re-fit EFFECT on the web app: an
 * observer / effect that re-aimed the camera whenever the node count went
 * from empty to non-empty, which made a palette drop onto an empty canvas
 * look like the view sliding out from under the cursor.
 *
 * `docs/PORTING_GAP.md` lists this as "Missing" on the desktop, but the
 * underlying mechanism it protects against -- an automatic refit reactive
 * to topology changes (on mount, session restore, or a node landing) --
 * does not exist here AT ALL: Canvas.svelte's own "Known limitations"
 * comment says the view "is otherwise only moved by the user (pan/zoom/
 * pinch), the Minimap's ongoto, or a fit" (Shift+1/2 / the cluster button,
 * all user-initiated). There is no `ResizeObserver`, no `fitSignal` prop
 * (Canvas.svelte takes no props at all), and no effect anywhere that calls
 * `fitToContent`/`fitTo` automatically.
 *
 * This test pins that: adding a node (the empty -> non-empty transition
 * upstream's bug specifically targeted) must not move the camera. It
 * currently passes trivially because the auto-refit feature it would
 * guard was never ported -- there is nothing here FOR a drop to race
 * against. If a future phase ports web's automatic re-fit-on-first-content
 * (a separate, larger feature not in this task's scope), THIS test is the
 * one that must keep passing: it is the regression guard upstream
 * `254a1b51` itself exists to satisfy.
 */
describe('camera holds still when a component lands', () => {
  it('does not move the view when a node is added to an empty canvas', async () => {
    const { container } = render(Canvas);
    await tick();
    const g = container.querySelector('g[transform]');
    expect(g).toBeInTheDocument();
    const before = g!.getAttribute('transform');

    topologyStore.topology.nodes.push(makeNode('client', 4000, 4000));
    await tick();

    const after = g!.getAttribute('transform');
    expect(after).toBe(before);
  });
});
