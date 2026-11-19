//! Post-hoc contrasts for factorial, repeated-measures, and mixed-model ANOVA: estimability checks,
//! simple effects, contrast families, Dunnett, and Newman–Keuls comparisons.
//!
//! References:
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - Copenhaver, M. D., & Holland, B. S. (1988). Computation of the distribution of the maximum
//!   studentized range statistic with application to multiple significance testing of simple
//!   effects. *Journal of Statistical Computation and Simulation* 30(1):1–15.
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558

use std::collections::BTreeSet;

use curvebook_doc::{
    MixedModelDfMethod, MultiplicityMethod, MultiplicitySpec, PValueFormat, PairwiseComparisonRecord, RepeatedMeasuresDesign, ResultSheet,
};
use curvebook_math::ConfidenceLevel;
use curvebook_math::dist::{
    dunnett_two_sided_crit_loadings, dunnett_two_sided_p_loadings, max_studentized_range_isf, max_studentized_range_sf, student_t_two_sided_isf,
    student_t_two_sided_p,
};
use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::SummaryMoments;

use crate::anova::factorial::{FactorialObservations, OlsPostHocModel};
use crate::anova::results::FactorialAnovaResult;
use crate::anova::rm_two_way::RmBalancedPostHocModel;
use crate::error::StatsError;
use crate::multiplicity::{MultiplicityProcedure, PairwiseComparisonDetail, adjust_p_values, build_multiplicity_tab};

#[derive(Debug, Clone)]
pub(in crate::anova) struct RemlContrastModel {
    pub(in crate::anova) factor_levels: Vec<usize>,
    pub(in crate::anova) factor_basis: Vec<Vec<Vec<f64>>>,
    pub(in crate::anova) subsets: Vec<Vec<usize>>,
    pub(in crate::anova) total_cols: usize,
    pub(in crate::anova) row_proj: DenseMatrix,
    pub(in crate::anova) col_map: Vec<Option<usize>>,
    pub(in crate::anova) beta: Vec<f64>,
    pub(in crate::anova) phi: DenseMatrix,
    pub(in crate::anova) phi_a: DenseMatrix,
    pub(in crate::anova) w_info_inv: DenseMatrix,
    pub(in crate::anova) phi_m_phi: Vec<DenseMatrix>,
    pub(in crate::anova) n_minus_p: f64,
    pub(in crate::anova) y_scale: f64,
    pub(in crate::anova) df_method: MixedModelDfMethod,
    pub(in crate::anova) observed_cells: BTreeSet<Vec<usize>>,
}

#[derive(Debug, Clone)]
pub(in crate::anova) enum FactorialPostHocContext {
    Ols(OlsPostHocModel),
    RmBalanced(RmBalancedPostHocModel),
    Reml(RemlContrastModel),
}

pub(in crate::anova) struct ContrastEstimate {
    pub(in crate::anova) diff: f64,
    pub(in crate::anova) se: f64,
    pub(in crate::anova) df: f64,
    pub(in crate::anova) ctrl_var_pair: Option<(f64, f64)>,
    pub(in crate::anova) cov_vectors: Option<(Vec<f64>, Vec<f64>)>,
}

#[derive(Clone, Copy)]
pub(in crate::anova) struct NamedSliceContrast<'a> {
    pub(in crate::anova) id_a: usize,
    pub(in crate::anova) id_b: usize,
    pub(in crate::anova) label_a: &'a str,
    pub(in crate::anova) label_b: &'a str,
    pub(in crate::anova) diff: f64,
    pub(in crate::anova) se: f64,
    pub(in crate::anova) df: f64,
}

#[inline]
pub(in crate::anova) fn dot_slice(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

pub(in crate::anova) fn quad_form(v: &[f64], mat: &DenseMatrix) -> f64 {
    let n = v.len();
    let mut acc = 0.0_f64;
    for (r, &vr) in v.iter().enumerate() {
        if vr == 0.0 {
            continue;
        }
        let mut row_dot = 0.0_f64;
        for (c, &vc) in v.iter().enumerate() {
            if vc != 0.0 {
                row_dot += mat.get(r, c).unwrap_or(0.0) * vc;
            }
        }
        acc += vr * row_dot;
    }
    let _ = n;
    acc
}

pub(in crate::anova) fn is_vec_estimable(row_proj: &DenseMatrix, v: &[f64]) -> Result<bool, StatsError> {
    let norm_sq = dot_slice(v, v);
    if norm_sq <= 0.0 {
        return Ok(true);
    }
    let pv = row_proj.matvec(v)?;
    let err_sq: f64 = v.iter().zip(pv.iter()).map(|(&a, &b)| (a - b) * (a - b)).sum();
    Ok(err_sq <= 1e-12 * norm_sq.max(1.0))
}

pub(in crate::anova) fn satterthwaite_two_term_df(t1_ms: f64, df1: f64, t2_ms: f64, df2: f64) -> f64 {
    let num = t1_ms + t2_ms;
    let den = (t1_ms * t1_ms) / df1.max(1.0) + (t2_ms * t2_ms) / df2.max(1.0);
    if den > 0.0 { (num * num / den).max(1.0) } else { df1.max(df2).max(1.0) }
}

pub(in crate::anova) fn welch_stats_from_weighted_terms(terms: &[(f64, f64)]) -> (f64, f64) {
    let v_sum: f64 = terms.iter().map(|&(v, _)| v.max(0.0)).sum();
    if v_sum <= 0.0 {
        let df_fallback = terms.iter().map(|&(_, df)| df).fold(0.0_f64, f64::max).max(1.0);
        return (0.0, df_fallback);
    }
    let denom: f64 = terms.iter().filter(|&&(v, _)| v > 0.0).map(|&(v, df)| (v * v) / df.max(1.0)).sum();
    let df = if denom > 0.0 { (v_sum * v_sum / denom).max(1.0) } else { 1.0 };
    (v_sum.sqrt(), df)
}

pub(in crate::anova) fn welch_two_slice_stats(a: &[f64], b: &[f64]) -> Result<(f64, f64, f64), StatsError> {
    let ma = SummaryMoments::from_slice(a)?;
    let mb = SummaryMoments::from_slice(b)?;
    let va = if a.len() >= 2 { (ma.css / ((a.len() - 1) as f64)).max(0.0) / (a.len() as f64) } else { 0.0 };
    let vb = if b.len() >= 2 { (mb.css / ((b.len() - 1) as f64)).max(0.0) / (b.len() as f64) } else { 0.0 };
    let dfa = (a.len().saturating_sub(1)).max(1) as f64;
    let dfb = (b.len().saturating_sub(1)).max(1) as f64;
    let (se, df) = welch_stats_from_weighted_terms(&[(va, dfa), (vb, dfb)]);
    Ok((ma.mean - mb.mean, se, df))
}

pub(in crate::anova) fn paired_diff_stats(diffs: &[f64]) -> Result<(f64, f64, f64), StatsError> {
    let m = SummaryMoments::from_slice(diffs)?;
    let n = diffs.len();
    let df = (n.saturating_sub(1)).max(1) as f64;
    let se = if n >= 2 { ((m.css / df).max(0.0) / (n as f64)).sqrt() } else { 0.0 };
    Ok((m.mean, se, df))
}

pub(in crate::anova) fn enumerate_cartesian(radices: &[usize]) -> Result<Vec<Vec<usize>>, StatsError> {
    let mut total = 1usize;
    for &r in radices {
        total = total.saturating_mul(r.max(1));
        if total > 8192 {
            return Err(StatsError::InvalidParameter { name: "factorial_levels", value: total as f64 });
        }
    }
    let mut out = vec![Vec::with_capacity(radices.len())];
    for &r in radices {
        let mut next = Vec::with_capacity(out.len() * r.max(1));
        for prefix in &out {
            for lvl in 0..r.max(1) {
                let mut row = prefix.clone();
                row.push(lvl);
                next.push(row);
            }
        }
        out = next;
    }
    Ok(out)
}

pub(in crate::anova) fn factorial_design_vector(
    total_cols: usize,
    subsets: &[Vec<usize>],
    factor_basis: &[Vec<Vec<f64>>],
    cell: &[usize],
) -> Vec<f64> {
    let mut row = Vec::with_capacity(total_cols);
    row.push(1.0);
    for sub in subsets {
        let mut prod = vec![1.0_f64];
        for &f_idx in sub {
            let b_row = &factor_basis[f_idx][cell[f_idx]];
            if b_row.is_empty() {
                prod.clear();
                break;
            }
            let mut next = Vec::with_capacity(prod.len() * b_row.len());
            for &pv in &prod {
                for &bv in b_row {
                    next.push(pv * bv);
                }
            }
            prod = next;
        }
        row.extend(prod);
    }
    row
}

pub(in crate::anova) fn avg_factorial_design_over(
    total_cols: usize,
    subsets: &[Vec<usize>],
    factor_basis: &[Vec<Vec<f64>>],
    n_factors: usize,
    other_factors: &[usize],
    tuples: &[Vec<usize>],
    fixed: &[(usize, usize)],
) -> Vec<f64> {
    let mut sum_vec = vec![0.0_f64; total_cols];
    let mut cell = vec![0_usize; n_factors];
    for &(f, lvl) in fixed {
        cell[f] = lvl;
    }
    for tup in tuples {
        for (&f, &v) in other_factors.iter().zip(tup.iter()) {
            cell[f] = v;
        }
        let dv = factorial_design_vector(total_cols, subsets, factor_basis, &cell);
        for (dst, &val) in sum_vec.iter_mut().zip(dv.iter()) {
            *dst += val;
        }
    }
    let inv = 1.0 / (tuples.len().max(1) as f64);
    for dst in &mut sum_vec {
        *dst *= inv;
    }
    sum_vec
}

pub(in crate::anova) fn factorial_pair_cells_exist(
    observed: &BTreeSet<Vec<usize>>,
    n_factors: usize,
    other_factors: &[usize],
    tup: &[usize],
    fixed_a: &[(usize, usize)],
    fixed_b: &[(usize, usize)],
) -> bool {
    let mut cell = vec![0_usize; n_factors];
    for (&f, &v) in other_factors.iter().zip(tup.iter()) {
        cell[f] = v;
    }
    for &(f, lvl) in fixed_a {
        cell[f] = lvl;
    }
    if !observed.contains(&cell) {
        return false;
    }
    for &(f, lvl) in fixed_b {
        cell[f] = lvl;
    }
    observed.contains(&cell)
}

impl OlsPostHocModel {
    pub(in crate::anova) fn avg_design_over(&self, other_factors: &[usize], tuples: &[Vec<usize>], fixed: &[(usize, usize)]) -> Vec<f64> {
        avg_factorial_design_over(self.total_cols, &self.subsets, &self.factor_basis, self.factor_levels.len(), other_factors, tuples, fixed)
    }

    pub(in crate::anova) fn pair_cells_exist(
        &self,
        other_factors: &[usize],
        tup: &[usize],
        fixed_a: &[(usize, usize)],
        fixed_b: &[(usize, usize)],
    ) -> bool {
        let mut cell = vec![0_usize; self.factor_levels.len()];
        for (&f, &v) in other_factors.iter().zip(tup.iter()) {
            cell[f] = v;
        }
        for &(f, lvl) in fixed_a {
            cell[f] = lvl;
        }
        if !self.cell_stats.contains_key(&cell) {
            return false;
        }
        for &(f, lvl) in fixed_b {
            cell[f] = lvl;
        }
        self.cell_stats.contains_key(&cell)
    }

    pub(in crate::anova) fn eval_contrast_from_tuples(
        &self,
        other_factors: &[usize],
        tuples: &[Vec<usize>],
        fixed_a: &[(usize, usize)],
        fixed_b: &[(usize, usize)],
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let shared: Vec<Vec<usize>> = tuples.iter().filter(|t| self.pair_cells_exist(other_factors, t, fixed_a, fixed_b)).cloned().collect();
        let all_a = self.avg_design_over(other_factors, tuples, fixed_a);
        let all_b = self.avg_design_over(other_factors, tuples, fixed_b);
        let all_l: Vec<f64> = all_a.iter().zip(all_b.iter()).map(|(&a, &b)| a - b).collect();

        let (chosen, w_a, w_b, l) = if is_vec_estimable(&self.row_proj, &all_l)? {
            let use_tups = if shared.len() == tuples.len() {
                tuples
            } else if !shared.is_empty() && unequal_var {
                shared.as_slice()
            } else {
                tuples
            };
            let wa = self.avg_design_over(other_factors, use_tups, fixed_a);
            let wb = self.avg_design_over(other_factors, use_tups, fixed_b);
            let lv: Vec<f64> = wa.iter().zip(wb.iter()).map(|(&a, &b)| a - b).collect();
            (use_tups, wa, wb, lv)
        } else if !shared.is_empty() {
            let wa = self.avg_design_over(other_factors, &shared, fixed_a);
            let wb = self.avg_design_over(other_factors, &shared, fixed_b);
            let lv: Vec<f64> = wa.iter().zip(wb.iter()).map(|(&a, &b)| a - b).collect();
            if !is_vec_estimable(&self.row_proj, &lv)? {
                return Ok(None);
            }
            (shared.as_slice(), wa, wb, lv)
        } else {
            return Ok(None);
        };

        let diff = dot_slice(&l, &self.beta);
        if unequal_var && chosen.iter().all(|t| self.pair_cells_exist(other_factors, t, fixed_a, fixed_b)) {
            let w = 1.0 / (chosen.len() as f64);
            let w2 = w * w;
            let mut terms = Vec::with_capacity(chosen.len() * 2);
            let mut cell = vec![0_usize; self.factor_levels.len()];
            for tup in chosen {
                for (&f, &v) in other_factors.iter().zip(tup.iter()) {
                    cell[f] = v;
                }
                for fixed in [fixed_a, fixed_b] {
                    for &(f, lvl) in fixed {
                        cell[f] = lvl;
                    }
                    if let Some(st) = self.cell_stats.get(&cell) {
                        let (v_c, df_c) =
                            if st.count >= 2 { (st.var_unbiased / (st.count as f64), (st.count - 1) as f64) } else { (self.ms_resid, self.df_resid) };
                        terms.push((w2 * v_c, df_c));
                    }
                }
            }
            let (se, df) = welch_stats_from_weighted_terms(&terms);
            return Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        let cov_l_raw = self.cov_unscaled.matvec(&l)?;
        let cov_l: Vec<f64> = cov_l_raw.into_iter().map(|v| v * self.ms_resid).collect();
        let se = dot_slice(&l, &cov_l).max(0.0).sqrt();
        let ctrl_var_pair = if is_vec_estimable(&self.row_proj, &w_a)? && is_vec_estimable(&self.row_proj, &w_b)? {
            Some(((self.ms_resid * quad_form(&w_a, &self.cov_unscaled)).max(0.0), (self.ms_resid * quad_form(&w_b, &self.cov_unscaled)).max(0.0)))
        } else {
            None
        };
        Ok(Some(ContrastEstimate { diff, se, df: self.df_resid, ctrl_var_pair, cov_vectors: Some((l, cov_l)) }))
    }

    pub(in crate::anova) fn eval_marginal_pair(
        &self,
        f_idx: usize,
        lvl_a: usize,
        lvl_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        self.eval_contrast_from_tuples(&other_factors, &tuples, &[(f_idx, lvl_a)], &[(f_idx, lvl_b)], unequal_var)
    }

    pub(in crate::anova) fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != cond_f && f != tgt_f).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        let has_shared =
            tuples.iter().any(|t| self.pair_cells_exist(&other_factors, t, &[(cond_f, cond_lvl), (tgt_f, t_a)], &[(cond_f, cond_lvl), (tgt_f, t_b)]));
        if !has_shared {
            return Ok(None);
        }
        self.eval_contrast_from_tuples(&other_factors, &tuples, &[(cond_f, cond_lvl), (tgt_f, t_a)], &[(cond_f, cond_lvl), (tgt_f, t_b)], unequal_var)
    }

    pub(in crate::anova) fn level_mean_estimate(&self, f_idx: usize, lvl: usize) -> Result<Option<f64>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f].len()).collect();
        let tuples = enumerate_cartesian(&radices)?;
        let w = self.avg_design_over(&other_factors, &tuples, &[(f_idx, lvl)]);
        if is_vec_estimable(&self.row_proj, &w)? { Ok(Some(dot_slice(&w, &self.beta))) } else { Ok(None) }
    }
}

impl RmBalancedPostHocModel {
    pub(in crate::anova) fn subject_marginal_values(&self, factor_idx: usize, lvl: usize) -> Vec<f64> {
        let s = self.s_count;
        if factor_idx == 0 {
            let inv_b = 1.0 / (self.b_count as f64);
            (0..s).map(|si| self.subject_data[si][lvl].iter().sum::<f64>() * inv_b).collect()
        } else {
            let inv_a = 1.0 / (self.a_count as f64);
            (0..s).map(|si| (0..self.a_count).map(|ai| self.subject_data[si][ai][lvl]).sum::<f64>() * inv_a).collect()
        }
    }

    pub(in crate::anova) fn eval_marginal_pair(
        &self,
        f_idx: usize,
        lvl_a: usize,
        lvl_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let sf = self.s_count as f64;
        let diff = if f_idx == 0 { self.row_means[lvl_a] - self.row_means[lvl_b] } else { self.col_means[lvl_a] - self.col_means[lvl_b] };
        if !unequal_var {
            let (ms, df, n_eff) = if f_idx == 0 {
                (self.err_a_ms, self.err_a_df, sf * (self.b_count as f64))
            } else {
                (self.err_b_ms, self.err_b_df, sf * (self.a_count as f64))
            };
            let v_lvl = (ms / n_eff).max(0.0);
            return Ok(Some(ContrastEstimate { diff, se: (2.0 * v_lvl).sqrt(), df, ctrl_var_pair: Some((v_lvl, v_lvl)), cov_vectors: None }));
        }

        let is_between =
            matches!((self.design, f_idx), (RepeatedMeasuresDesign::ColumnFactorMatched, 0) | (RepeatedMeasuresDesign::RowFactorMatched, 1));
        if is_between {
            let va = self.subject_marginal_values(f_idx, lvl_a);
            let vb = self.subject_marginal_values(f_idx, lvl_b);
            let (d, se, df) = welch_two_slice_stats(&va, &vb)?;
            return Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        if self.design == RepeatedMeasuresDesign::BothFactorsMatched {
            let va = self.subject_marginal_values(f_idx, lvl_a);
            let vb = self.subject_marginal_values(f_idx, lvl_b);
            let diffs: Vec<f64> = va.into_iter().zip(vb).map(|(x, y)| x - y).collect();
            let (d, se, df) = paired_diff_stats(&diffs)?;
            return Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }));
        }

        // Mixed design within-subject factor: paired diffs stratified by the between-subject factor.
        let n_between = if f_idx == 1 { self.a_count } else { self.b_count };
        let inv_g = 1.0 / (n_between as f64);
        let w2 = inv_g * inv_g;
        let mut terms = Vec::with_capacity(n_between);
        for g in 0..n_between {
            let diffs: Vec<f64> = (0..self.s_count)
                .map(|si| {
                    if f_idx == 1 {
                        self.subject_data[si][g][lvl_a] - self.subject_data[si][g][lvl_b]
                    } else {
                        self.subject_data[si][lvl_a][g] - self.subject_data[si][lvl_b][g]
                    }
                })
                .collect();
            let (_, g_se, g_df) = paired_diff_stats(&diffs)?;
            terms.push((w2 * g_se * g_se, g_df));
        }
        let (se, df) = welch_stats_from_weighted_terms(&terms);
        Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair: None, cov_vectors: None }))
    }

    pub(in crate::anova) fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let _ = cond_f;
        let cell = |tgt: usize| -> (usize, usize) { if tgt_f == 0 { (tgt, cond_lvl) } else { (cond_lvl, tgt) } };
        let (ra, ca) = cell(t_a);
        let (rb, cb) = cell(t_b);
        let diff = self.cell_means[ra][ca] - self.cell_means[rb][cb];
        let sf = self.s_count as f64;

        if !unequal_var {
            let is_pure_within =
                matches!((self.design, tgt_f), (RepeatedMeasuresDesign::ColumnFactorMatched, 1) | (RepeatedMeasuresDesign::RowFactorMatched, 0));
            let (ms_pool, df_pool) = if is_pure_within {
                (self.err_ab_ms, self.err_ab_df)
            } else {
                let (err_t_ms, err_t_df, n_other) = if tgt_f == 0 {
                    (self.err_a_ms, self.err_a_df, self.b_count as f64)
                } else {
                    (self.err_b_ms, self.err_b_df, self.a_count as f64)
                };
                let ms_num = err_t_ms + (n_other - 1.0) * self.err_ab_ms;
                let df = satterthwaite_two_term_df(err_t_ms, err_t_df, (n_other - 1.0) * self.err_ab_ms, self.err_ab_df);
                (ms_num / n_other, df)
            };
            let v_lvl = (ms_pool / sf).max(0.0);
            return Ok(Some(ContrastEstimate {
                diff,
                se: (2.0 * v_lvl).sqrt(),
                df: df_pool,
                ctrl_var_pair: Some((v_lvl, v_lvl)),
                cov_vectors: None,
            }));
        }

        let is_between =
            matches!((self.design, tgt_f), (RepeatedMeasuresDesign::ColumnFactorMatched, 0) | (RepeatedMeasuresDesign::RowFactorMatched, 1));
        let va: Vec<f64> = (0..self.s_count).map(|si| self.subject_data[si][ra][ca]).collect();
        let vb: Vec<f64> = (0..self.s_count).map(|si| self.subject_data[si][rb][cb]).collect();
        if is_between {
            let (d, se, df) = welch_two_slice_stats(&va, &vb)?;
            Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }))
        } else {
            let diffs: Vec<f64> = va.into_iter().zip(vb).map(|(x, y)| x - y).collect();
            let (d, se, df) = paired_diff_stats(&diffs)?;
            Ok(Some(ContrastEstimate { diff: d, se, df, ctrl_var_pair: None, cov_vectors: None }))
        }
    }
}

impl RemlContrastModel {
    pub(in crate::anova) fn avg_design_over(&self, other_factors: &[usize], tuples: &[Vec<usize>], fixed: &[(usize, usize)]) -> Vec<f64> {
        avg_factorial_design_over(self.total_cols, &self.subsets, &self.factor_basis, self.factor_levels.len(), other_factors, tuples, fixed)
    }

    pub(in crate::anova) fn map_to_eff(&self, v_orig: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0_f64; self.beta.len()];
        for (&src, opt_dst) in v_orig.iter().zip(self.col_map.iter()) {
            if let Some(dst) = *opt_dst {
                out[dst] = src;
            }
        }
        out
    }

    pub(in crate::anova) fn eval_weight_vecs(&self, wa_orig: &[f64], wb_orig: &[f64]) -> Result<Option<ContrastEstimate>, StatsError> {
        let l_orig: Vec<f64> = wa_orig.iter().zip(wb_orig.iter()).map(|(&a, &b)| a - b).collect();
        if !is_vec_estimable(&self.row_proj, &l_orig)? {
            return Ok(None);
        }
        let l = self.map_to_eff(&l_orig);
        let diff = dot_slice(&l, &self.beta) * self.y_scale;
        let v = quad_form(&l, &self.phi).max(0.0);
        let cov_mat = match self.df_method {
            MixedModelDfMethod::KenwardRoger => &self.phi_a,
            MixedModelDfMethod::Satterthwaite => &self.phi,
        };
        let v_cov = quad_form(&l, cov_mat).max(0.0);
        let se = v_cov.sqrt() * self.y_scale;
        let grad: Vec<f64> = self.phi_m_phi.iter().map(|pmp| quad_form(&l, pmp)).collect();
        let wg = self.w_info_inv.matvec(&grad)?;
        let denom = dot_slice(&grad, &wg);
        let df = if v > 0.0 && denom > 1e-30 * v * v { (2.0 * v * v / denom).clamp(1.0, self.n_minus_p) } else { self.n_minus_p };

        let y2 = self.y_scale * self.y_scale;
        let cov_l_raw = cov_mat.matvec(&l)?;
        let cov_l: Vec<f64> = cov_l_raw.into_iter().map(|val| val * y2).collect();
        let ctrl_var_pair = if is_vec_estimable(&self.row_proj, wa_orig)? && is_vec_estimable(&self.row_proj, wb_orig)? {
            let wa = self.map_to_eff(wa_orig);
            let wb = self.map_to_eff(wb_orig);
            Some(((quad_form(&wa, cov_mat) * y2).max(0.0), (quad_form(&wb, cov_mat) * y2).max(0.0)))
        } else {
            None
        };
        Ok(Some(ContrastEstimate { diff, se, df, ctrl_var_pair, cov_vectors: Some((l, cov_l)) }))
    }

    pub(in crate::anova) fn eval_contrast_over(
        &self,
        other_factors: &[usize],
        tuples: &[Vec<usize>],
        fixed_a: &[(usize, usize)],
        fixed_b: &[(usize, usize)],
        require_shared: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let n_factors = self.factor_levels.len();
        let shared: Vec<Vec<usize>> = tuples
            .iter()
            .filter(|t| factorial_pair_cells_exist(&self.observed_cells, n_factors, other_factors, t, fixed_a, fixed_b))
            .cloned()
            .collect();
        if require_shared && shared.is_empty() {
            return Ok(None);
        }
        let all_a = self.avg_design_over(other_factors, tuples, fixed_a);
        let all_b = self.avg_design_over(other_factors, tuples, fixed_b);
        if let Some(est) = self.eval_weight_vecs(&all_a, &all_b)? {
            return Ok(Some(est));
        }
        if shared.is_empty() {
            return Ok(None);
        }
        let wa = self.avg_design_over(other_factors, &shared, fixed_a);
        let wb = self.avg_design_over(other_factors, &shared, fixed_b);
        self.eval_weight_vecs(&wa, &wb)
    }

    pub(in crate::anova) fn eval_marginal_pair(&self, f_idx: usize, lvl_a: usize, lvl_b: usize) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f]).collect();
        let tuples = enumerate_cartesian(&radices)?;
        self.eval_contrast_over(&other_factors, &tuples, &[(f_idx, lvl_a)], &[(f_idx, lvl_b)], false)
    }

    pub(in crate::anova) fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != cond_f && f != tgt_f).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f]).collect();
        let tuples = enumerate_cartesian(&radices)?;
        self.eval_contrast_over(&other_factors, &tuples, &[(cond_f, cond_lvl), (tgt_f, t_a)], &[(cond_f, cond_lvl), (tgt_f, t_b)], true)
    }

    pub(in crate::anova) fn level_mean_estimate(&self, f_idx: usize, lvl: usize) -> Result<Option<f64>, StatsError> {
        let other_factors: Vec<usize> = (0..self.factor_levels.len()).filter(|&f| f != f_idx).collect();
        let radices: Vec<usize> = other_factors.iter().map(|&f| self.factor_levels[f]).collect();
        let tuples = enumerate_cartesian(&radices)?;
        let w_orig = self.avg_design_over(&other_factors, &tuples, &[(f_idx, lvl)]);
        if is_vec_estimable(&self.row_proj, &w_orig)? {
            let w = self.map_to_eff(&w_orig);
            Ok(Some(dot_slice(&w, &self.beta) * self.y_scale))
        } else {
            Ok(None)
        }
    }
}

impl FactorialPostHocContext {
    pub(in crate::anova) fn eval_marginal_pair(
        &self,
        f_idx: usize,
        lvl_a: usize,
        lvl_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        match self {
            Self::Ols(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b, unequal_var),
            Self::RmBalanced(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b, unequal_var),
            Self::Reml(m) => m.eval_marginal_pair(f_idx, lvl_a, lvl_b),
        }
    }

    pub(in crate::anova) fn eval_simple_pair(
        &self,
        cond_f: usize,
        cond_lvl: usize,
        tgt_f: usize,
        t_a: usize,
        t_b: usize,
        unequal_var: bool,
    ) -> Result<Option<ContrastEstimate>, StatsError> {
        match self {
            Self::Ols(m) => m.eval_simple_pair(cond_f, cond_lvl, tgt_f, t_a, t_b, unequal_var),
            Self::RmBalanced(m) => m.eval_simple_pair(cond_f, cond_lvl, tgt_f, t_a, t_b, unequal_var),
            Self::Reml(m) => m.eval_simple_pair(cond_f, cond_lvl, tgt_f, t_a, t_b),
        }
    }

    pub(in crate::anova) fn level_mean_estimate(&self, f_idx: usize, lvl: usize) -> Result<Option<f64>, StatsError> {
        match self {
            Self::Ols(m) => m.level_mean_estimate(f_idx, lvl),
            Self::RmBalanced(m) => Ok(Some(if f_idx == 0 { m.row_means[lvl] } else { m.col_means[lvl] })),
            Self::Reml(m) => m.level_mean_estimate(f_idx, lvl),
        }
    }
}

pub(in crate::anova) fn push_pairwise_outputs_with_footnote(
    out: &mut ResultSheet,
    details: &[PairwiseComparisonDetail],
    tab_id: &str,
    title: &str,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
    extra_footnote: Option<String>,
) {
    if details.is_empty() {
        return;
    }
    let mut tab = build_multiplicity_tab(details, title, p_fmt, confidence);
    tab.id = tab_id.to_owned();
    if let Some(note) = extra_footnote {
        tab.footnotes.push(note);
    }
    out.tabs.push(tab);
    for d in details {
        out.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: d.group_a,
            group_b: d.group_b,
            label_a: d.label_a.clone(),
            label_b: d.label_b.clone(),
            estimate_diff: Some(d.mean_diff),
            raw_p_or_q: d.adjusted_p_or_q,
            is_fdr_q: d.is_fdr_q,
            significant: d.significant,
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::anova) fn build_pair_detail(
    group_a: usize,
    group_b: usize,
    label_a: String,
    label_b: String,
    diff: f64,
    se_diff: f64,
    df: f64,
    statistic: f64,
    raw_p: f64,
    adjusted_p_or_q: f64,
    is_fdr_q: bool,
    ci: Option<(f64, f64)>,
    threshold: f64,
) -> PairwiseComparisonDetail {
    PairwiseComparisonDetail {
        group_a,
        group_b,
        label_a,
        label_b,
        mean_diff: diff,
        se_diff,
        statistic,
        df,
        raw_p,
        adjusted_p_or_q,
        is_fdr_q,
        ci,
        significant: threshold.is_finite() && adjusted_p_or_q.is_finite() && adjusted_p_or_q <= threshold,
    }
}

pub(in crate::anova) fn zero_se_pair(diff: f64, unsigned: bool) -> (f64, f64) {
    if diff != 0.0 { (if unsigned { f64::INFINITY } else { diff.signum() * f64::INFINITY }, 0.0) } else { (0.0, 1.0) }
}

pub(in crate::anova) fn compare_independent_newman_keuls(
    entries: &[NamedSliceContrast<'_>],
    stats: &[f64],
    raw_p: &[f64],
    threshold: f64,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let m = entries.len();
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by(|&a, &b| raw_p[a].total_cmp(&raw_p[b]).then_with(|| stats[b].abs().total_cmp(&stats[a].abs())));

    let mut adj_vec = vec![1.0_f64; m];
    let mut running_max = 0.0_f64;
    for (rank, &orig_idx) in order.iter().enumerate() {
        let e = &entries[orig_idx];
        let m_rem = m - rank;
        let cand = if e.se > 0.0 {
            let t_abs = (e.diff / e.se).abs();
            max_studentized_range_sf(std::f64::consts::SQRT_2 * t_abs, m_rem, 2, e.df)?
        } else {
            raw_p[orig_idx]
        };
        running_max = running_max.max(cand).max(raw_p[orig_idx]).clamp(0.0, 1.0);
        adj_vec[orig_idx] = running_max;
    }

    let mut out = Vec::with_capacity(m);
    for (idx, e) in entries.iter().enumerate() {
        out.push(build_pair_detail(
            e.id_a,
            e.id_b,
            e.label_a.to_owned(),
            e.label_b.to_owned(),
            e.diff,
            e.se,
            e.df,
            stats[idx],
            raw_p[idx],
            adj_vec[idx],
            false,
            None,
            threshold,
        ));
    }
    Ok(out)
}

pub(in crate::anova) fn compare_independent_contrast_family(
    entries: &[NamedSliceContrast<'_>],
    proc_req: MultiplicityProcedure,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let m = entries.len();
    if m == 0 {
        return Ok(Vec::new());
    }
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if report_ci && (!ci_alpha.is_finite() || ci_alpha <= 0.0 || ci_alpha >= 1.0) {
        return Err(StatsError::InvalidParameter { name: "ci_alpha", value: ci_alpha });
    }
    let uses_q_stat =
        matches!(proc_req, MultiplicityProcedure::TukeyKramer | MultiplicityProcedure::NewmanKeuls | MultiplicityProcedure::GamesHowell);
    let mut raw_p = Vec::with_capacity(m);
    let mut stats = Vec::with_capacity(m);
    for e in entries {
        if !e.diff.is_finite() || !e.se.is_finite() || e.se < 0.0 || !e.df.is_finite() || e.df <= 0.0 {
            return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "contrast_estimate", value: f64::NAN }));
        }
        if e.se > 0.0 {
            let t = e.diff / e.se;
            let rp = student_t_two_sided_p(t, e.df)?;
            if !rp.is_finite() {
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "contrast_p_value", value: rp }));
            }
            raw_p.push(rp);
            stats.push(if uses_q_stat { t.abs() * std::f64::consts::SQRT_2 } else { t });
        } else {
            let (st, rp) = zero_se_pair(e.diff, uses_q_stat);
            raw_p.push(rp);
            stats.push(st);
        }
    }

    if m == 1 || matches!(proc_req, MultiplicityProcedure::None | MultiplicityProcedure::FisherLsd) {
        let mut out = Vec::with_capacity(m);
        for (idx, e) in entries.iter().enumerate() {
            let ci = if report_ci && !matches!(proc_req, MultiplicityProcedure::NewmanKeuls) {
                let tc = student_t_two_sided_isf(ci_alpha.clamp(1e-12, 0.9999), e.df)?;
                Some((e.diff - tc * e.se, e.diff + tc * e.se))
            } else {
                None
            };
            out.push(build_pair_detail(
                e.id_a,
                e.id_b,
                e.label_a.to_owned(),
                e.label_b.to_owned(),
                e.diff,
                e.se,
                e.df,
                stats[idx],
                raw_p[idx],
                raw_p[idx],
                false,
                ci,
                threshold,
            ));
        }
        return Ok(out);
    }

    match proc_req {
        MultiplicityProcedure::NewmanKeuls => compare_independent_newman_keuls(entries, &stats, &raw_p, threshold),
        MultiplicityProcedure::TukeyKramer
        | MultiplicityProcedure::GamesHowell
        | MultiplicityProcedure::DunnettT3
        | MultiplicityProcedure::Dunnett { .. } => {
            let mut out = Vec::with_capacity(m);
            for (idx, e) in entries.iter().enumerate() {
                let adj_p = if e.se > 0.0 {
                    let t_abs = (e.diff / e.se).abs();
                    max_studentized_range_sf(std::f64::consts::SQRT_2 * t_abs, m, 2, e.df)?
                } else {
                    raw_p[idx]
                };
                let ci = if report_ci {
                    let crit = max_studentized_range_isf(ci_alpha, m, 2, e.df)? * std::f64::consts::FRAC_1_SQRT_2;
                    Some((e.diff - crit * e.se, e.diff + crit * e.se))
                } else {
                    None
                };
                out.push(build_pair_detail(
                    e.id_a,
                    e.id_b,
                    e.label_a.to_owned(),
                    e.label_b.to_owned(),
                    e.diff,
                    e.se,
                    e.df,
                    stats[idx],
                    raw_p[idx],
                    adj_p,
                    false,
                    ci,
                    threshold,
                ));
            }
            Ok(out)
        }
        _ => {
            let eff_proc = if matches!(proc_req, MultiplicityProcedure::TamhaneT2) { MultiplicityProcedure::Sidak } else { proc_req };
            let (adj_p, is_fdr) = adjust_p_values(&raw_p, eff_proc, threshold);
            let per_alpha = match eff_proc {
                MultiplicityProcedure::Bonferroni => Some(ci_alpha / (m as f64)),
                MultiplicityProcedure::Sidak => Some(1.0 - (1.0 - ci_alpha).powf(1.0 / (m as f64))),
                _ => None,
            };
            let mut out = Vec::with_capacity(m);
            for (idx, e) in entries.iter().enumerate() {
                let ci = match (report_ci, per_alpha) {
                    (true, Some(a)) => {
                        let tc = student_t_two_sided_isf(a.clamp(1e-12, 0.9999), e.df)?;
                        Some((e.diff - tc * e.se, e.diff + tc * e.se))
                    }
                    _ => None,
                };
                out.push(build_pair_detail(
                    e.id_a,
                    e.id_b,
                    e.label_a.to_owned(),
                    e.label_b.to_owned(),
                    e.diff,
                    e.se,
                    e.df,
                    stats[idx],
                    raw_p[idx],
                    adj_p[idx],
                    is_fdr,
                    ci,
                    threshold,
                ));
            }
            Ok(out)
        }
    }
}

pub(in crate::anova) struct ContrastFamilyEval<'a> {
    pub(in crate::anova) levels: &'a [(usize, String)],
    pub(in crate::anova) level_means: &'a [Option<f64>],
    pub(in crate::anova) mult_spec: &'a MultiplicitySpec,
    pub(in crate::anova) confidence: ConfidenceLevel,
    pub(in crate::anova) n_slices: usize,
    pub(in crate::anova) pairs: Vec<(usize, usize, ContrastEstimate)>,
}

pub(in crate::anova) type ContrastFamilyOutcome = (Vec<PairwiseComparisonDetail>, Option<String>);

pub(in crate::anova) fn compare_contrast_family(eval: ContrastFamilyEval<'_>) -> Result<ContrastFamilyOutcome, StatsError> {
    let k = eval.levels.len();
    let proc_req = MultiplicityProcedure::from(eval.mult_spec.method);
    let threshold = eval.mult_spec.threshold;
    let ci_alpha = eval.confidence.alpha();
    let report_ci = eval.mult_spec.report_confidence_intervals;
    let n_slices = eval.n_slices.max(1);
    if !threshold.is_finite() || threshold <= 0.0 || threshold >= 1.0 {
        return Err(StatsError::InvalidParameter { name: "threshold", value: threshold });
    }
    if eval.pairs.is_empty() {
        return Ok((Vec::new(), None));
    }

    let mut used = vec![false; k];
    for &(i, j, _) in &eval.pairs {
        used[i] = true;
        used[j] = true;
    }
    let k_eff = used.iter().filter(|&&u| u).count().max(2);

    match proc_req {
        MultiplicityProcedure::Dunnett { .. } => compare_contrast_dunnett(&eval, threshold, ci_alpha, report_ci),
        MultiplicityProcedure::TukeyKramer | MultiplicityProcedure::GamesHowell => {
            let mut out = Vec::with_capacity(eval.pairs.len());
            for &(i, j, ref est) in &eval.pairs {
                let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
                let (q_stat, raw_p, adj_p) = if se_q > 0.0 {
                    let q = est.diff.abs() / se_q;
                    let rp = student_t_two_sided_p(est.diff / est.se, est.df)?;
                    (q, rp, max_studentized_range_sf(q, n_slices, k_eff, est.df)?)
                } else {
                    let (st, rp) = zero_se_pair(est.diff, true);
                    (st, rp, rp)
                };
                let ci = if report_ci {
                    let qc = max_studentized_range_isf(ci_alpha, n_slices, k_eff, est.df)?;
                    Some((est.diff - qc * se_q, est.diff + qc * se_q))
                } else {
                    None
                };
                out.push(build_pair_detail(
                    eval.levels[i].0,
                    eval.levels[j].0,
                    eval.levels[i].1.clone(),
                    eval.levels[j].1.clone(),
                    est.diff,
                    est.se,
                    est.df,
                    q_stat,
                    raw_p,
                    adj_p,
                    false,
                    ci,
                    threshold,
                ));
            }
            Ok((out, None))
        }
        MultiplicityProcedure::NewmanKeuls => Ok((compare_contrast_newman_keuls(&eval, threshold)?, None)),
        _ => {
            let entries: Vec<NamedSliceContrast<'_>> = eval
                .pairs
                .iter()
                .map(|&(i, j, ref est)| NamedSliceContrast {
                    id_a: eval.levels[i].0,
                    id_b: eval.levels[j].0,
                    label_a: eval.levels[i].1.as_str(),
                    label_b: eval.levels[j].1.as_str(),
                    diff: est.diff,
                    se: est.se,
                    df: est.df,
                })
                .collect();
            let details = compare_independent_contrast_family(&entries, proc_req, threshold, ci_alpha, report_ci)?;
            Ok((details, None))
        }
    }
}

pub(in crate::anova) fn factor_loadings_from_cov_vectors(pairs: &[(usize, usize, ContrastEstimate)]) -> Option<Vec<f64>> {
    let m = pairs.len();
    if m <= 1 {
        return Some(vec![std::f64::consts::FRAC_1_SQRT_2; m]);
    }
    if pairs.iter().all(|(_, _, e)| e.se <= 0.0) {
        return Some(vec![std::f64::consts::FRAC_1_SQRT_2; m]);
    }
    if !pairs.iter().all(|(_, _, e)| e.se > 0.0 && e.cov_vectors.is_some()) {
        return None;
    }
    let mut rhos = vec![vec![0.0_f64; m]; m];
    let mut max_abs_rho = 0.0_f64;
    for a in 0..m {
        let (la, _) = pairs[a].2.cov_vectors.as_ref()?;
        for b in (a + 1)..m {
            let (_, cov_lb) = pairs[b].2.cov_vectors.as_ref()?;
            let denom = pairs[a].2.se * pairs[b].2.se;
            if denom <= 0.0 {
                return None;
            }
            let r = dot_slice(la, cov_lb) / denom;
            if !r.is_finite() {
                return None;
            }
            max_abs_rho = max_abs_rho.max(r.abs());
            rhos[a][b] = r;
            rhos[b][a] = r;
        }
    }
    if max_abs_rho <= 1e-12 {
        return Some(vec![0.0; m]);
    }
    if m == 2 {
        let r01 = rhos[0][1];
        if (0.0..0.999_999).contains(&r01) {
            let lam = r01.sqrt();
            return Some(vec![lam, lam]);
        }
        return None;
    }
    let mut log_row_sums = vec![0.0_f64; m];
    for a in 0..m {
        for (b, &r) in rhos[a].iter().enumerate() {
            if a == b {
                continue;
            }
            if !(1e-12..0.999_999).contains(&r) {
                return None;
            }
            log_row_sums[a] += r.ln();
        }
    }
    let sum_s: f64 = log_row_sums.iter().sum();
    let offset = sum_s / (2.0 * ((m - 1) as f64));
    let denom_m = (m - 2) as f64;
    let loadings: Vec<f64> = log_row_sums.iter().map(|&sa| ((sa - offset) / denom_m).exp().clamp(0.0, 0.999_999)).collect();
    for a in 0..m {
        for b in (a + 1)..m {
            if (rhos[a][b] - loadings[a] * loadings[b]).abs() > 1e-4 {
                return None;
            }
        }
    }
    Some(loadings)
}

pub(in crate::anova) fn resolve_dunnett_loadings(pairs: &[(usize, usize, ContrastEstimate)]) -> Option<Vec<f64>> {
    if pairs.len() > 1 && pairs.iter().all(|(_, _, e)| e.se > 0.0 && e.cov_vectors.is_some()) {
        return factor_loadings_from_cov_vectors(pairs);
    }
    if let Some(loadings) = factor_loadings_from_cov_vectors(pairs) {
        return Some(loadings);
    }
    let first_vc = pairs.first().and_then(|(_, _, e)| e.ctrl_var_pair.map(|(_, vc)| vc))?;
    if pairs.iter().all(|(_, _, e)| matches!(e.ctrl_var_pair, Some((vt, vc)) if vt + vc > 0.0 && (vc - first_vc).abs() <= 1e-9 * first_vc.max(1.0))) {
        return Some(
            pairs
                .iter()
                .map(|(_, _, e)| match e.ctrl_var_pair {
                    Some((v_tr, v_ctrl)) if v_tr + v_ctrl > 0.0 => (v_ctrl / (v_tr + v_ctrl)).sqrt().clamp(0.0, 0.999_999),
                    _ => 0.0,
                })
                .collect(),
        );
    }
    None
}

pub(in crate::anova) fn compare_contrast_dunnett(
    eval: &ContrastFamilyEval<'_>,
    threshold: f64,
    ci_alpha: f64,
    report_ci: bool,
) -> Result<ContrastFamilyOutcome, StatsError> {
    let m = eval.pairs.len();
    let n_slices = eval.n_slices.max(1);
    let Some(loadings) = resolve_dunnett_loadings(&eval.pairs) else {
        let m_tot = (m * n_slices).max(1);
        let mut out = Vec::with_capacity(m);
        for &(i, j, ref est) in &eval.pairs {
            let (t_stat, raw_p, adj_p) = if est.se > 0.0 {
                let t = est.diff / est.se;
                let rp = student_t_two_sided_p(t, est.df)?;
                let ap = max_studentized_range_sf(std::f64::consts::SQRT_2 * t.abs(), m_tot, 2, est.df)?;
                (t, rp, ap)
            } else {
                let (st, rp) = zero_se_pair(est.diff, false);
                (st, rp, rp)
            };
            let ci = if report_ci {
                let crit = max_studentized_range_isf(ci_alpha, m_tot, 2, est.df)? * std::f64::consts::FRAC_1_SQRT_2;
                Some((est.diff - crit * est.se, est.diff + crit * est.se))
            } else {
                None
            };
            out.push(build_pair_detail(
                eval.levels[i].0,
                eval.levels[j].0,
                eval.levels[i].1.clone(),
                eval.levels[j].1.clone(),
                est.diff,
                est.se,
                est.df,
                t_stat,
                raw_p,
                adj_p,
                false,
                ci,
                threshold,
            ));
        }
        let note = String::from(
            "Contrast correlation structure is non-estimable in single-factor Dunnett form; conservative Studentized Maximum Modulus adjustment applied.",
        );
        return Ok((out, Some(note)));
    };

    let df_joint = eval.pairs.iter().map(|(_, _, e)| e.df).fold(f64::INFINITY, f64::min).max(1.0);
    let slice_ci_alpha = if n_slices > 1 { -((-ci_alpha.clamp(1e-12, 0.9999)).ln_1p() / (n_slices as f64)).exp_m1() } else { ci_alpha };
    let d_crit = if report_ci {
        Some(if m == 1 {
            student_t_two_sided_isf(slice_ci_alpha.clamp(1e-12, 0.9999), df_joint)?
        } else {
            dunnett_two_sided_crit_loadings(slice_ci_alpha.clamp(1e-12, 0.9999), df_joint, &loadings)?
        })
    } else {
        None
    };

    let mut out = Vec::with_capacity(m);
    for &(i, j, ref est) in &eval.pairs {
        let (t_stat, raw_p, adj_p) = if est.se > 0.0 {
            let t = est.diff / est.se;
            let rp = student_t_two_sided_p(t, est.df)?;
            let ap_slice = if m == 1 { rp } else { dunnett_two_sided_p_loadings(t.abs(), df_joint, &loadings)? };
            let ap = if n_slices > 1 {
                let pc = ap_slice.clamp(0.0, 1.0);
                if pc >= 1.0 { 1.0 } else { (-((n_slices as f64) * (-pc).ln_1p()).exp_m1()).clamp(pc, 1.0) }
            } else {
                ap_slice
            };
            (t, rp, ap)
        } else {
            let (st, rp) = zero_se_pair(est.diff, false);
            (st, rp, rp)
        };
        let ci = d_crit.map(|dc| (est.diff - dc * est.se, est.diff + dc * est.se));
        out.push(build_pair_detail(
            eval.levels[i].0,
            eval.levels[j].0,
            eval.levels[i].1.clone(),
            eval.levels[j].1.clone(),
            est.diff,
            est.se,
            est.df,
            t_stat,
            raw_p,
            adj_p,
            false,
            ci,
            threshold,
        ));
    }
    Ok((out, None))
}

pub(in crate::anova) fn resolve_level_order_for_nk(k: usize, eval: &ContrastFamilyEval<'_>) -> Vec<usize> {
    let mut means = vec![None; k];
    for (idx, m) in means.iter_mut().enumerate() {
        *m = eval.level_means.get(idx).copied().flatten();
    }
    let mut part: Vec<usize> = Vec::new();
    for i in 0..k {
        if eval.pairs.iter().any(|&(a, b, _)| a == i || b == i) {
            part.push(i);
        }
    }
    if part.iter().any(|&idx| means[idx].is_none()) && !part.is_empty() {
        means[part[0]] = Some(0.0);
        for _ in 0..part.len() {
            for &(a, b, ref est) in &eval.pairs {
                if let (Some(ma), None) = (means[a], means[b]) {
                    means[b] = Some(ma - est.diff);
                } else if let (None, Some(mb)) = (means[a], means[b]) {
                    means[a] = Some(mb + est.diff);
                }
            }
        }
    }
    part.sort_by(|&a, &b| means[a].unwrap_or(0.0).total_cmp(&means[b].unwrap_or(0.0)));
    part
}

pub(in crate::anova) fn compare_contrast_newman_keuls(
    eval: &ContrastFamilyEval<'_>,
    threshold: f64,
) -> Result<Vec<PairwiseComparisonDetail>, StatsError> {
    let k_total = eval.levels.len();
    let n_slices = eval.n_slices.max(1);
    let sorted_idx = resolve_level_order_for_nk(k_total, eval);
    let k = sorted_idx.len();
    if k < 2 {
        return Ok(Vec::new());
    }
    let mut pair_map: std::collections::BTreeMap<(usize, usize), &ContrastEstimate> = std::collections::BTreeMap::new();
    for &(i, j, ref est) in &eval.pairs {
        pair_map.insert((i.min(j), i.max(j)), est);
    }

    let mut p_mat = vec![vec![1.0_f64; k]; k];
    for span in (2..=k).rev() {
        for lo in 0..=(k - span) {
            let hi = lo + span - 1;
            let u = sorted_idx[lo];
            let v = sorted_idx[hi];
            let raw_p = if let Some(&est) = pair_map.get(&(u.min(v), u.max(v))) {
                let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
                if se_q > 0.0 { max_studentized_range_sf(est.diff.abs() / se_q, n_slices, span, est.df)? } else { zero_se_pair(est.diff, true).1 }
            } else {
                0.0
            };
            let mut adj = raw_p;
            if lo > 0 {
                adj = adj.max(p_mat[lo - 1][hi]);
            }
            if hi + 1 < k {
                adj = adj.max(p_mat[lo][hi + 1]);
            }
            p_mat[lo][hi] = adj.clamp(0.0, 1.0);
        }
    }

    let mut pos_of = vec![0_usize; k_total];
    for (pos, &orig) in sorted_idx.iter().enumerate() {
        pos_of[orig] = pos;
    }

    let mut out = Vec::with_capacity(eval.pairs.len());
    for &(i, j, ref est) in &eval.pairs {
        let se_q = est.se * std::f64::consts::FRAC_1_SQRT_2;
        let (q_stat, raw_p) =
            if se_q > 0.0 { (est.diff.abs() / se_q, student_t_two_sided_p(est.diff / est.se, est.df)?) } else { zero_se_pair(est.diff, true) };
        let (p_lo, p_hi) = (pos_of[i].min(pos_of[j]), pos_of[i].max(pos_of[j]));
        let adj_p = p_mat[p_lo][p_hi];
        out.push(build_pair_detail(
            eval.levels[i].0,
            eval.levels[j].0,
            eval.levels[i].1.clone(),
            eval.levels[j].1.clone(),
            est.diff,
            est.se,
            est.df,
            q_stat,
            raw_p,
            adj_p,
            false,
            None,
            threshold,
        ));
    }
    Ok(out)
}

pub(in crate::anova) struct SimpleEffectsSpec<'a> {
    pub(in crate::anova) cond_factor: usize,
    pub(in crate::anova) target_factor: usize,
    pub(in crate::anova) mult_spec: &'a MultiplicitySpec,
    pub(in crate::anova) tab_id: &'a str,
    pub(in crate::anova) title: &'a str,
    pub(in crate::anova) p_fmt: PValueFormat,
    pub(in crate::anova) confidence: ConfidenceLevel,
}

pub(in crate::anova) fn validate_dunnett_ctrl(mult_spec: &MultiplicitySpec, n_levels: usize) -> Result<(), StatsError> {
    if let MultiplicityMethod::Dunnett { control_column } = mult_spec.method
        && control_column >= n_levels
    {
        return Err(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 });
    }
    Ok(())
}

/// Resolve the single factorial factor targeted by `MultiplicityMethod::Dunnett { control_column }`.
///
/// Prefers primary factor `0` when `control_column` is in bounds there, otherwise falls back to the
/// first factor with at least 2 levels where `control_column < levels.len()`, returning
/// `StatsError::InvalidParameter` when `control_column` is out of bounds for every factor.
pub(in crate::anova) fn resolve_factorial_dunnett_target(obs: &FactorialObservations, control_column: usize) -> Result<usize, StatsError> {
    for (f_idx, levels) in obs.factor_levels.iter().enumerate() {
        if levels.len() >= 2 && control_column < levels.len() {
            return Ok(f_idx);
        }
    }
    Err(StatsError::InvalidParameter { name: "control_column", value: control_column as f64 })
}

pub(in crate::anova) fn is_unequal_variance_method(method: MultiplicityMethod) -> bool {
    matches!(method, MultiplicityMethod::GamesHowell | MultiplicityMethod::TamhaneT2 | MultiplicityMethod::DunnettT3)
}

pub(in crate::anova) fn is_contrast_count_multiplicity_method(method: MultiplicityMethod) -> bool {
    matches!(
        method,
        MultiplicityMethod::None
            | MultiplicityMethod::FisherLsd
            | MultiplicityMethod::Bonferroni
            | MultiplicityMethod::Sidak
            | MultiplicityMethod::HolmSidak
            | MultiplicityMethod::TamhaneT2
            | MultiplicityMethod::DunnettT3
            | MultiplicityMethod::BenjaminiHochbergFdr
            | MultiplicityMethod::BenjaminiYekutieliFdr
            | MultiplicityMethod::TwoStageBkyFdr
    )
}

pub(in crate::anova) fn collect_factor_pairs<F>(
    n_levels: usize,
    method: MultiplicityMethod,
    mut eval_pair: F,
) -> Result<Vec<(usize, usize, ContrastEstimate)>, StatsError>
where
    F: FnMut(usize, usize) -> Result<Option<ContrastEstimate>, StatsError>,
{
    let mut pairs = Vec::new();
    if let MultiplicityMethod::Dunnett { control_column } = method {
        for tr in 0..n_levels {
            if tr == control_column {
                continue;
            }
            if let Some(est) = eval_pair(tr, control_column)? {
                pairs.push((tr, control_column, est));
            }
        }
    } else {
        for i in 0..n_levels {
            for j in (i + 1)..n_levels {
                if let Some(est) = eval_pair(i, j)? {
                    pairs.push((i, j, est));
                }
            }
        }
    }
    Ok(pairs)
}

pub(in crate::anova) fn run_simple_effects_for_pair(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    post_hoc: &FactorialPostHocContext,
    cfg: &SimpleEffectsSpec<'_>,
    next_group_id: &mut usize,
) -> Result<(), StatsError> {
    let cond_levels = &obs.factor_levels[cfg.cond_factor];
    let target_levels = &obs.factor_levels[cfg.target_factor];
    let k_t = target_levels.len();
    if k_t < 2 {
        return Ok(());
    }
    validate_dunnett_ctrl(cfg.mult_spec, k_t)?;
    let unequal = is_unequal_variance_method(cfg.mult_spec.method);
    let ci_alpha = cfg.confidence.alpha();

    if k_t == 2 {
        let (idx_a, idx_b) = match cfg.mult_spec.method {
            MultiplicityMethod::Dunnett { control_column: 0 } => (1, 0),
            _ => (0, 1),
        };
        struct TwoLvlItem {
            id_a: usize,
            id_b: usize,
            lbl_a: String,
            lbl_b: String,
            est: ContrastEstimate,
        }
        let mut items = Vec::with_capacity(cond_levels.len());
        for (c_idx, cond_label) in cond_levels.iter().enumerate() {
            if let Some(est) = post_hoc.eval_simple_pair(cfg.cond_factor, c_idx, cfg.target_factor, idx_a, idx_b, unequal)? {
                let id_a = *next_group_id;
                let id_b = *next_group_id + 1;
                *next_group_id += 2;
                items.push(TwoLvlItem {
                    id_a,
                    id_b,
                    lbl_a: format!("{cond_label} | {}", target_levels[idx_a]),
                    lbl_b: format!("{cond_label} | {}", target_levels[idx_b]),
                    est,
                });
            }
        }
        let slice_refs: Vec<NamedSliceContrast<'_>> = items
            .iter()
            .map(|it| NamedSliceContrast {
                id_a: it.id_a,
                id_b: it.id_b,
                label_a: it.lbl_a.as_str(),
                label_b: it.lbl_b.as_str(),
                diff: it.est.diff,
                se: it.est.se,
                df: it.est.df,
            })
            .collect();
        let combined = compare_independent_contrast_family(
            &slice_refs,
            cfg.mult_spec.method.into(),
            cfg.mult_spec.threshold,
            ci_alpha,
            cfg.mult_spec.report_confidence_intervals,
        )?;
        let note = if combined.len() >= 2 && !matches!(cfg.mult_spec.method, MultiplicityMethod::None | MultiplicityMethod::FisherLsd) {
            Some(format!("Multiplicity adjusted across all {} simple-effect comparisons.", combined.len()))
        } else {
            None
        };
        push_pairwise_outputs_with_footnote(out, &combined, cfg.tab_id, cfg.title, cfg.p_fmt, cfg.confidence, note);
        return Ok(());
    }

    let mut slice_groups = Vec::with_capacity(cond_levels.len());
    for (c_idx, cond_label) in cond_levels.iter().enumerate() {
        let base_id = *next_group_id;
        *next_group_id += k_t;
        let slice_levels: Vec<(usize, String)> =
            target_levels.iter().enumerate().map(|(i, l)| (base_id + i, format!("{cond_label} | {l}"))).collect();
        let pairs = collect_factor_pairs(k_t, cfg.mult_spec.method, |i, j| {
            post_hoc.eval_simple_pair(cfg.cond_factor, c_idx, cfg.target_factor, i, j, unequal)
        })?;
        if !pairs.is_empty() {
            slice_groups.push((slice_levels, pairs));
        }
    }
    if slice_groups.is_empty() {
        return Ok(());
    }

    let n_slices = slice_groups.len();
    let mut dunnett_fallback_note: Option<String> = None;
    let combined = if is_contrast_count_multiplicity_method(cfg.mult_spec.method) {
        let mut flat_refs = Vec::new();
        for (slice_levels, pairs) in &slice_groups {
            for &(i, j, ref est) in pairs {
                flat_refs.push(NamedSliceContrast {
                    id_a: slice_levels[i].0,
                    id_b: slice_levels[j].0,
                    label_a: slice_levels[i].1.as_str(),
                    label_b: slice_levels[j].1.as_str(),
                    diff: est.diff,
                    se: est.se,
                    df: est.df,
                });
            }
        }
        compare_independent_contrast_family(
            &flat_refs,
            cfg.mult_spec.method.into(),
            cfg.mult_spec.threshold,
            ci_alpha,
            cfg.mult_spec.report_confidence_intervals,
        )?
    } else {
        let level_means: Vec<Option<f64>> = vec![None; k_t];
        let mut all_details = Vec::new();
        for (slice_levels, pairs) in slice_groups {
            let (details, note_opt) = compare_contrast_family(ContrastFamilyEval {
                levels: &slice_levels,
                level_means: &level_means,
                mult_spec: cfg.mult_spec,
                confidence: cfg.confidence,
                n_slices,
                pairs,
            })?;
            if dunnett_fallback_note.is_none() {
                dunnett_fallback_note = note_opt;
            }
            all_details.extend(details);
        }
        all_details
    };

    let mut notes = Vec::new();
    if combined.len() >= 2 && !matches!(cfg.mult_spec.method, MultiplicityMethod::None | MultiplicityMethod::FisherLsd) {
        notes.push(format!("Multiplicity adjusted across all {} simple-effect comparisons.", combined.len()));
    }
    if let Some(dn) = dunnett_fallback_note {
        notes.push(dn);
    }
    let merged_note = if notes.is_empty() { None } else { Some(notes.join(" ")) };
    push_pairwise_outputs_with_footnote(out, &combined, cfg.tab_id, cfg.title, cfg.p_fmt, cfg.confidence, merged_note);
    Ok(())
}

pub(in crate::anova) fn run_factorial_multiplicity(
    out: &mut ResultSheet,
    obs: &FactorialObservations,
    post_hoc: &FactorialPostHocContext,
    result: &FactorialAnovaResult,
    mult_spec: &MultiplicitySpec,
    p_fmt: PValueFormat,
    confidence: ConfidenceLevel,
) -> Result<(), StatsError> {
    if mult_spec.method == MultiplicityMethod::None {
        return Ok(());
    }
    if !result.df_residual.is_finite() || result.df_residual < 1.0 || !result.ms_residual.is_finite() || result.ms_residual < 0.0 {
        return Ok(());
    }
    let dunnett_target = match mult_spec.method {
        MultiplicityMethod::Dunnett { control_column } => Some((resolve_factorial_dunnett_target(obs, control_column)?, control_column)),
        _ => None,
    };
    let unequal = is_unequal_variance_method(mult_spec.method);
    let mut next_group_id = 0_usize;

    for (f_idx, levels) in obs.factor_levels.iter().enumerate() {
        let k = levels.len();
        if k < 2 {
            continue;
        }
        if let Some((target_f, _)) = dunnett_target
            && f_idx != target_f
        {
            continue;
        }
        validate_dunnett_ctrl(mult_spec, k)?;
        let base_id = next_group_id;
        next_group_id += k;
        let numbered_levels: Vec<(usize, String)> = levels.iter().enumerate().map(|(i, l)| (base_id + i, l.clone())).collect();
        let pairs = collect_factor_pairs(k, mult_spec.method, |i, j| post_hoc.eval_marginal_pair(f_idx, i, j, unequal))?;
        if pairs.is_empty() {
            continue;
        }
        let mut level_means = Vec::with_capacity(k);
        for lvl in 0..k {
            level_means.push(post_hoc.level_mean_estimate(f_idx, lvl)?);
        }
        let (details, fam_note) = compare_contrast_family(ContrastFamilyEval {
            levels: &numbered_levels,
            level_means: &level_means,
            mult_spec,
            confidence,
            n_slices: 1,
            pairs,
        })?;
        let tab_id = match f_idx {
            0 => String::from("multiple-comparisons-factor-a"),
            1 => String::from("multiple-comparisons-factor-b"),
            n => format!("multiple-comparisons-factor-{}", n + 1),
        };
        let f_name = obs.factor_names.get(f_idx).map_or("Factor", String::as_str);
        let title = format!("Multiple comparisons: {f_name} (marginal means)");
        let mut notes = Vec::new();
        if let Some((_, ctrl_idx)) = dunnett_target
            && obs.factor_levels.len() >= 2
        {
            let ctrl_lbl = levels.get(ctrl_idx).map_or("control", String::as_str);
            notes.push(format!("Dunnett many-to-one comparisons target {f_name} against control level `{ctrl_lbl}` (index {ctrl_idx})."));
        }
        if let Some(fn_str) = fam_note {
            notes.push(fn_str);
        }
        let extra_note = if notes.is_empty() { None } else { Some(notes.join(" ")) };
        push_pairwise_outputs_with_footnote(out, &details, &tab_id, &title, p_fmt, confidence, extra_note);
    }

    if obs.factor_levels.len() >= 2 {
        let name_a = obs.factor_names.first().map_or("Factor A", String::as_str);
        let name_b = obs.factor_names.get(1).map_or("Factor B", String::as_str);
        if dunnett_target.is_none_or(|(target_f, _)| target_f == 0) {
            let title_a_within_b = format!("Simple effects: {name_a} within {name_b}");
            run_simple_effects_for_pair(
                out,
                obs,
                post_hoc,
                &SimpleEffectsSpec {
                    cond_factor: 1,
                    target_factor: 0,
                    mult_spec,
                    tab_id: "multiple-comparisons-simple-a-within-b",
                    title: &title_a_within_b,
                    p_fmt,
                    confidence,
                },
                &mut next_group_id,
            )?;
        }
        if dunnett_target.is_none_or(|(target_f, _)| target_f == 1) {
            let title_b_within_a = format!("Simple effects: {name_b} within {name_a}");
            run_simple_effects_for_pair(
                out,
                obs,
                post_hoc,
                &SimpleEffectsSpec {
                    cond_factor: 0,
                    target_factor: 1,
                    mult_spec,
                    tab_id: "multiple-comparisons-simple-b-within-a",
                    title: &title_b_within_a,
                    p_fmt,
                    confidence,
                },
                &mut next_group_id,
            )?;
        }
    }

    Ok(())
}
