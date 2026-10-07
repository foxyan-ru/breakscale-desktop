import { render, fireEvent } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import Canvas from '../Canvas.svelte';
import { fitViewTo } from '../geometry';
import type { Rect } from '../edge-route';

/** The canvas's stores import the Tauri command wrappers transitively;
 *  keep every IPC bridge out of jsdom even though no test path calls it. */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

/** jsdom has no ResizeObserver, and Svelte's `bind:clientWidth` creates one
 *  unguarded the moment Canvas renders. Never fires: surfaceW/H stay 0. */
class ResizeObserverStub {
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;

const SURFACE: Rect = { x: 0, y: 0, w: 1200, h: 900 };

/** Dispatch a real cancelable keydown on window, like a student would. */
function press(code: string, opts: { shift?: boolean } = {}): KeyboardEvent {
  const ev = new KeyboardEvent('keydown', {
    code,
    shiftKey: opts.shift ?? false,
    bubbles: true,
    cancelable: true,
  });
  window.dispatchEvent(ev);
  return ev;
}

describe('fitViewTo', () => {
  it('frames an 800x500 diagram in a wide viewport at 1.09', () => {
    const out = fitViewTo([{ x: 100, y: 100, w: 800, h: 500 }], SURFACE, {
      x: 0,
      y: 0,
      w: 1000,
      h: 800,
    });
    expect(out).not.toBeNull();
    // Wide (>720) means a 64px margin: k = (1000-128)/800 = 1.09, which
    // beats the height ratio (672/500 = 1.344) and sits under FIT_MAX.
    expect(out!.k).toBe(1.09);
    expect(out!.x).toBeCloseTo(-45);
    expect(out!.y).toBeCloseTo(18.5);
  });

  it('uses the narrow margin at 720px wide and lands lower (0.84)', () => {
    const out = fitViewTo([{ x: 100, y: 100, w: 800, h: 500 }], SURFACE, {
      x: 0,
      y: 0,
      w: 720,
      h: 800,
    });
    expect(out!.k).toBe(0.84);
    expect(out!.x).toBeCloseTo(-60);
    expect(out!.y).toBeCloseTo(106);
  });

  it('clamps at FIT_MAX for a single small node', () => {
    const out = fitViewTo([{ x: 0, y: 0, w: 184, h: 88 }], SURFACE, {
      x: 0,
      y: 0,
      w: 1000,
      h: 800,
    });
    // Raw ratio would be ~4.7; the web app never fits tighter than 150%.
    expect(out!.k).toBe(1.5);
    expect(out!.x).toBeCloseTo(362);
    expect(out!.y).toBeCloseTo(334);
  });

  it('clamps at FIT_MIN for a diagram far larger than the stage', () => {
    const out = fitViewTo([{ x: 0, y: 0, w: 5000, h: 5000 }], SURFACE, {
      x: 0,
      y: 0,
      w: 1000,
      h: 800,
    });
    // Raw ratio would be ~0.134; never fit looser than the detail zoom,
    // so labels and glyphs stay legible after a fit.
    expect(out!.k).toBe(0.7);
    expect(out!.x).toBeCloseTo(-1250);
    expect(out!.y).toBeCloseTo(-1350);
  });

  it('returns null for an empty diagram or an unmeasured viewport', () => {
    expect(fitViewTo([], SURFACE, { x: 0, y: 0, w: 1000, h: 800 })).toBeNull();
    // `visible` defaults to `surface`, so an unmeasured surface is the
    // same null case -- jsdom's 0x0 getBoundingClientRect lands here.
    expect(fitViewTo([{ x: 0, y: 0, w: 184, h: 88 }], { x: 0, y: 0, w: 0, h: 0 })).toBeNull();
    const implicit = fitViewTo([{ x: 0, y: 0, w: 184, h: 88 }], SURFACE);
    const explicit = fitViewTo([{ x: 0, y: 0, w: 184, h: 88 }], SURFACE, SURFACE);
    expect(implicit).not.toBeNull();
    expect(implicit).toEqual(explicit);
  });

  it('measures against the visible frame, not the full surface', () => {
    // Panels to the left shrink the usable area, so the same diagram must
    // fit smaller than it does on a clear stage.
    const open = fitViewTo([{ x: 0, y: 0, w: 800, h: 500 }], SURFACE, {
      x: 400,
      y: 0,
      w: 800,
      h: 900,
    });
    const clear = fitViewTo([{ x: 0, y: 0, w: 800, h: 500 }], SURFACE, {
      x: 0,
      y: 0,
      w: 1200,
      h: 900,
    });
    expect(open!.k).toBeLessThan(clear!.k);
    // The offset stays surface-relative: x = visible.x - surface.x + centering.
    expect(open!.x).toBeGreaterThan(clear!.x);
  });
});

describe('fit chrome and shortcuts', () => {
  it('turns the level readout into a real fit button next to an icon fit button', () => {
    const { container, getByRole } = render(Canvas);

    const level = container.querySelector('.cv-zoom-level');
    expect(level).not.toBeNull();
    expect(level!.tagName).toBe('BUTTON');
    // Text stays "100%": the pre-existing zoom test reads this readout.
    expect(level!.textContent).toBe('100%');
    expect(level).toHaveAccessibleName('Zoom 100 percent. Fit the diagram on screen');
    expect(getByRole('button', { name: 'Fit the diagram on screen' })).toBeInTheDocument();
  });

  it('fit is a no-op while the stage is unmeasured instead of throwing', async () => {
    const { container, getByRole } = render(Canvas);

    await fireEvent.click(container.querySelector('.cv-zoom-level')!);
    await tick();
    await fireEvent.click(getByRole('button', { name: 'Fit the diagram on screen' }));
    await tick();

    // jsdom reports a 0x0 surface, so fitViewTo refuses and the camera
    // stays where it was rather than zooming to the 0.7 clamp.
    expect(container.querySelector('.cv-zoom-level')?.textContent).toBe('100%');
  });

  it('consumes Shift+1 but leaves an empty Shift+2 to the browser', async () => {
    render(Canvas);
    await tick();

    expect(press('Digit1', { shift: true }).defaultPrevented).toBe(true);
    expect(press('Digit2', { shift: true }).defaultPrevented).toBe(false);
    // Positional codes, so Shift+1 works on layouts where it means "!".
    expect(press('Digit1').defaultPrevented).toBe(false);
  });
});
