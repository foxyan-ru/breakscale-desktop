<script lang="ts">
  /* ==========================================================================
     TrafficControl -- the load slider and the readouts around it.

     Port of `src/components/Inspector.tsx`'s `TrafficControl` (lines
     2945-3260) plus its styles from `src/components/Inspector.css`
     (476-927). It never existed on the desktop: the header had two text
     buttons ("Play/Pause/Reset") where the web app has this control, so the
     ONE input a student drives -- offered load -- and the numbers that show
     what it did (p99, goodput, errors, dropped) were simply absent, and
     `Shortcuts.svelte` documented an `S` step key nothing handled.

     PROPS, not stores. Every value crosses as a prop and every action as a
     callback, so this file stays a pure view: `+page.svelte` derives
     `rps` / `system` / `lost` / `empty` / `noTrafficSource` from the stores
     and owns the write paths (the web app does the same -- the component is
     a function of its props there too). It also makes the whole control
     renderable in jsdom with no Tauri bridge at all, which is what the
     regression tests in `__tests__/traffic-control.test.ts` rely on.

     SLIDER MATH. The range is 0..SLIDER_STEPS mapped exponentially onto
     1..5000 rps: a linear track would spend 90% of its travel above 500rps
     where nothing new happens, and the interesting knee is at the low end.
     `positionToRps` rounds to something a student can read and reproduce
     (1 / 5 / 10 / 50 by magnitude); `rpsToPosition` clamps and inverts it.
     Both are transcribed verbatim from the web component.

     COMMIT TIMING. Dragging writes a local draft only -- the readout and the
     fill track the pointer, and nothing leaves the webview -- and `change`
     (pointer-up) is what calls `onRpsChange`, which in `+page.svelte` writes
     through to the Rust engine. Same contract as every slider in
     `inspector/Inspector.svelte`: one IPC per gesture rather than one per
     pixel of travel (MIGRATION_PLAN explicitly says live-drag timing need
     not match the web app's).

     ICONS. The web app uses lucide-react; this project has no icon
     dependency (package.json ships only `@tauri-apps/api` and
     `plugin-dialog`), so the four glyphs are the same lucide paths drawn
     inline -- the repo's established convention for every other button in
     `routes/+page.svelte`.
     ========================================================================== */

  import { tooltip } from './Tooltip.svelte';
  import {
    formatMs,
    formatPct,
    formatRate,
    formatRateBare,
    healthOfErr,
    healthOfLatency,
    toneClass,
  } from '$lib/components/canvas/format';
  import type { SystemStats } from '$lib/domain';

  interface Props {
    /** Total offered load, summed from the client nodes' `config.rps`. */
    rps: number;
    onRpsChange: (rps: number) => void;
    running: boolean;
    onToggleRun: () => void;
    onStep: () => void;
    onReset: () => void;
    system: SystemStats;
    /** Requests actually lost per second, from the engine's per-reason counters. */
    lost: number;
    /** No components at all: nothing to measure, so say that instead of "100% errors". */
    empty: boolean;
    /**
     * No client on the canvas, so nothing is offering load.
     *
     * Separate from `empty`: a design can hold a database and a cache and
     * still have no traffic source, and the slider is inert there too. It
     * writes `rps` onto client nodes, so with none to write to it would
     * silently snap back -- which reads as a broken control rather than an
     * inapplicable one, hence the disabled state and the sentence that
     * explains it.
     */
    noTrafficSource: boolean;
  }

  let {
    rps,
    onRpsChange,
    running,
    onToggleRun,
    onStep,
    onReset,
    system,
    lost,
    empty,
    noTrafficSource,
  }: Props = $props();

  /* ------------------------------------------------------------------ *
   * Slider geometry -- verbatim from the web component.
   * ------------------------------------------------------------------ */

  const RPS_MIN = 1;
  const RPS_MAX = 5000;
  const LOG_MIN = Math.log(RPS_MIN);
  const LOG_MAX = Math.log(RPS_MAX);
  /** Slider travel in discrete steps. Finer than 1-per-rps at the low end. */
  const SLIDER_STEPS = 1000;

  /** Slider position (0..SLIDER_STEPS) -> real requests per second. */
  function positionToRps(pos: number): number {
    const t = pos / SLIDER_STEPS;
    const rpsValue = Math.exp(LOG_MIN + t * (LOG_MAX - LOG_MIN));
    // Round to something a student can actually read and reproduce.
    if (rpsValue < 20) return Math.round(rpsValue);
    if (rpsValue < 200) return Math.round(rpsValue / 5) * 5;
    if (rpsValue < 1000) return Math.round(rpsValue / 10) * 10;
    return Math.round(rpsValue / 50) * 50;
  }

  /** Real requests per second -> slider position (0..SLIDER_STEPS). */
  function rpsToPosition(value: number): number {
    const safe = Number.isFinite(value) ? value : RPS_MIN;
    const clamped = Math.min(RPS_MAX, Math.max(RPS_MIN, safe));
    const t = (Math.log(clamped) - LOG_MIN) / (LOG_MAX - LOG_MIN);
    return Math.round(t * SLIDER_STEPS);
  }

  /** Linear fill for the track's `--fill-pct`, in the 0..SLIDER_STEPS space. */
  function fillPct(value: number, min: number, max: number): string {
    const span = max - min;
    if (!Number.isFinite(span) || span <= 0) return '0%';
    const v = Number.isFinite(value) ? value : min;
    const t = (v - min) / span;
    return `${Math.max(0, Math.min(1, t)) * 100}%`;
  }

  /**
   * Scale marks, placed at their true log position rather than spread
   * evenly: an evenly-spaced 1/10/100/1k/5k row lies about where the thumb
   * will land (1k sits at 69% of the travel, not 75%).
   */
  const SCALE_MARKS = [1, 10, 100, 1000, 5000];

  /* The web component generates this with `useId()`; one static id is
     equivalent here because exactly one of these mounts per page, and a
     static value keeps the `<label for>` / `<input id>` pair resolvable in
     a source-reading test (precedent: `ins-*` ids in Inspector.svelte). */
  const sliderId = 'traffic-rps';

  /* ------------------------------------------------------------------ *
   * Drag draft. See COMMIT TIMING above.
   * ------------------------------------------------------------------ */

  /** The slider's position while the pointer is down, else null. */
  let sliderDraft: number | null = $state(null);

  const shownPos = $derived(sliderDraft ?? rpsToPosition(rps));

  /** What the readout shows: live during a drag, the committed value after. */
  const readoutRps = $derived(sliderDraft !== null ? positionToRps(sliderDraft) : rps);

  const fill = $derived(fillPct(shownPos, 0, SLIDER_STEPS));

  function onSliderInput(e: Event): void {
    sliderDraft = Number((e.currentTarget as HTMLInputElement).value);
  }

  function onSliderChange(e: Event): void {
    const next = positionToRps(Number((e.currentTarget as HTMLInputElement).value));
    // Cleared before the callback so both this write and the parent's
    // `rps` land in the same flush: the thumb moves once, to where the new
    // value sits, instead of snapping back and then forward.
    sliderDraft = null;
    onRpsChange(next);
  }

  /* ------------------------------------------------------------------ *
   * Readouts.
   * ------------------------------------------------------------------ */

  const errTone = $derived(toneClass(healthOfErr(system.errorRate)));

  // Traffic that actually FAILED, while it is failing. Deliberately not
  // `offered - goodput`: that gap is mostly in-flight work and is largest
  // during warm-up, so using it made a healthy system flash a red
  // "Dropped 10/s" next to an error rate of exactly zero. `errorRate` is
  // the engine's own fraction of requests that errored or were shed.
  const dropped = $derived(Number.isFinite(lost) ? Math.max(0, lost) : 0);
  const showDropped = $derived(dropped > 0.5);

  /**
   * p99 of WHAT? With zero goodput nothing is completing, so a latency
   * percentile does not exist -- and "0ms" beside "Errors 100%" read as
   * "instantly fast" in the middle of a total outage. No data renders as
   * the sentinel, never as a fake zero.
   */
  const p99 = $derived(system.p99 === 0 && system.goodputRps === 0 ? null : system.p99);
  const p99Tone = $derived(toneClass(healthOfLatency(p99)));

  const p99Class = $derived(p99Tone ? `num num-hero ${p99Tone}` : 'num num-hero');
  const errClass = $derived(errTone ? `num num-md ${errTone}` : 'num num-md');

  const runLabel = $derived(running ? 'Pause' : 'Play');
</script>

<div class="traffic">
  <!-- Cause. The only thing on screen the student directly controls, and
       therefore the only input that gets a hero-sized number. -->
  <div class="traffic-load">
    <div class="traffic-load-head">
      <!-- The <label> keeps its `for`, so clicking the word still focuses
           the slider; the tooltip trigger only wraps the text inside it.
           Nesting the trigger the other way round would make the tooltip's
           own click target steal the label's activation. -->
      <label class="label" for={sliderId}>
        <span use:tooltip={{ id: 'offered' }}>Offered load</span>
      </label>
      <span class="traffic-load-readout">
        {#if noTrafficSource}
          <!-- Not "0". With no client there is no offered load to report,
               and a confident zero in the app's darkest ink outranks the
               faint note under the slider that explains why. Saying what is
               missing puts the reason where the eye already is. -->
          <span class="traffic-load-none">No traffic source</span>
        {:else}
          <!-- num-lg, not the hero. The slider's own value is feedback the
               student just caused; p99 is the consequence they are meant to
               watch, and it reads as the most important number on screen
               only if it is the ONLY number at the hero size. -->
          <span class="num num-lg">{formatRateBare(readoutRps)}</span>
          <span class="label traffic-load-unit">
            <span use:tooltip={{ id: 'rps' }}>requests / sec</span>
          </span>
        {/if}
      </span>
    </div>
    <div class="traffic-load-track">
      <input
        id={sliderId}
        class="slider"
        type="range"
        min="0"
        max={SLIDER_STEPS}
        step="1"
        value={shownPos}
        style="--fill-pct: {fill}"
        disabled={noTrafficSource}
        aria-valuetext={noTrafficSource
          ? 'No traffic source on the canvas'
          : `${Math.round(readoutRps)} request${Math.round(readoutRps) === 1 ? '' : 's'} per second`}
        oninput={onSliderInput}
        onchange={onSliderChange}
      />
      {#if noTrafficSource}
        <!-- The scale describes a range the slider cannot reach, so it is
             replaced by the reason rather than sitting under a dead
             control. Says what to do, not what went wrong. -->
        <p class="traffic-scale-note">Add a client to send traffic.</p>
      {:else}
        <div class="traffic-scale" aria-hidden="true">
          {#each SCALE_MARKS as mark (mark)}
            <span
              class="num traffic-scale-mark"
              style="left: {(rpsToPosition(mark) / SLIDER_STEPS) * 100}%">{mark >= 1000 ? `${mark / 1000}k` : mark}</span
            >
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <div class="traffic-spacer"></div>

  <!-- Effect. p99 is the number students should watch, so it carries the
       only other hero, toned by the same threshold as the chart line.

       Every metric slot is ALWAYS rendered, including "Dropped" at zero.
       It used to mount only once dropped traffic appeared, which meant
       Goodput and Errors jumped sideways at the exact moment a system
       started failing -- motion arriving precisely when the student needs
       to read the numbers, not watch them move. -->
  <div class="traffic-headline">
    {#if empty}
      <!-- No components: nothing to measure. One quiet sentence, in the
           same slot the numbers occupy, rather than a row of red zeroes. -->
      <p class="traffic-empty">Nothing to measure yet. Add a component to get started.</p>
    {:else}
      <div class="traffic-metric">
        <span class="label">System <span use:tooltip={{ id: 'p99' }}>p99</span></span>
        <span class={p99Class}>{formatMs(p99)}</span>
      </div>

      <div class="traffic-metric-group">
        <div class="traffic-metric">
          <span class="label"><span use:tooltip={{ id: 'goodput' }}>Goodput</span></span>
          <span class="num num-md">{formatRate(system.goodputRps)}</span>
        </div>
        <div class="traffic-metric">
          <span class="label"><span use:tooltip={{ id: 'error-rate' }}>Errors</span></span>
          <span class={errClass}>{formatPct(system.errorRate)}</span>
        </div>
        <div class="traffic-metric">
          <span class="label"><span use:tooltip={{ id: 'dropped' }}>Dropped</span></span>
          <span class="num num-md" class:is-danger={showDropped}>{formatRate(dropped)}</span>
        </div>
      </div>
    {/if}
  </div>

  <!-- The transport. Icon-only, the settled convention for these three
       actions everywhere from video players to debuggers; the words move
       into aria-label and the title, where the shortcut is printed beside
       the action the same way the undo pair prints Ctrl+Z.

       role="group" with a name, so a screen reader announces "Simulation
       transport" once and then three short labels, instead of three
       orphaned buttons. -->
  <div class="traffic-actions" role="group" aria-label="Simulation transport">
    <button
      type="button"
      class="btn btn-icon transport-toggle"
      onclick={onToggleRun}
      aria-label={runLabel}
      title="{runLabel} (Space)"
    >
      <!-- No aria-pressed. The label itself flips Pause <-> Play, so the
           button already states its effect; a pressed state made a running
           simulation announce "Pause, pressed", which reads as ALREADY
           paused. Both glyphs are filled: that is how a transport states
           "this is the go/stop switch", and the fill is what marks it
           primary now that no icon sits in a coloured box. -->
      {#if running}
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="currentColor"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <rect x="6" y="4" width="4" height="16" rx="1" />
          <rect x="14" y="4" width="4" height="16" rx="1" />
        </svg>
      {:else}
        <!-- A triangle's visual mass sits left of its bounding-box centre;
             the class nudges it right so play and pause land on the same
             optical centre and the swap does not flicker. -->
        <svg
          class="transport-play"
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="currentColor"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <polygon points="6 3 20 12 6 21 6 3" />
        </svg>
      {/if}
    </button>
    <!-- Stepping is only meaningful against a stopped clock, and it pauses
         on its own, so the tooltip states what it will do. -->
    <button type="button" class="btn btn-icon" onclick={onStep} aria-label="Step one tick" title="Step one tick (S)">
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <line x1="6" x2="4" y1="3" y2="21" />
        <polygon points="20 3 20 21 8 12 20 3" fill="currentColor" />
      </svg>
    </button>
    <!-- No shortcut: reset throws away the run's numbers, and an action
         with a cost is exactly the one that should require the pointer. -->
    <button
      type="button"
      class="btn btn-icon"
      onclick={onReset}
      aria-label="Reset simulation"
      title="Reset simulation"
    >
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
        <path d="M3 3v5h5" />
      </svg>
    </button>
  </div>
</div>

<style>
  /* Ported wholesale from the web app's Inspector.css (476-927). The
     traffic control is the top bar's whole middle island here, so these
     are the island's own layout rules rather than a strip inside a
     different card; primitives (.btn, .num, .label, .slider) come from
     `src/app.css` and are deliberately not restated. */

  .traffic {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    min-width: 0;

    /* This control is a CONTAINER, and everything it sheds as space runs
       out is keyed on ITS width, not the viewport's: the width it gets is
       the island minus nothing, and the island changes with the window at
       its own breakpoints. Inline-size containment is also what makes the
       degradation sound -- the readouts inside can never push the island
       wider than the window, so the transport can never be painted over. */
    container-type: inline-size;
    container-name: traffic;
  }

  /* ------------------------------------------------------------------
     Offered load -- the cause
     ------------------------------------------------------------------ */

  /* The readout sits BESIDE the track rather than stacked above it. The
     number and the control that changes it then read as one line, instead
     of sitting at opposite ends of a stack -- and the island stays short
     enough to leave the canvas the height it deserves. */
  .traffic-load {
    display: flex;
    flex: 0 1 420px;
    align-items: center;
    gap: var(--sp-4);
    min-width: 280px;
  }

  /* The caption and the figure, stacked tight on the left of the control. */
  .traffic-load-head {
    display: flex;
    flex: none;
    flex-direction: column;
    align-items: flex-start;
    gap: 0;
  }

  .traffic-load-head :global(.label) {
    color: var(--text-faint);
    line-height: var(--lh-label);
    white-space: nowrap;
  }

  /* Baseline alignment, so the unit sits on the numeral's baseline rather
     than being centred against the hero's box. */
  .traffic-load-readout {
    display: flex;
    align-items: baseline;
    gap: var(--sp-1);
  }

  /* Fixed minimum width for four glyphs at the 22px step, so the slider
     beside it does not resize as the value steps between "9", "370" and
     "5k". */
  .traffic-load-readout :global(.num-lg) {
    min-width: 52px;
  }

  .traffic-load-unit {
    color: var(--text-faint);
    text-transform: none;
    letter-spacing: 0;
  }

  .traffic-load-track {
    position: relative;
    flex: 1 1 auto;
    min-width: 0;
    /* Track height plus the scale row beneath it. Fixed, so the island's
       height never changes as the value does. */
    padding-bottom: 12px;
  }

  /* Stands in for the scale row when there is no traffic source. Same slot,
     same height, so the control does not change size as a client is added
     or removed. */
  .traffic-scale-note {
    position: absolute;
    inset: auto 0 0;
    height: 12px;
    margin: 0;
    /* --text-dim, not --text-faint. The faint tone is for text a reader
       may ignore; this one is the answer to "why will the slider not
       move", and it was losing the page to a bold 0 directly above it. */
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: 12px;
    white-space: nowrap;
  }

  /* The scale marks sit at their TRUE log positions, which is why they are
     absolutely positioned rather than spread with space-between: 1k lands
     at 69% of the travel, and an evenly-spaced row would lie about where
     the thumb will go. */
  .traffic-scale {
    position: absolute;
    inset: auto 0 0;
    height: 12px;
    /* The thumb's centre can only reach half its own width from either
       end. Insetting the mark rail by that much makes a mark sit under the
       thumb position it names instead of drifting outward at the extremes.
       Derived from the token so the two cannot drift apart. */
    margin: 0 calc(var(--thumb) / 2);
    pointer-events: none;
  }

  .traffic-scale-mark {
    position: absolute;
    top: 0;
    /* Centres the label on its own position. */
    transform: translateX(-50%);
    font-size: var(--fs-label);
    font-weight: var(--fw-body);
    line-height: 1;
    letter-spacing: 0;
    color: var(--text-faint);
    white-space: nowrap;
  }

  /* Stands where the load figure would be when there is nothing to report.
     Sized and coloured as text rather than as a number: it is a sentence,
     and setting it in the mono figure stack would make "No traffic source"
     read as a value the simulator had measured. */
  .traffic-load-none {
    color: var(--text-dim);
    font-size: var(--fs-base);
    font-weight: var(--fw-med);
    white-space: nowrap;
  }

  /* Whitespace between the control half of the island and the readout
     half. */
  .traffic-spacer {
    flex: 1 1 auto;
    min-width: var(--sp-4);
  }

  /* ------------------------------------------------------------------
     Readouts -- the effect
     ------------------------------------------------------------------ */

  /* The hero and the supporting set.

     p99 is deliberately larger than Goodput / Errors / Dropped: it is the
     one number a student should be watching, and the size IS that
     instruction.

     The two tiers are aligned on their LABELS rather than on `baseline`:
     at 34px and 16px a baseline alignment puts their text baselines 13.8px
     apart and makes the eye drop and jump reading left to right. Every
     eyebrow now sits on one line and the values hang below at their own
     sizes. */
  .traffic-headline {
    display: flex;
    flex: none;
    align-items: flex-start;
    gap: var(--sp-7);
    min-width: 0;
  }

  /* No components on the canvas: one quiet sentence where the numbers go. */
  .traffic-empty {
    margin: 0;
    align-self: center;
    max-width: 46ch;
    font-size: var(--fs-base);
    line-height: var(--lh-base);
    color: var(--text-faint);
    text-wrap: pretty;
  }

  .traffic-metric {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    /* Right-aligned so each figure's last digit sits on a fixed axis: a
       value going from 99 to 100 must not shove its own label sideways. */
    align-items: flex-end;
    min-width: 0;
  }

  .traffic-metric :global(.label) {
    color: var(--text-faint);
    white-space: nowrap;
  }

  /* The secondary readouts. Grouped so they read as a set beside the hero
     rather than as three more competing headlines. */
  .traffic-metric-group {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-5);
  }

  /* ------------------------------------------------------------------
     The transport, as ONE segmented cluster.

     Three flush buttons sharing hairlines, not three outlined boxes in a
     row: the shared edge is what makes play/step/reset read as a single
     control the way a media player's transport does. The negative margin
     collapses each pair of 1px borders into one, and the radius survives
     only on the cluster's outer corners. */
  .traffic-actions {
    display: flex;
    flex: none;
    align-items: center;
  }

  .traffic-actions :global(.btn) {
    border-radius: 0;
  }

  .traffic-actions :global(.btn + .btn) {
    margin-left: calc(-1 * var(--bw));
  }

  .traffic-actions :global(.btn:first-child) {
    border-top-left-radius: var(--r-btn);
    border-bottom-left-radius: var(--r-btn);
  }

  .traffic-actions :global(.btn:last-child) {
    border-top-right-radius: var(--r-btn);
    border-bottom-right-radius: var(--r-btn);
  }

  /* Collapsed borders are shared borders, so the hovered or focused button
     must climb above its neighbours or its highlighted edge and focus ring
     would be half-eaten by the button drawn after it. */
  .traffic-actions :global(.btn:hover),
  .traffic-actions :global(.btn:active),
  .traffic-actions :global(.btn:focus-visible) {
    position: relative;
    z-index: 1;
  }

  /* Play/pause is the primary action, and it says so by size and by ink:
     a wider box than its siblings and the accent on the glyph itself. NOT
     by a filled accent box: an icon in a coloured rounded square is on the
     ban list, and the fill would also fight p99's claim to be the loudest
     thing in the island.

     A width, not a min-width: the box must hold still while the glyph
     inside it swaps between play and pause. */
  .traffic-actions :global(.transport-toggle) {
    width: 48px;
    color: var(--accent);
  }

  /* The optical-centre nudge for the play triangle (see the markup). */
  .transport-play {
    transform: translateX(1px);
  }

  /* ----------------------------------------------------------------------
     Narrow traffic control.

     Container queries against the control's own width (the container is
     declared on .traffic above), because what these readouts compete with
     is the slider BESIDE them, not the window: a wide monitor and a narrow
     laptop with the window already stripped hand this control very
     different widths at the same window size. Nothing here may wrap: the
     island grows if it does, and it is meant to stay one line tall.

     The thresholds are set by measurement, not taste. The control's pieces
     at full size: load 420 + spacer 16 + p99 98 + secondary readouts 228 +
     transport 118 + three 20px gaps, about 940px.

     What gives way, in order:
       860  the secondary readouts. Past that, keeping three more figures
            would crush the one control the student drives. The charts
            strip still carries every one of these numbers.
       620  the slider's floor comes down to 260.
       560  the "requests / sec" unit and the spacer's guarantee go, and the
            slider may reach 220. The slider's aria-valuetext still says
            the unit.

     Never dropped: the load slider, the p99 hero, the transport.
     ---------------------------------------------------------------------- */

  @container traffic (max-width: 860px) {
    .traffic-metric-group {
      display: none;
    }

    .traffic-headline {
      gap: var(--sp-4);
    }
  }

  @container traffic (max-width: 620px) {
    .traffic-load {
      min-width: 260px;
    }
  }

  @container traffic (max-width: 560px) {
    .traffic-load {
      min-width: 220px;
    }

    .traffic-load-unit {
      display: none;
    }

    .traffic-spacer {
      min-width: 0;
    }
  }

  /* The one thing the container cannot style is its own gap (a container
     query cannot match the container it measures against). Purely spacing,
     so a coarse window step is honest here. */
  @media (max-width: 1000px) {
    .traffic {
      gap: var(--sp-4);
    }
  }
</style>
