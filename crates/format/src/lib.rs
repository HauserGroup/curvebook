//! Native `.curvebook` / `.cbtemplate` archive serialization, delimited tabular (CSV/TSV)
//! import/export, `ImportLinkSpec` pipeline execution, tidy `ResultSheet` exports, and
//! clean-room `.pzfx` / `.prism` adapters.
//!
//! Layer L3 file-format crate. Governing spec: `docs/file-format-parity.md` and
//! `docs/data-tables-and-transforms.md`.
#![forbid(unsafe_code)]

pub mod archive;
pub mod csv;
pub mod error;
pub mod export;
pub mod prism;
pub mod pzfx;
pub mod recovery;
pub mod registry;
pub mod schemas;
pub mod versioned;
pub mod xml;
pub mod zip;

pub use archive::{
    ArchiveKind, ArchiveManifest, MAX_ARCHIVE_FILE_BYTES, SheetManifestEntry, detect_dependency_cycles, instantiate_template_bytes,
    is_unpacked_project_dir, migrate_document_json, open_archive_bytes, open_archive_bytes_with_limits, open_project_bytes, open_project_file,
    open_project_unpacked_dir, open_template_bytes, open_template_file, sanitize_circular_hooks, save_project_bytes, save_project_file,
    save_project_unpacked_dir, save_template_bytes, save_template_file, strip_project_data_for_template, validate_project_document,
};
pub use csv::{
    CsvOptions, FilteredDelimitedGrid, ImportBaselineAdjustment, MAX_CSV_FILE_BYTES, apply_csv_import_link, export_datasheet_to_csv,
    filter_delimited_text, format_cell_token, format_csv_field, import_csv_to_datasheet, parse_cell_token, parse_delimited_records,
    rearrange_and_baseline_grid, write_delimited_records,
};
pub use error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
pub use export::{ResultSheetExportJson, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
pub use prism::{import_prism_bytes, import_prism_bytes_with_limits};
pub use pzfx::{import_pzfx_bytes, import_pzfx_str};
#[cfg(not(target_arch = "wasm32"))]
pub use recovery::DirectoryJournalBackend;
pub use recovery::{
    CURRENT_JOURNAL_VERSION, CURRENT_LOCK_VERSION, CrashRecoveryJournal, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, JournalOpRecord,
    JournalRecoveryWarning, LOCKFILE_NAME, MemoryJournalBackend, RecoveryReport, SNAPSHOT_FILE_NAME, compute_record_crc32,
};
pub use registry::{ExportOptions, ExportSource, Exporter, FormatDescriptor, FormatKind, FormatRegistry, ImportOptions, ImportPayload, Importer};
pub use schemas::{all_persisted_schemas, archive_manifest_schema, batch_v1_schema, document_schema, journal_record_schema, preferences_schema};
pub use versioned::{
    BATCH_SCHEMA_V1, CURRENT_PREFERENCES_VERSION, MAX_PREFERENCES_FILE_BYTES, PreferencesFile, VersionStep, load_preferences_file,
    parse_preferences_json, save_preferences_file, upgrade, validate_batch_manifest_version,
};
pub use xml::{MAX_XML_BYTES, MAX_XML_DEPTH, MAX_XML_NODES, XmlElement, XmlLimits, XmlNode, parse_xml, parse_xml_with_limits};
pub use zip::{MAX_ZIP_ENTRIES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_UNCOMPRESSED_BYTES, ZipArchive, ZipLimits, ZipWriter, sanitize_zip_path};

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
