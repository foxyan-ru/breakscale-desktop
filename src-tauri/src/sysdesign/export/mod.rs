//! Export formats for a `SystemDesignDoc` -- MIGRATION_PLAN.md #8. All four
//! read the same document; only the shape of the output differs.

pub mod json;
pub mod markdown;
pub mod terraform;
pub mod yaml;

use crate::error::AppResult;
use crate::sysdesign::model::{ExportFormat, SystemDesignDoc};

/// What one export call produced. `Multi` is used by formats (Terraform)
/// that naturally emit more than one file; the caller (a Tauri command)
/// decides whether `path` is a file or a directory based on which variant
/// comes back, or simply by checking the `ExportFormat` up front.
pub enum ExportOutput {
    Single { file_name: String, contents: String },
    /// `(file_name, contents)` pairs, in the order they should be written.
    Multi(Vec<(String, String)>),
}

/// Export `doc` in `format`. JSON/YAML/Markdown never fail once `doc` exists
/// in memory (there is nothing left to validate -- `serde_json`/`serde_yaml`
/// only error on types they cannot represent, and this document's shape
/// always can be); the `AppResult` exists because the underlying serializers
/// are fallible APIs, not because a real failure is expected here.
pub fn export(doc: &SystemDesignDoc, format: ExportFormat) -> AppResult<ExportOutput> {
    let stem = file_stem(&doc.name);
    match format {
        ExportFormat::Json => Ok(ExportOutput::Single {
            file_name: format!("{stem}.json"),
            contents: json::export(doc)?,
        }),
        ExportFormat::Yaml => Ok(ExportOutput::Single {
            file_name: format!("{stem}.yaml"),
            contents: yaml::export(doc)?,
        }),
        ExportFormat::Markdown => Ok(ExportOutput::Single {
            file_name: format!("{stem}.md"),
            contents: markdown::export(doc),
        }),
        ExportFormat::Terraform => Ok(ExportOutput::Multi(terraform::export(doc))),
    }
}

/// Lowercase, hyphenated, ASCII file stem from a design name -- the same
/// convention `designFile.ts`'s `slug()` uses for `.breakscale` file names,
/// so an exported file's name reads the same way a saved design's does.
fn file_stem(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-');
    let collapsed = trimmed
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "design".to_string()
    } else {
        collapsed.chars().take(60).collect()
    }
}
