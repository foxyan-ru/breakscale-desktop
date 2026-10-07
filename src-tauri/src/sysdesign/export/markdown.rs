//! Markdown export: a hand-written generator, not a templating crate, so
//! the output has exactly the sections MIGRATION_PLAN.md #8 asks for, in
//! order, and every section that has no data yet says so in words instead
//! of rendering as a heading with nothing under it -- a blank-looking
//! section reads as broken, an italic placeholder reads as "nothing here
//! yet, on purpose".
//!
//! Deviation from the plan's literal example worth flagging: the plan's
//! sketch of the sequence-step line uses an em dash ("{action} — {note}").
//! AGENTS.md's Copy section is explicit that this codebase does not use em
//! dashes, so steps are rendered as two sentences instead. See
//! `sequence_flows` below.

use crate::sim::types::{NodeKind, Topology};
use crate::sysdesign::model::SystemDesignDoc;
use crate::vendors::types::VendorId;

pub fn export(doc: &SystemDesignDoc) -> String {
    let mut out = String::new();
    title(&mut out, doc);
    overview(&mut out, doc);
    components(&mut out, doc);
    external_dependencies(&mut out, doc);
    data_flows(&mut out, doc);
    quality_attributes(&mut out, doc);
    api_contracts(&mut out, doc);
    entity_models(&mut out, doc);
    sequence_flows(&mut out, doc);
    state_machines(&mut out, doc);
    deployment(&mut out, doc);
    out
}

fn title(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str(&format!("# {}\n\n", doc.name));
    out.push_str(&format!(
        "_Generated {} from this design's current topology and notes._\n\n",
        crate::util::iso8601::now()
    ));
}

fn overview(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Overview\n\n");
    out.push_str(&placeholder(&doc.architecture.summary));
    out.push_str("\n\n");
}

fn components(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Components\n\n");
    if doc.architecture.components.is_empty() {
        out.push_str("_No components documented yet._\n\n");
        return;
    }
    out.push_str("| Node | Technology | Rationale | Owner |\n");
    out.push_str("|---|---|---|---|\n");
    for c in &doc.architecture.components {
        let owner = c
            .owner
            .as_deref()
            .map(placeholder)
            .unwrap_or_else(|| "_Not yet documented._".to_string());
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            esc(&node_ref(&doc.topology, &c.node_id)),
            placeholder(&c.technology),
            placeholder(&c.rationale),
            owner,
        ));
    }
    out.push('\n');
}

fn external_dependencies(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## External Dependencies\n\n");
    if doc.architecture.external_dependencies.is_empty() {
        out.push_str("_No external dependencies documented yet._\n\n");
        return;
    }
    out.push_str("| Name | Description | Owner |\n");
    out.push_str("|---|---|---|\n");
    for d in &doc.architecture.external_dependencies {
        let owner = d
            .owner
            .as_deref()
            .map(placeholder)
            .unwrap_or_else(|| "_Not yet documented._".to_string());
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            esc(&d.name),
            placeholder(&d.description),
            owner,
        ));
    }
    out.push('\n');
}

fn data_flows(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Data Flows\n\n");
    if doc.architecture.data_flows.is_empty() {
        out.push_str("_No data flows documented yet._\n\n");
        return;
    }
    out.push_str("| From -> To | Protocol | Classification | Description |\n");
    out.push_str("|---|---|---|---|\n");
    for f in &doc.architecture.data_flows {
        let endpoints = match doc.topology.edges.iter().find(|e| e.id == f.edge_id) {
            Some(edge) => format!(
                "{} -> {}",
                node_ref(&doc.topology, &edge.from),
                node_ref(&doc.topology, &edge.to)
            ),
            None => format!("{} (connection not on canvas)", f.edge_id),
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            esc(&endpoints),
            placeholder(&f.protocol),
            placeholder(&f.data_classification),
            placeholder(&f.description),
        ));
    }
    out.push('\n');
}

fn quality_attributes(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Quality Attributes\n\n");
    let qa = &doc.architecture.quality_attributes;
    for (heading, text) in [
        ("Scalability", &qa.scalability),
        ("Reliability", &qa.reliability),
        ("Security", &qa.security),
        ("Observability", &qa.observability),
        ("Cost", &qa.cost),
    ] {
        out.push_str(&format!("### {heading}\n\n"));
        out.push_str(&placeholder(text));
        out.push_str("\n\n");
    }
}

fn api_contracts(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## API Contracts\n\n");
    if doc.low_level.apis.is_empty() {
        out.push_str("_No API contracts documented yet._\n\n");
        return;
    }
    for api in &doc.low_level.apis {
        out.push_str(&format!(
            "### {} {}\n\n",
            non_empty(&api.method.trim().to_uppercase(), "?"),
            non_empty(&api.path, "(no path set)")
        ));
        if !api.description.trim().is_empty() {
            out.push_str(&api.description);
            out.push_str("\n\n");
        }
        out.push_str("Request schema:\n\n```\n");
        out.push_str(raw_or(&api.request_schema, "(not documented)"));
        out.push_str("\n```\n\n");
        out.push_str("Response schema:\n\n```\n");
        out.push_str(raw_or(&api.response_schema, "(not documented)"));
        out.push_str("\n```\n\n");
        if api.errors.is_empty() {
            out.push_str("_No error responses documented yet._\n\n");
        } else {
            out.push_str("| Status | Code | Description |\n|---|---|---|\n");
            for e in &api.errors {
                out.push_str(&format!(
                    "| {} | {} | {} |\n",
                    e.status_code,
                    esc(&e.code),
                    placeholder(&e.description)
                ));
            }
            out.push('\n');
        }
    }
}

fn entity_models(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Entity Models\n\n");
    if doc.low_level.entities.is_empty() {
        out.push_str("_No entity models documented yet._\n\n");
        return;
    }
    for ent in &doc.low_level.entities {
        out.push_str(&format!("### {}\n\n", non_empty(&ent.name, "(unnamed entity)")));
        if !ent.description.trim().is_empty() {
            out.push_str(&ent.description);
            out.push_str("\n\n");
        }
        if ent.fields.is_empty() {
            out.push_str("_No fields documented yet._\n\n");
        } else {
            out.push_str("| Field | Type | Required | Description |\n|---|---|---|---|\n");
            for f in &ent.fields {
                out.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    esc(&f.name),
                    esc(&f.field_type),
                    if f.required { "yes" } else { "no" },
                    placeholder(&f.description),
                ));
            }
            out.push('\n');
        }
        if ent.relationships.is_empty() {
            out.push_str("_No relationships documented yet._\n\n");
        } else {
            out.push_str("Relationships:\n\n");
            for r in &ent.relationships {
                out.push_str(&format!(
                    "- **{}** -> {}: {}\n",
                    placeholder(&r.kind),
                    esc(&r.to_entity),
                    placeholder(&r.description)
                ));
            }
            out.push('\n');
        }
    }
}

fn sequence_flows(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Sequence Flows\n\n");
    if doc.low_level.sequences.is_empty() {
        out.push_str("_No sequence flows documented yet._\n\n");
        return;
    }
    for seq in &doc.low_level.sequences {
        out.push_str(&format!("### {}\n\n", non_empty(&seq.name, "(unnamed flow)")));
        if seq.steps.is_empty() {
            out.push_str("_This flow has no steps yet._\n\n");
            continue;
        }
        for (i, step) in seq.steps.iter().enumerate() {
            // "{action}. {note}" rather than the plan's em-dash sketch --
            // see the module doc comment.
            out.push_str(&format!(
                "{}. **{}** -> **{}**: {}. {}\n",
                i + 1,
                esc(&step.from),
                esc(&step.to),
                non_empty(&step.action, "request"),
                non_empty(&step.note, "No note yet.")
            ));
        }
        out.push('\n');
    }
}

fn state_machines(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## State Machines\n\n");
    if doc.low_level.state_machines.is_empty() {
        out.push_str("_No state machines documented yet._\n\n");
        return;
    }
    for sm in &doc.low_level.state_machines {
        out.push_str(&format!("### {}\n\n", non_empty(&sm.name, "(unnamed machine)")));
        if sm.states.is_empty() {
            out.push_str("_No states documented yet._\n\n");
        } else {
            for s in &sm.states {
                out.push_str(&format!("- {}\n", esc(s)));
            }
            out.push('\n');
        }
        if sm.transitions.is_empty() {
            out.push_str("_No transitions documented yet._\n\n");
        } else {
            out.push_str("| From | To | Trigger | Description |\n|---|---|---|---|\n");
            for t in &sm.transitions {
                out.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    esc(&t.from),
                    esc(&t.to),
                    placeholder(&t.trigger),
                    placeholder(&t.description)
                ));
            }
            out.push('\n');
        }
    }
}

fn deployment(out: &mut String, doc: &SystemDesignDoc) {
    out.push_str("## Deployment\n\n");
    let d = &doc.low_level.deployment;
    out.push_str(&format!(
        "- **Environment**: {}\n",
        non_empty(&d.environment, "_not set_")
    ));
    out.push_str(&format!(
        "- **Vendor**: {}\n",
        d.vendor.map(vendor_label).unwrap_or("_not chosen_")
    ));
    let region = d
        .region
        .as_deref()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or("_not set_");
    out.push_str(&format!("- **Region**: {region}\n\n"));
    if !d.notes.trim().is_empty() {
        out.push_str(&d.notes);
        out.push_str("\n\n");
    }
    if d.resources.is_empty() {
        out.push_str("_No deployment resources documented yet._\n\n");
        return;
    }
    out.push_str("| Node | Size | Notes |\n|---|---|---|\n");
    for r in &d.resources {
        let size = r
            .size_name
            .as_deref()
            .map(esc)
            .unwrap_or_else(|| "_not set_".to_string());
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            esc(&node_ref(&doc.topology, &r.node_id)),
            size,
            placeholder(&r.notes),
        ));
    }
    out.push('\n');
}

fn vendor_label(vendor: VendorId) -> &'static str {
    match vendor {
        VendorId::Generic => "Generic",
        VendorId::Aws => "AWS",
        VendorId::Gcp => "GCP",
        VendorId::Azure => "Azure",
    }
}

fn node_ref(topology: &Topology, node_id: &str) -> String {
    match topology.nodes.iter().find(|n| n.id == node_id) {
        Some(n) => format!("{} ({})", n.label, kind_str(&n.kind)),
        None => format!("{node_id} (not on canvas)"),
    }
}

fn kind_str(kind: &NodeKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Escapes a value for a Markdown table cell or inline list text: a raw `|`
/// would be read as a column separator, and a raw newline would break a
/// table row or renumber a list.
fn esc(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ").replace('\r', "")
}

/// A field the reader hasn't filled in yet renders as an italic placeholder,
/// never as a blank table cell -- a blank cell looks like a rendering bug,
/// an italic note reads as "nothing here yet, on purpose".
fn placeholder(s: &str) -> String {
    if s.trim().is_empty() {
        "_Not yet documented._".to_string()
    } else {
        esc(s)
    }
}

/// Like `placeholder`, but with a caller-supplied fallback instead of the
/// standard "not yet documented" wording, for spots where a more specific
/// default reads better (a step's action defaulting to "request", say).
fn non_empty(s: &str, fallback: &str) -> String {
    if s.trim().is_empty() {
        fallback.to_string()
    } else {
        esc(s)
    }
}

/// Like `non_empty`, but for text placed inside a fenced code block, where
/// escaping pipes/newlines would corrupt the very schema text it is meant
/// to preserve verbatim.
fn raw_or<'a>(s: &'a str, fallback: &'a str) -> &'a str {
    if s.trim().is_empty() {
        fallback
    } else {
        s
    }
}
