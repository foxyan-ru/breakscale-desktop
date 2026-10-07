<script lang="ts">
  /**
   * Edits `HighLevelArchitecture` (see `$lib/domain/system-design.ts` and
   * MIGRATION_PLAN.md §8): the document summary, one `ComponentNote` per
   * canvas node, one `DataFlowNote` per canvas edge, a free-form list of
   * `ExternalDependency` for things the simulator has no node for, and the
   * `QualityAttributes` narrative block.
   *
   * `ValidationIssue`s are matched to a specific control by a best-effort
   * substring test on `issue.field` (see `matches` below) rather than an
   * assumed exact path grammar, since this file was built against the TS
   * contract alone and the Rust validator's exact `field` string format
   * (`validate.rs`) was not read. Anything that cannot be matched with
   * reasonable confidence falls back to the small list at the top, per this
   * task's own instruction to prefer inline but not force a bad match.
   */
  import { sysdesignStore, newLocalId } from '$lib/state/sysdesign.svelte';
  import { topologyStore } from '$lib/state/topology.svelte';
  import type { ValidationIssue } from '$lib/domain';

  const doc = $derived(sysdesignStore.doc);
  const issues = $derived(sysdesignStore.validationIssues);

  let justSaved = $state(false);

  /** Keep one `ComponentNote`/`DataFlowNote` per node/edge CURRENTLY on the canvas, so a node added after `deriveFromCurrentTopology()` still gets a row here instead of being silently uneditable. */
  $effect(() => {
    const d = sysdesignStore.doc;
    if (!d) return;
    let changed = false;
    for (const node of topologyStore.topology.nodes) {
      if (!d.architecture.components.some((c) => c.nodeId === node.id)) {
        d.architecture.components.push({ nodeId: node.id, technology: '', rationale: '', owner: null });
        changed = true;
      }
    }
    for (const edge of topologyStore.topology.edges) {
      if (!d.architecture.dataFlows.some((f) => f.edgeId === edge.id)) {
        d.architecture.dataFlows.push({ edgeId: edge.id, protocol: '', dataClassification: '', description: '' });
        changed = true;
      }
    }
    if (changed) sysdesignStore.scheduleValidate();
  });

  function nodeLabel(nodeId: string): string {
    const n = topologyStore.topology.nodes.find((n) => n.id === nodeId);
    return n ? `${n.label} · ${n.kind}` : nodeId;
  }

  function edgeLabel(edgeId: string): string {
    const e = topologyStore.topology.edges.find((e) => e.id === edgeId);
    if (!e) return edgeId;
    const from = topologyStore.topology.nodes.find((n) => n.id === e.from);
    const to = topologyStore.topology.nodes.find((n) => n.id === e.to);
    return `${from?.label ?? e.from} → ${to?.label ?? e.to}`;
  }

  // ---- validation-issue matching -------------------------------------------
  const matches = $derived.by(() => {
    const consumed = new Set<ValidationIssue>();
    function pick(all: string[], exclude: string[] = []): ValidationIssue[] {
      const found = issues.filter((iss) => {
        if (consumed.has(iss)) return false;
        const f = iss.field.toLowerCase();
        if (!all.every((t) => f.includes(t.toLowerCase()))) return false;
        if (exclude.some((t) => f.includes(t.toLowerCase()))) return false;
        return true;
      });
      for (const f of found) consumed.add(f);
      return found;
    }

    const summary = pick(['summary']);
    const scalability = pick(['scalability']);
    const reliability = pick(['reliability']);
    const security = pick(['security']);
    const observability = pick(['observability']);
    const cost = pick(['cost']);

    const components = new Map<string, ValidationIssue[]>();
    for (const c of doc?.architecture.components ?? []) {
      // Exclude deployment/resource so a DeploymentResource keyed to the same
      // nodeId (edited in LowLevelEditor) never gets attributed here.
      components.set(c.nodeId, pick([c.nodeId], ['deployment', 'resource']));
    }
    const dataFlows = new Map<string, ValidationIssue[]>();
    for (const f of doc?.architecture.dataFlows ?? []) {
      dataFlows.set(f.edgeId, pick([f.edgeId]));
    }
    const externalDeps = new Map<string, ValidationIssue[]>();
    for (const dep of doc?.architecture.externalDependencies ?? []) {
      externalDeps.set(dep.id, pick([dep.id]));
    }

    const leftover = issues.filter((iss) => !consumed.has(iss));
    return {
      summary,
      quality: { scalability, reliability, security, observability, cost },
      components,
      dataFlows,
      externalDeps,
      leftover,
    };
  });

  function addExternalDependency(): void {
    sysdesignStore.mutate((d) => {
      d.architecture.externalDependencies.push({
        id: newLocalId('ext'),
        name: '',
        description: '',
        owner: null,
      });
    });
  }

  function removeExternalDependency(id: string): void {
    sysdesignStore.mutate((d) => {
      d.architecture.externalDependencies = d.architecture.externalDependencies.filter(
        (x) => x.id !== id,
      );
    });
  }

  async function handleSave(): Promise<void> {
    await sysdesignStore.save();
    justSaved = true;
    setTimeout(() => {
      justSaved = false;
    }, 2000);
  }
</script>

{#if !doc}
  <div class="empty">
    <p class="row-k">No system design yet</p>
    <p class="prose">
      Generate a starting document from the components and connections already on your canvas.
      Every field it produces is editable afterward.
    </p>
    <button
      type="button"
      class="btn btn-primary"
      onclick={() => sysdesignStore.deriveFromCurrentTopology()}
      disabled={sysdesignStore.busy}
    >
      {sysdesignStore.busy ? 'Generating…' : 'Generate from canvas'}
    </button>
  </div>
{:else}
  <div class="arch-editor">
    <div class="arch-toolbar">
      <div class="field-stack arch-name">
        <label class="label" for="sd-name">Design name</label>
        <input
          id="sd-name"
          type="text"
          bind:value={doc.name}
          oninput={() => sysdesignStore.scheduleValidate()}
        />
      </div>
      <div class="arch-toolbar-actions">
        <button type="button" class="btn btn-sm" onclick={() => sysdesignStore.validateNow()}>
          Check now
        </button>
        <button
          type="button"
          class="btn btn-sm btn-primary"
          onclick={handleSave}
          disabled={sysdesignStore.busy}
        >
          {justSaved ? 'Saved' : 'Save'}
        </button>
      </div>
    </div>

    {#if matches.leftover.length > 0}
      <ul class="issue-list panel">
        {#each matches.leftover as issue (issue.field + issue.message)}
          <li class="issue-row">
            <span class="label">{issue.field}</span>
            <span class="prose">{issue.message}</span>
          </li>
        {/each}
      </ul>
    {/if}

    <section class="field-stack">
      <label class="label" for="sd-summary">Summary</label>
      <textarea
        id="sd-summary"
        bind:value={doc.architecture.summary}
        oninput={() => sysdesignStore.scheduleValidate()}
        placeholder="What is this system, in a few sentences?"
      ></textarea>
      {#each matches.summary as issue (issue.message)}
        <p class="issue-text">{issue.message}</p>
      {/each}
    </section>

    <section>
      <h3 class="label section-title">Components</h3>
      {#if doc.architecture.components.length === 0}
        <p class="prose">No components yet — add nodes to the canvas.</p>
      {/if}
      {#each doc.architecture.components as note (note.nodeId)}
        <div class="panel arch-row">
          <div class="arch-row-head">
            <span class="pill">{nodeLabel(note.nodeId)}</span>
          </div>
          <div class="field-stack">
            <label class="label" for="tech-{note.nodeId}">Technology</label>
            <input
              id="tech-{note.nodeId}"
              type="text"
              bind:value={note.technology}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. Postgres 16, Go service, Redis"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="rat-{note.nodeId}">Rationale</label>
            <textarea
              id="rat-{note.nodeId}"
              bind:value={note.rationale}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="Why this component, why here"
            ></textarea>
          </div>
          <div class="field-stack">
            <label class="label" for="own-{note.nodeId}">Owner</label>
            <input
              id="own-{note.nodeId}"
              type="text"
              value={note.owner ?? ''}
              oninput={(e) => {
                note.owner = (e.currentTarget as HTMLInputElement).value || null;
                sysdesignStore.scheduleValidate();
              }}
              placeholder="Team or person (optional)"
            />
          </div>
          {#each matches.components.get(note.nodeId) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        </div>
      {/each}
    </section>

    <section>
      <div class="arch-row-head">
        <h3 class="label section-title">External dependencies</h3>
        <button type="button" class="btn btn-sm" onclick={addExternalDependency}>
          + Add dependency
        </button>
      </div>
      {#if doc.architecture.externalDependencies.length === 0}
        <p class="prose">Third-party APIs or SaaS the simulator has no node for.</p>
      {/if}
      {#each doc.architecture.externalDependencies as dep (dep.id)}
        <div class="panel arch-row">
          <div class="field-stack">
            <label class="label" for="dep-name-{dep.id}">Name</label>
            <input
              id="dep-name-{dep.id}"
              type="text"
              bind:value={dep.name}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. Stripe, Twilio, SendGrid"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="dep-desc-{dep.id}">Description</label>
            <textarea
              id="dep-desc-{dep.id}"
              bind:value={dep.description}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="What it does for this system"
            ></textarea>
          </div>
          <div class="field-stack">
            <label class="label" for="dep-owner-{dep.id}">Owner</label>
            <input
              id="dep-owner-{dep.id}"
              type="text"
              value={dep.owner ?? ''}
              oninput={(e) => {
                dep.owner = (e.currentTarget as HTMLInputElement).value || null;
                sysdesignStore.scheduleValidate();
              }}
              placeholder="Who owns this relationship, if tracked"
            />
          </div>
          {#each matches.externalDeps.get(dep.id) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
          <button
            type="button"
            class="btn btn-sm btn-danger"
            onclick={() => removeExternalDependency(dep.id)}
          >
            Remove
          </button>
        </div>
      {/each}
    </section>

    <section>
      <h3 class="label section-title">Data flows</h3>
      {#if doc.architecture.dataFlows.length === 0}
        <p class="prose">No connections yet — wire nodes together on the canvas.</p>
      {/if}
      {#each doc.architecture.dataFlows as flow (flow.edgeId)}
        <div class="panel arch-row">
          <div class="arch-row-head">
            <span class="pill">{edgeLabel(flow.edgeId)}</span>
          </div>
          <div class="field-stack">
            <label class="label" for="proto-{flow.edgeId}">Protocol</label>
            <input
              id="proto-{flow.edgeId}"
              type="text"
              bind:value={flow.protocol}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. HTTPS/JSON, gRPC, AMQP"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="class-{flow.edgeId}">Data classification</label>
            <input
              id="class-{flow.edgeId}"
              type="text"
              bind:value={flow.dataClassification}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. PII, internal, public"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="desc-{flow.edgeId}">Description</label>
            <textarea
              id="desc-{flow.edgeId}"
              bind:value={flow.description}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="What crosses this connection, and why"
            ></textarea>
          </div>
          {#each matches.dataFlows.get(flow.edgeId) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        </div>
      {/each}
    </section>

    <section>
      <h3 class="label section-title">Quality attributes</h3>

      <div class="field-stack">
        <label class="label" for="qa-scalability">Scalability</label>
        <textarea
          id="qa-scalability"
          bind:value={doc.architecture.qualityAttributes.scalability}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="How this system scales, and where it stops"
        ></textarea>
        {#each matches.quality.scalability as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>

      <div class="field-stack">
        <label class="label" for="qa-reliability">Reliability</label>
        <textarea
          id="qa-reliability"
          bind:value={doc.architecture.qualityAttributes.reliability}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="Failure modes, redundancy, recovery"
        ></textarea>
        {#each matches.quality.reliability as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>

      <div class="field-stack">
        <label class="label" for="qa-security">Security</label>
        <textarea
          id="qa-security"
          bind:value={doc.architecture.qualityAttributes.security}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="AuthN/authZ, data protection, threat model notes"
        ></textarea>
        {#each matches.quality.security as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>

      <div class="field-stack">
        <label class="label" for="qa-observability">Observability</label>
        <textarea
          id="qa-observability"
          bind:value={doc.architecture.qualityAttributes.observability}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="Metrics, logs, traces, alerting"
        ></textarea>
        {#each matches.quality.observability as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>

      <div class="field-stack">
        <label class="label" for="qa-cost">Cost</label>
        <textarea
          id="qa-cost"
          bind:value={doc.architecture.qualityAttributes.cost}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="Narrative context for the estimate in the cost panel"
        ></textarea>
        {#each matches.quality.cost as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>
    </section>
  </div>
{/if}

<style>
  /* Plain auto-height block: `.sd-body` in +page.svelte is the ONE scroll
     container. A nested `.scroll` root that never overflows would swallow
     the wheel (overscroll-behavior: contain) instead of letting it reach
     `.sd-body`. */
  .arch-editor {
    display: flex;
    flex-direction: column;
    gap: var(--sp-7);
    padding: var(--sp-5);
  }

  .arch-toolbar {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .arch-name {
    flex: 1 1 auto;
    max-width: 420px;
  }

  .arch-toolbar-actions {
    display: flex;
    gap: var(--sp-2);
    flex: none;
  }

  .section-title {
    margin-bottom: var(--sp-3);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .arch-row-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .arch-row {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-4);
  }

  .issue-list {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    border: var(--bw) solid var(--danger);
    list-style: none;
  }

  .issue-row {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }

  .issue-text {
    font-size: var(--fs-sm);
    color: var(--danger);
  }
</style>
