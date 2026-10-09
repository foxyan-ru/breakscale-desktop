/**
 * Re-exports `$lib/sim/annotations` (a byte-for-byte copy of upstream's
 * `src/sim/annotations.ts`) under the domain barrel's existing path, so
 * every existing `import ... from '$lib/domain'` call site keeps working
 * unchanged. This file USED to be its own hand-maintained field-for-field
 * port of the same upstream source; now that the simulation engine itself
 * lives in `$lib/sim/` (see `state/simulation.svelte.ts`'s header comment
 * for why), keeping a second, independently-drifting copy here would be
 * pure risk for no benefit -- `$lib/sim/annotations.ts` IS the source of
 * truth now, verified to have an identical export surface before this file
 * was turned into a re-export.
 */
export * from '$lib/sim/annotations';
