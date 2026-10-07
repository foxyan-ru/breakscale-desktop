/**
 * Typed wrappers around the `designs_*` and `design_file_*` Tauri commands.
 *
 * This is the ONLY module allowed to call `invoke()` for saved-design and
 * design-file persistence -- every Svelte component and store goes through
 * it. Call-site object keys are camelCase; Tauri's IPC layer converts them
 * to the Rust commands' snake_case parameter names automatically.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  DesignParseResult,
  SavedDesign,
  SavedSummary,
  SaveResult,
  Topology,
} from '$lib/domain';

/** List every saved design, newest first, without their topologies. */
export async function designsList(): Promise<SavedSummary[]> {
  return invoke('designs_list');
}

/** Save `topology` under `name`. Saving over an existing name replaces it. */
export async function designsSave(name: string, topology: Topology): Promise<SaveResult> {
  return invoke('designs_save', { name, topology });
}

/** Load one saved design by id. Rejects (throws) if it no longer exists. */
export async function designsGet(id: string): Promise<SavedDesign> {
  return invoke('designs_get', { id });
}

/** Delete one saved design by id. */
export async function designsDelete(id: string): Promise<void> {
  return invoke('designs_delete', { id });
}

/** Rename a saved design. Resolves false when the name is empty or taken. */
export async function designsRename(id: string, name: string): Promise<boolean> {
  return invoke('designs_rename', { id, name });
}

/** Build the text of a `.breakscale` file for `topology`, without writing it. */
export async function designFileBuild(
  topology: Topology,
  name?: string | null,
): Promise<string> {
  return invoke('design_file_build', { topology, name: name ?? null });
}

/** Validate and parse the text of a `.breakscale` file already in memory. */
export async function designFileParse(text: string): Promise<DesignParseResult> {
  return invoke('design_file_parse', { text });
}

/** Build and write a `.breakscale` file to an already-resolved path. */
export async function designFileWrite(
  path: string,
  topology: Topology,
  name?: string | null,
): Promise<void> {
  return invoke('design_file_write', { path, topology, name: name ?? null });
}

/** Read and validate a `.breakscale` file from an already-resolved path. */
export async function designFileRead(path: string): Promise<DesignParseResult> {
  return invoke('design_file_read', { path });
}
