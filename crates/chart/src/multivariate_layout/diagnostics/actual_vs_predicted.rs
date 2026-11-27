//! Actual (observed) vs predicted diagnostic scatter and Hosmer–Lemeshow calibration plot
//! with deterministic `y = x` identity reference line.
//!
//! Reference: Hosmer, D. W., Lemeshow, S., & Sturdivant, R. X. (2013). *Applied Logistic
//! Regression* (3rd ed.). Wiley.

use curvebook_doc::{
    AnalysisKind, GlmFamily, GlmSpec, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, TableArchetype,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::RichText;

use super::residuals_qq::{ols_residuals, selected_dataset_label_in_tab};
use super::{
    DiagPoint, active_xy_pairs_with_sources, collect_ds_active_point_sources, extract_residuals_cols, find_header_col,
    find_matching_result_where_checked, has_blocked_matching_result, resolve_analysis_source_data_sheet, resolve_source_data_sheet_checked,
    result_cell_num, row_label_dataset_prefix, selected_dataset_prefix_in_row_labels,
};
use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    LegendEntry, PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, draw_legend_entries, layout_and_render_segmented_chrome, path_line,
    path_move, solid_stroke,
};
use crate::xy::format_point_value;

struct ActualVsPredExtracted {
    /// Row-level `(predicted, observed)` scatter points.
    points: Vec<DiagPoint>,
    /// Optional Hosmer–Lemeshow calibration bin points `(expected_rate, observed_rate, bin_label)`.
    calibration_bins: Vec<(f64, f64, String)>,
    x_label: String,
    y_label: String,
    is_probability_scale: bool,
    hl_badge: Option<String>,
}

const GLM_DIAG_TABS: &[&str] = &["glm-linear-diagnostics", "glm-logistic-diagnostics", "glm-poisson-diagnostics"];

fn is_actual_pred_tab(tab: &ResultTab) -> bool {
    if GLM_DIAG_TABS.contains(&tab.id.as_str()) || tab.id == "glm-logistic-calibration" {
        return true;
    }
    if matches!(tab.id.as_str(), "pca-scores" | "pca-standardized-data" | "cox-snell-residuals" | "cox-covariate-residuals") {
        return false;
    }
    if tab.kind == ResultTabKind::Residuals {
        let (_, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
        return x_col.is_some() && (obs_col.is_some() || y_col.is_some());
    }
    if tab.kind == ResultTabKind::Diagnostics {
        let (_, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
        return x_col.is_some() && (obs_col.is_some() || y_col.is_some());
    }
    false
}

fn is_actual_pred_result(res: &ResultSheet) -> bool {
    !matches!(res.analysis.kind, AnalysisKind::BlandAltman(_)) && res.tabs.iter().any(is_actual_pred_tab)
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::ActualVsPredictedPlot`].
pub(in crate::multivariate_layout) fn compile_actual_vs_predicted_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let extracted = extract_actual_vs_pred(graph, doc, &mut scene.warnings);
    let pts: Vec<DiagPoint> = extracted.points.into_iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
    let cal_bins: Vec<(f64, f64, String)> = extracted.calibration_bins.into_iter().filter(|(x, y, _)| x.is_finite() && y.is_finite()).collect();

    if pts.is_empty() && cal_bins.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_actual_vs_predicted", "no observed / predicted coordinate pairs available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in &pts {
        lo = lo.min(p.x).min(p.y);
        hi = hi.max(p.x).max(p.y);
    }
    for &(x, y, _) in &cal_bins {
        lo = lo.min(x).min(y);
        hi = hi.max(x).max(y);
    }
    let (x_bounds, y_bounds) = if extracted.is_probability_scale && lo >= -1e-6 && hi <= 1.0 + 1e-6 {
        ((-0.04, 1.04), (-0.04, 1.04))
    } else {
        let pad = (hi - lo).max(1.0) * 0.08;
        ((lo - pad, hi + pad), (lo - pad, hi + pad))
    };

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = extracted.x_label;
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = extracted.y_label;
    }

    let has_both = !pts.is_empty() && !cal_bins.is_empty();
    let info_id = doc.paired_info_sheet_id(&frame_graph.spec.primary_source_sheet_id);
    let legend_box_width_pt = if has_both {
        crate::scales::auto_legend_box_width_pt(&frame_graph.spec.legend, ["Observations", "Calibration bins"], doc, info_id)
    } else {
        None
    };

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds,
            y_bounds,
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig { has_legend: has_both, legend_box_width_pt, ..PlotMarginConfig::default() },
        },
    );
    let r = scene.plot_rect_pt;

    // Deterministic y = x identity reference line across every active (seg_x, seg_y) pair.
    emit_identity_reference_line(scene, &x_axis, &y_axis);

    let obs_color = graph.spec.palette.color_at(0);
    let cal_color = if has_both { graph.spec.palette.color_at(1) } else { obs_color };

    // 1. Row-level observed vs predicted scatter points.
    for pt in &pts {
        if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.x), y_axis.map_value(pt.y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: if has_both { 3.8 } else { 4.8 },
                fill_color: if has_both { obs_color.with_alpha(150) } else { obs_color },
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.45,
            };
            if let Some(marker) = crate::resolve_overridden_marker_for_col(
                &graph.spec,
                &pt.source.sheet_id,
                pt.source.col,
                pt.source.row,
                Some(pt.source.subcol),
                base_marker,
            ) {
                let center_pt = PointPt::new(px, py);
                emit_marker(&mut scene.primitives, center_pt, &marker, Some(r));
                scene.plotted_points.push(PlottedPointMeta {
                    center_pt,
                    radius_pt: (marker.size_pt * 0.5).max(3.0),
                    source_cell: pt.source.clone(),
                    series_label: pt.series_label.clone(),
                    value_summary: format!("Predicted={}, Observed={}", format_point_value(pt.x), format_point_value(pt.y)),
                });
            }
        }
    }

    // 2. Hosmer–Lemeshow calibration bins (Expected rate vs Observed rate) + connecting polyline.
    if !cal_bins.is_empty() {
        let mut sorted_bins = cal_bins;
        sorted_bins.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)));
        let mut line_path = Path2D::new();
        let mut started = false;
        let mut mapped_bins = Vec::with_capacity(sorted_bins.len());
        for (exp_r, obs_r, lbl) in sorted_bins {
            if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(exp_r), y_axis.map_value(obs_r)) {
                let pt = PointPt::new(px, py);
                if !started {
                    path_move(&mut line_path, pt);
                    started = true;
                } else {
                    path_line(&mut line_path, pt);
                }
                mapped_bins.push((pt, exp_r, obs_r, lbl));
            }
        }
        if mapped_bins.len() >= 2 {
            scene.primitives.push(ScenePrimitive::Path {
                path: line_path,
                fill: None,
                stroke: Some(solid_stroke(cal_color, 1.6, LineDashStyle::Solid)),
                clip: Some(r),
            });
        }
        let bin_marker = MarkerStyle {
            shape: MarkerShape::Diamond,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 5.8,
            fill_color: cal_color,
            stroke_color: Rgba8::BLACK,
            stroke_width_pt: 0.6,
        };
        let sheet_id = graph.spec.primary_source_sheet_id.clone();
        for (idx, (center_pt, exp_r, obs_r, lbl)) in mapped_bins.into_iter().enumerate() {
            emit_marker(&mut scene.primitives, center_pt, &bin_marker, Some(r));
            scene.plotted_points.push(PlottedPointMeta {
                center_pt,
                radius_pt: 3.5,
                source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: idx, col: 0, subcol: 0 },
                series_label: if lbl.is_empty() { String::from("Calibration bin") } else { lbl },
                value_summary: format!("Expected={}, Observed={}", format_point_value(exp_r), format_point_value(obs_r)),
            });
        }
    }

    if let Some(badge) = extracted.hl_badge
        && r.width >= 130.0
    {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(badge),
            anchor_pt: PointPt::new(r.right() - 6.0, r.bottom() - 8.0),
            anchor: TextAnchor::End,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::rgb(45, 52, 64),
        }));
    }

    if has_both {
        let entries = vec![
            LegendEntry {
                label: String::from("Observations"),
                color: obs_color,
                marker: Some(MarkerStyle { fill_color: obs_color, stroke_color: obs_color, size_pt: 4.5, ..MarkerStyle::default() }),
                dash: None,
                is_line: false,
            },
            LegendEntry {
                label: String::from("Calibration bins"),
                color: cal_color,
                marker: Some(MarkerStyle {
                    shape: MarkerShape::Diamond,
                    fill_color: cal_color,
                    stroke_color: Rgba8::BLACK,
                    size_pt: 5.2,
                    ..MarkerStyle::default()
                }),
                dash: Some(LineDashStyle::Solid),
                is_line: true,
            },
        ];
        draw_legend_entries(scene, &frame_graph.spec, &entries, doc);
    }

    Ok(())
}

fn emit_identity_reference_line(scene: &mut ChartScene, x_axis: &crate::xy::SegmentedAxis, y_axis: &crate::xy::SegmentedAxis) {
    let ref_stroke = solid_stroke(Rgba8::rgb(115, 123, 134), 1.1, LineDashStyle::Dashed);
    for seg_x in &x_axis.segments {
        for seg_y in &y_axis.segments {
            let sx_lo = seg_x.domain_min.min(seg_x.domain_max);
            let sx_hi = seg_x.domain_min.max(seg_x.domain_max);
            let sy_lo = seg_y.domain_min.min(seg_y.domain_max);
            let sy_hi = seg_y.domain_min.max(seg_y.domain_max);
            let lo = sx_lo.max(sy_lo);
            let hi = sx_hi.min(sy_hi);
            if !lo.is_finite() || !hi.is_finite() || hi <= lo {
                continue;
            }
            if let (Some(px0), Some(px1), Some(py0), Some(py1)) = (
                seg_x.mapping().map_value(lo, seg_x.span_start_pt, seg_x.span_end_pt),
                seg_x.mapping().map_value(hi, seg_x.span_start_pt, seg_x.span_end_pt),
                seg_y.mapping().map_value(lo, seg_y.span_start_pt, seg_y.span_end_pt),
                seg_y.mapping().map_value(hi, seg_y.span_start_pt, seg_y.span_end_pt),
            ) {
                let cell_clip = RectPt::new(
                    seg_x.span_start_pt.min(seg_x.span_end_pt),
                    seg_y.span_start_pt.min(seg_y.span_end_pt),
                    (seg_x.span_end_pt - seg_x.span_start_pt).abs(),
                    (seg_y.span_end_pt - seg_y.span_start_pt).abs(),
                );
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::line(PointPt::new(px0, py0), PointPt::new(px1, py1)),
                    fill: None,
                    stroke: Some(ref_stroke),
                    clip: Some(cell_clip),
                });
            }
        }
    }
}

fn extract_calibration_bins(tab: &ResultTab) -> Vec<(f64, f64, String)> {
    let exp_rate = find_header_col(&tab.column_headers, |h| h.contains("expected") && h.contains("rate"))
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("expected") && h.contains("prob")));
    let obs_rate = find_header_col(&tab.column_headers, |h| h.contains("observed") && h.contains("rate"))
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("observed") && h.contains("prob")));
    let (exp_col, obs_col) = match (exp_rate, obs_rate) {
        (Some(e), Some(o)) => (Some(e), Some(o)),
        _ => (find_header_col(&tab.column_headers, |h| h.contains("expected")), find_header_col(&tab.column_headers, |h| h.contains("observed"))),
    };
    let (Some(ec), Some(oc)) = (exp_col, obs_col) else {
        return Vec::new();
    };
    tab.rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            let ex = result_cell_num(row.get(ec)?)?;
            let ob = result_cell_num(row.get(oc)?)?;
            let lbl = tab.row_labels.get(i).cloned().unwrap_or_else(|| format!("Bin {}", i + 1));
            Some((ex, ob, lbl))
        })
        .collect()
}

fn extract_actual_pred_tab_points(res: &ResultSheet, tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> (Vec<DiagPoint>, String, String) {
    let (ds_col, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
    let target_label = selected_dataset_label_in_tab(res, tab, ds_col, graph, doc);
    let legacy_row_prefix = if ds_col.is_none() { selected_dataset_prefix_in_row_labels(res, tab, graph, doc) } else { None };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
    let fallback_sheet_id = ds_opt.map(|d| d.id.clone()).unwrap_or_else(|| graph.spec.primary_source_sheet_id.clone());

    let x_hdr = x_col.and_then(|c| tab.column_headers.get(c)).cloned().unwrap_or_else(|| String::from("Predicted Value"));
    let y_hdr = obs_col.and_then(|c| tab.column_headers.get(c)).cloned().unwrap_or_else(|| String::from("Observed Value"));

    let pairs: Vec<(usize, f64, f64)> = tab
        .rows
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
            {
                return None;
            }
            if let Some(prefix) = legacy_row_prefix.as_deref() {
                let row_lbl = tab.row_labels.get(i).map(String::as_str).unwrap_or("");
                if !matches!(row_label_dataset_prefix(row_lbl), Some((p, _)) if p == prefix) {
                    return None;
                }
            }
            let (pred, obs) = match (x_col, obs_col, y_col) {
                (Some(xc), Some(oc), _) => (result_cell_num(row.get(xc)?)?, result_cell_num(row.get(oc)?)?),
                (Some(xc), None, Some(yc)) => {
                    let fitted = result_cell_num(row.get(xc)?)?;
                    let resid = result_cell_num(row.get(yc)?)?;
                    (fitted, fitted + resid)
                }
                _ => return None,
            };
            (pred.is_finite() && obs.is_finite()).then_some((i, pred, obs))
        })
        .collect();

    if pairs.is_empty() {
        return (Vec::new(), x_hdr, y_hdr);
    }

    let source_col = match &res.analysis.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => glm.response_column,
        _ => ds_idx,
    };
    let (resolved_sources, series_label) = if let Some(ds) = ds_opt {
        let (all_rep_sources, lbl) = collect_ds_active_point_sources(ds, source_col, true);
        if all_rep_sources.len() == pairs.len() {
            (Some(all_rep_sources), lbl)
        } else {
            let (row_sources, lbl_row) = collect_ds_active_point_sources(ds, source_col, false);
            if row_sources.len() == pairs.len() { (Some(row_sources), lbl_row) } else { (None, lbl) }
        }
    } else {
        (None, target_label.clone().or_else(|| legacy_row_prefix.clone()).unwrap_or_else(|| graph.spec.title.clone()))
    };
    let resolved_label = if series_label.is_empty() { graph.spec.title.clone() } else { series_label };
    let filtered_by_dataset = target_label.is_some() || legacy_row_prefix.is_some();

    let pts = pairs
        .into_iter()
        .enumerate()
        .map(|(k, (tab_row_idx, x, y))| {
            let fallback_row = if filtered_by_dataset {
                tab.row_labels.get(tab_row_idx).and_then(|l| row_label_dataset_prefix(l).map(|(_, idx)| idx)).unwrap_or(k)
            } else {
                tab_row_idx
            };
            let source = resolved_sources.as_ref().and_then(|v| v.get(k)).cloned().unwrap_or(SourceCellId {
                sheet_id: fallback_sheet_id.clone(),
                row: fallback_row,
                col: source_col,
                subcol: 0,
            });
            DiagPoint { x, y, label: tab.row_labels.get(tab_row_idx).cloned(), source, series_label: resolved_label.clone() }
        })
        .collect();

    (pts, x_hdr, y_hdr)
}

fn extract_actual_vs_pred(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> ActualVsPredExtracted {
    let empty = ActualVsPredExtracted {
        points: Vec::new(),
        calibration_bins: Vec::new(),
        x_label: String::from("Predicted Value"),
        y_label: String::from("Observed Value"),
        is_probability_scale: false,
        hl_badge: None,
    };

    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_actual_pred_result) {
        let is_logistic = matches!(
            res.analysis.kind,
            AnalysisKind::GeneralizedLinearModel(GlmSpec { family: GlmFamily::Logistic, .. }) | AnalysisKind::SimpleLogisticRegression(_)
        );
        let calibration_bins = res.tabs.iter().find(|t| t.id == "glm-logistic-calibration").map(extract_calibration_bins).unwrap_or_default();
        let hl_badge =
            res.scalars.iter().find(|s| s.key == "Hosmer-Lemeshow P" && s.value.is_finite()).map(|s| format!("Hosmer-Lemeshow P = {:.4}", s.value));

        let row_tab = res
            .tabs
            .iter()
            .find(|t| GLM_DIAG_TABS.contains(&t.id.as_str()))
            .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals && is_actual_pred_tab(t)))
            .or_else(|| {
                res.tabs.iter().find(|t| t.kind == ResultTabKind::Diagnostics && t.id != "glm-logistic-calibration" && is_actual_pred_tab(t))
            });

        let (points, x_hdr, y_hdr) = match row_tab {
            Some(tab) => extract_actual_pred_tab_points(res, tab, graph, doc),
            None => (
                Vec::new(),
                if !calibration_bins.is_empty() { String::from("Expected Rate") } else { String::from("Predicted Value") },
                if !calibration_bins.is_empty() { String::from("Observed Rate") } else { String::from("Observed Value") },
            ),
        };
        let prob_scale = is_logistic || (!calibration_bins.is_empty() && points.is_empty());
        return ActualVsPredExtracted { points, calibration_bins, x_label: x_hdr, y_label: y_hdr, is_probability_scale: prob_scale, hl_badge };
    }

    if has_blocked_matching_result(graph, doc, is_actual_pred_result) {
        return empty;
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else {
        return empty;
    };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, ds_idx);
            let raw: Vec<(f64, f64)> = pairs.iter().map(|(x, y, _, _)| (*x, *y)).collect();
            let pts = ols_residuals(&raw)
                .into_iter()
                .zip(pairs)
                .map(|((fitted, _), (_, obs_y, source, series_label))| DiagPoint { x: fitted, y: obs_y, label: None, source, series_label })
                .collect();
            ActualVsPredExtracted { points: pts, ..empty }
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let x_title = mv.variables.get(xi).map_or_else(|| String::from("Predicted Value"), |v| v.title.clone());
            let y_title = mv.variables.get(yi).map_or_else(|| String::from("Observed Value"), |v| v.title.clone());
            let raw: Vec<(f64, f64, SourceCellId, String)> = (0..mv.row_count())
                .filter_map(|r| {
                    let x = mv.variables.get(xi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    let y = mv.variables.get(yi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    Some((x, y, SourceCellId { sheet_id: ds.id.clone(), row: r, col: yi, subcol: 0 }, y_title.clone()))
                })
                .collect();
            let xl = x_title.to_ascii_lowercase();
            let yl = y_title.to_ascii_lowercase();
            if xl.contains("pred") || xl.contains("fit") || xl.contains("expect") || yl.contains("obs") || yl.contains("actual") {
                let pts = raw.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label }).collect();
                return ActualVsPredExtracted { points: pts, x_label: x_title, y_label: y_title, ..empty };
            }
            let pairs: Vec<(f64, f64)> = raw.iter().map(|(x, y, _, _)| (*x, *y)).collect();
            let pts = ols_residuals(&pairs)
                .into_iter()
                .zip(raw)
                .map(|((fitted, _), (_, obs_y, source, series_label))| DiagPoint { x: fitted, y: obs_y, label: None, source, series_label })
                .collect();
            ActualVsPredExtracted { points: pts, ..empty }
        }
        _ => empty,
    }
}
