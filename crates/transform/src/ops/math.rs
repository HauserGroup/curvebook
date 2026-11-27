//! Mathematical transforms on X and/or Y series with analytical and endpoint error
//! propagation (`TransformAnalysisSpec::Math`, `docs/implementation/data-tables-and-transforms.md` §5.1).
//!
//! ## Subcolumn Error Propagation Contract
//! - `MeanSdN`, `MeanSemN`, and `MeanCvN` preserve their `[Mean, Spread, N]` 3-subcolumn schema:
//!   `FirstOrderDeltaMethod` scales spread by `|g'(mean)|` (converting `MeanCvN` via
//!   `SD = (CV / 100) * |mean|` before and `100 * SD_out / |mean_out|` after); `ReplicateEndpoints`
//!   evaluates `[mean - SD, mean + SD]` and stores the symmetric half-width
//!   `0.5 * |g(mean + SD) - g(mean - SD)|` (complementing the endpoint-interval description on
//!   [`ErrorPropagationMode::ReplicateEndpoints`]).
//! - `MeanLowHigh` retains `[Mean, -Error, +Error]` (`ReplicateEndpoints` maps `[mean - (-Err),
//!   mean + (+Err)]` to sorted `[lo_end, hi_end]` offsets; `FirstOrderDeltaMethod` scales each
//!   offset by `|g'(mean)|` and swaps `-Error <-> +Error` when `g'(mean) < 0`).
//! - `UpperLowerLimits` and `MedianIqr` transform center and bounds independently and reorder (or
//!   swap when `g' < 0`) lower/upper limits.
//! - Whenever an endpoint, derivative, zero CV denominator (`mean_out == 0`), or explicit bound
//!   falls outside the transform domain and blanks an active spread/error cell, a non-fatal warning
//!   is emitted.
//!
//! References:
//! - Joint Committee for Guides in Metrology (2008). *Evaluation of measurement data — Guide to
//!   the expression of uncertainty in measurement* (JCGM 100:2008 / GUM §5.1.2).
//!   `doi:10.59161/JCGM100-2008E`.
//! - Zar, J. H. (2010). *Biostatistical Analysis* (5th ed.). Prentice Hall (logit, probit, and
//!   angular arcsine square-root transformations).

use curvebook_doc::{
    Cell, CellValue, DataSetColumn, DataSheet, DocError, ErrorPropagationMode, InfoSheet, MathTransformPreset, ResultCell, SubcolumnMode,
    TableArchetype, TableKind, XColumnMode,
};
use curvebook_math::dist;

use crate::error::TransformError;
use crate::formula::{Expr, ScalarEnv, eval_scalar_expr, parse_formula};
use crate::ops::{
    TransformContext, TransformOutput, build_summary_tab, remap_swapped_error_notes_batch, snapshot_calculated_variables, swap_error_bound_cells,
    validate_parts_of_whole_non_negative,
};

/// Extended mathematical transform operation supporting presets, constant arithmetic, arbitrary
/// antilogs, and custom user expressions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MathTransformOp {
    /// Delegate to a document-layer [`MathTransformPreset`].
    Preset(MathTransformPreset),
    /// Base-10 logarithm `log10(u)` (`u > 0`).
    Log10,
    /// Natural logarithm `ln(u)` (`u > 0`).
    Ln,
    /// Base-2 logarithm `log2(u)` (`u > 0`).
    Log2,
    /// Base-10 exponential `10^u`.
    Exp10,
    /// Natural exponential `exp(u)`.
    ExpE,
    /// Multiplicative inverse `1 / u` (`u != 0`).
    Reciprocal,
    /// Quadratic square `u^2`.
    Square,
    /// Principal square root `sqrt(u)` (`u >= 0`).
    Sqrt,
    /// Scale by constant factor `u * k`.
    MultiplyByConstant(f64),
    /// Divide by non-zero constant `u / k` (`k != 0`).
    DivideByConstant(f64),
    /// Add constant offset `u + c`.
    AddConstant(f64),
    /// Subtract constant offset `u - c`.
    SubtractConstant(f64),
    /// Unit-proportion log-odds `ln(u / (1 - u))` (`u in (0, 1)`).
    Logit,
    /// Arbitrary-base exponential `base^u` (`base > 0`, `base != 1`).
    Antilog {
        /// Positive exponential base (`> 0`, `!= 1`).
        base: f64,
    },
    /// Finney's probit transform `5 + Phi^{-1}(u)` (`u in (0, 1)`, Finney 1971; Zar 2010).
    Probit,
    /// Angular arcsine square-root transform `arcsin(sqrt(u))` in radians (`u in [0, 1]`).
    ArcsinSqrt,
    /// Custom scalar formula expression evaluated with bindings `x`, `y`, and paired `InfoSheet` constants.
    UserExpression(String),
}

enum CompiledOp {
    Preset(MathTransformPreset),
    Multiply(f64),
    Divide(f64),
    Add(f64),
    Subtract(f64),
    Antilog(f64),
    UserExpr(Expr),
}

impl CompiledOp {
    fn compile(op: &MathTransformOp) -> Result<Self, TransformError> {
        let compiled = match op {
            MathTransformOp::Preset(p) => {
                validate_preset(*p)?;
                Self::Preset(*p)
            }
            MathTransformOp::Log10 => Self::Preset(MathTransformPreset::Log10),
            MathTransformOp::Ln => Self::Preset(MathTransformPreset::Ln),
            MathTransformOp::Log2 => Self::Preset(MathTransformPreset::Log2),
            MathTransformOp::Exp10 => Self::Preset(MathTransformPreset::Exp10),
            MathTransformOp::ExpE => Self::Preset(MathTransformPreset::Exp),
            MathTransformOp::Reciprocal => Self::Preset(MathTransformPreset::Reciprocal),
            MathTransformOp::Square => Self::Preset(MathTransformPreset::Square),
            MathTransformOp::Sqrt => Self::Preset(MathTransformPreset::Sqrt),
            MathTransformOp::Logit => Self::Preset(MathTransformPreset::Logit),
            MathTransformOp::Probit => Self::Preset(MathTransformPreset::Probit),
            MathTransformOp::ArcsinSqrt => Self::Preset(MathTransformPreset::ArcsinSqrt),
            MathTransformOp::MultiplyByConstant(c) => {
                check_finite(*c, "MultiplyByConstant")?;
                Self::Multiply(*c)
            }
            MathTransformOp::DivideByConstant(c) => {
                check_finite(*c, "DivideByConstant")?;
                if *c == 0.0 {
                    return Err(TransformError::InvalidSpec { message: String::from("DivideByConstant divisor cannot be zero") });
                }
                Self::Divide(*c)
            }
            MathTransformOp::AddConstant(c) => {
                check_finite(*c, "AddConstant")?;
                Self::Add(*c)
            }
            MathTransformOp::SubtractConstant(c) => {
                check_finite(*c, "SubtractConstant")?;
                Self::Subtract(*c)
            }
            MathTransformOp::Antilog { base } => {
                check_finite(*base, "Antilog base")?;
                if *base <= 0.0 || *base == 1.0 {
                    return Err(TransformError::InvalidSpec { message: format!("Antilog base must be > 0 and != 1, got {base}") });
                }
                Self::Antilog(*base)
            }
            MathTransformOp::UserExpression(s) => Self::UserExpr(parse_formula(s)?),
        };
        Ok(compiled)
    }

    fn eval(&self, v: f64, x_ctx: Option<f64>, is_x_axis: bool, info: Option<&InfoSheet>) -> Option<f64> {
        if !v.is_finite() {
            return None;
        }
        let out = match self {
            Self::Preset(p) => eval_preset(*p, v)?,
            Self::Multiply(c) => v * c,
            Self::Divide(c) => v / c,
            Self::Add(c) => v + c,
            Self::Subtract(c) => v - c,
            Self::Antilog(base) => base.powf(v),
            Self::UserExpr(expr) => eval_user_expr(expr, v, x_ctx, is_x_axis, info)?,
        };
        out.is_finite().then_some(out)
    }

    fn derivative(&self, v: f64, x_ctx: Option<f64>, is_x_axis: bool, info: Option<&InfoSheet>) -> Option<f64> {
        if !v.is_finite() {
            return None;
        }
        let d = match self {
            Self::Preset(p) => eval_preset_derivative(*p, v)?,
            Self::Multiply(c) => *c,
            Self::Divide(c) => 1.0 / *c,
            Self::Add(_) | Self::Subtract(_) => 1.0,
            Self::Antilog(base) => base.powf(v) * base.ln(),
            Self::UserExpr(expr) => {
                let h = if v != 0.0 { 1e-6 * v.abs() } else { 1e-6 };
                if h == 0.0 || !h.is_finite() {
                    return None;
                }
                let f_hi = eval_user_expr(expr, v + h, x_ctx, is_x_axis, info);
                let f_lo = eval_user_expr(expr, v - h, x_ctx, is_x_axis, info);
                match (f_hi, f_lo) {
                    (Some(hi), Some(lo)) => (hi - lo) / (2.0 * h),
                    (Some(hi), None) => {
                        let f_mid = eval_user_expr(expr, v, x_ctx, is_x_axis, info)?;
                        (hi - f_mid) / h
                    }
                    (None, Some(lo)) => {
                        let f_mid = eval_user_expr(expr, v, x_ctx, is_x_axis, info)?;
                        (f_mid - lo) / h
                    }
                    (None, None) => return None,
                }
            }
        };
        d.is_finite().then_some(d)
    }
}

fn check_finite(v: f64, label: &str) -> Result<(), TransformError> {
    if !v.is_finite() {
        return Err(TransformError::InvalidSpec { message: format!("{label} requires a finite constant, got {v}") });
    }
    Ok(())
}

fn validate_preset(preset: MathTransformPreset) -> Result<(), TransformError> {
    match preset {
        MathTransformPreset::LinearScale { slope, intercept } => {
            check_finite(slope, "LinearScale slope")?;
            check_finite(intercept, "LinearScale intercept")?;
        }
        MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            check_finite(molar_unit_factor, "molar_unit_factor")?;
            if molar_unit_factor <= 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("molar_unit_factor must be > 0, got {molar_unit_factor}") });
            }
        }
        _ => {}
    }
    Ok(())
}

fn eval_preset(preset: MathTransformPreset, v: f64) -> Option<f64> {
    match preset {
        MathTransformPreset::Ln => (v > 0.0).then(|| v.ln()),
        MathTransformPreset::Log10 => (v > 0.0).then(|| v.log10()),
        MathTransformPreset::Log2 => (v > 0.0).then(|| v.log2()),
        MathTransformPreset::Exp10 => Some(10.0_f64.powf(v)),
        MathTransformPreset::Exp => Some(v.exp()),
        MathTransformPreset::Reciprocal => (v != 0.0).then(|| 1.0 / v),
        MathTransformPreset::Sqrt => (v >= 0.0).then(|| v.sqrt()),
        MathTransformPreset::Square => Some(v * v),
        MathTransformPreset::Logit => (v > 0.0 && v < 1.0).then(|| v.ln() - (-v).ln_1p()),
        MathTransformPreset::Probit => {
            if v > 0.0 && v < 1.0 {
                dist::norm_ppf(v).ok().map(|z| 5.0 + z)
            } else {
                None
            }
        }
        MathTransformPreset::ArcsinSqrt => (0.0..=1.0).contains(&v).then(|| v.sqrt().asin()),
        MathTransformPreset::LinearScale { slope, intercept } => Some(slope * v + intercept),
        MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            (v > 0.0 && molar_unit_factor > 0.0).then(|| -(v.log10() + molar_unit_factor.log10()))
        }
    }
}

fn eval_preset_derivative(preset: MathTransformPreset, v: f64) -> Option<f64> {
    match preset {
        MathTransformPreset::Ln => (v > 0.0).then(|| 1.0 / v),
        MathTransformPreset::Log10 => (v > 0.0).then(|| 1.0 / (v * std::f64::consts::LN_10)),
        MathTransformPreset::Log2 => (v > 0.0).then(|| 1.0 / (v * std::f64::consts::LN_2)),
        MathTransformPreset::Exp10 => Some(10.0_f64.powf(v) * std::f64::consts::LN_10),
        MathTransformPreset::Exp => Some(v.exp()),
        MathTransformPreset::Reciprocal => (v != 0.0).then(|| -1.0 / (v * v)),
        MathTransformPreset::Sqrt => (v > 0.0).then(|| 0.5 / v.sqrt()),
        MathTransformPreset::Square => Some(2.0 * v),
        MathTransformPreset::Logit => (v > 0.0 && v < 1.0).then(|| 1.0 / (v * (1.0 - v))),
        MathTransformPreset::Probit => {
            if v > 0.0 && v < 1.0 {
                let z = dist::norm_ppf(v).ok()?;
                let pdf = dist::norm_pdf(z);
                (pdf > 0.0).then(|| 1.0 / pdf)
            } else {
                None
            }
        }
        MathTransformPreset::ArcsinSqrt => (v > 0.0 && v < 1.0).then(|| 0.5 / (v * (1.0 - v)).sqrt()),
        MathTransformPreset::LinearScale { slope, .. } => Some(slope),
        MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor } => {
            (v > 0.0 && molar_unit_factor > 0.0).then(|| -1.0 / (v * std::f64::consts::LN_10))
        }
    }
}

fn eval_user_expr(expr: &Expr, v: f64, x_ctx: Option<f64>, is_x_axis: bool, info: Option<&InfoSheet>) -> Option<f64> {
    let mut bindings = Vec::with_capacity(4);
    if is_x_axis {
        bindings.push(("x", v));
        bindings.push(("val", v));
        bindings.push(("v", v));
    } else {
        bindings.push(("y", v));
        bindings.push(("val", v));
        bindings.push(("v", v));
        if let Some(xv) = x_ctx {
            bindings.push(("x", xv));
        }
    }
    let env = ScalarEnv::new(bindings, info);
    eval_scalar_expr(expr, &env).ok()?.as_finite_f64()
}

/// Apply a single [`MathTransformOp`] to Y (and optionally X) columns.
pub fn execute_math_op(
    source: &DataSheet,
    y_op: &MathTransformOp,
    error_propagation: ErrorPropagationMode,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    execute_math_transform(source, None, Some(y_op), error_propagation, ctx)
}

/// Apply optional mathematical transforms on X and/or Y columns with error propagation.
pub fn execute_math_transform(
    source: &DataSheet,
    x_op: Option<&MathTransformOp>,
    y_op: Option<&MathTransformOp>,
    error_propagation: ErrorPropagationMode,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    source.archetype.validate()?;
    if matches!(source.archetype, TableArchetype::Survival(_)) && y_op.is_some() {
        return Err(TransformError::IncompatibleTable { operation: "math_transform_y", kind: TableKind::Survival });
    }
    let compiled_x = x_op.map(CompiledOp::compile).transpose()?;
    let compiled_y = y_op.map(CompiledOp::compile).transpose()?;

    let mut out = source.clone();
    let (id, title) = ctx.derived_ids(source, "math", "Transform");
    out.id = curvebook_doc::SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();

    let mut warnings = Vec::new();
    let mut transformed_cells = 0_usize;
    let info = ctx.primary_info_sheet();

    let x_values_before: Vec<Option<f64>> = match &source.archetype {
        TableArchetype::Xy(xy) => (0..xy.row_count()).map(|r| xy.raw_x_value_at(r)).collect(),
        _ => Vec::new(),
    };

    if let Some(ref cx) = compiled_x {
        match &mut out.archetype {
            TableArchetype::Xy(xy) => {
                transform_xy_x_axis(xy, cx, error_propagation, info, &mut transformed_cells, &mut warnings);
            }
            TableArchetype::Survival(srv) => {
                for (r, time_cell) in srv.times.iter_mut().enumerate() {
                    transform_single_cell(time_cell, cx, None, true, info, r, 0, &mut transformed_cells, &mut warnings);
                    if let Some(t_val) = time_cell.active_number()
                        && t_val < 0.0
                    {
                        return Err(TransformError::Doc(DocError::InvalidCellValue {
                            archetype: TableKind::Survival,
                            row: r,
                            col: 0,
                            reason: "elapsed survival time must be >= 0",
                        }));
                    }
                }
            }
            _ => {}
        }
    }

    if let Some(ref cy) = compiled_y {
        let col_count = out.archetype.column_count();
        let selected = ctx.active_columns(col_count)?;
        let swapped = transform_archetype_y(
            &mut out.archetype,
            &selected,
            cy,
            error_propagation,
            &x_values_before,
            info,
            &mut transformed_cells,
            &mut warnings,
        )?;
        remap_swapped_error_notes_batch(&mut out.exclusion_notes, &swapped);
        for &(row, col) in &swapped {
            crate::randomize::remap_swapped_error_origins(&mut out.plate_well_origins, &[row], col);
        }
    }

    out.prune_stale_plate_well_origins();
    out.validate()?;
    let summary_tab = build_summary_tab(
        "Math transform summary",
        vec![
            ("Transformed cells", ResultCell::Number { value: transformed_cells as f64, prefix: None, units: None }),
            ("Warnings", ResultCell::Number { value: warnings.len() as f64, prefix: None, units: None }),
        ],
        warnings.clone(),
    );

    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
}

fn transform_xy_x_axis(
    xy: &mut curvebook_doc::XyTable,
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    info: Option<&InfoSheet>,
    count: &mut usize,
    warnings: &mut Vec<String>,
) {
    let rows = xy.row_count();
    let sub_mode = match xy.x_mode {
        XColumnMode::Numbers { subcolumn_mode } => subcolumn_mode,
        XColumnMode::DateOrElapsedTime { .. } | XColumnMode::GeneratedSequence { .. } => {
            let mut materialized = Vec::with_capacity(rows);
            for r in 0..rows {
                materialized.push(vec![xy.materialized_x_cell_at(r)]);
            }
            xy.x_cells = materialized;
            xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
            SubcolumnMode::Single
        }
    };

    for (r, row_cells) in xy.x_cells.iter_mut().enumerate() {
        let mut temp_col =
            DataSetColumn { title: xy.x_title.clone(), cells: vec![row_cells.clone()], subcolumn_titles: Vec::new(), display_decimals: None };
        transform_row_slice(&mut temp_col.cells[0], sub_mode, op, err_mode, None, true, info, r, 0, count, warnings);
        *row_cells = temp_col.cells.swap_remove(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn transform_archetype_y(
    arch: &mut TableArchetype,
    selected: &[usize],
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    x_ctx: &[Option<f64>],
    info: Option<&InfoSheet>,
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<(usize, usize)>, TransformError> {
    let swapped = match arch {
        TableArchetype::Xy(t) => transform_dataset_vec(&mut t.data_sets, t.y_subcolumn_mode, selected, op, err_mode, x_ctx, info, count, warnings),
        TableArchetype::Column(t) => transform_dataset_vec(&mut t.data_sets, t.subcolumn_mode, selected, op, err_mode, x_ctx, info, count, warnings),
        TableArchetype::Grouped(t) => transform_dataset_vec(&mut t.data_sets, t.subcolumn_mode, selected, op, err_mode, x_ctx, info, count, warnings),
        TableArchetype::Contingency(t) => {
            let swapped = transform_dataset_vec(&mut t.data_sets, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings);
            t.validate_counts()?;
            swapped
        }
        TableArchetype::Survival(_) => {
            return Err(TransformError::IncompatibleTable { operation: "math_transform_y", kind: TableKind::Survival });
        }
        TableArchetype::PartsOfWhole(t) => {
            let swapped = transform_dataset_vec(&mut t.data_sets, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings);
            validate_parts_of_whole_non_negative(t)?;
            swapped
        }
        TableArchetype::Nested(t) => {
            for &c in selected {
                if let Some(grp) = t.groups.get_mut(c) {
                    let mode = SubcolumnMode::RawReplicates(grp.biological_unit_count().max(1));
                    for (r, row_cells) in grp.cells.iter_mut().enumerate() {
                        transform_row_slice(row_cells, mode, op, err_mode, None, false, info, r, c, count, warnings);
                    }
                }
            }
            Vec::new()
        }
        TableArchetype::MultipleVariables(_) => {
            if let TableArchetype::MultipleVariables(mv) = arch {
                for &c in selected {
                    if let Some(var) = mv.variables.get_mut(c) {
                        for (r, cell) in var.cells.iter_mut().enumerate() {
                            let mut one = [cell.clone()];
                            transform_row_slice(&mut one, SubcolumnMode::Single, op, err_mode, None, false, info, r, c, count, warnings);
                            *cell = one[0].clone();
                        }
                    }
                }
            }
            snapshot_calculated_variables(arch);
            Vec::new()
        }
    };
    Ok(swapped)
}

#[allow(clippy::too_many_arguments)]
fn transform_dataset_vec(
    data_sets: &mut [DataSetColumn],
    mode: SubcolumnMode,
    selected: &[usize],
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    x_ctx: &[Option<f64>],
    info: Option<&InfoSheet>,
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> Vec<(usize, usize)> {
    let mut swapped = Vec::new();
    for &c in selected {
        let Some(ds) = data_sets.get_mut(c) else {
            continue;
        };
        for (r, row_cells) in ds.cells.iter_mut().enumerate() {
            let x_val = x_ctx.get(r).copied().flatten();
            if transform_row_slice(row_cells, mode, op, err_mode, x_val, false, info, r, c, count, warnings) {
                swapped.push((r, c));
            }
        }
    }
    swapped
}

#[allow(clippy::too_many_arguments)]
fn transform_row_slice(
    row_cells: &mut [Cell],
    mode: SubcolumnMode,
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    x_val: Option<f64>,
    is_x_axis: bool,
    info: Option<&InfoSheet>,
    row_idx: usize,
    col_idx: usize,
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> bool {
    match mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
            for cell in row_cells.iter_mut() {
                transform_single_cell(cell, op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
            }
            false
        }
        SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN | SubcolumnMode::MeanSd | SubcolumnMode::MeanSem => {
            transform_parametric_summary(row_cells, mode, op, err_mode, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
            false
        }
        SubcolumnMode::MeanLowHigh => transform_mean_low_high(row_cells, op, err_mode, x_val, is_x_axis, info, row_idx, col_idx, count, warnings),
        SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
            transform_explicit_limits(row_cells, op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn transform_single_cell(
    cell: &mut Cell,
    op: &CompiledOp,
    x_val: Option<f64>,
    is_x_axis: bool,
    info: Option<&InfoSheet>,
    row_idx: usize,
    col_idx: usize,
    count: &mut usize,
    warnings: &mut Vec<String>,
) {
    let Some(v) = cell.value.finite_f64() else {
        return;
    };
    if let Some(res) = op.eval(v, x_val, is_x_axis, info) {
        cell.value = CellValue::Number(res);
        *count += 1;
    } else {
        cell.value = CellValue::Empty;
        if !cell.excluded {
            warnings.push(format!("row {row_idx} col {col_idx}: value {v} outside transform domain"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn transform_parametric_summary(
    row_cells: &mut [Cell],
    mode: SubcolumnMode,
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    x_val: Option<f64>,
    is_x_axis: bool,
    info: Option<&InfoSheet>,
    row_idx: usize,
    col_idx: usize,
    count: &mut usize,
    warnings: &mut Vec<String>,
) {
    if row_cells.len() < 2 {
        return;
    }
    let mean_excluded = row_cells[0].excluded;
    let Some(mean_in) = row_cells[0].value.finite_f64() else {
        return;
    };
    let Some(mean_out) = op.eval(mean_in, x_val, is_x_axis, info) else {
        row_cells[0].value = CellValue::Empty;
        row_cells[1].value = CellValue::Empty;
        if !mean_excluded {
            warnings.push(format!("row {row_idx} col {col_idx}: mean {mean_in} outside transform domain"));
        }
        return;
    };
    row_cells[0] = Cell { value: CellValue::Number(mean_out), excluded: mean_excluded };
    *count += 1;

    let spread_excluded = row_cells[1].excluded;
    let Some(spread_in) = row_cells[1].value.finite_f64() else {
        return;
    };
    if mode == SubcolumnMode::MeanCvN && mean_in == 0.0 {
        row_cells[1] = Cell { value: CellValue::Empty, excluded: spread_excluded };
        if !spread_excluded {
            warnings.push(format!("row {row_idx} col {col_idx}: input mean is zero; CV% spread is undefined and blanked"));
        }
        return;
    }
    let abs_spread_in = match mode {
        SubcolumnMode::MeanCvN => (spread_in / 100.0) * mean_in.abs(),
        _ => spread_in,
    };

    let abs_spread_out = match err_mode {
        ErrorPropagationMode::FirstOrderDeltaMethod => op.derivative(mean_in, x_val, is_x_axis, info).map(|d| d.abs() * abs_spread_in),
        ErrorPropagationMode::ReplicateEndpoints => {
            let lo = op.eval(mean_in - abs_spread_in, x_val, is_x_axis, info);
            let hi = op.eval(mean_in + abs_spread_in, x_val, is_x_axis, info);
            match (lo, hi) {
                (Some(a), Some(b)) => {
                    let diff = b - a;
                    let half = if diff.is_finite() { 0.5 * diff.abs() } else { (0.5 * b - 0.5 * a).abs() };
                    Some(half)
                }
                _ => None,
            }
        }
    };

    let final_spread = match (mode, abs_spread_out) {
        (SubcolumnMode::MeanCvN, Some(sd_out)) => {
            if mean_out != 0.0 {
                let cv = (sd_out / mean_out.abs()) * 100.0;
                if cv.is_finite() {
                    Some(cv)
                } else {
                    if !spread_excluded {
                        warnings.push(format!("row {row_idx} col {col_idx}: transformed CV% overflowed; spread blanked"));
                    }
                    None
                }
            } else {
                if !spread_excluded {
                    warnings.push(format!("row {row_idx} col {col_idx}: transformed mean is zero; CV% spread blanked"));
                }
                None
            }
        }
        (_, Some(s)) if s.is_finite() => Some(s),
        (_, _) => {
            if !spread_excluded {
                warnings.push(format!("row {row_idx} col {col_idx}: spread endpoint or derivative outside transform domain; spread blanked"));
            }
            None
        }
    };

    row_cells[1] = match final_spread {
        Some(s) => Cell { value: CellValue::Number(s), excluded: spread_excluded },
        None => Cell { value: CellValue::Empty, excluded: spread_excluded },
    };
}

#[allow(clippy::too_many_arguments)]
fn transform_mean_low_high(
    row_cells: &mut [Cell],
    op: &CompiledOp,
    err_mode: ErrorPropagationMode,
    x_val: Option<f64>,
    is_x_axis: bool,
    info: Option<&InfoSheet>,
    row_idx: usize,
    col_idx: usize,
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> bool {
    if row_cells.len() < 3 {
        return false;
    }
    let Some(mean_in) = row_cells[0].value.finite_f64() else {
        return false;
    };
    let mean_excluded = row_cells[0].excluded;
    let Some(mean_out) = op.eval(mean_in, x_val, is_x_axis, info) else {
        row_cells[0].value = CellValue::Empty;
        row_cells[1].value = CellValue::Empty;
        row_cells[2].value = CellValue::Empty;
        if !mean_excluded {
            warnings.push(format!("row {row_idx} col {col_idx}: mean {mean_in} outside transform domain"));
        }
        return false;
    };
    row_cells[0] = Cell { value: CellValue::Number(mean_out), excluded: mean_excluded };
    *count += 1;

    let low_in = row_cells[1].value.finite_f64();
    let high_in = row_cells[2].value.finite_f64();
    let low_excluded = row_cells[1].excluded;
    let high_excluded = row_cells[2].excluded;
    match err_mode {
        ErrorPropagationMode::FirstOrderDeltaMethod => {
            let deriv = op.derivative(mean_in, x_val, is_x_axis, info);
            let reverses = deriv.is_some_and(|d| d < 0.0);
            let slope = deriv.map(f64::abs);
            let out_lo = match (low_in, slope) {
                (Some(d), Some(s)) => {
                    let v = d * s;
                    v.is_finite().then_some(v)
                }
                _ => None,
            };
            let out_hi = match (high_in, slope) {
                (Some(u), Some(s)) => {
                    let v = u * s;
                    v.is_finite().then_some(v)
                }
                _ => None,
            };
            if (!low_excluded && low_in.is_some() && out_lo.is_none()) || (!high_excluded && high_in.is_some() && out_hi.is_none()) {
                warnings.push(format!("row {row_idx} col {col_idx}: derivative outside transform domain; error bounds blanked"));
            }
            row_cells[1].value = out_lo.map_or(CellValue::Empty, CellValue::Number);
            row_cells[2].value = out_hi.map_or(CellValue::Empty, CellValue::Number);
            reverses && swap_error_bound_cells(row_cells)
        }
        ErrorPropagationMode::ReplicateEndpoints => {
            let t_lo = low_in.and_then(|d| op.eval(mean_in - d, x_val, is_x_axis, info));
            let t_hi = high_in.and_then(|u| op.eval(mean_in + u, x_val, is_x_axis, info));
            let lo_failed = low_in.is_some() && t_lo.is_none();
            let hi_failed = high_in.is_some() && t_hi.is_none();
            if lo_failed || hi_failed {
                row_cells[1].value = CellValue::Empty;
                row_cells[2].value = CellValue::Empty;
                if (!low_excluded && lo_failed) || (!high_excluded && hi_failed) {
                    warnings.push(format!("row {row_idx} col {col_idx}: error endpoint outside transform domain; error bounds blanked"));
                }
                return false;
            }
            match (t_lo, t_hi) {
                (Some(a), Some(b)) => {
                    let swapped = a > b;
                    let (lo_end, hi_end) = if swapped { (b, a) } else { (a, b) };
                    if swapped {
                        let tmp = row_cells[1].excluded;
                        row_cells[1].excluded = row_cells[2].excluded;
                        row_cells[2].excluded = tmp;
                    }
                    let lo_off = (mean_out - lo_end).max(0.0);
                    let hi_off = (hi_end - mean_out).max(0.0);
                    let lo_overflow = !lo_off.is_finite();
                    let hi_overflow = !hi_off.is_finite();
                    row_cells[1].value = CellValue::from_finite_or_empty(lo_off);
                    row_cells[2].value = CellValue::from_finite_or_empty(hi_off);
                    if (!row_cells[1].excluded && lo_overflow) || (!row_cells[2].excluded && hi_overflow) {
                        warnings
                            .push(format!("row {row_idx} col {col_idx}: transformed error offset overflowed finite f64 range; error bound blanked"));
                    }
                    swapped
                }
                (Some(a), None) => {
                    let off = (mean_out - a).abs();
                    if !off.is_finite() && !row_cells[1].excluded {
                        warnings
                            .push(format!("row {row_idx} col {col_idx}: transformed error offset overflowed finite f64 range; error bound blanked"));
                    }
                    row_cells[1].value = CellValue::from_finite_or_empty(off);
                    row_cells[2].value = CellValue::Empty;
                    if a > mean_out { swap_error_bound_cells(row_cells) } else { false }
                }
                (None, Some(b)) => {
                    let off = (b - mean_out).abs();
                    if !off.is_finite() && !row_cells[2].excluded {
                        warnings
                            .push(format!("row {row_idx} col {col_idx}: transformed error offset overflowed finite f64 range; error bound blanked"));
                    }
                    row_cells[1].value = CellValue::Empty;
                    row_cells[2].value = CellValue::from_finite_or_empty(off);
                    if b < mean_out { swap_error_bound_cells(row_cells) } else { false }
                }
                (None, None) => false,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn transform_explicit_limits(
    row_cells: &mut [Cell],
    op: &CompiledOp,
    x_val: Option<f64>,
    is_x_axis: bool,
    info: Option<&InfoSheet>,
    row_idx: usize,
    col_idx: usize,
    count: &mut usize,
    warnings: &mut Vec<String>,
) -> bool {
    if row_cells.len() < 3 {
        return false;
    }
    let reverses =
        row_cells.iter().take(3).find_map(|c| c.value.finite_f64().and_then(|v| op.derivative(v, x_val, is_x_axis, info))).is_some_and(|d| d < 0.0);
    transform_single_cell(&mut row_cells[0], op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
    let v1 = row_cells[1].value.finite_f64();
    let v2 = row_cells[2].value.finite_f64();
    let t1 = v1.and_then(|v| op.eval(v, x_val, is_x_axis, info));
    let t2 = v2.and_then(|v| op.eval(v, x_val, is_x_axis, info));
    if let Some(in1) = v1
        && t1.is_none()
        && !row_cells[1].excluded
    {
        warnings.push(format!("row {row_idx} col {col_idx}: lower bound {in1} outside transform domain; bound blanked"));
    }
    if let Some(in2) = v2
        && t2.is_none()
        && !row_cells[2].excluded
    {
        warnings.push(format!("row {row_idx} col {col_idx}: upper bound {in2} outside transform domain; bound blanked"));
    }
    row_cells[1].value = t1.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
    row_cells[2].value = t2.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
    let should_swap = match (t1, t2) {
        (Some(a), Some(b)) => a > b || (a == b && reverses),
        _ => reverses,
    };
    should_swap && swap_error_bound_cells(row_cells)
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, CellCoord, CellValue, DataSheet, DocError, ErrorPropagationMode as Epm, MAX_TITLE_BYTES, MathTransformPreset, MultipleVariablesTable,
        PartsOfWholeTable, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableRole, VariableSource, XyTable,
    };

    use super::{MathTransformOp as Op, execute_math_op, execute_math_transform};
    use crate::error::TransformError;
    use crate::formula::evaluate_calculated_variables;
    use crate::ops::{TransformContext, TransformOutput};
    use crate::test_helpers as th;

    fn run(s: &DataSheet, op: Op, mode: Epm) -> Result<TransformOutput, TransformError> {
        execute_math_op(s, &op, mode, &TransformContext::default())
    }

    fn delta(s: &DataSheet, op: Op) -> Result<TransformOutput, TransformError> {
        run(s, op, Epm::FirstOrderDeltaMethod)
    }

    #[test]
    fn aud04_09_math_rejects_survival_y_allows_nonneg_survival_time_and_rejects_negative_pow() {
        let ctx = TransformContext::default();
        let mut srv = SurvivalTable::new(1, 3).unwrap();
        srv.times = th::nums(&[10.0, 20.0, 30.0]);
        srv.cohorts[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(0.0)], vec![Cell::number(1.0)]];
        let srv_s = DataSheet::new("srv-1", "Survival", TableArchetype::Survival(srv)).unwrap();

        assert!(matches!(
            delta(&srv_s, Op::Log10),
            Err(TransformError::IncompatibleTable { operation: "math_transform_y", kind: TableKind::Survival })
        ));
        let ok_x = execute_math_transform(&srv_s, Some(&Op::MultiplyByConstant(24.0)), None, Epm::FirstOrderDeltaMethod, &ctx).unwrap();
        let TableArchetype::Survival(out_srv) = &ok_x.data_sheet.archetype else { panic!("expected Survival") };
        assert_eq!(
            (th::num(&out_srv.times[0]), th::num(&out_srv.cohorts[0].cells[0][0]), th::num(&out_srv.cohorts[0].cells[1][0])),
            (240.0, 1.0, 0.0)
        );
        let bad_x = execute_math_transform(&srv_s, Some(&Op::AddConstant(-15.0)), None, Epm::FirstOrderDeltaMethod, &ctx);
        assert!(matches!(bad_x, Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Survival, row: 0, .. }))));

        let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
        pow.data_sets[0].cells = vec![vec![Cell::number(5.0)], vec![Cell::number(15.0)]];
        let pow_s = DataSheet::new("pow-1", "Slices", TableArchetype::PartsOfWhole(pow)).unwrap();
        assert!(matches!(
            delta(&pow_s, Op::SubtractConstant(10.0)),
            Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::PartsOfWhole, row: 0, .. }))
        ));
        let ok_pow = delta(&pow_s, Op::MultiplyByConstant(2.0)).unwrap();
        let TableArchetype::PartsOfWhole(out_pow) = &ok_pow.data_sheet.archetype else { panic!("expected PartsOfWhole") };
        assert_eq!(out_pow.data_sets[0].cells[0][0].active_number(), Some(10.0));
    }

    #[test]
    fn aud04_10_math_mean_cv_zero_input_mean_blanks_cv_with_warning() {
        let rows = vec![th::nums(&[0.0, 25.0, 5.0]), vec![Cell::number(0.0), Cell::excluded_number(25.0), Cell::number(5.0)]];
        let sheet = th::xy_sheet("cv-zero-in", SubcolumnMode::MeanCvN, &[1.0, 2.0], &[rows]);
        for mode in [Epm::FirstOrderDeltaMethod, Epm::ReplicateEndpoints] {
            let out = run(&sheet, Op::AddConstant(10.0), mode).unwrap();
            let cells = &th::as_xy(&out.data_sheet).data_sets[0].cells;
            assert_eq!((cells[0][0].active_number(), &cells[0][1].value, cells[0][2].active_number()), (Some(10.0), &CellValue::Empty, Some(5.0)));
            assert!(cells[1][1].value == CellValue::Empty && cells[1][1].excluded);
            assert!(out.warnings.len() == 1 && out.warnings[0].contains("input mean is zero"), "{:?}", out.warnings);
        }
    }

    #[test]
    fn aud04_14_math_snapshots_all_calculated_mv_variables() {
        let mut mv = MultipleVariablesTable::new(3, 2).unwrap();
        for (i, t) in ["A", "B", "Tag"].into_iter().enumerate() {
            mv.variables[i].title = String::from(t);
        }
        mv.variables[0].cells = th::nums(&[10.0, 100.0]);
        mv.variables[1].set_calculated_formula("=A * 2");
        mv.variables[2].set_calculated_formula("=if(A > 50, \"Hi\", \"Lo\")");
        evaluate_calculated_variables(&mut mv, None).unwrap();
        let sheet = DataSheet::new("mv-math", "MV Math", TableArchetype::MultipleVariables(mv)).unwrap();
        let ctx = TransformContext { selected_data_sets: vec![0], ..TransformContext::default() };
        let mut out_mv = th::into_mv(execute_math_op(&sheet, &Op::Log10, Epm::FirstOrderDeltaMethod, &ctx).unwrap().data_sheet);
        assert_eq!((out_mv.variables[1].source.clone(), out_mv.variables[1].role), (VariableSource::Entered, VariableRole::Continuous));
        assert_eq!((out_mv.variables[2].source.clone(), out_mv.variables[2].role), (VariableSource::Entered, VariableRole::Categorical));
        assert!(evaluate_calculated_variables(&mut out_mv, None).unwrap().is_empty());
        assert_eq!((out_mv.variables[1].cells[0].active_number(), out_mv.variables[2].cells[1].active_text()), (Some(20.0), Some("Hi")));
    }

    #[test]
    fn aud04_38_math_batched_swapped_error_notes_exact() {
        let row = vec![Cell::number(10.0), Cell::excluded_number(1.0), Cell::number(2.0)];
        let mut sheet = th::xy_sheet("mlh-swap", SubcolumnMode::MeanLowHigh, &[1.0, 2.0, 3.0, 4.0], &[vec![row; 4]]);
        sheet.exclusion_notes.extend((0..4).map(|r| th::excl_note(r, 0, 1, "qc", &format!("r{r}"))));
        let out = delta(&sheet, Op::Preset(MathTransformPreset::LinearScale { slope: -1.0, intercept: 0.0 })).unwrap();
        let coords: Vec<_> = out.data_sheet.exclusion_notes.iter().map(|n| n.coord).collect();
        assert_eq!(coords, (0..4).map(|r| CellCoord::new(r, 0, 2)).collect::<Vec<_>>());
    }

    #[test]
    fn aud04_45_math_validates_output_sheet_and_bounds_long_titles() {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 1).unwrap();
        xy.set_x_cell(0, Cell::number(1.0)).unwrap();
        xy.data_sets[0].cells[0][0] = Cell::number(10.0);
        let sheet = DataSheet::new("s-long", "T".repeat(MAX_TITLE_BYTES), TableArchetype::Xy(xy)).unwrap();
        let ok = delta(&sheet, Op::Log10).unwrap();
        assert!(ok.data_sheet.validate().is_ok() && ok.data_sheet.title.len() <= MAX_TITLE_BYTES);
        let bad_ctx = TransformContext { output_id: Some(String::from("bad\0id")), ..TransformContext::default() };
        assert!(matches!(
            execute_math_op(&sheet, &Op::Log10, Epm::FirstOrderDeltaMethod, &bad_ctx),
            Err(TransformError::Doc(DocError::InvalidIdentifier { .. }))
        ));
    }

    #[test]
    fn aud04_48_math_concentration_log_and_cv_avoid_premature_underflow_overflow() {
        let col = vec![vec![Cell::number(1e-305)], vec![Cell::number(1e-316)], vec![Cell::number(1e305)]];
        let sheet = th::xy_sheet("conc-ext", SubcolumnMode::Single, &[1.0, 2.0, 3.0], &[col]);
        for (factor, row, expected, tol) in [(1e-25, 0, 330.0, 1e-12), (1e-9, 1, 325.0, 1e-7), (1e9, 2, -314.0, 1e-12)] {
            let op = Op::Preset(MathTransformPreset::ConcentrationToNegativeLogM { molar_unit_factor: factor });
            th::assert_close(th::num(&th::into_xy(delta(&sheet, op).unwrap().data_sheet).data_sets[0].cells[row][0]), expected, tol);
        }
        let cv_s = th::xy_sheet("cv-big", SubcolumnMode::MeanCvN, &[1.0], &[vec![th::nums(&[1e308, 5.0, 3.0])]]);
        th::assert_close(th::num(&th::into_xy(delta(&cv_s, Op::MultiplyByConstant(1.0)).unwrap().data_sheet).data_sets[0].cells[0][1]), 5.0, 1e-12);
    }

    #[test]
    fn aud04_50_user_expr_relative_derivative_and_mean_low_high_overflow_warning() {
        let nano_s = th::xy_sheet("nano", SubcolumnMode::MeanSdN, &[1.0], &[vec![th::nums(&[1e-9, 1e-10, 3.0])]]);
        let nano = delta(&nano_s, Op::UserExpression(String::from("-log10(y)"))).unwrap();
        assert!(nano.warnings.is_empty(), "unexpected warnings: {:?}", nano.warnings);
        let r0 = &th::as_xy(&nano.data_sheet).data_sets[0].cells[0];
        th::assert_close(th::num(&r0[0]), 9.0, 1e-12);
        th::assert_close(th::num(&r0[1]), 0.1 / std::f64::consts::LN_10, 1e-8);

        let bnd_s = th::xy_sheet("bnd", SubcolumnMode::MeanSdN, &[1.0], &[vec![th::nums(&[0.0, 0.01, 3.0])]]);
        let bnd = delta(&bnd_s, Op::UserExpression(String::from("if(y >= 0, 2 * y, nan)"))).unwrap();
        assert!(bnd.warnings.is_empty(), "{:?}", bnd.warnings);
        th::assert_close(th::num(&th::as_xy(&bnd.data_sheet).data_sets[0].cells[0][1]), 0.02, 1e-9);

        let lh_s = th::xy_sheet("lh-ovf", SubcolumnMode::MeanLowHigh, &[1.0], &[vec![th::nums(&[1e307, 2e307, 1e306])]]);
        let lh = run(&lh_s, Op::MultiplyByConstant(10.0), Epm::ReplicateEndpoints).unwrap();
        assert_eq!(th::as_xy(&lh.data_sheet).data_sets[0].cells[0][1].value, CellValue::Empty);
        assert!(lh.warnings.iter().any(|w| w.contains("overflowed")), "{:?}", lh.warnings);
    }
}
