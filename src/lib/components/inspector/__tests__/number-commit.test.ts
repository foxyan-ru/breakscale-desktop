import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import Inspector from '../Inspector.svelte';
import { specFor, type RangeFieldSpec } from '../field-schema';
import { topologyStore, select, clearSelection } from '$lib/state/topology.svelte';
import { makeNode } from '$lib/components/canvas/geometry';
import type { SimNode } from '$lib/domain';

/**
 * Bug 4: the Inspector's typed number commits were ungated.
 *
 * `Number('')` is 0, `Number('abc')` is NaN, and `type="number"` reports
 * both without complaint -- so a cleared box or a typo travelled straight
 * into `updateNodeConfig`, which pushes `NaN` through JSON as `null`. The
 * Rust side merges patches with `serde_json::from_value::<NodeConfig>`
 * (engine.rs), which rejects the null, while the optimistic store had
 * already shown the value: the panel then displayed a number the engine
 * never took, and it stayed wrong until the next node selection. The fix
 * rejects empty/NaN (restoring the box to the node's live value) and
 * clamps to the field's own min/max -- the same interval the input's
 * `min`/`max` attributes promise.
 *
 * These tests seed the selection exactly as `inspector-close.test.ts` does
 * and assert against the IPC bridge: no write reaches Rust unless it is a
 * finite, in-range number.
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

/** The number input for `capacity`, the field these tests drive. */
function capacityInput(container: Element): HTMLInputElement {
  const input = container.querySelector('#ins-capacity');
  if (!input) throw new Error('capacity field did not render');
  return input as HTMLInputElement;
}

describe('typed number commits', () => {
  let node: SimNode;

  beforeEach(() => {
    node = makeNode('service', 0, 0);
    topologyStore.topology.nodes.push(node);
    select(node.id, null);
    mockedInvoke.mockClear();
  });

  afterEach(() => {
    clearSelection();
    topologyStore.topology.nodes.length = 0;
  });

  function capacity(): number {
    const n = topologyStore.topology.nodes.find((x) => x.id === node.id);
    return n ? n.config.capacity : NaN;
  }

  it('commits a valid typed number, in range', async () => {
    const { container } = render(Inspector);
    mockedInvoke.mockClear();

    await fireEvent.change(capacityInput(container), { target: { value: '20' } });
    await tick();

    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).toHaveBeenCalledWith('sim_update_node_config', {
      nodeId: node.id,
      patch: { capacity: 20 },
    });
    expect(capacity()).toBe(20);
  });

  it('does not write an emptied box -- and puts the real value back', async () => {
    const { container } = render(Inspector);
    const input = capacityInput(container);
    const before = capacity();
    mockedInvoke.mockClear();

    await fireEvent.change(input, { target: { value: '' } });
    await tick();

    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(capacity()).toBe(before);
    // The box shows what the engine has, not what was typed: an empty
    // field that stayed empty would read as a value of "".
    expect(capacityInput(container).value).toBe(String(before));
  });

  it('does not write non-numeric text', async () => {
    const { container } = render(Inspector);
    const before = capacity();
    mockedInvoke.mockClear();

    // jsdom may keep "abc" or sanitise it to "" depending on how much of
    // the number-input sanitization it implements; both readings must end
    // in the same place -- no IPC, store untouched.
    await fireEvent.change(capacityInput(container), { target: { value: 'abc' } });
    await tick();

    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(capacity()).toBe(before);
    expect(capacityInput(container).value).toBe(String(before));
  });

  it('clamps an out-of-range number to the field\'s own max, not to null', async () => {
    const { container } = render(Inspector);
    const spec = specFor('service', 'capacity') as RangeFieldSpec;
    mockedInvoke.mockClear();

    await fireEvent.change(capacityInput(container), {
      target: { value: String(spec.max + 999) },
    });
    await tick();

    // What reaches Rust is the in-range number; nothing arrives as null.
    expect(mockedInvoke).toHaveBeenCalledWith('sim_update_node_config', {
      nodeId: node.id,
      patch: { capacity: spec.max },
    });
    expect(capacity()).toBe(spec.max);
    expect(capacityInput(container).value).toBe(String(spec.max));
  });

  it('clamps below the field\'s min too', async () => {
    const { container } = render(Inspector);
    const spec = specFor('service', 'capacity') as RangeFieldSpec;
    mockedInvoke.mockClear();

    await fireEvent.change(capacityInput(container), { target: { value: '-5' } });
    await tick();

    expect(mockedInvoke).toHaveBeenCalledWith('sim_update_node_config', {
      nodeId: node.id,
      patch: { capacity: spec.min },
    });
    expect(capacity()).toBe(spec.min);
  });
});
