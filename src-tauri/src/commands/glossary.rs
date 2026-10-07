//! `glossary_list` command, invented by the frontend's Glossary/Tooltip
//! port -- see `desktop/src/lib/components/glossary/Glossary.svelte` and
//! `desktop/src/lib/components/shell/Tooltip.svelte`'s integration notes.

use crate::error::AppResult;
use crate::sim::glossary::{self, GlossaryEntry};

#[tauri::command]
pub fn glossary_list() -> AppResult<Vec<GlossaryEntry>> {
    Ok(glossary::load())
}
