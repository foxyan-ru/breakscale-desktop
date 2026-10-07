import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { groupDragTargets, sharedSnappedDelta } from '../pointer-input';
import { makeNode } from '../geometry';
import { makeNote } from '$lib/domain/annotations';
import type { Note } from '$lib/domain/annotations';
import { topologyStore, selectOne, clearSelection } from '$lib/state/topology.svelte';

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

const NODE_IDS = ['n1', 'n2', 'n3'];
const ANN_IDS = ['a1', 'a2'];

describe('groupDragTargets', () => {
  it('is null for a background press, an unselected node, or a lone selection', () => {
    expect(groupDragTargets(null, new Set(['n1']), NODE_IDS, ANN_IDS)).toBeNull();
    // The pressed id is not in the selection: this press should select and
    // move only itself (the caller does that with the null).
    expect(groupDragTargets('n3', new Set(['n1', 'n2']), NODE_IDS, ANN_IDS)).toBeNull();
    // Exactly one member selected means there is no group to carry.
    expect(groupDragTargets('n1', new Set(['n1']), NODE_IDS, ANN_IDS)).toBeNull();
    expect(groupDragTargets('n1', new Set(), NODE_IDS, ANN_IDS)).toBeNull();
  });

  it('carries every OTHER selected node and all selected annotations', () => {
    const out = groupDragTargets('n1', new Set(['n1', 'n2', 'a1']), NODE_IDS, ANN_IDS);
    expect(out).not.toBeNull();
    // n3 was never selected; the grabbed node is excluded so the shared
    // delta is not applied to it twice.
    expect(out!.nodes).toEqual(['n2']);
    expect(out!.annotations).toEqual(['a1']);
  });

  it('keeps topology order so the delta applies deterministically', () => {
    const out = groupDragTargets('n2', new Set(['n1', 'n2', 'n3']), NODE_IDS, ANN_IDS);
    expect(out!.nodes).toEqual(['n1', 'n3']);
  });
});

describe('sharedSnappedDelta', () => {
  const snap8 = (v: number): number => Math.round(v / 8) * 8;

  it('snaps BOTH terms before subtracting so the group keeps its shape', () => {
    // Raw difference is 5, but both ends land in the same 8px cell, so a
    // member that shared the cell with the grabbed node must not drift.
    expect(sharedSnappedDelta(41, 36, snap8)).toBe(0);
    // Across cells BOTH ends snap first: 41 -> 40 and 4 -> 8, so the delta
    // is 32 -- a whole grid step -- never the raw 37. Snapping only one end
    // (40 - 4 = 36) would shear the group, which is the bug this guards.
    expect(sharedSnappedDelta(41, 4, snap8)).toBe(32);
  });

  it('is a plain difference when the place function is the identity', () => {
    expect(sharedSnappedDelta(7, 3, (v) => v)).toBe(4);
    expect(sharedSnappedDelta(7, 7, (v) => v)).toBe(0);
  });
});

function nodeAt(id: string) {
  const n = topologyStore.topology.nodes.find((x) => x.id === id);
  if (!n) throw new Error(`expected node ${id} in the topology`);
  return n;
}

function noteAt(id: string): Note {
  const a = topologyStore.topology.annotations?.find((x) => x.id === id);
  if (!a || a.kind !== 'note') throw new Error(`expected note ${id} in the topology`);
  return a;
}

/** Press → drag 40px right → release, all on one pointer id. */
async function drag(el: Element, from: { x: number; y: number }, to: { x: number; y: number }) {
  const base = { pointerId: 1, pointerType: 'mouse' as const };
  await fireEvent.pointerDown(el, { ...base, button: 0, clientX: from.x, clientY: from.y });
  await fireEvent.pointerMove(el, { ...base, clientX: to.x, clientY: to.y });
  await fireEvent.pointerUp(el, { ...base, button: 0, clientX: to.x, clientY: to.y });
  await tick();
}

afterEach(() => {
  clearSelection();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  if (topologyStore.topology.annotations) topologyStore.topology.annotations.length = 0;
});

describe('multi-selection group drag', () => {
  it('moves the grabbed node, the other selected node and the selected note by one delta', async () => {
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
    const el = container.querySelector(`[data-hit="node"][data-id="${a.id}"]`);
    expect(el).toBeInTheDocument();

    // toWorld = (client - 40) / 1 at the initial camera, so the press lands
    // inside a at world (60, 20); a 40px rightward drag is well over the
    // 4px mouse threshold.
    await drag(el!, { x: 100, y: 60 }, { x: 140, y: 60 });

    expect(nodeAt(a.id)).toMatchObject({ x: 40, y: 0 });
    expect(nodeAt(b.id)).toMatchObject({ x: 440, y: 0 });
    expect(noteAt(note.id)).toMatchObject({ x: 1040, y: 500 });
    // A press on a selected member must NOT collapse the selection.
    expect(topologyStore.selectedIds.size).toBe(3);
  });

  it('takes over an unselected node without dragging the old selection along', async () => {
    const a = makeNode('client', 0, 0);
    const b = makeNode('queue', 400, 0);
    const c = makeNode('db', 800, 0);
    topologyStore.topology.nodes.push(a, b, c);
    selectOne(a.id, false);
    selectOne(b.id, true);

    const { container } = render(Canvas);
    await tick();
    const el = container.querySelector(`[data-hit="node"][data-id="${c.id}"]`);
    expect(el).toBeInTheDocument();

    // world (860, 20) sits inside c; dragging it 40px right.
    await drag(el!, { x: 900, y: 60 }, { x: 940, y: 60 });

    expect(nodeAt(c.id)).toMatchObject({ x: 840, y: 0 });
    // The rest of the old selection never moves, and the press replaces
    // the selection with just this node (plain-click semantics).
    expect(nodeAt(a.id)).toMatchObject({ x: 0, y: 0 });
    expect(nodeAt(b.id)).toMatchObject({ x: 400, y: 0 });
    expect(topologyStore.selectedIds.size).toBe(1);
    expect(topologyStore.selectedIds.has(c.id)).toBe(true);
  });
});
