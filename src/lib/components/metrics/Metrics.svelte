<script lang="ts">
  /* ========================================================================
   * The bottom charts strip. Ported from `src/components/Metrics.tsx` +
   * `Metrics.css`.
   *
   * Self-contained: reads `simulationStore.snapshot` directly (no props),
   * matching `Trace.svelte`/`Cost.svelte` in this port, and renders both of
   * those as its last two grid children -- the same five-panel `.mx` grid
   * the original composed (3 charts + trace + cost), just now assembled
   * from separately-mountable Svelte files instead of one React component
   * owning all five.
   *
   * Every chart is two stacked SVGs sharing one box: a plot layer with
   * `preserveAspectRatio="none"` so polylines stretch to fill the
   * container's width, and an overlay layer at 1:1 scale carrying all the
   * text -- so labels stay crisp at any width, never smeared by the
   * stretched coordinate space. The overlay's real pixel width is measured
   * with Svelte's `bind:clientWidth`, which replaces the original's
   * ResizeObserver-backed `useMeasuredWidth` hook outright (calc() is
   * invalid in SVG geometry presentation attributes and silently resolves
   * to 0, which is why this is measured rather than computed from CSS).
   *
   * THE CALM RULE. A healthy system must look calm: nothing here is
   * coloured until something is actually wrong. The latency trace is
   * neutral grey until p99 crosses a threshold, the dropped-traffic wedge is
   * not painted at all until traffic is genuinely being lost, and the
   * failures panel collapses to a single sentence when there is nothing to
   * report. Colour that is always present carries no information; colour
   * that appears is a signal.
   * ======================================================================== */

  import { simulationStore } from '$lib/state/simulation.svelte';
  import { tooltip } from '$lib/components/shell/Tooltip.svelte';
  import {
    formatMs,
    formatPct,
    formatRate,
    formatCompact,
    healthOfLatency,
    toneClass,
  } from '$lib/components/canvas/format';
  import Trace from '../trace/Trace.svelte';
  import Cost from '../cost/Cost.svelte';
  import type { FailureReason, HistoryPoint } from '$lib/domain';

  /* ---- chart geometry, unchanged from the source file ------------------ */

  const PLOT_W = 1000;
  const PLOT_H = 100;
  const PAD_L = 40;
  const PAD_R = 16;
  const PAD_T = 16;
  const PAD_B = 28;
  const CHART_H = 176;
  const WINDOW_MS = 60_000;
  const FALLBACK_W = 320;

  /* ---- scales, ported unchanged from Metrics.tsx ------------------------ */

  /** Round `v` up to the next 1/2/5 x 10^n. Always returns > 0. */
  function niceCeil(v: number): number {
    if (!Number.isFinite(v) || v <= 0) return 1;
    const exp = Math.floor(Math.log10(v));
    const pow = 10 ** exp;
    if (!Number.isFinite(pow) || pow <= 0) return 1;
    const frac = v / pow;
    const nice = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 5 ? 5 : 10;
    const out = nice * pow;
    return Number.isFinite(out) && out > 0 ? out : 1;
  }

  /**
   * Tick values for an axis, chosen so every LABEL is a round number. A
   * round STEP from the 1/2/5 family is picked so the axis carries four or
   * five gridlines, and ticks are multiples of it -- quartering the raw top
   * would label the axis with rounded approximations of arbitrary values.
   * The returned top may exceed the requested one; the axis must contain
   * the data, so a round ceiling that covers it is the point.
   */
  function ticksFor(top: number): number[] {
    if (!Number.isFinite(top) || top <= 0) return [0, 1];
    const raw = top / 4;
    const exp = Math.floor(Math.log10(raw));
    const pow = 10 ** exp;
    if (!Number.isFinite(pow) || pow <= 0) return [0, top];
    const frac = raw / pow;
    const nice = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 5 ? 5 : 10;
    // All three axes count things (ms, rps, failures/s), so a fractional
    // gridline is not a finer reading, it is meaningless -- and unrenderable,
    // since formatCompact floors anything under 0.05 to "<0.1".
    const step = Math.max(1, nice * pow);
    if (!Number.isFinite(step) || step <= 0) return [0, top];
    // Ticks must COVER the data: the loop runs until it has passed `top`
    // rather than stopping at it, or the ceiling could sit below the max.
    const out: number[] = [];
    const EPS = step / 1e6;
    for (let i = 0; i <= 8; i += 1) {
      const v = Number((i * step).toPrecision(12));
      out.push(v);
      if (v >= top - EPS) break;
    }
    return out.length >= 2 ? out : [0, top];
  }

  /** Project a value into plot-space y. `top` is guaranteed > 0 by the caller. */
  function yOf(v: number, top: number): number {
    const safe = Number.isFinite(v) ? v : 0;
    const denom = Number.isFinite(top) && top > 0 ? top : 1;
    return PLOT_H - (Math.max(0, Math.min(denom, safe)) / denom) * PLOT_H;
  }

  /** Build an SVG points string. A single sample is emitted twice so the
   * polyline still renders a visible mark rather than nothing at all. */
  function polylinePoints(
    history: readonly HistoryPoint[],
    xAt: (t: number) => number,
    value: (p: HistoryPoint) => number,
    top: number,
  ): string {
    if (history.length === 0) return '';
    const pts = history.map((p) => `${xAt(p.t).toFixed(2)},${yOf(value(p), top).toFixed(2)}`);
    if (pts.length === 1) return `${pts[0]} ${pts[0]}`;
    return pts.join(' ');
  }

  function xScale(history: readonly { t: number }[]): (t: number) => number {
    const n = history.length;
    if (n === 0) return () => 0;
    const tEnd = history[n - 1]!.t;
    const tStart = tEnd - WINDOW_MS;
    return (t: number) => {
      const f = (t - tStart) / WINDOW_MS;
      return Number.isFinite(f) ? Math.max(0, Math.min(1, f)) * PLOT_W : 0;
    };
  }

  /**
   * A y-axis top that rises immediately but falls only reluctantly, so the
   * axis does not visibly rescale under the reader's cursor mid-drag. An
   * increase applies at once; a decrease waits until the observed max has
   * stayed below half the current top for a continuous 5 real seconds.
   *
   * Ported as a small reactive factory (called once per chart) rather than
   * the source's `useStickyAxis` hook + ref: Svelte runes used inside a
   * plain function defined in a component's script block stay reactive and
   * scoped to that call, which is the direct equivalent. Uses a real
   * `setTimeout` for the 5s hold rather than re-checking `performance.now()`
   * only when the effect happens to re-run (which is what the React version
   * does, since its effect's dependency array skips a rerun whenever
   * `target`/`top` are unchanged between renders) -- a small deliberate
   * improvement so the shrink is not dependent on the window's max jittering
   * again within the 5s, and is cleaned up on every reschedule/unmount.
   */
  function stickyAxis(getMax: () => number, fallback: number) {
    let top = $state(fallback);
    let lowSince: number | null = null;

    $effect(() => {
      const max = getMax();
      const safe = Number.isFinite(max) && max > 0 ? max : 0;
      const target = safe > 0 ? niceCeil(safe * 1.15) : fallback;

      let timer: ReturnType<typeof setTimeout> | undefined;

      if (target > top) {
        lowSince = null;
        top = target;
      } else if (target >= top) {
        lowSince = null;
      } else if (target > top / 2) {
        // Not low enough to count -- a 5% dip should not start the clock.
        lowSince = null;
      } else {
        if (lowSince === null) lowSince = performance.now();
        const remaining = Math.max(0, 5000 - (performance.now() - lowSince));
        timer = setTimeout(() => {
          lowSince = null;
          top = target;
        }, remaining);
      }

      return () => {
        if (timer !== undefined) clearTimeout(timer);
      };
    });

    // The axis ceiling is raised to the HIGHEST TICK rather than the ticks
    // being squeezed under an arbitrary ceiling, or the top gridline could
    // float below the plot's top and a trace could be drawn above its own
    // axis.
    const safeTop = $derived(Number.isFinite(top) && top > 0 ? top : 1);
    const ticks = $derived(ticksFor(safeTop));
    const ceiling = $derived.by(() => {
      const highest = ticks[ticks.length - 1] ?? safeTop;
      return Number.isFinite(highest) && highest > 0 ? highest : safeTop;
    });

    return {
      get top() {
        return ceiling;
      },
      get ticks() {
        return ticks;
      },
    };
  }

  /* ---- failure breakdown vocabulary, ported unchanged ------------------- */

  const REASON_ORDER: FailureReason[] = [
    'error',
    'shed',
    'timeout',
    'no-route',
    'depth',
    'throttled',
    'rejected',
    'crashed',
    'partitioned',
    'region-down',
    'conn-refused',
    'unauthorized',
    'bulkhead-full',
    'acquire-timeout',
    'deprioritized',
  ];

  const REASON_LABEL: Record<FailureReason, string> = {
    error: 'error',
    shed: 'shed',
    timeout: 'timeout',
    'no-route': 'no route',
    depth: 'depth',
    throttled: 'throttled',
    rejected: 'rejected',
    crashed: 'crashed',
    partitioned: 'partitioned',
    'region-down': 'region down',
    'conn-refused': 'conn refused',
    unauthorized: 'unauthorized',
    'bulkhead-full': 'bulkhead full',
    'acquire-timeout': 'acquire timeout',
    deprioritized: 'deprioritized',
  };

  /** Glossary id for each failure reason, for the `tooltip` action. */
  const REASON_TERM: Record<FailureReason, string> = {
    error: 'error-rate',
    shed: 'shed',
    timeout: 'timeout',
    'no-route': 'no-route',
    depth: 'depth-limit',
    throttled: 'throttled',
    rejected: 'rejected',
    crashed: 'crashed',
    partitioned: 'partitioned',
    'region-down': 'region-down',
    'conn-refused': 'conn-refused',
    unauthorized: 'unauthorized',
    'bulkhead-full': 'bulkhead-full',
    // Reuses the 'bulkhead-full' glossary entry rather than a dedicated
    // one: an acquire-timeout is a bulkhead refusing a waiter, the same
    // concept the existing entry already explains (extended to mention the
    // wait path -- see glossary.json's 'bulkhead-full' entry). Matches
    // upstream's own choice here (`behaviour-resilience.ts`'s
    // `REASON_TERM`, upstream `351327c4`, PR #77).
    'acquire-timeout': 'bulkhead-full',
    deprioritized: 'deprioritized',
  };

  const EMPTY_BY: Record<FailureReason, number> = Object.fromEntries(
    REASON_ORDER.map((r) => [r, 0]),
  ) as Record<FailureReason, number>;

  interface FailSample {
    t: number;
    by: Record<FailureReason, number>;
  }

  /* ---- live data ---------------------------------------------------------
   * The engine now runs in-process (see `$lib/state/simulation.svelte.ts`'s
   * header), same as the web app's: `history` and `failuresByReason` are
   * the SAME object across snapshots, mutated in place every tick (engine
   * perf optimisation -- see `$lib/sim/engine.ts`'s own `snapshot()`
   * comment). This USED TO say the opposite, back when every snapshot
   * crossed Tauri IPC as a freshly deserialised object -- that was never
   * true upstream and isn't true here anymore either.
   *
   * This means a `$derived` computed FROM `failuresByReason` alone (nothing
   * here does, currently) would silently stop updating: Svelte's `$derived`
   * skips notifying dependents when its result is referentially unchanged
   * (https://svelte.dev/docs/svelte/$derived), and `failuresByReason`'s
   * object identity never changes. The one place that reads it below (the
   * rate-conversion effect) stays correct only because it ALSO reads
   * `system?.timeMs`, which IS a fresh value every tick and so keeps the
   * effect re-running regardless -- reading the mutated-in-place
   * `failuresByReason` on each of those runs still gets the current
   * counts. Don't remove that `system?.timeMs` read without replacing it
   * with some other always-changing dependency (e.g. depending on
   * `snapshot` itself, as `Canvas.svelte`'s `backlogs` does), or this
   * effect stops updating too.
   * ------------------------------------------------------------------- */

  const snapshot = $derived(simulationStore.snapshot);
  const system = $derived(snapshot?.system ?? null);
  const failuresByReason = $derived(snapshot?.failuresByReason ?? null);
  const loading = $derived(snapshot === null);

  // Only the trailing 60s window is ever drawn, so the charts stay cheap no
  // matter how long the sim has run.
  const windowed = $derived.by((): HistoryPoint[] => {
    const history = snapshot?.history ?? [];
    if (history.length === 0) return [];
    const cutoff = history[history.length - 1]!.t - WINDOW_MS;
    let i = 0;
    while (i < history.length && history[i]!.t < cutoff) i += 1;
    return history.slice(i);
  });

  /* `failuresByReason` is a LIFETIME COUNT, not a rate -- the engine
   * increments it on each failure and only zeroes it on reset. Rendering it
   * directly labelled "/s" would be wrong three ways: the unit would be a
   * lie, the figure would only ever grow, and a recovered system would still
   * show thousands "failing" because a counter cannot fall. Differencing
   * successive samples against elapsed SIM time (not a wall clock, which
   * would invent traffic while the sim is paused) turns the counter into the
   * rate this panel actually shows. */
  let prevCounts: Record<FailureReason, number> | null = $state(null);
  let prevTime = $state(0);
  let rates: Record<FailureReason, number> = $state({ ...EMPTY_BY });

  $effect(() => {
    const cumulative = failuresByReason;
    const now = system?.timeMs;
    if (!cumulative || now === undefined || !Number.isFinite(now)) return;

    const full = { ...EMPTY_BY, ...cumulative };
    const prev = prevCounts;
    const dtMs = now - prevTime;
    const wentBackwards =
      prev !== null && REASON_ORDER.some((r) => (full[r] ?? 0) < (prev[r] ?? 0));

    // First sample, or a reset (time or any counter moved backwards): adopt
    // the counts as the new baseline and report nothing this tick.
    if (prev === null || dtMs < 0 || wentBackwards) {
      prevCounts = full;
      prevTime = now;
      rates = { ...EMPTY_BY };
      return;
    }
    // Sample no faster than 250ms of sim time: below that the divisor is
    // tiny and the quotient is mostly quantisation noise.
    if (dtMs < 250) return;

    const next = { ...EMPTY_BY };
    const perSec = 1000 / dtMs;
    for (const r of REASON_ORDER) {
      const delta = (full[r] ?? 0) - (prev[r] ?? 0);
      next[r] = delta > 0 ? delta * perSec : 0;
    }
    prevCounts = full;
    prevTime = now;
    rates = next;
  });

  /** Loss per second, from the differenced rates -- drives the throughput
   * chart's dropped-traffic wedge. Gating that on a lifetime count would
   * leave the wedge painted for the rest of the session after one early
   * failure, so it must be a rate. */
  const lostRate = $derived.by(() => {
    let s = 0;
    for (const r of REASON_ORDER) {
      const v = rates[r];
      if (Number.isFinite(v) && v > 0) s += v;
    }
    return s;
  });

  /* The engine's history has no per-reason series, so the strip keeps its
   * own 60-entry / 1Hz ring buffer of the live failure mix, sampled on sim
   * time crossing a 1s boundary so it follows pause/step/reset exactly. */
  let failRef: FailSample[] = $state([]);
  let lastBucket = $state(-1);

  $effect(() => {
    const now = system?.timeMs;
    const by = rates;
    if (now === undefined || !Number.isFinite(now)) return;
    const bucket = Math.floor(now / 1000);
    if (bucket < lastBucket) {
      failRef = [];
      lastBucket = -1;
    }
    if (bucket === lastBucket) return;
    lastBucket = bucket;
    const next = [...failRef, { t: now, by }];
    failRef = next.length > 60 ? next.slice(next.length - 60) : next;
  });

  /* ---- measured widths, via Svelte's own bind:clientWidth --------------- */

  let latencyW = $state(FALLBACK_W);
  let throughputW = $state(FALLBACK_W);
  let failureW = $state(FALLBACK_W);

  const xAt = $derived(xScale(windowed));

  /* ---- latency chart ----------------------------------------------------- */

  const latencyMax = $derived.by(() => {
    let m = 0;
    for (const p of windowed) if (Number.isFinite(p.p99) && p.p99 > m) m = p.p99;
    return m;
  });
  const latencyAxis = stickyAxis(() => latencyMax, 100);

  const p50Path = $derived(polylinePoints(windowed, xAt, (p) => p.p50, latencyAxis.top));
  const p95Path = $derived(polylinePoints(windowed, xAt, (p) => p.p95, latencyAxis.top));
  const p99Path = $derived(polylinePoints(windowed, xAt, (p) => p.p99, latencyAxis.top));

  const p99Now = $derived(system?.p99 ?? 0);
  const latencyTone = $derived(healthOfLatency(p99Now));
  const latencyEmpty = $derived(windowed.length === 0);

  /* ---- throughput chart --------------------------------------------------- *
   * The most instructive visual in the app. The gap between offered and
   * goodput IS the dropped traffic, drawn as literal area rather than left
   * for the eye to infer: when the system keeps up the two lines coincide
   * and the band has zero area, when it sheds a wedge opens. */

  const throughputMax = $derived.by(() => {
    let m = 0;
    for (const p of windowed) {
      if (Number.isFinite(p.offered) && p.offered > m) m = p.offered;
      if (Number.isFinite(p.goodput) && p.goodput > m) m = p.goodput;
    }
    return m;
  });
  const throughputAxis = stickyAxis(() => throughputMax, 10);

  const offeredPath = $derived(polylinePoints(windowed, xAt, (p) => p.offered, throughputAxis.top));
  const goodputPath = $derived(polylinePoints(windowed, xAt, (p) => p.goodput, throughputAxis.top));

  const safeOffered = $derived.by(() => {
    const v = system?.offeredRps;
    return typeof v === 'number' && Number.isFinite(v) ? Math.max(0, v) : 0;
  });
  const safeGoodput = $derived.by(() => {
    const v = system?.goodputRps;
    return typeof v === 'number' && Number.isFinite(v) ? Math.max(0, v) : 0;
  });
  const dropped = $derived(Number.isFinite(lostRate) ? Math.max(0, lostRate) : 0);
  const live = $derived(dropped > 0.5);
  const throughputEmpty = $derived(windowed.length === 0);

  // Only draw the gap when traffic is genuinely being LOST. `offered -
  // goodput` is not loss on its own: it is also the in-flight backlog, which
  // is non-zero in every healthy pipeline. Gating on the real failure total
  // keeps a working system free of red haze.
  const gapPath = $derived.by(() => {
    if (windowed.length <= 1 || lostRate <= 0.5) return '';
    const top = throughputAxis.top;
    const fwd = windowed.map((p) => `${xAt(p.t).toFixed(2)},${yOf(p.offered, top).toFixed(2)}`);
    const rev = windowed
      .map((p) => `${xAt(p.t).toFixed(2)},${yOf(p.goodput, top).toFixed(2)}`)
      .reverse();
    return [...fwd, ...rev].join(' ');
  });

  /* ---- failure breakdown chart -------------------------------------------- */

  const failXAt = $derived(xScale(failRef));

  const active = $derived.by(() => {
    const rows = REASON_ORDER.map((reason) => {
      const raw = rates[reason];
      return { reason, rate: Number.isFinite(raw) && raw > 0 ? raw : 0 };
    }).filter((r) => r.rate > 0);
    rows.sort((a, b) => b.rate - a.rate);
    return rows;
  });
  const total = $derived(active.reduce((s, r) => s + r.rate, 0));

  // Axis top tracks the largest stacked total in the window AND the live
  // total -- for the first second after a reset there is only one sample, so
  // a window-only maximum would leave the axis at its floor while the header
  // already states a real figure.
  const maxFail = $derived.by(() => {
    let m = total;
    for (const s of failRef) {
      let sum = 0;
      for (const reason of REASON_ORDER) {
        const v = s.by[reason];
        if (Number.isFinite(v) && v > 0) sum += v;
      }
      if (sum > m) m = sum;
    }
    return m;
  });
  const failureAxis = stickyAxis(() => maxFail, 1);

  /** Stacked bands, bottom to top: each is a closed polygon, its own
   * cumulative top edge forward and the previous band's top edge back. */
  const bands = $derived.by(() => {
    const samples = failRef;
    if (samples.length === 0) return [] as { reason: FailureReason; points: string }[];
    const top = failureAxis.top;
    const cum = samples.map(() => 0);
    const out: { reason: FailureReason; points: string }[] = [];

    for (const reason of REASON_ORDER) {
      const lower = cum.slice();
      let any = false;
      samples.forEach((s, i) => {
        const v = s.by[reason];
        if (Number.isFinite(v) && v > 0) {
          cum[i] = lower[i]! + v;
          any = true;
        } else {
          cum[i] = lower[i]!;
        }
      });
      if (!any) continue;

      // A single sample has no horizontal extent, so forward and reverse
      // edges would collapse onto one another and paint nothing -- give it a
      // short run back from the right edge instead.
      const SOLO_W = 6;
      const xs = samples.map((s) => (samples.length === 1 ? PLOT_W - SOLO_W : failXAt(s.t)));
      const fwd = samples.map((_s, i) => `${xs[i]!.toFixed(2)},${yOf(cum[i]!, top).toFixed(2)}`);
      if (samples.length === 1) fwd.push(`${PLOT_W.toFixed(2)},${yOf(cum[0]!, top).toFixed(2)}`);
      const rev = samples
        .map((_s, i) => `${xs[i]!.toFixed(2)},${yOf(lower[i]!, top).toFixed(2)}`)
        .reverse();
      if (samples.length === 1) rev.unshift(`${PLOT_W.toFixed(2)},${yOf(lower[0]!, top).toFixed(2)}`);
      out.push({ reason, points: [...fwd, ...rev].join(' ') });
    }
    return out;
  });

  const windowHadFailures = $derived(bands.length > 0);
  const failureEmpty = $derived(failRef.length === 0);
  const healthy = $derived(total <= 0);

  /* ---- aria summaries: a chart's text alternative, not decoration -------- */

  const latencyAriaLabel = $derived(
    `Latency over the last 60 seconds. p99 is ${formatMs(p99Now)}.`,
  );
  const throughputAriaLabel = $derived(
    `Throughput over the last 60 seconds. ${formatCompact(safeGoodput)} succeeding per second` +
      (live ? `, ${formatCompact(dropped)} lost per second.` : '.'),
  );
  const failureAriaLabel = $derived(
    `Failures over the last 60 seconds. ${healthy ? 'Nothing is failing right now.' : `${formatCompact(total)} failures per second.`}`,
  );
</script>

{#snippet frame(ticks: number[], top: number, w: number, empty: boolean)}
  {@const plotH = CHART_H - PAD_T - PAD_B}
  {@const right = Math.max(PAD_L + 1, w - PAD_R)}
  {@const baseY = PAD_T + plotH}
  {@const safeTop = Number.isFinite(top) && top > 0 ? top : 1}
  {@const xTicks = [0, 1, 2, 3].map((i) => PAD_L + ((right - PAD_L) * i) / 3)}
  {#each ticks as t, i (i)}
    {@const y = Math.round(PAD_T + plotH - (t / safeTop) * plotH) + 0.5}
    <g>
      <line
        class={i === 0 ? 'mx-zero' : 'mx-grid'}
        x1={i === 0 ? PAD_L - 4 : PAD_L}
        y1={y}
        x2={right}
        y2={y}
      />
      <text class="mx-ytick num" x={PAD_L - 8} y={y} dy="0.32em" text-anchor="end">
        {formatCompact(t)}
      </text>
    </g>
  {/each}

  {#each xTicks as x, i (i)}
    <line
      class="mx-xtick"
      x1={Math.round(x) + 0.5}
      y1={baseY}
      x2={Math.round(x) + 0.5}
      y2={baseY + 3}
    />
  {/each}

  <line
    class="mx-now"
    x1={Math.round(right) + 0.5}
    y1={PAD_T}
    x2={Math.round(right) + 0.5}
    y2={baseY}
  />

  <text class="mx-xlabel" x={PAD_L} y={CHART_H - 8}>60s ago</text>
  <text class="mx-xlabel" x={right} y={CHART_H - 8} text-anchor="end">now</text>

  {#if empty}
    <text
      class="mx-empty"
      x={(PAD_L + right) / 2}
      y={PAD_T + plotH / 2}
      dy="0.32em"
      text-anchor="middle"
    >
      Waiting for traffic
    </text>
  {/if}
{/snippet}

{#if loading}
  <div class="mx">
    <p class="mx-loading">Waiting for the simulation to start.</p>
  </div>
{:else}
  <div class="mx">
    <!-- ------------------------------------------------------------ Latency -->
    <section class="mx-chart" aria-label={latencyAriaLabel}>
      <header class="mx-head">
        <span class="label mx-eyebrow" use:tooltip={{ id: 'latency' }}>Latency</span>
        <span class="mx-head-right">
          <span class="mx-readout-wrap">
            <span class={`num num-lg mx-readout ${toneClass(latencyTone) ?? ''}`}>
              {formatMs(p99Now)}
            </span>
            <span class="unit mx-unit" use:tooltip={{ id: 'p99' }}>p99</span>
          </span>
        </span>
      </header>

      <div class="mx-plot" style={`height: ${CHART_H}px`} bind:clientWidth={latencyW}>
        <svg
          class="mx-layer mx-layer-scaled"
          viewBox={`0 0 ${PLOT_W} ${PLOT_H}`}
          preserveAspectRatio="none"
          style={`top: ${PAD_T}px; left: ${PAD_L}px; width: ${Math.max(1, latencyW - PAD_L - PAD_R)}px; height: ${CHART_H - PAD_T - PAD_B}px`}
          aria-hidden="true"
        >
          <polyline class="mx-line mx-line-p50" points={p50Path} />
          <polyline class="mx-line mx-line-p95" points={p95Path} />
          <polyline
            class={`mx-line mx-line-p99 ${toneClass(latencyTone) ?? ''}`}
            points={p99Path}
          />
        </svg>
        <svg class="mx-layer" aria-hidden="true">
          {@render frame(latencyAxis.ticks, latencyAxis.top, latencyW, latencyEmpty)}
        </svg>
      </div>

      <ul class="mx-legend">
        <li class="mx-key">
          <span class="mx-key-line mx-key-p50" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'p50' }}>p50</span>
          <span class="num num-sm mx-key-value">{formatMs(system?.p50 ?? null)}</span>
        </li>
        <li class="mx-key">
          <span class="mx-key-line mx-key-p95" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'p95' }}>p95</span>
          <span class="num num-sm mx-key-value">{formatMs(system?.p95 ?? null)}</span>
        </li>
        <li class="mx-key">
          <span class="mx-key-line mx-key-p99" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'p99' }}>p99</span>
          <span class="num num-sm mx-key-value">{formatMs(p99Now)}</span>
        </li>
      </ul>
    </section>

    <!-- --------------------------------------------------------- Throughput -->
    <section class="mx-chart" aria-label={throughputAriaLabel}>
      <header class="mx-head">
        <span class="label mx-eyebrow" use:tooltip={{ id: 'throughput' }}>Throughput</span>
        <span class="mx-head-right">
          {#if live}
            <span class="mx-dropped">
              <span class="label" use:tooltip={{ id: 'dropped' }}>Lost</span>
              <span class="num num-md is-danger">{formatCompact(dropped)}</span>
            </span>
          {/if}
          <span class="mx-readout-wrap">
            <span class="num num-lg mx-readout">{formatCompact(safeGoodput)}</span>
            <span class="unit mx-unit" use:tooltip={{ id: 'goodput' }}>succeeding /s</span>
          </span>
        </span>
      </header>

      <div class="mx-plot" style={`height: ${CHART_H}px`} bind:clientWidth={throughputW}>
        <svg
          class="mx-layer mx-layer-scaled"
          viewBox={`0 0 ${PLOT_W} ${PLOT_H}`}
          preserveAspectRatio="none"
          style={`top: ${PAD_T}px; left: ${PAD_L}px; width: ${Math.max(1, throughputW - PAD_L - PAD_R)}px; height: ${CHART_H - PAD_T - PAD_B}px`}
          aria-hidden="true"
        >
          {#if gapPath}
            <polygon class="mx-gap" points={gapPath} />
          {/if}
          <polyline class="mx-line mx-line-offered" points={offeredPath} />
          <polyline class="mx-line mx-line-goodput" points={goodputPath} />
        </svg>
        <svg class="mx-layer" aria-hidden="true">
          {@render frame(throughputAxis.ticks, throughputAxis.top, throughputW, throughputEmpty)}
        </svg>
      </div>

      <ul class="mx-legend">
        <li class="mx-key">
          <span class="mx-key-line mx-key-offered" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'offered' }}>offered</span>
          <span class="num num-sm mx-key-value">{formatCompact(safeOffered)}</span>
        </li>
        <li class="mx-key">
          <span class="mx-key-line mx-key-goodput" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'goodput' }}>succeeded</span>
          <span class="num num-sm mx-key-value">{formatCompact(safeGoodput)}</span>
        </li>
        <li class="mx-key" data-dim={!live || undefined}>
          <span class="mx-key-gap" aria-hidden="true"></span>
          <span class="mx-key-name" use:tooltip={{ id: 'dropped' }}>lost</span>
          <span class="num num-sm mx-key-value">{formatCompact(dropped)}</span>
        </li>
      </ul>
    </section>

    <!-- ----------------------------------------------------------- Failures -->
    <section class="mx-chart mx-fail-col" aria-label={failureAriaLabel}>
      <header class="mx-head">
        <span class="label mx-eyebrow" use:tooltip={{ id: 'error-rate' }}>Failures</span>
        <span class="mx-head-right">
          <span class="mx-readout-wrap">
            <span class={`num num-lg mx-readout ${healthy ? '' : 'is-danger'}`}>
              {healthy ? '0' : formatCompact(total)}
            </span>
            <span class="unit mx-unit" use:tooltip={{ id: 'rps' }}>/s</span>
          </span>
        </span>
      </header>

      <div class="mx-plot" style={`height: ${CHART_H}px`} bind:clientWidth={failureW}>
        <svg
          class="mx-layer mx-layer-scaled"
          viewBox={`0 0 ${PLOT_W} ${PLOT_H}`}
          preserveAspectRatio="none"
          style={`top: ${PAD_T}px; left: ${PAD_L}px; width: ${Math.max(1, failureW - PAD_L - PAD_R)}px; height: ${CHART_H - PAD_T - PAD_B}px`}
          aria-hidden="true"
        >
          {#each bands as b (b.reason)}
            <polygon class={`mx-band mx-fill-${b.reason}`} points={b.points} />
          {/each}
        </svg>
        <svg class="mx-layer" aria-hidden="true">
          {@render frame(failureAxis.ticks, failureAxis.top, failureW, failureEmpty)}
        </svg>
      </div>

      <!-- THE HEALTHY CASE IS ONE LINE, NOT FOURTEEN ZEROS. A working system
           says so in a single calm sentence; a permanent block of `0/s` rows
           trains the eye to skip the panel, so when a row finally goes
           non-zero nobody is looking at it any more. -->
      {#if healthy}
        <p class="mx-clean">
          {failureEmpty
            ? 'No traffic yet.'
            : windowHadFailures
              ? 'Recovered. Nothing is failing now.'
              : 'Nothing has failed in the last 60 seconds.'}
        </p>
      {:else}
        <ul class="mx-legend mx-legend-stack">
          {#each active as r (r.reason)}
            {@const share = total > 0 ? r.rate / total : 0}
            <li class="mx-key mx-key-fail">
              <span class={`mx-key-swatch mx-fill-${r.reason}`} aria-hidden="true"></span>
              <span class="mx-key-name" use:tooltip={{ id: REASON_TERM[r.reason] }}>
                {REASON_LABEL[r.reason]}
              </span>
              <span class="mx-key-bar" aria-hidden="true">
                <span
                  class={`mx-key-bar-fill mx-fill-${r.reason}`}
                  style={`width: ${Math.min(100, share * 100)}%`}
                ></span>
              </span>
              <span class="num num-sm mx-key-value">{formatRate(r.rate)}</span>
              <span class="num num-sm mx-key-share">{formatPct(share)}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <Trace />
    <!-- Renders nothing on generic, where there are no prices to show. -->
    <Cost />
  </div>
{/if}

<style>
  /* Ported from src/components/Metrics.css. Everything shared across panels
     (.mx-head/.mx-eyebrow/.mx-head-right/.mx-readout-wrap/.mx-readout/.mx-unit)
     is already global, defined once in app.css -- this block is exactly
     what remains: this strip's own grid, plot geometry, per-series colours
     and legend, which belong to no other panel. */

  .mx {
    display: grid;
    /* Auto-fitting rather than a fixed count: the strip carries three
       charts, the request trace and, once a vendor is chosen, the cost. A
       hardcoded column count would either squeeze five panels into four
       columns or leave a gap on generic where the cost panel renders
       nothing. */
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: var(--sp-5);
    align-items: start;
    padding: var(--sp-4) var(--sp-5) var(--sp-5);
    min-width: 0;
  }

  .mx-loading {
    margin: 0;
    padding: var(--sp-5) 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
    grid-column: 1 / -1;
  }

  .mx-chart {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  /* --------------------------------------------------------------------
     Plot box: two stacked layers sharing one coordinate origin
     -------------------------------------------------------------------- */

  .mx-plot {
    position: relative;
    width: 100%;
    min-width: 0;
  }

  .mx-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  /* The stretched layer is positioned and sized by the component in real
     pixels, so it must not inherit the inset above. */
  .mx-layer-scaled {
    inset: auto;
  }

  /* --------------------------------------------------------------------
     Frame: gridlines, axes, ticks. The grid recedes almost to invisibility
     -- a ruled field reads as a workspace, a loud one competes with the
     data drawn on top of it.
     -------------------------------------------------------------------- */

  .mx-grid {
    stroke: var(--grid-major);
    stroke-width: 1;
  }

  /* The zero line is the one rule that carries meaning: the floor every
     trace is measured from. */
  .mx-zero {
    stroke: var(--border-strong);
    stroke-width: 1;
  }

  .mx-xtick {
    stroke: var(--border-strong);
    stroke-width: 1;
  }

  /* "Now" -- the right edge traces scroll into. Dashed so it reads as a
     moving reference rather than another axis. */
  .mx-now {
    stroke: var(--border-strong);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }

  .mx-ytick {
    fill: var(--text-faint);
    font-family: var(--mono);
    font-variant-numeric: tabular-nums;
    font-feature-settings: 'tnum' 1;
    font-size: var(--fs-label);
    letter-spacing: 0;
  }

  .mx-xlabel {
    fill: var(--text-faint);
    font-family: var(--sans);
    font-size: var(--fs-label);
    font-weight: var(--fw-med);
    letter-spacing: var(--tr-label);
  }

  .mx-empty {
    fill: var(--text-faint);
    font-family: var(--sans);
    font-size: var(--fs-label);
    font-weight: var(--fw-med);
    letter-spacing: var(--tr-label);
  }

  /* --------------------------------------------------------------------
     Traces. vector-effect keeps the stroke a true 1.5px no matter how far
     the plot layer is stretched horizontally.
     -------------------------------------------------------------------- */

  .mx-line {
    fill: none;
    stroke-width: 1.5;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }

  /* Latency: one ink at three weights. p99 is the subject, so it is the
     darkest and the only one that ever takes a health colour. */
  .mx-line-p50 {
    stroke: var(--line-3);
  }

  .mx-line-p95 {
    stroke: var(--line-4);
  }

  .mx-line-p99 {
    stroke: var(--text-dim);
    stroke-width: 2;
  }

  .mx-line-p99.is-warn {
    stroke: var(--warn-mark);
  }

  .mx-line-p99.is-danger {
    stroke: var(--danger-mark);
  }

  /* Throughput. Offered is the reference and stays quiet; goodput is the
     answer to "how much actually got through", so it carries the accent
     and the heavier stroke. */
  .mx-line-offered {
    stroke: var(--line-3);
    stroke-dasharray: 4 3;
  }

  .mx-line-goodput {
    stroke: var(--accent);
    stroke-width: 2;
  }

  /* The gap between offered and goodput IS the dropped traffic. Painted
     only when the engine reports real loss, so a healthy chart has no red
     haze at all -- the two lines simply coincide. */
  .mx-gap {
    fill: var(--danger-mark);
    fill-opacity: 0.14;
    stroke: none;
  }

  /* Stacked failure bands. Solid but not heavy: up to fifteen of them can
     be on screen at once, and at full opacity the stack becomes one dark
     mass. */
  .mx-band {
    stroke: none;
    fill-opacity: 0.85;
  }

  /* --------------------------------------------------------------------
     Failure reason colours. Ten of the fifteen below (error through
     region-down) are ported unchanged from Metrics.css, where every one
     measures at least 3.27:1 against --surface (chosen by maximising the
     minimum OKLab distance between any two). The remaining five
     (conn-refused/unauthorized/bulkhead-full/acquire-timeout/
     deprioritized) are a GAP this port fixes: `FailureReason`
     (sim-types.ts) had 14 variants when this comment was first written
     (now 15, with 'acquire-timeout' added by upstream `351327c4`, PR #77),
     but the source Metrics.css only ever defined 10 `.mx-fill-*` rules, so
     those reasons rendered with no fill at all in the original app. Filled
     in here using each reason's OWN originating component's
     `--kind-*-stroke` token (conn-refused -> websocket, unauthorized ->
     apigateway, bulkhead-full/acquire-timeout -> bulkhead, deprioritized ->
     loadshedder) -- already-measured AA-for-graphics colours from the
     per-kind system in app.css, and a mnemonic a reader can learn
     ("bulkhead-full is bulkhead-coloured").
     -------------------------------------------------------------------- */

  .mx-fill-error {
    fill: var(--danger-mark);
    color: var(--danger-mark);
  }

  .mx-fill-shed {
    fill: var(--warn-mark);
    color: var(--warn-mark);
  }

  .mx-fill-timeout {
    fill: var(--cat-violet);
    color: var(--cat-violet);
  }

  .mx-fill-no-route {
    fill: var(--line-3);
    color: var(--line-3);
  }

  .mx-fill-depth {
    fill: var(--cat-blue);
    color: var(--cat-blue);
  }

  .mx-fill-throttled {
    fill: var(--kind-autoscaler-stroke);
    color: var(--kind-autoscaler-stroke);
  }

  .mx-fill-rejected {
    fill: var(--kind-region-stroke);
    color: var(--kind-region-stroke);
  }

  .mx-fill-crashed {
    fill: var(--line-5);
    color: var(--line-5);
  }

  .mx-fill-partitioned {
    fill: var(--kind-cdn-stroke);
    color: var(--kind-cdn-stroke);
  }

  .mx-fill-region-down {
    fill: var(--kind-lb-stroke);
    color: var(--kind-lb-stroke);
  }

  /* Added in this port -- see the comment above the .mx-fill-error block. */
  .mx-fill-conn-refused {
    fill: var(--kind-websocket-stroke);
    color: var(--kind-websocket-stroke);
  }

  .mx-fill-unauthorized {
    fill: var(--kind-apigateway-stroke);
    color: var(--kind-apigateway-stroke);
  }

  .mx-fill-bulkhead-full {
    fill: var(--kind-bulkhead-stroke);
    color: var(--kind-bulkhead-stroke);
  }

  .mx-fill-acquire-timeout {
    fill: var(--kind-bulkhead-stroke);
    color: var(--kind-bulkhead-stroke);
  }

  .mx-fill-deprioritized {
    fill: var(--kind-loadshedder-stroke);
    color: var(--kind-loadshedder-stroke);
  }

  /* --------------------------------------------------------------------
     Legend
     -------------------------------------------------------------------- */

  .mx-legend {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1) var(--sp-4);
    margin: var(--sp-2) 0 0;
    padding: 0;
    min-width: 0;
  }

  .mx-key {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    min-width: 0;
  }

  .mx-key-line {
    flex: none;
    width: 14px;
    height: 0;
    border-top: 2px solid var(--line-3);
  }

  .mx-key-p50 {
    border-top-color: var(--line-3);
  }

  .mx-key-p95 {
    border-top-color: var(--line-4);
  }

  .mx-key-p99 {
    border-top-color: var(--text-dim);
  }

  .mx-key-offered {
    border-top-style: dashed;
    border-top-color: var(--line-3);
  }

  .mx-key-goodput {
    border-top-color: var(--accent);
  }

  /* The dropped key is an area, not a line, so its sample is a filled band
     rather than a rule -- matching how it is actually drawn on the chart. */
  .mx-key-gap {
    flex: none;
    width: 14px;
    height: 8px;
    border-radius: var(--r-mark);
    background: var(--danger-mark);
    opacity: 0.28;
  }

  .mx-key-name {
    min-width: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mx-key-value {
    flex: none;
    color: var(--text);
    font-weight: var(--fw-num);
  }

  /* A legend row for a series currently at zero. Dimmed rather than
     removed, so the row does not appear and disappear under the cursor,
     but clearly inactive so it is never mistaken for live data. */
  .mx-key[data-dim] .mx-key-name,
  .mx-key[data-dim] .mx-key-value {
    color: var(--text-faint);
    font-weight: var(--fw-body);
  }

  .mx-key[data-dim] .mx-key-gap {
    opacity: 0.16;
  }

  .mx-dropped {
    display: flex;
    align-items: baseline;
    gap: var(--sp-1);
  }

  .mx-dropped .label {
    color: var(--text-faint);
  }

  /* --------------------------------------------------------------------
     Failure legend -- a ranked table, one row per active reason
     -------------------------------------------------------------------- */

  .mx-legend-stack {
    flex-direction: column;
    flex-wrap: nowrap;
    gap: var(--sp-1);
    max-height: 76px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--line) transparent;
  }

  .mx-key-fail {
    --swatch: 8px;
    display: grid;
    grid-template-columns: var(--swatch) minmax(0, 1fr) 40px auto 38px;
    align-items: center;
    gap: var(--sp-2);
    width: 100%;
    min-height: 20px;
  }

  .mx-key-swatch {
    width: var(--swatch);
    height: var(--swatch);
    border-radius: var(--r-mark);
    background: currentColor;
  }

  .mx-key-bar {
    display: block;
    width: 100%;
    height: 4px;
    border-radius: var(--r-pill);
    background: var(--border-strong);
    overflow: hidden;
  }

  .mx-key-bar-fill {
    display: block;
    min-width: 2px;
    height: 100%;
    border-radius: var(--r-pill);
    background: currentColor;
  }

  .mx-key-fail .mx-key-value {
    text-align: right;
  }

  .mx-key-share {
    flex: none;
    text-align: right;
    color: var(--text-faint);
    font-weight: var(--fw-body);
  }

  /* The healthy case: one calm line, not fifteen rows of zeros. */
  .mx-clean {
    margin-top: var(--sp-2);
    min-height: 18px;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-faint);
  }

  /* --------------------------------------------------------------------
     Responsive. Container queries against .app-stage, not media queries:
     .mx lives inside the stage, which is the viewport minus both rails, so
     a viewport query cannot see the width the charts actually get.
     -------------------------------------------------------------------- */

  @container stage (max-width: 840px) {
    .mx {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }

    .mx-fail-col {
      grid-column: 1 / -1;
    }
  }

  @container stage (max-width: 560px) {
    .mx {
      grid-template-columns: minmax(0, 1fr);
      gap: var(--sp-4);
    }
  }
</style>
