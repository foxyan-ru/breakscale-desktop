//! Named saves.
//!
//! Ported from `src/savedDesigns.ts`, reinterpreted per MIGRATION_PLAN.md §7:
//! there is no `localStorage` on the backend side of a Tauri app, so this
//! shelf of designs lives as one JSON file (`saved_designs.json`) in the OS
//! app-data directory instead. Every function takes an explicit `path`
//! rather than resolving it itself, so this module stays free of any
//! Tauri-runtime dependency and is easy to unit test: the caller (a Tauri
//! command, written elsewhere) resolves the real app-data path via
//! `tauri::Manager::path().app_data_dir()` and passes it in.
//!
//! Everything read back crosses a trust boundary exactly as a `localStorage`
//! read did: the file can be edited by hand, half-written if the process
//! was killed mid-write, or left over from an older version of the format,
//! so nothing here trusts its own shelf.

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::sim::types::Topology;

/// How many designs are kept.
///
/// A cap rather than unlimited, mirroring the TS localStorage-budget
/// reasoning even though a file on disk has no comparable 5MB ceiling: the
/// point was never the byte budget, it was that twenty is already more
/// ideas than anyone juggles at once. The oldest is evicted when a new save
/// would exceed it, and the caller is told which, so it can say so rather
/// than letting work vanish quietly.
pub const MAX_SAVED: usize = 20;

/// Longest name kept. Anything past this is a paragraph, not a name.
pub const MAX_NAME: usize = 60;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SavedDesign {
    /// Stable id, so a rename never orphans the entry.
    pub id: String,
    pub name: String,
    /// Milliseconds since the epoch, for ordering and for "saved 2 hours ago".
    pub saved_at: i64,
    pub topology: Topology,
}

/// What the interface shows in a list, without paying to parse every
/// topology's full contents into the frontend.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSummary {
    pub id: String,
    pub name: String,
    pub saved_at: i64,
    pub node_count: usize,
}

/// Same `{ ok, ...variant fields }` flattened-struct pattern as
/// `design_file::DesignParseResult` -- see that type's doc comment for why
/// this shape rather than a tagged enum.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `Some(None)` -> `"evicted":null`; the outer `None` skips the field
    /// entirely (only on the error branch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evicted: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SaveResult {
    pub fn ok(id: String, evicted: Option<String>) -> Self {
        Self {
            ok: true,
            id: Some(id),
            evicted: Some(evicted),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            id: None,
            evicted: None,
            error: Some(message.into()),
        }
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Write-then-rename rather than a direct write, so a crash mid-write can
/// never leave a half-written JSON file behind. This is a deliberate
/// upgrade over the TS baseline: `localStorage.setItem` is atomic by
/// construction (the browser guarantees it), but a plain `std::fs::write`
/// to the real file is not, so the port adds back the guarantee the
/// original storage layer gave for free. Also used by `backup::restore_backup`
/// when it writes the restored files back.
pub(crate) fn atomic_write(path: &std::path::Path, text: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(".tmp");
    let tmp_path = std::path::PathBuf::from(tmp_os);
    std::fs::write(&tmp_path, text)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Validate one stored entry. Returns `None` rather than failing the whole
/// load, so one corrupt design costs its own row and not the whole list --
/// ported from `parseEntry` in savedDesigns.ts, which a plain
/// `Vec<SavedDesign>` deserialize cannot reproduce (one bad element would
/// fail the entire array), hence the manual `serde_json::Value` walk here.
fn parse_entry(raw: &serde_json::Value) -> Option<SavedDesign> {
    let obj = raw.as_object()?;

    let id = obj.get("id")?.as_str()?;
    if id.is_empty() {
        return None;
    }

    let raw_topology = obj
        .get("topology")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let topology = super::design_file::is_topology(&raw_topology)?;

    let annotations = super::design_file::sanitize_annotations(topology.annotations.clone());
    let topology = Topology {
        nodes: topology.nodes,
        edges: topology.edges,
        annotations,
    };

    // NOTE: existence is checked against the TRIMMED name, but the stored
    // value is sliced from the ORIGINAL (untrimmed) string -- this exactly
    // matches `e.name.slice(0, MAX_NAME)` in the TS `parseEntry`, which
    // checks `e.name.trim()` truthy but then slices `e.name` itself, not
    // the trimmed copy. Kept byte-for-byte faithful rather than "fixed".
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.chars().take(MAX_NAME).collect::<String>())
        .unwrap_or_else(|| "Untitled".to_string());

    let saved_at = obj
        .get("savedAt")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .map(|v| v as i64)
        .unwrap_or(0);

    Some(SavedDesign {
        id: id.to_string(),
        name,
        saved_at,
        topology,
    })
}

/// Everything on the shelf, newest first. Never errors: an unreadable,
/// missing or corrupt file reads back as an empty shelf, exactly like the
/// TS `loadDesigns()`'s try/catch.
pub fn load_designs(path: &std::path::Path) -> Vec<SavedDesign> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(arr) = parsed.as_array() else {
        return Vec::new();
    };

    let mut designs: Vec<SavedDesign> = arr.iter().filter_map(parse_entry).collect();
    designs.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
    designs
}

/// The list the interface renders, without the topologies.
pub fn list_designs(path: &std::path::Path) -> Vec<SavedSummary> {
    load_designs(path)
        .into_iter()
        .map(|d| SavedSummary {
            id: d.id,
            name: d.name,
            saved_at: d.saved_at,
            node_count: d.topology.nodes.len(),
        })
        .collect()
}

/// Save the current design under a name.
///
/// Saving over an existing NAME (case-insensitive) replaces that entry
/// rather than adding a second one, which is what someone pressing save
/// twice means. Everything else is an insert.
///
/// The TS error text for a full shelf ("There is no room left in this
/// browser...") is `localStorage`-specific and would be a false statement
/// on a desktop filesystem, so this port adapts the wording to the real
/// failure mode (disk, not browser quota) while keeping the same meaning
/// and the same trigger. Flagged here because it is the one message in this
/// module that is NOT a byte-for-byte port of its TS original.
pub fn save_design(path: &std::path::Path, name: &str, topology: &Topology) -> SaveResult {
    let clean: String = name.trim().chars().take(MAX_NAME).collect();
    if clean.is_empty() {
        return SaveResult::err("Give the design a name first.");
    }

    let mut designs = load_designs(path);
    let existing = designs
        .iter()
        .position(|d| d.name.to_lowercase() == clean.to_lowercase());

    let id = match existing {
        Some(i) => designs[i].id.clone(),
        None => crate::util::id::new_id(),
    };
    let entry = SavedDesign {
        id: id.clone(),
        name: clean,
        saved_at: now_millis(),
        topology: topology.clone(),
    };

    match existing {
        Some(i) => designs[i] = entry,
        None => designs.insert(0, entry),
    }
    designs.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));

    // The oldest goes when the shelf is full, and the caller is told which.
    let mut evicted: Option<String> = None;
    if designs.len() > MAX_SAVED {
        evicted = designs.last().map(|d| d.name.clone());
        designs.truncate(MAX_SAVED);
    }

    let text = match serde_json::to_string_pretty(&designs) {
        Ok(t) => t,
        Err(_) => {
            return SaveResult::err("There is no room left on this device to save another design.");
        }
    };
    if atomic_write(path, &text).is_err() {
        return SaveResult::err("There is no room left on this device to save another design.");
    }

    SaveResult::ok(id, evicted)
}

/// One design, by id, or `None` if it is gone.
pub fn get_design(path: &std::path::Path, id: &str) -> Option<SavedDesign> {
    load_designs(path).into_iter().find(|d| d.id == id)
}

/// Remove one design by id. Unlike the TS `deleteDesign` (which silently
/// discards a failed `write()`), this surfaces an I/O failure to the
/// caller as `AppError::Io`, per the `AppResult<()>` signature specified
/// for this port -- errors reaching the frontend should say so rather than
/// pretending the delete worked.
pub fn delete_design(path: &std::path::Path, id: &str) -> AppResult<()> {
    let designs: Vec<SavedDesign> = load_designs(path)
        .into_iter()
        .filter(|d| d.id != id)
        .collect();
    let text = serde_json::to_string_pretty(&designs)?;
    atomic_write(path, &text)
}

/// Rename in place. Returns `false` when the name is empty, already taken
/// by a different id, the id does not exist, or the write itself fails.
pub fn rename_design(path: &std::path::Path, id: &str, name: &str) -> bool {
    let clean: String = name.trim().chars().take(MAX_NAME).collect();
    if clean.is_empty() {
        return false;
    }

    let mut designs = load_designs(path);
    let taken = designs
        .iter()
        .any(|d| d.id != id && d.name.to_lowercase() == clean.to_lowercase());
    if taken {
        return false;
    }

    let Some(idx) = designs.iter().position(|d| d.id == id) else {
        return false;
    };
    designs[idx].name = clean;

    let text = match serde_json::to_string_pretty(&designs) {
        Ok(t) => t,
        Err(_) => return false,
    };
    atomic_write(path, &text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "breakscale_saved_designs_test_{name}_{}.json",
            now_millis()
        ));
        p
    }

    fn sample_topology() -> serde_json::Value {
        serde_json::json!({
            "nodes": [{
                "id": "n1", "kind": "client", "label": "Client", "x": 0, "y": 0,
                "config": {
                    "capacity": 1, "serviceMs": 1, "serviceCv": 0, "queueLimit": 1,
                    "hitRate": 0, "errorRate": 0, "timeoutMs": 0, "retries": 0, "rps": 1,
                    "replicaCount": 1, "replicationLagMs": 0, "readFraction": 1,
                    "shardCount": 1, "shardCapacity": 1, "hotKeyFraction": 0
                }
            }],
            "edges": []
        })
    }

    #[test]
    fn load_missing_file_is_empty() {
        let path = tmp_path("missing");
        assert!(load_designs(&path).is_empty());
    }

    #[test]
    fn load_corrupt_file_is_empty() {
        let path = tmp_path("corrupt");
        std::fs::write(&path, "not json at all").unwrap();
        assert!(load_designs(&path).is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = tmp_path("roundtrip");
        let topology: Topology = serde_json::from_value(sample_topology()).unwrap();
        let result = save_design(&path, "My Design", &topology);
        assert!(result.ok);

        let loaded = load_designs(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "My Design");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("json.tmp"));
    }

    #[test]
    fn save_rejects_empty_name() {
        let path = tmp_path("emptyname");
        let topology: Topology = serde_json::from_value(sample_topology()).unwrap();
        let result = save_design(&path, "   ", &topology);
        assert!(!result.ok);
        assert_eq!(result.error.as_deref(), Some("Give the design a name first."));
    }
}
