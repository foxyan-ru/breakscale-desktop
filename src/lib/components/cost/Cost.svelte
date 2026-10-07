<script lang="ts">
  /**
   * What this design would cost to run, per month.
   *
   * Ported from `src/components/Cost.tsx`. Self-contained: reads the
   * topology from `topologyStore`, the chosen vendor from `settingsStore`,
   * and the per-node sizes from `vendorSizesStore`
   * (`$lib/state/vendor-sizes.svelte`) itself, so it can be mounted with no
   * props, matching `Trace.svelte`/`Metrics.svelte` in this port.
   *
   * WHY IT EXISTS. "Add a read replica" and "buy a bigger box" both drain a
   * queue, and which one is right almost always turns on money. A simulator
   * that only reports latency can argue one half of a real design review.
   *
   * Only shown once a vendor is chosen: on generic there are no prices, and
   * an empty panel would teach nothing. Always labelled as compute at
   * on-demand rates -- the difference between an estimate and a bill is
   * exactly the kind of gap this project refuses to paper over (`COST_NOTE`
   * below).
   */
  import { simulationStore } from '$lib/state/simulation.svelte';
  import { topologyStore } from '$lib/state/topology.svelte';
  import { settingsStore } from '$lib/state/settings.svelte';
  import { getSize } from '$lib/state/vendor-sizes.svelte';
  import { peekVendor, vendorsGet } from '$lib/api/vendors';
  import { costDesign, formatMoney, COST_NOTE } from './cost';
  import type { Vendor } from '$lib/domain';

  const loading = $derived(simulationStore.snapshot === null);

  /**
   * The vendor currently chosen, once its data has arrived. Null while
   * generic is selected. Mirrors `useVendor()` in
   * `src/content/vendors/useVendor.ts`: synchronous from the cache when
   * available (so switching back to a vendor already fetched this session
   * does not flash the generic names), fetched via `vendorsGet` otherwise.
   */
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

  const cost = $derived.by(() =>
    costDesign(topologyStore.topology.nodes, vendor, (node) => {
      if (!vendor) return null;
      const name = getSize(vendor.id, node.id);
      if (!name) return null;
      return vendor.kinds[node.kind]?.sizes?.find((s) => s.name === name) ?? null;
    }),
  );

  const priced = $derived(cost.nodes.length > 0);
</script>

{#if loading}
  <section class="ct" aria-label="Estimated cost">
    <header class="mx-head">
      <span class="label mx-eyebrow">Monthly cost</span>
    </header>
    <p class="ct-empty">Waiting for the simulation to start.</p>
  </section>
{:else if vendor}
  <section
    class="ct"
    aria-label={priced
      ? `Estimated cost: ${formatMoney(cost.perMonth)} per month`
      : 'Estimated cost: no components priced yet'}
  >
    <header class="mx-head">
      <span class="label mx-eyebrow">Monthly cost</span>
      {#if priced}
        <span class="mx-head-right">
          <span class="mx-readout-wrap">
            <span class="num num-lg mx-readout">{formatMoney(cost.perMonth)}</span>
            <span class="unit mx-unit">/mo</span>
          </span>
        </span>
      {/if}
    </header>

    {#if !priced}
      <p class="ct-empty">
        Pick an instance size for a component in the inspector and its cost will appear here.
      </p>
    {:else}
      <ol class="ct-rows">
        {#each cost.nodes as n (n.nodeId)}
          <li class="ct-row">
            <span class="ct-name" title={n.label}>{n.label}</span>
            <span class="ct-size">{n.sizeName}{n.fleet > 1 ? ` x${n.fleet}` : ''}</span>
            <span class="num ct-money">{formatMoney(n.perMonth)}</span>
          </li>
        {/each}
      </ol>

      {#if cost.unpriced.length > 0}
        <!-- Said out loud. A total that quietly skips three components looks
             complete and is wrong by an unknown amount, which is worse than
             a total that admits what it left out. -->
        <p class="ct-unpriced">Not counted, no size chosen: {cost.unpriced.join(', ')}.</p>
      {/if}

      <p class="ct-note">{COST_NOTE}</p>
    {/if}
  </section>
{/if}

<style>
  /* Ported from src/components/Cost.css.

     Shares the charts strip's header shapes (.mx-head and friends, defined
     globally in app.css) so it reads as another panel in that row rather
     than as a visitor. */

  .ct {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .ct-empty,
  .ct-note,
  .ct-unpriced {
    margin: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
  }

  .ct-note {
    margin-top: var(--sp-2);
  }

  /* The gap in the total, marked rather than left as body text: it is the
     one line here that changes how the number above should be read. */
  .ct-unpriced {
    margin-top: var(--sp-2);
    color: var(--warn);
  }

  .ct-rows {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
    min-width: 0;
  }

  .ct-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: baseline;
    gap: var(--sp-2);
    min-width: 0;
    font-size: var(--fs-sm);
  }

  .ct-name {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The size is the receipt for the number beside it: present but never
     competing with the name or the money. */
  .ct-size {
    font-family: var(--mono);
    font-size: var(--fs-label);
    color: var(--text-faint);
  }

  .ct-money {
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
</style>
