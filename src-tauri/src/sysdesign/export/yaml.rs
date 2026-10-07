//! YAML export. `serde_yaml` is the one dependency this migration could not
//! avoid (MIGRATION_PLAN.md #10): a hand-rolled emitter would have to get
//! YAML's quoting/escaping rules right for arbitrary user text (a design
//! name containing a colon, for instance), and that is exactly the kind of
//! thing that looks fine until it doesn't.

use crate::error::AppResult;
use crate::sysdesign::model::SystemDesignDoc;

pub fn export(doc: &SystemDesignDoc) -> AppResult<String> {
    Ok(serde_yaml::to_string(doc)?)
}
