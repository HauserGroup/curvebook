//! Unified format descriptor, `Importer`/`Exporter` seam traits, and `FormatRegistry`.

use std::path::Path;
use std::sync::Arc;

use curvebook_doc::{DataSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableKind};

use crate::archive::{
    MAX_ARCHIVE_FILE_BYTES, is_unpacked_project_dir, open_project_bytes, open_project_unpacked_dir, open_template_bytes, save_project_bytes,
    save_project_unpacked_dir, save_template_bytes, validate_data_sheet_floats,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::archive::{platform_read_regular_file, save_project_file, save_template_file};
use crate::csv::{CsvOptions, MAX_CSV_FILE_BYTES, export_datasheet_to_csv, import_csv_to_datasheet};
use crate::error::{FormatError, ImportReport};
use crate::export::{export_result_sheet_json, try_export_result_sheet_tidy_csv};
use crate::prism::import_prism_bytes;
use crate::pzfx::import_pzfx_bytes;
use crate::xml::MAX_XML_BYTES;

/// Coarse category of a supported file format or container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormatKind {
    Archive,
    Template,
    ProjectDir,
    DelimitedText,
    ForeignProject,
    ResultTable,
    FigureVector,
    FigureRaster,
}

/// Static metadata identifying a registered import or export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatDescriptor {
    /// Canonical lowercase format identifier (e.g. `"curvebook"`, `"curvebook.d"`, `"tiff"`).
    pub id: &'static str,
    /// Human-readable display label.
    pub label: &'static str,
    /// Normalized lowercase extensions without leading dot (primary extension first).
    pub extensions: &'static [&'static str],
    /// Semantic format classification.
    pub kind: FormatKind,
}

/// Typed payload returned by an [`Importer`].
#[derive(Debug, Clone, PartialEq)]
pub enum ImportPayload {
    Document(ProjectDocument),
    DataSheet(DataSheet),
    Report(ImportReport),
}

impl ImportPayload {
    /// Extract a [`ProjectDocument`] if this payload contains a full project or foreign import report.
    pub fn into_document(self) -> Option<ProjectDocument> {
        match self {
            Self::Document(doc) => Some(doc),
            Self::Report(report) => Some(report.document),
            Self::DataSheet(_) => None,
        }
    }

    /// Extract a [`DataSheet`] if this payload represents a single imported sheet.
    pub fn into_data_sheet(self) -> Option<DataSheet> {
        match self {
            Self::DataSheet(sheet) => Some(sheet),
            Self::Document(_) | Self::Report(_) => None,
        }
    }
}

/// Caller options passed to an [`Importer`].
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Target sheet identifier used by single-sheet importers such as CSV/TSV.
    pub sheet_id: SheetId,
    /// Target sheet title used by single-sheet importers such as CSV/TSV.
    pub title: String,
    /// Target data-table archetype used by delimited-text importers.
    pub table_kind: TableKind,
    /// Subcolumn replicate mode used by delimited-text importers.
    pub subcolumn_mode: SubcolumnMode,
    /// Optional explicit CSV/TSV parsing configuration overriding format defaults.
    pub csv: Option<CsvOptions>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            sheet_id: SheetId::new("imported-1"),
            title: String::from("Imported Sheet"),
            table_kind: TableKind::Column,
            subcolumn_mode: SubcolumnMode::Single,
            csv: None,
        }
    }
}

/// Borrowed data source supplied to an [`Exporter`].
#[derive(Debug, Clone, Copy)]
pub enum ExportSource<'a> {
    Document(&'a ProjectDocument),
    DataSheet(&'a DataSheet),
    ResultSheet(&'a ResultSheet),
}

/// Caller options passed to an [`Exporter`].
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Optional explicit CSV/TSV formatting configuration.
    pub csv: Option<CsvOptions>,
    /// Whether JSON or structured outputs should be pretty-printed.
    pub pretty: bool,
    /// Optional target sheet identifier when exporting one sheet from a [`ProjectDocument`].
    pub sheet_id: Option<String>,
}

/// Strategy trait implemented by format importers.
pub trait Importer: Send + Sync {
    /// Metadata descriptor for this importer.
    fn descriptor(&self) -> FormatDescriptor;

    /// Maximum regular-file byte length permitted when importing from disk via [`Self::import_path`].
    fn max_input_bytes(&self) -> usize {
        MAX_ARCHIVE_FILE_BYTES
    }

    /// Decode an in-memory byte slice into a typed [`ImportPayload`].
    fn import_bytes(&self, bytes: &[u8], opts: &ImportOptions) -> Result<ImportPayload, FormatError>;

    /// Decode from a filesystem path (overridden by directory-backed formats such as `.curvebook.d`).
    fn import_path(&self, path: &Path, opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let bytes = platform_read_regular_file(path, self.max_input_bytes())?;
            self.import_bytes(&bytes, opts)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, opts);
            Err(FormatError::Io(String::from("filesystem import is not supported on wasm32")))
        }
    }
}

/// Strategy trait implemented by format exporters.
pub trait Exporter: Send + Sync {
    /// Metadata descriptor for this exporter.
    fn descriptor(&self) -> FormatDescriptor;

    /// Encode `source` into an in-memory byte buffer.
    fn export_bytes(&self, source: ExportSource<'_>, opts: &ExportOptions) -> Result<Vec<u8>, FormatError>;

    /// Persist `source` to `path` atomically (overridden by directory-backed formats such as `.curvebook.d`).
    fn export_path(&self, source: ExportSource<'_>, path: &Path, opts: &ExportOptions) -> Result<(), FormatError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let bytes = self.export_bytes(source, opts)?;
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)?;
            }
            curvebook_platform::fs::write_atomic(path, &bytes)?;
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, path, opts);
            Err(FormatError::Io(String::from("filesystem export is not supported on wasm32")))
        }
    }
}

/// Shared lookup registry unifying built-in and caller-supplied format adapters.
#[derive(Clone)]
pub struct FormatRegistry {
    importers: Vec<Arc<dyn Importer>>,
    exporters: Vec<Arc<dyn Exporter>>,
}

impl std::fmt::Debug for FormatRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormatRegistry")
            .field("importers", &self.supported_import_formats())
            .field("exporters", &self.supported_export_formats())
            .finish()
    }
}

impl Default for FormatRegistry {
    fn default() -> Self {
        Self::default_registry()
    }
}

impl FormatRegistry {
    /// Construct an empty registry with no pre-registered adapters.
    pub fn new() -> Self {
        Self { importers: Vec::new(), exporters: Vec::new() }
    }

    /// Construct the standard registry populated with CurveBook native, delimited-text,
    /// foreign-project, result-table, and figure descriptors.
    pub fn default_registry() -> Self {
        let mut reg = Self::new();
        reg.register_importer(Arc::new(CurveBookArchiveAdapter));
        reg.register_importer(Arc::new(UnpackedProjectDirAdapter));
        reg.register_importer(Arc::new(CbtemplateAdapter));
        reg.register_importer(Arc::new(PrismAdapter));
        reg.register_importer(Arc::new(PzfxAdapter));
        reg.register_importer(Arc::new(CsvAdapter));
        reg.register_importer(Arc::new(TsvAdapter));

        reg.register_exporter(Arc::new(CurveBookArchiveAdapter));
        reg.register_exporter(Arc::new(UnpackedProjectDirAdapter));
        reg.register_exporter(Arc::new(CbtemplateAdapter));
        reg.register_exporter(Arc::new(CsvAdapter));
        reg.register_exporter(Arc::new(TsvAdapter));
        reg.register_exporter(Arc::new(JsonResultAdapter));
        reg.register_exporter(Arc::new(FigurePlaceholderExporter::new(DESC_SVG)));
        reg.register_exporter(Arc::new(FigurePlaceholderExporter::new(DESC_PDF)));
        reg.register_exporter(Arc::new(FigurePlaceholderExporter::new(DESC_PNG)));
        reg.register_exporter(Arc::new(FigurePlaceholderExporter::new(DESC_TIFF)));
        reg.register_exporter(Arc::new(FigurePlaceholderExporter::new(DESC_EMF)));
        reg
    }

    /// Register or replace an [`Importer`] matching the same [`FormatDescriptor::id`].
    pub fn register_importer(&mut self, importer: Arc<dyn Importer>) {
        let id = importer.descriptor().id;
        if let Some(slot) = self.importers.iter_mut().find(|existing| existing.descriptor().id == id) {
            *slot = importer;
        } else {
            self.importers.push(importer);
        }
    }

    /// Register or replace an [`Exporter`] matching the same [`FormatDescriptor::id`].
    pub fn register_exporter(&mut self, exporter: Arc<dyn Exporter>) {
        let id = exporter.descriptor().id;
        if let Some(slot) = self.exporters.iter_mut().find(|existing| existing.descriptor().id == id) {
            *slot = exporter;
        } else {
            self.exporters.push(exporter);
        }
    }

    /// Look up a registered [`Importer`] by format id or extension token (case-insensitive, optional leading `.`).
    pub fn importer_by_format(&self, fmt: &str) -> Option<&dyn Importer> {
        let token = normalize_format_token(fmt);
        if token.is_empty() {
            return None;
        }
        self.importers.iter().find(|imp| descriptor_matches_token(&imp.descriptor(), &token)).map(Arc::as_ref)
    }

    /// Look up a registered [`Importer`] for `path`, checking unpacked project directories and
    /// multi-dot suffixes like `.curvebook.d` before single-extension matching, and rejecting
    /// non-project directories from single-file adapters.
    pub fn importer_for_path(&self, path: &Path) -> Option<&dyn Importer> {
        if is_unpacked_project_dir(path) {
            return self.importer_by_format(DESC_PROJECT_DIR.id);
        }
        if let Some(ext_token) = path_multi_dot_token(path, &self.supported_import_formats()) {
            return self.importer_by_format(ext_token);
        }
        if path.is_dir() {
            return None;
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str())
            && let Some(found) = self.importer_by_format(ext)
        {
            return Some(found);
        }
        None
    }

    /// Look up a registered [`Exporter`] by format id or extension token (case-insensitive, optional leading `.`).
    pub fn exporter_by_format(&self, fmt: &str) -> Option<&dyn Exporter> {
        let token = normalize_format_token(fmt);
        if token.is_empty() {
            return None;
        }
        self.exporters.iter().find(|exp| descriptor_matches_token(&exp.descriptor(), &token)).map(Arc::as_ref)
    }

    /// Look up a registered [`Exporter`] for `path`, checking unpacked project directories and
    /// multi-dot suffixes like `.curvebook.d` before single-extension matching, and refusing to
    /// treat arbitrary existing directories as `.curvebook.d` targets.
    pub fn exporter_for_path(&self, path: &Path) -> Option<&dyn Exporter> {
        if is_unpacked_project_dir(path) {
            return self.exporter_by_format(DESC_PROJECT_DIR.id);
        }
        if let Some(ext_token) = path_multi_dot_token(path, &self.supported_export_formats()) {
            return self.exporter_by_format(ext_token);
        }
        if path.is_dir() {
            return None;
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str())
            && let Some(found) = self.exporter_by_format(ext)
        {
            return Some(found);
        }
        None
    }

    /// Return descriptors for all registered importers in registration order.
    pub fn supported_import_formats(&self) -> Vec<FormatDescriptor> {
        self.importers.iter().map(|imp| imp.descriptor()).collect()
    }

    /// Return descriptors for all registered exporters in registration order.
    pub fn supported_export_formats(&self) -> Vec<FormatDescriptor> {
        self.exporters.iter().map(|exp| exp.descriptor()).collect()
    }
}

fn normalize_format_token(fmt: &str) -> String {
    fmt.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn descriptor_matches_token(desc: &FormatDescriptor, token: &str) -> bool {
    desc.id.eq_ignore_ascii_case(token) || desc.extensions.iter().any(|ext| ext.eq_ignore_ascii_case(token))
}

fn path_multi_dot_token<'a>(path: &Path, descriptors: &[FormatDescriptor]) -> Option<&'a str>
where
    FormatDescriptor: 'a,
{
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    for desc in descriptors {
        for &ext in desc.extensions {
            if ext.contains('.') {
                let suffix = format!(".{ext}");
                if file_name.ends_with(&suffix) {
                    return Some(ext);
                }
            }
        }
    }
    None
}

const DESC_ARCHIVE: FormatDescriptor =
    FormatDescriptor { id: "curvebook", label: "CurveBook Project Archive", extensions: &["curvebook"], kind: FormatKind::Archive };
const DESC_PROJECT_DIR: FormatDescriptor =
    FormatDescriptor { id: "curvebook.d", label: "CurveBook Unpacked Project", extensions: &["curvebook.d"], kind: FormatKind::ProjectDir };
const DESC_TEMPLATE: FormatDescriptor =
    FormatDescriptor { id: "cbtemplate", label: "CurveBook Template", extensions: &["cbtemplate"], kind: FormatKind::Template };
const DESC_PRISM: FormatDescriptor =
    FormatDescriptor { id: "prism", label: "External Project Archive (.prism)", extensions: &["prism"], kind: FormatKind::ForeignProject };
const DESC_PZFX: FormatDescriptor =
    FormatDescriptor { id: "pzfx", label: "PZFX XML Document", extensions: &["pzfx"], kind: FormatKind::ForeignProject };
const DESC_CSV: FormatDescriptor =
    FormatDescriptor { id: "csv", label: "Comma-Separated Values", extensions: &["csv"], kind: FormatKind::DelimitedText };
const DESC_TSV: FormatDescriptor =
    FormatDescriptor { id: "tsv", label: "Tab-Separated Values", extensions: &["tsv"], kind: FormatKind::DelimitedText };
const DESC_JSON: FormatDescriptor = FormatDescriptor { id: "json", label: "JSON Result Table", extensions: &["json"], kind: FormatKind::ResultTable };
const DESC_SVG: FormatDescriptor =
    FormatDescriptor { id: "svg", label: "Scalable Vector Graphics", extensions: &["svg"], kind: FormatKind::FigureVector };
const DESC_PDF: FormatDescriptor =
    FormatDescriptor { id: "pdf", label: "Portable Document Format", extensions: &["pdf"], kind: FormatKind::FigureVector };
const DESC_PNG: FormatDescriptor =
    FormatDescriptor { id: "png", label: "Portable Network Graphics", extensions: &["png"], kind: FormatKind::FigureRaster };
const DESC_TIFF: FormatDescriptor =
    FormatDescriptor { id: "tiff", label: "Tagged Image File Format", extensions: &["tiff", "tif"], kind: FormatKind::FigureRaster };
const DESC_EMF: FormatDescriptor = FormatDescriptor { id: "emf", label: "Enhanced Metafile", extensions: &["emf"], kind: FormatKind::FigureVector };

#[cfg(not(target_arch = "wasm32"))]
fn ensure_parent_dir_exists(path: &Path) -> Result<(), FormatError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

struct CurveBookArchiveAdapter;
impl Importer for CurveBookArchiveAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_ARCHIVE
    }

    fn import_bytes(&self, bytes: &[u8], _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Ok(ImportPayload::Document(open_project_bytes(bytes)?))
    }
}
impl Exporter for CurveBookArchiveAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_ARCHIVE
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document_source(source, DESC_ARCHIVE.id)?;
        save_project_bytes(doc)
    }

    fn export_path(&self, source: ExportSource<'_>, path: &Path, _opts: &ExportOptions) -> Result<(), FormatError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let doc = expect_document_source(source, DESC_ARCHIVE.id)?;
            ensure_parent_dir_exists(path)?;
            save_project_file(path, doc)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, path);
            Err(FormatError::Io(String::from("filesystem export is not supported on wasm32")))
        }
    }
}

struct UnpackedProjectDirAdapter;
impl Importer for UnpackedProjectDirAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PROJECT_DIR
    }

    fn import_bytes(&self, _bytes: &[u8], _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Err(FormatError::UnsupportedFormat { reason: String::from("`curvebook.d` requires a directory path rather than raw bytes") })
    }

    fn import_path(&self, path: &Path, _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Ok(ImportPayload::Document(open_project_unpacked_dir(path)?))
    }
}
impl Exporter for UnpackedProjectDirAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PROJECT_DIR
    }

    fn export_bytes(&self, _source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        Err(FormatError::UnsupportedFormat { reason: String::from("`curvebook.d` requires a directory path rather than raw bytes") })
    }

    fn export_path(&self, source: ExportSource<'_>, path: &Path, _opts: &ExportOptions) -> Result<(), FormatError> {
        let doc = expect_document_source(source, DESC_PROJECT_DIR.id)?;
        save_project_unpacked_dir(doc, path)
    }
}

struct CbtemplateAdapter;
impl Importer for CbtemplateAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_TEMPLATE
    }

    fn import_bytes(&self, bytes: &[u8], _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Ok(ImportPayload::Document(open_template_bytes(bytes)?))
    }
}
impl Exporter for CbtemplateAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_TEMPLATE
    }

    fn export_bytes(&self, source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let doc = expect_document_source(source, DESC_TEMPLATE.id)?;
        save_template_bytes(doc)
    }

    fn export_path(&self, source: ExportSource<'_>, path: &Path, _opts: &ExportOptions) -> Result<(), FormatError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let doc = expect_document_source(source, DESC_TEMPLATE.id)?;
            ensure_parent_dir_exists(path)?;
            save_template_file(path, doc)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (source, path);
            Err(FormatError::Io(String::from("filesystem export is not supported on wasm32")))
        }
    }
}

struct PrismAdapter;
impl Importer for PrismAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PRISM
    }

    fn import_bytes(&self, bytes: &[u8], _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Ok(ImportPayload::Report(import_prism_bytes(bytes)?))
    }
}

struct PzfxAdapter;
impl Importer for PzfxAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_PZFX
    }

    fn max_input_bytes(&self) -> usize {
        MAX_XML_BYTES
    }

    fn import_bytes(&self, bytes: &[u8], _opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        Ok(ImportPayload::Report(import_pzfx_bytes(bytes)?))
    }
}

struct CsvAdapter;
impl Importer for CsvAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_CSV
    }

    fn max_input_bytes(&self) -> usize {
        MAX_CSV_FILE_BYTES
    }

    fn import_bytes(&self, bytes: &[u8], opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        let text = decode_utf8_input(bytes, DESC_CSV.id)?;
        let csv_opts = opts.csv.clone().unwrap_or_else(|| CsvOptions::detect(text));
        let sheet = import_csv_to_datasheet(opts.sheet_id.clone(), opts.title.clone(), text, opts.table_kind, opts.subcolumn_mode, &csv_opts)?;
        Ok(ImportPayload::DataSheet(sheet))
    }
}
impl Exporter for CsvAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_CSV
    }

    fn export_bytes(&self, source: ExportSource<'_>, opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let csv_opts = opts.csv.clone().unwrap_or_else(CsvOptions::csv);
        export_delimited_bytes(source, opts, &csv_opts, DESC_CSV.id)
    }
}

struct TsvAdapter;
impl Importer for TsvAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_TSV
    }

    fn max_input_bytes(&self) -> usize {
        MAX_CSV_FILE_BYTES
    }

    fn import_bytes(&self, bytes: &[u8], opts: &ImportOptions) -> Result<ImportPayload, FormatError> {
        let text = decode_utf8_input(bytes, DESC_TSV.id)?;
        let csv_opts = opts.csv.clone().unwrap_or_else(CsvOptions::tsv);
        let sheet = import_csv_to_datasheet(opts.sheet_id.clone(), opts.title.clone(), text, opts.table_kind, opts.subcolumn_mode, &csv_opts)?;
        Ok(ImportPayload::DataSheet(sheet))
    }
}
impl Exporter for TsvAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_TSV
    }

    fn export_bytes(&self, source: ExportSource<'_>, opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let csv_opts = opts.csv.clone().unwrap_or_else(CsvOptions::tsv);
        export_delimited_bytes(source, opts, &csv_opts, DESC_TSV.id)
    }
}

struct JsonResultAdapter;
impl Exporter for JsonResultAdapter {
    fn descriptor(&self) -> FormatDescriptor {
        DESC_JSON
    }

    fn export_bytes(&self, source: ExportSource<'_>, opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        let text = match source {
            ExportSource::ResultSheet(sheet) => export_result_sheet_json(sheet, opts.pretty)?,
            ExportSource::DataSheet(sheet) => {
                validate_data_sheet_floats(sheet)?;
                serialize_json_text(sheet, opts.pretty)?
            }
            ExportSource::Document(doc) => {
                if let Some(target_id) = opts.sheet_id.as_deref() {
                    if let Some(rs) = doc.result_sheets.iter().find(|s| s.id.as_str() == target_id) {
                        export_result_sheet_json(rs, opts.pretty)?
                    } else if let Some(ds) = doc.data_sheets.iter().find(|s| s.id.as_str() == target_id) {
                        validate_data_sheet_floats(ds)?;
                        serialize_json_text(ds, opts.pretty)?
                    } else {
                        return Err(FormatError::Doc(curvebook_doc::DocError::SheetNotFound { sheet_id: SheetId::new(target_id) }));
                    }
                } else {
                    let Some(rs) = doc.result_sheets.first() else {
                        return Err(FormatError::UnsupportedFormat { reason: String::from("project has no result sheet to export as `json`") });
                    };
                    export_result_sheet_json(rs, opts.pretty)?
                }
            }
        };
        Ok(text.into_bytes())
    }
}

struct FigurePlaceholderExporter {
    desc: FormatDescriptor,
}

impl FigurePlaceholderExporter {
    const fn new(desc: FormatDescriptor) -> Self {
        Self { desc }
    }
}

impl Exporter for FigurePlaceholderExporter {
    fn descriptor(&self) -> FormatDescriptor {
        self.desc
    }

    fn export_bytes(&self, _source: ExportSource<'_>, _opts: &ExportOptions) -> Result<Vec<u8>, FormatError> {
        Err(FormatError::UnsupportedFormat { reason: format!("figure exporter `{}` requires a host rendering adapter", self.desc.id) })
    }
}

fn expect_document_source<'a>(source: ExportSource<'a>, format_id: &str) -> Result<&'a ProjectDocument, FormatError> {
    match source {
        ExportSource::Document(doc) => Ok(doc),
        ExportSource::DataSheet(_) | ExportSource::ResultSheet(_) => {
            Err(FormatError::UnsupportedFormat { reason: format!("format `{format_id}` requires a ProjectDocument source") })
        }
    }
}

fn decode_utf8_input<'a>(bytes: &'a [u8], format_id: &str) -> Result<&'a str, FormatError> {
    if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
        return Err(FormatError::UnsupportedFormat { reason: format!("binary ZIP archive payload cannot be imported as `{format_id}`") });
    }
    std::str::from_utf8(bytes).map_err(|err| FormatError::CsvParse { line: 1, reason: format!("invalid UTF-8 in {format_id} input: {err}") })
}

fn serialize_json_text<T: serde::Serialize>(value: &T, pretty: bool) -> Result<String, FormatError> {
    if pretty { Ok(serde_json::to_string_pretty(value)?) } else { Ok(serde_json::to_string(value)?) }
}

fn export_delimited_bytes(source: ExportSource<'_>, opts: &ExportOptions, csv_opts: &CsvOptions, format_id: &str) -> Result<Vec<u8>, FormatError> {
    let text = match source {
        ExportSource::DataSheet(sheet) => export_datasheet_to_csv(sheet, csv_opts)?,
        ExportSource::ResultSheet(sheet) => try_export_result_sheet_tidy_csv(sheet, csv_opts)?,
        ExportSource::Document(doc) => export_document_delimited_text(doc, opts, csv_opts, format_id)?,
    };
    Ok(text.into_bytes())
}

fn export_document_delimited_text(
    doc: &ProjectDocument,
    opts: &ExportOptions,
    csv_opts: &CsvOptions,
    format_id: &str,
) -> Result<String, FormatError> {
    if let Some(target_id) = opts.sheet_id.as_deref() {
        if let Some(ds) = doc.data_sheets.iter().find(|s| s.id.as_str() == target_id) {
            return export_datasheet_to_csv(ds, csv_opts);
        }
        if let Some(rs) = doc.result_sheets.iter().find(|s| s.id.as_str() == target_id) {
            return try_export_result_sheet_tidy_csv(rs, csv_opts);
        }
        return Err(FormatError::Doc(curvebook_doc::DocError::SheetNotFound { sheet_id: SheetId::new(target_id) }));
    }
    if let Some(ds) = doc.data_sheets.first() {
        return export_datasheet_to_csv(ds, csv_opts);
    }
    if let Some(rs) = doc.result_sheets.first() {
        return try_export_result_sheet_tidy_csv(rs, csv_opts);
    }
    Err(FormatError::UnsupportedFormat { reason: format!("project has no data or result sheet to export as `{format_id}`") })
}
