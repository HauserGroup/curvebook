//! Data-table transformations, calculated-variable formula DAG, normalization, baseline removal,
//! transpose, row/column pruning, curve calculus, 4-stage import pipeline, and Monte Carlo data
//! simulation for CurveBook.
//!
//! Layer L2 domain crate. Governing spec: `docs/implementation/data-tables-and-transforms.md` (§4–§6).
//!
//! # Example
//!
//! ```
//! use curvebook_transform::{AucOptions, compute_auc};
//!
//! // Concentration-time profile over t = 0..4 h above baseline 0.0:
//! let profile = [(0.0, 0.0), (1.0, 4.0), (2.0, 6.0), (4.0, 2.0)];
//! let auc = compute_auc(&profile, &AucOptions::default()).expect("sorted finite curve");
//!
//! assert!((auc.net_area - 15.0).abs() < 1e-12);
//! assert_eq!(auc.peaks.len(), 1);
//! assert_eq!(auc.peaks[0].peak_x, 2.0);
//! assert_eq!(auc.peaks[0].peak_y, 6.0);
//! assert!((auc.peaks[0].area - 15.0).abs() < 1e-12);
//! ```
#![forbid(unsafe_code)]
#![allow(clippy::indexing_slicing)]
// Legacy dense matrix/solver indexing (FA-SEC23-03); string_slice remains denied and all 14 I/O/boundary/app crates enforce workspace indexing_slicing = "deny".
#![deny(missing_docs)]

pub mod archetype_convert;
pub mod curve;
pub mod error;
pub mod formula;
pub mod import;
pub mod ops;
pub mod plate_and_tidy;
pub mod randomize;
pub mod simulate;
pub mod spline;

pub use archetype_convert::{ConversionPreview, TargetArchetypeKind, convert_datasheet_archetype, preview_datasheet_conversion};
pub use curve::{
    AucOptions, AucResult, CurveSheetOp, DerivativeOrder, PeakSummary, SmoothMethod, apply_curve_calculus_to_sheet, compute_auc, compute_auc_with_se,
    compute_sheet_auc, differentiate_xy, integrate_xy, smooth_xy,
};
pub use error::TransformError;
pub use formula::{
    BinaryOp, DependencyTarget, Expr, FormulaContext, FormulaValue, ScalarEnv, TableEvalContext, UnaryOp, VarRef, eval_column_expr, eval_scalar_expr,
    evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_formula_for_sheet, extract_dependencies, parse_formula,
    validate_no_formula_cycle,
};
pub use import::{ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, apply_import_link_pipeline, detect_link_cycles};
pub use ops::{
    BaselineKind, MathTransformOp, PruneMethod, SelectRule, TransformContext, TransformOutput, execute_analysis_transform, execute_math_op,
    execute_math_transform, execute_normalize, execute_prune, execute_remove_baseline, execute_remove_baseline_kind, execute_select,
    execute_select_with_derived, execute_transform, execute_transpose,
};
pub use plate_and_tidy::{PlateGeometry, PlateMapTemplate, PlateWellRole, TidyColumnMapping, map_plate_to_datasheet, pivot_tidy_long_to_datasheet};
pub use randomize::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RandomizationOutput, RowSampleConfig, SimulatedColumnSpec,
    SimulatedDistribution, SubjectRandomizationConfig, random_sample_rows, randomize_subjects_to_groups, sample_row_indices,
    simulate_distribution_columns,
};
pub use simulate::{evaluate_equation, simulate_xy_sheet};
pub use spline::{
    CubicPiece, FittedCurveModel, SmoothingSplineFit, execute_spline_lowess, fit_akima_subspline, fit_lowess_curve, fit_natural_cubic_spline,
    fit_point_to_point, fit_smoothing_spline, sample_curve,
};

/// Built-in families of table transformations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformKind {
    /// Rescale each dataset or global series between configurable baseline (`0%`) and ceiling (`100%`) anchors.
    Normalize,
    /// Subtract, divide, or compute percentage/fractional change relative to designated baseline columns or rows.
    RemoveBaseline,
    /// Swap table rows and dataset columns while preserving numeric values, text labels, and exclusion flags.
    Transpose,
    /// Express each cell as a proportion or percentage of its row total, column total, or grand total.
    FractionOfTotal,
    /// Downsample or aggregate dense XY/column series by decimation or fixed-width *X*-binning.
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
