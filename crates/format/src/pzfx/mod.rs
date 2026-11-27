//! Clean-room `.pzfx` XML project importer and deterministic data-only exporter.
//!
//! Submodules:
//! - [`import`]: Full `.pzfx` XML importer supporting encoding detection, XSLT-wrapped documents,
//!   opaque `<Template>` body skipping up to 128 MiB, `<Table>` / `<HugeTable>` / `<Table1024>`,
//!   `ExtTableType` (`Nested` / `MultipleVariables`), v9 2-subcolumn `MeanSd` / `MeanSem` modes,
//!   `<SubColumnTitles>`, `Decimals`, per-table `<Info>` links, sequence ordering & selection,
//!   colour tags, rich text, and Symbol-font decoding.
//! - [`richtext`]: Adobe `SYMBOL.TXT` mapping, `.pzfx` rich-text <-> `curvebook-text` inline
//!   markup conversion, strict ISO-8601 date parsing, and colour-tag / highlight-palette helpers.
//! - [`export`]: Shared [`preflight_pzfx_export`] summary, deterministic data-only
//!   [`export_pzfx_bytes`] writer, tracked §6.3 [`PZFX_FEATURE_CORPUS_COVERAGE`] table, and
//!   reusable [`compare_pzfx_data_layer`] fidelity checker.

pub mod export;
pub mod import;
pub(crate) mod richtext;

#[cfg(not(target_arch = "wasm32"))]
pub use export::export_pzfx_file;
pub use export::{
    PZFX_FEATURE_CORPUS_COVERAGE, PZFX_ROOT_ELEMENT, PzfxDataLayerDiff, PzfxDataLayerDifference, PzfxDroppedGroup, PzfxDroppedKind,
    PzfxExportPreflight, PzfxExportReport, PzfxExportWarning, PzfxExportWarningKind, PzfxFeatureCoverageRow, PzfxFeatureCoverageStatus,
    compare_pzfx_data_layer, export_pzfx_bytes, lookup_pzfx_feature_coverage, preflight_pzfx_export,
};
pub use import::{
    BINARY_PZF_MAGIC, BINARY_PZF_UNSUPPORTED_MESSAGE, MAX_IMPORT_CELLS, binary_pzf_unsupported_error, import_pzfx_bytes, import_pzfx_str,
    is_binary_pzf_bytes,
};

pub(crate) use import::{allocate_valid_unique_sheet_id, clamp_title, import_pzfx_bytes_with_used_cells};
