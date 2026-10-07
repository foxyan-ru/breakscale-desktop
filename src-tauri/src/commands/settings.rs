//! `settings_load`/`settings_save`/`layout_load`/`layout_save` commands.
//!
//! Preferences and panel-layout state are frontend-owned shapes (see
//! `desktop/src/lib/state/settings.svelte.ts`'s `Preferences` interface and
//! its `TODO(integration)` note) with no Rust domain type of their own --
//! this crate treats both as opaque JSON, the same way
//! `persistence::backup::acceptable` already only shape-checks them
//! (object, not array) rather than deeply validating every field, since
//! each preference's own loader on the frontend already falls back per
//! field for anything malformed.

use crate::commands::{app_data_file, LAYOUT_FILE, PREFERENCES_FILE};
use crate::error::AppResult;
use serde_json::Value;
use tauri::AppHandle;

fn load(app: &AppHandle, file: &str) -> AppResult<Value> {
    let path = app_data_file(app, file)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str(&text).unwrap_or(Value::Null)),
        Err(_) => Ok(Value::Null),
    }
}

fn save(app: &AppHandle, file: &str, value: Value) -> AppResult<()> {
    let path = app_data_file(app, file)?;
    std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

#[tauri::command]
pub fn settings_load(app: AppHandle) -> AppResult<Value> {
    load(&app, PREFERENCES_FILE)
}

#[tauri::command]
pub fn settings_save(app: AppHandle, preferences: Value) -> AppResult<()> {
    save(&app, PREFERENCES_FILE, preferences)
}

#[tauri::command]
pub fn layout_load(app: AppHandle) -> AppResult<Value> {
    load(&app, LAYOUT_FILE)
}

#[tauri::command]
pub fn layout_save(app: AppHandle, layout: Value) -> AppResult<()> {
    save(&app, LAYOUT_FILE, layout)
}
