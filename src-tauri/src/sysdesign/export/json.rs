//! JSON export: the document, serialized exactly as the wire format the
//! rest of the app already speaks. No special-casing for export -- a
//! `SystemDesignDoc` exported as JSON and one sent over Tauri IPC look
//! identical, which is what makes re-importing an exported file trivial.

use crate::error::AppResult;
use crate::sysdesign::model::SystemDesignDoc;

pub fn export(doc: &SystemDesignDoc) -> AppResult<String> {
    Ok(serde_json::to_string_pretty(doc)?)
}
