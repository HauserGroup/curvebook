//! Symmetric `p x p` diverging parameter correlation heatmap (`[-1, 1]`) for Linear/Logistic/Poisson
//! GLMs (`glm-*-correlation`), Cox PH (`cox-covariance`), nonlinear covariance, and correlation/PCA matrices.

use curvebook_doc::{
    AnalysisKind, ColorChannelMapping, GraphSheet, HeatmapColumnLabelAngle, HeatmapColumnLabelMode, HeatmapRowLabelMode, MultivariateSpec,
    ProjectDocument, ResultCell, ResultSheet, ResultTab, ResultTabKind, TableArchetype, VariableRole,
};
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, Rgba8};
use curvebook_stats::multivariate::correlation_matrix;
use curvebook_text::{RichText, parse_markup};

use super::heatmap::{cap_heatmap_dimensions, spreadsheet_column_letters};
use super::{find_matching_result_where_checked, has_blocked_matching_result, resolve_source_data_sheet_checked};
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor};
use crate::survival_cat::{
    PlotMarginConfig, draw_colorbar, draw_empty_open_frame, path_line, path_move, sanitize_range, solid_stroke, text_metrics,
    update_plot_rect_for_labels,
};

/// Maximum cells for automatic numeric value annotations inside correlation matrix tiles.
const MAX_PARAM_CORR_ANNOTATED_CELLS: usize = 625;

const DIRECT_CORR_TAB_IDS: &[&str] =
    &["glm-linear-correlation", "glm-logistic-correlation", "glm-poisson-correlation", "cox-covariance", "corr-matrix-r"];

struct ParamCorrMatrix {
    row_labels: Vec<String>,
    col_labels: Vec<String>,
    values: Vec<Vec<Option<f64>>>,
}

fn find_param_corr_tab(res: &ResultSheet) -> Option<&ResultTab> {
    res.tabs
        .iter()
        .find(|t| DIRECT_CORR_TAB_IDS.contains(&t.id.as_str()))
        .or_else(|| {
            res.tabs.iter().find(|t| {
                let id_l = t.id.to_ascii_lowercase();
                let title_l = t.title.to_ascii_lowercase();
                (id_l.contains("correlation") || title_l.contains("correlation"))
                    && matches!(t.kind, ResultTabKind::CovarianceMatrix | ResultTabKind::MainSummary)
            })
        })
        .or_else(|| res.tabs.iter().find(|t| t.id == "pca-cov-corr-matrix"))
        .or_else(|| {
            res.tabs.iter().find(|t| {
                t.kind == ResultTabKind::CovarianceMatrix && !matches!(t.id.as_str(), "clustering-distance-matrix" | "cophenetic-distance-matrix")
            })
        })
        .or_else(|| {
            matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
                .then(|| res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary))
                .flatten()
        })
}

fn is_param_corr_result(res: &ResultSheet) -> bool {
    find_param_corr_tab(res).is_some()
}

/// Convert a square covariance matrix into a correlation matrix (`[-1, 1]`) when entries are not
/// already normalized correlations.
fn normalize_matrix_to_correlation(raw: Vec<Vec<Option<f64>>>, already_correlation: bool) -> Vec<Vec<Option<f64>>> {
    let n = raw.len();
    if n == 0 {
        return raw;
    }
    let is_square = raw.iter().all(|row| row.len() == n);
    if !is_square {
        return raw.into_iter().map(|row| row.into_iter().map(|v| v.map(|x| x.clamp(-1.0, 1.0))).collect()).collect();
    }
    let diag_looks_unit = (0..n).all(|i| raw[i][i].is_none_or(|d| (d - 1.0).abs() <= 1e-4));
    let all_in_unit = raw.iter().flatten().flatten().all(|&v| v.abs() <= 1.0 + 1e-4);
    if already_correlation || (diag_looks_unit && all_in_unit) {
        return raw.into_iter().map(|row| row.into_iter().map(|v| v.map(|x| x.clamp(-1.0, 1.0))).collect()).collect();
    }

    let mut out = vec![vec![None; n]; n];
    for i in 0..n {
        let Some(vii) = raw[i][i].filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        let si = vii.sqrt();
        for j in 0..n {
            if i == j {
                out[i][j] = Some(1.0);
                continue;
            }
            let Some(vjj) = raw[j][j].filter(|v| v.is_finite() && *v > 0.0) else {
                continue;
            };
            if let Some(cij) = raw[i][j].filter(|v| v.is_finite()) {
                let r = cij / (si * vjj.sqrt());
                if r.is_finite() {
                    out[i][j] = Some(r.clamp(-1.0, 1.0));
                }
            }
        }
    }
    out
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::ParameterCorrelationHeatmap`].
pub(in crate::multivariate_layout) fn compile_parameter_correlation_heatmap(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let Some(mat) = extract_param_corr_matrix(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new("empty_correlation_heatmap", "no parameter correlation or covariance matrix available"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let n_rows = mat.values.len();
    let n_cols = mat.col_labels.len();
    if n_rows == 0 || n_cols == 0 || !mat.values.iter().flatten().any(|v| v.is_some_and(f64::is_finite)) {
        scene.warnings.push(ChartWarning::new("empty_correlation_heatmap", "no finite numeric entries available for correlation heatmap"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let (ramp, dom_lo, dom_hi) = match graph.spec.multivariate.as_ref().map(|m| &m.color) {
        Some(ColorChannelMapping::Continuous { ramp, domain_clamp, .. }) => {
            let (lo, hi) = domain_clamp.map_or((-1.0, 1.0), |(a, b)| sanitize_range(a, b, (-1.0, 1.0)));
            (ramp.clone(), lo, hi)
        }
        _ => (ColorRamp::Named { id: ColormapId::DivergingBlueRed, reversed: false }, -1.0, 1.0),
    };

    let lbl_spec = graph.spec.heatmap_labels;
    let max_row_lbl_w = match lbl_spec.row_labels {
        HeatmapRowLabelMode::RowTitles => mat.row_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max),
        HeatmapRowLabelMode::RowNumbers => text_metrics(&format!("{n_rows}"), 7.5).width_pt,
        HeatmapRowLabelMode::None => 0.0,
    };
    let resolved_col_labels: Vec<String> = match lbl_spec.column_labels {
        HeatmapColumnLabelMode::ColumnTitles => mat.col_labels.clone(),
        HeatmapColumnLabelMode::ColumnLetters => (0..n_cols).map(spreadsheet_column_letters).collect(),
        HeatmapColumnLabelMode::None => Vec::new(),
    };
    let max_col_lbl_w = resolved_col_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max);
    let col_lbl_proj_h = if lbl_spec.column_labels == HeatmapColumnLabelMode::None || resolved_col_labels.is_empty() {
        0.0
    } else {
        match lbl_spec.column_angle {
            HeatmapColumnLabelAngle::Horizontal0 => 16.0,
            HeatmapColumnLabelAngle::Angled45 => (max_col_lbl_w * std::f64::consts::FRAC_1_SQRT_2 + 10.0).max(18.0),
            HeatmapColumnLabelAngle::Vertical90 => (max_col_lbl_w + 10.0).max(18.0),
        }
    };

    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig {
            extra_bottom_pt: (col_lbl_proj_h - 6.0).max(16.0).min(scene.height_pt * 0.35),
            extra_left_pt: crate::clamp_f64_safe(max_row_lbl_w + 8.0, 14.0, 110.0),
            has_colorbar: true,
            suppress_y_axis_labels: true,
            ..PlotMarginConfig::default()
        },
    );

    let r = scene.plot_rect_pt;
    let grid = r;
    let cell_w = grid.width / (n_cols as f64);
    let cell_h = grid.height / (n_rows as f64);
    let gap = if lbl_spec.cell_gap_pt.is_finite() { lbl_spec.cell_gap_pt.clamp(0.0, (cell_w.min(cell_h) * 0.35).max(0.0)) } else { 0.0 };
    let half_gap = gap * 0.5;
    let cell_border = solid_stroke(Rgba8::rgba(255, 255, 255, 180), 0.5, LineDashStyle::Solid);
    let missing_border = solid_stroke(Rgba8::rgba(170, 176, 186, 180), 0.5, LineDashStyle::Solid);
    let missing_hatch = solid_stroke(Rgba8::rgba(150, 156, 166, 175), 0.65, LineDashStyle::Solid);
    let denom = (dom_hi - dom_lo).abs().max(1e-12);

    let total_cells = n_rows.saturating_mul(n_cols);
    let show_cell_values = total_cells <= MAX_PARAM_CORR_ANNOTATED_CELLS && (cell_w - gap) >= 16.0 && (cell_h - gap) >= 9.0;

    for row_idx in 0..n_rows {
        let Some(row_vals) = mat.values.get(row_idx) else { continue };
        let y = grid.y + (row_idx as f64) * cell_h;
        for col_idx in 0..n_cols {
            let x = grid.x + (col_idx as f64) * cell_w;
            let tile_rect = RectPt::new(x + half_gap, y + half_gap, (cell_w - gap).max(1.0), (cell_h - gap).max(1.0));
            if let Some(val) = row_vals.get(col_idx).copied().flatten().filter(|v| v.is_finite()) {
                let t = crate::clamp_f64_safe((val - dom_lo) / denom, 0.0, 1.0);
                let fill_color = ramp.sample(t);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(tile_rect),
                    fill: Some(FillPaint::solid(fill_color)),
                    stroke: Some(cell_border),
                    clip: None,
                });
                if show_cell_values {
                    let font_pt = crate::clamp_f64_safe(tile_rect.width.min(tile_rect.height) * 0.30, 5.5, 9.0);
                    let ink =
                        if Rgba8::WHITE.contrast_ratio(fill_color) >= Rgba8::BLACK.contrast_ratio(fill_color) { Rgba8::WHITE } else { Rgba8::BLACK };
                    let txt = format!("{val:.2}");
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: RichText::plain(txt),
                        anchor_pt: PointPt::new(tile_rect.x + tile_rect.width * 0.5, tile_rect.y + tile_rect.height * 0.5 + font_pt * 0.35),
                        anchor: TextAnchor::Middle,
                        font_size_pt: font_pt,
                        rotation_deg: 0.0,
                        color: ink,
                    }));
                }
            } else {
                scene.primitives.push(ScenePrimitive::Path { path: Path2D::rect(tile_rect), fill: None, stroke: Some(missing_border), clip: None });
                let mut slash = Path2D::new();
                path_move(&mut slash, PointPt::new(tile_rect.x, tile_rect.y + tile_rect.height));
                path_line(&mut slash, PointPt::new(tile_rect.x + tile_rect.width, tile_rect.y));
                scene.primitives.push(ScenePrimitive::Path { path: slash, fill: None, stroke: Some(missing_hatch), clip: None });
            }
        }

        match lbl_spec.row_labels {
            HeatmapRowLabelMode::RowTitles => {
                let lbl = mat.row_labels.get(row_idx).map(String::as_str).unwrap_or("");
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(lbl),
                    anchor_pt: PointPt::new(grid.x - 5.0, y + cell_h * 0.5 + 3.0),
                    anchor: TextAnchor::End,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            HeatmapRowLabelMode::RowNumbers => {
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: RichText::plain(format!("{}", row_idx + 1)),
                    anchor_pt: PointPt::new(grid.x - 5.0, y + cell_h * 0.5 + 3.0),
                    anchor: TextAnchor::End,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            HeatmapRowLabelMode::None => {}
        }
    }

    for (c, lbl) in resolved_col_labels.iter().enumerate() {
        let cx = grid.x + ((c as f64) + 0.5) * cell_w;
        let (anchor_pt, anchor, rotation_deg) = match lbl_spec.column_angle {
            HeatmapColumnLabelAngle::Horizontal0 => (PointPt::new(cx, grid.bottom() + 11.0), TextAnchor::Middle, 0.0),
            HeatmapColumnLabelAngle::Angled45 => (PointPt::new(cx + 2.0, grid.bottom() + 6.0), TextAnchor::End, -45.0),
            HeatmapColumnLabelAngle::Vertical90 => (PointPt::new(cx + 2.5, grid.bottom() + 6.0), TextAnchor::End, -90.0),
        };
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(lbl),
            anchor_pt,
            anchor,
            font_size_pt: 7.5,
            rotation_deg,
            color: Rgba8::BLACK,
        }));
    }

    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(grid),
        fill: None,
        stroke: Some(solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid)),
        clip: None,
    });
    draw_colorbar(scene, &ramp, dom_lo, dom_hi, "r", RectPt::new(grid.right() + 10.0, grid.y + 8.0, 12.0, (grid.height - 12.0).max(24.0)));
    Ok(())
}

fn extract_tab_matrix(tab: &ResultTab, warnings: &mut Vec<ChartWarning>) -> ParamCorrMatrix {
    let orig_rows = tab.rows.len();
    let orig_cols = tab.column_headers.len();
    let (n_rows, n_cols, _) = cap_heatmap_dimensions(orig_rows, orig_cols, warnings);
    let mut raw = Vec::with_capacity(n_rows);
    for row in tab.rows.iter().take(n_rows) {
        let mut vals = Vec::with_capacity(n_cols);
        for c in 0..n_cols {
            let v = row.get(c).and_then(|cell| match cell {
                ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
                _ => None,
            });
            vals.push(v);
        }
        raw.push(vals);
    }
    let already_corr = DIRECT_CORR_TAB_IDS.contains(&tab.id.as_str())
        || tab.id.to_ascii_lowercase().contains("correlation")
        || tab.title.to_ascii_lowercase().contains("correlation");
    let values = normalize_matrix_to_correlation(raw, already_corr);
    let row_labels: Vec<String> =
        (0..n_rows).map(|i| tab.row_labels.get(i).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("P{}", i + 1))).collect();
    let col_labels: Vec<String> =
        (0..n_cols).map(|i| tab.column_headers.get(i).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("P{}", i + 1))).collect();
    ParamCorrMatrix { row_labels, col_labels, values }
}

fn extract_param_corr_matrix(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<ParamCorrMatrix> {
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_param_corr_result)
        && let Some(tab) = find_param_corr_tab(res)
    {
        return Some(extract_tab_matrix(tab, warnings));
    }
    if has_blocked_matching_result(graph, doc, is_param_corr_result) {
        return None;
    }

    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<(usize, String)> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                .map(|(i, v)| (i, v.title.clone()))
                .collect();
            if num_cols.is_empty() {
                return None;
            }
            // Direct square correlation matrix table (including 1x1).
            if mv.row_count() == num_cols.len() {
                let raw: Vec<Vec<Option<f64>>> = (0..mv.row_count())
                    .map(|r| {
                        num_cols
                            .iter()
                            .map(|(ci, _)| {
                                mv.variables.get(*ci).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()).filter(|x| x.is_finite())
                            })
                            .collect()
                    })
                    .collect();
                let all_in_unit = raw.iter().flatten().flatten().all(|&v| v.abs() <= 1.0 + 1e-4);
                let has_diag_one = (0..raw.len()).any(|i| raw[i][i].is_some_and(|v| (v - 1.0).abs() <= 1e-4));
                let diag_ones = has_diag_one && (0..raw.len()).all(|i| raw[i][i].is_none_or(|v| (v - 1.0).abs() <= 1e-4));
                if all_in_unit && diag_ones {
                    let col_labels: Vec<String> = num_cols.iter().map(|(_, t)| t.clone()).collect();
                    let row_labels: Vec<String> = (0..raw.len())
                        .map(|r| mv.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| col_labels[r].clone()))
                        .collect();
                    return Some(ParamCorrMatrix { row_labels, col_labels, values: normalize_matrix_to_correlation(raw, true) });
                }
            }
            if num_cols.len() == 1 {
                let lbl = num_cols[0].1.clone();
                return Some(ParamCorrMatrix { row_labels: vec![lbl.clone()], col_labels: vec![lbl], values: vec![vec![Some(1.0)]] });
            }
            let opt_cols: Vec<Vec<Option<f64>>> = num_cols
                .iter()
                .map(|(ci, _)| mv.variables[*ci].cells.iter().map(|c| c.active_number().filter(|v| v.is_finite())).collect())
                .collect();
            let refs: Vec<(&str, &[Option<f64>])> = num_cols.iter().zip(opt_cols.iter()).map(|((_, t), v)| (t.as_str(), v.as_slice())).collect();
            let corr = correlation_matrix(&refs, false).ok()?;
            let values = corr.r_matrix.into_iter().map(|row| row.into_iter().map(|v| v.is_finite().then(|| v.clamp(-1.0, 1.0))).collect()).collect();
            Some(ParamCorrMatrix { row_labels: corr.labels.clone(), col_labels: corr.labels, values })
        }
        TableArchetype::Column(col) => {
            if col.data_sets.is_empty() {
                return None;
            }
            if col.data_sets.len() == 1 {
                let lbl = col.data_sets[0].title.clone();
                let has_finite = col.data_sets[0].cells.iter().any(|s| s.first().and_then(|c| c.active_number()).is_some_and(f64::is_finite));
                return has_finite.then(|| ParamCorrMatrix { row_labels: vec![lbl.clone()], col_labels: vec![lbl], values: vec![vec![Some(1.0)]] });
            }
            let labels: Vec<String> = col.data_sets.iter().map(|d| d.title.clone()).collect();
            let opt_cols: Vec<Vec<Option<f64>>> = col
                .data_sets
                .iter()
                .map(|d| d.cells.iter().map(|s| s.first().and_then(|c| c.active_number()).filter(|v| v.is_finite())).collect())
                .collect();
            let refs: Vec<(&str, &[Option<f64>])> = labels.iter().zip(opt_cols.iter()).map(|(l, v)| (l.as_str(), v.as_slice())).collect();
            let corr = correlation_matrix(&refs, false).ok()?;
            let values = corr.r_matrix.into_iter().map(|row| row.into_iter().map(|v| v.is_finite().then(|| v.clamp(-1.0, 1.0))).collect()).collect();
            Some(ParamCorrMatrix { row_labels: corr.labels.clone(), col_labels: corr.labels, values })
        }
        _ => None,
    }
}
