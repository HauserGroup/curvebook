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
//!
//! Module layout: resource limits (`limits`), syntax and bytecode types (`ast`), ODE integration and
//! budgets (`ode`), the compiled model (`model`), the implicit-equation solver (`implicit`), statement
//! and section parsing (`statements`), the tokenizer (`lexer`), the expression parser (`parser`),
//! bytecode compilation (`compile`), the stack VM (`vm`), and special functions (`special`). Every
//! public item is re-exported here.

mod ast;
mod compile;
mod implicit;
mod lexer;
mod limits;
mod model;
mod ode;
mod parser;
mod special;
mod statements;
#[cfg(test)]
mod tests;
mod vm;

pub use ast::{BinaryKind, BuiltinFn, DatasetFilter, DslCompileWarning, DslEquationKind, DslWarningKind, OpCode, UnaryKind};
pub(crate) use ast::{BinaryOp, Expr, UnaryOp};
pub(crate) use limits::{DEFAULT_MAX_ODE_STEPS_PER_FIT, MAX_DSL_LINES, MAX_DSL_SOURCE_BYTES, MAX_TOTAL_DATASET_INDICES};
pub use model::CompiledDslModel;
pub use ode::integrate_rk45;
pub(crate) use ode::{enter_ode_fit_budget, ode_fit_budget_exceeded};
pub(crate) use parser::parse_expr_collecting_locs;
pub(crate) use statements::{DslSection, is_valid_ident, parse_dataset_prefix, parse_section_header, split_top_level_assignment};
