//! ROC curve (sensitivity vs 1-specificity + AUC) and forest plot diagnostic builders.

use curvebook_doc::{
    AnalysisKind, FittedCurveOverlay, GlmFamily, GlmSpec, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, VariableRole,
};
use curvebook_math::dist::norm_two_sided_isf;
use curvebook_math::{AxisScale, ConfidenceLevel};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::roc::{RocCiMethod, roc_curve_analysis};
use curvebook_text::{RichText, parse_markup};

use super::super::multivariate::{cell_token, column_categorical_levels};
use super::{
    find_header_col, find_matching_result_where_checked, has_blocked_matching_result, raw_matching_results, resolve_source_data_sheet_checked,
    result_cell_num,
};
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
    let mut last_px = r.right();
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
        last_px = px;
    }
    if started {
        path_line(&mut area_p, PointPt::new(last_px, r.bottom()));
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
    matches!(norm.as_str(), "roc_auc" | "auc" | "area_under_roc_curve_(auc)")
}

fn find_roc_coords_tab(res: &ResultSheet) -> Option<&ResultTab> {
    res.tabs.iter().find(|t| t.id == "glm-logistic-roc")
}

fn extract_roc_points_from_tab(tab: &ResultTab) -> Vec<(f64, f64)> {
    let Some(fpr_col) = find_header_col(&tab.column_headers, |h| h.contains("1 - specificity") || h.contains("1-specificity") || h == "fpr") else {
        return Vec::new();
    };
    let Some(tpr_col) = find_header_col(&tab.column_headers, |h| h.contains("sensitivity") || h == "tpr") else {
        return Vec::new();
    };
    let mut pts: Vec<(f64, f64)> = tab
        .rows
        .iter()
        .filter_map(|row| {
            let fpr = result_cell_num(row.get(fpr_col)?)?;
            let tpr = result_cell_num(row.get(tpr_col)?)?;
            Some((fpr, tpr))
        })
        .collect();
    if pts.len() >= 2 && pts.first().is_some_and(|a| pts.last().is_some_and(|b| a.0 > b.0)) {
        pts.reverse();
    }
    pts
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
            .or_else(|| find_matching_result_where_checked(graph, doc, warnings, |r| find_roc_coords_tab(r).is_some()))
            .or_else(|| find_matching_result_where_checked(graph, doc, warnings, |r| is_roc_or_logistic_analysis(&r.analysis.kind)));

    if let Some(res) = roc_res {
        let roc_curves: Vec<&FittedCurveOverlay> = res.fitted_curves.iter().filter(|fc| fc.equation_id.eq_ignore_ascii_case("ROC")).collect();
        if let Some(curve) = roc_curves.iter().copied().find(|fc| fc.data_set_index == ds_idx).or_else(|| roc_curves.first().copied()) {
            let mut pts: Vec<(f64, f64)> = curve.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).map(|p| (p.x, p.y)).collect();
            if pts.len() >= 2 {
                if pts.first().is_some_and(|a| pts.last().is_some_and(|b| a.0 > b.0)) {
                    pts.reverse();
                }
                let auc = resolve_roc_auc_scalar(res, curve.data_set_index, roc_curves.len() == 1).unwrap_or_else(|| trapezoidal_auc(&pts));
                return Some((pts, auc));
            }
        }
        if let Some(tab) = find_roc_coords_tab(res) {
            let pts = extract_roc_points_from_tab(tab);
            if pts.len() >= 2 {
                let auc = resolve_roc_auc_scalar(res, ds_idx, true).unwrap_or_else(|| trapezoidal_auc(&pts));
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

    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
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

struct ForestExtracted {
    entries: Vec<ForestEntry>,
    is_covariate_regression: bool,
    is_ratio_scale: bool,
    confidence: ConfidenceLevel,
    /// `true` when `confidence` is an assumed 95% (raw data tables carry no interval level).
    confidence_assumed: bool,
}

pub(in crate::multivariate_layout) fn compile_forest_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let extracted = extract_forest_entries(graph, doc, &mut scene.warnings);
    let mut entries = extracted.entries;
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
            && (extracted.is_ratio_scale
                || graph.spec.x_axis.title.contains("OR")
                || graph.spec.x_axis.title.contains("HR")
                || graph.spec.x_axis.title.contains("Ratio")))
    {
        1.0
    } else {
        0.0
    };

    if graph.spec.forest_show_pooled_diamond && entries.len() >= 2 && !entries.iter().any(|e| e.is_pooled) {
        if extracted.is_covariate_regression {
            scene.warnings.push(ChartWarning::new(
                "pooled_diamond_suppressed_for_covariates",
                "fixed-effect pooled summary diamond is not applicable to heterogeneous regression covariates and was omitted",
            ));
        } else if let Some(mut pooled) = compute_pooled_forest_entry(&entries, is_log || ref_x == 1.0, extracted.confidence, &mut scene.warnings) {
            // Raw data tables record no confidence level, so the pooled standard errors assume
            // two-sided 95% study intervals. State that on the figure itself rather than as a
            // warning, which would fire on every ordinary raw-table forest plot (`R0.36`).
            if extracted.confidence_assumed {
                pooled.label = String::from("Pooled (FE, 95% CIs assumed)");
            }
            entries.push(pooled);
        }
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

fn is_covariate_regression_kind(kind: &AnalysisKind) -> bool {
    matches!(
        kind,
        AnalysisKind::GeneralizedLinearModel(_)
            | AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { .. })
            | AnalysisKind::SimpleLogisticRegression(_)
            | AnalysisKind::LinearRegression(_)
            | AnalysisKind::NonlinearRegression(_)
    )
}

fn is_forest_result_sheet(res: &ResultSheet) -> bool {
    is_covariate_regression_kind(&res.analysis.kind)
        || res.tabs.iter().any(|t| {
            (t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.kind == ResultTabKind::MainSummary)
                && t.rows.iter().any(|r| r.iter().any(|c| matches!(c, ResultCell::ConfidenceInterval { .. })))
        })
}

fn is_excluded_forest_est_header(lower: &str) -> bool {
    lower.contains("ci")
        || lower.contains("likelihood")
        || lower.contains("p value")
        || lower.contains("p-value")
        || lower.contains("wald")
        || lower.contains("schoenfeld")
        || lower == "se"
        || lower.contains("std. error")
        || lower.contains("asymptotic se")
}

fn find_forest_ci_col(tab: &ResultTab, preferred_idx: Option<usize>) -> Option<usize> {
    let has_ci_cell = |col: usize| tab.rows.iter().any(|r| matches!(r.get(col), Some(ResultCell::ConfidenceInterval { .. })));
    if let Some(pref) = preferred_idx
        && has_ci_cell(pref)
    {
        return Some(pref);
    }
    let ratio_ci = tab.column_headers.iter().enumerate().position(|(idx, h)| {
        let l = h.to_ascii_lowercase();
        has_ci_cell(idx) && (l.contains("(hr)") || l.contains("(or)") || l.contains("(rr)") || l.contains("ratio"))
    });
    if ratio_ci.is_some() {
        return ratio_ci;
    }
    (0..tab.column_headers.len())
        .find(|&idx| has_ci_cell(idx))
        .or_else(|| tab.rows.iter().find_map(|r| r.iter().position(|c| matches!(c, ResultCell::ConfidenceInterval { .. }))))
}

/// Select `(estimate_col, ci_col, is_ratio_scale)` for a parameter/summary result tab so ratio
/// columns (`Hazard Ratio`, `Odds Ratio`, `Rate Ratio`) are preferred over log-scale `"beta"`
/// whenever paired with a ratio-scale CI (`GAP-02-02`).
fn select_forest_columns(tab: &ResultTab) -> (Option<usize>, Option<usize>, bool) {
    let ratio_col = tab.column_headers.iter().position(|h| {
        let l = h.to_ascii_lowercase();
        !is_excluded_forest_est_header(&l) && (l.contains("ratio") || l == "hr" || l == "or" || l == "rr")
    });
    if let Some(rc) = ratio_col {
        let ci_col = find_forest_ci_col(tab, Some(rc + 1));
        let ci_is_beta = ci_col.and_then(|c| tab.column_headers.get(c)).is_some_and(|h| {
            let l = h.to_ascii_lowercase();
            l.contains("beta") || l.contains("log(")
        });
        if !ci_is_beta {
            return (Some(rc), ci_col, true);
        }
    }
    let ci_col = find_forest_ci_col(tab, None);
    let est_col = tab
        .column_headers
        .iter()
        .position(|h| {
            let l = h.to_ascii_lowercase();
            !is_excluded_forest_est_header(&l) && (l.contains("estimate") || l.contains("best-fit") || l.contains("diff") || l.contains("effect"))
        })
        .or_else(|| {
            tab.column_headers.iter().position(|h| {
                let l = h.to_ascii_lowercase();
                !is_excluded_forest_est_header(&l) && (l.contains("beta") || l.contains("coef"))
            })
        })
        .or_else(|| (ci_col != Some(0)).then_some(0));
    (est_col, ci_col, false)
}

fn extract_forest_entries(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> ForestExtracted {
    let empty = ForestExtracted {
        entries: Vec::new(),
        is_covariate_regression: false,
        is_ratio_scale: false,
        confidence: ConfidenceLevel::DEFAULT_95,
        confidence_assumed: true,
    };
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_forest_result_sheet)
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::ParametersAndConfidenceIntervals || t.kind == ResultTabKind::MainSummary)
    {
        let mut entries = Vec::new();
        let is_log_x = is_log_like(graph.spec.x_axis.segments.first().map_or(AxisScale::Linear, |s| s.scale));
        let (est_col_opt, ci_col_opt, is_ratio_scale) = select_forest_columns(tab);
        for (r, row) in tab.rows.iter().enumerate() {
            let ci_pair = ci_col_opt
                .and_then(|ci_c| match row.get(ci_c) {
                    Some(ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(lo), upper: curvebook_doc::CiBound::Finite(hi) }) => {
                        Some((*lo, *hi))
                    }
                    _ => None,
                })
                .or_else(|| {
                    row.iter().find_map(|c| match c {
                        ResultCell::ConfidenceInterval { lower: curvebook_doc::CiBound::Finite(lo), upper: curvebook_doc::CiBound::Finite(hi) } => {
                            Some((*lo, *hi))
                        }
                        _ => None,
                    })
                });
            if let Some((lo, hi)) = ci_pair {
                let est = match est_col_opt.and_then(|ec| row.get(ec)) {
                    Some(ResultCell::Number { value, .. }) if value.is_finite() => *value,
                    _ if (is_log_x || is_ratio_scale) && lo > 0.0 && hi > 0.0 => (lo * hi).sqrt(),
                    _ => 0.5 * (lo + hi),
                };
                let label = tab.row_labels.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Study {}", r + 1));
                push_validated_forest_entry(&mut entries, warnings, label, est, lo, hi);
            }
        }
        if !entries.is_empty() || is_covariate_regression_kind(&res.analysis.kind) {
            return ForestExtracted {
                entries,
                is_covariate_regression: is_covariate_regression_kind(&res.analysis.kind),
                is_ratio_scale,
                confidence: res.analysis.confidence_level,
                confidence_assumed: false,
            };
        }
    }

    // Never silently fall back to raw MultipleVariables columns (e.g. Time/Status/Dose) when a
    // linked regression/survival/forest result is pending recalculation or failed (`GAP-02-02`/`GAP-06-03`).
    if has_blocked_matching_result(graph, doc, is_forest_result_sheet) {
        return empty;
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else { return empty };
    let entries = match &ds.archetype {
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
                return empty;
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
            let Some(ds_col) = grouped.data_sets.first() else { return empty };
            extract_subcolumn_forest_entries(grouped.subcolumn_mode, &ds_col.cells, &grouped.row_titles, warnings)
        }
        TableArchetype::Column(col) if matches!(col.subcolumn_mode, SubcolumnMode::MeanLowHigh | SubcolumnMode::UpperLowerLimits) => {
            let Some(ds_col) = col.data_sets.first() else { return empty };
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
            let Some(ds_col) = xy.data_sets.first() else { return empty };
            extract_subcolumn_forest_entries(xy.y_subcolumn_mode, &ds_col.cells, &xy.row_titles, warnings)
        }
        _ => Vec::new(),
    };
    ForestExtracted {
        entries,
        is_covariate_regression: false,
        is_ratio_scale: false,
        confidence: ConfidenceLevel::DEFAULT_95,
        confidence_assumed: true,
    }
}

/// Inverse-variance fixed-effect pooled estimate for the forest-plot summary diamond.
///
/// Follows Borenstein, Hedges, Higgins & Rothstein (2009), *Introduction to Meta-Analysis*,
/// Wiley, ch. 11 "Fixed-Effect Model": weights `W_i = 1 / V_i`, pooled
/// `M = sum(W_i Y_i) / sum(W_i)`, `V_M = 1 / sum(W_i)`, and limits `M -/+ z_{1-alpha/2} sqrt(V_M)`.
/// Each study's standard error is recovered from its reported symmetric two-sided interval at
/// `confidence` as `SE_i = (upper - lower) / (2 z_{1-alpha/2})`; ratio measures (odds, hazard,
/// risk ratios) are pooled on the natural-log scale with `SE_i = (ln upper - ln lower) / (2 z)` and
/// back-transformed (Borenstein et al. 2009, ch. 5 "Effect Sizes Based on Binary Data").
/// Returns `None` (with a `pooled_diamond_degenerate_ci` warning for zero-width intervals) when any
/// entry is non-positive on the log scale or has a non-finite or zero-width interval.
fn compute_pooled_forest_entry(
    entries: &[ForestEntry],
    on_log_scale: bool,
    confidence: ConfidenceLevel,
    warnings: &mut Vec<ChartWarning>,
) -> Option<ForestEntry> {
    let z = norm_two_sided_isf(confidence.alpha()).unwrap_or(1.959_963_984_540_054);
    let mut sum_w = 0.0;
    let mut sum_wy = 0.0;
    for e in entries {
        if on_log_scale && (e.estimate <= 0.0 || e.lower <= 0.0 || e.upper <= 0.0) {
            return None;
        }
        let (y, half_width) =
            if on_log_scale { (e.estimate.ln(), 0.5 * (e.upper.ln() - e.lower.ln())) } else { (e.estimate, 0.5 * (e.upper - e.lower)) };
        if !y.is_finite() || !half_width.is_finite() || half_width <= 1e-12 {
            warnings.push(ChartWarning::new(
                "pooled_diamond_degenerate_ci",
                format!("omitted pooled fixed-effect diamond: entry '{}' has zero-width or non-finite CI", e.label),
            ));
            return None;
        }
        let se = half_width / z;
        let w = 1.0 / (se * se);
        if !w.is_finite() || w <= 0.0 {
            return None;
        }
        sum_w += w;
        sum_wy += w * y;
    }
    if !sum_w.is_finite() || sum_w <= 0.0 {
        return None;
    }
    let y_pool = sum_wy / sum_w;
    let se_pool = (1.0 / sum_w).sqrt();
    let (est, lo, hi) = if on_log_scale {
        (y_pool.exp(), (y_pool - z * se_pool).exp(), (y_pool + z * se_pool).exp())
    } else {
        (y_pool, y_pool - z * se_pool, y_pool + z * se_pool)
    };
    if !est.is_finite() || !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    Some(ForestEntry { label: String::from("Pooled (FE)"), estimate: est, lower: lo, upper: hi, is_pooled: true })
}

#[cfg(test)]
mod tests {
    use curvebook_math::ConfidenceLevel;

    use super::{ForestEntry, compute_pooled_forest_entry};

    fn entry(label: &str, estimate: f64, lower: f64, upper: f64) -> ForestEntry {
        ForestEntry { label: String::from(label), estimate, lower, upper, is_pooled: false }
    }

    #[test]
    fn pooled_fixed_effect_matches_inverse_variance_hand_derivation() {
        // z = qnorm(0.975) = 1.959963984540054 (R 4.5.2).
        let z = 1.959_963_984_540_054;
        // Two studies with SE_1 = 1 and SE_2 = 0.5, reported as symmetric 95% intervals Y_i -/+ z SE_i.
        // Borenstein et al. (2009) ch. 11: W = (1, 4), M = (1*2 + 4*1) / 5 = 1.2, SE_M = sqrt(1/5).
        // R 4.5.2: w <- c(1, 4); y <- c(2, 1); M <- sum(w*y)/sum(w); se <- sqrt(1/sum(w))
        //          c(M, M - qnorm(0.975)*se, M + qnorm(0.975)*se) -> 1.2 0.323477459423419 2.076522540576581
        let linear = [entry("S1", 2.0, 2.0 - z, 2.0 + z), entry("S2", 1.0, 1.0 - 0.5 * z, 1.0 + 0.5 * z)];
        let mut warnings = Vec::new();
        let pooled = compute_pooled_forest_entry(&linear, false, ConfidenceLevel::DEFAULT_95, &mut warnings).expect("pooled entry");
        assert!(warnings.is_empty());
        assert!(pooled.is_pooled);
        // Tolerance 1e-12: only floating-point round-off separates the closed form from the pooled sums.
        for (got, want) in [(pooled.estimate, 1.2), (pooled.lower, 0.323_477_459_423_419), (pooled.upper, 2.076_522_540_576_581)] {
            assert!((got - want).abs() <= 1e-12, "linear pooled {got} != {want}");
        }

        // Ratio scale: OR_1 = 1 (log 0, SE 1) and OR_2 = 2 (log 2, SE 0.5) reported as exp(log OR -/+ z SE).
        // Pooled log OR = (0 + 4 ln 2) / 5 = 0.8 ln 2, so OR = 2^0.8.
        // R 4.5.2: yl <- c(0, log(2)); Ml <- sum(w*yl)/sum(w)
        //          exp(c(Ml, Ml - qnorm(0.975)*se, Ml + qnorm(0.975)*se)) -> 1.741101126592248 0.724694714592503 4.183048491978276
        let ln2 = std::f64::consts::LN_2;
        let ratio = [entry("S1", 1.0, (-z).exp(), z.exp()), entry("S2", 2.0, (ln2 - 0.5 * z).exp(), (ln2 + 0.5 * z).exp())];
        let pooled_or = compute_pooled_forest_entry(&ratio, true, ConfidenceLevel::DEFAULT_95, &mut warnings).expect("pooled OR");
        for (got, want) in
            [(pooled_or.estimate, 1.741_101_126_592_248), (pooled_or.lower, 0.724_694_714_592_503), (pooled_or.upper, 4.183_048_491_978_276)]
        {
            assert!((got - want).abs() <= 1e-12, "ratio pooled {got} != {want}");
        }
    }
}
