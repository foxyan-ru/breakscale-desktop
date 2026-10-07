/**
 * Svelte 5 rune store for the current `SystemDesignDoc` (see
 * MIGRATION_PLAN.md §8 and `$lib/domain/system-design.ts`).
 *
 * A class rather than the `create*Store()` factory the other stores in this
 * directory use (`topology.svelte.ts`, `simulation.svelte.ts`, ...): this
 * feature's actions (`deriveFromCurrentTopology`, `save`, `load`, `exportTo`)
 * are naturally methods on the document the user is editing, called as
 * `sysdesignStore.exportTo(format)` from the components, rather than free
 * functions that take the store as an implicit global the way the topology
 * store's `addNode`/`removeNode` do. `$state` class fields work the same way
 * in a `.svelte.ts` module as the `$state<T>({...})` object literal those
 * other stores use -- both are compiled into the same reactive-proxy
 * machinery -- so this is a style choice, not a different reactivity model.
 *
 * EDIT FLOW. `mutate()` is the single funnel every structural edit (add/
 * remove a list row) should go through, since it also schedules the
 * debounced validate pass. A plain two-way `bind:value` on a text field
 * mutates the doc directly (bind can't call through `mutate`), so those
 * bindings pair with an explicit `oninput={() => sysdesignStore.scheduleValidate()}`
 * in the editor components instead -- see ArchitectureEditor.svelte /
 * LowLevelEditor.svelte.
 */

import {
  sysdesignDeriveHighLevel,
  sysdesignExport,
  sysdesignLoad,
  sysdesignSave,
  sysdesignValidate,
} from '$lib/api/sysdesign';
import { pickDirectory, pickSavePath } from '$lib/api/dialog';
import { isAppError } from '$lib/api';
import { pushError } from './ui.svelte';
import { topologyStore } from './topology.svelte';
import type { ExportFormat, SystemDesignDoc, ValidationIssue } from '$lib/domain';

/** Debounce window for validate-on-edit, matching a normal form-validation UX. */
const VALIDATE_DEBOUNCE_MS = 500;

const EXPORT_FILTERS: Record<Exclude<ExportFormat, 'terraform'>, { name: string; extensions: string[] }[]> = {
  json: [{ name: 'JSON', extensions: ['json'] }],
  yaml: [{ name: 'YAML', extensions: ['yaml', 'yml'] }],
  markdown: [{ name: 'Markdown', extensions: ['md'] }],
};

const EXPORT_EXTENSIONS: Record<Exclude<ExportFormat, 'terraform'>, string> = {
  json: 'json',
  yaml: 'yaml',
  markdown: 'md',
};

export interface ExportResult {
  status: 'ok' | 'cancelled' | 'error';
  /** Set only when `status === 'error'`. */
  message?: string;
}

function describe(e: unknown): string {
  if (isAppError(e)) return e.message;
  return e instanceof Error ? e.message : String(e);
}

let localIdSeq = 0;

/**
 * A short id for a NEW row a form adds locally (an external dependency, an
 * API contract, ...), in the same spirit as `savedDesigns.ts`'s `newId()`:
 * no `uuid` dependency, a timestamp plus a process-local counter is enough
 * uniqueness for something the user can see and delete in the same session.
 */
export function newLocalId(prefix: string): string {
  localIdSeq += 1;
  return `${prefix}-${Date.now().toString(36)}${localIdSeq.toString(36)}`;
}

class SysdesignStore {
  doc: SystemDesignDoc | null = $state(null);
  validationIssues: ValidationIssue[] = $state([]);
  /** True while a derive/save/load call is in flight -- not set during export, which has its own per-format status in ExportPanel. */
  busy: boolean = $state(false);

  private validateTimer: ReturnType<typeof setTimeout> | undefined;

  /** Seed a new `SystemDesignDoc` from the topology currently on the canvas. This is how a user starts. */
  async deriveFromCurrentTopology(): Promise<void> {
    this.busy = true;
    try {
      this.doc = await sysdesignDeriveHighLevel(topologyStore.topology);
      this.scheduleValidate();
    } catch (e) {
      pushError(`Generating a system design from the canvas failed: ${describe(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Persist the current document. No-op if there is nothing to save. */
  async save(): Promise<void> {
    if (!this.doc) return;
    this.busy = true;
    try {
      await sysdesignSave(this.doc);
    } catch (e) {
      pushError(`Saving the system design failed: ${describe(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /** Load a document by its `designId`, replacing the current one. */
  async load(designId: string): Promise<void> {
    this.busy = true;
    try {
      this.doc = await sysdesignLoad(designId);
      this.scheduleValidate();
    } catch (e) {
      pushError(`Loading the system design failed: ${describe(e)}`);
    } finally {
      this.busy = false;
    }
  }

  /**
   * Apply a structural edit (add/remove/reorder a list row) to the current
   * document and schedule a debounced validate pass. No-op if there is no
   * document yet -- every call site only renders once `doc` is non-null, but
   * this stays defensive against a stray call racing a load/reset.
   */
  mutate(fn: (doc: SystemDesignDoc) => void): void {
    if (!this.doc) return;
    fn(this.doc);
    this.scheduleValidate();
  }

  /** Debounced validate-on-edit. Public so a plain `bind:value` field (which can't route through `mutate`) can still trigger it from `oninput`. */
  scheduleValidate(): void {
    if (this.validateTimer) clearTimeout(this.validateTimer);
    this.validateTimer = setTimeout(() => {
      void this.runValidate();
    }, VALIDATE_DEBOUNCE_MS);
  }

  /** Run validation immediately, bypassing the debounce -- an explicit "check" action. */
  async validateNow(): Promise<void> {
    if (this.validateTimer) {
      clearTimeout(this.validateTimer);
      this.validateTimer = undefined;
    }
    await this.runValidate();
  }

  private async runValidate(): Promise<void> {
    const doc = this.doc;
    if (!doc) {
      this.validationIssues = [];
      return;
    }
    try {
      this.validationIssues = await sysdesignValidate(doc);
    } catch (e) {
      pushError(`Validating the system design failed: ${describe(e)}`);
    }
  }

  /**
   * Export the current document to `format`.
   *
   * Every format but `terraform` resolves a single save-file path via
   * `pickSavePath`. `terraform` emits a small file SET
   * (`provider.tf`/`variables.tf`/`main.tf`/`outputs.tf` plus a README of
   * assumptions -- see MIGRATION_PLAN.md §8), so it prompts for a destination
   * DIRECTORY via `pickDirectory` instead of a single file.
   *
   * Resolves `{ status: 'cancelled' }` rather than throwing when the user
   * dismisses the picker, so a caller can tell "nothing happened" apart from
   * a real failure.
   */
  async exportTo(format: ExportFormat): Promise<ExportResult> {
    const doc = this.doc;
    if (!doc) return { status: 'error', message: 'There is no system design to export yet.' };

    let path: string | null;
    if (format === 'terraform') {
      path = await pickDirectory();
    } else {
      const baseName = doc.name.trim() || 'system-design';
      path = await pickSavePath(`${baseName}.${EXPORT_EXTENSIONS[format]}`, EXPORT_FILTERS[format]);
    }
    if (!path) return { status: 'cancelled' };

    try {
      await sysdesignExport(doc, format, path);
      return { status: 'ok' };
    } catch (e) {
      const message = describe(e);
      pushError(`Exporting the system design to ${format} failed: ${message}`);
      return { status: 'error', message };
    }
  }
}

export const sysdesignStore = new SysdesignStore();
