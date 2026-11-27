//! Regression residual scatter and normal Q-Q diagnostic plot builders.

use curvebook_doc::{AnalysisKind, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, TableArchetype};
use curvebook_math::dist::norm_ppf;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::common::quartiles_hinge;

use super::{
    DiagPoint, active_xy_pairs_with_sources, collect_ds_active_values_with_sources, extract_residuals_tab_points, find_header_col,
    find_matching_result_where_checked, has_blocked_matching_result, is_ds_pre_summarized, resolve_analysis_source_data_sheet,
    resolve_source_data_sheet_checked, result_cell_num,
};
use crate::markers::emit_marker;
use crate::scene::{ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, RectPt, ScenePrimitive, SourceCellId};
use crate::survival_cat::{PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, solid_stroke};
use crate::xy::format_point_value;

// 2f. Residual Plot (Fitted vs Residual + Zero Baseline)

pub(in crate::multivariate_layout) fn compile_residual_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let pts = extract_residual_points(graph, doc, &mut scene.warnings);
    if pts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_residuals", "no fitted/residual pairs available"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y).min(0.0), b.max(p.y).max(0.0)));
    let x_pad = (x_max - x_min).max(1.0) * 0.08;
    let y_pad = (y_max - y_min).max(1.0) * 0.12;
    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("Fitted Value");
    }
    if frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("Residual");
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (x_min - x_pad, x_max + x_pad),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    if let Some((_, y0)) = y_axis.map_value(0.0) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, y0), PointPt::new(r.right(), y0)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (xv, yv) = (pt.x, pt.y);
        if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(xv), y_axis.map_value(yv)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
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
                    value_summary: format!("Fitted={}, Residual={}", format_point_value(xv), format_point_value(yv)),
                });
            }
        }
    }
    Ok(())
}

fn extract_residual_points(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<DiagPoint> {
    let has_resid_tab =
        |r: &ResultSheet| !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals);
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, has_resid_tab)
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let pts = extract_residuals_tab_points(res, tab, graph, doc);
        if !pts.is_empty() {
            return pts;
        }
    }
    if has_blocked_matching_result(graph, doc, has_resid_tab) {
        return Vec::new();
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else { return Vec::new() };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let pairs_with_sources: Vec<(f64, f64, SourceCellId, String)> = match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, ds_idx);
            if !pairs.is_empty() && xy.y_subcolumn_mode.is_pre_summarized() {
                warnings.push(ChartWarning::new(
                    "pre_summarized_summary_only",
                    "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
                ));
            }
            pairs
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            let y_lbl = mv.variables.get(yi).map_or_else(|| String::from("Residual"), |v| v.title.clone());
            let raw: Vec<(f64, f64, SourceCellId, String)> = (0..mv.row_count())
                .filter_map(|r| {
                    let x = mv.variables.get(xi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    let y = mv.variables.get(yi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    Some((x, y, SourceCellId { sheet_id: ds.id.clone(), row: r, col: yi, subcol: 0 }, y_lbl.clone()))
                })
                .collect();
            let y_title_low = y_lbl.to_ascii_lowercase();
            if y_title_low.contains("resid") {
                return raw.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y, label: None, source, series_label }).collect();
            }
            raw
        }
        TableArchetype::Column(_) | TableArchetype::Grouped(_) => {
            let (val_sources, lbl) = collect_ds_active_values_with_sources(ds, ds_idx);
            if !val_sources.is_empty() && is_ds_pre_summarized(ds) {
                warnings.push(ChartWarning::new(
                    "pre_summarized_summary_only",
                    "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
                ));
            }
            let vals: Vec<f64> = val_sources.iter().map(|(v, _)| *v).collect();
            let mean = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / (vals.len() as f64) };
            return val_sources
                .into_iter()
                .enumerate()
                .map(|(i, (v, source))| DiagPoint { x: (i + 1) as f64, y: v - mean, label: None, source, series_label: lbl.clone() })
                .collect();
        }
        _ => Vec::new(),
    };
    let raw_pairs: Vec<(f64, f64)> = pairs_with_sources.iter().map(|(x, y, _, _)| (*x, *y)).collect();
    ols_residuals(&raw_pairs)
        .into_iter()
        .zip(pairs_with_sources)
        .map(|((fitted, resid), (_, _, source, series_label))| DiagPoint { x: fitted, y: resid, label: None, source, series_label })
        .collect()
}

fn ols_residuals(pairs: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let n = pairs.len();
    if n < 2 {
        return pairs.to_vec();
    }
    let nf = n as f64;
    let mx = pairs.iter().map(|p| p.0).sum::<f64>() / nf;
    let my = pairs.iter().map(|p| p.1).sum::<f64>() / nf;
    let sxx = pairs.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum::<f64>();
    let sxy = pairs.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>();
    let slope = if sxx > 1e-12 { sxy / sxx } else { 0.0 };
    let intercept = my - slope * mx;
    pairs
        .iter()
        .map(|&(x, y)| {
            let y_hat = intercept + slope * x;
            (y_hat, y - y_hat)
        })
        .collect()
}

// 2g. Normal Q-Q Plot (Theoretical Normal Quantiles vs Sorted Observations)

pub(in crate::multivariate_layout) fn compile_normal_qq_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let mut pts = extract_qq_points(graph, doc, &mut scene.warnings);
    pts.retain(|pt| pt.x.is_finite() && pt.y.is_finite());
    if pts.len() < 3 {
        scene.warnings.push(ChartWarning::new("insufficient_qq_sample", "at least 3 finite observations required for Normal Q-Q plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
    let x_pad = (x_max - x_min).max(1.0) * 0.1;
    let y_pad = (y_max - y_min).max(1.0) * 0.1;
    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("Theoretical Normal Quantiles");
    }
    if frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = String::from("Sample Quantiles");
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (x_min - x_pad, x_max + x_pad),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    let mut xs: Vec<f64> = pts.iter().map(|p| p.x).collect();
    let mut ys: Vec<f64> = pts.iter().map(|p| p.y).collect();
    xs.sort_by(f64::total_cmp);
    ys.sort_by(f64::total_cmp);
    if let (Some((qx1, _, qx3)), Some((qy1, _, qy3))) = (quartiles_hinge(&xs), quartiles_hinge(&ys))
        && (qx3 - qx1).abs() > 1e-12
    {
        let slope = (qy3 - qy1) / (qx3 - qx1);
        let intercept = qy1 - slope * qx1;
        for seg_x in &x_axis.segments {
            for seg_y in &y_axis.segments {
                let sx0 = seg_x.domain_min.min(seg_x.domain_max);
                let sx1 = seg_x.domain_min.max(seg_x.domain_max);
                let sy0 = intercept + slope * sx0;
                let sy1 = intercept + slope * sx1;
                if let (Some(px0), Some(px1), Some(py0), Some(py1)) = (
                    seg_x.mapping().map_value(sx0, seg_x.span_start_pt, seg_x.span_end_pt),
                    seg_x.mapping().map_value(sx1, seg_x.span_start_pt, seg_x.span_end_pt),
                    seg_y.mapping().map_value(sy0, seg_y.span_start_pt, seg_y.span_end_pt),
                    seg_y.mapping().map_value(sy1, seg_y.span_start_pt, seg_y.span_end_pt),
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
                        stroke: Some(solid_stroke(Rgba8::rgb(120, 128, 138), 1.1, LineDashStyle::Dashed)),
                        clip: Some(cell_clip),
                    });
                }
            }
        }
    }

    let color = graph.spec.palette.color_at(0);
    for pt in &pts {
        let (z, y) = (pt.x, pt.y);
        if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(z), y_axis.map_value(y)) {
            let base_marker = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.8,
                fill_color: color,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
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
                    value_summary: format!("Theoretical={}, Sample={}", format_point_value(z), format_point_value(y)),
                });
            }
        }
    }
    Ok(())
}

pub(super) fn selected_dataset_label_in_tab(
    res: &ResultSheet,
    tab: &ResultTab,
    ds_col: Option<usize>,
    graph: &GraphSheet,
    doc: &ProjectDocument,
) -> Option<String> {
    let dc = ds_col?;
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let mut seen: Vec<String> = Vec::new();
    for row in &tab.rows {
        if let Some(ResultCell::Text(s)) = row.get(dc)
            && !s.is_empty()
            && !seen.iter().any(|existing| existing == s)
        {
            seen.push(s.clone());
        }
    }
    if seen.len() <= 1 {
        return seen.into_iter().next();
    }
    if let Some(ds) = resolve_analysis_source_data_sheet(res, graph, doc) {
        let desired_title = match &ds.archetype {
            TableArchetype::Column(col) => col.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::Grouped(g) => g.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::Xy(xy) => xy.data_sets.get(ds_idx).map(|d| d.title.as_str()),
            TableArchetype::MultipleVariables(mv) => mv.variables.get(ds_idx).map(|v| v.title.as_str()),
            _ => None,
        };
        if let Some(title) = desired_title
            && seen.iter().any(|s| s == title)
        {
            return Some(title.to_owned());
        }
    }
    seen.get(ds_idx).or_else(|| seen.first()).cloned()
}

/// Sorted values paired with Blom normal scores for a normal Q-Q plot.
///
/// The `i`-th smallest of `n` values (1-based) is plotted against `Phi^{-1}((i - 3/8) / (n + 1/4))`,
/// the plotting positions of Blom, G. (1958), *Statistical Estimates and Transformed Beta-Variables*,
/// Wiley — the same positions the nonlinear-fit residual tab uses
/// (`curvebook_fit` `append_nonlinear_residuals_tab`), so raw and result-sourced Q-Q plots agree.
fn values_to_qq_diag_points(mut vals: Vec<(f64, SourceCellId, String)>) -> Vec<DiagPoint> {
    vals.retain(|(v, _, _)| v.is_finite());
    vals.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.row.cmp(&b.1.row)).then_with(|| a.1.subcol.cmp(&b.1.subcol)));
    let n = vals.len();
    if n < 3 {
        return Vec::new();
    }
    vals.into_iter()
        .enumerate()
        .filter_map(|(i, (y, source, series_label))| {
            // 0-based rank i: (i + 1 - 3/8) / (n + 1/4) = (i + 0.625) / (n + 0.25).
            let p = ((i as f64) + 0.625) / ((n as f64) + 0.25);
            let z = norm_ppf(p).ok()?;
            Some(DiagPoint { x: z, y, label: None, source, series_label })
        })
        .collect()
}

fn extract_qq_points(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Vec<DiagPoint> {
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);

    // 1. Prefer explicit ResultTabKind::NormalQq (excluding BlandAltman).
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::NormalQq)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::NormalQq)
    {
        let ds_col = find_header_col(&tab.column_headers, |h| h == "data set" || h == "dataset" || h == "group");
        let target_label = selected_dataset_label_in_tab(res, tab, ds_col, graph, doc);
        let exp_col = find_header_col(&tab.column_headers, |h| {
            (h.contains("expected") || h.contains("theoretical") || h.contains("normal q")) && !h.contains("lognormal")
        })
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("expected") || h.contains("theoretical")));
        let obs_col = find_header_col(&tab.column_headers, |h| {
            (h.contains("observed") || h.contains("sample") || h.contains("residual")) && !h.contains("lognormal")
        })
        .or_else(|| find_header_col(&tab.column_headers, |h| h.contains("observed") || h.contains("sample") || h.contains("residual")));

        let (xc, yc) = match (exp_col, obs_col) {
            (Some(e), Some(o)) => (Some(e), Some(o)),
            _ => {
                let mut num_cols = Vec::new();
                for (ci, h) in tab.column_headers.iter().enumerate() {
                    let hl = h.to_ascii_lowercase();
                    if ds_col == Some(ci) || hl.contains("order") || hl == "index" {
                        continue;
                    }
                    if tab.rows.first().and_then(|r| r.get(ci)).and_then(result_cell_num).is_some() {
                        num_cols.push(ci);
                    }
                }
                if num_cols.is_empty()
                    && let Some(first_row) = tab.rows.first()
                {
                    for (ci, cell) in first_row.iter().enumerate() {
                        if result_cell_num(cell).is_some() {
                            num_cols.push(ci);
                        }
                    }
                }
                match num_cols.as_slice() {
                    [a, b, ..] => (Some(*a), Some(*b)),
                    [a] => (None, Some(*a)),
                    [] => (None, None),
                }
            }
        };

        let fallback_sheet_id = res.analysis.source_sheet_id.clone();
        let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
        let ds_col_idx = match ds_opt.map(|d| &d.archetype) {
            Some(TableArchetype::MultipleVariables(_)) => graph.spec.multivariate.as_ref().map_or(ds_idx, |m| m.x_column),
            _ => ds_idx,
        };
        let mut sorted_ds_vals = ds_opt.map_or_else(|| (Vec::new(), String::new()), |ds| collect_ds_active_values_with_sources(ds, ds_col_idx));
        sorted_ds_vals.0.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.row.cmp(&b.1.row)).then_with(|| a.1.subcol.cmp(&b.1.subcol)));
        let series_label =
            if sorted_ds_vals.1.is_empty() { target_label.clone().unwrap_or_else(|| graph.spec.title.clone()) } else { sorted_ds_vals.1.clone() };

        if let (Some(ec), Some(oc)) = (xc, yc) {
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
                    Some((i, result_cell_num(row.get(ec)?)?, result_cell_num(row.get(oc)?)?))
                })
                .collect();
            if pairs.len() >= 3 {
                let can_zip_sorted = sorted_ds_vals.0.len() == pairs.len()
                    && sorted_ds_vals.0.iter().zip(pairs.iter()).all(|((dv, _), (_, _, obs))| (dv - obs).abs() <= 1e-6 * dv.abs().max(1.0));
                return pairs
                    .into_iter()
                    .enumerate()
                    .map(|(k, (tab_i, exp, obs))| {
                        let source = if can_zip_sorted {
                            sorted_ds_vals.0[k].1.clone()
                        } else {
                            SourceCellId { sheet_id: fallback_sheet_id.clone(), row: tab_i, col: ds_col_idx, subcol: 0 }
                        };
                        DiagPoint { x: exp, y: obs, label: None, source, series_label: series_label.clone() }
                    })
                    .collect();
            }
        } else if let Some(oc) = yc {
            let vals: Vec<(usize, f64)> = tab
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, row)| {
                    if let (Some(dc), Some(lbl)) = (ds_col, target_label.as_deref())
                        && matches!(row.get(dc), Some(ResultCell::Text(t)) if t != lbl)
                    {
                        return None;
                    }
                    Some((i, result_cell_num(row.get(oc)?)?))
                })
                .collect();
            if vals.len() >= 3 {
                let mut sorted_raw = sorted_ds_vals.0.clone();
                let mut tab_sorted: Vec<(usize, f64)> = vals.clone();
                tab_sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
                let can_zip = sorted_raw.len() == tab_sorted.len()
                    && sorted_raw.iter().zip(tab_sorted.iter()).all(|((dv, _), (_, ov))| (dv - ov).abs() <= 1e-6 * dv.abs().max(1.0));
                let inputs: Vec<(f64, SourceCellId, String)> = if can_zip {
                    sorted_raw.drain(..).map(|(v, src)| (v, src, series_label.clone())).collect()
                } else {
                    vals.into_iter()
                        .map(|(tab_i, v)| {
                            (v, SourceCellId { sheet_id: fallback_sheet_id.clone(), row: tab_i, col: ds_col_idx, subcol: 0 }, series_label.clone())
                        })
                        .collect()
                };
                let pts = values_to_qq_diag_points(inputs);
                if pts.len() >= 3 {
                    return pts;
                }
            }
        }
    }

    // 2. Fallback to ResultTabKind::Residuals (excluding BlandAltman pair coordinate tabs).
    let has_qq_or_resid = |r: &ResultSheet| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_))
            && r.tabs.iter().any(|t| matches!(t.kind, ResultTabKind::NormalQq | ResultTabKind::Residuals))
    };
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| {
        !matches!(r.analysis.kind, AnalysisKind::BlandAltman(_)) && r.tabs.iter().any(|t| t.kind == ResultTabKind::Residuals)
    }) && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals)
    {
        let vals: Vec<(f64, SourceCellId, String)> =
            extract_residuals_tab_points(res, tab, graph, doc).into_iter().map(|pt| (pt.y, pt.source, pt.series_label)).collect();
        let pairs = values_to_qq_diag_points(vals);
        if pairs.len() >= 3 {
            return pairs;
        }
    }
    if has_blocked_matching_result(graph, doc, has_qq_or_resid) {
        return Vec::new();
    }

    // 3. Fallback to raw DataSheet observations.
    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else { return Vec::new() };
    let target_col = match &ds.archetype {
        TableArchetype::MultipleVariables(_) => graph.spec.multivariate.as_ref().map_or(ds_idx, |m| m.x_column),
        _ => ds_idx,
    };
    let (val_sources, label) = collect_ds_active_values_with_sources(ds, target_col);
    if !val_sources.is_empty() && is_ds_pre_summarized(ds) {
        warnings.push(ChartWarning::new(
            "pre_summarized_summary_only",
            "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
        ));
    }
    let series_label = if label.is_empty() { graph.spec.title.clone() } else { label };
    let tagged = val_sources.into_iter().map(|(v, src)| (v, src, series_label.clone())).collect();
    values_to_qq_diag_points(tagged)
}

#[cfg(test)]
mod tests {
    use super::values_to_qq_diag_points;
    use crate::scene::SourceCellId;

    #[test]
    fn qq_points_use_blom_positions() {
        let vals: Vec<(f64, SourceCellId, String)> = [3.0, 1.0, 5.0, 2.0, 4.0]
            .into_iter()
            .enumerate()
            .map(|(row, v)| (v, SourceCellId { sheet_id: curvebook_doc::SheetId::new("qq"), row, col: 0, subcol: 0 }, String::from("S")))
            .collect();
        let pts = values_to_qq_diag_points(vals);
        assert_eq!(pts.len(), 5);
        // R 4.5.2: qnorm((1:5 - 3/8) / 5.25)
        //   -1.179761117611861 -0.497200570681554 0.000000000000000 0.497200570681554 1.179761117611861
        // (Hazen positions (i - 0.5) / n would give qnorm(0.1) = -1.281551565545 for the smallest value.)
        // Tolerance 1e-9: far above `norm_ppf` round-off (Halley-refined probit), far below the 0.1 Hazen gap.
        let expected = [-1.179_761_117_611_861, -0.497_200_570_681_554, 0.0, 0.497_200_570_681_554, 1.179_761_117_611_861];
        for (pt, (want_x, want_y)) in pts.iter().zip(expected.into_iter().zip([1.0, 2.0, 3.0, 4.0, 5.0])) {
            assert!((pt.x - want_x).abs() <= 1e-9, "Blom score {} != {want_x}", pt.x);
            assert!((pt.y - want_y).abs() <= f64::EPSILON, "sorted value {} != {want_y}", pt.y);
        }
    }
}
