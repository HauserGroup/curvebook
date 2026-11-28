//! Principal Component Analysis biplot combining observation scores `(PC1, PC2)` with scaled
//! variable loading vectors (Gabriel 1971; Jolliffe 2002).
//!
//! References:
//! - Gabriel, K. R. (1971). The biplot graphic display of matrices with application to principal
//!   component analysis. *Biometrika*, 58(3), 453–467.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). Springer.

use std::collections::BTreeMap;

use curvebook_doc::{AnalysisKind, Cell, GraphSheet, MultivariateSpec, ProjectDocument, ResultSheet, TableArchetype, VariableRole};
use curvebook_math::linalg::DenseMatrix;
use curvebook_palette::{LineDashStyle, MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};
use curvebook_stats::multivariate::pca_analysis;
use curvebook_text::RichText;

use super::{find_matching_result_where_checked, has_blocked_matching_result, resolve_source_data_sheet_checked, result_cell_num};
use crate::markers::emit_marker;
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, path_line, path_move, solid_stroke,
};
use crate::xy::format_point_value;

struct BiplotScore {
    label: String,
    pc1: f64,
    pc2: f64,
    group_idx: usize,
    group_label: Option<String>,
}

struct BiplotLoading {
    label: String,
    pc1: f64,
    pc2: f64,
}

struct PcaBiplotExtracted {
    scores: Vec<BiplotScore>,
    loadings: Vec<BiplotLoading>,
    pc1_var_pct: Option<f64>,
    pc2_var_pct: Option<f64>,
    group_names: Vec<String>,
}

fn is_pca_result(res: &ResultSheet) -> bool {
    matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Pca { .. }))
        || (res.tabs.iter().any(|t| t.id == "pca-loadings") && res.tabs.iter().any(|t| t.id == "pca-eigenvalues"))
}

fn extract_optional_mv_groups(
    doc: &ProjectDocument,
    source_sheet_id: Option<&curvebook_doc::SheetId>,
    selected_cols: &[usize],
    expected_n: usize,
) -> (Vec<Option<String>>, Vec<String>) {
    let Some(sid) = source_sheet_id else {
        return (vec![None; expected_n], Vec::new());
    };
    let Some(ds) = doc.data_sheet(sid) else {
        return (vec![None; expected_n], Vec::new());
    };
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return (vec![None; expected_n], Vec::new());
    };
    let Some(cat_col) = mv.variables.iter().position(|v| v.role == VariableRole::Categorical) else {
        return (vec![None; expected_n], Vec::new());
    };
    let num_cols: Vec<usize> = if selected_cols.is_empty() {
        mv.variables
            .iter()
            .enumerate()
            .filter(|(_, v)| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
            .map(|(i, _)| i)
            .collect()
    } else {
        selected_cols.iter().copied().filter(|&c| c < mv.variables.len()).collect()
    };
    if num_cols.len() < 2 {
        return (vec![None; expected_n], Vec::new());
    }
    let mut row_groups = Vec::new();
    let mut group_names = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for r in 0..mv.row_count() {
        let complete =
            num_cols.iter().all(|&c| mv.variables.get(c).and_then(|v| v.cells.get(r)).and_then(Cell::active_number).is_some_and(f64::is_finite));
        if !complete {
            continue;
        }
        let grp = mv.variables[cat_col].cells.get(r).and_then(|cell| {
            if cell.excluded {
                return None;
            }
            cell.active_text()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .or_else(|| cell.active_number().filter(|v| v.is_finite()).map(|v| format!("{v}")))
        });
        if let Some(ref g) = grp
            && !seen.contains_key(g)
        {
            seen.insert(g.clone(), group_names.len());
            group_names.push(g.clone());
        }
        row_groups.push(grp);
    }
    if row_groups.len() != expected_n || group_names.is_empty() {
        return (vec![None; expected_n], Vec::new());
    }
    (row_groups, group_names)
}

fn compute_biplot_from_datasheet(ds: &curvebook_doc::DataSheet, selected: &[usize], standardize: bool) -> Option<PcaBiplotExtracted> {
    let (var_labels, obs_labels, rows, row_groups, group_names) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<usize> = if selected.is_empty() {
                mv.variables
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                    .map(|(i, _)| i)
                    .collect()
            } else {
                selected.iter().copied().filter(|&c| c < mv.variables.len()).collect()
            };
            if num_cols.len() < 2 {
                return None;
            }
            let label_col = mv.variables.iter().position(|v| v.role == VariableRole::Label);
            let cat_col = mv.variables.iter().position(|v| v.role == VariableRole::Categorical);
            let v_labels: Vec<String> = num_cols.iter().map(|&c| mv.variables[c].title.clone()).collect();
            let mut o_labels = Vec::new();
            let mut mat_rows = Vec::new();
            let mut r_groups = Vec::new();
            let mut g_names = Vec::new();
            let mut seen: BTreeMap<String, usize> = BTreeMap::new();
            for r in 0..mv.row_count() {
                let vals: Option<Vec<f64>> =
                    num_cols.iter().map(|&c| mv.variables.get(c)?.cells.get(r)?.active_number().filter(|v| v.is_finite())).collect();
                if let Some(v) = vals {
                    let ol = label_col
                        .and_then(|lc| mv.variables.get(lc)?.cells.get(r)?.active_text().map(str::to_owned))
                        .or_else(|| mv.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned())
                        .unwrap_or_else(|| format!("Obs {}", r + 1));
                    let grp = cat_col.and_then(|cc| {
                        let cell = mv.variables.get(cc)?.cells.get(r)?;
                        if cell.excluded {
                            return None;
                        }
                        cell.active_text().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
                    });
                    if let Some(ref g) = grp
                        && !seen.contains_key(g)
                    {
                        seen.insert(g.clone(), g_names.len());
                        g_names.push(g.clone());
                    }
                    o_labels.push(ol);
                    mat_rows.push(v);
                    r_groups.push(grp);
                }
            }
            (v_labels, o_labels, mat_rows, r_groups, g_names)
        }
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() || col.data_sets.len() < 2 {
                return None;
            }
            let cols: Vec<usize> = if selected.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                selected.iter().copied().filter(|&c| c < col.data_sets.len()).collect()
            };
            if cols.len() < 2 {
                return None;
            }
            let v_labels: Vec<String> = cols.iter().map(|&c| col.data_sets[c].title.clone()).collect();
            let mut o_labels = Vec::new();
            let mut mat_rows = Vec::new();
            for r in 0..col.row_count() {
                let vals: Option<Vec<f64>> =
                    cols.iter().map(|&c| col.data_sets.get(c)?.cells.get(r)?.first()?.active_number().filter(|v| v.is_finite())).collect();
                if let Some(v) = vals {
                    o_labels.push(format!("Row {}", r + 1));
                    mat_rows.push(v);
                }
            }
            let n = mat_rows.len();
            (v_labels, o_labels, mat_rows, vec![None; n], Vec::new())
        }
        _ => return None,
    };

    let n = rows.len();
    let p = var_labels.len();
    if n < 3 || p < 2 {
        return None;
    }
    let flat: Vec<f64> = rows.into_iter().flat_map(IntoIterator::into_iter).collect();
    let mat = DenseMatrix::from_row_major(n, p, flat).ok()?;
    let pca = pca_analysis(&mat, &var_labels, standardize, None).ok()?;
    if pca.eigenvalues.len() < 2 {
        return None;
    }
    let mut g_index: BTreeMap<&str, usize> = BTreeMap::new();
    for (idx, name) in group_names.iter().enumerate() {
        g_index.insert(name.as_str(), idx);
    }
    let scores = pca
        .scores
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            let pc1 = *row.first()?;
            let pc2 = *row.get(1)?;
            let gl = row_groups.get(i).cloned().flatten();
            let g_idx = gl.as_deref().and_then(|g| g_index.get(g).copied()).unwrap_or(0);
            Some(BiplotScore {
                label: obs_labels.get(i).cloned().unwrap_or_else(|| format!("Obs {}", i + 1)),
                pc1,
                pc2,
                group_idx: g_idx,
                group_label: gl,
            })
        })
        .collect();
    let loadings = pca
        .loadings
        .iter()
        .enumerate()
        .filter_map(|(j, row)| {
            Some(BiplotLoading {
                label: var_labels.get(j).cloned().unwrap_or_else(|| format!("Var {}", j + 1)),
                pc1: *row.first()?,
                pc2: *row.get(1)?,
            })
        })
        .collect();

    Some(PcaBiplotExtracted {
        scores,
        loadings,
        pc1_var_pct: pca.variance_proportion.first().map(|v| v * 100.0),
        pc2_var_pct: pca.variance_proportion.get(1).map(|v| v * 100.0),
        group_names,
    })
}

fn extract_pca_biplot_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<PcaBiplotExtracted> {
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_pca_result) {
        let ev_tab = res.tabs.iter().find(|t| t.id == "pca-eigenvalues");
        let load_tab = res.tabs.iter().find(|t| t.id == "pca-loadings");
        let score_tab = res.tabs.iter().find(|t| t.id == "pca-scores");

        if let (Some(lt), Some(st)) = (load_tab, score_tab)
            && lt.column_headers.len() >= 2
            && st.column_headers.len() >= 2
        {
            let pc1_var_pct = ev_tab.and_then(|t| t.rows.first()).and_then(|r| r.get(1)).and_then(result_cell_num);
            let pc2_var_pct = ev_tab.and_then(|t| t.rows.get(1)).and_then(|r| r.get(1)).and_then(result_cell_num);

            let (row_groups, group_names) =
                extract_optional_mv_groups(doc, res.analysis.source_sheet_id.as_ref(), &res.analysis.selected_data_sets, st.rows.len());
            let mut g_index: BTreeMap<&str, usize> = BTreeMap::new();
            for (idx, name) in group_names.iter().enumerate() {
                g_index.insert(name.as_str(), idx);
            }

            let scores: Vec<BiplotScore> = st
                .rows
                .iter()
                .enumerate()
                .filter_map(|(i, row)| {
                    let pc1 = result_cell_num(row.first()?)?;
                    let pc2 = result_cell_num(row.get(1)?)?;
                    let gl = row_groups.get(i).cloned().flatten();
                    let g_idx = gl.as_deref().and_then(|g| g_index.get(g).copied()).unwrap_or(0);
                    Some(BiplotScore {
                        label: st.row_labels.get(i).cloned().unwrap_or_else(|| format!("Obs {}", i + 1)),
                        pc1,
                        pc2,
                        group_idx: g_idx,
                        group_label: gl,
                    })
                })
                .collect();

            let loadings: Vec<BiplotLoading> = lt
                .rows
                .iter()
                .enumerate()
                .filter_map(|(j, row)| {
                    let pc1 = result_cell_num(row.first()?)?;
                    let pc2 = result_cell_num(row.get(1)?)?;
                    Some(BiplotLoading { label: lt.row_labels.get(j).cloned().unwrap_or_else(|| format!("Var {}", j + 1)), pc1, pc2 })
                })
                .collect();

            if !scores.is_empty() && !loadings.is_empty() {
                return Some(PcaBiplotExtracted { scores, loadings, pc1_var_pct, pc2_var_pct, group_names });
            }
        }

        // Legacy PCA result sheet without `pca-scores` tab: recompute from source DataSheet using the spec's `standardize` flag.
        let standardize = match &res.analysis.kind {
            AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize, .. }) => *standardize,
            _ => true,
        };
        if let Some(src_ds) = res.analysis.source_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid)) {
            return compute_biplot_from_datasheet(src_ds, &res.analysis.selected_data_sets, standardize);
        }
    }

    if has_blocked_matching_result(graph, doc, is_pca_result) {
        return None;
    }

    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
    compute_biplot_from_datasheet(ds, &[], true)
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::PcaBiplot`] combining observation scores
/// `(PC1, PC2)` with Gabriel-scaled variable loading arrows and variance-explained axis titles.
pub(in crate::multivariate_layout) fn compile_pca_biplot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let Some(mut data) = extract_pca_biplot_data(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new(
            "empty_pca_biplot",
            "no PCA scores and loadings available for biplot (requires >= 2 variables and >= 3 observations)",
        ));
        draw_empty_open_frame(scene);
        return Ok(());
    };
    data.scores.retain(|s| s.pc1.is_finite() && s.pc2.is_finite());
    data.loadings.retain(|l| l.pc1.is_finite() && l.pc2.is_finite());
    if data.scores.is_empty() || data.loadings.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_pca_biplot", "no finite PC1/PC2 coordinates available for biplot"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    // Gabriel biplot arrow scaling: scale unit-norm loading vectors to ~80% of the score radius.
    let score_radius = data.scores.iter().map(|s| s.pc1.abs().max(s.pc2.abs())).fold(0.0_f64, f64::max).max(1e-6);
    let max_loading_norm = data.loadings.iter().map(|l| l.pc1.hypot(l.pc2)).fold(0.0_f64, f64::max).max(1e-6);
    let arrow_scale = 0.80 * score_radius / max_loading_norm;

    let mut x_min = 0.0_f64;
    let mut x_max = 0.0_f64;
    let mut y_min = 0.0_f64;
    let mut y_max = 0.0_f64;
    for s in &data.scores {
        x_min = x_min.min(s.pc1);
        x_max = x_max.max(s.pc1);
        y_min = y_min.min(s.pc2);
        y_max = y_max.max(s.pc2);
    }
    for l in &data.loadings {
        let lx = l.pc1 * arrow_scale;
        let ly = l.pc2 * arrow_scale;
        x_min = x_min.min(lx);
        x_max = x_max.max(lx);
        y_min = y_min.min(ly);
        y_max = y_max.max(ly);
    }
    let x_pad = (x_max - x_min).max(1e-3) * 0.18;
    let y_pad = (y_max - y_min).max(1e-3) * 0.18;

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = match data.pc1_var_pct {
            Some(pct) if pct.is_finite() => format!("PC1 ({pct:.1}%)"),
            _ => String::from("PC1"),
        };
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = match data.pc2_var_pct {
            Some(pct) if pct.is_finite() => format!("PC2 ({pct:.1}%)"),
            _ => String::from("PC2"),
        };
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

    // 1. Subtle dashed origin crosshairs (PC1 = 0, PC2 = 0).
    let origin_stroke = solid_stroke(Rgba8::rgba(130, 138, 150, 140), 0.75, LineDashStyle::Dashed);
    if let Some((_, x0)) = x_axis.map_value(0.0) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(x0, r.y), PointPt::new(x0, r.bottom())),
            fill: None,
            stroke: Some(origin_stroke),
            clip: Some(r),
        });
    }
    if let Some((_, y0)) = y_axis.map_value(0.0) {
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, y0), PointPt::new(r.right(), y0)),
            fill: None,
            stroke: Some(origin_stroke),
            clip: Some(r),
        });
    }

    // 2. Observation score scatter markers.
    let sheet_id = graph.spec.primary_source_sheet_id.clone();
    let show_obs_labels = data.scores.len() <= 28;
    for (idx, sc) in data.scores.iter().enumerate() {
        let (Some((_, px)), Some((_, py))) = (x_axis.map_value(sc.pc1), y_axis.map_value(sc.pc2)) else {
            continue;
        };
        let col = graph.spec.palette.color_at(sc.group_idx);
        let marker = MarkerStyle {
            shape: MarkerShape::Circle,
            fill_mode: MarkerFillMode::Filled,
            size_pt: 5.2,
            fill_color: col,
            stroke_color: Rgba8::WHITE,
            stroke_width_pt: 0.6,
        };
        let center = PointPt::new(px, py);
        emit_marker(&mut scene.primitives, center, &marker, Some(r));
        if show_obs_labels {
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(sc.label.clone()),
                anchor_pt: PointPt::new((px + 4.0).min(r.right() - 4.0), (py - 3.5).max(r.y + 8.0)),
                anchor: TextAnchor::Start,
                font_size_pt: 7.0,
                rotation_deg: 0.0,
                color: Rgba8::rgb(70, 76, 86),
            }));
        }
        scene.plotted_points.push(PlottedPointMeta {
            center_pt: center,
            radius_pt: 3.5,
            source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: idx, col: 0, subcol: 0 },
            series_label: sc.group_label.clone().unwrap_or_else(|| String::from("Scores")),
            value_summary: format!("{}: PC1={}, PC2={}", sc.label, format_point_value(sc.pc1), format_point_value(sc.pc2)),
        });
    }

    // 3. Scaled variable loading vectors (arrows from origin + arrowheads + variable labels).
    let arrow_color = Rgba8::rgb(204, 51, 51);
    let arrow_stroke = solid_stroke(arrow_color, 1.35, LineDashStyle::Solid);
    if let (Some((_, ox)), Some((_, oy))) = (x_axis.map_value(0.0), y_axis.map_value(0.0)) {
        let origin = PointPt::new(ox, oy);
        for load in &data.loadings {
            let lx = load.pc1 * arrow_scale;
            let ly = load.pc2 * arrow_scale;
            let (Some((_, tx)), Some((_, ty))) = (x_axis.map_value(lx), y_axis.map_value(ly)) else {
                continue;
            };
            let tip = PointPt::new(tx, ty);
            scene.primitives.push(ScenePrimitive::Path { path: Path2D::line(origin, tip), fill: None, stroke: Some(arrow_stroke), clip: Some(r) });

            let dx = tx - ox;
            let dy = ty - oy;
            let len = dx.hypot(dy);
            if len > 2.0 {
                let ux = dx / len;
                let uy = dy / len;
                let head_len = 6.5_f64.min(len * 0.45);
                let head_w = 2.8;
                let base_x = tx - ux * head_len;
                let base_y = ty - uy * head_len;
                let left_pt = PointPt::new(base_x - uy * head_w, base_y + ux * head_w);
                let right_pt = PointPt::new(base_x + uy * head_w, base_y - ux * head_w);
                let mut head = Path2D::new();
                path_move(&mut head, tip);
                path_line(&mut head, left_pt);
                path_line(&mut head, right_pt);
                head.commands.push(crate::scene::PathCmd::Close);
                scene.primitives.push(ScenePrimitive::Path {
                    path: head,
                    fill: Some(crate::scene::FillPaint { color: arrow_color }),
                    stroke: None,
                    clip: Some(r),
                });

                let lbl_x = (tx + ux * 5.0).clamp(r.x + 12.0, r.right() - 12.0);
                let lbl_y = (ty + uy * 5.0).clamp(r.y + 10.0, r.bottom() - 4.0);
                let anchor = if ux > 0.25 {
                    TextAnchor::Start
                } else if ux < -0.25 {
                    TextAnchor::End
                } else {
                    TextAnchor::Middle
                };
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: RichText::plain(load.label.clone()),
                    anchor_pt: PointPt::new(lbl_x, lbl_y),
                    anchor,
                    font_size_pt: 8.2,
                    rotation_deg: 0.0,
                    color: arrow_color,
                }));
            }
        }
    }

    // 4. Compact group legend when categorical groups colored the score points.
    if !data.group_names.is_empty() && graph.spec.legend.visible {
        let mut ly = r.y + 10.0;
        let lx = r.right() - 6.0;
        for (idx, gname) in data.group_names.iter().take(8).enumerate() {
            let col = graph.spec.palette.color_at(idx);
            let m = MarkerStyle {
                shape: MarkerShape::Circle,
                fill_mode: MarkerFillMode::Filled,
                size_pt: 4.6,
                fill_color: col,
                stroke_color: Rgba8::WHITE,
                stroke_width_pt: 0.5,
            };
            emit_marker(&mut scene.primitives, PointPt::new(lx - 4.0, ly - 2.5), &m, Some(r));
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(gname.clone()),
                anchor_pt: PointPt::new(lx - 10.0, ly),
                anchor: TextAnchor::End,
                font_size_pt: 7.5,
                rotation_deg: 0.0,
                color: Rgba8::rgb(45, 50, 60),
            }));
            ly += 11.0;
        }
    }

    Ok(())
}
