//! Shared simulation DATA that the remaining Tauri commands still serve:
//! the structural/wire types (`types`), the built-in examples (`presets`),
//! the challenge briefs and their goal checks (`challenge`, `challenges`)
//! and the glossary (`glossary`).
//!
//! There is no engine here any more. The discrete-event engine now runs
//! in-process in the webview as upstream's own TypeScript (`src/sim/*.ts`,
//! copied byte-for-byte into `desktop/src/lib/sim/`), so the Rust port of
//! `engine.ts`, `behaviour*.ts`, `engine-types.ts`, `heap.ts` and
//! `random.ts` -- and the `sim_*` commands and `sim://*` events that drove
//! it over IPC -- were removed. What stays is what `presets_list`,
//! `preset_load`, `challenges_list`, `challenge_start`, `glossary_list`,
//! the design-file/saved-design persistence, `sysdesign` and `vendors` need
//! to (de)serialize a `Topology` and its content.
//!
//! Still mirrors upstream `src/sim`'s rule (this repository's AGENTS.md):
//! pure data and logic, no file I/O, no Tauri -- the callers in `commands`
//! and `persistence` own every side effect.

pub mod challenge;
pub mod challenges;
pub mod glossary;
pub mod presets;
pub mod types;
