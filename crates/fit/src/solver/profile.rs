//! Profile-likelihood confidence intervals by re-fitting with one parameter held fixed.
//!
//! References:
//! - Bates, D. M. & Watts, D. G. (1988). *Nonlinear Regression Analysis and Its Applications*. Wiley.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.

use curvebook_math::linalg::DenseMatrix;

use crate::diagnostics::ProfileStep;
use crate::error::FitStatus;
use crate::library::ParametricModel;
use crate::solver::inference::obs_y_ss;
use crate::solver::irls::{assign_poisson_irls_weights, poisson_deviance};
use crate::solver::layout::{ObsRecord, ParameterLayout, project_theta};
use crate::solver::lm::{
    LmDampWorkspace, LmStepContext, TrStepOutcome, compute_jacobian, eval_residuals, eval_residuals_with_domain, mat_get, mat_set, project_tr_trial,
    quadratic_pred_num, scaled_l2_norm, solve_trust_region_subproblem, stalled_lm_status, update_lm_diag_and_projected_grad, update_tr_radius_more,
};

pub(in crate::solver) struct ReducedLmState {
    pub(in crate::solver) y_hat: Vec<f64>,
    pub(in crate::solver) ss_curr: f64,
    pub(in crate::solver) converged: bool,
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

    let ftol = 1e-12_f64;
    let xtol = 1e-12_f64;
    let gtol = 1e-11_f64;
    let stall_gtol = 1e-4_f64;
    let y_norm = obs_y_ss(obs).sqrt();

    let mut diag = vec![0.0_f64; sub_k];
    let mut delta = 0.0_f64;
    let mut iter = 0_usize;
    let mut last_max_g_cos = f64::INFINITY;
    let mut lm_ws = LmDampWorkspace::new(obs.len(), sub_k).ok()?;
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    let (mut w_res, mut y_hat, mut ss_curr) = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean)?;

    while iter < max_iters {
        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
            work[orig] = v;
        }
        let j_full = compute_jacobian(model, layout, obs, work, &y_hat, full_bounds)?;
        let mut j_sub = DenseMatrix::zeros(obs.len(), sub_k).ok()?;
        for (sub_j, &orig_j) in map_indices.iter().enumerate() {
            for i in 0..obs.len() {
                mat_set(&mut j_sub, i, sub_j, mat_get(&j_full, i, orig_j));
            }
        }
        if ss_curr == 0.0 {
            return Some(ReducedLmState { y_hat, ss_curr, converged: true });
        }
        let max_g_cos = update_lm_diag_and_projected_grad(&j_sub, &w_res, &sub_theta, &sub_bounds, ss_curr, &mut diag);
        last_max_g_cos = max_g_cos;
        if max_g_cos <= gtol {
            return Some(ReducedLmState { y_hat, ss_curr, converged: true });
        }
        if iter == 0 {
            let theta_scale = scaled_l2_norm(&diag, &sub_theta);
            delta = 10.0 * if theta_scale > 0.0 { theta_scale } else { 1.0 };
        }

        let mut accepted = false;
        let mut lam = 0.0_f64;
        for _ in 0..25 {
            let Ok((step, lam_u)) = solve_trust_region_subproblem(&j_sub, &w_res, &diag, &sub_theta, delta, lam, &mut lm_ws) else {
                break;
            };
            lam = lam_u;
            let ctx = LmStepContext { j_mat: &j_sub, w_res: &w_res, diag: &diag, theta: &sub_theta };
            let Ok((trial, actual_step, p_norm)) = project_tr_trial(&ctx, &step, &sub_bounds, delta, &mut lam) else {
                break;
            };
            if iter == 0 && p_norm > 0.0 {
                delta = delta.min(p_norm);
            }

            let pred_num = quadratic_pred_num(&j_sub, &w_res, &actual_step).max(0.0);
            let pred_red = if ss_curr > 0.0 { pred_num / ss_curr } else { 0.0 };

            for (&orig, &v) in map_indices.iter().zip(trial.iter()) {
                work[orig] = v;
            }
            let trial_eval = eval_residuals_with_domain(model, layout, obs, work, require_positive_mean);
            let act_red = match &trial_eval {
                Some((_, _, ss_t)) if *ss_t < ss_curr => 1.0 - *ss_t / ss_curr,
                _ => -1.0,
            };
            let rho = if pred_red > 0.0 && trial_eval.is_some() {
                act_red / pred_red
            } else if act_red > 0.0 {
                1.0
            } else {
                -1.0
            };

            let ss_trial_val = trial_eval.as_ref().map(|(_, _, ss_t)| *ss_t);
            let outcome = TrStepOutcome { actual_step: &actual_step, p_norm, ss_curr, ss_trial: ss_trial_val, rho };
            update_tr_radius_more(&ctx, &outcome, &mut delta, &mut lam);

            if rho >= 1e-4
                && let Some((r_t, y_t, ss_t)) = trial_eval
            {
                sub_theta = trial;
                w_res = r_t;
                y_hat = y_t;
                ss_curr = ss_t;
                iter += 1;
                accepted = true;
                let raw_xnorm = scaled_l2_norm(&diag, &sub_theta);
                let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
                if (act_red.abs() <= ftol && pred_red <= ftol && rho <= 2.0) || p_norm <= xtol * x_norm {
                    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                        work[orig] = v;
                    }
                    return Some(ReducedLmState { y_hat, ss_curr, converged: true });
                }
                break;
            }

            let raw_xnorm = scaled_l2_norm(&diag, &sub_theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            if p_norm <= xtol * x_norm || delta <= 1e-15 * x_norm {
                for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                    work[orig] = v;
                }
                let converged = stalled_lm_status(max_g_cos, ss_curr, y_norm, stall_gtol) == FitStatus::Converged;
                return Some(ReducedLmState { y_hat, ss_curr, converged });
            }
        }
        if !accepted {
            break;
        }
    }
    for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
        work[orig] = v;
    }
    let converged = iter < max_iters && stalled_lm_status(last_max_g_cos, ss_curr, y_norm, stall_gtol) == FitStatus::Converged;
    Some(ReducedLmState { y_hat, ss_curr, converged })
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
/// Returns the profile sum of squares `S~(theta_k)` of Bates & Watts (1988) along with
/// reduced-LM convergence status via [`run_reduced_lm_with_domain`] (Moré 1978 trust-region
/// gain-ratio step acceptance and `ftol`/`xtol`/`gtol` stopping).
pub(in crate::solver) fn solve_profile_subproblem(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
    ss_within: f64,
) -> Option<ProfileStep> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    if k == 1 {
        let (_, _, ss) = eval_residuals(model, layout, obs, &work)?;
        return Some(ProfileStep { objective: ss + ss_within, theta: work, converged: true });
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let state = run_reduced_lm_with_domain(model, layout, obs, &mut work, &map_indices, &full_bounds, 200, false)?;
    Some(ProfileStep { objective: state.ss_curr + ss_within, theta: work, converged: state.converged })
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
) -> Option<ProfileStep> {
    let full_bounds = check_profile_fixed_param(layout, fixed_idx, fixed_val)?;
    let k = layout.k_free();
    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    let (_, init_y_hat, _) = eval_residuals_with_domain(model, layout, obs, &work, true)?;
    if k == 1 {
        let dev = poisson_deviance(obs, &init_y_hat)?;
        return Some(ProfileStep { objective: dev, theta: work, converged: true });
    }

    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let mut work_obs = obs.to_vec();
    let mut curr_y_hat = init_y_hat;
    let mut curr_dev = poisson_deviance(&work_obs, &curr_y_hat)?;
    let mut irls_converged = false;

    for _ in 0..20 {
        if assign_poisson_irls_weights(&mut work_obs, &curr_y_hat).is_err() {
            return None;
        }
        let mut trial = work.clone();
        let Some(state) = run_reduced_lm_with_domain(model, layout, &work_obs, &mut trial, &map_indices, &full_bounds, 80, true) else {
            irls_converged = false;
            break;
        };
        if let Some(next_dev) = poisson_deviance(&work_obs, &state.y_hat)
            && next_dev <= curr_dev
        {
            let rel_change = (curr_dev - next_dev).abs() / curr_dev.max(1.0);
            work = trial;
            curr_y_hat = state.y_hat;
            curr_dev = next_dev;
            if rel_change < 1e-10 && state.converged {
                irls_converged = true;
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
                let rel_change = (curr_dev - blend_dev).abs() / curr_dev.max(1.0);
                work = blended;
                curr_y_hat = blend_yh;
                curr_dev = blend_dev;
                improved = true;
                if rel_change < 1e-10 && state.converged {
                    irls_converged = true;
                }
                break;
            }
            alpha *= 0.5;
        }
        if irls_converged || !improved {
            if !improved && state.converged {
                irls_converged = true;
            }
            break;
        }
    }
    poisson_deviance(&work_obs, &curr_y_hat).map(|dev| ProfileStep { objective: dev, theta: work, converged: irls_converged })
}
