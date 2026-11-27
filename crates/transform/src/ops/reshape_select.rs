//! Table transposition, row pruning/decimation, and subset/archetype projection
//! (`TransformAnalysisSpec::Transpose`, `PruneRows`, `SelectAndTransform`,
//! `docs/implementation/data-tables-and-transforms.md` §5.4).

use std::collections::{BTreeSet, HashMap, HashSet};

use curvebook_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSetColumn, DataSheet, DerivedColumnFormula,
    DocError, GroupedTable, InfoSheet, MAX_SHEET_CELLS, MultipleVariablesTable, NestedTable, PartsOfWholeTable, PlateCellOrigin, ReplicateStacking,
    ResultCell, SheetId, SubcolumnMode, SummarizedPoint, SummaryModeRefusalReason, SurvivalTable, TableArchetype, TableKind, VariableColumn,
    VariableRole, VariableSource, XColumnMode, XyTable, default_column_label, transpose_cell_grid, validate_cell_grid, validate_data_sets,
    validate_rows, validate_sheet_cells, validate_title,
};
use curvebook_math::sum;

use crate::error::TransformError;
use crate::formula::{DependencyTarget, Expr, FormulaValue, TableEvalContext, eval_column_expr, extract_dependencies, parse_formula};
use crate::import::validate_cell_budget;
use crate::ops::{TransformContext, TransformOutput, build_summary_tab, snapshot_calculated_variables};

/// Decimation strategy for [`execute_prune`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PruneMethod {
    /// Retain every `k`-th row (`k >= 1`) without averaging intermediate rows.
    EveryNth(usize),
    /// Select approximately `target_points` rows spaced uniformly in `log10(X)` across strictly positive *X* coordinates.
    LogarithmicSpacing {
        /// Desired approximate number of retained *X* grid points (`>= 2`).
        target_points: usize,
    },
    /// Partition rows into consecutive windows of width `k` (`k >= 1`) and emit the mean *X* and *Y* per window.
    AverageBins(usize),
    /// Partition rows into consecutive windows of width `k` (`k >= 1`) and retain both the minimum-*Y* and maximum-*Y* envelope points per bin (preserving sharp peaks during downsampling).
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
    TopN {
        /// Zero-based column index ranked descending.
        column: usize,
        /// Maximum number of top rows retained.
        n: usize,
    },
    /// Keep the `n` rows with smallest active numeric values in `column`.
    BottomN {
        /// Zero-based column index ranked ascending.
        column: usize,
        /// Maximum number of bottom rows retained.
        n: usize,
    },
}

/// Transpose rows and data-set columns using [`curvebook_doc::transpose_cell_grid`].
pub fn execute_transpose(source: &DataSheet, swap_titles: bool, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    let (id, title) = ctx.derived_ids(source, "transpose", "Transposed");
    let (archetype, warnings) = transpose_archetype(&source.archetype, swap_titles)?;
    let mut out = DataSheet::new(id, title, archetype)?;
    out.exclusion_notes = remap_transposed_exclusion_notes(source);
    out.prune_stale_exclusion_notes();
    out.plate_well_origins = remap_transposed_plate_well_origins(source);
    out.prune_stale_plate_well_origins();
    out.archetype.validate()?;

    let summary_tab = build_summary_tab(
        "Transpose summary",
        vec![
            ("Output rows", ResultCell::Number { value: out.archetype.row_count() as f64, prefix: None, units: None }),
            ("Output columns", ResultCell::Number { value: out.archetype.column_count() as f64, prefix: None, units: None }),
            ("Swapped titles", ResultCell::Boolean(swap_titles)),
        ],
        warnings.clone(),
    );
    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
}

fn remap_transposed_exclusion_notes(source: &DataSheet) -> Vec<CellExclusionNote> {
    if source.exclusion_notes.is_empty() {
        return Vec::new();
    }
    let src_rows = source.archetype.row_count();
    let src_cols = source.archetype.column_count();
    let xy_rep_stride = match &source.archetype {
        TableArchetype::Xy(t) => match t.y_subcolumn_mode {
            SubcolumnMode::RawReplicates(n) if n >= 2 => Some(n),
            _ => None,
        },
        _ => None,
    };
    let mut out: Vec<CellExclusionNote> = source
        .exclusion_notes
        .iter()
        .filter_map(|note| {
            if note.coord.col >= src_cols || note.coord.row >= src_rows {
                return None;
            }
            let new_coord = if let Some(stride) = xy_rep_stride {
                if note.coord.subcol >= stride {
                    return None;
                }
                let new_row = note.coord.col.checked_mul(stride)?.checked_add(note.coord.subcol)?;
                CellCoord::new(new_row, note.coord.row, 0)
            } else {
                CellCoord::new(note.coord.col, note.coord.row, note.coord.subcol)
            };
            Some(CellExclusionNote {
                coord: new_coord,
                reason_tag: note.reason_tag.clone(),
                note: note.note.clone(),
                timestamp_utc: note.timestamp_utc.clone(),
            })
        })
        .collect();
    out.sort_by_key(|n| n.coord);
    out
}

fn remap_transposed_plate_well_origins(source: &DataSheet) -> Vec<PlateCellOrigin> {
    if source.plate_well_origins.is_empty() {
        return Vec::new();
    }
    let xy_rep_stride = match &source.archetype {
        TableArchetype::Xy(t) => match t.y_subcolumn_mode {
            SubcolumnMode::RawReplicates(n) if n >= 2 => Some(n),
            _ => None,
        },
        _ => None,
    };

    let mut out: Vec<PlateCellOrigin> = source
        .plate_well_origins
        .iter()
        .filter_map(|origin| {
            let new_coord = if let Some(stride) = xy_rep_stride {
                if origin.coord.subcol >= stride {
                    return None;
                }
                CellCoord::new(origin.coord.col * stride + origin.coord.subcol, origin.coord.row, 0)
            } else {
                CellCoord::new(origin.coord.col, origin.coord.row, origin.coord.subcol)
            };
            Some(PlateCellOrigin { coord: new_coord, well: origin.well.clone() })
        })
        .collect();
    out.sort_by_key(|o| o.coord);
    out.dedup_by_key(|o| o.coord);
    out
}

fn transpose_archetype(arch: &TableArchetype, swap_titles: bool) -> Result<(TableArchetype, Vec<String>), TransformError> {
    arch.validate()?;
    if arch.row_count() == 0 || arch.column_count() == 0 {
        return Err(TransformError::InvalidInput { message: String::from("cannot transpose a table with 0 rows or 0 columns") });
    }
    match arch {
        TableArchetype::Column(t) => {
            if t.subcolumn_mode != SubcolumnMode::Single {
                return Err(TransformError::UnsupportedSubcolumnMode {
                    operation: "Transform: Transpose (Column)",
                    table_kind: TableKind::Column,
                    mode: t.subcolumn_mode,
                    reason: SummaryModeRefusalReason::UnsupportedSummaryMode.message(),
                });
            }
            let tc = transpose_single_columns(&t.data_sets, &t.row_titles, swap_titles)?;
            let row_titles_expanded = !tc.row_titles.iter().all(String::is_empty);
            let out = ColumnTable {
                design_hint: t.design_hint,
                subcolumn_mode: SubcolumnMode::Single,
                row_titles: tc.row_titles,
                row_titles_expanded,
                data_sets: tc.data_sets,
            };
            out.validate()?;
            Ok((TableArchetype::Column(out), Vec::new()))
        }
        TableArchetype::Contingency(t) => {
            let tc = transpose_single_columns(&t.data_sets, &t.row_titles, swap_titles)?;
            let out = ContingencyTable {
                row_factor_title: t.column_factor_title.clone(),
                column_factor_title: t.row_factor_title.clone(),
                row_titles: tc.row_titles,
                data_sets: tc.data_sets,
            };
            out.validate()?;
            Ok((TableArchetype::Contingency(out), Vec::new()))
        }
        TableArchetype::PartsOfWhole(t) => {
            let tc = transpose_single_columns(&t.data_sets, &t.slice_labels, swap_titles)?;
            let out = PartsOfWholeTable { slice_labels: tc.row_titles, data_sets: tc.data_sets };
            out.validate()?;
            Ok((TableArchetype::PartsOfWhole(out), Vec::new()))
        }
        TableArchetype::MultipleVariables(mv) => Ok((transpose_multivariate(mv, swap_titles)?, Vec::new())),
        TableArchetype::Grouped(t) => Ok((transpose_grouped(t, swap_titles)?, Vec::new())),
        TableArchetype::Xy(t) => transpose_xy(t, swap_titles),
        TableArchetype::Survival(_) | TableArchetype::Nested(_) => {
            Err(TransformError::IncompatibleTable { operation: "Transpose", kind: arch.kind() })
        }
    }
}

struct TransposedColumns {
    data_sets: Vec<DataSetColumn>,
    row_titles: Vec<String>,
}

fn transpose_subcolumn_datasets(
    data_sets: &[DataSetColumn],
    row_titles: &[String],
    subs: usize,
    swap_titles: bool,
) -> Result<TransposedColumns, TransformError> {
    let rows = data_sets.first().map_or(row_titles.len(), DataSetColumn::row_count);
    let cols = data_sets.len();
    let new_rows = cols;
    let new_cols = rows;
    validate_rows(new_rows)?;
    validate_data_sets(new_cols)?;
    validate_cell_budget(new_rows, new_cols, subs, 0, "transpose")?;

    let mut sub_grids = Vec::with_capacity(subs);
    for s in 0..subs {
        let mut grid = vec![vec![Cell::empty(); cols]; rows];
        for (c, ds) in data_sets.iter().enumerate() {
            for (r, row_cells) in ds.cells.iter().enumerate() {
                if let Some(cell) = row_cells.get(s)
                    && let Some(slot) = grid.get_mut(r).and_then(|gr| gr.get_mut(c))
                {
                    *slot = cell.clone();
                }
            }
        }
        sub_grids.push(transpose_cell_grid(&grid));
    }

    let new_col_titles: Vec<String> = if swap_titles {
        (0..new_cols).map(|c| row_titles.get(c).cloned().unwrap_or_else(|| default_column_label(c))).collect()
    } else {
        (0..new_cols).map(default_column_label).collect()
    };
    let new_row_titles: Vec<String> = if new_cols == 0 {
        Vec::new()
    } else if swap_titles {
        data_sets.iter().map(|ds| ds.title.clone()).collect()
    } else {
        vec![String::new(); new_rows]
    };

    let mut out_sets = Vec::with_capacity(new_cols);
    for c in 0..new_cols {
        let title = new_col_titles.get(c).cloned().unwrap_or_else(|| default_column_label(c));
        let mut ds = DataSetColumn::new(title, new_rows, subs)?;
        for (r, row_cells) in ds.cells.iter_mut().enumerate().take(new_rows) {
            for (s, sub_grid) in sub_grids.iter().enumerate().take(subs) {
                if let Some(slot) = row_cells.get_mut(s) {
                    *slot = sub_grid.get(r).and_then(|gr| gr.get(c)).cloned().unwrap_or_default();
                }
            }
        }
        out_sets.push(ds);
    }

    Ok(TransposedColumns { data_sets: out_sets, row_titles: new_row_titles })
}

fn transpose_single_columns(data_sets: &[DataSetColumn], row_titles: &[String], swap_titles: bool) -> Result<TransposedColumns, TransformError> {
    transpose_subcolumn_datasets(data_sets, row_titles, 1, swap_titles)
}

fn transpose_multivariate(mv: &MultipleVariablesTable, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    let rows = mv.row_count();
    let cols = mv.variables.len();
    let new_rows = cols;
    let new_cols = rows;
    validate_rows(new_rows)?;
    validate_data_sets(new_cols)?;
    validate_cell_budget(new_rows, new_cols, 1, 0, "transpose")?;
    let mut grid = vec![vec![Cell::empty(); cols]; rows];
    for (c, var) in mv.variables.iter().enumerate() {
        for (r, cell) in var.cells.iter().enumerate() {
            if let Some(slot) = grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = cell.clone();
            }
        }
    }
    let transposed = transpose_cell_grid(&grid);
    let mut variables = Vec::with_capacity(new_cols);
    for c in 0..new_cols {
        let title = if swap_titles {
            mv.row_titles.get(c).filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| format!("Variable {}", default_column_label(c)))
        } else {
            format!("Variable {}", default_column_label(c))
        };
        let mut var = VariableColumn::new(title, new_rows)?;
        for r in 0..new_rows {
            if let Some(slot) = var.cells.get_mut(r) {
                *slot = transposed.get(r).and_then(|row| row.get(c)).cloned().unwrap_or_default();
            }
        }
        var.refresh_inferred_role();
        variables.push(var);
    }
    let row_titles = if swap_titles { mv.variables.iter().map(|v| v.title.clone()).collect() } else { vec![String::new(); new_rows] };
    let row_titles_expanded = !row_titles.iter().all(String::is_empty);
    let out = MultipleVariablesTable { row_titles, row_titles_expanded, variables };
    out.validate()?;
    Ok(TableArchetype::MultipleVariables(out))
}

fn transpose_grouped(t: &GroupedTable, swap_titles: bool) -> Result<TableArchetype, TransformError> {
    let subs = t.subcolumn_mode.subcolumn_count();
    let tc = transpose_subcolumn_datasets(&t.data_sets, &t.row_titles, subs, swap_titles)?;
    let out = GroupedTable {
        row_factor_title: t.column_factor_title.clone(),
        column_factor_title: t.row_factor_title.clone(),
        row_titles: tc.row_titles,
        row_numeric_spacing: None,
        subcolumn_mode: t.subcolumn_mode,
        replicate_stacking: ReplicateStacking::SideBySide,
        three_way_grouping: None,
        data_sets: tc.data_sets,
    };
    out.validate()?;
    Ok(TableArchetype::Grouped(out))
}

fn transpose_xy(t: &XyTable, swap_titles: bool) -> Result<(TableArchetype, Vec<String>), TransformError> {
    let mut has_x_data = false;
    let row_labels: Vec<String> = (0..t.row_count())
        .map(|r| {
            let x_cell = t.materialized_x_cell_at(r);
            let x_label = match &x_cell.value {
                CellValue::Number(x) if x.is_finite() => {
                    has_x_data = true;
                    let norm = if *x == 0.0 { 0.0 } else { *x };
                    Some(format!("{norm}"))
                }
                CellValue::Text(s) if !s.trim().is_empty() => {
                    has_x_data = true;
                    Some(s.trim().to_owned())
                }
                _ => None,
            };
            let rt = t.row_titles.get(r).map(String::as_str).unwrap_or("");
            if !rt.is_empty() {
                rt.to_owned()
            } else if let Some(lbl) = x_label {
                lbl
            } else {
                default_column_label(r)
            }
        })
        .collect();
    let mut warnings = Vec::new();
    if !swap_titles && has_x_data {
        warnings.push(String::from("transposing XY table with swap_row_and_column_titles=false discarded source X values"));
    }
    if t.x_mode.subcolumn_count() > 1 {
        warnings.push(String::from("Column tables do not support X error subcolumns; transpose omitted X error values"));
    }
    match t.y_subcolumn_mode {
        SubcolumnMode::Single | SubcolumnMode::RawReplicates(1) => {
            let tc = transpose_single_columns(&t.data_sets, &row_labels, swap_titles)?;
            let row_titles_expanded = !tc.row_titles.iter().all(String::is_empty);
            let out = ColumnTable {
                design_hint: ColumnDesignHint::UnpairedReplicates,
                subcolumn_mode: SubcolumnMode::Single,
                row_titles: tc.row_titles,
                row_titles_expanded,
                data_sets: tc.data_sets,
            };
            out.validate()?;
            Ok((TableArchetype::Column(out), warnings))
        }
        SubcolumnMode::MeanSdN
        | SubcolumnMode::MeanSemN
        | SubcolumnMode::MeanCvN
        | SubcolumnMode::MeanSd
        | SubcolumnMode::MeanSem
        | SubcolumnMode::MeanLowHigh
        | SubcolumnMode::UpperLowerLimits
        | SubcolumnMode::MedianIqr => {
            let tc = transpose_subcolumn_datasets(&t.data_sets, &row_labels, t.y_subcolumn_mode.subcolumn_count(), swap_titles)?;
            let row_titles_expanded = !tc.row_titles.iter().all(String::is_empty);
            let out = ColumnTable {
                design_hint: ColumnDesignHint::PrecomputedError,
                subcolumn_mode: t.y_subcolumn_mode,
                row_titles: tc.row_titles,
                row_titles_expanded,
                data_sets: tc.data_sets,
            };
            out.validate()?;
            Ok((TableArchetype::Column(out), warnings))
        }
        SubcolumnMode::RawReplicates(n) => {
            let rows = t.row_count();
            let cols = t.data_sets.len();
            let new_rows = cols
                .checked_mul(n)
                .ok_or_else(|| TransformError::InvalidSpec { message: String::from("transposed replicate row count overflow") })?;
            validate_rows(new_rows)?;
            validate_data_sets(rows)?;
            validate_cell_budget(new_rows, rows, 1, 0, "transpose")?;
            let mut out = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, rows.max(1), new_rows)?;
            if rows == 0 {
                out.data_sets.clear();
            } else {
                for (r, ds_out) in out.data_sets.iter_mut().enumerate().take(rows) {
                    ds_out.title =
                        if swap_titles { row_labels.get(r).cloned().unwrap_or_else(|| default_column_label(r)) } else { default_column_label(r) };
                    for (c, ds_in) in t.data_sets.iter().enumerate() {
                        for s in 0..n {
                            if let Some(cell) = ds_in.cell(r, s)
                                && let Some(slot) = ds_out.cell_mut(c * n + s, 0)
                            {
                                *slot = cell.clone();
                            }
                        }
                    }
                }
            }
            out.row_titles = if swap_titles {
                let mut rtitles = Vec::with_capacity(new_rows);
                for ds_in in &t.data_sets {
                    for s in 0..n {
                        rtitles.push(format!("{} (Y{})", ds_in.title, s + 1));
                    }
                }
                rtitles
            } else {
                vec![String::new(); new_rows]
            };
            out.row_titles_expanded = !out.row_titles.iter().all(String::is_empty);
            warnings.push(format!("transposed XY RawReplicates({n}) into {new_rows} stacked Column rows to preserve all replicate observations"));
            Ok((TableArchetype::Column(out), warnings))
        }
    }
}

/// Decimate or bin rows of a [`DataSheet`] according to [`PruneMethod`].
pub fn execute_prune(source: &DataSheet, method: PruneMethod, ctx: &TransformContext<'_>) -> Result<TransformOutput, TransformError> {
    source.archetype.validate()?;
    let row_count = source.archetype.row_count();
    let (id, title) = ctx.derived_ids(source, "prune", "Pruned");

    let mut out = source.clone();
    out.id = SheetId::new(id);
    out.title = title;
    out.frozen = false;
    out.linked_regions.clear();

    let (kept_rows, warnings) = match method {
        PruneMethod::EveryNth(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("EveryNth step k must be >= 1") });
            }
            let idxs: Vec<usize> = (0..row_count).step_by(k).collect();
            try_retain_rows(&mut out.archetype, &idxs)?;
            snapshot_calculated_variables(&mut out.archetype);
            out.exclusion_notes = crate::randomize::remap_exclusion_notes(&out.exclusion_notes, &idxs);
            out.prune_stale_exclusion_notes();
            out.plate_well_origins = crate::randomize::remap_plate_well_origins(&out.plate_well_origins, &idxs);
            out.prune_stale_plate_well_origins();
            (idxs.len(), Vec::new())
        }
        PruneMethod::LogarithmicSpacing { target_points } => {
            if target_points == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("LogarithmicSpacing target_points must be >= 1") });
            }
            let idxs = select_log_spaced_indices(&source.archetype, target_points);
            try_retain_rows(&mut out.archetype, &idxs)?;
            snapshot_calculated_variables(&mut out.archetype);
            out.exclusion_notes = crate::randomize::remap_exclusion_notes(&out.exclusion_notes, &idxs);
            out.prune_stale_exclusion_notes();
            out.plate_well_origins = crate::randomize::remap_plate_well_origins(&out.plate_well_origins, &idxs);
            out.prune_stale_plate_well_origins();
            (idxs.len(), Vec::new())
        }
        PruneMethod::AverageBins(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("AverageBins width k must be >= 1") });
            }
            let res = average_bins_archetype(&mut out.archetype, k)?;
            remap_binned_exclusion_notes(&mut out, k);
            out.plate_well_origins.clear();
            res
        }
        PruneMethod::MinMaxBins(k) => {
            if k == 0 {
                return Err(TransformError::InvalidSpec { message: String::from("MinMaxBins width k must be >= 1") });
            }
            let idxs = select_minmax_bin_indices(&source.archetype, k);
            try_retain_rows(&mut out.archetype, &idxs)?;
            snapshot_calculated_variables(&mut out.archetype);
            out.exclusion_notes = crate::randomize::remap_exclusion_notes(&out.exclusion_notes, &idxs);
            out.prune_stale_exclusion_notes();
            out.plate_well_origins = crate::randomize::remap_plate_well_origins(&out.plate_well_origins, &idxs);
            out.prune_stale_plate_well_origins();
            (idxs.len(), Vec::new())
        }
    };

    out.validate()?;

    let summary_tab = build_summary_tab(
        "Prune summary",
        vec![
            ("Original rows", ResultCell::Number { value: row_count as f64, prefix: None, units: None }),
            ("Kept rows", ResultCell::Number { value: kept_rows as f64, prefix: None, units: None }),
        ],
        warnings.clone(),
    );

    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
}

fn remap_binned_exclusion_notes(sheet: &mut DataSheet, k: usize) {
    if sheet.exclusion_notes.is_empty() {
        return;
    }
    let mut seen: HashSet<CellCoord> = HashSet::new();
    let mut mapped = Vec::with_capacity(sheet.exclusion_notes.len());
    let mut notes = std::mem::take(&mut sheet.exclusion_notes);
    notes.sort_by_key(|n| n.coord);
    for mut note in notes {
        note.coord.row /= k;
        if seen.insert(note.coord) {
            mapped.push(note);
        }
    }
    sheet.exclusion_notes = mapped;
    sheet.prune_stale_exclusion_notes();
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

    if let TableArchetype::Xy(xy) = arch {
        let mut xs: Vec<(usize, f64)> = (0..n).filter_map(|r| xy.x_value_at(r).filter(|&x| x.is_finite() && x > 0.0).map(|x| (r, x.ln()))).collect();
        xs.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        if let (Some(&(_, log_min)), Some(&(_, log_max))) = (xs.first(), xs.last())
            && xs.len() >= 2
            && log_max > log_min
        {
            let mut selected = BTreeSet::new();
            for step in 0..target_points {
                let target_ln = log_min + ((step as f64) / ((target_points - 1) as f64)) * (log_max - log_min);
                let pos = xs.partition_point(|&(_, lx)| lx < target_ln);
                let best_r = match (pos.checked_sub(1).and_then(|i| xs.get(i)), xs.get(pos)) {
                    (Some(&(r_lo, l_lo)), Some(&(r_hi, l_hi))) => {
                        if (target_ln - l_lo) <= (l_hi - target_ln) {
                            r_lo
                        } else {
                            r_hi
                        }
                    }
                    (Some(&(r_lo, _)), None) => r_lo,
                    (None, Some(&(r_hi, _))) => r_hi,
                    (None, None) => continue,
                };
                selected.insert(best_r);
            }
            return selected.into_iter().collect();
        }
    }

    let mut out = BTreeSet::new();
    for step in 0..target_points {
        let frac = (step as f64) / ((target_points - 1) as f64);
        let log_pos = ((n as f64).powf(frac) - 1.0).round() as usize;
        out.insert(log_pos.min(n - 1));
    }
    out.into_iter().collect()
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

fn dataset_row_active_scalar(data_sets: &[DataSetColumn], col: usize, row: usize, mode: SubcolumnMode) -> Option<f64> {
    let subs = data_sets.get(col)?.cells.get(row)?;
    if mode.is_pre_summarized() || mode == SubcolumnMode::Single {
        subs.first()?.active_number()
    } else {
        let active: Vec<f64> = subs.iter().filter_map(Cell::active_number).collect();
        sum::mean(&active).ok()
    }
}

fn dataset_row_has_active(data_sets: &[DataSetColumn], col: usize, row: usize, mode: SubcolumnMode) -> bool {
    let Some(subs) = data_sets.get(col).and_then(|d| d.cells.get(row)) else {
        return false;
    };
    if mode.is_pre_summarized() {
        subs.first().is_some_and(|c| !c.excluded && !c.is_empty())
    } else {
        subs.iter().any(|c| !c.excluded && !c.is_empty())
    }
}

fn primary_row_scalar(arch: &TableArchetype, row: usize) -> Option<f64> {
    match arch {
        TableArchetype::Xy(t) => dataset_row_active_scalar(&t.data_sets, 0, row, t.y_subcolumn_mode),
        TableArchetype::Column(t) => dataset_row_active_scalar(&t.data_sets, 0, row, t.subcolumn_mode),
        TableArchetype::Grouped(t) => dataset_row_active_scalar(&t.data_sets, 0, row, t.subcolumn_mode),
        TableArchetype::Contingency(t) => t.data_sets.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::Survival(t) => t.cohorts.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.first()?.cells.get(row)?.first()?.active_number(),
        TableArchetype::MultipleVariables(mv) => mv.variables.first()?.cells.get(row)?.active_number(),
        TableArchetype::Nested(t) => {
            let active: Vec<f64> = t.groups.first()?.cells.get(row)?.iter().filter_map(Cell::active_number).collect();
            sum::mean(&active).ok()
        }
    }
}

fn average_bins_archetype(arch: &mut TableArchetype, k: usize) -> Result<(usize, Vec<String>), TransformError> {
    if matches!(arch, TableArchetype::Survival(_)) {
        return Err(TransformError::IncompatibleTable { operation: "PruneRows (AverageBins)", kind: TableKind::Survival });
    }
    let n = arch.row_count();
    let mut warnings = Vec::new();
    if n == 0 {
        return Ok((0, warnings));
    }
    let bin_count = n.div_ceil(k);
    match arch {
        TableArchetype::Xy(t) => {
            let x_mode = match &t.x_mode {
                XColumnMode::Numbers { subcolumn_mode } => *subcolumn_mode,
                _ => {
                    validate_cell_grid(bin_count, t.data_sets.len(), t.y_subcolumn_mode.subcolumn_count(), 1)?;
                    let mut warned_mixed = false;
                    let mut warned_text_conflict = false;
                    let mut binned_x = Vec::with_capacity(bin_count);
                    let mut start = 0;
                    while start < n {
                        let end = (start + k).min(n);
                        let chunk: Vec<Vec<Cell>> = (start..end).map(|r| vec![t.materialized_x_cell_at(r)]).collect();
                        binned_x.push(vec![average_single_bin_subcol(
                            &chunk,
                            0,
                            &t.x_title,
                            &mut warned_mixed,
                            &mut warned_text_conflict,
                            &mut warnings,
                        )]);
                        start = end;
                    }
                    t.x_cells = binned_x;
                    t.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
                    t.row_titles = bin_titles(&t.row_titles, k);
                    for ds in &mut t.data_sets {
                        ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, t.y_subcolumn_mode, &ds.title, &mut warnings);
                    }
                    return Ok((bin_count, warnings));
                }
            };
            t.x_cells = average_cell_grid_bins_with_mode(&t.x_cells, k, x_mode, &t.x_title, &mut warnings);
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, t.y_subcolumn_mode, &ds.title, &mut warnings);
            }
        }
        TableArchetype::Column(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, t.subcolumn_mode, &ds.title, &mut warnings);
            }
        }
        TableArchetype::Grouped(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            if let Some(spacing) = &mut t.row_numeric_spacing {
                *spacing = spacing
                    .chunks(k)
                    .map(|chunk| {
                        let nums: Vec<f64> = chunk.iter().filter_map(|v| v.filter(|x| x.is_finite())).collect();
                        sum::mean(&nums).ok()
                    })
                    .collect();
            }
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, t.subcolumn_mode, &ds.title, &mut warnings);
            }
        }
        TableArchetype::Contingency(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, SubcolumnMode::Single, &ds.title, &mut warnings);
            }
            t.validate_counts()?;
        }
        TableArchetype::PartsOfWhole(t) => {
            t.slice_labels = bin_titles(&t.slice_labels, k);
            for ds in &mut t.data_sets {
                ds.cells = average_cell_grid_bins_with_mode(&ds.cells, k, SubcolumnMode::Single, &ds.title, &mut warnings);
            }
        }
        TableArchetype::Survival(_) => {
            return Err(TransformError::IncompatibleTable { operation: "PruneRows (AverageBins)", kind: TableKind::Survival });
        }
        TableArchetype::MultipleVariables(mv) => {
            mv.row_titles = bin_titles(&mv.row_titles, k);
            for var in &mut mv.variables {
                if matches!(var.source, VariableSource::Calculated { .. }) || var.role == VariableRole::Calculated {
                    var.source = VariableSource::Entered;
                    var.role = VariableRole::Continuous;
                    var.role_inferred = true;
                }
                let grid: Vec<Vec<Cell>> = var.cells.iter().map(|c| vec![c.clone()]).collect();
                var.cells = average_cell_grid_bins_with_mode(&grid, k, SubcolumnMode::Single, &var.title, &mut warnings)
                    .into_iter()
                    .map(|r| r.into_iter().next().unwrap_or_default())
                    .collect();
                var.refresh_inferred_role();
            }
        }
        TableArchetype::Nested(t) => {
            t.row_titles = bin_titles(&t.row_titles, k);
            for grp in &mut t.groups {
                grp.cells = average_cell_grid_bins_with_mode(&grp.cells, k, SubcolumnMode::Single, &grp.title, &mut warnings);
            }
        }
    }
    Ok((bin_count, warnings))
}

fn bin_titles(titles: &[String], k: usize) -> Vec<String> {
    titles.chunks(k).map(|chunk| chunk.first().cloned().unwrap_or_default()).collect()
}

fn pooled_parametric_moments(points: &[SummarizedPoint]) -> Option<(f64, f64, f64)> {
    let n_vals: Vec<f64> = points.iter().map(|p| p.n).collect();
    let n_tot = sum::sum(&n_vals);
    if n_tot <= 0.0 || !n_tot.is_finite() {
        return None;
    }

    let max_abs_mean = points.iter().map(|p| p.mean.abs()).fold(0.0_f64, f64::max);
    let mean_p = if max_abs_mean == 0.0 {
        0.0
    } else {
        let weighted_terms: Vec<f64> = points.iter().map(|p| (p.n / n_tot) * (p.mean / max_abs_mean)).collect();
        sum::sum(&weighted_terms) * max_abs_mean
    };
    if !mean_p.is_finite() {
        return None;
    }

    let sd_p = if n_tot <= 1.0 {
        0.0
    } else {
        let df = n_tot - 1.0;
        let diff_overflows = points.iter().any(|p| !(p.mean - mean_p).is_finite());
        let sd_scale = if diff_overflows {
            max_abs_mean
        } else {
            let max_diff = points.iter().map(|p| (p.mean - mean_p).abs()).fold(0.0_f64, f64::max);
            let max_within_rms = points.iter().map(|p| (p.ss_within.max(0.0) / df).sqrt()).fold(0.0_f64, f64::max);
            max_diff.max(max_within_rms)
        };
        if sd_scale == 0.0 {
            0.0
        } else {
            let mut scaled_terms = Vec::with_capacity(points.len() * 2);
            for pt in points {
                let d_scaled = if diff_overflows { (pt.mean / sd_scale) - (mean_p / sd_scale) } else { (pt.mean - mean_p) / sd_scale };
                scaled_terms.push((pt.n / df) * d_scaled * d_scaled);
                let w_scaled = (pt.ss_within.max(0.0) / df).sqrt() / sd_scale;
                scaled_terms.push(w_scaled * w_scaled);
            }
            sd_scale * sum::sum(&scaled_terms).max(0.0).sqrt()
        }
    };
    if !sd_p.is_finite() {
        return None;
    }
    Some((mean_p, sd_p, n_tot))
}

fn pool_parametric_bin(
    chunk: &[Vec<Cell>],
    mode: SubcolumnMode,
    subcols: usize,
    col_label: &str,
    warned_incomplete: &mut bool,
    warnings: &mut Vec<String>,
) -> Vec<Cell> {
    let active_rows: Vec<&[Cell]> = chunk.iter().filter_map(|row| row.first().and_then(Cell::active_number).map(|_| row.as_slice())).collect();
    if active_rows.is_empty() {
        return vec![Cell::empty(); subcols];
    }

    let mut points = Vec::with_capacity(active_rows.len());
    for row in &active_rows {
        match SummarizedPoint::reconstruct(mode, row) {
            Ok(Some(pt)) => points.push(pt),
            _ => {
                if !*warned_incomplete {
                    warnings.push(format!(
                        "could not pool incomplete pre-summarized {mode:?} rows in `{col_label}`; averaged primary means and cleared spread/N subcolumns"
                    ));
                    *warned_incomplete = true;
                }
                let means: Vec<f64> = active_rows.iter().filter_map(|r| r.first().and_then(Cell::active_number)).collect();
                let mut out = vec![Cell::empty(); subcols];
                if let Ok(avg) = sum::mean(&means)
                    && let Some(slot) = out.first_mut()
                {
                    *slot = Cell::number(avg);
                }
                return out;
            }
        }
    }

    let Some((mean_p, sd_p, n_tot)) = pooled_parametric_moments(&points) else {
        if !*warned_incomplete {
            warnings.push(format!("pooled parametric moments overflowed finite f64 range in `{col_label}`; cleared binned summary row"));
            *warned_incomplete = true;
        }
        return vec![Cell::empty(); subcols];
    };

    let spread_cell = match mode {
        SubcolumnMode::MeanSdN => Cell::number(sd_p),
        SubcolumnMode::MeanSemN => Cell::number(sd_p / n_tot.sqrt()),
        SubcolumnMode::MeanCvN => {
            if mean_p.abs() > 0.0 {
                let cv = (sd_p / mean_p.abs()) * 100.0;
                if cv.is_finite() {
                    Cell::number(cv)
                } else {
                    if !*warned_incomplete {
                        warnings.push(format!("pooled %CV overflowed in `{col_label}`; %CV subcolumn left empty"));
                        *warned_incomplete = true;
                    }
                    Cell::empty()
                }
            } else if sd_p == 0.0 {
                Cell::number(0.0)
            } else {
                if !*warned_incomplete {
                    warnings.push(format!("pooled mean is zero in `{col_label}`; %CV subcolumn is undefined and left empty"));
                    *warned_incomplete = true;
                }
                Cell::empty()
            }
        }
        _ => Cell::empty(),
    };

    vec![Cell::number(mean_p), spread_cell, Cell::number(n_tot)]
}

fn average_cell_grid_bins_with_mode(
    rows: &[Vec<Cell>],
    k: usize,
    mode: SubcolumnMode,
    col_label: &str,
    warnings: &mut Vec<String>,
) -> Vec<Vec<Cell>> {
    let subcols = rows.first().map_or(0, Vec::len);
    if mode.supports_parametric_moments() && subcols == 3 {
        let mut warned_incomplete = false;
        return rows.chunks(k).map(|chunk| pool_parametric_bin(chunk, mode, subcols, col_label, &mut warned_incomplete, warnings)).collect();
    }
    if mode.is_pre_summarized() {
        warnings.push(format!(
            "cannot pool non-parametric summary mode {mode:?} in `{col_label}`; averaged primary subcolumn 0 and cleared unpoolable interval subcolumns"
        ));
        return rows
            .chunks(k)
            .map(|chunk| {
                let mut out = vec![Cell::empty(); subcols];
                let nums: Vec<f64> = chunk.iter().filter_map(|r| r.first().and_then(Cell::active_number)).collect();
                if let Ok(avg) = sum::mean(&nums)
                    && let Some(slot) = out.first_mut()
                {
                    *slot = Cell::number(avg);
                }
                out
            })
            .collect();
    }

    let mut warned_mixed = false;
    let mut warned_text_conflict = false;
    rows.chunks(k)
        .map(|chunk| {
            (0..subcols).map(|s| average_single_bin_subcol(chunk, s, col_label, &mut warned_mixed, &mut warned_text_conflict, warnings)).collect()
        })
        .collect()
}

fn average_single_bin_subcol(
    chunk: &[Vec<Cell>],
    subcol: usize,
    col_label: &str,
    warned_mixed: &mut bool,
    warned_text_conflict: &mut bool,
    warnings: &mut Vec<String>,
) -> Cell {
    let nums: Vec<f64> = chunk.iter().filter_map(|r| r.get(subcol).and_then(Cell::active_number)).collect();
    let texts: Vec<&str> = chunk.iter().filter_map(|r| r.get(subcol).and_then(Cell::active_text)).collect();
    if !nums.is_empty() {
        if !texts.is_empty() && !*warned_mixed {
            warnings.push(format!("column `{col_label}` contains mixed text and numeric cells in binned rows; ignored text values"));
            *warned_mixed = true;
        }
        return sum::mean(&nums).map_or_else(|_| Cell::empty(), Cell::number);
    }
    if let Some(&first_text) = texts.first() {
        if texts.iter().all(|&t| t == first_text) {
            return Cell::text(first_text);
        }
        if !*warned_text_conflict {
            warnings.push(format!("column `{col_label}` contains conflicting non-numeric/text values in binned rows; cleared ambiguous binned cell"));
            *warned_text_conflict = true;
        }
        return Cell::empty();
    }
    fallback_excluded_bin_cell(chunk, subcol)
}

fn fallback_excluded_bin_cell(chunk: &[Vec<Cell>], subcol: usize) -> Cell {
    let excl_nums: Vec<f64> = chunk.iter().filter_map(|r| r.get(subcol)).filter(|c| c.excluded).filter_map(|c| c.value.finite_f64()).collect();
    if !excl_nums.is_empty() {
        return sum::mean(&excl_nums).map_or_else(|_| Cell::empty(), Cell::excluded_number);
    }
    let excl_texts: Vec<&str> = chunk
        .iter()
        .filter_map(|r| r.get(subcol))
        .filter(|c| c.excluded)
        .filter_map(|c| match &c.value {
            curvebook_doc::CellValue::Text(s) if !s.trim().is_empty() => Some(s.as_str()),
            _ => None,
        })
        .collect();
    if let Some(&first_text) = excl_texts.first()
        && excl_texts.iter().all(|&t| t == first_text)
    {
        return Cell::excluded_text(first_text);
    }
    Cell::empty()
}

pub(crate) fn try_retain_rows(arch: &mut TableArchetype, indices: &[usize]) -> Result<(), TransformError> {
    if let TableArchetype::Xy(t) = arch
        && matches!(t.x_mode, XColumnMode::GeneratedSequence { .. })
    {
        let rows = t.row_count();
        let kept_rows = indices.iter().filter(|&&r| r < rows).count();
        validate_cell_grid(kept_rows, t.data_sets.len(), t.y_subcolumn_mode.subcolumn_count(), 1)?;
    }
    retain_rows(arch, indices);
    Ok(())
}

pub(crate) fn retain_rows(arch: &mut TableArchetype, indices: &[usize]) {
    match arch {
        TableArchetype::Xy(t) => {
            if matches!(t.x_mode, XColumnMode::GeneratedSequence { .. }) {
                let rows = t.row_count();
                let mat: Vec<Vec<Cell>> = indices.iter().filter(|&&r| r < rows).map(|&r| vec![t.materialized_x_cell_at(r)]).collect();
                t.x_cells = mat;
                t.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
            } else {
                t.x_cells = indices.iter().filter_map(|&r| t.x_cells.get(r).cloned()).collect();
            }
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
    execute_select_with_derived(source, rules, &[], target_archetype, ctx)
}

/// Filter rows according to [`SelectRule`]s, evaluate custom derived column-transform formulas
/// (`derived_columns`), and optionally project into `target_archetype`.
pub fn execute_select_with_derived(
    source: &DataSheet,
    rules: &[SelectRule],
    derived_columns: &[DerivedColumnFormula],
    target_archetype: TableKind,
    ctx: &TransformContext<'_>,
) -> Result<TransformOutput, TransformError> {
    source.archetype.validate()?;
    let row_count = source.archetype.row_count();
    let base_selected_cols = ctx.active_columns(source.archetype.column_count())?;

    let mut parsed_derived: Vec<(String, Expr)> = Vec::with_capacity(derived_columns.len());
    for dc in derived_columns {
        let title = dc.title.trim();
        if title.is_empty() {
            return Err(TransformError::InvalidSpec { message: String::from("derived column title cannot be empty") });
        }
        validate_title(title)?;
        let expr = parse_formula(&dc.formula)?;
        parsed_derived.push((title.to_owned(), expr));
    }

    let predicate_needs_derived = predicate_depends_on_derived(rules, &source.archetype, &parsed_derived);

    let mut working = source.clone();
    let mut output_cols = base_selected_cols.clone();
    let kept_indices: Vec<usize>;

    if predicate_needs_derived {
        for (title, expr) in &parsed_derived {
            let col_idx = apply_derived_column(&mut working, title, expr, ctx.primary_info_sheet())?;
            working.plate_well_origins.retain(|o| o.coord.col != col_idx);
            if !output_cols.contains(&col_idx) {
                output_cols.push(col_idx);
            }
        }
        let mut keep_mask = vec![true; row_count];
        for rule in rules {
            apply_select_rule(&working, rule, &base_selected_cols, &mut keep_mask, ctx)?;
        }
        kept_indices = keep_mask.iter().enumerate().filter_map(|(r, &k)| k.then_some(r)).collect();
        try_retain_rows(&mut working.archetype, &kept_indices)?;
        working.exclusion_notes = crate::randomize::remap_exclusion_notes(&working.exclusion_notes, &kept_indices);
        working.prune_stale_exclusion_notes();
        working.plate_well_origins = crate::randomize::remap_plate_well_origins(&working.plate_well_origins, &kept_indices);
        working.prune_stale_plate_well_origins();
    } else {
        let mut keep_mask = vec![true; row_count];
        for rule in rules {
            apply_select_rule(source, rule, &base_selected_cols, &mut keep_mask, ctx)?;
        }
        kept_indices = keep_mask.iter().enumerate().filter_map(|(r, &k)| k.then_some(r)).collect();
        try_retain_rows(&mut working.archetype, &kept_indices)?;
        working.exclusion_notes = crate::randomize::remap_exclusion_notes(&working.exclusion_notes, &kept_indices);
        working.prune_stale_exclusion_notes();
        working.plate_well_origins = crate::randomize::remap_plate_well_origins(&working.plate_well_origins, &kept_indices);
        working.prune_stale_plate_well_origins();
        for (title, expr) in &parsed_derived {
            let col_idx = apply_derived_column(&mut working, title, expr, ctx.primary_info_sheet())?;
            working.plate_well_origins.retain(|o| o.coord.col != col_idx);
            if !output_cols.contains(&col_idx) {
                output_cols.push(col_idx);
            }
        }
    }

    let (id, title) = ctx.derived_ids(source, "select", "Selected");
    let mut warnings = Vec::new();
    let (projected, projected_notes, projected_origins) = if working.kind() == target_archetype {
        let (remapped_notes, remapped_origins) = if !ctx.selected_data_sets.is_empty() || !parsed_derived.is_empty() {
            retain_columns(&mut working.archetype, &output_cols)?;
            (
                remap_selected_column_notes(&working.exclusion_notes, &output_cols, false),
                remap_selected_column_origins(&working.plate_well_origins, &output_cols, false),
            )
        } else {
            (working.exclusion_notes.clone(), working.plate_well_origins.clone())
        };
        (working.archetype, remapped_notes, remapped_origins)
    } else {
        let y_subs = match &working.archetype {
            TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
            TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Nested(t) => {
                output_cols.iter().filter_map(|&c| t.groups.get(c).map(curvebook_doc::NestedGroupColumn::biological_unit_count)).max().unwrap_or(1)
            }
            _ => 1,
        };
        let x_subs = match &working.archetype {
            TableArchetype::Xy(t) => t.x_mode.subcolumn_count(),
            _ => 1,
        };
        let max_subs = y_subs.max(x_subs);
        if max_subs > 1 {
            warnings.push(format!(
                "source table carries {max_subs} subcolumns per column; projecting to {} retained primary subcolumn 0 only",
                target_archetype.label()
            ));
        }
        let has_external_axis = matches!(&working.archetype, TableArchetype::Xy(_) | TableArchetype::Survival(_));
        let drops_axis_target = matches!(target_archetype, TableKind::Contingency | TableKind::PartsOfWhole);
        let include_axis = has_external_axis && !drops_axis_target;
        if has_external_axis && drops_axis_target {
            warnings.push(format!("source X / Time column omitted when projecting to {} (counts/proportions target)", target_archetype.label()));
        }
        let axis_shift = if include_axis && !matches!(target_archetype, TableKind::Xy | TableKind::Survival) {
            ProjectedAxisShift::PrependExternalAxis
        } else if !has_external_axis && matches!(target_archetype, TableKind::Xy | TableKind::Survival) && output_cols.len() >= 2 {
            ProjectedAxisShift::ConsumeFirstColumnAsAxis
        } else {
            ProjectedAxisShift::None
        };
        let remapped_notes = remap_projected_column_notes(&working.exclusion_notes, &output_cols, axis_shift, target_archetype, &mut warnings);
        let remapped_origins = remap_projected_column_origins(&working.plate_well_origins, &output_cols, axis_shift);
        (project_archetype(&working.archetype, target_archetype, &output_cols, include_axis)?, remapped_notes, remapped_origins)
    };
    let mut out = DataSheet::new(id, title, projected)?;
    out.exclusion_notes = projected_notes;
    out.prune_stale_exclusion_notes();
    out.plate_well_origins = projected_origins;
    out.prune_stale_plate_well_origins();

    let mut summary_rows = vec![
        ("Original rows", ResultCell::Number { value: row_count as f64, prefix: None, units: None }),
        ("Selected rows", ResultCell::Number { value: kept_indices.len() as f64, prefix: None, units: None }),
        ("Target archetype", ResultCell::Text(String::from(target_archetype.label()))),
    ];
    if !parsed_derived.is_empty() {
        summary_rows.push(("Derived columns", ResultCell::Number { value: parsed_derived.len() as f64, prefix: None, units: None }));
    }
    out.archetype.validate()?;

    let summary_tab = build_summary_tab("Select and transform summary", summary_rows, warnings.clone());
    Ok(TransformOutput { data_sheet: out, summary_tab, scalars: Vec::new(), warnings })
}

#[derive(Clone, Copy)]
enum ProjectedAxisShift {
    None,
    PrependExternalAxis,
    ConsumeFirstColumnAsAxis,
}

fn remap_selected_column_notes(notes: &[CellExclusionNote], selected_cols: &[usize], primary_only: bool) -> Vec<CellExclusionNote> {
    if notes.is_empty() {
        return Vec::new();
    }
    let col_map: HashMap<usize, usize> = selected_cols.iter().enumerate().map(|(new_c, &old_c)| (old_c, new_c)).collect();
    let mut out: Vec<CellExclusionNote> = notes
        .iter()
        .filter_map(|note| {
            if primary_only && note.coord.subcol != 0 {
                return None;
            }
            col_map.get(&note.coord.col).map(|&new_col| CellExclusionNote {
                coord: CellCoord::new(note.coord.row, new_col, note.coord.subcol),
                reason_tag: note.reason_tag.clone(),
                note: note.note.clone(),
                timestamp_utc: note.timestamp_utc.clone(),
            })
        })
        .collect();
    out.sort_by_key(|n| n.coord);
    out
}

fn remap_projected_column_notes(
    notes: &[CellExclusionNote],
    selected_cols: &[usize],
    shift: ProjectedAxisShift,
    target_archetype: TableKind,
    warnings: &mut Vec<String>,
) -> Vec<CellExclusionNote> {
    match shift {
        ProjectedAxisShift::None => remap_selected_column_notes(notes, selected_cols, true),
        ProjectedAxisShift::PrependExternalAxis => {
            let mut shifted = remap_selected_column_notes(notes, selected_cols, true);
            for note in &mut shifted {
                note.coord.col += 1;
            }
            shifted
        }
        ProjectedAxisShift::ConsumeFirstColumnAsAxis => {
            if let Some(&axis_col) = selected_cols.first()
                && notes.iter().any(|n| n.coord.col == axis_col && n.coord.subcol == 0)
            {
                warnings.push(format!(
                    "exclusion note(s) on column {axis_col} moved to {} axis could not be retained as cell exclusion notes",
                    target_archetype.label()
                ));
            }
            remap_selected_column_notes(notes, selected_cols.get(1..).unwrap_or(&[]), true)
        }
    }
}

fn archetype_column_titles(arch: &TableArchetype) -> Vec<&str> {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::Grouped(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::Contingency(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::Survival(t) => t.cohorts.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::PartsOfWhole(t) => t.data_sets.iter().map(|d| d.title.as_str()).collect(),
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.title.as_str()).collect(),
        TableArchetype::Nested(t) => t.groups.iter().map(|g| g.title.as_str()).collect(),
    }
}

fn resolve_simulated_title_index(titles: &[String], needle: &str) -> Option<usize> {
    let trimmed = needle.trim();
    if let Some(pos) = titles.iter().position(|t| t.trim() == trimmed) {
        return Some(pos);
    }
    if let Some(pos) = titles.iter().position(|t| t.trim().eq_ignore_ascii_case(trimmed)) {
        return Some(pos);
    }
    (0..titles.len()).find(|&i| {
        let lbl = default_column_label(i);
        if trimmed.eq_ignore_ascii_case("e") { lbl == trimmed } else { lbl.eq_ignore_ascii_case(trimmed) }
    })
}

fn predicate_depends_on_derived(rules: &[SelectRule], source_arch: &TableArchetype, parsed_derived: &[(String, Expr)]) -> bool {
    if parsed_derived.is_empty() {
        return false;
    }
    let mut sim_titles: Vec<String> = archetype_column_titles(source_arch).into_iter().map(str::to_owned).collect();
    let base_col_count = sim_titles.len();
    let mut derived_indices: HashSet<usize> = HashSet::new();
    for (d_title, _) in parsed_derived {
        let borrowed: Vec<&str> = sim_titles.iter().map(String::as_str).collect();
        if let Some(existing) = find_matching_column_title(&borrowed, d_title) {
            derived_indices.insert(existing);
        } else {
            let new_idx = sim_titles.len();
            sim_titles.push(d_title.clone());
            derived_indices.insert(new_idx);
        }
    }

    rules.iter().any(|rule| {
        let SelectRule::Predicate(formula) = rule else {
            return false;
        };
        parse_formula(formula).is_ok_and(|expr| {
            extract_dependencies(&expr).iter().any(|dep| match dep {
                DependencyTarget::ColumnIndex(idx) => derived_indices.contains(idx) || *idx >= base_col_count,
                DependencyTarget::ColumnName(name) => {
                    let trimmed = name.trim();
                    parsed_derived.iter().any(|(d_title, _)| d_title.eq_ignore_ascii_case(trimmed))
                        || resolve_simulated_title_index(&sim_titles, trimmed).is_some_and(|idx| derived_indices.contains(&idx))
                }
                _ => false,
            })
        })
    })
}

fn remap_selected_column_origins(origins: &[PlateCellOrigin], selected_cols: &[usize], primary_only: bool) -> Vec<PlateCellOrigin> {
    if origins.is_empty() {
        return Vec::new();
    }
    let col_map: HashMap<usize, usize> = selected_cols.iter().enumerate().map(|(new_c, &old_c)| (old_c, new_c)).collect();
    let mut out: Vec<PlateCellOrigin> = origins
        .iter()
        .filter_map(|origin| {
            if primary_only && origin.coord.subcol != 0 {
                return None;
            }
            col_map
                .get(&origin.coord.col)
                .map(|&new_col| PlateCellOrigin { coord: CellCoord::new(origin.coord.row, new_col, origin.coord.subcol), well: origin.well.clone() })
        })
        .collect();
    out.sort_by_key(|o| o.coord);
    out.dedup_by_key(|o| o.coord);
    out
}

fn remap_projected_column_origins(origins: &[PlateCellOrigin], selected_cols: &[usize], shift: ProjectedAxisShift) -> Vec<PlateCellOrigin> {
    match shift {
        ProjectedAxisShift::None => remap_selected_column_origins(origins, selected_cols, true),
        ProjectedAxisShift::PrependExternalAxis => {
            let mut shifted = remap_selected_column_origins(origins, selected_cols, true);
            for origin in &mut shifted {
                origin.coord.col += 1;
            }
            shifted
        }
        ProjectedAxisShift::ConsumeFirstColumnAsAxis => remap_selected_column_origins(origins, selected_cols.get(1..).unwrap_or(&[]), true),
    }
}

fn find_matching_column_title(titles: &[&str], target: &str) -> Option<usize> {
    titles.iter().position(|t| t.trim() == target).or_else(|| titles.iter().position(|t| t.trim().eq_ignore_ascii_case(target)))
}

fn validate_derived_column_growth(arch: &TableArchetype, title: &str, rows: usize) -> Result<(), TransformError> {
    let titles = archetype_column_titles(arch);
    if find_matching_column_title(&titles, title).is_some() {
        return Ok(());
    }
    validate_data_sets(titles.len().saturating_add(1))?;
    let added_subcols = match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count().max(1),
        TableArchetype::Nested(t) => t.groups.first().map_or(1, curvebook_doc::NestedGroupColumn::biological_unit_count).max(1),
        TableArchetype::MultipleVariables(_) | TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) => 1,
    };
    let overflow = || TransformError::Doc(DocError::TooManyCells { count: usize::MAX, max: MAX_SHEET_CELLS });
    let added_cells = rows.checked_mul(added_subcols).ok_or_else(overflow)?;
    let projected = arch.allocated_cell_slots().checked_add(added_cells).ok_or_else(overflow)?;
    validate_sheet_cells(projected)?;
    Ok(())
}

fn apply_derived_column(working: &mut DataSheet, title: &str, expr: &Expr, info_sheet: Option<&InfoSheet>) -> Result<usize, TransformError> {
    let rows = working.archetype.row_count();
    validate_derived_column_growth(&working.archetype, title, rows)?;
    let vals = eval_column_expr(expr, &TableEvalContext::new(working, info_sheet))?;
    let mut cells: Vec<Cell> = vals.into_iter().map(FormulaValue::into_cell).collect();
    cells.resize_with(rows, Cell::empty);

    match &mut working.archetype {
        TableArchetype::MultipleVariables(mv) => {
            let titles: Vec<&str> = mv.variables.iter().map(|v| v.title.as_str()).collect();
            if let Some(idx) = find_matching_column_title(&titles, title) {
                if let Some(var) = mv.variables.get_mut(idx) {
                    var.source = VariableSource::Entered;
                    var.cells = cells;
                    var.role = VariableRole::Continuous;
                    var.role_inferred = true;
                    var.refresh_inferred_role();
                }
                Ok(idx)
            } else {
                validate_data_sets(mv.variables.len() + 1)?;
                let mut var = VariableColumn::new(title.to_owned(), rows)?;
                var.cells = cells;
                var.refresh_inferred_role();
                mv.variables.push(var);
                Ok(mv.variables.len() - 1)
            }
        }
        TableArchetype::Xy(t) => apply_derived_to_datasets(&mut t.data_sets, t.y_subcolumn_mode.subcolumn_count(), title, &cells, rows),
        TableArchetype::Column(t) => apply_derived_to_datasets(&mut t.data_sets, t.subcolumn_mode.subcolumn_count(), title, &cells, rows),
        TableArchetype::Grouped(t) => {
            let old_len = t.data_sets.len();
            let idx = apply_derived_to_datasets(&mut t.data_sets, t.subcolumn_mode.subcolumn_count(), title, &cells, rows)?;
            if t.data_sets.len() != old_len {
                t.three_way_grouping = None;
            }
            Ok(idx)
        }
        TableArchetype::Contingency(t) => {
            let idx = apply_derived_to_datasets(&mut t.data_sets, 1, title, &cells, rows)?;
            t.validate_counts()?;
            Ok(idx)
        }
        TableArchetype::Survival(t) => apply_derived_to_datasets(&mut t.cohorts, 1, title, &cells, rows),
        TableArchetype::PartsOfWhole(t) => apply_derived_to_datasets(&mut t.data_sets, 1, title, &cells, rows),
        TableArchetype::Nested(t) => {
            let titles: Vec<&str> = t.groups.iter().map(|g| g.title.as_str()).collect();
            if let Some(idx) = find_matching_column_title(&titles, title) {
                if let Some(grp) = t.groups.get_mut(idx) {
                    for (r, cell) in cells.iter().enumerate() {
                        if let Some(row_cells) = grp.cells.get_mut(r) {
                            for sub in row_cells.iter_mut() {
                                *sub = Cell::empty();
                            }
                            if let Some(first) = row_cells.first_mut() {
                                *first = cell.clone();
                            }
                        }
                    }
                }
                Ok(idx)
            } else {
                validate_data_sets(t.groups.len() + 1)?;
                let bio = t.groups.first().map_or(1, curvebook_doc::NestedGroupColumn::biological_unit_count).max(1);
                let mut grp = curvebook_doc::NestedGroupColumn::new(title.to_owned(), bio, rows)?;
                for (r, cell) in cells.iter().enumerate() {
                    if let Some(slot) = grp.cells.get_mut(r).and_then(|row_cells| row_cells.first_mut()) {
                        *slot = cell.clone();
                    }
                }
                t.groups.push(grp);
                Ok(t.groups.len() - 1)
            }
        }
    }
}

fn apply_derived_to_datasets(
    data_sets: &mut Vec<DataSetColumn>,
    subcols: usize,
    title: &str,
    cells: &[Cell],
    rows: usize,
) -> Result<usize, TransformError> {
    let subs = subcols.max(1);
    let titles: Vec<&str> = data_sets.iter().map(|d| d.title.as_str()).collect();
    if let Some(idx) = find_matching_column_title(&titles, title) {
        if let Some(ds) = data_sets.get_mut(idx) {
            for (r, cell) in cells.iter().enumerate() {
                if let Some(row_cells) = ds.cells.get_mut(r) {
                    for sub in row_cells.iter_mut() {
                        *sub = Cell::empty();
                    }
                    if let Some(first) = row_cells.first_mut() {
                        *first = cell.clone();
                    }
                }
            }
        }
        Ok(idx)
    } else {
        validate_data_sets(data_sets.len() + 1)?;
        let mut ds = DataSetColumn::new(title.to_owned(), rows, subs)?;
        for (r, cell) in cells.iter().enumerate() {
            if let Some(slot) = ds.cells.get_mut(r).and_then(|row_cells| row_cells.first_mut()) {
                *slot = cell.clone();
            }
        }
        data_sets.push(ds);
        Ok(data_sets.len() - 1)
    }
}

fn apply_select_rule(
    source: &DataSheet,
    rule: &SelectRule,
    base_selected_cols: &[usize],
    mask: &mut [bool],
    ctx: &TransformContext<'_>,
) -> Result<(), TransformError> {
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
            let eval_ctx = TableEvalContext::new(source, ctx.primary_info_sheet());
            let vals = eval_column_expr(&expr, &eval_ctx)?;
            for (r, flag) in mask.iter_mut().enumerate() {
                if vals.get(r).and_then(|v| v.as_bool()) != Some(true) {
                    *flag = false;
                }
            }
        }
        SelectRule::ExcludeMissing => {
            for (r, flag) in mask.iter_mut().enumerate() {
                if !*flag {
                    continue;
                }
                let row_complete = base_selected_cols.iter().all(|&c| row_col_has_active(&source.archetype, r, c));
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
            let mut candidates: Vec<(usize, f64)> = (0..n)
                .filter(|&r| mask.get(r).copied().unwrap_or(false))
                .filter_map(|r| row_col_active_number(&source.archetype, r, *column).map(|v| (r, v)))
                .collect();
            candidates.sort_by(|a, b| if is_top { b.1.total_cmp(&a.1) } else { a.1.total_cmp(&b.1) });
            let mut chosen = vec![false; n];
            for (r, _) in candidates.into_iter().take(*top_k) {
                if let Some(slot) = chosen.get_mut(r) {
                    *slot = true;
                }
            }
            for (r, flag) in mask.iter_mut().enumerate() {
                *flag = *flag && chosen.get(r).copied().unwrap_or(false);
            }
        }
    }
    Ok(())
}

fn row_col_has_active(arch: &TableArchetype, row: usize, col: usize) -> bool {
    match arch {
        TableArchetype::Xy(t) => dataset_row_has_active(&t.data_sets, col, row, t.y_subcolumn_mode),
        TableArchetype::Column(t) => dataset_row_has_active(&t.data_sets, col, row, t.subcolumn_mode),
        TableArchetype::Grouped(t) => dataset_row_has_active(&t.data_sets, col, row, t.subcolumn_mode),
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
        TableArchetype::Xy(t) => dataset_row_active_scalar(&t.data_sets, col, row, t.y_subcolumn_mode),
        TableArchetype::Column(t) => dataset_row_active_scalar(&t.data_sets, col, row, t.subcolumn_mode),
        TableArchetype::Grouped(t) => dataset_row_active_scalar(&t.data_sets, col, row, t.subcolumn_mode),
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
        TableArchetype::Grouped(t) => {
            let full_keep = selected.len() == t.data_sets.len() && selected.iter().enumerate().all(|(i, &c)| i == c);
            t.data_sets = retain_col_vec(&t.data_sets, selected);
            if !full_keep {
                t.three_way_grouping = None;
            }
        }
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
                    if !full_keep && (matches!(var.source, VariableSource::Calculated { .. }) || var.role == VariableRole::Calculated) {
                        // Subsetting/reordering variables can drop or re-index formula inputs; snapshot evaluated cells.
                        var.source = VariableSource::Entered;
                        var.role = VariableRole::Continuous;
                        var.role_inferred = true;
                        var.refresh_inferred_role();
                    }
                    Some(var)
                })
                .collect();
        }
        TableArchetype::Nested(t) => t.groups = retain_col_vec(&t.groups, selected),
    }
    Ok(())
}

fn project_archetype(arch: &TableArchetype, target: TableKind, selected: &[usize], include_axis: bool) -> Result<TableArchetype, TransformError> {
    let rows = arch.row_count();
    let (titles, row_titles, grid) = flatten_primary_columns(arch, selected, include_axis);
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
        TableKind::Xy => project_into_xy(rows, &titles, row_titles, &grid),
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
        TableKind::Survival => project_into_survival(rows, &titles, row_titles, &grid),
        TableKind::MultipleVariables => project_into_mv(rows, &titles, row_titles, &grid),
        TableKind::Nested => project_into_nested(rows, &titles, row_titles, &grid),
    }
}

fn project_into_xy(rows: usize, titles: &[String], row_titles: Vec<String>, grid: &[Vec<Cell>]) -> Result<TableArchetype, TransformError> {
    let cols = titles.len();
    let y_cols = cols.saturating_sub(1).max(1);
    let mut t = XyTable::new(SubcolumnMode::Single, y_cols, rows)?;
    if cols <= 1 {
        for r in 0..rows {
            #[allow(clippy::cast_precision_loss)]
            t.set_x_cell(r, Cell::number((r + 1) as f64))?;
            if let Some(cell) = grid.get(r).and_then(|row| row.first())
                && let Some(slot) = t.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut())
            {
                *slot = cell.clone();
            }
        }
        if let Some(first_title) = titles.first()
            && let Some(ds0) = t.data_sets.first_mut()
        {
            ds0.title = first_title.clone();
        }
    } else {
        if let Some(x_title) = titles.first() {
            t.x_title = x_title.clone();
        }
        for (r, row_vals) in grid.iter().enumerate().take(rows) {
            if let Some(cell) = row_vals.first() {
                t.set_x_cell(r, cell.clone())?;
            }
        }
        for (y_idx, ds) in t.data_sets.iter_mut().enumerate() {
            if let Some(title) = titles.get(y_idx + 1) {
                ds.title = title.clone();
            }
            for (r, row_vals) in grid.iter().enumerate().take(rows) {
                if let Some(cell) = row_vals.get(y_idx + 1)
                    && let Some(slot) = ds.cells.get_mut(r).and_then(|row| row.first_mut())
                {
                    *slot = cell.clone();
                }
            }
        }
    }
    t.row_titles_expanded = row_titles.iter().any(|s| !s.trim().is_empty());
    t.row_titles = row_titles;
    Ok(TableArchetype::Xy(t))
}

fn project_into_survival(rows: usize, titles: &[String], row_titles: Vec<String>, grid: &[Vec<Cell>]) -> Result<TableArchetype, TransformError> {
    let cols = titles.len();
    let cohort_cols = cols.saturating_sub(1).max(1);
    let mut t = SurvivalTable::new(cohort_cols, rows)?;
    if cols >= 2 {
        if let Some(time_title) = titles.first() {
            t.time_title = time_title.clone();
        }
        for (r, row_vals) in grid.iter().enumerate().take(rows) {
            if let Some(cell) = row_vals.first()
                && let Some(slot) = t.times.get_mut(r)
            {
                *slot = cell.clone();
            }
            for (c, ds) in t.cohorts.iter_mut().enumerate() {
                if let Some(title) = titles.get(c + 1) {
                    ds.title = title.clone();
                }
                if let Some(cell) = row_vals.get(c + 1)
                    && let Some(slot) = ds.cells.get_mut(r).and_then(|row| row.first_mut())
                {
                    *slot = cell.clone();
                }
            }
        }
    } else if cols == 1 {
        if let Some(title) = titles.first()
            && let Some(ds0) = t.cohorts.first_mut()
        {
            ds0.title = title.clone();
        }
        for (r, row_vals) in grid.iter().enumerate().take(rows) {
            if let Some(cell) = row_vals.first()
                && let Some(slot) = t.cohorts.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut())
            {
                *slot = cell.clone();
            }
        }
    }
    t.row_titles = row_titles;
    Ok(TableArchetype::Survival(t))
}

fn project_into_mv(rows: usize, titles: &[String], row_titles: Vec<String>, grid: &[Vec<Cell>]) -> Result<TableArchetype, TransformError> {
    let cols = titles.len();
    let mut t = MultipleVariablesTable::new(cols.max(1), rows)?;
    if cols == 0 {
        t.variables.clear();
    } else {
        for (c, var) in t.variables.iter_mut().enumerate() {
            if let Some(title) = titles.get(c) {
                var.title = title.clone();
            }
            var.role = VariableRole::Continuous;
            for (r, row_vals) in grid.iter().enumerate().take(rows) {
                if let Some(cell) = row_vals.get(c)
                    && let Some(slot) = var.cells.get_mut(r)
                {
                    *slot = cell.clone();
                }
            }
            var.refresh_inferred_role();
        }
    }
    t.row_titles_expanded = row_titles.iter().any(|s| !s.trim().is_empty());
    t.row_titles = row_titles;
    Ok(TableArchetype::MultipleVariables(t))
}

fn project_into_nested(rows: usize, titles: &[String], row_titles: Vec<String>, grid: &[Vec<Cell>]) -> Result<TableArchetype, TransformError> {
    let cols = titles.len();
    let mut t = NestedTable::new(cols.max(1), 1, rows)?;
    if cols == 0 {
        t.groups.clear();
    } else {
        for (c, grp) in t.groups.iter_mut().enumerate() {
            if let Some(title) = titles.get(c) {
                grp.title = title.clone();
            }
            grp.subcolumn_titles = vec![format!("{}:1", grp.title)];
            for (r, row_vals) in grid.iter().enumerate().take(rows) {
                if let Some(cell) = row_vals.get(c)
                    && let Some(slot) = grp.cells.get_mut(r).and_then(|row| row.first_mut())
                {
                    *slot = cell.clone();
                }
            }
        }
    }
    t.row_titles = row_titles;
    Ok(TableArchetype::Nested(t))
}

fn populate_single_datasets(data_sets: &mut [DataSetColumn], titles: &[String], grid: &[Vec<Cell>]) {
    for (c, ds) in data_sets.iter_mut().enumerate() {
        if let Some(title) = titles.get(c) {
            ds.title = title.clone();
        }
        for (r, row) in grid.iter().enumerate() {
            if let Some(cell) = row.get(c)
                && let Some(slot) = ds.cells.get_mut(r).and_then(|cells_row| cells_row.first_mut())
            {
                *slot = cell.clone();
            }
        }
    }
}

fn flatten_primary_columns(arch: &TableArchetype, selected: &[usize], include_axis: bool) -> (Vec<String>, Vec<String>, Vec<Vec<Cell>>) {
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
    let axis_title = if include_axis {
        match arch {
            TableArchetype::Xy(t) => Some(if t.x_title.trim().is_empty() { String::from("X") } else { t.x_title.clone() }),
            TableArchetype::Survival(t) => Some(if t.time_title.trim().is_empty() { String::from("Time") } else { t.time_title.clone() }),
            _ => None,
        }
    } else {
        None
    };
    let mut titles = Vec::with_capacity(selected.len() + usize::from(axis_title.is_some()));
    if let Some(ref label) = axis_title {
        titles.push(label.clone());
    }
    titles.extend(selected.iter().filter_map(|&c| all_titles.get(c).cloned()));
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
        let mut row = Vec::with_capacity(selected.len() + usize::from(axis_title.is_some()));
        if axis_title.is_some() {
            let axis_cell = match arch {
                TableArchetype::Xy(t) => t.materialized_x_cell_at(r),
                TableArchetype::Survival(t) => t.times.get(r).cloned().unwrap_or_default(),
                _ => Cell::empty(),
            };
            row.push(axis_cell);
        }
        for &c in selected {
            let cell = arch.data_cell(CellCoord::new(r, c, 0)).cloned().unwrap_or_default();
            row.push(cell);
        }
        grid.push(row);
    }
    (titles, row_titles, grid)
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, ColumnDesignHint, ColumnTable, DataSheet, DerivedColumnFormula, DocError, GroupedTable, MAX_SHEET_CELLS, MAX_TITLE_BYTES,
        MultipleVariablesTable, SequenceProgression, SubcolumnMode, TableArchetype, TableKind, ThreeWayColumnGrouping, VariableRole, VariableSource,
        XyTable,
    };

    use super::{PruneMethod, SelectRule, execute_prune, execute_select, execute_select_with_derived, execute_transpose, try_retain_rows};
    use crate::error::TransformError;
    use crate::formula::evaluate_calculated_variables;
    use crate::ops::{TransformContext, TransformOutput};
    use crate::test_helpers as th;

    fn prune(s: &DataSheet, m: PruneMethod) -> TransformOutput {
        execute_prune(s, m, &TransformContext::default()).unwrap()
    }

    fn sel(s: &DataSheet, rules: &[SelectRule], kind: TableKind) -> DataSheet {
        execute_select(s, rules, kind, &TransformContext::default()).unwrap().data_sheet
    }

    fn sel_d(s: &DataSheet, rules: &[SelectRule], derived: &[DerivedColumnFormula], kind: TableKind) -> Result<DataSheet, TransformError> {
        execute_select_with_derived(s, rules, derived, kind, &TransformContext::default()).map(|o| o.data_sheet)
    }

    #[test]
    fn aud04_03_transpose_bounds_checks_and_checked_mul_exclusion_notes() {
        let row = vec![Cell::excluded_number(10.0), Cell::number(20.0)];
        let mut sheet = th::xy_sheet("tr-oob", SubcolumnMode::RawReplicates(2), &[1.0, 2.0], &[vec![row; 2]]);
        sheet.exclusion_notes.push(th::excl_note(0, (usize::MAX / 2) + 1, 0, "spoof", "overflowing col"));
        let out = execute_transpose(&sheet, true, &TransformContext::default()).unwrap();
        assert!(out.data_sheet.exclusion_notes.is_empty(), "spoofed OOB note must be dropped, got {:?}", out.data_sheet.exclusion_notes);
    }

    #[test]
    fn aud04_14_prune_and_average_bins_snapshot_calculated_mv_variables() {
        let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
        (mv.variables[0].title, mv.variables[1].title) = (String::from("X"), String::from("Doubled"));
        mv.variables[0].cells = th::nums(&[1.0, 2.0, 3.0, 4.0]);
        mv.variables[1].set_calculated_formula("=X * 2");
        evaluate_calculated_variables(&mut mv, None).unwrap();
        let sheet = DataSheet::new("mv-prune", "MV Prune", TableArchetype::MultipleVariables(mv)).unwrap();
        for method in [PruneMethod::AverageBins(2), PruneMethod::EveryNth(2), PruneMethod::MinMaxBins(2)] {
            let v1 = &th::into_mv(prune(&sheet, method).data_sheet).variables[1];
            assert_eq!((v1.source.clone(), v1.role), (VariableSource::Entered, VariableRole::Continuous), "{method:?}");
        }
    }

    #[test]
    fn aud04_23_select_derived_column_enforces_cumulative_cell_budget_before_alloc() {
        let xy = XyTable::new(SubcolumnMode::RawReplicates(500), 1, 20_000).unwrap();
        let sheet = DataSheet::new("big-xy", "Big XY", TableArchetype::Xy(xy)).unwrap();
        let derived = [DerivedColumnFormula::new("D1", "1"), DerivedColumnFormula::new("D2", "2"), DerivedColumnFormula::new("D3", "3")];
        let err = sel_d(&sheet, &[], &derived, TableKind::Xy).unwrap_err();
        assert!(matches!(err, TransformError::Doc(DocError::TooManyCells { max: MAX_SHEET_CELLS, .. })), "unexpected err: {err:?}");
    }

    #[test]
    fn aud04_30_average_bins_overflow_safe_means_and_parametric_pooling_no_orphan_n() {
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 1, 2).unwrap();
        (grp.row_numeric_spacing, grp.data_sets[0].cells) = (Some(vec![Some(1e308), Some(1e308)]), vec![vec![Cell::number(1e308)]; 2]);
        let binned = prune(&DataSheet::new("grp-big", "Grp Big", TableArchetype::Grouped(grp)).unwrap(), PruneMethod::AverageBins(2));
        let b_grp = th::as_grp(&binned.data_sheet);
        assert_eq!((&b_grp.row_numeric_spacing, b_grp.data_sets[0].cells[0][0].active_number()), (&Some(vec![Some(1e308)]), Some(1e308)));
        assert!(binned.data_sheet.validate_finite_metadata().is_ok());

        let c0 = vec![th::nums(&[1e308, 1.0, 2.0]), th::nums(&[1e308, 1.0, 2.0])];
        let c1 = vec![th::nums(&[-1e160, 1.0, 2.0]), th::nums(&[1e160, 1.0, 2.0])];
        let p_xy =
            th::into_xy(prune(&th::xy_sheet("pool-big", SubcolumnMode::MeanSdN, &[1.0, 2.0], &[c0, c1]), PruneMethod::AverageBins(2)).data_sheet);
        assert_eq!((p_xy.data_sets[0].cells[0][0].active_number(), p_xy.data_sets[0].cells[0][2].active_number()), (Some(1e308), Some(4.0)));
        th::assert_close(th::num(&p_xy.data_sets[0].cells[0][1]), (2.0_f64 / 3.0).sqrt(), 1e-12);
        let exp_sd = 1e160 * (4.0_f64 / 3.0).sqrt();
        th::assert_close((th::num(&p_xy.data_sets[1].cells[0][1]) - exp_sd) / exp_sd, 0.0, 1e-12);

        let ovf_c = vec![th::nums(&[-1.7e308, 1.0, 2.0]), th::nums(&[1.7e308, 1.0, 2.0])];
        let ovf_out = prune(&th::xy_sheet("pool-ovf", SubcolumnMode::MeanSdN, &[1.0, 2.0], &[ovf_c]), PruneMethod::AverageBins(2));
        let row0 = &th::as_xy(&ovf_out.data_sheet).data_sets[0].cells[0];
        assert!(row0.iter().all(Cell::is_empty) && !ovf_out.warnings.is_empty(), "must not emit orphaned N: {row0:?}");
    }

    #[test]
    fn aud04_32_presummarized_subcolumns_in_topn_exclude_missing_and_minmax_bins() {
        let mut grp = GroupedTable::new(SubcolumnMode::MeanSdN, 1, 3).unwrap();
        grp.data_sets[0].cells =
            vec![th::nums(&[10.0, 1.0, 100.0]), th::nums(&[50.0, 2.0, 3.0]), vec![Cell::empty(), Cell::empty(), Cell::number(500.0)]];
        let sheet = DataSheet::new("grp-sum", "Grp Sum", TableArchetype::Grouped(grp)).unwrap();

        let top_g = th::into_grp(sel(&sheet, &[SelectRule::TopN { column: 0, n: 1 }], TableKind::Grouped));
        assert_eq!((top_g.row_count(), top_g.data_sets[0].cells[0][0].active_number()), (1, Some(50.0)));
        assert_eq!(sel(&sheet, &[SelectRule::ExcludeMissing], TableKind::Grouped).archetype.row_count(), 2);

        let mm_g = th::into_grp(prune(&sheet, PruneMethod::MinMaxBins(3)).data_sheet);
        let c0 = &mm_g.data_sets[0].cells;
        assert_eq!((mm_g.row_count(), c0[0][0].active_number(), c0[1][0].active_number()), (2, Some(10.0), Some(50.0)));

        let xy_s = th::xy_sheet("xy-rep", SubcolumnMode::RawReplicates(2), &[1.0, 2.0], &[vec![th::nums(&[10.0, 10.0]), th::nums(&[5.0, 95.0])]]);
        assert_eq!(th::into_xy(sel(&xy_s, &[SelectRule::TopN { column: 0, n: 1 }], TableKind::Xy)).x_value_at(0), Some(2.0));
    }

    #[test]
    fn aud04_39_log_spaced_prune_binary_search_exact_and_fast() {
        let n = 10_000_usize;
        let xs: Vec<f64> = (1..=n).map(|r| r as f64).collect();
        let ys: Vec<Vec<Cell>> = (0..n).map(|r| vec![Cell::number(r as f64)]).collect();
        let sheet = th::xy_sheet("log-fast", SubcolumnMode::Single, &xs, &[ys]);
        let out_xy = th::into_xy(prune(&sheet, PruneMethod::LogarithmicSpacing { target_points: 5 }).data_sheet);
        let got: Vec<f64> = (0..out_xy.row_count()).map(|r| out_xy.x_value_at(r).unwrap()).collect();
        assert_eq!(got, vec![1.0, 10.0, 100.0, 1000.0, 10000.0]);
    }

    #[test]
    fn aud04_45_prune_validates_output_sheet_and_guards_generated_sequence_cells() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        col.data_sets[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        let sheet = DataSheet::new("pr-long", "P".repeat(MAX_TITLE_BYTES), TableArchetype::Column(col)).unwrap();
        let pruned = prune(&sheet, PruneMethod::EveryNth(1));
        assert!(pruned.data_sheet.validate().is_ok() && pruned.data_sheet.title.len() <= MAX_TITLE_BYTES);

        let mut arch = TableArchetype::Xy(th::xy_gen_seq(0.0, 1.0, SequenceProgression::Arithmetic, SubcolumnMode::RawReplicates(300), 1, 100_000));
        let err = try_retain_rows(&mut arch, &(0..100_000).collect::<Vec<_>>()).unwrap_err();
        assert!(matches!(err, TransformError::Doc(DocError::TooManyCells { max: MAX_SHEET_CELLS, .. })), "unexpected err: {err:?}");
    }

    #[test]
    fn aud04_46_select_predicate_col_index_letter_alias_exclude_missing_and_three_way_clear() {
        let sheet = th::mv_sheet("sel-alias", &[("A", None, th::nums(&[1.0, 5.0, 10.0])), ("B", None, th::nums(&[2.0, 6.0, 20.0]))]);
        let derived = [DerivedColumnFormula::new("Score", "[A] + [B]")];
        for pred in ["col(2) > 10", "C > 10"] {
            let out = th::into_mv(sel_d(&sheet, &[SelectRule::Predicate(pred.to_owned())], &derived, TableKind::MultipleVariables).unwrap());
            let c2 = &out.variables[2].cells;
            assert_eq!((out.row_count(), c2[0].active_number(), c2[1].active_number()), (2, Some(11.0), Some(30.0)), "pred `{pred}`");
        }

        let rules = [SelectRule::ExcludeMissing, SelectRule::Predicate(String::from("isvalid(Ratio) OR [A] == 1"))];
        let derived_na = [DerivedColumnFormula::new("Ratio", "if([A] > 1, [B] / [A], nan)")];
        assert_eq!(sel_d(&sheet, &rules, &derived_na, TableKind::MultipleVariables).unwrap().archetype.row_count(), 3);

        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 4, 2).unwrap();
        grp.three_way_grouping = Some(ThreeWayColumnGrouping {
            factor_1_name: String::from("Drug"),
            factor_1_levels: [String::from("No"), String::from("Yes")],
            factor_2_name: String::from("Sex"),
            factor_2_levels: [String::from("F"), String::from("M")],
        });
        let grp_sheet = DataSheet::new("grp-3w", "3-Way", TableArchetype::Grouped(grp)).unwrap();
        let sub_ctx = TransformContext { selected_data_sets: vec![0, 1], ..TransformContext::default() };
        assert!(th::into_grp(execute_select(&grp_sheet, &[], TableKind::Grouped, &sub_ctx).unwrap().data_sheet).three_way_grouping.is_none());
        let extra = [DerivedColumnFormula::new("Extra", "1")];
        assert!(th::into_grp(sel_d(&grp_sheet, &[], &extra, TableKind::Grouped).unwrap()).three_way_grouping.is_none());
    }

    #[test]
    fn aud04_51_transpose_all_archetypes_direct_construction_preserves_data() {
        let mut grp = GroupedTable::new(SubcolumnMode::MeanSdN, 2, 3).unwrap();
        grp.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
        (grp.data_sets[0].title, grp.data_sets[1].title) = (String::from("C0"), String::from("C1"));
        grp.data_sets[1].cells[2] = th::nums(&[42.0, 3.5, 6.0]);
        let sheet = DataSheet::new("tr-grp", "Grp", TableArchetype::Grouped(grp)).unwrap();
        let tr = th::into_grp(execute_transpose(&sheet, true, &TransformContext::default()).unwrap().data_sheet);
        let vals: Vec<_> = tr.data_sets[2].cells[1].iter().map(Cell::active_number).collect();
        assert_eq!((tr.data_sets.len(), tr.row_count(), vals), (3, 2, vec![Some(42.0), Some(3.5), Some(6.0)]));
    }
}
