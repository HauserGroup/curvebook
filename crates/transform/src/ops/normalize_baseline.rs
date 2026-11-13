//! Normalization (`0%..100%` / `0..1`) and baseline removal (`TransformAnalysisSpec::Normalize`,
//! `TransformAnalysisSpec::RemoveBaseline`, `docs/data-tables-and-transforms.md` §5.2–§5.3).

use lumencraft_doc::{
    BaselineOperation, Cell, CellValue, DataSetColumn, DataSheet, NormalizeAnchor, ResultCell, ResultScalarEntry, SheetId, SubcolumnMode,
    TableArchetype,
};
use lumencraft_math::sum;

use crate::error::TransformError;
use crate::ops::{TransformContext, TransformOutput, build_summary_tab};

/// Generalized baseline source for [`execute_remove_baseline_kind`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BaselineKind {
    /// Row-by-row mean across `columns`.
    ColumnsRowWise(Vec<usize>),
    /// Single grand mean across `columns` and all rows.
    ColumnsMean(Vec<usize>),
    /// Constant scalar baseline.
    Constant(f64),
    /// Per-column mean over inclusive row slice `start_row..=end_row`.
    RowRangeMean { start_row: usize, end_row: usize },
    /// Per-column linear baseline interpolated between mean of first `first_k` rows and last `last_k` rows.
    FirstAndLastLinear { first_k: usize, last_k: usize },
}

/// Normalize a [`DataSheet`] between `zero_anchor` and `hundred_anchor`.
pub fn execute_normalize(
    source: &DataSheet,
    zero_anchor: &NormalizeAnchor,
    hundred_anchor: &NormalizeAnchor,
    output_percent: bool,
    per_data_set: bool,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    let col_count = source.archetype.column_count();
    if col_count == 0 {
        return Err(TransformError::InvalidInput { message: String::from("table has no data columns to normalize") });
    }
    let selected = ctx.active_columns(col_count)?;
    let scale = if output_percent { 100.0 } else { 1.0 };

    let mut out = source.clone();
    let (id, title) = ctx.derived_ids(source, "normalize", "Normalized");
    out.id = SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();

    let mut scalars = Vec::new();
    let mut warnings = Vec::new();

    if per_data_set {
        for &c in &selected {
            let z = resolve_anchor(source, zero_anchor, &[c], ctx)?;
            let h = resolve_anchor(source, hundred_anchor, &[c], ctx)?;
            if (h - z).abs() == 0.0 {
                return Err(TransformError::InvalidSpec { message: format!("normalization span is zero for column {c} (zero={z}, hundred={h})") });
            }
            apply_linear_norm_column(&mut out.archetype, c, z, h, scale);
            scalars.push(ResultScalarEntry { key: String::from("ZeroAnchor"), data_set_index: Some(c), value: z, units: None });
            scalars.push(ResultScalarEntry { key: String::from("HundredAnchor"), data_set_index: Some(c), value: h, units: None });
        }
    } else {
        let z = resolve_anchor(source, zero_anchor, &selected, ctx)?;
        let h = resolve_anchor(source, hundred_anchor, &selected, ctx)?;
        if (h - z).abs() == 0.0 {
            return Err(TransformError::InvalidSpec { message: format!("global normalization span is zero (zero={z}, hundred={h})") });
        }
        for &c in &selected {
            apply_linear_norm_column(&mut out.archetype, c, z, h, scale);
        }
        scalars.push(ResultScalarEntry { key: String::from("ZeroAnchor"), data_set_index: None, value: z, units: None });
        scalars.push(ResultScalarEntry { key: String::from("HundredAnchor"), data_set_index: None, value: h, units: None });
    }

    if selected.is_empty() {
        warnings.push(String::from("no columns selected for normalization"));
    }

    let summary_tab = build_summary_tab(
        "Normalization summary",
        vec![
            ("Per data set", ResultCell::Boolean(per_data_set)),
            ("Output percent", ResultCell::Boolean(output_percent)),
            ("Columns normalized", ResultCell::Number { value: selected.len() as f64, prefix: None, units: None }),
        ],
        warnings.clone(),
    );

    Ok(TransformOutput { data_sheet: out, summary_tab, scalars, warnings })
}

/// Remove baseline using `TransformAnalysisSpec::RemoveBaseline` parameters.
pub fn execute_remove_baseline(
    source: &DataSheet,
    baseline_columns: &[usize],
    operation: BaselineOperation,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    if baseline_columns.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("baseline_columns must not be empty") });
    }
    execute_remove_baseline_kind(source, &BaselineKind::ColumnsRowWise(baseline_columns.to_vec()), operation, ctx)
}

/// Remove baseline using a generalized [`BaselineKind`].
pub fn execute_remove_baseline_kind(
    source: &DataSheet,
    baseline: &BaselineKind,
    operation: BaselineOperation,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    let col_count = source.archetype.column_count();
    let row_count = source.archetype.row_count();
    let selected = ctx.active_columns(col_count)?;

    let per_col_baselines = compute_baselines(source, baseline, &selected, col_count, row_count)?;

    let mut out = source.clone();
    let (id, title) = ctx.derived_ids(source, "baseline", "Baseline Removed");
    out.id = SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();

    let mut warnings = Vec::new();
    for (idx_pos, &col) in selected.iter().enumerate() {
        let base_rows = &per_col_baselines[idx_pos];
        apply_baseline_to_column(&mut out.archetype, col, base_rows, operation, &mut warnings);
    }

    let summary_tab = build_summary_tab(
        "Baseline removal summary",
        vec![
            ("Operation", ResultCell::Text(format!("{operation:?}"))),
            ("Columns processed", ResultCell::Number { value: selected.len() as f64, prefix: None, units: None }),
            ("Warnings", ResultCell::Number { value: warnings.len() as f64, prefix: None, units: None }),
        ],
        warnings.clone(),
    );

    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
}

fn resolve_anchor(source: &DataSheet, anchor: &NormalizeAnchor, cols: &[usize], ctx: &TransformContext<'_>) -> Result<f64, TransformError> {
    match anchor {
        NormalizeAnchor::FixedValue(v) => {
            if !v.is_finite() {
                return Err(TransformError::InvalidSpec { message: format!("FixedValue anchor must be finite, got {v}") });
            }
            Ok(*v)
        }
        NormalizeAnchor::Hooked(hook) => ctx.resolve_hook(hook, source),
        NormalizeAnchor::ControlColumnMean { column } => {
            if *column >= source.archetype.column_count() {
                return Err(TransformError::InvalidSpec { message: format!("control column {column} out of bounds") });
            }
            let vals = collect_active_primary_values(&source.archetype, &[*column]);
            sum::mean(&vals).map_err(|_| TransformError::InvalidInput { message: format!("control column {column} has no active numeric values") })
        }
        NormalizeAnchor::ColumnMin => {
            let vals = collect_active_primary_values(&source.archetype, cols);
            vals.into_iter()
                .reduce(f64::min)
                .ok_or_else(|| TransformError::InvalidInput { message: String::from("no active numeric values to compute ColumnMin anchor") })
        }
        NormalizeAnchor::ColumnMax => {
            let vals = collect_active_primary_values(&source.archetype, cols);
            vals.into_iter()
                .reduce(f64::max)
                .ok_or_else(|| TransformError::InvalidInput { message: String::from("no active numeric values to compute ColumnMax anchor") })
        }
        NormalizeAnchor::ColumnSum => {
            let vals = collect_active_primary_values(&source.archetype, cols);
            if vals.is_empty() {
                return Err(TransformError::InvalidInput { message: String::from("no active numeric values to compute ColumnSum anchor") });
            }
            Ok(sum::sum(&vals))
        }
        NormalizeAnchor::FirstRow => row_anchor_mean(&source.archetype, cols, true),
        NormalizeAnchor::LastRow => row_anchor_mean(&source.archetype, cols, false),
    }
}

fn row_anchor_mean(arch: &TableArchetype, cols: &[usize], first: bool) -> Result<f64, TransformError> {
    let row_count = arch.row_count();
    if row_count == 0 {
        return Err(TransformError::InvalidInput { message: String::from("table has 0 rows") });
    }
    let row = if first { 0 } else { row_count - 1 };
    let mut vals = Vec::new();
    for &c in cols {
        collect_row_active_numbers(arch, c, row, &mut vals);
    }
    sum::mean(&vals)
        .map_err(|_| TransformError::InvalidInput { message: format!("{} row has no active numeric values", if first { "first" } else { "last" }) })
}

fn collect_active_primary_values(arch: &TableArchetype, cols: &[usize]) -> Vec<f64> {
    let mut out = Vec::new();
    let rows = arch.row_count();
    for &c in cols {
        for r in 0..rows {
            collect_row_active_numbers(arch, c, r, &mut out);
        }
    }
    out
}

fn collect_row_active_numbers(arch: &TableArchetype, col: usize, row: usize, out: &mut Vec<f64>) {
    match arch {
        TableArchetype::Xy(t) => collect_subcells(&t.data_sets, col, row, t.y_subcolumn_mode, out),
        TableArchetype::Column(t) => collect_subcells(&t.data_sets, col, row, t.subcolumn_mode, out),
        TableArchetype::Grouped(t) => collect_subcells(&t.data_sets, col, row, t.subcolumn_mode, out),
        TableArchetype::Contingency(t) => collect_subcells(&t.data_sets, col, row, SubcolumnMode::Single, out),
        TableArchetype::Survival(t) => collect_subcells(&t.cohorts, col, row, SubcolumnMode::Single, out),
        TableArchetype::PartsOfWhole(t) => collect_subcells(&t.data_sets, col, row, SubcolumnMode::Single, out),
        TableArchetype::MultipleVariables(mv) => {
            if let Some(v) = mv.variables.get(col).and_then(|v| v.cells.get(row)).and_then(Cell::active_number) {
                out.push(v);
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(row_cells) = t.groups.get(col).and_then(|g| g.cells.get(row)) {
                for cell in row_cells {
                    if let Some(v) = cell.active_number() {
                        out.push(v);
                    }
                }
            }
        }
    }
}

fn collect_subcells(data_sets: &[DataSetColumn], col: usize, row: usize, mode: SubcolumnMode, out: &mut Vec<f64>) {
    let Some(row_cells) = data_sets.get(col).and_then(|ds| ds.cells.get(row)) else {
        return;
    };
    if mode.is_pre_summarized() {
        if let Some(v) = row_cells.first().and_then(Cell::active_number) {
            out.push(v);
        }
    } else {
        for cell in row_cells {
            if let Some(v) = cell.active_number() {
                out.push(v);
            }
        }
    }
}

fn apply_linear_norm_column(arch: &mut TableArchetype, col: usize, zero: f64, hundred: f64, scale: f64) {
    let factor = scale / (hundred - zero);
    let map_val = |v: f64| (v - zero) * factor;
    let map_spread = |s: f64| s * factor.abs();

    match arch {
        TableArchetype::Xy(t) => normalize_dataset_col(&mut t.data_sets, col, t.y_subcolumn_mode, map_val, map_spread),
        TableArchetype::Column(t) => normalize_dataset_col(&mut t.data_sets, col, t.subcolumn_mode, map_val, map_spread),
        TableArchetype::Grouped(t) => normalize_dataset_col(&mut t.data_sets, col, t.subcolumn_mode, map_val, map_spread),
        TableArchetype::Contingency(t) => normalize_dataset_col(&mut t.data_sets, col, SubcolumnMode::Single, map_val, map_spread),
        TableArchetype::Survival(t) => normalize_dataset_col(&mut t.cohorts, col, SubcolumnMode::Single, map_val, map_spread),
        TableArchetype::PartsOfWhole(t) => normalize_dataset_col(&mut t.data_sets, col, SubcolumnMode::Single, map_val, map_spread),
        TableArchetype::MultipleVariables(mv) => {
            if let Some(var) = mv.variables.get_mut(col) {
                for cell in &mut var.cells {
                    if let Some(v) = cell.value.finite_f64() {
                        cell.value = CellValue::from_finite_or_empty(map_val(v));
                    }
                }
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                for row_cells in &mut grp.cells {
                    for cell in row_cells {
                        if let Some(v) = cell.value.finite_f64() {
                            cell.value = CellValue::from_finite_or_empty(map_val(v));
                        }
                    }
                }
            }
        }
    }
}

fn normalize_dataset_col(
    data_sets: &mut [DataSetColumn],
    col: usize,
    mode: SubcolumnMode,
    map_val: impl Fn(f64) -> f64,
    map_spread: impl Fn(f64) -> f64,
) {
    let Some(ds) = data_sets.get_mut(col) else {
        return;
    };
    for row_cells in &mut ds.cells {
        match mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
                for cell in row_cells {
                    if let Some(v) = cell.value.finite_f64() {
                        cell.value = CellValue::from_finite_or_empty(map_val(v));
                    }
                }
            }
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    row_cells[0].value = CellValue::from_finite_or_empty(map_val(m));
                }
                if let Some(s) = row_cells.get(1).and_then(|c| c.value.finite_f64()) {
                    row_cells[1].value = CellValue::from_finite_or_empty(map_spread(s));
                }
            }
            SubcolumnMode::MeanCvN => {
                if let (Some(m), Some(cv)) =
                    (row_cells.first().and_then(|c| c.value.finite_f64()), row_cells.get(1).and_then(|c| c.value.finite_f64()))
                {
                    let sd_in = (cv / 100.0) * m.abs();
                    let m_out = map_val(m);
                    let sd_out = map_spread(sd_in);
                    row_cells[0].value = CellValue::from_finite_or_empty(m_out);
                    row_cells[1].value = if m_out != 0.0 { CellValue::from_finite_or_empty(100.0 * sd_out / m_out.abs()) } else { CellValue::Empty };
                }
            }
            SubcolumnMode::MeanLowHigh => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    row_cells[0].value = CellValue::from_finite_or_empty(map_val(m));
                }
                let lo = row_cells.get(1).and_then(|c| c.value.finite_f64()).map(&map_spread);
                let hi = row_cells.get(2).and_then(|c| c.value.finite_f64()).map(&map_spread);
                if row_cells.len() >= 3 {
                    row_cells[1].value = lo.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
                    row_cells[2].value = hi.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
                }
            }
            SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    row_cells[0].value = CellValue::from_finite_or_empty(map_val(m));
                }
                if let (Some(a), Some(b)) = (
                    row_cells.get(1).and_then(|c| c.value.finite_f64()).map(&map_val),
                    row_cells.get(2).and_then(|c| c.value.finite_f64()).map(&map_val),
                ) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    row_cells[1].value = CellValue::from_finite_or_empty(lo);
                    row_cells[2].value = CellValue::from_finite_or_empty(hi);
                }
            }
        }
    }
}

fn compute_baselines(
    source: &DataSheet,
    baseline: &BaselineKind,
    selected: &[usize],
    col_count: usize,
    row_count: usize,
) -> Result<Vec<Vec<Option<f64>>>, TransformError> {
    match baseline {
        BaselineKind::ColumnsRowWise(cols) => {
            validate_baseline_cols(cols, col_count)?;
            let mut row_bases = Vec::with_capacity(row_count);
            for r in 0..row_count {
                let mut vals = Vec::new();
                for &c in cols {
                    collect_row_active_numbers(&source.archetype, c, r, &mut vals);
                }
                row_bases.push(sum::mean(&vals).ok());
            }
            Ok(vec![row_bases; selected.len()])
        }
        BaselineKind::ColumnsMean(cols) => {
            validate_baseline_cols(cols, col_count)?;
            let vals = collect_active_primary_values(&source.archetype, cols);
            let grand = sum::mean(&vals)
                .map_err(|_| TransformError::InvalidInput { message: String::from("baseline columns contain no active numeric values") })?;
            Ok(vec![vec![Some(grand); row_count]; selected.len()])
        }
        BaselineKind::Constant(v) => {
            if !v.is_finite() {
                return Err(TransformError::InvalidSpec { message: format!("baseline constant must be finite, got {v}") });
            }
            Ok(vec![vec![Some(*v); row_count]; selected.len()])
        }
        BaselineKind::RowRangeMean { start_row, end_row } => {
            if row_count == 0 || *start_row > *end_row || *end_row >= row_count {
                return Err(TransformError::InvalidSpec {
                    message: format!("invalid baseline row range {start_row}..={end_row} for row_count={row_count}"),
                });
            }
            let mut out = Vec::with_capacity(selected.len());
            for &c in selected {
                let mut vals = Vec::new();
                for r in *start_row..=*end_row {
                    collect_row_active_numbers(&source.archetype, c, r, &mut vals);
                }
                let m = sum::mean(&vals).ok();
                out.push(vec![m; row_count]);
            }
            Ok(out)
        }
        BaselineKind::FirstAndLastLinear { first_k, last_k } => {
            if *first_k == 0 || *last_k == 0 || *first_k > row_count || *last_k > row_count {
                return Err(TransformError::InvalidSpec {
                    message: format!("invalid FirstAndLastLinear window counts ({first_k}, {last_k}) for row_count={row_count}"),
                });
            }
            let mut out = Vec::with_capacity(selected.len());
            for &c in selected {
                out.push(linear_baseline_for_col(&source.archetype, c, row_count, *first_k, *last_k));
            }
            Ok(out)
        }
    }
}

fn validate_baseline_cols(cols: &[usize], col_count: usize) -> Result<(), TransformError> {
    if cols.is_empty() {
        return Err(TransformError::InvalidSpec { message: String::from("baseline column list must not be empty") });
    }
    for &c in cols {
        if c >= col_count {
            return Err(TransformError::InvalidSpec { message: format!("baseline column {c} exceeds column count {col_count}") });
        }
    }
    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn linear_baseline_for_col(arch: &TableArchetype, col: usize, row_count: usize, first_k: usize, last_k: usize) -> Vec<Option<f64>> {
    let mut first_vals = Vec::new();
    for r in 0..first_k {
        collect_row_active_numbers(arch, col, r, &mut first_vals);
    }
    let mut last_vals = Vec::new();
    for r in (row_count - last_k)..row_count {
        collect_row_active_numbers(arch, col, r, &mut last_vals);
    }
    let (Ok(y_start), Ok(y_end)) = (sum::mean(&first_vals), sum::mean(&last_vals)) else {
        return vec![None; row_count];
    };
    let x_start = ((first_k - 1) as f64) * 0.5;
    let x_end = ((row_count - last_k) as f64) + ((last_k - 1) as f64) * 0.5;
    let span = x_end - x_start;
    (0..row_count)
        .map(|r| {
            if span == 0.0 {
                Some(0.5 * (y_start + y_end))
            } else {
                let t = ((r as f64) - x_start) / span;
                Some(y_start + t * (y_end - y_start))
            }
        })
        .collect()
}

fn apply_baseline_to_column(
    arch: &mut TableArchetype,
    col: usize,
    baselines: &[Option<f64>],
    operation: BaselineOperation,
    warnings: &mut Vec<String>,
) {
    match arch {
        TableArchetype::Xy(t) => apply_baseline_datasets(&mut t.data_sets, col, t.y_subcolumn_mode, baselines, operation, warnings),
        TableArchetype::Column(t) => apply_baseline_datasets(&mut t.data_sets, col, t.subcolumn_mode, baselines, operation, warnings),
        TableArchetype::Grouped(t) => apply_baseline_datasets(&mut t.data_sets, col, t.subcolumn_mode, baselines, operation, warnings),
        TableArchetype::Contingency(t) => apply_baseline_datasets(&mut t.data_sets, col, SubcolumnMode::Single, baselines, operation, warnings),
        TableArchetype::Survival(t) => apply_baseline_datasets(&mut t.cohorts, col, SubcolumnMode::Single, baselines, operation, warnings),
        TableArchetype::PartsOfWhole(t) => apply_baseline_datasets(&mut t.data_sets, col, SubcolumnMode::Single, baselines, operation, warnings),
        TableArchetype::MultipleVariables(mv) => {
            if let Some(var) = mv.variables.get_mut(col) {
                for (r, cell) in var.cells.iter_mut().enumerate() {
                    apply_baseline_cell(cell, baselines.get(r).copied().flatten(), operation, r, col, warnings);
                }
            }
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                for (r, row_cells) in grp.cells.iter_mut().enumerate() {
                    let b = baselines.get(r).copied().flatten();
                    for cell in row_cells {
                        apply_baseline_cell(cell, b, operation, r, col, warnings);
                    }
                }
            }
        }
    }
}

fn apply_baseline_datasets(
    data_sets: &mut [DataSetColumn],
    col: usize,
    mode: SubcolumnMode,
    baselines: &[Option<f64>],
    operation: BaselineOperation,
    warnings: &mut Vec<String>,
) {
    let Some(ds) = data_sets.get_mut(col) else {
        return;
    };
    for (r, row_cells) in ds.cells.iter_mut().enumerate() {
        let b_opt = baselines.get(r).copied().flatten();
        match mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
                for cell in row_cells {
                    apply_baseline_cell(cell, b_opt, operation, r, col, warnings);
                }
            }
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    let res = b_opt.and_then(|b| apply_baseline_op(m, b, operation));
                    row_cells[0].value = res.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
                }
                if let (Some(s), Some(b)) = (row_cells.get(1).and_then(|c| c.value.finite_f64()), b_opt) {
                    let scale = baseline_spread_scale(b, operation);
                    row_cells[1].value = scale.map_or(CellValue::Empty, |sc| CellValue::from_finite_or_empty(s * sc));
                } else if row_cells.len() > 1 {
                    row_cells[1].value = CellValue::Empty;
                }
            }
            SubcolumnMode::MeanCvN => {
                if let (Some(m), Some(cv), Some(b)) =
                    (row_cells.first().and_then(|c| c.value.finite_f64()), row_cells.get(1).and_then(|c| c.value.finite_f64()), b_opt)
                {
                    let sd_in = (cv / 100.0) * m.abs();
                    let m_out = apply_baseline_op(m, b, operation);
                    let sd_out = baseline_spread_scale(b, operation).map(|sc| sd_in * sc);
                    row_cells[0].value = m_out.map_or(CellValue::Empty, CellValue::from_finite_or_empty);
                    row_cells[1].value = match (m_out, sd_out) {
                        (Some(mo), Some(so)) if mo != 0.0 => CellValue::from_finite_or_empty(100.0 * so / mo.abs()),
                        _ => CellValue::Empty,
                    };
                }
            }
            SubcolumnMode::MeanLowHigh => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    row_cells[0].value =
                        b_opt.and_then(|b| apply_baseline_op(m, b, operation)).map_or(CellValue::Empty, CellValue::from_finite_or_empty);
                }
                let scale = b_opt.and_then(|b| baseline_spread_scale(b, operation));
                for idx in 1..3 {
                    if let Some(cell) = row_cells.get_mut(idx) {
                        cell.value = match (cell.value.finite_f64(), scale) {
                            (Some(v), Some(sc)) => CellValue::from_finite_or_empty(v * sc),
                            _ => CellValue::Empty,
                        };
                    }
                }
            }
            SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                for cell in row_cells {
                    apply_baseline_cell(cell, b_opt, operation, r, col, warnings);
                }
            }
        }
    }
}

fn apply_baseline_cell(cell: &mut Cell, b_opt: Option<f64>, operation: BaselineOperation, row: usize, col: usize, warnings: &mut Vec<String>) {
    let Some(v) = cell.value.finite_f64() else {
        return;
    };
    let Some(b) = b_opt else {
        cell.value = CellValue::Empty;
        return;
    };
    if let Some(out) = apply_baseline_op(v, b, operation) {
        cell.value = CellValue::Number(out);
    } else {
        cell.value = CellValue::Empty;
        if !cell.excluded {
            warnings.push(format!("row {row} col {col}: baseline operation undefined for baseline={b}"));
        }
    }
}

fn apply_baseline_op(v: f64, b: f64, op: BaselineOperation) -> Option<f64> {
    let out = match op {
        BaselineOperation::Difference => v - b,
        BaselineOperation::Ratio => (b != 0.0).then(|| v / b)?,
        BaselineOperation::PercentChange => (b != 0.0).then(|| 100.0 * (v - b) / b)?,
        BaselineOperation::FractionalChange => (b != 0.0).then(|| (v - b) / b)?,
    };
    out.is_finite().then_some(out)
}

fn baseline_spread_scale(b: f64, op: BaselineOperation) -> Option<f64> {
    let sc = match op {
        BaselineOperation::Difference => 1.0,
        BaselineOperation::Ratio | BaselineOperation::FractionalChange => (b != 0.0).then(|| 1.0 / b.abs())?,
        BaselineOperation::PercentChange => (b != 0.0).then(|| 100.0 / b.abs())?,
    };
    sc.is_finite().then_some(sc)
}
