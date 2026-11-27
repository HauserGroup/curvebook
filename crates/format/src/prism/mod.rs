//! Clean-room `.prism` ZIP container importer, analysis translator, preview ingestor, and
//! experimental structural writer.
//!
//! Architecture:
//! - [`schema`]: external ID-indexed JSON models, whole-segment archive ID resolution,
//!   polymorphic rich-text conversion, strict ISO-8601 date parsing, and color parsing.
//! - [`import`]: lazy [`crate::zip::ZipIndex`] loader with dialect routing across CurveBook-native
//!   embedded `document.json`, external ID-indexed `.prism` archives (headerless CSVs with
//!   corpus-verified column ordering), and legacy generic `manifest.json`/CSV archives.
//! - [`analyses`]: 3-hop `preferredResultSheet` imported-result ingestion, data-driven
//!   [`ANALYSIS_MAPPINGS`] table with textbook justifications and verification status, and
//!   cached-vs-recomputed diagnostic comparison helpers.
//! - [`graphs`] & [`layouts`]: raster `preview.png` / `media/*` asset ingestion and structured
//!   `UnsupportedGraphOrLayout` warnings for opaque binary graph/layout entries.
//! - [`export`]: experimental `.prism` structural writer gated behind
//!   [`ArchiveExportOptions::experimental`] (import-only for Beta because external archives
//!   require undocumented `.dt` digest tokens and opaque binary `.bin` blobs).

pub mod analyses;
pub mod export;
pub mod graphs;
pub mod import;
pub mod layouts;
pub mod schema;

pub use analyses::{
    ANALYSIS_MAPPINGS, AnalysisMappingRow, AnalysisVerificationStatus, CACHED_SCALAR_LINKS, CachedResultDiagnostic, CachedScalarDiff,
    CachedScalarLink, CrosswalkComparison, ExternalDataSetPositions, TranslatedAnalysis, build_disagreement_checklist_item, cached_values_for_link,
    check_control_in_selection, compare_recomputed_against_cached_results, find_analysis_mapping, primary_input_data_sheet_uid,
    resolve_selected_data_sets, translate_analysis_parameters,
};
pub use export::{ArchiveExportOptions, ArchiveExportReport, export_prism_bytes_experimental, export_prism_bytes_with_options};
pub use import::{MAX_PROJECT_CELLS, import_prism_bytes, import_prism_bytes_with_limits};
pub use schema::{
    ArchiveIdIndex, ExternalDocumentDialect, IdLookup, parse_archive_color, parse_rich_text_value, parse_row_index_ranges, parse_strict_iso8601_date,
};
