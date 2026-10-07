// This file only exists so `src-tauri` builds as a binary crate too; every
// real symbol lives in `lib.rs` behind `breakscale_desktop_lib`, so both the
// binary and a future mobile target (which links the lib directly) share one
// implementation.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    breakscale_desktop_lib::run();
}
