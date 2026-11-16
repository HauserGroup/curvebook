//! CJK and right-to-left (RTL) Wave 6 UI string catalog shard (`R10.21`).
//!
//! Downstream Wave 6 `cjk_rtl` workers add new [`CatalogEntry`] rows to [`ENTRIES`] without editing `i18n.rs`.

use super::CatalogEntry;

/// CJK / RTL localization shard entries registered into [`super::EXTRA_SHARD_CATALOGS`].
pub const ENTRIES: &[CatalogEntry] = &[];
