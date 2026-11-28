//! `ResultSheet` builder for factorial specs: two-way, three-way, repeated-measures two-way,
//! mixed-model REML on incomplete repeated-measures data, and multifactor *N*-way ANOVA.
//!
//! References:
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistStatus, DataSheet, FactorialAnovaSpec, MixedModelDfMethod, ObservationAccounting, ObservationAccountingKind,
    PValueFormat, RepeatedMeasuresDesign, ResidualCovariance, ResultCell, ResultSheet, ResultTab, ResultTabKind, SubcolumnMode, SumOfSquaresType,
    TableArchetype,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::SummaryMoments;

use crate::anova::factorial::{
    FactorialObservations, build_xtx_pinv_and_proj, finalize_term_p_values, fit_factorial_anova_with_model,
    fit_factorial_anova_with_model_and_subsets,
};
use crate::anova::posthoc::{FactorialPostHocContext, RemlContrastModel, run_factorial_multiplicity_with_options};
use crate::anova::reml::RemlContrastCore;
use crate::anova::results::{
    FactorialAnovaResult, FactorialAnovaTerm, RmSphericityCorrection, RmThreeWayAnovaResult, RmTwoWayAnovaResult, SphericityResult,
};
use crate::anova::rm_two_way::{
    apply_rm_sphericity_p, fit_rm_three_way_anova, fit_rm_two_way_anova_with_posthoc, fit_rm_two_way_split_plot_with_posthoc,
};
use crate::anova::sheet::one_way::{
    RemlEffectDesign, build_reml_effect_design, emit_multi_vc_rm_reml_with_policy, emit_random_intercept_reml_with_policy, emit_unstructured_rm_reml,
};
use crate::common::{
    TabBuilder, append_ignored_text_checklist_and_footnote, append_listwise_filtered_checklist_and_footnote, checklist_item,
    enforce_summary_mode_matrix, init_result_sheet, is_ignored_non_numeric_cell, num_cell, opt_num_cell, p_cell_at_alpha, push_scalar,
    record_omitted_row_reason, sanitize_result_sheet_floats, text_cell, try_resolve_selected_indices, unpaired_observation_accounting,
};
use crate::error::StatsError;
use crate::formula_design::resolve_factorial_formula_subsets;
use crate::multivariate::spearman_correlation;

/// Execute a [`FactorialAnovaSpec`] (Two-way, Three-way `2x2xK`, Repeated-Measures Two-way,
/// Mixed-Effects REML on missing RM data, or Multifactor `N`-way ANOVA).
pub fn run_factorial_anova(sheet: &DataSheet, spec: &AnalysisSpec, fac_spec: &FactorialAnovaSpec) -> Result<ResultSheet, StatsError> {
    enforce_summary_mode_matrix(sheet, &AnalysisKind::FactorialAnova(fac_spec.clone()))?;
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;

    let outcome = match &sheet.archetype {
        TableArchetype::Grouped(grouped) => run_grouped_factorial(grouped, &mut out, &spec.selected_data_sets, fac_spec, p_fmt, confidence.alpha())?,
        TableArchetype::MultipleVariables(mv) => {
            if !spec.selected_data_sets.is_empty() {
                let _ = try_resolve_selected_indices(&spec.selected_data_sets, mv.variables.len())?;
            }
            run_multifactor_mv(mv, &mut out, fac_spec, p_fmt, confidence.alpha())?
        }
        other => return Err(StatsError::IncompatibleTable { analysis: "factorial ANOVA", actual: other.kind() }),
    };
    let GroupedFactorialOutcome {
        mut result,
        observations: obs,
        post_hoc,
        aux_tabs,
        estimator_note,
        factor_source_indices,
        accounting,
        ignored_non_numeric,
    } = outcome;

    if result.terms.iter().any(|t| t.df > 0.0 && t.p_value.is_none()) {
        finalize_term_p_values(&mut result)?;
    }

    for term in &result.terms {
        if let Some(f) = term.f_statistic {
            push_scalar(&mut out, format!("{}: F", term.name), None, f);
        }
        if let Some(p) = term.p_value {
            push_scalar(&mut out, format!("{}: P", term.name), None, p);
        }
    }
    push_scalar(&mut out, "MS residual", None, result.ms_residual);

    let mut headers = vec![String::from("SS"), String::from("df"), String::from("MS"), String::from("F"), String::from("P value")];
    if fac_spec.compute_effect_sizes {
        headers.extend([String::from("Partial eta squared"), String::from("Eta squared")]);
    }
    let tab_title = if fac_spec.three_way { "Three-way ANOVA" } else { "Factorial ANOVA" };
    let mut main_tab = TabBuilder::new("factorial-anova", tab_title, ResultTabKind::AnovaTable).headers(headers);
    for t in &result.terms {
        let mut r = vec![
            num_cell(t.ss),
            num_cell(t.df),
            num_cell(t.ms),
            opt_num_cell(t.f_statistic),
            t.p_value.map_or(ResultCell::Empty, |p| p_cell_at_alpha(p, p_fmt, confidence.alpha())),
        ];
        if fac_spec.compute_effect_sizes {
            r.push(opt_num_cell(t.partial_eta_squared));
            r.push(opt_num_cell(t.eta_squared));
        }
        main_tab.push_row(t.name.clone(), r);
    }
    let mut r_res =
        vec![num_cell(result.ss_residual), num_cell(result.df_residual), num_cell(result.ms_residual), ResultCell::Empty, ResultCell::Empty];
    let mut r_tot = vec![num_cell(result.ss_total), num_cell(result.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if fac_spec.compute_effect_sizes {
        r_res.extend([ResultCell::Empty, ResultCell::Empty]);
        r_tot.extend([ResultCell::Empty, ResultCell::Empty]);
    }
    main_tab.push_row("Residual", r_res).push_row("Total", r_tot);
    if let Some(note) = estimator_note {
        main_tab.push_footnote(note);
    }
    out.tabs.push(main_tab.build());
    out.tabs.extend(aux_tabs);

    if fac_spec.emit_consolidated_data_tab {
        out.tabs.push(build_consolidated_data_tab(&obs));
    }

    if fac_spec.heteroscedasticity_test {
        append_spearman_heteroscedasticity(&mut out, &obs, &post_hoc, p_fmt, confidence.alpha())?;
    }

    let mut effective_mult = fac_spec.multiplicity.clone();
    match crate::multiplicity::validate_multiplicity_family_spec(&effective_mult)? {
        crate::multiplicity::MultiplicityFamilySelection::AllPairs => {}
        crate::multiplicity::MultiplicityFamilySelection::ControlVsAll(raw_ctrl) => {
            if let Some(target_f) = fac_spec.control_factor {
                if target_f >= obs.factor_levels.len() {
                    return Err(StatsError::InvalidParameter { name: "control_factor", value: target_f as f64 });
                }
                if let Some(Some(src_indices)) = factor_source_indices.get(target_f) {
                    let pos = src_indices
                        .iter()
                        .position(|&idx| idx == raw_ctrl)
                        .ok_or(StatsError::InvalidParameter { name: "control_column", value: raw_ctrl as f64 })?;
                    effective_mult = effective_mult.with_control_column(pos);
                }
            }
        }
        crate::multiplicity::MultiplicityFamilySelection::SelectedPairs(raw_pairs) => {
            if let Some(target_f) = fac_spec.control_factor {
                if target_f >= obs.factor_levels.len() {
                    return Err(StatsError::InvalidParameter { name: "control_factor", value: target_f as f64 });
                }
                if let Some(Some(src_indices)) = factor_source_indices.get(target_f) {
                    let remapped = crate::multiplicity::resolve_selected_pair_positions(raw_pairs, |idx| src_indices.iter().position(|&c| c == idx))?;
                    effective_mult = effective_mult.with_selected_pairs(remapped);
                }
            }
        }
    }

    run_factorial_multiplicity_with_options(
        &mut out,
        &obs,
        &post_hoc,
        &result,
        &effective_mult,
        fac_spec.control_factor,
        fac_spec.comparison_family_mode,
        &fac_spec.preselected_pairs,
        fac_spec.swap_direction,
        p_fmt,
        confidence,
    )?;

    out.checklist.push(checklist_item(
        "Cell balance & sums of squares",
        if result.unbalanced { ChecklistStatus::Info } else { ChecklistStatus::Pass },
        Some(format!("Total N = {}, df_resid = {:.1}", result.total_n, result.df_residual)),
        if result.unbalanced {
            match result.ss_type {
                SumOfSquaresType::TypeIII => "Unbalanced cell sizes detected; Type III partial sums of squares (effect coding) used.",
                SumOfSquaresType::TypeI => "Unbalanced design with Type I sequential sums of squares (order-dependent).",
            }
        } else {
            "Balanced cell sizes; Type I and Type III sums of squares coincide."
        },
    ));

    let anova_ctx = if fac_spec.three_way {
        "Three-way ANOVA"
    } else if matches!(sheet.archetype, TableArchetype::MultipleVariables(_)) {
        "Multifactor ANOVA"
    } else {
        "Two-way ANOVA"
    };
    if accounting.kind == ObservationAccountingKind::ListwiseAnova && accounting.rows_omitted() > 0 {
        append_listwise_filtered_checklist_and_footnote(&mut out, anova_ctx, accounting.rows_omitted(), accounting.analyzed);
    }
    append_ignored_text_checklist_and_footnote(&mut out, anova_ctx, &ignored_non_numeric);
    out.observation_accounting = Some(accounting);

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

pub(in crate::anova) struct GroupedFactorialOutcome {
    pub(in crate::anova) result: FactorialAnovaResult,
    pub(in crate::anova) observations: FactorialObservations,
    pub(in crate::anova) post_hoc: FactorialPostHocContext,
    pub(in crate::anova) aux_tabs: Vec<ResultTab>,
    pub(in crate::anova) estimator_note: Option<String>,
    pub(in crate::anova) factor_source_indices: Vec<Option<Vec<usize>>>,
    pub(in crate::anova) accounting: ObservationAccounting,
    pub(in crate::anova) ignored_non_numeric: Vec<(String, usize)>,
}

fn scan_grouped_cells_audit(grouped: &curvebook_doc::GroupedTable, selected_cols: &[usize]) -> (usize, usize, Vec<(String, usize)>) {
    let mut excluded_cells = 0_usize;
    let mut non_numeric_total = 0_usize;
    let mut ignored = Vec::new();
    for &c in selected_cols {
        let Some(ds) = grouped.data_sets.get(c) else {
            continue;
        };
        let mut col_non_num = 0_usize;
        for row in &ds.cells {
            for cell in row {
                if cell.excluded {
                    excluded_cells += 1;
                } else if is_ignored_non_numeric_cell(cell) {
                    col_non_num += 1;
                }
            }
        }
        if col_non_num > 0 {
            non_numeric_total += col_non_num;
            ignored.push((ds.title.clone(), col_non_num));
        }
    }
    (excluded_cells, non_numeric_total, ignored)
}

fn scan_grouped_rm_cell_accounting(
    grouped: &curvebook_doc::GroupedTable,
    active_rows: &[usize],
    active_cols: &[usize],
    active_subs: &[usize],
    selected_cols: &[usize],
    kind: ObservationAccountingKind,
    analyzed: usize,
) -> (ObservationAccounting, Vec<(String, usize)>) {
    let (all_excluded, all_non_num, ignored) = scan_grouped_cells_audit(grouped, selected_cols);
    let mut omitted_excluded = 0_usize;
    let mut omitted_non_numeric = 0_usize;
    let mut omitted_missing = 0_usize;
    for &orig_s in active_subs {
        for &orig_r in active_rows {
            for &orig_c in active_cols {
                match grouped.observation(orig_r, orig_c, orig_s) {
                    Some(cell) if cell.active_number().is_some() => {}
                    Some(cell) if cell.excluded => omitted_excluded += 1,
                    Some(cell) if is_ignored_non_numeric_cell(cell) => omitted_non_numeric += 1,
                    _ => omitted_missing += 1,
                }
            }
        }
    }
    let acc = ObservationAccounting {
        omitted_excluded,
        omitted_non_numeric,
        omitted_missing,
        ignored_non_numeric_cells: all_non_num,
        excluded_cells: all_excluded,
        ..ObservationAccounting::new(kind, analyzed)
    };
    (acc, ignored)
}

pub(in crate::anova) fn build_rm_sphericity_tab(
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<Option<ResultTab>, StatsError> {
    let design = rm_res.design;
    let res = &rm_res.anova;
    let sph = &rm_res.sphericities;
    let inter_name = format!("{row_name} × {col_name}");
    let entries: [(&str, Option<&SphericityResult>, usize); 3] = match design {
        RepeatedMeasuresDesign::BothFactorsMatched => {
            [(row_name, sph.row.as_ref(), 0), (col_name, sph.column.as_ref(), 1), (inter_name.as_str(), sph.interaction.as_ref(), 2)]
        }
        RepeatedMeasuresDesign::ColumnFactorMatched => {
            [(col_name, sph.column.as_ref(), 1), (inter_name.as_str(), sph.column.as_ref(), 2), ("", None, usize::MAX)]
        }
        RepeatedMeasuresDesign::RowFactorMatched => {
            [(row_name, sph.row.as_ref(), 0), (inter_name.as_str(), sph.row.as_ref(), 2), ("", None, usize::MAX)]
        }
        RepeatedMeasuresDesign::None => return Ok(None),
    };

    let mut tab = TabBuilder::new("rm-sphericity", "Repeated-measures sphericity", ResultTabKind::Diagnostics).headers([
        "Mauchly W",
        "Mauchly chi2",
        "df",
        "Mauchly P",
        "G-G epsilon",
        "H-F epsilon",
        "G-G P",
        "H-F P",
    ]);
    let mut row_count = 0_usize;
    for (label, s_opt, term_idx) in entries {
        let (Some(s), Some(term), Some(&df_err)) = (s_opt, res.terms.get(term_idx), rm_res.term_error_dfs.get(term_idx)) else {
            continue;
        };
        if s.mauchly_df <= 0.0 {
            continue;
        }
        let f_val = term.f_statistic.unwrap_or(0.0);
        let p_gg = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::GreenhouseGeisser)?;
        let p_hf = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::HuynhFeldt)?;
        tab.push_row(
            label,
            [
                num_cell(s.mauchly_w),
                num_cell(s.mauchly_chi2),
                num_cell(s.mauchly_df),
                p_cell_at_alpha(s.mauchly_p, p_fmt, alpha),
                num_cell(s.greenhouse_geisser_eps),
                num_cell(s.huynh_feldt_eps),
                p_cell_at_alpha(p_gg, p_fmt, alpha),
                p_cell_at_alpha(p_hf, p_fmt, alpha),
            ],
        );
        row_count += 1;
    }
    Ok((row_count > 0).then(|| tab.build()))
}

pub(in crate::anova) fn push_rm_sphericity_outputs(
    out: &mut ResultSheet,
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<Option<ResultTab>, StatsError> {
    let sph = &rm_res.sphericities;
    let inter_name = format!("{row_name} × {col_name}");
    let items: [(&str, Option<&SphericityResult>); 3] =
        [(row_name, sph.row.as_ref()), (col_name, sph.column.as_ref()), (&inter_name, sph.interaction.as_ref())];
    let mut worst_gg = 1.0_f64;
    let mut summary_parts = Vec::new();
    for (name, s_opt) in items {
        let Some(s) = s_opt else {
            continue;
        };
        worst_gg = worst_gg.min(s.greenhouse_geisser_eps);
        push_scalar(out, format!("{name}: G-G epsilon"), None, s.greenhouse_geisser_eps);
        push_scalar(out, format!("{name}: H-F epsilon"), None, s.huynh_feldt_eps);
        let mauchly_lbl = if s.mauchly_p.is_finite() { format!("{:.4}", s.mauchly_p) } else { String::from("n/a") };
        summary_parts.push(format!("{name}: eps_GG={:.4}, eps_HF={:.4}, P={mauchly_lbl}", s.greenhouse_geisser_eps, s.huynh_feldt_eps));
    }
    if !summary_parts.is_empty() {
        out.checklist.push(checklist_item(
            "Sphericity (RM two-way)",
            if worst_gg < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(summary_parts.join("; ")),
            "Greenhouse-Geisser and Huynh-Feldt epsilons summarize covariance sphericity across within-subject factors.",
        ));
    }
    build_rm_sphericity_tab(row_name, col_name, rm_res, p_fmt, alpha)
}

fn build_consolidated_data_tab(obs: &FactorialObservations) -> ResultTab {
    let mut headers = obs.factor_names.clone();
    headers.push(String::from("Value"));
    let mut tab = TabBuilder::new("consolidated-data", "Consolidated Data", ResultTabKind::Diagnostics).headers(headers);
    for (idx, (lvls, y)) in obs.rows.iter().enumerate() {
        let mut row = Vec::with_capacity(lvls.len() + 1);
        for (f_idx, &lvl) in lvls.iter().enumerate() {
            let lbl = obs.factor_levels.get(f_idx).and_then(|v| v.get(lvl)).map_or("?", String::as_str);
            row.push(text_cell(lbl));
        }
        row.push(num_cell(*y));
        tab.push_row(format!("Obs {}", idx + 1), row);
    }
    tab.build()
}

fn snap_near_ties(values: &mut [f64], tol: f64) {
    if values.len() < 2 || !tol.is_finite() || tol <= 0.0 {
        return;
    }
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut i = 0;
    while i < order.len() {
        let anchor = values[order[i]];
        let mut j = i + 1;
        while j < order.len() && (values[order[j]] - anchor).abs() <= tol {
            values[order[j]] = anchor;
            j += 1;
        }
        i = j;
    }
}

fn append_spearman_heteroscedasticity(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    post_hoc: &FactorialPostHocContext,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<(), StatsError> {
    if obs.rows.len() < 3 {
        return Ok(());
    }
    let mut fitted = Vec::with_capacity(obs.rows.len());
    let mut abs_resid = Vec::with_capacity(obs.rows.len());
    let mut y_scale = 1.0_f64;
    for (lvls, y) in &obs.rows {
        y_scale = y_scale.max(y.abs());
        let yhat = post_hoc.predict_cell(lvls);
        fitted.push(yhat);
        abs_resid.push((y - yhat).abs());
    }
    // Snap sub-1e-11 solver rounding jitter so identical cell means and symmetric residuals
    // receive exact mathematical mid-ranks independent of contrast coding or row order.
    let tie_tol = 1e-11 * y_scale;
    snap_near_ties(&mut fitted, tie_tol);
    snap_near_ties(&mut abs_resid, tie_tol);
    if let Ok(sp) = spearman_correlation(&fitted, &abs_resid) {
        push_scalar(out, "Heteroscedasticity Spearman rho", None, sp.r);
        push_scalar(out, "Heteroscedasticity P", None, sp.p_value);
        let homoscedastic = sp.p_value >= alpha;
        let status_text = if homoscedastic { "Homoscedasticity not rejected" } else { "Heteroscedasticity detected" };
        out.tabs.push(
            TabBuilder::new("heteroscedasticity", "Homoscedasticity Test (Spearman)", ResultTabKind::Diagnostics)
                .headers(["Spearman rho", "t statistic", "df", "P value", "N", "Status"])
                .row(
                    "Predicted vs. |Residual|",
                    [
                        num_cell(sp.r),
                        num_cell(sp.statistic),
                        opt_num_cell(sp.df),
                        p_cell_at_alpha(sp.p_value, p_fmt, alpha),
                        num_cell(sp.n as f64),
                        text_cell(status_text),
                    ],
                )
                .footnote("Spearman rank correlation between fitted cell means and absolute residuals |y - y_hat| tests monotonic variance-mean heteroscedasticity.")
                .build(),
        );
        out.checklist.push(checklist_item(
            "Homoscedasticity (Spearman rank test)",
            if homoscedastic { ChecklistStatus::Pass } else { ChecklistStatus::Warn },
            Some(format!("rho = {:.4}, P = {:.4}", sp.r, sp.p_value)),
            if homoscedastic {
                "No significant monotonic relationship between fitted values and residual magnitude was detected."
            } else {
                "Absolute residuals correlate significantly with fitted cell means, indicating heteroscedasticity."
            },
        ));
    }
    Ok(())
}

fn push_rm_three_way_sphericity_outputs(
    out: &mut ResultSheet,
    rm3: &RmThreeWayAnovaResult,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<Option<ResultTab>, StatsError> {
    let mut tab = TabBuilder::new("rm-sphericity", "Repeated-measures sphericity (three-way)", ResultTabKind::Diagnostics).headers([
        "Mauchly W",
        "Mauchly chi2",
        "df",
        "Mauchly P",
        "G-G epsilon",
        "H-F epsilon",
        "G-G P",
        "H-F P",
    ]);
    let mut row_count = 0_usize;
    let mut worst_gg = 1.0_f64;
    let mut summary_parts = Vec::new();
    for (idx, term) in rm3.anova.terms.iter().enumerate() {
        let Some(Some(s)) = rm3.term_sphericities.get(idx) else {
            continue;
        };
        worst_gg = worst_gg.min(s.greenhouse_geisser_eps);
        push_scalar(out, format!("{}: G-G epsilon", term.name), None, s.greenhouse_geisser_eps);
        push_scalar(out, format!("{}: H-F epsilon", term.name), None, s.huynh_feldt_eps);
        if s.mauchly_df <= 0.0 {
            continue;
        }
        let df_err = rm3.term_error_dfs.get(idx).copied().unwrap_or(rm3.anova.df_residual);
        let f_val = term.f_statistic.unwrap_or(0.0);
        let p_gg = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::GreenhouseGeisser)?;
        let p_hf = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::HuynhFeldt)?;
        let mauchly_lbl = if s.mauchly_p.is_finite() { format!("{:.4}", s.mauchly_p) } else { String::from("n/a") };
        summary_parts.push(format!("{}: eps_GG={:.4}, eps_HF={:.4}, P={mauchly_lbl}", term.name, s.greenhouse_geisser_eps, s.huynh_feldt_eps));
        tab.push_row(
            term.name.clone(),
            [
                num_cell(s.mauchly_w),
                num_cell(s.mauchly_chi2),
                num_cell(s.mauchly_df),
                p_cell_at_alpha(s.mauchly_p, p_fmt, alpha),
                num_cell(s.greenhouse_geisser_eps),
                num_cell(s.huynh_feldt_eps),
                p_cell_at_alpha(p_gg, p_fmt, alpha),
                p_cell_at_alpha(p_hf, p_fmt, alpha),
            ],
        );
        row_count += 1;
    }
    if !summary_parts.is_empty() {
        out.checklist.push(checklist_item(
            "Sphericity (RM three-way)",
            if worst_gg < 0.75 { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(summary_parts.join("; ")),
            "Greenhouse-Geisser and Huynh-Feldt (Lecoutre 1991) epsilons summarize covariance sphericity across within-subject strata.",
        ));
    }
    Ok((row_count > 0).then(|| tab.build()))
}

/// Resolve three-way within-subject flags in canonical `[factor_0, factor_1, factor_2]` axis order:
/// - `GroupedTable`: `[Factor A (factor_1), Factor B (factor_2), Row factor]` matching `factor_names`.
/// - `MultipleVariablesTable`: `[multifactor_columns[0], multifactor_columns[1], multifactor_columns[2]]`.
fn effective_three_way_rm_factors(fac_spec: &FactorialAnovaSpec) -> [bool; 3] {
    if let Some(flags) = fac_spec.three_way_rm_factors {
        return flags;
    }
    match fac_spec.repeated_measures {
        RepeatedMeasuresDesign::None => [false, false, false],
        RepeatedMeasuresDesign::RowFactorMatched => [false, false, true],
        RepeatedMeasuresDesign::ColumnFactorMatched => [true, true, false],
        RepeatedMeasuresDesign::BothFactorsMatched => [true, true, true],
    }
}

fn active_grouped_rows(grouped: &curvebook_doc::GroupedTable, selected_cols: &[usize]) -> Vec<usize> {
    (0..grouped.row_count()).filter(|&r| selected_cols.iter().any(|&c| !grouped.active_replicates(r, c).is_empty())).collect()
}

fn active_grouped_cols(grouped: &curvebook_doc::GroupedTable, selected_cols: &[usize], active_rows: &[usize]) -> Vec<usize> {
    selected_cols.iter().copied().filter(|&c| active_rows.iter().any(|&r| !grouped.active_replicates(r, c).is_empty())).collect()
}

fn active_grouped_subs(grouped: &curvebook_doc::GroupedTable, active_rows: &[usize], active_cols: &[usize], n_subs: usize) -> Vec<usize> {
    (0..n_subs)
        .filter(|&s| {
            active_rows.iter().any(|&r| active_cols.iter().any(|&c| grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_some()))
        })
        .collect()
}

fn grouped_row_levels_for(grouped: &curvebook_doc::GroupedTable, active_rows: &[usize]) -> Vec<String> {
    active_rows.iter().map(|&r| grouped.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect()
}

/// Extract split-plot subject vectors `groups[group][subject][within_level]` for `RowFactorMatched`
/// or `ColumnFactorMatched`, distinguishing genuinely partial subjects (missing repeated measures)
/// from empty subcolumn slots caused by unequal between-subject group sizes.
fn extract_split_plot_groups(
    grouped: &curvebook_doc::GroupedTable,
    active_rows: &[usize],
    active_cols: &[usize],
    n_subs: usize,
    design: RepeatedMeasuresDesign,
) -> (Vec<Vec<Vec<f64>>>, bool, usize, usize) {
    let mut groups = Vec::new();
    let mut has_partial_subject = false;
    let mut observed_in_nonempty = 0_usize;
    let mut required_in_nonempty = 0_usize;
    if design == RepeatedMeasuresDesign::RowFactorMatched {
        let w_len = active_rows.len();
        for &orig_c in active_cols {
            let mut grp_subjs = Vec::new();
            for orig_s in 0..n_subs {
                let mut vals = Vec::with_capacity(w_len);
                for &orig_r in active_rows {
                    if let Some(v) = grouped.observation(orig_r, orig_c, orig_s).and_then(|cell| cell.active_number()) {
                        vals.push(v);
                    }
                }
                if vals.is_empty() {
                    continue;
                }
                observed_in_nonempty += vals.len();
                required_in_nonempty += w_len;
                if vals.len() == w_len {
                    grp_subjs.push(vals);
                } else {
                    has_partial_subject = true;
                }
            }
            groups.push(grp_subjs);
        }
    } else {
        let w_len = active_cols.len();
        for &orig_r in active_rows {
            let mut grp_subjs = Vec::new();
            for orig_s in 0..n_subs {
                let mut vals = Vec::with_capacity(w_len);
                for &orig_c in active_cols {
                    if let Some(v) = grouped.observation(orig_r, orig_c, orig_s).and_then(|cell| cell.active_number()) {
                        vals.push(v);
                    }
                }
                if vals.is_empty() {
                    continue;
                }
                observed_in_nonempty += vals.len();
                required_in_nonempty += w_len;
                if vals.len() == w_len {
                    grp_subjs.push(vals);
                } else {
                    has_partial_subject = true;
                }
            }
            groups.push(grp_subjs);
        }
    }
    (groups, has_partial_subject, observed_in_nonempty, required_in_nonempty)
}

fn count_rm_active_cells(
    grouped: &curvebook_doc::GroupedTable,
    active_rows: &[usize],
    active_cols: &[usize],
    active_subs: &[usize],
) -> (usize, usize) {
    let total = active_rows.len() * active_cols.len() * active_subs.len();
    let mut observed = 0_usize;
    for &r in active_rows {
        for &c in active_cols {
            for &s in active_subs {
                if grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_some() {
                    observed += 1;
                }
            }
        }
    }
    (observed, total)
}

pub(in crate::anova) fn run_grouped_factorial(
    grouped: &curvebook_doc::GroupedTable,
    out: &mut ResultSheet,
    selected: &[usize],
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<GroupedFactorialOutcome, StatsError> {
    if grouped.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let selected_cols = try_resolve_selected_indices(selected, grouped.data_sets.len())?;
    if fac_spec.three_way && selected_cols.len() != 4 {
        return Err(StatsError::InvalidParameter { name: "three_way_column_count", value: selected_cols.len() as f64 });
    }
    if selected_cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial column levels", required: 2, actual: selected_cols.len() });
    }
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };

    // Compact unobserved, blank, or excluded row/column factor levels before effect coding (#30).
    let active_rows = active_grouped_rows(grouped, &selected_cols);
    if active_rows.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    // Three-way 2 x 2 x K path (supports all 8 RM factor combinations; classical balanced RM with
    // per-stratum sphericity when complete and not forced to REML, or mixed-effects REML otherwise).
    if fac_spec.three_way && selected_cols.len() == 4 {
        let rm_flags = effective_three_way_rm_factors(fac_spec);
        let any_rm = rm_flags.iter().any(|&v| v);
        if any_rm {
            let active_subs = active_grouped_subs(grouped, &active_rows, &selected_cols, n_subs);
            if active_subs.is_empty() {
                return Err(StatsError::EmptyInput);
            }
            let (observed_cells, total_cells) = count_rm_active_cells(grouped, &active_rows, &selected_cols, &active_subs);
            let has_missing_selected = observed_cells < total_cells;
            if has_missing_selected
                && !fac_spec.fit_reml_on_missing
                && fac_spec.residual_covariance != ResidualCovariance::Unstructured
                && !fac_spec.mixed_effects_reml
            {
                return Err(StatsError::InsufficientSampleSize {
                    context: "complete repeated-measures cells (fit_reml_on_missing is false)",
                    required: total_cells,
                    actual: observed_cells,
                });
            }
            let use_classical_balanced_rm = !has_missing_selected
                && !fac_spec.mixed_effects_reml
                && fac_spec.residual_covariance != ResidualCovariance::Unstructured
                && (fac_spec.three_way_rm_factors.is_some() || fac_spec.sphericity_correction != RmSphericityCorrection::None)
                && active_rows.len() >= 2
                && active_subs.len() >= 2;
            if use_classical_balanced_rm {
                let grouping = grouped.three_way_grouping.as_ref();
                let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
                let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
                let f3_name = grouped.row_factor_title.clone();
                let f1_levels = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
                let f2_levels = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
                let f3_levels = grouped_row_levels_for(grouped, &active_rows);
                let n_eff_rows = active_rows.len();
                let n_eff_subs = active_subs.len();
                let mut subject_data = vec![vec![vec![vec![0.0_f64; n_eff_rows]; 2]; 2]; n_eff_subs];
                let mut obs = Vec::with_capacity(n_eff_subs * 4 * n_eff_rows);
                for (s_pos, &orig_s) in active_subs.iter().enumerate() {
                    for (r_pos, &orig_r) in active_rows.iter().enumerate() {
                        for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
                            let l1 = c_pos / 2;
                            let l2 = c_pos % 2;
                            let Some(v) = grouped.observation(orig_r, orig_c, orig_s).and_then(|cell| cell.active_number()) else {
                                return Err(StatsError::InsufficientSampleSize {
                                    context: "complete repeated-measures cells",
                                    required: total_cells,
                                    actual: observed_cells,
                                });
                            };
                            subject_data[s_pos][l1][l2][r_pos] = v;
                            obs.push((vec![l1, l2, r_pos], v));
                        }
                    }
                }
                let rm3 = fit_rm_three_way_anova(&subject_data, [&f1_name, &f2_name, &f3_name], rm_flags, fac_spec.sphericity_correction)?;
                let sph_tab = push_rm_three_way_sphericity_outputs(out, &rm3, p_fmt, alpha)?;
                let data = FactorialObservations {
                    factor_names: vec![f1_name, f2_name, f3_name],
                    factor_levels: vec![f1_levels, f2_levels, f3_levels],
                    rows: obs,
                };
                let (_, mut post_model) = fit_factorial_anova_with_model(&data, 3, SumOfSquaresType::TypeIII)?;
                post_model.ms_resid = rm3.anova.ms_residual;
                post_model.df_resid = rm3.anova.df_residual;
                let corr_lbl = match fac_spec.sphericity_correction {
                    RmSphericityCorrection::None => "uncorrected",
                    RmSphericityCorrection::GreenhouseGeisser => "Greenhouse-Geisser corrected",
                    RmSphericityCorrection::HuynhFeldt => "Huynh-Feldt corrected",
                };
                let estimator_desc = format!("Classical balanced three-way RM ANOVA ({corr_lbl})");
                out.checklist.push(checklist_item(
                    "Repeated-measures estimator",
                    ChecklistStatus::Info,
                    Some(estimator_desc.clone()),
                    format!("Fitted classical balanced three-way RM ANOVA with within-subject flags {:?} and {corr_lbl} sphericity.", rm_flags),
                ));
                let (accounting, ignored_non_numeric) = scan_grouped_rm_cell_accounting(
                    grouped,
                    &active_rows,
                    &selected_cols,
                    &active_subs,
                    &selected_cols,
                    ObservationAccountingKind::ListwiseAnova,
                    data.rows.len(),
                );
                return Ok(GroupedFactorialOutcome {
                    result: rm3.anova,
                    observations: data,
                    post_hoc: FactorialPostHocContext::Ols(post_model),
                    aux_tabs: sph_tab.into_iter().collect(),
                    estimator_note: Some(format!("Estimator: {estimator_desc}.")),
                    factor_source_indices: vec![None, None, None],
                    accounting,
                    ignored_non_numeric,
                });
            }
            return run_rm_grouped_reml(grouped, out, &selected_cols, &active_rows, &active_subs, fac_spec, has_missing_selected, &[], p_fmt, alpha);
        }
        let grouping = grouped.three_way_grouping.as_ref();
        let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
        let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
        let f3_name = grouped.row_factor_title.clone();
        let raw_f1 = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
        let raw_f2 = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
        let f3_levels = grouped_row_levels_for(grouped, &active_rows);

        let mut raw_obs = Vec::new();
        let mut obs_a = [false; 2];
        let mut obs_b = [false; 2];
        for (r_pos, &orig_r) in active_rows.iter().enumerate() {
            for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
                let l1 = c_pos / 2;
                let l2 = c_pos % 2;
                for y in grouped.active_replicates(orig_r, orig_c) {
                    obs_a[l1] = true;
                    obs_b[l2] = true;
                    raw_obs.push((l1, l2, r_pos, y));
                }
            }
        }
        let (f1_levels, l1_remap) = compact_binary_levels(&raw_f1, &obs_a);
        let (f2_levels, l2_remap) = compact_binary_levels(&raw_f2, &obs_b);
        let obs = raw_obs.into_iter().map(|(l1, l2, r_pos, y)| (vec![l1_remap[l1], l2_remap[l2], r_pos], y)).collect();
        let data =
            FactorialObservations { factor_names: vec![f1_name, f2_name, f3_name], factor_levels: vec![f1_levels, f2_levels, f3_levels], rows: obs };
        let explicit_subsets = fac_spec.formula.as_ref().map(|f| resolve_factorial_formula_subsets(f, data.factor_names.len(), &[])).transpose()?;
        let (res, model) =
            fit_factorial_anova_with_model_and_subsets(&data, fac_spec.max_interaction_order.clamp(1, 3), explicit_subsets, fac_spec.ss_type)?;
        let (excluded_cells, non_numeric_total, ignored_non_numeric) = scan_grouped_cells_audit(grouped, &selected_cols);
        let accounting = unpaired_observation_accounting(data.rows.len(), non_numeric_total, excluded_cells);
        return Ok(GroupedFactorialOutcome {
            result: res,
            observations: data,
            post_hoc: FactorialPostHocContext::Ols(model),
            aux_tabs: Vec::new(),
            estimator_note: Some(String::from("Estimator: Ordinary least-squares three-way factorial ANOVA.")),
            factor_source_indices: vec![None, None, None],
            accounting,
            ignored_non_numeric,
        });
    }

    let active_cols = active_grouped_cols(grouped, &selected_cols, &active_rows);
    if active_cols.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    if active_cols.len() < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial column levels", required: 2, actual: active_cols.len() });
    }

    // Repeated-measures 2-way path: classical split-plot / balanced RM ANOVA when every subject has
    // complete within-subject observations (including unequal between-subject group sizes in
    // split-plot designs), or REML mixed-effects on partial subjects, unstructured covariance, or
    // explicit REML mode.
    if fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
        let active_subs = active_grouped_subs(grouped, &active_rows, &active_cols, n_subs);
        if active_subs.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let is_split_plot =
            matches!(fac_spec.repeated_measures, RepeatedMeasuresDesign::RowFactorMatched | RepeatedMeasuresDesign::ColumnFactorMatched);
        let (split_groups, has_missing_selected, observed_cells, total_cells) = if is_split_plot {
            let (groups, has_partial, obs_cnt, req_cnt) =
                extract_split_plot_groups(grouped, &active_rows, &active_cols, n_subs, fac_spec.repeated_measures);
            (Some(groups), has_partial, obs_cnt, req_cnt)
        } else {
            let (obs_cnt, tot_cnt) = count_rm_active_cells(grouped, &active_rows, &active_cols, &active_subs);
            (None, obs_cnt < tot_cnt, obs_cnt, tot_cnt)
        };
        if has_missing_selected
            && !fac_spec.fit_reml_on_missing
            && fac_spec.residual_covariance != ResidualCovariance::Unstructured
            && !fac_spec.mixed_effects_reml
        {
            return Err(StatsError::InsufficientSampleSize {
                context: "complete repeated-measures cells (fit_reml_on_missing is false)",
                required: total_cells,
                actual: observed_cells,
            });
        }
        if (has_missing_selected && fac_spec.fit_reml_on_missing)
            || fac_spec.residual_covariance == ResidualCovariance::Unstructured
            || fac_spec.mixed_effects_reml
        {
            let dropped_tp_labels = if fac_spec.residual_covariance == ResidualCovariance::Unstructured {
                match fac_spec.repeated_measures {
                    RepeatedMeasuresDesign::ColumnFactorMatched => {
                        selected_cols.iter().filter(|c| !active_cols.contains(c)).map(|&c| grouped.data_sets[c].title.clone()).collect()
                    }
                    RepeatedMeasuresDesign::RowFactorMatched => {
                        let first_active = active_rows.first().copied().unwrap_or(0);
                        let last_active = active_rows.last().copied().unwrap_or(0);
                        (0..grouped.row_count())
                            .filter(|&r| {
                                if active_rows.contains(&r) {
                                    return false;
                                }
                                let has_title = grouped.row_titles.get(r).is_some_and(|s| !s.trim().is_empty());
                                let has_entry = selected_cols.iter().any(|&c| {
                                    grouped
                                        .data_sets
                                        .get(c)
                                        .and_then(|ds| ds.cells.get(r))
                                        .is_some_and(|subs| subs.iter().any(|cell| cell.excluded || !cell.value.is_empty()))
                                });
                                let is_interior = r > first_active && r < last_active;
                                has_title || has_entry || is_interior
                            })
                            .map(|r| grouped.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1)))
                            .collect()
                    }
                    _ => Vec::new(),
                }
            } else {
                Vec::new()
            };
            return run_rm_grouped_reml(
                grouped,
                out,
                &active_cols,
                &active_rows,
                &active_subs,
                fac_spec,
                has_missing_selected,
                &dropped_tp_labels,
                p_fmt,
                alpha,
            );
        }

        let include_interaction = fac_spec.max_interaction_order >= 2;
        let (rm_res, rm_post_hoc, obs) = if let Some(groups) = split_groups {
            let mut obs = Vec::new();
            for (g_pos, grp) in groups.iter().enumerate() {
                for subj in grp {
                    for (w_pos, &v) in subj.iter().enumerate() {
                        let cell = if fac_spec.repeated_measures == RepeatedMeasuresDesign::RowFactorMatched {
                            vec![w_pos, g_pos]
                        } else {
                            vec![g_pos, w_pos]
                        };
                        obs.push((cell, v));
                    }
                }
            }
            let (rm_res, rm_post_hoc) = fit_rm_two_way_split_plot_with_posthoc(
                &groups,
                [grouped.row_factor_title.as_str(), grouped.column_factor_title.as_str()],
                fac_spec.repeated_measures,
                fac_spec.ss_type,
                include_interaction,
                fac_spec.sphericity_correction,
            )?;
            (rm_res, rm_post_hoc, obs)
        } else {
            let n_eff_rows = active_rows.len();
            let n_eff_cols = active_cols.len();
            let n_eff_subs = active_subs.len();
            let mut subject_data = vec![vec![vec![0.0_f64; n_eff_cols]; n_eff_rows]; n_eff_subs];
            let mut obs = Vec::with_capacity(n_eff_subs * n_eff_rows * n_eff_cols);
            for (&orig_s, subj) in active_subs.iter().zip(subject_data.iter_mut()) {
                for (&orig_r, (r_pos, row_vec)) in active_rows.iter().zip(subj.iter_mut().enumerate()) {
                    for (&orig_c, (c_pos, slot)) in active_cols.iter().zip(row_vec.iter_mut().enumerate()) {
                        let Some(v) = grouped.observation(orig_r, orig_c, orig_s).and_then(|cell| cell.active_number()) else {
                            return Err(StatsError::InsufficientSampleSize {
                                context: "complete repeated-measures cells (fit_reml_on_missing is false)",
                                required: total_cells,
                                actual: observed_cells,
                            });
                        };
                        *slot = v;
                        obs.push((vec![r_pos, c_pos], v));
                    }
                }
            }
            let (rm_res, rm_post_hoc) = fit_rm_two_way_anova_with_posthoc(
                &subject_data,
                &grouped.row_factor_title,
                &grouped.column_factor_title,
                fac_spec.repeated_measures,
                fac_spec.sphericity_correction,
                include_interaction,
            )?;
            (rm_res, rm_post_hoc, obs)
        };
        let sph_tab = push_rm_sphericity_outputs(out, &grouped.row_factor_title, &grouped.column_factor_title, &rm_res, p_fmt, alpha)?;
        let corr_lbl = match fac_spec.sphericity_correction {
            RmSphericityCorrection::None => "uncorrected",
            RmSphericityCorrection::GreenhouseGeisser => "Greenhouse-Geisser corrected",
            RmSphericityCorrection::HuynhFeldt => "Huynh-Feldt corrected",
        };
        let (estimator_desc, estimator_expl) = if rm_res.anova.unbalanced {
            (
                format!("Classical split-plot repeated-measures ANOVA (OLS, {corr_lbl})"),
                format!(
                    "All subjects have complete within-subject observations with unequal between-subject group sizes; fitted classical split-plot repeated-measures ANOVA ({corr_lbl})."
                ),
            )
        } else {
            (
                format!("Classical balanced repeated-measures ANOVA (OLS, {corr_lbl})"),
                format!("All active repeated-measures cells are complete; fitted classical balanced repeated-measures ANOVA ({corr_lbl})."),
            )
        };
        out.checklist.push(checklist_item("Repeated-measures estimator", ChecklistStatus::Info, Some(estimator_desc.clone()), estimator_expl));
        let data = FactorialObservations {
            factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            factor_levels: vec![
                grouped_row_levels_for(grouped, &active_rows),
                active_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect(),
            ],
            rows: obs,
        };
        let (excluded_cells, non_numeric_total, ignored_non_numeric) = scan_grouped_cells_audit(grouped, &selected_cols);
        let accounting = ObservationAccounting {
            ignored_non_numeric_cells: non_numeric_total,
            excluded_cells,
            ..ObservationAccounting::new(ObservationAccountingKind::ListwiseAnova, data.rows.len())
        };
        return Ok(GroupedFactorialOutcome {
            result: rm_res.anova,
            observations: data,
            post_hoc: FactorialPostHocContext::RmBalanced(rm_post_hoc),
            aux_tabs: sph_tab.into_iter().collect(),
            estimator_note: Some(format!("Estimator: {estimator_desc}.")),
            factor_source_indices: vec![Some(active_rows), Some(active_cols)],
            accounting,
            ignored_non_numeric,
        });
    }

    // Standard Two-Way ANOVA (Row Factor x Column Factor).
    let row_levels = grouped_row_levels_for(grouped, &active_rows);
    let col_levels: Vec<String> = active_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();
    let mut obs = Vec::new();
    for (r_pos, &orig_r) in active_rows.iter().enumerate() {
        for (c_pos, &orig_c) in active_cols.iter().enumerate() {
            for y in grouped.active_replicates(orig_r, orig_c) {
                obs.push((vec![r_pos, c_pos], y));
            }
        }
    }
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![row_levels, col_levels],
        rows: obs,
    };
    let explicit_subsets = fac_spec.formula.as_ref().map(|f| resolve_factorial_formula_subsets(f, data.factor_names.len(), &[])).transpose()?;
    let (res, model) =
        fit_factorial_anova_with_model_and_subsets(&data, fac_spec.max_interaction_order.clamp(1, 2), explicit_subsets, fac_spec.ss_type)?;
    let (excluded_cells, non_numeric_total, ignored_non_numeric) = scan_grouped_cells_audit(grouped, &selected_cols);
    let accounting = unpaired_observation_accounting(data.rows.len(), non_numeric_total, excluded_cells);
    Ok(GroupedFactorialOutcome {
        result: res,
        observations: data,
        post_hoc: FactorialPostHocContext::Ols(model),
        aux_tabs: Vec::new(),
        estimator_note: Some(String::from("Estimator: Ordinary least-squares two-way factorial ANOVA.")),
        factor_source_indices: vec![Some(active_rows), Some(active_cols)],
        accounting,
        ignored_non_numeric,
    })
}

fn compact_binary_levels(levels: &[String], observed: &[bool; 2]) -> (Vec<String>, [usize; 2]) {
    let mut out = Vec::new();
    let mut remap = [0_usize; 2];
    for i in 0..2 {
        if observed[i] {
            remap[i] = out.len();
            if let Some(lbl) = levels.get(i) {
                out.push(lbl.clone());
            }
        }
    }
    (out, remap)
}

pub(in crate::anova) fn push_reml_df_scalars(out: &mut ResultSheet, terms: &[FactorialAnovaTerm], satt: &[f64], kr: &[f64]) {
    for (idx, term) in terms.iter().enumerate() {
        if term.df <= 0.0 {
            continue;
        }
        if let Some(&df_s) = satt.get(idx)
            && df_s.is_finite()
        {
            push_scalar(out, format!("{}: Satterthwaite df", term.name), None, df_s);
        }
        if let Some(&df_k) = kr.get(idx)
            && df_k.is_finite()
        {
            push_scalar(out, format!("{}: Kenward-Roger df", term.name), None, df_k);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fit_three_way_cs_reml(
    out: &mut ResultSheet,
    x_mat: &DenseMatrix,
    y_vec: &[f64],
    obs_cells: &[Vec<usize>],
    subj_ids: &[usize],
    factor_names: &[String],
    factor_levels: &[Vec<String>],
    term_slices: &[(&str, usize, usize)],
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<(Vec<FactorialAnovaTerm>, f64, f64, f64, RemlContrastCore), StatsError> {
    let vc_policy = fac_spec.reml_zero_variance_policy;
    let rm_flags = effective_three_way_rm_factors(fac_spec);
    let dims3 = [factor_levels[0].len().max(1), factor_levels[1].len().max(1), factor_levels[2].len().max(1)];
    let bet_axes: Vec<usize> = (0..3).filter(|&f| !rm_flags[f]).collect();
    let wth_axes: Vec<usize> = (0..3).filter(|&f| rm_flags[f]).collect();
    if wth_axes.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    let make_ids = |axes: &[usize]| -> Vec<usize> {
        obs_cells
            .iter()
            .zip(subj_ids.iter())
            .map(|(cell, &s)| {
                let mut id = s;
                for &ax in axes {
                    id = id * dims3[ax] + cell[ax];
                }
                id
            })
            .collect()
    };
    if wth_axes.len() == 1 {
        let subj_bet = make_ids(&bet_axes);
        return emit_random_intercept_reml_with_policy(out, x_mat, y_vec, &subj_bet, term_slices, fac_spec.df_method, vc_policy, p_fmt, alpha);
    }
    // Build Subject(Bet) + every proper non-empty subset W of Wth crossed with Subject(Bet).
    let mut owned_strata: Vec<(String, String, Vec<usize>)> = Vec::new();
    owned_strata.push((String::from("subject"), String::from("Subject (random)"), make_ids(&bet_axes)));
    let full_mask = (1_usize << wth_axes.len()) - 1;
    for sub_mask in 1..full_mask {
        let mut axes = bet_axes.clone();
        let mut s_parts = vec![String::from("subject")];
        let mut t_parts = vec![String::from("Subject")];
        for (bit, &ax) in wth_axes.iter().enumerate() {
            if (sub_mask & (1 << bit)) != 0 {
                axes.push(ax);
                s_parts.push(factor_names[ax].clone());
                t_parts.push(factor_names[ax].clone());
            }
        }
        owned_strata.push((s_parts.join(" × "), t_parts.join(" × "), make_ids(&axes)));
    }
    let strata_refs: Vec<(&str, &str, &[usize])> = owned_strata.iter().map(|(s, t, v)| (s.as_str(), t.as_str(), v.as_slice())).collect();
    emit_multi_vc_rm_reml_with_policy(out, x_mat, y_vec, &strata_refs, term_slices, fac_spec.df_method, vc_policy)
}

struct UnstructuredClusterSpec {
    cluster_ids: Vec<usize>,
    tp_ids: Vec<usize>,
    tp_labels: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
fn resolve_unstructured_rm_clusters(
    is_three_way: bool,
    fac_spec: &FactorialAnovaSpec,
    n_rows: usize,
    n_cols: usize,
    obs_cells: &[Vec<usize>],
    subj_ids: &[usize],
    subj_row_ids: &[usize],
    subj_col_ids: &[usize],
    row_ids: &[usize],
    col_ids: &[usize],
    factor_levels: &[Vec<String>],
) -> Result<UnstructuredClusterSpec, StatsError> {
    if is_three_way {
        let rm_flags = effective_three_way_rm_factors(fac_spec);
        let bet_axes: Vec<usize> = (0..3).filter(|&f| !rm_flags[f]).collect();
        let wth_axes: Vec<usize> = (0..3).filter(|&f| rm_flags[f]).collect();
        if wth_axes.is_empty() {
            return Err(StatsError::IncompatibleAnalysisSpec {
                expected: "at least one within-subject repeated-measures factor for Unstructured covariance REML",
            });
        }
        let dims3 = [factor_levels[0].len().max(1), factor_levels[1].len().max(1), factor_levels[2].len().max(1)];
        let cluster_ids: Vec<usize> = obs_cells
            .iter()
            .zip(subj_ids.iter())
            .map(|(cell, &s)| {
                let mut id = s;
                for &ax in &bet_axes {
                    id = id * dims3[ax] + cell[ax];
                }
                id
            })
            .collect();
        let tp_ids: Vec<usize> = obs_cells
            .iter()
            .map(|cell| {
                let mut tp = 0_usize;
                for &ax in &wth_axes {
                    tp = tp * dims3[ax] + cell[ax];
                }
                tp
            })
            .collect();
        let mut labels: Vec<String> = vec![String::new()];
        for &ax in &wth_axes {
            let mut next = Vec::with_capacity(labels.len() * factor_levels[ax].len());
            for prefix in &labels {
                for lvl in &factor_levels[ax] {
                    if prefix.is_empty() {
                        next.push(lvl.clone());
                    } else {
                        next.push(format!("{prefix} × {lvl}"));
                    }
                }
            }
            labels = next;
        }
        return Ok(UnstructuredClusterSpec { cluster_ids, tp_ids, tp_labels: labels });
    }
    let (cluster_ids, tp_ids, tp_labels) = match fac_spec.repeated_measures {
        RepeatedMeasuresDesign::ColumnFactorMatched => (subj_row_ids.to_vec(), col_ids.to_vec(), factor_levels[1].clone()),
        RepeatedMeasuresDesign::RowFactorMatched => (subj_col_ids.to_vec(), row_ids.to_vec(), factor_levels[0].clone()),
        _ => {
            let tps: Vec<usize> = row_ids.iter().zip(col_ids.iter()).map(|(&r, &c)| r * n_cols + c).collect();
            let mut labels = Vec::with_capacity(n_rows * n_cols);
            for r_lbl in &factor_levels[0] {
                for c_lbl in &factor_levels[1] {
                    labels.push(format!("{r_lbl} × {c_lbl}"));
                }
            }
            (subj_ids.to_vec(), tps, labels)
        }
    };
    Ok(UnstructuredClusterSpec { cluster_ids, tp_ids, tp_labels })
}

#[allow(clippy::too_many_arguments)]
pub(in crate::anova) fn run_rm_grouped_reml(
    grouped: &curvebook_doc::GroupedTable,
    out: &mut ResultSheet,
    active_cols: &[usize],
    active_rows: &[usize],
    active_subs: &[usize],
    fac_spec: &FactorialAnovaSpec,
    has_missing: bool,
    dropped_tp_labels: &[String],
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<GroupedFactorialOutcome, StatsError> {
    let n_rows = active_rows.len();
    let n_cols = active_cols.len();
    if fac_spec.three_way && n_cols != 4 {
        return Err(StatsError::InvalidParameter { name: "three_way_column_count", value: n_cols as f64 });
    }
    let is_three_way = fac_spec.three_way && n_cols == 4;

    let mut obs_a = [false; 2];
    let mut obs_b = [false; 2];
    if is_three_way {
        for &orig_r in active_rows {
            for (c_pos, &orig_c) in active_cols.iter().enumerate() {
                if active_subs.iter().any(|&s| grouped.observation(orig_r, orig_c, s).and_then(|cell| cell.active_number()).is_some()) {
                    obs_a[c_pos / 2] = true;
                    obs_b[c_pos % 2] = true;
                }
            }
        }
    }
    let l1_remap: [usize; 2];
    let l2_remap: [usize; 2];
    let (factor_names, factor_levels, max_order) = if is_three_way {
        let grouping = grouped.three_way_grouping.as_ref();
        let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
        let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
        let f3_name = grouped.row_factor_title.clone();
        let raw_f1 = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
        let raw_f2 = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
        let (f1_levels, r1) = compact_binary_levels(&raw_f1, &obs_a);
        let (f2_levels, r2) = compact_binary_levels(&raw_f2, &obs_b);
        l1_remap = r1;
        l2_remap = r2;
        let f3_levels = grouped_row_levels_for(grouped, active_rows);
        (vec![f1_name, f2_name, f3_name], vec![f1_levels, f2_levels, f3_levels], fac_spec.max_interaction_order.clamp(1, 3))
    } else {
        l1_remap = [0, 1];
        l2_remap = [0, 1];
        let row_levels = grouped_row_levels_for(grouped, active_rows);
        let col_levels: Vec<String> = active_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();
        (
            vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            vec![row_levels, col_levels],
            fac_spec.max_interaction_order.clamp(1, 2),
        )
    };

    let mut obs_cells = Vec::new();
    let mut obs = Vec::new();
    let mut y_vec = Vec::new();
    let mut subj_ids = Vec::new();
    let mut subj_row_ids = Vec::new();
    let mut subj_col_ids = Vec::new();
    let mut row_ids = Vec::new();
    let mut col_ids = Vec::new();

    for (s_pos, &orig_s) in active_subs.iter().enumerate() {
        for (r_pos, &orig_r) in active_rows.iter().enumerate() {
            for (c_pos, &orig_c) in active_cols.iter().enumerate() {
                let Some(v) = grouped.observation(orig_r, orig_c, orig_s).and_then(|cell| cell.active_number()) else {
                    continue;
                };
                let cell = if is_three_way { vec![l1_remap[c_pos / 2], l2_remap[c_pos % 2], r_pos] } else { vec![r_pos, c_pos] };
                obs_cells.push(cell.clone());
                obs.push((cell, v));
                y_vec.push(v);
                subj_ids.push(s_pos);
                subj_row_ids.push(s_pos * n_rows + r_pos);
                subj_col_ids.push(s_pos * n_cols + c_pos);
                row_ids.push(r_pos);
                col_ids.push(c_pos);
            }
        }
    }

    let n_obs = y_vec.len();
    if n_obs == 0 {
        return Err(StatsError::EmptyInput);
    }
    let RemlEffectDesign { x_mat, subsets, factor_basis, terms: term_ranges, observed_cells } =
        build_reml_effect_design(&obs_cells, &factor_names, &factor_levels, max_order)?;
    let p_total = x_mat.ncols();
    let term_slices: Vec<(&str, usize, usize)> = term_ranges.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();

    let (fixed_terms, sigma2_e, df_residual, reml_core, estimator_desc) = if fac_spec.residual_covariance == ResidualCovariance::Unstructured {
        let UnstructuredClusterSpec { cluster_ids, tp_ids, tp_labels } = resolve_unstructured_rm_clusters(
            is_three_way,
            fac_spec,
            n_rows,
            n_cols,
            &obs_cells,
            &subj_ids,
            &subj_row_ids,
            &subj_col_ids,
            &row_ids,
            &col_ids,
            &factor_levels,
        )?;
        let (terms, ms_e, df_r, core) =
            emit_unstructured_rm_reml(out, &x_mat, &y_vec, &cluster_ids, &tp_ids, &tp_labels, &term_slices, fac_spec.df_method)?;
        if !dropped_tp_labels.is_empty() {
            let note = format!("Excluded all-missing timepoint(s): {}.", dropped_tp_labels.join(", "));
            for tab in out.tabs.iter_mut().filter(|t| t.id == "unstructured-reml-covariance" || t.id == "unstructured-reml-correlation") {
                tab.footnotes.push(note.clone());
            }
            if let Some(item) = out.checklist.iter_mut().find(|c| c.assumption == "Unstructured covariance REML") {
                item.status = ChecklistStatus::Warn;
                item.explanation.push(' ');
                item.explanation.push_str(&note);
            }
        }
        let df_lbl = match fac_spec.df_method {
            MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
            MixedModelDfMethod::Satterthwaite => "Satterthwaite",
        };
        let desc = format!("Linear mixed-effects REML (Unstructured, {df_lbl} df)");
        out.checklist.push(checklist_item(
            "Repeated-measures estimator",
            ChecklistStatus::Info,
            Some(desc.clone()),
            format!("Fitted unstructured covariance REML with {df_lbl} degrees of freedom."),
        ));
        (terms, ms_e, df_r, core, desc)
    } else {
        let vc_policy = fac_spec.reml_zero_variance_policy;
        let (fixed_terms, sigma2_e, reml_ll, icc_subj, core) = if is_three_way {
            fit_three_way_cs_reml(out, &x_mat, &y_vec, &obs_cells, &subj_ids, &factor_names, &factor_levels, &term_slices, fac_spec, p_fmt, alpha)?
        } else {
            match fac_spec.repeated_measures {
                RepeatedMeasuresDesign::BothFactorsMatched => {
                    let sr_name = format!("subject × {}", factor_names[0]);
                    let sc_name = format!("subject × {}", factor_names[1]);
                    let sr_title = format!("Subject × {}", factor_names[0]);
                    let sc_title = format!("Subject × {}", factor_names[1]);
                    let strata = [
                        ("subject", "Subject (random)", subj_ids.as_slice()),
                        (sr_name.as_str(), sr_title.as_str(), subj_row_ids.as_slice()),
                        (sc_name.as_str(), sc_title.as_str(), subj_col_ids.as_slice()),
                    ];
                    emit_multi_vc_rm_reml_with_policy(out, &x_mat, &y_vec, &strata, &term_slices, fac_spec.df_method, vc_policy)?
                }
                RepeatedMeasuresDesign::ColumnFactorMatched => emit_random_intercept_reml_with_policy(
                    out,
                    &x_mat,
                    &y_vec,
                    &subj_row_ids,
                    &term_slices,
                    fac_spec.df_method,
                    vc_policy,
                    p_fmt,
                    alpha,
                )?,
                RepeatedMeasuresDesign::RowFactorMatched => emit_random_intercept_reml_with_policy(
                    out,
                    &x_mat,
                    &y_vec,
                    &subj_col_ids,
                    &term_slices,
                    fac_spec.df_method,
                    vc_policy,
                    p_fmt,
                    alpha,
                )?,
                RepeatedMeasuresDesign::None => return Err(StatsError::EmptyInput),
            }
        };

        let df_lbl = match fac_spec.df_method {
            MixedModelDfMethod::KenwardRoger => "Kenward-Roger",
            MixedModelDfMethod::Satterthwaite => "Satterthwaite",
        };
        let (check_title, check_desc) = if has_missing {
            (
                "Missing-value mixed-effects REML",
                format!(
                    "Missing replicates detected in repeated-measures cells; fitted linear mixed-effects REML with {df_lbl} degrees of freedom instead of listwise deletion."
                ),
            )
        } else if is_three_way {
            ("Three-way repeated-measures mixed-effects REML", format!("Fitted three-way mixed-effects REML with {df_lbl} degrees of freedom."))
        } else {
            ("Repeated-measures mixed-effects REML", format!("Fitted linear mixed-effects REML with {df_lbl} degrees of freedom."))
        };
        out.checklist.push(checklist_item(
            check_title,
            ChecklistStatus::Info,
            Some(format!("ICC = {icc_subj:.3}, REML logL = {reml_ll:.2}")),
            check_desc,
        ));
        let desc = format!("Linear mixed-effects REML (Compound Symmetry, {df_lbl} df)");
        out.checklist.push(checklist_item(
            "Repeated-measures estimator",
            ChecklistStatus::Info,
            Some(desc.clone()),
            format!("Fitted compound-symmetry linear mixed-effects REML with {df_lbl} degrees of freedom."),
        ));
        let df_res = core.n_minus_p;
        (fixed_terms, sigma2_e, df_res, core, desc)
    };

    let ym = SummaryMoments::from_slice(&y_vec)?;
    let res = FactorialAnovaResult {
        terms: fixed_terms,
        ss_residual: sigma2_e * df_residual,
        df_residual,
        ms_residual: sigma2_e,
        ss_total: ym.css,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced: has_missing,
        ss_type: SumOfSquaresType::TypeIII,
    };
    let data = FactorialObservations { factor_names, factor_levels: factor_levels.clone(), rows: obs };
    let (_, row_proj) = build_xtx_pinv_and_proj(&x_mat)?;
    let factor_level_counts: Vec<usize> = factor_levels.iter().map(Vec::len).collect();
    let reml_model = RemlContrastModel {
        factor_levels: factor_level_counts,
        factor_basis,
        subsets,
        total_cols: p_total,
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
    let factor_source_indices = if is_three_way { vec![None, None, None] } else { vec![Some(active_rows.to_vec()), Some(active_cols.to_vec())] };
    let (accounting, ignored_non_numeric) = scan_grouped_rm_cell_accounting(
        grouped,
        active_rows,
        active_cols,
        active_subs,
        active_cols,
        ObservationAccountingKind::MixedEffectsAnova,
        n_obs,
    );
    Ok(GroupedFactorialOutcome {
        result: res,
        observations: data,
        post_hoc: FactorialPostHocContext::Reml(reml_model),
        aux_tabs: Vec::new(),
        estimator_note: Some(format!("Estimator: {estimator_desc}.")),
        factor_source_indices,
        accounting,
        ignored_non_numeric,
    })
}

pub(in crate::anova) fn run_multifactor_mv(
    mv: &curvebook_doc::MultipleVariablesTable,
    out: &mut ResultSheet,
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
    alpha: f64,
) -> Result<GroupedFactorialOutcome, StatsError> {
    if fac_spec.repeated_measures != RepeatedMeasuresDesign::None
        || fac_spec.mixed_effects_reml
        || fac_spec.residual_covariance == ResidualCovariance::Unstructured
    {
        return Err(StatsError::IncompatibleAnalysisSpec {
            expected: "independent-observations OLS multifactor ANOVA on MultipleVariablesTable (repeated_measures = None, mixed_effects_reml = false, residual_covariance = CompoundSymmetry)",
        });
    }
    let resp_idx = fac_spec.response_column.ok_or(StatsError::MissingColumn { index: 0 })?;
    let resp_var = mv.variables.get(resp_idx).ok_or(StatsError::MissingColumn { index: resp_idx })?;
    if fac_spec.multifactor_columns.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "multifactor columns", required: 1, actual: 0 });
    }
    if fac_spec.three_way && fac_spec.multifactor_columns.len() != 3 {
        return Err(StatsError::InvalidParameter { name: "three_way_factor_count", value: fac_spec.multifactor_columns.len() as f64 });
    }
    let mut seen_factor_cols = std::collections::BTreeSet::new();
    let mut factor_names = Vec::with_capacity(fac_spec.multifactor_columns.len());
    let mut factor_levels = Vec::with_capacity(fac_spec.multifactor_columns.len());
    for &c_idx in &fac_spec.multifactor_columns {
        if c_idx == resp_idx || !seen_factor_cols.insert(c_idx) {
            return Err(StatsError::InvalidParameter { name: "multifactor_columns", value: c_idx as f64 });
        }
        let var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
        factor_names.push(var.title.clone());
        let levels: Vec<String> = var.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
        factor_levels.push(levels);
    }

    let n_rows = mv.row_count();
    let cell_has_entry = |c: Option<&curvebook_doc::Cell>| c.is_some_and(|cell| cell.excluded || !cell.is_empty());
    let active_end = (0..n_rows).rfind(|&r| {
        cell_has_entry(resp_var.cells.get(r))
            || fac_spec.multifactor_columns.iter().any(|&c_idx| mv.variables.get(c_idx).is_some_and(|v| cell_has_entry(v.cells.get(r))))
    });
    let mut obs = Vec::new();
    let mut accounting = ObservationAccounting::new(ObservationAccountingKind::ListwiseAnova, 0);
    let mut resp_non_num = 0_usize;
    if let Some(end_row) = active_end {
        for r in 0..=end_row {
            let rc = resp_var.cells.get(r);
            let any_factor_entry =
                fac_spec.multifactor_columns.iter().any(|&c_idx| mv.variables.get(c_idx).is_some_and(|v| cell_has_entry(v.cells.get(r))));
            if !cell_has_entry(rc) && !any_factor_entry {
                continue;
            }
            let mut row_has_excluded = false;
            if rc.is_some_and(|c| c.excluded) {
                row_has_excluded = true;
                accounting.excluded_cells = accounting.excluded_cells.saturating_add(1);
            }
            let row_resp_non_num = rc.is_some_and(is_ignored_non_numeric_cell);
            if row_resp_non_num {
                resp_non_num += 1;
            }
            let mut lvls = Vec::with_capacity(fac_spec.multifactor_columns.len());
            let mut valid = true;
            for (f_pos, &c_idx) in fac_spec.multifactor_columns.iter().enumerate() {
                let factor_var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
                let cell = factor_var.cells.get(r).ok_or(StatsError::DimensionMismatch { expected: n_rows, actual: factor_var.cells.len() })?;
                if cell.excluded {
                    row_has_excluded = true;
                    accounting.excluded_cells = accounting.excluded_cells.saturating_add(1);
                    valid = false;
                    continue;
                }
                if !valid {
                    continue;
                }
                let tok = match &cell.value {
                    curvebook_doc::CellValue::Text(s) => s.trim().to_owned(),
                    curvebook_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
                    _ => {
                        valid = false;
                        continue;
                    }
                };
                let Some(pos) = factor_levels[f_pos].iter().position(|l| l == &tok) else {
                    valid = false;
                    continue;
                };
                lvls.push(pos);
            }
            if let Some(y) = rc.and_then(|c| c.active_number())
                && valid
            {
                obs.push((lvls, y));
            } else {
                record_omitted_row_reason(&mut accounting, row_has_excluded, row_resp_non_num, false);
            }
        }
    }
    if obs.is_empty() {
        return Err(StatsError::EmptyInput);
    }
    accounting.analyzed = obs.len();
    accounting.ignored_non_numeric_cells = resp_non_num;
    let ignored_non_numeric = if resp_non_num > 0 { vec![(resp_var.title.clone(), resp_non_num)] } else { Vec::new() };
    // Drop unobserved/excluded factor levels after complete-case row filtering before effect coding (#30).
    let n_factors = factor_levels.len();
    let mut remaps = Vec::with_capacity(n_factors);
    for (f_pos, levels) in factor_levels.iter_mut().enumerate() {
        let mut seen = vec![false; levels.len()];
        for (row_lvls, _) in &obs {
            seen[row_lvls[f_pos]] = true;
        }
        let mut remap = vec![0_usize; levels.len()];
        let mut compacted = Vec::with_capacity(levels.len());
        for (orig_idx, lvl) in levels.drain(..).enumerate() {
            if seen[orig_idx] {
                remap[orig_idx] = compacted.len();
                compacted.push(lvl);
            }
        }
        if compacted.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        *levels = compacted;
        remaps.push(remap);
    }
    for (row_lvls, _) in &mut obs {
        for (f_pos, lvl) in row_lvls.iter_mut().enumerate() {
            *lvl = remaps[f_pos][*lvl];
        }
    }
    let data = FactorialObservations { factor_names, factor_levels, rows: obs };
    // Three-way RM on MultipleVariablesTable uses `[factor_0, factor_1, factor_2]` axis order
    // matching `multifactor_columns`. Require balanced complete replicates (>= 2 per cell) rather
    // than silently degrading to independent-observations OLS.
    if fac_spec.three_way
        && let Some(rm_flags) = fac_spec.three_way_rm_factors
        && rm_flags.iter().any(|&v| v)
    {
        if data.factor_levels.len() != 3 {
            return Err(StatsError::IncompatibleAnalysisSpec {
                expected: "exactly 3 categorical factors for three-way repeated-measures ANOVA on MultipleVariablesTable",
            });
        }
        let (la, lb, lc) = (data.factor_levels[0].len(), data.factor_levels[1].len(), data.factor_levels[2].len());
        let mut cell_buckets = vec![vec![vec![Vec::new(); lc]; lb]; la];
        for (lvls, y) in &data.rows {
            cell_buckets[lvls[0]][lvls[1]][lvls[2]].push(*y);
        }
        let s_count = cell_buckets[0][0][0].len();
        if s_count < 2 {
            return Err(StatsError::InsufficientSampleSize { context: "three-way RM ANOVA replicates per cell", required: 2, actual: s_count });
        }
        let balanced = cell_buckets.iter().all(|p| p.iter().all(|r| r.iter().all(|v| v.len() == s_count)));
        if !balanced {
            return Err(StatsError::IncompatibleAnalysisSpec {
                expected: "balanced replicates across all 3-way cells for classical three-way RM ANOVA on MultipleVariablesTable",
            });
        }
        let mut subject_data = vec![vec![vec![vec![0.0_f64; lc]; lb]; la]; s_count];
        for s in 0..s_count {
            for a in 0..la {
                for b in 0..lb {
                    for c in 0..lc {
                        subject_data[s][a][b][c] = cell_buckets[a][b][c][s];
                    }
                }
            }
        }
        let rm3 = fit_rm_three_way_anova(
            &subject_data,
            [data.factor_names[0].as_str(), data.factor_names[1].as_str(), data.factor_names[2].as_str()],
            rm_flags,
            fac_spec.sphericity_correction,
        )?;
        let sph_tab = push_rm_three_way_sphericity_outputs(out, &rm3, p_fmt, alpha)?;
        let (_, mut post_model) = fit_factorial_anova_with_model(&data, 3, SumOfSquaresType::TypeIII)?;
        post_model.ms_resid = rm3.anova.ms_residual;
        post_model.df_resid = rm3.anova.df_residual;
        let corr_lbl = match fac_spec.sphericity_correction {
            RmSphericityCorrection::None => "uncorrected",
            RmSphericityCorrection::GreenhouseGeisser => "Greenhouse-Geisser corrected",
            RmSphericityCorrection::HuynhFeldt => "Huynh-Feldt corrected",
        };
        let estimator_desc = format!("Classical balanced three-way RM ANOVA ({corr_lbl})");
        out.checklist.push(checklist_item(
            "Repeated-measures estimator",
            ChecklistStatus::Info,
            Some(estimator_desc.clone()),
            format!("Fitted classical balanced three-way RM ANOVA with within-subject flags {:?} and {corr_lbl} sphericity.", rm_flags),
        ));
        return Ok(GroupedFactorialOutcome {
            result: rm3.anova,
            observations: data,
            post_hoc: FactorialPostHocContext::Ols(post_model),
            aux_tabs: sph_tab.into_iter().collect(),
            estimator_note: Some(format!("Estimator: {estimator_desc}.")),
            factor_source_indices: vec![None, None, None],
            accounting,
            ignored_non_numeric,
        });
    }
    let explicit_subsets = fac_spec
        .formula
        .as_ref()
        .map(|f| resolve_factorial_formula_subsets(f, data.factor_names.len(), &fac_spec.multifactor_columns))
        .transpose()?;
    let (res, model) = fit_factorial_anova_with_model_and_subsets(&data, fac_spec.max_interaction_order, explicit_subsets, fac_spec.ss_type)?;
    let n_fac = data.factor_levels.len();
    Ok(GroupedFactorialOutcome {
        result: res,
        observations: data,
        post_hoc: FactorialPostHocContext::Ols(model),
        aux_tabs: Vec::new(),
        estimator_note: None,
        factor_source_indices: vec![None; n_fac],
        accounting,
        ignored_non_numeric,
    })
}
