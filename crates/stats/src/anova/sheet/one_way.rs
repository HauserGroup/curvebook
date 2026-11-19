//! `ResultSheet` builder for one-way ANOVA specs (ordinary, unequal-variance, repeated-measures,
//! mixed-model REML, Kruskal–Wallis, and Friedman), including checklists and multiplicity tabs.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisSpec, ChecklistItem, ChecklistStatus, DataSheet, MixedModelDfMethod, MultiplicityMethod, MultiplicitySpec, OneWayAnovaSpec, OneWayMethod,
    PValueFormat, ResidualCovariance, ResultCell, ResultSheet, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::SummaryMoments;

use crate::anova::factorial::{build_xtx_pinv_and_proj, collect_index_combinations, columns_to_matrix, kronecker_term_columns, sum_contrast_vectors};
use crate::anova::nonparametric::{friedman_test, kruskal_wallis_test};
use crate::anova::one_way::{ordinary_one_way_anova, ordinary_one_way_from_summary, rm_one_way_anova, unequal_variance_one_way_anova};
use crate::anova::posthoc::{
    ContrastFamilyEval, RemlContrastModel, collect_factor_pairs, compare_contrast_family, factorial_design_vector,
    push_pairwise_outputs_with_footnote,
};
use crate::anova::reml::{
    RemlContrastCore, cluster_ids_to_g_matrix, fit_multi_vc_reml_with_method_and_core, fit_random_intercept_reml_with_method_and_core,
};
use crate::anova::reml_unstructured::fit_unstructured_rm_reml_with_method_and_core;
use crate::anova::results::{FactorialAnovaTerm, OneWayAnovaResult, UnstructuredFallbackReason, VarianceHomogeneityResult};
use crate::anova::sheet::factorial::push_reml_df_scalars;
use crate::column::shapiro_wilk;
use crate::common::{
    append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, build_tab, checklist_item, extract_numeric_columns,
    init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell, push_scalar, rank_with_ties, resolve_selected_indices,
    sanitize_result_sheet_floats,
};
use crate::error::StatsError;
use crate::multiplicity::{
    GroupComparisonInput, MultiplicityProcedure, PairwiseComparisonDetail, compare_dunn_ranks, compare_groups_pairwise_with_confidence,
    compare_groups_with_confidence, linear_trend_test,
};

/// Execute a [`OneWayAnovaSpec`] against a `Column` or `MultipleVariables` [`DataSheet`].
pub fn run_one_way_anova(sheet: &DataSheet, spec: &AnalysisSpec, anova_spec: &OneWayAnovaSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if matches!(anova_spec.method, OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman) {
        return run_rm_one_way_anova_sheet(sheet, &mut out, anova_spec, confidence, p_fmt);
    }

    // Handle pre-summarized ColumnTable (`MeanSdN`, `MeanSemN`, `MeanCvN`) vs raw replicates.
    let (group_inputs, raw_res_opt, raw_slices_owned, ignored_non_numeric) = extract_one_way_inputs(sheet, &spec.selected_data_sets)?;

    match anova_spec.method {
        OneWayMethod::OrdinaryAnova => {
            let res = if let Some(r) = raw_res_opt { r } else { ordinary_one_way_from_summary(&group_inputs, confidence)? };
            push_scalar(&mut out, "F", None, res.f_statistic);
            push_scalar(&mut out, "P value", None, res.p_value);
            push_scalar(&mut out, "R squared", None, res.r_squared);
            push_scalar(&mut out, "MS residual", None, res.ms_within);
            push_scalar(&mut out, "Eta squared", None, res.effect_sizes.eta_squared);
            push_scalar(&mut out, "Omega squared", None, res.effect_sizes.omega_squared);

            let mut main_tab = build_one_way_tab(&res, anova_spec.compute_effect_sizes, confidence, p_fmt);
            if !res.variance_tests.available {
                main_tab.footnotes.push(String::from(
                    "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data.",
                ));
            }
            out.tabs.push(main_tab);
            out.tabs.push(build_variance_homogeneity_tab(&res.variance_tests, p_fmt));
            append_one_way_multiplicity(
                &mut out,
                &group_inputs,
                res.ms_within,
                res.df_within,
                res.ss_total,
                &anova_spec.multiplicity,
                p_fmt,
                confidence,
            )?;
            append_one_way_checklist(&mut out, &res, &group_inputs, &raw_slices_owned, false, confidence.alpha());
        }
        OneWayMethod::BrownForsytheAndWelch => {
            let base = if let Some(r) = raw_res_opt { r } else { ordinary_one_way_from_summary(&group_inputs, confidence)? };
            let uv = unequal_variance_one_way_anova(&group_inputs)?;
            push_scalar(&mut out, "Welch W", None, uv.welch_w);
            push_scalar(&mut out, "Welch P value", None, uv.welch_p);
            push_scalar(&mut out, "Brown-Forsythe F*", None, uv.brown_forsythe_f_star);
            push_scalar(&mut out, "Brown-Forsythe P value", None, uv.brown_forsythe_p);

            let headers = vec![String::from("Statistic"), String::from("DFn"), String::from("DFd"), String::from("P value")];
            let labels = vec![String::from("Brown-Forsythe ANOVA"), String::from("Welch's ANOVA")];
            let rows = vec![
                vec![
                    num_cell(uv.brown_forsythe_f_star),
                    num_cell(uv.brown_forsythe_df1),
                    num_cell(uv.brown_forsythe_df2),
                    p_cell(uv.brown_forsythe_p, p_fmt),
                ],
                vec![num_cell(uv.welch_w), num_cell(uv.welch_df1), num_cell(uv.welch_df2), p_cell(uv.welch_p, p_fmt)],
            ];
            let mut uv_tab = build_tab("unequal-var-anova", "Brown-Forsythe and Welch ANOVA", ResultTabKind::AnovaTable, headers, labels, rows);
            if !base.variance_tests.available {
                uv_tab.footnotes.push(String::from(
                    "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data.",
                ));
            }
            out.tabs.push(uv_tab);
            out.tabs.push(build_variance_homogeneity_tab(&base.variance_tests, p_fmt));

            let proc_eff = match MultiplicityProcedure::from(anova_spec.multiplicity.method) {
                MultiplicityProcedure::GamesHowell | MultiplicityProcedure::TamhaneT2 | MultiplicityProcedure::DunnettT3 => {
                    MultiplicityProcedure::from(anova_spec.multiplicity.method)
                }
                _ => MultiplicityProcedure::GamesHowell,
            };
            let details = compare_groups_with_confidence(
                &group_inputs,
                base.ms_within,
                base.df_within,
                proc_eff,
                anova_spec.multiplicity.threshold,
                anova_spec.multiplicity.report_confidence_intervals,
                confidence,
            )?;
            push_pairwise_outputs(&mut out, &details, "Multiple comparisons (Games-Howell / T2 / T3)", p_fmt, confidence);
            append_one_way_checklist(&mut out, &base, &group_inputs, &raw_slices_owned, true, confidence.alpha());
        }
        OneWayMethod::KruskalWallis => {
            if raw_slices_owned.is_empty() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let slice_refs: Vec<&[f64]> = raw_slices_owned.iter().map(Vec::as_slice).collect();
            let kw = kruskal_wallis_test(&slice_refs)?;
            push_scalar(&mut out, "Kruskal-Wallis H", None, kw.h_statistic);
            push_scalar(&mut out, "P value", None, kw.p_value);
            push_scalar(&mut out, "Epsilon squared", None, kw.epsilon_squared);

            let headers = vec![
                String::from("Statistic H"),
                String::from("df"),
                String::from("P value"),
                String::from("Tie correction"),
                String::from("Epsilon squared"),
            ];
            let rows = vec![vec![
                num_cell(kw.h_statistic),
                num_cell(kw.df),
                p_cell(kw.p_value, p_fmt),
                num_cell(kw.tie_correction),
                num_cell(kw.epsilon_squared),
            ]];
            out.tabs.push(build_tab(
                "kruskal-wallis",
                "Kruskal-Wallis test",
                ResultTabKind::AnovaTable,
                headers,
                vec![String::from("Kruskal-Wallis")],
                rows,
            ));

            // Compute global ties for Dunn's post-hoc test.
            let mut flat = Vec::new();
            for s in &raw_slices_owned {
                flat.extend_from_slice(s);
            }
            let ranked = rank_with_ties(&flat)?;
            let labels: Vec<(usize, String)> = group_inputs.iter().map(|g| (g.index, g.label.clone())).collect();
            let ns: Vec<usize> = raw_slices_owned.iter().map(Vec::len).collect();
            let details = compare_dunn_ranks(&labels, &ns, &kw.group_mean_ranks, kw.total_n, ranked.tie_sum_t3_minus_t, &anova_spec.multiplicity)?;
            let (dunn_title, dunn_footnote) = match MultiplicityProcedure::from(anova_spec.multiplicity.method) {
                MultiplicityProcedure::TukeyKramer
                | MultiplicityProcedure::NewmanKeuls
                | MultiplicityProcedure::GamesHowell
                | MultiplicityProcedure::DunnettT3 => (
                    "Dunn's multiple comparisons (Bonferroni fallback)",
                    Some("Parametric FWER post-hoc method mapped to Dunn's rank-sum comparisons with Bonferroni adjustment for Kruskal-Wallis."),
                ),
                MultiplicityProcedure::TamhaneT2 => (
                    "Dunn's multiple comparisons (Sidak fallback)",
                    Some("Tamhane's T2 mapped to Dunn's rank-sum comparisons with Sidak adjustment for Kruskal-Wallis."),
                ),
                MultiplicityProcedure::Dunnett { .. } => (
                    "Dunn's vs-control comparisons (Bonferroni fallback)",
                    Some(
                        "Dunnett vs-control selection mapped to Dunn's rank-sum comparisons vs control with Bonferroni adjustment over k - 1 pairs.",
                    ),
                ),
                MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd => (
                    "Dunn's multiple comparisons (uncorrected)",
                    Some("Unadjusted Dunn's rank-sum pairwise z tests (no multiplicity correction applied)."),
                ),
                _ => ("Dunn's multiple comparisons", None),
            };
            push_pairwise_outputs(&mut out, &details, dunn_title, p_fmt, confidence);
            if let Some(note) = dunn_footnote
                && let Some(tab) = out.tabs.last_mut()
            {
                tab.footnotes.push(String::from(note));
            }
            out.checklist.push(checklist_item(
                "Rank-based omnibus test",
                ChecklistStatus::Pass,
                Some(format!("H = {:.4}, N = {}", kw.h_statistic, kw.total_n)),
                "Kruskal-Wallis compares group rank distributions without assuming Gaussian residuals.",
            ));
        }
        OneWayMethod::RepeatedMeasures { .. } | OneWayMethod::Friedman => {}
    }

    append_ignored_text_checklist_and_footnote(&mut out, "one-way ANOVA", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

pub(in crate::anova) struct RmOneWayScan {
    pub(in crate::anova) blocks: Vec<Vec<f64>>,
    pub(in crate::anova) incomplete_rows: usize,
    pub(in crate::anova) non_numeric_cells: usize,
    pub(in crate::anova) y_vec: Vec<f64>,
    pub(in crate::anova) subj_ids: Vec<usize>,
    pub(in crate::anova) cond_ids: Vec<usize>,
    pub(in crate::anova) cond_counts: Vec<usize>,
}

pub(in crate::anova) fn scan_rm_one_way_column_table(col_tab: &curvebook_doc::ColumnTable, indices: &[usize]) -> RmOneWayScan {
    let row_count = col_tab.row_count();
    let mut active_end = 0_usize;
    for r in (0..row_count).rev() {
        let has_any = indices
            .iter()
            .any(|&c| col_tab.data_sets.get(c).and_then(|ds| ds.cell(r, 0)).is_some_and(|cell| cell.excluded || !cell.value.is_empty()));
        if has_any {
            active_end = r + 1;
            break;
        }
    }

    let mut blocks = Vec::new();
    let mut incomplete_rows = 0_usize;
    let mut non_numeric_cells = 0_usize;
    let mut y_vec = Vec::new();
    let mut subj_ids = Vec::new();
    let mut cond_ids = Vec::new();
    let mut cond_counts = vec![0_usize; indices.len()];

    for r in 0..active_end {
        let mut row_vals = Vec::with_capacity(indices.len());
        let mut row_long: Vec<(usize, f64)> = Vec::with_capacity(indices.len());
        let mut complete = true;
        let mut row_has_entry = false;

        for (pos, &c) in indices.iter().enumerate() {
            let cell = col_tab.data_sets.get(c).and_then(|ds| ds.cell(r, 0));
            if let Some(cell) = cell {
                if cell.excluded || !cell.value.is_empty() {
                    row_has_entry = true;
                }
                if !cell.excluded {
                    match &cell.value {
                        curvebook_doc::CellValue::Text(s) if !s.trim().is_empty() => non_numeric_cells += 1,
                        curvebook_doc::CellValue::Number(v) if !v.is_finite() => non_numeric_cells += 1,
                        _ => {}
                    }
                }
            }
            if let Some(val) = cell.and_then(curvebook_doc::Cell::active_number) {
                row_vals.push(val);
                row_long.push((pos, val));
            } else {
                complete = false;
            }
        }

        if complete && !row_vals.is_empty() {
            blocks.push(row_vals);
        } else if row_has_entry {
            incomplete_rows += 1;
        }
        if !row_long.is_empty() {
            for (pos, val) in row_long {
                subj_ids.push(r);
                cond_ids.push(pos);
                cond_counts[pos] += 1;
                y_vec.push(val);
            }
        }
    }

    RmOneWayScan { blocks, incomplete_rows, non_numeric_cells, y_vec, subj_ids, cond_ids, cond_counts }
}

pub(in crate::anova) fn emit_random_intercept_reml(
    out: &mut ResultSheet,
    x_mat: &DenseMatrix,
    y_vec: &[f64],
    cluster_ids: &[usize],
    term_slices: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
    p_fmt: PValueFormat,
) -> Result<(Vec<FactorialAnovaTerm>, f64, f64, f64, RemlContrastCore), StatsError> {
    let (reml, core) = fit_random_intercept_reml_with_method_and_core(x_mat, y_vec, cluster_ids, term_slices, df_method)?;
    push_scalar(out, "REML sigma^2 (subject)", None, reml.sigma2_random);
    push_scalar(out, "REML sigma^2 (residual)", None, reml.sigma2_error);
    push_scalar(out, "REML ICC", None, reml.icc);
    push_scalar(out, "REML log-likelihood", None, reml.reml_log_lik);
    push_reml_df_scalars(out, &reml.fixed_terms, &reml.satterthwaite_df, &reml.kenward_roger_df);

    let reml_headers =
        vec![String::from("Component"), String::from("Variance"), String::from("ICC"), String::from("LRT chi2"), String::from("LRT P")];
    let reml_rows = vec![
        vec![
            ResultCell::Text(String::from("Subject (random)")),
            num_cell(reml.sigma2_random),
            num_cell(reml.icc),
            num_cell(reml.lrt_random_chi2),
            p_cell(reml.lrt_random_p, p_fmt),
        ],
        vec![ResultCell::Text(String::from("Residual")), num_cell(reml.sigma2_error), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
    ];
    out.tabs.push(build_tab(
        "mixed-reml",
        "Mixed-effects REML variance components",
        ResultTabKind::ParametersAndConfidenceIntervals,
        reml_headers,
        vec![String::from("Subject"), String::from("Residual")],
        reml_rows,
    ));
    Ok((reml.fixed_terms, reml.sigma2_error, reml.reml_log_lik, reml.icc, core))
}

pub(in crate::anova) fn emit_multi_vc_rm_reml(
    out: &mut ResultSheet,
    x_mat: &DenseMatrix,
    y_vec: &[f64],
    strata: &[(&str, &str, &[usize])],
    term_slices: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<(Vec<FactorialAnovaTerm>, f64, f64, f64, RemlContrastCore), StatsError> {
    let mut specs = Vec::with_capacity(strata.len());
    for &(scalar_name, _, ids) in strata {
        let (g_mat, rank_c) = cluster_ids_to_g_matrix(ids)?;
        specs.push((scalar_name, g_mat, rank_c));
    }
    let refs: Vec<(&str, DenseMatrix, usize)> = specs.iter().map(|(n, g, c)| (*n, g.clone(), *c)).collect();
    let (mres, core) = fit_multi_vc_reml_with_method_and_core(x_mat, y_vec, &refs, term_slices, df_method)?;
    let sum_rand: f64 = mres.variance_components.iter().map(|(_, v)| *v).sum();
    let tot = (sum_rand + mres.sigma2_error).max(1e-18);
    let icc_subj = mres.variance_components.first().map_or(0.0, |(_, v)| *v / tot);

    for (name, var) in &mres.variance_components {
        push_scalar(out, format!("REML sigma^2 ({name})"), None, *var);
    }
    push_scalar(out, "REML sigma^2 (residual)", None, mres.sigma2_error);
    push_scalar(out, "REML ICC", None, icc_subj);
    push_scalar(out, "REML log-likelihood", None, mres.reml_log_lik);
    push_reml_df_scalars(out, &mres.fixed_terms, &mres.satterthwaite_df, &mres.kenward_roger_df);

    let vc_headers = vec![String::from("Component"), String::from("Variance"), String::from("SD"), String::from("% of total")];
    let mut vc_rows = Vec::with_capacity(strata.len() + 1);
    let mut vc_labels = Vec::with_capacity(strata.len() + 1);
    for (idx, (_, row_title, _)) in strata.iter().enumerate() {
        let var = mres.variance_components.get(idx).map_or(0.0, |(_, v)| *v);
        vc_labels.push((*row_title).to_owned());
        vc_rows.push(vec![ResultCell::Text((*row_title).to_owned()), num_cell(var), num_cell(var.sqrt()), num_cell(100.0 * var / tot)]);
    }
    vc_labels.push(String::from("Residual"));
    vc_rows.push(vec![
        ResultCell::Text(String::from("Residual")),
        num_cell(mres.sigma2_error),
        num_cell(mres.sigma2_error.sqrt()),
        num_cell(100.0 * mres.sigma2_error / tot),
    ]);
    out.tabs.push(build_tab(
        "mixed-reml",
        "Mixed-effects REML variance components",
        ResultTabKind::ParametersAndConfidenceIntervals,
        vc_headers,
        vc_labels,
        vc_rows,
    ));
    Ok((mres.fixed_terms, mres.sigma2_error, mres.reml_log_lik, icc_subj, core))
}

pub(in crate::anova) fn unstructured_fallback_label(reason: UnstructuredFallbackReason) -> &'static str {
    match reason {
        UnstructuredFallbackReason::SingleSubject => "single-subject isotropic covariance fallback",
        UnstructuredFallbackReason::SmallSampleRankDeficient => "small-sample rank-deficient shrinkage fallback",
        UnstructuredFallbackReason::SingularCovarianceEstimate => "singular covariance boundary shrinkage fallback",
        UnstructuredFallbackReason::IterationLimitReached => "iteration-budget regularized iterate fallback",
    }
}

pub(in crate::anova) fn build_matrix_tab(id: &str, title: &str, labels: &[String], matrix: &[Vec<f64>], footnotes: Vec<String>) -> ResultTab {
    let rows: Vec<Vec<ResultCell>> = matrix.iter().map(|row| row.iter().map(|&v| num_cell(v)).collect()).collect();
    let mut tab = build_tab(id, title, ResultTabKind::CovarianceMatrix, labels.to_vec(), labels.to_vec(), rows);
    tab.footnotes = footnotes;
    tab
}

#[allow(clippy::too_many_arguments)]
pub(in crate::anova) fn emit_unstructured_rm_reml(
    out: &mut ResultSheet,
    x_mat: &DenseMatrix,
    y_vec: &[f64],
    cluster_ids: &[usize],
    tp_ids: &[usize],
    tp_labels: &[String],
    term_slices: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<(Vec<FactorialAnovaTerm>, f64, f64, RemlContrastCore), StatsError> {
    let (ures, core) = fit_unstructured_rm_reml_with_method_and_core(x_mat, y_vec, cluster_ids, tp_ids, tp_labels, term_slices, df_method)?;
    push_scalar(out, "REML mean diagonal variance", None, ures.mean_diagonal_variance);
    push_scalar(out, "REML log-likelihood", None, ures.reml_log_lik);
    push_reml_df_scalars(out, &ures.fixed_terms, &ures.satterthwaite_df, &ures.kenward_roger_df);

    let mut dropped_labels = Vec::new();
    for (idx, lbl) in tp_labels.iter().enumerate() {
        if !ures.active_timepoint_indices.contains(&idx) {
            dropped_labels.push(lbl.clone());
        }
    }

    let mut footnotes = Vec::new();
    if !dropped_labels.is_empty() {
        footnotes.push(format!("Excluded all-missing timepoint(s): {}.", dropped_labels.join(", ")));
    }
    if let Some(reason) = ures.fallback_reason {
        footnotes.push(format!("Regularized SPD covariance used ({}).", unstructured_fallback_label(reason)));
    }

    out.tabs.push(build_matrix_tab(
        "unstructured-reml-covariance",
        "Estimated residual covariance matrix (REML)",
        &ures.timepoint_labels,
        &ures.covariance_matrix,
        footnotes.clone(),
    ));
    out.tabs.push(build_matrix_tab(
        "unstructured-reml-correlation",
        "Estimated residual correlation matrix (REML)",
        &ures.timepoint_labels,
        &ures.correlation_matrix,
        footnotes,
    ));

    let status = if ures.fallback_reason.is_some() || !dropped_labels.is_empty() { ChecklistStatus::Warn } else { ChecklistStatus::Info };
    let mut explanation = String::from(
        "Fitted unstructured t x t repeated-measures residual covariance by Cholesky REML with Fai-Cornelius Satterthwaite and Kenward-Roger degrees of freedom.",
    );
    if !dropped_labels.is_empty() {
        explanation.push_str(&format!(" Excluded all-missing timepoint(s): {}.", dropped_labels.join(", ")));
    }
    if let Some(reason) = ures.fallback_reason {
        explanation.push_str(&format!(" Engaged {}.", unstructured_fallback_label(reason)));
    }
    out.checklist.push(checklist_item(
        "Unstructured covariance REML",
        status,
        Some(format!(
            "t = {}, REML logL = {:.2}, mean diag var = {:.4}",
            ures.timepoint_labels.len(),
            ures.reml_log_lik,
            ures.mean_diagonal_variance
        )),
        explanation,
    ));

    let df_sum: f64 = ures.fixed_terms.iter().map(|t| t.df).sum();
    let df_resid = ((y_vec.len() as f64) - 1.0 - df_sum).max(1.0);
    Ok((ures.fixed_terms, ures.mean_diagonal_variance, df_resid, core))
}

pub(in crate::anova) struct RemlEffectDesign {
    pub(in crate::anova) x_mat: DenseMatrix,
    pub(in crate::anova) subsets: Vec<Vec<usize>>,
    pub(in crate::anova) factor_basis: Vec<Vec<Vec<f64>>>,
    pub(in crate::anova) terms: Vec<(String, usize, usize)>,
    pub(in crate::anova) observed_cells: BTreeSet<Vec<usize>>,
}

pub(in crate::anova) fn build_reml_effect_design(
    obs_cells: &[Vec<usize>],
    factor_names: &[String],
    factor_levels: &[Vec<String>],
    max_interaction_order: usize,
) -> Result<RemlEffectDesign, StatsError> {
    let n_obs = obs_cells.len();
    let n_factors = factor_names.len();
    let order_cap = max_interaction_order.clamp(1, n_factors);
    let mut subsets: Vec<Vec<usize>> = Vec::new();
    for ord in 1..=order_cap {
        collect_index_combinations(n_factors, ord, 0, &mut Vec::new(), &mut subsets);
    }
    let factor_basis: Vec<Vec<Vec<f64>>> = factor_levels.iter().map(|levels| sum_contrast_vectors(levels.len())).collect();
    let obs_dummy: Vec<(Vec<usize>, f64)> = obs_cells.iter().map(|c| (c.clone(), 0.0)).collect();
    let mut cols: Vec<Vec<f64>> = vec![vec![1.0_f64; n_obs]];
    let mut terms = Vec::with_capacity(subsets.len());
    for sub in &subsets {
        let col_start = cols.len();
        let prod_cols = kronecker_term_columns(&obs_dummy, &factor_basis, sub);
        let col_count = prod_cols.len();
        cols.extend(prod_cols);
        let name = sub.iter().map(|&idx| factor_names[idx].as_str()).collect::<Vec<_>>().join(" × ");
        terms.push((name, col_start, col_count));
    }
    let x_mat = columns_to_matrix(&cols, n_obs)?;
    let observed_cells: BTreeSet<Vec<usize>> = obs_cells.iter().cloned().collect();
    Ok(RemlEffectDesign { x_mat, subsets, factor_basis, terms, observed_cells })
}

#[allow(clippy::too_many_arguments)]
pub(in crate::anova) fn run_rm_one_way_reml(
    col_tab: &curvebook_doc::ColumnTable,
    out: &mut ResultSheet,
    indices: &[usize],
    scan: &RmOneWayScan,
    anova_spec: &OneWayAnovaSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let k = indices.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "repeated-measures groups", required: 2, actual: k });
    }
    if scan.y_vec.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    if let Some((empty_pos, _)) = scan.cond_counts.iter().enumerate().find(|(_, count)| **count == 0) {
        let _ = empty_pos;
        return Err(StatsError::EmptyInput);
    }
    let n_obs = scan.y_vec.len();
    if anova_spec.residual_covariance == ResidualCovariance::CompoundSymmetry && n_obs <= k {
        return Err(StatsError::InsufficientSampleSize { context: "RM REML observations", required: k + 1, actual: n_obs });
    }

    let cond_labels: Vec<String> = indices.iter().map(|&c| col_tab.data_sets[c].title.clone()).collect();
    let factor_names = vec![String::from("Treatment (between columns)")];
    let factor_levels = vec![cond_labels.clone()];
    let obs_cells: Vec<Vec<usize>> = scan.cond_ids.iter().map(|&c| vec![c]).collect();
    let RemlEffectDesign { x_mat, subsets, factor_basis, terms: term_ranges, observed_cells } =
        build_reml_effect_design(&obs_cells, &factor_names, &factor_levels, 1)?;
    let term_slices: Vec<(&str, usize, usize)> = term_ranges.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();

    let (fixed_terms, sigma2_e, df_resid, reml_core, model_desc) = match anova_spec.residual_covariance {
        ResidualCovariance::Unstructured => {
            let (terms, ms_e, df_r, core) = emit_unstructured_rm_reml(
                out,
                &x_mat,
                &scan.y_vec,
                &scan.subj_ids,
                &scan.cond_ids,
                &cond_labels,
                &term_slices,
                anova_spec.df_method,
            )?;
            (terms, ms_e, df_r, core, "unstructured repeated-measures residual covariance")
        }
        ResidualCovariance::CompoundSymmetry => {
            let (terms, ms_e, _, _, core) =
                emit_random_intercept_reml(out, &x_mat, &scan.y_vec, &scan.subj_ids, &term_slices, anova_spec.df_method, p_fmt)?;
            (terms, ms_e, (n_obs - k) as f64, core, "random-intercept compound-symmetry REML")
        }
    };

    let trt_term = fixed_terms.first().ok_or(StatsError::EmptyInput)?;
    let denom_df = match anova_spec.df_method {
        MixedModelDfMethod::KenwardRoger => scalar_value_or_fallback(&out.scalars, "Treatment (between columns): Kenward-Roger df", df_resid),
        MixedModelDfMethod::Satterthwaite => scalar_value_or_fallback(&out.scalars, "Treatment (between columns): Satterthwaite df", df_resid),
    };
    let f_trt = trt_term.f_statistic.unwrap_or(0.0);
    let p_trt = trt_term.p_value.unwrap_or(1.0);
    push_scalar(out, "F (treatment)", None, f_trt);
    push_scalar(out, "P value", None, p_trt);
    push_scalar(out, "Denominator df", None, denom_df);
    if let Some(pe) = trt_term.partial_eta_squared {
        push_scalar(out, "Partial eta squared", None, pe);
    }

    let mut headers =
        vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value"), String::from("Denom df")];
    if anova_spec.compute_effect_sizes {
        headers.push(String::from("Partial eta squared"));
    }
    let ss_res = sigma2_e * denom_df;
    let mut row_trt =
        vec![num_cell(trt_term.ss), num_cell(trt_term.df), num_cell(trt_term.ms), num_cell(f_trt), p_cell(p_trt, p_fmt), num_cell(denom_df)];
    let mut row_res = vec![num_cell(ss_res), num_cell(denom_df), num_cell(sigma2_e), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if anova_spec.compute_effect_sizes {
        row_trt.push(opt_num_cell(trt_term.partial_eta_squared));
        row_res.push(ResultCell::Empty);
    }
    out.tabs.push(build_tab(
        "rm-one-way-anova",
        "RM one-way ANOVA (Mixed-effects REML)",
        ResultTabKind::AnovaTable,
        headers,
        vec![String::from("Treatment (between columns)"), String::from("Residual (REML)")],
        vec![row_trt, row_res],
    ));

    let (_, row_proj) = build_xtx_pinv_and_proj(&x_mat)?;
    let reml_model = RemlContrastModel {
        factor_levels: vec![k],
        factor_basis,
        subsets,
        total_cols: x_mat.ncols(),
        row_proj,
        col_map: reml_core.col_map,
        beta: reml_core.beta,
        phi: reml_core.phi,
        phi_a: reml_core.phi_a,
        w_info_inv: reml_core.w_info_inv,
        phi_m_phi: reml_core.phi_m_phi,
        n_minus_p: reml_core.n_minus_p,
        y_scale: reml_core.y_scale,
        df_method: reml_core.df_method,
        observed_cells,
    };

    if anova_spec.multiplicity.method != MultiplicityMethod::None {
        if anova_spec.multiplicity.method == MultiplicityMethod::LinearTrend {
            let kf = k as f64;
            let weights: Vec<f64> = (0..k).map(|j| (j as f64) - 0.5 * (kf - 1.0)).collect();
            let pos_sum: f64 = weights.iter().filter(|&&w| w > 0.0).sum();
            let mut wa = vec![0.0_f64; x_mat.ncols()];
            let mut wb = vec![0.0_f64; x_mat.ncols()];
            for (j, &w) in weights.iter().enumerate() {
                let dv = factorial_design_vector(reml_model.total_cols, &reml_model.subsets, &reml_model.factor_basis, &[j]);
                let scale = w.abs() / pos_sum.max(1.0);
                let dst = if w >= 0.0 { &mut wa } else { &mut wb };
                for (d, &v) in dst.iter_mut().zip(dv.iter()) {
                    *d += scale * v;
                }
            }
            if let Some(est) = reml_model.eval_weight_vecs(&wa, &wb)? {
                let f_tr = if est.se > 0.0 { (est.diff / est.se).powi(2) } else { 0.0 };
                let p_tr = f_sf(f_tr, 1.0, est.df)?;
                push_scalar(out, "Linear trend F", None, f_tr);
                push_scalar(out, "Linear trend P", None, p_tr);
                out.tabs.push(build_tab(
                    "linear-trend",
                    "Test for linear trend",
                    ResultTabKind::MultipleComparisons,
                    vec![String::from("Contrast"), String::from("SE"), String::from("F"), String::from("P value")],
                    vec![String::from("Linear trend")],
                    vec![vec![num_cell(est.diff), num_cell(est.se), num_cell(f_tr), p_cell(p_tr, p_fmt)]],
                ));
            }
        } else {
            let mut mapped_mult = anova_spec.multiplicity;
            if let MultiplicityMethod::Dunnett { control_column } = mapped_mult.method {
                let ctrl_pos = indices
                    .iter()
                    .position(|&c| c == control_column)
                    .ok_or(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })?;
                mapped_mult.method = MultiplicityMethod::Dunnett { control_column: ctrl_pos };
            }
            let pairs = collect_factor_pairs(k, mapped_mult.method, |i, j| reml_model.eval_marginal_pair(0, i, j))?;
            if !pairs.is_empty() {
                let numbered_levels: Vec<(usize, String)> = indices.iter().zip(cond_labels.iter()).map(|(&c, l)| (c, l.clone())).collect();
                let mut level_means = Vec::with_capacity(k);
                for lvl in 0..k {
                    level_means.push(reml_model.level_mean_estimate(0, lvl)?);
                }
                let (details, fam_note) = compare_contrast_family(ContrastFamilyEval {
                    levels: &numbered_levels,
                    level_means: &level_means,
                    mult_spec: &mapped_mult,
                    confidence,
                    n_slices: 1,
                    pairs,
                })?;
                push_pairwise_outputs_with_footnote(out, &details, "multiple-comparisons", "Multiple comparisons", p_fmt, confidence, fam_note);
            }
        }
    }

    let n_subjects = scan.subj_ids.iter().copied().collect::<BTreeSet<_>>().len();
    let df_lbl = match anova_spec.df_method {
        MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite",
    };
    out.checklist.push(checklist_item(
        "Mixed-effects REML (repeated measures)",
        ChecklistStatus::Info,
        Some(format!(
            "{} subjects, {} observations, {} incomplete block(s), {} df = {:.2}",
            n_subjects, n_obs, scan.incomplete_rows, df_lbl, denom_df
        )),
        format!("Fitted mixed-effects REML ({model_desc}) with {df_lbl} degrees of freedom across all available repeated measurements without listwise deletion."),
    ));
    if scan.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(
            out,
            "Repeated-measures one-way ANOVA",
            &[(String::from("Selected columns"), scan.non_numeric_cells)],
        );
    }
    sanitize_result_sheet_floats(out)?;
    Ok(out.clone())
}

pub(in crate::anova) fn scalar_value_or_fallback(scalars: &[curvebook_doc::ResultScalarEntry], key: &str, fallback: f64) -> f64 {
    scalars.iter().find(|s| s.key == key).map_or(fallback, |s| s.value)
}

pub(in crate::anova) fn run_rm_one_way_anova_sheet(
    sheet: &DataSheet,
    out: &mut ResultSheet,
    anova_spec: &OneWayAnovaSpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Column(col_tab) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "RM one-way ANOVA", actual: sheet.archetype.kind() });
    };
    let indices = resolve_selected_indices(&spec_selected_from_sheet(out), col_tab.data_sets.len());
    let scan = scan_rm_one_way_column_table(col_tab, &indices);

    if let OneWayMethod::RepeatedMeasures { .. } = anova_spec.method {
        if scan.y_vec.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        if scan.incomplete_rows > 0 || anova_spec.mixed_effects_reml || anova_spec.residual_covariance == ResidualCovariance::Unstructured {
            return run_rm_one_way_reml(col_tab, out, &indices, &scan, anova_spec, confidence, p_fmt);
        }
    }

    if scan.blocks.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let blocks = &scan.blocks;

    let rm_analysis_name = match anova_spec.method {
        OneWayMethod::RepeatedMeasures { .. } => "Repeated-measures one-way ANOVA",
        OneWayMethod::Friedman => "Friedman test",
        _ => "Repeated-measures analysis",
    };

    match anova_spec.method {
        OneWayMethod::RepeatedMeasures { geisser_greenhouse } => {
            let rm = rm_one_way_anova(blocks)?;
            let p_eff = if geisser_greenhouse { rm.p_greenhouse_geisser } else { rm.p_uncorrected };
            push_scalar(out, "F (treatment)", None, rm.f_treatment);
            push_scalar(out, "P value", None, p_eff);
            push_scalar(out, "Greenhouse-Geisser epsilon", None, rm.sphericity.greenhouse_geisser_eps);
            push_scalar(out, "Huynh-Feldt epsilon", None, rm.sphericity.huynh_feldt_eps);
            push_scalar(out, "Partial eta squared", None, rm.partial_eta_squared);

            let headers = vec![
                String::from("SS"),
                String::from("df"),
                String::from("MS"),
                String::from("F"),
                String::from("P value"),
                String::from("G-G P"),
                String::from("H-F P"),
            ];
            let labels = vec![
                String::from("Treatment (between columns)"),
                String::from("Individual (subjects)"),
                String::from("Residual (random)"),
                String::from("Total"),
            ];
            let rows = vec![
                vec![
                    num_cell(rm.ss_treatment),
                    num_cell(rm.df_treatment),
                    num_cell(rm.ms_treatment),
                    num_cell(rm.f_treatment),
                    p_cell(rm.p_uncorrected, p_fmt),
                    p_cell(rm.p_greenhouse_geisser, p_fmt),
                    p_cell(rm.p_huynh_feldt, p_fmt),
                ],
                vec![
                    num_cell(rm.ss_subjects),
                    num_cell(rm.df_subjects),
                    num_cell(rm.ms_subjects),
                    num_cell(rm.f_subjects),
                    p_cell(rm.p_subjects, p_fmt),
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
                vec![
                    num_cell(rm.ss_residual),
                    num_cell(rm.df_residual),
                    num_cell(rm.ms_residual),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
                vec![
                    num_cell(rm.ss_total),
                    num_cell(rm.df_treatment + rm.df_subjects + rm.df_residual),
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                    ResultCell::Empty,
                ],
            ];
            out.tabs.push(build_tab("rm-one-way-anova", "RM one-way ANOVA", ResultTabKind::AnovaTable, headers, labels, rows));

            // Group summaries from complete blocks for multiplicity.
            let mut inputs = Vec::with_capacity(indices.len());
            for (pos, &col_idx) in indices.iter().enumerate() {
                let col_vals: Vec<f64> = blocks.iter().map(|b| b[pos]).collect();
                let m = SummaryMoments::from_slice(&col_vals)?;
                inputs.push(GroupComparisonInput {
                    index: col_idx,
                    label: col_tab.data_sets[col_idx].title.clone(),
                    n: rm.n_subjects as f64,
                    mean: m.mean,
                    sd: m.sample_sd().unwrap_or(0.0),
                });
            }
            append_one_way_multiplicity(out, &inputs, rm.ms_residual, rm.df_residual, rm.ss_total, &anova_spec.multiplicity, p_fmt, confidence)?;

            let sph_status = if rm.sphericity.greenhouse_geisser_eps < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass };
            let mauchly_p_lbl = if rm.sphericity.mauchly_p.is_finite() { format!("{:.4}", rm.sphericity.mauchly_p) } else { String::from("n/a") };
            out.checklist.push(checklist_item(
                "Sphericity (Mauchly / Greenhouse-Geisser)",
                sph_status,
                Some(format!(
                    "eps_GG = {:.4}, eps_HF = {:.4}, Mauchly P = {mauchly_p_lbl}",
                    rm.sphericity.greenhouse_geisser_eps, rm.sphericity.huynh_feldt_eps
                )),
                if geisser_greenhouse {
                    "Geisser-Greenhouse correction is applied to treatment degrees of freedom."
                } else {
                    "Greenhouse-Geisser epsilon < 0.75 indicates departure from covariance sphericity for unadjusted repeated-measures F degrees of freedom."
                },
            ));
            let condition_slices: Vec<Vec<f64>> = (0..indices.len()).map(|pos| blocks.iter().map(|b| b[pos]).collect()).collect();
            append_group_normality_checklist(
                &mut out.checklist,
                &condition_slices,
                confidence.alpha(),
                "Repeated-measures condition normality (Shapiro-Wilk)",
            );
        }
        OneWayMethod::Friedman => {
            let fr = friedman_test(blocks)?;
            push_scalar(out, "Friedman Q", None, fr.q_statistic);
            push_scalar(out, "P value", None, fr.p_value);
            push_scalar(out, "Kendall's W", None, fr.kendalls_w);
            let headers = vec![
                String::from("Friedman statistic Q"),
                String::from("df"),
                String::from("P value"),
                String::from("Kendall's W"),
                String::from("Blocks"),
            ];
            let rows = vec![vec![
                num_cell(fr.q_statistic),
                num_cell(fr.df),
                p_cell(fr.p_value, p_fmt),
                num_cell(fr.kendalls_w),
                num_cell(fr.n_blocks as f64),
            ]];
            out.tabs.push(build_tab("friedman", "Friedman test", ResultTabKind::AnovaTable, headers, vec![String::from("Friedman")], rows));
            out.checklist.push(checklist_item(
                "Matched nonparametric blocks",
                ChecklistStatus::Pass,
                Some(format!("{} complete subject blocks, W = {:.3}", fr.n_blocks, fr.kendalls_w)),
                "Friedman ranks conditions within each matched subject block.",
            ));
        }
        _ => {}
    }
    append_listwise_filtered_checklist_and_footnote(out, rm_analysis_name, scan.incomplete_rows, blocks.len());
    if scan.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(out, rm_analysis_name, &[(String::from("Selected columns"), scan.non_numeric_cells)]);
    }
    sanitize_result_sheet_floats(out)?;
    Ok(out.clone())
}

pub(in crate::anova) fn spec_selected_from_sheet(sheet: &ResultSheet) -> Vec<usize> {
    sheet.analysis.selected_data_sets.clone()
}

pub(in crate::anova) type ExtractedOneWayInputs = (Vec<GroupComparisonInput>, Option<OneWayAnovaResult>, Vec<Vec<f64>>, Vec<(String, usize)>);

pub(in crate::anova) fn extract_one_way_inputs(sheet: &DataSheet, selected: &[usize]) -> Result<ExtractedOneWayInputs, StatsError> {
    if let TableArchetype::Column(col_tab) = &sheet.archetype
        && col_tab.subcolumn_mode.supports_parametric_moments()
    {
        let indices = resolve_selected_indices(selected, col_tab.data_sets.len());
        let mut inputs = Vec::with_capacity(indices.len());
        for idx in indices {
            let ds = &col_tab.data_sets[idx];
            let mut mean_val = None;
            let mut err_val = None;
            let mut n_val = None;
            for subs in &ds.cells {
                if let (Some(m), Some(e), Some(n)) = (
                    subs.first().and_then(|c| c.active_number()),
                    subs.get(1).and_then(|c| c.active_number()),
                    subs.get(2).and_then(|c| c.active_number()),
                ) {
                    mean_val = Some(m);
                    err_val = Some(e);
                    n_val = Some(n);
                    break;
                }
            }
            let (Some(mean), Some(err), Some(n)) = (mean_val, err_val, n_val) else {
                return Err(StatsError::InsufficientSampleSize { context: "pre-summarized group", required: 2, actual: 0 });
            };
            let sd = match col_tab.subcolumn_mode {
                SubcolumnMode::MeanSdN => err,
                SubcolumnMode::MeanSemN => err * n.sqrt(),
                SubcolumnMode::MeanCvN => (err / 100.0) * mean.abs(),
                _ => err,
            };
            inputs.push(GroupComparisonInput { index: idx, label: ds.title.clone(), n, mean, sd });
        }
        return Ok((inputs, None, Vec::new(), Vec::new()));
    }

    let cols = extract_numeric_columns(sheet, selected)?;
    let ignored_non_numeric: Vec<(String, usize)> =
        cols.iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect();
    let raw_slices: Vec<Vec<f64>> = cols.iter().map(|c| c.values.clone()).collect();
    let paired: Vec<(&str, &[f64])> = cols.iter().map(|c| (c.label.as_str(), c.values.as_slice())).collect();
    let res = ordinary_one_way_anova(&paired, ConfidenceLevel::DEFAULT_95)?;
    let inputs = cols
        .iter()
        .enumerate()
        .map(|(i, c)| GroupComparisonInput {
            index: c.index,
            label: c.label.clone(),
            n: res.group_n[i] as f64,
            mean: res.group_means[i],
            sd: res.group_sds[i],
        })
        .collect();
    Ok((inputs, Some(res), raw_slices, ignored_non_numeric))
}

pub(in crate::anova) fn build_variance_homogeneity_tab(vt: &VarianceHomogeneityResult, p_fmt: PValueFormat) -> ResultTab {
    let headers = vec![String::from("Statistic"), String::from("DFn / df"), String::from("DFd"), String::from("P value"), String::from("Status")];
    let labels = vec![String::from("Brown-Forsythe"), String::from("Bartlett")];
    let mut tab = if vt.available && vt.brown_forsythe_p.is_finite() {
        let rows = vec![
            vec![
                num_cell(vt.brown_forsythe_f),
                num_cell(vt.brown_forsythe_df1),
                num_cell(vt.brown_forsythe_df2),
                p_cell(vt.brown_forsythe_p, p_fmt),
                ResultCell::Text(if vt.brown_forsythe_p >= 0.05 {
                    String::from("Equal variances not rejected")
                } else {
                    String::from("Variances differ")
                }),
            ],
            vec![
                num_cell(vt.bartlett_chi2),
                num_cell(vt.bartlett_df),
                ResultCell::Empty,
                p_cell(vt.bartlett_p, p_fmt),
                ResultCell::Text(if vt.bartlett_p >= 0.05 { String::from("Equal variances not rejected") } else { String::from("Variances differ") }),
            ],
        ];
        build_tab("variance-homogeneity", "Test for homogeneity of variances", ResultTabKind::Diagnostics, headers, labels, rows)
    } else {
        let rows = vec![
            vec![
                ResultCell::Empty,
                num_cell(vt.brown_forsythe_df1),
                num_cell(vt.brown_forsythe_df2),
                ResultCell::Empty,
                ResultCell::Text(String::from("Not computable from summary statistics")),
            ],
            vec![
                ResultCell::Empty,
                num_cell(vt.bartlett_df),
                ResultCell::Empty,
                ResultCell::Empty,
                ResultCell::Text(String::from("Not computable from summary statistics")),
            ],
        ];
        build_tab("variance-homogeneity", "Test for homogeneity of variances", ResultTabKind::Diagnostics, headers, labels, rows)
    };
    if !vt.available {
        tab.footnotes.push(String::from(
            "Brown-Forsythe and Bartlett variance homogeneity tests cannot be computed from pre-summarized Mean/SD/N data; raw replicate observations are required.",
        ));
    }
    tab
}

pub(in crate::anova) fn build_one_way_tab(
    res: &OneWayAnovaResult,
    include_effect: bool,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> ResultTab {
    let mut headers = vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")];
    if include_effect {
        headers.push(String::from("Eta squared"));
        headers.push(format!("{}% CI (partial eta2)", confidence.percent()));
        headers.push(String::from("Omega squared"));
        headers.push(String::from("Cohen's f"));
    }
    let labels = vec![String::from("Treatment (between columns)"), String::from("Residual (within columns)"), String::from("Total")];
    let mut row_treat =
        vec![num_cell(res.ss_between), num_cell(res.df_between), num_cell(res.ms_between), num_cell(res.f_statistic), p_cell(res.p_value, p_fmt)];
    if include_effect {
        row_treat.push(num_cell(res.effect_sizes.eta_squared));
        row_treat.push(opt_ci_cell(res.effect_sizes.partial_eta_squared_ci));
        row_treat.push(num_cell(res.effect_sizes.omega_squared));
        row_treat.push(num_cell(res.effect_sizes.cohens_f));
    }
    let mut row_resid = vec![num_cell(res.ss_within), num_cell(res.df_within), num_cell(res.ms_within), ResultCell::Empty, ResultCell::Empty];
    let mut row_total = vec![num_cell(res.ss_total), num_cell(res.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if include_effect {
        row_resid.extend(vec![ResultCell::Empty; 4]);
        row_total.extend(vec![ResultCell::Empty; 4]);
    }
    build_tab("anova-table", "ANOVA summary", ResultTabKind::AnovaTable, headers, labels, vec![row_treat, row_resid, row_total])
}

#[allow(clippy::too_many_arguments)]
pub(in crate::anova) fn append_one_way_multiplicity(
    out: &mut ResultSheet,
    inputs: &[GroupComparisonInput],
    ms_within: f64,
    df_within: f64,
    ss_total: f64,
    mult: &MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    if mult.method == MultiplicityMethod::LinearTrend {
        let trend = linear_trend_test(inputs, ms_within, df_within)?;
        let r_squared = if ss_total > 0.0 { (trend.ss_trend / ss_total).clamp(0.0, 1.0) } else { trend.r_squared };
        push_scalar(out, "Linear trend F", None, trend.f_statistic);
        push_scalar(out, "Linear trend P", None, trend.p_value);
        let headers = vec![String::from("Slope"), String::from("R squared"), String::from("F"), String::from("P value")];
        let rows = vec![vec![num_cell(trend.slope_estimate), num_cell(r_squared), num_cell(trend.f_statistic), p_cell(trend.p_value, p_fmt)]];
        out.tabs.push(build_tab(
            "linear-trend",
            "Test for linear trend",
            ResultTabKind::MultipleComparisons,
            headers,
            vec![String::from("Linear trend")],
            rows,
        ));
        return Ok(());
    }
    let details = compare_groups_pairwise_with_confidence(inputs, ms_within, df_within, mult, confidence)?;
    push_pairwise_outputs(out, &details, "Multiple comparisons", p_fmt, confidence);
    Ok(())
}

pub(in crate::anova) fn push_pairwise_outputs(
    out: &mut ResultSheet,
    details: &[PairwiseComparisonDetail],
    title: &str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) {
    push_pairwise_outputs_with_id(out, details, "multiple-comparisons", title, p_fmt, confidence);
}

pub(in crate::anova) fn push_pairwise_outputs_with_id(
    out: &mut ResultSheet,
    details: &[PairwiseComparisonDetail],
    tab_id: &str,
    title: &str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) {
    push_pairwise_outputs_with_footnote(out, details, tab_id, title, p_fmt, confidence, None);
}

pub(in crate::anova) fn append_group_normality_checklist(checklist: &mut Vec<ChecklistItem>, raw_slices: &[Vec<f64>], alpha: f64, assumption: &str) {
    let worst = raw_slices.iter().filter_map(|s| shapiro_wilk(s, alpha).ok()).min_by(|a, b| a.p_value.total_cmp(&b.p_value));
    if let Some(sw) = worst {
        let violated = !sw.passed;
        checklist.push(checklist_item(
            assumption,
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("min Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
            if violated {
                "At least one group departs significantly from Gaussian normality; inspect group distributions and Normal Q-Q diagnostics."
            } else {
                "Group distributions do not depart significantly from Gaussian normality."
            },
        ));
    }
}

pub(in crate::anova) fn append_one_way_checklist(
    out: &mut ResultSheet,
    res: &OneWayAnovaResult,
    inputs: &[GroupComparisonInput],
    raw_slices: &[Vec<f64>],
    is_welch: bool,
    alpha: f64,
) {
    if res.variance_tests.available && res.variance_tests.brown_forsythe_p.is_finite() {
        let equal_var_ok = res.variance_tests.brown_forsythe_p >= 0.05;
        let (var_status, var_explanation) = if equal_var_ok {
            (ChecklistStatus::Pass, "Group variances do not differ significantly; ordinary pooled-variance ANOVA is appropriate.")
        } else if is_welch {
            (ChecklistStatus::Info, "Variances appear unequal (P < 0.05); Brown-Forsythe & Welch ANOVA does not assume equal variances.")
        } else {
            (ChecklistStatus::Warn, "Variances appear unequal (Brown-Forsythe P < 0.05), violating the pooled equal-variance assumption.")
        };
        out.checklist.push(checklist_item(
            "Homogeneity of variances (Brown-Forsythe)",
            var_status,
            Some(format!("F = {:.3}, P = {:.4}", res.variance_tests.brown_forsythe_f, res.variance_tests.brown_forsythe_p)),
            var_explanation,
        ));
    } else {
        out.checklist.push(checklist_item(
            "Homogeneity of variances (Brown-Forsythe / Bartlett)",
            ChecklistStatus::Info,
            Some(String::from("Not computable from summary statistics")),
            "Brown-Forsythe and Bartlett variance homogeneity tests are not computable from pre-summarized Mean/SD/N data; raw replicate observations are required.",
        ));
    }
    append_group_normality_checklist(&mut out.checklist, raw_slices, alpha, "Group normality (Shapiro-Wilk)");
    let min_n = inputs.iter().map(|g| g.n as usize).min().unwrap_or(0);
    out.checklist.push(checklist_item(
        "Group sample size",
        if min_n >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("min n = {min_n}, total N = {}", res.total_n)),
        "ANOVA is moderately robust to mild non-normality when group sizes are balanced and n >= 5.",
    ));
}
