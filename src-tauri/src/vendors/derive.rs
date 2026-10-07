//! Faithful port of `src/content/vendors/derive.ts`.
//!
//! From a published spec to the engine's knobs.
//!
//! THIS FILE IS THE MODEL, AND IT IS THE ONLY ONE.
//!
//! Every number in the vendor data files (`data/vendors/*.json`) is a
//! published fact with a URL. Nothing here is. No vendor states how many
//! concurrent requests an instance serves, or how long each takes, because
//! that depends on what the software does: the same db.r6g.large answers
//! 50k trivial key reads a second or 20 heavy joins, and both are correct.
//!
//! The derivation below is therefore a DEFAULT, not a measurement, and it
//! is deliberately simple so that it is obviously a rule of thumb rather
//! than something that looks researched. Anything cleverer would invite
//! the reader to trust it more than it deserves.
//!
//! The interface labels every value that comes through here as derived,
//! and a student who wants their own system's behaviour is told to set
//! the knobs directly (`DERIVED_NOTE`). That honesty is the feature; the
//! numbers are scaffolding.

use crate::sim::types::{NodeConfig, NodeKind};
use crate::vendors::types::VendorSize;

/// Concurrent requests one vCPU is assumed to handle at a time.
///
/// One. A busy request occupies a core, so a 4 vCPU machine gets 4 slots.
///
/// This is wrong in both directions and is meant to be: an IO-bound
/// service holds thousands of connections on four cores, and a CPU-bound
/// one manages fewer than one per core once it is context switching. It is
/// here to give a switched vendor a starting point that scales with the
/// size a student picked, not to predict anything.
pub const SLOTS_PER_VCPU: f64 = 1.0;

/// Service time is NOT derived from the instance.
///
/// A bigger machine does not make a query faster; it makes more of them
/// run at once. Latency comes from the work, and the work is what the
/// student is modelling. Changing `serviceMs` when someone picks a larger
/// instance would teach the opposite of the thing this simulator exists to
/// teach, so whatever the component already had is kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Derived {
    pub capacity: f64,
    /// Always `None`. Present -- rather than leaving `Derived` a bare
    /// `f64` -- so the caller cannot forget the reason above: this struct
    /// mirrors the TS `{ capacity: number; serviceMs: null }`, where
    /// `serviceMs` is typed as the literal `null` for the same purpose.
    pub service_ms: Option<f64>,
}

/// Kinds where instance sizing is the thing that limits concurrency.
///
/// Kinds that are not sized this way get nothing: a CDN, a queue or a
/// managed pub/sub does not expose a vCPU count that a student is
/// choosing, and inventing a slot count for one would be exactly the
/// plausible-looking number AGENTS.md forbids.
pub static SIZED_KINDS: &[NodeKind] = &[
    NodeKind::Service,
    NodeKind::Db,
    NodeKind::Cache,
    NodeKind::Worker,
    NodeKind::Replica,
    NodeKind::Shard,
    NodeKind::SearchIndex,
    NodeKind::TimeSeriesDb,
    NodeKind::GraphDb,
    NodeKind::VectorDb,
    NodeKind::Transcoder,
];

pub fn is_sized_kind(kind: NodeKind) -> bool {
    SIZED_KINDS.contains(&kind)
}

/// Derive the engine knobs a vendor size implies.
///
/// Returns `None` where there is nothing honest to say, which the caller
/// must treat as "leave the component alone" rather than as a zero.
pub fn derive_from_size(kind: NodeKind, size: &VendorSize) -> Option<Derived> {
    if !is_sized_kind(kind) {
        return None;
    }

    // A size with no published vCPU count gets nothing. Azure Managed
    // Redis states its per-SKU counts only inside an image, and inventing
    // one to fill the gap is the plausible-looking number AGENTS.md
    // forbids.
    let vcpu = size.vcpu?;
    if !vcpu.is_finite() || vcpu <= 0.0 {
        return None;
    }

    // A published max-connections figure is a REAL ceiling, so where the
    // vendor states one it wins over the vCPU rule of thumb. This is the
    // one place the derivation gets to stand on a citable number.
    let from_vcpu = (vcpu * SLOTS_PER_VCPU).round().max(1.0);
    let capacity = match size.max_connections {
        Some(mc) if mc > 0.0 => from_vcpu.min(mc),
        _ => from_vcpu,
    };

    Some(Derived {
        capacity,
        service_ms: None,
    })
}

/// Apply a size to a config, leaving everything the derivation cannot
/// speak to untouched.
pub fn apply_size(kind: NodeKind, config: &NodeConfig, size: &VendorSize) -> NodeConfig {
    match derive_from_size(kind, size) {
        Some(derived) => NodeConfig {
            capacity: derived.capacity,
            ..config.clone()
        },
        None => config.clone(),
    }
}

/// The sentence shown wherever a derived number appears.
///
/// One string, in one place, so the caveat cannot drift between the
/// inspector and anywhere else that grows a need for it later.
pub const DERIVED_NOTE: &str = "Capacity is our estimate from the vCPU count, not a figure the vendor publishes. Set it yourself to model your own service.";
