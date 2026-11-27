//! General factorial (two-way, three-way, and *N*-way) fixed-effects ANOVA engine with Type I and
//! Type III sums of squares.
//!
//! References:
//! - Searle, S. R. (1987). *Linear Models for Unbalanced Data*. Wiley.
//! - Maxwell, S. E., & Delaney, H. D. (2004). *Designing Experiments and Analyzing Data: A Model
//!   Comparison Perspective* (2nd ed.). Lawrence Erlbaum.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Pearson Prentice Hall.

use curvebook_doc::SumOfSquaresType;
use curvebook_math::dist::f_sf;
use curvebook_math::linalg::matrix::MAX_MATRIX_ELEMENTS;
use curvebook_math::linalg::{DenseMatrix, solve_linear_least_squares, sym_eigen};
use curvebook_math::sum::SummaryMoments;

use crate::anova::reml::symmetrize_dense;
use crate::anova::results::{FactorialAnovaResult, FactorialAnovaTerm};
use crate::common::validate_finite_slice;
use crate::error::StatsError;

/// Maximum number of design-matrix columns permitted in a factorial / REML effect-coded design.
pub(in crate::anova) const MAX_FACTORIAL_DESIGN_COLS: usize = 4_096;
/// Maximum number of model terms (main effects + interactions) permitted in a factorial design.
pub(in crate::anova) const MAX_FACTORIAL_TERMS: usize = 4_096;

/// Allocate a zero-initialized dense row-major buffer after checking `rows * cols <= MAX_MATRIX_ELEMENTS`.
pub(in crate::anova) fn checked_dense_vec(rows: usize, cols: usize) -> Result<Vec<f64>, StatsError> {
    let Some(len) = rows.checked_mul(cols) else {
        return Err(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY }));
    };
    if len > MAX_MATRIX_ELEMENTS {
        return Err(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: len as f64 }));
    }
    Ok(vec![0.0_f64; len])
}

/// Pre-flight check on factorial interaction subset count, total design column count, and total
/// matrix element count (`n_obs * total_cols <= MAX_MATRIX_ELEMENTS`) using elementary symmetric
/// polynomial DP with checked arithmetic before enumerating any factor combinations.
pub(in crate::anova) fn plan_factorial_design_subsets(
    n_obs: usize,
    factor_level_counts: &[usize],
    max_interaction_order: usize,
) -> Result<(Vec<Vec<usize>>, usize), StatsError> {
    let n_factors = factor_level_counts.len();
    if n_factors == 0 {
        return Ok((Vec::new(), 1));
    }
    let order_cap = max_interaction_order.clamp(1, n_factors);
    // DP over elementary symmetric polynomials:
    // term_dp[k] = C(seen, k), col_dp[k] = sum_{|S|=k} prod_{f in S} (levels_f - 1).
    let mut term_dp = vec![0_usize; order_cap + 1];
    let mut col_dp = vec![0_usize; order_cap + 1];
    term_dp[0] = 1;
    col_dp[0] = 1;
    for &n_lvl in factor_level_counts {
        let df_f = n_lvl.saturating_sub(1);
        for k in (1..=order_cap).rev() {
            let next_terms =
                term_dp[k].checked_add(term_dp[k - 1]).ok_or(StatsError::InvalidParameter { name: "factorial_terms", value: f64::INFINITY })?;
            if next_terms > MAX_FACTORIAL_TERMS {
                return Err(StatsError::InvalidParameter { name: "factorial_terms", value: next_terms as f64 });
            }
            term_dp[k] = next_terms;
            let add_cols =
                col_dp[k - 1].checked_mul(df_f).ok_or(StatsError::InvalidParameter { name: "factorial_design_columns", value: f64::INFINITY })?;
            let next_cols =
                col_dp[k].checked_add(add_cols).ok_or(StatsError::InvalidParameter { name: "factorial_design_columns", value: f64::INFINITY })?;
            if next_cols > MAX_FACTORIAL_DESIGN_COLS {
                return Err(StatsError::InvalidParameter { name: "factorial_design_columns", value: next_cols as f64 });
            }
            col_dp[k] = next_cols;
        }
    }
    let mut total_terms = 0_usize;
    let mut total_cols = 1_usize; // intercept column
    for k in 1..=order_cap {
        total_terms = total_terms.checked_add(term_dp[k]).ok_or(StatsError::InvalidParameter { name: "factorial_terms", value: f64::INFINITY })?;
        total_cols =
            total_cols.checked_add(col_dp[k]).ok_or(StatsError::InvalidParameter { name: "factorial_design_columns", value: f64::INFINITY })?;
    }
    if total_terms > MAX_FACTORIAL_TERMS {
        return Err(StatsError::InvalidParameter { name: "factorial_terms", value: total_terms as f64 });
    }
    if total_cols > MAX_FACTORIAL_DESIGN_COLS {
        return Err(StatsError::InvalidParameter { name: "factorial_design_columns", value: total_cols as f64 });
    }
    let total_elems = n_obs
        .checked_mul(total_cols)
        .ok_or(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY }))?;
    if total_elems > MAX_MATRIX_ELEMENTS {
        return Err(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: total_elems as f64 }));
    }
    let mut subsets: Vec<Vec<usize>> = Vec::with_capacity(total_terms);
    for ord in 1..=order_cap {
        collect_index_combinations(n_factors, ord, 0, &mut Vec::new(), &mut subsets);
    }
    Ok((subsets, total_cols))
}

/// Factor definition and active observations for general factorial ANOVA (`N >= 1` factors).
#[derive(Debug, Clone)]
pub struct FactorialObservations {
    /// Ordered names of the `N` experimental factors (`len == N`).
    pub factor_names: Vec<String>,
    /// Ordered distinct level labels per factor (`factor_levels[f][level_idx]`).
    pub factor_levels: Vec<Vec<String>>,
    /// Per observation: `(level_indices_per_factor, y)`.
    pub rows: Vec<(Vec<usize>, f64)>,
}

#[derive(Debug, Clone)]
pub(in crate::anova) struct TermSpec {
    pub(in crate::anova) name: String,
    pub(in crate::anova) col_start: usize,
    pub(in crate::anova) col_count: usize,
}

#[derive(Debug, Clone)]
pub(in crate::anova) struct CellSampleStats {
    pub(in crate::anova) count: usize,
    pub(in crate::anova) var_unbiased: f64,
}

#[derive(Debug, Clone)]
pub(in crate::anova) struct OlsPostHocModel {
    pub(in crate::anova) factor_levels: Vec<Vec<String>>,
    pub(in crate::anova) factor_basis: Vec<Vec<Vec<f64>>>,
    pub(in crate::anova) subsets: Vec<Vec<usize>>,
    pub(in crate::anova) total_cols: usize,
    pub(in crate::anova) beta: Vec<f64>,
    pub(in crate::anova) cov_unscaled: DenseMatrix,
    pub(in crate::anova) row_proj: DenseMatrix,
    pub(in crate::anova) ms_resid: f64,
    pub(in crate::anova) df_resid: f64,
    pub(in crate::anova) cell_stats: std::collections::BTreeMap<Vec<usize>, CellSampleStats>,
}

pub(in crate::anova) fn build_xtx_pinv_and_proj(x: &DenseMatrix) -> Result<(DenseMatrix, DenseMatrix), StatsError> {
    let p = x.ncols();
    let xtx = symmetrize_dense(&x.transpose().matmul(x)?)?;
    let eig = sym_eigen(&xtx)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let tol = max_ev.max(1.0) * (p.max(1) as f64) * 1e-11;
    let mut pinv = checked_dense_vec(p, p)?;
    let mut proj = checked_dense_vec(p, p)?;
    for (k, &ev) in eig.eigenvalues.iter().enumerate() {
        if ev <= tol {
            continue;
        }
        let inv_ev = 1.0 / ev;
        for i in 0..p {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..p {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let dp = qi * qj;
                let di = dp * inv_ev;
                proj[i * p + j] += dp;
                pinv[i * p + j] += di;
                if i != j {
                    proj[j * p + i] += dp;
                    pinv[j * p + i] += di;
                }
            }
        }
    }
    Ok((DenseMatrix::from_row_major(p, p, pinv)?, DenseMatrix::from_row_major(p, p, proj)?))
}

pub(in crate::anova) fn build_cell_stats_map(
    rows: &[(Vec<usize>, f64)],
) -> Result<std::collections::BTreeMap<Vec<usize>, CellSampleStats>, StatsError> {
    let mut grouped: std::collections::BTreeMap<Vec<usize>, Vec<f64>> = std::collections::BTreeMap::new();
    for (lvls, y) in rows {
        grouped.entry(lvls.clone()).or_default().push(*y);
    }
    let mut out = std::collections::BTreeMap::new();
    for (cell, vals) in grouped {
        let count = vals.len();
        let m = SummaryMoments::from_slice(&vals)?;
        let var_unbiased = if count >= 2 && m.css.is_finite() { (m.css / ((count - 1) as f64)).max(0.0) } else { 0.0 };
        out.insert(cell, CellSampleStats { count, var_unbiased });
    }
    Ok(out)
}

/// Fit Type I (sequential) or Type III (partial, sum/effect-coded) general `N`-way factorial ANOVA
/// via `curvebook_math::linalg::solve_linear_least_squares`.
///
/// Sums of squares come from full-vs-reduced model comparisons (Maxwell & Delaney, 2004, ch. 7;
/// Searle, 1987). Type I: sequential RSS reductions in term order. Type III: RSS increase from
/// dropping each term's sum-to-zero-coded columns from the full model. With no empty cells this
/// matches the usual Type III hypotheses on unweighted marginal means. With empty cells it tests
/// whatever estimable function remains, which is not the Type IV construction.
pub fn fit_factorial_anova(
    data: &FactorialObservations,
    max_interaction_order: usize,
    ss_type: SumOfSquaresType,
) -> Result<FactorialAnovaResult, StatsError> {
    let (res, _) = fit_factorial_anova_with_model(data, max_interaction_order, ss_type)?;
    Ok(res)
}

pub(in crate::anova) fn fit_factorial_anova_with_model(
    data: &FactorialObservations,
    max_interaction_order: usize,
    ss_type: SumOfSquaresType,
) -> Result<(FactorialAnovaResult, OlsPostHocModel), StatsError> {
    fit_factorial_anova_with_model_and_subsets(data, max_interaction_order, None, ss_type)
}

pub(in crate::anova) fn fit_factorial_anova_with_model_and_subsets(
    data: &FactorialObservations,
    max_interaction_order: usize,
    explicit_subsets: Option<Vec<Vec<usize>>>,
    ss_type: SumOfSquaresType,
) -> Result<(FactorialAnovaResult, OlsPostHocModel), StatsError> {
    let n_obs = data.rows.len();
    let n_factors = data.factor_names.len();
    if n_obs < 3 || n_factors == 0 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial ANOVA observations", required: 3, actual: n_obs });
    }
    if data.factor_levels.len() != n_factors {
        return Err(StatsError::DimensionMismatch { expected: n_factors, actual: data.factor_levels.len() });
    }
    for (lvls, _) in &data.rows {
        if lvls.len() != n_factors {
            return Err(StatsError::DimensionMismatch { expected: n_factors, actual: lvls.len() });
        }
        for (f_idx, &lvl) in lvls.iter().enumerate() {
            if lvl >= data.factor_levels[f_idx].len() {
                return Err(StatsError::InvalidParameter { name: "factor_level_index", value: lvl as f64 });
            }
        }
    }
    let y: Vec<f64> = data.rows.iter().map(|(_, yv)| *yv).collect();
    validate_finite_slice(&y)?;
    let y_moments = SummaryMoments::from_slice(&y)?;

    // Pre-check interaction term/column/element bounds before expanding factor combinations.
    let level_counts: Vec<usize> = data.factor_levels.iter().map(Vec::len).collect();
    let (subsets, expected_cols) = if let Some(custom_subsets) = explicit_subsets {
        if custom_subsets.is_empty() {
            return Err(StatsError::EmptyInput);
        }
        let mut total_cols = 1_usize;
        for sub in &custom_subsets {
            let mut sub_cols = 1_usize;
            for &f in sub {
                let n_lvl = *level_counts.get(f).ok_or(StatsError::MissingColumn { index: f })?;
                sub_cols = sub_cols
                    .checked_mul(n_lvl.saturating_sub(1))
                    .ok_or(StatsError::InvalidParameter { name: "factorial_design_columns", value: f64::INFINITY })?;
            }
            total_cols =
                total_cols.checked_add(sub_cols).ok_or(StatsError::InvalidParameter { name: "factorial_design_columns", value: f64::INFINITY })?;
            if total_cols > MAX_FACTORIAL_DESIGN_COLS {
                return Err(StatsError::InvalidParameter { name: "factorial_design_columns", value: total_cols as f64 });
            }
        }
        let total_elems = n_obs
            .checked_mul(total_cols)
            .ok_or(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: f64::INFINITY }))?;
        if total_elems > MAX_MATRIX_ELEMENTS {
            return Err(StatsError::Math(curvebook_math::MathError::InvalidParameter { name: "matrix_elements", value: total_elems as f64 }));
        }
        (custom_subsets, total_cols)
    } else {
        plan_factorial_design_subsets(n_obs, &level_counts, max_interaction_order)?
    };

    // Build sum-to-zero effect coding (+1 / -1) for each factor (needed for valid Type III partial SS!).
    let factor_basis: Vec<Vec<Vec<f64>>> = data.factor_levels.iter().map(|levels| sum_contrast_vectors(levels.len())).collect();

    let mut cols: Vec<Vec<f64>> = Vec::with_capacity(expected_cols);
    cols.push(vec![1.0_f64; n_obs]); // column 0 = intercept
    let mut terms = Vec::with_capacity(subsets.len());

    for sub in &subsets {
        let col_start = cols.len();
        let prod_cols = kronecker_term_columns(&data.rows, &factor_basis, sub);
        let col_count = prod_cols.len();
        cols.extend(prod_cols);
        let name = sub.iter().map(|&idx| data.factor_names[idx].as_str()).collect::<Vec<_>>().join(" × ");
        terms.push(TermSpec { name, col_start, col_count });
    }

    let total_cols = cols.len();
    let full_x = columns_to_matrix(&cols, n_obs)?;
    let (full_sol, _) = solve_linear_least_squares(&full_x, &y)?;
    let ss_resid = full_sol.rss.max(0.0);
    let df_resid = full_sol.df_residual as f64;
    if df_resid <= 0.0 {
        return Err(StatsError::InsufficientSampleSize { context: "factorial ANOVA design df", required: total_cols + 1, actual: n_obs });
    }
    let ms_resid = ss_resid / df_resid;
    let ss_total = y_moments.css.max(ss_resid);

    let unbalanced = is_design_unbalanced(&data.rows, &data.factor_levels);
    let mut out_terms = Vec::with_capacity(terms.len());

    match ss_type {
        SumOfSquaresType::TypeIII => {
            for t in &terms {
                let mut dropped_cols = Vec::with_capacity(total_cols - t.col_count);
                for (c_idx, col) in cols.iter().enumerate() {
                    if c_idx < t.col_start || c_idx >= t.col_start + t.col_count {
                        dropped_cols.push(col.clone());
                    }
                }
                let red_x = columns_to_matrix(&dropped_cols, n_obs)?;
                let (red_sol, _) = solve_linear_least_squares(&red_x, &y)?;
                let ss_t = (red_sol.rss - ss_resid).max(0.0);
                let df_t = full_sol.rank.saturating_sub(red_sol.rank) as f64;
                out_terms.push(build_factorial_term(&t.name, ss_t, df_t, ms_resid, ss_resid, ss_total)?);
            }
        }
        SumOfSquaresType::TypeI => {
            let mut prev_cols = vec![cols[0].clone()];
            let mut prev_rss = y_moments.css;
            let mut prev_rank = 1_usize;
            for t in &terms {
                prev_cols.extend_from_slice(&cols[t.col_start..(t.col_start + t.col_count)]);
                let seq_x = columns_to_matrix(&prev_cols, n_obs)?;
                let (seq_sol, _) = solve_linear_least_squares(&seq_x, &y)?;
                let ss_t = (prev_rss - seq_sol.rss).max(0.0);
                let df_t = seq_sol.rank.saturating_sub(prev_rank) as f64;
                prev_rss = seq_sol.rss;
                prev_rank = seq_sol.rank;
                out_terms.push(build_factorial_term(&t.name, ss_t, df_t, ms_resid, ss_resid, ss_total)?);
            }
        }
    }

    let mut res = FactorialAnovaResult {
        terms: out_terms,
        ss_residual: ss_resid,
        df_residual: df_resid,
        ms_residual: ms_resid,
        ss_total,
        df_total: (n_obs - 1) as f64,
        total_n: n_obs,
        unbalanced,
        ss_type,
    };
    finalize_term_p_values(&mut res)?;
    let (cov_unscaled, row_proj) = build_xtx_pinv_and_proj(&full_x)?;
    let cell_stats = build_cell_stats_map(&data.rows)?;
    let model = OlsPostHocModel {
        factor_levels: data.factor_levels.clone(),
        factor_basis,
        subsets,
        total_cols,
        beta: full_sol.coefficients,
        cov_unscaled,
        row_proj,
        ms_resid,
        df_resid,
        cell_stats,
    };
    Ok((res, model))
}

pub(in crate::anova) fn build_factorial_term(
    name: &str,
    ss: f64,
    df: f64,
    ms_resid: f64,
    ss_resid: f64,
    ss_total: f64,
) -> Result<FactorialAnovaTerm, StatsError> {
    if df <= 0.0 {
        return Ok(FactorialAnovaTerm {
            name: name.to_owned(),
            ss: 0.0,
            df: 0.0,
            ms: 0.0,
            f_statistic: None,
            p_value: None,
            partial_eta_squared: None,
            eta_squared: None,
        });
    }
    let ms = ss / df;
    let f_val = if ms_resid > 0.0 {
        ms / ms_resid
    } else if ms > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    Ok(FactorialAnovaTerm {
        name: name.to_owned(),
        ss,
        df,
        ms,
        f_statistic: Some(f_val),
        p_value: None,
        partial_eta_squared: Some(if ss + ss_resid > 0.0 { ss / (ss + ss_resid) } else { 0.0 }),
        eta_squared: Some(if ss_total > 0.0 { ss / ss_total } else { 0.0 }),
    })
}

pub(in crate::anova) fn finalize_term_p_values(res: &mut FactorialAnovaResult) -> Result<(), StatsError> {
    for t in &mut res.terms {
        if t.df > 0.0
            && let Some(f_val) = t.f_statistic
        {
            t.p_value = Some(f_sf(f_val, t.df, res.df_residual.max(1.0))?);
        }
    }
    Ok(())
}

pub(in crate::anova) fn collect_index_combinations(n: usize, k: usize, start: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if cur.len() == k {
        out.push(cur.clone());
        return;
    }
    for i in start..n {
        cur.push(i);
        collect_index_combinations(n, k, i + 1, cur, out);
        cur.pop();
    }
}

pub(in crate::anova) fn sum_contrast_vectors(n_levels: usize) -> Vec<Vec<f64>> {
    if n_levels <= 1 {
        return Vec::new();
    }
    let m = n_levels - 1;
    let mut basis = vec![vec![0.0_f64; m]; n_levels];
    for (lvl, row) in basis.iter_mut().enumerate().take(m) {
        row[lvl] = 1.0;
    }
    basis[n_levels - 1].fill(-1.0);
    basis
}

pub(in crate::anova) fn kronecker_term_columns(rows: &[(Vec<usize>, f64)], factor_basis: &[Vec<Vec<f64>>], sub_factors: &[usize]) -> Vec<Vec<f64>> {
    let mut prod = vec![vec![1.0_f64; rows.len()]];
    for &f_idx in sub_factors {
        let Some(f_rows) = factor_basis.get(f_idx) else {
            return Vec::new();
        };
        let n_cols_f = f_rows.first().map_or(0, Vec::len);
        if n_cols_f == 0 {
            return Vec::new();
        }
        let mut next = Vec::with_capacity(prod.len() * n_cols_f);
        for prev in &prod {
            next.extend((0..n_cols_f).map(|c_f| {
                rows.iter()
                    .enumerate()
                    .map(|(r_idx, (lvls, _))| {
                        let coeff = lvls.get(f_idx).and_then(|&lvl| f_rows.get(lvl)).and_then(|row| row.get(c_f)).copied().unwrap_or(0.0);
                        prev[r_idx] * coeff
                    })
                    .collect()
            }));
        }
        prod = next;
    }
    prod
}

pub(in crate::anova) fn columns_to_matrix(cols: &[Vec<f64>], n_rows: usize) -> Result<DenseMatrix, StatsError> {
    let n_cols = cols.len();
    let mut data = checked_dense_vec(n_rows, n_cols)?;
    for (c, col) in cols.iter().enumerate() {
        for (r, &v) in col.iter().enumerate() {
            data[r * n_cols + c] = v;
        }
    }
    Ok(DenseMatrix::from_row_major(n_rows, n_cols, data)?)
}

pub(in crate::anova) fn is_design_unbalanced(rows: &[(Vec<usize>, f64)], levels: &[Vec<String>]) -> bool {
    if levels.is_empty() {
        return false;
    }
    let mut counts: std::collections::BTreeMap<Vec<usize>, usize> = std::collections::BTreeMap::new();
    for (lvls, _) in rows {
        *counts.entry(lvls.clone()).or_insert(0) += 1;
    }
    let Some(expected_cells) = levels.iter().try_fold(1_usize, |acc, v| acc.checked_mul(v.len().max(1))) else {
        return true;
    };
    if counts.len() < expected_cells {
        return true;
    }
    let first = counts.values().next().copied().unwrap_or(0);
    counts.values().any(|&c| c != first)
}
