//! Discrete-event simulation core. Port of `src/sim/engine.ts`
//! (MIGRATION_PLAN.md #9, item 2 -- the highest-risk, lowest-verifiability
//! piece of this whole migration, per that section's own ordering).
//!
//! `AGENTS.md`'s contract -- "same seed and topology means byte-identical
//! snapshots" -- is the bar every method below was ported against: event
//! ordering, RNG draw order, and float-arithmetic order are preserved
//! operation-for-operation wherever Rust's ownership model did not force a
//! different shape. Where it did, the deviation is a restructuring of HOW
//! state is threaded between calls, never of WHAT is computed, and each one
//! is called out below and in the porting report.
//!
//! ## Request identity: `RequestPool` replaces TS's `acquireReq`/`recycle`
//!
//! TS pools `Req` objects in a free-list, but because JS objects are
//! reference types, holding one in an array (`state.waiting: Req[]`) is
//! holding the SAME object -- identity (in particular the `parent` chain,
//! how a result ever bubbles back to its original caller) survives a
//! queueing delay for free. `Req.token` exists only so a stale event-heap
//! timer can recognize its request was since recycled
//! (`ev.req.token === ev.token`).
//!
//! Rust has no such free aliasing, so every live `Req` here is owned by one
//! `RequestPool` slab and addressed everywhere else by `ReqHandle { index,
//! generation }` (already defined in `engine_types.rs`, which this file is
//! what makes real: `handle_of`/`serve_deferred`/`add_service_delay_deferred`
//! are implemented below). `ReqHandle.generation` IS the Rust form of
//! `Req.token`: `RequestPool::get`/`get_mut` return `None` for a handle whose
//! generation no longer matches the slot's, which is the exact check
//! `ev.req.token === ev.token` was performing -- so `Ev` here carries no
//! separate token field at all; a stale event is simply one whose
//! `Option<ReqHandle>` no longer resolves.
//!
//! `NodeState.waiting` is `VecDeque<ReqHandle>`, not an array with a
//! `waitHead` cursor: `VecDeque::pop_front` is already O(1), so TS's
//! `compactWaiting()` (periodic `splice` to reclaim a consumed prefix) is
//! dead weight here and is dropped entirely -- the same simplification
//! already applied the same way in `behaviour/data.rs` and
//! `behaviour/store.rs`.
//!
//! ## Borrow-checker shape: views and handles, not long-lived references
//!
//! TS threads `state: NodeState` and `req: Req` object references freely
//! through deeply recursive calls, including calls that hand `this` (the
//! whole engine, as `BehaviourCtx`) to a behaviour hook while a `state`/`req`
//! reference is still "held" -- free in JS, impossible in Rust once `ctx:
//! &mut dyn BehaviourCtx` IS `&mut self` and `state`/`req` would have to stay
//! borrowed from inside `self` across that same call.
//!
//! This file resolves that with three small, deliberate devices, used
//! uniformly everywhere the conflict would otherwise arise:
//!
//! 1. **`node_id: &str` / `ReqHandle` instead of `&NodeState` / `&Req`**
//!    between private `Engine` methods. Cheap to pass, and the real data is
//!    looked up fresh from `self.nodes` / `self.pool` at the point each
//!    method actually needs it -- trading a hash lookup for compatibility
//!    with the recursive, re-entrant call shape the TS logic genuinely has
//!    (this reentrancy is real, not a bug to design away: see
//!    `behaviour/messaging.rs`'s streambroker `pumping` guard, which exists
//!    precisely because a behaviour's own `emit_detached` call can
//!    synchronously call back into that same node's `on_downstream_result`).
//! 2. **`NodeView`/`ReqView`**: cheap owned snapshots of the handful of
//!    fields `NodeStateLike`/`ReqLike` expose, built fresh at each call into
//!    a behaviour hook instead of borrowing `&NodeState`/`&Req` out of
//!    `self.nodes`/`self.pool` for the duration of that call. Nothing is
//!    lost: the real `NodeState`/`Req` stay where they are, and any mutation
//!    a behaviour wants happens through `ctx`'s methods (which look the real
//!    data up again), never through the view.
//! 3. **`with_node_ext`**: a node's `ext` is `std::mem::take`n out of
//!    `NodeState` for the duration of a behaviour hook call (which needs
//!    `ext: &mut Ext` alongside `ctx: &mut dyn BehaviourCtx`, i.e. `&mut
//!    self` -- the one piece of per-node state that genuinely cannot be
//!    cloned into a view, since a behaviour's persistent state must be the
//!    SAME value after the call, not a copy) and put back afterward. The
//!    Rust analogue of "the object reference is still the same object" for
//!    exactly the one field where that property has to be real, not
//!    observed.
//!
//! None of this changes what is computed, only how the engine gets from one
//! piece of state to the next while computing it.

use crate::sim::behaviour::{
    behaviour_for, clamp01, AdmitAction, CompleteAction, ComponentBehaviour, Ext, InstanceModel,
    PumpMode, RouteMode,
};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqHandle, ReqLike};
use crate::sim::heap::{MinHeap, Timed};
use crate::sim::random::Rng;
use crate::sim::types::{
    empty_failures_by_reason, ActiveFailure, EdgeState, FailureKind, FailureOpts, FailureReason,
    FailuresByReason, HistoryPoint, NodeConfig, NodeKind, NodeStats, RequestTrace, SimEdge,
    SimNode, SimSnapshot, SystemStats, Topology, TraceHop, TrafficPattern,
    DEFAULT_TRAFFIC_PERIOD_S,
};
use std::collections::{HashMap, HashSet, VecDeque};

/* ------------------------------------------------------------------ *
 * Tuning constants
 * ------------------------------------------------------------------ */

/// Wall-clock delta is clamped to this so a backgrounded tab cannot
/// death-spiral.
const MAX_DELTA_MS: f64 = 100.0;
/// Hard ceiling on events processed per `advance()` call.
const MAX_EVENTS_PER_ADVANCE: u32 = 60000;
/// Hop-depth ceiling; deeper resolves as `FailureReason::Depth`.
const MAX_HOP_DEPTH: u32 = 32;
/// Instance-count ceiling, matching the Inspector's own slider. The engine
/// writes one array element per instance on every snapshot
/// (`fill_slot_instances`/`finish_instances`), so an unbounded count is an
/// unbounded allocation -- and a design does not only come from the editor:
/// a shared link, a `.breakscale` file and a restored session all carry
/// `instances` straight through with no validation of their own.
/// engine.ts's `MAX_INSTANCES`, PR #58.
const MAX_INSTANCES: f64 = 512.0;
/// Trailing window for latency percentiles.
const LATENCY_WINDOW_MS: f64 = 5000.0;
/// Capacity of each latency ring buffer.
const LATENCY_RING: usize = 4096;
/// Max samples sorted per percentile computation; beyond this the window is
/// strided.
const PERCENTILE_SAMPLE_CAP: usize = 512;

/* Traffic pattern shape. Chosen so a spike's MEAN over one cycle stays near
   the baseline: the reader compares the same work arriving unevenly, not
   more of it. 0.9 quiet at 0.1x plus 0.1 loud at 4x averages 0.49x, near
   enough that the comparison is about burstiness rather than volume. */
const SPIKE_QUIET_FRACTION: f64 = 0.9;
const SPIKE_PEAK: f64 = 4.0;
const SPIKE_TROUGH: f64 = 0.1;

/* A day swings between a quarter of the baseline and twice it. */
const DIURNAL_TROUGH: f64 = 0.25;
const DIURNAL_PEAK: f64 = 2.0;

const RATE_WINDOW_MS: f64 = 1000.0;
/// Number of buckets the rate window is split into.
const RATE_BUCKETS: usize = 10;
/// How often a `HistoryPoint` is appended.
const HISTORY_INTERVAL_MS: f64 = 250.0;
/// How many `HistoryPoint`s are retained (~60s).
const HISTORY_MAX: usize = 240;
/// Base delay before a retry is re-issued; doubles per attempt, with jitter.
const RETRY_BASE_BACKOFF_MS: f64 = 25.0;
/// Guard so a runaway topology cannot allocate unbounded requests.
const MAX_LIVE_REQUESTS: u32 = 200_000;
/// Size of the keyspace every client draws request keys from. Deliberately
/// small -- see `src/sim/engine.ts`'s doc comment on the same constant, and
/// `behaviour/data.rs`'s `KEYSPACE`, which must agree with this one.
const KEYSPACE: u32 = 64;

/* ------------------------------------------------------------------ *
 * Event kinds
 * ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvKind {
    Arrival,
    ServiceDone,
    Timeout,
    WorkerPoll,
    Retry,
    /// A request finished traversing an edge with a `latencyMs` and is now
    /// offered.
    LinkArrive,
    /// A behaviour-owned timer for a request it held at admission (the
    /// bulkhead's acquire queue, so far the only user). Port of
    /// `engine.ts`'s `EV_BEHAVIOUR_WAKE` (upstream `351327c4`, PR #77).
    BehaviourWake,
}

/// One scheduled event. See the module doc ("Request identity") for why
/// this carries no separate token/guard field the way TS's `Ev.token` does
/// for every event -- `ReqHandle.generation` already covers every
/// request-bearing kind. `Arrival` events carry no `ReqHandle` at all, so
/// they need their own staleness guard: `arrival_generation`, the Rust
/// analogue of `Ev.token` narrowed to the one case that still needs it (PR
/// #33). See `push_arrival`.
struct Ev {
    time: f64,
    seq: u64,
    kind: EvKind,
    /// Node this event targets.
    node_id: String,
    /// Request this event concerns; `None` for arrivals and worker polls.
    req: Option<ReqHandle>,
    /// Set only on an `Arrival` event, to the generating `NodeState`'s
    /// `arrival_generation` at schedule time. `None` for every other kind.
    /// Compared against the node's CURRENT generation at dispatch so a
    /// stale arrival from a removed or retyped client is dropped instead of
    /// firing against whatever now holds that id.
    arrival_generation: Option<u32>,
}

impl Timed for Ev {
    fn time(&self) -> f64 {
        self.time
    }
    fn seq(&self) -> u64 {
        self.seq
    }
}

/* ------------------------------------------------------------------ *
 * Request object (pooled)
 * ------------------------------------------------------------------ */

/// A pooled request. Owned by `RequestPool`; referenced everywhere else by
/// `ReqHandle`. Does not need TS's `token` (the pool slot's `generation`
/// plays that role) or `next` (free-list threading lives in
/// `RequestPool::free`, not in the struct itself).
struct Req {
    node_id: String,
    /// Parent request that issued this call, or `None` for a client root /
    /// queue message.
    parent: Option<ReqHandle>,
    /// Number of child calls still outstanding.
    pending: u32,
    /// Largest latency reported by any resolved child (fan-out join).
    max_child_ms: f64,
    /// True once any child failed.
    child_failed: bool,
    /// Failure reason bubbled up from a child.
    child_reason: FailureReason,
    /// Simulated time this request entered its current node.
    enter_ms: f64,
    /// Simulated time this request ARRIVED at its current node; see
    /// `Engine::record_hop` for why this is kept separate from `enter_ms`.
    arrive_ms: f64,
    /// Hops recorded so far, for the ONE request being traced, else `None`.
    trace: Option<Vec<TraceHop>>,
    /// Simulated time the root request was generated (client only).
    root_start_ms: f64,
    /// Hop depth from the client root.
    hop: u32,
    /// Which retry attempt this call represents at the parent.
    attempt: u32,
    /// True once the parent stopped waiting (timeout). Work continues,
    /// result discarded.
    abandoned: bool,
    /// Set while the request occupies a server slot, so release is
    /// idempotent.
    holding_slot: bool,
    /// Edge this request traversed to reach `node_id`, for `edge_flow`
    /// accounting. (Mirrors TS's field; currently write-only here, same as
    /// the TS source -- kept for parity and for a future readout.)
    #[allow(dead_code)]
    via_edge: String,
    /// True when this request is a detached queue message being drained by
    /// a worker.
    detached: bool,
    /// Node that must be re-called on retry (actually an edge id, matching
    /// TS's own reuse of the field for both purposes -- see `on_retry`).
    retry_target: String,
    /// Attempt number for a scheduled retry.
    retry_attempt: u32,
    /// Own service time already spent at this node, part of parent latency
    /// accounting.
    own_ms: f64,
    /// Guards double resolution.
    resolved: bool,
    /// Partition/cache key this request concerns.
    key: u32,
    /// True when this request is a write.
    is_write: bool,
    /// Extra service time (ms) a behaviour asked for, consumed by
    /// `serve_within`/`serve_deferred`.
    extra_service_ms: f64,
    /// Callback a self-managing kind registers via `serve_within`/
    /// `serve_deferred`, fired when this request's service time elapses.
    on_drained: Option<Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>>,
}

impl Req {
    fn fresh() -> Self {
        Req {
            node_id: String::new(),
            parent: None,
            pending: 0,
            max_child_ms: 0.0,
            child_failed: false,
            child_reason: FailureReason::Error,
            enter_ms: 0.0,
            arrive_ms: 0.0,
            trace: None,
            root_start_ms: 0.0,
            hop: 0,
            attempt: 0,
            abandoned: false,
            holding_slot: false,
            via_edge: String::new(),
            detached: false,
            retry_target: String::new(),
            retry_attempt: 0,
            own_ms: 0.0,
            resolved: false,
            key: 0,
            is_write: false,
            extra_service_ms: 0.0,
            on_drained: None,
        }
    }
}

impl ReqLike for Req {
    fn node_id(&self) -> &str {
        &self.node_id
    }
    fn hop(&self) -> u32 {
        self.hop
    }
    fn attempt(&self) -> u32 {
        self.attempt
    }
    fn own_ms(&self) -> f64 {
        self.own_ms
    }
    fn detached(&self) -> bool {
        self.detached
    }
    fn key(&self) -> u32 {
        self.key
    }
    fn is_write(&self) -> bool {
        self.is_write
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/* ------------------------------------------------------------------ *
 * Request pool (slab), addressed by ReqHandle
 * ------------------------------------------------------------------ */

enum Slot {
    Empty { generation: u32 },
    Occupied { generation: u32, req: Req },
}

/// Owns every live `Req`. See the module doc comment for why this replaces
/// TS's `acquireReq`/`recycle` free-list-through-the-object mechanism.
struct RequestPool {
    slots: Vec<Slot>,
    free: Vec<u32>,
    /// Mirrors TS's `this.liveRequests`.
    live: u32,
}

impl RequestPool {
    fn new() -> Self {
        RequestPool { slots: Vec::new(), free: Vec::new(), live: 0 }
    }

    /// The Rust equivalent of `acquireReq()`: take a free slot (bumping
    /// nothing -- the generation only changes on recycle) or grow the slab.
    fn acquire(&mut self, req: Req) -> ReqHandle {
        self.live += 1;
        if let Some(index) = self.free.pop() {
            let generation = match &self.slots[index as usize] {
                Slot::Empty { generation } => *generation,
                Slot::Occupied { .. } => unreachable!("free list pointed at an occupied slot"),
            };
            self.slots[index as usize] = Slot::Occupied { generation, req };
            return ReqHandle { index, generation };
        }
        let index = self.slots.len() as u32;
        let generation = 0;
        self.slots.push(Slot::Occupied { generation, req });
        ReqHandle { index, generation }
    }

    fn get(&self, h: ReqHandle) -> Option<&Req> {
        match self.slots.get(h.index as usize) {
            Some(Slot::Occupied { generation, req }) if *generation == h.generation => Some(req),
            _ => None,
        }
    }

    fn get_mut(&mut self, h: ReqHandle) -> Option<&mut Req> {
        match self.slots.get_mut(h.index as usize) {
            Some(Slot::Occupied { generation, req }) if *generation == h.generation => Some(req),
            _ => None,
        }
    }

    /// Mirrors `recycle()`'s token bump + free-list push: the generation is
    /// advanced so any handle or event still referencing this incarnation
    /// becomes stale (its `get`/`get_mut` return `None` from then on), and
    /// the slot is returned to the free list for reuse.
    fn recycle(&mut self, h: ReqHandle) {
        let matches = matches!(
            self.slots.get(h.index as usize),
            Some(Slot::Occupied { generation, .. }) if *generation == h.generation
        );
        if !matches {
            return;
        }
        self.live = self.live.saturating_sub(1);
        self.slots[h.index as usize] = Slot::Empty { generation: h.generation.wrapping_add(1) };
        self.free.push(h.index);
    }

    /// Full reset for `Engine::reset()`: every slot, free or occupied, is
    /// dropped and the pool starts over exactly as a freshly constructed
    /// one would (TS's `reset()` zeroes `freeReq`/`liveRequests`; it has no
    /// separate slab to clear, since a recycled `Req` is simply abandoned
    /// to the free list rather than dropped -- here the slab itself is
    /// cleared, which is the equivalent "nothing survives a reset" outcome).
    fn reset(&mut self) {
        self.slots.clear();
        self.free.clear();
        self.live = 0;
    }
}

/* ------------------------------------------------------------------ *
 * Rate counter: bucketed trailing window
 * ------------------------------------------------------------------ */

struct RateCounter {
    buckets: [f64; RATE_BUCKETS],
    stamps: [f64; RATE_BUCKETS],
    bucket_ms: f64,
}

impl RateCounter {
    fn new() -> Self {
        RateCounter {
            buckets: [0.0; RATE_BUCKETS],
            stamps: [-1.0; RATE_BUCKETS],
            bucket_ms: RATE_WINDOW_MS / RATE_BUCKETS as f64,
        }
    }

    fn add(&mut self, now: f64, n: f64) {
        let stamp = (now / self.bucket_ms).floor();
        let idx = (((stamp as i64) % RATE_BUCKETS as i64 + RATE_BUCKETS as i64) % RATE_BUCKETS as i64) as usize;
        if self.stamps[idx] != stamp {
            self.stamps[idx] = stamp;
            self.buckets[idx] = 0.0;
        }
        self.buckets[idx] += n;
    }

    /// Events per second over the trailing window. See `engine.ts`'s own
    /// doc comment on this method (carried over in spirit): every counter
    /// divides by the SAME fixed span and excludes the in-progress bucket,
    /// so rates from different counters stay comparable.
    fn rate(&self, now: f64) -> f64 {
        let current = (now / self.bucket_ms).floor();
        let mut total = 0.0;
        for i in 0..RATE_BUCKETS {
            let age = current - self.stamps[i];
            if age > 0.0 && age < RATE_BUCKETS as f64 {
                total += self.buckets[i];
            }
        }
        if total == 0.0 {
            return 0.0;
        }
        let elapsed_buckets = (current as i64).clamp(1, RATE_BUCKETS as i64 - 1) as f64;
        (total * 1000.0) / (elapsed_buckets * self.bucket_ms)
    }

    fn reset(&mut self) {
        self.buckets = [0.0; RATE_BUCKETS];
        self.stamps = [-1.0; RATE_BUCKETS];
    }
}

/* ------------------------------------------------------------------ *
 * Latency reservoir: ring buffer with timestamps, exact percentiles
 * ------------------------------------------------------------------ */

struct LatencyRing {
    vals: Vec<f64>,
    times: Vec<f64>,
    head: usize,
    count: usize,
    scratch: Vec<f64>,
    /// Monotonic count of samples ever added; part of the memo key.
    added: u64,
    cache_time: f64,
    cache_added: i64,
    cache0: f64,
    cache1: f64,
    cache2: f64,
}

impl LatencyRing {
    fn new() -> Self {
        LatencyRing {
            vals: vec![0.0; LATENCY_RING],
            times: vec![0.0; LATENCY_RING],
            head: 0,
            count: 0,
            scratch: vec![0.0; LATENCY_RING],
            added: 0,
            cache_time: -1.0,
            cache_added: -1,
            cache0: 0.0,
            cache1: 0.0,
            cache2: 0.0,
        }
    }

    fn add(&mut self, now: f64, v: f64) {
        self.vals[self.head] = v;
        self.times[self.head] = now;
        self.head = (self.head + 1) % LATENCY_RING;
        if self.count < LATENCY_RING {
            self.count += 1;
        }
        self.added += 1;
    }

    /// Fills `out[0..2]` with p50/p95/p99 over the trailing window.
    /// Memoized per (time, sample count) -- see `engine.ts`'s doc comment
    /// on the same method for why: `snapshot()` polls at 10Hz but the
    /// underlying samples only change when a request completes.
    fn percentiles(&mut self, now: f64, out: &mut [f64; 3]) {
        if now == self.cache_time && self.added as i64 == self.cache_added {
            out[0] = self.cache0;
            out[1] = self.cache1;
            out[2] = self.cache2;
            return;
        }

        let cutoff = now - LATENCY_WINDOW_MS;
        let limit = if self.count < PERCENTILE_SAMPLE_CAP { self.count } else { PERCENTILE_SAMPLE_CAP };
        let stride = if self.count > PERCENTILE_SAMPLE_CAP { self.count / PERCENTILE_SAMPLE_CAP } else { 1 };
        let mut n = 0usize;
        let mut taken = 0usize;
        let mut i = 0usize;
        while taken < limit && i < self.count {
            let idx = (self.head + 2 * LATENCY_RING - 1 - i) % LATENCY_RING;
            if self.times[idx] < cutoff {
                break;
            }
            self.scratch[n] = self.vals[idx];
            n += 1;
            taken += 1;
            i += stride;
        }

        if n == 0 {
            out[0] = 0.0;
            out[1] = 0.0;
            out[2] = 0.0;
        } else {
            let view = &mut self.scratch[0..n];
            view.sort_by(|a, b| a.partial_cmp(b).unwrap());
            out[0] = quantile(view, n, 0.5);
            out[1] = quantile(view, n, 0.95);
            out[2] = quantile(view, n, 0.99);
        }

        self.cache_time = now;
        self.cache_added = self.added as i64;
        self.cache0 = out[0];
        self.cache1 = out[1];
        self.cache2 = out[2];
    }

    fn reset(&mut self) {
        self.head = 0;
        self.count = 0;
        self.added = 0;
        self.cache_time = -1.0;
        self.cache_added = -1;
    }
}

fn quantile(sorted: &[f64], n: usize, q: f64) -> f64 {
    if n == 1 {
        return sorted[0];
    }
    let pos = q * (n - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    if lo == hi {
        return sorted[lo];
    }
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

/* ------------------------------------------------------------------ *
 * Injected failures
 * ------------------------------------------------------------------ */

struct Fault {
    kind: FailureKind,
    /// Simulated time the fault was injected.
    since_ms: f64,
    /// 'slow': service-time multiplier, >= 1.
    factor: f64,
    /// 'errors': forced failure fraction, 0..1.
    rate: f64,
    /// 'partition': edge ids cut. Empty means "every edge leaving the node".
    edge_ids: Vec<String>,
}

/* ------------------------------------------------------------------ *
 * Per-node runtime state
 * ------------------------------------------------------------------ */

struct NodeState {
    id: String,
    kind: NodeKind,
    /// This kind's behaviour, resolved once at `build_nodes()` time.
    behaviour: &'static dyn ComponentBehaviour,
    config: NodeConfig,
    /// Requests occupying a server slot right now.
    busy: f64,
    /// FIFO of requests waiting for a slot, or buffered messages (queue).
    waiting: VecDeque<ReqHandle>,
    /// Outgoing REQUEST edges. Control edges are deliberately absent.
    out: Vec<SimEdge>,
    /// Outgoing CONTROL edges -- "this node acts on that one".
    ctrl: Vec<SimEdge>,
    /// Queue nodes that feed this worker.
    sources: Vec<String>,
    /// Integrated busy-slot-milliseconds, for utilization.
    busy_ms_accum: f64,
    /// Simulated time `busy_ms_accum` was last integrated.
    last_integrate_ms: f64,
    /// Rolling utilization sample, recomputed each stats tick.
    utilization: f64,

    arrivals: RateCounter,
    completions: RateCounter,
    errors: RateCounter,
    sheds: RateCounter,
    timeouts: RateCounter,
    hits: RateCounter,
    misses: RateCounter,

    latency: LatencyRing,

    total_completed: f64,
    total_failed: f64,

    /// Identifies the arrival stream for THIS incarnation of a load
    /// generator. Assigned fresh in `build_nodes()` only when a node is
    /// genuinely new (created, or kept-by-id but retyped) -- never when an
    /// existing generator's `NodeState` is reused across a hot swap, which
    /// is what lets a retained client's already-scheduled `Arrival` event
    /// keep ticking through the edit instead of being silently restarted.
    /// A stale event from an incarnation this id no longer has (removed, or
    /// retyped away from a load-generating kind and back) carries the OLD
    /// generation and is dropped at `dispatch()` rather than firing against
    /// the new state. engine.ts:420-423 `NodeState.arrivalGeneration`, PR
    /// #33.
    arrival_generation: u32,

    /// True once a worker poll event is scheduled, so we do not stack
    /// pollers.
    poll_scheduled: bool,
    /// Per-shard utilisation published by a self-partitioning behaviour.
    shard_util: Vec<f64>,

    /// Working per-unit utilisation, mutated in place. Length is the unit
    /// count.
    instance_units: Vec<f64>,
    /// The array published to the last snapshot, or `None` if none yet.
    instance_published: Option<Vec<f64>>,
    /// Units decided but not yet serving (autoscaler warm-up).
    instance_pending: f64,
    /// `instance_pending` as last published, so a change in it alone is
    /// noticed.
    instance_pending_published: f64,

    /// Behaviour-private scratch state, allocated once from the
    /// behaviour's `init_state()` hook.
    ext: Ext,

    /// Behaviour-defined rate counters, created on first use.
    custom: Option<HashMap<String, RateCounter>>,

    /// Occupancy reported by a kind that runs its own slot discipline:
    /// `(busy, capacity)`, or `None` for every kind the engine slots
    /// itself.
    own_occupancy: Option<(f64, f64)>,
}

impl NodeStateLike for NodeState {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> NodeKind {
        self.kind
    }
    fn config(&self) -> &NodeConfig {
        &self.config
    }
    fn busy(&self) -> f64 {
        self.busy
    }
    fn out(&self) -> &[SimEdge] {
        &self.out
    }
    fn sources(&self) -> &[String] {
        &self.sources
    }
}

/// A cheap, owned snapshot of the handful of `NodeStateLike` fields a
/// behaviour hook reads. See the module doc comment ("Borrow-checker
/// shape", device 2) for why this exists instead of borrowing
/// `&NodeState` out of `self.nodes` for the duration of a hook call.
struct NodeView {
    id: String,
    kind: NodeKind,
    config: NodeConfig,
    busy: f64,
    out: Vec<SimEdge>,
    sources: Vec<String>,
}

impl NodeView {
    fn of(state: &NodeState) -> Self {
        NodeView {
            id: state.id.clone(),
            kind: state.kind,
            config: state.config.clone(),
            busy: state.busy,
            out: state.out.clone(),
            sources: state.sources.clone(),
        }
    }
}

impl NodeStateLike for NodeView {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> NodeKind {
        self.kind
    }
    fn config(&self) -> &NodeConfig {
        &self.config
    }
    fn busy(&self) -> f64 {
        self.busy
    }
    fn out(&self) -> &[SimEdge] {
        &self.out
    }
    fn sources(&self) -> &[String] {
        &self.sources
    }
}

/// Same idea as `NodeView`, for `ReqLike`. Carries its originating
/// `ReqHandle` (not part of the `ReqLike` trait surface -- a behaviour
/// never reads it) so that `Engine`'s `BehaviourCtx` methods that take a
/// `req: &dyn ReqLike` parameter can recover it via `ReqLike::as_any` and
/// look up the real pooled `Req`. See `ReqLike::as_any`'s doc comment in
/// `engine_types.rs` for why this is necessary, not merely convenient.
struct ReqView {
    handle: ReqHandle,
    node_id: String,
    hop: u32,
    attempt: u32,
    own_ms: f64,
    detached: bool,
    key: u32,
    is_write: bool,
}

impl ReqView {
    fn of(handle: ReqHandle, req: &Req) -> Self {
        ReqView {
            handle,
            node_id: req.node_id.clone(),
            hop: req.hop,
            attempt: req.attempt,
            own_ms: req.own_ms,
            detached: req.detached,
            key: req.key,
            is_write: req.is_write,
        }
    }
}

impl ReqLike for ReqView {
    fn node_id(&self) -> &str {
        &self.node_id
    }
    fn hop(&self) -> u32 {
        self.hop
    }
    fn attempt(&self) -> u32 {
        self.attempt
    }
    fn own_ms(&self) -> f64 {
        self.own_ms
    }
    fn detached(&self) -> bool {
        self.detached
    }
    fn key(&self) -> u32 {
        self.key
    }
    fn is_write(&self) -> bool {
        self.is_write
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Recover the `ReqHandle` behind a `req: &dyn ReqLike` parameter handed
/// back to a `BehaviourCtx` method. Every such parameter in this codebase
/// is, by construction, a `ReqView` `Engine` itself built (no behaviour
/// file implements `ReqLike`), so this downcast is infallible in practice;
/// it returns `None` rather than panicking for a value that somehow is
/// not one, since a wrong answer here must fail the specific operation
/// that needed it, not the whole simulation.
fn req_handle_of(req: &dyn ReqLike) -> Option<ReqHandle> {
    req.as_any().downcast_ref::<ReqView>().map(|v| v.handle)
}

/* ------------------------------------------------------------------ *
 * Engine
 * ------------------------------------------------------------------ */

pub struct Engine {
    topology: Topology,
    seed: u32,
    rng: Rng,

    /// Current simulated time in ms.
    now: f64,
    seq: u64,
    heap: MinHeap<Ev>,

    nodes: HashMap<String, NodeState>,
    client_ids: Vec<String>,
    /// Source of the next `NodeState::arrival_generation` handed out in
    /// `build_nodes()`. Monotonic for the life of the engine; rewound to 1
    /// only by `reset()`, mirroring `nextArrivalGeneration` in engine.ts
    /// (PR #33).
    next_arrival_generation: u32,
    edge_flow: HashMap<String, RateCounter>,

    pool: RequestPool,

    /// Root-level (end-to-end) measurements.
    sys_latency: LatencyRing,
    sys_offered: RateCounter,
    sys_good: RateCounter,
    sys_failed: RateCounter,
    total_requests: f64,
    total_failed: f64,
    failures: FailuresByReason,

    /// Injected failures, keyed by node id. At most one fault per node.
    faults: HashMap<String, Fault>,
    /// Every edge id cut by an active partition, flattened out of `faults`
    /// so the send path tests a partition with one `HashSet` lookup.
    cut_edges: HashSet<String>,

    history: Vec<HistoryPoint>,
    last_history_ms: f64,

    /// Capacity each node was authored with, by node id. See `engine.ts`'s
    /// doc comment on the same field for why `reset()` needs this rather
    /// than rebuilding from the (possibly controller-mutated) topology.
    authored_capacity: HashMap<String, f64>,
    /// Same, for the per-shard knob a sharded store is scaled through.
    authored_shard_capacity: HashMap<String, f64>,
    /// Same, for the instance count a plain service/lb/cache/worker is
    /// scaled through. `instances` is optional in `NodeConfig`, so an entry
    /// can be `None`: that records "the preset left it unset", and
    /// `reset()` has to put the field back to unset rather than skip it,
    /// because a scale-up will have written a number there. engine.ts
    /// `authoredInstances`, PR #48.
    authored_instances: HashMap<String, Option<f64>>,

    /// The request currently being traced, or `None` between samples.
    tracing: Option<ReqHandle>,
    /// The last completed trace, handed to every snapshot until replaced.
    last_trace: Option<RequestTrace>,

    /// Node ids with an active `with_node_ext` frame on the call stack
    /// right now. A behaviour hook can re-enter the engine for the SAME
    /// node synchronously -- e.g. `streambroker`'s `on_downstream_result`
    /// pumps the next queued delivery via `emit_detached`, which can
    /// resolve immediately against a zero-queue downstream capacity limit,
    /// and that resolution reports back to the very broker node whose
    /// `ext` is still checked out by the outer frame. `behaviour-
    /// messaging.ts`'s own doc comment calls this out as an open question
    /// "once sim::engine exists": in JS, re-entering just mutates the same
    /// object; in Rust, `ext.take()` would hand the inner frame a `None`
    /// it was never meant to see, which panicked at the downcast.
    ///
    /// This set turns that inner call into a no-op instead: a reentrant
    /// hook invocation is skipped entirely rather than run against a
    /// missing `ext`. That is a deliberate, narrow trade-off, not a full
    /// fix -- a behaviour whose reentrant call would have flipped its own
    /// bookkeeping (e.g. clearing an in-flight flag for the partition that
    /// just resolved) does not get to do so, so that bookkeeping can be
    /// left stuck until the next unrelated event nudges it. Preferred over
    /// the alternative (deferring the reentrant call as a same-tick queued
    /// event) for now because that would need threading the child's
    /// result data through a new event variant rather than the call stack
    /// it currently rides on; revisit if a kind other than streambroker's
    /// own self-pumping is ever found relying on this path resolving
    /// synchronously.
    ext_in_progress: HashSet<String>,
}

impl Engine {
    pub fn new(topology: Topology, seed: u32) -> Engine {
        let mut engine = Engine {
            topology,
            seed,
            rng: Rng::new(seed),
            now: 0.0,
            seq: 0,
            heap: MinHeap::new(),
            nodes: HashMap::new(),
            client_ids: Vec::new(),
            next_arrival_generation: 1,
            edge_flow: HashMap::new(),
            pool: RequestPool::new(),
            sys_latency: LatencyRing::new(),
            sys_offered: RateCounter::new(),
            sys_good: RateCounter::new(),
            sys_failed: RateCounter::new(),
            total_requests: 0.0,
            total_failed: 0.0,
            failures: empty_failures_by_reason(),
            faults: HashMap::new(),
            cut_edges: HashSet::new(),
            history: Vec::new(),
            last_history_ms: 0.0,
            authored_capacity: HashMap::new(),
            authored_shard_capacity: HashMap::new(),
            authored_instances: HashMap::new(),
            tracing: None,
            last_trace: None,
            ext_in_progress: HashSet::new(),
        };
        engine.record_authored_capacity();
        engine.build_nodes();
        engine
    }

    /// Snapshot the authored scale of every node, for `reset()` to restore.
    /// All three scalable knobs are recorded, because a controller may
    /// write any one of them depending on the target kind's scale field --
    /// restoring only `capacity` would let a controller-scaled sharded
    /// store carry its grown `shard_capacity` across a reset, or an
    /// autoscaled service its grown `instances`, so the same seed would
    /// not replay. engine.ts `recordAuthoredCapacity`, PR #48.
    fn record_authored_capacity(&mut self) {
        self.authored_capacity.clear();
        self.authored_shard_capacity.clear();
        self.authored_instances.clear();
        for n in &self.topology.nodes {
            self.authored_capacity.insert(n.id.clone(), n.config.capacity);
            self.authored_shard_capacity.insert(n.id.clone(), n.config.shard_capacity);
            self.authored_instances.insert(n.id.clone(), n.config.instances);
        }
    }

    /* ---------------- public API ---------------- */

    pub fn set_topology(&mut self, t: Topology) {
        self.topology = t;
        self.record_authored_capacity();
        self.build_nodes();
        // Faults survive a topology edit, but a whole-node partition names
        // its edges implicitly, so the flattened set has to be recomputed
        // against the new wiring. A fault on a node that no longer exists
        // is dropped.
        let stale: Vec<String> = self
            .faults
            .keys()
            .filter(|id| !self.nodes.contains_key(id.as_str()))
            .cloned()
            .collect();
        for id in stale {
            self.faults.remove(&id);
        }
        self.rebuild_cut_edges();
    }

    /// Merge a partial JSON patch onto a node's config. The closest
    /// faithful Rust equivalent of TS's `Object.assign(node.config,
    /// patch)` with a `Partial<NodeConfig>`: the frontend may send only the
    /// fields it changed, so a merge-by-key over `serde_json::Value`
    /// (rather than requiring a full, always-complete `NodeConfig`) is what
    /// keeps that contract. Errors if the merged result no longer
    /// deserializes as a `NodeConfig` (a malformed patch), which TS's
    /// `Object.assign` has no equivalent failure mode for, but which Rust's
    /// strongly-typed `NodeConfig` makes a real possibility worth reporting
    /// rather than silently corrupting the node's config.
    pub fn update_node_config(&mut self, id: &str, patch: serde_json::Value) -> crate::error::AppResult<()> {
        let patch_obj = match patch.as_object() {
            Some(o) => o.clone(),
            None => return Ok(()), // not an object: nothing to merge, same as TS ignoring a non-patch
        };

        if let Some(node) = self.topology.nodes.iter_mut().find(|n| n.id == id) {
            let mut merged = serde_json::to_value(&node.config)?;
            if let Some(map) = merged.as_object_mut() {
                for (k, v) in patch_obj.iter() {
                    map.insert(k.clone(), v.clone());
                }
            }
            node.config = serde_json::from_value(merged)?;

            // A capacity the student typed is a new authored value, so
            // reset() should return to it rather than to the one the
            // preset shipped with. A capacity written by a controller goes
            // through set_scale() and deliberately does NOT land here.
            if let Some(cap) = patch_obj.get("capacity").and_then(|v| v.as_f64()) {
                self.authored_capacity.insert(id.to_string(), cap.floor().max(1.0));
            }
            if let Some(cap) = patch_obj.get("shardCapacity").and_then(|v| v.as_f64()) {
                self.authored_shard_capacity.insert(id.to_string(), cap.floor().max(1.0));
            }
            // Same for instances: a value the student typed is authored, so
            // a later hot swap (build_nodes) must treat it as authored, not
            // mistake it for a controller write. A value written by a
            // controller goes through set_scale() and deliberately does NOT
            // land here. PR #48.
            if let Some(inst) = patch_obj.get("instances").and_then(|v| v.as_f64()) {
                self.authored_instances.insert(id.to_string(), Some(inst.floor().max(1.0)));
            }
        }

        let has_state = self.nodes.contains_key(id);
        if !has_state {
            return Ok(());
        }
        if let Some(state) = self.nodes.get_mut(id) {
            let mut merged = serde_json::to_value(&state.config)?;
            if let Some(map) = merged.as_object_mut() {
                for (k, v) in patch_obj.iter() {
                    map.insert(k.clone(), v.clone());
                }
            }
            state.config = serde_json::from_value(merged)?;
        }
        // A capacity increase may free up slots for waiting work immediately.
        self.pump_queue(id);
        Ok(())
    }

    /* ---------------- failure injection ---------------- *
     *
     * Chaos is a first-class engine operation rather than a component, so
     * any node in any topology can be faulted without rewiring anything.
     * Each fault intercepts at exactly one point in the request path:
     *
     *   crash      admit() refuses immediately, and in-flight work at the
     *              node is failed at injection time.
     *   slow       service_time_for() multiplies the drawn service time.
     *   errors     on_service_complete() rolls it after the node's own
     *              error_rate.
     *   partition  send_child() refuses to cross a cut edge, so the CALLER
     *              sees the failure.
     * ------------------------------------------------------ */

    /// Attach a fault to a node. One fault per node: injecting again
    /// replaces whatever was there.
    pub fn inject_failure(&mut self, node_id: &str, kind: FailureKind, opts: FailureOpts) {
        if !self.nodes.contains_key(node_id) {
            return;
        }
        let fault = Fault {
            kind,
            since_ms: self.now,
            // A fault may not make a node faster, so a factor below 1 is
            // clamped.
            factor: match opts.factor {
                Some(f) if f > 1.0 => f,
                _ => 1.0,
            },
            rate: match opts.rate {
                Some(r) => clamp01(r),
                None => 1.0,
            },
            edge_ids: opts.edge_ids.unwrap_or_default(),
        };
        self.faults.insert(node_id.to_string(), fault);
        self.rebuild_cut_edges();

        // A crash is retroactive: work already inside the node dies with it.
        if kind == FailureKind::Crash {
            self.kill_in_flight(node_id);
        }
    }

    /// Heal a node. Work that already failed stays failed; new work
    /// succeeds.
    pub fn clear_failure(&mut self, node_id: &str) {
        if self.faults.remove(node_id).is_none() {
            return;
        }
        self.rebuild_cut_edges();
        // A healed node may have callers' work waiting on it right now.
        self.pump_queue(node_id);
    }

    /// Every fault in force, for the UI.
    pub fn active_failures(&self) -> Vec<ActiveFailure> {
        self.faults.iter().map(|(id, f)| describe_fault(id, f)).collect()
    }

    /// Fail everything a crashed node is holding: requests queued behind
    /// it. Collected into an owned `Vec` first because `resolve()` can
    /// re-enter the engine through the parent's retry path, which must
    /// not see a half-drained `state.waiting` still borrowed.
    fn kill_in_flight(&mut self, node_id: &str) {
        let waiting: Vec<ReqHandle> = match self.nodes.get_mut(node_id) {
            Some(state) => state.waiting.drain(..).collect(),
            None => return,
        };

        for handle in waiting {
            let already_resolved = self.pool.get(handle).map(|r| r.resolved).unwrap_or(true);
            if already_resolved {
                continue;
            }
            // A buffered queue message has no caller to inform; it is
            // simply lost, which is what an unreplicated broker losing its
            // disk looks like.
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.total_failed += 1.0;
            }
            self.resolve(handle, false, FailureReason::Crashed, 0.0);
        }

        // Requests already in service are deliberately NOT unwound here.
        // Each one still holds a real slot, and its SERVICE_DONE event will
        // release that slot and then find the node crashed in
        // on_service_complete, failing it as 'crashed'. Zeroing state.busy
        // here instead would double-release those slots.
    }

    fn rebuild_cut_edges(&mut self) {
        self.cut_edges.clear();
        let partitions: Vec<(String, Vec<String>)> = self
            .faults
            .iter()
            .filter(|(_, f)| f.kind == FailureKind::Partition)
            .map(|(id, f)| (id.clone(), f.edge_ids.clone()))
            .collect();
        for (node_id, edge_ids) in partitions {
            if !edge_ids.is_empty() {
                for id in edge_ids {
                    self.cut_edges.insert(id);
                }
                continue;
            }
            // No edge ids given: cut everything leaving the node. Control
            // edges included -- "unplug this box" severs the controller's
            // reach as much as its traffic.
            if let Some(state) = self.nodes.get(&node_id) {
                for edge in &state.out {
                    self.cut_edges.insert(edge.id.clone());
                }
                for edge in &state.ctrl {
                    self.cut_edges.insert(edge.id.clone());
                }
            }
        }
    }

    /* ---------------- behaviour-hook call helper ---------------- */

    /// Run `f` with this node's behaviour, a fresh read-only view of its
    /// state, and its `ext` taken out for the duration of the call (put
    /// back afterward). See the module doc comment, device 3, for why.
    fn with_node_ext<R>(
        &mut self,
        node_id: &str,
        f: impl FnOnce(&mut Engine, &'static dyn ComponentBehaviour, &NodeView, &mut Ext) -> R,
    ) -> Option<R> {
        // See `ext_in_progress`'s doc comment: a synchronous re-entry for
        // this exact node has nothing safe to do here (its state is
        // already being mutated by the outer frame), so it is a no-op
        // rather than a double-take on `state.ext`.
        if self.ext_in_progress.contains(node_id) {
            return None;
        }
        self.ext_in_progress.insert(node_id.to_string());
        let (behaviour, view, mut ext) = {
            let state = match self.nodes.get_mut(node_id) {
                Some(s) => s,
                None => {
                    self.ext_in_progress.remove(node_id);
                    return None;
                }
            };
            (state.behaviour, NodeView::of(state), state.ext.take())
        };
        let result = f(self, behaviour, &view, &mut ext);
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.ext = ext;
        }
        self.ext_in_progress.remove(node_id);
        Some(result)
    }

    /* ---------------- small by-id wrappers over the BehaviourCtx surface ---------------- *
     * `BehaviourCtx`'s methods take `&dyn NodeStateLike`; most call sites in
     * this file only have a `node_id`. These build the borrow, call the
     * trait method, and drop it -- never held across another `&mut self`
     * call.
     * ------------------------------------------------------ */

    fn effective_capacity_by_id(&self, node_id: &str) -> f64 {
        match self.nodes.get(node_id) {
            Some(s) => self.effective_capacity(s),
            None => 1.0,
        }
    }

    fn queue_depth_by_id(&self, node_id: &str) -> f64 {
        match self.nodes.get(node_id) {
            Some(s) => self.queue_depth(s),
            None => 0.0,
        }
    }

    fn effective_queue_limit_by_id(&self, node_id: &str) -> f64 {
        match self.nodes.get(node_id) {
            Some(s) => self.effective_queue_limit(s),
            None => 0.0,
        }
    }

    /// The rate this client is offering RIGHT NOW, after its traffic
    /// pattern. Not part of `BehaviourCtx` (no behaviour ever calls it;
    /// only the engine's own client-arrival scheduling does), so this is a
    /// plain private method, not a trait impl.
    ///
    /// A pure function of simulation time and config: nothing is carried
    /// between ticks and the RNG is untouched, so the same seed and
    /// topology still replay byte-identically.
    fn effective_rps(&self, state: &dyn NodeStateLike) -> f64 {
        let base = state.config().rps;
        let pattern = state.config().traffic;
        if pattern.is_none() || pattern == Some(TrafficPattern::Steady) || !(base > 0.0) {
            return base;
        }
        let period_s = state.config().traffic_period_s.unwrap_or(DEFAULT_TRAFFIC_PERIOD_S);
        if !(period_s > 0.0) {
            return base;
        }
        let t = (self.now / 1000.0) % period_s;
        let phase = t / period_s;
        match pattern.unwrap() {
            TrafficPattern::Ramp => {
                // Climbs over the first cycle and holds.
                if self.now / 1000.0 >= period_s {
                    base
                } else {
                    base * phase
                }
            }
            TrafficPattern::Spike => {
                // Quiet, then a burst in the last tenth of the cycle.
                if phase >= SPIKE_QUIET_FRACTION {
                    base * SPIKE_PEAK
                } else {
                    base * SPIKE_TROUGH
                }
            }
            TrafficPattern::Diurnal => {
                // A day as a cosine between a quarter of the baseline and
                // twice it.
                let swing = (1.0 - (2.0 * std::f64::consts::PI * phase).cos()) / 2.0;
                base * (DIURNAL_TROUGH + (DIURNAL_PEAK - DIURNAL_TROUGH) * swing)
            }
            TrafficPattern::Steady => base,
        }
    }

    fn effective_rps_by_id(&self, node_id: &str) -> f64 {
        match self.nodes.get(node_id) {
            Some(state) => self.effective_rps(state),
            None => 0.0,
        }
    }

    /* ---------------- event dispatch ---------------- */

    /// Schedule an event at simulated time `at`. The insertion sequence
    /// number breaks ties, so two events landing on the same millisecond
    /// pop in the order they were pushed -- which is what keeps replay
    /// deterministic. Mirrors TS `Engine.push()` minus its `freeEv`
    /// recycling pool, which only existed to dodge JS garbage collection.
    fn push(&mut self, at: f64, kind: EvKind, node_id: &str, req: Option<ReqHandle>) {
        let seq = self.seq;
        self.seq = seq + 1;
        self.heap.push(Ev {
            time: at,
            seq,
            kind,
            node_id: node_id.to_string(),
            req,
            arrival_generation: None,
        });
    }

    /// Schedule an `Arrival` event carrying `generation` (the generating
    /// node's `arrival_generation` at schedule time), so a stale event from
    /// an incarnation of this id that no longer exists is recognized and
    /// dropped at `dispatch()` rather than firing against whatever now
    /// holds the id. See `Ev::arrival_generation`'s doc comment; PR #33.
    fn push_arrival(&mut self, at: f64, node_id: &str, generation: u32) {
        let seq = self.seq;
        self.seq = seq + 1;
        self.heap.push(Ev {
            time: at,
            seq,
            kind: EvKind::Arrival,
            node_id: node_id.to_string(),
            req: None,
            arrival_generation: Some(generation),
        });
    }

    fn dispatch(&mut self, ev: Ev) {
        if !self.nodes.contains_key(&ev.node_id) {
            return;
        }
        match ev.kind {
            EvKind::Arrival => {
                // A stale arrival from a client this id no longer is (the
                // node was removed and a different kind rebuilt under the
                // same id, or retyped away from a load generator and back)
                // must not fire against the new incarnation -- that is what
                // let repeated structural edits multiply one client's
                // measured rate far past its configured rps. The node
                // existence check above is not enough: `contains_key` is
                // still true for a retyped id. PR #33.
                let fires = self
                    .nodes
                    .get(&ev.node_id)
                    .map(|s| s.behaviour.generates_load() && Some(s.arrival_generation) == ev.arrival_generation)
                    .unwrap_or(false);
                if fires {
                    self.on_client_arrival(&ev.node_id);
                }
            }
            EvKind::ServiceDone => {
                if let Some(handle) = ev.req {
                    if self.pool.get(handle).is_some() {
                        self.on_service_done(&ev.node_id, handle);
                    }
                }
            }
            EvKind::Timeout => {
                if let Some(handle) = ev.req {
                    if self.pool.get(handle).is_some() {
                        self.on_timeout(handle);
                    }
                }
            }
            EvKind::WorkerPoll => {
                if let Some(state) = self.nodes.get_mut(&ev.node_id) {
                    state.poll_scheduled = false;
                }
                self.pump_worker(&ev.node_id);
            }
            EvKind::Retry => {
                if let Some(handle) = ev.req {
                    if self.pool.get(handle).is_some() {
                        self.on_retry(&ev.node_id, handle);
                    }
                }
            }
            EvKind::LinkArrive => {
                // A caller that already gave up during the crossing leaves
                // a resolved request, which is dropped here.
                if let Some(handle) = ev.req {
                    let ok = self.pool.get(handle).map(|r| !r.resolved).unwrap_or(false);
                    if ok {
                        self.admit(&ev.node_id, handle);
                    }
                }
            }
            EvKind::BehaviourWake => {
                // `pool.get` is the same staleness guard every other
                // request-bearing event kind above uses: a request that
                // resolved and was recycled by the time this timer fires
                // (succeeded, failed some other way, or its caller gave up)
                // has a handle that no longer resolves, so the wake is
                // silently dropped here, exactly like a stale
                // ServiceDone/Timeout/Retry. A request that was instead
                // RESUMED via `resume_admission` before this timer fired is
                // still live (now in service further downstream) and DOES
                // reach `on_behaviour_wake` -- the behaviour's own `on_wake`
                // is what recognizes that case (its waiter bookkeeping no
                // longer lists the request) and no-ops, the same two-layer
                // guard `on_timeout`/child-resolution already use elsewhere
                // in this file.
                if let Some(handle) = ev.req {
                    if self.pool.get(handle).is_some() {
                        self.on_behaviour_wake(&ev.node_id, handle);
                    }
                }
            }
        }
    }

    /* ---------------- client ---------------- */

    fn schedule_arrival(&mut self, node_id: &str) {
        let (generates_load, generation) = match self.nodes.get(node_id) {
            Some(s) => (s.behaviour.generates_load(), s.arrival_generation),
            None => return,
        };
        if !generates_load {
            return;
        }
        // The pattern's rate, not the baseline: a spike's quiet phase must
        // actually schedule arrivals further apart.
        let rps = self.effective_rps_by_id(node_id);
        if !(rps > 0.0) {
            // Poll again shortly so raising the slider, or a pattern
            // coming back up off its trough, resumes traffic.
            self.push_arrival(self.now + 50.0, node_id, generation);
            return;
        }
        let gap = self.rng.exponential(1000.0 / rps);
        self.push_arrival(self.now + gap, node_id, generation);
    }

    fn on_client_arrival(&mut self, node_id: &str) {
        self.schedule_arrival(node_id);
        if self.effective_rps_by_id(node_id) <= 0.0 {
            return;
        }
        if self.pool.live >= MAX_LIVE_REQUESTS {
            // Past the live-request ceiling the request is still OFFERED,
            // and saying so is the whole point: returning silently here
            // left every rate reading at zero while the design was
            // maximally overloaded, so a client sending a million a second
            // reported "offered 0.0/s, served 0.0/s, 0.0% failed" beside a
            // service pinned at 100% busy -- reading as idle rather than
            // drowning. The ceiling is this engine protecting itself rather
            // than anything the design did, but a request the system could
            // not take IS a shed from the reader's side. engine.ts
            // `onClientArrival`, commit `7cc1fea9` (no PR).
            self.total_requests += 1.0;
            self.sys_offered.add(self.now, 1.0);
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.arrivals.add(self.now, 1.0);
                state.sheds.add(self.now, 1.0);
                state.total_failed += 1.0;
            }
            *self.failures.entry(FailureReason::Shed).or_insert(0) += 1;
            self.sys_failed.add(self.now, 1.0);
            return;
        }

        let mut root = Req::fresh();
        root.node_id = node_id.to_string();
        root.parent = None;
        root.root_start_ms = self.now;
        root.enter_ms = self.now;
        root.hop = 0;
        root.attempt = 0;
        root.pending = 0;
        root.max_child_ms = 0.0;
        root.own_ms = 0.0;
        // Draw this request's key. Every downstream call inherits it.
        root.key = (self.rng.next() * KEYSPACE as f64).floor() as u32;

        let handle = self.pool.acquire(root);

        // Trace one request at a time, and only once the previous one has
        // been published.
        if self.tracing.is_none() {
            self.tracing = Some(handle);
            if let Some(r) = self.pool.get_mut(handle) {
                r.trace = Some(Vec::new());
            }
        }

        self.total_requests += 1.0;
        self.sys_offered.add(self.now, 1.0);
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.arrivals.add(self.now, 1.0);
        }

        self.dispatch_downstream(node_id, handle);
    }

    /* ---------------- routing ---------------- */

    /// Issue this node's downstream calls. For an lb exactly one edge is
    /// chosen; for anything else every distinct downstream is called and
    /// joined.
    fn dispatch_downstream(&mut self, node_id: &str, req: ReqHandle) {
        let hop = match self.pool.get(req) {
            Some(r) => r.hop,
            None => return,
        };
        if hop >= MAX_HOP_DEPTH {
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.resolve(req, false, FailureReason::Depth, own_ms);
            return;
        }

        let out_len = self.nodes.get(node_id).map(|s| s.out.len()).unwrap_or(0);
        if out_len == 0 {
            // Terminal node: nothing downstream, this call is done.
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.resolve(req, true, FailureReason::Error, own_ms);
            return;
        }

        let mode = {
            let req_view = match self.pool.get(req) {
                Some(r) => ReqView::of(req, r),
                None => return,
            };
            self.with_node_ext(node_id, |engine, behaviour, node_view, ext| {
                behaviour.route(engine, node_view, &req_view, ext)
            })
            .unwrap_or(RouteMode::All)
        };

        if mode == RouteMode::None {
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.complete_node(node_id, req, own_ms);
            return;
        }

        if mode == RouteMode::One {
            let out = self.nodes.get(node_id).map(|s| s.out.clone()).unwrap_or_default();
            let edge = {
                let req_view = match self.pool.get(req) {
                    Some(r) => ReqView::of(req, r),
                    None => return,
                };
                self.with_node_ext(node_id, |engine, behaviour, node_view, ext| {
                    behaviour.pick_edge(engine, node_view, &req_view, &out, ext)
                })
                .flatten()
            };

            let edge = match edge {
                Some(e) => e,
                None => {
                    // A kind whose pick_edge can decline for a reason of
                    // its own names it here (a region node with no
                    // healthy region reports 'region-down', not a generic
                    // routing error).
                    let reason = self
                        .nodes
                        .get(node_id)
                        .and_then(|s| s.behaviour.no_route_reason())
                        .unwrap_or(FailureReason::NoRoute);
                    if reason != FailureReason::NoRoute {
                        // A deliberate refusal by this node, so it owns
                        // the failure.
                        if let Some(state) = self.nodes.get_mut(node_id) {
                            state.errors.add(self.now, 1.0);
                            state.total_failed += 1.0;
                        }
                    }
                    let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
                    self.resolve(req, false, reason, own_ms);
                    return;
                }
            };
            if let Some(r) = self.pool.get_mut(req) {
                r.pending = 1;
            }
            self.send_child(node_id, req, &edge, 0);
            return;
        }

        // mode == RouteMode::All
        let out = self.nodes.get(node_id).map(|s| s.out.clone()).unwrap_or_default();
        if let Some(r) = self.pool.get_mut(req) {
            r.pending = out.len() as u32;
        }
        // Snapshot the count: a child may resolve synchronously (shed) and
        // decrement `pending` mid-loop.
        for edge in &out {
            let resolved = self.pool.get(req).map(|r| r.resolved).unwrap_or(true);
            if resolved {
                return;
            }
            self.send_child(node_id, req, edge, 0);
        }
    }

    fn send_child(&mut self, parent_node_id: &str, parent: ReqHandle, edge: &SimEdge, attempt: u32) {
        if !self.nodes.contains_key(&edge.to) {
            self.child_resolved(parent, false, FailureReason::NoRoute, 0.0);
            return;
        }

        // A partitioned link fails at the CALLER: the packet never
        // arrives, so the target node sees no arrival at all. Reported
        // through child_resolved_from rather than child_resolved so the
        // caller's retry budget applies.
        if self.cut_edges.contains(&edge.id) {
            if let Some(state) = self.nodes.get_mut(parent_node_id) {
                state.errors.add(self.now, 1.0);
                state.total_failed += 1.0;
            }
            // A throwaway request built just to carry `attempt` through
            // child_resolved_from, mirroring TS's `stub` object -- never
            // admitted anywhere, recycled immediately after.
            let mut stub = Req::fresh();
            stub.attempt = attempt;
            stub.retry_target = edge.id.clone();
            stub.resolved = true;
            let stub_handle = self.pool.acquire(stub);
            self.child_resolved_from(stub_handle, parent, false, FailureReason::Partitioned, 0.0);
            self.pool.recycle(stub_handle);
            return;
        }

        let (parent_hop, parent_root_start_ms, parent_detached, parent_key, parent_is_write) = {
            let p = match self.pool.get(parent) {
                Some(p) => p,
                None => return,
            };
            (p.hop, p.root_start_ms, p.detached, p.key, p.is_write)
        };

        let mut child = Req::fresh();
        child.node_id = edge.to.clone();
        child.parent = Some(parent);
        child.root_start_ms = parent_root_start_ms;
        child.enter_ms = self.now;
        child.hop = parent_hop + 1;
        child.attempt = attempt;
        child.via_edge = edge.id.clone();
        child.retry_target = edge.id.clone();
        child.detached = parent_detached;
        child.key = parent_key;
        child.is_write = parent_is_write;
        let child_handle = self.pool.acquire(child);

        if let Some(counter) = self.edge_flow.get_mut(&edge.id) {
            counter.add(self.now, 1.0);
        }

        // The caller's timeout bounds THIS attempt, not the whole retry
        // budget: armed at send time, so link latency counts against the
        // deadline exactly as propagation delay does for a real caller.
        self.arm_timeout(parent_node_id, child_handle);

        // Link propagation delay. Absent, this is a plain synchronous
        // admission and the event stream is unchanged.
        if let Some(ms) = edge.latency_ms {
            if ms > 0.0 {
                self.push(self.now + ms, EvKind::LinkArrive, &edge.to, Some(child_handle));
                return;
            }
        }

        self.admit(&edge.to, child_handle);
    }

    /* ---------------- admission ---------------- */

    /// The request currently being traced's recorded hops. Called once per
    /// hop, at the moment the node's own work finishes.
    fn record_hop(&mut self, req: ReqHandle) {
        let traced = match self.tracing {
            Some(t) => t,
            None => return,
        };
        // Only hops belonging to the sampled journey: walk up to the root.
        let mut root = req;
        loop {
            let parent = match self.pool.get(root) {
                Some(r) => r.parent,
                None => return,
            };
            match parent {
                Some(p) => root = p,
                None => break,
            }
        }
        if root != traced {
            return;
        }
        let (node_id, hop, parent_is_none, enter_ms, arrive_ms, own_ms) = {
            let r = match self.pool.get(req) {
                Some(r) => r,
                None => return,
            };
            (r.node_id.clone(), r.hop, r.parent.is_none(), r.enter_ms, r.arrive_ms, r.own_ms)
        };
        // A client root is not admitted to a queue and holds no slot, so
        // it has no arrival stamp of its own; its elapsed time is already
        // reported as the total. Booking it as a hop would double-count
        // the whole path.
        if parent_is_none && hop == 0 {
            return;
        }
        // A node that never waited entered service the moment it arrived,
        // so this is zero without a special case.
        let queued_ms = (enter_ms - arrive_ms).max(0.0);
        if let Some(tr) = self.pool.get_mut(traced) {
            if let Some(trace) = tr.trace.as_mut() {
                trace.push(TraceHop {
                    node_id,
                    depth: hop as f64,
                    queued_ms,
                    // The node's OWN work, not its wall clock.
                    service_ms: own_ms,
                });
            }
        }
    }

    /// Offer a request to a node: shed, buffer, or start service.
    fn admit(&mut self, node_id: &str, req: ReqHandle) {
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.arrivals.add(self.now, 1.0);
        }
        // Stamped here, at the ONE point every request enters a node, so
        // the queued time the tracer reports cannot miss a path.
        if let Some(r) = self.pool.get_mut(req) {
            r.arrive_ms = self.now;
        }

        // A crashed node accepts nothing. Checked before the behaviour
        // runs, so a crash takes a node out whatever kind it is.
        let crashed = self.faults.get(node_id).map(|f| f.kind == FailureKind::Crash).unwrap_or(false);
        if crashed {
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.errors.add(self.now, 1.0);
                state.total_failed += 1.0;
            }
            self.resolve(req, false, FailureReason::Crashed, 0.0);
            return;
        }

        let action = {
            let req_view = match self.pool.get(req) {
                Some(r) => ReqView::of(req, r),
                None => return,
            };
            self.with_node_ext(node_id, |engine, behaviour, node_view, ext| {
                behaviour.on_admit(engine, node_view, &req_view, ext)
            })
            .unwrap_or(AdmitAction::Serve)
        };

        match action {
            AdmitAction::Handled => {}
            AdmitAction::Shed => self.shed(node_id, req),
            AdmitAction::Passthru => {
                // Effectively zero-capacity dispatchers: no queueing,
                // immediate hand-off.
                if let Some(r) = self.pool.get_mut(req) {
                    r.own_ms = 0.0;
                }
                self.begin_zero_service(node_id, req);
            }
            AdmitAction::Serve => {
                let capacity = self.effective_capacity_by_id(node_id);
                let busy = self.nodes.get(node_id).map(|s| s.busy).unwrap_or(0.0);
                if busy < capacity {
                    self.start_service(node_id, req);
                    return;
                }
                let depth = self.queue_depth_by_id(node_id);
                let limit = self.effective_queue_limit_by_id(node_id);
                if depth >= limit {
                    self.shed(node_id, req);
                    return;
                }
                if let Some(state) = self.nodes.get_mut(node_id) {
                    state.waiting.push_back(req);
                }
            }
        }
    }

    /// Draw one service time for a node, with any injected 'slow' fault
    /// applied. Every service-time draw in the engine goes through here,
    /// so a slow fault cannot be bypassed by whichever path a kind
    /// happens to take.
    fn service_time_for(&mut self, node_id: &str) -> f64 {
        let (service_ms, service_cv) = match self.nodes.get(node_id) {
            Some(s) => (s.config.service_ms, s.config.service_cv),
            None => return 0.0,
        };
        if !(service_ms > 0.0) {
            return 0.0;
        }
        let ms = self.rng.service_time(service_ms, service_cv);
        let fault_factor = self.faults.get(node_id).and_then(|f| {
            if f.kind == FailureKind::Slow {
                Some(f.factor)
            } else {
                None
            }
        });
        match fault_factor {
            Some(factor) => ms * factor,
            None => ms,
        }
    }

    /// Book a shed against a node and fail the call.
    fn shed(&mut self, node_id: &str, req: ReqHandle) {
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.sheds.add(self.now, 1.0);
            state.total_failed += 1.0;
        }
        self.resolve(req, false, FailureReason::Shed, 0.0);
    }

    /// A buffering node acknowledges immediately: the caller's chain
    /// resolves as a success right here, and the message is parked for
    /// the consumers.
    fn ack_and_buffer_impl(&mut self, node_id: &str, req: ReqHandle) {
        let ack_ms = self.service_time_for(node_id);

        // Detach a buffered copy for the workers, then ack the caller.
        let req_hop = self.pool.get(req).map(|r| r.hop).unwrap_or(0);
        let mut msg = Req::fresh();
        msg.node_id = node_id.to_string();
        msg.parent = None;
        msg.root_start_ms = self.now;
        msg.enter_ms = self.now;
        msg.hop = req_hop;
        msg.detached = true;
        let msg_handle = self.pool.acquire(msg);
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.waiting.push_back(msg_handle);
        }

        if let Some(state) = self.nodes.get_mut(node_id) {
            state.completions.add(self.now, 1.0);
            state.total_completed += 1.0;
            state.latency.add(self.now, ack_ms);
        }
        self.resolve(req, true, FailureReason::Error, ack_ms);

        // Wake any worker that feeds off this queue.
        self.wake_workers_for(node_id);
    }

    /// Acknowledge the caller and RELAY a detached copy through this
    /// node's own slot discipline -- the delivery-side sibling of
    /// `ack_and_buffer_impl`.
    fn ack_and_relay_impl(&mut self, node_id: &str, req: ReqHandle, extra_delivery_ms: Option<f64>) {
        let ack_ms = self.service_time_for(node_id);

        let (req_hop, req_key, req_is_write) = match self.pool.get(req) {
            Some(r) => (r.hop, r.key, r.is_write),
            None => return,
        };
        let mut msg = Req::fresh();
        msg.node_id = node_id.to_string();
        msg.parent = None;
        msg.root_start_ms = self.now;
        msg.enter_ms = self.now;
        msg.hop = req_hop;
        msg.detached = true;
        msg.key = req_key;
        msg.is_write = req_is_write;
        if let Some(extra) = extra_delivery_ms {
            if extra > 0.0 {
                msg.extra_service_ms = extra;
            }
        }
        let msg_handle = self.pool.acquire(msg);

        // Start delivering now if a slot is free, else park it in the
        // waiting list for pump_queue to drain. Started BEFORE the caller
        // is acked, in the same buffer-first order ack_and_buffer_impl
        // uses.
        let capacity = self.effective_capacity_by_id(node_id);
        let busy = self.nodes.get(node_id).map(|s| s.busy).unwrap_or(0.0);
        if busy < capacity {
            self.start_service(node_id, msg_handle);
        } else if let Some(state) = self.nodes.get_mut(node_id) {
            state.waiting.push_back(msg_handle);
        }

        if let Some(state) = self.nodes.get_mut(node_id) {
            state.completions.add(self.now, 1.0);
            state.total_completed += 1.0;
            state.latency.add(self.now, ack_ms);
        }
        self.resolve(req, true, FailureReason::Error, ack_ms);
    }

    fn wake_workers_for(&mut self, queue_id: &str) {
        let worker_ids: Vec<String> = self
            .nodes
            .iter()
            .filter(|(_, s)| s.behaviour.pulls_from_queues() && s.sources.iter().any(|src| src == queue_id))
            .map(|(id, _)| id.clone())
            .collect();
        for id in worker_ids {
            self.pump_worker(&id);
        }
    }

    /// lb / client style pass-through with a tiny (possibly zero) service
    /// time.
    fn begin_zero_service(&mut self, node_id: &str, req: ReqHandle) {
        let ms = self.service_time_for(node_id);
        if let Some(r) = self.pool.get_mut(req) {
            r.own_ms = ms;
        }
        if ms > 0.0 {
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.busy += 1.0;
            }
            if let Some(r) = self.pool.get_mut(req) {
                r.holding_slot = true;
            }
            self.push(self.now + ms, EvKind::ServiceDone, node_id, Some(req));
        } else {
            self.on_service_complete(node_id, req);
        }
    }

    /// A behaviour-owned timer (`wake_after`) elapsed for `req`. Dispatched
    /// only when the request is still live (see `dispatch`'s
    /// `EvKind::BehaviourWake` arm); the behaviour's own `on_wake` decides
    /// whether it still cares -- the bulkhead's, for instance, is a no-op
    /// when `resume_admission` already won the race. Port of `engine.ts`'s
    /// `EV_BEHAVIOUR_WAKE` dispatch (upstream `351327c4`, PR #77).
    fn on_behaviour_wake(&mut self, node_id: &str, req: ReqHandle) {
        let req_view = match self.pool.get(req) {
            Some(r) => ReqView::of(req, r),
            None => return,
        };
        self.with_node_ext(node_id, |engine, behaviour, node_view, ext| {
            behaviour.on_wake(engine, node_view, &req_view, ext);
        });
    }

    fn start_service(&mut self, node_id: &str, req: ReqHandle) {
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.busy += 1.0;
        }
        let extra = match self.pool.get_mut(req) {
            Some(r) => {
                r.holding_slot = true;
                r.enter_ms = self.now;
                // Any extra service time a behaviour attached to this
                // request (a write-behind cache's flush residence, booked
                // via ack_and_relay) is consumed here, exactly as
                // serve_within/serve_deferred consume it for the
                // self-managed kinds.
                let extra = r.extra_service_ms;
                if extra > 0.0 {
                    r.extra_service_ms = 0.0;
                }
                extra
            }
            None => return,
        };
        let ms = self.service_time_for(node_id) + extra;
        if let Some(r) = self.pool.get_mut(req) {
            r.own_ms = ms;
        }
        if ms > 0.0 {
            self.push(self.now + ms, EvKind::ServiceDone, node_id, Some(req));
        } else {
            self.on_service_done(node_id, req);
        }
    }

    fn on_service_done(&mut self, node_id: &str, req: ReqHandle) {
        // The slot is released the instant this node's own work finishes,
        // even if the caller already gave up -- abandoned work burned the
        // slot until now.
        self.release_slot(node_id, req);
        self.pump_queue(node_id);

        // A self-managing kind holds its slot bookkeeping in `ext` rather
        // than in `state.busy`, so it is told to free that slot here, at
        // exactly the moment the engine frees an ordinary one. Taken
        // (cleared) first so a recycled request can never re-fire a stale
        // callback.
        let drained = self.pool.get_mut(req).and_then(|r| r.on_drained.take());
        if let Some(cb) = drained {
            let node_view = match self.nodes.get(node_id) {
                Some(s) => NodeView::of(s),
                None => return,
            };
            let req_view = match self.pool.get(req) {
                Some(r) => ReqView::of(req, r),
                None => return,
            };
            cb(self, &node_view, &req_view);
        }
        self.on_service_complete(node_id, req);
    }

    fn release_slot(&mut self, node_id: &str, req: ReqHandle) {
        let holding = self.pool.get(req).map(|r| r.holding_slot).unwrap_or(false);
        if !holding {
            return;
        }
        if let Some(r) = self.pool.get_mut(req) {
            r.holding_slot = false;
        }
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.busy = (state.busy - 1.0).max(0.0);
        }
    }

    /// Start serving whoever is next in line, if a slot is free.
    fn pump_queue(&mut self, node_id: &str) {
        let mode = match self.nodes.get(node_id) {
            Some(s) => s.behaviour.pump(),
            None => return,
        };
        match mode {
            PumpMode::None => {}
            PumpMode::Sources => self.pump_worker(node_id),
            PumpMode::Own => {
                let capacity = self.effective_capacity_by_id(node_id);
                loop {
                    let next = match self.nodes.get_mut(node_id) {
                        Some(state) => {
                            if state.busy < capacity {
                                state.waiting.pop_front()
                            } else {
                                None
                            }
                        }
                        None => return,
                    };
                    let req = match next {
                        Some(r) => r,
                        None => break,
                    };
                    let resolved = self.pool.get(req).map(|r| r.resolved).unwrap_or(true);
                    if resolved {
                        continue;
                    }
                    self.start_service(node_id, req);
                }
            }
        }
    }

    /// Workers pull messages out of the queue nodes that feed them.
    fn pump_worker(&mut self, node_id: &str) {
        let capacity = self.effective_capacity_by_id(node_id);
        loop {
            let busy = match self.nodes.get(node_id) {
                Some(s) => s.busy,
                None => return,
            };
            if !(busy < capacity) {
                break;
            }
            let msg = match self.take_from_sources(node_id) {
                Some(m) => m,
                None => break,
            };
            if let Some(r) = self.pool.get_mut(msg) {
                r.node_id = node_id.to_string();
                r.enter_ms = self.now;
            }
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.arrivals.add(self.now, 1.0);
            }
            self.start_service(node_id, msg);
        }
    }

    /// Round-robin-free deterministic pull: oldest message across feeding
    /// queues.
    fn take_from_sources(&mut self, node_id: &str) -> Option<ReqHandle> {
        let sources = self.nodes.get(node_id)?.sources.clone();
        let mut best_source: Option<String> = None;
        let mut best_time = f64::INFINITY;
        for src in &sources {
            let head = match self.nodes.get(src) {
                Some(q) => q.waiting.front().copied(),
                None => None,
            };
            let head = match head {
                Some(h) => h,
                None => continue,
            };
            let enter_ms = self.pool.get(head).map(|r| r.enter_ms).unwrap_or(f64::INFINITY);
            if enter_ms < best_time {
                best_time = enter_ms;
                best_source = Some(src.clone());
            }
        }
        let best_source = best_source?;
        self.nodes.get_mut(&best_source)?.waiting.pop_front()
    }

    /* ---------------- completion of a node's own work ---------------- */

    /// This node finished its own service; decide whether to call
    /// downstream.
    fn on_service_complete(&mut self, node_id: &str, req: ReqHandle) {
        let resolved = self.pool.get(req).map(|r| r.resolved).unwrap_or(true);
        if resolved {
            return;
        }

        let fault_kind = self.faults.get(node_id).map(|f| f.kind);

        // The node died while this request was in service.
        if fault_kind == Some(FailureKind::Crash) {
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.errors.add(self.now, 1.0);
                state.total_failed += 1.0;
            }
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.resolve(req, false, FailureReason::Crashed, own_ms);
            return;
        }

        // Independent per-attempt failure.
        let error_rate = self.nodes.get(node_id).map(|s| s.config.error_rate).unwrap_or(0.0);
        if error_rate > 0.0 && self.rng.next() < error_rate {
            if let Some(state) = self.nodes.get_mut(node_id) {
                state.errors.add(self.now, 1.0);
                state.total_failed += 1.0;
            }
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.resolve(req, false, FailureReason::Error, own_ms);
            return;
        }

        // Injected error rate, rolled after (and independently of) the
        // node's own.
        let fault_rate = self.faults.get(node_id).and_then(|f| {
            if f.kind == FailureKind::Errors {
                Some(f.rate)
            } else {
                None
            }
        });
        if let Some(rate) = fault_rate {
            if self.rng.next() < rate {
                if let Some(state) = self.nodes.get_mut(node_id) {
                    state.errors.add(self.now, 1.0);
                    state.total_failed += 1.0;
                }
                let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
                self.resolve(req, false, FailureReason::Error, own_ms);
                return;
            }
        }

        let complete_action = {
            let req_view = match self.pool.get(req) {
                Some(r) => ReqView::of(req, r),
                None => return,
            };
            self.with_node_ext(node_id, |engine, behaviour, node_view, ext| {
                behaviour.on_service_complete(engine, node_view, &req_view, ext)
            })
        };

        if complete_action == Some(CompleteAction::Complete) {
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.complete_node(node_id, req, own_ms);
            return;
        }

        let out_len = self.nodes.get(node_id).map(|s| s.out.len()).unwrap_or(0);
        if out_len == 0 {
            let own_ms = self.pool.get(req).map(|r| r.own_ms).unwrap_or(0.0);
            self.complete_node(node_id, req, own_ms);
            return;
        }

        self.dispatch_downstream(node_id, req);
    }

    /// Record this node's own latency and resolve the call upward.
    fn complete_node(&mut self, node_id: &str, req: ReqHandle, latency_ms: f64) {
        if let Some(state) = self.nodes.get_mut(node_id) {
            state.completions.add(self.now, 1.0);
            state.total_completed += 1.0;
            state.latency.add(self.now, latency_ms);
        }
        self.resolve(req, true, FailureReason::Error, latency_ms);
    }

    /* ---------------- timeouts ---------------- */

    /// Arm the caller's deadline on one outgoing call.
    fn arm_timeout(&mut self, caller_id: &str, call: ReqHandle) {
        let t = self.nodes.get(caller_id).map(|s| s.config.timeout_ms).unwrap_or(0.0);
        if !(t > 0.0) {
            return;
        }
        self.push(self.now + t, EvKind::Timeout, caller_id, Some(call));
    }

    /// The caller gave up on this call. The call itself is NOT cancelled:
    /// it keeps its server slot and keeps running until its service time
    /// elapses.
    fn on_timeout(&mut self, call: ReqHandle) {
        let resolved = self.pool.get(call).map(|r| r.resolved).unwrap_or(true);
        if resolved {
            return;
        }

        // Attribute the timeout to the caller that gave up. `on_timeout`
        // solely owns the `timeouts` counter -- giving up is what it means,
        // and this is the only place that knows it happened. The failure
        // itself is NOT booked here: `child_resolved`/`resolve` already book
        // this same caller's `total_failed` once its own request resolves
        // (every node has exactly one such path, and it fires for every
        // failure reason, not only this one). Booking it here too double
        // counted every timeout as two failures instead of one -- up to a
        // 26% phantom loss -- which is also why the root's `errorRate`
        // (below, in `snapshot`) has to add `timeoutRate` back in rather
        // than leave it uncounted. engine.ts:1921-1930, PR #57.
        let parent = self.pool.get(call).and_then(|r| r.parent);
        if let Some(p) = parent {
            if let Some(caller_node_id) = self.pool.get(p).map(|r| r.node_id.clone()) {
                if let Some(state) = self.nodes.get_mut(&caller_node_id) {
                    state.timeouts.add(self.now, 1.0);
                }
            }
        }

        // Detach the call from its parent so its eventual completion is
        // discarded, then report the timeout upward (which may trigger a
        // retry).
        if let Some(r) = self.pool.get_mut(call) {
            r.parent = None;
            r.abandoned = true;
        }

        if let Some(p) = parent {
            let parent_resolved = self.pool.get(p).map(|r| r.resolved).unwrap_or(true);
            if !parent_resolved {
                let enter_ms = self.pool.get(call).map(|r| r.enter_ms).unwrap_or(self.now);
                self.child_resolved_from(call, p, false, FailureReason::Timeout, self.now - enter_ms);
            }
        }
    }

    /* ---------------- resolution & join ---------------- */

    /// Resolve one request. `latency_ms` is the latency this call
    /// contributes to its parent (own service time plus the joined
    /// subtree).
    fn resolve(&mut self, req: ReqHandle, ok: bool, reason: FailureReason, latency_ms: f64) {
        let already_resolved = self.pool.get(req).map(|r| r.resolved).unwrap_or(true);
        if already_resolved {
            return;
        }
        if let Some(r) = self.pool.get_mut(req) {
            r.resolved = true;
        }

        // resolve() is the one funnel every request passes through
        // however it ends, so a hop cannot be missed by a path that
        // finishes some other way.
        self.record_hop(req);

        let parent = self.pool.get(req).and_then(|r| r.parent);

        if parent.is_none() {
            let (detached, abandoned) = self.pool.get(req).map(|r| (r.detached, r.abandoned)).unwrap_or((false, false));
            if detached || abandoned {
                // Either a queue message that finished its worker path, or
                // work the caller already timed out on. Nobody is waiting
                // for this result.
                if self.tracing == Some(req) {
                    self.tracing = None;
                }
                self.pool.recycle(req);
                return;
            }

            // Root request measured at the client.
            let (root_start_ms, node_id, trace) = match self.pool.get_mut(req) {
                Some(r) => (r.root_start_ms, r.node_id.clone(), r.trace.take()),
                None => return,
            };
            let total = self.now - root_start_ms;
            if let Some(mut hops) = trace {
                // Deepest first would be arbitrary; ordering by depth then
                // by the moment each hop finished gives the reader the
                // path in the order the request actually walked it.
                hops.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap());
                self.last_trace = Some(RequestTrace {
                    start_ms: root_start_ms,
                    total_ms: total,
                    ok,
                    reason: if ok { None } else { Some(reason) },
                    hops,
                });
                self.tracing = None;
            }
            if ok {
                self.sys_good.add(self.now, 1.0);
                self.sys_latency.add(self.now, total);
                if let Some(client) = self.nodes.get_mut(&node_id) {
                    client.completions.add(self.now, 1.0);
                    client.total_completed += 1.0;
                    client.latency.add(self.now, total);
                }
            } else {
                self.sys_failed.add(self.now, 1.0);
                self.total_failed += 1.0;
                *self.failures.entry(reason).or_insert(0) += 1;
                if let Some(client) = self.nodes.get_mut(&node_id) {
                    client.total_failed += 1.0;
                    // resolve() owns root totalFailed (just above), but NOT
                    // the timeout arm of this match: on_timeout already
                    // attributed the timeout to whichever node actually gave
                    // up (the direct caller, not necessarily this root
                    // client), so re-adding it here on every level the
                    // failure bubbles through is what made a client's
                    // timeout rate outrun the load it offered. engine.ts
                    // `resolve()`, PR #57.
                    match reason {
                        FailureReason::Shed => client.sheds.add(self.now, 1.0),
                        FailureReason::Timeout => {}
                        _ => client.errors.add(self.now, 1.0),
                    }
                }
            }
            self.pool.recycle(req);
            return;
        }

        let parent = parent.unwrap();
        self.child_resolved_from(req, parent, ok, reason, latency_ms);
        self.pool.recycle(req);
    }

    /// A backoff delay elapsed: re-issue the failed downstream call.
    fn on_retry(&mut self, node_id: &str, parent: ReqHandle) {
        let resolved = self.pool.get(parent).map(|r| r.resolved).unwrap_or(true);
        if resolved {
            return;
        }
        let retry_target = self.pool.get(parent).map(|r| r.retry_target.clone()).unwrap_or_default();
        let retry_attempt = self.pool.get(parent).map(|r| r.retry_attempt).unwrap_or(0);
        let edge = self
            .nodes
            .get(node_id)
            .and_then(|s| s.out.iter().find(|e| e.id == retry_target).cloned());
        match edge {
            Some(e) => self.send_child(node_id, parent, &e, retry_attempt),
            None => self.child_resolved(parent, false, FailureReason::NoRoute, 0.0),
        }
    }

    fn child_resolved_from(&mut self, child: ReqHandle, parent: ReqHandle, ok: bool, reason: FailureReason, latency_ms: f64) {
        let parent_resolved = self.pool.get(parent).map(|r| r.resolved).unwrap_or(true);
        if parent_resolved {
            return; // Parent already gave up: discard the result.
        }

        // Report the outcome to the caller's behaviour, for kinds that
        // watch their dependency's health. Deliberately BEFORE the retry
        // decision, so a breaker sees every individual attempt rather than
        // only the last one. Purely observational: it cannot change what
        // happens next.
        let parent_node_id = self.pool.get(parent).map(|r| r.node_id.clone());
        if let Some(ref caller_id) = parent_node_id {
            let observes = self.nodes.get(caller_id).map(|s| s.behaviour.observes_outcome()).unwrap_or(false);
            if observes {
                let child_view = match self.pool.get(child) {
                    Some(r) => ReqView::of(child, r),
                    None => return,
                };
                self.with_node_ext(caller_id, |engine, behaviour, node_view, ext| {
                    behaviour.on_downstream_result(engine, node_view, &child_view, ok, reason, ext);
                });
            }
        }

        if !ok {
            if let Some(ref parent_state_id) = parent_node_id {
                let retries = self
                    .nodes
                    .get(parent_state_id)
                    .map(|s| s.config.retries.floor().max(0.0) as u32)
                    .unwrap_or(0);
                let child_attempt = self.pool.get(child).map(|r| r.attempt).unwrap_or(0);
                if child_attempt < retries && reason != FailureReason::Depth && reason != FailureReason::NoRoute {
                    // Re-issue the same downstream call. This is real
                    // extra load.
                    let edge_id = self.pool.get(child).map(|r| r.retry_target.clone()).unwrap_or_default();
                    let edge = self
                        .nodes
                        .get(parent_state_id)
                        .and_then(|s| s.out.iter().find(|e| e.id == edge_id).cloned());
                    if let Some(edge) = edge {
                        let attempt = child_attempt + 1;
                        // Exponential backoff with jitter. Without a delay
                        // a shed (which fails in zero time) would let a
                        // request burn its whole retry budget in the same
                        // instant.
                        let backoff = RETRY_BASE_BACKOFF_MS * 2f64.powi(attempt as i32 - 1);
                        let delay = backoff * (0.5 + self.rng.next());
                        if let Some(r) = self.pool.get_mut(parent) {
                            r.retry_target = edge.id.clone();
                            r.retry_attempt = attempt;
                        }
                        self.push(self.now + delay, EvKind::Retry, parent_state_id, Some(parent));
                        return;
                    }
                }
            }
        }

        self.child_resolved(parent, ok, reason, latency_ms);
    }

    fn child_resolved(&mut self, parent: ReqHandle, ok: bool, reason: FailureReason, latency_ms: f64) {
        let parent_resolved = self.pool.get(parent).map(|r| r.resolved).unwrap_or(true);
        if parent_resolved {
            return;
        }

        if let Some(r) = self.pool.get_mut(parent) {
            if !ok && !r.child_failed {
                r.child_failed = true;
                r.child_reason = reason;
            }
            if latency_ms > r.max_child_ms {
                r.max_child_ms = latency_ms;
            }
            r.pending = r.pending.saturating_sub(1);
        }

        let pending = self.pool.get(parent).map(|r| r.pending).unwrap_or(0);
        if pending > 0 {
            return;
        }

        let (parent_node_id, own_ms, max_child_ms, child_failed, child_reason) = match self.pool.get(parent) {
            Some(r) => (r.node_id.clone(), r.own_ms, r.max_child_ms, r.child_failed, r.child_reason),
            None => return,
        };
        let total = own_ms + max_child_ms;

        if child_failed {
            // Same as the root path: resolve() books the client's failure
            // itself; this only credits the intermediate node's own
            // counter.
            let credits = self.nodes.get(&parent_node_id).map(|s| s.behaviour.credits_join_completion()).unwrap_or(false);
            if credits {
                if let Some(state) = self.nodes.get_mut(&parent_node_id) {
                    state.total_failed += 1.0;
                }
            }
            self.resolve(parent, false, child_reason, total);
            return;
        }

        // A client is credited once, in resolve(), where end-to-end
        // latency is measured. Counting it here as well would double
        // every root request.
        let credits = self.nodes.get(&parent_node_id).map(|s| s.behaviour.credits_join_completion()).unwrap_or(false);
        if credits {
            if let Some(state) = self.nodes.get_mut(&parent_node_id) {
                state.completions.add(self.now, 1.0);
                state.total_completed += 1.0;
                state.latency.add(self.now, total);
            }
        }
        self.resolve(parent, true, FailureReason::Error, total);
    }

    /* ---------------- by-id BehaviourCtx-adjacent helpers ---------------- */

    fn effective_instances_by_id(&self, node_id: &str) -> f64 {
        match self.nodes.get(node_id) {
            Some(s) => self.effective_instances(s),
            None => 1.0,
        }
    }

    /// Service a request on behalf of a behaviour that runs its own slot
    /// discipline (`serve_within`/`serve_deferred`'s shared body).
    fn serve_within_impl(
        &mut self,
        node_id: &str,
        req: ReqHandle,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    ) {
        let (service_ms, service_cv) = match self.nodes.get(node_id) {
            Some(s) => (s.config.service_ms, s.config.service_cv),
            None => return,
        };
        let extra = self.pool.get(req).map(|r| r.extra_service_ms).unwrap_or(0.0);
        let ms = self.rng.service_time(service_ms, service_cv) + extra;
        if let Some(r) = self.pool.get_mut(req) {
            r.enter_ms = self.now;
            r.on_drained = Some(on_drained);
            r.extra_service_ms = 0.0;
            r.own_ms = ms;
        }
        if ms > 0.0 {
            self.push(self.now + ms, EvKind::ServiceDone, node_id, Some(req));
        } else {
            self.on_service_done(node_id, req);
        }
    }

    /* ---------------- utilization integration ---------------- */

    fn integrate_all(&mut self) {
        let now = self.now;
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in node_ids {
            let dt = match self.nodes.get_mut(&id) {
                Some(s) => {
                    let dt = now - s.last_integrate_ms;
                    s.last_integrate_ms = now;
                    dt
                }
                None => continue,
            };
            if dt <= 0.0 {
                continue;
            }
            let (own_occupancy, serves_requests, busy_raw) = match self.nodes.get(&id) {
                Some(s) => (s.own_occupancy, s.behaviour.serves_requests(), s.busy),
                None => continue,
            };
            let capacity = match own_occupancy {
                Some((_, cap)) => cap,
                None => self.effective_capacity_by_id(&id),
            };
            let busy = match own_occupancy {
                Some((b, _)) => b,
                None => busy_raw,
            };
            // A buffer's slot count is meaningless, so it reports zero
            // utilisation.
            let instant = if serves_requests {
                (if capacity > 0.0 { busy / capacity } else { 0.0 }).min(1.0)
            } else {
                0.0
            };
            // Exponential smoothing over roughly a 1s window.
            let alpha = 1.0 - (-dt / 500.0).exp();
            if let Some(s) = self.nodes.get_mut(&id) {
                s.utilization += (instant - s.utilization) * alpha;
                s.busy_ms_accum += busy * dt;
            }
        }
    }

    /// Autonomous per-advance work for kinds that act without a request
    /// arriving. Unlike TS's `tickNodes` (a list of nodes whose behaviour
    /// DECLARES `onTick`, kept so the per-advance walk costs nothing when
    /// no such kind exists), Rust's `ComponentBehaviour::on_tick` always
    /// has a callable default (a no-op), so there is no way to ask "did
    /// this kind override it" without a trait method `behaviour/mod.rs`
    /// does not declare -- and several kinds in THIS registry (autoscaler,
    /// replica, shard, websocket, lambda, cron, the five pooled stores,
    /// coldstorage) genuinely do override it, unlike when that TS comment
    /// was written. Calling the method unconditionally for every node is
    /// behaviourally identical (a no-op default costs one vtable call) at
    /// a cost negligible for a desktop app, so that is what this does.
    fn run_ticks(&mut self, dt_ms: f64) {
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in node_ids {
            self.with_node_ext(&id, |engine, behaviour, node_view, ext| {
                behaviour.on_tick(engine, node_view, dt_ms, ext);
            });
        }
    }

    /* ---------------- history ---------------- */

    fn maybe_record_history(&mut self) {
        while self.now - self.last_history_ms >= HISTORY_INTERVAL_MS {
            self.last_history_ms += HISTORY_INTERVAL_MS;
            let t = self.last_history_ms;
            let mut pct = [0.0; 3];
            self.sys_latency.percentiles(self.now, &mut pct);
            let good = self.sys_good.rate(self.now);
            let failed = self.sys_failed.rate(self.now);
            let done = good + failed;
            self.history.push(HistoryPoint {
                t,
                p50: pct[0],
                p95: pct[1],
                p99: pct[2],
                goodput: good,
                offered: self.sys_offered.rate(self.now),
                error_rate: if done > 0.0 { failed / done } else { 0.0 },
            });
            if self.history.len() > HISTORY_MAX {
                let excess = self.history.len() - HISTORY_MAX;
                self.history.drain(0..excess);
            }
        }
    }

    /* ---------------- advance / snapshot / reset ---------------- */

    pub fn advance(&mut self, delta_ms: f64) {
        if !(delta_ms > 0.0) {
            return;
        }
        let dt = delta_ms.min(MAX_DELTA_MS);
        let target = self.now + dt;
        let mut budget = MAX_EVENTS_PER_ADVANCE;

        loop {
            let next_time = match self.heap.peek() {
                Some(ev) => ev.time(),
                None => break,
            };
            if next_time > target {
                break;
            }
            if budget == 0 {
                break;
            }
            budget -= 1;
            let ev = self.heap.pop().unwrap();
            // Integrate utilization up to this event before state changes.
            self.now = ev.time;
            self.dispatch(ev);
        }

        self.now = target;
        self.integrate_all();
        self.run_ticks(dt);
        self.maybe_record_history();
    }

    /// Fill the working instance buffer for an `InstanceModel::Slots`
    /// kind. See `engine.ts`'s doc comment on `fillSlotInstances` for the
    /// "waterline" reasoning, carried over verbatim.
    fn fill_slot_instances(&mut self, node_id: &str) {
        let instances = self.effective_instances_by_id(node_id).max(1.0) as usize;
        let util = match self.nodes.get(node_id) {
            Some(s) => {
                if s.utilization > 1.0 {
                    1.0
                } else if s.utilization > 0.0 {
                    s.utilization
                } else {
                    0.0
                }
            }
            None => return,
        };
        let mut remaining = util * instances as f64;
        let mut units = vec![0.0f64; instances];
        for slot in units.iter_mut() {
            if remaining >= 1.0 {
                *slot = 1.0;
                remaining -= 1.0;
            } else {
                *slot = remaining.max(0.0);
                remaining = 0.0;
            }
        }
        if let Some(s) = self.nodes.get_mut(node_id) {
            s.instance_units = units;
        }
    }

    /// Copy the working instance buffer into the snapshot, allocating a
    /// new array only when something actually changed. See `engine.ts`'s
    /// doc comment on `finishInstances` for why this copy-on-change
    /// discipline exists (a memoised frontend consumer keys off array
    /// identity).
    fn finish_instances(&mut self, node_id: &str, entry: &mut NodeStats) {
        let state = match self.nodes.get_mut(node_id) {
            Some(s) => s,
            None => return,
        };
        let units = &state.instance_units;
        let mut changed = match &state.instance_published {
            None => true,
            Some(p) => p.len() != units.len(),
        };
        if !changed {
            if let Some(p) = &state.instance_published {
                for i in 0..units.len() {
                    if p[i] != units[i] {
                        changed = true;
                        break;
                    }
                }
            }
        }
        if state.instance_pending != state.instance_pending_published {
            changed = true;
        }
        if changed {
            state.instance_published = Some(state.instance_units.clone());
            state.instance_pending_published = state.instance_pending;
        }
        entry.instances = Some(state.instance_units.len() as f64);
        entry.per_instance = state
            .instance_published
            .clone()
            .or_else(|| Some(state.instance_units.clone()));
        if state.instance_pending > 0.0 {
            entry.instances_pending = Some(state.instance_pending);
        }
    }

    /// Hand each autoscaler's booked-but-not-yet-live units to the node
    /// they were booked FOR. See `engine.ts`'s doc comment on
    /// `attachPendingInstances` for the full reasoning.
    fn attach_pending_instances(&self, node_out: &mut HashMap<String, NodeStats>) {
        let mut updates: Vec<(String, f64)> = Vec::new();
        for state in self.nodes.values() {
            if state.ctrl.is_empty() {
                continue;
            }
            let own = match node_out.get(&state.id) {
                Some(o) if o.scaling == Some(true) => o,
                _ => continue,
            };
            let watched = self.control_target_of(state);
            if watched.is_empty() {
                continue;
            }
            let target_instances = match node_out.get(&watched).and_then(|t| t.instances) {
                Some(v) => v,
                None => continue,
            };
            let booked = own.target_instances.unwrap_or(0.0);
            let pending = booked - target_instances;
            if pending > 0.0 {
                updates.push((watched, pending));
            }
        }
        for (watched, pending) in updates {
            if let Some(t) = node_out.get_mut(&watched) {
                t.instances_pending = Some(pending);
            }
        }
    }

    /// Classify every edge for the snapshot. Resolution order is fixed:
    /// an injected cut outranks a kind's own `edge_state_for`, which
    /// outranks the generic flow-based fallback.
    fn snapshot_edge_state(&mut self, rates: &HashMap<String, f64>) -> HashMap<String, EdgeState> {
        let mut out: HashMap<String, EdgeState> = HashMap::new();
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in &node_ids {
            let (out_edges, ctrl_edges) = match self.nodes.get(id) {
                Some(s) => (s.out.clone(), s.ctrl.clone()),
                None => continue,
            };
            for (i, edge) in out_edges.iter().enumerate() {
                if !self.cut_edges.is_empty() && self.cut_edges.contains(&edge.id) {
                    out.insert(edge.id.clone(), EdgeState::Cut);
                    continue;
                }
                let declared = self
                    .with_node_ext(id, |engine, behaviour, node_view, ext| {
                        behaviour.edge_state_for(engine, node_view, edge, i, ext)
                    })
                    .flatten();
                match declared {
                    Some(s) => {
                        out.insert(edge.id.clone(), s);
                    }
                    None => {
                        let live = rates.get(&edge.id).copied().unwrap_or(0.0) > 0.0;
                        out.insert(edge.id.clone(), if live { EdgeState::Live } else { EdgeState::Idle });
                    }
                }
            }
            // Control edges get an entry too: never 'live' (no request
            // ever crosses one), 'standby' otherwise.
            for edge in &ctrl_edges {
                let state = if !self.cut_edges.is_empty() && self.cut_edges.contains(&edge.id) {
                    EdgeState::Cut
                } else {
                    EdgeState::Standby
                };
                out.insert(edge.id.clone(), state);
            }
        }
        // An edge whose source node no longer exists still has a flow
        // counter, so give it a state rather than a hole.
        for (id, rate) in rates {
            out.entry(id.clone())
                .or_insert(if *rate > 0.0 { EdgeState::Live } else { EdgeState::Idle });
        }
        out
    }

    fn snapshot_failures(&self) -> Vec<ActiveFailure> {
        self.faults.iter().map(|(id, f)| describe_fault(id, f)).collect()
    }

    pub fn snapshot(&mut self) -> SimSnapshot {
        let now = self.now;
        let mut pct = [0.0; 3];
        self.sys_latency.percentiles(now, &mut pct);

        let offered_rps = self.sys_offered.rate(now);
        let goodput_rps = self.sys_good.rate(now);
        let fail_rate = self.sys_failed.rate(now);
        let done = goodput_rps + fail_rate;
        let system = SystemStats {
            time_ms: now,
            offered_rps,
            goodput_rps,
            error_rate: if done > 0.0 { fail_rate / done } else { 0.0 },
            p50: pct[0],
            p95: pct[1],
            p99: pct[2],
            total_requests: self.total_requests,
            total_failed: self.total_failed,
        };

        let mut node_out: HashMap<String, NodeStats> = HashMap::new();
        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in &node_ids {
            let mut node_pct = [0.0; 3];
            let snap = self.nodes.get_mut(id).map(|state| {
                state.latency.percentiles(now, &mut node_pct);
                (
                    state.completions.rate(now),
                    state.errors.rate(now),
                    state.sheds.rate(now),
                    state.timeouts.rate(now),
                    state.hits.rate(now),
                    state.misses.rate(now),
                    state.arrivals.rate(now),
                    state.busy,
                    state.waiting.len() as f64,
                    state.utilization,
                    state.config.queue_limit.floor().max(0.0),
                    state.total_completed,
                    state.total_failed,
                )
            });
            let (
                completions,
                errors_per_sec,
                shed_rate,
                timeout_rate,
                hits,
                misses,
                arrival_rate,
                busy,
                queued,
                utilization,
                queue_limit,
                total_completed,
                total_failed,
            ) = match snap {
                Some(v) => v,
                None => continue,
            };
            // Timeouts belong in both halves of this ratio. They were in
            // neither: `errorRate` answered "how many of the requests that
            // did NOT time out went wrong", while the cell rendering it
            // says "failing", and a node losing a quarter of its traffic to
            // a slow dependency read 0%. engine.ts:833-880, PR #57.
            let resolved = completions + errors_per_sec + shed_rate + timeout_rate;

            // A fresh struct per snapshot, deliberately -- see
            // `engine.ts`'s doc comment on the same choice in its own
            // `snapshot()`: a memoised consumer must see a new identity
            // when there is something new to show.
            let mut entry = NodeStats {
                in_flight: busy,
                queued,
                throughput: completions,
                arrival_rate,
                utilization,
                p50: node_pct[0],
                p95: node_pct[1],
                p99: node_pct[2],
                error_rate: if resolved > 0.0 { (errors_per_sec + shed_rate + timeout_rate) / resolved } else { 0.0 },
                shed_rate,
                timeout_rate,
                hit_rate: if hits + misses > 0.0 { hits / (hits + misses) } else { 0.0 },
                total_completed,
                total_failed,
                instances: None,
                instances_pending: None,
                per_instance: None,
                queue_limit,
                target_instances: None,
                watched_instances: None,
                pending_instances: None,
                watched_id: None,
                watched_util: None,
                setpoint: None,
                scale_phase: None,
                phase_remaining_ms: None,
                scaling: None,
                active_region: None,
                failing_over: None,
                failover_remaining_ms: None,
                regions_healthy: None,
                regions_total: None,
                live_edge_id: None,
                stale_read_rate: 0.0,
                max_shard_utilization: 0.0,
                min_shard_utilization: 0.0,
                shard_utilization: Vec::new(),
                origin_fetch_rate: None,
                admitted_rate: None,
                throttled_rate: None,
                tokens: None,
                breaker_state: None,
                breaker_error_rate: None,
                rejected_rate: None,
                breaker_trips: None,
                open_remaining_ms: None,
                stale_search_rate: None,
                search_rate: None,
                index_write_rate: None,
                append_rate: None,
                range_query_rate: None,
                traversal_cost_ms: None,
                query_cost_ms: None,
                consumer_lag: None,
                consumer_lag_by_group: None,
                delivery_rate: None,
                retention_drop_rate: None,
                fanout: None,
                publish_amplification: None,
                connections_open: None,
                max_connections: None,
                connect_rate: None,
                connection_reject_rate: None,
                auth_reject_rate: None,
                consecutive_fails: None,
                upstream_fail_rate: None,
                cold_start_rate: None,
                cold_starts_per_sec: None,
                warm_idle: None,
                running_now: None,
                next_fire_in_ms: None,
                batch_emitted: None,
                burst_size: None,
                bulkhead_in_flight: None,
                bulkhead_limit: None,
                bulkhead_rejected_rate: None,
                watched_unscalable: None,
                bulkhead_waiting: None,
                bulkhead_acquire_latency_ms: None,
                bulkhead_acquire_timeout_rate: None,
                delivered_rate: None,
                redelivery_rate: None,
                dead_letter_rate: None,
                dead_letters: None,
                dirty_writes: None,
                flushed_rate: None,
                flush_fail_rate: None,
                edge_handled_rate: None,
                passed_through_rate: None,
                high_admitted_rate: None,
                low_admitted_rate: None,
                high_shedded_rate: None,
                low_shedded_rate: None,
                read_rate: None,
                write_rate: None,
                lock_wait_ms: None,
                slowdown_rate: None,
                output_rate: None,
                cpu_exceeded_rate: None,
            };

            // Kind-specific readouts.
            self.with_node_ext(id, |engine, behaviour, node_view, ext| {
                behaviour.decorate_stats(engine, node_view, &mut entry, ext);
            });

            // The instance vector is built AFTER decorate_stats -- see
            // `engine.ts`'s doc comment on the same ordering.
            let model = self.nodes.get(id).map(|s| s.behaviour.instance_model());
            match model {
                Some(Some(InstanceModel::Slots)) => {
                    self.fill_slot_instances(id);
                    self.finish_instances(id, &mut entry);
                }
                Some(Some(InstanceModel::Custom)) => {
                    if let Some(s) = self.nodes.get_mut(id) {
                        s.instance_pending = 0.0;
                    }
                    self.with_node_ext(id, |engine, behaviour, node_view, ext| {
                        behaviour.report_instances(engine, node_view, ext);
                    });
                    self.finish_instances(id, &mut entry);
                }
                _ => {}
            }

            node_out.insert(id.clone(), entry);
        }

        // Warm-up units belong to the node being SCALED, not to the
        // controller that ordered them.
        self.attach_pending_instances(&mut node_out);

        let mut edge_out: HashMap<String, f64> = HashMap::new();
        let edge_ids: Vec<String> = self.edge_flow.keys().cloned().collect();
        for id in edge_ids {
            if let Some(counter) = self.edge_flow.get(&id) {
                edge_out.insert(id, counter.rate(now));
            }
        }

        let edge_state = self.snapshot_edge_state(&edge_out);
        let failures_by_reason = self.failures.clone();
        let active_failures = self.snapshot_failures();

        SimSnapshot {
            system,
            nodes: node_out,
            history: self.history.clone(),
            edge_flow: edge_out,
            edge_state,
            failures_by_reason,
            active_failures,
            trace: self.last_trace.clone(),
        }
    }

    pub fn reset(&mut self) {
        self.rng = Rng::new(self.seed);
        self.now = 0.0;
        self.seq = 0;
        // `nodes.clear()` below makes every node `build_nodes()` sees "new",
        // so every one gets a fresh generation; rewinding the counter here
        // is what makes those fresh generations start from 1 again, the
        // same way the first build from `Engine::new()` did. PR #33.
        self.next_arrival_generation = 1;
        self.heap.clear();
        self.pool.reset();
        self.sys_latency.reset();
        self.sys_offered.reset();
        self.sys_good.reset();
        self.sys_failed.reset();
        self.total_requests = 0.0;
        self.total_failed = 0.0;
        self.failures = empty_failures_by_reason();
        self.history.clear();
        self.last_history_ms = 0.0;
        // Edge-flow counters are deliberately carried across set_topology
        // (so a live edit does not blank every edge label), but reset()
        // rewinds the clock to 0, and a stale bucket would double-count.
        for counter in self.edge_flow.values_mut() {
            counter.reset();
        }
        // Chaos is part of the run, not of the topology.
        self.faults.clear();
        self.cut_edges.clear();
        // The trace belongs to the run that produced it.
        self.tracing = None;
        self.last_trace = None;
        // Undo anything a controller wrote back into the topology during
        // the run, so the rebuild below starts from the capacities the
        // run started with.
        let authored_capacity = self.authored_capacity.clone();
        let authored_shard_capacity = self.authored_shard_capacity.clone();
        let authored_instances = self.authored_instances.clone();
        for n in self.topology.nodes.iter_mut() {
            if let Some(cap) = authored_capacity.get(&n.id) {
                n.config.capacity = *cap;
            }
            if let Some(cap) = authored_shard_capacity.get(&n.id) {
                n.config.shard_capacity = *cap;
            }
            // `instances` is optional, so an authored value of `None` is a
            // real state to restore to, not a missing entry: a scale-up
            // wrote a number here, and leaving it would replay the run from
            // the grown fleet instead of from what the design was authored
            // with. Matching on the OUTER `Option` from `.get()` (entry
            // present at all) rather than flattening is what lets `None`
            // still be applied. PR #48.
            if let Some(inst) = authored_instances.get(&n.id) {
                n.config.instances = *inst;
            }
        }
        // Drop every node's runtime state instead of carrying it into the
        // new run. `build_nodes()` reuses a previous `NodeState` whenever
        // the kind matches -- which is exactly what preserves in-flight
        // work across a LIVE topology edit -- but across a reset that reuse
        // is wrong: the clock is back at 0, so busy slots, waiting queues,
        // the per-node rate counters, the latency ring, autoscaler
        // warm-up (`instance_*`) and behaviour-private `ext` scratch all
        // belong to a run that is over, and they made a reset run differ
        // from the first one (reset's whole contract is reproducing the
        // original trajectory from t=0). The web engine sidesteps this by
        // calling `buildNodes(null)` from `reset()` (engine.ts:1054),
        // which turns the reuse path off for exactly this call; clearing
        // `self.nodes` here is the same switch. `out`/`ctrl`/`sources` are
        // rebuilt from the topology a few lines below either way.
        self.nodes.clear();
        self.build_nodes();
    }

    /* ---------------- topology wiring ---------------- */

    fn build_nodes(&mut self) {
        let mut previous = std::mem::take(&mut self.nodes);
        let mut next: HashMap<String, NodeState> = HashMap::new();
        self.client_ids.clear();
        // Retained clients (same id, same kind, reused `NodeState`) keep
        // whatever `Arrival` event they already have in flight; only a
        // genuinely new incarnation needs one scheduled below. PR #33.
        let mut new_client_ids: Vec<String> = Vec::new();

        for node in &self.topology.nodes {
            let kept = previous.remove(&node.id);
            let is_new = match &kept {
                Some(s) => s.kind != node.kind,
                None => true,
            };
            let mut state = match kept {
                Some(mut s) if s.kind == node.kind => {
                    // Preserve in-flight work across a hot swap. A
                    // controller (the autoscaler) writes its scale decision
                    // into `s.config` via `set_scale()` and into the live
                    // topology mirror, but never into `authored_instances`/
                    // `authored_shard_capacity`; `node.config` here may
                    // still be the authored shell's (a fresh `set_topology`
                    // call built from the frontend's own, non-autoscaled
                    // mirror), so overwriting `s.config` with it
                    // unconditionally would revert a live autoscaled fleet
                    // mid-run and leave the controller's target pointing at
                    // a size the node no longer has. Keep the live value
                    // whenever it diverges from authored -- a student edit
                    // cannot be mistaken for a controller write, since
                    // `update_node_config` moves the authored map and
                    // `s.config` together and so shows no divergence.
                    // engine.ts `buildNodes`:1106-1131, PR #48.
                    let field = s.behaviour.scale_field();
                    let live_instances = if field == Some(crate::sim::behaviour::ScaleField::Instances)
                        && s.config.instances != self.authored_instances.get(&node.id).cloned().flatten()
                    {
                        Some(s.config.instances)
                    } else {
                        None
                    };
                    let live_shard_capacity = if field == Some(crate::sim::behaviour::ScaleField::ShardCapacity)
                        && self
                            .authored_shard_capacity
                            .get(&node.id)
                            .map(|a| *a != s.config.shard_capacity)
                            .unwrap_or(false)
                    {
                        Some(s.config.shard_capacity)
                    } else {
                        None
                    };
                    s.config = node.config.clone();
                    if let Some(v) = live_instances {
                        s.config.instances = v;
                    }
                    if let Some(v) = live_shard_capacity {
                        s.config.shard_capacity = v;
                    }
                    s.out.clear();
                    s.ctrl.clear();
                    s.sources.clear();
                    s
                }
                _ => {
                    let mut s = create_node_state(node);
                    // A genuinely new incarnation of this id (freshly
                    // created, or kept-by-id but retyped) gets its own
                    // arrival generation, so a stale `Arrival` event
                    // scheduled by whatever this id used to be is
                    // recognized and dropped at `dispatch()` instead of
                    // firing against this new state. PR #33.
                    s.arrival_generation = self.next_arrival_generation;
                    self.next_arrival_generation += 1;
                    s
                }
            };
            state.last_integrate_ms = self.now;
            if state.behaviour.generates_load() {
                self.client_ids.push(node.id.clone());
                if is_new {
                    new_client_ids.push(node.id.clone());
                }
            }
            next.insert(node.id.clone(), state);
        }

        for edge in &self.topology.edges {
            if !next.contains_key(&edge.from) || !next.contains_key(&edge.to) {
                continue;
            }
            let from_controls_target = next.get(&edge.from).unwrap().behaviour.controls_target();
            // A control edge is a supervisory relationship, not a traffic
            // path, so it never enters the routing set.
            if edge.control == Some(true) || from_controls_target {
                next.get_mut(&edge.from).unwrap().ctrl.push(edge.clone());
                continue;
            }
            let (to_pulls, from_buffers, from_id) = {
                let to = next.get(&edge.to).unwrap();
                let from = next.get(&edge.from).unwrap();
                (to.behaviour.pulls_from_queues(), from.behaviour.buffers_for_consumers(), from.id.clone())
            };
            next.get_mut(&edge.from).unwrap().out.push(edge.clone());
            if to_pulls && from_buffers {
                next.get_mut(&edge.to).unwrap().sources.push(from_id);
            }
        }
        // A consumer may be drawn in either direction relative to its
        // buffer; treat an outgoing edge to a buffer as a pull source too.
        let mut additions: Vec<(String, String)> = Vec::new();
        for state in next.values() {
            if !state.behaviour.pulls_from_queues() {
                continue;
            }
            for edge in &state.out {
                if let Some(target) = next.get(&edge.to) {
                    if target.behaviour.buffers_for_consumers() && !state.sources.contains(&target.id) {
                        additions.push((state.id.clone(), target.id.clone()));
                    }
                }
            }
        }
        for (state_id, target_id) in additions {
            if let Some(state) = next.get_mut(&state_id) {
                if !state.sources.contains(&target_id) {
                    state.sources.push(target_id);
                }
            }
        }

        self.nodes = next;

        // Drop state belonging to removed nodes: their in-flight requests
        // are gone. NOTE: TS runs this discard pass BEFORE reassigning
        // `this.nodes`, while the old (complete) map is still visible to
        // any `this.nodes.get()` lookup a cascading `resolve()` makes.
        // Rust cannot keep a `kept` node simultaneously reachable through
        // both `previous` and the newly assigned `self.nodes` without
        // cloning `NodeState` (impossible: `ext` is not `Clone`), so this
        // reassigns `self.nodes` FIRST. The only observable difference is
        // during this brief window, if discarding a removed node cascades
        // (via retry/observesOutcome) into a caller that was ALSO
        // rebuilt in this same edit: that caller is now looked up via its
        // freshly rebuilt state rather than its pre-edit one. Flagged in
        // the porting report as a deliberate, narrow deviation.
        for (_, state) in previous.into_iter() {
            self.discard_node_state(state);
        }

        let mut flow: HashMap<String, RateCounter> = HashMap::new();
        for edge in &self.topology.edges {
            let counter = self.edge_flow.remove(&edge.id).unwrap_or_else(RateCounter::new);
            flow.insert(edge.id.clone(), counter);
        }
        self.edge_flow = flow;

        // Start the generator for each NEW client only. A retained client
        // keeps the `Arrival` event it already had scheduled; re-arming it
        // here on every live edit is what let repeated structural edits
        // multiply one client's measured rate far past its configured rps
        // (10 edits moved a 51.1 rps client to 488.9 measured rps).
        // engine.ts `buildNodes`, PR #33.
        for id in &new_client_ids {
            self.schedule_arrival(id);
        }

        let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
        for id in &node_ids {
            if let Some(state) = self.nodes.get_mut(id) {
                state.poll_scheduled = false;
            }
            // Allocate behaviour-private state before anything can run. A
            // node kept across a hot swap keeps the state it already had.
            let needs_init = self.nodes.get(id).map(|s| s.ext.is_none()).unwrap_or(false);
            if needs_init {
                self.with_node_ext(id, |_engine, behaviour, node_view, ext| {
                    *ext = behaviour.init_state(node_view);
                });
            }
            self.pump_queue(id);
        }
    }

    fn discard_node_state(&mut self, mut state: NodeState) {
        let waiting: Vec<ReqHandle> = state.waiting.drain(..).collect();
        for handle in waiting {
            self.resolve(handle, false, FailureReason::NoRoute, 0.0);
        }
        state.busy = 0.0;
    }
}

/* ------------------------------------------------------------------ *
 * BehaviourCtx
 *
 * The small, deliberate set of engine operations a behaviour may drive.
 * ------------------------------------------------------------------ */

impl BehaviourCtx for Engine {
    fn now(&self) -> f64 {
        self.now
    }

    fn roll(&mut self) -> f64 {
        self.rng.next()
    }

    fn queue_depth(&self, state: &dyn NodeStateLike) -> f64 {
        match self.nodes.get(state.id()) {
            Some(s) => s.waiting.len() as f64,
            None => 0.0,
        }
    }

    fn effective_queue_limit(&self, state: &dyn NodeStateLike) -> f64 {
        state.config().queue_limit.floor().max(0.0)
    }

    fn effective_capacity(&self, state: &dyn NodeStateLike) -> f64 {
        state.config().capacity.floor().max(1.0) * self.effective_instances(state)
    }

    fn effective_instances(&self, state: &dyn NodeStateLike) -> f64 {
        match state.config().instances {
            None => 1.0,
            // `.max(1.0)` already answers "NaN" the way this needs -- Rust's
            // `f64::max` returns the non-NaN operand, unlike JS's
            // `Math.max(1, Math.floor(NaN))` (which stays NaN and crashes
            // `units.length = NaN` downstream, the bug PR #58 fixes). What
            // Rust still needs is the UPPER bound: an unbounded or infinite
            // `instances` is an unbounded per-instance array allocation in
            // `fill_slot_instances`/`finish_instances`. engine.ts
            // `effectiveInstances`, `MAX_INSTANCES`, PR #58.
            Some(raw) => raw.floor().max(1.0).min(MAX_INSTANCES),
        }
    }

    fn count_hit(&mut self, state: &dyn NodeStateLike) {
        let now = self.now;
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.hits.add(now, 1.0);
        }
    }

    fn count_miss(&mut self, state: &dyn NodeStateLike) {
        let now = self.now;
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.misses.add(now, 1.0);
        }
    }

    fn ack_and_buffer(&mut self, state: &dyn NodeStateLike, req: &dyn ReqLike) {
        let node_id = state.id().to_string();
        let handle = match req_handle_of(req) {
            Some(h) => h,
            None => return,
        };
        self.ack_and_buffer_impl(&node_id, handle);
    }

    fn ack_and_relay(&mut self, state: &dyn NodeStateLike, req: &dyn ReqLike, extra_delivery_ms: Option<f64>) {
        let node_id = state.id().to_string();
        let handle = match req_handle_of(req) {
            Some(h) => h,
            None => return,
        };
        self.ack_and_relay_impl(&node_id, handle, extra_delivery_ms);
    }

    fn pick_weighted_or_least_loaded(&mut self, out: &[SimEdge]) -> Option<SimEdge> {
        // A partitioned edge is not a candidate; see `engine.ts`'s doc
        // comment on this method for why that matters more than it looks
        // (a cut edge's target is spuriously the idlest thing around).
        let any_cut = !self.cut_edges.is_empty();

        if out.len() == 1 {
            return if any_cut && self.cut_edges.contains(&out[0].id) {
                None
            } else {
                Some(out[0].clone())
            };
        }

        let mut total = 0.0;
        let mut uniform = true;
        let mut first_live: Option<usize> = None;
        for i in 0..out.len() {
            if any_cut && self.cut_edges.contains(&out[i].id) {
                continue;
            }
            let w = if out[i].weight > 0.0 { out[i].weight } else { 0.0 };
            total += w;
            match first_live {
                None => first_live = Some(i),
                Some(fl) => {
                    if (out[i].weight - out[fl].weight).abs() > 1e-9 {
                        uniform = false;
                    }
                }
            }
        }
        let first_live = match first_live {
            Some(i) => i,
            None => return None,
        };

        if uniform {
            // Least-loaded: fewest queued+busy relative to capacity, ties
            // by index.
            let mut best: Option<SimEdge> = None;
            let mut best_load = f64::INFINITY;
            for i in 0..out.len() {
                if any_cut && self.cut_edges.contains(&out[i].id) {
                    continue;
                }
                let target = match self.nodes.get(&out[i].to) {
                    Some(t) => t,
                    None => continue,
                };
                let depth = target.busy + target.waiting.len() as f64;
                let cap = if target.config.capacity > 0.0 { target.config.capacity } else { 1.0 };
                let load = depth / cap;
                if load < best_load {
                    best_load = load;
                    best = Some(out[i].clone());
                }
            }
            return best.or_else(|| Some(out[first_live].clone()));
        }

        if total <= 0.0 {
            return Some(out[first_live].clone());
        }
        let mut r = self.rng.next() * total;
        let mut last = out[first_live].clone();
        for i in 0..out.len() {
            if any_cut && self.cut_edges.contains(&out[i].id) {
                continue;
            }
            let w = if out[i].weight > 0.0 { out[i].weight } else { 0.0 };
            last = out[i].clone();
            r -= w;
            if r <= 0.0 {
                return Some(out[i].clone());
            }
        }
        Some(last)
    }

    fn fail(&mut self, req: &dyn ReqLike, reason: FailureReason, latency_ms: f64) {
        if let Some(h) = req_handle_of(req) {
            self.resolve(h, false, reason, latency_ms);
        }
    }

    fn reject(&mut self, state: &dyn NodeStateLike, req: &dyn ReqLike, reason: FailureReason) {
        let now = self.now;
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.errors.add(now, 1.0);
            s.total_failed += 1.0;
        }
        if let Some(h) = req_handle_of(req) {
            self.resolve(h, false, reason, 0.0);
        }
    }

    fn resume_admission(&mut self, state: &dyn NodeStateLike, req: ReqHandle) {
        // Mirrors `admit()`'s own `AdmitAction::Passthru` arm: zero the
        // drawn service time, then hand off through the same zero-service
        // path a fresh pass-through admission uses. `on_admit` is
        // deliberately NOT re-invoked -- it already ran once, when this
        // request first reached the node; re-running it would ask the
        // bulkhead to admit the same request a second time. Port of
        // `engine.ts`'s `resumeAdmission` (upstream `351327c4`, PR #77).
        if let Some(r) = self.pool.get_mut(req) {
            r.own_ms = 0.0;
        }
        let node_id = state.id().to_string();
        self.begin_zero_service(&node_id, req);
    }

    fn wake_after(&mut self, state: &dyn NodeStateLike, req: ReqHandle, delay_ms: f64) {
        self.push(self.now + delay_ms.max(0.0), EvKind::BehaviourWake, state.id(), Some(req));
    }

    fn count_custom(&mut self, state: &dyn NodeStateLike, name: &str, n: f64) {
        let now = self.now;
        if let Some(s) = self.nodes.get_mut(state.id()) {
            let map = s.custom.get_or_insert_with(HashMap::new);
            map.entry(name.to_string()).or_insert_with(RateCounter::new).add(now, n);
        }
    }

    fn counter_rate(&self, state: &dyn NodeStateLike, name: &str) -> f64 {
        match self.nodes.get(state.id()).and_then(|s| s.custom.as_ref()).and_then(|m| m.get(name)) {
            Some(c) => c.rate(self.now),
            None => 0.0,
        }
    }

    fn utilization_of(&self, node_id: &str) -> Option<f64> {
        self.nodes.get(node_id).map(|s| s.utilization)
    }

    fn scale_of(&self, node_id: &str) -> Option<f64> {
        let state = self.nodes.get(node_id)?;
        let field = state.behaviour.scale_field()?;
        match field {
            crate::sim::behaviour::ScaleField::ShardCapacity => Some(state.config.shard_capacity.floor().max(1.0)),
            crate::sim::behaviour::ScaleField::Instances => Some(self.effective_instances(state)),
        }
    }

    fn set_scale(&mut self, node_id: &str, units: f64) {
        let next = units.floor().max(1.0);
        let field = match self.nodes.get(node_id).and_then(|s| s.behaviour.scale_field()) {
            Some(f) => f,
            None => return,
        };
        let current = match field {
            crate::sim::behaviour::ScaleField::Instances => self.nodes.get(node_id).map(|s| s.config.instances.unwrap_or(1.0)),
            crate::sim::behaviour::ScaleField::ShardCapacity => self.nodes.get(node_id).map(|s| s.config.shard_capacity),
        };
        if current == Some(next) {
            return;
        }
        if let Some(state) = self.nodes.get_mut(node_id) {
            match field {
                crate::sim::behaviour::ScaleField::Instances => state.config.instances = Some(next),
                crate::sim::behaviour::ScaleField::ShardCapacity => state.config.shard_capacity = next,
            }
        }
        if let Some(node) = self.topology.nodes.iter_mut().find(|n| n.id == node_id) {
            match field {
                crate::sim::behaviour::ScaleField::Instances => node.config.instances = Some(next),
                crate::sim::behaviour::ScaleField::ShardCapacity => node.config.shard_capacity = next,
            }
        }
        self.pump_queue(node_id);
    }

    fn control_target_of(&self, state: &dyn NodeStateLike) -> String {
        let ctrl = match self.nodes.get(state.id()) {
            Some(s) => &s.ctrl,
            None => return String::new(),
        };
        if ctrl.is_empty() {
            return String::new();
        }
        if self.cut_edges.contains(&ctrl[0].id) {
            return String::new();
        }
        ctrl[0].to.clone()
    }

    fn is_crashed(&self, node_id: &str) -> bool {
        self.faults.get(node_id).map(|f| f.kind == FailureKind::Crash).unwrap_or(false)
    }

    fn is_edge_cut(&self, edge_id: &str) -> bool {
        self.cut_edges.contains(edge_id)
    }

    fn serve_within(
        &mut self,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    ) {
        let node_id = state.id().to_string();
        let handle = match req_handle_of(req) {
            Some(h) => h,
            None => return,
        };
        self.serve_within_impl(&node_id, handle, on_drained);
    }

    fn handle_of(&self, req: &dyn ReqLike) -> ReqHandle {
        // Falls back to a handle that can never resolve (`get`/`get_mut`
        // always return `None` for it) rather than panicking, matching
        // this file's house style of failing the one operation that
        // needed identity rather than the whole simulation.
        req_handle_of(req).unwrap_or(ReqHandle { index: u32::MAX, generation: u32::MAX })
    }

    fn serve_deferred(
        &mut self,
        state: &dyn NodeStateLike,
        req: ReqHandle,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    ) {
        let node_id = state.id().to_string();
        self.serve_within_impl(&node_id, req, on_drained);
    }

    fn add_service_delay(&mut self, req: &dyn ReqLike, extra_ms: f64) {
        if !(extra_ms > 0.0) {
            return;
        }
        if let Some(h) = req_handle_of(req) {
            if let Some(r) = self.pool.get_mut(h) {
                r.extra_service_ms += extra_ms;
            }
        }
    }

    fn add_service_delay_deferred(&mut self, req: ReqHandle, extra_ms: f64) {
        if !(extra_ms > 0.0) {
            return;
        }
        if let Some(r) = self.pool.get_mut(req) {
            r.extra_service_ms += extra_ms;
        }
    }

    fn report_shard_utilization(&mut self, state: &dyn NodeStateLike, per_shard: &[f64]) {
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.shard_util.clear();
            s.shard_util.extend_from_slice(per_shard);
        }
    }

    fn report_instances(&mut self, state: &dyn NodeStateLike, per_unit: &[f64], pending: f64) {
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.instance_units.clear();
            s.instance_units.extend_from_slice(per_unit);
            s.instance_pending = if pending > 0.0 { pending.floor() } else { 0.0 };
        }
    }

    fn report_occupancy(&mut self, state: &dyn NodeStateLike, busy_slots: f64, capacity: f64) {
        let busy = busy_slots.max(0.0);
        let cap = if capacity > 0.0 { capacity } else { 1.0 };
        if let Some(s) = self.nodes.get_mut(state.id()) {
            s.own_occupancy = Some((busy, cap));
        }
    }

    fn mark_write(&mut self, req: &dyn ReqLike, is_write: bool) {
        if let Some(h) = req_handle_of(req) {
            if let Some(r) = self.pool.get_mut(h) {
                r.is_write = is_write;
            }
        }
    }

    fn emit_detached(&mut self, state: &dyn NodeStateLike, edge: &SimEdge, key: u32) -> bool {
        if self.pool.live >= MAX_LIVE_REQUESTS {
            return false;
        }
        if self.cut_edges.contains(&edge.id) {
            return false;
        }
        if !self.nodes.contains_key(&edge.to) {
            return false;
        }
        let node_id = state.id().to_string();
        let mut msg = Req::fresh();
        msg.node_id = node_id.clone();
        msg.parent = None;
        msg.root_start_ms = self.now;
        msg.enter_ms = self.now;
        msg.hop = 0;
        msg.detached = true;
        msg.key = key % KEYSPACE;
        msg.pending = 1;
        let handle = self.pool.acquire(msg);
        self.send_child(&node_id, handle, edge, 0);
        true
    }
}

/* ------------------------------------------------------------------ *
 * helpers
 * ------------------------------------------------------------------ */

fn create_node_state(node: &SimNode) -> NodeState {
    NodeState {
        id: node.id.clone(),
        kind: node.kind,
        behaviour: behaviour_for(node.kind),
        config: node.config.clone(),
        busy: 0.0,
        waiting: VecDeque::new(),
        out: Vec::new(),
        ctrl: Vec::new(),
        sources: Vec::new(),
        busy_ms_accum: 0.0,
        last_integrate_ms: 0.0,
        utilization: 0.0,
        arrivals: RateCounter::new(),
        completions: RateCounter::new(),
        errors: RateCounter::new(),
        sheds: RateCounter::new(),
        timeouts: RateCounter::new(),
        hits: RateCounter::new(),
        misses: RateCounter::new(),
        latency: LatencyRing::new(),
        total_completed: 0.0,
        total_failed: 0.0,
        // Reassigned in `build_nodes()` for a genuinely new incarnation;
        // left at 0 here the same way TS's `createNodeState` does, since
        // this function has no access to the engine's generation counter.
        arrival_generation: 0,
        poll_scheduled: false,
        shard_util: Vec::new(),
        instance_units: Vec::new(),
        instance_published: None,
        instance_pending: 0.0,
        instance_pending_published: -1.0,
        ext: None,
        custom: None,
        own_occupancy: None,
    }
}

/// Project an internal `Fault` into the public `ActiveFailure` shape,
/// carrying only the knobs that actually apply to its kind.
fn describe_fault(node_id: &str, f: &Fault) -> ActiveFailure {
    let mut out = ActiveFailure {
        node_id: node_id.to_string(),
        kind: f.kind,
        since_ms: f.since_ms,
        factor: None,
        rate: None,
        edge_ids: None,
    };
    match f.kind {
        FailureKind::Slow => out.factor = Some(f.factor),
        FailureKind::Errors => out.rate = Some(f.rate),
        FailureKind::Partition => out.edge_ids = Some(f.edge_ids.clone()),
        FailureKind::Crash => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::presets::boot_topology;

    /// Reset must leave NOTHING of the run that just finished behind.
    ///
    /// `build_nodes()` reuses a previous `NodeState` whenever the node's
    /// kind is unchanged -- the mechanism that preserves in-flight work
    /// across a LIVE topology edit -- and `reset()` was feeding it the
    /// states it had just unwound the clock past: busy slots, waiting
    /// queues, lifetime counters, rate windows, latency rings, autoscaler
    /// warm-up and behaviour-private `ext` scratch all carried over. The
    /// fix clears `self.nodes` first (the desktop equivalent of the web
    /// engine's `buildNodes(null)` call at engine.ts:1054), which is what
    /// this pins: after `reset()`, every node is as fresh as one built at
    /// t=0.
    #[test]
    fn reset_zeroes_every_nodes_runtime_state() {
        let mut engine = Engine::new(boot_topology(), 1);
        engine.advance(3_000.0);

        // Guard against the assertion below passing vacuously: the boot
        // topology has to have actually done some work first.
        let worked = engine.nodes.values().any(|n| {
            n.total_completed > 0.0 || n.total_failed > 0.0 || n.busy > 0.0 || !n.waiting.is_empty()
        });
        assert!(worked, "boot topology ran for 3s without touching a request");

        engine.reset();

        assert_eq!(engine.now, 0.0, "the clock must rewind to zero");
        assert_eq!(
            engine.nodes.len(),
            engine.topology.nodes.len(),
            "reset must rebuild state for every node in the topology"
        );
        for (id, state) in &engine.nodes {
            assert_eq!(state.total_completed, 0.0, "lifetime completions survived for {id}");
            assert_eq!(state.total_failed, 0.0, "lifetime failures survived for {id}");
            assert_eq!(state.busy, 0.0, "busy slots survived for {id}");
            assert!(state.waiting.is_empty(), "waiting queue survived for {id}");
        }
    }

    /// The user-visible half of the same contract: with the clock, RNG,
    /// heap and node state all rewound, the same advance must reproduce
    /// the same work node-for-node (AGENTS.md: "same seed and topology
    /// means byte-identical snapshots").
    ///
    /// This is the behaviour Reset is FOR -- replaying a scenario to see
    /// the same answer -- and carrying node state across the reset broke
    /// it: the second run started from the end of the first, so its
    /// numbers differed and only a page reload reproduced them.
    #[test]
    fn a_reset_run_repeats_the_first_run() {
        let mut engine = Engine::new(boot_topology(), 7);
        engine.advance(2_000.0);

        let first: HashMap<String, f64> = engine
            .nodes
            .iter()
            .map(|(id, state)| (id.clone(), state.total_completed))
            .collect();
        assert!(
            first.values().any(|v| *v > 0.0),
            "boot topology ran for 2s without completing anything"
        );

        engine.reset();
        engine.advance(2_000.0);

        for (id, state) in &engine.nodes {
            let expected = first.get(id).copied().unwrap_or(f64::NAN);
            assert_eq!(
                state.total_completed, expected,
                "node {id} did different work after reset"
            );
        }
    }

    /* ---------------- Phase 3A engine-correctness fixes ---------------- */

    use crate::sim::presets::base_config;

    fn node(id: &str, kind: NodeKind, config: NodeConfig) -> SimNode {
        SimNode { id: id.to_string(), kind, label: id.to_string(), x: 0.0, y: 0.0, config }
    }

    fn edge(id: &str, from: &str, to: &str) -> SimEdge {
        SimEdge {
            id: id.to_string(),
            from: from.to_string(),
            to: to.to_string(),
            weight: 1.0,
            control: None,
            latency_ms: None,
            bandwidth_rps: None,
            loss_rate: None,
        }
    }

    /// PR #56 (`behaviour.ts`'s `lb`): an lb's `onAdmit` must stop
    /// returning `passthru` so `capacity`/`instances`/`queueLimit` bind
    /// something. Before the fix `AdmitAction::Passthru` routed every call
    /// through `begin_zero_service`, which never touches `busy`/`waiting`
    /// at all -- so a pair sized for 2 concurrent calls carried arbitrary
    /// load with `queued` pinned at 0 no matter how overloaded it was.
    #[test]
    fn lb_admits_through_its_own_pool() {
        let topo = Topology {
            nodes: vec![
                node("client", NodeKind::Client, NodeConfig { rps: 1000.0, ..base_config(NodeKind::Client) }),
                node(
                    "lb",
                    NodeKind::Lb,
                    NodeConfig {
                        capacity: 2.0,
                        instances: Some(1.0),
                        service_ms: 100.0,
                        queue_limit: 1000.0,
                        ..base_config(NodeKind::Lb)
                    },
                ),
                node(
                    "svc",
                    NodeKind::Service,
                    NodeConfig { capacity: 1000.0, service_ms: 1.0, ..base_config(NodeKind::Service) },
                ),
            ],
            edges: vec![edge("e1", "client", "lb"), edge("e2", "lb", "svc")],
            annotations: None,
        };
        let mut engine = Engine::new(topo, 1);
        engine.advance(200.0);
        let snap = engine.snapshot();
        let lb = snap.nodes.get("lb").expect("lb in snapshot");

        assert!(
            lb.in_flight <= 2.0 + 1e-9,
            "lb's in_flight must be bounded by its own capacity*instances (2), got {}",
            lb.in_flight
        );
        assert!(
            lb.queued > 10.0,
            "an lb overloaded well past its own capacity must show a real backlog, not stay a silent passthrough (got {})",
            lb.queued
        );
    }

    /// PR #57: a timeout must be counted exactly once. `on_timeout` owns
    /// the `timeouts` counter; `resolve()` owns root `totalFailed`. Before
    /// the fix both bumped `total_failed` for the same timeout (once in
    /// `on_timeout` for the caller, once again when `resolve()` finally
    /// resolved the root as failed with reason `Timeout`), so one real
    /// timeout read as two failures -- up to 26% phantom loss upstream
    /// measured.
    #[test]
    fn a_timeout_is_counted_exactly_once() {
        let topo = Topology {
            nodes: vec![
                node(
                    "client",
                    NodeKind::Client,
                    NodeConfig { rps: 100.0, timeout_ms: 10.0, retries: 0.0, ..base_config(NodeKind::Client) },
                ),
                node(
                    "svc",
                    NodeKind::Service,
                    // Long enough that nothing ever completes inside this
                    // test's window -- every call must resolve via timeout.
                    NodeConfig { capacity: 10.0, service_ms: 1.0e8, ..base_config(NodeKind::Service) },
                ),
            ],
            edges: vec![edge("e1", "client", "svc")],
            annotations: None,
        };
        let mut engine = Engine::new(topo, 3);
        // `advance()` clamps a single call to `MAX_DELTA_MS` (100ms), so
        // reaching 500 simulated ms takes five calls, not one -- the same
        // pattern every other multi-tick test in this module loops in
        // frame-sized steps for.
        for _ in 0..5 {
            engine.advance(100.0);
        }

        let total_requests = engine.total_requests;
        assert!(total_requests > 1.0, "test must generate multiple requests to meaningfully exercise the counter");
        // Not exact equality: the most recently arrived request(s) -- those
        // still inside their 10ms deadline at the instant of this snapshot
        // -- have not timed out yet and are legitimately still in flight.
        // The invariant this test actually guards is "not double-counted"
        // (the old bug read roughly 2x total_requests here), so a small
        // tolerance for that handful of in-flight stragglers is correct;
        // exact equality is not.
        assert!(
            engine.total_failed <= total_requests,
            "engine-level failures must never exceed total requests -- a double-counted timeout would read 2x total_requests (requests {total_requests}, failed {})",
            engine.total_failed
        );
        assert!(
            total_requests - engine.total_failed <= 5.0,
            "all but a handful of still-in-flight stragglers must have resolved as failures by now (requests {total_requests}, failed {})",
            engine.total_failed
        );

        let snap = engine.snapshot();
        let client = snap.nodes.get("client").expect("client in snapshot");
        assert_eq!(
            client.total_failed, engine.total_failed,
            "the client's own totalFailed must match the engine's, not double- or under-count it"
        );
    }

    /// PR #33: a retained client must keep its already-scheduled `Arrival`
    /// event across a structural edit rather than getting a second one
    /// stacked on top. Before the fix `build_nodes()` called
    /// `schedule_arrival` for every client on every edit, so N edits left N
    /// extra arrival streams running concurrently with the original --
    /// upstream measured a 51.1 rps client read as 488.9 rps after 10
    /// edits.
    #[test]
    fn repeated_structural_edits_do_not_inflate_a_retained_clients_rate() {
        let topo = Topology {
            nodes: vec![
                node("client", NodeKind::Client, NodeConfig { rps: 50.0, ..base_config(NodeKind::Client) }),
                node(
                    "svc",
                    NodeKind::Service,
                    NodeConfig { capacity: 1000.0, service_ms: 1.0, ..base_config(NodeKind::Service) },
                ),
            ],
            edges: vec![edge("e1", "client", "svc")],
            annotations: None,
        };

        let mut baseline = Engine::new(topo.clone(), 42);
        // Same 100ms-chunked loop the edited run below uses (`advance()`
        // clamps a single call to `MAX_DELTA_MS`, 100ms) -- both runs must
        // cover equal elapsed simulated time or the ratio compares a
        // shorter window against a longer one instead of stable-vs-edited.
        for _ in 0..10 {
            baseline.advance(100.0);
        }
        let baseline_count = baseline.total_requests;
        assert!(baseline_count > 0.0, "baseline run must generate traffic");

        let mut edited = Engine::new(topo.clone(), 42);
        for _ in 0..10 {
            edited.advance(100.0);
            // Re-submitting the identical topology is a live structural
            // edit from the engine's point of view: build_nodes() runs
            // again and the client's NodeState is KEPT (same id, same
            // kind) -- exactly the retained-client path the fix targets.
            edited.set_topology(topo.clone());
        }
        let edited_count = edited.total_requests;

        let ratio = edited_count / baseline_count;
        assert!(
            ratio < 1.5,
            "10 structural edits must not multiply a retained client's measured rate (baseline {baseline_count}, edited {edited_count}, ratio {ratio})"
        );
    }

    /// PR #48: a controller's scale decision must survive a hot swap. A
    /// controller writes `instances` via `set_scale()` without updating the
    /// authored map; before the fix `build_nodes()` always overwrote a
    /// kept node's config with the incoming topology's (authored) one,
    /// silently reverting a live autoscaled fleet on every structural edit.
    #[test]
    fn a_controller_scaled_instance_count_survives_a_hot_swap() {
        let topo = Topology {
            nodes: vec![node(
                "svc",
                NodeKind::Service,
                NodeConfig { capacity: 10.0, ..base_config(NodeKind::Service) },
            )],
            edges: vec![],
            annotations: None,
        };
        let mut engine = Engine::new(topo.clone(), 1);
        assert_eq!(engine.scale_of("svc"), Some(1.0), "service starts at the authored instance count");

        // Simulate the autoscaler scaling this node up.
        engine.set_scale("svc", 5.0);
        assert_eq!(engine.scale_of("svc"), Some(5.0));

        // A live structural edit elsewhere re-submits the SAME (authored,
        // un-scaled) node config, exactly as the frontend's own topology
        // mirror would -- it never learned about the controller's write.
        engine.set_topology(topo.clone());

        assert_eq!(
            engine.scale_of("svc"),
            Some(5.0),
            "the controller's scale-up must survive a hot swap, not revert to the authored instance count"
        );
    }

    /// PR #58: an absurd `instances` must not blow up the per-instance
    /// snapshot arrays (`fill_slot_instances` allocates one `f64` per
    /// instance), and a non-numeric one must not propagate NaN into them.
    #[test]
    fn huge_or_nan_instances_does_not_crash_the_snapshot() {
        let topo = Topology {
            nodes: vec![node(
                "svc",
                NodeKind::Service,
                NodeConfig { capacity: 10.0, ..base_config(NodeKind::Service) },
            )],
            edges: vec![],
            annotations: None,
        };
        let mut engine = Engine::new(topo, 1);

        engine.update_node_config("svc", serde_json::json!({ "instances": 1.0e18 })).unwrap();
        engine.advance(10.0);
        let snap = engine.snapshot();
        let instances = snap.nodes.get("svc").and_then(|s| s.instances).unwrap_or(0.0);
        assert!(instances <= 512.0, "instances must be clamped to MAX_INSTANCES, got {instances}");

        // NaN cannot travel over the wire as JSON, so the realistic
        // non-numeric path is exercised directly against the stored config
        // rather than through update_node_config's JSON patch.
        if let Some(state) = engine.nodes.get_mut("svc") {
            state.config.instances = Some(f64::NAN);
        }
        engine.advance(10.0);
        let snap2 = engine.snapshot();
        let instances2 = snap2.nodes.get("svc").and_then(|s| s.instances).unwrap_or(0.0);
        assert_eq!(instances2, 1.0, "a NaN instances count must fall back to 1, not propagate NaN");
    }

    /// Commit `7cc1fea9` (no PR#): a request arriving once the live-request
    /// ceiling is already hit must still be counted as OFFERED and as a
    /// shed, not silently dropped -- a silent drop left every rate reading
    /// at zero while the design was maximally overloaded.
    #[test]
    fn an_arrival_past_the_live_request_ceiling_is_counted_as_a_shed() {
        let topo = Topology {
            nodes: vec![node("client", NodeKind::Client, NodeConfig { rps: 1.0, ..base_config(NodeKind::Client) })],
            edges: vec![],
            annotations: None,
        };
        let mut engine = Engine::new(topo, 1);
        engine.pool.live = MAX_LIVE_REQUESTS;

        let requests_before = engine.total_requests;
        let shed_before = *engine.failures.get(&FailureReason::Shed).unwrap_or(&0);

        engine.on_client_arrival("client");

        assert_eq!(
            engine.total_requests,
            requests_before + 1.0,
            "a request shed at the live-request ceiling must still be counted as offered"
        );
        assert_eq!(
            *engine.failures.get(&FailureReason::Shed).unwrap_or(&0),
            shed_before + 1,
            "it must also be booked as a shed, not dropped silently"
        );
        let snap = engine.snapshot();
        let client = snap.nodes.get("client").expect("client in snapshot");
        assert_eq!(client.total_failed, 1.0, "the client's own totalFailed must count this shed");
    }

    /// Upstream PR #63 fixes a NaN `regions`/`activeRegion` config killing
    /// a region switch's routing. This Rust port was found to already be
    /// immune: `region_count`'s `f64::min`/`max` return the non-NaN operand
    /// (unlike JS's `Math.min`/`Math.max`, which propagate NaN), and `NaN
    /// as i64` saturates to 0 (unlike JS's `Math.floor(NaN) === NaN`) --
    /// so this pins that finding rather than a fix: see
    /// `behaviour::control::region_count`/`RegionBehaviour::pick_edge`
    /// (untouched, owned by the sibling agent in this phase).
    #[test]
    fn region_switch_routes_through_nan_config_without_a_fix() {
        let topo = Topology {
            nodes: vec![
                node("client", NodeKind::Client, NodeConfig { rps: 50.0, ..base_config(NodeKind::Client) }),
                node(
                    "region",
                    NodeKind::Region,
                    NodeConfig {
                        regions: Some(f64::NAN),
                        active_region: Some(f64::NAN),
                        failover_ms: Some(0.0),
                        ..base_config(NodeKind::Region)
                    },
                ),
                node(
                    "svc0",
                    NodeKind::Service,
                    NodeConfig { capacity: 100.0, service_ms: 1.0, ..base_config(NodeKind::Service) },
                ),
                node(
                    "svc1",
                    NodeKind::Service,
                    NodeConfig { capacity: 100.0, service_ms: 1.0, ..base_config(NodeKind::Service) },
                ),
            ],
            edges: vec![
                edge("e1", "client", "region"),
                edge("e2", "region", "svc0"),
                edge("e3", "region", "svc1"),
            ],
            annotations: None,
        };
        let mut engine = Engine::new(topo, 1);
        engine.advance(500.0);
        let snap = engine.snapshot();
        let region = snap.nodes.get("region").expect("region in snapshot");

        assert_eq!(
            region.regions_total,
            Some(2.0),
            "a NaN regions count must fall back to the wired edge count, not propagate NaN"
        );
        assert_eq!(
            region.active_region,
            Some(0.0),
            "a NaN activeRegion must fall back to region 0, not stay unresolved"
        );
        let client_completed = snap.nodes.get("client").map(|c| c.total_completed).unwrap_or(0.0);
        assert!(client_completed > 0.0, "the region must still be routing traffic through the NaN config");
    }
}
