//! Document loading helpers and `lumencraft-cli convert <in> <out>` implementation.

use std::fs;
use std::path::Path;

use lumencraft_chart::{ChartScene, compile_graph, compile_layout};
use lumencraft_doc::{DataSheet, GraphSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode, TableKind};
use lumencraft_format::{
    ArchiveKind, CsvOptions, ImportReport, export_datasheet_to_csv, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv,
    import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes, open_archive_bytes, save_project_bytes,
};
use lumencraft_pdf::{PdfExportOptions, render_pdf};
use lumencraft_render::{RenderTargetSpec, TiffCompression, encode_png, encode_tiff, render_scene};
use lumencraft_svg::{SvgExportOptions, render_svg};
use serde_json::json;

use crate::error::CliError;

/// Default raster DPI used when `--dpi` is omitted on PNG/TIFF export.
const DEFAULT_RASTER_DPI: u16 = 300;

/// Maximum raster canvas edge in pixels (`16_384`).
const MAX_RASTER_EDGE_PX: f64 = 16_384.0;

/// Loaded project document paired with its detected container format tag and import warnings.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub doc: ProjectDocument,
    pub format_tag: &'static str,
    pub warnings: Vec<String>,
}

/// Supported target export formats for `convert` and batch manifest `exports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Lumencraft,
    Svg,
    Pdf,
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
            Self::Lumencraft => "lumencraft",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
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
    sheet: Option<String>,
    dpi: Option<u16>,
    json: bool,
}

enum VisualTarget<'a> {
    Graph(&'a GraphSheet),
    Layout(&'a lumencraft_doc::LayoutSheet),
}

enum TabularTarget<'a> {
    Data(&'a DataSheet),
    Result(&'a ResultSheet),
}

/// Execute `lumencraft-cli convert <in> <out> [--sheet <id>] [--dpi <u16>] [--json]`.
pub fn run_convert(args: &[String]) -> Result<(), CliError> {
    let parsed = parse_convert_args(args)?;
    let in_path = Path::new(&parsed.input_path);
    let out_path = Path::new(&parsed.output_path);

    let bytes = fs::read(in_path).map_err(|err| CliError::Runtime(format!("failed to read `{}`: {err}", in_path.display())))?;
    let loaded = load_project_from_bytes(in_path, &bytes, true)?;
    let format = detect_export_format_from_path(out_path)?;

    let summary = export_from_document(&ExportRequest {
        doc: &loaded.doc,
        out_path,
        format,
        sheet: parsed.sheet.as_deref(),
        dpi: parsed.dpi,
        pretty: true,
        tab_index: None,
    })?;

    if parsed.json {
        let payload = json!({
            "ok": true,
            "input": parsed.input_path,
            "input_format": loaded.format_tag,
            "output": summary.out_path,
            "format": summary.format,
            "sheet_id": summary.sheet_id,
            "bytes_written": summary.bytes_written,
            "warnings": loaded.warnings,
        });
        let text = serde_json::to_string_pretty(&payload).map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
        println!("{text}");
    } else {
        println!(
            "converted `{}` ({}) -> `{}` ({}, {} bytes)",
            parsed.input_path, loaded.format_tag, summary.out_path, summary.format, summary.bytes_written
        );
    }
    Ok(())
}

/// Parse an export format keyword or file extension.
pub fn parse_export_format(raw: &str) -> Result<ExportFormat, CliError> {
    let norm = raw.trim().trim_start_matches('.').to_ascii_lowercase();
    match norm.as_str() {
        "lumencraft" => Ok(ExportFormat::Lumencraft),
        "svg" => Ok(ExportFormat::Svg),
        "pdf" => Ok(ExportFormat::Pdf),
        "png" => Ok(ExportFormat::Png),
        "tiff" | "tif" => Ok(ExportFormat::Tiff),
        "csv" => Ok(ExportFormat::Csv),
        "tsv" => Ok(ExportFormat::Tsv),
        "json" => Ok(ExportFormat::Json),
        _ => Err(CliError::Usage(format!("unsupported export format `{raw}` (expected lumencraft, svg, pdf, png, tiff, csv, tsv, json)"))),
    }
}

/// Infer [`ExportFormat`] from an output file extension.
pub fn detect_export_format_from_path(path: &Path) -> Result<ExportFormat, CliError> {
    let ext = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| CliError::Usage(format!("output path `{}` has no recognizable format extension", path.display())))?;
    parse_export_format(ext)
}

/// Load a [`ProjectDocument`] from archive, interchange, JSON, or optionally CSV/TSV bytes.
pub fn load_project_from_bytes(path_hint: &Path, bytes: &[u8], allow_delimited: bool) -> Result<LoadedProject, CliError> {
    let ext = path_hint.extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("lumencraft" | "lctemplate") => load_native_archive(bytes),
        Some("prism") => load_import_report(import_prism_bytes(bytes)?, "prism"),
        Some("pzfx") => load_import_report(import_pzfx_bytes(bytes)?, "pzfx"),
        Some("json") => load_project_json(bytes),
        Some("csv") if allow_delimited => load_delimited_bytes(path_hint, bytes, &CsvOptions::csv(), "csv"),
        Some("tsv") if allow_delimited => load_delimited_bytes(path_hint, bytes, &CsvOptions::tsv(), "tsv"),
        Some("csv" | "tsv") => Err(CliError::Usage(String::from("delimited CSV/TSV files are not project archives"))),
        _ => sniff_project_bytes(bytes),
    }
}

/// Write an exported sheet or whole document to `req.out_path`.
pub fn export_from_document(req: &ExportRequest<'_>) -> Result<ExportOutcomeSummary, CliError> {
    if let Some(dpi) = req.dpi
        && dpi == 0
    {
        return Err(CliError::Usage(String::from("--dpi must be >= 1")));
    }

    let (bytes, sheet_id) = match req.format {
        ExportFormat::Lumencraft => (save_project_bytes(req.doc)?, None),
        ExportFormat::Svg => {
            let (id, scene) = compile_visual_target(req.doc, req.sheet)?;
            let svg = render_svg(&scene, &SvgExportOptions::default())?;
            (svg.into_bytes(), Some(id))
        }
        ExportFormat::Pdf => {
            let (id, scene) = compile_visual_target(req.doc, req.sheet)?;
            let pdf = render_pdf(&scene, &PdfExportOptions::default())?;
            (pdf, Some(id))
        }
        ExportFormat::Png => {
            let (id, scene) = compile_visual_target(req.doc, req.sheet)?;
            let img = rasterize_scene(&scene, req.dpi.unwrap_or(DEFAULT_RASTER_DPI))?;
            (encode_png(&img)?, Some(id))
        }
        ExportFormat::Tiff => {
            let (id, scene) = compile_visual_target(req.doc, req.sheet)?;
            let img = rasterize_scene(&scene, req.dpi.unwrap_or(DEFAULT_RASTER_DPI))?;
            (encode_tiff(&img, TiffCompression::Deflate)?, Some(id))
        }
        ExportFormat::Csv => export_tabular_csv(req.doc, req.sheet, req.tab_index, &CsvOptions::csv())?,
        ExportFormat::Tsv => export_tabular_csv(req.doc, req.sheet, req.tab_index, &CsvOptions::tsv())?,
        ExportFormat::Json => export_json_target(req.doc, req.sheet, req.pretty)?,
    };

    if let Some(parent) = req.out_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|err| CliError::Runtime(format!("failed to create output directory `{}`: {err}", parent.display())))?;
    }
    fs::write(req.out_path, &bytes).map_err(|err| CliError::Runtime(format!("failed to write `{}`: {err}", req.out_path.display())))?;

    Ok(ExportOutcomeSummary {
        out_path: req.out_path.to_string_lossy().into_owned(),
        format: req.format.as_str(),
        sheet_id,
        bytes_written: bytes.len(),
    })
}

fn load_native_archive(bytes: &[u8]) -> Result<LoadedProject, CliError> {
    let (kind, doc) = open_archive_bytes(bytes)?;
    let format_tag = match kind {
        ArchiveKind::Project => "lumencraft",
        ArchiveKind::Template => "lctemplate",
    };
    Ok(LoadedProject { doc, format_tag, warnings: Vec::new() })
}

fn load_import_report(report: ImportReport, format_tag: &'static str) -> Result<LoadedProject, CliError> {
    let warnings = report.warnings.into_iter().map(|w| w.message).collect();
    Ok(LoadedProject { doc: report.document, format_tag, warnings })
}

fn load_project_json(bytes: &[u8]) -> Result<LoadedProject, CliError> {
    let doc: ProjectDocument = serde_json::from_slice(bytes).map_err(|err| CliError::Runtime(format!("invalid project JSON: {err}")))?;
    Ok(LoadedProject { doc, format_tag: "json", warnings: Vec::new() })
}

fn load_delimited_bytes(path_hint: &Path, bytes: &[u8], opts: &CsvOptions, format_tag: &'static str) -> Result<LoadedProject, CliError> {
    let text = std::str::from_utf8(bytes).map_err(|err| CliError::Runtime(format!("input file is not valid UTF-8: {err}")))?;
    let stem = path_hint.file_stem().and_then(std::ffi::OsStr::to_str).filter(|s| !s.trim().is_empty()).unwrap_or("Imported Table");
    let ds = import_csv_to_datasheet(SheetId::new("data_1"), stem, text, TableKind::Xy, SubcolumnMode::Single, opts)?;
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
        return load_import_report(import_prism_bytes(bytes)?, "prism");
    }
    let trimmed = bytes.iter().copied().find(|b| !b.is_ascii_whitespace());
    match trimmed {
        Some(b'<') => load_import_report(import_pzfx_bytes(bytes)?, "pzfx"),
        Some(b'{') => load_project_json(bytes),
        _ => Err(CliError::Runtime(String::from("unrecognized input file format"))),
    }
}

fn compile_visual_target(doc: &ProjectDocument, sheet: Option<&str>) -> Result<(String, ChartScene), CliError> {
    let target = resolve_visual_target(doc, sheet)?;
    match target {
        VisualTarget::Graph(graph) => {
            let scene = compile_graph(graph, doc)?;
            Ok((graph.id.as_str().to_owned(), scene))
        }
        VisualTarget::Layout(layout) => {
            let scene = compile_layout(layout, doc)?;
            Ok((layout.id.as_str().to_owned(), scene))
        }
    }
}

fn resolve_visual_target<'a>(doc: &'a ProjectDocument, sheet: Option<&str>) -> Result<VisualTarget<'a>, CliError> {
    if let Some(needle) = sheet {
        if let Some(g) = find_graph_sheet(doc, needle) {
            return Ok(VisualTarget::Graph(g));
        }
        if let Some(l) = find_layout_sheet(doc, needle) {
            return Ok(VisualTarget::Layout(l));
        }
        return Err(CliError::Runtime(format!("visual sheet `{needle}` was not found among graphs or layouts")));
    }

    if doc.layout_sheets.len() == 1 && doc.graph_sheets.is_empty() {
        return Ok(VisualTarget::Layout(&doc.layout_sheets[0]));
    }
    if let Some(g) = doc.graph_sheets.iter().find(|g| g.initialized).or_else(|| doc.graph_sheets.first()) {
        return Ok(VisualTarget::Graph(g));
    }
    if let Some(l) = doc.layout_sheets.first() {
        return Ok(VisualTarget::Layout(l));
    }
    Err(CliError::Runtime(String::from("project contains no graph or layout sheets to render")))
}

fn find_graph_sheet<'a>(doc: &'a ProjectDocument, needle: &str) -> Option<&'a GraphSheet> {
    doc.graph_sheets
        .iter()
        .find(|g| g.id.as_str() == needle)
        .or_else(|| doc.graph_sheets.iter().find(|g| g.title == needle))
        .or_else(|| doc.graph_sheets.iter().find(|g| g.title.eq_ignore_ascii_case(needle)))
}

fn find_layout_sheet<'a>(doc: &'a ProjectDocument, needle: &str) -> Option<&'a lumencraft_doc::LayoutSheet> {
    doc.layout_sheets
        .iter()
        .find(|l| l.id.as_str() == needle)
        .or_else(|| doc.layout_sheets.iter().find(|l| l.title == needle))
        .or_else(|| doc.layout_sheets.iter().find(|l| l.title.eq_ignore_ascii_case(needle)))
}

fn resolve_tabular_target<'a>(doc: &'a ProjectDocument, sheet: Option<&str>) -> Result<TabularTarget<'a>, CliError> {
    if let Some(needle) = sheet {
        if let Some(d) = find_data_sheet(doc, needle) {
            return Ok(TabularTarget::Data(d));
        }
        if let Some(r) = find_result_sheet(doc, needle) {
            return Ok(TabularTarget::Result(r));
        }
        return Err(CliError::Runtime(format!("tabular sheet `{needle}` was not found")));
    }
    if let Some(d) = doc.data_sheets.first() {
        return Ok(TabularTarget::Data(d));
    }
    if let Some(r) = doc.result_sheets.first() {
        return Ok(TabularTarget::Result(r));
    }
    Err(CliError::Runtime(String::from("project contains no data or result sheets to export")))
}

fn find_data_sheet<'a>(doc: &'a ProjectDocument, needle: &str) -> Option<&'a DataSheet> {
    doc.data_sheets
        .iter()
        .find(|d| d.id.as_str() == needle)
        .or_else(|| doc.data_sheets.iter().find(|d| d.title == needle))
        .or_else(|| doc.data_sheets.iter().find(|d| d.title.eq_ignore_ascii_case(needle)))
}

fn find_result_sheet<'a>(doc: &'a ProjectDocument, needle: &str) -> Option<&'a ResultSheet> {
    doc.result_sheets
        .iter()
        .find(|r| r.id.as_str() == needle)
        .or_else(|| doc.result_sheets.iter().find(|r| r.title() == needle))
        .or_else(|| doc.result_sheets.iter().find(|r| r.title().eq_ignore_ascii_case(needle)))
}

fn export_tabular_csv(
    doc: &ProjectDocument,
    sheet: Option<&str>,
    tab_index: Option<usize>,
    opts: &CsvOptions,
) -> Result<(Vec<u8>, Option<String>), CliError> {
    match resolve_tabular_target(doc, sheet)? {
        TabularTarget::Data(ds) => Ok((export_datasheet_to_csv(ds, opts)?.into_bytes(), Some(ds.id.as_str().to_owned()))),
        TabularTarget::Result(res) => {
            let text = match tab_index {
                Some(idx) => {
                    let tab = res.tabs.get(idx).ok_or_else(|| CliError::Runtime(format!("result sheet `{}` has no tab at index {idx}", res.id)))?;
                    export_result_tab_csv(tab, opts)
                }
                None => export_result_sheet_tidy_csv(res, opts),
            };
            Ok((text.into_bytes(), Some(res.id.as_str().to_owned())))
        }
    }
}

fn export_json_target(doc: &ProjectDocument, sheet: Option<&str>, pretty: bool) -> Result<(Vec<u8>, Option<String>), CliError> {
    if let Some(needle) = sheet {
        if let Some(res) = find_result_sheet(doc, needle) {
            return Ok((export_result_sheet_json(res, pretty)?.into_bytes(), Some(res.id.as_str().to_owned())));
        }
        if let Some(ds) = find_data_sheet(doc, needle) {
            let json_text = if pretty { serde_json::to_string_pretty(ds) } else { serde_json::to_string(ds) }
                .map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            return Ok((json_text.into_bytes(), Some(ds.id.as_str().to_owned())));
        }
        if let Some(info) = doc.info_sheets.iter().find(|s| s.id.as_str() == needle || s.title == needle) {
            let json_text = if pretty { serde_json::to_string_pretty(info) } else { serde_json::to_string(info) }
                .map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
            return Ok((json_text.into_bytes(), Some(info.id.as_str().to_owned())));
        }
        return Err(CliError::Runtime(format!("sheet `{needle}` not found for JSON export")));
    }

    if doc.result_sheets.len() == 1 && doc.data_sheets.is_empty() && doc.graph_sheets.is_empty() && doc.layout_sheets.is_empty() {
        let res = &doc.result_sheets[0];
        return Ok((export_result_sheet_json(res, pretty)?.into_bytes(), Some(res.id.as_str().to_owned())));
    }

    let json_text = if pretty { serde_json::to_string_pretty(doc) } else { serde_json::to_string(doc) }
        .map_err(|err| CliError::Runtime(format!("JSON serialization failed: {err}")))?;
    Ok((json_text.into_bytes(), None))
}

fn rasterize_scene(scene: &ChartScene, dpi: u16) -> Result<lumencraft_render::RasterImage, CliError> {
    let scale = f64::from(dpi) / 72.0;
    let width_px = clamped_pixel_dim(scene.width_pt.max(1.0) * scale);
    let height_px = clamped_pixel_dim(scene.height_pt.max(1.0) * scale);
    Ok(render_scene(scene, RenderTargetSpec { width_px, height_px, dpi })?)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamped_pixel_dim(raw: f64) -> u32 {
    raw.round().clamp(1.0, MAX_RASTER_EDGE_PX) as u32
}

fn parse_convert_args(args: &[String]) -> Result<ConvertArgs, CliError> {
    let mut positionals = Vec::new();
    let mut sheet = None;
    let mut dpi = None;
    let mut json = false;
    let mut idx = 0;

    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--json" {
            json = true;
            idx += 1;
        } else if arg == "--sheet" {
            idx += 1;
            let val = args.get(idx).ok_or_else(|| CliError::Usage(String::from("--sheet requires a sheet ID or title")))?;
            sheet = Some(val.clone());
            idx += 1;
        } else if let Some(val) = arg.strip_prefix("--sheet=") {
            sheet = Some(val.to_owned());
            idx += 1;
        } else if arg == "--dpi" {
            idx += 1;
            let val = args.get(idx).ok_or_else(|| CliError::Usage(String::from("--dpi requires an integer >= 1")))?;
            dpi = Some(parse_dpi_value(val)?);
            idx += 1;
        } else if let Some(val) = arg.strip_prefix("--dpi=") {
            dpi = Some(parse_dpi_value(val)?);
            idx += 1;
        } else if arg.starts_with('-') {
            return Err(CliError::Usage(format!("unknown flag `{arg}` for `convert`")));
        } else {
            positionals.push(arg.clone());
            idx += 1;
        }
    }

    if positionals.len() != 2 {
        return Err(CliError::Usage(String::from("usage: lumencraft-cli convert <in> <out> [--sheet <id>] [--dpi <u16>] [--json]")));
    }

    let Some(output_path) = positionals.pop() else {
        return Err(CliError::Usage(String::from("missing <out> argument")));
    };
    let Some(input_path) = positionals.pop() else {
        return Err(CliError::Usage(String::from("missing <in> argument")));
    };

    Ok(ConvertArgs { input_path, output_path, sheet, dpi, json })
}

fn parse_dpi_value(raw: &str) -> Result<u16, CliError> {
    let dpi: u16 = raw.parse().map_err(|_| CliError::Usage(format!("invalid --dpi `{raw}` (expected positive integer <= {})", u16::MAX)))?;
    if dpi == 0 {
        return Err(CliError::Usage(String::from("--dpi must be >= 1")));
    }
    Ok(dpi)
}
