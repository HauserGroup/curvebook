//! Consolidated CurveBook desktop performance, stress, and R10 routing integration test suite.
#![forbid(unsafe_code)]

#[path = "../common/mod.rs"]
mod common;

mod r10_29_desktop_import_export;
mod r10_command_routing;
mod r17_3_perf_scenarios;
