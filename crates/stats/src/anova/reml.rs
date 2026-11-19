//! Mixed-effects REML for random-intercept and multiple-variance-component models, with
//! Satterthwaite and Kenward–Roger denominator degrees of freedom for fixed-effect Wald tests.
//!
//! References:
//! - Patterson, H. D., & Thompson, R. (1971). Recovery of inter-block information when block sizes
//!   are unequal. *Biometrika* 58(3):545–554. doi:10.1093/biomet/58.3.545
//! - Gilmour, A. R., Thompson, R., & Cullis, B. R. (1995). Average Information REML: an efficient
//!   algorithm for variance parameter estimation in linear mixed models. *Biometrics*
//!   51(4):1440–1450. doi:10.2307/2533274
//! - Self, S. G., & Liang, K.-Y. (1987). Asymptotic properties of maximum likelihood estimators and
//!   likelihood ratio tests under nonstandard conditions. *Journal of the American Statistical
//!   Association* 82(398):605–610. doi:10.1080/01621459.1987.10478472
//! - Satterthwaite, F. E. (1946). An approximate distribution of estimates of variance components.
//!   *Biometrics Bulletin* 2(6):110–114. doi:10.2307/3002019
//! - Giesbrecht, F. G., & Burns, J. C. (1985). Two-stage analysis based on a mixed model:
//!   large-sample asymptotic theory and small-sample simulation results. *Biometrics*
//!   41(2):477–486. doi:10.2307/2530872
//! - Kenward, M. G., & Roger, J. H. (1997). Small sample inference for fixed effects from
//!   restricted maximum likelihood. *Biometrics* 53(3):983–997. doi:10.2307/2533558
//! - Fai, A. H.-T., & Cornelius, P. L. (1996). Approximate F-tests of multiple degree of freedom
//!   hypotheses in generalized least squares analyses of unbalanced split-plot experiments.
//!   *Journal of Statistical Computation and Simulation* 54(4):363–378. doi:10.1080/00949659608811740

use curvebook_doc::MixedModelDfMethod;
use curvebook_math::dist::{chi2_sf, f_sf};
use curvebook_math::linalg::{DenseMatrix, cholesky, solve_linear_least_squares, sym_eigen};
use curvebook_math::sum::{NeumaierSum, SummaryMoments};

use crate::anova::results::{FactorialAnovaTerm, MixedRemlResult, MultiVcRemlResult};
use crate::common::validate_finite_slice;
use crate::error::StatsError;

/// Upper bound on $u = \ln(1 + \sigma_u^2 / \sigma_e^2)$ for random-intercept REML profile search
/// ($\tau \le e^{36} - 1 \approx 4.3 \times 10^{15}$, supporting nested/clustered designs down to
/// near-zero technical replicate error without truncating high ICC regimes).
const REML_LOG1P_TAU_MAX: f64 = 36.0;

pub(in crate::anova) struct RiClusterStats {
    pub(in crate::anova) m_c: f64,
    pub(in crate::anova) y_bar: f64,
    pub(in crate::anova) x_bar: Vec<f64>,
    pub(in crate::anova) xy_within: Vec<f64>,
    pub(in crate::anova) xx_within: Vec<f64>,
    pub(in crate::anova) within_rows: Vec<(Vec<f64>, f64)>,
}

pub(in crate::anova) struct OwnedRemlFisherMats {
    pub(in crate::anova) info_e: DenseMatrix,
    pub(in crate::anova) m_mats: Vec<DenseMatrix>,
    pub(in crate::anova) q_mats: Vec<Vec<DenseMatrix>>,
}

pub(in crate::anova) struct KrTermContext<'a> {
    pub(in crate::anova) phi: &'a DenseMatrix,
    pub(in crate::anova) phi_a: &'a DenseMatrix,
    pub(in crate::anova) beta: &'a [f64],
    pub(in crate::anova) sigma2_e: f64,
    pub(in crate::anova) w_info_inv: &'a DenseMatrix,
    pub(in crate::anova) phi_m_phi: &'a [DenseMatrix],
    pub(in crate::anova) n_minus_p: f64,
}

pub(in crate::anova) struct MultiVcEvalState {
    pub(in crate::anova) ll: f64,
    pub(in crate::anova) score: Vec<f64>,
    pub(in crate::anova) ai: DenseMatrix,
    pub(in crate::anova) beta: Vec<f64>,
    pub(in crate::anova) phi: DenseMatrix,
    pub(in crate::anova) vi: DenseMatrix,
    pub(in crate::anova) p_mat: DenseMatrix,
    pub(in crate::anova) vir: Vec<f64>,
    pub(in crate::anova) quad: f64,
}

pub(in crate::anova) fn symmetrize_dense(m: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let nr = m.nrows();
    let nc = m.ncols();
    if nr != nc {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: nr, actual: nc }));
    }
    let mut data = vec![0.0_f64; nr * nr];
    for i in 0..nr {
        data[i * nr + i] = m.get(i, i).unwrap_or(0.0);
        for j in (i + 1)..nr {
            let v = 0.5 * (m.get(i, j).unwrap_or(0.0) + m.get(j, i).unwrap_or(0.0));
            data[i * nr + j] = v;
            data[j * nr + i] = v;
        }
    }
    Ok(DenseMatrix::from_row_major(nr, nr, data)?)
}

pub(in crate::anova) fn scale_dense(m: &DenseMatrix, factor: f64) -> Result<DenseMatrix, StatsError> {
    let data: Vec<f64> = m.as_slice().iter().map(|&v| v * factor).collect();
    Ok(DenseMatrix::from_row_major(m.nrows(), m.ncols(), data)?)
}

pub(in crate::anova) fn dense_sub(a: &DenseMatrix, b: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    if a.nrows() != b.nrows() || a.ncols() != b.ncols() {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: a.nrows(), actual: b.nrows() }));
    }
    let data: Vec<f64> = a.as_slice().iter().zip(b.as_slice().iter()).map(|(&x, &y)| x - y).collect();
    Ok(DenseMatrix::from_row_major(a.nrows(), a.ncols(), data)?)
}

pub(in crate::anova) fn trace_product(a: &DenseMatrix, b: &DenseMatrix) -> f64 {
    let n = a.nrows().min(b.ncols());
    let k = a.ncols().min(b.nrows());
    let mut acc = NeumaierSum::new();
    for i in 0..n {
        for j in 0..k {
            acc.add(a.get(i, j).unwrap_or(0.0) * b.get(j, i).unwrap_or(0.0));
        }
    }
    acc.total()
}

pub(in crate::anova) fn subblock_dense(m: &DenseMatrix, start: usize, len: usize) -> Result<DenseMatrix, StatsError> {
    let mut data = Vec::with_capacity(len * len);
    for r in 0..len {
        for c in 0..len {
            data.push(m.get(start + r, start + c).unwrap_or(0.0));
        }
    }
    Ok(DenseMatrix::from_row_major(len, len, data)?)
}

pub(in crate::anova) fn invert_reml_info(info: &DenseMatrix) -> Result<DenseMatrix, StatsError> {
    let sym = symmetrize_dense(info)?;
    let r = sym.nrows();
    // Diagonal equilibration prevents ill-conditioned variance-component scaling (large tau)
    // from triggering false singularity fallback or dropping small Fisher eigenvalues.
    let diag_scale: Vec<f64> = (0..r)
        .map(|i| {
            let v = sym.get(i, i).unwrap_or(0.0);
            if v > 0.0 { v.sqrt() } else { 0.0 }
        })
        .collect();
    let mut scaled_data = vec![0.0_f64; r * r];
    for i in 0..r {
        if diag_scale[i] == 0.0 {
            continue;
        }
        for j in 0..r {
            if diag_scale[j] == 0.0 {
                continue;
            }
            scaled_data[i * r + j] = sym.get(i, j).unwrap_or(0.0) / (diag_scale[i] * diag_scale[j]);
        }
    }
    let scaled = DenseMatrix::from_row_major(r, r, scaled_data)?;
    let scaled_inv = if let Ok(chol) = cholesky(&scaled)
        && let Ok(inv) = chol.inverse()
    {
        inv
    } else {
        let eig = sym_eigen(&scaled)?;
        let max_ev = eig.eigenvalues.iter().copied().fold(0.0_f64, f64::max);
        let tol = 1e-12 * max_ev.max(1e-30);
        let mut inv_data = vec![0.0_f64; r * r];
        for (k, &ev) in eig.eigenvalues.iter().enumerate() {
            if ev <= tol {
                continue;
            }
            let inv_ev = 1.0 / ev;
            for i in 0..r {
                let qi = eig.eigenvectors.get(i, k).unwrap_or(0.0);
                for j in i..r {
                    let qj = eig.eigenvectors.get(j, k).unwrap_or(0.0);
                    let delta = qi * qj * inv_ev;
                    inv_data[i * r + j] += delta;
                    if i != j {
                        inv_data[j * r + i] += delta;
                    }
                }
            }
        }
        DenseMatrix::from_row_major(r, r, inv_data)?
    };

    let mut inv_data = vec![0.0_f64; r * r];
    for i in 0..r {
        if diag_scale[i] == 0.0 {
            continue;
        }
        for j in 0..r {
            if diag_scale[j] == 0.0 {
                continue;
            }
            inv_data[i * r + j] = scaled_inv.get(i, j).unwrap_or(0.0) / (diag_scale[i] * diag_scale[j]);
        }
    }
    Ok(DenseMatrix::from_row_major(r, r, inv_data)?)
}

pub(in crate::anova) fn ri_precompute_clusters(x: &DenseMatrix, y: &[f64], cluster_ids: &[usize]) -> Vec<RiClusterStats> {
    let p = x.ncols();
    let mut grouped = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    for (row_idx, &cid) in cluster_ids.iter().enumerate() {
        grouped.entry(cid).or_default().push(row_idx);
    }
    let mut clusters = Vec::with_capacity(grouped.len());
    for rows in grouped.into_values() {
        let m_c = rows.len() as f64;
        let mut sum_y = NeumaierSum::new();
        let mut sum_x = vec![NeumaierSum::new(); p];
        for &r in &rows {
            sum_y.add(y[r]);
            for j in 0..p {
                sum_x[j].add(x.get(r, j).unwrap_or(0.0));
            }
        }
        let y_bar = sum_y.total() / m_c;
        let x_bar: Vec<f64> = sum_x.iter().map(|s| s.total() / m_c).collect();

        let mut xy_within = vec![0.0_f64; p];
        let mut xx_within = vec![0.0_f64; p * p];
        let mut within_rows = Vec::with_capacity(rows.len());
        for r in rows {
            let dy = y[r] - y_bar;
            let mut dx = Vec::with_capacity(p);
            for j in 0..p {
                dx.push(x.get(r, j).unwrap_or(0.0) - x_bar[j]);
            }
            for j in 0..p {
                let dxj = dx[j];
                xy_within[j] += dxj * dy;
                for k in j..p {
                    xx_within[j * p + k] += dxj * dx[k];
                }
            }
            within_rows.push((dx, dy));
        }
        for j in 0..p {
            for k in 0..j {
                xx_within[j * p + k] = xx_within[k * p + j];
            }
        }
        clusters.push(RiClusterStats { m_c, y_bar, x_bar, xy_within, xx_within, within_rows });
    }
    clusters
}

pub(in crate::anova) type RiProfileEval = (f64, f64, DenseMatrix, Vec<f64>);

pub(in crate::anova) fn ri_eval_profile(clusters: &[RiClusterStats], p: usize, n_minus_p: f64, tau_raw: f64) -> Result<RiProfileEval, StatsError> {
    let tau = tau_raw.max(0.0);
    let mut xtvx = vec![0.0_f64; p * p];
    let mut xtvy = vec![0.0_f64; p];
    let mut ln_det_v = 0.0_f64;

    // Orthogonal within/between decomposition avoids catastrophic cancellation in V_c^{-1} = I - (tau / (1 + m_c * tau)) 1 1^T
    // when cluster means dominate within-cluster spread and tau is very large.
    for cl in clusters {
        let factor = 1.0 + cl.m_c * tau;
        let a_c = 1.0 / factor;
        let w_bet = cl.m_c * a_c;
        ln_det_v += factor.ln();
        for r_row in 0..p {
            xtvy[r_row] += cl.xy_within[r_row] + w_bet * cl.x_bar[r_row] * cl.y_bar;
            for c_col in r_row..p {
                xtvx[r_row * p + c_col] += cl.xx_within[r_row * p + c_col] + w_bet * cl.x_bar[r_row] * cl.x_bar[c_col];
            }
        }
    }
    for r_row in 0..p {
        for c_col in 0..r_row {
            xtvx[r_row * p + c_col] = xtvx[c_col * p + r_row];
        }
    }
    let xtvx_mat = DenseMatrix::from_row_major(p, p, xtvx)?;
    let chol_x = cholesky(&xtvx_mat)?;
    let ln_det_xtvx = chol_x.ln_det();
    let xtvx_inv = chol_x.inverse()?;
    let beta = chol_x.solve(&xtvy)?;

    let mut rss_acc = NeumaierSum::new();
    for cl in clusters {
        let a_c = 1.0 / (1.0 + cl.m_c * tau);
        let xb: f64 = cl.x_bar.iter().zip(beta.iter()).map(|(&x, &b)| x * b).sum();
        let r_bar = cl.y_bar - xb;
        rss_acc.add(cl.m_c * a_c * r_bar * r_bar);
        for (dx, dy) in &cl.within_rows {
            let dxb: f64 = dx.iter().zip(beta.iter()).map(|(&x, &b)| x * b).sum();
            let r_in = *dy - dxb;
            rss_acc.add(r_in * r_in);
        }
    }
    let rss = rss_acc.total().max(1e-30);
    let sigma2_e = rss / n_minus_p;
    let ll = -0.5 * (n_minus_p * sigma2_e.ln() + ln_det_v + ln_det_xtvx + n_minus_p);
    Ok((ll, sigma2_e, xtvx_inv, beta))
}

pub(in crate::anova) fn ri_build_fisher_mats(
    clusters: &[RiClusterStats],
    p: usize,
    n_obs: usize,
    tau: f64,
    sigma2_e: f64,
    phi_rel: &DenseMatrix,
) -> Result<OwnedRemlFisherMats, StatsError> {
    let mut m0 = vec![0.0_f64; p * p];
    let mut m1 = vec![0.0_f64; p * p];
    let mut q00 = vec![0.0_f64; p * p];
    let mut q01 = vec![0.0_f64; p * p];
    let mut q11 = vec![0.0_f64; p * p];
    let mut sum_a2 = 0.0_f64;
    let mut sum_ma2 = 0.0_f64;
    let mut sum_m2a2 = 0.0_f64;

    for cl in clusters {
        let a_c = 1.0 / (1.0 + cl.m_c * tau);
        let a2 = a_c * a_c;
        let a3 = a2 * a_c;
        let w_m0 = cl.m_c * a2;
        let w_m1 = cl.m_c * cl.m_c * a2;
        let w_q00 = cl.m_c * a3;
        let w_q01 = cl.m_c * cl.m_c * a3;
        let w_q11 = w_q01 * cl.m_c;
        sum_a2 += a2;
        sum_ma2 += cl.m_c * a2;
        sum_m2a2 += cl.m_c * cl.m_c * a2;
        for r in 0..p {
            for c in r..p {
                let idx = r * p + c;
                let xx_in = cl.xx_within[idx];
                let xbar_rc = cl.x_bar[r] * cl.x_bar[c];
                m0[idx] += xx_in + w_m0 * xbar_rc;
                m1[idx] += w_m1 * xbar_rc;
                q00[idx] += xx_in + w_q00 * xbar_rc;
                q01[idx] += w_q01 * xbar_rc;
                q11[idx] += w_q11 * xbar_rc;
            }
        }
    }
    for mat in [&mut m0, &mut m1, &mut q00, &mut q01, &mut q11] {
        for r in 0..p {
            for c in 0..r {
                mat[r * p + c] = mat[c * p + r];
            }
        }
    }
    let m0_mat = DenseMatrix::from_row_major(p, p, m0)?;
    let m1_mat = DenseMatrix::from_row_major(p, p, m1)?;
    let q00_mat = DenseMatrix::from_row_major(p, p, q00)?;
    let q01_mat = DenseMatrix::from_row_major(p, p, q01)?;
    let q11_mat = DenseMatrix::from_row_major(p, p, q11)?;

    let tr_v2 = ((n_obs - clusters.len()) as f64) + sum_a2;
    let m_rel = [&m0_mat, &m1_mat];
    let q_rel = [[&q00_mat, &q01_mat], [&q01_mat, &q11_mat]];
    let tr_vg_vg = [[tr_v2, sum_ma2], [sum_ma2, sum_m2a2]];
    let mut ie_data = [0.0_f64; 4];
    for r in 0..2 {
        let pm_r = phi_rel.matmul(m_rel[r])?;
        for s in r..2 {
            let pm_s = phi_rel.matmul(m_rel[s])?;
            let ie_rs = 0.5 * (tr_vg_vg[r][s] - 2.0 * trace_product(phi_rel, q_rel[r][s]) + trace_product(&pm_r, &pm_s));
            ie_data[r * 2 + s] = ie_rs;
            ie_data[s * 2 + r] = ie_rs;
        }
    }
    let inv_s2 = 1.0 / sigma2_e;
    let inv_s4 = inv_s2 * inv_s2;
    let inv_s6 = inv_s4 * inv_s2;
    let info_e = scale_dense(&DenseMatrix::from_row_major(2, 2, ie_data.to_vec())?, inv_s4)?;
    let m_mats = vec![scale_dense(&m0_mat, inv_s4)?, scale_dense(&m1_mat, inv_s4)?];
    let q01_s = scale_dense(&q01_mat, inv_s6)?;
    let q_mats = vec![vec![scale_dense(&q00_mat, inv_s6)?, q01_s.clone()], vec![q01_s, scale_dense(&q11_mat, inv_s6)?]];
    Ok(OwnedRemlFisherMats { info_e, m_mats, q_mats })
}

pub(in crate::anova) fn kr_scale_and_df(qf: f64, a1: f64, a2: f64, nu_sat: f64, n_minus_p: f64) -> (f64, f64) {
    if a2 <= 1e-14 || 1.0 - a2 / qf <= 1e-6 {
        return (1.0, nu_sat);
    }
    let b_kr = (a1 + 6.0 * a2) / (2.0 * qf);
    let g_kr = ((qf + 1.0) * a1 - (qf + 4.0) * a2) / ((qf + 2.0) * a2);
    let den_c = 3.0 * qf + 2.0 * (1.0 - g_kr);
    if den_c.abs() <= 1e-14 {
        return (1.0, nu_sat);
    }
    let c1 = g_kr / den_c;
    let c2 = (qf - g_kr) / den_c;
    let c3 = (qf + 2.0 - g_kr) / den_c;
    let e_star = 1.0 / (1.0 - a2 / qf);
    let v_num = 1.0 + c1 * b_kr;
    let v_den = (1.0 - c2 * b_kr).powi(2) * (1.0 - c3 * b_kr);
    if v_num <= 0.0 || v_den <= 1e-6 {
        return (1.0, nu_sat);
    }
    let v_star = (2.0 / qf) * (v_num / v_den);
    let rho = v_star / (2.0 * e_star * e_star);
    if qf * rho - 1.0 <= 1e-12 {
        return (1.0 / e_star, n_minus_p);
    }
    let nu_kr = (4.0 + (qf + 2.0) / (qf * rho - 1.0)).clamp(1.0, n_minus_p);
    let lambda = if nu_kr > 2.0 { nu_kr / (e_star * (nu_kr - 2.0)) } else { 1.0 / e_star };
    (lambda.max(0.0), nu_kr)
}

pub(in crate::anova) struct KrSatTermPair {
    pub(in crate::anova) kr: FactorialAnovaTerm,
    pub(in crate::anova) sat: FactorialAnovaTerm,
    pub(in crate::anova) nu_sat: f64,
    pub(in crate::anova) nu_kr: f64,
}

pub(in crate::anova) fn build_wald_term(term_name: &str, qf: f64, f_val: f64, nu: f64, sigma2_e: f64) -> Result<FactorialAnovaTerm, StatsError> {
    let f_clean = f_val.max(0.0);
    let p_val = f_sf(f_clean, qf, nu)?;
    let ms = f_clean * sigma2_e;
    let part_eta = if qf * f_clean + nu > 0.0 { Some((qf * f_clean) / (qf * f_clean + nu)) } else { Some(0.0) };
    Ok(FactorialAnovaTerm {
        name: term_name.to_owned(),
        ss: ms * qf,
        df: qf,
        ms,
        f_statistic: Some(f_clean),
        p_value: Some(p_val),
        partial_eta_squared: part_eta,
        eta_squared: None,
    })
}

pub(in crate::anova) fn kr_single_term_df1(ctx: &KrTermContext<'_>, term_name: &str, col: usize) -> Result<KrSatTermPair, StatsError> {
    let v = ctx.phi.get(col, col).unwrap_or(0.0).max(0.0);
    let v_a_raw = ctx.phi_a.get(col, col).unwrap_or(v);
    let v_a = if v_a_raw > 1e-12 * v { v_a_raw } else { v };
    let b = ctx.beta[col];
    let f_from_var = |var: f64| -> f64 {
        if var > 0.0 {
            (b * b) / var
        } else if b != 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    };
    let f_kr = f_from_var(v_a);
    let f_sat = f_from_var(v);
    let mut grad = Vec::with_capacity(ctx.phi_m_phi.len());
    for pmp in ctx.phi_m_phi {
        grad.push(pmp.get(col, col).unwrap_or(0.0));
    }
    let wg = ctx.w_info_inv.matvec(&grad)?;
    let denom: f64 = grad.iter().zip(wg.iter()).map(|(&a, &c)| a * c).sum();
    let nu = if denom > 1e-30 * v * v { (2.0 * v * v / denom).clamp(1.0, ctx.n_minus_p) } else { ctx.n_minus_p };
    let kr = build_wald_term(term_name, 1.0, f_kr, nu, ctx.sigma2_e)?;
    let sat = build_wald_term(term_name, 1.0, f_sat, nu, ctx.sigma2_e)?;
    Ok(KrSatTermPair { kr, sat, nu_sat: nu, nu_kr: nu })
}

pub(in crate::anova) fn kr_multi_term_df(ctx: &KrTermContext<'_>, term_name: &str, start: usize, q: usize) -> Result<KrSatTermPair, StatsError> {
    let qf = q as f64;
    let sigma = symmetrize_dense(&subblock_dense(ctx.phi, start, q)?)?;
    let sigma_a = symmetrize_dense(&subblock_dense(ctx.phi_a, start, q)?)?;
    let beta_sub = &ctx.beta[start..(start + q)];
    let inv_a = match cholesky(&sigma_a).and_then(|c| c.inverse()) {
        Ok(inv) => inv,
        Err(_) => invert_reml_info(&sigma_a)?,
    };
    let inv_b = inv_a.matvec(beta_sub)?;
    let quad: f64 = beta_sub.iter().zip(inv_b.iter()).map(|(&b, &ib)| b * ib).sum();
    let f_raw = (quad / qf).max(0.0);

    // Fai & Cornelius (1996) Satterthwaite df via spectral decomposition of L Phi L^T.
    let eig = sym_eigen(&sigma)?;
    let n_vc = ctx.w_info_inv.nrows();
    let mut nu_modes = Vec::with_capacity(q);
    for m in 0..q {
        let v_m = eig.eigenvalues[m];
        if v_m <= 1e-30 {
            nu_modes.push(ctx.n_minus_p);
            continue;
        }
        let mut u_m = Vec::with_capacity(q);
        for r in 0..q {
            u_m.push(eig.eigenvectors.get(r, m).unwrap_or(0.0));
        }
        let mut grad_m = Vec::with_capacity(n_vc);
        for pmp in ctx.phi_m_phi {
            let sub = subblock_dense(pmp, start, q)?;
            let sv = sub.matvec(&u_m)?;
            grad_m.push(u_m.iter().zip(sv.iter()).map(|(&a, &b)| a * b).sum());
        }
        let wg = ctx.w_info_inv.matvec(&grad_m)?;
        let denom_m: f64 = grad_m.iter().zip(wg.iter()).map(|(&a, &b)| a * b).sum();
        let nu_m = if denom_m > 1e-30 * v_m * v_m { 2.0 * v_m * v_m / denom_m } else { ctx.n_minus_p };
        nu_modes.push(nu_m);
    }
    let nu_sat = if nu_modes.iter().all(|&nm| nm > 2.0) {
        let e_fc: f64 = nu_modes.iter().map(|&nm| nm / (nm - 2.0)).sum();
        if e_fc > qf + 1e-12 { (2.0 * e_fc / (e_fc - qf)).clamp(1.0, ctx.n_minus_p) } else { ctx.n_minus_p }
    } else {
        nu_modes.iter().copied().fold(ctx.n_minus_p, f64::min).clamp(1.0, ctx.n_minus_p)
    };

    // Kenward & Roger (1997, Biometrics 53:985 Eq. (6)) scaled F* and denominator df.
    let sigma_inv = invert_reml_info(&sigma)?;
    let inv_b_sat = sigma_inv.matvec(beta_sub)?;
    let quad_sat: f64 = beta_sub.iter().zip(inv_b_sat.iter()).map(|(&b, &ib)| b * ib).sum();
    let f_sat = (quad_sat / qf).max(0.0);

    let mut t_mats = Vec::with_capacity(n_vc);
    let mut tr_t = Vec::with_capacity(n_vc);
    for pmp in ctx.phi_m_phi {
        let d_sub = subblock_dense(pmp, start, q)?;
        let ti = sigma_inv.matmul(&d_sub)?;
        let mut tr = 0.0_f64;
        for k in 0..q {
            tr += ti.get(k, k).unwrap_or(0.0);
        }
        tr_t.push(tr);
        t_mats.push(ti);
    }
    let mut a1 = 0.0_f64;
    let mut a2 = 0.0_f64;
    for i in 0..n_vc {
        for j in 0..n_vc {
            let wij = ctx.w_info_inv.get(i, j).unwrap_or(0.0);
            a1 += wij * tr_t[i] * tr_t[j];
            a2 += wij * trace_product(&t_mats[i], &t_mats[j]);
        }
    }
    let (lambda_kr, nu_kr) = kr_scale_and_df(qf, a1, a2, nu_sat, ctx.n_minus_p);
    let f_kr = (lambda_kr * f_raw).max(0.0);
    let kr = build_wald_term(term_name, qf, f_kr, nu_kr, ctx.sigma2_e)?;
    let sat = build_wald_term(term_name, qf, f_sat, nu_sat, ctx.sigma2_e)?;
    Ok(KrSatTermPair { kr, sat, nu_sat, nu_kr })
}

pub(in crate::anova) type RemlWaldTermsOut = (Vec<FactorialAnovaTerm>, Vec<f64>, Vec<f64>);

pub(in crate::anova) struct RemlContrastCore {
    pub(in crate::anova) beta: Vec<f64>,
    pub(in crate::anova) phi: DenseMatrix,
    pub(in crate::anova) phi_a: DenseMatrix,
    pub(in crate::anova) w_info_inv: DenseMatrix,
    pub(in crate::anova) phi_m_phi: Vec<DenseMatrix>,
    pub(in crate::anova) n_minus_p: f64,
    pub(in crate::anova) y_scale: f64,
    pub(in crate::anova) col_map: Vec<Option<usize>>,
    pub(in crate::anova) df_method: MixedModelDfMethod,
}

pub(in crate::anova) fn eval_reml_wald_terms_for_df(
    phi: &DenseMatrix,
    beta: &[f64],
    sigma2_e: f64,
    fisher: &OwnedRemlFisherMats,
    terms: &[(&str, usize, usize)],
    n_minus_p: f64,
    df_method: MixedModelDfMethod,
) -> Result<(RemlWaldTermsOut, RemlContrastCore), StatsError> {
    let w_info_inv = invert_reml_info(&fisher.info_e)?;
    let n_vc = fisher.m_mats.len();
    let p = phi.nrows();
    let mut phi_m_phi = Vec::with_capacity(n_vc);
    for m_mat in &fisher.m_mats {
        phi_m_phi.push(symmetrize_dense(&phi.matmul(m_mat)?.matmul(phi)?)?);
    }
    let mut lambda_data = vec![0.0_f64; p * p];
    for r in 0..n_vc {
        for s in 0..n_vc {
            let wrs = w_info_inv.get(r, s).unwrap_or(0.0);
            if wrs == 0.0 {
                continue;
            }
            let m_phi_m = fisher.m_mats[r].matmul(phi)?.matmul(&fisher.m_mats[s])?;
            let diff = dense_sub(&fisher.q_mats[r][s], &m_phi_m)?;
            let contrib = phi.matmul(&diff)?.matmul(phi)?;
            for (dst, &val) in lambda_data.iter_mut().zip(contrib.as_slice().iter()) {
                *dst += wrs * val;
            }
        }
    }
    let phi_a_raw: Vec<f64> = phi.as_slice().iter().zip(lambda_data.iter()).map(|(&a, &l)| a + 2.0 * l).collect();
    let phi_a_sym = symmetrize_dense(&DenseMatrix::from_row_major(p, p, phi_a_raw)?)?;
    let phi_a = if cholesky(&phi_a_sym).is_ok() { phi_a_sym } else { phi.clone() };
    let ctx = KrTermContext { phi, phi_a: &phi_a, beta, sigma2_e, w_info_inv: &w_info_inv, phi_m_phi: &phi_m_phi, n_minus_p };

    let mut fixed_terms = Vec::with_capacity(terms.len());
    let mut satt_dfs = Vec::with_capacity(terms.len());
    let mut kr_dfs = Vec::with_capacity(terms.len());
    for &(name, start, len) in terms {
        if len == 0 || start + len > p {
            continue;
        }
        let pair = if len == 1 { kr_single_term_df1(&ctx, name, start)? } else { kr_multi_term_df(&ctx, name, start, len)? };
        fixed_terms.push(match df_method {
            MixedModelDfMethod::KenwardRoger => pair.kr,
            MixedModelDfMethod::Satterthwaite => pair.sat,
        });
        satt_dfs.push(pair.nu_sat);
        kr_dfs.push(pair.nu_kr);
    }
    let core = RemlContrastCore {
        beta: beta.to_vec(),
        phi: phi.clone(),
        phi_a,
        w_info_inv,
        phi_m_phi,
        n_minus_p,
        y_scale: 1.0,
        col_map: (0..p).map(Some).collect(),
        df_method,
    };
    Ok(((fixed_terms, satt_dfs, kr_dfs), core))
}

/// Fit a linear mixed-effects random-intercept model via Restricted Maximum Likelihood (REML)
/// (Patterson & Thompson, 1971) with genuine Fai & Cornelius (1996) / Satterthwaite (1946)
/// and Kenward & Roger (1997) denominator degrees of freedom, adjusted covariance, and scaled *F*.
pub fn fit_random_intercept_reml(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    terms: &[(&str, usize, usize)],
) -> Result<MixedRemlResult, StatsError> {
    fit_random_intercept_reml_with_method_and_core(x, y, cluster_ids, terms, MixedModelDfMethod::KenwardRoger).map(|(res, _)| res)
}

/// Variant of [`fit_random_intercept_reml`] selecting Kenward–Roger (`Phi_A`, scaled `F*`) vs
/// Fai–Cornelius Satterthwaite (`Phi`, unscaled `F`) denominator *df* inference for fixed effects.
pub fn fit_random_intercept_reml_with_df_method(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<MixedRemlResult, StatsError> {
    fit_random_intercept_reml_with_method_and_core(x, y, cluster_ids, terms, df_method).map(|(res, _)| res)
}

pub(in crate::anova) fn fit_random_intercept_reml_with_method_and_core(
    x: &DenseMatrix,
    y: &[f64],
    cluster_ids: &[usize],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<(MixedRemlResult, RemlContrastCore), StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if cluster_ids.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: cluster_ids.len() }));
    }
    if n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "REML mixed model", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;
    let n_minus_p = (n - p) as f64;
    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let clusters = ri_precompute_clusters(x, &y_scaled, cluster_ids);
    let eval_u = |u: f64| -> Result<(f64, RiProfileEval), StatsError> {
        let tau = u.exp_m1().max(0.0);
        let ev = ri_eval_profile(&clusters, p, n_minus_p, tau)?;
        Ok((tau, ev))
    };

    let (_, zero_eval) = eval_u(0.0)?;
    let ll_zero = zero_eval.0;

    // Coarse search on u = ln(1 + tau) in [0, REML_LOG1P_TAU_MAX] + fine search in [0, 0.25].
    let n_coarse = 144_usize;
    let coarse_step = REML_LOG1P_TAU_MAX / (n_coarse as f64);
    let mut best_u = 0.0_f64;
    let mut best_ll = ll_zero;
    let mut best_step = coarse_step;
    for i in 1..=n_coarse {
        let u = (i as f64) * coarse_step;
        if let Ok((_, (ll, _, _, _))) = eval_u(u)
            && ll > best_ll
        {
            best_ll = ll;
            best_u = u;
        }
    }
    for i in 1..=25 {
        let u = (i as f64) * (0.25 / 25.0);
        if let Ok((_, (ll, _, _, _))) = eval_u(u)
            && ll > best_ll
        {
            best_ll = ll;
            best_u = u;
            best_step = 0.25 / 25.0;
        }
    }

    let mut a_u = (best_u - best_step).max(0.0);
    let mut b_u = (best_u + best_step).min(REML_LOG1P_TAU_MAX);
    let phi_gold = 0.5 * (5.0_f64.sqrt() - 1.0);
    let mut c_u = b_u - phi_gold * (b_u - a_u);
    let mut d_u = a_u + phi_gold * (b_u - a_u);
    let mut f_c = eval_u(c_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
    let mut f_d = eval_u(d_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
    for _ in 0..80 {
        if f_c < f_d {
            a_u = c_u;
            c_u = d_u;
            f_c = f_d;
            d_u = a_u + phi_gold * (b_u - a_u);
            f_d = eval_u(d_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
        } else {
            b_u = d_u;
            d_u = c_u;
            f_d = f_c;
            c_u = b_u - phi_gold * (b_u - a_u);
            f_c = eval_u(c_u).map_or(f64::NEG_INFINITY, |(_, ev)| ev.0);
        }
    }
    let cand_u = 0.5 * (a_u + b_u);
    let (cand_tau, cand_eval) = eval_u(cand_u)?;
    let (u_hat, tau_hat, (opt_ll, sigma2_e_scaled, xtvx_rel_inv, beta_scaled)) = if cand_eval.0 > best_ll && cand_eval.0 > ll_zero + 1e-12 {
        (cand_u, cand_tau, cand_eval)
    } else if best_ll > ll_zero + 1e-12 {
        (best_u, best_u.exp_m1().max(0.0), eval_u(best_u)?.1)
    } else {
        (0.0, 0.0, zero_eval)
    };
    let hit_upper_bound = u_hat >= REML_LOG1P_TAU_MAX - 1e-4;
    let at_boundary = tau_hat == 0.0 || hit_upper_bound;
    let converged = !hit_upper_bound && (b_u - a_u).abs() <= 1e-8 && opt_ll.is_finite();

    let lrt_chi2 = (2.0 * (opt_ll - ll_zero)).max(0.0);
    let lrt_p = if tau_hat > 0.0 && lrt_chi2 > 0.0 { 0.5 * chi2_sf(lrt_chi2, 1.0)? } else { 1.0 };

    let sigma2_e = sigma2_e_scaled * y_scale * y_scale;
    let sigma2_u = tau_hat * sigma2_e;
    let icc = if sigma2_u + sigma2_e > 0.0 { sigma2_u / (sigma2_u + sigma2_e) } else { 0.0 };
    let reml_log_lik = opt_ll - n_minus_p * y_scale.ln();
    let beta: Vec<f64> = beta_scaled.into_iter().map(|b| b * y_scale).collect();
    let phi = scale_dense(&xtvx_rel_inv, sigma2_e)?;

    let fisher = ri_build_fisher_mats(&clusters, p, n, tau_hat, sigma2_e, &xtvx_rel_inv)?;
    let ((fixed_terms, satterthwaite_df, kenward_roger_df), core) =
        eval_reml_wald_terms_for_df(&phi, &beta, sigma2_e, &fisher, terms, n_minus_p, df_method)?;

    let res = MixedRemlResult {
        sigma2_random: sigma2_u,
        sigma2_error: sigma2_e,
        icc,
        reml_log_lik,
        lrt_random_chi2: lrt_chi2,
        lrt_random_p: lrt_p,
        fixed_terms,
        satterthwaite_df,
        kenward_roger_df,
        converged,
        at_boundary,
    };
    Ok((res, core))
}

pub(in crate::anova) fn cluster_ids_to_g_matrix(cluster_ids: &[usize]) -> Result<(DenseMatrix, usize), StatsError> {
    let n = cluster_ids.len();
    let mut remap = std::collections::BTreeMap::<usize, usize>::new();
    let mut mapped = Vec::with_capacity(n);
    for &cid in cluster_ids {
        let next = remap.len();
        let idx = *remap.entry(cid).or_insert(next);
        mapped.push(idx);
    }
    let c_count = remap.len();
    let mut g = vec![0.0_f64; n * n];
    for i in 0..n {
        g[i * n + i] = 1.0;
        for j in (i + 1)..n {
            if mapped[i] == mapped[j] {
                g[i * n + j] = 1.0;
                g[j * n + i] = 1.0;
            }
        }
    }
    Ok((DenseMatrix::from_row_major(n, n, g)?, c_count))
}

pub(in crate::anova) fn multi_vc_eval_state(x: &DenseMatrix, y: &[f64], gs: &[DenseMatrix], theta: &[f64]) -> Result<MultiVcEvalState, StatsError> {
    let n = x.nrows();
    let n_vc = gs.len();
    let mut v_data = vec![0.0_f64; n * n];
    for (&th, g) in theta.iter().zip(gs.iter()) {
        for (dst, &gv) in v_data.iter_mut().zip(g.as_slice().iter()) {
            *dst += th * gv;
        }
    }
    let v_mat = symmetrize_dense(&DenseMatrix::from_row_major(n, n, v_data)?)?;
    let chol_v = cholesky(&v_mat)?;
    let ld_v = chol_v.ln_det();
    let vi = chol_v.inverse()?;

    let xt_vi = x.transpose().matmul(&vi)?;
    let xtvix = symmetrize_dense(&xt_vi.matmul(x)?)?;
    let chol_m = cholesky(&xtvix)?;
    let ld_m = chol_m.ln_det();
    let phi = chol_m.inverse()?;
    let xtvi_y = xt_vi.matvec(y)?;
    let beta = chol_m.solve(&xtvi_y)?;
    let x_beta = x.matvec(&beta)?;
    let resid: Vec<f64> = y.iter().zip(x_beta.iter()).map(|(&yv, &xb)| yv - xb).collect();
    let vir = vi.matvec(&resid)?;
    let quad: f64 = resid.iter().zip(vir.iter()).map(|(&r, &vr)| r * vr).sum();
    let ll = -0.5 * (ld_v + ld_m + quad);

    let proj = xt_vi.transpose().matmul(&phi)?.matmul(&xt_vi)?;
    let p_mat = symmetrize_dense(&dense_sub(&vi, &proj)?)?;

    let mut g_vir = Vec::with_capacity(n_vc);
    let mut score = Vec::with_capacity(n_vc);
    for g in gs {
        let gv = g.matvec(&vir)?;
        let tr_pg = trace_product(&p_mat, g);
        let q_g: f64 = vir.iter().zip(gv.iter()).map(|(&a, &b)| a * b).sum();
        score.push(-0.5 * (tr_pg - q_g));
        g_vir.push(gv);
    }
    let mut ai_data = vec![0.0_f64; n_vc * n_vc];
    for i in 0..n_vc {
        let p_g_vir_i = p_mat.matvec(&g_vir[i])?;
        for j in i..n_vc {
            let val: f64 = g_vir[j].iter().zip(p_g_vir_i.iter()).map(|(&a, &b)| a * b).sum();
            let ai_ij = 0.5 * val;
            ai_data[i * n_vc + j] = ai_ij;
            ai_data[j * n_vc + i] = ai_ij;
        }
    }
    let ai = DenseMatrix::from_row_major(n_vc, n_vc, ai_data)?;
    Ok(MultiVcEvalState { ll, score, ai, beta, phi, vi, p_mat, vir, quad })
}

pub(in crate::anova) fn multi_vc_em_step(st: &MultiVcEvalState, gs: &[DenseMatrix], dofs: &[f64], theta: &[f64], floor_positive: bool) -> Vec<f64> {
    let mut next = Vec::with_capacity(theta.len());
    for k in 0..theta.len() {
        let th = theta[k];
        let g_vir = gs[k].matvec(&st.vir).unwrap_or_default();
        let uhat_sq = th * th * st.vir.iter().zip(g_vir.iter()).map(|(&a, &b)| a * b).sum::<f64>();
        let cov_trace = dofs[k] * th - th * th * trace_product(&st.p_mat, &gs[k]);
        let floor = if k == 0 || floor_positive { 1e-12 } else { 0.0 };
        next.push(((uhat_sq + cov_trace) / dofs[k]).max(floor));
    }
    next
}

pub(in crate::anova) fn multi_vc_ai_step(
    x: &DenseMatrix,
    y: &[f64],
    gs: &[DenseMatrix],
    dofs: &[f64],
    theta: &mut [f64],
    n_minus_p: f64,
) -> Result<(bool, bool), StatsError> {
    let st = multi_vc_eval_state(x, y, gs, theta)?;
    let n_vc = theta.len();
    let free: Vec<usize> = (0..n_vc).filter(|&k| theta[k] > 1e-11 || st.score[k] > 1e-9).collect();
    if free.is_empty() {
        return Ok((true, true));
    }
    let max_active_grad = free.iter().map(|&k| st.score[k].abs()).fold(0.0_f64, f64::max);
    if max_active_grad < 1e-9 {
        return Ok((true, true));
    }

    let nf = free.len();
    let g_f: Vec<f64> = free.iter().map(|&k| st.score[k]).collect();
    let mut ai_f_data = Vec::with_capacity(nf * nf);
    for &i in &free {
        for &j in &free {
            ai_f_data.push(st.ai.get(i, j).unwrap_or(0.0));
        }
    }
    let ai_f = symmetrize_dense(&DenseMatrix::from_row_major(nf, nf, ai_f_data)?)?;
    let mut step_f = if let Ok(chol) = cholesky(&ai_f)
        && let Ok(sol) = chol.solve(&g_f)
    {
        sol
    } else {
        let eig = sym_eigen(&ai_f)?;
        let max_ev = eig.eigenvalues.iter().map(|&v| v.abs()).fold(0.0_f64, f64::max).max(1e-12);
        let qtg = eig.eigenvectors.t_matvec(&g_f)?;
        let scaled: Vec<f64> = qtg.iter().zip(eig.eigenvalues.iter()).map(|(&c, &ev)| c / ev.max(1e-8 * max_ev)).collect();
        eig.eigenvectors.matvec(&scaled)?
    };
    let directional: f64 = step_f.iter().zip(g_f.iter()).map(|(&d, &g)| d * g).sum();
    if directional <= 0.0 || !directional.is_finite() {
        step_f = g_f;
    }

    let mut step = vec![0.0_f64; n_vc];
    for (&k, &df) in free.iter().zip(step_f.iter()) {
        step[k] = df;
    }

    let mut alpha = 1.0_f64;
    for _ in 0..25 {
        let mut cand: Vec<f64> = theta.iter().zip(step.iter()).map(|(&t, &d)| (t + alpha * d).max(0.0)).collect();
        cand[0] = cand[0].max(1e-12);
        if let Ok(st_c) = multi_vc_eval_state(x, y, gs, &cand)
            && st_c.ll >= st.ll - 1e-13
        {
            let c_scale = st_c.quad / n_minus_p;
            let mut chosen = cand.clone();
            let mut chosen_ll = st_c.ll;
            if c_scale.is_finite() && c_scale > 1e-12 {
                let mut cand_r: Vec<f64> = cand.iter().map(|&v| v * c_scale).collect();
                cand_r[0] = cand_r[0].max(1e-12);
                if let Ok(st_r) = multi_vc_eval_state(x, y, gs, &cand_r)
                    && st_r.ll > chosen_ll
                {
                    chosen = cand_r;
                    chosen_ll = st_r.ll;
                }
            }
            let max_delta = theta.iter().zip(chosen.iter()).map(|(&a, &b)| (a - b).abs()).fold(0.0_f64, f64::max);
            let improved = chosen_ll > st.ll + 1e-13;
            theta.copy_from_slice(&chosen);
            return Ok((max_delta < 1e-11 && !improved, true));
        }
        alpha *= 0.5;
    }

    // Backtracking failed: take one guaranteed-monotone EM step.
    let em_next = multi_vc_em_step(&st, gs, dofs, theta, false);
    let max_delta = theta.iter().zip(em_next.iter()).map(|(&a, &b)| (a - b).abs()).fold(0.0_f64, f64::max);
    theta.copy_from_slice(&em_next);
    Ok((max_delta < 1e-11, false))
}

/// Fit a multi-variance-component linear mixed model `V = sigma_e^2 I + sum_k sigma_k^2 G_k`
/// via EM warm-start followed by projected Average-Information REML (Gilmour, Thompson & Cullis,
/// 1995), with genuine Fai & Cornelius (1996) Satterthwaite and Kenward & Roger (1997) fixed-effect
/// inference across all `(r + 1)` variance components.
pub fn fit_multi_vc_reml(
    x: &DenseMatrix,
    y: &[f64],
    random_effects: &[(&str, DenseMatrix, usize)],
    terms: &[(&str, usize, usize)],
) -> Result<MultiVcRemlResult, StatsError> {
    fit_multi_vc_reml_with_method_and_core(x, y, random_effects, terms, MixedModelDfMethod::KenwardRoger).map(|(res, _)| res)
}

/// Variant of [`fit_multi_vc_reml`] selecting Kenward–Roger vs Fai–Cornelius Satterthwaite fixed-effect inference.
pub fn fit_multi_vc_reml_with_df_method(
    x: &DenseMatrix,
    y: &[f64],
    random_effects: &[(&str, DenseMatrix, usize)],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<MultiVcRemlResult, StatsError> {
    fit_multi_vc_reml_with_method_and_core(x, y, random_effects, terms, df_method).map(|(res, _)| res)
}

pub(in crate::anova) fn fit_multi_vc_reml_with_method_and_core(
    x: &DenseMatrix,
    y: &[f64],
    random_effects: &[(&str, DenseMatrix, usize)],
    terms: &[(&str, usize, usize)],
    df_method: MixedModelDfMethod,
) -> Result<(MultiVcRemlResult, RemlContrastCore), StatsError> {
    let n = x.nrows();
    let p = x.ncols();
    if y.len() != n {
        return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: y.len() }));
    }
    if n <= p {
        return Err(StatsError::InsufficientSampleSize { context: "multi-VC REML model", required: p + 1, actual: n });
    }
    validate_finite_slice(y)?;
    let n_minus_p = (n - p) as f64;
    let y_moments = SummaryMoments::from_slice(y)?;
    let y_scale_raw = (y_moments.css / (n as f64)).sqrt();
    let y_scale = if y_scale_raw.is_finite() && y_scale_raw > 0.0 { y_scale_raw } else { 1.0 };
    let y_scaled: Vec<f64> = y.iter().map(|&v| v / y_scale).collect();

    let n_vc = 1 + random_effects.len();
    let mut gs = Vec::with_capacity(n_vc);
    let mut dofs = Vec::with_capacity(n_vc);
    let mut names = Vec::with_capacity(random_effects.len());
    gs.push(DenseMatrix::identity(n)?);
    dofs.push(n as f64);
    for (name, g_mat, rank_c) in random_effects {
        if g_mat.nrows() != n || g_mat.ncols() != n {
            return Err(StatsError::Math(curvebook_math::MathError::DimensionMismatch { expected: n, actual: g_mat.nrows().min(g_mat.ncols()) }));
        }
        gs.push(g_mat.clone());
        dofs.push((*rank_c).max(1) as f64);
        names.push((*name).to_owned());
    }

    let var0 = solve_linear_least_squares(x, &y_scaled).map_or(1.0, |(sol, _)| (sol.rss / n_minus_p).max(1e-6));
    let mut theta = vec![(var0 / (n_vc as f64)).max(1e-8); n_vc];
    for _ in 0..25 {
        if let Ok(st) = multi_vc_eval_state(x, &y_scaled, &gs, &theta) {
            theta = multi_vc_em_step(&st, &gs, &dofs, &theta, true);
        } else {
            break;
        }
    }
    for _ in 0..80 {
        let (converged, _) = multi_vc_ai_step(x, &y_scaled, &gs, &dofs, &mut theta, n_minus_p)?;
        if converged {
            break;
        }
    }

    let st = multi_vc_eval_state(x, &y_scaled, &gs, &theta)?;
    let mut ie_data = vec![0.0_f64; n_vc * n_vc];
    let mut m_mats = Vec::with_capacity(n_vc);
    let xt_vi = x.transpose().matmul(&st.vi)?;
    let vi_x = st.vi.matmul(x)?;
    for g in &gs {
        m_mats.push(symmetrize_dense(&xt_vi.matmul(g)?.matmul(&vi_x)?)?);
    }
    let mut q_mats = vec![Vec::with_capacity(n_vc); n_vc];
    let mut pg_mats = Vec::with_capacity(n_vc);
    for (r, gr) in gs.iter().enumerate() {
        pg_mats.push(st.p_mat.matmul(gr)?);
        let xt_vi_gr = xt_vi.matmul(gr)?;
        for gs_mat in &gs {
            q_mats[r].push(xt_vi_gr.matmul(&st.vi)?.matmul(gs_mat)?.matmul(&vi_x)?);
        }
    }
    for r in 0..n_vc {
        for s in r..n_vc {
            let ie_rs = 0.5 * trace_product(&pg_mats[r], &pg_mats[s]);
            ie_data[r * n_vc + s] = ie_rs;
            ie_data[s * n_vc + r] = ie_rs;
        }
    }
    let fisher = OwnedRemlFisherMats { info_e: DenseMatrix::from_row_major(n_vc, n_vc, ie_data)?, m_mats, q_mats };
    let sigma2_error = theta[0] * y_scale * y_scale;
    let ((mut fixed_terms, satterthwaite_df, kenward_roger_df), mut core) =
        eval_reml_wald_terms_for_df(&st.phi, &st.beta, theta[0], &fisher, terms, n_minus_p, df_method)?;
    core.y_scale = y_scale;
    for term in &mut fixed_terms {
        if let Some(f_val) = term.f_statistic {
            let ms = f_val * sigma2_error;
            term.ms = ms;
            term.ss = ms * term.df;
        }
    }
    let variance_components = names.into_iter().zip(theta.iter().skip(1).map(|&v| v * y_scale * y_scale)).collect();
    let reml_log_lik = st.ll - n_minus_p * y_scale.ln();

    let res = MultiVcRemlResult { variance_components, sigma2_error, reml_log_lik, fixed_terms, satterthwaite_df, kenward_roger_df };
    Ok((res, core))
}
