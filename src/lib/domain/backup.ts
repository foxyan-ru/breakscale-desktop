/**
 * Whole-app backup shapes.
 *
 * Mirrors `src/backup.ts` from the web app. The web app bundled
 * `localStorage` keys into one `.breakscale-backup.json`; the desktop app's
 * `persistence::backup` (Rust) bundles the app-data JSON files it owns
 * instead, replacing `BACKED_UP_KEYS` (a list of `localStorage` keys) with a
 * list of app-data file names -- see MIGRATION_PLAN.md §7. This file is
 * types only; `api/backup.ts` wraps the Tauri commands that build/restore.
 */

export const BACKUP_APP = 'breakscale-backup';
export const BACKUP_VERSION = 1;
export const BACKUP_EXT = '.breakscale-backup.json';

/**
 * Result of a restore attempt, matching the Rust `Result`-shaped JSON.
 *
 * `restored` names the app-data files that were actually written back;
 * empty (paired with `ok: false`) means the backup held nothing this
 * version could read.
 */
export type BackupResult =
  | { ok: true; restored: string[] }
  | { ok: false; error: string };
