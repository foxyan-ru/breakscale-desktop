/**
 * Named-save shapes.
 *
 * Mirrors `src/savedDesigns.ts` from the web app. The web app kept these in
 * `localStorage`; the desktop app persists them through Rust
 * (`persistence::saved_designs`, backed by a JSON file in the OS app-data
 * directory) via `api/designs.ts` -- see MIGRATION_PLAN.md §7. This file is
 * types only.
 */

import type { Topology } from './sim-types';

/**
 * How many designs are kept, and the longest name kept.
 *
 * Mirrored from the web app for UI copy (e.g. "20 saved, oldest will be
 * evicted"); the Rust side enforces these limits, not this constant.
 */
export const MAX_SAVED = 20;
export const MAX_NAME = 60;

export interface SavedDesign {
  /** Stable id, so a rename never orphans the entry. */
  id: string;
  name: string;
  /** Milliseconds since the epoch, for ordering and for "saved 2 hours ago". */
  savedAt: number;
  topology: Topology;
}

/** What the interface shows in a list, without paying to parse every topology. */
export interface SavedSummary {
  id: string;
  name: string;
  savedAt: number;
  nodeCount: number;
}

/**
 * Result of a save attempt, matching the Rust `Result`-shaped JSON.
 *
 * Saving over an existing NAME replaces that entry; the shelf is capped at
 * `MAX_SAVED`, and `evicted` names the entry dropped to make room, or null
 * when nothing was evicted.
 */
export type SaveResult =
  | { ok: true; id: string; evicted: string | null }
  | { ok: false; error: string };
