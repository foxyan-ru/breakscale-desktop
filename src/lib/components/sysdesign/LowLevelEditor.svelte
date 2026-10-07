<script lang="ts">
  /**
   * Edits `LowLevelDesign` (see `$lib/domain/system-design.ts` and
   * MIGRATION_PLAN.md §8): API contracts, entity models, sequence flows,
   * state machines, and the deployment configuration.
   *
   * Six repeatable-row families live here (apis/entities/sequences/
   * stateMachines/deployment-resources, plus each row's own sub-lists), so
   * the shared "bordered card with a Remove button" shape is factored into
   * the `card` snippet below rather than written out six times. Sub-list
   * rows (an API's errors, an entity's fields/relationships, a sequence's
   * steps, a state machine's states/transitions) stay plain inline rows —
   * app.css's boundary rule caps nesting at one bordered box, so a sub-list
   * row is deliberately NOT another `.panel` inside the card.
   *
   * `ValidationIssue` matching follows the same best-effort substring
   * approach as ArchitectureEditor.svelte (see that file's header comment):
   * matched at the row level by the row's own `id`/`nodeId`, with an
   * "Other validation issues" fallback list for anything that can't be
   * placed with confidence.
   */
  import type { Snippet } from 'svelte';
  import { sysdesignStore, newLocalId } from '$lib/state/sysdesign.svelte';
  import { topologyStore } from '$lib/state/topology.svelte';
  import type {
    ApiContract,
    EntityModel,
    SequenceFlow,
    StateMachine,
    ValidationIssue,
    VendorId,
  } from '$lib/domain';

  const doc = $derived(sysdesignStore.doc);
  const issues = $derived(sysdesignStore.validationIssues);

  const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

  const VENDOR_OPTIONS: { value: '' | VendorId; label: string }[] = [
    { value: '', label: 'None' },
    { value: 'generic', label: 'Generic' },
    { value: 'aws', label: 'AWS' },
    { value: 'gcp', label: 'GCP' },
    { value: 'azure', label: 'Azure' },
  ];

  function nodeLabel(nodeId: string): string {
    const n = topologyStore.topology.nodes.find((n) => n.id === nodeId);
    return n ? `${n.label} · ${n.kind}` : nodeId;
  }

  // ---- validation-issue matching (row-level; see ArchitectureEditor.svelte) --
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

    const apis = new Map<string, ValidationIssue[]>();
    for (const a of doc?.lowLevel.apis ?? []) apis.set(a.id, pick([a.id]));
    const entities = new Map<string, ValidationIssue[]>();
    for (const e of doc?.lowLevel.entities ?? []) entities.set(e.id, pick([e.id]));
    const sequences = new Map<string, ValidationIssue[]>();
    for (const s of doc?.lowLevel.sequences ?? []) sequences.set(s.id, pick([s.id]));
    const stateMachines = new Map<string, ValidationIssue[]>();
    for (const sm of doc?.lowLevel.stateMachines ?? []) stateMachines.set(sm.id, pick([sm.id]));

    const resources = new Map<string, ValidationIssue[]>();
    for (const r of doc?.lowLevel.deployment.resources ?? []) {
      resources.set(r.nodeId, pick([r.nodeId, 'deployment']));
    }
    const deployEnvironment = pick(['deployment', 'environment']);
    const deployRegion = pick(['deployment', 'region']);
    const deployVendor = pick(['deployment', 'vendor']);
    const deployNotes = pick(['deployment', 'notes']);

    const leftover = issues.filter((iss) => !consumed.has(iss));
    return {
      apis,
      entities,
      sequences,
      stateMachines,
      resources,
      deployEnvironment,
      deployRegion,
      deployVendor,
      deployNotes,
      leftover,
    };
  });

  // ---- apis -------------------------------------------------------------
  function addApi(): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.apis.push({
        id: newLocalId('api'),
        method: 'GET',
        path: '',
        description: '',
        requestSchema: '',
        responseSchema: '',
        errors: [],
      });
    });
  }
  function removeApi(id: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.apis = d.lowLevel.apis.filter((a) => a.id !== id);
    });
  }
  function addApiError(api: ApiContract): void {
    sysdesignStore.mutate(() => {
      api.errors.push({ statusCode: 400, code: '', description: '' });
    });
  }
  function removeApiError(api: ApiContract, index: number): void {
    sysdesignStore.mutate(() => {
      api.errors.splice(index, 1);
    });
  }

  // ---- entities -----------------------------------------------------------
  function addEntity(): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.entities.push({
        id: newLocalId('ent'),
        name: '',
        description: '',
        fields: [],
        relationships: [],
      });
    });
  }
  function removeEntity(id: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.entities = d.lowLevel.entities.filter((e) => e.id !== id);
    });
  }
  function addEntityField(entity: EntityModel): void {
    sysdesignStore.mutate(() => {
      entity.fields.push({ name: '', fieldType: '', required: false, description: '' });
    });
  }
  function removeEntityField(entity: EntityModel, index: number): void {
    sysdesignStore.mutate(() => {
      entity.fields.splice(index, 1);
    });
  }
  function addEntityRelationship(entity: EntityModel): void {
    sysdesignStore.mutate(() => {
      entity.relationships.push({ toEntity: '', kind: '', description: '' });
    });
  }
  function removeEntityRelationship(entity: EntityModel, index: number): void {
    sysdesignStore.mutate(() => {
      entity.relationships.splice(index, 1);
    });
  }

  // ---- sequences ----------------------------------------------------------
  function addSequence(): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.sequences.push({ id: newLocalId('seq'), name: '', steps: [] });
    });
  }
  function removeSequence(id: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.sequences = d.lowLevel.sequences.filter((s) => s.id !== id);
    });
  }
  function addSequenceStep(seq: SequenceFlow): void {
    sysdesignStore.mutate(() => {
      seq.steps.push({ from: '', to: '', action: '', note: '' });
    });
  }
  function removeSequenceStep(seq: SequenceFlow, index: number): void {
    sysdesignStore.mutate(() => {
      seq.steps.splice(index, 1);
    });
  }

  // ---- state machines -------------------------------------------------------
  function addStateMachine(): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.stateMachines.push({ id: newLocalId('sm'), name: '', states: [], transitions: [] });
    });
  }
  function removeStateMachine(id: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.stateMachines = d.lowLevel.stateMachines.filter((s) => s.id !== id);
    });
  }
  function addState(sm: StateMachine): void {
    sysdesignStore.mutate(() => {
      sm.states.push('');
    });
  }
  function removeState(sm: StateMachine, index: number): void {
    sysdesignStore.mutate(() => {
      sm.states.splice(index, 1);
    });
  }
  function addTransition(sm: StateMachine): void {
    sysdesignStore.mutate(() => {
      sm.transitions.push({ from: '', to: '', trigger: '', description: '' });
    });
  }
  function removeTransition(sm: StateMachine, index: number): void {
    sysdesignStore.mutate(() => {
      sm.transitions.splice(index, 1);
    });
  }

  // ---- deployment -----------------------------------------------------------
  function addResource(): void {
    sysdesignStore.mutate((d) => {
      const used = new Set(d.lowLevel.deployment.resources.map((r) => r.nodeId));
      const nextNode = topologyStore.topology.nodes.find((n) => !used.has(n.id));
      d.lowLevel.deployment.resources.push({
        nodeId: nextNode?.id ?? topologyStore.topology.nodes[0]?.id ?? '',
        sizeName: null,
        notes: '',
      });
    });
  }
  function removeResource(index: number): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.deployment.resources.splice(index, 1);
    });
  }
  function setVendor(value: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.deployment.vendor = value === '' ? null : (value as VendorId);
    });
  }
  function setResourceNode(index: number, nodeId: string): void {
    sysdesignStore.mutate((d) => {
      d.lowLevel.deployment.resources[index].nodeId = nodeId;
    });
  }
</script>

<!-- Shared "bordered card with a Remove button" shape -- see header comment. -->
{#snippet card(remove: () => void, content: Snippet)}
  <div class="panel ld-item">
    {@render content()}
    <div class="ld-item-foot">
      <button type="button" class="btn btn-sm btn-danger" onclick={remove}>Remove</button>
    </div>
  </div>
{/snippet}

{#if !doc}
  <div class="empty">
    <p class="row-k">No system design yet</p>
    <p class="prose">
      Generate a starting document from your canvas first, on the Architecture tab -- the sequence
      flows here are seeded from a walk of your current wiring, so there is real content to start
      from instead of a blank form.
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
  <div class="ld-editor scroll">
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

    <!-- ================= API contracts ================= -->
    <section>
      <div class="ld-row-head">
        <h3 class="label section-title">API contracts</h3>
        <button type="button" class="btn btn-sm" onclick={addApi}>+ Add API</button>
      </div>
      {#if doc.lowLevel.apis.length === 0}
        <p class="prose">No API contracts yet.</p>
      {/if}
      {#each doc.lowLevel.apis as api (api.id)}
        {#snippet apiContent()}
          <div class="ld-inline">
            <div class="field-stack ld-grow">
              <label class="label" for="api-desc-{api.id}">Description</label>
              <input
                id="api-desc-{api.id}"
                type="text"
                bind:value={api.description}
                oninput={() => sysdesignStore.scheduleValidate()}
                placeholder="What this endpoint does, e.g. Create order"
              />
            </div>
            <div class="field-stack">
              <label class="label" for="api-method-{api.id}">Method</label>
              <select
                id="api-method-{api.id}"
                value={api.method}
                onchange={(e) => {
                  api.method = (e.currentTarget as HTMLSelectElement).value;
                  sysdesignStore.scheduleValidate();
                }}
              >
                {#each HTTP_METHODS as m (m)}
                  <option value={m}>{m}</option>
                {/each}
              </select>
            </div>
          </div>
          <div class="field-stack">
            <label class="label" for="api-path-{api.id}">Path</label>
            <input
              id="api-path-{api.id}"
              type="text"
              bind:value={api.path}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="/v1/orders/:id"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="api-req-{api.id}">Request schema</label>
            <textarea
              id="api-req-{api.id}"
              bind:value={api.requestSchema}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="Body / query / headers shape"
            ></textarea>
          </div>
          <div class="field-stack">
            <label class="label" for="api-res-{api.id}">Response schema</label>
            <textarea
              id="api-res-{api.id}"
              bind:value={api.responseSchema}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="Success response shape"
            ></textarea>
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">Errors</span>
              <button type="button" class="btn btn-sm" onclick={() => addApiError(api)}>+ Add error</button>
            </div>
            {#each api.errors as err, i (i)}
              <div class="ld-subrow">
                <input
                  type="number"
                  aria-label="Status code"
                  bind:value={api.errors[i].statusCode}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="404"
                />
                <input
                  type="text"
                  aria-label="Error code"
                  bind:value={api.errors[i].code}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="Code, e.g. ORDER_NOT_FOUND"
                />
                <input
                  type="text"
                  aria-label="Error description"
                  bind:value={api.errors[i].description}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="When it happens"
                />
                <button
                  type="button"
                  class="btn btn-icon btn-sm ld-remove"
                  aria-label="Remove error"
                  onclick={() => removeApiError(api, i)}>×</button
                >
              </div>
            {/each}
          </div>
          {#each matches.apis.get(api.id) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        {/snippet}
        {@render card(() => removeApi(api.id), apiContent)}
      {/each}
    </section>

    <!-- ================= Entities ================= -->
    <section>
      <div class="ld-row-head">
        <h3 class="label section-title">Entity models</h3>
        <button type="button" class="btn btn-sm" onclick={addEntity}>+ Add entity</button>
      </div>
      {#if doc.lowLevel.entities.length === 0}
        <p class="prose">No entities yet.</p>
      {/if}
      {#each doc.lowLevel.entities as entity (entity.id)}
        {#snippet entityContent()}
          <div class="field-stack">
            <label class="label" for="ent-name-{entity.id}">Name</label>
            <input
              id="ent-name-{entity.id}"
              type="text"
              bind:value={entity.name}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. Order"
            />
          </div>
          <div class="field-stack">
            <label class="label" for="ent-desc-{entity.id}">Description</label>
            <input
              id="ent-desc-{entity.id}"
              type="text"
              bind:value={entity.description}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="What this entity represents"
            />
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">Fields</span>
              <button type="button" class="btn btn-sm" onclick={() => addEntityField(entity)}>
                + Add field
              </button>
            </div>
            {#each entity.fields as field, i (i)}
              <div class="ld-subrow ld-subrow-wide">
                <input
                  type="text"
                  aria-label="Field name"
                  bind:value={entity.fields[i].name}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="name"
                />
                <input
                  type="text"
                  aria-label="Field type"
                  bind:value={entity.fields[i].fieldType}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="type"
                />
                <label class="ld-checkbox">
                  <input
                    type="checkbox"
                    bind:checked={entity.fields[i].required}
                    onchange={() => sysdesignStore.scheduleValidate()}
                  />
                  <span class="label">Required</span>
                </label>
                <input
                  type="text"
                  aria-label="Field description"
                  bind:value={entity.fields[i].description}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="what it holds and why"
                />
                <button
                  type="button"
                  class="btn btn-icon btn-sm ld-remove"
                  aria-label="Remove field"
                  onclick={() => removeEntityField(entity, i)}>×</button
                >
              </div>
            {/each}
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">Relationships</span>
              <button type="button" class="btn btn-sm" onclick={() => addEntityRelationship(entity)}>
                + Add relationship
              </button>
            </div>
            {#each entity.relationships as rel, i (i)}
              <div class="ld-subrow">
                <input
                  type="text"
                  aria-label="Related entity"
                  bind:value={entity.relationships[i].toEntity}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="To entity"
                />
                <input
                  type="text"
                  aria-label="Relationship kind"
                  bind:value={entity.relationships[i].kind}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="e.g. one-to-many"
                />
                <input
                  type="text"
                  aria-label="Relationship description"
                  bind:value={entity.relationships[i].description}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="Description"
                />
                <button
                  type="button"
                  class="btn btn-icon btn-sm ld-remove"
                  aria-label="Remove relationship"
                  onclick={() => removeEntityRelationship(entity, i)}>×</button
                >
              </div>
            {/each}
          </div>
          {#each matches.entities.get(entity.id) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        {/snippet}
        {@render card(() => removeEntity(entity.id), entityContent)}
      {/each}
    </section>

    <!-- ================= Sequence flows ================= -->
    <section>
      <div class="ld-row-head">
        <h3 class="label section-title">Sequence flows</h3>
        <button type="button" class="btn btn-sm" onclick={addSequence}>+ Add flow</button>
      </div>
      <p class="prose">
        Auto-generated flows come from a walk of your canvas's routing graph starting at each
        client node -- read them top to bottom as one request's path.
      </p>
      {#if doc.lowLevel.sequences.length === 0}
        <p class="prose">No sequence flows yet.</p>
      {/if}
      {#each doc.lowLevel.sequences as seq (seq.id)}
        {#snippet sequenceContent()}
          <div class="field-stack">
            <label class="label" for="seq-name-{seq.id}">Name</label>
            <input
              id="seq-name-{seq.id}"
              type="text"
              bind:value={seq.name}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. Place order"
            />
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">Steps ({seq.steps.length})</span>
              <button type="button" class="btn btn-sm" onclick={() => addSequenceStep(seq)}>
                + Add step
              </button>
            </div>
            <ol class="ld-steps">
              {#each seq.steps as step, i (i)}
                <li class="ld-subrow ld-subrow-wide">
                  <span class="ld-step-index num mono">{i + 1}</span>
                  <input
                    type="text"
                    aria-label="From"
                    bind:value={seq.steps[i].from}
                    oninput={() => sysdesignStore.scheduleValidate()}
                    placeholder="From"
                  />
                  <input
                    type="text"
                    aria-label="To"
                    bind:value={seq.steps[i].to}
                    oninput={() => sysdesignStore.scheduleValidate()}
                    placeholder="To"
                  />
                  <input
                    type="text"
                    aria-label="Action"
                    bind:value={seq.steps[i].action}
                    oninput={() => sysdesignStore.scheduleValidate()}
                    placeholder="Action"
                  />
                  <input
                    type="text"
                    aria-label="Note"
                    bind:value={seq.steps[i].note}
                    oninput={() => sysdesignStore.scheduleValidate()}
                    placeholder="Note (optional)"
                  />
                  <button
                    type="button"
                    class="btn btn-icon btn-sm ld-remove"
                    aria-label="Remove step"
                    onclick={() => removeSequenceStep(seq, i)}>×</button
                  >
                </li>
              {/each}
            </ol>
          </div>
          {#each matches.sequences.get(seq.id) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        {/snippet}
        {@render card(() => removeSequence(seq.id), sequenceContent)}
      {/each}
    </section>

    <!-- ================= State machines ================= -->
    <section>
      <div class="ld-row-head">
        <h3 class="label section-title">State machines</h3>
        <button type="button" class="btn btn-sm" onclick={addStateMachine}>+ Add state machine</button>
      </div>
      {#if doc.lowLevel.stateMachines.length === 0}
        <p class="prose">No state machines yet.</p>
      {/if}
      {#each doc.lowLevel.stateMachines as sm (sm.id)}
        {#snippet stateMachineContent()}
          <div class="field-stack">
            <label class="label" for="sm-name-{sm.id}">Name</label>
            <input
              id="sm-name-{sm.id}"
              type="text"
              bind:value={sm.name}
              oninput={() => sysdesignStore.scheduleValidate()}
              placeholder="e.g. Order lifecycle"
            />
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">States</span>
              <button type="button" class="btn btn-sm" onclick={() => addState(sm)}>+ Add state</button>
            </div>
            <div class="ld-chip-row">
              {#each sm.states as _state, i (i)}
                <div class="ld-subrow">
                  <input
                    type="text"
                    aria-label="State name"
                    bind:value={sm.states[i]}
                    oninput={() => sysdesignStore.scheduleValidate()}
                    placeholder="State"
                  />
                  <button
                    type="button"
                    class="btn btn-icon btn-sm ld-remove"
                    aria-label="Remove state"
                    onclick={() => removeState(sm, i)}>×</button
                  >
                </div>
              {/each}
            </div>
          </div>
          <div class="ld-sub">
            <div class="ld-row-head">
              <span class="label">Transitions</span>
              <button type="button" class="btn btn-sm" onclick={() => addTransition(sm)}>
                + Add transition
              </button>
            </div>
            {#each sm.transitions as t, i (i)}
              <div class="ld-subrow ld-subrow-wide">
                <input
                  type="text"
                  aria-label="From state"
                  bind:value={sm.transitions[i].from}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="From"
                />
                <input
                  type="text"
                  aria-label="To state"
                  bind:value={sm.transitions[i].to}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="To"
                />
                <input
                  type="text"
                  aria-label="Trigger"
                  bind:value={sm.transitions[i].trigger}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="What causes it, e.g. payment confirmed"
                />
                <input
                  type="text"
                  aria-label="Description"
                  bind:value={sm.transitions[i].description}
                  oninput={() => sysdesignStore.scheduleValidate()}
                  placeholder="Extra detail (optional)"
                />
                <button
                  type="button"
                  class="btn btn-icon btn-sm ld-remove"
                  aria-label="Remove transition"
                  onclick={() => removeTransition(sm, i)}>×</button
                >
              </div>
            {/each}
          </div>
          {#each matches.stateMachines.get(sm.id) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        {/snippet}
        {@render card(() => removeStateMachine(sm.id), stateMachineContent)}
      {/each}
    </section>

    <!-- ================= Deployment ================= -->
    <section>
      <h3 class="label section-title">Deployment</h3>
      <div class="ld-inline">
        <div class="field-stack ld-grow">
          <label class="label" for="dep-env">Environment</label>
          <input
            id="dep-env"
            type="text"
            bind:value={doc.lowLevel.deployment.environment}
            oninput={() => sysdesignStore.scheduleValidate()}
            placeholder="e.g. production"
          />
          {#each matches.deployEnvironment as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        </div>
        <div class="field-stack">
          <label class="label" for="dep-vendor">Vendor</label>
          <select
            id="dep-vendor"
            value={doc.lowLevel.deployment.vendor ?? ''}
            onchange={(e) => setVendor((e.currentTarget as HTMLSelectElement).value)}
          >
            {#each VENDOR_OPTIONS as opt (opt.value)}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
          {#each matches.deployVendor as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        </div>
        <div class="field-stack">
          <label class="label" for="dep-region">Region</label>
          <input
            id="dep-region"
            type="text"
            value={doc.lowLevel.deployment.region ?? ''}
            oninput={(e) => {
              doc.lowLevel.deployment.region = (e.currentTarget as HTMLInputElement).value || null;
              sysdesignStore.scheduleValidate();
            }}
            placeholder="e.g. us-east-1"
          />
          {#each matches.deployRegion as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        </div>
      </div>
      <div class="field-stack">
        <label class="label" for="dep-notes">Notes</label>
        <textarea
          id="dep-notes"
          bind:value={doc.lowLevel.deployment.notes}
          oninput={() => sysdesignStore.scheduleValidate()}
          placeholder="Anything about the deployment that doesn't fit a resource row"
        ></textarea>
        {#each matches.deployNotes as issue (issue.message)}
          <p class="issue-text">{issue.message}</p>
        {/each}
      </div>

      <div class="ld-row-head">
        <span class="label">Resources</span>
        <button
          type="button"
          class="btn btn-sm"
          onclick={addResource}
          disabled={topologyStore.topology.nodes.length === 0}
        >
          + Add resource
        </button>
      </div>
      {#if doc.lowLevel.deployment.resources.length === 0}
        <p class="prose">No deployment resources yet.</p>
      {/if}
      {#each doc.lowLevel.deployment.resources as resource, i (i)}
        {#snippet resourceContent()}
          <div class="ld-inline">
            <div class="field-stack ld-grow">
              <label class="label" for="res-node-{i}">Node</label>
              <select
                id="res-node-{i}"
                value={resource.nodeId}
                onchange={(e) => setResourceNode(i, (e.currentTarget as HTMLSelectElement).value)}
              >
                {#each topologyStore.topology.nodes as n (n.id)}
                  <option value={n.id}>{n.label} · {n.kind}</option>
                {/each}
              </select>
            </div>
            <div class="field-stack">
              <label class="label" for="res-size-{i}">Size</label>
              <input
                id="res-size-{i}"
                type="text"
                value={resource.sizeName ?? ''}
                oninput={(e) => {
                  resource.sizeName = (e.currentTarget as HTMLInputElement).value || null;
                  sysdesignStore.scheduleValidate();
                }}
                placeholder="optional"
              />
            </div>
          </div>
          <div class="field-stack">
            <label class="label" for="res-notes-{i}">Notes</label>
            <textarea
              id="res-notes-{i}"
              bind:value={resource.notes}
              oninput={() => sysdesignStore.scheduleValidate()}
            ></textarea>
          </div>
          <p class="prose ld-resource-label">{nodeLabel(resource.nodeId)}</p>
          {#each matches.resources.get(resource.nodeId) ?? [] as issue (issue.message)}
            <p class="issue-text">{issue.message}</p>
          {/each}
        {/snippet}
        {@render card(() => removeResource(i), resourceContent)}
      {/each}
    </section>
  </div>
{/if}

<style>
  .ld-editor {
    display: flex;
    flex-direction: column;
    gap: var(--sp-7);
    padding: var(--sp-5);
    height: 100%;
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }

  .section-title {
    margin-bottom: var(--sp-1);
  }

  .ld-row-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-3);
  }

  .ld-item {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    padding: var(--sp-4);
  }

  .ld-item-foot {
    display: flex;
    justify-content: flex-end;
  }

  .ld-inline {
    display: flex;
    gap: var(--sp-3);
    flex-wrap: wrap;
  }

  .ld-grow {
    flex: 1 1 200px;
  }

  .ld-sub {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding-top: var(--sp-2);
    border-top: var(--bw) solid var(--border);
  }

  .ld-subrow {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }

  .ld-subrow-wide input {
    flex: 1 1 0;
    min-width: 0;
  }

  .ld-subrow input {
    min-width: 0;
  }

  .ld-checkbox {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    flex: none;
    white-space: nowrap;
  }

  .ld-remove {
    flex: none;
    color: var(--danger);
  }

  .ld-steps {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    list-style: none;
  }

  .ld-step-index {
    flex: none;
    width: 20px;
    color: var(--text-faint);
    text-align: right;
  }

  .ld-chip-row {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }

  .ld-resource-label {
    margin: 0;
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
