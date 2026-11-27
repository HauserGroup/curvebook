//! `ResultSheet` builder for nested (hierarchical) comparison specs.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558
//! - Nakagawa, S., & Schielzeth, H. (2013). A general and simple method for obtaining $R^2$ from
//!   generalized linear mixed-effects models. *Methods in Ecology and Evolution* 4(2):133–142.
//!   doi:10.1111/j.2041-210x.2012.00261.x

use std::collections::BTreeSet;

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, DataSheet, MixedModelDfMethod, MultiplicityMethod, NestedAnalysisSpec, ResultCell, ResultSheet, ResultTabKind,
    TableArchetype,
};
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::DenseMatrix;

use crate::anova::nested::nested_one_way_anova_with_reml_status;
use crate::anova::posthoc::{
    ContrastFamilyEval, RemlContrastModel, collect_factor_pairs, compare_contrast_family, factorial_design_vector,
    push_pairwise_outputs_with_footnote,
};
use crate::anova::reml::RemlContrastCore;
use crate::anova::results::NestedAnovaResult;
use crate::anova::sheet::one_way::append_reml_boundary_checklist;
use crate::common::{
    TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, ci_cell, init_result_sheet, is_ignored_non_numeric_cell, num_cell,
    p_cell_at_alpha, push_scalar, sanitize_result_sheet_floats, t_crit_two_sided, try_resolve_selected_indices,
};
use crate::error::StatsError;

fn treatment_contrast_vectors(n_levels: usize) -> Vec<Vec<f64>> {
    if n_levels <= 1 {
        return Vec::new();
    }
    let m = n_levels - 1;
    let mut basis = vec![vec![0.0_f64; m]; n_levels];
    for (lvl, row) in basis.iter_mut().enumerate().skip(1) {
        row[lvl - 1] = 1.0;
    }
    basis
}

fn validate_nested_multiplicity(method: MultiplicityMethod) -> Result<(), StatsError> {
    if matches!(
        method,
        MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3 | MultiplicityMethod::DunnRankSum
    ) {
        return Err(StatsError::IncompatibleAnalysisSpec {
            expected: "nested REML parametric post-hoc procedure (None, TukeyHsd, NewmanKeuls, Dunnett, Bonferroni, Sidak, HolmSidak, FisherLsd, LinearTrend, or FDR)",
        });
    }
    Ok(())
}

/// Execute a [`NestedAnalysisSpec`] against a `Nested` [`DataSheet`].
pub fn run_nested_analysis(sheet: &DataSheet, spec: &AnalysisSpec, nested_spec: &NestedAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Nested(nested) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "nested ANOVA / nested t-test", actual: sheet.archetype.kind() });
    };
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let alpha = spec.confidence_level.alpha();

    let selected_groups = try_resolve_selected_indices(&spec.selected_data_sets, nested.groups.len())?;
    let (res, reml_core, reml_converged, reml_at_boundary) =
        nested_one_way_anova_with_reml_status(nested, &selected_groups, nested_spec.two_group_t_test, nested_spec.df_method)?;
    let mut ignored_non_numeric = Vec::new();
    for &g_idx in &selected_groups {
        let group = &nested.groups[g_idx];
        let count = group.cells.iter().flatten().filter(|cell| is_ignored_non_numeric_cell(cell)).count();
        if count > 0 {
            ignored_non_numeric.push((group.title.clone(), count));
        }
    }
    let active_df2 = match nested_spec.df_method {
        MixedModelDfMethod::KenwardRoger => res.df2_groups_kenward_roger,
        MixedModelDfMethod::Satterthwaite => res.df2_groups_satterthwaite,
    };
    push_scalar(&mut out, "F (among groups)", None, res.f_among_groups);
    push_scalar(&mut out, "P value (groups)", None, res.p_among_groups);
    push_scalar(&mut out, "Group denominator df", None, active_df2);
    push_scalar(&mut out, "Subcluster variance", None, res.variance_subclusters);
    push_scalar(&mut out, "Residual variance", None, res.variance_error);
    push_scalar(&mut out, "ICC (subcluster)", None, res.icc_subclusters);
    let two_group_summary = if let Some(t_val) = res.t_nested {
        push_scalar(&mut out, "Nested t", None, t_val);
        let mean_diff = -reml_core.beta.get(1).copied().unwrap_or(0.0) * reml_core.y_scale;
        let cov_mat = match nested_spec.df_method {
            MixedModelDfMethod::KenwardRoger => &reml_core.phi_a,
            MixedModelDfMethod::Satterthwaite => &reml_core.phi,
        };
        let var_diff = cov_mat.get(1, 1).unwrap_or(0.0).max(0.0);
        let se_diff = var_diff.sqrt() * reml_core.y_scale;
        let t_crit = t_crit_two_sided(spec.confidence_level, active_df2)?;
        let ci_lo = mean_diff - t_crit * se_diff;
        let ci_hi = mean_diff + t_crit * se_diff;
        let pct = spec.confidence_level.percent();
        push_scalar(&mut out, "Mean difference", None, mean_diff);
        push_scalar(&mut out, "SE of difference", None, se_diff);
        push_scalar(&mut out, format!("{pct}% CI lower"), None, ci_lo);
        push_scalar(&mut out, format!("{pct}% CI upper"), None, ci_hi);
        Some((t_val, mean_diff, se_diff, ci_lo, ci_hi))
    } else {
        None
    };

    let title = if nested_spec.two_group_t_test { "Nested t test" } else { "Nested one-way ANOVA" };
    out.tabs.push(
        TabBuilder::new("nested-anova", title, ResultTabKind::AnovaTable)
            .headers(["SS", "df", "MS", "F", "P value"])
            .row(
                "Among groups",
                [
                    num_cell(res.ss_groups),
                    num_cell(res.df1_groups),
                    num_cell(res.ms_groups),
                    num_cell(res.f_among_groups),
                    p_cell_at_alpha(res.p_among_groups, p_fmt, alpha),
                ],
            )
            .row(
                "Among subclusters within groups",
                [
                    num_cell(res.ss_subclusters),
                    num_cell(res.df1_subclusters),
                    num_cell(res.ms_subclusters),
                    num_cell(res.f_among_subclusters),
                    p_cell_at_alpha(res.p_among_subclusters, p_fmt, alpha),
                ],
            )
            .row(
                "Error (technical replicates)",
                [num_cell(res.ss_error), num_cell(res.df2_error), num_cell(res.ms_error), ResultCell::Empty, ResultCell::Empty],
            )
            .build(),
    );

    if let Some((t_val, mean_diff, se_diff, ci_lo, ci_hi)) = two_group_summary {
        let pct = spec.confidence_level.percent();
        let contrast_lbl =
            format!("{} - {}", res.group_labels.first().map_or("Group 1", String::as_str), res.group_labels.get(1).map_or("Group 2", String::as_str));
        out.tabs.push(
            TabBuilder::new("nested-t-summary", "Nested t test summary", ResultTabKind::ParametersAndConfidenceIntervals)
                .headers(["Mean difference", "SE of difference", "t", "df", "P value", &format!("{pct}% CI")])
                .row(
                    contrast_lbl,
                    [
                        num_cell(mean_diff),
                        num_cell(se_diff),
                        num_cell(t_val),
                        num_cell(active_df2),
                        p_cell_at_alpha(res.p_among_groups, p_fmt, alpha),
                        ci_cell(ci_lo, ci_hi),
                    ],
                )
                .build(),
        );
    }

    // Variance components tab (scale-invariant percentages).
    let tot_var = res.variance_subclusters + res.variance_error;
    let pct_of_tot = |v: f64| if tot_var > 0.0 { 100.0 * v / tot_var } else { 0.0 };
    let mut vc_tab = TabBuilder::new("variance-components", "Variance components (REML)", ResultTabKind::ParametersAndConfidenceIntervals)
        .headers(["Variance component", "SD", "% of total", "Kenward-Roger df", "Satterthwaite df"])
        .row(
            "Subclusters (biological units)",
            [
                num_cell(res.variance_subclusters),
                num_cell(res.variance_subclusters.sqrt()),
                num_cell(pct_of_tot(res.variance_subclusters)),
                num_cell(res.df2_groups_kenward_roger),
                num_cell(res.df2_groups_satterthwaite),
            ],
        )
        .row(
            "Residual (technical)",
            [
                num_cell(res.variance_error),
                num_cell(res.variance_error.sqrt()),
                num_cell(pct_of_tot(res.variance_error)),
                ResultCell::Empty,
                ResultCell::Empty,
            ],
        );
    if !reml_converged {
        vc_tab.push_footnote("Warning: REML variance-ratio optimization reached the upper search limit without an interior optimum.");
    } else if reml_at_boundary {
        vc_tab.push_footnote("Subcluster variance component estimate is on the zero boundary (sigma^2_subcluster = 0).");
    }
    out.tabs.push(vc_tab.build());

    // Group-level post-hoc contrasts using exact REML GLS means, Phi / Phi_A SEs, and per-contrast KR/Satterthwaite df.
    if !nested_spec.two_group_t_test {
        validate_nested_multiplicity(nested_spec.multiplicity.method)?;
        if selected_groups.len() >= 3 {
            append_nested_multiplicity(&mut out, &selected_groups, &res, reml_core, spec, nested_spec)?;
        }
    }

    let df_lbl = match nested_spec.df_method {
        MixedModelDfMethod::KenwardRoger => "KR",
        MixedModelDfMethod::Satterthwaite => "Satterthwaite",
    };
    out.checklist.push(checklist_item(
        "Biological vs. technical nesting",
        if res.icc_subclusters > 0.05 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("ICC = {:.3}, {df_lbl} df = {:.2}", res.icc_subclusters, active_df2)),
        "Nested REML prevents pseudoreplication by separating biological-unit variance from technical replicate variance.",
    ));
    out.checklist.push(append_reml_boundary_checklist(reml_converged, reml_at_boundary, res.variance_subclusters, res.variance_error));
    append_ignored_text_checklist_and_footnote(&mut out, "nested ANOVA", &ignored_non_numeric);
    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

pub(in crate::anova) fn append_nested_multiplicity(
    out: &mut ResultSheet,
    selected_groups: &[usize],
    res: &NestedAnovaResult,
    reml_core: RemlContrastCore,
    spec: &AnalysisSpec,
    nested_spec: &NestedAnalysisSpec,
) -> Result<(), StatsError> {
    if nested_spec.multiplicity.method == MultiplicityMethod::None {
        return Ok(());
    }
    validate_nested_multiplicity(nested_spec.multiplicity.method)?;
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;
    let k = selected_groups.len();
    let row_proj = DenseMatrix::identity(k)?;
    let observed_cells: BTreeSet<Vec<usize>> = (0..k).map(|g| vec![g]).collect();
    let reml_model = RemlContrastModel {
        factor_levels: vec![k],
        factor_basis: vec![treatment_contrast_vectors(k)],
        subsets: vec![vec![0]],
        total_cols: k,
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

    if nested_spec.multiplicity.method == MultiplicityMethod::LinearTrend {
        let kf = k as f64;
        let center = 0.5 * (kf - 1.0);
        let weights: Vec<f64> = (0..k).map(|j| (j as f64) - center).collect();
        let sum_c2: f64 = weights.iter().map(|&c| c * c).sum();
        let mut wa = vec![0.0_f64; k];
        let mut wb = vec![0.0_f64; k];
        for (j, &c) in weights.iter().enumerate() {
            let dv = factorial_design_vector(reml_model.total_cols, &reml_model.subsets, &reml_model.factor_basis, &[j]);
            let scale = c.abs() / sum_c2.max(1.0);
            let dst = if c >= 0.0 { &mut wa } else { &mut wb };
            for (d, &v) in dst.iter_mut().zip(dv.iter()) {
                *d += scale * v;
            }
        }
        if let Some(est) = reml_model.eval_weight_vecs(&wa, &wb)? {
            let (f_tr, p_tr) = if est.se > 0.0 {
                let f = (est.diff / est.se).powi(2);
                (f, f_sf(f, 1.0, est.df)?)
            } else if est.diff != 0.0 {
                (f64::INFINITY, 0.0)
            } else {
                (0.0, 1.0)
            };
            // Marginal variance explained by the linear trend across REML GLS group means
            // (Nakagawa & Schielzeth, 2013): var_trend / (var_fixed + sigma^2_u + sigma^2_e).
            let mut gls_means = Vec::with_capacity(k);
            for g in 0..k {
                if let Some(m) = reml_model.level_mean_estimate(0, g)? {
                    gls_means.push(m);
                }
            }
            let mean_of_means = if gls_means.is_empty() { 0.0 } else { gls_means.iter().sum::<f64>() / (gls_means.len() as f64) };
            let var_fixed = if gls_means.is_empty() {
                0.0
            } else {
                gls_means.iter().map(|&m| (m - mean_of_means).powi(2)).sum::<f64>() / (gls_means.len() as f64)
            };
            let var_trend = est.diff * est.diff * sum_c2 / kf;
            let var_total = var_fixed + res.variance_subclusters + res.variance_error;
            let r_squared = if var_total > 0.0 { (var_trend / var_total).clamp(0.0, 1.0) } else { 0.0 };

            push_scalar(out, "Linear trend F", None, f_tr);
            push_scalar(out, "Linear trend P", None, p_tr);
            out.tabs.push(
                TabBuilder::new("linear-trend", "Test for linear trend", ResultTabKind::MultipleComparisons)
                    .headers(["Slope", "SE", "DFd", "R squared", "F", "P value"])
                    .row(
                        "Linear trend",
                        [
                            num_cell(est.diff),
                            num_cell(est.se),
                            num_cell(est.df),
                            num_cell(r_squared),
                            num_cell(f_tr),
                            p_cell_at_alpha(p_tr, p_fmt, confidence.alpha()),
                        ],
                    )
                    .footnote("REML GLS linear-trend contrast with per-contrast denominator df; R squared is marginal trend variance divided by total REML variance (Nakagawa & Schielzeth, 2013).")
                    .build(),
            );
        }
        return Ok(());
    }

    let mut mapped_mult = nested_spec.multiplicity.clone();
    match crate::multiplicity::validate_multiplicity_family_spec(&mapped_mult)? {
        crate::multiplicity::MultiplicityFamilySelection::AllPairs => {}
        crate::multiplicity::MultiplicityFamilySelection::ControlVsAll(control_column) => {
            let ctrl_pos = selected_groups
                .iter()
                .position(|&c| c == control_column)
                .ok_or(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })?;
            mapped_mult = mapped_mult.with_control_column(ctrl_pos);
        }
        crate::multiplicity::MultiplicityFamilySelection::SelectedPairs(pairs) => {
            let remapped = crate::multiplicity::resolve_selected_pair_positions(pairs, |c| selected_groups.iter().position(|&g| g == c))?;
            mapped_mult = mapped_mult.with_selected_pairs(remapped);
        }
    }
    let pairs = collect_factor_pairs(k, &mapped_mult, |i, j| reml_model.eval_marginal_pair(0, i, j))?;
    if !pairs.is_empty() {
        let numbered_levels: Vec<(usize, String)> =
            selected_groups.iter().zip(res.group_labels.iter()).map(|(&orig_idx, lbl)| (orig_idx, lbl.clone())).collect();
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
        push_pairwise_outputs_with_footnote(out, &details, "multiple-comparisons", "Nested multiple comparisons", p_fmt, confidence, fam_note);
    }
    Ok(())
}
