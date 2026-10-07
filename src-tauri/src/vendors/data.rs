//! Faithful port of `src/content/vendors/index.ts` (the vendor registry)
//! and, indirectly, of `aws.ts`/`gcp.ts`/`azure.ts` (the published spec
//! data itself, embedded here as JSON rather than transcribed into Rust
//! struct literals -- see MIGRATION_PLAN.md #5 and this port's top-level
//! doc comment for why).
//!
//! One place that knows which vendors exist, so a picker, a lookup and a
//! saved preference cannot disagree about the set.

use crate::vendors::types::{Vendor, VendorId};

static AWS_JSON: &str = include_str!("../../data/vendors/aws.json");
static GCP_JSON: &str = include_str!("../../data/vendors/gcp.json");
static AZURE_JSON: &str = include_str!("../../data/vendors/azure.json");

/// Generic is not a vendor and has no data file: it is the ABSENCE of one,
/// which is why it carries no kinds. Returned here (rather than `None`, as
/// the TS side's async `loadVendor` does for generic, which has nothing to
/// lazily import) because the Rust API is synchronous and the value costs
/// nothing to construct -- this mirrors `index.ts`'s own `GENERIC` constant,
/// which does exist as a real `Vendor` value.
fn generic() -> Vendor {
    Vendor {
        id: VendorId::Generic,
        label: "Generic".to_string(),
        region: None,
        kinds: std::collections::HashMap::new(),
    }
}

/// Parse one embedded vendor JSON file. A parse failure means a checked-in
/// data file is malformed -- a build-time data bug, not a runtime condition
/// a caller can do anything about -- so it panics with the vendor id and
/// the error rather than silently falling back to `Generic`, which would
/// hide the real problem.
fn parse(id: &str, json: &str) -> Vendor {
    match serde_json::from_str::<Vendor>(json) {
        Ok(v) => v,
        Err(err) => panic!("data/vendors/{id}.json is malformed: {err}"),
    }
}

/// Look up a vendor's full product mapping and specs by id.
///
/// Always `Some` -- every `VendorId` has a value, `Generic`'s just has
/// empty `kinds` -- unlike the TS `loadVendor`, whose `Promise<Vendor |
/// null>` models a lazy dynamic `import()` that simply does not apply once
/// all three files are embedded at compile time.
pub fn vendor(id: VendorId) -> Option<Vendor> {
    Some(match id {
        VendorId::Generic => generic(),
        VendorId::Aws => parse("aws", AWS_JSON),
        VendorId::Gcp => parse("gcp", GCP_JSON),
        VendorId::Azure => parse("azure", AZURE_JSON),
    })
}

/// The vendor picker list: `(id, label)` for every vendor, in the order
/// `index.ts`'s `VENDORS` constant lists them. Note these labels are the
/// short picker labels (`"Google"`, `"Azure"`), which is deliberately not
/// always the same string as that vendor's own `Vendor.label` (`"Google
/// Cloud"`, `"Microsoft Azure"`) -- `index.ts` keeps the two separate, and
/// so does this port.
pub fn all_vendor_ids() -> Vec<(VendorId, String)> {
    vec![
        (VendorId::Generic, "Generic".to_string()),
        (VendorId::Aws, "AWS".to_string()),
        (VendorId::Gcp, "Google".to_string()),
        (VendorId::Azure, "Azure".to_string()),
    ]
}
