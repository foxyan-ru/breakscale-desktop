/**
 * Thin wrapper around `@tauri-apps/plugin-dialog`'s native open/save file
 * pickers.
 *
 * Used to resolve a filesystem path before calling one of the
 * `designFileWrite`/`designFileRead`/`sysdesignExport`/`backupWrite`/
 * `backupRestoreFromPath` functions in the other `api/*.ts` modules, which
 * all take an already-resolved path rather than picking one themselves.
 * This is the ONLY module allowed to call the dialog plugin directly.
 */

import { open, save } from '@tauri-apps/plugin-dialog';

/** Prompt for a file to open. Resolves null when the user cancels. */
export async function pickOpenPath(
  filters: { name: string; extensions: string[] }[],
): Promise<string | null> {
  const result = await open({ multiple: false, directory: false, filters });
  return typeof result === 'string' ? result : null;
}

/** Prompt for a save destination. Resolves null when the user cancels. */
export async function pickSavePath(
  defaultName: string,
  filters: { name: string; extensions: string[] }[],
): Promise<string | null> {
  const result = await save({ defaultPath: defaultName, filters });
  return result ?? null;
}

/**
 * Prompt for a destination DIRECTORY rather than a single file.
 *
 * Added for the system-design Terraform export (see MIGRATION_PLAN.md §8),
 * which writes a small file SET (`provider.tf`/`variables.tf`/`main.tf`/
 * `outputs.tf` plus a README) rather than one document -- `pickSavePath`
 * resolves a single file path and has no directory mode, so this fills that
 * gap using the same dialog plugin the rest of this module wraps. Resolves
 * null when the user cancels.
 */
export async function pickDirectory(): Promise<string | null> {
  const result = await open({ multiple: false, directory: true });
  return typeof result === 'string' ? result : null;
}
