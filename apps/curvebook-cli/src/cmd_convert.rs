//! Document loading helpers and `curvebook-cli convert <in> <out>` implementation.

use std::fs;
use std::path::Path;

use curvebook_doc::{ColumnEqualsFilter, ErrorCode, ProjectDocument, ResultStatus, SheetId};
use curvebook_engine::EngineError;
use curvebook_engine::services::{
    self, DEFAULT_EXPORT_DPI, DocumentExportFormat, ExportArtifactRequest, ExportOptionsSpec, ExportTarget, RasterSizingPolicy,
    export_document_artifact, resolve_json_target_by_name_or_id, resolve_tabular_sheet_by_name_or_id, resolve_visual_sheet_by_name_or_id,
};
pub use curvebook_engine::services::{LoadedProject, is_unpacked_project_path, pdf_version_str};
pub(crate) use curvebook_engine::services::{MAX_RASTER_DPI, MIN_RASTER_DPI};
use curvebook_format::XlsxImportOptions;
use curvebook_format::registry::{ExportOptions, ExportSource, FormatRegistry};
use curvebook_pdf::PdfVersion;
use curvebook_platform::fs::write_atomic;
use curvebook_render::TiffCompression;
use serde_json::json;

use crate::error::CliError;

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
    /// Figure warnings as `code: message` lines (for example a fitted curve omitted because its fit
    /// failed or is pending recalculation). Callers are responsible for printing them.
    pub warnings: Vec<String>,
}

struct ConvertArgs {
    input_path: String,
    output_path: String,
    format: Option<ExportFormat>,
    sheet: Option<String>,
    xlsx_sheets: Option<Vec<String>>,
    xlsx_range: Option<String>,
    xlsx_skip_rows: Option<usize>,
    xlsx_fill_merged: bool,
    xlsx_split_by_column: Option<usize>,
    xlsx_row_match: Option<ColumnEqualsFilter>,
    xlsx_extract_preamble: bool,
    xlsx_no_header: bool,
    dpi: Option<u16>,
    pdf_version: PdfVersion,
    ttf_font: Option<String>,
    json: bool,
}

impl ConvertArgs {
    fn build_xlsx_options(&self) -> Option<XlsxImportOptions> {
        let any_xlsx = self.xlsx_sheets.is_some()
            || self.xlsx_range.is_some()
            || self.xlsx_skip_rows.is_some()
            || self.xlsx_fill_merged
            || self.xlsx_split_by_column.is_some()
            || self.xlsx_row_match.is_some()
            || self.xlsx_extract_preamble
            || self.xlsx_no_header;
        if !any_xlsx {
            return None;
        }
        let has_header = !self.xlsx_no_header;
        Some(XlsxImportOptions {
            sheet_names: self.xlsx_sheets.clone(),
            has_header,
            header_rows: usize::from(has_header),
            cell_range: self.xlsx_range.clone(),
            skip_rows: self.xlsx_skip_rows.unwrap_or(0),
            fill_merged_cells: self.xlsx_fill_merged,
            split_by_column: self.xlsx_split_by_column,
            row_match: self.xlsx_row_match.clone(),
            extract_preamble_info: self.xlsx_extract_preamble,
            ..XlsxImportOptions::default()
        })
    }
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

        let xlsx_options = parsed.build_xlsx_options();
        let (mut loaded, _input_bytes) = load_project_from_path_with_xlsx_options(in_path, true, xlsx_options.as_ref())?;
        let format = match parsed.format {
            Some(fmt) => fmt,
            None => detect_export_format_from_path(out_path)?,
        };

        let dirty_ids: Vec<SheetId> =
            loaded.doc.result_sheets.iter().filter(|res| matches!(res.status, ResultStatus::Dirty)).map(|res| res.id.clone()).collect();
        if !dirty_ids.is_empty() {
            if matches!(format, ExportFormat::CurveBook | ExportFormat::CurveBookDir) {
                let ids = dirty_ids.iter().map(SheetId::as_str).collect::<Vec<_>>().join(", ");
                loaded.warnings.push(format!("pending_result_sheet: {} analysis sheet(s) pending recalculation ({ids})", dirty_ids.len()));
            } else {
                match curvebook_engine::dag::recalculate_dirty_with_report(&mut loaded.doc, &dirty_ids) {
                    Ok(report) => {
                        if !report.failed_result_sheets.is_empty() {
                            let failed = report.failed_result_sheets.iter().map(SheetId::as_str).collect::<Vec<_>>().join(", ");
                            loaded.warnings.push(format!(
                                "failed_recalculation: recalculated pending analyses before export; {} failed ({failed})",
                                report.failed_result_sheets.len()
                            ));
                        }
                    }
                    Err(err) => {
                        loaded.warnings.push(recalculation_failed_warning(format, &err));
                    }
                }
            }
        }

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
                    "figure_warnings": summary.warnings,
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
                    "figure_warnings": summary.warnings,
                })
            };
            let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            println!("{text}");
        } else {
            for warning in &loaded.warnings {
                eprintln!("warning: {warning}");
            }
            for warning in &summary.warnings {
                eprintln!("warning: {warning}");
            }
            println!(
                "{verb} `{}` ({}) -> `{}` ({}, {} bytes)",
                parsed.input_path, loaded.format_tag, summary.out_path, summary.format, summary.bytes_written
            );
        }
        Ok(())
    })
}

/// Warning for a failed pre-export recalculation; the consequence wording matches the export kind
/// (tabular data exports still write the stale table, figure exports omit it).
fn recalculation_failed_warning(format: ExportFormat, err: &dyn std::fmt::Display) -> String {
    let consequence = if matches!(format, ExportFormat::Csv | ExportFormat::Tsv | ExportFormat::Json) {
        "exported data may be stale"
    } else {
        "stale results omitted"
    };
    format!("pending_recalculation_failed: could not recalculate pending analyses before export ({err}); {consequence}")
}

/// Parse a PDF specification version string (`1.4` or `1.7`).
pub fn parse_pdf_version(raw: &str) -> Result<PdfVersion, CliError> {
    services::parse_pdf_version(raw).map_err(|_| CliError::Usage(format!("invalid --pdf-version `{}` (expected `1.4` or `1.7`)", raw.trim())))
}

/// Read and validate TrueType SFNT bytes from `path`.
pub fn load_validated_ttf_font(path: &Path) -> Result<Vec<u8>, CliError> {
    services::load_validated_ttf_font_bytes(path).map_err(|err| match err.code {
        ErrorCode::ResourceLimit => CliError::from(err),
        ErrorCode::IoError => CliError::Runtime(format!("failed to read TrueType font `{}`: {}", path.display(), err.message())),
        _ => CliError::Runtime(err.message().to_owned()),
    })
}

/// Compute total file byte size inside an unpacked directory tree.
pub fn dir_size_bytes(root: &Path) -> Result<usize, CliError> {
    services::dir_size_bytes(root).map_err(|err| CliError::Runtime(err.message().to_owned()))
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

fn map_project_load_error(path: &Path, err: EngineError) -> CliError {
    if err.code == ErrorCode::UnsupportedFormat && err.message().contains("delimited CSV/TSV") {
        return CliError::Usage(err.message().to_owned());
    }
    if err.code == ErrorCode::IoError || (err.code == ErrorCode::BadInput && err.message().starts_with("invalid file ")) {
        return CliError::Runtime(format!("failed to read `{}`: {}", path.display(), err.message()));
    }
    CliError::from(err)
}

/// Load a [`ProjectDocument`] from a file path or unpacked `.curvebook.d` directory, returning `(loaded, input_bytes)`.
pub fn load_project_from_path(path: &Path, allow_delimited: bool) -> Result<(LoadedProject, usize), CliError> {
    load_project_from_path_with_xlsx_sheets(path, allow_delimited, None)
}

/// Load a [`ProjectDocument`] from a file path or unpacked `.curvebook.d` directory, optionally
/// restricting `.xlsx` imports to specific worksheet tab names.
#[allow(dead_code)]
pub fn load_project_from_path_with_xlsx_sheets(
    path: &Path,
    allow_delimited: bool,
    xlsx_sheets: Option<&[String]>,
) -> Result<(LoadedProject, usize), CliError> {
    services::load_project_from_path_with_xlsx_sheets(path, allow_delimited, xlsx_sheets).map_err(|err| map_project_load_error(path, err))
}

/// Load a [`ProjectDocument`] from a file path or unpacked `.curvebook.d` directory, optionally
/// forwarding full [`XlsxImportOptions`] for `.xlsx` workbooks.
pub fn load_project_from_path_with_xlsx_options(
    path: &Path,
    allow_delimited: bool,
    xlsx_options: Option<&XlsxImportOptions>,
) -> Result<(LoadedProject, usize), CliError> {
    services::load_project_from_path_with_xlsx_options(path, allow_delimited, xlsx_options).map_err(|err| map_project_load_error(path, err))
}

/// Write an exported sheet or whole document to `req.out_path` using atomic writes and [`FormatRegistry`].
pub fn export_from_document(req: &ExportRequest<'_>) -> Result<ExportOutcomeSummary, CliError> {
    run_guarded_cli("cli.export_from_document", || {
        if let Some(dpi) = req.dpi {
            let _ = services::validate_dpi(dpi).map_err(|err| CliError::Usage(err.message().to_owned()))?;
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
                warnings: Vec::new(),
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

        let mut warnings: Vec<String> = artifact.warnings.iter().map(|w| format!("{}: {}", w.code, w.message)).collect();
        if matches!(req.format, ExportFormat::Csv | ExportFormat::Tsv | ExportFormat::Json) {
            match sheet_id.as_deref() {
                Some(sid) => {
                    if let Some(res) = req.doc.result_sheet(&SheetId::new(sid))
                        && matches!(res.status, ResultStatus::Dirty)
                    {
                        warnings.push(format!("pending_result_sheet: result sheet '{sid}' is pending recalculation; exported data may be stale"));
                    }
                }
                None => {
                    for res in &req.doc.result_sheets {
                        if matches!(res.status, ResultStatus::Dirty) {
                            warnings.push(format!(
                                "pending_result_sheet: result sheet '{}' is pending recalculation; exported data may be stale",
                                res.id
                            ));
                        }
                    }
                }
            }
        }
        Ok(ExportOutcomeSummary {
            out_path: req.out_path.to_string_lossy().into_owned(),
            format: req.format.as_str(),
            sheet_id,
            bytes_written: artifact.bytes.len(),
            warnings,
        })
    })
}

fn build_cli_export_request(req: &ExportRequest<'_>) -> Result<(ExportArtifactRequest, Option<String>), CliError> {
    let doc_format = match req.format {
        ExportFormat::Svg => DocumentExportFormat::Svg,
        ExportFormat::Pdf => DocumentExportFormat::Pdf,
        ExportFormat::Emf => DocumentExportFormat::Emf,
        ExportFormat::Png => DocumentExportFormat::Png,
        ExportFormat::Tiff => DocumentExportFormat::Tiff,
        ExportFormat::Csv => DocumentExportFormat::Csv,
        ExportFormat::Tsv => DocumentExportFormat::Tsv,
        ExportFormat::Json => DocumentExportFormat::Json,
        ExportFormat::CurveBook | ExportFormat::CurveBookDir => {
            return Err(CliError::Runtime(format!("format `{}` is exported via FormatRegistry", req.format.as_str())));
        }
    };
    let mut spec = ExportOptionsSpec::new(doc_format);
    spec.pretty = req.pretty;
    spec.dpi = Some(req.dpi.unwrap_or(DEFAULT_EXPORT_DPI));
    spec.sizing = RasterSizingPolicy::StrictLimits;
    spec.tiff_compression = TiffCompression::Deflate;
    spec.pdf_version = req.pdf_version;
    spec.embedded_ttf_font = req.embedded_truetype_font.map(<[u8]>::to_vec);
    spec.tab_index = req.tab_index;
    spec.tidy = req.tab_index.is_none();

    let sheet_label = match req.format {
        ExportFormat::Svg | ExportFormat::Pdf | ExportFormat::Emf | ExportFormat::Png | ExportFormat::Tiff => {
            let id = resolve_visual_sheet_by_name_or_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            spec.sheet_id = Some(id);
            Some(label)
        }
        ExportFormat::Csv | ExportFormat::Tsv => {
            let id = resolve_tabular_sheet_by_name_or_id(req.doc, req.sheet)?;
            let label = id.as_str().to_owned();
            spec.sheet_id = Some(id);
            Some(label)
        }
        ExportFormat::Json => {
            let (target, label) = resolve_json_target_by_name_or_id(req.doc, req.sheet)?;
            if let ExportTarget::Explicit(id) = target {
                spec.sheet_id = Some(id);
            }
            label
        }
        ExportFormat::CurveBook | ExportFormat::CurveBookDir => None,
    };
    let artifact_req = spec.into_artifact_request(req.doc, false)?;
    Ok((artifact_req, sheet_label))
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
    let mut xlsx_range = None;
    let mut xlsx_skip_rows = None;
    let mut xlsx_fill_merged = false;
    let mut xlsx_split_by_column = None;
    let mut xlsx_row_match = None;
    let mut xlsx_extract_preamble = false;
    let mut xlsx_no_header = false;
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
        } else if arg == "--xlsx-fill-merged" {
            xlsx_fill_merged = true;
            idx += 1;
        } else if arg == "--xlsx-extract-preamble" {
            xlsx_extract_preamble = true;
            idx += 1;
        } else if arg == "--xlsx-no-header" {
            xlsx_no_header = true;
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
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--xlsx-range", "--xlsx-range requires an A1 range such as B4:M11")? {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                return Err(CliError::Usage(String::from("--xlsx-range requires a non-empty A1 range")));
            }
            xlsx_range = Some(trimmed.to_owned());
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--xlsx-skip-rows", "--xlsx-skip-rows requires a non-negative integer")? {
            let n = val
                .trim()
                .parse::<usize>()
                .map_err(|_| CliError::Usage(format!("invalid --xlsx-skip-rows `{val}` (expected a non-negative integer)")))?;
            xlsx_skip_rows = Some(n);
        } else if let Some(val) =
            consume_flag_opt(args, &mut idx, "--xlsx-split-by-column", "--xlsx-split-by-column requires a zero-based column index")?
        {
            let col = val
                .trim()
                .parse::<usize>()
                .map_err(|_| CliError::Usage(format!("invalid --xlsx-split-by-column `{val}` (expected a non-negative integer)")))?;
            xlsx_split_by_column = Some(col);
        } else if let Some(val) = consume_flag_opt(args, &mut idx, "--xlsx-row-match", "--xlsx-row-match requires `<col>=<value>`")? {
            let Some((col_raw, eq_raw)) = val.split_once('=') else {
                return Err(CliError::Usage(format!("invalid --xlsx-row-match `{val}` (expected `<col>=<value>`)")));
            };
            let column = col_raw
                .trim()
                .parse::<usize>()
                .map_err(|_| CliError::Usage(format!("invalid --xlsx-row-match column `{col_raw}` (expected a non-negative integer)")))?;
            xlsx_row_match = Some(ColumnEqualsFilter::new(column, eq_raw.trim()));
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
    Ok(ConvertArgs {
        input_path,
        output_path,
        format,
        sheet,
        xlsx_sheets,
        xlsx_range,
        xlsx_skip_rows,
        xlsx_fill_merged,
        xlsx_split_by_column,
        xlsx_row_match,
        xlsx_extract_preamble,
        xlsx_no_header,
        dpi,
        pdf_version,
        ttf_font,
        json,
    })
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
    services::parse_dpi_str(raw).map_err(|err| CliError::Usage(err.message().to_owned()))
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

    /// `R19.5`: a graph whose fit is pending recalculation exports without the cached curve and
    /// `convert` reports the omission as a figure warning.
    #[test]
    fn export_reports_pending_fit_as_figure_warning() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, CurveSamplePoint, FittedCurveOverlay, NonlinearFitSpec, RegressionOverlaySpec, ResultSheet, ResultStatus,
        };

        let mut doc = build_sample_doc_with_graph_and_layout().unwrap();
        let spec = AnalysisSpec::new("fit-1", "data-1", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("linear"))).unwrap();
        let mut res = ResultSheet::new(spec).unwrap();
        res.status = ResultStatus::Dirty;
        let pt = |x: f64, y: f64| CurveSamplePoint { x, y, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None };
        res.fitted_curves.push(FittedCurveOverlay {
            data_set_index: 0,
            equation_id: String::from("linear"),
            converged: true,
            ambiguous: false,
            points: vec![pt(1.0, 2.5), pt(3.0, 7.5)],
        });
        doc.add_result_sheet(res).unwrap();
        doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
            result_sheet_id: SheetId::new("fit-1"),
            sample_segments: 20,
            show_confidence_band: false,
            show_prediction_band: false,
            shaded_ribbon: false,
        });

        let dir = unique_temp_dir("pending-fit");
        let out_path = dir.join("figure.svg");
        let summary = super::export_from_document(&super::ExportRequest {
            doc: &doc,
            out_path: &out_path,
            format: ExportFormat::Svg,
            sheet: Some("graph-1"),
            dpi: None,
            pretty: true,
            tab_index: None,
            pdf_version: PdfVersion::default(),
            embedded_truetype_font: None,
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            summary.warnings.iter().any(|w| w.starts_with("pending_regression_result: ")),
            "expected a pending-fit figure warning, got {:?}",
            summary.warnings
        );
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

    #[test]
    fn gap_01_03_cli_convert_recalculates_dirty_analysis_and_warns_on_direct_dirty_tabular_export() {
        use curvebook_doc::{
            AnalysisKind, AnalysisSpec, DescriptiveSpec, EmbeddedTableStyle, GraphAnnotation, RectPt as DocRectPt, ResultCell, ResultSheet,
            ResultStatus, ResultTab, ResultTabKind,
        };

        let mut doc = build_sample_doc_with_graph_and_layout().unwrap();
        let spec = AnalysisSpec::new("res-desc", "data-1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).unwrap();
        let mut res = ResultSheet::new(spec).unwrap();
        res.status = ResultStatus::Dirty;
        res.tabs.push(ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("Value")],
            row_labels: vec![String::from("Stale")],
            rows: vec![vec![ResultCell::Text(String::from("STALECACHED999"))]],
            footnotes: Vec::new(),
        });
        doc.add_result_sheet(res).unwrap();
        doc.graph_sheet_mut(&SheetId::new("graph-1")).unwrap().spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("res-desc"),
            tab_id: None,
            bounds_pt: DocRectPt::new(180.0, 20.0, 120.0, 50.0),
            selected_rows: None,
            selected_cols: None,
            style: EmbeddedTableStyle::default(),
        });

        let dir = unique_temp_dir("gap-01-03-dirty-convert");
        // 1. Direct `export_from_document` on Dirty doc: SVG omits stale table with `pending_embedded_result_table`,
        //    and CSV warns `pending_result_sheet`.
        let direct_svg = dir.join("direct.svg");
        let svg_summary = super::export_from_document(&super::ExportRequest {
            doc: &doc,
            out_path: &direct_svg,
            format: ExportFormat::Svg,
            sheet: Some("graph-1"),
            dpi: None,
            pretty: true,
            tab_index: None,
            pdf_version: PdfVersion::default(),
            embedded_truetype_font: None,
        })
        .unwrap();
        assert!(svg_summary.warnings.iter().any(|w| w.starts_with("pending_embedded_result_table: ")), "warnings: {:?}", svg_summary.warnings);
        let direct_svg_text = std::fs::read_to_string(&direct_svg).unwrap();
        assert!(!direct_svg_text.contains("STALECACHED999"), "stale cached cell leaked into SVG");

        let direct_csv = dir.join("direct.csv");
        let csv_summary = super::export_from_document(&super::ExportRequest {
            doc: &doc,
            out_path: &direct_csv,
            format: ExportFormat::Csv,
            sheet: Some("res-desc"),
            dpi: None,
            pretty: true,
            tab_index: None,
            pdf_version: PdfVersion::default(),
            embedded_truetype_font: None,
        })
        .unwrap();
        assert!(csv_summary.warnings.iter().any(|w| w.starts_with("pending_result_sheet: ")), "csv warnings: {:?}", csv_summary.warnings);

        // 2. `run_convert` on a saved archive with Dirty `res-desc` recalculates before exporting SVG,
        //    so the exported SVG replaces `STALECACHED999` with freshly computed DescriptiveStats values.
        let src_archive = dir.join("dirty_study.curvebook");
        save_project_file(&src_archive, &doc).unwrap();
        let converted_svg = dir.join("recalculated.svg");
        run_convert(&[src_archive.display().to_string(), converted_svg.display().to_string(), String::from("--sheet"), String::from("graph-1")])
            .unwrap();
        let converted_svg_text = std::fs::read_to_string(&converted_svg).unwrap();
        assert!(!converted_svg_text.contains("STALECACHED999"), "recalculated convert SVG must not contain stale cached text");
        assert!(converted_svg_text.contains("Mean") || converted_svg_text.contains("5"), "recalculated convert SVG should render fresh stats");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn r0_36_recalculation_failed_warning_wording_matches_export_kind() {
        let data = super::recalculation_failed_warning(ExportFormat::Csv, &"boom");
        assert!(data.ends_with("exported data may be stale"), "{data}");
        assert!(!data.contains("omitted"), "{data}");
        let fig = super::recalculation_failed_warning(ExportFormat::Svg, &"boom");
        assert!(fig.ends_with("stale results omitted"), "{fig}");
    }
}
