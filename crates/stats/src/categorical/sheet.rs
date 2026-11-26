//! `ResultSheet` builders for contingency table tests, parts-of-whole goodness of fit, exact
//! binomial tests, `2 x 2` effect ratios, and inter-rater kappa agreement.
//!
//! References:
//! - Pearson, K. (1900). On the criterion that a given system of deviations from the probable in the
//!   case of a correlated system of variables is such that it can be reasonably supposed to have
//!   arisen from random sampling. *Philosophical Magazine*, 50(302), 157-175.
//!   `doi:10.1080/14786440009463897`.
//! - Yates, F. (1934). Contingency tables involving small numbers and the chi-squared test.
//!   *Supplement to the Journal of the Royal Statistical Society*, 1(2), 217-235. `doi:10.2307/2983604`.
//! - Wilks, S. S. (1938). The large-sample distribution of the likelihood ratio for testing composite
//!   hypotheses. *Annals of Mathematical Statistics*, 9(1), 60-62. `doi:10.1214/aoms/1177732360`.
//! - Cochran, W. G. (1954). Some methods for strengthening the common chi-squared tests.
//!   *Biometrics*, 10(4), 417-451. `doi:10.2307/3001616`.
//! - McNemar, Q. (1947). Note on the sampling error of the difference between correlated proportions
//!   or percentages. *Psychometrika*, 12(2), 153-157. `doi:10.1007/bf02295996`.
//! - Sterne, T. E. (1954). Some remarks on confidence or fiducial limits. *Biometrika*, 41(1-2),
//!   275-278. `doi:10.1093/biomet/41.1-2.275`.
//! - Cohen, J. (1960). A coefficient of agreement for nominal scales. *Educational and Psychological
//!   Measurement*, 20(1), 37-46. `doi:10.1177/001316446002000104`.
//! - Fleiss, J. L. (1971). Measuring nominal scale agreement among many raters. *Psychological
//!   Bulletin*, 76(5), 378-382. `doi:10.1037/h0031619`.

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, ContingencyMethod, ContingencySpec, DataSheet, KappaWeighting, PValueFormat, ResultCell, ResultSheet,
    ResultTabKind, SubcolumnMode, TableArchetype,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::chi2_sf;

use super::binomial::exact_binomial_test;
use super::contingency::{
    BARNARD_MAX_GRID_CELLS, BARNARD_MAX_TOTAL, ReducedContingencyTable, analyze_reduced_contingency, barnard_exact_2x2, checked_sum_u64,
    checked_table_margins, fisher_exact_rxc, mcnemar_detailed, reduce_zero_margin_table,
};
use super::kappa::{cohens_kappa, fleiss_kappa};
use super::ratios_nnt::analyze_2x2_ratios;
use super::types::{BarnardExactResult, ContingencyResult, KappaResult};
use crate::common::{
    KvTabBuilder, TabBuilder, checklist_item, ci_cell, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell, p_cell_at_alpha, push_scalar,
    sanitize_result_sheet_floats, text_cell, try_resolve_selected_indices,
};
use crate::error::StatsError;

fn validate_and_run_barnard_if_requested(counts: &[Vec<u64>], method: ContingencyMethod) -> Result<Option<BarnardExactResult>, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    match method {
        ContingencyMethod::McNemarPaired if n_rows != 2 || n_cols != 2 => {
            Err(StatsError::InvalidParameter { name: "mcnemar_dimensions", value: n_rows.max(n_cols) as f64 })
        }
        ContingencyMethod::CochranArmitageTrend if !((n_rows == 2 && n_cols >= 3) || (n_cols == 2 && n_rows >= 3)) => {
            let bad = if n_rows == 2 || n_cols == 2 { n_rows.max(n_cols) } else { n_rows.min(n_cols) };
            Err(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: bad as f64 })
        }
        ContingencyMethod::BarnardExact => {
            if n_rows != 2 || n_cols != 2 || counts.iter().any(|r| r.len() != 2) {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_dimensions", value: n_rows.max(n_cols) as f64 });
            }
            let n1 = counts[0][0]
                .checked_add(counts[0][1])
                .ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let n2 = counts[1][0]
                .checked_add(counts[1][1])
                .ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let total = n1.checked_add(n2).ok_or(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: f64::INFINITY })?;
            let grid_cells = n1.saturating_add(1).saturating_mul(n2.saturating_add(1));
            if total > BARNARD_MAX_TOTAL || grid_cells > BARNARD_MAX_GRID_CELLS {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: total as f64 });
            }
            let br = barnard_exact_2x2(counts[0][0], counts[0][1], counts[1][0], counts[1][1])?;
            if br.used_asymptotic_fallback {
                return Err(StatsError::InvalidParameter { name: "barnard_exact_max_total", value: total as f64 });
            }
            Ok(Some(br))
        }
        _ => Ok(None),
    }
}

struct ContingencyPrimaryOutcome {
    name: &'static str,
    stat: Option<f64>,
    df: Option<f64>,
    p_value: f64,
    footnote: Option<&'static str>,
}

impl ContingencyPrimaryOutcome {
    fn plain(name: &'static str, stat: Option<f64>, df: Option<f64>, p_value: f64) -> Self {
        Self { name, stat, df, p_value, footnote: None }
    }
}

fn resolve_contingency_primary(
    out: &mut ResultSheet,
    sheet: &DataSheet,
    counts: &[Vec<u64>],
    res: &ContingencyResult,
    barnard_res: Option<&BarnardExactResult>,
    method: ContingencyMethod,
) -> Result<ContingencyPrimaryOutcome, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    match method {
        ContingencyMethod::PearsonChiSquare { yates_continuity } => {
            let yates_applied = yates_continuity && n_rows == 2 && n_cols == 2;
            let lbl = if yates_applied { "Chi-square (Yates)" } else { "Pearson chi-square" };
            let footnote = if yates_continuity && !yates_applied {
                Some("Yates' continuity correction applies only to 2 x 2 tables; uncorrected Pearson chi-square is reported.")
            } else {
                None
            };
            Ok(ContingencyPrimaryOutcome { name: lbl, stat: Some(res.chi_square), df: Some(res.df), p_value: res.chi_square_p, footnote })
        }
        ContingencyMethod::LikelihoodRatioG2 => {
            Ok(ContingencyPrimaryOutcome::plain("Likelihood-ratio G2", Some(res.g_squared), Some(res.df), res.g_squared_p))
        }
        ContingencyMethod::FisherExact => {
            if n_rows == 2 && n_cols == 2 {
                let fp = res.fisher_exact_p.ok_or(StatsError::InvalidParameter { name: "fisher_exact_max_total", value: res.total_count as f64 })?;
                Ok(ContingencyPrimaryOutcome::plain("Fisher's exact test", None, None, fp))
            } else {
                let mc_p = match res.fisher_exact_p {
                    Some(fp) => fp,
                    None => {
                        let p = fisher_exact_rxc(counts)?;
                        push_scalar(out, "Fisher's exact P", None, p);
                        p
                    }
                };
                push_scalar(out, "Fisher's exact Monte Carlo draws", None, 10_000.0);
                let note = "R x C Fisher's exact test uses a deterministic Monte Carlo permutation test (10,000 draws; max N = 50,000).";
                Ok(ContingencyPrimaryOutcome { name: "Fisher's exact test (Monte Carlo)", stat: None, df: None, p_value: mc_p, footnote: Some(note) })
            }
        }
        ContingencyMethod::BarnardExact => {
            let br = barnard_res.ok_or(StatsError::InvalidParameter { name: "barnard_exact_dimensions", value: n_rows.max(n_cols) as f64 })?;
            Ok(ContingencyPrimaryOutcome::plain("Barnard's exact test", Some(br.statistic), None, br.p_value))
        }
        ContingencyMethod::CochranArmitageTrend => {
            let ca_p = res
                .cochran_armitage_p
                .ok_or(StatsError::InvalidParameter { name: "cochran_armitage_dimensions", value: n_rows.max(n_cols) as f64 })?;
            Ok(ContingencyPrimaryOutcome::plain("Cochran-Armitage trend", res.cochran_armitage_z, None, ca_p))
        }
        ContingencyMethod::McNemarPaired => {
            let err = if n_rows == 2 && n_cols == 2 {
                StatsError::InvalidParameter { name: "mcnemar_max_total", value: res.total_count as f64 }
            } else {
                StatsError::InvalidParameter { name: "mcnemar_dimensions", value: n_rows.max(n_cols) as f64 }
            };
            let mc_p = res.mcnemar_exact_p.ok_or(err)?;
            Ok(ContingencyPrimaryOutcome::plain("McNemar's test", res.mcnemar_chi2, Some(1.0), mc_p))
        }
        ContingencyMethod::CohenKappa { .. } | ContingencyMethod::FleissKappa | ContingencyMethod::ExactBinomial => {
            Err(StatsError::IncompatibleTable { analysis: "contingency analysis", actual: sheet.archetype.kind() })
        }
    }
}

/// Execute a [`ContingencySpec`] against a `Contingency`, `PartsOfWhole`, or binary `Column` [`DataSheet`].
pub fn run_contingency_analysis(sheet: &DataSheet, spec: &AnalysisSpec, cont_spec: &ContingencySpec) -> Result<ResultSheet, StatsError> {
    let mut result = run_contingency_analysis_inner(sheet, spec, cont_spec)?;
    sanitize_result_sheet_floats(&mut result)?;
    Ok(result)
}

fn append_reduced_margins_note(out: &mut ResultSheet, reduced: &ReducedContingencyTable) {
    if reduced.dropped_rows == 0 && reduced.dropped_cols == 0 {
        return;
    }
    let msg = format!(
        "Dropped {} empty (all-zero) row(s) and {} empty (all-zero) column(s); analysis evaluated on the reduced {} x {} table.",
        reduced.dropped_rows, reduced.dropped_cols, reduced.n_rows, reduced.n_cols
    );
    if let Some(tab) = out.tabs.iter_mut().find(|t| t.id == "contingency-main") {
        tab.footnotes.push(msg.clone());
    }
    out.checklist.push(checklist_item(
        "Empty margin reduction",
        ChecklistStatus::Info,
        Some(format!("{} row(s), {} col(s) dropped -> {} x {}", reduced.dropped_rows, reduced.dropped_cols, reduced.n_rows, reduced.n_cols)),
        msg,
    ));
}

fn append_mcnemar_outputs(out: &mut ResultSheet, b: u64, c: u64, confidence: ConfidenceLevel, p_fmt: PValueFormat) -> Result<(), StatsError> {
    let mc = mcnemar_detailed(b, c, confidence)?;
    push_scalar(out, "McNemar chi-square", None, mc.chi_square_yates);
    push_scalar(out, "McNemar exact P", None, mc.exact_p_value);
    if let Some(or_val) = mc.discordant_odds_ratio {
        push_scalar(out, "Discordant odds ratio", None, or_val);
    }
    if let Some((or_lo, or_hi)) = mc.discordant_or_ci {
        push_scalar(out, "Discordant OR CI lower", None, or_lo);
        push_scalar(out, "Discordant OR CI upper", None, or_hi);
    }
    let or_cell = match mc.discordant_odds_ratio {
        Some(v) if v.is_finite() => num_cell(v),
        Some(_) => ResultCell::Text(String::from("∞")),
        None => ResultCell::Empty,
    };
    let disc_total = mc.b.checked_add(mc.c).ok_or(StatsError::InvalidParameter { name: "mcnemar_discordant_total", value: f64::INFINITY })?;
    let disc_prop_cell = if disc_total > 0 { num_cell((mc.b as f64) / (disc_total as f64)) } else { ResultCell::Empty };
    out.tabs.push(
        TabBuilder::new("contingency-mcnemar", "McNemar discordant pairs & exact odds ratio", ResultTabKind::ParametersAndConfidenceIntervals)
            .headers([String::from("Estimate"), format!("{}% exact CI", confidence.percent())])
            .row("Discordant pair ratio (b / c)", [text_cell(format!("{} / {}", mc.b, mc.c)), ResultCell::Empty])
            .row("Discordant odds ratio (Clopper-Pearson)", [or_cell, opt_ci_cell(mc.discordant_or_ci)])
            .row("Discordant proportion b / (b + c)", [disc_prop_cell, ci_cell(mc.discordant_p_ci.0, mc.discordant_p_ci.1)])
            .row("McNemar chi-square (Yates)", [num_cell(mc.chi_square_yates), ResultCell::Empty])
            .row("McNemar exact P value", [p_cell_at_alpha(mc.exact_p_value, p_fmt, confidence.alpha()), ResultCell::Empty])
            .build(),
    );
    Ok(())
}

fn opt_ratio_cell(val: Option<f64>) -> ResultCell {
    match val {
        Some(v) if v.is_finite() => num_cell(v),
        Some(v) if v.is_infinite() && v.is_sign_positive() => ResultCell::Text(String::from("∞")),
        _ => ResultCell::Empty,
    }
}

fn append_2x2_ratios_tab(
    out: &mut ResultSheet,
    counts: &[Vec<u64>],
    cont_spec: &ContingencySpec,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    let r2x2 = analyze_2x2_ratios(counts[0][0], counts[0][1], counts[1][0], counts[1][1], confidence)?;
    if let Some(or_val) = r2x2.odds_ratio {
        push_scalar(out, "Odds ratio", None, or_val);
    }
    if let Some(rr_val) = r2x2.relative_risk {
        push_scalar(out, "Relative risk", None, rr_val);
    }
    push_scalar(out, "Risk difference", None, r2x2.risk_difference);
    if let Some(nnt_v) = r2x2.nnt_value {
        push_scalar(out, "NNT", None, nnt_v);
    }

    let mut tab = TabBuilder::new("contingency-ratios", "Effect sizes & diagnostic proportions", ResultTabKind::ParametersAndConfidenceIntervals)
        .headers([String::from("Estimate"), format!("{}% CI", confidence.percent())]);
    if cont_spec.compute_odds_and_risk_ratios {
        tab.push_row("Odds ratio (Baptista-Pike CI)", [opt_ratio_cell(r2x2.odds_ratio), opt_ci_cell(r2x2.or_baptista_pike_ci)])
            .push_row("Relative risk (Koopman CI)", [opt_ratio_cell(r2x2.relative_risk), opt_ci_cell(r2x2.rr_koopman_ci)])
            .push_row("Risk difference (Newcombe CI)", [num_cell(r2x2.risk_difference), opt_ci_cell(r2x2.rd_newcombe_ci)])
            .push_row("Number needed to treat (NNT)", [text_cell(r2x2.nnt_point_label.clone()), text_cell(r2x2.nnt_ci_label.clone())])
            .push_row("NNT magnitude (1 / |RD|)", [opt_num_cell(r2x2.nnt_value), opt_ci_cell(r2x2.nnt_ci_bounds)]);
    }
    if cont_spec.compute_sensitivity_specificity {
        tab.push_row("Sensitivity (Wilson CI)", [opt_num_cell(r2x2.sensitivity), opt_ci_cell(r2x2.sensitivity_wilson_ci)])
            .push_row("Specificity (Wilson CI)", [opt_num_cell(r2x2.specificity), opt_ci_cell(r2x2.specificity_wilson_ci)])
            .push_row("PPV (Wilson CI)", [num_cell(r2x2.ppv), ci_cell(r2x2.ppv_wilson_ci.0, r2x2.ppv_wilson_ci.1)])
            .push_row("NPV (Wilson CI)", [num_cell(r2x2.npv), ci_cell(r2x2.npv_wilson_ci.0, r2x2.npv_wilson_ci.1)]);
    }
    out.tabs.push(tab.build());
    Ok(())
}

fn run_contingency_analysis_inner(sheet: &DataSheet, spec: &AnalysisSpec, cont_spec: &ContingencySpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let confidence = spec.confidence_level;
    let p_fmt = spec.p_value_format;

    if matches!(cont_spec.method, ContingencyMethod::ExactBinomial) {
        return run_exact_binomial_analysis(sheet, spec, cont_spec, out, confidence, p_fmt);
    }

    if let TableArchetype::PartsOfWhole(pow) = &sheet.archetype {
        let incompat = |analysis: &'static str| Err(StatsError::IncompatibleTable { analysis, actual: sheet.archetype.kind() });
        return match cont_spec.method {
            ContingencyMethod::ExactBinomial => run_exact_binomial_analysis(sheet, spec, cont_spec, out, confidence, p_fmt),
            ContingencyMethod::PearsonChiSquare { yates_continuity: false } | ContingencyMethod::LikelihoodRatioG2 => {
                run_parts_of_whole_gof(pow, spec, cont_spec, &mut out, confidence, p_fmt)
            }
            ContingencyMethod::PearsonChiSquare { yates_continuity: true } => incompat("Yates' continuity-corrected chi-square"),
            ContingencyMethod::FisherExact => incompat("Fisher's exact test"),
            ContingencyMethod::BarnardExact => incompat("Barnard's exact test"),
            ContingencyMethod::CochranArmitageTrend => incompat("Cochran-Armitage trend test"),
            ContingencyMethod::McNemarPaired => incompat("McNemar's paired test"),
            ContingencyMethod::CohenKappa { .. } => incompat("Cohen's kappa"),
            ContingencyMethod::FleissKappa => incompat("Fleiss' kappa"),
        };
    }

    let TableArchetype::Contingency(table) = &sheet.archetype else {
        return Err(StatsError::IncompatibleTable { analysis: "contingency analysis", actual: sheet.archetype.kind() });
    };
    table.validate_counts()?;
    let selected_cols = try_resolve_selected_indices(&spec.selected_data_sets, table.data_sets.len())?;
    if selected_cols.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let n_rows = table.row_count();
    let n_cols = selected_cols.len();
    let mut counts = vec![vec![0_u64; n_cols]; n_rows];
    for (r, row_vec) in counts.iter_mut().enumerate() {
        for (c_out, &c_src) in selected_cols.iter().enumerate() {
            row_vec[c_out] = table.count_at(r, c_src)?.unwrap_or(0);
        }
    }

    match cont_spec.method {
        ContingencyMethod::CohenKappa { weighting } => {
            return run_cohen_kappa_sheet(&counts, weighting, confidence, p_fmt, out);
        }
        ContingencyMethod::FleissKappa => {
            return run_fleiss_kappa_sheet(&counts, confidence, p_fmt, out);
        }
        ContingencyMethod::McNemarPaired => {
            return run_mcnemar_sheet(&counts, cont_spec, confidence, p_fmt, out);
        }
        _ => {}
    }

    let reduced = reduce_zero_margin_table(&counts)?;
    let yates = matches!(cont_spec.method, ContingencyMethod::PearsonChiSquare { yates_continuity: true });
    let barnard_res = validate_and_run_barnard_if_requested(&reduced.counts, cont_spec.method)?;

    let res = analyze_reduced_contingency(&reduced, yates, confidence)?;
    push_scalar(&mut out, "Chi-square", None, res.chi_square);
    push_scalar(&mut out, "Chi-square P value", None, res.chi_square_p);
    push_scalar(&mut out, "G-squared", None, res.g_squared);
    push_scalar(&mut out, "Cramer's V", None, res.cramers_v);
    if let Some(fp) = res.fisher_exact_p {
        push_scalar(&mut out, "Fisher's exact P", None, fp);
    }
    if let Some(br) = &barnard_res {
        push_scalar(&mut out, "Barnard's exact P", None, br.p_value);
        push_scalar(&mut out, "Barnard nuisance pi", None, br.nuisance_pi);
        push_scalar(&mut out, "Barnard Z-pooled", None, br.statistic);
    }

    let primary = resolve_contingency_primary(&mut out, sheet, &reduced.counts, &res, barnard_res.as_ref(), cont_spec.method)?;

    let mut headers = vec![String::from("Statistic"), String::from("df"), String::from("P value")];
    let mut row_main = vec![opt_num_cell(primary.stat), opt_num_cell(primary.df), p_cell_at_alpha(primary.p_value, p_fmt, confidence.alpha())];
    if cont_spec.compute_phi_and_cramers_v {
        headers.push(String::from("Cramer's V / Phi"));
        headers.push(format!("{}% CI", confidence.percent()));
        row_main.push(num_cell(res.phi.unwrap_or(res.cramers_v)));
        row_main.push(opt_ci_cell(res.cramers_v_ci));
    }
    let mut main_tab =
        TabBuilder::new("contingency-main", "Contingency analysis", ResultTabKind::MainSummary).headers(headers).row(primary.name, row_main);
    if let Some(note) = primary.footnote {
        main_tab.push_footnote(note);
    }
    out.tabs.push(main_tab.build());
    append_reduced_margins_note(&mut out, &reduced);

    if reduced.n_rows == 2 && reduced.n_cols == 2 && (cont_spec.compute_odds_and_risk_ratios || cont_spec.compute_sensitivity_specificity) {
        append_2x2_ratios_tab(&mut out, &reduced.counts, cont_spec, confidence)?;
    }

    if let Some(br) = &barnard_res {
        let status = if br.used_asymptotic_fallback { ChecklistStatus::Info } else { ChecklistStatus::Pass };
        out.checklist.push(checklist_item(
            "Barnard unconditional nuisance maximization",
            status,
            Some(format!("max P = {:.4} at pi = {:.4}, Z_pooled = {:.4}", br.p_value, br.nuisance_pi, br.statistic)),
            if br.used_asymptotic_fallback {
                "Table total exceeded the exact unconditional grid cap; asymptotic pooled score P value was reported."
            } else {
                "Unconditional exact test maximized the two-binomial null probability across nuisance pi in [0, 1] without fixing column margins."
            },
        ));
    }

    let expected_ok =
        res.min_expected_count >= 5.0 || ((res.n_rows > 2 || res.n_cols > 2) && res.cells_lt5_fraction <= 0.2 && res.min_expected_count >= 1.0);
    out.checklist.push(checklist_item(
        "Minimum expected cell count",
        if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("min E = {:.2}, {:.1}% cells < 5", res.min_expected_count, 100.0 * res.cells_lt5_fraction)),
        if expected_ok {
            "Expected cell counts satisfy Cochran's guidelines for asymptotic chi-square approximation."
        } else {
            "Small expected counts detected (< 5); large-sample chi-square asymptotic accuracy is reduced."
        },
    ));

    Ok(out)
}

/// Dispatch McNemar's paired `2 x 2` test directly on the unreduced table so valid zero-margin
/// paired tables are not rejected and asymmetric `>= 3 x 3` tables cannot be misaligned into `2 x 2`.
fn run_mcnemar_sheet(
    counts: &[Vec<u64>],
    cont_spec: &ContingencySpec,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
    mut out: ResultSheet,
) -> Result<ResultSheet, StatsError> {
    let n_rows = counts.len();
    let n_cols = counts.first().map_or(0, Vec::len);
    if n_rows != 2 || n_cols != 2 || counts.iter().any(|r| r.len() != 2) {
        return Err(StatsError::InvalidParameter { name: "mcnemar_dimensions", value: n_rows.max(n_cols) as f64 });
    }
    let (r_sums, c_sums, total_count) = checked_table_margins(counts)?;
    if total_count == 0 {
        return Err(StatsError::EmptyInput);
    }

    let mc = mcnemar_detailed(counts[0][1], counts[1][0], confidence)?;
    let all_margins_positive = !r_sums.contains(&0) && !c_sums.contains(&0);

    let mut headers = vec![String::from("Statistic"), String::from("df"), String::from("P value")];
    let mut row_main = vec![num_cell(mc.chi_square_yates), num_cell(1.0), p_cell_at_alpha(mc.exact_p_value, p_fmt, confidence.alpha())];

    let mut aux_res: Option<ContingencyResult> = None;
    if all_margins_positive {
        let reduced = ReducedContingencyTable {
            counts: counts.to_vec(),
            active_rows: vec![0, 1],
            active_cols: vec![0, 1],
            r_sums: r_sums.clone(),
            c_sums: c_sums.clone(),
            total_count,
            n_rows: 2,
            n_cols: 2,
            dropped_rows: 0,
            dropped_cols: 0,
        };
        let res = analyze_reduced_contingency(&reduced, false, confidence)?;
        push_scalar(&mut out, "Chi-square", None, res.chi_square);
        push_scalar(&mut out, "Chi-square P value", None, res.chi_square_p);
        push_scalar(&mut out, "G-squared", None, res.g_squared);
        push_scalar(&mut out, "Cramer's V", None, res.cramers_v);
        if let Some(fp) = res.fisher_exact_p {
            push_scalar(&mut out, "Fisher's exact P", None, fp);
        }
        aux_res = Some(res);
    }

    if cont_spec.compute_phi_and_cramers_v {
        headers.push(String::from("Cramer's V / Phi"));
        headers.push(format!("{}% CI", confidence.percent()));
        if let Some(res) = &aux_res {
            row_main.push(num_cell(res.phi.unwrap_or(res.cramers_v)));
            row_main.push(opt_ci_cell(res.cramers_v_ci));
        } else {
            row_main.push(ResultCell::Empty);
            row_main.push(ResultCell::Empty);
        }
    }

    let mut main_tab =
        TabBuilder::new("contingency-main", "Contingency analysis", ResultTabKind::MainSummary).headers(headers).row("McNemar's test", row_main);
    if !all_margins_positive {
        main_tab.push_footnote(
            "One or more row or column margins are zero; paired 2 x 2 cell alignment was preserved for McNemar's test while auxiliary chi-square and Phi/Cramer's V were omitted.",
        );
    }
    out.tabs.push(main_tab.build());

    append_mcnemar_outputs(&mut out, counts[0][1], counts[1][0], confidence, p_fmt)?;

    if (cont_spec.compute_odds_and_risk_ratios || cont_spec.compute_sensitivity_specificity) && r_sums[0] > 0 && r_sums[1] > 0 {
        append_2x2_ratios_tab(&mut out, counts, cont_spec, confidence)?;
    }

    if let Some(res) = &aux_res {
        let expected_ok = res.min_expected_count >= 5.0;
        out.checklist.push(checklist_item(
            "Minimum expected cell count",
            if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("min E = {:.2}, {:.1}% cells < 5", res.min_expected_count, 100.0 * res.cells_lt5_fraction)),
            if expected_ok {
                "Expected cell counts satisfy Cochran's guidelines for asymptotic chi-square approximation."
            } else {
                "Small expected counts detected (< 5); large-sample chi-square asymptotic accuracy is reduced."
            },
        ));
    } else {
        out.checklist.push(checklist_item(
            "Paired table zero margin",
            ChecklistStatus::Info,
            Some(format!("b = {}, c = {}", counts[0][1], counts[1][0])),
            "McNemar's paired test evaluated directly on the unreduced 2 x 2 discordant pairs (b, c) without dropping empty margins.",
        ));
    }

    Ok(out)
}

fn push_kappa_outputs(
    out: &mut ResultSheet,
    res: &KappaResult,
    row_label: &str,
    diag_rows: &[(&str, f64)],
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) {
    push_scalar(out, "kappa", None, res.kappa);
    push_scalar(out, "se", None, res.se);
    push_scalar(out, "z", None, res.z_statistic);
    push_scalar(out, "p_value", None, res.p_value);
    push_scalar(out, "ci_lower", None, res.ci.0);
    push_scalar(out, "ci_upper", None, res.ci.1);
    push_scalar(out, "observed_agreement", None, res.observed_agreement);
    push_scalar(out, "expected_agreement", None, res.expected_agreement);

    out.tabs.push(
        TabBuilder::new("contingency-kappa", "Inter-rater agreement (kappa)", ResultTabKind::MainSummary)
            .headers([
                String::from("Kappa"),
                String::from("ASE (SE)"),
                format!("{}% CI", confidence.percent()),
                String::from("Z"),
                String::from("P value"),
            ])
            .row(
                row_label,
                [
                    num_cell(res.kappa),
                    num_cell(res.se),
                    ci_cell(res.ci.0, res.ci.1),
                    num_cell(res.z_statistic),
                    p_cell_at_alpha(res.p_value, p_fmt, confidence.alpha()),
                ],
            )
            .build(),
    );

    let mut diag = KvTabBuilder::new("contingency-kappa-diagnostics", "Agreement decomposition diagnostics", ResultTabKind::Diagnostics, "Value");
    for &(label, val) in diag_rows {
        diag.push_num(label, val);
    }
    out.tabs.push(diag.build());
}

fn run_cohen_kappa_sheet(
    counts: &[Vec<u64>],
    weighting: KappaWeighting,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
    mut out: ResultSheet,
) -> Result<ResultSheet, StatsError> {
    let res = cohens_kappa(counts, weighting, confidence)?;
    let k = counts.len();
    let (_, _, total) = checked_table_margins(counts)?;
    let label = match weighting {
        KappaWeighting::Unweighted => "Cohen's kappa (unweighted)",
        KappaWeighting::Linear => "Cohen's kappa (linear weighted)",
        KappaWeighting::Quadratic => "Cohen's kappa (quadratic weighted)",
    };
    push_kappa_outputs(
        &mut out,
        &res,
        label,
        &[
            ("Observed agreement (P_o)", res.observed_agreement),
            ("Chance-expected agreement (P_e)", res.expected_agreement),
            ("Chance-corrected denominator (1 - P_e)", 1.0 - res.expected_agreement),
            ("Subjects / total paired ratings (N)", total as f64),
            ("Categories (K)", k as f64),
        ],
        confidence,
        p_fmt,
    );
    let excess_ok = (1.0 - res.expected_agreement) > 0.05;
    out.checklist.push(checklist_item(
        "Marginal agreement & category balance",
        if excess_ok && total >= 10 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("N = {total}, K = {k}, P_o = {:.4}, P_e = {:.4}", res.observed_agreement, res.expected_agreement)),
        if excess_ok {
            "Cohen's kappa compares observed concordance against marginal chance agreement on square K x K classifications."
        } else {
            "High marginal concentration yields P_e near 1.0 (kappa paradox regime), limiting sensitivity to discordant pairs."
        },
    ));
    Ok(out)
}

fn run_fleiss_kappa_sheet(
    counts: &[Vec<u64>],
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
    mut out: ResultSheet,
) -> Result<ResultSheet, StatsError> {
    let res = fleiss_kappa(counts, confidence)?;
    let n = counts.len();
    let k = counts.first().map_or(0, Vec::len);
    let m = counts.first().map_or(Ok(0), |r| checked_sum_u64(r, "fleiss_raters_per_subject"))?;
    push_kappa_outputs(
        &mut out,
        &res,
        "Fleiss' multi-rater kappa",
        &[
            ("Mean observed agreement (P_bar)", res.observed_agreement),
            ("Chance-expected agreement (P_e)", res.expected_agreement),
            ("Chance-corrected denominator (1 - P_e)", 1.0 - res.expected_agreement),
            ("Subjects (N)", n as f64),
            ("Categories (K)", k as f64),
            ("Raters per subject (m)", m as f64),
        ],
        confidence,
        p_fmt,
    );
    let excess_ok = (1.0 - res.expected_agreement) > 0.05;
    out.checklist.push(checklist_item(
        "Multi-rater design & chance agreement",
        if excess_ok && n >= 5 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("N = {n} subjects, m = {m} raters, K = {k}, P_bar = {:.4}, P_e = {:.4}", res.observed_agreement, res.expected_agreement)),
        if excess_ok {
            "Fleiss' kappa evaluates chance-corrected nominal agreement with equal rater counts per subject."
        } else {
            "Category prevalence concentration drives P_e near 1.0, amplifying sensitivity to small disagreement shifts."
        },
    ));
    Ok(out)
}

fn resolve_parts_of_whole_series_index(pow: &curvebook_doc::PartsOfWholeTable, selected: &[usize]) -> Result<usize, StatsError> {
    let n_series = pow.data_sets.len();
    if n_series == 0 {
        return Err(StatsError::EmptyInput);
    }
    for &idx in selected {
        if idx >= n_series {
            return Err(StatsError::MissingColumn { index: idx });
        }
    }
    if selected.is_empty() {
        if n_series == 1 { Ok(0) } else { Err(StatsError::InvalidParameter { name: "selected_data_sets", value: n_series as f64 }) }
    } else if selected.len() == 1 {
        Ok(selected[0])
    } else {
        Err(StatsError::InvalidParameter { name: "selected_data_sets", value: selected.len() as f64 })
    }
}

fn validate_integer_count(v: f64, name: &'static str) -> Result<u64, StatsError> {
    const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0;
    if !v.is_finite() || v < 0.0 || v.fract() != 0.0 || v >= TWO_POW_64 {
        return Err(StatsError::InvalidParameter { name, value: v });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v as u64)
}

fn extract_exact_binomial_counts(sheet: &DataSheet, spec: &AnalysisSpec) -> Result<(u64, u64), StatsError> {
    match &sheet.archetype {
        TableArchetype::PartsOfWhole(pow) => {
            let series_idx = resolve_parts_of_whole_series_index(pow, &spec.selected_data_sets)?;
            let slices = pow.fractions_for_series(series_idx)?;
            if slices.len() != 2 {
                return Err(StatsError::DimensionMismatch { expected: 2, actual: slices.len() });
            }
            let k = validate_integer_count(slices[0].value, "binomial_count")?;
            let f = validate_integer_count(slices[1].value, "binomial_count")?;
            let n = k.checked_add(f).ok_or(StatsError::InvalidParameter { name: "exact_binomial_total", value: f64::INFINITY })?;
            if n == 0 {
                return Err(StatsError::EmptyInput);
            }
            Ok((k, n))
        }
        TableArchetype::Contingency(table) => {
            table.validate_counts()?;
            let selected_cols = try_resolve_selected_indices(&spec.selected_data_sets, table.data_sets.len())?;
            if selected_cols.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            let n_rows = table.row_count();
            let n_cols = selected_cols.len();
            let (k, f) = match (n_rows, n_cols) {
                (1, 2) => (table.count_at(0, selected_cols[0])?.unwrap_or(0), table.count_at(0, selected_cols[1])?.unwrap_or(0)),
                (2, 1) => (table.count_at(0, selected_cols[0])?.unwrap_or(0), table.count_at(1, selected_cols[0])?.unwrap_or(0)),
                _ => return Err(StatsError::DimensionMismatch { expected: 2, actual: n_rows * n_cols }),
            };
            let n = k.checked_add(f).ok_or(StatsError::InvalidParameter { name: "exact_binomial_total", value: f64::INFINITY })?;
            if n == 0 {
                return Err(StatsError::EmptyInput);
            }
            Ok((k, n))
        }
        TableArchetype::Column(col_table) => {
            if !matches!(col_table.subcolumn_mode, SubcolumnMode::Single | SubcolumnMode::RawReplicates(1)) {
                return Err(StatsError::RequiresRawReplicates);
            }
            let indices = try_resolve_selected_indices(&spec.selected_data_sets, col_table.data_sets.len())?;
            if !spec.selected_data_sets.is_empty() && indices.len() != 1 {
                return Err(StatsError::InvalidParameter { name: "selected_data_sets", value: indices.len() as f64 });
            }
            let &col_idx = indices.first().ok_or(StatsError::EmptyInput)?;
            let values = col_table.active_column_values(col_idx);
            if values.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            let mut k = 0_u64;
            for v in &values {
                if *v == 1.0 {
                    k += 1;
                } else if *v != 0.0 {
                    return Err(StatsError::InvalidParameter { name: "binary_value", value: *v });
                }
            }
            Ok((k, values.len() as u64))
        }
        _ => Err(StatsError::IncompatibleTable { analysis: "exact binomial test", actual: sheet.archetype.kind() }),
    }
}

fn run_exact_binomial_analysis(
    sheet: &DataSheet,
    spec: &AnalysisSpec,
    cont_spec: &ContingencySpec,
    mut out: ResultSheet,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let (k, n) = extract_exact_binomial_counts(sheet, spec)?;
    let p0 = cont_spec.null_proportion.unwrap_or(0.5);
    let res = exact_binomial_test(k, n, p0, confidence)?;

    push_scalar(&mut out, "proportion", None, res.proportion);
    push_scalar(&mut out, "p_value", None, res.p_value_sterne);
    push_scalar(&mut out, "p_value_doubled_tail", None, res.p_value_doubled_tail);
    push_scalar(&mut out, "p_value_lower", None, res.p_value_lower);
    push_scalar(&mut out, "p_value_upper", None, res.p_value_upper);
    push_scalar(&mut out, "null_proportion", None, res.null_proportion);
    push_scalar(&mut out, "clopper_pearson_lower", None, res.clopper_pearson_ci.0);
    push_scalar(&mut out, "clopper_pearson_upper", None, res.clopper_pearson_ci.1);
    push_scalar(&mut out, "wilson_lower", None, res.wilson_ci.0);
    push_scalar(&mut out, "wilson_upper", None, res.wilson_ci.1);
    push_scalar(&mut out, "agresti_coull_lower", None, res.agresti_coull_ci.0);
    push_scalar(&mut out, "agresti_coull_upper", None, res.agresti_coull_ci.1);
    push_scalar(&mut out, "successes", None, res.successes as f64);
    push_scalar(&mut out, "total", None, res.total as f64);

    let alpha = confidence.alpha();
    out.tabs.push(
        TabBuilder::new("exact-binomial-main", "Exact binomial test", ResultTabKind::MainSummary)
            .headers([
                "Successes (k)",
                "Trials (n)",
                "Observed p",
                "Null p0",
                "Exact P (Sterne)",
                "Exact P (doubled tail)",
                "Lower P(X <= k)",
                "Upper P(X >= k)",
            ])
            .row(
                "Binomial proportion vs null",
                [
                    num_cell(res.successes as f64),
                    num_cell(res.total as f64),
                    num_cell(res.proportion),
                    num_cell(res.null_proportion),
                    p_cell_at_alpha(res.p_value_sterne, p_fmt, alpha),
                    p_cell_at_alpha(res.p_value_doubled_tail, p_fmt, alpha),
                    p_cell_at_alpha(res.p_value_lower, p_fmt, alpha),
                    p_cell_at_alpha(res.p_value_upper, p_fmt, alpha),
                ],
            )
            .build(),
    );

    let ci_row = |ci: (f64, f64)| [num_cell(res.proportion), ci_cell(ci.0, ci.1), num_cell(ci.0), num_cell(ci.1)];
    out.tabs.push(
        TabBuilder::new("exact-binomial-cis", "Binomial proportion confidence intervals", ResultTabKind::ParametersAndConfidenceIntervals)
            .headers([String::from("Observed p"), format!("{}% CI", confidence.percent()), String::from("Lower bound"), String::from("Upper bound")])
            .row("Clopper-Pearson (exact)", ci_row(res.clopper_pearson_ci))
            .row("Wilson score", ci_row(res.wilson_ci))
            .row("Agresti-Coull", ci_row(res.agresti_coull_ci))
            .build(),
    );

    let np0 = (n as f64) * p0;
    let nq0 = (n as f64) * (1.0 - p0);
    let min_expected = np0.min(nq0);
    out.tabs.push(
        KvTabBuilder::new("exact-binomial-diagnostics", "Binomial model diagnostics", ResultTabKind::Diagnostics, "Value")
            .num("Expected successes under H0 (n * p0)", np0)
            .num("Expected failures under H0 (n * (1 - p0))", nq0)
            .num("Minimum expected cell count min(n*p0, n*(1-p0))", min_expected)
            .build(),
    );

    out.checklist.push(checklist_item(
        "Exact discrete binomial enumeration",
        if min_expected >= 5.0 { ChecklistStatus::Pass } else { ChecklistStatus::Info },
        Some(format!("k = {k}, n = {n}, p0 = {p0:.4}, min(np0, nq0) = {min_expected:.2}")),
        if min_expected >= 5.0 {
            "Exact binomial probabilities were summed directly under Binomial(n, p0); normal-approximation conditions are also met."
        } else {
            "Small expected count min(np0, nq0) < 5 causes normal approximations to diverge from discrete binomial tails; exact binomial enumeration evaluates the discrete distribution directly."
        },
    ));

    Ok(out)
}

fn run_parts_of_whole_gof(
    pow: &curvebook_doc::PartsOfWholeTable,
    spec: &AnalysisSpec,
    cont_spec: &ContingencySpec,
    out: &mut ResultSheet,
    confidence: ConfidenceLevel,
    p_fmt: PValueFormat,
) -> Result<ResultSheet, StatsError> {
    let series_idx = resolve_parts_of_whole_series_index(pow, &spec.selected_data_sets)?;
    let slices = pow.fractions_for_series(series_idx)?;
    let k = slices.len();
    if k < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "parts-of-whole goodness-of-fit", required: 2, actual: k });
    }
    let mut total_u64 = 0_u64;
    for slice in &slices {
        let count = validate_integer_count(slice.value, "goodness_of_fit_count")?;
        total_u64 = total_u64.checked_add(count).ok_or(StatsError::InvalidParameter { name: "goodness_of_fit_total", value: f64::INFINITY })?;
    }
    if total_u64 == 0 {
        return Err(StatsError::EmptyInput);
    }
    let total = total_u64 as f64;

    let (proportions, is_custom) = if let Some(weights) = &cont_spec.expected_proportions {
        if weights.len() != k {
            return Err(StatsError::DimensionMismatch { expected: k, actual: weights.len() });
        }
        if let Some(&bad) = weights.iter().find(|&&w| !w.is_finite() || w < 0.0) {
            return Err(StatsError::InvalidParameter { name: "expected_proportions", value: bad });
        }
        let sum: f64 = weights.iter().sum();
        if !sum.is_finite() || sum <= 0.0 {
            return Err(StatsError::InvalidParameter { name: "expected_proportions_sum", value: sum });
        }
        (weights.iter().map(|&w| w / sum).collect::<Vec<_>>(), true)
    } else {
        (vec![1.0 / (k as f64); k], false)
    };

    let mut chi2 = 0.0_f64;
    let mut g2 = 0.0_f64;
    let mut active_categories = 0_usize;
    let mut min_expected = f64::INFINITY;
    let mut lt5_count = 0_usize;
    let mut cat_tab =
        TabBuilder::new("parts-gof-categories", "Category observed vs expected proportions", ResultTabKind::ParametersAndConfidenceIntervals)
            .headers(["Observed", "Observed proportion", "Expected proportion", "Expected count", "Pearson residual"]);

    for (idx, (slice, &p_exp)) in slices.iter().zip(proportions.iter()).enumerate() {
        let obs = slice.value;
        let exp = total * p_exp;
        if exp <= 0.0 && obs > 0.0 {
            return Err(StatsError::InvalidParameter { name: "expected_proportions_zero_for_nonzero_observed", value: obs });
        }
        let pearson_resid = if exp > 0.0 {
            active_categories += 1;
            min_expected = min_expected.min(exp);
            if exp < 5.0 {
                lt5_count += 1;
            }
            let diff = obs - exp;
            chi2 += (diff * diff) / exp;
            if obs > 0.0 {
                g2 += 2.0 * obs * (obs / exp).ln();
            }
            diff / exp.sqrt()
        } else {
            0.0
        };
        let label = if slice.label.trim().is_empty() { format!("Category {}", idx + 1) } else { slice.label.clone() };
        cat_tab.push_row(label, [num_cell(obs), num_cell(obs / total), num_cell(p_exp), num_cell(exp), num_cell(pearson_resid)]);
    }

    if active_categories < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "parts-of-whole goodness-of-fit", required: 2, actual: active_categories });
    }
    let is_g2 = matches!(cont_spec.method, ContingencyMethod::LikelihoodRatioG2);
    let chi2 = if chi2 <= 0.0 { 0.0 } else { chi2 };
    let g2 = if g2 <= 0.0 { 0.0 } else { g2 };
    let df = (active_categories - 1) as f64;
    let p_val = chi2_sf(chi2, df)?;
    let g2_p = chi2_sf(g2, df)?;
    push_scalar(out, "Goodness-of-fit chi-square", None, chi2);
    push_scalar(out, "Likelihood-ratio G-squared", None, g2);
    push_scalar(out, "P value", None, if is_g2 { g2_p } else { p_val });
    push_scalar(out, "Chi-square P value", None, p_val);
    push_scalar(out, "G-squared P value", None, g2_p);

    let title = match (is_g2, is_custom) {
        (true, true) => "Likelihood-ratio G2 goodness of fit (custom expected proportions)",
        (true, false) => "Likelihood-ratio G2 goodness of fit (uniform expected)",
        (false, true) => "Chi-square goodness of fit (custom expected proportions)",
        (false, false) => "Chi-square goodness of fit (uniform expected)",
    };
    let alpha = confidence.alpha();
    let chi2_row = [num_cell(chi2), num_cell(df), p_cell_at_alpha(p_val, p_fmt, alpha)];
    let g2_row = [num_cell(g2), num_cell(df), p_cell_at_alpha(g2_p, p_fmt, alpha)];
    let gof_tab = TabBuilder::new("parts-gof", title, ResultTabKind::MainSummary).headers(["Statistic", "df", "P value"]);
    out.tabs.push(
        if is_g2 {
            gof_tab.row("Likelihood-ratio G2", g2_row).row("Pearson chi-square", chi2_row)
        } else {
            gof_tab.row("Pearson chi-square", chi2_row).row("Likelihood-ratio G2", g2_row)
        }
        .build(),
    );
    out.tabs.push(cat_tab.build());

    let expected_ok = min_expected >= 5.0 || (active_categories > 2 && (lt5_count as f64) / (active_categories as f64) <= 0.2 && min_expected >= 1.0);
    out.checklist.push(checklist_item(
        "Minimum expected category count",
        if expected_ok { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
        Some(format!("min E = {min_expected:.2}, {lt5_count}/{active_categories} categories < 5")),
        if expected_ok {
            "Expected category counts satisfy guidelines for asymptotic chi-square goodness-of-fit approximation."
        } else {
            "Small expected category counts (< 5) reduce asymptotic accuracy of Pearson chi-square and G2 tests."
        },
    ));

    Ok(out.clone())
}
