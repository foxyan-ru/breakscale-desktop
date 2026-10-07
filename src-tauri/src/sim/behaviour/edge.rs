//! Edge-protection component behaviours: cdn, ratelimiter, breaker. Port of
//! `src/sim/behaviour-edge.ts`.
//!
//! These three are the layer that sits in FRONT of a system and decides
//! what ever reaches it. Grouped in their own module because they share a
//! theme a student can name -- "how do I stop load before it hurts me".
//!
//! A note on determinism, which constrains all three. `advance(dt)` is
//! driven by wall-clock frames, so dt varies from frame to frame. Anything
//! that decides "has enough time passed?" therefore has to compare
//! *simulated timestamps*, never accumulate per-tick deltas -- otherwise
//! the same seed produces different results at a different frame rate.
//! That is why the token bucket refills against `now - last_refill_ms` and
//! the breaker's state transitions are evaluated lazily when a request
//! arrives, rather than from `on_tick`.
//!
//! ## Deviation from the porting brief's summary, caught against the source
//!
//! `src/sim/types.ts`'s `NodeStats` doc comment (carried into
//! `sim::types::NodeStats` here) lists `cdn` among the kinds with an
//! instance model ("service, worker, db, cache, lb, cdn -- ONE UNIT = ONE
//! INSTANCE"). The actual `cdn` behaviour object in
//! `behaviour-edge.ts`, read in full below, declares neither
//! `instanceModel` nor `scaleField` -- unlike `service`/`cache`/`worker`/`lb`,
//! which all declare `instanceModel: 'slots'` explicitly. This port follows
//! the actual behaviour source (no instance model for `cdn`), not the
//! summary in the types doc comment; see the porting agent's final report
//! for this flagged as a cross-file inconsistency in the TS source itself.

use crate::sim::behaviour::{
    clamp01, AdmitAction, ComponentBehaviour, CompleteAction, Ext, PumpMode,
};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqLike};
use crate::sim::types::{BreakerState, EdgeState, FailureReason, NodeKind, NodeStats, SimEdge};

/* ------------------------------------------------------------------ *
 * cdn
 * ------------------------------------------------------------------ */

/// A content delivery network edge cache.
///
/// Mechanically this is a cache that sits geographically in front of
/// everything: `service_ms` is tiny (you are talking to a nearby PoP, not
/// the origin) and `hit_rate` is high. The teaching point is entirely in
/// what the numbers do to the rest of the topology -- at hit_rate 0.9 the
/// origin sees a tenth of the offered load, so the CDN is doing 90% of the
/// work of ten extra servers for none of the cost.
///
/// The one behavioural difference from `cache`: a miss here is expensive,
/// because the origin is far away. That falls out naturally rather than
/// being special-cased -- the miss goes downstream and pays whatever the
/// origin path costs, including any edge latency wired onto the link.
pub struct CdnBehaviour;

impl ComponentBehaviour for CdnBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Cdn
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

    fn on_service_complete(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        // One RNG draw, unconditionally, in a fixed position. Drawing only
        // on some paths would make the stream depend on config and break
        // replay.
        if ctx.roll() < clamp01(state.config().hit_rate) {
            ctx.count_hit(state);
            return CompleteAction::Complete;
        }
        ctx.count_miss(state);
        // A miss must be fetched from origin. Book it separately from the
        // generic miss counter so the UI can show origin load in
        // requests/sec -- "what actually reached your servers" is the
        // number worth teaching.
        ctx.count_custom(state, "originFetch", 1.0);
        // Nothing behind the CDN: it still answers, it just cannot
        // demonstrate the origin-offload lesson.
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
        stats.origin_fetch_rate = Some(ctx.counter_rate(state, "originFetch"));
    }
}

/* ------------------------------------------------------------------ *
 * ratelimiter
 * ------------------------------------------------------------------ */

/// Token bucket state.
struct BucketState {
    /// Tokens available right now. Fractional: refill is continuous.
    tokens: f64,
    /// Simulated time the bucket was last refilled, in ms.
    last_refill_ms: f64,
    /// Bucket size the tokens were last capped against, to detect a
    /// config change.
    last_burst: f64,
}

/// `rate_limit_rps`, defaulted and floored to something sane.
fn limit_rate(state: &dyn NodeStateLike) -> f64 {
    match state.config().rate_limit_rps {
        Some(r) if r > 0.0 => r,
        _ => 0.0,
    }
}

/// Bucket size in tokens. Defaults to one second's worth of rate.
fn limit_burst(state: &dyn NodeStateLike) -> f64 {
    if let Some(b) = state.config().burst {
        if b > 0.0 {
            return b;
        }
    }
    let r = limit_rate(state);
    if r > 0.0 {
        r
    } else {
        1.0
    }
}

/// Bring the bucket up to date with the current simulated time.
///
/// Refill is computed from elapsed simulated ms, NOT accumulated per tick.
/// This is what makes the limiter exact: over T seconds it grants exactly
/// rate*T tokens regardless of how many `advance()` calls that T was split
/// across, so a 16ms frame and a 100ms frame produce identical admissions.
/// A per-tick `tokens += rate * dt` would drift with frame rate and,
/// worse, would make the sim non-deterministic against a variable frame
/// clock.
fn refill(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, b: &mut BucketState) {
    let burst = limit_burst(state);

    // The student moved the burst slider. Re-cap rather than letting a
    // bucket filled under the old ceiling stay oversized forever.
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

    let rate = limit_rate(state);
    if rate <= 0.0 {
        return;
    }

    b.tokens += (elapsed / 1000.0) * rate;
    if b.tokens > burst {
        b.tokens = burst;
    }
}

/// The bucket level as of `ctx.now()`, WITHOUT mutating it.
///
/// Same arithmetic as `refill()`, used for reporting only. Kept separate
/// so there is no way for a snapshot to accidentally advance the bucket:
/// the engine may be snapshotted many times between two arrivals, and each
/// of those must be a pure read.
fn projected_tokens(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike, b: &BucketState) -> f64 {
    let burst = limit_burst(state);
    let mut tokens = if b.tokens > burst { burst } else { b.tokens };
    let rate = limit_rate(state);
    let elapsed = ctx.now() - b.last_refill_ms;
    if rate > 0.0 && elapsed > 0.0 {
        tokens += (elapsed / 1000.0) * rate;
        if tokens > burst {
            tokens = burst;
        }
    }
    tokens
}

/// A token bucket in front of whatever it fronts.
///
/// Every request costs one token. A request that finds the bucket empty is
/// refused immediately as 'throttled' -- it never queues, never occupies a
/// slot, and never touches downstream. That cheapness is the whole point:
/// the lesson is that refusing 400 rps at the door costs almost nothing,
/// whereas accepting them and letting them queue is what actually takes
/// the system down. A student can watch the same offered load produce a
/// healthy service with a limiter and a collapsed one without.
///
/// Zero RNG draws, so a limiter never perturbs the random stream.
pub struct RateLimiterBehaviour;

impl ComponentBehaviour for RateLimiterBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::RateLimiter
    }
    // A limiter is a doorman, not a server: it holds no slots and its
    // utilisation would be meaningless, so it reports none.
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
        // A fresh limiter starts FULL, so an idle system absorbs one full
        // burst instantly -- which is exactly the behaviour a token
        // bucket is chosen for, and the first thing worth demonstrating.
        let burst = limit_burst(state);
        Some(Box::new(BucketState {
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
            .and_then(|e| e.downcast_mut::<BucketState>())
            .expect("ratelimiter ext");
        refill(ctx, state, b);

        // rate <= 0 means "no limit configured": pass everything rather
        // than silently black-holing the topology a student just wired
        // up.
        if limit_rate(state) <= 0.0 {
            ctx.count_custom(state, "admitted", 1.0);
            return AdmitAction::Passthru;
        }

        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            ctx.count_custom(state, "admitted", 1.0);
            return AdmitAction::Passthru;
        }

        ctx.count_custom(state, "throttled", 1.0);
        ctx.reject(state, req, FailureReason::Throttled);
        AdmitAction::Handled
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        stats.admitted_rate = Some(ctx.counter_rate(state, "admitted"));
        stats.throttled_rate = Some(ctx.counter_rate(state, "throttled"));
        // Report the bucket as of NOW, not as of the last arrival. Refill
        // is lazy -- b.tokens is only brought up to date when a request
        // turns up -- so reading the stored value straight out would show
        // an idle limiter's bucket frozen instead of visibly filling, and
        // would misreport by up to rate*(now - last_refill) whenever
        // traffic is sparse. Computed here rather than by calling
        // refill(), because snapshotting must not mutate simulation
        // state: taking a snapshot has to be observation only.
        stats.tokens = Some(match ext.as_ref().and_then(|e| e.downcast_ref::<BucketState>()) {
            Some(b) => projected_tokens(ctx, state, b),
            None => 0.0,
        });
    }
}

/* ------------------------------------------------------------------ *
 * breaker
 * ------------------------------------------------------------------ */

/// How many buckets the trailing error window is split into.
const BREAKER_BUCKETS: usize = 10;

/// Minimum downstream calls in the window before the breaker will trip.
///
/// Without this a single unlucky failure at t=0 is a 100% error rate and
/// the circuit trips on a sample of one. Real breakers all have this
/// guard; it is also what stops the sim looking broken the instant a
/// student adds a node with a small `error_rate`.
const BREAKER_MIN_SAMPLES: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerPhase {
    Closed,
    Open,
    HalfOpen,
}

/// Circuit breaker state.
struct BreakerExt {
    phase: BreakerPhase,
    /// Simulated time the circuit last tripped OPEN, in ms.
    opened_at_ms: f64,
    /// Probes already admitted in the current HALF-OPEN attempt.
    probes_sent: i64,
    /// Probes that came back successful in the current HALF-OPEN attempt.
    probes_ok: i64,
    /// Rolling window of downstream outcomes: total calls per bucket.
    total: [f64; BREAKER_BUCKETS],
    /// Rolling window of downstream outcomes: failed calls per bucket.
    failed: [f64; BREAKER_BUCKETS],
    /// Bucket index stamps, so a stale bucket is zeroed rather than summed.
    stamps: [f64; BREAKER_BUCKETS],
    /// Times the circuit has tripped OPEN since sim start.
    trips: f64,
}

fn cfg_error_threshold(state: &dyn NodeStateLike) -> f64 {
    match state.config().error_threshold {
        Some(v) => clamp01(v),
        None => 0.5,
    }
}
fn cfg_window_ms(state: &dyn NodeStateLike) -> f64 {
    match state.config().window_ms {
        Some(v) if v > 0.0 => v,
        _ => 5000.0,
    }
}
fn cfg_open_ms(state: &dyn NodeStateLike) -> f64 {
    match state.config().open_ms {
        Some(v) if v > 0.0 => v,
        _ => 3000.0,
    }
}
fn cfg_half_open_probes(state: &dyn NodeStateLike) -> i64 {
    match state.config().half_open_probes {
        Some(v) if v >= 1.0 => v.floor() as i64,
        _ => 3,
    }
}

/// Which bucket of the trailing window a timestamp falls in.
fn bucket_of(now: f64, window_ms: f64) -> i64 {
    (now / (window_ms / BREAKER_BUCKETS as f64)).floor() as i64
}

/// Record one downstream outcome into the trailing window.
///
/// Bucketed rather than a list of timestamps so memory is O(1) per
/// breaker however much traffic flows through it -- a breaker in front of
/// a 10k rps service must not accumulate 10k entries per second.
fn record_outcome(b: &mut BreakerExt, now: f64, window_ms: f64, ok: bool) {
    let stamp = bucket_of(now, window_ms);
    let idx = (((stamp % BREAKER_BUCKETS as i64) + BREAKER_BUCKETS as i64) % BREAKER_BUCKETS as i64) as usize;
    if b.stamps[idx] != stamp as f64 {
        // This slot belongs to an older revolution of the window: reset
        // it rather than adding to counts from `window_ms` ago.
        b.stamps[idx] = stamp as f64;
        b.total[idx] = 0.0;
        b.failed[idx] = 0.0;
    }
    b.total[idx] += 1.0;
    if !ok {
        b.failed[idx] += 1.0;
    }
}

struct WindowStats {
    rate: f64,
    total: f64,
}

/// Failed/total over the trailing window, plus the total, for the trip
/// test.
fn window_stats(b: &BreakerExt, now: f64, window_ms: f64) -> WindowStats {
    let current = bucket_of(now, window_ms);
    let mut total = 0.0;
    let mut failed = 0.0;
    for i in 0..BREAKER_BUCKETS {
        let age = current as f64 - b.stamps[i];
        // age === 0 is the bucket still filling and IS counted here,
        // unlike the engine's display rates: a breaker must react to what
        // is happening right now, not wait a bucket for it to become
        // official.
        if age >= 0.0 && age < BREAKER_BUCKETS as f64 {
            total += b.total[i];
            failed += b.failed[i];
        }
    }
    WindowStats {
        rate: if total > 0.0 { failed / total } else { 0.0 },
        total,
    }
}

/// Clear the window. Used whenever the circuit changes phase.
fn clear_window(b: &mut BreakerExt) {
    b.total = [0.0; BREAKER_BUCKETS];
    b.failed = [0.0; BREAKER_BUCKETS];
    b.stamps = [-1.0; BREAKER_BUCKETS];
}

/// A circuit breaker wrapping its downstream.
///
/// CLOSED     -> pass everything, watch the downstream error rate.
/// OPEN       -> fail fast as 'rejected'. Downstream receives NOTHING.
/// HALF-OPEN  -> admit exactly `half_open_probes` requests. All succeed ->
///               CLOSED. Any one fails -> straight back to OPEN.
///
/// The teaching point is what OPEN does for the *dependency*: a service
/// that is falling over because it is overwhelmed cannot recover while
/// its caller keeps hammering it. Cutting the traffic to zero for
/// `open_ms` is what gives it room to come back. A student can watch the
/// downstream's queue drain during the OPEN window and see it recover --
/// and then remove the breaker and watch it never recover at all.
///
/// State transitions are evaluated lazily, at admission, by comparing
/// simulated timestamps. Doing this from `on_tick` would tie the
/// transition instant to the frame cadence and lose determinism.
pub struct BreakerBehaviour;

impl ComponentBehaviour for BreakerBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Breaker
    }
    // Like the limiter, a pass-through gate rather than a server.
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
    // The whole point: it needs to see how its dependency is doing.
    fn observes_outcome(&self) -> bool {
        true
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(BreakerExt {
            phase: BreakerPhase::Closed,
            opened_at_ms: 0.0,
            probes_sent: 0,
            probes_ok: 0,
            total: [0.0; BREAKER_BUCKETS],
            failed: [0.0; BREAKER_BUCKETS],
            stamps: [-1.0; BREAKER_BUCKETS],
            trips: 0.0,
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
            .and_then(|e| e.downcast_mut::<BreakerExt>())
            .expect("breaker ext");

        // Lazy transition: OPEN has served its time, start probing.
        if b.phase == BreakerPhase::Open && ctx.now() - b.opened_at_ms >= cfg_open_ms(state) {
            b.phase = BreakerPhase::HalfOpen;
            b.probes_sent = 0;
            b.probes_ok = 0;
        }

        if b.phase == BreakerPhase::Open {
            // Fail fast. No downstream call is made at all -- this is the
            // entire value of the component, and it is why the check
            // lives in on_admit rather than in route(): by the time
            // route() runs the request has already been admitted and
            // costed.
            ctx.count_custom(state, "rejected", 1.0);
            ctx.reject(state, req, FailureReason::Rejected);
            return AdmitAction::Handled;
        }

        if b.phase == BreakerPhase::HalfOpen {
            // Only a fixed number of probes are allowed through; everything
            // else is still refused, so a recovering dependency gets a
            // trickle rather than the full firehose the instant the timer
            // expires.
            if b.probes_sent >= cfg_half_open_probes(state) {
                ctx.count_custom(state, "rejected", 1.0);
                ctx.reject(state, req, FailureReason::Rejected);
                return AdmitAction::Handled;
            }
            b.probes_sent += 1;
        }

        ctx.count_custom(state, "admitted", 1.0);
        AdmitAction::Passthru
    }

    fn on_downstream_result(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        ok: bool,
        _reason: FailureReason,
        ext: &mut Ext,
    ) {
        let b = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BreakerExt>())
            .expect("breaker ext");

        if b.phase == BreakerPhase::HalfOpen {
            if !ok {
                // One failed probe is enough: the dependency is still
                // sick. Back to OPEN for another full open_ms.
                b.phase = BreakerPhase::Open;
                b.opened_at_ms = ctx.now();
                b.trips += 1.0;
                clear_window(b);
                return;
            }
            b.probes_ok += 1;
            if b.probes_ok >= cfg_half_open_probes(state) {
                // Every probe came back clean: the dependency is healthy
                // again.
                b.phase = BreakerPhase::Closed;
                clear_window(b);
            }
            return;
        }

        if b.phase != BreakerPhase::Closed {
            return;
        }

        let window_ms = cfg_window_ms(state);
        record_outcome(b, ctx.now(), window_ms, ok);

        let ws = window_stats(b, ctx.now(), window_ms);
        if ws.total >= BREAKER_MIN_SAMPLES && ws.rate > cfg_error_threshold(state) {
            b.phase = BreakerPhase::Open;
            b.opened_at_ms = ctx.now();
            b.trips += 1.0;
            // Drop the evidence that tripped it. Keeping it would make the
            // breaker re-trip instantly on its first probe failure's stale
            // company.
            clear_window(b);
        }
    }

    /// An OPEN breaker's downstream edge is carrying nothing, and it is
    /// carrying nothing for a REASON -- the component is doing its job.
    /// Reporting it as 'blocked' is what lets the canvas draw that wire
    /// severed instead of merely quiet, which is the difference between a
    /// student seeing the breaker work and seeing a link that looks
    /// identical to an idle one.
    ///
    /// HALF-OPEN is deliberately NOT blocked: a trickle of probes really
    /// is crossing, and drawing it cut would contradict the traffic on it.
    fn edge_state_for(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _edge: &SimEdge,
        _index: usize,
        ext: &mut Ext,
    ) -> Option<EdgeState> {
        let b = ext.as_mut().and_then(|e| e.downcast_mut::<BreakerExt>())?;
        // Same lazy transition decorate_stats reports, so the wire and the
        // badge never disagree: a circuit whose open_ms has elapsed is
        // already probing.
        let open = b.phase == BreakerPhase::Open && ctx.now() - b.opened_at_ms < cfg_open_ms(state);
        if open {
            Some(EdgeState::Blocked)
        } else {
            None
        }
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let b = match ext.as_mut().and_then(|e| e.downcast_mut::<BreakerExt>()) {
            Some(b) => b,
            None => return,
        };
        // Report the phase the breaker would act on if a request arrived
        // right now, so the UI never shows OPEN for a circuit whose timer
        // has expired.
        let phase = if b.phase == BreakerPhase::Open && ctx.now() - b.opened_at_ms >= cfg_open_ms(state) {
            BreakerPhase::HalfOpen
        } else {
            b.phase
        };
        stats.breaker_state = Some(match phase {
            BreakerPhase::Closed => BreakerState::Closed,
            BreakerPhase::Open => BreakerState::Open,
            BreakerPhase::HalfOpen => BreakerState::HalfOpen,
        });
        stats.breaker_error_rate = Some(window_stats(b, ctx.now(), cfg_window_ms(state)).rate);
        stats.rejected_rate = Some(ctx.counter_rate(state, "rejected"));
        stats.breaker_trips = Some(b.trips);
        // Countdown to the half-open probe, on exactly the phase that has
        // one.
        stats.open_remaining_ms = Some(if phase == BreakerPhase::Open {
            (cfg_open_ms(state) - (ctx.now() - b.opened_at_ms)).max(0.0)
        } else {
            0.0
        });
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static CDN: CdnBehaviour = CdnBehaviour;
static RATELIMITER: RateLimiterBehaviour = RateLimiterBehaviour;
static BREAKER: BreakerBehaviour = BreakerBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::Cdn, &CDN),
    (NodeKind::RateLimiter, &RATELIMITER),
    (NodeKind::Breaker, &BREAKER),
];
