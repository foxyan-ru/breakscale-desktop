//! The one error type every Tauri command returns.
//!
//! A `Debug`-formatted Rust error (which can carry a full file-system path,
//! or a source-line reference) must never cross the IPC boundary to the
//! webview -- see MIGRATION_PLAN.md #11. Every command therefore returns
//! `Result<T, AppError>`, and `AppError` serialises to exactly the fields the
//! frontend is allowed to see: a machine-readable `kind` and a sentence a
//! user can act on, in the same voice the web app's own validation errors
//! use (see `persistence::design_file`).

use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "message")]
pub enum AppError {
    /// Input failed validation before it ever reached the engine or the
    /// filesystem -- a malformed topology, a name that is empty after
    /// trimming, a file that is not a Breakscale document.
    Validation(String),
    /// The thing asked for (a saved design, a node id) does not exist.
    NotFound(String),
    /// A filesystem operation failed for a reason the caller cannot fix by
    /// changing their input (permissions, disk full, path vanished).
    Io(String),
    /// A value could not be encoded/decoded (JSON/YAML build or parse).
    Serialization(String),
    /// The simulation is not in the state the command requires (e.g. no
    /// engine has been created yet).
    State(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Validation(m)
            | AppError::NotFound(m)
            | AppError::Io(m)
            | AppError::Serialization(m)
            | AppError::State(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        // The Rust io::Error text is developer-facing detail, not the
        // user-facing sentence; commands that can give a better message
        // (e.g. "That file could not be read.") should map the error
        // themselves rather than relying on this fallback.
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Serialization(e.to_string())
    }
}

impl From<serde_yaml::Error> for AppError {
    fn from(e: serde_yaml::Error) -> Self {
        AppError::Serialization(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
