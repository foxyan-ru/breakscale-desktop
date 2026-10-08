//! Control-plane component behaviours: autoscaler, region. Port of
//! `src/sim/behaviour-control.ts`.
//!
//! These two are different in kind from everything else in the registry. A
//! service, a cache, a queue are all things a request passes THROUGH. These
//! are things that act ON the topology: an autoscaler writes another node's
//! capacity, a region node decides which downstream is allowed to exist
//! right now. They are the control plane, and grouping them says so.
//!
//! A note on determinism, which shapes both. `advance(dt)` is driven by
//! wall-clock frames, so dt varies frame to frame. Anything asking "has
//! enough time passed?" must compare *simulated timestamps* rather than
//! accumulate per-tick deltas, or the same seed yields different results at
//! a different frame rate. That is why the autoscaler stores
//! `last_decision_ms` and `warmup_due_ms` as absolute simulated times and
//! ignores its own `dt_ms` argument entirely, and why the region node's
//! failover deadline is an absolute time evaluated lazily when a request
//! arrives.
//!
//! Neither behaviour draws from the RNG. A controller that consumed random
//! numbers would shift the shared random stream for every other node
//! depending on how often it happened to act, which is exactly the kind of
//! coupling that makes a "deterministic" simulator quietly non-reproducible.

use crate::sim::behaviour::{
    clamp01, AdmitAction, ComponentBehaviour, Ext, PumpMode, RouteMode,
};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqLike};
use crate::sim::types::{EdgeState, FailureReason, NodeKind, NodeStats, ScalePhase, SimEdge};

/* ------------------------------------------------------------------ *
 * autoscaler
 * ------------------------------------------------------------------ */

/// Utilisation band below `target_util` inside which the controller does
/// nothing. Without it the controller flaps by one step every cooldown
/// while utilisation sits naturally just under its setpoint.
const DEAD_BAND: f64 = 0.1;

/// Fallbacks for the optional config knobs, so an unset field is never NaN.
///
/// The timings are chosen so a student dragging the load slider sees the
/// whole story inside a few seconds of watching -- see the note on
/// `warmup_ms` in presets.ts for the measured timeline.
const DEFAULT_TARGET_UTIL: f64 = 0.7;
const DEFAULT_MIN_INSTANCES: f64 = 1.0;
const DEFAULT_MAX_INSTANCES: f64 = 64.0;
const DEFAULT_COOLDOWN_MS: f64 = 5000.0;
const DEFAULT_STEP_PCT: f64 = 0.5;
const DEFAULT_WARMUP_MS: f64 = 0.0;

/// One optional knob, or its fallback.
///
/// WHY: `behaviour-control.ts:57-70` (upstream `ea11c5b3`, PR #64). None of
/// these six knobs is among the nine config numbers the web's `isTopology`
/// validates, so a shared link, a `.breakscale` file or a restored session
/// can hand the controller a config value that is `Some(NaN)` or
/// `Some(infinity)`. A plain `.unwrap_or(DEFAULT)` only replaces an ABSENT
/// field, so such a value sailed through -- and because Rust's
/// `f64::max`/`f64::min` quietly ignore a NaN operand (unlike JS's
/// `Math.max`, which propagates it), the bug here was not a crash but a
/// silent wrong answer: a NaN `max_capacity` made `max_inst` collapse to
/// `min_inst` (`NaN.floor().max(min_inst)` returns `min_inst`, not NaN),
/// freezing the fleet at its floor forever, exactly like the upstream bug
/// report ("the knob left the fleet at one instance"). Guarding every knob
/// here, before any arithmetic runs, is what actually restores the
/// documented fallback instead of relying on each call site's `.max`/`.min`
/// happening to fail safe.
fn knob(v: Option<f64>, fallback: f64) -> f64 {
    match v {
        Some(x) if x.is_finite() => x,
        _ => fallback,
    }
}

struct AutoscalerState {
    /// Simulated time of the last decision; `-Infinity` means "never
    /// decided".
    last_decision_ms: f64,
    /// Instance count the controller believes the watched node should
    /// have. Tracked separately from the node's live count because during
    /// a warm-up the two deliberately disagree, and that gap is the thing
    /// worth showing.
    target_instances: f64,
    /// Simulated time at which a pending scale-UP lands, or -1 when
    /// nothing is warming up. Holding a granted decision here instead of
    /// applying it at once is the entire point of this component.
    warmup_due_ms: f64,
    /// Instance count to write when the warm-up completes.
    pending_instances: f64,
    /// Node being driven, resolved from the controller's first out edge.
    watched_id: String,
    /// Simulated time the controller may take its FIRST decision about the
    /// current target, or -1 once it has taken one.
    ///
    /// Utilisation is a smoothed average, so at t=0 -- and for a moment
    /// after the controller is pointed at a new node -- it reads near zero
    /// no matter how loaded that node actually is. Acting on that cold
    /// reading makes the controller's opening move a scale-DOWN to
    /// min_capacity on a node that is in fact saturated, which then takes
    /// several warm-ups to undo. A real autoscaler will not act until it
    /// has a full metric window, and this is that rule: observe for one
    /// cooldown, then decide.
    observe_until_ms: f64,
}

/// A fleet controller: it adds and removes INSTANCES.
///
/// It sits BESIDE the node it scales rather than in front of it, joined to
/// it by a CONTROL EDGE that carries no requests -- the engine keeps
/// control edges out of routing entirely, so a request can never be
/// dispatched down one. Once per `cooldown_ms` it compares the watched
/// node's utilisation against `target_util` and steps its instance count by
/// `scale_step_pct`, clamped to `[min_capacity, max_capacity]` instances.
///
/// WHAT IT WRITES, AND WHY THAT CHANGED. It used to write the watched
/// node's `capacity` -- its thread count. That was numerically fine and
/// pedagogically useless: "traffic went up so we added threads" is not the
/// sentence anyone means, and because nothing on the canvas counted
/// threads, the controller appeared to do nothing at all. It now writes
/// `instances`, so the drawn stack of machines grows by exactly the number
/// of machines it decided to add. `capacity` -- how big one machine is --
/// belongs to the student and the controller never touches it.
///
/// The teaching point is the lag. A scale-UP does not take effect until
/// `warmup_ms` after the decision -- machines boot, images pull, JITs warm
/// -- so a student who steps the load up watches utilisation pin at 1.0,
/// and the queue build behind it, for a visible interval before capacity
/// answers. And because the controller reacts to a utilisation its own
/// past decisions caused, a short cooldown against a long warmup makes it
/// overshoot and oscillate. That is a real failure mode, and being able to
/// reproduce it on purpose is worth more than a controller that always
/// behaves.
///
/// Scale-DOWN is immediate, as it is in reality: releasing a machine needs
/// no warm-up. This asymmetry is deliberate and is itself part of why
/// autoscalers oscillate downward faster than they recover.
pub struct AutoscalerBehaviour;

impl ComponentBehaviour for AutoscalerBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Autoscaler
    }
    // A controller is not in the request path: it serves nothing, holds
    // nothing, and reports no utilisation of its own. `serves_requests:
    // false` is what keeps its (permanently zero) slot count out of the
    // utilisation integration, so it renders as a control box rather than
    // an idle server.
    fn serves_requests(&self) -> bool {
        false
    }
    fn generates_load(&self) -> bool {
        false
    }
    // Every edge out of an autoscaler names the node it drives, never a
    // hop. The engine reads this at wiring time and leaves those edges out
    // of the routing set, so "requests are never routed down a control
    // edge" is a structural property rather than something enforced
    // request by request.
    fn controls_target(&self) -> bool {
        true
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
        false
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(AutoscalerState {
            last_decision_ms: f64::NEG_INFINITY,
            target_instances: 0.0,
            warmup_due_ms: -1.0,
            pending_instances: 0.0,
            watched_id: String::new(),
            observe_until_ms: -1.0,
        }))
    }

    /// Traffic reaching an autoscaler means the student wired requests
    /// INTO the controller -- which now takes deliberate effort, since the
    /// edges the controller itself owns are control edges and carry
    /// nothing. Refusing explicitly is far more legible than serving it,
    /// which would make the controller look like a hop that mysteriously
    /// adds latency.
    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        ctx.reject(state, req, FailureReason::NoRoute);
        AdmitAction::Handled
    }

    /// `dt_ms` is deliberately unused: every decision compares absolute
    /// simulated timestamps, so the controller behaves identically at any
    /// frame rate.
    fn on_tick(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _dt_ms: f64,
        ext: &mut Ext,
    ) {
        let st = match ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<AutoscalerState>())
        {
            Some(s) => s,
            None => return,
        };

        // The watched node is whatever this controller points at,
        // re-resolved each tick so rewiring the edge on the canvas
        // retargets it live. Its target arrives on a CONTROL edge, which
        // is a separate list from `out` precisely because it is not a
        // traffic path.
        let watched = ctx.control_target_of(state);
        if watched != st.watched_id {
            st.watched_id = watched.clone();
            // A new target invalidates every decision made about the old
            // one.
            st.warmup_due_ms = -1.0;
            st.target_instances = 0.0;
            st.last_decision_ms = f64::NEG_INFINITY;
            // ...and starts a fresh observation window, because the new
            // node's smoothed utilisation is not a signal yet.
            st.observe_until_ms =
                ctx.now() + knob(state.config().cooldown_ms, DEFAULT_COOLDOWN_MS).max(0.0);
        }
        if watched.is_empty() {
            return;
        }

        // How many instances the target is running. `None` means the
        // target is a kind with no fleet to move (a queue, a breaker): the
        // controller stays put rather than ramping a number nothing reads.
        let live_instances = match ctx.scale_of(&watched) {
            Some(v) => v,
            None => return,
        };

        // First tick: adopt what the node already has, so the controller
        // never yanks the fleet to a default the student did not ask for.
        if st.target_instances == 0.0 {
            st.target_instances = live_instances;
        }

        // A pending scale-up whose warm-up elapsed becomes real capacity
        // now.
        if st.warmup_due_ms >= 0.0 && ctx.now() >= st.warmup_due_ms {
            st.warmup_due_ms = -1.0;
            st.target_instances = st.pending_instances;
            ctx.set_scale(&watched, st.pending_instances);
        }

        // A scale-up already booked and still warming up blocks further
        // decisions.
        //
        // Without this the controller re-derives the same "scale up"
        // verdict on every cooldown -- utilisation is still high, because
        // the capacity it ordered has not arrived yet -- and each
        // re-decision pushes warmup_due_ms forward by another full warmup.
        // The pending capacity then never lands at all, and the node stays
        // at its original size forever. A real autoscaler has exactly this
        // problem and solves it the same way: it does not issue a second
        // order while the first is still being fulfilled.
        if st.warmup_due_ms >= 0.0 {
            return;
        }

        let cfg = state.config();
        let cooldown = knob(cfg.cooldown_ms, DEFAULT_COOLDOWN_MS).max(0.0);

        // Hold off until the watched node's utilisation is a real
        // measurement rather than an average still climbing out of its
        // initial zero.
        if st.observe_until_ms >= 0.0 {
            if ctx.now() < st.observe_until_ms {
                return;
            }
            st.observe_until_ms = -1.0;
        }

        if ctx.now() - st.last_decision_ms < cooldown {
            return;
        }

        // A crashed node reports whatever utilisation it held when it
        // died, which is not a signal. Acting on it would either spin
        // capacity up against a dead box or scale it away just before it
        // recovers.
        if ctx.is_crashed(&watched) {
            return;
        }

        let util = match ctx.utilization_of(&watched) {
            Some(v) => v,
            None => return,
        };

        // min_capacity/max_capacity keep their field names for
        // compatibility, but the unit they bound is now INSTANCES -- the
        // fleet size, not the thread count. For every topology written
        // before instances existed the two readings coincide, because
        // those nodes run exactly one instance.
        let min_inst = knob(cfg.min_capacity, DEFAULT_MIN_INSTANCES)
            .floor()
            .max(1.0);
        let max_inst = knob(cfg.max_capacity, DEFAULT_MAX_INSTANCES)
            .floor()
            .max(min_inst);
        let target = clamp01(knob(cfg.target_util, DEFAULT_TARGET_UTIL));
        let step = knob(cfg.scale_step_pct, DEFAULT_STEP_PCT).max(0.01);
        // An instance count is integral, so a step must move at least one
        // machine: a small percentage of a small fleet would otherwise
        // round to a permanent no-op and the controller would silently do
        // nothing forever.
        let delta = (st.target_instances * step).round().max(1.0);

        // The fleet size that would put utilisation exactly on the
        // setpoint, given that (util * current_instances)
        // instance-equivalents are busy right now. Used to bound a step so
        // the controller cannot overshoot THROUGH its own setpoint: a
        // fixed percentage step alone will happily take a node from 40%
        // utilisation straight into saturation, and then have to scale
        // back up, which is self-inflicted oscillation on top of the real
        // kind.
        //
        // Utilisation is dimensionless -- busy slots over total slots --
        // so this arithmetic is identical whether the fleet is counted in
        // machines or in threads, which is why moving the controller onto
        // instances changed the unit without changing the control law.
        let busy_instances = util * st.target_instances;
        let ideal_instances = (busy_instances / if target > 0.0 { target } else { 1.0 })
            .ceil()
            .max(1.0);

        let mut want = st.target_instances;
        if util > target {
            // Scale up by a step, but never past what the setpoint
            // actually needs.
            want = (st.target_instances + delta).min(ideal_instances.max(st.target_instances + 1.0));
        } else if util < target - DEAD_BAND {
            // Scale down by a step, but never below what the setpoint
            // needs. The asymmetry with scale-up is deliberate: shedding
            // too much capacity causes an immediate outage, while adding
            // too much only costs money.
            want = (st.target_instances - delta).max(ideal_instances);
        }

        want = if want < min_inst {
            min_inst
        } else if want > max_inst {
            max_inst
        } else {
            want
        };
        if want == st.target_instances {
            return;
        }

        st.last_decision_ms = ctx.now();

        if want < st.target_instances {
            // Removing instances is immediate; nothing has to boot. Any
            // pending scale-up is cancelled outright rather than left
            // half-set, so a later read of pending_instances cannot
            // resurrect a decision this one reversed.
            st.target_instances = want;
            st.warmup_due_ms = -1.0;
            st.pending_instances = want;
            ctx.set_scale(&watched, want);
            return;
        }

        let warmup = knob(cfg.warmup_ms, DEFAULT_WARMUP_MS).max(0.0);
        if warmup == 0.0 {
            st.target_instances = want;
            ctx.set_scale(&watched, want);
            return;
        }
        // Book the decision. The machines do not exist yet -- this is the
        // lag.
        st.pending_instances = want;
        st.warmup_due_ms = ctx.now() + warmup;
    }

    /// Publish everything needed to say, in one sentence, what this
    /// controller is doing and why. A student should never have to open
    /// the config panel to find out that the box is sitting in a cooldown.
    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let st = match ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<AutoscalerState>())
        {
            Some(s) => s,
            None => return,
        };
        let scaling = st.warmup_due_ms >= 0.0;
        // `scale_of` returns None for a watched kind with no fleet concept
        // (e.g. a blob store) — distinct from a watched kind legitimately
        // sitting at 0 instances. Upstream `7e285598` (PR #26) surfaces that
        // distinction as `watchedUnscalable` so the Inspector can say
        // "adding servers is not the fix" instead of implying a stuck scale.
        let watched_scale = if !st.watched_id.is_empty() {
            ctx.scale_of(&st.watched_id)
        } else {
            None
        };
        let live = watched_scale.unwrap_or(0.0);
        let watched_unscalable = !st.watched_id.is_empty() && watched_scale.is_none();
        // While warming up, report the fleet size being BOOKED rather than
        // the one in force: paired with watched_instances, the gap between
        // the two numbers is the visible form of the lag this component
        // exists to teach.
        let wanted = if scaling {
            st.pending_instances
        } else {
            st.target_instances
        };

        stats.watched_id = Some(st.watched_id.clone());
        stats.target_instances = Some(wanted);
        stats.watched_instances = Some(live);
        stats.watched_unscalable = Some(watched_unscalable);
        stats.pending_instances = Some(if scaling { (wanted - live).max(0.0) } else { 0.0 });
        stats.scaling = Some(scaling);
        stats.watched_util = Some(if !st.watched_id.is_empty() {
            ctx.utilization_of(&st.watched_id).unwrap_or(0.0)
        } else {
            0.0
        });
        stats.setpoint = Some(clamp01(knob(
            state.config().target_util,
            DEFAULT_TARGET_UTIL,
        )));

        // Which of the three waits it is in, and how much of it is left.
        // Resolved in the same order on_tick() applies them, so the label
        // never claims the controller is free to act when the next tick
        // will find it blocked.
        let cooldown = knob(state.config().cooldown_ms, DEFAULT_COOLDOWN_MS).max(0.0);
        if scaling {
            stats.scale_phase = Some(ScalePhase::Warming);
            stats.phase_remaining_ms = Some((st.warmup_due_ms - ctx.now()).max(0.0));
        } else if st.observe_until_ms >= 0.0 && ctx.now() < st.observe_until_ms {
            stats.scale_phase = Some(ScalePhase::Observing);
            stats.phase_remaining_ms = Some((st.observe_until_ms - ctx.now()).max(0.0));
        } else if ctx.now() - st.last_decision_ms < cooldown {
            stats.scale_phase = Some(ScalePhase::Cooldown);
            stats.phase_remaining_ms = Some((cooldown - (ctx.now() - st.last_decision_ms)).max(0.0));
        } else {
            stats.scale_phase = Some(ScalePhase::Steady);
            stats.phase_remaining_ms = Some(0.0);
        }
    }
}

/* ------------------------------------------------------------------ *
 * region
 * ------------------------------------------------------------------ */

struct RegionState {
    /// Region index currently serving, 0-based. -1 until first use.
    active: i64,
    /// Simulated time an in-progress failover completes, or -1 when
    /// traffic is flowing normally. While set, every request to this node
    /// is refused.
    failover_due_ms: f64,
    /// Region the in-progress failover is moving to.
    failover_target: i64,
    /// The value of `config.active_region` the last time this behaviour
    /// looked.
    ///
    /// `config.active_region` is the student's INTENT, not the live state:
    /// once traffic has failed over, the two legitimately disagree, and
    /// the config still names the region that died. Tracking the
    /// last-seen value is what separates "the student moved the slider"
    /// (follow it) from "the config merely still differs because we
    /// failed away from it" (ignore it). Without this the adopt step
    /// drags `active` back onto the dead region on the very next request
    /// and failover never completes.
    last_configured: i64,
}

/// How many of this node's out edges count as regions.
fn region_count(state: &dyn NodeStateLike) -> i64 {
    let out_len = state.out().len() as f64;
    let declared = state.config().regions.unwrap_or(out_len).floor();
    declared.min(out_len).max(1.0) as i64
}

/// Is region `i` usable right now?
///
/// A region is unreachable either because the node serving it is crashed
/// or because the link to it is partitioned. Both are injected failures,
/// which is what lets a student cause a regional outage without deleting
/// anything.
fn region_healthy(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike, i: i64) -> bool {
    if i < 0 {
        return false;
    }
    let edge = match state.out().get(i as usize) {
        Some(e) => e,
        None => return false,
    };
    if ctx.is_edge_cut(&edge.id) {
        return false;
    }
    !ctx.is_crashed(&edge.to)
}

/// A multi-region failover switch.
///
/// Deliberately the simplest construct that teaches the idea: outgoing
/// edge i is region i, exactly one region serves traffic at a time, and
/// when that region becomes unreachable traffic moves to the next healthy
/// one in a fixed scan order.
///
/// The part worth seeing is that failover is not free. Detection,
/// DNS/anycast convergence and connection re-establishment are collapsed
/// into one `failover_ms` window during which the node serves nothing and
/// every request fails as 'region-down'. A student who sets `failover_ms`
/// to 30000 to feel safe watches half a minute of total outage; one who
/// sets it to 0 gets a cutover no real system achieves. That trade is the
/// lesson, and collapsing three mechanisms into one number is what keeps
/// it legible.
///
/// What this deliberately is NOT: a geo-latency model, a replication
/// model, or an active-active router. Those need real links with
/// propagation delay, and `SimEdge.latency_ms` is where that will go in
/// the networking phase.
pub struct RegionBehaviour;

impl ComponentBehaviour for RegionBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Region
    }
    // A pure switch, like an lb: it forwards without holding slots of its
    // own.
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
    // pick_edge returning None here is not a wiring mistake: it means no
    // region is serving, either mid-failover or with every region down.
    fn no_route_reason(&self) -> Option<FailureReason> {
        Some(FailureReason::RegionDown)
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(RegionState {
            active: -1,
            failover_due_ms: -1.0,
            failover_target: -1,
            last_configured: -1,
        }))
    }

    fn on_admit(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
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

    /// Returning `None` means "no region is serving", which the engine
    /// resolves as a routing failure using `no_route_reason` ('region-down'),
    /// so a student sees WHY it failed rather than a generic routing error.
    fn pick_edge(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        out: &[SimEdge],
        ext: &mut Ext,
    ) -> Option<SimEdge> {
        let st = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<RegionState>())
            .expect("region ext");

        let cfg = state.config();
        let count = region_count(state);

        // Adopt the configured region on first use, and follow the
        // student if they CHANGE it in the Inspector. A manual switch is
        // instant and deliberate: a planned migration, not an outage.
        //
        // The trigger is a change in the configured value, never a
        // disagreement between it and the live region -- after a failover
        // the two differ by design, and treating that as a manual switch
        // would drag traffic back onto the region that just died.
        let configured = cfg.active_region.unwrap_or(0.0).floor() as i64;
        let wanted = if configured < 0 {
            0
        } else if configured >= count {
            count - 1
        } else {
            configured
        };
        if st.active == -1 || wanted != st.last_configured {
            st.last_configured = wanted;
            st.active = wanted;
            st.failover_due_ms = -1.0;
        }

        // A failover in progress lands once its window elapses. Evaluated
        // here, lazily on arrival, rather than from a tick -- so it
        // depends only on simulated time and not on how often the node
        // happened to be polled.
        if st.failover_due_ms >= 0.0 {
            if ctx.now() < st.failover_due_ms {
                return None; // still dark
            }
            st.active = st.failover_target;
            st.failover_due_ms = -1.0;
        }

        if region_healthy(ctx, state, st.active) {
            return out.get(st.active as usize).cloned();
        }

        // The active region is down. Scan forward with wraparound so
        // regions are always tried in the same order; an arbitrary or
        // random pick here would make failover non-reproducible from the
        // same seed.
        let mut next: i64 = -1;
        for i in 1..=count {
            let candidate = (st.active + i) % count;
            if region_healthy(ctx, state, candidate) {
                next = candidate;
                break;
            }
        }
        // Every region is down: there is nowhere to fail over to, so the
        // node stays dark rather than pretending a switch would help.
        if next == -1 {
            return None;
        }

        let failover_ms = cfg.failover_ms.unwrap_or(0.0).max(0.0);
        if failover_ms == 0.0 {
            st.active = next;
            return out.get(next as usize).cloned();
        }

        st.failover_target = next;
        st.failover_due_ms = ctx.now() + failover_ms;
        None // dark for the length of the failover window
    }

    /// Which outgoing edge is live, and which are merely standing by.
    ///
    /// A standby region is wired, healthy and deliberately unused. Left to
    /// the engine's flow-based fallback it would read 'idle' --
    /// indistinguishable from a dead link -- and the entire point of the
    /// component (there is a second region sitting there ready) would be
    /// invisible on the canvas.
    ///
    /// Strictly a read: `pick_edge` is where the state machine advances,
    /// and duplicating any of that here would let the number of times the
    /// UI polled change the simulation.
    fn edge_state_for(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _edge: &SimEdge,
        index: usize,
        ext: &mut Ext,
    ) -> Option<EdgeState> {
        let st = ext.as_mut().and_then(|e| e.downcast_mut::<RegionState>())?;
        let count = region_count(state);
        // Edges past `regions` are not regions at all; the engine's
        // flow-based fallback describes them better than this hook can.
        if index as i64 >= count {
            return None;
        }

        let live = live_region_index(ctx, state, st);
        // Nothing is serving -- mid-failover, or every region down. No
        // edge is live, and calling the others 'standby' would still be
        // true.
        if live == -1 {
            return Some(EdgeState::Standby);
        }
        Some(if index as i64 == live {
            EdgeState::Live
        } else {
            EdgeState::Standby
        })
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let st = match ext.as_mut().and_then(|e| e.downcast_mut::<RegionState>()) {
            Some(s) => s,
            None => return,
        };
        let failing_over = st.failover_due_ms >= 0.0 && ctx.now() < st.failover_due_ms;
        stats.active_region = Some(if st.active < 0 { 0.0 } else { st.active as f64 });
        stats.failing_over = Some(failing_over);
        // How much dark window is left. A pure read of the deadline
        // pick_edge set, so a UI can count the outage down instead of
        // freezing on a flag.
        stats.failover_remaining_ms = Some(if failing_over {
            (st.failover_due_ms - ctx.now()).max(0.0)
        } else {
            0.0
        });
        // Healthy-region census, from the same predicate pick_edge routes
        // with, so the readout and the routing can never disagree about
        // reachability.
        let count = region_count(state);
        let mut healthy = 0i64;
        for i in 0..count {
            if region_healthy(ctx, state, i) {
                healthy += 1;
            }
        }
        stats.regions_healthy = Some(healthy as f64);
        stats.regions_total = Some(count as f64);
        let live = live_region_index(ctx, state, st);
        stats.live_edge_id = if live == -1 {
            None
        } else {
            state.out().get(live as usize).map(|e| e.id.clone())
        };
    }
}

/// The region index traffic would take if a request arrived right now, or
/// -1 when none would: mid-failover, or with every region unreachable.
///
/// A pure mirror of the decision `pick_edge` makes, and it must stay pure
/// -- this is called from `decorate_stats`/`edge_state_for`, which run
/// inside the snapshot pass, so advancing `active` or landing a failover
/// from here would make the simulation depend on the UI's polling rate. It
/// therefore reads the failover deadline without clearing it and scans for
/// a healthy region without adopting one; `pick_edge` does the committing
/// when a real request turns up.
///
/// It intentionally does NOT re-apply the config-adoption step. That step
/// is driven by the student changing the Inspector value, and running it
/// here would consume the change -- `pick_edge` would then never see it,
/// and a manual region switch would silently do nothing whenever the UI
/// happened to snapshot first.
fn live_region_index(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike, st: &RegionState) -> i64 {
    let count = region_count(state);

    // A failover still inside its window: the node is dark, exactly as
    // pick_edge would report by returning None.
    if st.failover_due_ms >= 0.0 && ctx.now() < st.failover_due_ms {
        return -1;
    }

    // The window elapsed but no request has arrived to land it yet. The
    // next one will land on failover_target, so that is the honest answer
    // now.
    let active = if st.failover_due_ms >= 0.0 {
        st.failover_target
    } else {
        st.active
    };
    if active < 0 {
        // Nothing has been adopted yet; pick_edge would take the
        // configured region.
        let configured = state.config().active_region.unwrap_or(0.0).floor() as i64;
        let wanted = if configured < 0 {
            0
        } else if configured >= count {
            count - 1
        } else {
            configured
        };
        return if region_healthy(ctx, state, wanted) {
            wanted
        } else {
            -1
        };
    }

    if region_healthy(ctx, state, active) {
        return active;
    }

    // The active region is down. pick_edge would begin a failover, which
    // is a dark window before the next region takes over -- so report
    // dark, in the same fixed scan order, only conceding a live edge when
    // failover_ms is 0 and the cutover really is instant.
    let mut next: i64 = -1;
    for i in 1..=count {
        let candidate = (active + i) % count;
        if region_healthy(ctx, state, candidate) {
            next = candidate;
            break;
        }
    }
    if next == -1 {
        return -1;
    }
    if state.config().failover_ms.unwrap_or(0.0).max(0.0) == 0.0 {
        next
    } else {
        -1
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static AUTOSCALER: AutoscalerBehaviour = AutoscalerBehaviour;
static REGION: RegionBehaviour = RegionBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::Autoscaler, &AUTOSCALER),
    (NodeKind::Region, &REGION),
];

#[cfg(test)]
mod tests {
    use crate::sim::engine::Engine;
    use crate::sim::presets::default_config;
    use crate::sim::types::{NodeConfig, NodeKind, NodeStats, SimEdge, SimNode, Topology};

    fn node(id: &str, kind: NodeKind, config: NodeConfig) -> SimNode {
        SimNode { id: id.to_string(), kind, label: id.to_string(), x: 0.0, y: 0.0, config }
    }

    /// A client -> service pair with an autoscaler watching the service
    /// through a control edge, patched with whatever knob the test wants to
    /// try breaking. Mirrors `behaviour-control.autoscaler.test.ts`
    /// (upstream `ea11c5b3`, PR #64): capacity 2, 40ms service time, 400rps
    /// offered -- a load the lone starting instance cannot keep up with, so
    /// the controller MUST add instances for the design to drain.
    fn topology(patch: impl FnOnce(&mut NodeConfig)) -> Topology {
        let client_cfg = NodeConfig { rps: 400.0, ..default_config(NodeKind::Client) };
        let svc_cfg = NodeConfig {
            capacity: 2.0,
            service_ms: 40.0,
            instances: Some(1.0),
            ..default_config(NodeKind::Service)
        };
        let mut auto_cfg = default_config(NodeKind::Autoscaler);
        patch(&mut auto_cfg);

        Topology {
            nodes: vec![
                node("client", NodeKind::Client, client_cfg),
                node("svc", NodeKind::Service, svc_cfg),
                node("auto", NodeKind::Autoscaler, auto_cfg),
            ],
            edges: vec![
                SimEdge {
                    id: "e1".into(),
                    from: "client".into(),
                    to: "svc".into(),
                    weight: 1.0,
                    control: None,
                    latency_ms: None,
                    bandwidth_rps: None,
                    loss_rate: None,
                },
                SimEdge {
                    id: "e2".into(),
                    from: "auto".into(),
                    to: "svc".into(),
                    weight: 1.0,
                    control: Some(true),
                    latency_ms: None,
                    bandwidth_rps: None,
                    loss_rate: None,
                },
            ],
            annotations: None,
        }
    }

    fn run(patch: impl FnOnce(&mut NodeConfig)) -> (NodeStats, NodeStats) {
        let mut engine = Engine::new(topology(patch), 7);
        // Thirty simulated seconds at 60fps: several cooldowns, so the
        // controller has had every chance to act -- same budget as the web
        // test this ports.
        for _ in 0..1800 {
            engine.advance(1000.0 / 60.0);
        }
        let mut snap = engine.snapshot();
        (
            snap.nodes.remove("auto").expect("auto node"),
            snap.nodes.remove("svc").expect("svc node"),
        )
    }

    /// WHY: `behaviour-control.ts:57-70` (upstream `ea11c5b3`, PR #64).
    /// Before the `knob()` guard, a NaN `max_capacity` collapsed `max_inst`
    /// to `min_inst` (see the WHY-comment on `knob` above): the fleet froze
    /// at its floor instead of scaling, exactly the upstream bug report ("a
    /// knob left the fleet at one instance"). The fix is that the
    /// controller keeps scaling -- and keeps publishing finite stats -- no
    /// matter which of its six knobs a shared link or restored session
    /// hands it as NaN.
    #[test]
    fn autoscaler_still_scales_when_a_knob_is_nan() {
        let (_, baseline_svc) = run(|_| {});
        assert!(
            baseline_svc.instances.unwrap_or(0.0) > 5.0,
            "baseline design should need more than one instance"
        );
        assert!(
            baseline_svc.total_completed > 5000.0,
            "baseline design should complete substantial work in 30s"
        );

        let knobs: [(&str, fn(&mut NodeConfig)); 6] = [
            ("target_util", |c| c.target_util = Some(f64::NAN)),
            ("min_capacity", |c| c.min_capacity = Some(f64::NAN)),
            ("max_capacity", |c| c.max_capacity = Some(f64::NAN)),
            ("cooldown_ms", |c| c.cooldown_ms = Some(f64::NAN)),
            ("scale_step_pct", |c| c.scale_step_pct = Some(f64::NAN)),
            ("warmup_ms", |c| c.warmup_ms = Some(f64::NAN)),
        ];
        for (name, apply) in knobs {
            let (auto, svc) = run(apply);
            assert!(
                svc.instances.unwrap_or(0.0) > 5.0,
                "{name} = NaN should not stop the fleet from scaling (got {:?})",
                svc.instances
            );
            assert!(
                svc.total_completed > 4000.0,
                "{name} = NaN should not stall throughput (got {})",
                svc.total_completed
            );
            assert!(
                !auto.setpoint.unwrap_or(f64::NAN).is_nan(),
                "{name} = NaN must not make the published setpoint NaN"
            );
            assert!(
                !auto.target_instances.unwrap_or(f64::NAN).is_nan(),
                "{name} = NaN must not make the published target_instances NaN"
            );
        }
    }

    /// WHY: `behaviour-control.ts:57-70` (upstream `ea11c5b3`, PR #64).
    /// `DEFAULT_TARGET_UTIL`/`DEFAULT_MIN_INSTANCES`/`DEFAULT_STEP_PCT`
    /// happen to equal the preset's own configured `target_util`/
    /// `min_capacity`/`scale_step_pct`, so a NaN in exactly one of these
    /// three is indistinguishable from the value it replaced -- the
    /// sharpest available statement that the guard changed nothing real
    /// for a design the Inspector could actually produce.
    #[test]
    fn nan_identical_to_baseline_when_fallback_matches_configured_value() {
        let (_, baseline) = run(|_| {});

        let knobs: [(&str, fn(&mut NodeConfig)); 3] = [
            ("target_util", |c| c.target_util = Some(f64::NAN)),
            ("min_capacity", |c| c.min_capacity = Some(f64::NAN)),
            ("scale_step_pct", |c| c.scale_step_pct = Some(f64::NAN)),
        ];
        for (name, apply) in knobs {
            let (_, svc) = run(apply);
            assert_eq!(
                svc.instances, baseline.instances,
                "{name} = NaN should match baseline instances exactly"
            );
            assert_eq!(
                svc.total_completed, baseline.total_completed,
                "{name} = NaN should match baseline throughput exactly"
            );
        }
    }

    /// WHY: `behaviour-control.ts` `decorateStats` (upstream `7e285598`,
    /// PR #26). An autoscaler pointed at an `ObjectStore` -- a kind with no
    /// `scale_field` at all, unlike a `Service`/`Autoscaler` target sitting
    /// at a legitimate 0 instances -- must say so distinctly: `scale_of`
    /// returns `None` rather than `Some(0.0)`, and that `None` is what
    /// `watched_unscalable` surfaces to the Inspector ("adding servers is
    /// not the fix").
    #[test]
    fn watching_a_fleetless_kind_is_reported_as_unscalable() {
        let client_cfg = NodeConfig { rps: 10.0, ..default_config(NodeKind::Client) };
        let store_cfg = default_config(NodeKind::ObjectStore);
        let auto_cfg = default_config(NodeKind::Autoscaler);

        let topo = Topology {
            nodes: vec![
                node("client", NodeKind::Client, client_cfg),
                node("store", NodeKind::ObjectStore, store_cfg),
                node("auto", NodeKind::Autoscaler, auto_cfg),
            ],
            edges: vec![
                SimEdge {
                    id: "e1".into(),
                    from: "client".into(),
                    to: "store".into(),
                    weight: 1.0,
                    control: None,
                    latency_ms: None,
                    bandwidth_rps: None,
                    loss_rate: None,
                },
                SimEdge {
                    id: "e2".into(),
                    from: "auto".into(),
                    to: "store".into(),
                    weight: 1.0,
                    control: Some(true),
                    latency_ms: None,
                    bandwidth_rps: None,
                    loss_rate: None,
                },
            ],
            annotations: None,
        };

        let mut engine = Engine::new(topo, 7);
        for _ in 0..60 {
            engine.advance(1000.0 / 60.0);
        }
        let mut snap = engine.snapshot();
        let auto = snap.nodes.remove("auto").expect("auto node");

        assert_eq!(
            auto.watched_id.as_deref(),
            Some("store"),
            "autoscaler should resolve its control edge to the store"
        );
        assert_eq!(
            auto.watched_unscalable,
            Some(true),
            "watching a kind with no scale_field must report unscalable"
        );
    }
}
