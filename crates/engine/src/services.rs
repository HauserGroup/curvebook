//! Shared application services for figure/tabular export, project inspection, and atomic batches.
//!
//! Centralizes domain logic shared by engine commands (`cmd::{export,project}`) and higher-level
//! frontends (MCP server, live control server, headless CLI) without depending on any UI or
//! transport crate.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use curvebook_chart::{ChartScene, compile_graph, compile_layout};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, DataSheet, ErrorCode, InfoConstant, ParameterScope, ProjectDocument, ResultSheet, SheetId, SheetSection,
};
use curvebook_format::{CsvOptions, export_datasheet_to_csv, export_result_sheet_json, try_export_result_sheet_tidy_csv, try_export_result_tab_csv};
use curvebook_pdf::{MAX_PDF_PAGES, MAX_PDF_TOTAL_PRIMITIVES, PdfExportOptions, render_pdf, render_pdf_page_refs, render_pdf_pages};
use curvebook_render::{MAX_RASTER_DIM_PX, MAX_RASTER_PIXELS, RenderTargetSpec, TiffCompression, encode_emf, encode_png, encode_tiff, render_scene};
use curvebook_svg::{SvgExportOptions, render_svg};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::cmd::{Effects, registry};
use crate::{Caller, CommandOutcome, EngineError, ExternalPathPolicy, Session};

// ---------------------------------------------------------------------------
// Document Artifact Export Service
// ---------------------------------------------------------------------------

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
    /// Count of non-fatal scene compilation warnings.
    pub warnings_count: usize,
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
    let dpi_val = dpi.unwrap_or(300);
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
        let warnings_count = page_refs.iter().map(|s| s.warnings.len()).sum();
        let bytes = render_pdf_page_refs(&page_refs, options)?;
        return Ok(ExportedArtifact {
            target_id: String::from("multi-page"),
            bytes,
            page_count: page_refs.len(),
            width_pt: w,
            height_pt: h,
            warnings_count,
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
        let warnings_count = scenes.iter().map(|s| s.warnings.len()).sum();
        let bytes = render_pdf_pages(&scenes, &opts)?;
        return Ok(ExportedArtifact {
            target_id: String::from("all"),
            bytes,
            page_count: scenes.len(),
            width_pt: w,
            height_pt: h,
            warnings_count,
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
        warnings_count: scene.warnings.len(),
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
    Ok(ExportedArtifact { target_id, bytes: csv_text.into_bytes(), page_count: 1, width_pt: None, height_pt: None, warnings_count: 0, raster: None })
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
    Ok(ExportedArtifact { target_id, bytes, page_count: 1, width_pt: None, height_pt: None, warnings_count: 0, raster: None })
}

struct RasterExportParams {
    width_px: Option<u32>,
    height_px: Option<u32>,
    dpi: Option<u16>,
    sizing: RasterSizingPolicy,
    tiff_compression: Option<TiffCompression>,
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
            RenderTargetSpec::from_scene_capped(&scene, params.width_px, params.height_px, params.dpi.unwrap_or(default_dpi))
        }
    };
    let raster = render_scene(&scene, spec)?;
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
        warnings_count: scene.warnings.len(),
        raster: Some(RasterMetadata { width_px: raster.width_px, height_px: raster.height_px, dpi: spec.dpi }),
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
                warnings_count: scene.warnings.len(),
                raster: None,
            })
        }
        ExportArtifactFormat::Pdf { options, all_graphs, sheet_ids } => {
            export_pdf_artifact(doc, &request.target, options, *all_graphs, sheet_ids.as_deref(), cancelled, context)
        }
        ExportArtifactFormat::Png { width_px, height_px, dpi, sizing } => export_raster_artifact(
            doc,
            &request.target,
            RasterExportParams { width_px: *width_px, height_px: *height_px, dpi: *dpi, sizing: *sizing, tiff_compression: None },
            cancelled,
            context,
        ),
        ExportArtifactFormat::Tiff { width_px, height_px, dpi, compression, sizing } => export_raster_artifact(
            doc,
            &request.target,
            RasterExportParams { width_px: *width_px, height_px: *height_px, dpi: *dpi, sizing: *sizing, tiff_compression: Some(*compression) },
            cancelled,
            context,
        ),
        ExportArtifactFormat::Emf { require_kind } => {
            let (id, scene) = match require_kind {
                Some(EmfSheetKind::Graph) => {
                    let (ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id))) = &request.target else {
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
                    let (ExportTarget::Explicit(id) | ExportTarget::AutoOrExplicit(Some(id))) = &request.target else {
                        return Err(EngineError::with_code(ErrorCode::BadInput, "EMF layout export requires `sheet_id`"));
                    };
                    let layout = doc.layout_sheet(id).ok_or_else(|| {
                        EngineError::with_code(ErrorCode::NotFound, format!("layout sheet `{}` not found", id.as_str()))
                            .with_remediation("Inspect available layout sheet IDs before retrying.")
                            .with_details(json!({ "sheet_id": id.as_str() }))
                    })?;
                    (id.clone(), compile_layout(layout, doc)?)
                }
                None => resolve_visual_scene(doc, &request.target)?,
            };
            check_cancelled(cancelled, context)?;
            let emf = encode_emf(&scene)?;
            Ok(ExportedArtifact {
                target_id: id.as_str().to_owned(),
                bytes: emf,
                page_count: 1,
                width_pt: Some(scene.width_pt),
                height_pt: Some(scene.height_pt),
                warnings_count: scene.warnings.len(),
                raster: None,
            })
        }
        ExportArtifactFormat::Csv { options, tab_id, tab_index, tidy } => {
            export_csv_artifact(doc, &request.target, options, tab_id.as_deref(), *tab_index, *tidy)
        }
        ExportArtifactFormat::Json { pretty } => export_json_artifact(doc, &request.target, *pretty),
    }
}

// ---------------------------------------------------------------------------
// Project & Sheet Inspection Service
// ---------------------------------------------------------------------------

/// Focused section and upstream dependency metadata for a single requested sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SheetFocusSummary {
    /// Focused sheet identifier.
    pub sheet_id: String,
    /// Debug name of the [`SheetSection`].
    pub section: String,
    /// Upstream sheet identifiers in dependency order.
    pub upstream_dependencies: Vec<String>,
}

/// Summary metadata for a data sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Debug representation of the table archetype kind (e.g. `"Xy"`).
    pub kind_debug: String,
    /// Display label of the table archetype kind (e.g. `"XY"`).
    pub kind_label: String,
    /// Whether edits to this sheet are frozen.
    pub frozen: bool,
    /// Number of rows.
    pub rows: usize,
    /// Number of columns.
    pub cols: usize,
    /// Total allocated cell slots.
    pub allocated_cells: usize,
    /// Optional linked info sheet identifier.
    pub info_sheet_id: Option<String>,
    /// Number of linked import regions.
    pub linked_regions: usize,
    /// Upstream sheet identifiers.
    pub upstream_dependencies: Vec<String>,
}

/// Summary metadata for an info sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InfoSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Byte length of the markdown notes body.
    pub notes_len: usize,
    /// Linked data sheet identifiers.
    pub linked_data_sheets: Vec<String>,
    /// Named constants stored on the info sheet.
    pub constants: Vec<InfoConstant>,
}

/// Summary metadata for a result sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResultSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Upstream source sheet identifier.
    pub source_sheet_id: String,
    /// Effective analysis title.
    pub title: String,
    /// Debug representation of [`curvebook_doc::ResultStatus`].
    pub status: String,
    /// Number of configured hook references.
    pub hook_count: usize,
    /// Number of output tables/tabs.
    pub tabs: usize,
    /// Number of emitted scalar hooks.
    pub scalars: usize,
    /// Number of diagnostic checklist entries.
    pub checklist: usize,
    /// Whether this result sheet exposes downstream-analyzable data.
    pub analyzable: bool,
}

/// Summary metadata for a graph sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Whether the graph has been initialized with a plot type.
    pub initialized: bool,
    /// Debug representation of [`curvebook_doc::GraphKind`].
    pub kind_debug: String,
    /// Primary source sheet identifier.
    pub primary_source_sheet_id: String,
    /// Number of plotted series.
    pub series_count: usize,
    /// Canvas width in typographic points.
    pub width_pt: f64,
    /// Canvas height in typographic points.
    pub height_pt: f64,
}

/// Summary metadata for a layout sheet in [`ProjectInspectSummary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayoutSheetSummary {
    /// Sheet identifier.
    pub id: String,
    /// Human-readable sheet title.
    pub title: String,
    /// Number of placed layout items.
    pub item_count: usize,
    /// Page width in typographic points.
    pub width_pt: f64,
    /// Page height in typographic points.
    pub height_pt: f64,
}

/// Complete project overview produced by [`inspect_project_summary`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProjectInspectSummary {
    /// Document archive schema version.
    pub schema_version: u32,
    /// Project title.
    pub title: String,
    /// Whether the owning session has unsaved modifications.
    pub dirty: bool,
    /// Associated archive path, if any.
    pub file_path: Option<String>,
    /// Total count of sheets across all folders.
    pub total_sheet_count: usize,
    /// Active sheet identifier, if any.
    pub active_sheet_id: Option<String>,
    /// Active family anchor sheet identifier, if any.
    pub family_anchor_sheet_id: Option<String>,
    /// Number of saved named checkpoints.
    pub checkpoints: usize,
    /// Data sheet summaries.
    pub data_sheets: Vec<DataSheetSummary>,
    /// Info sheet summaries.
    pub info_sheets: Vec<InfoSheetSummary>,
    /// Result sheet summaries.
    pub result_sheets: Vec<ResultSheetSummary>,
    /// Graph sheet summaries.
    pub graph_sheets: Vec<GraphSheetSummary>,
    /// Layout sheet summaries.
    pub layout_sheets: Vec<LayoutSheetSummary>,
    /// Optional focused sheet section and upstream dependency breakdown.
    pub focus: Option<SheetFocusSummary>,
}

impl ProjectInspectSummary {
    /// Project this summary into the canonical `project.inspect` command JSON payload shape.
    #[must_use]
    pub fn to_project_inspect_json(&self) -> Value {
        let data_sheets: Vec<Value> = self
            .data_sheets
            .iter()
            .map(|ds| {
                json!({
                    "id": ds.id,
                    "title": ds.title,
                    "kind": ds.kind_debug,
                    "frozen": ds.frozen,
                    "rows": ds.rows,
                    "cols": ds.cols,
                    "allocated_cells": ds.allocated_cells,
                    "info_sheet_id": ds.info_sheet_id,
                    "linked_regions": ds.linked_regions,
                })
            })
            .collect();
        let info_sheets: Vec<Value> = self
            .info_sheets
            .iter()
            .map(|info| {
                json!({
                    "id": info.id,
                    "title": info.title,
                    "notes_len": info.notes_len,
                    "constants": info.constants.iter().map(|c| json!({ "name": c.name, "value": &c.value, "units": c.units })).collect::<Vec<_>>(),
                })
            })
            .collect();
        let result_sheets: Vec<Value> = self
            .result_sheets
            .iter()
            .map(|rs| {
                json!({
                    "id": rs.id,
                    "source_sheet_id": rs.source_sheet_id,
                    "title": rs.title,
                    "status": rs.status,
                    "hook_count": rs.hook_count,
                    "tabs": rs.tabs,
                    "scalars": rs.scalars,
                    "analyzable": rs.analyzable,
                })
            })
            .collect();
        let graph_sheets: Vec<Value> = self
            .graph_sheets
            .iter()
            .map(|gs| {
                json!({
                    "id": gs.id,
                    "title": gs.title,
                    "initialized": gs.initialized,
                    "kind": gs.kind_debug,
                    "primary_source_sheet_id": gs.primary_source_sheet_id,
                    "series_count": gs.series_count,
                })
            })
            .collect();
        let layout_sheets: Vec<Value> =
            self.layout_sheets.iter().map(|ls| json!({ "id": ls.id, "title": ls.title, "item_count": ls.item_count })).collect();
        let focus = self.focus.as_ref().map(|f| {
            json!({
                "sheet_id": f.sheet_id,
                "section": f.section,
                "upstream_dependencies": f.upstream_dependencies,
            })
        });
        json!({
            "schema_version": self.schema_version,
            "title": self.title,
            "total_sheet_count": self.total_sheet_count,
            "active_sheet_id": self.active_sheet_id,
            "family_anchor_sheet_id": self.family_anchor_sheet_id,
            "checkpoints": self.checkpoints,
            "data_sheets": data_sheets,
            "info_sheets": info_sheets,
            "result_sheets": result_sheets,
            "graph_sheets": graph_sheets,
            "layout_sheets": layout_sheets,
            "focus": focus,
        })
    }
}

fn count_analysis_hooks(spec: &AnalysisSpec) -> usize {
    match &spec.kind {
        AnalysisKind::NonlinearRegression(nl) => nl.parameter_rules.iter().filter(|r| matches!(r.scope, ParameterScope::HookedConstant(_))).count(),
        AnalysisKind::PowerAnalysis(p) => usize::from(p.info_sheet_id.is_some()),
        _ => 0,
    }
}

/// Inspect project-wide metadata, sheet summaries, and optional focused sheet dependencies.
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `focus_sheet_id` does not exist in `doc`.
pub fn inspect_project_summary(
    doc: &ProjectDocument,
    dirty: bool,
    file_path: Option<&Path>,
    focus_sheet_id: Option<&SheetId>,
) -> Result<ProjectInspectSummary, EngineError> {
    let focus = if let Some(focus_id) = focus_sheet_id {
        let section = doc
            .sheet_section(focus_id)
            .ok_or_else(|| EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", focus_id.as_str())))?;
        let upstream = doc.upstream_dependencies(focus_id).into_iter().map(|id| id.as_str().to_owned()).collect();
        Some(SheetFocusSummary { sheet_id: focus_id.as_str().to_owned(), section: format!("{section:?}"), upstream_dependencies: upstream })
    } else {
        None
    };

    let data_sheets = doc
        .data_sheets
        .iter()
        .map(|ds| DataSheetSummary {
            id: ds.id.as_str().to_owned(),
            title: ds.title.clone(),
            kind_debug: format!("{:?}", ds.kind()),
            kind_label: ds.kind().label().to_owned(),
            frozen: ds.frozen,
            rows: ds.archetype.row_count(),
            cols: ds.archetype.column_count(),
            allocated_cells: ds.archetype.allocated_cell_slots(),
            info_sheet_id: ds.info_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
            linked_regions: ds.linked_regions.len(),
            upstream_dependencies: doc.upstream_dependencies(&ds.id).into_iter().map(|id| id.as_str().to_owned()).collect(),
        })
        .collect();

    let info_sheets = doc
        .info_sheets
        .iter()
        .map(|info| InfoSheetSummary {
            id: info.id.as_str().to_owned(),
            title: info.title.clone(),
            notes_len: info.notes_markdown.len(),
            linked_data_sheets: info.linked_data_sheets.iter().map(|id| id.as_str().to_owned()).collect(),
            constants: info.constants.clone(),
        })
        .collect();

    let result_sheets = doc
        .result_sheets
        .iter()
        .map(|rs| ResultSheetSummary {
            id: rs.id.as_str().to_owned(),
            source_sheet_id: rs.analysis.source_sheet_id.as_str().to_owned(),
            title: rs.analysis.effective_title().to_owned(),
            status: format!("{:?}", rs.status),
            hook_count: count_analysis_hooks(&rs.analysis),
            tabs: rs.tabs.len(),
            scalars: rs.scalars.len(),
            checklist: rs.checklist.len(),
            analyzable: rs.analyzable_data.is_some(),
        })
        .collect();

    let graph_sheets = doc
        .graph_sheets
        .iter()
        .map(|gs| GraphSheetSummary {
            id: gs.id.as_str().to_owned(),
            title: gs.title.clone(),
            initialized: gs.initialized,
            kind_debug: format!("{:?}", gs.spec.kind),
            primary_source_sheet_id: gs.spec.primary_source_sheet_id.as_str().to_owned(),
            series_count: gs.spec.series.len(),
            width_pt: gs.spec.canvas_size_pt.width,
            height_pt: gs.spec.canvas_size_pt.height,
        })
        .collect();

    let layout_sheets = doc
        .layout_sheets
        .iter()
        .map(|ls| {
            let size = ls.spec.page_size_pt();
            LayoutSheetSummary {
                id: ls.id.as_str().to_owned(),
                title: ls.title.clone(),
                item_count: ls.spec.items.len(),
                width_pt: size.width,
                height_pt: size.height,
            }
        })
        .collect();

    Ok(ProjectInspectSummary {
        schema_version: doc.schema_version,
        title: doc.title.clone(),
        dirty,
        file_path: file_path.map(|p| p.display().to_string()),
        total_sheet_count: doc.total_sheet_count(),
        active_sheet_id: doc.active_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
        family_anchor_sheet_id: doc.family_anchor_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
        checkpoints: doc.checkpoints.len(),
        data_sheets,
        info_sheets,
        result_sheets,
        graph_sheets,
        layout_sheets,
        focus,
    })
}

/// Inspect a single sheet in detail (including optional CSV preview, tabs, checklist, or specs).
///
/// # Errors
/// Returns [`EngineError`] with [`ErrorCode::NotFound`] if `sheet_id` does not exist in `doc`.
pub fn inspect_sheet_detail(doc: &ProjectDocument, sheet_id: &SheetId, include_cells: bool) -> Result<Value, EngineError> {
    let Some(section) = doc.sheet_section(sheet_id) else {
        return Err(EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", sheet_id.as_str()))
            .with_details(json!({ "sheet_id": sheet_id.as_str() })));
    };
    let deps: Vec<String> = doc.upstream_dependencies(sheet_id).into_iter().map(|id| id.as_str().to_owned()).collect();
    let detail = match section {
        SheetSection::Data => match doc.data_sheet(sheet_id) {
            Some(ds) => {
                let mut val = json!({
                    "section": "Data",
                    "id": ds.id.as_str(),
                    "title": ds.title,
                    "kind": ds.kind().label(),
                    "rows": ds.archetype.row_count(),
                    "cols": ds.archetype.column_count(),
                    "columns": ds.archetype.column_count(),
                    "frozen": ds.frozen,
                    "info_sheet_id": ds.info_sheet_id.as_ref().map(SheetId::as_str),
                    "linked_regions": ds.linked_regions.len(),
                    "upstream_dependencies": deps,
                });
                if include_cells && let Some(obj) = val.as_object_mut() {
                    let (csv_preview, csv_preview_truncated) = inspect_preview_bounds::build_csv_preview(ds)?;
                    obj.insert(String::from("csv_preview"), Value::String(csv_preview));
                    obj.insert(String::from("csv_preview_truncated"), Value::Bool(csv_preview_truncated));
                }
                Some(val)
            }
            None => None,
        },
        SheetSection::Info => doc.info_sheet(sheet_id).map(|info| {
            let mut val = json!({
                "section": "Info",
                "id": info.id.as_str(),
                "title": info.title,
                "constants": info.constants.len(),
                "linked_data_sheets": info.linked_data_sheets.iter().map(SheetId::as_str).collect::<Vec<_>>(),
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("constants_entries"), serde_json::to_value(&info.constants).unwrap_or(Value::Null));
                obj.insert(String::from("notes_markdown"), Value::String(info.notes_markdown.clone()));
            }
            val
        }),
        SheetSection::Analyses => doc.result_sheet(sheet_id).map(|rs| {
            let mut val = json!({
                "section": "Results",
                "id": rs.id.as_str(),
                "title": rs.analysis.effective_title(),
                "source_sheet_id": rs.analysis.source_sheet_id.as_str(),
                "status": format!("{:?}", rs.status),
                "tabs": rs.tabs.len(),
                "scalars": rs.scalars.len(),
                "checklist": rs.checklist.len(),
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                let (tabs_preview, tabs_truncated) = inspect_preview_bounds::slice_result_tabs_preview(&rs.tabs);
                let (scalars_preview, scalars_truncated) =
                    inspect_preview_bounds::slice_preview_vec(&rs.scalars, inspect_preview_bounds::MAX_RESULT_PREVIEW_SCALARS);
                let (checklist_preview, checklist_truncated) =
                    inspect_preview_bounds::slice_preview_vec(&rs.checklist, inspect_preview_bounds::MAX_RESULT_PREVIEW_CHECKLIST);
                obj.insert(String::from("tabs_data"), serde_json::to_value(&tabs_preview).unwrap_or(Value::Null));
                obj.insert(String::from("tabs_truncated"), Value::Bool(tabs_truncated));
                obj.insert(String::from("scalars_data"), serde_json::to_value(&scalars_preview).unwrap_or(Value::Null));
                obj.insert(String::from("scalars_truncated"), Value::Bool(scalars_truncated));
                obj.insert(String::from("checklist_entries"), serde_json::to_value(&checklist_preview).unwrap_or(Value::Null));
                obj.insert(String::from("checklist_truncated"), Value::Bool(checklist_truncated));
            }
            val
        }),
        SheetSection::Graphs => doc.graph_sheet(sheet_id).map(|gs| {
            let mut val = json!({
                "section": "Graphs",
                "id": gs.id.as_str(),
                "title": gs.title,
                "initialized": gs.initialized,
                "kind": format!("{:?}", gs.spec.kind),
                "width_pt": gs.spec.canvas_size_pt.width,
                "height_pt": gs.spec.canvas_size_pt.height,
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("spec"), serde_json::to_value(&gs.spec).unwrap_or(Value::Null));
            }
            val
        }),
        SheetSection::Layouts => doc.layout_sheet(sheet_id).map(|ls| {
            let size = ls.spec.page_size_pt();
            let mut val = json!({
                "section": "Layouts",
                "id": ls.id.as_str(),
                "title": ls.title,
                "items": ls.spec.items.len(),
                "width_pt": size.width,
                "height_pt": size.height,
                "upstream_dependencies": deps,
            });
            if include_cells && let Some(obj) = val.as_object_mut() {
                obj.insert(String::from("spec"), serde_json::to_value(&ls.spec).unwrap_or(Value::Null));
            }
            val
        }),
    };
    detail.ok_or_else(|| {
        EngineError::with_code(ErrorCode::NotFound, format!("sheet `{}` not found", sheet_id.as_str()))
            .with_details(json!({ "sheet_id": sheet_id.as_str() }))
    })
}

mod inspect_preview_bounds {
    use curvebook_doc::{
        ColumnTable, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, MultipleVariablesTable, NestedGroupColumn, NestedTable,
        PartsOfWholeTable, ResultTab, SurvivalTable, TableArchetype, VariableColumn, XyTable,
    };
    use curvebook_format::{CsvOptions, export_datasheet_to_csv};

    use crate::{EngineError, ErrorCode};

    pub(super) const MAX_CSV_PREVIEW_ROWS: usize = 100;
    pub(super) const MAX_CSV_PREVIEW_BYTES: usize = 65_536;
    pub(super) const MAX_RESULT_PREVIEW_TABS: usize = 32;
    pub(super) const MAX_RESULT_PREVIEW_ROWS: usize = 100;
    pub(super) const MAX_RESULT_PREVIEW_COLS: usize = 64;
    pub(super) const MAX_RESULT_PREVIEW_TOTAL_CELLS: usize = 4_096;
    pub(super) const MAX_RESULT_PREVIEW_FOOTNOTES: usize = 64;
    pub(super) const MAX_RESULT_PREVIEW_CHECKLIST: usize = 128;
    pub(super) const MAX_RESULT_PREVIEW_SCALARS: usize = 2_048;

    fn slice_dataset_columns(cols: &[DataSetColumn], max_rows: usize) -> Vec<DataSetColumn> {
        cols.iter().map(|c| DataSetColumn { title: c.title.clone(), cells: c.cells.iter().take(max_rows).cloned().collect() }).collect()
    }

    fn slice_archetype_rows(archetype: &TableArchetype, max_rows: usize) -> TableArchetype {
        match archetype {
            TableArchetype::Xy(t) => TableArchetype::Xy(XyTable {
                x_title: t.x_title.clone(),
                x_mode: t.x_mode.clone(),
                x_cells: t.x_cells.iter().take(max_rows).cloned().collect(),
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_titles_expanded: t.row_titles_expanded,
                y_subcolumn_mode: t.y_subcolumn_mode,
                data_sets: slice_dataset_columns(&t.data_sets, max_rows),
            }),
            TableArchetype::Column(t) => TableArchetype::Column(ColumnTable {
                design_hint: t.design_hint,
                subcolumn_mode: t.subcolumn_mode,
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_titles_expanded: t.row_titles_expanded,
                data_sets: slice_dataset_columns(&t.data_sets, max_rows),
            }),
            TableArchetype::Grouped(t) => TableArchetype::Grouped(GroupedTable {
                row_factor_title: t.row_factor_title.clone(),
                column_factor_title: t.column_factor_title.clone(),
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_numeric_spacing: t.row_numeric_spacing.as_ref().map(|v| v.iter().take(max_rows).copied().collect()),
                subcolumn_mode: t.subcolumn_mode,
                replicate_stacking: t.replicate_stacking,
                three_way_grouping: t.three_way_grouping.clone(),
                data_sets: slice_dataset_columns(&t.data_sets, max_rows),
            }),
            TableArchetype::Contingency(t) => TableArchetype::Contingency(ContingencyTable {
                row_factor_title: t.row_factor_title.clone(),
                column_factor_title: t.column_factor_title.clone(),
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                data_sets: slice_dataset_columns(&t.data_sets, max_rows),
            }),
            TableArchetype::Survival(t) => TableArchetype::Survival(SurvivalTable {
                time_title: t.time_title.clone(),
                times: t.times.iter().take(max_rows).cloned().collect(),
                event_code: t.event_code,
                censored_code: t.censored_code,
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_titles_expanded: t.row_titles_expanded,
                cohorts: slice_dataset_columns(&t.cohorts, max_rows),
            }),
            TableArchetype::PartsOfWhole(t) => TableArchetype::PartsOfWhole(PartsOfWholeTable {
                slice_labels: t.slice_labels.iter().take(max_rows).cloned().collect(),
                data_sets: slice_dataset_columns(&t.data_sets, max_rows),
            }),
            TableArchetype::MultipleVariables(t) => TableArchetype::MultipleVariables(MultipleVariablesTable {
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_titles_expanded: t.row_titles_expanded,
                variables: t
                    .variables
                    .iter()
                    .map(|v| VariableColumn {
                        title: v.title.clone(),
                        role: v.role,
                        role_inferred: v.role_inferred,
                        source: v.source.clone(),
                        level_order: v.level_order.clone(),
                        cells: v.cells.iter().take(max_rows).cloned().collect(),
                    })
                    .collect(),
            }),
            TableArchetype::Nested(t) => TableArchetype::Nested(NestedTable {
                row_titles: t.row_titles.iter().take(max_rows).cloned().collect(),
                row_titles_expanded: t.row_titles_expanded,
                groups: t
                    .groups
                    .iter()
                    .map(|g| NestedGroupColumn {
                        title: g.title.clone(),
                        subcolumn_titles: g.subcolumn_titles.clone(),
                        cells: g.cells.iter().take(max_rows).cloned().collect(),
                    })
                    .collect(),
            }),
        }
    }

    fn truncate_csv_bytes(mut csv: String, max_bytes: usize) -> (String, bool) {
        if csv.len() <= max_bytes {
            return (csv, false);
        }
        let mut end = max_bytes;
        while end > 0 && !csv.is_char_boundary(end) {
            end -= 1;
        }
        if let Some(nl) = csv.get(..end).and_then(|s| s.rfind('\n'))
            && nl > 0
        {
            end = nl + 1;
        }
        csv.truncate(end);
        (csv, true)
    }

    fn map_preview_err(err: impl std::fmt::Display) -> EngineError {
        EngineError::with_code(ErrorCode::BadInput, format!("CSV preview export failed: {err}"))
    }

    pub(super) fn build_csv_preview(ds: &DataSheet) -> Result<(String, bool), EngineError> {
        ds.archetype.validate().map_err(map_preview_err)?;
        let row_count = ds.archetype.row_count();
        let row_truncated = row_count > MAX_CSV_PREVIEW_ROWS;
        let raw_csv = if row_truncated {
            let sliced = slice_archetype_rows(&ds.archetype, MAX_CSV_PREVIEW_ROWS);
            let preview_ds = DataSheet::new(ds.id.clone(), ds.title.clone(), sliced).map_err(map_preview_err)?;
            export_datasheet_to_csv(&preview_ds, &CsvOptions::csv())
        } else {
            export_datasheet_to_csv(ds, &CsvOptions::csv())
        }
        .map_err(map_preview_err)?;
        let (capped, byte_truncated) = truncate_csv_bytes(raw_csv, MAX_CSV_PREVIEW_BYTES);
        Ok((capped, row_truncated || byte_truncated))
    }

    pub(super) fn slice_preview_vec<T: Clone>(items: &[T], limit: usize) -> (Vec<T>, bool) {
        let truncated = items.len() > limit;
        (items.iter().take(limit).cloned().collect(), truncated)
    }

    pub(super) fn slice_result_tabs_preview(tabs: &[ResultTab]) -> (Vec<ResultTab>, bool) {
        let mut truncated = tabs.len() > MAX_RESULT_PREVIEW_TABS;
        let mut remaining_cells = MAX_RESULT_PREVIEW_TOTAL_CELLS;
        let mut out = Vec::with_capacity(tabs.len().min(MAX_RESULT_PREVIEW_TABS));

        for tab in tabs.iter().take(MAX_RESULT_PREVIEW_TABS) {
            if tab.column_headers.len() > MAX_RESULT_PREVIEW_COLS || tab.rows.len() > MAX_RESULT_PREVIEW_ROWS {
                truncated = true;
            }
            let column_headers: Vec<String> = tab.column_headers.iter().take(MAX_RESULT_PREVIEW_COLS).cloned().collect();
            let mut out_rows = Vec::new();
            let mut out_row_labels = Vec::new();
            for (r_idx, row) in tab.rows.iter().take(MAX_RESULT_PREVIEW_ROWS).enumerate() {
                if row.len() > MAX_RESULT_PREVIEW_COLS {
                    truncated = true;
                }
                let take_cols = row.len().min(MAX_RESULT_PREVIEW_COLS);
                if take_cols > remaining_cells {
                    truncated = true;
                    break;
                }
                remaining_cells -= take_cols;
                out_rows.push(row.iter().take(take_cols).cloned().collect());
                if let Some(lbl) = tab.row_labels.get(r_idx) {
                    out_row_labels.push(lbl.clone());
                }
            }
            if tab.row_labels.len() > out_rows.len() || tab.footnotes.len() > MAX_RESULT_PREVIEW_FOOTNOTES {
                truncated = true;
            }
            let footnotes = tab.footnotes.iter().take(MAX_RESULT_PREVIEW_FOOTNOTES).cloned().collect();
            out.push(ResultTab {
                id: tab.id.clone(),
                title: tab.title.clone(),
                kind: tab.kind,
                column_headers,
                row_labels: out_row_labels,
                rows: out_rows,
                footnotes,
            });
        }

        (out, truncated)
    }
}

// ---------------------------------------------------------------------------
// Atomic Batch Execution Service
// ---------------------------------------------------------------------------

/// Single command step inside an atomic batch execution request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchCommandStep {
    /// Registered engine command identifier (e.g. `"table.setCells"`).
    pub command_id: String,
    /// JSON parameter object for the command.
    pub params: Value,
}

impl BatchCommandStep {
    /// Construct a new batch step.
    #[must_use]
    pub fn new(command_id: impl Into<String>, params: Value) -> Self {
        Self { command_id: command_id.into(), params }
    }
}

/// Structured failure report emitted when [`Session::execute_batch_atomic`] rolls back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchStepFailure {
    /// Zero-based index of the failing step in the submitted batch.
    pub failed_index: usize,
    /// Identifier of the failing command.
    pub failed_command_id: String,
    /// Underlying structured engine error.
    pub error: EngineError,
    /// Outcomes of steps that succeeded prior to the failure (before rollback).
    pub completed_before_failure: Vec<CommandOutcome>,
    /// Indices and command IDs of steps that performed host filesystem writes before failure.
    pub persisted_filesystem_steps: Vec<(usize, String)>,
}

fn has_non_empty_dest_path_param(params: &Value) -> bool {
    let Some(obj) = params.as_object() else {
        return false;
    };
    ["path", "output_path", "outputPath", "file_path", "filePath"]
        .iter()
        .any(|key| obj.get(*key).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()))
}

fn is_optional_path_export_or_save_cmd(command_id: &str) -> bool {
    matches!(
        command_id,
        "project.save" | "project.saveTemplate" | "table.exportCsv" | "analysis.exportJson" | "analysis.exportCsv" | "result.generateReport"
    ) || command_id.starts_with("export.")
        || command_id.contains(".export")
}

/// Return `true` when invoking `command_id` with `params` will write to the host filesystem.
#[must_use]
pub fn command_step_writes_filesystem(command_id: &str, params: &Value) -> bool {
    if command_id == "ui.screenshot" {
        return has_non_empty_dest_path_param(params);
    }
    let Some(spec) = registry().get(command_id) else {
        return false;
    };
    if !spec.effects.contains(Effects::WRITES_FILESYSTEM) {
        return false;
    }
    if is_optional_path_export_or_save_cmd(command_id) { has_non_empty_dest_path_param(params) } else { true }
}

fn outcome_has_written_path(outcome: &CommandOutcome) -> bool {
    outcome.payload.get("path").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty())
}

impl Session {
    /// Execute a slice of [`BatchCommandStep`]s atomically against `self`.
    ///
    /// Snapshots the entire session state (`doc`, `file_path`, `dirty`, `undo_stack`, `redo_stack`)
    /// before running any step so rollback on error or cancellation is exact even when the batch
    /// executes more mutating commands than [`crate::MAX_UNDO_ENTRIES`] or triggers undo-stack
    /// memory-budget eviction.
    ///
    /// On success, intermediate undo entries collapse into a single undo step via
    /// [`Self::collapse_atomic_group`]; purely read-only batches leave `dirty` and undo/redo stacks
    /// untouched.
    ///
    /// # Errors
    /// Returns [`BatchStepFailure`] describing the failing step, prior outcomes, and any non-revertible
    /// host filesystem writes after restoring `self` to its exact pre-batch state.
    pub fn execute_batch_atomic(
        &mut self,
        steps: &[BatchCommandStep],
        caller: Caller,
        cancelled: &AtomicBool,
    ) -> Result<Vec<CommandOutcome>, Box<BatchStepFailure>> {
        let baseline = self.clone();
        let mut outcomes = Vec::with_capacity(steps.len());
        let mut persisted_filesystem_steps: Vec<(usize, String)> = Vec::new();

        for (idx, step) in steps.iter().enumerate() {
            if cancelled.load(Ordering::Acquire) {
                *self = baseline;
                return Err(Box::new(BatchStepFailure {
                    failed_index: idx,
                    failed_command_id: step.command_id.clone(),
                    error: EngineError::with_code(ErrorCode::Cancelled, format!("command `{}` cancelled", step.command_id))
                        .with_retryable(true)
                        .with_details(json!({ "command_id": step.command_id })),
                    completed_before_failure: outcomes,
                    persisted_filesystem_steps,
                }));
            }
            let writes_fs_hint = command_step_writes_filesystem(&step.command_id, &step.params);
            match self.execute_with_caller(&step.command_id, step.params.clone(), caller, cancelled) {
                Ok(outcome) => {
                    if writes_fs_hint || outcome_has_written_path(&outcome) {
                        persisted_filesystem_steps.push((idx, step.command_id.clone()));
                    }
                    outcomes.push(outcome);
                }
                Err(error) => {
                    *self = baseline;
                    return Err(Box::new(BatchStepFailure {
                        failed_index: idx,
                        failed_command_id: step.command_id.clone(),
                        error,
                        completed_before_failure: outcomes,
                        persisted_filesystem_steps,
                    }));
                }
            }
        }

        let _ = self.collapse_atomic_group(baseline);
        Ok(outcomes)
    }
}
