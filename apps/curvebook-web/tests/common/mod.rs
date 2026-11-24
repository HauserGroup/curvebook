#![forbid(unsafe_code)]
#![allow(dead_code)]
//! Shared integration-test fixtures for `apps/curvebook-web`.

/// Web host shell HTML embedded from the package root.
pub const INDEX_HTML: &str = include_str!("../../index.html");
