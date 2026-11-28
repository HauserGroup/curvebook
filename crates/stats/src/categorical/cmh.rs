//! Stratified `2 × 2 × K` Cochran–Mantel–Haenszel contingency analysis, common odds ratio with
//! Robins–Breslow–Greenland (1986) confidence interval, and Breslow–Day / Tarone (1985) test for
//! homogeneity of odds ratios across strata.
//!
//! References:
//! - Agresti, A. (2013). *Categorical Data Analysis* (3rd ed.). Hoboken, NJ: Wiley.
//! - Breslow, N. E., & Day, N. E. (1980). *Statistical Methods in Cancer Research, Vol. 1: The
//!   Analysis of Case-Control Studies*. Lyon: IARC Scientific Publications No. 32.
//! - Mantel, N., & Haenszel, W. (1959). Statistical aspects of the analysis of data from
//!   retrospective studies of disease. *Journal of the National Cancer Institute*, 22(4), 719-748.
//! - Robins, J., Breslow, N., & Greenland, S. (1986). Estimators of the Mantel–Haenszel variance
//!   consistent in both sparse data and large-strata limiting models. *Biometrics*, 42(2),
//!   311-323. `doi:10.2307/2531052`.
//! - Tarone, R. E. (1985). On heterogeneity tests based on efficient scores. *Biometrika*, 72(1),
//!   91-95. `doi:10.1093/biomet/72.1.91`.

use std::collections::BTreeMap;

use curvebook_doc::{
    AnalysisSpec, ChecklistStatus, CmhSpec, DataSheet, ResultCell, ResultSheet, ResultTabKind, TableArchetype, VariableColumn, VariableRole,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{chi2_sf, norm_two_sided_isf};
use curvebook_math::sum::NeumaierSum;
use serde::{Deserialize, Serialize};

use crate::common::{
    TabBuilder, append_listwise_filtered_checklist_and_footnote, checklist_item, init_result_sheet, num_cell, opt_ci_cell, opt_num_cell,
    p_cell_at_alpha, push_scalar, sanitize_result_sheet_floats, try_resolve_selected_indices,
};
use crate::error::StatsError;

/// Single `2 × 2` contingency table stratum `[a, b; c, d]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CmhStratum {
    /// Stratum label (for example `"Center 1"` or `"Age >= 65"`).
    pub label: String,
    /// Exposed / row-1, event / col-1 count (`a_k`).
    pub a: u64,
    /// Exposed / row-1, non-event / col-2 count (`b_k`).
    pub b: u64,
    /// Unexposed / row-2, event / col-1 count (`c_k`).
    pub c: u64,
    /// Unexposed / row-2, non-event / col-2 count (`d_k`).
    pub d: u64,
}

/// Per-stratum summary quantities emitted alongside the pooled CMH statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmhStratumSummary {
    /// Stratum display label.
    pub label: String,
    /// Cell count `a_k`.
    pub a: u64,
    /// Cell count `b_k`.
    pub b: u64,
    /// Cell count `c_k`.
    pub c: u64,
    /// Cell count `d_k`.
    pub d: u64,
    /// Total observations `n_k = a_k + b_k + c_k + d_k`.
    pub total: u64,
    /// Hypergeometric null expectation `E_k = (a_k + b_k)(a_k + c_k) / n_k`.
    pub expected_a: f64,
    /// Hypergeometric null variance `V_k = r1_k r2_k c1_k c2_k / (n_k^2 (n_k - 1))`.
    pub variance_a: f64,
    /// Within-stratum empirical odds ratio `(a_k * d_k) / (b_k * c_k)` (`None` if zero denominator).
    pub stratum_or: Option<f64>,
    /// Fitted count $\tilde{a}_k$ under the common Mantel–Haenszel odds ratio (`None` if degenerate).
    pub fitted_a_mh: Option<f64>,
}

/// Complete stratified `2 × 2 × K` Cochran–Mantel–Haenszel test output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmhResult {
    /// Total number of non-empty strata analyzed.
    pub n_strata: usize,
    /// Number of strata with strictly positive hypergeometric variance (`r1, r2, c1, c2 > 0`).
    pub informative_strata: usize,
    /// Grand total count across all strata.
    pub total_count: u64,
    /// Continuity-corrected CMH $\chi^2$ statistic `max(0, |sum(a_k - E_k)| - 0.5)^2 / sum(V_k)` (`df = 1`).
    pub cmh_chi2_corrected: f64,
    /// Upper-tail $\chi^2(1)$ *P* value for `cmh_chi2_corrected`.
    pub cmh_p_corrected: f64,
    /// Uncorrected CMH $\chi^2$ statistic `(sum(a_k - E_k))^2 / sum(V_k)` (`df = 1`).
    pub cmh_chi2_uncorrected: f64,
    /// Upper-tail $\chi^2(1)$ *P* value for `cmh_chi2_uncorrected`.
    pub cmh_p_uncorrected: f64,
    /// Common Mantel–Haenszel odds ratio $\widehat{\text{OR}}_{\text{MH}} = R / S$ (`None` if `S == 0` and `R == 0`, or `+inf` mapped to `None` when `S == 0`).
    pub common_or: Option<f64>,
    /// Robins–Breslow–Greenland (1986) asymptotic standard error of $\ln(\widehat{\text{OR}}_{\text{MH}})$.
    pub rbg_se_log_or: Option<f64>,
    /// Two-sided Robins–Breslow–Greenland confidence interval for `common_or` at configured `ConfidenceLevel`.
    pub common_or_ci: Option<(f64, f64)>,
    /// Classical Breslow–Day (1980) homogeneity statistic across strata (`None` when `< 2` informative strata or non-estimable common OR).
    pub breslow_day_chi2: Option<f64>,
    /// Tarone-corrected (1985) Breslow–Day homogeneity statistic (`None` when non-estimable).
    pub breslow_day_tarone_chi2: Option<f64>,
    /// Homogeneity test degrees of freedom (`informative_strata - 1`).
    pub breslow_day_df: Option<f64>,
    /// Upper-tail *P* value for classical Breslow–Day statistic.
    pub breslow_day_p: Option<f64>,
    /// Upper-tail *P* value for Tarone-corrected Breslow–Day statistic.
    pub breslow_day_tarone_p: Option<f64>,
    /// Per-stratum diagnostic details.
    pub strata: Vec<CmhStratumSummary>,
}

/// Solve the Mantel–Haenszel constant-OR quadratic for fitted cell count $\tilde{a}_k$ in
/// $[\max(0, r_{1k} + c_{1k} - n_k), \min(r_{1k}, c_{1k})]$.
fn solve_fitted_a(r1: f64, c1: f64, n: f64, psi: f64) -> Option<f64> {
    if !psi.is_finite() || psi <= 0.0 || r1 <= 0.0 || c1 <= 0.0 || r1 >= n || c1 >= n {
        return None;
    }
    let lo = (r1 + c1 - n).max(0.0);
    let hi = r1.min(c1);
    if hi <= lo {
        return None;
    }
    if (psi - 1.0).abs() <= 1e-12 {
        return Some(r1 * c1 / n);
    }
    let a_coef = psi - 1.0;
    let b_coef = -(r1 * psi + c1 * psi + (n - r1 - c1));
    let c_coef = psi * r1 * c1;
    let disc = (b_coef * b_coef - 4.0 * a_coef * c_coef).max(0.0).sqrt();
    let root1 = (-b_coef + disc) / (2.0 * a_coef);
    let root2 = (-b_coef - disc) / (2.0 * a_coef);
    let eps = 1e-9 * n.max(1.0);
    if root1 >= lo - eps && root1 <= hi + eps {
        Some(root1.clamp(lo, hi))
    } else if root2 >= lo - eps && root2 <= hi + eps {
        Some(root2.clamp(lo, hi))
    } else {
        None
    }
}

/// Compute the stratified `2 × 2 × K` Cochran–Mantel–Haenszel test (with and without continuity
/// correction), Mantel–Haenszel common odds ratio with Robins–Breslow–Greenland (1986) CI, and
/// Breslow–Day / Tarone (1985) test for homogeneity of odds ratios across strata.
///
/// # Errors
/// Returns [`StatsError`] when `strata` is empty, counts overflow `u64`, or zero informative
/// strata have positive hypergeometric variance.
pub fn cochran_mantel_haenszel(strata: &[CmhStratum], confidence: ConfidenceLevel) -> Result<CmhResult, StatsError> {
    if strata.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let mut total_count = 0_u64;
    let mut non_empty_strata = 0_usize;
    let mut informative_strata = 0_usize;

    let mut delta_sum = NeumaierSum::new();
    let mut var_sum = NeumaierSum::new();
    let mut r_sum = NeumaierSum::new();
    let mut s_sum = NeumaierSum::new();
    let mut pr_sum = NeumaierSum::new();
    let mut ps_plus_qr_sum = NeumaierSum::new();
    let mut qs_sum = NeumaierSum::new();

    let mut summaries = Vec::with_capacity(strata.len());

    for st in strata {
        let r1_u = st.a.checked_add(st.b).ok_or(StatsError::InvalidParameter { name: "cmh_count", value: f64::INFINITY })?;
        let r2_u = st.c.checked_add(st.d).ok_or(StatsError::InvalidParameter { name: "cmh_count", value: f64::INFINITY })?;
        let n_u = r1_u.checked_add(r2_u).ok_or(StatsError::InvalidParameter { name: "cmh_count", value: f64::INFINITY })?;
        total_count = total_count.checked_add(n_u).ok_or(StatsError::InvalidParameter { name: "cmh_total", value: f64::INFINITY })?;

        if n_u == 0 {
            continue;
        }
        non_empty_strata += 1;

        let a = st.a as f64;
        let b = st.b as f64;
        let c = st.c as f64;
        let d = st.d as f64;
        let r1 = a + b;
        let r2 = c + d;
        let c1 = a + c;
        let c2 = b + d;
        let n = n_u as f64;

        let e_k = (r1 * c1) / n;
        let v_k = if n_u > 1 { (r1 * r2 * c1 * c2) / (n * n * (n - 1.0)) } else { 0.0 };
        if v_k > 0.0 {
            informative_strata += 1;
        }
        delta_sum.add(a - e_k);
        var_sum.add(v_k);

        let r_k = (a * d) / n;
        let s_k = (b * c) / n;
        let p_k = (a + d) / n;
        let q_k = (b + c) / n;
        r_sum.add(r_k);
        s_sum.add(s_k);
        pr_sum.add(p_k * r_k);
        ps_plus_qr_sum.add(p_k * s_k + q_k * r_k);
        qs_sum.add(q_k * s_k);

        let stratum_or = if st.b > 0 && st.c > 0 { Some((a * d) / (b * c)) } else { None };
        summaries.push(CmhStratumSummary {
            label: st.label.clone(),
            a: st.a,
            b: st.b,
            c: st.c,
            d: st.d,
            total: n_u,
            expected_a: e_k,
            variance_a: v_k,
            stratum_or,
            fitted_a_mh: None,
        });
    }

    if non_empty_strata == 0 {
        return Err(StatsError::EmptyInput);
    }

    let delta = delta_sum.total();
    let v_tot = var_sum.total();
    if !v_tot.is_finite() || v_tot <= 0.0 {
        return Err(StatsError::ZeroVariance { context: "Cochran-Mantel-Haenszel pooled hypergeometric variance" });
    }

    let cmh_chi2_uncorrected = (delta * delta) / v_tot;
    let cmh_p_uncorrected = chi2_sf(cmh_chi2_uncorrected, 1.0)?;

    let corr_num = (delta.abs() - 0.5).max(0.0);
    let cmh_chi2_corrected = (corr_num * corr_num) / v_tot;
    let cmh_p_corrected = chi2_sf(cmh_chi2_corrected, 1.0)?;

    // Common Mantel-Haenszel odds ratio and Robins-Breslow-Greenland (1986) CI.
    let r_tot = r_sum.total();
    let s_tot = s_sum.total();
    let (common_or, rbg_se_log_or, common_or_ci) = if r_tot > 0.0 && s_tot > 0.0 {
        let or_mh = r_tot / s_tot;
        let var_log_or =
            (pr_sum.total() / (2.0 * r_tot * r_tot)) + (ps_plus_qr_sum.total() / (2.0 * r_tot * s_tot)) + (qs_sum.total() / (2.0 * s_tot * s_tot));
        let se_log = var_log_or.max(0.0).sqrt();
        let z = norm_two_sided_isf(confidence.alpha())?;
        let log_or = or_mh.ln();
        let lo = (log_or - z * se_log).exp();
        let hi = (log_or + z * se_log).exp();
        (Some(or_mh), Some(se_log), (lo.is_finite() && hi.is_finite()).then_some((lo, hi)))
    } else if r_tot == 0.0 && s_tot > 0.0 {
        (Some(0.0), None, None)
    } else {
        (None, None, None)
    };

    // Breslow–Day (1980) and Tarone (1985) test for homogeneity of odds ratios across strata.
    let mut bd_sum = NeumaierSum::new();
    let mut diff_sum = NeumaierSum::new();
    let mut v_bd_sum = NeumaierSum::new();
    let mut bd_strata_used = 0_usize;

    if let Some(psi) = common_or
        && psi > 0.0
        && psi.is_finite()
    {
        for s in &mut summaries {
            let a = s.a as f64;
            let b = s.b as f64;
            let c = s.c as f64;
            let r1 = a + b;
            let c1 = a + c;
            let n = s.total as f64;
            if let Some(a_hat) = solve_fitted_a(r1, c1, n, psi) {
                let b_hat = r1 - a_hat;
                let c_hat = c1 - a_hat;
                let d_hat = n - r1 - c1 + a_hat;
                if a_hat > 0.0 && b_hat > 0.0 && c_hat > 0.0 && d_hat > 0.0 {
                    s.fitted_a_mh = Some(a_hat);
                    let inv_v = 1.0 / a_hat + 1.0 / b_hat + 1.0 / c_hat + 1.0 / d_hat;
                    let v_k = 1.0 / inv_v;
                    let diff = a - a_hat;
                    bd_sum.add((diff * diff) / v_k);
                    diff_sum.add(diff);
                    v_bd_sum.add(v_k);
                    bd_strata_used += 1;
                }
            }
        }
    }

    let (breslow_day_chi2, breslow_day_tarone_chi2, breslow_day_df, breslow_day_p, breslow_day_tarone_p) = if bd_strata_used >= 2 {
        let df_bd = (bd_strata_used - 1) as f64;
        let bd = bd_sum.total().max(0.0);
        let d_tot = diff_sum.total();
        let v_tot_bd = v_bd_sum.total();
        let tarone = if v_tot_bd > 0.0 { (bd - (d_tot * d_tot) / v_tot_bd).max(0.0) } else { bd };
        (Some(bd), Some(tarone), Some(df_bd), Some(chi2_sf(bd, df_bd)?), Some(chi2_sf(tarone, df_bd)?))
    } else {
        (None, None, None, None, None)
    };

    Ok(CmhResult {
        n_strata: non_empty_strata,
        informative_strata,
        total_count,
        cmh_chi2_corrected,
        cmh_p_corrected,
        cmh_chi2_uncorrected,
        cmh_p_uncorrected,
        common_or,
        rbg_se_log_or,
        common_or_ci,
        breslow_day_chi2,
        breslow_day_tarone_chi2,
        breslow_day_df,
        breslow_day_p,
        breslow_day_tarone_p,
        strata: summaries,
    })
}

fn mv_token(col: &VariableColumn, row: usize) -> Option<String> {
    let cell = col.cells.get(row)?;
    if cell.excluded || cell.is_empty() {
        return None;
    }
    match &cell.value {
        curvebook_doc::CellValue::Text(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_owned())
        }
        curvebook_doc::CellValue::Number(v) if v.is_finite() => {
            if v.fract().abs() < 1e-12 {
                Some(format!("{v:.0}"))
            } else {
                Some(format!("{v}"))
            }
        }
        _ => None,
    }
}

fn parse_nonneg_int_count(v: f64) -> Result<u64, StatsError> {
    if !v.is_finite() || v < 0.0 || (v - v.round()).abs() > 1e-9 || v > (u64::MAX as f64) {
        return Err(StatsError::InvalidParameter { name: "cmh_count", value: v });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v.round() as u64)
}

fn extract_cmh_contingency_strata(
    table: &curvebook_doc::ContingencyTable,
    selected_data_sets: &[usize],
) -> Result<(Vec<CmhStratum>, usize), StatsError> {
    table.validate_counts()?;
    let selected = try_resolve_selected_indices(selected_data_sets, table.data_sets.len())?;
    if selected.len() != 4 {
        return Err(StatsError::InvalidParameter { name: "cmh_contingency_columns", value: selected.len() as f64 });
    }
    let mut strata = Vec::new();
    let mut omitted_rows = 0_usize;
    for row in 0..table.row_count() {
        let mut vals = [0_u64; 4];
        let mut any_non_empty = false;
        let mut complete = true;
        for (slot, &c_idx) in selected.iter().enumerate() {
            let cell = &table.data_sets[c_idx].cells[row][0];
            if cell.is_empty() || cell.excluded {
                complete = false;
            } else if let Some(v) = cell.active_number() {
                any_non_empty = true;
                vals[slot] = parse_nonneg_int_count(v)?;
            } else {
                complete = false;
            }
        }
        if complete && any_non_empty {
            let lbl =
                table.row_titles.get(row).map(|s| s.trim()).filter(|s| !s.is_empty()).map_or_else(|| format!("Stratum {}", row + 1), str::to_owned);
            strata.push(CmhStratum { label: lbl, a: vals[0], b: vals[1], c: vals[2], d: vals[3] });
        } else if any_non_empty {
            omitted_rows += 1;
        }
    }
    Ok((strata, omitted_rows))
}

fn pick_cmh_cat_column(
    explicit: Option<usize>,
    fallback_pos: usize,
    cat_cols: &[usize],
    n_vars: usize,
    used: &[usize],
    name: &'static str,
) -> Result<usize, StatsError> {
    if let Some(idx) = explicit {
        if idx >= n_vars || used.contains(&idx) {
            return Err(StatsError::InvalidParameter { name, value: idx as f64 });
        }
        return Ok(idx);
    }
    cat_cols
        .get(fallback_pos)
        .copied()
        .filter(|c| !used.contains(c))
        .or_else(|| (0..n_vars).find(|c| !used.contains(c)))
        .ok_or(StatsError::InvalidParameter { name, value: fallback_pos as f64 })
}

fn order_active_levels(col: &VariableColumn, active_first_seen: Vec<String>) -> Vec<String> {
    let mut ordered = Vec::with_capacity(active_first_seen.len());
    for s in col.ordered_levels() {
        if active_first_seen.contains(&s.level) && !ordered.contains(&s.level) {
            ordered.push(s.level);
        }
    }
    for lvl in active_first_seen {
        if !ordered.contains(&lvl) {
            ordered.push(lvl);
        }
    }
    ordered
}

fn extract_cmh_mv_strata(mv: &curvebook_doc::MultipleVariablesTable, cmh_spec: &CmhSpec) -> Result<(Vec<CmhStratum>, usize, String), StatsError> {
    let n_vars = mv.variables.len();
    if n_vars < 3 {
        return Err(StatsError::InsufficientSampleSize { context: "CMH columns (exposure, outcome, stratum)", required: 3, actual: n_vars });
    }
    let cat_cols: Vec<usize> = (0..n_vars).filter(|&i| mv.variables[i].role == VariableRole::Categorical).collect();
    let exp_col = pick_cmh_cat_column(cmh_spec.exposure_column, 0, &cat_cols, n_vars, &[], "exposure_column")?;
    let out_col = pick_cmh_cat_column(cmh_spec.outcome_column, 1, &cat_cols, n_vars, &[exp_col], "outcome_column")?;
    let str_col = pick_cmh_cat_column(cmh_spec.stratum_column, 2, &cat_cols, n_vars, &[exp_col, out_col], "stratum_column")?;
    let cnt_col = match cmh_spec.count_column {
        Some(idx) if idx < n_vars && idx != exp_col && idx != out_col && idx != str_col => Some(idx),
        Some(idx) => return Err(StatsError::InvalidParameter { name: "count_column", value: idx as f64 }),
        None => (0..n_vars).find(|&i| {
            i != exp_col && i != out_col && i != str_col && matches!(mv.variables[i].role, VariableRole::Continuous | VariableRole::Calculated)
        }),
    };

    let mut exp_seen: Vec<String> = Vec::new();
    let mut out_seen: Vec<String> = Vec::new();
    let mut str_seen: Vec<String> = Vec::new();

    let mut rows_parsed: Vec<(String, String, String, u64)> = Vec::new();
    let mut omitted_rows = 0_usize;
    for row in 0..mv.row_count() {
        let e_tok = mv_token(&mv.variables[exp_col], row);
        let o_tok = mv_token(&mv.variables[out_col], row);
        let s_tok = mv_token(&mv.variables[str_col], row);
        let c_val = match cnt_col {
            Some(cc) => match mv.variables[cc].cells.get(row).and_then(|c| c.active_number()) {
                Some(v) => Some(parse_nonneg_int_count(v)?),
                None => None,
            },
            None => Some(1_u64),
        };
        if let (Some(e), Some(o), Some(s), Some(cnt)) = (e_tok, o_tok, s_tok, c_val) {
            if !exp_seen.contains(&e) {
                exp_seen.push(e.clone());
            }
            if !out_seen.contains(&o) {
                out_seen.push(o.clone());
            }
            if !str_seen.contains(&s) {
                str_seen.push(s.clone());
            }
            rows_parsed.push((e, o, s, cnt));
        } else if [exp_col, out_col, str_col].into_iter().any(|c| mv.variables[c].cells.get(row).is_some_and(|cell| !cell.is_empty())) {
            omitted_rows += 1;
        }
    }

    let exp_levels = order_active_levels(&mv.variables[exp_col], exp_seen);
    let out_levels = order_active_levels(&mv.variables[out_col], out_seen);
    let str_levels = order_active_levels(&mv.variables[str_col], str_seen);

    if exp_levels.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "exposure_levels_must_be_2", value: exp_levels.len() as f64 });
    }
    if out_levels.len() != 2 {
        return Err(StatsError::InvalidParameter { name: "outcome_levels_must_be_2", value: out_levels.len() as f64 });
    }
    let note = format!(
        "Exposure levels: [{} vs {}]; Outcome levels: [{} vs {}]; Stratum column: {}.",
        exp_levels[0], exp_levels[1], out_levels[0], out_levels[1], mv.variables[str_col].title
    );

    let mut cells_by_stratum: BTreeMap<String, [u64; 4]> = BTreeMap::new();
    for (e, o, s, cnt) in rows_parsed {
        let r_idx = usize::from(e != exp_levels[0]);
        let c_idx = usize::from(o != out_levels[0]);
        let slot = r_idx * 2 + c_idx;
        let entry = cells_by_stratum.entry(s).or_insert([0; 4]);
        entry[slot] = entry[slot].checked_add(cnt).ok_or(StatsError::InvalidParameter { name: "cmh_count", value: f64::INFINITY })?;
    }
    let mut strata = Vec::new();
    for lbl in str_levels {
        if let Some(arr) = cells_by_stratum.remove(&lbl) {
            strata.push(CmhStratum { label: lbl, a: arr[0], b: arr[1], c: arr[2], d: arr[3] });
        }
    }
    Ok((strata, omitted_rows, note))
}

fn push_cmh_scalars(out: &mut ResultSheet, res: &CmhResult, primary_chi2: f64, primary_p: f64) {
    push_scalar(out, "CMH chi-square", None, primary_chi2);
    push_scalar(out, "CMH P", None, primary_p);
    push_scalar(out, "CMH chi-square (continuity-corrected)", None, res.cmh_chi2_corrected);
    push_scalar(out, "CMH P (continuity-corrected)", None, res.cmh_p_corrected);
    push_scalar(out, "CMH chi-square (uncorrected)", None, res.cmh_chi2_uncorrected);
    push_scalar(out, "CMH P (uncorrected)", None, res.cmh_p_uncorrected);
    if let Some(or_val) = res.common_or {
        push_scalar(out, "Common OR (Mantel-Haenszel)", None, or_val);
    }
    if let Some(se_log) = res.rbg_se_log_or {
        push_scalar(out, "SE(ln Common OR)", None, se_log);
    }
    if let Some((lo, hi)) = res.common_or_ci {
        push_scalar(out, "Common OR CI lower", None, lo);
        push_scalar(out, "Common OR CI upper", None, hi);
    }
    if let Some(bd) = res.breslow_day_chi2 {
        push_scalar(out, "Breslow-Day chi-square", None, bd);
    }
    if let Some(bdt) = res.breslow_day_tarone_chi2 {
        push_scalar(out, "Breslow-Day-Tarone chi-square", None, bdt);
    }
    if let Some(df_bd) = res.breslow_day_df {
        push_scalar(out, "Breslow-Day df", None, df_bd);
    }
    if let Some(p_bd) = res.breslow_day_p {
        push_scalar(out, "Breslow-Day P", None, p_bd);
    }
    if let Some(p_bdt) = res.breslow_day_tarone_p {
        push_scalar(out, "Breslow-Day-Tarone P", None, p_bdt);
    }
    push_scalar(out, "Strata", None, res.n_strata as f64);
    push_scalar(out, "Total N", None, res.total_count as f64);
}

/// Execute a stratified `2 × 2 × K` Cochran–Mantel–Haenszel analysis against a `Contingency`
/// (`K × 4` `[a, b, c, d]` strata rows) or `MultipleVariables` (`exposure`, `outcome`, `stratum`,
/// optional `count`) [`DataSheet`].
pub fn run_cmh_analysis(sheet: &DataSheet, spec: &AnalysisSpec, cmh_spec: &CmhSpec) -> Result<ResultSheet, StatsError> {
    let (strata, omitted_rows, exposure_levels_note) = match &sheet.archetype {
        TableArchetype::Contingency(table) => {
            let (s, omitted) = extract_cmh_contingency_strata(table, &spec.selected_data_sets)?;
            (s, omitted, String::new())
        }
        TableArchetype::MultipleVariables(mv) => extract_cmh_mv_strata(mv, cmh_spec)?,
        other => {
            return Err(StatsError::IncompatibleTable { analysis: "Cochran-Mantel-Haenszel test", actual: other.kind() });
        }
    };

    let confidence = spec.confidence_level;
    let alpha = confidence.alpha();
    let p_fmt = spec.p_value_format;
    let res = cochran_mantel_haenszel(&strata, confidence)?;

    let mut out = init_result_sheet(spec)?;
    let (primary_chi2, primary_p) = if cmh_spec.continuity_correction {
        (res.cmh_chi2_corrected, res.cmh_p_corrected)
    } else {
        (res.cmh_chi2_uncorrected, res.cmh_p_uncorrected)
    };
    push_cmh_scalars(&mut out, &res, primary_chi2, primary_p);

    // Tab 1: Main CMH & Homogeneity summary.
    let ci_pct = confidence.percent();
    let mut main_tab = TabBuilder::new("cmh-summary", "Cochran-Mantel-Haenszel summary", ResultTabKind::MainSummary).headers([
        String::from("Estimate / Statistic"),
        String::from("df"),
        String::from("P value"),
        format!("{ci_pct:.0}% CI"),
    ]);
    main_tab
        .push_row(
            "CMH chi-square (continuity-corrected)",
            [num_cell(res.cmh_chi2_corrected), num_cell(1.0), p_cell_at_alpha(res.cmh_p_corrected, p_fmt, alpha), ResultCell::Empty],
        )
        .push_row(
            "CMH chi-square (uncorrected)",
            [num_cell(res.cmh_chi2_uncorrected), num_cell(1.0), p_cell_at_alpha(res.cmh_p_uncorrected, p_fmt, alpha), ResultCell::Empty],
        )
        .push_row(
            "Common odds ratio (Mantel-Haenszel, RBG CI)",
            [opt_num_cell(res.common_or), ResultCell::Empty, ResultCell::Empty, opt_ci_cell(res.common_or_ci)],
        )
        .push_row(
            "Breslow-Day homogeneity test",
            [
                opt_num_cell(res.breslow_day_chi2),
                opt_num_cell(res.breslow_day_df),
                res.breslow_day_p.map_or(ResultCell::Empty, |p| p_cell_at_alpha(p, p_fmt, alpha)),
                ResultCell::Empty,
            ],
        )
        .push_row(
            "Breslow-Day-Tarone homogeneity test",
            [
                opt_num_cell(res.breslow_day_tarone_chi2),
                opt_num_cell(res.breslow_day_df),
                res.breslow_day_tarone_p.map_or(ResultCell::Empty, |p| p_cell_at_alpha(p, p_fmt, alpha)),
                ResultCell::Empty,
            ],
        );
    let mut main_tab = main_tab.build();
    main_tab.footnotes.push(format!(
        "Analyzed {} strata ({} informative, N = {}). Common OR CI uses the Robins-Breslow-Greenland (1986) variance estimator.",
        res.n_strata, res.informative_strata, res.total_count
    ));
    if !exposure_levels_note.is_empty() {
        main_tab.footnotes.push(exposure_levels_note);
    }
    out.tabs.push(main_tab);

    // Tab 2: Per-stratum breakdown.
    let mut strata_tab = TabBuilder::new("cmh-strata", "Per-stratum 2 x 2 tables", ResultTabKind::ParametersAndConfidenceIntervals).headers([
        "a",
        "b",
        "c",
        "d",
        "Total",
        "Expected a",
        "Var(a)",
        "Stratum OR",
        "Fitted a (MH)",
    ]);
    for s in &res.strata {
        strata_tab.push_row(
            s.label.clone(),
            [
                num_cell(s.a as f64),
                num_cell(s.b as f64),
                num_cell(s.c as f64),
                num_cell(s.d as f64),
                num_cell(s.total as f64),
                num_cell(s.expected_a),
                num_cell(s.variance_a),
                opt_num_cell(s.stratum_or),
                opt_num_cell(s.fitted_a_mh),
            ],
        );
    }
    out.tabs.push(strata_tab.build());

    // Checklist items.
    let fmt_p = |p: f64| if p < 1e-4 { format!("{p:.3e}") } else { format!("{p:.4}") };
    out.checklist.push(checklist_item(
        "Conditional independence (CMH test)",
        ChecklistStatus::Info,
        Some(format!("M^2 = {primary_chi2:.4}, P = {}", fmt_p(primary_p))),
        if primary_p < alpha {
            "Cochran-Mantel-Haenszel test rejects conditional independence across strata at the configured significance threshold."
        } else {
            "Cochran-Mantel-Haenszel test does not reject conditional independence across strata at the configured significance threshold."
        },
    ));
    if let Some(p_tarone) = res.breslow_day_tarone_p {
        let hetero = p_tarone < alpha;
        out.checklist.push(checklist_item(
            "Homogeneity of odds ratios (Breslow-Day-Tarone)",
            if hetero { ChecklistStatus::Warn } else { ChecklistStatus::Pass },
            Some(format!(
                "X^2 = {:.4}, df = {:.0}, P = {}",
                res.breslow_day_tarone_chi2.unwrap_or(0.0),
                res.breslow_day_df.unwrap_or(0.0),
                fmt_p(p_tarone)
            )),
            if hetero {
                "Breslow-Day-Tarone test indicates significant heterogeneity of odds ratios across strata; interpret the pooled common OR with caution."
            } else {
                "Breslow-Day-Tarone test shows no significant heterogeneity of odds ratios across strata, supporting a pooled common OR."
            },
        ));
    }
    append_listwise_filtered_checklist_and_footnote(&mut out, "CMH stratified analysis", omitted_rows, res.total_count as usize);

    sanitize_result_sheet_floats(&mut out)?;
    Ok(out)
}
