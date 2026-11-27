//! Residual and Jacobian evaluation, the trust-region Levenberg–Marquardt core with bound handling,
//! and the Nelder–Mead simplex rescue.
//!
//! References:
//! - Levenberg, K. (1944). A method for the solution of certain non-linear problems in least squares. *Quarterly of Applied Mathematics* 2(2), 164–168.
//! - Marquardt, D. W. (1963). An algorithm for least-squares estimation of nonlinear parameters. *Journal of the Society for Industrial and Applied Mathematics* 11(2), 431–441.
//! - Moré, J. J. (1978). The Levenberg–Marquardt algorithm: implementation and theory. In G. A. Watson (ed.), *Numerical Analysis*, Lecture Notes in Mathematics 630, Springer.
//! - Moré, J. J., Garbow, B. S. & Hillstrom, K. E. (1980). *User Guide for MINPACK-1*. Report ANL-80-74, Argonne National Laboratory.
//! - Nocedal, J. & Wright, S. J. (2006). *Numerical Optimization*, 2nd ed. Springer.
//! - Nelder, J. A. & Mead, R. (1965). A simplex method for function minimization. *The Computer Journal* 7(4), 308–313.
//! - Lagarias, J. C., Reeds, J. A., Wright, M. H. & Wright, P. E. (1998). Convergence properties of the Nelder–Mead simplex method in low dimensions. *SIAM Journal on Optimization* 9(1), 112–147.

use curvebook_doc::{ConvergenceStrictness, FitMethod};
use curvebook_math::linalg::{DenseMatrix, qr, qr_col_pivoted};
use curvebook_math::sum::NeumaierSum;

use crate::dsl::ode_fit_budget_exceeded;
use crate::error::{FitError, FitStatus};
use crate::library::ParametricModel;
use crate::solver::inference::obs_y_ss;
use crate::solver::layout::{ObsRecord, ParamBinding, ParameterLayout, model_dataset_index, obs_is_sweep_ordered, ode_budget_error, project_theta};
use crate::solver::results::SolverConfig;

pub(in crate::solver) fn eval_predictions_ordered_orig(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_orig: &[f64],
    flat_params: &mut [f64],
    slot_ds: Option<usize>,
    y_hat_out: &mut [f64],
) -> bool {
    layout.fill_all_params(theta_orig, flat_params);
    eval_predictions_with_flat_params(model, layout, obs, flat_params, slot_ds, y_hat_out)
}

pub(in crate::solver) fn eval_predictions_ordered(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    u: &[f64],
    flat_params: &mut [f64],
    slot_ds: Option<usize>,
    y_hat_out: &mut [f64],
) -> bool {
    if !layout.fill_all_params_u(u, flat_params) {
        return false;
    }
    eval_predictions_with_flat_params(model, layout, obs, flat_params, slot_ds, y_hat_out)
}

fn eval_predictions_with_flat_params(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    flat_params: &[f64],
    slot_ds: Option<usize>,
    y_hat_out: &mut [f64],
) -> bool {
    let p_m = layout.p_model;

    if obs_is_sweep_ordered(obs, slot_ds) {
        for (dst, rec) in y_hat_out.iter_mut().zip(obs.iter()) {
            if slot_ds.is_none_or(|d| d == rec.ds_local_idx) {
                let start = rec.ds_local_idx * p_m;
                let ds_arg = model_dataset_index(model, rec.ds_local_idx, rec.point.data_set_index);
                let Some(pred) = model.eval(&rec.point.x, &flat_params[start..(start + p_m)], ds_arg) else {
                    return false;
                };
                if !pred.is_finite() {
                    return false;
                }
                *dst = pred;
            }
        }
        return true;
    }

    // Evaluate unsorted observations in per-dataset ascending X order so stateful/ODE sweeps reuse
    // integration trajectories across points while writing back into original observation indices.
    let mut order: Vec<usize> = obs.iter().enumerate().filter(|(_, rec)| slot_ds.is_none_or(|d| d == rec.ds_local_idx)).map(|(i, _)| i).collect();
    order.sort_by(|&a, &b| obs[a].ds_local_idx.cmp(&obs[b].ds_local_idx).then_with(|| obs[a].point.x0().total_cmp(&obs[b].point.x0())));
    for idx in order {
        let rec = &obs[idx];
        let start = rec.ds_local_idx * p_m;
        let ds_arg = model_dataset_index(model, rec.ds_local_idx, rec.point.data_set_index);
        let Some(pred) = model.eval(&rec.point.x, &flat_params[start..(start + p_m)], ds_arg) else {
            return false;
        };
        if !pred.is_finite() {
            return false;
        }
        y_hat_out[idx] = pred;
    }
    true
}

/// Evaluate weighted residual vector `r_i = sqrt_w_i * (y_i - f(x_i; theta))` and predictions.
pub(in crate::solver) fn eval_residuals_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    require_positive_mean: bool,
) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    let mut flat_params = vec![0.0_f64; layout.n_datasets * layout.p_model];
    let mut y_hat = vec![0.0_f64; obs.len()];
    if !eval_predictions_ordered(model, layout, obs, theta, &mut flat_params, None, &mut y_hat) {
        return None;
    }
    if require_positive_mean && y_hat.iter().any(|&pred| !pred.is_finite() || pred <= 0.0) {
        return None;
    }

    let mut weighted_res = Vec::with_capacity(obs.len());
    let mut ss = NeumaierSum::new();
    for (rec, &pred) in obs.iter().zip(y_hat.iter()) {
        let r = rec.sqrt_weight * (rec.point.y - pred);
        if !r.is_finite() {
            return None;
        }
        weighted_res.push(r);
        ss.add(r * r);
    }
    let total_ss = ss.total();
    if total_ss.is_finite() { Some((weighted_res, y_hat, total_ss)) } else { None }
}

/// Evaluate weighted residual vector `r_i = sqrt_w_i * (y_i - f(x_i; theta))` and predictions.
pub(in crate::solver) fn eval_residuals(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    eval_residuals_with_domain(model, layout, obs, theta, false)
}

pub(in crate::solver) fn eval_predictions_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_orig: &[f64],
    flat_params: &mut [f64],
    y_hat_out: &mut [f64],
) -> bool {
    eval_predictions_ordered_orig(model, layout, obs, theta_orig, flat_params, None, y_hat_out)
}

pub(in crate::solver) fn mat_get(m: &DenseMatrix, r: usize, c: usize) -> f64 {
    m.get(r, c).unwrap_or(0.0)
}

pub(in crate::solver) fn mat_set(m: &mut DenseMatrix, r: usize, c: usize, val: f64) {
    if let Some(slot) = m.get_mut(r, c) {
        *slot = val;
    }
}

/// Scale-aware finite-difference step `h = eps * |theta_j|` (or `eps` at `0`), capped by
/// `0.25 * (ub - lb)` on narrow box bounds so nanomolar/picomolar parameters and narrow boxes
/// never step out of bounds or distort derivatives (`AUD-03-10`; Nocedal & Wright 2006, Ch. 8).
pub(in crate::solver) fn fd_step_size(xj: f64, eps: f64, lb: Option<f64>, ub: Option<f64>) -> f64 {
    let mut h = if xj != 0.0 && xj.is_finite() { eps * xj.abs() } else { eps };
    if let (Some(l), Some(u)) = (lb, ub)
        && u > l
        && (u - l).is_finite()
    {
        let box_w = u - l;
        if xj == 0.0 && box_w < 1.0 {
            h = eps * box_w;
        }
        h = h.min(0.25 * box_w);
    }
    let min_ulp = 4.0 * f64::EPSILON * xj.abs().max(f64::MIN_POSITIVE);
    if h < min_ulp {
        h = match (lb, ub) {
            (Some(l), Some(u)) if u > l => min_ulp.min(0.25 * (u - l)),
            _ => min_ulp,
        };
    }
    h
}

/// Compute weighted Jacobian `J_{i, j} = sqrt_w_i * df_i / dtheta_j` (`N x K`) using analytical
/// `model.grad` when supported and finite, or scale-aware central/forward finite differences otherwise.
pub(in crate::solver) fn compute_jacobian(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    y_hat0: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
) -> Option<DenseMatrix> {
    let all_slots: Vec<usize> = (0..layout.k_free()).collect();
    compute_jacobian_subset(model, layout, obs, theta, y_hat0, bounds, &all_slots)
}

fn try_analytical_jacobian_subset(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    active_slots: &[usize],
    slot_to_col: &[Option<usize>],
) -> Option<DenseMatrix> {
    let n = obs.len();
    let k_sub = active_slots.len();
    let mut j_mat = DenseMatrix::zeros(n, k_sub).ok()?;
    let p_m = layout.p_model;
    let k_free = layout.k_free();
    let theta_orig = layout.u_to_theta(theta);
    let mut flat_params = vec![0.0_f64; layout.n_datasets * p_m];
    layout.fill_all_params(&theta_orig, &mut flat_params);

    let mut grad_buf = vec![0.0_f64; p_m];
    let mut g_orig_k = vec![0.0_f64; k_free];
    let mut g_u_k = vec![0.0_f64; k_free];
    for (i, rec) in obs.iter().enumerate() {
        grad_buf.fill(0.0);
        let start = rec.ds_local_idx * p_m;
        let ds_arg = model_dataset_index(model, rec.ds_local_idx, rec.point.data_set_index);
        if !model.grad(&rec.point.x, &flat_params[start..(start + p_m)], ds_arg, &mut grad_buf) {
            return None;
        }
        if layout.has_relational {
            g_orig_k.fill(0.0);
            for (p_idx, &g_val) in grad_buf.iter().enumerate().take(p_m) {
                if let ParamBinding::Free(slot) = layout.bindings[rec.ds_local_idx][p_idx] {
                    let next_val = g_orig_k[slot] + g_val;
                    if !g_val.is_finite() || !next_val.is_finite() {
                        return None;
                    }
                    g_orig_k[slot] = next_val;
                }
            }
            layout.apply_chain_rule_grad(&g_orig_k, &mut g_u_k, &theta_orig);
            for (col, &slot) in active_slots.iter().enumerate() {
                let weighted_g = rec.sqrt_weight * g_u_k[slot];
                if !weighted_g.is_finite() {
                    return None;
                }
                mat_set(&mut j_mat, i, col, weighted_g);
            }
        } else {
            for (p_idx, &g_val) in grad_buf.iter().enumerate().take(p_m) {
                if let ParamBinding::Free(slot) = layout.bindings[rec.ds_local_idx][p_idx]
                    && let Some(col) = slot_to_col.get(slot).copied().flatten()
                {
                    let next_val = mat_get(&j_mat, i, col) + rec.sqrt_weight * g_val;
                    if !g_val.is_finite() || !next_val.is_finite() {
                        return None;
                    }
                    mat_set(&mut j_mat, i, col, next_val);
                }
            }
        }
    }
    Some(j_mat)
}

/// Compute weighted Jacobian columns strictly for `active_slots` (`N x active_slots.len()`),
/// never perturbing held-fixed parameters (used by full LM and reduced profile-likelihood LM;
/// `AUD-03-10`). Falls back to finite differences if analytical `model.grad` yields non-finite entries.
pub(in crate::solver) fn compute_jacobian_subset(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    y_hat0: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    active_slots: &[usize],
) -> Option<DenseMatrix> {
    let n = obs.len();
    let k_sub = active_slots.len();
    if k_sub == 0 {
        return DenseMatrix::zeros(n, 0).ok();
    }

    let mut slot_to_col = vec![None; layout.k_free()];
    for (col, &slot) in active_slots.iter().enumerate() {
        if let Some(dst) = slot_to_col.get_mut(slot) {
            *dst = Some(col);
        }
    }

    if let Some(j_mat) = try_analytical_jacobian_subset(model, layout, obs, theta, active_slots, &slot_to_col) {
        return Some(j_mat);
    }

    // Scale-aware central difference Jacobian (with bounded one-sided fallback when near domain/box edges).
    let mut j_mat = DenseMatrix::zeros(n, k_sub).ok()?;
    let mut flat_params = vec![0.0_f64; layout.n_datasets * layout.p_model];
    let eps_central = 1e-6_f64;
    let eps_forward = 1.5e-8_f64;
    let mut work = theta.to_vec();
    let mut yp = vec![0.0_f64; n];
    let mut ym = vec![0.0_f64; n];

    for (col, &slot) in active_slots.iter().enumerate() {
        let info = &layout.free_slots[slot];
        let fd_slot_ds = if layout.has_relational { None } else { info.ds_local_idx };
        let xj = theta[slot];
        let (lb, ub) = bounds[slot];
        let h_c = fd_step_size(xj, eps_central, lb, ub);

        let mut can_plus = h_c > 0.0 && ub.is_none_or(|u| xj + h_c <= u);
        let mut can_minus = h_c > 0.0 && lb.is_none_or(|l| xj - h_c >= l);

        if can_plus {
            work[slot] = xj + h_c;
            can_plus = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, fd_slot_ds, &mut yp);
        }
        if can_minus {
            work[slot] = xj - h_c;
            can_minus = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, fd_slot_ds, &mut ym);
        }
        work[slot] = xj;

        if can_plus && can_minus {
            let inv_2h = 1.0 / (2.0 * h_c);
            for (i, rec) in obs.iter().enumerate() {
                if fd_slot_ds.is_none_or(|d| d == rec.ds_local_idx) {
                    let jv = rec.sqrt_weight * (yp[i] - ym[i]) * inv_2h;
                    if !jv.is_finite() {
                        return None;
                    }
                    mat_set(&mut j_mat, i, col, jv);
                }
            }
            continue;
        }

        // One-sided fallback with smaller scale-aware step `h_f`.
        let h_f = fd_step_size(xj, eps_forward, lb, ub);
        let signs = if (!can_plus && can_minus) || ub.is_some_and(|u| xj + h_f > u) { [-1.0_f64, 1.0_f64] } else { [1.0_f64, -1.0_f64] };
        let mut found_sign = None;
        for &sign in &signs {
            let x_try = xj + sign * h_f;
            if h_f <= 0.0 || lb.is_some_and(|l| x_try < l) || ub.is_some_and(|u| x_try > u) {
                continue;
            }
            work[slot] = x_try;
            let ok = eval_predictions_for_slot_into(model, layout, obs, &work, &mut flat_params, fd_slot_ds, &mut yp);
            work[slot] = xj;
            if ok {
                found_sign = Some(sign);
                break;
            }
        }
        let sign = found_sign?;
        let inv_h = sign / h_f;
        for (i, rec) in obs.iter().enumerate() {
            if fd_slot_ds.is_none_or(|d| d == rec.ds_local_idx) {
                let jv = rec.sqrt_weight * (yp[i] - y_hat0[i]) * inv_h;
                if !jv.is_finite() {
                    return None;
                }
                mat_set(&mut j_mat, i, col, jv);
            }
        }
    }
    Some(j_mat)
}

pub(in crate::solver) fn eval_predictions_for_slot_into(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    flat_params: &mut [f64],
    slot_ds: Option<usize>,
    out: &mut [f64],
) -> bool {
    eval_predictions_ordered(model, layout, obs, theta, flat_params, slot_ds, out)
}

#[derive(Debug, Clone)]
pub(in crate::solver) struct LmRunOutput {
    pub(in crate::solver) theta: Vec<f64>,
    pub(in crate::solver) weighted_res: Vec<f64>,
    pub(in crate::solver) y_hat: Vec<f64>,
    pub(in crate::solver) weighted_ss: f64,
    pub(in crate::solver) j_mat: DenseMatrix,
    pub(in crate::solver) status: FitStatus,
    pub(in crate::solver) iterations: usize,
}

/// Core Moré (1978) trust-region Levenberg–Marquardt solver using QR decomposition on the
/// augmented Jacobian `[J; sqrt(lambda) * D]` (`curvebook_math::linalg::qr`).
///
/// Follows Moré (1978) / MINPACK-1 `lmder` (Moré, Garbow & Hillstrom 1980) for: column scaling
/// `D_j = max(D_j, ||J_{:,j}||)`, initial radius `factor * ||D theta||`, the gain ratio
/// `rho = actual / predicted reduction`, the radius update (shrink by a clamped quadratic
/// `temp` in `[0.1, 0.5]` when `rho < 0.25`, expand to `2 ||D p||` when `rho > 0.75` or
/// `lambda == 0`), step acceptance at `rho >= 1e-4`, and the `ftol`/`xtol`/`gtol` tests.
///
/// Deviations from Moré (1978): the Levenberg–Marquardt parameter is located by the simplified
/// safeguarded iteration in [`solve_trust_region_subproblem`] rather than Moré's Newton
/// iteration; the expansion step uses `max(delta, 2 ||D p||)` (MINPACK sets `2 ||D p||`);
/// and box constraints are handled by projecting each trial point onto the bounds (not part of
/// Moré's unconstrained algorithm), with the predicted reduction evaluated on the projected step.
#[derive(Debug, Clone, Copy)]
pub(in crate::solver) struct LmRunOptions {
    pub(in crate::solver) max_iters: usize,
    pub(in crate::solver) init_factor: f64,
    pub(in crate::solver) require_positive_mean: bool,
    pub(in crate::solver) strictness: ConvergenceStrictness,
}

pub(in crate::solver) fn run_trust_region_lm_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    max_iters: usize,
    init_factor: f64,
    require_positive_mean: bool,
) -> Result<LmRunOutput, FitError> {
    run_trust_region_lm_with_options(
        model,
        layout,
        obs,
        theta_init,
        LmRunOptions { max_iters, init_factor, require_positive_mean, strictness: ConvergenceStrictness::Default },
    )
}

pub(in crate::solver) fn run_trust_region_lm_with_options(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    opts: LmRunOptions,
) -> Result<LmRunOutput, FitError> {
    let LmRunOptions { max_iters, init_factor, require_positive_mean, strictness } = opts;
    let n = obs.len();
    let k = layout.k_free();
    let bounds = layout.u_bounds();
    let mut theta = theta_init.to_vec();
    project_theta(&mut theta, &bounds);

    let Some((mut w_res, mut y_hat, mut ss_curr)) = eval_residuals_with_domain(model, layout, obs, &theta, require_positive_mean) else {
        if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if require_positive_mean && eval_residuals_with_domain(model, layout, obs, &theta, false).is_some() {
            return Err(FitError::ImpossibleWeights {
                reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
            });
        }
        return Err(FitError::BadInitialValues { reason: "Initial parameters evaluate outside model domain".to_owned() });
    };

    if k == 0 {
        return Ok(LmRunOutput {
            theta,
            weighted_res: w_res,
            y_hat,
            weighted_ss: ss_curr,
            j_mat: DenseMatrix::zeros(n, 0)?,
            status: FitStatus::Converged,
            iterations: 0,
        });
    }

    let ftol = 1e-15_f64;
    let xtol = 1e-14_f64;
    let gtol = 1e-13_f64;
    let stall_gtol = 1e-4_f64;
    let y_norm = obs_y_ss(obs).sqrt();

    let mut diag = vec![0.0_f64; k];
    let mut delta = 0.0_f64;
    let mut iter = 0_usize;
    let mut last_max_g_cos = f64::INFINITY;
    let mut j_mat = DenseMatrix::zeros(n, k)?;
    let mut damp_ws = LmDampWorkspace::new(n, k)?;
    let preset_stop = match strictness {
        ConvergenceStrictness::Default => None,
        ConvergenceStrictness::Quick => Some((2_usize, 1e-4_f64)),
        ConvergenceStrictness::Medium => Some((5_usize, 1e-6_f64)),
        ConvergenceStrictness::Strict => Some((5_usize, 1e-10_f64)),
    };
    let mut consecutive_small_ss = 0_usize;

    while iter < max_iters {
        j_mat = match compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds) {
            Some(jm) => jm,
            None => {
                if let Some(err) = ode_budget_error() {
                    return Err(err);
                }
                return Err(FitError::BadInitialValues { reason: "Jacobian evaluation returned non-finite values".to_owned() });
            }
        };

        if ss_curr == 0.0 {
            return Ok(LmRunOutput {
                theta,
                weighted_res: w_res,
                y_hat,
                weighted_ss: ss_curr,
                j_mat,
                status: FitStatus::Converged,
                iterations: iter,
            });
        }
        let max_g_cos = update_lm_diag_and_projected_grad(&j_mat, &w_res, &theta, &bounds, ss_curr, &mut diag);
        if iter == 0 {
            let xnorm = scaled_l2_norm(&diag, &theta);
            delta = init_factor * if xnorm > 0.0 { xnorm } else { 1.0 };
        }
        last_max_g_cos = max_g_cos;
        if max_g_cos <= gtol {
            return Ok(LmRunOutput {
                theta,
                weighted_res: w_res,
                y_hat,
                weighted_ss: ss_curr,
                j_mat,
                status: FitStatus::Converged,
                iterations: iter,
            });
        }

        let mut accepted = false;
        let mut lambda = 0.0_f64;
        for _ in 0..40 {
            let (step, lam_used) = solve_trust_region_subproblem(&j_mat, &w_res, &diag, &theta, delta, lambda, &mut damp_ws)?;
            lambda = lam_used;

            let ctx = LmStepContext { j_mat: &j_mat, w_res: &w_res, diag: &diag, theta: &theta };
            let (theta_trial, actual_step, p_norm) = project_tr_trial(&ctx, &step, &bounds, delta, &mut lambda)?;
            if iter == 0 && p_norm > 0.0 {
                delta = delta.min(p_norm);
            }

            // Predicted reduction: 2 r^T (J p) - ||J p||^2 (valid under active-bound projection too).
            let pred_num = quadratic_pred_num(&j_mat, &w_res, &actual_step).max(0.0);
            let pred_red = if ss_curr > 0.0 { pred_num / ss_curr } else { 0.0 };

            let (act_red, trial_eval) = match eval_residuals_with_domain(model, layout, obs, &theta_trial, require_positive_mean) {
                Some((r_t, y_t, ss_t)) => {
                    let act = if ss_t < ss_curr { 1.0 - ss_t / ss_curr } else { -1.0 };
                    (act, Some((r_t, y_t, ss_t)))
                }
                None => {
                    if let Some(err) = ode_budget_error() {
                        return Err(err);
                    }
                    (-1.0, None)
                }
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
            update_tr_radius_more(&ctx, &outcome, &mut delta, &mut lambda);

            if rho >= 1e-4
                && let Some((r_t, y_t, ss_t)) = trial_eval
            {
                theta = theta_trial;
                w_res = r_t;
                y_hat = y_t;
                ss_curr = ss_t;
                iter += 1;
                accepted = true;

                let preset_converged = if let Some((req_consec, rel_tol)) = preset_stop {
                    if act_red.abs() < rel_tol {
                        consecutive_small_ss += 1;
                    } else {
                        consecutive_small_ss = 0;
                    }
                    consecutive_small_ss >= req_consec
                } else {
                    false
                };

                let raw_xnorm = scaled_l2_norm(&diag, &theta);
                let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
                let ftol_hit = act_red.abs() <= ftol && pred_red <= ftol && rho <= 2.0;
                let xtol_hit = p_norm <= xtol * x_norm;
                if preset_converged || ftol_hit || xtol_hit {
                    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
                    let status = if preset_converged {
                        FitStatus::Converged
                    } else if xtol_hit && !ftol_hit {
                        // A tiny accepted step caused by trust-region collapse at an implicit
                        // domain wall must verify projected-gradient stationarity before declaring
                        // `Converged`.
                        let mut diag_final = diag.clone();
                        let g_cos_final = update_lm_diag_and_projected_grad(&j_final, &w_res, &theta, &bounds, ss_curr, &mut diag_final);
                        stalled_lm_status(g_cos_final, ss_curr, y_norm, stall_gtol)
                    } else {
                        FitStatus::Converged
                    };
                    return Ok(LmRunOutput { theta, weighted_res: w_res, y_hat, weighted_ss: ss_curr, j_mat: j_final, status, iterations: iter });
                }
                break;
            }

            let raw_xnorm = scaled_l2_norm(&diag, &theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            if p_norm <= xtol * x_norm || delta <= 1e-15 * x_norm {
                let status = stalled_lm_status(max_g_cos, ss_curr, y_norm, stall_gtol);
                return Ok(LmRunOutput { theta, weighted_res: w_res, y_hat, weighted_ss: ss_curr, j_mat, status, iterations: iter });
            }
        }

        if !accepted {
            break;
        }
    }

    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
    let status = if iter >= max_iters { FitStatus::HitMaxIters } else { stalled_lm_status(last_max_g_cos, ss_curr, y_norm, stall_gtol) };
    Ok(LmRunOutput { theta, weighted_res: w_res, y_hat, weighted_ss: ss_curr, j_mat: j_final, status, iterations: iter })
}

pub(in crate::solver) fn update_lm_diag_and_projected_grad(
    j_mat: &DenseMatrix,
    w_res: &[f64],
    theta: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    ss_curr: f64,
    diag: &mut [f64],
) -> f64 {
    let n = j_mat.nrows();
    let fnorm = ss_curr.sqrt();
    if !fnorm.is_finite() {
        return f64::INFINITY;
    }
    let mut max_g_cos = 0.0_f64;
    for (j, d_j) in diag.iter_mut().enumerate() {
        let cnorm = j_mat.col_norm2(j);
        if !cnorm.is_finite() {
            return f64::INFINITY;
        }
        let eff = if cnorm > f64::MIN_POSITIVE.sqrt() { cnorm } else { 1.0 };
        *d_j = if *d_j == 0.0 { eff } else { d_j.max(eff) };
        if cnorm > 0.0 && fnorm > 0.0 {
            // Compute normalized cosine sum_i (J_{i,j} / ||J_{:,j}||) * (r_i / ||r||) directly so
            // large Jacobian entries (|J| > 1.34e154) never overflow `jv * jv` or `jv * w_r` (`AUD-03-10`).
            let mut cos_acc = NeumaierSum::new();
            let inv_cn = 1.0 / cnorm;
            let inv_fn = 1.0 / fnorm;
            for (i, &w_r) in w_res.iter().enumerate().take(n) {
                let jv = mat_get(j_mat, i, j);
                if !jv.is_finite() || !w_r.is_finite() {
                    return f64::INFINITY;
                }
                cos_acc.add((jv * inv_cn) * (w_r * inv_fn));
            }
            let cos_j = cos_acc.total();
            if !cos_j.is_finite() {
                return f64::INFINITY;
            }
            let at_lo = bounds[j].0.is_some_and(|l| (theta[j] - l).abs() <= 1e-12 * l.abs().max(f64::MIN_POSITIVE) && cos_j < 0.0);
            let at_hi = bounds[j].1.is_some_and(|u| (u - theta[j]).abs() <= 1e-12 * u.abs().max(f64::MIN_POSITIVE) && cos_j > 0.0);
            if !at_lo && !at_hi {
                max_g_cos = max_g_cos.max(cos_j.abs());
            }
        }
    }
    max_g_cos
}

pub(in crate::solver) fn stalled_lm_status(max_g_cos: f64, ss_curr: f64, y_norm: f64, stall_gtol: f64) -> FitStatus {
    if ss_curr == 0.0 {
        return FitStatus::Converged;
    }
    let fnorm = ss_curr.sqrt();
    let eff_g = if max_g_cos.is_finite() { max_g_cos * (fnorm / fnorm.max(1e-8 * y_norm)) } else { f64::INFINITY };
    if eff_g <= stall_gtol { FitStatus::Converged } else { FitStatus::Diverged }
}

pub(in crate::solver) struct LmStepContext<'a> {
    pub(in crate::solver) j_mat: &'a DenseMatrix,
    pub(in crate::solver) w_res: &'a [f64],
    pub(in crate::solver) diag: &'a [f64],
    pub(in crate::solver) theta: &'a [f64],
}

pub(in crate::solver) struct TrStepOutcome<'a> {
    pub(in crate::solver) actual_step: &'a [f64],
    pub(in crate::solver) p_norm: f64,
    pub(in crate::solver) ss_curr: f64,
    pub(in crate::solver) ss_trial: Option<f64>,
    pub(in crate::solver) rho: f64,
}

pub(in crate::solver) fn project_tr_trial(
    ctx: &LmStepContext<'_>,
    step: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    delta: f64,
    lambda: &mut f64,
) -> Result<(Vec<f64>, Vec<f64>, f64), FitError> {
    let mut trial: Vec<f64> = ctx.theta.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
    project_theta(&mut trial, bounds);
    let mut actual_step: Vec<f64> = trial.iter().zip(ctx.theta.iter()).map(|(a, b)| a - b).collect();
    let mut p_norm = scaled_l2_norm(ctx.diag, &actual_step);
    let needs_face_step = has_active_bound_step(ctx.theta, step, bounds)
        && (p_norm <= 1e-14 * delta.max(1.0) || quadratic_pred_num(ctx.j_mat, ctx.w_res, &actual_step) <= 0.0);
    if needs_face_step
        && let Some((face_step, face_lam)) = solve_active_face_subproblem(ctx.j_mat, ctx.w_res, ctx.diag, ctx.theta, step, bounds, delta, *lambda)?
    {
        let mut face_trial: Vec<f64> = ctx.theta.iter().zip(face_step.iter()).map(|(a, b)| a + b).collect();
        project_theta(&mut face_trial, bounds);
        let face_actual: Vec<f64> = face_trial.iter().zip(ctx.theta.iter()).map(|(a, b)| a - b).collect();
        let face_norm = scaled_l2_norm(ctx.diag, &face_actual);
        if face_norm > 0.0 {
            trial = face_trial;
            actual_step = face_actual;
            p_norm = face_norm;
            *lambda = face_lam;
        }
    }
    Ok((trial, actual_step, p_norm))
}

pub(in crate::solver) fn update_tr_radius_more(ctx: &LmStepContext<'_>, outcome: &TrStepOutcome<'_>, delta: &mut f64, lambda: &mut f64) {
    if outcome.rho < 0.25 {
        let mut temp = 0.5_f64;
        if let Some(ss_t) = outcome.ss_trial
            && ss_t > outcome.ss_curr
            && let Ok(jp) = ctx.j_mat.matvec(outcome.actual_step)
        {
            let r_dot_jp: f64 = ctx.w_res.iter().zip(jp.iter()).map(|(r, j)| r * j).sum();
            let dir_deriv = -r_dot_jp;
            let ratio_t = 0.5 * dir_deriv / (dir_deriv + 0.5 * (outcome.ss_curr - ss_t));
            if ratio_t.is_finite() {
                temp = ratio_t.clamp(0.1, 0.5);
            }
        }
        let bound_scale = if *lambda == 0.0 { outcome.p_norm } else { outcome.p_norm * 10.0 };
        *delta = temp * delta.min(bound_scale).max(*delta * 0.1);
        if *lambda == 0.0 {
            let raw_xnorm = scaled_l2_norm(ctx.diag, ctx.theta);
            let x_norm = if raw_xnorm > 0.0 { raw_xnorm } else { 1.0 };
            *delta = delta.min(0.8 * outcome.p_norm).max(1e-15 * x_norm);
        }
    } else if *lambda == 0.0 || outcome.rho > 0.75 {
        *delta = delta.max(2.0 * outcome.p_norm);
        *lambda *= 0.5;
    }
}

pub(in crate::solver) struct LmDampWorkspace {
    pub(in crate::solver) aug: DenseMatrix,
    pub(in crate::solver) rhs: Vec<f64>,
    pub(in crate::solver) col_scales: Vec<f64>,
}

impl LmDampWorkspace {
    pub(in crate::solver) fn new(n: usize, k: usize) -> Result<Self, FitError> {
        Ok(Self { aug: DenseMatrix::zeros(n + k, k)?, rhs: vec![0.0_f64; n + k], col_scales: vec![1.0_f64; k] })
    }
}

fn equilibrate_jacobian(j_mat: &DenseMatrix) -> Result<(DenseMatrix, Vec<f64>), FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let mut scales = vec![0.0_f64; k];
    let mut j_eq = DenseMatrix::zeros(n, k)?;
    for (j, sc) in scales.iter_mut().enumerate() {
        let cn = j_mat.col_norm2(j);
        *sc = cn;
        if cn > 0.0 && cn.is_finite() {
            let inv = 1.0 / cn;
            for i in 0..n {
                mat_set(&mut j_eq, i, j, mat_get(j_mat, i, j) * inv);
            }
        }
    }
    Ok((j_eq, scales))
}

/// Solve the Moré trust-region subproblem `(J^T J + lambda D^2) p = J^T r` with `||D p|| <= delta`
/// via column-equilibrated QR decomposition of `J` and `[J; sqrt(lambda) D]`.
///
/// Follows Moré (1978, *Numerical Analysis*, Lecture Notes in Math. 630) Algorithm 4.1 /
/// MINPACK-1 `lmpar`:
/// 1. Equilibrates columns of `J` by Euclidean norm before `qr_col_pivoted` so mixed parameter
///    units never trigger false rank deficiency (`pqr.rank() < k`).
/// 2. Accepts the full-rank Gauss–Newton step (`lambda = 0`) whenever `||D p_gn|| <= 1.1 delta`.
/// 3. Computes exact upper bound `paru = ||D^{-1} J^T r|| / delta` and lower bound `parl` via
///    `R_eq^T s = P^T ((D^2 p_gn / ||D p_gn||) / col_scale)` when `J` has full column rank (`0` otherwise).
/// 4. Refines `lambda` using Moré's rational-model Newton update `parc = phi / (delta * ||s||^2)`
///    where `R_eq^T s = (D^2 p / ||D p||) / col_scale` from the equilibrated QR factor `R_eq` of `[J; sqrt(lambda) D]`.
pub(in crate::solver) fn solve_trust_region_subproblem(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    _theta: &[f64],
    delta: f64,
    lambda_hint: f64,
    ws: &mut LmDampWorkspace,
) -> Result<(Vec<f64>, f64), FitError> {
    let k = j_mat.ncols();
    let (j_eq, col_scales) = equilibrate_jacobian(j_mat)?;
    let pqr = qr_col_pivoted(&j_eq)?;
    let mut parl = 0.0_f64;
    let mut gn_norm_for_init = 0.0_f64;
    if pqr.rank() == k
        && col_scales.iter().all(|&c| c > 0.0 && c.is_finite())
        && let Ok(mut p_gn) = pqr.solve_basic(r)
    {
        for (pj, &cj) in p_gn.iter_mut().zip(col_scales.iter()) {
            *pj /= cj;
        }
        if p_gn.iter().all(|v| v.is_finite()) {
            let p_norm = scaled_l2_norm(diag, &p_gn);
            if p_norm <= 1.1 * delta {
                return Ok((p_gn, 0.0));
            }
            gn_norm_for_init = p_norm;
            if let Ok(r_gn) = pqr.r()
                && let Some(s_norm_sq) = more_r_inv_t_norm_sq(&r_gn, diag, &p_gn, p_norm, Some(pqr.permutation()), Some(&col_scales))
            {
                parl = ((p_norm - delta) / (delta.max(f64::MIN_POSITIVE) * s_norm_sq)).max(0.0);
            }
        }
    }

    // Scaled gradient norm ||D^{-1} J^T r||_2 and upper bracket paru = ||D^{-1} J^T r|| / delta.
    let mut grad_scaled_ss = 0.0_f64;
    for (j, &dj) in diag.iter().enumerate().take(k) {
        let mut g_j = 0.0_f64;
        for (i, &ri) in r.iter().enumerate() {
            g_j += mat_get(j_mat, i, j) * ri;
        }
        let s = g_j / dj;
        grad_scaled_ss += s * s;
    }
    let gnorm = grad_scaled_ss.sqrt();
    if gnorm == 0.0 || !gnorm.is_finite() {
        return Ok((vec![0.0_f64; k], 0.0));
    }
    let raw_paru = gnorm / delta.max(f64::MIN_POSITIVE);
    let mut paru = if raw_paru.is_finite() { raw_paru.clamp(1e-12, 1e150) } else { 1e150 };
    // When rank-deficient (parl == 0), keep sqrt(lambda) safely above QR rounding noise in nullspace.
    let lam_floor = if pqr.rank() < k { (1e-10 * paru).max(1e-12).min(paru) } else { (1e-14 * paru).max(1e-24).min(paru) };
    let mut parl = parl.min(paru);
    let mut lam = lambda_hint.max(parl).min(paru);
    if lam < lam_floor {
        let fallback = if gn_norm_for_init > 0.0 { gnorm / gn_norm_for_init } else { 0.001 * paru };
        lam = fallback.clamp(parl.max(lam_floor), paru);
    }

    let mut best_p = vec![0.0_f64; k];
    let mut fp_prev = 0.0_f64;
    for iter in 0..10 {
        if lam < lam_floor {
            lam = (0.001 * paru).max(parl).max(lam_floor).min(paru);
        }
        let (p, r_mat) = solve_damped_qr_with_r(j_mat, r, diag, lam, ws)?;
        let p_norm = scaled_l2_norm(diag, &p);
        let fp = p_norm - delta;
        best_p = p;
        if fp.abs() <= 0.1 * delta || (parl == 0.0 && fp <= fp_prev && fp_prev < 0.0) || iter == 9 {
            return Ok((best_p, lam));
        }
        let parc = match more_r_inv_t_norm_sq(&r_mat, diag, &best_p, p_norm, None, Some(&ws.col_scales)) {
            Some(s_norm_sq) => fp / (delta.max(f64::MIN_POSITIVE) * s_norm_sq),
            None => 0.0,
        };
        if fp > 0.0 {
            parl = parl.max(lam);
        } else if fp < 0.0 {
            paru = paru.min(lam);
        }
        let next_lam = parl.max(lam + parc).clamp(lam_floor.min(paru), paru);
        if (next_lam - lam).abs() <= 1e-12 * lam.max(1e-30) || (paru - parl) <= 1e-12 * paru.max(1e-30) {
            return Ok((best_p, next_lam));
        }
        lam = next_lam;
        fp_prev = fp;
    }
    Ok((best_p, lam))
}

pub(in crate::solver) fn solve_damped_qr_with_r(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    lambda: f64,
    ws: &mut LmDampWorkspace,
) -> Result<(Vec<f64>, DenseMatrix), FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let sqrt_lam = lambda.sqrt();
    for j in 0..k {
        let dj = diag.get(j).copied().unwrap_or(1.0);
        let raw_sj = j_mat.col_norm2(j).hypot(sqrt_lam * dj);
        let sj = if raw_sj > 0.0 && raw_sj.is_finite() { raw_sj } else { 1.0 };
        ws.col_scales[j] = sj;
        let inv_sj = 1.0 / sj;
        for i in 0..n {
            mat_set(&mut ws.aug, i, j, mat_get(j_mat, i, j) * inv_sj);
        }
        for i in 0..k {
            mat_set(&mut ws.aug, n + i, j, if i == j { sqrt_lam * dj * inv_sj } else { 0.0 });
        }
    }
    ws.rhs[..n].copy_from_slice(r);
    ws.rhs[n..(n + k)].fill(0.0);

    let qr_res = qr(&ws.aug)?;
    let mut p = qr_res.solve_basic(&ws.rhs)?;
    let r_mat = qr_res.r()?;
    for (pj, &sj) in p.iter_mut().zip(ws.col_scales.iter()) {
        *pj /= sj;
    }
    Ok((p, r_mat))
}

pub(in crate::solver) fn more_r_inv_t_norm_sq(
    r_mat: &DenseMatrix,
    diag: &[f64],
    p: &[f64],
    p_norm: f64,
    perm: Option<&[usize]>,
    col_scales: Option<&[f64]>,
) -> Option<f64> {
    if p_norm <= 0.0 || !p_norm.is_finite() {
        return None;
    }
    let k = diag.len();
    let mut s = vec![0.0_f64; k];
    for j in 0..k {
        let orig_j = perm.map_or(j, |pm| pm[j]);
        let mut sum = (diag[orig_j] * diag[orig_j] * p[orig_j]) / p_norm;
        if let Some(cs) = col_scales {
            let c = cs[orig_j];
            if !c.is_finite() || c <= 0.0 {
                return None;
            }
            sum /= c;
        }
        for (i, &si) in s.iter().enumerate().take(j) {
            sum -= mat_get(r_mat, i, j) * si;
        }
        let rjj = mat_get(r_mat, j, j);
        if !rjj.is_finite() || rjj.abs() <= 1e-300 {
            return None;
        }
        s[j] = sum / rjj;
    }
    let norm_sq: f64 = s.iter().map(|v| v * v).sum();
    (norm_sq.is_finite() && norm_sq > 0.0).then_some(norm_sq)
}

pub(in crate::solver) fn quadratic_pred_num(j_mat: &DenseMatrix, w_res: &[f64], step: &[f64]) -> f64 {
    let Ok(jp) = j_mat.matvec(step) else {
        return 0.0;
    };
    let jp_ss: f64 = jp.iter().map(|v| v * v).sum();
    let r_dot_jp: f64 = w_res.iter().zip(jp.iter()).map(|(r, j)| r * j).sum();
    2.0 * r_dot_jp - jp_ss
}

pub(in crate::solver) fn is_bound_actively_blocked(theta_j: f64, step_j: f64, bound: (Option<f64>, Option<f64>)) -> bool {
    let at_lo = bound.0.is_some_and(|lo| {
        let tol = 1e-12 * lo.abs().max(f64::MIN_POSITIVE);
        theta_j <= lo + tol && step_j < 0.0
    });
    let at_hi = bound.1.is_some_and(|hi| {
        let tol = 1e-12 * hi.abs().max(f64::MIN_POSITIVE);
        theta_j >= hi - tol && step_j > 0.0
    });
    at_lo || at_hi
}

pub(in crate::solver) fn has_active_bound_step(theta: &[f64], step: &[f64], bounds: &[(Option<f64>, Option<f64>)]) -> bool {
    theta.iter().zip(step.iter()).zip(bounds.iter()).any(|((&tj, &sj), &b)| is_bound_actively_blocked(tj, sj, b))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::solver) fn solve_active_face_subproblem(
    j_mat: &DenseMatrix,
    r: &[f64],
    diag: &[f64],
    theta: &[f64],
    unconstrained_step: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
    delta: f64,
    lambda_hint: f64,
) -> Result<Option<(Vec<f64>, f64)>, FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    let free_cols: Vec<usize> = (0..k).filter(|&j| !is_bound_actively_blocked(theta[j], unconstrained_step[j], bounds[j])).collect();
    if free_cols.is_empty() || free_cols.len() == k {
        return Ok(None);
    }
    let kf = free_cols.len();
    let mut j_free = DenseMatrix::zeros(n, kf)?;
    let mut diag_free = Vec::with_capacity(kf);
    let mut theta_free = Vec::with_capacity(kf);
    for (col_idx, &orig_j) in free_cols.iter().enumerate() {
        diag_free.push(diag[orig_j]);
        theta_free.push(theta[orig_j]);
        for i in 0..n {
            mat_set(&mut j_free, i, col_idx, mat_get(j_mat, i, orig_j));
        }
    }
    let mut ws_free = LmDampWorkspace::new(n, kf)?;
    let (sub_step, sub_lam) = solve_trust_region_subproblem(&j_free, r, &diag_free, &theta_free, delta, lambda_hint, &mut ws_free)?;
    let mut full_step = vec![0.0_f64; k];
    for (col_idx, &orig_j) in free_cols.iter().enumerate() {
        full_step[orig_j] = sub_step[col_idx];
    }
    Ok(Some((full_step, sub_lam)))
}

pub(in crate::solver) fn scaled_l2_norm(diag: &[f64], v: &[f64]) -> f64 {
    let mut s = NeumaierSum::new();
    for (&d, &x) in diag.iter().zip(v.iter()) {
        let prod = d * x;
        s.add(prod * prod);
    }
    s.total().sqrt()
}

pub(in crate::solver) fn run_trust_region_lm(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    max_iters: usize,
    init_factor: f64,
) -> Result<LmRunOutput, FitError> {
    run_trust_region_lm_with_domain(model, layout, obs, theta_init, max_iters, init_factor, false)
}

/// Nelder–Mead simplex warm-start / local rescue for difficult starts or non-finite initial points.
///
/// Nelder & Mead (1965) with standard coefficients (reflection 1, expansion 2, contraction 0.5,
/// shrink 0.5) and the ordering, greedy-expansion, and outside/inside contraction rules of
/// Lagarias et al. (1998). Box constraints are enforced by projecting vertices before
/// evaluation (a pragmatic extension, not part of either reference). Termination is on the
/// relative spread of objective values or the evaluation budget only.
pub(in crate::solver) fn nelder_mead_optimize<F>(
    layout: &ParameterLayout,
    seed: &[f64],
    max_evals: usize,
    zero_step: f64,
    mut eval_obj: F,
) -> Option<Vec<f64>>
where
    F: FnMut(&[f64]) -> f64,
{
    let k = layout.k_free();
    if k == 0 {
        return Some(Vec::new());
    }
    let bounds = layout.u_bounds();

    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(k + 1);
    let mut v0 = seed.to_vec();
    project_theta(&mut v0, &bounds);
    let f0 = eval_obj(&v0);
    if ode_fit_budget_exceeded().is_some() {
        return None;
    }
    simplex.push((v0.clone(), f0));

    for j in 0..k {
        let mut vj = v0.clone();
        let step = if vj[j].abs() > 1e-8 { 0.1 * vj[j] } else { zero_step };
        vj[j] = v0[j] + step;
        project_theta(&mut vj, &bounds);
        if (vj[j] - v0[j]).abs() <= 1e-15 * v0[j].abs().max(1.0) {
            vj[j] = v0[j] - step;
            project_theta(&mut vj, &bounds);
        }
        if (vj[j] - v0[j]).abs() <= 1e-15 * v0[j].abs().max(1.0)
            && let (Some(l), Some(u)) = bounds[j]
            && u > l
        {
            vj[j] = 0.5 * (l + u);
        }
        let fj = eval_obj(&vj);
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        simplex.push((vj, fj));
    }

    let mut evals = k + 1;
    while evals < max_evals {
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
        if simplex[0].1.is_finite() && simplex[k].1.is_finite() {
            let spread = (simplex[k].1 - simplex[0].1).abs();
            if spread <= 1e-12 * simplex[0].1.abs().max(1.0) {
                break;
            }
        }

        let mut centroid = vec![0.0_f64; k];
        for (v, _) in &simplex[..k] {
            for (c, &x) in centroid.iter_mut().zip(v.iter()) {
                *c += x;
            }
        }
        for c in &mut centroid {
            *c /= k as f64;
        }

        let worst = &simplex[k].0;
        let mut xr: Vec<f64> = centroid.iter().zip(worst.iter()).map(|(c, w)| c + (c - w)).collect();
        project_theta(&mut xr, &bounds);
        let fr = eval_obj(&xr);
        evals += 1;

        if fr < simplex[0].1 {
            let mut xe: Vec<f64> = centroid.iter().zip(xr.iter()).map(|(c, r)| c + 2.0 * (r - c)).collect();
            project_theta(&mut xe, &bounds);
            let fe = eval_obj(&xe);
            evals += 1;
            simplex[k] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < simplex[k - 1].1 {
            simplex[k] = (xr, fr);
        } else {
            let use_outside = fr < simplex[k].1;
            let base = if use_outside { &xr } else { &simplex[k].0 };
            let f_base = if use_outside { fr } else { simplex[k].1 };
            let mut xc: Vec<f64> = centroid.iter().zip(base.iter()).map(|(c, b)| c + 0.5 * (b - c)).collect();
            project_theta(&mut xc, &bounds);
            let fc = eval_obj(&xc);
            evals += 1;
            if fc <= f_base {
                simplex[k] = (xc, fc);
            } else {
                let best = simplex[0].0.clone();
                for vert in simplex.iter_mut().take(k + 1).skip(1) {
                    for (vj, &b) in vert.0.iter_mut().zip(best.iter()) {
                        *vj = b + 0.5 * (*vj - b);
                    }
                    project_theta(&mut vert.0, &bounds);
                    vert.1 = eval_obj(&vert.0);
                    evals += 1;
                }
            }
        }
    }

    simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
    if simplex[0].1.is_finite() {
        let mut best = simplex[0].0.clone();
        project_theta(&mut best, &bounds);
        Some(best)
    } else {
        None
    }
}

/// Nelder–Mead simplex warm-start / local rescue for difficult starts or non-finite initial points.
///
/// Nelder & Mead (1965) with standard coefficients (reflection 1, expansion 2, contraction 0.5,
/// shrink 0.5) and the ordering, greedy-expansion, and outside/inside contraction rules of
/// Lagarias et al. (1998). Box constraints are enforced by projecting vertices before
/// evaluation (a pragmatic extension, not part of either reference). Termination is on the
/// relative spread of objective values or the evaluation budget only.
pub(in crate::solver) fn nelder_mead_rescue_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    max_evals: usize,
    require_positive_mean: bool,
) -> Option<Vec<f64>> {
    let bounds = layout.u_bounds();
    nelder_mead_optimize(layout, seed, max_evals, 0.05, |th| {
        if ode_fit_budget_exceeded().is_some() {
            return f64::INFINITY;
        }
        let mut clamped = th.to_vec();
        project_theta(&mut clamped, &bounds);
        eval_residuals_with_domain(model, layout, obs, &clamped, require_positive_mean).map_or(f64::INFINITY, |(_, _, ss)| ss)
    })
}

pub(in crate::solver) fn nelder_mead_poisson_feasibility_rescue(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
) -> Option<Vec<f64>> {
    let bounds = layout.u_bounds();
    let y_scale = obs.iter().map(|o| o.point.y.abs()).fold(0.0_f64, f64::max).max(1e-6);
    let eps_pos = 1e-4 * y_scale;
    let cand = nelder_mead_optimize(layout, seed, 400, 1.0, |th| {
        if ode_fit_budget_exceeded().is_some() {
            return f64::INFINITY;
        }
        let mut clamped = th.to_vec();
        project_theta(&mut clamped, &bounds);
        match eval_residuals_with_domain(model, layout, obs, &clamped, false) {
            Some((_, y_hat, ss)) => {
                let mut penalty = 0.0_f64;
                let mut all_pos = true;
                for &mu in &y_hat {
                    if mu <= 0.0 {
                        all_pos = false;
                    }
                    if mu < eps_pos {
                        let d = eps_pos - mu;
                        penalty += d * d;
                    }
                }
                if all_pos { ss } else { 1e50 + 1e30 * penalty + ss.min(1e20) }
            }
            None => f64::INFINITY,
        }
    })?;
    eval_residuals_with_domain(model, layout, obs, &cand, true).map(|_| cand)
}

/// Multi-factor trust-region LM driver with optional simplex rescue for hard starts.
pub(in crate::solver) fn solve_with_warm_start(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    solve_with_warm_start_with_domain(model, layout, obs, theta_init, config, config.method == FitMethod::PoissonDeviance)
}

pub(in crate::solver) fn solve_with_warm_start_with_domain(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    config: &SolverConfig,
    require_positive_mean: bool,
) -> Result<LmRunOutput, FitError> {
    let mut start = theta_init.to_vec();
    if eval_residuals_with_domain(model, layout, obs, &start, require_positive_mean).is_none() {
        if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if config.use_simplex_warm_start
            && let Some(rescued) = nelder_mead_rescue_with_domain(model, layout, obs, &start, 240, require_positive_mean)
        {
            start = rescued;
        } else if let Some(err) = ode_budget_error() {
            return Err(err);
        } else if require_positive_mean && eval_residuals_with_domain(model, layout, obs, &start, false).is_some() {
            if !config.use_simplex_warm_start && layout.k_free() > 0 {
                return Err(FitError::BadInitialValues {
                    reason: "Starting parameters produce non-positive Poisson fitted means (mu <= 0)".to_owned(),
                });
            }
            return Err(FitError::ImpossibleWeights {
                reason: "Poisson deviance regression requires strictly positive fitted means (mu > 0)".to_owned(),
            });
        } else {
            return Err(FitError::BadInitialValues { reason: "Starting parameters evaluate outside model domain".to_owned() });
        }
    }

    let make_opts = |init_factor: f64| LmRunOptions {
        max_iters: config.max_iters,
        init_factor,
        require_positive_mean,
        strictness: config.convergence_strictness,
    };
    let mut best = run_trust_region_lm_with_options(model, layout, obs, &start, make_opts(100.0))?;
    // If primary run hit max iters or suspiciously poor gradient / flat plateau, try smaller step
    // trust-region factor `1.0` and optional Nelder-Mead restart purely from `start` or `best.theta`.
    if config.use_simplex_warm_start && (best.status != FitStatus::Converged || best.weighted_ss > 1e-3 * obs_y_ss(obs)) {
        match run_trust_region_lm_with_options(model, layout, obs, &start, make_opts(1.0)) {
            Ok(alt1)
                if alt1.weighted_ss < best.weighted_ss
                    || (best.status != FitStatus::Converged && alt1.status == FitStatus::Converged && alt1.weighted_ss <= best.weighted_ss) =>
            {
                best = alt1;
            }
            Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
            _ => {}
        }
        if let Some(nm) = nelder_mead_rescue_with_domain(model, layout, obs, &best.theta, 400, require_positive_mean) {
            match run_trust_region_lm_with_options(model, layout, obs, &nm, make_opts(1.0)) {
                Ok(alt2)
                    if alt2.weighted_ss < best.weighted_ss
                        || (best.status != FitStatus::Converged && alt2.status == FitStatus::Converged && alt2.weighted_ss <= best.weighted_ss) =>
                {
                    best = alt2;
                }
                Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                _ => {}
            }
        } else if let Some(err) = ode_budget_error() {
            return Err(err);
        }
        if best.weighted_ss > 1e-2 * obs_y_ss(obs) || best.status != FitStatus::Converged {
            if let Some(nm_start) = nelder_mead_rescue_with_domain(model, layout, obs, &start, 600, require_positive_mean) {
                match run_trust_region_lm_with_options(model, layout, obs, &nm_start, make_opts(0.5)) {
                    Ok(alt3)
                        if alt3.weighted_ss < best.weighted_ss
                            || (best.status != FitStatus::Converged
                                && alt3.status == FitStatus::Converged
                                && alt3.weighted_ss <= best.weighted_ss) =>
                    {
                        best = alt3;
                    }
                    Err(err @ FitError::OdeIntegrationFailed { .. }) => return Err(err),
                    _ => {}
                }
            } else if let Some(err) = ode_budget_error() {
                return Err(err);
            }
        }
    }
    Ok(best)
}
