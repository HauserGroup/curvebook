//! Shared style, hook expansion, segmented frame, path geometry, legend, colorbar, and sheet-grid helpers.

use std::f64::consts::PI;

use curvebook_doc::{
    DataSheet, GraphAnnotation, GraphSheet, GraphSpec, ProjectDocument, ResultCell, ResultStatus, ResultTab, SheetId, TableArchetype, TickLabelFormat,
};
use curvebook_math::AxisScale;
use curvebook_palette::{ColorRamp, LineDashStyle, MarkerFillMode, MarkerStyle, Rgba8};
use curvebook_text::{LocaleNumberPolicy, RichText, TextMetricsPt, format_axis_tick, measure_text_pt, parse_markup};

use crate::scales::{LegendEntry as ScaledLegendEntry, format_tick_markup, sanitize_linear_range};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};
use crate::xy::{ChromeAxes, SegmentedAxis, render_cartesian_chrome_full};

const COLORBAR_STEPS: usize = 48;

/// Margin calculation hints for dynamic `plot_rect_pt` sizing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlotMarginConfig {
    pub extra_top_pt: f64,
    pub extra_bottom_pt: f64,
    pub extra_left_pt: f64,
    pub extra_right_pt: f64,
    pub has_legend: bool,
    pub legend_box_width_pt: Option<f64>,
    pub has_colorbar: bool,
    pub suppress_y_axis_labels: bool,
}

impl Default for PlotMarginConfig {
    fn default() -> Self {
        Self {
            extra_top_pt: 0.0,
            extra_bottom_pt: 0.0,
            extra_left_pt: 0.0,
            extra_right_pt: 0.0,
            has_legend: false,
            legend_box_width_pt: None,
            has_colorbar: false,
            suppress_y_axis_labels: false,
        }
    }
}

/// Shared axis domain + margin specification rendered through `xy::SegmentedAxis` and
/// `xy::render_cartesian_chrome_full`.
pub(crate) struct SegmentedFrameSpec<'a> {
    pub x_bounds: (f64, f64),
    pub y_bounds: (f64, f64),
    pub y2_bounds: Option<(f64, f64)>,
    pub cat_x: Option<&'a [(f64, String)]>,
    pub cat_y: Option<&'a [(f64, String)]>,
    pub margins: PlotMarginConfig,
}

/// Legend swatch entry rendered beside or over the plot area.
#[derive(Debug, Clone)]
pub(crate) struct LegendEntry {
    pub label: String,
    pub color: Rgba8,
    pub marker: Option<MarkerStyle>,
    pub dash: Option<LineDashStyle>,
    pub is_line: bool,
}

// ============================================================================
// Shared Style, Hook, Axis & Geometry Primitives
// ============================================================================

pub(crate) fn solid_stroke(color: Rgba8, width_pt: f64, dash: LineDashStyle) -> StrokePaint {
    let mut paint = StrokePaint::solid(color, if width_pt.is_finite() && width_pt > 0.0 { width_pt } else { 1.0 });
    paint.dash = dash;
    paint
}

pub(crate) fn alpha_fill(color: Rgba8, alpha: u8) -> FillPaint {
    FillPaint::solid(Rgba8::rgba(color.r, color.g, color.b, alpha))
}

pub(crate) fn draw_empty_open_frame(scene: &mut ChartScene) {
    let r = scene.plot_rect_pt;
    let axis_stroke = solid_stroke(Rgba8::BLACK, 1.0, LineDashStyle::Solid);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(r.x, r.y), PointPt::new(r.x, r.bottom())),
        fill: None,
        stroke: Some(axis_stroke),
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(r.x, r.bottom()), PointPt::new(r.right(), r.bottom())),
        fill: None,
        stroke: Some(axis_stroke),
        clip: None,
    });
}

/// Expand `{{info:...}}` and `{{result:...}}` template hooks using `doc`, preferring `primary_info_id`.
pub(crate) fn resolve_graph_text(raw: &str, doc: &ProjectDocument, primary_info_id: Option<&SheetId>) -> String {
    crate::scales::resolve_template_hooks_with_info(raw, doc, primary_info_id)
}

pub(crate) fn sanitize_range(min_v: f64, max_v: f64, fallback: (f64, f64)) -> (f64, f64) {
    sanitize_linear_range(min_v, max_v, fallback)
}

pub(crate) fn format_tick_value(val: f64, scale: AxisScale, fmt: TickLabelFormat) -> String {
    if !val.is_finite() {
        return String::new();
    }
    format_tick_markup(val, scale, fmt, &LocaleNumberPolicy::default())
}

pub(crate) fn text_metrics(text: &str, font_size_pt: f64) -> TextMetricsPt {
    let rich = if text.contains('<') { parse_markup(text) } else { RichText::plain(text) };
    measure_text_pt(&rich, font_size_pt)
}

/// Adjust `scene.plot_rect_pt` dynamically from formatted Y tick label widths and chrome budgets.
pub(crate) fn update_plot_rect_for_labels(scene: &mut ChartScene, spec: &GraphSpec, y_ticks: &[(f64, String)], cfg: PlotMarginConfig) -> f64 {
    let max_y_w =
        if cfg.suppress_y_axis_labels { 0.0 } else { y_ticks.iter().map(|(_, lbl)| text_metrics(lbl, 8.5).width_pt).fold(0.0_f64, f64::max) };
    let y_title_space = if !cfg.suppress_y_axis_labels && !spec.y_axis.title.trim().is_empty() { 15.0 } else { 0.0 };
    let left_base = if cfg.suppress_y_axis_labels { 20.0 } else { (max_y_w + y_title_space + 16.0).max(38.0) };
    let left = (left_base + cfg.extra_left_pt).min(scene.width_pt * 0.48);

    let top = (if spec.title.trim().is_empty() { 20.0 } else { 34.0 } + cfg.extra_top_pt.max(0.0)).min(scene.height_pt * 0.42);
    let right_legend: f64 = if cfg.has_colorbar {
        76.0
    } else if cfg.has_legend && spec.legend.visible && spec.legend.position_pt.is_none() {
        cfg.legend_box_width_pt.map_or(86.0, |box_w| (box_w + 12.0).max(86.0))
    } else {
        18.0
    };
    let max_right = if cfg.extra_right_pt > 0.0 || cfg.legend_box_width_pt.is_some() { scene.width_pt * 0.44 } else { scene.width_pt * 0.34 };
    let right = (right_legend + cfg.extra_right_pt.max(0.0)).min(max_right);

    let x_title_space = if !spec.x_axis.title.trim().is_empty() { 15.0 } else { 0.0 };
    let rotated_tick_extra = if spec.x_axis.tick_angle_deg.abs() > 10.0 { 16.0 } else { 0.0 };
    let bottom = (24.0 + x_title_space + rotated_tick_extra + cfg.extra_bottom_pt).min(scene.height_pt * 0.52);

    let w = (scene.width_pt - left - right).max(24.0);
    let h = (scene.height_pt - top - bottom).max(24.0);
    scene.plot_rect_pt = RectPt::new(left, top, w, h);
    max_y_w
}

/// Resolve segmented X/Y(/Y2) axes, adjust `scene.plot_rect_pt`, and emit full cartesian chrome.
pub(crate) fn layout_and_render_segmented_chrome(
    scene: &mut ChartScene,
    graph: &GraphSheet,
    doc: &ProjectDocument,
    frame: &SegmentedFrameSpec<'_>,
) -> (SegmentedAxis, SegmentedAxis, Option<SegmentedAxis>) {
    let policy = LocaleNumberPolicy::default();
    let y_preview_axis = SegmentedAxis::build_exact(&graph.spec.y_axis, scene.height_pt - 40.0, 20.0, frame.y_bounds);
    let y_preview_ticks: Vec<(f64, String)> = if frame.margins.suppress_y_axis_labels {
        Vec::new()
    } else if let Some(cy) = frame.cat_y {
        cy.to_vec()
    } else {
        y_preview_axis.preview_tick_labels(&policy).into_iter().enumerate().map(|(idx, lbl)| (idx as f64, lbl)).collect()
    };
    let _ = update_plot_rect_for_labels(scene, &graph.spec, &y_preview_ticks, frame.margins);

    let r = scene.plot_rect_pt;
    let x_axis = SegmentedAxis::build_exact(&graph.spec.x_axis, r.x, r.right(), frame.x_bounds);
    let y_axis = SegmentedAxis::build_exact(&graph.spec.y_axis, r.bottom(), r.y, frame.y_bounds);
    let y2_axis = frame.y2_bounds.and_then(|b| graph.spec.y2_axis.as_ref().map(|y2| SegmentedAxis::build_exact(y2, r.bottom(), r.y, b)));
    render_cartesian_chrome_full(
        scene,
        graph,
        doc,
        &ChromeAxes { x: &x_axis, y: &y_axis, y2: y2_axis.as_ref(), cat_x: frame.cat_x, cat_y: frame.cat_y },
    );
    (x_axis, y_axis, y2_axis)
}

pub(crate) fn path_move(path: &mut Path2D, pt: PointPt) {
    path.commands.push(PathCmd::MoveTo(pt));
}

pub(crate) fn path_line(path: &mut Path2D, pt: PointPt) {
    path.commands.push(PathCmd::LineTo(pt));
}

pub(crate) fn path_cubic(path: &mut Path2D, c1: PointPt, c2: PointPt, to: PointPt) {
    path.commands.push(PathCmd::CubicTo { c1, c2, to });
}

pub(crate) fn path_close(path: &mut Path2D) {
    path.commands.push(PathCmd::Close);
}

pub(crate) fn polygon_path(pts: &[(f64, f64)]) -> Path2D {
    let mut p = Path2D::new();
    let Some(&(x0, y0)) = pts.first() else { return p };
    path_move(&mut p, PointPt::new(x0, y0));
    for &(x, y) in pts.iter().skip(1) {
        path_line(&mut p, PointPt::new(x, y));
    }
    path_close(&mut p);
    p
}

/// Build an annular or full wedge path from `start_rad` to `end_rad` using cubic Beziers.
pub(crate) fn wedge_path(cx: f64, cy: f64, r_outer: f64, r_inner: f64, start_rad: f64, end_rad: f64) -> Path2D {
    let sweep = end_rad - start_rad;
    if sweep.abs() < 1e-6 || r_outer <= 0.0 {
        return Path2D::new();
    }
    let segments = ((sweep.abs() / (PI * 0.5)).ceil() as usize).clamp(1, 8);
    let mut path = Path2D::new();
    let x0 = cx + r_outer * start_rad.cos();
    let y0 = cy + r_outer * start_rad.sin();

    if r_inner <= 1e-4 {
        path_move(&mut path, PointPt::new(cx, cy));
        path_line(&mut path, PointPt::new(x0, y0));
        append_arc_beziers(&mut path, cx, cy, r_outer, start_rad, end_rad, segments);
        path_close(&mut path);
    } else {
        path_move(&mut path, PointPt::new(x0, y0));
        append_arc_beziers(&mut path, cx, cy, r_outer, start_rad, end_rad, segments);
        let xi_end = cx + r_inner * end_rad.cos();
        let yi_end = cy + r_inner * end_rad.sin();
        path_line(&mut path, PointPt::new(xi_end, yi_end));
        append_arc_beziers(&mut path, cx, cy, r_inner, end_rad, start_rad, segments);
        path_close(&mut path);
    }
    path
}

fn append_arc_beziers(path: &mut Path2D, cx: f64, cy: f64, r: f64, a_start: f64, a_end: f64, segments: usize) {
    let dt = (a_end - a_start) / (segments as f64);
    let k = (4.0 / 3.0) * (dt * 0.25).tan();
    for i in 0..segments {
        let a0 = a_start + (i as f64) * dt;
        let a1 = a0 + dt;
        let (s0, c0) = a0.sin_cos();
        let (s1, c1) = a1.sin_cos();
        let p1 = PointPt::new(cx + r * (c0 - k * s0), cy + r * (s0 + k * c0));
        let p2 = PointPt::new(cx + r * (c1 + k * s1), cy + r * (s1 - k * c1));
        let p3 = PointPt::new(cx + r * c1, cy + r * s1);
        path_cubic(path, p1, p2, p3);
    }
}

pub(crate) fn draw_legend_entries(scene: &mut ChartScene, spec: &GraphSpec, entries: &[LegendEntry], doc: &ProjectDocument) {
    if !spec.legend.visible || entries.is_empty() {
        return;
    }
    let scaled: Vec<ScaledLegendEntry> = entries
        .iter()
        .map(|e| {
            if e.is_line {
                ScaledLegendEntry {
                    label: e.label.clone(),
                    swatch_color: e.color,
                    stroke_color: Some(e.color),
                    marker_shape: e.marker.map(|m| m.shape),
                    marker_fill_mode: e.marker.map_or(MarkerFillMode::Filled, |m| m.fill_mode),
                    dash_style: e.dash.or(Some(LineDashStyle::Solid)),
                    is_box_swatch: false,
                }
            } else if let Some(m) = e.marker {
                ScaledLegendEntry {
                    label: e.label.clone(),
                    swatch_color: m.fill_color,
                    stroke_color: Some(m.stroke_color),
                    marker_shape: Some(m.shape),
                    marker_fill_mode: m.fill_mode,
                    dash_style: None,
                    is_box_swatch: false,
                }
            } else {
                ScaledLegendEntry {
                    label: e.label.clone(),
                    swatch_color: e.color,
                    stroke_color: Some(Rgba8::rgb(40, 40, 40)),
                    marker_shape: None,
                    marker_fill_mode: MarkerFillMode::Filled,
                    dash_style: None,
                    is_box_swatch: true,
                }
            }
        })
        .collect();
    let info_id = doc.paired_info_sheet_id(&spec.primary_source_sheet_id);
    let _ = crate::scales::render_legend_with_placement(
        &spec.legend,
        &scaled,
        doc,
        info_id,
        crate::scales::LegendPlacement { outside_right: true, outside_right_offset_pt: 0.0 },
        scene,
    );
}

pub(crate) fn draw_colorbar(scene: &mut ChartScene, ramp: &ColorRamp, min_v: f64, max_v: f64, title: &str, bar_rect: RectPt) {
    let step_h = bar_rect.height / (COLORBAR_STEPS as f64);
    for i in 0..COLORBAR_STEPS {
        let t = 1.0 - ((i as f64) + 0.5) / (COLORBAR_STEPS as f64);
        let color = ramp.sample(t);
        let y = bar_rect.y + (i as f64) * step_h;
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(bar_rect.x, y, bar_rect.width, step_h + 0.5)),
            fill: Some(FillPaint::solid(color)),
            stroke: None,
            clip: None,
        });
    }
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(bar_rect),
        fill: None,
        stroke: Some(solid_stroke(Rgba8::BLACK, 0.75, LineDashStyle::Solid)),
        clip: None,
    });

    if !title.trim().is_empty() {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(title),
            anchor_pt: PointPt::new(bar_rect.x + bar_rect.width * 0.5, (bar_rect.y - 5.0).max(10.0)),
            anchor: TextAnchor::Middle,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }

    let (lo, hi) = sanitize_range(min_v, max_v, (0.0, 1.0));
    let mid = 0.5 * (lo + hi);
    for (v, y) in [(hi, bar_rect.y + 4.0), (mid, bar_rect.y + bar_rect.height * 0.5 + 3.0), (lo, bar_rect.bottom())] {
        let lbl = format_tick_value(v, AxisScale::Linear, TickLabelFormat::Decimal);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(lbl),
            anchor_pt: PointPt::new(bar_rect.right() + 4.0, y),
            anchor: TextAnchor::Start,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}

pub(crate) fn draw_graph_annotations(
    scene: &mut ChartScene,
    annotations: &[GraphAnnotation],
    doc: &ProjectDocument,
    primary_info_id: Option<&SheetId>,
) -> Result<(), ChartError> {
    let mut budget = crate::scales::AnnotationBudget::default();
    crate::scales::render_annotations_with_budget(annotations, doc, primary_info_id, &mut budget, scene)
}

pub(crate) fn format_result_cell(cell: &ResultCell) -> String {
    format_result_cell_with_sig_figs(cell, None)
}

pub(crate) fn format_result_cell_with_sig_figs(cell: &ResultCell, sig_figs: Option<u8>) -> String {
    let fmt_num = |v: f64| match sig_figs {
        Some(d) => curvebook_doc::format_significant_digits(v, d),
        None => crate::scales::format_scalar_clean(v),
    };
    match cell {
        ResultCell::Empty => String::new(),
        ResultCell::Number { value, prefix, units } => {
            let mut out = String::new();
            if let Some(p) = prefix {
                out.push_str(p);
            }
            out.push_str(&fmt_num(*value));
            if let Some(u) = units
                && !u.is_empty()
            {
                out.push(' ');
                out.push_str(u);
            }
            out
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            if matches!(lower, curvebook_doc::CiBound::Unavailable) && matches!(upper, curvebook_doc::CiBound::Unavailable) {
                return String::from("Unavailable");
            }
            let fmt_b = |b: &curvebook_doc::CiBound, open_str: &str| match b {
                curvebook_doc::CiBound::Finite(v) => fmt_num(*v),
                curvebook_doc::CiBound::Open => String::from(open_str),
                curvebook_doc::CiBound::Unavailable => String::from("Unavailable"),
            };
            format!("[{}, {}]", fmt_b(lower, "-∞"), fmt_b(upper, "+∞"))
        }
        ResultCell::PValue { formatted, stars, .. } => {
            if stars.trim().is_empty() {
                formatted.clone()
            } else {
                format!("{formatted} {stars}")
            }
        }
        ResultCell::Text(formatted) => formatted.clone(),
        ResultCell::Boolean(b) => String::from(if *b { "Yes" } else { "No" }),
    }
}

pub(crate) struct SheetGrid {
    pub headers: Vec<String>,
    pub row_labels: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub(crate) fn extract_sheet_grid(doc: &ProjectDocument, source_sheet_id: &SheetId, tab_id: Option<&str>) -> Option<SheetGrid> {
    if let Some(res) = doc.result_sheet(source_sheet_id) {
        if !res.has_displayable_output() {
            return None;
        }
        let tab: Option<&ResultTab> = tab_id.and_then(|tid| res.tabs.iter().find(|t| t.id == tid)).or_else(|| res.tabs.first());
        if let Some(t) = tab {
            let rows = t.rows.iter().map(|r| r.iter().map(format_result_cell).collect()).collect();
            return Some(SheetGrid { headers: t.column_headers.clone(), row_labels: t.row_labels.clone(), rows });
        }
        let finite_scalars: Vec<_> = res.scalars.iter().filter(|s| s.value.is_finite()).collect();
        if !finite_scalars.is_empty() {
            let policy = LocaleNumberPolicy::default();
            let headers = vec![String::from("Value")];
            let row_labels = finite_scalars.iter().map(|s| s.key.clone()).collect();
            let rows = finite_scalars.iter().map(|s| vec![format_axis_tick(s.value, AxisScale::Linear, &policy)]).collect();
            return Some(SheetGrid { headers, row_labels, rows });
        }
        if let Some(ds) = res.analyzable_data.as_ref() {
            return Some(extract_data_sheet_grid(ds));
        }
    }
    if let Some(ds) = doc.data_sheet(source_sheet_id) {
        return Some(extract_data_sheet_grid(ds));
    }
    if let Some(info) = doc.info_sheet(source_sheet_id) {
        let headers = vec![String::from("Value"), String::from("Units")];
        let row_labels = info.constants.iter().map(|c| c.name.clone()).collect();
        let rows = info.constants.iter().map(|c| vec![c.value.display_text(), c.units.clone().unwrap_or_default()]).collect();
        return Some(SheetGrid { headers, row_labels, rows });
    }
    None
}

/// Resolve a [`DataSheet`] from `id`, refusing to read stale `Dirty` or `Failed` intermediate
/// [`curvebook_doc::ResultSheet`]s and emitting an explicit [`ChartWarning`] when blocked.
pub(super) fn resolve_fresh_data_sheet_or_upstream<'a>(
    doc: &'a ProjectDocument,
    id: &SheetId,
    warnings: &mut Vec<ChartWarning>,
) -> Option<&'a DataSheet> {
    let mut visited = std::collections::HashSet::new();
    let mut cursor = id;
    while visited.insert(cursor) {
        if let Some(ds) = doc.data_sheet(cursor) {
            return Some(ds);
        }
        let res = doc.result_sheet(cursor)?;
        match &res.status {
            ResultStatus::UpToDate => {}
            ResultStatus::Failed { .. } => {
                let msg = format!("result sheet '{}' failed; omitted stale analysis data", res.id);
                if !warnings.iter().any(|w| w.code == "failed_result_sheet" && w.message == msg) {
                    warnings.push(ChartWarning::new("failed_result_sheet", msg));
                }
                return None;
            }
            ResultStatus::Dirty => {
                let msg = format!("result sheet '{}' is pending recalculation; omitted stale analysis data", res.id);
                if !warnings.iter().any(|w| w.code == "pending_analysis_result" && w.message == msg) {
                    warnings.push(ChartWarning::new("pending_analysis_result", msg));
                }
                return None;
            }
            ResultStatus::Imported | ResultStatus::ImportedStale => return None,
        }
        if let Some(analyzable) = res.analyzable_data.as_ref() {
            return Some(analyzable);
        }
        let next = res.analysis.source_sheet_id.as_ref()?;
        cursor = next;
    }
    None
}

pub(super) fn trim_trailing_zeros(buf: &mut String) {
    if buf.contains('.') {
        while buf.ends_with('0') {
            let _ = buf.pop();
        }
        if buf.ends_with('.') {
            let _ = buf.pop();
        }
    }
}

fn extract_data_sheet_grid(ds: &DataSheet) -> SheetGrid {
    let fmt_cell = |c: &curvebook_doc::Cell| -> String {
        match &c.value {
            curvebook_doc::CellValue::Empty => String::new(),
            curvebook_doc::CellValue::Number(v) if v.is_finite() => {
                let mut s = format!("{v:.4}");
                trim_trailing_zeros(&mut s);
                s
            }
            curvebook_doc::CellValue::Number(_) => String::from("NaN"),
            curvebook_doc::CellValue::Text(t) => t.clone(),
        }
    };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let headers = mv.variables.iter().map(|v| v.title.clone()).collect();
            let nrows = mv.row_count().min(20);
            let mut rows = Vec::with_capacity(nrows);
            let mut row_labels = Vec::with_capacity(nrows);
            for r in 0..nrows {
                row_labels.push(mv.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("{}", r + 1)));
                rows.push(mv.variables.iter().map(|v| v.cells.get(r).map_or_else(String::new, fmt_cell)).collect());
            }
            SheetGrid { headers, row_labels, rows }
        }
        TableArchetype::Column(col) => {
            let headers = col.data_sets.iter().map(|d| d.title.clone()).collect();
            let nrows = col.row_count().min(20);
            let mut rows = Vec::with_capacity(nrows);
            let mut row_labels = Vec::with_capacity(nrows);
            for r in 0..nrows {
                row_labels.push(col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("{}", r + 1)));
                rows.push(col.data_sets.iter().map(|d| d.cells.get(r).and_then(|s| s.first()).map_or_else(String::new, fmt_cell)).collect());
            }
            SheetGrid { headers, row_labels, rows }
        }
        TableArchetype::Contingency(c) => {
            let headers = c.data_sets.iter().map(|d| d.title.clone()).collect();
            let nrows = c.row_count().min(20);
            let mut rows = Vec::with_capacity(nrows);
            let mut row_labels = Vec::with_capacity(nrows);
            for r in 0..nrows {
                row_labels.push(c.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)));
                rows.push(c.data_sets.iter().map(|d| d.cells.get(r).and_then(|s| s.first()).map_or_else(String::new, fmt_cell)).collect());
            }
            SheetGrid { headers, row_labels, rows }
        }
        _ => SheetGrid {
            headers: vec![String::from("Sheet")],
            row_labels: vec![ds.title.clone()],
            rows: vec![vec![format!("{} rows", ds.archetype.row_count())]],
        },
    }
}
