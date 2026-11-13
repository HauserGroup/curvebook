//! Trust-region Levenberg–Marquardt nonlinear regression engine, global sharing, constraints,
//! Nelder–Mead simplex warm-start, weighting, Poisson deviance IRLS, Cauchy robust IRLS,
//! ROUT outlier elimination, curve bands, and Brent inverse unknown interpolation.

use lumencraft_doc::{CiBound, CurveSamplePoint, FitMethod, FittedCurveOverlay, ModelComparisonSpec, ParameterRule, ParameterScope, WeightingScheme};
use lumencraft_math::ConfidenceLevel;
use lumencraft_math::dist::student_t_two_sided_isf;
use lumencraft_math::linalg::{DenseMatrix, qr, qr_col_pivoted};
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::deviance_bd0;
use lumencraft_math::sum::NeumaierSum;

use crate::diagnostics::{
    AiccComparisonResult, CovarianceDiagnostics, ExtraSumOfSquaresResult, LackOfFitResult, NormalityTestResult, RoutDetectionResult, RunsTestResult,
    compare_models_aicc, compare_models_f_test, compute_covariance_diagnostics, compute_hougaard_skewness, compute_information_criteria,
    compute_profile_likelihood_cis, compute_rsdr, dagostino_pearson_test, replicates_lack_of_fit, rout_detect_outliers, runs_test, shapiro_wilk_test,
};
use crate::error::{FitError, FitStatus, ParameterNote};
use crate::library::ParametricModel;
use crate::types::{DataPoint, DatasetSeries, DerivedScalar, HookContext};

/// Configuration for executing nonlinear regression via [`fit_nonlinear`].
#[derive(Debug, Clone)]
pub struct SolverConfig {
    pub method: FitMethod,
    pub weighting: WeightingScheme,
    pub parameter_rules: Vec<ParameterRule>,
    pub confidence: ConfidenceLevel,
    pub max_iters: usize,
    pub compute_profile_ci: bool,
    pub compute_hougaard: bool,
    pub compute_bands: bool,
    pub compute_prediction_bands: bool,
    pub compute_runs_test: bool,
    pub compute_lack_of_fit: bool,
    pub compute_normality: bool,
    pub interpolate_unknowns: bool,
    pub use_simplex_warm_start: bool,
}

impl Default for SolverConfig {
    fn default() -> Self {
        Self {
            method: FitMethod::LeastSquares,
            weighting: WeightingScheme::None,
            parameter_rules: Vec::new(),
            confidence: ConfidenceLevel::DEFAULT_95,
            max_iters: 1_000,
            compute_profile_ci: true,
            compute_hougaard: true,
            compute_bands: true,
            compute_prediction_bands: false,
            compute_runs_test: true,
            compute_lack_of_fit: false,
            compute_normality: false,
            interpolate_unknowns: false,
            use_simplex_warm_start: true,
        }
    }
}

/// Fitted value for a single free parameter slot (local or globally shared).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FittedParameterSummary {
    pub slot_index: usize,
    pub name: String,
    pub dataset_index: Option<usize>,
    pub estimate: f64,
    pub se: f64,
    pub ci_lower: CiBound,
    pub ci_upper: CiBound,
    pub symmetric_ci_lower: f64,
    pub symmetric_ci_upper: f64,
    pub dependency: f64,
    pub hougaard_g1: f64,
    pub notes: Vec<ParameterNote>,
}

/// Interpolated unknown (`X` from `Y` or `Y` from `X`) with confidence interval.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InterpolatedUnknown {
    pub data_set_index: usize,
    pub row: usize,
    pub input_value: f64,
    pub output_estimate: f64,
    pub ci_lower: CiBound,
    pub ci_upper: CiBound,
    pub note: Option<String>,
}

/// Per-dataset fitted curve summary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FittedDatasetResult {
    pub data_set_index: usize,
    pub label: String,
    pub params: Vec<f64>,
    pub param_names: Vec<String>,
    pub y_hat: Vec<f64>,
    pub residuals: Vec<f64>,
    pub weighted_residuals: Vec<f64>,
    pub ss_residual: f64,
    pub r_squared: f64,
    pub sy_x: f64,
    pub curve_overlay: Option<FittedCurveOverlay>,
}

/// Model comparison output (`Extra-SS F` or `AICc`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ModelComparisonOutcome {
    ExtraSumOfSquaresF(ExtraSumOfSquaresResult),
    Aicc(AiccComparisonResult),
}

/// Complete nonlinear regression result across one or multiple globally fitted datasets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NonlinearFitResult {
    pub equation_id: String,
    pub equation_name: String,
    pub status: FitStatus,
    pub iterations: usize,
    pub n_obs: usize,
    pub n_outliers_excluded: usize,
    pub k_free: usize,
    pub df: usize,
    pub rss: f64,
    pub weighted_ss: f64,
    pub rsd: f64,
    pub r_squared: f64,
    pub aic_c: Option<f64>,
    pub bic: Option<f64>,
    pub parameters: Vec<FittedParameterSummary>,
    pub fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
    pub covariance: CovarianceDiagnostics,
    pub datasets: Vec<FittedDatasetResult>,
    pub derived_scalars: Vec<Vec<DerivedScalar>>,
    pub runs_test: Option<RunsTestResult>,
    pub lack_of_fit: Option<LackOfFitResult>,
    pub normality: Option<NormalityTestResult>,
    pub rout: Option<RoutDetectionResult>,
    pub interpolated_x: Vec<InterpolatedUnknown>,
    pub interpolated_y: Vec<InterpolatedUnknown>,
}

#[derive(Debug, Clone)]
struct FreeSlot {
    dataset_idx: Option<usize>,
    name: String,
    lower: Option<f64>,
    upper: Option<f64>,
    init: f64,
    is_shared: bool,
}

#[derive(Debug, Clone)]
enum ParamBinding {
    Free(usize),
    Fixed(f64),
}

#[derive(Debug, Clone)]
struct ParameterLayout {
    p_model: usize,
    n_datasets: usize,
    bindings: Vec<Vec<ParamBinding>>,
    free_slots: Vec<FreeSlot>,
    fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
}

impl ParameterLayout {
    fn build(model: &dyn ParametricModel, datasets: &[DatasetSeries], hooks: &HookContext, rules: &[ParameterRule]) -> Result<Self, FitError> {
        let param_names = model.param_names();
        let p_model = param_names.len();
        let n_datasets = datasets.len();
        let all_points: Vec<DataPoint> = datasets.iter().flat_map(|d| d.points.iter().cloned()).collect();
        let default_init = model.initial_guess(&all_points);
        let default_bounds = model.default_bounds();

        let per_ds_init: Vec<Vec<f64>> = datasets.iter().map(|d| model.initial_guess(&d.points)).collect();

        let mut bindings = vec![vec![ParamBinding::Fixed(0.0); p_model]; n_datasets];
        let mut free_slots = Vec::new();
        let mut fixed_constants = Vec::new();

        for (p_idx, pname) in param_names.iter().enumerate() {
            let rule = rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
            let def_b = default_bounds.get(p_idx).copied().unwrap_or((None, None));
            let lb = rule.and_then(|r| r.lower_bound).or(def_b.0);
            let ub = rule.and_then(|r| r.upper_bound).or(def_b.1);
            if let (Some(l), Some(u)) = (lb, ub)
                && l > u
            {
                return Err(FitError::InvalidParameter {
                    name: "parameter_rules",
                    reason: format!("Lower bound ({l}) > upper bound ({u}) for `{pname}`"),
                });
            }

            let scope = rule.map(|r| &r.scope);
            match scope {
                Some(ParameterScope::Constant(val)) => {
                    if !val.is_finite() {
                        return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Non-finite constant for `{pname}`") });
                    }
                    for ds_row in &mut bindings {
                        ds_row[p_idx] = ParamBinding::Fixed(*val);
                    }
                    fixed_constants.push((pname.clone(), *val, vec![ParameterNote::Constant]));
                }
                Some(ParameterScope::HookedConstant(src)) => {
                    let val = hooks.resolve(src)?;
                    for ds_row in &mut bindings {
                        ds_row[p_idx] = ParamBinding::Fixed(val);
                    }
                    fixed_constants.push((pname.clone(), val, vec![ParameterNote::Hooked]));
                }
                Some(ParameterScope::SharedGlobal) => {
                    let slot = free_slots.len();
                    let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                    free_slots.push(FreeSlot {
                        dataset_idx: None,
                        name: pname.clone(),
                        lower: lb,
                        upper: ub,
                        init: clamp_to_bounds(init_v, lb, ub),
                        is_shared: n_datasets > 1,
                    });
                    for ds_row in &mut bindings {
                        ds_row[p_idx] = ParamBinding::Free(slot);
                    }
                }
                None | Some(ParameterScope::Local) => {
                    if n_datasets <= 1 {
                        let slot = free_slots.len();
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        free_slots.push(FreeSlot {
                            dataset_idx: Some(0),
                            name: pname.clone(),
                            lower: lb,
                            upper: ub,
                            init: clamp_to_bounds(init_v, lb, ub),
                            is_shared: false,
                        });
                        if let Some(ds_row) = bindings.first_mut() {
                            ds_row[p_idx] = ParamBinding::Free(slot);
                        }
                    } else {
                        for ds_idx in 0..n_datasets {
                            let slot = free_slots.len();
                            let init_v =
                                rule.and_then(|r| r.initial_override).unwrap_or_else(|| per_ds_init[ds_idx].get(p_idx).copied().unwrap_or(1.0));
                            free_slots.push(FreeSlot {
                                dataset_idx: Some(ds_idx),
                                name: pname.clone(),
                                lower: lb,
                                upper: ub,
                                init: clamp_to_bounds(init_v, lb, ub),
                                is_shared: false,
                            });
                            bindings[ds_idx][p_idx] = ParamBinding::Free(slot);
                        }
                    }
                }
            }
        }

        Ok(Self { p_model, n_datasets, bindings, free_slots, fixed_constants })
    }

    fn k_free(&self) -> usize {
        self.free_slots.len()
    }

    fn bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        self.free_slots.iter().map(|s| (s.lower, s.upper)).collect()
    }

    fn expand_params(&self, theta: &[f64], ds_idx: usize) -> Vec<f64> {
        let row = &self.bindings[ds_idx.min(self.n_datasets - 1)];
        row.iter()
            .map(|b| match b {
                ParamBinding::Free(slot) => theta[*slot],
                ParamBinding::Fixed(v) => *v,
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct ObsRecord {
    point: DataPoint,
    ds_local_idx: usize,
    sqrt_weight: f64,
}

fn clamp_to_bounds(val: f64, lb: Option<f64>, ub: Option<f64>) -> f64 {
    let mut v = if val.is_finite() { val } else { 0.0 };
    if let Some(l) = lb {
        v = v.max(l);
    }
    if let Some(u) = ub {
        v = v.min(u);
    }
    v
}

fn project_theta(theta: &mut [f64], bounds: &[(Option<f64>, Option<f64>)]) {
    for (v, &(lb, ub)) in theta.iter_mut().zip(bounds.iter()) {
        *v = clamp_to_bounds(*v, lb, ub);
    }
}

fn compute_point_weight(pt: &DataPoint, scheme: WeightingScheme) -> Result<f64, FitError> {
    if let Some(w_exp) = pt.explicit_weight {
        if !w_exp.is_finite() || w_exp <= 0.0 {
            return Err(FitError::ImpossibleWeights { reason: "Explicit observation weight must be finite and strictly positive".to_owned() });
        }
        return Ok(w_exp);
    }
    let w = match scheme {
        WeightingScheme::None => 1.0,
        WeightingScheme::RelativeSquare => {
            if pt.y == 0.0 || !pt.y.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/Y^2 weighting is undefined where Y == 0".to_owned() });
            }
            1.0 / (pt.y * pt.y)
        }
        WeightingScheme::Relative => {
            if pt.y == 0.0 || !pt.y.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/|Y| weighting is undefined where Y == 0".to_owned() });
            }
            1.0 / pt.y.abs()
        }
        WeightingScheme::GeneralPowerY(k) => {
            if pt.y == 0.0 && k > 0.0 {
                return Err(FitError::ImpossibleWeights { reason: format!("1/|Y|^{k} weighting is undefined where Y == 0") });
            }
            let denom = pt.y.abs().powf(k);
            if !denom.is_finite() || denom <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "General power Y weight underflowed or overflowed".to_owned() });
            }
            1.0 / denom
        }
        WeightingScheme::InverseX => {
            let x0 = pt.x0();
            if x0 == 0.0 || !x0.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/|X| weighting is undefined where X == 0".to_owned() });
            }
            1.0 / x0.abs()
        }
        WeightingScheme::InverseX2 => {
            let x0 = pt.x0();
            if x0 == 0.0 || !x0.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: "1/X^2 weighting is undefined where X == 0".to_owned() });
            }
            1.0 / (x0 * x0)
        }
        WeightingScheme::ObservedSd => {
            let Some(sd) = pt.sd else {
                return Err(FitError::ImpossibleWeights { reason: "1/SD^2 weighting requires pre-summarized SD values".to_owned() });
            };
            if !sd.is_finite() || sd <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "1/SD^2 weighting is undefined where SD <= 0".to_owned() });
            }
            1.0 / (sd * sd)
        }
    };
    if w.is_finite() && w > 0.0 { Ok(w) } else { Err(FitError::ImpossibleWeights { reason: "Weight overflowed f64 range".to_owned() }) }
}

/// Flatten datasets into active observations with verified square-root weights.
fn build_observations(datasets: &[DatasetSeries], scheme: WeightingScheme) -> Result<Vec<ObsRecord>, FitError> {
    let mut obs = Vec::new();
    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        for pt in &ds.points {
            let w = compute_point_weight(pt, scheme)?;
            obs.push(ObsRecord { point: pt.clone(), ds_local_idx, sqrt_weight: w.sqrt() });
        }
    }
    if obs.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(obs)
}

/// Evaluate weighted residual vector `r_i = sqrt_w_i * (y_i - f(x_i; theta))` and predictions.
fn eval_residuals(model: &dyn ParametricModel, layout: &ParameterLayout, obs: &[ObsRecord], theta: &[f64]) -> Option<(Vec<f64>, Vec<f64>, f64)> {
    let mut full_per_ds = Vec::with_capacity(layout.n_datasets);
    for ds_idx in 0..layout.n_datasets {
        full_per_ds.push(layout.expand_params(theta, ds_idx));
    }

    let mut weighted_res = Vec::with_capacity(obs.len());
    let mut y_hat = Vec::with_capacity(obs.len());
    let mut ss = NeumaierSum::new();

    for rec in obs {
        let p = &full_per_ds[rec.ds_local_idx];
        let pred = model.eval(&rec.point.x, p, rec.point.data_set_index)?;
        if !pred.is_finite() {
            return None;
        }
        let r = rec.sqrt_weight * (rec.point.y - pred);
        if !r.is_finite() {
            return None;
        }
        y_hat.push(pred);
        weighted_res.push(r);
        ss.add(r * r);
    }
    let total_ss = ss.total();
    if total_ss.is_finite() { Some((weighted_res, y_hat, total_ss)) } else { None }
}

fn mat_get(m: &DenseMatrix, r: usize, c: usize) -> f64 {
    m.get(r, c).unwrap_or(0.0)
}

fn mat_set(m: &mut DenseMatrix, r: usize, c: usize, val: f64) {
    if let Some(slot) = m.get_mut(r, c) {
        *slot = val;
    }
}

/// Compute weighted Jacobian `J_{i, j} = sqrt_w_i * df_i / dtheta_j` (`N x K`) using analytical
/// `model.grad` when supported, or central/forward finite differences otherwise.
fn compute_jacobian(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    y_hat0: &[f64],
    bounds: &[(Option<f64>, Option<f64>)],
) -> Option<DenseMatrix> {
    let n = obs.len();
    let k = layout.k_free();
    let mut j_mat = DenseMatrix::zeros(n, k).ok()?;
    if k == 0 {
        return Some(j_mat);
    }

    // Try analytical gradient path first across all observations.
    let mut full_per_ds = Vec::with_capacity(layout.n_datasets);
    for ds_idx in 0..layout.n_datasets {
        full_per_ds.push(layout.expand_params(theta, ds_idx));
    }

    let mut grad_buf = vec![0.0_f64; layout.p_model];
    let mut analytical_ok = true;
    for (i, rec) in obs.iter().enumerate() {
        grad_buf.fill(0.0);
        if !model.grad(&rec.point.x, &full_per_ds[rec.ds_local_idx], rec.point.data_set_index, &mut grad_buf) {
            analytical_ok = false;
            break;
        }
        for (p_idx, &g_val) in grad_buf.iter().enumerate().take(layout.p_model) {
            if let ParamBinding::Free(slot) = layout.bindings[rec.ds_local_idx][p_idx] {
                let prev = mat_get(&j_mat, i, slot);
                mat_set(&mut j_mat, i, slot, prev + rec.sqrt_weight * g_val);
            }
        }
    }
    if analytical_ok {
        return Some(j_mat);
    }

    // Central difference Jacobian (with bounded one-sided fallback when near domain/box edges).
    j_mat = DenseMatrix::zeros(n, k).ok()?;
    let eps_central = 1e-6_f64;
    let eps_forward = 1.5e-8_f64;
    let mut work = theta.to_vec();

    for slot in 0..k {
        let info = &layout.free_slots[slot];
        let xj = theta[slot];
        let h_c = eps_central * xj.abs().max(1.0);
        let (lb, ub) = bounds[slot];

        let mut can_plus = ub.is_none_or(|u| xj + h_c <= u);
        let mut can_minus = lb.is_none_or(|l| xj - h_c >= l);

        let mut yp: Option<Vec<f64>> = None;
        let mut ym: Option<Vec<f64>> = None;

        if can_plus {
            work[slot] = xj + h_c;
            yp = eval_predictions_for_slot(model, layout, obs, &work, info.dataset_idx);
            if yp.is_none() {
                can_plus = false;
            }
        }
        if can_minus {
            work[slot] = xj - h_c;
            ym = eval_predictions_for_slot(model, layout, obs, &work, info.dataset_idx);
            if ym.is_none() {
                can_minus = false;
            }
        }
        work[slot] = xj;

        if can_plus
            && can_minus
            && let (Some(fp), Some(fm)) = (yp, ym)
        {
            let inv_2h = 1.0 / (2.0 * h_c);
            for (i, rec) in obs.iter().enumerate() {
                if info.dataset_idx.is_none_or(|d| d == rec.ds_local_idx) {
                    mat_set(&mut j_mat, i, slot, rec.sqrt_weight * (fp[i] - fm[i]) * inv_2h);
                }
            }
            continue;
        }

        // One-sided fallback with smaller step `eps_forward`.
        let h_f = eps_forward * xj.abs().max(1.0);
        let sign = if ub.is_some_and(|u| xj + h_f > u) { -1.0 } else { 1.0 };
        work[slot] = xj + sign * h_f;
        let fp_opt = eval_predictions_for_slot(model, layout, obs, &work, info.dataset_idx);
        work[slot] = xj;
        let fp = fp_opt?;
        let inv_h = sign / h_f;
        for (i, rec) in obs.iter().enumerate() {
            if info.dataset_idx.is_none_or(|d| d == rec.ds_local_idx) {
                mat_set(&mut j_mat, i, slot, rec.sqrt_weight * (fp[i] - y_hat0[i]) * inv_h);
            }
        }
    }
    Some(j_mat)
}

fn eval_predictions_for_slot(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta: &[f64],
    slot_ds: Option<usize>,
) -> Option<Vec<f64>> {
    let mut full_per_ds = Vec::with_capacity(layout.n_datasets);
    for ds_idx in 0..layout.n_datasets {
        full_per_ds.push(layout.expand_params(theta, ds_idx));
    }
    let mut out = vec![0.0_f64; obs.len()];
    for (i, rec) in obs.iter().enumerate() {
        if slot_ds.is_none_or(|d| d == rec.ds_local_idx) {
            let v = model.eval(&rec.point.x, &full_per_ds[rec.ds_local_idx], rec.point.data_set_index)?;
            if !v.is_finite() {
                return None;
            }
            out[i] = v;
        }
    }
    Some(out)
}

#[derive(Debug, Clone)]
struct LmRunOutput {
    theta: Vec<f64>,
    weighted_res: Vec<f64>,
    y_hat: Vec<f64>,
    weighted_ss: f64,
    j_mat: DenseMatrix,
    status: FitStatus,
    iterations: usize,
}

/// Core Moré (1978) trust-region Levenberg–Marquardt solver using QR decomposition on the
/// augmented Jacobian `[J; sqrt(lambda) * D]` (`lumencraft_math::linalg::qr`).
fn run_trust_region_lm(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    max_iters: usize,
    init_factor: f64,
) -> Result<LmRunOutput, FitError> {
    let n = obs.len();
    let k = layout.k_free();
    let bounds = layout.bounds();
    let mut theta = theta_init.to_vec();
    project_theta(&mut theta, &bounds);

    let Some((mut w_res, mut y_hat, mut ss_curr)) = eval_residuals(model, layout, obs, &theta) else {
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

    let mut diag = vec![0.0_f64; k];
    let mut delta = 0.0_f64;
    let mut iter = 0_usize;
    let mut j_mat = DenseMatrix::zeros(n, k)?;

    while iter < max_iters {
        j_mat = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds)
            .ok_or_else(|| FitError::BadInitialValues { reason: "Jacobian evaluation returned non-finite values".to_owned() })?;

        // Update column scaling D_j = max(D_j, ||J_{:, j}||_2).
        for (j, d_j) in diag.iter_mut().enumerate() {
            let mut col_ss = NeumaierSum::new();
            for i in 0..n {
                let v = mat_get(&j_mat, i, j);
                col_ss.add(v * v);
            }
            let cnorm = col_ss.total().sqrt();
            let eff = if cnorm > 1e-12 { cnorm } else { 1.0 };
            *d_j = if *d_j == 0.0 { eff } else { d_j.max(eff) };
        }

        if iter == 0 {
            let xnorm = scaled_l2_norm(&diag, &theta);
            delta = init_factor * if xnorm > 0.0 { xnorm } else { 1.0 };
        }

        // Check projected/scaled gradient norm (gtol).
        let mut max_g_cos = 0.0_f64;
        if ss_curr > 0.0 {
            let fnorm = ss_curr.sqrt();
            for j in 0..k {
                let mut col_dot = NeumaierSum::new();
                let mut col_ss = NeumaierSum::new();
                for (i, &w_r) in w_res.iter().enumerate().take(n) {
                    let jv = mat_get(&j_mat, i, j);
                    col_dot.add(jv * w_r);
                    col_ss.add(jv * jv);
                }
                let cnorm = col_ss.total().sqrt();
                if cnorm > 0.0 {
                    let grad_j = col_dot.total();
                    let at_lo = bounds[j].0.is_some_and(|l| (theta[j] - l).abs() <= 1e-12 && grad_j < 0.0);
                    let at_hi = bounds[j].1.is_some_and(|u| (u - theta[j]).abs() <= 1e-12 && grad_j > 0.0);
                    if !at_lo && !at_hi {
                        max_g_cos = max_g_cos.max((grad_j / (cnorm * fnorm)).abs());
                    }
                }
            }
        } else {
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

        // Inner trust-region radius reduction loop.
        let mut accepted = false;
        let mut lambda = 0.0_f64;
        for _ in 0..40 {
            let (step, lam_used) = solve_trust_region_subproblem(&j_mat, &w_res, &diag, delta, lambda)?;
            lambda = lam_used;

            let mut theta_trial: Vec<f64> = theta.iter().zip(step.iter()).map(|(a, b)| a + b).collect();
            project_theta(&mut theta_trial, &bounds);
            let actual_step: Vec<f64> = theta_trial.iter().zip(theta.iter()).map(|(a, b)| a - b).collect();
            let p_norm = scaled_l2_norm(&diag, &actual_step);
            if iter == 0 {
                delta = delta.min(p_norm.max(1e-12));
            }

            // Predicted reduction: ||J p||^2 + 2 lambda ||D p||^2 (or 2 r^T J p - ||J p||^2 under clamping).
            let jp = matvec(&j_mat, &actual_step);
            let jp_ss: f64 = jp.iter().map(|v| v * v).sum();
            let mut r_dot_jp = 0.0_f64;
            for i in 0..n {
                r_dot_jp += w_res[i] * jp[i];
            }
            let pred_num = (2.0 * r_dot_jp - jp_ss).max(0.0);
            let pred_red = if ss_curr > 0.0 { pred_num / ss_curr } else { 0.0 };

            let (act_red, trial_eval) = match eval_residuals(model, layout, obs, &theta_trial) {
                Some((r_t, y_t, ss_t)) => {
                    let act = if ss_t < ss_curr { 1.0 - ss_t / ss_curr } else { -1.0 };
                    (act, Some((r_t, y_t, ss_t)))
                }
                None => (-1.0, None),
            };

            let rho = if pred_red > 0.0 && trial_eval.is_some() {
                act_red / pred_red
            } else if act_red > 0.0 {
                1.0
            } else {
                -1.0
            };

            if rho < 0.25 {
                let mut temp = 0.5_f64;
                if let Some((_, _, ss_t)) = &trial_eval
                    && ss_t > &ss_curr
                {
                    let dir_deriv = -r_dot_jp;
                    let ratio_t = 0.5 * dir_deriv / (dir_deriv + 0.5 * (ss_curr - ss_t));
                    if ratio_t.is_finite() {
                        temp = ratio_t.clamp(0.1, 0.5);
                    }
                }
                delta = temp * delta.min(p_norm * 10.0).max(delta * 0.1);
                lambda = (lambda / temp.max(0.1)).max(1e-12);
            } else if lambda == 0.0 || rho > 0.75 {
                delta = delta.max(2.0 * p_norm);
                lambda *= 0.5;
            }

            if rho >= 1e-4
                && let Some((r_t, y_t, ss_t)) = trial_eval
            {
                theta = theta_trial;
                w_res = r_t;
                y_hat = y_t;
                ss_curr = ss_t;
                iter += 1;
                accepted = true;

                let x_norm = scaled_l2_norm(&diag, &theta).max(1.0);
                if (act_red.abs() <= ftol && pred_red <= ftol && rho <= 2.0) || p_norm <= xtol * x_norm {
                    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
                    return Ok(LmRunOutput {
                        theta,
                        weighted_res: w_res,
                        y_hat,
                        weighted_ss: ss_curr,
                        j_mat: j_final,
                        status: FitStatus::Converged,
                        iterations: iter,
                    });
                }
                break;
            }

            let x_norm = scaled_l2_norm(&diag, &theta).max(1.0);
            if p_norm <= xtol * x_norm || delta <= 1e-15 * x_norm {
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
        }

        if !accepted {
            break;
        }
    }

    let j_final = compute_jacobian(model, layout, obs, &theta, &y_hat, &bounds).unwrap_or(j_mat);
    Ok(LmRunOutput {
        theta,
        weighted_res: w_res,
        y_hat,
        weighted_ss: ss_curr,
        j_mat: j_final,
        status: if iter >= max_iters { FitStatus::HitMaxIters } else { FitStatus::Converged },
        iterations: iter,
    })
}

/// Solve the Moré trust-region subproblem `(J^T J + lambda D^2) p = J^T r` with `||D p|| <= delta`
/// via QR decomposition of the augmented matrix `[J; sqrt(lambda) D]`.
fn solve_trust_region_subproblem(j_mat: &DenseMatrix, r: &[f64], diag: &[f64], delta: f64, lambda_hint: f64) -> Result<(Vec<f64>, f64), FitError> {
    let k = j_mat.ncols();
    if let Ok(p_gn) = solve_damped_qr(j_mat, r, diag, 0.0) {
        let p_norm = scaled_l2_norm(diag, &p_gn);
        if p_norm <= 1.1 * delta && p_gn.iter().all(|v| v.is_finite()) {
            return Ok((p_gn, 0.0));
        }
    }

    // Upper bound on lambda from ||J^T r / D|| / delta.
    let mut grad_scaled_ss = 0.0_f64;
    for (j, &dj) in diag.iter().enumerate().take(k) {
        let mut g_j = 0.0_f64;
        for (i, &ri) in r.iter().enumerate() {
            g_j += mat_get(j_mat, i, j) * ri;
        }
        let s = g_j / dj;
        grad_scaled_ss += s * s;
    }
    let paru = (grad_scaled_ss.sqrt() / delta.max(1e-30)).max(1e-6);
    let mut parl = 0.0_f64;
    let mut lam = lambda_hint.clamp(1e-12, paru).max(0.001 * paru);

    let mut best_p = vec![0.0_f64; k];
    for _ in 0..10 {
        if lam == 0.0 {
            lam = (0.001 * paru).max(1e-12);
        }
        let p = solve_damped_qr(j_mat, r, diag, lam)?;
        let p_norm = scaled_l2_norm(diag, &p);
        let fp = p_norm - delta;
        best_p = p;
        if fp.abs() <= 0.1 * delta {
            return Ok((best_p, lam));
        }
        if fp > 0.0 {
            parl = parl.max(lam);
        } else {
            // Shrink upper bracket carefully.
        }
        let corr = lam * (fp / delta);
        let next_lam = if fp > 0.0 { (lam + corr.abs()).min(paru.max(lam * 4.0)) } else { (0.5 * (lam + parl)).max(0.1 * lam) };
        if (next_lam - lam).abs() <= 1e-12 * lam.max(1.0) {
            return Ok((best_p, next_lam));
        }
        lam = next_lam;
    }
    Ok((best_p, lam))
}

/// QR solve of `min || [J; sqrt(lambda) D] p - [r; 0] ||_2`.
fn solve_damped_qr(j_mat: &DenseMatrix, r: &[f64], diag: &[f64], lambda: f64) -> Result<Vec<f64>, FitError> {
    let n = j_mat.nrows();
    let k = j_mat.ncols();
    if lambda <= 0.0 {
        let pqr = qr_col_pivoted(j_mat)?;
        if pqr.rank() < k {
            return Err(FitError::InvalidParameter { name: "jacobian", reason: "Rank-deficient Gauss-Newton step".to_owned() });
        }
        return Ok(pqr.solve_basic(r)?);
    }
    let sqrt_lam = lambda.sqrt();
    let mut aug = DenseMatrix::zeros(n + k, k)?;
    for i in 0..n {
        for j in 0..k {
            mat_set(&mut aug, i, j, mat_get(j_mat, i, j));
        }
    }
    for (j, &dj) in diag.iter().enumerate().take(k) {
        mat_set(&mut aug, n + j, j, sqrt_lam * dj);
    }
    let mut rhs = Vec::with_capacity(n + k);
    rhs.extend_from_slice(r);
    rhs.resize(n + k, 0.0);

    let qr_res = qr(&aug)?;
    Ok(qr_res.solve_basic(&rhs)?)
}

fn scaled_l2_norm(diag: &[f64], v: &[f64]) -> f64 {
    let mut s = NeumaierSum::new();
    for (&d, &x) in diag.iter().zip(v.iter()) {
        let prod = d * x;
        s.add(prod * prod);
    }
    s.total().sqrt()
}

fn matvec(a: &DenseMatrix, v: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0_f64; a.nrows()];
    for (i, row_out) in out.iter_mut().enumerate() {
        let mut acc = NeumaierSum::new();
        for (j, &vj) in v.iter().enumerate() {
            acc.add(mat_get(a, i, j) * vj);
        }
        *row_out = acc.total();
    }
    out
}

/// Nelder–Mead simplex warm-start / local rescue for difficult starts or non-finite initial points.
fn nelder_mead_rescue(model: &dyn ParametricModel, layout: &ParameterLayout, obs: &[ObsRecord], seed: &[f64], max_evals: usize) -> Option<Vec<f64>> {
    let k = layout.k_free();
    if k == 0 {
        return Some(Vec::new());
    }
    let bounds = layout.bounds();
    let eval_obj = |th: &[f64]| -> f64 {
        let mut clamped = th.to_vec();
        project_theta(&mut clamped, &bounds);
        eval_residuals(model, layout, obs, &clamped).map_or(f64::INFINITY, |(_, _, ss)| ss)
    };

    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(k + 1);
    let mut v0 = seed.to_vec();
    project_theta(&mut v0, &bounds);
    let f0 = eval_obj(&v0);
    simplex.push((v0.clone(), f0));

    for j in 0..k {
        let mut vj = v0.clone();
        let step = if vj[j].abs() > 1e-8 { 0.1 * vj[j] } else { 0.05 };
        vj[j] += step;
        project_theta(&mut vj, &bounds);
        let fj = eval_obj(&vj);
        simplex.push((vj, fj));
    }

    let mut evals = k + 1;
    while evals < max_evals {
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
        let xr: Vec<f64> = centroid.iter().zip(worst.iter()).map(|(c, w)| c + (c - w)).collect();
        let fr = eval_obj(&xr);
        evals += 1;

        if fr < simplex[0].1 {
            let xe: Vec<f64> = centroid.iter().zip(xr.iter()).map(|(c, r)| c + 2.0 * (r - c)).collect();
            let fe = eval_obj(&xe);
            evals += 1;
            simplex[k] = if fe < fr { (xe, fe) } else { (xr, fr) };
        } else if fr < simplex[k - 1].1 {
            simplex[k] = (xr, fr);
        } else {
            let use_outside = fr < simplex[k].1;
            let base = if use_outside { &xr } else { &simplex[k].0 };
            let f_base = if use_outside { fr } else { simplex[k].1 };
            let xc: Vec<f64> = centroid.iter().zip(base.iter()).map(|(c, b)| c + 0.5 * (b - c)).collect();
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

/// Multi-factor trust-region LM driver with optional simplex rescue for hard starts.
fn solve_with_warm_start(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    theta_init: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    let mut start = theta_init.to_vec();
    if eval_residuals(model, layout, obs, &start).is_none() {
        if config.use_simplex_warm_start
            && let Some(rescued) = nelder_mead_rescue(model, layout, obs, &start, 240)
        {
            start = rescued;
        } else {
            return Err(FitError::BadInitialValues { reason: "Starting parameters evaluate outside model domain".to_owned() });
        }
    }

    let mut best = run_trust_region_lm(model, layout, obs, &start, config.max_iters, 100.0)?;
    // If primary run hit max iters or suspiciously poor gradient / flat plateau, try smaller step
    // trust-region factor `1.0` and optional Nelder-Mead restart purely from `start` or `best.theta`.
    if config.use_simplex_warm_start && (best.status != FitStatus::Converged || best.weighted_ss > 1e-3 * obs_y_ss(obs)) {
        if let Ok(alt1) = run_trust_region_lm(model, layout, obs, &start, config.max_iters, 1.0)
            && alt1.weighted_ss < best.weighted_ss
        {
            best = alt1;
        }
        if let Some(nm) = nelder_mead_rescue(model, layout, obs, &best.theta, 400)
            && let Ok(alt2) = run_trust_region_lm(model, layout, obs, &nm, config.max_iters, 1.0)
            && alt2.weighted_ss < best.weighted_ss
        {
            best = alt2;
        }
        if best.weighted_ss > 1e-2 * obs_y_ss(obs)
            && let Some(nm_start) = nelder_mead_rescue(model, layout, obs, &start, 600)
            && let Ok(alt3) = run_trust_region_lm(model, layout, obs, &nm_start, config.max_iters, 0.5)
            && alt3.weighted_ss < best.weighted_ss
        {
            best = alt3;
        }
    }
    Ok(best)
}

fn obs_y_ss(obs: &[ObsRecord]) -> f64 {
    let mut acc = NeumaierSum::new();
    for o in obs {
        acc.add(o.sqrt_weight * o.sqrt_weight * o.point.y * o.point.y);
    }
    acc.total().max(1.0)
}

/// Execute nonlinear regression for one or more datasets using any built-in or DSL [`ParametricModel`].
pub fn fit_nonlinear(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
) -> Result<NonlinearFitResult, FitError> {
    if datasets.is_empty() {
        return Err(FitError::EmptyInput);
    }
    if matches!(config.method, FitMethod::RobustCauchy | FitMethod::RoutOutlierElimination { .. }) && datasets.iter().any(|d| d.is_pre_summarized) {
        return Err(FitError::RequiresRawReplicates);
    }

    let layout = ParameterLayout::build(model, datasets, hooks, &config.parameter_rules)?;
    let k_free = layout.k_free();
    let mut obs = build_observations(datasets, config.weighting)?;
    if obs.len() <= k_free {
        return Err(FitError::InsufficientData { required: k_free + 1, actual: obs.len() });
    }

    let mut init_theta: Vec<f64> = layout.free_slots.iter().map(|s| s.init).collect();
    let mut rout_result: Option<RoutDetectionResult> = None;
    let mut n_outliers_excluded = 0_usize;

    let mut lm_out = match config.method {
        FitMethod::LeastSquares => solve_with_warm_start(model, &layout, &obs, &init_theta, config)?,
        FitMethod::PoissonDeviance => fit_poisson_irls(model, &layout, &obs, &init_theta, config)?,
        FitMethod::RobustCauchy => fit_cauchy_irls(model, &layout, &obs, &init_theta, config)?,
        FitMethod::RoutOutlierElimination { fdr_q_percent } => {
            let cauchy_out = fit_cauchy_irls(model, &layout, &obs, &init_theta, config)?;
            let pts: Vec<DataPoint> = obs.iter().map(|o| o.point.clone()).collect();
            let raw_res: Vec<f64> = obs.iter().zip(cauchy_out.y_hat.iter()).map(|(o, yh)| o.point.y - yh).collect();
            let rout = rout_detect_outliers(&pts, &raw_res, k_free, fdr_q_percent)?;
            let mut is_out = vec![false; obs.len()];
            for &idx in &rout.outlier_indices {
                is_out[idx] = true;
            }
            let clean_obs: Vec<ObsRecord> = obs.iter().enumerate().filter(|&(i, _)| !is_out[i]).map(|(_, r)| r.clone()).collect();
            if clean_obs.len() <= k_free {
                return Err(FitError::InsufficientData { required: k_free + 1, actual: clean_obs.len() });
            }
            n_outliers_excluded = obs.len() - clean_obs.len();
            obs = clean_obs;
            init_theta = cauchy_out.theta;
            rout_result = Some(rout);
            solve_with_warm_start(model, &layout, &obs, &init_theta, config)?
        }
    };

    let n_obs = obs.len();
    let df = n_obs - k_free;
    let cov = compute_covariance_diagnostics(&lm_out.j_mat, lm_out.weighted_ss, df, &lm_out.theta, config.confidence)?;

    if cov.ambiguous && lm_out.status == FitStatus::Converged {
        lm_out.status = FitStatus::Ambiguous;
    }

    // Hougaard's skewness (for moderate parameter dimensions where Hessian 3-tensor is tractable).
    let hougaard = if config.compute_hougaard && k_free > 0 && k_free <= 10 && !cov.ambiguous {
        let sqrt_w: Vec<f64> = obs.iter().map(|o| o.sqrt_weight).collect();
        let cov_unscaled = compute_unscaled_cov(&lm_out.j_mat);
        if let Some(cov_u) = cov_unscaled {
            let sigma = (lm_out.weighted_ss / (df as f64)).sqrt();
            compute_hougaard_skewness(&lm_out.theta, &sqrt_w, &lm_out.j_mat, &cov_u, sigma, |th| {
                eval_residuals(model, &layout, &obs, th).map(|(_, yh, _)| yh)
            })
        } else {
            vec![0.0; k_free]
        }
    } else {
        vec![0.0; k_free]
    };

    // Venzon-Moolgavkar Profile Likelihood Confidence Intervals.
    let bounds = layout.bounds();
    let profile_cis = if config.compute_profile_ci && k_free > 0 && !cov.ambiguous {
        compute_profile_likelihood_cis(&lm_out.theta, lm_out.weighted_ss, df, &cov, &bounds, config.confidence, |fixed_idx, fixed_val, seed| {
            solve_profile_subproblem(model, &layout, &obs, seed, fixed_idx, fixed_val)
        })
    } else {
        cov.symmetric_ci_lower
            .iter()
            .zip(cov.symmetric_ci_upper.iter())
            .map(
                |(&lo, &hi)| {
                    if lo.is_finite() && hi.is_finite() { (CiBound::Finite(lo), CiBound::Finite(hi)) } else { (CiBound::Open, CiBound::Open) }
                },
            )
            .collect()
    };

    let parameters = build_fitted_parameter_summaries(&layout, &lm_out.theta, &cov, &hougaard, &profile_cis);

    // Unweighted RSS & R^2 across clean observations.
    let mut raw_rss_acc = NeumaierSum::new();
    let mut y_mean_acc = NeumaierSum::new();
    for (rec, &yh) in obs.iter().zip(lm_out.y_hat.iter()) {
        let diff = rec.point.y - yh;
        raw_rss_acc.add(diff * diff);
        y_mean_acc.add(rec.point.y);
    }
    let raw_rss = raw_rss_acc.total();
    let y_mean = y_mean_acc.total() / (n_obs as f64);
    let mut tss_acc = NeumaierSum::new();
    for rec in &obs {
        let d = rec.point.y - y_mean;
        tss_acc.add(d * d);
    }
    let tss = tss_acc.total();
    let r_squared = if tss > 0.0 { 1.0 - raw_rss / tss } else { 1.0 };
    let rsd = (raw_rss / (df as f64)).sqrt();

    let ic = compute_information_criteria(lm_out.weighted_ss, n_obs, k_free).ok();
    let cov_unscaled = compute_unscaled_cov(&lm_out.j_mat);
    let t_crit = student_t_two_sided_isf(config.confidence.alpha(), df as f64).unwrap_or(1.96);
    let sigma_w = (lm_out.weighted_ss / (df as f64)).sqrt();

    let assembled = assemble_dataset_results(
        model,
        &layout,
        datasets,
        &obs,
        &lm_out,
        &DatasetAssemblyConfig { cov_unscaled: cov_unscaled.as_ref(), sigma_w, t_crit, rsd, ambiguous: cov.ambiguous, solver: config },
    );
    let ds_results = assembled.ds_results;
    let derived_scalars = assembled.derived_scalars;
    let interpolated_x = assembled.interpolated_x;
    let interpolated_y = assembled.interpolated_y;

    let raw_residuals_all: Vec<f64> = obs.iter().zip(lm_out.y_hat.iter()).map(|(o, yh)| o.point.y - yh).collect();
    let clean_points: Vec<DataPoint> = obs.iter().map(|o| o.point.clone()).collect();

    let runs_res = if config.compute_runs_test {
        let mut pairs: Vec<(f64, f64)> = obs.iter().zip(raw_residuals_all.iter()).map(|(o, &r)| (o.point.x0(), r)).collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let ordered_res: Vec<f64> = pairs.into_iter().map(|(_, r)| r).collect();
        Some(runs_test(&ordered_res))
    } else {
        None
    };

    let lof_res = if config.compute_lack_of_fit { replicates_lack_of_fit(&clean_points, lm_out.weighted_ss, df) } else { None };

    let normality_res =
        if config.compute_normality { shapiro_wilk_test(&raw_residuals_all).or_else(|| dagostino_pearson_test(&raw_residuals_all)) } else { None };

    Ok(NonlinearFitResult {
        equation_id: model.id().to_owned(),
        equation_name: model.name().to_owned(),
        status: lm_out.status,
        iterations: lm_out.iterations,
        n_obs,
        n_outliers_excluded,
        k_free,
        df,
        rss: raw_rss,
        weighted_ss: lm_out.weighted_ss,
        rsd,
        r_squared,
        aic_c: ic.as_ref().map(|x| x.aicc),
        bic: ic.as_ref().map(|x| x.bic),
        parameters,
        fixed_constants: layout.fixed_constants,
        covariance: cov,
        datasets: ds_results,
        derived_scalars,
        runs_test: runs_res,
        lack_of_fit: lof_res,
        normality: normality_res,
        rout: rout_result,
        interpolated_x,
        interpolated_y,
    })
}

fn build_fitted_parameter_summaries(
    layout: &ParameterLayout,
    theta: &[f64],
    cov: &CovarianceDiagnostics,
    hougaard: &[f64],
    profile_cis: &[(CiBound, CiBound)],
) -> Vec<FittedParameterSummary> {
    let mut parameters = Vec::with_capacity(layout.k_free());
    for (idx, slot) in layout.free_slots.iter().enumerate() {
        let est = theta[idx];
        let mut notes = Vec::new();
        if slot.is_shared {
            notes.push(ParameterNote::SharedGlobal);
        }
        if slot.lower.is_some_and(|l| (est - l).abs() <= 1e-9 * est.abs().max(1.0)) {
            notes.push(ParameterNote::HitLowerBound);
        }
        if slot.upper.is_some_and(|u| (u - est).abs() <= 1e-9 * est.abs().max(1.0)) {
            notes.push(ParameterNote::HitUpperBound);
        }
        if cov.dependencies.get(idx).copied().unwrap_or(0.0) > 0.9999 {
            notes.push(ParameterNote::Ambiguous);
        }
        if matches!(profile_cis[idx].0, CiBound::Open) || matches!(profile_cis[idx].1, CiBound::Open) {
            notes.push(ParameterNote::UnboundedCi);
        }

        parameters.push(FittedParameterSummary {
            slot_index: idx,
            name: slot.name.clone(),
            dataset_index: slot.dataset_idx,
            estimate: est,
            se: cov.standard_errors.get(idx).copied().unwrap_or(f64::NAN),
            ci_lower: profile_cis[idx].0,
            ci_upper: profile_cis[idx].1,
            symmetric_ci_lower: cov.symmetric_ci_lower.get(idx).copied().unwrap_or(f64::NAN),
            symmetric_ci_upper: cov.symmetric_ci_upper.get(idx).copied().unwrap_or(f64::NAN),
            dependency: cov.dependencies.get(idx).copied().unwrap_or(0.0),
            hougaard_g1: hougaard.get(idx).copied().unwrap_or(0.0),
            notes,
        });
    }
    parameters
}

struct DatasetAssemblyConfig<'a> {
    cov_unscaled: Option<&'a DenseMatrix>,
    sigma_w: f64,
    t_crit: f64,
    rsd: f64,
    ambiguous: bool,
    solver: &'a SolverConfig,
}

struct AssembledDatasets {
    ds_results: Vec<FittedDatasetResult>,
    derived_scalars: Vec<Vec<DerivedScalar>>,
    interpolated_x: Vec<InterpolatedUnknown>,
    interpolated_y: Vec<InterpolatedUnknown>,
}

fn assemble_dataset_results(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    datasets: &[DatasetSeries],
    obs: &[ObsRecord],
    lm_out: &LmRunOutput,
    cfg: &DatasetAssemblyConfig<'_>,
) -> AssembledDatasets {
    let k_free = layout.k_free();
    let mut ds_results = Vec::with_capacity(datasets.len());
    let mut derived_scalars = Vec::with_capacity(datasets.len());
    let mut interpolated_x = Vec::new();
    let mut interpolated_y = Vec::new();

    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let ds_params = layout.expand_params(&lm_out.theta, ds_local_idx);
        derived_scalars.push(model.derived_transforms(&ds_params));

        let mut ds_y_hat = Vec::new();
        let mut ds_res = Vec::new();
        let mut ds_w_res = Vec::new();
        let mut ds_rss = NeumaierSum::new();
        let mut ds_y_vals = Vec::new();
        for (i, rec) in obs.iter().enumerate() {
            if rec.ds_local_idx == ds_local_idx {
                let yh = lm_out.y_hat[i];
                let r = rec.point.y - yh;
                ds_y_hat.push(yh);
                ds_res.push(r);
                ds_w_res.push(lm_out.weighted_res[i]);
                ds_rss.add(r * r);
                ds_y_vals.push(rec.point.y);
            }
        }
        let ds_ss = ds_rss.total();
        let ds_r2 = if ds_y_vals.len() > 1 {
            let m = ds_y_vals.iter().sum::<f64>() / (ds_y_vals.len() as f64);
            let dtss: f64 = ds_y_vals.iter().map(|v| (v - m) * (v - m)).sum();
            if dtss > 0.0 { 1.0 - ds_ss / dtss } else { 1.0 }
        } else {
            1.0
        };
        let ds_sy_x = if ds_y_vals.len() > k_free { (ds_ss / ((ds_y_vals.len() - k_free) as f64)).sqrt() } else { cfg.rsd };

        let converged_or_ambig = lm_out.status == FitStatus::Converged || lm_out.status == FitStatus::Ambiguous;
        let curve_overlay = if cfg.solver.compute_bands {
            Some(build_curve_overlay(
                model,
                layout,
                &lm_out.theta,
                &ds_params,
                ds,
                cfg.cov_unscaled,
                cfg.sigma_w,
                cfg.t_crit,
                cfg.solver.compute_prediction_bands,
                converged_or_ambig,
                cfg.ambiguous,
            ))
        } else {
            None
        };

        if cfg.solver.interpolate_unknowns {
            for &(row, y_target) in &ds.unknowns_y {
                interpolated_x.push(interpolate_x_from_y(
                    model,
                    layout,
                    &lm_out.theta,
                    &ds_params,
                    ds,
                    row,
                    y_target,
                    cfg.cov_unscaled,
                    cfg.sigma_w,
                    cfg.t_crit,
                ));
            }
            for &(row, x_val) in &ds.unknowns_x {
                interpolated_y.push(interpolate_y_from_x(
                    model,
                    layout,
                    &lm_out.theta,
                    &ds_params,
                    ds,
                    row,
                    x_val,
                    cfg.cov_unscaled,
                    cfg.sigma_w,
                    cfg.t_crit,
                ));
            }
        }

        ds_results.push(FittedDatasetResult {
            data_set_index: ds.data_set_index,
            label: ds.label.clone(),
            params: ds_params,
            param_names: model.param_names().to_vec(),
            y_hat: ds_y_hat,
            residuals: ds_res,
            weighted_residuals: ds_w_res,
            ss_residual: ds_ss,
            r_squared: ds_r2,
            sy_x: ds_sy_x,
            curve_overlay,
        });
    }

    AssembledDatasets { ds_results, derived_scalars, interpolated_x, interpolated_y }
}

/// Compare primary fitted model against an alternative nested/non-nested model via Extra-SS *F*
/// or $\text{AIC}_c$ (`ModelComparisonSpec`).
pub fn compare_models(
    primary: &NonlinearFitResult,
    alt_model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    hooks: &HookContext,
    config: &SolverConfig,
    spec: &ModelComparisonSpec,
) -> Result<(NonlinearFitResult, ModelComparisonOutcome), FitError> {
    let alt = fit_nonlinear(alt_model, datasets, hooks, config)?;
    let outcome = match spec {
        ModelComparisonSpec::ExtraSumOfSquaresF { .. } => {
            let (ss_simple, df_simple, ss_complex, df_complex) = if primary.df >= alt.df {
                (primary.weighted_ss, primary.df, alt.weighted_ss, alt.df)
            } else {
                (alt.weighted_ss, alt.df, primary.weighted_ss, primary.df)
            };
            ModelComparisonOutcome::ExtraSumOfSquaresF(compare_models_f_test(
                ss_simple,
                df_simple,
                ss_complex,
                df_complex,
                config.confidence.alpha(),
            )?)
        }
        ModelComparisonSpec::Aicc { .. } => {
            ModelComparisonOutcome::Aicc(compare_models_aicc(primary.weighted_ss, primary.k_free, alt.weighted_ss, alt.k_free, primary.n_obs)?)
        }
    };
    Ok((alt, outcome))
}

fn fit_poisson_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    for rec in obs {
        if rec.point.y < 0.0 || !rec.point.y.is_finite() {
            return Err(FitError::ImpossibleWeights { reason: "Poisson deviance regression requires non-negative counts (Y >= 0)".to_owned() });
        }
    }
    let mut theta = init_theta.to_vec();
    let mut work_obs = obs.to_vec();
    let mut last_out = solve_with_warm_start(model, layout, &work_obs, &theta, config)?;
    let mut prev_dev = poisson_deviance(&work_obs, &last_out.y_hat);

    for _ in 0..12 {
        theta = last_out.theta.clone();
        for (rec, &yh) in work_obs.iter_mut().zip(last_out.y_hat.iter()) {
            let mu = yh.max(1e-8);
            rec.sqrt_weight = 1.0 / mu.sqrt();
        }
        let next = run_trust_region_lm(model, layout, &work_obs, &theta, config.max_iters, 10.0)?;
        let dev = poisson_deviance(&work_obs, &next.y_hat);
        let rel_change = (prev_dev - dev).abs() / prev_dev.max(1.0);
        last_out = next;
        prev_dev = dev;
        if rel_change < 1e-10 {
            break;
        }
    }
    last_out.weighted_ss = poisson_deviance(&work_obs, &last_out.y_hat);
    Ok(last_out)
}

fn poisson_deviance(obs: &[ObsRecord], y_hat: &[f64]) -> f64 {
    let mut acc = NeumaierSum::new();
    for (rec, &mu) in obs.iter().zip(y_hat.iter()) {
        acc.add(2.0 * deviance_bd0(rec.point.y, mu.max(1e-12)));
    }
    acc.total()
}

fn fit_cauchy_irls(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    init_theta: &[f64],
    config: &SolverConfig,
) -> Result<LmRunOutput, FitError> {
    let mut theta = init_theta.to_vec();
    let mut work_obs = obs.to_vec();
    let mut last_out = solve_with_warm_start(model, layout, &work_obs, &theta, config)?;
    let k_free = layout.k_free();

    for _ in 0..15 {
        let prev_theta = last_out.theta.clone();
        let abs_res: Vec<f64> = obs.iter().zip(last_out.y_hat.iter()).map(|(o, yh)| (o.point.y - yh).abs()).collect();
        let rsdr = compute_rsdr(&abs_res, k_free).max(1e-12);
        for (i, rec) in work_obs.iter_mut().enumerate() {
            let z = abs_res[i] / rsdr;
            let cauchy_w = 1.0 / (1.0 + z * z);
            rec.sqrt_weight = obs[i].sqrt_weight * cauchy_w.sqrt();
        }
        theta = prev_theta.clone();
        last_out = run_trust_region_lm(model, layout, &work_obs, &theta, config.max_iters, 10.0)?;
        let max_step = prev_theta.iter().zip(last_out.theta.iter()).map(|(a, b)| (a - b).abs() / b.abs().max(1.0)).fold(0.0_f64, f64::max);
        if max_step < 1e-8 {
            break;
        }
    }
    Ok(last_out)
}

fn compute_unscaled_cov(j_mat: &DenseMatrix) -> Option<DenseMatrix> {
    if j_mat.ncols() == 0 || j_mat.nrows() < j_mat.ncols() {
        return None;
    }
    let pqr = qr_col_pivoted(j_mat).ok()?;
    if pqr.rank() < j_mat.ncols() {
        return None;
    }
    pqr.xtx_inverse().ok()
}

/// Optimize the remaining `K - 1` nuisance parameters with `theta[fixed_idx] = fixed_val` held constant.
fn solve_profile_subproblem(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    obs: &[ObsRecord],
    seed: &[f64],
    fixed_idx: usize,
    fixed_val: f64,
) -> Option<(f64, Vec<f64>)> {
    let k = layout.k_free();
    if k == 0 || fixed_idx >= k {
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

    let mut work = seed.to_vec();
    work[fixed_idx] = fixed_val;
    if k == 1 {
        let (_, _, ss) = eval_residuals(model, layout, obs, &work)?;
        return Some((ss, work));
    }

    // Build reduced slot mapping (excluding `fixed_idx`).
    let map_indices: Vec<usize> = (0..k).filter(|&i| i != fixed_idx).collect();
    let sub_k = map_indices.len();
    let sub_bounds: Vec<(Option<f64>, Option<f64>)> = map_indices.iter().map(|&i| full_bounds[i]).collect();

    let mut sub_theta: Vec<f64> = map_indices.iter().map(|&i| work[i]).collect();
    project_theta(&mut sub_theta, &sub_bounds);

    let mut diag = vec![1.0_f64; sub_k];
    let mut delta = 1.0_f64;
    let (_, mut y_hat, mut ss_curr) = {
        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
            work[orig] = v;
        }
        eval_residuals(model, layout, obs, &work)?
    };
    let mut w_res = {
        let (r, _, _) = eval_residuals(model, layout, obs, &work)?;
        r
    };

    for iter in 0..40 {
        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
            work[orig] = v;
        }
        let j_full = compute_jacobian(model, layout, obs, &work, &y_hat, &full_bounds)?;
        let mut j_sub = DenseMatrix::zeros(obs.len(), sub_k).ok()?;
        for (sub_j, &orig_j) in map_indices.iter().enumerate() {
            let mut col_ss = 0.0_f64;
            for i in 0..obs.len() {
                let val = mat_get(&j_full, i, orig_j);
                mat_set(&mut j_sub, i, sub_j, val);
                col_ss += val * val;
            }
            let cn = col_ss.sqrt().max(1e-12);
            diag[sub_j] = if iter == 0 { cn } else { diag[sub_j].max(cn) };
        }
        if iter == 0 {
            delta = 10.0 * scaled_l2_norm(&diag, &sub_theta).max(1.0);
        }

        let mut accepted = false;
        let mut lam = 0.0_f64;
        for _ in 0..15 {
            let Ok((step, lam_u)) = solve_trust_region_subproblem(&j_sub, &w_res, &diag, delta, lam) else {
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
            if let Some((r_t, y_t, ss_t)) = eval_residuals(model, layout, obs, &work) {
                let act = ss_curr - ss_t;
                if act >= 0.0 {
                    sub_theta = trial;
                    w_res = r_t;
                    y_hat = y_t;
                    ss_curr = ss_t;
                    accepted = true;
                    delta = delta.max(2.0 * p_norm);
                    if act <= 1e-11 * ss_curr.max(1.0) || p_norm <= 1e-10 {
                        for (&orig, &v) in map_indices.iter().zip(sub_theta.iter()) {
                            work[orig] = v;
                        }
                        return Some((ss_curr, work));
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
    Some((ss_curr, work))
}

#[allow(clippy::too_many_arguments)]
fn build_curve_overlay(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds: &DatasetSeries,
    cov_unscaled: Option<&DenseMatrix>,
    sigma: f64,
    t_crit: f64,
    prediction_bands: bool,
    converged: bool,
    ambiguous: bool,
) -> FittedCurveOverlay {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    for pt in &ds.points {
        let x0 = pt.x0();
        if x0.is_finite() {
            x_min = x_min.min(x0);
            x_max = x_max.max(x0);
        }
    }
    if !x_min.is_finite() || !x_max.is_finite() || (x_max - x_min).abs() < 1e-15 {
        x_min = -1.0;
        x_max = 1.0;
    }

    let n_grid = 101_usize;
    let mut points = Vec::with_capacity(n_grid);
    for i in 0..n_grid {
        let frac = (i as f64) / ((n_grid - 1) as f64);
        let x = x_min + frac * (x_max - x_min);
        let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
            continue;
        };
        if !y.is_finite() {
            continue;
        }
        let (band_lo, band_hi, pred_lo, pred_hi) = if let Some(cov_u) = cov_unscaled {
            let g = delta_grad_at_x(model, layout, theta, ds, x);
            let quad = quadratic_form(cov_u, &g).max(0.0);
            let se_f = sigma * quad.sqrt();
            let b_lo = Some(y - t_crit * se_f);
            let b_hi = Some(y + t_crit * se_f);
            let (p_lo, p_hi) = if prediction_bands {
                let se_p = sigma * (1.0 + quad).sqrt();
                (Some(y - t_crit * se_p), Some(y + t_crit * se_p))
            } else {
                (None, None)
            };
            (b_lo, b_hi, p_lo, p_hi)
        } else {
            (None, None, None, None)
        };
        points.push(CurveSamplePoint { x, y, ci_lower: band_lo, ci_upper: band_hi, pred_lower: pred_lo, pred_upper: pred_hi });
    }

    FittedCurveOverlay { data_set_index: ds.data_set_index, equation_id: model.id().to_owned(), converged, ambiguous, points }
}

fn delta_grad_at_x(model: &dyn ParametricModel, layout: &ParameterLayout, theta: &[f64], ds: &DatasetSeries, x: f64) -> Vec<f64> {
    let k = layout.k_free();
    let mut g = vec![0.0_f64; k];
    let mut work = theta.to_vec();
    for slot in 0..k {
        let info = &layout.free_slots[slot];
        if let Some(d_idx) = info.dataset_idx
            && d_idx != ds.data_set_index
            && layout.n_datasets > 1
        {
            continue;
        }
        let h = 1e-6 * theta[slot].abs().max(1.0);
        work[slot] = theta[slot] + h;
        let fp = model.eval(&[x], &layout.expand_params(&work, 0), ds.data_set_index);
        work[slot] = theta[slot] - h;
        let fm = model.eval(&[x], &layout.expand_params(&work, 0), ds.data_set_index);
        work[slot] = theta[slot];
        if let (Some(p), Some(m)) = (fp, fm) {
            g[slot] = (p - m) / (2.0 * h);
        }
    }
    g
}

fn quadratic_form(m: &DenseMatrix, v: &[f64]) -> f64 {
    let n = v.len().min(m.nrows()).min(m.ncols());
    let mut acc = NeumaierSum::new();
    for i in 0..n {
        for j in 0..n {
            acc.add(v[i] * mat_get(m, i, j) * v[j]);
        }
    }
    acc.total()
}

/// Interpolate unknown `X` from target `Y` along the fitted curve using scan + `brent_root`,
/// returning inverse confidence bounds (`CiBound::Open` when plateauing beyond band limits).
#[allow(clippy::too_many_arguments)]
fn interpolate_x_from_y(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds: &DatasetSeries,
    row: usize,
    y_target: f64,
    cov_unscaled: Option<&DenseMatrix>,
    sigma: f64,
    t_crit: f64,
) -> InterpolatedUnknown {
    let mut x_min = f64::INFINITY;
    let mut x_max = f64::NEG_INFINITY;
    for pt in &ds.points {
        x_min = x_min.min(pt.x0());
        x_max = x_max.max(pt.x0());
    }
    if !x_min.is_finite() || !x_max.is_finite() || (x_max - x_min).abs() < 1e-12 {
        x_min = -10.0;
        x_max = 10.0;
    }
    let span = (x_max - x_min).max(1.0);
    let scan_lo = x_min - 0.5 * span;
    let scan_hi = x_max + 0.5 * span;

    let f_diff = |x: f64| -> f64 { model.eval(&[x], ds_params, ds.data_set_index).map_or(f64::NAN, |y| y - y_target) };
    let Some(x_hat) = find_first_root_in_scan(f_diff, scan_lo, scan_hi) else {
        return InterpolatedUnknown {
            data_set_index: ds.data_set_index,
            row,
            input_value: y_target,
            output_estimate: f64::NAN,
            ci_lower: CiBound::Open,
            ci_upper: CiBound::Open,
            note: Some("Target Y falls outside fitted curve range".to_owned()),
        };
    };

    let (ci_lower, ci_upper) = if let Some(cov_u) = cov_unscaled {
        let band_lo_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds, x);
            let se = sigma * quadratic_form(cov_u, &g).max(0.0).sqrt();
            (y - t_crit * se) - y_target
        };
        let band_hi_fn = |x: f64| -> f64 {
            let Some(y) = model.eval(&[x], ds_params, ds.data_set_index) else {
                return f64::NAN;
            };
            let g = delta_grad_at_x(model, layout, theta, ds, x);
            let se = sigma * quadratic_form(cov_u, &g).max(0.0).sqrt();
            (y + t_crit * se) - y_target
        };
        let r1 = find_nearest_root(band_lo_fn, x_hat, scan_lo, scan_hi);
        let r2 = find_nearest_root(band_hi_fn, x_hat, scan_lo, scan_hi);
        match (r1, r2) {
            (Some(a), Some(b)) => (CiBound::Finite(a.min(b)), CiBound::Finite(a.max(b))),
            (Some(a), None) => {
                if a <= x_hat {
                    (CiBound::Finite(a), CiBound::Open)
                } else {
                    (CiBound::Open, CiBound::Finite(a))
                }
            }
            (None, Some(b)) => {
                if b <= x_hat {
                    (CiBound::Finite(b), CiBound::Open)
                } else {
                    (CiBound::Open, CiBound::Finite(b))
                }
            }
            (None, None) => (CiBound::Open, CiBound::Open),
        }
    } else {
        (CiBound::Open, CiBound::Open)
    };

    InterpolatedUnknown { data_set_index: ds.data_set_index, row, input_value: y_target, output_estimate: x_hat, ci_lower, ci_upper, note: None }
}

#[allow(clippy::too_many_arguments)]
fn interpolate_y_from_x(
    model: &dyn ParametricModel,
    layout: &ParameterLayout,
    theta: &[f64],
    ds_params: &[f64],
    ds: &DatasetSeries,
    row: usize,
    x_val: f64,
    cov_unscaled: Option<&DenseMatrix>,
    sigma: f64,
    t_crit: f64,
) -> InterpolatedUnknown {
    let Some(y_hat) = model.eval(&[x_val], ds_params, ds.data_set_index) else {
        return InterpolatedUnknown {
            data_set_index: ds.data_set_index,
            row,
            input_value: x_val,
            output_estimate: f64::NAN,
            ci_lower: CiBound::Open,
            ci_upper: CiBound::Open,
            note: Some("X outside fitted model domain".to_owned()),
        };
    };
    let (ci_lower, ci_upper) = if let Some(cov_u) = cov_unscaled {
        let g = delta_grad_at_x(model, layout, theta, ds, x_val);
        let se = sigma * quadratic_form(cov_u, &g).max(0.0).sqrt();
        (CiBound::Finite(y_hat - t_crit * se), CiBound::Finite(y_hat + t_crit * se))
    } else {
        (CiBound::Open, CiBound::Open)
    };
    InterpolatedUnknown { data_set_index: ds.data_set_index, row, input_value: x_val, output_estimate: y_hat, ci_lower, ci_upper, note: None }
}

fn find_first_root_in_scan<F: Fn(f64) -> f64>(f: F, lo: f64, hi: f64) -> Option<f64> {
    let steps = 200_usize;
    let mut prev_x = lo;
    let mut prev_y = f(prev_x);
    for i in 1..=steps {
        let x = lo + (i as f64) / (steps as f64) * (hi - lo);
        let y = f(x);
        if prev_y.is_finite() && y.is_finite() {
            if prev_y.abs() < 1e-13 {
                return Some(prev_x);
            }
            if prev_y * y <= 0.0
                && let Ok(root) = brent_root(&f, prev_x, x, RootOptions { abs_tol: 1e-11, rel_tol: 1e-11, max_iters: 80 })
            {
                return Some(root);
            }
        }
        prev_x = x;
        prev_y = y;
    }
    None
}

fn find_nearest_root<F: Fn(f64) -> f64>(f: F, anchor: f64, lo: f64, hi: f64) -> Option<f64> {
    let steps = 200_usize;
    let mut best: Option<f64> = None;
    let mut prev_x = lo;
    let mut prev_y = f(prev_x);
    for i in 1..=steps {
        let x = lo + (i as f64) / (steps as f64) * (hi - lo);
        let y = f(x);
        if prev_y.is_finite()
            && y.is_finite()
            && prev_y * y <= 0.0
            && let Ok(root) = brent_root(&f, prev_x, x, RootOptions { abs_tol: 1e-10, rel_tol: 1e-10, max_iters: 80 })
            && best.is_none_or(|b| (root - anchor).abs() < (b - anchor).abs())
        {
            best = Some(root);
        }
        prev_x = x;
        prev_y = y;
    }
    best
}
