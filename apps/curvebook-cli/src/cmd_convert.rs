//! Document loading helpers and `curvebook-cli convert <in> <out>` implementation.

use std::fs;
use std::path::Path;

use curvebook_doc::{
    GraphSheet, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, TableKind, XColumnMode, validate_project_cells, validate_sheet_cells,
};
use curvebook_engine::services::{ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterSizingPolicy, export_document_artifact};
use curvebook_format::registry::{ExportOptions, ExportSource, FormatKind, FormatRegistry, ImportOptions, ImportPayload};
use curvebook_format::{
    ArchiveKind, CsvOptions, ImportReport, MAX_ARCHIVE_FILE_BYTES, XlsxImportOptions, import_prism_bytes, import_pzfx_bytes, import_xlsx_bytes,
    is_unpacked_project_dir, open_archive_bytes,
};
use curvebook_pdf::{PdfExportOptions, PdfVersion};
use curvebook_platform::PlatformError;
use curvebook_platform::fs::write_atomic;
use curvebook_render::TiffCompression;
use curvebook_svg::SvgExportOptions;
use curvebook_text::parse_truetype_font_info;
use serde_json::json;

use crate::error::CliError;

/// Default raster DPI used when `--dpi` is omitted on PNG/TIFF export.
const DEFAULT_RASTER_DPI: u16 = 300;

/// Minimum supported raster DPI for PNG/TIFF exports.
pub(crate) const MIN_RASTER_DPI: u16 = 18;

/// Maximum supported raster DPI for PNG/TIFF exports.
pub(crate) const MAX_RASTER_DPI: u16 = 2400;

/// Maximum TrueType SFNT font file read size (`64 MiB`).
const MAX_FONT_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Loaded project document paired with its detected container format tag and import warnings.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub doc: ProjectDocument,
    pub format_tag: &'static str,
    pub warnings: Vec<String>,
}

/// Supported target export formats for `convert`, `export`, and batch manifest `exports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    CurveBook,
    CurveBookDir,
    Svg,
    Pdf,
    Emf,
    Png,
    Tiff,
    Csv,
    Tsv,
    Json,
}

impl ExportFormat {
    /// Canonical lowercase format name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurveBook => "curvebook",
            Self::CurveBookDir => "curvebook.d",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
            Self::Emf => "emf",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
        }
    }
}

/// Export configuration shared by `convert` and `batch`.
pub struct ExportRequest<'a> {
    pub doc: &'a ProjectDocument,
    pub out_path: &'a Path,
    pub format: ExportFormat,
    pub sheet: Option<&'a str>,
    pub dpi: Option<u16>,
    pub pretty: bool,
    pub tab_index: Option<usize>,
    pub pdf_version: PdfVersion,
    pub embedded_truetype_font: Option<&'a [u8]>,
}

/// Summary metadata produced after writing an export artifact.
#[derive(Debug, Clone)]
pub struct ExportOutcomeSummary {
    pub out_path: String,
    pub format: &'static str,
    pub sheet_id: Option<String>,
    pub bytes_written: usize,
}

struct ConvertArgs {
    input_path: String,
    output_path: String,
    format: Option<ExportFormat>,
    sheet: Option<String>,
    xlsx_sheets: Option<Vec<String>>,
    dpi: Option<u16>,
    pdf_version: PdfVersion,
    ttf_font: Option<String>,
    json: bool,
}

/// Execute `curvebook-cli convert <in> <out> [--sheet <id>] [--format <fmt>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]`.
pub fn run_convert(args: &[String]) -> Result<(), CliError> {
    run_convert_or_export(args, "convert", "converted")
}

/// Execute `curvebook-cli export <in> (<out> | --out <out>) [--sheet <id>] [--format <fmt>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]`.
pub fn run_export(args: &[String]) -> Result<(), CliError> {
    run_convert_or_export(args, "export", "exported")
}

pub(crate) fn run_guarded_cli<T, F>(op: &str, f: F) -> Result<T, CliError>
where
    F: FnOnce() -> Result<T, CliError>,
{
    match curvebook_engine::guard::run_guarded(op, || Ok(f())) {
        Ok(res) => res,
        Err(engine_err) => Err(CliError::Engine(engine_err)),
    }
}

fn run_convert_or_export(args: &[String], command_name: &str, verb: &str) -> Result<(), CliError> {
    let guard_op = if command_name == "export" { "cli.export" } else { "cli.convert" };
    run_guarded_cli(guard_op, || {
        let parsed = parse_export_or_convert_args(args, command_name)?;
        let in_path = Path::new(&parsed.input_path);
        let out_path = Path::new(&parsed.output_path);
        let font_bytes = parsed.ttf_font.as_deref().map(Path::new).map(load_validated_ttf_font).transpose()?;

        let (loaded, _input_bytes) = load_project_from_path_with_xlsx_sheets(in_path, true, parsed.xlsx_sheets.as_deref())?;
        let format = match parsed.format {
            Some(fmt) => fmt,
            None => detect_export_format_from_path(out_path)?,
        };

        let summary = export_from_document(&ExportRequest {
            doc: &loaded.doc,
            out_path,
            format,
            sheet: parsed.sheet.as_deref(),
            dpi: parsed.dpi,
            pretty: true,
            tab_index: None,
            pdf_version: parsed.pdf_version,
            embedded_truetype_font: font_bytes.as_deref(),
        })?;

        if parsed.json {
            let payload = if format == ExportFormat::Pdf {
                json!({
                    "ok": true,
                    "input": parsed.input_path,
                    "input_format": loaded.format_tag,
                    "output": summary.out_path,
                    "format": summary.format,
                    "sheet_id": summary.sheet_id,
                    "bytes_written": summary.bytes_written,
                    "pdf_version": pdf_version_str(parsed.pdf_version),
                    "ttf_font_embedded": font_bytes.is_some(),
                    "warnings": loaded.warnings,
                })
            } else {
                json!({
                    "ok": true,
                    "input": parsed.input_path,
                    "input_format": loaded.format_tag,
                    "output": summary.out_path,
                    "format": summary.format,
                    "sheet_id": summary.sheet_id,
                    "bytes_written": summary.bytes_written,
                    "warnings": loaded.warnings,
                })
            };
            let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            println!("{text}");
        } else {
            println!(
                "{verb} `{}` ({}) -> `{}` ({}, {} bytes)",
                parsed.input_path, loaded.format_tag, summary.out_path, summary.format, summary.bytes_written
            );
        }
        Ok(())
    })
}

/// Parse a PDF specification version string (`1.4` or `1.7`).
pub fn parse_pdf_version(raw: &str) -> Result<PdfVersion, CliError> {
    match raw.trim() {
        "1.4" => Ok(PdfVersion::Pdf14),
        "1.7" => Ok(PdfVersion::Pdf17),
        other => Err(CliError::Usage(format!("invalid --pdf-version `{other}` (expected `1.4` or `1.7`)"))),
    }
}

/// Canonical string representation of a [`PdfVersion`].
#[must_use]
pub const fn pdf_version_str(version: PdfVersion) -> &'static str {
    match version {
        PdfVersion::Pdf14 => "1.4",
        PdfVersion::Pdf17 => "1.7",
    }
}

/// Read and validate TrueType SFNT bytes from `path` via [`parse_truetype_font_info`].
pub fn load_validated_ttf_font(path: &Path) -> Result<Vec<u8>, CliError> {
    let bytes = curvebook_platform::fs::read_regular_file_bytes(path, MAX_FONT_FILE_BYTES).map_err(|err| match err {
        PlatformError::FileTooLarge { .. } => CliError::from(err),
        other => CliError::Runtime(format!("failed to read TrueType font `{}`: {other}", path.display())),
    })?;
    let _info = parse_truetype_font_info(&bytes).map_err(|err| CliError::Runtime(format!("invalid TrueType font `{}`: {err}", path.display())))?;
    if !matches!(bytes.get(0..4), Some([0x00, 0x01, 0x00, 0x00] | b"true")) {
        return Err(CliError::Runtime(format!(
            "unsupported SFNT font `{}`: expected TrueType outlines (`0x00010000` or `true`), not CFF/OTTO",
            path.display()
        )));
    }
    Ok(bytes)
}

/// Return `true` when `path` carries a `.curvebook.d` directory suffix (case-insensitive).
#[must_use]
pub fn has_unpacked_dir_suffix(path: &Path) -> bool {
    path.file_name().and_then(std::ffi::OsStr::to_str).is_some_and(|name| name.to_ascii_lowercase().ends_with(".curvebook.d"))
}

/// Return `true` when `path` refers to an unpacked `.curvebook.d` directory (either by
/// `.curvebook.d` suffix or an existing unpacked `manifest.json` marker), without classifying
/// arbitrary directories such as `.` (`R2-S12-02`).
#[must_use]
pub fn is_unpacked_project_path(path: &Path) -> bool {
    has_unpacked_dir_suffix(path) || is_unpacked_project_dir(path)
}

/// Compute total file byte size inside an unpacked directory tree.
pub fn dir_size_bytes(root: &Path) -> Result<usize, CliError> {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|err| CliError::Runtime(format!("failed to read `{}`: {err}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|err| CliError::Runtime(format!("failed to inspect `{}`: {err}", dir.display())))?;
            let meta = entry.metadata().map_err(|err| CliError::Runtime(format!("failed to stat `{}`: {err}", entry.path().display())))?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(usize::try_from(total).unwrap_or(usize::MAX))
}

fn export_format_from_descriptor_id(id: &str) -> Option<ExportFormat> {
    match id {
        "curvebook" => Some(ExportFormat::CurveBook),
        "curvebook.d" => Some(ExportFormat::CurveBookDir),
        "svg" => Some(ExportFormat::Svg),
        "pdf" => Some(ExportFormat::Pdf),
        "emf" => Some(ExportFormat::Emf),
        "png" => Some(ExportFormat::Png),
        "tiff" => Some(ExportFormat::Tiff),
        "csv" => Some(ExportFormat::Csv),
        "tsv" => Some(ExportFormat::Tsv),
        "json" => Some(ExportFormat::Json),
        _ => None,
    }
}

/// Parse an export format keyword or file extension via [`FormatRegistry`].
pub fn parse_export_format(raw: &str) -> Result<ExportFormat, CliError> {
    let norm = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    let canonical = match norm.as_str() {
        "curvebook-dir" | "unpacked" | "dir" => "curvebook.d",
        other => other,
    };
    let registry = FormatRegistry::default_registry();
    if let Some(exporter) = registry.exporter_by_format(canonical)
        && let Some(fmt) = export_format_from_descriptor_id(exporter.descriptor().id)
    {
        return Ok(fmt);
    }
    Err(CliError::Usage(format!("unsupported export format `{raw}` (expected curvebook, curvebook.d, svg, pdf, emf, png, tiff, csv, tsv, json)")))
}

/// Infer [`ExportFormat`] from an output path via [`FormatRegistry`] (including `.curvebook.d` directories).
pub fn detect_export_format_from_path(path: &Path) -> Result<ExportFormat, CliError> {
    if is_unpacked_project_path(path) {
        return Ok(ExportFormat::CurveBookDir);
    }
    if path.is_dir() {
        return Err(CliError::Usage(format!("output path `{}` is a directory without a `.curvebook.d` suffix", path.display())));
    }
    let registry = FormatRegistry::default_registry();
    if let Some(exporter) = registry.exporter_for_path(path)
        && exporter.descriptor().id != "curvebook.d"
        && let Some(fmt) = export_format_from_descriptor_id(exporter.descriptor().id)
    {
        return Ok(fmt);
    }
    let ext = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| CliError::Usage(format!("output path `{}` has no recognizable format extension", path.display())))?;
    parse_export_format(ext)
}

/// Load a [`ProjectDocument`] from a file path or unpacked `.curvebook.d` directory, returning `(loaded, input_bytes)`.
pub fn load_project_from_path(path: &Path, allow_delimited: bool) -> Result<(LoadedProject, usize), CliError> {
    load_project_from_path_with_xlsx_sheets(path, allow_delimited, None)
}

/// Load a [`ProjectDocument`] from a file path or unpacked `.curvebook.d` directory, optionally
/// restricting `.xlsx` imports to specific worksheet tab names.
pub fn load_project_from_path_with_xlsx_sheets(
    path: &Path,
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<(LoadedProject, usize), CliError> {
    if is_unpacked_project_path(path) {
        let registry = FormatRegistry::default_registry();
        let importer = registry
            .importer_by_format("curvebook.d")
            .ok_or_else(|| CliError::Runtime(String::from("missing `curvebook.d` importer in FormatRegistry")))?;
        let payload = importer.import_path(path, &ImportOptions::default())?;
        let doc = payload.into_document().ok_or_else(|| CliError::Runtime(String::from("`curvebook.d` importer returned non-document payload")))?;
        let total_bytes = dir_size_bytes(path)?;
        return Ok((LoadedProject { doc, format_tag: "curvebook.d", warnings: Vec::new() }, total_bytes));
    }
    let limit = FormatRegistry::default_registry().importer_for_path(path).map_or(MAX_ARCHIVE_FILE_BYTES, |importer| importer.max_input_bytes());
    let bytes = curvebook_platform::fs::read_regular_file_bytes(path, limit).map_err(|err| match err {
        PlatformError::FileTooLarge { .. } => CliError::from(err),
        other => CliError::Runtime(format!("failed to read `{}`: {other}", path.display())),
    })?;
    let loaded = load_project_from_bytes_with_xlsx_sheets(path, &bytes, allow_delimited, xlsx_sheets)?;
    Ok((loaded, bytes.len()))
}

fn load_project_from_bytes_with_xlsx_sheets(
    path_hint: &Path,
    bytes: &[u8],
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<LoadedProject, CliError> {
    let registry = FormatRegistry::default_registry();
    if let Some(importer) = registry.importer_for_path(path_hint) {
        let desc = importer.descriptor();
        return import_with_registered_adapter(importer, desc.id, desc.kind, path_hint, bytes, allow_delimited, xlsx_sheets);
    }
    let ext = path_hint.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    if ext.as_deref() == Some("json") {
        return load_project_json(bytes);
    }
    sniff_project_bytes(bytes)
}

fn import_with_registered_adapter(
    importer: &dyn curvebook_format::registry::Importer,
    format_tag: &'static str,
    kind: FormatKind,
    path_hint: &Path,
    bytes: &[u8],
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<LoadedProject, CliError> {
    match kind {
        FormatKind::Template => {
            let payload = importer.import_bytes(bytes, &ImportOptions::default())?;
            let doc = payload.into_document().ok_or_else(|| CliError::Runtime(format!("importer `{format_tag}` returned non-document payload")))?;
            Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
        }
        FormatKind::Archive => {
            let doc = match importer.import_bytes(bytes, &ImportOptions::default()) {
                Ok(payload) => {
                    payload.into_document().ok_or_else(|| CliError::Runtime(format!("importer `{format_tag}` returned non-document payload")))?
                }
                Err(registry_err) => {
                    if let Ok((_, doc)) = open_archive_bytes(bytes) {
                        doc
                    } else {
                        return Err(CliError::from(registry_err));
                    }
                }
            };
            Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
        }
        FormatKind::ForeignProject => {
            let mut opts = ImportOptions::default();
            if format_tag == "xlsx"
                && let Some(sheets) = xlsx_sheets
            {
                let stem = path_hint.file_stem().and_then(std::ffi::OsStr::to_str).map(str::to_owned);
                opts.xlsx = Some(XlsxImportOptions { sheet_names: Some(sheets.to_vec()), title: stem, ..XlsxImportOptions::default() });
            }
            match importer.import_bytes(bytes, &opts)? {
                ImportPayload::Report(report) => load_import_report(report, format_tag),
                ImportPayload::Document(doc) => Ok(LoadedProject { doc, format_tag, warnings: Vec::new() }),
                ImportPayload::DataSheet(_) => Err(CliError::Runtime(format!("importer `{format_tag}` returned unexpected single-sheet payload"))),
            }
        }
        FormatKind::DelimitedText => {
            if !allow_delimited {
                return Err(CliError::Usage(String::from("delimited CSV/TSV files are not project archives")));
            }
            load_delimited_with_importer(importer, path_hint, bytes, format_tag)
        }
        FormatKind::ProjectDir | FormatKind::ResultTable | FormatKind::FigureVector | FormatKind::FigureRaster => sniff_project_bytes(bytes),
    }
}

/// Write an exported sheet or whole document to `req.out_path` using atomic writes and [`FormatRegistry`].
pub fn export_from_document(req: &ExportRequest<'_>) -> Result<ExportOutcomeSummary, CliError> {
    run_guarded_cli("cli.export_from_document", || {
        if let Some(dpi) = req.dpi
            && !(MIN_RASTER_DPI..=MAX_RASTER_DPI).contains(&dpi)
        {
            return Err(CliError::Usage(format!("--dpi must be in {MIN_RASTER_DPI}..={MAX_RASTER_DPI} (got {dpi})")));
        }

        if matches!(req.format, ExportFormat::CurveBook | ExportFormat::CurveBookDir) {
            let registry = FormatRegistry::default_registry();
            let exporter = registry
                .exporter_by_format(req.format.as_str())
                .ok_or_else(|| CliError::Runtime(format!("missing `{}` exporter in FormatRegistry", req.format.as_str())))?;
            exporter.export_path(ExportSource::Document(req.doc), req.out_path, &ExportOptions::default())?;
            let bytes_written = if req.format == ExportFormat::CurveBookDir {
                dir_size_bytes(req.out_path)?
            } else {
                let meta = fs::metadata(req.out_path)
                    .map_err(|err| CliError::Runtime(format!("failed to stat exported archive `{}`: {err}", req.out_path.display())))?;
                usize::try_from(meta.len()).unwrap_or(usize::MAX)
            };
            return Ok(ExportOutcomeSummary {
                out_path: req.out_path.to_string_lossy().into_owned(),
                format: req.format.as_str(),
                sheet_id: None,
                bytes_written,
            });
        }

        let (artifact_req, sheet_id) = build_cli_export_request(req)?;
        let artifact = export_document_artifact(req.doc, &artifact_req, None, "cli.export_from_document")?;

        if let Some(parent) = req.out_path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            fs::create_dir_all(parent)
                .map_err(|err| CliError::Runtime(format!("failed to create output directory `{}`: {err}", parent.display())))?;
        }
        write_atomic(req.out_path, &artifact.bytes)?;

        Ok(ExportOutcomeSummary {
            out_path: req.out_path.to_string_lossy().into_owned(),
            format: req.format.as_str(),
            sheet_id,
            bytes_written: artifact.bytes.len(),
        })
    })
}

fn load_native_archive(bytes: &[u8]) -> Result<LoadedProject, CliError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    let format_tag = match kind {
        ArchiveKind::Project => "curvebook",
        ArchiveKind::Template => "cbtemplate",
    };
    Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
}

fn load_import_report(report: ImportReport, format_tag: &'static str) -> Result<LoadedProject, CliError> {
    let warnings = report.warnings.into_iter().map(|w| w.message).collect();
    Ok(LoadedProject { doc: report.document, format_tag, warnings })
}

fn load_project_json(bytes: &[u8]) -> Result<LoadedProject, CliError> {
    let raw_value: serde_json::Value = serde_json::from_slice(bytes).map_err(|err| CliError::Runtime(format!("invalid project JSON: {err}")))?;
    if !raw_value.is_object() {
        return Err(CliError::Runtime(String::from("invalid project JSON: expected top-level JSON object")));
    }
    let doc_version = match raw_value.get("schema_version") {
        None | Some(serde_json::Value::Null) => 0,
        Some(v) => {
            v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or_else(|| CliError::Runtime(format!("invalid project `schema_version`: {v}")))?
        }
    };
    let migrated_value = curvebook_format::archive::migrate_document_json(raw_value, doc_version)?;
    let doc: ProjectDocument = serde_json::from_value(migrated_value).map_err(|err| CliError::Runtime(format!("invalid project JSON: {err}")))?;
    validate_loaded_project_document(&doc)?;
    Ok(LoadedProject { doc, format_tag: "json", warnings: Vec::new() })
}

fn validate_loaded_project_document(doc: &ProjectDocument) -> Result<(), CliError> {
    curvebook_format::archive::validate_project_document(doc)?;
    let mut total_cells = 0_usize;
    for sheet in &doc.data_sheets {
        if let TableArchetype::Xy(xy) = &sheet.archetype
            && let XColumnMode::Numbers { subcolumn_mode } = xy.x_mode
        {
            subcolumn_mode.validate()?;
        }
        let slots = sheet.archetype.allocated_cell_slots();
        validate_sheet_cells(slots)?;
        total_cells = validate_project_cells(total_cells, slots)?;
    }
    Ok(())
}

fn load_delimited_with_importer(
    importer: &dyn curvebook_format::registry::Importer,
    path_hint: &Path,
    bytes: &[u8],
    format_tag: &'static str,
) -> Result<LoadedProject, CliError> {
    let stem = path_hint.file_stem().and_then(std::ffi::OsStr::to_str).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Table");
    let opts = ImportOptions {
        sheet_id: SheetId::new("data_1"),
        title: stem.to_owned(),
        table_kind: TableKind::Xy,
        subcolumn_mode: SubcolumnMode::Single,
        csv: None,
        xlsx: None,
    };
    let ds = importer
        .import_bytes(bytes, &opts)?
        .into_data_sheet()
        .ok_or_else(|| CliError::Runtime(format!("delimited importer `{format_tag}` did not return a DataSheet")))?;
    let mut doc = ProjectDocument::new(stem)?;
    let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph_1"), &ds)?;
    graph.initialized = true;
    doc.add_data_sheet(ds)?;
    doc.add_graph_sheet(graph)?;
    Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
}

fn sniff_project_bytes(bytes: &[u8]) -> Result<LoadedProject, CliError> {
    if bytes.starts_with(b"PK\x03\x04") {
        if let Ok(loaded) = load_native_archive(bytes) {
            return Ok(loaded);
        }
        if let Ok(report) = import_xlsx_bytes(bytes) {
            return load_import_report(report, "xlsx");
        }
        return load_import_report(import_prism_bytes(bytes)?, "prism");
    }
    let trimmed = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    match trimmed {
        Some(b'<') => load_import_report(import_pzfx_bytes(bytes)?, "pzfx"),
        Some(b'{') => load_project_json(bytes),
        _ => Err(CliError::Runtime(String::from("unrecognized input file format"))),
    }
}

fn find_sheet_id_by_name<T>(items: &[T], needle: &str, id_of: impl Fn(&T) -> &SheetId, title_of: impl Fn(&T) -> &str) -> Option<SheetId> {
    items
        .iter()
        .find(|item| id_of(item).as_str() == needle)
        .or_else(|| items.iter().find(|item| title_of(item) == needle))
        .or_else(|| items.iter().find(|item| title_of(item).eq_ignore_ascii_case(needle)))
        .map(id_of)
        .cloned()
}

fn resolve_visual_sheet_id(doc: &ProjectDocument, sheet: Option<&str>) -> Result<SheetId, CliError> {
    if let Some(needle) = sheet {
        if let Some(id) = find_sheet_id_by_name(&doc.graph_sheets, needle, |g| &g.id, |g| &g.title) {
            return Ok(id);
        }
        if let Some(id) = find_sheet_id_by_name(&doc.layout_sheets, needle, |l| &l.id, |l| &l.title) {
            return Ok(id);
        }
        return Err(CliError::Runtime(format!("visual sheet `{needle}` was not found among graphs or layouts")));
    }

    if doc.graph_sheets.is_empty()
        && let [only_layout] = doc.layout_sheets.as_slice()
    {
        return Ok(only_layout.id.clone());
    }
    if let Some(g) = doc.graph_sheets.iter().find(|g| g.initialized).or_else(|| doc.graph_sheets.first()) {
        return Ok(g.id.clone());
    }
    if let Some(l) = doc.layout_sheets.first() {
        return Ok(l.id.clone());
    }
    Err(CliError::Runtime(String::from("project contains no graph or layout sheets to render")))
}

fn resolve_tabular_sheet_id(doc: &ProjectDocument, sheet: Option<&str>) -> Result<SheetId, CliError> {
    if let Some(needle) = sheet {
        if let Some(id) = find_sheet_id_by_name(&doc.data_sheets, needle, |d| &d.id, |d| &d.title) {
            return Ok(id);
        }
        if let Some(id) = find_sheet_id_by_name(&doc.result_sheets, needle, |r| &r.id, |r| r.title()) {
            return Ok(id);
        }
        return Err(CliError::Runtime(format!("tabular sheet `{needle}` was not found")));
    }
    if let Some(d) = doc.data_sheets.first() {
        return Ok(d.id.clone());
    }
    if let Some(r) = doc.result_sheets.first() {
        return Ok(r.id.clone());
    }
    Err(CliError::Runtime(String::from("project contains no data or result sheets to export")))
}

fn resolve_json_target(doc: &ProjectDocument, sheet: Option<&str>) -> Result<(ExportTarget, Option<String>), CliError> {
    if let Some(needle) = sheet {
        let found = find_sheet_id_by_name(&doc.result_sheets, needle, |r| &r.id, |r| r.title())
            .or_else(|| find_sheet_id_by_name(&doc.data_sheets, needle, |d| &d.id, |d| &d.title))
            .or_else(|| find_sheet_id_by_name(&doc.info_sheets, needle, |s| &s.id, |s| &s.title));
        let id = found.ok_or_else(|| CliError::Runtime(format!("sheet `{needle}` not found for JSON export")))?;
        let label = id.as_str().to_owned();
        return Ok((ExportTarget::Explicit(id), Some(label)));
    }
    if doc.data_sheets.is_empty()
        && doc.graph_sheets.is_empty()
        && doc.layout_sheets.is_empty()
        && let [res] = doc.result_sheets.as_slice()
    {
        return Ok((ExportTarget::Explicit(res.id.clone()), Some(res.id.as_str().to_owned())));
    }
    Ok((ExportTarget::AutoOrExplicit(None), None))
}

fn build_cli_export_request(req: &ExportRequest<'_>) -> Result<(ExportArtifactRequest, Option<String>), CliError> {
    let dpi = Some(req.dpi.unwrap_or(DEFAULT_RASTER_DPI));
    let (target, sheet_id, format) = match req.format {
        ExportFormat::Svg => {
            let id = resolve_visual_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            (ExportTarget::Explicit(id), Some(label), ExportArtifactFormat::Svg { options: SvgExportOptions::default() })
        }
        ExportFormat::Pdf => {
            let id = resolve_visual_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            let options = PdfExportOptions {
                version: req.pdf_version,
                embedded_truetype_font: req.embedded_truetype_font.map(<[u8]>::to_vec),
                ..PdfExportOptions::default()
            };
            (ExportTarget::Explicit(id), Some(label), ExportArtifactFormat::Pdf { options, all_graphs: false, sheet_ids: None })
        }
        ExportFormat::Emf => {
            let id = resolve_visual_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            (ExportTarget::Explicit(id), Some(label), ExportArtifactFormat::Emf { require_kind: None })
        }
        ExportFormat::Png => {
            let id = resolve_visual_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            (
                ExportTarget::Explicit(id),
                Some(label),
                ExportArtifactFormat::Png { width_px: None, height_px: None, dpi, sizing: RasterSizingPolicy::StrictLimits },
            )
        }
        ExportFormat::Tiff => {
            let id = resolve_visual_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            (
                ExportTarget::Explicit(id),
                Some(label),
                ExportArtifactFormat::Tiff {
                    width_px: None,
                    height_px: None,
                    dpi,
                    compression: TiffCompression::Deflate,
                    sizing: RasterSizingPolicy::StrictLimits,
                },
            )
        }
        ExportFormat::Csv | ExportFormat::Tsv => {
            let id = resolve_tabular_sheet_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            let options = if req.format == ExportFormat::Tsv { CsvOptions::tsv() } else { CsvOptions::csv() };
            (
                ExportTarget::Explicit(id),
                Some(label),
                ExportArtifactFormat::Csv { options, tab_id: None, tab_index: req.tab_index, tidy: req.tab_index.is_none() },
            )
        }
        ExportFormat::Json => {
            let (target, label) = resolve_json_target(req.doc, req.sheet)?;
            (target, label, ExportArtifactFormat::Json { pretty: req.pretty })
        }
        ExportFormat::CurveBook | ExportFormat::CurveBookDir => {
            return Err(CliError::Runtime(format!("format `{}` is exported via FormatRegistry", req.format.as_str())));
        }
    };
    Ok((ExportArtifactRequest { target, format }, sheet_id))
}

#[cfg(test)]
fn parse_convert_args(args: &[String]) -> Result<ConvertArgs, CliError> {
    parse_export_or_convert_args(args, "convert")
}

fn parse_export_or_convert_args(args: &[String], cmd_name: &str) -> Result<ConvertArgs, CliError> {
    let mut positionals = Vec::new();
    let mut explicit_out = None;
    let mut format = None;
    let mut sheet = None;
    let mut xlsx_sheets = None;
    let mut dpi = None;
    let mut pdf_version = PdfVersion::Pdf17;
    let mut ttf_font = None;
    let mut json = false;
    let mut idx = 0;

    while idx < args.len() {
        let Some(arg) = args.get(idx) else {
            break;
        };
        if arg == "--json" {
            json = true;
            idx += 1;
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--out", "--out requires an output file path")? {
            explicit_out = Some(val);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "-o", "-o requires an output file path")? {
            explicit_out = Some(val);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--format", "--format requires an export format name")? {
            format = Some(parse_export_format(&val)?);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--sheet", "--sheet requires a sheet ID or title")? {
            sheet = Some(val);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--xlsx-sheets", "--xlsx-sheets requires comma-separated worksheet names")? {
            let parsed_names: Vec<String> = val.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect();
            if parsed_names.is_empty() {
                return Err(CliError::Usage(String::from("--xlsx-sheets requires at least one non-empty sheet name")));
            }
            xlsx_sheets = Some(parsed_names);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--dpi", "--dpi requires an integer in 18..=2400")? {
            dpi = Some(parse_dpi_value(&val)?);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--pdf-version", "--pdf-version requires `1.4` or `1.7`")? {
            pdf_version = parse_pdf_version(&val)?;
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--ttf-font", "--ttf-font requires a TrueType font file path")? {
            if val.trim().is_empty() {
                return Err(CliError::Usage(String::from("--ttf-font requires a non-empty font file path")));
            }
            ttf_font = Some(val);
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}` for `{cmd_name}`")));
        } else {
            positionals.push(arg.clone());
            idx += 1;
        }
    }

    let (input_path, output_path) = resolve_io_positionals(&mut positionals, explicit_out, cmd_name)?;
    Ok(ConvertArgs { input_path, output_path, format, sheet, xlsx_sheets, dpi, pdf_version, ttf_font, json })
}

fn resolve_io_positionals(positionals: &mut Vec<String>, explicit_out: Option<String>, cmd_name: &str) -> Result<(String, String), CliError> {
    if let Some(out) = explicit_out {
        if positionals.len() != 1 {
            return Err(CliError::Usage(format!(
                "usage: curvebook-cli {cmd_name} <in> --out <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]"
            )));
        }
        let Some(input) = positionals.pop() else {
            return Err(CliError::Usage(String::from("missing <in> argument")));
        };
        return Ok((input, out));
    }

    if positionals.len() != 2 {
        return Err(CliError::Usage(format!(
            "usage: curvebook-cli {cmd_name} <in> <out> [--format <fmt>] [--sheet <id>] [--dpi <u16>] [--pdf-version <1.4|1.7>] [--ttf-font <path>] [--json]"
        )));
    }
    let Some(output_path) = positionals.pop() else {
        return Err(CliError::Usage(String::from("missing <out> argument")));
    };
    let Some(input_path) = positionals.pop() else {
        return Err(CliError::Usage(String::from("missing <in> argument")));
    };
    Ok((input_path, output_path))
}

fn consume_flag_opt(args: &[String], idx: &mut usize, flag: &str, missing_msg: &str) -> Result<Option<String>, CliError> {
    let Some(arg) = args.get(*idx) else {
        return Ok(None);
    };
    if arg == flag {
        *idx += 1;
        let val = args.get(*idx).ok_or_else(|| CliError::Usage(String::from(missing_msg)))?;
        *idx += 1;
        return Ok(Some(val.clone()));
    }
    if let Some(rest) = arg.strip_prefix(flag).and_then(|s| s.strip_prefix('=')) {
        *idx += 1;
        return Ok(Some(rest.to_owned()));
    }
    Ok(None)
}

fn parse_dpi_value(raw: &str) -> Result<u16, CliError> {
    let dpi: u16 =
        raw.parse().map_err(|_| CliError::Usage(format!("invalid --dpi `{raw}` (expected integer in {MIN_RASTER_DPI}..={MAX_RASTER_DPI})")))?;
    if !(MIN_RASTER_DPI..=MAX_RASTER_DPI).contains(&dpi) {
        return Err(CliError::Usage(format!("--dpi must be in {MIN_RASTER_DPI}..={MAX_RASTER_DPI} (got {dpi})")));
    }
    Ok(dpi)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use curvebook_doc::{
        Cell, DataSheet, GraphSheet, LayoutItem, LayoutSheet, LayoutSpec, ProjectDocument, RectPt, SheetId, SubcolumnMode, TableArchetype, XyTable,
    };
    use curvebook_format::{is_unpacked_project_dir, save_project_file};
    use curvebook_pdf::PdfVersion;

    use super::{
        ExportFormat, detect_export_format_from_path, load_project_from_path, parse_convert_args, parse_export_format, parse_pdf_version,
        run_convert, run_export,
    };
    use crate::cmd_info;
    use crate::cmd_run_batch;
    use crate::error::CliError;

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("cb-cli-convert-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn build_sample_doc_with_graph_and_layout() -> Result<ProjectDocument, Box<dyn std::error::Error>> {
        let mut doc = ProjectDocument::new("CLI Unpacked & EMF Study")?;
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3)?;
        xy.x_cells[0][0] = Cell::number(1.0);
        xy.x_cells[1][0] = Cell::number(2.0);
        xy.x_cells[2][0] = Cell::number(3.0);
        xy.data_sets[0].cells[0][0] = Cell::number(2.5);
        xy.data_sets[0].cells[1][0] = Cell::number(5.0);
        xy.data_sets[0].cells[2][0] = Cell::number(7.5);
        let ds = DataSheet::new("data-1", "Dose Response", TableArchetype::Xy(xy))?;

        let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &ds)?;
        graph.title = String::from("Response Plot");
        graph.initialized = true;

        doc.add_data_sheet(ds)?;
        doc.add_graph_sheet(graph)?;

        let mut layout_spec = LayoutSpec::default();
        layout_spec.items.push(LayoutItem::LiveGraph {
            id: String::from("item-1"),
            graph_sheet_id: SheetId::new("graph-1"),
            bounds_pt: RectPt::new(24.0, 24.0, 220.0, 160.0),
            scale_percent: 100.0,
            badge_override: None,
        });
        let layout = LayoutSheet::new("layout-1", "Figure 1", layout_spec)?;
        doc.add_layout_sheet(layout)?;
        Ok(doc)
    }

    fn assert_emf_magic(bytes: &[u8]) {
        assert!(bytes.len() >= 88, "EMF header shorter than 88 bytes ({})", bytes.len());
        let record_type = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(record_type, 1, "expected EMR_HEADER record type 1");
        let magic = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(magic, 0x464D_4520, "expected EMF header signature 0x464D4520, got {magic:#010X}");
    }

    #[test]
    fn parse_pdf_version_accepts_1_4_and_1_7_only() {
        assert_eq!(parse_pdf_version("1.4").unwrap(), PdfVersion::Pdf14);
        assert_eq!(parse_pdf_version("1.7").unwrap(), PdfVersion::Pdf17);
        assert_eq!(parse_pdf_version(" 1.4 ").unwrap(), PdfVersion::Pdf14);
        assert!(matches!(parse_pdf_version("1.5"), Err(CliError::Usage(_))));
    }

    #[test]
    fn parse_convert_args_handles_pdf_version_and_ttf_font_flags() {
        let defaults = parse_convert_args(&[String::from("in.curvebook"), String::from("out.pdf")]).unwrap();
        assert_eq!(defaults.pdf_version, PdfVersion::Pdf17);
        assert!(defaults.ttf_font.is_none());

        let custom = parse_convert_args(&[
            String::from("in.curvebook"),
            String::from("out.pdf"),
            String::from("--pdf-version"),
            String::from("1.4"),
            String::from("--ttf-font=fonts/sans.ttf"),
        ])
        .unwrap();
        assert_eq!(custom.pdf_version, PdfVersion::Pdf14);
        assert_eq!(custom.ttf_font.as_deref(), Some("fonts/sans.ttf"));
    }

    #[test]
    fn format_detection_handles_unpacked_dirs_and_emf() {
        assert_eq!(parse_export_format("emf").unwrap(), ExportFormat::Emf);
        assert_eq!(parse_export_format("curvebook.d").unwrap(), ExportFormat::CurveBookDir);
        assert_eq!(detect_export_format_from_path(std::path::Path::new("figure.emf")).unwrap(), ExportFormat::Emf);
        assert_eq!(detect_export_format_from_path(std::path::Path::new("study.curvebook.d")).unwrap(), ExportFormat::CurveBookDir);
    }

    #[test]
    fn unpacked_dir_round_trip_and_emf_export_work_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
        let dir = unique_temp_dir("emf-unpacked");
        let doc = build_sample_doc_with_graph_and_layout()?;
        let src_zip = dir.join("source.curvebook");
        save_project_file(&src_zip, &doc)?;

        // 1. Convert archive -> `.curvebook.d` unpacked directory, inspect with `info`, then convert back to `.curvebook`.
        let unpacked_dir = dir.join("study.curvebook.d");
        run_convert(&[src_zip.display().to_string(), unpacked_dir.display().to_string(), String::from("--json")])?;
        assert!(is_unpacked_project_dir(&unpacked_dir));
        assert!(unpacked_dir.join("manifest.json").is_file());

        cmd_info::run_info(&[unpacked_dir.display().to_string(), String::from("--json")])?;
        let (reloaded, size_bytes) = load_project_from_path(&unpacked_dir, false)?;
        assert_eq!(reloaded.format_tag, "curvebook.d");
        assert_eq!(reloaded.doc.title, doc.title);
        assert_eq!(reloaded.doc.data_sheets.len(), 1);
        assert!(size_bytes > 0);

        let repacked_zip = dir.join("repacked.curvebook");
        run_convert(&[unpacked_dir.display().to_string(), repacked_zip.display().to_string()])?;
        let (repacked_loaded, _) = load_project_from_path(&repacked_zip, false)?;
        assert_eq!(repacked_loaded.doc.title, doc.title);
        assert_eq!(repacked_loaded.doc.graph_sheets.len(), 1);

        // 2. Direct `curvebook-cli export` to `.emf` from the unpacked directory using positional and `--out` forms.
        let graph_emf = dir.join("graph-1.emf");
        run_export(&[
            unpacked_dir.display().to_string(),
            graph_emf.display().to_string(),
            String::from("--sheet"),
            String::from("graph-1"),
            String::from("--json"),
        ])?;
        assert_emf_magic(&std::fs::read(&graph_emf)?);

        let layout_emf = dir.join("layout-1.emf");
        run_export(&[
            unpacked_dir.display().to_string(),
            String::from("--out"),
            layout_emf.display().to_string(),
            String::from("--sheet"),
            String::from("layout-1"),
        ])?;
        assert_emf_magic(&std::fs::read(&layout_emf)?);

        // 3. Scripted engine `graph.exportEmf` & `layout.exportEmf` via `curvebook-cli run` from `.curvebook.d` base.
        let script_graph_emf = dir.join("script_graph.emf");
        let script_layout_emf = dir.join("script_layout.emf");
        let script_path = dir.join("export_emf.jsonl");
        let script_lines = format!(
            "{}\n{}\n{}\n",
            serde_json::json!({ "id": "c1", "cmd": "project.open", "params": { "path": unpacked_dir.display().to_string() } }),
            serde_json::json!({ "id": "c2", "cmd": "graph.exportEmf", "params": { "sheet_id": "graph-1", "path": script_graph_emf.display().to_string() } }),
            serde_json::json!({ "id": "c3", "cmd": "layout.exportEmf", "params": { "sheet_id": "layout-1", "path": script_layout_emf.display().to_string() } }),
        );
        std::fs::write(&script_path, script_lines)?;
        cmd_run_batch::run_script(&[script_path.display().to_string(), String::from("--json")])?;
        assert_emf_magic(&std::fs::read(&script_graph_emf)?);
        assert_emf_magic(&std::fs::read(&script_layout_emf)?);

        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    #[allow(clippy::panic)]
    fn run_guarded_cli_converts_panic_into_internal_panic_cli_error() {
        let err = super::run_guarded_cli::<(), _>("cli.test_export", || panic!("simulated svg panic"))
            .expect_err("panic must be caught by run_guarded_cli");
        assert_eq!(err.exit_code(), 1);
        let env = err.to_json_envelope();
        assert_eq!(env.error.code, curvebook_doc::ErrorCode::InternalPanic);
        assert!(env.error.message.contains("simulated svg panic"));

        let usage = super::run_guarded_cli::<(), _>("cli.test_usage", || Err(crate::error::CliError::Usage(String::from("bad flag"))))
            .expect_err("non-panic CliError must pass through unchanged");
        assert_eq!(usage.exit_code(), 2);
    }

    #[test]
    fn r2_s12_02_unpacked_path_rejects_dot_and_arbitrary_dirs() {
        let base = std::env::temp_dir().join(format!("cb_r2_s12_02_unit_{}", std::process::id()));
        let plain = base.join("workspace");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&plain).unwrap();

        assert!(!super::is_unpacked_project_path(std::path::Path::new(".")));
        assert!(!super::is_unpacked_project_path(&plain));
        assert!(super::detect_export_format_from_path(&plain).is_err());

        let suffixed = base.join("study.curvebook.d");
        assert!(super::is_unpacked_project_path(&suffixed));
        assert_eq!(super::detect_export_format_from_path(&suffixed).unwrap(), super::ExportFormat::CurveBookDir);

        let _ = std::fs::remove_dir_all(&base);
    }
}
