<script module lang="ts">
  /* ==========================================================================
     Explanation tooltips. Ported from `src/components/Tooltip.tsx` +
     `Tooltip.css`.

     THE NATIVE `title` ATTRIBUTE IS NOT GOOD ENOUGH for a teaching tool: it
     waits about a second, cannot be styled, never appears for keyboard
     users, and does nothing on a touch screen. Every one of those matters
     when the entire point is that a confused student can find out what a
     word means.

     TRIGGER API -- READ THIS FIRST.

     There is no `<Term>` wrapper component here. The web app's `<Term>` was
     already "a plain span with event handlers, no state, no effects, no
     refs" specifically to stay cheap on a tree that re-renders at 10Hz
     (AGENTS.md: "never `setState` per frame"); a Svelte ACTION is the more
     idiomatic way to get exactly that shape, so the whole trigger surface
     is one function attached with `use:`, not a component:

         <span use:tooltip={{ id: 'p99' }}>1200 ms</span>

     `id` is a glossary entry id (the same ids `KIND_TERM` in
     `canvas/geometry.ts` produces, e.g. `'load-balancer'`). It is resolved
     against the module-level glossary registry populated by `setGlossary()`
     -- see the TODO below; until that is wired, an `id`-only trigger
     resolves to no entry and stays completely inert (no underline, no
     listeners), the same silent degrade `<Term>` gives an unknown id.

     A caller that already has its own text and does not want to wait on
     that wiring can pass content directly instead:

         <span use:tooltip={{ entry: { term: 'p99 latency', short: '...', why: '...' } }}>

     This path does not touch the glossary registry at all, so it works
     today. Both forms accept `bare: true` to drop the dotted underline
     while keeping the hover/focus/touch behaviour (mirrors `<Term bare>`,
     used where the row itself already reads as a label -- see
     `palette/Palette.svelte`'s "?" marks for a real example of both the
     action and `bare`).

     Mount `<Tooltip />` (this file's default export) exactly ONCE, near the
     root of the app shell. It renders the single floating panel -- there is
     only ever one open at a time -- and portals it to `document.body` (a
     small local `portal` action, not a new dependency) so no scrolling
     panel can clip it.

     ARCHITECTURE. State lives in this module's top-level `$state`, shared by
     every component that imports it (Svelte gives module-scope state one
     instance for the whole app, which is exactly the external-store
     behaviour the original built with `useSyncExternalStore`). A trigger's
     `$effect` attaches/detaches its listeners reactively as
     `settingsStore.tooltips` and the glossary registry change, so turning
     the preference on live does not require remounting anything.

     GLOSSARY INTEGRATION -- CURRENTLY STUBBED.

     `src/content/glossary.ts` (990 lines) is ported as DATA to
     `src-tauri/data/glossary.json` per MIGRATION_PLAN.md Â§4, read
     from the frontend rather than bundled into it. As of this file there is
     no `src-tauri/src/commands/` module at all, so there is no
     `glossary_lookup` / `glossary_list` command yet for `$lib/api` to wrap.
     `setGlossary()` below is the intended seam: once that command exists,
     call it once at startup (e.g. from the root layout) and pass the
     result to `setGlossary(entries)`; every `id`-based trigger already
     wired throughout the app will then start resolving without any other
     change. Until then `glossaryById` stays empty and `id`-based triggers
     are inert -- the `entry`-inline path above is unaffected and is how a
     panel-porting agent can ship a working hover explanation today.
     ========================================================================== */

  import { settingsStore } from '$lib/state/settings.svelte';
  import { SvelteMap } from 'svelte/reactivity';

  /* ---- timing, ported unchanged from Tooltip.tsx --------------------- */

  /** Delay before opening, so sweeping across the UI does not strobe tooltips. */
  const OPEN_DELAY_MS = 400;
  /**
   * Once one tooltip has been open, its neighbours open immediately.
   * Re-waiting four tenths of a second per term makes comparing two numbers
   * feel broken, which is precisely the thing a student does most here.
   */
  const GRACE_MS = 500;
  /**
   * Delay before closing. Long enough that the pointer can cross the gap
   * into the panel to reach a "see also" link, short enough not to feel sticky.
   */
  const CLOSE_DELAY_MS = 140;

  /* ---- geometry, ported unchanged from Tooltip.tsx -------------------- */

  /** Keep the panel clear of the viewport edge by this much. */
  const VIEWPORT_MARGIN = 8;
  /** Gap between the trigger and the panel, leaving room for the arrow. */
  const OFFSET = 10;
  /** Half the arrow square's side. Published to CSS as `--tip-arrow`. */
  const ARROW = 6;
  /** The arrow never rides closer than this to a panel corner. */
  const ARROW_INSET = 16;
  /** Must match `max-width` on `.tip` below. */
  const PANEL_MAX_W = 300;

  /** Exposed for a future `Tooltip.place.test.ts`, mirroring `TOOLTIP_GEOMETRY`. */
  export const TOOLTIP_GEOMETRY = {
    VIEWPORT_MARGIN,
    OFFSET,
    ARROW,
    ARROW_INSET,
    PANEL_MAX_W,
  } as const;

  type Side = 'top' | 'bottom' | 'left' | 'right';

  interface Placement {
    left: number;
    top: number;
    side: Side;
    /** Arrow offset along the panel's cross axis, in panel-local px. */
    arrowOffset: number;
  }

  /**
   * Chooses a side and a position that keep the whole panel on screen.
   * Ported unchanged from Tooltip.tsx's `place()` -- see that file's
   * comments for why vertical is tried before horizontal and why both axes
   * are clamped unconditionally. Exported (as `computeTooltipPlacement`)
   * purely so a future test can exercise it directly without going through
   * the DOM, matching `__placeForTest` in the source.
   */
  export function computeTooltipPlacement(t: DOMRect, pw: number, ph: number): Placement {
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    const fitsAbove = t.top - OFFSET - ph >= VIEWPORT_MARGIN;
    const fitsBelow = t.bottom + OFFSET + ph <= vh - VIEWPORT_MARGIN;
    const fitsRight = t.right + OFFSET + pw <= vw - VIEWPORT_MARGIN;
    const fitsLeft = t.left - OFFSET - pw >= VIEWPORT_MARGIN;

    let side: Side;
    if (fitsAbove) side = 'top';
    else if (fitsBelow) side = 'bottom';
    else if (fitsLeft) side = 'left';
    else if (fitsRight) side = 'right';
    else side = t.top >= vh - t.bottom ? 'top' : 'bottom';

    const maxLeft = Math.max(VIEWPORT_MARGIN, vw - pw - VIEWPORT_MARGIN);
    const maxTop = Math.max(VIEWPORT_MARGIN, vh - ph - VIEWPORT_MARGIN);
    const clampX = (x: number) => Math.min(Math.max(x, VIEWPORT_MARGIN), maxLeft);
    const clampY = (y: number) => Math.min(Math.max(y, VIEWPORT_MARGIN), maxTop);

    const cx = t.left + t.width / 2;
    const cy = t.top + t.height / 2;

    let left: number;
    let top: number;
    if (side === 'top' || side === 'bottom') {
      left = clampX(cx - pw / 2);
      top = side === 'top' ? t.top - OFFSET - ph : t.bottom + OFFSET;
    } else {
      left = side === 'left' ? t.left - OFFSET - pw : t.right + OFFSET;
      top = cy - ph / 2;
    }

    left = clampX(left);
    top = clampY(top);

    const along = side === 'top' || side === 'bottom' ? cx - left : cy - top;
    const span = side === 'top' || side === 'bottom' ? pw : ph;
    const arrowOffset =
      along < ARROW_INSET || along > span - ARROW_INSET ? -1 : Math.round(along);

    return { left: Math.round(left), top: Math.round(top), side, arrowOffset };
  }

  /* ---- glossary registry (stubbed -- see file header) ----------------- */

  export interface GlossaryEntryLike {
    id: string;
    term: string;
    short: string;
    why: string;
    see?: string[];
  }

  // TODO(integration): populate via a `glossary_lookup` / `glossary_list`
  // Tauri command once `src-tauri/src/commands/` exists and exposes
  // one. `SvelteMap` so every open trigger reacts automatically the moment
  // this is filled in, with no other change anywhere.
  const glossaryById = new SvelteMap<string, GlossaryEntryLike>();

  /** Replace the whole registry. Call once, after loading real glossary data. */
  export function setGlossary(entries: readonly GlossaryEntryLike[]): void {
    glossaryById.clear();
    for (const e of entries) glossaryById.set(e.id, e);
  }

  /**
   * Where "see also" links go. The shell registers the glossary panel's
   * opener here once, mirroring `setGlossaryNavigate` in the source. Pass
   * null to unregister.
   */
  let navigateHandler: ((id: string) => void) | null = $state(null);
  export function setGlossaryNavigate(fn: ((id: string) => void) | null): void {
    navigateHandler = fn;
  }

  /* ---- controller ------------------------------------------------------
     A module-level `$state` singleton, shared by every trigger and by the
     one <Tooltip/> layer, replacing the source's `useSyncExternalStore`
     controller with Svelte's own fine-grained reactivity. */

  interface OpenState {
    entry: GlossaryEntryLike;
    trigger: HTMLElement;
    /**
     * How it was opened. Keyboard and touch openings are "sticky": they
     * ignore pointerleave, because there is no pointer to leave. Only an
     * explicit dismissal closes them.
     */
    source: 'hover' | 'focus' | 'touch';
  }

  let open: OpenState | null = $state(null);
  /** The live panel element, so blur can ask whether focus moved into it. */
  let panelEl: HTMLDivElement | null = $state(null);

  let openTimer: number | undefined;
  let closeTimer: number | undefined;
  let lastCloseAt = 0;
  /** Set while the pointer is inside the open panel itself. */
  let pointerInPanel = false;

  function clearTimers(): void {
    if (openTimer !== undefined) window.clearTimeout(openTimer);
    if (closeTimer !== undefined) window.clearTimeout(closeTimer);
    openTimer = undefined;
    closeTimer = undefined;
  }

  /*
   * The engaged state is written straight onto the trigger's DOM node
   * rather than derived reactively, exactly as the source does: marking
   * the one open trigger must not force every other trigger to re-evaluate.
   */
  function markTrigger(el: HTMLElement | undefined, on: boolean): void {
    if (!el) return;
    if (on) el.dataset.open = 'true';
    else delete el.dataset.open;
  }

  function commitOpen(next: OpenState): void {
    clearTimers();
    if (open?.trigger === next.trigger && open.source === next.source) return;
    markTrigger(open?.trigger, false);
    open = next;
    markTrigger(next.trigger, true);
  }

  function closeNow(): void {
    clearTimers();
    if (!open) return;
    markTrigger(open.trigger, false);
    open = null;
    pointerInPanel = false;
    lastCloseAt = Date.now();
  }

  /**
   * Closes and returns focus to whichever trigger was open, so Escape from
   * a tooltip does not strand the keyboard user at the top of the document.
   */
  function dismissAndRestoreFocus(): void {
    const trigger = open?.trigger;
    closeNow();
    trigger?.focus();
  }

  function scheduleOpen(next: OpenState): void {
    clearTimers();
    // Inside the grace window the reader is already reading tooltips, so
    // making them wait again would just feel unresponsive.
    const delay = Date.now() - lastCloseAt < GRACE_MS ? 0 : OPEN_DELAY_MS;
    if (delay === 0) {
      commitOpen(next);
      return;
    }
    openTimer = window.setTimeout(() => {
      openTimer = undefined;
      commitOpen(next);
    }, delay);
  }

  function scheduleClose(): void {
    clearTimers();
    closeTimer = window.setTimeout(() => {
      closeTimer = undefined;
      // The pointer may have landed in the panel during the grace period.
      if (pointerInPanel) return;
      closeNow();
    }, CLOSE_DELAY_MS);
  }

  /** Cancels a pending open without disturbing an already-open tooltip. */
  function cancelPendingOpen(): void {
    if (openTimer !== undefined) {
      window.clearTimeout(openTimer);
      openTimer = undefined;
    }
  }

  /**
   * Force any open tooltip shut. For the shell to call when it opens a
   * dialog or the glossary panel, so a tooltip is never left floating over
   * a surface that has just taken over the screen.
   */
  export function closeTooltip(): void {
    closeNow();
  }

  function descId(id: string): string {
    return `term-desc-${id}`;
  }

  /* ---- trigger action ---------------------------------------------- */

  export interface TooltipParams {
    /**
     * Glossary entry id. Resolved against the registry above; unresolved
     * (including "not wired yet") degrades to fully inert, same as an
     * unknown id in the source `<Term>`.
     */
    id?: string;
    /**
     * Inline content, used instead of a glossary lookup. Takes priority
     * over `id` when both are given. This is what lets a trigger work
     * before the glossary command exists.
     */
    entry?: { term: string; short: string; why: string; see?: string[] };
    /** Drop the dotted underline while keeping hover/focus/touch behaviour. */
    bare?: boolean;
  }

  function paramsEqual(a: TooltipParams, b: TooltipParams): boolean {
    if (a.id !== b.id || a.bare !== b.bare) return false;
    if (a.entry === b.entry) return true;
    if (!a.entry || !b.entry) return false;
    return a.entry.term === b.entry.term && a.entry.short === b.entry.short && a.entry.why === b.entry.why;
  }

  let idSeq = 0;

  /**
   * Svelte action: `use:tooltip={{ id: '...' }}` or `use:tooltip={{ entry: {...} }}`.
   * See the file header comment for the full API and rationale.
   */
  export function tooltip(node: HTMLElement, initial: TooltipParams = {}) {
    const instanceId = `t${(idSeq++).toString(36)}`;
    let current = $state(initial);
    let inlineDescEl: HTMLElement | null = null;

    function resolvedEntry(): GlossaryEntryLike | undefined {
      if (current.entry) return { id: current.id ?? `inline-${instanceId}`, ...current.entry };
      if (current.id) return glossaryById.get(current.id);
      return undefined;
    }

    function removeInlineDesc(): void {
      inlineDescEl?.remove();
      inlineDescEl = null;
    }

    /**
     * Every `id`-based trigger for the same glossary entry shares one
     * hidden description node (several terms on a screen may share an id,
     * e.g. p99 in the inspector and in the metrics strip, and duplicate
     * element ids are invalid). An `entry`-inline trigger has no shared
     * home in the registry, so it gets its own local hidden node instead.
     */
    function attachDescribedBy(en: GlossaryEntryLike): void {
      if (current.id) {
        node.setAttribute('aria-describedby', descId(current.id));
        return;
      }
      const id = `term-desc-inline-${instanceId}`;
      const span = document.createElement('span');
      span.className = 'sr-only';
      span.id = id;
      span.textContent = `${en.term}. ${en.short}. Press Enter for the full explanation.`;
      node.insertAdjacentElement('afterend', span);
      inlineDescEl = span;
      node.setAttribute('aria-describedby', id);
    }

    function onPointerEnter(e: PointerEvent): void {
      // A touch tap also emits pointerenter; the click handler owns touch
      // so the tooltip toggles rather than sticking open behind the finger.
      if (e.pointerType === 'touch') return;
      const en = resolvedEntry();
      if (!en) return;
      scheduleOpen({ entry: en, trigger: node, source: 'hover' });
    }
    function onPointerLeave(e: PointerEvent): void {
      if (e.pointerType === 'touch') return;
      cancelPendingOpen();
      // A tooltip opened by keyboard or tap is not dismissed by a stray
      // pointer wandering off the trigger.
      if (open && open.trigger === node && open.source !== 'hover') return;
      scheduleClose();
    }
    function onFocus(): void {
      const en = resolvedEntry();
      if (!en) return;
      // Keyboard focus is deliberate, so it opens immediately.
      commitOpen({ entry: en, trigger: node, source: 'focus' });
    }
    function onBlur(e: FocusEvent): void {
      cancelPendingOpen();
      // Focus moving INTO the panel (to reach a "see also" link) must not
      // close the thing being reached.
      const next = e.relatedTarget as Node | null;
      if (next && panelEl?.contains(next)) return;
      if (open && open.trigger === node) closeNow();
    }
    /**
     * Touch has no hover, so a tap is the only gesture available: tap to
     * read, tap again to dismiss. `click` fires for mouse, touch, pen and
     * the keyboard's Enter/Space activation alike.
     */
    function onClick(e: MouseEvent): void {
      const en = resolvedEntry();
      if (!en) return;
      if (open && open.trigger === node) {
        closeNow();
        return;
      }
      // `detail === 0` means the click was synthesised by the keyboard
      // rather than produced by a pointer, so it keeps the sticky 'focus' source.
      commitOpen({ entry: en, trigger: node, source: e.detail === 0 ? 'focus' : 'touch' });
    }
    function onKeyDown(e: KeyboardEvent): void {
      if (e.key !== 'Enter' && e.key !== ' ') return;
      e.preventDefault(); // Space would scroll the nearest scroll container.
      const en = resolvedEntry();
      if (!en) return;
      if (open && open.trigger === node) closeNow();
      else commitOpen({ entry: en, trigger: node, source: 'focus' });
    }

    /*
     * Reactive attach/detach: re-evaluates whenever `settingsStore.tooltips`
     * is toggled, the glossary registry gains the entry this trigger wants
     * (once wired), or `current` changes via `update()`. Fully inert
     * (no class, no listeners, no aria) whenever there is no enabled entry
     * to show, mirroring `<Term>` returning bare children in the source.
     */
    $effect(() => {
      const enabled = settingsStore.tooltips;
      const en = enabled ? resolvedEntry() : undefined;
      if (!en) return;
      node.classList.add('term');
      node.classList.toggle('term-bare', !!current.bare);
      node.tabIndex = 0;
      attachDescribedBy(en);
      node.addEventListener('pointerenter', onPointerEnter);
      node.addEventListener('pointerleave', onPointerLeave);
      node.addEventListener('focus', onFocus);
      node.addEventListener('blur', onBlur);
      node.addEventListener('click', onClick);
      node.addEventListener('keydown', onKeyDown);
      return () => {
        node.classList.remove('term', 'term-bare');
        node.removeAttribute('aria-describedby');
        removeInlineDesc();
        node.removeEventListener('pointerenter', onPointerEnter);
        node.removeEventListener('pointerleave', onPointerLeave);
        node.removeEventListener('focus', onFocus);
        node.removeEventListener('blur', onBlur);
        node.removeEventListener('click', onClick);
        node.removeEventListener('keydown', onKeyDown);
        if (open && open.trigger === node) closeNow();
      };
    });

    return {
      update(next: TooltipParams) {
        if (paramsEqual(current, next)) return;
        current = next;
      },
    };
  }

  /** Moves `node` to `document.body` on mount, removes it on destroy. No
   *  new dependency: this is what the source's `createPortal(..., document.body)`
   *  becomes without React portals. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        node.remove();
      },
    };
  }
</script>

<script lang="ts">
  let placement: Placement | null = $state(null);

  /*
   * Measure and position whenever `open` or the panel element changes.
   * `visibility: hidden` until `placement` is set (below) is the belt to
   * this effect's suspenders: it does not matter whether Svelte flushes
   * this before or after the browser's next paint, the panel is never
   * visible at the wrong coordinates.
   */
  $effect(() => {
    if (!open) {
      placement = null;
      return;
    }
    const panel = panelEl;
    if (!panel) return;
    placement = computeTooltipPlacement(
      open.trigger.getBoundingClientRect(),
      panel.offsetWidth,
      panel.offsetHeight,
    );
  });

  /*
   * Global dismissals. Escape closes and restores focus. Scrolling or
   * resizing dismisses rather than chasing the trigger. A pointerdown
   * anywhere else closes too, which is what gives touch users a way out --
   * they have no Escape key and no pointerleave. Capture phase throughout:
   * these must win over anything that stops propagation on its way up
   * (the canvas does exactly that, per AGENTS.md).
   */
  $effect(() => {
    if (!open) return;

    function onKey(e: KeyboardEvent): void {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      e.preventDefault();
      dismissAndRestoreFocus();
    }
    function onPointerDown(e: PointerEvent): void {
      const target = e.target as Node | null;
      if (!target) return;
      if (panelEl?.contains(target)) return;
      if (open && open.trigger.contains(target)) return; // the trigger handles itself
      closeNow();
    }
    function onDismiss(): void {
      closeNow();
    }

    document.addEventListener('keydown', onKey, true);
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('scroll', onDismiss, true);
    window.addEventListener('resize', onDismiss);
    return () => {
      document.removeEventListener('keydown', onKey, true);
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('scroll', onDismiss, true);
      window.removeEventListener('resize', onDismiss);
    };
  });

  /* Never leave a tooltip behind if the layer itself unmounts. */
  $effect(() => clearTimers);

  const seeList = $derived.by(() => {
    if (!open) return [] as GlossaryEntryLike[];
    const ids = open.entry.see ?? [];
    const out: GlossaryEntryLike[] = [];
    for (const id of ids) {
      const e = glossaryById.get(id);
      if (e) out.push(e);
    }
    return out;
  });

  /** One hidden description per registered glossary entry -- see file header. */
  const descriptions = $derived([...glossaryById.values()]);
</script>

<div class="sr-only" aria-hidden="false">
  {#each descriptions as e (e.id)}
    <span id={descId(e.id)}>{e.term}. {e.short}. Press Enter for the full explanation.</span>
  {/each}
</div>

{#if open}
  <div
    use:portal
    bind:this={panelEl}
    role="tooltip"
    class="tip tip-{placement?.side ?? 'top'}"
    style="left:{placement?.left ?? 0}px; top:{placement?.top ?? 0}px; visibility:{placement
      ? 'visible'
      : 'hidden'}; max-width:{PANEL_MAX_W}px; --tip-arrow:{ARROW}px"
    onpointerenter={() => {
      pointerInPanel = true;
      clearTimers();
    }}
    onpointerleave={() => {
      pointerInPanel = false;
      if (open?.source === 'hover') scheduleClose();
    }}
  >
    {#if placement && placement.arrowOffset >= 0}
      <span
        class="tip-arrow"
        aria-hidden="true"
        style={placement.side === 'top' || placement.side === 'bottom'
          ? `left:${placement.arrowOffset}px`
          : `top:${placement.arrowOffset}px`}
      ></span>
    {/if}

    <p class="tip-term">{open.entry.term}</p>
    <p class="tip-short">{open.entry.short}</p>
    <p class="tip-why">{open.entry.why}</p>

    {#if seeList.length > 0 && navigateHandler}
      <p class="tip-see">
        <span class="tip-see-label">See also</span>
        {#each seeList as other (other.id)}
          <button
            type="button"
            class="tip-see-link"
            onclick={() => {
              const go = navigateHandler;
              closeNow();
              go?.(other.id);
            }}
          >
            {other.term}
          </button>
        {/each}
      </p>
    {/if}
  </div>
{/if}

<style>
  /* --------------------------------------------------------------------
     Trigger. `:global` because these classes are applied by the `tooltip`
     action to elements that live in OTHER components' templates, never in
     this component's own -- Svelte's scoping would otherwise strip them.
     -------------------------------------------------------------------- */

  :global(.term) {
    /* --line-3, not --border-strong: measured at 3.27:1 / 3.11:1 / 3.00:1
       across the surfaces a term can sit on, clearing the 3:1 WCAG 1.4.11
       floor for a graphical object required to understand the interface. */
    text-decoration: underline dotted var(--line-3);
    text-underline-offset: 3px;
    text-decoration-thickness: 1px;
    cursor: help;
    border-radius: var(--r-sm);
    transition:
      text-decoration-color var(--dur-fast) var(--ease),
      background-color var(--dur-fast) var(--ease);
  }

  :global(.term:hover),
  :global(.term[data-open='true']) {
    text-decoration-color: var(--accent);
    background: var(--accent-soft);
  }

  :global(.term:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
    background: var(--accent-soft);
  }

  /* For places where the surrounding element already reads as a label and
     a second underline is only noise (e.g. Palette's "?" marks). Still
     focusable, still explains. */
  :global(.term-bare) {
    text-decoration: none;
  }

  @media (prefers-reduced-motion: reduce) {
    :global(.term) {
      transition: none;
    }
  }

  /* --------------------------------------------------------------------
     Panel. Portalled to <body> (see the `portal` action) and fixed, so no
     panel's own overflow can clip it.
     -------------------------------------------------------------------- */

  .tip {
    position: fixed;
    z-index: 400;
    width: max-content;
    max-width: 300px;
    padding: var(--sp-3);
    background: var(--surface);
    border: var(--bw) solid var(--border-strong);
    border-radius: var(--r-btn);
    box-shadow: var(--shadow-lg);
    pointer-events: auto;
    overscroll-behavior: contain;
  }

  .tip-arrow {
    position: absolute;
    width: calc(var(--tip-arrow) * 2);
    height: calc(var(--tip-arrow) * 2);
    background: var(--surface);
    border: var(--bw) solid var(--border-strong);
    border-top: none;
    border-left: none;
  }

  .tip-top .tip-arrow {
    bottom: calc(var(--tip-arrow) * -1);
    transform: translateX(-50%) rotate(45deg);
  }
  .tip-bottom .tip-arrow {
    top: calc(var(--tip-arrow) * -1);
    transform: translateX(-50%) rotate(225deg);
  }
  .tip-left .tip-arrow {
    right: calc(var(--tip-arrow) * -1);
    transform: translateY(-50%) rotate(-45deg);
  }
  .tip-right .tip-arrow {
    left: calc(var(--tip-arrow) * -1);
    transform: translateY(-50%) rotate(135deg);
  }

  .tip-term {
    margin: 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-med);
    line-height: var(--lh-sm);
    color: var(--text);
  }

  .tip-short {
    margin: var(--sp-1) 0 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-body);
    line-height: var(--lh-sm);
    color: var(--text-dim);
    text-wrap: pretty;
  }

  .tip-why {
    margin: var(--sp-2) 0 0;
    font-size: var(--fs-sm);
    font-weight: var(--fw-body);
    line-height: var(--lh-prose);
    color: var(--text);
    text-wrap: pretty;
  }

  .tip-see {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--sp-1) var(--sp-2);
    margin: var(--sp-3) 0 0;
    padding-top: var(--sp-2);
    border-top: var(--bw) solid var(--border);
  }

  .tip-see-label {
    font-size: var(--fs-label);
    font-weight: var(--fw-med);
    line-height: var(--lh-label);
    letter-spacing: var(--tr-label);
    text-transform: uppercase;
    color: var(--text-faint);
  }

  .tip-see-link {
    padding: 0;
    border: 0;
    background: none;
    font-family: inherit;
    font-size: var(--fs-sm);
    font-weight: var(--fw-body);
    line-height: var(--lh-sm);
    color: var(--accent-ink);
    text-decoration: underline;
    text-underline-offset: 2px;
    border-radius: var(--r-sm);
    cursor: pointer;
    transition: color var(--dur-fast) var(--ease);
  }

  .tip-see-link:hover {
    color: var(--accent);
  }

  .tip-see-link:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }

  @media (prefers-reduced-motion: reduce) {
    .tip-see-link {
      transition: none;
    }
  }
</style>
