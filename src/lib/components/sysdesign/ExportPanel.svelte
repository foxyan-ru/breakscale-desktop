<script lang="ts">
  /**
   * One button per `ExportFormat` (see `$lib/domain/system-design.ts` and
   * MIGRATION_PLAN.md §8's "Export formats" section, paraphrased here for a
   * user rather than a developer), each calling `sysdesignStore.exportTo(format)`
   * and showing its own success/error state once that call resolves or
   * rejects.
   *
   * Unlike ArchitectureEditor/LowLevelEditor this panel does not show the
   * full "generate from canvas" empty state -- that CTA belongs to the two
   * editors the user would reach first. With no document yet, the buttons
   * are simply disabled with a short explanatory note.
   */
  import { sysdesignStore } from '$lib/state/sysdesign.svelte';
  import type { ExportFormat } from '$lib/domain';

  const doc = $derived(sysdesignStore.doc);

  interface FormatInfo {
    format: ExportFormat;
    label: string;
    description: string;
  }

  const FORMATS: FormatInfo[] = [
    {
      format: 'json',
      label: 'JSON',
      description: 'The full document, machine-readable -- for feeding into another tool or storing alongside the design file.',
    },
    {
      format: 'yaml',
      label: 'YAML',
      description: 'The same document as JSON, in a format that reads better in a code review or a docs repo.',
    },
    {
      format: 'markdown',
      label: 'Markdown',
      description:
        'A readable write-up: overview, component and data-flow tables, quality attributes, API contracts, entity models, sequence flows, state machines and deployment -- ready to paste into a wiki or PR description.',
    },
    {
      format: 'terraform',
      label: 'Terraform',
      description:
        'Infrastructure-as-code scaffolding for the deployment section. This produces a small SET of files (provider.tf, variables.tf, main.tf, outputs.tf) plus a README explaining what was assumed -- no VPC/network/IAM, and any component with no known mapping for the chosen vendor is left as a clearly marked manual step. Because it writes several files at once, this picks a destination FOLDER instead of a single file.',
    },
  ];

  interface FormatStatus {
    state: 'idle' | 'pending' | 'ok' | 'cancelled' | 'error';
    message?: string;
  }

  const status: Record<ExportFormat, FormatStatus> = $state({
    json: { state: 'idle' },
    yaml: { state: 'idle' },
    markdown: { state: 'idle' },
    terraform: { state: 'idle' },
  });

  async function handleExport(format: ExportFormat): Promise<void> {
    status[format] = { state: 'pending' };
    const result = await sysdesignStore.exportTo(format);
    if (result.status === 'ok') {
      status[format] = { state: 'ok' };
    } else if (result.status === 'cancelled') {
      status[format] = { state: 'idle' };
    } else {
      status[format] = { state: 'error', message: result.message };
    }
  }
</script>

<div class="export-panel scroll">
  {#if !doc}
    <p class="prose">Generate or load a system design before exporting.</p>
  {/if}

  <div class="export-grid">
    {#each FORMATS as info (info.format)}
      {@const s = status[info.format]}
      <div class="panel export-card">
        <div class="export-card-head">
          <h3 class="row-v export-title">{info.label}</h3>
          {#if s.state === 'ok'}
            <span class="pill is-ok">Exported</span>
          {:else if s.state === 'error'}
            <span class="pill is-danger">Failed</span>
          {:else if s.state === 'pending'}
            <span class="pill">Exporting…</span>
          {/if}
        </div>
        <p class="prose">{info.description}</p>
        <button
          type="button"
          class="btn btn-primary"
          disabled={!doc || s.state === 'pending'}
          onclick={() => handleExport(info.format)}
        >
          {info.format === 'terraform' ? 'Choose folder…' : 'Choose file…'}
        </button>
        {#if s.state === 'error' && s.message}
          <p class="issue-text">{s.message}</p>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .export-panel {
    display: flex;
    flex-direction: column;
    gap: var(--sp-5);
    padding: var(--sp-5);
    height: 100%;
  }

  .export-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--sp-4);
    align-items: start;
  }

  .export-card {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-5);
  }

  .export-card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
  }

  .export-title {
    text-align: left;
  }

  .issue-text {
    font-size: var(--fs-sm);
    color: var(--danger);
  }
</style>
