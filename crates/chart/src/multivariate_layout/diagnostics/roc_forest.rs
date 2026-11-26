//! ROC curve (sensitivity vs 1-specificity + AUC) and forest plot diagnostic builders.

use curvebook_doc::{
    AnalysisKind, FittedCurveOverlay, GlmFamily, GlmSpec, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultTabKind, SubcolumnMode,
    TableArchetype, VariableRole,
};
use curvebook_math::{AxisScale, ConfidenceLevel};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::roc::{RocCiMethod, roc_curve_analysis};
use curvebook_text::{RichText, parse_markup};

use super::super::multivariate::{cell_token, column_categorical_levels};
use super::{find_matching_result_checked, find_matching_result_where_checked, raw_matching_results, resolve_source_data_sheet};
use crate::markers::emit_marker;
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, alpha_fill, draw_empty_open_frame, layout_and_render_segmented_chrome, path_close, path_line, path_move,
    polygon_path, solid_stroke, text_metrics,
};
use crate::xy::is_log_like;

// 2d. ROC Curve (Sensitivity vs 1-Specificity + Diagonal + AUC)

pub(in crate::multivariate_layout) fn compile_roc_curve(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some((points, auc)) = extract_roc_data(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new("empty_roc", "no ROC operating points or controls/cases columns available"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("1 - Specificity (FPR)");
    }
    if frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("Sensitivity (TPR)");
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (0.0, 1.0),
            y_bounds: (0.0, 1.0),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    if let (Some((_, x0)), Some((_, y0)), Some((_, x1)), Some((_, y1))) =
        (x_axis.map_value(0.0), y_axis.map_value(0.0), x_axis.map_value(1.0), y_axis.map_value(1.0))
    {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, y0), PointPt::new(x1, y1)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(130, 138, 148), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let color = graph.spec.palette.color_at(0);
    let mut curve_p = Path2D::new();
    let mut area_p = Path2D::new();
    let mut started = false;
    for &(fpr, tpr) in &points {
        let (Some((_, px)), Some((_, py))) =
            (x_axis.map_value(crate::clamp_f64_safe(fpr, 0.0, 1.0)), y_axis.map_value(crate::clamp_f64_safe(tpr, 0.0, 1.0)))
        else {
            continue;
        };
        if !started {
            path_move(&mut curve_p, PointPt::new(px, py));
            path_move(&mut area_p, PointPt::new(px, r.bottom()));
            path_line(&mut area_p, PointPt::new(px, py));
            started = true;
        } else {
            path_line(&mut curve_p, PointPt::new(px, py));
            path_line(&mut area_p, PointPt::new(px, py));
        }
    }
    if started {
        path_line(&mut area_p, PointPt::new(r.right(), r.bottom()));
        path_close(&mut area_p);
        scene.primitives.push(ScenePrimitive::Path { path: area_p, fill: Some(alpha_fill(color, 34)), stroke: None, clip: Some(r) });
        scene.primitives.push(ScenePrimitive::Path {
            path: curve_p,
            fill: None,
            stroke: Some(solid_stroke(color, 2.0, LineDashStyle::Solid)),
            clip: Some(r),
        });
    }

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup(&format!("<b>AUC = {auc:.3}</b>")),
        anchor_pt: PointPt::new(r.right() - 8.0, r.bottom() - 12.0),
        anchor: TextAnchor::End,
        font_size_pt: 9.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    Ok(())
}

fn is_roc_or_logistic_analysis(kind: &AnalysisKind) -> bool {
    matches!(
        kind,
        AnalysisKind::RocCurve(_)
            | AnalysisKind::SimpleLogisticRegression(_)
            | AnalysisKind::GeneralizedLinearModel(GlmSpec { family: GlmFamily::Logistic, .. })
    )
}

fn is_roc_auc_scalar_key(key: &str) -> bool {
    let norm = key.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    matches!(norm.as_str(), "roc_auc" | "auc")
}

fn resolve_roc_auc_scalar(res: &ResultSheet, curve_ds_idx: usize, single_roc_curve: bool) -> Option<f64> {
    res.scalars
        .iter()
        .find(|s| s.data_set_index == Some(curve_ds_idx) && is_roc_auc_scalar_key(&s.key) && s.value.is_finite())
        .or_else(|| res.scalars.iter().find(|s| s.data_set_index.is_none() && is_roc_auc_scalar_key(&s.key) && s.value.is_finite()))
        .or_else(|| single_roc_curve.then(|| res.scalars.iter().find(|s| is_roc_auc_scalar_key(&s.key) && s.value.is_finite())).flatten())
        .map(|s| s.value)
}

fn extract_roc_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<(Vec<(f64, f64)>, f64)> {
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let roc_res =
        find_matching_result_where_checked(graph, doc, warnings, |r| r.fitted_curves.iter().any(|fc| fc.equation_id.eq_ignore_ascii_case("ROC")))
            .or_else(|| find_matching_result_where_checked(graph, doc, warnings, |r| is_roc_or_logistic_analysis(&r.analysis.kind)));

    if let Some(res) = roc_res {
        let roc_curves: Vec<&FittedCurveOverlay> = res.fitted_curves.iter().filter(|fc| fc.equation_id.eq_ignore_ascii_case("ROC")).collect();
        if let Some(curve) = roc_curves.iter().copied().find(|fc| fc.data_set_index == ds_idx).or_else(|| roc_curves.first().copied()) {
            let pts: Vec<(f64, f64)> = curve.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| (p.x, p.y)).collect();
            if pts.len() >= 2 {
                let auc = resolve_roc_auc_scalar(res, curve.data_set_index, roc_curves.len() == 1).unwrap_or_else(|| trapezoidal_auc(&pts));
                return Some((pts, auc));
            }
        }
        if is_roc_or_logistic_analysis(&res.analysis.kind) {
            warnings.push(ChartWarning::new("missing_roc_overlay", format!("logistic/ROC result sheet '{}' contains no ROC curve overlay", res.id)));
            return None;
        }
    }

    // Never silently fall through to raw XY `(Dose, {0,1})` data when a matching logistic/ROC
    // analysis result failed or lacks an `"ROC"` overlay (`AUD-13-06`).
    if raw_matching_results(graph, doc).iter().any(|r| is_roc_or_logistic_analysis(&r.analysis.kind)) {
        return None;
    }

    let ds = resolve_source_data_sheet(graph, doc)?;
    match &ds.archetype {
        TableArchetype::Column(col) if col.data_sets.len() >= 2 => {
            let controls: Vec<f64> = col.data_sets.first()?.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect();
            let cases: Vec<f64> = col.data_sets.get(1)?.cells.iter().filter_map(|s| s.first().and_then(|c| c.active_number())).collect();
            let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
            Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc))
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let (Some(c0), Some(c1)) = (mv.variables.get(xi), mv.variables.get(yi)) else { return None };
            if c0.role == VariableRole::Categorical || c1.role == VariableRole::Categorical {
                let (cat_col, score_col) = if c0.role == VariableRole::Categorical { (c0, c1) } else { (c1, c0) };
                let levels = column_categorical_levels(cat_col, None);
                if levels.len() >= 2 {
                    let mut controls = Vec::new();
                    let mut cases = Vec::new();
                    for r in 0..mv.row_count() {
                        if let (Some(tok), Some(sc)) = (cell_token(cat_col, r), score_col.cells.get(r).and_then(|c| c.active_number())) {
                            if levels.first().is_some_and(|l| l == &tok) {
                                controls.push(sc);
                            } else if levels.get(1).is_some_and(|l| l == &tok) {
                                cases.push(sc);
                            }
                        }
                    }
                    if !controls.is_empty() && !cases.is_empty() {
                        let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
                        return Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc));
                    }
                }
            }
            let controls: Vec<f64> = c0.cells.iter().filter_map(|c| c.active_number()).collect();
            let cases: Vec<f64> = c1.cells.iter().filter_map(|c| c.active_number()).collect();
            let res = roc_curve_analysis(&controls, &cases, RocCiMethod::DeLong, ConfidenceLevel::DEFAULT_95).ok()?;
            Some((res.points.into_iter().map(|p| (p.one_minus_specificity, p.sensitivity)).collect(), res.auc))
        }
        TableArchetype::Xy(xy) => {
            let mut pts: Vec<(f64, f64)> = xy.active_xy_pairs(ds_idx).into_iter().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
            if pts.len() < 2 {
                return None;
            }
            if !pts.iter().all(|&(x, y)| (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)) {
                warnings.push(ChartWarning::new(
                    "invalid_roc_xy_coordinates",
                    "XY ROC plot requires pre-computed (1 - Specificity, Sensitivity) coordinates in [0, 1] or a logistic/ROC analysis result",
                ));
                return None;
            }
            pts.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
            let auc = trapezoidal_auc(&pts);
            Some((pts, auc))
        }
        _ => None,
    }
}

fn trapezoidal_auc(pts: &[(f64, f64)]) -> f64 {
    let mut auc = 0.0;
    for win in pts.windows(2) {
        if let (Some(&(x0, y0)), Some(&(x1, y1))) = (win.first(), win.get(1)) {
            auc += (x1 - x0) * 0.5 * (y0 + y1);
        }
    }
    crate::clamp_f64_safe(auc, 0.0, 1.0)
}

// 2e. Forest Plot (OR/HR CIs + Diamond Pooled Estimate)

struct ForestEntry {
    label: String,
    estimate: f64,
    lower: f64,
    upper: f64,
    is_pooled: bool,
}

pub(in crate::multivariate_layout) fn compile_forest_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let mut entries = extract_forest_entries(graph, doc, &mut scene.warnings);
    if entries.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_forest", "no effect estimates & confidence intervals for forest plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let x_scale = graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale);
    let is_log = matches!(x_scale, AxisScale::Log10 | AxisScale::Log2 | AxisScale::Ln);
    let all_positive = entries.iter().all(|e| e.lower > 0.0 && e.estimate > 0.0 && e.upper > 0.0);
    let ref_x: f64 = if is_log
        || (all_positive
            && (graph.spec.x_axis.title.contains("OR") || graph.spec.x_axis.title.contains("HR") || graph.spec.x_axis.title.contains("Ratio")))
    {
        1.0
    } else {
        0.0
    };

    if graph.spec.forest_show_pooled_diamond
        && entries.len() >= 2
        && !entries.iter().any(|e| e.is_pooled)
        && let Some(pooled) = compute_pooled_forest_entry(&entries, is_log || ref_x == 1.0)
    {
        entries.push(pooled);
    }

    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    for e in &entries {
        for v in [e.lower, e.estimate, e.upper] {
            if v.is_finite() && (!is_log || v > 0.0) {
                x_min = x_min.min(v);
                x_max = x_max.max(v);
            }
        }
    }
    if ref_x.is_finite() && (!is_log || ref_x > 0.0) {
        x_min = x_min.min(ref_x);
        x_max = x_max.max(ref_x);
    }
    if !x_min.is_finite() || !x_max.is_finite() {
        x_min = if is_log { 0.5 } else { -1.0 };
        x_max = if is_log { 2.0 } else { 1.0 };
    }
    for tick in &graph.spec.x_axis.additional_ticks {
        if tick.value.is_finite() && (!is_log || tick.value > 0.0) {
            x_min = x_min.min(tick.value);
            x_max = x_max.max(tick.value);
        }
    }
    for thr in &graph.spec.x_axis.thresholds {
        if thr.value.is_finite() && (!is_log || thr.value > 0.0) {
            x_min = x_min.min(thr.value);
            x_max = x_max.max(thr.value);
        }
        if let Some(end_v) = thr.shade_to
            && end_v.is_finite()
            && (!is_log || end_v > 0.0)
        {
            x_min = x_min.min(end_v);
            x_max = x_max.max(end_v);
        }
    }
    let x_bounds = if is_log {
        (x_min * 0.8, x_max * 1.25)
    } else {
        let pad = (x_max - x_min).max(1.0) * 0.1;
        (x_min - pad, x_max + pad)
    };

    let ci_labels: Vec<String> = entries.iter().map(|e| format!("{:.2} [{:.2}, {:.2}]", e.estimate, e.lower, e.upper)).collect();
    let max_ci_w = ci_labels.iter().map(|s| text_metrics(s, 7.5).width_pt).fold(0.0_f64, f64::max);
    let max_lbl_w = entries.iter().map(|e| text_metrics(&e.label, 8.0).width_pt).fold(0.0_f64, f64::max);
    let empty_cat_y: [(f64, String); 0] = [];
    let (x_axis, _, _) = layout_and_render_segmented_chrome(
        scene,
        graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds,
            y_bounds: (0.0, entries.len() as f64),
            y2_bounds: None,
            cat_x: None,
            cat_y: Some(&empty_cat_y),
            margins: PlotMarginConfig {
                extra_left_pt: crate::clamp_f64_safe(max_lbl_w + 10.0, 32.0, 96.0),
                extra_right_pt: if max_ci_w > 0.0 { crate::clamp_f64_safe(max_ci_w + 14.0, 56.0, 150.0) } else { 0.0 },
                has_legend: false,
                suppress_y_axis_labels: true,
                ..PlotMarginConfig::default()
            },
        },
    );

    let r = scene.plot_rect_pt;
    let user_has_ref_line = graph.spec.x_axis.additional_ticks.iter().any(|t| t.draw_gridline && (t.value - ref_x).abs() <= 1e-9)
        || graph.spec.x_axis.thresholds.iter().any(|t| (t.value - ref_x).abs() <= 1e-9);
    if !user_has_ref_line && let Some((_, x_ref_pt)) = x_axis.map_value(ref_x) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x_ref_pt, r.y), PointPt::new(x_ref_pt, r.bottom())),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(120, 126, 134), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let row_h = r.height / (entries.len() as f64);
    let primary_col = graph.spec.palette.color_at(0);
    let pooled_col = graph.spec.palette.color_at(1);
    for (idx, (entry, ci_text)) in entries.iter().zip(ci_labels.iter()).enumerate() {
        let cy = r.y + ((idx as f64) + 0.5) * row_h;
        let lbl_markup = if entry.is_pooled { format!("<b>{}</b>", entry.label) } else { entry.label.clone() };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(&lbl_markup),
            anchor_pt: PointPt::new(r.x - 6.0, cy + 3.0),
            anchor: TextAnchor::End,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));

        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(ci_text.clone()),
            anchor_pt: PointPt::new(r.right() + 6.0, cy + 3.0),
            anchor: TextAnchor::Start,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::rgb(35, 35, 35),
        }));

        let x_lo_opt = x_axis.map_value(entry.lower).map(|(_, p)| p);
        let x_hi_opt = x_axis.map_value(entry.upper).map(|(_, p)| p);
        let x_est_opt = x_axis.map_value(entry.estimate).map(|(_, p)| p);
        let (x_lo_pt, clip_lo) = x_lo_opt.map_or_else(
            || {
                let (_, p, c) = x_axis.map_clamped_to_nearest(entry.lower);
                (p, c)
            },
            |p| (p, false),
        );
        let (x_hi_pt, clip_hi) = x_hi_opt.map_or_else(
            || {
                let (_, p, c) = x_axis.map_clamped_to_nearest(entry.upper);
                (p, c)
            },
            |p| (p, false),
        );
        if (clip_lo && is_log && entry.lower <= 0.0)
            || (clip_hi && is_log && entry.upper <= 0.0)
            || (x_est_opt.is_none() && is_log && entry.estimate <= 0.0)
        {
            scene.warnings.push(ChartWarning::new(
                "log_axis_floor_clip",
                format!("clipped non-positive forest interval for '{}' to log axis floor", entry.label),
            ));
        }

        if entry.is_pooled {
            let x_est_pt = x_est_opt.unwrap_or_else(|| x_axis.map_clamped_to_nearest(entry.estimate).1);
            let dh = crate::clamp_f64_safe(row_h * 0.28, 3.5, 7.0);
            scene.primitives.push(ScenePrimitive::Path {
                path: polygon_path(&[(x_lo_pt, cy), (x_est_pt, cy - dh), (x_hi_pt, cy), (x_est_pt, cy + dh)]),
                fill: Some(FillPaint::solid(pooled_col)),
                stroke: Some(solid_stroke(Rgba8::BLACK, 0.8, LineDashStyle::Solid)),
                clip: Some(r),
            });
        } else {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(x_lo_pt, cy), PointPt::new(x_hi_pt, cy)),
                fill: None,
                stroke: Some(solid_stroke(primary_col, 1.2, LineDashStyle::Solid)),
                clip: Some(r),
            });
            if let Some(x_est_pt) = x_est_opt {
                emit_marker(
                    &mut scene.primitives,
                    PointPt::new(x_est_pt, cy),
                    &MarkerStyle {
                        shape: MarkerShape::Square,
                        fill_mode: MarkerFillMode::Filled,
                        size_pt: 5.5,
                        fill_color: primary_col,
                        stroke_color: Rgba8::BLACK,
                        stroke_width_pt: 0.6,
                    },
                    Some(r),
                );
            }
        }
    }
    Ok(())
}

fn push_validated_forest_entry(entries: &mut Vec<ForestEntry>, warnings: &mut Vec<ChartWarning>, label: String, est: f64, s1: f64, s2: f64) {
    let lo = s1.min(s2);
    let hi = s1.max(s2);
    if !est.is_finite() || !lo.is_finite() || !hi.is_finite() || est < lo - 1e-12 || est > hi + 1e-12 {
        warnings
            .push(ChartWarning::new("invalid_forest_interval", format!("omitted forest row '{label}': estimate {est} lies outside [{lo}, {hi}]")));
        return;
    }
    entries.push(ForestEntry {
        is_pooled: label.to_ascii_lowercase().contains("pooled"),
        label,
        estimate: est,
        lower: lo.min(est),
        upper: hi.max(est),
    });
}

fn extract_subcolumn_forest_entries(
    mode: SubcolumnMode,
    rows: &[Vec<curvebook_doc::Cell>],
    row_titles: &[String],
    warnings: &mut Vec<ChartWarning>,
) -> Vec<ForestEntry> {
    let mut entries = Vec::new();
    for (r, subs) in rows.iter().enumerate() {
        if let (Some(est), Some(s1), Some(s2)) =
            (subs.first().and_then(|c| c.active_number()), subs.get(1).and_then(|c| c.active_number()), subs.get(2).and_then(|c| c.active_number()))
        {
            let label = row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
            let (lo, hi) = if mode == SubcolumnMode::MeanLowHigh {
                if !est.is_finite() || !s1.is_finite() || !s2.is_finite() || s1 < 0.0 || s2 < 0.0 {
                    warnings.push(ChartWarning::new(
                        "invalid_forest_interval",
                        format!("omitted forest row '{label}': negative or non-finite error delta ({s1}, {s2})"),
                    ));
                    continue;
                }
                (est - s1, est + s2)
            } else {
                (s1, s2)
            };
            push_validated_forest_entry(&mut entries, warnings, label, est, lo, hi);
        }
    }
    entries
}

fn extract_forest_entries(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<ForestEntry> {
    if let Some(res) = find_matching_result_checked(graph, doc, warnings)
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.kind == ResultTabKind::MainSummary)
    {
        let mut entries = Vec::new();
        let is_log_x = is_log_like(graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale));
        let est_idx = tab
            .column_headers
            .iter()
            .position(|h| {
                let l = h.to_ascii_lowercase();
                l.contains("ratio") || l.contains("estimate") || l.contains("diff") || l.contains("beta")
            })
            .unwrap_or(0);
        for (r, row) in tab.rows.iter().enumerate() {
            if let Some((lo, hi)) = row.iter().find_map(|c| match c {
                ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(lo), upper: curvebook_doc::CiBound::Finite(hi) } => {
                    Some((*lo, *hi))
                }
                _ => None,
            }) {
                let est = match row.get(est_idx) {
                    Some(ResultCell::Number { value, .. }) if value.is_finite() => *value,
                    _ if is_log_x && lo > 0.0 && hi > 0.0 => (lo * hi).sqrt(),
                    _ => 0.5 * (lo + hi),
                };
                let label = tab.row_labels.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                push_validated_forest_entry(&mut entries, warnings, label, est, lo, hi);
            }
        }
        if !entries.is_empty() {
            return entries;
        }
    }

    let Some(ds) = resolve_source_data_sheet(graph, doc) else { return Vec::new() };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 3 => {
            let num_cols: Vec<usize> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| v.role != VariableRole::Label && v.role != VariableRole::Categorical)
                .map(|(i, _)| i)
                .collect();
            let lbl_col = graph
                .spec
                .multivariate
                .as_ref()
                .and_then(|m| m.label_column)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Label || v.role == VariableRole::Categorical));
            let (Some(&c_est), Some(&c_lo), Some(&c_hi)) = (num_cols.first(), num_cols.get(1), num_cols.get(2)) else {
                return Vec::new();
            };
            let mut entries = Vec::new();
            for r in 0..mv.row_count() {
                if let (Some(est), Some(s1), Some(s2)) = (
                    mv.variables.get(c_est).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_lo).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                    mv.variables.get(c_hi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()),
                ) {
                    let label = lbl_col
                        .and_then(|lc| mv.variables.get(lc))
                        .and_then(|v| cell_token(v, r))
                        .or_else(|| mv.row_titles.get(r).filter(|s| !s.is_empty()).cloned())
                        .unwrap_or_else(|| format!("Study {}", r + 1));
                    push_validated_forest_entry(&mut entries, warnings, label, est, s1, s2);
                }
            }
            entries
        }
        TableArchetype::Grouped(grouped) if matches!(grouped.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = grouped.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(grouped.subcolumn_mode, &ds_col.cells, &grouped.row_titles, warnings)
        }
        TableArchetype::Column(col) if matches!(col.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = col.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(col.subcolumn_mode, &ds_col.cells, &col.row_titles, warnings)
        }
        TableArchetype::Column(col) if col.data_sets.len() >= 3 => {
            let mut entries = Vec::new();
            for r in 0..col.row_count() {
                if let (Some(est), Some(s1), Some(s2)) = (
                    col.data_sets.first().and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(1).and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                    col.data_sets.get(2).and_then(|d| d.cells.get(r)).and_then(|c| c.first()).and_then(|c| c.active_number()),
                ) {
                    let label = col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                    push_validated_forest_entry(&mut entries, warnings, label, est, s1, s2);
                }
            }
            entries
        }
        TableArchetype::Xy(xy) if matches!(xy.y_subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = xy.data_sets.first() else { return Vec::new() };
            extract_subcolumn_forest_entries(xy.y_subcolumn_mode, &ds_col.cells, &xy.row_titles, warnings)
        }
        _ => Vec::new(),
    }
}

fn compute_pooled_forest_entry(entries: &[ForestEntry], on_log_scale: bool) -> Option<ForestEntry> {
    let mut sum_w = 0.0;
    let mut sum_wy = 0.0;
    for e in entries {
        if on_log_scale && (e.estimate <= 0.0 || e.lower <= 0.0 || e.upper <= 0.0) {
            return None;
        }
        let (y, se) = if on_log_scale {
            (e.estimate.ln(), ((e.upper.ln() - e.lower.ln()) / (2.0 * 1.96)).max(1e-6))
        } else {
            (e.estimate, ((e.upper - e.lower) / (2.0 * 1.96)).max(1e-6))
        };
        let w = 1.0 / (se * se);
        sum_w += w;
        sum_wy += w * y;
    }
    if sum_w <= 0.0 {
        return None;
    }
    let y_pool = sum_wy / sum_w;
    let se_pool = (1.0 / sum_w).sqrt();
    let (est, lo, hi) = if on_log_scale {
        (y_pool.exp(), (y_pool - 1.96 * se_pool).exp(), (y_pool + 1.96 * se_pool).exp())
    } else {
        (y_pool, y_pool - 1.96 * se_pool, y_pool + 1.96 * se_pool)
    };
    Some(ForestEntry { label: String::from("Pooled (FE)"), estimate: est, lower: lo, upper: hi, is_pooled: true })
}
