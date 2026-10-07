//! `sysdesign_*` commands. Names match `desktop/src/lib/api/sysdesign.ts`.

use crate::error::{AppError, AppResult};
use crate::sim::types::Topology;
use crate::sysdesign::model::{ExportFormat, SystemDesignDoc, ValidationIssue};
use crate::sysdesign::{derive, export, validate};
use tauri::AppHandle;

/// Where a design's `SystemDesignDoc` is persisted: one JSON file per
/// design id, under `<app-data>/sysdesigns/`. Separate from
/// `saved_designs.json` (which holds the `Topology` a design's canvas
/// state) because a `SystemDesignDoc` is optional, larger, and edited on
/// its own independent cadence -- keeping the two files apart means saving
/// one never risks corrupting the other.
fn doc_path(app: &AppHandle, design_id: &str) -> AppResult<std::path::PathBuf> {
    let safe_id: String = design_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_id.is_empty() {
        return Err(AppError::Validation("That design has no id to save against.".into()));
    }
    let dir = crate::commands::app_data_file(app, "sysdesigns")?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("{safe_id}.json")))
}

#[tauri::command]
pub fn sysdesign_derive_high_level(topology: Topology) -> AppResult<SystemDesignDoc> {
    // A fresh, not-yet-saved document: the frontend names/saves it later
    // (see `sysdesign_save`), so the id is a placeholder the caller is
    // expected to replace once the user picks a real saved-design id to
    // attach this to.
    Ok(derive::from_topology(&topology, "unsaved", "Untitled design"))
}

#[tauri::command]
pub fn sysdesign_save(app: AppHandle, doc: SystemDesignDoc) -> AppResult<()> {
    let path = doc_path(&app, &doc.id)?;
    let text = serde_json::to_string_pretty(&doc)?;
    std::fs::write(path, text)?;
    Ok(())
}

#[tauri::command]
pub fn sysdesign_load(app: AppHandle, design_id: String) -> AppResult<SystemDesignDoc> {
    let path = doc_path(&app, &design_id)?;
    let text = std::fs::read_to_string(&path).map_err(|_| {
        AppError::NotFound("That design has no saved architecture document yet.".into())
    })?;
    serde_json::from_str(&text).map_err(|e| {
        AppError::Serialization(format!("That architecture document is damaged: {e}"))
    })
}

#[tauri::command]
pub fn sysdesign_validate(doc: SystemDesignDoc) -> AppResult<Vec<ValidationIssue>> {
    Ok(validate::validate(&doc))
}

#[tauri::command]
pub fn sysdesign_export(doc: SystemDesignDoc, format: ExportFormat, path: String) -> AppResult<()> {
    match export::export(&doc, format)? {
        export::ExportOutput::Single { contents, .. } => {
            std::fs::write(&path, contents)?;
        }
        export::ExportOutput::Multi(files) => {
            std::fs::create_dir_all(&path)?;
            let dir = std::path::Path::new(&path);
            for (file_name, contents) in files {
                std::fs::write(dir.join(file_name), contents)?;
            }
        }
    }
    Ok(())
}
