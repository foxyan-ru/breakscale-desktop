import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Inspector from '../Inspector.svelte';
import { topologyStore, select, clearSelection } from '$lib/state/topology.svelte';
import { makeNode } from '$lib/components/canvas/geometry';

/**
 * The inspector is hard to leave (Escape and a background click only),
 * so the panel header got an explicit close button. These tests seed the
 * selection directly and assert the button deselects for real.
 *
 * The Tauri layers the stores import transitively are mocked so nothing
 * ever reaches a runtime IPC bridge in jsdom; closing itself is pure
 * state (`clearSelection` only clears the selection set).
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

describe('inspector close button', () => {
  beforeEach(() => {
    const node = makeNode('client', 0, 0);
    topologyStore.topology.nodes.push(node);
    select(node.id, null);
  });

  afterEach(() => {
    clearSelection();
    topologyStore.topology.nodes.length = 0;
  });

  it('shows a close button for the selected node', () => {
    const { getByRole } = render(Inspector);

    const node = topologyStore.topology.nodes[0];
    expect(getByRole('heading', { level: 2 })).toHaveTextContent(node.label);
    expect(getByRole('button', { name: 'Close inspector' })).toBeInTheDocument();
  });

  it('closes the inspector by clearing the selection', async () => {
    const { getByRole, queryByRole } = render(Inspector);

    await fireEvent.click(getByRole('button', { name: 'Close inspector' }));
    await tick();

    expect(topologyStore.selectedIds.size).toBe(0);
    expect(topologyStore.selectedNodeId).toBeNull();
    // No node selected -> the panel falls back to its empty state.
    expect(queryByRole('button', { name: 'Close inspector' })).not.toBeInTheDocument();
    expect(queryByRole('heading', { level: 2 })).not.toBeInTheDocument();
  });
});
