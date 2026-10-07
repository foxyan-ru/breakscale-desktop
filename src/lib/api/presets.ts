/**
 * Typed wrappers around the `presets_list`/`preset_load` Tauri commands.
 *
 * NOTE(integration) in `$lib/components/shell/Examples.svelte` flagged these
 * as needed but not yet wrapped on the frontend. The backend commands
 * already exist (`src-tauri/src/commands/presets.rs`, wired into
 * `lib.rs`'s `generate_handler!`, backed by `sim::presets::all_presets()` /
 * `sim::presets::preset_by_id()`), so this file is purely the missing thin
 * wrapper, added in the same style as the other `api/*.ts` modules. Not
 * re-exported from `api/index.ts` (finished/do-not-modify for this pass) --
 * callers import this module directly.
 */

import { invoke } from '@tauri-apps/api/core';
import type { PresetSummary } from '$lib/components/shell/Examples.svelte';
import type { Topology } from '$lib/domain';

/** A preset's summary plus the topology it loads. Mirrors Rust `Preset`. */
export interface Preset extends PresetSummary {
  topology: Topology;
}

/** Every built-in example's browsable summary, without its topology. */
export async function presetsList(): Promise<PresetSummary[]> {
  return invoke('presets_list');
}

/** Load one example by id, topology included. */
export async function presetLoad(id: string): Promise<Preset> {
  return invoke('preset_load', { id });
}
