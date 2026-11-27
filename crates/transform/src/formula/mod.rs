//! Calculated-variable formula parser, AST, vectorized evaluator, and topological DAG solver.
//!
//! Governing spec: `docs/implementation/data-tables-and-transforms.md` (§4).

pub mod dag;
pub mod eval;
pub mod parser;

pub use dag::{
    DependencyTarget, evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, extract_dependencies, unlink_calculated_variable,
    validate_no_formula_cycle,
};
pub use eval::{FormulaContext, FormulaValue, ScalarEnv, TableEvalContext, eval_column_expr, eval_scalar_expr, evaluate_formula_for_sheet};
pub use parser::{
    BinaryOp, Expr, FrameBound, NullsOrder, OrderByExpr, SortDirection, UnaryOp, VarRef, WindowFrame, WindowSpec, parse_formula,
    rewrite_formula_column_refs,
};
