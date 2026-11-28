//! Standalone hierarchical clustering dendrogram diagnostic plot with merge-height axis,
//! ordered leaf labels, optional cut-height reference line, and subtree cluster coloring.
//!
//! References:
//! - Ward, J. H. (1963). Hierarchical grouping to optimize an objective function. *Journal of the
//!   American Statistical Association*, 58(301), 236–244.
//! - Murtagh, F., & Legendre, P. (2014). Ward's hierarchical agglomerative clustering method:
//!   Which algorithms implement Ward's criterion? *Journal of Classification*, 31(3), 274–295.

use curvebook_doc::{
    AnalysisKind, ClusterDirection, ClusterDistance, ClusterLinkage, GraphSheet, MultivariateSpec, ProjectDocument, ResultSheet, TableArchetype,
    VariableRole,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_stats::multivariate::{DendrogramMerge, cut_dendrogram_at_height, hierarchical_clustering_with_metrics};
use curvebook_text::RichText;

use super::heatmap::{decode_dendro_leaf, dendrogram_leaf_order, extract_dendro_merges_from_tab};
use super::{find_matching_result_where_checked, has_blocked_matching_result, resolve_source_data_sheet_checked};
use crate::scene::{
    ChartError, ChartScene, ChartWarning, Path2D, PlottedPointMeta, PointPt, PositionedText, ScenePrimitive, SourceCellId, TextAnchor,
};
use crate::survival_cat::{
    PlotMarginConfig, SegmentedFrameSpec, draw_empty_open_frame, layout_and_render_segmented_chrome, path_line, path_move, solid_stroke,
};
use crate::xy::format_point_value;

const MAX_AUTO_DENDRO_LEAVES: usize = 120;

struct DendrogramExtracted {
    leaf_labels: Vec<String>,
    merges: Vec<DendrogramMerge>,
    cut_height: Option<f64>,
    linkage_label: String,
    axis_subject_label: String,
}

fn is_clustering_result(res: &ResultSheet) -> bool {
    matches!(res.analysis.kind, AnalysisKind::Multivariate(MultivariateSpec::Clustering { .. }))
        || res.tabs.iter().any(|t| t.id == "hierarchical-dendrogram" || t.id == "hierarchical-dendrogram-columns")
}

fn linkage_display_name(linkage: ClusterLinkage) -> &'static str {
    match linkage {
        ClusterLinkage::Ward => "Ward",
        ClusterLinkage::Complete => "Complete",
        ClusterLinkage::Single => "Single",
        ClusterLinkage::Upgma => "UPGMA",
        ClusterLinkage::Wpgma => "WPGMA",
        ClusterLinkage::Centroid => "Centroid",
        ClusterLinkage::Median => "Median",
    }
}

fn extract_dendrogram_data(graph: &GraphSheet, doc: &ProjectDocument, warnings: &mut Vec<ChartWarning>) -> Option<DendrogramExtracted> {
    if let Some(res) = find_matching_result_where_checked(graph, doc, warnings, is_clustering_result) {
        let (direction, cut_height, linkage_label) = match &res.analysis.kind {
            AnalysisKind::Multivariate(MultivariateSpec::Clustering { cluster_direction, cut_height, linkage, .. }) => {
                (*cluster_direction, *cut_height, String::from(linkage_display_name(*linkage)))
            }
            _ => (ClusterDirection::Rows, None, String::from("Hierarchical")),
        };

        let use_columns = direction == ClusterDirection::Columns && res.tabs.iter().any(|t| t.id == "hierarchical-dendrogram-columns");
        let dendro_tab = if use_columns {
            res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram-columns")
        } else {
            res.tabs
                .iter()
                .find(|t| t.id == "hierarchical-dendrogram")
                .or_else(|| res.tabs.iter().find(|t| t.id == "hierarchical-dendrogram-columns"))
        }?;

        let n_leaves = dendro_tab.rows.len().saturating_add(1);
        if n_leaves < 2 {
            return None;
        }
        let merges = extract_dendro_merges_from_tab(dendro_tab, n_leaves);
        if merges.is_empty() {
            return None;
        }

        let mut leaf_labels: Vec<String> = if use_columns {
            res.tabs
                .iter()
                .find(|t| t.id == "hierarchical-height-cut-columns")
                .map(|t| t.row_labels.clone())
                .or_else(|| {
                    let src_ds = res.analysis.source_sheet_id.as_ref().and_then(|sid| doc.data_sheet(sid));
                    src_ds.and_then(|ds| match &ds.archetype {
                        TableArchetype::MultipleVariables(mv) => {
                            let cols: Vec<String> = if res.analysis.selected_data_sets.is_empty() {
                                mv.variables
                                    .iter()
                                    .filter(|v| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                                    .map(|v| v.title.clone())
                                    .collect()
                            } else {
                                res.analysis.selected_data_sets.iter().filter_map(|&c| mv.variables.get(c).map(|v| v.title.clone())).collect()
                            };
                            (cols.len() == n_leaves).then_some(cols)
                        }
                        _ => None,
                    })
                })
                .unwrap_or_else(|| (1..=n_leaves).map(|i| format!("Var {i}")).collect())
        } else {
            res.tabs
                .iter()
                .find(|t| t.id == "kmeans-assignments" || t.id == "hierarchical-height-cut" || t.id == "clustering-distance-matrix")
                .filter(|t| t.row_labels.len() == n_leaves)
                .map(|t| t.row_labels.clone())
                .unwrap_or_else(|| (1..=n_leaves).map(|i| format!("Obs {i}")).collect())
        };
        if leaf_labels.len() < n_leaves {
            for i in (leaf_labels.len() + 1)..=n_leaves {
                leaf_labels.push(format!("Leaf {i}"));
            }
        } else {
            leaf_labels.truncate(n_leaves);
        }

        return Some(DendrogramExtracted {
            leaf_labels,
            merges,
            cut_height,
            linkage_label,
            axis_subject_label: String::from(if use_columns { "Variables" } else { "Observations" }),
        });
    }

    if has_blocked_matching_result(graph, doc, is_clustering_result) {
        return None;
    }

    let ds = resolve_source_data_sheet_checked(graph, doc, warnings)?;
    let (mut rows, mut labels) = match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let num_cols: Vec<usize> = mv
                .variables
                .iter()
                .enumerate()
                .filter(|(_, v)| matches!(v.role, VariableRole::Continuous | VariableRole::Calculated))
                .map(|(i, _)| i)
                .collect();
            if num_cols.is_empty() {
                return None;
            }
            let label_col = mv.variables.iter().position(|v| v.role == VariableRole::Label);
            let mut r_vec = Vec::new();
            let mut l_vec = Vec::new();
            for r in 0..mv.row_count() {
                let vals: Option<Vec<f64>> =
                    num_cols.iter().map(|&c| mv.variables.get(c)?.cells.get(r)?.active_number().filter(|v| v.is_finite())).collect();
                if let Some(v) = vals {
                    let lbl = label_col
                        .and_then(|lc| mv.variables.get(lc)?.cells.get(r)?.active_text().map(str::to_owned))
                        .or_else(|| mv.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned())
                        .unwrap_or_else(|| format!("Obs {}", r + 1));
                    r_vec.push(v);
                    l_vec.push(lbl);
                }
            }
            (r_vec, l_vec)
        }
        TableArchetype::Grouped(g) => {
            let mut r_vec = Vec::new();
            let mut l_vec = Vec::new();
            for r in 0..g.row_count() {
                let vals: Option<Vec<f64>> =
                    g.data_sets.iter().map(|col| col.cells.get(r)?.first()?.active_number().filter(|v| v.is_finite())).collect();
                if let Some(v) = vals {
                    let lbl = g.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1));
                    r_vec.push(v);
                    l_vec.push(lbl);
                }
            }
            (r_vec, l_vec)
        }
        _ => return None,
    };

    if rows.len() < 2 {
        return None;
    }
    if rows.len() > MAX_AUTO_DENDRO_LEAVES {
        warnings.push(ChartWarning::new(
            "dendrogram_truncated",
            format!("standalone dendrogram capped to first {MAX_AUTO_DENDRO_LEAVES} of {} complete observations", rows.len()),
        ));
        rows.truncate(MAX_AUTO_DENDRO_LEAVES);
        labels.truncate(MAX_AUTO_DENDRO_LEAVES);
    }
    let merges = hierarchical_clustering_with_metrics(&rows, ClusterDistance::Euclidean, ClusterLinkage::Ward).ok()?;
    Some(DendrogramExtracted {
        leaf_labels: labels,
        merges,
        cut_height: None,
        linkage_label: String::from("Ward"),
        axis_subject_label: String::from("Observations"),
    })
}

/// Compile [`curvebook_doc::DiagnosticPlotKind::Dendrogram`] into a standalone vertical dendrogram
/// with merge-height *Y* axis, leaf-ordered *X* labels, and optional cut-height cluster coloring.
pub(in crate::multivariate_layout) fn compile_dendrogram_plot(
    graph: &GraphSheet,
    doc: &ProjectDocument,
    scene: &mut ChartScene,
) -> Result<(), ChartError> {
    let Some(extracted) = extract_dendrogram_data(graph, doc, &mut scene.warnings) else {
        scene.warnings.push(ChartWarning::new("empty_dendrogram", "no hierarchical clustering merge schedule available for dendrogram"));
        draw_empty_open_frame(scene);
        return Ok(());
    };
    let n_leaves = extracted.leaf_labels.len();
    if n_leaves < 2 || extracted.merges.is_empty() {
        scene.warnings.push(ChartWarning::new("empty_dendrogram", "dendrogram requires at least 2 observations and 1 merge step"));
        draw_empty_open_frame(scene);
        return Ok(());
    }

    let leaf_order = dendrogram_leaf_order(&extracted.merges, n_leaves);
    let max_merge_dist =
        extracted.merges.iter().filter_map(|m| (m.distance.is_finite() && m.distance >= 0.0).then_some(m.distance)).fold(0.0_f64, f64::max);
    let cut_h = extracted.cut_height.filter(|h| h.is_finite() && *h >= 0.0);
    let y_max = max_merge_dist.max(cut_h.unwrap_or(0.0)).max(1e-6) * 1.12;

    let mut frame_graph = graph.clone();
    if frame_graph.spec.x_axis.title.trim().is_empty() || frame_graph.spec.x_axis.title == "X" {
        frame_graph.spec.x_axis.title = extracted.axis_subject_label.clone();
    }
    if frame_graph.spec.y_axis.title.trim().is_empty() || frame_graph.spec.y_axis.title == "Y" {
        frame_graph.spec.y_axis.title = format!("Merge Distance ({})", extracted.linkage_label);
    }

    // Build leaf tick labels (thin automatically when dense so ticks stay legible).
    let tick_stride = if n_leaves <= 24 { 1 } else { n_leaves.div_ceil(20) };
    let cat_ticks: Vec<(f64, String)> = leaf_order
        .iter()
        .enumerate()
        .filter(|(vis_idx, _)| vis_idx % tick_stride == 0)
        .map(|(vis_idx, &orig_leaf)| {
            let raw = extracted.leaf_labels.get(orig_leaf).map_or("", String::as_str);
            let truncated: String =
                if raw.chars().count() > 14 { format!("{}...", raw.chars().take(12).collect::<String>()) } else { raw.to_owned() };
            (vis_idx as f64, truncated)
        })
        .collect();

    let (x_axis, y_axis, _) = layout_and_render_segmented_chrome(
        scene,
        &frame_graph,
        doc,
        &SegmentedFrameSpec {
            x_bounds: (-0.5, (n_leaves as f64) - 0.5),
            y_bounds: (0.0, y_max),
            y2_bounds: None,
            cat_x: Some(&cat_ticks),
            cat_y: None,
            margins: PlotMarginConfig::default(),
        },
    );
    let r = scene.plot_rect_pt;

    // Optional subtree cluster coloring below `cut_height`.
    let leaf_clusters: Option<Vec<usize>> = cut_h.and_then(|h| cut_dendrogram_at_height(n_leaves, &extracted.merges, h).ok());
    let mut leaf_x = vec![0.0_f64; n_leaves];
    for (vis_idx, &orig_leaf) in leaf_order.iter().enumerate() {
        if let Some(slot) = leaf_x.get_mut(orig_leaf) {
            *slot = vis_idx as f64;
        }
    }

    let mut merge_coords: Vec<(f64, f64)> = vec![(0.0, 0.0); extracted.merges.len()];
    let mut merge_cluster: Vec<Option<usize>> = vec![None; extracted.merges.len()];
    let default_color = Rgba8::rgb(55, 62, 74);

    let lookup_node = |id: isize, mc: &[(f64, f64)], mcl: &[Option<usize>]| -> (f64, f64, Option<usize>) {
        if id < 0 {
            let leaf = decode_dendro_leaf(id, n_leaves).unwrap_or(0);
            let x = leaf_x.get(leaf).copied().unwrap_or(0.0);
            let cl = leaf_clusters.as_ref().and_then(|c| c.get(leaf).copied());
            (x, 0.0, cl)
        } else {
            let idx = usize::try_from(id).ok().filter(|&i| i < mc.len());
            match idx {
                Some(i) => (mc[i].0, mc[i].1, mcl[i]),
                None => (0.0, 0.0, None),
            }
        }
    };

    let sheet_id = graph.spec.primary_source_sheet_id.clone();
    for (step, m) in extracted.merges.iter().enumerate() {
        let (x_l, h_l, cl_l) = lookup_node(m.left, &merge_coords, &merge_cluster);
        let (x_r, h_r, cl_r) = lookup_node(m.right, &merge_coords, &merge_cluster);
        let h_m = if m.distance.is_finite() && m.distance >= 0.0 { m.distance } else { h_l.max(h_r) };
        let same_cl = match (cut_h, cl_l, cl_r) {
            (Some(ch), Some(a), Some(b)) if h_m <= ch && a == b => Some(a),
            _ => None,
        };
        let color = same_cl.map_or(default_color, |c| graph.spec.palette.color_at(c));
        let stroke = solid_stroke(color, 1.35, LineDashStyle::Solid);

        let (_, px_l, _) = x_axis.map_clamped_to_nearest(x_l);
        let (_, px_r, _) = x_axis.map_clamped_to_nearest(x_r);
        let (_, py_l, _) = y_axis.map_clamped_to_nearest(h_l);
        let (_, py_r, _) = y_axis.map_clamped_to_nearest(h_r);
        let (_, py_m, _) = y_axis.map_clamped_to_nearest(h_m);

        let mut path = Path2D::new();
        path_move(&mut path, PointPt::new(px_l, py_l));
        path_line(&mut path, PointPt::new(px_l, py_m));
        path_line(&mut path, PointPt::new(px_r, py_m));
        path_line(&mut path, PointPt::new(px_r, py_r));
        scene.primitives.push(ScenePrimitive::Path { path, fill: None, stroke: Some(stroke), clip: Some(r) });

        let x_mid = 0.5 * (x_l + x_r);
        if let Some(slot) = merge_coords.get_mut(step) {
            *slot = (x_mid, h_m);
        }
        if let Some(slot) = merge_cluster.get_mut(step) {
            *slot = same_cl;
        }
        let (_, px_mid, _) = x_axis.map_clamped_to_nearest(x_mid);
        scene.plotted_points.push(PlottedPointMeta {
            center_pt: PointPt::new(px_mid, py_m),
            radius_pt: 3.5,
            source_cell: SourceCellId { sheet_id: sheet_id.clone(), row: step, col: 2, subcol: 0 },
            series_label: format!("Merge {}", step + 1),
            value_summary: format!("distance={}, size={}", format_point_value(h_m), m.cluster_size),
        });
    }

    // Optional cut-height dashed horizontal reference line.
    if let Some(ch) = cut_h
        && let Some((_, py_cut)) = y_axis.map_value(ch)
    {
        let accent = graph.spec.palette.color_at(1);
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(r.x, py_cut), PointPt::new(r.right(), py_cut)),
            fill: None,
            stroke: Some(solid_stroke(accent, 1.1, LineDashStyle::Dashed)),
            clip: Some(r),
        });
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: RichText::plain(format!("Cut h = {}", format_point_value(ch))),
            anchor_pt: PointPt::new(r.right() - 4.0, (py_cut - 4.0).max(r.y + 10.0)),
            anchor: TextAnchor::End,
            font_size_pt: 8.0,
            rotation_deg: 0.0,
            color: accent,
        }));
    }

    Ok(())
}
