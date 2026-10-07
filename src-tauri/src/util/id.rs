//! Id generation without a `uuid`/`rand` dependency -- see
//! MIGRATION_PLAN.md #10. Mirrors the spirit of `savedDesigns.ts`'s
//! `newId()` fallback path: a millisecond timestamp plus a process-local
//! source of entropy, good enough for "does not collide within one user's
//! saved-design shelf", which is the only property anything here relies on.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A new id, unique within this process (and, by the millisecond timestamp
/// component, overwhelmingly likely to be unique across processes too).
pub fn new_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("d{millis:x}{seq:x}")
}
