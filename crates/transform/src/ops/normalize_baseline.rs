//! Normalization (`0%..100%` / `0..1`) and baseline removal (`TransformAnalysisSpec::Normalize`,
//! `TransformAnalysisSpec::RemoveBaseline`, `docs/implementation/data-tables-and-transforms.md` §5.2–§5.3).

use curvebook_doc::{
    BaselineOperation, Cell, CellValue, DataSetColumn, DataSheet, NormalizeAnchor, ResultCell, ResultScalarEntry, SheetId, SubcolumnMode,
    TableArchetype, TableKind,
};
use curvebook_math::sum;

use crate::error::TransformError;
use crate::ops::{
    TransformContext, TransformOutput, build_summary_tab, remap_swapped_error_notes_batch, snapshot_calculated_variables, swap_error_bound_cells,
    validate_parts_of_whole_non_negative,
};

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
    RowRangeMean {
        /// Zero-based inclusive starting row index of the baseline window.
        start_row: usize,
        /// Zero-based inclusive ending row index of the baseline window (`end_row >= start_row`).
        end_row: usize,
    },
    /// Per-column linear baseline interpolated between active-observation centroids `(x_start, y_start)`
    /// across rows `0..first_k` and `(x_end, y_end)` across rows `(row_count - last_k)..row_count`
    /// (weighting each row by its active replicate count; falling back to `0.5 * (y_start + y_end)` when `x_end == x_start`).
    FirstAndLastLinear {
        /// Number of leading rows (`>= 1`) whose active observations define `(x_start, y_start)`.
        first_k: usize,
        /// Number of trailing rows (`>= 1`) whose active observations define `(x_end, y_end)`.
        last_k: usize,
    },
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
    source.archetype.validate()?;
    if matches!(source.archetype, TableArchetype::Survival(_)) {
        return Err(TransformError::IncompatibleTable { operation: "Normalize", kind: TableKind::Survival });
    }
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
    let mut all_swapped = Vec::new();

    if per_data_set {
        for &c in &selected {
            let z = resolve_anchor(source, zero_anchor, &[c], ctx)?;
            let h = resolve_anchor(source, hundred_anchor, &[c], ctx)?;
            validate_anchor_span(z, h, Some(c))?;
            let swapped = apply_linear_norm_column(&mut out.archetype, c, z, h, scale, &mut warnings);
            all_swapped.extend(swapped.into_iter().map(|r| (r, c)));
            scalars.push(ResultScalarEntry { key: String::from("ZeroAnchor"), data_set_index: Some(c), value: z, units: None });
            scalars.push(ResultScalarEntry { key: String::from("HundredAnchor"), data_set_index: Some(c), value: h, units: None });
        }
    } else {
        let z = resolve_anchor(source, zero_anchor, &selected, ctx)?;
        let h = resolve_anchor(source, hundred_anchor, &selected, ctx)?;
        validate_anchor_span(z, h, None)?;
        for &c in &selected {
            let swapped = apply_linear_norm_column(&mut out.archetype, c, z, h, scale, &mut warnings);
            all_swapped.extend(swapped.into_iter().map(|r| (r, c)));
        }
        scalars.push(ResultScalarEntry { key: String::from("ZeroAnchor"), data_set_index: None, value: z, units: None });
        scalars.push(ResultScalarEntry { key: String::from("HundredAnchor"), data_set_index: None, value: h, units: None });
    }
    remap_swapped_error_notes_batch(&mut out.exclusion_notes, &all_swapped);
    snapshot_calculated_variables(&mut out.archetype);

    if let TableArchetype::Contingency(t) = &out.archetype {
        t.validate_counts()?;
    }
    if let TableArchetype::PartsOfWhole(t) = &out.archetype {
        validate_parts_of_whole_non_negative(t)?;
    }
    out.validate()?;

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

fn validate_anchor_span(z: f64, h: f64, col: Option<usize>) -> Result<(), TransformError> {
    if !z.is_finite() || !h.is_finite() {
        return Err(TransformError::InvalidSpec { message: format!("normalization anchors must be finite (zero={z}, hundred={h})") });
    }
    if z == h {
        let scope = col.map_or_else(|| String::from("global"), |c| format!("for column {c}"));
        return Err(TransformError::InvalidSpec { message: format!("normalization span is zero {scope} (zero={z}, hundred={h})") });
    }
    Ok(())
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
    source.archetype.validate()?;
    if matches!(source.archetype, TableArchetype::Survival(_)) {
        return Err(TransformError::IncompatibleTable { operation: "RemoveBaseline", kind: TableKind::Survival });
    }
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
    let mut all_swapped = Vec::new();
    for (idx_pos, &col) in selected.iter().enumerate() {
        let base_rows = &per_col_baselines[idx_pos];
        let swapped = apply_baseline_to_column(&mut out.archetype, col, base_rows, operation, &mut warnings);
        all_swapped.extend(swapped.into_iter().map(|r| (r, col)));
    }
    remap_swapped_error_notes_batch(&mut out.exclusion_notes, &all_swapped);
    snapshot_calculated_variables(&mut out.archetype);

    if let TableArchetype::Contingency(t) = &out.archetype {
        t.validate_counts()?;
    }
    if let TableArchetype::PartsOfWhole(t) = &out.archetype {
        validate_parts_of_whole_non_negative(t)?;
    }
    out.validate()?;

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
    let mut vals = Vec::new();
    for step in 0..row_count {
        let row = if first { step } else { row_count - 1 - step };
        vals.clear();
        for &c in cols {
            collect_row_active_numbers(arch, c, row, &mut vals);
        }
        if !vals.is_empty() {
            return sum::mean(&vals).map_err(|_| TransformError::InvalidInput {
                message: format!("{} row has no active numeric values", if first { "first" } else { "last" }),
            });
        }
    }
    Err(TransformError::InvalidInput { message: format!("{} row has no active numeric values", if first { "first" } else { "last" }) })
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

fn assign_norm_cell(cell: &mut Cell, out: f64, row: usize, col: usize, label: &str, warnings: &mut Vec<String>) {
    if out.is_finite() {
        cell.value = CellValue::Number(out);
    } else {
        cell.value = CellValue::Empty;
        if !cell.excluded {
            warnings.push(format!("row {row} col {col}: normalized {label} overflowed finite f64 range; cell blanked"));
        }
    }
}

fn apply_linear_norm_column(arch: &mut TableArchetype, col: usize, zero: f64, hundred: f64, scale: f64, warnings: &mut Vec<String>) -> Vec<usize> {
    let span = hundred - zero;
    let factor_opt = span.is_finite().then(|| scale / span).filter(|f| f.is_finite());
    let half_span = 0.5 * hundred - 0.5 * zero;
    let reverses = hundred < zero;
    let map_val = |v: f64| -> f64 {
        if let Some(factor) = factor_opt
            && (v - zero).is_finite()
        {
            return (v - zero) * factor;
        }
        if span.is_finite() && (v - zero).is_finite() {
            return ((v - zero) / span) * scale;
        }
        ((0.5 * v - 0.5 * zero) / half_span) * scale
    };
    let map_spread = |s: f64| -> f64 {
        if let Some(factor) = factor_opt {
            return s * factor.abs();
        }
        if span.is_finite() {
            return (s / span.abs()) * scale;
        }
        ((0.5 * s) / half_span.abs()) * scale
    };

    match arch {
        TableArchetype::Xy(t) => normalize_dataset_col(&mut t.data_sets, col, t.y_subcolumn_mode, reverses, map_val, map_spread, warnings),
        TableArchetype::Column(t) => normalize_dataset_col(&mut t.data_sets, col, t.subcolumn_mode, reverses, map_val, map_spread, warnings),
        TableArchetype::Grouped(t) => normalize_dataset_col(&mut t.data_sets, col, t.subcolumn_mode, reverses, map_val, map_spread, warnings),
        TableArchetype::Contingency(t) => {
            normalize_dataset_col(&mut t.data_sets, col, SubcolumnMode::Single, reverses, map_val, map_spread, warnings)
        }
        TableArchetype::Survival(t) => normalize_dataset_col(&mut t.cohorts, col, SubcolumnMode::Single, reverses, map_val, map_spread, warnings),
        TableArchetype::PartsOfWhole(t) => {
            normalize_dataset_col(&mut t.data_sets, col, SubcolumnMode::Single, reverses, map_val, map_spread, warnings)
        }
        TableArchetype::MultipleVariables(mv) => {
            if let Some(var) = mv.variables.get_mut(col) {
                for (r, cell) in var.cells.iter_mut().enumerate() {
                    if let Some(v) = cell.value.finite_f64() {
                        assign_norm_cell(cell, map_val(v), r, col, "cell", warnings);
                    }
                }
            }
            Vec::new()
        }
        TableArchetype::Nested(t) => {
            if let Some(grp) = t.groups.get_mut(col) {
                for (r, row_cells) in grp.cells.iter_mut().enumerate() {
                    for cell in row_cells {
                        if let Some(v) = cell.value.finite_f64() {
                            assign_norm_cell(cell, map_val(v), r, col, "cell", warnings);
                        }
                    }
                }
            }
            Vec::new()
        }
    }
}

fn normalize_mean_cv_row(
    row_cells: &mut [Cell],
    r: usize,
    col: usize,
    map_val: &impl Fn(f64) -> f64,
    map_spread: &impl Fn(f64) -> f64,
    warnings: &mut Vec<String>,
) {
    let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) else {
        return;
    };
    let m_out = map_val(m);
    assign_norm_cell(&mut row_cells[0], m_out, r, col, "mean", warnings);
    let Some(cv) = row_cells.get(1).and_then(|c| c.value.finite_f64()) else {
        return;
    };
    if m == 0.0 {
        row_cells[1].value = CellValue::Empty;
        if !row_cells[1].excluded {
            warnings.push(format!("row {r} col {col}: input mean is zero; CV% spread is undefined and blanked"));
        }
        return;
    }
    if m_out.is_finite() && m_out != 0.0 {
        let sd_in = (cv / 100.0) * m.abs();
        let sd_out = map_spread(sd_in);
        let cv_out = (sd_out / m_out.abs()) * 100.0;
        if cv_out.is_finite() {
            row_cells[1].value = CellValue::Number(cv_out);
        } else {
            row_cells[1].value = CellValue::Empty;
            if !row_cells[1].excluded {
                warnings.push(format!("row {r} col {col}: normalized CV% overflowed; spread blanked"));
            }
        }
    } else {
        row_cells[1].value = CellValue::Empty;
        if !row_cells[1].excluded {
            let reason = if m_out == 0.0 { "normalized mean is zero" } else { "normalized mean overflowed" };
            warnings.push(format!("row {r} col {col}: {reason}; CV% spread blanked"));
        }
    }
}

fn normalize_dataset_col(
    data_sets: &mut [DataSetColumn],
    col: usize,
    mode: SubcolumnMode,
    reverses: bool,
    map_val: impl Fn(f64) -> f64,
    map_spread: impl Fn(f64) -> f64,
    warnings: &mut Vec<String>,
) -> Vec<usize> {
    let Some(ds) = data_sets.get_mut(col) else {
        return Vec::new();
    };
    let mut swapped_rows = Vec::new();
    for (r, row_cells) in ds.cells.iter_mut().enumerate() {
        match mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
                for cell in row_cells {
                    if let Some(v) = cell.value.finite_f64() {
                        assign_norm_cell(cell, map_val(v), r, col, "cell", warnings);
                    }
                }
            }
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    assign_norm_cell(&mut row_cells[0], map_val(m), r, col, "mean", warnings);
                }
                if let Some(s) = row_cells.get(1).and_then(|c| c.value.finite_f64()) {
                    assign_norm_cell(&mut row_cells[1], map_spread(s), r, col, "spread", warnings);
                }
            }
            SubcolumnMode::MeanCvN => {
                normalize_mean_cv_row(row_cells, r, col, &map_val, &map_spread, warnings);
            }
            SubcolumnMode::MeanLowHigh => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    assign_norm_cell(&mut row_cells[0], map_val(m), r, col, "mean", warnings);
                }
                if row_cells.len() >= 3 {
                    if let Some(lo) = row_cells[1].value.finite_f64() {
                        assign_norm_cell(&mut row_cells[1], map_spread(lo), r, col, "lower bound", warnings);
                    }
                    if let Some(hi) = row_cells[2].value.finite_f64() {
                        assign_norm_cell(&mut row_cells[2], map_spread(hi), r, col, "upper bound", warnings);
                    }
                    // Decreasing linear normalization (zero > hundred) reverses -Error and +Error.
                    if reverses && swap_error_bound_cells(row_cells) {
                        swapped_rows.push(r);
                    }
                }
            }
            SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                if let Some(m) = row_cells.first().and_then(|c| c.value.finite_f64()) {
                    assign_norm_cell(&mut row_cells[0], map_val(m), r, col, "center", warnings);
                }
                if row_cells.len() >= 3 {
                    if let Some(v1) = row_cells[1].value.finite_f64() {
                        assign_norm_cell(&mut row_cells[1], map_val(v1), r, col, "lower limit", warnings);
                    }
                    if let Some(v2) = row_cells[2].value.finite_f64() {
                        assign_norm_cell(&mut row_cells[2], map_val(v2), r, col, "upper limit", warnings);
                    }
                    let should_swap = match (row_cells[1].value.finite_f64(), row_cells[2].value.finite_f64()) {
                        (Some(a), Some(b)) => a > b || (a == b && reverses),
                        _ => reverses,
                    };
                    if should_swap && swap_error_bound_cells(row_cells) {
                        swapped_rows.push(r);
                    }
                }
            }
        }
    }
    swapped_rows
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
fn window_centroid(arch: &TableArchetype, col: usize, rows: std::ops::Range<usize>) -> Option<(f64, f64)> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut row_vals = Vec::new();
    for r in rows {
        row_vals.clear();
        collect_row_active_numbers(arch, col, r, &mut row_vals);
        for &v in &row_vals {
            xs.push(r as f64);
            ys.push(v);
        }
    }
    match (sum::mean(&xs), sum::mean(&ys)) {
        (Ok(x), Ok(y)) => Some((x, y)),
        _ => None,
    }
}

#[allow(clippy::cast_precision_loss)]
fn linear_baseline_for_col(arch: &TableArchetype, col: usize, row_count: usize, first_k: usize, last_k: usize) -> Vec<Option<f64>> {
    let Some((x_start, y_start)) = window_centroid(arch, col, 0..first_k) else {
        return vec![None; row_count];
    };
    let Some((x_end, y_end)) = window_centroid(arch, col, (row_count - last_k)..row_count) else {
        return vec![None; row_count];
    };
    let span = x_end - x_start;
    (0..row_count)
        .map(|r| {
            let val = if span == 0.0 {
                0.5 * y_start + 0.5 * y_end
            } else {
                let t = ((r as f64) - x_start) / span;
                if t == 0.0 {
                    y_start
                } else if t == 1.0 {
                    y_end
                } else {
                    let convex = (1.0 - t) * y_start + t * y_end;
                    if convex.is_finite() { convex } else { y_start + t * (y_end - y_start) }
                }
            };
            val.is_finite().then_some(val)
        })
        .collect()
}

fn apply_baseline_to_column(
    arch: &mut TableArchetype,
    col: usize,
    baselines: &[Option<f64>],
    operation: BaselineOperation,
    warnings: &mut Vec<String>,
) -> Vec<usize> {
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
            Vec::new()
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
            Vec::new()
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
) -> Vec<usize> {
    let Some(ds) = data_sets.get_mut(col) else {
        return Vec::new();
    };
    let mut swapped_rows = Vec::new();
    for (r, row_cells) in ds.cells.iter_mut().enumerate() {
        let b_opt = baselines.get(r).copied().flatten();
        match mode {
            SubcolumnMode::Single | SubcolumnMode::RawReplicates(_) => {
                for cell in row_cells {
                    apply_baseline_cell(cell, b_opt, operation, r, col, warnings);
                }
            }
            SubcolumnMode::UpperLowerLimits | SubcolumnMode::MedianIqr => {
                for cell in row_cells.iter_mut() {
                    apply_baseline_cell(cell, b_opt, operation, r, col, warnings);
                }
                let reverses = b_opt.is_some_and(|b| b < 0.0)
                    && matches!(operation, BaselineOperation::Ratio | BaselineOperation::PercentChange | BaselineOperation::FractionalChange);
                let should_swap = match (row_cells.get(1).and_then(|c| c.value.finite_f64()), row_cells.get(2).and_then(|c| c.value.finite_f64())) {
                    (Some(lo), Some(hi)) => lo > hi || (lo == hi && reverses),
                    _ => reverses,
                };
                if should_swap && swap_error_bound_cells(row_cells) {
                    swapped_rows.push(r);
                }
            }
            SubcolumnMode::MeanSdN | SubcolumnMode::MeanSemN => {
                if let Some(mean_cell) = row_cells.first_mut() {
                    apply_baseline_cell(mean_cell, b_opt, operation, r, col, warnings);
                }
                if let Some(spread_cell) = row_cells.get_mut(1) {
                    apply_baseline_spread_cell(spread_cell, b_opt, operation, r, col, warnings);
                }
            }
            SubcolumnMode::MeanCvN => {
                apply_baseline_mean_cv_row(row_cells, b_opt, operation, r, col, warnings);
            }
            SubcolumnMode::MeanLowHigh => {
                if let Some(mean_cell) = row_cells.first_mut() {
                    apply_baseline_cell(mean_cell, b_opt, operation, r, col, warnings);
                }
                for idx in 1..3 {
                    if let Some(spread_cell) = row_cells.get_mut(idx) {
                        apply_baseline_spread_cell(spread_cell, b_opt, operation, r, col, warnings);
                    }
                }
                if b_opt.is_some_and(|b| b < 0.0)
                    && matches!(operation, BaselineOperation::Ratio | BaselineOperation::PercentChange | BaselineOperation::FractionalChange)
                    && swap_error_bound_cells(row_cells)
                {
                    swapped_rows.push(r);
                }
            }
        }
    }
    swapped_rows
}

fn apply_baseline_mean_cv_row(
    row_cells: &mut [Cell],
    b_opt: Option<f64>,
    operation: BaselineOperation,
    row: usize,
    col: usize,
    warnings: &mut Vec<String>,
) {
    let m_opt = row_cells.first().and_then(|c| c.value.finite_f64());
    let Some(b) = b_opt else {
        if let Some(mean_cell) = row_cells.first_mut() {
            apply_baseline_cell(mean_cell, None, operation, row, col, warnings);
        }
        if let Some(cv_cell) = row_cells.get_mut(1) {
            apply_baseline_spread_cell(cv_cell, None, operation, row, col, warnings);
        }
        return;
    };

    let m_out = if let Some(mean_cell) = row_cells.first_mut() {
        apply_baseline_cell(mean_cell, Some(b), operation, row, col, warnings);
        mean_cell.value.finite_f64()
    } else {
        None
    };

    if let Some(cv_cell) = row_cells.get_mut(1)
        && let Some(cv) = cv_cell.value.finite_f64()
    {
        let cv_new = match (m_opt, m_out, baseline_spread_scale(b, operation)) {
            (Some(m), Some(mo), Some(sc)) if m != 0.0 && mo != 0.0 => {
                let sd_in = (cv / 100.0) * m.abs();
                let sd_out = sd_in * sc;
                let cv_val = if sd_out.is_finite() { (sd_out / mo.abs()) * 100.0 } else { ((sd_in / mo.abs()) * sc) * 100.0 };
                cv_val.is_finite().then_some(cv_val)
            }
            _ => None,
        };
        if let Some(v) = cv_new {
            cv_cell.value = CellValue::Number(v);
        } else {
            cv_cell.value = CellValue::Empty;
            if !cv_cell.excluded {
                if m_opt == Some(0.0) {
                    warnings.push(format!("row {row} col {col}: input mean is zero; baseline %CV is undefined and blanked"));
                } else {
                    warnings.push(format!("row {row} col {col}: baseline %CV undefined for baseline={b}"));
                }
            }
        }
    }
}

fn apply_baseline_transform<F>(cell: &mut Cell, b_opt: Option<f64>, row: usize, col: usize, label: &str, warnings: &mut Vec<String>, op: F)
where
    F: FnOnce(f64, f64) -> Option<f64>,
{
    let Some(v) = cell.value.finite_f64() else {
        return;
    };
    let Some(b) = b_opt else {
        cell.value = CellValue::Empty;
        if !cell.excluded {
            warnings.push(format!("row {row} col {col}: missing baseline; {label} blanked"));
        }
        return;
    };
    if let Some(out) = op(v, b).filter(|x| x.is_finite()) {
        cell.value = CellValue::Number(out);
    } else {
        cell.value = CellValue::Empty;
        if !cell.excluded {
            warnings.push(format!("row {row} col {col}: baseline {label} undefined for baseline={b}"));
        }
    }
}

fn apply_baseline_cell(cell: &mut Cell, b_opt: Option<f64>, operation: BaselineOperation, row: usize, col: usize, warnings: &mut Vec<String>) {
    apply_baseline_transform(cell, b_opt, row, col, "cell", warnings, |v, b| apply_baseline_op(v, b, operation));
}

fn apply_baseline_spread_cell(cell: &mut Cell, b_opt: Option<f64>, operation: BaselineOperation, row: usize, col: usize, warnings: &mut Vec<String>) {
    apply_baseline_transform(cell, b_opt, row, col, "spread", warnings, |s, b| baseline_spread_scale(b, operation).map(|sc| s * sc));
}

fn apply_baseline_op(v: f64, b: f64, op: BaselineOperation) -> Option<f64> {
    if !v.is_finite() || !b.is_finite() {
        return None;
    }
    let out = match op {
        BaselineOperation::Difference => v - b,
        BaselineOperation::Ratio => (b != 0.0).then(|| v / b)?,
        BaselineOperation::PercentChange => {
            if b == 0.0 {
                return None;
            }
            let diff = v - b;
            let frac = if diff.is_finite() { diff / b } else { v / b - 1.0 };
            frac * 100.0
        }
        BaselineOperation::FractionalChange => {
            if b == 0.0 {
                return None;
            }
            let diff = v - b;
            if diff.is_finite() { diff / b } else { v / b - 1.0 }
        }
    };
    out.is_finite().then_some(out)
}

fn baseline_spread_scale(b: f64, op: BaselineOperation) -> Option<f64> {
    if !b.is_finite() {
        return None;
    }
    let sc = match op {
        BaselineOperation::Difference => 1.0,
        BaselineOperation::Ratio | BaselineOperation::FractionalChange => (b != 0.0).then(|| 1.0 / b.abs())?,
        BaselineOperation::PercentChange => (b != 0.0).then(|| 100.0 / b.abs())?,
    };
    sc.is_finite().then_some(sc)
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        BaselineOperation, Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, DataSheet, DocError, MAX_TITLE_BYTES,
        MultipleVariablesTable, NormalizeAnchor, PartsOfWholeTable, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableRole,
        VariableSource, XyTable,
    };

    use super::{BaselineKind, execute_normalize, execute_remove_baseline_kind};
    use crate::error::TransformError;
    use crate::formula::evaluate_calculated_variables;
    use crate::ops::TransformContext;

    #[test]
    fn aud04_08_normalize_mean_cv_row_normalizes_mean_when_cv_empty() {
        let mut xy = XyTable::new(SubcolumnMode::MeanCvN, 1, 3).unwrap();
        for (r, row) in [
            vec![Cell::number(10.0), Cell::number(5.0), Cell::number(3.0)],
            vec![Cell::number(50.0), Cell::empty(), Cell::number(1.0)],
            vec![Cell::number(90.0), Cell::number(10.0), Cell::number(3.0)],
        ]
        .into_iter()
        .enumerate()
        {
            xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
            xy.data_sets[0].cells[r] = row;
        }
        let sheet = DataSheet::new("cv-empty", "CV Empty", TableArchetype::Xy(xy)).unwrap();
        let out =
            execute_normalize(&sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, true, &TransformContext::default()).unwrap();
        let TableArchetype::Xy(out_xy) = &out.data_sheet.archetype else {
            panic!("expected Xy");
        };
        assert!((out_xy.data_sets[0].cells[0][0].active_number().unwrap() - 0.0).abs() < 1e-12);
        assert!((out_xy.data_sets[0].cells[1][0].active_number().unwrap() - 0.5).abs() < 1e-12);
        assert_eq!(out_xy.data_sets[0].cells[1][1].value, CellValue::Empty);
        assert!((out_xy.data_sets[0].cells[2][0].active_number().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn aud04_09_normalize_and_baseline_reject_survival_and_negative_parts_of_whole() {
        let mut srv = SurvivalTable::new(1, 2).unwrap();
        srv.times = vec![Cell::number(5.0), Cell::number(10.0)];
        srv.cohorts[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(0.0)]];
        let srv_sheet = DataSheet::new("srv", "Survival", TableArchetype::Survival(srv)).unwrap();
        let ctx = TransformContext::default();

        assert!(matches!(
            execute_normalize(&srv_sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, false, true, &ctx),
            Err(TransformError::IncompatibleTable { operation: "Normalize", kind: TableKind::Survival })
        ));
        assert!(matches!(
            execute_remove_baseline_kind(&srv_sheet, &BaselineKind::Constant(1.0), BaselineOperation::Difference, &ctx),
            Err(TransformError::IncompatibleTable { operation: "RemoveBaseline", kind: TableKind::Survival })
        ));

        let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
        pow.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(30.0)]];
        let pow_sheet = DataSheet::new("pow", "Slices", TableArchetype::PartsOfWhole(pow)).unwrap();

        assert!(matches!(
            execute_normalize(&pow_sheet, &NormalizeAnchor::FixedValue(20.0), &NormalizeAnchor::FixedValue(30.0), true, true, &ctx),
            Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::PartsOfWhole, row: 0, .. }))
        ));
        assert!(matches!(
            execute_remove_baseline_kind(&pow_sheet, &BaselineKind::Constant(20.0), BaselineOperation::Difference, &ctx),
            Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::PartsOfWhole, row: 0, .. }))
        ));
    }

    #[test]
    fn aud04_10_normalize_and_baseline_mean_cv_zero_input_mean_blanks_cv_with_warning() {
        let mut xy = XyTable::new(SubcolumnMode::MeanCvN, 1, 1).unwrap();
        xy.set_x_cell(0, Cell::number(1.0)).unwrap();
        xy.data_sets[0].cells[0] = vec![Cell::number(0.0), Cell::number(25.0), Cell::number(5.0)];
        let sheet = DataSheet::new("cv-z", "CV Zero", TableArchetype::Xy(xy)).unwrap();
        let ctx = TransformContext::default();

        let norm = execute_normalize(&sheet, &NormalizeAnchor::FixedValue(-10.0), &NormalizeAnchor::FixedValue(10.0), true, true, &ctx).unwrap();
        let TableArchetype::Xy(n_xy) = &norm.data_sheet.archetype else {
            panic!("expected Xy");
        };
        assert!((n_xy.data_sets[0].cells[0][0].active_number().unwrap() - 50.0).abs() < 1e-12);
        assert_eq!(n_xy.data_sets[0].cells[0][1].value, CellValue::Empty);
        assert!(norm.warnings.iter().any(|w| w.contains("input mean is zero")), "{:?}", norm.warnings);

        let base = execute_remove_baseline_kind(&sheet, &BaselineKind::Constant(-5.0), BaselineOperation::Difference, &ctx).unwrap();
        let TableArchetype::Xy(b_xy) = &base.data_sheet.archetype else {
            panic!("expected Xy");
        };
        assert!((b_xy.data_sets[0].cells[0][0].active_number().unwrap() - 5.0).abs() < 1e-12);
        assert_eq!(b_xy.data_sets[0].cells[0][1].value, CellValue::Empty);
        assert!(base.warnings.iter().any(|w| w.contains("input mean is zero")), "{:?}", base.warnings);
    }

    #[test]
    fn aud04_14_normalize_and_baseline_snapshot_all_calculated_mv_variables() {
        let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
        mv.variables[0].title = String::from("X");
        mv.variables[1].title = String::from("Double");
        mv.variables[0].cells = vec![Cell::number(10.0), Cell::number(20.0)];
        mv.variables[1].set_calculated_formula("=X * 2");
        evaluate_calculated_variables(&mut mv, None).unwrap();
        let sheet = DataSheet::new("mv-nb", "MV NB", TableArchetype::MultipleVariables(mv)).unwrap();
        let ctx = TransformContext { selected_data_sets: vec![0], ..TransformContext::default() };

        let norm = execute_normalize(&sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, true, true, &ctx).unwrap();
        let TableArchetype::MultipleVariables(n_mv) = &norm.data_sheet.archetype else {
            panic!("expected MV");
        };
        assert_eq!(n_mv.variables[1].source, VariableSource::Entered);
        assert_eq!(n_mv.variables[1].role, VariableRole::Continuous);

        let base = execute_remove_baseline_kind(&sheet, &BaselineKind::Constant(5.0), BaselineOperation::Difference, &ctx).unwrap();
        let TableArchetype::MultipleVariables(b_mv) = &base.data_sheet.archetype else {
            panic!("expected MV");
        };
        assert_eq!(b_mv.variables[1].source, VariableSource::Entered);
        assert_eq!(b_mv.variables[1].role, VariableRole::Continuous);
    }

    #[test]
    fn aud04_31_linear_baseline_overflow_safe_and_non_finite_baseline_guards() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
        col.data_sets[0].cells = vec![vec![Cell::number(-1e308)], vec![Cell::number(5e307)], vec![Cell::number(1e308)]];
        let sheet = DataSheet::new("lin-big", "Big", TableArchetype::Column(col)).unwrap();
        let out = execute_remove_baseline_kind(
            &sheet,
            &BaselineKind::FirstAndLastLinear { first_k: 1, last_k: 1 },
            BaselineOperation::Ratio,
            &TransformContext::default(),
        )
        .unwrap();
        let TableArchetype::Column(out_col) = &out.data_sheet.archetype else {
            panic!("expected Column");
        };
        // Row 0: -1e308 / -1e308 = 1.0; Row 1: baseline = 0 -> blanked; Row 2: 1e308 / 1e308 = 1.0 (NOT 0.0!).
        assert!((out_col.data_sets[0].cells[0][0].active_number().unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(out_col.data_sets[0].cells[1][0].value, CellValue::Empty);
        assert!((out_col.data_sets[0].cells[2][0].active_number().unwrap() - 1.0).abs() < 1e-12);

        // Single-row large value [1e308] uses 0.5*y_start + 0.5*y_end without overflowing to Inf.
        let mut one = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
        one.data_sets[0].cells = vec![vec![Cell::number(1e308)]];
        let one_sheet = DataSheet::new("lin-one", "One", TableArchetype::Column(one)).unwrap();
        let one_out = execute_remove_baseline_kind(
            &one_sheet,
            &BaselineKind::FirstAndLastLinear { first_k: 1, last_k: 1 },
            BaselineOperation::Ratio,
            &TransformContext::default(),
        )
        .unwrap();
        let TableArchetype::Column(one_col) = &one_out.data_sheet.archetype else {
            panic!("expected Column");
        };
        assert!((one_col.data_sets[0].cells[0][0].active_number().unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn aud04_36_normalize_first_and_last_row_anchors_scan_non_empty_rows() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6).unwrap();
        // Col 0: blank row 0, active rows 1..=3 (10, 20, 30), blank rows 4..=5.
        col.data_sets[0].cells = vec![
            vec![Cell::empty()],
            vec![Cell::number(10.0)],
            vec![Cell::number(20.0)],
            vec![Cell::number(30.0)],
            vec![Cell::empty()],
            vec![Cell::empty()],
        ];
        // Col 1: excluded row 0, active rows 1..=2 (100, 200), blank rows 3..=5.
        col.data_sets[1].cells = vec![
            vec![Cell::excluded_number(999.0)],
            vec![Cell::number(100.0)],
            vec![Cell::number(200.0)],
            vec![Cell::empty()],
            vec![Cell::empty()],
            vec![Cell::empty()],
        ];
        let sheet = DataSheet::new("first-last", "FirstLast", TableArchetype::Column(col)).unwrap();
        let out = execute_normalize(&sheet, &NormalizeAnchor::FirstRow, &NormalizeAnchor::LastRow, true, true, &TransformContext::default()).unwrap();
        let TableArchetype::Column(out_col) = &out.data_sheet.archetype else {
            panic!("expected Column");
        };
        assert!((out_col.data_sets[0].cells[1][0].active_number().unwrap() - 0.0).abs() < 1e-12);
        assert!((out_col.data_sets[0].cells[2][0].active_number().unwrap() - 50.0).abs() < 1e-12);
        assert!((out_col.data_sets[0].cells[3][0].active_number().unwrap() - 100.0).abs() < 1e-12);
        assert!((out_col.data_sets[1].cells[1][0].active_number().unwrap() - 0.0).abs() < 1e-12);
        assert!((out_col.data_sets[1].cells[2][0].active_number().unwrap() - 100.0).abs() < 1e-12);
    }

    #[test]
    fn aud04_38_normalize_and_baseline_batched_swapped_notes() {
        let mut xy = XyTable::new(SubcolumnMode::MeanLowHigh, 2, 2).unwrap();
        for r in 0..2 {
            xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
            for c in 0..2 {
                xy.data_sets[c].cells[r] = vec![Cell::number(10.0), Cell::excluded_number(1.0), Cell::number(2.0)];
            }
        }
        let mut sheet = DataSheet::new("nb-swap", "Swap", TableArchetype::Xy(xy)).unwrap();
        sheet.exclusion_notes.push(CellExclusionNote {
            coord: CellCoord::new(0, 0, 1),
            reason_tag: String::from("qc"),
            note: String::from("c0"),
            timestamp_utc: String::new(),
        });
        sheet.exclusion_notes.push(CellExclusionNote {
            coord: CellCoord::new(1, 1, 1),
            reason_tag: String::from("qc"),
            note: String::from("c1"),
            timestamp_utc: String::new(),
        });
        let norm = execute_normalize(
            &sheet,
            &NormalizeAnchor::FixedValue(20.0),
            &NormalizeAnchor::FixedValue(0.0),
            true,
            false,
            &TransformContext::default(),
        )
        .unwrap();
        assert_eq!(norm.data_sheet.exclusion_notes[0].coord, CellCoord::new(0, 0, 2));
        assert_eq!(norm.data_sheet.exclusion_notes[1].coord, CellCoord::new(1, 1, 2));
    }

    #[test]
    fn aud04_45_normalize_and_baseline_validate_output_sheet_and_bound_long_titles() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        col.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        let sheet = DataSheet::new("nb-long", "T".repeat(MAX_TITLE_BYTES), TableArchetype::Column(col)).unwrap();
        let norm =
            execute_normalize(&sheet, &NormalizeAnchor::ColumnMin, &NormalizeAnchor::ColumnMax, true, true, &TransformContext::default()).unwrap();
        assert!(norm.data_sheet.validate().is_ok());
        assert!(norm.data_sheet.title.len() <= MAX_TITLE_BYTES);

        let bad_ctx = TransformContext { output_id: Some(String::from("bad\0id")), ..TransformContext::default() };
        assert!(matches!(
            execute_remove_baseline_kind(&sheet, &BaselineKind::Constant(1.0), BaselineOperation::Difference, &bad_ctx),
            Err(TransformError::Doc(DocError::InvalidIdentifier { .. }))
        ));
    }

    #[test]
    fn aud04_48_baseline_percent_change_and_cv_avoid_premature_overflow() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
        col.data_sets[0].cells = vec![vec![Cell::number(1e308)]];
        let sheet = DataSheet::new("pct-big", "PctBig", TableArchetype::Column(col)).unwrap();
        let out =
            execute_remove_baseline_kind(&sheet, &BaselineKind::Constant(1e307), BaselineOperation::PercentChange, &TransformContext::default())
                .unwrap();
        let TableArchetype::Column(out_col) = &out.data_sheet.archetype else {
            panic!("expected Column");
        };
        assert!((out_col.data_sets[0].cells[0][0].active_number().unwrap() - 900.0).abs() < 1e-10);

        let mut cv = XyTable::new(SubcolumnMode::MeanCvN, 1, 1).unwrap();
        cv.set_x_cell(0, Cell::number(1.0)).unwrap();
        cv.data_sets[0].cells[0] = vec![Cell::number(1e308), Cell::number(5.0), Cell::number(4.0)];
        let cv_sheet = DataSheet::new("cv-norm-big", "CVNormBig", TableArchetype::Xy(cv)).unwrap();
        let norm = execute_normalize(
            &cv_sheet,
            &NormalizeAnchor::FixedValue(0.0),
            &NormalizeAnchor::FixedValue(1e308),
            false,
            true,
            &TransformContext::default(),
        )
        .unwrap();
        let TableArchetype::Xy(norm_xy) = &norm.data_sheet.archetype else {
            panic!("expected Xy");
        };
        assert!((norm_xy.data_sets[0].cells[0][1].active_number().unwrap() - 5.0).abs() < 1e-12);
    }
}
