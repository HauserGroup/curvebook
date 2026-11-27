//! `ResultSheet` builders for multivariate analyses (correlation matrices, PCA / Principal
//! Component Regression, *k*-means++ and hierarchical clustering, and MICE multiple imputation).
//!
//! References:
//! - Pearson, K. (1896). Mathematical contributions to the theory of evolution. III. Regression,
//!   heredity, and panmixia. *Philosophical Transactions of the Royal Society A*, 187, 253-318.
//!   `doi:10.1098/rsta.1896.0007`.
//! - Spearman, C. (1904). The proof and measurement of association between two things. *American
//!   Journal of Psychology*, 15(1), 72-101. `doi:10.2307/1412159`.
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). New York: Springer.
//! - Arthur, D., & Vassilvitskii, S. (2007). k-means++: The advantages of careful seeding.
//!   *Proceedings of the 18th Annual ACM-SIAM Symposium on Discrete Algorithms (SODA)*, 1027-1035.
//! - Ward, J. H. (1963). Hierarchical grouping to optimize an objective function. *Journal of the
//!   American Statistical Association*, 58(301), 236-244. `doi:10.1080/01621459.1963.10500845`.
//! - Murtagh, F., & Legendre, P. (2014). Ward's hierarchical agglomerative clustering method: Which
//!   algorithms implement Ward's criterion? *Journal of Classification*, 31(3), 274-295.
//!   `doi:10.1007/s00357-014-9161-z`.
//! - Rubin, D. B. (1987). *Multiple Imputation for Nonresponse in Surveys*. Wiley.
//! - Barnard, J., & Rubin, D. B. (1999). Small-sample degrees of freedom with multiple imputation.
//!   *Biometrika*, 86(4), 948-955.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistStatus, ClusterDistance, ClusterLinkage, DataSheet, MultivariateSpec, ResultCell, ResultSheet,
    ResultTabKind, SubcolumnMode, TableArchetype, VariableRole, XColumnMode,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::student_t_two_sided_p;
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::SummaryMoments;

use super::clustering::{
    MAX_CLUSTER_K_SWEEP, MAX_CLUSTER_MATRIX_TAB_N, MAX_CLUSTERING_N, compute_mean_silhouette_from_dist, cophenetic_correlation,
    cophenetic_distance_matrix, count_distinct_by_dist, count_distinct_rows, cut_dendrogram, hierarchical_clustering_from_dist, kmeans_pp_from_dist,
    pairwise_distance_matrix, pairwise_distance_unchecked,
};
use super::correlation::correlation_matrix;
use super::mice::mice_predictive_mean_matching;
use super::pca::pca_analysis;
use super::types::{KMeansResult, MultipleImputationConfig, MultipleImputationResult, PcaResult};
use crate::common::{
    build_tab, checklist_item, ci_cell, enforce_summary_mode_matrix, finite_ci_cell, init_result_sheet, num_cell, opt_num_cell, p_cell_at_alpha,
    push_scalar, resolve_selected_indices, sanitize_result_sheet_floats, t_crit_two_sided,
};
use crate::error::StatsError;
use crate::regression::multiple_linear_regression;

/// Execute a [`MultivariateSpec`] (`Pca`, `CorrelationMatrix`, or `Clustering`) on a `DataSheet`.
pub fn run_multivariate_analysis(sheet: &DataSheet, spec: &AnalysisSpec, mv_spec: &MultivariateSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::Multivariate(mv_spec.clone()))?;
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;

    match mv_spec {
        MultivariateSpec::CorrelationMatrix { spearman_rank } => {
            let (labels, opt_cols) = extract_optional_columns(sheet, &spec.selected_data_sets)?;
            let refs: Vec<(&str, &[Option<f64>])> = labels.iter().zip(opt_cols.iter()).map(|(l, v)| (l.as_str(), v.as_slice())).collect();
            let res = correlation_matrix(&refs, *spearman_rank)?;

            let mut tab_r = build_tab(
                "corr-matrix-r",
                if *spearman_rank { "Spearman correlation matrix (rho)" } else { "Pearson correlation matrix (r)" },
                ResultTabKind::MainSummary,
                res.labels.clone(),
                res.labels.clone(),
                res.r_matrix.iter().map(|row| row.iter().map(|&v| num_cell(v)).collect()).collect(),
            );
            let mut tab_p = build_tab(
                "corr-matrix-p",
                "Correlation P values",
                ResultTabKind::ParametersAndConfidenceIntervals,
                res.labels.clone(),
                res.labels.clone(),
                res.p_matrix.iter().map(|row| row.iter().map(|&v| p_cell_at_alpha(v, p_fmt, spec.confidence_level.alpha())).collect()).collect(),
            );
            let mut tab_q = build_tab(
                "corr-matrix-fdr",
                "BH FDR-adjusted q values",
                ResultTabKind::MultipleComparisons,
                res.labels.clone(),
                res.labels.clone(),
                res.fdr_q_matrix.iter().map(|row| row.iter().map(|&v| p_cell_at_alpha(v, p_fmt, spec.confidence_level.alpha())).collect()).collect(),
            );
            let p_vars = res.labels.len();
            let mut undefined_pairs = 0_usize;
            for i in 0..p_vars {
                for j in (i + 1)..p_vars {
                    if !res.r_matrix[i][j].is_finite() {
                        undefined_pairs += 1;
                    }
                }
            }
            if undefined_pairs > 0 {
                let msg = format!(
                    "{undefined_pairs} variable pair(s) had undefined correlation (constant column or < 3 complete pairs), omitted from display and excluded from Benjamini-Hochberg FDR adjustment."
                );
                tab_r.footnotes.push(msg.clone());
                tab_p.footnotes.push(msg.clone());
                tab_q.footnotes.push(msg);
            }
            out.tabs.extend([tab_r, tab_p, tab_q]);

            let min_n = res.n_matrix.iter().flat_map(|r| r.iter()).copied().min().unwrap_or(0);
            out.checklist.push(checklist_item(
                "Pairwise sample sizes & multiplicity",
                if min_n >= 10 && undefined_pairs == 0 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
                Some(format!("min pairwise n = {min_n}, {} variables, {undefined_pairs} undefined pair(s)", res.labels.len())),
                if undefined_pairs > 0 {
                    "Defined upper-triangle P values are adjusted via Benjamini-Hochberg; undefined pairs (< 3 complete pairs or zero variance) were excluded from the FDR family."
                } else {
                    "Upper-triangle P values are adjusted for false discovery rate via Benjamini-Hochberg."
                },
            ));
        }
        MultivariateSpec::Pca { standardize, parallel_analysis_seed, pcr_outcome_column } => {
            if let Some(outcome_col) = *pcr_outcome_column {
                return run_pcr_branch(sheet, spec, *standardize, *parallel_analysis_seed, outcome_col);
            }
            let (labels, rows) = extract_complete_matrix_rows(sheet, &spec.selected_data_sets)?;
            let n = rows.len();
            let p = labels.len();
            let flat: Vec<f64> = rows.into_iter().flat_map(|r| r.into_iter()).collect();
            let mat = DenseMatrix::from_row_major(n, p, flat)?;
            let pca = pca_analysis(&mat, &labels, *standardize, *parallel_analysis_seed)?;
            append_pca_outputs(&mut out, &pca, labels);
        }
        MultivariateSpec::Clustering { k_means_k, distance, linkage, k_sweep_max } => {
            if let Some(k_max_req) = k_sweep_max
                && *k_max_req < 2
            {
                return Err(StatsError::InvalidParameter { name: "k_sweep_max", value: *k_max_req as f64 });
            }
            let (var_labels, obs_labels, rows) = extract_complete_matrix_rows_with_names(sheet, &spec.selected_data_sets)?;
            let n = rows.len();
            if n > MAX_CLUSTERING_N {
                return Err(StatsError::InvalidParameter { name: "clustering_n", value: n as f64 });
            }
            let p = var_labels.len();
            let dist_metric = *distance;
            let link_rule = *linkage;

            let dist_mat = pairwise_distance_matrix(&rows, dist_metric)?;
            let merges = hierarchical_clustering_from_dist(&dist_mat, link_rule)?;
            let coph_mat = cophenetic_distance_matrix(n, &merges)?;
            let cophenetic_r = cophenetic_correlation(&dist_mat, &coph_mat);

            let n_distinct = count_distinct_rows(&rows).min(count_distinct_by_dist(&dist_mat));
            let k_max = match k_sweep_max {
                Some(req) => (*req).min(n_distinct).min(MAX_CLUSTER_K_SWEEP),
                None => 6.min(n.saturating_sub(1)).min(n_distinct).max(2.min(n_distinct)),
            };
            let mut sweep_results: Vec<KMeansResult> = Vec::new();
            for k_try in 2..=k_max {
                if let Ok(res) = kmeans_pp_from_dist(&rows, k_try, dist_metric, &dist_mat, 0xC105_7E57) {
                    sweep_results.push(res);
                }
            }

            let km = if let Some(k) = k_means_k {
                if let Some(found) = sweep_results.iter().find(|r| r.k == *k) {
                    found.clone()
                } else {
                    kmeans_pp_from_dist(&rows, *k, dist_metric, &dist_mat, 0xC105_7E57)?
                }
            } else if let Some(best) = sweep_results
                .iter()
                .filter(|r| n < 3 || r.k < n)
                .max_by(|a, b| {
                    if (a.mean_silhouette - b.mean_silhouette).abs() <= 1.0e-12 {
                        b.k.cmp(&a.k)
                    } else {
                        a.mean_silhouette.total_cmp(&b.mean_silhouette)
                    }
                })
                .or_else(|| sweep_results.first())
            {
                best.clone()
            } else {
                kmeans_pp_from_dist(&rows, n_distinct.clamp(1, 2), dist_metric, &dist_mat, 0xC105_7E57)?
            };

            push_scalar(&mut out, "Clusters (k)", None, km.k as f64);
            push_scalar(&mut out, "Total WCSS", None, km.wcss);
            push_scalar(&mut out, "Mean silhouette", None, km.mean_silhouette);
            if let Some(r_coph) = cophenetic_r {
                push_scalar(&mut out, "cophenetic_r", None, r_coph);
            }

            let dist_label = cluster_distance_label(dist_metric);
            let link_label = cluster_linkage_label(link_rule);

            if k_means_k.is_some() || k_sweep_max.is_some() || !sweep_results.is_empty() {
                let mut c_headers = vec![String::from("Cluster"), String::from("Size")];
                c_headers.extend(var_labels.iter().map(|l| format!("Centroid ({l})")));
                let mut c_labels = Vec::with_capacity(km.k);
                let mut c_rows = Vec::with_capacity(km.k);
                for (c_idx, cent) in km.centroids.iter().enumerate() {
                    c_labels.push(format!("Cluster {}", c_idx + 1));
                    let cnt = km.assignments.iter().filter(|&&a| a == c_idx).count();
                    let mut r = vec![num_cell((c_idx + 1) as f64), num_cell(cnt as f64)];
                    r.extend(cent.iter().map(|&v| num_cell(v)));
                    c_rows.push(r);
                }
                out.tabs.push(build_tab(
                    "kmeans-clusters",
                    format!("k-means++ cluster centroids ({dist_label})"),
                    ResultTabKind::MainSummary,
                    c_headers,
                    c_labels,
                    c_rows,
                ));

                let mut assign_rows = Vec::with_capacity(n);
                for (i, row_vec) in rows.iter().enumerate() {
                    let c_idx = km.assignments[i];
                    let dist_c = pairwise_distance_unchecked(row_vec, &km.centroids[c_idx], dist_metric);
                    assign_rows.push(vec![num_cell((c_idx + 1) as f64), num_cell(dist_c)]);
                }
                out.tabs.push(build_tab(
                    "kmeans-assignments",
                    "k-means++ observation cluster assignments",
                    ResultTabKind::ParametersAndConfidenceIntervals,
                    vec![String::from("Assigned cluster"), format!("Distance to centroid ({dist_label})")],
                    obs_labels.clone(),
                    assign_rows,
                ));

                if !sweep_results.is_empty() {
                    let s_labels: Vec<String> = sweep_results.iter().map(|r| format!("k = {}", r.k)).collect();
                    let s_rows: Vec<Vec<ResultCell>> =
                        sweep_results.iter().map(|r| vec![num_cell(r.k as f64), num_cell(r.wcss), num_cell(r.mean_silhouette)]).collect();
                    out.tabs.push(build_tab(
                        "kmeans-sweep-curve",
                        "Candidate-k elbow (WCSS) & silhouette curve",
                        ResultTabKind::ModelComparison,
                        vec![String::from("Clusters (k)"), String::from("WCSS"), String::from("Mean silhouette")],
                        s_labels,
                        s_rows,
                    ));
                }
            }

            let d_headers = vec![String::from("Left"), String::from("Right"), format!("{link_label} distance"), String::from("Cluster size")];
            let d_labels: Vec<String> = (1..=merges.len()).map(|s| format!("Merge {s}")).collect();
            let d_rows: Vec<Vec<ResultCell>> = merges
                .iter()
                .map(|m| vec![num_cell(m.left as f64), num_cell(m.right as f64), num_cell(m.distance), num_cell(m.cluster_size as f64)])
                .collect();
            out.tabs.push(build_tab(
                "hierarchical-dendrogram",
                format!("{link_label} dendrogram linkage ({dist_label})"),
                ResultTabKind::ModelComparison,
                d_headers,
                d_labels,
                d_rows,
            ));

            let max_hier_k = n.saturating_sub(1).min(8);
            if max_hier_k >= 2 {
                let mut h_labels = Vec::with_capacity(max_hier_k - 1);
                let mut h_rows = Vec::with_capacity(max_hier_k - 1);
                for k_cut in 2..=max_hier_k {
                    let assigns = cut_dendrogram(n, &merges, k_cut)?;
                    let sil = compute_mean_silhouette_from_dist(&dist_mat, &assigns, k_cut);
                    let mut counts = vec![0_usize; k_cut];
                    for &c in &assigns {
                        counts[c] += 1;
                    }
                    let min_sz = counts.iter().copied().min().unwrap_or(0);
                    let max_sz = counts.iter().copied().max().unwrap_or(0);
                    let cut_height = merges.get(n - k_cut).map_or(0.0, |m| m.distance);
                    h_labels.push(format!("k = {k_cut}"));
                    h_rows.push(vec![num_cell(k_cut as f64), num_cell(cut_height), num_cell(sil), num_cell(min_sz as f64), num_cell(max_sz as f64)]);
                }
                out.tabs.push(build_tab(
                    "hierarchical-cut-silhouettes",
                    format!("Hierarchical cut silhouette widths ({link_label}, {dist_label})"),
                    ResultTabKind::Diagnostics,
                    vec![
                        String::from("Clusters (k)"),
                        String::from("Cut height"),
                        String::from("Mean silhouette"),
                        String::from("Min cluster size"),
                        String::from("Max cluster size"),
                    ],
                    h_labels,
                    h_rows,
                ));
            }

            if n <= MAX_CLUSTER_MATRIX_TAB_N {
                let dist_rows: Vec<Vec<ResultCell>> = dist_mat.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
                out.tabs.push(build_tab(
                    "clustering-distance-matrix",
                    format!("Pairwise dissimilarity matrix ({dist_label})"),
                    ResultTabKind::CovarianceMatrix,
                    obs_labels.clone(),
                    obs_labels.clone(),
                    dist_rows,
                ));

                let coph_rows: Vec<Vec<ResultCell>> = coph_mat.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
                out.tabs.push(build_tab(
                    "cophenetic-distance-matrix",
                    format!("Cophenetic distance matrix ({link_label})"),
                    ResultTabKind::CovarianceMatrix,
                    obs_labels.clone(),
                    obs_labels,
                    coph_rows,
                ));
            } else {
                out.checklist.push(checklist_item(
                    "Dense pairwise & cophenetic matrices omitted for large n",
                    ChecklistStatus::Info,
                    Some(format!("n = {n} exceeds dense table cap ({MAX_CLUSTER_MATRIX_TAB_N})")),
                    "Cophenetic correlation (r), dendrogram linkage schedule, and cluster assignments are still reported.",
                ));
            }

            out.tabs.push(build_tab(
                "clustering-diagnostics",
                "Clustering configuration & cophenetic diagnostics",
                ResultTabKind::Diagnostics,
                vec![String::from("Value")],
                vec![
                    String::from("Cophenetic correlation (r)"),
                    String::from("Distance metric"),
                    String::from("Linkage rule"),
                    String::from("Complete observations (n)"),
                    String::from("Variables (p)"),
                ],
                vec![
                    vec![opt_num_cell(cophenetic_r)],
                    vec![ResultCell::Text(dist_label.to_owned())],
                    vec![ResultCell::Text(link_label.to_owned())],
                    vec![num_cell(n as f64)],
                    vec![num_cell(p as f64)],
                ],
            ));

            out.checklist.push(checklist_item(
                "Cluster separation quality & cophenetic fit",
                if km.mean_silhouette >= 0.25 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
                Some(format!(
                    "k = {}, silhouette = {:.3}, WCSS = {:.2}, cophenetic r = {}",
                    km.k,
                    km.mean_silhouette,
                    km.wcss,
                    cophenetic_r.map_or_else(|| String::from("n/a"), |r| format!("{r:.3}"))
                )),
                format!(
                    "Mean silhouette near +1 indicates tight, well-separated clusters under {dist_label} dissimilarity; {link_label} dendrogram cophenetic r measures ultrametric tree fidelity."
                ),
            ));
        }
        MultivariateSpec::MultipleImputation { imputations_m, max_iters, seed, response_column, predictor_columns } => {
            let extracted = extract_mi_columns(sheet, &spec.selected_data_sets, *response_column, predictor_columns)?;
            let config = MultipleImputationConfig {
                imputations_m: *imputations_m,
                max_iters: *max_iters,
                seed: *seed,
                response_col: extracted.model_response_col,
                predictor_cols: extracted.model_predictor_cols,
                confidence: spec.confidence_level,
            };
            let res = mice_predictive_mean_matching(&extracted.labels, &extracted.cols, &config)?;
            populate_mi_result_sheet(&mut out, &res, p_fmt, spec.confidence_level);
        }
    }

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

struct ExtractedMiData {
    labels: Vec<String>,
    cols: Vec<Vec<Option<f64>>>,
    model_response_col: Option<usize>,
    model_predictor_cols: Vec<usize>,
}

const fn xy_x_is_pre_summarized(xy: &curvebook_doc::XyTable) -> bool {
    matches!(&xy.x_mode, XColumnMode::Numbers { subcolumn_mode } if subcolumn_mode.is_pre_summarized())
}

const fn xy_x_raw_subcols(xy: &curvebook_doc::XyTable) -> usize {
    match &xy.x_mode {
        XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(k) } if *k > 1 => *k,
        _ => 1,
    }
}

fn xy_x_active_at(xy: &curvebook_doc::XyTable, row: usize, sub: usize, x_subs: usize) -> Option<f64> {
    if x_subs > 1 { xy.x_cells.get(row).and_then(|subs| subs.get(sub)).and_then(curvebook_doc::Cell::active_number) } else { xy.x_value_at(row) }
}

struct RawOptionalColumns {
    labels: Vec<String>,
    cols: Vec<Vec<Option<f64>>>,
    is_label: Vec<bool>,
}

fn extract_raw_optional_columns(sheet: &DataSheet) -> Result<RawOptionalColumns, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let mut labels = Vec::with_capacity(mv.variables.len());
            let mut cols = Vec::with_capacity(mv.variables.len());
            let mut is_label = Vec::with_capacity(mv.variables.len());
            for v in &mv.variables {
                labels.push(v.title.clone());
                cols.push(v.cells.iter().map(|c| c.active_number()).collect());
                is_label.push(matches!(v.role, VariableRole::Label));
            }
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if matches!(col_tab.subcolumn_mode, SubcolumnMode::RawReplicates(k) if k > 1) {
                return Err(StatsError::IncompatibleTable { analysis: "multiple imputation", actual: curvebook_doc::TableKind::Column });
            }
            let labels = col_tab.data_sets.iter().map(|ds| ds.title.clone()).collect();
            let cols =
                col_tab.data_sets.iter().map(|ds| ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect()).collect();
            let is_label = vec![false; col_tab.data_sets.len()];
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let x_subs = xy_x_raw_subcols(xy);
            let y_subs = xy.y_subcolumn_mode.subcolumn_count().max(1);
            let n_subs = x_subs.max(y_subs);
            let mut labels = Vec::with_capacity(1 + xy.data_sets.len());
            labels.push(xy.x_title.clone());
            for ds in &xy.data_sets {
                labels.push(ds.title.clone());
            }
            let mut cols = vec![Vec::new(); 1 + xy.data_sets.len()];
            for r in 0..xy.row_count() {
                for sub in 0..n_subs {
                    if n_subs > 1 && sub > 0 {
                        let any_y_entry = xy.data_sets.iter().any(|ds| ds.cell(r, sub).is_some_and(|c| !c.is_empty() || c.excluded));
                        let any_x_entry = x_subs > 1 && xy.x_cells.get(r).and_then(|s| s.get(sub)).is_some_and(|c| !c.is_empty() || c.excluded);
                        if !any_y_entry && !any_x_entry {
                            continue;
                        }
                    }
                    cols[0].push(xy_x_active_at(xy, r, sub, x_subs));
                    for (d_idx, ds) in xy.data_sets.iter().enumerate() {
                        cols[1 + d_idx].push(ds.cell(r, sub).and_then(curvebook_doc::Cell::active_number));
                    }
                }
            }
            let is_label = vec![false; labels.len()];
            Ok(RawOptionalColumns { labels, cols, is_label })
        }
        other => Err(StatsError::IncompatibleTable { analysis: "multiple imputation", actual: other.kind() }),
    }
}

fn push_unique(target: &mut Vec<usize>, val: usize) {
    if !target.contains(&val) {
        target.push(val);
    }
}

fn collect_mi_column_indices(
    raw_cols: &[Vec<Option<f64>>],
    is_label: &[bool],
    selected: &[usize],
    response_column: Option<usize>,
    predictor_columns: &[usize],
) -> Vec<usize> {
    let total_cols = raw_cols.len();
    let is_numeric_usable = |idx: usize| !is_label[idx] && raw_cols[idx].iter().any(|c| c.is_some_and(f64::is_finite));
    let mut chosen = Vec::new();

    if response_column.is_some() || !predictor_columns.is_empty() {
        if let Some(resp) = response_column {
            push_unique(&mut chosen, resp);
        }
        for &pred in predictor_columns {
            push_unique(&mut chosen, pred);
        }
        let extra = if !selected.is_empty() {
            resolve_selected_indices(selected, total_cols)
        } else if response_column.is_some() && predictor_columns.is_empty() {
            (0..total_cols).collect()
        } else {
            Vec::new()
        };
        for idx in extra {
            if is_numeric_usable(idx) {
                push_unique(&mut chosen, idx);
            }
        }
        return chosen;
    }

    if !selected.is_empty() {
        for idx in resolve_selected_indices(selected, total_cols) {
            push_unique(&mut chosen, idx);
        }
        return chosen;
    }

    for idx in 0..total_cols {
        if is_numeric_usable(idx) {
            chosen.push(idx);
        }
    }
    chosen
}

fn extract_mi_columns(
    sheet: &DataSheet,
    selected: &[usize],
    response_column: Option<usize>,
    predictor_columns: &[usize],
) -> Result<ExtractedMiData, StatsError> {
    let raw = extract_raw_optional_columns(sheet)?;
    let (raw_labels, raw_cols, is_label) = (raw.labels, raw.cols, raw.is_label);
    let total_cols = raw_cols.len();
    if total_cols == 0 {
        return Err(StatsError::EmptyInput);
    }
    validate_selected_data_sets(selected, total_cols)?;

    if let Some(resp) = response_column
        && resp >= total_cols
    {
        return Err(StatsError::MissingColumn { index: resp });
    }
    for &pred in predictor_columns {
        if pred >= total_cols {
            return Err(StatsError::MissingColumn { index: pred });
        }
    }

    let chosen_orig = collect_mi_column_indices(&raw_cols, &is_label, selected, response_column, predictor_columns);
    if chosen_orig.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    if let TableArchetype::Xy(xy) = &sheet.archetype
        && chosen_orig.contains(&0)
        && xy_x_is_pre_summarized(xy)
    {
        return Err(StatsError::RequiresRawReplicates);
    }

    let max_len = chosen_orig.iter().map(|&idx| raw_cols[idx].len()).max().unwrap_or(0);
    let mut padded_cols: Vec<Vec<Option<f64>>> = chosen_orig
        .iter()
        .map(|&idx| {
            let mut v: Vec<Option<f64>> = raw_cols[idx].iter().map(|&c| c.filter(|x| x.is_finite())).collect();
            v.resize(max_len, None);
            v
        })
        .collect();

    // Trim trailing completely blank spreadsheet rows without dropping internal missingness.
    let mut effective_rows = max_len;
    while effective_rows > 0 && padded_cols.iter().all(|c| c[effective_rows - 1].is_none()) {
        effective_rows -= 1;
    }
    for col in &mut padded_cols {
        col.truncate(effective_rows);
    }

    let labels: Vec<String> = chosen_orig.iter().map(|&idx| raw_labels[idx].clone()).collect();
    let model_response_col = response_column.and_then(|r| chosen_orig.iter().position(|&orig| orig == r));
    let model_predictor_cols = if let Some(resp_pos) = model_response_col {
        if predictor_columns.is_empty() {
            (0..chosen_orig.len()).filter(|&pos| pos != resp_pos).collect()
        } else {
            predictor_columns
                .iter()
                .filter_map(|&p_orig| chosen_orig.iter().position(|&orig| orig == p_orig))
                .filter(|&pos| pos != resp_pos)
                .collect()
        }
    } else {
        Vec::new()
    };

    Ok(ExtractedMiData { labels, cols: padded_cols, model_response_col, model_predictor_cols })
}

fn populate_mi_result_sheet(out: &mut ResultSheet, res: &MultipleImputationResult, p_fmt: curvebook_doc::PValueFormat, confidence: ConfidenceLevel) {
    push_scalar(out, "Imputations (m)", None, res.m as f64);
    push_scalar(out, "Max iterations", None, res.max_iters as f64);
    push_scalar(out, "Total missing cells", None, res.total_missing as f64);
    for pm in &res.pooled_means {
        push_scalar(out, format!("{}: Pooled mean", pm.term), None, pm.estimate);
        push_scalar(out, format!("{}: Pooled SE", pm.term), None, pm.pooled_se);
        push_scalar(out, format!("{}: FMI", pm.term), None, pm.fmi);
    }

    let ci_header = format!("{}% CI", confidence.percent());
    let mean_headers = vec![
        String::from("Pooled mean"),
        String::from("Within var (U)"),
        String::from("Between var (B)"),
        String::from("Total var (T)"),
        String::from("Pooled SE"),
        ci_header.clone(),
        String::from("df (Barnard-Rubin)"),
        String::from("RIV"),
        String::from("FMI"),
        String::from("Missing count"),
    ];
    let mut mean_rows = Vec::with_capacity(res.pooled_means.len());
    for (pm, &mis_cnt) in res.pooled_means.iter().zip(res.missing_counts.iter()) {
        mean_rows.push(vec![
            num_cell(pm.estimate),
            num_cell(pm.within_variance),
            num_cell(pm.between_variance),
            num_cell(pm.total_variance),
            num_cell(pm.pooled_se),
            finite_ci_cell(pm.ci.0, pm.ci.1),
            num_cell(pm.df),
            num_cell(pm.riv),
            num_cell(pm.fmi),
            num_cell(mis_cnt as f64),
        ]);
    }
    out.tabs.push(build_tab(
        "mi-pooled-means",
        "Pooled descriptive estimates (Rubin 1987)",
        ResultTabKind::MainSummary,
        mean_headers,
        res.variable_labels.clone(),
        mean_rows,
    ));

    if let Some(reg_terms) = &res.pooled_regression {
        let reg_headers = vec![
            String::from("Pooled estimate"),
            String::from("Pooled SE"),
            ci_header,
            String::from("t"),
            String::from("df"),
            String::from("P value"),
            String::from("Within var (U)"),
            String::from("Between var (B)"),
            String::from("Total var (T)"),
            String::from("RIV"),
            String::from("FMI"),
        ];
        let mut reg_labels = Vec::with_capacity(reg_terms.len());
        let mut reg_rows = Vec::with_capacity(reg_terms.len());
        let mut aliased_terms = Vec::new();
        for term in reg_terms {
            push_scalar(out, format!("Reg {}: Pooled estimate", term.term), None, term.estimate);
            push_scalar(out, format!("Reg {}: Pooled SE", term.term), None, term.pooled_se);
            push_scalar(out, format!("Reg {}: P value", term.term), None, term.p_value);
            push_scalar(out, format!("Reg {}: FMI", term.term), None, term.fmi);
            reg_labels.push(term.term.clone());
            if !term.estimate.is_finite() {
                aliased_terms.push(term.term.clone());
                reg_rows.push(vec![
                    ResultCell::Text(String::from("Aliased")),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    num_cell(term.df),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ]);
            } else {
                reg_rows.push(vec![
                    num_cell(term.estimate),
                    num_cell(term.pooled_se),
                    finite_ci_cell(term.ci.0, term.ci.1),
                    num_cell(term.t_statistic),
                    num_cell(term.df),
                    p_cell_at_alpha(term.p_value, p_fmt, confidence.alpha()),
                    num_cell(term.within_variance),
                    num_cell(term.between_variance),
                    num_cell(term.total_variance),
                    num_cell(term.riv),
                    num_cell(term.fmi),
                ]);
            }
        }
        let mut reg_tab = build_tab(
            "mi-pooled-regression",
            "Pooled regression parameters (Rubin 1987)",
            ResultTabKind::ParametersAndConfidenceIntervals,
            reg_headers,
            reg_labels,
            reg_rows,
        );
        if !aliased_terms.is_empty() {
            reg_tab.footnotes.push(format!("Aliased predictor(s) across imputed regression fits marked \"Aliased\": {}.", aliased_terms.join(", ")));
        }
        out.tabs.push(reg_tab);
    }

    let diag_headers = vec![
        String::from("Observed n"),
        String::from("Missing count"),
        String::from("Missing %"),
        String::from("Imputations (m)"),
        String::from("FMI"),
    ];
    let diag_rows: Vec<Vec<ResultCell>> = res
        .missing_counts
        .iter()
        .zip(res.pooled_means.iter())
        .map(|(&mis, pm)| {
            let obs = res.n_rows.saturating_sub(mis);
            let pct = if res.n_rows > 0 { 100.0 * (mis as f64) / (res.n_rows as f64) } else { 0.0 };
            vec![num_cell(obs as f64), num_cell(mis as f64), num_cell(pct), num_cell(res.m as f64), num_cell(pm.fmi)]
        })
        .collect();
    out.tabs.push(build_tab(
        "mi-missingness",
        "Missingness & imputation diagnostics",
        ResultTabKind::Diagnostics,
        diag_headers,
        res.variable_labels.clone(),
        diag_rows,
    ));

    let max_fmi = res
        .pooled_means
        .iter()
        .map(|e| e.fmi)
        .chain(res.pooled_regression.iter().flatten().map(|e| e.fmi))
        .filter(|v| v.is_finite())
        .fold(0.0_f64, f64::max);
    let adequacy_status = if res.total_missing == 0 {
        ChecklistStatus::Info
    } else if (res.m as f64) + 1.0e-9 >= (100.0 * max_fmi).ceil().min(50.0) || max_fmi <= 0.25 {
        ChecklistStatus::Pass
    } else {
        ChecklistStatus::Warn
    };
    out.checklist.push(checklist_item(
        "Multiple imputation pooling & FMI",
        adequacy_status,
        Some(format!("m = {}, {} missing cell(s), max FMI = {:.3}", res.m, res.total_missing, max_fmi)),
        "Chained equations use Type-1 predictive mean matching with posterior parameter draws and Barnard–Rubin small-sample df; higher FMI indicates greater between-imputation missingness uncertainty.",
    ));
}

#[inline]
const fn cluster_distance_label(distance: ClusterDistance) -> &'static str {
    match distance {
        ClusterDistance::Euclidean => "Euclidean",
        ClusterDistance::Manhattan => "Manhattan",
        ClusterDistance::Pearson => "Pearson (1 - r)",
        ClusterDistance::Cosine => "Cosine (1 - cos)",
    }
}

#[inline]
const fn cluster_linkage_label(linkage: ClusterLinkage) -> &'static str {
    match linkage {
        ClusterLinkage::Ward => "Ward.D2",
        ClusterLinkage::Complete => "Complete",
        ClusterLinkage::Single => "Single",
        ClusterLinkage::Upgma => "UPGMA",
        ClusterLinkage::Wpgma => "WPGMA",
        ClusterLinkage::Centroid => "Centroid",
        ClusterLinkage::Median => "Median",
    }
}

fn validate_selected_data_sets(selected: &[usize], total: usize) -> Result<(), StatsError> {
    let mut seen = Vec::with_capacity(selected.len());
    for &idx in selected {
        if idx >= total {
            return Err(StatsError::MissingColumn { index: idx });
        }
        if seen.contains(&idx) {
            return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: idx as f64 });
        }
        seen.push(idx);
    }
    Ok(())
}

type OptionalColumnsPair = (Vec<String>, Vec<Vec<Option<f64>>>);

type OptionalColumnsWithRowLabels = (Vec<String>, Vec<Vec<Option<f64>>>, Vec<String>);

fn format_obs_row_labels(row_titles: &[String], n_rows: usize) -> Vec<String> {
    (0..n_rows)
        .map(|r| row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map_or_else(|| format!("Obs {}", r + 1), ToOwned::to_owned))
        .collect()
}

fn extract_optional_columns_with_row_labels(sheet: &DataSheet, selected: &[usize]) -> Result<OptionalColumnsWithRowLabels, StatsError> {
    match &sheet.archetype {
        TableArchetype::MultipleVariables(mv) => {
            validate_selected_data_sets(selected, mv.variables.len())?;
            let indices = if selected.is_empty() {
                (0..mv.variables.len()).filter(|&idx| !matches!(mv.variables[idx].role, VariableRole::Categorical | VariableRole::Label)).collect()
            } else {
                resolve_selected_indices(selected, mv.variables.len())
            };
            let mut labels = Vec::with_capacity(indices.len());
            let mut cols = Vec::with_capacity(indices.len());
            for idx in indices {
                let v = &mv.variables[idx];
                labels.push(v.title.clone());
                cols.push(v.cells.iter().map(|c| c.active_number()).collect());
            }
            let row_labels = format_obs_row_labels(&mv.row_titles, mv.row_count());
            Ok((labels, cols, row_labels))
        }
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if matches!(col_tab.subcolumn_mode, SubcolumnMode::RawReplicates(k) if k > 1) {
                return Err(StatsError::IncompatibleTable { analysis: "multivariate correlation", actual: curvebook_doc::TableKind::Column });
            }
            validate_selected_data_sets(selected, col_tab.data_sets.len())?;
            let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
            let mut labels = Vec::with_capacity(indices.len());
            let mut cols = Vec::with_capacity(indices.len());
            for idx in indices {
                let ds = &col_tab.data_sets[idx];
                labels.push(ds.title.clone());
                cols.push(ds.cells.iter().map(|subs| subs.first().and_then(|c| c.active_number())).collect());
            }
            let row_labels = format_obs_row_labels(&col_tab.row_titles, col_tab.row_count());
            Ok((labels, cols, row_labels))
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let total_cols = 1 + xy.data_sets.len();
            validate_selected_data_sets(selected, total_cols)?;
            let indices = resolve_selected_indices(selected, total_cols);
            if indices.contains(&0) && xy_x_is_pre_summarized(xy) {
                return Err(StatsError::RequiresRawReplicates);
            }
            let x_subs = xy_x_raw_subcols(xy);
            let y_subs = xy.y_subcolumn_mode.subcolumn_count().max(1);
            let uses_y = indices.iter().any(|&i| i > 0);
            let uses_x = indices.contains(&0);
            let n_subs = if uses_y && uses_x {
                y_subs.max(x_subs)
            } else if uses_y {
                y_subs
            } else {
                x_subs
            };

            let mut labels = Vec::with_capacity(indices.len());
            for &idx in &indices {
                if idx == 0 {
                    labels.push(xy.x_title.clone());
                } else {
                    labels.push(xy.data_sets[idx - 1].title.clone());
                }
            }
            let mut cols = vec![Vec::new(); indices.len()];
            let mut row_labels = Vec::new();
            for r in 0..xy.row_count() {
                let base_lbl =
                    xy.row_titles.get(r).map(|s| s.trim()).filter(|s| !s.is_empty()).map_or_else(|| format!("Obs {}", r + 1), ToOwned::to_owned);
                for sub in 0..n_subs {
                    if n_subs > 1 && sub > 0 {
                        let any_selected_entry = indices.iter().any(|&idx| {
                            if idx == 0 {
                                x_subs > 1 && xy.x_cells.get(r).and_then(|s| s.get(sub)).is_some_and(|c| !c.is_empty() || c.excluded)
                            } else {
                                xy.data_sets[idx - 1].cell(r, sub).is_some_and(|c| !c.is_empty() || c.excluded)
                            }
                        });
                        if !any_selected_entry {
                            continue;
                        }
                    }
                    for (pos, &idx) in indices.iter().enumerate() {
                        let val = if idx == 0 {
                            xy_x_active_at(xy, r, sub, x_subs)
                        } else {
                            xy.data_sets[idx - 1].cell(r, sub).and_then(curvebook_doc::Cell::active_number)
                        };
                        cols[pos].push(val);
                    }
                    row_labels.push(if n_subs > 1 { format!("{base_lbl} (rep {})", sub + 1) } else { base_lbl.clone() });
                }
            }
            Ok((labels, cols, row_labels))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "multivariate correlation", actual: other.kind() }),
    }
}

fn extract_optional_columns(sheet: &DataSheet, selected: &[usize]) -> Result<OptionalColumnsPair, StatsError> {
    let (labels, cols, _) = extract_optional_columns_with_row_labels(sheet, selected)?;
    Ok((labels, cols))
}

type CompleteMatrixWithNames = (Vec<String>, Vec<String>, Vec<Vec<f64>>);

fn append_pca_outputs(out: &mut ResultSheet, pca: &PcaResult, labels: Vec<String>) {
    if let Some(ev1) = pca.eigenvalues.first() {
        push_scalar(out, "PC1 eigenvalue", None, *ev1);
    }
    if let Some(vp1) = pca.variance_proportion.first() {
        push_scalar(out, "PC1 variance proportion", None, *vp1);
    }
    push_scalar(out, "Kaiser retained PCs", None, pca.kaiser_components as f64);

    let mut headers = vec![String::from("Eigenvalue"), String::from("Variance %"), String::from("Cumulative %")];
    if pca.parallel_analysis_thresholds.is_some() {
        headers.push(String::from("Parallel Analysis (95th pct)"));
    }
    let mut pc_labels = Vec::with_capacity(pca.eigenvalues.len());
    let mut scree_rows = Vec::with_capacity(pca.eigenvalues.len());
    for (idx, &ev) in pca.eigenvalues.iter().enumerate() {
        pc_labels.push(format!("PC{}", idx + 1));
        let mut r = vec![num_cell(ev), num_cell(100.0 * pca.variance_proportion[idx]), num_cell(100.0 * pca.cumulative_proportion[idx])];
        if let Some(pa) = &pca.parallel_analysis_thresholds {
            r.push(opt_num_cell(pa.get(idx).copied()));
        }
        scree_rows.push(r);
    }
    out.tabs.push(build_tab("pca-eigenvalues", "PCA eigenvalues & scree", ResultTabKind::MainSummary, headers, pc_labels.clone(), scree_rows));

    let load_rows: Vec<Vec<ResultCell>> = pca.loadings.iter().map(|r| r.iter().map(|&v| num_cell(v)).collect()).collect();
    out.tabs.push(build_tab("pca-loadings", "PCA component loadings", ResultTabKind::ParametersAndConfidenceIntervals, pc_labels, labels, load_rows));

    out.checklist.push(checklist_item(
        "Component retention (Kaiser & Parallel Analysis)",
        ChecklistStatus::Pass,
        Some(format!(
            "Kaiser = {} PCs{}",
            pca.kaiser_components,
            pca.parallel_analysis_components.map_or(String::new(), |c| format!(", Parallel Analysis = {c} PCs"))
        )),
        "Horn's parallel analysis compares empirical eigenvalues to 95th-percentile eigenvalues of uncorrelated Gaussian matrices.",
    ));
}

fn validate_pcr_column_indices(sheet: &DataSheet, selected: &[usize], outcome_col: usize) -> Result<Vec<usize>, StatsError> {
    let raw = extract_raw_optional_columns(sheet)?;
    if outcome_col >= raw.cols.len() {
        return Err(StatsError::MissingColumn { index: outcome_col });
    }
    let is_excluded_role = |col: usize| {
        raw.is_label.get(col).copied().unwrap_or(false)
            || matches!(
                &sheet.archetype,
                TableArchetype::MultipleVariables(mv)
                    if mv.variables.get(col).is_some_and(|v| matches!(v.role, VariableRole::Label | VariableRole::Categorical))
            )
    };
    if is_excluded_role(outcome_col) {
        return Err(StatsError::InvalidParameter { name: "pcr_outcome_column", value: outcome_col as f64 });
    }
    let mut pred_indices = Vec::new();
    if selected.is_empty() {
        for col in 0..raw.cols.len() {
            if col != outcome_col && !is_excluded_role(col) && raw.cols[col].iter().any(Option::is_some) {
                pred_indices.push(col);
            }
        }
    } else {
        for &col in selected {
            if col >= raw.cols.len() {
                return Err(StatsError::MissingColumn { index: col });
            }
            if col == outcome_col {
                return Err(StatsError::InvalidParameter { name: "pcr_outcome_column", value: outcome_col as f64 });
            }
            if is_excluded_role(col) {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: col as f64 });
            }
            if !pred_indices.contains(&col) {
                pred_indices.push(col);
            }
        }
    }
    if pred_indices.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "PCR predictors", required: 2, actual: pred_indices.len() });
    }
    Ok(pred_indices)
}

fn run_pcr_branch(
    sheet: &DataSheet,
    spec: &AnalysisSpec,
    standardize: bool,
    parallel_analysis_seed: Option<u64>,
    outcome_col: usize,
) -> Result<ResultSheet, StatsError> {
    let pred_indices = validate_pcr_column_indices(sheet, &spec.selected_data_sets, outcome_col)?;
    let p = pred_indices.len();
    let mut combined_cols = pred_indices;
    combined_cols.push(outcome_col);

    let (mut all_labels, obs_labels, rows) = extract_complete_matrix_rows_with_names(sheet, &combined_cols)?;
    let outcome_label = all_labels.pop().unwrap_or_else(|| format!("Column {}", outcome_col + 1));
    let pred_labels = all_labels;
    let n = rows.len();

    let mut flat_x = Vec::with_capacity(n * p);
    let mut y = Vec::with_capacity(n);
    let mut col_vals = vec![Vec::with_capacity(n); p];
    for row in &rows {
        for (j, &xv) in row[..p].iter().enumerate() {
            flat_x.push(xv);
            col_vals[j].push(xv);
        }
        y.push(row[p]);
    }
    let y_moments = SummaryMoments::from_slice(&y)?;
    let y_sd = y_moments.sample_sd().unwrap_or(0.0);
    if !y_sd.is_finite() || y_sd <= 0.0 {
        return Err(StatsError::ZeroVariance { context: "PCR outcome column" });
    }

    let x_mat = DenseMatrix::from_row_major(n, p, flat_x)?;
    let pca = pca_analysis(&x_mat, &pred_labels, standardize, parallel_analysis_seed)?;

    let k_avail = pca.loadings.first().map_or(0, Vec::len).min(pca.scores.first().map_or(0, Vec::len));
    if k_avail == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "PCR components", required: 1, actual: 0 });
    }
    let k_raw = pca.parallel_analysis_components.unwrap_or(pca.kaiser_components).max(1);
    let k_ret = k_raw.min(k_avail).min(n.saturating_sub(2)).max(1);
    if n < k_ret + 2 {
        return Err(StatsError::InsufficientSampleSize { context: "PCR observations", required: k_ret + 2, actual: n });
    }

    let mut score_data = Vec::with_capacity(n * (k_ret + 1));
    for score_row in &pca.scores {
        score_data.push(1.0);
        score_data.extend_from_slice(&score_row[..k_ret]);
    }
    let score_mat = DenseMatrix::from_row_major(n, k_ret + 1, score_data)?;
    let mut pc_names = Vec::with_capacity(k_ret + 1);
    pc_names.push(String::from("Intercept"));
    for c in 1..=k_ret {
        pc_names.push(format!("PC{c}"));
    }
    let pc_reg = multiple_linear_regression(&score_mat, &y, &pc_names, true, true, spec.confidence_level)?;

    let mut means = Vec::with_capacity(p);
    let mut scales = Vec::with_capacity(p);
    for vals in &col_vals {
        let m = SummaryMoments::from_slice(vals)?;
        means.push(m.mean);
        let scale = if standardize { m.sample_sd().ok().filter(|&s| s.is_finite() && s > 0.0).unwrap_or(1.0) } else { 1.0 };
        scales.push(scale);
    }

    // Build linear transform A ((p + 1) x (k_ret + 1)) mapping [gamma_0, gamma_1..gamma_k] -> [beta_0, beta_1..beta_p].
    let mut transform = vec![vec![0.0_f64; k_ret + 1]; p + 1];
    transform[0][0] = 1.0;
    for j in 0..p {
        for c in 0..k_ret {
            let w_jc = pca.loadings[j][c] / scales[j];
            transform[j + 1][c + 1] = w_jc;
            transform[0][c + 1] -= means[j] * w_jc;
        }
    }

    let df_res = pc_reg.df_residual;
    let alpha = spec.confidence_level.alpha();
    let p_fmt = spec.p_value_format;
    let ci_header = format!("{}% CI", spec.confidence_level.percent());
    let t_crit = if df_res > 0.0 { t_crit_two_sided(spec.confidence_level, df_res)? } else { f64::NAN };
    let gamma: Vec<f64> = pc_reg.terms.iter().map(|t| t.estimate).collect();
    let mut back_rows = Vec::with_capacity(p + 1);
    let mut back_names = Vec::with_capacity(p + 1);
    back_names.push(String::from("Intercept"));
    back_names.extend(pred_labels.iter().cloned());

    for a_row in &transform {
        let est: f64 = a_row.iter().zip(&gamma).map(|(a, g)| a * g).sum();
        let var_est = if pc_reg.cov_beta.len() == k_ret + 1 {
            let mut v = 0.0_f64;
            for (a_idx, &ai) in a_row.iter().enumerate() {
                for (b_idx, &bi) in a_row.iter().enumerate() {
                    v += ai * pc_reg.cov_beta[a_idx][b_idx] * bi;
                }
            }
            v.max(0.0)
        } else {
            a_row.iter().zip(&pc_reg.terms).map(|(a, t)| (a * t.se).powi(2)).sum()
        };
        let se = var_est.sqrt();
        let t_stat = if se > 0.0 { est / se } else { f64::NAN };
        let p_val = if t_stat.is_finite() && df_res > 0.0 { student_t_two_sided_p(t_stat, df_res)? } else { f64::NAN };
        let ci = if se.is_finite() && t_crit.is_finite() { finite_ci_cell(est - t_crit * se, est + t_crit * se) } else { ResultCell::Empty };
        back_rows.push(vec![num_cell(est), num_cell(se), num_cell(t_stat), p_cell_at_alpha(p_val, p_fmt, alpha), ci]);
    }

    let rmse = (pc_reg.rss / (n as f64)).sqrt();
    let mut out = init_result_sheet(spec)?;
    push_scalar(&mut out, "PCR R squared", None, pc_reg.r_squared);
    push_scalar(&mut out, "PCR Adjusted R squared", None, pc_reg.adjusted_r_squared);
    push_scalar(&mut out, "PCR F statistic", None, pc_reg.f_statistic);
    push_scalar(&mut out, "PCR F P value", None, pc_reg.f_p_value);
    push_scalar(&mut out, "PCR Sy.x", None, pc_reg.residual_sd);
    push_scalar(&mut out, "PCR RMSE", None, rmse);
    push_scalar(&mut out, "PCR Retained components", None, k_ret as f64);
    push_scalar(&mut out, "PCR Predictor count", None, p as f64);
    push_scalar(&mut out, "PCR Observation count", None, n as f64);
    append_pca_outputs(&mut out, &pca, pred_labels);

    out.tabs.insert(
        0,
        build_tab(
            "pcr-summary",
            "PCR model summary",
            ResultTabKind::MainSummary,
            vec![String::from("Value")],
            vec![
                format!("Outcome ({outcome_label})"),
                String::from("Retained components (k)"),
                String::from("R squared"),
                String::from("Adjusted R squared"),
                String::from("F statistic"),
                String::from("F P value"),
                String::from("Residual SD (Sy.x)"),
                String::from("RMSE"),
                String::from("AIC"),
                String::from("BIC"),
                String::from("Observations (n)"),
                String::from("Predictors (p)"),
            ],
            vec![
                vec![ResultCell::Text(outcome_label.clone())],
                vec![num_cell(k_ret as f64)],
                vec![num_cell(pc_reg.r_squared)],
                vec![num_cell(pc_reg.adjusted_r_squared)],
                vec![num_cell(pc_reg.f_statistic)],
                vec![p_cell_at_alpha(pc_reg.f_p_value, p_fmt, alpha)],
                vec![num_cell(pc_reg.residual_sd)],
                vec![num_cell(rmse)],
                vec![num_cell(pc_reg.aic)],
                vec![num_cell(pc_reg.bic)],
                vec![num_cell(n as f64)],
                vec![num_cell(p as f64)],
            ],
        ),
    );
    out.tabs.insert(
        1,
        build_tab(
            "pcr-predictors",
            "PCR predictor coefficients (original scale)",
            ResultTabKind::ParametersAndConfidenceIntervals,
            vec![String::from("Estimate"), String::from("SE"), String::from("t"), String::from("P value"), ci_header.clone()],
            back_names,
            back_rows,
        ),
    );
    let comp_rows: Vec<Vec<ResultCell>> = pc_reg
        .terms
        .iter()
        .map(|t| {
            vec![
                num_cell(t.estimate),
                num_cell(t.se),
                num_cell(t.t_statistic),
                p_cell_at_alpha(t.p_value, p_fmt, alpha),
                finite_ci_cell(t.ci.0, t.ci.1),
                opt_num_cell(t.vif),
            ]
        })
        .collect();
    let comp_names: Vec<String> = pc_reg.terms.iter().map(|t| t.name.clone()).collect();
    out.tabs.insert(
        2,
        build_tab(
            "pcr-components",
            "PCR retained principal component regression",
            ResultTabKind::ParametersAndConfidenceIntervals,
            vec![String::from("Estimate"), String::from("SE"), String::from("t"), String::from("P value"), ci_header, String::from("VIF")],
            comp_names,
            comp_rows,
        ),
    );
    let diag_rows: Vec<Vec<ResultCell>> = (0..n)
        .map(|i| {
            vec![
                num_cell(y[i]),
                num_cell(pc_reg.fitted[i]),
                num_cell(pc_reg.residuals[i]),
                num_cell(pc_reg.leverage[i]),
                num_cell(pc_reg.cooks_distance[i]),
            ]
        })
        .collect();
    out.tabs.insert(
        3,
        build_tab(
            "pcr-diagnostics",
            "PCR fitted values & residuals",
            ResultTabKind::Diagnostics,
            vec![
                format!("Observed ({outcome_label})"),
                String::from("Predicted Y"),
                String::from("Residual"),
                String::from("Leverage"),
                String::from("Cook's D"),
            ],
            obs_labels,
            diag_rows,
        ),
    );
    out.checklist.push(checklist_item(
        "Supervised PCR outcome exclusion (target leakage guard)",
        ChecklistStatus::Pass,
        Some(format!("Outcome = `{outcome_label}` (col {outcome_col}), retained PCs = {k_ret}/{k_avail}")),
        "Outcome column was strictly excluded from PCA decomposition and regressed on retained orthogonal component scores.",
    ));
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

fn extract_complete_matrix_rows_with_names(sheet: &DataSheet, selected: &[usize]) -> Result<CompleteMatrixWithNames, StatsError> {
    let (labels, opt_cols, all_row_labels) = extract_optional_columns_with_row_labels(sheet, selected)?;
    if labels.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate variables", required: 2, actual: labels.len() });
    }
    let n_rows = opt_cols[0].len();
    let mut obs_labels = Vec::new();
    let mut rows = Vec::new();
    for r in 0..n_rows {
        let mut row = Vec::with_capacity(labels.len());
        let mut ok = true;
        for col in &opt_cols {
            if let Some(v) = col.get(r).and_then(|x| *x)
                && v.is_finite()
            {
                row.push(v);
            } else {
                ok = false;
                break;
            }
        }
        if ok {
            let r_label = all_row_labels.get(r).cloned().unwrap_or_else(|| format!("Obs {}", r + 1));
            obs_labels.push(r_label);
            rows.push(row);
        }
    }
    if rows.len() < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "multivariate complete rows", required: 3, actual: rows.len() });
    }
    Ok((labels, obs_labels, rows))
}

fn extract_complete_matrix_rows(sheet: &DataSheet, selected: &[usize]) -> Result<(Vec<String>, Vec<Vec<f64>>), StatsError> {
    let (labels, _obs_labels, rows) = extract_complete_matrix_rows_with_names(sheet, selected)?;
    Ok((labels, rows))
}

/// Expose Pearson `r` helper directly for pairwise convenience.
#[must_use]
pub fn ci_cell_from_option(ci: Option<(f64, f64)>) -> ResultCell {
    match ci {
        Some((lo, hi)) => ci_cell(lo, hi),
        None => ResultCell::Empty,
    }
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ClusterDistance, ClusterLinkage, DataSheet, MultipleVariablesTable, MultivariateSpec, SheetId,
        TableArchetype, VariableRole,
    };

    use super::{extract_optional_columns, run_multivariate_analysis};

    #[test]
    fn empty_selection_excludes_categorical_and_label_columns_including_appended_cluster() -> Result<(), Box<dyn std::error::Error>> {
        let mut mv = MultipleVariablesTable::new(5, 6)?;
        mv.variables[0].title = String::from("SampleLabel");
        mv.variables[0].role = VariableRole::Label;
        mv.variables[1].title = String::from("F1");
        mv.variables[1].role = VariableRole::Continuous;
        mv.variables[2].title = String::from("F2");
        mv.variables[2].role = VariableRole::Continuous;
        mv.variables[3].title = String::from("F3Calc");
        mv.variables[3].role = VariableRole::Calculated;
        mv.variables[4].title = String::from("Cluster");
        mv.variables[4].role = VariableRole::Categorical;

        let coords = [[1.0, 1.0, 2.0], [1.1, 0.9, 2.1], [0.9, 1.1, 1.9], [10.0, 10.0, 8.0], [10.2, 9.8, 8.2], [9.9, 10.1, 7.9]];
        for (r, vals) in coords.iter().enumerate() {
            mv.variables[0].cells[r] = Cell::text(format!("S{}", r + 1));
            mv.variables[1].cells[r] = Cell::number(vals[0]);
            mv.variables[2].cells[r] = Cell::number(vals[1]);
            mv.variables[3].cells[r] = Cell::number(vals[2]);
            mv.variables[4].cells[r] = Cell::text(if r < 3 { "Cluster 1" } else { "Cluster 2" });
        }

        let sheet = DataSheet::new(SheetId::new("mv-cluster"), "Cluster MV", TableArchetype::MultipleVariables(mv))?;
        let (labels, cols) = extract_optional_columns(&sheet, &[])?;
        assert_eq!(labels, vec![String::from("F1"), String::from("F2"), String::from("F3Calc")]);
        assert_eq!(cols.len(), 3);

        let explicit = extract_optional_columns(&sheet, &[1, 4])?;
        assert_eq!(explicit.0, vec![String::from("F1"), String::from("Cluster")]);

        let mv_spec = MultivariateSpec::Clustering {
            k_means_k: Some(2),
            distance: ClusterDistance::Euclidean,
            linkage: ClusterLinkage::Ward,
            k_sweep_max: None,
        };
        let spec = AnalysisSpec::new(SheetId::new("res-cl"), SheetId::new("mv-cluster"), AnalysisKind::Multivariate(mv_spec.clone()))?;
        let res = run_multivariate_analysis(&sheet, &spec, &mv_spec)?;
        assert!(res.tabs.iter().any(|t| t.id == "kmeans-assignments"));
        Ok(())
    }
}
