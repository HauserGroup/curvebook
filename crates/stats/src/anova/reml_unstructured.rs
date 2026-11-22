//! Repeated-measures REML with an unstructured residual covariance (log-Cholesky
//! parametrisation), including the documented fallbacks when the unstructured fit is not usable.
//!
//! References:
//! - Pinheiro, J. C., & Bates, D. M. (1996). Unconstrained parametrizations for variance-covariance
//!   matrices. *Statistics and Computing* 6(3):289–296. doi:10.1007/BF00140873
//! - Lindstrom, M. J., & Bates, D. M. (1988). Newton-Raphson and EM algorithms for linear
//!   mixed-effects models for repeated-measures data. *Journal of the American Statistical
//!   Association* 83(404):1014–1022. doi:10.1080/01621459.1988.10478693
//! - Verbeke, G., & Molenberghs, G. (2000). *Linear Mixed Models for Longitudinal Data*. Springer.
//! - Patterson, H. D., & Thompson, R. (1971). Recovery of inter-block information when block sizes
//!   are unequal. *Biometrika* 58(3):545–554. doi:10.1093/biomet/58.3.545
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558

use curvebook_doc::MixedModelDfMethod;
use curvebook_math::linalg::{DenseMatrix, cholesky, qr_col_pivoted, sym_eigen};
use curvebook_math::sum::SummaryMoments;

use crate::anova::reml::{OwnedRemlFisherMats, RemlContrastCore, eval_reml_wald_terms_for_df, scale_dense, symmetrize_dense, trace_product};
use crate::anova::results::{UnstructuredFallbackReason, UnstructuredRemlResult};
use crate::common::validate_finite_slice;
use crate::error::StatsError;

pub(in crate::anova) const LOG_CHOL_DIAG_MIN: f64 = -9.0;
pub(in crate::anova) const LOG_CHOL_DIAG_MAX: f64 = 9.0;

pub(in crate::anova) fn chol_diag_mask(t: usize) -> Vec<bool> {
    let mut is_diag = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..=i {
            is_diag.push(i == j);
        }
    }
    is_diag
}

pub(in crate::anova) struct UnstructuredClusterBlock {
    pub(in crate::anova) x_c: DenseMatrix,
    pub(in crate::anova) y_c: Vec<f64>,
    pub(in crate::anova) tps: Vec<usize>,
}

pub(in crate::anova) struct UnstructuredEval {
    pub(in crate::anova) ll: f64,
    pub(in crate::anova) beta: Vec<f64>,
    pub(in crate::anova) phi: DenseMatrix,
    pub(in crate::anova) g_sigma: DenseMatrix,
}

pub(in crate::anova) struct UnstructuredPrepared {
    pub(in crate::anova) blocks: Vec<UnstructuredClusterBlock>,
    pub(in crate::anova) reduced_terms: Vec<(String, usize, usize)>,
    pub(in crate::anova) col_map: Vec<Option<usize>>,
    pub(in crate::anova) active_tp_indices: Vec<usize>,
    pub(in crate::anova) active_labels: Vec<String>,
    pub(in crate::anova) p_eff: usize,
    pub(in crate::anova) n_obs: usize,
    pub(in crate::anova) y_scale: f64,
}

pub(in crate::anova) fn sigma_from_log_chol(eta: &[f64], t: usize) -> Result<(DenseMatrix, DenseMatrix), StatsError> {
    let mut l_data = vec![0.0_f64; t * t];
    let mut idx = 0_usize;
    for i in 0..t {
        for j in 0..i {
            l_data[i * t + j] = eta[idx];
            idx += 1;
        }
        l_data[i * t + i] = eta[idx].clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX).exp();
        idx += 1;
    }
    let l_mat = DenseMatrix::from_row_major(t, t, l_data)?;
    let sigma = symmetrize_dense(&l_mat.matmul(&l_mat.transpose())?)?;
    Ok((l_mat, sigma))
}

pub(in crate::anova) fn log_chol_from_sigma(sigma: &DenseMatrix) -> Result<Vec<f64>, StatsError> {
    let chol = cholesky(&symmetrize_dense(sigma)?)?;
    let l_mat = chol.l();
    let t = l_mat.nrows();
    let mut eta = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..i {
            eta.push(l_mat.get(i, j).unwrap_or(0.0));
        }
        let d = l_mat.get(i, i).unwrap_or(1.0).max(1e-12);
        eta.push(d.ln().clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX));
    }
    Ok(eta)
}

pub(in crate::anova) fn log_chol_neg_grad(g_sigma: &DenseMatrix, l_mat: &DenseMatrix) -> Result<Vec<f64>, StatsError> {
    let gl = g_sigma.matmul(l_mat)?;
    let t = l_mat.nrows();
    let mut g_neg = Vec::with_capacity(t * (t + 1) / 2);
    for i in 0..t {
        for j in 0..i {
            g_neg.push(-2.0 * gl.get(i, j).unwrap_or(0.0));
        }
        g_neg.push(-2.0 * gl.get(i, i).unwrap_or(0.0) * l_mat.get(i, i).unwrap_or(1.0));
    }
    Ok(g_neg)
}

pub(in crate::anova) fn unstructured_floor_spd(mat: &DenseMatrix, min_ratio: f64, default_scale: f64) -> Result<(DenseMatrix, bool), StatsError> {
    let sym = symmetrize_dense(mat)?;
    let eig = sym_eigen(&sym)?;
    let t = sym.nrows();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    let floor = if max_ev > 1e-30 { (min_ratio * max_ev).max(1e-12) } else { default_scale.max(1e-8) };
    let mut adjusted = max_ev <= 1e-30;
    let mut out = vec![0.0_f64; t * t];
    for (k, &ev_raw) in eig.eigenvalues.iter().enumerate() {
        let ev = if ev_raw < floor {
            adjusted = true;
            floor
        } else {
            ev_raw
        };
        for i in 0..t {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..t {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let delta = ev * qi * qj;
                out[i * t + j] += delta;
                if i != j {
                    out[j * t + i] += delta;
                }
            }
        }
    }
    Ok((symmetrize_dense(&DenseMatrix::from_row_major(t, t, out)?)?, adjusted))
}

pub(in crate::anova) fn unstructured_eigen_ratio(mat: &DenseMatrix) -> Result<(f64, f64, f64), StatsError> {
    let eig = sym_eigen(&symmetrize_dense(mat)?)?;
    let max_ev = eig.eigenvalues.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min);
    let t = mat.nrows().max(1) as f64;
    let mean_diag: f64 = (0..mat.nrows()).map(|i| mat.get(i, i).unwrap_or(0.0).max(0.0)).sum::<f64>() / t;
    Ok((min_ev, max_ev, mean_diag))
}

pub(in crate::anova) fn unstructured_shrink_spd(mat: &DenseMatrix, target_ratio: f64, fallback_var: f64) -> Result<DenseMatrix, StatsError> {
    let eig = sym_eigen(&symmetrize_dense(mat)?)?;
    let t = mat.nrows();
    let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
    if max_ev <= 1e-30 {
        return scale_dense(&DenseMatrix::identity(t)?, fallback_var.max(1e-8));
    }
    let min_ev = eig.eigenvalues.iter().copied().fold(f64::INFINITY, f64::min).max(0.0);
    let mut psd = vec![0.0_f64; t * t];
    for (k, &ev_raw) in eig.eigenvalues.iter().enumerate() {
        let ev = ev_raw.max(0.0);
        if ev == 0.0 {
            continue;
        }
        for i in 0..t {
            let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
            for j in i..t {
                let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                let delta = ev * qi * qj;
                psd[i * t + j] += delta;
                if i != j {
                    psd[j * t + i] += delta;
                }
            }
        }
    }
    let mean_diag: f64 = (0..t).map(|i| psd[i * t + i]).sum::<f64>() / (t.max(1) as f64);
    if mean_diag <= 1e-30 {
        return scale_dense(&DenseMatrix::identity(t)?, fallback_var.max(1e-8));
    }
    if min_ev / max_ev >= target_ratio {
        return symmetrize_dense(&DenseMatrix::from_row_major(t, t, psd)?);
    }
    let denom = (mean_diag - min_ev) + target_ratio * (max_ev - mean_diag);
    let delta = if denom > 1e-30 { ((target_ratio * max_ev - min_ev) / denom).clamp(0.005, 0.25) } else { 0.05 };
    for i in 0..t {
        for j in 0..t {
            psd[i * t + j] *= 1.0 - delta;
            if i == j {
                psd[i * t + j] += delta * mean_diag;
            }
        }
    }
    symmetrize_dense(&DenseMatrix::from_row_major(t, t, psd)?)
}

pub(in crate::anova) type ReducedDesignOut = (DenseMatrix, Vec<(String, usize, usize)>, Vec<Option<usize>>, usize);

pub(in crate::anova) fn unstructured_reduce_design(x: &DenseMatrix, terms: &[(&str, usize, usize)]) -> Result<ReducedDesignOut, StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    for &(_, start, len) in terms {
        if start > p || start.checked_add(len).is_none_or(|end| end > p) {
            return Err(StatsError::InvalidParameter { name: "terms", value: start as f64 });
        }
    }
    if n == 0 || p == 0 {
        return Err(StatsError::EmptyInput);
    }
    let qr = qr_col_pivoted(x)?;
    if qr.rank() == p {
        let kept_terms = terms.iter().map(|&(name, s, l)| (name.to_owned(), s, l)).collect();
        let col_map = (0..p).map(Some).collect();
        return Ok((x.clone(), kept_terms, col_map, p));
    }

    // Sequential re-orthogonalized Modified Gram-Schmidt preserves term column ordering while
    // dropping structurally aliased columns (e.g., when a factor level/timepoint is all missing).
    let mut keep = vec![false; p];
    let mut q_cols: Vec<Vec<f64>> = Vec::with_capacity(p);
    for (j, slot) in keep.iter_mut().enumerate() {
        let mut w: Vec<f64> = (0..n).map(|r| x.get(r, j).unwrap_or(0.0)).collect();
        let norm0 = w.iter().map(|&v| v * v).sum::<f64>().sqrt();
        if norm0 <= 1e-14 {
            continue;
        }
        for _ in 0..2 {
            for qk in &q_cols {
                let dot: f64 = w.iter().zip(qk.iter()).map(|(&a, &b)| a * b).sum();
                for (wi, &qi) in w.iter_mut().zip(qk.iter()) {
                    *wi -= dot * qi;
                }
            }
        }
        let norm_rem = w.iter().map(|&v| v * v).sum::<f64>().sqrt();
        if norm_rem > 1e-10 * norm0 {
            *slot = true;
            let inv = 1.0 / norm_rem;
            for wi in &mut w {
                *wi *= inv;
            }
            q_cols.push(w);
        }
    }

    let mut col_map = vec![None; p];
    let mut p_eff = 0_usize;
    for (j, &is_kept) in keep.iter().enumerate() {
        if is_kept {
            col_map[j] = Some(p_eff);
            p_eff += 1;
        }
    }
    if p_eff == 0 {
        return Err(StatsError::EmptyInput);
    }

    let mut x_data = Vec::with_capacity(n * p_eff);
    for r in 0..n {
        for (j, &is_kept) in keep.iter().enumerate() {
            if is_kept {
                x_data.push(x.get(r, j).unwrap_or(0.0));
            }
        }
    }
    let x_red = DenseMatrix::from_row_major(n, p_eff, x_data)?;
    let mut reduced_terms = Vec::with_capacity(terms.len());
    for &(name, start, len) in terms {
        let s = start.min(p);
        let end = s.saturating_add(len).min(p);
        let mapped: Vec<usize> = col_map[s..end].iter().filter_map(|&m| m).collect();
        let new_start = mapped.first().copied().unwrap_or(0);
        reduced_terms.push((name.to_owned(), new_start, mapped.len()));
    }
    Ok((x_red, reduced_terms, col_map, p_eff))
}

pub(in crate::anova) fn unstructured_prepare_clusters(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
) -> Result<UnstructuredPrepared, StatsError> {
    let n = x.nrows();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if cluster_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: cluster_ids.len() }));
    }
    if timepoint_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: timepoint_ids.len() }));
    }
    if !x.all_finite() {
        return Err(StatsError::Math(curvebook_math::MathError::DomainError { name: "x", value: f64::NAN }));
    }
    validate_finite_slice(y)?;
    let n_declared = timepoint_labels.len();
    if n_declared == 0 {
        return Err(StatsError::EmptyInput);
    }

    let mut tp_seen = vec![false; n_declared];
    for &tp in timepoint_ids {
        if tp >= n_declared {
            return Err(StatsError::InvalidParameter { name: "timepoint_id", value: tp as f64 });
        }
        tp_seen[tp] = true;
    }
    let mut active_tp_indices = Vec::new();
    let mut active_labels = Vec::new();
    let mut tp_remap = vec![None; n_declared];
    for (orig_tp, &seen) in tp_seen.iter().enumerate() {
        if seen {
            tp_remap[orig_tp] = Some(active_tp_indices.len());
            active_tp_indices.push(orig_tp);
            active_labels.push(timepoint_labels[orig_tp].clone());
        }
    }
    if active_tp_indices.is_empty() {
        return Err(StatsError::EmptyInput);
    }

    let (x_red, reduced_terms, col_map, p_eff) = unstructured_reduce_design(x, terms)?;
    if n <= p_eff {
        return Err(StatsError::InsufficientSampleSize { context: "unstructured RM REML", required: p_eff + 1, actual: n });
    }

    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let mut grouped = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (row_idx, &cid) in cluster_ids.iter().enumerate() {
        grouped.entry(cid).or_default().push(row_idx);
    }
    let mut blocks = Vec::with_capacity(grouped.len());
    for mut rows in grouped.into_values() {
        rows.sort_by_key(|&r| timepoint_ids[r]);
        let m = rows.len();
        let mut tps = Vec::with_capacity(m);
        let mut x_c_data = Vec::with_capacity(m * p_eff);
        let mut y_c = Vec::with_capacity(m);
        for &r in &rows {
            let compact = tp_remap[timepoint_ids[r]].ok_or(StatsError::EmptyInput)?;
            if tps.last() == Some(&compact) {
                return Err(StatsError::InvalidParameter { name: "timepoint_id", value: timepoint_ids[r] as f64 });
            }
            tps.push(compact);
            y_c.push(y_scaled[r]);
            for c in 0..p_eff {
                x_c_data.push(x_red.get(r, c).unwrap_or(0.0));
            }
        }
        let x_c = DenseMatrix::from_row_major(m, p_eff, x_c_data)?;
        blocks.push(UnstructuredClusterBlock { x_c, y_c, tps });
    }

    Ok(UnstructuredPrepared { blocks, reduced_terms, col_map, active_tp_indices, active_labels, p_eff, n_obs: n, y_scale })
}

pub(in crate::anova) fn unstructured_eval_state(
    sigma: &DenseMatrix,
    blocks: &[UnstructuredClusterBlock],
    p_eff: usize,
) -> Result<UnstructuredEval, StatsError> {
    let t_obs = sigma.nrows();
    let mut sum_ln_det_v = 0.0_f64;
    let mut xtvx_data = vec![0.0_f64; p_eff * p_eff];
    let mut xtvy = vec![0.0_f64; p_eff];
    let mut block_cache = Vec::with_capacity(blocks.len());

    for block in blocks {
        let m = block.tps.len();
        let mut v_data = Vec::with_capacity(m * m);
        for &ti in &block.tps {
            for &tj in &block.tps {
                v_data.push(sigma.get(ti, tj).unwrap_or(0.0));
            }
        }
        let chol_v = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(m, m, v_data)?)?)?;
        sum_ln_det_v += chol_v.ln_det();
        let w_c = chol_v.inverse()?;
        let w_c_x_c = w_c.matmul(&block.x_c)?;
        let w_c_y_c = w_c.matvec(&block.y_c)?;
        for a in 0..m {
            for c1 in 0..p_eff {
                let x_a1 = block.x_c.get(a, c1).unwrap_or(0.0);
                xtvy[c1] += x_a1 * w_c_y_c[a];
                for c2 in c1..p_eff {
                    xtvx_data[c1 * p_eff + c2] += x_a1 * w_c_x_c.get(a, c2).unwrap_or(0.0);
                }
            }
        }
        block_cache.push((w_c, w_c_x_c));
    }
    for c1 in 0..p_eff {
        for c2 in 0..c1 {
            xtvx_data[c1 * p_eff + c2] = xtvx_data[c2 * p_eff + c1];
        }
    }

    let chol_x = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, xtvx_data)?)?)?;
    let ln_det_xtvx = chol_x.ln_det();
    let phi = chol_x.inverse()?;
    let beta = chol_x.solve(&xtvy)?;

    let mut quad = 0.0_f64;
    let mut s1 = vec![0.0_f64; t_obs * t_obs];
    let mut s2 = vec![0.0_f64; t_obs * t_obs];
    let mut s3 = vec![0.0_f64; t_obs * t_obs];

    for (block, (w_c, w_c_x_c)) in blocks.iter().zip(block_cache.iter()) {
        let m = block.tps.len();
        let x_beta = block.x_c.matvec(&beta)?;
        let r_c: Vec<f64> = block.y_c.iter().zip(x_beta.iter()).map(|(&yv, &xb)| yv - xb).collect();
        let w_r_c = w_c.matvec(&r_c)?;
        quad += r_c.iter().zip(w_r_c.iter()).map(|(&r, &wr)| r * wr).sum::<f64>();
        let u_phi = w_c_x_c.matmul(&phi)?;

        for a in 0..m {
            let ta = block.tps[a];
            for b in a..m {
                let tb = block.tps[b];
                let w_ab = w_c.get(a, b).unwrap_or(0.0);
                let s3_ab = w_r_c[a] * w_r_c[b];
                let mut s2_ab = 0.0_f64;
                for c_idx in 0..p_eff {
                    s2_ab += u_phi.get(a, c_idx).unwrap_or(0.0) * w_c_x_c.get(b, c_idx).unwrap_or(0.0);
                }
                s1[ta * t_obs + tb] += w_ab;
                s2[ta * t_obs + tb] += s2_ab;
                s3[ta * t_obs + tb] += s3_ab;
                if ta != tb {
                    s1[tb * t_obs + ta] += w_ab;
                    s2[tb * t_obs + ta] += s2_ab;
                    s3[tb * t_obs + ta] += s3_ab;
                }
            }
        }
    }

    let g_data: Vec<f64> = (0..(t_obs * t_obs)).map(|idx| 0.5 * (s3[idx] - s1[idx] + s2[idx])).collect();
    let g_sigma = symmetrize_dense(&DenseMatrix::from_row_major(t_obs, t_obs, g_data)?)?;
    let ll = -0.5 * (sum_ln_det_v + ln_det_xtvx + quad);
    Ok(UnstructuredEval { ll, beta, phi, g_sigma })
}

pub(in crate::anova) fn unstructured_ols_pairwise_init(
    blocks: &[UnstructuredClusterBlock],
    p_eff: usize,
    t_obs: usize,
    n_obs: usize,
) -> Result<(DenseMatrix, f64, f64), StatsError> {
    let mut xtx_data = vec![0.0_f64; p_eff * p_eff];
    let mut xty = vec![0.0_f64; p_eff];
    for block in blocks {
        let m = block.tps.len();
        for a in 0..m {
            for c1 in 0..p_eff {
                let x_a1 = block.x_c.get(a, c1).unwrap_or(0.0);
                xty[c1] += x_a1 * block.y_c[a];
                for c2 in c1..p_eff {
                    xtx_data[c1 * p_eff + c2] += x_a1 * block.x_c.get(a, c2).unwrap_or(0.0);
                }
            }
        }
    }
    for c1 in 0..p_eff {
        for c2 in 0..c1 {
            xtx_data[c1 * p_eff + c2] = xtx_data[c2 * p_eff + c1];
        }
    }
    let chol = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, xtx_data)?)?)?;
    let beta_ols = chol.solve(&xty)?;

    let mut rss = 0.0_f64;
    let mut sum_prod = vec![0.0_f64; t_obs * t_obs];
    let mut counts = vec![0_usize; t_obs * t_obs];
    let df_ratio = (n_obs as f64) / ((n_obs - p_eff).max(1) as f64);

    for block in blocks {
        let xb = block.x_c.matvec(&beta_ols)?;
        let res: Vec<f64> = block.y_c.iter().zip(xb.iter()).map(|(&yv, &bv)| yv - bv).collect();
        rss += res.iter().map(|&r| r * r).sum::<f64>();
        for (a, &ta) in block.tps.iter().enumerate() {
            for (b, &tb) in block.tps.iter().enumerate() {
                sum_prod[ta * t_obs + tb] += res[a] * res[b];
                counts[ta * t_obs + tb] += 1;
            }
        }
    }
    let ols_var = (rss / ((n_obs - p_eff).max(1) as f64)).max(1e-8);
    let mut cov0 = vec![0.0_f64; t_obs * t_obs];
    for i in 0..t_obs {
        let cnt_ii = counts[i * t_obs + i];
        cov0[i * t_obs + i] = if cnt_ii > 0 { (sum_prod[i * t_obs + i] / (cnt_ii as f64) * df_ratio).max(1e-8) } else { ols_var };
    }
    for i in 0..t_obs {
        for j in (i + 1)..t_obs {
            let cnt = counts[i * t_obs + j];
            if cnt > 0 {
                let raw = sum_prod[i * t_obs + j] / (cnt as f64) * df_ratio;
                let max_off = 0.92 * (cov0[i * t_obs + i] * cov0[j * t_obs + j]).sqrt();
                let clamped = raw.clamp(-max_off, max_off);
                cov0[i * t_obs + j] = clamped;
                cov0[j * t_obs + i] = clamped;
            }
        }
    }
    let raw_mat = symmetrize_dense(&DenseMatrix::from_row_major(t_obs, t_obs, cov0)?)?;
    let (min_ev, max_ev, _) = unstructured_eigen_ratio(&raw_mat)?;
    let scatter_ratio = if max_ev > 1e-30 { min_ev / max_ev } else { 0.0 };
    let (spd_init, _) = unstructured_floor_spd(&raw_mat, 0.02, ols_var)?;
    Ok((spd_init, ols_var, scatter_ratio))
}

pub(in crate::anova) fn unstructured_em_warm_start(
    blocks: &[UnstructuredClusterBlock],
    sigma_init: DenseMatrix,
    p_eff: usize,
    ols_var: f64,
    max_iters: usize,
) -> Result<(DenseMatrix, usize), StatsError> {
    let mut sigma = sigma_init;
    let mut st = unstructured_eval_state(&sigma, blocks, p_eff)?;
    let c_scale = 2.0 / (blocks.len().max(1) as f64);
    let mut iters = 0_usize;

    for step_idx in 0..max_iters {
        let grad_norm = st.g_sigma.frobenius_norm();
        if grad_norm < 1e-6 {
            iters = step_idx;
            break;
        }
        let step_mat = scale_dense(&sigma.matmul(&st.g_sigma)?.matmul(&sigma)?, c_scale)?;
        let mut alpha = 1.0_f64;
        let mut accepted = false;
        for _ in 0..8 {
            let scaled_step = scale_dense(&step_mat, alpha)?;
            let cand_raw = symmetrize_dense(&DenseMatrix::from_row_major(
                sigma.nrows(),
                sigma.ncols(),
                sigma.as_slice().iter().zip(scaled_step.as_slice().iter()).map(|(&a, &b)| a + b).collect(),
            )?)?;
            if let Ok((cand_spd, _)) = unstructured_floor_spd(&cand_raw, 1e-7, ols_var)
                && let Ok(cand_st) = unstructured_eval_state(&cand_spd, blocks, p_eff)
                && cand_st.ll >= st.ll - 1e-12
            {
                let improve = (cand_st.ll - st.ll).abs();
                sigma = cand_spd;
                st = cand_st;
                accepted = true;
                if improve <= 1e-12 * (1.0 + st.ll.abs()) {
                    return Ok((sigma, step_idx + 1));
                }
                break;
            }
            alpha *= 0.5;
        }
        iters = step_idx + 1;
        if !accepted {
            break;
        }
    }
    Ok((sigma, iters))
}

pub(in crate::anova) fn unstructured_bfgs_optimize(
    blocks: &[UnstructuredClusterBlock],
    sigma_start: DenseMatrix,
    p_eff: usize,
    max_iters: usize,
) -> Result<(DenseMatrix, bool, usize), StatsError> {
    let t_obs = sigma_start.nrows();
    let is_diag = chol_diag_mask(t_obs);
    let mut eta = log_chol_from_sigma(&sigma_start)?;
    let k_dim = eta.len();
    let (l_init, mut sigma) = sigma_from_log_chol(&eta, t_obs)?;
    let mut st = unstructured_eval_state(&sigma, blocks, p_eff)?;
    let mut g = log_chol_neg_grad(&st.g_sigma, &l_init)?;
    let mut h_inv: Vec<f64> = (0..(k_dim * k_dim)).map(|idx| if idx / k_dim == idx % k_dim { 1.0 } else { 0.0 }).collect();

    for iter in 0..max_iters {
        let g_inf = g.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
        if g_inf <= 1e-8 {
            return Ok((sigma, true, iter));
        }

        let mut dir = vec![0.0_f64; k_dim];
        for i in 0..k_dim {
            let mut sum = 0.0_f64;
            for j in 0..k_dim {
                sum -= h_inv[i * k_dim + j] * g[j];
            }
            dir[i] = sum;
        }
        let mut g_dot_d: f64 = g.iter().zip(dir.iter()).map(|(&gi, &di)| gi * di).sum();
        if !g_dot_d.is_finite() || g_dot_d >= -1e-16 * g_inf {
            for (idx, val) in h_inv.iter_mut().enumerate() {
                *val = if idx / k_dim == idx % k_dim { 1.0 } else { 0.0 };
            }
            dir = g.iter().map(|&gi| -gi).collect();
            g_dot_d = -g.iter().map(|&gi| gi * gi).sum::<f64>();
        }

        let mut alpha = 1.0_f64;
        let mut accepted = None;
        for _ in 0..24 {
            let mut eta_cand = vec![0.0_f64; k_dim];
            for i in 0..k_dim {
                let val = eta[i] + alpha * dir[i];
                eta_cand[i] = if is_diag[i] { val.clamp(LOG_CHOL_DIAG_MIN, LOG_CHOL_DIAG_MAX) } else { val };
            }
            if let Ok((lc, sig_c)) = sigma_from_log_chol(&eta_cand, t_obs)
                && let Ok(st_c) = unstructured_eval_state(&sig_c, blocks, p_eff)
                && st_c.ll.is_finite()
                && st_c.ll >= st.ll - 1e-4 * alpha * g_dot_d
            {
                accepted = Some((eta_cand, lc, sig_c, st_c));
                break;
            }
            alpha *= 0.5;
        }
        let Some((eta_next, l_next, sigma_next, st_next)) = accepted else {
            return Ok((sigma, g_inf <= 1e-4, iter));
        };

        let s: Vec<f64> = eta_next.iter().zip(eta.iter()).map(|(&a, &b)| a - b).collect();
        let step_inf = s.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
        let g_next = log_chol_neg_grad(&st_next.g_sigma, &l_next)?;
        let y_vec: Vec<f64> = g_next.iter().zip(g.iter()).map(|(&a, &b)| a - b).collect();
        let sy: f64 = s.iter().zip(y_vec.iter()).map(|(&si, &yi)| si * yi).sum();
        let yy: f64 = y_vec.iter().map(|&yi| yi * yi).sum();
        let ss: f64 = s.iter().map(|&si| si * si).sum();
        if iter == 0 && sy > 1e-14 && yy > 1e-14 {
            let gamma = sy / yy;
            for (idx, val) in h_inv.iter_mut().enumerate() {
                *val = if idx / k_dim == idx % k_dim { gamma } else { 0.0 };
            }
        }
        if sy > 1e-12 * (ss * yy).sqrt() {
            let rho = 1.0 / sy;
            let mut hy = vec![0.0_f64; k_dim];
            for i in 0..k_dim {
                hy[i] = (0..k_dim).map(|j| h_inv[i * k_dim + j] * y_vec[j]).sum();
            }
            let yhy: f64 = y_vec.iter().zip(hy.iter()).map(|(&yi, &hyi)| yi * hyi).sum();
            let coef_ss = rho + rho * rho * yhy;
            for i in 0..k_dim {
                for j in i..k_dim {
                    let next_ij = h_inv[i * k_dim + j] + coef_ss * s[i] * s[j] - rho * (hy[i] * s[j] + s[i] * hy[j]);
                    h_inv[i * k_dim + j] = next_ij;
                    h_inv[j * k_dim + i] = next_ij;
                }
            }
        }

        eta = eta_next;
        sigma = sigma_next;
        st = st_next;
        g = g_next;

        if step_inf <= 1e-10 && g_inf <= 1e-6 {
            return Ok((sigma, true, iter + 1));
        }
        if is_diag.iter().enumerate().any(|(i, &d)| d && eta[i] <= LOG_CHOL_DIAG_MIN + 1.0) {
            let (min_ev, max_ev, _) = unstructured_eigen_ratio(&sigma)?;
            if max_ev <= 0.0 || min_ev / max_ev <= 1e-6 {
                return Ok((sigma, false, iter + 1));
            }
        }
    }

    let g_inf = g.iter().copied().fold(0.0_f64, |acc, v| acc.max(v.abs()));
    Ok((sigma, g_inf <= 1e-4, max_iters))
}

pub(in crate::anova) struct UnstructuredBlockFisher<'a> {
    pub(in crate::anova) w_tilde: Vec<f64>,
    pub(in crate::anova) u_rows: Vec<Vec<f64>>,
    pub(in crate::anova) pair_ab: &'a [(usize, usize)],
}

pub(in crate::anova) fn unstructured_accumulate_fisher(
    blk: &UnstructuredBlockFisher<'_>,
    t_obs: usize,
    p_eff: usize,
    m_data: &mut [Vec<f64>],
    q_data: &mut [Vec<f64>],
    tr_ww: &mut [f64],
) {
    let k_dim = blk.pair_ab.len();
    let row_contrib = |k_idx: usize| -> [(usize, usize); 2] {
        let (a, b) = blk.pair_ab[k_idx];
        [(a, b), (b, a)]
    };

    for (k_idx, &(a, b)) in blk.pair_ab.iter().enumerate() {
        let ua = &blk.u_rows[a];
        let ub = &blk.u_rows[b];
        let md = &mut m_data[k_idx];
        if a == b {
            for r in 0..p_eff {
                for c in 0..p_eff {
                    md[r * p_eff + c] += ua[r] * ua[c];
                }
            }
        } else {
            for r in 0..p_eff {
                for c in 0..p_eff {
                    md[r * p_eff + c] += ua[r] * ub[c] + ub[r] * ua[c];
                }
            }
        }

        let pairs_k = row_contrib(k_idx);
        let len_k = if a == b { 1 } else { 2 };
        for l_idx in k_idx..k_dim {
            let (cp, dp) = blk.pair_ab[l_idx];
            let pairs_l = row_contrib(l_idx);
            let len_l = if cp == dp { 1 } else { 2 };
            let qd = &mut q_data[k_idx * k_dim + l_idx];
            let mut tr_acc = 0.0_f64;
            for &(r1, s1) in &pairs_k[..len_k] {
                let v1 = &blk.u_rows[s1];
                for &(r2, s2) in &pairs_l[..len_l] {
                    let w12 = blk.w_tilde[r1 * t_obs + r2];
                    if w12 != 0.0 {
                        let v2 = &blk.u_rows[s2];
                        for r in 0..p_eff {
                            let wr = w12 * v1[r];
                            for c in 0..p_eff {
                                qd[r * p_eff + c] += wr * v2[c];
                            }
                        }
                    }
                    tr_acc += blk.w_tilde[r1 * t_obs + s2] * blk.w_tilde[s1 * t_obs + r2];
                }
            }
            tr_ww[k_idx * k_dim + l_idx] += tr_acc;
        }
    }
}

pub(in crate::anova) fn unstructured_build_fisher_mats(
    blocks: &[UnstructuredClusterBlock],
    sigma: &DenseMatrix,
    phi: &DenseMatrix,
    p_eff: usize,
) -> Result<OwnedRemlFisherMats, StatsError> {
    let t_obs = sigma.nrows();
    let k_dim = t_obs * (t_obs + 1) / 2;
    let mut pair_ab = Vec::with_capacity(k_dim);
    for a in 0..t_obs {
        for b in 0..=a {
            pair_ab.push((a, b));
        }
    }
    let mut m_data = vec![vec![0.0_f64; p_eff * p_eff]; k_dim];
    let mut q_data = vec![vec![0.0_f64; p_eff * p_eff]; k_dim * k_dim];
    let mut tr_ww = vec![0.0_f64; k_dim * k_dim];

    for block in blocks {
        let m = block.tps.len();
        let mut v_data = Vec::with_capacity(m * m);
        for &ti in &block.tps {
            for &tj in &block.tps {
                v_data.push(sigma.get(ti, tj).unwrap_or(0.0));
            }
        }
        let chol_v = cholesky(&symmetrize_dense(&DenseMatrix::from_row_major(m, m, v_data)?)?)?;
        let w_c = chol_v.inverse()?;
        let w_c_x_c = w_c.matmul(&block.x_c)?;

        let mut w_tilde = vec![0.0_f64; t_obs * t_obs];
        let mut u_rows = vec![vec![0.0_f64; p_eff]; t_obs];
        for (a, &ta) in block.tps.iter().enumerate() {
            for c in 0..p_eff {
                u_rows[ta][c] = w_c_x_c.get(a, c).unwrap_or(0.0);
            }
            for (b, &tb) in block.tps.iter().enumerate() {
                w_tilde[ta * t_obs + tb] = w_c.get(a, b).unwrap_or(0.0);
            }
        }
        let blk = UnstructuredBlockFisher { w_tilde, u_rows, pair_ab: &pair_ab };
        unstructured_accumulate_fisher(&blk, t_obs, p_eff, &mut m_data, &mut q_data, &mut tr_ww);
    }

    let mut m_mats = Vec::with_capacity(k_dim);
    let mut phi_m = Vec::with_capacity(k_dim);
    for data in m_data {
        let m_mat = symmetrize_dense(&DenseMatrix::from_row_major(p_eff, p_eff, data)?)?;
        phi_m.push(phi.matmul(&m_mat)?);
        m_mats.push(m_mat);
    }

    let mut q_mats = vec![vec![DenseMatrix::zeros(p_eff, p_eff)?; k_dim]; k_dim];
    let mut ie_data = vec![0.0_f64; k_dim * k_dim];
    for r in 0..k_dim {
        for s in r..k_dim {
            let q_rs = DenseMatrix::from_row_major(p_eff, p_eff, std::mem::take(&mut q_data[r * k_dim + s]))?;
            let ie_rs = 0.5 * (tr_ww[r * k_dim + s] - 2.0 * trace_product(phi, &q_rs) + trace_product(&phi_m[r], &phi_m[s]));
            ie_data[r * k_dim + s] = ie_rs;
            ie_data[s * k_dim + r] = ie_rs;
            if r == s {
                q_mats[r][s] = symmetrize_dense(&q_rs)?;
            } else {
                q_mats[s][r] = q_rs.transpose();
                q_mats[r][s] = q_rs;
            }
        }
    }
    let info_e = symmetrize_dense(&DenseMatrix::from_row_major(k_dim, k_dim, ie_data)?)?;
    Ok(OwnedRemlFisherMats { info_e, m_mats, q_mats })
}

pub(in crate::anova) fn unstructured_corr_from_cov(cov: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let t = cov.len();
    let mut corr = vec![vec![0.0_f64; t]; t];
    for i in 0..t {
        let sd_i = cov[i][i].max(0.0).sqrt();
        corr[i][i] = if cov[i][i] > 0.0 { 1.0 } else { 0.0 };
        for j in (i + 1)..t {
            let sd_j = cov[j][j].max(0.0).sqrt();
            let val = if sd_i > 0.0 && sd_j > 0.0 { (cov[i][j] / (sd_i * sd_j)).clamp(-1.0, 1.0) } else { 0.0 };
            corr[i][j] = val;
            corr[j][i] = val;
        }
    }
    corr
}

/// Fit an unstructured `t x t` repeated-measures residual covariance matrix
/// (`\Sigma = L L^T`, `t(t + 1)/2` variance-covariance parameters) via log-Cholesky REML
/// (Pinheiro & Bates, 1996; Lindstrom & Bates, 1988) with EM warm-start, BFGS quasi-Newton
/// refinement, and exact Satterthwaite (1946) / Fai & Cornelius (1996) and Kenward & Roger (1997)
/// inference using the linear `vech(\Sigma)` parameterization (`\partial^2 V / \partial\theta_r\partial\theta_s = 0`).
///
/// Engages documented positive-definite diagonal/shrinkage fallbacks for single-subject designs,
/// rank-deficient small samples, all-missing timepoints (compacted from active timepoints alongside
/// rank-reduced fixed effects), or singular covariance boundaries.
pub fn fit_unstructured_rm_reml(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
) -> Result<UnstructuredRemlResult, StatsError> {
    fit_unstructured_rm_reml_with_method_and_core(x, y, cluster_ids, timepoint_ids, timepoint_labels, terms, MixedModelDfMethod::KenwardRoger)
        .map(|(res, _)| res)
}

/// Variant of [`fit_unstructured_rm_reml`] selecting Kenward–Roger vs Fai–Cornelius Satterthwaite fixed-effect inference.
pub fn fit_unstructured_rm_reml_with_df_method(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<UnstructuredRemlResult, StatsError> {
    fit_unstructured_rm_reml_with_method_and_core(x, y, cluster_ids, timepoint_ids, timepoint_labels, terms, df_method).map(|(res, _)| res)
}

pub(in crate::anova) fn fit_unstructured_rm_reml_with_method_and_core(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    timepoint_ids: &[usize],
    timepoint_labels: &[String],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<(UnstructuredRemlResult, RemlContrastCore), StatsError> {
    let prep = unstructured_prepare_clusters(x, y, cluster_ids, timepoint_ids, timepoint_labels, terms)?;
    let t_obs = prep.active_tp_indices.len();
    let n_clusters = prep.blocks.len();
    let n_minus_p = (prep.n_obs - prep.p_eff) as f64;
    let k_params = t_obs * (t_obs + 1) / 2;

    let (sigma_init, ols_var, scatter_ratio) = unstructured_ols_pairwise_init(&prep.blocks, prep.p_eff, t_obs, prep.n_obs)?;
    let (sigma_scaled, converged, iterations, fallback_reason) = if n_clusters == 1 {
        let diag = scale_dense(&DenseMatrix::identity(t_obs)?, ols_var.max(1e-8))?;
        (diag, true, 0, Some(UnstructuredFallbackReason::SingleSubject))
    } else if n_clusters < t_obs || (prep.n_obs - prep.p_eff) < k_params {
        let shr = unstructured_shrink_spd(&sigma_init, 1e-3, ols_var)?;
        (shr, true, 0, Some(UnstructuredFallbackReason::SmallSampleRankDeficient))
    } else {
        let (em_sig, em_iters) = unstructured_em_warm_start(&prep.blocks, sigma_init, prep.p_eff, ols_var, 20)?;
        let (bfgs_sig, bfgs_conv, bfgs_iters) = unstructured_bfgs_optimize(&prep.blocks, em_sig, prep.p_eff, 220)?;
        let total_iters = em_iters + bfgs_iters;
        let (min_ev, max_ev, _) = unstructured_eigen_ratio(&bfgs_sig)?;
        let cov_ratio = if max_ev > 1e-30 { min_ev / max_ev } else { 0.0 };
        if cov_ratio <= 1e-6 || scatter_ratio <= 1e-8 {
            let reason = if n_clusters <= t_obs || (prep.n_obs - prep.p_eff) <= k_params {
                UnstructuredFallbackReason::SmallSampleRankDeficient
            } else {
                UnstructuredFallbackReason::SingularCovarianceEstimate
            };
            let shr = unstructured_shrink_spd(&bfgs_sig, 1e-3, ols_var)?;
            (shr, bfgs_conv, total_iters, Some(reason))
        } else if !bfgs_conv {
            (bfgs_sig, false, total_iters, Some(UnstructuredFallbackReason::IterationLimitReached))
        } else {
            (bfgs_sig, true, total_iters, None)
        }
    };

    let st_final = unstructured_eval_state(&sigma_scaled, &prep.blocks, prep.p_eff)?;
    let fisher = unstructured_build_fisher_mats(&prep.blocks, &sigma_scaled, &st_final.phi, prep.p_eff)?;
    let term_refs: Vec<(&str, usize, usize)> = prep.reduced_terms.iter().map(|(n, s, l)| (n.as_str(), *s, *l)).collect();
    let mean_diag_scaled: f64 = (0..t_obs).map(|i| sigma_scaled.get(i, i).unwrap_or(1.0)).sum::<f64>() / (t_obs as f64);

    let ((mut fixed_terms, satterthwaite_df, kenward_roger_df), mut core) =
        eval_reml_wald_terms_for_df(&st_final.phi, &st_final.beta, mean_diag_scaled, &fisher, &term_refs, n_minus_p, df_method)?;
    core.y_scale = prep.y_scale;
    core.col_map = prep.col_map;

    let var_scale = prep.y_scale * prep.y_scale;
    let mean_diagonal_variance = mean_diag_scaled * var_scale;
    for term in &mut fixed_terms {
        if let Some(f_val) = term.f_statistic {
            let ms = f_val * mean_diagonal_variance;
            term.ms = ms;
            term.ss = ms * term.df;
        }
    }

    let mut covariance_matrix = vec![vec![0.0_f64; t_obs]; t_obs];
    for (i, row) in covariance_matrix.iter_mut().enumerate() {
        for (j, slot) in row.iter_mut().enumerate() {
            *slot = sigma_scaled.get(i, j).unwrap_or(0.0) * var_scale;
        }
    }
    let correlation_matrix = unstructured_corr_from_cov(&covariance_matrix);
    let reml_log_lik = st_final.ll - n_minus_p * prep.y_scale.ln();

    let res = UnstructuredRemlResult {
        timepoint_labels: prep.active_labels,
        active_timepoint_indices: prep.active_tp_indices,
        covariance_matrix,
        correlation_matrix,
        mean_diagonal_variance,
        reml_log_lik,
        fixed_terms,
        satterthwaite_df,
        kenward_roger_df,
        converged,
        iterations,
        fallback_reason,
    };
    Ok((res, core))
}
