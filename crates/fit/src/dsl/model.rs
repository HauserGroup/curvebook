//! `CompiledDslModel`: compilation entry point and its `ParametricModel` implementation.
//!
//! References:
//! - Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae. *Journal of Computational and Applied Mathematics* 6(1), 19–26.
//! - Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::Ordering;

use curvebook_math::roots::{RootOptions, brent_root};

use crate::dsl::ast::{CompiledInitRule, CompiledOdeInit, CompiledProgram, DatasetFilter, DslCompileWarning, DslEquationKind, Expr, OpCode};
use crate::dsl::compile::compile_program_refs;
use crate::dsl::implicit::{implicit_newton_fallback, implicit_probes_and_scale};
use crate::dsl::limits::{MAX_DSL_LINES, MAX_DSL_SOURCE_BYTES, MAX_ODE_VM_OPS_PER_EVAL, MAX_TOTAL_DATASET_INDICES, MAX_TOTAL_OPCODES};
use crate::dsl::ode::{
    NEXT_ODE_MODEL_ID, OdeRhsSource, consume_ode_fit_rk_step, consume_ode_fit_vm_ops, eval_ode_on_side, ode_fit_budget_exceeded,
    store_cached_ode_trajectory, take_cached_ode_trajectory,
};
use crate::dsl::statements::{
    DslSection, ParsedStatement, ResolvedEquationBody, byte_offset_in_line, collect_free_idents, compile_initial_rules, compile_transforms,
    compute_dataset_stats, is_builtin_constant_or_var, parse_dataset_prefix, parse_section_header, parse_statement, parse_statement_with_locs,
    resolve_equation_branches, strip_comment,
};
use crate::dsl::vm::{eval_bytecode, eval_bytecode_counted, eval_bytecode_with_locals};
use crate::error::FitError;
use crate::library::ParametricModel;
use crate::types::{DataPoint, DerivedScalar, EquationCategory};

/// Compiled custom equation implementing [`ParametricModel`] safely inside a bounded VM.
#[derive(Debug, Clone)]
pub struct CompiledDslModel {
    pub(in crate::dsl) id: String,
    pub(in crate::dsl) model_id: u64,
    pub(in crate::dsl) name: String,
    pub(in crate::dsl) kind: DslEquationKind,
    pub(in crate::dsl) param_names: Vec<String>,
    pub(in crate::dsl) programs: Vec<(DatasetFilter, CompiledProgram)>,
    pub(in crate::dsl) program_by_ds: HashMap<usize, usize>,
    pub(in crate::dsl) default_program_idx: Option<usize>,
    pub(in crate::dsl) ode_inits: Vec<(DatasetFilter, CompiledOdeInit)>,
    pub(in crate::dsl) ode_init_by_ds: HashMap<usize, usize>,
    pub(in crate::dsl) default_ode_init_idx: Option<usize>,
    pub(in crate::dsl) initial_hints: HashMap<String, f64>,
    pub(in crate::dsl) initial_rules: Vec<CompiledInitRule>,
    pub(in crate::dsl) init_local_count: usize,
    pub(in crate::dsl) transforms: Vec<(String, usize, CompiledProgram)>,
    pub(in crate::dsl) transform_local_count: usize,
    pub(in crate::dsl) warnings: Vec<DslCompileWarning>,
}

impl CompiledDslModel {
    /// Compile a user equation string (explicit `Y=...`, conditional `IF`, `<A>` dataset lines,
    /// implicit `LHS=RHS`, or 1st-order ODE `dY/dX=...` / `D(Y, X)=...`).
    ///
    /// # Operator Precedence (highest to lowest)
    /// 1. Parentheses `( ... )`, literals, variables, and function calls `f(...)`
    /// 2. Exponentiation `^` (right-associative; binds tighter than prefix unary `+`/`-`/`!`,
    ///    so `-X^2` parses as `-(X^2)` and `-2^2 == -4`, while prefix unary operators are
    ///    permitted in the right-hand exponent as in `X^-2` = `X^(-2)`)
    /// 3. Prefix unary `+`, `-`, `!`
    /// 4. Multiplicative `*`, `/`, `%` (left-associative)
    /// 5. Additive `+`, `-` (left-associative)
    /// 6. Relational/equality `==`, `=`, `!=`, `<`, `<=`, `>`, `>=`
    /// 7. Logical `&&` (`AND`), then `||` (`OR`)
    pub fn compile(id: impl Into<String>, source: &str) -> Result<Self, FitError> {
        if source.len() > MAX_DSL_SOURCE_BYTES {
            return Err(FitError::DslCompileError {
                message: format!("Custom equation exceeds maximum source length ({MAX_DSL_SOURCE_BYTES} bytes)"),
                line: 1,
                column: 1,
            });
        }
        let id = id.into();
        let mut section = DslSection::Equation;
        let mut parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)> = Vec::new();
        let mut raw_init_lines: Vec<(usize, DatasetFilter, String, Expr)> = Vec::new();
        let mut raw_transform_lines: Vec<(usize, String, Expr)> = Vec::new();
        let mut initial_hints = HashMap::new();
        let mut ident_locs: HashMap<String, (usize, usize)> = HashMap::new();
        let mut total_dataset_indices = 0_usize;

        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = idx + 1;
            if line_no > MAX_DSL_LINES {
                return Err(FitError::DslCompileError {
                    message: format!("Custom equation exceeds maximum line limit ({MAX_DSL_LINES})"),
                    line: line_no,
                    column: 1,
                });
            }
            let trimmed = strip_comment(raw_line, line_no, &mut initial_hints)?;
            if trimmed.is_empty() {
                continue;
            }
            if let Some(next_section) = parse_section_header(trimmed, line_no)? {
                section = next_section;
                continue;
            }
            let (filter, rest) = parse_dataset_prefix(trimmed, line_no)?;
            let filter_len = match &filter {
                DatasetFilter::Any => 0,
                DatasetFilter::Include(v) | DatasetFilter::Exclude(v) => v.len(),
            };
            total_dataset_indices = total_dataset_indices.saturating_add(filter_len);
            if total_dataset_indices > MAX_TOTAL_DATASET_INDICES {
                return Err(FitError::DslCompileError {
                    message: format!("Total dataset selector indices exceed global limit ({MAX_TOTAL_DATASET_INDICES})"),
                    line: line_no,
                    column: 1,
                });
            }
            if rest.is_empty() {
                continue;
            }
            let col_offset = byte_offset_in_line(raw_line, rest);
            match section {
                DslSection::Equation => {
                    let stmt = parse_statement_with_locs(rest, line_no, col_offset, Some(&mut ident_locs))?;
                    parsed_lines.push((line_no, filter, stmt));
                }
                DslSection::InitialValues => match parse_statement(rest, line_no)? {
                    ParsedStatement::Assign { name, rhs } | ParsedStatement::OdeInitial { lhs_name: Some(name), rhs, .. } => {
                        if is_builtin_constant_or_var(&name) {
                            return Err(FitError::DslCompileError {
                                message: format!("Cannot assign initial value rule to reserved identifier `{name}`"),
                                line: line_no,
                                column: col_offset + 1,
                            });
                        }
                        raw_init_lines.push((line_no, filter, name, rhs));
                    }
                    _ => {
                        return Err(FitError::DslCompileError {
                            message: "Expected `PARAM = expr` assignment in [InitialValues]".to_owned(),
                            line: line_no,
                            column: 1,
                        });
                    }
                },
                DslSection::Transforms => {
                    if filter != DatasetFilter::Any {
                        return Err(FitError::DslCompileError {
                            message: "Dataset selectors are not supported in [Transforms]".to_owned(),
                            line: line_no,
                            column: 1,
                        });
                    }
                    match parse_statement(rest, line_no)? {
                        ParsedStatement::Assign { name, rhs } | ParsedStatement::OdeInitial { lhs_name: Some(name), rhs, .. } => {
                            raw_transform_lines.push((line_no, name, rhs));
                        }
                        _ => {
                            return Err(FitError::DslCompileError {
                                message: "Expected `SCALAR = expr` assignment in [Transforms]".to_owned(),
                                line: line_no,
                                column: 1,
                            });
                        }
                    }
                }
            }
        }

        if parsed_lines.is_empty() {
            return Err(FitError::DslCompileError { message: "Empty custom equation".to_owned(), line: 1, column: 1 });
        }

        let ResolvedEquationBody { kind, body_stmts, ode_init_branches, warnings } = resolve_equation_branches(parsed_lines, &ident_locs)?;
        if body_stmts.is_empty() {
            return Err(FitError::DslCompileError { message: "No target expression (`Y = ...` or expression) found".to_owned(), line: 1, column: 1 });
        }

        // Collect all free parameter identifiers in deterministic first-appearance order.
        let mut param_order: Vec<String> = Vec::new();
        let mut seen_params: BTreeSet<String> = BTreeSet::new();

        for (_, locals, target) in &body_stmts {
            let mut bound_locals: BTreeSet<String> = BTreeSet::new();
            for (lname, rhs) in locals {
                collect_free_idents(rhs, &bound_locals, &mut seen_params, &mut param_order);
                bound_locals.insert(lname.clone());
            }
            collect_free_idents(target, &bound_locals, &mut seen_params, &mut param_order);
        }
        for (_, _, locals, rhs) in &ode_init_branches {
            let mut bound_locals: BTreeSet<String> = BTreeSet::new();
            for (lname, lrhs) in locals {
                collect_free_idents(lrhs, &bound_locals, &mut seen_params, &mut param_order);
                bound_locals.insert(lname.clone());
            }
            collect_free_idents(rhs, &bound_locals, &mut seen_params, &mut param_order);
        }
        if kind == DslEquationKind::OdeFirstOrder && ode_init_branches.is_empty() && !seen_params.contains("Y0") {
            param_order.push("Y0".to_owned());
            seen_params.insert("Y0".to_owned());
        }

        let param_map: HashMap<String, usize> = param_order.iter().enumerate().map(|(i, p)| (p.clone(), i)).collect();
        let mut total_opcodes = 0_usize;

        let mut programs = Vec::with_capacity(body_stmts.len());
        let mut program_by_ds = HashMap::new();
        let mut default_program_idx = None;
        for (filter, locals, target) in body_stmts {
            let remaining = MAX_TOTAL_OPCODES.saturating_sub(total_opcodes);
            let prog = compile_program_refs(&locals, &target, &param_map, remaining)?;
            total_opcodes = total_opcodes.saturating_add(prog.code.len());
            if total_opcodes > MAX_TOTAL_OPCODES {
                return Err(FitError::DslCompileError {
                    message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                    line: 1,
                    column: 1,
                });
            }
            let prog_idx = programs.len();
            match &filter {
                DatasetFilter::Include(v) => {
                    for &ds in v {
                        program_by_ds.insert(ds, prog_idx);
                    }
                }
                DatasetFilter::Exclude(_) | DatasetFilter::Any => {
                    default_program_idx = Some(prog_idx);
                }
            }
            programs.push((filter, prog));
        }

        let mut ode_inits = Vec::new();
        let mut ode_init_by_ds = HashMap::new();
        let mut default_ode_init_idx = None;
        if kind == DslEquationKind::OdeFirstOrder {
            if ode_init_branches.is_empty() {
                let y0_idx = *param_map.get("Y0").unwrap_or(&0);
                let prog = CompiledProgram { code: vec![OpCode::LoadParam(y0_idx), OpCode::Halt], local_count: 0 };
                total_opcodes = total_opcodes.saturating_add(prog.code.len());
                if total_opcodes > MAX_TOTAL_OPCODES {
                    return Err(FitError::DslCompileError {
                        message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                        line: 1,
                        column: 1,
                    });
                }
                ode_inits.push((DatasetFilter::Any, CompiledOdeInit { t0: 0.0, prog }));
                default_ode_init_idx = Some(0);
            } else {
                for (filter, t0, locals, init_expr) in ode_init_branches {
                    let remaining = MAX_TOTAL_OPCODES.saturating_sub(total_opcodes);
                    let prog = compile_program_refs(&locals, &init_expr, &param_map, remaining)?;
                    total_opcodes = total_opcodes.saturating_add(prog.code.len());
                    if total_opcodes > MAX_TOTAL_OPCODES {
                        return Err(FitError::DslCompileError {
                            message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                            line: 1,
                            column: 1,
                        });
                    }
                    let init_idx = ode_inits.len();
                    match &filter {
                        DatasetFilter::Include(v) => {
                            for &ds in v {
                                ode_init_by_ds.insert(ds, init_idx);
                            }
                        }
                        DatasetFilter::Exclude(_) | DatasetFilter::Any => {
                            default_ode_init_idx = Some(init_idx);
                        }
                    }
                    ode_inits.push((filter, CompiledOdeInit { t0, prog }));
                }
            }
        }

        let (initial_rules, init_local_count) = compile_initial_rules(raw_init_lines, &param_order, &param_map, &mut total_opcodes)?;
        let (transforms, transform_local_count) = compile_transforms(raw_transform_lines, &param_map, &mut total_opcodes)?;

        Ok(Self {
            name: id.clone(),
            id,
            model_id: NEXT_ODE_MODEL_ID.fetch_add(1, Ordering::Relaxed),
            kind,
            param_names: param_order,
            programs,
            program_by_ds,
            default_program_idx,
            ode_inits,
            ode_init_by_ds,
            default_ode_init_idx,
            initial_hints,
            initial_rules,
            init_local_count,
            transforms,
            transform_local_count,
            warnings,
        })
    }

    /// Return the structural form (`Explicit`, `Implicit`, or `OdeFirstOrder`) of this compiled equation.
    #[must_use]
    pub fn kind(&self) -> DslEquationKind {
        self.kind
    }

    /// Return non-fatal compile warnings (such as reserved identifier usage) emitted during compilation.
    #[must_use]
    pub fn warnings(&self) -> &[DslCompileWarning] {
        &self.warnings
    }

    pub(in crate::dsl) fn select_program(&self, ds_idx: usize) -> Option<&CompiledProgram> {
        let idx = self.program_by_ds.get(&ds_idx).copied().or(self.default_program_idx)?;
        self.programs.get(idx).filter(|(f, _)| f.matches(ds_idx)).map(|(_, p)| p)
    }

    pub(in crate::dsl) fn select_ode_init(&self, ds_idx: usize) -> Option<&CompiledOdeInit> {
        let idx = self.ode_init_by_ds.get(&ds_idx).copied().or(self.default_ode_init_idx)?;
        self.ode_inits.get(idx).filter(|(f, _)| f.matches(ds_idx)).map(|(_, init)| init)
    }

    /// Solve the implicit equation `F(x, y; theta) = 0` for `y`: test `y = 0`, scan positive and
    /// negative half-lines outward across a multi-decade, scale-augmented probe grid to bracket a
    /// sign change, refine with Brent (1973) at scale-matched tolerances (`abs_tol` scaled to the
    /// bracket, `rel_tol = 1e-13`, with a tight `[0, root]` re-bracket near zero), and validate the
    /// root against poles/discontinuities via `verify_implicit_root` (`|F(root)| <= max(1e-2 *
    /// local_var, 1e-3 * end_min, 1e-12 * end_max)`). If no sign change is bracketed, fall back to
    /// damped Newton iteration (`implicit_newton_fallback`) from the lowest-`|F|` probes using
    /// scale-invariant residual and step tolerances (`|F| <= 1e-12 * ref_f` mid-loop or `<= 1e-11 *
    /// ref_f` on exit). Only one verified root is returned (the first found), so equations with
    /// several real roots in `y` may select a branch that depends on the probe order.
    pub(in crate::dsl) fn eval_implicit(&self, x: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        let prog = self.select_program(ds_idx)?;
        let f = |y: f64| -> f64 { eval_bytecode(prog, x, y, params, ds_idx).unwrap_or(f64::NAN) };
        let verify_implicit_root = |root: f64, lo: f64, hi: f64, abs_tol: f64| -> bool {
            if !root.is_finite() {
                return false;
            }
            let f_root = f(root);
            if !f_root.is_finite() {
                return false;
            }
            if f_root == 0.0 {
                return true;
            }
            let f_lo = f(lo);
            let f_hi = f(hi);
            if !f_lo.is_finite() || !f_hi.is_finite() {
                return false;
            }
            let end_min = f_lo.abs().min(f_hi.abs());
            let end_max = f_lo.abs().max(f_hi.abs());
            if end_min > 0.0 && f_root.abs() >= end_min && f_root.abs() > 1e-12 * end_max {
                return false;
            }
            let delta = (root.abs() * 1e-7).max(abs_tol * 16.0).max(f64::MIN_POSITIVE * 1e8);
            let f_p = f(root + delta);
            let f_m = f(root - delta);
            let local_var = match (f_p.is_finite(), f_m.is_finite()) {
                (true, true) => {
                    if f_root.abs() > 2.0 * f_p.abs().max(f_m.abs()) {
                        return false;
                    }
                    (f_p - f_m).abs()
                }
                (true, false) => {
                    if f_root.abs() > 2.0 * f_p.abs() {
                        return false;
                    }
                    (f_p - f_root).abs()
                }
                (false, true) => {
                    if f_root.abs() > 2.0 * f_m.abs() {
                        return false;
                    }
                    (f_root - f_m).abs()
                }
                (false, false) => return false,
            };
            let allowed = (1e-2 * local_var).max(1e-3 * end_min).max(1e-12 * end_max);
            f_root.abs() <= allowed
        };
        let brent_scaled = |lo: f64, hi: f64| -> Option<f64> {
            let bracket_scale = lo.abs().max(hi.abs());
            let abs_tol = if bracket_scale > 0.0 { (bracket_scale * 1e-14).max(f64::MIN_POSITIVE * 1e6) } else { 1e-300 };
            let root = brent_root(&f, lo, hi, RootOptions { abs_tol, rel_tol: 1e-13, max_iters: 120 }).ok()?;
            if !root.is_finite() {
                return None;
            }
            if root != 0.0 && lo <= 0.0 && hi >= 0.0 && root.abs() < 1e-3 * bracket_scale {
                let (tight_lo, tight_hi) = if root > 0.0 { (0.0, (2.0 * root).min(hi)) } else { ((2.0 * root).max(lo), 0.0) };
                let f_tlo = f(tight_lo);
                let f_thi = f(tight_hi);
                if f_tlo.is_finite() && f_thi.is_finite() && (f_tlo == 0.0 || f_thi == 0.0 || (f_tlo > 0.0) != (f_thi > 0.0)) {
                    let tight_tol = (root.abs() * 1e-14).max(f64::MIN_POSITIVE * 1e6);
                    if let Ok(refined) = brent_root(&f, tight_lo, tight_hi, RootOptions { abs_tol: tight_tol, rel_tol: 1e-13, max_iters: 120 })
                        && verify_implicit_root(refined, tight_lo, tight_hi, tight_tol)
                    {
                        return Some(refined);
                    }
                }
            }
            verify_implicit_root(root, lo, hi, abs_tol).then_some(root)
        };

        let f0 = f(0.0);
        if f0 == 0.0 {
            return Some(0.0);
        }

        let (pos_probes, anchor_scale) = implicit_probes_and_scale(x, params);
        let mut best_starts: Vec<(f64, f64)> = Vec::new();
        let mut push_finite = |g: f64, val: f64| {
            if best_starts.len() < 3 {
                best_starts.push((g, val));
            } else if let Some((worst_idx, &(_, worst_val))) = best_starts.iter().enumerate().max_by(|a, b| a.1.1.abs().total_cmp(&b.1.1.abs()))
                && val.abs() < worst_val.abs()
            {
                best_starts[worst_idx] = (g, val);
            }
        };
        if f0.is_finite() {
            push_finite(0.0, f0);
        }

        // Scan positive then negative half-lines outward from 0 so tight scale-matched brackets
        // are tested before large magnitudes and positive physical roots are found first.
        for sign in [1.0_f64, -1.0_f64] {
            let mut prev_y: Option<f64> = if f0.is_finite() { Some(0.0) } else { None };
            let mut prev_val = f0;
            for &m in &pos_probes {
                let g = sign * m;
                let val = f(g);
                if val.is_finite() {
                    if val == 0.0 {
                        return Some(g);
                    }
                    push_finite(g, val);
                    if let Some(py) = prev_y
                        && (prev_val > 0.0) != (val > 0.0)
                        && let Some(root) = brent_scaled(py.min(g), py.max(g))
                    {
                        return Some(root);
                    }
                    prev_y = Some(g);
                    prev_val = val;
                }
            }
        }

        best_starts.sort_by(|a, b| a.1.abs().total_cmp(&b.1.abs()));
        implicit_newton_fallback(&f, &brent_scaled, &best_starts, anchor_scale)
    }

    /// Integrate ODE `dy/dt = f(t, y; theta)` from `t0` to target `t_end` using adaptive Dormand–Prince RK45
    /// with scale-invariant tolerances, VM-operation budget accounting, and order-independent frontier caching.
    pub(in crate::dsl) fn eval_ode_single(&self, t_end: f64, x_tail: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        let prog = self.select_program(ds_idx)?;
        let init = self.select_ode_init(ds_idx)?;
        let t0 = init.t0;
        if !t0.is_finite() || !t_end.is_finite() {
            return None;
        }

        let mut budget = OdeEvalBudget { remaining_ops: MAX_ODE_VM_OPS_PER_EVAL, exceeded: false, stack: Vec::with_capacity(32) };
        let mut xv = Vec::with_capacity(1 + x_tail.len());
        xv.push(t0);
        xv.extend_from_slice(x_tail);
        let mut init_locals = vec![None; init.prog.local_count];
        let y0 = budget.eval(&init.prog, &xv, 0.0, params, ds_idx, &mut init_locals)?;
        if !y0.is_finite() {
            return None;
        }

        let dir = if t_end >= t0 { 1.0 } else { -1.0 };
        let target_dist = (t_end - t0) * dir;
        if target_dist <= 4.0 * f64::EPSILON * t_end.abs().max(t0.abs()) {
            return Some(y0);
        }
        if !target_dist.is_finite() {
            return None;
        }

        let mut traj = take_cached_ode_trajectory(self.model_id, ds_idx, t0, y0, params, x_tail);
        let side = if dir > 0.0 { &mut traj.pos } else { &mut traj.neg };

        if let Some(cached) = side.lookup_point(t_end.to_bits()) {
            store_cached_ode_trajectory(traj);
            return Some(cached);
        }
        if side.failed_beyond_dist.is_some_and(|fail_d| target_dist > fail_d) {
            store_cached_ode_trajectory(traj);
            return None;
        }

        let mut rhs_locals = vec![None; prog.local_count];
        let mut rhs_src = VmOdeRhs { prog, xv: &mut xv, params, ds_idx, rhs_locals: &mut rhs_locals, budget: &mut budget };
        let result = eval_ode_on_side(side, t0, t_end, dir, target_dist, &mut rhs_src);
        store_cached_ode_trajectory(traj);
        result
    }
}

struct OdeEvalBudget {
    remaining_ops: usize,
    exceeded: bool,
    stack: Vec<f64>,
}

impl OdeEvalBudget {
    fn is_exceeded(&self) -> bool {
        self.exceeded || ode_fit_budget_exceeded().is_some()
    }

    fn eval(&mut self, prog: &CompiledProgram, x: &[f64], y: f64, params: &[f64], ds_idx: usize, locals: &mut [Option<f64>]) -> Option<f64> {
        if self.is_exceeded() || self.remaining_ops == 0 {
            self.exceeded = true;
            return None;
        }
        self.stack.clear();
        let (res, ops) = eval_bytecode_counted(prog, x, y, params, ds_idx, locals, &mut self.stack);
        if ops > self.remaining_ops {
            self.remaining_ops = 0;
            self.exceeded = true;
            let _ = consume_ode_fit_vm_ops(ops);
            return None;
        }
        self.remaining_ops -= ops;
        if !consume_ode_fit_vm_ops(ops) {
            self.exceeded = true;
            return None;
        }
        res
    }
}

struct VmOdeRhs<'a> {
    prog: &'a CompiledProgram,
    xv: &'a mut [f64],
    params: &'a [f64],
    ds_idx: usize,
    rhs_locals: &'a mut [Option<f64>],
    budget: &'a mut OdeEvalBudget,
}

impl OdeRhsSource for VmOdeRhs<'_> {
    fn eval_rhs(&mut self, t: f64, y: f64) -> Option<f64> {
        self.xv[0] = t;
        self.rhs_locals.fill(None);
        self.budget.eval(self.prog, self.xv, y, self.params, self.ds_idx, self.rhs_locals)
    }
    fn budget_exceeded(&self) -> bool {
        self.budget.is_exceeded()
    }
    fn charge_fit_step(&self) -> bool {
        consume_ode_fit_rk_step()
    }
}

impl ParametricModel for CompiledDslModel {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn category(&self) -> EquationCategory {
        EquationCategory::UserDefined
    }
    fn param_names(&self) -> &[String] {
        &self.param_names
    }
    fn eval(&self, x: &[f64], params: &[f64], dataset_index: usize) -> Option<f64> {
        match self.kind {
            DslEquationKind::Explicit => {
                let prog = self.select_program(dataset_index)?;
                eval_bytecode(prog, x, 0.0, params, dataset_index)
            }
            DslEquationKind::Implicit => self.eval_implicit(x, params, dataset_index),
            DslEquationKind::OdeFirstOrder => {
                let t = x.first().copied()?;
                let tail = if x.len() > 1 { &x[1..] } else { &[] };
                self.eval_ode_single(t, tail, params, dataset_index)
            }
        }
    }
    fn initial_guess(&self, points: &[DataPoint]) -> Vec<f64> {
        let finite_ys: Vec<f64> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| p.y).collect();
        let mean_y = if finite_ys.is_empty() {
            1.0
        } else {
            let n = finite_ys.len() as f64;
            let avg = finite_ys.iter().map(|&y| y / n).sum::<f64>();
            if avg.is_finite() { avg } else { 1.0 }
        };
        let mut guesses: Vec<f64> = self
            .param_names
            .iter()
            .map(|p| {
                if let Some(&v) = self.initial_hints.get(p) {
                    return v;
                }
                if p.eq_ignore_ascii_case("Y0") || p.eq_ignore_ascii_case("Top") { mean_y.max(0.1) } else { 1.0 }
            })
            .collect();
        if self.initial_rules.is_empty() {
            return guesses;
        }
        let stats = compute_dataset_stats(points);
        let same_ds = points.first().map(|p0| p0.data_set_index).filter(|&first| points.iter().all(|p| p.data_set_index == first));
        let eval_ds = same_ds.unwrap_or(0);
        let mut applied = vec![false; guesses.len()];
        let mut locals = vec![None; self.init_local_count];
        for pass in 0..2 {
            if pass == 1 && same_ds.is_some() {
                break;
            }
            for rule in &self.initial_rules {
                let matches = match (same_ds, pass) {
                    (Some(ds), 0) => rule.filter.matches(ds),
                    (Some(_), _) => false,
                    (None, 0) => rule.filter == DatasetFilter::Any,
                    (None, _) => rule.filter != DatasetFilter::Any && rule.filter.matches(0),
                };
                if !matches {
                    continue;
                }
                if let Some(val) = eval_bytecode_with_locals(&rule.prog, &[0.0], f64::NAN, &stats, eval_ds, &mut locals)
                    && val.is_finite()
                {
                    if rule.slot < locals.len() {
                        locals[rule.slot] = Some(val);
                    }
                    if let Some(p_idx) = rule.param_idx
                        && !(pass == 1 && applied[p_idx])
                    {
                        guesses[p_idx] = val;
                        applied[p_idx] = true;
                    }
                }
            }
        }
        guesses
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::with_capacity(self.transforms.len());
        let mut locals = vec![None; self.transform_local_count];
        for (name, slot, prog) in &self.transforms {
            if let Some(val) = eval_bytecode_with_locals(prog, &[0.0], f64::NAN, params, 0, &mut locals)
                && val.is_finite()
            {
                if *slot < locals.len() {
                    locals[*slot] = Some(val);
                }
                out.push(DerivedScalar { name: name.clone(), value: val, units: None });
            } else if *slot < locals.len() {
                locals[*slot] = None;
            }
        }
        out
    }
}
