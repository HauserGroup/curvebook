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
//! Reference: Olver, F. W. J., Olde Daalhuis, A. B., Lozier, D. W., Schneider, B. I., Boisvert, R. F., Clark, C. W., Miller, B. R., Saunders, B. V., Cohl, H. S. & McClain, M. A. (eds.). *NIST Digital Library of Mathematical Functions* (DLMF), Ch. 10 (Bessel Functions), Ch. 13 (Confluent Hypergeometric Functions), Ch. 15 (Hypergeometric Function).
//!
//! Reference: Thompson, I. J. & Barnett, A. R. (1986). Coulomb and Bessel functions of complex arguments and order. *Journal of Computational Physics* 64(2), 490–509.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use lumencraft_math::dist::{norm_cdf, norm_ppf};
use lumencraft_math::quad::{QuadOptions, integrate_adaptive};
use lumencraft_math::roots::{RootOptions, brent_root};
use lumencraft_math::special::{beta, digamma, erf, erfc, gamma, ln_gamma};
use lumencraft_math::sum::NeumaierSum;

use crate::error::FitError;
use crate::library::{ParametricModel, estimate_x_at_ymid};
use crate::types::{DataPoint, DerivedScalar, EquationCategory};

const MAX_VM_STEPS: usize = 100_000;
const MAX_ODE_STEPS: usize = 50_000;
pub(crate) const DEFAULT_MAX_ODE_STEPS_PER_FIT: usize = 300_000;
const MAX_CACHED_ODE_TRAJECTORIES: usize = 4;
const MAX_CACHED_ODE_KNOTS: usize = 4_096;
const MAX_PARSE_DEPTH: usize = 128;
const MAX_AST_NODES: usize = 4_096;
const MAX_DATASET_EXPANSION: usize = 10_000;
const MAX_DSL_SOURCE_BYTES: usize = 1_048_576;
const MAX_DSL_LINES: usize = 16_384;
const MAX_TOTAL_DATASET_INDICES: usize = 50_000;
const MAX_DATASET_CLASSES: usize = 4_096;
const MAX_TOTAL_OPCODES: usize = 500_000;
const MAX_BESSEL_ORDER: i32 = 100_000;

/// Dataset-filter predicate parsed from `<A>`, `<~A>`, `<B:D>`, or `<0>` line prefixes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone)]
struct CompiledInitRule {
    filter: DatasetFilter,
    slot: usize,
    param_idx: Option<usize>,
    prog: CompiledProgram,
}

#[derive(Debug, Clone, Copy)]
struct OdeFitBudgetState {
    used_steps: usize,
    max_steps: usize,
    exceeded: bool,
}

std::thread_local! {
    static ACTIVE_ODE_BUDGET: RefCell<Option<OdeFitBudgetState>> = const { RefCell::new(None) };
}

/// RAII scope bounding total adaptive RK45 steps across a single nonlinear fit execution.
pub(crate) struct OdeFitBudgetGuard {
    prev: Option<OdeFitBudgetState>,
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

fn consume_ode_fit_step() -> bool {
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
struct OdeKnot {
    dist: f64,
    t: f64,
    y: f64,
    h: f64,
}

#[derive(Debug, Clone, Default)]
struct TrajectorySide {
    knots: Vec<OdeKnot>,
    steps: usize,
    failed_beyond_dist: Option<f64>,
}

impl TrajectorySide {
    fn thin_if_needed(&mut self) {
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

    fn insert_knots(&mut self, insert_at: usize, new_knots: Vec<OdeKnot>) {
        if new_knots.is_empty() {
            return;
        }
        self.knots.splice(insert_at..insert_at, new_knots);
        self.thin_if_needed();
    }
}

#[derive(Debug, Clone)]
struct CachedOdeTrajectory {
    model_id: u64,
    ds_idx: usize,
    t0: f64,
    y0: f64,
    params: Vec<f64>,
    x_tail: Vec<f64>,
    pos: TrajectorySide,
    neg: TrajectorySide,
}

fn slice_bits_eq(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.to_bits() == y.to_bits())
}

impl CachedOdeTrajectory {
    fn new(model_id: u64, ds_idx: usize, t0: f64, y0: f64, params: &[f64], x_tail: &[f64]) -> Self {
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

    fn matches(&self, model_id: u64, ds_idx: usize, t0: f64, y0: f64, params: &[f64], x_tail: &[f64]) -> bool {
        self.model_id == model_id
            && self.ds_idx == ds_idx
            && self.t0.to_bits() == t0.to_bits()
            && self.y0.to_bits() == y0.to_bits()
            && slice_bits_eq(&self.params, params)
            && slice_bits_eq(&self.x_tail, x_tail)
    }
}

static NEXT_ODE_MODEL_ID: AtomicU64 = AtomicU64::new(1);

std::thread_local! {
    static ODE_TRAJECTORY_CACHE: RefCell<Vec<CachedOdeTrajectory>> = const { RefCell::new(Vec::new()) };
}

fn take_cached_ode_trajectory(model_id: u64, ds_idx: usize, t0: f64, y0: f64, params: &[f64], x_tail: &[f64]) -> CachedOdeTrajectory {
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

fn store_cached_ode_trajectory(entry: CachedOdeTrajectory) {
    let _ = ODE_TRAJECTORY_CACHE.try_with(|slot| {
        if let Ok(mut entries) = slot.try_borrow_mut() {
            entries.retain(|e| !e.matches(entry.model_id, entry.ds_idx, entry.t0, entry.y0, &entry.params, &entry.x_tail));
            entries.insert(0, entry);
            entries.truncate(MAX_CACHED_ODE_TRAJECTORIES);
        }
    });
}

/// Compiled custom equation implementing [`ParametricModel`] safely inside a bounded VM.
#[derive(Debug, Clone)]
pub struct CompiledDslModel {
    id: String,
    model_id: u64,
    name: String,
    kind: DslEquationKind,
    param_names: Vec<String>,
    programs: Vec<(DatasetFilter, CompiledProgram)>,
    program_by_ds: HashMap<usize, usize>,
    default_program_idx: Option<usize>,
    ode_init_program: Option<CompiledProgram>,
    ode_t0: f64,
    initial_hints: HashMap<String, f64>,
    initial_rules: Vec<CompiledInitRule>,
    init_local_count: usize,
    transforms: Vec<(String, usize, CompiledProgram)>,
    transform_local_count: usize,
}

impl CompiledDslModel {
    /// Compile a user equation string (explicit `Y=...`, conditional `IF`, `<A>` dataset lines,
    /// implicit `LHS=RHS`, or 1st-order ODE `dY/dX=...` / `D(Y, X)=...`).
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
        let mut ode_t0 = 0.0_f64;
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
            let trimmed = strip_comment(raw_line, &mut initial_hints, &mut ode_t0);
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
            match section {
                DslSection::Equation => {
                    let stmt = parse_statement(rest, line_no)?;
                    parsed_lines.push((line_no, filter, stmt));
                }
                DslSection::InitialValues => match parse_statement(rest, line_no)? {
                    ParsedStatement::Assign { name, rhs } | ParsedStatement::OdeInitial { lhs_name: Some(name), rhs, .. } => {
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

        let ode_init_program = if kind == DslEquationKind::OdeFirstOrder {
            let remaining = MAX_TOTAL_OPCODES.saturating_sub(total_opcodes);
            let prog = if let Some((_, init_expr)) = init_stmts.last() {
                compile_program_refs(&[], init_expr, &param_map, remaining)?
            } else {
                let y0_idx = *param_map.get("Y0").unwrap_or(&0);
                CompiledProgram { code: vec![OpCode::LoadParam(y0_idx), OpCode::Halt], local_count: 0 }
            };
            total_opcodes = total_opcodes.saturating_add(prog.code.len());
            if total_opcodes > MAX_TOTAL_OPCODES {
                return Err(FitError::DslCompileError {
                    message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                    line: 1,
                    column: 1,
                });
            }
            Some(prog)
        } else {
            None
        };

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
            ode_init_program,
            ode_t0,
            initial_hints,
            initial_rules,
            init_local_count,
            transforms,
            transform_local_count,
        })
    }

    #[must_use]
    pub fn kind(&self) -> DslEquationKind {
        self.kind
    }

    fn select_program(&self, ds_idx: usize) -> Option<&CompiledProgram> {
        let idx = self.program_by_ds.get(&ds_idx).copied().or(self.default_program_idx)?;
        self.programs.get(idx).map(|(_, p)| p)
    }

    /// Solve the implicit equation `F(x, y; theta) = 0` for `y`: bracket a sign change over a
    /// fixed list of trial `y` values and refine with Brent (1973); if no bracket is found, fall
    /// back to a step-clamped Newton iteration with central-difference derivative and accept
    /// only `|F| < 1e-7`. Only one root is returned (the first bracket found), so equations with
    /// several real roots in `y` may select a branch that depends on the trial order.
    fn eval_implicit(&self, x: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        let prog = self.select_program(ds_idx)?;
        let f = |y: f64| -> f64 { eval_bytecode(prog, x, y, params, ds_idx).unwrap_or(f64::NAN) };
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
                        && refined.is_finite()
                    {
                        return Some(refined);
                    }
                }
            }
            Some(root)
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
    /// with per-trajectory continuation caching so sweeps reuse work across points.
    fn eval_ode_single(&self, t_end: f64, x_tail: &[f64], params: &[f64], ds_idx: usize) -> Option<f64> {
        if ode_fit_budget_exceeded().is_some() {
            return None;
        }
        let prog = self.select_program(ds_idx)?;
        let init_prog = self.ode_init_program.as_ref()?;
        if !self.ode_t0.is_finite() || !t_end.is_finite() {
            return None;
        }

        let mut xv = Vec::with_capacity(1 + x_tail.len());
        xv.push(self.ode_t0);
        xv.extend_from_slice(x_tail);
        let mut stack = Vec::with_capacity(32);
        let mut init_locals = vec![None; init_prog.local_count];
        let y0 = eval_bytecode_with_bufs(init_prog, &xv, 0.0, params, ds_idx, &mut init_locals, &mut stack)?;
        if !y0.is_finite() {
            return None;
        }
        if (t_end - self.ode_t0).abs() < 1e-15 {
            return Some(y0);
        }

        let dir = if t_end >= self.ode_t0 { 1.0 } else { -1.0 };
        let target_dist = (t_end - self.ode_t0) * dir;
        if target_dist <= 0.0 || !target_dist.is_finite() {
            return None;
        }

        let mut traj = take_cached_ode_trajectory(self.model_id, ds_idx, self.ode_t0, y0, params, x_tail);
        let side = if dir > 0.0 { &mut traj.pos } else { &mut traj.neg };

        if side.failed_beyond_dist.is_some_and(|fail_d| target_dist > fail_d) {
            store_cached_ode_trajectory(traj);
            return None;
        }

        let idx = side.knots.partition_point(|k| k.dist <= target_dist);
        let cached_y = [idx.checked_sub(1).and_then(|i| side.knots.get(i)), side.knots.get(idx)]
            .into_iter()
            .flatten()
            .find(|c| (target_dist - c.dist).abs() <= 1e-14 * target_dist || (t_end - c.t).abs() <= 4.0 * f64::EPSILON * t_end.abs())
            .map(|c| c.y);
        if let Some(y) = cached_y {
            store_cached_ode_trajectory(traj);
            return Some(y);
        }

        let start_knot = idx.checked_sub(1).and_then(|i| side.knots.get(i).cloned()).unwrap_or(OdeKnot { dist: 0.0, t: self.ode_t0, y: y0, h: 0.0 });
        let advancing_frontier = idx == side.knots.len();
        let rem_steps = if advancing_frontier { MAX_ODE_STEPS.saturating_sub(side.steps) } else { MAX_ODE_STEPS.saturating_sub(side.steps).max(64) };
        if rem_steps == 0 {
            if advancing_frontier {
                side.failed_beyond_dist = Some(side.failed_beyond_dist.map_or(start_knot.dist, |prev| prev.min(start_knot.dist)));
            }
            store_cached_ode_trajectory(traj);
            return None;
        }

        let mut rhs_locals = vec![None; prog.local_count];
        let mut collected = Vec::new();
        let outcome =
            integrate_rk45_segment(start_knot.t, t_end, start_knot.y, start_knot.h, target_dist, rem_steps, true, &mut collected, |t, y| {
                xv[0] = t;
                rhs_locals.fill(None);
                stack.clear();
                eval_bytecode_with_bufs(prog, &xv, y, params, ds_idx, &mut rhs_locals, &mut stack)
            });

        let mut last_kd = start_knot.dist;
        let mut new_knots = Vec::with_capacity(collected.len() + 1);
        for (kt, ky, kh) in collected {
            let kd = (kt - self.ode_t0) * dir;
            if kd > last_kd && kd < target_dist {
                new_knots.push(OdeKnot { dist: kd, t: kt, y: ky, h: kh });
                last_kd = kd;
            }
        }

        let result = match outcome {
            Rk45Outcome::Reached { y, h, steps_taken } => {
                side.steps = side.steps.saturating_add(steps_taken);
                new_knots.push(OdeKnot { dist: target_dist, t: t_end, y, h });
                side.insert_knots(idx, new_knots);
                Some(y)
            }
            Rk45Outcome::IntrinsicFailed { last_t, steps_taken } => {
                side.steps = side.steps.saturating_add(steps_taken);
                let last_accepted_dist = new_knots.last().map_or(start_knot.dist, |k| k.dist).max((last_t - self.ode_t0) * dir);
                side.insert_knots(idx, new_knots);
                if advancing_frontier {
                    side.failed_beyond_dist = Some(side.failed_beyond_dist.map_or(last_accepted_dist, |prev| prev.min(last_accepted_dist)));
                }
                None
            }
            Rk45Outcome::BudgetExceeded { steps_taken } => {
                side.steps = side.steps.saturating_add(steps_taken);
                side.insert_knots(idx, new_knots);
                None
            }
        };
        store_cached_ode_trajectory(traj);
        result
    }
}

fn implicit_probes_and_scale(x: &[f64], params: &[f64]) -> (Vec<f64>, f64) {
    const MAGS: [f64; 34] = [
        1e-30, 1e-24, 1e-18, 1e-15, 1e-14, 1e-13, 1e-12, 1e-11, 1e-10, 1e-9, 1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 0.05, 0.1, 0.25, 0.5, 1.0,
        2.0, 5.0, 10.0, 100.0, 1e3, 1e4, 1e6, 1e9, 1e15, 1e18, 1e24, 1e30,
    ];
    let mut pos_probes = Vec::with_capacity(64);
    pos_probes.extend_from_slice(&MAGS);
    let mut max_data_scale = 0.0_f64;
    for &v in x.iter().chain(params.iter()) {
        let av = v.abs();
        if v.is_finite() && av > 0.0 {
            max_data_scale = max_data_scale.max(av);
            for factor in [0.1, 0.5, 1.0, 2.0, 10.0] {
                let scaled = av * factor;
                if scaled.is_finite() && scaled > 0.0 {
                    pos_probes.push(scaled);
                }
            }
        }
    }
    pos_probes.sort_by(f64::total_cmp);
    pos_probes.dedup_by(|a, b| (*a - *b).abs() <= 1e-12 * a.abs().max(b.abs()));
    (pos_probes, max_data_scale.max(1e-12))
}

fn implicit_newton_fallback<F, B>(f: &F, brent_scaled: &B, best_starts: &[(f64, f64)], anchor_scale: f64) -> Option<f64>
where
    F: Fn(f64) -> f64,
    B: Fn(f64, f64) -> Option<f64>,
{
    for &(start_y, start_fy) in best_starts {
        let mut y = start_y;
        let mut fy = start_fy;
        let init_f_abs = fy.abs();
        for _ in 0..48 {
            if !fy.is_finite() {
                break;
            }
            if fy == 0.0 {
                return Some(y);
            }
            let h = 1e-6 * if y != 0.0 { y.abs() } else { anchor_scale };
            let fp = f(y + h);
            let fm = f(y - h);
            let df = if fp.is_finite() && fm.is_finite() {
                (fp - fm) / (2.0 * h)
            } else if fp.is_finite() {
                (fp - fy) / h
            } else if fm.is_finite() {
                (fy - fm) / h
            } else {
                break;
            };
            if !df.is_finite() || df == 0.0 {
                break;
            }
            let delta = fy / df;
            if !delta.is_finite() {
                break;
            }
            let y_tol = 1e-11 * y.abs().max(anchor_scale * 1e-6);
            let ref_f = init_f_abs.max(df.abs() * y.abs().max(anchor_scale * 1e-6)).max(f64::MIN_POSITIVE);
            if fy.abs() <= 1e-12 * ref_f && delta.abs() <= y_tol {
                return Some(y);
            }
            let mut alpha = 1.0_f64;
            let mut accepted = false;
            for _ in 0..16 {
                let cand = y - alpha * delta;
                if cand.is_finite() {
                    let f_cand = f(cand);
                    if f_cand.is_finite() {
                        if f_cand == 0.0 {
                            return Some(cand);
                        }
                        if (f_cand > 0.0) != (fy > 0.0)
                            && let Some(root) = brent_scaled(y.min(cand), y.max(cand))
                        {
                            return Some(root);
                        }
                        if f_cand.abs() < fy.abs() {
                            y = cand;
                            fy = f_cand;
                            accepted = true;
                            break;
                        }
                    }
                }
                alpha *= 0.5;
            }
            if !accepted {
                break;
            }
        }
        let h_end = 1e-6 * if y != 0.0 { y.abs() } else { anchor_scale };
        let df_end = ((f(y + h_end) - f(y - h_end)) / (2.0 * h_end)).abs();
        let ref_f = init_f_abs.max(if df_end.is_finite() { df_end * y.abs().max(anchor_scale * 1e-6) } else { 0.0 }).max(f64::MIN_POSITIVE);
        if fy.is_finite()
            && df_end.is_finite()
            && df_end > 0.0
            && fy.abs() <= 1e-11 * ref_f
            && (fy / df_end).abs() <= 1e-10 * y.abs().max(anchor_scale * 1e-6)
        {
            return Some(y);
        }
    }
    None
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

enum Rk45Outcome {
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
/// the initial step is `0.05 * span`. Non-finite stages shrink the step by 4; integration fails
/// (`None`) below `1e-13 * max(span, 1)` or after `MAX_ODE_STEPS` steps.
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
fn integrate_rk45_segment<F>(
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
        let Some(k1) = rhs(t, y) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let Some(k2) = rhs(t + 0.2 * dt, y + dt * A21 * k1) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let Some(k3) = rhs(t + 0.3 * dt, y + dt * (A31 * k1 + A32 * k2)) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let Some(k4) = rhs(t + 0.8 * dt, y + dt * (A41 * k1 + A42 * k2 + A43 * k3)) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let Some(k5) = rhs(t + (8.0 / 9.0) * dt, y + dt * (A51 * k1 + A52 * k2 + A53 * k3 + A54 * k4)) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };
        let Some(k6) = rhs(t + dt, y + dt * (A61 * k1 + A62 * k2 + A63 * k3 + A64 * k4 + A65 * k5)) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
        };

        let y_next = y + dt * (B1 * k1 + B3 * k3 + B4 * k4 + B5 * k5 + B6 * k6);
        if !y_next.is_finite() {
            h *= 0.25;
            if h < min_h {
                return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
            }
            continue;
        }
        let Some(k7) = rhs(t + dt, y_next) else {
            return Rk45Outcome::IntrinsicFailed { last_t: t, steps_taken };
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

// ============================================================================
// Parser & Bytecode Compiler
// ============================================================================

enum ParsedStatement {
    OdeDerivative(Expr),
    OdeInitial { lhs_name: Option<String>, t0: Option<f64>, rhs: Expr },
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
    indices.sort_unstable();
    indices.dedup();
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
        if lhs_upper == "Y0" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: Some(lhs_trim.to_owned()), t0: Some(0.0), rhs });
        }
        if lhs_upper == "Y(0)" {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(0.0), rhs });
        }
        if let Some(inner) = lhs_upper.strip_prefix("Y(").and_then(|s| s.strip_suffix(')'))
            && let Ok(t0_val) = inner.parse::<f64>()
        {
            let rhs = parse_expr(rhs_s, line_no)?;
            return Ok(ParsedStatement::OdeInitial { lhs_name: None, t0: Some(t0_val), rhs });
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

struct BuiltEqItems {
    kind: DslEquationKind,
    items: Vec<EqItem>,
    init_stmts: Vec<(usize, Expr)>,
    ode_t0: f64,
}

fn build_eq_items(parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>, mut ode_t0: f64) -> Result<BuiltEqItems, FitError> {
    let has_ode = parsed_lines.iter().any(|(_, _, s)| matches!(s, ParsedStatement::OdeDerivative(_)));
    let mut kind = DslEquationKind::Explicit;
    let mut items = Vec::with_capacity(parsed_lines.len());
    let mut init_stmts = Vec::new();
    for (line_no, filter, stmt) in parsed_lines {
        match stmt {
            ParsedStatement::OdeDerivative(rhs) => {
                kind = DslEquationKind::OdeFirstOrder;
                items.push(EqItem::Ode { filter, rhs });
            }
            ParsedStatement::OdeInitial { lhs_name, t0, rhs } => {
                if has_ode {
                    if let Some(t) = t0 {
                        ode_t0 = t;
                    }
                    init_stmts.push((line_no, rhs));
                } else if let Some(name) = lhs_name {
                    // Non-ODE `Y0 = expr` is an ordinary local assignment rather than a hidden/discarded ODE IC.
                    items.push(EqItem::Local { filter, name, rhs });
                } else {
                    return Err(FitError::DslCompileError {
                        message: "ODE initial condition `Y(t0) = ...` requires an ODE derivative equation (`dY/dX = ...`)".to_owned(),
                        line: line_no,
                        column: 1,
                    });
                }
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
    Ok(BuiltEqItems { kind, items, init_stmts, ode_t0 })
}

struct FilterClassPartition<'a> {
    explicit_classes: Vec<DatasetFilter>,
    outside_filter: DatasetFilter,
    filter_ids: HashMap<&'a DatasetFilter, usize>,
    matching_classes: Vec<Vec<usize>>,
}

fn collect_filter_classes(eq_items: &[EqItem]) -> Result<FilterClassPartition<'_>, FitError> {
    let mut unique_filters: Vec<&DatasetFilter> = Vec::new();
    let mut filter_ids: HashMap<&DatasetFilter, usize> = HashMap::new();
    for item in eq_items {
        let f = item.filter();
        if *f != DatasetFilter::Any && !filter_ids.contains_key(f) {
            let fi = unique_filters.len();
            unique_filters.push(f);
            filter_ids.insert(f, fi);
        }
    }

    let mut events_by_ds: HashMap<usize, Vec<i32>> = HashMap::new();
    for (fi, &f) in unique_filters.iter().enumerate() {
        let tag = i32::try_from(fi + 1).map_err(|_| FitError::DslCompileError {
            message: "Too many distinct dataset filters".to_owned(),
            line: 1,
            column: 1,
        })?;
        match f {
            DatasetFilter::Any => {}
            DatasetFilter::Include(v) => {
                for &ds in v {
                    events_by_ds.entry(ds).or_default().push(tag);
                }
            }
            DatasetFilter::Exclude(v) => {
                for &ds in v {
                    events_by_ds.entry(ds).or_default().push(-tag);
                }
            }
        }
    }

    let mut mentioned: Vec<usize> = events_by_ds.keys().copied().collect();
    mentioned.sort_unstable();
    let outside_filter = if mentioned.is_empty() { DatasetFilter::Any } else { DatasetFilter::Exclude(mentioned) };

    let mut groups: HashMap<Vec<i32>, Vec<usize>> = HashMap::new();
    for (ds, ev) in events_by_ds {
        groups.entry(ev).or_default().push(ds);
    }
    let mut explicit_classes: Vec<(usize, DatasetFilter)> = groups
        .into_values()
        .map(|mut indices| {
            indices.sort_unstable();
            let rep = indices[0];
            (rep, DatasetFilter::Include(indices))
        })
        .collect();
    explicit_classes.sort_by_key(|(rep, _)| *rep);
    if explicit_classes.len() > MAX_DATASET_CLASSES {
        return Err(FitError::DslCompileError {
            message: format!("Distinct dataset equivalence classes exceed limit ({MAX_DATASET_CLASSES})"),
            line: 1,
            column: 1,
        });
    }

    let explicit_filters: Vec<DatasetFilter> = explicit_classes.into_iter().map(|(_, f)| f).collect();
    let num_explicit = explicit_filters.len();
    let total_classes = num_explicit + 1;

    let mut ds_to_class = HashMap::new();
    for (c_idx, f) in explicit_filters.iter().enumerate() {
        if let DatasetFilter::Include(v) = f {
            for &ds in v {
                ds_to_class.insert(ds, c_idx);
            }
        }
    }

    let mut matching_classes = vec![Vec::new(); unique_filters.len()];
    let mut stamp = vec![usize::MAX; total_classes];
    for (fi, &f) in unique_filters.iter().enumerate() {
        match f {
            DatasetFilter::Any => {}
            DatasetFilter::Include(v) => {
                for &ds in v {
                    if let Some(&c_idx) = ds_to_class.get(&ds)
                        && stamp[c_idx] != fi
                    {
                        stamp[c_idx] = fi;
                        matching_classes[fi].push(c_idx);
                    }
                }
            }
            DatasetFilter::Exclude(v) => {
                for &ds in v {
                    if let Some(&c_idx) = ds_to_class.get(&ds) {
                        stamp[c_idx] = fi;
                    }
                }
                for (c_idx, st) in stamp.iter().enumerate().take(total_classes) {
                    if *st != fi {
                        matching_classes[fi].push(c_idx);
                    }
                }
            }
        }
    }

    Ok(FilterClassPartition { explicit_classes: explicit_filters, outside_filter, filter_ids, matching_classes })
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

fn target_expr_for_unbound_check(item: &EqItem, kind: DslEquationKind) -> Option<&Expr> {
    match (kind, item) {
        (DslEquationKind::OdeFirstOrder, EqItem::Ode { rhs, .. }) => Some(rhs),
        (DslEquationKind::Implicit, EqItem::Implicit { diff, .. }) => Some(diff),
        (DslEquationKind::Implicit | DslEquationKind::Explicit, EqItem::Explicit { expr, .. }) => Some(expr),
        _ => None,
    }
}

fn branch_uses_unbound_scoped_indices(
    eq_items: &[EqItem],
    local_indices: &[usize],
    target_idx: usize,
    kind: DslEquationKind,
    scoped_names: &HashSet<String>,
) -> bool {
    let Some(target) = target_expr_for_unbound_check(&eq_items[target_idx], kind) else {
        return false;
    };
    let mut free = Vec::new();
    let mut seen = BTreeSet::new();
    let mut bound = BTreeSet::new();
    for &idx in local_indices {
        if let EqItem::Local { name, rhs, .. } = &eq_items[idx] {
            collect_free_idents(rhs, &bound, &mut seen, &mut free);
            bound.insert(name.clone());
        }
    }
    collect_free_idents(target, &bound, &mut seen, &mut free);
    free.iter().any(|ident| scoped_names.contains(ident))
}

struct ResolvedEquationBody {
    kind: DslEquationKind,
    body_stmts: Vec<OutputBranch>,
    init_stmts: Vec<(usize, Expr)>,
    ode_t0: f64,
}

fn resolve_equation_branches(parsed_lines: Vec<(usize, DatasetFilter, ParsedStatement)>, ode_t0: f64) -> Result<ResolvedEquationBody, FitError> {
    let BuiltEqItems { kind, items: eq_items, init_stmts, ode_t0 } = build_eq_items(parsed_lines, ode_t0)?;
    let has_ode_output = eq_items.iter().any(|it| matches!(it, EqItem::Ode { .. }));
    if kind == DslEquationKind::OdeFirstOrder && !has_ode_output {
        return Err(FitError::DslCompileError { message: "Missing ODE derivative expression".to_owned(), line: 1, column: 1 });
    }
    let scoped_local_names: HashSet<String> = eq_items
        .iter()
        .filter_map(|it| match it {
            EqItem::Local { filter, name, .. } if *filter != DatasetFilter::Any => Some(name.clone()),
            _ => None,
        })
        .collect();

    let partition = collect_filter_classes(&eq_items)?;
    let num_explicit = partition.explicit_classes.len();
    let outside_class = num_explicit;
    let total_classes = num_explicit + 1;

    let mut class_locals: Vec<Vec<usize>> = vec![Vec::new(); total_classes];
    let mut explicit_outputs: Vec<Option<(usize, usize)>> = vec![None; num_explicit];
    let mut outside_outputs: Vec<(usize, usize, bool)> = Vec::new();
    let mut total_branch_refs = 0_usize;

    for (item_idx, item) in eq_items.iter().enumerate() {
        let f = item.filter();
        let is_local = matches!(item, EqItem::Local { .. });
        let is_target = target_expr_for_unbound_check(item, kind).is_some();
        if !is_local && !is_target {
            continue;
        }
        let was_any = *f == DatasetFilter::Any;
        let classes_slice: &[usize] = if was_any {
            &[]
        } else if let Some(&fi) = partition.filter_ids.get(f) {
            &partition.matching_classes[fi]
        } else {
            &[]
        };

        let count_matched = if was_any { total_classes } else { classes_slice.len() };
        if is_local {
            total_branch_refs = total_branch_refs.saturating_add(count_matched);
            if total_branch_refs > MAX_TOTAL_OPCODES {
                return Err(FitError::DslCompileError {
                    message: format!("Compiled equation exceeds branch-expansion budget ({MAX_TOTAL_OPCODES})"),
                    line: 1,
                    column: 1,
                });
            }
        }

        if was_any {
            for c_idx in 0..total_classes {
                if is_local {
                    class_locals[c_idx].push(item_idx);
                } else if c_idx == outside_class {
                    outside_outputs.push((class_locals[outside_class].len(), item_idx, true));
                } else {
                    explicit_outputs[c_idx] = Some((class_locals[c_idx].len(), item_idx));
                }
            }
        } else {
            for &c_idx in classes_slice {
                if is_local {
                    class_locals[c_idx].push(item_idx);
                } else if c_idx == outside_class {
                    outside_outputs.push((class_locals[outside_class].len(), item_idx, false));
                } else {
                    explicit_outputs[c_idx] = Some((class_locals[c_idx].len(), item_idx));
                }
            }
        }
    }

    let materialize_locals = |indices: &[usize]| -> Vec<(String, Expr)> {
        indices
            .iter()
            .filter_map(|&idx| match &eq_items[idx] {
                EqItem::Local { name, rhs, .. } => Some((name.clone(), rhs.clone())),
                _ => None,
            })
            .collect()
    };

    let mut body_stmts = Vec::new();
    for (c_idx, class_filter) in partition.explicit_classes.into_iter().enumerate() {
        if let Some((loc_len, item_idx)) = explicit_outputs[c_idx]
            && let Some((target, _)) = target_for_kind(&eq_items[item_idx], kind)
        {
            let locals = materialize_locals(&class_locals[c_idx][..loc_len]);
            body_stmts.push((class_filter, locals, target));
        }
    }
    let any_explicit_branch = !body_stmts.is_empty();
    for (loc_len, item_idx, was_any) in outside_outputs.into_iter().rev() {
        let local_slice = &class_locals[outside_class][..loc_len];
        if any_explicit_branch && was_any && branch_uses_unbound_scoped_indices(&eq_items, local_slice, item_idx, kind, &scoped_local_names) {
            continue;
        }
        if let Some((target, _)) = target_for_kind(&eq_items[item_idx], kind) {
            let locals = materialize_locals(local_slice);
            body_stmts.push((partition.outside_filter, locals, target));
            break;
        }
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

fn canonicalize_init_expr(expr: &mut Expr, any_slots: &HashMap<String, (usize, usize)>, specific_slots: Option<&HashMap<String, (usize, usize)>>) {
    let mut stack = vec![expr];
    while let Some(node) = stack.pop() {
        match node {
            Expr::Num(_) => {}
            Expr::Ident(name) => {
                let is_shadowed = any_slots.contains_key(name.as_str()) || specific_slots.is_some_and(|m| m.contains_key(name.as_str()));
                if !is_shadowed && let Some(&canon) = INIT_STAT_NAMES.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
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
    param_names: &[String],
    param_map: &HashMap<String, usize>,
    total_opcodes: &mut usize,
) -> Result<(Vec<CompiledInitRule>, usize), FitError> {
    if raw_init_lines.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let stat_map: HashMap<String, usize> = INIT_STAT_NAMES.iter().enumerate().map(|(i, &n)| (n.to_owned(), i)).collect();
    let mut any_slots: HashMap<String, (usize, usize)> = HashMap::new();
    let mut scoped_slots: HashMap<DatasetFilter, HashMap<String, (usize, usize)>> = HashMap::new();
    let total_slots = raw_init_lines.len();
    let mut compiled = Vec::with_capacity(total_slots);

    for (rule_idx, (line_no, filter, name, mut rhs)) in raw_init_lines.into_iter().enumerate() {
        let specific = if filter == DatasetFilter::Any { None } else { scoped_slots.get(&filter) };
        canonicalize_init_expr(&mut rhs, &any_slots, specific);
        let resolver = LocalResolver::Scoped { any: &any_slots, specific };
        let remaining = MAX_TOTAL_OPCODES.saturating_sub(*total_opcodes);
        let prog = compile_target_resolved(&rhs, &resolver, &stat_map, remaining).map_err(|e| with_line(e, line_no))?;
        *total_opcodes = total_opcodes.saturating_add(prog.code.len());
        if *total_opcodes > MAX_TOTAL_OPCODES {
            return Err(FitError::DslCompileError {
                message: format!("[InitialValues] exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                line: line_no,
                column: 1,
            });
        }
        ensure_no_data_vars(&prog, "[InitialValues]", line_no)?;
        let param_idx = param_map.get(&name).copied().or_else(|| param_names.iter().position(|p| p.eq_ignore_ascii_case(&name)));
        let slot = rule_idx;
        if filter == DatasetFilter::Any {
            any_slots.insert(name, (slot, rule_idx));
        } else {
            scoped_slots.entry(filter.clone()).or_default().insert(name, (slot, rule_idx));
        }
        compiled.push(CompiledInitRule { filter, slot, param_idx, prog });
    }
    Ok((compiled, total_slots))
}

type CompiledTransforms = Vec<(String, usize, CompiledProgram)>;

fn compile_transforms(
    raw_transform_lines: Vec<(usize, String, Expr)>,
    param_map: &HashMap<String, usize>,
    total_opcodes: &mut usize,
) -> Result<(CompiledTransforms, usize), FitError> {
    if raw_transform_lines.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut compiled = Vec::with_capacity(raw_transform_lines.len());
    for (line_no, name, rhs) in raw_transform_lines {
        let resolver = LocalResolver::Flat(&local_map);
        let remaining = MAX_TOTAL_OPCODES.saturating_sub(*total_opcodes);
        let prog = compile_target_resolved(&rhs, &resolver, param_map, remaining).map_err(|e| with_line(e, line_no))?;
        *total_opcodes = total_opcodes.saturating_add(prog.code.len());
        if *total_opcodes > MAX_TOTAL_OPCODES {
            return Err(FitError::DslCompileError {
                message: format!("[Transforms] exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
                line: line_no,
                column: 1,
            });
        }
        ensure_no_data_vars(&prog, "[Transforms]", line_no)?;
        let slot = if let Some(&s) = local_map.get(&name) {
            s
        } else {
            let s = local_map.len();
            local_map.insert(name.clone(), s);
            s
        };
        compiled.push((name, slot, prog));
    }
    Ok((compiled, local_map.len()))
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

enum LocalResolver<'a> {
    Flat(&'a HashMap<String, usize>),
    Scoped { any: &'a HashMap<String, (usize, usize)>, specific: Option<&'a HashMap<String, (usize, usize)>> },
}

impl LocalResolver<'_> {
    fn slot(&self, name: &str) -> Option<usize> {
        match self {
            Self::Flat(map) => map.get(name).copied(),
            Self::Scoped { any, specific } => match (any.get(name), specific.and_then(|s| s.get(name))) {
                (Some(&(s1, o1)), Some(&(s2, o2))) => Some(if o2 >= o1 { s2 } else { s1 }),
                (Some(&(s, _)), None) | (None, Some(&(s, _))) => Some(s),
                (None, None) => None,
            },
        }
    }
}

fn check_opcode_budget(code: &[OpCode], max_opcodes: usize) -> Result<(), FitError> {
    if code.len() > max_opcodes {
        return Err(FitError::DslCompileError {
            message: format!("Compiled equation exceeds total bytecode budget ({MAX_TOTAL_OPCODES} opcodes)"),
            line: 1,
            column: 1,
        });
    }
    Ok(())
}

fn compile_program_refs(
    locals: &[(String, Expr)],
    target: &Expr,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    let mut local_map: HashMap<String, usize> = HashMap::new();
    let mut code = Vec::new();
    for (lname, rhs) in locals {
        let resolver = LocalResolver::Flat(&local_map);
        compile_expr_resolved(rhs, &resolver, params, &mut code, max_opcodes)?;
        let slot = if let Some(&s) = local_map.get(lname) {
            s
        } else {
            let s = local_map.len();
            local_map.insert(lname.clone(), s);
            s
        };
        code.push(OpCode::StoreLocal(slot));
        check_opcode_budget(&code, max_opcodes)?;
    }
    let resolver = LocalResolver::Flat(&local_map);
    compile_expr_resolved(target, &resolver, params, &mut code, max_opcodes)?;
    code.push(OpCode::Halt);
    check_opcode_budget(&code, max_opcodes)?;
    Ok(CompiledProgram { code, local_count: local_map.len() })
}

fn compile_target_resolved(
    target: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    max_opcodes: usize,
) -> Result<CompiledProgram, FitError> {
    let mut code = Vec::new();
    compile_expr_resolved(target, locals, params, &mut code, max_opcodes)?;
    code.push(OpCode::Halt);
    check_opcode_budget(&code, max_opcodes)?;
    Ok(CompiledProgram { code, local_count: 0 })
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
    locals: &LocalResolver<'_>,
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

#[cfg(test)]
fn compile_expr(expr: &Expr, locals: &HashMap<String, usize>, params: &HashMap<String, usize>, code: &mut Vec<OpCode>) -> Result<(), FitError> {
    compile_expr_resolved(expr, &LocalResolver::Flat(locals), params, code, MAX_TOTAL_OPCODES)
}

fn compile_expr_resolved(
    expr: &Expr,
    locals: &LocalResolver<'_>,
    params: &HashMap<String, usize>,
    code: &mut Vec<OpCode>,
    max_opcodes: usize,
) -> Result<(), FitError> {
    let mut tasks = vec![CompileTask::Eval(expr)];
    while let Some(task) = tasks.pop() {
        check_opcode_budget(code, max_opcodes)?;
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
    check_opcode_budget(code, max_opcodes)
}

fn compile_ident(name: &str, locals: &LocalResolver<'_>, params: &HashMap<String, usize>, code: &mut Vec<OpCode>) -> Result<(), FitError> {
    if let Some(slot) = locals.slot(name) {
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
    let mut locals = vec![None; prog.local_count];
    eval_bytecode_with_locals(prog, x, y, params, ds_idx, &mut locals)
}

fn eval_bytecode_with_locals(prog: &CompiledProgram, x: &[f64], y: f64, params: &[f64], ds_idx: usize, locals: &mut [Option<f64>]) -> Option<f64> {
    let mut stack: Vec<f64> = Vec::with_capacity(32);
    eval_bytecode_with_bufs(prog, x, y, params, ds_idx, locals, &mut stack)
}

fn eval_bytecode_with_bufs(
    prog: &CompiledProgram,
    x: &[f64],
    y: f64,
    params: &[f64],
    ds_idx: usize,
    locals: &mut [Option<f64>],
    stack: &mut Vec<f64>,
) -> Option<f64> {
    stack.clear();
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
                stack.push(*x.get(idx)?);
            }
            OpCode::LoadY => stack.push(y),
            OpCode::LoadDatasetIndex => stack.push(ds_idx as f64),
            OpCode::LoadParam(idx) => stack.push(*params.get(idx)?),
            OpCode::LoadLocal(slot) => stack.push(locals.get(slot).copied().flatten()?),
            OpCode::StoreLocal(slot) => {
                let v = stack.pop()?;
                *locals.get_mut(slot)? = Some(v);
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
// Pure-Rust Clean-Room Bessel & Hypergeometric Special Functions
// (J/Y: NIST DLMF §10 ascending series in 106-bit compensated DoubleDouble,
//  Hankel asymptotic expansions, upward Y_n recurrence, and Steed/Lentz CF1 +
//  Wronskian normalization; I/K: A&S 9.8.1-9.8.8; hypergeometric: DLMF §13/§15)
// ============================================================================

const BESSEL_ASYMP_X: f64 = 20.0;

#[derive(Clone, Copy)]
struct Dd {
    hi: f64,
    lo: f64,
}

impl Dd {
    const ZERO: Self = Self { hi: 0.0, lo: 0.0 };
    const ONE: Self = Self { hi: 1.0, lo: 0.0 };

    #[inline]
    const fn from_f64(x: f64) -> Self {
        Self { hi: x, lo: 0.0 }
    }

    #[inline]
    fn to_f64(self) -> f64 {
        self.hi + self.lo
    }

    #[inline]
    fn two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let v = s - a;
        let e = (a - (s - v)) + (b - v);
        (s, e)
    }

    #[inline]
    fn quick_two_sum(a: f64, b: f64) -> (f64, f64) {
        let s = a + b;
        let e = b - (s - a);
        (s, e)
    }

    #[inline]
    fn add(self, rhs: Self) -> Self {
        let (s1, s2) = Self::two_sum(self.hi, rhs.hi);
        let (t1, t2) = Self::two_sum(self.lo, rhs.lo);
        let (v_hi, v_lo) = Self::quick_two_sum(s1, s2 + t1);
        let (hi, lo) = Self::quick_two_sum(v_hi, t2 + v_lo);
        Self { hi, lo }
    }

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        self.add(Self { hi: -rhs.hi, lo: -rhs.lo })
    }

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        let p1 = self.hi * rhs.hi;
        let p2 = self.hi.mul_add(rhs.hi, -p1);
        let c = p2 + self.hi * rhs.lo + self.lo * rhs.hi;
        let (hi, lo) = Self::quick_two_sum(p1, c);
        Self { hi, lo }
    }

    #[inline]
    fn div(self, rhs: Self) -> Self {
        let q1 = self.hi / rhs.hi;
        let r = self.sub(Self::from_f64(q1).mul(rhs));
        let q2 = r.hi / rhs.hi;
        let r2 = r.sub(Self::from_f64(q2).mul(rhs));
        let q3 = r2.hi / rhs.hi;
        let (s1, s2) = Self::quick_two_sum(q1, q2);
        Self { hi: s1, lo: s2 }.add(Self::from_f64(q3))
    }
}

struct BesselDdConsts {
    pi_2: Dd,
    pi_4: Dd,
    two_over_pi: Dd,
    ln2: Dd,
    euler_gamma: Dd,
}

static BESSEL_DD_CONSTS: OnceLock<BesselDdConsts> = OnceLock::new();

fn atan_recip_dd(inv_x: f64) -> Dd {
    let x = Dd::ONE.div(Dd::from_f64(inv_x));
    let x2 = x.mul(x);
    let mut term = x;
    let mut sum = x;
    for k in 1..40_u32 {
        term = term.mul(x2);
        let delta = term.div(Dd::from_f64(f64::from(2 * k + 1)));
        sum = if k % 2 == 1 { sum.sub(delta) } else { sum.add(delta) };
        if delta.hi.abs() <= 1e-34 {
            break;
        }
    }
    sum
}

fn bessel_dd_consts() -> &'static BesselDdConsts {
    BESSEL_DD_CONSTS.get_or_init(|| {
        // Machin's identity: pi / 4 = 4 * atan(1 / 5) - atan(1 / 239).
        let pi_4 = Dd::from_f64(4.0).mul(atan_recip_dd(5.0)).sub(atan_recip_dd(239.0));
        let pi_2 = pi_4.mul(Dd::from_f64(2.0));
        let pi = pi_2.mul(Dd::from_f64(2.0));
        let two_over_pi = Dd::from_f64(2.0).div(pi);

        // ln(2) = 2 * atanh(1 / 3) = 2 * sum_{k>=0} 1 / ((2k + 1) * 3^(2k + 1)).
        let inv9 = Dd::ONE.div(Dd::from_f64(9.0));
        let mut p = Dd::ONE.div(Dd::from_f64(3.0));
        let mut ln2_half = Dd::ZERO;
        for k in 0..40_u32 {
            let delta = p.div(Dd::from_f64(f64::from(2 * k + 1)));
            ln2_half = ln2_half.add(delta);
            if delta.hi.abs() <= 1e-34 {
                break;
            }
            p = p.mul(inv9);
        }
        let ln2 = ln2_half.mul(Dd::from_f64(2.0));

        // Euler-Mascheroni constant via Euler-Maclaurin at N = 1024 = 2^10 (remainder < 1e-32).
        const N: usize = 1024;
        let mut h_n = Dd::ZERO;
        for k in (1..=N).rev() {
            h_n = h_n.add(Dd::ONE.div(Dd::from_f64(k as f64)));
        }
        let n_dd = Dd::from_f64(N as f64);
        let inv_n = Dd::ONE.div(n_dd);
        let inv_n2 = inv_n.mul(inv_n);
        let inv_n4 = inv_n2.mul(inv_n2);
        let inv_n6 = inv_n4.mul(inv_n2);
        let inv_n8 = inv_n4.mul(inv_n4);
        let ln_n = Dd::from_f64(10.0).mul(ln2);
        let em = inv_n
            .mul(Dd::from_f64(-0.5))
            .add(inv_n2.div(Dd::from_f64(12.0)))
            .sub(inv_n4.div(Dd::from_f64(120.0)))
            .add(inv_n6.div(Dd::from_f64(252.0)))
            .sub(inv_n8.div(Dd::from_f64(240.0)));
        let euler_gamma = h_n.sub(ln_n).add(em);

        BesselDdConsts { pi_2, pi_4, two_over_pi, ln2, euler_gamma }
    })
}

/// 106-bit natural logarithm `ln(u)` for positive finite `u` via power-of-2 reduction into
/// `[sqrt(0.5), sqrt(2)]` and `atanh` series.
fn ln_dd(u: f64, consts: &BesselDdConsts) -> Dd {
    let mut f = u;
    let mut k: i32 = 0;
    while f >= core::f64::consts::SQRT_2 {
        f *= 0.5;
        k += 1;
    }
    while f < core::f64::consts::FRAC_1_SQRT_2 {
        f *= 2.0;
        k -= 1;
    }
    let f_dd = Dd::from_f64(f);
    let t = f_dd.sub(Dd::ONE).div(f_dd.add(Dd::ONE));
    let t2 = t.mul(t);
    let mut pow = t;
    let mut acc = Dd::ZERO;
    for j in 0..24_u32 {
        let term = pow.div(Dd::from_f64(f64::from(2 * j + 1)));
        acc = acc.add(term);
        if term.hi.abs() <= 1e-34 {
            break;
        }
        pow = pow.mul(t2);
    }
    acc.mul(Dd::from_f64(2.0)).add(Dd::from_f64(f64::from(k)).mul(consts.ln2))
}

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

/// Compute `J0(ax)` and optionally `Y0(ax)` for `0 < ax <= BESSEL_ASYMP_X` from DLMF §10.2.2 &
/// §10.8.1 ascending series in 106-bit `DoubleDouble` precision (reaches full `f64` accuracy even
/// across the sign cancellations around `|x| ~ 20` and near zeros).
fn bessel_j0_y0_series(ax: f64, need_y0: bool) -> (f64, Option<f64>) {
    if ax == 0.0 {
        return (1.0, None);
    }
    let z = Dd::from_f64(ax).mul(Dd::from_f64(0.5));
    let neg_z2 = Dd::ZERO.sub(z.mul(z));
    let mut term = Dd::ONE;
    let mut j0_sum = Dd::ONE;
    let mut h_k = Dd::ZERO;
    let mut y0_sum = Dd::ZERO;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        term = term.mul(neg_z2).div(Dd::from_f64(kf * kf));
        j0_sum = j0_sum.add(term);
        if need_y0 {
            h_k = h_k.add(Dd::ONE.div(Dd::from_f64(kf)));
            y0_sum = y0_sum.sub(term.mul(h_k));
        }
        if term.hi.abs() <= 1e-32 * j0_sum.hi.abs().max(1.0) && (!need_y0 || term.hi.abs() * h_k.hi.abs() <= 1e-32) {
            break;
        }
    }

    let j0_val = j0_sum.to_f64();
    if !need_y0 {
        return (j0_val, None);
    }
    let consts = bessel_dd_consts();
    let log_factor = ln_dd(0.5 * ax, consts).add(consts.euler_gamma);
    let y0_val = consts.two_over_pi.mul(log_factor.mul(j0_sum).add(y0_sum)).to_f64();
    (j0_val, y0_val.is_finite().then_some(y0_val))
}

/// Compute `J1(ax)` and optionally `Y1(ax)` for `0 < ax <= BESSEL_ASYMP_X` from DLMF §10.2.2 &
/// §10.8.1 ascending series in 106-bit `DoubleDouble` precision.
fn bessel_j1_y1_series(ax: f64, need_y1: bool) -> (f64, Option<f64>) {
    if ax == 0.0 {
        return (0.0, None);
    }
    let z = Dd::from_f64(ax).mul(Dd::from_f64(0.5));
    let neg_z2 = Dd::ZERO.sub(z.mul(z));
    let mut term = z;
    let mut j1_sum = z;
    let mut h_kp1 = Dd::ONE;
    let mut y1_sum = z;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        let kp1 = f64::from(k + 1);
        term = term.mul(neg_z2).div(Dd::from_f64(kf * kp1));
        j1_sum = j1_sum.add(term);
        if need_y1 {
            let h_k = h_kp1;
            h_kp1 = h_kp1.add(Dd::ONE.div(Dd::from_f64(kp1)));
            y1_sum = y1_sum.add(term.mul(h_k.add(h_kp1)));
        }
        if term.hi.abs() <= 1e-32 * j1_sum.hi.abs().max(ax * 0.5) {
            break;
        }
    }

    let j1_val = j1_sum.to_f64();
    if !need_y1 {
        return (j1_val, None);
    }
    let consts = bessel_dd_consts();
    let log_factor = ln_dd(0.5 * ax, consts).add(consts.euler_gamma);
    let inv_ax = Dd::ONE.div(Dd::from_f64(ax));
    let y1_inner = log_factor.mul(j1_sum).sub(inv_ax).sub(y1_sum.mul(Dd::from_f64(0.5)));
    let y1_val = consts.two_over_pi.mul(y1_inner).to_f64();
    (j1_val, y1_val.is_finite().then_some(y1_val))
}

/// Evaluate Hankel's asymptotic polynomials `P(nu, x)` and `Q(nu, x)` (DLMF §10.17.1–§10.17.4)
/// for `ax > BESSEL_ASYMP_X = 20.0`, where optimal truncation error is `< 1e-16`.
fn hankel_pq(mu_4nu2: f64, ax: f64) -> (f64, f64) {
    let mut p_acc = NeumaierSum::new();
    let mut q_acc = NeumaierSum::new();
    p_acc.add(1.0);
    let mut term = 1.0_f64;
    let mut prev_abs = f64::INFINITY;

    for k in 1..=64_u32 {
        let kf = f64::from(k);
        let odd = 2.0 * kf - 1.0;
        let factor = (mu_4nu2 - odd * odd) / (8.0 * kf * ax);
        term *= factor;
        let cur_abs = term.abs();
        if cur_abs > prev_abs && k > 4 {
            break;
        }
        prev_abs = cur_abs;
        if k % 2 == 1 {
            let sign = if ((k - 1) / 2) % 2 == 0 { 1.0 } else { -1.0 };
            q_acc.add(sign * term);
        } else {
            let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
            p_acc.add(sign * term);
        }
        if cur_abs <= 0.25 * f64::EPSILON {
            break;
        }
    }
    (p_acc.total(), q_acc.total())
}

/// Compute `(cos(ax - pi/4), sin(ax - pi/4))` without losing phase precision at large `ax`.
fn hankel_phase_nu0(ax: f64) -> (f64, f64) {
    if ax <= 1e12 {
        let consts = bessel_dd_consts();
        let q_f64 = ((ax - core::f64::consts::FRAC_PI_4) * core::f64::consts::FRAC_2_PI).round();
        let r = Dd::from_f64(ax).sub(consts.pi_4).sub(Dd::from_f64(q_f64).mul(consts.pi_2)).to_f64();
        let (sr, cr) = r.sin_cos();
        let quad = (q_f64 as i64).rem_euclid(4);
        match quad {
            0 => (cr, sr),
            1 => (-sr, cr),
            2 => (-cr, -sr),
            _ => (sr, -cr),
        }
    } else {
        let (s, c) = ax.sin_cos();
        ((c + s) * core::f64::consts::FRAC_1_SQRT_2, (s - c) * core::f64::consts::FRAC_1_SQRT_2)
    }
}

/// Bessel `J0(x)`: 106-bit compensated ascending series (DLMF §10.2.2) for `|x| <= 20` and Hankel
/// asymptotic expansion (DLMF §10.17.3) with `DoubleDouble` phase reduction for `|x| > 20`.
/// Reaches near-`f64` accuracy (`~1e-15` relative error away from zeros, `~1e-16` absolute near zeros).
fn bessel_j0(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    if ax <= BESSEL_ASYMP_X {
        bessel_j0_y0_series(ax, false).0
    } else {
        let (p, q) = hankel_pq(0.0, ax);
        let (cos0, sin0) = hankel_phase_nu0(ax);
        (core::f64::consts::FRAC_2_PI / ax).sqrt() * (cos0 * p - sin0 * q)
    }
}

/// Bessel `J1(x)`: 106-bit compensated ascending series (DLMF §10.2.2) for `|x| <= 20` and Hankel
/// asymptotic expansion (DLMF §10.17.3) for `|x| > 20`. Near-`f64` accuracy.
fn bessel_j1(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let ax = x.abs();
    let mag = if ax <= BESSEL_ASYMP_X {
        bessel_j1_y1_series(ax, false).0
    } else {
        let (p, q) = hankel_pq(4.0, ax);
        let (cos0, sin0) = hankel_phase_nu0(ax);
        // chi_1 = chi_0 - pi/2 => cos(chi_1) = sin(chi_0), sin(chi_1) = -cos(chi_0).
        (core::f64::consts::FRAC_2_PI / ax).sqrt() * (sin0 * p + cos0 * q)
    };
    if x < 0.0 { -mag } else { mag }
}

/// Steed / modified Lentz continued fraction `f_n = J'_n(x) / J_n(x)` for `0 < x < n`
/// (DLMF §10.10.1 + §10.6.2): `f_n = n/x - 1 / (2(n+1)/x - 1 / (2(n+2)/x - ...))`.
fn bessel_cf1_jprime_over_j(n: i32, ax: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let nf = f64::from(n);
    let mut f = nf / ax;
    let mut c = f;
    let mut d = 0.0_f64;
    for k in 1..=10_000_usize {
        let b_k = 2.0 * (nf + k as f64) / ax;
        d = b_k - d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b_k - 1.0 / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() <= 0.5 * f64::EPSILON {
            break;
        }
    }
    f
}

/// Integer-order `J_n(x)` from NIST DLMF §10:
/// - Upward recurrence from `J0`, `J1` in the oscillatory regime `|x| >= n` (DLMF §10.6.1);
/// - For `0 < |x| < n`: small-argument series (`|x| < 1e-10`), DLMF §10.14.4 early underflow bound,
///   or Steed/Lentz continued fraction `J'_n / J_n` (DLMF §10.10.1) + power-of-2-scaled upward `Y_k`
///   recurrence + Wronskian normalization `J_n = 2 / (pi * x * (Y'_n - f_n Y_n))` (DLMF §10.5.2).
///
/// Achieves near-`f64` accuracy (`~1e-15`–`1e-12` depending on `n`) without Miller downward loops.
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
    let nf = f64::from(n);
    let ans = if ax >= nf {
        let tox = 2.0 / ax;
        let mut bjm = bessel_j0(ax);
        let mut bj = bessel_j1(ax);
        for j in 1..n {
            let bjp = (f64::from(j) * tox).mul_add(bj, -bjm);
            bjm = bj;
            bj = bjp;
        }
        bj
    } else {
        bessel_jn_evanescent(n, nf, ax)
    };
    if x < 0.0 && n % 2 != 0 { -ans } else { ans }
}

fn bessel_jn_evanescent(n: i32, nf: f64, ax: f64) -> f64 {
    // DLMF §10.14.4 upper bound |J_n(x)| <= (x / 2)^n / n!: guarantees exact f64 underflow to 0.
    let Ok(lg_np1) = ln_gamma(nf + 1.0) else {
        return 0.0;
    };
    let ln_lead = nf * (0.5 * ax).ln() - lg_np1;
    if ln_lead < -746.0 {
        return 0.0;
    }
    if ax < 1e-10 {
        let z2 = 0.25 * ax * ax;
        let poly = 1.0 - (z2 / (nf + 1.0)) * (1.0 - z2 / (2.0 * (nf + 2.0)));
        return ln_lead.exp() * poly;
    }

    let f_n = bessel_cf1_jprime_over_j(n, ax);
    let (Some(mut bym), Some(mut by)) = (bessel_y0(ax), bessel_y1(ax)) else {
        return 0.0;
    };
    let mut scale_exp2: i32 = 0;
    const RESCALE_LIMIT: f64 = 1e100;
    // Exact 2^-512 power-of-2 scaling introduces zero mantissa rounding error.
    let scale_down = 2.0_f64.powi(-512);
    for j in 1..n {
        let byp = (f64::from(2 * j) / ax).mul_add(by, -bym);
        bym = by;
        by = byp;
        if by.abs() > RESCALE_LIMIT {
            by *= scale_down;
            bym *= scale_down;
            scale_exp2 = scale_exp2.saturating_add(512);
            if scale_exp2 > 2_000 {
                return 0.0;
            }
        }
    }
    let y_prime_scaled = bym - (nf / ax) * by;
    let denom_scaled = y_prime_scaled - f_n * by;
    let mut val = (core::f64::consts::FRAC_2_PI / ax) / denom_scaled;
    let mut rem = scale_exp2;
    while rem > 0 && val != 0.0 {
        let step = rem.min(512);
        val *= 2.0_f64.powi(-step);
        rem -= step;
    }
    val
}

/// Bessel `Y0(x)` for `x > 0`: 106-bit compensated DLMF §10.8.1 series (`x <= 20`) and Hankel
/// asymptotic expansion (`x > 20`, DLMF §10.17.4). Near-`f64` accuracy.
fn bessel_y0(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    if x <= BESSEL_ASYMP_X {
        bessel_j0_y0_series(x, true).1
    } else {
        let (p, q) = hankel_pq(0.0, x);
        let (cos0, sin0) = hankel_phase_nu0(x);
        let ans = (core::f64::consts::FRAC_2_PI / x).sqrt() * (sin0 * p + cos0 * q);
        ans.is_finite().then_some(ans)
    }
}

/// Bessel `Y1(x)` for `x > 0`: 106-bit compensated DLMF §10.8.1 series (`x <= 20`) and Hankel
/// asymptotic expansion (`x > 20`, DLMF §10.17.4). Near-`f64` accuracy.
fn bessel_y1(x: f64) -> Option<f64> {
    if !x.is_finite() || x <= 0.0 {
        return None;
    }
    if x <= BESSEL_ASYMP_X {
        bessel_j1_y1_series(x, true).1
    } else {
        let (p, q) = hankel_pq(4.0, x);
        let (cos0, sin0) = hankel_phase_nu0(x);
        // chi_1 = chi_0 - pi/2 => sin(chi_1) = -cos(chi_0), cos(chi_1) = sin(chi_0).
        let ans = (core::f64::consts::FRAC_2_PI / x).sqrt() * (-cos0 * p + sin0 * q);
        ans.is_finite().then_some(ans)
    }
}

/// Integer-order `Y_n(x)` by (stable) upward recurrence from `Y0`, `Y1` (DLMF §10.6.1 / A&S 9.1.27),
/// returning `None` if `x <= 0` or the dominant solution overflows `f64`.
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
        let byp = (f64::from(j) * tox).mul_add(by, -bym);
        bym = by;
        by = byp;
        if !by.is_finite() {
            return None;
        }
    }
    Some(by)
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

fn is_non_positive_integer(v: f64) -> bool {
    v <= 0.0 && (v - v.round()).abs() < 1e-12
}

/// Optimal-truncation asymptotic series `2F0(a, a - b + 1; ; inv_z)` (DLMF §13.7.1–§13.7.3)
/// where `inv_z = -1 / z` (`-1 / z < 0` for `U(a, b, z)` at `z > 0`, and `-1 / z > 0` for
/// `1F1(a; b; z)` at `z < 0`). Returns `Some(sum)` only when optimal truncation reaches
/// `|last_term| <= 1e-13 * |sum|` and alternating cancellation remains `<= 1e10`.
fn hyp2f0_asymp(a: f64, b: f64, inv_z: f64) -> Option<f64> {
    const MAX_TERMS: usize = 128;
    let c2 = a - b + 1.0;
    let mut sum = NeumaierSum::new();
    sum.add(1.0);
    let mut term = 1.0_f64;
    let mut prev_abs = 1.0_f64;
    let mut max_abs = 1.0_f64;

    for k in 0..MAX_TERMS {
        let kf = k as f64;
        let factor = ((a + kf) * (c2 + kf) / (kf + 1.0)) * inv_z;
        let next = term * factor;
        if !next.is_finite() {
            return None;
        }
        if next == 0.0 {
            let total = sum.total();
            return (total.is_finite() && max_abs <= 1e10 * total.abs().max(f64::MIN_POSITIVE)).then_some(total);
        }
        let next_abs = next.abs();
        let cur_total = sum.total();
        if k >= 2 && next_abs > prev_abs {
            return (cur_total.is_finite() && prev_abs <= 1e-13 * cur_total.abs().max(f64::MIN_POSITIVE) && max_abs <= 1e10 * cur_total.abs())
                .then_some(cur_total);
        }
        term = next;
        prev_abs = next_abs;
        max_abs = max_abs.max(next_abs);
        sum.add(term);
        let updated = sum.total();
        if !updated.is_finite() {
            return None;
        }
        max_abs = max_abs.max(updated.abs());
        if k >= 2 && next_abs <= 1e-15 * updated.abs().max(f64::MIN_POSITIVE) {
            return (max_abs <= 1e10 * updated.abs().max(f64::MIN_POSITIVE)).then_some(updated);
        }
    }
    None
}

/// Confluent hypergeometric function `1F1(a; b; z)` (Kummer's `M(a, b, z)`, DLMF §13.2 & §13.7):
///
/// - Rejects non-positive integer poles `b in {0, -1, -2, ...}` (`None`);
/// - Uses exact identities `1F1(0; b; z) = 1`, `1F1(a; b; 0) = 1`, `1F1(b; b; z) = exp(z)`;
/// - Uses Kummer's transformation `1F1(a; b; z) = exp(z) 1F1(b - a; b; -z)` for `z < -1`
///   (DLMF §13.2.39), falling back for large negative `z <= -30` when the transformed series
///   overflows (`z < -709`) or suffers `> 1e12` cancellation to DLMF §13.7.2 asymptotic
///   `(Gamma(b)/Gamma(b-a)) (-z)^(-a) 2F0(a, a-b+1; ; -1/z)` whenever the `exp(z)` branch is
///   `< 1e-16` relative to the algebraic branch and optimal truncation error is `<= 1e-13`;
/// - Uses Neumaier-compensated Kummer power series (up to `4000` terms) elsewhere, returning `None`
///   if `f64` overflows or cancellation exceeds `1e12` (`error <= ~1e-13` relative when accepted).
fn hyp1f1(a: f64, b: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b.is_finite() || !z.is_finite() || is_non_positive_integer(b) {
        return None;
    }
    if z == 0.0 || a == 0.0 {
        return Some(1.0);
    }
    if (a - b).abs() == 0.0 {
        let v = z.exp();
        return v.is_finite().then_some(v);
    }
    if z < -1.0 {
        if let Some(transformed) = hyp1f1_series(b - a, b, -z) {
            let scaled = z.exp() * transformed;
            if scaled.is_finite() {
                return Some(scaled);
            }
        }
        if z <= -30.0
            && let Some(asymp) = hyp1f1_large_neg_z(a, b, z)
        {
            return Some(asymp);
        }
    }
    hyp1f1_series(a, b, z)
}

fn hyp1f1_large_neg_z(a: f64, b: f64, z: f64) -> Option<f64> {
    // At z <= -30, the algebraic term dominates the exp(z) term only when b - a is not a pole
    // (if b - a <= 0 integer, M(b-a, b, -z) is a polynomial) and the log-ratio < -37 (< 1e-16).
    let bma = b - a;
    if is_non_positive_integer(bma) {
        return None;
    }
    let neg_z = -z;
    if !is_non_positive_integer(a) {
        let gbma = gamma(bma).ok()?;
        let ga = gamma(a).ok()?;
        let log_exp_over_alg = (gbma.abs() / ga.abs()).ln() + z + (2.0 * a - b) * neg_z.ln();
        if log_exp_over_alg > -37.0 {
            return None;
        }
    }
    let asymp_series = hyp2f0_asymp(a, b, -1.0 / z)?;
    let gb = gamma(b).ok()?;
    let gbma = gamma(bma).ok()?;
    let val = (gb / gbma) * neg_z.powf(-a) * asymp_series;
    val.is_finite().then_some(val)
}

fn hyp1f1_series(a: f64, b: f64, z: f64) -> Option<f64> {
    const MAX_ITERS: usize = 4_000;
    let mut term = 1.0_f64;
    let mut sum = NeumaierSum::new();
    sum.add(1.0);
    let mut max_abs = 1.0_f64;
    let mut prev_small = false;
    for k in 0..MAX_ITERS {
        let kf = k as f64;
        let denom = b + kf;
        if denom == 0.0 {
            return None;
        }
        let factor = ((a + kf) / denom) * (z / (kf + 1.0));
        term *= factor;
        if !term.is_finite() {
            return None;
        }
        if term == 0.0 {
            let total = sum.total();
            return if max_abs > 1e12 * total.abs().max(1.0) { None } else { Some(total) };
        }
        max_abs = max_abs.max(term.abs());
        sum.add(term);
        let total = sum.total();
        if !total.is_finite() {
            return None;
        }
        max_abs = max_abs.max(total.abs());
        let is_small = term.abs() <= 1e-14 * total.abs().max(f64::MIN_POSITIVE);
        let past_growth = (a + kf > 0.0 || kf + 1.0 > z.abs()) && b + kf > 0.0 && factor.abs() < 0.9;
        if k >= 1 && is_small && prev_small && past_growth {
            return if max_abs > 1e12 * total.abs().max(f64::MIN_POSITIVE) { None } else { Some(total) };
        }
        prev_small = is_small;
    }
    None
}

/// Gauss hypergeometric function `2F1(a, b; c; z)` for `z < 1` via Pfaff continuation (`z < -0.5`, A&S 15.3.4)
/// and the Gauss series (A&S 15.1.1), returning `None` on non-convergence.
fn hyp2f1(a: f64, b: f64, c: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b.is_finite() || !c.is_finite() || !z.is_finite() || z >= 1.0 || (c <= 0.0 && (c - c.round()).abs() < 1e-12) {
        return None;
    }
    if z == 0.0 || a == 0.0 || b == 0.0 {
        return Some(1.0);
    }
    if z < -0.5 {
        let w = z / (z - 1.0);
        if w.is_finite() && w < 1.0 {
            let one_minus_z = 1.0 - z;
            if let Some(series_val) = hyp2f1_series(a, c - b, c, w) {
                let res = one_minus_z.powf(-a) * series_val;
                if res.is_finite() {
                    return Some(res);
                }
            }
            if let Some(series_val) = hyp2f1_series(c - a, b, c, w) {
                let res = one_minus_z.powf(-b) * series_val;
                if res.is_finite() {
                    return Some(res);
                }
            }
        }
        if z <= -1.0 {
            return None;
        }
    }
    hyp2f1_series(a, b, c, z)
}

fn hyp2f1_series(a: f64, b: f64, c: f64, z: f64) -> Option<f64> {
    const MAX_ITERS: usize = 50_000;
    let mut term = 1.0_f64;
    let mut sum = 1.0_f64;
    let mut max_abs = 1.0_f64;
    let mut prev_small = false;
    for k in 0..MAX_ITERS {
        let kf = k as f64;
        let denom = c + kf;
        if denom == 0.0 {
            return None;
        }
        let factor = ((a + kf) * (b + kf) / denom) * (z / (kf + 1.0));
        term *= factor;
        if !term.is_finite() {
            return None;
        }
        if term == 0.0 {
            return if max_abs > 1e12 * sum.abs().max(1.0) { None } else { Some(sum) };
        }
        max_abs = max_abs.max(term.abs());
        sum += term;
        if !sum.is_finite() {
            return None;
        }
        max_abs = max_abs.max(sum.abs());
        let is_small = term.abs() <= 1e-14 * sum.abs().max(f64::MIN_POSITIVE);
        let past_pole = c + kf > 0.0 && (a + kf > 0.0 || b + kf > 0.0 || kf + 1.0 > a.abs().max(b.abs()));
        if k >= 1 && is_small && prev_small && past_pole && factor.abs() < 0.9999 {
            return if max_abs > 1e12 * sum.abs().max(f64::MIN_POSITIVE) { None } else { Some(sum) };
        }
        prev_small = is_small;
    }
    None
}

/// Exact terminating polynomial `U(-m, b, z)` for integer `m >= 0` (DLMF §13.3.7 recurrence).
fn confluent_u_neg_int_a(m: usize, b: f64, z: f64) -> Option<f64> {
    if m == 0 {
        return Some(1.0);
    }
    let mut u_prev = 1.0_f64;
    let mut u_cur = z - b;
    for k in 1..m {
        let kf = k as f64;
        let u_next = (z - b - 2.0 * kf) * u_cur - kf * (b + kf - 1.0) * u_prev;
        if !u_next.is_finite() {
            return None;
        }
        u_prev = u_cur;
        u_cur = u_next;
    }
    u_cur.is_finite().then_some(u_cur)
}

/// Exact integer-`b` logarithmic/digamma series (DLMF §13.2.9 / A&S 13.1.6) for `b = n + 1 >= 1`
/// and moderate `z in (0, 4]`, replacing the unverified `b + 1e-7` perturbation.
fn confluent_u_integer_b_series(a: f64, n: usize, z: f64) -> Option<f64> {
    let nf = n as f64;
    if a <= 0.0 && a.fract() == 0.0 {
        return None;
    }
    let ga = gamma(a).ok()?;
    let g_an = gamma(a - nf).ok()?;
    let psi_a0 = digamma(a).ok()?;
    let mut psi_a = psi_a0;
    let mut psi_1 = -core::f64::consts::EULER_GAMMA;
    let mut psi_np1 = -core::f64::consts::EULER_GAMMA;
    for j in 1..=n {
        psi_np1 += 1.0 / (j as f64);
    }

    let mut fact_n = 1.0_f64;
    for j in 2..=n {
        fact_n *= j as f64;
    }

    // Infinite series: sum_{k>=0} ((a)_k / ((n+1)_k k!)) * (psi(a+k) - psi(1+k) - psi(1+n+k)) z^k
    let mut poch_term = 1.0_f64;
    let mut m_sum = NeumaierSum::new();
    let mut psi_sum = NeumaierSum::new();
    m_sum.add(1.0);
    psi_sum.add(psi_a - psi_1 - psi_np1);
    let mut max_psi_abs = (psi_a - psi_1 - psi_np1).abs();

    for k in 0..512_usize {
        let kf = k as f64;
        poch_term *= ((a + kf) / (nf + 1.0 + kf)) * (z / (kf + 1.0));
        if !poch_term.is_finite() {
            return None;
        }
        psi_a += 1.0 / (a + kf);
        psi_1 += 1.0 / (kf + 1.0);
        psi_np1 += 1.0 / (nf + 1.0 + kf);
        let d_psi = psi_a - psi_1 - psi_np1;
        let psi_term = poch_term * d_psi;
        if !psi_term.is_finite() {
            return None;
        }
        max_psi_abs = max_psi_abs.max(psi_term.abs());
        m_sum.add(poch_term);
        psi_sum.add(psi_term);
        if poch_term.abs() <= 1e-15 * m_sum.total().abs().max(1.0) && psi_term.abs() <= 1e-15 * psi_sum.total().abs().max(1.0) {
            break;
        }
    }

    let sign = if n.is_multiple_of(2) { -1.0 } else { 1.0 };
    let log_part = (sign / (fact_n * g_an)) * (m_sum.total() * z.ln() + psi_sum.total());

    let finite_part = if n == 0 {
        0.0
    } else {
        let fact_nm1 = fact_n / nf;
        let mut f_term = 1.0_f64;
        let mut f_sum = NeumaierSum::new();
        f_sum.add(1.0);
        for k in 0..(n - 1) {
            let kf = k as f64;
            f_term *= ((a - nf + kf) / (1.0 - nf + kf)) * (z / (kf + 1.0));
            f_sum.add(f_term);
        }
        (fact_nm1 / ga) * z.powi(-(n as i32)) * f_sum.total()
    };

    let ans = log_part + finite_part;
    if !ans.is_finite() || (max_psi_abs / (fact_n * g_an).abs() + finite_part.abs()) > 1e8 * ans.abs().max(1e-15) {
        return None;
    }
    Some(ans)
}

/// Strictly positive Laplace integral (DLMF §13.4.4) for `a > 0, z > 0, b in R`:
/// `U(a, b, z) = (z^-a / Gamma(a)) * int_0^inf exp(-u) u^(a-1) (1 + u/z)^(b-a-1) du`,
/// evaluated in log-variable `s = ln(u)` centered on the exact log-integrand peak `u^*` using
/// adaptive 15-point Gauss–Kronrod quadrature (`error <= ~1e-12` relative, zero integer-`b` pole).
fn confluent_u_laplace_pos_a(a: f64, b: f64, z: f64) -> Option<f64> {
    if a <= 0.0 || z <= 0.0 {
        return None;
    }
    let lg_a = ln_gamma(a).ok()?;
    let p = b - a - 1.0;
    let c = b - 1.0 - z;
    let disc = c * c + 4.0 * a * z;
    let u_star = if c >= 0.0 { 0.5 * (c + disc.sqrt()) } else { (2.0 * a * z) / (-c + disc.sqrt()) };
    if !u_star.is_finite() || u_star <= 0.0 {
        return None;
    }
    let s_star = u_star.ln();
    let log_integrand = |s: f64| -> f64 {
        let u = s.exp();
        -u + a * s + p * (u / z).ln_1p()
    };
    let g_max = log_integrand(s_star);
    if !g_max.is_finite() {
        return None;
    }

    let delta = (1e-15 * z.min(1.0) / p.abs().max(1.0)).max(1e-300);
    let s_delta = delta.ln();
    let s_target_lo = s_star - 42.0 / a.min(100.0);
    let (s_min, small_tail) = if s_target_lo >= s_delta {
        let mut slo = s_target_lo;
        for _ in 0..8 {
            if log_integrand(slo) - g_max < -42.0 {
                break;
            }
            slo -= 2.0 / a.min(10.0);
        }
        (slo.max(s_delta), 0.0)
    } else {
        // Integrate [0, delta] analytically as delta^a / a since exp(-u)(1+u/z)^p = 1 + O(1e-15).
        let log_small = a * s_delta - a.ln() - g_max;
        let tail = if log_small < -745.0 { 0.0 } else { log_small.exp() };
        (s_delta, tail)
    };

    let mut s_max = s_star + 1.0;
    for _ in 0..64 {
        if log_integrand(s_max) - g_max < -42.0 {
            break;
        }
        s_max += 1.0;
    }

    let opts = QuadOptions { abs_tol: 1e-14, rel_tol: 1e-12, max_subdivisions: 256 };
    let integral = integrate_adaptive(
        |s| {
            let d = log_integrand(s) - g_max;
            if d < -745.0 { 0.0 } else { d.exp() }
        },
        s_min,
        s_max,
        opts,
    )
    .ok()?;
    let total_scaled = integral + small_tail;
    if !total_scaled.is_finite() || total_scaled <= 0.0 {
        return None;
    }
    let ln_u = -a * z.ln() - lg_a + g_max + total_scaled.ln();
    let val = if ln_u < -745.0 { 0.0 } else { ln_u.exp() };
    val.is_finite().then_some(val)
}

/// Evaluate `U(a, b, z)` for `z > 0` via Laplace quadrature (`a > 0`) or stable downward
/// `a`-recurrence (DLMF §13.3.7) from `a + m > 0` when `a < 0`.
fn confluent_u_via_laplace(a: f64, b: f64, z: f64) -> Option<f64> {
    if a > 0.0 {
        return confluent_u_laplace_pos_a(a, b, z);
    }
    let shift = (1.0 - a).ceil();
    if !shift.is_finite() || shift > 200.0 {
        return None;
    }
    let m = shift as usize;
    let a_hi = a + (m as f64);
    let mut u_hi = confluent_u_laplace_pos_a(a_hi + 1.0, b, z)?;
    let mut u_cur = confluent_u_laplace_pos_a(a_hi, b, z)?;
    for k in (1..=m).rev() {
        let ak = a + (k as f64);
        let u_lo = (z + 2.0 * ak - b) * u_cur - ak * (ak - b + 1.0) * u_hi;
        if !u_lo.is_finite() {
            return None;
        }
        u_hi = u_cur;
        u_cur = u_lo;
    }
    Some(u_cur)
}

/// Tricomi confluent hypergeometric `U(a, b, z)` for `z > 0` (DLMF §13.2, §13.4, §13.7):
///
/// - Exact terminating polynomials for non-positive integer `a` or non-negative integer `b - a - 1`
///   (including `U(a, a + 1, z) = z^(-a)`, exact for all real `a` and `z > 0`);
/// - DLMF §13.7.3 asymptotic expansion `z^(-a) 2F0(a, a - b + 1; ; -1/z)` for `z >= 25` when optimal
///   truncation achieves `<= 1e-13` relative remainder (avoids `exp(+z)` cancellation in `1F1` difference);
/// - Exact DLMF §13.2.9 (A&S 13.1.6) logarithmic/digamma series at integer `b` for small `z <= 4`
///   (with DLMF §13.2.40 reflection `U(a, 1 - m, z) = z^m U(a + m, 1 + m, z)` for `b <= 0`);
/// - DLMF §13.2.42 (A&S 13.1.3) `1F1` connection formula for non-integer `b` (`dist(b, Z) >= 0.05`,
///   `z <= 4`, cancellation `<= 1e8`);
/// - Strictly positive Laplace integral (DLMF §13.4.4) + adaptive Gauss–Kronrod quadrature (plus
///   stable downward `a`-recurrence DLMF §13.3.7 when `a < 0`) for near-integer `b` and moderate `z`.
///
/// Achieves `<= ~1e-12` relative accuracy across all covered `(a, b, z > 0)` domains.
fn confluent_u(a: f64, b_in: f64, z: f64) -> Option<f64> {
    if !a.is_finite() || !b_in.is_finite() || !z.is_finite() || z <= 0.0 {
        return None;
    }
    if a == 0.0 {
        return Some(1.0);
    }
    if a < 0.0 && a.fract() == 0.0 && a >= -200.0 {
        return confluent_u_neg_int_a((-a) as usize, b_in, z);
    }
    let k_poly = b_in - a - 1.0;
    if k_poly >= 0.0 && k_poly.fract() == 0.0 && k_poly <= 200.0 {
        let poly = confluent_u_neg_int_a(k_poly as usize, 2.0 - b_in, z)?;
        let val = z.powf(1.0 - b_in) * poly;
        return val.is_finite().then_some(val);
    }

    if z >= 25.0
        && let Some(asymp) = hyp2f0_asymp(a, b_in, -1.0 / z)
    {
        let val = z.powf(-a) * asymp;
        if val.is_finite() {
            return Some(val);
        }
    }

    if z <= 4.0 {
        if b_in.fract() == 0.0 && b_in.abs() <= 100.0 && a.abs() <= 50.0 {
            if b_in <= 0.0 {
                let m = (1.0 - b_in) as usize;
                if let Some(u_pos) = confluent_u_integer_b_series(a + m as f64, m, z) {
                    let val = z.powi(m as i32) * u_pos;
                    if val.is_finite() {
                        return Some(val);
                    }
                }
            } else {
                let n = (b_in - 1.0) as usize;
                if let Some(val) = confluent_u_integer_b_series(a, n, z) {
                    return Some(val);
                }
            }
        } else if (b_in - b_in.round()).abs() >= 0.05 {
            let pi = core::f64::consts::PI;
            if let (Some(m1), Some(m2), Ok(g1), Ok(g2), Ok(g3), Ok(g4)) =
                (hyp1f1(a, b_in, z), hyp1f1(1.0 + a - b_in, 2.0 - b_in, z), gamma(1.0 + a - b_in), gamma(b_in), gamma(a), gamma(2.0 - b_in))
            {
                let p1 = m1 / (g1 * g2);
                let p2 = z.powf(1.0 - b_in) * m2 / (g3 * g4);
                let diff = p1 - p2;
                let res = (pi / (pi * b_in).sin()) * diff;
                if res.is_finite() && (p1.abs() + p2.abs()) <= 1e8 * diff.abs().max(1e-300) {
                    return Some(res);
                }
            }
        }
    }

    confluent_u_via_laplace(a, b_in, z)
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
