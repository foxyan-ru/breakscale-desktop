//! Seeds a `SystemDesignDoc` from an existing `Topology` -- MIGRATION_PLAN.md
//! #8. The feature starts from what the user already built on the canvas
//! rather than a blank form; everything this produces is then editable.
//!
//! Nothing here invents a fact. A component's technology, a data flow's
//! protocol, a quality attribute's narrative -- all of that is left blank
//! for the reader to fill in, exactly as AGENTS.md's "never a
//! plausible-looking number" rule demands for invented text, not only for
//! invented numbers. The only things this module writes are facts already
//! true of the topology: which nodes and edges exist, and how a client's
//! traffic actually routes through them.

use std::collections::VecDeque;

use crate::sim::types::{NodeKind, Topology};
use crate::util::id::new_id;
use crate::util::iso8601::now;

use super::model::{
    ComponentNote, DataFlowNote, DeploymentConfig, HighLevelArchitecture, LowLevelDesign,
    QualityAttributes, SequenceFlow, SequenceStep, SystemDesignDoc,
};

/// Hop-depth guard mirroring `sim::engine`'s `MAX_HOP_DEPTH`: a topology can
/// contain a routing cycle (a mis-wired loop back to an earlier node), and a
/// pure graph walk has no other reason to terminate. The engine bounds a
/// live request's hop count for the same reason -- see `SimEdge.control`'s
/// doc comment in `sim::types` for why a control edge is excluded below
/// rather than walked.
const MAX_HOP_DEPTH: u32 = 32;

/// Seed a full `SystemDesignDoc` from `topology`.
///
/// One `ComponentNote` per node and one `DataFlowNote` per edge, both left
/// blank apart from the id they reference; a factual summary sentence built
/// from real counts; and one best-effort `SequenceFlow` per client node,
/// built by walking the routing graph breadth-first from that client.
pub fn from_topology(topology: &Topology, design_id: &str, name: &str) -> SystemDesignDoc {
    let components: Vec<ComponentNote> = topology
        .nodes
        .iter()
        .map(|node| ComponentNote {
            node_id: node.id.clone(),
            technology: String::new(),
            rationale: String::new(),
            owner: None,
        })
        .collect();

    let data_flows: Vec<DataFlowNote> = topology
        .edges
        .iter()
        .map(|edge| DataFlowNote {
            edge_id: edge.id.clone(),
            protocol: String::new(),
            data_classification: String::new(),
            description: String::new(),
        })
        .collect();

    let summary = format!(
        "{} component(s), {} connection(s).",
        topology.nodes.len(),
        topology.edges.len()
    );

    let sequences: Vec<SequenceFlow> = topology
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Client)
        .map(|client| SequenceFlow {
            id: new_id(),
            name: format!("{} flow", client.label),
            steps: walk_request_path(topology, &client.id),
        })
        .collect();

    let timestamp = now();

    SystemDesignDoc {
        id: design_id.to_string(),
        name: name.to_string(),
        schema_version: super::model::SCHEMA_VERSION,
        created_at: timestamp.clone(),
        updated_at: timestamp,
        topology: topology.clone(),
        architecture: HighLevelArchitecture {
            summary,
            components,
            data_flows,
            external_dependencies: vec![],
            quality_attributes: QualityAttributes {
                scalability: String::new(),
                reliability: String::new(),
                security: String::new(),
                observability: String::new(),
                cost: String::new(),
            },
        },
        low_level: LowLevelDesign {
            apis: vec![],
            entities: vec![],
            sequences,
            state_machines: vec![],
            deployment: DeploymentConfig {
                environment: "development".to_string(),
                vendor: None,
                region: None,
                resources: vec![],
                notes: String::new(),
            },
        },
    }
}

/// Breadth-first walk of the routing graph starting at `start_node_id`,
/// following `topology.edges` in the order they appear and skipping control
/// edges (`edge.control == Some(true)`), which are supervisory relationships
/// and never carry a request -- see `SimEdge.control`'s doc comment in
/// `sim::types`. Each branch stops at a node with no further outgoing
/// non-control edges, or once it has taken `MAX_HOP_DEPTH` hops.
///
/// No visited-node set is kept, deliberately: the engine's own hop-depth
/// guard does not track visited nodes either (a request may legitimately
/// revisit a node along a different path), so mirroring that means a
/// topology with a routing cycle produces a bounded, not infinite, walk
/// here too.
fn walk_request_path(topology: &Topology, start_node_id: &str) -> Vec<SequenceStep> {
    let mut steps = Vec::new();
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((start_node_id.to_string(), 0));

    while let Some((node_id, depth)) = queue.pop_front() {
        if depth >= MAX_HOP_DEPTH {
            continue;
        }
        for edge in &topology.edges {
            if edge.from != node_id {
                continue;
            }
            if edge.control == Some(true) {
                continue;
            }
            steps.push(SequenceStep {
                from: edge.from.clone(),
                to: edge.to.clone(),
                action: "request".to_string(),
                note: String::new(),
            });
            queue.push_back((edge.to.clone(), depth + 1));
        }
    }

    steps
}
