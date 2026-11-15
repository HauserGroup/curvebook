//! Survival Kaplan-Meier step curves, contingency bars, and parts-of-whole geometry builders,
//! plus shared axis, marker, annotation, and table-grid primitives for `lumencraft-chart`.

use std::f64::consts::PI;

use lumencraft_doc::{
    AnalysisKind, AxisSpec, AxisThreshold, CategoricalTickSource, DataSheet, FramePreset, GraphAnnotation, GraphAxisScale, GraphKind, GraphSheet,
    GraphSpec, GraphicalDataObject, GroupedArrangement, PartsOfWholeLayout, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SeriesSpec, SheetId, SurvivalAnalysisSpec, SurvivalObservation, SurvivalStatus, TableArchetype, TickLabelFormat,
};
use lumencraft_math::{AxisScale, ConfidenceLevel};
use lumencraft_palette::{ColorRamp, LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use lumencraft_stats::survival::kaplan_meier_curve;
use lumencraft_text::{LocaleNumberPolicy, RichText, TextMetricsPt, expand_template_hooks, format_axis_tick, measure_text_pt, parse_markup};

use crate::scales::{
    AxisMapping, AxisRenderOptions, ResolvedAdditionalTick, compute_additional_ticks_for_segments, emit_additional_tick_gridline_segments,
    emit_additional_x_tick_primitives, emit_additional_y_tick_primitives,
};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PathCmd, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive,
    SourceCellId, StrokePaint, TextAnchor,
};
use crate::xy::{compute_safe_linear_ticks, format_point_value, generate_prob_ticks_for_range, prob_auto_domain_max, prob_auto_domain_min};

const CIRCLE_KAPPA: f64 = 0.552_284_749_830_793_6;
const COLORBAR_STEPS: usize = 48;

/// Margin calculation hints for dynamic `plot_rect_pt` sizing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PlotMarginConfig {
    pub extra_bottom_pt: f64,
    pub extra_left_pt: f64,
    pub has_legend: bool,
    pub has_colorbar: bool,
    pub suppress_y_axis_labels: bool,
}

impl Default for PlotMarginConfig {
    fn default() -> Self {
        Self { extra_bottom_pt: 0.0, extra_left_pt: 0.0, has_legend: false, has_colorbar: false, suppress_y_axis_labels: false }
    }
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

/// Compile a survival, contingency, or parts-of-whole graph sheet into `scene`.
pub fn compile_survival_or_categorical_graph(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    if doc.data_sheet(&graph.spec.primary_source_sheet_id).is_none() && doc.result_sheet(&graph.spec.primary_source_sheet_id).is_none() {
        scene
            .warnings
            .push(ChartWarning::new("missing_source_sheet", format!("primary data sheet '{}' not found", graph.spec.primary_source_sheet_id)));
    }

    match graph.spec.kind {
        GraphKind::Survival => compile_survival(graph, doc, scene),
        GraphKind::Contingency => compile_contingency(graph, doc, scene),
        GraphKind::PartsOfWhole => compile_parts_of_whole(graph, doc, scene),
        _ => {
            scene.warnings.push(ChartWarning::new("unexpected_graph_kind", "non-categorical kind dispatched to survival_cat"));
            draw_empty_open_frame(scene);
            Ok(())
        }
    }?;

    draw_graph_annotations(scene, &graph.spec.annotations, doc);
    Ok(())
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

/// Expand `{{info:...}}` and `{{result:...}}` template hooks using `doc`.
pub(crate) fn resolve_graph_text(raw: &str, doc: &ProjectDocument) -> String {
    if !raw.contains("{{") {
        return raw.to_owned();
    }
    expand_template_hooks(raw, |token| resolve_hook_token(token, doc))
}

fn resolve_hook_token(token: &str, doc: &ProjectDocument) -> Option<String> {
    let parts: Vec<&str> = token.split(':').map(str::trim).collect();
    match parts.as_slice() {
        ["info", name] => {
            for info in &doc.info_sheets {
                if let Some(constant) = info.get_constant(name) {
                    let txt = constant.value.display_text();
                    if !txt.is_empty() {
                        return Some(txt);
                    }
                }
            }
            None
        }
        ["info", sheet, name] => {
            let info = doc.info_sheet(&SheetId::new(*sheet))?;
            let constant = info.get_constant(name)?;
            Some(constant.value.display_text())
        }
        ["result", sheet, param] => resolve_result_scalar(doc.result_sheet(&SheetId::new(*sheet))?, param, None),
        ["result", sheet, ds_str, param] => {
            let ds = ds_str.parse::<usize>().ok()?;
            resolve_result_scalar(doc.result_sheet(&SheetId::new(*sheet))?, param, Some(ds))
        }
        _ => None,
    }
}

fn resolve_result_scalar(res: &ResultSheet, key: &str, data_set_index: Option<usize>) -> Option<String> {
    let needle = key.trim();
    if let Some(entry) =
        res.scalars.iter().find(|s| s.key.trim().eq_ignore_ascii_case(needle) && (data_set_index.is_none() || s.data_set_index == data_set_index))
    {
        if !entry.value.is_finite() {
            return None;
        }
        let mut s = format!("{:.4}", entry.value);
        trim_trailing_zeros(&mut s);
        if let Some(u) = &entry.units
            && !u.trim().is_empty()
        {
            s.push(' ');
            s.push_str(u.trim());
        }
        return Some(s);
    }
    res.scalars.iter().find(|s| s.key.trim().eq_ignore_ascii_case(needle)).and_then(|entry| {
        if !entry.value.is_finite() {
            return None;
        }
        let mut s = format!("{:.4}", entry.value);
        trim_trailing_zeros(&mut s);
        Some(s)
    })
}

fn trim_trailing_zeros(buf: &mut String) {
    if buf.contains('.') {
        while buf.ends_with('0') {
            let _ = buf.pop();
        }
        if buf.ends_with('.') {
            let _ = buf.pop();
        }
    }
}

pub(crate) fn sanitize_range(min_v: f64, max_v: f64, fallback: (f64, f64)) -> (f64, f64) {
    if !min_v.is_finite() || !max_v.is_finite() {
        return fallback;
    }
    let (lo, hi) = if min_v <= max_v { (min_v, max_v) } else { (max_v, min_v) };
    if (hi - lo).abs() <= 1e-12 {
        let pad = if lo.abs() > 1.0 { lo.abs() * 0.1 } else { 1.0 };
        (lo - pad, hi + pad)
    } else {
        (lo, hi)
    }
}

pub(crate) fn resolve_axis_domain(axis: &AxisSpec, data_min: f64, data_max: f64) -> (f64, f64) {
    let mut unused = Vec::new();
    resolve_axis_domain_with_warnings(axis, data_min, data_max, &mut unused)
}

fn repair_prob_bound(
    val: f64,
    is_min: bool,
    is_explicit: bool,
    raw_fallback: f64,
    upper: f64,
    scale: AxisScale,
    warnings: &mut Vec<ChartWarning>,
) -> f64 {
    if val.is_finite() && scale.transform(val).is_some() {
        return val;
    }
    if is_explicit {
        warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired out-of-domain bound {val} for {scale:?} axis")));
    }
    if !is_explicit && raw_fallback.is_finite() && scale.transform(raw_fallback).is_some() {
        return if is_min { prob_auto_domain_min(raw_fallback, upper, scale) } else { prob_auto_domain_max(raw_fallback, upper, scale) };
    }
    let edge = 0.01 * (upper / 100.0);
    if is_min { edge } else { upper - edge }
}

pub(crate) fn resolve_axis_domain_with_warnings(axis: &AxisSpec, data_min: f64, data_max: f64, warnings: &mut Vec<ChartWarning>) -> (f64, f64) {
    let seg = axis.segments.first();
    let scale = seg.map_or(GraphAxisScale::Linear, |s| s.scale);
    let (raw_lo, raw_hi) = sanitize_range(data_min, data_max, (0.0, 1.0));

    let explicit_min = match seg.and_then(|s| s.min) {
        Some(v) if v.is_finite() => Some(v),
        Some(v) => {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("ignored non-finite minimum {v} for {scale:?} axis")));
            None
        }
        None => None,
    };
    let explicit_max = match seg.and_then(|s| s.max) {
        Some(v) if v.is_finite() => Some(v),
        Some(v) => {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("ignored non-finite maximum {v} for {scale:?} axis")));
            None
        }
        None => None,
    };

    let mut lo = explicit_min.unwrap_or(raw_lo);
    let mut hi = explicit_max.unwrap_or(raw_hi);
    if (hi - lo).abs() <= 1e-12 {
        if explicit_min.is_some() && explicit_max.is_some() {
            warnings.push(ChartWarning::new("invalid_axis_domain", format!("expanded degenerate explicit domain [{lo}, {hi}] for {scale:?} axis")));
        }
        (lo, hi) = sanitize_range(lo, hi, (0.0, 1.0));
    }

    match scale {
        AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln => {
            if lo <= 0.0 || hi <= 0.0 || scale.transform(lo).is_none() || scale.transform(hi).is_none() {
                if explicit_min.is_some() || explicit_max.is_some() {
                    warnings.push(ChartWarning::new("invalid_axis_domain", format!("repaired non-positive domain [{lo}, {hi}] for {scale:?} axis")));
                }
                if lo <= 0.0 {
                    lo = if raw_lo > 0.0 { raw_lo } else { 0.1 };
                }
                if hi <= 0.0 {
                    hi = if raw_hi > 0.0 { raw_hi } else { lo * 10.0 };
                }
                if (hi - lo).abs() <= 1e-12 {
                    hi = lo * 10.0;
                }
            }
        }
        AxisScale::Logit(domain) => {
            let upper = match domain {
                lumencraft_math::LogitDomain::Percent => 100.0,
                lumencraft_math::LogitDomain::Proportion => 1.0,
            };
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, upper, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, upper, scale, warnings);
            if (hi - lo).abs() <= 1e-12 {
                let edge = 0.01 * (upper / 100.0);
                lo = edge;
                hi = upper - edge;
            }
        }
        AxisScale::Probit => {
            lo = repair_prob_bound(lo, true, explicit_min.is_some(), raw_lo, 100.0, scale, warnings);
            hi = repair_prob_bound(hi, false, explicit_max.is_some(), raw_hi, 100.0, scale, warnings);
            if (hi - lo).abs() <= 1e-12 {
                lo = 0.01;
                hi = 99.99;
            }
        }
        AxisScale::Reciprocal => {
            if !lo.is_finite() || !hi.is_finite() || lo == 0.0 || hi == 0.0 || lo.signum() != hi.signum() || (hi - lo).abs() <= 1e-12 {
                if explicit_min.is_some() || explicit_max.is_some() || (lo < 0.0 && hi > 0.0) || (lo > 0.0 && hi < 0.0) {
                    warnings.push(ChartWarning::new(
                        "invalid_axis_domain",
                        format!("repaired zero-crossing or invalid domain [{lo}, {hi}] for Reciprocal axis"),
                    ));
                }
                if hi > 0.0 && lo <= 0.0 {
                    lo = if raw_lo > 0.0 && raw_lo < hi { raw_lo } else { (hi * 0.01).clamp(1e-6, hi * 0.5) };
                } else if lo > 0.0 && hi <= 0.0 {
                    hi = if raw_hi > 0.0 && raw_hi < lo { raw_hi } else { (lo * 0.01).clamp(1e-6, lo * 0.5) };
                } else {
                    lo = 1.0;
                    hi = 100.0;
                }
                if (hi - lo).abs() <= 1e-12 {
                    lo = 1.0;
                    hi = 100.0;
                }
            }
        }
        AxisScale::Linear => {}
    }
    (lo, hi)
}

pub(crate) fn nice_axis_ticks(axis: &AxisSpec, domain_min: f64, domain_max: f64, target_count: usize) -> Vec<(f64, String)> {
    let mut unused = Vec::new();
    nice_axis_ticks_with_warnings(axis, domain_min, domain_max, target_count, &mut unused)
}

pub(crate) fn nice_axis_ticks_with_warnings(
    axis: &AxisSpec,
    domain_min: f64,
    domain_max: f64,
    target_count: usize,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<(f64, String)> {
    let seg = axis.segments.first();
    let scale = seg.map_or(AxisScale::Linear, |s| s.scale);
    let fmt = axis.tick_format;
    let step_override = seg.and_then(|s| s.major_tick_interval).filter(|v| v.is_finite() && *v > 0.0);

    let vals = match scale {
        AxisScale::Log10 if step_override.is_none() => log10_ticks(domain_min, domain_max, warnings),
        AxisScale::Probit | AxisScale::Logit(_) if step_override.is_none() => generate_prob_ticks_for_range(domain_min, domain_max, scale),
        _ => linear_nice_ticks(domain_min, domain_max, step_override, target_count, warnings),
    };
    vals.into_iter().map(|v| (v, format_tick_value(v, scale, fmt))).collect()
}

fn log10_ticks(domain_min: f64, domain_max: f64, warnings: &mut Vec<ChartWarning>) -> Vec<f64> {
    let lo = domain_min.min(domain_max);
    let hi = domain_min.max(domain_max);
    if lo <= 0.0 || hi <= lo || !lo.is_finite() || !hi.is_finite() {
        return vec![0.1, 1.0, 10.0];
    }
    let e_min = lo.log10().floor() as i32;
    let e_max = hi.log10().ceil() as i32;
    let mut out = Vec::new();
    let span = (e_max - e_min).clamp(0, 12);
    let step = if span > 6 { 2 } else { 1 };
    let mut e = e_min;
    while e <= e_max {
        let v = 10.0_f64.powi(e);
        if v >= lo * 0.999 && v <= hi * 1.001 {
            out.push(v);
        }
        e += step;
    }
    if out.len() < 2 { linear_nice_ticks(lo, hi, None, 4, warnings) } else { out }
}

fn linear_nice_ticks(min_v: f64, max_v: f64, step_override: Option<f64>, target_count: usize, warnings: &mut Vec<ChartWarning>) -> Vec<f64> {
    let (lo, hi) = sanitize_range(min_v, max_v, (0.0, 1.0));
    compute_safe_linear_ticks(lo, hi, step_override, target_count.clamp(2, 12), warnings).0
}

pub(crate) fn format_tick_value(val: f64, scale: AxisScale, fmt: TickLabelFormat) -> String {
    if !val.is_finite() {
        return String::new();
    }
    let clean = if val.abs() < 1e-12 { 0.0 } else { val };
    let policy = LocaleNumberPolicy::default();
    match fmt {
        TickLabelFormat::ScientificE => format!("{clean:.2e}"),
        TickLabelFormat::PowerOfTenSuperscript if clean > 0.0 => {
            let exp = clean.log10();
            if (exp - exp.round()).abs() < 1e-4 { format!("10<sup>{}</sup>", exp.round() as i32) } else { format_axis_tick(clean, scale, &policy) }
        }
        TickLabelFormat::EngineeringSi => {
            let abs = clean.abs();
            if abs >= 1_000_000.0 {
                format!("{:.2}M", clean / 1_000_000.0)
            } else if abs >= 1_000.0 {
                format!("{:.2}k", clean / 1_000.0)
            } else {
                format_axis_tick(clean, scale, &policy)
            }
        }
        _ => format_axis_tick(clean, scale, &policy),
    }
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

    let top = if spec.title.trim().is_empty() { 20.0 } else { 34.0 };
    let right_legend: f64 = if cfg.has_colorbar {
        76.0
    } else if cfg.has_legend && spec.legend.visible && spec.legend.position_pt.is_none() {
        86.0
    } else {
        18.0
    };
    let right = right_legend.min(scene.width_pt * 0.34);

    let x_title_space = if !spec.x_axis.title.trim().is_empty() { 15.0 } else { 0.0 };
    let rotated_tick_extra = if spec.x_axis.tick_angle_deg.abs() > 10.0 { 16.0 } else { 0.0 };
    let bottom = (24.0 + x_title_space + rotated_tick_extra + cfg.extra_bottom_pt).min(scene.height_pt * 0.52);

    let w = (scene.width_pt - left - right).max(24.0);
    let h = (scene.height_pt - top - bottom).max(24.0);
    scene.plot_rect_pt = RectPt::new(left, top, w, h);
    max_y_w
}

/// Bundle for drawing axes cleanly within clippy argument count limits.
pub(crate) struct CartesianFrame<'a> {
    pub spec: &'a GraphSpec,
    pub x_ticks: &'a [(f64, String)],
    pub y_ticks: &'a [(f64, String)],
    pub x_map: AxisMapping,
    pub y_map: AxisMapping,
    pub max_y_label_w: f64,
    pub doc: &'a ProjectDocument,
}

pub(crate) fn draw_cartesian_frame(scene: &mut ChartScene, frame: &CartesianFrame<'_>) {
    let r = scene.plot_rect_pt;
    let axis_stroke = solid_stroke(Rgba8::BLACK, frame.spec.x_axis.stroke_width_pt.max(frame.spec.y_axis.stroke_width_pt), LineDashStyle::Solid);

    draw_axis_thresholds(scene, frame);

    match frame.spec.frame_preset {
        FramePreset::PlainBoxed => {
            scene.primitives.push(ScenePrimitive::Path { path: Path2D::rect(r), fill: None, stroke: Some(axis_stroke), clip: None });
        }
        FramePreset::OriginCrossing => {
            let y0 = crate::clamp_f64_safe(frame.y_map.map_value(0.0, r.bottom(), r.y).unwrap_or(r.bottom()), r.y, r.bottom());
            let x0 = crate::clamp_f64_safe(frame.x_map.map_value(0.0, r.x, r.right()).unwrap_or(r.x), r.x, r.right());
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x0, r.y), PointPt::new(x0, r.bottom())),
                fill: None,
                stroke: Some(axis_stroke),
                clip: None,
            });
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(r.x, y0), PointPt::new(r.right(), y0)),
                fill: None,
                stroke: Some(axis_stroke),
                clip: None,
            });
        }
        FramePreset::OpenL | FramePreset::OffsetRangeTufte | FramePreset::DualYSecondaryRight => {
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
    }

    draw_x_numeric_ticks(scene, frame);
    draw_y_numeric_ticks(scene, frame.spec, frame.y_ticks, frame.y_map, frame.max_y_label_w, frame.doc);
}

fn resolve_single_axis_additional(
    axis: &AxisSpec,
    map: AxisMapping,
    span_start: f64,
    span_end: f64,
    doc: &ProjectDocument,
    primary_id: &SheetId,
) -> Vec<ResolvedAdditionalTick> {
    if axis.additional_ticks.is_empty() {
        return Vec::new();
    }
    let policy = LocaleNumberPolicy::default();
    compute_additional_ticks_for_segments(
        &[(0, map, span_start, span_end)],
        &axis.additional_ticks,
        axis.tick_format,
        &policy,
        Some(doc),
        Some(primary_id),
    )
}

fn draw_axis_thresholds(scene: &mut ChartScene, frame: &CartesianFrame<'_>) {
    let r = scene.plot_rect_pt;
    let grid_color = AxisRenderOptions::default().grid_color;
    let x_add = resolve_single_axis_additional(&frame.spec.x_axis, frame.x_map, r.x, r.right(), frame.doc, &frame.spec.primary_source_sheet_id);
    let y_add = resolve_single_axis_additional(&frame.spec.y_axis, frame.y_map, r.bottom(), r.y, frame.doc, &frame.spec.primary_source_sheet_id);
    emit_additional_tick_gridline_segments(&x_add, true, &[(r.bottom(), r.y)], grid_color, &mut scene.primitives);
    emit_additional_tick_gridline_segments(&y_add, false, &[(r.x, r.right())], grid_color, &mut scene.primitives);
    for thr in &frame.spec.x_axis.thresholds {
        if let Some(x_pt) = frame.x_map.map_value(thr.value, r.x, r.right()) {
            emit_vertical_threshold(scene, r, x_pt, thr, frame.x_map);
        }
    }
    for thr in &frame.spec.y_axis.thresholds {
        if let Some(y_pt) = frame.y_map.map_value(thr.value, r.bottom(), r.y) {
            emit_horizontal_threshold(scene, r, y_pt, thr, frame.y_map);
        }
    }
}

fn emit_vertical_threshold(scene: &mut ChartScene, r: RectPt, x_pt: f64, thr: &AxisThreshold, map: AxisMapping) {
    if let Some(end_v) = thr.shade_to
        && let Some(x_end) = map.map_value(end_v, r.x, r.right())
    {
        let lo = crate::clamp_f64_safe(x_pt.min(x_end), r.x, r.right());
        let hi = crate::clamp_f64_safe(x_pt.max(x_end), r.x, r.right());
        if hi > lo {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(lo, r.y, hi - lo, r.height)),
                fill: Some(alpha_fill(thr.color, 28)),
                stroke: None,
                clip: Some(r),
            });
        }
    }
    if x_pt >= r.x - 0.5 && x_pt <= r.right() + 0.5 {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x_pt, r.y), PointPt::new(x_pt, r.bottom())),
            fill: None,
            stroke: Some(solid_stroke(thr.color, thr.stroke_width_pt, thr.dash_style)),
            clip: Some(r),
        });
    }
}

fn emit_horizontal_threshold(scene: &mut ChartScene, r: RectPt, y_pt: f64, thr: &AxisThreshold, map: AxisMapping) {
    if let Some(end_v) = thr.shade_to
        && let Some(y_end) = map.map_value(end_v, r.bottom(), r.y)
    {
        let lo = crate::clamp_f64_safe(y_pt.min(y_end), r.y, r.bottom());
        let hi = crate::clamp_f64_safe(y_pt.max(y_end), r.y, r.bottom());
        if hi > lo {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(r.x, lo, r.width, hi - lo)),
                fill: Some(alpha_fill(thr.color, 28)),
                stroke: None,
                clip: Some(r),
            });
        }
    }
    if y_pt >= r.y - 0.5 && y_pt <= r.bottom() + 0.5 {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, y_pt), PointPt::new(r.right(), y_pt)),
            fill: None,
            stroke: Some(solid_stroke(thr.color, thr.stroke_width_pt, thr.dash_style)),
            clip: Some(r),
        });
    }
}

fn draw_x_numeric_ticks(scene: &mut ChartScene, frame: &CartesianFrame<'_>) {
    let r = scene.plot_rect_pt;
    let tick_stroke = solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid);
    let angle = frame.spec.x_axis.tick_angle_deg;
    for (val, label) in frame.x_ticks {
        let Some(x_pt) = frame.x_map.map_value(*val, r.x, r.right()) else { continue };
        if x_pt < r.x - 1.0 || x_pt > r.right() + 1.0 {
            continue;
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x_pt, r.bottom()), PointPt::new(x_pt, r.bottom() + 4.0)),
            fill: None,
            stroke: Some(tick_stroke),
            clip: None,
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(label),
            anchor_pt: PointPt::new(x_pt, r.bottom() + 13.0),
            anchor: if angle.abs() > 10.0 { TextAnchor::End } else { TextAnchor::Middle },
            font_size_pt: 8.5,
            rotation_deg: angle,
            color: Rgba8::BLACK,
        }));
    }
    let x_add = resolve_single_axis_additional(&frame.spec.x_axis, frame.x_map, r.x, r.right(), frame.doc, &frame.spec.primary_source_sheet_id);
    if !x_add.is_empty() {
        emit_additional_x_tick_primitives(&x_add, r.bottom(), angle, tick_stroke, &mut scene.primitives);
    }
    emit_x_title(scene, frame.spec, frame.doc, angle.abs() > 10.0);
}

pub(crate) fn draw_categorical_x_ticks(scene: &mut ChartScene, spec: &GraphSpec, cats: &[(f64, String)], doc: &ProjectDocument) {
    let r = scene.plot_rect_pt;
    let tick_stroke = solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid);
    let angle = spec.x_axis.tick_angle_deg;
    for (x_pt, label) in cats {
        if *x_pt < r.x - 1.0 || *x_pt > r.right() + 1.0 {
            continue;
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(*x_pt, r.bottom()), PointPt::new(*x_pt, r.bottom() + 4.0)),
            fill: None,
            stroke: Some(tick_stroke),
            clip: None,
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(label),
            anchor_pt: PointPt::new(*x_pt, r.bottom() + 13.0),
            anchor: if angle.abs() > 10.0 { TextAnchor::End } else { TextAnchor::Middle },
            font_size_pt: 8.5,
            rotation_deg: angle,
            color: Rgba8::BLACK,
        }));
    }
    emit_x_title(scene, spec, doc, angle.abs() > 10.0);
}

fn emit_x_title(scene: &mut ChartScene, spec: &GraphSpec, doc: &ProjectDocument, rotated_ticks: bool) {
    let title = resolve_graph_text(&spec.x_axis.title, doc);
    if title.trim().is_empty() {
        return;
    }
    let r = scene.plot_rect_pt;
    let y = if rotated_ticks { r.bottom() + 36.0 } else { r.bottom() + 26.0 };
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&title),
        anchor_pt: PointPt::new(r.x + r.width * 0.5, y),
        anchor: TextAnchor::Middle,
        font_size_pt: 9.5,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
}

pub(crate) fn draw_y_numeric_ticks(
    scene: &mut ChartScene,
    spec: &GraphSpec,
    y_ticks: &[(f64, String)],
    y_map: AxisMapping,
    max_y_label_w: f64,
    doc: &ProjectDocument,
) {
    let r = scene.plot_rect_pt;
    let tick_stroke = solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid);
    for (val, label) in y_ticks {
        let Some(y_pt) = y_map.map_value(*val, r.bottom(), r.y) else { continue };
        if y_pt < r.y - 1.0 || y_pt > r.bottom() + 1.0 {
            continue;
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x - 4.0, y_pt), PointPt::new(r.x, y_pt)),
            fill: None,
            stroke: Some(tick_stroke),
            clip: None,
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(label),
            anchor_pt: PointPt::new(r.x - 6.0, y_pt + 3.0),
            anchor: TextAnchor::End,
            font_size_pt: 8.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
    let y_add = resolve_single_axis_additional(&spec.y_axis, y_map, r.bottom(), r.y, doc, &spec.primary_source_sheet_id);
    if !y_add.is_empty() {
        emit_additional_y_tick_primitives(&y_add, r.x, true, 0.0, 6.0, tick_stroke, &mut scene.primitives);
    }
    emit_y_title(scene, spec, max_y_label_w, doc);
}

pub(crate) fn emit_y_title(scene: &mut ChartScene, spec: &GraphSpec, max_y_label_w: f64, doc: &ProjectDocument) {
    let y_title = resolve_graph_text(&spec.y_axis.title, doc);
    if y_title.trim().is_empty() {
        return;
    }
    let r = scene.plot_rect_pt;
    let max_x = (r.x - 8.0).max(0.0);
    let min_x = 11.0_f64.min(max_x);
    let x = crate::clamp_f64_safe(r.x - max_y_label_w - 14.0, min_x, max_x);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&y_title),
        anchor_pt: PointPt::new(x, r.y + r.height * 0.5),
        anchor: TextAnchor::Middle,
        font_size_pt: 9.5,
        rotation_deg: -90.0,
        color: Rgba8::BLACK,
    }));
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

pub(crate) fn circle_path(cx: f64, cy: f64, r: f64) -> Path2D {
    ellipse_path(cx, cy, r, r, 0.0)
}

pub(crate) fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64, angle_rad: f64) -> Path2D {
    let (sin_a, cos_a) = angle_rad.sin_cos();
    let xf = |lx: f64, ly: f64| PointPt::new(cx + lx * cos_a - ly * sin_a, cy + lx * sin_a + ly * cos_a);
    let kx = rx * CIRCLE_KAPPA;
    let ky = ry * CIRCLE_KAPPA;

    let mut path = Path2D::new();
    path_move(&mut path, xf(rx, 0.0));
    path_cubic(&mut path, xf(rx, ky), xf(kx, ry), xf(0.0, ry));
    path_cubic(&mut path, xf(-kx, ry), xf(-rx, ky), xf(-rx, 0.0));
    path_cubic(&mut path, xf(-rx, -ky), xf(-kx, -ry), xf(0.0, -ry));
    path_cubic(&mut path, xf(kx, -ry), xf(rx, -ky), xf(rx, 0.0));
    path_close(&mut path);
    path
}

pub(crate) fn marker_path(shape: MarkerShape, cx: f64, cy: f64, radius: f64) -> Path2D {
    let r = radius.max(0.75);
    match shape {
        MarkerShape::Circle => circle_path(cx, cy, r),
        MarkerShape::Square => Path2D::rect(RectPt::new(cx - r, cy - r, 2.0 * r, 2.0 * r)),
        MarkerShape::TriangleUp => polygon_path(&[(cx, cy - r * 1.15), (cx + r, cy + r * 0.85), (cx - r, cy + r * 0.85)]),
        MarkerShape::TriangleDown => polygon_path(&[(cx - r, cy - r * 0.85), (cx + r, cy - r * 0.85), (cx, cy + r * 1.15)]),
        MarkerShape::Diamond => polygon_path(&[(cx, cy - r * 1.15), (cx + r * 1.05, cy), (cx, cy + r * 1.15), (cx - r * 1.05, cy)]),
        MarkerShape::Hexagon => regular_polygon_path(cx, cy, r, 6, 0.0),
        MarkerShape::Star => star_path(cx, cy, r * 1.15, r * 0.48, 5),
        MarkerShape::Plus => {
            let mut p = Path2D::new();
            path_move(&mut p, PointPt::new(cx - r, cy));
            path_line(&mut p, PointPt::new(cx + r, cy));
            path_move(&mut p, PointPt::new(cx, cy - r));
            path_line(&mut p, PointPt::new(cx, cy + r));
            p
        }
        MarkerShape::Cross => {
            let d = r * 0.78;
            let mut p = Path2D::new();
            path_move(&mut p, PointPt::new(cx - d, cy - d));
            path_line(&mut p, PointPt::new(cx + d, cy + d));
            path_move(&mut p, PointPt::new(cx - d, cy + d));
            path_line(&mut p, PointPt::new(cx + d, cy - d));
            p
        }
        MarkerShape::HorizontalBar => Path2D::line(PointPt::new(cx - r * 1.2, cy), PointPt::new(cx + r * 1.2, cy)),
        MarkerShape::VerticalBar => Path2D::line(PointPt::new(cx, cy - r * 1.2), PointPt::new(cx, cy + r * 1.2)),
    }
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

fn regular_polygon_path(cx: f64, cy: f64, r: f64, sides: usize, offset_rad: f64) -> Path2D {
    let n = sides.max(3);
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let a = offset_rad + (i as f64) * (2.0 * PI / (n as f64));
        pts.push((cx + r * a.cos(), cy + r * a.sin()));
    }
    polygon_path(&pts)
}

fn star_path(cx: f64, cy: f64, r_outer: f64, r_inner: f64, tips: usize) -> Path2D {
    let count = tips.max(3) * 2;
    let mut pts = Vec::with_capacity(count);
    for i in 0..count {
        let a = -PI * 0.5 + (i as f64) * (PI / (tips as f64));
        let r = if i % 2 == 0 { r_outer } else { r_inner };
        pts.push((cx + r * a.cos(), cy + r * a.sin()));
    }
    polygon_path(&pts)
}

pub(crate) fn emit_marker(scene: &mut ChartScene, center: PointPt, style: MarkerStyle, clip: Option<RectPt>) {
    let radius = (style.size_pt * 0.5).max(1.0);
    let path = marker_path(style.shape, center.x, center.y, radius);
    let stroke_only_shape = matches!(style.shape, MarkerShape::Plus | MarkerShape::Cross | MarkerShape::HorizontalBar | MarkerShape::VerticalBar);
    let stroke = Some(solid_stroke(style.stroke_color, style.stroke_width_pt.max(0.75), LineDashStyle::Solid));

    if stroke_only_shape {
        scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke, clip });
        return;
    }

    match style.fill_mode {
        MarkerFillMode::Filled => {
            scene.primitives.push(ScenePrimitive::Path { path, fill: Some(FillPaint::solid(style.fill_color)), stroke, clip });
        }
        MarkerFillMode::Open => {
            scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke, clip });
        }
        MarkerFillMode::SemiFilledTop | MarkerFillMode::SemiFilledBottom | MarkerFillMode::SemiFilledLeft | MarkerFillMode::SemiFilledRight => {
            let half = match style.fill_mode {
                MarkerFillMode::SemiFilledTop => RectPt::new(center.x - radius - 1.0, center.y - radius - 1.0, 2.0 * (radius + 1.0), radius + 1.0),
                MarkerFillMode::SemiFilledBottom => RectPt::new(center.x - radius - 1.0, center.y, 2.0 * (radius + 1.0), radius + 1.0),
                MarkerFillMode::SemiFilledLeft => RectPt::new(center.x - radius - 1.0, center.y - radius - 1.0, radius + 1.0, 2.0 * (radius + 1.0)),
                _ => RectPt::new(center.x, center.y - radius - 1.0, radius + 1.0, 2.0 * (radius + 1.0)),
            };
            scene.primitives.push(ScenePrimitive::Path {
                path: path.clone(),
                fill: Some(FillPaint::solid(style.fill_color)),
                stroke: None,
                clip: Some(half),
            });
            scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke, clip });
        }
    }
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
    let r = scene.plot_rect_pt;
    let max_label_w = entries.iter().map(|e| text_metrics(&e.label, 8.0).width_pt).fold(0.0_f64, f64::max);
    let box_w = crate::clamp_f64_safe(max_label_w + 26.0, 54.0, (scene.width_pt - r.right() - 8.0).max(56.0));
    let row_h = 13.0;
    let box_h = (entries.len() as f64) * row_h + 8.0;
    let (lx, ly) = if let Some((px, py)) = spec.legend.position_pt {
        (px, py)
    } else if scene.width_pt - r.right() >= box_w + 10.0 {
        (r.right() + 8.0, r.y)
    } else {
        ((r.right() - box_w - 4.0).max(r.x + 2.0), r.y + 4.0)
    };

    if spec.legend.bordered {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(lx, ly, box_w, box_h)),
            fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 235))),
            stroke: Some(solid_stroke(Rgba8::rgb(120, 120, 120), 0.75, LineDashStyle::Solid)),
            clip: None,
        });
    }

    for (idx, entry) in entries.iter().enumerate() {
        let cy = ly + 10.0 + (idx as f64) * row_h;
        if entry.is_line {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(lx + 4.0, cy), PointPt::new(lx + 16.0, cy)),
                fill: None,
                stroke: Some(solid_stroke(entry.color, 1.5, entry.dash.unwrap_or(LineDashStyle::Solid))),
                clip: None,
            });
        } else if let Some(mut marker) = entry.marker {
            marker.size_pt = marker.size_pt.min(6.0);
            emit_marker(scene, PointPt::new(lx + 10.0, cy), marker, None);
        } else {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(lx + 5.0, cy - 4.0, 9.0, 8.0)),
                fill: Some(FillPaint::solid(entry.color)),
                stroke: Some(solid_stroke(Rgba8::rgb(40, 40, 40), 0.5, LineDashStyle::Solid)),
                clip: None,
            });
        }
        let resolved = resolve_graph_text(&entry.label, doc);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&resolved),
            anchor_pt: PointPt::new(lx + 20.0, cy + 3.0),
            anchor: TextAnchor::Start,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
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

pub(crate) fn draw_graph_annotations(scene: &mut ChartScene, annotations: &[GraphAnnotation], doc: &ProjectDocument) {
    for ann in annotations {
        match ann {
            GraphAnnotation::TextBox { bounds_pt, markdown, bordered } => {
                let rect = RectPt::from(*bounds_pt);
                if *bordered {
                    scene.primitives.push(ScenePrimitive::Path {
                        path: Path2D::rect(rect),
                        fill: Some(FillPaint::solid(Rgba8::rgba(255, 255, 255, 230))),
                        stroke: Some(solid_stroke(Rgba8::BLACK, 0.75, LineDashStyle::Solid)),
                        clip: None,
                    });
                }
                let expanded = resolve_graph_text(markdown, doc);
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(&expanded),
                    anchor_pt: PointPt::new(rect.x + 4.0, rect.y + 11.0),
                    anchor: TextAnchor::Start,
                    font_size_pt: 8.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            GraphAnnotation::LineOrArrow { start_pt, end_pt, arrowhead, dash, color } => {
                let p0 = PointPt::new(start_pt.0, start_pt.1);
                let p1 = PointPt::new(end_pt.0, end_pt.1);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(p0, p1),
                    fill: None,
                    stroke: Some(solid_stroke(*color, 1.2, *dash)),
                    clip: None,
                });
                if *arrowhead {
                    let dx = p1.x - p0.x;
                    let dy = p1.y - p0.y;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len > 1e-4 {
                        let ux = dx / len;
                        let uy = dy / len;
                        let back = PointPt::new(p1.x - 6.0 * ux, p1.y - 6.0 * uy);
                        let left = PointPt::new(back.x - 3.0 * uy, back.y + 3.0 * ux);
                        let right = PointPt::new(back.x + 3.0 * uy, back.y - 3.0 * ux);
                        scene.primitives.push(ScenePrimitive::Path {
                            path: polygon_path(&[(p1.x, p1.y), (left.x, left.y), (right.x, right.y)]),
                            fill: Some(FillPaint::solid(*color)),
                            stroke: None,
                            clip: None,
                        });
                    }
                }
            }
            GraphAnnotation::Shape { kind, bounds_pt, stroke_color, stroke_width_pt, dash, fill_color } => {
                let rect = RectPt::from(*bounds_pt);
                let path = match kind {
                    lumencraft_doc::AnnotationShapeKind::Rectangle | lumencraft_doc::AnnotationShapeKind::RoundedRectangle => Path2D::rect(rect),
                    lumencraft_doc::AnnotationShapeKind::Ellipse => {
                        ellipse_path(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5, rect.width * 0.5, rect.height * 0.5, 0.0)
                    }
                };
                scene.primitives.push(ScenePrimitive::Path {
                    path,
                    fill: fill_color.map(FillPaint::solid),
                    stroke: Some(solid_stroke(*stroke_color, *stroke_width_pt, *dash)),
                    clip: None,
                });
            }
            GraphAnnotation::EmbeddedResultTable { result_sheet_id, tab_id, bounds_pt } => {
                if let Some(grid) = extract_sheet_grid(doc, result_sheet_id, tab_id.as_deref()) {
                    render_data_grid_table(scene, RectPt::from(*bounds_pt), &grid.headers, &grid.row_labels, &grid.rows, Rgba8::rgb(120, 120, 120));
                }
            }
            GraphAnnotation::ModelEquation { result_sheet_id, data_set_index, mode, bounds_pt, font_size_pt, bordered } => {
                crate::equation_box::render_model_equation_annotation(
                    scene,
                    doc,
                    &crate::equation_box::ModelEquationParams {
                        result_sheet_id,
                        data_set_index: *data_set_index,
                        mode: *mode,
                        bounds_pt: *bounds_pt,
                        font_size_pt: *font_size_pt,
                        bordered: *bordered,
                    },
                );
            }
        }
    }
}

pub(crate) fn format_result_cell(cell: &ResultCell) -> String {
    match cell {
        ResultCell::Empty => String::new(),
        ResultCell::Number { value, prefix, units } => {
            if !value.is_finite() {
                return String::from("NaN");
            }
            let mut s = format!("{value:.4}");
            trim_trailing_zeros(&mut s);
            let mut out = String::new();
            if let Some(p) = prefix {
                out.push_str(p);
            }
            out.push_str(&s);
            if let Some(u) = units {
                out.push(' ');
                out.push_str(u);
            }
            out
        }
        ResultCell::ConfidenceInterval { lower, upper } => {
            let fmt_b = |b: &lumencraft_doc::CiBound| match b {
                lumencraft_doc::CiBound::Finite(v) if v.is_finite() => {
                    let mut s = format!("{v:.3}");
                    trim_trailing_zeros(&mut s);
                    s
                }
                lumencraft_doc::CiBound::Open => String::from("+∞"),
                lumencraft_doc::CiBound::Finite(_) => String::from("NaN"),
            };
            format!("[{}, {}]", fmt_b(lower), fmt_b(upper))
        }
        ResultCell::PValue { formatted, .. } | ResultCell::Text(formatted) => formatted.clone(),
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
        let tab: Option<&ResultTab> = tab_id.and_then(|tid| res.tabs.iter().find(|t| t.id == tid)).or_else(|| res.tabs.first());
        if let Some(t) = tab {
            let rows = t.rows.iter().map(|r| r.iter().map(format_result_cell).collect()).collect();
            return Some(SheetGrid { headers: t.column_headers.clone(), row_labels: t.row_labels.clone(), rows });
        }
        if !res.scalars.is_empty() {
            let headers = vec![String::from("Value")];
            let row_labels = res.scalars.iter().map(|s| s.key.clone()).collect();
            let rows = res.scalars.iter().map(|s| vec![format!("{:.4}", s.value)]).collect();
            return Some(SheetGrid { headers, row_labels, rows });
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

fn extract_data_sheet_grid(ds: &DataSheet) -> SheetGrid {
    let fmt_cell = |c: &lumencraft_doc::Cell| -> String {
        match &c.value {
            lumencraft_doc::CellValue::Empty => String::new(),
            lumencraft_doc::CellValue::Number(v) if v.is_finite() => {
                let mut s = format!("{v:.4}");
                trim_trailing_zeros(&mut s);
                s
            }
            lumencraft_doc::CellValue::Number(_) => String::from("NaN"),
            lumencraft_doc::CellValue::Text(t) => t.clone(),
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

/// Render a clipped crisp data/result table grid into `scene`.
pub(crate) fn render_data_grid_table(
    scene: &mut ChartScene,
    bounds: RectPt,
    headers: &[String],
    row_labels: &[String],
    rows: &[Vec<String>],
    border_color: Rgba8,
) {
    if bounds.width <= 4.0 || bounds.height <= 4.0 {
        return;
    }
    let has_row_labels = row_labels.iter().any(|l| !l.trim().is_empty());
    let total_cols = (headers.len() + usize::from(has_row_labels)).max(1);
    let col_w = bounds.width / (total_cols as f64);
    let row_h = 14.0_f64;
    let max_rows = ((bounds.height / row_h).floor() as usize).saturating_sub(1).min(rows.len());

    scene.primitives.push(ScenePrimitive::PushClip { rect: bounds });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(RectPt::new(bounds.x, bounds.y, bounds.width, row_h)),
        fill: Some(FillPaint::solid(Rgba8::rgb(238, 241, 246))),
        stroke: None,
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(bounds),
        fill: None,
        stroke: Some(solid_stroke(border_color, 0.75, LineDashStyle::Solid)),
        clip: None,
    });

    for c in 0..headers.len() {
        let x = bounds.x + ((c + usize::from(has_row_labels)) as f64) * col_w + 4.0;
        let hdr = headers.get(c).map(String::as_str).unwrap_or("");
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>{hdr}</b>")),
            anchor_pt: PointPt::new(x, bounds.y + 10.5),
            anchor: TextAnchor::Start,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }

    for r in 0..max_rows {
        let y_top = bounds.y + ((r + 1) as f64) * row_h;
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(bounds.x, y_top), PointPt::new(bounds.right(), y_top)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(215, 220, 226), 0.5, LineDashStyle::Solid)),
            clip: None,
        });
        if has_row_labels {
            let lbl = row_labels.get(r).map(String::as_str).unwrap_or("");
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: parse_markup(lbl),
                anchor_pt: PointPt::new(bounds.x + 4.0, y_top + 10.0),
                anchor: TextAnchor::Start,
                font_size_pt: 7.5,
                rotation_deg: 0.0,
                color: Rgba8::BLACK,
            }));
        }
        if let Some(row_cells) = rows.get(r) {
            for (c, cell_txt) in row_cells.iter().enumerate() {
                let x = bounds.x + ((c + usize::from(has_row_labels)) as f64) * col_w + 4.0;
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(cell_txt),
                    anchor_pt: PointPt::new(x, y_top + 10.0),
                    anchor: TextAnchor::Start,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::rgb(30, 30, 30),
                }));
            }
        }
    }
    scene.primitives.push(ScenePrimitive::PopClip);
}

// ============================================================================
// 1. Kaplan-Meier Survival Graph (Step Curve + CI Ribbon + Censors + At-Risk)
// ============================================================================

struct KmCurveData {
    sheet_id: Option<SheetId>,
    col_index: usize,
    label: String,
    color: Rgba8,
    stroke_width_pt: f64,
    show_censored_ticks: bool,
    show_greenwood_ci: bool,
    ci_dashed: bool,
    n_total: usize,
    steps: Vec<KmPointStep>,
    censored_times: Vec<(f64, Option<usize>, usize)>,
    observations: Vec<SurvivalObservation>,
}

#[derive(Debug, Clone, Copy)]
struct KmPointStep {
    time: f64,
    survival: f64,
    ci_lower: f64,
    ci_upper: f64,
    n_at_risk_before: usize,
}

fn compile_survival(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let cohorts = extract_km_cohorts(graph, doc);
    if cohorts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_survival_data", "no survival cohort observations or result curves available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let scale_pct = survival_y_scale_factor(&graph.spec);
    let max_time = cohorts.iter().flat_map(|c| c.steps.iter().map(|s| s.time)).fold(1.0_f64, f64::max).max(1.0);
    let (x_lo, x_hi) = resolve_axis_domain_with_warnings(&graph.spec.x_axis, 0.0, max_time * 1.05, &mut scene.warnings);
    let (y_lo, y_hi) = resolve_axis_domain_with_warnings(&graph.spec.y_axis, 0.0, scale_pct, &mut scene.warnings);

    let x_ticks = nice_axis_ticks_with_warnings(&graph.spec.x_axis, x_lo, x_hi, 6, &mut scene.warnings);
    let y_ticks = nice_axis_ticks_with_warnings(&graph.spec.y_axis, y_lo, y_hi, 5, &mut scene.warnings);
    let show_nar = graph.spec.survival_number_at_risk && cohorts.iter().any(|c| c.n_total > 0 || !c.observations.is_empty());
    let extra_bottom = if show_nar { 20.0 + (cohorts.len() as f64) * 13.0 } else { 0.0 };
    let max_cohort_lbl_w = if show_nar { cohorts.iter().map(|c| text_metrics(&c.label, 8.0).width_pt + 10.0).fold(0.0_f64, f64::max) } else { 0.0 };

    let max_y_w = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &y_ticks,
        PlotMarginConfig {
            extra_bottom_pt: extra_bottom,
            extra_left_pt: max_cohort_lbl_w.saturating_sub_nonneg(36.0),
            has_legend: cohorts.len() > 1,
            has_colorbar: false,
            suppress_y_axis_labels: false,
        },
    );

    let x_map = AxisMapping { min: x_lo, max: x_hi, scale: graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: graph.spec.y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };

    let frame = CartesianFrame { spec: &graph.spec, x_ticks: &x_ticks, y_ticks: &y_ticks, x_map, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);

    let r = scene.plot_rect_pt;
    let mut legend_entries = Vec::with_capacity(cohorts.len());
    for cohort in &cohorts {
        draw_single_km_cohort(scene, cohort, x_map, y_map, x_lo.max(x_hi), scale_pct);
        legend_entries.push(LegendEntry {
            label: cohort.label.clone(),
            color: cohort.color,
            marker: None,
            dash: Some(LineDashStyle::Solid),
            is_line: true,
        });
    }

    if show_nar {
        draw_number_at_risk_table(scene, r, &cohorts, &x_ticks, x_map);
    }
    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

trait NonNegSub {
    fn saturating_sub_nonneg(self, rhs: f64) -> f64;
}
impl NonNegSub for f64 {
    fn saturating_sub_nonneg(self, rhs: f64) -> f64 {
        (self - rhs).max(0.0)
    }
}

fn survival_y_scale_factor(spec: &GraphSpec) -> f64 {
    let seg_max = spec.y_axis.segments.first().and_then(|s| s.max);
    let title_low = spec.y_axis.title.to_ascii_lowercase();
    if seg_max.is_some_and(|m| m > 1.5) || title_low.contains('%') || title_low.contains("percent") { 100.0 } else { 1.0 }
}

fn extract_km_cohorts(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<KmCurveData> {
    let primary = &graph.spec.primary_source_sheet_id;
    let km_result = doc.result_sheet(primary).or_else(|| {
        doc.result_sheets
            .iter()
            .find(|r| {
                r.analysis.source_sheet_id == *primary && matches!(r.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier { .. }))
            })
            .or_else(|| {
                doc.result_sheets.iter().find(|r| {
                    r.analysis.source_sheet_id == *primary
                        && matches!(r.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. }))
                        && !r.fitted_curves.is_empty()
                })
            })
    });

    if let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id)
        && let TableArchetype::Survival(surv) = &ds.archetype
    {
        return build_km_from_survival_table(graph, &ds.id, surv, km_result);
    }

    if let Some(res) = km_result {
        return build_km_from_result_sheet(graph, res);
    }
    Vec::new()
}

fn km_series_style(graph: &GraphSheet, data_set_idx: usize, fallback_idx: usize) -> (Rgba8, f64, bool, bool, bool, Option<String>) {
    let default_col = graph.spec.palette.color_at(fallback_idx);
    let series = graph.spec.series.iter().find(|s| s.data_set_index == data_set_idx).or_else(|| graph.spec.series.get(fallback_idx));
    if let Some(s) = series {
        if let GraphicalDataObject::KaplanMeierStep { line_color, stroke_width_pt, show_censored_ticks, show_greenwood_ci, ci_dashed } = s.gdo {
            return (line_color, stroke_width_pt, show_censored_ticks, show_greenwood_ci, ci_dashed, s.legend_label.clone());
        }
        return (default_col, 1.8, true, true, false, s.legend_label.clone());
    }
    (default_col, 1.8, true, true, false, None)
}

fn build_km_from_survival_table(
    graph: &GraphSheet,
    sheet_id: &SheetId,
    surv: &lumencraft_doc::SurvivalTable,
    km_result: Option<&ResultSheet>,
) -> Vec<KmCurveData> {
    let indices: Vec<usize> = if graph.spec.series.is_empty() {
        (0..surv.cohorts.len()).collect()
    } else {
        graph.spec.series.iter().filter(|s| s.data_set_index < surv.cohorts.len()).map(|s| s.data_set_index).collect()
    };

    let mut out = Vec::with_capacity(indices.len());
    for (pos, &c_idx) in indices.iter().enumerate() {
        let Some(cohort_col) = surv.cohorts.get(c_idx) else { continue };
        let Ok(obs) = surv.cohort_observations(c_idx) else { continue };
        if obs.is_empty() {
            continue;
        }
        let (color, stroke_w, show_censor, show_ci, ci_dashed, label_override) = km_series_style(graph, c_idx, pos);
        let label = label_override.unwrap_or_else(|| cohort_col.title.clone());
        let censored_times: Vec<(f64, Option<usize>, usize)> =
            obs.iter().filter(|o| o.status == SurvivalStatus::Censored).map(|o| (o.time, Some(o.row), c_idx)).collect();

        if let Some(res) = km_result
            && let Some(curve) = res.fitted_curves.iter().find(|fc| fc.data_set_index == c_idx)
            && !curve.points.is_empty()
        {
            let steps: Vec<KmPointStep> = curve
                .points
                .iter()
                .map(|p| KmPointStep {
                    time: p.x,
                    survival: p.y,
                    ci_lower: p.ci_lower.unwrap_or(p.y),
                    ci_upper: p.ci_upper.unwrap_or(p.y),
                    n_at_risk_before: obs.iter().filter(|o| o.time >= p.x).count(),
                })
                .collect();
            out.push(KmCurveData {
                sheet_id: Some(sheet_id.clone()),
                col_index: c_idx,
                label,
                color,
                stroke_width_pt: stroke_w,
                show_censored_ticks: show_censor,
                show_greenwood_ci: show_ci,
                ci_dashed,
                n_total: obs.len(),
                steps,
                censored_times,
                observations: obs,
            });
            continue;
        }

        if let Ok(km) = kaplan_meier_curve(&label, &obs, false, ConfidenceLevel::DEFAULT_95) {
            let mut steps = Vec::with_capacity(km.steps.len() + 1);
            steps.push(KmPointStep { time: 0.0, survival: 1.0, ci_lower: 1.0, ci_upper: 1.0, n_at_risk_before: km.n_total });
            for s in &km.steps {
                steps.push(KmPointStep {
                    time: s.time,
                    survival: s.survival,
                    ci_lower: s.ci_lower,
                    ci_upper: s.ci_upper,
                    n_at_risk_before: s.n_at_risk,
                });
            }
            out.push(KmCurveData {
                sheet_id: Some(sheet_id.clone()),
                col_index: c_idx,
                label,
                color,
                stroke_width_pt: stroke_w,
                show_censored_ticks: show_censor,
                show_greenwood_ci: show_ci,
                ci_dashed,
                n_total: km.n_total,
                steps,
                censored_times,
                observations: obs,
            });
        }
    }
    out
}

fn result_curve_label(res: &ResultSheet, fc: &lumencraft_doc::FittedCurveOverlay, idx: usize) -> String {
    match fc.equation_id.as_str() {
        "cox_baseline" => String::from("Baseline"),
        "cox_covariate" => {
            let param_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.id == "cox-ph");
            fc.data_set_index
                .checked_sub(1)
                .and_then(|cov_idx| param_tab.and_then(|t| t.row_labels.get(cov_idx)))
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Covariate {}", fc.data_set_index))
        }
        _ => res
            .tabs
            .iter()
            .find(|t| t.kind == ResultTabKind::MainSummary)
            .and_then(|t| t.row_labels.get(idx))
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| format!("Cohort {}", fc.data_set_index + 1)),
    }
}

fn build_km_from_result_sheet(graph: &GraphSheet, res: &ResultSheet) -> Vec<KmCurveData> {
    let nar_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::NumberAtRisk);
    let mut out = Vec::with_capacity(res.fitted_curves.len());
    for (idx, fc) in res.fitted_curves.iter().enumerate() {
        if fc.points.is_empty() {
            continue;
        }
        let (color, stroke_w, show_censor, show_ci, ci_dashed, label_override) = km_series_style(graph, fc.data_set_index, idx);
        let label = label_override.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| result_curve_label(res, fc, idx));
        let has_ci = fc.points.iter().any(|p| p.ci_lower.is_some() && p.ci_upper.is_some());
        let steps: Vec<KmPointStep> = fc
            .points
            .iter()
            .map(|p| KmPointStep {
                time: p.x,
                survival: p.y,
                ci_lower: p.ci_lower.unwrap_or(p.y),
                ci_upper: p.ci_upper.unwrap_or(p.y),
                n_at_risk_before: nar_at_time_from_tab(nar_tab, &label, p.x).unwrap_or(0),
            })
            .collect();
        let n_total = steps.first().map_or(0, |s| s.n_at_risk_before);
        let mut censored_times = Vec::new();
        if let Some(tab) = nar_tab {
            extract_censored_from_nar_tab(tab, &label, fc.data_set_index, &mut censored_times);
        }
        out.push(KmCurveData {
            sheet_id: None,
            col_index: fc.data_set_index,
            label,
            color,
            stroke_width_pt: stroke_w,
            show_censored_ticks: show_censor,
            show_greenwood_ci: show_ci && has_ci,
            ci_dashed,
            n_total,
            steps,
            censored_times,
            observations: Vec::new(),
        });
    }
    out
}

fn nar_at_time_from_tab(nar_tab: Option<&ResultTab>, cohort_label: &str, t: f64) -> Option<usize> {
    let tab = nar_tab?;
    for row in &tab.rows {
        let Some(ResultCell::Text(lbl)) = row.first() else { continue };
        if lbl != cohort_label {
            continue;
        }
        let Some(ResultCell::Number { value: row_t, .. }) = row.get(1) else { continue };
        let Some(ResultCell::Number { value: risk, .. }) = row.get(2) else { continue };
        if *row_t >= t - 1e-9 && risk.is_finite() && *risk >= 0.0 {
            return Some(*risk as usize);
        }
    }
    None
}

fn extract_censored_from_nar_tab(tab: &ResultTab, cohort_label: &str, col_idx: usize, out: &mut Vec<(f64, Option<usize>, usize)>) {
    let mut prev_risk: Option<usize> = None;
    for row in &tab.rows {
        let Some(ResultCell::Text(lbl)) = row.first() else { continue };
        if lbl != cohort_label {
            continue;
        }
        let Some(ResultCell::Number { value: row_t, .. }) = row.get(1) else { continue };
        let Some(ResultCell::Number { value: risk, .. }) = row.get(2) else { continue };
        let Some(ResultCell::Number { value: events, .. }) = row.get(3) else { continue };
        let r_now = (*risk).max(0.0) as usize;
        let ev_now = (*events).max(0.0) as usize;
        if let Some(prev) = prev_risk
            && prev > r_now + ev_now
        {
            out.push((*row_t, None, col_idx));
        } else if ev_now == 0 && r_now > 0 {
            out.push((*row_t, None, col_idx));
        }
        prev_risk = Some(r_now);
    }
}

fn draw_single_km_cohort(scene: &mut ChartScene, cohort: &KmCurveData, x_map: AxisMapping, y_map: AxisMapping, x_max: f64, scale_pct: f64) {
    let r = scene.plot_rect_pt;
    if cohort.steps.is_empty() {
        return;
    }

    if cohort.show_greenwood_ci {
        draw_km_ci_band(scene, r, cohort, x_map, y_map, scale_pct);
    }

    let mut step_path = Path2D::new();
    let Some(first) = cohort.steps.first() else { return };
    if let (Some(x0), Some(y0)) = (x_map.map_value(first.time, r.x, r.right()), y_map.map_value(first.survival * scale_pct, r.bottom(), r.y)) {
        path_move(&mut step_path, PointPt::new(x0, y0));
    }
    for win in cohort.steps.windows(2) {
        let (Some(a), Some(b)) = (win.first(), win.get(1)) else { continue };
        let (Some(xb), Some(ya), Some(yb)) = (
            x_map.map_value(b.time, r.x, r.right()),
            y_map.map_value(a.survival * scale_pct, r.bottom(), r.y),
            y_map.map_value(b.survival * scale_pct, r.bottom(), r.y),
        ) else {
            continue;
        };
        path_line(&mut step_path, PointPt::new(xb, ya));
        path_line(&mut step_path, PointPt::new(xb, yb));
    }
    if let Some(last) = cohort.steps.last()
        && let Some(max_cens) = cohort.censored_times.iter().map(|&(t, _, _)| t).max_by(|a, b| a.total_cmp(b))
        && max_cens > last.time
        && let (Some(xe), Some(yl)) =
            (x_map.map_value(max_cens.min(x_max), r.x, r.right()), y_map.map_value(last.survival * scale_pct, r.bottom(), r.y))
    {
        path_line(&mut step_path, PointPt::new(xe, yl));
    }

    scene.primitives.push(ScenePrimitive::Path {
        path: step_path,
        fill: None,
        stroke: Some(solid_stroke(cohort.color, cohort.stroke_width_pt, LineDashStyle::Solid)),
        clip: Some(r),
    });

    if let Some(sheet_id) = &cohort.sheet_id {
        for obs in &cohort.observations {
            if obs.status != SurvivalStatus::Event {
                continue;
            }
            let s_val = eval_km_step(&cohort.steps, obs.time) * scale_pct;
            if let (Some(xe), Some(ye)) = (x_map.map_value(obs.time, r.x, r.right()), y_map.map_value(s_val, r.bottom(), r.y)) {
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt: PointPt::new(xe, ye),
                    radius_pt: 4.0,
                    source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: obs.row, col: cohort.col_index, subcol: 0 },
                    series_label: cohort.label.clone(),
                    value_summary: format!("t={}, S={}", format_point_value(obs.time), format_point_value(s_val)),
                });
            }
        }
    }

    if cohort.show_censored_ticks {
        let censor_stroke = solid_stroke(cohort.color, (cohort.stroke_width_pt * 0.85).max(1.0), LineDashStyle::Solid);
        for &(tc, row_opt, col_idx) in &cohort.censored_times {
            let s_val = eval_km_step(&cohort.steps, tc) * scale_pct;
            if let (Some(xc), Some(yc)) = (x_map.map_value(tc, r.x, r.right()), y_map.map_value(s_val, r.bottom(), r.y)) {
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(xc, yc - 3.5), PointPt::new(xc, yc + 3.5)),
                    fill: None,
                    stroke: Some(censor_stroke),
                    clip: Some(r),
                });
                if let (Some(sheet_id), Some(row)) = (&cohort.sheet_id, row_opt) {
                    scene.plotted_points.push(PlottedPointMeta {
                        center_pt: PointPt::new(xc, yc),
                        radius_pt: 4.5,
                        source_cell: SourceCellId { sheet_id: sheet_id.clone(), row, col: col_idx, subcol: 0 },
                        series_label: cohort.label.clone(),
                        value_summary: format!("t={}, S={}", format_point_value(tc), format_point_value(s_val)),
                    });
                }
            }
        }
    }
}

fn draw_km_ci_band(scene: &mut ChartScene, r: RectPt, cohort: &KmCurveData, x_map: AxisMapping, y_map: AxisMapping, scale_pct: f64) {
    let mut upper_pts = Vec::new();
    let mut lower_pts = Vec::new();
    for win in cohort.steps.windows(2) {
        let (Some(a), Some(b)) = (win.first(), win.get(1)) else { continue };
        let (Some(xa), Some(xb)) = (x_map.map_value(a.time, r.x, r.right()), x_map.map_value(b.time, r.x, r.right())) else {
            continue;
        };
        if let Some(y_hi) = y_map.map_value(a.ci_upper * scale_pct, r.bottom(), r.y) {
            upper_pts.push((xa, y_hi));
            upper_pts.push((xb, y_hi));
        }
        if let Some(y_lo) = y_map.map_value(a.ci_lower * scale_pct, r.bottom(), r.y) {
            lower_pts.push((xa, y_lo));
            lower_pts.push((xb, y_lo));
        }
    }
    if upper_pts.is_empty() || lower_pts.is_empty() {
        return;
    }

    if cohort.ci_dashed {
        let mut up_p = Path2D::new();
        if let Some(&(x0, y0)) = upper_pts.first() {
            path_move(&mut up_p, PointPt::new(x0, y0));
            for &(x, y) in upper_pts.iter().skip(1) {
                path_line(&mut up_p, PointPt::new(x, y));
            }
        }
        let mut lo_p = Path2D::new();
        if let Some(&(x0, y0)) = lower_pts.first() {
            path_move(&mut lo_p, PointPt::new(x0, y0));
            for &(x, y) in lower_pts.iter().skip(1) {
                path_line(&mut lo_p, PointPt::new(x, y));
            }
        }
        let dash_stroke = solid_stroke(cohort.color, 1.0, LineDashStyle::Dashed);
        scene.primitives.push(ScenePrimitive::Path { path: up_p, fill: None, stroke: Some(dash_stroke), clip: Some(r) });
        scene.primitives.push(ScenePrimitive::Path { path: lo_p, fill: None, stroke: Some(dash_stroke), clip: Some(r) });
    } else {
        let mut poly = Vec::with_capacity(upper_pts.len() + lower_pts.len());
        poly.extend(upper_pts);
        poly.extend(lower_pts.into_iter().rev());
        scene.primitives.push(ScenePrimitive::Path {
            path: polygon_path(&poly),
            fill: Some(alpha_fill(cohort.color, 46)),
            stroke: None,
            clip: Some(r),
        });
    }
}

fn eval_km_step(steps: &[KmPointStep], t: f64) -> f64 {
    let mut current = steps.first().map_or(1.0, |s| s.survival);
    for s in steps {
        if s.time <= t + 1e-12 {
            current = s.survival;
        } else {
            break;
        }
    }
    current
}

fn draw_number_at_risk_table(scene: &mut ChartScene, r: RectPt, cohorts: &[KmCurveData], x_ticks: &[(f64, String)], x_map: AxisMapping) {
    let top_y = r.bottom() + 40.0;
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>Number at risk</b>"),
        anchor_pt: PointPt::new(r.x - 6.0, top_y),
        anchor: TextAnchor::End,
        font_size_pt: 8.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    for (idx, cohort) in cohorts.iter().enumerate() {
        let row_y = top_y + 13.0 + (idx as f64) * 12.5;
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(cohort.label.clone()),
            anchor_pt: PointPt::new(r.x - 6.0, row_y),
            anchor: TextAnchor::End,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: cohort.color,
        }));
        for &(t_val, _) in x_ticks {
            let Some(x_pt) = x_map.map_value(t_val, r.x, r.right()) else { continue };
            if x_pt < r.x - 2.0 || x_pt > r.right() + 2.0 {
                continue;
            }
            let n_risk = if !cohort.observations.is_empty() {
                cohort.observations.iter().filter(|o| o.time >= t_val - 1e-9).count()
            } else if t_val <= 1e-9 {
                cohort.n_total
            } else {
                cohort.steps.iter().find(|s| s.time >= t_val - 1e-9).map_or(0, |s| s.n_at_risk_before)
            };
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(format!("{n_risk}")),
                anchor_pt: PointPt::new(x_pt, row_y),
                anchor: TextAnchor::Middle,
                font_size_pt: 8.0,
                rotation_deg: 0.0,
                color: Rgba8::rgb(25, 25, 25),
            }));
        }
    }
}

// ============================================================================
// 2. Contingency Grouped & Stacked Bars (Counts & 100% Proportions)
// ============================================================================

struct ContingencyMatrix {
    category_labels: Vec<String>,
    series_labels: Vec<String>,
    series_colors: Vec<Rgba8>,
    /// `counts[cat_idx][series_idx]`
    counts: Vec<Vec<f64>>,
}

fn compile_contingency(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(mat) = extract_contingency_matrix(graph, doc) else {
        scene.warnings.push(ChartWarning::new("empty_contingency_data", "no contingency counts found"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let proportion_scale = detect_proportion_scale(&graph.spec);
    let is_stacked = matches!(graph.spec.grouped_arrangement, GroupedArrangement::Stacked);
    let is_superimposed = matches!(graph.spec.grouped_arrangement, GroupedArrangement::Superimposed);

    let mut values = mat.counts.clone();
    if let Some(scale_top) = proportion_scale {
        for row in &mut values {
            let row_sum: f64 = row.iter().sum();
            if row_sum > 0.0 {
                for cell in row.iter_mut() {
                    *cell = (*cell / row_sum) * scale_top;
                }
            }
        }
    }

    let data_max = if let Some(scale_top) = proportion_scale {
        if is_stacked { scale_top } else { values.iter().flat_map(|r| r.iter()).copied().fold(0.0_f64, f64::max).max(1.0) }
    } else if is_stacked {
        values.iter().map(|r| r.iter().sum::<f64>()).fold(0.0_f64, f64::max).max(1.0)
    } else {
        values.iter().flat_map(|r| r.iter()).copied().fold(0.0_f64, f64::max).max(1.0)
    };

    let (y_lo, y_hi) = resolve_axis_domain_with_warnings(
        &graph.spec.y_axis,
        0.0,
        if proportion_scale.is_some() && is_stacked { data_max } else { data_max * 1.1 },
        &mut scene.warnings,
    );
    let y_ticks = nice_axis_ticks_with_warnings(&graph.spec.y_axis, y_lo, y_hi, 5, &mut scene.warnings);
    let max_y_w = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &y_ticks,
        PlotMarginConfig { has_legend: !mat.series_labels.is_empty(), ..PlotMarginConfig::default() },
    );

    let r = scene.plot_rect_pt;
    let y_map = AxisMapping { min: y_lo, max: y_hi, scale: graph.spec.y_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale) };
    let n_cats = mat.category_labels.len().max(1);
    let slot_w = r.width / (n_cats as f64);
    let mut x_cat_positions = Vec::with_capacity(n_cats);
    for (idx, lbl) in mat.category_labels.iter().enumerate() {
        x_cat_positions.push((r.x + ((idx as f64) + 0.5) * slot_w, lbl.clone()));
    }

    let x_map_dummy = AxisMapping { min: 0.0, max: n_cats as f64, scale: AxisScale::Linear };
    let frame = CartesianFrame { spec: &graph.spec, x_ticks: &[], y_ticks: &y_ticks, x_map: x_map_dummy, y_map, max_y_label_w: max_y_w, doc };
    draw_cartesian_frame(scene, &frame);
    draw_categorical_x_ticks(scene, &graph.spec, &x_cat_positions, doc);

    let bar_border = solid_stroke(Rgba8::rgb(40, 40, 40), 0.75, LineDashStyle::Solid);
    let y_base_pt = crate::clamp_f64_safe(y_map.map_value(0.0, r.bottom(), r.y).unwrap_or(r.bottom()), r.y, r.bottom());

    for (cat_idx, row_vals) in values.iter().enumerate() {
        let center_x = r.x + ((cat_idx as f64) + 0.5) * slot_w;
        if is_stacked {
            let bar_w = (slot_w * 0.62).min(48.0);
            let mut acc = 0.0;
            for (s_idx, &val) in row_vals.iter().enumerate() {
                let top_val = acc + val;
                let (Some(y_top), Some(y_bot)) = (y_map.map_value(top_val, r.bottom(), r.y), y_map.map_value(acc, r.bottom(), r.y)) else {
                    acc = top_val;
                    continue;
                };
                let rect = RectPt::new(center_x - bar_w * 0.5, y_top.min(y_bot), bar_w, (y_bot - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
                acc = top_val;
            }
        } else if is_superimposed {
            let base_w = (slot_w * 0.62).min(44.0);
            let mut order: Vec<(usize, f64)> = row_vals.iter().copied().enumerate().collect();
            order.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()).then_with(|| a.0.cmp(&b.0)));
            for (rank, &(s_idx, val)) in order.iter().enumerate() {
                let Some(y_top) = y_map.map_value(val, r.bottom(), r.y) else { continue };
                let bar_w = (base_w * (1.0 - 0.12 * (rank as f64).min(4.0))).max(4.0);
                let rect = RectPt::new(center_x - bar_w * 0.5, y_top.min(y_base_pt), bar_w, (y_base_pt - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
            }
        } else {
            let n_s = mat.series_labels.len().max(1);
            let group_ratio = if matches!(graph.spec.grouped_arrangement, GroupedArrangement::Separated) { 0.58 } else { 0.74 };
            let group_w = slot_w * group_ratio;
            let bar_w = (group_w / (n_s as f64)).min(32.0);
            let total_w = bar_w * (n_s as f64);
            let start_x = center_x - total_w * 0.5;
            for (s_idx, &val) in row_vals.iter().enumerate() {
                let Some(y_top) = y_map.map_value(val, r.bottom(), r.y) else { continue };
                let x_left = start_x + (s_idx as f64) * bar_w;
                let rect = RectPt::new(x_left, y_top.min(y_base_pt), (bar_w - 1.0).max(2.0), (y_base_pt - y_top).abs().max(0.5));
                let color = mat.series_colors.get(s_idx).copied().unwrap_or_else(|| graph.spec.palette.color_at(s_idx));
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(rect),
                    fill: Some(FillPaint::solid(color)),
                    stroke: Some(bar_border),
                    clip: Some(r),
                });
            }
        }
    }

    let legend_entries: Vec<LegendEntry> = mat
        .series_labels
        .iter()
        .zip(mat.series_colors.iter())
        .map(|(lbl, &col)| LegendEntry { label: lbl.clone(), color: col, marker: None, dash: None, is_line: false })
        .collect();
    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

fn detect_proportion_scale(spec: &GraphSpec) -> Option<f64> {
    let seg_max = spec.y_axis.segments.first().and_then(|s| s.max);
    let title = spec.y_axis.title.to_ascii_lowercase();
    if title.contains("proportion") || title.contains("fraction") || seg_max.is_some_and(|m| (m - 1.0).abs() < 1e-6) {
        Some(1.0)
    } else if title.contains('%') || title.contains("percent") || seg_max.is_some_and(|m| (m - 100.0).abs() < 1e-6) {
        Some(100.0)
    } else {
        None
    }
}

fn extract_contingency_matrix(graph: &GraphSheet, doc: &ProjectDocument) -> Option<ContingencyMatrix> {
    let ds = doc.data_sheet(&graph.spec.primary_source_sheet_id)?;
    let by_columns = matches!(graph.spec.x_axis.tick_format, TickLabelFormat::Categorical(CategoricalTickSource::ColumnTitles));
    match &ds.archetype {
        TableArchetype::Contingency(cont) => {
            let nrows = cont.row_count();
            let ncols = cont.data_sets.len();
            if nrows == 0 || ncols == 0 {
                return None;
            }
            if by_columns {
                let category_labels = cont.data_sets.iter().map(|c| c.title.clone()).collect();
                let series_labels: Vec<String> = (0..nrows)
                    .map(|r| cont.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)))
                    .collect();
                let series_colors = (0..nrows).map(|r| series_color_or_palette(graph, r)).collect();
                let mut counts = vec![vec![0.0; nrows]; ncols];
                for (c, col_counts) in counts.iter_mut().enumerate() {
                    for (r, slot) in col_counts.iter_mut().enumerate() {
                        *slot = cont.count_at(r, c).ok().flatten().unwrap_or(0) as f64;
                    }
                }
                Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
            } else {
                let category_labels: Vec<String> = (0..nrows)
                    .map(|r| cont.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)))
                    .collect();
                let series_labels: Vec<String> = (0..ncols)
                    .map(|c| {
                        graph
                            .spec
                            .series
                            .iter()
                            .find(|s| s.data_set_index == c)
                            .and_then(|s| s.legend_label.clone())
                            .unwrap_or_else(|| cont.data_sets.get(c).map_or_else(|| format!("Series {c}"), |d| d.title.clone()))
                    })
                    .collect();
                let series_colors = (0..ncols).map(|c| series_color_or_palette(graph, c)).collect();
                let mut counts = vec![vec![0.0; ncols]; nrows];
                for (r, row_counts) in counts.iter_mut().enumerate() {
                    for (c, slot) in row_counts.iter_mut().enumerate() {
                        *slot = cont.count_at(r, c).ok().flatten().unwrap_or(0) as f64;
                    }
                }
                Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
            }
        }
        TableArchetype::Column(col) => {
            let nrows = col.row_count();
            let ncols = col.data_sets.len();
            if nrows == 0 || ncols == 0 {
                return None;
            }
            let category_labels =
                (0..nrows).map(|r| col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect();
            let series_labels = col.data_sets.iter().map(|d| d.title.clone()).collect();
            let series_colors = (0..ncols).map(|c| series_color_or_palette(graph, c)).collect();
            let mut counts = vec![vec![0.0; ncols]; nrows];
            for (r, row_counts) in counts.iter_mut().enumerate() {
                for (c, ds_col) in col.data_sets.iter().enumerate() {
                    row_counts[c] = ds_col.cells.get(r).and_then(|s| s.first()).and_then(|cell| cell.active_number()).unwrap_or(0.0).max(0.0);
                }
            }
            Some(ContingencyMatrix { category_labels, series_labels, series_colors, counts })
        }
        _ => None,
    }
}

fn series_color_or_palette(graph: &GraphSheet, idx: usize) -> Rgba8 {
    if let Some(s) = graph.spec.series.iter().find(|s| s.data_set_index == idx).or_else(|| graph.spec.series.get(idx)) {
        match s.gdo {
            GraphicalDataObject::BarWithReplicates { fill_color, .. } | GraphicalDataObject::FloatingBar { fill_color, .. } => return fill_color,
            GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } => return m.fill_color,
            _ => {}
        }
    }
    graph.spec.palette.color_at(idx)
}

// ============================================================================
// 3. Parts-of-Whole (Pie, Donut, StackedFractionBar, 10x10 Waffle)
// ============================================================================

fn compile_parts_of_whole(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let (layout_mode, show_center_total) = resolve_parts_of_whole_style(&graph.spec.series);
    let slices = extract_parts_of_whole_slices(graph, doc);
    if slices.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_parts_of_whole", "no non-negative parts-of-whole slices available"));
        return Ok(());
    }

    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig { has_legend: true, suppress_y_axis_labels: true, ..PlotMarginConfig::default() },
    );

    let mut legend_entries = Vec::with_capacity(slices.len());
    for (idx, slice) in slices.iter().enumerate() {
        let color = graph.spec.palette.color_at(idx);
        let name = if slice.label.trim().is_empty() { format!("Slice {}", idx + 1) } else { slice.label.clone() };
        legend_entries.push(LegendEntry {
            label: format!("{name} ({:.1}%)", slice.fraction * 100.0),
            color,
            marker: None,
            dash: None,
            is_line: false,
        });
    }

    match layout_mode {
        PartsOfWholeLayout::Pie => render_pie_or_donut(scene, &graph.spec, &slices, 0.0, false),
        PartsOfWholeLayout::Donut => render_pie_or_donut(scene, &graph.spec, &slices, 0.56, show_center_total),
        PartsOfWholeLayout::StackedFractionBar => render_stacked_fraction_bar(scene, &graph.spec, &slices),
        PartsOfWholeLayout::Waffle10x10 => render_waffle_10x10(scene, &graph.spec, &slices),
    }

    draw_legend_entries(scene, &graph.spec, &legend_entries, doc);
    Ok(())
}

fn resolve_parts_of_whole_style(series: &[SeriesSpec]) -> (PartsOfWholeLayout, bool) {
    for s in series {
        if let GraphicalDataObject::PartsOfWhole { layout, show_center_total } = s.gdo {
            return (layout, show_center_total);
        }
    }
    (PartsOfWholeLayout::Donut, true)
}

fn extract_parts_of_whole_slices(graph: &GraphSheet, doc: &ProjectDocument) -> Vec<lumencraft_doc::SliceFraction> {
    let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) else { return Vec::new() };
    let series_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    match &ds.archetype {
        TableArchetype::PartsOfWhole(pow) => pow.fractions_for_series(series_idx).unwrap_or_default(),
        TableArchetype::Column(col) => {
            let Some(col_data) = col.data_sets.get(series_idx) else { return Vec::new() };
            let mut raw = Vec::new();
            let mut total = 0.0;
            for (r, subs) in col_data.cells.iter().enumerate() {
                if let Some(val) = subs.first().and_then(|c| c.active_number())
                    && val >= 0.0
                {
                    total += val;
                    let lbl = col.row_titles.get(r).cloned().unwrap_or_else(|| format!("Item {}", r + 1));
                    raw.push((r, lbl, val));
                }
            }
            raw.into_iter()
                .map(|(row, label, value)| lumencraft_doc::SliceFraction {
                    row,
                    label,
                    value,
                    fraction: if total > 0.0 { value / total } else { 0.0 },
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn render_pie_or_donut(
    scene: &mut ChartScene,
    spec: &GraphSpec,
    slices: &[lumencraft_doc::SliceFraction],
    inner_ratio: f64,
    show_center_total: bool,
) {
    let r = scene.plot_rect_pt;
    let cx = r.x + r.width * 0.5;
    let cy = r.y + r.height * 0.5;
    let r_outer = (r.width.min(r.height) * 0.43).max(16.0);
    let r_inner = r_outer * inner_ratio;
    let border = solid_stroke(Rgba8::WHITE, 1.0, LineDashStyle::Solid);

    let mut angle = -PI * 0.5;
    let mut total_val = 0.0;
    for (idx, slice) in slices.iter().enumerate() {
        total_val += slice.value;
        let sweep = slice.fraction * 2.0 * PI;
        if sweep <= 1e-6 {
            continue;
        }
        let end_angle = angle + sweep;
        let path = wedge_path(cx, cy, r_outer, r_inner, angle, end_angle);
        let fill = FillPaint::solid(spec.palette.color_at(idx));
        scene.primitives.push(ScenePrimitive::Path { path, fill: Some(fill), stroke: Some(border), clip: None });
        angle = end_angle;
    }

    if show_center_total && inner_ratio > 0.05 {
        let mut num_str = format!("{total_val:.2}");
        trim_trailing_zeros(&mut num_str);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&format!("<b>Total: {num_str}</b>")),
            anchor_pt: PointPt::new(cx, cy + 3.5),
            anchor: TextAnchor::Middle,
            font_size_pt: 9.5,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}

fn render_stacked_fraction_bar(scene: &mut ChartScene, spec: &GraphSpec, slices: &[lumencraft_doc::SliceFraction]) {
    let r = scene.plot_rect_pt;
    let bar_h = crate::clamp_f64_safe(r.height * 0.36, 24.0, 56.0);
    let bar_y = r.y + (r.height - bar_h) * 0.5;
    let border = solid_stroke(Rgba8::WHITE, 0.9, LineDashStyle::Solid);

    let mut cursor_x = r.x;
    for (idx, slice) in slices.iter().enumerate() {
        let seg_w = slice.fraction * r.width;
        if seg_w <= 0.5 {
            continue;
        }
        let rect = RectPt::new(cursor_x, bar_y, seg_w, bar_h);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(rect),
            fill: Some(FillPaint::solid(spec.palette.color_at(idx))),
            stroke: Some(border),
            clip: None,
        });
        cursor_x += seg_w;
    }

    let axis_y = bar_y + bar_h + 10.0;
    let axis_stroke = solid_stroke(Rgba8::BLACK, 0.9, LineDashStyle::Solid);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(r.x, axis_y), PointPt::new(r.right(), axis_y)),
        fill: None,
        stroke: Some(axis_stroke),
        clip: None,
    });
    for pct in [0, 25, 50, 75, 100] {
        let x = r.x + ((pct as f64) / 100.0) * r.width;
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x, axis_y), PointPt::new(x, axis_y + 3.5)),
            fill: None,
            stroke: Some(axis_stroke),
            clip: None,
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(format!("{pct}%")),
            anchor_pt: PointPt::new(x, axis_y + 12.0),
            anchor: TextAnchor::Middle,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
    }
}

fn render_waffle_10x10(scene: &mut ChartScene, spec: &GraphSpec, slices: &[lumencraft_doc::SliceFraction]) {
    let r = scene.plot_rect_pt;
    let side = r.width.min(r.height).max(40.0) * 0.88;
    let gap = 2.0;
    let cell = ((side - 9.0 * gap) / 10.0).max(2.5);
    let total_side = 10.0 * cell + 9.0 * gap;
    let origin_x = r.x + (r.width - total_side) * 0.5;
    let origin_y = r.y + (r.height - total_side) * 0.5;

    let counts = largest_remainder_100(slices);
    let mut cell_colors = Vec::with_capacity(100);
    for (idx, &cnt) in counts.iter().enumerate() {
        let color = spec.palette.color_at(idx);
        for _ in 0..cnt {
            if cell_colors.len() < 100 {
                cell_colors.push(color);
            }
        }
    }
    while cell_colors.len() < 100 {
        cell_colors.push(Rgba8::rgb(220, 224, 230));
    }

    let cell_border = solid_stroke(Rgba8::rgba(255, 255, 255, 180), 0.5, LineDashStyle::Solid);
    for idx in 0..100 {
        let row_from_bottom = idx / 10;
        let col = idx % 10;
        let row_from_top = 9 - row_from_bottom;
        let x = origin_x + (col as f64) * (cell + gap);
        let y = origin_y + (row_from_top as f64) * (cell + gap);
        let color = cell_colors.get(idx).copied().unwrap_or(Rgba8::rgb(220, 224, 230));
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(x, y, cell, cell)),
            fill: Some(FillPaint::solid(color)),
            stroke: Some(cell_border),
            clip: None,
        });
    }
}

fn largest_remainder_100(slices: &[lumencraft_doc::SliceFraction]) -> Vec<usize> {
    if slices.is_empty() {
        return Vec::new();
    }
    let sum_frac: f64 = slices.iter().map(|s| s.fraction.max(0.0)).sum();
    if sum_frac <= 0.0 {
        return vec![0; slices.len()];
    }
    let mut base: Vec<usize> = Vec::with_capacity(slices.len());
    let mut remainders: Vec<(usize, f64)> = Vec::with_capacity(slices.len());
    let mut assigned = 0_usize;
    for (i, s) in slices.iter().enumerate() {
        let exact = (s.fraction.max(0.0) / sum_frac) * 100.0;
        let floor_i = crate::clamp_f64_safe(exact.floor(), 0.0, 100.0) as usize;
        base.push(floor_i);
        assigned += floor_i;
        remainders.push((i, exact - (floor_i as f64)));
    }
    remainders.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let remaining = 100_usize.saturating_sub(assigned);
    for (i, _) in remainders.into_iter().take(remaining) {
        if let Some(slot) = base.get_mut(i) {
            *slot += 1;
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{
        ContingencyTable, DataSheet, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, PartsOfWholeLayout,
        PartsOfWholeTable, ProjectDocument, SheetId, SurvivalTable, TableArchetype,
    };

    use super::{compile_survival_or_categorical_graph, largest_remainder_100};
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn survival_compiles_curves_censors_ci_and_number_at_risk() {
        let mut surv = SurvivalTable::new(2, 6).expect("surv");
        surv.cohorts[0].title = String::from("Vehicle");
        surv.cohorts[1].title = String::from("Drug");
        for (r, (t, s0, s1)) in
            [(2.0, 1.0, 0.0), (4.0, 1.0, 1.0), (6.0, 0.0, 0.0), (8.0, 1.0, 1.0), (10.0, 1.0, 0.0), (12.0, 1.0, 1.0)].into_iter().enumerate()
        {
            surv.times[r] = lumencraft_doc::Cell::number(t);
            surv.cohorts[0].cells[r][0] = lumencraft_doc::Cell::number(s0);
            surv.cohorts[1].cells[r][0] = lumencraft_doc::Cell::number(s1);
        }
        let ds = DataSheet::new(SheetId::new("surv-1"), "OS", TableArchetype::Survival(surv)).expect("ds");
        let mut graph = GraphSheet::new_uninitialized_for_data(SheetId::new("g-1"), &ds).expect("gs");
        for series in &mut graph.spec.series {
            if let GraphicalDataObject::KaplanMeierStep { show_greenwood_ci, ci_dashed, .. } = &mut series.gdo {
                *show_greenwood_ci = true;
                *ci_dashed = false;
            }
        }
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds).expect("add");
        let mut scene = ChartScene::new("g-1", "OS", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&graph, &doc, &mut scene).expect("compile");

        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Number at risk"))));
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), stroke: None, .. })));
    }

    #[test]
    fn waffle_10x10_emits_100_cells_and_donut_emits_center_total() {
        let mut pow = PartsOfWholeTable::new(1, 3).expect("pow");
        pow.slice_labels = vec![String::from("Alpha"), String::from("Beta"), String::from("Gamma")];
        for (r, v) in [50.0, 30.0, 20.0].into_iter().enumerate() {
            pow.data_sets[0].cells[r][0] = lumencraft_doc::Cell::number(v);
        }
        let ds = DataSheet::new(SheetId::new("pow-1"), "Share", TableArchetype::PartsOfWhole(pow)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");

        let mut waffle_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-w"), &ds).expect("gs");
        waffle_gs.spec.series[0].gdo = GraphicalDataObject::PartsOfWhole { layout: PartsOfWholeLayout::Waffle10x10, show_center_total: true };
        let mut w_scene = ChartScene::new("g-w", "Waffle", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&waffle_gs, &doc, &mut w_scene).expect("compile waffle");
        let waffle_cells = w_scene.primitives.iter().filter(|p| matches!(p, ScenePrimitive::Path { fill: Some(_), .. })).count();
        assert!(waffle_cells >= 100);

        let donut_gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-d"), &ds).expect("gs");
        let mut d_scene = ChartScene::new("g-d", "Donut", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&donut_gs, &doc, &mut d_scene).expect("compile donut");
        assert!(d_scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text().contains("Total: 100"))));

        let fractions = ds.archetype_fractions_test();
        assert_eq!(largest_remainder_100(&fractions).iter().sum::<usize>(), 100);
    }

    trait PowHelper {
        fn archetype_fractions_test(&self) -> Vec<lumencraft_doc::SliceFraction>;
    }
    impl PowHelper for DataSheet {
        fn archetype_fractions_test(&self) -> Vec<lumencraft_doc::SliceFraction> {
            match &self.archetype {
                TableArchetype::PartsOfWhole(p) => p.fractions_for_series(0).expect("fractions"),
                _ => Vec::new(),
            }
        }
    }

    #[test]
    fn contingency_stacked_and_proportion_compile() {
        let mut cont = ContingencyTable::new(2, 2).expect("cont");
        cont.row_titles = vec![String::from("Control"), String::from("Treated")];
        cont.data_sets[0].title = String::from("Responder");
        cont.data_sets[1].title = String::from("NonResponder");
        cont.data_sets[0].cells[0][0] = lumencraft_doc::Cell::number(10.0);
        cont.data_sets[1].cells[0][0] = lumencraft_doc::Cell::number(30.0);
        cont.data_sets[0].cells[1][0] = lumencraft_doc::Cell::number(24.0);
        cont.data_sets[1].cells[1][0] = lumencraft_doc::Cell::number(16.0);
        let ds = DataSheet::new(SheetId::new("c-1"), "Contingency", TableArchetype::Contingency(cont)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-c"), &ds).expect("gs");
        gs.spec.kind = GraphKind::Contingency;
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.y_axis = GraphSpec::default_for_data_sheet(&ds).y_axis;
        gs.spec.y_axis.title = String::from("Percent (%)");

        let mut scene = ChartScene::new("g-c", "Contingency", 420.0, 300.0, None);
        compile_survival_or_categorical_graph(&gs, &doc, &mut scene).expect("compile");
        assert!(scene.primitives.iter().any(|p| matches!(p, ScenePrimitive::Text(t) if t.rich.plain_text() == "100")));
    }

    #[test]
    fn narrow_and_tiny_canvases_do_not_panic() {
        assert!((crate::clamp_f64_safe(5.0, 11.0, 6.4) - 6.4).abs() < 1e-12);
        assert!((crate::clamp_f64_safe(f64::NAN, 11.0, 6.4) - 6.4).abs() < 1e-12);
        assert!(crate::clamp_f64_safe(f64::NAN, f64::NAN, f64::NAN).abs() < 1e-12);

        let mut cont = ContingencyTable::new(2, 2).expect("cont");
        cont.row_titles = vec![String::from("Control"), String::from("Treated")];
        cont.data_sets[0].title = String::from("A");
        cont.data_sets[1].title = String::from("B");
        cont.data_sets[0].cells[0][0] = lumencraft_doc::Cell::number(10.0);
        cont.data_sets[1].cells[0][0] = lumencraft_doc::Cell::number(20.0);
        cont.data_sets[0].cells[1][0] = lumencraft_doc::Cell::number(15.0);
        cont.data_sets[1].cells[1][0] = lumencraft_doc::Cell::number(25.0);
        let ds = DataSheet::new(SheetId::new("c-narrow"), "Contingency", TableArchetype::Contingency(cont)).expect("ds");
        let mut doc = ProjectDocument::default();
        doc.add_data_sheet(ds.clone()).expect("add");
        let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("g-narrow"), &ds).expect("gs");
        gs.spec.kind = GraphKind::Contingency;
        gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
        gs.spec.y_axis = GraphSpec::default_for_data_sheet(&ds).y_axis;
        gs.spec.y_axis.title = String::from("Count");

        for (w, h) in [(30.0, 200.0), (10.0, 200.0), (200.0, 10.0), (1.0, 1.0)] {
            gs.initialized = true;
            gs.spec = GraphSpec::default_for_data_sheet(&ds);
            gs.spec.y_axis.title = String::from("Count");
            gs.spec.canvas_size_pt = lumencraft_doc::SizePt::new(w, h);
            let default_scene = crate::compile_graph(&gs, &doc).expect("narrow default canvas should compile cleanly");
            assert!(!default_scene.primitives.is_empty());

            gs.spec.kind = GraphKind::Contingency;
            gs.spec.grouped_arrangement = GroupedArrangement::Stacked;
            let stacked_scene = crate::compile_graph(&gs, &doc).expect("narrow stacked canvas should compile cleanly");
            assert!(!stacked_scene.primitives.is_empty());
        }
    }
}
