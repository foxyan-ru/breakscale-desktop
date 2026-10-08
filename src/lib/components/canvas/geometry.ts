/**
 * Canvas geometry: node/edge dimensions, zoom limits, and per-kind readouts.
 *
 * Ported from the load-bearing constants block at the top of
 * `src/components/Canvas.tsx` (see MIGRATION_PLAN.md's Canvas interaction
 * task) plus that file's `readoutFor`/`sourceBacklogs` functions, which are
 * pure (kind/stats/config in, display values out) and worth keeping exactly
 * as tuned even though this is a from-scratch Svelte rendering layer.
 *
 * Re-exports the icon/name/group constants from `node-visuals.ts` and the
 * routing primitives from `edge-route.ts` too, so a sibling component (the
 * Palette, the Minimap) can import everything it needs to stay pixel-
 * compatible with the canvas from this one module, per the task brief.
 */

import type { NodeConfig, NodeKind, NodeStats, SimNode, Topology } from '$lib/domain/sim-types';
import {
  formatCount,
  formatMs,
  formatPct,
  formatRate,
  healthOfErr,
  healthOfLoad,
} from './format';
import type { Health } from './format';
import { KIND_NAME } from './node-visuals';
// The `Rect` re-export below (`export type { ... } from './edge-route'`) does
// not bind a local name, so `fitViewTo`'s signature needs its own import.
import type { Rect } from './edge-route';

export {
  ICON_BOX,
  ICON_STROKE,
  KIND_GROUPS,
  KIND_ICON,
  KIND_NAME,
  KIND_TERM,
  NODE_DND_MIME,
  STACK_MAX_LAYERS,
  STACK_STEP,
  STRIP_GAP_MAX,
  cellStrip,
  groupOfKind,
  stackBadge,
  stackLayers,
} from './node-visuals';
export type { CellStrip, IconNode, KindGroup, StackLayer } from './node-visuals';

export {
  ARROW_HALF,
  ARROW_INSET,
  ARROW_LEN,
  EDGE_RADIUS,
  EDGE_STUB,
  LANE_OFFSET,
  arrowPath,
  labelDyById,
  previewPath,
  roundedPath,
  routeEdge,
} from './edge-route';
export type { EdgeDir, EdgeRoute, Pt, Rect } from './edge-route';

/* ------------------------------------------------------------------ *
 * Geometry.
 *
 * Preset coordinates treat x/y as the node's top-left corner. 184x88 is
 * measured against the real preset spacing in the web app, not chosen for
 * looks:
 *
 *   async-workers   worker(730,200) and db(860,380) overlap horizontally
 *                   (130px apart, 184px wide) and read only because of the
 *                   vertical gap: 380 - (200 + 88) = 92px. Safe.
 *   load-balanced   api1/api2/api3 stack at y=80/220/360. At H=88 the gutter
 *                   is 140 - 88 = 52px, enough for three edges to fan
 *                   through. At H=96 it would be 44px and the fan-out that
 *                   preset exists to teach starts to crowd.
 *   async-workers   api(260) -> queue(500) is the tightest horizontal pair:
 *                   240 - 184 = 56px of edge. Fine.
 *
 * Any future change to these two numbers must be re-checked against those
 * three pairs (the desktop app's presets are ported 1:1 from the same
 * source, see MIGRATION_PLAN.md §5).
 * ------------------------------------------------------------------ */

export const NODE_W = 184;
export const NODE_H = 88;
/** Radii come from the design scale (3/4/6). The node body is the 6. */
export const NODE_R = 6;
/**
 * Gap between the node's outline and the selection ring drawn around it.
 *
 * 3px. Large enough that the ring is a separate object rather than a fringe
 * on the border, small enough that at 0.4x zoom (where it is 1.2 device px)
 * the ring has not visually detached from the node it belongs to.
 */
export const RING_GAP = 3;

/**
 * Internal layout.
 *
 * The node is a single surface divided by WHITESPACE and one hairline, never
 * by nested boxes. Vertical rhythm, all on the 4px space scale:
 *
 *    0   top edge, health rule rides here
 *    8   glyph top (18px box)
 *   26   header baseline area ends
 *   30   hairline under the header
 *   52   primary value baseline
 *   68   secondary row baseline
 *   80   meter
 */
export const HEAD_H = 30;
/** Horizontal inset for everything inside the body. */
export const PAD_X = 12;

/**
 * Width budget for ONE side of the two-cell secondary row.
 *
 * The row is two texts anchored to opposite edges of the node, so they grow
 * toward each other. Half the interior each, minus a gutter so they never
 * touch even when both are at their limit.
 */
export const SEC_HALF = (NODE_W - PAD_X * 2 - 14) / 2;

export const SPARK_X = 108;
export const SPARK_Y = 38;
export const SPARK_W = 64;
export const SPARK_H = 18;
/** One sample per second over the same 60s window the charts show. */
export const SPARK_LEN = 60;

/**
 * The per-unit strip's band: full inner width, in the gap above the meter.
 *
 * A shard's partitions are the node's primary content, so the strip gets the
 * full 160px rather than the sparkline's 64px slot. That width is what keeps
 * 64 partitions legible: 64 cells in 64px is 1.00px each, which cannot carry
 * a readable fill height, while 64 cells in 160px is 2.50px each, which can.
 */
export const STRIP_Y = 56;
export const STRIP_H = 12;

export const METER_Y = 80;
export const METER_H = 3;
export const METER_W = NODE_W - PAD_X * 2;

/**
 * Load at which the utilisation meter grows a threshold tick.
 *
 * DERIVED from `healthOfLoad` rather than restated, so this can never drift
 * out of agreement with `format.ts` about where warn begins (same approach
 * the web app's Canvas.tsx uses).
 */
export const WARN_AT = (() => {
  for (let v = 0; v <= 1000; v++) if (healthOfLoad(v / 1000) === 'warn') return v / 1000;
  return 0.7;
})();

/** Grid step. Node drag snaps to this when settingsStore.snapToGrid is on. */
export const GRID = 8;
/** Visual radius of a port dot. */
export const PORT_R = 5;
/**
 * Invisible hit radius. Roughly 3x the visual radius, per the rule that a
 * 5px target is not a target. Drawn as a transparent circle UNDER the visible
 * dot so the dot still paints at its true size.
 */
export const PORT_HIT_R = 15;
export const PORT_CY = NODE_H / 2;

export const MIN_ZOOM = 0.4;
export const MAX_ZOOM = 2.5;
/** One keyboard or button zoom step. Shared so the two cannot drift. */
export const ZOOM_STEP = 1.25;
/** Below this the body numbers and sparkline drop. */
export const DETAIL_ZOOM = 0.7;
/** Below this only the name, health rule and meter survive. */
export const MINIMAL_ZOOM = 0.5;

/** Auto-fit margin and clamp, matching the web app's Canvas.tsx exactly. */
export const FIT_MARGIN_WIDE = 64;
export const FIT_MARGIN_NARROW = 24;
export const NARROW_FIT_WIDTH = 720;

export function fitMarginFor(viewWidth: number): number {
  return viewWidth <= NARROW_FIT_WIDTH ? FIT_MARGIN_NARROW : FIT_MARGIN_WIDE;
}
export const FIT_MIN = DETAIL_ZOOM;
export const FIT_MAX = 1.5;

/**
 * Camera that frames `boxes` inside `visible`, expressed relative to
 * `surface` (the element the pan/zoom transform is anchored to).
 *
 * Field-for-field port of the web app's `fitTo` (`Canvas.tsx`): the scale
 * and the centring both use the VISIBLE rect, so a fit with panels open
 * frames the diagram inside the uncovered area and never parks a node under
 * an opaque panel, while the returned offset stays surface-relative because
 * that is what `transform: translate(x,y) scale(k)` reads. `visible`
 * defaults to `surface`, which is the all-panels-closed case.
 *
 * The scale is rounded to 3 decimals before the clamp, exactly as a zoom
 * step does, so a fit and a keyboard zoom can never report two spellings of
 * the same percentage. Returns null for an empty diagram or a zero-size
 * viewport (which is what an unmeasured layout looks like in a headless
 * test environment) and the caller leaves the camera alone.
 */
export function fitViewTo(
  boxes: readonly Rect[],
  surface: Rect,
  visible: Rect = surface,
): { x: number; y: number; k: number } | null {
  if (boxes.length === 0) return null;
  if (visible.w === 0 || visible.h === 0) return null;

  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const b of boxes) {
    if (b.x < minX) minX = b.x;
    if (b.y < minY) minY = b.y;
    if (b.x + b.w > maxX) maxX = b.x + b.w;
    if (b.y + b.h > maxY) maxY = b.y + b.h;
  }
  const bw = Math.max(1, maxX - minX);
  const bh = Math.max(1, maxY - minY);
  const margin = fitMarginFor(visible.w);
  const k = clamp(
    Math.round(
      Math.min((visible.w - margin * 2) / bw, (visible.h - margin * 2) / bh) * 1000,
    ) / 1000,
    FIT_MIN,
    FIT_MAX,
  );
  return {
    k,
    x: visible.x - surface.x + (visible.w - bw * k) / 2 - minX * k,
    y: visible.y - surface.y + (visible.h - bh * k) / 2 - minY * k,
  };
}

/** Glyph geometry: see the web app's Canvas.tsx for the measured derivation. */
export const GLYPH_PX = 18;
export const GLYPH_INK_CENTER = 12.11;
export const HEAD_CENTER_Y = 15;
export const NAME_GAP = 8;
export const MARK_SIZE = 7;
export const MARK_RESERVE = MARK_SIZE + NAME_GAP;

export function clamp(v: number, lo: number, hi: number): number {
  return v < lo ? lo : v > hi ? hi : v;
}

export function snapTo(v: number, grid: number): number {
  return Math.round(v / grid) * grid;
}

/**
 * Does this drag land on the grid?
 *
 * Field-for-field port of the web app's `snapsToGrid` (upstream `1db4ac61`,
 * PR #37, `src/components/pointerInput.ts`). Two inputs, and they are
 * different kinds of thing: `snapOn` is the standing `settingsStore.
 * snapToGrid` preference, which persists and is what the G key and the
 * Settings switch both write; `ctrlHeld` is a momentary override for the
 * drag in progress and changes nothing that outlives it. Ctrl only ever
 * LOOSENS -- holding it while the preference is already off does not turn
 * snapping back on, which would be a strange thing for a bypass to do.
 *
 * Kept here rather than inline in Canvas.svelte so the join between the
 * preference and the four drag paths that must obey it (node, group-drag,
 * resize, palette-drop) can be pinned by a unit test without mounting the
 * canvas -- see `__tests__/snap-to-grid.test.ts`, the same reason web pins
 * it in `Canvas.pointer.test.ts` without mounting `Canvas.tsx`.
 */
export function snapsToGrid(snapOn: boolean, ctrlHeld: boolean): boolean {
  return snapOn && !ctrlHeld;
}

/**
 * A generic starting config for a node placed from the palette.
 *
 * The web app's equivalent, `defaultConfig(kind)` in `src/sim/presets.ts`, is
 * a 34-way switch tuning each kind's starting numbers individually (a fresh
 * cache starts with a sane `hitRate`, a fresh shard with a sane
 * `shardCount`, and so on). That file is data/logic outside this task's
 * scope (see MIGRATION_PLAN.md §5 -- it ports separately, as
 * `sim::presets::default_config` on the Rust side plus a JSON preset port).
 * `Canvas.svelte` still needs SOME complete `NodeConfig` to hand `addNode`
 * the moment a palette drop lands, so this is one flat, honestly generic
 * default: every field the type requires gets a safe, plausible value, and
 * the handful of fields that would be structurally broken at 0 (shard/
 * replica counts) get a floor of 1 or 2. A student drops a node and then
 * tunes it in the Inspector, same as always -- this just stops the very
 * first frame after a drop from being nonsensical.
 *
 * KNOWN LIMITATION: unlike the web app, a freshly dropped node of every kind
 * starts numerically identical (same capacity/serviceMs/etc.) rather than
 * pre-tuned per kind. See this migration's "Known limitations".
 */
export function defaultNodeConfig(kind: NodeKind): NodeConfig {
  return {
    capacity: 10,
    serviceMs: 20,
    serviceCv: 0.5,
    queueLimit: 100,
    hitRate: kind === 'cache' || kind === 'cdn' ? 0.8 : 0,
    errorRate: 0.01,
    timeoutMs: 2000,
    retries: 0,
    rps: kind === 'client' ? 50 : 0,
    replicaCount: 2,
    replicationLagMs: 50,
    readFraction: 0.8,
    shardCount: 4,
    shardCapacity: 10,
    hotKeyFraction: 0,
  };
}

/** Monotonic id generator for nodes/edges created directly on the canvas. */
let idSeq = 0;
export function newId(prefix: string): string {
  idSeq += 1;
  return `${prefix}-${Date.now().toString(36)}-${idSeq.toString(36)}`;
}

/**
 * Mint a node for the canvas.
 *
 * `taken` is the set of ids already on the canvas -- the LIVE topology
 * mirror, read at the call site, not a snapshot captured earlier. Field-
 * for-field port of upstream `4fd46c47` (PR #65, "never mint a node id that
 * is already on the canvas"): `newId`'s counter is already time-seeded
 * (`Date.now().toString(36)`), which makes an ACCIDENTAL collision between
 * two nodes minted in the same session astronomically unlikely, but that is
 * a different guarantee from "never reuses an id already on the canvas" --
 * a design loaded from a file, a share link, or an older session can carry
 * ids this session's counter knows nothing about, and nothing upstream of
 * this function previously checked for that. `taken` closes exactly that
 * gap: when it is supplied (every real call site does, via
 * `topologyStore.topology.nodes`), a collision retries with a fresh id
 * instead of silently handing out one two nodes would then share --
 * selecting either would select both, the Inspector would read "2
 * components", and an edge or a config change meant for one would land on
 * both. Optional, and checked only when present, so the many existing
 * `makeNode('client', 0, 0)` call sites in this repo's own tests (which
 * have no topology to check against) are unaffected.
 */
export function makeNode(
  kind: NodeKind,
  x: number,
  y: number,
  taken?: ReadonlySet<string>,
): SimNode {
  let id = newId('node');
  while (taken?.has(id)) id = newId('node');
  return {
    id,
    kind,
    label: KIND_NAME[kind],
    x,
    y,
    config: defaultNodeConfig(kind),
  };
}

/* ------------------------------------------------------------------ *
 * Readouts.
 *
 * Five live values per node, all from fields the engine already computes.
 * Field-for-field port of `readoutFor` in the web app's Canvas.tsx: this
 * switch is deliberately EXHAUSTIVE over NodeKind, so a future kind not
 * handled here is a compile error rather than a silent "0% util, 0ms p99,
 * 0 queue" for every kind the switch does not know.
 * ------------------------------------------------------------------ */

export interface Cell {
  value: string;
  label: string;
}

export interface Readout {
  primary: Cell;
  a: Cell;
  b: Cell;
  /** 0..1. Drives the meter width and the health band. */
  load: number;
  /** Health of the primary metric specifically. */
  health: Health;
  /** Value the sparkline plots, in the primary's units. */
  spark: number;
  /**
   * True when `spark` is a 0..1 fraction and the sparkline should plot it
   * against a fixed unit domain; false for counts and rates, which autoscale.
   */
  sparkUnit: boolean;
  /** True when traffic is actively being lost here. */
  losing: boolean;
}

/** A safe 0..1 ratio: 0 whenever the denominator cannot support one. */
function frac(n: number, d: number): number {
  if (!Number.isFinite(n) || !Number.isFinite(d) || d <= 0) return 0;
  return clamp(n / d, 0, 1);
}

/**
 * The worse of two healths, so a kind can combine "how loaded" with "is it
 * refusing work" without either channel masking the other.
 */
function worstHealth(a: Health, b: Health): Health {
  if (a === 'danger' || b === 'danger') return 'danger';
  if (a === 'warn' || b === 'warn') return 'warn';
  return 'ok';
}

/**
 * What the three numbers on a node MEAN, per kind.
 *
 * `backlog` is the summed depth of the buffer nodes feeding a pull-based
 * consumer (worker, transcoder). Their own queue is structurally always 0;
 * the work they are behind on lives in the queues that feed them. See
 * `sourceBacklogs` below.
 */
export function readoutFor(
  kind: NodeKind,
  s: NodeStats,
  cfg: NodeConfig,
  backlog = 0,
): Readout {
  const util = clamp(s.utilization, 0, 1);
  const losing = s.shedRate + s.timeoutRate > 0;

  switch (kind) {
    case 'client': {
      const err = clamp(s.errorRate, 0, 1);
      return {
        primary: { value: formatRate(s.arrivalRate), label: 'sent' },
        a: { value: formatRate(s.throughput), label: 'ok' },
        b: { value: formatPct(err), label: 'failing' },
        load: err,
        health: healthOfErr(err),
        spark: s.arrivalRate,
        sparkUnit: false,
        losing: losing || err > 0,
      };
    }

    case 'lb':
      return {
        primary: { value: formatRate(s.throughput), label: 'served' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b: { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health: healthOfLoad(util),
        spark: s.throughput,
        sparkUnit: false,
        losing,
      };

    case 'cache': {
      const hit = clamp(s.hitRate, 0, 1);
      const miss = 1 - hit;
      return {
        primary: { value: formatPct(hit), label: 'hit' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b: { value: formatRate(s.throughput), label: 'served' },
        load: miss,
        health: healthOfLoad(miss),
        spark: hit,
        sparkUnit: true,
        losing,
      };
    }

    case 'queue': {
      const depth = s.queued + s.inFlight;
      const limit = cfg.queueLimit > 0 ? cfg.queueLimit : 1;
      const fill = clamp(depth / limit, 0, 1);
      return {
        primary: { value: formatCount(depth), label: 'waiting' },
        a: { value: formatRate(s.arrivalRate), label: 'in' },
        b: { value: formatRate(s.throughput), label: 'out' },
        load: fill,
        health: healthOfLoad(fill),
        spark: depth,
        sparkUnit: false,
        losing,
      };
    }

    case 'shard': {
      const hot = clamp(s.maxShardUtilization, 0, 1);
      return {
        primary: { value: formatPct(hot), label: 'hot' },
        a: { value: formatPct(clamp(s.minShardUtilization, 0, 1)), label: 'cold' },
        b: { value: formatCount(s.queued), label: 'waiting' },
        load: hot,
        health: healthOfLoad(hot),
        spark: hot,
        sparkUnit: true,
        losing,
      };
    }

    case 'replica': {
      const stale = clamp(s.staleReadRate, 0, 1);
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a:
          stale > 0
            ? { value: formatPct(stale), label: 'stale' }
            : { value: formatMs(s.p99), label: 'p99' },
        b: { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health: healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };
    }

    case 'autoscaler': {
      const watched = clamp(s.watchedUtil ?? 0, 0, 1);
      const setpoint = s.setpoint ?? 0.7;
      const want = s.targetInstances ?? 0;
      const have = s.watchedInstances ?? 0;
      const drift = setpoint > 0 ? clamp(watched / setpoint, 0, 1) : 0;
      const phase = s.scalePhase ?? 'idle';
      return {
        primary: { value: formatPct(watched), label: 'watching' },
        a: { value: `${have}/${want}`, label: 'pods' },
        b: { value: '', label: phase },
        load: drift,
        health: healthOfLoad(drift),
        spark: watched,
        sparkUnit: true,
        losing: false,
      };
    }

    case 'service':
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b: { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health: healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };

    case 'db': {
      const lockWait = s.lockWaitMs ?? 0;
      const contended = lockWait >= 1;
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b: contended
          ? { value: formatMs(lockWait), label: 'lock' }
          : { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health:
          contended && lockWait > s.p50
            ? worstHealth(healthOfLoad(util), 'warn')
            : healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };
    }

    case 'objectstore': {
      const slowdown = s.slowdownRate ?? 0;
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b:
          slowdown > 0
            ? { value: formatRate(slowdown), label: 'refused' }
            : { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health: slowdown > 0 ? 'danger' : healthOfLoad(util),
        spark: slowdown > 0 ? slowdown : util,
        sparkUnit: slowdown === 0,
        losing: losing || slowdown > 0,
      };
    }

    case 'worker':
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatRate(s.throughput), label: 'done' },
        b: { value: formatCount(backlog), label: 'waiting' },
        load: util,
        health: healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };

    case 'transcoder': {
      const stuck = util >= 0.999 && backlog > 0;
      return {
        primary: { value: formatCount(s.inFlight), label: 'jobs' },
        a: { value: formatCount(backlog), label: 'waiting' },
        b: { value: formatMs(s.p50), label: 'per job' },
        load: util,
        health: stuck ? 'danger' : healthOfLoad(util),
        spark: backlog,
        sparkUnit: false,
        losing,
      };
    }

    case 'region': {
      const total = s.regionsTotal ?? 1;
      const healthy = s.regionsHealthy ?? total;
      const dark = s.failingOver === true;
      const err = clamp(s.errorRate, 0, 1);
      return {
        primary: dark
          ? { value: 'dark', label: 'failover' }
          : { value: `R${s.activeRegion ?? 0}`, label: 'serving' },
        a: { value: `${healthy}/${total}`, label: 'healthy' },
        b: dark
          ? { value: formatMs(s.failoverRemainingMs), label: 'back in' }
          : { value: formatRate(s.throughput), label: 'served' },
        load: dark ? 1 : 1 - frac(healthy, total),
        health: dark || healthy === 0 ? 'danger' : healthy < total ? 'warn' : 'ok',
        spark: err,
        sparkUnit: true,
        losing: dark || err > 0,
      };
    }

    case 'cdn': {
      const hit = clamp(s.hitRate, 0, 1);
      const miss = 1 - hit;
      return {
        primary: { value: formatPct(hit), label: 'hit' },
        a: { value: formatRate(s.originFetchRate), label: 'origin' },
        b: { value: formatRate(s.throughput), label: 'served' },
        load: miss,
        health: healthOfLoad(miss),
        spark: hit,
        sparkUnit: true,
        losing,
      };
    }

    case 'ratelimiter': {
      const throttled = s.throttledRate ?? 0;
      const admitted = s.admittedRate ?? 0;
      const burst = cfg.burst ?? cfg.rateLimitRps ?? 1;
      return {
        primary: { value: formatRate(throttled), label: 'refused' },
        a: { value: formatRate(admitted), label: 'passed' },
        b: { value: formatCount(s.tokens), label: 'tokens' },
        load: 1 - frac(s.tokens ?? 0, burst),
        health: throttled <= 0 ? 'ok' : throttled < admitted ? 'warn' : 'danger',
        spark: throttled,
        sparkUnit: false,
        losing: throttled > 0,
      };
    }

    case 'breaker': {
      const state = s.breakerState ?? 'closed';
      const errRate = clamp(s.breakerErrorRate ?? 0, 0, 1);
      const rejected = s.rejectedRate ?? 0;
      const threshold = cfg.errorThreshold ?? 0.5;
      const strain = state === 'closed' ? frac(errRate, threshold) : 1;
      return {
        primary:
          state === 'open'
            ? { value: 'OPEN', label: 'circuit' }
            : state === 'half-open'
              ? { value: 'probing', label: 'circuit' }
              : { value: 'closed', label: 'circuit' },
        a: { value: formatPct(errRate), label: 'dep fails' },
        b:
          state === 'open'
            ? { value: formatRate(rejected), label: 'refused' }
            : { value: formatCount(s.breakerTrips), label: 'trips' },
        load: strain,
        health:
          state === 'open'
            ? 'danger'
            : state === 'half-open'
              ? 'warn'
              : healthOfLoad(strain),
        spark: errRate,
        sparkUnit: true,
        losing: rejected > 0,
      };
    }

    case 'searchindex': {
      const stale = clamp(s.staleSearchRate ?? 0, 0, 1);
      return {
        primary: { value: formatPct(stale), label: 'stale' },
        a: { value: formatRate(s.searchRate), label: 'search' },
        b: { value: formatRate(s.indexWriteRate), label: 'index' },
        load: util,
        health: worstHealth(healthOfLoad(util), stale > 0.2 ? 'warn' : 'ok'),
        spark: stale,
        sparkUnit: true,
        losing,
      };
    }

    case 'timeseriesdb':
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatRate(s.appendRate), label: 'append' },
        b: { value: formatRate(s.rangeQueryRate), label: 'range' },
        load: util,
        health: healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };

    case 'graphdb':
      return {
        primary: { value: formatMs(s.traversalCostMs), label: 'query' },
        a: { value: formatPct(util), label: 'busy' },
        b: { value: formatRate(s.throughput), label: 'served' },
        load: util,
        health: healthOfLoad(util),
        spark: s.traversalCostMs ?? 0,
        sparkUnit: false,
        losing,
      };

    case 'coldstorage': {
      const denied = s.throttledRate ?? 0;
      return {
        primary: { value: formatMs(s.p99), label: 'restore' },
        a: { value: formatPct(util), label: 'busy' },
        b:
          denied > 0
            ? { value: formatRate(denied), label: 'refused' }
            : { value: formatCount(s.inFlight), label: 'jobs' },
        load: util,
        health: denied > 0 ? 'danger' : healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing: losing || denied > 0,
      };
    }

    case 'vectordb':
      return {
        primary: { value: formatMs(s.queryCostMs), label: 'query' },
        a: { value: formatPct(util), label: 'busy' },
        b: { value: formatRate(s.throughput), label: 'served' },
        load: util,
        health: healthOfLoad(util),
        spark: s.queryCostMs ?? 0,
        sparkUnit: false,
        losing,
      };

    case 'streambroker': {
      const lag = s.consumerLag ?? 0;
      const dropRate = s.retentionDropRate ?? 0;
      const fill = frac(lag, s.queueLimit);
      return {
        primary: { value: formatCount(lag), label: 'lag' },
        a:
          dropRate > 0
            ? { value: formatRate(dropRate), label: 'lost' }
            : { value: formatRate(s.deliveryRate), label: 'deliver' },
        b: { value: formatRate(s.throughput), label: 'publish' },
        load: fill,
        health: dropRate > 0 ? 'danger' : healthOfLoad(fill),
        spark: lag,
        sparkUnit: false,
        losing: dropRate > 0 || losing,
      };
    }

    case 'pubsub':
      return {
        primary: { value: formatRate(s.deliveryRate), label: 'out' },
        a: { value: formatRate(s.throughput), label: 'in' },
        b: { value: `x${s.fanout ?? 0}`, label: 'fan-out' },
        load: 0,
        health: 'ok',
        spark: s.deliveryRate ?? 0,
        sparkUnit: false,
        losing: false,
      };

    case 'websocket': {
      const open = s.connectionsOpen ?? 0;
      const max = s.maxConnections ?? 0;
      const refused = s.connectionRejectRate ?? 0;
      const fill = frac(open, max);
      return {
        primary: { value: formatCount(open), label: 'conns' },
        a: { value: formatCount(max), label: 'slots' },
        b:
          refused > 0
            ? { value: formatRate(refused), label: 'refused' }
            : { value: formatRate(s.connectRate), label: 'new' },
        load: fill,
        health: refused > 0 ? 'danger' : healthOfLoad(fill),
        spark: fill,
        sparkUnit: true,
        losing: refused > 0,
      };
    }

    case 'apigateway': {
      const admitted = s.admittedRate ?? 0;
      const throttled = s.throttledRate ?? 0;
      const badAuth = s.authRejectRate ?? 0;
      const burst = cfg.burst ?? cfg.rateLimitRps ?? 1;
      return {
        primary: { value: formatRate(admitted), label: 'passed' },
        a: { value: formatRate(throttled), label: 'limited' },
        b: { value: formatRate(badAuth), label: 'denied' },
        load: 1 - frac(s.tokens ?? 0, burst),
        health: throttled > admitted ? 'danger' : throttled > 0 ? 'warn' : 'ok',
        spark: admitted,
        sparkUnit: false,
        losing: throttled + badAuth > 0,
      };
    }

    case 'sidecar': {
      const state = s.breakerState ?? 'closed';
      const fails = s.upstreamFailRate ?? 0;
      const rejected = s.rejectedRate ?? 0;
      const after = Math.max(1, cfg.outlierAfter ?? 5);
      const strain = state === 'closed' ? frac(s.consecutiveFails ?? 0, after) : 1;
      return {
        primary:
          state === 'open'
            ? { value: 'EJECTED', label: 'upstream' }
            : state === 'half-open'
              ? { value: 'probing', label: 'upstream' }
              : { value: 'proxying', label: '' },
        a: { value: formatRate(fails), label: 'fails' },
        b:
          state === 'open'
            ? { value: formatRate(rejected), label: 'refused' }
            : { value: formatMs(s.p99), label: 'p99' },
        load: strain,
        health:
          state === 'open'
            ? 'danger'
            : state === 'half-open' || strain > 0
              ? 'warn'
              : 'ok',
        spark: fails,
        sparkUnit: false,
        losing: rejected > 0,
      };
    }

    case 'lambda': {
      const cold = clamp(s.coldStartRate ?? 0, 0, 1);
      const running = s.runningNow ?? 0;
      const cap = Math.max(1, cfg.maxConcurrency ?? 1);
      const throttled = s.throttledRate ?? 0;
      const fill = frac(running, cap);
      return {
        primary: { value: formatPct(cold), label: 'cold' },
        a: { value: `${formatCount(running)}/${formatCount(cap)}`, label: 'running' },
        b:
          throttled > 0
            ? { value: formatRate(throttled), label: 'refused' }
            : { value: formatCount(s.warmIdle), label: 'warm' },
        load: fill,
        health:
          throttled > 0
            ? 'danger'
            : worstHealth(healthOfLoad(fill), cold > 0.5 ? 'warn' : 'ok'),
        spark: cold,
        sparkUnit: true,
        losing: throttled > 0,
      };
    }

    case 'cron': {
      const interval = Math.max(1, cfg.intervalMs ?? 20000);
      const next = s.nextFireInMs ?? interval;
      return {
        primary: { value: formatMs(next), label: 'next run' },
        a: { value: formatCount(s.burstSize), label: 'burst' },
        b: { value: formatCount(s.batchEmitted), label: 'sent' },
        load: 1 - frac(next, interval),
        health: 'ok',
        spark: s.batchEmitted ?? 0,
        sparkUnit: false,
        losing: false,
      };
    }

    case 'bulkhead': {
      const inUse = s.bulkheadInFlight ?? 0;
      const limit = s.bulkheadLimit ?? 0;
      const refused = s.bulkheadRejectedRate ?? 0;
      const fill = frac(inUse, limit);
      return {
        primary: {
          value: `${formatCount(inUse)}/${formatCount(limit)}`,
          label: 'pool',
        },
        a: { value: formatRate(refused), label: 'refused' },
        b: { value: formatMs(s.p99), label: 'p99' },
        load: fill,
        health: refused > 0 ? 'danger' : healthOfLoad(fill),
        spark: fill,
        sparkUnit: true,
        losing: refused > 0,
      };
    }

    case 'retryqueue': {
      const dead = s.deadLetters ?? 0;
      const deadRate = s.deadLetterRate ?? 0;
      const redeliver = s.redeliveryRate ?? 0;
      const fill = frac(s.queued, s.queueLimit);
      return {
        primary: { value: formatCount(dead), label: 'dead' },
        a: { value: formatRate(s.deliveredRate), label: 'deliver' },
        b: { value: formatRate(redeliver), label: 'retry' },
        load: fill,
        health:
          deadRate > 0
            ? 'danger'
            : worstHealth(healthOfLoad(fill), redeliver > 0 ? 'warn' : 'ok'),
        spark: dead,
        sparkUnit: false,
        losing: deadRate > 0 || losing,
      };
    }

    case 'edgecompute': {
      const handled = s.edgeHandledRate ?? 0;
      const passed = s.passedThroughRate ?? 0;
      const share = frac(handled, handled + passed);
      return {
        primary: { value: formatPct(share), label: 'at edge' },
        a: { value: formatRate(passed), label: 'origin' },
        b: { value: formatMs(s.p99), label: 'p99' },
        load: 1 - share,
        health: healthOfLoad(util),
        spark: share,
        sparkUnit: true,
        losing,
      };
    }

    case 'writebehind': {
      const failRate = s.flushFailRate ?? 0;
      return {
        primary: { value: formatCount(s.dirtyWrites), label: 'at risk' },
        a: { value: formatRate(s.flushedRate), label: 'flushed' },
        b: { value: formatRate(s.throughput), label: 'acked' },
        load: util,
        health: failRate > 0 ? 'danger' : healthOfLoad(util),
        spark: s.dirtyWrites ?? 0,
        sparkUnit: false,
        losing: losing || failRate > 0,
      };
    }

    case 'loadshedder': {
      const lowShed = s.lowSheddedRate ?? 0;
      const highShed = s.highSheddedRate ?? 0;
      const highIn = s.highAdmittedRate ?? 0;
      const burst = cfg.burst ?? cfg.rateLimitRps ?? 1;
      return {
        primary: { value: formatRate(lowShed), label: 'shed low' },
        a: { value: formatRate(highIn), label: 'high in' },
        b:
          highShed > 0
            ? { value: formatRate(highShed), label: 'shed high' }
            : { value: formatCount(s.tokens), label: 'tokens' },
        load: 1 - frac(s.tokens ?? 0, burst),
        health: highShed > 0 ? 'danger' : lowShed > 0 ? 'warn' : 'ok',
        spark: lowShed + highShed,
        sparkUnit: false,
        losing: lowShed + highShed > 0,
      };
    }

    default: {
      // Runtime fallback for a kind not (yet) handled above -- reachable only
      // from data outside the type system (a corrupted save loaded from an
      // older/newer app version), never from a live TS call site, since the
      // switch above is exhaustive over NodeKind.
      return {
        primary: { value: formatPct(util), label: 'busy' },
        a: { value: formatMs(s.p99), label: 'p99' },
        b: { value: formatCount(s.queued), label: 'waiting' },
        load: util,
        health: healthOfLoad(util),
        spark: util,
        sparkUnit: true,
        losing,
      };
    }
  }
}

/**
 * Summed buffer depth feeding each pull-based consumer, keyed by node id.
 *
 * A worker's own `queued` is structurally ALWAYS zero: it pulls, so its
 * backlog lives in the queue nodes wired to it. In the web app this is
 * derived from `behaviourFor(kind).pullsFromQueues` /
 * `.buffersForConsumers`, flags that live on the TS simulation engine
 * (`src/sim/behaviour*.ts`). The desktop app's engine runs in Rust and
 * exposes no equivalent lookup to the frontend, so the two kind sets are
 * hardcoded here instead -- verified against the web app's behaviour
 * modules, where exactly one kind sets each flag: `queue` is the only
 * `buffersForConsumers: true`, and `worker`/`transcoder` are the only
 * `pullsFromQueues: true`. If a future kind changes either flag on the Rust
 * side, this table needs a matching edit; nothing enforces that
 * automatically, which is a known gap (see this module's caller for the
 * "known limitations" note).
 */
const BACKLOG_CONSUMER_KINDS: ReadonlySet<NodeKind> = new Set(['worker', 'transcoder']);
const BACKLOG_BUFFER_KINDS: ReadonlySet<NodeKind> = new Set(['queue']);

export function sourceBacklogs(
  topology: Topology,
  nodes: Record<string, NodeStats>,
): Map<string, number> {
  const out = new Map<string, number>();
  const kindById = new Map<string, NodeKind>();
  for (const n of topology.nodes) kindById.set(n.id, n.kind);

  const add = (consumerId: string, bufferId: string) => {
    const s = nodes[bufferId];
    if (!s) return;
    out.set(consumerId, (out.get(consumerId) ?? 0) + Math.max(0, s.queued));
  };

  for (const e of topology.edges) {
    if (e.control) continue;
    const fromKind = kindById.get(e.from);
    const toKind = kindById.get(e.to);
    if (!fromKind || !toKind) continue;
    // buffer -> consumer, and consumer -> buffer: either drawing is accepted.
    if (BACKLOG_CONSUMER_KINDS.has(toKind) && BACKLOG_BUFFER_KINDS.has(fromKind)) {
      add(e.to, e.from);
    } else if (
      BACKLOG_CONSUMER_KINDS.has(fromKind) &&
      BACKLOG_BUFFER_KINDS.has(toKind)
    ) {
      add(e.from, e.to);
    }
  }
  return out;
}
