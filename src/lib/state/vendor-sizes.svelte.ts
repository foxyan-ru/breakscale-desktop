/**
 * Svelte 5 rune store for which vendor instance size each component is
 * running on.
 *
 * Ported from `src/content/vendors/sizing.ts`. NOT part of `NodeConfig`, and
 * deliberately so: the engine never reads a vendor size -- picking
 * `db.r6g.xlarge` writes a `capacity` and a `sizeName` label the interface
 * keeps only so it can price the design and show what was chosen. That is
 * exactly the same reasoning `topology.svelte.ts` and `settings.svelte.ts`
 * already follow for UI-only state that mirrors but never replaces engine
 * state (`src/sim` is off-limits to solve an interface problem).
 *
 * Keyed by "<vendorId>:<nodeId>", exactly as `sizing.ts`'s own `key()` does,
 * so the same component priced on AWS and on Azure is two different answers:
 * a reader switching between them never finds their AWS choice
 * reinterpreted as an Azure size.
 *
 * PERSISTENCE. In-memory only for this pass, matching `settingsStore`'s own
 * TODO(integration) in `state/settings.svelte.ts`. The web app persisted
 * this to `localStorage['breakscale.sizes.v1']`; there is no `localStorage`
 * in a Tauri webview's backend context, and no `sizes_*`/`preferences_*`
 * command exists in the fixed `api/*.ts` contract yet. A relaunch currently
 * forgets every size chosen. Wire this to a `preferences.json`-style command
 * alongside `settingsStore`'s own TODO when one exists (MIGRATION_PLAN.md §7
 * already earmarks `preferences.json` in the app-data directory for exactly
 * this kind of per-person choice).
 */

import type { VendorId } from '$lib/domain';

const key = (vendor: VendorId, nodeId: string): string => `${vendor}:${nodeId}`;

function createVendorSizesStore() {
  const state = $state<Record<string, string>>({});
  return state;
}

/** `"<vendorId>:<nodeId>" -> sizeName`. Prefer the functions below over reading this directly. */
export const vendorSizesStore = createVendorSizesStore();

export function getSize(vendor: VendorId, nodeId: string): string | null {
  return vendorSizesStore[key(vendor, nodeId)] ?? null;
}

export function setSize(vendor: VendorId, nodeId: string, name: string): void {
  vendorSizesStore[key(vendor, nodeId)] = name;
}

/** Forget a node's size, for when the component is deleted. */
export function clearSize(vendor: VendorId, nodeId: string): void {
  delete vendorSizesStore[key(vendor, nodeId)];
}

export { key as sizeKey };
