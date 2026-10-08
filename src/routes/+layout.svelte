<script lang="ts">
  /* ==========================================================================
     Root layout.

     Owns exactly the things that are global to the whole app and must exist
     exactly once, no matter which route is showing (today there is only
     one, `+page.svelte`, but this is where that contract lives):

       - The two global stylesheets (`app.css`'s tokens/primitives,
         `shell/shell.css`'s app-shell grid), imported here as side effects
         so every component's scoped styles can rely on the tokens and
         primitives being there.
       - Starting the theme reaction (`startTheme`), from `onMount` only --
         `window.matchMedia` does not exist during the prerender build step
         (see `+layout.ts`).

         NOTE: the `sim://snapshot` event subscription (`startListening`)
         used to start here too, alongside `glossaryList()` below. A
         shipped build showed the canvas never animating; attaching
         DevTools to the installed app (WebView2's
         `--remote-debugging-port`) showed the Rust tick thread running
         correctly but the frontend never once calling
         `plugin:event|listen` -- this root layout's `onMount` silently
         never ran its async continuations in that build, for reasons not
         yet understood (not a thrown exception -- none fires -- just a
         mount hook whose promise-returning body never executes, while
         every sibling/child component's `onMount` fires exactly as
         written). `startListening()` was moved to `+page.svelte`'s
         `onMount`, which IS confirmed reliable (every `invoke()` in it
         shows up over the wire every time). `glossaryList()` below has
         not been moved and may share the same bug -- flagged, not yet
         fixed; the symptom would be every tooltip reading "No term
         matches" and the Glossary panel never populating.
       - Loading the glossary once and handing it to `Tooltip.svelte`'s
         module-level registry (`setGlossary`), so every `use:tooltip={{id}}`
         trigger anywhere in the app starts resolving.
       - Mounting `<Tooltip />` itself: a singleton floating layer, per its
         own header comment ("mount exactly ONCE, near the root").
       - Mounting `<Glossary />` alongside it. This is a deliberate departure
         from the obvious reading of the brief (which lists Glossary among
         `+page.svelte`'s overlay panels): `Tooltip.svelte`'s "see also"
         links call `setGlossaryNavigate`, a module-level hook registered
         once, and the one piece of state that hook needs to drive --- which
         entry to land on and highlight --- has nowhere to live if Glossary
         is owned by the page instead. `uiStore.activeView` (shared, real
         global state) still decides whether the panel is open either way;
         only the "jump to this term" wiring is what moved up here, next to
         the other thing (Tooltip) that depends on it existing at the root.
       - The one global error surface: every store's fire-and-forget IPC
         sync routes a failure into `uiStore.pushError`, and nothing else in
         this codebase ever reads that queue -- so if nothing renders it
         here, every sync failure vanishes into an array nobody looks at.
     ========================================================================== */

  import '../app.css';
  import '$lib/components/shell/shell.css';

  import { onMount } from 'svelte';
  import { startTheme } from '$lib/state/theme.svelte';
  import { uiStore, dismissError, pushError } from '$lib/state/ui.svelte';
  import { isAppError } from '$lib/api';
  import { glossaryList } from '$lib/api/glossary';
  import Tooltip, { setGlossary, setGlossaryNavigate } from '$lib/components/shell/Tooltip.svelte';
  import Glossary from '$lib/components/glossary/Glossary.svelte';
  import type { GlossaryEntry } from '$lib/components/glossary/Glossary.svelte';

  let { children } = $props();

  /** Every glossary entry, once `glossary_list` resolves. Empty until then --
   *  Glossary.svelte's own `loading` prop covers that gap explicitly. */
  let glossaryEntries: GlossaryEntry[] = $state([]);
  let glossaryLoading = $state(true);
  /** Which entry a tooltip's "see also" link (or the glossary button's own
   *  shortcut) should land on when the panel opens. */
  let glossaryFocusId: string | undefined = $state(undefined);

  function describeErr(e: unknown): string {
    if (isAppError(e)) return e.message;
    return e instanceof Error ? e.message : String(e);
  }

  function openGlossary(focusId?: string): void {
    uiStore.activeView = 'glossary';
    glossaryFocusId = focusId;
  }

  function closeGlossary(): void {
    uiStore.activeView = 'canvas';
  }

  onMount(() => {
    const stopTheme = startTheme();

    void glossaryList()
      .then((entries) => {
        glossaryEntries = entries;
        setGlossary(entries);
      })
      .catch((e) => {
        // Glossary content is a teaching aid, never load-bearing: per
        // Tooltip.svelte's own documented degrade, a missing/failed
        // glossary_list just leaves every `use:tooltip={{id}}` trigger inert
        // and this panel showing "No term matches" for everything -- not a
        // crash. Still surfaced once, so a real backend problem is visible
        // rather than silently swallowed.
        pushError(`Loading the glossary failed: ${describeErr(e)}`);
      })
      .finally(() => {
        glossaryLoading = false;
      });

    setGlossaryNavigate((id) => openGlossary(id));

    return () => {
      stopTheme();
      setGlossaryNavigate(null);
    };
  });
</script>

<Tooltip />

<Glossary
  open={uiStore.activeView === 'glossary'}
  onClose={closeGlossary}
  entries={glossaryEntries}
  loading={glossaryLoading}
  focusId={glossaryFocusId}
/>

{@render children()}

{#if uiStore.errors.length > 0}
  <div class="toast-stack" role="region" aria-label="Errors">
    {#each uiStore.errors as err (err.id)}
      <button
        type="button"
        class="app-toast app-toast-error"
        onclick={() => dismissError(err.id)}
        title="Dismiss"
      >
        {err.message}
      </button>
    {/each}
  </div>
{/if}

<style>
  /* Stacks however many errors are queued, newest at the bottom, reusing
     shell.css's already-styled `.app-toast`/`.app-toast-error` look for each
     individual message. `.app-toast` is `position: fixed` in shell.css
     (correct for a single receipt that owns the whole screen's bottom-centre
     spot); with more than one queued that would stack every toast on the
     exact same pixels, so each one is pinned to `static` here and the
     POSITIONING moves to this wrapper instead. */
  .toast-stack {
    position: fixed;
    bottom: var(--sp-5);
    left: 50%;
    transform: translateX(-50%);
    z-index: 300;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--sp-2);
    max-width: calc(100vw - var(--sp-6));
  }

  .toast-stack :global(.app-toast) {
    position: static;
    transform: none;
    animation: none;
    border: none;
    cursor: pointer;
    font: inherit;
  }
</style>
