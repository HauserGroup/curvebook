//! Dataset filters, tokens, the expression tree, bytecode opcodes, built-in functions, and compiled program types.
//!
//! References:
//! - Aho, A. V., Lam, M. S., Sethi, R. & Ullman, J. D. (2006). *Compilers: Principles, Techniques, and
//!   Tools*, 2nd ed. Addison-Wesley.

use std::fmt;

/// Dataset-filter predicate parsed from `<A>`, `<~A>`, `<B:D>`, or `<0>` line prefixes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DatasetFilter {
    /// Applies to every dataset in the table (`no <...>` prefix).
    Any,
    /// Applies only when zero-based dataset index is in this inclusion list (e.g. `<A>` -> `[0]`).
    Include(Vec<usize>),
    /// Applies to every dataset except zero-based indices in this exclusion list (e.g. `<~A>` -> `[0]`).
    Exclude(Vec<usize>),
}

impl DatasetFilter {
    /// Return `true` when zero-based dataset index `ds_idx` satisfies this filter predicate.
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
pub(in crate::dsl) enum TokenKind {
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
pub(in crate::dsl) struct Token {
    pub(in crate::dsl) kind: TokenKind,
    pub(in crate::dsl) text: String,
    pub(in crate::dsl) num: f64,
    pub(in crate::dsl) line: usize,
    pub(in crate::dsl) col: usize,
}

pub(crate) enum Expr {
    Num(f64),
    Ident(String),
    Unary { op: UnaryOp, rhs: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
    If { cond: Box<Expr>, then_br: Box<Expr>, else_br: Box<Expr> },
}

impl Expr {
    pub(in crate::dsl) fn is_leaf(&self) -> bool {
        matches!(self, Self::Num(_) | Self::Ident(_))
    }

    pub(in crate::dsl) fn needs_iterative_drop(&self) -> bool {
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
pub(crate) enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum BinaryOp {
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
    /// Push a finite literal `f64` constant onto the operand stack.
    PushConst(f64),
    /// Load predictor coordinate `x[idx]` (for example `X` at slot `0`, or multivariate predictor `X1`, `X2`, ...).
    LoadPredictor(usize),
    /// Load dependent response `Y` (used in implicit `LHS = RHS` or ODE equations).
    LoadY,
    /// Load the active zero-based dataset column index as `f64`.
    LoadDatasetIndex,
    /// Load model parameter value at slot index `idx`.
    LoadParam(usize),
    /// Load temporary local statement variable at slot index `idx`.
    LoadLocal(usize),
    /// Store top of stack into local statement variable slot `idx`.
    StoreLocal(usize),
    /// Apply a unary scalar operator to top of stack.
    Unary(UnaryKind),
    /// Pop two operands and push the result of a binary scalar operator.
    Binary(BinaryKind),
    /// Invoke built-in mathematical function `fn_id` with `argc` stack arguments.
    CallBuiltin(BuiltinFn, usize),
    /// Unconditional jump to instruction pointer offset.
    Jump(usize),
    /// Pop top of stack and jump to offset when equal to `0.0` (`false`).
    JumpIfZero(usize),
    /// Discard top value from operand stack.
    Pop,
    /// Enter a fault-catching scope for `is_defined(expr)` whose handler jumps to instruction offset `target`.
    /// Domain errors, division by zero, `NaN`, or missing predictors before [`OpCode::ExitDefinedScope`] unwind
    /// the operand stack to the saved depth, push `0.0`, and jump to `target`; step-budget exhaustion stays fatal.
    EnterDefinedScope(usize),
    /// Leave the innermost `is_defined(expr)` fault-catching scope normally.
    ExitDefinedScope,
    /// Terminate VM execution and return top of stack.
    Halt,
}

/// Unary scalar operator executed by [`OpCode::Unary`].
#[derive(Debug, Clone, Copy)]
pub enum UnaryKind {
    /// Arithmetic negation (`-x`).
    Neg,
    /// Logical negation (`!x`, returning `1.0` when `x == 0.0` else `0.0`).
    Not,
}

/// Binary scalar arithmetic or relational operator executed by [`OpCode::Binary`].
#[derive(Debug, Clone, Copy)]
pub enum BinaryKind {
    /// Addition (`a + b`).
    Add,
    /// Subtraction (`a - b`).
    Sub,
    /// Multiplication (`a * b`).
    Mul,
    /// Floating-point division (`a / b`).
    Div,
    /// Remainder (`a % b`).
    Rem,
    /// Exponentiation (`a.powf(b)`).
    Pow,
    /// Equality comparison (`a == b` returning `1.0` or `0.0`).
    Eq,
    /// Inequality comparison (`a != b` returning `1.0` or `0.0`).
    Ne,
    /// Strict less-than (`a < b` returning `1.0` or `0.0`).
    Lt,
    /// Less-than-or-equal (`a <= b` returning `1.0` or `0.0`).
    Le,
    /// Strict greater-than (`a > b` returning `1.0` or `0.0`).
    Gt,
    /// Greater-than-or-equal (`a >= b` returning `1.0` or `0.0`).
    Ge,
}

/// Built-in mathematical, statistical, and special function callable inside custom DSL equations.
#[derive(Debug, Clone, Copy)]
pub enum BuiltinFn {
    /// Absolute value `|x|`.
    Abs,
    /// Square root `sqrt(x)` (NaN for `x < 0`).
    Sqrt,
    /// Cube root `cbrt(x)`.
    Cbrt,
    /// Natural exponential `exp(x)`.
    Exp,
    /// Base-10 exponential `10^x`.
    Exp10,
    /// Natural logarithm `ln(x)`.
    Ln,
    /// Base-10 logarithm `log10(x)`.
    Log10,
    /// Base-2 logarithm `log2(x)`.
    Log2,
    /// Trigonometric sine `sin(x)` (radians).
    Sin,
    /// Trigonometric cosine `cos(x)` (radians).
    Cos,
    /// Trigonometric tangent `tan(x)` (radians).
    Tan,
    /// Inverse sine `asin(x)` in `[-pi/2, pi/2]`.
    Asin,
    /// Inverse cosine `acos(x)` in `[0, pi]`.
    Acos,
    /// Inverse tangent `atan(x)` in `[-pi/2, pi/2]`.
    Atan,
    /// Two-argument quadrant-aware inverse tangent `atan2(y, x)`.
    Atan2,
    /// Hyperbolic sine `sinh(x)`.
    Sinh,
    /// Hyperbolic cosine `cosh(x)`.
    Cosh,
    /// Hyperbolic tangent `tanh(x)`.
    Tanh,
    /// Inverse hyperbolic sine `asinh(x)`.
    Asinh,
    /// Inverse hyperbolic cosine `acosh(x)` for `x >= 1`.
    Acosh,
    /// Inverse hyperbolic tangent `atanh(x)` for `|x| < 1`.
    Atanh,
    /// Floor `floor(x)` (largest integer `<= x`).
    Floor,
    /// Ceiling `ceil(x)` (smallest integer `>= x`).
    Ceil,
    /// Nearest integer `round(x)` (ties away from zero).
    Round,
    /// Truncate fractional part toward zero `trunc(x)`.
    Trunc,
    /// Signum (`-1`, `0`, or `+1`).
    Sign,
    /// Variadic minimum `min(a, b, ...)` (2 or more arguments).
    Min,
    /// Variadic maximum `max(a, b, ...)` (2 or more arguments).
    Max,
    /// Clamp `x` to `[lo, hi]` (`clamp(x, lo, hi)`).
    Clamp,
    /// Euclidean norm `sqrt(a^2 + b^2)` (`hypot(a, b)`).
    Hypot,
    /// Gauss error function `erf(x)`.
    Erf,
    /// Complementary error function `erfc(x) = 1 - erf(x)`.
    Erfc,
    /// Standard normal CDF `Phi(x)` (zero mean, unit SD).
    NormCdf,
    /// Standard normal PDF `phi(x)` (zero mean, unit SD).
    NormPdf,
    /// Inverse standard normal CDF `Phi^-1(p)` for `p in (0, 1)`.
    NormInv,
    /// Three-argument normal CDF `Phi((x - mean) / sd)` (`normdist(x, mean, sd)`).
    NormDist3,
    /// Three-argument inverse normal CDF `mean + sd * Phi^-1(p)` (`norminv(p, mean, sd)`).
    NormInv3,
    /// Three-argument normal PDF `phi((x - mean) / sd) / sd` (`normpdf(x, mean, sd)`).
    NormPdf3,
    /// Regularized incomplete beta function `I_x(a, b)` (`ibeta(a, b, x)`).
    IBeta,
    /// Lower regularized incomplete gamma function `P(a, x)` (`igamma(a, x)`).
    IGamma,
    /// Upper regularized incomplete gamma function `Q(a, x)` (`igammac(a, x)`).
    IGammaC,
    /// Student `t` one-tailed upper probability `P(T_df > t)` (`tdist(t, df)`).
    TDist,
    /// Student `t` two-tailed critical quantile `t_{1 - p/2, df}` (`tinv(p, df)`).
    TInv,
    /// Student `t` probability density `f_t(t; df)` (`tpdf(t, df)`).
    TPdf,
    /// Snedecor `F` upper-tail probability `P(F_{df1, df2} > f)` (`fdist(f, df1, df2)`).
    FDist,
    /// Snedecor `F` upper-tail critical quantile `F_{1 - p; df1, df2}` (`finv(p, df1, df2)`).
    FInv,
    /// Snedecor `F` probability density `f_F(f; df1, df2)` (`fpdf(f, df1, df2)`).
    FPdf,
    /// Chi-square upper-tail probability `P(chi^2_df > x2)` (`chidist(x2, df)`).
    ChiDist,
    /// Chi-square lower cumulative probability `P(chi^2_df <= x2)` (`chisqc(x2, df)`).
    ChiSqC,
    /// Chi-square upper-tail critical quantile (`chiinv(p, df)`).
    ChiInv,
    /// Binomial upper cumulative tail `sum_{j = ceil(k)}^{n} C(n, j) p^j (1 - p)^(n - j)` (`binomial(k, n, p)`).
    Binomial,
    /// Factorial `n!` for non-negative integers `n` (`fact(n)`).
    Fact,
    /// Logit log-odds `ln(p / (1 - p))` for `p in (0, 1)` (`logit(p)`).
    Logit,
    /// Finney probit `5 + Phi^-1(p)` for `p in (0, 1)` (`probit(p)`).
    Probit,
    /// Percentage to proportion `x / 100` (`perc(x)`).
    Perc,
    /// Radians to degrees (`deg(rad)`).
    Deg,
    /// Degrees to radians (`rad(deg)`).
    Rad,
    /// Square `x^2` (`sqr(x)`).
    Sqr,
    /// Floating-point remainder `x % y` (`mod(x, y)`).
    Mod,
    /// Finite / non-blank predicate returning `1.0` when `x` is finite and `0.0` otherwise (`is_defined(x)`).
    IsDefined,
    /// Complete Gamma function `Gamma(x)`.
    Gamma,
    /// Natural log of `|Gamma(x)|` (`lngamma(x)`).
    LnGamma,
    /// Euler Beta function `B(a, b)`.
    Beta,
    /// Digamma logarithmic derivative `psi(x) = d/dx ln Gamma(x)`.
    Digamma,
    /// Cylindrical Bessel function of the first kind, order 0 (`J_0(x)`).
    BesselJ0,
    /// Cylindrical Bessel function of the first kind, order 1 (`J_1(x)`).
    BesselJ1,
    /// Cylindrical Bessel function of the first kind, integer order `n` (`J_n(x)`).
    BesselJn,
    /// Cylindrical Bessel function of the second kind, order 0 (`Y_0(x)`, `x > 0`).
    BesselY0,
    /// Cylindrical Bessel function of the second kind, order 1 (`Y_1(x)`, `x > 0`).
    BesselY1,
    /// Cylindrical Bessel function of the second kind, integer order `n` (`Y_n(x)`, `x > 0`).
    BesselYn,
    /// Modified Bessel function of the first kind, order 0 (`I_0(x)`).
    BesselI0,
    /// Modified Bessel function of the first kind, order 1 (`I_1(x)`).
    BesselI1,
    /// Modified Bessel function of the first kind, integer order `n` (`I_n(x)`).
    BesselIn,
    /// Modified Bessel function of the second kind, order 0 (`K_0(x)`, `x > 0`).
    BesselK0,
    /// Modified Bessel function of the second kind, order 1 (`K_1(x)`, `x > 0`).
    BesselK1,
    /// Modified Bessel function of the second kind, integer order `n` (`K_n(x)`, `x > 0`).
    BesselKn,
    /// Confluent hypergeometric function of the first kind (Kummer `M(a, b, z) = 1F1(a; b; z)`).
    Hyp1F1,
    /// Gauss hypergeometric function `2F1(a, b; c; z)`.
    Hyp2F1,
    /// Tricomi confluent hypergeometric function `U(a, b, z)`.
    ConfluentU,
    /// Function-call power `pow(base, exp)`.
    PowFn,
}

/// Compiled custom equation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DslEquationKind {
    /// Closed-form explicit equation `Y = f(X, theta)`.
    Explicit,
    /// Implicit algebraic relation `LHS(X, Y, theta) = RHS(X, Y, theta)` solved per point via Brent root-finding.
    Implicit,
    /// First-order ordinary differential equation `dY/dX = f(X, Y, theta)` integrated via adaptive RK45 with optional `Y0 = ...` / `Y(t0) = ...` initial rule.
    OdeFirstOrder,
}

/// Classification of a non-fatal diagnostic emitted while compiling a custom DSL equation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DslWarningKind {
    /// A reserved built-in constant or predictor alias (`E`, `PI`, `T`, `DS`, `DATASET`,
    /// `INF`, `NAN`, `LN2`, `LN10`, `SQRT2`, `X1`, ...) resolved as a built-in symbol rather than a fitted parameter.
    ReservedIdentifierUsed,
    /// A reserved built-in identifier was written with non-canonical casing (e.g. `e`, `Pi`, `nan`),
    /// which still resolves case-insensitively to the built-in symbol rather than a fitted parameter.
    NonCanonicalReservedCase,
    /// Both `T` and `X` appear in the same equation; `T` is a reserved alias for predictor `X` (`x[0]`).
    AliasTWithPredictorX,
}

/// Typed non-fatal warning recorded during [`CompiledDslModel::compile`](crate::dsl::CompiledDslModel::compile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DslCompileWarning {
    /// Category of the compile diagnostic.
    pub kind: DslWarningKind,
    /// Source identifier token as written in the equation (for example `"e"`, `"Pi"`, `"T"`).
    pub identifier: String,
    /// Built-in symbol or constant meaning that the identifier resolves to.
    pub resolves_to: &'static str,
    /// One-based source line number where the identifier appeared.
    pub line: usize,
    /// One-based source column number where the identifier appeared.
    pub column: usize,
    /// Human-readable diagnostic message.
    pub message: String,
}

/// Compiled statement/program chunk gated by a [`DatasetFilter`].
#[derive(Debug, Clone)]
pub(in crate::dsl) struct CompiledProgram {
    pub(in crate::dsl) code: Vec<OpCode>,
    pub(in crate::dsl) local_count: usize,
}

#[derive(Debug, Clone)]
pub(in crate::dsl) struct CompiledOdeInit {
    pub(in crate::dsl) t0: f64,
    pub(in crate::dsl) prog: CompiledProgram,
}

#[derive(Debug, Clone)]
pub(in crate::dsl) struct CompiledInitRule {
    pub(in crate::dsl) filter: DatasetFilter,
    pub(in crate::dsl) slot: usize,
    pub(in crate::dsl) param_idx: Option<usize>,
    pub(in crate::dsl) prog: CompiledProgram,
}
