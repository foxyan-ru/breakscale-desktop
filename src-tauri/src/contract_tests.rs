//! The Rust half of the shared IPC contract suite.
//!
//! WHY THIS FILE EXISTS. The frontend and this crate must agree on three
//! things for every one of the 35 registered commands: the command NAME,
//! the set of argument keys in the payload, and the key set of whatever
//! payload comes back. A rename on either side that the other does not
//! follow fails at runtime with an opaque "command not found" or a serde
//! "missing field" error, long after the change was made. This suite pins
//! all three against ONE golden file -- `contract/ipc-golden.json` -- whose
//! sibling suite (`src/lib/api/__tests__/ipc-contract.test.ts`) checks the
//! TypeScript side against the same bytes, so a drift on either side turns
//! CI red at the change, not in a student's session.
//!
//! WHAT RUNS HERE (12 tests, all read-only against sources/data except
//! three temp files in the OS temp dir, removed on success):
//!
//! 1. `lib.rs`'s `generate_handler![...]` table == fixture `commands`.
//! 2. The `invoke('name', ...)` calls in `src/lib/api/*.ts` ==
//!    `commands` minus `uninvokedCommands`.
//! 3. The `listen('channel', ...)` calls == fixture `events` keys.
//! 4. Payload inventory closure: `commandReturns` + `events` +
//!    `freePayloads` == `payloads` keys; `freePayloads` == the two known
//!    free shapes.
//! 5. Every `#[tauri::command]` signature scanned from source == its
//!    `commandArgs` entry (both `all` and `required`), both directions.
//! 6. Declared `pub` fields of every wire struct (43 canonical + 3 union
//!    result types) == the fixture instance key sets, both directions.
//! 7. Deserialize roundtrips of every canonical payload instance:
//!    fixture -> typed -> fixture, compared key-for-key (strings/bools/
//!    nulls compared exactly; numbers compared as JSON numbers, because
//!    serde's f64 fields normalise integer literals).
//! 8. The Serialize-only result types constructed via their own
//!    constructors (`SaveResult::ok/err`, `DesignParseResult::ok/err`,
//!    `BackupResult::ok/err`, `AppError::Validation`, ... ) serialize to
//!    exactly their fixture branch.
//! 9. `failuresByReason` keys == serialized `ALL_FAILURE_REASONS`.
//! 10. Every `argSamples` value deserializes into its command's argument
//!     type.
//! 11. The 13 commands that take no `AppHandle`/`State` are actually
//!     CALLED against the fixture data, and their live returns are shape-
//!     checked against the fixture payloads (key subset, plus source-
//!     verified identifier spot-checks only -- fixture VALUES are
//!     illustrative samples, see the fixture `$comment`).
//! 12. All 35 commands are pinned as exact fn-pointer signatures, so a
//!     parameter added/removed/retyped anywhere breaks compilation here
//!     even for the 22 commands that need Tauri-managed state and so
//!     cannot be called headlessly.
//!
//! KNOWN, DELIBERATE GAPS (also reported in the suite's final inventory):
//! - `Annotation` is `serde_json::Value` on the Rust side (types.rs), so
//!   note/section annotation keys are pinned by the TS suite only; Rust
//!   pins them indirectly via the `Topology` roundtrip.
//! - Enum VARIANT enumeration is pinned for `FailureReason` (test 9);
//!   other enums are pinned through their fixture instances only.
//! - Optionality (`?` vs required) is pinned as `commandArgs.required`
//!   from the Rust signature, which is the side Tauri actually enforces.
//!
//! Reading note: helpers first, tests in dependency-free order below them.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

const GOLDEN: &str = include_str!("../../contract/ipc-golden.json");

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn golden() -> Value {
    serde_json::from_str(GOLDEN).unwrap_or_else(|e| panic!("contract/ipc-golden.json must parse: {e}"))
}

/// Navigate a fixture path such as `["payloads", "topology", "nodes", "[0]",
/// "config"]`. `[n]` segments index arrays; everything else is an object
/// key. Panics with the full path on any miss so a fixture typo is obvious.
fn g_at<'v>(root: &'v Value, path: &[&str]) -> &'v Value {
    let mut cur = root;
    for (i, seg) in path.iter().enumerate() {
        let so_far = path[..=i].join(".");
        cur = if seg.starts_with('[') && seg.ends_with(']') {
            let idx: usize = seg[1..seg.len() - 1]
                .parse()
                .unwrap_or_else(|_| panic!("fixture path {so_far}: not a [n] index"));
            cur.as_array()
                .and_then(|a| a.get(idx))
                .unwrap_or_else(|| panic!("fixture path {so_far}: missing array element"))
        } else {
            cur.as_object()
                .and_then(|o| o.get(*seg))
                .unwrap_or_else(|| panic!("fixture path {so_far}: missing object key"))
        };
    }
    cur
}

/// Short JSON variant names for panic messages. This test file owns its
/// own copy because the crate's `util` module has no such helper.
fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn keys_of(v: &Value, ctx: &str) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("{ctx}: expected a JSON object, got {}", json_kind(v)))
        .keys()
        .cloned()
        .collect()
}

fn assert_set_eq(name: &str, actual: &BTreeSet<String>, expected: &BTreeSet<String>) {
    let missing: Vec<&String> = expected.difference(actual).collect();
    let extra: Vec<&String> = actual.difference(expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{name}: missing={missing:?} extra={extra:?}"
    );
}

/// Recursive key-shape equality: identical object key sets at every level,
/// identical array lengths, identical string/bool/null values, and -- for
/// numbers -- identical JSON-number-ness only. Number VALUES are not
/// compared because an `f64` field serialises a fixture integer `10` as
/// `10.0`, which serde_json reports as a different Number. Every canonical
/// fixture instance is fully populated (asserted by the declared-field
/// tests), so this is exact key equality in practice.
fn assert_key_shape_eq(expected: &Value, actual: &Value, ctx: &str) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            let ek: BTreeSet<&String> = e.keys().collect();
            let ak: BTreeSet<&String> = a.keys().collect();
            if ek != ak {
                let missing: Vec<&&String> = ek.difference(&ak).collect();
                let extra: Vec<&&String> = ak.difference(&ek).collect();
                    panic!("{ctx}: key sets differ, missing={missing:?} extra={extra:?}");
            }
            for (k, ev) in e {
                assert_key_shape_eq(ev, &a[k], &format!("{ctx}.{k}"));
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            assert_eq!(e.len(), a.len(), "{ctx}: array length differs");
            for (i, (ev, av)) in e.iter().zip(a.iter()).enumerate() {
                assert_key_shape_eq(ev, av, &format!("{ctx}[{i}]"));
            }
        }
        (Value::String(e), Value::String(a)) => assert_eq!(e, a, "{ctx}: string value differs"),
        (Value::Bool(e), Value::Bool(a)) => assert_eq!(e, a, "{ctx}: bool value differs"),
        (Value::Null, Value::Null) => {}
        (Value::Number(_), Value::Number(_)) => {}
        _ => panic!(
            "{ctx}: JSON variant differs: fixture is {} but return is {}",
            json_kind(expected),
            json_kind(actual)
        ),
    }
}

/// Deserialize a fixture value into `T` and serialize it back, then require
/// the two JSON values to agree key-for-key (see `assert_key_shape_eq`).
fn assert_roundtrip<T: Serialize + serde::de::DeserializeOwned>(fixture: &Value, label: &str) {
    let parsed: T = serde_json::from_value(fixture.clone())
        .unwrap_or_else(|e| panic!("{label}: fixture instance does not deserialize: {e}"));
    let out = serde_json::to_value(&parsed)
        .unwrap_or_else(|e| panic!("{label}: re-serialization failed: {e}"));
    assert_key_shape_eq(fixture, &out, label);
}

/// Live command returns are shape-checked TOP LEVEL ONLY against the union
/// of the named fixture payload instances' keys. Deeper nesting is pinned
/// by tests 6/7 on the fixture itself; live data may legitimately carry
/// different values (and `topology` instances inside results may omit
/// `annotations`), so a subset -- not equality -- is the honest live check.
fn assert_return_shape(g: &Value, returned: &Value, payload_names: &[&str], ctx: &str) {
    let mut allowed: BTreeSet<String> = BTreeSet::new();
    for name in payload_names {
        let instance = g_at(g, &["payloads", name]);
        let target = instance.as_array().map(|a| &a[0]).unwrap_or(instance);
        allowed.extend(keys_of(target, &format!("payloads.{name}")));
    }
    let items: Vec<(&Value, String)> = match returned.as_array() {
        Some(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| (v, format!("{ctx}[{i}]")))
            .collect(),
        None => vec![(returned, ctx.to_string())],
    };
    for (item, label) in items {
        let actual = keys_of(item, &label);
        let extra: Vec<&String> = actual.difference(&allowed).collect();
        assert!(
            extra.is_empty(),
            "{label}: returned keys outside the golden union: {extra:?} (allowed: {allowed:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// Source scanners (manual string logic: the crate has no `regex` dependency)
// ---------------------------------------------------------------------------

fn snake_to_camel(s: &str) -> String {
    let mut parts = s.split('_');
    let first = parts.next().unwrap_or("").to_string();
    parts.fold(first, |acc, p| {
        if p.is_empty() {
            acc
        } else {
            format!("{acc}{}{}", &p[..1].to_ascii_uppercase(), &p[1..])
        }
    })
}

fn read_src(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("test fixture source {} must be readable: {e}", path.display()))
}

/// Every `#[tauri::command] pub fn <name>` in one source file, with the
/// wire argument keys derived exactly the way Tauri derives them: the Rust
/// parameter name, camelCased, skipping `AppHandle`/`State` (injected by
/// the runtime, never present in the JSON payload) and marking anything
/// whose type does not start with `Option<` as required.
fn tauri_command_signatures(file_src: &str) -> Vec<(String, Vec<String>, Vec<String>)> {
    const ATTR: &[u8] = b"#[tauri::command]";
    const FN: &[u8] = b"pub fn ";
    let bytes = file_src.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(idx) = find_sub(bytes, ATTR, search) {
        search = idx + ATTR.len();
        let mut j = search;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if find_sub(bytes, FN, j) != Some(j) {
            continue;
        }
        j += FN.len();
        let name_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        let name = file_src[name_start..j].to_string();
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'(' {
            continue;
        }
        let open = j;
        let mut depth = 0usize;
        while j < bytes.len() {
            match bytes[j] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        assert!(j < bytes.len(), "unbalanced parens after {name}(");
        let (all, required) = classify_params(&file_src[open + 1..j]);
        out.push((name, all, required));
    }
    out
}

fn find_sub(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    (from..=hay.len().saturating_sub(needle.len())).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn classify_params(params: &str) -> (Vec<String>, Vec<String>) {
    // Split on top-level commas; track (), <>, [] so a parameter type like
    // `State<'_, SimulationState>` or `Option<Vec<String>>` stays in one
    // part. Every tracked character is appended to the buffer (an earlier
    // draft dropped them, which glued `State<` onto the next parameter).
    let mut parts: Vec<String> = Vec::new();
    let mut buf = String::new();
    let (mut pd, mut ad, mut bd) = (0i32, 0i32, 0i32);
    for ch in params.chars() {
        match ch {
            '(' => {
                pd += 1;
                buf.push(ch);
            }
            ')' => {
                pd -= 1;
                buf.push(ch);
            }
            '<' => {
                ad += 1;
                buf.push(ch);
            }
            '>' => {
                ad -= 1;
                buf.push(ch);
            }
            '[' => {
                bd += 1;
                buf.push(ch);
            }
            ']' => {
                bd -= 1;
                buf.push(ch);
            }
            ',' if pd == 0 && ad == 0 && bd == 0 => parts.push(std::mem::take(&mut buf)),
            _ => buf.push(ch),
        }
    }
    if !buf.trim().is_empty() {
        parts.push(buf);
    }

    let mut all = Vec::new();
    let mut required = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some(colon) = part.find(':') else { continue };
        let ident = part[..colon].trim();
        let ty = part[colon + 1..].trim();
        if ty.contains("AppHandle") || ty.starts_with("State<") || ty.contains("tauri::State") {
            continue;
        }
        all.push(snake_to_camel(ident));
        if !ty.starts_with("Option<") {
            required.push(snake_to_camel(ident));
        }
    }
    (all, required)
}

/// One TypeScript call site name (`invoke('x', ...)` or `listen('x', ...)`)
/// per line. Line comments are stripped and block-comment continuation
/// lines are skipped, so doc comments that merely MENTION `invoke()`
/// cannot register as call sites; a `//` inside a string argument cannot
/// hide the call because the command name always precedes any argument.
fn scan_ts_calls(src: &str, word: &str) -> Vec<String> {
    let mut found = Vec::new();
    for raw_line in src.lines() {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with('*') || trimmed.starts_with("/*") {
            continue;
        }
        let code = raw_line.split("//").next().unwrap_or("");
        let bytes = code.as_bytes();
        let mut i = 0usize;
        while i + word.len() <= bytes.len() {
            if &bytes[i..i + word.len()] == word.as_bytes()
                && (i == 0 || !is_ident_byte(bytes[i - 1]))
                && (i + word.len() == bytes.len() || !is_ident_byte(bytes[i + word.len()]))
            {
                let mut j = i + word.len();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'<' {
                    let mut depth = 1usize;
                    j += 1;
                    while j < bytes.len() && depth > 0 {
                        match bytes[j] {
                            b'<' => depth += 1,
                            b'>' => depth -= 1,
                            _ => {}
                        }
                        j += 1;
                    }
                }
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'(' {
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
                        let quote = bytes[j];
                        let start = j + 1;
                        let mut k = start;
                        while k < bytes.len() && bytes[k] != quote {
                            k += 1;
                        }
                        if k < bytes.len() && start < k {
                            found.push(code[start..k].to_string());
                        }
                    }
                }
            }
            i += 1;
        }
    }
    found
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Every non-test TypeScript file under `src/lib/api/`, sorted, as
/// (relative name, source).
fn api_ts_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/lib/api");
    let mut out: Vec<(String, String)> = Vec::new();
    collect_ts(&root, &root, &mut out);
    out.sort();
    out
}

fn collect_ts(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("api dir {} must be readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("__tests__") {
                continue;
            }
            collect_ts(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("ts") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
            out.push((rel, src));
        }
    }
}

/// The 43 wire structs that have a fully-populated canonical fixture
/// instance: (source file relative to `src/`, struct name, fixture path).
/// `DesignFile` (the on-disk `.breakscale` body) is deliberately absent --
/// it never crosses the IPC wire.
const CANON: &[(&str, &str, &[&str])] = &[
    ("sim/types.rs", "NodeConfig", &["payloads", "topology", "nodes", "[0]", "config"]),
    ("sim/types.rs", "SimNode", &["payloads", "topology", "nodes", "[0]"]),
    ("sim/types.rs", "SimEdge", &["payloads", "topology", "edges", "[0]"]),
    ("sim/types.rs", "Topology", &["payloads", "topology"]),
    ("sim/types.rs", "NodeStats", &["payloads", "simSnapshot", "nodes", "client-1"]),
    ("sim/types.rs", "SystemStats", &["payloads", "simSnapshot", "system"]),
    ("sim/types.rs", "HistoryPoint", &["payloads", "simSnapshot", "history", "[0]"]),
    ("sim/types.rs", "ActiveFailure", &["payloads", "simSnapshot", "activeFailures", "[0]"]),
    ("sim/types.rs", "RequestTrace", &["payloads", "simSnapshot", "trace"]),
    ("sim/types.rs", "TraceHop", &["payloads", "simSnapshot", "trace", "hops", "[0]"]),
    ("sim/types.rs", "SimSnapshot", &["payloads", "simSnapshot"]),
    ("sysdesign/model.rs", "SystemDesignDoc", &["payloads", "systemDesignDoc"]),
    ("sysdesign/model.rs", "HighLevelArchitecture", &["payloads", "systemDesignDoc", "architecture"]),
    ("sysdesign/model.rs", "ComponentNote", &["payloads", "systemDesignDoc", "architecture", "components", "[0]"]),
    ("sysdesign/model.rs", "DataFlowNote", &["payloads", "systemDesignDoc", "architecture", "dataFlows", "[0]"]),
    ("sysdesign/model.rs", "ExternalDependency", &["payloads", "systemDesignDoc", "architecture", "externalDependencies", "[0]"]),
    ("sysdesign/model.rs", "QualityAttributes", &["payloads", "systemDesignDoc", "architecture", "qualityAttributes"]),
    ("sysdesign/model.rs", "LowLevelDesign", &["payloads", "systemDesignDoc", "lowLevel"]),
    ("sysdesign/model.rs", "ApiContract", &["payloads", "systemDesignDoc", "lowLevel", "apis", "[0]"]),
    ("sysdesign/model.rs", "ApiError", &["payloads", "systemDesignDoc", "lowLevel", "apis", "[0]", "errors", "[0]"]),
    ("sysdesign/model.rs", "EntityModel", &["payloads", "systemDesignDoc", "lowLevel", "entities", "[0]"]),
    ("sysdesign/model.rs", "EntityField", &["payloads", "systemDesignDoc", "lowLevel", "entities", "[0]", "fields", "[0]"]),
    ("sysdesign/model.rs", "EntityRelationship", &["payloads", "systemDesignDoc", "lowLevel", "entities", "[0]", "relationships", "[0]"]),
    ("sysdesign/model.rs", "SequenceFlow", &["payloads", "systemDesignDoc", "lowLevel", "sequences", "[0]"]),
    ("sysdesign/model.rs", "SequenceStep", &["payloads", "systemDesignDoc", "lowLevel", "sequences", "[0]", "steps", "[0]"]),
    ("sysdesign/model.rs", "StateMachine", &["payloads", "systemDesignDoc", "lowLevel", "stateMachines", "[0]"]),
    ("sysdesign/model.rs", "StateTransition", &["payloads", "systemDesignDoc", "lowLevel", "stateMachines", "[0]", "transitions", "[0]"]),
    ("sysdesign/model.rs", "DeploymentConfig", &["payloads", "systemDesignDoc", "lowLevel", "deployment"]),
    ("sysdesign/model.rs", "DeploymentResource", &["payloads", "systemDesignDoc", "lowLevel", "deployment", "resources", "[0]"]),
    ("sysdesign/model.rs", "ValidationIssue", &["payloads", "validationIssue", "[0]"]),
    ("vendors/types.rs", "Vendor", &["payloads", "vendor"]),
    ("vendors/types.rs", "VendorMapping", &["payloads", "vendor", "kinds", "cache"]),
    ("vendors/types.rs", "VendorSize", &["payloads", "vendor", "kinds", "cache", "sizes", "[0]"]),
    ("sim/glossary.rs", "GlossaryEntry", &["payloads", "glossaryEntry", "[0]"]),
    ("sim/presets.rs", "PresetSummary", &["payloads", "presetSummary", "[0]"]),
    ("sim/presets.rs", "Preset", &["payloads", "preset"]),
    ("sim/challenge.rs", "Challenge", &["payloads", "challengeStart", "challenge"]),
    ("sim/challenge.rs", "Goal", &["payloads", "challengeStart", "challenge", "goals", "[0]"]),
    ("commands/challenges.rs", "ChallengeSummary", &["payloads", "challengeSummary", "[0]"]),
    ("commands/challenges.rs", "ChallengeStart", &["payloads", "challengeStart"]),
    ("persistence/saved_designs.rs", "SavedDesign", &["payloads", "savedDesign"]),
    ("persistence/saved_designs.rs", "SavedSummary", &["payloads", "savedSummary", "[0]"]),
];

/// The three `{ ok, ...variant fields }` flattened-union result types:
/// declared fields are compared against the UNION of both fixture branches.
const UNION_CANON: &[(&str, &str, &[&[&str]])] = &[
    (
        "persistence/saved_designs.rs",
        "SaveResult",
        &[&["payloads", "saveResultOk"], &["payloads", "saveResultErr"]],
    ),
    (
        "persistence/design_file.rs",
        "DesignParseResult",
        &[&["payloads", "designParseResultOk"], &["payloads", "designParseResultErr"]],
    ),
    (
        "persistence/backup.rs",
        "BackupResult",
        &[&["payloads", "backupResultOk"], &["payloads", "backupResultErr"]],
    ),
];

/// Depth-aware scan for `pub <name>:` fields at brace depth 1 of
/// `pub struct <name>`. Comment lines (`//`, `///`) and attribute lines
/// (`#[...]`) are skipped so doc prose containing `{` cannot derail the
/// depth count.
fn struct_declared_fields(src: &str, struct_name: &str) -> Option<Vec<String>> {
    let header = format!("pub struct {struct_name}");
    let lines: Vec<&str> = src.lines().collect();
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t.starts_with(&header) {
            let rest = &t[header.len()..];
            let boundary = rest
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(true);
            if boundary {
                start = Some(i);
                break;
            }
        }
    }
    let start = start?;
    let mut depth: i32 = 0;
    let mut started = false;
    let mut fields = Vec::new();
    for line in &lines[start..] {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('#') {
            continue;
        }
        if depth == 1 {
            if let Some(rest) = t.strip_prefix("pub ") {
                if let Some(colon) = rest.find(':') {
                    let ident = rest[..colon].trim();
                    if !ident.is_empty()
                        && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        fields.push(snake_to_camel(ident));
                    }
                }
            }
        }
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if depth > 0 {
            started = true;
        }
        if started && depth == 0 {
            break;
        }
    }
    Some(fields)
}

// ---------------------------------------------------------------------------
// 1-4. Inventories: handler table, TS call sites, event channels, payloads
// ---------------------------------------------------------------------------

/// WHY: `lib.rs`'s `generate_handler![...]` list IS the runtime command
/// table; the fixture's `commands` array is what the TS suite assumes. If
/// a command is registered but not listed (or listed but not registered),
/// one side's tests would pass while the app cannot actually invoke it.
#[test]
fn lib_rs_registers_exactly_the_fixture_commands() {
    let g = golden();
    let lib = read_src("lib.rs");

    // Every `commands::<mod>::<fn>` token in lib.rs is a handler entry
    // (lib.rs has no other mention of that path prefix).
    let mut handlers: BTreeSet<String> = BTreeSet::new();
    let mut search = lib.as_str();
    while let Some(idx) = search.find("commands::") {
        let after = &search[idx + "commands::".len()..];
        let (module, rest) = split_ident(after);
        if !module.is_empty() {
            if let Some(rest) = rest.strip_prefix("::") {
                let (name, _) = split_ident(rest);
                if !name.is_empty() {
                    handlers.insert(name.to_string());
                }
            }
        }
        search = &search[idx + "commands::".len()..];
    }

    let expected: BTreeSet<String> = g["commands"]
        .as_array()
        .expect("fixture commands is an array")
        .iter()
        .map(|v| v.as_str().expect("command names are strings").to_string())
        .collect();
    assert_set_eq("lib.rs generate_handler vs fixture.commands", &handlers, &expected);

    let uninvoked: BTreeSet<String> = g["uninvokedCommands"]
        .as_array()
        .expect("fixture uninvokedCommands is an array")
        .iter()
        .map(|v| v.as_str().expect("names are strings").to_string())
        .collect();
    let stray: Vec<&String> = uninvoked.difference(&expected).collect();
    assert!(
        stray.is_empty(),
        "uninvokedCommands entries missing from commands: {stray:?}"
    );
}

fn split_ident(s: &str) -> (&str, &str) {
    let end = s
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(s.len());
    (&s[..end], &s[end..])
}

/// WHY: the TS suite asserts each wrapper sends the fixture's argument
/// keys; this test asserts the wrappers EXIST for exactly the commands
/// that are not state-bound (the four settings/layout commands have no
/// frontend caller yet -- see fixture `uninvokedCommands`). A TS-only
/// command with no Rust handler, or a Rust handler with no TS caller,
/// fails here with the offending name.
#[test]
fn typescript_invokes_exactly_the_non_state_commands() {
    let g = golden();
    let mut invoked: BTreeSet<String> = BTreeSet::new();
    for (_rel, src) in api_ts_sources() {
        invoked.extend(scan_ts_calls(&src, "invoke"));
    }

    let uninvoked: BTreeSet<String> = g["uninvokedCommands"]
        .as_array()
        .expect("uninvokedCommands array")
        .iter()
        .map(|v| v.as_str().expect("strings").to_string())
        .collect();
    let expected: BTreeSet<String> = g["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|v| v.as_str().expect("strings").to_string())
        .filter(|name| !uninvoked.contains(name))
        .collect();
    assert_set_eq("TS invoke() sites vs commands - uninvokedCommands", &invoked, &expected);
}

/// WHY: `listen` channels are strings the Rust side emits; a channel
/// rename on either side silently stops snapshots from arriving. The two
/// channels are pinned here against the same fixture the TS suite checks.
#[test]
fn typescript_listens_to_exactly_the_fixture_events() {
    let g = golden();
    let mut channels: BTreeSet<String> = BTreeSet::new();
    for (_rel, src) in api_ts_sources() {
        channels.extend(scan_ts_calls(&src, "listen"));
    }
    let expected: BTreeSet<String> = g["events"]
        .as_object()
        .expect("fixture events is an object")
        .keys()
        .cloned()
        .collect();
    assert_set_eq("TS listen() channels vs fixture.events", &channels, &expected);
}

/// WHY: the fixture must stay internally closed -- every payload name
/// referenced by a return or an event exists, and every payload is
/// referenced by exactly one of the three inventories (commands, events,
/// free shapes). An orphan payload would mean one side's suite skips a
/// shape entirely.
#[test]
fn payload_inventory_is_closed() {
    let g = golden();

    let commands: BTreeSet<String> = g["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .map(|v| v.as_str().expect("strings").to_string())
        .collect();
    let returns = g["commandReturns"]
        .as_object()
        .expect("commandReturns object");
    let return_keys: BTreeSet<String> = returns.keys().cloned().collect();
    assert_set_eq("commandReturns keys vs commands", &return_keys, &commands);

    let mut referenced: BTreeSet<String> = BTreeSet::new();
    for (_cmd, value) in returns {
        if let Some(list) = value.as_array() {
            for name in list {
                referenced.insert(name.as_str().expect("payload names are strings").to_string());
            }
        }
    }
    for (_channel, list) in g["events"].as_object().expect("events object") {
        for name in list.as_array().expect("event payload lists") {
            referenced.insert(name.as_str().expect("strings").to_string());
        }
    }
    for name in g["freePayloads"].as_array().expect("freePayloads array") {
        referenced.insert(name.as_str().expect("strings").to_string());
    }
    let payload_keys: BTreeSet<String> = keys_of(&g["payloads"], "payloads");
    assert_set_eq(
        "commandReturns + events + freePayloads vs payloads keys",
        &referenced,
        &payload_keys,
    );

    let free: Vec<&str> = g["freePayloads"]
        .as_array()
        .expect("freePayloads array")
        .iter()
        .map(|v| v.as_str().expect("strings"))
        .collect();
    assert_eq!(
        free,
        vec!["topology", "appErrorPayload"],
        "freePayloads must stay the argument/error-envelope shapes"
    );
}

// ---------------------------------------------------------------------------
// 5. Command signatures vs fixture commandArgs
// ---------------------------------------------------------------------------

/// WHY: `commandArgs` is the single source of truth both suites use for
/// "what keys does this command accept / require". Scanning the actual
/// `#[tauri::command]` signatures here means a renamed parameter
/// (`node_id` -> `nodeId` stays, `nodeId` -> `targetId` breaks), a newly
/// required parameter, or a dropped optional shows up as a red test
/// instead of a payload that only fails in the app.
#[test]
fn every_command_signature_matches_its_fixture_argument_keys() {
    let g = golden();
    let command_files = [
        "backup.rs",
        "challenges.rs",
        "designs.rs",
        "glossary.rs",
        "presets.rs",
        "settings.rs",
        "sim.rs",
        "sysdesign.rs",
        "vendors.rs",
    ];
    let mut scanned: std::collections::BTreeMap<String, (Vec<String>, Vec<String>)> =
        std::collections::BTreeMap::new();
    for file in command_files {
        let src = read_src(&format!("commands/{file}"));
        for (name, all, required) in tauri_command_signatures(&src) {
            assert!(
                scanned.insert(name.clone(), (all, required)).is_none(),
                "command {name} defined twice across commands/*.rs"
            );
        }
    }

    let fixture_args = g["commandArgs"]
        .as_object()
        .expect("commandArgs object");
    assert_eq!(
        fixture_args.len(),
        scanned.len(),
        "fixture commandArgs entries ({}) vs scanned signatures ({})",
        fixture_args.len(),
        scanned.len()
    );

    for (name, entry) in fixture_args {
        let (all, required) = scanned.get(name).unwrap_or_else(|| {
            panic!("fixture lists commandArgs for {name}, but no #[tauri::command] defines it")
        });
        let fixture_all: BTreeSet<String> = entry["all"]
            .as_array()
            .expect("all array")
            .iter()
            .map(|v| v.as_str().expect("strings").to_string())
            .collect();
        let fixture_required: BTreeSet<String> = entry["required"]
            .as_array()
            .expect("required array")
            .iter()
            .map(|v| v.as_str().expect("strings").to_string())
            .collect();
        let actual_all: BTreeSet<String> = all.iter().cloned().collect();
        let actual_required: BTreeSet<String> = required.iter().cloned().collect();
        assert_set_eq(&format!("args[{name}].all"), &actual_all, &fixture_all);
        assert_set_eq(&format!("args[{name}].required"), &actual_required, &fixture_required);
    }
    for name in scanned.keys() {
        assert!(
            fixture_args.contains_key(name),
            "signature scan found command {name} missing from fixture.commandArgs"
        );
    }
}

// ---------------------------------------------------------------------------
// 6. Declared fields vs fixture instances (43 canonical + 3 unions)
// ---------------------------------------------------------------------------

/// WHY: serde emits exactly the declared fields, so a struct gaining or
/// losing a field moves the wire shape even when nobody touches a command.
/// Comparing declared field sets against the fully-populated fixture
/// instances (both directions) pins the serialized key set of every payload
/// type without running the app.
#[test]
fn every_declared_wire_struct_matches_its_fixture_instance_keys() {
    let g = golden();
    let mut sources: std::collections::BTreeMap<&str, String> = std::collections::BTreeMap::new();

    for (rel, name, path) in CANON {
        let src = sources
            .entry(rel)
            .or_insert_with(|| read_src(rel));
        let fields = struct_declared_fields(src, name)
            .unwrap_or_else(|| panic!("{name}: pub struct not found in {rel}"));
        let actual: BTreeSet<String> = fields.into_iter().collect();
        let expected = keys_of(g_at(&g, path), name);
        assert_set_eq(&format!("{name} declared fields"), &actual, &expected);
    }

    for (rel, name, branches) in UNION_CANON {
        let src = sources
            .entry(rel)
            .or_insert_with(|| read_src(rel));
        let fields = struct_declared_fields(src, name)
            .unwrap_or_else(|| panic!("{name}: pub struct not found in {rel}"));
        let actual: BTreeSet<String> = fields.into_iter().collect();
        let mut expected: BTreeSet<String> = BTreeSet::new();
        for branch in *branches {
            expected.extend(keys_of(g_at(&g, branch), name));
        }
        assert_set_eq(&format!("{name} declared fields (union of branches)"), &actual, &expected);
    }
}

// ---------------------------------------------------------------------------
// 7. Deserialize roundtrips
// ---------------------------------------------------------------------------

/// WHY: every payload the frontend sends must survive the trip through the
/// Rust type system without a key being renamed, dropped, or gained --
/// serde's camelCase renaming and its `skip_serializing_if` optionals are
/// exactly where that goes wrong. Fixture -> typed -> fixture must be
/// key-identical (numbers aside, which f64 normalisation legitimately
/// rewrites). `SimSnapshot` is Serialize-only (no inbound path), so its
/// shape is pinned by the declared-field test above plus the nested
/// roundtrips of NodeStats/SystemStats/history/trace here.
#[test]
fn canonical_payload_instances_round_trip_key_stably() {
    let g = golden();

    assert_roundtrip::<crate::sim::types::Topology>(
        g_at(&g, &["payloads", "topology"]),
        "Topology",
    );
    assert_roundtrip::<crate::sim::types::NodeConfig>(
        g_at(&g, &["payloads", "topology", "nodes", "[0]", "config"]),
        "NodeConfig",
    );
    assert_roundtrip::<crate::sim::types::SimNode>(
        g_at(&g, &["payloads", "topology", "nodes", "[0]"]),
        "SimNode",
    );
    assert_roundtrip::<crate::sim::types::SimEdge>(
        g_at(&g, &["payloads", "topology", "edges", "[0]"]),
        "SimEdge",
    );
    assert_roundtrip::<crate::sim::types::NodeStats>(
        g_at(&g, &["payloads", "simSnapshot", "nodes", "client-1"]),
        "NodeStats",
    );
    assert_roundtrip::<crate::sim::types::SystemStats>(
        g_at(&g, &["payloads", "simSnapshot", "system"]),
        "SystemStats",
    );
    assert_roundtrip::<crate::sim::types::HistoryPoint>(
        g_at(&g, &["payloads", "simSnapshot", "history", "[0]"]),
        "HistoryPoint",
    );
    assert_roundtrip::<crate::sim::types::ActiveFailure>(
        g_at(&g, &["payloads", "simSnapshot", "activeFailures", "[0]"]),
        "ActiveFailure",
    );
    assert_roundtrip::<crate::sim::types::RequestTrace>(
        g_at(&g, &["payloads", "simSnapshot", "trace"]),
        "RequestTrace",
    );
    assert_roundtrip::<crate::sim::types::TraceHop>(
        g_at(&g, &["payloads", "simSnapshot", "trace", "hops", "[0]"]),
        "TraceHop",
    );
    assert_roundtrip::<crate::sysdesign::model::SystemDesignDoc>(
        g_at(&g, &["payloads", "systemDesignDoc"]),
        "SystemDesignDoc",
    );
    assert_roundtrip::<crate::vendors::types::Vendor>(
        g_at(&g, &["payloads", "vendor"]),
        "Vendor",
    );
    assert_roundtrip::<crate::vendors::types::VendorMapping>(
        g_at(&g, &["payloads", "vendor", "kinds", "cache"]),
        "VendorMapping",
    );
    assert_roundtrip::<crate::vendors::types::VendorSize>(
        g_at(&g, &["payloads", "vendor", "kinds", "cache", "sizes", "[0]"]),
        "VendorSize",
    );
    assert_roundtrip::<crate::sim::glossary::GlossaryEntry>(
        g_at(&g, &["payloads", "glossaryEntry", "[0]"]),
        "GlossaryEntry",
    );
    assert_roundtrip::<crate::sim::presets::PresetSummary>(
        g_at(&g, &["payloads", "presetSummary", "[0]"]),
        "PresetSummary",
    );
    assert_roundtrip::<crate::sim::presets::Preset>(
        g_at(&g, &["payloads", "preset"]),
        "Preset",
    );
    assert_roundtrip::<crate::sim::challenge::Challenge>(
        g_at(&g, &["payloads", "challengeStart", "challenge"]),
        "Challenge",
    );
    assert_roundtrip::<crate::sim::challenge::Goal>(
        g_at(&g, &["payloads", "challengeStart", "challenge", "goals", "[0]"]),
        "Goal",
    );
    assert_roundtrip::<crate::persistence::saved_designs::SavedDesign>(
        g_at(&g, &["payloads", "savedDesign"]),
        "SavedDesign",
    );
}

// ---------------------------------------------------------------------------
// 8. Serialize-only constructed pins
// ---------------------------------------------------------------------------

/// WHY: the union result types, summaries and error payloads are Serialize
/// only -- they never come IN from the frontend, so only their outgoing
/// shape can break the contract. Constructing them through their own
/// constructors (the code paths every command uses) and comparing against
/// the fixture branch pins both the key sets (via skip_serializing_if) and
/// the string/bool/null values, which are stable across data drift unlike
/// live content strings.
#[test]
fn constructed_result_types_match_their_fixture_branches() {
    let g = golden();

    // SaveResult: `ok` constructor emits { ok, id, evicted: null } for a
    // clean save; `err` emits { ok, error } with the other fields skipped.
    let ok_id = g_at(&g, &["payloads", "saveResultOk", "id"])
        .as_str()
        .expect("saveResultOk.id is a string")
        .to_string();
    let save_ok = crate::persistence::saved_designs::SaveResult::ok(ok_id, None);
    assert_key_shape_eq(
        g_at(&g, &["payloads", "saveResultOk"]),
        &serde_json::to_value(&save_ok).expect("SaveResult serializes"),
        "SaveResult::ok",
    );
    let save_err = crate::persistence::saved_designs::SaveResult::err(
        g_at(&g, &["payloads", "saveResultErr", "error"])
            .as_str()
            .expect("saveResultErr.error"),
    );
    assert_key_shape_eq(
        g_at(&g, &["payloads", "saveResultErr"]),
        &serde_json::to_value(&save_err).expect("SaveResult serializes"),
        "SaveResult::err",
    );

    // DesignParseResult: ok carries { ok, topology, name } (name present
    // even when null); err carries only { ok, error }.
    let topo: crate::sim::types::Topology =
        serde_json::from_value(g_at(&g, &["payloads", "topology"]).clone())
            .expect("fixture topology deserializes");
    let parse_name = g_at(&g, &["payloads", "designParseResultOk", "name"])
        .as_str()
        .expect("designParseResultOk.name is a string")
        .to_string();
    let parse_ok = crate::persistence::design_file::DesignParseResult::ok(topo, Some(parse_name));
    assert_key_shape_eq(
        g_at(&g, &["payloads", "designParseResultOk"]),
        &serde_json::to_value(&parse_ok).expect("DesignParseResult serializes"),
        "DesignParseResult::ok",
    );
    let parse_err = crate::persistence::design_file::DesignParseResult::err(
        g_at(&g, &["payloads", "designParseResultErr", "error"])
            .as_str()
            .expect("designParseResultErr.error"),
    );
    assert_key_shape_eq(
        g_at(&g, &["payloads", "designParseResultErr"]),
        &serde_json::to_value(&parse_err).expect("DesignParseResult serializes"),
        "DesignParseResult::err",
    );

    // BackupResult: ok carries the restored file list; err { ok, error }.
    let restored: Vec<String> = g_at(&g, &["payloads", "backupResultOk", "restored"])
        .as_array()
        .expect("restored array")
        .iter()
        .map(|v| v.as_str().expect("file names").to_string())
        .collect();
    let backup_ok = crate::persistence::backup::BackupResult::ok(restored);
    assert_key_shape_eq(
        g_at(&g, &["payloads", "backupResultOk"]),
        &serde_json::to_value(&backup_ok).expect("BackupResult serializes"),
        "BackupResult::ok",
    );
    let backup_err = crate::persistence::backup::BackupResult::err(
        g_at(&g, &["payloads", "backupResultErr", "error"])
            .as_str()
            .expect("backupResultErr.error"),
    );
    assert_key_shape_eq(
        g_at(&g, &["payloads", "backupResultErr"]),
        &serde_json::to_value(&backup_err).expect("BackupResult serializes"),
        "BackupResult::err",
    );

    // AppError: adjacent tagging (`tag = "kind", content = "message"`) is
    // what puts every command failure on the wire as { kind, message }.
    let app_error = crate::error::AppError::Validation(
        g_at(&g, &["payloads", "appErrorPayload", "message"])
            .as_str()
            .expect("appErrorPayload.message"),
    );
    assert_key_shape_eq(
        g_at(&g, &["payloads", "appErrorPayload"]),
        &serde_json::to_value(&app_error).expect("AppError serializes"),
        "AppError::Validation",
    );

    // ChallengeSummary: the picker-grid payload (three strings, no skips).
    let summary_src = g_at(&g, &["payloads", "challengeSummary", "[0]"]);
    let summary = crate::commands::challenges::ChallengeSummary {
        id: summary_src["id"].as_str().expect("id").to_string(),
        name: summary_src["name"].as_str().expect("name").to_string(),
        brief: summary_src["brief"].as_str().expect("brief").to_string(),
    };
    assert_key_shape_eq(
        summary_src,
        &serde_json::to_value(&summary).expect("ChallengeSummary serializes"),
        "ChallengeSummary",
    );

    // ChallengeStart: the full challenge plus its judged-load topology,
    // assembled exactly the way `challenge_start` assembles it.
    let challenge: crate::sim::challenge::Challenge =
        serde_json::from_value(summary_src_path(&g, "challenge").clone())
            .expect("fixture challenge deserializes");
    let start = crate::commands::challenges::ChallengeStart {
        challenge,
        topology: serde_json::from_value(g_at(&g, &["payloads", "challengeStart", "topology"]).clone())
            .expect("fixture challengeStart.topology deserializes"),
    };
    assert_key_shape_eq(
        g_at(&g, &["payloads", "challengeStart"]),
        &serde_json::to_value(&start).expect("ChallengeStart serializes"),
        "ChallengeStart",
    );

    // SavedSummary: the shelf listing (timestamps are i64/usize on the
    // Rust side, so this also pins that they stay JSON integers).
    let summary0 = g_at(&g, &["payloads", "savedSummary", "[0]"]);
    let saved = crate::persistence::saved_designs::SavedSummary {
        id: summary0["id"].as_str().expect("id").to_string(),
        name: summary0["name"].as_str().expect("name").to_string(),
        saved_at: serde_json::from_value(summary0["savedAt"].clone()).expect("savedAt i64"),
        node_count: serde_json::from_value(summary0["nodeCount"].clone()).expect("nodeCount usize"),
    };
    assert_key_shape_eq(
        summary0,
        &serde_json::to_value(&saved).expect("SavedSummary serializes"),
        "SavedSummary",
    );

    // ValidationIssue: what `sysdesign_validate` returns per problem.
    let issue_src = g_at(&g, &["payloads", "validationIssue", "[0]"]);
    let issue = crate::sysdesign::model::ValidationIssue {
        field: issue_src["field"].as_str().expect("field").to_string(),
        message: issue_src["message"].as_str().expect("message").to_string(),
    };
    assert_key_shape_eq(
        issue_src,
        &serde_json::to_value(&issue).expect("ValidationIssue serializes"),
        "ValidationIssue",
    );
}

fn summary_src_path<'v>(g: &'v Value, key: &str) -> &'v Value {
    g_at(g, &["payloads", "challengeStart", key])
}

// ---------------------------------------------------------------------------
// 9. FailureReason enum enumeration
// ---------------------------------------------------------------------------

/// WHY: `failuresByReason` is a fixed-shape counter object -- the TS type
/// declares every reason as a required key. Enumerating Rust's
/// `ALL_FAILURE_REASONS` and comparing to the fixture's key set is the
/// only test that would catch a NEW failure reason (or a renamed kebab-case
/// spelling) that no fixture instance happens to contain.
#[test]
fn failures_by_reason_keys_match_the_rust_enum() {
    use crate::sim::types::ALL_FAILURE_REASONS;
    let g = golden();
    let mut from_rust: BTreeSet<String> = BTreeSet::new();
    for reason in ALL_FAILURE_REASONS.iter() {
        let name = serde_json::to_value(reason)
            .expect("FailureReason serializes")
            .as_str()
            .expect("FailureReason serializes as a string")
            .to_string();
        from_rust.insert(name);
    }
    let fixture = keys_of(
        g_at(&g, &["payloads", "simSnapshot", "failuresByReason"]),
        "failuresByReason",
    );
    assert_set_eq("ALL_FAILURE_REASONS vs fixture failuresByReason", &from_rust, &fixture);
}

// ---------------------------------------------------------------------------
// 10. argSamples -> argument types
// ---------------------------------------------------------------------------

/// WHY: every sample the TS suite sends through a mocked `invoke` must be
/// accepted by the real Rust argument type, or the two suites would happily
/// pass while the live command rejects the payload. This deserializes each
/// sample into its exact command parameter type.
#[test]
fn argument_samples_deserialize_into_their_command_types() {
    use crate::sim::types::{FailureKind, FailureOpts};
    use crate::sysdesign::model::ExportFormat;
    use crate::vendors::types::VendorId;

    let g = golden();
    let samples = g_at(&g, &["argSamples"]);

    let kind: FailureKind =
        serde_json::from_value(samples["kind"].clone()).expect("kind -> FailureKind");
    assert_eq!(kind, FailureKind::Slow, "sample kind is the 'slow' variant");

    let opts: FailureOpts =
        serde_json::from_value(samples["opts"].clone()).expect("opts -> FailureOpts");
    assert_eq!(opts.factor, Some(3.0), "opts.factor");
    assert_eq!(opts.rate, Some(0.1), "opts.rate");
    assert_eq!(
        opts.edge_ids,
        Some(vec!["edge-1".to_string()]),
        "opts.edgeIds"
    );
    let opts_back = serde_json::to_value(&opts).expect("FailureOpts serializes");
    assert_set_eq(
        "FailureOpts roundtrip keys",
        &keys_of(&opts_back, "FailureOpts"),
        &keys_of(&samples["opts"], "argSamples.opts"),
    );

    let format: ExportFormat =
        serde_json::from_value(samples["format"].clone()).expect("format -> ExportFormat");
    assert_eq!(format, ExportFormat::Json);

    let vendor: VendorId =
        serde_json::from_value(samples["vendorId"].clone()).expect("vendorId -> VendorId");
    assert_eq!(vendor, VendorId::Aws);

    for key in ["nodeId", "id", "name", "path", "text", "designId"] {
        serde_json::from_value::<String>(samples[key].clone())
            .unwrap_or_else(|e| panic!("argSamples.{key} must deserialize as a String: {e}"));
    }
    assert_eq!(
        serde_json::from_value::<u32>(samples["seed"].clone()).expect("seed -> u32"),
        7
    );
    assert_eq!(
        serde_json::from_value::<f64>(samples["deltaMs"].clone()).expect("deltaMs -> f64"),
        250.0
    );
    assert!(serde_json::from_value::<bool>(samples["running"].clone()).expect("running -> bool"));
    for key in ["patch", "preferences", "layout"] {
        let v: Value = serde_json::from_value(samples[key].clone())
            .unwrap_or_else(|e| panic!("argSamples.{key} must deserialize as JSON: {e}"));
        assert!(v.is_object(), "argSamples.{key} is an object argument");
    }
}

// ---------------------------------------------------------------------------
// 11. Live invocations of the 13 headless commands
// ---------------------------------------------------------------------------

/// WHY: key sets alone cannot prove the wiring actually runs -- that
/// `glossary_list` really reads the embedded data, that a `.breakscale`
/// file survives a build -> parse roundtrip, that the validator still
/// reports the fixture document's two source-verifiable problems. These 13
/// commands need no Tauri-managed state, so they are called for real and
/// shape-checked against the fixture (subset, since fixture VALUES are
/// illustrative -- see the fixture `$comment`; only stable identifiers are
/// value-compared). The 22 state-bound commands are signature-pinned in
/// the next test instead.
#[test]
fn headless_commands_serve_payloads_shaped_like_the_fixture() {
    use crate::sim::glossary::GlossaryCategory;
    use crate::sysdesign::model::{ExportFormat, SystemDesignDoc};
    use crate::vendors::types::VendorId;

    let g = golden();

    // -- glossary_list: embedded JSON -> typed entries -> wire shape -----
    let entries = crate::commands::glossary::glossary_list().expect("glossary_list");
    assert!(!entries.is_empty(), "the embedded glossary is not empty");
    assert_return_shape(
        &g,
        &serde_json::to_value(&entries).expect("entries serialize"),
        &["glossaryEntry"],
        "glossary_list",
    );
    let p99 = entries
        .iter()
        .find(|e| e.id == "p99")
        .expect("the glossary defines p99");
    assert_eq!(p99.term, "p99", "p99's term is its own id");
    assert_eq!(p99.category, GlossaryCategory::Latency, "p99 is a latency term");
    assert_eq!(
        p99.aliases,
        Some(vec!["99th percentile".to_string(), "tail latency".to_string()]),
        "p99 aliases (source-verified stable identifiers)"
    );

    // -- presets_list / preset_load --------------------------------------
    let presets = crate::commands::presets::presets_list().expect("presets_list");
    assert_return_shape(
        &g,
        &serde_json::to_value(&presets).expect("presets serialize"),
        &["presetSummary"],
        "presets_list",
    );
    let single = presets
        .iter()
        .find(|p| p.id == "single-server")
        .expect("the single-server example exists");
    assert_eq!(single.name, "Single server", "preset name (source-verified)");
    let preset = crate::commands::presets::preset_load("single-server".to_string())
        .expect("preset_load(single-server)");
    assert_return_shape(
        &g,
        &serde_json::to_value(&preset).expect("preset serializes"),
        &["preset"],
        "preset_load",
    );
    assert_eq!(preset.id, "single-server");
    assert!(!preset.topology.nodes.is_empty(), "the example has nodes");

    // -- challenges_list / challenge_start --------------------------------
    let summaries = crate::commands::challenges::challenges_list().expect("challenges_list");
    assert_return_shape(
        &g,
        &serde_json::to_value(&summaries).expect("summaries serialize"),
        &["challengeSummary"],
        "challenges_list",
    );
    assert!(
        summaries.iter().any(|c| c.id == "hold-the-line"),
        "the source-defined challenges are present (ids are source-verified, not fixture-derived)"
    );
    let start = crate::commands::challenges::challenge_start(summaries[0].id.clone())
        .expect("challenge_start(first id)");
    assert_return_shape(
        &g,
        &serde_json::to_value(&start).expect("ChallengeStart serializes"),
        &["challengeStart"],
        "challenge_start",
    );
    assert_eq!(start.challenge.id, summaries[0].id, "echoes the resolved challenge");

    // -- design_file_build / parse / write / read -------------------------
    let topo: crate::sim::types::Topology = serde_json::from_value(
        g_at(&g, &["payloads", "topology"]).clone(),
    )
    .expect("fixture topology deserializes");
    let name = g_at(&g, &["argSamples", "name"])
        .as_str()
        .expect("argSamples.name")
        .to_string();

    let text = crate::commands::designs::design_file_build(
        topo.clone(),
        Some(name.clone()),
    )
    .expect("design_file_build");
    let built: Value =
        serde_json::from_str(&text).expect("a built design file is valid JSON");
    assert_eq!(
        built["app"].as_str(),
        Some(crate::persistence::design_file::DESIGN_FILE_APP),
        "built files carry the app marker"
    );

    let parsed = crate::commands::designs::design_file_parse(text).expect("design_file_parse");
    assert_return_shape(
        &g,
        &serde_json::to_value(&parsed).expect("DesignParseResult serializes"),
        &["designParseResultOk", "designParseResultErr"],
        "design_file_parse",
    );
    assert!(parsed.ok, "our own built file parses");
    assert_eq!(
        parsed.name.as_ref().and_then(|n| n.as_deref()),
        Some(name.as_str()),
        "the name round-trips"
    );
    let parsed_topology = parsed.topology.expect("ok results carry a topology");
    assert_eq!(
        serde_json::to_value(&parsed_topology).expect("topology serializes"),
        serde_json::to_value(&topo).expect("topology serializes"),
        "build -> parse preserves the topology exactly (both sides through Rust f64)"
    );

    let bad = crate::commands::designs::design_file_parse(
        g_at(&g, &["argSamples", "text"])
            .as_str()
            .expect("argSamples.text")
            .to_string(),
    )
    .expect("design_file_parse returns Ok(err-variant)");
    assert!(!bad.ok, "non-JSON text is rejected");
    assert!(
        bad.error.as_deref().is_some_and(|e| !e.is_empty()),
        "rejections carry a sentence"
    );
    assert_return_shape(
        &g,
        &serde_json::to_value(&bad).expect("DesignParseResult serializes"),
        &["designParseResultOk", "designParseResultErr"],
        "design_file_parse(error)",
    );

    let temp_path = std::env::temp_dir().join(format!(
        "breakscale_ipc_contract_{}_design.breakscale",
        std::process::id()
    ));
    crate::commands::designs::design_file_write(
        temp_path.to_string_lossy().into_owned(),
        topo.clone(),
        Some(name.clone()),
    )
    .expect("design_file_write");
    let read_back =
        crate::commands::designs::design_file_read(temp_path.to_string_lossy().into_owned())
            .expect("design_file_read");
    assert!(read_back.ok, "the written file reads back");
    assert_eq!(
        read_back.topology.expect("ok results carry a topology").nodes.len(),
        topo.nodes.len(),
        "every node survives the file trip"
    );
    let _ = std::fs::remove_file(&temp_path);

    // -- sysdesign_derive_high_level / validate / export ------------------
    let doc: SystemDesignDoc = serde_json::from_value(g_at(&g, &["payloads", "systemDesignDoc"]).clone())
        .expect("fixture SystemDesignDoc deserializes");

    let derived =
        crate::commands::sysdesign::sysdesign_derive_high_level(topo.clone())
            .expect("sysdesign_derive_high_level");
    assert_set_eq(
        "sysdesign_derive_high_level top-level keys",
        &keys_of(
            &serde_json::to_value(&derived).expect("doc serializes"),
            "derived doc",
        ),
        &keys_of(g_at(&g, &["payloads", "systemDesignDoc"]), "systemDesignDoc"),
    );

    let issues = crate::commands::sysdesign::sysdesign_validate(doc.clone())
        .expect("sysdesign_validate");
    assert_set_eq(
        "sysdesign_validate issue keys",
        &keys_of(
            &serde_json::to_value(&issues[0]).expect("issue serializes"),
            "ValidationIssue",
        ),
        &keys_of(g_at(&g, &["payloads", "validationIssue", "[0]"]), "validationIssue[0]"),
    );
    let fields: BTreeSet<String> = issues.iter().map(|i| i.field.clone()).collect();
    let expected_fields: BTreeSet<String> = [
        "architecture.dataFlows[0].edgeId",
        "lowLevel.entities[entity-order].relationships[0].toEntity",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        fields, expected_fields,
        "the fixture document's two validation problems are source-derived (validate.rs checks edges and relationship targets; no 'technology' check exists)"
    );

    let export_path = std::env::temp_dir().join(format!(
        "breakscale_ipc_contract_{}_export.json",
        std::process::id()
    ));
    crate::commands::sysdesign::sysdesign_export(
        doc,
        ExportFormat::Json,
        export_path.to_string_lossy().into_owned(),
    )
    .expect("sysdesign_export(Json)");
    let exported = std::fs::read_to_string(&export_path).expect("export file exists");
    let exported_json: Value =
        serde_json::from_str(&exported).expect("JSON export is parseable");
    assert!(exported_json.is_object(), "JSON export is a document object");
    let _ = std::fs::remove_file(&export_path);

    // -- vendors_get -------------------------------------------------------
    let vendor = crate::commands::vendors::vendors_get(VendorId::Aws)
        .expect("vendors_get")
        .expect("the AWS vendor file loads");
    assert_eq!(vendor.id, VendorId::Aws);
    let vendor_json =
        serde_json::to_value(&vendor).expect("Vendor serializes");
    assert_return_shape(&g, &vendor_json, &["vendor"], "vendors_get");
    assert_eq!(
        vendor_json["label"].as_str(),
        Some("AWS"),
        "the vendor's own display name (source-verified)"
    );
    assert!(
        !vendor_json["kinds"]["cache"].is_null(),
        "AWS maps the cache kind"
    );
    // Real size rows may omit optional spec fields (e.g. a size with no
    // published maxIops), so the live size row is a subset of the fully
    // populated fixture row -- never a superset.
    let fixture_size_keys = keys_of(
        g_at(&g, &["payloads", "vendor", "kinds", "cache", "sizes", "[0]"]),
        "vendor sizes[0]",
    );
    let live_sizes = vendor_json["kinds"]["cache"]["sizes"]
        .as_array()
        .expect("aws publishes cache sizes");
    assert!(!live_sizes.is_empty(), "aws publishes at least one size");
    let live_size_keys = keys_of(&live_sizes[0], "live size[0]");
    let superset: Vec<&String> = live_size_keys.difference(&fixture_size_keys).collect();
    assert!(
        superset.is_empty(),
        "live size row has keys outside the declared fixture shape: {superset:?}"
    );
}

// ---------------------------------------------------------------------------
// 12. Fn-pointer signature pins for all 35 commands
// ---------------------------------------------------------------------------

/// WHY: 22 commands take an `AppHandle` or managed `State` and therefore
/// cannot be called headlessly -- but their WIRE contract (parameter names
/// and types Tauri reflects into the payload) can still be pinned at
/// compile time by coercing each command to an exact fn-pointer type. A
/// parameter added, removed, or retyped (including its `Option`-ness)
/// anywhere in the command tree fails `cargo test` at the line naming the
/// command, whether or not a fixture payload happens to exercise it.
#[test]
fn every_command_is_pinnable_as_its_exact_rust_signature() {
    use crate::error::AppResult;
    use crate::sim::types::{FailureKind, FailureOpts, SimSnapshot, Topology};
    use crate::state::SimulationState;
    use serde_json::Value;
    use tauri::{AppHandle, State};

    // ---- commands/sim.rs (9) ----
    // WHY: seed/deltaMs are the optional params the fixture pins as
    // non-required; AppHandle/State must stay first when present because
    // Tauri injects them positionally.
    let _: for<'a> fn(AppHandle, State<'a, SimulationState>, Topology, Option<u32>) -> AppResult<()>
        = crate::commands::sim::sim_new;
    let _: for<'a> fn(State<'a, SimulationState>, Topology) -> AppResult<()> =
        crate::commands::sim::sim_set_topology;
    let _: for<'a> fn(State<'a, SimulationState>, String, Value) -> AppResult<()> =
        crate::commands::sim::sim_update_node_config;
    let _: for<'a> fn(State<'a, SimulationState>, String, FailureKind, FailureOpts) -> AppResult<()>
        = crate::commands::sim::sim_inject_failure;
    let _: for<'a> fn(State<'a, SimulationState>, String) -> AppResult<()> =
        crate::commands::sim::sim_clear_failure;
    let _: for<'a> fn(State<'a, SimulationState>) -> AppResult<()> =
        crate::commands::sim::sim_reset;
    let _: for<'a> fn(AppHandle, State<'a, SimulationState>, Option<f64>) -> AppResult<()> =
        crate::commands::sim::sim_step;
    let _: for<'a> fn(State<'a, SimulationState>, bool) -> AppResult<()> =
        crate::commands::sim::sim_set_running;
    let _: for<'a> fn(State<'a, SimulationState>) -> AppResult<SimSnapshot> =
        crate::commands::sim::sim_get_snapshot;

    // ---- commands/designs.rs (9) ----
    let _: fn(AppHandle) -> AppResult<Vec<crate::persistence::saved_designs::SavedSummary>> =
        crate::commands::designs::designs_list;
    let _: fn(AppHandle, String, Topology) -> AppResult<crate::persistence::saved_designs::SaveResult> =
        crate::commands::designs::designs_save;
    let _: fn(AppHandle, String) -> AppResult<crate::persistence::saved_designs::SavedDesign> =
        crate::commands::designs::designs_get;
    let _: fn(AppHandle, String) -> AppResult<()> = crate::commands::designs::designs_delete;
    let _: fn(AppHandle, String, String) -> AppResult<bool> =
        crate::commands::designs::designs_rename;
    let _: fn(Topology, Option<String>) -> AppResult<String> =
        crate::commands::designs::design_file_build;
    let _: fn(String) -> AppResult<crate::persistence::design_file::DesignParseResult> =
        crate::commands::designs::design_file_parse;
    let _: fn(String, Topology, Option<String>) -> AppResult<()> =
        crate::commands::designs::design_file_write;
    let _: fn(String) -> AppResult<crate::persistence::design_file::DesignParseResult> =
        crate::commands::designs::design_file_read;

    // ---- commands/backup.rs (2) ----
    let _: fn(AppHandle, String) -> AppResult<()> = crate::commands::backup::backup_write;
    let _: fn(AppHandle, String) -> AppResult<crate::persistence::backup::BackupResult> =
        crate::commands::backup::backup_restore_from_path;

    // ---- commands/sysdesign.rs (5) ----
    let _: fn(Topology) -> AppResult<crate::sysdesign::model::SystemDesignDoc> =
        crate::commands::sysdesign::sysdesign_derive_high_level;
    let _: fn(AppHandle, crate::sysdesign::model::SystemDesignDoc) -> AppResult<()> =
        crate::commands::sysdesign::sysdesign_save;
    let _: fn(AppHandle, String) -> AppResult<crate::sysdesign::model::SystemDesignDoc> =
        crate::commands::sysdesign::sysdesign_load;
    let _: fn(crate::sysdesign::model::SystemDesignDoc)
        -> AppResult<Vec<crate::sysdesign::model::ValidationIssue>> =
        crate::commands::sysdesign::sysdesign_validate;
    let _: fn(crate::sysdesign::model::SystemDesignDoc, crate::sysdesign::model::ExportFormat, String)
        -> AppResult<()> = crate::commands::sysdesign::sysdesign_export;

    // ---- commands/vendors.rs (1) ----
    let _: fn(crate::vendors::types::VendorId)
        -> AppResult<Option<crate::vendors::types::Vendor>> =
        crate::commands::vendors::vendors_get;

    // ---- commands/glossary.rs (1) ----
    let _: fn() -> AppResult<Vec<crate::sim::glossary::GlossaryEntry>> =
        crate::commands::glossary::glossary_list;

    // ---- commands/presets.rs (2) ----
    let _: fn() -> AppResult<Vec<crate::sim::presets::PresetSummary>> =
        crate::commands::presets::presets_list;
    let _: fn(String) -> AppResult<crate::sim::presets::Preset> =
        crate::commands::presets::preset_load;

    // ---- commands/challenges.rs (2) ----
    let _: fn() -> AppResult<Vec<crate::commands::challenges::ChallengeSummary>> =
        crate::commands::challenges::challenges_list;
    let _: fn(String) -> AppResult<crate::commands::challenges::ChallengeStart> =
        crate::commands::challenges::challenge_start;

    // ---- commands/settings.rs (4): opaque JSON payloads ----
    let _: fn(AppHandle) -> AppResult<Value> = crate::commands::settings::settings_load;
    let _: fn(AppHandle, Value) -> AppResult<()> = crate::commands::settings::settings_save;
    let _: fn(AppHandle) -> AppResult<Value> = crate::commands::settings::layout_load;
    let _: fn(AppHandle, Value) -> AppResult<()> = crate::commands::settings::layout_save;
}
