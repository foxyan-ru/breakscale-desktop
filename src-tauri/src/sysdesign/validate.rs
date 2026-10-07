//! Validates a `SystemDesignDoc` -- MIGRATION_PLAN.md #8.
//!
//! Every check here produces a `ValidationIssue { field, message }` a reader
//! can act on, in the same plain, specific register as `designFile.ts`'s
//! error strings ("That file is not valid JSON. It may have been edited or
//! only partly saved.") -- never a generic "invalid input". An empty
//! `Vec<ValidationIssue>` is the correct "everything's fine" result; there is
//! no separate boolean.

use std::collections::HashSet;

use super::model::{SystemDesignDoc, ValidationIssue};

fn issue(field: impl Into<String>, message: impl Into<String>) -> ValidationIssue {
    ValidationIssue {
        field: field.into(),
        message: message.into(),
    }
}

/// Check a design document for stale references and missing required
/// fields. Nothing here mutates `doc` or fixes anything; every problem is
/// reported so the caller can point the reader at it.
pub fn validate(doc: &SystemDesignDoc) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    if doc.name.trim().is_empty() {
        // Deliberately the exact phrase `savedDesigns.ts`'s `saveDesign` uses
        // for the same problem, for voice consistency across the app.
        issues.push(issue("name", "Give the design a name first."));
    }

    let node_ids: HashSet<&str> = doc.topology.nodes.iter().map(|n| n.id.as_str()).collect();
    let edge_ids: HashSet<&str> = doc.topology.edges.iter().map(|e| e.id.as_str()).collect();

    for (i, note) in doc.architecture.components.iter().enumerate() {
        if !node_ids.contains(note.node_id.as_str()) {
            issues.push(issue(
                format!("architecture.components[{i}].nodeId"),
                format!(
                    "This component note points at node \"{}\", which no longer exists on the canvas.",
                    note.node_id
                ),
            ));
        }
    }

    for (i, note) in doc.architecture.data_flows.iter().enumerate() {
        if !edge_ids.contains(note.edge_id.as_str()) {
            issues.push(issue(
                format!("architecture.dataFlows[{i}].edgeId"),
                format!(
                    "This data flow note points at connection \"{}\", which no longer exists on the canvas.",
                    note.edge_id
                ),
            ));
        }
    }

    let entity_ids: HashSet<&str> = doc.low_level.entities.iter().map(|e| e.id.as_str()).collect();
    let entity_names: HashSet<&str> =
        doc.low_level.entities.iter().map(|e| e.name.as_str()).collect();

    for entity in &doc.low_level.entities {
        for (i, rel) in entity.relationships.iter().enumerate() {
            let target = rel.to_entity.as_str();
            if !entity_ids.contains(target) && !entity_names.contains(target) {
                issues.push(issue(
                    format!("lowLevel.entities[{}].relationships[{i}].toEntity", entity.id),
                    format!("\"{target}\" references an entity that doesn't exist in this design."),
                ));
            }
        }
    }

    // Sequence steps are meant to be editable free-form once seeded from the
    // topology, so this deliberately does not check that `from`/`to` still
    // resolve to a real node -- only that they were filled in at all.
    for (si, seq) in doc.low_level.sequences.iter().enumerate() {
        for (ti, step) in seq.steps.iter().enumerate() {
            if step.from.trim().is_empty() || step.to.trim().is_empty() {
                issues.push(issue(
                    format!("lowLevel.sequences[{si}].steps[{ti}]"),
                    "This step needs a source/target.",
                ));
            }
        }
    }

    for (i, resource) in doc.low_level.deployment.resources.iter().enumerate() {
        if !node_ids.contains(resource.node_id.as_str()) {
            issues.push(issue(
                format!("lowLevel.deployment.resources[{i}].nodeId"),
                format!(
                    "This deployment resource points at node \"{}\", which no longer exists on the canvas.",
                    resource.node_id
                ),
            ));
        }
    }

    issues
}
