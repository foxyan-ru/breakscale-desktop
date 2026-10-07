import { describe, expect, it } from 'vitest';
import { labelDyById, routeEdge } from '../edge-route';
import type { LabelDyEntry, Rect } from '../edge-route';

/**
 * Label de-confliction for coincident rate labels.
 *
 * The fixture is a two-row CROSSED fan, not a single-source fan-out: a
 * fan from one source to non-overlapping targets can never collide,
 * because the anchors spread along the target row. Here three sources sit
 * on the top row and three targets on the bottom row in REVERSE x order,
 * so S1->Ta, S2->Tb and S3->Tc all share one mid-leg -- and because the
 * paired x sums are all 2000, all three anchors land on the identical
 * point (1087, 194). That is the shape the web app measured as "3k/3k/s
 * garble".
 *
 * 184x88 is NODE_W/NODE_H, spelled out so the arithmetic below is checkable
 * by eye: start.x = a.x + 184, end.x = b.x - 2 (tip backoff) - 8 (arrow
 * inset), mid = (start.x + end.x) / 2, y = (44 + 344) / 2.
 */
const box = (x: number, y: number): Rect => ({ x, y, w: 184, h: 88 });

const S1 = box(0, 0);
const S2 = box(400, 0);
const S3 = box(800, 0);
const TA = box(2000, 300);
const TB = box(1600, 300);
const TC = box(1200, 300);

/** An edge whose label renders (live flow), as the template would see it. */
function live(id: string, a: Rect | null, b: Rect | null): LabelDyEntry {
  return { id, a, b, lane: 0, control: false, severed: false, flow: 12 };
}

describe('labelDyById', () => {
  it('fixture: the three crossed-fan edges really do anchor identically', () => {
    for (const [a, b] of [
      [S1, TA],
      [S2, TB],
      [S3, TC],
    ] as const) {
      expect(routeEdge(a, b, 0).label).toEqual({ x: 1087, y: 194 });
    }
    // The fourth edge takes a different corridor, so it must NOT collide.
    expect(routeEdge(S1, TC, 0).label).toEqual({ x: 687, y: 194 });
  });

  it('staggers the 2nd and 3rd label landing in one bucket, leaves the 1st alone', () => {
    const out = labelDyById(
      [live('e1', S1, TA), live('e2', S2, TB), live('e3', S3, TC)],
      true,
    );
    // Bucket "68:12" (1087/16, 194/16): first label needs no offset, so it
    // has no entry at all -- the template falls back to 0.
    expect(out.has('e1')).toBe(false);
    expect(out.get('e2')).toBe(10);
    expect(out.get('e3')).toBe(20);
    // The unrelated corridor stays untouched.
    expect(labelDyById([live('e4', S1, TC)], true).has('e4')).toBe(false);
  });

  it('returns an empty map when labels are hidden', () => {
    expect(
      labelDyById([live('e1', S1, TA), live('e2', S2, TB)], false).size,
    ).toBe(0);
  });

  it('lets a label render only for participants: idle/severed edges never displace a live one', () => {
    const idle: LabelDyEntry = {
      ...live('e2', S2, TB),
      flow: 0,
      severed: false,
      control: false,
    };
    const out = labelDyById([live('e1', S1, TA), idle, live('e3', S3, TC)], true);
    expect(out.has('e2')).toBe(false);
    // e2 was skipped entirely, so e3 is only the SECOND participant and
    // gets 10, not 20.
    expect(out.get('e3')).toBe(10);
  });

  it('counts control edges even with no flow, and severed ones never', () => {
    const control: LabelDyEntry = {
      ...live('c1', S1, TA),
      control: true,
      flow: 0,
    };
    const severed: LabelDyEntry = { ...live('c2', S2, TB), severed: true, flow: 50 };
    const out = labelDyById([control, severed, live('c3', S3, TC)], true);
    expect(out.has('c1')).toBe(false); // first participant
    expect(out.has('c2')).toBe(false); // skipped, not a participant
    expect(out.get('c3')).toBe(10); // second participant overall
  });

  it('ignores an edge with a missing endpoint instead of throwing', () => {
    const out = labelDyById(
      [live('broken', null, TA), live('e1', S1, TA), live('e2', S2, TB)],
      true,
    );
    expect(out.has('broken')).toBe(false);
    expect(out.has('e1')).toBe(false);
    expect(out.get('e2')).toBe(10);
  });

  it('separates labels that land in different buckets', () => {
    const out = labelDyById([live('e1', S1, TA), live('e4', S1, TC)], true);
    expect(out.size).toBe(0);
  });
});
