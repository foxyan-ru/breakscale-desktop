//! Crate root: the Tauri app builder. Registers every command handler and
//! the one piece of managed state (`state::SimulationState`); everything
//! else lives in its own module -- see MIGRATION_PLAN.md #3 for the full
//! directory map.

pub mod commands;
pub mod error;
pub mod persistence;
pub mod sim;
pub mod state;
pub mod sysdesign;
pub mod util;
pub mod vendors;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::SimulationState::default())
        .invoke_handler(tauri::generate_handler![
            // sim
            commands::sim::sim_new,
            commands::sim::sim_set_topology,
            commands::sim::sim_update_node_config,
            commands::sim::sim_inject_failure,
            commands::sim::sim_clear_failure,
            commands::sim::sim_reset,
            commands::sim::sim_set_running,
            commands::sim::sim_get_snapshot,
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
