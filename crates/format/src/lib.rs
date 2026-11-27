//! Native `.curvebook` / `.cbtemplate` archive serialization, delimited tabular (CSV/TSV)
//! import/export, `ImportLinkSpec` pipeline execution, tidy `ResultSheet` exports, and
//! clean-room `.pzfx` / `.prism` adapters.
//!
//! Layer L3 file-format crate. Governing spec: `docs/evidence/file-formats.md` and
//! `docs/implementation/data-tables-and-transforms.md`.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

pub mod archive;
pub mod csv;
pub mod error;
pub mod export;
pub mod image_asset;
pub mod plate;
pub mod preamble;
pub mod prism;
pub mod pzfx;
pub mod recovery;
pub mod registry;
pub mod schemas;
pub mod versioned;
pub mod xlsx;
pub mod xml;
pub mod zip;

pub use archive::{
    ArchiveKind, ArchiveManifest, AssetManifestEntry, MAX_ARCHIVE_ENTRY_BYTES, MAX_ARCHIVE_FILE_BYTES, MAX_ARCHIVE_UNCOMPRESSED_BYTES,
    SheetManifestEntry, detect_dependency_cycles, ensure_existing_archive_not_newer, ensure_existing_unpacked_dir_not_newer,
    instantiate_template_bytes, is_unpacked_project_dir, migrate_document_json, native_archive_zip_limits, open_archive_bytes,
    open_archive_bytes_with_limits, open_archive_bytes_with_limits_and_warnings, open_archive_bytes_with_warnings, open_project_bytes,
    open_project_file, open_project_unpacked_dir, open_project_unpacked_dir_with_warnings, open_template_bytes, open_template_file,
    sanitize_circular_hooks, save_project_bytes, save_project_file, save_project_unpacked_dir, save_template_bytes, save_template_file,
    strip_project_data_for_template, validate_project_document, validate_project_document_skip_checkpoints,
};
pub use csv::{
    CsvMultiImportResult, CsvOptions, FilteredDelimitedGrid, ImportBaselineAdjustment, MAX_CSV_FILE_BYTES, apply_csv_import_link,
    collapse_merged_header_rows, decode_delimited_text_bytes, export_datasheet_to_csv, filter_delimited_text, format_cell_token, format_csv_field,
    import_csv_sheets_with_filter, import_csv_to_datasheet, import_csv_to_datasheet_with_x_mode, parse_cell_token, parse_delimited_records,
    rearrange_and_baseline_grid, write_delimited_records,
};
pub use error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
pub use export::{
    ResultSheetExportJson, ensure_result_sheet_csv_exportable, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv,
    result_sheet_export_warning, try_export_result_sheet_tidy_csv, try_export_result_sheet_tidy_csv_with_warnings, try_export_result_tab_csv,
    try_export_result_tab_csv_with_warnings,
};
pub use image_asset::{ingest_image_asset, validate_image_asset};
pub use plate::{
    CURRENT_PLATE_MAP_VERSION, MAX_PLATE_MAP_FILE_BYTES, PLATE_MAP_EXTENSION, PlateMapFile, load_cbplate_file, parse_cbplate_json, save_cbplate_file,
    serialize_cbplate_json,
};
pub use preamble::{PreambleInfo, build_preamble_info_sheet, extract_preamble_info, parse_preamble_constant_value, try_parse_preamble_kv};
pub use prism::{
    ANALYSIS_MAPPINGS, AnalysisDesignKey, AnalysisMappingRow, AnalysisVerificationStatus, ArchiveExportOptions, ArchiveExportReport,
    CACHED_PAIRWISE_LINKS, CACHED_SCALAR_LINKS, CachedAnalysisReference, CachedBound, CachedPairwiseLink, CachedPairwiseRef, CachedResultDiagnostic,
    CachedScalarDiff, CachedScalarLink, CachedScalarRef, CorpusDesignEvidence, CrosswalkComparison, DesignTail, ExternalDataSetPositions,
    ImportedAnalysisFallback, MIN_VERIFIED_DESIGN_AGREEMENT_DEN, MIN_VERIFIED_DESIGN_AGREEMENT_NUM, MIN_VERIFIED_DESIGN_ANALYSES,
    MIN_VERIFIED_DESIGN_SOURCES, PromotedAnalysisRef, REBUILT_GRAPH_WARNING_LABEL, TranslatedAnalysis, VERIFIED_ANALYSIS_DESIGNS,
    VerifiedAnalysisDesign, cached_values_for_link, check_control_in_selection, check_source_sheet_option_compatibility,
    compare_recomputed_against_cached_results, compare_recomputed_against_reference, export_prism_bytes_experimental,
    export_prism_bytes_with_options, extract_analysis_design_key, extract_cached_analysis_reference, find_analysis_mapping,
    find_verified_analysis_design, import_prism_bytes, import_prism_bytes_with_limits, primary_input_data_sheet_uid, resolve_selected_data_sets,
    translate_analysis_parameters,
};
#[cfg(not(target_arch = "wasm32"))]
pub use pzfx::export_pzfx_file;
pub use pzfx::{
    BINARY_PZF_MAGIC, BINARY_PZF_UNSUPPORTED_MESSAGE, PZFX_FEATURE_CORPUS_COVERAGE, PZFX_ROOT_ELEMENT, PzfxDataLayerDiff, PzfxDataLayerDifference,
    PzfxDroppedGroup, PzfxDroppedKind, PzfxExportPreflight, PzfxExportReport, PzfxExportWarning, PzfxExportWarningKind, PzfxFeatureCoverageRow,
    PzfxFeatureCoverageStatus, binary_pzf_unsupported_error, compare_pzfx_data_layer, export_pzfx_bytes, import_pzfx_bytes, import_pzfx_str,
    is_binary_pzf_bytes, lookup_pzfx_feature_coverage, preflight_pzfx_export,
};
#[cfg(not(target_arch = "wasm32"))]
pub use recovery::DirectoryJournalBackend;
pub use recovery::{
    CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION, CrashRecoveryJournal, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, JournalOpRecord,
    JournalRecoveryWarning, LOCKFILE_NAME, MemoryJournalBackend, RecoveryReport, SNAPSHOT_FILE_NAME, compute_record_crc32,
};
pub use registry::{
    DESC_PLATE_MAP, ExportOptions, ExportSource, Exporter, FormatDescriptor, FormatKind, FormatRegistry, ImportOptions, ImportPayload, Importer,
};
pub use schemas::{
    all_persisted_schemas, archive_manifest_schema, batch_v1_schema, document_schema, journal_record_schema, plate_map_schema, preferences_schema,
};
pub use versioned::{
    BATCH_SCHEMA_V1, CURRENT_PREFERENCES_VERSION, MAX_PREFERENCES_FILE_BYTES, PreferencesFile, VersionStep, load_preferences_file,
    normalize_batch_manifest_v1, normalize_batch_manifest_v1_with_warnings, normalize_command_params_in_place, parse_preferences_json,
    save_preferences_file, upgrade, validate_batch_manifest_version,
};
pub use xlsx::{XlsxImportOptions, XlsxSheetInfo, import_xlsx_bytes, import_xlsx_bytes_with_options, list_xlsx_sheets};
pub use xml::{
    MAX_XML_BYTES, MAX_XML_DEPTH, MAX_XML_NODES, MAX_XML_OPAQUE_BYTES, ParsedXml, SkippedOpaqueElement, XmlElement, XmlEncoding, XmlLimits, XmlNode,
    XmlParseOptions, XmlWriter, decode_xml_entities, detect_xml_encoding, parse_xml, parse_xml_bytes, parse_xml_bytes_with_options,
    parse_xml_with_limits, parse_xml_with_options,
};
pub use zip::{
    MAX_ZIP_ENTRIES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_UNCOMPRESSED_BYTES, ZipArchive, ZipEntryClass, ZipEntryInfo, ZipIndex, ZipLimits, ZipReadOptions,
    ZipWriter, classify_zip_entry_name, decode_cp437, sanitize_zip_path, sanitize_zip_path_with_options,
};

/// Canonical file extension for native CurveBook project archives.
pub const EXTENSION: &str = "curvebook";

/// Canonical file extension for CurveBook project templates.
pub const TEMPLATE_EXTENSION: &str = "cbtemplate";

#[cfg(test)]
mod tests {
    use super::{EXTENSION, TEMPLATE_EXTENSION};

    #[test]
    fn native_extensions_are_non_empty() {
        assert_eq!(EXTENSION, "curvebook");
        assert_eq!(TEMPLATE_EXTENSION, "cbtemplate");
    }
}
