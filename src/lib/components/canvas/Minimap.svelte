<script lang="ts">
  /* ==========================================================================
     Minimap. Ported from `src/components/Minimap.tsx` + `Minimap.css`.

     WHY IT EXISTS. A twenty-node company architecture is several screens
     wide at a readable zoom, and the only ways to find the part you wanted
     were to zoom out until the labels were unreadable, or to pan and hope.
     This shows the whole diagram at once and says which piece of it you are
     looking at.

     DELIBERATELY NOT A SECOND RENDERER. Nodes are plain rectangles here, in
     their kind's colour, with no labels, no metrics and no edges. It is a
     map, not a thumbnail: at this size a label is illegible and an edge is
     noise, and every detail added is another thing that can disagree with
     the canvas.

     PLACEMENT. Lives beside Canvas.svelte in this directory rather than its
     own `components/minimap/` folder -- a deliberate exception to "one
     directory per panel" -- because it reads the canvas's live view
     transform and surface size on every frame the canvas re-renders.

     CONTRACT WITH Canvas.svelte. `ongoto` is a callback PROP, not a custom
     DOM event: Canvas.svelte passes `ongoto={(x, y) => ...}` and this
     component calls `ongoto(worldX, worldY)` on every click and every drag
     tick, exactly mirroring the React `onGoTo` callback prop. Whoever wires
     the two together should recentre the canvas's view transform on that
     world point (matching Minimap.tsx's contract), not wait for any kind of
     "commit" event -- the original scrubs continuously while dragging.
     ========================================================================== */
  import type { SimNode } from '$lib/domain';
  import { settingsStore } from '$lib/state/settings.svelte';
  import { NODE_W, NODE_H } from './geometry';

  interface View {
    x: number;
    y: number;
    k: number;
  }
  interface Surface {
    width: number;
    height: number;
  }

  interface Props {
    nodes: readonly SimNode[];
    /** Current viewport in world units, so the map can outline it. */
    view: View;
    /** Size of the canvas surface in screen px. */
    surface: Surface;
    /** Centre the canvas on this world point. Called continuously while dragging. */
    ongoto?: (worldX: number, worldY: number) => void;
  }

  let { nodes, view, surface, ongoto }: Props = $props();

  /** Padding, in world px, around the diagram's own bounds. */
  const PAD = 120;
  /** The map's own box; the diagram's longer side is scaled to fit this. */
  const FIT_BOX = 168;

  let boxEl: HTMLDivElement | null = $state(null);
  let dragging = false;

  /** World bounds of the whole diagram, padded so nothing touches the edge. */
  const world = $derived.by(() => {
    if (nodes.length === 0) return null;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const n of nodes) {
      if (n.x < minX) minX = n.x;
      if (n.y < minY) minY = n.y;
      if (n.x + NODE_W > maxX) maxX = n.x + NODE_W;
      if (n.y + NODE_H > maxY) maxY = n.y + NODE_H;
    }
    return {
      x: minX - PAD,
      y: minY - PAD,
      w: maxX - minX + PAD * 2,
      h: maxY - minY + PAD * 2,
    };
  });

  /* One scale for both axes, so the map is not a distorted picture of the
     diagram. The smaller of the two fits the long side. */
  const fit = $derived.by(() => {
    const w = world;
    if (!w) return null;
    const k = Math.min(FIT_BOX / w.w, FIT_BOX / w.h);
    return { k, w: w.w * k, h: w.h * k };
  });

  /** The viewport, expressed in world units then mapped like everything else. */
  const viewWorld = $derived.by(() => {
    if (!fit) return null;
    return {
      x: -view.x / view.k,
      y: -view.y / view.k,
      w: surface.width / view.k,
      h: surface.height / view.k,
    };
  });

  /**
   * The viewport rectangle, clamped to the map's own bounds.
   *
   * Zoomed out far enough, or on a small diagram, the visible area is
   * LARGER than everything there is to see, and an unclamped rectangle
   * spills past the map and out over the canvas. Clamping BOTH edges (not
   * just the near one) says the honest thing instead: panning away visibly
   * shrinks the box until it disappears, rather than it sticking to a
   * corner and looking unchanged.
   */
  const viewportRect = $derived.by(() => {
    const w = world;
    const f = fit;
    const vw = viewWorld;
    if (!w || !f || !vw) return null;
    const x0 = (vw.x - w.x) * f.k;
    const y0 = (vw.y - w.y) * f.k;
    const left = Math.min(Math.max(0, x0), f.w);
    const top = Math.min(Math.max(0, y0), f.h);
    const right = Math.min(Math.max(0, x0 + vw.w * f.k), f.w);
    const bottom = Math.min(Math.max(0, y0 + vw.h * f.k), f.h);
    return {
      left,
      top,
      width: Math.max(0, right - left),
      height: Math.max(0, bottom - top),
    };
  });

  /** Convert a click in the map back to the world point it stands for. */
  function goToEvent(e: { clientX: number; clientY: number }): void {
    const el = boxEl;
    const w = world;
    const f = fit;
    if (!el || !w || !f) return;
    const r = el.getBoundingClientRect();
    ongoto?.(w.x + (e.clientX - r.left) / f.k, w.y + (e.clientY - r.top) / f.k);
  }

  function onDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    e.preventDefault();
    dragging = true;
    try {
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    } catch {
      // Capture is a convenience; the gesture still works without it.
    }
    goToEvent(e);
  }

  function onMove(e: PointerEvent): void {
    // Dragging scrubs the view, which is how a minimap is actually used:
    // press roughly where you want to be, then adjust without letting go.
    if (!dragging || e.buttons === 0) return;
    goToEvent(e);
  }

  function onUp(e: PointerEvent): void {
    dragging = false;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // Already released.
    }
  }
</script>

{#if settingsStore.minimap && world && fit && nodes.length > 0}
  <div
    bind:this={boxEl}
    class="mm"
    data-chrome="minimap"
    style="width:{fit.w}px; height:{fit.h}px"
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
    role="presentation"
    aria-hidden="true"
  >
    {#each nodes as n (n.id)}
      <span
        class="mm-node"
        data-kind={n.kind}
        style="left:{(n.x - world.x) * fit.k}px; top:{(n.y - world.y) * fit.k}px; width:{Math.max(2, NODE_W * fit.k)}px; height:{Math.max(2, NODE_H * fit.k)}px"
      ></span>
    {/each}
    {#if viewportRect}
      <span
        class="mm-view"
        style="left:{viewportRect.left}px; top:{viewportRect.top}px; width:{viewportRect.width}px; height:{viewportRect.height}px"
      ></span>
    {/if}
  </div>
{/if}

<style>
  /* Sits in the canvas's top-right corner, opposite the zoom cluster, so the
     two pieces of view chrome do not compete for the same corner. Chrome,
     so it carries [data-chrome] and the pointer router leaves it alone
     (AGENTS.md "Canvas": any floating overlay carries [data-chrome] or it
     will swallow gestures). */
  .mm {
    position: absolute;
    top: var(--bar-clear, 84px);
    right: var(--sp-3);
    z-index: 8;
    background: var(--surface);
    border: var(--bw) solid var(--border);
    border-radius: var(--r-md);
    box-shadow: var(--shadow-sm);
    cursor: pointer;
    touch-action: none;
    overflow: hidden;
  }

  /* A node is a plain mark in its kind's colour. No label, no metric, no
     border. The colour comes from the global [data-kind] -> --k-* mapping
     in app.css, which already covers all 33 kinds -- unlike the web app's
     Minimap.css, nothing is redefined here. */
  .mm-node {
    position: absolute;
    background: var(--k-line, var(--line-2));
    border-radius: 1px;
  }

  /* The viewport. An outline rather than a tint, so it never obscures the
     marks it is meant to locate. */
  .mm-view {
    position: absolute;
    border: 1.5px solid var(--accent);
    border-radius: 2px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    pointer-events: none;
  }

  /* A small window is mostly canvas already, so the map would be taking
     room from the thing it helps you navigate. */
  @media (max-width: 1100px), (max-height: 620px) {
    .mm {
      display: none;
    }
  }
</style>
