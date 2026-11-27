//! Free-parameter layout (global sharing, constraints, bounds, relational reparameterization),
//! observation records, and point weighting.
//!
//! References:
//! - Motulsky, H. & Christopoulos, A. (2004). *Fitting Models to Biological Data Using Linear and Nonlinear Regression*. Oxford University Press.
//! - Seber, G. A. F. & Wild, C. J. (1989). *Nonlinear Regression*. Wiley.
//! - Nocedal, J. & Wright, S. J. (2006). *Numerical Optimization*, 2nd ed. Springer.

use std::collections::{BTreeMap, VecDeque};

use curvebook_doc::{
    FitMethod, ParameterRule, ParameterScope, RelationalConstraint, RelationalConstraintOp, WeightingScheme, WeightingSource,
    parse_column_title_numeric_hook,
};
use curvebook_math::linalg::DenseMatrix;
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

/// Normalized directed relational inequality `theta[high] - theta[low] >= min_gap`.
#[derive(Debug, Clone, Copy)]
pub(in crate::solver) struct RelationalEdge {
    pub(in crate::solver) high: usize,
    pub(in crate::solver) low: usize,
    pub(in crate::solver) min_gap: f64,
    pub(in crate::solver) strict: bool,
}

#[derive(Debug, Clone)]
pub(in crate::solver) struct ParameterLayout {
    pub(in crate::solver) p_model: usize,
    pub(in crate::solver) n_datasets: usize,
    pub(in crate::solver) bindings: Vec<Vec<ParamBinding>>,
    pub(in crate::solver) free_slots: Vec<FreeSlot>,
    pub(in crate::solver) fixed_constants: Vec<(String, f64, Vec<ParameterNote>)>,
    pub(in crate::solver) rel_edges: Vec<RelationalEdge>,
    pub(in crate::solver) rel_parents: Vec<Vec<(usize, f64)>>,
    pub(in crate::solver) topo_order: Vec<usize>,
    pub(in crate::solver) has_relational: bool,
}

impl ParameterLayout {
    #[cfg(test)]
    pub(in crate::solver) fn build(
        model: &dyn ParametricModel,
        datasets: &[DatasetSeries],
        hooks: &HookContext,
        rules: &[ParameterRule],
    ) -> Result<Self, FitError> {
        Self::build_with_options(model, datasets, hooks, rules, &[], None)
    }

    pub(in crate::solver) fn build_with_constraints(
        model: &dyn ParametricModel,
        datasets: &[DatasetSeries],
        hooks: &HookContext,
        rules: &[ParameterRule],
        relational_constraints: &[RelationalConstraint],
    ) -> Result<Self, FitError> {
        Self::build_with_options(model, datasets, hooks, rules, relational_constraints, None)
    }

    pub(in crate::solver) fn build_with_options(
        model: &dyn ParametricModel,
        datasets: &[DatasetSeries],
        hooks: &HookContext,
        rules: &[ParameterRule],
        relational_constraints: &[RelationalConstraint],
        override_column_titles: Option<&[f64]>,
    ) -> Result<Self, FitError> {
        let param_names = model.param_names();
        validate_rules_and_mandatory_constants(model, param_names, rules)?;

        let p_model = param_names.len();
        let n_datasets = datasets.len();
        let all_points: Vec<DataPoint> = datasets.iter().flat_map(|d| d.points.iter().cloned()).collect();
        let extracted_titles: Vec<f64> = if override_column_titles.is_some() { Vec::new() } else { collect_column_title_values(datasets) };
        let col_titles = override_column_titles.unwrap_or(&extracted_titles);
        let default_init = model.initial_guess_with_column_titles(&all_points, col_titles);
        let default_bounds = model.default_bounds();
        let default_rules = model.default_parameter_rules(&all_points);

        let per_ds_init: Vec<Vec<f64>> = datasets.iter().map(|d| model.initial_guess_with_column_titles(&d.points, col_titles)).collect();

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
                    let init_ctx = ParamBindCtx { model, datasets, p_idx, pname, rule, lb, ub, equal_bound, per_ds_init: &per_ds_init };
                    bind_per_dataset_constants(&init_ctx, pairs, &mut bindings, &mut free_slots, &mut fixed_constants)?;
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
                    let any_used = datasets
                        .iter()
                        .enumerate()
                        .any(|(ds_idx, ds)| model.param_used_in_dataset(p_idx, model_dataset_index(model, ds_idx, ds.data_set_index)));
                    if !any_used && n_datasets > 0 {
                        let init_v = rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(p_idx).copied().unwrap_or(1.0));
                        let clamped = clamp_to_bounds(init_v, lb, ub);
                        for ds_row in &mut bindings {
                            ds_row[p_idx] = ParamBinding::Fixed(clamped);
                        }
                    } else if let Some(eq_val) = equal_bound {
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
                    let init_ctx = ParamBindCtx { model, datasets, p_idx, pname, rule, lb, ub, equal_bound, per_ds_init: &per_ds_init };
                    bind_local_parameter(&init_ctx, &default_init, &mut bindings, &mut free_slots, &mut fixed_constants);
                }
            }
        }

        // Merge model default relational constraints (e.g. DSL `[Constraints]`) with caller-supplied constraints.
        let mut merged_constraints = model.default_relational_constraints();
        merged_constraints.extend(relational_constraints.iter().cloned());
        let raw_edges = collect_relational_edges(model, datasets, param_names, &bindings, &mut free_slots, &merged_constraints)?;

        let (rel_edges, rel_parents, topo_order) = compile_relational_dag(&mut free_slots, raw_edges)?;
        let has_relational = !rel_edges.is_empty();

        Ok(Self { p_model, n_datasets, bindings, free_slots, fixed_constants, rel_edges, rel_parents, topo_order, has_relational })
    }

    pub(in crate::solver) fn k_free(&self) -> usize {
        self.free_slots.len()
    }

    pub(in crate::solver) fn bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        self.free_slots.iter().map(|s| (s.lower, s.upper)).collect()
    }

    /// Optimizer-space box bounds (`u`-space). Equal to [`Self::bounds`] when no relational
    /// constraints are present; otherwise child slots carry their gap lower bound `min_gap`.
    pub(in crate::solver) fn u_bounds(&self) -> Vec<(Option<f64>, Option<f64>)> {
        if !self.has_relational {
            return self.bounds();
        }
        let mut out = Vec::with_capacity(self.free_slots.len());
        for (j, slot) in self.free_slots.iter().enumerate() {
            let parents = &self.rel_parents[j];
            if parents.is_empty() {
                out.push((slot.lower, slot.upper));
            } else if parents.len() == 1 {
                let (p, min_gap) = parents[0];
                let ub = match (slot.upper, self.free_slots[p].lower) {
                    (Some(u_j), Some(l_p)) => Some((u_j - l_p).max(min_gap)),
                    _ => None,
                };
                out.push((Some(min_gap), ub));
            } else {
                out.push((Some(0.0), None));
            }
        }
        out
    }

    /// Feasible initial vector in optimizer `u`-space respecting both box bounds and relational
    /// inequality constraints.
    pub(in crate::solver) fn u_init(&self) -> Vec<f64> {
        let mut theta: Vec<f64> = self.free_slots.iter().map(|s| clamp_to_bounds(s.init, s.lower, s.upper)).collect();
        if !self.has_relational {
            return theta;
        }
        for &j in &self.topo_order {
            let parents = &self.rel_parents[j];
            if !parents.is_empty() {
                let mut req_min = f64::NEG_INFINITY;
                for &(p, eps) in parents {
                    let cand = theta[p] + eps;
                    if cand > req_min {
                        req_min = cand;
                    }
                }
                if theta[j] < req_min {
                    let bump = if req_min.abs() > 1e-6 { 0.05 * req_min.abs() } else { 1e-2 };
                    let target = req_min + bump;
                    theta[j] = match self.free_slots[j].upper {
                        Some(u) => target.min(u).max(req_min),
                        None => target,
                    };
                }
                theta[j] = clamp_to_bounds(theta[j], self.free_slots[j].lower, self.free_slots[j].upper);
            }
        }
        let mut u = self.theta_to_u(&theta);
        project_theta(&mut u, &self.u_bounds());
        u
    }

    /// Map optimizer `u`-space vector to original parameter vector `theta_orig`.
    pub(in crate::solver) fn u_to_theta_into(&self, u: &[f64], theta_out: &mut [f64]) {
        if !self.has_relational {
            theta_out.copy_from_slice(u);
            return;
        }
        for &j in &self.topo_order {
            let parents = &self.rel_parents[j];
            theta_out[j] = if parents.is_empty() {
                u[j]
            } else if parents.len() == 1 {
                theta_out[parents[0].0] + u[j]
            } else {
                let base = parents.iter().map(|&(p, eps)| theta_out[p] + eps).fold(f64::NEG_INFINITY, f64::max);
                base + u[j]
            };
        }
    }

    /// Map optimizer `u`-space vector to original parameter vector `theta_orig`.
    pub(in crate::solver) fn u_to_theta(&self, u: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0_f64; u.len()];
        self.u_to_theta_into(u, &mut out);
        out
    }

    /// Map feasible original parameter vector `theta_orig` into optimizer `u`-space.
    pub(in crate::solver) fn theta_to_u(&self, theta: &[f64]) -> Vec<f64> {
        if !self.has_relational {
            return theta.to_vec();
        }
        let mut u = vec![0.0_f64; theta.len()];
        for &j in &self.topo_order {
            let parents = &self.rel_parents[j];
            u[j] = if parents.is_empty() {
                theta[j]
            } else if parents.len() == 1 {
                theta[j] - theta[parents[0].0]
            } else {
                let base = parents.iter().map(|&(p, eps)| theta[p] + eps).fold(f64::NEG_INFINITY, f64::max);
                (theta[j] - base).max(0.0)
            };
        }
        u
    }

    /// Verify original-space box bounds for a decoded `theta_orig` vector.
    pub(in crate::solver) fn check_orig_bounds(&self, theta: &[f64]) -> bool {
        for (slot, &val) in self.free_slots.iter().zip(theta.iter()) {
            if !val.is_finite() {
                return false;
            }
            if let Some(l) = slot.lower
                && val < l - 1e-12 * l.abs().max(1.0)
            {
                return false;
            }
            if let Some(u) = slot.upper
                && val > u + 1e-12 * u.abs().max(1.0)
            {
                return false;
            }
        }
        true
    }

    /// Reverse-mode chain rule `g_u = T^T g_orig` mapping original-space parameter gradients to
    /// optimizer `u`-space gradients.
    pub(in crate::solver) fn apply_chain_rule_grad(&self, g_orig: &[f64], g_u: &mut [f64], theta_orig: &[f64]) {
        g_u.copy_from_slice(g_orig);
        if !self.has_relational {
            return;
        }
        for &j in self.topo_order.iter().rev() {
            let parents = &self.rel_parents[j];
            if parents.is_empty() {
                continue;
            }
            let gj = g_u[j];
            if parents.len() == 1 {
                g_u[parents[0].0] += gj;
            } else {
                let mut best_p = parents[0].0;
                let mut best_val = f64::NEG_INFINITY;
                for &(p, eps) in parents {
                    let cand = theta_orig[p] + eps;
                    if cand > best_val {
                        best_val = cand;
                        best_p = p;
                    }
                }
                g_u[best_p] += gj;
            }
        }
    }

    /// Exact inverse chain-rule Jacobian transform `J_orig = J_u * T^{-1}` back to original
    /// parameter coordinates so asymptotic covariance `s^2 (J_orig^T J_orig)^{-1}` equals
    /// `T * Cov_u * T^T` without matrix inversion.
    pub(in crate::solver) fn jacobian_u_to_orig(&self, j_u: &DenseMatrix, theta_orig: &[f64]) -> DenseMatrix {
        if !self.has_relational {
            return j_u.clone();
        }
        let n = j_u.nrows();
        let k = j_u.ncols();
        let mut j_orig = j_u.clone();
        for &j in &self.topo_order {
            if j >= k {
                continue;
            }
            let parents = &self.rel_parents[j];
            if parents.is_empty() {
                continue;
            }
            let parent = if parents.len() == 1 {
                parents[0].0
            } else {
                let mut best_p = parents[0].0;
                let mut best_val = f64::NEG_INFINITY;
                for &(p, eps) in parents {
                    let cand = theta_orig[p] + eps;
                    if cand > best_val {
                        best_val = cand;
                        best_p = p;
                    }
                }
                best_p
            };
            if parent < k {
                for r in 0..n {
                    let v_j = j_u.get(r, j).unwrap_or(0.0);
                    if let Some(cell) = j_orig.get_mut(r, parent) {
                        *cell -= v_j;
                    }
                }
            }
        }
        j_orig
    }

    /// Flag slots whose relational lower-bound gap `u[j] >= min_gap` is active at `u`.
    pub(in crate::solver) fn relational_bound_hits(&self, u: &[f64]) -> Vec<bool> {
        let mut hits = vec![false; self.free_slots.len()];
        if !self.has_relational {
            return hits;
        }
        let ub = self.u_bounds();
        for j in 0..self.free_slots.len() {
            if !self.rel_parents[j].is_empty()
                && let Some(lb) = ub[j].0
                && (u[j] - lb).abs() <= 1e-9 * lb.abs().max(1.0)
            {
                hits[j] = true;
            }
        }
        hits
    }

    /// Build a reduced [`ParameterLayout`] holding `theta_orig[fixed_idx] = fixed_val` (and every
    /// `u in extra_fixed` at `orig_theta[u]`) fixed, turning adjacent relational edges into box
    /// bounds while keeping remaining free-to-free relational edges active.
    pub(in crate::solver) fn fix_slots_for_profile(
        &self,
        fixed_idx: usize,
        fixed_val: f64,
        extra_fixed: &[usize],
        orig_theta: &[f64],
    ) -> Option<(Self, Vec<usize>)> {
        let k = self.free_slots.len();
        if fixed_idx >= k || !fixed_val.is_finite() || orig_theta.len() < k {
            return None;
        }
        let target_slot = &self.free_slots[fixed_idx];
        if target_slot.lower.is_some_and(|l| fixed_val < l) || target_slot.upper.is_some_and(|u| fixed_val > u) {
            return None;
        }

        let mut fixed_val_by_slot: Vec<Option<f64>> = vec![None; k];
        fixed_val_by_slot[fixed_idx] = Some(fixed_val);
        for &u in extra_fixed {
            if u < k && u != fixed_idx {
                let v = orig_theta[u];
                if !v.is_finite() {
                    return None;
                }
                fixed_val_by_slot[u] = Some(v);
            }
        }

        let mut old_to_new: Vec<Option<usize>> = vec![None; k];
        let mut new_to_old: Vec<usize> = Vec::with_capacity(k);
        let mut sub_slots: Vec<FreeSlot> = Vec::with_capacity(k);
        for (old_idx, slot) in self.free_slots.iter().enumerate() {
            if fixed_val_by_slot[old_idx].is_none() {
                let new_idx = sub_slots.len();
                old_to_new[old_idx] = Some(new_idx);
                new_to_old.push(old_idx);
                let mut cloned = slot.clone();
                cloned.init = clamp_to_bounds(orig_theta[old_idx], cloned.lower, cloned.upper);
                sub_slots.push(cloned);
            }
        }

        let mut sub_bindings = vec![vec![ParamBinding::Fixed(0.0); self.p_model]; self.n_datasets];
        for d in 0..self.n_datasets {
            for p in 0..self.p_model {
                sub_bindings[d][p] = match self.bindings[d][p] {
                    ParamBinding::Fixed(v) => ParamBinding::Fixed(v),
                    ParamBinding::Free(old_s) => {
                        if let Some(fv) = fixed_val_by_slot[old_s] {
                            ParamBinding::Fixed(fv)
                        } else {
                            ParamBinding::Free(old_to_new[old_s]?)
                        }
                    }
                };
            }
        }

        let mut sub_edges: Vec<RelationalEdge> = Vec::new();
        for edge in &self.rel_edges {
            match (fixed_val_by_slot[edge.high], fixed_val_by_slot[edge.low]) {
                (Some(v_hi), Some(v_lo)) => {
                    let ok = if edge.strict && edge.min_gap <= 1e-15 { v_hi > v_lo } else { v_hi >= v_lo + edge.min_gap };
                    if !ok {
                        return None;
                    }
                }
                (None, Some(v_lo)) => {
                    let nh = old_to_new[edge.high]?;
                    let min_hi = v_lo + edge.min_gap;
                    let slot = &mut sub_slots[nh];
                    slot.lower = Some(slot.lower.map_or(min_hi, |l| l.max(min_hi)));
                    if let (Some(l), Some(u)) = (slot.lower, slot.upper)
                        && l > u
                    {
                        return None;
                    }
                    slot.init = clamp_to_bounds(slot.init, slot.lower, slot.upper);
                }
                (Some(v_hi), None) => {
                    let nl = old_to_new[edge.low]?;
                    let max_lo = v_hi - edge.min_gap;
                    let slot = &mut sub_slots[nl];
                    slot.upper = Some(slot.upper.map_or(max_lo, |u| u.min(max_lo)));
                    if let (Some(l), Some(u)) = (slot.lower, slot.upper)
                        && l > u
                    {
                        return None;
                    }
                    slot.init = clamp_to_bounds(slot.init, slot.lower, slot.upper);
                }
                (None, None) => {
                    let nh = old_to_new[edge.high]?;
                    let nl = old_to_new[edge.low]?;
                    sub_edges.push(RelationalEdge { high: nh, low: nl, min_gap: edge.min_gap, strict: edge.strict });
                }
            }
        }

        let (rel_edges, rel_parents, topo_order) = compile_relational_dag(&mut sub_slots, sub_edges).ok()?;
        let has_relational = !rel_edges.is_empty();
        let sub_layout = Self {
            p_model: self.p_model,
            n_datasets: self.n_datasets,
            bindings: sub_bindings,
            free_slots: sub_slots,
            fixed_constants: Vec::new(),
            rel_edges,
            rel_parents,
            topo_order,
            has_relational,
        };
        Some((sub_layout, new_to_old))
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

    /// Decode optimizer `u`-space coordinates into original parameter space, validate original
    /// box bounds when relational constraints are active, and fill `flat_out`.
    pub(in crate::solver) fn fill_all_params_u(&self, u: &[f64], flat_out: &mut [f64]) -> bool {
        if !self.has_relational {
            self.fill_all_params(u, flat_out);
            return true;
        }
        let theta = self.u_to_theta(u);
        if !self.check_orig_bounds(&theta) {
            return false;
        }
        self.fill_all_params(&theta, flat_out);
        true
    }
}

type CompiledRelationalDag = (Vec<RelationalEdge>, Vec<Vec<(usize, f64)>>, Vec<usize>);
type FixedConstantEntry = (String, f64, Vec<ParameterNote>);

struct ParamBindCtx<'a> {
    model: &'a dyn ParametricModel,
    datasets: &'a [DatasetSeries],
    p_idx: usize,
    pname: &'a str,
    rule: Option<&'a ParameterRule>,
    lb: Option<f64>,
    ub: Option<f64>,
    equal_bound: Option<f64>,
    per_ds_init: &'a [Vec<f64>],
}

fn validate_rules_and_mandatory_constants(model: &dyn ParametricModel, param_names: &[String], rules: &[ParameterRule]) -> Result<(), FitError> {
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

    // Validate mandatory constant parameters declared by the model (e.g. DSL `[Constraints]` prompts).
    for req_name in model.mandatory_constant_params() {
        let explicit = rules.iter().find(|r| r.name.eq_ignore_ascii_case(req_name));
        let is_bound_const = explicit.is_some_and(|r| {
            matches!(
                r.scope,
                ParameterScope::Constant(_)
                    | ParameterScope::HookedConstant(_)
                    | ParameterScope::PerDatasetConstant(_)
                    | ParameterScope::ColumnTitleConstant
            ) || matches!((r.lower_bound, r.upper_bound), (Some(l), Some(u)) if l == u)
        });
        if !is_bound_const {
            return Err(FitError::InvalidParameter {
                name: "parameter_rules",
                reason: format!("Parameter `{req_name}` is a mandatory constant and must be fixed to a numeric or hooked value before fitting"),
            });
        }
    }
    Ok(())
}

fn bind_per_dataset_constants(
    ctx: &ParamBindCtx<'_>,
    pairs: &[(usize, f64)],
    bindings: &mut [Vec<ParamBinding>],
    free_slots: &mut Vec<FreeSlot>,
    fixed_constants: &mut Vec<FixedConstantEntry>,
) -> Result<(), FitError> {
    let n_datasets = ctx.datasets.len();
    for &(_, val) in pairs {
        if !val.is_finite() {
            return Err(FitError::InvalidParameter {
                name: "parameter_rules",
                reason: format!("Non-finite per-dataset constant for `{}`", ctx.pname),
            });
        }
    }
    for (ds_idx, ds) in ctx.datasets.iter().enumerate() {
        let eval_ds = model_dataset_index(ctx.model, ds_idx, ds.data_set_index);
        if let Some(&(_, val)) = pairs.iter().rev().find(|&&(idx, _)| idx == ds.data_set_index) {
            bindings[ds_idx][ctx.p_idx] = ParamBinding::Fixed(val);
            let label = if n_datasets > 1 { per_dataset_parameter_key(ctx.pname, ds.data_set_index) } else { ctx.pname.to_owned() };
            fixed_constants.push((label, val, vec![ParameterNote::Constant]));
        } else if !ctx.model.param_used_in_dataset(ctx.p_idx, eval_ds) {
            let init_v = ctx.rule.and_then(|r| r.initial_override).unwrap_or_else(|| ctx.per_ds_init[ds_idx].get(ctx.p_idx).copied().unwrap_or(1.0));
            bindings[ds_idx][ctx.p_idx] = ParamBinding::Fixed(clamp_to_bounds(init_v, ctx.lb, ctx.ub));
        } else if let Some(eq_val) = ctx.equal_bound {
            bindings[ds_idx][ctx.p_idx] = ParamBinding::Fixed(eq_val);
            let label = if n_datasets > 1 { per_dataset_parameter_key(ctx.pname, ds.data_set_index) } else { ctx.pname.to_owned() };
            fixed_constants.push((label, eq_val, vec![ParameterNote::Constant]));
        } else {
            let slot = free_slots.len();
            let init_v = ctx.rule.and_then(|r| r.initial_override).unwrap_or_else(|| ctx.per_ds_init[ds_idx].get(ctx.p_idx).copied().unwrap_or(1.0));
            free_slots.push(FreeSlot {
                ds_local_idx: Some(ds_idx),
                dataset_idx: Some(ds.data_set_index),
                name: ctx.pname.to_owned(),
                lower: ctx.lb,
                upper: ctx.ub,
                init: clamp_to_bounds(init_v, ctx.lb, ctx.ub),
                is_shared: false,
            });
            bindings[ds_idx][ctx.p_idx] = ParamBinding::Free(slot);
        }
    }
    Ok(())
}

fn bind_local_parameter(
    ctx: &ParamBindCtx<'_>,
    default_init: &[f64],
    bindings: &mut [Vec<ParamBinding>],
    free_slots: &mut Vec<FreeSlot>,
    fixed_constants: &mut Vec<FixedConstantEntry>,
) {
    let n_datasets = ctx.datasets.len();
    if let Some(eq_val) = ctx.equal_bound {
        for ds_row in bindings.iter_mut() {
            ds_row[ctx.p_idx] = ParamBinding::Fixed(eq_val);
        }
        fixed_constants.push((ctx.pname.to_owned(), eq_val, vec![ParameterNote::Constant]));
    } else if n_datasets <= 1 {
        let eval_ds = ctx.datasets.first().map_or(0, |d| model_dataset_index(ctx.model, 0, d.data_set_index));
        let init_v = ctx.rule.and_then(|r| r.initial_override).unwrap_or_else(|| default_init.get(ctx.p_idx).copied().unwrap_or(1.0));
        if n_datasets == 1 && !ctx.model.param_used_in_dataset(ctx.p_idx, eval_ds) {
            if let Some(ds_row) = bindings.first_mut() {
                ds_row[ctx.p_idx] = ParamBinding::Fixed(clamp_to_bounds(init_v, ctx.lb, ctx.ub));
            }
        } else {
            let slot = free_slots.len();
            let ds_ext_idx = ctx.datasets.first().map_or(0, |d| d.data_set_index);
            free_slots.push(FreeSlot {
                ds_local_idx: Some(0),
                dataset_idx: Some(ds_ext_idx),
                name: ctx.pname.to_owned(),
                lower: ctx.lb,
                upper: ctx.ub,
                init: clamp_to_bounds(init_v, ctx.lb, ctx.ub),
                is_shared: false,
            });
            if let Some(ds_row) = bindings.first_mut() {
                ds_row[ctx.p_idx] = ParamBinding::Free(slot);
            }
        }
    } else {
        for (ds_idx, ds) in ctx.datasets.iter().enumerate() {
            let eval_ds = model_dataset_index(ctx.model, ds_idx, ds.data_set_index);
            let init_v = ctx.rule.and_then(|r| r.initial_override).unwrap_or_else(|| ctx.per_ds_init[ds_idx].get(ctx.p_idx).copied().unwrap_or(1.0));
            if !ctx.model.param_used_in_dataset(ctx.p_idx, eval_ds) {
                bindings[ds_idx][ctx.p_idx] = ParamBinding::Fixed(clamp_to_bounds(init_v, ctx.lb, ctx.ub));
                continue;
            }
            let slot = free_slots.len();
            free_slots.push(FreeSlot {
                ds_local_idx: Some(ds_idx),
                dataset_idx: Some(ds.data_set_index),
                name: ctx.pname.to_owned(),
                lower: ctx.lb,
                upper: ctx.ub,
                init: clamp_to_bounds(init_v, ctx.lb, ctx.ub),
                is_shared: false,
            });
            bindings[ds_idx][ctx.p_idx] = ParamBinding::Free(slot);
        }
    }
}

fn collect_relational_edges(
    model: &dyn ParametricModel,
    datasets: &[DatasetSeries],
    param_names: &[String],
    bindings: &[Vec<ParamBinding>],
    free_slots: &mut [FreeSlot],
    merged_constraints: &[RelationalConstraint],
) -> Result<Vec<RelationalEdge>, FitError> {
    let mut raw_edges: Vec<RelationalEdge> = Vec::new();
    for rc in merged_constraints {
        if !rc.margin.is_finite() || rc.margin < 0.0 {
            return Err(FitError::InvalidParameter {
                name: "relational_constraints",
                reason: format!("Relational constraint margin must be finite and >= 0 (got {})", rc.margin),
            });
        }
        let Some(p_lhs) = param_names.iter().position(|p| p.eq_ignore_ascii_case(&rc.lhs_param)) else {
            return Err(FitError::InvalidParameter {
                name: "relational_constraints",
                reason: format!("Unknown parameter `{}` in relational constraint", rc.lhs_param),
            });
        };
        let Some(p_rhs) = param_names.iter().position(|p| p.eq_ignore_ascii_case(&rc.rhs_param)) else {
            return Err(FitError::InvalidParameter {
                name: "relational_constraints",
                reason: format!("Unknown parameter `{}` in relational constraint", rc.rhs_param),
            });
        };
        if p_lhs == p_rhs {
            return Err(FitError::InvalidParameter {
                name: "relational_constraints",
                reason: format!("Relational constraint cannot relate `{}` to itself", rc.lhs_param),
            });
        }
        let (p_hi, p_lo, strict) = match rc.op {
            RelationalConstraintOp::GreaterThan => (p_lhs, p_rhs, true),
            RelationalConstraintOp::GreaterOrEqual => (p_lhs, p_rhs, false),
            RelationalConstraintOp::LessThan => (p_rhs, p_lhs, true),
            RelationalConstraintOp::LessOrEqual => (p_rhs, p_lhs, false),
        };
        let eps = if strict && rc.margin == 0.0 { 1e-15_f64 } else { rc.margin };

        for (ds_idx, ds) in datasets.iter().enumerate() {
            let eval_ds = model_dataset_index(model, ds_idx, ds.data_set_index);
            if !model.param_used_in_dataset(p_hi, eval_ds) || !model.param_used_in_dataset(p_lo, eval_ds) {
                continue;
            }
            match (&bindings[ds_idx][p_hi], &bindings[ds_idx][p_lo]) {
                (ParamBinding::Fixed(v_hi), ParamBinding::Fixed(v_lo)) => {
                    let ok = if strict { *v_hi > *v_lo + rc.margin } else { *v_hi >= *v_lo + rc.margin };
                    if !ok {
                        return Err(FitError::InvalidParameter {
                            name: "relational_constraints",
                            reason: format!(
                                "Fixed constants violate relational constraint `{}` ({v_hi}) vs `{}` ({v_lo}) with margin {}",
                                param_names[p_hi], param_names[p_lo], rc.margin
                            ),
                        });
                    }
                }
                (ParamBinding::Free(s_hi), ParamBinding::Fixed(v_lo)) => {
                    let min_hi = *v_lo + eps;
                    let slot = &mut free_slots[*s_hi];
                    slot.lower = Some(slot.lower.map_or(min_hi, |l| l.max(min_hi)));
                    if let (Some(l), Some(u)) = (slot.lower, slot.upper)
                        && l > u
                    {
                        return Err(FitError::InvalidParameter {
                            name: "relational_constraints",
                            reason: format!("Relational constraint makes bounds infeasible for `{}` ([{l}, {u}])", slot.name),
                        });
                    }
                    slot.init = clamp_to_bounds(slot.init, slot.lower, slot.upper);
                }
                (ParamBinding::Fixed(v_hi), ParamBinding::Free(s_lo)) => {
                    let max_lo = *v_hi - eps;
                    let slot = &mut free_slots[*s_lo];
                    slot.upper = Some(slot.upper.map_or(max_lo, |u| u.min(max_lo)));
                    if let (Some(l), Some(u)) = (slot.lower, slot.upper)
                        && l > u
                    {
                        return Err(FitError::InvalidParameter {
                            name: "relational_constraints",
                            reason: format!("Relational constraint makes bounds infeasible for `{}` ([{l}, {u}])", slot.name),
                        });
                    }
                    slot.init = clamp_to_bounds(slot.init, slot.lower, slot.upper);
                }
                (ParamBinding::Free(s_hi), ParamBinding::Free(s_lo)) => {
                    if s_hi == s_lo {
                        return Err(FitError::InvalidParameter {
                            name: "relational_constraints",
                            reason: "Relational constraint cannot link a parameter slot to itself".to_owned(),
                        });
                    }
                    raw_edges.push(RelationalEdge { high: *s_hi, low: *s_lo, min_gap: eps, strict });
                }
            }
        }
    }
    Ok(raw_edges)
}

fn compile_relational_dag(free_slots: &mut [FreeSlot], raw_edges: Vec<RelationalEdge>) -> Result<CompiledRelationalDag, FitError> {
    let k = free_slots.len();
    if raw_edges.is_empty() {
        return Ok((Vec::new(), vec![Vec::new(); k], (0..k).collect()));
    }
    // Deduplicate (low, high) pairs, retaining the tightest min_gap.
    let mut edge_map: BTreeMap<(usize, usize), (f64, bool)> = BTreeMap::new();
    for e in raw_edges {
        edge_map
            .entry((e.low, e.high))
            .and_modify(|(gap, strict)| {
                if e.min_gap > *gap {
                    *gap = e.min_gap;
                }
                *strict |= e.strict;
            })
            .or_insert((e.min_gap, e.strict));
    }

    let mut edges = Vec::with_capacity(edge_map.len());
    let mut parents: Vec<Vec<(usize, f64)>> = vec![Vec::new(); k];
    let mut children: Vec<Vec<(usize, f64)>> = vec![Vec::new(); k];
    let mut indeg = vec![0_usize; k];

    for ((low, high), (min_gap, strict)) in edge_map {
        edges.push(RelationalEdge { high, low, min_gap, strict });
        parents[high].push((low, min_gap));
        children[low].push((high, min_gap));
        indeg[high] += 1;
    }

    let mut queue: VecDeque<usize> = (0..k).filter(|&i| indeg[i] == 0).collect();
    let mut topo_order = Vec::with_capacity(k);
    while let Some(u) = queue.pop_front() {
        topo_order.push(u);
        for &(v, _) in &children[u] {
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    if topo_order.len() != k {
        return Err(FitError::InvalidParameter {
            name: "relational_constraints",
            reason: "Cyclic or contradictory relational inequality constraints detected".to_owned(),
        });
    }

    // Propagate lower bounds forward and upper bounds backward to reject infeasible bound combinations early.
    for &j in &topo_order {
        for &(p, eps) in &parents[j] {
            if let Some(lp) = free_slots[p].lower {
                let req = lp + eps;
                free_slots[j].lower = Some(free_slots[j].lower.map_or(req, |l| l.max(req)));
            }
        }
        if let (Some(l), Some(u)) = (free_slots[j].lower, free_slots[j].upper)
            && l > u
        {
            return Err(FitError::InvalidParameter {
                name: "relational_constraints",
                reason: format!("Relational constraint makes bounds infeasible for `{}` ([{l}, {u}])", free_slots[j].name),
            });
        }
    }
    for &j in topo_order.iter().rev() {
        if let Some(uj) = free_slots[j].upper {
            for &(p, eps) in &parents[j] {
                let req = uj - eps;
                free_slots[p].upper = Some(free_slots[p].upper.map_or(req, |u| u.min(req)));
                if let (Some(l), Some(u)) = (free_slots[p].lower, free_slots[p].upper)
                    && l > u
                {
                    return Err(FitError::InvalidParameter {
                        name: "relational_constraints",
                        reason: format!("Relational constraint makes bounds infeasible for `{}` ([{l}, {u}])", free_slots[p].name),
                    });
                }
            }
        }
    }

    Ok((edges, parents, topo_order))
}

#[inline]
pub(in crate::solver) fn model_dataset_index(model: &dyn ParametricModel, ds_local_idx: usize, data_set_index: usize) -> usize {
    if model.uses_sheet_dataset_index() { data_set_index } else { ds_local_idx }
}

pub(in crate::solver) fn collect_column_title_values(datasets: &[DatasetSeries]) -> Vec<f64> {
    datasets
        .iter()
        .filter_map(|d| d.column_title_value.or_else(|| parse_column_title_numeric_hook(&d.label).ok()))
        .filter(|v| v.is_finite())
        .collect()
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
            if !k.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: format!("General power Y exponent must be finite, got {k}") });
            }
            if !pt.y.is_finite() || (pt.y == 0.0 && k > 0.0) {
                return Err(FitError::ImpossibleWeights { reason: format!("1/|Y|^{k} weighting is undefined where Y == 0 or non-finite") });
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

/// Observation weight `w_i = 1 / |y_hat_i|^k` evaluated from fitted curve height `y_hat_i`
/// (Carroll & Ruppert 1988; Motulsky & Christopoulos 2004).
///
/// Returns `Err(FitError::ImpossibleWeights)` if `y_hat` is zero (for positive powers) or
/// non-finite, or if the resulting weight overflows.
pub(in crate::solver) fn compute_predicted_y_weight(y_hat: f64, scheme: WeightingScheme, explicit_weight: Option<f64>) -> Result<f64, FitError> {
    if let Some(w_exp) = explicit_weight {
        if !w_exp.is_finite() || w_exp <= 0.0 {
            return Err(FitError::ImpossibleWeights { reason: "Explicit observation weight must be finite and strictly positive".to_owned() });
        }
        return Ok(w_exp);
    }
    let w = match scheme {
        WeightingScheme::RelativeSquare => {
            if !y_hat.is_finite() || y_hat == 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "1/Y_curve^2 weighting is undefined where predicted Y_hat == 0".to_owned() });
            }
            1.0 / (y_hat * y_hat)
        }
        WeightingScheme::Relative => {
            if !y_hat.is_finite() || y_hat == 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "1/|Y_curve| weighting is undefined where predicted Y_hat == 0".to_owned() });
            }
            1.0 / y_hat.abs()
        }
        WeightingScheme::GeneralPowerY(k) => {
            if !k.is_finite() {
                return Err(FitError::ImpossibleWeights { reason: format!("General power Y exponent must be finite, got {k}") });
            }
            if !y_hat.is_finite() || (y_hat == 0.0 && k > 0.0) {
                return Err(FitError::ImpossibleWeights { reason: format!("1/|Y_curve|^{k} weighting is undefined where predicted Y_hat == 0") });
            }
            let denom = y_hat.abs().powf(k);
            if !denom.is_finite() || denom <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "Curve-predicted power Y weight underflowed or overflowed".to_owned() });
            }
            1.0 / denom
        }
        _ => 1.0,
    };
    if w.is_finite() && w > 0.0 {
        Ok(w)
    } else {
        Err(FitError::ImpossibleWeights { reason: "Curve-predicted weight overflowed f64 range".to_owned() })
    }
}

/// Apply curve-predicted weights `w_i = 1 / |y_hat_i|^k` across `obs` atomically (two-phase:
/// validates every observation before committing any update), preserving `(Mean, SD, n)`
/// sufficient statistics (`sqrt_weight = sqrt(w_i * rep_n)` and `ss_within = w_i * raw_ss_within`).
///
/// Returns `false` if any predicted value yields an impossible weight (e.g., `y_hat == 0` or overflow),
/// leaving `obs` completely unmodified.
pub(in crate::solver) fn apply_predicted_y_weights(obs: &mut [ObsRecord], y_hat: &[f64], scheme: WeightingScheme) -> bool {
    if obs.len() != y_hat.len() {
        return false;
    }
    let mut staged = Vec::with_capacity(obs.len());
    for (rec, &yh) in obs.iter().zip(y_hat.iter()) {
        let Ok(w) = compute_predicted_y_weight(yh, scheme, rec.point.explicit_weight) else {
            return false;
        };
        let weighted_n = w * (rec.rep_n as f64);
        let weighted_ss_i = w * rec.raw_ss_within;
        if !weighted_n.is_finite() || weighted_n <= 0.0 || !weighted_ss_i.is_finite() {
            return false;
        }
        let sw = weighted_n.sqrt();
        if !sw.is_finite() || sw <= 0.0 {
            return false;
        }
        staged.push((sw, weighted_ss_i));
    }
    for (rec, (sw, ss_i)) in obs.iter_mut().zip(staged) {
        rec.sqrt_weight = sw;
        rec.ss_within = ss_i;
    }
    true
}

/// Reset `obs` to unweighted (`w_i = 1`, or explicit weight if set) while preserving replicate
/// summary sufficient statistics (`rep_n` and `raw_ss_within`).
pub(in crate::solver) fn reset_observations_unweighted(obs: &mut [ObsRecord]) -> Result<(), FitError> {
    for rec in obs.iter_mut() {
        let w = if let Some(w_exp) = rec.point.explicit_weight {
            if !w_exp.is_finite() || w_exp <= 0.0 {
                return Err(FitError::ImpossibleWeights { reason: "Explicit observation weight must be finite and strictly positive".to_owned() });
            }
            w_exp
        } else {
            1.0
        };
        rec.sqrt_weight = (w * (rec.rep_n as f64)).sqrt();
        rec.ss_within = w * rec.raw_ss_within;
    }
    Ok(())
}

/// Flatten datasets into active observations with verified square-root weights.
#[cfg(test)]
pub(in crate::solver) fn build_observations(
    datasets: &[DatasetSeries],
    scheme: WeightingScheme,
    honor_rep_moments: bool,
) -> Result<Vec<ObsRecord>, FitError> {
    build_observations_with_source(datasets, scheme, WeightingSource::ObservedY, honor_rep_moments)
}

/// Flatten datasets into active observations with verified square-root weights, honoring
/// [`WeightingSource::PredictedY`] (which starts unweighted and defers `1/|y_hat|^k` to IRLS).
pub(in crate::solver) fn build_observations_with_source(
    datasets: &[DatasetSeries],
    scheme: WeightingScheme,
    weighting_source: WeightingSource,
    honor_rep_moments: bool,
) -> Result<Vec<ObsRecord>, FitError> {
    let defer_predicted_y = matches!(weighting_source, WeightingSource::PredictedY)
        && matches!(scheme, WeightingScheme::RelativeSquare | WeightingScheme::Relative | WeightingScheme::GeneralPowerY(_));
    let effective_scheme = if defer_predicted_y { WeightingScheme::None } else { scheme };
    let mut obs = Vec::new();
    for (ds_local_idx, ds) in datasets.iter().enumerate() {
        let honor_ds_moments = honor_rep_moments && (effective_scheme == WeightingScheme::None || ds.is_pre_summarized);
        for pt in &ds.points {
            let (sqrt_weight, rep_n, ss_within, raw_ss_within) = if honor_ds_moments && pt.explicit_weight.is_none() {
                let w = compute_point_weight(pt, effective_scheme)?;
                let (n_f, n_u, ss_i) = validated_replicate_moments(pt)?;
                let weighted_n = w * n_f;
                let weighted_ss_i = w * ss_i;
                if !weighted_n.is_finite() || weighted_n <= 0.0 || !weighted_ss_i.is_finite() {
                    return Err(FitError::ImpossibleWeights { reason: "Weighted replicate summary moments overflowed f64 range".to_owned() });
                }
                (weighted_n.sqrt(), n_u, weighted_ss_i, ss_i)
            } else {
                let w = compute_point_weight(pt, effective_scheme)?;
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
