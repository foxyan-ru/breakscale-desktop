<script lang="ts">
  /**
   * What this component is called at the chosen vendor, and how big it is.
   *
   * Ported from `src/components/VendorPanel.tsx`. This is the PER-NODE size
   * selector only -- the vendor-mode picker (generic/AWS/GCP/Azure) lives in
   * the web app's `Settings.tsx`, out of scope for this pass, and reads/writes
   * `settingsStore.vendor` (already ported, `state/settings.svelte.ts`).
   * `VendorPanel` here only ever READS that choice.
   *
   * Meant to be embedded in the (not-yet-built) node inspector, one
   * selected-node's kind/config at a time -- hence prop-driven rather than
   * self-contained like `Cost.svelte`/`Trace.svelte`, matching the original's
   * own `VendorPanelProps` shape exactly so the inspector agent's wiring
   * needs no translation.
   *
   * Only rendered when a vendor is chosen AND that vendor has a mapping for
   * this kind. On generic it is absent entirely rather than empty, because a
   * reader learning what a load balancer IS should not be asked to pick an
   * instance class for it.
   *
   * THE HONESTY RULE. Every spec shown here is published and linked. The
   * capacity a size implies is NOT published by anyone, so it is labelled as
   * ours wherever it appears (`derivedNote` below). Without that labelling
   * this panel would be the app quietly inventing numbers, the exact thing
   * it exists to argue against.
   */
  import { settingsStore } from '$lib/state/settings.svelte';
  import { getSize, setSize } from '$lib/state/vendor-sizes.svelte';
  import { peekVendor, vendorsGet } from '$lib/api/vendors';
  import type { NodeConfig, NodeKind, Vendor, VendorSize } from '$lib/domain';

  interface Props {
    nodeId: string;
    kind: NodeKind;
    config: NodeConfig;
    onChange: (patch: Partial<NodeConfig>) => void;
  }

  let { nodeId, kind, config, onChange }: Props = $props();

  let vendor: Vendor | null = $state(peekVendor(settingsStore.vendor));

  $effect(() => {
    const id = settingsStore.vendor;
    if (id === 'generic') {
      vendor = null;
      return;
    }
    const cached = peekVendor(id);
    if (cached) {
      vendor = cached;
      return;
    }
    let live = true;
    void vendorsGet(id).then((v) => {
      if (live) vendor = v;
    });
    return () => {
      live = false;
    };
  });

  const mapping = $derived(vendor?.kinds[kind] ?? null);
  const chosen = $derived(vendor ? getSize(vendor.id, nodeId) : null);
  const sizes = $derived(mapping?.sizes ?? []);

  /* --------------------------------------------------------------------- *
   * Size -> capacity derivation, inlined from `src/content/vendors/derive.ts`.
   *
   * NOT a new module: MIGRATION_PLAN.md §4 plans this as Rust
   * (`vendors::derive`, alongside `vendors::cost`, being ported in parallel),
   * and the frontend has no `vendors_apply_size`-shaped command for it yet
   * (same gap as `vendors_get` -- see `$lib/api/vendors.ts`'s own
   * NOTE(integration)). It is a few lines of pure, stateless arithmetic with
   * no simulation state behind it, so it is kept here rather than as a
   * seventh file this pass was not scoped to add. Delete this block (and
   * call a real command instead) once one exists.
   * --------------------------------------------------------------------- */

  /** Concurrent requests one vCPU is assumed to handle at a time. Not a
   * measurement -- see the source file's own commentary on why this is a
   * deliberately simple, honestly-labelled rule of thumb. */
  const SLOTS_PER_VCPU = 1;

  const SIZED_KINDS: ReadonlySet<NodeKind> = new Set([
    'service',
    'db',
    'cache',
    'worker',
    'replica',
    'shard',
    'searchindex',
    'timeseriesdb',
    'graphdb',
    'vectordb',
    'transcoder',
  ]);

  function isSizedKind(k: NodeKind): boolean {
    return SIZED_KINDS.has(k);
  }

  function deriveCapacity(k: NodeKind, size: VendorSize): number | null {
    if (!isSizedKind(k)) return null;
    const vcpu = size.vcpu;
    if (typeof vcpu !== 'number' || !Number.isFinite(vcpu) || vcpu <= 0) return null;
    const fromVcpu = Math.max(1, Math.round(vcpu * SLOTS_PER_VCPU));
    return typeof size.maxConnections === 'number' && size.maxConnections > 0
      ? Math.min(fromVcpu, size.maxConnections)
      : fromVcpu;
  }

  const DERIVED_NOTE =
    'Capacity is our estimate from the vCPU count, not a figure the vendor publishes. Set it yourself to model your own service.';

  const sizeable = $derived(isSizedKind(kind) && sizes.length > 0);

  function pick(name: string): void {
    const picked = sizes.find((s) => s.name === name);
    if (!picked || !vendor) return;
    // Remembered so the design can be priced and so the choice survives a
    // reload. The engine never sees this: it only ever learns the capacity
    // written below.
    setSize(vendor.id, nodeId, picked.name);
    const capacity = deriveCapacity(kind, picked);
    if (capacity !== null) onChange({ capacity });
  }
</script>

{#if vendor && mapping}
  <section class="vp" aria-label={`${vendor.label} details for ${mapping.product}`}>
    <p class="label vp-eyebrow">On {vendor.label}</p>
    <p class="vp-product">{mapping.product}</p>
    {#if mapping.note}
      <p class="vp-note">{mapping.note}</p>
    {/if}

    {#if sizes.length > 0}
      <label class="vp-field">
        <span class="vp-label">Size</span>
        <!-- Uncontrolled in spirit: picking a size is an ACTION, not a
             stored property of the node -- it writes a capacity and the
             component then belongs to the reader, who may tune it further.
             The select's value still mirrors the remembered choice
             (`chosen`) so switching between nodes shows the right size. -->
        <select
          class="vp-select"
          value={chosen ?? ''}
          onchange={(e) => pick(e.currentTarget.value)}
        >
          <option value="">Choose a size</option>
          {#each sizes as s (s.name)}
            <option value={s.name}>
              {s.name}{s.vcpu ? ` · ${s.vcpu} vCPU` : ''} · {s.memory} {s.memoryUnit}{s.pricePerHour
                ? ` · $${s.pricePerHour}/h`
                : ''}
            </option>
          {/each}
        </select>
      </label>

      {#if sizeable}
        <p class="vp-derived">{DERIVED_NOTE}</p>
      {:else}
        <!-- A size is shown for its published specs and price, but nothing
             about it changes the simulation, and saying so beats letting a
             reader assume the picker did something. -->
        <p class="vp-derived">
          These sizes are for their published specs and prices. This kind is not limited by an
          instance size, so picking one changes nothing in the simulation.
        </p>
      {/if}

      <p class="vp-source">
        Specs and prices published by {vendor.label}{vendor.region ? ` for ${vendor.region}` : ''}.
        <a href={sizes[0]!.source} target="_blank" rel="noreferrer noopener">Source</a>{sizes[0]!
          .pricedOn
          ? `, read ${sizes[0]!.pricedOn}`
          : ''}.
      </p>
    {/if}
  </section>
{/if}

<style>
  /* Ported from src/components/VendorPanel.css.

     Quiet by design. The knobs above it are what a reader changes to learn
     something; this is context, and it should not compete with them. */

  .vp {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-3) 0 0;
    border-top: var(--bw) solid var(--border);
  }

  .vp-eyebrow {
    margin: 0;
    color: var(--text-faint);
  }

  .vp-product {
    margin: 0;
    font-size: var(--fs-base);
    color: var(--text);
  }

  .vp-note,
  .vp-derived,
  .vp-source {
    margin: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
  }

  /* The caveat that the capacity is ours, not the vendor's. Marked with the
     warning ramp rather than left as body text: it is the one line here a
     reader must not skim past, because everything else on this panel IS
     published and this is the part that is not. */
  .vp-derived {
    padding: var(--sp-2);
    margin-top: var(--sp-1);
    border-left: 2px solid var(--warn-mark);
    border-radius: 0 var(--r-sm) var(--r-sm) 0;
    background: var(--warn-soft);
    color: var(--text);
  }

  .vp-field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-top: var(--sp-2);
  }

  .vp-label {
    font-size: var(--fs-sm);
    color: var(--text-dim);
  }

  .vp-select {
    width: 100%;
    height: 32px;
    padding: 0 var(--sp-2);
    border: var(--bw) solid var(--border-strong);
    border-radius: var(--r-btn);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
  }

  .vp-select:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  .vp-source a {
    color: var(--accent);
  }
</style>
