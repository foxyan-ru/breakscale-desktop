//! Structural types shared between the engine and the behaviour registry,
//! ported from `src/sim/engine-types.ts`.
//!
//! They live in their own module so `sim::behaviour` (ported separately)
//! can describe what it is handed without importing the `Engine` type
//! (which will need to import behaviour registrations back). In the TS
//! source, the engine's real `NodeState` and `Req` satisfy `NodeStateLike`
//! and `ReqLike` structurally, with no casting or adapter layer. Rust has no
//! structural typing, so here those two become **traits**: the engine's
//! concrete node/request types will `impl` them, and behaviour code is
//! written against `&dyn NodeStateLike` / `&dyn ReqLike` instead of a
//! concrete struct -- the same "small surface" discipline the TS interfaces
//! documented, now enforced by the compiler rather than by convention.
//!
//! `BehaviourCtx` is likewise a trait: the slice of the engine a behaviour
//! is allowed to drive. Every method here is written to take `&dyn
//! NodeStateLike` / `&dyn ReqLike` parameters (rather than being generic
//! over `NodeStateLike`/`ReqLike`) specifically so `BehaviourCtx` itself
//! stays object-safe -- `Engine` needs to hand a single `&mut dyn
//! BehaviourCtx` to behaviour functions for heterogeneous node/request
//! implementors, which a generic method could not do.
//!
//! Signatures here are the target contract from `engine-types.ts`;
//! `sim::engine`'s concrete `Engine` impl is the source of truth if the two
//! ever disagree, since Rust's ownership model sometimes forces a shape
//! TS's structural typing didn't need to consider.
//!
//! `serve_within`/`serve_deferred`'s callback is `Box<dyn FnOnce(..) +
//! Send>`, not bare `Box<dyn FnOnce(..)>`: `sim::engine::Engine` lives
//! behind `Arc<Mutex<Option<Engine>>>` (see `state.rs`), moved onto a
//! background tick thread and locked from Tauri's async command handlers on
//! whatever thread they run on, so `Engine` -- and therefore every value it
//! can end up owning, including a request's stored `on_drained` closure --
//! must be `Send`. A closure built from `Send` captures (the common case:
//! primitives, `String`, `Arc<Mutex<_>>`) coerces to this bound
//! automatically at the `Box::new(..)` call site; this only matters where a
//! callback's type is written out explicitly rather than inferred inline.

use crate::sim::types::{FailureReason, NodeConfig, NodeKind, SimEdge};

/// The engine's per-node runtime state, as far as a behaviour may see it.
///
/// The TS interface also carries an `ext: unknown` field: behaviour-private
/// scratch state, created once by the behaviour's `initState()` hook and
/// owned entirely by that behaviour. `ext` is deliberately NOT part of this
/// trait -- it is engine-internal, and behaviour code that needs it will get
/// it through a different mechanism the `sim::engine` implementer designs
/// (a concrete downcast, an associated type, or similar), not through this
/// shared read-only view. See `sim::engine` for how behaviours actually
/// access it.
pub trait NodeStateLike {
    fn id(&self) -> &str;
    fn kind(&self) -> NodeKind;
    fn config(&self) -> &NodeConfig;
    /// Requests occupying a server slot right now.
    fn busy(&self) -> f64;
    /// Outgoing edges, resolved from the topology.
    fn out(&self) -> &[SimEdge];
    /// Queue nodes feeding this node, for pull-based kinds.
    fn sources(&self) -> &[String];
}

/// A stable reference to a request the engine is tracking, valid until that
/// request resolves.
///
/// `&dyn ReqLike` is a borrow tied to the one hook call that produced it --
/// exactly the tool a behaviour needs to *read* the request it was just
/// handed, and exactly the tool it cannot store. Several kinds need to do
/// the latter: a replica set or a sharded store admits a request, finds no
/// free slot, and must hold onto that SAME request (not a copy of its
/// field values -- the original, including whatever lets the engine
/// eventually bubble a result back to its parent) until a slot frees, then
/// hand it to `serve_within`. `ReqHandle` is that storable token: cheap to
/// copy, opaque, and redeemable through `serve_within` once. It carries no
/// accessors of its own -- a behaviour that needs to read the request again
/// before redeeming the handle keeps the `&dyn ReqLike` view it was
/// originally given (or the field values it cares about) alongside the
/// handle, the same way the TS engine's behaviours hold the real `Req`
/// object and read whatever they need from it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReqHandle {
    pub index: u32,
    pub generation: u32,
}

/// An in-flight request, as far as a behaviour may see it.
pub trait ReqLike {
    fn node_id(&self) -> &str;
    /// Hop depth from the client root.
    fn hop(&self) -> u32;
    /// Which retry attempt this call represents at the parent.
    fn attempt(&self) -> u32;
    /// Own service time already spent at this node.
    fn own_ms(&self) -> f64;
    /// True when this is a detached queue message being drained by a
    /// worker.
    fn detached(&self) -> bool;
    /// Partition/cache key this request concerns. Drawn by the client from
    /// a small keyspace and inherited unchanged by every downstream call
    /// the request makes, so a replica set or a shard several hops away
    /// sees the key the client actually asked for. Kinds that do not
    /// partition by key ignore it entirely.
    fn key(&self) -> u32;
    /// True when this request is a write rather than a read. Set by the
    /// kind that first classifies it (a replica set, from `readFraction`)
    /// and inherited downstream, so a later node sees the same
    /// classification.
    fn is_write(&self) -> bool;

    /// Added while porting `sim::engine`: lets a `BehaviourCtx` method that
    /// takes a `req: &dyn ReqLike` parameter (`handle_of`, `ack_and_buffer`,
    /// `ack_and_relay`, `serve_within`, `mark_write`, `add_service_delay`,
    /// `fail`, `reject`) recover the CONCRETE value behind the trait object
    /// via `downcast_ref`, which is the only way `sim::engine` can map an
    /// opaque `&dyn ReqLike` back to the real pooled request it has to
    /// mutate or resolve.
    ///
    /// WHY THIS IS LOAD-BEARING, NOT A CONVENIENCE. Every `BehaviourCtx`
    /// method above is handed whatever `&dyn ReqLike` the behaviour passes
    /// back -- normally the exact same reference it was just given in the
    /// current hook call. In TS this needs nothing extra: `req` is the real
    /// `Req` object, and `this.resolve(req, ...)` already has the only
    /// identity that exists. In Rust, `sim::engine` cannot hand a behaviour
    /// hook a live borrow of its actual pooled `Req` AND `&mut self` (as
    /// `ctx`) in the same call -- see `sim::engine`'s module doc, device 2
    /// ("NodeView`/`ReqView`") -- so every `req: &dyn ReqLike` a behaviour
    /// ever sees is necessarily a disconnected view value, not the real
    /// request. Without a way back from that view to the request's actual
    /// slot in `sim::engine`'s request pool, `handle_of` (and therefore
    /// `ack_and_buffer`/`ack_and_relay`/`serve_within`/...) would have no
    /// sound implementation at all. This is exactly the kind of gap this
    /// porting task's instructions ask to be fixed by adding to this file
    /// rather than worked around: `sim::engine` implements this for its own
    /// request view type, and nothing in `sim::behaviour` needs to change,
    /// since no behaviour file implements `ReqLike` itself (every one of
    /// them only ever receives `&dyn ReqLike`, never constructs a value
    /// that satisfies it).
    fn as_any(&self) -> &dyn std::any::Any;
}

/// The slice of the engine a behaviour is allowed to drive.
///
/// Deliberately small: a behaviour decides policy and asks the engine to
/// carry it out. Nothing here exposes the heap, the request pool, or the
/// clock's mutability, so a behaviour cannot desynchronise the simulation.
///
/// Signatures here are the target contract from `engine-types.ts`;
/// `sim::engine`'s concrete `Engine` impl is the source of truth if the two
/// ever disagree, since Rust's ownership model sometimes forces a shape
/// TS's structural typing didn't need to consider.
pub trait BehaviourCtx {
    /// Current simulated time in ms.
    fn now(&self) -> f64;

    /// One draw from the deterministic RNG. Behaviours must take draws in a
    /// fixed order for a given event, or replay stops matching.
    fn roll(&mut self) -> f64;

    /// Queued (not yet in service) request count for a node.
    fn queue_depth(&self, state: &dyn NodeStateLike) -> f64;
    /// `queueLimit` floored to a non-negative integer.
    fn effective_queue_limit(&self, state: &dyn NodeStateLike) -> f64;
    /// How many requests this node can serve at once: `instances *
    /// capacity`, floored to at least one slot. `capacity` is the slot
    /// count of one instance; this is the whole node's parallelism.
    fn effective_capacity(&self, state: &dyn NodeStateLike) -> f64;
    /// How many instances this node runs, >= 1. Absent config means one.
    fn effective_instances(&self, state: &dyn NodeStateLike) -> f64;

    /// Book a cache hit against a node's rolling counters.
    fn count_hit(&mut self, state: &dyn NodeStateLike);
    /// Book a cache miss against a node's rolling counters.
    fn count_miss(&mut self, state: &dyn NodeStateLike);

    /// Queue-node admission: ack the caller and buffer a detached copy.
    fn ack_and_buffer(&mut self, state: &dyn NodeStateLike, req: &dyn ReqLike);

    /// Relay-node admission: ack the caller and deliver a detached copy
    /// DOWNSTREAM through this node's own slots, service time, routing and
    /// retry machinery. The sibling of `ack_and_buffer` for kinds that push
    /// rather than being pulled from (a retry queue, a write-behind
    /// cache). `extra_delivery_ms` is added to the relayed copy's service
    /// time only, modelling time the message deliberately sits before
    /// delivery (a write-behind flush interval). The behaviour must check
    /// its own depth limit against `queue_depth` before calling.
    fn ack_and_relay(
        &mut self,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        extra_delivery_ms: Option<f64>,
    );

    /// The shared edge-selection policy: weighted-random, or least-loaded
    /// on ties. Returns an owned copy of the chosen edge (rather than a
    /// borrow) so this method -- and therefore the trait -- stays
    /// object-safe; `SimEdge` is cheap to clone.
    fn pick_weighted_or_least_loaded(&mut self, out: &[SimEdge]) -> Option<SimEdge>;

    /// Fail a request with a reason, contributing `latency_ms` to its
    /// parent.
    fn fail(&mut self, req: &dyn ReqLike, reason: FailureReason, latency_ms: f64);

    /// Refuse a request at `state` with an arbitrary reason, booking it
    /// against that node's error counters. This is the general form of the
    /// engine's own shed path: the two differ only in which counter is
    /// credited. A behaviour uses it to refuse with a reason the engine has
    /// no opinion about -- 'throttled' for a spent token bucket, 'rejected'
    /// for an open circuit.
    fn reject(&mut self, state: &dyn NodeStateLike, req: &dyn ReqLike, reason: FailureReason);

    /// Resume a request a behaviour deliberately held at admission (a
    /// bulkhead's acquire queue, so far the only caller) by routing it
    /// through the engine's zero-service dispatcher, WITHOUT counting a
    /// second arrival -- the admission already happened once, when the
    /// request first reached this node. `req` is a handle obtained earlier
    /// via `handle_of`, not a live view: the request is being resumed from
    /// a HOOK CALL OTHER THAN the one that admitted it (typically
    /// `on_downstream_result`, once a slot frees), so no `&dyn ReqLike`
    /// view of it is available to pass instead. Port of
    /// `BehaviourCtx.resumeAdmission` (`engine-types.ts`, upstream
    /// `351327c4`, PR #77).
    fn resume_admission(&mut self, state: &dyn NodeStateLike, req: ReqHandle);

    /// Schedule a behaviour-owned wakeup: `req`'s behaviour's `on_wake` hook
    /// fires in `delay_ms`, unless the request resolves or is recycled
    /// first (the same `ReqHandle` generation guard every other timer in
    /// this engine uses -- see `sim::engine`'s module doc, "Request
    /// identity"). The bulkhead's acquire queue uses this for its
    /// `acquireTimeoutMs` deadline: a request that is resumed via
    /// `resume_admission` before this timer fires must recognize that in
    /// its own `on_wake` (its waiter bookkeeping already dropped the
    /// request) and do nothing. Port of `BehaviourCtx.wakeAfter`
    /// (`engine-types.ts`, upstream `351327c4`, PR #77).
    fn wake_after(&mut self, state: &dyn NodeStateLike, req: ReqHandle, delay_ms: f64);

    /// Book a behaviour-defined counter against a node, in events per
    /// second over the engine's standard rate window. Counter names are
    /// namespaced per node, so two behaviours can never collide. Read back
    /// with `counter_rate`. This exists so a behaviour can publish a
    /// readout of its own without the engine growing a fixed field for
    /// every kind that wants one.
    fn count_custom(&mut self, state: &dyn NodeStateLike, name: &str, n: f64);
    /// Read back a counter booked with `count_custom`, as events per
    /// second.
    fn counter_rate(&self, state: &dyn NodeStateLike, name: &str) -> f64;

    // ---- controller surface --------------------------------------------
    // For kinds that act ON other nodes rather than serving requests: an
    // autoscaler writes a watched node's capacity, a region node needs to
    // know whether its chosen downstream is actually alive.
    // ---------------------------------------------------------------------

    /// A node's smoothed utilisation, 0..1, or `None` if it does not
    /// exist. This is the same figure the UI shows, so a student watching
    /// the meter is watching exactly what the controller reacts to.
    fn utilization_of(&self, node_id: &str) -> Option<f64>;
    /// How big a node's fleet is, in the unit that kind scales along:
    /// instances for an ordinary server, slots-per-shard for a sharded
    /// store. `None` when the node does not exist, or is a kind with no
    /// fleet to move.
    fn scale_of(&self, node_id: &str) -> Option<f64>;
    /// Resize a node's fleet, in the same unit `scale_of` reports. Applied
    /// to both runtime state and the stored topology so the Inspector
    /// shows what the controller did, and any work waiting on the newly
    /// freed slots starts immediately. A no-op for a kind that cannot be
    /// scaled.
    fn set_scale(&mut self, node_id: &str, units: f64);
    /// The node this controller drives, from its CONTROL edge, or `""`
    /// when it is not wired to one -- or when that edge is cut. Control
    /// edges are held apart from the routing set, so this is the only way
    /// to reach a target.
    fn control_target_of(&self, state: &dyn NodeStateLike) -> String;
    /// Is this node currently taken out by an injected 'crash'? A
    /// controller must not treat a crashed node as merely idle, and a
    /// region node uses it to decide the active region has failed.
    fn is_crashed(&self, node_id: &str) -> bool;
    /// Is this edge cut by an injected 'partition'?
    fn is_edge_cut(&self, edge_id: &str) -> bool;

    // ---- self-managed service -------------------------------------------
    //
    // For a kind whose internal structure the engine has no concept of: a
    // sharded store is N independent queue-and-server units, which cannot
    // be expressed in the single busy/waiting pair on NodeState. These let
    // such a behaviour keep its own slot bookkeeping in `ext` while still
    // driving the engine's real service timing, error roll, timeout and
    // completion path, instead of reimplementing them and drifting from
    // the other kinds.
    // ---------------------------------------------------------------------

    /// Draw a service time and schedule `req`'s completion at this node,
    /// WITHOUT touching the node's own busy counter -- the behaviour owns
    /// that. Once the service time elapses, `on_drained` fires (so the
    /// behaviour can release its slot and start whatever it had waiting)
    /// and the request then continues down the normal completion path:
    /// error roll, `on_service_complete`, routing.
    ///
    /// TS takes `onDrained` as a `(ctx, state, req) => void` closure so the
    /// behaviour can release its slot and start whatever it had waiting; the
    /// Rust equivalent is a boxed `FnOnce` so the engine can store and
    /// invoke it once the drawn service time elapses, without `BehaviourCtx`
    /// needing a generic (and therefore non-object-safe) callback
    /// parameter. For a request the behaviour still holds a live view of
    /// (the common case: admit it, then serve it within the same hook
    /// call). For a request captured earlier and held in `ext` while it
    /// waited for a free slot, use `serve_deferred` instead -- see there for
    /// why that needs a different signature.
    fn serve_within(
        &mut self,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    );

    /// Obtain a storable handle for a request the behaviour currently holds
    /// a live view of, typically during `on_admit` right before deciding
    /// there is no free slot and the request must wait. Store the returned
    /// `ReqHandle` (cheap `Copy`) in `ext`; redeem it later with
    /// `serve_deferred`. Calling this does not change the request's state in
    /// any way -- it is a pure lookup of the request's stable identity, the
    /// Rust answer to "just keep the object reference around" now that the
    /// object is a borrow that cannot outlive the current call.
    fn handle_of(&self, req: &dyn ReqLike) -> ReqHandle;

    /// Like `serve_within`, but for a request captured earlier via
    /// `handle_of` and held in `ext` while it waited for a free slot -- the
    /// deferred path a replica set, a sharded store, or any other queuing
    /// behaviour needs, where the request that is NOW getting a slot is not
    /// the one the current hook call was invoked with. Redeems the handle
    /// against the engine's real pooled request, which is what preserves its
    /// parent chain so the original caller still gets a result (re-deriving
    /// a fresh detached request from remembered field values would NOT do
    /// this -- it would silently orphan the caller). Otherwise behaves
    /// exactly like `serve_within`, including handing `on_drained` a fresh
    /// `&dyn ReqLike` view once the service time elapses.
    fn serve_deferred(
        &mut self,
        state: &dyn NodeStateLike,
        req: ReqHandle,
        on_drained: Box<dyn FnOnce(&mut dyn BehaviourCtx, &dyn NodeStateLike, &dyn ReqLike) + Send>,
    );

    /// Add `extra_ms` to the service time of a request the behaviour is
    /// about to serve -- replication lag on a write, for instance. Applied
    /// by `serve_within` on top of the drawn service time.
    fn add_service_delay(&mut self, req: &dyn ReqLike, extra_ms: f64);

    /// Like `add_service_delay`, for a request about to be served via
    /// `serve_deferred` rather than `serve_within`.
    fn add_service_delay_deferred(&mut self, req: ReqHandle, extra_ms: f64);

    /// Publish a per-shard utilisation vector for the snapshot. The engine
    /// copies it into `NodeStats` and computes max/min; it has no opinion
    /// about what a "shard" is.
    fn report_shard_utilization(&mut self, state: &dyn NodeStateLike, per_shard: &[f64]);

    /// Publish this node's instance vector: how many units it is made of
    /// right now, and each unit's utilisation. Called from
    /// `report_instances` by a kind whose structure the engine cannot
    /// derive -- a shard's partitions, a replica set's primary plus read
    /// replicas.
    ///
    /// `per_unit` is COPIED, so a behaviour may hand over the same scratch
    /// buffer every frame. The engine only publishes a new array to the
    /// snapshot when the contents actually changed, which is what lets a
    /// memoised consumer compare by reference and still never miss an
    /// update.
    ///
    /// The meaning of an index is fixed per kind and documented on
    /// `NodeStats`. `pending` is units decided but not yet serving; pass 0
    /// when the kind has no such notion.
    fn report_instances(&mut self, state: &dyn NodeStateLike, per_unit: &[f64], pending: f64);

    /// Report true occupancy for a kind that runs its own slot discipline.
    ///
    /// The engine integrates utilisation from `state.busy`, which
    /// `serve_within` deliberately never touches -- so a kind holding its
    /// slots in `ext` (a replica set's two pools, a sharded store's
    /// partitions) would otherwise report utilisation 0.0 no matter how
    /// saturated it actually is. That is not merely a dead meter:
    /// `utilization_of` is the signal an autoscaler acts on, so a
    /// permanently-zero reading makes a controller scale a melting node
    /// DOWN. `busy_slots` and `capacity` are this kind's own notion of
    /// both, and the engine smooths the ratio exactly as it does for an
    /// ordinary server, so the number means the same thing on the meter
    /// and to a controller.
    fn report_occupancy(&mut self, state: &dyn NodeStateLike, busy_slots: f64, capacity: f64);

    /// Mark a request as a write, so downstream kinds see the
    /// classification.
    fn mark_write(&mut self, req: &dyn ReqLike, is_write: bool);

    /// Create a detached request at `state` and dispatch it down one
    /// specific outgoing edge. For kinds that ORIGINATE traffic on their
    /// own schedule: a stream broker delivering to a consumer group, a
    /// pub/sub topic fanning a publish out, a cron job dumping its batch.
    /// The message is a detached root (nothing upstream waits on it), its
    /// outcome is reported back via `on_downstream_result` when the
    /// behaviour declares `observes_outcome`, and it books
    /// arrivals/completions at the nodes it visits like any request.
    ///
    /// Returns `false` without emitting when the edge is cut, the target
    /// node does not exist, or the engine's live-request ceiling is hit;
    /// the caller decides what an undeliverable message means. Consumes no
    /// randomness.
    fn emit_detached(&mut self, state: &dyn NodeStateLike, edge: &SimEdge, key: u32) -> bool;
}
