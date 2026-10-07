<script lang="ts">
  /**
   * The challenge picker.
   *
   * Ported from `src/components/Challenges.tsx`. Deliberately reuses the
   * examples dialog's shell and stylesheet rather than a third dialog
   * design -- a reader who has opened Examples already knows how this
   * behaves, and the two are the same act: pick a system to load, this one
   * arriving with a goal attached. No search box: there are four of these
   * against twenty-three examples, and a filter over four rows is furniture.
   *
   * The `.ex-*` markup/CSS below is a DELIBERATE DUPLICATE of
   * `shell/Examples.svelte`'s own dialog shell (mounted/closing lifecycle,
   * focus management, Escape handling, and the `.ex-root`/`.ex-card`/...
   * class names), not a shared import: Svelte scopes a component's style
   * to its own markup, so the plain-CSS-file sharing the original React app
   * used (`Examples.css` imported by both `Examples.tsx` and `Challenges.tsx`)
   * has no direct equivalent without either a `:global()` escape hatch or a
   * shared `Dialog.svelte` wrapper -- and `Examples.svelte` is another
   * agent's finished file, out of scope to modify here. A future pass could
   * extract both into a shared dialog-shell component; noted rather than
   * done, to stay within this pass's six files.
   */
  import type { ChallengeSummary } from '$lib/api/challenges';

  interface Props {
    open: boolean;
    onClose: () => void;
    challenges: readonly ChallengeSummary[];
    /** True while `challenges` is still being fetched from Rust. */
    loading?: boolean;
    onStart: (id: string) => void;
  }

  let { open, onClose, challenges, loading = false, onStart }: Props = $props();

  let cardEl: HTMLDivElement | undefined = $state(undefined);
  let mounted = $state(false);
  let closing = $state(false);

  $effect(() => {
    if (open) {
      closing = false;
      mounted = true;
    } else if (mounted) {
      closing = true;
    }
  });

  function handleAnimationEnd(e: AnimationEvent) {
    if (closing && e.target === e.currentTarget) {
      mounted = false;
      closing = false;
    }
  }

  /* Focus goes to the dialog card itself (there is no search field here,
     unlike Examples). Focus returns to whatever opened it. */
  $effect(() => {
    if (!open) return;
    const opener =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    cardEl?.focus();
    return () => opener?.focus();
  });

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    }
  }
</script>

{#if mounted}
  <div class={`ex-root${closing ? ' is-closing' : ''}`} inert={closing || undefined}>
    <div class="ex-scrim" onclick={onClose} aria-hidden="true"></div>
    <div
      bind:this={cardEl}
      class="ex-card"
      role="dialog"
      aria-modal="true"
      aria-labelledby="chal-title"
      tabindex="-1"
      onkeydown={onKeyDown}
      onanimationend={handleAnimationEnd}
    >
      <header class="ex-head">
        <div>
          <h2 id="chal-title" class="ex-title">Challenges</h2>
          <p class="ex-sub">A system that does not meet its requirement. Work out why, and fix it.</p>
        </div>
        <button type="button" class="btn" onclick={onClose}>Close</button>
      </header>

      {#if loading}
        <p class="ex-empty">Loading challengesâ€¦</p>
      {:else if challenges.length === 0}
        <p class="ex-empty">No challenges are available yet.</p>
      {:else}
        <div class="ex-grid">
          {#each challenges as c (c.id)}
            <button
              type="button"
              class="ex-item"
              onclick={() => {
                onStart(c.id);
                onClose();
              }}
            >
              <span class="ex-item-name">{c.name}</span>
              <span class="ex-item-desc">{c.brief}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* Duplicated from shell/Examples.svelte's own style block -- see this file's
     script-block comment for why a Svelte component cannot simply "reuse"
     another component's scoped styles by class name. */

  .ex-root {
    position: fixed;
    inset: 0;
    z-index: 500;
    display: grid;
    place-items: center;
    padding: var(--sp-5);
  }

  .ex-scrim {
    position: absolute;
    inset: 0;
    background: var(--scrim);
    cursor: default;
    animation: ex-scrim-in var(--dur-slow) var(--ease-out);
  }

  .ex-root.is-closing .ex-scrim {
    animation: ex-scrim-out var(--dur-base) var(--ease) forwards;
  }

  .ex-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    width: min(720px, 100%);
    max-height: min(600px, 100%);
    min-height: 0;
    padding: var(--sp-5);
    background: var(--surface);
    border: var(--bw) solid var(--border-strong);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-lg);
    outline: none;
    animation: ex-card-in var(--dur-slow) var(--ease-out);
  }

  .ex-root.is-closing .ex-card {
    animation: ex-card-out var(--dur-base) var(--ease) forwards;
  }

  .ex-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--sp-4);
  }

  .ex-title {
    margin: 0;
    font-size: var(--fs-lg);
    font-weight: var(--fw-med);
    line-height: var(--lh-sm);
    color: var(--text);
  }

  .ex-sub {
    margin: var(--sp-1) 0 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-body);
    line-height: var(--lh-sm);
    color: var(--text-dim);
  }

  .ex-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: var(--sp-3);
    margin: 0;
    padding: 0 var(--sp-1) var(--sp-1) 0;
    overflow-y: auto;
    min-height: 0;
    scrollbar-width: thin;
    scrollbar-color: var(--line) transparent;
    overscroll-behavior: contain;
  }

  .ex-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--sp-1);
    width: 100%;
    height: 100%;
    padding: var(--sp-3);
    text-align: left;
    background: var(--surface-2);
    border: var(--bw) solid var(--border);
    border-radius: var(--r-btn);
    cursor: pointer;
    font: inherit;
    transition:
      background-color var(--dur-fast) var(--ease),
      border-color var(--dur-fast) var(--ease);
  }

  .ex-item:hover {
    background: var(--surface-3);
    border-color: var(--border-strong);
  }

  .ex-item:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  .ex-item-name {
    font-size: var(--fs-base);
    font-weight: var(--fw-med);
    line-height: var(--lh-sm);
    color: var(--text);
  }

  .ex-item-desc {
    margin-top: var(--sp-1);
    font-size: var(--fs-sm);
    font-weight: var(--fw-body);
    line-height: var(--lh-prose);
    color: var(--text-faint);
    text-wrap: pretty;
  }

  .ex-empty {
    margin: 0;
    padding: var(--sp-5) 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-prose);
    color: var(--text-dim);
    text-align: center;
  }

  @keyframes ex-card-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes ex-card-out {
    from {
      opacity: 1;
      transform: translateY(0);
    }
    to {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  @keyframes ex-scrim-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes ex-scrim-out {
    from {
      opacity: 1;
    }
    to {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ex-card,
    .ex-scrim,
    .ex-root.is-closing .ex-card,
    .ex-root.is-closing .ex-scrim {
      animation-duration: 0.01ms;
    }

    .ex-item {
      transition: none;
    }
  }
</style>
