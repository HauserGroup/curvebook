//! Standalone Levene / Brown–Forsythe homogeneity-of-variance analysis (`R19.2`).
//!
//! Each selected column of a raw-replicate `Column` or `MultipleVariables` table is one group. The
//! test statistic is the ordinary one-way ANOVA *F* computed on the absolute deviations
//! `z_ij = |x_ij - c_i|`, with `c_i` the group mean (Levene 1960) or the group median
//! (Brown & Forsythe 1974), referred to `F(k - 1, N - k)`.
//!
//! # References
//!
//! - Levene, H. (1960). Robust tests for equality of variances. In I. Olkin et al. (Eds.),
//!   *Contributions to Probability and Statistics: Essays in Honor of Harold Hotelling*, pp. 278–292.
//!   Stanford University Press.
//! - Brown, M. B., & Forsythe, A. B. (1974). Robust tests for the equality of variances.
//!   *Journal of the American Statistical Association* 69(346):364–367. doi:10.1080/01621459.1974.10482955
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 10 (testing
//!   homogeneity of variances among several samples).
//!
//! # Edge cases
//!
//! Every failure is a typed [`StatsError`]; nothing panics.
//!
//! - Tables other than `Column` / `MultipleVariables` → [`StatsError::IncompatibleTable`].
//! - Pre-summarized `Column` subcolumns (mean/SD/N) → [`StatsError::RequiresRawReplicates`], because
//!   absolute deviations need individual replicates.
//! - Fewer than two selected groups → [`StatsError::InsufficientSampleSize`] (`"Levene test"`).
//! - Any group with fewer than two active values → [`StatsError::InsufficientSampleSize`] (`"Levene group"`).
//! - Out-of-range or duplicate `selected_data_sets` → [`StatsError::MissingColumn`] /
//!   [`StatsError::InvalidParameter`].
//! - Blank, excluded, text, `NaN`, and `±Inf` cells are not observations: sheet extraction skips them
//!   (text and non-finite cells are reported in the checklist and a footnote), and the slice API
//!   [`levene_test_groups`] rejects non-finite input with [`StatsError::Math`].
//! - Zero within-group spread of the absolute deviations (for example every group constant, or every
//!   group of size two, whose two deviations are always equal) leaves the *F* ratio undefined and is
//!   rejected with [`StatsError::ZeroVariance`] rather than reporting `F = 0` or `F = +Inf`. The
//!   threshold is the rounding-noise floor `N * (8 * eps * max |x|)^2`.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistStatus, DataSheet, LeveneCenter, LeveneTestSpec, ResultSheet, ResultTabKind, TableArchetype,
};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};

use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, checklist_item, enforce_summary_mode_matrix, extract_numeric_columns,
    init_result_sheet, num_cell, push_scalar, sanitize_result_sheet_floats,
};
use crate::error::StatsError;
use crate::two_group::{LeveneTestResult, levene_variance_core};

/// Per-group summary reported alongside a standalone Levene / Brown–Forsythe test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeveneGroupSummary {
    /// Zero-based source column (data set or variable) index.
    pub index: usize,
    /// Source column title.
    pub label: String,
    /// Number of active finite observations in the group.
    pub n: usize,
    /// Group centre `c_i` (median for Brown–Forsythe, mean for Levene).
    pub center: f64,
    /// Mean absolute deviation `mean_j |x_ij - c_i|` (the group mean entering the ANOVA on deviations).
    pub mean_abs_deviation: f64,
    /// Sample standard deviation of the raw group values (`n - 1` denominator).
    pub sd: f64,
}

/// Complete output of a standalone Levene / Brown–Forsythe homogeneity-of-variance test.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeveneAnalysisResult {
    /// Deviation centre used.
    pub center: LeveneCenter,
    /// Omnibus *F*, `df_between = k - 1`, `df_within = N - k`, and upper-tail *P* value.
    pub test: LeveneTestResult,
    /// Per-group summaries in input order.
    pub groups: Vec<LeveneGroupSummary>,
}

/// Run a Levene (`LeveneCenter::Mean`) or Brown–Forsythe (`LeveneCenter::Median`) test over labeled
/// groups `(index, label, values)`.
///
/// See the module documentation for the formula and the edge-case contract (fewer than two groups,
/// a group with `n < 2`, non-finite values, and zero spread of the absolute deviations all return
/// typed errors).
pub fn levene_test_groups(groups: &[(usize, &str, &[f64])], center: LeveneCenter) -> Result<LeveneAnalysisResult, StatsError> {
    if groups.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "Levene test", required: 2, actual: groups.len() });
    }
    let slices: Vec<&[f64]> = groups.iter().map(|(_, _, v)| *v).collect();
    let core = levene_variance_core(&slices, matches!(center, LeveneCenter::Median))?;
    if core.ss_within <= core.ss_within_noise_floor {
        return Err(StatsError::ZeroVariance { context: "Levene test absolute deviations (every group has identical |x - center|)" });
    }
    let mut summaries = Vec::with_capacity(groups.len());
    for ((&(index, label, values), &center_val), &mad) in groups.iter().zip(core.centers.iter()).zip(core.mean_abs_deviations.iter()) {
        let sd = SummaryMoments::from_slice(values)?.sample_sd()?;
        summaries.push(LeveneGroupSummary { index, label: label.to_owned(), n: values.len(), center: center_val, mean_abs_deviation: mad, sd });
    }
    Ok(LeveneAnalysisResult { center, test: core.test, groups: summaries })
}

/// Execute a [`LeveneTestSpec`] against a raw-replicate `Column` or `MultipleVariables` [`DataSheet`],
/// treating each selected column as one group.
pub fn run_levene_analysis(sheet: &DataSheet, spec: &AnalysisSpec, lt: &LeveneTestSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::LeveneTest(*lt))?;
    if !matches!(sheet.archetype, TableArchetype::Column(_) | TableArchetype::MultipleVariables(_)) {
        return Err(StatsError::IncompatibleTable { analysis: "Levene / Brown-Forsythe test", actual: sheet.archetype.kind() });
    }
    let cols = extract_numeric_columns(sheet, &spec.selected_data_sets)?;
    let ignored: Vec<(String, usize)> = cols.iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect();
    let excluded_total: usize = cols.iter().map(|c| c.excluded_count).sum();
    let groups: Vec<(usize, &str, &[f64])> = cols.iter().map(|c| (c.index, c.label.as_str(), c.values.as_slice())).collect();
    let out = levene_test_groups(&groups, lt.center)?;

    let mut res = init_result_sheet(spec)?;
    let alpha = spec.confidence_level.alpha();
    let p_fmt = spec.p_value_format;
    let k = out.groups.len();
    let total_n: usize = out.groups.iter().map(|g| g.n).sum();
    let center_header = match out.center {
        LeveneCenter::Mean => "Mean (center)",
        LeveneCenter::Median => "Median (center)",
    };
    let reject = out.test.p_value <= alpha;
    let conclusion = if reject { "Variances differ" } else { "Equal variances not rejected" };

    res.tabs.push(
        KvTabBuilder::new("levene-summary", "Homogeneity of variances", ResultTabKind::MainSummary, format!("{k} groups"))
            .text("Method", out.center.method_label())
            .text("Deviations from group", out.center.center_name())
            .num("Number of groups", k as f64)
            .num("Total N", total_n as f64)
            .num("F", out.test.f_statistic)
            .num("DFn", out.test.df_between)
            .num("DFd", out.test.df_within)
            .p("P value", out.test.p_value, p_fmt, alpha)
            .text(format!("Significantly different variances (alpha = {alpha})?"), if reject { "Yes" } else { "No" })
            .text("Conclusion", conclusion)
            .build(),
    );

    let mut group_tab = TabBuilder::new("levene-groups", "Group centers and absolute deviations", ResultTabKind::DescriptiveSummary).headers([
        "N",
        center_header,
        "Mean |deviation|",
        "SD",
    ]);
    for g in &out.groups {
        group_tab.push_row(g.label.clone(), [num_cell(g.n as f64), num_cell(g.center), num_cell(g.mean_abs_deviation), num_cell(g.sd)]);
    }
    group_tab.push_footnote(format!(
        "Absolute deviations |x - {}| per group; F is the one-way ANOVA F on these deviations with DFn = k - 1 and DFd = N - k.",
        out.center.center_name()
    ));
    res.tabs.push(group_tab.build());

    push_scalar(&mut res, "F", None, out.test.f_statistic);
    push_scalar(&mut res, "DFn", None, out.test.df_between);
    push_scalar(&mut res, "DFd", None, out.test.df_within);
    push_scalar(&mut res, "P value", None, out.test.p_value);
    push_scalar(&mut res, "Groups", None, k as f64);
    push_scalar(&mut res, "Total N", None, total_n as f64);
    for g in &out.groups {
        push_scalar(&mut res, "N", Some(g.index), g.n as f64);
        push_scalar(&mut res, "Center", Some(g.index), g.center);
        push_scalar(&mut res, "Mean abs deviation", Some(g.index), g.mean_abs_deviation);
    }

    res.checklist.push(checklist_item(
        "Homogeneity of variances",
        if reject { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
        Some(format!("F({}, {}) = {:.4}, P = {:.4}", out.test.df_between, out.test.df_within, out.test.f_statistic, out.test.p_value)),
        if reject {
            "The group variances differ at the chosen alpha; prefer methods that do not assume equal variances (for example Welch or Brown-Forsythe ANOVA)."
        } else {
            "No evidence that the group variances differ at the chosen alpha. A non-significant result does not prove the variances are equal."
        },
    ));
    let min_n = out.groups.iter().map(|g| g.n).min().unwrap_or(0);
    if min_n < 5 {
        res.checklist.push(checklist_item(
            "Group sample size",
            ChecklistStatus::Warn,
            Some(format!("smallest group n = {min_n}")),
            "With very small groups the test has little power to detect unequal variances.",
        ));
    }
    if matches!(out.center, LeveneCenter::Mean) {
        res.checklist.push(checklist_item(
            "Choice of center",
            ChecklistStatus::Info,
            None,
            "Mean-centered Levene deviations are sensitive to skewed or heavy-tailed data; the median-centered Brown-Forsythe variant is the robust default.",
        ));
    }
    if excluded_total > 0 {
        res.checklist.push(checklist_item(
            "Excluded values",
            ChecklistStatus::Info,
            Some(format!("{excluded_total} excluded cell(s)")),
            "Excluded cells were left out of every group.",
        ));
    }
    append_ignored_text_checklist_and_footnote(&mut res, "Levene / Brown-Forsythe test", &ignored);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}
