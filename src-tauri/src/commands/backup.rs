//! `backup_*` commands. Command names match `desktop/src/lib/api/backup.ts`.

use crate::error::AppResult;
use crate::persistence::backup;
use tauri::{AppHandle, Manager};

fn app_data_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    let dir = app.path().app_data_dir().map_err(|e| {
        crate::error::AppError::Io(format!(
            "Could not find a place on this computer to store app data: {e}"
        ))
    })?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[tauri::command]
pub fn backup_write(app: AppHandle, path: String) -> AppResult<()> {
    let dir = app_data_dir(&app)?;
    backup::write_backup(&dir, std::path::Path::new(&path))
}

#[tauri::command]
pub fn backup_restore_from_path(app: AppHandle, path: String) -> AppResult<backup::BackupResult> {
    let dir = app_data_dir(&app)?;
    Ok(backup::restore_backup_from_path(&dir, std::path::Path::new(&path)))
}
