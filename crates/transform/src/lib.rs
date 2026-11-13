//! Data-table transformations, calculated-variable formula DAG, normalization, baseline removal,
//! transpose, row/column pruning, curve calculus, 4-stage import pipeline, and Monte Carlo data
//! simulation for LumenCraft.
//!
//! Layer L2 domain crate. Governing spec: `docs/data-tables-and-transforms.md` (§4–§6).
#![forbid(unsafe_code)]

pub mod curve;
pub mod error;
pub mod formula;
pub mod import;
pub mod ops;
pub mod simulate;

pub use curve::{
    AucOptions, AucResult, CurveSheetOp, DerivativeOrder, PeakSummary, SmoothMethod, apply_curve_calculus_to_sheet, compute_auc, compute_auc_with_se,
    compute_sheet_auc, differentiate_xy, integrate_xy, smooth_xy,
};
pub use error::TransformError;
pub use formula::{
    BinaryOp, DependencyTarget, Expr, FormulaContext, FormulaValue, ScalarEnv, TableEvalContext, UnaryOp, VarRef, eval_column_expr, eval_scalar_expr,
    evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_formula_for_sheet, extract_dependencies, parse_formula,
};
pub use import::{ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, apply_import_link_pipeline, detect_link_cycles};
pub use ops::{
    BaselineKind, MathTransformOp, PruneMethod, SelectRule, TransformContext, TransformOutput, execute_analysis_transform, execute_math_op,
    execute_math_transform, execute_normalize, execute_prune, execute_remove_baseline, execute_remove_baseline_kind, execute_select,
    execute_transform, execute_transpose,
};
pub use simulate::{evaluate_equation, simulate_xy_sheet};

/// Built-in families of table transformations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    Normalize,
    RemoveBaseline,
    Transpose,
    FractionOfTotal,
    PruneRows,
}

#[cfg(test)]
mod tests {
    use super::TransformKind;

    #[test]
    fn transform_kinds_are_distinct() {
        assert_ne!(TransformKind::Normalize, TransformKind::Transpose);
    }
}
