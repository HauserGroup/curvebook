//! Clustered heatmap diagnostic plot with perceptual colorbar and optional hierarchical dendrogram.

use curvebook_doc::{
    AnalysisKind, ClusterDistance, ClusterLinkage, ColorChannelMapping, GraphSheet, HeatmapColumnLabelAngle, HeatmapColumnLabelMode,
    HeatmapRowLabelMode, MultivariateSpec, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, TableArchetype,
    VariableRole,
};
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, Rgba8};
use curvebook_stats::multivariate::{DendrogramMerge, hierarchical_clustering_pairwise_complete, hierarchical_clustering_with_metrics};
use curvebook_text::{RichText, parse_markup};

use super::super::multivariate::cell_token;
use super::{find_matching_result_where_checked, resolve_source_data_sheet, result_cell_num};
use crate::scene::{ChartError, ChartScene, ChartWarning, FillPaint, Path2D, PointPt, PositionedText, RectPt, ScenePrimitive, TextAnchor};
use crate::survival_cat::{
    PlotMarginConfig, draw_colorbar, draw_empty_open_frame, path_line, path_move, sanitize_range, solid_stroke, text_metrics,
    update_plot_rect_for_labels,
};

// 2a. Heatmap (+ Perceptual Colorbar + Optional Hierarchical Dendrogram)

struct HeatmapData {
    row_labels: Vec<String>,
    col_labels: Vec<String>,
    values: Vec<Vec<Option<f64>>>,
    merges: Vec<DendrogramMerge>,
    cluster_distance: ClusterDistance,
    cluster_linkage: ClusterLinkage,
    clustering_cue: Option<String>,
}

fn cluster_metric_labels(distance: ClusterDistance, linkage: ClusterLinkage) -> (&'static str, &'static str) {
    let d = match distance {
        ClusterDistance::Euclidean => "Euclidean",
        ClusterDistance::Manhattan => "Manhattan",
        ClusterDistance::Pearson => "Pearson",
        ClusterDistance::Cosine => "Cosine",
    };
    let l = match linkage {
        ClusterLinkage::Ward => "Ward",
        ClusterLinkage::Complete => "Complete",
        ClusterLinkage::Single => "Single",
        ClusterLinkage::Upgma => "UPGMA",
        ClusterLinkage::Wpgma => "WPGMA",
        ClusterLinkage::Centroid => "Centroid",
        ClusterLinkage::Median => "Median",
    };
    (d, l)
}

pub(in crate::multivariate_layout) fn compile_heatmap(graph: &GraphSheet, doc: &ProjectDocument, scene: &mut ChartScene) -> Result<(), ChartError> {
    let Some(mut hm) = extract_heatmap_data(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new("empty_heatmap", "no numeric matrix available for heatmap"));
        draw_empty_open_frame(scene);
        return Ok(());
    };

    let n_rows = hm.values.len();
    let n_cols = hm.col_labels.len();
    if n_rows == 0 || n_cols == 0 {
        draw_empty_open_frame(scene);
        return Ok(());
    }

    if !hm.values.iter().flatten().any(|v| v.is_some_and(f64::is_finite)) {
        scene.warnings.push(ChartWarning::new("empty_heatmap", "no finite numeric values available for heatmap"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    if hm.merges.is_empty() && (2..=128).contains(&n_rows) && n_cols >= 2 {
        hm.merges = cluster_heatmap_rows(&hm.values, n_cols, hm.cluster_distance, hm.cluster_linkage, &mut scene.warnings);
    }

    let row_order = if !hm.merges.is_empty() { dendrogram_leaf_order(&hm.merges, n_rows) } else { (0..n_rows).collect() };
    let has_dendro = !hm.merges.is_empty();

    let (min_v, max_v) = heatmap_min_max(&hm.values);
    let has_neg_and_pos = min_v < 0.0 && max_v > 0.0;
    let (ramp, dom_lo, dom_hi) = match graph.spec.multivariate.as_ref().map(|m| &m.color) {
        Some(ColorChannelMapping::Continuous { ramp, domain_clamp, .. }) => {
            let (lo, hi) = domain_clamp.map_or((min_v, max_v), |(a, b)| sanitize_range(a, b, (min_v, max_v)));
            (ramp.clone(), lo, hi)
        }
        _ if has_neg_and_pos => {
            let m = min_v.abs().max(max_v.abs()).max(1e-6);
            (ColorRamp::Named { id: ColormapId::DivergingBlueRed, reversed: false }, -m, m)
        }
        _ => (ColorRamp::Named { id: ColormapId::Viridis, reversed: false }, min_v, max_v),
    };

    let lbl_spec = graph.spec.heatmap_labels;
    let max_row_lbl_w = match lbl_spec.row_labels {
        HeatmapRowLabelMode::RowTitles => hm.row_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max),
        HeatmapRowLabelMode::RowNumbers => text_metrics(&format!("{n_rows}"), 7.5).width_pt,
        HeatmapRowLabelMode::None => 0.0,
    };
    let resolved_col_labels: Vec<String> = match lbl_spec.column_labels {
        HeatmapColumnLabelMode::ColumnTitles => hm.col_labels.clone(),
        HeatmapColumnLabelMode::ColumnLetters => (0..n_cols).map(spreadsheet_column_letters).collect(),
        HeatmapColumnLabelMode::None => Vec::new(),
    };
    let max_col_lbl_w = resolved_col_labels.iter().map(|l| text_metrics(l, 7.5).width_pt).fold(0.0_f64, f64::max);
    let col_lbl_proj_h = if lbl_spec.column_labels == HeatmapColumnLabelMode::None || resolved_col_labels.is_empty() {
        0.0
    } else {
        match lbl_spec.column_angle {
            HeatmapColumnLabelAngle::Horizontal0 => 14.0,
            HeatmapColumnLabelAngle::Angled45 => (max_col_lbl_w * std::f64::consts::FRAC_1_SQRT_2 + 10.0).max(16.0),
            HeatmapColumnLabelAngle::Vertical90 => (max_col_lbl_w + 10.0).max(16.0),
        }
    };
    let cue_extra = if has_dendro && hm.clustering_cue.is_some() { 13.0 } else { 0.0 };
    let desired_bottom_extra = (col_lbl_proj_h + cue_extra - 10.0).max(if has_dendro && hm.clustering_cue.is_some() { 26.0 } else { 18.0 });
    let _ = update_plot_rect_for_labels(
        scene,
        &graph.spec,
        &[],
        PlotMarginConfig {
            extra_bottom_pt: desired_bottom_extra.min(scene.height_pt * 0.35),
            extra_left_pt: crate::clamp_f64_safe(max_row_lbl_w + if has_dendro { 34.0 } else { 8.0 }, 12.0, 90.0),
            has_colorbar: true,
            suppress_y_axis_labels: true,
            ..PlotMarginConfig::default()
        },
    );

    let r = scene.plot_rect_pt;
    let dendro_w = if has_dendro { crate::clamp_f64_safe(r.width * 0.16, 20.0, 48.0) } else { 0.0 };
    let grid = RectPt::new(r.x + dendro_w, r.y, (r.width - dendro_w).max(20.0), r.height);
    let cell_w = grid.width / (n_cols as f64);
    let cell_h = grid.height / (n_rows as f64);
    let gap = if lbl_spec.cell_gap_pt.is_finite() { lbl_spec.cell_gap_pt.clamp(0.0, (cell_w.min(cell_h) * 0.4).max(0.0)) } else { 0.0 };
    let half_gap = gap * 0.5;
    let cell_border = solid_stroke(Rgba8::rgba(255, 255, 255, 160), 0.5, LineDashStyle::Solid);
    let missing_border = solid_stroke(Rgba8::rgba(170, 176, 186, 180), 0.5, LineDashStyle::Solid);
    let missing_hatch = solid_stroke(Rgba8::rgba(150, 156, 166, 175), 0.65, LineDashStyle::Solid);
    let denom = (dom_hi - dom_lo).abs().max(1e-12);

    for (vis_row, &orig_row) in row_order.iter().enumerate() {
        let Some(row_vals) = hm.values.get(orig_row) else { continue };
        let y = grid.y + (vis_row as f64) * cell_h;
        for c in 0..n_cols {
            let x = grid.x + (c as f64) * cell_w;
            let tile_rect = RectPt::new(x + half_gap, y + half_gap, (cell_w - gap).max(1.0), (cell_h - gap).max(1.0));
            if let Some(val) = row_vals.get(c).copied().flatten().filter(|v| v.is_finite()) {
                let t = crate::clamp_f64_safe((val - dom_lo) / denom, 0.0, 1.0);
                let fill_color = ramp.sample(t);
                scene.primitives.push(ScenePrimitive::Path {
                    path: Path2D::rect(tile_rect),
                    fill: Some(FillPaint::solid(fill_color)),
                    stroke: Some(cell_border),
                    clip: None,
                });
                if lbl_spec.annotate_cells {
                    let font_pt = crate::clamp_f64_safe(tile_rect.width.min(tile_rect.height) * 0.36, 5.5, 9.0);
                    let ink =
                        if Rgba8::WHITE.contrast_ratio(fill_color) >= Rgba8::BLACK.contrast_ratio(fill_color) { Rgba8::WHITE } else { Rgba8::BLACK };
                    scene.primitives.push(ScenePrimitive::Text(PositionedText {
                        rich: RichText::plain(crate::scales::format_scalar_clean(val)),
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
                let lbl = hm.row_labels.get(orig_row).map(String::as_str).unwrap_or("");
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: parse_markup(lbl),
                    anchor_pt: PointPt::new(r.x - 5.0, y + cell_h * 0.5 + 3.0),
                    anchor: TextAnchor::End,
                    font_size_pt: 7.5,
                    rotation_deg: 0.0,
                    color: Rgba8::BLACK,
                }));
            }
            HeatmapRowLabelMode::RowNumbers => {
                scene.primitives.push(ScenePrimitive::Text(PositionedText {
                    rich: RichText::plain(format!("{}", orig_row + 1)),
                    anchor_pt: PointPt::new(r.x - 5.0, y + cell_h * 0.5 + 3.0),
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

    if has_dendro {
        draw_left_dendrogram(scene, RectPt::new(r.x, r.y, dendro_w - 3.0, grid.height), &hm.merges, &row_order, cell_h);
        if let Some(cue) = &hm.clustering_cue {
            let cue_y = (grid.bottom() + col_lbl_proj_h.max(14.0) + 7.5).min(scene.height_pt - 4.0);
            scene.primitives.push(ScenePrimitive::Text(PositionedText {
                rich: RichText::plain(cue.clone()),
                anchor_pt: PointPt::new(grid.x + grid.width * 0.5, cue_y),
                anchor: TextAnchor::Middle,
                font_size_pt: 7.0,
                rotation_deg: 0.0,
                color: Rgba8::rgb(85, 92, 102),
            }));
        }
    }

    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::rect(grid),
        fill: None,
        stroke: Some(solid_stroke(Rgba8::BLACK, 0.85, LineDashStyle::Solid)),
        clip: None,
    });
    draw_colorbar(scene, &ramp, dom_lo, dom_hi, "", RectPt::new(grid.right() + 10.0, grid.y + 4.0, 12.0, (grid.height - 8.0).max(24.0)));
    Ok(())
}

fn spreadsheet_column_letters(mut index: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push((b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

fn correlation_heatmap_data(res: &ResultSheet, tab: &ResultTab) -> HeatmapData {
    let mut matrix = Vec::with_capacity(tab.rows.len());
    for row in &tab.rows {
        let vals: Vec<Option<f64>> = row
            .iter()
            .map(|c| match c {
                ResultCell::Number { value, .. } if value.is_finite() => Some(*value),
                _ => None,
            })
            .collect();
        matrix.push(vals);
    }
    let n_rows = matrix.len();
    let has_missing = matrix.iter().flatten().any(Option::is_none);
    let merges = if has_missing {
        Vec::new()
    } else {
        res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram").map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, n_rows))
    };
    HeatmapData {
        row_labels: tab.row_labels.clone(),
        col_labels: tab.column_headers.clone(),
        values: matrix,
        merges,
        cluster_distance: ClusterDistance::Euclidean,
        cluster_linkage: ClusterLinkage::Ward,
        clustering_cue: None,
    }
}

fn resolve_selected_heatmap_cols(selected: &[usize], len: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for &idx in selected {
        if idx < len && !out.contains(&idx) {
            out.push(idx);
        }
    }
    out
}

fn extract_heatmap_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<HeatmapData> {
    let direct_res = doc.result_sheet(&graph.spec.primary_source_sheet_id);
    if let Some(res) = direct_res
        && matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
    {
        if matches!(res.status, ResultStatus::Failed { .. }) {
            warnings.push(ChartWarning::new("failed_result_sheet", format!("result sheet '{}' failed; omitted stale heatmap matrix", res.id)));
            return None;
        }
        if let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary) {
            return Some(correlation_heatmap_data(res, tab));
        }
    }

    let clustering_res = find_matching_result_where_checked(graph, doc, warnings, |r| {
        matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }))
    });

    if clustering_res.is_none()
        && direct_res.is_none()
        && let Some(res) = find_matching_result_where_checked(graph, doc, warnings, |r| {
            matches!(r.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::CorrelationMatrix { .. }))
        })
        && let Some(tab) = res.tabs.iter().find(|t| t.kind == ResultTabKind::MainSummary)
    {
        return Some(correlation_heatmap_data(res, tab));
    }

    let selected_cols = clustering_res.map(|r| r.analysis.selected_data_sets.as_slice()).unwrap_or(&[]);
    let ds = resolve_source_data_sheet(graph, doc)?;
    let (mut hm, expect_rows) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<(usize, String)> = if selected_cols.is_empty() {
                mv.variables
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.role == VariableRole::Continuous || v.role == VariableRole::Calculated)
                    .map(|(i, v)| (i, v.title.clone()))
                    .collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, mv.variables.len())
                    .into_iter()
                    .filter_map(|i| {
                        let v = mv.variables.get(i)?;
                        (v.role == VariableRole::Continuous || v.role == VariableRole::Calculated).then(|| (i, v.title.clone()))
                    })
                    .collect()
            };
            if num_cols.is_empty() {
                return None;
            }
            let label_col = graph
                .spec
                .multivariate
                .as_ref()
                .and_then(|m| m.label_column)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Label));
            let mut row_labels = Vec::new();
            let mut values = Vec::new();
            for r in 0..mv.row_count() {
                let row_vals: Vec<Option<f64>> = num_cols
                    .iter()
                    .map(|(ci, _)| mv.variables.get(*ci).and_then(|v| v.cells.get(r)).and_then(|c| c.active_number()).filter(|x| x.is_finite()))
                    .collect();
                let lbl = label_col
                    .and_then(|lc| mv.variables.get(lc))
                    .and_then(|v| cell_token(v, r))
                    .or_else(|| mv.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned())
                    .unwrap_or_else(|| format!("R{}", r + 1));
                row_labels.push(lbl);
                values.push(row_vals);
            }
            let n_rows = values.len();
            let col_labels = num_cols.into_iter().map(|(_, t)| t).collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                n_rows,
            )
        }
        TableArchetype::Column(col) => {
            let col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..col.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, col.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| col.data_sets.get(c).map(|d| d.title.clone())).collect();
            let nrows = col.row_count();
            let row_labels =
                (0..nrows).map(|r| col.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col_indices
                        .iter()
                        .map(|&c| {
                            col.data_sets
                                .get(c)
                                .and_then(|d| d.cells.get(r))
                                .and_then(|s| s.first())
                                .and_then(|c| c.active_number())
                                .filter(|x| x.is_finite())
                        })
                        .collect()
                })
                .collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                nrows,
            )
        }
        TableArchetype::Grouped(g) => {
            let col_indices: Vec<usize> = if selected_cols.is_empty() {
                (0..g.data_sets.len()).collect()
            } else {
                resolve_selected_heatmap_cols(selected_cols, g.data_sets.len())
            };
            if col_indices.is_empty() {
                return None;
            }
            let col_labels: Vec<String> = col_indices.iter().filter_map(|&c| g.data_sets.get(c).map(|d| d.title.clone())).collect();
            let nrows = g.row_count();
            let row_labels =
                (0..nrows).map(|r| g.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("R{}", r + 1))).collect();
            let values = (0..nrows)
                .map(|r| {
                    col_indices
                        .iter()
                        .map(|&c| {
                            if g.subcolumn_mode.is_pre_summarized() {
                                g.observation(r, c, 0).and_then(curvebook_doc::Cell::active_number).filter(|x| x.is_finite())
                            } else {
                                let reps: Vec<f64> = g.active_replicates(r, c).into_iter().filter(|x| x.is_finite()).collect();
                                if reps.is_empty() {
                                    None
                                } else {
                                    let mean = reps.iter().sum::<f64>() / (reps.len() as f64);
                                    mean.is_finite().then_some(mean)
                                }
                            }
                        })
                        .collect()
                })
                .collect();
            (
                HeatmapData {
                    row_labels,
                    col_labels,
                    values,
                    merges: Vec::new(),
                    cluster_distance: ClusterDistance::Euclidean,
                    cluster_linkage: ClusterLinkage::Ward,
                    clustering_cue: None,
                },
                nrows,
            )
        }
        _ => return None,
    };

    if let Some(cres) = clustering_res
        && let AnalysisKind::Multivariate(MultivariateSpec::Clustering { distance, linkage, .. }) = cres.analysis.kind
    {
        hm.cluster_distance = distance;
        hm.cluster_linkage = linkage;
        let (dist_lbl, link_lbl) = cluster_metric_labels(distance, linkage);
        hm.clustering_cue = Some(format!("Clustered: {dist_lbl} / {link_lbl}"));
        let has_missing = hm.values.iter().flatten().any(Option::is_none);
        if !has_missing {
            let tab_merges = cres
                .tabs
                .iter()
                .find(|t| t.id == "hierarchical-dendrogram")
                .map_or_else(Vec::new, |t| extract_dendro_merges_from_tab(t, expect_rows));
            if !tab_merges.is_empty() && tab_merges.len() + 1 == expect_rows {
                hm.merges = tab_merges;
            }
        }
    }

    Some(hm)
}

fn parse_dendro_node_id(raw: f64, step: usize, n_leaves: usize) -> Option<isize> {
    if !raw.is_finite() || raw.fract() != 0.0 || raw.abs() > 9_007_199_254_740_991.0 {
        return None;
    }
    let min_leaf_id = isize::try_from(n_leaves).ok().and_then(isize::checked_neg)?;
    let step_i = isize::try_from(step).ok()?;
    let id = raw as isize;
    ((id as f64) == raw && id >= min_leaf_id && id < step_i).then_some(id)
}

fn decode_dendro_leaf(id: isize, n_leaves: usize) -> Option<usize> {
    if id >= 0 {
        return None;
    }
    let pos = id.checked_neg()?;
    let leaf = pos.checked_sub(1)?;
    let idx = usize::try_from(leaf).ok()?;
    (idx < n_leaves).then_some(idx)
}

fn extract_dendro_merges_from_tab(tab: &ResultTab, n_leaves: usize) -> Vec<DendrogramMerge> {
    if n_leaves < 2 || tab.rows.len() + 1 != n_leaves {
        return Vec::new();
    }
    let mut merges = Vec::with_capacity(tab.rows.len());
    for (step, row) in tab.rows.iter().enumerate() {
        let (Some(l), Some(r), Some(d), Some(sz)) = (
            row.first().and_then(result_cell_num),
            row.get(1).and_then(result_cell_num),
            row.get(2).and_then(result_cell_num),
            row.get(3).and_then(result_cell_num),
        ) else {
            return Vec::new();
        };
        let (Some(left), Some(right)) = (parse_dendro_node_id(l, step, n_leaves), parse_dendro_node_id(r, step, n_leaves)) else {
            return Vec::new();
        };
        if !d.is_finite() || d < 0.0 || left == right || !sz.is_finite() || sz.fract() != 0.0 || sz < 1.0 || sz > 9_007_199_254_740_991.0 {
            return Vec::new();
        }
        let cluster_size = sz as usize;
        if (cluster_size as f64) != sz || cluster_size == 0 || cluster_size > n_leaves {
            return Vec::new();
        }
        merges.push(DendrogramMerge { left, right, distance: d, cluster_size });
    }
    merges
}

fn heatmap_min_max(values: &[Vec<Option<f64>>]) -> (f64, f64) {
    let (lo, hi) = values
        .iter()
        .flat_map(|r| r.iter())
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(v), b.max(v)));
    sanitize_range(lo, hi, (0.0, 1.0))
}

fn cluster_heatmap_rows(
    rows: &[Vec<Option<f64>>],
    n_cols: usize,
    distance: ClusterDistance,
    linkage: ClusterLinkage,
    warnings: &mut Vec<ChartWarning>,
) -> Vec<DendrogramMerge> {
    let is_complete = rows.iter().all(|r| r.len() >= n_cols && r[..n_cols].iter().all(|v| v.is_some_and(f64::is_finite)));
    if is_complete {
        let dense: Vec<Vec<f64>> = rows.iter().map(|r| r.iter().take(n_cols).flatten().copied().collect()).collect();
        return hierarchical_clustering_with_metrics(&dense, distance, linkage).unwrap_or_default();
    }
    if let Ok(Some(merges)) = hierarchical_clustering_pairwise_complete(rows, n_cols, distance, linkage) {
        return merges;
    }
    warnings.push(ChartWarning::new(
        "clustering_skipped_missing_data",
        "hierarchical clustering skipped because rows lack overlapping finite observations",
    ));
    Vec::new()
}

fn dendrogram_leaf_order(merges: &[DendrogramMerge], n_leaves: usize) -> Vec<usize> {
    if merges.is_empty() || n_leaves == 0 {
        return (0..n_leaves).collect();
    }
    let Ok(root_id) = isize::try_from(merges.len() - 1) else {
        return (0..n_leaves).collect();
    };
    let mut order = Vec::with_capacity(n_leaves);
    let mut seen_leaves = vec![false; n_leaves];
    let mut visited_merges = vec![false; merges.len()];
    let mut stack: Vec<isize> = vec![root_id];
    let mut guard = 0_usize;
    let max_steps = n_leaves.saturating_mul(4).saturating_add(16);
    while let Some(node) = stack.pop() {
        guard += 1;
        if guard > max_steps {
            break;
        }
        if node < 0 {
            if let Some(leaf) = decode_dendro_leaf(node, n_leaves)
                && let Some(seen) = seen_leaves.get_mut(leaf)
                && !*seen
            {
                *seen = true;
                order.push(leaf);
            }
        } else if let Ok(merge_idx) = usize::try_from(node)
            && let Some(visited) = visited_merges.get_mut(merge_idx)
            && !*visited
            && let Some(m) = merges.get(merge_idx)
        {
            *visited = true;
            stack.push(m.right);
            stack.push(m.left);
        }
    }
    for (i, seen) in seen_leaves.iter().enumerate() {
        if !*seen {
            order.push(i);
        }
    }
    order
}

fn draw_left_dendrogram(scene: &mut ChartScene, box_rect: RectPt, merges: &[DendrogramMerge], leaf_order: &[usize], cell_h: f64) {
    let max_dist =
        merges.iter().filter_map(|m| (m.distance.is_finite() && m.distance >= 0.0).then_some(m.distance)).fold(0.0_f64, f64::max).max(1e-9);
    let n_leaves = leaf_order.len();
    let mut node_y = vec![0.0_f64; n_leaves];
    for (vis_idx, &orig_leaf) in leaf_order.iter().enumerate() {
        if let Some(slot) = node_y.get_mut(orig_leaf) {
            *slot = box_rect.y + ((vis_idx as f64) + 0.5) * cell_h;
        }
    }
    let mut merge_coords: Vec<(f64, f64)> = vec![(box_rect.right(), box_rect.y); merges.len()];
    let lookup_pos = |id: isize, mc: &[(f64, f64)]| -> (f64, f64) {
        if id < 0 {
            let y = decode_dendro_leaf(id, n_leaves).and_then(|leaf| node_y.get(leaf).copied()).unwrap_or(box_rect.y);
            (box_rect.right(), y)
        } else {
            usize::try_from(id).ok().and_then(|idx| mc.get(idx).copied()).unwrap_or((box_rect.right(), box_rect.y))
        }
    };
    let stroke = solid_stroke(Rgba8::rgb(65, 70, 80), 0.9, LineDashStyle::Solid);
    for (step, m) in merges.iter().enumerate() {
        let (x_l, y_l) = lookup_pos(m.left, &merge_coords);
        let (x_r, y_r) = lookup_pos(m.right, &merge_coords);
        let dist_frac = if m.distance.is_finite() { (m.distance / max_dist).clamp(0.0, 1.0) } else { 0.0 };
        let x_m = box_rect.right() - dist_frac * box_rect.width;
        let mut p = Path2D::new();
        path_move(&mut p, PointPt::new(x_l, y_l));
        path_line(&mut p, PointPt::new(x_m, y_l));
        path_line(&mut p, PointPt::new(x_m, y_r));
        path_line(&mut p, PointPt::new(x_r, y_r));
        scene.primitives.push(ScenePrimitive::Path { path: p, fill: None, stroke: Some(stroke), clip: None });
        if let Some(slot) = merge_coords.get_mut(step) {
            *slot = (x_m, 0.5 * (y_l + y_r));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::scene::{ChartScene, ScenePrimitive};

    #[test]
    fn dendrogram_node_conversion_guards_extreme_and_malformed_ids() {
        use curvebook_doc::{ResultCell, ResultTab, ResultTabKind};
        use curvebook_stats::multivariate::DendrogramMerge;

        use super::{dendrogram_leaf_order, draw_left_dendrogram, extract_dendro_merges_from_tab};
        use crate::scene::{PathCmd, RectPt};

        for bad_row in [
            vec![
                ResultCell::Number { value: -1e300, prefix: None, units: None },
                ResultCell::Number { value: 1e300, prefix: None, units: None },
                ResultCell::Number { value: 0.5, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: -1.5, prefix: None, units: None },
                ResultCell::Number { value: -2.0, prefix: None, units: None },
                ResultCell::Number { value: 0.25, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
            vec![
                ResultCell::Number { value: isize::MIN as f64, prefix: None, units: None },
                ResultCell::Number { value: -1.0, prefix: None, units: None },
                ResultCell::Number { value: f64::NAN, prefix: None, units: None },
                ResultCell::Number { value: 2.0, prefix: None, units: None },
            ],
        ] {
            let tab = ResultTab {
                id: String::from("hierarchical-dendrogram"),
                title: String::from("Dendrogram"),
                kind: ResultTabKind::MainSummary,
                column_headers: vec![],
                row_labels: vec![],
                rows: vec![bad_row],
                footnotes: vec![],
            };
            assert!(extract_dendro_merges_from_tab(&tab, 2).is_empty());
        }

        let malformed = [
            DendrogramMerge { left: isize::MIN, right: isize::MAX, distance: f64::NAN, cluster_size: usize::MAX },
            DendrogramMerge { left: 0, right: 1, distance: f64::INFINITY, cluster_size: 2 },
            DendrogramMerge { left: -999, right: 0, distance: -4.0, cluster_size: 3 },
        ];
        let order = dendrogram_leaf_order(&malformed, 4);
        assert_eq!(order.len(), 4);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![0, 1, 2, 3]);

        let mut scene = ChartScene::new("hm-bad", "Bad Dendro", 320.0, 240.0, None);
        draw_left_dendrogram(&mut scene, RectPt::new(20.0, 20.0, 40.0, 120.0), &malformed, &order, 30.0);
        assert!(!scene.primitives.is_empty());
        for prim in &scene.primitives {
            if let ScenePrimitive::Path { path, .. } = prim {
                for cmd in &path.commands {
                    if let PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) = cmd {
                        assert!(pt.x.is_finite() && pt.y.is_finite(), "non-finite dendrogram pt: {pt:?}");
                    }
                }
            }
        }
    }
}
