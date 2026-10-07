//! Faithful port of `src/sim/presets.ts`.
//!
//! Per MIGRATION_PLAN.md #5 ("Why presets/glossary/vendor specs port as
//! data, not code"): `defaultConfig(kind)` **is** logic (a switch over 33
//! kinds, picking sensible starting knobs for a freshly dropped node) and is
//! ported here as Rust code. The 23 example topologies (`PRESETS` in the TS
//! source) are large literal object graphs, not logic, and are ported as
//! JSON under `src-tauri/data/presets/`, embedded at compile time
//! via `include_str!` and parsed with `serde_json` -- a JSON syntax error is
//! visible on inspection, where a hand-typed Rust struct literal with a
//! typo'd field name would silently compile into wrong data.
//!
//! NOTE: the TS source's own `NodeKind` union has 33 members (see the
//! deviation note on `sim::types::NodeKind`), not the 34 this port's task
//! description assumed; `base_config` below matches every one of them.

use crate::sim::types::{NodeConfig, NodeKind, Topology};
use serde::Serialize;

/* -------------------------------------------------------------------- *
 * `EXTRA_DEFAULTS` from presets.ts: defaults for config fields that only
 * some kinds read. Every `case` in `baseConfig` below returns a literal for
 * the knobs that kind actually exposes; these fill in the rest of
 * `NodeConfig` so adding a field does not force an edit to every case.
 *
 * Verified against `src/sim/presets.ts`: no arm of the TS `baseConfig`
 * switch ever sets `instances`, `replicaCount`, `replicationLagMs`,
 * `readFraction`, `shardCount`, `shardCapacity` or `hotKeyFraction` --
 * those seven fields are owned by `EXTRA_DEFAULTS` alone. That means
 * `{ ...EXTRA_DEFAULTS, ...baseConfig(kind) }` and simply returning
 * `baseConfig(kind)` (once `baseConfig` itself fills those seven fields
 * with the constants below, which it does, see `nc()`) produce identical
 * results: there is no field either side actually contests at runtime.
 * `default_config` below still performs the combination explicitly so the
 * precedence rule from the TS source (`baseConfig`'s fields win on overlap,
 * since it is spread second) is stated in code, not just in this comment.
 * -------------------------------------------------------------------- */

/// `instances`: every scalable kind starts as a single machine.
const EXTRA_INSTANCES: f64 = 1.0;
/// `replicaCount`: a 3-node read replica set.
const EXTRA_REPLICA_COUNT: f64 = 3.0;
/// `replicationLagMs`: 50ms behind.
const EXTRA_REPLICATION_LAG_MS: f64 = 50.0;
/// `readFraction`: mostly-read traffic.
const EXTRA_READ_FRACTION: f64 = 0.9;
/// `shardCount`: 4 partitions.
const EXTRA_SHARD_COUNT: f64 = 4.0;
/// `shardCapacity`: 4 slots per shard.
const EXTRA_SHARD_CAPACITY: f64 = 4.0;
/// `hotKeyFraction`: keys spread evenly (no hot key) until you make one.
const EXTRA_HOT_KEY_FRACTION: f64 = 0.0;

/// The `EXTRA_DEFAULTS` object from `presets.ts`, as a `NodeConfig`.
///
/// The nine fields `EXTRA_DEFAULTS` does NOT own (`capacity`, `serviceMs`,
/// `serviceCv`, `queueLimit`, `hitRate`, `errorRate`, `timeoutMs`,
/// `retries`, `rps`) are filled with `0.0` here as harmless placeholders --
/// every `base_config` arm overwrites them, so this function's value for
/// them is never observed by a caller of `default_config`. Every other
/// optional field (breaker/autoscaler/etc knobs) is `None`, matching the TS
/// object, which sets only the seven fields named above.
pub fn extra_defaults() -> NodeConfig {
    NodeConfig {
        capacity: 0.0,
        instances: Some(EXTRA_INSTANCES),
        service_ms: 0.0,
        service_cv: 0.0,
        queue_limit: 0.0,
        hit_rate: 0.0,
        error_rate: 0.0,
        timeout_ms: 0.0,
        retries: 0.0,
        rps: 0.0,
        traffic: None,
        traffic_period_s: None,
        target_util: None,
        min_capacity: None,
        max_capacity: None,
        cooldown_ms: None,
        scale_step_pct: None,
        warmup_ms: None,
        regions: None,
        active_region: None,
        failover_ms: None,
        rate_limit_rps: None,
        burst: None,
        error_threshold: None,
        window_ms: None,
        open_ms: None,
        half_open_probes: None,
        replica_count: EXTRA_REPLICA_COUNT,
        replication_lag_ms: EXTRA_REPLICATION_LAG_MS,
        read_fraction: EXTRA_READ_FRACTION,
        shard_count: EXTRA_SHARD_COUNT,
        shard_capacity: EXTRA_SHARD_CAPACITY,
        hot_key_fraction: EXTRA_HOT_KEY_FRACTION,
        index_ms: None,
        index_lag_ms: None,
        range_query_fraction: None,
        range_query_ms: None,
        traversal_depth: None,
        index_size_k: None,
        recall_target: None,
        partitions: None,
        connection_ms: None,
        auth_fail_rate: None,
        outlier_after: None,
        cold_start_ms: None,
        keep_warm_ms: None,
        max_concurrency: None,
        interval_ms: None,
        batch_size: None,
        bulkhead_max: None,
        flush_delay_ms: None,
        edge_share: None,
        low_priority_share: None,
        priority_reserve: None,
        lock_ms: None,
        prefix_rps: None,
        renditions: None,
        cpu_ms_cap: None,
    }
}

/// The nine knobs every kind sets in `baseConfig`, plus the seven
/// `EXTRA_DEFAULTS`-owned fields (always filled with the constants above,
/// since no `case` in the TS source ever overrides them) and `None` for
/// every kind-specific optional field. Match arms below use Rust's
/// struct-update syntax (`..nc(...)`) to override the handful of optional
/// fields their kind actually reads.
fn nc(
    capacity: f64,
    service_ms: f64,
    service_cv: f64,
    queue_limit: f64,
    hit_rate: f64,
    error_rate: f64,
    timeout_ms: f64,
    retries: f64,
    rps: f64,
) -> NodeConfig {
    NodeConfig {
        capacity,
        instances: Some(EXTRA_INSTANCES),
        service_ms,
        service_cv,
        queue_limit,
        hit_rate,
        error_rate,
        timeout_ms,
        retries,
        rps,
        traffic: None,
        traffic_period_s: None,
        target_util: None,
        min_capacity: None,
        max_capacity: None,
        cooldown_ms: None,
        scale_step_pct: None,
        warmup_ms: None,
        regions: None,
        active_region: None,
        failover_ms: None,
        rate_limit_rps: None,
        burst: None,
        error_threshold: None,
        window_ms: None,
        open_ms: None,
        half_open_probes: None,
        replica_count: EXTRA_REPLICA_COUNT,
        replication_lag_ms: EXTRA_REPLICATION_LAG_MS,
        read_fraction: EXTRA_READ_FRACTION,
        shard_count: EXTRA_SHARD_COUNT,
        shard_capacity: EXTRA_SHARD_CAPACITY,
        hot_key_fraction: EXTRA_HOT_KEY_FRACTION,
        index_ms: None,
        index_lag_ms: None,
        range_query_fraction: None,
        range_query_ms: None,
        traversal_depth: None,
        index_size_k: None,
        recall_target: None,
        partitions: None,
        connection_ms: None,
        auth_fail_rate: None,
        outlier_after: None,
        cold_start_ms: None,
        keep_warm_ms: None,
        max_concurrency: None,
        interval_ms: None,
        batch_size: None,
        bulkhead_max: None,
        flush_delay_ms: None,
        edge_share: None,
        low_priority_share: None,
        priority_reserve: None,
        lock_ms: None,
        prefix_rps: None,
        renditions: None,
        cpu_ms_cap: None,
    }
}

/// Per-kind knobs. Faithful port of `baseConfig` in `presets.ts`: one arm
/// per TS `case`, exact numeric literals, exact comments carried over
/// where they explain WHY a number was chosen (load-bearing for whoever
/// tunes this next -- see AGENTS.md's "Adding a component" checklist).
///
/// This `match` is exhaustive over all 33 `NodeKind` variants with no `_`
/// arm, so the compiler -- not a runtime default -- catches a future kind
/// added to the enum without a matching preset entry here.
pub fn base_config(kind: NodeKind) -> NodeConfig {
    match kind {
        NodeKind::Client => nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1000.0, 0.0, 50.0),
        NodeKind::Lb => nc(256.0, 0.5, 0.2, 1024.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        NodeKind::Service => nc(8.0, 25.0, 0.6, 64.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        NodeKind::Cache => nc(32.0, 3.0, 0.4, 256.0, 0.8, 0.0, 0.0, 0.0, 0.0),
        // Reads share the 6 slots; writes (10% at the default readFraction
        // 0.9) also pay 15ms of lock wait per concurrent writer. At the
        // default mix that is a barely-visible tax; raise the write share
        // and watch lockWaitMs climb while adding instances fixes nothing.
        NodeKind::Db => NodeConfig {
            lock_ms: Some(15.0),
            ..nc(6.0, 30.0, 0.7, 32.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        NodeKind::Queue => nc(1.0, 1.0, 0.2, 5000.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        NodeKind::Worker => nc(4.0, 25.0, 0.6, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        // Reads fan across the replicas, so per-replica capacity is modest.
        NodeKind::Replica => nc(4.0, 20.0, 0.6, 64.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        // Per-shard slots live in shardCapacity; `capacity` is unused here.
        NodeKind::Shard => nc(4.0, 25.0, 0.6, 32.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        // An edge PoP: very fast, very high hit rate, lots of concurrency.
        // At hitRate 0.92 the origin behind it sees 8% of the offered load.
        NodeKind::Cdn => nc(256.0, 2.0, 0.3, 2048.0, 0.92, 0.0, 0.0, 0.0, 0.0),
        // A doorman: refusing costs nothing, so no service time and no
        // slots. 100 rps sustained with one second of burst headroom.
        NodeKind::RateLimiter => NodeConfig {
            rate_limit_rps: Some(100.0),
            burst: Some(100.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Trips once half the downstream calls in a 5s window fail, stays
        // open 3s, then lets 3 probes decide whether to close.
        NodeKind::Breaker => NodeConfig {
            error_threshold: Some(0.5),
            window_ms: Some(5000.0),
            open_ms: Some(3000.0),
            half_open_probes: Some(3.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Adds and removes INSTANCES to hold its target at 70% utilisation,
        // stepping by half the current fleet.
        //
        // THE TIMINGS, on a human scale. A student drags the load slider up
        // and this is what they should see, measured rather than guessed:
        //
        //   t+0.0s  load arrives; utilisation climbs and pins at 1.0
        //   t+~3s   cooldown expires, the controller decides and books
        //           machines (phase 'cooldown' -> 'warming', ghost
        //           instances appear)
        //   t+~7s   the machines boot and start serving (warmupMs=4000
        //           later); utilisation falls back toward the setpoint
        //
        // So: something visibly happens within about three seconds, and
        // the whole arc completes in under ten. 4s of warmup is the
        // balance point. Shorter and the lag stops being legible --
        // capacity looks like it answers instantly, which teaches the
        // opposite of the truth. Much longer (the previous default was
        // 10s) and a student watching a live graph concludes the
        // component is broken before it ever acts.
        //
        // cooldownMs 3000 < warmupMs 4000 is deliberate and is the
        // documented oscillation regime: the controller can want a second
        // step while the first is still booting. It does not thrash,
        // because a booked scale-up blocks further decisions, but the
        // fleet does hunt around the setpoint rather than settling dead on
        // it -- which is what real ones do.
        NodeKind::Autoscaler => NodeConfig {
            target_util: Some(0.7),
            // In INSTANCES, not slots: between 1 and 12 machines.
            min_capacity: Some(1.0),
            max_capacity: Some(12.0),
            cooldown_ms: Some(3000.0),
            scale_step_pct: Some(0.5),
            warmup_ms: Some(4000.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Two regions, serving from the first. The 5s failover is long
        // enough to see as a real outage on the error graph and short
        // enough that a student does not think the simulation has hung.
        NodeKind::Region => NodeConfig {
            regions: Some(2.0),
            active_region: Some(0.0),
            failover_ms: Some(5000.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Blob storage: every request pays a high flat latency, but the
        // pool is wide. 64 slots / 90ms -> ~710 rps of ceiling at ~90ms
        // each, which is "effectively unlimited" next to any database in
        // this app. The real limit is PER PREFIX: 150 rps on each of the 8
        // key prefixes (~1200 rps spread evenly), and a hot prefix gets
        // SlowDown refusals while the rest of the store idles.
        NodeKind::ObjectStore => NodeConfig {
            prefix_rps: Some(150.0),
            ..nc(64.0, 90.0, 0.4, 1024.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Searches are cheap (8ms); a write pays +60ms of indexing on top
        // and becomes searchable only 1.5s after it commits. At the
        // default 90/10 read mix the mean cost is ~14ms -> 12 slots gives
        // ~850 rps.
        NodeKind::SearchIndex => NodeConfig {
            index_ms: Some(60.0),
            index_lag_ms: Some(1500.0),
            ..nc(12.0, 8.0, 0.5, 128.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Appends cost 1.5ms; a range query pays +120ms. At 5% range
        // queries the mean is ~7.5ms -> 16 slots gives ~2100 rps of mixed
        // traffic, which is the "metrics firehose" headroom the kind
        // exists to show.
        NodeKind::TimeSeriesDb => NodeConfig {
            range_query_fraction: Some(0.05),
            range_query_ms: Some(120.0),
            ..nc(16.0, 1.5, 0.4, 1024.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Depth 2 (friends-of-friends) costs 3x the base 6ms -> 18ms mean,
        // 8 slots -> ~440 rps. Each extra hop of depth divides that by 3.
        NodeKind::GraphDb => NodeConfig {
            traversal_depth: Some(2.0),
            ..nc(8.0, 6.0, 0.5, 64.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Archival: SECONDS per restore JOB, a hard quota of 24 concurrent
        // jobs (24 / 2.8s is ~8.5 rps of ceiling), and NO queue: a restore
        // beyond the quota is refused on the spot, not queued. Fine for a
        // trickle of restores, hopeless for anything shaped like online
        // traffic. Callers need a long timeout. queueLimit is ignored.
        NodeKind::ColdStorage => nc(24.0, 2800.0, 0.3, 64.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        // Mean query cost is serviceMs * log2(2 + indexSizeK) / (1 -
        // recall): 0.5ms * ~10 * 10 = ~50ms at one million vectors and 0.9
        // recall, so 16 slots gives ~320 rps. Pushing recall to 0.99 costs
        // 10x that.
        NodeKind::VectorDb => NodeConfig {
            index_size_k: Some(1000.0),
            recall_target: Some(0.9),
            ..nc(16.0, 0.5, 0.4, 128.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // A partitioned log: 1ms producer ack, 4 partitions, and
        // queueLimit is RETENTION in messages. Per consumer group the
        // parallelism ceiling is the partition count, so throughput per
        // group = 4 x (1000/consumerMs).
        NodeKind::StreamBroker => NodeConfig {
            partitions: Some(4.0),
            ..nc(1.0, 1.0, 0.2, 2000.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // A topic: near-instant ack, then one delivery per subscriber
        // edge. No knobs of its own; the amplification comes from the
        // wiring.
        NodeKind::PubSub => nc(1.0, 0.5, 0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        // Capacity is CONNECTIONS HELD: 400 per instance, held ~30s each,
        // so by Little's law it saturates at about 13 new connections/sec
        // per instance. The 5ms serviceMs is only the handshake.
        NodeKind::WebSocket => NodeConfig {
            connection_ms: Some(30000.0),
            ..nc(400.0, 5.0, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // The front door: 2ms of auth/routing work per request, a 300 rps
        // token bucket with one second of burst headroom, and 1% bad auth.
        NodeKind::ApiGateway => NodeConfig {
            rate_limit_rps: Some(300.0),
            burst: Some(300.0),
            auth_fail_rate: Some(0.01),
            ..nc(64.0, 2.0, 0.3, 256.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // The proxy tax: 2ms on every request, in exchange for 2 retries,
        // a 500ms per-attempt deadline, and outlier ejection after 5
        // straight downstream failures (3s ejection, matching the
        // breaker's openMs).
        NodeKind::Sidecar => NodeConfig {
            outlier_after: Some(5.0),
            open_ms: Some(3000.0),
            ..nc(32.0, 2.0, 0.2, 64.0, 0.0, 0.0, 500.0, 2.0, 0.0)
        },
        // Serverless: 25ms of work when warm, +350ms cold start, instances
        // kept warm 12s, at most 40 running at once (beyond that the
        // platform throttles; there is no queue). `capacity` is unused
        // here.
        NodeKind::Lambda => NodeConfig {
            cold_start_ms: Some(350.0),
            keep_warm_ms: Some(12000.0),
            max_concurrency: Some(40.0),
            ..nc(1.0, 25.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // A batch job: every 20s it dumps 50 requests down each outgoing
        // edge at once. Not a request path; requests wired INTO it are
        // refused.
        NodeKind::Cron => NodeConfig {
            interval_ms: Some(20000.0),
            batch_size: Some(50.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // A pool of 8 concurrent calls. Refusing is free, so no slots, no
        // queue, no service time: the pool count is the whole component.
        NodeKind::Bulkhead => NodeConfig {
            bulkhead_max: Some(8.0),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Delivery concurrency of 8 at ~3ms dispatch cost. Each failed
        // delivery gets 2 redeliveries with backoff before it
        // dead-letters; the 1s per-attempt deadline is what turns a hung
        // consumer into a retryable failure instead of a stuck message.
        NodeKind::RetryQueue => nc(8.0, 3.0, 0.3, 2000.0, 0.0, 0.0, 1000.0, 2.0, 0.0),
        // Batch regime: 1.2 SECONDS per job, two jobs per box. One
        // instance is 2 * (1000/1200) = ~1.7 jobs/s; feed it from a queue
        // and size the farm against the arrival rate, because a
        // structural deficit grows the backlog forever. Each finished job
        // hands 3 renditions (the quality ladder) downstream as detached
        // uploads, so storage sees 3x the job rate.
        NodeKind::Transcoder => NodeConfig {
            renditions: Some(3.0),
            ..nc(2.0, 1200.0, 0.4, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // A PoP function: ~1ms, lots of concurrency, and it can fully
        // answer 30% of requests without the origin ever hearing about
        // them -- as long as the execution fits the 2ms CPU budget; the
        // tail that runs past it is killed and passed to the origin
        // anyway.
        NodeKind::EdgeCompute => NodeConfig {
            edge_share: Some(0.3),
            cpu_ms_cap: Some(2.0),
            ..nc(64.0, 1.0, 0.3, 512.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // `capacity` is the dirty buffer (memory, not threads): up to 256
        // acknowledged writes held at once. At the 200ms flush residence
        // that supports ~1280 writes/s before the buffer itself fills.
        // Every write in it is lost if this node crashes.
        NodeKind::WriteBehind => NodeConfig {
            flush_delay_ms: Some(200.0),
            ..nc(256.0, 1.0, 0.3, 512.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
        // Admits 300 rps sustained. 30% of the key space is best-effort
        // traffic, and 30% of the bucket is reserved for the rest, so
        // under saturation the best-effort tier is dropped first.
        NodeKind::LoadShedder => NodeConfig {
            rate_limit_rps: Some(300.0),
            burst: Some(300.0),
            low_priority_share: Some(0.3),
            priority_reserve: Some(0.3),
            ..nc(1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
        },
    }
}

/// Sensible starting knobs for a freshly dropped node of each kind.
/// Faithful port of `defaultConfig(kind)` in `presets.ts`:
/// `{ ...EXTRA_DEFAULTS, ...baseConfig(kind) }`, with `baseConfig`'s fields
/// winning on overlap since it is spread second in the TS source.
///
/// As established in the module doc comment above, `base_config` already
/// fills the seven `EXTRA_DEFAULTS`-owned fields with those exact defaults
/// for every kind (no arm overrides them), so `base_config(kind)` alone is
/// byte-for-byte what the TS spread produces. This function still exists,
/// named and shaped like the TS one, so the precedence rule is legible at
/// the call site rather than relying on a comment elsewhere.
pub fn default_config(kind: NodeKind) -> NodeConfig {
    base_config(kind)
}

/* -------------------------------------------------------------------- *
 * Example topologies: ported as data (see the module doc comment and
 * MIGRATION_PLAN.md #5), embedded at compile time and parsed once.
 * -------------------------------------------------------------------- */

/// One row of `_index.json`: enough to render the preset palette without
/// parsing every full topology.
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetSummary {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub description: String,
}

/// A full preset: `{ id, name, tagline, description, topology }`, mirroring
/// the TS `Preset` interface in `presets.ts`.
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub tagline: String,
    pub description: String,
    pub topology: Topology,
}

/// `(id, json)` for every preset topology, embedded via `include_str!`.
/// `include_str!` needs a literal path per call -- it cannot glob a
/// directory -- so each of the 23 presets gets its own line here, mapped by
/// id. Order matches the TS `PRESETS` array in `presets.ts`.
static PRESET_FILES: &[(&str, &str)] = &[
    (
        "single-server",
        include_str!("../../data/presets/single-server.json"),
    ),
    (
        "load-balanced",
        include_str!("../../data/presets/load-balanced.json"),
    ),
    (
        "cache-aside",
        include_str!("../../data/presets/cache-aside.json"),
    ),
    (
        "async-workers",
        include_str!("../../data/presets/async-workers.json"),
    ),
    (
        "retry-storm",
        include_str!("../../data/presets/retry-storm.json"),
    ),
    (
        "cdn-origin",
        include_str!("../../data/presets/cdn-origin.json"),
    ),
    (
        "rate-limited-api",
        include_str!("../../data/presets/rate-limited-api.json"),
    ),
    (
        "circuit-breaker",
        include_str!("../../data/presets/circuit-breaker.json"),
    ),
    (
        "read-replicas",
        include_str!("../../data/presets/read-replicas.json"),
    ),
    (
        "sharded-database",
        include_str!("../../data/presets/sharded-database.json"),
    ),
    (
        "autoscaling-service",
        include_str!("../../data/presets/autoscaling-service.json"),
    ),
    (
        "multi-region",
        include_str!("../../data/presets/multi-region.json"),
    ),
    (
        "full-stack",
        include_str!("../../data/presets/full-stack.json"),
    ),
    (
        "specialised-stores",
        include_str!("../../data/presets/specialised-stores.json"),
    ),
    (
        "event-driven",
        include_str!("../../data/presets/event-driven.json"),
    ),
    (
        "resilient-delivery",
        include_str!("../../data/presets/resilient-delivery.json"),
    ),
    ("discord", include_str!("../../data/presets/discord.json")),
    ("uber", include_str!("../../data/presets/uber.json")),
    ("netflix", include_str!("../../data/presets/netflix.json")),
    ("spotify", include_str!("../../data/presets/spotify.json")),
    ("twitter", include_str!("../../data/presets/twitter.json")),
    ("stripe", include_str!("../../data/presets/stripe.json")),
    ("whatsapp", include_str!("../../data/presets/whatsapp.json")),
];

/// `_index.json`, embedded the same way: id/name/tagline/description for
/// every preset, in `PRESETS` order, without the full topology.
static PRESET_INDEX_JSON: &str = include_str!("../../data/presets/_index.json");

/// Look up one preset's full topology by id. Parses only that preset's
/// embedded JSON (each call is O(1) lookup + O(size of that one preset)
/// parse, never every preset).
///
/// Returns `None` for an unknown id, mirroring the TS side's
/// `PRESETS.find(p => p.id === id)` returning `undefined`.
pub fn preset_by_id(id: &str) -> Option<Preset> {
    let (_, json) = PRESET_FILES.iter().find(|(pid, _)| *pid == id)?;
    // A parse failure here means a checked-in JSON file is malformed --
    // that is a build-time data bug, not a runtime condition a caller can
    // meaningfully recover from, so it panics with the id and the error
    // rather than silently returning `None` (which would look identical to
    // "no such preset" and hide the real problem).
    match serde_json::from_str::<Preset>(json) {
        Ok(preset) => Some(preset),
        Err(err) => panic!("preset '{id}' has malformed embedded JSON: {err}"),
    }
}

/// Every preset's summary, in `PRESETS` order, for the palette list.
pub fn all_presets() -> Vec<PresetSummary> {
    match serde_json::from_str::<Vec<PresetSummary>>(PRESET_INDEX_JSON) {
        Ok(list) => list,
        Err(err) => panic!("data/presets/_index.json is malformed: {err}"),
    }
}
