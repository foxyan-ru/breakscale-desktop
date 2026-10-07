//! Everything that reads or writes a Breakscale document outside the live
//! simulation: `.breakscale` design files, the named-designs shelf, and
//! whole-app backup/restore. Ported from `src/designFile.ts`,
//! `src/savedDesigns.ts` and `src/backup.ts` (see MIGRATION_PLAN.md §6-7).
//!
//! Every module here is filesystem-backed rather than DOM/`localStorage`-
//! backed, but keeps the same trust-boundary discipline as its TS source:
//! nothing panics on untrusted input, and every rejection returns a
//! sentence a user can act on rather than a stack trace or a raw I/O error.

pub mod backup;
pub mod design_file;
pub mod saved_designs;
