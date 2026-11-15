//! Table transposition, row pruning/decimation, and subset/archetype projection
//! (`TransformAnalysisSpec::Transpose`, `PruneRows`, `SelectAndTransform`,
//! `docs/data-tables-and-transforms.md` §5.4).

use lumencraft_doc::{
    Cell, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, GroupedTable, MultipleVariablesTable, NestedTable,
    PartsOfWholeTable, ResultCell, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, VariableColumn, VariableRole, VariableSource,
    XColumnMode, XyTable, default_column_label, transpose_cell_grid,
};
use lumencraft_math::sum;

use crate::error::TransformError;
use crate::formula::{TableEvalContext, eval_column_expr, parse_formula};
use crate::ops::{TransformContext, TransformOutput, build_summary_tab};

/// Decimation strategy for [`execute_prune`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PruneMethod {
    EveryNth(usize),
    LogarithmicSpacing { target_points: usize },
    AverageBins(usize),
    MinMaxBins(usize),
}

/// Subset selection rule for [`execute_select`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SelectRule {
    /// Keep rows in inclusive range `start..=end`.
    RowRange(usize, usize),
    /// Keep rows where formula evaluates to truthy (`!= 0.0`, non-missing).
    Predicate(String),
    /// Keep rows that have no missing/excluded active cells across selected columns.
    ExcludeMissing,
    /// Keep the `n` rows with largest active numeric values in `column`.
    TopN { column: usize, n: usize },
    /// Keep the `n` rows with smallest active numeric values in `column`.
    BottomN { column: usize, n: usize },
}

/// Transpose rows and data-set columns using [`lumencraft_doc::transpose_cell_grid`].
pub fn execute_transpose(source: &DataSheet, swap_titles: bool, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    let (id, title) = ctx.derived_ids(source, "transpose", "Transposed");
    let archetype = transpose_archetype(&source.archetype, swap_titles)?;
    let out = DataSheet::new(id, title, archetype)?;
    let summary_tab = build_summary_tab(
        "Transpose summary",
        vec![
            ("Output rows", ResultCell::Number { value: out.archetype.row_count() as f64, prefix: None, units: None }),
            ("Output columns", ResultCell::Number { value: out.archetype.column_count() as f64, prefix: None, units: None }),
            ("Swapped titles", ResultCell::Boolean(swap_titles)),
        ],
        Vec::new(),
    );
    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
}

fn transpose_archetype(arch: &TableArchetype, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    match arch {
        TableArchetype::Column(t) => {
            if t.subcolumn_mode != SubcolumnMode::Single {
                return Err(TransformError::IncompatibleTable { operation: "Transpose on multi-subcolumn Column table", kind: TableKind::Column });
            }
            let tc = transpose_single_columns(&t.data_sets, &t.row_titles, swap_titles)?;
            let mut out = ColumnTable::new(t.design_hint, tc.data_sets.len().max(1), tc.data_sets.first().map_or(0, DataSetColumn::row_count))?;
            if tc.data_sets.is_empty() {
                out.data_sets.clear();
            } else {
                out.data_sets = tc.data_sets;
                for (i, ds) in out.data_sets.iter_mut().enumerate() {
                    if let Some(ct) = tc.col_titles.get(i) {
                        ds.title = ct.clone();
                    }
                }
            }
            out.row_titles = tc.row_titles;
            out.row_titles_expanded = !out.row_titles.iter().all(String::is_empty);
            Ok(TableArchetype::Column(out))
        }
        TableArchetype::Contingency(t) => {
            let tc = transpose_single_columns(&t.data_sets, &t.row_titles, swap_titles)?;
            let mut out = ContingencyTable::new(tc.data_sets.len().max(1), tc.data_sets.first().map_or(0, DataSetColumn::row_count))?;
            if tc.data_sets.is_empty() {
                out.data_sets.clear();
            } else {
                out.data_sets = tc.data_sets;
                for (i, ds) in out.data_sets.iter_mut().enumerate() {
                    if let Some(ct) = tc.col_titles.get(i) {
                        ds.title = ct.clone();
                    }
                }
            }
            out.row_titles = tc.row_titles;
            out.row_factor_title = t.column_factor_title.clone();
            out.column_factor_title = t.row_factor_title.clone();
            Ok(TableArchetype::Contingency(out))
        }
        TableArchetype::PartsOfWhole(t) => {
            let tc = transpose_single_columns(&t.data_sets, &t.slice_labels, swap_titles)?;
            let mut out = PartsOfWholeTable::new(tc.data_sets.len().max(1), tc.data_sets.first().map_or(0, DataSetColumn::row_count))?;
            if tc.data_sets.is_empty() {
                out.data_sets.clear();
            } else {
                out.data_sets = tc.data_sets;
                for (i, ds) in out.data_sets.iter_mut().enumerate() {
                    if let Some(ct) = tc.col_titles.get(i) {
                        ds.title = ct.clone();
                    }
                }
            }
            out.slice_labels = tc.row_titles;
            Ok(TableArchetype::PartsOfWhole(out))
        }
        TableArchetype::MultipleVariables(mv) => transpose_multivariate(mv, swap_titles),
        TableArchetype::Grouped(t) => transpose_grouped(t, swap_titles),
        TableArchetype::Xy(t) => transpose_xy(t, swap_titles),
        TableArchetype::Survival(_) | TableArchetype::Nested(_) => {
            Err(TransformError::IncompatibleTable { operation: "Transpose", kind: arch.kind() })
        }
    }
}

struct TransposedColumns {
    data_sets: Vec<DataSetColumn>,
    col_titles: Vec<String>,
    row_titles: Vec<String>,
}

fn transpose_single_columns(data_sets: &[DataSetColumn], row_titles: &[String], swap_titles: bool) -> Result<TransposedColumns, TransformError> {
    let rows = data_sets.first().map_or(row_titles.len(), DataSetColumn::row_count);
    let cols = data_sets.len();
    let mut grid = vec![vec![Cell::empty(); cols]; rows];
    for (c, ds) in data_sets.iter().enumerate() {
        for (r, row_cells) in ds.cells.iter().enumerate() {
            if let Some(cell) = row_cells.first()
                && let Some(slot) = grid.get_mut(r).and_then(|gr| gr.get_mut(c))
            {
                *slot = cell.clone();
            }
        }
    }
    let transposed = transpose_cell_grid(&grid);
    let new_rows = cols;
    let new_cols = rows;

    let new_col_titles: Vec<String> = if swap_titles {
        (0..new_cols).map(|c| row_titles.get(c).cloned().unwrap_or_else(|| default_column_label(c))).collect()
    } else {
        (0..new_cols).map(default_column_label).collect()
    };
    let new_row_titles: Vec<String> = if swap_titles {
        (0..new_rows).map(|r| data_sets.get(r).map_or_else(String::new, |ds| ds.title.clone())).collect()
    } else {
        vec![String::new(); new_rows]
    };

    let mut out_cols = Vec::with_capacity(new_cols);
    for c in 0..new_cols {
        let title = new_col_titles.get(c).cloned().unwrap_or_else(|| default_column_label(c));
        let mut ds = DataSetColumn::new(title, new_rows, 1)?;
        for r in 0..new_rows {
            if let Some(cell) = transposed.get(r).and_then(|tr| tr.get(c)) {
                ds.cells[r][0] = cell.clone();
            }
        }
        out_cols.push(ds);
    }

    Ok(TransposedColumns { data_sets: out_cols, col_titles: new_col_titles, row_titles: new_row_titles })
}

fn transpose_multivariate(mv: &MultipleVariablesTable, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    let rows = mv.row_count();
    let cols = mv.variables.len();
    let mut grid = vec![vec![Cell::empty(); cols]; rows];
    for (c, var) in mv.variables.iter().enumerate() {
        for (r, cell) in var.cells.iter().enumerate() {
            grid[r][c] = cell.clone();
        }
    }
    let transposed = transpose_cell_grid(&grid);
    let new_rows = cols;
    let new_cols = rows;
    let mut out = MultipleVariablesTable::new(new_cols.max(1), new_rows)?;
    if new_cols == 0 {
        out.variables.clear();
    } else {
        out.variables.clear();
        for c in 0..new_cols {
            let title = if swap_titles {
                mv.row_titles.get(c).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Variable {}", default_column_label(c)))
            } else {
                format!("Variable {}", default_column_label(c))
            };
            let mut var = VariableColumn::new(title, new_rows)?;
            for r in 0..new_rows {
                var.cells[r] = transposed.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
            }
            var.refresh_inferred_role();
            out.variables.push(var);
        }
    }
    out.row_titles = if swap_titles { mv.variables.iter().map(|v| v.title.clone()).collect() } else { vec![String::new(); new_rows] };
    out.row_titles_expanded = !out.row_titles.iter().all(String::is_empty);
    Ok(TableArchetype::MultipleVariables(out))
}

fn transpose_grouped(t: &GroupedTable, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    let rows = t.row_count();
    let cols = t.data_sets.len();
    let subs = t.subcolumn_mode.subcolumn_count();

    let mut sub_grids = Vec::with_capacity(subs);
    for s in 0..subs {
        let mut grid = vec![vec![Cell::empty(); cols]; rows];
        for (c, ds) in t.data_sets.iter().enumerate() {
            for (r, row_cells) in ds.cells.iter().enumerate() {
                if let Some(cell) = row_cells.get(s) {
                    grid[r][c] = cell.clone();
                }
            }
        }
        sub_grids.push(transpose_cell_grid(&grid));
    }

    let new_rows = cols;
    let new_cols = rows;
    let mut out = GroupedTable::new(t.subcolumn_mode, new_cols.max(1), new_rows)?;
    if new_cols == 0 {
        out.data_sets.clear();
    } else {
        for c in 0..new_cols {
            if swap_titles {
                out.data_sets[c].title = t.row_titles.get(c).cloned().unwrap_or_else(|| default_column_label(c));
            }
            for r in 0..new_rows {
                for (s, sub_grid) in sub_grids.iter().enumerate().take(subs) {
                    out.data_sets[c].cells[r][s] = sub_grid.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
                }
            }
        }
    }
    out.row_titles = if swap_titles { t.data_sets.iter().map(|ds| ds.title.clone()).collect() } else { vec![String::new(); new_rows] };
    out.row_factor_title = t.column_factor_title.clone();
    out.column_factor_title = t.row_factor_title.clone();
    Ok(TableArchetype::Grouped(out))
}

fn transpose_xy(t: &XyTable, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    let row_labels: Vec<String> = (0..t.row_count())
        .map(|r| {
            let rt = t.row_titles.get(r).map(String::as_str).unwrap_or("");
            if !rt.is_empty() {
                rt.to_owned()
            } else if let Some(x) = t.x_value_at(r) {
                format!("{x}")
            } else {
                default_column_label(r)
            }
        })
        .collect();
    let tc = transpose_single_columns(&t.data_sets, &row_labels, swap_titles)?;
    let mut out =
        ColumnTable::new(ColumnDesignHint::UnpairedReplicates, tc.data_sets.len().max(1), tc.data_sets.first().map_or(0, DataSetColumn::row_count))?;
    if tc.data_sets.is_empty() {
        out.data_sets.clear();
    } else {
        out.data_sets = tc.data_sets;
        for (i, ds) in out.data_sets.iter_mut().enumerate() {
            if let Some(ct) = tc.col_titles.get(i) {
                ds.title = ct.clone();
            }
        }
    }
    out.row_titles = tc.row_titles;
    out.row_titles_expanded = !out.row_titles.iter().all(String::is_empty);
    Ok(TableArchetype::Column(out))
}

/// Decimate or bin rows of a [`DataSheet`] according to [`PruneMethod`].
pub fn execute_prune(source: &DataSheet, method: PruneMethod, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    let row_count = source.archetype.row_count();
    let (id, title) = ctx.derived_ids(source, "prune", "Pruned");

    let mut out = source.clone();
    out.id = SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();

    let kept_rows = match method {
        PruneMethod::EveryNth(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("EveryNth step k must be >= 1") });
            }
            let idxs: Vec<usize> = (0..row_count).step_by(k).collect();
            retain_rows(&mut out.archetype, &idxs);
            idxs.len()
        }
        PruneMethod::LogarithmicSpacing { target_points } => {
            if target_points == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("LogarithmicSpacing target_points must be >= 1") });
            }
            let idxs = select_log_spaced_indices(&source.archetype, target_points);
            retain_rows(&mut out.archetype, &idxs);
            idxs.len()
        }
        PruneMethod::AverageBins(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("AverageBins width k must be >= 1") });
            }
            average_bins_archetype(&mut out.archetype, k)
        }
        PruneMethod::MinMaxBins(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("MinMaxBins width k must be >= 1") });
            }
            let idxs = select_minmax_bin_indices(&source.archetype, k);
            retain_rows(&mut out.archetype, &idxs);
            idxs.len()
        }
    };

    let summary_tab = build_summary_tab(
        "Prune summary",
        vec![
            ("Original rows", ResultCell::Number { value: row_count as f64, prefix: None, units: None }),
            ("Kept rows", ResultCell::Number { value: kept_rows as f64, prefix: None, units: None }),
        ],
        Vec::new(),
    );

    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
fn select_log_spaced_indices(arch: &TableArchetype, target_points: usize) -> Vec<usize> {
    let n = arch.row_count();
    if n <= target_points || n <= 1 {
        return (0..n).collect();
    }
    if target_points == 1 {
        return vec![0];
    }

    let mut out = Vec::new();
    if let TableArchetype::Xy(xy) = arch {
        let xs: Vec<(usize, f64)> = (0..n).filter_map(|r| xy.x_value_at(r).filter(|&x| x > 0.0).map(|x| (r, x))).collect();
        if xs.len() >= 2 {
            let log_min = xs[0].1.ln();
            let log_max = xs[xs.len() - 1].1.ln();
            if log_max > log_min {
                for step in 0..target_points {
                    let target_ln = log_min + ((step as f64) / ((target_points - 1) as f64)) * (log_max - log_min);
                    if let Some((best_r, _)) = xs.iter().min_by(|a, b| (a.1.ln() - target_ln).abs().total_cmp(&(b.1.ln() - target_ln).abs()))
                        && out.last().copied() != Some(*best_r)
                    {
                        out.push(*best_r);
                    }
                }
                return out;
            }
        }
    }

    for step in 0..target_points {
        let frac = (step as f64) / ((target_points - 1) as f64);
        let log_pos = ((n as f64).powf(frac) - 1.0).round() as usize;
        let clamped = log_pos.min(n - 1);
        if out.last().copied() != Some(clamped) {
            out.push(clamped);
        }
    }
    out
}

fn select_minmax_bin_indices(arch: &TableArchetype, bin_width: usize) -> Vec<usize> {
    let n = arch.row_count();
    let mut out = Vec::new();
    let mut start = 0;
    while start < n {
        let end = (start + bin_width).min(n);
        let mut min_pair: Option<(usize, f64)> = None;
        let mut max_pair: Option<(usize, f64)> = None;
        for r in start..end {
            let Some(v) = primary_row_scalar(arch, r) else {
                continue;
            };
            if min_pair.is_none_or(|(_, cur)| v < cur) {
                min_pair = Some((r, v));
            }
            if max_pair.is_none_or(|(_, cur)| v > cur) {
                max_pair = Some((r, v));
            }
        }
        match (min_pair, max_pair) {
            (Some((r_min, _)), Some((r_max, _))) => {
                if r_min == r_max {
                    out.push(r_min);
                } else if r_min < r_max {
                    out.push(r_min);
                    out.push(r_max);
                } else {
                    out.push(r_max);
                    out.push(r_min);
                }
            }
            _ => out.push(start),
        }
        start = end;
    }
    out
}

fn primary_row_scalar(arch: &TableArchetype, row: usize) -> Option<f64> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.first()?.cells.get(row)?.iter().filter_map(Cell::active_number).next(),
        TableArchetype::Column(t) => t.data_sets.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Grouped(t) => t.data_sets.first()?.cells.get(row)?.iter().filter_map(Cell::active_number).next(),
        TableArchetype::Contingency(t) => t.data_sets.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Survival(t) => t.cohorts.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::MultipleVariables(mv) => mv.variables.first()?.cells.get(row)?.active_number(),
        TableArchetype::Nested(t) => t.groups.first()?.cells.get(row)?.iter().filter_map(Cell::active_number).next(),
    }
}

fn average_bins_archetype(arch: &mut TableArchetype, k: usize) -> usize {
    let n = arch.row_count();
    if n == 0 {
        return 0;
    }
    let bin_count = n.div_ceil(k);
    match arch {
        TableArchetype::Xy(t) => {
            if !matches!(t.x_mode, XColumnMode::Numbers { .. }) {
                let mat: Vec<Vec<Cell>> = (0..n).map(|r| vec![t.x_value_at(r).map_or_else(Cell::empty, Cell::number)]).collect();
                t.x_cells = mat;
                t.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
            }
            t.x_cells = average_cell_grid_bins(&t.x_cells, k);
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::Column(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::Grouped(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::Contingency(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            t.slice_labels = bin_titles(&t.slice_labels, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::Survival(t) => {
            let time_grid: Vec<Vec<Cell>> = t.times.iter().map(|c| vec![c.clone()]).collect();
            t.times = average_cell_grid_bins(&time_grid, k).into_iter().map(|r| r.into_iter().next().unwrap_or_default()).collect();
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.cohorts {
                ds.cells = average_cell_grid_bins(&ds.cells, k);
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            mv.row_titles = bin_titles(&mv.row_titles, k);
            for var in &mut mv.variables {
                let grid: Vec<Vec<Cell>> = var.cells.iter().map(|c| vec![c.clone()]).collect();
                var.cells = average_cell_grid_bins(&grid, k).into_iter().map(|r| r.into_iter().next().unwrap_or_default()).collect();
            }
        }
        TableArchetype::Nested(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for grp in &mut t.groups {
                grp.cells = average_cell_grid_bins(&grp.cells, k);
            }
        }
    }
    bin_count
}

fn bin_titles(titles: &[String], k: usize) -> Vec<String> {
    titles.chunks(k).map(|chunk| chunk.first().cloned().unwrap_or_default()).collect()
}

fn average_cell_grid_bins(rows: &[Vec<Cell>], k: usize) -> Vec<Vec<Cell>> {
    let subcols = rows.first().map_or(0, Vec::len);
    rows.chunks(k)
        .map(|chunk| {
            (0..subcols)
                .map(|s| {
                    let nums: Vec<f64> = chunk.iter().filter_map(|r| r.get(s).and_then(Cell::active_number)).collect();
                    if nums.is_empty() { Cell::empty() } else { Cell::number(sum::sum(&nums) / (nums.len() as f64)) }
                })
                .collect()
        })
        .collect()
}

pub(crate) fn retain_rows(arch: &mut TableArchetype, indices: &[usize]) {
    match arch {
        TableArchetype::Xy(t) => {
            if matches!(t.x_mode, XColumnMode::GeneratedSequence { .. }) {
                let mat: Vec<Vec<Cell>> = (0..t.row_count()).map(|r| vec![t.x_value_at(r).map_or_else(Cell::empty, Cell::number)]).collect();
                t.x_cells = mat;
                t.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
            }
            t.x_cells = indices.iter().filter_map(|&r| t.x_cells.get(r).cloned()).collect();
            t.row_titles = retain_strings(&t.row_titles, indices);
            for ds in &mut t.data_sets {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::Column(t) => {
            t.row_titles = retain_strings(&t.row_titles, indices);
            for ds in &mut t.data_sets {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::Grouped(t) => {
            t.row_titles = retain_strings(&t.row_titles, indices);
            if let Some(spacing) = &mut t.row_numeric_spacing {
                *spacing = indices.iter().map(|&r| spacing.get(r).copied().flatten()).collect();
            }
            for ds in &mut t.data_sets {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::Contingency(t) => {
            t.row_titles = retain_strings(&t.row_titles, indices);
            for ds in &mut t.data_sets {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::Survival(t) => {
            t.times = indices.iter().filter_map(|&r| t.times.get(r).cloned()).collect();
            t.row_titles = retain_strings(&t.row_titles, indices);
            for ds in &mut t.cohorts {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::PartsOfWhole(t) => {
            t.slice_labels = retain_strings(&t.slice_labels, indices);
            for ds in &mut t.data_sets {
                ds.cells = indices.iter().filter_map(|&r| ds.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::MultipleVariables(mv) => {
            mv.row_titles = retain_strings(&mv.row_titles, indices);
            for var in &mut mv.variables {
                var.cells = indices.iter().filter_map(|&r| var.cells.get(r).cloned()).collect();
            }
        }
        TableArchetype::Nested(t) => {
            t.row_titles = retain_strings(&t.row_titles, indices);
            for grp in &mut t.groups {
                grp.cells = indices.iter().filter_map(|&r| grp.cells.get(r).cloned()).collect();
            }
        }
    }
}

fn retain_strings(items: &[String], indices: &[usize]) -> Vec<String> {
    indices.iter().map(|&r| items.get(r).cloned().unwrap_or_default()).collect()
}

/// Filter rows according to [`SelectRule`]s and optionally project into `target_archetype`.
pub fn execute_select(
    source: &DataSheet,
    rules: &[SelectRule],
    target_archetype: TableKind,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    let row_count = source.archetype.row_count();
    let mut keep_mask = vec![true; row_count];

    for rule in rules {
        apply_select_rule(source, rule, &mut keep_mask, ctx)?;
    }

    let kept_indices: Vec<usize> = keep_mask.iter().enumerate().filter_map(|(r, &k)| k.then_some(r)).collect();
    let mut filtered = source.clone();
    retain_rows(&mut filtered.archetype, &kept_indices);

    let (id, title) = ctx.derived_ids(source, "select", "Selected");
    let selected_cols = ctx.active_columns(filtered.archetype.column_count())?;
    let projected = if filtered.kind() == target_archetype {
        if !ctx.selected_data_sets.is_empty() {
            retain_columns(&mut filtered.archetype, &selected_cols)?;
        }
        filtered.archetype
    } else {
        project_archetype(&filtered.archetype, target_archetype, &selected_cols)?
    };
    let out = DataSheet::new(id, title, projected)?;

    let summary_tab = build_summary_tab(
        "Select and transform summary",
        vec![
            ("Original rows", ResultCell::Number { value: row_count as f64, prefix: None, units: None }),
            ("Selected rows", ResultCell::Number { value: kept_indices.len() as f64, prefix: None, units: None }),
            ("Target archetype", ResultCell::Text(String::from(target_archetype.label()))),
        ],
        Vec::new(),
    );
    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings: Vec::new() })
}

fn apply_select_rule(source: &DataSheet, rule: &SelectRule, mask: &mut [bool], ctx: &TransformContext<'_>) -> Result<(), TransformError> {
    let n = mask.len();
    match rule {
        SelectRule::RowRange(start, end) => {
            if *start > *end {
                return Err(TransformError::InvalidSpec { message: format!("RowRange start {start} > end {end}") });
            }
            for (r, flag) in mask.iter_mut().enumerate() {
                if r < *start || r > *end {
                    *flag = false;
                }
            }
        }
        SelectRule::Predicate(formula) => {
            let expr = parse_formula(formula)?;
            let eval_ctx = TableEvalContext::new(source, ctx.info_sheet);
            let vals = eval_column_expr(&expr, &eval_ctx)?;
            for (r, flag) in mask.iter_mut().enumerate() {
                if vals.get(r).and_then(|v| v.as_bool()) != Some(true) {
                    *flag = false;
                }
            }
        }
        SelectRule::ExcludeMissing => {
            let cols = ctx.active_columns(source.archetype.column_count())?;
            for (r, flag) in mask.iter_mut().enumerate() {
                if !*flag {
                    continue;
                }
                let row_complete = cols.iter().all(|&c| row_col_has_active(&source.archetype, r, c));
                if !row_complete {
                    *flag = false;
                }
            }
        }
        SelectRule::TopN { column, n: top_k } | SelectRule::BottomN { column, n: top_k } => {
            let is_top = matches!(rule, SelectRule::TopN { .. });
            if *column >= source.archetype.column_count() {
                return Err(TransformError::InvalidSpec { message: format!("TopN/BottomN column {column} out of bounds") });
            }
            let mut candidates: Vec<(usize, f64)> =
                (0..n).filter(|&r| mask[r]).filter_map(|r| row_col_active_number(&source.archetype, r, *column).map(|v| (r, v))).collect();
            candidates.sort_by(|a, b| if is_top { b.1.total_cmp(&a.1) } else { a.1.total_cmp(&b.1) });
            let mut chosen = vec![false; n];
            for (r, _) in candidates.into_iter().take(*top_k) {
                chosen[r] = true;
            }
            for (r, flag) in mask.iter_mut().enumerate() {
                *flag = *flag && chosen[r];
            }
        }
    }
    Ok(())
}

fn row_col_has_active(arch: &TableArchetype, row: usize, col: usize) -> bool {
    match arch {
        TableArchetype::Xy(t) => {
            t.data_sets.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::Column(t) => {
            t.data_sets.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::Grouped(t) => {
            t.data_sets.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::Contingency(t) => {
            t.data_sets.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::Survival(t) => {
            t.cohorts.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::PartsOfWhole(t) => {
            t.data_sets.get(col).and_then(|d| d.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
        TableArchetype::MultipleVariables(mv) => mv.variables.get(col).and_then(|v| v.cells.get(row)).is_some_and(|c| !c.excluded && !c.is_empty()),
        TableArchetype::Nested(t) => {
            t.groups.get(col).and_then(|g| g.cells.get(row)).is_some_and(|subs| subs.iter().any(|c| !c.excluded && !c.is_empty()))
        }
    }
}

fn row_col_active_number(arch: &TableArchetype, row: usize, col: usize) -> Option<f64> {
    match arch {
        TableArchetype::MultipleVariables(mv) => mv.variables.get(col)?.cells.get(row)?.active_number(),
        TableArchetype::Xy(t) => t.data_sets.get(col)?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Column(t) => t.data_sets.get(col)?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Grouped(t) => {
            let active: Vec<f64> = t.data_sets.get(col)?.cells.get(row)?.iter().filter_map(Cell::active_number).collect();
            sum::mean(&active).ok()
        }
        TableArchetype::Contingency(t) => t.data_sets.get(col)?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Survival(t) => t.cohorts.get(col)?.cells.get(row)?.first()?.active_number(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col)?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Nested(t) => {
            let active: Vec<f64> = t.groups.get(col)?.cells.get(row)?.iter().filter_map(Cell::active_number).collect();
            sum::mean(&active).ok()
        }
    }
}

fn retain_col_vec<T: Clone>(items: &[T], selected: &[usize]) -> Vec<T> {
    selected.iter().filter_map(|&c| items.get(c).cloned()).collect()
}

fn retain_columns(arch: &mut TableArchetype, selected: &[usize]) -> Result<(), TransformError> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets = retain_col_vec(&t.data_sets, selected),
        TableArchetype::Column(t) => t.data_sets = retain_col_vec(&t.data_sets, selected),
        TableArchetype::Grouped(t) => t.data_sets = retain_col_vec(&t.data_sets, selected),
        TableArchetype::Contingency(t) => {
            t.data_sets = retain_col_vec(&t.data_sets, selected);
            t.validate_counts()?;
        }
        TableArchetype::Survival(t) => t.cohorts = retain_col_vec(&t.cohorts, selected),
        TableArchetype::PartsOfWhole(t) => t.data_sets = retain_col_vec(&t.data_sets, selected),
        TableArchetype::MultipleVariables(mv) => {
            let full_keep = selected.len() == mv.variables.len() && selected.iter().enumerate().all(|(i, &c)| i == c);
            mv.variables = selected
                .iter()
                .filter_map(|&c| {
                    let mut var = mv.variables.get(c)?.clone();
                    if !full_keep && matches!(var.source, VariableSource::Calculated { .. }) {
                        // Subsetting/reordering variables can drop or re-index formula inputs; snapshot evaluated cells.
                        var.source = VariableSource::Entered;
                    }
                    Some(var)
                })
                .collect();
        }
        TableArchetype::Nested(t) => t.groups = retain_col_vec(&t.groups, selected),
    }
    Ok(())
}

fn project_archetype(arch: &TableArchetype, target: TableKind, selected: &[usize]) -> Result<TableArchetype, TransformError> {
    let rows = arch.row_count();
    let (titles, row_titles, grid) = flatten_primary_columns(arch, selected);
    let cols = titles.len();

    match target {
        TableKind::Column => {
            let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols.max(1), rows)?;
            if cols == 0 {
                t.data_sets.clear();
            } else {
                populate_single_datasets(&mut t.data_sets, &titles, &grid);
            }
            t.row_titles_expanded = row_titles.iter().any(|s| !s.trim().is_empty());
            t.row_titles = row_titles;
            Ok(TableArchetype::Column(t))
        }
        TableKind::Xy => {
            let y_cols = cols.saturating_sub(1).max(1);
            let mut t = XyTable::new(SubcolumnMode::Single, y_cols, rows)?;
            if cols <= 1 {
                for r in 0..rows {
                    #[allow(clippy::cast_precision_loss)]
                    t.set_x_cell(r, Cell::number((r + 1) as f64))?;
                    if let Some(cell) = grid.get(r).and_then(|row| row.first()) {
                        t.data_sets[0].cells[r][0] = cell.clone();
                    }
                }
                if let Some(first_title) = titles.first() {
                    t.data_sets[0].title = first_title.clone();
                }
            } else {
                t.x_title = titles[0].clone();
                for (r, row_vals) in grid.iter().enumerate().take(rows) {
                    t.set_x_cell(r, row_vals[0].clone())?;
                }
                for (y_idx, ds) in t.data_sets.iter_mut().enumerate() {
                    ds.title = titles[y_idx + 1].clone();
                    for (r, row_vals) in grid.iter().enumerate().take(rows) {
                        ds.cells[r][0] = row_vals[y_idx + 1].clone();
                    }
                }
            }
            t.row_titles_expanded = row_titles.iter().any(|s| !s.trim().is_empty());
            t.row_titles = row_titles;
            Ok(TableArchetype::Xy(t))
        }
        TableKind::Grouped => {
            let mut t = GroupedTable::new(SubcolumnMode::RawReplicates(1), cols.max(1), rows)?;
            if cols == 0 {
                t.data_sets.clear();
            } else {
                populate_single_datasets(&mut t.data_sets, &titles, &grid);
            }
            t.row_titles = row_titles;
            Ok(TableArchetype::Grouped(t))
        }
        TableKind::Contingency => {
            let mut t = ContingencyTable::new(cols.max(1), rows)?;
            if cols == 0 {
                t.data_sets.clear();
            } else {
                populate_single_datasets(&mut t.data_sets, &titles, &grid);
            }
            t.row_titles = row_titles;
            t.validate_counts()?;
            Ok(TableArchetype::Contingency(t))
        }
        TableKind::PartsOfWhole => {
            let mut t = PartsOfWholeTable::new(cols.max(1), rows)?;
            if cols == 0 {
                t.data_sets.clear();
            } else {
                populate_single_datasets(&mut t.data_sets, &titles, &grid);
            }
            t.slice_labels = row_titles;
            Ok(TableArchetype::PartsOfWhole(t))
        }
        TableKind::Survival => {
            let cohort_cols = cols.saturating_sub(1).max(1);
            let mut t = SurvivalTable::new(cohort_cols, rows)?;
            if cols >= 2 {
                t.time_title = titles[0].clone();
                for (r, row_vals) in grid.iter().enumerate().take(rows) {
                    t.times[r] = row_vals[0].clone();
                    for c in 0..cohort_cols {
                        t.cohorts[c].title = titles[c + 1].clone();
                        t.cohorts[c].cells[r][0] = row_vals[c + 1].clone();
                    }
                }
            } else if cols == 1 {
                t.cohorts[0].title = titles[0].clone();
                for (r, row_vals) in grid.iter().enumerate().take(rows) {
                    t.cohorts[0].cells[r][0] = row_vals[0].clone();
                }
            }
            t.row_titles = row_titles;
            Ok(TableArchetype::Survival(t))
        }
        TableKind::MultipleVariables => {
            let mut t = MultipleVariablesTable::new(cols.max(1), rows)?;
            if cols == 0 {
                t.variables.clear();
            } else {
                for (c, var) in t.variables.iter_mut().enumerate() {
                    var.title = titles[c].clone();
                    var.role = VariableRole::Continuous;
                    for (r, row_vals) in grid.iter().enumerate().take(rows) {
                        var.cells[r] = row_vals[c].clone();
                    }
                    var.refresh_inferred_role();
                }
            }
            t.row_titles_expanded = row_titles.iter().any(|s| !s.trim().is_empty());
            t.row_titles = row_titles;
            Ok(TableArchetype::MultipleVariables(t))
        }
        TableKind::Nested => {
            let mut t = NestedTable::new(cols.max(1), 1, rows)?;
            if cols == 0 {
                t.groups.clear();
            } else {
                for (c, grp) in t.groups.iter_mut().enumerate() {
                    grp.title = titles[c].clone();
                    grp.subcolumn_titles = vec![format!("{}:1", grp.title)];
                    for (r, row_vals) in grid.iter().enumerate().take(rows) {
                        grp.cells[r][0] = row_vals[c].clone();
                    }
                }
            }
            t.row_titles = row_titles;
            Ok(TableArchetype::Nested(t))
        }
    }
}

fn populate_single_datasets(data_sets: &mut [DataSetColumn], titles: &[String], grid: &[Vec<Cell>]) {
    for (c, ds) in data_sets.iter_mut().enumerate() {
        if let Some(title) = titles.get(c) {
            ds.title = title.clone();
        }
        for (r, row) in grid.iter().enumerate() {
            if let Some(cell) = row.get(c) {
                ds.cells[r][0] = cell.clone();
            }
        }
    }
}

fn flatten_primary_columns(arch: &TableArchetype, selected: &[usize]) -> (Vec<String>, Vec<String>, Vec<Vec<Cell>>) {
    let rows = arch.row_count();
    let all_titles: Vec<String> = match arch {
        TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::Survival(t) => t.cohorts.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.clone()).collect(),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.title.clone()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.clone()).collect(),
    };
    let titles: Vec<String> = selected.iter().filter_map(|&c| all_titles.get(c).cloned()).collect();
    let row_titles: Vec<String> = match arch {
        TableArchetype::Xy(t) => t.row_titles.clone(),
        TableArchetype::Column(t) => t.row_titles.clone(),
        TableArchetype::Grouped(t) => t.row_titles.clone(),
        TableArchetype::Contingency(t) => t.row_titles.clone(),
        TableArchetype::Survival(t) => t.row_titles.clone(),
        TableArchetype::PartsOfWhole(t) => t.slice_labels.clone(),
        TableArchetype::MultipleVariables(mv) => mv.row_titles.clone(),
        TableArchetype::Nested(t) => t.row_titles.clone(),
    };
    let mut grid = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut row = Vec::with_capacity(selected.len());
        for &c in selected {
            let cell = arch.data_cell(lumencraft_doc::CellCoord::new(r, c, 0)).cloned().unwrap_or_default();
            row.push(cell);
        }
        grid.push(row);
    }
    (titles, row_titles, grid)
}
