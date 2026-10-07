//! Vendor/cost model: faithful port of `src/content/vendors/*.ts`.
//!
//! Mirrors MIGRATION_PLAN.md #4's row for `content/vendors/*.ts`: specs as
//! JSON (`data/vendors/*.json`, loaded by `data.rs`), `cost.rs`/`derive.rs`
//! port the arithmetic (`fleetSize`, `costDesign`, `deriveFromSize`)
//! faithfully, including the "derived, not measured" caveats as doc
//! comments -- AGENTS.md is explicit that this distinction must survive.

pub mod cost;
pub mod data;
pub mod derive;
pub mod types;
