//! Principal Component Analysis (SVD decomposition, Kaiser–Guttman retention, and Horn's
//! parallel analysis with Glorfeld 95th-percentile thresholds).
//!
//! References:
//! - Jolliffe, I. T. (2002). *Principal Component Analysis* (2nd ed.). New York: Springer.
//! - Horn, J. L. (1965). A rationale and test for the number of factors in factor analysis.
//!   *Psychometrika*, 30(2), 179-185.
//! - Glorfeld, L. W. (1995). An improvement on Horn's parallel analysis methodology for selecting
//!   the correct number of factors to retain. *Educational and Psychological Measurement*, 55(3),
//!   377-393.

use curvebook_math::linalg::{DenseMatrix, svd};
use curvebook_math::rng::DeterministicRng;
use curvebook_math::sum::SummaryMoments;

use super::types::PcaResult;
use crate::common::quantile_sorted;
use crate::error::StatsError;

/// Principal Component Analysis via SVD of centered (and optionally standardized) `n x p` matrix
/// (Jolliffe 2002).
///
/// Guarantees `sum_k eigenvalue_k == trace(Cov)` (or `trace(Corr) = p` when `standardize = true`).
pub fn pca_analysis(
    data: &DenseMatrix,
    variable_labels: &[String],
    standardize: bool,
    parallel_analysis_seed: Option<u64>,
) -> Result<PcaResult, StatsError> {
    let n = data.nrows();
    let p = data.ncols();
    if n < 3 || p < 2 {
        return Err(StatsError::InsufficientSampleSize { context: "PCA", required: 3, actual: n });
    }
    if variable_labels.len() != p {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: p, actual: variable_labels.len() }));
    }

    let mut col_sds = Vec::with_capacity(p);
    let mut scaled_cols = Vec::with_capacity(p);
    for c in 0..p {
        let mut col = Vec::with_capacity(n);
        for r in 0..n {
            let v = data.get(r, c).unwrap_or(f64::NAN);
            if !v.is_finite() {
                return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "pca_x", value: v }));
            }
            col.push(v);
        }
        let m = SummaryMoments::from_slice(&col)?;
        let sd = m.sample_sd().unwrap_or(0.0);
        if standardize && (!sd.is_finite() || sd <= 0.0) {
            return Err(StatsError::ZeroVariance { context: "standardized PCA constant column" });
        }
        col_sds.push(sd);
        let denom = if standardize { sd } else { 1.0 };
        scaled_cols.push(col.into_iter().map(|v| (v - m.mean) / denom).collect::<Vec<_>>());
    }
    if !standardize && col_sds.iter().all(|&sd| sd <= 0.0) {
        return Err(StatsError::ZeroVariance { context: "PCA covariance matrix" });
    }

    // Build X_norm = X_centered / sqrt(n - 1) so singular_values^2 == eigenvalues of Cov/Corr.
    let scale_n = ((n - 1) as f64).sqrt();
    let mut x_scaled_data = vec![0.0_f64; n * p];
    let mut x_norm_data = vec![0.0_f64; n * p];
    for r in 0..n {
        for c in 0..p {
            let v = scaled_cols[c][r];
            x_scaled_data[r * p + c] = v;
            x_norm_data[r * p + c] = v / scale_n;
        }
    }
    let x_scaled = DenseMatrix::from_row_major(n, p, x_scaled_data)?;
    let x_norm = DenseMatrix::from_row_major(n, p, x_norm_data)?;
    let dec = svd(&x_norm)?;

    let k_comp = n.min(p);
    let mut eigenvalues = vec![0.0_f64; k_comp];
    for (i, &s) in dec.singular_values.iter().take(k_comp).enumerate() {
        eigenvalues[i] = s * s;
    }
    let total_var: f64 = eigenvalues.iter().sum::<f64>().max(1e-30);
    let variance_proportion: Vec<f64> = eigenvalues.iter().map(|&ev| ev / total_var).collect();
    let mut cumulative_proportion = Vec::with_capacity(k_comp);
    let mut run = 0.0_f64;
    for &vp in &variance_proportion {
        run += vp;
        cumulative_proportion.push(run.min(1.0));
    }

    let kaiser_thresh = if standardize { 1.0 } else { total_var / (p as f64) };
    let kaiser_components = eigenvalues.iter().filter(|&&ev| ev > kaiser_thresh).count();

    // Loadings V (p x k_comp) with deterministic sign convention (Jolliffe 2002): orient each
    // component j so its largest-magnitude loading (breaking near-ties within relative 1e-12 by
    // smallest variable index) is non-negative, then compute Scores = X_scaled * V.
    let mut loadings = vec![vec![0.0_f64; k_comp]; p];
    let mut contributions = vec![vec![0.0_f64; k_comp]; p];
    for j in 0..k_comp {
        let mut max_abs = 0.0_f64;
        for i in 0..p {
            max_abs = max_abs.max(dec.v.get(i, j).unwrap_or(0.0).abs());
        }
        let mut sign = 1.0_f64;
        if max_abs > 0.0 {
            let tie_thresh = max_abs * (1.0 - 1e-12);
            for i in 0..p {
                let vij = dec.v.get(i, j).unwrap_or(0.0);
                if vij.abs() >= tie_thresh {
                    if vij < 0.0 {
                        sign = -1.0;
                    }
                    break;
                }
            }
        }
        for i in 0..p {
            let v_ij = sign * dec.v.get(i, j).unwrap_or(0.0);
            loadings[i][j] = v_ij;
            contributions[i][j] = 100.0 * v_ij * v_ij;
        }
    }
    let mut scores = vec![vec![0.0_f64; k_comp]; n];
    for (r, row_scores) in scores.iter_mut().enumerate() {
        for (j, slot) in row_scores.iter_mut().enumerate() {
            let mut acc = 0.0_f64;
            for (c, load_row) in loadings.iter().enumerate() {
                acc += x_scaled.get(r, c).unwrap_or(0.0) * load_row[j];
            }
            *slot = acc;
        }
    }

    let (pa_thresholds, pa_count) = if let Some(seed) = parallel_analysis_seed {
        let thresh = horn_parallel_analysis(n, p, k_comp, standardize, &col_sds, seed)?;
        let cnt = eigenvalues.iter().zip(thresh.iter()).take_while(|(ev, th)| **ev > **th).count();
        (Some(thresh), Some(cnt))
    } else {
        (None, None)
    };

    Ok(PcaResult {
        n,
        p,
        standardized: standardize,
        variable_labels: variable_labels.to_vec(),
        eigenvalues,
        variance_proportion,
        cumulative_proportion,
        kaiser_components,
        parallel_analysis_thresholds: pa_thresholds,
        parallel_analysis_components: pa_count,
        loadings,
        scores,
        variable_contributions_pct: contributions,
    })
}

/// Horn (1965, *Psychometrika* 30:179-185) parallel analysis with Glorfeld (1995) 95th-percentile
/// threshold: simulate independent Gaussian columns matched to the empirical variance scale
/// (standardized to unit variance when `standardize = true`; centered and scaled by each observed
/// variable's sample SD `col_sds[c]` when `standardize = false`).
fn horn_parallel_analysis(n: usize, p: usize, k_comp: usize, standardize: bool, col_sds: &[f64], seed: u64) -> Result<Vec<f64>, StatsError> {
    let n_iter = 40_usize;
    let mut rng = DeterministicRng::from_seed(seed);
    let mut sim_eigs = vec![Vec::with_capacity(n_iter); k_comp];
    let scale_n = ((n - 1) as f64).sqrt();

    for _ in 0..n_iter {
        let mut mat_data = vec![0.0_f64; n * p];
        for c in 0..p {
            let mut col: Vec<f64> = (0..n).map(|_| rng.standard_normal()).collect();
            if let Ok(m) = SummaryMoments::from_slice(&col) {
                if standardize {
                    let sd = m.sample_sd().unwrap_or(1.0).max(1e-12);
                    for v in &mut col {
                        *v = (*v - m.mean) / sd;
                    }
                } else {
                    let target_sd = col_sds.get(c).copied().unwrap_or(0.0).max(0.0);
                    for v in &mut col {
                        *v = (*v - m.mean) * target_sd;
                    }
                }
            }
            for (r, &val) in col.iter().enumerate() {
                mat_data[r * p + c] = val / scale_n;
            }
        }
        let m = DenseMatrix::from_row_major(n, p, mat_data)?;
        let dec = svd(&m)?;
        for (j, &s) in dec.singular_values.iter().take(k_comp).enumerate() {
            sim_eigs[j].push(s * s);
        }
    }
    let fallback = if standardize { 1.0 } else { 0.0 };
    let mut out = Vec::with_capacity(k_comp);
    for mut v in sim_eigs {
        v.sort_by(f64::total_cmp);
        out.push(quantile_sorted(&v, 0.95).unwrap_or(fallback));
    }
    Ok(out)
}
