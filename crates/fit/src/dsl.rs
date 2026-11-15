//! Custom Equation Bytecode Compiler, Stack VM, Implicit Solver, and Dormand–Prince RK45 ODE Solver.
//!
//! Supports:
//! - Standard algebraic formulas (`Y = ...` or bare expressions), multiline local variables
//! - Short-circuit `IF(cond, then, else)` conditional piecewise models and `AND`/`OR`/`NOT`
//! - Dataset-indexed line prefixes (`<A>`, `<B>`, `<~A>`, `<A:C>`) for global multi-curve models
//! - Implicit equations (`LHS = RHS` involving `Y`, solved via bracket scan + `brent_root`)
//! - First-order ODE models (`dY/dX = ...` / `dY/dt = ...` / `D(Y, X) = ...`) integrated with
//!   adaptive Dormand–Prince RK45 (`rtol`, `atol`, step rejection, bounded steps)
//! - Rich scalar math + statistical/special functions (Bessel J/Y/I/K, Kummer 1F1, Gauss 2F1,
//!   `erf`, `gamma`, `normcdf`, etc.) executed in a bounded, WASM-safe stack VM.
//!
//! # References
//!
//! Short `Author (Year)` citations on individual items refer to the list below.
//!
//! Reference: Dormand, J. R. & Prince, P. J. (1980). A family of embedded Runge–Kutta formulae. *Journal of Computational and Applied Mathematics* 6(1), 19–26. doi:10.1016/0771-050X(80)90013-3
//!
//! Reference: Hairer, E., Nørsett, S. P. & Wanner, G. (1993). *Solving Ordinary Differential Equations I: Nonstiff Problems*, 2nd rev. ed. Springer Series in Computational Mathematics 8. Springer.
//!
//! Reference: Brent, R. P. (1973). *Algorithms for Minimization without Derivatives*. Prentice-Hall.
//!
//! Reference: Abramowitz, M. & Stegun, I. A. (eds.) (1964). *Handbook of Mathematical Functions with Formulas, Graphs, and Mathematical Tables*. National Bureau of Standards Applied Mathematics Series 55. U.S. Government Printing Office.
//!
//! Reference: Press, W. H., Teukolsky, S. A., Vetterling, W. T. & Flannery, B. P. (1992). *Numerical Recipes in C: The Art of Scientific Computing*, 2nd ed. Cambridge University Press.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;

use lumencraft_math::dist::{norm_cdf, norm_ppf};
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::{beta, digamma, erf, erfc, gamma, ln_gamma};

use crate::error::FitError;
use crate::library::{ParametricModel, estimate_x_at_ymid};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};

const MAX_VM_STEPS: usize = 100_000;
const MAX_ODE_STEPS: usize = 50_000;
const MAX_PARSE_DEPTH: usize = 128;
const MAX_AST_NODES: usize = 4_096;
const MAX_DATASET_EXPANSION: usize = 10_000;
const MAX_BESSEL_ORDER: i32 = 100_000;

/// Dataset-filter predicate parsed from `<A>`, `<~A>`, `<B:D>`, or `<0>` line prefixes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetFilter {
    Any,
    Include(Vec<usize>),
    Exclude(Vec<usize>),
}

impl DatasetFilter {
    #[must_use]
    pub fn matches(&self, ds_idx: usize) -> bool {
        match self {
            Self::Any => true,
            Self::Include(v) => v.contains(&ds_idx),
            Self::Exclude(v) => !v.contains(&ds_idx),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Number,
    Ident,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    Eq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    text: String,
    num: f64,
    line: usize,
    col: usize,
}

enum Expr {
    Num(f64),
    Ident(String),
    Unary { op: UnaryOp, rhs: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
    If { cond: Box<Expr>, then_br: Box<Expr>, else_br: Box<Expr> },
}

impl Expr {
    fn is_leaf(&self) -> bool {
        matches!(self, Self::Num(_) | Self::Ident(_))
    }

    fn needs_iterative_drop(&self) -> bool {
        match self {
            Self::Num(_) | Self::Ident(_) => false,
            Self::Unary { rhs, .. } => !rhs.is_leaf(),
            Self::Binary { lhs, rhs, .. } => !lhs.is_leaf() || !rhs.is_leaf(),
            Self::Call { args, .. } => args.iter().any(|a| !a.is_leaf()),
            Self::If { cond, then_br, else_br } => !cond.is_leaf() || !then_br.is_leaf() || !else_br.is_leaf(),
        }
    }
}

impl Clone for Expr {
    fn clone(&self) -> Self {
        match self {
            Self::Num(v) => return Self::Num(*v),
            Self::Ident(name) => return Self::Ident(name.clone()),
            _ => {}
        }
        enum CloneTask<'a> {
            Visit(&'a Expr),
            BuildUnary(UnaryOp),
            BuildBinary(BinaryOp),
            BuildCall(String, usize),
            BuildIf,
        }
        let mut tasks = vec![CloneTask::Visit(self)];
        let mut results: Vec<Expr> = Vec::new();
        while let Some(task) = tasks.pop() {
            match task {
                CloneTask::Visit(node) => match node {
                    Expr::Num(v) => results.push(Expr::Num(*v)),
                    Expr::Ident(name) => results.push(Expr::Ident(name.clone())),
                    Expr::Unary { op, rhs } => {
                        tasks.push(CloneTask::BuildUnary(*op));
                        tasks.push(CloneTask::Visit(rhs));
                    }
                    Expr::Binary { op, lhs, rhs } => {
                        tasks.push(CloneTask::BuildBinary(*op));
                        tasks.push(CloneTask::Visit(rhs));
                        tasks.push(CloneTask::Visit(lhs));
                    }
                    Expr::Call { name, args } => {
                        tasks.push(CloneTask::BuildCall(name.clone(), args.len()));
                        for arg in args.iter().rev() {
                            tasks.push(CloneTask::Visit(arg));
                        }
                    }
                    Expr::If { cond, then_br, else_br } => {
                        tasks.push(CloneTask::BuildIf);
                        tasks.push(CloneTask::Visit(else_br));
                        tasks.push(CloneTask::Visit(then_br));
                        tasks.push(CloneTask::Visit(cond));
                    }
                },
                CloneTask::BuildUnary(op) => {
                    if let Some(rhs) = results.pop() {
                        results.push(Expr::Unary { op, rhs: Box::new(rhs) });
                    }
                }
                CloneTask::BuildBinary(op) => {
                    if let (Some(rhs), Some(lhs)) = (results.pop(), results.pop()) {
                        results.push(Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) });
                    }
                }
                CloneTask::BuildCall(name, arity) => {
                    let start = results.len().saturating_sub(arity);
                    let args = results.split_off(start);
                    results.push(Expr::Call { name, args });
                }
                CloneTask::BuildIf => {
                    if let (Some(else_br), Some(then_br), Some(cond)) = (results.pop(), results.pop(), results.pop()) {
                        results.push(Expr::If { cond: Box::new(cond), then_br: Box::new(then_br), else_br: Box::new(else_br) });
                    }
                }
            }
        }
        results.pop().unwrap_or(Self::Num(0.0))
    }
}

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        enum DebugTask<'a> {
            Expr(&'a Expr),
            Str(&'static str),
        }
        let mut tasks = vec![DebugTask::Expr(self)];
        while let Some(task) = tasks.pop() {
            match task {
                DebugTask::Str(s) => f.write_str(s)?,
                DebugTask::Expr(node) => match node {
                    Self::Num(v) => write!(f, "Num({v:?})")?,
                    Self::Ident(name) => write!(f, "Ident({name:?})")?,
                    Self::Unary { op, rhs } => {
                        write!(f, "Unary {{ op: {op:?}, rhs: ")?;
                        tasks.push(DebugTask::Str(" }"));
                        tasks.push(DebugTask::Expr(rhs));
                    }
                    Self::Binary { op, lhs, rhs } => {
                        write!(f, "Binary {{ op: {op:?}, lhs: ")?;
                        tasks.push(DebugTask::Str(" }"));
                        tasks.push(DebugTask::Expr(rhs));
                        tasks.push(DebugTask::Str(", rhs: "));
                        tasks.push(DebugTask::Expr(lhs));
                    }
                    Self::Call { name, args } => {
                        write!(f, "Call {{ name: {name:?}, args: [")?;
                        tasks.push(DebugTask::Str("] }"));
                        for (idx, arg) in args.iter().enumerate().rev() {
                            tasks.push(DebugTask::Expr(arg));
                            if idx > 0 {
                                tasks.push(DebugTask::Str(", "));
                            }
                        }
                    }
                    Self::If { cond, then_br, else_br } => {
                        f.write_str("If { cond: ")?;
                        tasks.push(DebugTask::Str(" }"));
                        tasks.push(DebugTask::Expr(else_br));
                        tasks.push(DebugTask::Str(", else_br: "));
                        tasks.push(DebugTask::Expr(then_br));
                        tasks.push(DebugTask::Str(", then_br: "));
                        tasks.push(DebugTask::Expr(cond));
                    }
                },
            }
        }
        Ok(())
    }
}

impl Drop for Expr {
    fn drop(&mut self) {
        if !self.needs_iterative_drop() {
            return;
        }
        let mut stack = Vec::new();
        stack.push(std::mem::replace(self, Self::Num(0.0)));
        while let Some(mut node) = stack.pop() {
            match &mut node {
                Self::Num(_) | Self::Ident(_) => {}
                Self::Unary { rhs, .. } => {
                    if !rhs.is_leaf() {
                        stack.push(std::mem::replace(rhs.as_mut(), Self::Num(0.0)));
                    }
                }
                Self::Binary { lhs, rhs, .. } => {
                    if !lhs.is_leaf() {
                        stack.push(std::mem::replace(lhs.as_mut(), Self::Num(0.0)));
                    }
                    if !rhs.is_leaf() {
                        stack.push(std::mem::replace(rhs.as_mut(), Self::Num(0.0)));
                    }
                }
                Self::Call { args, .. } => {
                    for arg in args.iter_mut() {
                        if !arg.is_leaf() {
                            stack.push(std::mem::replace(arg, Self::Num(0.0)));
                        }
                    }
                }
                Self::If { cond, then_br, else_br } => {
                    if !cond.is_leaf() {
                        stack.push(std::mem::replace(cond.as_mut(), Self::Num(0.0)));
                    }
                    if !then_br.is_leaf() {
                        stack.push(std::mem::replace(then_br.as_mut(), Self::Num(0.0)));
                    }
                    if !else_br.is_leaf() {
                        stack.push(std::mem::replace(else_br.as_mut(), Self::Num(0.0)));
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

/// Safe bytecode instructions executed by the DSL stack VM.
#[derive(Debug, Clone)]
pub enum OpCode {
    PushConst(f64),
    LoadPredictor(usize),
    LoadY,
    LoadDatasetIndex,
    LoadParam(usize),
    LoadLocal(usize),
    StoreLocal(usize),
    Unary(UnaryKind),
    Binary(BinaryKind),
    CallBuiltin(BuiltinFn, usize),
    Jump(usize),
    JumpIfZero(usize),
    Pop,
    Halt,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryKind {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryKind {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy)]
pub enum BuiltinFn {
    Abs,
    Sqrt,
    Cbrt,
    Exp,
    Exp10,
    Ln,
    Log10,
    Log2,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,
    Asinh,
    Acosh,
    Atanh,
    Floor,
    Ceil,
    Round,
    Trunc,
    Sign,
    Min,
    Max,
    Clamp,
    Hypot,
    Erf,
    Erfc,
    NormCdf,
    NormInv,
    Gamma,
    LnGamma,
    Beta,
    Digamma,
    BesselJ0,
    BesselJ1,
    BesselJn,
    BesselY0,
    BesselY1,
    BesselYn,
    BesselI0,
    BesselI1,
    BesselK0,
    BesselK1,
    Hyp1F1,
    Hyp2F1,
    ConfluentU,
    PowFn,
}

/// Compiled custom equation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DslEquationKind {
    Explicit,
    Implicit,
    OdeFirstOrder,
}

/// Compiled statement/program chunk gated by a [`DatasetFilter`].
#[derive(Debug, Clone)]
struct CompiledProgram {
    code: Vec<OpCode>,
    local_count: usize,
}

/// Compiled custom equation implementing [`ParametricModel`] safely inside a bounded VM.
#[derive(Debug, Clone)]
pub struct CompiledDslModel {
    id: String,
    name: String,
    kind: DslEquationKind,
    param_names: Vec<String>,
    programs: Vec<(DatasetFilter, CompiledProgram)>,
    ode_init_program: Option<CompiledProgram>,
    ode_t0: f64,
    initial_hints: HashMap<String, f64>,
    initial_rules: Vec<(DatasetFilter, String, CompiledProgram)>,
    transforms: Vec<(String, CompiledProgram)>,
}

impl CompiledDslModel {
    /// Compile a user equation string (explicit `Y=...`, conditional `IF`, `<A>` dataset lines,
    /// implicit `LHS=RHS`, or 1st-order ODE `dY/dX=...` / `D(Y, X)=...`).
    pub fn compile(id: impl Into<String>, source: &str) -> Result<Self, FitError> {
        let id = id.into();
        let mut section = DslSection::Equation;
        let mut parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)> = Vec::new();
        let mut raw_init_lines: Vec<(usize, DatasetFilter, String, Expr)> = Vec::new();
        let mut raw_transform_lines: Vec<(usize, String, Expr)> = Vec::new();
        let mut initial_hints = HashMap::new();
        let mut ode_t0 = 0.0_f64;

        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let trimmed = strip_comment(raw_line, &mut initial_hints, &mut ode_t0);
            if trimmed.is_empty() {
                continue;
            }
            if let Some(next_section) = parse_section_header(trimmed, line_no)? {
                section = next_section;
                continue;
            }
            let (filter, rest) = parse_dataset_prefix(trimmed, line_no)?;
            if rest.is_empty() {
                continue;
            }
            match section {
                DslSection::Equation => {
                    let stmt = parse_statement(rest, line_no)?;
                    parsed_lines.push((line_no, filter, stmt));
                }
                DslSection::InitialValues => match parse_statement(rest, line_no)? {
                    ParsedStatement::Assign { name, rhs } => raw_init_lines.push((line_no, filter, name, rhs)),
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
                        ParsedStatement::Assign { name, rhs } => raw_transform_lines.push((line_no, name, rhs)),
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

        let ResolvedEquationBody { kind, body_stmts, init_stmts, ode_t0 } = resolve_equation_branches(parsed_lines, ode_t0)?;
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
        let empty_locals = BTreeSet::new();
        for (_, rhs) in &init_stmts {
            collect_free_idents(rhs, &empty_locals, &mut seen_params, &mut param_order);
        }
        if kind == DslEquationKind::OdeFirstOrder && init_stmts.is_empty() && !seen_params.contains("Y0") {
            param_order.push("Y0".to_owned());
            seen_params.insert("Y0".to_owned());
        }

        let param_map: HashMap<String, usize> = param_order.iter().enumerate().map(|(i, p)| (p.clone(), i)).collect();

        let mut programs = Vec::with_capacity(body_stmts.len());
        for (filter, locals, target) in body_stmts {
            let prog = compile_program(&locals, &target, &param_map)?;
            programs.push((filter, prog));
        }

        let ode_init_program = if kind == DslEquationKind::OdeFirstOrder {
            if let Some((_, init_expr)) = init_stmts.last() {
                Some(compile_program(&[], init_expr, &param_map)?)
            } else {
                let y0_idx = *param_map.get("Y0").unwrap_or(&0);
                Some(CompiledProgram { code: vec![OpCode::LoadParam(y0_idx), OpCode::Halt], local_count: 0 })
            }
        } else {
            None
        };

        let initial_rules = compile_initial_rules(raw_init_lines)?;
        let transforms = compile_transforms(raw_transform_lines, &param_map)?;

        Ok(Self {
            name: id.clone(),
            id,
            kind,
            param_names: param_order,
            programs,
            ode_init_program,
            ode_t0,
            initial_hints,
            initial_rules,
            transforms,
        })
    }

    #[must_use]
    pub fn kind(&self) -> DslEquationKind {
        self.kind
    }

    fn select_program(&self, ds_idx: usize) -> Option<&CompiledProgram> {
        self.programs.iter().rfind(|(f, _)| f.matches(ds_idx)).map(|(_, p)| p)
    }

    /// Solve the implicit equation `F(x, y; theta) = 0` for `y`: bracket a sign change over a
    /// fixed list of trial `y` values and refine with Brent (1973); if no bracket is found, fall
    /// back to a step-clamped Newton iteration with central-difference derivative and accept
    /// only `|F| < 1e-7`. Only one root is returned (the first bracket found), so equations with
    /// several real roots in `y` may select a branch that depends on the trial order.
    fn eval_implicit(&self, x: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        let prog = self.select_program(ds_idx)?;
        let f = |y: f64| -> f64 { eval_bytecode(prog, x, y, params, ds_idx).unwrap_or(f64::NAN) };
        let guesses = [0.0, 1.0, -1.0, 10.0, -10.0, 100.0, 0.1, -0.1, 1e-4, 1000.0];
        let mut last_y: Option<f64> = None;
        let mut last_val = f64::NAN;
        for &g in &guesses {
            let val = f(g);
            if val.is_finite() {
                if val.abs() < 1e-13 {
                    return Some(g);
                }
                if let Some(prev_y) = last_y
                    && last_val * val < 0.0
                    && let Ok(root) = brent_root(f, prev_y.min(g), prev_y.max(g), RootOptions { abs_tol: 1e-12, rel_tol: 1e-12, max_iters: 100 })
                {
                    return Some(root);
                }
                last_y = Some(g);
                last_val = val;
            }
        }
        // Try safeguarded secant/Newton around initial finite point.
        let mut y = last_y?;
        for _ in 0..40 {
            let fy = f(y);
            if !fy.is_finite() {
                return None;
            }
            if fy.abs() < 1e-12 {
                return Some(y);
            }
            let h = 1e-6 * (1.0 + y.abs());
            let df = (f(y + h) - f(y - h)) / (2.0 * h);
            if !df.is_finite() || df.abs() < 1e-15 {
                break;
            }
            y -= (fy / df).clamp(-100.0, 100.0);
        }
        if f(y).abs() < 1e-7 { Some(y) } else { None }
    }

    /// Integrate ODE `dy/dt = f(t, y; theta)` from `t0` to target `t_end` using adaptive Dormand–Prince RK45.
    fn eval_ode_single(&self, t_end: f64, x_tail: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        let prog = self.select_program(ds_idx)?;
        let init_prog = self.ode_init_program.as_ref()?;
        if !self.ode_t0.is_finite() || !t_end.is_finite() {
            return None;
        }
        let y0 = eval_bytecode(init_prog, &[self.ode_t0], 0.0, params, ds_idx)?;
        if !y0.is_finite() {
            return None;
        }
        if (t_end - self.ode_t0).abs() < 1e-15 {
            return Some(y0);
        }
        integrate_rk45(self.ode_t0, t_end, y0, |t, y| {
            let mut xv = Vec::with_capacity(1 + x_tail.len());
            xv.push(t);
            xv.extend_from_slice(x_tail);
            eval_bytecode(prog, &xv, y, params, ds_idx)
        })
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
        let mean_y = if points.is_empty() { 1.0 } else { points.iter().map(|p| p.y).sum::<f64>() / points.len() as f64 };
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
        for pass in 0..2 {
            for (filter, name, prog) in &self.initial_rules {
                let matches = match (same_ds, pass) {
                    (Some(ds), 0) => filter.matches(ds),
                    (Some(_), _) => false,
                    (None, 0) => *filter == DatasetFilter::Any,
                    (None, _) => filter.matches(0),
                };
                if !matches {
                    continue;
                }
                let Some(p_idx) =
                    self.param_names.iter().position(|p| p == name).or_else(|| self.param_names.iter().position(|p| p.eq_ignore_ascii_case(name)))
                else {
                    continue;
                };
                if pass == 1 && applied[p_idx] {
                    continue;
                }
                if let Some(val) = eval_bytecode(prog, &[0.0], f64::NAN, &stats, eval_ds)
                    && val.is_finite()
                {
                    guesses[p_idx] = val;
                    applied[p_idx] = true;
                }
            }
        }
        guesses
    }

    fn derived_transforms(&self, params: &[f64]) -> Vec<DerivedScalar> {
        let mut out = Vec::with_capacity(self.transforms.len());
        for (name, prog) in &self.transforms {
            if let Some(val) = eval_bytecode(prog, &[0.0], f64::NAN, params, 0)
                && val.is_finite()
            {
                out.push(DerivedScalar { name: name.clone(), value: val, units: None });
            }
        }
        out
    }
}

/// Adaptive Dormand–Prince 5(4) Runge–Kutta integrator from `t0` to `t_end` for scalar `y`.
///
/// Butcher tableau, 5th-order propagating weights, and embedded error weights `b - b*` of
/// Dormand & Prince (1980) (local extrapolation: the 5th-order solution is advanced). Error
/// control follows Hairer, Nørsett & Wanner (1993), Sec. II.4: `err <= atol + rtol * max(|y|,
/// |y_new|)` with `rtol = 1e-8`, `atol = 1e-10`; accepted steps grow by
/// `0.9 * ratio^(-1/5)` clamped to `[0.2, 5]`.
///
/// Deviations: rejected steps shrink by `0.9 * ratio^(-1/4)` clamped to `[0.1, 0.5]` (the
/// reference controller uses exponent `-1/5` throughout); the first-same-as-last stage is
/// recomputed rather than reused (cost only); there is no automatic initial step selection
/// (the first step is 5% of the span). Non-finite stages shrink the step by 4; integration
/// fails (`None`) below `1e-13 * max(span, 1)` or after `MAX_ODE_STEPS` steps.
pub fn integrate_rk45<F>(t0: f64, t_end: f64, y0: f64, mut rhs: F) -> Option<f64>
where
    F: FnMut(f64, f64) -> Option<f64>,
{
    if !t0.is_finite() || !t_end.is_finite() || !y0.is_finite() {
        return None;
    }
    let dir = if t_end >= t0 { 1.0 } else { -1.0 };
    let span = (t_end - t0).abs();
    if span == 0.0 {
        return Some(y0);
    }
    if !span.is_finite() {
        return None;
    }
    let rtol = 1e-8_f64;
    let atol = 1e-10_f64;

    let mut t = t0;
    let mut y = y0;
    let min_init_h = (span * 1e-6).max(1e-18).min(span);
    let mut h = (span * 0.05).max(min_init_h).min(span);
    let min_h = (1e-13 * span.max(1.0)).min(span * 0.5);
    if min_h <= 0.0 {
        return None;
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

    for _ in 0..MAX_ODE_STEPS {
        let rem = (t_end - t) * dir;
        if rem <= 1e-14 * span || rem <= 4.0 * f64::EPSILON * t_end.abs() {
            return Some(y);
        }
        if h > rem {
            h = rem;
        }
        let dt = dir * h;
        let k1 = rhs(t, y)?;
        let k2 = rhs(t + 0.2 * dt, y + dt * A21 * k1)?;
        let k3 = rhs(t + 0.3 * dt, y + dt * (A31 * k1 + A32 * k2))?;
        let k4 = rhs(t + 0.8 * dt, y + dt * (A41 * k1 + A42 * k2 + A43 * k3))?;
        let k5 = rhs(t + (8.0 / 9.0) * dt, y + dt * (A51 * k1 + A52 * k2 + A53 * k3 + A54 * k4))?;
        let k6 = rhs(t + dt, y + dt * (A61 * k1 + A62 * k2 + A63 * k3 + A64 * k4 + A65 * k5))?;

        let y_next = y + dt * (B1 * k1 + B3 * k3 + B4 * k4 + B5 * k5 + B6 * k6);
        if !y_next.is_finite() {
            h *= 0.25;
            if h < min_h {
                return None;
            }
            continue;
        }
        let k7 = rhs(t + dt, y_next)?;
        let err = (dt * (E1 * k1 + E3 * k3 + E4 * k4 + E5 * k5 + E6 * k6 + E7 * k7)).abs();
        let tol = atol + rtol * y.abs().max(y_next.abs());
        let ratio = err / tol;
        if !err.is_finite() || !tol.is_finite() || !ratio.is_finite() {
            h *= 0.25;
            if h < min_h {
                return None;
            }
            continue;
        }

        if ratio <= 1.0 {
            let t_next = t + dt;
            if t_next == t {
                return if h >= rem { Some(y_next) } else { None };
            }
            t = t_next;
            y = y_next;
            let factor = if ratio < 1e-12 { 5.0 } else { (0.9 * ratio.powf(-0.2)).clamp(0.2, 5.0) };
            h = (h * factor).max(min_h);
        } else {
            let factor = (0.9 * ratio.powf(-0.25)).clamp(0.1, 0.5);
            h *= factor;
            if h < min_h {
                return None;
            }
        }
    }
    None
}

// ============================================================================
// Parser & Bytecode Compiler
// ============================================================================

enum ParsedStatement {
    OdeDerivative(Expr),
    OdeInitial { t0: Option<f64>, rhs: Expr },
    Assign { name: String, rhs: Expr },
    ImplicitEquation { lhs: Expr, rhs: Expr },
    BareExpr(Expr),
}

fn strip_comment<'a>(line: &'a str, hints: &mut HashMap<String, f64>, _ode_t0: &mut f64) -> &'a str {
    let trimmed = line.trim();
    for marker in ['#', ';'] {
        if let Some(pos) = trimmed.find(marker) {
            let comment = trimmed[pos + 1..].trim();
            if let Some(rest) = comment.strip_prefix("initial:") {
                for pair in rest.split(',') {
                    if let Some((k, v)) = pair.split_once('=')
                        && let Ok(num) = v.trim().parse::<f64>()
                        && num.is_finite()
                    {
                        hints.insert(k.trim().to_owned(), num);
                    }
                }
            }
            return trimmed[..pos].trim();
        }
    }
    trimmed
}

fn parse_dataset_prefix(line: &str, line_no: usize) -> Result<(DatasetFilter, &str), FitError> {
    if !line.starts_with('<') {
        return Ok((DatasetFilter::Any, line));
    }
    let Some(close) = line.find('>') else {
        return Err(FitError::DslCompileError { message: "Unclosed `<...>` dataset prefix".to_owned(), line: line_no, column: 1 });
    };
    let inside = line[1..close].trim();
    let rest = line[close + 1..].trim();
    let (negated, body) = if let Some(s) = inside.strip_prefix('~') { (true, s.trim()) } else { (false, inside) };
    let mut indices = Vec::new();
    for part in body.split(',') {
        let p = part.trim();
        if let Some((start_s, end_s)) = p.split_once(':') {
            let s_idx = parse_ds_token(start_s.trim(), line_no)?;
            let e_idx = parse_ds_token(end_s.trim(), line_no)?;
            if s_idx <= e_idx {
                let count = e_idx.checked_sub(s_idx).and_then(|d| d.checked_add(1));
                if count.is_none_or(|c| c > MAX_DATASET_EXPANSION || indices.len().saturating_add(c) > MAX_DATASET_EXPANSION) {
                    return Err(FitError::DslCompileError {
                        message: format!("Dataset range `{p}` exceeds maximum expansion limit ({MAX_DATASET_EXPANSION})"),
                        line: line_no,
                        column: 1,
                    });
                }
                indices.extend(s_idx..=e_idx);
            }
        } else if !p.is_empty() {
            if indices.len() >= MAX_DATASET_EXPANSION {
                return Err(FitError::DslCompileError {
                    message: format!("Dataset selector exceeds maximum limit ({MAX_DATASET_EXPANSION})"),
                    line: line_no,
                    column: 1,
                });
            }
            indices.push(parse_ds_token(p, line_no)?);
        }
    }
    let filter = if negated { DatasetFilter::Exclude(indices) } else { DatasetFilter::Include(indices) };
    Ok((filter, rest))
}

fn parse_ds_token(tok: &str, line_no: usize) -> Result<usize, FitError> {
    if let Ok(idx) = tok.parse::<usize>() {
        return Ok(idx);
    }
    let upper = tok.to_ascii_uppercase();
    if upper.len() == 1 {
        let b = upper.as_bytes()[0];
        if b.is_ascii_uppercase() {
            return Ok((b - b'A') as usize);
        }
    }
    Err(FitError::DslCompileError { message: format!("Invalid dataset selector `{tok}`"), line: line_no, column: 1 })
}

fn parse_statement(rest: &str, line_no: usize) -> Result<ParsedStatement, FitError> {
    // Detect `LHS = RHS` (without `==`) cleanly at top level.
    if let Some((lhs_s, rhs_s)) = split_top_level_assignment(rest) {
        let lhs_trim = lhs_s.trim();
        let lhs_compact: String = lhs_trim.chars().filter(|c| !c.is_whitespace()).collect();
        let lhs_upper = lhs_compact.to_ascii_uppercase();
        if lhs_upper == "DY/DX" || lhs_upper == "DY/DT" || lhs_upper == "D(Y,X)" || lhs_upper == "D(Y,T)" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeDerivative(rhs));
        }
        if lhs_upper == "Y0" || lhs_upper == "Y(0)" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { t0: Some(0.0), rhs });
        }
        if let Some(inner) = lhs_upper.strip_prefix("Y(").and_then(|s| s.strip_suffix(')'))
            && let Ok(t0_val) = inner.parse::<f64>()
        {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { t0: Some(t0_val), rhs });
        }
        if is_valid_ident(lhs_trim) {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::Assign { name: lhs_trim.to_owned(), rhs });
        }
        let lhs = parse_expr(lhs_trim, line_no)?;
        let rhs = parse_expr(rhs_s, line_no)?;
        return Ok(ParsedStatement::ImplicitEquation { lhs, rhs });
    }
    Ok(ParsedStatement::BareExpr(parse_expr(rest, line_no)?))
}

fn split_top_level_assignment(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut depth = 0_i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'=' if depth == 0 => {
                let prev = if i > 0 { bytes[i - 1] } else { 0 };
                let next = bytes.get(i + 1).copied().unwrap_or(0);
                if prev != b'!' && prev != b'<' && prev != b'>' && prev != b'=' && next != b'=' {
                    return Some((&s[..i], &s[i + 1..]));
                }
                if next == b'=' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_valid_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DslSection {
    Equation,
    InitialValues,
    Transforms,
}

const INIT_STAT_NAMES: [&str; 11] = ["YMIN", "YMAX", "XMIN", "XMAX", "XMID", "YMID", "XATYMIN", "XATYMAX", "XATY50", "SPAN", "SIGN"];

type OutputBranch = (DatasetFilter, Vec<(String, Expr)>, Expr);

enum EqItem {
    Local { filter: DatasetFilter, name: String, rhs: Expr },
    Explicit { filter: DatasetFilter, expr: Expr },
    Implicit { filter: DatasetFilter, diff: Expr },
    Ode { filter: DatasetFilter, rhs: Expr },
}

impl EqItem {
    fn filter(&self) -> &DatasetFilter {
        match self {
            Self::Local { filter, .. } | Self::Explicit { filter, .. } | Self::Implicit { filter, .. } | Self::Ode { filter, .. } => filter,
        }
    }
}

fn parse_section_header(trimmed: &str, line_no: usize) -> Result<Option<DslSection>, FitError> {
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Ok(None);
    }
    let inner: String =
        trimmed[1..trimmed.len() - 1].chars().filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-').flat_map(char::to_lowercase).collect();
    match inner.as_str() {
        "equation" | "equations" | "body" | "model" => Ok(Some(DslSection::Equation)),
        "initialvalues" | "initialvalue" | "initials" | "init" => Ok(Some(DslSection::InitialValues)),
        "transforms" | "transform" | "derived" | "derivedscalars" => Ok(Some(DslSection::Transforms)),
        _ => Err(FitError::DslCompileError { message: format!("Unknown DSL section `{trimmed}`"), line: line_no, column: 1 }),
    }
}

fn build_eq_items(
    parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>,
    mut ode_t0: f64,
) -> (DslEquationKind, Vec<EqItem>, Vec<(usize, Expr)>, f64) {
    let mut kind = DslEquationKind::Explicit;
    let mut items = Vec::with_capacity(parsed_lines.len());
    let mut init_stmts = Vec::new();
    for (line_no, filter, stmt) in parsed_lines {
        match stmt {
            ParsedStatement::OdeDerivative(rhs) => {
                kind = DslEquationKind::OdeFirstOrder;
                items.push(EqItem::Ode { filter, rhs });
            }
            ParsedStatement::OdeInitial { t0, rhs } => {
                if let Some(t) = t0 {
                    ode_t0 = t;
                }
                init_stmts.push((line_no, rhs));
            }
            ParsedStatement::Assign { name, rhs } => {
                if name.eq_ignore_ascii_case("Y") {
                    if expr_references_y(&rhs) {
                        kind = DslEquationKind::Implicit;
                        let diff = Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(Expr::Ident("Y".to_owned())), rhs: Box::new(rhs) };
                        items.push(EqItem::Implicit { filter, diff });
                    } else {
                        items.push(EqItem::Explicit { filter, expr: rhs });
                    }
                } else {
                    items.push(EqItem::Local { filter, name, rhs });
                }
            }
            ParsedStatement::ImplicitEquation { lhs, rhs } => {
                kind = DslEquationKind::Implicit;
                let diff = Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(lhs), rhs: Box::new(rhs) };
                items.push(EqItem::Implicit { filter, diff });
            }
            ParsedStatement::BareExpr(rhs) => {
                if expr_references_y(&rhs) {
                    kind = DslEquationKind::Implicit;
                    items.push(EqItem::Implicit { filter, diff: rhs });
                } else {
                    items.push(EqItem::Explicit { filter, expr: rhs });
                }
            }
        }
    }
    (kind, items, init_stmts, ode_t0)
}

fn collect_filter_classes(eq_items: &[EqItem]) -> (Vec<(DatasetFilter, usize)>, DatasetFilter, usize) {
    let mut non_any_filters: Vec<&DatasetFilter> = Vec::new();
    let mut mentioned_set: BTreeSet<usize> = BTreeSet::new();
    for item in eq_items {
        let f = item.filter();
        match f {
            DatasetFilter::Any => {}
            DatasetFilter::Include(v) | DatasetFilter::Exclude(v) => {
                mentioned_set.extend(v.iter().copied());
                if !non_any_filters.contains(&f) {
                    non_any_filters.push(f);
                }
            }
        }
    }
    let mentioned: Vec<usize> = mentioned_set.into_iter().collect();
    let pos_by_ds: HashMap<usize, usize> = mentioned.iter().enumerate().map(|(pos, &ds)| (ds, pos)).collect();
    let outside_rep = (0..=mentioned.len()).find(|ds| !pos_by_ds.contains_key(ds)).unwrap_or(0);
    let outside_filter = if mentioned.is_empty() { DatasetFilter::Any } else { DatasetFilter::Exclude(mentioned.clone()) };
    if mentioned.is_empty() {
        return (Vec::new(), outside_filter, outside_rep);
    }

    let words = non_any_filters.len().div_ceil(64);
    let mut exclude_default = vec![0_u64; words];
    for (fi, f) in non_any_filters.iter().enumerate() {
        if matches!(f, DatasetFilter::Exclude(_)) {
            exclude_default[fi / 64] |= 1_u64 << (fi % 64);
        }
    }
    let mut sigs = vec![exclude_default; mentioned.len()];
    for (fi, f) in non_any_filters.iter().enumerate() {
        let word = fi / 64;
        let mask = 1_u64 << (fi % 64);
        match f {
            DatasetFilter::Include(v) => {
                for &ds in v {
                    if let Some(&pos) = pos_by_ds.get(&ds) {
                        sigs[pos][word] |= mask;
                    }
                }
            }
            DatasetFilter::Exclude(v) => {
                for &ds in v {
                    if let Some(&pos) = pos_by_ds.get(&ds) {
                        sigs[pos][word] &= !mask;
                    }
                }
            }
            DatasetFilter::Any => {}
        }
    }

    let mut groups: BTreeMap<Vec<u64>, Vec<usize>> = BTreeMap::new();
    for (pos, &ds) in mentioned.iter().enumerate() {
        groups.entry(sigs[pos].clone()).or_default().push(ds);
    }
    let mut classes: Vec<(DatasetFilter, usize)> =
        groups.into_values().map(|indices| (DatasetFilter::Include(indices.clone()), indices[0])).collect();
    classes.sort_by_key(|(_, rep)| *rep);
    (classes, outside_filter, outside_rep)
}

fn target_for_kind(item: &EqItem, kind: DslEquationKind) -> Option<(Expr, bool)> {
    let was_any = *item.filter() == DatasetFilter::Any;
    match (kind, item) {
        (DslEquationKind::OdeFirstOrder, EqItem::Ode { rhs, .. }) => Some((rhs.clone(), was_any)),
        (DslEquationKind::Implicit, EqItem::Implicit { diff, .. }) => Some((diff.clone(), was_any)),
        (DslEquationKind::Implicit, EqItem::Explicit { expr, .. }) => {
            Some((Expr::Binary { op: BinaryOp::Sub, lhs: Box::new(Expr::Ident("Y".to_owned())), rhs: Box::new(expr.clone()) }, was_any))
        }
        (DslEquationKind::Explicit, EqItem::Explicit { expr, .. }) => Some((expr.clone(), was_any)),
        _ => None,
    }
}

fn branch_uses_unbound_scoped(locals: &[(String, Expr)], target: &Expr, scoped_names: &BTreeSet<String>) -> bool {
    let mut free = Vec::new();
    let mut seen = BTreeSet::new();
    let mut bound = BTreeSet::new();
    for (lname, rhs) in locals {
        collect_free_idents(rhs, &bound, &mut seen, &mut free);
        bound.insert(lname.clone());
    }
    collect_free_idents(target, &bound, &mut seen, &mut free);
    free.iter().any(|ident| scoped_names.contains(ident))
}

fn collect_branches_for_rep(eq_items: &[EqItem], kind: DslEquationKind, class_filter: &DatasetFilter, rep_ds: usize) -> Vec<(OutputBranch, bool)> {
    let mut active_locals: Vec<(String, Expr)> = Vec::new();
    let mut out = Vec::new();
    for item in eq_items {
        if !item.filter().matches(rep_ds) {
            continue;
        }
        if let EqItem::Local { name, rhs, .. } = item {
            active_locals.push((name.clone(), rhs.clone()));
            continue;
        }
        if let Some((target, was_any)) = target_for_kind(item, kind) {
            out.push(((class_filter.clone(), active_locals.clone(), target), was_any));
        }
    }
    out
}

struct ResolvedEquationBody {
    kind: DslEquationKind,
    body_stmts: Vec<OutputBranch>,
    init_stmts: Vec<(usize, Expr)>,
    ode_t0: f64,
}

fn resolve_equation_branches(parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>, ode_t0: f64) -> Result<ResolvedEquationBody, FitError> {
    let (kind, eq_items, init_stmts, ode_t0) = build_eq_items(parsed_lines, ode_t0);
    let has_ode_output = eq_items.iter().any(|it| matches!(it, EqItem::Ode { .. }));
    if kind == DslEquationKind::OdeFirstOrder && !has_ode_output {
        return Err(FitError::DslCompileError { message: "Missing ODE derivative expression".to_owned(), line: 1, column: 1 });
    }
    let scoped_local_names: BTreeSet<String> = eq_items
        .iter()
        .filter_map(|it| match it {
            EqItem::Local { filter, name, .. } if *filter != DatasetFilter::Any => Some(name.clone()),
            _ => None,
        })
        .collect();

    let (explicit_classes, outside_filter, outside_rep) = collect_filter_classes(&eq_items);
    let mut body_stmts = Vec::new();
    for (class_filter, rep_ds) in &explicit_classes {
        for (branch, _) in collect_branches_for_rep(&eq_items, kind, class_filter, *rep_ds) {
            body_stmts.push(branch);
        }
    }
    let any_explicit_branch = !body_stmts.is_empty();
    for (branch, was_any) in collect_branches_for_rep(&eq_items, kind, &outside_filter, outside_rep) {
        if any_explicit_branch && was_any && branch_uses_unbound_scoped(&branch.1, &branch.2, &scoped_local_names) {
            continue;
        }
        body_stmts.push(branch);
    }
    Ok(ResolvedEquationBody { kind, body_stmts, init_stmts, ode_t0 })
}

fn compute_dataset_stats(points: &[DataPoint]) -> [f64; 11] {
    let mut finite_pts: Vec<(f64, f64)> = points.iter().filter(|p| p.x0().is_finite() && p.y.is_finite()).map(|p| (p.x0(), p.y)).collect();
    if finite_pts.is_empty() {
        return [0.0, 1.0, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, 0.5, 1.0, 1.0];
    }
    finite_pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let xmin = finite_pts[0].0;
    let xmax = finite_pts[finite_pts.len() - 1].0;
    let mut ymin = finite_pts[0].1;
    let mut ymax = finite_pts[0].1;
    let mut x_at_ymin = finite_pts[0].0;
    let mut x_at_ymax = finite_pts[0].0;
    for &(x, y) in &finite_pts {
        if y < ymin {
            ymin = y;
            x_at_ymin = x;
        }
        if y > ymax {
            ymax = y;
            x_at_ymax = x;
        }
    }
    let eps = 1e-12 * (xmax - xmin).max(1e-12);
    let left_vals: Vec<f64> = finite_pts.iter().filter(|&&(x, _)| (x - xmin).abs() <= eps).map(|&(_, y)| y).collect();
    let right_vals: Vec<f64> = finite_pts.iter().filter(|&&(x, _)| (xmax - x).abs() <= eps).map(|&(_, y)| y).collect();
    let y_left = if left_vals.is_empty() { finite_pts[0].1 } else { left_vals.iter().sum::<f64>() / left_vals.len() as f64 };
    let y_right = if right_vals.is_empty() { finite_pts[finite_pts.len() - 1].1 } else { right_vals.iter().sum::<f64>() / right_vals.len() as f64 };
    let xmid = 0.5 * (xmin + xmax);
    let ymid = 0.5 * (ymin + ymax);
    let span = ymax - ymin;
    let sign = if y_right >= y_left { 1.0 } else { -1.0 };
    let est_50 = estimate_x_at_ymid(points);
    let x_at_y50 = if est_50.is_finite() { est_50 } else { xmid };
    [ymin, ymax, xmin, xmax, xmid, ymid, x_at_ymin, x_at_ymax, x_at_y50, span, sign]
}

fn canonicalize_init_expr(expr: &mut Expr, shadowed: &BTreeSet<String>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                if !shadowed.contains(name.as_str())
                    && let Some(&canon) = INIT_STAT_NAMES.iter().find(|&&s| s.eq_ignore_ascii_case(name))
                {
                    *name = canon.to_owned();
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs.as_mut()),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(lhs.as_mut());
                stack.push(rhs.as_mut());
            }
            Expr::Call { args, .. } => {
                for arg in args {
                    stack.push(arg);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(cond.as_mut());
                stack.push(then_br.as_mut());
                stack.push(else_br.as_mut());
            }
        }
    }
}

fn with_line(mut err: FitError, line_no: usize) -> FitError {
    if let FitError::DslCompileError { line, .. } = &mut err
        && *line <= 1
    {
        *line = line_no;
    }
    err
}

fn ensure_no_data_vars(prog: &CompiledProgram, section_name: &str, line_no: usize) -> Result<(), FitError> {
    if prog.code.iter().any(|op| matches!(op, OpCode::LoadPredictor(_) | OpCode::LoadY)) {
        return Err(FitError::DslCompileError { message: format!("{section_name} cannot reference X or Y"), line: line_no, column: 1 });
    }
    Ok(())
}

fn compile_initial_rules(
    raw_init_lines: Vec<(usize, DatasetFilter, String, Expr)>,
) -> Result<Vec<(DatasetFilter, String, CompiledProgram)>, FitError> {
    if raw_init_lines.is_empty() {
        return Ok(Vec::new());
    }
    let stat_map: HashMap<String, usize> = INIT_STAT_NAMES.iter().enumerate().map(|(i, &n)| (n.to_owned(), i)).collect();
    let mut history: Vec<(DatasetFilter, String, Expr)> = Vec::new();
    let mut compiled = Vec::with_capacity(raw_init_lines.len());
    for (line_no, filter, name, mut rhs) in raw_init_lines {
        let mut prior_locals = Vec::new();
        let mut shadowed = BTreeSet::new();
        for (prev_filter, prev_name, prev_rhs) in &history {
            if *prev_filter == DatasetFilter::Any || *prev_filter == filter {
                prior_locals.push((prev_name.clone(), prev_rhs.clone()));
                shadowed.insert(prev_name.clone());
            }
        }
        canonicalize_init_expr(&mut rhs, &shadowed);
        let prog = compile_program(&prior_locals, &rhs, &stat_map).map_err(|e| with_line(e, line_no))?;
        ensure_no_data_vars(&prog, "[InitialValues]", line_no)?;
        compiled.push((filter.clone(), name.clone(), prog));
        history.push((filter, name, rhs));
    }
    Ok(compiled)
}

fn compile_transforms(
    raw_transform_lines: Vec<(usize, String, Expr)>,
    param_map: &HashMap<String, usize>,
) -> Result<Vec<(String, CompiledProgram)>, FitError> {
    if raw_transform_lines.is_empty() {
        return Ok(Vec::new());
    }
    let mut prior_locals: Vec<(String, Expr)> = Vec::new();
    let mut compiled = Vec::with_capacity(raw_transform_lines.len());
    for (line_no, name, rhs) in raw_transform_lines {
        let prog = compile_program(&prior_locals, &rhs, param_map).map_err(|e| with_line(e, line_no))?;
        ensure_no_data_vars(&prog, "[Transforms]", line_no)?;
        prior_locals.push((name.clone(), rhs));
        compiled.push((name, prog));
    }
    Ok(compiled)
}

fn expr_references_y(expr: &Expr) -> bool {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(n) => {
                if n.eq_ignore_ascii_case("Y") {
                    return true;
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => {
                for a in args {
                    stack.push(a);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
    false
}

fn is_builtin_constant_or_var(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    if matches!(u.as_str(), "X" | "T" | "Y" | "DS" | "DATASET" | "PI" | "E" | "LN2" | "LN10" | "SQRT2" | "INF" | "NAN") {
        return true;
    }
    if let Some(rest) = u.strip_prefix('X')
        && !rest.is_empty()
        && rest != "0"
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    false
}

fn collect_free_idents(expr: &Expr, bound_locals: &BTreeSet<String>, seen: &mut BTreeSet<String>, order: &mut Vec<String>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                if !is_builtin_constant_or_var(name) && !bound_locals.contains(name) && !seen.contains(name) {
                    seen.insert(name.clone());
                    order.push(name.clone());
                }
            }
            Expr::Unary { rhs, .. } => stack.push(rhs),
            Expr::Binary { lhs, rhs, .. } => {
                stack.push(rhs);
                stack.push(lhs);
            }
            Expr::Call { args, .. } => {
                for a in args.iter().rev() {
                    stack.push(a);
                }
            }
            Expr::If { cond, then_br, else_br } => {
                stack.push(else_br);
                stack.push(then_br);
                stack.push(cond);
            }
        }
    }
}

fn tokenize(input: &str, line: usize) -> Result<Vec<Token>, FitError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let col = i + 1;
        if ch.is_ascii_digit() || (ch == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                let save = i;
                i += 1;
                if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                if i < bytes.len() && bytes[i].is_ascii_digit() {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let slice = &input[start..i];
            let num = slice.parse::<f64>().map_err(|_| FitError::DslCompileError {
                message: format!("Invalid numeric literal `{slice}`"),
                line,
                column: col,
            })?;
            tokens.push(Token { kind: TokenKind::Number, text: slice.to_owned(), num, line, col });
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(Token { kind: TokenKind::Ident, text: input[start..i].to_owned(), num: 0.0, line, col });
            continue;
        }
        let next = bytes.get(i + 1).copied().unwrap_or(0);
        let (kind, len) = match (ch, next) {
            (b'=', b'=') => (TokenKind::EqEq, 2),
            (b'!', b'=') => (TokenKind::NotEq, 2),
            (b'<', b'>') => (TokenKind::NotEq, 2),
            (b'<', b'=') => (TokenKind::Le, 2),
            (b'>', b'=') => (TokenKind::Ge, 2),
            (b'&', b'&') => (TokenKind::AndAnd, 2),
            (b'|', b'|') => (TokenKind::OrOr, 2),
            (b'*', b'*') => (TokenKind::Caret, 2),
            (b'+', _) => (TokenKind::Plus, 1),
            (b'-', _) => (TokenKind::Minus, 1),
            (b'*', _) => (TokenKind::Star, 1),
            (b'/', _) => (TokenKind::Slash, 1),
            (b'%', _) => (TokenKind::Percent, 1),
            (b'^', _) => (TokenKind::Caret, 1),
            (b'(', _) => (TokenKind::LParen, 1),
            (b')', _) => (TokenKind::RParen, 1),
            (b',', _) => (TokenKind::Comma, 1),
            (b'=', _) => (TokenKind::Eq, 1),
            (b'<', _) => (TokenKind::Lt, 1),
            (b'>', _) => (TokenKind::Gt, 1),
            (b'!', _) => (TokenKind::Bang, 1),
            _ => {
                return Err(FitError::DslCompileError { message: format!("Unexpected character `{}`", ch as char), line, column: col });
            }
        };
        tokens.push(Token { kind, text: input[i..i + len].to_owned(), num: 0.0, line, col });
        i += len;
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    line: usize,
    depth: usize,
    nodes: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos)?.clone();
        self.pos += 1;
        Some(t)
    }
    fn track_node(&mut self, line: usize, col: usize) -> Result<(), FitError> {
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > MAX_AST_NODES {
            return Err(FitError::DslCompileError { message: format!("Expression AST exceeds maximum nodes ({MAX_AST_NODES})"), line, column: col });
        }
        Ok(())
    }
    fn enter_depth(&mut self, line: usize, col: usize) -> Result<(), FitError> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(FitError::DslCompileError {
                message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                line,
                column: col,
            });
        }
        self.depth += 1;
        Ok(())
    }
    fn leave_depth(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
    fn parse_or(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_and()?;
        while let Some((tok_line, tok_col)) = self.peek().filter(|t| t.kind == TokenKind::OrOr).map(|t| (t.line, t.col)) {
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_and()?;
            lhs = Expr::Binary { op: BinaryOp::Or, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    fn parse_and(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_cmp()?;
        while let Some((tok_line, tok_col)) = self.peek().filter(|t| t.kind == TokenKind::AndAnd).map(|t| (t.line, t.col)) {
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_cmp()?;
            lhs = Expr::Binary { op: BinaryOp::And, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    fn parse_cmp(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_add()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::EqEq | TokenKind::Eq => BinaryOp::Eq,
                TokenKind::NotEq => BinaryOp::Ne,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Le => BinaryOp::Le,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Ge => BinaryOp::Ge,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_add()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    fn parse_add(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_mul()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_mul()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    fn parse_mul(&mut self) -> Result<Expr, FitError> {
        let mut lhs = self.parse_pow()?;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col) = (tok.line, tok.col);
            let op = match tok.kind {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => break,
            };
            self.bump();
            self.track_node(tok_line, tok_col)?;
            let rhs = self.parse_pow()?;
            lhs = Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) };
        }
        Ok(lhs)
    }
    fn parse_pow(&mut self) -> Result<Expr, FitError> {
        let mut factors = vec![self.parse_unary()?];
        while let Some(tok) = self.peek() {
            if tok.kind != TokenKind::Caret {
                break;
            }
            let (tok_line, tok_col) = (tok.line, tok.col);
            self.bump();
            if self.depth + factors.len() > MAX_PARSE_DEPTH {
                return Err(FitError::DslCompileError {
                    message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                    line: tok_line,
                    column: tok_col,
                });
            }
            self.track_node(tok_line, tok_col)?;
            factors.push(self.parse_unary()?);
        }
        let mut acc = factors.pop().ok_or_else(|| FitError::DslCompileError {
            message: "Unexpected end of expression".to_owned(),
            line: self.line,
            column: 1,
        })?;
        while let Some(base) = factors.pop() {
            acc = Expr::Binary { op: BinaryOp::Pow, lhs: Box::new(base), rhs: Box::new(acc) };
        }
        Ok(acc)
    }
    fn parse_unary(&mut self) -> Result<Expr, FitError> {
        let mut ops: Vec<UnaryOp> = Vec::new();
        let mut chain_len = 0_usize;
        while let Some(tok) = self.peek() {
            let (tok_line, tok_col, kind) = (tok.line, tok.col, tok.kind);
            match kind {
                TokenKind::Plus => {
                    self.bump();
                }
                TokenKind::Minus => {
                    self.bump();
                    self.track_node(tok_line, tok_col)?;
                    ops.push(UnaryOp::Neg);
                }
                TokenKind::Bang => {
                    self.bump();
                    self.track_node(tok_line, tok_col)?;
                    ops.push(UnaryOp::Not);
                }
                _ => break,
            }
            chain_len += 1;
            if self.depth + chain_len > MAX_PARSE_DEPTH {
                return Err(FitError::DslCompileError {
                    message: format!("Expression nesting exceeds maximum depth ({MAX_PARSE_DEPTH})"),
                    line: tok_line,
                    column: tok_col,
                });
            }
        }
        let mut expr = self.parse_primary()?;
        while let Some(op) = ops.pop() {
            expr = Expr::Unary { op, rhs: Box::new(expr) };
        }
        Ok(expr)
    }
    fn parse_call_args(&mut self, tok: &Token) -> Result<Vec<Expr>, FitError> {
        self.bump();
        let mut args = Vec::new();
        if !self.peek().is_some_and(|t| t.kind == TokenKind::RParen) {
            loop {
                args.push(self.parse_or()?);
                if self.peek().is_some_and(|t| t.kind == TokenKind::Comma) {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        if !self.bump().is_some_and(|t| t.kind == TokenKind::RParen) {
            return Err(FitError::DslCompileError { message: format!("Expected `)` after call to `{}`", tok.text), line: tok.line, column: tok.col });
        }
        Ok(args)
    }
    fn build_call_expr(tok: Token, args: Vec<Expr>) -> Result<Expr, FitError> {
        let upper = tok.text.to_ascii_uppercase();
        if upper == "IF" {
            if args.len() != 3 {
                return Err(FitError::DslCompileError {
                    message: "`IF(cond, then, else)` requires 3 arguments".to_owned(),
                    line: tok.line,
                    column: tok.col,
                });
            }
            let mut it = args.into_iter();
            if let (Some(c), Some(t_br), Some(e_br)) = (it.next(), it.next(), it.next()) {
                Ok(Expr::If { cond: Box::new(c), then_br: Box::new(t_br), else_br: Box::new(e_br) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid IF arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "AND" && args.len() == 2 {
            let mut it = args.into_iter();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                Ok(Expr::Binary { op: BinaryOp::And, lhs: Box::new(a), rhs: Box::new(b) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid AND arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "OR" && args.len() == 2 {
            let mut it = args.into_iter();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                Ok(Expr::Binary { op: BinaryOp::Or, lhs: Box::new(a), rhs: Box::new(b) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid OR arguments".to_owned(), line: tok.line, column: tok.col })
            }
        } else if upper == "NOT" && args.len() == 1 {
            if let Some(a) = args.into_iter().next() {
                Ok(Expr::Unary { op: UnaryOp::Not, rhs: Box::new(a) })
            } else {
                Err(FitError::DslCompileError { message: "Invalid NOT argument".to_owned(), line: tok.line, column: tok.col })
            }
        } else {
            Ok(Expr::Call { name: tok.text, args })
        }
    }
    fn parse_primary(&mut self) -> Result<Expr, FitError> {
        let Some(tok) = self.bump() else {
            return Err(FitError::DslCompileError { message: "Unexpected end of expression".to_owned(), line: self.line, column: 1 });
        };
        match tok.kind {
            TokenKind::Number => {
                self.track_node(tok.line, tok.col)?;
                Ok(Expr::Num(tok.num))
            }
            TokenKind::LParen => {
                self.enter_depth(tok.line, tok.col)?;
                let inner = self.parse_or();
                self.leave_depth();
                let inner = inner?;
                match self.bump() {
                    Some(t) if t.kind == TokenKind::RParen => Ok(inner),
                    _ => Err(FitError::DslCompileError { message: "Expected `)`".to_owned(), line: tok.line, column: tok.col }),
                }
            }
            TokenKind::Ident => {
                if self.peek().is_some_and(|t| t.kind == TokenKind::LParen) {
                    self.track_node(tok.line, tok.col)?;
                    self.enter_depth(tok.line, tok.col)?;
                    let args = self.parse_call_args(&tok);
                    self.leave_depth();
                    Self::build_call_expr(tok, args?)
                } else {
                    self.track_node(tok.line, tok.col)?;
                    Ok(Expr::Ident(tok.text))
                }
            }
            _ => Err(FitError::DslCompileError { message: format!("Unexpected token `{}`", tok.text), line: tok.line, column: tok.col }),
        }
    }
}

fn parse_expr(input: &str, line: usize) -> Result<Expr, FitError> {
    let tokens = tokenize(input, line)?;
    let mut parser = Parser { tokens: &tokens, pos: 0, line, depth: 0, nodes: 0 };
    let expr = parser.parse_or()?;
    if let Some(rem) = parser.peek() {
        return Err(FitError::DslCompileError { message: format!("Unexpected trailing token `{}`", rem.text), line: rem.line, column: rem.col });
    }
    Ok(expr)
}

fn compile_program(locals: &[(String, Expr)], target: &Expr, params: &HashMap<String, usize>) -> Result<CompiledProgram, FitError> {
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut code = Vec::new();
    for (lname, rhs) in locals {
        compile_expr(rhs, &local_map, params, &mut code)?;
        let slot = if let Some(&s) = local_map.get(lname) {
            s
        } else {
            let s = local_map.len();
            local_map.insert(lname.clone(), s);
            s
        };
        code.push(OpCode::StoreLocal(slot));
    }
    compile_expr(target, &local_map, params, &mut code)?;
    code.push(OpCode::Halt);
    Ok(CompiledProgram { code, local_count: local_map.len() })
}

enum CompileTask<'a> {
    Eval(&'a Expr),
    EmitUnary(UnaryKind),
    EmitBinary(BinaryKind),
    EmitCall(BuiltinFn, usize),
    AndAfterLhs { rhs: &'a Expr },
    AndAfterRhs { jz_idx: usize },
    OrAfterLhs { rhs: &'a Expr },
    OrAfterRhs { jmp_end: usize },
    IfAfterCond { then_br: &'a Expr, else_br: &'a Expr },
    IfAfterThen { else_br: &'a Expr, jz_idx: usize },
    IfAfterElse { jmp_end: usize },
}

fn binary_op_to_kind(op: BinaryOp) -> BinaryKind {
    match op {
        BinaryOp::Add => BinaryKind::Add,
        BinaryOp::Sub => BinaryKind::Sub,
        BinaryOp::Mul => BinaryKind::Mul,
        BinaryOp::Div => BinaryKind::Div,
        BinaryOp::Rem => BinaryKind::Rem,
        BinaryOp::Pow => BinaryKind::Pow,
        BinaryOp::Eq => BinaryKind::Eq,
        BinaryOp::Ne => BinaryKind::Ne,
        BinaryOp::Lt => BinaryKind::Lt,
        BinaryOp::Le => BinaryKind::Le,
        BinaryOp::Gt => BinaryKind::Gt,
        BinaryOp::Ge => BinaryKind::Ge,
        BinaryOp::And | BinaryOp::Or => BinaryKind::Eq,
    }
}

fn schedule_expr_compile<'a>(
    node: &'a Expr,
    locals: &HashMap<String, usize>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
    tasks: &mut Vec<CompileTask<'a>>,
) -> Result<(), FitError> {
    match node {
        Expr::Num(v) => code.push(OpCode::PushConst(*v)),
        Expr::Ident(name) => compile_ident(name, locals, params, code)?,
        Expr::Unary { op, rhs } => {
            let kind = match op {
                UnaryOp::Neg => UnaryKind::Neg,
                UnaryOp::Not => UnaryKind::Not,
            };
            tasks.push(CompileTask::EmitUnary(kind));
            tasks.push(CompileTask::Eval(rhs));
        }
        Expr::Binary { op, lhs, rhs } => match op {
            BinaryOp::And => {
                tasks.push(CompileTask::AndAfterLhs { rhs });
                tasks.push(CompileTask::Eval(lhs));
            }
            BinaryOp::Or => {
                tasks.push(CompileTask::OrAfterLhs { rhs });
                tasks.push(CompileTask::Eval(lhs));
            }
            _ => {
                tasks.push(CompileTask::EmitBinary(binary_op_to_kind(*op)));
                tasks.push(CompileTask::Eval(rhs));
                tasks.push(CompileTask::Eval(lhs));
            }
        },
        Expr::If { cond, then_br, else_br } => {
            tasks.push(CompileTask::IfAfterCond { then_br, else_br });
            tasks.push(CompileTask::Eval(cond));
        }
        Expr::Call { name, args } => {
            let (bf, expected_arity) = resolve_builtin_fn(name)?;
            if args.len() != expected_arity {
                return Err(FitError::DslCompileError {
                    message: format!("Function `{name}` expects {expected_arity} argument(s), got {}", args.len()),
                    line: 1,
                    column: 1,
                });
            }
            tasks.push(CompileTask::EmitCall(bf, args.len()));
            for a in args.iter().rev() {
                tasks.push(CompileTask::Eval(a));
            }
        }
    }
    Ok(())
}

fn compile_expr(expr: &Expr, locals: &HashMap<String, usize>, params: &HashMap<String, usize>, code: &mut Vec<OpCode>) -> Result<(), FitError> {
    let mut tasks = vec![CompileTask::Eval(expr)];
    while let Some(task) = tasks.pop() {
        match task {
            CompileTask::Eval(node) => schedule_expr_compile(node, locals, params, code, &mut tasks)?,
            CompileTask::EmitUnary(kind) => code.push(OpCode::Unary(kind)),
            CompileTask::EmitBinary(kind) => code.push(OpCode::Binary(kind)),
            CompileTask::EmitCall(bf, arity) => code.push(OpCode::CallBuiltin(bf, arity)),
            CompileTask::AndAfterLhs { rhs } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                tasks.push(CompileTask::AndAfterRhs { jz_idx });
                tasks.push(CompileTask::Eval(rhs));
            }
            CompileTask::AndAfterRhs { jz_idx } => {
                code.push(OpCode::PushConst(0.0));
                code.push(OpCode::Binary(BinaryKind::Ne));
                let jmp_end = code.len();
                code.push(OpCode::Jump(0));
                let false_target = code.len();
                code.push(OpCode::PushConst(0.0));
                let end_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(false_target);
                code[jmp_end] = OpCode::Jump(end_target);
            }
            CompileTask::OrAfterLhs { rhs } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                code.push(OpCode::PushConst(1.0));
                let jmp_end = code.len();
                code.push(OpCode::Jump(0));
                let rhs_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(rhs_target);
                tasks.push(CompileTask::OrAfterRhs { jmp_end });
                tasks.push(CompileTask::Eval(rhs));
            }
            CompileTask::OrAfterRhs { jmp_end } => {
                code.push(OpCode::PushConst(0.0));
                code.push(OpCode::Binary(BinaryKind::Ne));
                let end_target = code.len();
                code[jmp_end] = OpCode::Jump(end_target);
            }
            CompileTask::IfAfterCond { then_br, else_br } => {
                let jz_idx = code.len();
                code.push(OpCode::JumpIfZero(0));
                tasks.push(CompileTask::IfAfterThen { else_br, jz_idx });
                tasks.push(CompileTask::Eval(then_br));
            }
            CompileTask::IfAfterThen { else_br, jz_idx } => {
                let jmp_end = code.len();
                code.push(OpCode::Jump(0));
                let else_target = code.len();
                code[jz_idx] = OpCode::JumpIfZero(else_target);
                tasks.push(CompileTask::IfAfterElse { jmp_end });
                tasks.push(CompileTask::Eval(else_br));
            }
            CompileTask::IfAfterElse { jmp_end } => {
                let end_target = code.len();
                code[jmp_end] = OpCode::Jump(end_target);
            }
        }
    }
    Ok(())
}

fn compile_ident(name: &str, locals: &HashMap<String, usize>, params: &HashMap<String, usize>, code: &mut Vec<OpCode>) -> Result<(), FitError> {
    if let Some(&slot) = locals.get(name) {
        code.push(OpCode::LoadLocal(slot));
        return Ok(());
    }
    let u = name.to_ascii_uppercase();
    match u.as_str() {
        "X" | "T" => code.push(OpCode::LoadPredictor(0)),
        "Y" => code.push(OpCode::LoadY),
        "DS" | "DATASET" => code.push(OpCode::LoadDatasetIndex),
        "PI" => code.push(OpCode::PushConst(core::f64::consts::PI)),
        "E" => code.push(OpCode::PushConst(core::f64::consts::E)),
        "LN2" => code.push(OpCode::PushConst(core::f64::consts::LN_2)),
        "LN10" => code.push(OpCode::PushConst(core::f64::consts::LN_10)),
        "SQRT2" => code.push(OpCode::PushConst(core::f64::consts::SQRT_2)),
        "INF" => code.push(OpCode::PushConst(f64::INFINITY)),
        "NAN" => code.push(OpCode::PushConst(f64::NAN)),
        _ => {
            if let Some(rest) = u.strip_prefix('X')
                && !rest.is_empty()
                && rest != "0"
                && let Ok(idx) = rest.parse::<usize>()
                && idx >= 1
            {
                code.push(OpCode::LoadPredictor(idx - 1));
                return Ok(());
            }
            if let Some(&pidx) = params.get(name) {
                code.push(OpCode::LoadParam(pidx));
                return Ok(());
            }
            return Err(FitError::DslCompileError { message: format!("Unknown symbol `{name}`"), line: 1, column: 1 });
        }
    }
    Ok(())
}

fn resolve_builtin_fn(name: &str) -> Result<(BuiltinFn, usize), FitError> {
    let u = name.to_ascii_lowercase();
    let res = match u.as_str() {
        "abs" => (BuiltinFn::Abs, 1),
        "sqrt" => (BuiltinFn::Sqrt, 1),
        "cbrt" => (BuiltinFn::Cbrt, 1),
        "exp" => (BuiltinFn::Exp, 1),
        "exp10" => (BuiltinFn::Exp10, 1),
        "ln" | "loge" => (BuiltinFn::Ln, 1),
        "log" | "log10" => (BuiltinFn::Log10, 1),
        "log2" => (BuiltinFn::Log2, 1),
        "sin" => (BuiltinFn::Sin, 1),
        "cos" => (BuiltinFn::Cos, 1),
        "tan" => (BuiltinFn::Tan, 1),
        "asin" => (BuiltinFn::Asin, 1),
        "acos" => (BuiltinFn::Acos, 1),
        "atan" | "arctan" => (BuiltinFn::Atan, 1),
        "atan2" => (BuiltinFn::Atan2, 2),
        "sinh" => (BuiltinFn::Sinh, 1),
        "cosh" => (BuiltinFn::Cosh, 1),
        "tanh" => (BuiltinFn::Tanh, 1),
        "asinh" => (BuiltinFn::Asinh, 1),
        "acosh" => (BuiltinFn::Acosh, 1),
        "atanh" => (BuiltinFn::Atanh, 1),
        "floor" => (BuiltinFn::Floor, 1),
        "ceil" => (BuiltinFn::Ceil, 1),
        "round" => (BuiltinFn::Round, 1),
        "trunc" => (BuiltinFn::Trunc, 1),
        "sign" | "sgn" => (BuiltinFn::Sign, 1),
        "min" => (BuiltinFn::Min, 2),
        "max" => (BuiltinFn::Max, 2),
        "clamp" => (BuiltinFn::Clamp, 3),
        "hypot" => (BuiltinFn::Hypot, 2),
        "pow" => (BuiltinFn::PowFn, 2),
        "erf" => (BuiltinFn::Erf, 1),
        "erfc" => (BuiltinFn::Erfc, 1),
        "normcdf" | "phi" => (BuiltinFn::NormCdf, 1),
        "norminv" | "probit" => (BuiltinFn::NormInv, 1),
        "gamma" => (BuiltinFn::Gamma, 1),
        "lngamma" | "lgamma" => (BuiltinFn::LnGamma, 1),
        "beta" => (BuiltinFn::Beta, 2),
        "digamma" | "psi" => (BuiltinFn::Digamma, 1),
        "besselj0" | "j0" => (BuiltinFn::BesselJ0, 1),
        "besselj1" | "j1" => (BuiltinFn::BesselJ1, 1),
        "besseljn" | "jn" => (BuiltinFn::BesselJn, 2),
        "bessely0" | "y0" => (BuiltinFn::BesselY0, 1),
        "bessely1" | "y1" => (BuiltinFn::BesselY1, 1),
        "besselyn" | "yn" => (BuiltinFn::BesselYn, 2),
        "besseli0" | "i0" => (BuiltinFn::BesselI0, 1),
        "besseli1" | "i1" => (BuiltinFn::BesselI1, 1),
        "besselk0" | "k0" => (BuiltinFn::BesselK0, 1),
        "besselk1" | "k1" => (BuiltinFn::BesselK1, 1),
        "hyp1f1" | "m" => (BuiltinFn::Hyp1F1, 3),
        "hyp2f1" => (BuiltinFn::Hyp2F1, 4),
        "confluent_u" | "u" => (BuiltinFn::ConfluentU, 3),
        _ => {
            return Err(FitError::DslCompileError { message: format!("Unknown function `{name}`"), line: 1, column: 1 });
        }
    };
    Ok(res)
}

// ============================================================================
// Stack VM Evaluator
// ============================================================================

fn eval_bytecode(prog: &CompiledProgram, x: &[f64], y: f64, params: &[f64], ds_idx: usize) -> Option<f64> {
    let mut stack: Vec<f64> = Vec::with_capacity(32);
    let mut locals = vec![0.0_f64; prog.local_count];
    let mut ip = 0_usize;
    let mut steps = 0_usize;

    while ip < prog.code.len() {
        steps += 1;
        if steps > MAX_VM_STEPS {
            return None;
        }
        match prog.code[ip] {
            OpCode::PushConst(v) => stack.push(v),
            OpCode::LoadPredictor(idx) => {
                stack.push(x.get(idx).copied().unwrap_or(0.0));
            }
            OpCode::LoadY => stack.push(y),
            OpCode::LoadDatasetIndex => stack.push(ds_idx as f64),
            OpCode::LoadParam(idx) => stack.push(*params.get(idx)?),
            OpCode::LoadLocal(slot) => stack.push(*locals.get(slot)?),
            OpCode::StoreLocal(slot) => {
                let v = stack.pop()?;
                *locals.get_mut(slot)? = v;
            }
            OpCode::Unary(kind) => {
                let v = stack.pop()?;
                stack.push(match kind {
                    UnaryKind::Neg => -v,
                    UnaryKind::Not => {
                        if v == 0.0 {
                            1.0
                        } else {
                            0.0
                        }
                    }
                });
            }
            OpCode::Binary(kind) => {
                let b = stack.pop()?;
                let a = stack.pop()?;
                let v = match kind {
                    BinaryKind::Add => a + b,
                    BinaryKind::Sub => a - b,
                    BinaryKind::Mul => a * b,
                    BinaryKind::Div => {
                        if b == 0.0 {
                            return None;
                        }
                        a / b
                    }
                    BinaryKind::Rem => a % b,
                    BinaryKind::Pow => a.powf(b),
                    BinaryKind::Eq => f64::from((a - b).abs() == 0.0),
                    BinaryKind::Ne => f64::from(a != b),
                    BinaryKind::Lt => f64::from(a < b),
                    BinaryKind::Le => f64::from(a <= b),
                    BinaryKind::Gt => f64::from(a > b),
                    BinaryKind::Ge => f64::from(a >= b),
                };
                stack.push(v);
            }
            OpCode::CallBuiltin(bf, arity) => {
                if stack.len() < arity {
                    return None;
                }
                let base = stack.len() - arity;
                let res = eval_builtin(bf, &stack[base..])?;
                stack.truncate(base);
                stack.push(res);
            }
            OpCode::Jump(target) => {
                ip = target;
                continue;
            }
            OpCode::JumpIfZero(target) => {
                let cond = stack.pop()?;
                if cond == 0.0 || !cond.is_finite() {
                    ip = target;
                    continue;
                }
            }
            OpCode::Pop => {
                stack.pop()?;
            }
            OpCode::Halt => break,
        }
        ip += 1;
    }
    let out = stack.pop()?;
    if out.is_finite() { Some(out) } else { None }
}

fn eval_builtin(bf: BuiltinFn, a: &[f64]) -> Option<f64> {
    let val = match bf {
        BuiltinFn::Abs => a[0].abs(),
        BuiltinFn::Sqrt => {
            if a[0] < 0.0 {
                return None;
            }
            a[0].sqrt()
        }
        BuiltinFn::Cbrt => a[0].cbrt(),
        BuiltinFn::Exp => {
            if a[0] > 709.0 {
                return None;
            }
            a[0].exp()
        }
        BuiltinFn::Exp10 => {
            if a[0] > 308.0 {
                return None;
            }
            10.0_f64.powf(a[0])
        }
        BuiltinFn::Ln => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].ln()
        }
        BuiltinFn::Log10 => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].log10()
        }
        BuiltinFn::Log2 => {
            if a[0] <= 0.0 {
                return None;
            }
            a[0].log2()
        }
        BuiltinFn::Sin => a[0].sin(),
        BuiltinFn::Cos => a[0].cos(),
        BuiltinFn::Tan => a[0].tan(),
        BuiltinFn::Asin => a[0].asin(),
        BuiltinFn::Acos => a[0].acos(),
        BuiltinFn::Atan => a[0].atan(),
        BuiltinFn::Atan2 => a[0].atan2(a[1]),
        BuiltinFn::Sinh => a[0].sinh(),
        BuiltinFn::Cosh => a[0].cosh(),
        BuiltinFn::Tanh => a[0].tanh(),
        BuiltinFn::Asinh => a[0].asinh(),
        BuiltinFn::Acosh => a[0].acosh(),
        BuiltinFn::Atanh => a[0].atanh(),
        BuiltinFn::Floor => a[0].floor(),
        BuiltinFn::Ceil => a[0].ceil(),
        BuiltinFn::Round => a[0].round(),
        BuiltinFn::Trunc => a[0].trunc(),
        BuiltinFn::Sign => {
            if a[0] > 0.0 {
                1.0
            } else if a[0] < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        BuiltinFn::Min => a[0].min(a[1]),
        BuiltinFn::Max => a[0].max(a[1]),
        BuiltinFn::Clamp => {
            if a[0].is_nan() || a[1].is_nan() || a[2].is_nan() {
                f64::NAN
            } else {
                let lo = a[1].min(a[2]);
                let hi = a[1].max(a[2]);
                if lo <= hi { a[0].clamp(lo, hi) } else { f64::NAN }
            }
        }
        BuiltinFn::Hypot => a[0].hypot(a[1]),
        BuiltinFn::PowFn => a[0].powf(a[1]),
        BuiltinFn::Erf => erf(a[0]),
        BuiltinFn::Erfc => erfc(a[0]),
        BuiltinFn::NormCdf => norm_cdf(a[0]),
        BuiltinFn::NormInv => norm_ppf(a[0]).ok()?,
        BuiltinFn::Gamma => gamma(a[0]).ok()?,
        BuiltinFn::LnGamma => ln_gamma(a[0]).ok()?,
        BuiltinFn::Beta => beta(a[0], a[1]).ok()?,
        BuiltinFn::Digamma => digamma(a[0]).ok()?,
        BuiltinFn::BesselJ0 => bessel_j0(a[0]),
        BuiltinFn::BesselJ1 => bessel_j1(a[0]),
        BuiltinFn::BesselJn => {
            let n = parse_bessel_order(a[0])?;
            bessel_jn(n, a[1])
        }
        BuiltinFn::BesselY0 => bessel_y0(a[0])?,
        BuiltinFn::BesselY1 => bessel_y1(a[0])?,
        BuiltinFn::BesselYn => {
            let n = parse_bessel_order(a[0])?;
            bessel_yn(n, a[1])?
        }
        BuiltinFn::BesselI0 => bessel_i0(a[0])?,
        BuiltinFn::BesselI1 => bessel_i1(a[0])?,
        BuiltinFn::BesselK0 => bessel_k0(a[0])?,
        BuiltinFn::BesselK1 => bessel_k1(a[0])?,
        BuiltinFn::Hyp1F1 => hyp1f1(a[0], a[1], a[2])?,
        BuiltinFn::Hyp2F1 => hyp2f1(a[0], a[1], a[2], a[3])?,
        BuiltinFn::ConfluentU => confluent_u(a[0], a[1], a[2])?,
    };
    if val.is_finite() { Some(val) } else { None }
}

// ============================================================================
// Pure-Rust Bessel & Hypergeometric Special Functions
// (J/Y: rational approximations as given in Press et al. 1992, Sec. 6.5;
//  I/K: Abramowitz & Stegun 1964, 9.8.1-9.8.8; hypergeometric: A&S 13.1 and 15.1)
// ============================================================================

fn parse_bessel_order(v: f64) -> Option<i32> {
    if !v.is_finite() {
        return None;
    }
    let rounded = v.round();
    if rounded.abs() > f64::from(MAX_BESSEL_ORDER) {
        return None;
    }
    Some(rounded as i32)
}

/// Bessel `J0(x)`: rational approximation for `|x| < 8` and the asymptotic phase/amplitude form
/// for `|x| >= 8`, with the coefficients given in Press et al. (1992), Sec. 6.5. Accuracy is
/// single-precision level (roughly `1e-8`), far below `f64` resolution.
fn bessel_j0(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let num =
            57_568_490_574.0 + y * (-13_362_590_354.0 + y * (651_619_640.7 + y * (-11_214_424.18 + y * (77_392.330_17 + y * (-184.905_245_6)))));
        let den = 57_568_490_411.0 + y * (1_029_532_985.0 + y * (9_494_680.718 + y * (59_272.648_53 + y * (267.853_271_2 + y))));
        num / den
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - core::f64::consts::FRAC_PI_4;
        let p = 1.0 + y * (-0.109_862_862_7e-2 + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let q = -0.156_249_999_5e-1 + y * (0.143_048_876_5e-3 + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 - y * 0.934_935_152e-7)));
        (core::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p - z * xx.sin() * q)
    }
}

/// Bessel `J1(x)`; same scheme and accuracy as `bessel_j0` (Press et al. 1992, Sec. 6.5).
fn bessel_j1(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    if ax < 8.0 {
        let y = x * x;
        let num =
            x * (72_362_614_232.0 + y * (-7_895_059_235.0 + y * (242_396_853.1 + y * (-2_972_611.439 + y * (15_704.482_60 + y * (-30.160_366_06))))));
        let den = 144_725_228_442.0 + y * (2_300_535_178.0 + y * (18_583_304.74 + y * (99_447.433_94 + y * (376.999_139_7 + y))));
        num / den
    } else {
        let z = 8.0 / ax;
        let y = z * z;
        let xx = ax - 2.356_194_490_192_345;
        let p = 1.0 + y * (0.183_105e-2 + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019e-6))));
        let q = 0.046_874_999_95 + y * (-0.200_269_087_3e-3 + y * (0.844_919_909_6e-5 + y * (-0.882_289_87e-6 + y * 0.105_787_412e-6)));
        let ans = (core::f64::consts::FRAC_2_PI / ax).sqrt() * (xx.cos() * p - z * xx.sin() * q);
        if x < 0.0 { -ans } else { ans }
    }
}

/// Integer-order `J_n(x)`: upward recurrence from `J0`, `J1` when `|x| > n`, otherwise Miller's
/// downward recurrence normalised by `J0 + 2 sum J_{2k} = 1` (Press et al. 1992, Sec. 6.5;
/// recurrence A&S 9.1.27). Inherits the single-precision accuracy of `bessel_j0`/`bessel_j1`.
fn bessel_jn(n: i32, x: f64) -> f64 {
    if n.unsigned_abs() > MAX_BESSEL_ORDER as u32 || !x.is_finite() {
        return f64::NAN;
    }
    if n < 0 {
        let pos = bessel_jn(-n, x);
        return if (-n) % 2 == 0 { pos } else { -pos };
    }
    if n == 0 {
        return bessel_j0(x);
    }
    if n == 1 {
        return bessel_j1(x);
    }
    if x == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let tox = 2.0 / ax;
    let ans = if ax > n as f64 {
        let mut bjm = bessel_j0(ax);
        let mut bj = bessel_j1(ax);
        for j in 1..n {
            let bjp = j as f64 * tox * bj - bjm;
            bjm = bj;
            bj = bjp;
        }
        bj
    } else {
        let m = 2 * ((n + (40.0 * n as f64).sqrt() as i32) / 2);
        let mut bjp = 0.0;
        let mut bj = 1.0;
        let mut sum = 0.0;
        let mut ans = 0.0;
        let mut jsum = false;
        for j in (1..=m).rev() {
            let bjm = j as f64 * tox * bj - bjp;
            bjp = bj;
            bj = bjm;
            if bj.abs() > 1e10 {
                bj *= 1e-10;
                bjp *= 1e-10;
                ans *= 1e-10;
                sum *= 1e-10;
            }
            if jsum {
                sum += bj;
            }
            jsum = !jsum;
            if j == n {
                ans = bjp;
            }
        }
        sum = 2.0 * sum - bj;
        ans / sum
    };
    if x < 0.0 && n % 2 != 0 { -ans } else { ans }
}

/// Bessel `Y0(x)` for `x > 0`; rational + `(2/pi) J0(x) ln x` for `x < 8`, asymptotic form above
/// (Press et al. 1992, Sec. 6.5). Single-precision level accuracy.
fn bessel_y0(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    let ans = if x < 8.0 {
        let y = x * x;
        let num = -2_957_821_389.0 + y * (7_062_834_065.0 + y * (-512_359_803.6 + y * (10_879_881.29 + y * (-86_327.927_57 + y * 228.462_273_3))));
        let den = 40_076_544_269.0 + y * (745_249_964.8 + y * (7_189_466.438 + y * (47_447.264_70 + y * (226.103_024_4 + y))));
        num / den + core::f64::consts::FRAC_2_PI * bessel_j0(x) * x.ln()
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - core::f64::consts::FRAC_PI_4;
        let p = 1.0 + y * (-0.109_862_862_7e-2 + y * (0.273_451_040_7e-4 + y * (-0.207_337_063_9e-5 + y * 0.209_388_721_1e-6)));
        let q = -0.156_249_999_5e-1 + y * (0.143_048_876_5e-3 + y * (-0.691_114_765_1e-5 + y * (0.762_109_516_1e-6 + y * (-0.934_945_152e-7))));
        (core::f64::consts::FRAC_2_PI / x).sqrt() * (xx.sin() * p + z * xx.cos() * q)
    };
    Some(ans)
}

/// Bessel `Y1(x)` for `x > 0`; same scheme as `bessel_y0` (Press et al. 1992, Sec. 6.5).
fn bessel_y1(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    let ans = if x < 8.0 {
        let y = x * x;
        let num = x
            * (-0.490_060_494_3e13
                + y * (0.127_527_439_0e13 + y * (-0.515_343_813_9e11 + y * (0.734_926_455_1e9 + y * (-0.423_792_272_6e7 + y * 0.851_193_793_5e4)))));
        let den = 0.249_958_057_0e14
            + y * (0.424_441_966_4e12 + y * (0.373_365_036_7e10 + y * (0.224_590_400_2e8 + y * (0.102_042_605_0e6 + y * (0.354_963_288_5e3 + y)))));
        num / den + core::f64::consts::FRAC_2_PI * (bessel_j1(x) * x.ln() - 1.0 / x)
    } else {
        let z = 8.0 / x;
        let y = z * z;
        let xx = x - 2.356_194_490_192_345;
        let p = 1.0 + y * (0.183_105e-2 + y * (-0.351_639_649_6e-4 + y * (0.245_752_017_4e-5 + y * (-0.240_337_019e-6))));
        let q = 0.046_874_999_95 + y * (-0.200_269_087_3e-3 + y * (0.844_919_909_6e-5 + y * (-0.882_289_87e-6 + y * 0.105_787_412e-6)));
        (core::f64::consts::FRAC_2_PI / x).sqrt() * (xx.sin() * p + z * xx.cos() * q)
    };
    Some(ans)
}

/// Integer-order `Y_n(x)` by (stable) upward recurrence from `Y0`, `Y1` (A&S 9.1.27).
fn bessel_yn(n: i32, x: f64) -> Option<f64> {
    if n.unsigned_abs() > MAX_BESSEL_ORDER as u32 || !x.is_finite() || x <= 0.0 {
        return None;
    }
    if n < 0 {
        let pos = bessel_yn(-n, x)?;
        return Some(if (-n) % 2 == 0 { pos } else { -pos });
    }
    if n == 0 {
        return bessel_y0(x);
    }
    if n == 1 {
        return bessel_y1(x);
    }
    let tox = 2.0 / x;
    let mut bym = bessel_y0(x)?;
    let mut by = bessel_y1(x)?;
    for j in 1..n {
        let byp = j as f64 * tox * by - bym;
        bym = by;
        by = byp;
    }
    if by.is_finite() { Some(by) } else { None }
}

/// Modified Bessel `I0(x)`: polynomial approximations A&S 9.8.1 (`|x| < 3.75`) and 9.8.2
/// (`|x| >= 3.75`), whose tabulated error bounds are of order `1e-7` (A&S 1964).
fn bessel_i0(x: f64) -> Option<f64> {
    let ax = x.abs();
    if ax > 700.0 {
        return None;
    }
    let ans = if ax < 3.75 {
        let y = (x / 3.75).powi(2);
        1.0 + y * (3.515_622_9 + y * (3.089_942_4 + y * (1.206_749_2 + y * (0.265_973_2 + y * (0.036_076_8 + y * 0.004_581_3)))))
    } else {
        let y = 3.75 / ax;
        (ax.exp() / ax.sqrt())
            * (0.398_942_28
                + y * (0.013_285_92
                    + y * (0.002_253_19
                        + y * (-0.001_575_65
                            + y * (0.009_162_81 + y * (-0.020_577_06 + y * (0.026_355_37 + y * (-0.016_476_33 + y * 0.003_923_77))))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Modified Bessel `I1(x)`: A&S 9.8.3 / 9.8.4 polynomial approximations (error order `1e-7`).
fn bessel_i1(x: f64) -> Option<f64> {
    let ax = x.abs();
    if ax > 700.0 {
        return None;
    }
    let ans = if ax < 3.75 {
        let y = (x / 3.75).powi(2);
        ax * (0.5 + y * (0.878_905_94 + y * (0.514_988_69 + y * (0.150_849_34 + y * (0.026_587_33 + y * (0.003_015_32 + y * 0.000_324_11))))))
    } else {
        let y = 3.75 / ax;
        (ax.exp() / ax.sqrt())
            * (0.398_942_28
                + y * (-0.039_880_24
                    + y * (-0.003_620_18
                        + y * (0.001_638_01
                            + y * (-0.010_315_55 + y * (0.022_829_67 + y * (-0.028_953_12 + y * (0.017_876_54 - y * 0.004_200_59))))))))
    };
    let signed = if x < 0.0 { -ans } else { ans };
    if signed.is_finite() { Some(signed) } else { None }
}

/// Modified Bessel `K0(x)`, `x > 0`: A&S 9.8.5 (`x <= 2`) / 9.8.6 (`x > 2`) (error order `1e-7`).
fn bessel_k0(x: f64) -> Option<f64> {
    if x <= 0.0 {
        return None;
    }
    let ans = if x <= 2.0 {
        let y = x * x / 4.0;
        -(x / 2.0).ln() * bessel_i0(x)?
            + (-core::f64::consts::EULER_GAMMA
                + y * (0.422_784_20 + y * (0.230_697_56 + y * (0.034_885_90 + y * (0.002_626_98 + y * (0.000_107_50 + y * 0.000_007_40))))))
    } else {
        let y = 2.0 / x;
        ((-x).exp() / x.sqrt())
            * (1.253_314_14
                + y * (-0.078_323_58 + y * (0.021_895_68 + y * (-0.010_624_46 + y * (0.005_878_72 + y * (-0.002_515_40 + y * 0.000_532_08))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Modified Bessel `K1(x)`, `x > 0`: A&S 9.8.7 (`x <= 2`) / 9.8.8 (`x > 2`) (error order `1e-7`).
fn bessel_k1(x: f64) -> Option<f64> {
    if x <= 0.0 {
        return None;
    }
    let ans = if x <= 2.0 {
        let y = x * x / 4.0;
        (x / 2.0).ln() * bessel_i1(x)?
            + (1.0 / x)
                * (1.0
                    + y * (0.154_431_44 + y * (-0.672_785_79 + y * (-0.181_568_97 + y * (-0.019_194_02 + y * (-0.001_104_04 - y * 0.000_046_86))))))
    } else {
        let y = 2.0 / x;
        ((-x).exp() / x.sqrt())
            * (1.253_314_14
                + y * (0.234_986_19 + y * (-0.036_556_20 + y * (0.015_042_68 + y * (-0.007_803_53 + y * (0.003_256_14 - y * 0.000_682_45))))))
    };
    if ans.is_finite() { Some(ans) } else { None }
}

/// Confluent hypergeometric function `1F1(a; b; z)` via series for moderate `|z|`.
///
/// Kummer's power series (A&S 13.1.2), truncated when a term falls below `1e-14` of the sum or
/// after 300 terms. No asymptotic expansion is used, so for large negative `z` the alternating
/// series loses accuracy to cancellation; no error bound is enforced.
fn hyp1f1(a: f64, b: f64, z: f64) -> Option<f64> {
    if b <= 0.0 && (b - b.round()).abs() < 1e-12 {
        return None;
    }
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    for k in 0..300 {
        let kf = k as f64;
        term *= ((a + kf) / (b + kf)) * (z / (kf + 1.0));
        if !term.is_finite() {
            return None;
        }
        sum += term;
        if term.abs() <= 1e-14 * sum.abs().max(1.0) {
            return Some(sum);
        }
    }
    if sum.is_finite() { Some(sum) } else { None }
}

/// Gauss hypergeometric function `2F1(a, b; c; z)` for `|z| < 1`.
///
/// Gauss series (A&S 15.1.1), truncated at `1e-14` relative or 400 terms; convergence is slow as
/// `|z| -> 1` and no analytic continuation is applied.
fn hyp2f1(a: f64, b: f64, c: f64, z: f64) -> Option<f64> {
    if z.abs() >= 1.0 || (c <= 0.0 && (c - c.round()).abs() < 1e-12) {
        return None;
    }
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    for k in 0..400 {
        let kf = k as f64;
        term *= ((a + kf) * (b + kf) / (c + kf)) * (z / (kf + 1.0));
        if !term.is_finite() {
            return None;
        }
        sum += term;
        if term.abs() <= 1e-14 * sum.abs().max(1.0) {
            return Some(sum);
        }
    }
    if sum.is_finite() { Some(sum) } else { None }
}

/// Tricomi confluent hypergeometric `U(a, b, z)` via non-integer connection formula (`b` perturbed slightly at integers).
///
/// Connection formula A&S 13.1.3 in terms of `1F1`. The `1e-7` shift of integer `b` is a
/// heuristic without a documented error bound (the formula is singular at integer `b`, where
/// A&S 13.1.6 limit forms apply), and accuracy degrades for large `z` through `hyp1f1`.
fn confluent_u(a: f64, b_in: f64, z: f64) -> Option<f64> {
    if z <= 0.0 {
        return None;
    }
    let b = if (b_in - b_in.round()).abs() < 1e-7 { b_in + 1e-7 } else { b_in };
    let pi = core::f64::consts::PI;
    let t1 = (pi / (pi * b).sin())
        * (hyp1f1(a, b, z)? / (gamma(1.0 + a - b).ok()? * gamma(b).ok()?)
            - z.powf(1.0 - b) * hyp1f1(1.0 + a - b, 2.0 - b, z)? / (gamma(a).ok()? * gamma(2.0 - b).ok()?));
    if t1.is_finite() { Some(t1) } else { None }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use crate::library::ParametricModel;

    use super::{
        BinaryOp, CompiledDslModel, DatasetFilter, Expr, MAX_DATASET_EXPANSION, MAX_PARSE_DEPTH, UnaryOp, bessel_jn, bessel_yn, collect_free_idents,
        compile_expr, expr_references_y, integrate_rk45, parse_dataset_prefix,
    };

    #[test]
    fn rk45_tiny_and_non_finite_spans_do_not_panic_or_hang() {
        let tiny = integrate_rk45(0.0, 1e-10, 2.0, |_t, y| Some(-y)).unwrap();
        assert!((tiny - 2.0 * (-1e-10_f64).exp()).abs() < 1e-14);
        assert!(integrate_rk45(f64::NAN, 1.0, 1.0, |_t, y| Some(-y)).is_none());
        assert!(integrate_rk45(0.0, f64::NAN, 1.0, |_t, y| Some(-y)).is_none());
        assert!(integrate_rk45(0.0, f64::INFINITY, 1.0, |_t, y| Some(-y)).is_none());
        assert!(integrate_rk45(-1e308, 1e308, 1.0, |_t, _y| Some(0.0)).is_none());
    }

    #[test]
    fn clamp_and_bessel_guard_extremes() {
        let clamp_m = CompiledDslModel::compile("c", "Y = clamp(X, a, b)").unwrap();
        assert_eq!(clamp_m.eval(&[1.0], &[f64::NAN, f64::NAN], 0), None);
        assert_eq!(clamp_m.eval(&[5.0], &[0.0, f64::INFINITY], 0), Some(5.0));

        assert!(bessel_jn(i32::MIN, 1.0).is_nan());
        assert!(bessel_jn(1_000_000, 1.0).is_nan());
        assert!(bessel_yn(i32::MIN, 1.0).is_none());
        assert!(bessel_yn(1_000_000, 1.0).is_none());

        let jm = CompiledDslModel::compile("jn", "Y = jn(n, X)").unwrap();
        assert_eq!(jm.eval(&[1.0], &[-1e12], 0), None);
        assert_eq!(jm.eval(&[1.0], &[f64::NAN], 0), None);
    }

    #[test]
    fn dataset_prefix_caps_expansion_and_overflow() {
        let ok = parse_dataset_prefix("<0:9999> Y = X", 1).unwrap();
        assert!(matches!(ok.0, DatasetFilter::Include(v) if v.len() == MAX_DATASET_EXPANSION));
        assert!(parse_dataset_prefix("<0:10000> Y = X", 1).is_err());
        assert!(parse_dataset_prefix("<0:18446744073709551615> Y = X", 1).is_err());
    }

    #[test]
    fn parser_depth_limit_and_iterative_ast_walkers_prevent_stack_overflow() {
        let ok_src = format!("Y = {}X{}", "(".repeat(MAX_PARSE_DEPTH), ")".repeat(MAX_PARSE_DEPTH));
        assert!(CompiledDslModel::compile("ok_depth", &ok_src).is_ok());

        let too_deep = format!("Y = {}X{}", "(".repeat(MAX_PARSE_DEPTH + 1), ")".repeat(MAX_PARSE_DEPTH + 1));
        assert!(CompiledDslModel::compile("too_deep", &too_deep).is_err());

        let too_many_unary = format!("Y = {}X", "-".repeat(MAX_PARSE_DEPTH + 1));
        assert!(CompiledDslModel::compile("unary_deep", &too_many_unary).is_err());

        let mut deep = Expr::Ident("Y".to_owned());
        for _ in 0..20_000 {
            deep = Expr::Unary { op: UnaryOp::Neg, rhs: Box::new(deep) };
        }
        assert!(expr_references_y(&deep));
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        collect_free_idents(&deep, &BTreeSet::new(), &mut seen, &mut order);
        let mut code = Vec::new();
        compile_expr(&deep, &HashMap::new(), &HashMap::new(), &mut code).unwrap();
        assert_eq!(code.len(), 20_001);
        drop(deep);
    }

    #[test]
    fn deep_expr_all_variants_clone_and_debug_without_stack_overflow() {
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut expr = Expr::Ident("X".to_owned());
                for step in 0..50_000_usize {
                    expr = match step % 4 {
                        0 => Expr::Unary { op: UnaryOp::Neg, rhs: Box::new(expr) },
                        1 => Expr::Binary { op: BinaryOp::Add, lhs: Box::new(expr), rhs: Box::new(Expr::Num(1.0)) },
                        2 => Expr::Call { name: "sin".to_owned(), args: vec![Expr::Num(0.5), expr] },
                        _ => Expr::If { cond: Box::new(Expr::Ident("k".to_owned())), then_br: Box::new(expr), else_br: Box::new(Expr::Num(0.0)) },
                    };
                }
                let cloned = expr.clone();
                let dbg_orig = format!("{expr:?}");
                let dbg_cloned = format!("{cloned:?}");
                assert_eq!(dbg_orig, dbg_cloned);
                assert!(dbg_orig.contains("Unary"));
                assert!(dbg_orig.contains("Binary"));
                assert!(dbg_orig.contains("Call"));
                assert!(dbg_orig.contains("If"));
                assert!(dbg_orig.contains("Ident"));
                assert!(dbg_orig.contains("Num"));

                let mut empty_call = Expr::Call { name: "f".to_owned(), args: Vec::new() };
                for _ in 0..10 {
                    empty_call = Expr::Call { name: "g".to_owned(), args: vec![empty_call] };
                }
                assert_eq!(format!("{empty_call:?}"), format!("{:?}", empty_call.clone()));
                drop(cloned);
                drop(expr);
            })
            .unwrap();
        handle.join().unwrap();
    }
}
