#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../common/mod.rs"]
mod common;

mod archive_and_csv;
mod corrupt_archive_fuzz;
mod interop_and_export;
mod prop_format;
mod r13_7_image_assets;
mod recovery_and_interop_parity;
mod rv_ledger_format;
mod unpacked_and_pzfx_budget;
mod versioned_and_seams;
