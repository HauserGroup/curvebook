//! Sequential residual vs observation index (row order) diagnostic plot with `y = 0` reference line
//! for Linear/Logistic/Poisson GLMs, Cox PH residuals (`cox-snell-residuals`, `cox-covariate-residuals`),
//! nonlinear fits, and raw data tables.

use curvebook_doc::{AnalysisKind, GraphSheet, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, TableArchetype};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

use super::residuals_qq::{ols_residuals, selected_dataset_label_in_tab};
use super::{
    DiagPoint, active_xy_pairs_with_sources, collect_ds_active_point_sources, collect_ds_active_values_with_sources, extract_residuals_cols,
    find_header_col, find_matching_result_where_checked, has_blocked_matching_result, is_ds_pre_summarized, resolve_analysis_source_data_sheet,
    resolve_source_data_sheet_checked, result_cell_num, row_label_dataset_prefix, selected_dataset_prefix_in_row_labels,
};
use crate::markers::emit_marker;
use crate::scene::{ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, ScenePrimitive, SourceCellId};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, path_line, path_move, solid_stroke,
};
use crate::xy::format_point_value;

const PREFERRED_DIAG_TABS: &[&str] =
    &["glm-linear-diagnostics", "glm-logistic-diagnostics", "glm-poisson-diagnostics", "cox-snell-residuals", "cox-covariate-residuals"];

fn select_row_residual_col(headers: &[String]) -> Option<usize> {
    let preferred_exact = [
        "raw residual",
        "residual",
        "martingale residual",
        "pearson residual",
        "deviance residual",
        "standardized residual",
        "studentized residual",
        "cox-snell residual",
    ];
    for target in preferred_exact {
        if let Some(idx) = find_header_col(headers, |h| h == target) {
            return Some(idx);
        }
    }
    find_header_col(headers, |h| {
        (h.contains("residual") || h.contains("resid"))
            && !h.contains("normal")
            && !h.contains("qq")
            && !h.contains("q-q")
            && !h.contains("expected")
            && !h.contains("abs")
            && !h.contains("squared")
    })
}

fn is_row_residual_tab(tab: &ResultTab) -> bool {
    if PREFERRED_DIAG_TABS.contains(&tab.id.as_str()) {
        return true;
    }
    if matches!(
        tab.id.as_str(),
        "pca-scores"
            | "pca-standardized-data"
            | "glm-logistic-calibration"
            | "glm-logistic-classification"
            | "heteroscedasticity"
            | "variance-homogeneity"
    ) {
        return false;
    }
    if tab.kind == ResultTabKind::Residuals {
        return true;
    }
    if tab.kind == ResultTabKind::Diagnostics {
        if select_row_residual_col(&tab.column_headers).is_some() {
            return true;
        }
        let (_, x_col, _, obs_col) = extract_residuals_cols(&tab.column_headers);
        return x_col.is_some() && obs_col.is_some();
    }
    false
}

fn find_row_residual_tab(res: &ResultSheet) -> Option<&ResultTab> {
    if matches!(res.analysis.kind, AnalysisKind::BlandAltman(_)) {
        return None;
    }
    res.tabs
        .iter()
        .find(|t| PREFERRED_DIAG_TABS.contains(&t.id.as_str()))
        .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals && is_row_residual_tab(t)))
        .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Diagnostics && is_row_residual_tab(t)))
}

fn is_row_residual_result(res: &ResultSheet) -> bool {
    find_row_residual_tab(res).is_some()
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::ResidualVsRowPlot`] (residuals vs observation row order).
pub(in crate::multivariate_layout) fn compile_residual_vs_row_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let (pts, y_label) = extract_residual_vs_row_points(graph, doc, &mut scene.warnings);
    let pts: Vec<DiagPoint> = pts.into_iter().filter(|p| p.x.is_finite() && p.y.is_finite()).collect();
    if pts.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_residuals", "no residual observations available for residual-vs-row plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y_min, y_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y).min(0.0), b.max(p.y).max(0.0)));
    let x_pad = if (x_max - x_min).abs() < 1e-12 { 0.5 } else { (x_max - x_min) * 0.06 };
    let y_pad = (y_max - y_min).max(1.0) * 0.12;

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = String::from("Observation Index (Row Order)");
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = y_label;
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: ((x_min - x_pad).max(0.0), x_max + x_pad),
            y_bounds: (y_min - y_pad, y_max + y_pad),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    // Horizontal y = 0 reference line.
    if let Some((_, y0)) = y_axis.map_value(0.0) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, y0), PointPt::new(r.right(), y0)),
            fill: None,
            stroke: Some(solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed)),
            clip: Some(r),
        });
    }

    let color = graph.spec.palette.color_at(0);

    // Subtle sequential connecting line to reveal serial drift / autocorrelation along row order.
    if pts.len() >= 2 {
        let mut seq_path = Path2D::new();
        let mut started = false;
        for pt in &pts {
            if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.x), y_axis.map_value(pt.y)) {
                let p = PointPt::new(px, py);
                if !started {
                    path_move(&mut seq_path, p);
                    started = true;
                } else {
                    path_line(&mut seq_path, p);
                }
            }
        }
        if started {
            scene.primitives.push(ScenePrimitive::Path {
                path: seq_path,
                fill: None,
                stroke: Some(solid_stroke(color.with_alpha(105), 0.9, LineDashStyle::Solid)),
                clip: Some(r),
            });
        }
    }

    for pt in &pts {
        if let (Some((_, px)), Some((_, py))) = (x_axis.map_value(pt.x), y_axis.map_value(pt.y)) {
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
                    value_summary: format!("Row={}, Residual={}", format_point_value(pt.x), format_point_value(pt.y)),
                });
            }
        }
    }

    Ok(())
}

fn extract_residual_vs_row_points(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> (Vec<DiagPoint>, String) {
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_row_residual_result)
        && let Some(tab) = find_row_residual_tab(res)
    {
        return extract_row_residuals_from_tab(res, tab, graph, doc);
    }
    if has_blocked_matching_result(graph, doc, is_row_residual_result) {
        return (Vec::new(), String::from("Residual"));
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else {
        return (Vec::new(), String::from("Residual"));
    };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    match &ds.archetype {
        TableArchetype::Xy(xy) => {
            let pairs = active_xy_pairs_with_sources(&ds.id, xy, ds_idx);
            if !pairs.is_empty() && xy.y_subcolumn_mode.is_pre_summarized() {
                warnings.push(ChartWarning::new(
                    "pre_summarized_summary_only",
                    "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
                ));
            }
            let raw: Vec<(f64, f64)> = pairs.iter().map(|(x, y, _, _)| (*x, *y)).collect();
            let pts = ols_residuals(&raw)
                .into_iter()
                .zip(pairs)
                .enumerate()
                .map(|(i, ((_, resid), (_, _, source, series_label)))| DiagPoint { x: (i + 1) as f64, y: resid, label: None, source, series_label })
                .collect();
            (pts, String::from("Residual"))
        }
        TableArchetype::Column(_) | TableArchetype::Grouped(_) => {
            let (val_sources, lbl) = collect_ds_active_values_with_sources(ds, ds_idx);
            if !val_sources.is_empty() && is_ds_pre_summarized(ds) {
                warnings.push(ChartWarning::new(
                    "pre_summarized_summary_only",
                    "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
                ));
            }
            let mean = if val_sources.is_empty() { 0.0 } else { val_sources.iter().map(|(v, _)| *v).sum::<f64>() / (val_sources.len() as f64) };
            let pts = val_sources
                .into_iter()
                .enumerate()
                .map(|(i, (v, source))| DiagPoint { x: (i + 1) as f64, y: v - mean, label: None, source, series_label: lbl.clone() })
                .collect();
            (pts, String::from("Residual"))
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
            if y_lbl.to_ascii_lowercase().contains("resid") {
                let pts = raw
                    .into_iter()
                    .enumerate()
                    .map(|(i, (_, y, source, series_label))| DiagPoint { x: (i + 1) as f64, y, label: None, source, series_label })
                    .collect();
                return (pts, y_lbl);
            }
            let pairs: Vec<(f64, f64)> = raw.iter().map(|(x, y, _, _)| (*x, *y)).collect();
            let pts = ols_residuals(&pairs)
                .into_iter()
                .zip(raw)
                .enumerate()
                .map(|(i, ((_, resid), (_, _, source, series_label)))| DiagPoint { x: (i + 1) as f64, y: resid, label: None, source, series_label })
                .collect();
            (pts, String::from("Residual"))
        }
        _ => (Vec::new(), String::from("Residual")),
    }
}

fn extract_row_residuals_from_tab(res: &ResultSheet, tab: &ResultTab, graph: &GraphSheet, doc: &ProjectDocument) -> (Vec<DiagPoint>, String) {
    let (ds_col, x_col, _, obs_col) = extract_residuals_cols(&tab.column_headers);
    let resid_col = select_row_residual_col(&tab.column_headers);
    let target_label = selected_dataset_label_in_tab(res, tab, ds_col, graph, doc);
    let legacy_row_prefix = if ds_col.is_none() { selected_dataset_prefix_in_row_labels(res, tab, graph, doc) } else { None };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    let ds_opt = resolve_analysis_source_data_sheet(res, graph, doc);
    let fallback_sheet_id = ds_opt.map(|d| d.id.clone()).unwrap_or_else(|| graph.spec.primary_source_sheet_id.clone());

    let y_hdr = resid_col.and_then(|c| tab.column_headers.get(c)).cloned().unwrap_or_else(|| String::from("Residual"));

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
            if let Some(prefix) = legacy_row_prefix.as_deref() {
                let row_lbl = tab.row_labels.get(i).map(String::as_str).unwrap_or("");
                if !matches!(row_label_dataset_prefix(row_lbl), Some((p, _)) if p == prefix) {
                    return None;
                }
            }
            let rv = match (resid_col, x_col, obs_col) {
                (Some(rc), _, _) => result_cell_num(row.get(rc)?)?,
                (None, Some(xc), Some(oc)) if xc != oc => result_cell_num(row.get(oc)?)? - result_cell_num(row.get(xc)?)?,
                _ => row.iter().find_map(result_cell_num)?,
            };
            rv.is_finite().then_some((i, rv))
        })
        .collect();

    if vals.is_empty() {
        return (Vec::new(), y_hdr);
    }

    let source_col = match &res.analysis.kind {
        AnalysisKind::GeneralizedLinearModel(glm) => glm.response_column,
        _ => ds_idx,
    };
    let (resolved_sources, series_label) = if let Some(ds) = ds_opt {
        let (all_rep_sources, lbl) = collect_ds_active_point_sources(ds, source_col, true);
        if all_rep_sources.len() == vals.len() {
            (Some(all_rep_sources), lbl)
        } else {
            let (row_sources, lbl_row) = collect_ds_active_point_sources(ds, source_col, false);
            if row_sources.len() == vals.len() { (Some(row_sources), lbl_row) } else { (None, lbl) }
        }
    } else {
        (None, target_label.clone().or_else(|| legacy_row_prefix.clone()).unwrap_or_else(|| graph.spec.title.clone()))
    };
    let resolved_label = if series_label.is_empty() { graph.spec.title.clone() } else { series_label };
    let filtered_by_dataset = target_label.is_some() || legacy_row_prefix.is_some();

    let pts = vals
        .into_iter()
        .enumerate()
        .map(|(k, (tab_row_idx, y))| {
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
            DiagPoint { x: (k + 1) as f64, y, label: tab.row_labels.get(tab_row_idx).cloned(), source, series_label: resolved_label.clone() }
        })
        .collect();

    (pts, y_hdr)
}
