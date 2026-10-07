//! `designs_*` and `design_file_*` commands.
//!
//! Wraps `persistence::saved_designs` (the named-save shelf, backed by
//! `saved_designs.json` in the app data directory) and
//! `persistence::design_file` (the portable `.breakscale` file format).
//! Command names/argument shapes match `desktop/src/lib/api/designs.ts`
//! exactly.

use crate::commands::{app_data_file, SAVED_DESIGNS_FILE};
use crate::error::AppResult;
use crate::persistence::{design_file, saved_designs};
use crate::sim::types::Topology;
use tauri::AppHandle;

#[tauri::command]
pub fn designs_list(app: AppHandle) -> AppResult<Vec<saved_designs::SavedSummary>> {
    let path = app_data_file(&app, SAVED_DESIGNS_FILE)?;
    Ok(saved_designs::list_designs(&path))
}

#[tauri::command]
pub fn designs_save(
    app: AppHandle,
    name: String,
    topology: Topology,
) -> AppResult<saved_designs::SaveResult> {
    let path = app_data_file(&app, SAVED_DESIGNS_FILE)?;
    Ok(saved_designs::save_design(&path, &name, &topology))
}

#[tauri::command]
pub fn designs_get(app: AppHandle, id: String) -> AppResult<saved_designs::SavedDesign> {
    let path = app_data_file(&app, SAVED_DESIGNS_FILE)?;
    saved_designs::get_design(&path, &id)
        .ok_or_else(|| crate::error::AppError::NotFound("That design is no longer on the shelf.".into()))
}

#[tauri::command]
pub fn designs_delete(app: AppHandle, id: String) -> AppResult<()> {
    let path = app_data_file(&app, SAVED_DESIGNS_FILE)?;
    saved_designs::delete_design(&path, &id)
}

#[tauri::command]
pub fn designs_rename(app: AppHandle, id: String, name: String) -> AppResult<bool> {
    let path = app_data_file(&app, SAVED_DESIGNS_FILE)?;
    Ok(saved_designs::rename_design(&path, &id, &name))
}

#[tauri::command]
pub fn design_file_build(topology: Topology, name: Option<String>) -> AppResult<String> {
    Ok(design_file::build_design_file(&topology, name.as_deref()))
}

#[tauri::command]
pub fn design_file_parse(text: String) -> AppResult<design_file::DesignParseResult> {
    Ok(design_file::parse_design_file(&text))
}

#[tauri::command]
pub fn design_file_write(
    path: String,
    topology: Topology,
    name: Option<String>,
) -> AppResult<()> {
    let text = design_file::build_design_file(&topology, name.as_deref());
    std::fs::write(&path, text)?;
    Ok(())
}

#[tauri::command]
pub fn design_file_read(path: String) -> AppResult<design_file::DesignParseResult> {
    Ok(design_file::read_design_file(std::path::Path::new(&path)))
}
