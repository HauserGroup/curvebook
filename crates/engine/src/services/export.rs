//! Document artifact export services, raster/PDF/CSV option parsing, and SFNT font validation.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_chart::{ChartScene, ChartWarning, compile_graph, compile_layout};
use curvebook_doc::{DataSheet, ErrorCode, ProjectDocument, ResultSheet, SheetId};
use curvebook_format::{CsvOptions, export_datasheet_to_csv, export_result_sheet_json, try_export_result_sheet_tidy_csv, try_export_result_tab_csv};
use curvebook_pdf::{
    MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfColorSpace, PdfExportOptions, PdfVersion, render_pdf, render_pdf_page_refs, render_pdf_pages,
};
use curvebook_render::{
    MAX_RASTER_DIM_PX, MAX_RASTER_PIXELS, RenderOptions, RenderTargetSpec, TiffCompression, encode_emf, encode_png, encode_tiff,
    render_scene_with_options,
};
use curvebook_svg::{SvgExportOptions, render_svg};
use curvebook_text::{TrueTypeFontInfo, parse_truetype_font_info};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, ExternalPathPolicy};

/// Minimum supported raster DPI for bounded preview and CLI/MCP exports.
pub const MIN_RASTER_DPI: u16 = 18;

/// Maximum supported raster DPI for bounded preview and CLI/MCP exports.
pub const MAX_RASTER_DPI: u16 = 2400;

/// Default raster DPI used for interactive MCP and control-channel previews.
pub const DEFAULT_PREVIEW_DPI: u16 = 144;

/// Default raster DPI used for CLI publication exports.
pub const DEFAULT_EXPORT_DPI: u16 = 300;

/// Maximum TrueType SFNT font file read size (`64 MiB`).
pub const MAX_FONT_FILE_BYTES: usize = 64 * 1024 * 1024;

/// Target sheet selection strategy for [`export_document_artifact`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportTarget {
    /// Require the exact sheet identifier (returns [`ErrorCode::NotFound`] if absent or incompatible).
    Explicit(SheetId),
    /// Use the specified sheet if provided, otherwise fall back to the active sheet or first matching sheet.
    AutoOrExplicit(Option<SheetId>),
}

/// Optional sheet-kind restriction when exporting EMF vector graphics.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum EmfSheetKind {
    /// Target must be a graph sheet.
    Graph,
    /// Target must be a layout sheet.
    Layout,
}

/// Raster dimension enforcement policy for PNG and TIFF exports.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RasterSizingPolicy {
    /// Reject dimensions or DPI that exceed [`MAX_RASTER_DIM_PX`] or [`MAX_RASTER_PIXELS`] with
    /// [`ErrorCode::ResourceLimit`] (used by `export.png`, `export.tiff`, `graph.exportPng`).
    StrictLimits,
    /// Proportionally cap oversized dimensions to the bounded interactive preview budget via
    /// [`RenderTargetSpec::from_scene_capped`] (used by MCP and live-control preview/export endpoints).
    CapToLimits {
        /// Default DPI when the caller omits `dpi`.
        default_dpi: u16,
    },
}

/// Canonical export artifact format identifier shared across MCP, CLI, and live-control handlers.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DocumentExportFormat {
    /// Scalable Vector Graphics (SVG).
    Svg,
    /// Portable Document Format (PDF).
    Pdf,
    /// Windows Enhanced Metafile (EMF).
    Emf,
    /// Portable Network Graphics (PNG).
    Png,
    /// Tagged Image File Format (TIFF).
    Tiff,
    /// Comma-separated values (CSV).
    Csv,
    /// Tab-separated values (TSV).
    Tsv,
    /// JavaScript Object Notation (JSON).
    Json,
}

impl DocumentExportFormat {
    /// Parse a case-insensitive export format token (`svg`, `pdf`, `emf`, `png`, `tiff`, `csv`, `tsv`, `json`).
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::BadInput`] for unsupported format strings.
    pub fn parse(raw: &str) -> Result<Self, EngineError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "svg" => Ok(Self::Svg),
            "pdf" => Ok(Self::Pdf),
            "emf" => Ok(Self::Emf),
            "png" => Ok(Self::Png),
            "tiff" => Ok(Self::Tiff),
            "csv" => Ok(Self::Csv),
            "tsv" => Ok(Self::Tsv),
            "json" => Ok(Self::Json),
            other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported export format '{other}'"))
                .with_details(json!({ "pointer": "/format", "format": other }))),
        }
    }

    /// Canonical lowercase format name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
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

/// Format-agnostic export configuration assembled before building an [`ExportArtifactRequest`].
#[derive(Debug, Clone)]
pub struct ExportOptionsSpec {
    /// Selected artifact format.
    pub format: DocumentExportFormat,
    /// Explicit sheet ID (or `None` for active/auto target selection).
    pub sheet_id: Option<SheetId>,
    /// Pretty-print SVG/JSON output.
    pub pretty: bool,
    /// Emit tidy long-form CSV for result sheets.
    pub tidy: bool,
    /// Optional result tab ID (requires `tidy == false`).
    pub tab_id: Option<String>,
    /// Optional zero-based result tab index (requires `tidy == false`).
    pub tab_index: Option<usize>,
    /// Explicit CSV/TSV delimiter override.
    pub csv_options: Option<CsvOptions>,
    /// Optional SVG coordinate precision (`1..=8`).
    pub svg_precision: Option<u8>,
    /// Convert SVG text spans into glyph outline paths.
    pub svg_outline_text: bool,
    /// PDF color space.
    pub pdf_color_space: PdfColorSpace,
    /// PDF specification version (`1.4` or `1.7`).
    pub pdf_version: PdfVersion,
    /// Convert PDF text spans into glyph outline paths.
    pub pdf_outline_text: bool,
    /// Optional PDF document title metadata.
    pub pdf_title: Option<String>,
    /// Optional validated SFNT TrueType font bytes for PDF embedding.
    pub embedded_ttf_font: Option<Vec<u8>>,
    /// Export every graph and layout sheet into a multi-page PDF.
    pub all_graphs: bool,
    /// Explicit ordered sheet IDs for multi-page PDF export.
    pub pdf_sheet_ids: Option<Vec<SheetId>>,
    /// Optional explicit raster width in pixels.
    pub width_px: Option<u32>,
    /// Optional explicit raster height in pixels.
    pub height_px: Option<u32>,
    /// Optional raster DPI.
    pub dpi: Option<u16>,
    /// TIFF compression codec.
    pub tiff_compression: TiffCompression,
    /// Raster sizing policy (`StrictLimits` vs `CapToLimits`).
    pub sizing: RasterSizingPolicy,
    /// Use a transparent (`alpha = 0`) sheet canvas background for SVG, PNG, and TIFF exports.
    pub transparent_bg: bool,
    /// Optional EMF sheet-kind restriction.
    pub emf_require_kind: Option<EmfSheetKind>,
}

impl ExportOptionsSpec {
    /// Construct default export options for `format` (capped preview defaults).
    #[must_use]
    pub const fn new(format: DocumentExportFormat) -> Self {
        Self {
            format,
            sheet_id: None,
            pretty: true,
            tidy: true,
            tab_id: None,
            tab_index: None,
            csv_options: None,
            svg_precision: None,
            svg_outline_text: false,
            pdf_color_space: PdfColorSpace::Srgb,
            pdf_version: PdfVersion::Pdf17,
            pdf_outline_text: false,
            pdf_title: None,
            embedded_ttf_font: None,
            all_graphs: false,
            pdf_sheet_ids: None,
            width_px: None,
            height_px: None,
            dpi: None,
            tiff_compression: TiffCompression::Lzw,
            sizing: RasterSizingPolicy::CapToLimits { default_dpi: DEFAULT_PREVIEW_DPI },
            transparent_bg: false,
            emf_require_kind: None,
        }
    }

    /// Assemble an [`ExportArtifactRequest`] against `doc`, resolving CSV/TSV target sheets cleanly.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::BadInput`] if `tidy` conflicts with `tab_id`/`tab_index`,
    /// or [`ErrorCode::NotFound`] if a requested `tab_id` does not exist on any result sheet.
    pub fn into_artifact_request(
        self,
        doc: &ProjectDocument,
        prefer_result_when_active_non_tabular: bool,
    ) -> Result<ExportArtifactRequest, EngineError> {
        if self.tidy && (self.tab_id.is_some() || self.tab_index.is_some()) {
            return Err(EngineError::with_code(
                ErrorCode::BadInput,
                "export `tidy: true` cannot be combined with `tab_id` or `tab_index`; omit `tidy` or pass `tidy: false` to export a specific tab",
            ));
        }
        let default_target = ExportTarget::AutoOrExplicit(self.sheet_id.clone());
        let (target, artifact_format) = match self.format {
            DocumentExportFormat::Svg => {
                let mut options = SvgExportOptions {
                    pretty: self.pretty,
                    outline_text: self.svg_outline_text,
                    transparent_bg: self.transparent_bg,
                    ..SvgExportOptions::default()
                };
                if let Some(p) = self.svg_precision {
                    options.precision = p;
                }
                (default_target, ExportArtifactFormat::Svg { options })
            }
            DocumentExportFormat::Pdf => (
                default_target,
                ExportArtifactFormat::Pdf {
                    options: PdfExportOptions {
                        color_space: self.pdf_color_space,
                        version: self.pdf_version,
                        outline_text: self.pdf_outline_text,
                        title: self.pdf_title,
                        embedded_truetype_font: self.embedded_ttf_font,
                        ..PdfExportOptions::default()
                    },
                    all_graphs: self.all_graphs,
                    sheet_ids: self.pdf_sheet_ids,
                },
            ),
            DocumentExportFormat::Emf => (default_target, ExportArtifactFormat::Emf { require_kind: self.emf_require_kind }),
            DocumentExportFormat::Png => (
                default_target,
                ExportArtifactFormat::Png {
                    width_px: self.width_px,
                    height_px: self.height_px,
                    dpi: self.dpi,
                    sizing: self.sizing,
                    transparent_bg: self.transparent_bg,
                },
            ),
            DocumentExportFormat::Tiff => (
                default_target,
                ExportArtifactFormat::Tiff {
                    width_px: self.width_px,
                    height_px: self.height_px,
                    dpi: self.dpi,
                    compression: self.tiff_compression,
                    sizing: self.sizing,
                    transparent_bg: self.transparent_bg,
                },
            ),
            DocumentExportFormat::Csv => (
                resolve_csv_export_target(doc, self.sheet_id.as_ref(), self.tab_id.as_deref(), prefer_result_when_active_non_tabular)?,
                ExportArtifactFormat::Csv {
                    options: self.csv_options.unwrap_or_else(CsvOptions::csv),
                    tab_id: self.tab_id,
                    tab_index: self.tab_index,
                    tidy: self.tidy,
                },
            ),
            DocumentExportFormat::Tsv => (
                resolve_csv_export_target(doc, self.sheet_id.as_ref(), self.tab_id.as_deref(), prefer_result_when_active_non_tabular)?,
                ExportArtifactFormat::Csv {
                    options: self.csv_options.unwrap_or_else(CsvOptions::tsv),
                    tab_id: self.tab_id,
                    tab_index: self.tab_index,
                    tidy: self.tidy,
                },
            ),
            DocumentExportFormat::Json => (default_target, ExportArtifactFormat::Json { pretty: self.pretty }),
        };
        Ok(ExportArtifactRequest { target, format: artifact_format })
    }
}

/// Output format and format-specific options for [`export_document_artifact`].
#[derive(Debug, Clone)]
pub enum ExportArtifactFormat {
    /// Scalable Vector Graphics (SVG) export of a graph or layout sheet.
    Svg {
        /// SVG renderer options.
        options: SvgExportOptions,
    },
    /// Single-page, explicit multi-page, or `all_graphs` PDF export.
    Pdf {
        /// PDF renderer options.
        options: PdfExportOptions,
        /// When `true` and neither an explicit target nor `sheet_ids` is set, export every graph
        /// and layout sheet in document order subject to [`MAX_PDF_PAGES`] and
        /// [`MAX_PDF_TOTAL_PRIMITIVES`].
        all_graphs: bool,
        /// Explicit ordered multi-page sheet list (overrides single-sheet target when `Some`).
        sheet_ids: Option<Vec<SheetId>>,
    },
    /// Raster PNG export of a graph or layout sheet.
    Png {
        /// Optional explicit width in pixels.
        width_px: Option<u32>,
        /// Optional explicit height in pixels.
        height_px: Option<u32>,
        /// Optional target resolution in DPI.
        dpi: Option<u16>,
        /// Dimension/pixel-budget handling policy.
        sizing: RasterSizingPolicy,
        /// Use a transparent (`alpha = 0`) sheet canvas background when `true`.
        transparent_bg: bool,
    },
    /// Raster TIFF export of a graph or layout sheet.
    Tiff {
        /// Optional explicit width in pixels.
        width_px: Option<u32>,
        /// Optional explicit height in pixels.
        height_px: Option<u32>,
        /// Optional target resolution in DPI.
        dpi: Option<u16>,
        /// TIFF compression codec.
        compression: TiffCompression,
        /// Dimension/pixel-budget handling policy.
        sizing: RasterSizingPolicy,
        /// Use a transparent (`alpha = 0`) sheet canvas background when `true`.
        transparent_bg: bool,
    },
    /// Windows Enhanced Metafile (EMF) export of a graph or layout sheet.
    Emf {
        /// Optional strict sheet-kind requirement (`Graph` vs `Layout`).
        require_kind: Option<EmfSheetKind>,
    },
    /// Delimited CSV export of a data sheet or result sheet (guarded against failed result sheets).
    Csv {
        /// Delimiter and quoting options.
        options: CsvOptions,
        /// Optional result tab ID (requires a [`ResultSheet`] and `tidy == false`).
        tab_id: Option<String>,
        /// Optional zero-based result tab index (requires a [`ResultSheet`] and `tidy == false`).
        tab_index: Option<usize>,
        /// Emit tidy long-form CSV for result sheets when `true`.
        tidy: bool,
    },
    /// JSON export of a specific sheet or the entire [`ProjectDocument`].
    Json {
        /// Pretty-print JSON output when `true`.
        pretty: bool,
    },
}

/// Request payload for [`export_document_artifact`].
#[derive(Debug, Clone)]
pub struct ExportArtifactRequest {
    /// Target sheet selection mode.
    pub target: ExportTarget,
    /// Output format and options.
    pub format: ExportArtifactFormat,
}

/// Pixel dimensions and DPI metadata for raster exports.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterMetadata {
    /// Rendered raster width in pixels.
    pub width_px: u32,
    /// Rendered raster height in pixels.
    pub height_px: u32,
    /// Target resolution in dots per inch.
    pub dpi: u16,
}

/// Rendered export artifact returned by [`export_document_artifact`].
#[derive(Debug, Clone, PartialEq)]
pub struct ExportedArtifact {
    /// Resolved sheet ID, `"all"`, `"multi-page"`, or `"project"`.
    pub target_id: String,
    /// Encoded output bytes (UTF-8 for SVG/CSV/JSON, binary for PDF/PNG/TIFF/EMF).
    pub bytes: Vec<u8>,
    /// Number of rendered pages (1 for single figures and tabular exports).
    pub page_count: usize,
    /// Scene width in typographic points for single/first visual page exports.
    pub width_pt: Option<f64>,
    /// Scene height in typographic points for single/first visual page exports.
    pub height_pt: Option<f64>,
    /// Non-fatal scene compilation warnings, such as a fitted curve omitted because its fit failed or is pending.
    pub warnings: Vec<ChartWarning>,
    /// Pixel metadata for raster exports (`Png` / `Tiff`).
    pub raster: Option<RasterMetadata>,
}

impl ExportedArtifact {
    /// Decode UTF-8 text bytes (used for SVG, CSV, and JSON exports).
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::InternalError`] if `self.bytes` is not valid UTF-8.
    pub fn into_utf8_string(self) -> Result<String, EngineError> {
        String::from_utf8(self.bytes)
            .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("exported UTF-8 artifact was invalid: {err}")))
    }

    /// Atomically write `self.bytes` to `path` under `policy` when `path` is `Some`.
    ///
    /// # Errors
    /// Returns [`EngineError`] with [`ErrorCode::BadInput`] for blank paths, [`ErrorCode::PathEscape`]
    /// on policy violations, [`ErrorCode::IoError`] on write failure, or [`ErrorCode::UnsupportedFormat`]
    /// on `wasm32`.
    pub fn write_if_requested(&self, policy: &ExternalPathPolicy, path: Option<&str>) -> Result<(), EngineError> {
        let Some(dest) = path else {
            return Ok(());
        };
        if dest.trim().is_empty() {
            return Err(EngineError::with_code(ErrorCode::BadInput, "`path` must not be empty")
                .with_remediation("Provide a non-empty destination file path or omit `path` to return exported bytes inline.")
                .with_details(json!({ "pointer": "/path" })));
        }
        policy.write_atomic_bytes(dest, &self.bytes)
    }
}

/// Parse a PDF specification version string (`1.4` or `1.7`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for unsupported PDF version strings.
pub fn parse_pdf_version(raw: &str) -> Result<PdfVersion, EngineError> {
    match raw.trim() {
        "1.4" => Ok(PdfVersion::Pdf14),
        "1.7" => Ok(PdfVersion::Pdf17),
        other => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported pdf_version '{other}' (expected '1.4' or '1.7')"))
            .with_remediation("Use `1.4` or `1.7`.")
            .with_details(json!({ "pointer": "/pdf_version", "pdf_version": other }))),
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

/// Parse a PDF color space token (`srgb`, `rgb`, `cmyk`, `grayscale`, `gray`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for unsupported color spaces.
pub fn parse_pdf_color_space(raw: &str) -> Result<PdfColorSpace, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "srgb" | "rgb" => Ok(PdfColorSpace::Srgb),
        "cmyk" => Ok(PdfColorSpace::Cmyk),
        "grayscale" | "gray" => Ok(PdfColorSpace::Grayscale),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported pdf_color_space '{raw}'"))
            .with_remediation("Use `srgb`, `cmyk`, or `grayscale`.")
            .with_details(json!({ "pointer": "/pdf_color_space", "pdf_color_space": raw }))),
    }
}

/// Canonical lowercase string representation of a [`PdfColorSpace`].
#[must_use]
pub const fn pdf_color_space_str(color_space: PdfColorSpace) -> &'static str {
    match color_space {
        PdfColorSpace::Srgb => "srgb",
        PdfColorSpace::Cmyk => "cmyk",
        PdfColorSpace::Grayscale => "grayscale",
    }
}

/// Parse a TIFF compression codec token (`none`, `deflate`, `lzw`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for unsupported compression codecs.
pub fn parse_tiff_compression(raw: &str) -> Result<TiffCompression, EngineError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(TiffCompression::None),
        "deflate" => Ok(TiffCompression::Deflate),
        "lzw" => Ok(TiffCompression::Lzw),
        _ => Err(EngineError::with_code(ErrorCode::BadInput, format!("unsupported tiff_compression '{raw}'"))
            .with_remediation("Use `none`, `deflate`, or `lzw`.")
            .with_details(json!({ "pointer": "/tiff_compression", "tiff_compression": raw }))),
    }
}

/// Canonical lowercase string representation of a [`TiffCompression`] codec.
#[must_use]
pub const fn tiff_compression_str(compression: TiffCompression) -> &'static str {
    match compression {
        TiffCompression::None => "none",
        TiffCompression::Deflate => "deflate",
        TiffCompression::Lzw => "lzw",
    }
}

/// Parse an optional single-character CSV/TSV delimiter override into [`CsvOptions`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if `delim` is empty or longer than one Unicode scalar.
pub fn parse_csv_delimiter_opt(delim: Option<&str>, field_name: &str) -> Result<Option<CsvOptions>, EngineError> {
    let Some(raw) = delim else {
        return Ok(None);
    };
    let mut chars = raw.chars();
    let Some(ch) = chars.next() else {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("{field_name} must not be empty")));
    };
    if chars.next().is_some() {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("{field_name} must be a single character")));
    }
    let mut opts = CsvOptions::csv();
    opts.delimiter = ch;
    Ok(Some(opts))
}

/// Validate `explicit_tidy` and `tab_id` compatibility, returning the effective `tidy` boolean.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] when `explicit_tidy == Some(true)` and `tab_id.is_some()`.
pub fn resolve_tidy_flag(explicit_tidy: Option<bool>, tab_id: Option<&str>) -> Result<bool, EngineError> {
    if explicit_tidy == Some(true) && tab_id.is_some() {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "export `tidy: true` cannot be combined with `tab_id`; omit `tidy` or pass `tidy: false` to export a specific tab",
        ));
    }
    Ok(explicit_tidy.unwrap_or(tab_id.is_none()))
}

/// Validate a bounded raster DPI value in `MIN_RASTER_DPI..=MAX_RASTER_DPI` (`18..=2400`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] when `dpi` falls outside `18..=2400`.
pub fn validate_dpi(dpi: u16) -> Result<u16, EngineError> {
    if !(MIN_RASTER_DPI..=MAX_RASTER_DPI).contains(&dpi) {
        return Err(EngineError::with_code(ErrorCode::BadInput, format!("dpi must be in {MIN_RASTER_DPI}..={MAX_RASTER_DPI} (got {dpi})"))
            .with_remediation(format!("Provide an integer DPI between {MIN_RASTER_DPI} and {MAX_RASTER_DPI}."))
            .with_details(json!({ "pointer": "/dpi", "dpi": dpi, "min": MIN_RASTER_DPI, "max": MAX_RASTER_DPI })));
    }
    Ok(dpi)
}

/// Parse and validate a bounded raster DPI string (`18..=2400`).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if parsing fails or the value is out of range.
pub fn parse_dpi_str(raw: &str) -> Result<u16, EngineError> {
    let trimmed = raw.trim();
    let dpi: u16 = trimmed.parse().map_err(|_| {
        EngineError::with_code(ErrorCode::BadInput, format!("invalid --dpi `{raw}` (expected integer in {MIN_RASTER_DPI}..={MAX_RASTER_DPI})"))
            .with_details(json!({ "pointer": "/dpi" }))
    })?;
    validate_dpi(dpi)
}

/// Parse an optional JSON `dpi` value (`None` / `null` -> `Ok(None)`) in `18..=2400`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if non-null and not an integer in `18..=2400`.
pub fn parse_dpi_opt(val: Option<&Value>) -> Result<Option<u16>, EngineError> {
    let Some(v) = val else {
        return Ok(None);
    };
    if v.is_null() {
        return Ok(None);
    }
    let Some(num) = v.as_u64() else {
        return Err(
            EngineError::with_code(ErrorCode::BadInput, "parameter \"dpi\" must be a positive integer").with_details(json!({ "pointer": "/dpi" }))
        );
    };
    let parsed = u16::try_from(num).map_err(|_| {
        EngineError::with_code(ErrorCode::BadInput, format!("parameter \"dpi\" must be in [{MIN_RASTER_DPI}, {MAX_RASTER_DPI}]"))
            .with_details(json!({ "pointer": "/dpi" }))
    })?;
    validate_dpi(parsed).map(Some)
}

/// Parse a JSON `dpi` value with `default_dpi` fallback when absent or `null`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] if non-null and not an integer in `18..=2400`.
pub fn parse_dpi(val: Option<&Value>, default_dpi: u16) -> Result<u16, EngineError> {
    Ok(parse_dpi_opt(val)?.unwrap_or(default_dpi))
}

/// Validate raw TrueType SFNT bytes (`0x00010000` or `true` outlines, rejecting `OTTO`/CFF and malformed SFNT).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] when `bytes` is not a valid TrueType SFNT binary.
pub fn validate_ttf_font_bytes(bytes: &[u8], source_label: &str) -> Result<TrueTypeFontInfo, EngineError> {
    if bytes.get(0..4) == Some(b"OTTO") {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unsupported SFNT font `{source_label}`: expected TrueType outlines (`0x00010000` or `true`), not CFF/OTTO"),
        ));
    }
    let info = parse_truetype_font_info(bytes)
        .map_err(|err| EngineError::with_code(ErrorCode::BadInput, format!("invalid TrueType font `{source_label}`: {err}")))?;
    if !matches!(bytes.get(0..4), Some([0x00, 0x01, 0x00, 0x00] | b"true")) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("unsupported SFNT font `{source_label}`: expected TrueType outlines (`0x00010000` or `true`), not CFF/OTTO"),
        ));
    }
    Ok(info)
}

/// Read and validate TrueType SFNT bytes from `path` (bounded by [`MAX_FONT_FILE_BYTES`]).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for invalid regular files or SFNT binaries,
/// [`ErrorCode::ResourceLimit`] if larger than [`MAX_FONT_FILE_BYTES`], or [`ErrorCode::IoError`]
/// on read failures or `wasm32`.
pub fn load_validated_ttf_font_bytes(path: &Path) -> Result<Vec<u8>, EngineError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let bytes = curvebook_platform::fs::read_regular_file_bytes(path, MAX_FONT_FILE_BYTES).map_err(|err| match err {
            curvebook_platform::PlatformError::InvalidPath { reason } => {
                EngineError::with_code(ErrorCode::BadInput, format!("invalid TrueType font `{}`: {reason}", path.display()))
            }
            other => EngineError::from(other),
        })?;
        let label = path.display().to_string();
        let _info = validate_ttf_font_bytes(&bytes, &label)?;
        Ok(bytes)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        Err(EngineError::with_code(ErrorCode::IoError, "reading TrueType font files is not supported on wasm32"))
    }
}

/// Read and validate TrueType SFNT bytes under an explicit [`ExternalPathPolicy`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::PathEscape`] on policy violations, [`ErrorCode::BadInput`]
/// for invalid SFNT binaries or non-regular files, [`ErrorCode::ResourceLimit`] when oversized, or
/// [`ErrorCode::UnsupportedFormat`] on `wasm32`.
pub fn load_validated_ttf_font(policy: &ExternalPathPolicy, raw_path: &str) -> Result<Vec<u8>, EngineError> {
    let bytes = policy.read_binary_file(raw_path, "TrueType font", MAX_FONT_FILE_BYTES)?;
    let _info = validate_ttf_font_bytes(&bytes, raw_path)?;
    Ok(bytes)
}

/// Find a sheet ID by exact ID, exact title, or case-insensitive title match.
pub fn find_sheet_id_by_name<T>(items: &[T], needle: &str, id_of: impl Fn(&T) -> &SheetId, title_of: impl Fn(&T) -> &str) -> Option<SheetId> {
    items
        .iter()
        .find(|item| id_of(item).as_str() == needle)
        .or_else(|| items.iter().find(|item| title_of(item) == needle))
        .or_else(|| items.iter().find(|item| title_of(item).eq_ignore_ascii_case(needle)))
        .map(id_of)
        .cloned()
}

/// Resolve a graph or layout sheet by ID or title, or fall back to the primary figure in `doc`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if the target or any figure is missing.
pub fn resolve_visual_sheet_by_name_or_id(doc: &ProjectDocument, sheet: Option<&str>) -> Result<SheetId, EngineError> {
    if let Some(needle) = sheet {
        if let Some(id) = find_sheet_id_by_name(&doc.graph_sheets, needle, |g| &g.id, |g| &g.title) {
            return Ok(id);
        }
        if let Some(id) = find_sheet_id_by_name(&doc.layout_sheets, needle, |l| &l.id, |l| &l.title) {
            return Ok(id);
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("visual sheet `{needle}` was not found among graphs or layouts")));
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
    Err(EngineError::with_code(ErrorCode::NotFound, "project contains no graph or layout sheets to render"))
}

/// Resolve a data or result sheet by ID or title, or fall back to the first tabular sheet in `doc`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if the target or any tabular sheet is missing.
pub fn resolve_tabular_sheet_by_name_or_id(doc: &ProjectDocument, sheet: Option<&str>) -> Result<SheetId, EngineError> {
    if let Some(needle) = sheet {
        if let Some(id) = find_sheet_id_by_name(&doc.data_sheets, needle, |d| &d.id, |d| &d.title) {
            return Ok(id);
        }
        if let Some(id) = find_sheet_id_by_name(&doc.result_sheets, needle, |r| &r.id, |r| r.title()) {
            return Ok(id);
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("tabular sheet `{needle}` was not found")));
    }
    if let Some(d) = doc.data_sheets.first() {
        return Ok(d.id.clone());
    }
    if let Some(r) = doc.result_sheets.first() {
        return Ok(r.id.clone());
    }
    Err(EngineError::with_code(ErrorCode::NotFound, "project contains no data or result sheets to export"))
}

/// Resolve a JSON export target by ID or title, or fall back to a sole result sheet / whole project.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] when `sheet` is supplied but cannot be found.
pub fn resolve_json_target_by_name_or_id(doc: &ProjectDocument, sheet: Option<&str>) -> Result<(ExportTarget, Option<String>), EngineError> {
    if let Some(needle) = sheet {
        let found = find_sheet_id_by_name(&doc.result_sheets, needle, |r| &r.id, |r| r.title())
            .or_else(|| find_sheet_id_by_name(&doc.data_sheets, needle, |d| &d.id, |d| &d.title))
            .or_else(|| find_sheet_id_by_name(&doc.info_sheets, needle, |s| &s.id, |s| &s.title));
        let id = found.ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{needle}` not found for JSON export")))?;
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

/// Resolve a graph or layout sheet ID and `"graph"` / `"layout"` kind label for preview rendering.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `sheet_id` or any visual sheet is absent.
pub fn resolve_preview_sheet_id(doc: &ProjectDocument, sheet_id: Option<&str>) -> Result<(SheetId, &'static str), EngineError> {
    if let Some(id_str) = sheet_id {
        let id = SheetId::new(id_str);
        if doc.graph_sheet(&id).is_some() {
            return Ok((id, "graph"));
        }
        if doc.layout_sheet(&id).is_some() {
            return Ok((id, "layout"));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("graph or layout sheet '{id_str}' not found")));
    }
    if let Some(active) = &doc.active_sheet_id {
        if doc.graph_sheet(active).is_some() {
            return Ok((active.clone(), "graph"));
        }
        if doc.layout_sheet(active).is_some() {
            return Ok((active.clone(), "layout"));
        }
    }
    if let Some(gs) = doc.graph_sheets.first() {
        return Ok((gs.id.clone(), "graph"));
    }
    if let Some(ls) = doc.layout_sheets.first() {
        return Ok((ls.id.clone(), "layout"));
    }
    Err(EngineError::with_code(ErrorCode::NotFound, "no GraphSheet or LayoutSheet exists in the current document"))
}

/// Resolve the [`ExportTarget`] for CSV/TSV export given optional `sheet_id` and `tab_id`.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `tab_id` is given and no result sheet owns it.
pub fn resolve_csv_export_target(
    doc: &ProjectDocument,
    sheet_id: Option<&SheetId>,
    tab_id: Option<&str>,
    prefer_result_when_active_non_tabular: bool,
) -> Result<ExportTarget, EngineError> {
    if let Some(id) = sheet_id {
        return Ok(ExportTarget::Explicit(id.clone()));
    }
    if let Some(tid) = tab_id {
        if let Some(active) = &doc.active_sheet_id
            && let Some(rs) = doc.result_sheet(active)
            && rs.tabs.iter().any(|t| t.id == tid)
        {
            return Ok(ExportTarget::Explicit(rs.id.clone()));
        }
        if let Some(rs) = doc.result_sheets.iter().find(|rs| rs.tabs.iter().any(|t| t.id == tid)) {
            return Ok(ExportTarget::Explicit(rs.id.clone()));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("tab '{tid}' not found on any result sheet")));
    }
    if prefer_result_when_active_non_tabular {
        if let Some(active) = &doc.active_sheet_id
            && (doc.result_sheet(active).is_some() || doc.data_sheet(active).is_some())
        {
            return Ok(ExportTarget::Explicit(active.clone()));
        }
        if let Some(rs) = doc.result_sheets.first() {
            return Ok(ExportTarget::Explicit(rs.id.clone()));
        }
        return Ok(ExportTarget::AutoOrExplicit(None));
    }
    if doc.active_sheet_id.is_none()
        && let Some(rs) = doc.result_sheets.first()
    {
        return Ok(ExportTarget::Explicit(rs.id.clone()));
    }
    Ok(ExportTarget::AutoOrExplicit(None))
}

fn check_cancelled(cancelled: Option<&AtomicBool>, context: &str) -> Result<(), EngineError> {
    if cancelled.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(EngineError::with_code(ErrorCode::Cancelled, format!("command `{context}` cancelled"))
            .with_retryable(true)
            .with_details(json!({ "command_id": context })));
    }
    Ok(())
}

/// Compile a specific graph or layout sheet into a [`ChartScene`].
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `id` is not a graph or layout sheet,
/// or propagates chart compilation errors.
pub fn compile_visual_scene_for_sheet(doc: &ProjectDocument, id: &SheetId) -> Result<ChartScene, EngineError> {
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

fn resolve_visual_scene(doc: &ProjectDocument, target: &ExportTarget) -> Result<(SheetId, ChartScene), EngineError> {
    match target {
        ExportTarget::Explicit(id) => Ok((id.clone(), compile_visual_scene_for_sheet(doc, id)?)),
        ExportTarget::AutoOrExplicit(Some(id)) => Ok((id.clone(), compile_visual_scene_for_sheet(doc, id)?)),
        ExportTarget::AutoOrExplicit(None) => {
            if let Some(active) = &doc.active_sheet_id
                && (doc.graph_sheet(active).is_some() || doc.layout_sheet(active).is_some())
            {
                return Ok((active.clone(), compile_visual_scene_for_sheet(doc, active)?));
            }
            if let Some(gs) = doc.graph_sheets.first() {
                return Ok((gs.id.clone(), compile_graph(gs, doc)?));
            }
            if let Some(ls) = doc.layout_sheets.first() {
                return Ok((ls.id.clone(), compile_layout(ls, doc)?));
            }
            Err(EngineError::with_code(ErrorCode::NotFound, "no GraphSheet or LayoutSheet exists in the current document")
                .with_remediation("Create a graph or layout sheet before exporting a figure."))
        }
    }
}

fn check_raster_dim_bounds(w_f: f64, h_f: f64, dpi_val: u16, max_dim: u32) -> Result<(u32, u32), EngineError> {
    if !w_f.is_finite() || !h_f.is_finite() {
        return Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("raster dimensions at {dpi_val} DPI overflow numeric bounds"))
            .with_remediation("Reduce `dpi`, `width_px`, or `height_px`.")
            .with_details(json!({ "dpi": dpi_val, "max_dim_px": max_dim })));
    }
    if w_f < 1.0 || h_f < 1.0 {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            format!("scaled raster dimensions ({w_f}x{h_f}) at {dpi_val} DPI are smaller than 1 pixel"),
        )
        .with_remediation("Increase `dpi` or specify explicit `width_px` and `height_px`.")
        .with_details(json!({ "dpi": dpi_val, "width_px": w_f, "height_px": h_f })));
    }
    let max_dim_f = f64::from(max_dim);
    if w_f > max_dim_f || h_f > max_dim_f {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("raster dimensions {w_f}x{h_f} at {dpi_val} DPI exceed maximum {max_dim}x{max_dim}"),
        )
        .with_remediation("Reduce `width_px`, `height_px`, or `dpi`.")
        .with_details(json!({ "width_px": w_f, "height_px": h_f, "dpi": dpi_val, "max_dim_px": max_dim })));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok((w_f as u32, h_f as u32))
}

/// Resolve strict raster dimensions from scene point size, explicit pixel overrides, or DPI.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::BadInput`] for non-positive dimensions/DPI, or
/// [`ErrorCode::ResourceLimit`] when [`MAX_RASTER_DIM_PX`] or [`MAX_RASTER_PIXELS`] is exceeded.
pub fn resolve_strict_raster_target(
    scene: &ChartScene,
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: Option<u16>,
) -> Result<RenderTargetSpec, EngineError> {
    if !scene.width_pt.is_finite() || scene.width_pt <= 0.0 || !scene.height_pt.is_finite() || scene.height_pt <= 0.0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "scene canvas dimensions must be positive and finite"));
    }
    let dpi_val = dpi.unwrap_or(DEFAULT_EXPORT_DPI);
    if dpi_val == 0 {
        return Err(EngineError::with_code(ErrorCode::BadInput, "`dpi` must be greater than 0").with_details(json!({ "pointer": "/dpi" })));
    }
    let max_dim = MAX_RASTER_DIM_PX;
    let (w, h) = match (width_px, height_px) {
        (Some(w), Some(h)) => {
            if w == 0 || h == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "raster width and height must be greater than 0")
                    .with_details(json!({ "pointer": if w == 0 { "/width_px" } else { "/height_px" } })));
            }
            if w > max_dim || h > max_dim {
                return Err(EngineError::with_code(
                    ErrorCode::ResourceLimit,
                    format!("raster dimensions {w}x{h} exceed maximum {max_dim}x{max_dim}"),
                )
                .with_remediation("Reduce `width_px`, `height_px`, or `dpi`.")
                .with_details(json!({ "width_px": w, "height_px": h, "max_dim_px": max_dim })));
            }
            (w, h)
        }
        (Some(w), None) => {
            if w == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "raster width and height must be greater than 0")
                    .with_details(json!({ "pointer": "/width_px" })));
            }
            let w_f = f64::from(w);
            let h_f = (w_f * (scene.height_pt / scene.width_pt)).round();
            check_raster_dim_bounds(w_f, h_f, dpi_val, max_dim)?
        }
        (None, Some(h)) => {
            if h == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "raster width and height must be greater than 0")
                    .with_details(json!({ "pointer": "/height_px" })));
            }
            let h_f = f64::from(h);
            let w_f = (h_f * (scene.width_pt / scene.height_pt)).round();
            check_raster_dim_bounds(w_f, h_f, dpi_val, max_dim)?
        }
        (None, None) => {
            let scale = f64::from(dpi_val) / 72.0;
            let w_f = (scene.width_pt * scale).round();
            let h_f = (scene.height_pt * scale).round();
            check_raster_dim_bounds(w_f, h_f, dpi_val, max_dim)?
        }
    };
    let total_pixels = u64::from(w).saturating_mul(u64::from(h));
    if total_pixels > MAX_RASTER_PIXELS {
        return Err(EngineError::with_code(
            ErrorCode::ResourceLimit,
            format!("raster pixel count {total_pixels} ({w}x{h}) exceeds budget {MAX_RASTER_PIXELS}"),
        )
        .with_remediation("Reduce `width_px`, `height_px`, or `dpi`.")
        .with_details(json!({ "total_pixels": total_pixels, "max_pixels": MAX_RASTER_PIXELS, "width_px": w, "height_px": h, "dpi": dpi_val })));
    }
    Ok(RenderTargetSpec { width_px: w, height_px: h, dpi: dpi_val })
}

fn export_pdf_artifact(
    doc: &ProjectDocument,
    target: &ExportTarget,
    options: &PdfExportOptions,
    all_graphs: bool,
    sheet_ids: Option<&[SheetId]>,
    cancelled: Option<&AtomicBool>,
    context: &str,
) -> Result<ExportedArtifact, EngineError> {
    if let Some(ids) = sheet_ids {
        if ids.len() > MAX_PDF_PAGES {
            return Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("PDF page count {} exceeds maximum {MAX_PDF_PAGES}", ids.len()))
                .with_remediation("Export fewer pages or split into multiple PDF files.")
                .with_details(json!({ "page_count": ids.len(), "max_pages": MAX_PDF_PAGES })));
        }
        let mut compiled_by_id: BTreeMap<SheetId, ChartScene> = BTreeMap::new();
        for sid in ids {
            check_cancelled(cancelled, context)?;
            if !compiled_by_id.contains_key(sid) {
                compiled_by_id.insert(sid.clone(), compile_visual_scene_for_sheet(doc, sid)?);
            }
        }
        let mut page_refs: Vec<&ChartScene> = Vec::with_capacity(ids.len());
        let mut total_primitives: usize = 0;
        for sid in ids {
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
                    "page_count": ids.len(),
                })));
            }
            page_refs.push(scene);
        }
        let w = page_refs.first().map(|s| s.width_pt);
        let h = page_refs.first().map(|s| s.height_pt);
        let warnings = page_refs.iter().flat_map(|s| s.warnings.iter().cloned()).collect();
        let bytes = render_pdf_page_refs(&page_refs, options)?;
        return Ok(ExportedArtifact {
            target_id: String::from("multi-page"),
            bytes,
            page_count: page_refs.len(),
            width_pt: w,
            height_pt: h,
            warnings,
            raster: None,
        });
    }

    if all_graphs && matches!(target, ExportTarget::AutoOrExplicit(None)) {
        let total_sheets = doc.graph_sheets.len().saturating_add(doc.layout_sheets.len());
        if total_sheets == 0 {
            return Err(EngineError::with_code(ErrorCode::NotFound, "no GraphSheet or LayoutSheet available to export")
                .with_remediation("Create at least one graph or layout sheet before exporting PDF."));
        }
        if total_sheets > MAX_PDF_PAGES {
            return Err(EngineError::with_code(ErrorCode::ResourceLimit, format!("PDF page count {total_sheets} exceeds maximum {MAX_PDF_PAGES}"))
                .with_remediation("Export fewer pages or split into multiple PDF files.")
                .with_details(json!({ "page_count": total_sheets, "max_pages": MAX_PDF_PAGES })));
        }
        let mut scenes = Vec::with_capacity(total_sheets);
        let mut total_primitives: usize = 0;
        let all_ids = doc.graph_sheets.iter().map(|gs| &gs.id).chain(doc.layout_sheets.iter().map(|ls| &ls.id));
        for sid in all_ids {
            check_cancelled(cancelled, context)?;
            let scene = compile_visual_scene_for_sheet(doc, sid)?;
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
                    "page_count": total_sheets,
                })));
            }
            scenes.push(scene);
        }
        let mut opts = options.clone();
        if opts.title.is_none() {
            opts.title = Some(doc.title.clone());
        }
        let w = scenes.first().map(|s| s.width_pt);
        let h = scenes.first().map(|s| s.height_pt);
        let warnings = scenes.iter().flat_map(|s| s.warnings.iter().cloned()).collect();
        let bytes = render_pdf_pages(&scenes, &opts)?;
        return Ok(ExportedArtifact {
            target_id: String::from("all"),
            bytes,
            page_count: scenes.len(),
            width_pt: w,
            height_pt: h,
            warnings,
            raster: None,
        });
    }

    let (id, scene) = resolve_visual_scene(doc, target)?;
    check_cancelled(cancelled, context)?;
    let bytes = render_pdf(&scene, options)?;
    Ok(ExportedArtifact {
        target_id: id.as_str().to_owned(),
        bytes,
        page_count: 1,
        width_pt: Some(scene.width_pt),
        height_pt: Some(scene.height_pt),
        warnings: scene.warnings.clone(),
        raster: None,
    })
}

enum TabularTarget<'a> {
    Data(&'a DataSheet),
    Result(&'a ResultSheet),
}

fn resolve_tabular_target<'a>(
    doc: &'a ProjectDocument,
    target: &ExportTarget,
    requires_result: bool,
    tab_desc: Option<&str>,
) -> Result<TabularTarget<'a>, EngineError> {
    let explicit_id = match target {
        ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id)) => Some(id),
        ExportTarget::AutoOrExplicit(None) => None,
    };
    if let Some(id) = explicit_id {
        if let Some(rs) = doc.result_sheet(id) {
            return Ok(TabularTarget::Result(rs));
        }
        if let Some(ds) = doc.data_sheet(id) {
            if requires_result {
                let label = tab_desc.unwrap_or("tab");
                return Err(EngineError::with_code(
                    ErrorCode::BadInput,
                    format!("{label} requires a ResultSheet, but `{}` is a DataSheet", id.as_str()),
                ));
            }
            return Ok(TabularTarget::Data(ds));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("data or result sheet `{}` not found", id.as_str()))
            .with_details(json!({ "sheet_id": id.as_str() })));
    }

    if requires_result {
        if let Some(active) = &doc.active_sheet_id
            && let Some(rs) = doc.result_sheet(active)
        {
            return Ok(TabularTarget::Result(rs));
        }
        if let Some(rs) = doc.result_sheets.first() {
            return Ok(TabularTarget::Result(rs));
        }
        return Err(EngineError::with_code(ErrorCode::NotFound, "no ResultSheet available for tab CSV export"));
    }

    if let Some(active) = &doc.active_sheet_id {
        if let Some(rs) = doc.result_sheet(active) {
            return Ok(TabularTarget::Result(rs));
        }
        if let Some(ds) = doc.data_sheet(active) {
            return Ok(TabularTarget::Data(ds));
        }
    }
    if let Some(ds) = doc.data_sheets.first() {
        return Ok(TabularTarget::Data(ds));
    }
    if let Some(rs) = doc.result_sheets.first() {
        return Ok(TabularTarget::Result(rs));
    }
    Err(EngineError::with_code(ErrorCode::NotFound, "no tabular data or result sheet available to export"))
}

fn export_csv_artifact(
    doc: &ProjectDocument,
    target: &ExportTarget,
    options: &CsvOptions,
    tab_id: Option<&str>,
    tab_index: Option<usize>,
    tidy: bool,
) -> Result<ExportedArtifact, EngineError> {
    if tidy && (tab_id.is_some() || tab_index.is_some()) {
        return Err(EngineError::with_code(
            ErrorCode::BadInput,
            "export `tidy: true` cannot be combined with `tab_id` or `tab_index`; omit `tidy` or pass `tidy: false` to export a specific tab",
        ));
    }
    let tab_desc = tab_id.map(|id| format!("tab `{id}`")).or_else(|| tab_index.map(|idx| format!("tab index {idx}")));
    let (target_id, csv_text) = match resolve_tabular_target(doc, target, tab_desc.is_some(), tab_desc.as_deref())? {
        TabularTarget::Data(ds) => (ds.id.as_str().to_owned(), export_datasheet_to_csv(ds, options)?),
        TabularTarget::Result(rs) => {
            let text = if tidy {
                try_export_result_sheet_tidy_csv(rs, options)?
            } else if let Some(tid) = tab_id {
                let tab = rs.tabs.iter().find(|t| t.id == tid).ok_or_else(|| {
                    EngineError::with_code(ErrorCode::NotFound, format!("tab `{tid}` not found on result sheet `{}`", rs.id.as_str()))
                })?;
                try_export_result_tab_csv(rs, tab, options)?
            } else if let Some(idx) = tab_index {
                let tab = rs.tabs.get(idx).ok_or_else(|| {
                    EngineError::with_code(ErrorCode::NotFound, format!("result sheet `{}` has no tab at index {idx}", rs.id.as_str()))
                })?;
                try_export_result_tab_csv(rs, tab, options)?
            } else if let Some(first_tab) = rs.tabs.first() {
                try_export_result_tab_csv(rs, first_tab, options)?
            } else {
                try_export_result_sheet_tidy_csv(rs, options)?
            };
            (rs.id.as_str().to_owned(), text)
        }
    };
    Ok(ExportedArtifact {
        target_id,
        bytes: csv_text.into_bytes(),
        page_count: 1,
        width_pt: None,
        height_pt: None,
        warnings: Vec::new(),
        raster: None,
    })
}

fn serialize_json_bytes(value: &impl Serialize, pretty: bool) -> Result<Vec<u8>, EngineError> {
    let text = if pretty { serde_json::to_string_pretty(value) } else { serde_json::to_string(value) }
        .map_err(|err| EngineError::with_code(ErrorCode::InternalError, format!("JSON serialization failed: {err}")))?;
    Ok(text.into_bytes())
}

fn export_json_artifact(doc: &ProjectDocument, target: &ExportTarget, pretty: bool) -> Result<ExportedArtifact, EngineError> {
    let explicit_id = match target {
        ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id)) => Some(id),
        ExportTarget::AutoOrExplicit(None) => None,
    };
    let (target_id, bytes) = if let Some(id) = explicit_id {
        if let Some(rs) = doc.result_sheet(id) {
            (id.as_str().to_owned(), export_result_sheet_json(rs, pretty)?.into_bytes())
        } else if let Some(ds) = doc.data_sheet(id) {
            (id.as_str().to_owned(), serialize_json_bytes(ds, pretty)?)
        } else if let Some(info) = doc.info_sheet(id) {
            (id.as_str().to_owned(), serialize_json_bytes(info, pretty)?)
        } else if let Some(gs) = doc.graph_sheet(id) {
            (id.as_str().to_owned(), serialize_json_bytes(gs, pretty)?)
        } else if let Some(ls) = doc.layout_sheet(id) {
            (id.as_str().to_owned(), serialize_json_bytes(ls, pretty)?)
        } else {
            return Err(EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", id.as_str()))
                .with_details(json!({ "sheet_id": id.as_str() })));
        }
    } else {
        (String::from("project"), serialize_json_bytes(doc, pretty)?)
    };
    Ok(ExportedArtifact { target_id, bytes, page_count: 1, width_pt: None, height_pt: None, warnings: Vec::new(), raster: None })
}

struct RasterExportParams {
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: Option<u16>,
    sizing: RasterSizingPolicy,
    tiff_compression: Option<TiffCompression>,
    transparent_bg: bool,
}

fn export_raster_artifact(
    doc: &ProjectDocument,
    target: &ExportTarget,
    params: RasterExportParams,
    cancelled: Option<&AtomicBool>,
    context: &str,
) -> Result<ExportedArtifact, EngineError> {
    let (id, scene) = resolve_visual_scene(doc, target)?;
    check_cancelled(cancelled, context)?;
    let spec = match params.sizing {
        RasterSizingPolicy::StrictLimits => resolve_strict_raster_target(&scene, params.width_px, params.height_px, params.dpi)?,
        RasterSizingPolicy::CapToLimits { default_dpi } => {
            if params.width_px == Some(0) || params.height_px == Some(0) {
                return Err(EngineError::with_code(ErrorCode::BadInput, "raster width and height must be greater than 0")
                    .with_details(json!({ "pointer": if params.width_px == Some(0) { "/width_px" } else { "/height_px" } })));
            }
            let eff_dpi = params.dpi.unwrap_or(default_dpi);
            if eff_dpi == 0 {
                return Err(EngineError::with_code(ErrorCode::BadInput, "`dpi` must be greater than 0").with_details(json!({ "pointer": "/dpi" })));
            }
            RenderTargetSpec::from_scene_capped(&scene, params.width_px, params.height_px, eff_dpi)
        }
    };
    let raster = render_scene_with_options(&scene, spec, RenderOptions { transparent_bg: params.transparent_bg })?;
    let bytes = match params.tiff_compression {
        Some(compression) => encode_tiff(&raster, compression)?,
        None => encode_png(&raster)?,
    };
    Ok(ExportedArtifact {
        target_id: id.as_str().to_owned(),
        bytes,
        page_count: 1,
        width_pt: Some(scene.width_pt),
        height_pt: Some(scene.height_pt),
        warnings: scene.warnings.clone(),
        raster: Some(RasterMetadata { width_px: raster.width_px, height_px: raster.height_px, dpi: spec.dpi }),
    })
}

fn export_emf_artifact(
    doc: &ProjectDocument,
    target: &ExportTarget,
    require_kind: Option<EmfSheetKind>,
    cancelled: Option<&AtomicBool>,
    context: &str,
) -> Result<ExportedArtifact, EngineError> {
    let (id, scene) = match require_kind {
        Some(EmfSheetKind::Graph) => {
            let (ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id))) = target else {
                return Err(EngineError::with_code(ErrorCode::BadInput, "EMF graph export requires `sheet_id`"));
            };
            let graph = doc.graph_sheet(id).ok_or_else(|| {
                EngineError::with_code(ErrorCode::NotFound, format!("graph sheet `{}` not found", id.as_str()))
                    .with_remediation("Inspect available graph sheet IDs before retrying.")
                    .with_details(json!({ "sheet_id": id.as_str() }))
            })?;
            (id.clone(), compile_graph(graph, doc)?)
        }
        Some(EmfSheetKind::Layout) => {
            let (ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id))) = target else {
                return Err(EngineError::with_code(ErrorCode::BadInput, "EMF layout export requires `sheet_id`"));
            };
            let layout = doc.layout_sheet(id).ok_or_else(|| {
                EngineError::with_code(ErrorCode::NotFound, format!("layout sheet `{}` not found", id.as_str()))
                    .with_remediation("Inspect available layout sheet IDs before retrying.")
                    .with_details(json!({ "sheet_id": id.as_str() }))
            })?;
            (id.clone(), compile_layout(layout, doc)?)
        }
        None => resolve_visual_scene(doc, target)?,
    };
    check_cancelled(cancelled, context)?;
    let emf = encode_emf(&scene)?;
    Ok(ExportedArtifact {
        target_id: id.as_str().to_owned(),
        bytes: emf,
        page_count: 1,
        width_pt: Some(scene.width_pt),
        height_pt: Some(scene.height_pt),
        warnings: scene.warnings.clone(),
        raster: None,
    })
}

/// Export a figure or tabular artifact from `doc` in SVG, PDF, PNG, TIFF, EMF, CSV, or JSON format.
///
/// Enforces PDF page and primitive budgets, strict or capped raster limits, cooperative cancellation,
/// and the failed-result CSV export guard uniformly across engine commands, MCP, control I/O, and CLI.
///
/// # Errors
/// Returns [`EngineError`] with structured [`ErrorCode`] for missing targets, invalid options,
/// failed result sheets, resource-limit violations, cancellation, or rendering/encoding failures.
pub fn export_document_artifact(
    doc: &ProjectDocument,
    request: &ExportArtifactRequest,
    cancelled: Option<&AtomicBool>,
    context: &str,
) -> Result<ExportedArtifact, EngineError> {
    check_cancelled(cancelled, context)?;
    match &request.format {
        ExportArtifactFormat::Svg { options } => {
            let (id, scene) = resolve_visual_scene(doc, &request.target)?;
            check_cancelled(cancelled, context)?;
            let svg = render_svg(&scene, options)?;
            Ok(ExportedArtifact {
                target_id: id.as_str().to_owned(),
                bytes: svg.into_bytes(),
                page_count: 1,
                width_pt: Some(scene.width_pt),
                height_pt: Some(scene.height_pt),
                warnings: scene.warnings.clone(),
                raster: None,
            })
        }
        ExportArtifactFormat::Pdf { options, all_graphs, sheet_ids } => {
            export_pdf_artifact(doc, &request.target, options, *all_graphs, sheet_ids.as_deref(), cancelled, context)
        }
        ExportArtifactFormat::Png { width_px, height_px, dpi, sizing, transparent_bg } => export_raster_artifact(
            doc,
            &request.target,
            RasterExportParams {
                width_px: *width_px,
                height_px: *height_px,
                dpi: *dpi,
                sizing: *sizing,
                tiff_compression: None,
                transparent_bg: *transparent_bg,
            },
            cancelled,
            context,
        ),
        ExportArtifactFormat::Tiff { width_px, height_px, dpi, compression, sizing, transparent_bg } => export_raster_artifact(
            doc,
            &request.target,
            RasterExportParams {
                width_px: *width_px,
                height_px: *height_px,
                dpi: *dpi,
                sizing: *sizing,
                tiff_compression: Some(*compression),
                transparent_bg: *transparent_bg,
            },
            cancelled,
            context,
        ),
        ExportArtifactFormat::Emf { require_kind } => export_emf_artifact(doc, &request.target, *require_kind, cancelled, context),
        ExportArtifactFormat::Csv { options, tab_id, tab_index, tidy } => {
            export_csv_artifact(doc, &request.target, options, tab_id.as_deref(), *tab_index, *tidy)
        }
        ExportArtifactFormat::Json { pretty } => export_json_artifact(doc, &request.target, *pretty),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::ErrorCode;
    use curvebook_pdf::{PdfColorSpace, PdfVersion};
    use curvebook_render::TiffCompression;
    use curvebook_text::build_minimal_truetype_font;
    use serde_json::json;

    use super::{
        DEFAULT_PREVIEW_DPI, DocumentExportFormat, ExportOptionsSpec, MAX_RASTER_DPI, MIN_RASTER_DPI, parse_dpi, parse_dpi_str,
        parse_pdf_color_space, parse_pdf_version, parse_tiff_compression, pdf_version_str, validate_ttf_font_bytes,
    };
    use crate::{ExternalPathPolicy, services::load_validated_ttf_font};

    #[test]
    fn pdf_dpi_color_space_and_compression_parsers_are_consistent() {
        assert_eq!(parse_pdf_version("1.4").unwrap(), PdfVersion::Pdf14);
        assert_eq!(parse_pdf_version(" 1.7 ").unwrap(), PdfVersion::Pdf17);
        assert_eq!(pdf_version_str(PdfVersion::Pdf14), "1.4");
        assert_eq!(pdf_version_str(PdfVersion::Pdf17), "1.7");
        assert_eq!(parse_pdf_version("1.5").unwrap_err().code, ErrorCode::BadInput);

        assert_eq!(parse_pdf_color_space("RGB").unwrap(), PdfColorSpace::Srgb);
        assert_eq!(parse_pdf_color_space("cmyk").unwrap(), PdfColorSpace::Cmyk);
        assert_eq!(parse_pdf_color_space("gray").unwrap(), PdfColorSpace::Grayscale);
        assert_eq!(parse_pdf_color_space("lab").unwrap_err().code, ErrorCode::BadInput);

        assert_eq!(parse_tiff_compression("none").unwrap(), TiffCompression::None);
        assert_eq!(parse_tiff_compression("DEFLATE").unwrap(), TiffCompression::Deflate);
        assert_eq!(parse_tiff_compression("lzw").unwrap(), TiffCompression::Lzw);
        assert_eq!(parse_tiff_compression("zip").unwrap_err().code, ErrorCode::BadInput);

        assert_eq!(parse_dpi(None, DEFAULT_PREVIEW_DPI).unwrap(), DEFAULT_PREVIEW_DPI);
        assert_eq!(parse_dpi(Some(&json!(null)), DEFAULT_PREVIEW_DPI).unwrap(), DEFAULT_PREVIEW_DPI);
        assert_eq!(parse_dpi(Some(&json!(MIN_RASTER_DPI)), DEFAULT_PREVIEW_DPI).unwrap(), MIN_RASTER_DPI);
        assert_eq!(parse_dpi_str("2400").unwrap(), MAX_RASTER_DPI);
        assert_eq!(parse_dpi(Some(&json!(17)), DEFAULT_PREVIEW_DPI).unwrap_err().code, ErrorCode::BadInput);
        assert_eq!(parse_dpi(Some(&json!(2401)), DEFAULT_PREVIEW_DPI).unwrap_err().code, ErrorCode::BadInput);
        assert_eq!(parse_dpi(Some(&json!("300")), DEFAULT_PREVIEW_DPI).unwrap_err().code, ErrorCode::BadInput);
        assert_eq!(parse_dpi_str("17").unwrap_err().code, ErrorCode::BadInput);

        assert_eq!(DocumentExportFormat::parse("PDF").unwrap(), DocumentExportFormat::Pdf);
        assert_eq!(DocumentExportFormat::Pdf.as_str(), "pdf");
        assert!(ExportOptionsSpec::new(DocumentExportFormat::Svg).tidy);
    }

    #[test]
    fn ttf_font_byte_validation_and_reject_external_policy_work() {
        let valid = build_minimal_truetype_font();
        assert!(validate_ttf_font_bytes(&valid, "minimal.ttf").is_ok());
        assert_eq!(validate_ttf_font_bytes(b"OTTO_fake_cff", "cff.otf").unwrap_err().code, ErrorCode::BadInput);
        assert_eq!(validate_ttf_font_bytes(b"bad", "bad.ttf").unwrap_err().code, ErrorCode::BadInput);

        let denied = load_validated_ttf_font(&ExternalPathPolicy::RejectExternal, "font.ttf").unwrap_err();
        assert_eq!(denied.code, ErrorCode::PathEscape);
    }
}
