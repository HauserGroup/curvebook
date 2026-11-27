//! Homoscedasticity diagnostic plot (`|residual|` vs predicted value / group spread)
//! for ANOVA, Linear/Logistic/Poisson GLM diagnostics, nonlinear fits, and raw tables.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisKind, DataSheet, FactorialAnovaSpec, GraphSheet, OneWayMethod, ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind,
    TableArchetype, VariableRole,
};
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_text::RichText;

use super::super::multivariate::cell_token;
use super::residuals_qq::ols_residuals;
use super::{
    DiagPoint, active_xy_pairs_with_sources, collect_ds_active_values_with_sources, extract_residuals_cols, extract_residuals_tab_points,
    find_matching_result_where_checked, has_blocked_matching_result, is_ds_pre_summarized, resolve_analysis_source_data_sheet,
    resolve_source_data_sheet_checked, result_cell_num,
};
use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, RectPt, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, solid_stroke};
use crate::xy::format_point_value;

struct HomoscedasticityExtracted {
    points: Vec<DiagPoint>,
    x_label: String,
    y_label: String,
    stat_badge: Option<String>,
}

const GLM_DIAG_TAB_IDS: &[&str] = &["glm-linear-diagnostics", "glm-logistic-diagnostics", "glm-poisson-diagnostics"];

fn is_tabular_homoscedasticity_tab(tab: &ResultTab) -> bool {
    if GLM_DIAG_TAB_IDS.contains(&tab.id.as_str()) {
        return true;
    }
    if matches!(
        tab.id.as_str(),
        "heteroscedasticity"
            | "variance-homogeneity"
            | "glm-logistic-calibration"
            | "glm-logistic-classification"
            | "pca-scores"
            | "pca-standardized-data"
    ) {
        return false;
    }
    if tab.kind == ResultTabKind::Residuals {
        return true;
    }
    if tab.kind == ResultTabKind::Diagnostics {
        let (_, x_col, y_col, obs_col) = extract_residuals_cols(&tab.column_headers);
        return x_col.is_some() && (y_col.is_some() || obs_col.is_some());
    }
    false
}

fn find_homoscedasticity_tab(res: &ResultSheet) -> Option<&ResultTab> {
    if matches!(res.analysis.kind, AnalysisKind::BlandAltman(_)) {
        return None;
    }
    res.tabs
        .iter()
        .find(|t| GLM_DIAG_TAB_IDS.contains(&t.id.as_str()))
        .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Residuals && is_tabular_homoscedasticity_tab(t)))
        .or_else(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::Diagnostics && is_tabular_homoscedasticity_tab(t)))
}

fn is_homoscedasticity_result(res: &ResultSheet) -> bool {
    find_homoscedasticity_tab(res).is_some() || matches!(res.analysis.kind, AnalysisKind::OneWayAnova(_) | AnalysisKind::FactorialAnova(_))
}

fn resolve_homoscedasticity_badge(res: &ResultSheet) -> Option<String> {
    let rho = res.scalars.iter().find(|s| s.key == "Heteroscedasticity Spearman rho" && s.value.is_finite()).map(|s| s.value);
    let p = res.scalars.iter().find(|s| s.key == "Heteroscedasticity P" && s.value.is_finite()).map(|s| s.value);
    if let (Some(r), Some(pv)) = (rho, p) {
        return Some(format!("Spearman \u{03c1} = {r:.3}, P = {pv:.4}"));
    }
    if let Some(tab) = res.tabs.iter().find(|t| t.id == "variance-homogeneity")
        && let Some(row) = tab.rows.first()
        && let Some(ResultCell::PValue { raw, .. }) = row.get(3)
        && raw.is_finite()
    {
        return Some(format!("Brown-Forsythe P = {raw:.4}"));
    }
    None
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::HomoscedasticityPlot`] (`|residual|` vs predicted).
pub(in crate::multivariate_layout) fn compile_homoscedasticity_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let extracted = extract_homoscedasticity_points(graph, doc, &mut scene.warnings);
    let pts: Vec<DiagPoint> = extracted.points.into_iter().filter(|p| p.x.is_finite() && p.y.is_finite() && p.y >= 0.0).collect();
    if pts.is_empty() {
        scene
            .warnings
            .push(ChartWarning::new("empty_homoscedasticity", "no predicted / absolute-residual observations available for homoscedasticity plot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (x_min, x_max) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let y_max = pts.iter().map(|p| p.y).fold(0.0_f64, f64::max);
    let x_pad = (x_max - x_min).max(1.0) * 0.08;
    let y_top = if y_max > 0.0 { y_max * 1.15 } else { 1.0 };

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = extracted.x_label;
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = extracted.y_label;
    }

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (x_min - x_pad, x_max + x_pad),
            y_bounds: (0.0, y_top),
            y2_bounds: None,
            cat_x: None,
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    emit_homoscedasticity_reference_line(scene, &pts, &x_axis, &y_axis, r);

    let color = graph.spec.palette.color_at(0);
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
                    value_summary: format!("Predicted={}, |Residual|={}", format_point_value(pt.x), format_point_value(pt.y)),
                });
            }
        }
    }

    if let Some(badge) = extracted.stat_badge
        && r.width >= 130.0
    {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(badge),
            anchor_pt: PointPt::new(r.right() - 6.0, r.y + 11.0),
            anchor: TextAnchor::End,
            font_size_pt: 7.5,
            rotation_deg: 0.0,
            color: Rgba8::rgb(45, 52, 64),
        }));
    }
    Ok(())
}

fn emit_homoscedasticity_reference_line(
    scene: &mut ChartScene,
    pts: &[DiagPoint],
    x_axis: &crate::xy::SegmentedAxis,
    y_axis: &crate::xy::SegmentedAxis,
    r: RectPt,
) {
    let n = pts.len() as f64;
    let mx = pts.iter().map(|p| p.x).sum::<f64>() / n;
    let my = pts.iter().map(|p| p.y).sum::<f64>() / n;
    let sxx = pts.iter().map(|p| (p.x - mx) * (p.x - mx)).sum::<f64>();
    let sxy = pts.iter().map(|p| (p.x - mx) * (p.y - my)).sum::<f64>();
    let (slope, intercept) = if pts.len() >= 2 && sxx > 1e-12 {
        let b1 = sxy / sxx;
        (b1, my - b1 * mx)
    } else {
        (0.0, my)
    };
    let ref_stroke = solid_stroke(Rgba8::rgb(110, 118, 128), 1.0, LineDashStyle::Dashed);
    for seg_x in &x_axis.segments {
        let sx0 = seg_x.domain_min.min(seg_x.domain_max);
        let sx1 = seg_x.domain_min.max(seg_x.domain_max);
        let sy0 = (intercept + slope * sx0).max(0.0);
        let sy1 = (intercept + slope * sx1).max(0.0);
        if let (Some(px0), Some(px1), Some((_, py0)), Some((_, py1))) = (
            seg_x.mapping().map_value(sx0, seg_x.span_start_pt, seg_x.span_end_pt),
            seg_x.mapping().map_value(sx1, seg_x.span_start_pt, seg_x.span_end_pt),
            y_axis.map_value(sy0),
            y_axis.map_value(sy1),
        ) {
            scene.primitives.push(ScenePrimitive::Path {
                path: Path2D::line(PointPt::new(px0, py0), PointPt::new(px1, py1)),
                fill: None,
                stroke: Some(ref_stroke),
                clip: Some(r),
            });
        }
    }
}

fn extract_homoscedasticity_points(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> HomoscedasticityExtracted {
    let default_out = |points: Vec<DiagPoint>, x_label: &str, y_label: &str, stat_badge: Option<String>| HomoscedasticityExtracted {
        points,
        x_label: String::from(x_label),
        y_label: String::from(y_label),
        stat_badge,
    };

    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_homoscedasticity_result) {
        let badge = resolve_homoscedasticity_badge(res);
        if let Some(tab) = find_homoscedasticity_tab(res) {
            let (_, x_col, y_col, _) = extract_residuals_cols(&tab.column_headers);
            let x_hdr = x_col.and_then(|c| tab.column_headers.get(c)).cloned().unwrap_or_else(|| String::from("Predicted Value"));
            let y_hdr = y_col.and_then(|c| tab.column_headers.get(c)).map(|h| format!("|{h}|")).unwrap_or_else(|| String::from("|Residual|"));
            let pts: Vec<DiagPoint> = extract_residuals_tab_points(res, tab, graph, doc)
                .into_iter()
                .map(|mut p| {
                    p.y = p.y.abs();
                    p
                })
                .collect();
            return HomoscedasticityExtracted { points: pts, x_label: x_hdr, y_label: y_hdr, stat_badge: badge };
        }

        if let AnalysisKind::OneWayAnova(ow) = &res.analysis.kind {
            let lognormal = matches!(
                ow.method,
                OneWayMethod::LognormalOrdinary | OneWayMethod::LognormalWelchBf | OneWayMethod::LognormalRepeatedMeasures { .. }
            );
            if let Some(ds) = resolve_analysis_source_data_sheet(res, graph, doc) {
                let pts = extract_one_way_group_spread(ds, &res.analysis.selected_data_sets, lognormal, warnings);
                let x_lbl = if lognormal { "Predicted Mean ln(Y)" } else { "Predicted Group Mean" };
                let y_lbl = if lognormal { "|Residual ln(Y)|" } else { "|Residual|" };
                return default_out(pts, x_lbl, y_lbl, badge);
            }
        }

        if let AnalysisKind::FactorialAnova(fac) = &res.analysis.kind {
            let pts = extract_factorial_group_spread(res, fac, graph, doc);
            return default_out(pts, "Predicted Cell Mean", "|Residual|", badge);
        }
    }

    if has_blocked_matching_result(graph, doc, is_homoscedasticity_result) {
        return default_out(Vec::new(), "Predicted Value", "|Residual|", None);
    }

    let Some(ds) = resolve_source_data_sheet_checked(graph, doc, warnings) else {
        return default_out(Vec::new(), "Predicted Value", "|Residual|", None);
    };
    let ds_idx = graph.spec.series.first().map_or(0, |s| s.data_set_index);
    match &ds.archetype {
        TableArchetype::Column(_) => {
            let pts = extract_one_way_group_spread(ds, &[], false, warnings);
            default_out(pts, "Predicted Group Mean", "|Residual|", None)
        }
        TableArchetype::Grouped(g) => {
            let pts = extract_grouped_table_spread(&ds.id, g, &[]);
            default_out(pts, "Predicted Cell Mean", "|Residual|", None)
        }
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
                .map(|((fitted, resid), (_, _, source, series_label))| DiagPoint { x: fitted, y: resid.abs(), label: None, source, series_label })
                .collect();
            default_out(pts, "Predicted Value", "|Residual|", None)
        }
        TableArchetype::MultipleVariables(mv) if mv.variables.len() >= 2 => {
            let xi = graph.spec.multivariate.as_ref().map_or(0, |m| m.x_column);
            let yi = graph.spec.multivariate.as_ref().map_or(1, |m| m.y_column);
            if let (Some(cx), Some(_)) = (mv.variables.get(xi), mv.variables.get(yi))
                && cx.role == VariableRole::Categorical
            {
                let pts = extract_mv_categorical_spread(&ds.id, mv, xi, yi, false);
                return default_out(pts, "Predicted Group Mean", "|Residual|", None);
            }
            let y_lbl = mv.variables.get(yi).map_or_else(|| String::from("Residual"), |v| v.title.clone());
            let raw: Vec<(f64, f64, SourceCellId, String)> = (0..mv.row_count())
                .filter_map(|r| {
                    let x = mv.variables.get(xi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    let y = mv.variables.get(yi)?.cells.get(r)?.active_number().filter(|v| v.is_finite())?;
                    Some((x, y, SourceCellId { sheet_id: ds.id.clone(), row: r, col: yi, subcol: 0 }, y_lbl.clone()))
                })
                .collect();
            if y_lbl.to_ascii_lowercase().contains("resid") {
                let pts =
                    raw.into_iter().map(|(x, y, source, series_label)| DiagPoint { x, y: y.abs(), label: None, source, series_label }).collect();
                return default_out(pts, "Predicted Value", "|Residual|", None);
            }
            let pairs: Vec<(f64, f64)> = raw.iter().map(|(x, y, _, _)| (*x, *y)).collect();
            let pts = ols_residuals(&pairs)
                .into_iter()
                .zip(raw)
                .map(|((fitted, resid), (_, _, source, series_label))| DiagPoint { x: fitted, y: resid.abs(), label: None, source, series_label })
                .collect();
            default_out(pts, "Predicted Value", "|Residual|", None)
        }
        _ => default_out(Vec::new(), "Predicted Value", "|Residual|", None),
    }
}

fn extract_one_way_group_spread(ds: &DataSheet, selected: &[usize], lognormal: bool, warnings: &mut Vec<ChartWarning>) -> Vec<DiagPoint> {
    match &ds.archetype {
        TableArchetype::Column(col) => {
            if is_ds_pre_summarized(ds) {
                warnings.push(ChartWarning::new(
                    "pre_summarized_summary_only",
                    "pre-summarized table uses only the primary summary value (subcolumn 0) without replicate dispersion",
                ));
            }
            let cols: Vec<usize> = if selected.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                selected.iter().copied().filter(|&c| c < col.data_sets.len()).collect()
            };
            let mut out = Vec::new();
            for c in cols {
                let (vals, lbl) = collect_ds_active_values_with_sources(ds, c);
                let transformed: Vec<(f64, SourceCellId)> = vals
                    .into_iter()
                    .filter_map(|(v, src)| {
                        let y = if lognormal { (v > 0.0).then(|| v.ln())? } else { v };
                        y.is_finite().then_some((y, src))
                    })
                    .collect();
                if transformed.is_empty() {
                    continue;
                }
                let mean = transformed.iter().map(|(v, _)| *v).sum::<f64>() / (transformed.len() as f64);
                for (y, source) in transformed {
                    out.push(DiagPoint { x: mean, y: (y - mean).abs(), label: Some(lbl.clone()), source, series_label: lbl.clone() });
                }
            }
            out
        }
        TableArchetype::MultipleVariables(mv) => {
            let cols: Vec<usize> = if selected.is_empty() {
                mv.variables
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| !matches!(v.role, VariableRole::Categorical | VariableRole::Label))
                    .map(|(i, _)| i)
                    .collect()
            } else {
                selected.iter().copied().filter(|&c| c < mv.variables.len()).collect()
            };
            let mut out = Vec::new();
            for c in cols {
                let (vals, lbl) = collect_ds_active_values_with_sources(ds, c);
                let transformed: Vec<(f64, SourceCellId)> = vals
                    .into_iter()
                    .filter_map(|(v, src)| {
                        let y = if lognormal { (v > 0.0).then(|| v.ln())? } else { v };
                        y.is_finite().then_some((y, src))
                    })
                    .collect();
                if transformed.is_empty() {
                    continue;
                }
                let mean = transformed.iter().map(|(v, _)| *v).sum::<f64>() / (transformed.len() as f64);
                for (y, source) in transformed {
                    out.push(DiagPoint { x: mean, y: (y - mean).abs(), label: Some(lbl.clone()), source, series_label: lbl.clone() });
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

fn extract_mv_categorical_spread(
    sheet_id: &curvebook_doc::SheetId,
    mv: &curvebook_doc::MultipleVariablesTable,
    cat_col: usize,
    y_col: usize,
    lognormal: bool,
) -> Vec<DiagPoint> {
    let (Some(cc), Some(yc)) = (mv.variables.get(cat_col), mv.variables.get(y_col)) else {
        return Vec::new();
    };
    let mut groups: BTreeMap<String, Vec<(f64, usize)>> = BTreeMap::new();
    for r in 0..mv.row_count() {
        let Some(tok) = cell_token(cc, r) else { continue };
        let Some(raw) = yc.cells.get(r).and_then(|c| c.active_number()).filter(|v| v.is_finite()) else {
            continue;
        };
        let y = if lognormal {
            if raw <= 0.0 {
                continue;
            }
            raw.ln()
        } else {
            raw
        };
        if y.is_finite() {
            groups.entry(tok).or_default().push((y, r));
        }
    }
    let mut out = Vec::new();
    for (grp, items) in groups {
        if items.is_empty() {
            continue;
        }
        let mean = items.iter().map(|(v, _)| *v).sum::<f64>() / (items.len() as f64);
        for (y, r) in items {
            out.push(DiagPoint {
                x: mean,
                y: (y - mean).abs(),
                label: Some(grp.clone()),
                source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: y_col, subcol: 0 },
                series_label: grp.clone(),
            });
        }
    }
    out
}

fn extract_grouped_table_spread(sheet_id: &curvebook_doc::SheetId, g: &curvebook_doc::GroupedTable, selected: &[usize]) -> Vec<DiagPoint> {
    let cols: Vec<usize> =
        if selected.is_empty() { (0..g.data_sets.len()).collect() } else { selected.iter().copied().filter(|&c| c < g.data_sets.len()).collect() };
    let mut out = Vec::new();
    for &c in &cols {
        let Some(ds_col) = g.data_sets.get(c) else { continue };
        let lbl = ds_col.title.clone();
        for r in 0..g.row_count() {
            let Some(row_cells) = ds_col.cells.get(r) else { continue };
            let reps: Vec<(f64, usize)> = if g.subcolumn_mode.is_pre_summarized() {
                row_cells.first().and_then(|cell| cell.active_number()).filter(|v| v.is_finite()).map(|v| vec![(v, 0)]).unwrap_or_default()
            } else {
                row_cells.iter().enumerate().filter_map(|(sc, cell)| cell.active_number().filter(|v| v.is_finite()).map(|v| (v, sc))).collect()
            };
            if reps.is_empty() {
                continue;
            }
            let mean = reps.iter().map(|(v, _)| *v).sum::<f64>() / (reps.len() as f64);
            for (y, subcol) in reps {
                out.push(DiagPoint {
                    x: mean,
                    y: (y - mean).abs(),
                    label: Some(lbl.clone()),
                    source: SourceCellId { sheet_id: sheet_id.clone(), row: r, col: c, subcol },
                    series_label: lbl.clone(),
                });
            }
        }
    }
    out
}

fn extract_factorial_group_spread(res: &ResultSheet, fac: &FactorialAnovaSpec, graph: &GraphSheet, doc: &ProjectDocument) -> Vec<DiagPoint> {
    if let Some(tab) = res.tabs.iter().find(|t| t.id == "consolidated-data")
        && tab.column_headers.len() >= 2
    {
        let val_col = tab.column_headers.len() - 1;
        let mut cells: BTreeMap<Vec<String>, Vec<(f64, usize)>> = BTreeMap::new();
        for (i, row) in tab.rows.iter().enumerate() {
            let Some(y) = row.get(val_col).and_then(result_cell_num) else { continue };
            let key: Vec<String> = row[..val_col]
                .iter()
                .map(|c| match c {
                    ResultCell::Text(s) => s.clone(),
                    ResultCell::Number { value, .. } => format!("{value}"),
                    _ => String::new(),
                })
                .collect();
            cells.entry(key).or_default().push((y, i));
        }
        let mut out = Vec::new();
        for (key, items) in cells {
            if items.is_empty() {
                continue;
            }
            let mean = items.iter().map(|(v, _)| *v).sum::<f64>() / (items.len() as f64);
            let lbl = key.join(" / ");
            for (y, row_idx) in items {
                out.push(DiagPoint {
                    x: mean,
                    y: (y - mean).abs(),
                    label: Some(lbl.clone()),
                    source: SourceCellId {
                        sheet_id: res.analysis.source_sheet_id.clone().unwrap_or_else(|| res.id.clone()),
                        row: row_idx,
                        col: 0,
                        subcol: 0,
                    },
                    series_label: lbl.clone(),
                });
            }
        }
        if !out.is_empty() {
            return out;
        }
    }

    let Some(ds) = resolve_analysis_source_data_sheet(res, graph, doc) else {
        return Vec::new();
    };
    match &ds.archetype {
        TableArchetype::Grouped(g) => extract_grouped_table_spread(&ds.id, g, &res.analysis.selected_data_sets),
        TableArchetype::MultipleVariables(mv) => {
            let y_col = fac
                .response_column
                .or_else(|| mv.variables.iter().position(|v| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated)));
            let Some(yi) = y_col else { return Vec::new() };
            let factor_cols: Vec<usize> = if !fac.multifactor_columns.is_empty() {
                fac.multifactor_columns.iter().copied().filter(|&c| c < mv.variables.len() && c != yi).collect()
            } else {
                mv.variables.iter().enumerate().filter(|&(i, v)| i != yi && v.role == VariableRole::Categorical).map(|(i, _)| i).collect()
            };
            if factor_cols.is_empty() {
                return Vec::new();
            }
            let mut cells: BTreeMap<Vec<String>, Vec<(f64, usize)>> = BTreeMap::new();
            for r in 0..mv.row_count() {
                let Some(y) = mv.variables.get(yi).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()).filter(|v| v.is_finite()) else {
                    continue;
                };
                let mut key = Vec::with_capacity(factor_cols.len());
                let mut valid = true;
                for &fc in &factor_cols {
                    if let Some(tok) = mv.variables.get(fc).and_then(|v| cell_token(v, r)) {
                        key.push(tok);
                    } else {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    cells.entry(key).or_default().push((y, r));
                }
            }
            let mut out = Vec::new();
            for (key, items) in cells {
                if items.is_empty() {
                    continue;
                }
                let mean = items.iter().map(|(v, _)| *v).sum::<f64>() / (items.len() as f64);
                let lbl = key.join(" / ");
                for (y, r) in items {
                    out.push(DiagPoint {
                        x: mean,
                        y: (y - mean).abs(),
                        label: Some(lbl.clone()),
                        source: SourceCellId { sheet_id: ds.id.clone(), row: r, col: yi, subcol: 0 },
                        series_label: lbl.clone(),
                    });
                }
            }
            out
        }
        _ => Vec::new(),
    }
}
