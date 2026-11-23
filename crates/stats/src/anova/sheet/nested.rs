//! `ResultSheet` builder for nested (hierarchical) comparison specs.
//!
//! References:
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, DataSheet, MixedModelDfMethod, MultiplicityMethod, NestedAnalysisSpec, ResultCell, ResultSheet, ResultTabKind,
    TableArchetype,
};
use curvebook_math::sum::SummaryMoments;

use crate::anova::nested::nested_one_way_anova_with_reml_status;
use crate::anova::results::NestedAnovaResult;
use crate::anova::sheet::one_way::{append_reml_boundary_checklist, push_pairwise_outputs};
use crate::common::{
    TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, init_result_sheet, is_ignored_non_numeric_cell, num_cell,
    p_cell_at_alpha, push_scalar, sanitize_result_sheet_floats, try_resolve_selected_indices,
};
use crate::error::StatsError;
use crate::multiplicity::{GroupComparisonInput, MultiplicityProcedure, compare_groups_pairwise_with_confidence, linear_trend_test};

/// Execute a [`NestedAnalysisSpec`] against a `Nested` [`DataSheet`].
pub fn run_nested_analysis(sheet: &DataSheet, spec: &AnalysisSpec, nested_spec: &NestedAnalysisSpec) -> Result<ResultSheet, StatsError> {
    let TableArchetype::Nested(nested) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "nested ANOVA / nested t-test", actual: sheet.archetype.kind() });
    };
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let alpha = spec.confidence_level.alpha();

    let selected_groups = try_resolve_selected_indices(&spec.selected_data_sets, nested.groups.len())?;
    let (res, reml_converged, reml_at_boundary) =
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
    if let Some(t_val) = res.t_nested {
        push_scalar(&mut out, "Nested t", None, t_val);
    }

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

    // Variance components tab.
    let tot_var = (res.variance_subclusters + res.variance_error).max(1e-18);
    let mut vc_tab = TabBuilder::new("variance-components", "Variance components (REML)", ResultTabKind::ParametersAndConfidenceIntervals)
        .headers(["Variance component", "SD", "% of total", "Kenward-Roger df", "Satterthwaite df"])
        .row(
            "Subclusters (biological units)",
            [
                num_cell(res.variance_subclusters),
                num_cell(res.variance_subclusters.sqrt()),
                num_cell(100.0 * res.variance_subclusters / tot_var),
                num_cell(res.df2_groups_kenward_roger),
                num_cell(res.df2_groups_satterthwaite),
            ],
        )
        .row(
            "Residual (technical)",
            [
                num_cell(res.variance_error),
                num_cell(res.variance_error.sqrt()),
                num_cell(100.0 * res.variance_error / tot_var),
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

    // Optional group-level pairwise comparisons using hierarchical group-mean variances:
    // Var(mean_g) = sigma_u^2 * sum_j(n_{gj}^2) / N_g^2 + sigma_e^2 / N_g (Sokal & Rohlf / Montgomery).
    if !nested_spec.two_group_t_test && selected_groups.len() >= 3 {
        append_nested_multiplicity(&mut out, nested, &selected_groups, &res, active_df2, spec, nested_spec)?;
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
    nested: &curvebook_doc::NestedTable,
    selected_groups: &[usize],
    res: &NestedAnovaResult,
    active_df2: f64,
    spec: &AnalysisSpec,
    nested_spec: &NestedAnalysisSpec,
) -> Result<(), StatsError> {
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;
    let is_welch = matches!(
        MultiplicityProcedure::from(nested_spec.multiplicity.method),
        MultiplicityProcedure::GamesHowell | MultiplicityProcedure::TamhaneT2 | MultiplicityProcedure::DunnettT3
    );
    let mut group_stats = Vec::with_capacity(selected_groups.len());
    for &g_idx in selected_groups {
        let g = nested.groups.get(g_idx).ok_or(StatsError::MissingColumn { index: g_idx })?;
        let mut vals = Vec::new();
        let mut m_g = 0_usize;
        let mut sum_n_sq = 0.0_f64;
        for b in 0..g.biological_unit_count() {
            let reps = nested.active_technical_replicates(g_idx, b);
            if !reps.is_empty() {
                m_g += 1;
                let n_b = reps.len() as f64;
                sum_n_sq += n_b * n_b;
                vals.extend(reps);
            }
        }
        let m = SummaryMoments::from_slice(&vals)?;
        let total_n_g = vals.len() as f64;
        let var_mean_g = res.variance_subclusters * (sum_n_sq / (total_n_g * total_n_g)) + res.variance_error / total_n_g;
        group_stats.push((g_idx, g.title.clone(), m.mean, m_g as f64, var_mean_g.max(0.0)));
    }
    let all_pos = group_stats.iter().all(|(_, _, _, _, v)| *v > 0.0);
    let mse_eff = if is_welch || !all_pos { 0.0 } else { 1.0 };
    let mut g_inputs = Vec::with_capacity(group_stats.len());
    for (g_idx, label, mean, m_g, var_mean_g) in group_stats {
        let n_eff = if !is_welch && all_pos { 1.0 / var_mean_g } else { m_g.max(1.0) };
        let sd_eff = (m_g.max(1.0) * var_mean_g).sqrt();
        g_inputs.push(GroupComparisonInput { index: g_idx, label, n: n_eff, mean, sd: sd_eff });
    }
    if nested_spec.multiplicity.method == MultiplicityMethod::LinearTrend {
        let trend = linear_trend_test(&g_inputs, mse_eff, active_df2)?;
        push_scalar(out, "Linear trend F", None, trend.f_statistic);
        push_scalar(out, "Linear trend P", None, trend.p_value);
        out.tabs.push(
            TabBuilder::new("linear-trend", "Test for linear trend", ResultTabKind::MultipleComparisons)
                .headers(["Slope", "R squared", "F", "P value"])
                .row(
                    "Linear trend",
                    [
                        num_cell(trend.slope_estimate),
                        num_cell(trend.r_squared),
                        num_cell(trend.f_statistic),
                        p_cell_at_alpha(trend.p_value, p_fmt, confidence.alpha()),
                    ],
                )
                .build(),
        );
    } else {
        let details = compare_groups_pairwise_with_confidence(&g_inputs, mse_eff, active_df2, &nested_spec.multiplicity, confidence)?;
        push_pairwise_outputs(out, &details, "Nested multiple comparisons", p_fmt, confidence);
    }
    Ok(())
}
