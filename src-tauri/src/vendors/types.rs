//! Faithful port of `src/content/vendors/types.ts`.
//!
//! Cloud vendor mode. A student learning system design meets "load
//! balancer" first and "ALB" second, which is the right order: the concept
//! survives a decade, the product name does not. Generic therefore stays
//! the default, and a vendor is something you switch on once the idea has
//! landed.
//!
//! WHAT THIS DATA IS, AND IS NOT.
//!
//! Everything in the vendor files beside this one is PUBLISHED SPEC: a
//! vCPU count, a memory figure, a price, each with the URL it came from and
//! the date it was read. Those are facts, and AGENTS.md requires them to be
//! true.
//!
//! What is NOT a fact is how a vCPU count becomes the engine's `capacity`
//! and `serviceMs`. No vendor publishes "this instance serves N concurrent
//! requests at M milliseconds", because the answer depends entirely on
//! what the software is doing. Any mapping is a MODEL, and this project's
//! whole point is to not dress a model up as a measurement.
//!
//! So the two are kept apart on purpose. Specs are cited (this module and
//! `data.rs`). The derivation lives in one documented function (`derive.rs`),
//! is labelled as derived wherever it is shown, and a reader who wants the
//! real behaviour is told to set the knobs themselves.

use crate::sim::types::NodeKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which cloud vendor's product names and specs are in effect, or the
/// vendor-neutral default. `#[serde(rename_all = "lowercase")]` matches the
/// TS string union (`'generic' | 'aws' | 'gcp' | 'azure'`) exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VendorId {
    Generic,
    Aws,
    Gcp,
    Azure,
}

/// The unit a size's `memory` figure is published in. AWS and Azure publish
/// GiB, GCP publishes GB; kept as the vendor's own spelling rather than
/// converted, because converting is where a quoted figure quietly becomes
/// an approximated one (see `gcp.ts`'s header comment on units).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryUnit {
    #[serde(rename = "GiB")]
    GiB,
    #[serde(rename = "GB")]
    Gb,
}

/// A published hardware size. Every field here is quotable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorSize {
    /// Exactly as the vendor writes it, e.g. `db.r6g.large`.
    pub name: String,
    /// Absent where the vendor does not publish one in readable text. Azure
    /// Managed Redis states its per-SKU vCPU counts only inside an image,
    /// so those sizes omit this rather than carry a guessed number.
    /// `derive.rs` already treats a missing count as "nothing honest to
    /// say".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vcpu: Option<f64>,
    /// In the vendor's OWN unit. AWS and Azure publish GiB, GCP publishes
    /// GB.
    pub memory: f64,
    pub memory_unit: MemoryUnit,
    /// Network as PUBLISHED, kept as text.
    ///
    /// AWS says "Up to 10 Gigabit" for burstable classes and Azure
    /// publishes an "expected" figure. Turning either into a number would
    /// quietly convert a ceiling or an estimate into a promise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iops: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<f64>,
    /// On-demand, in the region named by the vendor file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_per_hour: Option<f64>,
    /// ISO date the price was read. Prices change; a stale one should show
    /// it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priced_on: Option<String>,
    /// Where every number above came from.
    pub source: String,
}

/// What one component kind is called at this vendor, and how it can be
/// sized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorMapping {
    /// The product name a student would see in the console.
    pub product: String,
    /// Why this product and not the vendor's other one. Present only where
    /// the choice is genuinely arguable, so its presence means something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Sizes worth offering. Absent where the vendor does not expose
    /// sizing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sizes: Option<Vec<VendorSize>>,
}

/// A vendor's product mappings, keyed by `NodeKind`. Mirrors the TS
/// `Partial<Record<NodeKind, VendorMapping>>`: a kind with no entry means
/// the vendor sells nothing that maps to it, which is the normal case, not
/// an error (see `lookup.ts`'s header comment).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vendor {
    pub id: VendorId,
    /// How the vendor writes its own name.
    pub label: String,
    /// The region every price in this file is quoted in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub kinds: HashMap<NodeKind, VendorMapping>,
}
