/**
 * Typed wrappers around the `sysdesign_*` Tauri commands.
 *
 * This is the ONLY module allowed to call `invoke()` for the system-design
 * feature -- every Svelte component and store goes through it. Call-site
 * object keys are camelCase; Tauri's IPC layer converts them to the Rust
 * commands' snake_case parameter names automatically.
 */

import { invoke } from '@tauri-apps/api/core';
import type { ExportFormat, SystemDesignDoc, Topology, ValidationIssue } from '$lib/domain';

/** Seed a full `SystemDesignDoc` from a topology (see MIGRATION_PLAN.md §8). */
export async function sysdesignDeriveHighLevel(topology: Topology): Promise<SystemDesignDoc> {
  return invoke('sysdesign_derive_high_level', { topology });
}

/** Persist a system-design document. */
export async function sysdesignSave(doc: SystemDesignDoc): Promise<void> {
  return invoke('sysdesign_save', { doc });
}

/** Load a system-design document by its `designId`. */
export async function sysdesignLoad(designId: string): Promise<SystemDesignDoc> {
  return invoke('sysdesign_load', { designId });
}

/** Validate a document, field by field, without saving it. */
export async function sysdesignValidate(doc: SystemDesignDoc): Promise<ValidationIssue[]> {
  return invoke('sysdesign_validate', { doc });
}

/** Export a document to `format` at an already-resolved path. */
export async function sysdesignExport(
  doc: SystemDesignDoc,
  format: ExportFormat,
  path: string,
): Promise<void> {
  return invoke('sysdesign_export', { doc, format, path });
}
