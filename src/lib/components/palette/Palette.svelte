<script module lang="ts">
  /**
   * The two annotation-tool kinds Palette exposes alongside components.
   * Mirrors `export type AnnotationTool` in `src/components/Palette.tsx`,
   * which `src/App.tsx` imports as `import type { AnnotationTool } from
   * './components/Palette'` -- the same pattern works here:
   * `import type { AnnotationTool } from '$lib/components/palette/Palette.svelte'`.
   */
  export type AnnotationTool = 'note' | 'section';

  /**
   * MIME type set on dragstart for a dragged annotation tool (Note /
   * Section), read back by Canvas.svelte's drop handler.
   *
   * NOT part of the canvas geometry contract: `canvas/geometry.ts` only
   * exports `NODE_DND_MIME`, for components. This is Palette's own
   * hand-off, matching `ANN_DND_MIME` in the web app's
   * `src/components/annotationLayout.ts`. Whoever wires Canvas.svelte's
   * drop handler for annotations needs this exact string -- import it from
   * here, or hardcode the same literal.
   */
  export const ANN_DND_MIME = 'application/x-breakscale-annotation';

  /* ------------------------------------------------------------------ *
   * The pickup -- ported from upstream commit `dc9c1a07` ("carry a card,
   * not a screenshot of the row"), `src/components/Palette.tsx`.
   *
   * The browser's default drag image is a translucent screenshot of the
   * whole row, list chrome and all, which reads as dragging a menu entry.
   * The thing being carried is a component about to exist on the canvas,
   * so what follows the cursor is a small card instead: the row's own
   * chip and name on the node radius with a lift shadow.
   *
   * A native drag image is sampled ONCE at dragstart and can never
   * animate, so the card is not handed to `setDragImage`. The native
   * image is replaced with a transparent pixel and the card is a real
   * element that follows the pointer from `dragover` on the document: it
   * pops in when picked up and banks a few degrees with the horizontal
   * velocity of the hand. Cost per move is one `translate3d` write on a
   * fixed, `pointer-events: none` element -- compositor work only,
   * nothing re-renders and nothing lays out. Module scope (not component
   * state) matches upstream: a drag is a singleton gesture, the browser
   * never starts a second one before `dragend` ends the first.
   * ------------------------------------------------------------------ */

  /**
   * Transparent stand-in for the native drag image. Created once and kept
   * hot: `setDragImage` needs a DECODED image at dragstart, and a data URI
   * this size is decoded long before a human can begin a drag. Guarded for
   * SSR, where `Image` does not exist when this module is first evaluated.
   */
  const BLANK_DRAG_IMAGE = typeof Image !== 'undefined' ? new Image() : null;
  if (BLANK_DRAG_IMAGE) {
    BLANK_DRAG_IMAGE.src =
      'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';
  }

  /**
   * The card, built from the row's rendered DOM rather than re-created
   * from scratch: the glyph is cloned as-is, the name is read back as
   * text, and the `data-kind` attribute rides along so the chip keeps its
   * colour trio through the same `[data-kind]` contract every other
   * surface uses (see the file header comment on KIND COLOUR).
   */
  function buildCard(row: HTMLButtonElement): HTMLDivElement | null {
    const glyph = row.querySelector('.pal-glyph');
    const name = row.querySelector('.pal-name')?.textContent;
    if (!glyph || !name) return null;
    const card = document.createElement('div');
    card.className = 'pal-carry-card';
    const kind = row.getAttribute('data-kind');
    if (kind !== null) card.setAttribute('data-kind', kind);
    card.appendChild(glyph.cloneNode(true));
    const label = document.createElement('span');
    label.className = 'pal-carry-name';
    label.textContent = name;
    card.appendChild(label);
    return card;
  }

  // Module state for the one live preview. A drag is a singleton gesture.
  let carryEl: HTMLDivElement | null = null;
  let carryCard: HTMLDivElement | null = null;
  let carryX = 0;

  function moveCarry(e: globalThis.DragEvent): void {
    if (!carryEl) return;
    carryEl.style.transform = `translate3d(${e.clientX}px, ${e.clientY}px, 0)`;
    // Bank with the hand. The inner card owns the rotation and eases it
    // with its own transition, so the outer element can jump to the
    // pointer with no easing at all: position must never lag the cursor,
    // only the tilt may.
    const dx = e.clientX - carryX;
    carryX = e.clientX;
    if (carryCard) {
      const tilt = Math.max(-6, Math.min(6, dx * 0.5));
      carryCard.style.transform = `rotate(${tilt}deg)`;
    }
  }

  function endCarry(): void {
    document.removeEventListener('dragover', moveCarry);
    carryEl?.remove();
    carryEl = null;
    carryCard = null;
  }

  /**
   * Fallback when the blank image is somehow not ready: the same card,
   * parked off screen and sampled once by `setDragImage`. Static, but
   * still a card.
   */
  function setStaticCardImage(event: globalThis.DragEvent): void {
    const dt = event.dataTransfer;
    if (!dt) return;
    const row = event.currentTarget as HTMLButtonElement;
    const card = buildCard(row);
    if (!card) return;
    const park = document.createElement('div');
    park.className = 'pal-carry-park';
    park.appendChild(card);
    document.body.appendChild(park);
    const rect = card.getBoundingClientRect();
    dt.setDragImage(card, rect.width / 2, rect.height / 2);
    setTimeout(() => park.remove(), 0);
  }

  /** Arms the custom carry preview on dragstart. Call after `dt.setData`. */
  function startCarry(event: globalThis.DragEvent): void {
    const dt = event.dataTransfer;
    if (!dt || typeof dt.setDragImage !== 'function') return;
    if (!BLANK_DRAG_IMAGE?.complete) {
      setStaticCardImage(event);
      return;
    }
    const row = event.currentTarget as HTMLButtonElement;
    const card = buildCard(row);
    if (!card) return;
    endCarry(); // a stale preview from an interrupted drag must not linger
    dt.setDragImage(BLANK_DRAG_IMAGE, 0, 0);

    const outer = document.createElement('div');
    outer.className = 'pal-carry';
    const pop = document.createElement('div');
    pop.className = 'pal-carry-pop';
    pop.appendChild(card);
    outer.appendChild(pop);
    outer.style.transform = `translate3d(${event.clientX}px, ${event.clientY}px, 0)`;
    document.body.appendChild(outer);

    carryEl = outer;
    carryCard = card;
    carryX = event.clientX;
    document.addEventListener('dragover', moveCarry);
  }
</script>

<script lang="ts">
  /* ==========================================================================
     Palette -- the left rail. Ported from `src/components/Palette.tsx` +
     `Palette.css`.

     Only what the current Palette.tsx actually renders is ported: a
     searchable, grouped list of components (drag or click to add) and an
     "Annotate" group for the Note/Section tools. Palette.css also carries
     rules for a collapsible-section chrome and preset/keyboard-reference
     rows (`.pal-toggle`, `.pal-row-preset`, `.pal-keylist`, ...) that
     Palette.tsx no longer renders at all (its own comments say the
     Components section stopped being a disclosure); those rules are not
     ported here since there is no markup left that would use them.

     KIND COLOUR. Unlike Minimap.css, this file defines no
     `[data-kind='...']` rules: `desktop/src/app.css` already resolves any
     `[data-kind]` element's `--k-fill` / `--k-line` / `--k-stroke` /
     `--k-ink` globally for all 33 kinds ("the SOLE owner of kind ->
     colour"), and `.pal-glyph` below is a plain descendant of the row that
     carries `data-kind`, so it inherits those custom properties for free.

     VENDOR NAMES -- STUBBED. The source shows a vendor's product name
     (e.g. "Amazon RDS") under the generic name via `nameFor(kind, vendor)`
     from `src/content/vendors/lookup.ts`, backed by ~2000 lines of vendor
     spec data (`src/content/vendors/*.ts`). Per MIGRATION_PLAN.md §4 that
     data ports to `src-tauri/data/vendors/*.json`, read through a
     Tauri command; neither the data nor a lookup command exist yet
     (`src-tauri/src/commands/` has not been created). `nameFor`
     below is therefore a stub that always returns the generic name, so the
     vendor line never renders and searching by a vendor's product name
     (e.g. "RDS") does not yet match -- searching by what a kind IS or DOES
     still works exactly as before. Swap the stub for a real lookup once
     that command exists; nothing else here needs to change.
     ========================================================================== */

  import type { NodeKind } from '$lib/domain';
  import { settingsStore } from '$lib/state/settings.svelte';
  import { tooltip } from '$lib/components/shell/Tooltip.svelte';
  import {
    ICON_BOX,
    ICON_STROKE,
    KIND_GROUPS,
    KIND_ICON,
    KIND_NAME,
    KIND_TERM,
    NODE_DND_MIME,
  } from '../canvas/geometry';

  type Group = (typeof KIND_GROUPS)[number];

  /**
   * Shown as the row's `title` only. Ported verbatim from Palette.tsx.
   */
  const KIND_HINT: Record<NodeKind, string> = {
    client: 'Sends requests at the rate you set',
    lb: 'Spreads requests across several servers',
    service: 'Handles a request, calls what it needs',
    cache: 'Answers repeat reads without the database',
    db: 'Stores the data. Usually saturates first',
    queue: 'Holds work so the sender does not wait',
    worker: 'Drains the queue in the background',
    replica: 'Scales reads, but they can be stale',
    shard: 'Splits data by key. A hot key ruins it',
    autoscaler: 'Adds capacity when load rises, after a delay',
    region: 'Fails traffic over to another region',
    cdn: 'Serves most requests before they reach you',
    ratelimiter: 'Refuses excess traffic cheaply, at the door',
    breaker: 'Stops calling a dependency that is failing',
    objectstore: 'Blobs: slow per request, near-unlimited',
    searchindex: 'Fast search, but writes index late',
    timeseriesdb: 'Swallows metrics; range queries cost',
    graphdb: 'Relationships. Depth multiplies the cost',
    coldstorage: 'Archive tier: cheap, and takes seconds',
    vectordb: 'Similarity search. Recall costs latency',
    streambroker: 'A replayable log; consumers fall behind',
    pubsub: 'One publish becomes N deliveries',
    websocket: 'Holds connections; they run out, not rps',
    apigateway: 'Auth, rate limits and routing at the door',
    sidecar: 'A proxy tax on every hop, buying retries',
    lambda: 'Scales instantly, but cold starts cost',
    cron: 'Dumps a burst of work on a schedule',
    bulkhead: 'Caps calls to one dependency; contains it',
    retryqueue: 'Redelivers failures; dead-letters the rest',
    transcoder: 'Grinds long CPU jobs pulled off a queue',
    edgecompute: 'Answers what it can at the edge itself',
    writebehind: 'Acks writes fast; a crash loses the buffer',
    loadshedder: 'Under load, drops low-priority traffic first',
  };

  /** The two annotation rows. Not components: no simulation behaviour, own group. */
  const ANN_ROWS: { tool: AnnotationTool; name: string; hint: string; icon: string[] }[] = [
    {
      tool: 'note',
      name: 'Note',
      hint: 'Click, then click the canvas to place text (N)',
      icon: ['M4 7V5h16v2', 'M9 20h6', 'M12 5v15'],
    },
    {
      tool: 'section',
      name: 'Section',
      hint: 'Click, then drag on the canvas to frame a group (B)',
      icon: [
        'M3 5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z',
        'M3 9h8',
      ],
    },
  ];

  // TODO(integration): swap for a real `nameFor(kind, vendor)` port once
  // src-tauri/data/vendors/*.json + a lookup command exist. See
  // the file header comment.
  function nameFor(kind: NodeKind, _vendor: string): string {
    return KIND_NAME[kind];
  }

  function svgAttrs(attrs: Record<string, string>): Record<string, string> {
    const { key: _key, ...rest } = attrs;
    return rest;
  }

  interface Props {
    /** Add a node of `kind` to the canvas at a default position. */
    onAdd: (kind: NodeKind) => void;
    /**
     * Arm the note or section tool. The next drag on the canvas draws the
     * shape; clicking the row again disarms.
     */
    onAddAnnotation?: (tool: AnnotationTool) => void;
    /** Which tool is armed, so the row can show it. */
    armedTool?: AnnotationTool | null;
  }

  let { onAdd, onAddAnnotation, armedTool = null }: Props = $props();

  const totalKinds = KIND_GROUPS.reduce((n, g) => n + g.kinds.length, 0);

  let query = $state('');
  let searchEl: HTMLInputElement | null = $state(null);
  const needle = $derived(query.trim().toLowerCase());

  function matchesKind(kind: NodeKind, group: Group, n: string): boolean {
    return (
      KIND_NAME[kind].toLowerCase().includes(n) ||
      nameFor(kind, settingsStore.vendor).toLowerCase().includes(n) ||
      KIND_HINT[kind].toLowerCase().includes(n) ||
      group.title.toLowerCase().includes(n)
    );
  }

  /**
   * The groups with non-matching kinds removed, empty groups dropped.
   * Filtering rather than reordering keeps the taxonomy intact while
   * searching.
   */
  const groups = $derived.by(() => {
    if (!needle) return KIND_GROUPS;
    return KIND_GROUPS.map((g) => ({
      ...g,
      kinds: g.kinds.filter((k) => matchesKind(k, g, needle)),
    })).filter((g) => g.kinds.length > 0);
  });

  const matchCount = $derived(groups.reduce((n, g) => n + g.kinds.length, 0));

  function onSearchInput(e: Event): void {
    query = (e.currentTarget as HTMLInputElement).value;
  }

  /** Escape clears the box, then gives it up. */
  function onSearchKeyDown(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    e.stopPropagation();
    if (query) {
      query = '';
      return;
    }
    searchEl?.blur();
  }

  /**
   * Enter and Space both activate a row. A <button> already does this, but
   * the row is also draggable, and Firefox drops the implicit Space
   * activation on a draggable button -- restored explicitly.
   */
  function onRowKeyDown(e: KeyboardEvent, kind: NodeKind): void {
    if (e.key === ' ' || e.key === 'Spacebar') {
      e.preventDefault();
      onAdd(kind);
    }
  }

  function handleDragStart(e: DragEvent, kind: NodeKind): void {
    const dt = e.dataTransfer;
    if (!dt) return;
    // Must match what Canvas checks for in ondragover / ondrop.
    dt.setData(NODE_DND_MIME, kind);
    dt.effectAllowed = 'copy';
    // dc9c1a07: carry the row's own card, not the browser's row screenshot.
    startCarry(e);
  }

  function handleAnnDragStart(e: DragEvent, tool: AnnotationTool): void {
    const dt = e.dataTransfer;
    if (!dt) return;
    dt.setData(ANN_DND_MIME, tool);
    dt.effectAllowed = 'copy';
    // dc9c1a07: same carried-card pickup as the component rows above.
    startCarry(e);
  }

  function onAnnKeyDown(e: KeyboardEvent, tool: AnnotationTool): void {
    if (e.key === ' ' || e.key === 'Spacebar') {
      e.preventDefault();
      onAddAnnotation?.(tool);
    }
  }
</script>

{#snippet kindGlyph(kind: NodeKind)}
  <svg
    width="1.1em"
    height="1.1em"
    viewBox="0 0 {ICON_BOX} {ICON_BOX}"
    fill="none"
    stroke="currentColor"
    stroke-width={ICON_STROKE}
    stroke-linecap="round"
    stroke-linejoin="round"
    role="presentation"
    aria-hidden="true"
  >
    {#each KIND_ICON[kind] as [tag, attrs], i (i)}
      <svelte:element this={tag} {...svgAttrs(attrs)} />
    {/each}
  </svg>
{/snippet}

<nav class="pal" aria-label="Components and examples">
  <div class="pal-scroll scroll">
    <div class="pal-section">
      <p class="label pal-heading">
        Components
        <span class="pal-heading-count">{needle ? `${matchCount} of ${totalKinds}` : totalKinds}</span>
      </p>

      <div class="pal-search">
        <svg
          class="pal-search-icon"
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.2-3.2" />
        </svg>
        <input
          bind:this={searchEl}
          type="search"
          class="pal-search-input"
          placeholder="Search components"
          aria-label="Search components"
          value={query}
          oninput={onSearchInput}
          onkeydown={onSearchKeyDown}
        />
        {#if query}
          <button
            type="button"
            class="pal-search-clear"
            aria-label="Clear search"
            onclick={() => {
              query = '';
              searchEl?.focus();
            }}
          >
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              aria-hidden="true"
            >
              <path d="M18 6 6 18M6 6l12 12" />
            </svg>
          </button>
        {/if}
      </div>

      <p class="sr-only" role="status">
        {needle ? `${matchCount} of ${totalKinds} components match ${query.trim()}` : ''}
      </p>

      {#if needle && matchCount === 0}
        <p class="pal-empty">
          Nothing matches &ldquo;{query.trim()}&rdquo;. Searching what a component does works too, like
          &ldquo;stale&rdquo; or &ldquo;refuse&rdquo;.
        </p>
      {/if}

      {#each groups as group (group.id)}
        <div class="pal-group">
          <p class="label pal-group-title">{group.title}</p>
          <ul class="pal-list">
            {#each group.kinds as kind (kind)}
              <li class="pal-item">
                <button
                  type="button"
                  class="pal-row"
                  data-kind={kind}
                  draggable="true"
                  ondragstart={(e) => handleDragStart(e, kind)}
                  ondragend={endCarry}
                  onclick={() => onAdd(kind)}
                  onkeydown={(e) => onRowKeyDown(e, kind)}
                  title={KIND_HINT[kind]}
                >
                  <span class="pal-glyph">{@render kindGlyph(kind)}</span>
                  <span class="pal-names">
                    <span class="pal-name">{KIND_NAME[kind]}</span>
                    {#if nameFor(kind, settingsStore.vendor) !== KIND_NAME[kind]}
                      <span class="pal-vendor">{nameFor(kind, settingsStore.vendor)}</span>
                    {/if}
                  </span>
                </button>
                <!--
                  Rendered only when tooltips are on, matching the source: an
                  inert "?" mark with no handler and an anchorless sr-only
                  label is worse than no mark, so it does not exist in the
                  DOM at all while the preference is off (see Palette.css's
                  own postmortem comment on `.pal-explain` for why that
                  matters). `bare` because the row is already a strong
                  affordance.
                -->
                {#if settingsStore.tooltips}
                  <span class="pal-explain" use:tooltip={{ id: KIND_TERM[kind], bare: true }}>
                    <span aria-hidden="true">?</span>
                    <span class="sr-only">What is a {KIND_NAME[kind]}?</span>
                  </span>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/each}

      {#if onAddAnnotation}
        <div class="pal-group">
          <p class="label pal-group-title">Annotate</p>
          <ul class="pal-list">
            {#each ANN_ROWS as row (row.tool)}
              <li class="pal-item">
                <button
                  type="button"
                  class={armedTool === row.tool ? 'pal-row is-armed' : 'pal-row'}
                  aria-pressed={armedTool === row.tool}
                  draggable="true"
                  ondragstart={(e) => handleAnnDragStart(e, row.tool)}
                  ondragend={endCarry}
                  onclick={() => onAddAnnotation?.(row.tool)}
                  onkeydown={(e) => onAnnKeyDown(e, row.tool)}
                  title={row.hint}
                >
                  <span class="pal-glyph">
                    <svg
                      width="1.1em"
                      height="1.1em"
                      viewBox="0 0 {ICON_BOX} {ICON_BOX}"
                      fill="none"
                      stroke="currentColor"
                      stroke-width={ICON_STROKE}
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      role="presentation"
                      aria-hidden="true"
                    >
                      {#each row.icon as d (d)}
                        <path {d} />
                      {/each}
                    </svg>
                  </span>
                  <span class="pal-name">{row.name}</span>
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}
    </div>
  </div>
</nav>

<style>
  /* ==========================================================================
     Palette rail. See Palette.tsx's header comment (ported into this
     file's script block above) for why the chip carries kind colour and
     why it is a chip rather than a dot.
     ========================================================================== */

  .pal {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }

  .pal-scroll {
    flex: 1 1 auto;
    padding: var(--sp-3) 0 var(--sp-5);
    overflow-x: hidden;
  }

  .pal-group + .pal-group {
    margin-top: var(--sp-3);
  }

  .pal-heading {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--sp-3);
    padding: var(--sp-3) var(--sp-4) var(--sp-2);
    margin: 0;
    color: var(--text-dim);
  }

  .pal-heading-count {
    font-variant-numeric: tabular-nums;
    color: var(--text-faint);
  }

  .pal-search {
    position: relative;
    display: flex;
    align-items: center;
    margin: 0 var(--sp-4) var(--sp-2);
  }

  .pal-search-icon {
    position: absolute;
    left: var(--sp-2);
    color: var(--text-faint);
    pointer-events: none;
  }

  .pal-search-input {
    width: 100%;
    height: 30px;
    padding: 0 26px 0 calc(var(--sp-2) + 14px + var(--sp-2));
    border: var(--bw) solid var(--border-strong);
    border-radius: var(--r-btn);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-sm);
  }

  .pal-search-input::placeholder {
    color: var(--text-faint);
  }

  .pal-search-input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
    border-color: var(--accent);
  }

  .pal-search-input::-webkit-search-decoration,
  .pal-search-input::-webkit-search-cancel-button,
  .pal-search-input::-webkit-search-results-button {
    appearance: none;
  }

  .pal-search-clear {
    position: absolute;
    right: var(--sp-1);
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: var(--r-sm);
    background: none;
    color: var(--text-faint);
    cursor: pointer;
  }

  .pal-search-clear:hover {
    background: var(--surface-3);
    color: var(--text);
  }

  .pal-search-clear:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .pal-empty {
    padding: var(--sp-2) var(--sp-4) var(--sp-4);
    margin: 0;
    font-size: var(--fs-sm);
    line-height: var(--lh-sm);
    color: var(--text-dim);
  }

  .pal-group-title {
    padding: var(--sp-2) var(--sp-4) var(--sp-1) calc(var(--sp-4) + 22px + var(--sp-3));
    color: var(--text-faint);
  }

  .pal-list {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  /* The <li> is the flex line so the explain mark can sit beside the
     button without being inside it -- the row keeps its click, drag and
     keyboard activation exactly as if the mark were not there. */
  .pal-item {
    display: flex;
    align-items: center;
  }

  /* Quiet until the row is engaged: opacity, not display, so the names
     never shift sideways when a row is hovered. */
  .pal-explain {
    flex: none;
    display: grid;
    place-items: center;
    position: relative;
    width: 24px;
    height: 24px;
    margin-right: var(--sp-2);
    border-radius: var(--r-sm);
    color: var(--text-faint);
    font-size: var(--fs-label);
    font-weight: var(--fw-med);
    line-height: 1;
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease);
  }

  .pal-item:hover .pal-explain,
  .pal-explain:focus-visible,
  .pal-explain[data-open='true'] {
    opacity: 1;
  }

  .pal-explain:hover,
  .pal-explain[data-open='true'] {
    color: var(--accent-ink);
  }

  .pal-row {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    width: 100%;
    min-height: 40px;
    padding: 0 var(--sp-4);
    border: none;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-base);
    font-weight: var(--fw-body);
    line-height: var(--lh-base);
    letter-spacing: var(--tr-base);
    text-align: left;
    cursor: grab;
    transition: background-color var(--dur-fast) var(--ease);
  }

  @media (pointer: coarse) {
    .pal-row {
      min-height: 44px;
    }
  }

  .pal-row:hover {
    background: var(--surface-2);
  }

  .pal-row:active {
    background: var(--surface-3);
    cursor: grabbing;
  }

  .pal-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    border-radius: 0;
  }

  /* An armed annotation tool: the canvas switches to a crosshair too, but
     that is only visible where the pointer is. */
  .pal-row.is-armed {
    background: var(--accent-soft);
    color: var(--accent-ink);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .pal-row.is-armed .pal-glyph {
    color: var(--accent);
  }

  .pal-glyph {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border: var(--bw) solid var(--k-stroke, var(--border-strong));
    border-radius: var(--r-sm);
    background: var(--k-fill, var(--surface-2));
    color: var(--k-ink, var(--text-dim));
    transition: transform var(--dur-fast) var(--ease);
  }

  .pal-row:hover .pal-glyph {
    transform: scale(1.06);
  }

  /* The carry: what the pointer holds once a row is picked up. Ported from
     Palette.css (commit `dc9c1a07`), "carry a card, not a screenshot of the
     row" -- ties to `startCarry`/`moveCarry`/`endCarry` in the module script
     above. Three layers, each owning ONE transform so none of them fight:
     the outer .pal-carry jumps to the pointer with no easing at all, because
     position lagging the cursor reads as sluggishness; .pal-carry-pop owns
     centring and the pickup pop; the card itself owns the velocity tilt,
     eased by its own transition. pointer-events: none keeps it out of every
     hit test, so Canvas's drop handling never knows it exists. Everything
     here is transform and opacity -- compositor work only -- and this
     repo's blanket `*` reduced-motion rule (`app.css`'s final
     `@media (prefers-reduced-motion: reduce)` block) already flattens the
     pop and the tilt to nothing, so no local override is needed here. */
  .pal-carry {
    position: fixed;
    top: 0;
    left: 0;
    z-index: 500; /* above the tooltip layer (400, Tooltip.svelte): nothing outranks the hand */
    pointer-events: none;
    will-change: transform;
  }

  @keyframes pal-carry-pop {
    from {
      opacity: 0;
      transform: translate(-50%, -50%) scale(0.85);
    }
    to {
      opacity: 1;
      transform: translate(-50%, -50%) scale(1);
    }
  }

  .pal-carry-pop {
    transform: translate(-50%, -50%);
    animation: pal-carry-pop var(--dur-base) var(--ease-out);
  }

  /* The card: the row's chip and name on the node radius with a lift
     shadow, because the thing being carried is a component about to exist
     on the canvas, not a line of a list. Colour comes through the same
     [data-kind] contract as the chip it clones (see the file header
     comment on KIND COLOUR). */
  .pal-carry-card {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding: var(--sp-2) var(--sp-4) var(--sp-2) var(--sp-3);
    border: var(--bw) solid var(--k-stroke, var(--border-strong));
    border-radius: var(--r-node);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-lg);
    font-size: var(--fs-base);
    white-space: nowrap;
    transition: transform var(--dur-base) var(--ease);
  }

  .pal-carry-name {
    font-weight: var(--fw-med);
  }

  /* Off-screen parking for the static fallback, when the blank drag image
     is not decoded yet: the same card, sampled once by setDragImage. */
  .pal-carry-park {
    position: fixed;
    top: -1000px;
    left: -1000px;
  }

  .pal-names {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    min-width: 0;
    gap: 1px;
  }

  .pal-vendor {
    overflow: hidden;
    color: var(--text-faint);
    font-size: var(--fs-label);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pal-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    .pal-row,
    .pal-glyph,
    .pal-explain {
      transition: none;
    }
  }
</style>
