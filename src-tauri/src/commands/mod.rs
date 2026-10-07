//! Tauri command handlers, grouped by domain.
//!
//! Every function in this tree is a thin wrapper: it validates/resolves
//! whatever is Tauri-specific (an `AppHandle`, a managed-state lock, a path
//! under the app data directory), then calls straight into the domain
//! modules (`sim`, `persistence`, `sysdesign`, `vendors`) for the actual
//! logic. No business logic belongs here -- see MIGRATION_PLAN.md #3.

pub mod backup;
pub mod challenges;
pub mod designs;
pub mod glossary;
pub mod presets;
pub mod settings;
pub mod sim;
pub mod sysdesign;
pub mod vendors;

use crate::error::{AppError, AppResult};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Resolve a file path inside the app's OS-specific data directory,
/// creating the directory if it does not exist yet.
///
/// Every persistence command resolves ITS OWN path this way rather than
/// accepting a raw path from the frontend for "where do my saved designs
/// live" -- the frontend never gets to say where app-owned state is
/// stored, only where a document it explicitly exported/imported via a
/// native file dialog goes. See MIGRATION_PLAN.md #11.
pub(crate) fn app_data_file(app: &AppHandle, file_name: &str) -> AppResult<PathBuf> {
    let dir = app.path().app_data_dir().map_err(|e| {
        AppError::Io(format!(
            "Could not find a place on this computer to store app data: {e}"
        ))
    })?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(file_name))
}

/// Filename for the saved-designs shelf, shared between `commands::designs`
/// and `commands::backup` (which must back the same file up).
pub(crate) const SAVED_DESIGNS_FILE: &str = "saved_designs.json";
pub(crate) const PREFERENCES_FILE: &str = "preferences.json";
pub(crate) const LAYOUT_FILE: &str = "layout.json";
