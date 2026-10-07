//! Messaging and coordination component behaviours: streambroker, pubsub,
//! websocket, apigateway, sidecar, lambda, cron. Port of
//! `src/sim/behaviour-messaging.ts`.
//!
//! These are the kinds real systems are glued together with: the
//! partitioned log between services, the fan-out topic, the long-lived
//! connection gateway, the front door, the per-service proxy, the function
//! that scales itself, and the batch job that arrives on a clock.
//!
//! Every one of them is a plain `ComponentBehaviour`. Three of them
//! (streambroker, pubsub, cron) ORIGINATE traffic of their own, which they
//! do through `ctx.emit_detached()`: a detached message that books its
//! arrivals and completions at the nodes it visits and is never waited on
//! upstream, exactly like a queue message drained by a worker.
//!
//! Determinism notes, which constrain everything here:
//!  - No behaviour in this module draws from the RNG except the
//!    apigateway's auth roll, which is taken in a fixed position at
//!    admission and only when `auth_fail_rate > 0` (the same
//!    conditional-on-config pattern the engine's own `error_rate` roll
//!    uses).
//!  - Anything asking "has enough time passed?" compares simulated
//!    timestamps, never accumulated per-tick deltas.
//!  - The cron fires from `on_tick`, which quantises its firing instant to
//!    the `advance()` boundary; that is the same latitude the autoscaler
//!    already takes for its decisions, and the fire time is derived from
//!    simulated time, so a given `advance()` pattern replays exactly.
//!  - Connection and warm-pool expiries are processed LAZILY at admission
//!    by comparing stored absolute expiry times against `ctx.now()`, so an
//!    admission decision never depends on whether a tick happened to run.
//!
//! ## Flagged for the engine-writing agent: streambroker reentrancy
//!
//! `pump_group`'s `pumping` guard exists because the TS source deliberately
//! relies on `ctx.emitDetached()` being able to resolve SYNCHRONOUSLY for
//! an instant-ack consumer, which re-enters `pumpGroup` through
//! `onDownstreamResult` before `emitDetached()` "returns" -- ordinary
//! reentrancy in a GC'd, single-threaded language. This file ports that
//! guard faithfully (`ext.groups[gi].pumping`), and keeps `BrokerExt`
//! behind a plain `Box`, matching every other non-deferred kind in this
//! file, rather than `Arc<Mutex<_>>` (see `data.rs`'s module doc for where
//! that wrapper is used and why): `Mutex` is not reentrant, so wrapping
//! `BrokerExt` in one would turn a synchronous re-entrant call into a
//! deadlock rather than a caught bug. Left as a plain `Box`, the SAME
//! reentrancy, if the engine's `emit_detached` ever calls back into this
//! node's `on_downstream_result` while `on_service_complete`/`pump_group`
//! still holds `ext: &mut Ext` for this node, will fail to COMPILE instead
//! (two live `&mut Ext` borrows for the same node), which is the outcome
//! worth having: it forces `sim::engine` to resolve the question of
//! whether `emit_detached` may re-enter behaviour hooks synchronously
//! before this code can even build, rather than deadlocking silently at
//! run time. This is unresolved and explicitly NOT worked around here.

use crate::sim::behaviour::{
    clamp01, AdmitAction, ComponentBehaviour, CompleteAction, Ext, InstanceModel, PumpMode,
    RouteMode, ScaleField,
};
use crate::sim::engine_types::{BehaviourCtx, NodeStateLike, ReqLike};
use crate::sim::types::{BreakerState, EdgeState, FailureReason, NodeKind, NodeStats, SimEdge};
use std::sync::{Arc, Mutex};

/// `clampInt(v, min, fallback)` from the TS source: floors `v`, falls back
/// when absent or NaN, and floors the result to `min`.
fn clamp_int(v: Option<f64>, min: i64, fallback: i64) -> i64 {
    match v {
        Some(x) if !x.is_nan() => {
            let n = x.floor() as i64;
            if n < min {
                min
            } else {
                n
            }
        }
        _ => fallback,
    }
}

/* ================================================================== *
 * streambroker -- a partitioned, replayable log (Kafka-shaped)
 * ================================================================== */

/// Per-partition retention floor, so a many-partition log still holds
/// something.
const MIN_RETENTION_PER_PARTITION: i64 = 16;
/// Hard cap on total retained messages, whatever the slider says.
const MAX_TOTAL_RETENTION: i64 = 65536;

struct BrokerGroup {
    /// Outgoing edge this group consumes through.
    edge_id: String,
    /// Target node id, used to correlate delivery results back to the
    /// group.
    target_id: String,
    /// Next sequence number to deliver, per partition.
    next: Vec<f64>,
    /// True while a delivery for this partition is in flight (order
    /// preserved).
    inflight: Vec<bool>,
    /// Deliveries resolved (ok or not) since sim start.
    done: f64,
    /// Messages skipped because they aged out of retention.
    skipped: f64,
    /// True while `pump_group()` is draining this group. An instant-ack
    /// consumer (a queue, a retry queue, a write-behind cache) resolves a
    /// delivery SYNCHRONOUSLY inside `emit_detached()`, which re-enters
    /// `pump_group` through `on_downstream_result`; the flag turns that
    /// recursion into one iterative drain loop instead of a stack that
    /// grows with the backlog. See this module's doc comment for the open
    /// question about whether this reentrancy is even expressible once
    /// `sim::engine` exists.
    pumping: bool,
}

struct BrokerExt {
    /// Partition count the vectors were sized for; a change rebuilds them.
    partitions: i64,
    /// Ring buffer of message keys per partition.
    rings: Vec<Vec<u32>>,
    /// Messages ever appended per partition (the head sequence).
    head: Vec<f64>,
    /// Retention per partition, in messages (the ring size).
    retention: i64,
    /// One group per non-control outgoing edge, in edge order.
    groups: Vec<BrokerGroup>,
    /// Joined edge ids, to detect rewiring without comparing arrays.
    group_sig: String,
}

fn broker_partitions(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().partitions, 1, 4)
}

fn broker_retention(state: &dyn NodeStateLike, partitions: i64) -> i64 {
    let floored = state.config().queue_limit.floor() as i64;
    let total = floored.max(1).min(MAX_TOTAL_RETENTION);
    let per = (total as f64 / partitions as f64).ceil() as i64;
    per.max(MIN_RETENTION_PER_PARTITION)
}

fn make_broker_group(edge: &SimEdge, partitions: usize, start_at: &[f64]) -> BrokerGroup {
    BrokerGroup {
        edge_id: edge.id.clone(),
        target_id: edge.to.clone(),
        next: start_at.to_vec(),
        inflight: vec![false; partitions],
        done: 0.0,
        skipped: 0.0,
        pumping: false,
    }
}

/// Keep the broker's structure in step with its config and wiring.
///
/// A partition-count change rebuilds the log and resets every cursor to
/// the head; changing the shape of a log discards in-flight position in
/// reality too (a Kafka repartition is a migration, not a slider), and
/// starting the groups caught-up is the least surprising of the honest
/// options. A rewired edge set rebuilds only the groups; a NEW group
/// starts at the current head, which is exactly a new Kafka group
/// starting at 'latest'.
fn ensure_broker(state: &dyn NodeStateLike, ext: &mut BrokerExt) {
    let partitions = broker_partitions(state);
    if partitions != ext.partitions {
        let retention = broker_retention(state, partitions);
        ext.partitions = partitions;
        ext.retention = retention;
        ext.rings = (0..partitions).map(|_| vec![0u32; retention as usize]).collect();
        ext.head = vec![0.0; partitions as usize];
        ext.groups = Vec::new();
        ext.group_sig = " stale".to_string(); // force group rebuild below
    }
    let retention = broker_retention(state, partitions);
    if retention != ext.retention {
        // Retention slider moved: resize the rings. Contents restart
        // empty and cursors snap to head, for the same repartition
        // honesty as above.
        ext.retention = retention;
        for p in 0..partitions as usize {
            ext.rings[p] = vec![0u32; retention as usize];
        }
        for g in ext.groups.iter_mut() {
            g.next = ext.head.clone();
            for f in g.inflight.iter_mut() {
                *f = false;
            }
        }
    }

    let mut sig = String::new();
    for e in state.out() {
        sig.push_str(&e.id);
        sig.push(' ');
    }
    if sig != ext.group_sig {
        let mut prev: std::collections::HashMap<String, BrokerGroup> =
            ext.groups.drain(..).map(|g| (g.edge_id.clone(), g)).collect();
        let mut groups = Vec::new();
        for edge in state.out() {
            if let Some(mut kept) = prev.remove(&edge.id) {
                if kept.next.len() == partitions as usize {
                    kept.target_id = edge.to.clone();
                    groups.push(kept);
                    continue;
                }
            }
            groups.push(make_broker_group(edge, partitions as usize, &ext.head));
        }
        ext.groups = groups;
        ext.group_sig = sig;
    }
}

fn partition_of(key: u32, partitions: i64) -> usize {
    let k = key as i64;
    (((k % partitions) + partitions) % partitions) as usize
}

/// Total messages this group still has ahead of it, deliveries in flight
/// included.
fn group_lag(ext: &BrokerExt, g: &BrokerGroup) -> f64 {
    let mut lag = 0.0;
    for p in 0..ext.partitions as usize {
        lag += ext.head[p] - g.next[p];
        lag += if g.inflight[p] { 1.0 } else { 0.0 };
    }
    lag
}

/// Deliver as much as this group is allowed to have in flight: at most one
/// message per partition, in partition order. Called whenever new
/// messages land and whenever one of the group's deliveries resolves, so
/// delivery is entirely event-driven and never depends on the frame
/// clock.
fn pump_group(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut BrokerExt, gi: usize, edge: &SimEdge) {
    // Re-entered from on_downstream_result while already draining: the
    // active drain loop below will pick the freed partition up on its
    // next pass.
    if ext.groups[gi].pumping {
        return;
    }
    ext.groups[gi].pumping = true;
    let mut progress = true;
    while progress {
        progress = false;
        for p in 0..ext.partitions as usize {
            if ext.groups[gi].inflight[p] {
                continue;
            }
            // Skip past anything that already aged out of retention.
            // Those messages are gone for this group; that loss is the
            // retention_drop_rate readout.
            let oldest = ext.head[p] - ext.retention as f64;
            if ext.groups[gi].next[p] < oldest {
                let skipped = oldest - ext.groups[gi].next[p];
                ext.groups[gi].skipped += skipped;
                ctx.count_custom(state, "retentionDrop", skipped);
                ext.groups[gi].next[p] = oldest;
            }
            if ext.groups[gi].next[p] >= ext.head[p] {
                continue;
            }
            let idx = (ext.groups[gi].next[p] as i64 as usize) % ext.retention as usize;
            let key = ext.rings[p][idx];
            // Mark the partition in flight BEFORE emitting: an
            // instant-ack consumer resolves the delivery synchronously
            // inside emit_detached, and on_downstream_result must find
            // the flag set to correlate it (it clears the flag, and this
            // loop's next pass re-fills the slot).
            ext.groups[gi].inflight[p] = true;
            ext.groups[gi].next[p] += 1.0;
            // A cut edge or missing target refuses the emit: the message
            // stays where it is and the group's lag keeps growing, which
            // is exactly what a partitioned broker link looks like from
            // the outside.
            if !ctx.emit_detached(state, edge, key) {
                ext.groups[gi].inflight[p] = false;
                ext.groups[gi].next[p] -= 1.0;
                ext.groups[gi].pumping = false;
                return;
            }
            if !ext.groups[gi].inflight[p] {
                progress = true;
            }
        }
    }
    ext.groups[gi].pumping = false;
}

fn pump_all_groups(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut BrokerExt) {
    for i in 0..ext.groups.len() {
        match state.out().get(i) {
            Some(edge) if edge.id == ext.groups[i].edge_id => {
                let edge = edge.clone();
                pump_group(ctx, state, ext, i, &edge);
            }
            _ => continue,
        }
    }
}

/// WHY THIS IS NOT THE QUEUE. The simple `queue` is one FIFO with one set
/// of competing consumers: a message is taken once and gone. A log is
/// different in every way that matters:
///
///  - PARTITIONED: a message lands in `key % partitions`, and order is
///    preserved per partition, so one consumer group can process at most
///    `partitions` messages in parallel. Partition count, not consumer
///    count, is the parallelism ceiling; that is the single most
///    misunderstood fact about Kafka and it falls straight out of this
///    model.
///  - MULTI-READER: every outgoing edge is an independent CONSUMER GROUP
///    with its own cursor over the same messages. A slow analytics group
///    falls behind without costing the billing group anything.
///  - LAG, NOT BACKPRESSURE: producers are acked immediately no matter how
///    far behind the consumers are. The cost of a slow consumer is
///    invisible to the producer and shows up only as CONSUMER LAG, the
///    headline metric here.
///  - BOUNDED BY RETENTION: `queue_limit` is the log's retention in
///    messages. A group that falls further behind than that skips forward
///    and the skipped messages are simply lost to it; nonzero
///    `retention_drop_rate` means "you are not just slow, you are losing
///    data".
pub struct StreamBrokerBehaviour;

impl ComponentBehaviour for StreamBrokerBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::StreamBroker
    }
    // A broker is a buffer with cursors, not a server: its ack slots mean
    // nothing, and its meaningful backlog is lag, published via
    // decorate_stats.
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
    // Deliveries join back through this node; crediting them would
    // double-count the broker's throughput, whose honest meaning is the
    // PUBLISH rate booked by the ack path.
    fn credits_join_completion(&self) -> bool {
        false
    }
    // The whole pacing mechanism: a delivery resolving is what frees its
    // partition and triggers the next one.
    fn observes_outcome(&self) -> bool {
        true
    }
    // One unit per partition, like a shard: the per-partition backlog is
    // where a hot partition becomes visible.
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Custom)
    }

    fn init_state(&self, state: &dyn NodeStateLike) -> Ext {
        let partitions = broker_partitions(state);
        let retention = broker_retention(state, partitions);
        Some(Box::new(BrokerExt {
            partitions,
            rings: (0..partitions).map(|_| vec![0u32; retention as usize]).collect(),
            head: vec![0.0; partitions as usize],
            retention,
            groups: Vec::new(),
            group_sig: String::new(),
        }))
    }

    // A publish is acked after the broker's own (tiny) service_ms, no
    // matter what the consumers are doing. That decoupling IS the
    // product.
    fn on_admit(
        &self,
        _ctx: &mut dyn BehaviourCtx,
        _state: &dyn NodeStateLike,
        _req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> AdmitAction {
        AdmitAction::Passthru
    }

    fn on_service_complete(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> CompleteAction {
        let e = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BrokerExt>())
            .expect("streambroker ext");
        ensure_broker(state, e);
        let p = partition_of(req.key(), e.partitions);
        let idx = (e.head[p] as i64 as usize) % e.retention as usize;
        e.rings[p][idx] = req.key();
        e.head[p] += 1.0;
        ctx.count_custom(state, "published", 1.0);
        pump_all_groups(ctx, state, e);
        // The producer's call ends here; deliveries are detached and
        // downstream never sees the publish request itself.
        CompleteAction::Complete
    }

    fn on_downstream_result(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ok: bool,
        _reason: FailureReason,
        ext: &mut Ext,
    ) {
        let e = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BrokerExt>())
            .expect("streambroker ext");
        ensure_broker(state, e);
        // Correlate the finished delivery back to its group (by the
        // consumer it landed on) and partition (by the key it carried).
        // At most one delivery per (group, partition) is ever in flight,
        // so the pair identifies it.
        let p = partition_of(req.key(), e.partitions);
        for i in 0..e.groups.len() {
            if e.groups[i].target_id != req.node_id() || !e.groups[i].inflight[p] {
                continue;
            }
            e.groups[i].inflight[p] = false;
            e.groups[i].done += 1.0;
            ctx.count_custom(state, "delivered", 1.0);
            if !ok {
                ctx.count_custom(state, "deliveryFailed", 1.0);
            }
            if let Some(edge) = state.out().get(i) {
                if edge.id == e.groups[i].edge_id {
                    let edge = edge.clone();
                    pump_group(ctx, state, e, i, &edge);
                }
            }
            return;
        }
    }

    /// Unit p is partition p, filled with the WORST group's backlog in
    /// that partition against retention; the partition strip therefore
    /// shows where in the keyspace the slow consumer is drowning.
    fn report_instances(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut Ext) {
        let e = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BrokerExt>())
            .expect("streambroker ext");
        ensure_broker(state, e);
        let mut out = vec![0.0f64; e.partitions as usize];
        for p in 0..e.partitions as usize {
            let mut worst = 0.0f64;
            for g in &e.groups {
                let behind = e.head[p] - g.next[p] + if g.inflight[p] { 1.0 } else { 0.0 };
                if behind > worst {
                    worst = behind;
                }
            }
            out[p] = clamp01(worst / e.retention as f64);
        }
        ctx.report_instances(state, &out, 0.0);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let e = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<BrokerExt>())
            .expect("streambroker ext");
        ensure_broker(state, e);
        let mut max_lag = 0.0f64;
        let mut inflight_total = 0.0f64;
        let mut by_group = Vec::with_capacity(e.groups.len());
        for g in &e.groups {
            let lag = group_lag(e, g);
            by_group.push(lag);
            if lag > max_lag {
                max_lag = lag;
            }
            for p in 0..e.partitions as usize {
                if g.inflight[p] {
                    inflight_total += 1.0;
                }
            }
        }
        stats.consumer_lag = Some(max_lag);
        stats.consumer_lag_by_group = Some(by_group);
        stats.delivery_rate = Some(ctx.counter_rate(state, "delivered"));
        stats.retention_drop_rate = Some(ctx.counter_rate(state, "retentionDrop"));
        // The generic meters, told the truth for this kind: the backlog
        // is the worst group's lag (drawn against queue_limit, which is
        // retention), and what is "in flight" is deliveries out at
        // consumers.
        stats.queued = max_lag;
        stats.in_flight = inflight_total;
    }
}

/* ================================================================== *
 * pubsub -- fan-out of one publish to every subscriber
 * ================================================================== */

/// The lesson is AMPLIFICATION. One publish becomes one delivery per
/// subscriber edge, all detached and all independent: a slow subscriber
/// queues and sheds at its own node without delaying the others or the
/// publisher, but the total work in the system is N times what the
/// publisher thinks it sent. Wiring five subscribers onto a 100 rps
/// publisher quietly makes 500 rps of load, and the `delivery_rate`
/// readout is that multiplication printed as a number.
///
/// Unlike the streambroker there is no cursor, no retention and no
/// pacing: delivery is immediate and at-most-once. If a subscriber cannot
/// keep up the excess is shed at the subscriber, not remembered here,
/// which is the honest difference between a topic and a log.
pub struct PubSubBehaviour;

impl ComponentBehaviour for PubSubBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::PubSub
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

    fn on_service_complete(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        _ext: &mut Ext,
    ) -> CompleteAction {
        for edge in state.out() {
            if ctx.emit_detached(state, edge, req.key()) {
                ctx.count_custom(state, "delivered", 1.0);
            }
        }
        CompleteAction::Complete
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        _ext: &mut Ext,
    ) {
        stats.fanout = Some(state.out().len() as f64);
        let delivered = ctx.counter_rate(state, "delivered");
        stats.delivery_rate = Some(delivered);
        stats.publish_amplification = Some(delivered);
    }
}

/* ================================================================== *
 * websocket -- capacity measured in CONNECTIONS HELD, not requests served
 * ================================================================== */

/// A long-lived-connection gateway. Every accepted connection occupies one
/// of `instances * capacity` slots for `connection_ms` of simulated time,
/// regardless of how quickly its handshake was served. By Little's law the
/// gateway settles at `rps * connection_ms / 1000` concurrent connections,
/// so a chat gateway taking a mere 40 conn/s with 30-second sessions is
/// holding 1200 connections, and THAT number, not the request rate, is
/// what it saturates on. This is why chat systems shard by connection
/// count and why Discord's gateway tier looks nothing like its API tier.
///
/// A connection that finds every slot held is refused as 'conn-refused'
/// immediately; there is no queue to wait in, because a socket you cannot
/// accept is a socket the client retries against some other gateway.
///
/// Expiry is lazy and exact: expiries are absolute simulated times in FIFO
/// order (the hold time is a constant read from config at accept time), so
/// releasing them at the next admission reproduces identically whatever
/// the frame cadence was.
struct WsExt {
    /// Absolute expiry time of each open connection, oldest first.
    expiry: Vec<f64>,
    /// Read cursor into `expiry`, so release is O(1).
    head: usize,
}

fn ws_open(ext: &WsExt) -> i64 {
    (ext.expiry.len() - ext.head) as i64
}

/// Drop every connection whose lifetime has ended, as of `now`.
fn ws_expire(ext: &mut WsExt, now: f64) {
    while ext.head < ext.expiry.len() && ext.expiry[ext.head] <= now {
        ext.head += 1;
    }
    if ext.head > 64 && ext.head * 2 >= ext.expiry.len() {
        ext.expiry.drain(0..ext.head);
        ext.head = 0;
    }
}

/// Open connections as of `now`, WITHOUT mutating: for pure snapshot
/// reads.
fn ws_open_projected(ext: &WsExt, now: f64) -> i64 {
    let mut head = ext.head;
    while head < ext.expiry.len() && ext.expiry[head] <= now {
        head += 1;
    }
    (ext.expiry.len() - head) as i64
}

pub struct WebSocketBehaviour;

impl ComponentBehaviour for WebSocketBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::WebSocket
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
    // Slot bookkeeping is connection-based and lives in `ext`; the
    // engine's busy/waiting pair never sees it, so there is nothing for it
    // to pump.
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
        true
    }
    // One unit per instance; the waterline is connection occupancy, fed
    // to the engine through report_occupancy below.
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(WsExt { expiry: Vec::new(), head: 0 }))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let e = ext.as_mut().and_then(|e| e.downcast_mut::<WsExt>()).expect("websocket ext");
        ws_expire(e, ctx.now());

        let cap = ctx.effective_capacity(state) as i64;
        if ws_open(e) >= cap {
            ctx.count_custom(state, "connRefused", 1.0);
            ctx.reject(state, req, FailureReason::ConnRefused);
            return AdmitAction::Handled;
        }

        let hold_ms = state.config().connection_ms.unwrap_or(30000.0).max(0.0);
        e.expiry.push(ctx.now() + hold_ms);
        ctx.count_custom(state, "connected", 1.0);

        // The HANDSHAKE is served now (service_ms), and the request
        // continues downstream (session setup, auth) like any other call;
        // the connection slot stays held long after the handshake
        // resolved, which is the whole resource model. Nothing to do on
        // drain: release is time-based.
        ctx.serve_within(state, req, Box::new(ws_noop_drain));
        AdmitAction::Handled
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let e = ext.as_mut().and_then(|e| e.downcast_mut::<WsExt>()).expect("websocket ext");
        ws_expire(e, ctx.now());
        let open = ws_open(e) as f64;
        let cap = ctx.effective_capacity(state);
        ctx.report_occupancy(state, open, cap);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let e = ext.as_mut().and_then(|e| e.downcast_mut::<WsExt>()).expect("websocket ext");
        let open = ws_open_projected(e, ctx.now()) as f64;
        stats.connections_open = Some(open);
        stats.max_connections = Some(ctx.effective_capacity(state));
        stats.connect_rate = Some(ctx.counter_rate(state, "connected"));
        stats.connection_reject_rate = Some(ctx.counter_rate(state, "connRefused"));
        // The generic in-flight meter means "connections held" here; the
        // handshake count is a detail nobody sizes a gateway by.
        stats.in_flight = open;
    }
}

fn ws_noop_drain(_ctx: &mut dyn BehaviourCtx, _state: &dyn NodeStateLike, _req: &dyn ReqLike) {
    // Connection slots are released by time, in ws_expire; the handshake
    // finishing frees nothing.
}

/* ================================================================== *
 * apigateway -- routing, auth and rate limiting in one front door
 * ================================================================== */

/// The front door of a real API: one component that authenticates, rate
/// limits, and routes to whichever backend owns the path. Mechanically it
/// is a token bucket (reusing `rate_limit_rps`/`burst`), an auth check
/// (`auth_fail_rate`, refused as 'unauthorized'), and a weighted
/// single-edge router (edge weights are the route table), in front of
/// real slots whose `service_ms` is the gateway's own processing cost.
///
/// Order at the door is bucket first, then auth: a burst of bad
/// credentials still consumes rate-limit tokens, as it does in life, and
/// the ordering is fixed so the RNG stream never depends on which check
/// happens to fail.
struct GatewayBucket {
    tokens: f64,
    last_refill_ms: f64,
    last_burst: f64,
}

fn gw_rate(state: &dyn NodeStateLike) -> f64 {
    match state.config().rate_limit_rps {
        Some(r) if r > 0.0 => r,
        _ => 0.0,
    }
}

fn gw_burst(state: &dyn NodeStateLike) -> f64 {
    if let Some(b) = state.config().burst {
        if b > 0.0 {
            return b;
        }
    }
    let r = gw_rate(state);
    if r > 0.0 {
        r
    } else {
        1.0
    }
}

/// Continuous refill against elapsed simulated time; same math as
/// `ratelimiter` in `edge.rs`.
fn gw_refill(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, b: &mut GatewayBucket) {
    let burst = gw_burst(state);
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
    let rate = gw_rate(state);
    if rate <= 0.0 {
        return;
    }
    b.tokens += (elapsed / 1000.0) * rate;
    if b.tokens > burst {
        b.tokens = burst;
    }
}

/// Bucket level as of now, without mutating: snapshotting must stay pure.
fn gw_projected_tokens(ctx: &dyn BehaviourCtx, state: &dyn NodeStateLike, b: &GatewayBucket) -> f64 {
    let burst = gw_burst(state);
    let mut tokens = if b.tokens > burst { burst } else { b.tokens };
    let rate = gw_rate(state);
    let elapsed = ctx.now() - b.last_refill_ms;
    if rate > 0.0 && elapsed > 0.0 {
        tokens += (elapsed / 1000.0) * rate;
        if tokens > burst {
            tokens = burst;
        }
    }
    tokens
}

pub struct ApiGatewayBehaviour;

impl ComponentBehaviour for ApiGatewayBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::ApiGateway
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
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
    }

    fn init_state(&self, state: &dyn NodeStateLike) -> Ext {
        let burst = gw_burst(state);
        Some(Box::new(GatewayBucket {
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
            .and_then(|e| e.downcast_mut::<GatewayBucket>())
            .expect("apigateway ext");
        gw_refill(ctx, state, b);

        if gw_rate(state) > 0.0 {
            if b.tokens < 1.0 {
                ctx.count_custom(state, "throttled", 1.0);
                ctx.reject(state, req, FailureReason::Throttled);
                return AdmitAction::Handled;
            }
            b.tokens -= 1.0;
        }

        let auth_fail = clamp01(state.config().auth_fail_rate.unwrap_or(0.0));
        if auth_fail > 0.0 && ctx.roll() < auth_fail {
            ctx.count_custom(state, "authRejected", 1.0);
            ctx.reject(state, req, FailureReason::Unauthorized);
            return AdmitAction::Handled;
        }

        ctx.count_custom(state, "admitted", 1.0);
        // Real slots and a real queue: auth and routing cost service_ms
        // each.
        AdmitAction::Serve
    }

    // The route table: exactly one backend per request, chosen by edge
    // weight.
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

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        stats.admitted_rate = Some(ctx.counter_rate(state, "admitted"));
        stats.throttled_rate = Some(ctx.counter_rate(state, "throttled"));
        stats.auth_reject_rate = Some(ctx.counter_rate(state, "authRejected"));
        stats.tokens = Some(match ext.as_ref().and_then(|e| e.downcast_ref::<GatewayBucket>()) {
            Some(b) => gw_projected_tokens(ctx, state, b),
            None => 0.0,
        });
    }
}

/* ================================================================== *
 * sidecar -- the per-service proxy, and what it costs
 * ================================================================== */

/// WHY 'sidecar' AND NOT 'serviceMesh'. A mesh is a property of the whole
/// graph -- every hop proxied -- and this simulator's unit of composition
/// is the NODE. A sidecar maps one-to-one onto that: the student places
/// one proxy in front of one service, and builds a mesh the way a mesh is
/// actually built, by putting a sidecar at every hop and watching the
/// latency taxes stack. A single 'serviceMesh' node would have to act on
/// edges it does not own, which is exactly the shape the behaviour
/// registry exists to avoid.
///
/// What it teaches: infrastructure is never free. The proxy charges its
/// `service_ms` on every single request in exchange for retries
/// (`config.retries`, run by the engine's own retry machinery), outlier
/// ejection (a consecutive-failure circuit, simpler than the breaker's
/// windowed rate on purpose; Envoy ships exactly this policy), and
/// observability (its readouts are the upstream's health, seen from the
/// caller's side). Put a sidecar at every hop of a five-hop chain and the
/// p50 grows by five taxes; remove them and lose the retries that were
/// hiding the flaky dependency. Both runs are one checkbox apart, and the
/// comparison is the lesson.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidecarPhase {
    Closed,
    Open,
    HalfOpen,
}

struct SidecarExt {
    phase: SidecarPhase,
    /// Simulated time the proxy last ejected its upstream.
    opened_at_ms: f64,
    /// Consecutive downstream failures observed while closed.
    consecutive: i64,
    /// True while the single half-open probe is out.
    probing: bool,
    /// Ejections since sim start.
    trips: f64,
}

fn sidecar_outlier_after(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().outlier_after, 1, 5)
}

fn sidecar_open_ms(state: &dyn NodeStateLike) -> f64 {
    match state.config().open_ms {
        Some(v) if v > 0.0 => v,
        _ => 3000.0,
    }
}

pub struct SidecarBehaviour;

impl ComponentBehaviour for SidecarBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Sidecar
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
    fn observes_outcome(&self) -> bool {
        true
    }
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Slots)
    }
    fn scale_field(&self) -> Option<ScaleField> {
        Some(ScaleField::Instances)
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(SidecarExt {
            phase: SidecarPhase::Closed,
            opened_at_ms: 0.0,
            consecutive: 0,
            probing: false,
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
        let st = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<SidecarExt>())
            .expect("sidecar ext");

        // Lazy transition on simulated time, exactly like the breaker: the
        // ejection has served its open_ms, so the next request may probe.
        if st.phase == SidecarPhase::Open && ctx.now() - st.opened_at_ms >= sidecar_open_ms(state) {
            st.phase = SidecarPhase::HalfOpen;
            st.probing = false;
        }

        if st.phase == SidecarPhase::Open || (st.phase == SidecarPhase::HalfOpen && st.probing) {
            ctx.count_custom(state, "rejected", 1.0);
            ctx.reject(state, req, FailureReason::Rejected);
            return AdmitAction::Handled;
        }

        if st.phase == SidecarPhase::HalfOpen {
            st.probing = true;
        }
        // The latency tax: real slots, real service_ms, on every request.
        AdmitAction::Serve
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
        let st = ext
            .as_mut()
            .and_then(|e| e.downcast_mut::<SidecarExt>())
            .expect("sidecar ext");
        if !ok {
            ctx.count_custom(state, "upstreamFail", 1.0);
        }

        if st.phase == SidecarPhase::HalfOpen {
            st.probing = false;
            if ok {
                st.phase = SidecarPhase::Closed;
                st.consecutive = 0;
            } else {
                st.phase = SidecarPhase::Open;
                st.opened_at_ms = ctx.now();
                st.trips += 1.0;
            }
            return;
        }
        if st.phase != SidecarPhase::Closed {
            return;
        }

        if ok {
            st.consecutive = 0;
            return;
        }
        st.consecutive += 1;
        if st.consecutive >= sidecar_outlier_after(state) {
            st.phase = SidecarPhase::Open;
            st.opened_at_ms = ctx.now();
            st.consecutive = 0;
            st.trips += 1.0;
        }
    }

    fn edge_state_for(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        _edge: &SimEdge,
        _index: usize,
        ext: &mut Ext,
    ) -> Option<EdgeState> {
        let st = ext.as_mut().and_then(|e| e.downcast_mut::<SidecarExt>())?;
        let open = st.phase == SidecarPhase::Open && ctx.now() - st.opened_at_ms < sidecar_open_ms(state);
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
        let st = match ext.as_mut().and_then(|e| e.downcast_mut::<SidecarExt>()) {
            Some(s) => s,
            None => return,
        };
        let phase = if st.phase == SidecarPhase::Open && ctx.now() - st.opened_at_ms >= sidecar_open_ms(state) {
            SidecarPhase::HalfOpen
        } else {
            st.phase
        };
        stats.breaker_state = Some(match phase {
            SidecarPhase::Closed => BreakerState::Closed,
            SidecarPhase::Open => BreakerState::Open,
            SidecarPhase::HalfOpen => BreakerState::HalfOpen,
        });
        stats.breaker_trips = Some(st.trips);
        stats.consecutive_fails = Some(st.consecutive as f64);
        stats.rejected_rate = Some(ctx.counter_rate(state, "rejected"));
        stats.upstream_fail_rate = Some(ctx.counter_rate(state, "upstreamFail"));
    }
}

/* ================================================================== *
 * lambda -- serverless: instant scale, cold starts
 * ================================================================== */

/// A function-as-a-service pool. There is no fixed fleet: an invocation
/// that finds a WARM idle instance starts immediately; one that does not
/// pays `cold_start_ms` on top of its service time while the platform
/// provisions an instance. Finished instances sit warm for `keep_warm_ms`
/// and are then reclaimed, and reuse is most-recent-first (LIFO), which is
/// what real platforms do and is exactly why the warm pool shrinks to fit
/// steady traffic and gets caught flat by a burst.
///
/// The visible consequences, all real in the numbers: an idle lambda's
/// first request is slow; a cron burst arriving at a cold pool pays
/// `cold_start_ms` almost across the board (watch `cold_start_rate` spike
/// on the cron's period); and beyond `max_concurrency` the platform simply
/// throttles, because a lambda has no queue.
///
/// Uses the `Arc<Mutex<_>>` ext pattern (see `data.rs`'s module doc): the
/// `on_drained` callback given to `serve_within` must decrement `busy` and
/// push a fresh warm-pool expiry, which needs `ext` access from inside a
/// `'static` closure that `NodeStateLike` does not expose it through.
struct LambdaExt {
    /// Absolute reclaim time of each warm idle instance, oldest first.
    warm_expiry: Vec<f64>,
    /// Read cursor into `warm_expiry`: entries before it are gone.
    head: usize,
    /// Invocations running right now.
    busy: i64,
}

fn lambda_ext(ext: &Ext) -> Arc<Mutex<LambdaExt>> {
    ext.as_ref()
        .and_then(|e| e.downcast_ref::<Arc<Mutex<LambdaExt>>>())
        .expect("lambda ext")
        .clone()
}

fn lambda_limit(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().max_concurrency, 1, 40)
}

/// Reclaim warm instances whose keep-alive ended, as of `now`.
fn lambda_reap(ext: &mut LambdaExt, now: f64) {
    while ext.head < ext.warm_expiry.len() && ext.warm_expiry[ext.head] <= now {
        ext.head += 1;
    }
    if ext.head > 64 && ext.head * 2 >= ext.warm_expiry.len() {
        ext.warm_expiry.drain(0..ext.head);
        ext.head = 0;
    }
}

/// Warm idle count as of `now` without mutating, for pure snapshot reads.
fn lambda_warm_projected(ext: &LambdaExt, now: f64) -> i64 {
    let mut head = ext.head;
    while head < ext.warm_expiry.len() && ext.warm_expiry[head] <= now {
        head += 1;
    }
    (ext.warm_expiry.len() - head) as i64
}

pub struct LambdaBehaviour;

impl ComponentBehaviour for LambdaBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Lambda
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
    // One unit per live instance, busy or warm; the stack GROWS WITH
    // LOAD, which is the one picture that says "serverless" truthfully.
    fn instance_model(&self) -> Option<InstanceModel> {
        Some(InstanceModel::Custom)
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(Arc::new(Mutex::new(LambdaExt {
            warm_expiry: Vec::new(),
            head: 0,
            busy: 0,
        }))))
    }

    fn on_admit(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        req: &dyn ReqLike,
        ext: &mut Ext,
    ) -> AdmitAction {
        let handle = lambda_ext(ext);
        let mut e = handle.lock().unwrap();
        lambda_reap(&mut e, ctx.now());

        if e.busy >= lambda_limit(state) {
            drop(e);
            ctx.count_custom(state, "throttled", 1.0);
            ctx.reject(state, req, FailureReason::Throttled);
            return AdmitAction::Handled;
        }

        let warm = e.warm_expiry.len() - e.head;
        if warm > 0 {
            // Reuse the most recently freed instance (LIFO), leaving the
            // oldest to expire; this is why keep-warm does not accumulate
            // a large fleet.
            e.warm_expiry.pop();
            drop(e);
            ctx.count_custom(state, "warmStart", 1.0);
        } else {
            drop(e);
            ctx.count_custom(state, "coldStart", 1.0);
            let delay = state.config().cold_start_ms.unwrap_or(350.0).max(0.0);
            ctx.add_service_delay(req, delay);
        }

        {
            let mut e = handle.lock().unwrap();
            e.busy += 1;
        }
        let cb_handle = handle.clone();
        ctx.serve_within(
            state,
            req,
            Box::new(move |ctx, state, req| on_lambda_drained(ctx, state, req, &cb_handle)),
        );
        AdmitAction::Handled
    }

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let handle = lambda_ext(ext);
        let mut e = handle.lock().unwrap();
        lambda_reap(&mut e, ctx.now());
        let busy = e.busy;
        drop(e);
        ctx.report_occupancy(state, busy as f64, lambda_limit(state) as f64);
    }

    fn report_instances(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, ext: &mut Ext) {
        let handle = lambda_ext(ext);
        let e = handle.lock().unwrap();
        let warm = lambda_warm_projected(&e, ctx.now());
        // Cap the drawn stack; the badge carries the true count past this.
        let total = (e.busy + warm).min(64).max(0) as usize;
        let busy = e.busy;
        drop(e);
        let mut out = vec![0.0f64; total];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = if (i as i64) < busy { 1.0 } else { 0.0 };
        }
        ctx.report_instances(state, &out, 0.0);
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let handle = lambda_ext(ext);
        let e = handle.lock().unwrap();
        let cold = ctx.counter_rate(state, "coldStart");
        let warm_starts = ctx.counter_rate(state, "warmStart");
        let started = cold + warm_starts;
        stats.cold_start_rate = Some(if started > 0.0 { cold / started } else { 0.0 });
        stats.cold_starts_per_sec = Some(cold);
        stats.warm_idle = Some(lambda_warm_projected(&e, ctx.now()) as f64);
        stats.running_now = Some(e.busy as f64);
        stats.in_flight = e.busy as f64;
        drop(e);
        // The 'throttled' counter was already being booked at admission;
        // publish it so the UI can show refusals instead of inferring
        // them from errors.
        stats.throttled_rate = Some(ctx.counter_rate(state, "throttled"));
    }
}

fn on_lambda_drained(ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _req: &dyn ReqLike, handle: &Arc<Mutex<LambdaExt>>) {
    let mut e = handle.lock().unwrap();
    if e.busy > 0 {
        e.busy -= 1;
    }
    // The freed instance stays warm for keep_warm_ms from THIS moment.
    let keep = state.config().keep_warm_ms.unwrap_or(12000.0).max(0.0);
    e.warm_expiry.push(ctx.now() + keep);
}

/* ================================================================== *
 * cron -- scheduled batch load
 * ================================================================== */

/// A job on a clock. Every `interval_ms` it dumps `batch_size` detached
/// requests down EACH outgoing edge, effectively simultaneously; between
/// firings it does nothing at all. That burst shape is the entire lesson:
/// a database sized comfortably for its steady interactive load falls
/// over every time the report job lands on it, and the graphs show
/// interactive p95 spiking on the cron's period, which is exactly the
/// page a real on-call gets at midnight.
///
/// Firing happens from `on_tick`, so the instant is quantised to the
/// `advance()` boundary; the same latitude the autoscaler's decisions
/// already take, and fully replayable for a given `advance()` pattern.
/// Message keys cycle a deterministic counter rather than drawing
/// randomness, so a cron never perturbs anyone else's RNG stream. If
/// firings were missed (a long stall), ONE batch fires and the schedule
/// resumes from now; a real cron with `skip` overlap policy, not a
/// backlog bomb.
struct CronExt {
    /// Absolute simulated time of the next firing; -1 until initialised.
    next_fire_ms: f64,
    /// Total requests emitted since sim start.
    emitted: f64,
    /// Deterministic key sequence for emitted messages.
    key_seq: u32,
}

fn cron_interval(state: &dyn NodeStateLike) -> f64 {
    match state.config().interval_ms {
        Some(v) if v >= 250.0 => v.floor(),
        Some(v) if v > 0.0 => 250.0,
        _ => 20000.0,
    }
}

/// Requests per edge per firing, bounded so a slider cannot wedge the
/// heap.
fn cron_batch(state: &dyn NodeStateLike) -> i64 {
    clamp_int(state.config().batch_size, 1, 50).min(2000)
}

pub struct CronBehaviour;

impl ComponentBehaviour for CronBehaviour {
    fn kind(&self) -> NodeKind {
        NodeKind::Cron
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
        false
    }
    fn pump(&self) -> PumpMode {
        PumpMode::None
    }
    fn credits_join_completion(&self) -> bool {
        false
    }

    fn init_state(&self, _state: &dyn NodeStateLike) -> Ext {
        Some(Box::new(CronExt {
            next_fire_ms: -1.0,
            emitted: 0.0,
            key_seq: 0,
        }))
    }

    // Traffic INTO a cron is a wiring mistake, same as into an autoscaler.
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

    fn on_tick(&self, ctx: &mut dyn BehaviourCtx, state: &dyn NodeStateLike, _dt_ms: f64, ext: &mut Ext) {
        let st = ext.as_mut().and_then(|e| e.downcast_mut::<CronExt>()).expect("cron ext");
        let interval = cron_interval(state);
        if st.next_fire_ms < 0.0 {
            // First fire one full interval in, so a fresh topology
            // settles first and the burst reads as an event rather than
            // as part of startup.
            st.next_fire_ms = ctx.now() + interval;
            return;
        }
        if ctx.now() < st.next_fire_ms {
            return;
        }

        let batch = cron_batch(state);
        for i in 0..state.out().len() {
            let edge = state.out()[i].clone();
            for _ in 0..batch {
                if !ctx.emit_detached(state, &edge, st.key_seq) {
                    break;
                }
                st.key_seq = (st.key_seq + 1) % 64;
                st.emitted += 1.0;
            }
        }
        ctx.count_custom(state, "fired", 1.0);
        st.next_fire_ms = ctx.now() + interval;
    }

    fn decorate_stats(
        &self,
        ctx: &mut dyn BehaviourCtx,
        state: &dyn NodeStateLike,
        stats: &mut NodeStats,
        ext: &mut Ext,
    ) {
        let st = match ext.as_mut().and_then(|e| e.downcast_mut::<CronExt>()) {
            Some(s) => s,
            None => return,
        };
        stats.next_fire_in_ms = Some(if st.next_fire_ms < 0.0 {
            cron_interval(state)
        } else {
            (st.next_fire_ms - ctx.now()).max(0.0)
        });
        stats.batch_emitted = Some(st.emitted);
        // What the next firing will dump downstream, all edges together.
        // A pure read of config and wiring, so the readout can say "burst
        // 150" before the burst ever lands.
        stats.burst_size = Some(cron_batch(state) as f64 * state.out().len() as f64);
    }
}

/* ------------------------------------------------------------------ *
 * Registry
 * ------------------------------------------------------------------ */

static STREAMBROKER: StreamBrokerBehaviour = StreamBrokerBehaviour;
static PUBSUB: PubSubBehaviour = PubSubBehaviour;
static WEBSOCKET: WebSocketBehaviour = WebSocketBehaviour;
static APIGATEWAY: ApiGatewayBehaviour = ApiGatewayBehaviour;
static SIDECAR: SidecarBehaviour = SidecarBehaviour;
static LAMBDA: LambdaBehaviour = LambdaBehaviour;
static CRON: CronBehaviour = CronBehaviour;

/// The behaviours defined in this module, for registration in
/// `sim::behaviour`.
pub static BEHAVIOURS: &[(NodeKind, &'static dyn ComponentBehaviour)] = &[
    (NodeKind::StreamBroker, &STREAMBROKER),
    (NodeKind::PubSub, &PUBSUB),
    (NodeKind::WebSocket, &WEBSOCKET),
    (NodeKind::ApiGateway, &APIGATEWAY),
    (NodeKind::Sidecar, &SIDECAR),
    (NodeKind::Lambda, &LAMBDA),
    (NodeKind::Cron, &CRON),
];
