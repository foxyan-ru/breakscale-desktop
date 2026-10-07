//! Storage behaviours: relational database, object storage, search index,
//! time-series store, graph database, cold storage, vector database. Port
//! of `src/sim/behaviour-store.ts`.
//!
//! All seven are pure registry entries -- no branch is added to the event
//! loop. Each one carries a mechanism no other kind has, because a store
//! whose only identity is its default sliders is a store the student can
//! rightly call a re-skinned service:
//!
//!   db           write lock contention (writes serialise against each other)
//!   objectstore  per-prefix rate ceilings and no queue (S3 SlowDown)
//!   searchindex  asynchronous indexing (a commit is not yet searchable)
//!   timeseriesdb two-class cost (cheap appends, expensive range queries)
//!   graphdb      traversal cost compounds with depth
//!   coldstorage  restore JOBS: flat multi-second latency, a slot quota,
//!                and no queue at all
//!   vectordb     recall target multiplies scan cost hyperbolically
//!
//! Five of them (`db`, `searchindex`, `timeseriesdb`, `graphdb`,
//! `vectordb`) share one piece of machinery in the TS source: a
//! self-managed slot pool whose per-request service cost is decided by
//! the kind. `behaviour-store.ts` factors this as a family of
//! higher-order functions (`makeDrained`, `poolAdmit`, ...) parameterised
//! by per-kind `classify`/`onStart`/`onServed` closures, relying on
//! `DbExt`/`SearchExt` structurally extending the shared `PoolExt` shape.
//!
//! ## Judgement call: the shared pool as a Rust generic, not a trait-object
//!
//! Rust has no structural interface extension, and the `ComponentBehaviour`
//! trait must stay object-safe, so the TS factoring cannot be carried over
//! verbatim. This file instead defines `Pool<X>` (`busy`, `queue`, and a
//! kind-specific `extra: X` -- `DbExtra { writers }`, `SearchExtra
//! { visible_at }`, or `()` for the three kinds with no extra state) and a
//! small set of GENERIC free functions (`pool_admit`, `start_pool_service`,
//! `pool_drained`, `report_pool`, `decorate_pool`, `mean_served_ms`)
//! parameterised over `X`. Each kind's `classify`/`on_start`/`on_served`
//! are plain `fn` items (never closures capturing environment -- exactly
//! what the TS versions are too, once you notice none of them close over
//! anything but `ctx`/`state`/`req`/the pool handle), so they satisfy the
//! `Copy + 'static` a shared `PoolHooks<X>` needs. This reproduces the TS
//! architecture's actual shape (shared mechanism, per-kind policy
//! injected as functions) without fighting the trait-object boundary.
//!
//! Same `Arc<Mutex<_>>` ext pattern as `data.rs` and `messaging.rs::lambda`
//! (see `data.rs`'s module doc): `ctx.serve_within`/`ctx.serve_deferred`'s
//! `on_drained` callback needs to reach back into the pool's
//! `busy`/`queue`/`extra` from a `'static` closure, which plain `&mut Ext`
//! cannot provide.
//!
//! ## Holding a queued request: `ReqHandle`, not a synthesized `ReqLike`
//!
//! `pool_admit`'s queueing path used to capture a request's field VALUES
//! into a synthetic `QueuedReq` and hand that to `serve_within` once a
//! slot freed -- flagged as unsound, because re-deriving a fresh detached
//! request from remembered values rather than resolving the engine's real
//! pooled one would silently orphan the original caller. `engine_types.rs`
//! now provides `ctx.handle_of`/`ctx.serve_deferred`/
//! `ctx.add_service_delay_deferred` for exactly this (see `data.rs`'s
//! module doc for the full explanation, not repeated per file). This file
//! uses the same small `Dispatch` seam `data.rs` does: `start_pool_service`
//! takes a `Dispatch` (a live `&dyn ReqLike` for the common "admit and
//! serve within the same hook call" path, or a `ReqHandle` plus the one
//! field -- `key`/`is_write` -- a pool's `on_start` hooks still need to
//! read before redeeming it, for the deferred pump path), so `on_start`
//! hooks (`db_start`, `search_start`, `tsdb_start`) take `&Dispatch`
//! instead of `&dyn ReqLike`. `classify` and `on_served` are unaffected:
//! `classify` only ever runs once, on a freshly admitted live request
//! (never replayed from the queue), and `on_served`/`on_drained` always
//! receives a fresh, live `&dyn ReqLike` view from the engine regardless
//! of which path served the request, per `serve_within`/`serve_deferred`'s
//! shared contract.
//!
//! One simplification versus the TS source: `searchindex` declares its
//! staleness check TWICE in TS (`searchStart`, and an structurally
//! identical inline closure inside `makeDrained`'s `onStart` argument),
//! because `poolAdmit`'s own `onStart` and `makeDrained`'s pump-time
//! `onStart` are separate closures there. This file threads one
//! `PoolHooks<X>` value through both the initial admission and every
//! later pump dispatch, so `search_start` is defined once and reused for
//! both, with no behavioural difference.
//!
//! Determinism: every RNG draw here happens in `classify`, once per
//! admitted request, unconditionally where a roll exists at all -- the
//! same discipline `data.rs` follows, so replay never depends on a
//! slider's value.

use crate::sim::behaviour::{clamp01, AdmitAction, ComponentBehaviour, Ext, InstanceModel, PumpMode, ScaleField};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqHandle, ReqLike};
use crate::sim::types::{FailureReason, NodeKind, NodeStats};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Keyspace the engine draws request keys from; mirrored here for sizing.
const KEYSPACE: usize = 64;

/// `num(v, fallback)` from the TS source, for a field whose Rust port
/// keeps it a plain (always-present) `f64` rather than `Option<f64>` --
/// `service_ms`, `read_fraction`. The TS helper guarded every numeric
/// config read uniformly against `undefined` AND `NaN`; the Rust port has
/// no `undefined` case for these fields, so only the NaN/finite guard
/// carries over, kept for parity with the source's defensiveness.
fn num(v: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// `num(v, fallback)` for a genuinely `Option<f64>` field.
fn num_opt(v: Option<f64>, fallback: f64) -> f64 {
    match v {
        Some(x) if x.is_finite() => x,
        _ => fallback,
    }
}

/// A request this file is about to hand to the engine for service: either
/// one it still holds a live `&dyn ReqLike` view of (admitted and served
/// within the same hook call), or one captured earlier via `ctx.handle_of`
/// and held in a pool's queue while it waited for a free slot. See this
/// module's doc comment ("Holding a queued request") and `data.rs`'s for
/// why this exists.
enum Dispatch<'a> {
    Live(&'a dyn ReqLike),
    Deferred { handle: ReqHandle, key: u32, is_write: bool },
}

impl<'a> Dispatch<'a> {
    fn key(&self) -> u32 {
        match self {
            Dispatch::Live(r) => r.key(),
            Dispatch::Deferred { key, .. } => *key,
        }
    }

    fn is_write(&self) -> bool {
        match self {
            Dispatch::Live(r) => r.is_write(),
            Dispatch::Deferred { is_write, .. } => *is_write,
        }
    }

    fn add_service_delay(&self, ctx: &mut dyn BehaviourCtx, extra_ms: f64) {
        match self {
            Dispatch::Live(r) => ctx.add_service_delay(*r, extra_ms),
            Dispatch::Deferred { handle, .. } => ctx.add_service_delay_deferred(*handle, extra_ms),
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

/// A queued request, waiting for a pool slot: a stable handle to redeem
/// later via `ctx.serve_deferred`, plus the fields this file's `on_start`
/// hooks read again before redeeming it.
#[derive(Clone, Copy)]
struct QueuedHandle {
    handle: ReqHandle,
    key: u32,
    is_write: bool,
}

/* ================================================================== *
 * Shared costed pool
 *
 * One busy counter and one FIFO, exactly the engine's own discipline,
 * kept in `ext` because the per-request cost (added via add_service_delay
 * in classify) is only honoured by serve_within/serve_deferred, never by
 * the engine-managed generic slot path. Capacity and queue limit are read
 * through `ctx` so `instances`, sliders and floors all mean the same
 * thing they mean everywhere else.
 * ================================================================== */

struct Pool<X> {
    /// Slots in use across the whole pool.
    busy: i64,
    /// Waiting requests, FIFO.
    queue: VecDeque<QueuedHandle>,
    /// Kind-specific extra state (`DbExtra`, `SearchExtra`, or `()`).
    extra: X,
}

fn init_pool<X>(extra: X) -> Pool<X> {
    Pool {
        busy: 0,
        queue: VecDeque::new(),
        extra,
    }
}

/// `on_start` runs just before a request's service time begins -- given a
/// `Dispatch` rather than a live `&dyn ReqLike`, because the deferred/pump
/// path only has a redeemed `ReqHandle` plus remembered fields at this
/// point (see this module's doc comment).
type StartFn<X> = fn(&mut dyn BehaviourCtx, &dyn NodeStateLike, &Dispatch<'_>, &Arc<Mutex<Pool<X>>>);
/// `on_served` runs when a request's service time has elapsed (where a
/// write's commit takes effect) and `classify` runs once at admission.
/// Both always see a live, engine-supplied `&dyn ReqLike`: `on_served` is
/// only ever called from `on_drained`, which `serve_within`/
/// `serve_deferred` both hand a fresh live view; `classify` only ever runs
/// on a request that was just admitted, never on one replayed from the
/// queue.
type ReqFn<X> = fn(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike, &Arc<Mutex<Pool<X>>>);

/// Either hook may be absent.
#[derive(Clone, Copy)]
struct PoolHooks<X> {
    on_start: Option<StartFn<X>>,
    on_served: Option<ReqFn<X>>,
}

fn start_pool_service<X: Send + 'static>(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    handle: &Arc<Mutex<Pool<X>>>,
    dispatch: Dispatch,
    hooks: PoolHooks<X>,
) {
    {
        let mut p = handle.lock().unwrap();
        p.busy += 1;
    }
    if let Some(f) = hooks.on_start {
        f(ctx, state, &dispatch, handle);
    }
    let cb_handle = handle.clone();
    dispatch.serve(
        ctx,
        state,
        Box::new(move |ctx, state, req| pool_drained(ctx, state, req, &cb_handle, hooks)),
    );
}

fn pool_drained<X: Send + 'static>(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    req: &dyn ReqLike,
    handle: &Arc<Mutex<Pool<X>>>,
    hooks: PoolHooks<X>,
) {
    {
        let mut p = handle.lock().unwrap();
        if p.busy > 0 {
            p.busy -= 1;
        }
    }
    // Measured cost booking: `served` is queries, `servedMs` is the
    // actual milliseconds each one took (surcharges included), so a mean
    // cost readout is measurement rather than arithmetic on the config.
    ctx.count_custom(state, "served", 1.0);
    ctx.count_custom(state, "servedMs", req.own_ms());
    if let Some(f) = hooks.on_served {
        f(ctx, state, req, handle);
    }

    // Pump: start whatever is waiting, up to capacity. Requests that
    // resolved while queued are skipped without occupying a slot -- moot
    // here since `QueuedHandle` is only ever pushed at admission, but kept
    // as the same loop shape as the TS source for a future behaviour that
    // might drop entries.
    loop {
        let next = {
            let mut p = handle.lock().unwrap();
            if (p.busy as f64) < ctx.effective_capacity(state) {
                p.queue.pop_front()
            } else {
                None
            }
        };
        match next {
            Some(q) => start_pool_service(
                ctx,
                state,
                handle,
                Dispatch::Deferred { handle: q.handle, key: q.key, is_write: q.is_write },
                hooks,
            ),
            None => break,
        }
    }
}

/// The shared admission path: classify, then slot-or-queue-or-shed.
fn pool_admit<X: Send + 'static>(
    ctx: &mut dyn BehaviourCtx,
    state: &dyn NodeStateLike,
    req: &dyn ReqLike,
    handle: &Arc<Mutex<Pool<X>>>,
    classify: ReqFn<X>,
    hooks: PoolHooks<X>,
) -> AdmitAction {
    classify(ctx, state, req, handle);
    let busy = handle.lock().unwrap().busy;
    if (busy as f64) < ctx.effective_capacity(state) {
        start_pool_service(ctx, state, handle, Dispatch::Live(req), hooks);
        return AdmitAction::Handled;
    }
    let queue_len = handle.lock().unwrap().queue.len();
    if queue_len as f64 >= ctx.effective_queue_limit(state) {
        return AdmitAction::Shed;
    }
    let h = ctx.handle_of(req);
    let mut p = handle.lock().unwrap();
    p.queue.push_back(QueuedHandle { handle: h, key: req.key(), is_write: req.is_write() });
    AdmitAction::Handled
}

/// Occupancy publishing shared by every pooled kind (see `data.rs`'s
/// replica for why this is necessary: slots live in `ext`, so
/// `state.busy` alone would read permanently idle).
fn report_pool<X: Send + 'static>(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, handle: &Arc<Mutex<Pool<X>>>) {
    let busy = handle.lock().unwrap().busy;
    let cap = ctx.effective_capacity(state);
    ctx.report_occupancy(state, busy as f64, cap);
}

fn decorate_pool<X>(handle: &Arc<Mutex<Pool<X>>>, stats: &mut NodeStats) {
    let p = handle.lock().unwrap();
    stats.in_flight = p.busy as f64;
    stats.queued = p.queue.len() as f64;
}

/// Measured mean ms per served request over the window, or 0 when idle.
fn mean_served_ms(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike) -> f64 {
    let served = ctx.counter_rate(state, "served");
    if !(served > 0.0) {
        return 0.0;
    }
    let ms = ctx.counter_rate(state, "servedMs") / served;
    if ms.is_finite() && ms > 0.0 {
        ms
    } else {
        0.0
    }
}

/* ================================================================== *
 * db -- a relational store with write lock contention
 *
 * The one thing a database does that a stateless service cannot: it
 * guards shared data. Reads share the slot pool freely. A WRITE entering
 * service pays `lock_ms` of extra wait for every other write already in
 * flight -- row and page locks, stated as arithmetic -- so write latency
 * grows with write CONCURRENCY, not with load per se.
 *
 * The consequence this exists to teach: `instances` multiplies the slot
 * pool, so adding instances fixes a slow READ path -- but the lock is a
 * property of the data, shared across the whole fleet, so the write path
 * gets nothing. Watch `lock_wait_ms` stay put while a scale-up halves
 * read latency, and the case for read replicas (split the reads out) and
 * sharding (split the DATA, and with it the lock) writes itself.
 *
 * Classification draws one unconditional roll against `read_fraction`,
 * the same convention the replica set and search index follow, so replay
 * never depends on the slider's value.
 * ================================================================== */

struct DbExtra {
    /// Writes currently holding service slots.
    writers: i64,
}

type DbPool = Pool<DbExtra>;

fn db_ext(ext: &Ext) -> Arc<Mutex<DbPool>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<DbPool>>>())
        .expect("db ext")
        .clone()
}

fn db_classify(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, _handle: &Arc<Mutex<DbPool>>) {
    let is_write = ctx.roll() >= clamp01(num(state.config().read_fraction, 0.9));
    ctx.mark_write(req, is_write);
}

/// At service start: judge contention against the writers holding slots
/// at this moment (not at admission -- a write that waited in the queue
/// meets the lock as it actually is when its turn comes).
fn db_start(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, dispatch: &Dispatch<'_>, handle: &Arc<Mutex<DbPool>>) {
    ctx.count_custom(state, if dispatch.is_write() { "write" } else { "read" }, 1.0);
    if !dispatch.is_write() {
        return;
    }
    let lock_ms = num_opt(state.config().lock_ms, 0.0).max(0.0);
    let writers = handle.lock().unwrap().extra.writers;
    let wait = lock_ms * writers as f64;
    if wait > 0.0 {
        dispatch.add_service_delay(ctx, wait);
        ctx.count_custom(state, "lockWaitMs", wait);
    }
    handle.lock().unwrap().extra.writers += 1;
}

fn db_served(_ctx: &mut dyn BehaviourCtx, _state: &dyn NodeStateLike, req: &dyn ReqLike, handle: &Arc<Mutex<DbPool>>) {
    if !req.is_write() {
        return;
    }
    let mut p = handle.lock().unwrap();
    if p.extra.writers > 0 {
        p.extra.writers -= 1;
    }
}

pub struct DbBehaviour;

impl ComponentBehaviour for DbBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Db
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
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

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(init_pool(DbExtra { writers: 0 })))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = db_ext(ext);
        pool_admit(
            ctx,
            state,
            req,
            &handle,
            db_classify,
            PoolHooks { on_start: Some(db_start), on_served: Some(db_served) },
        )
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = db_ext(ext);
        report_pool(ctx, state, &handle);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = db_ext(ext);
        decorate_pool(&handle, stats);
        let writes = ctx.counter_rate(state, "write");
        stats.read_rate = Some(ctx.counter_rate(state, "read"));
        stats.write_rate = Some(writes);
        let waited = ctx.counter_rate(state, "lockWaitMs");
        let mean = if writes > 0.0 { waited / writes } else { 0.0 };
        stats.lock_wait_ms = Some(if mean.is_finite() && mean > 0.0 { mean } else { 0.0 });
    }
}

/* ================================================================== *
 * objectstore -- blob storage (S3-like)
 *
 * A wide flat-latency pool with one mechanism of its own: PER-PREFIX rate
 * ceilings. A blob store partitions its keyspace by prefix and each
 * partition sustains `prefix_rps` on its own; a request that finds its
 * prefix over the ceiling is refused instantly with a SlowDown
 * ('throttled'), while the rest of the store idles. That is the S3 503,
 * and it is why blob-store performance advice is about key LAYOUT --
 * spread your prefixes -- rather than about capacity. The gate consumes
 * zero randomness (a continuous token bucket per prefix, refilled against
 * simulated time exactly like the rate limiter); requests that pass it
 * are served on the engine's ordinary wide slot pool (`AdmitAction::Serve`,
 * not a self-managed pool -- objectstore has no per-request cost surcharge
 * to carry through `serve_within`).
 *
 * No instance model and no scale field: it is a managed service, one
 * logical thing an autoscaler has no business resizing -- which is itself
 * part of the lesson, because when a prefix melts there is no "add
 * servers" knob to reach for.
 * ================================================================== */

/// Fixed prefix count the 64-key keyspace folds onto. A constant rather
/// than a knob: the lesson lives in `prefix_rps` and in the key layout of
/// the traffic, not in how many prefixes exist.
const OBJECT_PREFIXES: usize = 8;

struct ObjectStoreExt {
    /// Token bucket level per prefix. Fractional: refill is continuous.
    tokens: Vec<f64>,
    /// Simulated time each prefix's bucket was last refilled.
    last_refill_ms: Vec<f64>,
    /// Ceiling the buckets were last capped against, to catch slider
    /// moves.
    last_rate: f64,
}

fn object_prefix_rate(state: &dyn NodeStateLike) -> f64 {
    match state.config().prefix_rps {
        Some(r) if r > 0.0 => r,
        _ => 0.0,
    }
}

/// Bring ONE prefix's bucket up to date with simulated time.
fn object_refill(ext: &mut ObjectStoreExt, prefix: usize, rate: f64, now: f64) {
    if rate != ext.last_rate {
        // The ceiling moved: re-cap every bucket rather than letting one
        // filled under the old ceiling stay oversized.
        for p in 0..OBJECT_PREFIXES {
            if ext.tokens[p] > rate {
                ext.tokens[p] = rate;
            }
        }
        ext.last_rate = rate;
    }
    let elapsed = now - ext.last_refill_ms[prefix];
    if elapsed <= 0.0 {
        return;
    }
    ext.last_refill_ms[prefix] = now;
    ext.tokens[prefix] += (elapsed / 1000.0) * rate;
    if ext.tokens[prefix] > rate {
        ext.tokens[prefix] = rate;
    }
}

pub struct ObjectStoreBehaviour;

impl ComponentBehaviour for ObjectStoreBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::ObjectStore
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
        PumpMode::Own
    }
    fn credits_join_completion(&self) -> bool {
        true
    }

    fn init_state(&self, state: &dyn NodeStateLike) -> Ext {
        let rate = object_prefix_rate(state);
        // Buckets start full, like every other token bucket in the app:
        // an idle store absorbs one burst per prefix before the ceiling
        // bites.
        let fill = if rate > 0.0 { rate } else { 0.0 };
        Some(Box::new(ObjectStoreExt {
            tokens: vec![fill; OBJECT_PREFIXES],
            last_refill_ms: vec![0.0; OBJECT_PREFIXES],
            last_rate: rate,
        }))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let rate = object_prefix_rate(state);
        // No ceiling configured: a plain wide pool, exactly the old
        // behaviour.
        if rate <= 0.0 {
            return AdmitAction::Serve;
        }

        let e = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<ObjectStoreExt>())
            .expect("objectstore ext");
        let prefix = {
            let k = req.key() as i64;
            let n = OBJECT_PREFIXES as i64;
            (((k % n) + n) % n) as usize
        };
        object_refill(e, prefix, rate, ctx.now());
        if e.tokens[prefix] < 1.0 {
            ctx.count_custom(state, "slowdown", 1.0);
            ctx.reject(state, req, FailureReason::Throttled);
            return AdmitAction::Handled;
        }
        e.tokens[prefix] -= 1.0;
        AdmitAction::Serve
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
        stats.slowdown_rate = Some(ctx.counter_rate(state, "slowdown"));
    }
}

/* ================================================================== *
 * coldstorage -- archival tier (Glacier-like)
 *
 * NOT a slow database, and the difference is the whole component. A slow
 * database queues: push online traffic at it and latency compounds as
 * the line grows. An archive runs restore JOBS: a retrieval takes its
 * flat multi-second time no matter how busy the vault is, there is NO
 * queue to stand in, and the vault runs at most `capacity` concurrent
 * restores (its provisioned retrieval capacity). A request that finds
 * every restore slot taken is refused on the spot ('throttled', the
 * archival tier's RequestLimitExceeded).
 *
 * So under overload the two kinds fail in opposite directions: the slow
 * db's p99 explodes while its error rate stays low; the archive's p99
 * stays pinned at the flat retrieval time while refusals climb. Both
 * teach "keep online traffic away", but only this one teaches it the way
 * Glacier actually does.
 *
 * NOTE: unlike the shared pool kinds above, coldstorage never queues -- a
 * vault has no line to wait in at all -- so it always has a live `req` in
 * hand when it calls `serve_within` and never needs `ReqHandle`/
 * `serve_deferred`.
 * ================================================================== */

struct ColdStorageExt {
    /// Restore jobs currently running.
    jobs: i64,
}

fn cold_ext(ext: &Ext) -> Arc<Mutex<ColdStorageExt>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<ColdStorageExt>>>())
        .expect("coldstorage ext")
        .clone()
}

fn cold_drained(_ctx: &mut dyn BehaviourCtx, _state: &dyn NodeStateLike, _req: &dyn ReqLike, handle: &Arc<Mutex<ColdStorageExt>>) {
    let mut e = handle.lock().unwrap();
    if e.jobs > 0 {
        e.jobs -= 1;
    }
}

pub struct ColdStorageBehaviour;

impl ComponentBehaviour for ColdStorageBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::ColdStorage
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
    // No queue and no engine slots: jobs are tracked in ext, timed by
    // serve_within, and the only admission outcomes are "job started" and
    // "refused". queue_limit is ignored, because a vault has no line to
    // wait in.
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
        true
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(ColdStorageExt { jobs: 0 }))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = cold_ext(ext);
        let started = {
            let mut e = handle.lock().unwrap();
            if (e.jobs as f64) >= ctx.effective_capacity(state) {
                false
            } else {
                e.jobs += 1;
                true
            }
        };
        if !started {
            ctx.count_custom(state, "restoreDenied", 1.0);
            ctx.reject(state, req, FailureReason::Throttled);
            return AdmitAction::Handled;
        }
        let cb_handle = handle.clone();
        ctx.serve_within(
            state,
            req,
            Box::new(move |ctx, state, req| cold_drained(ctx, state, req, &cb_handle)),
        );
        AdmitAction::Handled
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = cold_ext(ext);
        let jobs = handle.lock().unwrap().jobs;
        let cap = ctx.effective_capacity(state);
        ctx.report_occupancy(state, jobs as f64, cap);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let jobs = ext
            .as_ref()
            .and_then(|e| e.downcast_ref::<Arc<Mutex<ColdStorageExt>>>())
            .map(|h| h.lock().unwrap().jobs)
            .unwrap_or(0);
        stats.in_flight = jobs as f64;
        stats.queued = 0.0;
        stats.throttled_rate = Some(ctx.counter_rate(state, "restoreDenied"));
    }
}

/* ================================================================== *
 * searchindex -- a search cluster with asynchronous indexing
 *
 * A write is acknowledged when its (expensive) indexing work completes,
 * but the document only becomes SEARCHABLE `index_lag_ms` after that
 * commit, which is the refresh interval of a real search engine. A
 * search hitting a key inside that window is served from the old index
 * and counted as stale: that counter is the indexing lag made visible.
 * ================================================================== */

struct SearchExtra {
    /// Searchability deadline per key; `-Infinity` means "never written".
    visible_at: Vec<f64>,
}

type SearchPool = Pool<SearchExtra>;

fn search_ext(ext: &Ext) -> Arc<Mutex<SearchPool>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<SearchPool>>>())
        .expect("searchindex ext")
        .clone()
}

/// At service start: a SEARCH is judged against the index it is actually
/// reading right now. Judged here rather than at admission so a query
/// that waited in line long enough for the refresh to land counts fresh,
/// which is exactly how a real cluster behaves. Reused for both the
/// initial admission and every later pump dispatch (see this module's
/// doc comment on the one simplification versus the TS source).
fn search_start(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, dispatch: &Dispatch<'_>, handle: &Arc<Mutex<SearchPool>>) {
    if dispatch.is_write() {
        return;
    }
    let key = (dispatch.key() as usize) % KEYSPACE;
    let stale = {
        let p = handle.lock().unwrap();
        ctx.now() < p.extra.visible_at[key]
    };
    ctx.count_custom(state, if stale { "staleSearch" } else { "freshSearch" }, 1.0);
}

/// At service end: the write has committed, so the refresh clock starts
/// HERE. The admission-time claim in `search_classify` was only a lower
/// bound.
fn search_served(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, handle: &Arc<Mutex<SearchPool>>) {
    if !req.is_write() {
        return;
    }
    let lag = num_opt(state.config().index_lag_ms, 0.0).max(0.0);
    let key = (req.key() as usize) % KEYSPACE;
    let visible = ctx.now() + lag;
    let mut p = handle.lock().unwrap();
    if visible > p.extra.visible_at[key] {
        p.extra.visible_at[key] = visible;
    }
}

fn search_classify(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, handle: &Arc<Mutex<SearchPool>>) {
    // One unconditional draw decides search vs write, the same
    // convention the replica set uses for read vs write.
    let is_write = ctx.roll() >= clamp01(num(state.config().read_fraction, 0.9));
    ctx.mark_write(req, is_write);
    if is_write {
        // Writes pay the indexing surcharge on top of the drawn
        // service_ms. `classify` always runs on a live, freshly admitted
        // request, so `ctx.add_service_delay(req, ...)` (not the
        // `_deferred` variant) is correct here.
        ctx.add_service_delay(req, num_opt(state.config().index_ms, 0.0).max(0.0));
        ctx.count_custom(state, "indexWrite", 1.0);
        // Claim a lower-bound searchability deadline immediately: a
        // zero-ms service completes synchronously inside serve_within, so
        // waiting for the commit alone would let a same-instant search
        // read a write that has not even been acknowledged yet. Pushed
        // to its true value (commit time + lag) in `search_served`
        // above.
        let lag = num_opt(state.config().index_lag_ms, 0.0).max(0.0);
        let key = (req.key() as usize) % KEYSPACE;
        let visible = ctx.now() + lag;
        let mut p = handle.lock().unwrap();
        if visible > p.extra.visible_at[key] {
            p.extra.visible_at[key] = visible;
        }
    }
}

pub struct SearchIndexBehaviour;

impl ComponentBehaviour for SearchIndexBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::SearchIndex
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
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

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(init_pool(SearchExtra {
            visible_at: vec![f64::NEG_INFINITY; KEYSPACE],
        })))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = search_ext(ext);
        pool_admit(
            ctx,
            state,
            req,
            &handle,
            search_classify,
            PoolHooks { on_start: Some(search_start), on_served: Some(search_served) },
        )
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = search_ext(ext);
        report_pool(ctx, state, &handle);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = search_ext(ext);
        decorate_pool(&handle, stats);
        let fresh = ctx.counter_rate(state, "freshSearch");
        let stale = ctx.counter_rate(state, "staleSearch");
        let total = fresh + stale;
        stats.search_rate = Some(total);
        stats.stale_search_rate = Some(if total > 0.0 { stale / total } else { 0.0 });
        stats.index_write_rate = Some(ctx.counter_rate(state, "indexWrite"));
    }
}

/* ================================================================== *
 * timeseriesdb -- cheap appends, expensive range queries
 *
 * An append costs only the drawn `service_ms` (fractions of the cost of
 * a row in a relational store); a range query pays `range_query_ms` on
 * top, for scanning and aggregating many points. A few percent of range
 * queries dominates the pool, which is the argument for keeping metrics
 * out of the main database stated as arithmetic.
 * ================================================================== */

type TsdbPool = Pool<()>;

fn tsdb_ext(ext: &Ext) -> Arc<Mutex<TsdbPool>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<TsdbPool>>>())
        .expect("timeseriesdb ext")
        .clone()
}

fn tsdb_start(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, dispatch: &Dispatch<'_>, _handle: &Arc<Mutex<TsdbPool>>) {
    // Appends were marked as writes in classify; count what is actually
    // being SERVED, so the readout matches throughput rather than
    // arrivals.
    ctx.count_custom(state, if dispatch.is_write() { "append" } else { "rangeQuery" }, 1.0);
}

fn tsdb_classify(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, _handle: &Arc<Mutex<TsdbPool>>) {
    // One unconditional draw: is this a range query? Taken whatever the
    // slider says, so replay does not depend on its value.
    let is_range = ctx.roll() < clamp01(num_opt(state.config().range_query_fraction, 0.0));
    ctx.mark_write(req, !is_range);
    if is_range {
        ctx.add_service_delay(req, num_opt(state.config().range_query_ms, 0.0).max(0.0));
    }
}

pub struct TimeSeriesDbBehaviour;

impl ComponentBehaviour for TimeSeriesDbBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::TimeSeriesDb
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
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

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(init_pool(())))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = tsdb_ext(ext);
        pool_admit(
            ctx,
            state,
            req,
            &handle,
            tsdb_classify,
            PoolHooks { on_start: Some(tsdb_start), on_served: None },
        )
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = tsdb_ext(ext);
        report_pool(ctx, state, &handle);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = tsdb_ext(ext);
        decorate_pool(&handle, stats);
        stats.append_rate = Some(ctx.counter_rate(state, "append"));
        stats.range_query_rate = Some(ctx.counter_rate(state, "rangeQuery"));
    }
}

/* ================================================================== *
 * graphdb -- traversal cost grows with depth
 *
 * Edges visited grow roughly with `outDegree^depth`, so with the
 * modelled mean out-degree of 3 a query costs `service_ms *
 * 3^(depth-1)`: depth 1 is one neighbourhood, depth 2
 * (friends-of-friends) is 3x, depth 3 is 9x. The multiplier is applied as
 * a deterministic surcharge on top of the drawn `service_ms`, so the
 * depth slider moves the mean without touching the variance shape.
 * ================================================================== */

/// Modelled mean out-degree per hop. Fixed, so depth is the single knob.
const GRAPH_FANOUT: f64 = 3.0;

type GraphPool = Pool<()>;

fn graph_ext(ext: &Ext) -> Arc<Mutex<GraphPool>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<GraphPool>>>())
        .expect("graphdb ext")
        .clone()
}

fn graph_classify(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, _handle: &Arc<Mutex<GraphPool>>) {
    let depth = num_opt(state.config().traversal_depth, 1.0).floor().max(1.0);
    let base = state.config().service_ms.max(0.0);
    let mult = GRAPH_FANOUT.powf(depth.min(8.0) - 1.0);
    ctx.add_service_delay(req, base * (mult - 1.0));
}

pub struct GraphDbBehaviour;

impl ComponentBehaviour for GraphDbBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::GraphDb
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
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

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(init_pool(())))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = graph_ext(ext);
        pool_admit(
            ctx,
            state,
            req,
            &handle,
            graph_classify,
            PoolHooks { on_start: None, on_served: None },
        )
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = graph_ext(ext);
        report_pool(ctx, state, &handle);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = graph_ext(ext);
        decorate_pool(&handle, stats);
        stats.traversal_cost_ms = Some(mean_served_ms(ctx, state));
    }
}

/* ================================================================== *
 * vectordb -- embedding search over a large index
 *
 * Cost model: an ANN index probes `log2(2 + index_size_k)` partitions,
 * and the recall target multiplies how much of each partition must
 * actually be scanned, as `1 / (1 - recall)`. So mean query cost is
 *
 *   service_ms * log2(2 + index_size_k) / (1 - recall_target)
 *
 * and the two knobs move it independently: growing the corpus is a
 * logarithm, chasing the last point of recall is a hyperbola. Recall is
 * clamped to 0.99, where the multiplier is already 100x.
 * ================================================================== */

type VectorPool = Pool<()>;

fn vector_ext(ext: &Ext) -> Arc<Mutex<VectorPool>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<VectorPool>>>())
        .expect("vectordb ext")
        .clone()
}

fn vector_classify(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, req: &dyn ReqLike, _handle: &Arc<Mutex<VectorPool>>) {
    let size_k = num_opt(state.config().index_size_k, 0.0).max(0.0);
    let recall = num_opt(state.config().recall_target, 0.0).max(0.0).min(0.99);
    let base = state.config().service_ms.max(0.0);
    let mult = (2.0 + size_k).log2() / (1.0 - recall);
    if mult > 1.0 {
        ctx.add_service_delay(req, base * (mult - 1.0));
    }
}

pub struct VectorDbBehaviour;

impl ComponentBehaviour for VectorDbBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::VectorDb
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
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

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(init_pool(())))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = vector_ext(ext);
        pool_admit(
            ctx,
            state,
            req,
            &handle,
            vector_classify,
            PoolHooks { on_start: None, on_served: None },
        )
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = vector_ext(ext);
        report_pool(ctx, state, &handle);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = vector_ext(ext);
        decorate_pool(&handle, stats);
        stats.query_cost_ms = Some(mean_served_ms(ctx, state));
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static DB: DbBehaviour = DbBehaviour;
static OBJECTSTORE: ObjectStoreBehaviour = ObjectStoreBehaviour;
static SEARCHINDEX: SearchIndexBehaviour = SearchIndexBehaviour;
static TIMESERIESDB: TimeSeriesDbBehaviour = TimeSeriesDbBehaviour;
static GRAPHDB: GraphDbBehaviour = GraphDbBehaviour;
static COLDSTORAGE: ColdStorageBehaviour = ColdStorageBehaviour;
static VECTORDB: VectorDbBehaviour = VectorDbBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::Db, &DB),
    (NodeKind::ObjectStore, &OBJECTSTORE),
    (NodeKind::SearchIndex, &SEARCHINDEX),
    (NodeKind::TimeSeriesDb, &TIMESERIESDB),
    (NodeKind::GraphDb, &GRAPHDB),
    (NodeKind::ColdStorage, &COLDSTORAGE),
    (NodeKind::VectorDb, &VECTORDB),
];
