//! Faithful port of `src/sim/types.ts`: every wire type the simulation
//! engine and the frontend agree on.
//!
//! All structs use `#[serde(rename_all = "camelCase")]` and all
//! lowercase-literal TS unions use `#[serde(rename_all = "lowercase")]` (or
//! `"kebab-case"` where the TS literal itself contains a hyphen), so the
//! JSON on the wire -- and the JSON written to `.breakscale` files -- is
//! field-for-field identical to `src/sim/types.ts`. See MIGRATION_PLAN.md
//! #4 ("Domain contract (Rust <-> TypeScript wire format)") for the
//! module-level contract this file implements.
//!
//! Every doc comment below is carried over from the TS source, because the
//! reasoning it records (WHY a field exists, not just what it is) is
//! load-bearing for whoever edits this next -- see AGENTS.md and
//! MIGRATION_PLAN.md #4.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The 33 kinds of node the simulator knows how to build. `#[serde(rename_all
/// = "lowercase")]` lowercases each PascalCase variant with no separator,
/// which is exactly the lowercase string literal `src/sim/types.ts` uses
/// (`'ratelimiter'`, `'objectstore'`, `'timeseriesdb'`, ...) -- this file was
/// checked variant-by-variant against that source so the wire strings match.
///
/// NOTE: the TS source (`src/sim/types.ts` lines 1-34) and
/// MIGRATION_PLAN.md #4 both describe this union as having 34 variants; the
/// actual source lists 33. This port follows the actual source file exactly
/// -- see the deviation note in the porting agent's final report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Client,
    Lb,
    Service,
    Cache,
    Db,
    Queue,
    Worker,
    Autoscaler,
    Region,
    Cdn,
    RateLimiter,
    Breaker,
    Replica,
    Shard,
    ObjectStore,
    SearchIndex,
    TimeSeriesDb,
    GraphDb,
    ColdStorage,
    VectorDb,
    StreamBroker,
    PubSub,
    WebSocket,
    ApiGateway,
    Sidecar,
    Lambda,
    Cron,
    Bulkhead,
    RetryQueue,
    Transcoder,
    EdgeCompute,
    WriteBehind,
    LoadShedder,
}

/// Tunable knobs per node kind. Not every field applies to every kind.
///
/// Every numeric field here is `f64`, mirroring the TS `number` type: the TS
/// source does not distinguish integer from float fields (it does arithmetic
/// like `capacity * instances` freely across all of them), so guessing a
/// narrower Rust type per field would be inventing a constraint the source
/// does not have.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig {
    /// Parallel request slots ON ONE INSTANCE -- threads, connections,
    /// workers inside a single process. This is a property of the machine
    /// image, not of the fleet: it is what a student sets, and an
    /// autoscaler never touches it.
    ///
    /// The quantity that actually decides how much work a node can do at
    /// once is `instances * capacity`. See `instances` below for why the
    /// two are split.
    pub capacity: f64,
    /// How many copies of this node are running -- machines, pods,
    /// containers. Defaults to 1 when absent, which is exactly the
    /// behaviour of every topology written before this field existed: one
    /// instance holding `capacity` slots, so `instances * capacity ===
    /// capacity`.
    ///
    /// WHY THIS EXISTS. `capacity` alone had to mean two different things
    /// at once. A student reads "traffic went up, so we added servers"; the
    /// engine read "the number of threads went up". Those are not the same
    /// lesson, and the ambiguity is what made the autoscaler feel like it
    /// did nothing: it incremented a thread count nobody could see.
    /// Splitting the two gives each number one job -- `capacity` is how big
    /// one box is, `instances` is how many boxes there are -- and it makes
    /// the autoscaler do the thing its name promises: it adds and removes
    /// INSTANCES, which is what a real one does.
    ///
    /// Only kinds with an instance model read it (service, worker, db,
    /// cache, lb, cdn). A shard's fleet size is `shardCount` and a replica
    /// set's is `replicaCount`; both already had their own honest count and
    /// neither is driven through this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances: Option<f64>,
    /// Mean service time in ms for one request.
    pub service_ms: f64,
    /// Coefficient of variation of service time. 0 = deterministic, 1 =
    /// exponential.
    pub service_cv: f64,
    /// Max queued requests before shedding load.
    pub queue_limit: f64,
    /// Fraction of reads answered without hitting downstream. Cache only.
    pub hit_rate: f64,
    /// Probability a request fails on its own (bad deploy, bug).
    pub error_rate: f64,
    /// ms after which the caller gives up. 0 = no timeout.
    pub timeout_ms: f64,
    /// Retries on failure/timeout.
    pub retries: f64,
    /// Client only: requests per second offered to the system.
    pub rps: f64,
    /// Client only: how the offered rate varies over time.
    ///
    /// `rps` stays the BASELINE the reader set, and the pattern scales it.
    /// Real traffic is never a flat line, and a system that copes with 500
    /// a second arriving evenly can still fall over when the same 500
    /// arrive in a burst.
    ///
    /// Absent means steady, so every topology and saved design written
    /// before patterns existed keeps its exact behaviour. The engine reads
    /// it through `effectiveRps`, which is a pure function of simulation
    /// time: no state is carried between ticks, so a seed and a topology
    /// still replay identically.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic: Option<TrafficPattern>,
    /// Client only: seconds for one full cycle of `traffic`.
    ///
    /// What a cycle means depends on the pattern: for `ramp` it is the
    /// climb from nothing to the baseline, for `spike` the gap between
    /// bursts, for `diurnal` a whole simulated day. Defaults to
    /// `DEFAULT_TRAFFIC_PERIOD_S` when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traffic_period_s: Option<f64>,

    // ---- autoscaler -----------------------------------------------------
    // An autoscaler is a controller, not a request path. It watches one
    // node's utilisation and writes that node's INSTANCE COUNT -- it adds
    // and removes machines, leaving `capacity` (the size of one machine)
    // alone.
    //
    // It is joined to the node it drives by a CONTROL EDGE
    // (`SimEdge.control`), which carries no requests. See `SimEdge`.
    // -----------------------------------------------------------------
    /// Autoscaler only: utilisation the controller aims to hold the watched
    /// node at, 0..1. Above it the controller scales up, below `targetUtil`
    /// minus a dead band it scales down.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_util: Option<f64>,
    /// Autoscaler only: fewest instances the controller will leave running.
    ///
    /// Named `minCapacity` for backward compatibility -- every existing
    /// preset and saved topology sets it -- but the unit is INSTANCES,
    /// matching what the controller now writes. For the
    /// one-instance-per-slot topologies that predate the instance model the
    /// two readings coincide, so nothing that already worked changes
    /// meaning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_capacity: Option<f64>,
    /// Autoscaler only: most instances the controller will run. In
    /// instances.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_capacity: Option<f64>,
    /// Autoscaler only: ms the controller must wait after one decision
    /// before it may make another. Models the metric/decision interval of a
    /// real autoscaler and is the main knob that determines whether it
    /// oscillates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_ms: Option<f64>,
    /// Autoscaler only: size of a single scaling step as a fraction of the
    /// current instance count, 0..1. 0.5 means "add or remove half the
    /// fleet". A step always moves at least one instance, so a small
    /// percentage of a small fleet is never a silent no-op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_step_pct: Option<f64>,
    /// Autoscaler only: ms between a scale-UP decision and the new
    /// instances actually serving traffic (boot + image pull + warm-up).
    /// Scale-DOWN is immediate, as it is in reality. This delay is what
    /// makes capacity lag load, and it is the single most important knob in
    /// the component: it is why a student sees requests fail *after* the
    /// autoscaler already decided to help.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_ms: Option<f64>,

    // ---- region -----------------------------------------------------
    // A region node is a failover switch in front of N downstream edges,
    // where edge index i is region i.
    // -----------------------------------------------------------------
    /// Region only: how many of this node's outgoing edges are treated as
    /// regions. Edge index i is region i; edges beyond `regions` are
    /// ignored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regions: Option<f64>,
    /// Region only: index of the region currently serving traffic,
    /// 0-based. Set by the student, and advanced automatically by
    /// failover.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_region: Option<f64>,
    /// Region only: ms of downtime between the active region being
    /// detected as failed and traffic actually landing on the next one.
    /// Requests arriving during this window fail with 'region-down' --
    /// failover is not free.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover_ms: Option<f64>,

    // ---- ratelimiter -----------------------------------------------------
    // A token bucket in front of a downstream. It refills continuously in
    // simulated time, so admissions over T seconds converge on rate*T
    // rather than being quantised by any tick interval.
    // -----------------------------------------------------------------
    /// Rate limiter only: sustained admission rate in tokens (requests) per
    /// second. One token is spent per admitted request; the bucket refills
    /// at exactly this many tokens per second of simulated time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_rps: Option<f64>,
    /// Rate limiter only: bucket size in tokens. This is the largest burst
    /// admitted instantly from an idle limiter, and it is also the
    /// bucket's refill ceiling. Defaults to `rateLimitRps` when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst: Option<f64>,

    // ---- breaker -----------------------------------------------------
    // A circuit breaker wrapping its downstream. It watches the
    // downstream's error rate over a trailing window and, once that
    // exceeds the threshold, fails fast without calling downstream at all.
    // -----------------------------------------------------------------
    /// Breaker only: downstream error fraction over the trailing window
    /// above which the breaker trips OPEN, 0..1. 0.5 means "trip once half
    /// the downstream calls in the window are failing".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_threshold: Option<f64>,
    /// Breaker only: length of the trailing window over which the
    /// downstream error fraction is measured, in ms. Outcomes older than
    /// this are forgotten, so a recovered downstream stops holding old
    /// failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_ms: Option<f64>,
    /// Breaker only: ms the breaker stays OPEN (failing fast) before it
    /// moves to HALF-OPEN and allows probe traffic through again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_ms: Option<f64>,
    /// Breaker only: number of probe requests admitted while HALF-OPEN. If
    /// all of them succeed the breaker CLOSES; if any fails it re-OPENS
    /// immediately and the remaining probes are not sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub half_open_probes: Option<f64>,

    // ---- cdn -----------------------------------------------------
    // An edge cache in front of everything. It reuses `hitRate`,
    // `serviceMs` and `capacity` from the common knobs above; a miss costs
    // a round trip to whatever origin is wired downstream, which is where
    // its teaching value lives. It needs no extra config field of its own.
    // ---- replica: a read-replica set behind a primary ----------------
    /// Number of read replicas in the set. Read capacity is `replicaCount *
    /// capacity` slots; writes are serialised through the primary's own
    /// `capacity` slots. Floored to at least 1.
    pub replica_count: f64,
    /// Replication lag in ms: how long a write takes to reach the
    /// replicas. A read of a key issued less than this long after that key
    /// was written is served from a replica that has not caught up yet,
    /// and is counted as a stale read. 0 means synchronous replication and
    /// never goes stale.
    pub replication_lag_ms: f64,
    /// Fraction of traffic that is reads, 0..1. Reads go to the replicas
    /// (and may be stale); the remainder are writes, which go to the
    /// primary and then propagate.
    pub read_fraction: f64,

    // ---- shard: horizontal partitioning ------------------------------
    /// Number of partitions. A request is routed to `key % shardCount`, so
    /// each shard is an independent queue-and-servers unit. Floored to >=
    /// 1.
    pub shard_count: f64,
    /// Parallel request slots *per shard*. Total capacity across the node
    /// is `shardCount * shardCapacity`, but a single shard can only ever
    /// use its own slots -- which is exactly why a hot key melts one
    /// shard.
    pub shard_capacity: f64,
    /// Fraction of traffic forced onto one single shard, 0..1, regardless
    /// of the request's own key. 0 spreads traffic by key as normal; 0.8
    /// sends 80% of it to shard 0 and leaves the rest nearly idle.
    pub hot_key_fraction: f64,

    // ---- searchindex: a search cluster with asynchronous indexing ------
    // Reads (searches) are cheap; writes pay an extra indexing cost, and a
    // committed write is not searchable until `indexLagMs` later. The read
    // vs write split reuses `readFraction` above.
    // -----------------------------------------------------------------
    /// Search index only: extra service milliseconds a WRITE pays on top
    /// of the drawn `serviceMs`, for tokenising and updating the inverted
    /// index. Searches pay only `serviceMs`. This asymmetry is why bulk
    /// writes can starve a search cluster whose queries were never the
    /// problem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_ms: Option<f64>,
    /// Search index only: milliseconds after a write COMMITS before the
    /// document is visible to searches (the refresh interval of a real
    /// search engine). A search for a key inside that window is served
    /// from the old index and is counted as a stale search. 0 means
    /// instantly searchable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_lag_ms: Option<f64>,

    // ---- timeseriesdb: cheap appends, expensive range queries ----------
    /// Time-series store only: fraction of traffic that is RANGE QUERIES,
    /// 0..1. The rest are appends, which cost only `serviceMs`. Raising
    /// this a few percent is enough to melt a store sized for pure
    /// ingest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_query_fraction: Option<f64>,
    /// Time-series store only: extra service milliseconds a range query
    /// pays on top of the drawn `serviceMs`, for scanning and aggregating
    /// many points. Appends never pay it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_query_ms: Option<f64>,

    // ---- graphdb: traversal cost grows with depth ----------------------
    /// Graph database only: how many hops a query traverses from its start
    /// node, >= 1. Each extra hop multiplies the edges visited by ~3 (the
    /// modelled mean out-degree), so query cost is `serviceMs *
    /// 3^(traversalDepth - 1)`: depth 1 reads one neighbourhood, depth 2 is
    /// friends-of-friends at 3x, depth 3 is 9x, and so on. This is the
    /// knob that shows why a deep traversal gets expensive fast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_depth: Option<f64>,

    // ---- vectordb: embedding search over a large index -----------------
    /// Vector database only: size of the vector index, in THOUSANDS of
    /// embeddings. Query cost grows with log2 of this, matching an ANN
    /// index that probes more partitions as the corpus grows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_size_k: Option<f64>,
    /// Vector database only: target recall of the approximate search,
    /// 0..0.99. Cost scales with `1 / (1 - recallTarget)`: 0.9 probes 10x
    /// more of the index than 0, 0.99 probes 100x. Mean query cost works
    /// out to `serviceMs * log2(2 + indexSizeK) / (1 - recallTarget)`,
    /// which is the recall/latency trade-off drawn as one curve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall_target: Option<f64>,

    // ---- streambroker: a partitioned, replayable log --------------------
    // Kafka-shaped: producers are acknowledged immediately, messages land
    // in a partition by key, and every outgoing edge is an independent
    // CONSUMER GROUP with its own cursor. Within one partition a group
    // consumes serially, so `partitions` is the parallelism ceiling of
    // every group. `queueLimit` doubles as the log's RETENTION in
    // messages: a group that falls further behind than that skips ahead,
    // and the skipped messages are lost to it, exactly like expiring out
    // of a Kafka topic.
    // -----------------------------------------------------------------
    /// Stream broker only: number of partitions in the log. A message
    /// lands in partition `key % partitions`, and one consumer group
    /// consumes at most one message per partition at a time, so this is
    /// also the maximum delivery parallelism of every group. Floored to >=
    /// 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partitions: Option<f64>,

    // ---- websocket: capacity in held connections -------------------------
    /// Websocket gateway only: mean lifetime of one connection in ms. Each
    /// accepted connection occupies one of the node's `instances *
    /// capacity` connection slots for exactly this long. Concurrent
    /// connections settle at `rps * connectionMs / 1000` (Little's law),
    /// which is why a chat gateway saturates on connection count rather
    /// than on request rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_ms: Option<f64>,

    // ---- apigateway: auth in front of routing ----------------------------
    /// API gateway only: fraction of requests refused as 'unauthorized',
    /// 0..1. Rolled at admission, after the token bucket, so a burst of
    /// bad credentials still spends rate-limit tokens the way it does in
    /// life.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_fail_rate: Option<f64>,

    // ---- sidecar: outlier ejection ----------------------------------------
    /// Sidecar only: consecutive downstream failures after which the proxy
    /// ejects its upstream and fails fast for `openMs`. A simpler policy
    /// than the breaker's windowed error rate on purpose; this is
    /// Envoy-style outlier detection, and having both flavours in the
    /// palette is itself a lesson. Floored to >= 1; defaults to 5 when
    /// unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outlier_after: Option<f64>,

    // ---- lambda: serverless scaling ----------------------------------------
    /// Lambda only: extra latency in ms paid by a request that arrives
    /// when no warm instance is idle. This is the cold start: the platform
    /// must provision and boot an instance before the function body even
    /// runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_start_ms: Option<f64>,
    /// Lambda only: ms an idle warm instance is kept alive after finishing
    /// a request before the platform reclaims it. Short keep-warm plus
    /// bursty traffic is the recipe for a high cold-start rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_warm_ms: Option<f64>,
    /// Lambda only: most invocations allowed to run at once (the account
    /// concurrency limit). Arrivals beyond it are refused as 'throttled';
    /// a lambda has no queue of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<f64>,

    // ---- cron: scheduled batch load ----------------------------------------
    /// Cron only: ms between firings of the scheduled job. Each firing
    /// dumps `batchSize` requests down every outgoing edge at once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<f64>,
    /// Cron only: requests emitted per outgoing edge each time the job
    /// fires. The burst arrives effectively simultaneously, which is the
    /// point: it is what a midnight report run does to a database.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<f64>,

    // ---- bulkhead: an isolated concurrency pool -----------------------------
    // A gate in front of ONE dependency. It caps how many calls may be
    // outstanding to that dependency at once; the excess fails fast as
    // 'bulkhead-full' instead of queueing behind a slow downstream.
    // -----------------------------------------------------------------
    /// Bulkhead only: most downstream calls allowed to be in flight at
    /// once through this bulkhead. Requests arriving with the pool full
    /// are refused immediately as 'bulkhead-full'. Floored to >= 1;
    /// defaults to 8 when unset. By Little's law the pool supports roughly
    /// `bulkheadMax / serviceSeconds` requests per second of admitted
    /// traffic, and the cap is what keeps a slowed dependency's backlog
    /// bounded instead of letting it grow without limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_max: Option<f64>,
    /// Bulkhead only: `Reject` (the default) fails a request immediately
    /// once the pool is full, exactly as before this knob existed. `Wait`
    /// holds the caller in a bounded acquire queue instead, up to
    /// `acquireQueueMax` deep, until a slot frees or its own
    /// `acquireTimeoutMs` elapses -- modelling connection-pool exhaustion
    /// (wait, then acquire-timeout, then the caller's own retry) instead of
    /// failing fast at the door. `None`/absent means `Reject`, so a
    /// topology saved before this feature existed is unaffected. Port of
    /// `behaviour-resilience.ts`'s `bulkheadMode` (upstream `351327c4`, PR
    /// #77).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_mode: Option<BulkheadMode>,
    /// Bulkhead only: waiting acquires allowed before a request is shed
    /// immediately as 'bulkhead-full' even with `bulkheadMode: Wait`.
    /// Floored to >= 0; defaults to 100 when unset. Only consulted when
    /// `bulkheadMode` is `Wait`. Port of `cfgAcquireQueueMax` (same upstream
    /// commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acquire_queue_max: Option<f64>,
    /// Bulkhead only: longest a waiting request may wait to acquire a pool
    /// slot before it fails as 'acquire-timeout', in ms. Defaults to 1000
    /// when unset. Only consulted when `bulkheadMode` is `Wait`. Port of
    /// `cfgAcquireTimeoutMs` (same upstream commit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acquire_timeout_ms: Option<f64>,

    // ---- retryqueue: retried delivery with a dead letter shelf ------------
    // Reuses the shared knobs: `capacity` is delivery concurrency,
    // `serviceMs` the per-delivery dispatch cost, `queueLimit` the
    // buffered message ceiling, `timeoutMs` the per-attempt deadline, and
    // `retries` how many redeliveries a failed message gets before it is
    // dead-lettered. No field of its own is needed.
    // ---- writebehind: a write buffer in front of a store -------------------
    /// Write-behind cache only: ms an acknowledged write sits dirty in the
    /// buffer before its flush lands on the backing store. Models the
    /// batch interval of a real write-behind cache. The standing dirty
    /// population is roughly `writeRate * (flushDelayMs / 1000)` writes,
    /// and that population is exactly what a crash of this node loses. 0
    /// means the flush is issued immediately.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush_delay_ms: Option<f64>,

    // ---- edgecompute: limited compute at the CDN edge ------------------------
    /// Edge compute only: fraction of requests the edge function can
    /// answer entirely on its own, 0..1. The rest pass through to whatever
    /// is wired downstream. This is capability, not caching: the edge is
    /// fast but limited, and this knob is how limited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_share: Option<f64>,

    // ---- loadshedder: priority-aware admission -------------------------------
    // A token bucket like the rate limiter's (it reuses `rateLimitRps` and
    // `burst`), but it refuses by PRIORITY: low-priority traffic is turned
    // away while the bucket still holds a reserve, so high-priority
    // traffic keeps finding tokens when the system is saturated.
    // -----------------------------------------------------------------
    /// Load shedder only: fraction of traffic classified low priority,
    /// 0..1. Priority is derived deterministically from the request key (a
    /// hash of the key against this share), so the same key is always the
    /// same priority and no RNG draw is spent on classification. Defaults
    /// to 0, under which every request is high priority and the shedder
    /// behaves like a plain rate limiter.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_priority_share: Option<f64>,
    /// Load shedder only: fraction of the token bucket reserved for
    /// high-priority traffic, 0..1. A low-priority request needs the
    /// bucket to hold more than `1 + priorityReserve * burst` tokens; a
    /// high-priority one needs only 1. Under saturation the bucket hovers
    /// inside the reserve, so low-priority traffic is dropped first while
    /// high-priority traffic keeps being admitted. Defaults to 0.3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_reserve: Option<f64>,

    // ---- db: write lock contention -------------------------------------------
    // Reads share the slot pool freely. Writes also contend with each
    // other: each write entering service waits an extra `lockMs` for every
    // other write already in flight, which is row/page lock contention
    // stated as arithmetic. The consequence it teaches: adding instances
    // multiplies read capacity but does nothing for the write path,
    // because the lock is a property of the data, not of the fleet.
    // -----------------------------------------------------------------
    /// Database only: extra service milliseconds a WRITE pays per
    /// concurrent write already in flight when it starts. 0 (or unset)
    /// disables contention entirely. The read/write split reuses
    /// `readFraction`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_ms: Option<f64>,

    // ---- objectstore: per-prefix rate ceiling ----------------------------------
    // Blob stores scale by key prefix: each prefix is its own partition
    // with its own request-rate ceiling, and the store as a whole has no
    // queue. A hot prefix is refused with a 'slowdown' while the rest of
    // the store idles -- which is why S3 performance guidance is about key
    // LAYOUT, not about buying a bigger bucket.
    // -----------------------------------------------------------------
    /// Object storage only: sustained requests per second ONE key prefix
    /// sustains before further requests to it are refused ('throttled',
    /// the S3 503 SlowDown). Keys map onto 8 fixed prefixes; uniform
    /// traffic therefore sustains ~8x this figure while a single-prefix
    /// hotspot gets exactly 1x. 0 or unset means no per-prefix limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_rps: Option<f64>,

    // ---- transcoder: the quality ladder -----------------------------------------
    /// Transcoder only: output files produced per finished job, per
    /// outgoing edge -- the bitrate ladder of a real encode pipeline. Each
    /// finished job hands `renditions` detached uploads to every node
    /// wired after the farm; nothing waits on them. Storage therefore sees
    /// `renditions` times the job rate, the write amplification every
    /// video pipeline budgets for. Floored to >= 1; defaults to 3 when
    /// unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renditions: Option<f64>,

    // ---- edgecompute: the CPU budget ---------------------------------------------
    /// Edge compute only: hard per-request CPU budget in ms. An edge
    /// runtime is not a server: a request whose execution runs past this
    /// budget is killed at the edge and passed through to the origin path
    /// instead, even when `edgeShare` says the code could have answered
    /// it. 0 or unset means no budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_ms_cap: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub config: NodeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Share of traffic leaving `from` that takes this edge (relative
    /// weight).
    pub weight: f64,

    /// This edge is a CONTROL relationship, not a request path.
    ///
    /// A control edge says "`from` acts on `to`" -- an autoscaler driving
    /// the node it resizes. It looks like a wire and it is drawn like one,
    /// but no request ever crosses it: the engine leaves control edges out
    /// of every node's routing set entirely, so fan-out, load-balancer
    /// edge selection and region indices never see them. A student who
    /// wires an autoscaler beside a service is stating a supervisory
    /// relationship, and previously that was indistinguishable from wiring
    /// traffic INTO the controller -- which is a mistake the topology had
    /// no way to express, let alone show.
    ///
    /// Absent or false means an ordinary request edge, so every topology
    /// written before this field existed keeps its exact meaning.
    ///
    /// The engine also treats it as control when the SOURCE is a kind that
    /// only ever supervises (an autoscaler), regardless of this flag --
    /// see `ComponentBehaviour.controlsTarget`. The flag is what lets the
    /// renderer and the topology be explicit about it, and what would let
    /// some future kind have a control edge and a request edge at the same
    /// time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control: Option<bool>,

    // ---- link characteristics (networking phase) -----------------------
    // All optional and absent from every topology today, so nothing
    // changes until one is set. Only latencyMs is implemented; the other
    // two are declared now so the shape does not have to churn later.
    // -----------------------------------------------------------------
    /// Propagation delay across this link, in ms. A request crossing the
    /// edge waits this long before it is offered to the target node.
    /// Undefined or 0 means an instantaneous hop, which is what every
    /// current preset uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    /// DECLARED BUT UNUSED. Link capacity in requests/sec. The networking
    /// phase will use this to queue and delay traffic once an edge is
    /// saturated; today the engine ignores it entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bandwidth_rps: Option<f64>,
    /// DECLARED BUT UNUSED. Fraction of requests the link drops, 0..1. The
    /// networking phase will fail crossings against this; today the engine
    /// ignores it entirely.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loss_rate: Option<f64>,
}

/// Placeholder: `sim::annotations` (ported separately) defines the real
/// Annotation enum/struct and will replace this alias. Kept as
/// `serde_json::Value` here so `Topology` round-trips any annotation the
/// engine never reads anyway (the engine never reads this field -- see the
/// TS comment on `Topology.annotations` below).
pub type Annotation = serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Topology {
    pub nodes: Vec<SimNode>,
    pub edges: Vec<SimEdge>,
    /// Canvas notes and sections. Documentation only: the engine never
    /// reads this field, annotations carry no traffic and affect no
    /// metric. Optional so every existing topology and saved design stays
    /// valid unchanged. The shape lives in `annotations.rs` (ported
    /// separately from `annotations.ts`), which is where the engine-free
    /// reasoning for it belongs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Annotation>>,
}

/// Autoscaler only: what the controller is doing right now.
///
///  - `Observing` gathering its first metric window on a target it has
///    just been pointed at; it will not act yet.
///  - `Warming`   a scale-up is booked and its instances are still
///    booting.
///  - `Cooldown`  it acted recently and is waiting out `cooldownMs`.
///  - `Steady`    free to act, and the utilisation does not call for it.
///
/// This is the field that turns the component from a silent box into
/// something a student can read a sentence off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScalePhase {
    Observing,
    Warming,
    Cooldown,
    Steady,
}

/// Breaker only: the circuit's current state. `HalfOpen` serialises as
/// `"half-open"` via `#[serde(rename_all = "kebab-case")]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// Rolling stats for one node over the last window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStats {
    /// Requests currently being served.
    pub in_flight: f64,
    /// Requests waiting for a slot.
    pub queued: f64,
    /// Completed requests per second.
    pub throughput: f64,
    /// Offered load per second (including those later dropped).
    pub arrival_rate: f64,
    /// Fraction of slots busy, 0..1. Can exceed 1 conceptually if
    /// oversubscribed.
    pub utilization: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    /// Fraction of requests that errored or were shed, 0..1.
    pub error_rate: f64,
    /// Requests dropped due to a full queue, per second.
    pub shed_rate: f64,
    /// Requests that exceeded the caller's timeout, per second.
    pub timeout_rate: f64,
    /// Cache hit fraction observed this window (cache nodes only).
    pub hit_rate: f64,
    /// Total completed since sim start.
    pub total_completed: f64,
    /// Total failed since sim start.
    pub total_failed: f64,

    // ====================================================================
    // INSTANCE MODEL -- what this node is actually made of
    //
    // Every field above describes a node as ONE thing with one meter.
    // Several kinds are not one thing: a service scaled to 5 is five
    // machines, a shard with 4 partitions is four independent
    // queue-and-server units, a replica set is a primary plus N read
    // replicas. The engine has always known this; these three fields are
    // how it says so, and they are what lets the canvas draw a stack of
    // instances or a row of partitions instead of a box.
    //
    // --------------------------------------------------------------
    // WHAT ONE UNIT MEANS, PER KIND -- the decision, so the next agent
    // follows it rather than inventing a second interpretation:
    //
    //   service, worker, db, cache, lb, cdn
    //     ONE UNIT = ONE INSTANCE -- one machine, pod or container,
    //     holding `capacity` request slots. `instances` is
    //     `config.instances` (1 when unset), and the node's real
    //     parallelism is `instances * capacity`.
    //
    //     This is a DELIBERATE REVISION of the earlier reading, which made
    //     one unit one capacity slot. That reading was chosen because
    //     `capacity` was the knob the autoscaler wrote, and it had the
    //     fatal property that the drawn stack meant "threads" while every
    //     student read it as "servers". The slots-per-machine knob it
    //     complained did not exist now does: it is `capacity` itself, and
    //     `instances` is the fleet size the controller moves. The stack
    //     and the controller's steps still line up exactly -- the
    //     controller adds 1 instance and the picture gains 1 card -- but
    //     now they line up on the quantity the student meant.
    //
    //     perInstance is a WATERLINE, not a per-machine busy flag. The
    //     engine integrates one smoothed utilisation per node, not one per
    //     instance, and requests are dispatched to whichever slot is free
    //     -- instance 3 has no identity that outlives a request. So
    //     `utilization * instances` busy-instance-equivalents are filled
    //     from index 0: full units read 1, one boundary unit carries the
    //     remainder, the rest read 0. A stack drawn from this shows "3 of
    //     5 busy" truthfully without inventing per-machine state the
    //     simulation does not have.
    //
    //   shard
    //     ONE UNIT = ONE PARTITION. perInstance[i] is shard i's own
    //     smoothed utilisation -- genuinely independent numbers, because a
    //     partition has its own slots and its own backlog. This is the
    //     kind where the vector carries the whole lesson: with a hot key
    //     one entry pins at 1.0 while the others sit near 0 and the
    //     node-level mean looks fine.
    //
    //   replica
    //     ONE UNIT = ONE MEMBER OF THE SET, ordered [primary, replica 0,
    //     replica 1, ...], so instances === replicaCount + 1. Index 0 is
    //     the primary and reports the WRITE pool's utilisation; the rest
    //     share the read pool's utilisation, which is the honest reading
    //     because a read is served by whichever replica is free and the
    //     engine pools them. Drawing the primary distinctly from the read
    //     set is the point: the write pool saturating while the read set
    //     idles is why adding read replicas does not fix a write
    //     bottleneck.
    //
    //   client, queue, autoscaler, region, ratelimiter, breaker
    //     NOT an instance model. These report `instances` undefined. A
    //     queue is a buffer (draw its depth against queueLimit), a
    //     breaker and a limiter are gates, an autoscaler and a region are
    //     controllers. Giving them a stack would say something false
    //     about what they are.
    //
    // A consumer must treat `instances === undefined` (in Rust: `None`)
    // as "this kind is one thing" and fall back to the scalar meters,
    // never as "zero instances".
    // ====================================================================
    /// How many units this node currently represents, >= 1. `None` for
    /// kinds that are genuinely one thing (see the table above). For a
    /// kind whose unit is an instance this equals the live
    /// `config.instances`, which is exactly what an autoscaler writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances: Option<f64>,
    /// Units that have been DECIDED but are not serving yet -- an
    /// autoscaler's scale-up during its `warmupMs`. Present and > 0 only
    /// while the watched node has instances booked and still booting.
    ///
    /// These are NOT included in `instances`, and deliberately so: they
    /// cannot take a request, so counting them as live would overstate the
    /// node and would make the drawn stack claim capacity the system does
    /// not have. The UI draws `instances` solid and `instancesPending`
    /// ghosted, and the interval where the ghosts sit there while the
    /// queue builds behind them is the whole teaching point of warm-up
    /// lag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instances_pending: Option<f64>,
    /// Utilisation per unit, 0..1, with `length === instances`. `None`
    /// whenever `instances` is. Index meaning is per-kind and fixed by the
    /// table above -- partition i for a shard, [primary, ...replicas] for
    /// a replica set, an anonymous waterline over interchangeable slots
    /// for the rest.
    ///
    /// OWNERSHIP: this array belongs to the snapshot that produced it. The
    /// engine hands out a fresh array whenever the contents changed, so a
    /// memoised consumer comparing the previous `NodeStats` by reference
    /// (or this field by reference) sees a new identity exactly when there
    /// is something new to draw. Do not mutate it, and do not retain it
    /// across snapshots expecting it to update in place -- it will not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_instance: Option<Vec<f64>>,

    /// The node's queue limit in requests, mirrored from config so a
    /// drawable `queued / queueLimit` fill needs only the snapshot. Always
    /// >= 0, and always the same integer the engine actually sheds
    /// against -- reading `config.queueLimit` instead would show the raw
    /// slider value rather than the floored one the simulation enforces.
    ///
    /// For a shard this is the limit PER PARTITION, matching the per-shard
    /// backlogs the behaviour keeps; for a replica set it is the one
    /// shared backlog reads and writes both draw from.
    pub queue_limit: f64,

    // ---- autoscaler readouts ------------------------------------------
    // Everything needed to narrate the controller's decision in words,
    // without the reader having to open the config panel:
    //
    //   "watching <watchedId>: it is at <watchedUtil> against my setpoint
    //    of <setpoint>. I want <targetInstances> instances, it has
    //    <watchedInstances>, <pendingInstances> are booting (<phase>)."
    // -----------------------------------------------------------------
    /// Autoscaler only: instance count the controller has decided the
    /// watched node should have. Differs from the watched node's live
    /// instance count for `warmupMs` after a scale-up, which is exactly
    /// the lag worth seeing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_instances: Option<f64>,
    /// Autoscaler only: the watched node's live instance count right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched_instances: Option<f64>,
    /// Autoscaler only: true when the watched node has no fleet to resize,
    /// so the controller can read the load correctly and still be unable
    /// to act.
    ///
    /// An object store is the case that prompted this: it is modelled as
    /// one managed service rather than a pool of machines, exactly as S3
    /// is, so there is no instance count for a controller to move.
    /// `scale_of()` returns `None` for exactly this case (a kind with
    /// `scale_field() == None`), which is NOT the same as zero instances --
    /// without this flag the panel showed a healthy-looking readout beside
    /// a fleet that never changed, and the only reasonable conclusion for a
    /// reader was that the autoscaler was broken. That it cannot help here
    /// is the lesson: when a blob store melts there is no "add servers"
    /// knob to reach for. types.ts `NodeStats.watchedUnscalable`, PR #26.
    ///
    /// Set by `AutoscalerBehaviour::decorate_stats` in `behaviour::control`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched_unscalable: Option<bool>,
    /// Autoscaler only: instances decided but still booting. Equal to
    /// `targetInstances - watchedInstances` while warming up, else 0. The
    /// same number is attached to the WATCHED node as `instancesPending`,
    /// so the stack that is about to grow can draw its own ghosts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_instances: Option<f64>,
    /// Autoscaler only: id of the node this controller drives, or '' when
    /// it is not wired to one. Resolved from its control edge, so a UI can
    /// name the target without re-walking the topology.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched_id: Option<String>,
    /// Autoscaler only: the utilisation it is reacting to, 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watched_util: Option<f64>,
    /// Autoscaler only: the setpoint it is holding that utilisation
    /// against, 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setpoint: Option<f64>,
    /// Autoscaler only: what the controller is doing right now. See
    /// `ScalePhase`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_phase: Option<ScalePhase>,
    /// Autoscaler only: ms until the current phase ends -- until
    /// observation completes, until booked instances land, or until the
    /// cooldown expires. 0 in 'steady'. Lets a UI draw a countdown rather
    /// than a frozen label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_remaining_ms: Option<f64>,
    /// Autoscaler only: true while a scale-up decision is booked but its
    /// instances have not warmed up yet. Equivalent to `scalePhase ===
    /// 'warming'` and kept because it reads well as a boolean at a call
    /// site.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scaling: Option<bool>,

    // ---- region readouts ------------------------------------------------
    /// Region only: index of the region serving traffic right now,
    /// 0-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_region: Option<f64>,
    /// Region only: true while mid-failover, when traffic is being
    /// dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failing_over: Option<bool>,
    /// Region only: ms of dark window left before the failover lands and
    /// the next region starts serving. 0 whenever no failover is in
    /// progress, so a UI can draw a countdown exactly while `failingOver`
    /// is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover_remaining_ms: Option<f64>,
    /// Region only: how many of the declared regions are reachable right
    /// now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regions_healthy: Option<f64>,
    /// Region only: how many regions the node is switching between.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regions_total: Option<f64>,
    /// Region only: the id of the outgoing edge traffic is actually taking
    /// right now, or `None` when none is -- every region down, or
    /// mid-failover. The standby edges are the node's other outgoing
    /// edges; they are also marked individually in
    /// `SimSnapshot.edgeState`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_edge_id: Option<String>,
    /// Fraction of reads served from a replica that had not yet caught up
    /// with the latest write to that key, 0..1, over the current window.
    /// Replica nodes only. Rises with `replicationLagMs` and with write
    /// volume.
    pub stale_read_rate: f64,
    /// Utilisation of the single busiest shard, 0..1. Shard nodes only.
    /// With a hot key this pins at 1 while `utilization` (the mean across
    /// shards) still looks healthy -- that gap is the lesson.
    pub max_shard_utilization: f64,
    /// Utilisation of the least busy shard, 0..1. Shard nodes only.
    pub min_shard_utilization: f64,
    /// Per-shard utilisation, 0..1, indexed by shard number. Shard nodes
    /// only; empty for every other kind. Length tracks `shardCount`.
    pub shard_utilization: Vec<f64>,

    // ---- cdn readouts -----------------------------------------------------
    /// CDN only: requests per second this edge cache had to fetch from
    /// origin because they missed. This is the load that actually reaches
    /// your servers; offered minus this is what the CDN absorbed. The
    /// observed hit fraction is reported in the shared `hitRate` field
    /// above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_fetch_rate: Option<f64>,

    // ---- ratelimiter readouts -----------------------------------------
    /// Rate limiter only: requests per second admitted (a token was
    /// spent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admitted_rate: Option<f64>,
    /// Rate limiter only: requests per second rejected as 'throttled'.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub throttled_rate: Option<f64>,
    /// Rate limiter only: tokens sitting in the bucket right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<f64>,

    // ---- breaker readouts ------------------------------------------------
    /// Breaker only: the circuit's current state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breaker_state: Option<BreakerState>,
    /// Breaker only: downstream error fraction measured over the trailing
    /// `windowMs`, 0..1. This is the quantity compared against
    /// `errorThreshold` to decide whether to trip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breaker_error_rate: Option<f64>,
    /// Breaker only: requests per second failed fast without calling
    /// downstream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_rate: Option<f64>,
    /// Breaker only: times the circuit has tripped OPEN since sim start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breaker_trips: Option<f64>,
    /// Breaker only: ms left before an OPEN circuit moves to half-open and
    /// starts probing. 0 whenever the circuit is not open, so a UI can
    /// draw a countdown on exactly the phase that has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_remaining_ms: Option<f64>,

    // ---- searchindex readouts -------------------------------------------
    /// Search index only: fraction of searches over the current window
    /// that hit a key whose latest write had committed but was NOT yet
    /// searchable (still inside `indexLagMs`), 0..1. This is the indexing
    /// lag made visible: it rises with the lag and with write volume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_search_rate: Option<f64>,
    /// Search index only: searches served per second over the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_rate: Option<f64>,
    /// Search index only: writes (documents indexed) per second over the
    /// window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_write_rate: Option<f64>,

    // ---- timeseriesdb readouts -------------------------------------------
    /// Time-series store only: appends served per second over the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub append_rate: Option<f64>,
    /// Time-series store only: range queries served per second over the
    /// window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range_query_rate: Option<f64>,

    // ---- graphdb readouts --------------------------------------------------
    /// Graph database only: MEASURED mean service milliseconds per
    /// traversal over the current window, including the depth multiplier.
    /// Compare it to `serviceMs` to see what the configured depth is
    /// actually costing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub traversal_cost_ms: Option<f64>,

    // ---- vectordb readouts --------------------------------------------------
    /// Vector database only: MEASURED mean service milliseconds per query
    /// over the current window, including the index-size and recall
    /// multipliers. This is the number that moves when the recall slider
    /// does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_cost_ms: Option<f64>,

    // ---- streambroker readouts -------------------------------------------
    /// Stream broker only: how many messages the WORST consumer group is
    /// behind the head of the log, in messages, including deliveries
    /// still in flight. This is the headline metric of a partitioned log:
    /// it grows while a group's consumers are slower than the producers
    /// and drains back toward zero when they catch up. Also mirrored into
    /// `queued` so the generic backlog meter shows it against
    /// `queueLimit` (retention).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_lag: Option<f64>,
    /// Stream broker only: lag per consumer group, in messages, indexed by
    /// the group's outgoing-edge order. Length tracks the number of
    /// non-cut outgoing edges; groups are independent, which is the
    /// point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_lag_by_group: Option<Vec<f64>>,
    /// Stream broker only: messages delivered to consumers per second, all
    /// groups.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_rate: Option<f64>,
    /// Stream broker only: messages per second a lagging group SKIPPED
    /// because they aged out of retention (`queueLimit`) before it reached
    /// them. Data loss for that group, invisible to the producer. Nonzero
    /// means a consumer is not merely behind; it is losing data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention_drop_rate: Option<f64>,

    // ---- pubsub readouts ----------------------------------------------------
    /// Pub/sub only: subscriber edges currently fanned out to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fanout: Option<f64>,
    /// Pub/sub only: deliveries per second across all subscribers. With N
    /// subscribers this reads N times the publish rate, and that
    /// amplification is the entire lesson of the component.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish_amplification: Option<f64>,

    // ---- websocket readouts --------------------------------------------------
    /// Websocket only: connections held open right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections_open: Option<f64>,
    /// Websocket only: the connection ceiling, `instances * capacity`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<f64>,
    /// Websocket only: connections accepted per second over the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_rate: Option<f64>,
    /// Websocket only: connections refused per second because the ceiling
    /// was hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_reject_rate: Option<f64>,

    // ---- apigateway readouts ---------------------------------------------------
    /// API gateway only: requests per second refused as 'unauthorized'.
    /// The gateway's throttle readouts reuse `admittedRate`,
    /// `throttledRate` and `tokens` above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_reject_rate: Option<f64>,

    // ---- sidecar readouts -------------------------------------------------------
    // The sidecar's circuit readouts reuse `breakerState` and
    // `rejectedRate` above; its policy differs (consecutive failures, not
    // a windowed rate) but the states mean the same thing.
    // -----------------------------------------------------------------
    /// Sidecar only: consecutive downstream failures observed right now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consecutive_fails: Option<f64>,
    /// Sidecar only: downstream call failures per second it observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream_fail_rate: Option<f64>,

    // ---- lambda readouts ----------------------------------------------------------
    /// Lambda only: fraction of invocations over the window that paid a
    /// cold start, 0..1. High after idling (the pool went cold) and during
    /// bursts (arrivals outrun the warm pool); near zero under steady
    /// traffic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_start_rate: Option<f64>,
    /// Lambda only: cold starts per second over the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cold_starts_per_sec: Option<f64>,
    /// Lambda only: warm instances sitting idle, ready to serve instantly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warm_idle: Option<f64>,
    /// Lambda only: invocations running right now, against
    /// `maxConcurrency`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running_now: Option<f64>,

    // ---- cron readouts ---------------------------------------------------------------
    /// Cron only: ms until the job fires next.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire_in_ms: Option<f64>,
    /// Cron only: total requests emitted since sim start, all firings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_emitted: Option<f64>,
    /// Cron only: requests the next firing will emit, summed across every
    /// outgoing edge. `batchSize x edges`, mirrored so a readout can state
    /// the size of the burst without re-deriving it from config and
    /// wiring.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_size: Option<f64>,

    // ---- bulkhead readouts ----------------------------------------------------------
    /// Bulkhead only: downstream calls in flight through the pool right
    /// now.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_in_flight: Option<f64>,
    /// Bulkhead only: the pool's concurrency ceiling, mirrored from
    /// config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_limit: Option<f64>,
    /// Bulkhead only: requests per second refused because the pool was
    /// full.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_rejected_rate: Option<f64>,
    /// Bulkhead only: requests currently waiting to acquire a pool slot.
    /// Only ever nonzero with `bulkheadMode: Wait`. Port of
    /// `BulkheadState.waiters.length` (upstream `351327c4`, PR #77).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_waiting: Option<f64>,
    /// Bulkhead only: the most recently acquired waiter's measured wait, in
    /// ms, before it got a pool slot. `None` before the first acquire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_acquire_latency_ms: Option<f64>,
    /// Bulkhead only: acquire attempts per second that reached their
    /// `acquireTimeoutMs` deadline before a slot opened, and failed as
    /// 'acquire-timeout'.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bulkhead_acquire_timeout_rate: Option<f64>,

    // ---- retryqueue readouts -----------------------------------------------------------
    /// Retry queue only: messages per second delivered downstream
    /// successfully.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_rate: Option<f64>,
    /// Retry queue only: failed delivery attempts per second that were
    /// given another try. Rises before dead letters do, so it is the early
    /// warning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redelivery_rate: Option<f64>,
    /// Retry queue only: messages per second exhausting their retries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letter_rate: Option<f64>,
    /// Retry queue only: messages that exhausted every retry since sim
    /// start. This is the dead letter shelf: failures with somewhere to
    /// go, counted instead of vanished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dead_letters: Option<f64>,

    // ---- writebehind readouts -----------------------------------------------------------
    /// Write-behind cache only: acknowledged writes sitting dirty in the
    /// buffer right now (buffered plus mid-flush). Every one of these is a
    /// write the caller believes is safe and a crash of this node would
    /// lose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty_writes: Option<f64>,
    /// Write-behind cache only: writes per second landing on the backing
    /// store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flushed_rate: Option<f64>,
    /// Write-behind cache only: flushes per second that FAILED at the
    /// backing store. The caller was told "saved" long ago, so every one
    /// of these is a silently lost write, which is the risk the component
    /// trades for latency.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flush_fail_rate: Option<f64>,

    // ---- edgecompute readouts -----------------------------------------------------------
    /// Edge compute only: requests per second answered entirely at the
    /// edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_handled_rate: Option<f64>,
    /// Edge compute only: requests per second passed through to the
    /// origin path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passed_through_rate: Option<f64>,

    // ---- loadshedder readouts ------------------------------------------------------------
    // The shedder's bucket readouts reuse `admittedRate` and `tokens`
    // above; the fields below split admissions and drops by priority,
    // which is the readout that shows graceful degradation actually
    // happening.
    // -----------------------------------------------------------------
    /// Load shedder only: high-priority requests per second admitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_admitted_rate: Option<f64>,
    /// Load shedder only: low-priority requests per second admitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_admitted_rate: Option<f64>,
    /// Load shedder only: high-priority requests per second dropped
    /// ('throttled').
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high_shedded_rate: Option<f64>,
    /// Load shedder only: low-priority requests per second dropped
    /// ('deprioritized').
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_shedded_rate: Option<f64>,

    // ---- db readouts --------------------------------------------------------------------
    // The database's own mechanism is the write lock: reads share the
    // pool, writes also serialise against each other. These three numbers
    // are that mechanism measured, and they are why "add more instances"
    // fixes a slow read path but does nothing for a slow write path.
    // -----------------------------------------------------------------
    /// Database only: reads per second entering service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_rate: Option<f64>,
    /// Database only: writes per second entering service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_rate: Option<f64>,
    /// Database only: mean extra milliseconds a write spent waiting on
    /// lock contention, measured over the window. Grows with concurrent
    /// writers, not with fleet size, which is the whole lesson.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_wait_ms: Option<f64>,

    // ---- objectstore readouts -------------------------------------------------------------
    /// Object storage only: requests per second refused because one key
    /// prefix exceeded its per-prefix rate ceiling. The pool can be nearly
    /// idle while this is nonzero: the limit is per prefix, not per store,
    /// which is why hot-prefix layouts melt and spread ones do not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slowdown_rate: Option<f64>,

    // ---- coldstorage readouts --------------------------------------------------------------
    // Cold storage reuses `throttledRate` above for restore requests
    // refused because every retrieval slot was taken; `inFlight` is
    // restore jobs currently running. There is no queue: that absence is
    // the component.
    // ---- transcoder readouts ----------------------------------------------------------------
    /// Transcoder only: finished renditions per second handed downstream
    /// as detached uploads. `renditions` output files leave per finished
    /// job, so this runs at `renditions * jobRate`: the write
    /// amplification a video pipeline pays for its quality ladder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_rate: Option<f64>,

    // ---- edgecompute readouts -----------------------------------------------------------------
    /// Edge compute only: requests per second the edge WOULD have
    /// answered but could not, because their execution exceeded the
    /// per-request CPU budget (`cpuMsCap`); they fell through to the
    /// origin path instead. The hard CPU ceiling is what separates an edge
    /// runtime from a real server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_exceeded_rate: Option<f64>,
}

/// End-to-end results measured at the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStats {
    pub time_ms: f64,
    pub offered_rps: f64,
    pub goodput_rps: f64,
    pub error_rate: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub total_requests: f64,
    pub total_failed: f64,
}

/// Bulkhead admission policy once the pool is full. Port of
/// `behaviour-resilience.ts`'s `bulkheadMode` union, `'reject' | 'wait'`
/// (upstream `351327c4`, PR #77); `#[serde(rename_all = "lowercase")]`
/// reproduces those two literals exactly, the same convention `FailureKind`
/// below uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BulkheadMode {
    /// Fail a request immediately once the pool is full. The default.
    Reject,
    /// Hold a request in the bounded acquire queue instead of failing it
    /// immediately.
    Wait,
}

/// How a client's offered rate varies over time.
///
/// Each pattern scales the client's `rps` baseline; none of them replaces
/// it. The shapes are the ones a student meets first, and each one breaks a
/// system a different way:
///
/// - `Steady`  flat. The default, and the only one that was possible
///   before.
/// - `Ramp`    climbs from nothing to the baseline over one period, then
///   holds. A launch, or traffic arriving as a region wakes up.
/// - `Spike`   quiet at a tenth of the baseline, then a short burst well
///   above it. This is the one that finds queue limits.
/// - `Diurnal` a smooth day: a trough overnight, a peak in the afternoon.
///   Teaches that a system sized for the average is undersized for half
///   the day.
///
/// A thundering herd is deliberately NOT here. The other four are rate
/// curves, but a herd is correlated ARRIVALS: many callers retrying at the
/// same instant after a recovery. That is a different mechanism, not a
/// different curve, and modelling it as a tall thin spike would be a
/// plausible-looking lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrafficPattern {
    Steady,
    Ramp,
    Spike,
    Diurnal,
}

/// Mirrors the TS `TRAFFIC_PATTERNS` constant: every `TrafficPattern`
/// variant, in the order a UI should offer them.
pub const TRAFFIC_PATTERNS: [TrafficPattern; 4] = [
    TrafficPattern::Steady,
    TrafficPattern::Ramp,
    TrafficPattern::Spike,
    TrafficPattern::Diurnal,
];

/// Seconds for one cycle when a client does not say.
///
/// 60 is chosen so a reader watching in real time sees a whole cycle
/// without waiting: a full ramp, or a gap between two spikes, inside a
/// minute.
pub const DEFAULT_TRAFFIC_PERIOD_S: f64 = 60.0;

/// Why a request failed.
///
/// NOTE: the TS source (`src/sim/types.ts` lines 1225-1248) and the porting
/// instructions both describe this union as having 13 variants; the actual
/// source lists 14 (`error`, `shed`, `timeout`, `no-route`, `depth`,
/// `throttled`, `rejected`, `crashed`, `partitioned`, `region-down`,
/// `conn-refused`, `unauthorized`, `bulkhead-full`, `deprioritized`). This
/// port follows the actual source file exactly -- see the deviation note in
/// the porting agent's final report. A 15th variant, `AcquireTimeout`, was
/// added afterward by upstream `351327c4` (PR #77).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureReason {
    Error,
    Shed,
    Timeout,
    NoRoute,
    Depth,
    /// Refused by a rate limiter: no token was available in its bucket.
    Throttled,
    /// Refused by an open circuit breaker: failed fast without calling
    /// downstream.
    Rejected,
    /// The node is crashed by an injected failure, or was in flight when
    /// it crashed.
    Crashed,
    /// The edge the call needed is cut by an injected network partition.
    Partitioned,
    /// No region was serving: the active one is down and failover has not
    /// landed.
    RegionDown,
    /// Refused by a websocket gateway: every connection slot was already
    /// held.
    ConnRefused,
    /// Refused by an API gateway: the request failed authentication.
    Unauthorized,
    /// Refused by a bulkhead: its concurrency pool was already full.
    BulkheadFull,
    /// A bulkhead waiter exceeded its `acquireTimeoutMs` deadline before a
    /// pool slot opened. Only reachable with `bulkheadMode: Wait`; a
    /// `Reject` bulkhead fails as `BulkheadFull` instead, exactly as before
    /// this variant existed. Port of `351327c4` (PR #77).
    AcquireTimeout,
    /// Dropped by a load shedder protecting higher-priority traffic.
    Deprioritized,
}

/// All 15 `FailureReason` variants, in declaration order. Used to build
/// `empty_failures_by_reason()` and by anything that needs to enumerate
/// every reason without hand-maintaining a second list.
pub const ALL_FAILURE_REASONS: [FailureReason; 15] = [
    FailureReason::Error,
    FailureReason::Shed,
    FailureReason::Timeout,
    FailureReason::NoRoute,
    FailureReason::Depth,
    FailureReason::Throttled,
    FailureReason::Rejected,
    FailureReason::Crashed,
    FailureReason::Partitioned,
    FailureReason::RegionDown,
    FailureReason::ConnRefused,
    FailureReason::Unauthorized,
    FailureReason::BulkheadFull,
    FailureReason::AcquireTimeout,
    FailureReason::Deprioritized,
];

/// `Record<FailureReason, number>` in the TS source: a fixed-shape object
/// literal with all reasons always present as keys, used as a counter map.
/// Ported as a `HashMap` rather than a fixed struct because the engine
/// (`this.failures = { error: 0, shed: 0, ... }` in `engine.ts`) builds and
/// indexes it exactly like a map; `empty_failures_by_reason()` below is what
/// keeps "all reasons always present" true on the Rust side too.
pub type FailuresByReason = HashMap<FailureReason, u32>;

/// Mirrors `engine.ts`'s `this.failures = { error: 0, shed: 0, ... }` object
/// literal: every `FailureReason` present at 0, so a consumer can index any
/// reason without an `Option`/`unwrap_or_default` dance. The engine agent
/// (porting `engine.ts` separately) calls this once to seed its counters.
pub fn empty_failures_by_reason() -> FailuresByReason {
    let mut map = HashMap::with_capacity(ALL_FAILURE_REASONS.len());
    for reason in ALL_FAILURE_REASONS {
        map.insert(reason, 0);
    }
    map
}

// --------------------------------------------------------------------
// Failure injection
//
// A chaos control, not a component: any node can be given a fault without
// rewiring the topology, and clearing it restores the node exactly.
// --------------------------------------------------------------------

/// The kind of an injected failure. Which `FailureOpts` fields apply
/// depends on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureKind {
    /// The node is dead: in-flight work fails now, new work fails on
    /// arrival.
    Crash,
    /// The node still works, but every service time is multiplied by
    /// `factor`.
    Slow,
    /// The node returns errors at `rate`, on top of its configured
    /// `errorRate`.
    Errors,
    /// Named edges are cut: a request offered to one fails as
    /// 'partitioned'.
    Partition,
}

/// Knobs for an injected failure. Which ones apply depends on the kind.
///
/// Constructed from the frontend as a Tauri command argument, hence
/// `Default`: a command can start from `FailureOpts::default()` and fill in
/// only the fields the chosen `FailureKind` needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureOpts {
    /// 'slow' only: multiplier applied to the node's service time. 3 means
    /// every request there takes three times as long. Values below 1 are
    /// clamped to 1, since a fault may not make a node faster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<f64>,
    /// 'errors' only: fraction of requests forced to fail at this node,
    /// 0..1. Rolled independently of, and in addition to, `config.errorRate`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// 'partition' only: ids of the edges to cut. Omitted or empty means
    /// every edge leaving the node, which is the "unplug this box" case.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_ids: Option<Vec<String>>,
}

/// An injected failure as reported back to the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveFailure {
    /// Node the failure is attached to.
    pub node_id: String,
    pub kind: FailureKind,
    /// Simulated time in ms at which it was injected.
    pub since_ms: f64,
    /// 'slow' only: the service-time multiplier in force.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub factor: Option<f64>,
    /// 'errors' only: the forced failure fraction, 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// 'partition' only: the edge ids actually cut.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_ids: Option<Vec<String>>,
}

/// Why an edge is or is not carrying traffic right now.
///
/// `edgeFlow` alone cannot answer this. An edge at 0 rps because a breaker
/// in front of it is OPEN and an edge at 0 rps because nobody happens to be
/// calling look identical as numbers, and they mean opposite things: the
/// first is a component doing its job, the second is an idle link. Drawing
/// them the same is precisely the dishonesty this field exists to remove --
/// a severed wire should look severed.
///
/// Exactly one state applies per edge, resolved in the order listed here: a
/// cut link is reported as `Cut` even if a breaker upstream is also open,
/// because the injected fault is the more specific truth about that wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeState {
    /// Cut by an injected 'partition' failure. Nothing crosses it at all.
    Cut,
    /// The source node is refusing to send: a breaker OPEN or half-open
    /// past its probe budget. The link is fine; the component decided not
    /// to use it.
    Blocked,
    /// A region node's non-active edge. Wired, healthy, deliberately
    /// unused -- this is the standby region, and it must not look like a
    /// dead link.
    Standby,
    /// Carrying traffic now.
    Live,
    /// Wired and permitted, but nothing is flowing. Ordinary idleness.
    Idle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSnapshot {
    pub system: SystemStats,
    pub nodes: HashMap<String, NodeStats>,
    /// Recent history for sparklines, newest last.
    pub history: Vec<HistoryPoint>,
    /// Per-edge requests/sec, keyed by edge id.
    pub edge_flow: HashMap<String, f64>,
    /// Why each edge is in the state it is, keyed by edge id -- one entry
    /// per edge in the topology, always populated. Read alongside
    /// `edgeFlow`: the rate says how much, this says whether the wire is
    /// even usable.
    ///
    /// 'live' vs 'idle' is decided from the edge's own rate, so it
    /// flickers with traffic and is only a hint; 'cut', 'blocked' and
    /// 'standby' are structural assertions the engine stands behind, and
    /// are the ones worth drawing differently.
    pub edge_state: HashMap<String, EdgeState>,
    pub failures_by_reason: FailuresByReason,
    /// Injected failures in force right now, one entry per faulted node.
    /// Empty on a healthy system, so the UI can render a chaos indicator
    /// from this alone without asking the engine anything else.
    pub active_failures: Vec<ActiveFailure>,
    /// The most recent completed request, hop by hop, or `None` before one
    /// has finished.
    ///
    /// Every other number in this snapshot is an aggregate: a rate, a
    /// percentile, a mean. Those say latency ROSE without saying where it
    /// went, and a student reading "p99 400ms" cannot tell whether the
    /// work got slower or whether the request simply stood in line. This
    /// is the one place the simulator answers that, which is why it is a
    /// single traced request rather than a statistic: an average of
    /// queueing tells you less than one honest example of it.
    pub trace: Option<RequestTrace>,
}

/// One request's path, sampled and recorded end to end.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTrace {
    /// Simulated ms at which the client issued it.
    pub start_ms: f64,
    /// End-to-end latency the client measured.
    pub total_ms: f64,
    pub ok: bool,
    /// Why it failed, when it did.
    pub reason: Option<FailureReason>,
    pub hops: Vec<TraceHop>,
}

/// One node on a traced request's path.
///
/// `queuedMs` and `serviceMs` are kept apart deliberately. They are the
/// whole point: under load the second stays roughly flat while the first
/// grows without bound, and seeing those two bars move differently is the
/// lesson that no p99 reading can teach.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceHop {
    pub node_id: String,
    /// Hop depth from the client, 0 for the client itself.
    pub depth: f64,
    /// Waiting in line before a server slot was free.
    pub queued_ms: f64,
    /// Doing the work, once a slot was held.
    pub service_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPoint {
    pub t: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub goodput: f64,
    pub offered: f64,
    pub error_rate: f64,
}
