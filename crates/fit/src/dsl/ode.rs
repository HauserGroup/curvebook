//! Per-fit ODE step budget, the trajectory cache, and adaptive Dormand–Prince RK45 integration.
//!
//! References:
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae. *Journal of Computational and Applied Mathematics* 6(1), 19–26.
//! - Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I: Nonstiff Problems*, 2nd rev. ed. Springer.

use std::cell::RefCell;
use std::sync::atomic::AtomicU64;

use crate::dsl::limits::{MAX_CACHED_ODE_KNOTS, MAX_CACHED_ODE_TRAJECTORIES, MAX_ODE_STEPS};

#[derive(Debug, Clone, Copy)]
pub(in crate::dsl) struct OdeFitBudgetState {
    pub(in crate::dsl) used_steps: usize,
    pub(in crate::dsl) max_steps: usize,
    pub(in crate::dsl) exceeded: bool,
}

std::thread_local! {
    static ACTIVE_ODE_BUDGET: RefCell<Option<OdeFitBudgetState>> = const { RefCell::new(None) };
}

/// RAII scope bounding total adaptive RK45 steps across a single nonlinear fit execution.
pub(crate) struct OdeFitBudgetGuard {
    pub(in crate::dsl) prev: Option<OdeFitBudgetState>,
}

impl Drop for OdeFitBudgetGuard {
    fn drop(&mut self) {
        let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
            if let Ok(mut cell) = slot.try_borrow_mut() {
                *cell = self.prev;
            }
        });
    }
}

pub(crate) fn enter_ode_fit_budget(max_steps: usize) -> OdeFitBudgetGuard {
    let mut prev = None;
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(mut cell) = slot.try_borrow_mut() {
            prev = cell.replace(OdeFitBudgetState { used_steps: 0, max_steps, exceeded: false });
        }
    });
    OdeFitBudgetGuard { prev }
}

pub(crate) fn ode_fit_budget_exceeded() -> Option<usize> {
    let mut out = None;
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(cell) = slot.try_borrow()
            && let Some(state) = *cell
            && state.exceeded
        {
            out = Some(state.max_steps);
        }
    });
    out
}

pub(in crate::dsl) fn consume_ode_fit_step() -> bool {
    let mut allowed = true;
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(mut cell) = slot.try_borrow_mut()
            && let Some(state) = cell.as_mut()
        {
            if state.exceeded || state.used_steps >= state.max_steps {
                state.exceeded = true;
                allowed = false;
            } else {
                state.used_steps += 1;
            }
        }
    });
    allowed
}

#[derive(Debug, Clone)]
pub(in crate::dsl) struct OdeKnot {
    pub(in crate::dsl) dist: f64,
    pub(in crate::dsl) t: f64,
    pub(in crate::dsl) y: f64,
    pub(in crate::dsl) h: f64,
}

#[derive(Debug, Clone, Default)]
pub(in crate::dsl) struct TrajectorySide {
    pub(in crate::dsl) knots: Vec<OdeKnot>,
    pub(in crate::dsl) steps: usize,
    pub(in crate::dsl) failed_beyond_dist: Option<f64>,
}

impl TrajectorySide {
    pub(in crate::dsl) fn thin_if_needed(&mut self) {
        if self.knots.len() <= MAX_CACHED_ODE_KNOTS {
            return;
        }
        let n = self.knots.len();
        let mut thinned = Vec::with_capacity(MAX_CACHED_ODE_KNOTS / 2 + 2);
        for (idx, knot) in self.knots.iter().enumerate() {
            if idx == 0 || idx + 1 == n || idx % 2 == 0 {
                thinned.push(knot.clone());
            }
        }
        self.knots = thinned;
    }

    pub(in crate::dsl) fn insert_knots(&mut self, insert_at: usize, new_knots: Vec<OdeKnot>) {
        if new_knots.is_empty() {
            return;
        }
        self.knots.splice(insert_at..insert_at, new_knots);
        self.thin_if_needed();
    }
}

#[derive(Debug, Clone)]
pub(in crate::dsl) struct CachedOdeTrajectory {
    pub(in crate::dsl) model_id: u64,
    pub(in crate::dsl) ds_idx: usize,
    pub(in crate::dsl) t0: f64,
    pub(in crate::dsl) y0: f64,
    pub(in crate::dsl) params: Vec<f64>,
    pub(in crate::dsl) x_tail: Vec<f64>,
    pub(in crate::dsl) pos: TrajectorySide,
    pub(in crate::dsl) neg: TrajectorySide,
}

pub(in crate::dsl) fn slice_bits_eq(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
}

impl CachedOdeTrajectory {
    pub(in crate::dsl) fn new(model_id: u64, ds_idx: usize, t0: f64, y0: f64, params: &[f64], x_tail: &[f64]) -> Self {
        let init_knot = OdeKnot { dist: 0.0, t: t0, y: y0, h: 0.0 };
        Self {
            model_id,
            ds_idx,
            t0,
            y0,
            params: params.to_vec(),
            x_tail: x_tail.to_vec(),
            pos: TrajectorySide { knots: vec![init_knot.clone()], steps: 0, failed_beyond_dist: None },
            neg: TrajectorySide { knots: vec![init_knot], steps: 0, failed_beyond_dist: None },
        }
    }

    pub(in crate::dsl) fn matches(&self, model_id: u64, ds_idx: usize, t0: f64, y0: f64, params: &[f64], x_tail: &[f64]) -> bool {
        self.model_id == model_id
            && self.ds_idx == ds_idx
            && self.t0.to_bits() == t0.to_bits()
            && self.y0.to_bits() == y0.to_bits()
            && slice_bits_eq(&self.params, params)
            && slice_bits_eq(&self.x_tail, x_tail)
    }
}

pub(in crate::dsl) static NEXT_ODE_MODEL_ID: AtomicU64 = AtomicU64::new(1);

std::thread_local! {
    static ODE_TRAJECTORY_CACHE: RefCell<Vec<CachedOdeTrajectory>> = const { RefCell::new(Vec::new()) };
}

pub(in crate::dsl) fn take_cached_ode_trajectory(
    model_id: u64,
    ds_idx: usize,
    t0: f64,
    y0: f64,
    params: &[f64],
    x_tail: &[f64],
) -> CachedOdeTrajectory {
    let mut found = None;
    let _ = ODE_TRAJECTORY_CACHE.try_with(|slot| {
        if let Ok(mut entries) = slot.try_borrow_mut()
            && let Some(pos) = entries.iter().position(|e| e.matches(model_id, ds_idx, t0, y0, params, x_tail))
        {
            found = Some(entries.remove(pos));
        }
    });
    found.unwrap_or_else(|| CachedOdeTrajectory::new(model_id, ds_idx, t0, y0, params, x_tail))
}

pub(in crate::dsl) fn store_cached_ode_trajectory(entry: CachedOdeTrajectory) {
    let _ = ODE_TRAJECTORY_CACHE.try_with(|slot| {
        if let Ok(mut entries) = slot.try_borrow_mut() {
            entries.retain(|e| !e.matches(entry.model_id, entry.ds_idx, entry.t0, entry.y0, &entry.params, &entry.x_tail));
            entries.insert(0, entry);
            entries.truncate(MAX_CACHED_ODE_TRAJECTORIES);
        }
    });
}

pub(in crate::dsl) enum Rk45Outcome {
    Reached { y: f64, h: f64, steps_taken: usize },
    IntrinsicFailed { last_t: f64, steps_taken: usize },
    BudgetExceeded { steps_taken: usize },
}

/// Adaptive Dormand–Prince 5(4) Runge–Kutta integrator from `t0` to `t_end` for scalar `y`.
///
/// Butcher tableau, 5th-order propagating weights, and embedded error weights `b - b*` of
/// Dormand & Prince (1980) (local extrapolation: the 5th-order solution is advanced). Error
/// control follows Hairer, Nørsett & Wanner (1993), §II.4 eq. (4.13): `err <= atol + rtol *
/// max(|y|, |y_new|)` with `rtol = 1e-8`, `atol = 1e-10`; both accepted and rejected steps scale
/// by `0.9 * ratio^(-1/5)` (`1/(q+1) = 1/5` for the 4th-order embedded error estimator), clamped
/// to `[0.2, 5]` on accept and `[0.1, 0.5]` on rejection.
///
/// Design notes: the first-same-as-last stage is recomputed rather than reused (cost only), and
/// the initial step is `0.05 * span`. Domain (`None`) or non-finite trial stages `k2..k7` reject
/// the step and shrink `h` by 4 (Hairer, Nørsett & Wanner 1993, §II.4); integration fails (`None`)
/// if `k1` at the current accepted state fails, below `1e-13 * max(span, 1)`, or after `MAX_ODE_STEPS` steps.
pub fn integrate_rk45<F>(t0: f64, t_end: f64, y0: f64, rhs: F) -> Option<f64>
where
    F: FnMut(f64, f64) -> Option<f64>,
{
    let span = (t_end - t0).abs();
    let mut unused_knots = Vec::new();
    match integrate_rk45_segment(t0, t_end, y0, 0.0, span, MAX_ODE_STEPS, true, &mut unused_knots, rhs) {
        Rk45Outcome::Reached { y, .. } => Some(y),
        Rk45Outcome::IntrinsicFailed { .. } | Rk45Outcome::BudgetExceeded { .. } => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::dsl) fn integrate_rk45_segment<F>(
    t0: f64,
    t_end: f64,
    y0: f64,
    h_hint: f64,
    total_span: f64,
    step_cap: usize,
    enforce_fit_budget: bool,
    knots_out: &mut Vec<(f64, f64, f64)>,
    mut rhs: F,
) -> Rk45Outcome
where
    F: FnMut(f64, f64) -> Option<f64>,
{
    if !t0.is_finite() || !t_end.is_finite() || !y0.is_finite() || !total_span.is_finite() {
        return Rk45Outcome::IntrinsicFailed { last_t: t0, steps_taken: 0 };
    }
    let dir = if t_end >= t0 { 1.0 } else { -1.0 };
    let span = (t_end - t0).abs();
    if span == 0.0 {
        return Rk45Outcome::Reached { y: y0, h: h_hint, steps_taken: 0 };
    }
    if !span.is_finite() {
        return Rk45Outcome::IntrinsicFailed { last_t: t0, steps_taken: 0 };
    }
    let scale_span = total_span.max(span);
    let rtol = 1e-8_f64;
    let atol = 1e-10_f64;

    let mut t = t0;
    let mut y = y0;
    let min_init_h = (span * 1e-6).max(1e-18).min(span);
    let mut h = if h_hint > 0.0 && h_hint.is_finite() { h_hint.max(min_init_h).min(span) } else { (span * 0.05).max(min_init_h).min(span) };
    let min_h = (1e-13 * scale_span.max(1.0)).min(span * 0.5);
    if min_h <= 0.0 {
        return Rk45Outcome::IntrinsicFailed { last_t: t0, steps_taken: 0 };
    }

    // Dormand-Prince Butcher tableau coefficients.
    const A21: f64 = 1.0 / 5.0;
    const A31: f64 = 3.0 / 40.0;
    const A32: f64 = 9.0 / 40.0;
    const A41: f64 = 44.0 / 45.0;
    const A42: f64 = -56.0 / 15.0;
    const A43: f64 = 32.0 / 9.0;
    const A51: f64 = 19372.0 / 6561.0;
    const A52: f64 = -25360.0 / 2187.0;
    const A53: f64 = 64448.0 / 6561.0;
    const A54: f64 = -212.0 / 729.0;
    const A61: f64 = 9017.0 / 3168.0;
    const A62: f64 = -355.0 / 33.0;
    const A63: f64 = 46732.0 / 5247.0;
    const A64: f64 = 49.0 / 176.0;
    const A65: f64 = -5103.0 / 18656.0;

    const B1: f64 = 35.0 / 384.0;
    const B3: f64 = 500.0 / 1113.0;
    const B4: f64 = 125.0 / 192.0;
    const B5: f64 = -2187.0 / 6784.0;
    const B6: f64 = 11.0 / 84.0;

    const E1: f64 = 71.0 / 57600.0;
    const E3: f64 = -71.0 / 16695.0;
    const E4: f64 = 71.0 / 1920.0;
    const E5: f64 = -17253.0 / 339200.0;
    const E6: f64 = 22.0 / 525.0;
    const E7: f64 = -1.0 / 40.0;

    let mut steps_taken = 0_usize;
    let mut resume_h = h;

    for _ in 0..step_cap {
        let rem = (t_end - t) * dir;
        if rem <= 1e-14 * scale_span || rem <= 4.0 * f64::EPSILON * t_end.abs() {
            return Rk45Outcome::Reached { y, h: resume_h, steps_taken };
        }
        if enforce_fit_budget && !consume_ode_fit_step() {
            return Rk45Outcome::BudgetExceeded { steps_taken };
        }
        steps_taken += 1;

        let natural_h = h;
        let was_clipped = h > rem;
        if was_clipped {
            h = rem;
        }
        let dt = dir * h;
        let Some(k1) = rhs(t, y).filter(|v| v.is_finite()) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let trial_stages = (|| -> Option<(f64, f64, f64, f64, f64, f64)> {
            let k2 = rhs(t + 0.2 * dt, y + dt * A21 * k1)?;
            let k3 = rhs(t + 0.3 * dt, y + dt * (A31 * k1 + A32 * k2))?;
            let k4 = rhs(t + 0.8 * dt, y + dt * (A41 * k1 + A42 * k2 + A43 * k3))?;
            let k5 = rhs(t + (8.0 / 9.0) * dt, y + dt * (A51 * k1 + A52 * k2 + A53 * k3 + A54 * k4))?;
            let k6 = rhs(t + dt, y + dt * (A61 * k1 + A62 * k2 + A63 * k3 + A64 * k4 + A65 * k5))?;
            let y_next = y + dt * (B1 * k1 + B3 * k3 + B4 * k4 + B5 * k5 + B6 * k6);
            if !y_next.is_finite() {
                return None;
            }
            let k7 = rhs(t + dt, y_next)?;
            Some((k3, k4, k5, k6, y_next, k7))
        })();
        let Some((k3, k4, k5, k6, y_next, k7)) = trial_stages else {
            h *= 0.25;
            if h < min_h {
                return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
            }
            continue;
        };
        let err = (dt * (E1 * k1 + E3 * k3 + E4 * k4 + E5 * k5 + E6 * k6 + E7 * k7)).abs();
        let tol = atol + rtol * y.abs().max(y_next.abs());
        let ratio = err / tol;
        if !err.is_finite() || !tol.is_finite() || !ratio.is_finite() {
            h *= 0.25;
            if h < min_h {
                return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
            }
            continue;
        }

        if ratio <= 1.0 {
            let t_next = t + dt;
            if t_next == t {
                return if h >= rem {
                    Rk45Outcome::Reached { y: y_next, h: natural_h, steps_taken }
                } else {
                    Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken }
                };
            }
            t = t_next;
            y = y_next;
            let factor = if ratio < 1e-12 { 5.0 } else { (0.9 * ratio.powf(-0.2)).clamp(0.2, 5.0) };
            h = (h * factor).max(min_h);
            resume_h = if was_clipped { natural_h } else { h };
            let after_rem = (t_end - t) * dir;
            if after_rem > 1e-14 * scale_span && after_rem > 4.0 * f64::EPSILON * t_end.abs() {
                knots_out.push((t, y, h));
            }
        } else {
            // Hairer, Nørsett & Wanner (1993) §II.4 eq. (4.13): error-per-step exponent -1/(q+1) = -1/5.
            let factor = (0.9 * ratio.powf(-0.2)).clamp(0.1, 0.5);
            h *= factor;
            if h < min_h {
                return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
            }
        }
    }

    let rem = (t_end - t) * dir;
    if rem <= 1e-14 * scale_span || rem <= 4.0 * f64::EPSILON * t_end.abs() {
        Rk45Outcome::Reached { y, h: resume_h, steps_taken }
    } else {
        Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken }
    }
}
