//! Resilience and delivery component behaviours: bulkhead, retryqueue,
//! transcoder, edgecompute, writebehind, loadshedder. Port of
//! `src/sim/behaviour-resilience.ts`.
//!
//! The theme this module shares is FAILURE HANDLING AS A DESIGN CHOICE.
//! The edge module (cdn, ratelimiter, breaker) is about stopping load
//! before it hurts; these six are about what a system does with the load,
//! and the failures, it has already accepted: contain them (bulkhead),
//! give them somewhere to go (retryqueue), buffer them at a known risk
//! (writebehind), answer them without travelling (edgecompute), grind
//! through them off the request path (transcoder), or choose WHICH of
//! them to fail (loadshedder).
//!
//! Two of them (retryqueue, writebehind) are built on
//! `ctx.ack_and_relay()`, the delivery-side sibling of the queue's
//! `ctx.ack_and_buffer()`: ack the caller, then deliver the detached copy
//! downstream through this node's own slots and the engine's ordinary
//! retry machinery. Neither of them, nor any other kind in this file,
//! calls `ctx.serve_within` -- so unlike `data.rs`/`messaging.rs`, no
//! behaviour here needs the `Arc<Mutex<_>>` ext-capture pattern; every
//! `ext` access happens synchronously inside a hook that already receives
//! `ext: &mut Ext` directly.
//!
//! Determinism notes, same rules as every other behaviour module:
//!   - timestamps are compared, never per-tick deltas accumulated;
//!   - RNG draws happen in a fixed position per event or not at all. The
//!     loadshedder classifies priority from a HASH of the request key
//!     rather than a roll, so it consumes zero randomness, exactly like
//!     the limiter.

use crate::sim::behaviour::{
    clamp01, AdmitAction, ComponentBehaviour, CompleteAction, Ext, InstanceModel, PumpMode,
    RouteMode, ScaleField,
};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqHandle, ReqLike};
use crate::sim::types::{BulkheadMode, FailureReason, NodeKind, NodeStats, SimEdge};
use std::collections::VecDeque;

/* ================================================================== *
 * bulkhead -- an isolated concurrency pool around one dependency
 * ================================================================== */

/// One request waiting to acquire a bulkhead slot, in arrival order. Port
/// of `BulkheadWaiter` (`behaviour-resilience.ts`, upstream `351327c4`, PR
/// #77).
struct BulkheadWaiter {
    /// Storable identity of the held request, obtained via `ctx.handle_of`
    /// at the moment it was queued (see `NodeStateLike`/`ReqLike`'s own
    /// doc comments on why a live view cannot be kept across hook calls).
    handle: ReqHandle,
    /// Simulated time this request joined the queue, for the acquire
    /// latency measured when it is finally admitted.
    entered_at: f64,
}

/// Bulkhead scratch state: slots plus an optional acquire queue.
struct BulkheadExt {
    /// Downstream calls currently outstanding through this pool.
    in_flight: i64,
    /// Requests waiting to acquire a pool slot, FIFO. Only ever non-empty
    /// with `bulkheadMode: Wait`. Port of `BulkheadState.waiters` (upstream
    /// `351327c4`, PR #77).
    waiters: VecDeque<BulkheadWaiter>,
    /// The measured wait of the most recently acquired waiter, or `None`
    /// before the first acquire.
    last_acquire_latency_ms: Option<f64>,
}

fn cfg_bulkhead_max(state: &dyn NodeStateLike) -> i64 {
    match state.config().bulkhead_max {
        Some(v) if v >= 1.0 => v.floor() as i64,
        _ => 8,
    }
}

/// Bulkhead only: waiting acquires allowed before a request is shed
/// immediately as 'bulkhead-full' even with `bulkheadMode: Wait`. Port of
/// `cfgAcquireQueueMax` (`behaviour-resilience.ts`, upstream `351327c4`, PR
/// #77): floored to >= 0, defaulting to 100 when unset.
fn cfg_acquire_queue_max(state: &dyn NodeStateLike) -> i64 {
    match state.config().acquire_queue_max {
        Some(v) if v >= 0.0 => v.floor() as i64,
        _ => 100,
    }
}

/// Bulkhead only: longest a waiting request may wait for a pool slot before
/// it fails as 'acquire-timeout'. Port of `cfgAcquireTimeoutMs` (same
/// upstream commit): defaults to 1000ms when unset.
fn cfg_acquire_timeout_ms(state: &dyn NodeStateLike) -> f64 {
    match state.config().acquire_timeout_ms {
        Some(v) if v >= 0.0 => v,
        _ => 1000.0,
    }
}

/// Promote the longest-waiting acquire once a slot frees. FIFO: the
/// request that has waited longest gets the slot first. Port of
/// `admitWaiter` (`behaviour-resilience.ts`, upstream `351327c4`, PR #77).
fn admit_waiter(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, b: &mut BulkheadExt) {
    let waiter = match b.waiters.pop_front() {
        Some(w) => w,
        None => return,
    };
    b.in_flight += 1;
    b.last_acquire_latency_ms = Some(ctx.now() - waiter.entered_at);
    ctx.count_custom(state, "admitted", 1.0);
    ctx.resume_admission(state, waiter.handle);
}

/// A bulkhead: at most `bulkhead_max` calls may be outstanding to the
/// dependency behind it at once. The default policy (`bulkheadMode: Reject`,
/// or the field left unset) fails the excess here, immediately, as
/// 'bulkhead-full'. Setting `bulkheadMode: Wait` switches to a bounded
/// acquire queue instead: a caller the pool cannot admit right now waits --
/// up to `acquireQueueMax` deep -- for a slot to free, or for its own
/// `acquireTimeoutMs` to elapse, whichever comes first. The former fails as
/// 'acquire-timeout' rather than 'bulkhead-full', and queue depth beyond
/// `acquireQueueMax` still fails fast exactly as before. This models real
/// connection-pool exhaustion (wait, then acquire-timeout, then whatever
/// retry policy the caller has) instead of only its fail-fast half. Port of
/// `behaviour-resilience.ts`'s `bulkhead.onAdmit`/`onWake` (upstream
/// `351327c4`, PR #77).
///
/// The teaching point is Little's law working as a safety property. When
/// the dependency is healthy, concurrency sits at `rate * latency` and the
/// cap is invisible. When the dependency slows down, concurrency is the
/// FIRST number to move -- it climbs toward `rate * new_latency` within
/// one latency of the change -- and the cap catches it before a backlog
/// can form. Admitted calls keep a bounded queue behind the dependency, so
/// their latency stays flat; everything else is refused in microseconds
/// instead of waiting out a timeout. Without the bulkhead the same
/// slowdown fills the dependency's queue to its limit, and every caller
/// pays seconds to learn what this component would have told them
/// instantly.
///
/// It observes concurrency rather than error rate, which is what
/// separates it from the breaker: a dependency that is slow-but-succeeding
/// never trips a breaker's error window, but it fills a bulkhead within
/// one round trip.
///
/// A bulkhead guards ONE pool: it routes each admitted request down
/// exactly one edge (weighted, like an lb, for the rare case of several).
/// To isolate two dependencies, wire two bulkheads; that is precisely how
/// the pattern is deployed in practice, one pool per dependency.
///
/// The pool is counted at admission and released when the downstream call
/// reports back through `on_downstream_result`. The count is deliberately
/// not decremented on this node's own error roll, so `error_rate` and
/// `retries` are not exposed for the kind (both would make the count
/// drift); the defaults keep them at zero.
pub struct BulkheadBehaviour;

impl ComponentBehaviour for BulkheadBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Bulkhead
    }
    // A gate, not a server: it holds no slots of its own and its
    // utilisation would be meaningless. The pool count is its real meter.
    fn serves_requests(&self) -> bool {
        false
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
    // The whole component is a count of downstream outcomes.
    fn observes_outcome(&self) -> bool {
        true
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(BulkheadExt { in_flight: 0, waiters: VecDeque::new(), last_acquire_latency_ms: None }))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        // Nothing wired behind it: there is no pool to guard, so pass
        // through rather than black-holing the topology a student is
        // mid-way through wiring up.
        if state.out().is_empty() {
            return AdmitAction::Passthru;
        }

        let b = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BulkheadExt>())
            .expect("bulkhead ext");
        if b.in_flight >= cfg_bulkhead_max(state) {
            // Bounded acquire queue: with `bulkheadMode: Wait` and room
            // left in the queue, hold the caller here instead of failing
            // it immediately -- it is woken by `admit_waiter` (a slot
            // freed) or `on_wake` (its own acquire timeout elapsed),
            // whichever comes first. Default mode is `Reject` (`None`
            // reads as `Reject` too), so a topology saved before this
            // feature existed keeps today's immediate-rejection behaviour
            // unchanged. Port of `onAdmit` (`behaviour-resilience.ts`,
            // upstream `351327c4`, PR #77).
            if state.config().bulkhead_mode == Some(BulkheadMode::Wait)
                && (b.waiters.len() as i64) < cfg_acquire_queue_max(state)
            {
                let handle = ctx.handle_of(req);
                b.waiters.push_back(BulkheadWaiter { handle, entered_at: ctx.now() });
                ctx.wake_after(state, handle, cfg_acquire_timeout_ms(state));
                return AdmitAction::Handled;
            }
            ctx.count_custom(state, "bulkheadRejected", 1.0);
            ctx.reject(state, req, FailureReason::BulkheadFull);
            return AdmitAction::Handled;
        }
        b.in_flight += 1;
        ctx.count_custom(state, "admitted", 1.0);
        AdmitAction::Passthru
    }

    fn route(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> RouteMode {
        RouteMode::One
    }

    fn pick_edge(
        &self,
        ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        out: &[SimEdge],
        ext: &mut Ext,
    ) -> Option<SimEdge> {
        let edge = ctx.pick_weighted_or_least_loaded(out);
        if edge.is_none() {
            // Every edge is cut: no downstream call will be made, so no
            // on_downstream_result will ever release the slot this
            // request was granted at admission. Hand it back here or the
            // pool leaks shut.
            if let Some(b) = ext.as_mut().and_then(|e| e.downcast_mut::<BulkheadExt>()) {
                if b.in_flight > 0 {
                    b.in_flight -= 1;
                }
            }
        }
        edge
    }

    fn on_downstream_result(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ok: bool,
        _reason: FailureReason,
        ext: &mut Ext,
    ) {
        let b = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BulkheadExt>())
            .expect("bulkhead ext");
        if b.in_flight > 0 {
            b.in_flight -= 1;
        }
        // A slot just freed: hand it to the longest-waiting acquire, if
        // any. Port of the same line in TS's `onDownstreamResult`
        // (upstream `351327c4`, PR #77).
        if b.in_flight < cfg_bulkhead_max(state) {
            admit_waiter(ctx, state, b);
        }
    }

    fn on_wake(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) {
        let b = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BulkheadExt>())
            .expect("bulkhead ext");
        let handle = ctx.handle_of(req);
        // Already admitted by `admit_waiter` before this timer fired: the
        // waiter is gone from the queue, so there is nothing left to time
        // out. Port of `onWake`'s `isWaiting` guard (`behaviour-
        // resilience.ts`, upstream `351327c4`, PR #77).
        let idx = match b.waiters.iter().position(|w| w.handle == handle) {
            Some(i) => i,
            None => return,
        };
        b.waiters.remove(idx);
        ctx.count_custom(state, "bulkheadAcquireTimeout", 1.0);
        ctx.reject(state, req, FailureReason::AcquireTimeout);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let (in_flight, waiting, last_acquire_latency_ms) = ext
            .as_ref()
            .and_then(|e| e.downcast_ref::<BulkheadExt>())
            .map(|b| (b.in_flight, b.waiters.len() as i64, b.last_acquire_latency_ms))
            .unwrap_or((0, 0, None));
        stats.bulkhead_in_flight = Some(in_flight as f64);
        stats.bulkhead_limit = Some(cfg_bulkhead_max(state) as f64);
        stats.bulkhead_rejected_rate = Some(ctx.counter_rate(state, "bulkheadRejected"));
        stats.bulkhead_waiting = Some(waiting as f64);
        stats.bulkhead_acquire_latency_ms = last_acquire_latency_ms;
        stats.bulkhead_acquire_timeout_rate = Some(ctx.counter_rate(state, "bulkheadAcquireTimeout"));
        // Show the pool as this node's occupancy so the canvas meter means
        // "how full is the bulkhead" rather than sitting at zero forever.
        stats.in_flight = in_flight as f64;
    }
}

/* ================================================================== *
 * retryqueue -- retried delivery with a dead letter shelf
 * ================================================================== */

/// Retry-queue scratch state.
struct RetryQueueExt {
    /// Messages that exhausted every retry since sim start.
    dead_letters: f64,
}

/// A delivery queue with a redrive policy -- SQS with `maxReceiveCount`, a
/// Rabbit queue with a DLX.
///
/// The caller is acknowledged instantly, exactly like the plain queue. The
/// difference is on the far side: this node delivers the message
/// downstream ITSELF (`capacity` is its delivery concurrency), and when a
/// delivery fails it is retried with exponential backoff -- the engine's
/// own retry path, driven by this node's `retries` -- instead of the
/// failure evaporating. A message that fails every attempt lands on the
/// DEAD LETTER shelf: a counter that only grows, in plain sight.
///
/// The teaching point is that failures need somewhere to go. A plain
/// queue feeding a flaky worker silently loses every failed message; a
/// student watching only throughput never learns it happened. Here the
/// same flakiness shows up as a redelivery rate (the early warning) and
/// then a dead letter count (the bill), and a transient error rate of e
/// is survived at a cost of roughly e + e^2 extra deliveries while only
/// the e^(retries+1) fraction is lost -- numbers a student can check
/// against the sliders.
///
/// `credits_join_completion` is FALSE, and must be: the ack already
/// booked one completion for the message, and the delivered message
/// joining its downstream call at this same node would book a second.
pub struct RetryQueueBehaviour;

impl ComponentBehaviour for RetryQueueBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::RetryQueue
    }
    // Delivery slots are real work: the meter reads delivery concurrency.
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
        PumpMode::Own
    }
    fn credits_join_completion(&self) -> bool {
        false
    }
    fn observes_outcome(&self) -> bool {
        true
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(RetryQueueExt { dead_letters: 0.0 }))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        if ctx.queue_depth(state) >= ctx.effective_queue_limit(state) {
            return AdmitAction::Shed;
        }
        ctx.ack_and_relay(state, req, None);
        AdmitAction::Handled
    }

    fn on_downstream_result(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ok: bool,
        reason: FailureReason,
        ext: &mut Ext,
    ) {
        if ok {
            ctx.count_custom(state, "delivered", 1.0);
            return;
        }
        // Mirror the engine's own retry predicate exactly, so this ledger
        // never disagrees with what actually happens next: a failed
        // attempt is redelivered unless the budget is spent or the
        // failure is one the engine never retries (a wiring error, a
        // depth blowout).
        let retries = state.config().retries.floor().max(0.0) as i64;
        let is_final = (req.attempt() as i64) >= retries
            || reason == FailureReason::Depth
            || reason == FailureReason::NoRoute;
        if is_final {
            ctx.count_custom(state, "deadLetter", 1.0);
            if let Some(e) = ext.as_mut().and_then(|e| e.downcast_mut::<RetryQueueExt>()) {
                e.dead_letters += 1.0;
            }
        } else {
            ctx.count_custom(state, "redelivery", 1.0);
        }
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        stats.delivered_rate = Some(ctx.counter_rate(state, "delivered"));
        stats.redelivery_rate = Some(ctx.counter_rate(state, "redelivery"));
        stats.dead_letter_rate = Some(ctx.counter_rate(state, "deadLetter"));
        stats.dead_letters = Some(
            ext.as_ref()
                .and_then(|e| e.downcast_ref::<RetryQueueExt>())
                .map(|e| e.dead_letters)
                .unwrap_or(0.0),
        );
    }
}

/* ================================================================== *
 * transcoder -- a CPU-bound batch job farm
 * ================================================================== */

/// Longest ladder the farm will encode -- the inspector's own maximum for
/// the `renditions` field (`field-schema.ts`: min 1 / max 12).
///
/// The cap lives here, not only at the number input, because
/// `on_service_complete` below runs a loop once per rendition per outgoing
/// edge per finished job, and `renditions` is not one of the config numbers
/// the topology loader validates: a shared link, a `.breakscale` file or a
/// restored session can still carry anything into that loop.
const MAX_RENDITIONS: i64 = 12;

/// Pure arithmetic behind `cfg_renditions`, factored out so the guard can be
/// unit-tested without a `NodeStateLike`.
///
/// WHY `v.is_finite()` and not just `v >= 1.0`: `Infinity >= 1.0` is true in
/// IEEE-754 and `.floor()` leaves `Infinity` unchanged, so the pre-#62 guard
/// let an unbounded rendition count reach the emit loop and never finish.
/// Port of `behaviour-resilience.ts` `cfgRenditions()` (upstream `dcb6980c`,
/// PR #62): guard non-finite input first, then hold the floored value at
/// `MAX_RENDITIONS` instead of letting it through uncapped.
fn clamp_renditions(v: Option<f64>) -> i64 {
    match v {
        Some(x) if x.is_finite() && x >= 1.0 => x.floor().min(MAX_RENDITIONS as f64) as i64,
        _ => 3,
    }
}

fn cfg_renditions(state: &dyn NodeStateLike) -> i64 {
    clamp_renditions(state.config().renditions)
}

/// A transcoder farm: workers whose jobs take SECONDS, and whose output is
/// FILES, not responses.
///
/// The intake is the worker's discipline -- it drains the queue nodes
/// that feed it -- but what happens at completion is what makes it an
/// encode farm rather than a worker in a slow regime. A finished job
/// produces the quality LADDER: `renditions` output files per outgoing
/// edge, handed downstream as detached uploads that nothing waits on.
/// Storage therefore sees `renditions * job_rate` writes -- upload one
/// video, store four encodes -- and a job that finished cleanly is done
/// even if an upload later fails, exactly as a real pipeline treats its
/// origin push.
///
/// Two lessons, both in the numbers. Capacity: at `service_ms` in the
/// thousands, `capacity` means "encodes per box", throughput is single
/// digits, and undersizing the farm by 10% grows the queue in front of it
/// FOREVER, because the deficit is structural. Amplification: the write
/// load on the store behind the farm is a multiple the ladder chose, not
/// the ingest rate the client sees.
///
/// Emitting consumes no randomness; failed emits (a cut edge, a missing
/// target) are simply lost artifacts, counted nowhere upstream.
pub struct TranscoderBehaviour;

impl ComponentBehaviour for TranscoderBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Transcoder
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
        true
    }
    fn buffers_for_consumers(&self) -> bool {
        false
    }
    fn pump(&self) -> PumpMode {
        PumpMode::Sources
    }
    // FALSE, and it must be: a finished job is booked here by the
    // 'complete' path below, and the detached uploads join back through
    // this node -- were joins credited too, every rendition landing on
    // storage would count as a second (and third, and fourth) completed
    // job, and the store's latency would pollute the farm's. Same
    // reasoning as the stream broker.
    fn credits_join_completion(&self) -> bool {
        false
    }

    fn on_service_complete(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        let renditions = cfg_renditions(state);
        for i in 0..state.out().len() {
            let edge = state.out()[i].clone();
            for _ in 0..renditions {
                if !ctx.emit_detached(state, &edge, req.key()) {
                    break;
                }
                ctx.count_custom(state, "output", 1.0);
            }
        }
        // The job is finished HERE: uploads are detached, so a slow store
        // makes artifacts queue at the store, never the encode slot wait
        // on it.
        CompleteAction::Complete
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
        stats.output_rate = Some(ctx.counter_rate(state, "output"));
    }
}

/* ================================================================== *
 * edgecompute -- limited compute at the CDN edge
 * ================================================================== */

/// Compute at the edge: a small function running in the PoP itself.
///
/// `edge_share` is the fraction of requests it can fully answer -- an
/// auth check, a redirect, a personalised fragment assembled from what is
/// already at the edge. Those complete in single-digit milliseconds
/// without a single byte travelling to the origin. Everything else passes
/// through and pays the full origin path.
///
/// The difference from the CDN is what the number means. A CDN's
/// `hit_rate` is a property of the TRAFFIC (how repetitive it is);
/// `edge_share` is a property of the CODE (how much logic you managed to
/// push outward), and raising it is engineering work, not luck. The
/// lesson is the same shape though: the p50 a client sees is a blend of
/// two distributions, and moving work to the edge moves requests between
/// them one slider-notch at a time.
///
/// What keeps it honest is the CPU BUDGET (`cpu_ms_cap`): an edge runtime
/// is not a server, and a request whose execution runs past the
/// per-request ceiling is killed at the edge and passed to the origin
/// even though the code could have answered it. Push heavier logic
/// outward (raise `service_ms`) and the budget starts eating the very
/// share you were trying to raise -- the trade every Workers/Lambda@Edge
/// deployment lives with.
///
/// One RNG draw per completion, unconditionally and in a fixed position,
/// for the same replay-stability reason the cache documents.
pub struct EdgeComputeBehaviour;

impl ComponentBehaviour for EdgeComputeBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::EdgeCompute
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
        PumpMode::Own
    }
    fn credits_join_completion(&self) -> bool {
        true
    }

    fn on_service_complete(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        // One RNG draw, unconditionally and first, so the stream never
        // depends on the CPU budget check below.
        let can_answer = ctx.roll() < clamp01(state.config().edge_share.unwrap_or(0.0));
        if can_answer {
            // The CPU budget: an edge runtime kills a request that runs
            // past its per-request ceiling, and the work falls through to
            // the origin even though the code COULD have answered it.
            // Judged against the actual execution time this request drew
            // (req.own_ms()), so a heavier function body (higher
            // service_ms) blows the budget more often -- the exact trade
            // a student makes when pushing logic outward.
            if let Some(cap) = state.config().cpu_ms_cap {
                if cap > 0.0 && req.own_ms() > cap {
                    ctx.count_custom(state, "cpuExceeded", 1.0);
                    ctx.count_custom(state, "passedThrough", 1.0);
                    return if state.out().is_empty() {
                        CompleteAction::Complete
                    } else {
                        CompleteAction::Downstream
                    };
                }
            }
            ctx.count_custom(state, "edgeHandled", 1.0);
            return CompleteAction::Complete;
        }
        ctx.count_custom(state, "passedThrough", 1.0);
        // Nothing behind it: it still answers, it just cannot demonstrate
        // the pass-through lesson.
        if state.out().is_empty() {
            CompleteAction::Complete
        } else {
            CompleteAction::Downstream
        }
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
        stats.edge_handled_rate = Some(ctx.counter_rate(state, "edgeHandled"));
        stats.passed_through_rate = Some(ctx.counter_rate(state, "passedThrough"));
        stats.cpu_exceeded_rate = Some(ctx.counter_rate(state, "cpuExceeded"));
    }
}

/* ================================================================== *
 * writebehind -- a write buffer that trades durability for latency
 * ================================================================== */

fn cfg_flush_delay_ms(state: &dyn NodeStateLike) -> f64 {
    match state.config().flush_delay_ms {
        Some(v) if v > 0.0 => v,
        _ => 0.0,
    }
}

/// A write-behind cache: writes are acknowledged from memory in about a
/// millisecond, and flushed to the backing store `flush_delay_ms` later.
///
/// Chosen over a read-through variant deliberately: the existing `cache`
/// already IS read-through (hit answers, miss falls downstream), so the
/// read path is covered. What the palette lacked was the write path's
/// bargain, and write-behind is the sharpest form of it: the caller is
/// told "saved" while the data exists only in this node's memory.
///
/// `capacity` is the buffer -- how many writes may sit dirty at once,
/// memory rather than threads -- and the standing dirty population is
/// `write_rate` times the flush delay, which the utilisation meter reads
/// as buffer fill.
///
/// The lesson has two halves. The visible half: downstream sees the same
/// write rate, just later and smoothed, and the caller's latency no
/// longer contains the store's -- that is why the pattern exists. The
/// half that matters: CRASH THIS NODE. Every buffered write fails as
/// 'crashed' in the same instant, and those are writes the callers were
/// already told succeeded. The failures spike on the graph is data loss
/// made countable, and no other component in the palette can show it.
///
/// `credits_join_completion` is FALSE for the same double-count reason as
/// the retry queue: the ack already booked the completion.
///
/// NOTE: unlike `retryqueue`, the TS source declares no `instanceModel`
/// or `scaleField` for `writebehind` despite `servesRequests: true`. This
/// is what the actual behaviour object says (checked line by line, not
/// inferred), so it is ported as-is: no instance model, no scale field.
pub struct WriteBehindBehaviour;

impl ComponentBehaviour for WriteBehindBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::WriteBehind
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
        false
    }
    fn observes_outcome(&self) -> bool {
        true
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        if ctx.queue_depth(state) >= ctx.effective_queue_limit(state) {
            return AdmitAction::Shed;
        }
        // The relayed copy carries the flush delay as extra service time,
        // so a dirty write HOLDS its buffer slot for the whole residence
        // interval -- which is what makes the buffer a real population a
        // crash can lose, not an accounting fiction.
        ctx.ack_and_relay(state, req, Some(cfg_flush_delay_ms(state)));
        AdmitAction::Handled
    }

    fn on_downstream_result(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        ok: bool,
        _reason: FailureReason,
        _ext: &mut Ext,
    ) {
        ctx.count_custom(state, if ok { "flushed" } else { "flushFailed" }, 1.0);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
        // Dirty writes: acknowledged, not yet landed on the store.
        // Buffered (queued) plus mid-residence (in_flight); both die with
        // the node.
        stats.dirty_writes = Some(stats.in_flight + stats.queued);
        stats.flushed_rate = Some(ctx.counter_rate(state, "flushed"));
        // The 'flushFailed' counter was already booked in
        // on_downstream_result; publish it. Every one of these is a write
        // the caller believes is safe.
        stats.flush_fail_rate = Some(ctx.counter_rate(state, "flushFailed"));
    }
}

/* ================================================================== *
 * loadshedder -- priority-aware admission control
 * ================================================================== */

/// Token bucket state, same shape as the rate limiter's.
struct ShedderExt {
    /// Tokens available right now. Fractional: refill is continuous.
    tokens: f64,
    /// Simulated time the bucket was last refilled, in ms.
    last_refill_ms: f64,
    /// Bucket size the tokens were last capped against, to detect a
    /// config change.
    last_burst: f64,
}

fn shed_rate(state: &dyn NodeStateLike) -> f64 {
    match state.config().rate_limit_rps {
        Some(r) if r > 0.0 => r,
        _ => 0.0,
    }
}

fn shed_burst(state: &dyn NodeStateLike) -> f64 {
    if let Some(b) = state.config().burst {
        if b > 0.0 {
            return b;
        }
    }
    let r = shed_rate(state);
    if r > 0.0 {
        r
    } else {
        1.0
    }
}

/// Refill against elapsed SIMULATED time, exactly as the rate limiter
/// does and for exactly the reasons documented there: over T seconds the
/// bucket grants rate*T tokens no matter how the frames fell.
fn shed_refill(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, b: &mut ShedderExt) {
    let burst = shed_burst(state);
    if burst != b.last_burst {
        if b.tokens > burst {
            b.tokens = burst;
        }
        b.last_burst = burst;
    }
    let elapsed = ctx.now() - b.last_refill_ms;
    if elapsed <= 0.0 {
        return;
    }
    b.last_refill_ms = ctx.now();
    let rate = shed_rate(state);
    if rate <= 0.0 {
        return;
    }
    b.tokens += (elapsed / 1000.0) * rate;
    if b.tokens > burst {
        b.tokens = burst;
    }
}

/// The bucket as of now, without mutating it. Snapshot-side arithmetic
/// only.
fn shed_projected(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike, b: &ShedderExt) -> f64 {
    let burst = shed_burst(state);
    let mut tokens = if b.tokens > burst { burst } else { b.tokens };
    let rate = shed_rate(state);
    let elapsed = ctx.now() - b.last_refill_ms;
    if rate > 0.0 && elapsed > 0.0 {
        tokens += (elapsed / 1000.0) * rate;
        if tokens > burst {
            tokens = burst;
        }
    }
    tokens
}

/// Is this request low priority?
///
/// Derived from the request KEY, not from an RNG draw: the key already
/// identifies who is asking (it is what shards partition on), so "the low
/// 30% of the key space is best-effort traffic" is the honest model of
/// tiered users -- the same caller is always the same priority, retries
/// included -- and it costs the random stream nothing. The key is hashed
/// first (Knuth's multiplicative constant) so priority does not correlate
/// with shard index, which is key modulo `shard_count` on the raw value.
fn is_low_priority(state: &dyn NodeStateLike, req: &dyn ReqLike) -> bool {
    let share = clamp01(state.config().low_priority_share.unwrap_or(0.0));
    if share <= 0.0 {
        return false;
    }
    // `Math.imul(req.key, 2654435761) >>> 0` in the TS source: 32-bit
    // integer multiplication with wraparound, reinterpreted unsigned.
    // Since `req.key()` is already a non-negative u32, an ordinary
    // wrapping u32 multiplication produces the identical bit pattern.
    let hashed = req.key().wrapping_mul(2654435761u32) as f64 / 4294967296.0;
    hashed < share
}

/// A load shedder: admission control that fails the RIGHT traffic.
///
/// Same token bucket as the rate limiter, one difference in the admission
/// rule: a high-priority request needs 1 token, a low-priority request
/// needs the bucket to also hold a reserve of `priority_reserve * burst`
/// on top. In an idle system the bucket sits full and everyone gets in.
/// Under saturation, tokens hover near zero, which is INSIDE the reserve
/// -- so low-priority traffic is refused ('deprioritized') while
/// high-priority traffic keeps finding its single token ('throttled' only
/// appears once even the high tier outruns the bucket).
///
/// The teaching point is graceful degradation as a policy rather than an
/// accident. A plain limiter at 2x load fails half of EVERYTHING --
/// checkout and thumbnail alike. This component fails the thumbnails
/// 100% and the checkouts 0%, at the same total admission rate, and the
/// per-priority readouts let a student verify exactly that trade. Zero
/// RNG draws.
pub struct LoadShedderBehaviour;

impl ComponentBehaviour for LoadShedderBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::LoadShedder
    }
    // A doorman, like the limiter: no slots, no meaningful utilisation.
    fn serves_requests(&self) -> bool {
        false
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

    fn init_state(&self, state: &dyn NodeStateLike) -> Ext {
        // Starts full, like the limiter: an idle system absorbs a burst.
        let burst = shed_burst(state);
        Some(Box::new(ShedderExt {
            tokens: burst,
            last_refill_ms: 0.0,
            last_burst: burst,
        }))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let b = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<ShedderExt>())
            .expect("loadshedder ext");
        shed_refill(ctx, state, b);
        let low = is_low_priority(state, req);

        // No rate configured means no limit: pass everything, as the
        // limiter does, rather than black-holing a half-configured node.
        if shed_rate(state) <= 0.0 {
            ctx.count_custom(state, if low { "lowAdmitted" } else { "highAdmitted" }, 1.0);
            return AdmitAction::Passthru;
        }

        // The admission floor this request must find in the bucket. High
        // priority pays list price; low priority must also leave the
        // reserve untouched, which is the entire mechanism.
        let floor = if low {
            1.0 + clamp01(state.config().priority_reserve.unwrap_or(0.3)) * shed_burst(state)
        } else {
            1.0
        };

        if b.tokens >= floor {
            b.tokens -= 1.0;
            ctx.count_custom(state, if low { "lowAdmitted" } else { "highAdmitted" }, 1.0);
            return AdmitAction::Passthru;
        }

        ctx.count_custom(state, if low { "lowShed" } else { "highShed" }, 1.0);
        ctx.reject(
            state,
            req,
            if low { FailureReason::Deprioritized } else { FailureReason::Throttled },
        );
        AdmitAction::Handled
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let high_admitted = ctx.counter_rate(state, "highAdmitted");
        let low_admitted = ctx.counter_rate(state, "lowAdmitted");
        stats.high_admitted_rate = Some(high_admitted);
        stats.low_admitted_rate = Some(low_admitted);
        stats.high_shedded_rate = Some(ctx.counter_rate(state, "highShed"));
        stats.low_shedded_rate = Some(ctx.counter_rate(state, "lowShed"));
        stats.admitted_rate = Some(high_admitted + low_admitted);
        // Projected, not stored: refill is lazy, same reasoning as the
        // limiter.
        stats.tokens = Some(match ext.as_ref().and_then(|e| e.downcast_ref::<ShedderExt>()) {
            Some(b) => shed_projected(ctx, state, b),
            None => 0.0,
        });
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static BULKHEAD: BulkheadBehaviour = BulkheadBehaviour;
static RETRYQUEUE: RetryQueueBehaviour = RetryQueueBehaviour;
static TRANSCODER: TranscoderBehaviour = TranscoderBehaviour;
static EDGECOMPUTE: EdgeComputeBehaviour = EdgeComputeBehaviour;
static WRITEBEHIND: WriteBehindBehaviour = WriteBehindBehaviour;
static LOADSHEDDER: LoadShedderBehaviour = LoadShedderBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::Bulkhead, &BULKHEAD),
    (NodeKind::RetryQueue, &RETRYQUEUE),
    (NodeKind::Transcoder, &TRANSCODER),
    (NodeKind::EdgeCompute, &EDGECOMPUTE),
    (NodeKind::WriteBehind, &WRITEBEHIND),
    (NodeKind::LoadShedder, &LOADSHEDDER),
];

#[cfg(test)]
mod tests {
    use super::clamp_renditions;
    use crate::sim::engine::Engine;
    use crate::sim::presets::default_config;
    use crate::sim::types::{
        BulkheadMode, FailureReason, FailuresByReason, NodeConfig, NodeKind, NodeStats, SimEdge,
        SimNode, Topology,
    };

    /// WHY: `behaviour-resilience.ts` `cfgRenditions()` (upstream
    /// `dcb6980c`, PR #62). Pure unit coverage of the helper itself:
    /// upstream's guard is `!Number.isFinite(v) || v < 1`, which routes
    /// `Infinity` to the fallback of 3 exactly like `NaN` -- only a huge
    /// but FINITE value (1e9 below) is held at `MAX_RENDITIONS`.
    #[test]
    fn clamp_renditions_guards_non_finite_and_caps_the_maximum() {
        assert_eq!(clamp_renditions(Some(f64::NAN)), 3);
        assert_eq!(clamp_renditions(Some(f64::INFINITY)), 3);
        assert_eq!(clamp_renditions(Some(f64::NEG_INFINITY)), 3);
        assert_eq!(clamp_renditions(Some(1e9)), 12);
        assert_eq!(clamp_renditions(Some(0.0)), 3);
        assert_eq!(clamp_renditions(None), 3);
        assert_eq!(clamp_renditions(Some(8.9)), 8);
    }

    fn node(id: &str, kind: NodeKind, config: NodeConfig) -> SimNode {
        SimNode { id: id.to_string(), kind, label: id.to_string(), x: 0.0, y: 0.0, config }
    }

    /// A client feeding a transcoder farm feeding an object store, patched
    /// with whatever `renditions` value the test wants to try breaking.
    /// Mirrors `behaviour-resilience.bounds.test.ts` (upstream `dcb6980c`,
    /// PR #62).
    fn topology(patch: impl FnOnce(&mut NodeConfig)) -> Topology {
        let client_cfg = NodeConfig { rps: 30.0, ..default_config(NodeKind::Client) };
        let mut farm_cfg = default_config(NodeKind::Transcoder);
        patch(&mut farm_cfg);
        let store_cfg = default_config(NodeKind::ObjectStore);
        let edge = |id: &str, from: &str, to: &str| SimEdge {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            weight: 1.0,
            control: None,
            latency_ms: None,
            bandwidth_rps: None,
            loss_rate: None,
        };
        Topology {
            nodes: vec![
                node("client", NodeKind::Client, client_cfg),
                node("farm", NodeKind::Transcoder, farm_cfg),
                node("store", NodeKind::ObjectStore, store_cfg),
            ],
            edges: vec![edge("e1", "client", "farm"), edge("e2", "farm", "store")],
            annotations: None,
        }
    }

    /// `farm`'s own stats after the engine has had time to finish at least
    /// one job. Mirrors `stats_for` in `behaviour/data.rs`'s tests. 600
    /// ticks (10 simulated seconds) rather than 120 (2s): the farm's own
    /// preset is a 1.2s-mean service time at `service_cv: 0.4`, so a 2s
    /// window leaves only a thin margin before the first job's randomly
    /// drawn duration can miss it entirely; 10s comfortably covers even a
    /// multiple-of-the-mean draw.
    fn farm_stats(patch: impl FnOnce(&mut NodeConfig)) -> NodeStats {
        let mut engine = Engine::new(topology(patch), 7);
        for _ in 0..600 {
            engine.advance(1000.0 / 60.0);
        }
        engine.snapshot().nodes.remove("farm").expect("farm node")
    }

    /// WHY: same upstream fix. Before it, a huge or non-finite `renditions`
    /// reached the `on_service_complete` emit loop -- one iteration per
    /// rendition per outgoing edge per finished job -- which never
    /// terminated. The engine must still run to completion in bounded time
    /// and must never publish a non-finite output rate.
    #[test]
    fn rendition_count_is_bounded_for_huge_or_non_finite_values() {
        for renditions in [f64::INFINITY, 1e9, f64::NAN] {
            let stats = farm_stats(|c| c.renditions = Some(renditions));
            assert!(
                stats.output_rate.unwrap_or(f64::NAN).is_finite(),
                "renditions={renditions} produced a non-finite output_rate"
            );
        }
    }

    /// The ceiling is the inspector's own maximum (`field-schema.ts`:
    /// `renditions` min 1 / max 12), so a ladder a reader can actually
    /// build through the UI is untouched: a farm configured for 8
    /// renditions still finishes jobs and reports output.
    #[test]
    fn a_rendition_count_the_inspector_can_set_is_unaffected() {
        assert_eq!(clamp_renditions(Some(8.0)), 8);
        let stats = farm_stats(|c| c.renditions = Some(8.0));
        assert!(stats.output_rate.unwrap_or(0.0) > 0.0, "farm configured for 8 renditions produced no output");
    }

    /* ---------------------------------------------------------------- *
     * Bulkhead acquire-queue (upstream `351327c4`, PR #77).
     *
     * A client feeding a single-slot bulkhead feeding a dependency
     * service, mirroring `bulkhead.acquire.test.ts`'s shape: the
     * dependency's `serviceMs` controls how long the pool's one slot
     * stays held, and so how often a waiter's own acquire timeout beats a
     * freed slot. The client's OWN `timeoutMs` is set far longer than any
     * test's run, so the only timeout-shaped failure in play is the
     * bulkhead's 'acquire-timeout' -- never the client giving up.
     * ---------------------------------------------------------------- */

    fn bulkhead_topology(
        client_rps: f64,
        dependency_service_ms: f64,
        patch: impl FnOnce(&mut NodeConfig),
    ) -> Topology {
        let client_cfg =
            NodeConfig { rps: client_rps, timeout_ms: 60000.0, ..default_config(NodeKind::Client) };
        let mut pool_cfg = default_config(NodeKind::Bulkhead);
        pool_cfg.bulkhead_max = Some(1.0);
        patch(&mut pool_cfg);
        let dep_cfg = NodeConfig {
            capacity: 1.0,
            service_ms: dependency_service_ms,
            service_cv: 0.0,
            queue_limit: 1000.0,
            ..default_config(NodeKind::Service)
        };
        let edge = |id: &str, from: &str, to: &str| SimEdge {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            weight: 1.0,
            control: None,
            latency_ms: None,
            bandwidth_rps: None,
            loss_rate: None,
        };
        Topology {
            nodes: vec![
                node("client", NodeKind::Client, client_cfg),
                node("pool", NodeKind::Bulkhead, pool_cfg),
                node("dep", NodeKind::Service, dep_cfg),
            ],
            edges: vec![edge("e1", "client", "pool"), edge("e2", "pool", "dep")],
            annotations: None,
        }
    }

    /// `pool`'s own stats, and the run's `failuresByReason`, after `seconds`
    /// of simulated time. Mirrors `farm_stats` above and
    /// `bulkhead.acquire.test.ts`'s `snapshotAfter` (same upstream commit).
    fn pool_stats(
        client_rps: f64,
        dependency_service_ms: f64,
        seconds: f64,
        patch: impl FnOnce(&mut NodeConfig),
    ) -> (NodeStats, FailuresByReason) {
        let mut engine = Engine::new(bulkhead_topology(client_rps, dependency_service_ms, patch), 7);
        let ticks = (seconds * 60.0) as u32;
        for _ in 0..ticks {
            engine.advance(1000.0 / 60.0);
        }
        let mut snapshot = engine.snapshot();
        let stats = snapshot.nodes.remove("pool").expect("pool node");
        (stats, snapshot.failures_by_reason)
    }

    /// (a) Regression guard: with every new field left `None` -- a topology
    /// saved before this feature existed -- the bulkhead behaves exactly as
    /// it did before #77. No waiting ever happens and the pool's excess
    /// fails fast as 'bulkhead-full', never 'acquire-timeout'.
    #[test]
    fn absent_bulkhead_mode_still_rejects_immediately() {
        let (stats, failures) = pool_stats(300.0, 200.0, 2.0, |c| {
            c.bulkhead_mode = None;
            c.acquire_queue_max = None;
            c.acquire_timeout_ms = None;
        });
        assert_eq!(stats.bulkhead_waiting, Some(0.0), "a Reject bulkhead must never queue a waiter");
        assert!(
            stats.bulkhead_rejected_rate.unwrap_or(0.0) > 0.0,
            "a single-slot pool under load must refuse its excess immediately"
        );
        assert_eq!(
            stats.bulkhead_acquire_timeout_rate,
            Some(0.0),
            "no waiter was ever queued, so no acquire-timeout can have fired"
        );
        assert_eq!(
            failures.get(&FailureReason::AcquireTimeout).copied().unwrap_or(0),
            0,
            "'reject' mode must never produce an acquire-timeout failure"
        );
    }

    /// (b) A queued request that gets a freed slot before its acquire
    /// timeout elapses completes successfully: the client's offered rate
    /// (50 rps, mean 20ms gap) stays BELOW the single-slot pool's own
    /// service rate (1 / 10ms = 100 rps), so the pool is loaded but
    /// stable -- an arrival that finds the one slot briefly busy still
    /// gets it well inside a generous acquire timeout, and the queue depth
    /// Poisson burstiness produces never approaches `acquireQueueMax`.
    /// Neither 'bulkhead-full' nor 'acquire-timeout' is ever booked.
    #[test]
    fn a_waiter_admitted_before_its_timeout_elapses_succeeds() {
        let (stats, failures) = pool_stats(50.0, 10.0, 2.0, |c| {
            c.bulkhead_mode = Some(BulkheadMode::Wait);
            c.acquire_queue_max = Some(100.0);
            c.acquire_timeout_ms = Some(5000.0);
        });
        assert_eq!(
            stats.bulkhead_acquire_timeout_rate,
            Some(0.0),
            "a 5s acquire timeout must never fire against a ~10ms dependency at this load"
        );
        assert_eq!(
            failures.get(&FailureReason::AcquireTimeout).copied().unwrap_or(0),
            0,
            "no request should have timed out waiting for a slot"
        );
        assert_eq!(
            failures.get(&FailureReason::BulkheadFull).copied().unwrap_or(0),
            0,
            "a stable pool (offered rate below its own service rate) must never overflow a 100-deep queue"
        );
        assert!(
            stats.bulkhead_acquire_latency_ms.is_some(),
            "at 50% utilisation some arrival must have found the one slot busy and actually waited for it"
        );
    }

    /// (c) A queued request that exceeds its acquire timeout is rejected
    /// with the distinct 'acquire-timeout' reason, and counted in the new
    /// rate stat: a short `acquireTimeoutMs` against a slow dependency
    /// means a waiter almost always times out before a slot frees.
    #[test]
    fn a_waiter_past_its_timeout_fails_as_acquire_timeout() {
        let (stats, failures) = pool_stats(100.0, 500.0, 2.0, |c| {
            c.bulkhead_mode = Some(BulkheadMode::Wait);
            c.acquire_queue_max = Some(50.0);
            c.acquire_timeout_ms = Some(20.0);
        });
        assert!(
            stats.bulkhead_acquire_timeout_rate.unwrap_or(0.0) > 0.0,
            "a 20ms acquire timeout against a 500ms dependency must fire repeatedly"
        );
        assert!(
            failures.get(&FailureReason::AcquireTimeout).copied().unwrap_or(0) > 0,
            "the engine's own failuresByReason must count the acquire-timeout too"
        );
    }

    /// (d) The acquire queue itself is bounded: beyond `acquireQueueMax`,
    /// a `wait` bulkhead still rejects immediately as 'bulkhead-full'
    /// rather than growing the queue without limit, and the live queue
    /// depth never exceeds the configured bound.
    #[test]
    fn the_acquire_queue_is_bounded() {
        let (stats, failures) = pool_stats(300.0, 1000.0, 2.0, |c| {
            c.bulkhead_mode = Some(BulkheadMode::Wait);
            c.acquire_queue_max = Some(1.0);
            c.acquire_timeout_ms = Some(5000.0);
        });
        assert!(
            stats.bulkhead_waiting.unwrap_or(f64::NAN) <= 1.0,
            "waiting ({:?}) must never exceed acquireQueueMax",
            stats.bulkhead_waiting
        );
        assert!(
            stats.bulkhead_rejected_rate.unwrap_or(0.0) > 0.0,
            "arrivals beyond the 1-deep queue must still fail fast as bulkhead-full"
        );
        assert!(
            failures.get(&FailureReason::BulkheadFull).copied().unwrap_or(0) > 0,
            "the engine's own failuresByReason must count the overflow rejections too"
        );
    }
}
