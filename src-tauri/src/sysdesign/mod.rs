//! System design generation -- MIGRATION_PLAN.md #8.
//!
//! The topology already is a lightweight architecture model: nodes are
//! components and stores, edges are data flows. This module extends that
//! model rather than replacing it, turning a topology into an editable
//! architecture write-up (`model::SystemDesignDoc`), seeded from what the
//! user already built (`derive::from_topology`), checked for stale
//! references (`validate::validate`), and exportable as JSON, YAML,
//! Markdown or a Terraform starter bundle (`export`).
//!
//! This is new functionality with no TypeScript source to port; every
//! design decision not dictated by MIGRATION_PLAN.md #8 is explained in the
//! doc comments of the module it lives in.

pub mod derive;
pub mod export;
pub mod model;
pub mod validate;
