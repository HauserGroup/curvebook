//! Native `.lumencraft` / `.lctemplate` archive serialization, delimited tabular (CSV/TSV)
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
pub mod xml;
pub mod zip;

pub use archive::{
    ArchiveKind, ArchiveManifest, SheetManifestEntry, detect_dependency_cycles, instantiate_template_bytes, migrate_document_json,
    open_archive_bytes, open_archive_bytes_with_limits, open_project_bytes, open_project_file, open_template_bytes, open_template_file,
    sanitize_circular_hooks, save_project_bytes, save_project_file, save_template_bytes, save_template_file, strip_project_data_for_template,
    validate_project_document,
};
pub use csv::{
    CsvOptions, FilteredDelimitedGrid, ImportBaselineAdjustment, apply_csv_import_link, export_datasheet_to_csv, filter_delimited_text,
    format_cell_token, format_csv_field, import_csv_to_datasheet, parse_cell_token, parse_delimited_records, rearrange_and_baseline_grid,
    write_delimited_records,
};
pub use error::{FormatError, ImportReport, ImportWarning, ImportWarningKind};
pub use export::{ResultSheetExportJson, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv};
pub use prism::{import_prism_bytes, import_prism_bytes_with_limits};
pub use pzfx::{import_pzfx_bytes, import_pzfx_str};
#[cfg(not(target_arch = "wasm32"))]
pub use recovery::DirectoryJournalBackend;
pub use recovery::{
    CrashRecoveryJournal, JOURNAL_FILE_NAME, JournalBackend, JournalLockLease, JournalOpRecord, LOCKFILE_NAME, MemoryJournalBackend, RecoveryReport,
    SNAPSHOT_FILE_NAME, compute_record_crc32,
};
pub use xml::{MAX_XML_BYTES, MAX_XML_DEPTH, MAX_XML_NODES, XmlElement, XmlLimits, XmlNode, parse_xml, parse_xml_with_limits};
pub use zip::{MAX_ZIP_ENTRIES, MAX_ZIP_ENTRY_BYTES, MAX_ZIP_UNCOMPRESSED_BYTES, ZipArchive, ZipLimits, ZipWriter, sanitize_zip_path};

/// Canonical file extension for native LumenCraft project archives.
pub const EXTENSION: &str = "lumencraft";

/// Canonical file extension for LumenCraft project templates.
pub const TEMPLATE_EXTENSION: &str = "lctemplate";

#[cfg(test)]
mod tests {
    use super::{EXTENSION, TEMPLATE_EXTENSION};

    #[test]
    fn native_extensions_are_non_empty() {
        assert_eq!(EXTENSION, "lumencraft");
        assert_eq!(TEMPLATE_EXTENSION, "lctemplate");
    }
}
