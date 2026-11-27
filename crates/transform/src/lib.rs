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
    compute_sheet_auc, differentiate_xy, execute_curve_calculus, integrate_xy, savitzky_golay_derivative, smooth_xy,
};
pub use error::TransformError;
pub use formula::{
    BinaryOp, DependencyTarget, Expr, FormulaContext, FormulaValue, ScalarEnv, TableEvalContext, UnaryOp, VarRef, eval_column_expr, eval_scalar_expr,
    evaluate_calculated_variables, evaluate_data_sheet_calculated_variables, evaluate_formula_for_sheet, extract_dependencies, parse_formula,
    unlink_calculated_variable, validate_no_formula_cycle,
};
pub use import::{ImportPipelineOptions, ImportPipelineReport, ImportSourcePayload, apply_import_link_pipeline, detect_link_cycles};
pub use ops::{
    BaselineKind, MathTransformOp, MathTransformOptions, PruneMethod, SelectRearrangeOptions, SelectRule, TransformContext, TransformOutput,
    clopper_pearson_ci, enforce_summary_mode_matrix, execute_analysis_transform, execute_fraction_of_total, execute_math_op, execute_math_transform,
    execute_math_transform_with_options, execute_normalize, execute_prune, execute_remove_baseline, execute_remove_baseline_kind,
    execute_remove_baseline_with_options, execute_row_statistics, execute_select, execute_select_and_rearrange, execute_select_with_derived,
    execute_transform, execute_transpose, quantile_type6_sorted, wilson_score_ci,
};
pub use plate_and_tidy::{
    ExtractedPlateValues, PlateGeometry, PlateGridBuildOptions, PlateKineticReducer, PlateMapOutput, PlateMapTemplate, PlateSourceExtraction,
    PlateWellCellEntry, PlateWellMatchMode, PlateWellRole, PlateXGrid, TidyColumnMapping, build_plate_template_from_grids,
    extract_plate_grid_from_datasheet, extract_plate_values_from_analyzable_sheet, extract_plate_x_grid_from_datasheet, map_plate_to_datasheet,
    map_plate_to_datasheet_with_origins, pivot_tidy_long_to_datasheet,
};
pub use randomize::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, RandomizationOutput, RowSampleConfig, SimulatedColumnSpec,
    SimulatedDistribution, SubjectRandomizationConfig, random_sample_rows, randomize_subjects_to_groups, sample_row_indices,
    simulate_distribution_columns,
};
pub use simulate::{evaluate_equation, simulate_column_sheet, simulate_contingency_2x2, simulate_xy_sheet};
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
pub(crate) mod test_helpers {
    use curvebook_doc::{
        Cell, CellCoord, CellExclusionNote, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, MultipleVariablesTable, SequenceProgression,
        SubcolumnMode, TableArchetype, VariableRole, XColumnMode, XyTable,
    };

    pub(crate) fn as_xy(s: &DataSheet) -> &XyTable {
        let TableArchetype::Xy(t) = &s.archetype else { panic!("expected Xy") };
        t
    }
    pub(crate) fn as_col(s: &DataSheet) -> &ColumnTable {
        let TableArchetype::Column(t) = &s.archetype else { panic!("expected Column") };
        t
    }
    pub(crate) fn as_grp(s: &DataSheet) -> &GroupedTable {
        let TableArchetype::Grouped(t) = &s.archetype else { panic!("expected Grouped") };
        t
    }
    pub(crate) fn as_mv(s: &DataSheet) -> &MultipleVariablesTable {
        let TableArchetype::MultipleVariables(t) = &s.archetype else { panic!("expected MV") };
        t
    }
    pub(crate) fn into_xy(s: DataSheet) -> XyTable {
        let TableArchetype::Xy(t) = s.archetype else { panic!("expected Xy") };
        t
    }
    pub(crate) fn into_col(s: DataSheet) -> ColumnTable {
        let TableArchetype::Column(t) = s.archetype else { panic!("expected Column") };
        t
    }
    pub(crate) fn into_grp(s: DataSheet) -> GroupedTable {
        let TableArchetype::Grouped(t) = s.archetype else { panic!("expected Grouped") };
        t
    }
    pub(crate) fn into_mv(s: DataSheet) -> MultipleVariablesTable {
        let TableArchetype::MultipleVariables(t) = s.archetype else { panic!("expected MV") };
        t
    }
    pub(crate) fn num(c: &Cell) -> f64 {
        c.active_number().unwrap()
    }
    pub(crate) fn nums(vs: &[f64]) -> Vec<Cell> {
        vs.iter().copied().map(Cell::number).collect()
    }
    pub(crate) fn texts(vs: &[&str]) -> Vec<Cell> {
        vs.iter().copied().map(Cell::text).collect()
    }
    pub(crate) fn col_sheet(id: &str, cols: &[Vec<Cell>]) -> DataSheet {
        let rows = cols.first().map_or(0, Vec::len);
        let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.len(), rows).unwrap();
        for (c, col) in cols.iter().enumerate() {
            t.data_sets[c].cells = col.iter().cloned().map(|cell| vec![cell]).collect();
        }
        DataSheet::new(id, id, TableArchetype::Column(t)).unwrap()
    }
    pub(crate) fn xy_sheet(id: &str, mode: SubcolumnMode, xs: &[f64], cols: &[Vec<Vec<Cell>>]) -> DataSheet {
        let mut t = XyTable::new(mode, cols.len(), xs.len()).unwrap();
        for (r, &x) in xs.iter().enumerate() {
            t.set_x_cell(r, Cell::number(x)).unwrap();
        }
        for (c, col) in cols.iter().enumerate() {
            t.data_sets[c].cells = col.clone();
        }
        DataSheet::new(id, id, TableArchetype::Xy(t)).unwrap()
    }
    pub(crate) fn xy_gen_seq(start: f64, step: f64, prog: SequenceProgression, mode: SubcolumnMode, cols: usize, rows: usize) -> XyTable {
        XyTable::new_with_x_mode(XColumnMode::GeneratedSequence { start, step, progression: prog }, mode, cols, rows).unwrap()
    }
    pub(crate) fn mv_sheet(id: &str, cols: &[(&str, Option<VariableRole>, Vec<Cell>)]) -> DataSheet {
        let rows = cols.first().map_or(0, |(_, _, c)| c.len());
        let mut mv = MultipleVariablesTable::new(cols.len(), rows).unwrap();
        for (i, (title, role, cells)) in cols.iter().enumerate() {
            mv.variables[i].title = (*title).to_owned();
            if let Some(r) = role {
                mv.variables[i].lock_role(*r);
            }
            mv.variables[i].cells = cells.clone();
        }
        DataSheet::new(id, id, TableArchetype::MultipleVariables(mv)).unwrap()
    }
    pub(crate) fn excl_note(row: usize, col: usize, sub: usize, tag: &str, note: &str) -> CellExclusionNote {
        CellExclusionNote { coord: CellCoord::new(row, col, sub), reason_tag: tag.to_owned(), note: note.to_owned(), timestamp_utc: String::new() }
    }
    pub(crate) fn assert_close(actual: f64, expected: f64, tol: f64) {
        assert!((actual - expected).abs() < tol, "expected {expected} ± {tol}, got {actual}");
    }
}

#[cfg(test)]
mod tests {
    use super::TransformKind;

    #[test]
    fn transform_kinds_are_distinct() {
        assert_ne!(TransformKind::Normalize, TransformKind::Transpose);
    }
}
