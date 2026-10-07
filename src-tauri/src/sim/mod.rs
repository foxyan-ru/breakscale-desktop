//! Pure simulation domain: the discrete-event engine and the deterministic
//! primitives (`random`, `heap`) and shared structural types
//! (`types`, `engine_types`) it is built from.
//!
//! Mirrors `src/sim`'s own rule, stated in this repository's AGENTS.md:
//! "`src/sim` is a pure simulation engine with no React, no DOM and no I/O;
//! everything else is the interface around it." Nothing in this module (or
//! its submodules) may perform file I/O, call into Tauri, or otherwise
//! reach outside the simulation -- that boundary is what lets the engine
//! (ported separately, into `sim::engine`) replay byte-identically from a
//! seed and a topology, on both sides of the port.
//!
//! See MIGRATION_PLAN.md #4 ("Domain contract (Rust <-> TypeScript wire
//! format)") for the full module map this crate is being built against;
//! this file covers `types.ts`, `random.ts`, `heap.ts` and
//! `engine-types.ts` only. `engine.ts`, `behaviour*.ts`, `annotations.ts`,
//! `challenge*.ts` and `presets.ts` are ported separately, into sibling
//! modules under `sim::`.

pub mod behaviour;
pub mod challenge;
pub mod challenges;
pub mod engine;
pub mod engine_types;
pub mod glossary;
pub mod heap;
pub mod presets;
pub mod random;
pub mod types;
