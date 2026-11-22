//! Free-parameter layout (global sharing, constraints, bounds), observation records, and point weighting.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.

use curvebook_doc::{FitMethod, ParameterRule, ParameterScope, WeightingScheme, parse_column_title_numeric_hook};
use curvebook_math::sum::NeumaierSum;

use crate::dsl::ode_fit_budget_exceeded;
use crate::error::{FitError, ParameterNote};
use crate::library::ParametricModel;
use crate::solver::results::SolverConfig;
use crate::types::{DataPoint, DatasetSeries, HookContext, per_dataset_parameter_key, validated_replicate_moments};

#[derive(Debug, Clone)]
pub(in crate::solver) struct FreeSlot {
    pub(in crate::solver) ds_local_idx: Option<usize>,
    pub(in crate::solver) dataset_idx: Option<usize>,
    pub(in crate::solver) name: String,
    pub(in crate::solver) lower: Option<f64>,
    pub(in crate::solver) upper: Option<f64>,
    pub(in crate::solver) init: f64,
    pub(in crate::solver) is_shared: bool,
}

#[derive(Debug, Clone)]
pub(in crate::solver) enum ParamBinding {
    Free(usize),
    Fixed(f64),
}

#[derive(Debug, Clone)]
pub(in crate::solver) struct ParameterLayout {
    pub(in crate::solver) p_model: usize,
    pub(in crate::solver) n_datasets: usize,
    pub(in crate::solver) bindings: Vec<Vec<ParamBinding>>,
    pub(in crate::solver) free_slots: Vec<FreeSlot>,
    pub(in crate::solver) fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
}

impl ParameterLayout {
    pub(in crate::solver) fn build(
        model: &dyn ParametricModel,
        datasets: &[DatasetSeries],
        hooks: &HookContext,
        rules: &[ParameterRule],
    ) -> Result<Self, FitError> {
        let param_names = model.param_names();
        let mut seen_rule_names: Vec<&str> = Vec::with_capacity(rules.len());
        for rule in rules {
            if !param_names.iter().any(|p| p.eq_ignore_ascii_case(&rule.name)) {
                return Err(FitError::InvalidParameter {
                    name: "parameter_rules",
                    reason: format!("Unknown parameter rule `{}` (expected one of: {})", rule.name, param_names.join(", ")),
                });
            }
            if seen_rule_names.iter().any(|prev| prev.eq_ignore_ascii_case(&rule.name)) {
                return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Duplicate parameter rule for `{}`", rule.name) });
            }
            seen_rule_names.push(&rule.name);
        }
        let p_model = param_names.len();
        let n_datasets = datasets.len();
        let all_points: Vec<DataPoint> = datasets.iter().flat_map(|d| d.points.iter().cloned()).collect();
        let default_init = model.initial_guess(&all_points);
        let default_bounds = model.default_bounds();
        let default_rules = model.default_parameter_rules(&all_points);

        let per_ds_init: Vec<Vec<f64>> = datasets.iter().map(|d| model.initial_guess(&d.points)).collect();

        let mut bindings = vec![vec![ParamBinding::Fixed(0.0); p_model]; n_datasets];
        let mut free_slots = Vec::new();
        let mut fixed_constants = Vec::new();

        for (p_idx, pname) in param_names.iter().enumerate() {
            let explicit_rule = rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname));
            let rule = explicit_rule.or_else(|| default_rules.iter().find(|r| r.name.eq_ignore_ascii_case(pname)));
            let def_b = default_bounds.get(p_idx).copied().unwrap_or((None, None));
            let lb = rule.and_then(|r| r.lower_bound).or(def_b.0);
            let ub = rule.and_then(|r| r.upper_bound).or(def_b.1);
            if lb.is_some_and(|l| !l.is_finite()) || ub.is_some_and(|u| !u.is_finite()) {
                return Err(FitError::InvalidParameter { name: "parameter_rules", reason: format!("Non-finite bound for `{pname}`") });
            }
            if let (Some(l), Some(u)) = (lb, ub)
                && l > u
            {
                return Err(FitError::InvalidParameter {
                    name: "parameter_rules",
                    reason: format!("Lower bound ({l}) > upper bound ({u}) for `{pname}`"),
                });
            }
            let equal_bound = match (lb, ub) {
                (Some(l), Some(u)) if l == u => Some(l),
                _ => None,
            };

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
                Some(ParameterScope::PerDatasetConstant(pairs)) => {
                    for &(_, val) in pairs {
                        if !val.is_finite() {
                            return Err(FitError::InvalidParameter {
                                name: "parameter_rules",
                                reason: format!("Non-finite per-dataset constant for `{pname}`"),
                            });
                        }
                    }
                    for (ds_idx, ds) in datasets.iter().enumerate() {
                        if let Some(&(_, val)) = pairs.iter().rev().find(|&&(idx, _)| idx == ds.data_set_index) {
                            bindings[ds_idx][p_idx] = ParamBinding::Fixed(val);
                            let label = if n_datasets > 1 { per_dataset_parameter_key(pname, ds.data_set_index) } else { pname.clone() };
                            fixed_constants.push((label, val, vec![ParameterNote::Constant]));
                        } else if let Some(eq_val) = equal_bound {
                            bindings[ds_idx][p_idx] = ParamBinding::Fixed(eq_val);
                            let label = if n_datasets > 1 { per_dataset_parameter_key(pname, ds.data_set_index) } else { pname.clone() };
                            fixed_constants.push((label, eq_val, vec![ParameterNote::Constant]));
                        } else {
                            let slot = free_slots.len();
                            let init_v =
                                rule.and_then(|r| r.initial_override).unwrap_or_else(|| per_ds_init[ds_idx].get(p_idx).copied().unwrap_or(1.0));
                            free_slots.push(FreeSlot {
                                ds_local_idx: Some(ds_idx),
                                dataset_idx: Some(ds.data_set_index),
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
                Some(ParameterScope::ColumnTitleConstant) => {
                    for (ds_idx, ds) in datasets.iter().enumerate() {
                        let val =
                            ds.column_title_value.or_else(|| parse_column_title_numeric_hook(&ds.label).ok()).filter(|v| v.is_finite()).ok_or_else(
                                || FitError::HookResolutionError {
                                    message: format!("Column title `{}` for `{pname}` has no finite numeric constant", ds.label),
                                },
                            )?;
                        bindings[ds_idx][p_idx] = ParamBinding::Fixed(val);
                        let label = if n_datasets > 1 { per_dataset_parameter_key(pname, ds.data_set_index) } else { pname.clone() };
                        fixed_constants.push((label, val, vec![ParameterNote::Hooked]));
                    }
                }
                Some(ParameterScope::SharedGlobal) => {
                    if let Some(eq_val) = equal_bound {
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Fixed(eq_val);
                        }
                        fixed_constants.push((pname.clone(), eq_val, vec![ParameterNote::Constant]));
                    } else {
                        let slot = free_slots.len();
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        free_slots.push(FreeSlot {
                            ds_local_idx: None,
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
                }
                None | Some(ParameterScope::Local) => {
                    if let Some(eq_val) = equal_bound {
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Fixed(eq_val);
                        }
                        fixed_constants.push((pname.clone(), eq_val, vec![ParameterNote::Constant]));
                    } else if n_datasets <= 1 {
                        let slot = free_slots.len();
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        let ds_ext_idx = datasets.first().map_or(0, |d| d.data_set_index);
                        free_slots.push(FreeSlot {
                            ds_local_idx: Some(0),
                            dataset_idx: Some(ds_ext_idx),
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
                        for (ds_idx, ds) in datasets.iter().enumerate() {
                            let slot = free_slots.len();
                            let init_v =
                                rule.and_then(|r| r.initial_override).unwrap_or_else(|| per_ds_init[ds_idx].get(p_idx).copied().unwrap_or(1.0));
                            free_slots.push(FreeSlot {
                                ds_local_idx: Some(ds_idx),
                                dataset_idx: Some(ds.data_set_index),
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

    pub(in crate::solver) fn k_free(&self) -> usize {
        self.free_slots.len()
    }

    pub(in crate::solver) fn bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        self.free_slots.iter().map(|s| (s.lower, s.upper)).collect()
    }

    pub(in crate::solver) fn expand_params(&self, theta: &[f64], ds_idx: usize) -> Vec<f64> {
        let mut out = vec![0.0_f64; self.p_model];
        self.expand_params_into(theta, ds_idx, &mut out);
        out
    }

    pub(in crate::solver) fn expand_params_into(&self, theta: &[f64], ds_idx: usize, out: &mut [f64]) {
        let row = &self.bindings[ds_idx.min(self.n_datasets - 1)];
        for (dst, b) in out.iter_mut().zip(row.iter()) {
            *dst = match b {
                ParamBinding::Free(slot) => theta[*slot],
                ParamBinding::Fixed(v) => *v,
            };
        }
    }

    pub(in crate::solver) fn fill_all_params(&self, theta: &[f64], flat_out: &mut [f64]) {
        for ds_idx in 0..self.n_datasets {
            let start = ds_idx * self.p_model;
            self.expand_params_into(theta, ds_idx, &mut flat_out[start..(start + self.p_model)]);
        }
    }
}

pub(in crate::solver) fn uses_shared_global(rules: &[ParameterRule]) -> bool {
    rules.iter().any(|r| matches!(r.scope, ParameterScope::SharedGlobal))
}

pub(in crate::solver) fn rules_for_dataset(rules: &[ParameterRule], data_set_index: usize) -> Vec<ParameterRule> {
    rules
        .iter()
        .map(|r| match &r.scope {
            ParameterScope::PerDatasetConstant(pairs) => {
                if let Some(&(_, v)) = pairs.iter().rev().find(|&&(d, _)| d == data_set_index) {
                    ParameterRule { scope: ParameterScope::Constant(v), ..r.clone() }
                } else {
                    ParameterRule { scope: ParameterScope::Local, ..r.clone() }
                }
            }
            _ => r.clone(),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(in crate::solver) struct ObsRecord {
    pub(in crate::solver) point: DataPoint,
    pub(in crate::solver) ds_local_idx: usize,
    pub(in crate::solver) sqrt_weight: f64,
    pub(in crate::solver) rep_n: usize,
    pub(in crate::solver) ss_within: f64,
    pub(in crate::solver) raw_ss_within: f64,
}

pub(in crate::solver) fn uses_replicate_summary_moments(config: &SolverConfig) -> bool {
    config.method == FitMethod::LeastSquares && config.weighting != WeightingScheme::ObservedSd
}

pub(in crate::solver) fn obs_replicate_count(obs: &[ObsRecord]) -> usize {
    obs.iter().map(|o| o.rep_n).sum()
}

pub(in crate::solver) fn obs_ss_within(obs: &[ObsRecord]) -> f64 {
    let mut acc = NeumaierSum::new();
    for o in obs {
        acc.add(o.ss_within);
    }
    acc.total().max(0.0)
}

pub(in crate::solver) fn clamp_to_bounds(val: f64, lb: Option<f64>, ub: Option<f64>) -> f64 {
    let mut v = if val.is_finite() { val } else { 0.0 };
    if let Some(l) = lb {
        v = v.max(l);
    }
    if let Some(u) = ub {
        v = v.min(u);
    }
    v
}

pub(in crate::solver) fn project_theta(theta: &mut [f64], bounds: &[(Option<f64>, Option<f64>)]) {
    for (v, &(lb, ub)) in theta.iter_mut().zip(bounds.iter()) {
        *v = clamp_to_bounds(*v, lb, ub);
    }
}

/// Observation weight `w_i` for weighted least squares (minimise `sum w_i (y_i - f_i)^2`).
///
/// Relative (`1/Y^2`, `1/|Y|`, `1/|Y|^k`), `1/X`, `1/X^2`, and inverse-variance (`1/SD^2`)
/// weighting follow the schemes described by Motulsky & Christopoulos (2004) and the weighted
/// least-squares framework of Seber & Wild (1989). Weights are computed from the observed
/// `Y` (not the fitted curve), and zero / non-finite denominators are rejected as
/// [`FitError::ImpossibleWeights`] rather than clamped.
pub(in crate::solver) fn compute_point_weight(pt: &DataPoint, scheme: WeightingScheme) -> Result<f64, FitError> {
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
pub(in crate::solver) fn build_observations(
    datasets: &[DatasetSeries],
    scheme: WeightingScheme,
    honor_rep_moments: bool,
) -> Result<Vec<ObsRecord>, FitError> {
    let mut obs = Vec::new();
    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let honor_ds_moments = honor_rep_moments && (scheme == WeightingScheme::None || ds.is_pre_summarized);
        for pt in &ds.points {
            let (sqrt_weight, rep_n, ss_within, raw_ss_within) = if honor_ds_moments && pt.explicit_weight.is_none() {
                let w = compute_point_weight(pt, scheme)?;
                let (n_f, n_u, ss_i) = validated_replicate_moments(pt)?;
                let weighted_n = w * n_f;
                let weighted_ss_i = w * ss_i;
                if !weighted_n.is_finite() || weighted_n <= 0.0 || !weighted_ss_i.is_finite() {
                    return Err(FitError::ImpossibleWeights { reason: "Weighted replicate summary moments overflowed f64 range".to_owned() });
                }
                (weighted_n.sqrt(), n_u, weighted_ss_i, ss_i)
            } else {
                let w = compute_point_weight(pt, scheme)?;
                (w.sqrt(), 1, 0.0, 0.0)
            };
            obs.push(ObsRecord { point: pt.clone(), ds_local_idx, sqrt_weight, rep_n, ss_within, raw_ss_within });
        }
    }
    if obs.is_empty() {
        return Err(FitError::EmptyInput);
    }
    Ok(obs)
}

pub(in crate::solver) fn ode_budget_error() -> Option<FitError> {
    ode_fit_budget_exceeded().map(|max_steps| FitError::OdeIntegrationFailed {
        reason: format!("ODE integration step budget exceeded ({max_steps} adaptive RK45 steps per fit)"),
    })
}

pub(in crate::solver) fn obs_is_sweep_ordered(obs: &[ObsRecord], slot_ds: Option<usize>) -> bool {
    let mut prev: Option<(usize, f64)> = None;
    for rec in obs {
        if slot_ds.is_some_and(|d| d != rec.ds_local_idx) {
            continue;
        }
        let curr = (rec.ds_local_idx, rec.point.x0());
        if let Some(p) = prev
            && (curr.0 < p.0 || (curr.0 == p.0 && curr.1.total_cmp(&p.1).is_lt()))
        {
            return false;
        }
        prev = Some(curr);
    }
    true
}
