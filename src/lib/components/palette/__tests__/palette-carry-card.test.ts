import { render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Palette from '../Palette.svelte';
import { NODE_DND_MIME, KIND_NAME } from '$lib/components/canvas/geometry';

/**
 * Smoke test for upstream commit `dc9c1a07` ("carry a card, not a
 * screenshot of the row"), ported into this file's module script
 * (`buildCard` / `startCarry` / `moveCarry` / `endCarry`) and wired into
 * `handleDragStart` / `handleAnnDragStart` below.
 *
 * jsdom ships no real image decoder, so the module's `BLANK_DRAG_IMAGE`
 * (an `Image()` whose `src` is a data URI) never reaches `.complete`
 * here -- `startCarry` therefore always takes its documented static
 * fallback (`setStaticCardImage`), never the live follow-the-pointer
 * path. That fallback still builds the same `.pal-carry-card` and still
 * calls `dt.setDragImage`, which is the actual behavior being pinned:
 * the browser must never fall back to its own default row screenshot.
 * (Per CLAUDE.md's jsdom pitfalls note -- fireEvent returns a Promise and
 * jsdom lacks a real DataTransfer -- the drag event is built by hand with
 * a dataTransfer stub rather than going through fireEvent.dragStart.)
 */
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve(null)) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));

function fireDrag(type: 'dragstart' | 'dragend', el: Element, dataTransfer: unknown) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'dataTransfer', { value: dataTransfer, configurable: true });
  Object.defineProperty(event, 'clientX', { value: 120, configurable: true });
  Object.defineProperty(event, 'clientY', { value: 80, configurable: true });
  el.dispatchEvent(event);
}

describe('palette row carry card (dc9c1a07)', () => {
  afterEach(() => {
    // Belt-and-suspenders: drop any card/park/live-wrapper a test left
    // behind so later tests in the same jsdom document start clean.
    document.querySelectorAll('.pal-carry, .pal-carry-park').forEach((el) => el.remove());
  });

  it('builds a card from the row instead of handing the browser nothing to replace, and preserves the existing DnD contract', () => {
    const { getByTitle } = render(Palette, { props: { onAdd: vi.fn() } });

    // 'client' row -- KIND_HINT.client in Palette.svelte.
    const row = getByTitle('Sends requests at the rate you set');
    const dataTransfer = {
      setData: vi.fn(),
      effectAllowed: '',
      setDragImage: vi.fn(),
    };

    fireDrag('dragstart', row, dataTransfer);

    // The row's own existing drop contract (Canvas.svelte reads this) must
    // be untouched by the carry-card change -- this is a presentation-only
    // port, not a behavior change.
    expect(dataTransfer.setData).toHaveBeenCalledWith(NODE_DND_MIME, 'client');
    expect(dataTransfer.effectAllowed).toBe('copy');
    // The native per-row screenshot drag image is only ever avoided if
    // something always calls setDragImage with a built element.
    expect(dataTransfer.setDragImage).toHaveBeenCalled();

    const card = document.querySelector('.pal-carry-card');
    expect(card).toBeTruthy();
    expect(card?.getAttribute('data-kind')).toBe('client');
    expect(card?.querySelector('.pal-carry-name')?.textContent).toBe(KIND_NAME.client);

    fireDrag('dragend', row, dataTransfer);

    // endCarry() must not leave the live follow-the-pointer wrapper mounted
    // (it is never created on this fallback path, but asserting its
    // absence holds under the live path too).
    expect(document.querySelector('.pal-carry')).toBeFalsy();
  });

  it('arms the same carry card for an annotation row (Note)', () => {
    const { getByTitle } = render(Palette, {
      props: { onAdd: vi.fn(), onAddAnnotation: vi.fn() },
    });

    const row = getByTitle('Click, then click the canvas to place text (N)');
    const dataTransfer = {
      setData: vi.fn(),
      effectAllowed: '',
      setDragImage: vi.fn(),
    };

    fireDrag('dragstart', row, dataTransfer);

    expect(dataTransfer.setDragImage).toHaveBeenCalled();
    const card = document.querySelector('.pal-carry-card');
    expect(card).toBeTruthy();
    // Annotation rows carry no `data-kind` (they are not NodeKind rows) --
    // buildCard() already handles that via `if (kind !== null)`.
    expect(card?.hasAttribute('data-kind')).toBe(false);
    expect(card?.querySelector('.pal-carry-name')?.textContent).toBe('Note');

    fireDrag('dragend', row, dataTransfer);
  });
});
