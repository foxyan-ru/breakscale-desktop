import { describe, expect, it } from 'vitest';
import { snapsToGrid } from '../geometry';

/* ------------------------------------------------------------------ *
 * Grid snap
 *
 * Field-for-field port of web's `Canvas.pointer.test.ts` "grid snap" suite
 * (upstream `1db4ac61`, PR #37): the preference used to be decoration on
 * the web app too -- stored, defaulted, sanitised on read and offered as a
 * Settings switch, with every drag choosing its rounding from the Ctrl key
 * alone. A typecheck cannot see a preference nobody reads and neither can a
 * screenshot, so the DECISION is pinned directly rather than any one
 * drag's behaviour.
 * ------------------------------------------------------------------ */

describe('snapsToGrid', () => {
  it('snaps when the preference is on and nothing is held', () => {
    expect(snapsToGrid(true, false)).toBe(true);
  });

  it('does not snap when the preference is off', () => {
    expect(snapsToGrid(false, false)).toBe(false);
  });

  it('lets Ctrl bypass the snap for the drag in progress', () => {
    expect(snapsToGrid(true, true)).toBe(false);
  });

  it('never turns snapping back on: Ctrl only ever loosens', () => {
    expect(snapsToGrid(false, true)).toBe(false);
  });
});
