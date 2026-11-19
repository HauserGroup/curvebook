//! Profile-likelihood confidence intervals by re-fitting with one parameter held fixed.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.

use curvebook_math::linalg::DenseMatrix;
use curvebook_math::sum::NeumaierSum;

use crate::library::ParametricModel;
use crate::solver::irls::{assign_poisson_irls_weights, poisson_deviance};
use crate::solver::layout::{ObsRecord, ParameterLayout, project_theta};
use crate::solver::lm::{
    LmDampWorkspace, compute_jacobian, eval_residuals, eval_residuals_with_domain, mat_get, mat_set, scaled_l2_norm, solve_trust_region_subproblem,
};

pub(in crate::solver) struct ReducedLmState {
    pub(in crate::solver) y_hat: Vec<f64>,
    pub(in crate::solver) ss_curr: f64,
}

#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn run_reduced_lm_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    work: &mut [f64],
    map_indices: &[usize],
    full_bounds: &[(Option<f64>, Option<f64>)],
    max_iters: usize,
    require_positive_mean: bool,
) -> Option<ReducedLmState> {
    let sub_k = map_indices.len();
    let sub_bounds: Vec<(Option<f64>, Option<f64>)> = map_indices.iter().map(|&i| full_bounds[i]).collect();
    let mut sub_theta: Vec<f64> = map_indices.iter().map(|&i| work[i]).collect();
    project_theta(&mut sub_theta, &sub_bounds);

    let mut diag = vec![1.0_f64; sub_k];
    let mut delta = 1.0_f64;
    let mut lm_ws = LmDampWorkspace::new(obs.len(), sub_k).ok()?;
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    let (mut w_res, mut y_hat, mut ss_curr) = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean)?;

    for iter in 0..max_iters {
        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
            work[orig] = v;
        }
        let j_full = compute_jacobian(model, layout, obs, work, &y_hat, full_bounds)?;
        let mut j_sub = DenseMatrix::zeros(obs.len(), sub_k).ok()?;
        let mut max_g_cos = 0.0_f64;
        let fnorm = ss_curr.sqrt();
        for (sub_j, &orig_j) in map_indices.iter().enumerate() {
            let mut col_ss = NeumaierSum::new();
            let mut col_dot = NeumaierSum::new();
            for (i, &wr) in w_res.iter().enumerate().take(obs.len()) {
                let val = mat_get(&j_full, i, orig_j);
                mat_set(&mut j_sub, i, sub_j, val);
                col_ss.add(val * val);
                col_dot.add(val * wr);
            }
            let raw_cn = col_ss.total().sqrt();
            if raw_cn > 0.0 && fnorm > 0.0 {
                let grad_j = col_dot.total();
                let xj = sub_theta[sub_j];
                let at_lo = sub_bounds[sub_j].0.is_some_and(|l| (xj - l).abs() <= 1e-12 * l.abs().max(f64::MIN_POSITIVE) && grad_j < 0.0);
                let at_hi = sub_bounds[sub_j].1.is_some_and(|u| (u - xj).abs() <= 1e-12 * u.abs().max(f64::MIN_POSITIVE) && grad_j > 0.0);
                if !at_lo && !at_hi {
                    max_g_cos = max_g_cos.max((grad_j / (raw_cn * fnorm)).abs());
                }
            }
            let cn = if raw_cn > f64::MIN_POSITIVE.sqrt() { raw_cn } else { 1.0 };
            diag[sub_j] = if iter == 0 { cn } else { diag[sub_j].max(cn) };
        }
        if ss_curr == 0.0 || max_g_cos <= 1e-12 {
            break;
        }
        if iter == 0 {
            let theta_scale = scaled_l2_norm(&diag, &sub_theta);
            delta = 10.0 * if theta_scale > 0.0 { theta_scale } else { 1.0 };
        }

        let mut accepted = false;
        let mut lam = 0.0_f64;
        for _ in 0..15 {
            let Ok((step, lam_u)) = solve_trust_region_subproblem(&j_sub, &w_res, &diag, &sub_theta, delta, lam, &mut lm_ws) else {
                break;
            };
            lam = lam_u;
            let mut trial: Vec<f64> = sub_theta.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
            project_theta(&mut trial, &sub_bounds);
            let actual_step: Vec<f64> = trial.iter().zip(sub_theta.iter()).map(|(a, b)| a - b).collect();
            let p_norm = scaled_l2_norm(&diag, &actual_step);

            for (&orig, &v) in map_indices.iter().zip(trial.iter()) {
                work[orig] = v;
            }
            if let Some((r_t, y_t, ss_t)) = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean) {
                let act = ss_curr - ss_t;
                if act >= 0.0 {
                    sub_theta = trial;
                    w_res = r_t;
                    y_hat = y_t;
                    ss_curr = ss_t;
                    accepted = true;
                    delta = delta.max(2.0 * p_norm);
                    let theta_norm = scaled_l2_norm(&diag, &sub_theta);
                    let step_tol = 1e-10 * theta_norm.max(f64::MIN_POSITIVE);
                    if act <= 1e-11 * ss_curr.max(f64::MIN_POSITIVE) || p_norm <= step_tol {
                        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                            work[orig] = v;
                        }
                        return Some(ReducedLmState { y_hat, ss_curr });
                    }
                    break;
                }
            }
            delta *= 0.25;
            lam = (lam * 4.0).max(1e-8);
        }
        if !accepted {
            break;
        }
    }
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    Some(ReducedLmState { y_hat, ss_curr })
}

pub(in crate::solver) fn check_profile_fixed_param(
    layout: &ParameterLayout,
    fixed_idx: usize,
    fixed_val: f64,
) -> Option<Vec<(Option<f64>, Option<f64>)>> {
    let k = layout.k_free();
    if k == 0 || fixed_idx >= k || !fixed_val.is_finite() {
        return None;
    }
    let full_bounds = layout.bounds();
    if let Some(lb) = full_bounds[fixed_idx].0
        && fixed_val < lb
    {
        return None;
    }
    if let Some(ub) = full_bounds[fixed_idx].1
        && fixed_val > ub
    {
        return None;
    }
    Some(full_bounds)
}

/// Optimize the remaining `K - 1` nuisance parameters with `theta[fixed_idx] = fixed_val` held constant.
///
/// Returns the profile sum of squares `S~(theta_k)` of Bates & Watts (1988). Delegates to
/// `run_reduced_lm`, a simplified trust-region LM (Moré 1978 subproblem solver, accepting any
/// non-increasing step and shrinking the radius by 4 otherwise, with no gain-ratio test).
pub(in crate::solver) fn solve_profile_subproblem(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
    ss_within: f64,
) -> Option<(f64, Vec<f64>)> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    if k == 1 {
        let (_, _, ss) = eval_residuals(model, layout, obs, &work)?;
        return Some((ss + ss_within, work));
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let state = run_reduced_lm_with_domain(model, layout, obs, &mut work, &map_indices, &full_bounds, 40, false)?;
    Some((state.ss_curr + ss_within, work))
}

/// Optimize the remaining `K - 1` nuisance parameters under Poisson deviance with `theta[fixed_idx] = fixed_val`
/// held constant via reduced IRLS + deviance monotonicity guard.
pub(in crate::solver) fn solve_profile_subproblem_poisson(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
) -> Option<(f64, Vec<f64>)> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    let (_, init_y_hat, _) = eval_residuals_with_domain(model, layout, obs, &work, true)?;
    if k == 1 {
        let dev = poisson_deviance(obs, &init_y_hat)?;
        return Some((dev, work));
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let mut work_obs = obs.to_vec();
    let mut curr_y_hat = init_y_hat;
    let mut curr_dev = poisson_deviance(&work_obs, &curr_y_hat)?;

    for _ in 0..12 {
        if assign_poisson_irls_weights(&mut work_obs, &curr_y_hat).is_err() {
            return None;
        }
        let mut trial = work.clone();
        let Some(state) = run_reduced_lm_with_domain(model, layout, &work_obs, &mut trial, &map_indices, &full_bounds, 30, true) else {
            break;
        };
        if let Some(next_dev) = poisson_deviance(&work_obs, &state.y_hat)
            && next_dev <= curr_dev
        {
            let rel_change = (curr_dev - next_dev).abs() / curr_dev.max(1.0);
            work = trial;
            curr_y_hat = state.y_hat;
            curr_dev = next_dev;
            if rel_change < 1e-10 {
                break;
            }
            continue;
        }
        // Step-halving line search on IRLS step if linearization briefly overshot deviance.
        let mut improved = false;
        let mut alpha = 0.5_f64;
        for _ in 0..5 {
            let mut blended = work.clone();
            for &idx in &map_indices {
                blended[idx] = work[idx] + alpha * (trial[idx] - work[idx]);
            }
            if let Some((_, blend_yh, _)) = eval_residuals_with_domain(model, layout, &work_obs, &blended, true)
                && let Some(blend_dev) = poisson_deviance(&work_obs, &blend_yh)
                && blend_dev < curr_dev
            {
                work = blended;
                curr_y_hat = blend_yh;
                curr_dev = blend_dev;
                improved = true;
                break;
            }
            alpha *= 0.5;
        }
        if !improved {
            break;
        }
    }
    poisson_deviance(&work_obs, &curr_y_hat).map(|dev| (dev, work))
}
