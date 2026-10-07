//! Saving a design to a file, and reading one back.
//!
//! Ported from `src/designFile.ts`. A share link carries a design between two
//! browsers; a file carries it between two machines, or between today and a
//! reformatted laptop. Everything that comes back in through a file picker is
//! treated as hostile until every field has been checked -- this module is
//! that trust boundary, and it is pure data-in data-out so it can be unit
//! tested without touching the filesystem (only `read_design_file` does).

use serde::{Deserialize, Serialize};

use crate::sim::types::{Annotation, NodeKind, Topology};

/// Marker written into every exported file, and required on the way back in.
/// Not security, a courtesy: a student who opens the wrong JSON gets "this is
/// not a Breakscale design" instead of a wall of field complaints about a
/// file that was never ours to begin with.
pub const DESIGN_FILE_APP: &str = "breakscale";

/// Format version of the file body.
///
/// Bumped only when the shape changes in a way an older reader could not
/// understand. A reader accepts its own version and anything below it; a
/// file from the future is refused with a message that says to update,
/// rather than being half-read into a design missing whatever it gained.
pub const DESIGN_FILE_VERSION: u32 = 1;

/// The extension every exported design carries. `.json` stays accepted on
/// import (see `parse_design_file`'s caller), but every write uses this.
pub const DESIGN_FILE_EXT: &str = ".breakscale";

/// What one exported file holds.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignFile {
    pub app: String,
    pub version: u32,
    /// When it was written, ISO 8601. Informational; nothing reads it back.
    pub saved_at: String,
    /// The preset it started from, when it started from one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub topology: Topology,
}

/// A design that survived validation, or the reason it did not.
///
/// The TS source models this as a discriminated union
/// (`{ ok: true; topology; name } | { ok: false; error }`); serde's
/// internally-tagged enum representation would put a STRING tag on the wire
/// (`"ok":"true"`), not the boolean the TS type actually serialises. A plain
/// struct with optional, presence-gated fields produces the exact wire shape
/// instead: `{"ok":true,"topology":{...},"name":"..."|null}` or
/// `{"ok":false,"error":"..."}`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignParseResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topology: Option<Topology>,
    /// `Some(None)` serialises as `"name":null`; `None` (the outer option)
    /// skips the field entirely -- so the field is present exactly when
    /// `ok` is true, and its own value is `string | null`, matching
    /// `name: string | null` in the TS success variant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DesignParseResult {
    pub fn ok(topology: Topology, name: Option<String>) -> Self {
        Self {
            ok: true,
            topology: Some(topology),
            name: Some(name),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            topology: None,
            name: None,
            error: Some(message.into()),
        }
    }
}

/// Serialise the current design, annotations included, as the text of a
/// `.breakscale` file. Pretty printed with two spaces, matching
/// `JSON.stringify(body, null, 2)`, because a design file is something a
/// student may well open in an editor to see what a component's settings
/// actually are.
pub fn build_design_file(topology: &Topology, name: Option<&str>) -> String {
    let body = DesignFile {
        app: DESIGN_FILE_APP.to_string(),
        version: DESIGN_FILE_VERSION,
        saved_at: crate::util::iso8601::now(),
        name: name.filter(|n| !n.is_empty()).map(|n| n.to_string()),
        topology: topology.clone(),
    };
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

/// The file name a save offers, from the design's name plus today's date.
///
/// The date is in the name rather than left to the operating system's "(2)"
/// suffix, because a student exporting the same example twice in a week
/// wants to know which one is which without opening both.
pub fn design_file_name(name: Option<&str>) -> String {
    let stem = slug(name);
    let stem = if stem.is_empty() {
        "design".to_string()
    } else {
        stem
    };
    format!("{stem}-{}{DESIGN_FILE_EXT}", current_date_stamp())
}

/// Today's date as `YYYY-MM-DD`, reusing `util::iso8601`'s formatter rather
/// than adding a date dependency (see MIGRATION_PLAN.md #10): the date is
/// just the first ten characters of the full ISO-8601 stamp.
fn current_date_stamp() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    date_stamp_at(millis)
}

fn date_stamp_at(total_ms: i64) -> String {
    crate::util::iso8601::format_unix_ms(total_ms)[..10].to_string()
}

/// Lowercase, hyphenated, ASCII. A design name is free text and can hold
/// anything a student can type, including characters a file system will
/// refuse or a shell will treat as an argument, so nothing that is not a
/// letter, a digit or a hyphen survives this.
fn slug(name: Option<&str>) -> String {
    let Some(name) = name else {
        return String::new();
    };
    let lower = name.to_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').chars().take(60).collect()
}

/// Structural validation of anything from outside the type system: a stored
/// design, a `.breakscale` file, a pasted clipboard payload. Ported from
/// `isTopology` in `clipboard.ts`. Every field the engine will dereference
/// is checked before it is trusted, and a dangling edge (which would make
/// the engine route into nothing) is rejected outright.
///
/// Unlike the TS version, which returns a type-narrowed VIEW of the same
/// object, this has to hand back a concrete `Topology` the engine can
/// actually use, so on success it also deserialises the (now-validated)
/// value into one. See the module-level judgement-call notes in this
/// crate's persistence port for the one place that final step can reject a
/// value the manual checks below already passed.
pub(crate) fn is_topology(value: &serde_json::Value) -> Option<Topology> {
    let obj = value.as_object()?;
    let nodes = obj.get("nodes")?.as_array()?;
    let edges = obj.get("edges")?.as_array()?;

    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in nodes {
        let n = raw.as_object()?;

        let id = n.get("id")?.as_str()?;
        if id.is_empty() {
            return None;
        }
        // Label just has to be a string; unlike id, empty is allowed.
        n.get("label")?.as_str()?;

        // Validity of `kind` is delegated to the authoritative `NodeKind`
        // enum (owned by `sim::types`) rather than a hand-maintained copy of
        // its variant list, so the two can never drift apart.
        let kind = n.get("kind")?;
        if serde_json::from_value::<NodeKind>(kind.clone()).is_err() {
            return None;
        }

        let x = n.get("x")?.as_f64()?;
        let y = n.get("y")?.as_f64()?;
        if !x.is_finite() || !y.is_finite() {
            return None;
        }

        let cfg = n.get("config")?.as_object()?;
        for key in [
            "capacity",
            "serviceMs",
            "serviceCv",
            "queueLimit",
            "hitRate",
            "errorRate",
            "timeoutMs",
            "retries",
            "rps",
        ] {
            let v = cfg.get(key)?.as_f64()?;
            if !v.is_finite() {
                return None;
            }
        }

        if ids.contains(id) {
            return None;
        }
        ids.insert(id.to_string());
    }

    for raw in edges {
        let e = raw.as_object()?;

        let id = e.get("id")?.as_str()?;
        if id.is_empty() {
            return None;
        }
        let from = e.get("from")?.as_str()?;
        let to = e.get("to")?.as_str()?;
        if !ids.contains(from) || !ids.contains(to) {
            return None;
        }
        let weight = e.get("weight")?.as_f64()?;
        if !weight.is_finite() {
            return None;
        }
    }

    serde_json::from_value::<Topology>(value.clone()).ok()
}

/// Conservative placeholder: full annotation validation (note/section
/// field-level checks from sim/annotations.ts) is not yet ported; serde has
/// already guaranteed the value deserialized as an array, so this only
/// keeps the non-empty case, matching sanitizeAnnotations()'s "must be an
/// array" precondition. See MIGRATION_PLAN.md.
pub(crate) fn sanitize_annotations(raw: Option<Vec<Annotation>>) -> Option<Vec<Annotation>> {
    match raw {
        Some(items) if !items.is_empty() => Some(items),
        _ => None,
    }
}

/// Validate the text of a file someone chose or dropped.
///
/// This is the trust boundary. A file can hold anything: half a download,
/// someone else's JSON, a design hand-edited into nonsense. Nothing here
/// panics and nothing here half-applies. Either every field the engine will
/// dereference has been checked and a whole topology comes back, or a
/// sentence comes back saying what was wrong.
pub fn parse_design_file(text: &str) -> DesignParseResult {
    if text.trim().is_empty() {
        return DesignParseResult::err("That file is empty.");
    }

    let parsed: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => {
            return DesignParseResult::err(
                "That file is not valid JSON. It may have been edited or only partly saved.",
            );
        }
    };

    let obj = match parsed.as_object() {
        Some(o) => o,
        None => {
            return DesignParseResult::err("That file does not hold a Breakscale design.");
        }
    };

    let app = obj.get("app").and_then(|v| v.as_str());
    if app != Some(DESIGN_FILE_APP) {
        return DesignParseResult::err("That file does not hold a Breakscale design.");
    }

    let version = obj
        .get("version")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    if version > DESIGN_FILE_VERSION as f64 {
        return DesignParseResult::err(
            "That design was saved by a newer version of Breakscale. Reload the page and try again.",
        );
    }

    let raw_topology = obj.get("topology").cloned().unwrap_or(serde_json::Value::Null);
    let topology = match is_topology(&raw_topology) {
        Some(t) => t,
        None => {
            return DesignParseResult::err(
                "That design is damaged: a component or a connection is missing something the simulator needs.",
            );
        }
    };

    // Rebuilt field by field rather than reused wholesale, so nothing the
    // file carried beyond nodes, edges and annotations reaches the engine.
    let annotations = sanitize_annotations(topology.annotations.clone());
    let topology = Topology {
        nodes: topology.nodes,
        edges: topology.edges,
        annotations,
    };

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(80).collect::<String>());

    DesignParseResult::ok(topology, name)
}

/// Read a chosen file and validate it.
///
/// A read can fail on its own (the file was moved, or a permission was
/// withdrawn between the pick and the read), so that failure gets the same
/// treatment as a malformed body: a sentence, never a propagated I/O error.
pub fn read_design_file(path: &std::path::Path) -> DesignParseResult {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_design_file(&text),
        Err(_) => DesignParseResult::err("That file could not be read."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_hyphenates() {
        assert_eq!(slug(Some("My Cool Design!!")), "my-cool-design");
        assert_eq!(slug(Some("  --Leading/Trailing--  ")), "leading-trailing");
        assert_eq!(slug(None), "");
        assert_eq!(slug(Some("")), "");
    }

    #[test]
    fn slug_caps_at_60_chars() {
        let long = "a".repeat(100);
        assert_eq!(slug(Some(&long)).len(), 60);
    }

    #[test]
    fn date_stamp_matches_known_date() {
        assert_eq!(date_stamp_at(1_705_314_600_000), "2024-01-15");
    }

    #[test]
    fn parse_rejects_empty_text() {
        let r = parse_design_file("   ");
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("That file is empty."));
    }

    #[test]
    fn parse_rejects_bad_json() {
        let r = parse_design_file("{not json");
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("That file is not valid JSON. It may have been edited or only partly saved.")
        );
    }

    #[test]
    fn parse_rejects_wrong_app_marker() {
        let r = parse_design_file(r#"{"app":"other","version":1,"topology":{}}"#);
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("That file does not hold a Breakscale design.")
        );
    }

    #[test]
    fn parse_rejects_future_version() {
        let r = parse_design_file(
            r#"{"app":"breakscale","version":99,"topology":{"nodes":[],"edges":[]}}"#,
        );
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("That design was saved by a newer version of Breakscale. Reload the page and try again.")
        );
    }

    #[test]
    fn parse_rejects_dangling_edge() {
        let text = r#"{
            "app": "breakscale",
            "version": 1,
            "topology": {
                "nodes": [{
                    "id": "n1", "kind": "client", "label": "Client", "x": 0, "y": 0,
                    "config": {
                        "capacity": 1, "serviceMs": 1, "serviceCv": 0, "queueLimit": 1,
                        "hitRate": 0, "errorRate": 0, "timeoutMs": 0, "retries": 0, "rps": 1,
                        "replicaCount": 1, "replicationLagMs": 0, "readFraction": 1,
                        "shardCount": 1, "shardCapacity": 1, "hotKeyFraction": 0
                    }
                }],
                "edges": [{"id": "e1", "from": "n1", "to": "does-not-exist", "weight": 1}]
            }
        }"#;
        let r = parse_design_file(text);
        assert!(!r.ok);
        assert_eq!(
            r.error.as_deref(),
            Some("That design is damaged: a component or a connection is missing something the simulator needs.")
        );
    }

    #[test]
    fn read_missing_file_reports_could_not_be_read() {
        let mut path = std::env::temp_dir();
        path.push("breakscale_design_file_test_definitely_missing_dir");
        path.push("nope.breakscale");
        let r = read_design_file(&path);
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("That file could not be read."));
    }
}
