//! Crate root: the Tauri app builder. Registers every command handler;
//! everything else lives in its own module -- see MIGRATION_PLAN.md #3 for
//! the full directory map. There is no managed state: the simulation engine
//! runs in-process in the webview (`desktop/src/lib/sim/`), so the Rust
//! engine, its tick thread and the `sim_*` commands are gone.

pub mod commands;
pub mod error;
pub mod persistence;
pub mod sim;
pub mod sysdesign;
pub mod util;
pub mod vendors;

// The Rust half of the shared IPC contract suite (`contract/ipc-golden.json`
// is the golden fixture; the frontend's vitest suite checks the TS side
// against the same bytes). WHY: it pins every command name, argument key
// set and payload key set of all 26 handlers, so a wire rename or a gained/
// dropped field fails `cargo test` here instead of surfacing as an opaque
// serde error (or a silently stale frontend) at runtime. Test-only, so it
// never links into the shipped app.
#[cfg(test)]
mod contract_tests;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // designs / design file
            commands::designs::designs_list,
            commands::designs::designs_save,
            commands::designs::designs_get,
            commands::designs::designs_delete,
            commands::designs::designs_rename,
            commands::designs::design_file_build,
            commands::designs::design_file_parse,
            commands::designs::design_file_write,
            commands::designs::design_file_read,
            // backup
            commands::backup::backup_write,
            commands::backup::backup_restore_from_path,
            // sysdesign
            commands::sysdesign::sysdesign_derive_high_level,
            commands::sysdesign::sysdesign_save,
            commands::sysdesign::sysdesign_load,
            commands::sysdesign::sysdesign_validate,
            commands::sysdesign::sysdesign_export,
            // vendors
            commands::vendors::vendors_get,
            // glossary
            commands::glossary::glossary_list,
            // presets / examples
            commands::presets::presets_list,
            commands::presets::preset_load,
            // challenges
            commands::challenges::challenges_list,
            commands::challenges::challenge_start,
            // settings / layout
            commands::settings::settings_load,
            commands::settings::settings_save,
            commands::settings::layout_load,
            commands::settings::layout_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Breakscale desktop application");
}
