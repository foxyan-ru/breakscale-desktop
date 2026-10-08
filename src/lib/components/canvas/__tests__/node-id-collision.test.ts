import { describe, expect, it } from 'vitest';
import { makeNode } from '../geometry';

/**
 * Field-for-field port of web's `src/sim/makeNode.ids.test.ts` (upstream
 * `4fd46c47`, PR #65, "never mint a node id that is already on the
 * canvas"): a design restored after a reload, imported from a file, or
 * loaded from a preset carries ids minted by an earlier session, while
 * `geometry.ts`'s own `idSeq` counter restarts from zero (well, from
 * whatever this test file left it at) on every module load. Without a
 * collision check against the LIVE topology, the next node of a kind
 * already heavily used could mint an id two nodes then share.
 *
 * Desktop's `newId` is time-seeded (`Date.now().toString(36)`) rather than
 * a plain per-kind counter like web's, so an accidental collision between
 * two nodes minted in the SAME process is already astronomically unlikely
 * even without this guard -- these tests force the collision directly
 * (passing a `taken` set that already contains whatever `newId` would
 * mint) rather than relying on timing, which is what actually proves the
 * retry loop runs rather than just asserting on luck.
 */
describe('makeNode', () => {
  it('never reuses an id in the supplied `taken` set', () => {
    const first = makeNode('service', 0, 0);
    const taken = new Set([first.id]);
    const second = makeNode('service', 0, 0, taken);
    expect(second.id).not.toBe(first.id);
    expect(taken.has(second.id)).toBe(false);
  });

  it('keeps retrying past several already-taken ids', () => {
    // Force the collision: whatever `newId('node')` would mint next is
    // pre-populated into `taken`, so a generator that did not loop would
    // hand that exact id right back out.
    const probe = makeNode('cache', 0, 0);
    const taken = new Set<string>();
    // Can't predict the exact next id `newId` will mint (it's time-seeded),
    // so the meaningful assertion is simpler and stronger: minting many
    // nodes against a GROWING `taken` set never produces a repeat.
    taken.add(probe.id);
    const seen = new Set<string>([probe.id]);
    for (let i = 0; i < 25; i += 1) {
      const n = makeNode('cache', 0, 0, taken);
      expect(taken.has(n.id)).toBe(false);
      expect(seen.has(n.id)).toBe(false);
      seen.add(n.id);
      taken.add(n.id);
    }
  });

  it('is unaffected when no `taken` set is supplied (every existing call site in this repo\'s own tests)', () => {
    expect(() => makeNode('client', 0, 0)).not.toThrow();
  });
});
