//! Data-tier component behaviours: replicated reads (`replica`), and
//! horizontal sharding (`shard`). Port of `src/sim/behaviour-data.ts`.
//!
//! Both are pure registry entries -- they add no branch to the event loop.
//! Each keeps whatever internal structure it needs on `state.ext`, which
//! the engine allocates and then never looks at, and drives service
//! through `ctx` so its timing, error rolls and timeouts are the same ones
//! every other kind gets.
//!
//! ## A Rust-specific wrinkle: accessing `ext` from inside `on_drained`
//!
//! `ctx.serve_within`'s completion callback is `Box<dyn FnOnce(&mut dyn
//! BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike)>` -- it does not receive
//! `ext`, and `NodeStateLike` deliberately does not expose it either (see
//! `engine_types.rs`: "a behaviour that needs it will get it through a
//! different mechanism the `sim::engine` implementer designs"). In TS this
//! is a non-issue: `onReplicaDrained`/`onShardDrained` just call
//! `replicaExt(state)`/`shardExt(state)` again, because `state.ext` is a
//! plain property read at call time.
//!
//! `Box<dyn FnOnce(..)>` defaults to `'static`, so the callback cannot
//! borrow the `&mut Ext` a hook receives -- that borrow does not outlive
//! the hook call. The fix used throughout this file: `init_state` stores
//! `Arc<Mutex<ReplicaExt>>` / `Arc<Mutex<ShardExt>>` inside `Ext` rather
//! than a bare `Box<ReplicaExt>` / `Box<ShardExt>`. Every hook clones the
//! `Arc` (a cheap refcount bump) and the `on_drained` closure moves its own
//! clone in, giving it an owned, `'static`, independently-lockable handle
//! to the same data. `Ext`'s `Send + Sync` bound already requires exactly
//! this shape to be legal, so no change to `engine_types.rs` is needed.
//!
//! ## Holding a request across a queueing delay: `ReqHandle`
//!
//! `replica` and `shard` both queue requests they cannot admit immediately
//! (backpressure), then re-offer them to the engine once a slot frees --
//! mirroring the TS `ext.readQueue`/`ext.writeQueue` and per-shard
//! `ext.queues[i]`, which hold the actual `Req` object reference. An
//! earlier version of this file worked around Rust's lack of an owned,
//! storable `ReqLike` by capturing field VALUES into a synthetic
//! `QueuedReq` and handing that to `serve_within` -- which was flagged as
//! unsound, because it would re-derive a fresh detached request instead of
//! resolving the engine's real pooled one, silently orphaning the original
//! caller. `engine_types.rs` now closes that gap directly: `ctx.handle_of`
//! mints a `ReqHandle` (cheap, `Copy`, opaque) for a request the behaviour
//! currently holds a live view of, and `ctx.serve_deferred` redeems that
//! handle later against the engine's real pooled request -- preserving its
//! parent chain -- instead of a synthesized stand-in. `add_service_delay`
//! has the equivalent `add_service_delay_deferred`.
//!
//! `ReqHandle` carries no accessors, so a behaviour that needs to read a
//! queued request's fields again before redeeming it (here: just `key`,
//! for the staleness/partition arithmetic) keeps that one field alongside
//! the handle rather than a full synthesized `ReqLike`. The small
//! `Dispatch` enum below is this file's single seam between "a request we
//! still hold a live view of" (the common case: admit it, serve it within
//! the same hook call) and "a request captured earlier via `handle_of`"
//! (the pump path), so `start_replica_service`/`start_shard_service` do
//! not need two near-duplicate bodies.

use crate::sim::behaviour::{clamp01, AdmitAction, ComponentBehaviour, Ext, InstanceModel, PumpMode, ScaleField};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqHandle, ReqLike};
use crate::sim::types::{NodeKind, NodeStats};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Keyspace the engine draws request keys from; mirrored here for sizing.
const KEYSPACE: usize = 64;

/// Fleet ceilings, each the maximum the inspector already offers for the
/// field, so nothing a design built through the UI is affected by them.
///
/// WHY: `behaviour-data.ts:21-28` (upstream `0820b8c0`, PR #60). None of
/// `shardCount`/`replicaCount`/`shardCapacity` is among the nine config
/// numbers the web's `isTopology` validates, so a shared link, a
/// `.breakscale` file or a restored session can hand the engine a count
/// that sizes the `Vec`s below. `NaN as i64` already saturates to 0 in Rust
/// (so a non-numeric count merely clamped to `min`, no panic) but
/// `f64::INFINITY as i64` saturates to `i64::MAX`, and a billion-shard
/// count is itself already past anything a real allocator should attempt --
/// both are a hole an upper bound closes.
const MAX_SHARDS: i64 = 64;
const MAX_REPLICAS: i64 = 64;
const MAX_SHARD_CAPACITY: i64 = 512;

/// A count from config: floored, held at `min`, capped at `max`.
fn clamp_int(v: f64, min: i64, max: i64) -> i64 {
    if !v.is_finite() {
        return min;
    }
    let n = v.floor() as i64;
    if n < min {
        min
    } else if n > max {
        max
    } else {
        n
    }
}

/// A request this file is about to hand to the engine for service: either
/// one it still holds a live `&dyn ReqLike` view of (admitted and served
/// within the same hook call), or one captured earlier via `ctx.handle_of`
/// and held in `ext` while it waited in a backlog for a free slot.
///
/// The `key` field on `Deferred` is the one piece of a queued request this
/// file ever needs to read again before redeeming its handle (staleness
/// lookup for a read, partition re-homing on a shard resize); everything
/// else a queued request might carry is irrelevant to what happens when it
/// is finally served.
enum Dispatch<'a> {
    Live(&'a dyn ReqLike),
    Deferred { handle: ReqHandle, key: u32 },
}

impl<'a> Dispatch<'a> {
    fn key(&self) -> u32 {
        match self {
            Dispatch::Live(r) => r.key(),
            Dispatch::Deferred { key, .. } => *key,
        }
    }

    /// Hand this request to the engine's real service timing, dispatching
    /// through whichever of `serve_within`/`serve_deferred` matches how
    /// this `Dispatch` was constructed.
    fn serve(
        self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    ) {
        match self {
            Dispatch::Live(r) => ctx.serve_within(state, r, on_drained),
            Dispatch::Deferred { handle, .. } => ctx.serve_deferred(state, handle, on_drained),
        }
    }
}

/// A queued request, waiting for a slot: a stable handle to redeem later
/// via `ctx.serve_deferred`, plus the one field (`key`) this file reads
/// again before redeeming it.
#[derive(Clone, Copy)]
struct QueuedHandle {
    handle: ReqHandle,
    key: u32,
}

/* ================================================================== *
 * replica -- a read replica set behind a primary
 * ================================================================== */

/// Per-node state for a replica set.
///
/// `visible_at[key]` is the simulated time at which the most recent write
/// to that key becomes visible on the replicas -- i.e. write completion
/// plus the replication lag. A read of that key before then is reading a
/// replica that has not caught up yet, which is precisely what a stale
/// read is. Storing the deadline rather than the write time means changing
/// the lag slider affects only writes issued after the change, which is
/// how a real system behaves.
struct ReplicaExt {
    /// Visibility deadline per key; `-Infinity` means "never written".
    visible_at: Vec<f64>,
    /// Read slots in use across the whole replica set.
    read_busy: i64,
    /// Write slots in use on the primary.
    write_busy: i64,
    /// Reads waiting for a replica slot.
    read_queue: VecDeque<QueuedHandle>,
    /// Writes waiting for a primary slot.
    write_queue: VecDeque<QueuedHandle>,
}

fn replica_ext(ext: &Ext) -> Arc<Mutex<ReplicaExt>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<ReplicaExt>>>())
        .expect("replica ext")
        .clone()
}

/// Total read slots: every replica serves reads in parallel.
fn read_capacity(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().capacity, 1, i64::MAX)
        * clamp_int(state.config().replica_count, 1, MAX_REPLICAS)
}

/// Write slots: the primary alone, which is why writes do not scale.
fn write_capacity(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().capacity, 1, i64::MAX)
}

pub struct ReplicaBehaviour;

impl ComponentBehaviour for ReplicaBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Replica
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn generates_load(&self) -> bool {
        false
    }
    fn pulls_from_queues(&self) -> bool {
        false
    }
    fn buffers_for_consumers(&self) -> bool {
        false
    }
    // Admission, queueing and slot release are all handled inside on_admit
    // and the drain callback, because reads and writes draw from two
    // different pools. The engine's single busy/waiting pair cannot
    // express that.
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
        true
    }
    // A replica set is a primary plus N read replicas, and drawing it as
    // one box hides the reason adding replicas does not fix a write
    // bottleneck.
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Custom)
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(ReplicaExt {
            visible_at: vec![f64::NEG_INFINITY; KEYSPACE],
            read_busy: 0,
            write_busy: 0,
            read_queue: VecDeque::new(),
            write_queue: VecDeque::new(),
        }))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = replica_ext(ext);

        // Classify read vs write ONCE, from a single RNG draw. A retry of
        // the same call re-draws, exactly as a real client re-issuing a
        // request would.
        let is_write = ctx.roll() >= clamp01(state.config().read_fraction);
        ctx.mark_write(req, is_write);

        let mut e = handle.lock().unwrap();
        if is_write {
            if e.write_busy < write_capacity(state) {
                drop(e);
                start_replica_service(ctx, state, &handle, Dispatch::Live(req), true);
                return AdmitAction::Handled;
            }
            // Reads and writes share one configured backlog; a full queue
            // sheds.
            if e.write_queue.len() as f64 >= ctx.effective_queue_limit(state) {
                return AdmitAction::Shed;
            }
            let h = ctx.handle_of(req);
            e.write_queue.push_back(QueuedHandle { handle: h, key: req.key() });
        } else {
            if e.read_busy < read_capacity(state) {
                drop(e);
                start_replica_service(ctx, state, &handle, Dispatch::Live(req), false);
                return AdmitAction::Handled;
            }
            if e.read_queue.len() as f64 >= ctx.effective_queue_limit(state) {
                return AdmitAction::Shed;
            }
            let h = ctx.handle_of(req);
            e.read_queue.push_back(QueuedHandle { handle: h, key: req.key() });
        }
        AdmitAction::Handled
    }

    /// Unit 0 is the PRIMARY and reports the write pool's utilisation;
    /// units 1..replica_count are the read replicas.
    ///
    /// The read replicas all report the same number, and that is the
    /// honest reading rather than a shortcut: the engine pools read slots
    /// across the whole set, so a read is served by whichever replica is
    /// free and no individual replica has a utilisation of its own.
    /// Splitting the pool's load evenly across the drawn units says
    /// exactly that. What the vector DOES separate -- the primary from the
    /// read set -- is the distinction that carries the lesson, because it
    /// is where the two pools genuinely differ.
    fn report_instances(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut Ext) {
        let handle = replica_ext(ext);
        let e = handle.lock().unwrap();
        let replicas = clamp_int(state.config().replica_count, 1, MAX_REPLICAS) as usize;
        let mut out = vec![0.0f64; replicas + 1];

        let write_cap = write_capacity(state);
        out[0] = if write_cap > 0 {
            (e.write_busy as f64 / write_cap as f64).min(1.0)
        } else {
            0.0
        };

        let read_cap = read_capacity(state);
        let read_util = if read_cap > 0 {
            (e.read_busy as f64 / read_cap as f64).min(1.0)
        } else {
            0.0
        };
        for slot in out.iter_mut().skip(1) {
            *slot = read_util;
        }
        drop(e);

        ctx.report_instances(state, &out, 0.0);
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        // Publish occupancy on the engine's clock. Reads and writes draw
        // from two different pools held in `ext`, so `state.busy` stays 0
        // and the engine would otherwise integrate this node's utilisation
        // as permanently idle -- which does not merely blank the meter, it
        // tells an autoscaler watching this node to scale it DOWN while
        // its queue is overflowing.
        let handle = replica_ext(ext);
        let e = handle.lock().unwrap();
        let busy = e.read_busy + e.write_busy;
        drop(e);
        ctx.report_occupancy(
            state,
            busy as f64,
            (read_capacity(state) + write_capacity(state)) as f64,
        );
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let fresh = ctx.counter_rate(state, "freshRead");
        let stale = ctx.counter_rate(state, "staleRead");
        let total = fresh + stale;
        stats.stale_read_rate = if total > 0.0 { stale / total } else { 0.0 };

        // Occupancy is the two pools together against their combined
        // slots, so the meter still means "how full is this thing".
        let handle = replica_ext(ext);
        let e = handle.lock().unwrap();
        stats.queued = (e.read_queue.len() + e.write_queue.len()) as f64;
        stats.in_flight = (e.read_busy + e.write_busy) as f64;
    }
}

/// Begin serving one request against the replica set.
///
/// A write occupies a primary slot and, on completion, pushes this key's
/// visibility deadline out by the replication lag. A read occupies a
/// replica slot and is judged stale at the moment it is served.
fn start_replica_service(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    handle: &Arc<Mutex<ReplicaExt>>,
    dispatch: Dispatch,
    is_write: bool,
) {
    let key = (dispatch.key() as usize) % KEYSPACE;
    {
        let mut e = handle.lock().unwrap();

        if is_write {
            e.write_busy += 1;
        } else {
            e.read_busy += 1;
            // Judge staleness at service time: the replica answers from
            // whatever it holds right now. `visible_at` is the write's
            // arrival deadline on the replicas, so "not yet visible" is
            // exactly a stale read.
            let stale = ctx.now() < e.visible_at[key];
            ctx.count_custom(state, if stale { "staleRead" } else { "freshRead" }, 1.0);
        }

        if is_write {
            // Claim the key's visibility deadline BEFORE starting service.
            // A zero-millisecond service time completes synchronously
            // inside serve_within/serve_deferred, so anything written
            // afterwards would be set after the request had already
            // drained. The deadline is a lower bound here (it ignores
            // this write's own service time) and is pushed to its true
            // value when the write actually lands, in
            // `on_replica_drained`.
            let lag = state.config().replication_lag_ms;
            let visible = ctx.now() + if lag > 0.0 { lag } else { 0.0 };
            if visible > e.visible_at[key] {
                e.visible_at[key] = visible;
            }
        }
    }

    let cb_handle = handle.clone();
    dispatch.serve(
        ctx,
        state,
        Box::new(move |ctx, state, req| on_replica_drained(ctx, state, req, &cb_handle)),
    );
}

/// A replica-set request finished its service time: free its slot, pull
/// next.
fn on_replica_drained(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    req: &dyn ReqLike,
    handle: &Arc<Mutex<ReplicaExt>>,
) {
    let key = (req.key() as usize) % KEYSPACE;
    let is_write = req.is_write();
    {
        let mut e = handle.lock().unwrap();
        if is_write {
            if e.write_busy > 0 {
                e.write_busy -= 1;
            }
            // The write has now committed on the primary. Replication
            // starts from here, so this is the moment the real visibility
            // deadline is known.
            let lag = state.config().replication_lag_ms;
            let visible = ctx.now() + if lag > 0.0 { lag } else { 0.0 };
            if visible > e.visible_at[key] {
                e.visible_at[key] = visible;
            }
        } else if e.read_busy > 0 {
            e.read_busy -= 1;
        }
    }
    pump_replica(ctx, state, handle);
}

/// Start whatever is waiting, up to each pool's own capacity.
fn pump_replica(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, handle: &Arc<Mutex<ReplicaExt>>) {
    loop {
        let next = {
            let mut e = handle.lock().unwrap();
            if e.write_busy < write_capacity(state) {
                e.write_queue.pop_front()
            } else {
                None
            }
        };
        match next {
            Some(q) => start_replica_service(ctx, state, handle, Dispatch::Deferred { handle: q.handle, key: q.key }, true),
            None => break,
        }
    }
    loop {
        let next = {
            let mut e = handle.lock().unwrap();
            if e.read_busy < read_capacity(state) {
                e.read_queue.pop_front()
            } else {
                None
            }
        };
        match next {
            Some(q) => start_replica_service(ctx, state, handle, Dispatch::Deferred { handle: q.handle, key: q.key }, false),
            None => break,
        }
    }
}

/* ================================================================== *
 * shard -- horizontal partitioning
 * ================================================================== */

/// Per-node state for a sharded store: `shard_count` independent
/// queue-and-servers units. They are deliberately NOT one shared pool -- a
/// request for a key can only be served by the shard that owns that key,
/// and that constraint is the entire lesson of the component.
struct ShardExt {
    /// Busy slots per shard.
    busy: Vec<i64>,
    /// Waiting requests per shard.
    queues: Vec<VecDeque<QueuedHandle>>,
    /// Integrated busy-slot-ms per shard, for utilisation.
    utilization: Vec<f64>,
    /// Shard count the vectors were sized for; a resize rebuilds them.
    sized: usize,
    /// Last time utilisation was integrated.
    last_integrate_ms: f64,
}

fn shard_ext(ext: &Ext) -> Arc<Mutex<ShardExt>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<ShardExt>>>())
        .expect("shard ext")
        .clone()
}

fn make_shard_ext(count: usize) -> ShardExt {
    ShardExt {
        busy: vec![0; count],
        queues: (0..count).map(|_| VecDeque::new()).collect(),
        utilization: vec![0.0; count],
        sized: count,
        last_integrate_ms: 0.0,
    }
}

/// Resize in place when the student moves the shard-count slider mid-run.
/// Requests already queued on a shard that no longer exists are re-homed
/// onto the shard their key now maps to, rather than being silently
/// dropped.
fn ensure_sized(state: &dyn NodeStateLike, e: &mut ShardExt) {
    let want = clamp_int(state.config().shard_count, 1, MAX_SHARDS) as usize;
    if e.sized == want {
        return;
    }

    let mut orphans: Vec<QueuedHandle> = Vec::new();
    for q in e.queues.iter_mut() {
        orphans.extend(q.drain(..));
    }

    let mut next = make_shard_ext(want);
    next.last_integrate_ms = e.last_integrate_ms;
    // Busy slots belong to requests already in service; their drain
    // callback will decrement whichever shard they were charged to (and
    // checks it still exists), so carry the counts across for the shards
    // that still exist.
    for i in 0..e.sized.min(want) {
        next.busy[i] = e.busy[i];
    }
    for q in orphans {
        let idx = (q.key as usize) % want;
        next.queues[idx].push_back(q);
    }

    *e = next;
}

/// Which shard owns this request: the hot key overrides the natural
/// mapping.
fn shard_index_for(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, count: usize) -> usize {
    let hot = clamp01(state.config().hot_key_fraction);
    // A single RNG draw decides whether this request is part of the
    // hot-key traffic. Taken unconditionally so the draw sequence -- and
    // therefore the replay -- does not depend on the slider's value.
    let roll = ctx.roll();
    if hot > 0.0 && roll < hot {
        return 0;
    }
    let count_i = count as i64;
    let key = req.key() as i64;
    (((key % count_i) + count_i) % count_i) as usize
}

pub struct ShardBehaviour;

impl ComponentBehaviour for ShardBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Shard
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn generates_load(&self) -> bool {
        false
    }
    fn pulls_from_queues(&self) -> bool {
        false
    }
    fn buffers_for_consumers(&self) -> bool {
        false
    }
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
        true
    }
    // A shard serves from shard_capacity (per partition), never from
    // `capacity`. Without this an autoscaler pointed here writes a field
    // this kind ignores.
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::ShardCapacity)
    }
    // One unit per partition. This is the kind the instance model exists
    // for: the per-partition numbers are genuinely independent, and the
    // gap between one pinned at 1.0 and the rest near idle IS the sharding
    // lesson.
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Custom)
    }

    fn init_state(&self, state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(make_shard_ext(
            clamp_int(state.config().shard_count, 1, MAX_SHARDS) as usize,
        )))))
    }

    /// Unit i is partition i, reporting that partition's own smoothed
    /// utilisation -- the same vector already surfaced as
    /// `shard_utilization`, published here through the general instance
    /// channel so a consumer can draw any partitioned kind without
    /// knowing the word "shard".
    fn report_instances(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut Ext) {
        let handle = shard_ext(ext);
        let mut e = handle.lock().unwrap();
        ensure_sized(state, &mut e);
        let out = e.utilization.clone();
        drop(e);
        ctx.report_instances(state, &out, 0.0);
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = shard_ext(ext);
        let count = {
            let mut e = handle.lock().unwrap();
            ensure_sized(state, &mut e);
            e.sized
        };
        let idx = shard_index_for(ctx, state, req, count);
        let capacity = clamp_int(state.config().shard_capacity, 1, MAX_SHARD_CAPACITY);

        let mut e = handle.lock().unwrap();
        if e.busy[idx] < capacity {
            drop(e);
            start_shard_service(ctx, state, &handle, idx, Dispatch::Live(req));
            return AdmitAction::Handled;
        }

        // Each shard has its own backlog. One shard filling up must not
        // steal the headroom of the others, or the hot-key demo would show
        // sheds everywhere instead of on the shard that is actually
        // melting.
        if e.queues[idx].len() as f64 >= ctx.effective_queue_limit(state) {
            return AdmitAction::Shed;
        }
        let h = ctx.handle_of(req);
        e.queues[idx].push_back(QueuedHandle { handle: h, key: req.key() });
        AdmitAction::Handled
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        // Integrate per-shard occupancy on the engine's own clock rather
        // than per event, so an idle shard's utilisation still decays
        // toward zero.
        let handle = shard_ext(ext);
        let mut e = handle.lock().unwrap();
        ensure_sized(state, &mut e);
        let dt = ctx.now() - e.last_integrate_ms;
        e.last_integrate_ms = ctx.now();
        if dt <= 0.0 {
            return;
        }
        let capacity = clamp_int(state.config().shard_capacity, 1, MAX_SHARD_CAPACITY);
        let alpha = 1.0 - (-dt / 500.0).exp();
        let mut busy_total = 0i64;
        for i in 0..e.sized {
            let instant = (e.busy[i] as f64 / capacity as f64).min(1.0);
            e.utilization[i] += (instant - e.utilization[i]) * alpha;
            busy_total += e.busy[i];
        }
        let sized = e.sized;
        drop(e);
        // Slots live per-shard in `ext`, so `state.busy` is 0 and the
        // engine would integrate this node as idle. Report the real
        // total, or an autoscaler watching a melting sharded store reads
        // 0.0 and scales it down.
        ctx.report_occupancy(state, busy_total as f64, (sized as i64 * capacity) as f64);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = shard_ext(ext);
        let mut e = handle.lock().unwrap();
        ensure_sized(state, &mut e);
        let n = e.sized;
        let mut max = 0.0f64;
        let mut min = f64::INFINITY;
        let mut sum = 0.0f64;
        let mut busy = 0i64;
        let mut queued = 0i64;
        for i in 0..n {
            let u = e.utilization[i];
            if u > max {
                max = u;
            }
            if u < min {
                min = u;
            }
            sum += u;
            busy += e.busy[i];
            queued += e.queues[i].len() as i64;
        }
        let report = e.utilization[..n].to_vec();
        drop(e);

        stats.max_shard_utilization = max;
        stats.min_shard_utilization = if min.is_infinite() { 0.0 } else { min };
        // The node-level meter is the mean across shards -- which is
        // exactly the number that looks healthy while one shard is pinned
        // at 1.0.
        stats.utilization = if n > 0 { sum / n as f64 } else { 0.0 };
        stats.in_flight = busy as f64;
        stats.queued = queued as f64;
        ctx.report_shard_utilization(state, &report);
        stats.shard_utilization = report;
    }
}

fn start_shard_service(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    handle: &Arc<Mutex<ShardExt>>,
    idx: usize,
    dispatch: Dispatch,
) {
    {
        let mut e = handle.lock().unwrap();
        e.busy[idx] += 1;
    }
    // The TS source tracks "which shard is this request charged to" with a
    // `WeakMap<ReqLike, number>` (`shardOf`), because a plain closure could
    // not otherwise recover `idx` when the drain fires. Rust closures own
    // their captures, so `idx` is simply moved into `on_drained` directly
    // -- no identity map needed.
    let cb_handle = handle.clone();
    dispatch.serve(
        ctx,
        state,
        Box::new(move |ctx, state, req| on_shard_drained(ctx, state, req, &cb_handle, idx)),
    );
}

fn on_shard_drained(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    _req: &dyn ReqLike,
    handle: &Arc<Mutex<ShardExt>>,
    idx: usize,
) {
    {
        let mut e = handle.lock().unwrap();
        if idx >= e.sized {
            return;
        }
        if e.busy[idx] > 0 {
            e.busy[idx] -= 1;
        }
    }

    let capacity = clamp_int(state.config().shard_capacity, 1, MAX_SHARD_CAPACITY);
    loop {
        let popped = {
            let mut e = handle.lock().unwrap();
            if idx >= e.sized || e.busy[idx] >= capacity {
                None
            } else {
                match e.queues[idx].pop_front() {
                    Some(next) => {
                        // Reserve the slot before releasing the lock, so
                        // this cannot race a concurrent admission for the
                        // same shard past capacity.
                        e.busy[idx] += 1;
                        Some(next)
                    }
                    None => None,
                }
            }
        };
        match popped {
            Some(q) => {
                let cb_handle = handle.clone();
                // The slot is already reserved above; drive the engine's
                // real service timing without incrementing `busy` a
                // second time (unlike the `on_admit` fast path, which
                // calls `start_shard_service` and reserves it there).
                Dispatch::Deferred { handle: q.handle, key: q.key }.serve(
                    ctx,
                    state,
                    Box::new(move |ctx, state, req| on_shard_drained(ctx, state, req, &cb_handle, idx)),
                );
            }
            None => break,
        }
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static REPLICA: ReplicaBehaviour = ReplicaBehaviour;
static SHARD: ShardBehaviour = ShardBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::Replica, &REPLICA),
    (NodeKind::Shard, &SHARD),
];

#[cfg(test)]
mod tests {
    use super::clamp_int;
    use crate::sim::engine::Engine;
    use crate::sim::presets::default_config;
    use crate::sim::types::{NodeConfig, NodeKind, NodeStats, SimEdge, SimNode, Topology};

    /// WHY: `behaviour-data.ts:21-34` (upstream `0820b8c0`, PR #60). Pure
    /// unit coverage of the helper itself: non-finite falls to `min`, and a
    /// value above `max` is held there rather than passed through.
    #[test]
    fn clamp_int_guards_non_finite_and_caps_the_maximum() {
        assert_eq!(clamp_int(f64::NAN, 1, 64), 1);
        assert_eq!(clamp_int(f64::INFINITY, 1, 64), 64);
        assert_eq!(clamp_int(f64::NEG_INFINITY, 1, 64), 1);
        assert_eq!(clamp_int(1e9, 1, 64), 64);
        assert_eq!(clamp_int(8.9, 1, 64), 8);
        assert_eq!(clamp_int(0.0, 1, 64), 1);
    }

    fn node(id: &str, kind: NodeKind, config: NodeConfig) -> SimNode {
        SimNode { id: id.to_string(), kind, label: id.to_string(), x: 0.0, y: 0.0, config }
    }

    /// A client feeding one `kind` node, patched with whatever field the
    /// test wants to try breaking. Mirrors
    /// `behaviour-data.bounds.test.ts` (upstream `0820b8c0`, PR #60).
    fn topology(kind: NodeKind, patch: impl FnOnce(&mut NodeConfig)) -> Topology {
        let client_cfg = NodeConfig { rps: 60.0, ..default_config(NodeKind::Client) };
        let mut target_cfg = default_config(kind);
        patch(&mut target_cfg);
        Topology {
            nodes: vec![node("client", NodeKind::Client, client_cfg), node("target", kind, target_cfg)],
            edges: vec![SimEdge {
                id: "e1".into(),
                from: "client".into(),
                to: "target".into(),
                weight: 1.0,
                control: None,
                latency_ms: None,
                bandwidth_rps: None,
                loss_rate: None,
            }],
            annotations: None,
        }
    }

    fn stats_for(kind: NodeKind, patch: impl FnOnce(&mut NodeConfig)) -> NodeStats {
        let mut engine = Engine::new(topology(kind, patch), 7);
        for _ in 0..60 {
            engine.advance(1000.0 / 60.0);
        }
        engine.snapshot().nodes.remove("target").expect("target node")
    }

    /// WHY: `behaviour-data.ts:21-34` (upstream `0820b8c0`, PR #60). One
    /// `Vec` per shard is sized from `shard_count`; before the fix a huge or
    /// non-finite count sized them unbounded (or, for `NaN`, merely
    /// happened not to panic in Rust -- see the WHY-comment on
    /// `clamp_int`). The engine must run to completion either way and must
    /// never publish a NaN utilisation.
    #[test]
    fn shard_count_is_bounded_for_huge_or_non_finite_values() {
        for shard_count in [f64::NAN, f64::INFINITY, 1e9] {
            let stats = stats_for(NodeKind::Shard, |c| c.shard_count = shard_count);
            assert!(
                stats.shard_utilization.len() <= 64,
                "shard_count={shard_count} produced {} shards",
                stats.shard_utilization.len()
            );
            assert!(!stats.utilization.is_nan(), "shard_count={shard_count} produced a NaN utilization");
            for u in &stats.shard_utilization {
                assert!(!u.is_nan(), "shard_count={shard_count} produced a NaN per-shard utilization");
            }
        }
    }

    /// WHY: same upstream fix, for the replica set's read pool.
    #[test]
    fn replica_count_is_bounded_for_huge_or_non_finite_values() {
        for replica_count in [f64::NAN, f64::INFINITY, 1e9] {
            let stats = stats_for(NodeKind::Replica, |c| c.replica_count = replica_count);
            let units = stats.per_instance.as_ref().map(|v| v.len()).unwrap_or(0);
            assert!(units <= 65, "replica_count={replica_count} produced {units} units");
        }
    }

    /// The ceilings are the Inspector's own maxima, so nothing a reader can
    /// build through the UI moves: eight shards are still eight shards.
    #[test]
    fn a_shard_count_the_inspector_can_set_is_unaffected() {
        let stats = stats_for(NodeKind::Shard, |c| c.shard_count = 8.0);
        assert_eq!(stats.shard_utilization.len(), 8);
    }
}
