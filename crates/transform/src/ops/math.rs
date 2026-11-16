//! Mathematical transforms on X and/or Y series with analytical and endpoint error
//! propagation (`TransformAnalysisSpec::Math`, `docs/data-tables-and-transforms.md` §5.1).

use curvebook_doc::{
    Cell, CellValue, DataSetColumn, DataSheet, ErrorPropagationMode, InfoSheet, MathTransformPreset, ResultCell, SubcolumnMode, TableArchetype,
    XColumnMode,
};
use curvebook_math::dist;

use crate::error::TransformError;
use crate::formula::{Expr, ScalarEnv, eval_scalar_expr, parse_formula};
use crate::ops::{TransformContext, TransformOutput, build_summary_tab};

/// Extended mathematical transform operation supporting presets, constant arithmetic, arbitrary
/// antilogs, and custom user expressions.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MathTransformOp {
    Preset(MathTransformPreset),
    Log10,
    Ln,
    Log2,
    Exp10,
    ExpE,
    Reciprocal,
    Square,
    Sqrt,
    MultiplyByConstant(f64),
    DivideByConstant(f64),
    AddConstant(f64),
    SubtractConstant(f64),
    Logit,
    Antilog { base: f64 },
    Probit,
    ArcsinSqrt,
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
        transform_archetype_y(
            &mut out.archetype,
            &selected,
            cy,
            error_propagation,
            &x_values_before,
            ctx.info_sheet,
            &mut transformed_cells,
            &mut warnings,
        )?;
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
) -> Result<(), TransformError> {
    match arch {
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
        }
    }
    Ok(())
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
) {
    for &c in selected {
        let Some(ds) = data_sets.get_mut(c) else {
            continue;
        };
        for (r, row_cells) in ds.cells.iter_mut().enumerate() {
            let x_val = x_ctx.get(r).copied().flatten();
            transform_row_slice(row_cells, mode, op, err_mode, x_val, false, info, r, c, count, warnings);
        }
    }
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
) {
    match mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
            for cell in row_cells.iter_mut() {
                transform_single_cell(cell, op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
            }
        }
        SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN | SubcolumnMode::MeanCvN => {
            transform_parametric_summary(row_cells, mode, op, err_mode, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
        }
        SubcolumnMode::MeanLowHigh => {
            transform_mean_low_high(row_cells, op, err_mode, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
        }
        SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
            transform_explicit_limits(row_cells, op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
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
                Some(100.0 * sd_out / mean_out.abs())
            } else {
                None
            }
        }
        (_, other) => other,
    };

    row_cells[1] = match final_spread {
        Some(s) if s.is_finite() => Cell { value: CellValue::Number(s), excluded: spread_excluded },
        _ => Cell { value: CellValue::Empty, excluded: spread_excluded },
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
) {
    if row_cells.len() < 3 {
        return;
    }
    let Some(mean_in) = row_cells[0].value.finite_f64() else {
        return;
    };
    let mean_excluded = row_cells[0].excluded;
    let Some(mean_out) = op.eval(mean_in, x_val, is_x_axis, info) else {
        row_cells[0].value = CellValue::Empty;
        row_cells[1].value = CellValue::Empty;
        row_cells[2].value = CellValue::Empty;
        if !mean_excluded {
            warnings.push(format!("row {row_idx} col {col_idx}: mean {mean_in} outside transform domain"));
        }
        return;
    };
    row_cells[0] = Cell { value: CellValue::Number(mean_out), excluded: mean_excluded };
    *count += 1;

    let low_in = row_cells[1].value.finite_f64();
    let high_in = row_cells[2].value.finite_f64();
    match err_mode {
        ErrorPropagationMode::FirstOrderDeltaMethod => {
            let slope = op.derivative(mean_in, x_val, is_x_axis, info).map(f64::abs);
            row_cells[1].value = match (low_in, slope) {
                (Some(d), Some(s)) => CellValue::from_finite_or_empty(d * s),
                _ => CellValue::Empty,
            };
            row_cells[2].value = match (high_in, slope) {
                (Some(u), Some(s)) => CellValue::from_finite_or_empty(u * s),
                _ => CellValue::Empty,
            };
        }
        ErrorPropagationMode::ReplicateEndpoints => {
            let t_lo = low_in.and_then(|d| op.eval(mean_in - d, x_val, is_x_axis, info));
            let t_hi = high_in.and_then(|u| op.eval(mean_in + u, x_val, is_x_axis, info));
            if let (Some(a), Some(b)) = (t_lo, t_hi) {
                let (lo_end, hi_end) = if a <= b { (a, b) } else { (b, a) };
                row_cells[1].value = CellValue::from_finite_or_empty((mean_out - lo_end).max(0.0));
                row_cells[2].value = CellValue::from_finite_or_empty((hi_end - mean_out).max(0.0));
            } else {
                row_cells[1].value = CellValue::Empty;
                row_cells[2].value = CellValue::Empty;
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
) {
    if row_cells.len() < 3 {
        return;
    }
    transform_single_cell(&mut row_cells[0], op, x_val, is_x_axis, info, row_idx, col_idx, count, warnings);
    let t1 = row_cells[1].value.finite_f64().and_then(|v| op.eval(v, x_val, is_x_axis, info));
    let t2 = row_cells[2].value.finite_f64().and_then(|v| op.eval(v, x_val, is_x_axis, info));
    match (t1, t2) {
        (Some(a), Some(b)) => {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            row_cells[1].value = CellValue::from_finite_or_empty(lo);
            row_cells[2].value = CellValue::from_finite_or_empty(hi);
        }
        _ => {
            row_cells[1].value = t1.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
            row_cells[2].value = t2.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
        }
    }
}
