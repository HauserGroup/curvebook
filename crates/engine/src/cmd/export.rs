//! Vector and raster figure export commands (`export.*`, `graph.export*`, `layout.export*`).

use std::collections::BTreeMap;
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use curvebook_chart::{ChartScene, compile_graph, compile_layout};
use curvebook_doc::{ErrorCode, ProjectDocument, SheetId};
use curvebook_pdf::{MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfColorSpace, PdfExportOptions, render_pdf, render_pdf_page_refs};
use curvebook_render::{MAX_RASTER_PIXELS, RenderTargetSpec, TiffCompression, encode_emf, encode_png, encode_tiff, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use serde::Deserialize;
use serde::de::{Deserializer, Error as DeError, Visitor};
use serde_json::{Value, json};

use super::helpers::{encode_base64, parse_sheet_id, require_graph_sheet, require_layout_sheet};
use super::params::{
    array_schema, bool_schema, int_schema, non_empty_str_schema, nullable_schema, obj_schema_with_aliases, parse_params, str_schema,
};
use super::{CommandGroup, CommandSpec, Ctx, Effects};
use crate::{CommandOutcome, EngineError, Session};

const EXPORT_EFFECTS: Effects = Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT);

/// Return all registered figure export command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        id: "export.svg",
        label: "Export SVG",
        menu_path: "File/Export/SVG...",
        shortcut: "",
        summary: "Render a graph or layout sheet to scalable vector graphics (SVG) and optionally write it atomically to disk.",
        beginner: "Exports the selected graph or layout as crisp SVG text, optionally saving it to a file path.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_svg,
        enabled: has_exportable_sheet,
        run: run_export_svg,
    },
    CommandSpec {
        id: "export.pdf",
        label: "Export PDF",
        menu_path: "File/Export/PDF...",
        shortcut: "",
        summary: "Render one or more graph or layout sheets to publication PDF (sRGB or print CMYK) and optionally write it atomically to disk.",
        beginner: "Creates a single-page or multi-page vector PDF from one or more graphs and layouts.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_pdf,
        enabled: has_exportable_sheet,
        run: run_export_pdf,
    },
    CommandSpec {
        id: "export.png",
        label: "Export PNG",
        menu_path: "File/Export/PNG...",
        shortcut: "",
        summary: "Rasterize a graph or layout sheet to a PNG image at the requested pixel dimensions or DPI.",
        beginner: "Renders a graph or layout to a high-DPI PNG image, returning base64 bytes and optionally saving to disk.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_png,
        enabled: has_exportable_sheet,
        run: run_export_png,
    },
    CommandSpec {
        id: "export.tiff",
        label: "Export TIFF",
        menu_path: "File/Export/TIFF...",
        shortcut: "",
        summary: "Rasterize a graph or layout sheet to a TIFF image with selectable compression (`none`, `lzw`, `deflate`).",
        beginner: "Renders a publication TIFF image at custom DPI or resolution with lossless compression.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_tiff,
        enabled: has_exportable_sheet,
        run: run_export_tiff,
    },
    CommandSpec {
        id: "graph.exportSvg",
        label: "Export Graph SVG",
        menu_path: "File/Export/SVG",
        shortcut: "",
        summary: "Export a graph sheet to SVG using the unified SVG renderer.",
        beginner: "Exports the active graph to vector SVG format.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_svg,
        enabled: has_exportable_sheet,
        run: run_graph_export_svg,
    },
    CommandSpec {
        id: "graph.exportPdf",
        label: "Export Graph PDF",
        menu_path: "File/Export/PDF",
        shortcut: "",
        summary: "Export a graph sheet (or sheet list) to vector PDF.",
        beginner: "Exports the active graph to publication PDF format.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_pdf,
        enabled: has_exportable_sheet,
        run: run_graph_export_pdf,
    },
    CommandSpec {
        id: "graph.exportPng",
        label: "Export Graph PNG",
        menu_path: "File/Export/PNG",
        shortcut: "",
        summary: "Export a graph sheet to a raster PNG image.",
        beginner: "Exports the active graph to a PNG bitmap at configurable DPI and size.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_png,
        enabled: has_exportable_sheet,
        run: run_graph_export_png,
    },
    CommandSpec {
        id: "graph.exportEmf",
        label: "Export Graph EMF",
        menu_path: "File/Export/Graph EMF...",
        shortcut: "",
        summary: "Export a graph sheet as a Windows Enhanced Metafile (EMF) vector graphic.",
        beginner: "Exports a graph sheet to EMF for insertion into Office documents.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_emf,
        enabled: has_graph_sheet,
        run: run_graph_export_emf,
    },
    CommandSpec {
        id: "layout.exportEmf",
        label: "Export Layout EMF",
        menu_path: "File/Export/Layout EMF...",
        shortcut: "",
        summary: "Export a multi-panel layout sheet as a Windows Enhanced Metafile (EMF) vector graphic.",
        beginner: "Exports a multi-panel layout sheet to EMF for insertion into Office documents.",
        group: CommandGroup::Export,
        effects: EXPORT_EFFECTS,
        input_schema: schema_export_emf,
        enabled: has_layout_sheet,
        run: run_layout_export_emf,
    },
];

fn has_exportable_sheet(session: &Session) -> bool {
    !session.document().graph_sheets.is_empty() || !session.document().layout_sheets.is_empty()
}

fn has_graph_sheet(session: &Session) -> bool {
    !session.document().graph_sheets.is_empty()
}

fn has_layout_sheet(session: &Session) -> bool {
    !session.document().layout_sheets.is_empty()
}

// ---------------------------------------------------------------------------
// Shared Export & Serde Utilities (reused across sibling cmd modules)
// ---------------------------------------------------------------------------

/// Atomically write exported bytes to `path`, creating parent directories when needed.
pub(super) fn write_atomic_output_file(path: &str, bytes: &[u8]) -> Result<(), EngineError> {
    if path.trim().is_empty() {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`path` must not be empty")
            .with_remediation("Provide a non-empty destination file path or omit `path` to return exported bytes inline.")
            .with_details(json!({ "pointer": "/path" })));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let target = Path::new(path);
        if let Some(parent) = target.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| {
                EngineError::with_code(ErrorCode::IoError, format!("failed creating parent directory `{}`: {err}", parent.display()))
                    .with_retryable(true)
                    .with_remediation("Ensure the target directory path is writable.")
                    .with_details(json!({ "path": path, "parent": parent.display().to_string() }))
            })?;
        }
        curvebook_platform::fs::write_atomic(target, bytes)?;
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        super::helpers::write_output_file(path, bytes)
    }
}

/// Compile either a graph sheet or a layout sheet into a device-independent [`ChartScene`].
pub(super) fn compile_scene_for_sheet(doc: &ProjectDocument, id: &SheetId) -> Result<ChartScene, EngineError> {
    if let Some(graph) = doc.graph_sheet(id) {
        return Ok(compile_graph(graph, doc)?);
    }
    if let Some(layout) = doc.layout_sheet(id) {
        return Ok(compile_layout(layout, doc)?);
    }
    Err(EngineError::with_code(ErrorCode::NotFound, format!("sheet `{id}` is not a graph or layout sheet"))
        .with_remediation("Supply an existing graph or layout `sheet_id`.")
        .with_details(json!({ "sheet_id": id.as_str() })))
}

struct WholeU64Visitor {
    max: u64,
}

impl<'de> Visitor<'de> for WholeU64Visitor {
    type Value = Option<u64>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "a whole non-negative integer in 0..={} or null", self.max)
    }

    fn visit_none<E: DeError>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: DeError>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(self)
    }

    fn visit_u64<E: DeError>(self, value: u64) -> Result<Self::Value, E> {
        if value > self.max {
            return Err(E::custom(format!("integer {value} exceeds maximum {}", self.max)));
        }
        Ok(Some(value))
    }

    fn visit_i64<E: DeError>(self, value: i64) -> Result<Self::Value, E> {
        let unsigned = u64::try_from(value).map_err(|_| E::custom(format!("expected non-negative integer, got {value}")))?;
        self.visit_u64(unsigned)
    }

    fn visit_f64<E: DeError>(self, value: f64) -> Result<Self::Value, E> {
        // Exact IEEE-754 whole-integer conversion up to 2^53 - 1 so `300.0` parses identically to `300`.
        const MAX_SAFE_INT_F64: f64 = 9_007_199_254_740_991.0;
        if !value.is_finite() || value < 0.0 || value.fract() != 0.0 || value > MAX_SAFE_INT_F64 {
            return Err(E::custom(format!("expected finite non-negative whole integer, got {value}")));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let parsed = value as u64;
        self.visit_u64(parsed)
    }
}

/// Deserialize an optional `u8`, accepting exact whole-number JSON floats such as `2.0`.
pub(super) fn de_opt_u8<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u8>, D::Error> {
    let raw = deserializer.deserialize_option(WholeU64Visitor { max: u64::from(u8::MAX) })?;
    raw.map(|v| u8::try_from(v).map_err(|_| D::Error::custom(format!("value {v} exceeds u8::MAX")))).transpose()
}

/// Deserialize an optional `u16`, accepting exact whole-number JSON floats such as `300.0`.
pub(super) fn de_opt_u16<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u16>, D::Error> {
    let raw = deserializer.deserialize_option(WholeU64Visitor { max: u64::from(u16::MAX) })?;
    raw.map(|v| u16::try_from(v).map_err(|_| D::Error::custom(format!("value {v} exceeds u16::MAX")))).transpose()
}

/// Deserialize an optional `u32`, accepting exact whole-number JSON floats such as `1200.0`.
pub(super) fn de_opt_u32<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    let raw = deserializer.deserialize_option(WholeU64Visitor { max: u64::from(u32::MAX) })?;
    raw.map(|v| u32::try_from(v).map_err(|_| D::Error::custom(format!("value {v} exceeds u32::MAX")))).transpose()
}

/// Deserialize an optional `usize`, accepting exact whole-number JSON floats such as `0.0`.
pub(super) fn de_opt_usize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<usize>, D::Error> {
    let raw = deserializer.deserialize_option(WholeU64Visitor { max: usize::MAX as u64 })?;
    raw.map(|v| usize::try_from(v).map_err(|_| D::Error::custom(format!("value {v} exceeds usize::MAX")))).transpose()
}

fn check_not_cancelled(ctx: &Ctx<'_>, command_id: &str) -> Result<(), EngineError> {
    if ctx.is_cancelled() {
        return Err(EngineError::with_code(ErrorCode::Cancelled, format!("command `{command_id}` cancelled"))
            .with_retryable(true)
            .with_details(json!({ "command_id": command_id })));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ExportSvgParams {
    #[serde(alias = "id")]
    sheet_id: String,
    path: Option<String>,
    pretty: Option<bool>,
    include_xml_declaration: Option<bool>,
    include_metadata: Option<bool>,
    embed_fonts: Option<bool>,
    #[serde(default, deserialize_with = "de_opt_u8")]
    precision: Option<u8>,
    outline_text: Option<bool>,
}

fn schema_export_svg() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph or layout sheet ID to export")),
            ("path", nullable_schema(non_empty_str_schema("Optional output file path"))),
            ("pretty", nullable_schema(bool_schema("Format SVG XML with indentation"))),
            ("include_xml_declaration", nullable_schema(bool_schema("Include leading XML declaration"))),
            ("include_metadata", nullable_schema(bool_schema("Include document metadata block"))),
            ("embed_fonts", nullable_schema(bool_schema("Embed bundled font faces"))),
            ("precision", nullable_schema(int_schema("Numeric coordinate decimal precision (0..=255)", Some(0), Some(i64::from(u8::MAX))))),
            ("outline_text", nullable_schema(bool_schema("Convert glyphs into vector outline paths"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportPdfParams {
    #[serde(alias = "id")]
    sheet_id: Option<String>,
    sheet_ids: Option<Vec<String>>,
    path: Option<String>,
    color_space: Option<String>,
    compress_streams: Option<bool>,
    title: Option<String>,
    #[serde(alias = "vector_text")]
    outline_text: Option<bool>,
}

fn schema_export_pdf() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", nullable_schema(non_empty_str_schema("Graph or layout sheet ID for single-page PDF"))),
            ("sheet_ids", nullable_schema(array_schema("Ordered sheet IDs for multi-page PDF export", non_empty_str_schema("Sheet ID"), None, None))),
            ("path", nullable_schema(non_empty_str_schema("Optional output PDF file path"))),
            ("color_space", nullable_schema(str_schema("PDF output color space (`srgb` or `cmyk`)"))),
            ("compress_streams", nullable_schema(bool_schema("Compress PDF content streams"))),
            ("title", nullable_schema(str_schema("Optional PDF document title metadata"))),
            ("outline_text", nullable_schema(bool_schema("Outline text glyphs into vector paths"))),
        ],
        &[],
        vec![("sheet_id", &["id"]), ("outline_text", &["vector_text"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportPngParams {
    #[serde(alias = "id")]
    sheet_id: String,
    path: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    width_px: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    height_px: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u16")]
    dpi: Option<u16>,
}

fn raster_dim_schema(label: &str) -> Value {
    nullable_schema(int_schema(label, Some(1), Some(i64::from(u32::MAX))))
}

fn raster_dpi_schema() -> Value {
    nullable_schema(int_schema("Target resolution in dots per inch (default 300)", Some(1), Some(i64::from(u16::MAX))))
}

fn schema_export_png() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph or layout sheet ID to rasterize")),
            ("path", nullable_schema(non_empty_str_schema("Optional output PNG file path"))),
            ("width_px", raster_dim_schema("Explicit raster width in pixels")),
            ("height_px", raster_dim_schema("Explicit raster height in pixels")),
            ("dpi", raster_dpi_schema()),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportTiffParams {
    #[serde(alias = "id")]
    sheet_id: String,
    path: Option<String>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    width_px: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u32")]
    height_px: Option<u32>,
    #[serde(default, deserialize_with = "de_opt_u16")]
    dpi: Option<u16>,
    compression: Option<String>,
}

fn schema_export_tiff() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Graph or layout sheet ID to rasterize")),
            ("path", nullable_schema(non_empty_str_schema("Optional output TIFF file path"))),
            ("width_px", raster_dim_schema("Explicit raster width in pixels")),
            ("height_px", raster_dim_schema("Explicit raster height in pixels")),
            ("dpi", raster_dpi_schema()),
            ("compression", nullable_schema(str_schema("TIFF compression codec (`none`, `lzw`, `deflate`)"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

#[derive(Debug, Deserialize)]
struct ExportEmfParams {
    #[serde(alias = "id")]
    sheet_id: String,
    path: Option<String>,
}

fn schema_export_emf() -> Value {
    obj_schema_with_aliases(
        vec![
            ("sheet_id", non_empty_str_schema("Target sheet ID to export to EMF")),
            ("path", nullable_schema(non_empty_str_schema("Optional output EMF file path"))),
        ],
        &["sheet_id"],
        vec![("sheet_id", &["id"])],
        false,
    )
}

// ---------------------------------------------------------------------------
// Command Runners
// ---------------------------------------------------------------------------

fn run_export_svg(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_svg_with_id(ctx, "export.svg", raw)
}

fn run_graph_export_svg(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_svg_with_id(ctx, "graph.exportSvg", raw)
}

fn run_svg_with_id(ctx: &mut Ctx<'_>, command_id: &str, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_export_svg();
    let params: ExportSvgParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let scene = compile_scene_for_sheet(ctx.session.document(), &id)?;
    check_not_cancelled(ctx, command_id)?;

    let mut opts = SvgExportOptions::default();
    if let Some(p) = params.pretty {
        opts.pretty = p;
    }
    if let Some(xml) = params.include_xml_declaration {
        opts.include_xml_declaration = xml;
    }
    if let Some(meta) = params.include_metadata {
        opts.include_metadata = meta;
    }
    if let Some(emb) = params.embed_fonts {
        opts.embed_fonts = emb;
    }
    if let Some(prec) = params.precision {
        opts.precision = prec;
    }
    if let Some(ot) = params.outline_text {
        opts.outline_text = ot;
    }

    let svg = render_svg(&scene, &opts)?;
    if let Some(ref p) = params.path {
        write_atomic_output_file(p, svg.as_bytes())?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported SVG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": svg.len(),
            "width_pt": scene.width_pt,
            "height_pt": scene.height_pt,
            "warnings_count": scene.warnings.len(),
            "path": params.path,
            "svg": svg,
        }),
    ))
}

fn run_export_pdf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_pdf_with_id(ctx, "export.pdf", raw)
}

fn run_graph_export_pdf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_pdf_with_id(ctx, "graph.exportPdf", raw)
}

fn parse_pdf_color_space(raw: Option<&str>) -> Result<PdfColorSpace, EngineError> {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("srgb" | "rgb") => Ok(PdfColorSpace::Srgb),
        Some("cmyk") => Ok(PdfColorSpace::Cmyk),
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown PDF `color_space` `{other}`"))
            .with_remediation("Use `srgb` or `cmyk`.")
            .with_details(json!({ "pointer": "/color_space", "color_space": other }))),
    }
}

fn run_pdf_with_id(ctx: &mut Ctx<'_>, command_id: &str, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_export_pdf();
    let params: ExportPdfParams = parse_params(&schema, raw)?;
    check_not_cancelled(ctx, command_id)?;

    let mut opts = PdfExportOptions { color_space: parse_pdf_color_space(params.color_space.as_deref())?, ..PdfExportOptions::default() };
    if let Some(comp) = params.compress_streams {
        opts.compress_streams = comp;
    }
    if let Some(title) = params.title {
        opts.title = Some(title);
    }
    if let Some(outline) = params.outline_text {
        opts.outline_text = outline;
    }

    let doc = ctx.session.document();
    let (pdf, sheet_label, width_pt, height_pt, page_count) = if let Some(ids) = params.sheet_ids {
        if ids.len() > MAX_PDF_PAGES {
            return Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("PDF page count {} exceeds maximum {MAX_PDF_PAGES}", ids.len()))
                .with_remediation("Export fewer pages or split into multiple PDF files.")
                .with_details(json!({ "page_count": ids.len(), "max_pages": MAX_PDF_PAGES })));
        }
        let mut parsed_ids = Vec::with_capacity(ids.len());
        for raw_id in ids {
            parsed_ids.push(parse_sheet_id(&raw_id)?);
        }
        let mut compiled_by_id: BTreeMap<SheetId, ChartScene> = BTreeMap::new();
        for sid in &parsed_ids {
            check_not_cancelled(ctx, command_id)?;
            if !compiled_by_id.contains_key(sid) {
                compiled_by_id.insert(sid.clone(), compile_scene_for_sheet(doc, sid)?);
            }
        }
        let mut page_refs: Vec<&ChartScene> = Vec::with_capacity(parsed_ids.len());
        let mut total_primitives: usize = 0;
        for sid in &parsed_ids {
            let scene = compiled_by_id
                .get(sid)
                .ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, format!("compiled scene missing for `{sid}`")))?;
            total_primitives = total_primitives.saturating_add(scene.primitives.len());
            if total_primitives > MAX_PDF_TOTAL_PRIMITIVES {
                return Err(EngineError::with_code(
                    ErrorCode::ResourceLimit,
                    format!("PDF primitive count {total_primitives} exceeds maximum {MAX_PDF_TOTAL_PRIMITIVES}"),
                )
                .with_remediation("Reduce chart complexity or split the PDF export into fewer pages.")
                .with_details(json!({
                    "primitive_count": total_primitives,
                    "max_primitives": MAX_PDF_TOTAL_PRIMITIVES,
                    "page_count": parsed_ids.len(),
                })));
            }
            page_refs.push(scene);
        }
        let w = page_refs.first().map_or(0.0, |s| s.width_pt);
        let h = page_refs.first().map_or(0.0, |s| s.height_pt);
        let pages = page_refs.len();
        let bytes = render_pdf_page_refs(&page_refs, &opts)?;
        (bytes, String::from("multi-page"), w, h, pages)
    } else {
        let raw_id = params.sheet_id.ok_or_else(|| {
            EngineError::with_code(ErrorCode::BadInput, "missing required parameter `sheet_id` or `sheet_ids`")
                .with_remediation("Pass `sheet_id` for a single page or `sheet_ids` for a multi-page PDF.")
                .with_details(json!({ "pointer": "/sheet_id" }))
        })?;
        let id = parse_sheet_id(&raw_id)?;
        let scene = compile_scene_for_sheet(doc, &id)?;
        let bytes = render_pdf(&scene, &opts)?;
        (bytes, id.as_str().to_owned(), scene.width_pt, scene.height_pt, 1_usize)
    };

    if let Some(ref p) = params.path {
        write_atomic_output_file(p, &pdf)?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported PDF figure",
        json!({
            "sheet_id": sheet_label,
            "byte_len": pdf.len(),
            "page_count": page_count,
            "width_pt": width_pt,
            "height_pt": height_pt,
            "color_space": opts.color_space,
            "path": params.path,
            "base64": encode_base64(&pdf),
        }),
    ))
}

fn resolve_raster_target(
    scene: &ChartScene,
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: Option<u16>,
) -> Result<RenderTargetSpec, EngineError> {
    if !scene.width_pt.is_finite() || scene.width_pt <= 0.0 || !scene.height_pt.is_finite() || scene.height_pt <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "scene canvas dimensions must be positive and finite"));
    }
    let dpi_val = dpi.unwrap_or(300);
    if dpi_val == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`dpi` must be greater than 0").with_details(json!({ "pointer": "/dpi" })));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let default_w = ((scene.width_pt * f64::from(dpi_val) / 72.0).round().clamp(16.0, 8192.0)) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let default_h = ((scene.height_pt * f64::from(dpi_val) / 72.0).round().clamp(16.0, 8192.0)) as u32;
    let w = width_px.unwrap_or(default_w);
    let h = height_px.unwrap_or(default_h);
    if w == 0 || h == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "raster width and height must be greater than 0")
            .with_details(json!({ "pointer": if w == 0 { "/width_px" } else { "/height_px" } })));
    }
    let max_dim = curvebook_render::rasterizer::MAX_RASTER_DIM_PX;
    if w > max_dim || h > max_dim {
        return Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("raster dimensions {w}x{h} exceed maximum {max_dim}x{max_dim}"))
            .with_remediation("Reduce `width_px`, `height_px`, or `dpi`.")
            .with_details(json!({ "width_px": w, "height_px": h, "max_dim_px": max_dim })));
    }
    let total_pixels = u64::from(w).saturating_mul(u64::from(h));
    if total_pixels > MAX_RASTER_PIXELS {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("raster pixel count {total_pixels} ({w}x{h}) exceeds budget {MAX_RASTER_PIXELS}"),
        )
        .with_remediation("Reduce `width_px`, `height_px`, or `dpi`.")
        .with_details(json!({ "total_pixels": total_pixels, "max_pixels": MAX_RASTER_PIXELS })));
    }
    Ok(RenderTargetSpec { width_px: w, height_px: h, dpi: dpi_val })
}

fn run_export_png(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_png_with_id(ctx, "export.png", raw)
}

fn run_graph_export_png(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_png_with_id(ctx, "graph.exportPng", raw)
}

fn run_png_with_id(ctx: &mut Ctx<'_>, command_id: &str, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_export_png();
    let params: ExportPngParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let scene = compile_scene_for_sheet(ctx.session.document(), &id)?;
    check_not_cancelled(ctx, command_id)?;

    let target = resolve_raster_target(&scene, params.width_px, params.height_px, params.dpi)?;
    let img = render_scene(&scene, target)?;
    let png = encode_png(&img)?;

    if let Some(ref p) = params.path {
        write_atomic_output_file(p, &png)?;
    }

    Ok(CommandOutcome::read_only(
        command_id,
        "exported PNG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": png.len(),
            "width_px": img.width_px,
            "height_px": img.height_px,
            "dpi": target.dpi,
            "path": params.path,
            "base64": encode_base64(&png),
        }),
    ))
}

fn parse_tiff_compression(raw: Option<&str>) -> Result<(&'static str, TiffCompression), EngineError> {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("deflate") => Ok(("deflate", TiffCompression::Deflate)),
        Some("none") => Ok(("none", TiffCompression::None)),
        Some("lzw") => Ok(("lzw", TiffCompression::Lzw)),
        Some(other) => Err(EngineError::with_code(ErrorCode::BadInput, format!("unknown TIFF compression `{other}`"))
            .with_remediation("Use `none`, `lzw`, or `deflate`.")
            .with_details(json!({ "pointer": "/compression", "compression": other }))),
    }
}

fn run_export_tiff(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_export_tiff();
    let params: ExportTiffParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let scene = compile_scene_for_sheet(ctx.session.document(), &id)?;
    check_not_cancelled(ctx, "export.tiff")?;

    let target = resolve_raster_target(&scene, params.width_px, params.height_px, params.dpi)?;
    let (compression_label, compression) = parse_tiff_compression(params.compression.as_deref())?;
    let img = render_scene(&scene, target)?;
    let tiff = encode_tiff(&img, compression)?;

    if let Some(ref p) = params.path {
        write_atomic_output_file(p, &tiff)?;
    }

    Ok(CommandOutcome::read_only(
        "export.tiff",
        "exported TIFF figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": tiff.len(),
            "width_px": img.width_px,
            "height_px": img.height_px,
            "dpi": target.dpi,
            "compression": compression_label,
            "path": params.path,
            "base64": encode_base64(&tiff),
        }),
    ))
}

fn run_graph_export_emf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_emf_with_kind(ctx, "graph.exportEmf", false, raw)
}

fn run_layout_export_emf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_emf_with_kind(ctx, "layout.exportEmf", true, raw)
}

fn run_emf_with_kind(ctx: &mut Ctx<'_>, command_id: &str, is_layout: bool, raw: Value) -> Result<CommandOutcome, EngineError> {
    let schema = schema_export_emf();
    let params: ExportEmfParams = parse_params(&schema, raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let doc = ctx.session.document();
    let scene = if is_layout {
        let layout = require_layout_sheet(doc, &id)?;
        compile_layout(layout, doc)?
    } else {
        let graph = require_graph_sheet(doc, &id)?;
        compile_graph(graph, doc)?
    };
    check_not_cancelled(ctx, command_id)?;

    let emf = encode_emf(&scene)?;
    let bytes_written = if let Some(ref p) = params.path {
        write_atomic_output_file(p, &emf)?;
        emf.len()
    } else {
        0
    };
    let magic = emf
        .get(40..44)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map_or_else(String::new, |bytes| format!("0x{:08X}", u32::from_le_bytes(bytes)));

    Ok(CommandOutcome::read_only(
        command_id,
        "exported EMF figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": emf.len(),
            "bytes_written": bytes_written,
            "header_magic": magic,
            "width_pt": scene.width_pt,
            "height_pt": scene.height_pt,
            "path": params.path,
            "base64": encode_base64(&emf),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, DataSheet, ErrorCode, GraphKind, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable,
    };
    use serde_json::json;

    use super::*;

    fn doc_with_graph_and_layout() -> ProjectDocument {
        let mut doc = ProjectDocument::new("Export Test").unwrap();
        let mut table = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
        table.x_cells[0][0] = Cell::number(1.0);
        table.x_cells[1][0] = Cell::number(2.0);
        table.data_sets[0].cells[0][0] = Cell::number(10.0);
        table.data_sets[0].cells[1][0] = Cell::number(20.0);
        let ds = DataSheet::new("d1", "Data", TableArchetype::Xy(table)).unwrap();
        let mut graph = GraphSheet::new_uninitialized_for_data("g1", &ds).unwrap();
        graph.spec.kind = GraphKind::Xy;
        graph.initialized = true;
        doc.add_data_sheet(ds).unwrap();
        doc.add_graph_sheet(graph).unwrap();
        doc.add_layout_sheet(LayoutSheet::new("l1", "Layout", LayoutSpec::default()).unwrap()).unwrap();
        doc
    }

    #[test]
    fn registry_contains_all_nine_export_commands_with_effects() {
        assert_eq!(specs().len(), 9);
        for spec in specs() {
            assert_eq!(spec.group, CommandGroup::Export);
            assert!(spec.effects.contains(Effects::WRITES_FILESYSTEM));
            assert!(spec.effects.contains(Effects::IDEMPOTENT));
            assert!(!spec.effects.contains(Effects::MUTATES_DOCUMENT));
            assert!(!spec.summary.is_empty());
            assert!(!spec.beginner.is_empty());
        }
    }

    #[test]
    fn export_commands_handle_float_dpi_and_atomic_file_writes() {
        let mut session = Session::from_document(doc_with_graph_and_layout());
        let out_dir = std::env::temp_dir().join(format!("cb_exp_test_{}", std::process::id()));
        let svg_path = out_dir.join("nested").join("fig.svg");
        let png_res = session
            .execute("export.png", json!({ "sheet_id": "g1", "width_px": 64, "height_px": 48, "dpi": 300.0 }))
            .expect("whole float dpi must parse cleanly");
        assert_eq!(png_res.payload["dpi"], json!(300));

        let svg_res = session
            .execute("graph.exportSvg", json!({ "sheet_id": "g1", "path": svg_path.to_string_lossy() }))
            .expect("atomic SVG export must create parent dirs and write");
        assert_eq!(svg_res.command_id, "graph.exportSvg");
        assert!(svg_path.exists());
        let _ = std::fs::remove_dir_all(&out_dir);

        let err = session.execute("export.png", json!({ "sheet_id": "g1", "dpi": 0 })).expect_err("zero dpi must fail");
        assert_eq!(err.code, ErrorCode::BadInput);
        let limit = session
            .execute("export.tiff", json!({ "sheet_id": "g1", "width_px": 16_384, "height_px": 16_384 }))
            .expect_err("over-budget raster must fail");
        assert_eq!(limit.code, ErrorCode::ResourceLimit);
        let emf_wrong = session.execute("layout.exportEmf", json!({ "sheet_id": "g1" })).expect_err("wrong sheet kind");
        assert_eq!(emf_wrong.code, ErrorCode::NotFound);
    }

    #[test]
    fn empty_session_disables_export_commands() {
        let empty = Session::new();
        for spec in specs() {
            assert!(!(spec.enabled)(&empty), "{} should be disabled when doc has no figures", spec.id);
        }
        let populated = Session::from_document(doc_with_graph_and_layout());
        for spec in specs() {
            assert!((spec.enabled)(&populated), "{} should be enabled when graph & layout exist", spec.id);
        }
        let mut s = Session::from_document(doc_with_graph_and_layout());
        let bad = s.execute("export.pdf", json!({ "color_space": "lab" })).expect_err("invalid pdf color space");
        assert_eq!(bad.code, ErrorCode::BadInput);
        assert!(SheetId::try_new("g1").is_ok());
    }
}
