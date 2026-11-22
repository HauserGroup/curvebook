//! Poisson deviance and Cauchy robust fitting by iteratively reweighted least squares.
//!
//! References:
//! - Green, P. J. (1984). Iteratively reweighted least squares for maximum likelihood estimation, and some robust and resistant alternatives. *Journal of the Royal Statistical Society B* 46(2), 149–192.
//! - McCullagh, P. & Nelder, J. A. (1989). *Generalized Linear Models*, 2nd ed. Chapman & Hall.
//! - Holland, P. W. & Welsch, R. E. (1977). Robust regression using iteratively reweighted least-squares. *Communications in Statistics – Theory and Methods* 6(9), 813–827.
//! - Motulsky, H. J. & Brown, R. E. (2006). Detecting outliers when fitting data with nonlinear regression – a new method based on robust nonlinear regression and the false discovery rate. *BMC Bioinformatics* 7, 123.

use curvebook_doc::WeightingScheme;
use curvebook_math::linalg::{DenseMatrix, qr_col_pivoted};
use curvebook_math::special::deviance_bd0;
use curvebook_math::sum::NeumaierSum;

use crate::diagnostics::compute_rsdr;
use crate::dsl::ode_fit_budget_exceeded;
use crate::error::{FitError, FitStatus};
use crate::library::ParametricModel;
use crate::solver::layout::{ObsRecord, ParameterLayout, ode_budget_error, project_theta};
use crate::solver::lm::{
    LmRunOutput, compute_jacobian, eval_residuals_with_domain, nelder_mead_optimize, nelder_mead_poisson_feasibility_rescue, run_trust_region_lm,
    run_trust_region_lm_with_domain, solve_with_warm_start, solve_with_warm_start_with_domain,
};
use crate::solver::results::SolverConfig;

/// Validate strictly positive Poisson fitted means `mu_i > 0` and assign IRLS weights `1 / sqrt(mu_i)`.
pub(in crate::solver) fn assign_poisson_irls_weights(work_obs: &mut [ObsRecord], y_hat: &[f64]) -> Result<(), FitError> {
    if work_obs.len() != y_hat.len() || y_hat.is_empty() {
        return Err(FitError::ImpossibleWeights {
            reason: "Poisson deviance regression requires non-empty strictly positive fitted means".to_owned(),
        });
    }
    for (rec, &yh) in work_obs.iter_mut().zip(y_hat.iter()) {
        if !yh.is_finite() || yh <= 0.0 {
            return Err(FitError::ImpossibleWeights {
                reason: format!("Poisson deviance regression requires strictly positive fitted means (mu > 0, got {yh})"),
            });
        }
        let inv_sqrt = 1.0 / yh.sqrt();
        if !inv_sqrt.is_finite() {
            return Err(FitError::ImpossibleWeights { reason: "Poisson IRLS weight 1 / sqrt(mu) overflowed f64 range".to_owned() });
        }
        rec.sqrt_weight = inv_sqrt;
    }
    Ok(())
}

/// Poisson maximum-likelihood regression by iteratively reweighted least squares
/// (Green 1984; McCullagh & Nelder 1989): weights `w_i = 1 / mu_i` are frozen at the current
/// fitted means and the weighted least-squares problem is re-solved by Levenberg–Marquardt
/// until the deviance stabilises. A fixed point satisfies the Poisson score equations
/// `sum (y_i - mu_i) / mu_i * d mu_i / d theta = 0`. The reported `weighted_ss` is the deviance.
pub(in crate::solver) fn fit_poisson_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    if config.weighting != WeightingScheme::None || obs.iter().any(|rec| rec.point.explicit_weight.is_some()) {
        return Err(FitError::InvalidParameter {
            name: "weighting",
            reason: "Poisson deviance regression defines variance Var(Y) = mu internally and cannot be combined with least-squares weighting schemes or explicit observation weights".to_owned(),
        });
    }
    for rec in obs {
        if rec.point.y < 0.0 || !rec.point.y.is_finite() {
            return Err(FitError::ImpossibleWeights { reason: "Poisson deviance regression requires non-negative counts (Y >= 0)".to_owned() });
        }
    }
    let mut theta = init_theta.to_vec();
    let mut work_obs = obs.to_vec();
    let mut last_out = if eval_residuals_with_domain(model, layout, &work_obs, &theta, true).is_some() {
        solve_with_warm_start_with_domain(model, layout, &work_obs, &theta, config, true)?
    } else if layout.k_free() == 0 {
        return Err(FitError::ImpossibleWeights {
            reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
        });
    } else if !config.use_simplex_warm_start {
        if eval_residuals_with_domain(model, layout, &work_obs, &theta, false).is_some() {
            return Err(FitError::BadInitialValues {
                reason: "Starting parameters produce non-positive Poisson predicted means (mu <= 0)".to_owned(),
            });
        }
        return Err(FitError::BadInitialValues { reason: "Starting parameters evaluate outside model domain".to_owned() });
    } else {
        // When initial parameters give mu <= 0, rescue via unconstrained LS pre-fit and/or a
        // feasibility-barrier Nelder-Mead search inside parameter bounds before rejecting as impossible.
        let mut ls_cand = None;
        if eval_residuals_with_domain(model, layout, &work_obs, &theta, false).is_some() {
            match solve_with_warm_start_with_domain(model, layout, &work_obs, &theta, config, false) {
                Ok(out) => ls_cand = Some(out),
                Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                Err(_) => {}
            }
        }
        let ls_outcome = ls_cand
            .map(|out| if eval_residuals_with_domain(model, layout, &work_obs, &out.theta, true).is_some() { Ok(out) } else { Err(out.theta) });
        match ls_outcome {
            Some(Ok(feasible_ls)) => feasible_ls,
            other => {
                let ls_seed = other.as_ref().and_then(|res| res.as_ref().err().map(Vec::as_slice));
                let feasible = nelder_mead_poisson_feasibility_rescue(model, layout, &work_obs, &theta)
                    .or_else(|| ls_seed.and_then(|s| nelder_mead_poisson_feasibility_rescue(model, layout, &work_obs, s)));
                if let Some(start_pos) = feasible {
                    solve_with_warm_start_with_domain(model, layout, &work_obs, &start_pos, config, true)?
                } else if let Some(err) = ode_budget_error() {
                    return Err(err);
                } else {
                    return Err(FitError::ImpossibleWeights {
                        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
                    });
                }
            }
        }
    };
    let mut prev_dev = poisson_deviance(&work_obs, &last_out.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
    })?;
    let mut total_iters = last_out.iterations;
    let bounds = layout.bounds();
    let k_free = layout.k_free();
    let mut outer_converged = k_free == 0 && last_out.status == FitStatus::Converged;

    if k_free > 0 {
        for _ in 0..12 {
            theta = last_out.theta.clone();
            assign_poisson_irls_weights(&mut work_obs, &last_out.y_hat)?;
            let next = run_trust_region_lm_with_domain(model, layout, &work_obs, &theta, config.max_iters, 10.0, true)?;
            total_iters = total_iters.saturating_add(next.iterations);

            if let Some(dev) = poisson_deviance(&work_obs, &next.y_hat)
                && dev <= prev_dev
            {
                let rel_change = (prev_dev - dev).abs() / prev_dev.max(1.0);
                let max_step = max_rel_theta_step(&theta, &next.theta);
                let next_status = next.status;
                last_out = next;
                prev_dev = dev;
                if next_status == FitStatus::Converged && rel_change < 1e-10 {
                    outer_converged = true;
                    break;
                }
                if next_status == FitStatus::Diverged && max_step == 0.0 {
                    break;
                }
                continue;
            }

            // Step-halving line search on IRLS step when linearization overshoots Poisson deviance
            // or lands at non-positive fitted means (matching `solve_profile_subproblem_poisson`).
            let mut improved = false;
            let mut alpha = 0.5_f64;
            for _ in 0..8 {
                let mut blended: Vec<f64> = theta.iter().zip(next.theta.iter()).map(|(&a, &b)| a + alpha * (b - a)).collect();
                project_theta(&mut blended, &bounds);
                if let Some((w_res, blend_yh, ss)) = eval_residuals_with_domain(model, layout, &work_obs, &blended, true)
                    && let Some(blend_dev) = poisson_deviance(&work_obs, &blend_yh)
                    && blend_dev < prev_dev
                {
                    let rel_change = (prev_dev - blend_dev).abs() / prev_dev.max(1.0);
                    let j_mat = compute_jacobian(model, layout, &work_obs, &blended, &blend_yh, &bounds).unwrap_or_else(|| next.j_mat.clone());
                    last_out = LmRunOutput {
                        theta: blended,
                        weighted_res: w_res,
                        y_hat: blend_yh,
                        weighted_ss: ss,
                        j_mat,
                        status: next.status,
                        iterations: next.iterations,
                    };
                    prev_dev = blend_dev;
                    improved = true;
                    if next.status == FitStatus::Converged && rel_change < 1e-10 {
                        outer_converged = true;
                    }
                    break;
                }
                alpha *= 0.5;
            }
            if outer_converged {
                break;
            }
            if !improved {
                if next.status == FitStatus::Converged && max_rel_theta_step(&theta, &next.theta) < 1e-8 {
                    outer_converged = true;
                } else if next.status == FitStatus::Diverged {
                    last_out.status = FitStatus::Diverged;
                }
                break;
            }
        }
    }

    last_out.iterations = total_iters;
    if !outer_converged && last_out.status != FitStatus::Diverged {
        last_out.status = FitStatus::HitMaxIters;
    }
    assign_poisson_irls_weights(&mut work_obs, &last_out.y_hat)?;
    if let (Some((wr, _, _)), Some(jm)) = (
        eval_residuals_with_domain(model, layout, &work_obs, &last_out.theta, true),
        compute_jacobian(model, layout, &work_obs, &last_out.theta, &last_out.y_hat, &bounds),
    ) {
        last_out.weighted_res = wr;
        last_out.j_mat = jm;
    }
    last_out.weighted_ss = poisson_deviance(&work_obs, &last_out.y_hat).ok_or_else(|| FitError::ImpossibleWeights {
        reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
    })?;
    Ok(last_out)
}

fn max_rel_theta_step(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&va, &vb)| (va - vb).abs() / vb.abs().max(va.abs()).max(f64::MIN_POSITIVE)).fold(0.0_f64, f64::max)
}

/// Poisson deviance `D = 2 sum [ y ln(y / mu) - (y - mu) ]` (McCullagh & Nelder 1989), evaluated
/// stably via `deviance_bd0`. Returns `None` if any observed count is negative/non-finite or any
/// fitted mean `mu <= 0`.
pub(in crate::solver) fn poisson_deviance(obs: &[ObsRecord], y_hat: &[f64]) -> Option<f64> {
    if obs.len() != y_hat.len() || obs.is_empty() {
        return None;
    }
    let mut acc = NeumaierSum::new();
    for (rec, &mu) in obs.iter().zip(y_hat.iter()) {
        if !rec.point.y.is_finite() || rec.point.y < 0.0 || !mu.is_finite() || mu <= 0.0 {
            return None;
        }
        acc.add(2.0 * deviance_bd0(rec.point.y, mu));
    }
    let dev = acc.total();
    dev.is_finite().then_some(dev)
}

/// Robust Cauchy (Lorentzian) fit minimising `sum ln(1 + (r_{w,i} / RSDR)^2)`, the merit function
/// of Motulsky & Brown (2006, *BMC Bioinformatics* 7:123), with `RSDR` re-estimated from the
/// base-weighted residuals `r_{w,i} = (y_i - \hat{y}_i) * sqrt(w_i)` at every iteration so
/// heteroscedastic weighting schemes (`1/Y^2`, `1/|Y|`, etc.) are standardized consistently.
///
/// Deviation: Motulsky & Brown minimise the Lorentzian merit with a modified Marquardt
/// algorithm; this routine instead uses IRLS with Cauchy weights `w_i = 1 / (1 + z_i^2)`,
/// `z_i = |r_{w,i}| / RSDR` (Holland & Welsch 1977), each weighted subproblem solved by
/// Levenberg–Marquardt. For a fixed scale both reach the same stationary point. Standard
/// errors from this fit are not robust-theory SEs and are not used by ROUT.
pub(in crate::solver) fn cauchy_marginal_nll(obs: &[ObsRecord], y_hat: &[f64], k_free: usize, w_y_scale: f64) -> f64 {
    let abs_res: Vec<f64> = obs.iter().zip(y_hat.iter()).map(|(o, yh)| ((o.point.y - yh) * o.sqrt_weight).abs()).collect();
    let s = compute_rsdr(&abs_res, k_free).max(1e-6 * w_y_scale);
    let penalty: f64 = abs_res
        .iter()
        .map(|&r| {
            let z = r / s;
            (1.0 + z * z).ln()
        })
        .sum();
    (obs.len() as f64) * s.ln() + penalty
}

pub(in crate::solver) fn run_cauchy_irls_loop(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    mut last_out: LmRunOutput,
    config: &SolverConfig,
    w_y_scale: f64,
) -> Result<LmRunOutput, FitError> {
    let k_free = layout.k_free();
    if k_free == 0 {
        return Ok(last_out);
    }
    let mut work_obs = obs.to_vec();
    let mut total_iters = last_out.iterations;
    let mut outer_converged = false;
    let max_passes = config.max_iters.clamp(1, 60);
    for _ in 0..max_passes {
        let prev_theta = last_out.theta.clone();
        let abs_res: Vec<f64> = obs.iter().zip(last_out.y_hat.iter()).map(|(o, yh)| ((o.point.y - yh) * o.sqrt_weight).abs()).collect();
        let rsdr = compute_rsdr(&abs_res, k_free).max(1e-12 * w_y_scale);
        for (i, rec) in work_obs.iter_mut().enumerate() {
            let z = abs_res[i] / rsdr;
            let cauchy_w = 1.0 / (1.0 + z * z);
            rec.sqrt_weight = obs[i].sqrt_weight * cauchy_w.sqrt();
        }
        let next = run_trust_region_lm(model, layout, &work_obs, &prev_theta, config.max_iters, 10.0)?;
        total_iters = total_iters.saturating_add(next.iterations);
        let max_step = max_rel_theta_step(&prev_theta, &next.theta);
        last_out = next;
        if last_out.status == FitStatus::Diverged {
            break;
        }
        if max_step < 1e-8 && last_out.status == FitStatus::Converged {
            outer_converged = true;
            break;
        }
    }
    last_out.iterations = total_iters;
    if !outer_converged && last_out.status != FitStatus::Diverged {
        last_out.status = FitStatus::HitMaxIters;
    }
    Ok(last_out)
}

pub(in crate::solver) fn fit_cauchy_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    let k_free = layout.k_free();
    let w_y_scale = obs.iter().map(|o| (o.point.y * o.sqrt_weight).abs()).fold(0.0_f64, f64::max).max(f64::MIN_POSITIVE);
    let ols_seed = solve_with_warm_start(model, layout, obs, init_theta, config)?;
    let mut best = run_cauchy_irls_loop(model, layout, obs, ols_seed, config, w_y_scale)?;

    // If L2 OLS warm start chased an outlier into an unrepresentative steep step, also launch
    // Cauchy IRLS directly from a Lorentzian-merit simplex search seeded at `init_theta`.
    if k_free > 0 && config.use_simplex_warm_start {
        let bounds = layout.bounds();
        let nm_seed = nelder_mead_optimize(layout, init_theta, 500, 0.05, |th| {
            if ode_fit_budget_exceeded().is_some() {
                return f64::INFINITY;
            }
            let mut clamped = th.to_vec();
            project_theta(&mut clamped, &bounds);
            eval_residuals_with_domain(model, layout, obs, &clamped, false)
                .map_or(f64::INFINITY, |(_, y_hat, _)| cauchy_marginal_nll(obs, &y_hat, k_free, w_y_scale))
        });
        if let Some(seed_th) = nm_seed
            && let Some((w_res, y_hat, ss)) = eval_residuals_with_domain(model, layout, obs, &seed_th, false)
            && let Ok(j_mat) = DenseMatrix::zeros(obs.len(), k_free)
        {
            let seed_out =
                LmRunOutput { theta: seed_th, weighted_res: w_res, y_hat, weighted_ss: ss, j_mat, status: FitStatus::Converged, iterations: 0 };
            if let Ok(alt) = run_cauchy_irls_loop(model, layout, obs, seed_out, config, w_y_scale) {
                let best_nll = cauchy_marginal_nll(obs, &best.y_hat, k_free, w_y_scale);
                let alt_nll = cauchy_marginal_nll(obs, &alt.y_hat, k_free, w_y_scale);
                let prefer_alt_status = alt.status == FitStatus::Converged && best.status != FitStatus::Converged && alt_nll <= best_nll + 1e-6;
                if alt_nll + 1e-6 < best_nll || prefer_alt_status {
                    best = alt;
                }
            }
        }
    }
    Ok(best)
}

pub(in crate::solver) fn compute_unscaled_cov(j_mat: &DenseMatrix) -> Option<DenseMatrix> {
    let p = j_mat.ncols();
    let nrows = j_mat.nrows();
    if p == 0 || nrows < p {
        return None;
    }
    let mut col_norms = vec![0.0_f64; p];
    let mut j_scaled = DenseMatrix::zeros(nrows, p).ok()?;
    for (c, cn_slot) in col_norms.iter_mut().enumerate().take(p) {
        let cn = j_mat.col_norm2(c);
        if !cn.is_finite() || cn <= 0.0 {
            return None;
        }
        *cn_slot = cn;
        let inv_cn = 1.0 / cn;
        for r in 0..nrows {
            if let Some(slot) = j_scaled.get_mut(r, c) {
                *slot = j_mat.get(r, c).unwrap_or(0.0) * inv_cn;
            }
        }
    }
    let pqr = qr_col_pivoted(&j_scaled).ok()?;
    if pqr.rank() < p {
        return None;
    }
    let inv_scaled = pqr.xtx_inverse().ok()?;
    let mut inv = DenseMatrix::zeros(p, p).ok()?;
    for i in 0..p {
        let di = col_norms[i];
        for j in 0..p {
            let dj = col_norms[j];
            if let Some(slot) = inv.get_mut(i, j) {
                *slot = (inv_scaled.get(i, j).unwrap_or(0.0) / di) / dj;
            }
        }
    }
    Some(inv)
}
