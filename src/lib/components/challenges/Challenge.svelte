<script lang="ts">
  /**
   * The active brief, and how the design is doing against it.
   *
   * Ported from `src/components/Challenge.tsx` (the source file's own name
   * for the export was `ChallengePanel`; this file is named `Challenge.svelte`
   * per this pass's Build list). Sits over the canvas rather than in a
   * dialog, because the whole point is to read the requirement and watch the
   * numbers move at the same time.
   *
   * Prop-driven, matching the original exactly, since the orchestration
   * (which challenge is active, calling `challengeStart`/`evaluateChallenge`
   * from `$lib/api/challenges` on every snapshot, persisting the choice) is
   * a shell-level concern outside this pass's six files -- whatever mounts
   * this component owns that wiring, the same way `App.tsx` did for the
   * original `ChallengePanel`.
   *
   * Every goal is shown at all times, met or not, with the number that
   * decided it: "failed" alone tells a reader nothing they can act on,
   * "p99 is 324ms and it needs to be under 200" tells them where to look.
   */
  import type { Challenge, ChallengeResult } from '$lib/api/challenges';

  interface Props {
    challenge: Challenge;
    result: ChallengeResult;
    onGiveUp: () => void;
  }

  let { challenge, result, onGiveUp }: Props = $props();

  const LABEL: Record<string, string> = {
    p99: 'p99 latency',
    p95: 'p95 latency',
    errorRate: 'Requests lost',
  };

  /** Metrics carry different units; a bare number is not a measurement. */
  function format(metric: string, value: number): string {
    if (metric === 'errorRate') return `${value.toFixed(1)}%`;
    return `${Math.round(value)}ms`;
  }

  /**
   * Hints are asked for, never volunteered. A panel that opens with three
   * paragraphs of help has answered a question nobody asked yet, and a
   * reader skips them. Asking is also the honest signal that someone is
   * stuck, which is the moment help is worth anything.
   */
  let shown = $state<string[]>([]);

  // Starting a different brief starts from no hints.
  $effect(() => {
    challenge.id;
    shown = [];
  });
</script>

<section class="chal{result.passed ? ' is-passed' : ''}" aria-label="Challenge">
  <header class="chal-head">
    <p class="label chal-kicker">Challenge</p>
    <h2 class="chal-name">{challenge.name}</h2>
  </header>

  <p class="chal-brief">{challenge.brief}</p>

  <ul class="chal-goals">
    {#each result.goals as g (g.goal.metric)}
      <li class="chal-goal{g.met ? ' is-met' : ''}">
        <span class="chal-goal-name">{LABEL[g.goal.metric] ?? g.goal.metric}</span>
        <span class="num chal-goal-actual">{format(g.goal.metric, g.actual)}</span>
        <span class="label chal-goal-target">needs {format(g.goal.metric, g.goal.max)} or less</span>
      </li>
    {/each}
  </ul>

  {#if result.passed}
    <p class="chal-verdict">Passed. The design meets every condition.</p>
    <!-- The explanation arrives only now. Shown from the start it is the
         answer, and the brief stops being one; withheld until the reader has
         already worked it out, it is the difference between passing and
         understanding, which is the whole point of the exercise. -->
    <p class="chal-lesson">{challenge.lesson}</p>
  {:else}
    <div class="chal-help">
      {#each shown as text, i (i)}
        <p class="chal-hint">{text}</p>
      {/each}
      {#if shown.length < challenge.hints.length}
        <button
          type="button"
          class="btn btn-sm btn-ghost chal-more"
          onclick={() => (shown = challenge.hints.slice(0, shown.length + 1))}
        >
          {shown.length === 0 ? 'Give me a hint' : 'Another hint'}
        </button>
      {/if}
    </div>
  {/if}

  <button type="button" class="btn btn-sm chal-exit" onclick={onGiveUp}>
    {result.passed ? 'Done' : 'Leave the challenge'}
  </button>
</section>

<style>
  /* Ported from src/components/Challenge.css.

     Floats over the canvas at the top left, in the space the components
     rail occupies when it is open, because a brief and the diagram it is
     about have to be readable at the same time. */

  .chal {
    position: absolute;
    top: var(--bar-clear, var(--sp-10));
    /* Right, not left: the components rail lives on the left and is open by
       default, and solving a brief means reaching for it. */
    right: var(--sp-3);
    z-index: 20;
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    width: 320px;
    max-width: calc(100vw - var(--sp-3) * 2);
    padding: var(--sp-4);
    border: var(--bw) solid var(--border);
    border-radius: var(--r-lg);
    background: var(--surface);
    box-shadow: var(--shadow-md);
  }

  /* Passing is the one state that earns colour on the frame -- the answer to
     the question the panel asks, visible without reading a word. */
  .chal.is-passed {
    border-color: var(--ok);
  }

  .chal-head {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chal-kicker {
    color: var(--text-faint);
  }

  .chal-name {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-med);
    line-height: 1.2;
  }

  .chal-brief {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: var(--lh-prose);
  }

  .chal-goals {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* A goal is a row of three: what is measured, what it reads now, and what
     it has to reach. The target stays visible after a goal is met, so the
     reader can see how much room they have rather than only that they are
     past it. */
  .chal-goal {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0 var(--sp-3);
    align-items: baseline;
    padding: var(--sp-2) var(--sp-3);
    border-radius: var(--r-sm);
    background: var(--surface-2);
  }

  .chal-goal-name {
    font-size: var(--fs-sm);
  }

  .chal-goal-actual {
    font-size: var(--fs-base);
    font-weight: var(--fw-num);
    /* Unmet is the loud state, not met: a reader scanning the panel is
       looking for what is still wrong. */
    color: var(--danger);
  }

  .chal-goal.is-met .chal-goal-actual {
    color: var(--ok);
  }

  .chal-goal-target {
    grid-column: 1 / -1;
    color: var(--text-faint);
  }

  .chal-verdict {
    margin: 0;
    color: var(--ok);
    font-size: var(--fs-sm);
  }

  /* Hints and the button that asks for them share one block, so the panel
     does not change height as each arrives. */
  .chal-help {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding-top: var(--sp-3);
    border-top: var(--bw) solid var(--border);
  }

  .chal-hint {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: var(--lh-prose);
  }

  /* Each hint after the first is stepped in, so the order they were asked
     for is visible rather than reading as one paragraph that grew. */
  .chal-hint + .chal-hint {
    padding-left: var(--sp-3);
    border-left: var(--bw) solid var(--border);
  }

  .chal-more {
    align-self: flex-start;
  }

  /* The explanation, once it is earned. Given the ok tone the passing frame
     already uses, so the panel reads as one state rather than two. */
  .chal-lesson {
    margin: 0;
    padding-top: var(--sp-3);
    border-top: var(--bw) solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-sm);
    line-height: var(--lh-prose);
  }

  .chal-exit {
    align-self: flex-start;
  }

  /* Selecting a component opens the inspector in this same corner, and the
     inspector is what the reader selected the component to use. The brief
     steps aside rather than fighting it. */
  :global(.app-body.has-inspector) .chal {
    right: calc(var(--ins-w, 320px) + var(--sp-3) * 2);
  }

  /* On a phone the panel is the whole width and sits under the header. */
  @media (max-width: 720px) {
    .chal {
      left: var(--sp-3);
      right: var(--sp-3);
      width: auto;
    }

    :global(.app-body.has-inspector) .chal {
      right: var(--sp-3);
    }
  }
</style>
