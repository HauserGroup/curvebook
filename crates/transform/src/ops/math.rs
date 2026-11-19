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
    Cell, CellValue, DataSetColumn, DataSheet, ErrorPropagationMode, InfoSheet, MathTransformPreset, ResultCell, SubcolumnMode, TableArchetype,
    XColumnMode,
};
use curvebook_math::dist;

use crate::error::TransformError;
use crate::formula::{Expr, ScalarEnv, eval_scalar_expr, parse_formula};
use crate::ops::{TransformContext, TransformOutput, build_summary_tab, remap_swapped_error_notes, swap_error_bound_cells};

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
        /// Positive exponential base (`> 0`).
        base: f64,
    },
    /// Inverse standard normal CDF `Phi^{-1}(u)` (`u in (0, 1)`).
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
                if *base <= 0.0 {
                    return Err(TransformError::InvalidSpec { message: format!("Antilog base must be > 0, got {base}") });
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
                let h = 1e-6 * v.abs().max(1.0);
                let f_hi = eval_user_expr(expr, v + h, x_ctx, is_x_axis, info)?;
                let f_lo = eval_user_expr(expr, v - h, x_ctx, is_x_axis, info)?;
                (f_hi - f_lo) / (2.0 * h)
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
            let m = v * molar_unit_factor;
            (m > 0.0).then(|| -m.log10())
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

    let x_values_before: Vec<Option<f64>> = match &source.archetype {
        TableArchetype::Xy(xy) => (0..xy.row_count()).map(|r| xy.raw_x_value_at(r)).collect(),
        _ => Vec::new(),
    };

    if let Some(ref cx) = compiled_x
        && let TableArchetype::Xy(xy) = &mut out.archetype
    {
        transform_xy_x_axis(xy, cx, error_propagation, ctx.info_sheet, &mut transformed_cells, &mut warnings);
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
            ctx.info_sheet,
            &mut transformed_cells,
            &mut warnings,
        )?;
        for &(row, col) in &swapped {
            remap_swapped_error_notes(&mut out.exclusion_notes, &[row], col);
        }
    }

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
        let mut temp_col = DataSetColumn { title: xy.x_title.clone(), cells: vec![row_cells.clone()] };
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
            transform_dataset_vec(&mut t.data_sets, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings)
        }
        TableArchetype::Survival(t) => {
            transform_dataset_vec(&mut t.cohorts, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings)
        }
        TableArchetype::PartsOfWhole(t) => {
            transform_dataset_vec(&mut t.data_sets, SubcolumnMode::Single, selected, op, err_mode, x_ctx, info, count, warnings)
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
        TableArchetype::MultipleVariables(mv) => {
            for &c in selected {
                if let Some(var) = mv.variables.get_mut(c) {
                    for (r, cell) in var.cells.iter_mut().enumerate() {
                        let mut one = [cell.clone()];
                        transform_row_slice(&mut one, SubcolumnMode::Single, op, err_mode, None, false, info, r, c, count, warnings);
                        *cell = one[0].clone();
                    }
                }
            }
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
        SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => {
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
    if row_cells.len() < 3 {
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
                (Some(a), Some(b)) => Some(0.5 * (b - a).abs()),
                _ => None,
            }
        }
    };

    let final_spread = match (mode, abs_spread_out) {
        (SubcolumnMode::MeanCvN, Some(sd_out)) => {
            if mean_out != 0.0 {
                let cv = 100.0 * sd_out / mean_out.abs();
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
                    row_cells[1].value = CellValue::from_finite_or_empty((mean_out - lo_end).max(0.0));
                    row_cells[2].value = CellValue::from_finite_or_empty((hi_end - mean_out).max(0.0));
                    swapped
                }
                (Some(a), None) => {
                    row_cells[1].value = CellValue::from_finite_or_empty((mean_out - a).abs());
                    row_cells[2].value = CellValue::Empty;
                    if a > mean_out { swap_error_bound_cells(row_cells) } else { false }
                }
                (None, Some(b)) => {
                    row_cells[1].value = CellValue::Empty;
                    row_cells[2].value = CellValue::from_finite_or_empty((b - mean_out).abs());
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
