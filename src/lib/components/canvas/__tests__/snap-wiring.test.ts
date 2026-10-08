import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { makeNode, GRID } from '../geometry';
import { topologyStore, selectOne, clearSelection } from '$lib/state/topology.svelte';
import { settingsStore, setSetting } from '$lib/state/settings.svelte';

/* Keep every IPC bridge out of jsdom, same guard group-drag.test.ts uses. */
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

function nodeAt(id: string) {
  const n = topologyStore.topology.nodes.find((x) => x.id === id);
  if (!n) throw new Error(`expected node ${id} in the topology`);
  return n;
}

/** Press -> drag -> release, all on one pointer id, optionally with Ctrl held. */
async function drag(
  el: Element,
  from: { x: number; y: number },
  to: { x: number; y: number },
  ctrlKey = false,
) {
  const base = { pointerId: 1, pointerType: 'mouse' as const, ctrlKey };
  await fireEvent.pointerDown(el, { ...base, button: 0, clientX: from.x, clientY: from.y });
  await fireEvent.pointerMove(el, { ...base, clientX: to.x, clientY: to.y });
  await fireEvent.pointerUp(el, { ...base, button: 0, clientX: to.x, clientY: to.y });
  await tick();
}

beforeEach(() => {
  setSetting('snapToGrid', true);
});

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  setSetting('snapToGrid', true);
});

/* ------------------------------------------------------------------ *
 * Upstream 1db4ac61 (PR #37): "G toggles snap to grid... Ctrl stays a
 * momentary override for the drag in progress and only ever loosens."
 * docs/PORTING_GAP.md flags this as VERIFY-and-fix, not from-scratch:
 * these pin the join between the preference, the G key and a live drag,
 * the same role web's Canvas.snap.wiring.test.ts plays for Canvas.tsx.
 * ------------------------------------------------------------------ */

describe('G toggles snap-to-grid', () => {
  it('flips settingsStore.snapToGrid on a plain "g" keydown', async () => {
    render(Canvas);
    await tick();
    expect(settingsStore.snapToGrid).toBe(true);
    await fireEvent.keyDown(window, { key: 'g' });
    expect(settingsStore.snapToGrid).toBe(false);
    await fireEvent.keyDown(window, { key: 'g' });
    expect(settingsStore.snapToGrid).toBe(true);
  });

  it('does not toggle when a modifier is held (reserved for other shortcuts)', async () => {
    render(Canvas);
    await tick();
    await fireEvent.keyDown(window, { key: 'g', ctrlKey: true });
    expect(settingsStore.snapToGrid).toBe(true);
  });
});

describe('node drag honours the snap preference and the Ctrl override', () => {
  // Press lands exactly on the node's own (0,0) world origin (client
  // (40,40): toWorld subtracts the default view's {x:40,y:40,k:1}), so
  // `grabDx`/`grabDy` are both 0 and the drag's own math reduces to
  // `place(worldDx)` -- simplest possible numbers to reason about, not
  // chosen to line up with any particular grid cell.
  const PRESS = { x: 40, y: 40 };
  // +23 world px: deliberately NOT a multiple of GRID (8), so snapped and
  // unsnapped outcomes are never equal by coincidence.
  const DRAG_TO = { x: 63, y: 40 };

  it('snaps a drag to the grid when the preference is on', async () => {
    const a = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(a);
    selectOne(a.id, false);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${a.id}"]`);
    expect(el).toBeInTheDocument();

    await drag(el!, PRESS, DRAG_TO);

    // snapTo(23, 8) = Math.round(23/8)*8 = 24.
    expect(nodeAt(a.id).x).toBe(24);
  });

  it('bypasses the snap for the drag while Ctrl is held', async () => {
    const a = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(a);
    selectOne(a.id, false);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${a.id}"]`);

    await drag(el!, PRESS, DRAG_TO, true);

    // Math.round(23) = 23, the grid is bypassed for this drag alone.
    expect(nodeAt(a.id).x).toBe(23);
  });

  it('never re-enables snapping when Ctrl is held and the preference is off', async () => {
    setSetting('snapToGrid', false);
    const a = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(a);
    selectOne(a.id, false);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${a.id}"]`);

    await drag(el!, PRESS, DRAG_TO, true);

    // Same unsnapped (Math.round) result as the preference-off, Ctrl-less
    // case would give: holding Ctrl while the preference is already off
    // changes nothing, because Ctrl only ever LOOSENS.
    expect(nodeAt(a.id).x).toBe(23);
  });
});
