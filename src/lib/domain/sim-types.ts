/**
 * Re-exports `$lib/sim/types` (a byte-for-byte copy of upstream's
 * `src/sim/types.ts`) under the domain barrel's existing path, so every
 * existing `import ... from '$lib/domain'` call site keeps working
 * unchanged. This file USED to be its own hand-maintained field-for-field
 * port of the same upstream source, documented as the wire contract with
 * the Rust engine (`src-tauri/src/sim/types.rs`); now that the simulation
 * engine itself lives in `$lib/sim/` and runs in-process (see
 * `state/simulation.svelte.ts`'s header comment for why), there is no wire
 * boundary left to mirror, and a second, independently-drifting copy here
 * would be pure risk for no benefit. `$lib/sim/types.ts` IS the source of
 * truth now -- it was already a strict superset of this file (it also has
 * `BulkheadMode`/`BULKHEAD_MODES`, upstream #77, which this file predated)
 * before this file was turned into a re-export.
 */
export * from '$lib/sim/types';
