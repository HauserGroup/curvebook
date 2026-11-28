//! `ResultSheet` builders for two-group comparisons, Bland–Altman method comparison, and
//! multiple row-wise *t*-tests.
//!
//! # References
//!
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall. Ch. 7–9.
//! - Student (1908). The probable error of a mean. *Biometrika* 6(1):1–25. doi:10.1093/biomet/6.1.1
//! - Welch, B. L. (1947). The generalization of 'Student's' problem when several different population
//!   variances are involved. *Biometrika* 34(1–2):28–35. doi:10.1093/biomet/34.1-2.28
//! - Wilcoxon, F. (1945). Individual comparisons by ranking methods. *Biometrics Bulletin* 1(6):80–83.
//!   doi:10.2307/3001968
//! - Mann, H. B., & Whitney, D. R. (1947). On a test of whether one of two random variables is
//!   stochastically larger than the other. *Annals of Mathematical Statistics* 18(1):50–60.
//!   doi:10.1214/aoms/1177730491
//! - Pratt, J. W. (1959). Remarks on zeros and ties in the Wilcoxon signed rank procedures.
//!   *Journal of the American Statistical Association* 54(287):655–667. doi:10.1080/01621459.1959.10501526
//! - Bland, J. M., & Altman, D. G. (1986). Statistical methods for assessing agreement between two
//!   methods of clinical measurement. *Lancet* 327(8476):307–310. doi:10.1016/S0140-6736(86)90837-8
//! - Bland, J. M., & Altman, D. G. (1999). Measuring agreement in method comparison studies.
//!   *Statistical Methods in Medical Research* 8(2):135–160. doi:10.1177/096228029900800204

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, CellValue, ChecklistItem, ChecklistStatus, CurveSamplePoint, DataSheet, FittedCurveOverlay,
    LognormalRowTestMode, MultipleRowTestsSpec, MultipleVariablesTable, ObservationAccounting, ObservationAccountingKind, PValueFormat,
    PairwiseComparisonRecord, ResultCell, ResultSheet, ResultTabKind, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole, XColumnMode,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::sum::SummaryMoments;

use super::bland_altman::bland_altman;
use super::multiple_row::{MultipleRowTestEntry, validate_raw_p_multiplicity_procedure};
use super::nonparametric::{
    MannWhitneyResult, PairedSignTestResult, TwoSampleKsResult, WilcoxonMatchedResult, ks_two_sample, mann_whitney_max_achievable_coverage_percent,
    mann_whitney_u, paired_sign_test, wilcoxon_matched_pairs,
};
use super::parametric::{
    PairedTResult, UnpairedTResult, lognormal_unpaired_t_test, one_sample_t_test_from_summary, paired_t_test, ratio_paired_t_test, unpaired_t_test,
    unpaired_t_test_from_summary, welch_t_test, welch_t_test_from_summary,
};
use crate::column::{OneSampleTResult, shapiro_wilk, summarize_pre_summarized_points, wilcoxon_max_achievable_coverage_percent};
use crate::common::{
    KvTabBuilder, TabBuilder, append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, checklist_item,
    enforce_summary_mode_matrix, extract_numeric_columns, extract_paired_values_with_report, init_result_sheet, is_ignored_non_numeric_cell,
    num_cell, opt_ci_cell, opt_num_cell, p_cell_at_alpha, push_scalar, record_omitted_row_reason, sanitize_result_sheet_floats,
    try_resolve_selected_indices, unpaired_observation_accounting,
};
use crate::error::StatsError;
use crate::lognormal::{OneSampleRatioTResult, lognormal_welch_t_test, one_sample_ratio_t_test, validate_and_ln_transform};
use crate::multiplicity::{MultiplicityProcedure, adjust_p_values};

#[derive(Clone, Copy)]
struct GroupPairMeta<'a> {
    idx_a: usize,
    idx_b: usize,
    lbl_a: &'a str,
    lbl_b: &'a str,
}

#[derive(Clone, Copy)]
struct SheetFormatCtx {
    conf: ConfidenceLevel,
    p_fmt: PValueFormat,
}

fn extract_pre_summarized_group(
    col_tab: &curvebook_doc::ColumnTable,
    col_idx: usize,
    conf: ConfidenceLevel,
) -> Result<(String, f64, f64, f64), StatsError> {
    let ds = col_tab.data_sets.get(col_idx).ok_or(StatsError::MissingColumn { index: col_idx })?;
    let mut pts = Vec::new();
    for r in 0..ds.row_count() {
        if let Some(pt) = col_tab.summarized_point(r, col_idx)? {
            pts.push(pt);
        }
    }
    if pts.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let pooled = summarize_pre_summarized_points(&ds.title, &pts, conf)?;
    if pooled.total_n < 2.0 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Err(StatsError::InsufficientSampleSize {
            context: "pre-summarized t-test group",
            required: 2,
            actual: pooled.total_n.max(0.0) as usize,
        });
    }
    let sd = pooled.sd.ok_or(StatsError::InsufficientSampleSize { context: "pre-summarized t-test SD", required: 2, actual: 1 })?;
    Ok((pooled.label, pooled.mean, sd, pooled.total_n))
}

/// Execute [`TwoGroupSpec`] on a [`DataSheet`] and emit a complete [`ResultSheet`].
pub fn run_two_group_analysis(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    let mut res = run_two_group_analysis_inner(sheet, spec, tg)?;
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

fn run_two_group_analysis_inner(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::TwoGroupComparison(*tg))?;
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let fmt = SheetFormatCtx { conf, p_fmt };

    if let TableArchetype::Column(col_tab) = &sheet.archetype
        && col_tab.subcolumn_mode.is_pre_summarized()
    {
        match tg.method {
            TwoGroupMethod::OneSampleT { hypothetical } => {
                let indices = try_resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len())?;
                if !spec.selected_data_sets.is_empty() && indices.len() != 1 {
                    return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
                }
                let &first_idx = indices.first().ok_or(StatsError::EmptyInput)?;
                let (label, mean, sd, n) = extract_pre_summarized_group(col_tab, first_idx, conf)?;
                let out = one_sample_t_test_from_summary(mean, sd, n, hypothetical, tg.two_tailed, conf)?;
                populate_one_sample_t_sheet_impl(&mut res, &label, None, &out, fmt);
                return Ok(res);
            }
            TwoGroupMethod::UnpairedT | TwoGroupMethod::WelchT => {
                let indices = try_resolve_selected_indices(&spec.selected_data_sets, col_tab.data_sets.len())?;
                if !spec.selected_data_sets.is_empty() && indices.len() != 2 {
                    return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
                }
                if indices.len() < 2 {
                    return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
                }
                let (idx_a, idx_b) = (indices[0], indices[1]);
                let (lbl_a, mean_a, sd_a, n_a) = extract_pre_summarized_group(col_tab, idx_a, conf)?;
                let (lbl_b, mean_b, sd_b, n_b) = extract_pre_summarized_group(col_tab, idx_b, conf)?;
                let out = if matches!(tg.method, TwoGroupMethod::WelchT) {
                    welch_t_test_from_summary((mean_a, sd_a, n_a), (mean_b, sd_b, n_b), tg.two_tailed, conf)?
                } else {
                    unpaired_t_test_from_summary((mean_a, sd_a, n_a), (mean_b, sd_b, n_b), tg.two_tailed, conf)?
                };
                let pair = GroupPairMeta { idx_a, idx_b, lbl_a: &lbl_a, lbl_b: &lbl_b };
                populate_unpaired_t_sheet_impl(&mut res, pair, &out, tg.compute_effect_sizes, fmt, false);
                return Ok(res);
            }
            _ => return Err(StatsError::RequiresRawReplicates),
        }
    }

    match tg.method {
        TwoGroupMethod::OneSampleT { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_t_test(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_t_sheet_impl(&mut res, &first.label, Some(&first.values), &out, fmt);
            res.observation_accounting = Some(unpaired_observation_accounting(first.values.len(), first.non_numeric_count, first.excluded_count));
            if first.non_numeric_count > 0 {
                append_ignored_text_checklist_and_footnote(&mut res, "one-sample t test", &[(first.label.clone(), first.non_numeric_count)]);
            }
            return Ok(res);
        }
        TwoGroupMethod::OneSampleWilcoxon { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = crate::column::one_sample_wilcoxon(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_w_sheet(&mut res, &first.label, &out, fmt);
            res.observation_accounting = Some(unpaired_observation_accounting(first.values.len(), first.non_numeric_count, first.excluded_count));
            if first.non_numeric_count > 0 {
                append_ignored_text_checklist_and_footnote(&mut res, "one-sample Wilcoxon test", &[(first.label.clone(), first.non_numeric_count)]);
            }
            return Ok(res);
        }
        TwoGroupMethod::OneSampleRatioT { hypothetical } => {
            let sel = resolve_mv_one_sample_selection(sheet, &spec.selected_data_sets, tg.mv_response_column)?;
            let cols = extract_numeric_columns(sheet, &sel)?;
            let first = cols.first().ok_or(StatsError::EmptyInput)?;
            let out = one_sample_ratio_t_test(&first.values, hypothetical, tg.two_tailed, conf)?;
            populate_one_sample_ratio_t_sheet(&mut res, &first.label, &first.values, &out, fmt);
            res.observation_accounting = Some(unpaired_observation_accounting(first.values.len(), first.non_numeric_count, first.excluded_count));
            if first.non_numeric_count > 0 {
                append_ignored_text_checklist_and_footnote(&mut res, "one-sample ratio t test", &[(first.label.clone(), first.non_numeric_count)]);
            }
            return Ok(res);
        }
        _ => {}
    }

    if let Some(mv_in) = resolve_mv_two_group_inputs(sheet, spec, tg)? {
        let MvTwoGroupExtraction { group_a: lvl_a, group_b: lvl_b, skipped_missing, ignored_non_numeric, accounting } = mv_in;
        if matches!(
            tg.method,
            TwoGroupMethod::PairedT | TwoGroupMethod::RatioPairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest
        ) {
            return Err(StatsError::InvalidParameter { name: "paired_method_on_mv_categorical_grouping", value: 0.0 });
        }
        let pair = GroupPairMeta { idx_a: 0, idx_b: 1, lbl_a: &lvl_a.label, lbl_b: &lvl_b.label };
        match tg.method {
            TwoGroupMethod::UnpairedT => {
                let out = unpaired_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, false, conf.alpha());
            }
            TwoGroupMethod::WelchT => {
                let out = welch_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, false, conf.alpha());
            }
            TwoGroupMethod::LognormalUnpairedT => {
                let out = lognormal_unpaired_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, true, conf.alpha());
            }
            TwoGroupMethod::MannWhitneyU => {
                let out = mann_whitney_u(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_mann_whitney_sheet(&mut res, pair, &out, fmt);
            }
            TwoGroupMethod::KolmogorovSmirnov => {
                let out = ks_two_sample(&lvl_a.values, &lvl_b.values)?;
                populate_ks_sheet(&mut res, pair, &out, fmt);
            }
            TwoGroupMethod::LognormalWelchT => {
                let out = lognormal_welch_t_test(&lvl_a.values, &lvl_b.values, tg.two_tailed, conf)?;
                populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
                append_unpaired_normality_checklist(&mut res.checklist, &lvl_a.values, &lvl_b.values, true, conf.alpha());
            }
            TwoGroupMethod::PairedT
            | TwoGroupMethod::RatioPairedT
            | TwoGroupMethod::WilcoxonSignedRank
            | TwoGroupMethod::PairedSignTest
            | TwoGroupMethod::OneSampleT { .. }
            | TwoGroupMethod::OneSampleWilcoxon { .. }
            | TwoGroupMethod::OneSampleRatioT { .. } => {}
        }
        res.observation_accounting = Some(accounting);
        append_listwise_filtered_checklist_and_footnote(&mut res, "Two-group comparison", skipped_missing, lvl_a.values.len() + lvl_b.values.len());
        append_ignored_text_checklist_and_footnote(&mut res, "two-group comparison", &ignored_non_numeric);
        return Ok(res);
    }

    let cols = extract_two_group_wide_columns(sheet, &spec.selected_data_sets)?;
    if cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: cols.len() });
    }
    let (ca, cb) = (&cols[0], &cols[1]);
    let pair = GroupPairMeta { idx_a: ca.index, idx_b: cb.index, lbl_a: &ca.label, lbl_b: &cb.label };
    let unpaired_ignored = || -> Vec<(String, usize)> {
        [ca, cb].into_iter().filter(|c| c.non_numeric_count > 0).map(|c| (c.label.clone(), c.non_numeric_count)).collect()
    };
    let unpaired_accounting = || {
        unpaired_observation_accounting(
            ca.values.len().saturating_add(cb.values.len()),
            ca.non_numeric_count.saturating_add(cb.non_numeric_count),
            ca.excluded_count.saturating_add(cb.excluded_count),
        )
    };

    match tg.method {
        TwoGroupMethod::UnpairedT => {
            let out = unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, false, conf.alpha());
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "unpaired t test", &unpaired_ignored());
        }
        TwoGroupMethod::WelchT => {
            let out = welch_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, false, conf.alpha());
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "Welch t test", &unpaired_ignored());
        }
        TwoGroupMethod::LognormalUnpairedT => {
            let out = lognormal_unpaired_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, true, conf.alpha());
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "lognormal unpaired t test", &unpaired_ignored());
        }
        TwoGroupMethod::PairedT => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = paired_t_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_paired_normality_checklist(&mut res.checklist, &report.xs, &report.ys, false, conf.alpha());
            report_paired_extraction(&mut res, "Paired t test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::RatioPairedT => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = ratio_paired_t_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_paired_normality_checklist(&mut res.checklist, &report.xs, &report.ys, true, conf.alpha());
            report_paired_extraction(&mut res, "Ratio paired t test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::MannWhitneyU => {
            let out = mann_whitney_u(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_mann_whitney_sheet(&mut res, pair, &out, fmt);
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "Mann-Whitney test", &unpaired_ignored());
        }
        TwoGroupMethod::WilcoxonSignedRank => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = wilcoxon_matched_pairs(&report.xs, &report.ys, tg.two_tailed, conf, true)?;
            populate_wilcoxon_paired_sheet(&mut res, pair, &out, fmt);
            report_paired_extraction(&mut res, "Wilcoxon signed-rank test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::PairedSignTest => {
            let report = extract_paired_values_with_report(sheet, ca.index, cb.index)?;
            let out = paired_sign_test(&report.xs, &report.ys, tg.two_tailed, conf)?;
            populate_paired_sign_sheet(&mut res, pair, &out, fmt);
            report_paired_extraction(&mut res, "Paired sign test", &ca.label, &cb.label, &report);
        }
        TwoGroupMethod::KolmogorovSmirnov => {
            let out = ks_two_sample(&ca.values, &cb.values)?;
            populate_ks_sheet(&mut res, pair, &out, fmt);
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "Kolmogorov-Smirnov test", &unpaired_ignored());
        }
        TwoGroupMethod::LognormalWelchT => {
            let out = lognormal_welch_t_test(&ca.values, &cb.values, tg.two_tailed, conf)?;
            populate_unpaired_t_sheet(&mut res, pair, &out, tg.compute_effect_sizes, fmt);
            append_unpaired_normality_checklist(&mut res.checklist, &ca.values, &cb.values, true, conf.alpha());
            res.observation_accounting = Some(unpaired_accounting());
            append_ignored_text_checklist_and_footnote(&mut res, "lognormal Welch t test", &unpaired_ignored());
        }
        TwoGroupMethod::OneSampleT { .. } | TwoGroupMethod::OneSampleWilcoxon { .. } | TwoGroupMethod::OneSampleRatioT { .. } => {}
    }
    Ok(res)
}

fn report_paired_extraction(res: &mut ResultSheet, analysis: &str, lbl_a: &str, lbl_b: &str, report: &crate::common::PairedValuesReport) {
    report.accounting.push_scalars(res);
    res.observation_accounting = Some(ObservationAccounting::from_paired(report.accounting, report.non_numeric_cells, report.excluded_cells));
    append_listwise_filtered_checklist_and_footnote(res, analysis, report.skipped_incomplete_rows, report.xs.len());
    if report.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(res, analysis, &[(format!("{lbl_a} / {lbl_b}"), report.non_numeric_cells)]);
    }
}

/// Execute [`BlandAltmanSpec`] on a `Column`, `MultipleVariables`, or `Xy` [`DataSheet`] and emit a
/// complete [`ResultSheet`] (`MainSummary`, pair coordinate table, `scalars`, and overlay curve).
pub fn run_bland_altman_analysis(sheet: &DataSheet, spec: &AnalysisSpec, ba: &BlandAltmanSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::BlandAltman(*ba))?;
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let ba_report = extract_bland_altman_pairs(sheet, ba)?;
    let (lbl_a, lbl_b, vals_a, vals_b) = (ba_report.label_a, ba_report.label_b, ba_report.xs, ba_report.ys);
    let out = bland_altman(&vals_a, &vals_b, ba.percent_difference, conf)?;

    let mode_text = if out.percent_difference { "Percent difference (100 * (A - B) / Mean)" } else { "Absolute difference (A - B)" };
    let pct = conf.percent();
    res.tabs.push(
        KvTabBuilder::new("bland-altman-summary", "Bland-Altman method comparison", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
            .text("Difference mode", mode_text)
            .num("N pairs", out.n as f64)
            .num("Bias", out.bias)
            .num("SD of bias", out.sd_bias)
            .num("SE of bias", out.se_bias)
            .ci(format!("{pct}% CI of bias"), out.bias_ci.0, out.bias_ci.1)
            .num(format!("Lower {pct}% LoA"), out.loa_lower)
            .ci(format!("{pct}% CI of lower LoA"), out.loa_lower_ci.0, out.loa_lower_ci.1)
            .num(format!("Upper {pct}% LoA"), out.loa_upper)
            .ci(format!("{pct}% CI of upper LoA"), out.loa_upper_ci.0, out.loa_upper_ci.1)
            .build(),
    );

    let diff_header = if out.percent_difference { "Percent Difference (%)" } else { "Difference (A - B)" };
    let mut coord_tab = TabBuilder::new("bland-altman-coordinates", "Bland-Altman Pair Coordinates", ResultTabKind::Residuals).headers([
        String::from("Average (A + B) / 2"),
        String::from(diff_header),
        lbl_a.clone(),
        lbl_b.clone(),
    ]);
    let mut overlay_pts = Vec::with_capacity(out.n);
    for (idx, ((&(avg, diff), &a), &b)) in out.points.iter().zip(vals_a.iter()).zip(vals_b.iter()).enumerate() {
        coord_tab.push_row(format!("Pair {}", idx + 1), [num_cell(avg), num_cell(diff), num_cell(a), num_cell(b)]);
        overlay_pts.push(CurveSamplePoint {
            x: avg,
            y: diff,
            ci_lower: Some(out.loa_lower),
            ci_upper: Some(out.loa_upper),
            pred_lower: None,
            pred_upper: None,
        });
    }
    res.tabs.push(coord_tab.build());
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: ba.column_a,
        equation_id: String::from("BlandAltman"),
        converged: true,
        ambiguous: false,
        points: overlay_pts,
    });

    push_scalar(&mut res, "N", None, out.n as f64);
    push_scalar(&mut res, "Bias", None, out.bias);
    push_scalar(&mut res, "SDBias", None, out.sd_bias);
    push_scalar(&mut res, "SEBias", None, out.se_bias);
    push_scalar(&mut res, "LoALower", None, out.loa_lower);
    push_scalar(&mut res, "LoAUpper", None, out.loa_upper);
    res.checklist.push(checklist_item(
        "Bland-Altman sample size & normality of differences",
        if out.n >= 10 { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("n = {} pairs", out.n)),
        "Limits of Agreement assume differences follow an approximately normal distribution with constant variance across the measurement range.",
    ));
    ba_report.accounting.push_scalars(&mut res);
    res.observation_accounting =
        Some(ObservationAccounting::from_paired(ba_report.accounting, ba_report.non_numeric_cells, ba_report.excluded_cells));
    append_listwise_filtered_checklist_and_footnote(&mut res, "Bland-Altman agreement", ba_report.skipped_incomplete_rows, out.n);
    if ba_report.non_numeric_cells > 0 {
        append_ignored_text_checklist_and_footnote(
            &mut res,
            "Bland-Altman agreement",
            &[(format!("{lbl_a} / {lbl_b}"), ba_report.non_numeric_cells)],
        );
    }
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

/// Execute [`MultipleRowTestsSpec`] row-by-row across two groups and emit a complete [`ResultSheet`]
/// (`MainSummary`, `MultipleComparisons`, `scalars`, and `PairwiseComparisonRecord`s for Volcano plots).
pub fn run_multiple_row_t_tests_analysis(sheet: &DataSheet, spec: &AnalysisSpec, mr: &MultipleRowTestsSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::MultipleRowTTests(mr.clone()))?;
    if mr.multiplicity.selected_pairs.is_some() {
        return Err(StatsError::IncompatibleAnalysisSpec {
            expected: "multiple-row t-tests compare rows across two groups and do not support selected_pairs",
        });
    }
    let mut res = init_result_sheet(spec)?;
    let conf = spec.confidence_level;
    let p_fmt = spec.p_value_format;
    let threshold = mr.multiplicity.threshold;
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "multiplicity_threshold", value: threshold });
    }
    let proc_eff = MultiplicityProcedure::from(mr.multiplicity.method);
    validate_raw_p_multiplicity_procedure(proc_eff)?;

    let effective_paired = mr.paired || mr.lognormal_mode == LognormalRowTestMode::RatioPaired;
    let is_lognormal_mode = mr.lognormal_mode != LognormalRowTestMode::None;
    let inputs = extract_multiple_row_inputs(sheet, spec, effective_paired)?;
    let (lbl_a, lbl_b, row_labels, group_a_rows, group_b_rows) = (inputs.label_a, inputs.label_b, inputs.row_labels, inputs.rows_a, inputs.rows_b);
    let mut raw_p = Vec::new();
    let mut staged = Vec::new();
    let mut skipped_rows = 0_usize;

    for (idx, (ra, rb)) in group_a_rows.iter().zip(group_b_rows.iter()).enumerate() {
        let row_lbl = row_labels.get(idx).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", idx + 1));
        let row_out = run_single_row_comparison(ra, rb, mr, conf);
        match row_out {
            Ok(outcome) if outcome.p_val.is_finite() => {
                raw_p.push(outcome.p_val);
                staged.push((idx, row_lbl, outcome));
            }
            Ok(_) | Err(_) => {
                skipped_rows += 1;
            }
        }
    }

    if staged.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let (adj_p, is_fdr) = adjust_p_values(&raw_p, proc_eff, threshold);
    let mut entries = Vec::with_capacity(staged.len());
    let mut ratio_extras = Vec::with_capacity(staged.len());
    for ((row_index, label, outcome), adj) in staged.into_iter().zip(adj_p) {
        entries.push(MultipleRowTestEntry {
            row_index,
            label,
            mean_a: outcome.loc_a,
            mean_b: outcome.loc_b,
            difference: outcome.diff,
            statistic: outcome.stat,
            df: outcome.df,
            raw_p: outcome.p_val,
            adjusted_p_or_q: adj,
            significant: adj.is_finite() && adj <= threshold,
        });
        ratio_extras.push((outcome.ratio, outcome.ratio_ci));
    }

    let discoveries = entries.iter().filter(|e| e.significant).count();
    let min_raw_p = entries.iter().map(|e| e.raw_p).fold(f64::INFINITY, f64::min);
    let min_adj_p = entries.iter().map(|e| e.adjusted_p_or_q).fold(f64::INFINITY, f64::min);
    let variant_label = if is_lognormal_mode {
        match (effective_paired, mr.assume_equal_sd) {
            (true, _) => "Ratio paired t test",
            (false, true) => "Lognormal unpaired Student t test",
            (false, false) => "Lognormal Welch unpaired t test",
        }
    } else {
        match (mr.paired, mr.nonparametric, mr.assume_equal_sd) {
            (true, true, _) => "Wilcoxon matched-pairs signed-rank",
            (true, false, _) => "Paired Student t test",
            (false, true, _) => "Mann-Whitney U test",
            (false, false, true) => "Unpaired Student t test",
            (false, false, false) => "Welch unpaired t test",
        }
    };

    let pair_desc = format!("{lbl_a} vs. {lbl_b}");
    res.tabs.push(
        KvTabBuilder::new("multiple-row-summary", "Multiple Row t-Tests Summary", ResultTabKind::MainSummary, pair_desc.clone())
            .num("Rows tested", entries.len() as f64)
            .num("Rows skipped (insufficient data)", skipped_rows as f64)
            .num("Significant discoveries", discoveries as f64)
            .text("Test family", variant_label)
            .text("Multiplicity method", format!("{:?}", mr.multiplicity.method))
            .num("Significance / FDR threshold", threshold)
            .p("Minimum raw P", min_raw_p, p_fmt, conf.alpha())
            .p("Minimum adjusted P / q", min_adj_p, p_fmt, threshold)
            .build(),
    );

    let mut cmp_tab = if is_lognormal_mode {
        TabBuilder::new("multiple-row-comparisons", "Multiple Row-Wise Comparisons", ResultTabKind::MultipleComparisons).headers([
            format!("Geo. Mean ({lbl_a})"),
            format!("Geo. Mean ({lbl_b})"),
            String::from("Geo. Mean Ratio (A / B)"),
            format!("{}% CI of Ratio", conf.percent()),
            String::from("Log Difference (ln A - ln B)"),
            String::from("Statistic"),
            String::from("df"),
            String::from("Raw P"),
            String::from("Adjusted P / q"),
            String::from("Significant"),
        ])
    } else {
        TabBuilder::new("multiple-row-comparisons", "Multiple Row-Wise Comparisons", ResultTabKind::MultipleComparisons).headers([
            format!("Mean ({lbl_a})"),
            format!("Mean ({lbl_b})"),
            String::from("Difference (A - B)"),
            String::from("Statistic"),
            String::from("df"),
            String::from("Raw P"),
            String::from("Adjusted P / q"),
            String::from("Significant"),
        ])
    };
    for (e, &(ratio_opt, ratio_ci_opt)) in entries.iter().zip(ratio_extras.iter()) {
        if is_lognormal_mode {
            cmp_tab.push_row(
                e.label.clone(),
                [
                    num_cell(e.mean_a),
                    num_cell(e.mean_b),
                    opt_num_cell(ratio_opt),
                    opt_ci_cell(ratio_ci_opt),
                    num_cell(e.difference),
                    num_cell(e.statistic),
                    num_cell(e.df),
                    p_cell_at_alpha(e.raw_p, p_fmt, conf.alpha()),
                    p_cell_at_alpha(e.adjusted_p_or_q, p_fmt, threshold),
                    ResultCell::Boolean(e.significant),
                ],
            );
        } else {
            cmp_tab.push_row(
                e.label.clone(),
                [
                    num_cell(e.mean_a),
                    num_cell(e.mean_b),
                    num_cell(e.difference),
                    num_cell(e.statistic),
                    num_cell(e.df),
                    p_cell_at_alpha(e.raw_p, p_fmt, conf.alpha()),
                    p_cell_at_alpha(e.adjusted_p_or_q, p_fmt, threshold),
                    ResultCell::Boolean(e.significant),
                ],
            );
        }
        res.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: e.row_index,
            group_b: 1,
            label_a: e.label.clone(),
            label_b: pair_desc.clone(),
            estimate_diff: Some(e.difference),
            raw_p_or_q: e.adjusted_p_or_q,
            is_fdr_q: is_fdr,
            significant: e.significant,
        });
    }
    if is_lognormal_mode {
        cmp_tab.push_footnote(
            "Lognormal / ratio-paired row comparisons evaluate Student or Welch t tests on ln(Y) for strictly positive values and back-transform log-differences and confidence intervals into geometric mean ratios (A / B).",
        );
    } else if mr.paired && mr.nonparametric {
        cmp_tab.push_footnote(
            "Paired nonparametric rows use Wilcoxon matched-pairs signed-rank with Pratt (1959) zero-difference handling; df is omitted (not applicable) for rank tests.",
        );
    } else if mr.nonparametric {
        cmp_tab.push_footnote("Unpaired nonparametric rows use the Mann-Whitney U test; df is omitted (not applicable) for rank tests.");
    }
    res.tabs.push(cmp_tab.build());

    push_scalar(&mut res, "RowsTested", None, entries.len() as f64);
    push_scalar(&mut res, "Discoveries", None, discoveries as f64);
    push_scalar(&mut res, "Threshold", None, threshold);
    push_scalar(&mut res, "MinRawP", None, min_raw_p);
    push_scalar(&mut res, "MinAdjustedP", None, min_adj_p);
    res.checklist.push(checklist_item(
        "Multiple row hypothesis testing",
        ChecklistStatus::Pass,
        Some(format!("{discoveries} / {} rows significant at threshold {threshold}", entries.len())),
        "Per-row test statistics and multiplicity-adjusted P/q values are available for Volcano plotting.",
    ));
    if effective_paired {
        append_listwise_filtered_checklist_and_footnote(&mut res, "Multiple row paired t-tests", inputs.skipped_incomplete_pairs, inputs.kept_pairs);
    }
    append_ignored_text_checklist_and_footnote(&mut res, "Multiple row t-tests", &inputs.ignored_non_numeric);
    sanitize_result_sheet_floats(&mut res)?;
    Ok(res)
}

#[derive(Debug, Clone)]
struct LabeledGroupSample {
    label: String,
    values: Vec<f64>,
}

struct MvTwoGroupExtraction {
    group_a: LabeledGroupSample,
    group_b: LabeledGroupSample,
    skipped_missing: usize,
    ignored_non_numeric: Vec<(String, usize)>,
    accounting: ObservationAccounting,
}

fn resolve_mv_one_sample_selection(sheet: &DataSheet, selected: &[usize], mv_response: Option<usize>) -> Result<Vec<usize>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            let indices = try_resolve_selected_indices(selected, col_tab.data_sets.len())?;
            if !selected.is_empty() && indices.len() != 1 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
            }
            let &first_idx = indices.first().ok_or(StatsError::EmptyInput)?;
            Ok(vec![first_idx])
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            if let Some(r_col) = mv_response {
                if r_col >= mv.variables.len() {
                    return Err(StatsError::MissingColumn { index: r_col });
                }
                return Ok(vec![r_col]);
            }
            let numeric_indices: Vec<usize> = indices.into_iter().filter(|&i| is_mv_numeric_response_candidate(&mv.variables[i])).collect();
            if !selected.is_empty() && numeric_indices.len() != 1 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: numeric_indices.len() as f64 });
            }
            let &first_idx = numeric_indices.first().ok_or(StatsError::EmptyInput)?;
            Ok(vec![first_idx])
        }
        _ => Ok(selected.to_vec()),
    }
}

fn extract_two_group_wide_columns(sheet: &DataSheet, selected: &[usize]) -> Result<Vec<crate::common::ExtractedColumn>, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col_tab) => {
            if col_tab.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(selected, col_tab.data_sets.len())?;
            if !selected.is_empty() && indices.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
            }
            if indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize { context: "two-group comparison columns", required: 2, actual: indices.len() });
            }
            extract_numeric_columns(sheet, &indices[..2])
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices = try_resolve_selected_indices(selected, mv.variables.len())?;
            let numeric_indices: Vec<usize> = indices.into_iter().filter(|&i| is_mv_numeric_response_candidate(&mv.variables[i])).collect();
            if !selected.is_empty() && numeric_indices.len() != 2 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: numeric_indices.len() as f64 });
            }
            if numeric_indices.len() < 2 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "two-group comparison columns",
                    required: 2,
                    actual: numeric_indices.len(),
                });
            }
            extract_numeric_columns(sheet, &numeric_indices[..2])
        }
        _ => extract_numeric_columns(sheet, selected),
    }
}

fn mv_cell_token(cell: &Cell) -> Option<String> {
    if cell.excluded {
        return None;
    }
    match &cell.value {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        CellValue::Number(_) => None,
    }
}

fn infer_mv_group_column(mv: &MultipleVariablesTable, response_col: Option<usize>, selected: &[usize]) -> Result<usize, StatsError> {
    let pool = try_resolve_selected_indices(selected, mv.variables.len())?;
    let candidates: Vec<usize> = pool
        .into_iter()
        .filter(|&idx| Some(idx) != response_col && mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Categorical))
        .collect();
    match candidates.as_slice() {
        [only] => Ok(*only),
        _ => Err(StatsError::InvalidParameter { name: "mv_group_column", value: candidates.len() as f64 }),
    }
}

fn is_mv_numeric_response_candidate(var: &curvebook_doc::VariableColumn) -> bool {
    !matches!(var.role, VariableRole::Categorical | VariableRole::Label)
}

fn select_default_mv_response_column(mv: &MultipleVariablesTable, candidates: &[usize]) -> Option<usize> {
    if candidates.len() <= 1 {
        return candidates.first().copied();
    }
    let active: Vec<usize> = candidates
        .iter()
        .copied()
        .filter(|&idx| mv.variables.get(idx).is_some_and(|v| v.cells.iter().any(|c| c.active_number().is_some())))
        .collect();
    // On multi-column MV tables (such as qPCR pipelines with derived columns), prefer the last
    // active calculated response column, then the last active numeric column, then the last candidate.
    active
        .iter()
        .copied()
        .rfind(|&idx| mv.variables.get(idx).is_some_and(|v| v.role == VariableRole::Calculated))
        .or_else(|| active.last().copied())
        .or_else(|| candidates.last().copied())
}

fn infer_mv_response_column(mv: &MultipleVariablesTable, group_col: usize, selected: &[usize]) -> Result<usize, StatsError> {
    let pool = try_resolve_selected_indices(selected, mv.variables.len())?;
    let candidates: Vec<usize> =
        pool.into_iter().filter(|&idx| idx != group_col && mv.variables.get(idx).is_some_and(is_mv_numeric_response_candidate)).collect();
    select_default_mv_response_column(mv, &candidates)
        .ok_or(StatsError::InvalidParameter { name: "mv_response_column", value: candidates.len() as f64 })
}

fn resolve_mv_two_group_inputs(sheet: &DataSheet, spec: &AnalysisSpec, tg: &TwoGroupSpec) -> Result<Option<MvTwoGroupExtraction>, StatsError> {
    let TableArchetype::MultipleVariables(mv) = &sheet.archetype else {
        return Ok(None);
    };
    let selected = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?;
    let explicit_mode = tg.mv_group_column.is_some() || tg.mv_response_column.is_some();
    let (g_col, r_col) = if explicit_mode {
        let g = match tg.mv_group_column {
            Some(g) => g,
            None => infer_mv_group_column(mv, tg.mv_response_column, &spec.selected_data_sets)?,
        };
        let r = match tg.mv_response_column {
            Some(r) => r,
            None => infer_mv_response_column(mv, g, &spec.selected_data_sets)?,
        };
        (g, r)
    } else {
        let cat_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables[i].role == VariableRole::Categorical).collect();
        let num_cols: Vec<usize> = selected.iter().copied().filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_response_candidate)).collect();
        if cat_cols.len() >= 2 {
            return Err(StatsError::InvalidParameter { name: "mv_group_column", value: cat_cols.len() as f64 });
        }
        if cat_cols.len() == 1 {
            let r_col = select_default_mv_response_column(mv, &num_cols)
                .ok_or(StatsError::InvalidParameter { name: "mv_response_column", value: num_cols.len() as f64 })?;
            (cat_cols[0], r_col)
        } else {
            return Ok(None);
        }
    };

    let g_var = mv.variables.get(g_col).ok_or(StatsError::MissingColumn { index: g_col })?;
    let r_var = mv.variables.get(r_col).ok_or(StatsError::MissingColumn { index: r_col })?;
    if g_col == r_col {
        return Err(StatsError::InvalidParameter { name: "mv_group_column", value: g_col as f64 });
    }

    let levels: Vec<String> = g_var.ordered_levels().into_iter().filter(|l| l.included_count > 0).map(|l| l.level).collect();
    if levels.len() != 2 {
        return Err(StatsError::DimensionMismatch { expected: 2, actual: levels.len() });
    }

    let mut vals_a = Vec::new();
    let mut vals_b = Vec::new();
    let mut skipped_missing = 0_usize;
    let mut non_numeric_response = 0_usize;
    let mut accounting = ObservationAccounting::new(ObservationAccountingKind::Unpaired, 0);
    for r in 0..mv.row_count() {
        let Some(gc) = g_var.cells.get(r) else {
            continue;
        };
        let Some(rc) = r_var.cells.get(r) else {
            continue;
        };
        if gc.excluded {
            accounting.excluded_cells = accounting.excluded_cells.saturating_add(1);
        }
        if rc.excluded {
            accounting.excluded_cells = accounting.excluded_cells.saturating_add(1);
        }
        let resp_non_num = is_ignored_non_numeric_cell(rc);
        if resp_non_num {
            non_numeric_response += 1;
        }
        let tok = mv_cell_token(gc);
        let num = rc.active_number();
        match (tok, num) {
            (Some(t), Some(y)) if t == levels[0] => vals_a.push(y),
            (Some(t), Some(y)) if t == levels[1] => vals_b.push(y),
            _ => {
                if !gc.is_empty() || !rc.is_empty() || gc.excluded || rc.excluded {
                    skipped_missing += 1;
                    record_omitted_row_reason(&mut accounting, gc.excluded || rc.excluded, resp_non_num, false);
                }
            }
        }
    }

    if vals_a.is_empty() || vals_b.is_empty() {
        return Err(StatsError::InsufficientSampleSize {
            context: "two-group MV level observations",
            required: 1,
            actual: vals_a.len().min(vals_b.len()),
        });
    }

    accounting.analyzed = vals_a.len().saturating_add(vals_b.len());
    accounting.ignored_non_numeric_cells = non_numeric_response;
    let ignored = if non_numeric_response > 0 { vec![(r_var.title.clone(), non_numeric_response)] } else { Vec::new() };
    Ok(Some(MvTwoGroupExtraction {
        group_a: LabeledGroupSample { label: levels[0].clone(), values: vals_a },
        group_b: LabeledGroupSample { label: levels[1].clone(), values: vals_b },
        skipped_missing,
        ignored_non_numeric: ignored,
        accounting,
    }))
}

struct BlandAltmanExtractedPairs {
    label_a: String,
    label_b: String,
    xs: Vec<f64>,
    ys: Vec<f64>,
    skipped_incomplete_rows: usize,
    non_numeric_cells: usize,
    excluded_cells: usize,
    /// Pairs analyzed and omitted rows by reason.
    accounting: curvebook_doc::PairedObservationAccounting,
}

fn extract_bland_altman_pairs(sheet: &DataSheet, ba: &BlandAltmanSpec) -> Result<BlandAltmanExtractedPairs, StatsError> {
    match &sheet.archetype {
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let report = extract_paired_values_with_report(sheet, ba.column_a, ba.column_b)?;
            let la = col.data_sets[ba.column_a].title.clone();
            let lb = col.data_sets[ba.column_b].title.clone();
            Ok(BlandAltmanExtractedPairs {
                label_a: la,
                label_b: lb,
                xs: report.xs,
                ys: report.ys,
                skipped_incomplete_rows: report.skipped_incomplete_rows,
                non_numeric_cells: report.non_numeric_cells,
                excluded_cells: report.excluded_cells,
                accounting: report.accounting,
            })
        }
        TableArchetype::MultipleVariables(mv) => {
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let report = extract_paired_values_with_report(sheet, ba.column_a, ba.column_b)?;
            let la = mv.variables[ba.column_a].title.clone();
            let lb = mv.variables[ba.column_b].title.clone();
            Ok(BlandAltmanExtractedPairs {
                label_a: la,
                label_b: lb,
                xs: report.xs,
                ys: report.ys,
                skipped_incomplete_rows: report.skipped_incomplete_rows,
                non_numeric_cells: report.non_numeric_cells,
                excluded_cells: report.excluded_cells,
                accounting: report.accounting,
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            if xy.data_sets.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            if ba.column_a == ba.column_b {
                return Err(StatsError::InvalidParameter { name: "column_b", value: ba.column_b as f64 });
            }
            let compare_x_vs_y = ba.xy_compare_x_vs_y.unwrap_or(xy.data_sets.len() == 1);
            if compare_x_vs_y {
                // X vs Y[0] mode: require canonical indices (0, 1) regardless of additional Y datasets.
                if ba.column_a != 0 {
                    return Err(StatsError::MissingColumn { index: ba.column_a });
                }
                if ba.column_b != 1 {
                    return Err(StatsError::MissingColumn { index: ba.column_b });
                }
                let ds = xy.data_sets.first().ok_or(StatsError::EmptyInput)?;
                let is_gen = matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. });
                let active_end = (0..xy.row_count()).rfind(|&r| {
                    let x_cell = xy.materialized_x_cell_at(r);
                    let x_active = if is_gen { x_cell.excluded } else { !x_cell.is_empty() || x_cell.excluded };
                    x_active || ds.cells.get(r).is_some_and(|subs| subs.iter().any(|c| !c.is_empty() || c.excluded))
                });
                let Some(end_row) = active_end else {
                    return Err(StatsError::EmptyInput);
                };
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                let mut non_numeric_cells = 0_usize;
                let mut excluded_cells = 0_usize;
                let mut skipped_incomplete_rows = 0_usize;
                let mut accounting = curvebook_doc::PairedObservationAccounting::default();
                for r in 0..=end_row {
                    let x_cell = xy.materialized_x_cell_at(r);
                    if !is_gen && x_cell.excluded {
                        excluded_cells += 1;
                    }
                    if !is_gen && is_ignored_non_numeric_cell(&x_cell) {
                        non_numeric_cells += 1;
                    }
                    let x_opt = xy.x_summary_at(r)?;
                    let x_has_entry = is_gen || !x_cell.is_empty() || x_cell.excluded;
                    for cy in ds.cells.get(r).map(Vec::as_slice).unwrap_or(&[]) {
                        if cy.excluded {
                            excluded_cells += 1;
                        }
                        if is_ignored_non_numeric_cell(cy) {
                            non_numeric_cells += 1;
                        }
                        if let (Some(x), Some(y)) = (x_opt, cy.active_number()) {
                            xs.push(x);
                            ys.push(y);
                        } else if x_has_entry || !cy.is_empty() || cy.excluded {
                            skipped_incomplete_rows += 1;
                            crate::common::record_omitted_pair(&mut accounting, (!is_gen).then_some(&x_cell), Some(cy));
                        }
                    }
                }
                if xs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                Ok(BlandAltmanExtractedPairs {
                    label_a: xy.x_title.clone(),
                    label_b: ds.title.clone(),
                    accounting: curvebook_doc::PairedObservationAccounting { pairs_analyzed: xs.len(), ..accounting },
                    xs,
                    ys,
                    skipped_incomplete_rows,
                    non_numeric_cells,
                    excluded_cells,
                })
            } else {
                let dsa = xy.data_sets.get(ba.column_a).ok_or(StatsError::MissingColumn { index: ba.column_a })?;
                let dsb = xy.data_sets.get(ba.column_b).ok_or(StatsError::MissingColumn { index: ba.column_b })?;
                if dsa.subcolumn_count() != dsb.subcolumn_count() {
                    return Err(StatsError::DimensionMismatch { expected: dsa.subcolumn_count(), actual: dsb.subcolumn_count() });
                }
                let mut xs = Vec::new();
                let mut ys = Vec::new();
                let mut non_numeric_cells = 0_usize;
                let mut excluded_cells = 0_usize;
                let mut skipped_incomplete_rows = 0_usize;
                let mut accounting = curvebook_doc::PairedObservationAccounting::default();
                for r in 0..xy.row_count() {
                    let sa = dsa.cells.get(r).map(Vec::as_slice).unwrap_or(&[]);
                    let sb = dsb.cells.get(r).map(Vec::as_slice).unwrap_or(&[]);
                    if sa.len() != sb.len() {
                        return Err(StatsError::DimensionMismatch { expected: sa.len(), actual: sb.len() });
                    }
                    for (ca, cb) in sa.iter().zip(sb.iter()) {
                        if ca.excluded {
                            excluded_cells += 1;
                        }
                        if cb.excluded {
                            excluded_cells += 1;
                        }
                        if is_ignored_non_numeric_cell(ca) {
                            non_numeric_cells += 1;
                        }
                        if is_ignored_non_numeric_cell(cb) {
                            non_numeric_cells += 1;
                        }
                        let has_entry = (!ca.is_empty() || ca.excluded) || (!cb.is_empty() || cb.excluded);
                        if let (Some(a), Some(b)) = (ca.active_number(), cb.active_number()) {
                            xs.push(a);
                            ys.push(b);
                        } else if has_entry {
                            skipped_incomplete_rows += 1;
                            crate::common::record_omitted_pair(&mut accounting, Some(ca), Some(cb));
                        }
                    }
                }
                if xs.is_empty() {
                    return Err(StatsError::EmptyInput);
                }
                Ok(BlandAltmanExtractedPairs {
                    label_a: dsa.title.clone(),
                    label_b: dsb.title.clone(),
                    accounting: curvebook_doc::PairedObservationAccounting { pairs_analyzed: xs.len(), ..accounting },
                    xs,
                    ys,
                    skipped_incomplete_rows,
                    non_numeric_cells,
                    excluded_cells,
                })
            }
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Bland-Altman", actual: other.kind() }),
    }
}

struct MultipleRowInputs {
    label_a: String,
    label_b: String,
    row_labels: Vec<String>,
    rows_a: Vec<Vec<f64>>,
    rows_b: Vec<Vec<f64>>,
    skipped_incomplete_pairs: usize,
    kept_pairs: usize,
    ignored_non_numeric: Vec<(String, usize)>,
}

fn extract_dataset_pair_multiple_row_inputs<F>(
    data_sets: &[curvebook_doc::DataSetColumn],
    spec: &AnalysisSpec,
    paired: bool,
    n_rows: usize,
    row_label: F,
) -> Result<MultipleRowInputs, StatsError>
where
    F: Fn(usize) -> String,
{
    let indices = try_resolve_selected_indices(&spec.selected_data_sets, data_sets.len())?;
    if paired && indices.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
    }
    if !spec.selected_data_sets.is_empty() && indices.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
    }
    if indices.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "multiple row t-tests groups", required: 2, actual: indices.len() });
    }
    let (ia, ib) = (indices[0], indices[1]);
    let dsa = &data_sets[ia];
    let dsb = &data_sets[ib];
    if paired && dsa.subcolumn_count() != dsb.subcolumn_count() {
        return Err(StatsError::DimensionMismatch { expected: dsa.subcolumn_count(), actual: dsb.subcolumn_count() });
    }
    let mut labels = Vec::with_capacity(n_rows);
    let mut rows_a = Vec::with_capacity(n_rows);
    let mut rows_b = Vec::with_capacity(n_rows);
    let mut non_numeric_a = 0_usize;
    let mut non_numeric_b = 0_usize;
    let mut skipped_incomplete_pairs = 0_usize;
    let mut kept_pairs = 0_usize;
    for r in 0..n_rows {
        let (ra, rb) = (dsa.cells.get(r), dsb.cells.get(r));
        if paired
            && let (Some(sa), Some(sb)) = (ra, rb)
            && sa.len() != sb.len()
        {
            return Err(StatsError::DimensionMismatch { expected: sa.len(), actual: sb.len() });
        }
        if let Some(sa) = ra {
            non_numeric_a += sa.iter().filter(|c| is_ignored_non_numeric_cell(c)).count();
        }
        if let Some(sb) = rb {
            non_numeric_b += sb.iter().filter(|c| is_ignored_non_numeric_cell(c)).count();
        }
        let (va, vb, skipped) = extract_paired_or_unpaired_subcolumns(ra, rb, paired);
        if paired {
            kept_pairs += va.len();
            skipped_incomplete_pairs += skipped;
        }
        labels.push(row_label(r));
        rows_a.push(va);
        rows_b.push(vb);
    }
    let mut ignored_non_numeric = Vec::new();
    if non_numeric_a > 0 {
        ignored_non_numeric.push((dsa.title.clone(), non_numeric_a));
    }
    if non_numeric_b > 0 {
        ignored_non_numeric.push((dsb.title.clone(), non_numeric_b));
    }
    Ok(MultipleRowInputs {
        label_a: dsa.title.clone(),
        label_b: dsb.title.clone(),
        row_labels: labels,
        rows_a,
        rows_b,
        skipped_incomplete_pairs,
        kept_pairs,
        ignored_non_numeric,
    })
}

fn extract_split_column_multiple_row_inputs<'a, L, C, T>(
    indices: &[usize],
    paired: bool,
    n_rows: usize,
    row_label: L,
    cell_at: C,
    col_title: T,
) -> MultipleRowInputs
where
    L: Fn(usize) -> String,
    C: Fn(usize, usize) -> Option<&'a Cell>,
    T: Fn(usize) -> String,
{
    let mid = indices.len() / 2;
    let (left, right) = (&indices[..mid], &indices[mid..]);
    let mut labels = Vec::with_capacity(n_rows);
    let mut rows_a = Vec::with_capacity(n_rows);
    let mut rows_b = Vec::with_capacity(n_rows);
    let mut non_numeric_counts = vec![0_usize; indices.len()];
    let mut skipped_incomplete_pairs = 0_usize;
    let mut kept_pairs = 0_usize;
    for r in 0..n_rows {
        let (va, vb) = if paired {
            let mut pa = Vec::new();
            let mut pb = Vec::new();
            for (pos, (&la, &rb_idx)) in left.iter().zip(right.iter()).enumerate() {
                let ca = cell_at(la, r);
                let cb = cell_at(rb_idx, r);
                if ca.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[pos] += 1;
                }
                if cb.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[mid + pos] += 1;
                }
                if let (Some(a), Some(b)) = (ca.and_then(Cell::active_number), cb.and_then(Cell::active_number)) {
                    pa.push(a);
                    pb.push(b);
                    kept_pairs += 1;
                } else if ca.is_some_and(|c| !c.is_empty() || c.excluded) || cb.is_some_and(|c| !c.is_empty() || c.excluded) {
                    skipped_incomplete_pairs += 1;
                }
            }
            (pa, pb)
        } else {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            for (pos, &col_idx) in indices.iter().enumerate() {
                let cell = cell_at(col_idx, r);
                if cell.is_some_and(is_ignored_non_numeric_cell) {
                    non_numeric_counts[pos] += 1;
                }
                if let Some(v) = cell.and_then(Cell::active_number) {
                    if pos < mid {
                        va.push(v);
                    } else {
                        vb.push(v);
                    }
                }
            }
            (va, vb)
        };
        labels.push(row_label(r));
        rows_a.push(va);
        rows_b.push(vb);
    }
    let ignored_non_numeric =
        indices.iter().zip(non_numeric_counts).filter(|(_, count)| *count > 0).map(|(&col_idx, count)| (col_title(col_idx), count)).collect();
    MultipleRowInputs {
        label_a: String::from("Group A"),
        label_b: String::from("Group B"),
        row_labels: labels,
        rows_a,
        rows_b,
        skipped_incomplete_pairs,
        kept_pairs,
        ignored_non_numeric,
    }
}

fn extract_multiple_row_inputs(sheet: &DataSheet, spec: &AnalysisSpec, paired: bool) -> Result<MultipleRowInputs, StatsError> {
    match &sheet.archetype {
        TableArchetype::Grouped(grp) => {
            if grp.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            extract_dataset_pair_multiple_row_inputs(&grp.data_sets, spec, paired, grp.row_count(), |r| {
                grp.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))
            })
        }
        TableArchetype::Xy(xy) => {
            if xy.y_subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            extract_dataset_pair_multiple_row_inputs(&xy.data_sets, spec, paired, xy.row_count(), |r| {
                xy.x_value_at(r).map_or_else(|| format!("Row {}", r + 1), |x| format!("{x}"))
            })
        }
        TableArchetype::Column(col) => {
            if col.subcolumn_mode.is_pre_summarized() {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(&spec.selected_data_sets, col.data_sets.len())?;
            if paired && !indices.len().is_multiple_of(2) {
                return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
            }
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests replicate columns per group",
                    required: 4,
                    actual: indices.len(),
                });
            }
            Ok(extract_split_column_multiple_row_inputs(
                &indices,
                paired,
                col.row_count(),
                |r| col.row_titles.get(r).filter(|s| !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)),
                |c, r| col.data_sets[c].cell(r, 0),
                |c| col.data_sets[c].title.clone(),
            ))
        }
        TableArchetype::MultipleVariables(mv) => {
            let indices: Vec<usize> = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?
                .into_iter()
                .filter(|&i| mv.variables.get(i).is_some_and(is_mv_numeric_response_candidate))
                .collect();
            if paired && !indices.len().is_multiple_of(2) {
                return Err(StatsError::InvalidParameter { name: "paired_selected_columns", value: indices.len() as f64 });
            }
            if indices.len() < 4 {
                return Err(StatsError::InsufficientSampleSize {
                    context: "multiple row t-tests numeric replicate columns",
                    required: 4,
                    actual: indices.len(),
                });
            }
            let label_col = mv
                .variables
                .iter()
                .position(|v| v.role == VariableRole::Label)
                .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Categorical));
            Ok(extract_split_column_multiple_row_inputs(
                &indices,
                paired,
                mv.row_count(),
                |r| label_col.and_then(|lc| mv.variables[lc].cells.get(r)).and_then(mv_cell_token).unwrap_or_else(|| format!("Row {}", r + 1)),
                |c, r| mv.variables[c].cells.get(r),
                |c| mv.variables[c].title.clone(),
            ))
        }
        other => Err(StatsError::IncompatibleTable { analysis: "Multiple row t-tests", actual: other.kind() }),
    }
}

fn extract_paired_or_unpaired_subcolumns(row_a: Option<&Vec<Cell>>, row_b: Option<&Vec<Cell>>, paired: bool) -> (Vec<f64>, Vec<f64>, usize) {
    match (row_a, row_b) {
        (Some(sa), Some(sb)) if paired => {
            let mut va = Vec::new();
            let mut vb = Vec::new();
            let mut skipped = 0_usize;
            for (ca, cb) in sa.iter().zip(sb.iter()) {
                if let (Some(a), Some(b)) = (ca.active_number(), cb.active_number()) {
                    va.push(a);
                    vb.push(b);
                } else if (!ca.is_empty() || ca.excluded) || (!cb.is_empty() || cb.excluded) {
                    skipped += 1;
                }
            }
            (va, vb, skipped)
        }
        (Some(sa), Some(sb)) => (sa.iter().filter_map(Cell::active_number).collect(), sb.iter().filter_map(Cell::active_number).collect(), 0),
        _ => (Vec::new(), Vec::new(), 0),
    }
}

struct SingleRowOutcome {
    loc_a: f64,
    loc_b: f64,
    diff: f64,
    ratio: Option<f64>,
    ratio_ci: Option<(f64, f64)>,
    stat: f64,
    df: f64,
    p_val: f64,
}

fn run_single_row_comparison(ra: &[f64], rb: &[f64], mr: &MultipleRowTestsSpec, conf: ConfidenceLevel) -> Result<SingleRowOutcome, StatsError> {
    if mr.lognormal_mode != LognormalRowTestMode::None {
        let effective_paired = mr.paired || mr.lognormal_mode == LognormalRowTestMode::RatioPaired;
        if effective_paired {
            let out = ratio_paired_t_test(ra, rb, true, conf)?;
            let la = validate_and_ln_transform(ra, "lognormal_x")?;
            let lb = validate_and_ln_transform(rb, "lognormal_x")?;
            let gm_a = SummaryMoments::from_slice(&la)?.mean.exp();
            let gm_b = SummaryMoments::from_slice(&lb)?.mean.exp();
            return Ok(SingleRowOutcome {
                loc_a: gm_a,
                loc_b: gm_b,
                diff: out.mean_of_differences,
                ratio: out.geometric_mean_ratio.or_else(|| Some(out.mean_of_differences.exp())),
                ratio_ci: out.geometric_mean_ratio_ci.or_else(|| Some((out.ci.0.exp(), out.ci.1.exp()))),
                stat: out.t_statistic,
                df: out.df,
                p_val: out.p_value,
            });
        }
        let out = if mr.assume_equal_sd { lognormal_unpaired_t_test(ra, rb, true, conf)? } else { lognormal_welch_t_test(ra, rb, true, conf)? };
        return Ok(SingleRowOutcome {
            loc_a: out.mean_a.exp(),
            loc_b: out.mean_b.exp(),
            diff: out.difference,
            ratio: out.geometric_mean_ratio.or_else(|| Some(out.difference.exp())),
            ratio_ci: out.geometric_mean_ratio_ci.or_else(|| Some((out.ci.0.exp(), out.ci.1.exp()))),
            stat: out.t_statistic,
            df: out.df,
            p_val: out.p_value,
        });
    }
    if mr.paired && mr.nonparametric {
        let out = wilcoxon_matched_pairs(ra, rb, true, conf, true)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok(SingleRowOutcome {
            loc_a: ma,
            loc_b: mb,
            diff: out.hodges_lehmann_pseudomedian,
            ratio: None,
            ratio_ci: None,
            stat: out.w_stat,
            df: f64::INFINITY,
            p_val: out.p_value,
        });
    }
    if mr.paired {
        let out = paired_t_test(ra, rb, true, conf)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok(SingleRowOutcome {
            loc_a: ma,
            loc_b: mb,
            diff: out.mean_of_differences,
            ratio: None,
            ratio_ci: None,
            stat: out.t_statistic,
            df: out.df,
            p_val: out.p_value,
        });
    }
    if mr.nonparametric {
        let out = mann_whitney_u(ra, rb, true, conf)?;
        let ma = SummaryMoments::from_slice(ra)?.mean;
        let mb = SummaryMoments::from_slice(rb)?.mean;
        return Ok(SingleRowOutcome {
            loc_a: ma,
            loc_b: mb,
            diff: out.hodges_lehmann_diff,
            ratio: None,
            ratio_ci: None,
            stat: out.u_min,
            df: f64::INFINITY,
            p_val: out.p_value,
        });
    }
    let out = if mr.assume_equal_sd { unpaired_t_test(ra, rb, true, conf)? } else { welch_t_test(ra, rb, true, conf)? };
    Ok(SingleRowOutcome {
        loc_a: out.mean_a,
        loc_b: out.mean_b,
        diff: out.difference,
        ratio: None,
        ratio_ci: None,
        stat: out.t_statistic,
        df: out.df,
        p_val: out.p_value,
    })
}

fn populate_unpaired_t_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &UnpairedTResult, include_es: bool, fmt: SheetFormatCtx) {
    populate_unpaired_t_sheet_impl(res, pair, out, include_es, fmt, true);
}

fn populate_unpaired_t_sheet_impl(
    res: &mut ResultSheet,
    pair: GroupPairMeta<'_>,
    out: &UnpairedTResult,
    include_es: bool,
    fmt: SheetFormatCtx,
    has_raw_replicates: bool,
) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let is_lognormal = out.geometric_mean_ratio.is_some();
    let alpha = conf.alpha();
    let pct = conf.percent();
    let suffix = if is_lognormal { " (ln scale)" } else { "" };
    let mut tab = KvTabBuilder::new("two-group", out.method_label, ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .num("t statistic", out.t_statistic)
        .num("df", out.df)
        .num(if is_lognormal { format!("Mean of ln({lbl_a})") } else { format!("Mean ({lbl_a})") }, out.mean_a)
        .num(if is_lognormal { format!("Mean of ln({lbl_b})") } else { format!("Mean ({lbl_b})") }, out.mean_b)
        .num(if is_lognormal { "Difference of ln means (ln A - ln B)" } else { "Difference between means (A - B)" }, out.difference)
        .num(if is_lognormal { "SE of ln difference" } else { "SE of difference" }, out.se_difference)
        .ci(if is_lognormal { format!("{pct}% CI of ln difference") } else { format!("{pct}% CI of difference") }, out.ci.0, out.ci.1)
        .num(if is_lognormal { "F-test to compare ln variances (F)" } else { "F-test to compare variances (F)" }, out.f_test.f_statistic)
        .p(format!("F-test P value{suffix}"), out.f_test.p_value, p_fmt, alpha);
    if let Some(gmr) = out.geometric_mean_ratio {
        tab.push_num("Geometric mean ratio (A / B)", gmr).push_opt_ci(format!("{pct}% CI of geometric ratio"), out.geometric_mean_ratio_ci);
    }
    if include_es {
        let std_grp = if is_lognormal { format!("ln({lbl_a})") } else { lbl_a.to_owned() };
        tab.push_num(format!("Cohen's d{suffix}"), out.effect_sizes.cohens_d)
            .push_opt_ci(format!("{pct}% CI of Cohen's d{suffix}"), out.effect_sizes.cohens_d_ci)
            .push_num(format!("Hedges' g{suffix}"), out.effect_sizes.hedges_g)
            .push_opt_ci(format!("{pct}% CI of Hedges' g{suffix}"), out.effect_sizes.hedges_g_ci)
            .push_opt_num(format!("Glass's Delta{suffix}"), out.effect_sizes.glass_delta)
            .push_text("Glass's Delta standardizing group", std_grp)
            .push_num(format!("R squared{suffix}"), out.effect_sizes.r_squared);
    }
    if is_lognormal {
        let note = if out.method_label == "Lognormal Welch's t test" {
            "Lognormal Welch t-test evaluates Welch-Satterthwaite t on ln(Y) without assuming equal log-scale variances; log-scale means, differences, CIs, and effect sizes are reported alongside the back-transformed geometric mean ratio."
        } else {
            "Lognormal unpaired t-test evaluates Student's t on ln(Y); log-scale means, differences, CIs, and effect sizes are on the natural-log scale alongside the back-transformed geometric mean ratio."
        };
        tab.push_footnote(note);
    }
    if include_es {
        if out.effect_sizes.glass_delta.is_none() {
            let msg = format!(
                "Glass's Delta standardizes by group \"{lbl_a}\" SD and is unavailable because \"{lbl_a}\" has zero SD (group \"{lbl_b}\" SD is not substituted)."
            );
            tab.push_footnote(msg.clone());
            res.checklist.push(checklist_item("Glass's Delta standardizing SD", ChecklistStatus::Info, Some(format!("SD({lbl_a}) = 0")), msg));
        } else {
            tab.push_footnote(format!("Glass's Delta standardizes by group \"{lbl_a}\" SD."));
        }
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    push_scalar(res, "CohensD", None, out.effect_sizes.cohens_d);
    push_scalar(res, "HedgesG", None, out.effect_sizes.hedges_g);
    if let Some(gmr) = out.geometric_mean_ratio {
        push_scalar(res, "GeometricMeanRatio", None, gmr);
    }
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.difference),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    append_unpaired_checklist(&mut res.checklist, out, alpha, has_raw_replicates);
}

fn append_unpaired_checklist(checklist: &mut Vec<ChecklistItem>, out: &UnpairedTResult, alpha: f64, has_raw_replicates: bool) {
    if out.n_a < 5 || out.n_b < 5 {
        checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Warn,
            Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
            "Small group sample sizes (n < 5) limit statistical power and make normality and variance homogeneity tests inconclusive.",
        ));
    } else {
        checklist.push(checklist_item(
            "Sample size adequacy",
            ChecklistStatus::Pass,
            Some(format!("n_A = {}, n_B = {}", out.n_a, out.n_b)),
            "Both groups contain at least 5 observations.",
        ));
    }
    let assumes_equal_var = !matches!(out.method_label, "Welch's t test" | "Lognormal Welch's t test");
    let f_violated = out.f_test.p_value <= alpha;
    let levene_opt = if has_raw_replicates { out.levene_median.as_ref() } else { None };
    let levene_violated = levene_opt.is_some_and(|lev| lev.p_value <= alpha);
    if assumes_equal_var && (f_violated || levene_violated) {
        let (metric, explanation) = if f_violated {
            (
                Some(format!("F = {:.3}, p = {:.4}", out.f_test.f_statistic, out.f_test.p_value)),
                "F-test indicates unequal group variances (p <= alpha), violating the equal-variance assumption of pooled-variance Student t tests.",
            )
        } else if let Some(levene) = levene_opt {
            (
                Some(format!("Levene(median) F = {:.3}, p = {:.4}", levene.f_statistic, levene.p_value)),
                "Brown-Forsythe (Levene median) test indicates unequal group variances (p <= alpha), violating the equal-variance assumption of pooled-variance Student t tests.",
            )
        } else {
            (
                Some(format!("F = {:.3}, p = {:.4}", out.f_test.f_statistic, out.f_test.p_value)),
                "Variance test indicates unequal group variances (p <= alpha), violating the equal-variance assumption of pooled-variance Student t tests.",
            )
        };
        checklist.push(checklist_item("Homogeneity of variances", ChecklistStatus::Violated, metric, explanation));
    } else if let Some(levene) = levene_opt {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            if levene_violated { ChecklistStatus::Info } else { ChecklistStatus::Pass },
            Some(format!("Levene(median) F = {:.3}, p = {:.4}", levene.f_statistic, levene.p_value)),
            if levene_violated {
                "Levene test indicates heterogeneous variances; Welch's t-test does not assume equal variances."
            } else {
                "No severe departure from equal group variances was detected."
            },
        ));
    } else {
        checklist.push(checklist_item(
            "Variance homogeneity check",
            if f_violated { ChecklistStatus::Info } else { ChecklistStatus::Pass },
            Some(format!("F = {:.3}, p = {:.4}", out.f_test.f_statistic, out.f_test.p_value)),
            if f_violated {
                "Variance F-test indicates unequal group variances (p <= alpha); Welch's t-test does not assume equal variances."
            } else {
                "Variance F-test shows no significant departure from equal group variances."
            },
        ));
    }
    if !has_raw_replicates {
        checklist.push(checklist_item(
            "Pre-summarized input mode",
            ChecklistStatus::Info,
            None,
            "Moments were reconstructed from entered Mean, Spread, and N; normality and Levene median tests require raw replicates.",
        ));
    }
}

fn append_unpaired_normality_checklist(checklist: &mut Vec<ChecklistItem>, vals_a: &[f64], vals_b: &[f64], is_lognormal: bool, alpha: f64) {
    let prep =
        |slice: &[f64]| -> Vec<f64> { if is_lognormal { slice.iter().copied().filter(|&x| x > 0.0).map(f64::ln).collect() } else { slice.to_vec() } };
    let a = prep(vals_a);
    let b = prep(vals_b);
    let sw_a = shapiro_wilk(&a, alpha).ok();
    let sw_b = shapiro_wilk(&b, alpha).ok();
    if sw_a.is_none() && sw_b.is_none() {
        return;
    }
    let min_sw = [sw_a.as_ref(), sw_b.as_ref()].into_iter().flatten().min_by(|x, y| x.p_value.total_cmp(&y.p_value));
    if let Some(worst) = min_sw {
        let violated = !worst.passed;
        checklist.push(checklist_item(
            if is_lognormal { "Lognormality of group values" } else { "Normality of group values" },
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("min Shapiro-Wilk W = {:.4}, p = {:.4}", worst.statistic, worst.p_value)),
            if violated {
                "Shapiro-Wilk test indicates at least one group departs significantly from Gaussian normality."
            } else {
                "Neither group shows a significant departure from Gaussian normality."
            },
        ));
    }
}

fn append_paired_normality_checklist(checklist: &mut Vec<ChecklistItem>, vals_a: &[f64], vals_b: &[f64], is_ratio: bool, alpha: f64) {
    let diffs: Vec<f64> = vals_a
        .iter()
        .zip(vals_b.iter())
        .filter_map(|(&a, &b)| if is_ratio { (a > 0.0 && b > 0.0).then(|| (a / b).ln()) } else { Some(a - b) })
        .collect();
    if diffs.len() < 5 {
        checklist.push(checklist_item(
            "Normality of paired differences",
            ChecklistStatus::Warn,
            Some(format!("n = {} pairs", diffs.len())),
            format!(
                "Sample size (n = {} pairs) is too small to assess normality of paired differences reliably; assumption test is inconclusive.",
                diffs.len()
            ),
        ));
    } else if let Ok(sw) = shapiro_wilk(&diffs, alpha) {
        let violated = !sw.passed;
        checklist.push(checklist_item(
            "Normality of paired differences",
            if violated { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!("Shapiro-Wilk W = {:.4}, p = {:.4}", sw.statistic, sw.p_value)),
            if violated {
                "Shapiro-Wilk test indicates paired differences depart significantly from Gaussian normality."
            } else {
                "Paired differences do not depart significantly from Gaussian normality."
            },
        ));
    }
}

fn populate_paired_t_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &PairedTResult, include_es: bool, fmt: SheetFormatCtx) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let alpha = conf.alpha();
    let pct = conf.percent();
    let mut tab = KvTabBuilder::new("paired-t", out.method_label, ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .num("t statistic", out.t_statistic)
        .num("df", out.df)
        .num("Number of pairs", out.n_pairs as f64)
        .num("Mean of differences (A - B)", out.mean_of_differences)
        .num("SD of differences", out.sd_of_differences)
        .ci(format!("{pct}% CI of difference"), out.ci.0, out.ci.1)
        .opt_num("Pairing correlation (r)", out.pairing_r)
        .cell("Pairing effectiveness P", out.pairing_p.map_or(ResultCell::Empty, |p| p_cell_at_alpha(p, p_fmt, alpha)));
    if let Some(gmr) = out.geometric_mean_ratio {
        tab.push_num("Geometric mean of ratios (A / B)", gmr).push_opt_ci(format!("{pct}% CI of ratio"), out.geometric_mean_ratio_ci);
    }
    if include_es {
        tab.push_num("Cohen's dz", out.cohens_dz)
            .push_opt_num("Hedges' gz", out.hedges_gz)
            .push_opt_ci(format!("{pct}% CI of Hedges' gz"), out.hedges_gz_ci);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.mean_of_differences),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });
    let pairing_status = match (out.pairing_r, out.pairing_p) {
        (Some(r), Some(p)) if r > 0.0 && p <= conf.alpha() => ChecklistStatus::Pass,
        (Some(_), Some(_)) => ChecklistStatus::Warn,
        _ => ChecklistStatus::Info,
    };
    res.checklist.push(checklist_item(
        "Pairing effectiveness",
        pairing_status,
        out.pairing_r.map(|r| format!("r = {r:.4}")),
        "Evaluates two-sided Pearson correlation r; positive significant covariance reduces paired-difference variance.",
    ));
}

fn format_hl_ci_row_and_warn(
    tab: &mut KvTabBuilder,
    checklist: &mut Vec<ChecklistItem>,
    target_desc: &str,
    ci: Option<(f64, f64)>,
    actual_conf_pct: Option<f64>,
    max_bound_pct: Option<f64>,
    conf: ConfidenceLevel,
) -> Option<String> {
    let nominal = conf.percent();
    let unattainable =
        actual_conf_pct.is_some_and(|a| a + 1e-9 < nominal) || (actual_conf_pct.is_none() && max_bound_pct.is_some_and(|m| m + 1e-9 < nominal));
    let ci_label = match actual_conf_pct {
        Some(actual) if actual + 1e-9 < nominal => format!("{actual:.2}% CI of {target_desc} (requested {nominal}%)"),
        _ => format!("{nominal}% CI of {target_desc}"),
    };
    tab.push_opt_ci(ci_label, ci).push_opt_num("HL CI actual confidence (%)", actual_conf_pct);

    if unattainable {
        let (metric, msg) = if let Some(actual) = actual_conf_pct {
            (
                Some(format!("achieved = {actual:.2}% (requested {nominal}%)")),
                format!(
                    "Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size; exact achieved coverage is {actual:.2}%."
                ),
            )
        } else if let Some(bound) = max_bound_pct {
            (
                Some(format!("max achievable <= {bound:.2}% (requested {nominal}%)")),
                format!(
                    "Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size (maximum distribution-free coverage <= {bound:.2}%)."
                ),
            )
        } else {
            (None, format!("Requested {nominal}% Hodges-Lehmann confidence level is unattainable at this sample size."))
        };
        checklist.push(checklist_item("Hodges-Lehmann CI achieved coverage", ChecklistStatus::Warn, metric, msg.clone()));
        Some(msg)
    } else {
        None
    }
}

fn populate_mann_whitney_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &MannWhitneyResult, fmt: SheetFormatCtx) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let alpha = conf.alpha();
    let mut tab = KvTabBuilder::new("mann-whitney", "Mann-Whitney test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .boolean("Exact P used", out.exact_p_used)
        .num("Mann-Whitney U", out.u_min)
        .num("Hodges-Lehmann median diff.", out.hodges_lehmann_diff);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "median diff.",
        out.hodges_lehmann_ci,
        out.ci_actual_confidence_percent,
        mann_whitney_max_achievable_coverage_percent(out.n_a, out.n_b),
        conf,
    );
    tab.push_num("Cliff's delta", out.cliffs_delta).push_num("Rank-biserial r", out.rank_biserial_r);
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "MannWhitneyU", None, out.u_min);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.hodges_lehmann_diff),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    res.checklist.push(checklist_item(
        "Nonparametric rank comparison",
        ChecklistStatus::Pass,
        Some(format!("n_A = {}, n_B = {}, exact P = {}", out.n_a, out.n_b, out.exact_p_used)),
        if out.exact_p_used {
            "Exact permutation DP computed under the conditional null rank distribution; does not assume Gaussian normality."
        } else {
            "Asymptotic continuity-corrected tie-adjusted normal approximation used; does not assume Gaussian normality."
        },
    ));
}

fn populate_wilcoxon_paired_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &WilcoxonMatchedResult, fmt: SheetFormatCtx) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let alpha = conf.alpha();
    let mut tab = KvTabBuilder::new("wilcoxon-paired", "Wilcoxon matched-pairs test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
        .p("P value", out.p_value, p_fmt, alpha)
        .boolean("Exact P used", out.exact_p_used)
        .num("Sum of positive ranks (W+)", out.w_plus)
        .num("Sum of negative ranks (W-)", out.w_minus)
        .num("Hodges-Lehmann pseudomedian", out.hodges_lehmann_pseudomedian);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "pseudomedian",
        out.hodges_lehmann_ci,
        out.ci_actual_confidence_percent,
        wilcoxon_max_achievable_coverage_percent(out.n_pairs),
        conf,
    );
    tab.push_num("Rank-biserial r", out.rank_biserial_r);
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    tab.push_footnote(
        "Zero differences follow Pratt's (1959) method: all pairs (including exact zeros) are ranked by |d|, zero-difference ranks are excluded from W+ and W-, and exact small-sample P values are used when no zero differences or ties occur (n <= 25).",
    );
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.hodges_lehmann_pseudomedian),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= alpha,
    });
    res.checklist.push(checklist_item(
        "Symmetry of paired differences",
        ChecklistStatus::Info,
        Some(format!("effective pairs = {} (Pratt zero handling)", out.n_effective)),
        "Wilcoxon signed-rank assumes paired differences are symmetrically distributed around the median; exact zero differences use Pratt (1959) zero-inclusion ranking.",
    ));
}

fn populate_ks_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &TwoSampleKsResult, fmt: SheetFormatCtx) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let alpha = conf.alpha();
    res.tabs.push(
        KvTabBuilder::new("ks-two-sample", "Kolmogorov-Smirnov test", ResultTabKind::MainSummary, format!("{lbl_a} vs. {lbl_b}"))
            .p("P value", out.p_value, p_fmt, alpha)
            .num("Kolmogorov-Smirnov D", out.d_statistic)
            .boolean("Exact P used", out.exact_p_used)
            .build(),
    );
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "KSD", None, out.d_statistic);
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: None,
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });
    res.checklist.push(checklist_item(
        "Two-sample empirical CDF test",
        ChecklistStatus::Pass,
        Some(format!("D = {:.4}", out.d_statistic)),
        "Sensitive to differences in location, spread, and distributional shape.",
    ));
}

fn populate_one_sample_t_sheet_impl(res: &mut ResultSheet, label: &str, values: Option<&[f64]>, out: &OneSampleTResult, fmt: SheetFormatCtx) {
    let SheetFormatCtx { conf, p_fmt } = fmt;
    res.tabs.push(
        KvTabBuilder::new("one-sample-t", "One-sample t test", ResultTabKind::MainSummary, label)
            .num("Hypothetical mean", out.hypothetical_mean)
            .num("Sample mean", out.sample_mean)
            .num("Discrepancy", out.discrepancy)
            .num("t statistic", out.t_statistic)
            .num("df", out.df)
            .p("P value", out.p_value, p_fmt, conf.alpha())
            .ci(format!("{}% CI of discrepancy", conf.percent()), out.ci.0, out.ci.1)
            .opt_num("Hedges' g", out.hedges_g)
            .build(),
    );
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);

    if let Some(raw_vals) = values {
        let (status, metric, explanation) = if out.n < 5 {
            (
                ChecklistStatus::Warn,
                Some(format!("n = {}", out.n)),
                format!("Sample size (n = {}) is too small to assess Gaussian normality reliably; assumption test is inconclusive.", out.n),
            )
        } else if let Ok(sw) = shapiro_wilk(raw_vals, conf.alpha())
            && !sw.passed
        {
            (
                ChecklistStatus::Warn,
                Some(format!("n = {}, Shapiro-Wilk W = {:.4}, p = {:.4}", out.n, sw.statistic, sw.p_value)),
                String::from("Shapiro-Wilk test indicates significant departure from Gaussian normality."),
            )
        } else {
            (
                ChecklistStatus::Pass,
                Some(format!("n = {}", out.n)),
                String::from("One-sample t test assumes independent observations from an approximately Gaussian population."),
            )
        };
        res.checklist.push(checklist_item("One-sample Gaussian assumption", status, metric, explanation));
    } else {
        res.checklist.push(checklist_item(
            "Pre-summarized input mode",
            ChecklistStatus::Info,
            Some(format!("n = {}", out.n)),
            "Moments were reconstructed from entered Mean, Spread, and N; Gaussian normality tests require raw replicates.",
        ));
    }
}

fn populate_one_sample_ratio_t_sheet(res: &mut ResultSheet, label: &str, values: &[f64], out: &OneSampleRatioTResult, fmt: SheetFormatCtx) {
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let pct = conf.percent();
    res.tabs.push(
        KvTabBuilder::new("one-sample-ratio-t", "One-sample ratio t test", ResultTabKind::MainSummary, label)
            .num("Hypothetical geometric mean", out.hypothetical_geo_mean)
            .num("Geometric mean", out.geometric_mean)
            .num("Geometric SD factor", out.geometric_sd_factor)
            .ci(format!("{pct}% CI of geometric mean"), out.geometric_mean_ci.0, out.geometric_mean_ci.1)
            .num("Geometric mean ratio (GM / Hypothetical)", out.ratio_gm_over_hypothetical)
            .ci(format!("{pct}% CI of ratio"), out.ratio_ci.0, out.ratio_ci.1)
            .num("Mean of ln(Y)", out.mean_ln)
            .num("Discrepancy (ln scale)", out.discrepancy_ln)
            .ci(format!("{pct}% CI of ln discrepancy"), out.ci_ln.0, out.ci_ln.1)
            .num("t statistic", out.t_statistic)
            .num("df", out.df)
            .p("P value", out.p_value, p_fmt, conf.alpha())
            .opt_num("Hedges' g (ln scale)", out.hedges_g)
            .footnote("One-sample ratio t-test evaluates Student's t on ln(Y) against ln(hypothetical geometric mean) and back-transforms the geometric mean, ratio, and asymmetric confidence intervals.")
            .build(),
    );
    push_scalar(res, "PValue", None, out.p_value);
    push_scalar(res, "t", None, out.t_statistic);
    push_scalar(res, "GeometricMean", None, out.geometric_mean);
    push_scalar(res, "GeometricMeanRatio", None, out.ratio_gm_over_hypothetical);

    let logs: Vec<f64> = values.iter().map(|v| v.ln()).collect();
    let (status, metric, explanation) = if out.n < 5 {
        (
            ChecklistStatus::Warn,
            Some(format!("n = {}", out.n)),
            format!("Sample size (n = {}) is too small to assess lognormal normality reliably; assumption test is inconclusive.", out.n),
        )
    } else if let Ok(sw) = shapiro_wilk(&logs, conf.alpha())
        && !sw.passed
    {
        (
            ChecklistStatus::Warn,
            Some(format!("n = {}, Shapiro-Wilk(ln) W = {:.4}, p = {:.4}", out.n, sw.statistic, sw.p_value)),
            String::from("Shapiro-Wilk test on ln(Y) indicates significant departure from a lognormal distribution."),
        )
    } else {
        (
            ChecklistStatus::Pass,
            Some(format!("n = {}", out.n)),
            String::from("One-sample ratio t test assumes strictly positive independent observations from an approximately lognormal population."),
        )
    };
    res.checklist.push(checklist_item("One-sample lognormal assumption", status, metric, explanation));
}

fn populate_paired_sign_sheet(res: &mut ResultSheet, pair: GroupPairMeta<'_>, out: &PairedSignTestResult, fmt: SheetFormatCtx) {
    let GroupPairMeta { idx_a, idx_b, lbl_a, lbl_b } = pair;
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let pair_hdr = format!("{lbl_a} vs. {lbl_b}");
    res.tabs.push(
        KvTabBuilder::new("paired-sign-test", "Paired sign test", ResultTabKind::MainSummary, pair_hdr.clone())
            .p("P value", out.p_value, p_fmt, conf.alpha())
            .num("Median difference (A - B)", out.median_difference)
            .opt_ci(format!("{}% CI of median difference", conf.percent()), out.ci)
            .opt_num("Median CI actual confidence (%)", out.ci_actual_confidence_percent)
            .num("Positive differences (n+)", out.n_positive as f64)
            .num("Negative differences (n-)", out.n_negative as f64)
            .num("Zero ties (n0)", out.n_ties as f64)
            .num("Effective pairs (n+ + n-)", out.n_nonzero as f64)
            .build(),
    );

    let pos_prop = if out.n_nonzero > 0 { Some((out.n_positive as f64) / (out.n_nonzero as f64)) } else { None };
    res.tabs.push(
        KvTabBuilder::new("paired-sign-diagnostics", "Paired sign test diagnostics", ResultTabKind::Diagnostics, pair_hdr)
            .num("Total matched pairs", out.n_total as f64)
            .num("Effective non-tie pairs", out.n_nonzero as f64)
            .num("Dropped exact zero ties", out.n_ties as f64)
            .opt_num("Positive sign proportion", pos_prop)
            .num("Null binomial probability (p0)", 0.5)
            .build(),
    );

    push_scalar(res, "median_difference", None, out.median_difference);
    push_scalar(res, "p_value", None, out.p_value);
    push_scalar(res, "n_positive", None, out.n_positive as f64);
    push_scalar(res, "n_negative", None, out.n_negative as f64);
    push_scalar(res, "n_ties", None, out.n_ties as f64);

    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: idx_a,
        group_b: idx_b,
        label_a: lbl_a.to_owned(),
        label_b: lbl_b.to_owned(),
        estimate_diff: Some(out.median_difference),
        raw_p_or_q: out.p_value,
        is_fdr_q: false,
        significant: out.p_value <= conf.alpha(),
    });

    let status = if out.n_nonzero < 6 || out.n_ties > 0 { ChecklistStatus::Info } else { ChecklistStatus::Pass };
    res.checklist.push(checklist_item(
        "Paired sign test assumptions",
        status,
        Some(format!("n+ = {}, n- = {}, ties = {}", out.n_positive, out.n_negative, out.n_ties)),
        "Exact binomial sign test evaluates sign concordance without assuming normality or symmetry; exact zero ties are excluded from the binomial denominator.",
    ));
}

fn populate_one_sample_w_sheet(res: &mut ResultSheet, label: &str, out: &crate::column::OneSampleWilcoxonResult, fmt: SheetFormatCtx) {
    let SheetFormatCtx { conf, p_fmt } = fmt;
    let mut tab = KvTabBuilder::new("one-sample-w", "One-sample Wilcoxon test", ResultTabKind::MainSummary, label)
        .num("Hypothetical median", out.hypothetical_median)
        .num("Sample median", out.sample_median)
        .num("W statistic (W+ - W-)", out.w_stat)
        .p("P value", out.p_value, p_fmt, conf.alpha())
        .num("Hodges-Lehmann pseudomedian", out.pseudomedian_hl);
    let hl_warn = format_hl_ci_row_and_warn(
        &mut tab,
        &mut res.checklist,
        "pseudomedian",
        out.pseudomedian_ci,
        out.ci_actual_confidence_percent,
        wilcoxon_max_achievable_coverage_percent(out.n_total),
        conf,
    );
    if let Some(msg) = hl_warn {
        tab.push_footnote(msg);
    }
    res.tabs.push(tab.build());
    push_scalar(res, "PValue", None, out.p_value);
    res.checklist.push(checklist_item(
        "One-sample symmetry",
        ChecklistStatus::Info,
        Some(format!("n_nonzero = {}", out.n_nonzero)),
        "One-sample Wilcoxon test assumes observations are symmetrically distributed around the population median.",
    ));
}
