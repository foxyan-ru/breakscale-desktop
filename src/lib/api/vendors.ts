/**
 * Typed wrapper around a `vendors_get` Tauri command this port needs but
 * which does not exist yet in the fixed command contract as of this pass
 * (`api/sim.ts`, `api/designs.ts`, `api/backup.ts`, `api/sysdesign.ts`,
 * `api/dialog.ts` and their barrel `api/index.ts` are marked
 * finished/read-only; none of them touch vendor specs).
 *
 * NOTE(integration): backend command "vendors_get" not yet implemented as of
 * this port; add a matching #[tauri::command] in commands/vendors.rs during
 * integration. Intended shape:
 *
 *   vendors_get(vendorId: VendorId) -> Vendor | null
 *
 * Returning the full published spec (per-kind product mapping + sizes) for
 * one vendor, backed by the embedded `data/vendors/*.json` files and
 * `vendors::lookup` described in MIGRATION_PLAN.md §4 -- the Rust-side
 * equivalent of the web app's `loadVendor()` (`src/content/vendors/lookup.ts`),
 * collapsed into one request/response round trip since Tauri IPC has no
 * "lazy `import()`" to mirror. `'generic'` has no data file (see
 * `src/content/vendors/index.ts`'s `GENERIC` constant) and this wrapper
 * resolves it to `null` locally, without a round trip, matching
 * `loadVendor`/`peekVendor`'s own special case.
 *
 * NOT re-exported from `api/index.ts`: that barrel is one of the files
 * marked finished/do-not-modify for this pass, so it has no
 * `export * from './vendors'` line. Callers import this module directly --
 * `import { vendorsGet } from '$lib/api/vendors'` -- until integration adds
 * that export.
 */

import { invoke } from '@tauri-apps/api/core';
import type { Vendor, VendorId } from '$lib/domain';

/**
 * In-memory cache, mirroring the web app's `lookup.ts` module-level `Map`:
 * a reader who never leaves generic never issues the round trip, and
 * switching back to a vendor already fetched this session is instant rather
 * than re-invoking IPC.
 */
const cache = new Map<VendorId, Vendor | null>();

/** Fetch (and cache) one vendor's full spec. `'generic'` always resolves to `null`. */
export async function vendorsGet(vendorId: VendorId): Promise<Vendor | null> {
  if (vendorId === 'generic') return null;
  const hit = cache.get(vendorId);
  if (hit !== undefined) return hit;
  try {
    const vendor = await invoke<Vendor | null>('vendors_get', { vendorId });
    cache.set(vendorId, vendor);
    return vendor;
  } catch {
    // A vendor spec that fails to load leaves the panel on generic names
    // rather than throwing through a render path -- the vendor data is a
    // labelling convenience, never something the simulation depends on,
    // mirroring `loadVendor`'s own try/catch.
    return null;
  }
}

/**
 * Synchronous read of an already-fetched vendor, for render paths that must
 * not await -- mirrors `peekVendor` in `src/content/vendors/lookup.ts`.
 */
export function peekVendor(vendorId: VendorId): Vendor | null {
  return vendorId === 'generic' ? null : (cache.get(vendorId) ?? null);
}
