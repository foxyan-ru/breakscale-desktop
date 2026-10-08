//! Component behaviour registry — port of `src/sim/behaviour.ts`.
//!
//! Every kind-specific decision the event loop needs to make lives here, one
//! `ComponentBehaviour` impl per `NodeKind`. `sim::engine::Engine` resolves a
//! node's behaviour once, at build time, and the hot path calls it through a
//! `&'static dyn ComponentBehaviour` reference from then on — never a
//! `match kind` on the event loop, mirroring the TS registry's own stated
//! goal exactly.
//!
//! `ComponentBehaviour` is policy, not state: every impl here is a
//! zero-field unit struct. Per-node runtime state a behaviour needs (an
//! autoscaler's cooldown clock, a breaker's window) lives in the engine's
//! `NodeState::ext`, created once by `init_state` and handed back to every
//! later hook as `ext: &mut Option<Box<dyn Any + Send + Sync>>` — the Rust
//! shape of the TS `state.ext: unknown`, which each behaviour downcasts to
//! its own private type. This file only defines the trait and the six
//! simplest behaviours (`client`, `lb`, `service`, `cache`, `queue`,
//! `worker` — verbatim from `behaviour.ts` itself); the other 27 kinds are
//! ported into the sibling `control`/`data`/`edge`/`messaging`/`resilience`/
//! `store` modules, one Rust file per TS `behaviour-*.ts` file, matching the
//! TS module boundaries exactly so a reader can still find "the shard
//! behaviour" in the file its TS counterpart lived in.

use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqLike};
use crate::sim::types::{EdgeState, FailureReason, NodeKind, NodeStats, SimEdge};
use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;

pub mod control;
pub mod data;
pub mod edge;
pub mod messaging;
pub mod resilience;
pub mod store;

/// Behaviour-private scratch state, opaque to the engine. `None` for a kind
/// with no state of its own (most kinds). The Rust shape of `state.ext:
/// unknown` in the TS engine.
pub type Ext = Option<Box<dyn Any + Send + Sync>>;

/// What a node does with a request offered to it. Defaults to `Serve` when a
/// behaviour does not override `on_admit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitAction {
    /// The node refused it; the engine books a shed and fails the call.
    Shed,
    /// Put it through the normal slot/queue discipline.
    Serve,
    /// Zero-capacity hand-off: draw a service time but never queue.
    Passthru,
    /// The behaviour fully handled admission itself (a queue acks the
    /// caller and buffers a detached copy via `ctx.ack_and_buffer`).
    Handled,
}

/// How a node fans its work out to its downstream neighbours. Defaults to
/// `All`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteMode {
    /// Call every outgoing edge and join the results (service, db, ...).
    All,
    /// Pick a single edge (load balancer).
    One,
    /// Do not call downstream at all; resolve here.
    None,
}

/// What happens once a node has finished its own service time. Defaults to
/// `Downstream`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteAction {
    /// Proceed to routing (the default).
    Downstream,
    /// Answer right here without calling downstream (a cache hit).
    Complete,
}

/// How a node's waiting work is drained when a slot frees up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpMode {
    /// Serve from this node's own FIFO.
    Own,
    /// Pull messages out of the queue nodes that feed this node.
    Sources,
    /// Nothing to pump: the node holds no work of its own.
    None,
}

/// Which `NodeConfig` knob a controller moves to make this kind bigger. See
/// the long reasoning comment on `ComponentBehaviour::scale_field` in the TS
/// source (`behaviour.ts`) — preserved there, not repeated per call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleField {
    Instances,
    ShardCapacity,
}

/// Is this kind really N things rather than one? See `NodeStats` in
/// `sim::types` for the full per-kind interpretation; this only selects
/// which mechanism produces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceModel {
    /// One unit per capacity slot; the engine derives the count itself.
    Slots,
    /// The kind knows its own structure and publishes it from
    /// `report_instances`.
    Custom,
}

/// Port of `ComponentBehaviour` from `behaviour.ts`. Static traits are
/// methods with a value (no meaningful "unset" state); optional TS fields
/// (`?`) become methods with a default implementation matching the TS
/// default described in that field's own doc comment.
pub trait ComponentBehaviour: Send + Sync {
    fn kind(&self) -> NodeKind;

    /* ---- static traits, read directly on the hot path ---- */

    /// Does this node occupy server slots and report a meaningful
    /// utilisation? A queue is a buffer, not a server: its slots mean
    /// nothing, so it reports 0.
    fn serves_requests(&self) -> bool;
    /// Does this node generate root requests (a traffic source)?
    fn generates_load(&self) -> bool;
    /// Is every edge leaving this kind a CONTROL edge rather than a request
    /// path? True only for the autoscaler; see `SimEdge.control`.
    fn controls_target(&self) -> bool {
        false
    }
    /// Does this node pull work from buffering nodes rather than being
    /// pushed to?
    fn pulls_from_queues(&self) -> bool;
    /// Does this node hold messages for pull-based consumers to drain?
    fn buffers_for_consumers(&self) -> bool;
    /// How waiting work is drained when capacity frees up.
    fn pump(&self) -> PumpMode;
    /// When a fan-out join completes at this node, does it book its own
    /// completion/latency? A client does not: `resolve()` already credits
    /// the root request end-to-end.
    fn credits_join_completion(&self) -> bool;
    /// Should the engine report downstream call outcomes back to this node
    /// via `on_downstream_result`?
    fn observes_outcome(&self) -> bool {
        false
    }
    /// Which config knob a controller moves to make this kind bigger.
    /// `None` means this kind cannot be scaled by a controller at all.
    fn scale_field(&self) -> Option<ScaleField> {
        None
    }
    /// Is this kind really N things rather than one? `None` means
    /// `NodeStats.instances` stays `None`.
    fn instance_model(&self) -> Option<InstanceModel> {
        None
    }
    /// `FailureReason` to use when this kind's `pick_edge` declines to
    /// choose an edge. `None` means the engine's generic `'no-route'`.
    fn no_route_reason(&self) -> Option<FailureReason> {
        None
    }

    /* ---- per-request hooks ---- */

    /// Decide what admission means for this kind. Defaults to `Serve`.
    fn on_admit(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        AdmitAction::Serve
    }

    /// Fan-out policy. Defaults to `All`.
    fn route(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> RouteMode {
        RouteMode::All
    }

    /// Choose the single edge when `route` returned `One`. `None` means
    /// this kind declines to choose (see `no_route_reason`).
    fn pick_edge(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _out: &[SimEdge],
        _ext: &mut Ext,
    ) -> Option<SimEdge> {
        None
    }

    /// Called after the node's own service time elapsed and its
    /// independent error roll passed. Defaults to `Downstream`.
    fn on_service_complete(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        CompleteAction::Downstream
    }

    /// Autonomous per-tick work for kinds that act without a request
    /// arriving. The engine only walks the tick list for nodes whose
    /// behaviour overrides this, so an empty default costs nothing.
    fn on_tick(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _dt_ms: f64,
        _ext: &mut Ext,
    ) {
    }

    /// Build this kind's private scratch state, stored on `NodeState::ext`.
    /// Called once per node, before anything can run. A kind with no state
    /// of its own returns `None` (the default) and `ext` stays `None`.
    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        None
    }

    /// Called when a downstream call issued by this node resolves, before
    /// the result is joined. Only invoked for behaviours where
    /// `observes_outcome()` is true.
    fn on_downstream_result(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ok: bool,
        _reason: FailureReason,
        _ext: &mut Ext,
    ) {
    }

    /// Fired for a request this behaviour deliberately held at admission
    /// via `ctx.wake_after` (so far only the bulkhead's acquire queue),
    /// once that timer elapses. The request may already have been resumed
    /// through `ctx.resume_admission` by the time this fires (a slot freed
    /// before the timeout did) -- the default no-op is safe for every
    /// behaviour that never calls `wake_after` in the first place, and a
    /// behaviour that does call it must check its own waiter bookkeeping
    /// before acting, exactly as `BulkheadBehaviour::on_wake` does. Port of
    /// `ComponentBehaviour.onWake` in `behaviour.ts` (upstream `351327c4`,
    /// PR #77).
    fn on_wake(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) {
    }

    /// Publish this kind's own readouts onto its `NodeStats` entry at
    /// snapshot time.
    fn decorate_stats(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
    }

    /// Publish the instance vector for an `InstanceModel::Custom` kind via
    /// `ctx.report_instances`. Called from the snapshot loop, before
    /// `decorate_stats`.
    fn report_instances(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _ext: &mut Ext,
    ) {
    }

    /// Classify one of this node's outgoing edges for the snapshot's
    /// `edge_state` map. Must be a pure read (runs inside `snapshot()`).
    fn edge_state_for(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _edge: &SimEdge,
        _index: usize,
        _ext: &mut Ext,
    ) -> Option<EdgeState> {
        None
    }
}

pub fn clamp01(v: f64) -> f64 {
    if v < 0.0 {
        0.0
    } else if v > 1.0 {
        1.0
    } else {
        v
    }
}

/* ------------------------------------------------------------------ *
 * The six behaviours declared directly in behaviour.ts (everything else
 * lives in the sibling modules, one file per TS `behaviour-*.ts`).
 * ------------------------------------------------------------------ */

/// A traffic source. Generates root requests and hands each one straight
/// downstream; it never queues and never occupies a slot.
pub struct ClientBehaviour;
impl ComponentBehaviour for ClientBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Client
    }
    fn serves_requests(&self) -> bool {
        true
    }
    fn generates_load(&self) -> bool {
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
    fn on_admit(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        AdmitAction::Passthru
    }
}

/// A dispatcher. Picks exactly one downstream per request: weighted-random
/// when the edge weights differ, least-loaded when they are all equal.
///
/// Its pool is real. There is deliberately no `on_admit` override here, so
/// the engine's default (`AdmitAction::Serve`) puts `capacity`, `instances`
/// and `queueLimit` through the same slot and queue discipline as any other
/// serving kind. A previous `on_admit` returning `Passthru` skipped all
/// three, which made an lb's sizing knobs inert -- a pair sized for 2
/// concurrent calls carried ~3000 rps, `waiting` never moved off 0, and an
/// autoscaler driving its `instances` had nothing to turn. Fixed upstream in
/// `behaviour.ts` (commit `79254ce0`, PR #56); see also `engine.ts:1731`'s
/// `beginZeroService` doc comment, updated in the same commit to stop
/// describing lb as pass-through.
pub struct LbBehaviour;
impl ComponentBehaviour for LbBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Lb
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
        _ext: &mut Ext,
    ) -> Option<SimEdge> {
        ctx.pick_weighted_or_least_loaded(out)
    }
}

/// A plain server: finite slots, a bounded FIFO, then fan-out to everything.
pub struct ServiceBehaviour;
impl ComponentBehaviour for ServiceBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Service
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
}

/// A read-through cache. On a hit it answers immediately; on a miss it
/// falls through to whatever backs it, or answers anyway if nothing is
/// wired up.
pub struct CacheBehaviour;
impl ComponentBehaviour for CacheBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Cache
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
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        if ctx.roll() < clamp01(state.config().hit_rate) {
            ctx.count_hit(state);
            return CompleteAction::Complete;
        }
        ctx.count_miss(state);
        if state.out().is_empty() {
            CompleteAction::Complete
        } else {
            CompleteAction::Downstream
        }
    }
}

/// A buffer, not a server. It acknowledges the caller immediately and parks
/// a detached copy of the message for workers to drain. Its "busy" count is
/// meaningless, so it reports zero utilisation.
pub struct QueueBehaviour;
impl ComponentBehaviour for QueueBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Queue
    }
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
        true
    }
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
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
        ctx.ack_and_buffer(state, req);
        AdmitAction::Handled
    }
}

/// A consumer. It is never pushed to: it pulls the oldest available message
/// across every queue node feeding it.
pub struct WorkerBehaviour;
impl ComponentBehaviour for WorkerBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Worker
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
    fn credits_join_completion(&self) -> bool {
        true
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static CLIENT: ClientBehaviour = ClientBehaviour;
static LB: LbBehaviour = LbBehaviour;
static SERVICE: ServiceBehaviour = ServiceBehaviour;
static CACHE: CacheBehaviour = CacheBehaviour;
static QUEUE: QueueBehaviour = QueueBehaviour;
static WORKER: WorkerBehaviour = WorkerBehaviour;

static REGISTRY: OnceLock<HashMap<NodeKind, &'static dyn ComponentBehaviour>> = OnceLock::new();

fn build_registry() -> HashMap<NodeKind, &'static dyn ComponentBehaviour> {
    let mut m: HashMap<NodeKind, &'static dyn ComponentBehaviour> = HashMap::new();
    for b in [
        &CLIENT as &'static dyn ComponentBehaviour,
        &LB,
        &SERVICE,
        &CACHE,
        &QUEUE,
        &WORKER,
    ] {
        m.insert(b.kind(), b);
    }
    for (kind, b) in control::BEHAVIOURS
        .iter()
        .chain(data::BEHAVIOURS.iter())
        .chain(edge::BEHAVIOURS.iter())
        .chain(messaging::BEHAVIOURS.iter())
        .chain(resilience::BEHAVIOURS.iter())
        .chain(store::BEHAVIOURS.iter())
    {
        m.insert(*kind, *b);
    }
    m
}

/// Resolve a kind's behaviour. Called once per node at build time and
/// cached on the node state; never called from the event loop. An unknown
/// kind degrades to plain-server semantics rather than panicking, so a
/// topology saved by a newer build still loads (mirrors `behaviourFor`'s
/// TS fallback to `service`).
pub fn behaviour_for(kind: NodeKind) -> &'static dyn ComponentBehaviour {
    let reg = REGISTRY.get_or_init(build_registry);
    reg.get(&kind).copied().unwrap_or(&SERVICE)
}
