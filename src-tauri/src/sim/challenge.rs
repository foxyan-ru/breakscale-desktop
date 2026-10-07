//! Port of `src/sim/challenge.ts`.
//!
//! A challenge is deliberately not a new kind of thing: a preset plus a
//! goal the engine can check against a live `SimSnapshot`, because the
//! expensive half of "add challenges" is authoring scenarios and the
//! presets already exist (`sim::presets`).

use crate::sim::types::{SimSnapshot, Topology};
use serde::{Deserialize, Serialize};

/// A brief with a pass condition the engine can check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    pub id: String,
    /// Shown as the heading. A short imperative: "Survive the spike".
    pub name: String,
    /// The brief, in the words a client would use rather than in metrics.
    pub brief: String,
    pub preset_id: String,
    /// Offered load the design is judged at, in requests per second.
    pub load_rps: f64,
    pub goals: Vec<Goal>,
    /// Nudges, in order, from the smallest one that helps to the one that
    /// names the component. None of them state the fix.
    pub hints: Vec<String>,
    /// Why the fix worked, shown only once the goals are met.
    pub lesson: String,
}

/// One checkable condition. `max` reads as "no more than": latency, errors
/// and cost are all things a brief is trying to keep down.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
    pub metric: GoalMetric,
    pub max: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalMetric {
    P99,
    ErrorRate,
    P95,
}

/// How one goal came out, with the number that decided it.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalResult {
    pub goal: Goal,
    pub actual: f64,
    pub met: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeResult {
    pub passed: bool,
    pub goals: Vec<GoalResult>,
}

/// Read the metric a goal names off a snapshot. `errorRate` is stored as a
/// fraction and stated in the brief as a percentage, because "under 1%" is
/// how the requirement is written down in real life and "under 0.01" is
/// not.
fn actual_for(metric: GoalMetric, snapshot: &SimSnapshot) -> f64 {
    let s = &snapshot.system;
    match metric {
        GoalMetric::P99 => s.p99,
        GoalMetric::P95 => s.p95,
        GoalMetric::ErrorRate => s.error_rate * 100.0,
    }
}

/// Judge a run. A dead system (no goodput) is a FAILURE, never a pass, even
/// if the arithmetic alone would call it a win: with nothing completing
/// there are no latencies to take a percentile of, so p99 reads 0 and would
/// satisfy any "under 200ms" goal it was given.
pub fn evaluate(challenge: &Challenge, snapshot: &SimSnapshot) -> ChallengeResult {
    let dead = snapshot.system.goodput_rps <= 0.0;
    let goals: Vec<GoalResult> = challenge
        .goals
        .iter()
        .map(|goal| {
            let actual = actual_for(goal.metric, snapshot);
            GoalResult {
                goal: *goal,
                actual,
                met: !dead && actual <= goal.max,
            }
        })
        .collect();
    let passed = !dead && goals.iter().all(|g| g.met);
    ChallengeResult { passed, goals }
}

/// Fields a brief holds still: `serviceMs` is how long the work takes, and
/// `serviceCv` is how uneven it is -- both properties of the work, not a
/// decision the reader gets to make. Everything else (capacity, instances,
/// timeouts, ...) stays editable, including choices that look like
/// cheating and are not: raising capacity to something absurd is a
/// legitimate answer that costs absurd money, which is the tradeoff a
/// budget makes visible later.
pub const FIXED_DURING_CHALLENGE: &[&str] = &["serviceMs", "serviceCv"];

pub fn is_fixed_during_challenge(field: &str) -> bool {
    FIXED_DURING_CHALLENGE.contains(&field)
}

/// The load a challenge is judged at, applied evenly to every traffic
/// source.
pub fn apply_load(mut topology: Topology, load_rps: f64) -> Topology {
    let client_count = topology
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, crate::sim::types::NodeKind::Client))
        .count();
    if client_count == 0 {
        return topology;
    }
    let each = (load_rps / client_count as f64).round().max(1.0);
    for node in topology.nodes.iter_mut() {
        if matches!(node.kind, crate::sim::types::NodeKind::Client) {
            node.config.rps = each;
        }
    }
    topology
}
