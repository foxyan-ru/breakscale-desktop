<script lang="ts">
  /**
   * One request, hop by hop, split into waiting and working.
   *
   * Ported from `src/components/Trace.tsx`. Self-contained rather than
   * prop-driven: it reads `simulationStore.snapshot?.trace` and resolves
   * node names from `topologyStore.topology.nodes` itself, so any panel can
   * mount `<Trace />` with no wiring, matching how `Metrics.svelte` and
   * `Cost.svelte` are also self-contained in this port.
   *
   * WHY IT EXISTS. Every other reading in this app is an aggregate: a rate,
   * a percentile, a mean. Those say latency ROSE, never WHERE it went, and a
   * reader watching p99 climb has no way to tell whether the work got
   * slower or whether the request simply stood in a line. Under load it is
   * almost always the line, and that is the single most important thing
   * this simulator has to teach -- one real request, not a statistic.
   */
  import { simulationStore } from '$lib/state/simulation.svelte';
  import { topologyStore } from '$lib/state/topology.svelte';
  import { tooltip } from '$lib/components/shell/Tooltip.svelte';
  import type { RequestTrace } from '$lib/domain';

  const snapshot = $derived(simulationStore.snapshot);
  const trace: RequestTrace | null = $derived(snapshot?.trace ?? null);

  const nameById = $derived.by(() => {
    const map = new Map<string, string>();
    for (const n of topologyStore.topology.nodes) map.set(n.id, n.label);
    return map;
  });

  /** Resolve a node id to the name shown on the canvas, falling back to the
   * id itself so a node renamed or deleted mid-trace stays readable. */
  function nameOf(id: string): string {
    return nameById.get(id) ?? id;
  }

  /** Widest total on any row, so every bar shares one scale. */
  function scaleOf(t: RequestTrace): number {
    let max = 0;
    for (const h of t.hops) {
      const total = h.queuedMs + h.serviceMs;
      if (total > max) max = total;
    }
    return max;
  }

  const fmt = (ms: number) => (ms >= 100 ? ms.toFixed(0) : ms.toFixed(1));

  const totals = $derived.by(() => {
    if (!trace) return null;
    let queued = 0;
    let service = 0;
    for (const h of trace.hops) {
      queued += h.queuedMs;
      service += h.serviceMs;
    }
    return { queued, service, scale: scaleOf(trace) };
  });

  // Which half of the latency dominates. Stated in words rather than left to
  // be inferred from two bar lengths, because that sentence is the panel's
  // whole point.
  const verdict = $derived.by(() => {
    if (!totals) return '';
    const { queued, service } = totals;
    if (queued > service * 1.5) return 'Most of this request was spent waiting, not working.';
    if (queued < service * 0.25) return 'Almost all of this was real work. Nothing was queueing.';
    return 'Waiting and working are close. Raise the load to see that change.';
  });

  const loading = $derived(snapshot === null);
</script>

<section class="tr" aria-label="Request trace">
  <header class="mx-head">
    <span class="label mx-eyebrow">One request</span>
    {#if trace && totals}
      <span class="mx-head-right">
        <span class="mx-readout-wrap">
          <span class={`num num-lg mx-readout${trace.ok ? '' : ' is-danger'}`}>
            {fmt(trace.totalMs)}
          </span>
          <span class="unit mx-unit">ms</span>
        </span>
      </span>
    {/if}
  </header>

  {#if loading}
    <p class="tr-empty">Waiting for the simulation to start.</p>
  {:else if !trace || !totals}
    <p class="tr-empty" aria-label="No request has been traced yet">
      Waiting for a request to finish. Press play, or raise the load.
    </p>
  {:else}
    {@const { queued, service, scale } = totals}
    {#if !trace.ok}
      <p class="tr-failed">
        This one did not finish: <strong>{trace.reason}</strong>. The hops below are how far it
        got.
      </p>
    {/if}

    <ol
      class="tr-rows"
      aria-label={`${trace.hops.length} hop request trace totalling ${fmt(trace.totalMs)}ms, ${fmt(queued)}ms waiting and ${fmt(service)}ms working`}
    >
      {#each trace.hops as h, i (`${h.nodeId}-${h.depth}-${i}`)}
        {@const total = h.queuedMs + h.serviceMs}
        {@const pct = (v: number) => (scale > 0 ? (v / scale) * 100 : 0)}
        <li class="tr-row">
          <span class="tr-name" title={nameOf(h.nodeId)}>{nameOf(h.nodeId)}</span>
          <span class="tr-bar">
            <span class="tr-queued" style={`width: ${pct(h.queuedMs)}%`}></span>
            <span class="tr-service" style={`width: ${pct(h.serviceMs)}%`}></span>
          </span>
          <span class="num tr-ms">{fmt(total)}</span>
        </li>
      {/each}
    </ol>

    <footer class="tr-foot">
      <span class="tr-key">
        <span class="tr-swatch tr-queued"></span>
        <span use:tooltip={{ id: 'queue' }}>waiting</span> {fmt(queued)}ms
      </span>
      <span class="tr-key">
        <span class="tr-swatch tr-service"></span>
        working {fmt(service)}ms
      </span>
    </footer>
    <p class="tr-verdict">{verdict}</p>
  {/if}
</section>

<style>
  /* Ported from src/components/Trace.css.

     Two colours carry the whole idea, so they must be tellable apart at a
     glance and mean the same thing everywhere: WAITING is the warning ramp,
     because time in a queue is the symptom of a system past its capacity,
     and WORKING is the neutral accent, because service time is the system
     doing its job. Reversing them would praise the wrong half. */

  .tr {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .tr-empty,
  .tr-failed,
  .tr-verdict {
    margin: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
  }

  .tr-failed {
    margin-bottom: var(--sp-2);
    color: var(--danger);
  }

  .tr-verdict {
    margin-top: var(--sp-2);
  }

  .tr-rows {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
    min-width: 0;
  }

  .tr-row {
    display: grid;
    /* Name, bar, number. The name is capped so one long component label
       cannot squeeze every bar in the panel down to nothing. */
    grid-template-columns: minmax(0, 8.5rem) 1fr auto;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .tr-name {
    overflow: hidden;
    font-size: var(--fs-sm);
    color: var(--text-dim);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tr-bar {
    display: flex;
    height: 12px;
    min-width: 0;
    background: var(--track);
    border-radius: var(--r-mark);
    overflow: hidden;
  }

  /* A hop that queued for zero still shows nothing, which is correct: an
     empty left half IS the reading at low load, and inventing a minimum
     width would draw a wait that never happened. */
  .tr-queued {
    background: var(--warn-mark);
  }

  .tr-service {
    background: var(--accent);
  }

  .tr-ms {
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }

  .tr-foot {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-3);
    margin-top: var(--sp-2);
    font-size: var(--fs-sm);
    color: var(--text-dim);
  }

  .tr-key {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
  }

  .tr-swatch {
    width: 10px;
    height: 10px;
    border-radius: var(--r-mark);
  }
</style>
