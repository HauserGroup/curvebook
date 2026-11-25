//! Consolidated integration suite for light `xtask` policy, release, and security gates.

#![forbid(unsafe_code)]

mod audit_p9_sec22;
mod audit_p9_sec23;
mod audit_r2_s13;
mod common;
mod r014_release_channel;
mod r026_release_scan;
mod r028_beta_assets;
mod r032_decoupled_gates;
mod zip_helper;
