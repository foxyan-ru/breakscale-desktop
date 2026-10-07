//! `vendors_get` command, invented by the frontend's Cost/VendorPanel port
//! (not in the original MIGRATION_PLAN.md command table -- see
//! `desktop/src/lib/api/vendors.ts`'s integration note).

use crate::error::AppResult;
use crate::vendors::data;
use crate::vendors::types::{Vendor, VendorId};

#[tauri::command]
pub fn vendors_get(vendor_id: VendorId) -> AppResult<Option<Vendor>> {
    Ok(data::vendor(vendor_id))
}
