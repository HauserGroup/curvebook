//! `ResultSheet` builder for factorial specs: two-way, three-way, repeated-measures two-way,
//! mixed-model REML on incomplete repeated-measures data, and multifactor *N*-way ANOVA.
//!
//! References:
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, DataSheet, FactorialAnovaSpec, MixedModelDfMethod, PValueFormat, RepeatedMeasuresDesign, ResidualCovariance,
    ResultCell, ResultSheet, ResultTab, ResultTabKind, SubcolumnMode, SumOfSquaresType, TableArchetype,
};
use curvebook_math::sum::SummaryMoments;

use crate::anova::factorial::{
    FactorialObservations, OlsPostHocModel, build_xtx_pinv_and_proj, finalize_term_p_values, fit_factorial_anova_with_model,
};
use crate::anova::posthoc::{FactorialPostHocContext, RemlContrastModel, run_factorial_multiplicity};
use crate::anova::results::{FactorialAnovaResult, FactorialAnovaTerm, RmSphericityCorrection, RmTwoWayAnovaResult, SphericityResult};
use crate::anova::rm_two_way::{apply_rm_sphericity_p, fit_rm_two_way_anova_with_posthoc};
use crate::anova::sheet::one_way::{
    RemlEffectDesign, build_reml_effect_design, emit_multi_vc_rm_reml, emit_random_intercept_reml, emit_unstructured_rm_reml,
};
use crate::common::{
    build_tab, checklist_item, init_result_sheet, num_cell, opt_num_cell, p_cell, push_scalar, resolve_selected_indices, sanitize_result_sheet_floats,
};
use crate::error::StatsError;

/// Execute a [`FactorialAnovaSpec`] (Two-way, Three-way `2x2xK`, Repeated-Measures Two-way,
/// Mixed-Effects REML on missing RM data, or Multifactor `N`-way ANOVA).
pub fn run_factorial_anova(sheet: &DataSheet, spec: &AnalysisSpec, fac_spec: &FactorialAnovaSpec) -> Result<ResultSheet, StatsError> {
    let mut out = init_result_sheet(spec)?;
    let p_fmt = spec.p_value_format;
    let confidence = spec.confidence_level;

    let (mut result, obs, post_hoc, aux_tabs) = match &sheet.archetype {
        TableArchetype::Grouped(grouped) => {
            let outcome = run_grouped_factorial(grouped, &mut out, &spec.selected_data_sets, fac_spec, p_fmt)?;
            (outcome.result, outcome.observations, outcome.post_hoc, outcome.aux_tabs)
        }
        TableArchetype::MultipleVariables(mv) => {
            let (res, obs, model) = run_multifactor_mv(mv, fac_spec)?;
            (res, obs, FactorialPostHocContext::Ols(model), Vec::new())
        }
        other => return Err(StatsError::IncompatibleTable { analysis: "factorial ANOVA", actual: other.kind() }),
    };

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
        headers.push(String::from("Partial eta squared"));
        headers.push(String::from("Eta squared"));
    }
    let mut row_labels = Vec::with_capacity(result.terms.len() + 2);
    let mut rows = Vec::with_capacity(result.terms.len() + 2);
    for t in &result.terms {
        row_labels.push(t.name.clone());
        let mut r = vec![
            num_cell(t.ss),
            num_cell(t.df),
            num_cell(t.ms),
            opt_num_cell(t.f_statistic),
            t.p_value.map_or(ResultCell::Empty, |p| p_cell(p, p_fmt)),
        ];
        if fac_spec.compute_effect_sizes {
            r.push(opt_num_cell(t.partial_eta_squared));
            r.push(opt_num_cell(t.eta_squared));
        }
        rows.push(r);
    }
    row_labels.push(String::from("Residual"));
    let mut r_res =
        vec![num_cell(result.ss_residual), num_cell(result.df_residual), num_cell(result.ms_residual), ResultCell::Empty, ResultCell::Empty];
    row_labels.push(String::from("Total"));
    let mut r_tot = vec![num_cell(result.ss_total), num_cell(result.df_total), ResultCell::Empty, ResultCell::Empty, ResultCell::Empty];
    if fac_spec.compute_effect_sizes {
        r_res.extend(vec![ResultCell::Empty, ResultCell::Empty]);
        r_tot.extend(vec![ResultCell::Empty, ResultCell::Empty]);
    }
    rows.push(r_res);
    rows.push(r_tot);

    let tab_title = if fac_spec.three_way { "Three-way ANOVA" } else { "Factorial ANOVA" };
    out.tabs.push(build_tab("factorial-anova", tab_title, ResultTabKind::AnovaTable, headers, row_labels, rows));
    out.tabs.extend(aux_tabs);

    run_factorial_multiplicity(&mut out, &obs, &post_hoc, &result, &fac_spec.multiplicity, p_fmt, confidence)?;

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

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}

pub(in crate::anova) fn grouped_row_levels(grouped: &curvebook_doc::GroupedTable, n_rows: usize) -> Vec<String> {
    (0..n_rows).map(|r| grouped.row_titles.get(r).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Row {}", r + 1))).collect()
}

pub(in crate::anova) struct GroupedFactorialOutcome {
    pub(in crate::anova) result: FactorialAnovaResult,
    pub(in crate::anova) observations: FactorialObservations,
    pub(in crate::anova) post_hoc: FactorialPostHocContext,
    pub(in crate::anova) aux_tabs: Vec<ResultTab>,
}

pub(in crate::anova) fn build_rm_sphericity_tab(
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
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

    let headers = vec![
        String::from("Mauchly W"),
        String::from("Mauchly chi2"),
        String::from("df"),
        String::from("Mauchly P"),
        String::from("G-G epsilon"),
        String::from("H-F epsilon"),
        String::from("G-G P"),
        String::from("H-F P"),
    ];
    let mut row_labels = Vec::new();
    let mut rows = Vec::new();
    for (label, s_opt, term_idx) in entries {
        let (Some(s), Some(term), Some(&df_err)) = (s_opt, res.terms.get(term_idx), rm_res.term_error_dfs.get(term_idx)) else {
            continue;
        };
        let f_val = term.f_statistic.unwrap_or(0.0);
        let p_gg = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::GreenhouseGeisser)?;
        let p_hf = apply_rm_sphericity_p(f_val, term.df, df_err, Some(s), RmSphericityCorrection::HuynhFeldt)?;
        row_labels.push(label.to_owned());
        rows.push(vec![
            num_cell(s.mauchly_w),
            num_cell(s.mauchly_chi2),
            num_cell(s.mauchly_df),
            p_cell(s.mauchly_p, p_fmt),
            num_cell(s.greenhouse_geisser_eps),
            num_cell(s.huynh_feldt_eps),
            p_cell(p_gg, p_fmt),
            p_cell(p_hf, p_fmt),
        ]);
    }
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(build_tab("rm-sphericity", "Repeated-measures sphericity", ResultTabKind::Diagnostics, headers, row_labels, rows)))
}

pub(in crate::anova) fn push_rm_sphericity_outputs(
    out: &mut ResultSheet,
    row_name: &str,
    col_name: &str,
    rm_res: &RmTwoWayAnovaResult,
    p_fmt: PValueFormat,
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
    build_rm_sphericity_tab(row_name, col_name, rm_res, p_fmt)
}

pub(in crate::anova) fn run_grouped_factorial(
    grouped: &curvebook_doc::GroupedTable,
    out: &mut ResultSheet,
    selected: &[usize],
    fac_spec: &FactorialAnovaSpec,
    p_fmt: PValueFormat,
) -> Result<GroupedFactorialOutcome, StatsError> {
    if grouped.subcolumn_mode.is_pre_summarized() {
        return Err(StatsError::RequiresRawReplicates);
    }
    let n_rows = grouped.row_count();
    let selected_cols = resolve_selected_indices(selected, grouped.data_sets.len());
    let n_cols = selected_cols.len();
    if fac_spec.three_way && n_cols != 4 {
        return Err(StatsError::InvalidParameter { name: "three_way_column_count", value: n_cols as f64 });
    }
    if n_cols < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial column levels", required: 2, actual: n_cols });
    }
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };

    // Repeated-measures 2-way path (REML mixed-effects on missing replicates, unstructured covariance, or explicit REML mode).
    if !fac_spec.three_way && fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
        let has_missing_selected = (0..n_rows).any(|r| {
            selected_cols.iter().any(|&c| (0..n_subs).any(|s| grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_none()))
        });
        if (has_missing_selected && fac_spec.fit_reml_on_missing)
            || fac_spec.residual_covariance == ResidualCovariance::Unstructured
            || fac_spec.mixed_effects_reml
        {
            return run_rm_grouped_reml(grouped, out, &selected_cols, fac_spec, has_missing_selected, p_fmt);
        }
        let mut subject_data = vec![vec![vec![0.0_f64; n_cols]; n_rows]; n_subs];
        let mut obs = Vec::with_capacity(n_subs * n_rows * n_cols);
        for (s, subj) in subject_data.iter_mut().enumerate() {
            for (r, row_vec) in subj.iter_mut().enumerate() {
                for (c_pos, (&orig_c, slot)) in selected_cols.iter().zip(row_vec.iter_mut()).enumerate() {
                    let Some(v) = grouped.observation(r, orig_c, s).and_then(|cell| cell.active_number()) else {
                        return run_rm_grouped_reml(grouped, out, &selected_cols, fac_spec, true, p_fmt);
                    };
                    *slot = v;
                    obs.push((vec![r, c_pos], v));
                }
            }
        }
        let (rm_res, rm_post_hoc) = fit_rm_two_way_anova_with_posthoc(
            &subject_data,
            &grouped.row_factor_title,
            &grouped.column_factor_title,
            fac_spec.repeated_measures,
            RmSphericityCorrection::None,
        )?;
        let sph_tab = push_rm_sphericity_outputs(out, &grouped.row_factor_title, &grouped.column_factor_title, &rm_res, p_fmt)?;
        let data = FactorialObservations {
            factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            factor_levels: vec![grouped_row_levels(grouped, n_rows), selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect()],
            rows: obs,
        };
        return Ok(GroupedFactorialOutcome {
            result: rm_res.anova,
            observations: data,
            post_hoc: FactorialPostHocContext::RmBalanced(rm_post_hoc),
            aux_tabs: sph_tab.into_iter().collect(),
        });
    }

    // Three-way 2 x 2 x K path (mixed-effects REML when repeated-measures design is active; OLS otherwise).
    if fac_spec.three_way && n_cols == 4 {
        if fac_spec.repeated_measures != RepeatedMeasuresDesign::None {
            let has_missing_selected = (0..n_rows).any(|r| {
                selected_cols.iter().any(|&c| (0..n_subs).any(|s| grouped.observation(r, c, s).and_then(|cell| cell.active_number()).is_none()))
            });
            return run_rm_grouped_reml(grouped, out, &selected_cols, fac_spec, has_missing_selected, p_fmt);
        }
        let grouping = grouped.three_way_grouping.as_ref();
        let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
        let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
        let f3_name = grouped.row_factor_title.clone();
        let f1_levels = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
        let f2_levels = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
        let f3_levels = grouped_row_levels(grouped, n_rows);

        let mut obs = Vec::new();
        for r in 0..n_rows {
            for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
                let l1 = c_pos / 2;
                let l2 = c_pos % 2;
                for y in grouped.active_replicates(r, orig_c) {
                    obs.push((vec![l1, l2, r], y));
                }
            }
        }
        let data =
            FactorialObservations { factor_names: vec![f1_name, f2_name, f3_name], factor_levels: vec![f1_levels, f2_levels, f3_levels], rows: obs };
        let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order.clamp(1, 3), fac_spec.ss_type)?;
        return Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Ols(model), aux_tabs: Vec::new() });
    }

    // Standard Two-Way ANOVA (Row Factor x Column Factor).
    let row_levels = grouped_row_levels(grouped, n_rows);
    let col_levels: Vec<String> = selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();
    let mut obs = Vec::new();
    for r in 0..n_rows {
        for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
            for y in grouped.active_replicates(r, orig_c) {
                obs.push((vec![r, c_pos], y));
            }
        }
    }
    let data = FactorialObservations {
        factor_names: vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
        factor_levels: vec![row_levels, col_levels],
        rows: obs,
    };
    let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order.clamp(1, 2), fac_spec.ss_type)?;
    Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Ols(model), aux_tabs: Vec::new() })
}

pub(in crate::anova) fn push_reml_df_scalars(out: &mut ResultSheet, terms: &[FactorialAnovaTerm], satt: &[f64], kr: &[f64]) {
    for (idx, term) in terms.iter().enumerate() {
        if let Some(&df_s) = satt.get(idx) {
            push_scalar(out, format!("{}: Satterthwaite df", term.name), None, df_s);
        }
        if let Some(&df_k) = kr.get(idx) {
            push_scalar(out, format!("{}: Kenward-Roger df", term.name), None, df_k);
        }
    }
}

pub(in crate::anova) fn run_rm_grouped_reml(
    grouped: &curvebook_doc::GroupedTable,
    out: &mut ResultSheet,
    selected_cols: &[usize],
    fac_spec: &FactorialAnovaSpec,
    has_missing: bool,
    p_fmt: PValueFormat,
) -> Result<GroupedFactorialOutcome, StatsError> {
    let n_rows = grouped.row_count();
    let n_cols = selected_cols.len();
    if fac_spec.three_way && n_cols != 4 {
        return Err(StatsError::InvalidParameter { name: "three_way_column_count", value: n_cols as f64 });
    }
    let SubcolumnMode::RawReplicates(n_subs) = grouped.subcolumn_mode else {
        return Err(StatsError::RequiresRawReplicates);
    };
    let is_three_way = fac_spec.three_way && n_cols == 4;

    let (factor_names, factor_levels, max_order, ab_labels) = if is_three_way {
        let grouping = grouped.three_way_grouping.as_ref();
        let f1_name = grouping.map_or_else(|| String::from("Factor A"), |g| g.factor_1_name.clone());
        let f2_name = grouping.map_or_else(|| String::from("Factor B"), |g| g.factor_2_name.clone());
        let f3_name = grouped.row_factor_title.clone();
        let f1_levels = grouping.map_or_else(|| [String::from("A1"), String::from("A2")], |g| g.factor_1_levels.clone()).to_vec();
        let f2_levels = grouping.map_or_else(|| [String::from("B1"), String::from("B2")], |g| g.factor_2_levels.clone()).to_vec();
        let f3_levels = grouped_row_levels(grouped, n_rows);
        let mut ab_lbls = Vec::with_capacity(4);
        for a_l in &f1_levels {
            for b_l in &f2_levels {
                ab_lbls.push(format!("{a_l} × {b_l}"));
            }
        }
        (vec![f1_name, f2_name, f3_name], vec![f1_levels, f2_levels, f3_levels], fac_spec.max_interaction_order.clamp(1, 3), ab_lbls)
    } else {
        let row_levels = grouped_row_levels(grouped, n_rows);
        let col_levels: Vec<String> = selected_cols.iter().map(|&c| grouped.data_sets[c].title.clone()).collect();
        (
            vec![grouped.row_factor_title.clone(), grouped.column_factor_title.clone()],
            vec![row_levels, col_levels],
            fac_spec.max_interaction_order.clamp(1, 2),
            Vec::new(),
        )
    };

    let mut obs_cells = Vec::new();
    let mut obs = Vec::new();
    let mut y_vec = Vec::new();
    let mut subj_ids = Vec::new();
    let mut subj_row_ids = Vec::new();
    let mut subj_col_ids = Vec::new();
    let mut id_sa = Vec::new();
    let mut id_sb = Vec::new();
    let mut id_sab = Vec::new();
    let mut id_sac = Vec::new();
    let mut id_sbc = Vec::new();
    let mut id_sr_a = Vec::new();
    let mut id_sr_b = Vec::new();
    let mut row_ids = Vec::new();
    let mut col_ids = Vec::new();

    for s in 0..n_subs {
        for r in 0..n_rows {
            for (c_pos, &orig_c) in selected_cols.iter().enumerate() {
                let Some(v) = grouped.observation(r, orig_c, s).and_then(|cell| cell.active_number()) else {
                    continue;
                };
                let cell = if is_three_way { vec![c_pos / 2, c_pos % 2, r] } else { vec![r, c_pos] };
                obs_cells.push(cell.clone());
                obs.push((cell, v));
                y_vec.push(v);
                subj_ids.push(s);
                subj_row_ids.push(s * n_rows + r);
                subj_col_ids.push(s * n_cols + c_pos);
                row_ids.push(r);
                col_ids.push(c_pos);
                if is_three_way {
                    let l1 = c_pos / 2;
                    let l2 = c_pos % 2;
                    id_sa.push(s * 2 + l1);
                    id_sb.push(s * 2 + l2);
                    id_sab.push(s * 4 + c_pos);
                    id_sac.push((s * 2 + l1) * n_rows + r);
                    id_sbc.push((s * 2 + l2) * n_rows + r);
                    let sr = s * n_rows + r;
                    id_sr_a.push(sr * 2 + l1);
                    id_sr_b.push(sr * 2 + l2);
                }
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
    if fac_spec.residual_covariance == ResidualCovariance::CompoundSymmetry && n_obs <= p_total {
        return Err(StatsError::InsufficientSampleSize { context: "RM REML observations", required: p_total + 1, actual: n_obs });
    }
    let term_slices: Vec<(&str, usize, usize)> = term_ranges.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();

    let (fixed_terms, sigma2_e, df_residual, reml_core) = if fac_spec.residual_covariance == ResidualCovariance::Unstructured {
        let (cluster_ids, tp_ids, tp_labels): (&[usize], Vec<usize>, Vec<String>) = if is_three_way {
            match fac_spec.repeated_measures {
                RepeatedMeasuresDesign::RowFactorMatched => (&id_sab, row_ids.clone(), factor_levels[2].clone()),
                RepeatedMeasuresDesign::ColumnFactorMatched => (&subj_row_ids, col_ids.clone(), ab_labels),
                _ => {
                    let tps: Vec<usize> = col_ids.iter().zip(row_ids.iter()).map(|(&c, &r)| c * n_rows + r).collect();
                    let mut labels = Vec::with_capacity(n_cols * n_rows);
                    for ab_l in &ab_labels {
                        for c_l in &factor_levels[2] {
                            labels.push(format!("{ab_l} × {c_l}"));
                        }
                    }
                    (&subj_ids, tps, labels)
                }
            }
        } else {
            match fac_spec.repeated_measures {
                RepeatedMeasuresDesign::ColumnFactorMatched => (&subj_row_ids, col_ids.clone(), factor_levels[1].clone()),
                RepeatedMeasuresDesign::RowFactorMatched => (&subj_col_ids, row_ids.clone(), factor_levels[0].clone()),
                _ => {
                    let tps: Vec<usize> = row_ids.iter().zip(col_ids.iter()).map(|(&r, &c)| r * n_cols + c).collect();
                    let mut labels = Vec::with_capacity(n_rows * n_cols);
                    for r_lbl in &factor_levels[0] {
                        for c_lbl in &factor_levels[1] {
                            labels.push(format!("{r_lbl} × {c_lbl}"));
                        }
                    }
                    (&subj_ids, tps, labels)
                }
            }
        };
        emit_unstructured_rm_reml(out, &x_mat, &y_vec, cluster_ids, &tp_ids, &tp_labels, &term_slices, fac_spec.df_method)?
    } else {
        let (fixed_terms, sigma2_e, reml_ll, icc_subj, core) = if is_three_way {
            match fac_spec.repeated_measures {
                RepeatedMeasuresDesign::BothFactorsMatched => {
                    let s_a = format!("subject × {}", factor_names[0]);
                    let s_b = format!("subject × {}", factor_names[1]);
                    let s_c = format!("subject × {}", factor_names[2]);
                    let s_ab = format!("subject × {} × {}", factor_names[0], factor_names[1]);
                    let s_ac = format!("subject × {} × {}", factor_names[0], factor_names[2]);
                    let s_bc = format!("subject × {} × {}", factor_names[1], factor_names[2]);
                    let t_a = format!("Subject × {}", factor_names[0]);
                    let t_b = format!("Subject × {}", factor_names[1]);
                    let t_c = format!("Subject × {}", factor_names[2]);
                    let t_ab = format!("Subject × {} × {}", factor_names[0], factor_names[1]);
                    let t_ac = format!("Subject × {} × {}", factor_names[0], factor_names[2]);
                    let t_bc = format!("Subject × {} × {}", factor_names[1], factor_names[2]);
                    let strata = [
                        ("subject", "Subject (random)", subj_ids.as_slice()),
                        (s_a.as_str(), t_a.as_str(), id_sa.as_slice()),
                        (s_b.as_str(), t_b.as_str(), id_sb.as_slice()),
                        (s_c.as_str(), t_c.as_str(), subj_row_ids.as_slice()),
                        (s_ab.as_str(), t_ab.as_str(), id_sab.as_slice()),
                        (s_ac.as_str(), t_ac.as_str(), id_sac.as_slice()),
                        (s_bc.as_str(), t_bc.as_str(), id_sbc.as_slice()),
                    ];
                    emit_multi_vc_rm_reml(out, &x_mat, &y_vec, &strata, &term_slices, fac_spec.df_method)?
                }
                RepeatedMeasuresDesign::ColumnFactorMatched => {
                    let s_a = format!("subject × {}", factor_names[0]);
                    let s_b = format!("subject × {}", factor_names[1]);
                    let t_a = format!("Subject × {}", factor_names[0]);
                    let t_b = format!("Subject × {}", factor_names[1]);
                    let strata = [
                        ("subject", "Subject (random)", subj_row_ids.as_slice()),
                        (s_a.as_str(), t_a.as_str(), id_sr_a.as_slice()),
                        (s_b.as_str(), t_b.as_str(), id_sr_b.as_slice()),
                    ];
                    emit_multi_vc_rm_reml(out, &x_mat, &y_vec, &strata, &term_slices, fac_spec.df_method)?
                }
                RepeatedMeasuresDesign::RowFactorMatched => {
                    emit_random_intercept_reml(out, &x_mat, &y_vec, &id_sab, &term_slices, fac_spec.df_method, p_fmt)?
                }
                RepeatedMeasuresDesign::None => return Err(StatsError::EmptyInput),
            }
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
                    emit_multi_vc_rm_reml(out, &x_mat, &y_vec, &strata, &term_slices, fac_spec.df_method)?
                }
                RepeatedMeasuresDesign::ColumnFactorMatched => {
                    emit_random_intercept_reml(out, &x_mat, &y_vec, &subj_row_ids, &term_slices, fac_spec.df_method, p_fmt)?
                }
                RepeatedMeasuresDesign::RowFactorMatched => {
                    emit_random_intercept_reml(out, &x_mat, &y_vec, &subj_col_ids, &term_slices, fac_spec.df_method, p_fmt)?
                }
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
        (fixed_terms, sigma2_e, (n_obs - p_total) as f64, core)
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
    Ok(GroupedFactorialOutcome { result: res, observations: data, post_hoc: FactorialPostHocContext::Reml(reml_model), aux_tabs: Vec::new() })
}

pub(in crate::anova) fn run_multifactor_mv(
    mv: &curvebook_doc::MultipleVariablesTable,
    fac_spec: &FactorialAnovaSpec,
) -> Result<(FactorialAnovaResult, FactorialObservations, OlsPostHocModel), StatsError> {
    let resp_idx = fac_spec.response_column.ok_or(StatsError::MissingColumn { index: 0 })?;
    let resp_var = mv.variables.get(resp_idx).ok_or(StatsError::MissingColumn { index: resp_idx })?;
    if fac_spec.multifactor_columns.is_empty() {
        return Err(StatsError::InsufficientSampleSize { context: "multifactor columns", required: 1, actual: 0 });
    }
    if fac_spec.three_way && fac_spec.multifactor_columns.len() != 3 {
        return Err(StatsError::InvalidParameter { name: "three_way_factor_count", value: fac_spec.multifactor_columns.len() as f64 });
    }
    let mut factor_names = Vec::with_capacity(fac_spec.multifactor_columns.len());
    let mut factor_levels = Vec::with_capacity(fac_spec.multifactor_columns.len());
    for &c_idx in &fac_spec.multifactor_columns {
        let var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
        factor_names.push(var.title.clone());
        let levels: Vec<String> = var.ordered_levels_with(&var.level_order).into_iter().map(|s| s.level).collect();
        factor_levels.push(levels);
    }

    let n_rows = mv.row_count();
    let mut obs = Vec::new();
    for r in 0..n_rows {
        let Some(y) = resp_var.cells.get(r).and_then(|c| c.active_number()) else {
            continue;
        };
        let mut lvls = Vec::with_capacity(fac_spec.multifactor_columns.len());
        let mut valid = true;
        for (f_pos, &c_idx) in fac_spec.multifactor_columns.iter().enumerate() {
            let factor_var = mv.variables.get(c_idx).ok_or(StatsError::MissingColumn { index: c_idx })?;
            let cell = factor_var.cells.get(r).ok_or(StatsError::DimensionMismatch { expected: n_rows, actual: factor_var.cells.len() })?;
            if cell.excluded {
                valid = false;
                break;
            }
            let tok = match &cell.value {
                curvebook_doc::CellValue::Text(s) => s.trim().to_owned(),
                curvebook_doc::CellValue::Number(v) if v.is_finite() => format!("{v}"),
                _ => {
                    valid = false;
                    break;
                }
            };
            let Some(pos) = factor_levels[f_pos].iter().position(|l| l == &tok) else {
                valid = false;
                break;
            };
            lvls.push(pos);
        }
        if valid {
            obs.push((lvls, y));
        }
    }
    let data = FactorialObservations { factor_names, factor_levels, rows: obs };
    let (res, model) = fit_factorial_anova_with_model(&data, fac_spec.max_interaction_order, fac_spec.ss_type)?;
    Ok((res, data, model))
}
