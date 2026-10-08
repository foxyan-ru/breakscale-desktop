/**
 * Data-driven config schema for the Inspector panel.
 *
 * The web app's `src/components/Inspector.tsx` (3,260 lines) hand-writes a
 * JSX field list per `NodeKind` inside one giant switch. This module is the
 * same knowledge -- which `NodeConfig` fields apply to which of the 33
 * kinds, and what control/range/unit each field renders with -- expressed
 * as data instead, so `Inspector.svelte` can render it with one generic
 * loop (per MIGRATION_PLAN.md's Inspector build note).
 *
 * SOURCE OF TRUTH. `FIELDS_BY_KIND` and every min/max/step/unit below is
 * transcribed directly from the web app's own `FIELDS_BY_KIND`,
 * `FIELD_SPECS` and `KIND_FIELD_OVERRIDES` tables (`src/components/
 * Inspector.tsx` lines ~107-1108) -- NOT re-derived by guessing from
 * `sim-types.ts` doc comments or `presets.ts` defaults. Those ranges are
 * the ones an instructor actually tuned for the teaching tool, so they are
 * strictly more trustworthy than a fresh guess would be. The doc comments
 * in `desktop/src/lib/domain/sim-types.ts` were used to write the `hint`
 * text for the handful of fields the web app left unexplained (`minCapacity`,
 * `maxCapacity`, `windowMs`, `batchSize`, `authFailRate`, `activeRegion`,
 * `regions`, `halfOpenProbes`, `indexSizeK`, `coldStartMs`, `intervalMs`),
 * and to write the two fields below that the web app's Inspector never
 * exposed at all (`traffic`, `trafficPeriodS` -- see the module doc on
 * `TRAFFIC_FIELDS` below).
 *
 * ONE DELIBERATE DEVIATION from the web app: `instances` is listed in the
 * web app's `FIELDS_BY_KIND` for several kinds but is then silently dropped
 * because `FIELD_GROUPS` (the thing that actually partitions what renders)
 * never claims it -- the web app only ever shows it read-only, via a
 * separate fleet-visualisation panel, never as an editable control. That
 * looks like an oversight rather than a decision (the field is real, has a
 * documented meaning and a default, and every other field on the list gets
 * a control). Since this port is explicitly data-driven rather than a
 * line-for-line copy, `instances` gets a real editable control here,
 * grouped under "How much it can handle" alongside `capacity`.
 */

import type { NodeConfig, NodeKind, NodeStats, SimNode, TrafficPattern } from '$lib/domain';
import { TRAFFIC_PATTERNS } from '$lib/domain';

/* ------------------------------------------------------------------ *
 * Kind display names.
 *
 * `canvas/geometry.ts` does not exist yet in this checkout (the canvas
 * agent's directory is still empty), so per the task brief this is a local
 * fallback rather than an `import type`. Values copied verbatim from the
 * web app's `KIND_NAME` (`src/components/nodeVisuals.ts` lines 233-267) so
 * the wording matches once the canvas module lands and this can be
 * replaced with a shared import.
 * ------------------------------------------------------------------ */
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
 * Kinds that hold no work of their own: they admit or refuse and pass
 * through, so their in-flight/queued/throughput counts are structurally
 * zero and must not be printed as if they were a live reading. Transcribed
 * from the web app's `GATE_KINDS` (`src/components/Inspector.tsx` line
 * ~335), which exists for exactly the reason AGENTS.md states for the
 * simulator itself: "a component with no meaningful value for a metric
 * shows something else, or nothing, never a plausible-looking number."
 */
export const GATE_KINDS: ReadonlySet<NodeKind> = new Set<NodeKind>([
  'ratelimiter',
  'breaker',
  'loadshedder',
  'bulkhead',
]);

/**
 * Kinds whose throughput ceiling is `instances x capacity x (1000 /
 * serviceMs)` -- "spare capacity" (headroom) is only ever a defined concept
 * for these, and they are therefore the only kinds the Inspector's suggested
 * fix (`$lib/content/suggestions.ts`) is ever offered for. Transcribed from
 * the web app's `HAS_THROUGHPUT_CEILING` (`src/components/Inspector.tsx`
 * lines ~342-361, upstream `781b7258`), which the suggestion feature itself
 * (`eb163673`, PR #71) is gated on.
 */
export const HAS_THROUGHPUT_CEILING: ReadonlySet<NodeKind> = new Set<NodeKind>([
  'lb',
  'service',
  'cache',
  'db',
  'worker',
  'objectstore',
  'coldstorage',
  'retryqueue',
  'transcoder',
  'edgecompute',
  'apigateway',
  'sidecar',
]);

/**
 * Spare capacity as a multiple of what is arriving: >= 1 means the node can
 * keep up, < 1 means it cannot. Null when the kind has no throughput
 * ceiling, when `serviceMs` is zero (no slot cost defined), or when nothing
 * has arrived yet -- an idle node is not "out of headroom". Field-for-field
 * port of the web app's own derivation (`Inspector.tsx` lines 2487-2509,
 * upstream `781b7258`): `fleet` prefers the engine's live instance count
 * over the configured one so a mid-scale autoscale is reflected the moment
 * it lands, the same reasoning `liveReadout`'s `watchedInstances` case
 * documents below. `suggestionFor` (PR #71, `eb163673`) is gated on exactly
 * this dropping below 1.
 */
export function headroomFor(n: SimNode, stats: NodeStats | null): number | null {
  if (!HAS_THROUGHPUT_CEILING.has(n.kind) || n.config.serviceMs <= 0) return null;
  const fleet = Math.max(1, Math.floor(stats?.instances ?? n.config.instances ?? 1));
  const maxThroughput = fleet * n.config.capacity * (1000 / n.config.serviceMs);
  const arrivals = stats?.arrivalRate ?? 0;
  return arrivals > 0 && maxThroughput > 0 ? maxThroughput / arrivals : null;
}

/** One plain-language sentence per kind, shown under the node name. */
export const KIND_BLURB: Record<NodeKind, string> = {
  client: 'Generates load and waits for an answer. Serves nothing itself.',
  lb: 'Forwards requests to whatever is wired downstream.',
  service: 'The general workhorse: takes slots, does work, can fail or time out.',
  cache: 'Answers a share of reads for free; the rest fall through downstream.',
  db: 'Holds data. Writes serialise against each other; reads do not.',
  queue: 'A buffer. Holds work until something drains it.',
  worker: 'Drains a queue, one job at a time per free slot.',
  replica: 'A primary plus read-only copies. Reads may be served stale.',
  shard: 'Data split across partitions by key. A hot key overloads one shard.',
  autoscaler: 'Watches one node and adds or removes its instances.',
  region: 'A failover switch across N downstream regions.',
  cdn: 'An edge cache in front of everything. A miss costs a round trip.',
  ratelimiter: 'A token bucket gating a downstream by sustained rate and burst.',
  breaker: 'Trips open on a bad downstream and fails fast instead of calling it.',
  objectstore: 'Blob storage. Each key prefix has its own rate ceiling.',
  searchindex: 'Cheap searches, surcharged writes, and a refresh lag between them.',
  timeseriesdb: 'Cheap appends; range queries cost extra.',
  graphdb: 'Traversal cost multiplies with every extra hop of depth.',
  coldstorage: 'A deliberately slow, thin archive tier.',
  vectordb: 'Embedding search. Cost trades against the recall target.',
  streambroker: 'A partitioned, replayable log with independent consumer groups.',
  pubsub: 'Fans one publish out to every subscriber edge.',
  websocket: 'Holds connections open; capacity is connections, not requests.',
  apigateway: 'The front door: rate limiting and auth in one place.',
  sidecar: 'A proxy that ejects an upstream after consecutive failures.',
  lambda: 'No fleet to size: cold starts and a concurrency ceiling instead.',
  cron: 'Fires on a schedule and dumps a batch downstream each time.',
  bulkhead: 'Caps in-flight calls to one dependency so it cannot sink everything.',
  retryqueue: 'Retries failed deliveries with a dead-letter shelf at the end.',
  transcoder: 'An encode farm; each finished job hands off a quality ladder.',
  edgecompute: 'Answers a share of requests at the edge under a CPU budget.',
  writebehind: 'Acknowledges a write before it reaches the backing store.',
  loadshedder: 'A token bucket that refuses low-priority traffic first.',
};

/* ------------------------------------------------------------------ *
 * Field specs
 * ------------------------------------------------------------------ */

export type ControlKind = 'slider' | 'number' | 'percent' | 'enum';

interface FieldSpecBase {
  label: string;
  /** Short suffix shown beside the value, e.g. "ms", "req/s", "instances". */
  unit?: string;
  /** One sentence of plain-language explanation, shown under the control. */
  hint?: string;
}

export interface RangeFieldSpec extends FieldSpecBase {
  control: 'slider' | 'number' | 'percent';
  min: number;
  max: number;
  step: number;
  /** Shown instead of the formatted number when the value is exactly 0. */
  zeroLabel?: string;
}

export interface EnumFieldSpec extends FieldSpecBase {
  control: 'enum';
  options: readonly { value: string; label: string }[];
}

export type FieldSpec = RangeFieldSpec | EnumFieldSpec;

/**
 * Every `NodeConfig` field the Inspector knows how to render a control for.
 * Aliased to `keyof NodeConfig` (rather than a hand-written union) so that
 * `FIELD_SPECS` below is a `Record`, not a `Partial<Record>`: TypeScript
 * then refuses to compile if a field is ever added to `NodeConfig` without
 * a matching spec here, the same completeness guarantee the web app's own
 * `FIELDS_BY_KIND` assertion was written to enforce at runtime.
 */
export type InspectorField = keyof NodeConfig;

const slider = (spec: Omit<RangeFieldSpec, 'control'>): RangeFieldSpec => ({
  control: 'slider',
  ...spec,
});
const number = (spec: Omit<RangeFieldSpec, 'control'>): RangeFieldSpec => ({
  control: 'number',
  ...spec,
});
const percent = (spec: Omit<RangeFieldSpec, 'control' | 'unit'>): RangeFieldSpec => ({
  control: 'percent',
  ...spec,
});

/**
 * Base spec per field, transcribed from the web app's `FIELD_SPECS`
 * (`src/components/Inspector.tsx` lines 500-1057). See the module doc for
 * what "transcribed" vs "written for this port" means per field.
 */
export const FIELD_SPECS: Record<InspectorField, FieldSpec> = {
  rps: slider({
    label: 'Offered load',
    unit: 'req/s',
    min: 1,
    max: 5000,
    step: 1,
    hint: 'Requests per second this client sends into the system.',
  }),
  traffic: {
    control: 'enum',
    label: 'Traffic pattern',
    hint: 'How the offered rate varies over time. It scales the baseline above; it never replaces it.',
    options: TRAFFIC_PATTERNS.map((p: TrafficPattern) => ({
      value: p,
      label: p === 'steady' ? 'Steady' : p === 'ramp' ? 'Ramp up' : p === 'spike' ? 'Spike' : 'Diurnal (day/night)',
    })),
  },
  trafficPeriodS: number({
    label: 'Seconds per cycle',
    unit: 's',
    min: 5,
    max: 600,
    step: 5,
    hint: 'One full cycle of the pattern above: a whole ramp, or the gap between two spikes.',
  }),
  instances: number({
    label: 'Instances',
    unit: 'machines',
    min: 1,
    max: 512,
    step: 1,
    hint: 'How many copies of this component are running. This is what an autoscaler adds and removes.',
  }),
  capacity: number({
    label: 'Slots per instance',
    unit: 'at once',
    min: 1,
    max: 4096,
    step: 1,
    hint: 'How many requests ONE machine handles at a time. Total parallelism is instances x this.',
  }),
  serviceMs: slider({
    label: 'Service time',
    unit: 'ms',
    min: 0.1,
    max: 500,
    step: 0.1,
  }),
  serviceCv: slider({
    label: 'How uneven the work is',
    unit: 'CV',
    min: 0,
    max: 2,
    step: 0.05,
    zeroLabel: 'every one the same',
    hint: '0 means every request takes exactly the mean; 1 means exponentially distributed.',
  }),
  queueLimit: number({
    label: 'Most that can wait in line',
    unit: 'requests',
    min: 0,
    max: 20000,
    step: 1,
  }),
  hitRate: percent({ label: 'Hit rate you set', min: 0, max: 1, step: 0.01 }),
  errorRate: percent({ label: 'Error rate', min: 0, max: 1, step: 0.005 }),
  timeoutMs: slider({
    label: 'Timeout',
    unit: 'ms',
    min: 0,
    max: 5000,
    step: 10,
    zeroLabel: 'none',
  }),
  retries: number({ label: 'Retries', unit: 'extra tries', min: 0, max: 10, step: 1 }),
  replicaCount: number({
    label: 'Read replicas',
    unit: 'copies',
    min: 1,
    max: 64,
    step: 1,
    hint: 'Read-only copies behind the primary. They add READ capacity only; every write still goes through the one primary.',
  }),
  replicationLagMs: slider({
    label: 'Replication lag',
    unit: 'ms',
    min: 0,
    max: 2000,
    step: 5,
    zeroLabel: 'synchronous',
  }),
  readFraction: percent({ label: 'Reads, as a share of traffic', min: 0, max: 1, step: 0.01 }),
  shardCount: number({
    label: 'Shards',
    unit: 'partitions',
    min: 1,
    max: 64,
    step: 1,
    hint: 'How many partitions the data is split across. A key goes to exactly one of them.',
  }),
  shardCapacity: number({
    label: 'Slots per shard',
    unit: 'at once',
    min: 1,
    max: 512,
    step: 1,
    hint: 'How many requests ONE partition handles at a time. A hot key can only ever use its own shard’s slots.',
  }),
  hotKeyFraction: percent({
    label: 'Traffic hitting one hot key',
    min: 0,
    max: 1,
    step: 0.01,
    zeroLabel: 'even',
  }),
  targetUtil: percent({ label: 'Keep utilisation near', min: 0.1, max: 0.95, step: 0.05 }),
  minCapacity: number({
    label: 'Never scale below',
    unit: 'instances',
    min: 1,
    max: 512,
    step: 1,
    hint: 'Fewest instances the controller will leave running.',
  }),
  maxCapacity: number({
    label: 'Never scale above',
    unit: 'instances',
    min: 1,
    max: 512,
    step: 1,
    hint: 'Most instances the controller will run.',
  }),
  cooldownMs: slider({
    label: 'Wait between changes',
    unit: 'ms',
    min: 0,
    max: 30000,
    step: 250,
  }),
  scaleStepPct: percent({
    label: 'Change the fleet by',
    min: 0.05,
    max: 1,
    step: 0.05,
  }),
  warmupMs: slider({
    label: 'New capacity takes',
    unit: 'ms',
    min: 0,
    max: 60000,
    step: 500,
    zeroLabel: 'instant',
  }),
  regions: number({
    label: 'Regions',
    unit: 'regions',
    min: 1,
    max: 8,
    step: 1,
    hint: 'How many outgoing edges are treated as regions. Edge index i is region i.',
  }),
  activeRegion: number({
    label: 'Serving from region',
    unit: '#',
    min: 0,
    max: 7,
    step: 1,
    hint: '0-based index of the region currently serving traffic.',
  }),
  failoverMs: slider({
    label: 'Failover takes',
    unit: 'ms',
    min: 0,
    max: 60000,
    step: 500,
    zeroLabel: 'instant',
  }),
  rateLimitRps: slider({
    label: 'Rate limit',
    unit: 'req/s',
    min: 0,
    max: 2000,
    step: 5,
    zeroLabel: 'unlimited',
  }),
  burst: number({
    label: 'Burst allowance',
    unit: 'requests',
    min: 1,
    max: 5000,
    step: 1,
    hint: 'The largest burst admitted instantly from an idle bucket, and the bucket’s refill ceiling.',
  }),
  errorThreshold: percent({ label: 'Trip when errors reach', min: 0, max: 1, step: 0.05 }),
  windowMs: slider({
    label: 'Measured over',
    unit: 'ms',
    min: 200,
    max: 30000,
    step: 100,
    hint: 'The trailing window the downstream error rate is measured over.',
  }),
  openMs: slider({ label: 'Stay open for', unit: 'ms', min: 100, max: 60000, step: 100 }),
  halfOpenProbes: number({
    label: 'Test requests before closing',
    unit: 'requests',
    min: 1,
    max: 50,
    step: 1,
    hint: 'Probe requests allowed through while half-open. One failure re-opens the circuit immediately.',
  }),
  indexMs: slider({
    label: 'Indexing cost per write',
    unit: 'ms extra',
    min: 0,
    max: 500,
    step: 5,
    zeroLabel: 'free',
    hint: 'What a write pays ON TOP of the service time. Searches never pay it.',
  }),
  indexLagMs: slider({
    label: 'Searchable after',
    unit: 'ms',
    min: 0,
    max: 10000,
    step: 100,
    zeroLabel: 'instantly',
    hint: 'How long after a write commits before searches can see it.',
  }),
  rangeQueryFraction: percent({
    label: 'Traffic that is range queries',
    min: 0,
    max: 1,
    step: 0.01,
    zeroLabel: 'appends only',
  }),
  rangeQueryMs: slider({
    label: 'Range query costs',
    unit: 'ms extra',
    min: 0,
    max: 2000,
    step: 10,
    zeroLabel: 'free',
    hint: 'What a range query pays ON TOP of the service time. Appends never pay it.',
  }),
  traversalDepth: number({
    label: 'Traversal depth',
    unit: 'hops',
    min: 1,
    max: 6,
    step: 1,
    hint: 'Every extra hop multiplies the work by about 3, so depth 3 costs 9x depth 1.',
  }),
  indexSizeK: slider({
    label: 'Index size',
    unit: 'k vectors',
    min: 1,
    max: 100000,
    step: 1,
    hint: 'Size of the vector index in thousands of embeddings. Query cost grows with its log2.',
  }),
  recallTarget: percent({
    label: 'Recall target',
    min: 0.5,
    max: 0.99,
    step: 0.01,
    hint: 'How close to perfect the search must be. Cost scales with 1 / (1 - recall).',
  }),
  partitions: number({
    label: 'Partitions',
    unit: 'partitions',
    min: 1,
    max: 64,
    step: 1,
    hint: 'A message lands in one partition by key; a consumer group takes at most one message per partition at a time.',
  }),
  connectionMs: slider({
    label: 'Connection lifetime',
    unit: 'ms held',
    min: 500,
    max: 120000,
    step: 500,
    hint: 'How long each accepted connection occupies a slot. Held connections settle at rate x lifetime.',
  }),
  authFailRate: percent({
    label: 'Failing auth',
    min: 0,
    max: 0.5,
    step: 0.005,
    hint: 'Fraction of requests refused as unauthorized, rolled after the rate limiter.',
  }),
  outlierAfter: number({
    label: 'Eject after',
    unit: 'fails in a row',
    min: 1,
    max: 50,
    step: 1,
    hint: 'Consecutive downstream failures before the proxy stops calling it for a while.',
  }),
  coldStartMs: slider({
    label: 'Cold start costs',
    unit: 'ms extra',
    min: 0,
    max: 5000,
    step: 25,
    zeroLabel: 'free',
    hint: 'Extra latency paid when a request arrives with no warm instance idle.',
  }),
  keepWarmMs: slider({
    label: 'Instances stay warm',
    unit: 'ms',
    min: 0,
    max: 60000,
    step: 500,
    zeroLabel: 'reclaim at once',
  }),
  maxConcurrency: number({
    label: 'Concurrency cap',
    unit: 'at once',
    min: 1,
    max: 1000,
    step: 1,
    hint: 'Past this, requests are throttled immediately; a lambda has no queue to wait in.',
  }),
  intervalMs: slider({
    label: 'Fires every',
    unit: 'ms',
    min: 1000,
    max: 120000,
    step: 1000,
    hint: 'Time between firings. Each firing dumps a batch down every outgoing edge at once.',
  }),
  batchSize: number({
    label: 'Requests per firing',
    unit: 'per edge',
    min: 1,
    max: 2000,
    step: 1,
    hint: 'Requests emitted per outgoing edge each time the job fires, effectively all at once.',
  }),
  bulkheadMax: number({
    label: 'Calls in flight, at most',
    unit: 'in flight',
    min: 1,
    max: 512,
    step: 1,
    hint: 'The pool. Requests arriving with it full are refused immediately.',
  }),
  // Port of the web app's `ChoiceSpec`/`bulkheadMode` field (upstream
  // `351327c4`, PR #77), rendered through this app's existing `'enum'`
  // control kind (the same one `traffic` uses) rather than a new control
  // type -- see Inspector.svelte's generic grouped-field loop.
  bulkheadMode: {
    control: 'enum',
    label: 'When the pool is full',
    hint: 'Reject fails immediately. Wait holds a request in a bounded queue until a slot opens, or its own acquire timeout elapses.',
    options: [
      { value: 'reject', label: 'Reject' },
      { value: 'wait', label: 'Wait' },
    ],
  },
  acquireQueueMax: number({
    label: 'Waiting acquires, at most',
    unit: 'requests',
    min: 0,
    max: 10000,
    step: 1,
    hint: 'Only matters in Wait mode. Beyond this many waiters, a request is refused immediately instead of joining the queue.',
  }),
  acquireTimeoutMs: slider({
    label: 'Acquire timeout',
    unit: 'ms',
    min: 0,
    max: 30000,
    step: 50,
    hint: 'Only matters in Wait mode. Longest a waiting request sits before it fails as an acquire timeout instead of getting a slot.',
  }),
  flushDelayMs: slider({
    label: 'Writes sit dirty for',
    unit: 'ms',
    min: 0,
    max: 10000,
    step: 50,
    zeroLabel: 'instant flush',
    hint: 'Time between the ack and the flush landing. Everything inside this window is lost if the node crashes.',
  }),
  edgeShare: percent({ label: 'Answered at the edge', min: 0, max: 1, step: 0.01 }),
  lowPriorityShare: percent({
    label: 'Low-priority traffic',
    min: 0,
    max: 1,
    step: 0.01,
    hint: 'Derived from the request key, so the same caller is always the same priority.',
  }),
  priorityReserve: percent({
    label: 'Reserved for high priority',
    min: 0,
    max: 0.9,
    step: 0.05,
    hint: 'Low-priority requests must leave this share of tokens untouched, which is why they are dropped first.',
  }),
  lockMs: slider({
    label: 'Lock wait per concurrent write',
    unit: 'ms each',
    min: 0,
    max: 100,
    step: 1,
    zeroLabel: 'no contention',
    hint: 'Each write entering service waits this long for every write already in flight. Fleet size does not appear in that sentence, which is the lesson.',
  }),
  prefixRps: slider({
    label: 'Ceiling per key prefix',
    unit: 'req/s',
    min: 0,
    max: 1000,
    step: 10,
    zeroLabel: 'no limit',
    hint: 'Keys map onto 8 prefixes. Evenly spread traffic sustains 8x this; a hot prefix gets exactly 1x.',
  }),
  renditions: number({
    label: 'Renditions per job',
    unit: 'files/job',
    min: 1,
    max: 12,
    step: 1,
    hint: 'The quality ladder. Storage behind the farm sees this many uploads per finished job.',
  }),
  cpuMsCap: slider({
    label: 'CPU budget per request',
    unit: 'ms',
    min: 0,
    max: 50,
    step: 0.5,
    zeroLabel: 'no budget',
    hint: 'A request that runs past this is killed at the edge and sent to the origin instead.',
  }),
};

/**
 * Per-kind overrides of a shared field's wording, e.g. `capacity` means
 * "held connections" on a websocket gateway, not "request slots". Only
 * wording changes here, never bounds -- transcribed from the web app's
 * `KIND_FIELD_OVERRIDES` (`src/components/Inspector.tsx` lines 1073-1108).
 */
const KIND_FIELD_OVERRIDES: Partial<Record<NodeKind, Partial<Record<InspectorField, Partial<FieldSpec>>>>> = {
  websocket: {
    capacity: {
      label: 'Connections per instance',
      unit: 'held at once',
      hint: 'A slot here is a held connection, not a request in service. Held connections settle at connect rate x lifetime.',
    },
  },
  writebehind: {
    capacity: {
      label: 'Dirty writes held',
      unit: 'buffered',
      hint: 'The buffer is memory: how many acknowledged writes may sit dirty at once. Not a thread count.',
    },
  },
  streambroker: {
    queueLimit: {
      label: 'Retention',
      unit: 'messages/partition',
      hint: 'How far back the log keeps messages. A consumer group that falls further behind than this skips ahead, losing the skipped messages.',
    },
  },
};

/** The spec actually rendered for `field` on a node of `kind`. */
export function specFor(kind: NodeKind, field: InspectorField): FieldSpec {
  const base = FIELD_SPECS[field];
  const patch = KIND_FIELD_OVERRIDES[kind]?.[field];
  return patch ? ({ ...base, ...patch } as FieldSpec) : base;
}

/* ------------------------------------------------------------------ *
 * Which fields apply to which kind.
 *
 * Transcribed field-for-field from the web app's `FIELDS_BY_KIND`
 * (`src/components/Inspector.tsx` lines 107-328), with `traffic` and
 * `trafficPeriodS` added to `client`. The web app's own Inspector and
 * `App.tsx` never wired those two into any control (grep confirms no
 * `TrafficPattern`/`trafficPeriodS` reference outside `sim/*` and its
 * tests) even though `NodeConfig` and the engine have supported them since
 * the traffic-pattern feature landed -- an apparent gap, not a decision.
 * The task brief for this port explicitly asks for a client traffic
 * section covering exactly these three fields, so they are included here.
 * ------------------------------------------------------------------ */
export const FIELDS_BY_KIND: Record<NodeKind, InspectorField[]> = {
  client: ['rps', 'traffic', 'trafficPeriodS', 'timeoutMs', 'retries'],
  lb: ['instances', 'capacity', 'serviceMs', 'queueLimit'],
  service: [
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
    'errorRate',
    'timeoutMs',
    'retries',
  ],
  cache: ['hitRate', 'instances', 'capacity', 'serviceMs', 'serviceCv', 'queueLimit'],
  db: [
    'readFraction',
    'lockMs',
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
    'errorRate',
  ],
  queue: ['queueLimit', 'serviceMs'],
  worker: ['instances', 'capacity', 'serviceMs', 'serviceCv', 'errorRate'],
  replica: [
    'replicaCount',
    'replicationLagMs',
    'readFraction',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
  ],
  shard: ['shardCount', 'shardCapacity', 'hotKeyFraction', 'serviceMs', 'serviceCv', 'queueLimit'],
  autoscaler: ['targetUtil', 'minCapacity', 'maxCapacity', 'cooldownMs', 'scaleStepPct', 'warmupMs'],
  region: ['regions', 'activeRegion', 'failoverMs'],
  cdn: ['hitRate', 'instances', 'capacity', 'serviceMs', 'queueLimit'],
  ratelimiter: ['rateLimitRps', 'burst'],
  breaker: ['errorThreshold', 'windowMs', 'openMs', 'halfOpenProbes'],
  objectstore: ['prefixRps', 'capacity', 'serviceMs', 'serviceCv', 'queueLimit', 'errorRate'],
  searchindex: [
    'readFraction',
    'indexMs',
    'indexLagMs',
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
  ],
  timeseriesdb: [
    'rangeQueryFraction',
    'rangeQueryMs',
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
  ],
  graphdb: ['traversalDepth', 'instances', 'capacity', 'serviceMs', 'serviceCv', 'queueLimit'],
  coldstorage: ['capacity', 'serviceMs', 'serviceCv', 'queueLimit'],
  vectordb: [
    'indexSizeK',
    'recallTarget',
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'queueLimit',
  ],
  streambroker: ['partitions', 'queueLimit', 'serviceMs'],
  pubsub: ['serviceMs'],
  websocket: ['connectionMs', 'instances', 'capacity', 'serviceMs', 'serviceCv'],
  apigateway: ['rateLimitRps', 'burst', 'authFailRate', 'instances', 'capacity', 'serviceMs', 'queueLimit'],
  sidecar: [
    'outlierAfter',
    'openMs',
    'retries',
    'timeoutMs',
    'instances',
    'capacity',
    'serviceMs',
    'queueLimit',
  ],
  lambda: ['coldStartMs', 'keepWarmMs', 'maxConcurrency', 'serviceMs', 'serviceCv', 'errorRate'],
  cron: ['intervalMs', 'batchSize'],
  bulkhead: ['bulkheadMax', 'bulkheadMode', 'acquireQueueMax', 'acquireTimeoutMs'],
  retryqueue: ['retries', 'timeoutMs', 'instances', 'capacity', 'serviceMs', 'queueLimit'],
  transcoder: [
    'renditions',
    'instances',
    'capacity',
    'serviceMs',
    'serviceCv',
    'errorRate',
    'queueLimit',
  ],
  edgecompute: ['edgeShare', 'cpuMsCap', 'instances', 'capacity', 'serviceMs', 'queueLimit'],
  writebehind: ['flushDelayMs', 'capacity', 'serviceMs', 'queueLimit'],
  loadshedder: ['rateLimitRps', 'burst', 'lowPriorityShare', 'priorityReserve'],
};

/**
 * Fields shown in the dedicated traffic section for `client`, rendered
 * separately from the generic grouped loop below (see Inspector.svelte).
 */
export const TRAFFIC_FIELDS: readonly InspectorField[] = ['rps', 'traffic', 'trafficPeriodS'];

/**
 * Grouping within a kind's field list, transcribed from the web app's
 * `FIELD_GROUPS` (`src/components/Inspector.tsx` lines 1127-1186), with
 * `instances` added to "How much it can handle" -- see the module doc for
 * why.
 */
export const FIELD_GROUPS: { title: string; fields: ReadonlySet<InspectorField> }[] = [
  {
    title: 'What it does',
    fields: new Set<InspectorField>([
      'hitRate',
      'replicaCount',
      'replicationLagMs',
      'readFraction',
      'shardCount',
      'shardCapacity',
      'hotKeyFraction',
      'targetUtil',
      'minCapacity',
      'maxCapacity',
      'cooldownMs',
      'scaleStepPct',
      'warmupMs',
      'regions',
      'activeRegion',
      'failoverMs',
      'rateLimitRps',
      'burst',
      'errorThreshold',
      'windowMs',
      'openMs',
      'halfOpenProbes',
      'indexMs',
      'indexLagMs',
      'rangeQueryFraction',
      'rangeQueryMs',
      'traversalDepth',
      'indexSizeK',
      'recallTarget',
      'partitions',
      'connectionMs',
      'authFailRate',
      'outlierAfter',
      'coldStartMs',
      'keepWarmMs',
      'maxConcurrency',
      'intervalMs',
      'batchSize',
      'bulkheadMax',
      'bulkheadMode',
      'acquireQueueMax',
      'acquireTimeoutMs',
      'flushDelayMs',
      'edgeShare',
      'lowPriorityShare',
      'priorityReserve',
    ]),
  },
  {
    title: 'How much it can handle',
    fields: new Set<InspectorField>(['instances', 'capacity', 'serviceMs', 'serviceCv', 'queueLimit']),
  },
  {
    title: 'When things go wrong',
    fields: new Set<InspectorField>(['errorRate', 'timeoutMs', 'retries']),
  },
];

/* ------------------------------------------------------------------ *
 * Formatting
 * ------------------------------------------------------------------ */

function roundTo(v: number, decimals: number): number {
  const f = 10 ** decimals;
  return Math.round(v * f) / f;
}

/** Fraction of a slider/percent control's travel that is filled, 0%-100%. */
export function fillPct(spec: RangeFieldSpec, value: number): string {
  const span = spec.max - spec.min;
  if (!Number.isFinite(span) || span <= 0) return '0%';
  const v = Number.isFinite(value) ? value : spec.min;
  const t = (v - spec.min) / span;
  return `${Math.max(0, Math.min(1, t)) * 100}%`;
}

/** Formats a raw `NodeConfig` value the way its control should display it. */
export function formatFieldValue(spec: FieldSpec, value: number | string): string {
  if (spec.control === 'enum') {
    return spec.options.find((o) => o.value === String(value))?.label ?? String(value);
  }
  const v = typeof value === 'number' ? value : Number(value);
  if (spec.zeroLabel && v === 0) return spec.zeroLabel;
  if (spec.control === 'percent') {
    const pct = v * 100;
    return `${pct !== 0 && pct < 0.1 ? '<0.1' : roundTo(pct, pct < 10 ? 1 : 0)}%`;
  }
  const n = spec.step < 1 ? roundTo(v, 2) : Math.round(v);
  const text = n.toLocaleString('en-US');
  return spec.unit ? `${text} ${spec.unit}` : text;
}

/* ------------------------------------------------------------------ *
 * Live readouts -- the value the simulation is actually measuring,
 * shown beside the control that sets its target. Not exhaustive over
 * every NodeStats field (that is the metrics/trace panels' job); this
 * covers the fields where seeing the live number next to the knob that
 * drives it is the point (MIGRATION_PLAN's "live readout beside the
 * control" requirement).
 * ------------------------------------------------------------------ */

/** Exported so Inspector.svelte's header "vitals" row can format the same
 * way as the per-field live readouts below, rather than duplicating it. */
export function pct(v: number | undefined): string | null {
  if (v === undefined || !Number.isFinite(v)) return null;
  const p = Math.max(0, v) * 100;
  return `${p > 0 && p < 0.1 ? '<0.1' : roundTo(p, p < 10 ? 1 : 0)}%`;
}

export function rate(v: number | undefined): string | null {
  if (v === undefined || !Number.isFinite(v)) return null;
  return `${v < 10 ? roundTo(v, 1) : Math.round(v)}/s`;
}

export function count(v: number | undefined): string | null {
  if (v === undefined || !Number.isFinite(v)) return null;
  return Math.round(v).toLocaleString('en-US');
}

/** A short "currently measuring X" string for `field`, or null when there is nothing to show. */
export function liveReadout(field: InspectorField, stats: NodeStats | null): string | null {
  if (!stats) return null;
  switch (field) {
    case 'capacity':
    case 'instances': {
      const u = pct(stats.utilization);
      return u ? `Busy now: ${u}` : null;
    }
    case 'queueLimit': {
      const q = count(stats.queued);
      return q ? `Waiting now: ${q} of ${stats.queueLimit}` : null;
    }
    case 'hitRate': {
      const h = pct(stats.hitRate);
      return h ? `Observed: ${h}` : null;
    }
    case 'errorRate': {
      const e = pct(stats.errorRate);
      return e ? `Observed: ${e}` : null;
    }
    case 'timeoutMs': {
      const t = rate(stats.timeoutRate);
      return t && stats.timeoutRate! > 0 ? `Timing out: ${t}` : null;
    }
    case 'rateLimitRps':
    case 'burst': {
      const tok = count(stats.tokens);
      const thr = rate(stats.throttledRate);
      return tok ? `Tokens: ${tok}${thr ? `, throttled ${thr}` : ''}` : null;
    }
    case 'errorThreshold':
    case 'windowMs': {
      const e = pct(stats.breakerErrorRate);
      return e ? `Measured now: ${e}` : null;
    }
    case 'openMs':
    case 'halfOpenProbes':
      return stats.breakerState ? `Circuit: ${stats.breakerState}` : null;
    case 'targetUtil': {
      const u = pct(stats.watchedUtil);
      if (!u) return null;
      // Reported rather than left silent: the controller reads the load
      // correctly and then cannot act, and a readout showing only the
      // utilisation looks like a bug rather than the lesson it is -- a
      // managed store has no fleet to grow. Ported from upstream
      // Inspector.tsx's AutoscalerPanel (PR #26); wording kept close to the
      // source ("adding servers is not the fix").
      return stats.watchedUnscalable
        ? `Watched node at ${u} -- adding servers is not the fix here`
        : `Watched node at ${u}`;
    }
    case 'minCapacity':
    case 'maxCapacity':
      if (stats.watchedUnscalable) {
        return 'Watched node has no instances to add or remove';
      }
      return stats.watchedInstances !== undefined
        ? `Has ${stats.watchedInstances} instance${stats.watchedInstances === 1 ? '' : 's'} now`
        : null;
    case 'cooldownMs':
    case 'scaleStepPct':
    case 'warmupMs':
      return stats.scalePhase ? `Phase: ${stats.scalePhase}` : null;
    case 'regions':
    case 'activeRegion':
      return stats.regionsHealthy !== undefined
        ? `${stats.regionsHealthy} of ${stats.regionsTotal ?? '?'} regions healthy`
        : null;
    case 'failoverMs':
      return stats.failingOver ? 'Failing over now' : null;
    case 'replicaCount':
    case 'readFraction': {
      const s = pct(stats.staleReadRate);
      return s ? `Stale reads: ${s}` : null;
    }
    case 'shardCount':
    case 'shardCapacity':
    case 'hotKeyFraction': {
      const hot = pct(stats.maxShardUtilization);
      const cold = pct(stats.minShardUtilization);
      return hot && cold ? `Busiest shard ${hot}, quietest ${cold}` : null;
    }
    case 'partitions': {
      const lag = count(stats.consumerLag);
      return lag ? `Consumer lag: ${lag} messages` : null;
    }
    case 'connectionMs': {
      const open = count(stats.connectionsOpen);
      return open ? `Open now: ${open} of ${stats.maxConnections ?? '?'}` : null;
    }
    case 'authFailRate': {
      const r = rate(stats.authRejectRate);
      return r ? `Rejected: ${r}` : null;
    }
    case 'outlierAfter':
      return stats.consecutiveFails !== undefined
        ? `Consecutive fails now: ${stats.consecutiveFails}`
        : null;
    case 'coldStartMs':
    case 'keepWarmMs': {
      const c = pct(stats.coldStartRate);
      return c ? `Cold starts: ${c}` : null;
    }
    case 'maxConcurrency':
      return stats.runningNow !== undefined ? `Running now: ${stats.runningNow}` : null;
    case 'intervalMs':
    case 'batchSize':
      return stats.nextFireInMs !== undefined ? `Fires again in ${Math.round(stats.nextFireInMs)}ms` : null;
    case 'bulkheadMax':
      return stats.bulkheadInFlight !== undefined
        ? `In flight: ${stats.bulkheadInFlight} of ${stats.bulkheadLimit ?? '?'}`
        : null;
    case 'bulkheadMode':
    case 'acquireQueueMax': {
      const w = count(stats.bulkheadWaiting);
      return w ? `Waiting to acquire: ${w}` : null;
    }
    case 'acquireTimeoutMs': {
      const t = rate(stats.bulkheadAcquireTimeoutRate);
      return t && (stats.bulkheadAcquireTimeoutRate ?? 0) > 0 ? `Acquire timeouts: ${t}` : null;
    }
    case 'flushDelayMs': {
      const d = count(stats.dirtyWrites);
      return d ? `Dirty writes now: ${d}` : null;
    }
    case 'edgeShare':
    case 'cpuMsCap': {
      const r = rate(stats.edgeHandledRate);
      return r ? `Handled at edge: ${r}` : null;
    }
    case 'lowPriorityShare':
    case 'priorityReserve': {
      const r = rate(stats.lowSheddedRate);
      return r ? `Low priority dropped: ${r}` : null;
    }
    case 'lockMs':
      return stats.lockWaitMs !== undefined ? `Measured wait: ${Math.round(stats.lockWaitMs)}ms` : null;
    case 'prefixRps': {
      const r = rate(stats.slowdownRate);
      return r ? `Slowdowns: ${r}` : null;
    }
    case 'renditions': {
      const r = rate(stats.outputRate);
      return r ? `Output rate: ${r}` : null;
    }
    case 'indexMs':
    case 'indexLagMs': {
      const s = pct(stats.staleSearchRate);
      return s ? `Stale searches: ${s}` : null;
    }
    case 'rangeQueryFraction':
    case 'rangeQueryMs': {
      const r = rate(stats.rangeQueryRate);
      return r ? `Range queries: ${r}` : null;
    }
    case 'traversalDepth':
      return stats.traversalCostMs !== undefined
        ? `Measured cost: ${Math.round(stats.traversalCostMs)}ms`
        : null;
    case 'indexSizeK':
    case 'recallTarget':
      return stats.queryCostMs !== undefined ? `Measured cost: ${Math.round(stats.queryCostMs)}ms` : null;
    default:
      return null;
  }
}
