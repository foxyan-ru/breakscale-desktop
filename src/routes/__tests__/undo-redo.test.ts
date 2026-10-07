import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Page from '../+page.svelte';
import { makeNode } from '$lib/components/canvas/geometry';
import { topologyStore, clearSelection } from '$lib/state/topology.svelte';
import { sessionHistory, currentSnapshot } from '$lib/state/history.svelte';

/** Keep every IPC bridge out of jsdom: the page pulls in the Tauri command
 *  wrappers transitively, and its bootstrap fires `presets_list` on mount. */
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
 * Drain the SHARED history (this file drives the app's real singleton) so
 * each case starts with both stacks empty. The stacks are complementary --
 * undoing everything fills the redo stack and vice versa -- so one
 * direction can never empty both. The sentinel commit wipes the redo stack
 * outright (`commit` sets `future = []`), and because it captures the
 * state as it stands, the undo that follows discards it through the no-op
 * skip rather than pushing it back. Nothing is APPLIED, so the topology
 * the test set up survives.
 */
function drainHistory(): void {
  while (sessionHistory.canUndo) sessionHistory.undo(currentSnapshot());
  if (sessionHistory.canRedo) {
    sessionHistory.commit('drain', currentSnapshot());
    sessionHistory.undo(currentSnapshot());
  }
  sessionHistory.receipt = null;
}

function control(container: HTMLElement, label: 'Undo' | 'Redo'): HTMLButtonElement {
  const el = container.querySelector(`button[aria-label="${label}"]`);
  if (!el) throw new Error(`${label} button did not render`);
  return el as HTMLButtonElement;
}

function paletteRow(container: HTMLElement, kind: string): HTMLButtonElement {
  const el = container.querySelector(`[data-kind="${kind}"]`);
  if (!el) throw new Error(`palette row for ${kind} did not render`);
  return el as HTMLButtonElement;
}

beforeEach(() => {
  drainHistory();
  topologyStore.topology.nodes.length = 0;
  topologyStore.topology.edges.length = 0;
  if (topologyStore.topology.annotations) topologyStore.topology.annotations.length = 0;
  clearSelection();
  // One client to add a second one to (and to give the load slider a
  // traffic source; the bootstrap's preset fetch fails in jsdom, so this
  // is the only topology the page will see).
  topologyStore.topology.nodes.push(makeNode('client', 120, 80));
});

describe('header undo/redo controls', () => {
  it('renders both buttons with accessible names, disabled until there is something to undo', async () => {
    const { container } = render(Page);
    await tick();

    const undo = control(container, 'Undo');
    const redo = control(container, 'Redo');
    expect(undo).toHaveAttribute('title', 'Undo (Ctrl+Z)');
    expect(redo).toHaveAttribute('title', 'Redo (Ctrl+Shift+Z)');
    expect(undo).toBeDisabled();
    expect(redo).toBeDisabled();
    // Real buttons, not spans styled like them: keyboard and screen reader
    // reach them without extra wiring.
    expect(undo.tagName).toBe('BUTTON');
    expect(redo.tagName).toBe('BUTTON');
  });

  it('undoes a palette add and redoes it, restoring the selection with it', async () => {
    const { container } = render(Page);
    await tick();

    await fireEvent.click(paletteRow(container, 'client'));
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(2);
    const added = topologyStore.topology.nodes[1].id;
    expect(topologyStore.selectedIds.has(added)).toBe(true);

    const undo = control(container, 'Undo');
    const redo = control(container, 'Redo');
    expect(undo).toBeEnabled();
    expect(redo).toBeDisabled();

    await fireEvent.click(undo);
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(1);
    expect(undo).toBeDisabled();
    expect(redo).toBeEnabled();
    // The entry carried the PRE-add selection (empty), not the post-add one.
    expect(topologyStore.selectedIds.size).toBe(0);

    // The receipt names what was reverted, and is announced politely.
    const toast = container.querySelector('.app-toast');
    expect(toast).toHaveTextContent('Undid add');
    expect(toast).toHaveAttribute('role', 'status');

    await fireEvent.click(redo);
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(2);
    expect(topologyStore.selectedIds.has(added)).toBe(true);
    expect(container.querySelector('.app-toast')).toHaveTextContent('Redid add');
  });

  it('takes Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y from the window', async () => {
    const { container } = render(Page);
    await tick();

    await fireEvent.click(paletteRow(container, 'client'));
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(2);

    await fireEvent.keyDown(window, { code: 'KeyZ', ctrlKey: true });
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(1);

    await fireEvent.keyDown(window, { code: 'KeyZ', ctrlKey: true, shiftKey: true });
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(2);

    await fireEvent.keyDown(window, { code: 'KeyZ', ctrlKey: true });
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(1);

    await fireEvent.keyDown(window, { code: 'KeyY', ctrlKey: true });
    await tick();
    expect(topologyStore.topology.nodes).toHaveLength(2);
  });

  it('ignores Ctrl+Z while a text field has focus', async () => {
    const { container } = render(Page);
    await tick();

    await fireEvent.click(paletteRow(container, 'client'));
    await tick();

    const slider = container.querySelector('#traffic-rps');
    expect(slider).toBeInTheDocument();
    await fireEvent.keyDown(slider!, { code: 'KeyZ', ctrlKey: true });
    await tick();

    // No undo: the chord belonged to whatever the student is typing.
    expect(topologyStore.topology.nodes).toHaveLength(2);
    expect(sessionHistory.canUndo).toBe(true);
  });
});
