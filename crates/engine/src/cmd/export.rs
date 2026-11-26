//! Vector and raster figure export commands (`export.*`, `graph.export*`, `layout.export*`).

use std::fmt;

use curvebook_chart::{ChartScene, ChartWarning};
use curvebook_doc::{ErrorCode, ProjectDocument, SheetId};
use curvebook_pdf::{PdfColorSpace, PdfExportOptions};
use curvebook_render::TiffCompression;
use curvebook_svg::SvgExportOptions;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::{Deserializer, Error as DeError, Visitor};
use serde_json::{Value, json};

use super::helpers::{bad_input, bad_input_ptr, encode_base64, has_exportable_sheet, has_graph_sheet, has_layout_sheet, parse_sheet_id};
use super::params::{NonEmptyStrSchema, command_schema, parse_params};
use super::{CommandGroup, CommandSpec, Ctx, Effects, command_specs};
use crate::services::{
    EmfSheetKind, ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterSizingPolicy, compile_visual_scene_for_sheet,
    export_document_artifact,
};
use crate::{CommandOutcome, EngineError};

const EXPORT_EFFECTS: Effects = Effects::WRITES_FILESYSTEM.with(Effects::IDEMPOTENT);

/// Return all registered figure export command specifications.
#[must_use]
pub fn specs() -> &'static [CommandSpec] {
    SPECS
}

/// Build a figure export outcome that states every scene warning, so an omitted fitted curve (failed,
/// pending recalculation, or missing) is reported rather than leaving a figure that looks complete.
///
/// The warnings appear in the payload as `warnings: [{code, message}]` with `warnings_count`, and in
/// [`CommandOutcome::warnings`] as `code: message` lines that the CLI, batch runner, and desktop show.
fn figure_export_outcome(command_id: &str, message: &str, mut payload: Value, warnings: &[ChartWarning]) -> CommandOutcome {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(String::from("warnings_count"), json!(warnings.len()));
        obj.insert(String::from("warnings"), Value::Array(warnings.iter().map(|w| json!({ "code": w.code, "message": w.message })).collect()));
    }
    let message = if warnings.is_empty() { message.to_owned() } else { format!("{message} with {} warning(s)", warnings.len()) };
    let mut outcome = CommandOutcome::read_only(command_id, message, payload);
    outcome.warnings = warnings.iter().map(|w| format!("{}: {}", w.code, w.message)).collect();
    outcome
}

static SPECS: &[CommandSpec] = &command_specs!(
    CommandGroup::Export;
    {
        id: "export.svg",
        label: "Export SVG",
        menu_path: "File/Export/SVG...",
        summary: "Render a graph or layout sheet to scalable vector graphics (SVG) and optionally write it atomically to disk.",
        beginner: "Exports the selected graph or layout as crisp SVG text, optionally saving it to a file path.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportSvgParams>,
        enabled: has_exportable_sheet,
        run: run_export_svg,
    },
    {
        id: "export.pdf",
        label: "Export PDF",
        menu_path: "File/Export/PDF...",
        summary: "Render one or more graph or layout sheets to publication PDF (sRGB, print CMYK, or grayscale) and optionally write it atomically to disk.",
        beginner: "Creates a single-page or multi-page vector PDF from one or more graphs and layouts.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportPdfParams>,
        enabled: has_exportable_sheet,
        run: run_export_pdf,
    },
    {
        id: "export.png",
        label: "Export PNG",
        menu_path: "File/Export/PNG...",
        summary: "Rasterize a graph or layout sheet to a PNG image at the requested pixel dimensions or DPI.",
        beginner: "Renders a graph or layout to a high-DPI PNG image, returning base64 bytes and optionally saving to disk.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportPngParams>,
        enabled: has_exportable_sheet,
        run: run_export_png,
    },
    {
        id: "export.tiff",
        label: "Export TIFF",
        menu_path: "File/Export/TIFF...",
        summary: "Rasterize a graph or layout sheet to a TIFF image with selectable compression (`none`, `lzw`, `deflate`).",
        beginner: "Renders a publication TIFF image at custom DPI or resolution with lossless compression.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportTiffParams>,
        enabled: has_exportable_sheet,
        run: run_export_tiff,
    },
    {
        id: "graph.exportSvg",
        label: "Export Graph SVG",
        menu_path: "File/Export/SVG",
        summary: "Export a graph sheet to SVG using the unified SVG renderer.",
        beginner: "Exports the active graph to vector SVG format.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportSvgParams>,
        enabled: has_exportable_sheet,
        run: run_graph_export_svg,
    },
    {
        id: "graph.exportPdf",
        label: "Export Graph PDF",
        menu_path: "File/Export/PDF",
        summary: "Export a graph sheet (or sheet list) to vector PDF.",
        beginner: "Exports the active graph to publication PDF format.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportPdfParams>,
        enabled: has_exportable_sheet,
        run: run_graph_export_pdf,
    },
    {
        id: "graph.exportPng",
        label: "Export Graph PNG",
        menu_path: "File/Export/PNG",
        summary: "Export a graph sheet to a raster PNG image.",
        beginner: "Exports the active graph to a PNG bitmap at configurable DPI and size.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportPngParams>,
        enabled: has_exportable_sheet,
        run: run_graph_export_png,
    },
    {
        id: "graph.exportEmf",
        label: "Export Graph EMF",
        menu_path: "File/Export/Graph EMF...",
        summary: "Export a graph sheet as a Windows Enhanced Metafile (EMF) vector graphic.",
        beginner: "Exports a graph sheet to EMF for insertion into Office documents.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportEmfParams>,
        enabled: has_graph_sheet,
        run: run_graph_export_emf,
    },
    {
        id: "layout.exportEmf",
        label: "Export Layout EMF",
        menu_path: "File/Export/Layout EMF...",
        summary: "Export a multi-panel layout sheet as a Windows Enhanced Metafile (EMF) vector graphic.",
        beginner: "Exports a multi-panel layout sheet to EMF for insertion into Office documents.",
        effects: EXPORT_EFFECTS,
        input_schema: command_schema::<ExportEmfParams>,
        enabled: has_layout_sheet,
        run: run_layout_export_emf,
    },
);

// ---------------------------------------------------------------------------
// Shared Export & Serde Utilities (reused across sibling cmd modules)
// ---------------------------------------------------------------------------

/// Compile either a graph sheet or a layout sheet into a device-independent [`ChartScene`].
pub(super) fn compile_scene_for_sheet(doc: &ProjectDocument, id: &SheetId) -> Result<ChartScene, EngineError> {
    compile_visual_scene_for_sheet(doc, id)
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

// ---------------------------------------------------------------------------
// Schemas & Typed Parameter Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportSvgParams {
    /// Graph or layout sheet ID to export.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional output file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// Format SVG XML with indentation.
    pretty: Option<bool>,
    /// Include leading XML declaration.
    include_xml_declaration: Option<bool>,
    /// Include document metadata block.
    include_metadata: Option<bool>,
    /// Embed bundled font faces.
    embed_fonts: Option<bool>,
    /// Numeric coordinate decimal precision (0..=255).
    #[serde(default, deserialize_with = "de_opt_u8")]
    precision: Option<u8>,
    /// Convert glyphs into vector outline paths.
    outline_text: Option<bool>,
    /// Omit opaque sheet canvas background fill for transparent SVG export.
    transparent_bg: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportPdfParams {
    /// Graph or layout sheet ID for single-page PDF.
    #[schemars(length(min = 1))]
    sheet_id: Option<String>,
    /// Ordered sheet IDs for multi-page PDF export.
    #[schemars(with = "Option<Vec<NonEmptyStrSchema>>")]
    sheet_ids: Option<Vec<String>>,
    /// Optional output PDF file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// PDF output color space (`srgb`, `cmyk`, or `grayscale`).
    color_space: Option<String>,
    /// Compress PDF content streams.
    compress_streams: Option<bool>,
    /// Optional PDF document title metadata.
    title: Option<String>,
    /// Outline text glyphs into vector paths.
    outline_text: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportPngParams {
    /// Graph or layout sheet ID to rasterize.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional output PNG file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// Explicit raster width in pixels.
    #[serde(default, deserialize_with = "de_opt_u32")]
    #[schemars(range(min = 1))]
    width_px: Option<u32>,
    /// Explicit raster height in pixels.
    #[serde(default, deserialize_with = "de_opt_u32")]
    #[schemars(range(min = 1))]
    height_px: Option<u32>,
    /// Target resolution in dots per inch (default 300).
    #[serde(default, deserialize_with = "de_opt_u16")]
    #[schemars(range(min = 1))]
    dpi: Option<u16>,
    /// Use transparent alpha=0 sheet canvas background.
    transparent_bg: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportTiffParams {
    /// Graph or layout sheet ID to rasterize.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional output TIFF file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
    /// Explicit raster width in pixels.
    #[serde(default, deserialize_with = "de_opt_u32")]
    #[schemars(range(min = 1))]
    width_px: Option<u32>,
    /// Explicit raster height in pixels.
    #[serde(default, deserialize_with = "de_opt_u32")]
    #[schemars(range(min = 1))]
    height_px: Option<u32>,
    /// Target resolution in dots per inch (default 300).
    #[serde(default, deserialize_with = "de_opt_u16")]
    #[schemars(range(min = 1))]
    dpi: Option<u16>,
    /// TIFF compression codec (`none`, `lzw`, `deflate`).
    compression: Option<String>,
    /// Use transparent alpha=0 sheet canvas background.
    transparent_bg: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportEmfParams {
    /// Target sheet ID to export to EMF.
    #[schemars(length(min = 1))]
    sheet_id: String,
    /// Optional output EMF file path.
    #[schemars(length(min = 1))]
    path: Option<String>,
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
    let params: ExportSvgParams = parse_params(&command_schema::<ExportSvgParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;

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
    if let Some(tb) = params.transparent_bg {
        opts.transparent_bg = tb;
    }

    let req = ExportArtifactRequest { target: ExportTarget::Explicit(id.clone()), format: ExportArtifactFormat::Svg { options: opts } };
    let artifact = export_document_artifact(ctx.session.document(), &req, Some(ctx.cancelled), command_id)?;
    artifact.write_if_requested(&ctx.session.path_policy, params.path.as_deref())?;
    let width_pt = artifact.width_pt.unwrap_or(0.0);
    let height_pt = artifact.height_pt.unwrap_or(0.0);
    let warnings = artifact.warnings.clone();
    let svg = artifact.into_utf8_string()?;

    Ok(figure_export_outcome(
        command_id,
        "exported SVG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": svg.len(),
            "width_pt": width_pt,
            "height_pt": height_pt,
            "path": params.path,
            "svg": svg,
        }),
        &warnings,
    ))
}

fn run_export_pdf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_pdf_with_id(ctx, "export.pdf", raw)
}

fn run_graph_export_pdf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_pdf_with_id(ctx, "graph.exportPdf", raw)
}

fn parse_pdf_color_space(raw: Option<&str>) -> Result<PdfColorSpace, EngineError> {
    let Some(token) = raw else {
        return Ok(PdfColorSpace::Srgb);
    };
    crate::services::parse_pdf_color_space(token).map_err(|mut err| {
        err.details = Some(json!({ "pointer": "/color_space", "color_space": token }));
        err
    })
}

fn run_pdf_with_id(ctx: &mut Ctx<'_>, command_id: &str, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: ExportPdfParams = parse_params(&command_schema::<ExportPdfParams>(), raw)?;
    if params.sheet_id.is_some() && params.sheet_ids.is_some() {
        return Err(bad_input_ptr("/sheet_ids", "cannot specify both `sheet_id` and `sheet_ids`")
            .with_remediation("Pass `sheet_id` for a single page or `sheet_ids` for a multi-page PDF, not both."));
    }

    let color_space = parse_pdf_color_space(params.color_space.as_deref())?;
    let mut opts = PdfExportOptions { color_space, ..PdfExportOptions::default() };
    if let Some(comp) = params.compress_streams {
        opts.compress_streams = comp;
    }
    if let Some(title) = params.title {
        opts.title = Some(title);
    }
    if let Some(outline) = params.outline_text {
        opts.outline_text = outline;
    }

    let (target, sheet_ids) = if let Some(ids) = params.sheet_ids {
        let mut parsed_ids = Vec::with_capacity(ids.len());
        for raw_id in ids {
            parsed_ids.push(parse_sheet_id(&raw_id)?);
        }
        (ExportTarget::AutoOrExplicit(None), Some(parsed_ids))
    } else {
        let raw_id = params.sheet_id.ok_or_else(|| {
            bad_input_ptr("/sheet_id", "missing required parameter `sheet_id` or `sheet_ids`")
                .with_remediation("Pass `sheet_id` for a single page or `sheet_ids` for a multi-page PDF.")
        })?;
        (ExportTarget::Explicit(parse_sheet_id(&raw_id)?), None)
    };

    let req = ExportArtifactRequest { target, format: ExportArtifactFormat::Pdf { options: opts, all_graphs: false, sheet_ids } };
    let artifact = export_document_artifact(ctx.session.document(), &req, Some(ctx.cancelled), command_id)?;
    artifact.write_if_requested(&ctx.session.path_policy, params.path.as_deref())?;

    Ok(figure_export_outcome(
        command_id,
        "exported PDF figure",
        json!({
            "sheet_id": artifact.target_id,
            "byte_len": artifact.bytes.len(),
            "page_count": artifact.page_count,
            "width_pt": artifact.width_pt.unwrap_or(0.0),
            "height_pt": artifact.height_pt.unwrap_or(0.0),
            "color_space": color_space,
            "path": params.path,
            "base64": encode_base64(&artifact.bytes),
        }),
        &artifact.warnings,
    ))
}

fn run_export_png(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_png_with_id(ctx, "export.png", raw)
}

fn run_graph_export_png(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_png_with_id(ctx, "graph.exportPng", raw)
}

fn run_png_with_id(ctx: &mut Ctx<'_>, command_id: &str, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: ExportPngParams = parse_params(&command_schema::<ExportPngParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let req = ExportArtifactRequest {
        target: ExportTarget::Explicit(id.clone()),
        format: ExportArtifactFormat::Png {
            width_px: params.width_px,
            height_px: params.height_px,
            dpi: params.dpi,
            sizing: RasterSizingPolicy::StrictLimits,
            transparent_bg: params.transparent_bg.unwrap_or(false),
        },
    };
    let artifact = export_document_artifact(ctx.session.document(), &req, Some(ctx.cancelled), command_id)?;
    artifact.write_if_requested(&ctx.session.path_policy, params.path.as_deref())?;
    let raster = artifact.raster.ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "PNG export missing raster metadata"))?;

    Ok(figure_export_outcome(
        command_id,
        "exported PNG figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": artifact.bytes.len(),
            "width_px": raster.width_px,
            "height_px": raster.height_px,
            "dpi": raster.dpi,
            "path": params.path,
            "base64": encode_base64(&artifact.bytes),
        }),
        &artifact.warnings,
    ))
}

fn parse_tiff_compression(raw: Option<&str>) -> Result<(&'static str, TiffCompression), EngineError> {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        None | Some("deflate") => Ok(("deflate", TiffCompression::Deflate)),
        Some("none") => Ok(("none", TiffCompression::None)),
        Some("lzw") => Ok(("lzw", TiffCompression::Lzw)),
        Some(other) => Err(bad_input(format!("unknown TIFF compression `{other}`"))
            .with_remediation("Use `none`, `lzw`, or `deflate`.")
            .with_details(json!({ "pointer": "/compression", "compression": other }))),
    }
}

fn run_export_tiff(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: ExportTiffParams = parse_params(&command_schema::<ExportTiffParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let (compression_label, compression) = parse_tiff_compression(params.compression.as_deref())?;
    let req = ExportArtifactRequest {
        target: ExportTarget::Explicit(id.clone()),
        format: ExportArtifactFormat::Tiff {
            width_px: params.width_px,
            height_px: params.height_px,
            dpi: params.dpi,
            compression,
            sizing: RasterSizingPolicy::StrictLimits,
            transparent_bg: params.transparent_bg.unwrap_or(false),
        },
    };
    let artifact = export_document_artifact(ctx.session.document(), &req, Some(ctx.cancelled), "export.tiff")?;
    artifact.write_if_requested(&ctx.session.path_policy, params.path.as_deref())?;
    let raster = artifact.raster.ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "TIFF export missing raster metadata"))?;

    Ok(figure_export_outcome(
        "export.tiff",
        "exported TIFF figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": artifact.bytes.len(),
            "width_px": raster.width_px,
            "height_px": raster.height_px,
            "dpi": raster.dpi,
            "compression": compression_label,
            "path": params.path,
            "base64": encode_base64(&artifact.bytes),
        }),
        &artifact.warnings,
    ))
}

fn run_graph_export_emf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_emf_with_kind(ctx, "graph.exportEmf", EmfSheetKind::Graph, raw)
}

fn run_layout_export_emf(ctx: &mut Ctx<'_>, raw: Value) -> Result<CommandOutcome, EngineError> {
    run_emf_with_kind(ctx, "layout.exportEmf", EmfSheetKind::Layout, raw)
}

fn run_emf_with_kind(ctx: &mut Ctx<'_>, command_id: &str, kind: EmfSheetKind, raw: Value) -> Result<CommandOutcome, EngineError> {
    let params: ExportEmfParams = parse_params(&command_schema::<ExportEmfParams>(), raw)?;
    let id = parse_sheet_id(&params.sheet_id)?;
    let req = ExportArtifactRequest { target: ExportTarget::Explicit(id.clone()), format: ExportArtifactFormat::Emf { require_kind: Some(kind) } };
    let artifact = export_document_artifact(ctx.session.document(), &req, Some(ctx.cancelled), command_id)?;
    let bytes_written = if params.path.is_some() {
        artifact.write_if_requested(&ctx.session.path_policy, params.path.as_deref())?;
        artifact.bytes.len()
    } else {
        0
    };
    let magic = artifact
        .bytes
        .get(40..44)
        .and_then(|slice| <[u8; 4]>::try_from(slice).ok())
        .map_or_else(String::new, |bytes| format!("0x{:08X}", u32::from_le_bytes(bytes)));

    Ok(figure_export_outcome(
        command_id,
        "exported EMF figure",
        json!({
            "sheet_id": id.as_str(),
            "byte_len": artifact.bytes.len(),
            "bytes_written": bytes_written,
            "header_magic": magic,
            "width_pt": artifact.width_pt.unwrap_or(0.0),
            "height_pt": artifact.height_pt.unwrap_or(0.0),
            "path": params.path,
            "base64": encode_base64(&artifact.bytes),
        }),
        &artifact.warnings,
    ))
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, DataSheet, ErrorCode, GraphKind, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable,
    };
    use serde_json::json;

    use super::*;
    use crate::Session;

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

    fn parse_png_ihdr_and_phys(bytes: &[u8]) -> (u32, u32, Option<(u32, u32, u8)>) {
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "PNG signature missing");
        let mut pos = 8_usize;
        let mut width = 0_u32;
        let mut height = 0_u32;
        let mut phys = None;
        while pos + 12 <= bytes.len() {
            let len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
            let tag = &bytes[pos + 4..pos + 8];
            let data = &bytes[pos + 8..pos + 8 + len];
            if tag == b"IHDR" && data.len() >= 8 {
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            } else if tag == b"pHYs" && data.len() == 9 {
                let x = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                let y = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                phys = Some((x, y, data[8]));
            }
            pos += 12 + len;
        }
        (width, height, phys)
    }

    fn parse_tiff_dimensions_and_dpi(bytes: &[u8]) -> (u32, u32, (u32, u32), (u32, u32), u16) {
        assert!(bytes.starts_with(b"II\x2a\x00"), "little-endian TIFF header missing");
        let u16_le = |off: usize| u16::from_le_bytes([bytes[off], bytes[off + 1]]);
        let u32_le = |off: usize| u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
        let ifd = u32_le(4) as usize;
        let count = u16_le(ifd) as usize;
        let mut width = 0_u32;
        let mut height = 0_u32;
        let mut x_res = (0_u32, 0_u32);
        let mut y_res = (0_u32, 0_u32);
        let mut res_unit = 0_u16;
        for idx in 0..count {
            let entry = ifd + 2 + idx * 12;
            let tag = u16_le(entry);
            let field_type = u16_le(entry + 2);
            let val_or_off = u32_le(entry + 8);
            match tag {
                256 => width = val_or_off,
                257 => height = val_or_off,
                282 if field_type == 5 => {
                    let off = val_or_off as usize;
                    x_res = (u32_le(off), u32_le(off + 4));
                }
                283 if field_type == 5 => {
                    let off = val_or_off as usize;
                    y_res = (u32_le(off), u32_le(off + 4));
                }
                296 => res_unit = u16_le(entry + 8),
                _ => {}
            }
        }
        (width, height, x_res, y_res, res_unit)
    }

    #[test]
    fn dpi_raster_export_preserves_aspect_ratio_and_enforces_limits() {
        let mut doc = doc_with_graph_and_layout();
        let wide_spec =
            LayoutSpec { width: 2000.0, height: 60.0, unit: curvebook_doc::PhysicalUnit::Points, grid_template: None, ..LayoutSpec::default() };
        doc.add_layout_sheet(LayoutSheet::new("l_wide", "Wide Banner", wide_spec).unwrap()).unwrap();
        let sq_spec =
            LayoutSpec { width: 1000.0, height: 1000.0, unit: curvebook_doc::PhysicalUnit::Points, grid_template: None, ..LayoutSpec::default() };
        doc.add_layout_sheet(LayoutSheet::new("l_sq", "Large Square", sq_spec).unwrap()).unwrap();

        let mut session = Session::from_document(doc);
        let png_res = session
            .execute("export.png", json!({ "sheet_id": "l_wide", "dpi": 360 }))
            .expect("wide 360 DPI layout fits MAX_RASTER_DIM_PX and MAX_RASTER_PIXELS");
        assert_eq!(png_res.payload["width_px"], json!(10_000));
        assert_eq!(png_res.payload["height_px"], json!(300));
        assert_eq!(png_res.payload["dpi"], json!(360));
        let png_bytes = crate::cmd::helpers::decode_base64(png_res.payload["base64"].as_str().expect("base64 string")).expect("decode png base64");
        let (pw, ph, phys) = parse_png_ihdr_and_phys(&png_bytes);
        assert_eq!((pw, ph), (10_000, 300));
        let expected_ppm = (360.0_f64 / 0.0254).round() as u32;
        assert_eq!(phys, Some((expected_ppm, expected_ppm, 1)));

        let graph_png =
            session.execute("graph.exportPng", json!({ "sheet_id": "g1", "dpi": 360 })).expect("graph.exportPng honors uniform DPI scaling");
        assert_eq!(graph_png.payload["dpi"], json!(360));
        let graph_png_bytes =
            crate::cmd::helpers::decode_base64(graph_png.payload["base64"].as_str().expect("base64 string")).expect("decode graph png base64");
        let (gw, gh, g_phys) = parse_png_ihdr_and_phys(&graph_png_bytes);
        assert_eq!(gw, graph_png.payload["width_px"].as_u64().expect("width_px") as u32);
        assert_eq!(gh, graph_png.payload["height_px"].as_u64().expect("height_px") as u32);
        assert_eq!(g_phys, Some((expected_ppm, expected_ppm, 1)));

        let tiff_res = session
            .execute("export.tiff", json!({ "sheet_id": "l_wide", "dpi": 360 }))
            .expect("wide 360 DPI TIFF fits MAX_RASTER_DIM_PX and MAX_RASTER_PIXELS");
        assert_eq!(tiff_res.payload["width_px"], json!(10_000));
        assert_eq!(tiff_res.payload["height_px"], json!(300));
        assert_eq!(tiff_res.payload["dpi"], json!(360));
        let tiff_bytes = crate::cmd::helpers::decode_base64(tiff_res.payload["base64"].as_str().expect("base64 string")).expect("decode tiff base64");
        let (tw, th, xr, yr, ru) = parse_tiff_dimensions_and_dpi(&tiff_bytes);
        assert_eq!((tw, th), (10_000, 300));
        assert_eq!((xr, yr, ru), ((360, 1), (360, 1), 2));

        // 720 DPI on 2000x60 pt asks for 20,000x600 (> MAX_RASTER_DIM_PX = 16,384): must fail cleanly.
        let err_png_dim =
            session.execute("export.png", json!({ "sheet_id": "l_wide", "dpi": 720 })).expect_err("oversized DPI width must return ResourceLimit");
        assert_eq!(err_png_dim.code, ErrorCode::ResourceLimit);
        let err_tiff_dim = session
            .execute("export.tiff", json!({ "sheet_id": "l_wide", "dpi": 720 }))
            .expect_err("oversized TIFF DPI width must return ResourceLimit");
        assert_eq!(err_tiff_dim.code, ErrorCode::ResourceLimit);

        // 600 DPI on 1000x1000 pt asks for 8333x8333 = 69,438,889 px (> MAX_RASTER_PIXELS): must fail cleanly.
        let err_png_pixels = session
            .execute("export.png", json!({ "sheet_id": "l_sq", "dpi": 600 }))
            .expect_err("over-pixel-budget PNG DPI must return ResourceLimit");
        assert_eq!(err_png_pixels.code, ErrorCode::ResourceLimit);
        let err_tiff_pixels = session
            .execute("export.tiff", json!({ "sheet_id": "l_sq", "dpi": 600 }))
            .expect_err("over-pixel-budget TIFF DPI must return ResourceLimit");
        assert_eq!(err_tiff_pixels.code, ErrorCode::ResourceLimit);

        let err_graph_dpi = session
            .execute("graph.exportPng", json!({ "sheet_id": "g1", "dpi": 5000 }))
            .expect_err("graph.exportPng oversized DPI must return ResourceLimit");
        assert_eq!(err_graph_dpi.code, ErrorCode::ResourceLimit);
    }

    fn tiff_first_pixel_rgba(tiff_bytes: &[u8]) -> [u8; 4] {
        let ifd_off = u32::from_le_bytes(tiff_bytes[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(tiff_bytes[ifd_off..ifd_off + 2].try_into().unwrap()) as usize;
        let mut strip_off = 0usize;
        for i in 0..count {
            let e = ifd_off + 2 + i * 12;
            let tag = u16::from_le_bytes(tiff_bytes[e..e + 2].try_into().unwrap());
            if tag == 273 {
                strip_off = u32::from_le_bytes(tiff_bytes[e + 8..e + 12].try_into().unwrap()) as usize;
            }
        }
        [tiff_bytes[strip_off], tiff_bytes[strip_off + 1], tiff_bytes[strip_off + 2], tiff_bytes[strip_off + 3]]
    }

    #[test]
    fn transparent_bg_command_flag_works_for_png_tiff_and_svg() {
        let doc = doc_with_graph_and_layout();
        let mut session = Session::from_document(doc);

        // SVG: default (opaque) includes white paper fill; transparent_bg=true omits full-canvas background rect.
        let svg_opaque = session.execute("graph.exportSvg", json!({ "sheet_id": "g1" })).expect("opaque svg");
        let svg_opaque_str = svg_opaque.payload["svg"].as_str().expect("opaque svg str");
        let svg_trans = session.execute("graph.exportSvg", json!({ "sheet_id": "g1", "transparent_bg": true })).expect("transparent svg");
        let svg_trans_str = svg_trans.payload["svg"].as_str().expect("transparent svg str");
        assert!(
            svg_opaque_str.contains("d=\"M0.000 0.000 L360.000 0.000 L360.000 260.000 L0.000 260.000 Z\" fill=\"#FFFFFF\""),
            "opaque SVG must include white paper fill: {svg_opaque_str}"
        );
        assert!(
            !svg_trans_str.contains("d=\"M0.000 0.000 L360.000 0.000 L360.000 260.000 L0.000 260.000 Z\" fill=\"#FFFFFF\""),
            "transparent SVG must omit full-canvas white paper fill: {svg_trans_str}"
        );

        // TIFF (uncompressed): corner pixel alpha == 255 when opaque, alpha == 0 when transparent_bg=true.
        let tiff_opaque = session
            .execute("export.tiff", json!({ "sheet_id": "g1", "width_px": 180, "height_px": 130, "dpi": 72, "compression": "none" }))
            .expect("opaque tiff");
        let tiff_opaque_bytes =
            crate::cmd::helpers::decode_base64(tiff_opaque.payload["base64"].as_str().expect("base64")).expect("decode opaque tiff");
        assert_eq!(tiff_first_pixel_rgba(&tiff_opaque_bytes)[3], 255, "opaque TIFF corner alpha must be 255");

        let tiff_trans = session
            .execute(
                "export.tiff",
                json!({ "sheet_id": "g1", "width_px": 180, "height_px": 130, "dpi": 72, "compression": "none", "transparent_bg": true }),
            )
            .expect("transparent tiff");
        let tiff_trans_bytes =
            crate::cmd::helpers::decode_base64(tiff_trans.payload["base64"].as_str().expect("base64")).expect("decode transparent tiff");
        assert_eq!(tiff_first_pixel_rgba(&tiff_trans_bytes), [0, 0, 0, 0], "transparent TIFF corner pixel must have alpha=0");

        // PNG: opaque vs transparent_bg=true differ in encoded payload and both have valid PNG headers.
        let png_opaque = session
            .execute("graph.exportPng", json!({ "sheet_id": "g1", "width_px": 180, "height_px": 130, "dpi": 72, "transparent_bg": false }))
            .expect("opaque png");
        let png_trans = session
            .execute("graph.exportPng", json!({ "sheet_id": "g1", "width_px": 180, "height_px": 130, "dpi": 72, "transparent_bg": true }))
            .expect("transparent png");
        let png_opaque_bytes = crate::cmd::helpers::decode_base64(png_opaque.payload["base64"].as_str().expect("base64")).expect("decode opaque png");
        let png_trans_bytes = crate::cmd::helpers::decode_base64(png_trans.payload["base64"].as_str().expect("base64")).expect("decode trans png");
        assert!(png_opaque_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png_trans_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_ne!(png_opaque_bytes, png_trans_bytes, "transparent_bg=true PNG must differ from opaque PNG");
    }
}
