//! Faithful port of `src/content/glossary.ts`'s data (`GLOSSARY`).
//!
//! Plain-language explanations for every term the interface puts in front
//! of a student. Ported as data, not code (MIGRATION_PLAN.md #4/#5): it is
//! a 990-line literal array with no computed values, so it is embedded as
//! JSON (`data/glossary.json`) via `include_str!` and parsed once per call,
//! the same mechanism `sim::presets` uses for the example topologies.
//!
//! House style for an entry, carried over from the TS source (learned from
//! watching people read this app):
//!   - `short` is what fits in a tooltip header and on a narrow screen. One
//!     line, no jargon, no trailing period.
//!   - `why` is the part that actually teaches. A metric a student cannot
//!     act on is trivia, so say what it tells them and what to do about
//!     it.
//!   - Never define a term using another undefined term. Where one entry
//!     leans on another, it is named in `see` so the reader can follow the
//!     thread.
//!   - Address the reader as "you". Prefer "the slowest 1% of requests"
//!     over "the 99th percentile of the latency distribution".
//!
//! The task description that produced this file suggested `pub fn load()
//! -> Vec<serde_json::Value>` as a fallback for an unstable shape; the
//! actual TS `GlossaryEntry` interface (`id`, `term`, `short`, `why`,
//! `category`, `see?`, `aliases?`) is simple and has held stable across
//! all 100 entries, so this port uses a proper typed struct instead --
//! `serde_json::Value` would silently accept a malformed entry that a
//! typed `Deserialize` catches at parse time, and this file's own
//! `EXTRA_DEFAULTS`-adjacent lesson (see `sim::presets`) is that a caught
//! error beats a silent one.

use serde::{Deserialize, Serialize};

/// Mirrors the TS `GlossaryCategory` string union exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlossaryCategory {
    Latency,
    Throughput,
    Failure,
    Capacity,
    Component,
    Unit,
}

/// One glossary entry. Field-for-field port of the TS `GlossaryEntry`
/// interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    /// Stable lookup key. Also the anchor id in the glossary panel.
    pub id: String,
    /// How the term appears in the interface.
    pub term: String,
    /// One-line definition. Tooltip header; no trailing period.
    pub short: String,
    /// Why a student should care. Two or three sentences at most.
    pub why: String,
    pub category: GlossaryCategory,
    /// Ids of related entries, for "see also" links.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub see: Option<Vec<String>>,
    /// Extra words that should match this entry when searching.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
}

static GLOSSARY_JSON: &str = include_str!("../../data/glossary.json");

/// Every glossary entry, in the same order as the TS `GLOSSARY` array.
///
/// A parse failure here means the checked-in JSON is malformed -- a
/// build-time data bug, not a runtime condition a caller can recover from
/// -- so it panics with the error rather than returning an empty list,
/// which would look like "no glossary" instead of "broken glossary".
pub fn load() -> Vec<GlossaryEntry> {
    match serde_json::from_str::<Vec<GlossaryEntry>>(GLOSSARY_JSON) {
        Ok(entries) => entries,
        Err(err) => panic!("data/glossary.json is malformed: {err}"),
    }
}
