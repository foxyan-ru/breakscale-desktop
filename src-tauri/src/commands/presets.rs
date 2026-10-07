//! `presets_list`/`preset_load` commands, invented by the frontend's
//! Examples port -- see `desktop/src/lib/components/shell/Examples.svelte`'s
//! integration notes.

use crate::error::{AppError, AppResult};
use crate::sim::presets::{self, Preset, PresetSummary};

#[tauri::command]
pub fn presets_list() -> AppResult<Vec<PresetSummary>> {
    Ok(presets::all_presets())
}

#[tauri::command]
pub fn preset_load(id: String) -> AppResult<Preset> {
    presets::preset_by_id(&id)
        .ok_or_else(|| AppError::NotFound(format!("\"{id}\" is not one of the built-in examples.")))
}
