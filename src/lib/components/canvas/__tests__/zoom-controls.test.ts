import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';

/** The canvas's stores import the Tauri command wrappers transitively;
 *  keep every IPC bridge out of jsdom even though no test path calls it. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/**
 * Zoom chrome regression tests for the "no zoom in/out buttons" report.
 *
 * jsdom has no ResizeObserver, and Svelte's `bind:clientWidth` on the canvas
 * surface creates one unguarded the moment Canvas renders -- so the browser
 * API is stubbed before the first render. The stub never fires callbacks:
 * surfaceW/H stay 0, which is fine because zoom clamping does not depend on
 * viewport size (the buttons zoom about the centre, wherever that is).
 */
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

function levelText(container: Element): string | null {
  return container.querySelector('.cv-zoom-level')?.textContent ?? null;
}

describe('zoom chrome', () => {
  it('renders the bottom-right zoom cluster with chrome-marked buttons', () => {
    const { container, getByRole } = render(Canvas);

    const cluster = container.querySelector('.cv-zoom');
    expect(cluster).toBeInTheDocument();
    // data-chrome keeps canvas pointer gestures from treating a button
    // press as a background click / marquee start.
    expect(cluster).toHaveAttribute('data-chrome', 'zoom');
    expect(getByRole('button', { name: 'Zoom in' })).toBeInTheDocument();
    expect(getByRole('button', { name: 'Zoom out' })).toBeInTheDocument();
    expect(levelText(container)).toBe('100%');
    // The old bottom-left readout moved into the cluster (shell.css could
    // not lift it behind the metrics strip where it sat).
    expect(container.querySelector('.cv-zoom-readout')).not.toBeInTheDocument();
  });

  it('steps zoom in and out from the viewport centre', async () => {
    const { container, getByRole } = render(Canvas);

    await fireEvent.click(getByRole('button', { name: 'Zoom in' }));
    await tick();
    expect(levelText(container)).toBe('125%');

    await fireEvent.click(getByRole('button', { name: 'Zoom out' }));
    await tick();
    expect(levelText(container)).toBe('100%');
  });

  it('clamps at the MIN_ZOOM/MAX_ZOOM limits', async () => {
    const { container, getByRole } = render(Canvas);
    const zoomIn = getByRole('button', { name: 'Zoom in' });
    const zoomOut = getByRole('button', { name: 'Zoom out' });

    for (let i = 0; i < 8; i++) await fireEvent.click(zoomIn);
    await tick();
    expect(levelText(container)).toBe('250%');

    for (let i = 0; i < 12; i++) await fireEvent.click(zoomOut);
    await tick();
    expect(levelText(container)).toBe('40%');
  });
});
