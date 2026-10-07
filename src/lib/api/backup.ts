/**
 * Typed wrappers around the `backup_*` Tauri commands.
 *
 * This is the ONLY module allowed to call `invoke()` for whole-app backup --
 * every Svelte component and store goes through it. Call-site object keys
 * are camelCase; Tauri's IPC layer converts them to the Rust commands'
 * snake_case parameter names automatically.
 */

import { invoke } from '@tauri-apps/api/core';
import type { BackupResult } from '$lib/domain';

/** Bundle every app-data file this app owns into one backup at `path`. */
export async function backupWrite(path: string): Promise<void> {
  return invoke('backup_write', { path });
}

/** Restore a backup from an already-resolved path, replacing current state. */
export async function backupRestoreFromPath(path: string): Promise<BackupResult> {
  return invoke('backup_restore_from_path', { path });
}
