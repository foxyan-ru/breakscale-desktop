//! Faithful port of `src/content/vendors/cost.ts`.
//!
//! What a design would cost.
//!
//! A system design argument is never only about latency. "Add a read
//! replica" and "buy a bigger box" both fix a queue, and which one is
//! right usually turns on money. Without a price the simulator can only
//! ever teach half the trade.
//!
//! WHAT THIS IS HONEST ABOUT.
//!
//! Every hourly rate comes from a vendor file and carries a source URL.
//! The arithmetic here is just rate times fleet size times hours, which is
//! genuinely how these bills work for compute.
//!
//! What it is NOT is a bill. Real invoices carry storage, egress, backup,
//! support, reserved-instance discounts and free tiers, none of which this
//! models. So the figure is labelled as compute only, everywhere it is
//! shown. An estimate presented as a bill would be the same dishonesty as
//! a derived number presented as a measurement.

use crate::sim::types::{NodeConfig, NodeKind, SimNode};
use crate::vendors::types::Vendor;
use serde::Serialize;

/// Hours in an average month.
///
/// 730, not 720. Vendors bill per hour and quote monthly figures on a
/// 730-hour month (365 days / 12), so using 30 days would put every number
/// here about 1.4 percent below the same number on the vendor's own
/// calculator, for no reason anyone could see.
pub const HOURS_PER_MONTH: f64 = 730.0;

/// How many of this thing are actually running.
///
/// The engine already tracks fleet size honestly and differently per kind,
/// so this reads what each kind really uses rather than assuming
/// `instances` everywhere: a shard's fleet is its partition count, a
/// replica set's is its replica count.
pub fn fleet_size(kind: NodeKind, config: &NodeConfig) -> f64 {
    // Each field is optional on the type because only some kinds read it,
    // so a missing one means "this kind does not have a fleet" and falls
    // back to a single box rather than to zero.
    fn count(v: Option<f64>) -> f64 {
        match v {
            Some(n) if n.is_finite() => n.floor().max(1.0),
            _ => 1.0,
        }
    }

    match kind {
        NodeKind::Shard => count(Some(config.shard_count)),
        // A replica set is the primary plus its replicas, which is what
        // you pay for: the primary is a box too.
        NodeKind::Replica => count(Some(config.replica_count)) + 1.0,
        _ => count(config.instances),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeCost {
    pub node_id: String,
    pub label: String,
    /// The size this price came from, so the reader can check it.
    pub size_name: String,
    pub fleet: f64,
    pub per_hour: f64,
    pub per_month: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignCost {
    pub nodes: Vec<NodeCost>,
    pub per_month: f64,
    /// Components with no priced size, which the total therefore excludes.
    pub unpriced: Vec<String>,
}

/// Cost a design, given the size chosen for each component.
///
/// `size_of` returns the size a node is running on, or `None` where the
/// student has not picked one. A component with no size is NOT guessed at:
/// it is listed as unpriced, because inventing a rate for it would make the
/// total look complete while being wrong by an unknown amount.
pub fn cost_design(
    nodes: &[SimNode],
    vendor: Option<&Vendor>,
    size_of: impl Fn(&SimNode) -> Option<crate::vendors::types::VendorSize>,
) -> DesignCost {
    let mut out: Vec<NodeCost> = Vec::new();
    let mut unpriced: Vec<String> = Vec::new();

    let Some(vendor) = vendor else {
        return DesignCost {
            nodes: out,
            per_month: 0.0,
            unpriced,
        };
    };

    for node in nodes {
        let size = size_of(node);
        let size = match size {
            Some(s) if s.price_per_hour.is_some() => s,
            _ => {
                // A client is not a thing you rent, so it is not
                // "unpriced" in the sense of missing data. Only components
                // the vendor actually sells are reported as a gap in the
                // total.
                if !matches!(node.kind, NodeKind::Client) && vendor.kinds.contains_key(&node.kind)
                {
                    unpriced.push(node.label.clone());
                }
                continue;
            }
        };
        let fleet = fleet_size(node.kind, &node.config);
        // `price_per_hour` was just confirmed `Some` above.
        let per_hour = size.price_per_hour.unwrap_or(0.0) * fleet;
        out.push(NodeCost {
            node_id: node.id.clone(),
            label: node.label.clone(),
            size_name: size.name.clone(),
            fleet,
            per_hour,
            per_month: per_hour * HOURS_PER_MONTH,
        });
    }

    out.sort_by(|a, b| b.per_month.partial_cmp(&a.per_month).unwrap());
    let per_month = out.iter().map(|n| n.per_month).sum();
    DesignCost {
        nodes: out,
        per_month,
        unpriced,
    }
}

/// Money, in the way a reader scans a bill rather than reads a number.
pub fn format_money(usd: f64) -> String {
    if usd >= 10_000.0 {
        return format!("${}", group_thousands(usd.round() as i64));
    }
    if usd >= 100.0 {
        return format!("${usd:.0}");
    }
    if usd >= 1.0 {
        return format!("${usd:.2}");
    }
    // Sub-dollar figures are real for a small instance, and rounding them
    // to $0 would tell a student the thing is free.
    format!("${usd:.3}")
}

/// `n.toLocaleString('en-US')` for a non-negative integer: digit groups of
/// three separated by commas, e.g. `12345` -> `"12,345"`.
fn group_thousands(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if n < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// The caveat, in one place, so it cannot drift between two surfaces.
pub const COST_NOTE: &str = "Compute only, at on-demand rates. A real bill also carries storage, network egress, backups and support, and discounts for commitment.";
