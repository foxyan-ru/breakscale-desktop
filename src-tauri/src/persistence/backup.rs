//! Whole-app backup.
//!
//! Ported from `src/backup.ts`, reinterpreted per MIGRATION_PLAN.md §7: the
//! TS version bundles a fixed list of `localStorage` keys
//! (`BACKED_UP_KEYS`); this desktop port has no `localStorage` at all, so it
//! bundles the fixed list of app-data FILES this app owns instead
//! (`BACKED_UP_FILES`). The bundle format and trust-boundary shape carry
//! over unchanged: one JSON file naming which files it carries, each one's
//! raw text keyed by file name, restored by REPLACING rather than merging.
//!
//! `saved_designs.json` is checked only for being a JSON array, same depth as
//! the TS `acceptable()`'s `Array.isArray(parsed)` -- a per-row `topology`
//! check here would reject the whole file over one row, which is exactly the
//! erasure the rejected-design guard (`saved_designs::Shelf`, upstream #54)
//! exists to prevent; `preferences.json` and `layout.json` do not have Rust
//! structs elsewhere in this codebase yet, so they are carried as opaque,
//! shape-checked-only JSON, exactly as the TS `acceptable()` treats any key
//! it does not special-case.

use serde::Serialize;

use crate::error::AppResult;

pub const BACKUP_APP: &str = "breakscale-backup";
pub const BACKUP_VERSION: u32 = 1;
pub const BACKUP_EXT: &str = ".breakscale-backup.json";

/// The files a backup carries.
///
/// Listed explicitly rather than swept from the app-data directory by glob,
/// so a file added later is a deliberate decision to include it, mirroring
/// why `BACKED_UP_KEYS` in the TS source is an explicit list rather than a
/// `localStorage` prefix scan.
pub const BACKED_UP_FILES: &[&str] = &["saved_designs.json", "preferences.json", "layout.json"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupFile {
    app: String,
    version: u32,
    saved_at: String,
    /// Raw stored file text, keyed by file name, exactly as `BackupFile.data`
    /// keys by `localStorage` key in the TS source. A `BTreeMap` rather than
    /// a `HashMap` so two backups of identical content serialise identically
    /// (byte-for-byte diffable), which a `HashMap`'s unordered iteration
    /// would not guarantee.
    data: std::collections::BTreeMap<String, String>,
}

/// Same `{ ok, ...variant fields }` flattened-struct pattern used throughout
/// this module -- see `design_file::DesignParseResult` for why.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BackupResult {
    pub fn ok(restored: Vec<String>) -> Self {
        Self {
            ok: true,
            restored: Some(restored),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            restored: None,
            error: Some(message.into()),
        }
    }
}

/// Everything this app-data directory holds, as the text of a backup file.
/// A file that does not exist yet (never saved to) is simply left out,
/// matching the TS `buildBackup`'s "a partial backup of what can be read
/// beats refusing to make one at all" reasoning for blocked storage.
pub fn build_backup(app_data_dir: &std::path::Path) -> String {
    let mut data = std::collections::BTreeMap::new();
    for &file in BACKED_UP_FILES {
        if let Ok(text) = std::fs::read_to_string(app_data_dir.join(file)) {
            data.insert(file.to_string(), text);
        }
    }
    let body = BackupFile {
        app: BACKUP_APP.to_string(),
        version: BACKUP_VERSION,
        saved_at: crate::util::iso8601::now(),
        data,
    };
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

/// Write a backup of `app_data_dir` to `dest`. The TS equivalent
/// (`downloadBackup`) hands the browser a Blob; here the caller (a Tauri
/// command, using the save-file dialog) picks `dest` and this just writes
/// the bytes.
pub fn write_backup(app_data_dir: &std::path::Path, dest: &std::path::Path) -> AppResult<()> {
    let text = build_backup(app_data_dir);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, text)?;
    Ok(())
}

/// Is this stored file's text one we are willing to write back?
///
/// Ported from `acceptable()` in backup.ts. An earlier version of this port
/// deliberately strengthened the designs-key check to require every row's
/// `topology` to pass `design_file::is_topology`, rejecting the whole file
/// over one malformed row -- self-flagged at the time as stricter than the
/// literal TS behaviour. Upstream PR #54 (`e0241b03`) makes that
/// strengthening actively wrong: the whole point of the rejected-design
/// guard (see `saved_designs::Shelf`) is that a row this build cannot open
/// must survive, not be erased, and a backup restore that refuses to write
/// `saved_designs.json` back because it contains exactly such a row erases
/// it just the same, only via restore instead of save/rename/delete. So this
/// now matches the TS `acceptable()` (backup.ts:99-104 in the pinned local
/// copy) again: only `Array.isArray(parsed)` is checked here, and a bad row
/// is the shelf's own problem to keep-but-not-show, not this gate's to
/// discard.
fn acceptable(file: &str, value: &str) -> bool {
    let parsed: serde_json::Value = match serde_json::from_str(value) {
        Ok(v) => v,
        Err(_) => return false,
    };

    if file == "saved_designs.json" {
        return parsed.as_array().is_some();
    }

    // preferences.json / layout.json: shape-checked only, not deeply
    // validated -- their own loaders (elsewhere in the codebase) validate
    // every field and fall back per field rather than trusting the blob,
    // matching the TS comment on `acceptable()`'s fallback branch.
    matches!(parsed, serde_json::Value::Object(_))
}

/// Write a backup back into `app_data_dir`.
///
/// Replaces rather than merges: merging two shelves means deciding which
/// copy of a design called "draft" wins, and there is no answer to that a
/// student would predict. Ported from `restoreBackup` in backup.ts,
/// including its exact validation cascade and error strings; only the
/// "no room" message is adapted (browser storage quota -> disk), the same
/// deliberate adaptation `saved_designs::save_design` makes.
pub fn restore_backup(app_data_dir: &std::path::Path, text: &str) -> BackupResult {
    if text.trim().is_empty() {
        return BackupResult::err("That file is empty.");
    }

    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            return BackupResult::err(
                "That file is not valid JSON. It may have been edited or partly saved.",
            );
        }
    };

    let Some(obj) = parsed.as_object() else {
        return BackupResult::err("That file does not hold a Breakscale backup.");
    };

    let app = obj.get("app").and_then(|v| v.as_str());
    if app != Some(BACKUP_APP) {
        return BackupResult::err("That file does not hold a Breakscale backup.");
    }

    let version = obj.get("version").and_then(|v| v.as_f64()).unwrap_or(0.0);
    if version > BACKUP_VERSION as f64 {
        return BackupResult::err("That backup was made by a newer version of Breakscale.");
    }

    let Some(data) = obj.get("data").and_then(|v| v.as_object()) else {
        return BackupResult::err("That backup is missing its contents.");
    };

    let mut restored: Vec<String> = Vec::new();
    for &file in BACKED_UP_FILES {
        let Some(value) = data.get(file).and_then(|v| v.as_str()) else {
            continue;
        };
        if !acceptable(file, value) {
            continue;
        }
        if super::saved_designs::atomic_write(&app_data_dir.join(file), value).is_err() {
            return BackupResult::err("There is no room left on this device to restore that backup.");
        }
        restored.push(file.to_string());
    }

    if restored.is_empty() {
        return BackupResult::err("That backup held nothing this version can read.");
    }
    BackupResult::ok(restored)
}

/// Read `src` and restore it into `app_data_dir`. No TS equivalent exists
/// (the browser's file input hands `restoreBackup` text directly); this is
/// the filesystem-facing counterpart, mirroring
/// `design_file::read_design_file`'s read-failure message for the same
/// reason: a read can fail on its own between the pick and the read, and
/// that failure deserves a sentence, not a propagated I/O error.
pub fn restore_backup_from_path(
    app_data_dir: &std::path::Path,
    src: &std::path::Path,
) -> BackupResult {
    match std::fs::read_to_string(src) {
        Ok(text) => restore_backup(app_data_dir, &text),
        Err(_) => BackupResult::err("That file could not be read."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let mut p = std::env::temp_dir();
        p.push(format!("breakscale_backup_test_{name}_{millis}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn build_backup_on_empty_dir_has_no_data_but_still_builds() {
        let dir = tmp_dir("empty");
        let text = build_backup(&dir);
        assert!(text.contains("\"app\": \"breakscale-backup\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_rejects_empty_text() {
        let dir = tmp_dir("restore_empty");
        let r = restore_backup(&dir, "  ");
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("That file is empty."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_rejects_wrong_app_marker() {
        let dir = tmp_dir("restore_wrong_app");
        let r = restore_backup(&dir, r#"{"app":"nope","version":1,"data":{}}"#);
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("That file does not hold a Breakscale backup.")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_round_trips_preferences() {
        let dir = tmp_dir("restore_roundtrip");
        let backup = serde_json::json!({
            "app": "breakscale-backup",
            "version": 1,
            "savedAt": "2024-01-15T10:30:00.000Z",
            "data": {
                "preferences.json": "{\"theme\":\"dark\"}"
            }
        })
        .to_string();
        let r = restore_backup(&dir, &backup);
        assert!(r.ok);
        assert_eq!(r.restored.as_deref(), Some(&["preferences.json".to_string()][..]));
        let written = std::fs::read_to_string(dir.join("preferences.json")).unwrap();
        assert_eq!(written, "{\"theme\":\"dark\"}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// WHY: a `saved_designs.json` backed up while it held a row the guard
    /// kept-but-could-not-open (#54) must still restore -- `acceptable()`
    /// rejecting the whole array over that one row would erase it on
    /// restore, the same mistake the guard exists to prevent on save/rename/
    /// delete. Pinned by checking the row survives byte-for-byte, since
    /// `restore_backup` writes the designs text back verbatim rather than
    /// reparsing it.
    #[test]
    fn restore_keeps_a_designs_backup_with_an_unreadable_row() {
        let dir = tmp_dir("restore_unreadable_row");
        let dup_node = serde_json::json!({
            "id": "dup", "kind": "client", "label": "dup", "x": 0, "y": 0,
            "config": {
                "capacity": 1, "serviceMs": 1, "serviceCv": 0, "queueLimit": 1,
                "hitRate": 0, "errorRate": 0, "timeoutMs": 0, "retries": 0, "rps": 1,
                "replicaCount": 1, "replicationLagMs": 0, "readFraction": 1,
                "shardCount": 1, "shardCapacity": 1, "hotKeyFraction": 0
            }
        });
        let designs_text = serde_json::json!([
            {
                "id": "a",
                "name": "Week 3 coursework",
                "savedAt": 1,
                "topology": { "nodes": [dup_node.clone(), dup_node], "edges": [] }
            }
        ])
        .to_string();

        let backup = serde_json::json!({
            "app": "breakscale-backup",
            "version": 1,
            "savedAt": "2024-01-15T10:30:00.000Z",
            "data": { "saved_designs.json": designs_text }
        })
        .to_string();

        let r = restore_backup(&dir, &backup);
        assert!(r.ok);
        assert_eq!(
            r.restored.as_deref(),
            Some(&["saved_designs.json".to_string()][..])
        );
        let written = std::fs::read_to_string(dir.join("saved_designs.json")).unwrap();
        assert_eq!(written, designs_text);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
