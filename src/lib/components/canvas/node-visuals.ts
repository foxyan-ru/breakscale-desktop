import type { NodeKind } from '$lib/domain/sim-types';

/* ------------------------------------------------------------------ *
 * Kind icons, from Lucide.
 *
 * Field-for-field port of `src/components/nodeVisuals.ts` from the web app
 * (see MIGRATION_PLAN.md's Canvas interaction task).
 *
 * WHY THE ICON DATA IS INLINED HERE, AND NOT IMPORTED FROM lucide-react.
 * The web app imports each icon's `__iconNode` (the raw [tag, attrs][]
 * primitive list a lucide-react component is built from) directly from
 * `lucide-react/dist/esm/icons/*.mjs`. `desktop/package.json` carries no
 * icon library (see MIGRATION_PLAN.md §10: "not added: an icon component
 * library"), and this migration may not add an npm dependency beyond what
 * is already declared. Lucide's icons are plain data -- an array of SVG
 * primitive tuples -- under the ISC licence, so each of the 33 primitive
 * lists below was copied verbatim from the installed `lucide-react` package
 * (`node_modules/lucide-react/dist/esm/icons/*.mjs`, v1.34.0) rather than
 * re-drawn or approximated. Same shapes, same 24x24 grid, same stroke
 * weight as the web app.
 *
 * One icon per kind, all cut on the same 24x24 grid at the same stroke
 * weight, which is what makes thirty-three of them read as a family rather
 * than as thirty-three separately-sourced pictures.
 *
 * CHOOSING RULES, in priority order (unchanged from the web app):
 *   BEHAVIOUR   the icon depicts what the component DOES to traffic, not
 *               what the hardware looks like, because the behaviour is the
 *               thing a student is here to learn.
 *   FAMILY      related kinds quote a shared shape and differ in one mark:
 *               db / cache / replica are all the cylinder, ratelimiter /
 *               loadshedder are both the funnel, cdn / edgecompute are
 *               both the cloud, queue / streambroker / retryqueue are all
 *               rows of items.
 *   CONTRAST    unrelated kinds must not collide at 14px. The circular
 *               outlines (globe, clock) stay unique in the set.
 * ------------------------------------------------------------------ */

/** MIME type the palette sets on dragstart and the canvas checks for on drop. */
export const NODE_DND_MIME = 'application/x-breakscale-node';

/**
 * One Lucide icon as its raw SVG primitives: [tagName, attributes] pairs.
 * Drawn in a 24x24 box (ICON_BOX) expecting stroke-width ICON_STROKE,
 * currentColor stroke and no fill from the surrounding container. A few
 * primitives (chart-scatter's points) carry their own fill="currentColor",
 * which the renderer must pass through untouched.
 */
export type IconNode = [elementName: string, attrs: Record<string, string>][];

/* Traffic */
const icClient: IconNode = [
  ['path', { d: 'M18 8V6a2 2 0 0 0-2-2H4a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h8' }],
  ['path', { d: 'M10 19v-3.96 3.15' }],
  ['path', { d: 'M7 19h5' }],
  ['rect', { width: '6', height: '10', x: '16', y: '12', rx: '2' }],
];
const icLb: IconNode = [
  ['path', { d: 'M16 3h5v5' }],
  ['path', { d: 'M8 3H3v5' }],
  ['path', { d: 'M12 22v-8.3a4 4 0 0 0-1.172-2.872L3 3' }],
  ['path', { d: 'm15 9 6-6' }],
];
const icCdn: IconNode = [
  ['path', { d: 'M12 13v8l-4-4' }],
  ['path', { d: 'm12 21 4-4' }],
  ['path', { d: 'M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284' }],
];
const icRegion: IconNode = [
  ['circle', { cx: '12', cy: '12', r: '10' }],
  ['path', { d: 'M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20' }],
  ['path', { d: 'M2 12h20' }],
];
/* Compute */
const icService: IconNode = [
  ['rect', { width: '20', height: '8', x: '2', y: '2', rx: '2', ry: '2' }],
  ['rect', { width: '20', height: '8', x: '2', y: '14', rx: '2', ry: '2' }],
  ['line', { x1: '6', x2: '6.01', y1: '6', y2: '6' }],
  ['line', { x1: '6', x2: '6.01', y1: '18', y2: '18' }],
];
const icWorker: IconNode = [
  ['path', { d: 'M11 10.27 7 3.34' }],
  ['path', { d: 'm11 13.73-4 6.93' }],
  ['path', { d: 'M12 22v-2' }],
  ['path', { d: 'M12 2v2' }],
  ['path', { d: 'M14 12h8' }],
  ['path', { d: 'm17 20.66-1-1.73' }],
  ['path', { d: 'm17 3.34-1 1.73' }],
  ['path', { d: 'M2 12h2' }],
  ['path', { d: 'm20.66 17-1.73-1' }],
  ['path', { d: 'm20.66 7-1.73 1' }],
  ['path', { d: 'm3.34 17 1.73-1' }],
  ['path', { d: 'm3.34 7 1.73 1' }],
  ['circle', { cx: '12', cy: '12', r: '2' }],
  ['circle', { cx: '12', cy: '12', r: '8' }],
];
const icLambda: IconNode = [
  ['rect', { width: '18', height: '18', x: '3', y: '3', rx: '2', ry: '2' }],
  ['path', { d: 'M9 17c2 0 2.8-1 2.8-2.8V10c0-2 1-3.3 3.2-3' }],
  ['path', { d: 'M9 11.2h5.7' }],
];
const icEdgecompute: IconNode = [
  ['path', { d: 'M6 16.326A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 .5 8.973' }],
  ['path', { d: 'm13 12-3 5h4l-3 5' }],
];
const icTranscoder: IconNode = [
  ['rect', { width: '18', height: '18', x: '3', y: '3', rx: '2' }],
  ['path', { d: 'M7 3v18' }],
  ['path', { d: 'M3 7.5h4' }],
  ['path', { d: 'M3 12h18' }],
  ['path', { d: 'M3 16.5h4' }],
  ['path', { d: 'M17 3v18' }],
  ['path', { d: 'M17 7.5h4' }],
  ['path', { d: 'M17 16.5h4' }],
];
const icCron: IconNode = [
  ['path', { d: 'M16 14v2.2l1.6 1' }],
  ['path', { d: 'M16 2v3' }],
  ['path', { d: 'M21 7.338V5a2 2 0 00-2-2H5a2 2 0 00-2 2v14a2 2 0 002 2h2.338' }],
  ['path', { d: 'M3 9h5.859' }],
  ['path', { d: 'M8 2v3' }],
  ['circle', { cx: '16', cy: '16', r: '6' }],
];
/* Data */
const icDb: IconNode = [
  ['ellipse', { cx: '12', cy: '5', rx: '9', ry: '3' }],
  ['path', { d: 'M3 5V19A9 3 0 0 0 21 19V5' }],
  ['path', { d: 'M3 12A9 3 0 0 0 21 12' }],
];
const icCache: IconNode = [
  ['ellipse', { cx: '12', cy: '5', rx: '9', ry: '3' }],
  ['path', { d: 'M3 5V19A9 3 0 0 0 15 21.84' }],
  ['path', { d: 'M21 5V8' }],
  ['path', { d: 'M21 12L18 17H22L19 22' }],
  ['path', { d: 'M3 12A9 3 0 0 0 14.59 14.87' }],
];
const icReplica: IconNode = [
  ['ellipse', { cx: '12', cy: '5', rx: '9', ry: '3' }],
  ['path', { d: 'M3 12a9 3 0 0 0 5 2.69' }],
  ['path', { d: 'M21 9.3V5' }],
  ['path', { d: 'M3 5v14a9 3 0 0 0 6.47 2.88' }],
  ['path', { d: 'M12 12v4h4' }],
  [
    'path',
    {
      d: 'M13 20a5 5 0 0 0 9-3 4.5 4.5 0 0 0-4.5-4.5c-1.33 0-2.54.54-3.41 1.41L12 16',
    },
  ],
];
const icShard: IconNode = [
  ['rect', { width: '18', height: '18', x: '3', y: '3', rx: '2' }],
  ['path', { d: 'M9 3v18' }],
  ['path', { d: 'M15 3v18' }],
];
const icObjectstore: IconNode = [
  [
    'path',
    {
      d: 'M11 21.73a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73z',
    },
  ],
  ['path', { d: 'M12 22V12' }],
  ['polyline', { points: '3.29 7 12 12 20.71 7' }],
  ['path', { d: 'm7.5 4.27 9 5.15' }],
];
const icSearchindex: IconNode = [
  ['path', { d: 'M21 5H3' }],
  ['path', { d: 'M10 12H3' }],
  ['path', { d: 'M10 19H3' }],
  ['circle', { cx: '17', cy: '15', r: '3' }],
  ['path', { d: 'm21 19-1.9-1.9' }],
];
const icTimeseriesdb: IconNode = [
  ['path', { d: 'M3 3v16a2 2 0 0 0 2 2h16' }],
  ['path', { d: 'm19 9-5 5-4-4-3 3' }],
];
const icGraphdb: IconNode = [
  ['path', { d: 'm10.586 5.414-5.172 5.172' }],
  ['path', { d: 'm18.586 13.414-5.172 5.172' }],
  ['path', { d: 'M6 12h12' }],
  ['circle', { cx: '12', cy: '20', r: '2' }],
  ['circle', { cx: '12', cy: '4', r: '2' }],
  ['circle', { cx: '20', cy: '12', r: '2' }],
  ['circle', { cx: '4', cy: '12', r: '2' }],
];
const icColdstorage: IconNode = [
  ['path', { d: 'm10 20-1.25-2.5L6 18' }],
  ['path', { d: 'M10 4 8.75 6.5 6 6' }],
  ['path', { d: 'm14 20 1.25-2.5L18 18' }],
  ['path', { d: 'm14 4 1.25 2.5L18 6' }],
  ['path', { d: 'm17 21-3-6h-4' }],
  ['path', { d: 'm17 3-3 6 1.5 3' }],
  ['path', { d: 'M2 12h6.5L10 9' }],
  ['path', { d: 'm20 10-1.5 2 1.5 2' }],
  ['path', { d: 'M22 12h-6.5L14 15' }],
  ['path', { d: 'm4 10 1.5 2L4 14' }],
  ['path', { d: 'm7 21 3-6-1.5-3' }],
  ['path', { d: 'm7 3 3 6h4' }],
];
const icVectordb: IconNode = [
  ['circle', { cx: '7.5', cy: '7.5', r: '.5', fill: 'currentColor' }],
  ['circle', { cx: '18.5', cy: '5.5', r: '.5', fill: 'currentColor' }],
  ['circle', { cx: '11.5', cy: '11.5', r: '.5', fill: 'currentColor' }],
  ['circle', { cx: '7.5', cy: '16.5', r: '.5', fill: 'currentColor' }],
  ['circle', { cx: '17.5', cy: '14.5', r: '.5', fill: 'currentColor' }],
  ['path', { d: 'M3 3v16a2 2 0 0 0 2 2h16' }],
];
const icWritebehind: IconNode = [
  ['path', { d: 'M12 2v8' }],
  ['path', { d: 'm16 6-4 4-4-4' }],
  ['rect', { width: '20', height: '8', x: '2', y: '14', rx: '2' }],
  ['path', { d: 'M6 18h.01' }],
  ['path', { d: 'M10 18h.01' }],
];
/* Messaging */
const icQueue: IconNode = [
  ['rect', { width: '18', height: '18', x: '3', y: '3', rx: '2' }],
  ['path', { d: 'M21 9H3' }],
  ['path', { d: 'M21 15H3' }],
];
const icStreambroker: IconNode = [
  ['path', { d: 'M3 5h1' }],
  ['path', { d: 'M3 12h1' }],
  ['path', { d: 'M3 19h1' }],
  ['path', { d: 'M8 5h1' }],
  ['path', { d: 'M8 12h1' }],
  ['path', { d: 'M8 19h1' }],
  ['path', { d: 'M13 5h8' }],
  ['path', { d: 'M13 12h8' }],
  ['path', { d: 'M13 19h8' }],
];
const icPubsub: IconNode = [
  ['circle', { cx: '18', cy: '5', r: '3' }],
  ['circle', { cx: '6', cy: '12', r: '3' }],
  ['circle', { cx: '18', cy: '19', r: '3' }],
  ['line', { x1: '8.59', x2: '15.42', y1: '13.51', y2: '17.49' }],
  ['line', { x1: '15.41', x2: '8.59', y1: '6.51', y2: '10.49' }],
];
const icWebsocket: IconNode = [
  [
    'path',
    { d: 'M17 19a1 1 0 0 1-1-1v-2a2 2 0 0 1 2-2h2a2 2 0 0 1 2 2v2a1 1 0 0 1-1 1z' },
  ],
  ['path', { d: 'M17 21v-2' }],
  ['path', { d: 'M19 14V6.5a1 1 0 0 0-7 0v11a1 1 0 0 1-7 0V10' }],
  ['path', { d: 'M21 21v-2' }],
  ['path', { d: 'M3 5V3' }],
  [
    'path',
    { d: 'M4 10a2 2 0 0 1-2-2V6a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2a2 2 0 0 1-2 2z' },
  ],
  ['path', { d: 'M7 5V3' }],
];
const icRetryqueue: IconNode = [
  ['path', { d: 'M21 5H3' }],
  ['path', { d: 'M7 12H3' }],
  ['path', { d: 'M7 19H3' }],
  [
    'path',
    {
      d: 'M12 18a5 5 0 0 0 9-3 4.5 4.5 0 0 0-4.5-4.5c-1.33 0-2.54.54-3.41 1.41L11 14',
    },
  ],
  ['path', { d: 'M11 10v4h4' }],
];
/* Control */
const icAutoscaler: IconNode = [
  ['path', { d: 'M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7' }],
  ['path', { d: 'M14 15H9v-5' }],
  ['path', { d: 'M16 3h5v5' }],
  ['path', { d: 'M21 3 9 15' }],
];
const icRatelimiter: IconNode = [
  [
    'path',
    {
      d: 'M10 20a1 1 0 0 0 .553.895l2 1A1 1 0 0 0 14 21v-7a2 2 0 0 1 .517-1.341L21.74 4.67A1 1 0 0 0 21 3H3a1 1 0 0 0-.742 1.67l7.225 7.989A2 2 0 0 1 10 14z',
    },
  ],
];
const icBreaker: IconNode = [
  ['path', { d: 'm19 5 3-3' }],
  ['path', { d: 'm2 22 3-3' }],
  [
    'path',
    { d: 'M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z' },
  ],
  ['path', { d: 'M7.5 13.5 10 11' }],
  ['path', { d: 'M10.5 16.5 13 14' }],
  [
    'path',
    { d: 'm12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z' },
  ],
];
const icApigateway: IconNode = [
  ['path', { d: 'M11 20H2' }],
  [
    'path',
    {
      d: 'M11 4.562v16.157a1 1 0 0 0 1.242.97L19 20V5.562a2 2 0 0 0-1.515-1.94l-4-1A2 2 0 0 0 11 4.561z',
    },
  ],
  ['path', { d: 'M11 4H8a2 2 0 0 0-2 2v14' }],
  ['path', { d: 'M14 12h.01' }],
  ['path', { d: 'M22 20h-3' }],
];
const icSidecar: IconNode = [
  [
    'path',
    {
      d: 'M10 22V7a1 1 0 0 0-1-1H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-5a1 1 0 0 0-1-1H2',
    },
  ],
  ['rect', { x: '14', y: '2', width: '8', height: '8', rx: '1' }],
];
const icBulkhead: IconNode = [
  ['path', { d: 'M8 19H5c-1 0-2-1-2-2V7c0-1 1-2 2-2h3' }],
  ['path', { d: 'M16 5h3c1 0 2 1 2 2v10c0 1-1 2-2 2h-3' }],
  ['line', { x1: '12', x2: '12', y1: '4', y2: '20' }],
];
const icLoadshedder: IconNode = [
  [
    'path',
    {
      d: 'M12.531 3H3a1 1 0 0 0-.742 1.67l7.225 7.989A2 2 0 0 1 10 14v6a1 1 0 0 0 .553.895l2 1A1 1 0 0 0 14 21v-7a2 2 0 0 1 .517-1.341l.427-.473',
    },
  ],
  ['path', { d: 'm16.5 3.5 5 5' }],
  ['path', { d: 'm21.5 3.5-5 5' }],
];

/**
 * The kind -> icon table. The reasoning for each pick is inline in the web
 * app's `nodeVisuals.ts`, because a future kind must be slotted into the
 * same system, not just given the first icon that looks nice.
 */
export const KIND_ICON: Record<NodeKind, IconNode> = {
  client: icClient,
  lb: icLb,
  service: icService,
  cache: icCache,
  db: icDb,
  queue: icQueue,
  worker: icWorker,
  replica: icReplica,
  shard: icShard,
  autoscaler: icAutoscaler,
  region: icRegion,
  cdn: icCdn,
  ratelimiter: icRatelimiter,
  breaker: icBreaker,
  objectstore: icObjectstore,
  searchindex: icSearchindex,
  timeseriesdb: icTimeseriesdb,
  graphdb: icGraphdb,
  coldstorage: icColdstorage,
  vectordb: icVectordb,
  streambroker: icStreambroker,
  pubsub: icPubsub,
  websocket: icWebsocket,
  apigateway: icApigateway,
  sidecar: icSidecar,
  lambda: icLambda,
  cron: icCron,
  bulkhead: icBulkhead,
  retryqueue: icRetryqueue,
  transcoder: icTranscoder,
  edgecompute: icEdgecompute,
  writebehind: icWritebehind,
  loadshedder: icLoadshedder,
};

/**
 * The viewBox every Lucide icon is drawn in, in icon units. Consumers scale
 * from this to their rendered size rather than the art being redrawn; one
 * number to change if the icon set is ever swapped again.
 */
export const ICON_BOX = 24;

/**
 * The stroke width the set is designed for, in icon units (so 2/24 of the
 * rendered size). Every consumer derives its stroke from this so all icons
 * everywhere carry the same visual weight.
 */
export const ICON_STROKE = 2;

/** Human-readable kind names. Used by the canvas, the palette and the inspector. */
export const KIND_NAME: Record<NodeKind, string> = {
  client: 'Client',
  lb: 'Load balancer',
  service: 'Service',
  cache: 'Cache',
  db: 'Database',
  queue: 'Queue',
  worker: 'Worker',
  replica: 'Read replicas',
  shard: 'Sharded store',
  autoscaler: 'Autoscaler',
  region: 'Region',
  cdn: 'CDN',
  ratelimiter: 'Rate limiter',
  breaker: 'Circuit breaker',
  objectstore: 'Object storage',
  searchindex: 'Search index',
  timeseriesdb: 'Time-series store',
  graphdb: 'Graph database',
  coldstorage: 'Cold storage',
  vectordb: 'Vector database',
  streambroker: 'Stream broker',
  pubsub: 'Pub/sub topic',
  websocket: 'WebSocket gateway',
  apigateway: 'API gateway',
  sidecar: 'Sidecar proxy',
  lambda: 'Lambda',
  cron: 'Cron job',
  bulkhead: 'Bulkhead',
  retryqueue: 'Retry queue',
  transcoder: 'Transcoder',
  edgecompute: 'Edge compute',
  writebehind: 'Write-behind cache',
  loadshedder: 'Load shedder',
};

/**
 * Glossary term key for each component kind. Lives beside KIND_NAME because
 * it is the same fact about a kind: what it is called, and what glossary
 * entry explains it. Ported for parity with the web app's `KIND_TERM`; the
 * desktop app's glossary panel (if/when ported) reads it the same way.
 */
export const KIND_TERM: Record<NodeKind, string> = {
  client: 'client',
  lb: 'load-balancer',
  service: 'service',
  cache: 'cache',
  db: 'database',
  queue: 'queue',
  worker: 'worker',
  replica: 'read-replica',
  shard: 'shard',
  autoscaler: 'autoscaler',
  region: 'region',
  cdn: 'cdn',
  ratelimiter: 'rate-limiter',
  breaker: 'breaker',
  objectstore: 'objectstore',
  searchindex: 'searchindex',
  timeseriesdb: 'timeseriesdb',
  graphdb: 'graphdb',
  coldstorage: 'coldstorage',
  vectordb: 'vectordb',
  streambroker: 'streambroker',
  pubsub: 'pubsub',
  websocket: 'websocket',
  apigateway: 'apigateway',
  sidecar: 'sidecar',
  lambda: 'lambda',
  cron: 'cron',
  bulkhead: 'bulkhead',
  retryqueue: 'retryqueue',
  transcoder: 'transcoder',
  edgecompute: 'edgecompute',
  writebehind: 'writebehind',
  loadshedder: 'loadshedder',
};

/* ================================================================== *
 * UNIT RENDERING -- how a node made of several things is drawn
 *
 * `NodeStats.instances` / `perInstance` say what a node is MADE OF: a
 * service scaled to 5 is five machines, a shard with 8 partitions is eight
 * independent units, a replica set is a primary plus N copies. These helpers
 * are the geometry that lets the canvas SHOW it: pure functions of numbers,
 * testable without mounting a component.
 *
 * Nothing here chooses a colour. Every function returns positions, counts and
 * fractions; the palette is applied in CSS from tokens.
 * ================================================================== */

/**
 * Most layered cards ever drawn in an instance stack.
 *
 * Past this the stack stops growing and the BADGE carries the true count.
 * This is not a rendering-cost limit -- it is a legibility one, and the
 * number is forced by the node's own geometry rather than picked for looks.
 * Each layer is offset by STACK_STEP px; at 5 layers the stack is 4 * 3 =
 * 12px deep, which fits in the margin the node already reserves above its
 * top edge without touching the row above it in any preset (the tightest
 * vertical gap in the preset set is 52px, see NODE_H in geometry.ts).
 *
 * The consequence is deliberate and worth stating: past 5 the stack is a
 * SYMBOL meaning "several", not a tally. The count badge is the precise
 * channel, and it is always present when instances > 1.
 */
export const STACK_MAX_LAYERS = 5;

/** Offset between successive cards in the stack, in world px, on the 4px scale. */
export const STACK_STEP = 3;

/**
 * How an instance stack should be drawn for a node with `live` serving units
 * and `pending` units still booting.
 *
 * Returns layer offsets from BACK to FRONT, so the consumer paints them in
 * array order and the front card lands last, on top. The front card is the
 * node body itself (offset 0) and is NOT included here: these are only the
 * cards that peek out BEHIND it, which is what makes a stack read as depth
 * rather than as a taller box.
 *
 * Pending units are drawn as extra layers behind the live ones and are
 * flagged so the consumer can ghost them. That ordering is the honest one: a
 * booting machine is not yet carrying traffic, so it sits behind the ones
 * that are, and the student watches it move forward when it lands.
 */
export interface StackLayer {
  /** Distance back from the front card, in world px. */
  offset: number;
  /** True when this layer is a unit that is still warming up. */
  pending: boolean;
}

export function stackLayers(live: number, pending: number): StackLayer[] {
  const liveN = Number.isFinite(live) ? Math.max(0, Math.floor(live)) : 0;
  const pendN = Number.isFinite(pending) ? Math.max(0, Math.floor(pending)) : 0;
  // The front card is the node body, so only liveN - 1 live cards are drawn
  // behind it. A single instance with nothing booting draws no stack at all,
  // which is the point: one machine must look like one box.
  const behind = Math.max(0, liveN - 1);
  const total = Math.min(STACK_MAX_LAYERS - 1, behind + pendN);
  if (total <= 0) return [];

  // Pending layers always get shown if there are any: a warm-up that is
  // invisible because the live count already filled the budget would hide the
  // exact thing the stack exists to teach. So pending claims its share first,
  // capped at the budget, and live takes what is left.
  const pendShown = Math.min(pendN, total);

  const out: StackLayer[] = [];
  // Back to front: pending sits furthest back, then live cards.
  for (let i = 0; i < total; i += 1) {
    const depth = total - i; // total..1, so the last one is nearest the front
    out.push({ offset: depth * STACK_STEP, pending: i < pendShown });
  }
  return out;
}

/**
 * The count badge text for a node of `n` units, or null when no badge is
 * warranted.
 *
 * One unit gets NO badge. A "1x" on every unscaled service would put a chip
 * on almost every node in every diagram, which trains the eye to ignore the
 * badge exactly when it starts to matter.
 */
export function stackBadge(n: number | undefined): string | null {
  if (n === undefined || !Number.isFinite(n)) return null;
  const v = Math.floor(n);
  return v > 1 ? `${v}x` : null;
}

/**
 * Layout for a strip of per-unit cells (a shard's partitions, a replica set).
 *
 * The hard requirement is 1..64 cells inside a fixed width while every cell
 * stays a readable object. Two regimes, and the switch between them is a
 * measured threshold rather than a guess:
 *
 *   CELLS   at or below `maxCells`, each partition is its own rect with a
 *           visible gap. This is the regime where a student can point at one
 *           cell and say "that shard is the hot one".
 *
 *   DENSE   above it, gaps are dropped and cells become adjacent columns of a
 *           continuous band. At 64 partitions in 160px a 1px gap would eat
 *           40% of the width and every cell would be sub-pixel; without gaps
 *           each column is 2.5px, which still resolves as a distinct bar.
 *
 * Either way the returned geometry is exact and the caller just draws rects.
 */
export interface CellStrip {
  /** x offset of cell i, in world px from the strip's left edge. */
  x: (i: number) => number;
  /** Width of one cell, in world px. Always > 0. */
  w: number;
  /** True when gaps were dropped because the count is high. */
  dense: boolean;
}

/** Below this many cells the strip keeps visible gaps between partitions. */
export const STRIP_GAP_MAX = 24;

export function cellStrip(count: number, width: number, gap = 1.5): CellStrip {
  const n = Math.max(1, Math.floor(count));
  const dense = n > STRIP_GAP_MAX;
  const g = dense ? 0 : gap;
  // Width per slot including its gap; the last cell has no trailing gap, so
  // the total comes out exactly `width` for any n.
  const slot = (width + g) / n;
  const w = Math.max(0.75, slot - g);
  return { x: (i: number) => i * slot, w, dense };
}

/* ------------------------------------------------------------------ *
 * The kind taxonomy.
 *
 * Thirty-three kinds listed flat is a wall of names nobody reads. Grouped,
 * each group answers one question, so a student looking for "how do I stop
 * this melting" goes straight to Control without reading the other names.
 * It is a lookup index, not decoration.
 * ------------------------------------------------------------------ */

export interface KindGroup {
  id: string;
  title: string;
  kinds: NodeKind[];
}

export const KIND_GROUPS: KindGroup[] = [
  { id: 'traffic', title: 'Traffic', kinds: ['client', 'lb', 'cdn', 'edgecompute'] },
  {
    id: 'compute',
    title: 'Compute',
    kinds: ['service', 'worker', 'queue', 'retryqueue', 'transcoder'],
  },
  {
    id: 'data',
    title: 'Data',
    kinds: ['db', 'cache', 'writebehind', 'replica', 'shard'],
  },
  {
    // The polyglot-persistence shelf: each of these exists because putting
    // its workload in the main database is the mistake it teaches against.
    id: 'stores',
    title: 'Specialised stores',
    kinds: [
      'objectstore',
      'searchindex',
      'timeseriesdb',
      'graphdb',
      'vectordb',
      'coldstorage',
    ],
  },
  {
    // How services talk when it is not one request calling one server:
    // logs, topics, sockets, functions and jobs on a clock.
    id: 'messaging',
    title: 'Messaging',
    kinds: ['streambroker', 'pubsub', 'websocket', 'lambda', 'cron'],
  },
  {
    id: 'control',
    title: 'Control',
    kinds: [
      'ratelimiter',
      'loadshedder',
      'breaker',
      'bulkhead',
      'autoscaler',
      'region',
      'apigateway',
      'sidecar',
    ],
  },
];

/** Which group a kind belongs to, resolved once rather than scanned per call. */
const GROUP_OF_KIND = new Map<NodeKind, KindGroup>();
for (const group of KIND_GROUPS) {
  for (const kind of group.kinds) GROUP_OF_KIND.set(kind, group);
}

export function groupOfKind(kind: NodeKind): KindGroup | undefined {
  return GROUP_OF_KIND.get(kind);
}
