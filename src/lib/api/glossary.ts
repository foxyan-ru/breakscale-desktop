/**
 * Typed wrapper around the `glossary_list` Tauri command.
 *
 * NOTE(integration) in `$lib/components/glossary/Glossary.svelte` and
 * `$lib/components/shell/Tooltip.svelte` flagged a `glossary_list` command
 * as needed but not yet wrapped on the frontend. The backend command itself
 * already exists (`src-tauri/src/commands/glossary.rs`, wired into
 * `lib.rs`'s `generate_handler!`), so this file is purely the missing thin
 * wrapper, added in the same style as the other `api/*.ts` modules (a
 * one-line `invoke()` call, camelCase args). Not re-exported from
 * `api/index.ts`: that barrel is finished/do-not-modify for this pass (see
 * `api/vendors.ts` and `api/challenges.ts` for the same situation) --
 * callers import this module directly.
 */

import { invoke } from '@tauri-apps/api/core';
import type { GlossaryEntry } from '$lib/components/glossary/Glossary.svelte';

/** Every glossary entry, in file order. */
export async function glossaryList(): Promise<GlossaryEntry[]> {
  return invoke('glossary_list');
}
