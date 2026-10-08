import { describe, expect, it } from 'vitest';
import { suggestionFor } from '../suggestions';
import { HAS_THROUGHPUT_CEILING } from '$lib/components/inspector/field-schema';
import type { NodeKind } from '$lib/domain';

/**
 * The bar these guard is not "some text came back". It is that the two kinds
 * with a tempting-but-wrong obvious fix do not offer it: a db told to just
 * get bigger, and a cache told to raise its hit rate, are both advice that
 * moves nothing for the node actually over its ceiling. Port of upstream
 * `src/content/suggestions.test.ts` (`eb163673`, PR #71).
 *
 * `KINDS_WITH_CEILING` is read from this repo's own `HAS_THROUGHPUT_CEILING`
 * (field-schema.ts) rather than copied as a literal list a second time, so
 * this file cannot drift from what `Inspector.svelte` actually gates on --
 * the gap the upstream PR's last commit specifically closed by driving its
 * own component test off `KIND_NAME` instead of a hand-copied list.
 */
const KINDS_WITH_CEILING = [...HAS_THROUGHPUT_CEILING] as NodeKind[];

describe('suggestionFor', () => {
  it('never tells a database to just get bigger', () => {
    const text = suggestionFor('db')!;
    expect(text).not.toMatch(/add (more )?(capacity|instances)/i);
    expect(text).toMatch(/reaching it/i);
  });

  it('tells a saturated cache that hit rate is not the lever', () => {
    // A hit and a miss both occupy a slot for serviceMs, so hit rate changes
    // what the cache forwards, never what it has to get through.
    const text = suggestionFor('cache')!;
    expect(text).toMatch(/hit rate will not help/i);
  });

  it('offers nothing for kinds with no throughput ceiling', () => {
    const kindsWithoutCeiling: NodeKind[] = ['queue', 'autoscaler', 'ratelimiter', 'client'];
    for (const kind of kindsWithoutCeiling) {
      expect(suggestionFor(kind)).toBeNull();
    }
  });

  it('has a suggestion for every kind with a throughput ceiling', () => {
    for (const kind of KINDS_WITH_CEILING) {
      expect(suggestionFor(kind)).toBeTruthy();
    }
  });

  it('uses no em dashes', () => {
    // Matches this repo's own WHY-comment convention of plain "--" rather
    // than an em dash; mirrors upstream CONTRIBUTING's writing rule that PR
    // #71's own history had to go back and fix.
    const offenders = KINDS_WITH_CEILING.filter((kind) => suggestionFor(kind)?.includes('—'));
    expect(offenders).toEqual([]);
  });
});
