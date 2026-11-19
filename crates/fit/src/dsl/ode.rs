//! Per-fit ODE step budget, the trajectory cache, and adaptive Dormand–Prince RK45 integration.
//!
//! References:
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae. *Journal of Computational and Applied Mathematics* 6(1), 19–26.
//! - Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I: Nonstiff Problems*, 2nd rev. ed. Springer.

use std::cell::RefCell;
use std::sync::atomic::AtomicU64;

use crate::dsl::limits::{MAX_CACHED_ODE_KNOTS, MAX_CACHED_ODE_TRAJECTORIES, MAX_ODE_STEPS, ODE_VM_OPS_PER_NOMINAL_STEP};

#[derive(Debug, Clone, Copy)]
pub(in crate::dsl) struct OdeFitBudgetState {
    pub(in crate::dsl) used_steps: usize,
    pub(in crate::dsl) max_steps: usize,
    pub(in crate::dsl) used_vm_ops: usize,
    pub(in crate::dsl) max_vm_ops: usize,
    pub(in crate::dsl) exceeded: bool,
}

std::thread_local! {
    static ACTIVE_ODE_BUDGET: RefCell<Option<OdeFitBudgetState>> = const { RefCell::new(None) };
}

/// RAII scope bounding total adaptive RK45 steps and VM operations across a single nonlinear fit execution.
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
    let max_vm_ops = max_steps.saturating_mul(ODE_VM_OPS_PER_NOMINAL_STEP);
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(mut cell) = slot.try_borrow_mut() {
            prev = cell.replace(OdeFitBudgetState { used_steps: 0, max_steps, used_vm_ops: 0, max_vm_ops, exceeded: false });
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

fn consume_ode_fit_step_internal(nominal_vm_ops: usize) -> bool {
    let mut allowed = true;
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(mut cell) = slot.try_borrow_mut()
            && let Some(state) = cell.as_mut()
        {
            let next_vm_ops = state.used_vm_ops.saturating_add(nominal_vm_ops);
            if state.exceeded || state.used_steps >= state.max_steps || (nominal_vm_ops > 0 && next_vm_ops > state.max_vm_ops) {
                state.exceeded = true;
                allowed = false;
            } else {
                state.used_steps += 1;
                state.used_vm_ops = next_vm_ops;
            }
        }
    });
    allowed
}

pub(in crate::dsl) fn consume_ode_fit_step() -> bool {
    consume_ode_fit_step_internal(64)
}

pub(in crate::dsl) fn consume_ode_fit_rk_step() -> bool {
    consume_ode_fit_step_internal(0)
}

pub(in crate::dsl) fn consume_ode_fit_vm_ops(ops: usize) -> bool {
    let mut allowed = true;
    let _ = ACTIVE_ODE_BUDGET.try_with(|slot| {
        if let Ok(mut cell) = slot.try_borrow_mut()
            && let Some(state) = cell.as_mut()
        {
            let next_vm_ops = state.used_vm_ops.saturating_add(ops);
            if state.exceeded || next_vm_ops > state.max_vm_ops {
                state.exceeded = true;
                allowed = false;
            } else {
                state.used_vm_ops = next_vm_ops;
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
    pub(in crate::dsl) frontier: Vec<OdeKnot>,
    pub(in crate::dsl) y_ref: f64,
    pub(in crate::dsl) steps: usize,
    pub(in crate::dsl) failed_beyond_dist: Option<f64>,
    pub(in crate::dsl) cached_points: Vec<(u64, f64)>,
}

impl TrajectorySide {
    pub(in crate::dsl) fn thin_if_needed(&mut self) {
        if self.frontier.len() <= MAX_CACHED_ODE_KNOTS {
            return;
        }
        let n = self.frontier.len();
        let mut thinned = Vec::with_capacity(MAX_CACHED_ODE_KNOTS / 2 + 2);
        for (idx, knot) in self.frontier.iter().enumerate() {
            if idx == 0 || idx + 1 == n || idx % 2 == 0 {
                thinned.push(knot.clone());
            }
        }
        self.frontier = thinned;
    }

    pub(in crate::dsl) fn push_frontier_knots(&mut self, new_knots: Vec<OdeKnot>) {
        if new_knots.is_empty() {
            return;
        }
        self.frontier.extend(new_knots);
        self.thin_if_needed();
    }

    pub(in crate::dsl) fn lookup_point(&self, t_bits: u64) -> Option<f64> {
        self.cached_points.iter().find_map(|&(bits, y)| (bits == t_bits).then_some(y))
    }

    pub(in crate::dsl) fn record_point(&mut self, t_bits: u64, y: f64) {
        if self.cached_points.len() >= MAX_CACHED_ODE_KNOTS {
            self.cached_points.drain(..MAX_CACHED_ODE_KNOTS / 2);
        }
        self.cached_points.push((t_bits, y));
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
        let y_ref = y0.abs();
        Self {
            model_id,
            ds_idx,
            t0,
            y0,
            params: params.to_vec(),
            x_tail: x_tail.to_vec(),
            pos: TrajectorySide { frontier: vec![init_knot.clone()], y_ref, steps: 0, failed_beyond_dist: None, cached_points: Vec::new() },
            neg: TrajectorySide { frontier: vec![init_knot], y_ref, steps: 0, failed_beyond_dist: None, cached_points: Vec::new() },
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

pub(in crate::dsl) trait OdeRhsSource {
    fn eval_rhs(&mut self, t: f64, y: f64) -> Option<f64>;
    fn budget_exceeded(&self) -> bool;
    fn charge_fit_step(&self) -> bool;
}

struct ClosureOdeRhs<F>(F);

impl<F: FnMut(f64, f64) -> Option<f64>> OdeRhsSource for ClosureOdeRhs<F> {
    fn eval_rhs(&mut self, t: f64, y: f64) -> Option<f64> {
        (self.0)(t, y)
    }
    fn budget_exceeded(&self) -> bool {
        ode_fit_budget_exceeded().is_some()
    }
    fn charge_fit_step(&self) -> bool {
        consume_ode_fit_step()
    }
}

pub(in crate::dsl) enum Rk45Outcome {
    Reached { y: f64 },
    IntrinsicFailed,
    BudgetExceeded,
}

const DP_A21: f64 = 1.0 / 5.0;
const DP_A31: f64 = 3.0 / 40.0;
const DP_A32: f64 = 9.0 / 40.0;
const DP_A41: f64 = 44.0 / 45.0;
const DP_A42: f64 = -56.0 / 15.0;
const DP_A43: f64 = 32.0 / 9.0;
const DP_A51: f64 = 19372.0 / 6561.0;
const DP_A52: f64 = -25360.0 / 2187.0;
const DP_A53: f64 = 64448.0 / 6561.0;
const DP_A54: f64 = -212.0 / 729.0;
const DP_A61: f64 = 9017.0 / 3168.0;
const DP_A62: f64 = -355.0 / 33.0;
const DP_A63: f64 = 46732.0 / 5247.0;
const DP_A64: f64 = 49.0 / 176.0;
const DP_A65: f64 = -5103.0 / 18656.0;

const DP_B1: f64 = 35.0 / 384.0;
const DP_B3: f64 = 500.0 / 1113.0;
const DP_B4: f64 = 125.0 / 192.0;
const DP_B5: f64 = -2187.0 / 6784.0;
const DP_B6: f64 = 11.0 / 84.0;

const DP_E1: f64 = 71.0 / 57600.0;
const DP_E3: f64 = -71.0 / 16695.0;
const DP_E4: f64 = 71.0 / 1920.0;
const DP_E5: f64 = -17253.0 / 339200.0;
const DP_E6: f64 = 22.0 / 525.0;
const DP_E7: f64 = -1.0 / 40.0;

struct Rk45Cfg {
    dir: f64,
    rtol: f64,
    atol_rel: f64,
    y_ref: f64,
    min_h: f64,
    target_t: f64,
    clip_to_target: bool,
}

enum Rk45StepResult {
    Accepted { t_next: f64, y_next: f64, h_next: f64 },
    Rejected { h_next: f64 },
    DomainRejected { h_next: f64 },
    K1Failed,
    BudgetExceeded,
    Underflow,
}

fn eval_dp_stages(t: f64, y: f64, dt: f64, k1: f64, rhs_src: &mut dyn OdeRhsSource) -> Option<(f64, f64, f64, f64, f64, f64)> {
    let k2 = rhs_src.eval_rhs(t + 0.2 * dt, y + dt * DP_A21 * k1).filter(|v| v.is_finite())?;
    let k3 = rhs_src.eval_rhs(t + 0.3 * dt, y + dt * (DP_A31 * k1 + DP_A32 * k2)).filter(|v| v.is_finite())?;
    let k4 = rhs_src.eval_rhs(t + 0.8 * dt, y + dt * (DP_A41 * k1 + DP_A42 * k2 + DP_A43 * k3)).filter(|v| v.is_finite())?;
    let k5 = rhs_src.eval_rhs(t + (8.0 / 9.0) * dt, y + dt * (DP_A51 * k1 + DP_A52 * k2 + DP_A53 * k3 + DP_A54 * k4)).filter(|v| v.is_finite())?;
    let k6 = rhs_src.eval_rhs(t + dt, y + dt * (DP_A61 * k1 + DP_A62 * k2 + DP_A63 * k3 + DP_A64 * k4 + DP_A65 * k5)).filter(|v| v.is_finite())?;
    let y_next = y + dt * (DP_B1 * k1 + DP_B3 * k3 + DP_B4 * k4 + DP_B5 * k5 + DP_B6 * k6);
    if !y_next.is_finite() {
        return None;
    }
    let k7 = rhs_src.eval_rhs(t + dt, y_next).filter(|v| v.is_finite())?;
    Some((k3, k4, k5, k6, y_next, k7))
}

/// Single Dormand–Prince 5(4) trial step with scale-invariant mixed error control
/// (Dormand & Prince 1980; Hairer, Nørsett & Wanner 1993, §II.4 eq. 4.13).
fn rk45_step(t: f64, y: f64, h: f64, cfg: &Rk45Cfg, rhs_src: &mut dyn OdeRhsSource) -> Rk45StepResult {
    if !rhs_src.charge_fit_step() || rhs_src.budget_exceeded() {
        return Rk45StepResult::BudgetExceeded;
    }
    let rem = (cfg.target_t - t) * cfg.dir;
    let was_clipped = cfg.clip_to_target && h > rem;
    let h_used = if was_clipped { rem } else { h };
    let dt = cfg.dir * h_used;
    let k1 = match rhs_src.eval_rhs(t, y) {
        Some(v) if v.is_finite() => v,
        _ if rhs_src.budget_exceeded() => return Rk45StepResult::BudgetExceeded,
        _ => return Rk45StepResult::K1Failed,
    };
    let Some((k3, k4, k5, k6, y_next, k7)) = eval_dp_stages(t, y, dt, k1, rhs_src) else {
        if rhs_src.budget_exceeded() {
            return Rk45StepResult::BudgetExceeded;
        }
        return Rk45StepResult::DomainRejected { h_next: h_used * 0.25 };
    };
    let err = (dt * (DP_E1 * k1 + DP_E3 * k3 + DP_E4 * k4 + DP_E5 * k5 + DP_E6 * k6 + DP_E7 * k7)).abs();
    let tol = cfg.atol_rel * cfg.y_ref + cfg.rtol * y.abs().max(y_next.abs());
    if !err.is_finite() || !tol.is_finite() {
        return Rk45StepResult::Rejected { h_next: h_used * 0.25 };
    }
    let ratio = if err == 0.0 {
        0.0
    } else if tol <= 0.0 {
        f64::INFINITY
    } else {
        err / tol
    };
    if !ratio.is_finite() {
        return Rk45StepResult::Rejected { h_next: h_used * 0.25 };
    }
    if ratio <= 1.0 {
        let t_next = t + dt;
        if t_next == t {
            if cfg.clip_to_target && h >= rem {
                return Rk45StepResult::Accepted { t_next: cfg.target_t, y_next, h_next: h };
            }
            return Rk45StepResult::Underflow;
        }
        let factor = if ratio < 1e-12 { 5.0 } else { (0.9 * ratio.powf(-0.2)).clamp(0.2, 5.0) };
        let h_next = if was_clipped { h } else { (h_used * factor).max(cfg.min_h) };
        return Rk45StepResult::Accepted { t_next, y_next, h_next };
    }
    // Hairer, Nørsett & Wanner (1993) §II.4 eq. (4.13): error-per-step exponent -1/(q+1) = -1/5.
    let factor = (0.9 * ratio.powf(-0.2)).clamp(0.1, 0.5);
    Rk45StepResult::Rejected { h_next: h_used * factor }
}

/// Adaptive Dormand–Prince 5(4) Runge–Kutta integrator from `t0` to `t_end` for scalar `y`.
///
/// Butcher tableau, 5th-order propagating weights, and embedded error weights `b - b*` of
/// Dormand & Prince (1980) (local extrapolation: the 5th-order solution is advanced). Error
/// control follows Hairer, Nørsett & Wanner (1993), §II.4 eq. (4.13): `err <= atol_rel * y_ref
/// + rtol * max(|y|, |y_new|)` with `rtol = 1e-8`, `atol_rel = 1e-10`, and `y_ref = |y0|` (or
/// `1.0` when `y0 == 0`); both accepted and rejected steps scale by `0.9 * ratio^(-1/5)`
/// (`1/(q+1) = 1/5` for the 4th-order embedded error estimator), clamped to `[0.2, 5]` on accept
/// and `[0.1, 0.5]` on rejection.
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
    let y_ref = if y0 != 0.0 && y0.is_finite() { y0.abs() } else { 1.0 };
    let mut src = ClosureOdeRhs(rhs);
    match integrate_rk45_segment_src(t0, t_end, y0, 0.0, MAX_ODE_STEPS, (1e-10, y_ref, span), &mut src) {
        Rk45Outcome::Reached { y } => Some(y),
        Rk45Outcome::IntrinsicFailed | Rk45Outcome::BudgetExceeded => None,
    }
}

/// Hairer, Nørsett & Wanner (1993), §II.4, p. 169 scale-invariant initial step selection (`hinit`)
/// and trajectory scale reference based strictly on `(t0, y0, rhs)`.
fn compute_ode_init_step_and_scale(t0: f64, y0: f64, dir: f64, rhs_src: &mut dyn OdeRhsSource) -> Result<(f64, f64), Rk45Outcome> {
    let t_scale = t0.abs().max(1.0);
    let f0 = match rhs_src.eval_rhs(t0, y0) {
        Some(v) if v.is_finite() => v,
        _ if rhs_src.budget_exceeded() => return Err(Rk45Outcome::BudgetExceeded),
        _ => return Err(Rk45Outcome::IntrinsicFailed),
    };
    let raw_scale = if y0 != 0.0 { y0.abs() } else { f0.abs() * t_scale };
    let sc = if raw_scale > 0.0 && raw_scale.is_finite() { 1e-8 * raw_scale } else { 1e-8 };
    let d0 = y0.abs() / sc;
    let d1 = f0.abs() / sc;
    let h0 = if d0 < 1e-5 || d1 < 1e-5 { 1e-6 * t_scale } else { 0.01 * (d0 / d1) };
    let h0 = h0.clamp(1e-12 * t_scale, 1e4 * t_scale);
    let y1 = y0 + dir * h0 * f0;
    let h_init = if y1.is_finite() {
        match rhs_src.eval_rhs(t0 + dir * h0, y1) {
            Some(f1) if f1.is_finite() => {
                let d2 = (f1 - f0).abs() / (sc * h0);
                let denom = d1.max(d2);
                let h1 = if denom <= 1e-15 { (1e-6 * t_scale).max(h0 * 1e-3) } else { (0.01 / denom).powf(0.2) };
                h1.min(100.0 * h0).clamp(1e-12 * t_scale, 1e4 * t_scale)
            }
            _ if rhs_src.budget_exceeded() => return Err(Rk45Outcome::BudgetExceeded),
            _ => (h0 * 0.25).max(1e-12 * t_scale),
        }
    } else {
        (h0 * 0.25).max(1e-12 * t_scale)
    };
    let y_ref = if y0 != 0.0 { y0.abs() } else { f0.abs() * t_scale * 1e-2 };
    Ok((h_init.max(1e-15 * t_scale), y_ref))
}

enum FrontierAdvanceOutcome {
    Reached,
    DomainBoundary { last_t: f64, last_y: f64, last_h: f64, last_dist: f64 },
    IntrinsicFailed { last_dist: f64 },
    BudgetExceeded,
}

fn advance_ode_frontier(
    side: &mut TrajectorySide,
    t0: f64,
    t_end: f64,
    dir: f64,
    target_dist: f64,
    rhs_src: &mut dyn OdeRhsSource,
) -> FrontierAdvanceOutcome {
    let Some(tail) = side.frontier.last().cloned() else {
        return FrontierAdvanceOutcome::IntrinsicFailed { last_dist: 0.0 };
    };
    let mut t = tail.t;
    let mut y = tail.y;
    let mut h = tail.h;
    let mut cur_dist = tail.dist;
    let mut new_knots = Vec::new();
    let step_cap = MAX_ODE_STEPS.saturating_sub(side.steps);
    let tol_dist = 4.0 * f64::EPSILON * t_end.abs().max(t0.abs());
    let mut domain_rejects = 0_usize;

    for _ in 0..step_cap {
        if cur_dist + tol_dist >= target_dist {
            side.push_frontier_knots(new_knots);
            return FrontierAdvanceOutcome::Reached;
        }
        let t_scale = t.abs().max(t0.abs()).max(1.0);
        let min_h = 1e-13 * t_scale;
        let cfg = Rk45Cfg { dir, rtol: 1e-8, atol_rel: 1e-15, y_ref: side.y_ref, min_h, target_t: t_end, clip_to_target: false };
        side.steps += 1;
        match rk45_step(t, y, h, &cfg, rhs_src) {
            Rk45StepResult::Accepted { t_next, y_next, h_next } => {
                domain_rejects = 0;
                t = t_next;
                y = y_next;
                h = h_next;
                cur_dist = (t - t0) * dir;
                new_knots.push(OdeKnot { dist: cur_dist, t, y, h });
                if cur_dist + tol_dist >= target_dist {
                    side.push_frontier_knots(new_knots);
                    return FrontierAdvanceOutcome::Reached;
                }
            }
            Rk45StepResult::DomainRejected { h_next } => {
                domain_rejects += 1;
                let rem = (t_end - t) * dir;
                if domain_rejects >= 8 && h > rem {
                    side.push_frontier_knots(new_knots);
                    return FrontierAdvanceOutcome::DomainBoundary { last_t: t, last_y: y, last_h: h_next.max(min_h), last_dist: cur_dist };
                }
                if h_next < min_h {
                    side.push_frontier_knots(new_knots);
                    return FrontierAdvanceOutcome::DomainBoundary { last_t: t, last_y: y, last_h: min_h, last_dist: cur_dist };
                }
                h = h_next;
            }
            Rk45StepResult::Rejected { h_next } => {
                if h_next < min_h {
                    side.push_frontier_knots(new_knots);
                    return FrontierAdvanceOutcome::IntrinsicFailed { last_dist: cur_dist };
                }
                h = h_next;
            }
            Rk45StepResult::BudgetExceeded => {
                side.push_frontier_knots(new_knots);
                return FrontierAdvanceOutcome::BudgetExceeded;
            }
            Rk45StepResult::K1Failed | Rk45StepResult::Underflow => {
                side.push_frontier_knots(new_knots);
                return FrontierAdvanceOutcome::IntrinsicFailed { last_dist: cur_dist };
            }
        }
    }

    side.push_frontier_knots(new_knots);
    if cur_dist + tol_dist >= target_dist { FrontierAdvanceOutcome::Reached } else { FrontierAdvanceOutcome::IntrinsicFailed { last_dist: cur_dist } }
}

pub(in crate::dsl) fn eval_ode_on_side(
    side: &mut TrajectorySide,
    t0: f64,
    t_end: f64,
    dir: f64,
    target_dist: f64,
    rhs_src: &mut dyn OdeRhsSource,
) -> Option<f64> {
    if side.frontier.is_empty() {
        return None;
    }
    if side.frontier.len() == 1 && side.frontier[0].h == 0.0 {
        let y0 = side.frontier[0].y;
        let Ok((h_init, y_ref)) = compute_ode_init_step_and_scale(t0, y0, dir, rhs_src) else {
            return None;
        };
        side.frontier[0].h = h_init;
        side.y_ref = y_ref;
    }

    let last_dist = side.frontier.last().map_or(0.0, |k| k.dist);
    let tol_dist = 4.0 * f64::EPSILON * t_end.abs().max(t0.abs());
    if last_dist + tol_dist < target_dist {
        match advance_ode_frontier(side, t0, t_end, dir, target_dist, rhs_src) {
            FrontierAdvanceOutcome::Reached => {}
            FrontierAdvanceOutcome::BudgetExceeded => return None,
            FrontierAdvanceOutcome::DomainBoundary { last_t, last_y, last_h, last_dist: fail_dist } => {
                let seg = integrate_rk45_segment_src(
                    last_t,
                    t_end,
                    last_y,
                    last_h.min((t_end - last_t).abs()),
                    MAX_ODE_STEPS.saturating_sub(side.steps).max(1),
                    (1e-15, side.y_ref, t_end.abs().max(t0.abs())),
                    rhs_src,
                );
                if let Rk45Outcome::Reached { y } = seg {
                    side.record_point(t_end.to_bits(), y);
                    return Some(y);
                }
                if !matches!(seg, Rk45Outcome::BudgetExceeded) {
                    side.failed_beyond_dist = Some(side.failed_beyond_dist.map_or(fail_dist, |p| p.min(fail_dist)));
                }
                return None;
            }
            FrontierAdvanceOutcome::IntrinsicFailed { last_dist: fail_dist } => {
                side.failed_beyond_dist = Some(side.failed_beyond_dist.map_or(fail_dist, |p| p.min(fail_dist)));
                return None;
            }
        }
    }

    let idx = side.frontier.partition_point(|k| k.dist < target_dist);
    if idx < side.frontier.len() && side.frontier[idx].t.to_bits() == t_end.to_bits() {
        let y = side.frontier[idx].y;
        side.record_point(t_end.to_bits(), y);
        return Some(y);
    }
    let bracket_idx = idx.saturating_sub(1);
    let prev = &side.frontier[bracket_idx];
    if prev.t.to_bits() == t_end.to_bits() {
        let y = prev.y;
        side.record_point(t_end.to_bits(), y);
        return Some(y);
    }

    match integrate_rk45_segment_src(prev.t, t_end, prev.y, prev.h, MAX_ODE_STEPS, (1e-15, side.y_ref, t_end.abs().max(t0.abs())), rhs_src) {
        Rk45Outcome::Reached { y } => {
            side.record_point(t_end.to_bits(), y);
            Some(y)
        }
        Rk45Outcome::IntrinsicFailed | Rk45Outcome::BudgetExceeded => None,
    }
}

fn integrate_rk45_segment_src(
    t0: f64,
    t_end: f64,
    y0: f64,
    h_hint: f64,
    step_cap: usize,
    tol_scale: (f64, f64, f64),
    rhs_src: &mut dyn OdeRhsSource,
) -> Rk45Outcome {
    let (atol_rel, y_ref, total_span) = tol_scale;
    if !t0.is_finite() || !t_end.is_finite() || !y0.is_finite() || !total_span.is_finite() {
        return Rk45Outcome::IntrinsicFailed;
    }
    let dir = if t_end >= t0 { 1.0 } else { -1.0 };
    let span = (t_end - t0).abs();
    if span == 0.0 {
        return Rk45Outcome::Reached { y: y0 };
    }
    if !span.is_finite() {
        return Rk45Outcome::IntrinsicFailed;
    }
    let scale_span = total_span.max(span);
    let min_init_h = (span * 1e-6).max(1e-18).min(span);
    let mut h = if h_hint > 0.0 && h_hint.is_finite() { h_hint.max(min_init_h).min(span) } else { (span * 0.05).max(min_init_h).min(span) };
    let min_h = (1e-13 * scale_span.max(1.0)).min(span * 0.5);
    if min_h <= 0.0 {
        return Rk45Outcome::IntrinsicFailed;
    }

    let mut t = t0;
    let mut y = y0;
    let end_tol = 1e-14 * scale_span;
    let eps_tol = 4.0 * f64::EPSILON * t_end.abs();

    for _ in 0..step_cap {
        let rem = (t_end - t) * dir;
        if rem <= end_tol || rem <= eps_tol {
            return Rk45Outcome::Reached { y };
        }
        let cfg = Rk45Cfg { dir, rtol: 1e-8, atol_rel, y_ref, min_h, target_t: t_end, clip_to_target: true };
        match rk45_step(t, y, h, &cfg, rhs_src) {
            Rk45StepResult::Accepted { t_next, y_next, h_next } => {
                t = t_next;
                y = y_next;
                h = h_next;
            }
            Rk45StepResult::Rejected { h_next } | Rk45StepResult::DomainRejected { h_next } => {
                if h_next < min_h {
                    return Rk45Outcome::IntrinsicFailed;
                }
                h = h_next;
            }
            Rk45StepResult::BudgetExceeded => return Rk45Outcome::BudgetExceeded,
            Rk45StepResult::K1Failed | Rk45StepResult::Underflow => {
                return Rk45Outcome::IntrinsicFailed;
            }
        }
    }

    let rem = (t_end - t) * dir;
    if rem <= end_tol || rem <= eps_tol { Rk45Outcome::Reached { y } } else { Rk45Outcome::IntrinsicFailed }
}
