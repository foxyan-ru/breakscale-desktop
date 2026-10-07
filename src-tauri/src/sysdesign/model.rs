//! The system-design document model -- MIGRATION_PLAN.md #8.
//!
//! A `SystemDesignDoc` is not a replacement for the topology; it is an
//! architecture write-up anchored to one. `topology` is kept alongside the
//! rest of the document (rather than looked up separately) so an exported
//! file is self-contained, and so `validate` and the exporters can resolve a
//! `ComponentNote.node_id` or `DataFlowNote.edge_id` back to the node/edge it
//! describes without a second round trip.
//!
//! This is new code, not a port, so there is no TS source to diff against --
//! see `src-tauri/src/sysdesign/derive.rs` and `validate.rs` for how
//! it is populated and checked. Every field still gets a doc comment, in
//! keeping with the rest of this codebase.

use serde::{Deserialize, Serialize};

use crate::sim::types::Topology;
use crate::vendors::types::VendorId;

/// Bumped only when the shape of `SystemDesignDoc` changes in a way an older
/// reader could not understand -- mirrors `persistence::design_file`'s
/// `DESIGN_FILE_VERSION` gate (MIGRATION_PLAN.md #6).
pub const SCHEMA_VERSION: u32 = 1;

/// A full architecture write-up for one topology: high-level narrative,
/// low-level contracts, and the topology it was derived from and stays
/// anchored to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemDesignDoc {
    /// Unique id for this design document, independent of any saved-design id.
    pub id: String,
    /// What the reader called this design. Must be non-empty after trimming
    /// -- see `validate::validate`.
    pub name: String,
    /// Shape version of this document; readers should refuse anything newer
    /// than `SCHEMA_VERSION`, matching the design-file convention.
    pub schema_version: u32,
    /// ISO-8601 UTC timestamp of when this document was first created.
    pub created_at: String,
    /// ISO-8601 UTC timestamp of the most recent edit.
    pub updated_at: String,
    /// The topology this design was derived from. Every node/edge id
    /// referenced elsewhere in this document (component notes, data-flow
    /// notes, deployment resources) is an id from here.
    pub topology: Topology,
    /// The high-level narrative: what each component and connection is,
    /// what depends on the outside world, and the non-functional story.
    pub architecture: HighLevelArchitecture,
    /// The low-level contracts: APIs, entities, sequence flows, state
    /// machines and how the whole thing deploys.
    pub low_level: LowLevelDesign,
}

/// The high-level architecture narrative for a topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HighLevelArchitecture {
    /// A short prose summary of the system, shown first in every export.
    pub summary: String,
    /// One note per topology node, naming its technology and why it was chosen.
    pub components: Vec<ComponentNote>,
    /// One note per topology edge, describing what actually crosses that wire.
    pub data_flows: Vec<DataFlowNote>,
    /// Third-party APIs or SaaS the system relies on that have no node of
    /// their own in the topology.
    pub external_dependencies: Vec<ExternalDependency>,
    /// Free-text narrative of the system's non-functional properties. The
    /// numeric cost estimate is the existing vendor/cost model, not
    /// reinvented here -- this is the story behind the number, not the
    /// number itself.
    pub quality_attributes: QualityAttributes,
}

/// What one topology node is built from, and why.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentNote {
    /// The `SimNode.id` this note is about.
    pub node_id: String,
    /// The concrete technology behind this node, e.g. "Postgres 16" or
    /// "Redis Cluster". Left blank by `derive::from_topology` -- inventing a
    /// plausible-looking technology name would be exactly the kind of
    /// fabricated fact AGENTS.md forbids for measured numbers, and the same
    /// standard applies to invented text.
    pub technology: String,
    /// Why this technology was chosen over the alternatives.
    pub rationale: String,
    /// Who owns this component, when the design tracks that. `None` means
    /// nobody has said yet, not "nobody".
    pub owner: Option<String>,
}

/// What actually crosses one topology edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFlowNote {
    /// The `SimEdge.id` this note is about.
    pub edge_id: String,
    /// The wire protocol, e.g. "HTTPS/JSON" or "gRPC".
    pub protocol: String,
    /// The sensitivity of what travels this edge, e.g. "PII", "public".
    pub data_classification: String,
    /// A sentence describing what is actually sent.
    pub description: String,
}

/// A third-party service the system depends on that has no node of its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDependency {
    /// Id for this dependency, unique within the document.
    pub id: String,
    /// The service's name, e.g. "Stripe" or "Twilio".
    pub name: String,
    /// What it is used for.
    pub description: String,
    /// Who owns the relationship with this dependency, when tracked.
    pub owner: Option<String>,
}

/// Free-text narrative of a design's non-functional properties. Every field
/// is prose written by the reader, not a derived metric -- the numbers live
/// in the vendor/cost model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityAttributes {
    /// How the system grows to handle more load.
    pub scalability: String,
    /// How the system tolerates failure of its own parts.
    pub reliability: String,
    /// How the system protects data and access to it.
    pub security: String,
    /// How an operator would know the system is unhealthy.
    pub observability: String,
    /// The cost story in words -- trade-offs, not the number itself.
    pub cost: String,
}

/// The low-level contracts a design commits to: APIs, data models,
/// interaction sequences, state machines and deployment shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LowLevelDesign {
    /// The HTTP/RPC surface this system exposes.
    pub apis: Vec<ApiContract>,
    /// The data model: entities and how they relate.
    pub entities: Vec<EntityModel>,
    /// Step-by-step interaction flows, one per traced scenario.
    pub sequences: Vec<SequenceFlow>,
    /// Any component whose behaviour is best described as states and
    /// transitions between them.
    pub state_machines: Vec<StateMachine>,
    /// Where and how this design deploys.
    pub deployment: DeploymentConfig,
}

/// One HTTP/RPC endpoint contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiContract {
    /// Id for this contract, unique within the document.
    pub id: String,
    /// HTTP method, e.g. "GET", "POST". Free text so an RPC method name
    /// fits too.
    pub method: String,
    /// The route or RPC name, e.g. "/v1/orders/{id}".
    pub path: String,
    /// What this endpoint does.
    pub description: String,
    /// The request body/params, as free-form schema text (JSON Schema,
    /// a TypeScript type, or plain prose -- whatever the author finds
    /// clearest). Not parsed or validated by this app.
    pub request_schema: String,
    /// The response body, same free-form convention as `request_schema`.
    pub response_schema: String,
    /// Error responses this endpoint can return.
    pub errors: Vec<ApiError>,
}

/// One error response an API contract can return.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    /// HTTP status code, e.g. 404.
    pub status_code: u16,
    /// A machine-readable error code, e.g. "ORDER_NOT_FOUND".
    pub code: String,
    /// A sentence describing when this error happens.
    pub description: String,
}

/// One entity in the data model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityModel {
    /// Id for this entity, unique within the document. `EntityRelationship`
    /// entries may reference either this or `name` -- see `validate::validate`.
    pub id: String,
    /// The entity's name, e.g. "Order".
    pub name: String,
    /// What this entity represents.
    pub description: String,
    /// The entity's own fields.
    pub fields: Vec<EntityField>,
    /// How this entity relates to others in the same design.
    pub relationships: Vec<EntityRelationship>,
}

/// One field on an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityField {
    /// The field's name.
    pub name: String,
    /// The field's type, as free text, e.g. "UUID", "string(120)".
    pub field_type: String,
    /// Whether the field must always be present.
    pub required: bool,
    /// What the field holds and why it exists.
    pub description: String,
}

/// One relationship from an entity to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRelationship {
    /// The other entity's `EntityModel.id` or `EntityModel.name` -- either
    /// is accepted, see `validate::validate`.
    pub to_entity: String,
    /// The relationship's cardinality/shape, e.g. "one-to-many", free text.
    pub kind: String,
    /// A sentence describing the relationship.
    pub description: String,
}

/// One request-path scenario, told step by step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceFlow {
    /// Id for this flow, unique within the document.
    pub id: String,
    /// A short name for the scenario, e.g. "Checkout flow".
    pub name: String,
    /// The steps of the scenario, in order.
    pub steps: Vec<SequenceStep>,
}

/// One hop in a sequence flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceStep {
    /// The id of the node the step starts from (a topology node id, by
    /// convention, though this field is free-form once a flow is edited).
    pub from: String,
    /// The id of the node the step ends at.
    pub to: String,
    /// What kind of interaction this is, e.g. "request", "publish".
    pub action: String,
    /// Any extra detail worth recording about this hop.
    pub note: String,
}

/// One component's behaviour described as states and transitions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachine {
    /// Id for this machine, unique within the document.
    pub id: String,
    /// A short name, e.g. "Order lifecycle".
    pub name: String,
    /// Every state the machine can be in.
    pub states: Vec<String>,
    /// Every transition the machine can make.
    pub transitions: Vec<StateTransition>,
}

/// One transition in a state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateTransition {
    /// The state the transition leaves.
    pub from: String,
    /// The state the transition enters.
    pub to: String,
    /// What causes the transition, e.g. "payment confirmed".
    pub trigger: String,
    /// Any extra detail worth recording about this transition.
    pub description: String,
}

/// Where and how a design deploys.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentConfig {
    /// The environment this configuration targets, e.g. "development",
    /// "production".
    pub environment: String,
    /// The cloud vendor this design targets, if one has been chosen. `None`
    /// means undecided, not "generic" -- Terraform export in particular
    /// treats the two the same way (it cannot generate a provider block for
    /// either), but they are kept distinct here because "undecided" and
    /// "deliberately vendor-neutral" are different facts about a design.
    pub vendor: Option<VendorId>,
    /// The deployment region, when one has been chosen.
    pub region: Option<String>,
    /// Per-node deployment sizing, one entry per node that needs
    /// provisioned infrastructure.
    pub resources: Vec<DeploymentResource>,
    /// Free-text notes about the deployment that don't fit a structured field.
    pub notes: String,
}

/// Deployment sizing for one topology node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentResource {
    /// The `SimNode.id` this resource is for.
    pub node_id: String,
    /// The size/SKU chosen for this node, e.g. "db.r6g.large", when one has
    /// been chosen.
    pub size_name: Option<String>,
    /// Free-text notes about this specific resource.
    pub notes: String,
}

/// The formats a `SystemDesignDoc` can be exported to -- see `export::export`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Yaml,
    Markdown,
    Terraform,
}

/// One problem found by `validate::validate`, naming the field it is about
/// and a sentence the reader can act on. An empty `Vec<ValidationIssue>` is
/// the "everything's fine" result -- there is no separate "ok" variant, on
/// both the Rust and the TypeScript side of this contract.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    /// A dotted/bracketed path to the offending field, e.g.
    /// `"architecture.components[2].nodeId"`, so a frontend can highlight it.
    pub field: String,
    /// A sentence describing what is wrong and, where possible, what to do
    /// about it.
    pub message: String,
}
