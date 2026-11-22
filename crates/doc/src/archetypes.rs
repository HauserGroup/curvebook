//! The eight typed scientific data-table archetypes (`XyTable`, `ColumnTable`, `GroupedTable`,
//! `ContingencyTable`, `SurvivalTable`, `PartsOfWholeTable`, `MultipleVariablesTable`,
//! `NestedTable`) and their unified [`TableArchetype`] sum type (`docs/implementation/data-tables-and-transforms.md` §2).

use std::cmp::Ordering;

use crate::TableKind;
use crate::cell::{Cell, CellValue, ReplicateStacking, SubcolumnMode, SummarizedPoint};
use crate::error::{
    DocError, MAX_SUBCOLUMNS, SummaryPointError, validate_cell_grid, validate_data_sets, validate_rows, validate_sheet_cells, validate_subcolumns,
    validate_title,
};

/// Zero-based 3D cell address `(row, col, subcol)` inside a data table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct CellCoord {
    /// Zero-based row index (`0..row_count`).
    pub row: usize,
    /// Zero-based data-set, cohort, variable, or group column index (`0..column_count`).
    pub col: usize,
    /// Zero-based replicate, summary, or biological-unit subcolumn index (`0..subcolumn_count`).
    pub subcol: usize,
}

impl CellCoord {
    /// Construct a zero-based `(row, col, subcol)` cell coordinate.
    #[must_use]
    pub const fn new(row: usize, col: usize, subcol: usize) -> Self {
        Self { row, col, subcol }
    }
}

/// Default spreadsheet-style column title (`A`, `B`, ..., `Z`, `AA`, `AB`, ...).
#[must_use]
pub fn default_column_label(mut index: usize) -> String {
    let mut chars = Vec::new();
    loop {
        let rem = (index % 26) as u8;
        chars.push(char::from(b'A' + rem));
        if index < 26 {
            break;
        }
        index = (index / 26) - 1;
    }
    chars.reverse();
    chars.into_iter().collect()
}

/// Named data-set column (`A`, `B`, ...) storing `[row][subcolumn]` cells.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DataSetColumn {
    /// Column header title (`<= MAX_TITLE_BYTES` UTF-8 bytes; numeric titles can also supply hooked concentrations via [`crate::HookSource::ColumnTitleValue`]).
    pub title: String,
    /// Rectangular 2D grid indexed as `cells[row][subcol]`.
    pub cells: Vec<Vec<Cell>>,
}

impl DataSetColumn {
    /// Allocate an empty `DataSetColumn` validated against dimension and title caps.
    pub fn new(title: impl Into<String>, row_count: usize, subcolumn_count: usize) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        validate_rows(row_count)?;
        validate_subcolumns(subcolumn_count)?;
        validate_cell_grid(row_count, 1, subcolumn_count, 0)?;
        Ok(Self { title, cells: vec![vec![Cell::empty(); subcolumn_count]; row_count] })
    }

    /// Number of allocated rows in this data-set column.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.cells.len()
    }

    /// Number of subcolumns per row (`0` when `row_count == 0`).
    #[must_use]
    pub fn subcolumn_count(&self) -> usize {
        self.cells.first().map_or(0, Vec::len)
    }

    /// Borrow the cell at `(row, subcol)`, returning `None` if out of bounds.
    #[must_use]
    pub fn cell(&self, row: usize, subcol: usize) -> Option<&Cell> {
        self.cells.get(row).and_then(|r| r.get(subcol))
    }

    /// Mutably borrow the cell at `(row, subcol)`, returning `None` if out of bounds.
    pub fn cell_mut(&mut self, row: usize, subcol: usize) -> Option<&mut Cell> {
        self.cells.get_mut(row).and_then(|r| r.get_mut(subcol))
    }

    /// Grow row count to at least `min_rows` using `subcolumn_count` empty cells per added row.
    pub fn ensure_rows(&mut self, min_rows: usize, subcolumn_count: usize) -> Result<(), DocError> {
        validate_rows(min_rows)?;
        validate_subcolumns(subcolumn_count)?;
        validate_cell_grid(min_rows, 1, subcolumn_count, 0)?;
        if self.cells.len() < min_rows {
            self.cells.resize_with(min_rows, || vec![Cell::empty(); subcolumn_count]);
        }
        Ok(())
    }
}

// ============================================================================
// 1. XY Table
// ============================================================================

/// Progression rule for generated X sequences (`X0`, `Delta X`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SequenceProgression {
    /// `X_r = start + r * step`.
    #[default]
    Arithmetic,
    /// `X_r = start * step^r`.
    Geometric,
}

/// Check that `titles` is either empty (omitted/legacy) or matches `row_count` and every string is within `MAX_TITLE_BYTES`.
fn validate_title_vec(kind: TableKind, titles: &[String], row_count: usize, mismatch_reason: &'static str) -> Result<(), DocError> {
    if !titles.is_empty() && titles.len() != row_count {
        return Err(DocError::InvalidCellValue { archetype: kind, row: titles.len(), col: 0, reason: mismatch_reason });
    }
    for title in titles {
        validate_title(title)?;
    }
    Ok(())
}

/// Check that every [`DataSetColumn`] has rectangular dimensions `(row_count × expected_subcols)`
/// and valid titles and subcolumn bounds.
fn validate_dataset_column_grid(kind: TableKind, columns: &[DataSetColumn], row_count: usize, expected_subcols: usize) -> Result<(), DocError> {
    validate_data_sets(columns.len())?;
    validate_subcolumns(expected_subcols)?;
    for (col_idx, col) in columns.iter().enumerate() {
        validate_title(&col.title)?;
        validate_rows(col.cells.len())?;
        if col.cells.len() != row_count {
            return Err(DocError::InvalidCellValue {
                archetype: kind,
                row: col.cells.len(),
                col: col_idx,
                reason: "column row count does not match table row count",
            });
        }
        for (row_idx, row) in col.cells.iter().enumerate() {
            if row.len() > MAX_SUBCOLUMNS {
                return Err(DocError::TooManySubcolumns { count: row.len(), max: MAX_SUBCOLUMNS });
            }
            if row.len() != expected_subcols {
                return Err(DocError::InvalidCellValue {
                    archetype: kind,
                    row: row_idx,
                    col: col_idx,
                    reason: "row subcolumn count does not match table subcolumn mode",
                });
            }
        }
    }
    Ok(())
}

/// Validate a primary `SubcolumnMode` against `kind` and subcolumn count caps.
fn validate_table_subcolumn_mode(kind: TableKind, mode: SubcolumnMode) -> Result<(), DocError> {
    mode.validate()?;
    if !kind.allows_subcolumn_mode(mode) {
        return Err(DocError::IncompatibleSubcolumnMode { archetype: kind, mode });
    }
    Ok(())
}

/// Independent X-column configuration for an [`XyTable`] (`docs/implementation/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum XColumnMode {
    /// Explicit numeric X coordinates with optional error subcolumns (`Single`, `MeanSdN`, etc.).
    Numbers {
        /// Subcolumn layout of `XyTable::x_cells` (commonly [`SubcolumnMode::Single`], or `MeanSdN`/`MeanSemN` when *X* carries horizontal error bars).
        subcolumn_mode: SubcolumnMode,
    },
    /// Elapsed time or date coordinates accompanied by a unit label.
    DateOrElapsedTime {
        /// Elapsed-time or calendar unit suffix displayed in headers and axis labels (for example `"min"`, `"h"`, or `"days"`).
        unit_label: String,
    },
    /// Automatically generated arithmetic or geometric series (`X0`, `Delta X`).
    GeneratedSequence {
        /// Initial coordinate `X_0` at `row == 0`.
        start: f64,
        /// Additive increment (`X_r = start + r * step` for [`SequenceProgression::Arithmetic`]) or multiplicative ratio (`X_r = start * step^r` for [`SequenceProgression::Geometric`]).
        step: f64,
        /// Arithmetic vs geometric recurrence rule.
        progression: SequenceProgression,
    },
}

#[derive(serde::Deserialize)]
enum RawXColumnMode {
    Numbers { subcolumn_mode: SubcolumnMode },
    DateOrElapsedTime { unit_label: String },
    GeneratedSequence { start: f64, step: f64, progression: SequenceProgression },
}

impl<'de> serde::Deserialize<'de> for XColumnMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawXColumnMode::deserialize(deserializer)?;
        let mode = match raw {
            RawXColumnMode::Numbers { subcolumn_mode } => Self::Numbers { subcolumn_mode },
            RawXColumnMode::DateOrElapsedTime { unit_label } => Self::DateOrElapsedTime { unit_label },
            RawXColumnMode::GeneratedSequence { start, step, progression } => Self::GeneratedSequence { start, step, progression },
        };
        mode.validate().map_err(serde::de::Error::custom)?;
        Ok(mode)
    }
}

impl Default for XColumnMode {
    fn default() -> Self {
        Self::Numbers { subcolumn_mode: SubcolumnMode::Single }
    }
}

impl XColumnMode {
    /// Number of stored `x_cells` subcolumns per row (`0` for [`Self::GeneratedSequence`], `1` for [`Self::DateOrElapsedTime`], or `subcolumn_mode.subcolumn_count()` for [`Self::Numbers`]).
    #[must_use]
    pub const fn subcolumn_count(&self) -> usize {
        match self {
            Self::Numbers { subcolumn_mode } => subcolumn_mode.subcolumn_count(),
            Self::DateOrElapsedTime { .. } => 1,
            Self::GeneratedSequence { .. } => 0,
        }
    }

    /// Validate X-column mode constraints (`MAX_SUBCOLUMNS`, non-zero raw replicates, unit label length)
    /// without checking numeric sequence finiteness (used by `XyTable::validate` so higher-level
    /// non-finite float sweeps can report `NonFiniteFloat` directly).
    pub fn validate_subcolumn_only(&self) -> Result<(), DocError> {
        match self {
            Self::Numbers { subcolumn_mode } => subcolumn_mode.validate(),
            Self::DateOrElapsedTime { unit_label } => validate_title(unit_label),
            Self::GeneratedSequence { .. } => Ok(()),
        }
    }

    /// Validate X-column mode constraints (`MAX_SUBCOLUMNS`, non-zero raw replicates, unit label length,
    /// and finite `GeneratedSequence` `start`/`step`).
    pub fn validate(&self) -> Result<(), DocError> {
        self.validate_subcolumn_only()?;
        if let Self::GeneratedSequence { start, step, .. } = self
            && (!start.is_finite() || !step.is_finite())
        {
            return Err(DocError::InvalidCellValue { archetype: TableKind::Xy, row: 0, col: 0, reason: "x_mode.generated_sequence" });
        }
        Ok(())
    }
}

/// Archetype 1: Continuous independent X variable paired with one or more Y response series.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct XyTable {
    /// Display title of the independent *X* column (default `"X"`).
    pub x_title: String,
    /// Storage and evaluation mode of the independent *X* column.
    pub x_mode: XColumnMode,
    /// Per-row *X* cells (`x_cells[row][subcol]`; width matches [`XColumnMode::subcolumn_count`], or optional `0..=1` exclusion-carrier cells under `GeneratedSequence`).
    pub x_cells: Vec<Vec<Cell>>,
    /// Optional per-row stub labels (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Whether the `Row Titles` stub column is visibly expanded in the spreadsheet UI.
    pub row_titles_expanded: bool,
    /// Replicate or pre-summarized subcolumn layout shared across all *Y* [`Self::data_sets`].
    pub y_subcolumn_mode: SubcolumnMode,
    /// Ordered *Y* response series columns (`A`, `B`, ...).
    pub data_sets: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawXyTable {
    x_title: String,
    x_mode: XColumnMode,
    x_cells: Vec<Vec<Cell>>,
    row_titles: Vec<String>,
    row_titles_expanded: bool,
    y_subcolumn_mode: SubcolumnMode,
    data_sets: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for XyTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawXyTable::deserialize(deserializer)?;
        let table = Self {
            x_title: raw.x_title,
            x_mode: raw.x_mode,
            x_cells: raw.x_cells,
            row_titles: raw.row_titles,
            row_titles_expanded: raw.row_titles_expanded,
            y_subcolumn_mode: raw.y_subcolumn_mode,
            data_sets: raw.data_sets,
        };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl XyTable {
    /// Construct an `XyTable` with default single-column numeric *X* (`XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }`).
    pub fn new(y_subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        Self::new_with_x_mode(XColumnMode::default(), y_subcolumn_mode, data_set_count, row_count)
    }

    /// Construct an `XyTable` with explicit X-column mode (including optional X error subcolumns).
    pub fn new_with_x_mode(x_mode: XColumnMode, y_subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        x_mode.validate()?;
        validate_table_subcolumn_mode(TableKind::Xy, y_subcolumn_mode)?;
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let x_subs = x_mode.subcolumn_count();
        let y_subs = y_subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, data_set_count, y_subs, x_subs)?;
        let mut data_sets = Vec::with_capacity(data_set_count);
        for idx in 0..data_set_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, y_subs)?);
        }
        Ok(Self {
            x_title: String::from("X"),
            x_mode,
            x_cells: vec![vec![Cell::empty(); x_subs]; row_count],
            row_titles: vec![String::new(); row_count],
            row_titles_expanded: false,
            y_subcolumn_mode,
            data_sets,
        })
    }

    /// Number of allocated rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.x_cells.len(), DataSetColumn::row_count)
    }

    /// Evaluate or read the active independent `X` value at `row`.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    pub fn x_value_at(&self, row: usize) -> Option<f64> {
        if row >= self.row_count() {
            return None;
        }
        match &self.x_mode {
            XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
                self.x_cells.get(row).and_then(|subs| subs.first()).and_then(Cell::active_number)
            }
            XColumnMode::GeneratedSequence { start, step, progression } => {
                let val = match progression {
                    SequenceProgression::Arithmetic => *start + (row as f64) * *step,
                    SequenceProgression::Geometric => *start * step.powi(row.min(i32::MAX as usize) as i32),
                };
                val.is_finite().then_some(val)
            }
        }
    }

    /// Read or evaluate the finite `X` value at `row` regardless of cell exclusion status.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    pub fn raw_x_value_at(&self, row: usize) -> Option<f64> {
        if row >= self.row_count() {
            return None;
        }
        match &self.x_mode {
            XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
                self.x_cells.get(row).and_then(|subs| subs.first()).and_then(|c| c.value.finite_f64())
            }
            XColumnMode::GeneratedSequence { start, step, progression } => {
                let val = match progression {
                    SequenceProgression::Arithmetic => *start + (row as f64) * *step,
                    SequenceProgression::Geometric => *start * step.powi(row.min(i32::MAX as usize) as i32),
                };
                val.is_finite().then_some(val)
            }
        }
    }

    /// Materialize the primary `X` cell at `row`, preserving both raw value and `excluded` mask.
    #[must_use]
    pub fn materialized_x_cell_at(&self, row: usize) -> Cell {
        if row >= self.row_count() {
            return Cell::empty();
        }
        match &self.x_mode {
            XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
                self.x_cells.get(row).and_then(|subs| subs.first()).cloned().unwrap_or_else(Cell::empty)
            }
            XColumnMode::GeneratedSequence { .. } => {
                let excluded = self.x_cells.get(row).and_then(|subs| subs.first()).is_some_and(|c| c.excluded);
                match self.raw_x_value_at(row) {
                    Some(v) => Cell { value: CellValue::Number(v), excluded },
                    None => Cell { value: CellValue::Empty, excluded: false },
                }
            }
        }
    }

    /// Set the primary `X` cell at `row` (subcolumn `0`).
    pub fn set_x_cell(&mut self, row: usize, cell: Cell) -> Result<(), DocError> {
        self.set_x_subcell(row, 0, cell)
    }

    /// Set an `X` cell or `X` error subcolumn at `(row, subcol)`.
    pub fn set_x_subcell(&mut self, row: usize, subcol: usize, cell: Cell) -> Result<(), DocError> {
        let Some(subs) = self.x_cells.get_mut(row) else {
            return Err(DocError::CellOutOfBounds { row, col: 0, subcol });
        };
        let Some(slot) = subs.get_mut(subcol) else {
            return Err(DocError::CellOutOfBounds { row, col: 0, subcol });
        };
        *slot = cell;
        Ok(())
    }

    /// Reconstruct parametric `(Mean, SD, N, SS_within)` for pre-summarized X rows.
    pub fn summarized_x_point(&self, row: usize) -> Result<Option<SummarizedPoint>, SummaryPointError> {
        let XColumnMode::Numbers { subcolumn_mode } = self.x_mode else {
            return Err(SummaryPointError::NotParametricSummaryMode { mode: SubcolumnMode::Single });
        };
        let Some(subcells) = self.x_cells.get(row) else {
            return Ok(None);
        };
        SummarizedPoint::reconstruct(subcolumn_mode, subcells)
    }

    /// Reconstruct parametric `(Mean, SD, N, SS_within)` for pre-summarized Y rows.
    pub fn summarized_y_point(&self, row: usize, data_set: usize) -> Result<Option<SummarizedPoint>, SummaryPointError> {
        let Some(ds) = self.data_sets.get(data_set) else {
            return Ok(None);
        };
        let Some(subcells) = ds.cells.get(row) else {
            return Ok(None);
        };
        SummarizedPoint::reconstruct(self.y_subcolumn_mode, subcells)
    }

    /// Collect all non-excluded finite `(x, y)` replicate observations for `data_set`.
    #[must_use]
    pub fn active_xy_pairs(&self, data_set: usize) -> Vec<(f64, f64)> {
        let Some(ds) = self.data_sets.get(data_set) else {
            return Vec::new();
        };
        let mut pairs = Vec::new();
        for (row, subcells) in ds.cells.iter().enumerate() {
            let Some(x) = self.x_value_at(row) else {
                continue;
            };
            if self.y_subcolumn_mode.is_pre_summarized() {
                if let Some(mean) = subcells.first().and_then(Cell::active_number) {
                    pairs.push((x, mean));
                }
            } else {
                for cell in subcells {
                    if let Some(y) = cell.active_number() {
                        pairs.push((x, y));
                    }
                }
            }
        }
        pairs
    }

    /// Validate rectangular row/subcolumn invariants, X-column mode, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.x_title)?;
        self.x_mode.validate_subcolumn_only()?;
        validate_table_subcolumn_mode(TableKind::Xy, self.y_subcolumn_mode)?;
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_rows(self.x_cells.len())?;
        if self.x_cells.len() != row_count {
            return Err(DocError::InvalidCellValue {
                archetype: TableKind::Xy,
                row: self.x_cells.len(),
                col: 0,
                reason: "X column row count does not match table row count",
            });
        }
        let x_subs = match &self.x_mode {
            XColumnMode::Numbers { .. } | XColumnMode::DateOrElapsedTime { .. } => {
                let expected = self.x_mode.subcolumn_count();
                for (row_idx, row) in self.x_cells.iter().enumerate() {
                    if row.len() > MAX_SUBCOLUMNS {
                        return Err(DocError::TooManySubcolumns { count: row.len(), max: MAX_SUBCOLUMNS });
                    }
                    if row.len() != expected {
                        return Err(DocError::InvalidCellValue {
                            archetype: TableKind::Xy,
                            row: row_idx,
                            col: 0,
                            reason: "X row subcolumn count does not match X column mode",
                        });
                    }
                }
                expected
            }
            XColumnMode::GeneratedSequence { .. } => {
                let width = self.x_cells.first().map_or(0, Vec::len);
                if width > MAX_SUBCOLUMNS {
                    return Err(DocError::TooManySubcolumns { count: width, max: MAX_SUBCOLUMNS });
                }
                for (row_idx, row) in self.x_cells.iter().enumerate() {
                    if row.len() > MAX_SUBCOLUMNS {
                        return Err(DocError::TooManySubcolumns { count: row.len(), max: MAX_SUBCOLUMNS });
                    }
                    if row.len() != width {
                        return Err(DocError::InvalidCellValue {
                            archetype: TableKind::Xy,
                            row: row_idx,
                            col: 0,
                            reason: "X cell row widths are not rectangular",
                        });
                    }
                }
                width
            }
        };
        let y_subs = self.y_subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, self.data_sets.len(), y_subs, x_subs)?;
        validate_title_vec(TableKind::Xy, &self.row_titles, row_count, "row title count does not match table row count")?;
        validate_dataset_column_grid(TableKind::Xy, &self.data_sets, row_count, y_subs)
    }
}

// ============================================================================
// 2. Column Table
// ============================================================================

/// Experimental row-index semantics for a [`ColumnTable`] (`docs/implementation/data-tables-and-transforms.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum ColumnDesignHint {
    /// Unmatched replicates stacked down each group column; row permutations within a single column
    /// leave unpaired test statistics invariant (`Row Titles` collapsed by default).
    #[default]
    UnpairedReplicates,
    /// Matched subject/block measured across columns at row `r`; `Row Titles` expanded by default
    /// and auto-created graph defaults to a before–after individual-value line plot.
    PairedRepeatedMeasures,
    /// Pre-computed group mean and spread entered directly.
    PrecomputedError,
}

/// Archetype 2: One-factor group comparison table (one treatment/group per column).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ColumnTable {
    /// Experimental pairing/summary hint guiding default analyses and graph types.
    pub design_hint: ColumnDesignHint,
    /// Subcolumn mode shared across all columns (`Single`, `MeanSdN`, `MeanSemN`, or `MeanCvN`).
    pub subcolumn_mode: SubcolumnMode,
    /// Optional subject/replicate row titles (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Whether the `Row Titles` stub column is visibly expanded in the spreadsheet UI.
    pub row_titles_expanded: bool,
    /// Ordered treatment/group columns (`A`, `B`, ...).
    pub data_sets: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawColumnTable {
    design_hint: ColumnDesignHint,
    subcolumn_mode: SubcolumnMode,
    row_titles: Vec<String>,
    row_titles_expanded: bool,
    data_sets: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for ColumnTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawColumnTable::deserialize(deserializer)?;
        let table = Self {
            design_hint: raw.design_hint,
            subcolumn_mode: raw.subcolumn_mode,
            row_titles: raw.row_titles,
            row_titles_expanded: raw.row_titles_expanded,
            data_sets: raw.data_sets,
        };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl ColumnTable {
    /// Construct a `ColumnTable` whose subcolumn mode is chosen from `design_hint` (`Single` for raw designs, `MeanSdN` for `PrecomputedError`).
    pub fn new(design_hint: ColumnDesignHint, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        let subcolumn_mode = match design_hint {
            ColumnDesignHint::UnpairedReplicates | ColumnDesignHint::PairedRepeatedMeasures => SubcolumnMode::Single,
            ColumnDesignHint::PrecomputedError => SubcolumnMode::MeanSdN,
        };
        Self::new_with_subcolumns(design_hint, subcolumn_mode, data_set_count, row_count)
    }

    /// Construct a `ColumnTable` with an explicit subcolumn layout (`Single`, `MeanSdN`,
    /// `MeanSemN`, or `MeanCvN`).
    pub fn new_with_subcolumns(
        design_hint: ColumnDesignHint,
        subcolumn_mode: SubcolumnMode,
        data_set_count: usize,
        row_count: usize,
    ) -> Result<Self, DocError> {
        validate_table_subcolumn_mode(TableKind::Column, subcolumn_mode)?;
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let subs = subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, data_set_count, subs, 0)?;
        let mut data_sets = Vec::with_capacity(data_set_count);
        for idx in 0..data_set_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, subs)?);
        }
        Ok(Self {
            design_hint,
            subcolumn_mode,
            row_titles: vec![String::new(); row_count],
            row_titles_expanded: matches!(design_hint, ColumnDesignHint::PairedRepeatedMeasures),
            data_sets,
        })
    }

    /// Number of allocated rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.row_titles.len(), DataSetColumn::row_count)
    }

    /// Reconstruct parametric `(Mean, SD, N, SS_within)` for pre-summarized column cells.
    pub fn summarized_point(&self, row: usize, col: usize) -> Result<Option<SummarizedPoint>, SummaryPointError> {
        let Some(ds) = self.data_sets.get(col) else {
            return Ok(None);
        };
        let Some(subcells) = ds.cells.get(row) else {
            return Ok(None);
        };
        SummarizedPoint::reconstruct(self.subcolumn_mode, subcells)
    }

    /// Non-excluded finite observations in column `col` (subcolumn `0`).
    #[must_use]
    pub fn active_column_values(&self, col: usize) -> Vec<f64> {
        let Some(ds) = self.data_sets.get(col) else {
            return Vec::new();
        };
        ds.cells.iter().filter_map(|subs| subs.first().and_then(Cell::active_number)).collect()
    }

    /// Complete subject blocks across `cols` where every selected column has an active finite value at row `r`.
    #[must_use]
    pub fn complete_paired_rows(&self, cols: &[usize]) -> Vec<Vec<f64>> {
        self.complete_paired_rows_with_report(cols).blocks
    }

    /// Complete subject blocks across `cols` plus counts of omitted incomplete/excluded subject rows
    /// and non-numeric cells encountered in numeric columns.
    #[must_use]
    pub fn complete_paired_rows_with_report(&self, cols: &[usize]) -> PairedRowsReport {
        if cols.is_empty() {
            return PairedRowsReport::default();
        }
        let rows = self.row_count();
        let active_end = (0..rows)
            .rfind(|&r| {
                cols.iter().any(|&c| self.data_sets.get(c).and_then(|ds| ds.cell(r, 0)).is_some_and(|cell| cell.excluded || !cell.value.is_empty()))
            })
            .map_or(0, |last| last + 1);
        let mut blocks = Vec::new();
        let mut skipped_incomplete_rows = 0_usize;
        let mut non_numeric_cells = 0_usize;
        for r in 0..active_end {
            let mut row_vals = Vec::with_capacity(cols.len());
            let mut complete = true;
            let mut row_has_entry = false;
            for &c in cols {
                let cell = self.data_sets.get(c).and_then(|ds| ds.cell(r, 0));
                if let Some(cell) = cell {
                    if cell.excluded || !cell.value.is_empty() {
                        row_has_entry = true;
                    }
                    if !cell.excluded {
                        match &cell.value {
                            CellValue::Text(s) if !s.trim().is_empty() => non_numeric_cells += 1,
                            CellValue::Number(v) if !v.is_finite() => non_numeric_cells += 1,
                            _ => {}
                        }
                    }
                }
                if let Some(val) = cell.and_then(Cell::active_number) {
                    row_vals.push(val);
                } else {
                    complete = false;
                }
            }
            if complete {
                blocks.push(row_vals);
            } else if row_has_entry {
                skipped_incomplete_rows += 1;
            }
        }
        PairedRowsReport { blocks, skipped_incomplete_rows, non_numeric_cells }
    }

    /// Validate rectangular row/subcolumn invariants, subcolumn mode compatibility, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_table_subcolumn_mode(TableKind::Column, self.subcolumn_mode)?;
        let row_count = self.row_count();
        validate_rows(row_count)?;
        let subs = self.subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, self.data_sets.len(), subs, 0)?;
        validate_title_vec(TableKind::Column, &self.row_titles, row_count, "row title count does not match table row count")?;
        validate_dataset_column_grid(TableKind::Column, &self.data_sets, row_count, subs)
    }
}

/// Complete subject blocks and listwise-exclusion summary returned by
/// [`ColumnTable::complete_paired_rows_with_report`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PairedRowsReport {
    /// Complete subject rows; each inner `Vec<f64>` has length `cols.len()` with active finite values across every requested column.
    pub blocks: Vec<Vec<f64>>,
    /// Count of non-blank subject rows omitted listwise because at least one selected column was empty, non-numeric, or excluded.
    pub skipped_incomplete_rows: usize,
    /// Count of non-excluded text or non-finite cells encountered inside the selected numeric columns.
    pub non_numeric_cells: usize,
}

// ============================================================================
// 3. Grouped Table
// ============================================================================

/// Optional `2 × 2 × K` Three-Way ANOVA column factor metadata mapping 4 data-set columns
/// `((F1_0, F2_0), (F1_0, F2_1), (F1_1, F2_0), (F1_1, F2_1))` (`docs/implementation/data-tables-and-transforms.md` §2.2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ThreeWayColumnGrouping {
    /// Display name of the outer two-level column factor *B* (columns `{0, 1}` vs `{2, 3}`).
    pub factor_1_name: String,
    /// Labels `[B_0, B_1]` of the two levels of `factor_1_name`.
    pub factor_1_levels: [String; 2],
    /// Display name of the inner two-level column factor *C* (columns `{0, 2}` vs `{1, 3}`).
    pub factor_2_name: String,
    /// Labels `[C_0, C_1]` of the two levels of `factor_2_name`.
    pub factor_2_levels: [String; 2],
}

/// Archetype 3: Two-factor (or `2 × 2 × K` three-factor) tensor `(Row r × Column c × Subcolumn s)`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GroupedTable {
    /// Display name of the row factor *A* (for example `"Time Point"` or `"Dose"`).
    pub row_factor_title: String,
    /// Display name of the primary column factor *B* (for example `"Genotype"` or `"Treatment"`).
    pub column_factor_title: String,
    /// Labels for each level `r` of the row factor (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Optional quantitative *X* coordinates corresponding to each row factor level (`len() == row_count()`) for proportional horizontal spacing on grouped line graphs; `None` spaces levels evenly.
    pub row_numeric_spacing: Option<Vec<Option<f64>>>,
    /// Replicate or pre-summarized subcolumn layout (`Single` is disallowed on `GroupedTable`).
    pub subcolumn_mode: SubcolumnMode,
    /// Visual arrangement of replicate subcolumns in the spreadsheet grid.
    pub replicate_stacking: ReplicateStacking,
    /// Optional `2 × 2` column-factor cross-classification enabling three-way factorial ANOVA across 4 data-set columns; `None` for two-way designs.
    pub three_way_grouping: Option<ThreeWayColumnGrouping>,
    /// Ordered column-factor data sets (`A`, `B`, ...).
    pub data_sets: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawGroupedTable {
    row_factor_title: String,
    column_factor_title: String,
    row_titles: Vec<String>,
    row_numeric_spacing: Option<Vec<Option<f64>>>,
    subcolumn_mode: SubcolumnMode,
    replicate_stacking: ReplicateStacking,
    three_way_grouping: Option<ThreeWayColumnGrouping>,
    data_sets: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for GroupedTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawGroupedTable::deserialize(deserializer)?;
        let table = Self {
            row_factor_title: raw.row_factor_title,
            column_factor_title: raw.column_factor_title,
            row_titles: raw.row_titles,
            row_numeric_spacing: raw.row_numeric_spacing,
            subcolumn_mode: raw.subcolumn_mode,
            replicate_stacking: raw.replicate_stacking,
            three_way_grouping: raw.three_way_grouping,
            data_sets: raw.data_sets,
        };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl GroupedTable {
    /// Construct a validated `GroupedTable` with `subcolumn_mode` (`RawReplicates(n)` or a pre-summarized mode).
    pub fn new(subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_table_subcolumn_mode(TableKind::Grouped, subcolumn_mode)?;
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let subs = subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, data_set_count, subs, 0)?;
        let mut data_sets = Vec::with_capacity(data_set_count);
        for idx in 0..data_set_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, subs)?);
        }
        Ok(Self {
            row_factor_title: String::from("Row Factor"),
            column_factor_title: String::from("Column Factor"),
            row_titles: vec![String::new(); row_count],
            row_numeric_spacing: None,
            subcolumn_mode,
            replicate_stacking: ReplicateStacking::SideBySide,
            three_way_grouping: None,
            data_sets,
        })
    }

    /// Number of allocated rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.row_titles.len(), DataSetColumn::row_count)
    }

    /// Read observation `y_{r, c, s}` at `(row_factor, col_factor, subcol_replicate)`.
    #[must_use]
    pub fn observation(&self, row: usize, col: usize, subcol: usize) -> Option<&Cell> {
        self.data_sets.get(col).and_then(|ds| ds.cell(row, subcol))
    }

    /// Non-excluded finite replicates in cell `(row, col)`.
    #[must_use]
    pub fn active_replicates(&self, row: usize, col: usize) -> Vec<f64> {
        let Some(subcells) = self.data_sets.get(col).and_then(|ds| ds.cells.get(row)) else {
            return Vec::new();
        };
        subcells.iter().filter_map(Cell::active_number).collect()
    }

    /// True if any `(row, col)` raw-replicate cell has missing or excluded subcolumns (indicating an
    /// unbalanced design or incomplete repeated-measures block requiring REML mixed effects).
    #[must_use]
    pub fn has_missing_replicates(&self) -> bool {
        let SubcolumnMode::RawReplicates(expected) = self.subcolumn_mode else {
            return false;
        };
        for ds in &self.data_sets {
            for row in &ds.cells {
                let active = row.iter().filter(|c| c.active_number().is_some()).count();
                if active < expected {
                    return true;
                }
            }
        }
        false
    }

    /// Validate rectangular row/subcolumn invariants, factor metadata, subcolumn mode, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.row_factor_title)?;
        validate_title(&self.column_factor_title)?;
        if let Some(ref three_way) = self.three_way_grouping {
            validate_title(&three_way.factor_1_name)?;
            validate_title(&three_way.factor_2_name)?;
            for lbl in &three_way.factor_1_levels {
                validate_title(lbl)?;
            }
            for lbl in &three_way.factor_2_levels {
                validate_title(lbl)?;
            }
        }
        validate_table_subcolumn_mode(TableKind::Grouped, self.subcolumn_mode)?;
        let row_count = self.row_count();
        validate_rows(row_count)?;
        let subs = self.subcolumn_mode.subcolumn_count();
        validate_cell_grid(row_count, self.data_sets.len(), subs, 0)?;
        validate_title_vec(TableKind::Grouped, &self.row_titles, row_count, "row title count does not match table row count")?;
        if let Some(ref spacing) = self.row_numeric_spacing
            && spacing.len() != row_count
        {
            return Err(DocError::InvalidCellValue {
                archetype: TableKind::Grouped,
                row: spacing.len(),
                col: 0,
                reason: "row numeric spacing count does not match table row count",
            });
        }
        validate_dataset_column_grid(TableKind::Grouped, &self.data_sets, row_count, subs)
    }

    /// Reject non-finite values in `row_numeric_spacing` (enforced on deserialization and strict validation).
    pub fn validate_finite(&self) -> Result<(), DocError> {
        if let Some(ref spacing) = self.row_numeric_spacing
            && spacing.iter().any(|v| v.is_some_and(|x| !x.is_finite()))
        {
            return Err(DocError::InvalidCellValue { archetype: TableKind::Grouped, row: 0, col: 0, reason: "row_numeric_spacing" });
        }
        Ok(())
    }
}

// ============================================================================
// 4. Contingency Table
// ============================================================================

/// Archetype 4: Categorical cross-tabulation of non-negative integer counts (`R × C`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ContingencyTable {
    /// Display name of the row classification variable (for example `"Exposure"`).
    pub row_factor_title: String,
    /// Display name of the column classification variable (for example `"Outcome"`).
    pub column_factor_title: String,
    /// Category labels for each row (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Single-subcolumn category columns storing non-negative integer counts.
    pub data_sets: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawContingencyTable {
    row_factor_title: String,
    column_factor_title: String,
    row_titles: Vec<String>,
    data_sets: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for ContingencyTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawContingencyTable::deserialize(deserializer)?;
        let table = Self {
            row_factor_title: raw.row_factor_title,
            column_factor_title: raw.column_factor_title,
            row_titles: raw.row_titles,
            data_sets: raw.data_sets,
        };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl ContingencyTable {
    /// Construct an `row_count × column_count` contingency table initialized with empty cells.
    pub fn new(column_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(column_count)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, column_count, 1, 0)?;
        let mut data_sets = Vec::with_capacity(column_count);
        for idx in 0..column_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, 1)?);
        }
        Ok(Self {
            row_factor_title: String::from("Outcome / Exposure"),
            column_factor_title: String::from("Group"),
            row_titles: vec![String::new(); row_count],
            data_sets,
        })
    }

    /// Number of row categories in the contingency table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.row_titles.len(), DataSetColumn::row_count)
    }

    /// Extract a validated non-negative integer count at `(row, col)`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    pub fn count_at(&self, row: usize, col: usize) -> Result<Option<u64>, DocError> {
        let Some(cell) = self.data_sets.get(col).and_then(|ds| ds.cell(row, 0)) else {
            return Err(DocError::CellOutOfBounds { row, col, subcol: 0 });
        };
        if cell.excluded || cell.is_empty() {
            return Ok(None);
        }
        match &cell.value {
            CellValue::Empty => Ok(None),
            CellValue::Number(v) if v.is_finite() && *v >= 0.0 && v.fract() == 0.0 && *v <= (u64::MAX as f64) => Ok(Some(*v as u64)),
            _ => Err(DocError::InvalidCellValue {
                archetype: TableKind::Contingency,
                row,
                col,
                reason: "contingency table cells must be non-negative integer counts",
            }),
        }
    }

    /// Validate that every non-excluded non-empty cell is a non-negative integer count.
    pub fn validate_counts(&self) -> Result<(), DocError> {
        for col in 0..self.data_sets.len() {
            for row in 0..self.row_count() {
                let _ = self.count_at(row, col)?;
            }
        }
        Ok(())
    }

    /// Validate rectangular row/subcolumn invariants, factor titles, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.row_factor_title)?;
        validate_title(&self.column_factor_title)?;
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_cell_grid(row_count, self.data_sets.len(), 1, 0)?;
        validate_title_vec(TableKind::Contingency, &self.row_titles, row_count, "row title count does not match table row count")?;
        validate_dataset_column_grid(TableKind::Contingency, &self.data_sets, row_count, 1)
    }
}

// ============================================================================
// 5. Survival Table
// ============================================================================

/// Subject endpoint status in a [`SurvivalTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SurvivalStatus {
    /// Subject experienced the endpoint event of interest (death, relapse, failure) at `time`.
    Event,
    /// Subject was right-censored (lost to follow-up or study end without event) at `time`.
    Censored,
}

/// Validated time-to-event record for a single subject.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SurvivalObservation {
    /// Zero-based originating row index in [`SurvivalTable`].
    pub row: usize,
    /// Non-negative elapsed follow-up time (`>= 0.0`, finite) in the table's time units.
    pub time: f64,
    /// Endpoint event vs right-censored indicator.
    pub status: SurvivalStatus,
}

/// Archetype 5: Time-to-event survival table (`t >= 0`, configurable event/censored status codes).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SurvivalTable {
    /// Display header for the elapsed-time column (default `"Elapsed Time"`).
    pub time_title: String,
    /// Per-row non-negative elapsed follow-up time cells (`len() == row_count()`).
    pub times: Vec<Cell>,
    /// Integer indicator code denoting [`SurvivalStatus::Event`] (default `1`).
    pub event_code: i64,
    /// Integer indicator code denoting [`SurvivalStatus::Censored`] (default `0`).
    pub censored_code: i64,
    /// Optional subject row identifiers (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Whether the `Row Titles` stub column is visibly expanded in the spreadsheet UI.
    pub row_titles_expanded: bool,
    /// Treatment/cohort arm columns; each non-empty cell in cohort `c` at row `r` holds `event_code` or `censored_code` for a subject at `times[r]`.
    pub cohorts: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawSurvivalTable {
    time_title: String,
    times: Vec<Cell>,
    event_code: i64,
    censored_code: i64,
    row_titles: Vec<String>,
    row_titles_expanded: bool,
    cohorts: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for SurvivalTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawSurvivalTable::deserialize(deserializer)?;
        let table = Self {
            time_title: raw.time_title,
            times: raw.times,
            event_code: raw.event_code,
            censored_code: raw.censored_code,
            row_titles: raw.row_titles,
            row_titles_expanded: raw.row_titles_expanded,
            cohorts: raw.cohorts,
        };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl SurvivalTable {
    /// Construct a `SurvivalTable` with `cohort_count` arms, `row_count` rows, `event_code = 1`, and `censored_code = 0`.
    pub fn new(cohort_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(cohort_count)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, cohort_count, 1, 1)?;
        let mut cohorts = Vec::with_capacity(cohort_count);
        for idx in 0..cohort_count {
            cohorts.push(DataSetColumn::new(default_column_label(idx), row_count, 1)?);
        }
        Ok(Self {
            time_title: String::from("Elapsed Time"),
            times: vec![Cell::empty(); row_count],
            event_code: 1,
            censored_code: 0,
            row_titles: vec![String::new(); row_count],
            row_titles_expanded: false,
            cohorts,
        })
    }

    /// Number of allocated subject rows in the survival table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.times.len()
    }

    /// Extract validated `(time, Event | Censored)` observations for `cohort_idx`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    pub fn cohort_observations(&self, cohort_idx: usize) -> Result<Vec<SurvivalObservation>, DocError> {
        let Some(cohort) = self.cohorts.get(cohort_idx) else {
            return Err(DocError::CellOutOfBounds { row: 0, col: cohort_idx, subcol: 0 });
        };
        let mut out = Vec::new();
        for (row, status_subs) in cohort.cells.iter().enumerate() {
            let Some(status_cell) = status_subs.first() else {
                continue;
            };
            if status_cell.excluded || status_cell.is_empty() {
                continue;
            }
            let Some(time_cell) = self.times.get(row) else {
                continue;
            };
            if time_cell.excluded || time_cell.is_empty() {
                continue;
            }
            let Some(t) = time_cell.active_number() else {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::Survival,
                    row,
                    col: 0,
                    reason: "elapsed survival time must be numeric",
                });
            };
            if t < 0.0 {
                return Err(DocError::InvalidCellValue { archetype: TableKind::Survival, row, col: 0, reason: "elapsed survival time must be >= 0" });
            }
            let Some(code_f) = status_cell.active_number() else {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::Survival,
                    row,
                    col: cohort_idx,
                    reason: "survival status code must be an integer matching event_code or censored_code",
                });
            };
            if code_f.fract() != 0.0 || code_f < (i64::MIN as f64) || code_f > (i64::MAX as f64) {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::Survival,
                    row,
                    col: cohort_idx,
                    reason: "survival status code must be an exact integer",
                });
            }
            let code = code_f as i64;
            let status = if code == self.event_code {
                SurvivalStatus::Event
            } else if code == self.censored_code {
                SurvivalStatus::Censored
            } else {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::Survival,
                    row,
                    col: cohort_idx,
                    reason: "unrecognized survival status code",
                });
            };
            out.push(SurvivalObservation { row, time: t, status });
        }
        Ok(out)
    }

    /// Validate rectangular row/subcolumn invariants, time title, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        validate_title(&self.time_title)?;
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_cell_grid(row_count, self.cohorts.len(), 1, 1)?;
        validate_title_vec(TableKind::Survival, &self.row_titles, row_count, "row title count does not match table row count")?;
        validate_dataset_column_grid(TableKind::Survival, &self.cohorts, row_count, 1)
    }
}

// ============================================================================
// 6. Parts of Whole Table
// ============================================================================

/// Validated non-negative slice observation and normalized fraction of total.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SliceFraction {
    /// Zero-based row index of the slice.
    pub row: usize,
    /// Category label for the slice.
    pub label: String,
    /// Raw non-negative slice magnitude (`>= 0.0`).
    pub value: f64,
    /// Unit proportion `value / sum(values)` in `[0.0, 1.0]` (`0.0` when the series sum is `0.0`).
    pub fraction: f64,
}

/// Archetype 6: Mutually exclusive categorical slices summing to a whole.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PartsOfWholeTable {
    /// Per-row category labels (`empty` or `len() == row_count()`).
    pub slice_labels: Vec<String>,
    /// Single-subcolumn series of non-negative slice magnitudes.
    pub data_sets: Vec<DataSetColumn>,
}

#[derive(serde::Deserialize)]
struct RawPartsOfWholeTable {
    slice_labels: Vec<String>,
    data_sets: Vec<DataSetColumn>,
}

impl<'de> serde::Deserialize<'de> for PartsOfWholeTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawPartsOfWholeTable::deserialize(deserializer)?;
        let table = Self { slice_labels: raw.slice_labels, data_sets: raw.data_sets };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl PartsOfWholeTable {
    /// Construct a `PartsOfWholeTable` with `series_count` series and `row_count` slice rows.
    pub fn new(series_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(series_count)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, series_count, 1, 0)?;
        let mut data_sets = Vec::with_capacity(series_count);
        for idx in 0..series_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, 1)?);
        }
        Ok(Self { slice_labels: vec![String::new(); row_count], data_sets })
    }

    /// Number of slice rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.slice_labels.len(), DataSetColumn::row_count)
    }

    /// Compute `(label, value, fraction)` for all non-excluded slices in `data_set`.
    pub fn fractions_for_series(&self, data_set: usize) -> Result<Vec<SliceFraction>, DocError> {
        let Some(ds) = self.data_sets.get(data_set) else {
            return Err(DocError::CellOutOfBounds { row: 0, col: data_set, subcol: 0 });
        };
        let mut raw_slices = Vec::new();
        let mut total = 0.0;
        for (row, subs) in ds.cells.iter().enumerate() {
            let Some(cell) = subs.first() else { continue };
            let Some(val) = cell.active_number() else { continue };
            if val < 0.0 {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::PartsOfWhole,
                    row,
                    col: data_set,
                    reason: "parts-of-whole slice values must be >= 0",
                });
            }
            total += val;
            let label = self.slice_labels.get(row).cloned().unwrap_or_default();
            raw_slices.push((row, label, val));
        }
        Ok(raw_slices
            .into_iter()
            .map(|(row, label, value)| SliceFraction { row, label, value, fraction: if total > 0.0 { value / total } else { 0.0 } })
            .collect())
    }

    /// Validate rectangular row/subcolumn invariants, slice labels, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_cell_grid(row_count, self.data_sets.len(), 1, 0)?;
        validate_title_vec(TableKind::PartsOfWhole, &self.slice_labels, row_count, "slice label count does not match table row count")?;
        validate_dataset_column_grid(TableKind::PartsOfWhole, &self.data_sets, row_count, 1)
    }
}

// ============================================================================
// 7. Multiple Variables Table
// ============================================================================

/// Typed semantic role of a column in a [`MultipleVariablesTable`] (`docs/implementation/data-tables-and-transforms.md` §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum VariableRole {
    /// Quantitative real-valued variable eligible for regression predictors/outcomes, PCA, correlation, and continuous graph axes.
    #[default]
    Continuous,
    /// Nominal or ordinal factor encoded into design-matrix dummy contrasts (first ordered level = reference level) and categorical color/symbol channels.
    Categorical,
    /// Subject/case identifier text used for point callouts and audit tables without entering numerical models.
    Label,
    /// Read-only column dynamically evaluated from [`VariableSource::Calculated`] formula expressions.
    Calculated,
}

/// Origin of values in a [`VariableColumn`] (`docs/implementation/data-tables-and-transforms.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum VariableSource {
    /// Directly entered or imported cell values.
    #[default]
    Entered,
    /// Formula stored normalized without a leading `=` character.
    Calculated {
        /// Normalized expression string evaluated across rows by `curvebook-transform::formula`.
        formula: String,
    },
}

/// Ordering rule for distinct levels of a `Categorical` variable (`docs/implementation/data-tables-and-transforms.md` §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LevelOrderKind {
    /// Order of first appearance in the column.
    #[default]
    EntryOrder,
    /// Natural case-insensitive alphanumeric sort (`"Group 2" < "Group 10"`).
    Alphabetical,
    /// Count of non-excluded rows descending (ties broken by entry order).
    FrequencyDescending,
    /// Count of non-excluded rows ascending (ties broken by entry order).
    FrequencyAscending,
    /// Manual sequence; levels absent from the explicit list append deterministically in entry order.
    Explicit(Vec<String>),
}

/// Complete categorical level-ordering specification (`LevelOrderKind` + `reversed` modifier).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct LevelOrderSpec {
    /// Base sorting rule for distinct factor levels (the first resulting level serves as the GLM/regression reference level).
    pub order: LevelOrderKind,
    /// When `true`, reverses the level sequence produced by `order`.
    pub reversed: bool,
}

impl LevelOrderSpec {
    /// Rewrite explicit level-order tokens using `(from, to)` recode rules, deduplicating while
    /// preserving first-occurrence order when multiple old levels map to the same token.
    pub fn recode_levels(&mut self, mappings: &[(String, String)]) {
        if mappings.is_empty() {
            return;
        }
        let LevelOrderKind::Explicit(order_list) = &mut self.order else {
            return;
        };
        let mut updated = Vec::with_capacity(order_list.len());
        let mut seen = std::collections::HashSet::with_capacity(order_list.len());
        for level in order_list.drain(..) {
            let mut target = level;
            for (from, to) in mappings {
                if (target == *from || target.trim() == from.trim()) && from != to {
                    target = to.trim().to_owned();
                    break;
                }
            }
            let canonical = target.trim();
            if !canonical.is_empty() && seen.insert(canonical.to_owned()) {
                updated.push(if target.len() == canonical.len() { target } else { canonical.to_owned() });
            }
        }
        *order_list = updated;
    }
}

/// Frequency summary for a single categorical level in a [`VariableColumn`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CategoricalLevelSummary {
    /// Canonical trimmed level token.
    pub level: String,
    /// Number of active (`!excluded`) rows carrying `level`.
    pub included_count: usize,
    /// Number of masked (`excluded == true`) rows carrying `level`.
    pub excluded_count: usize,
}

/// Single named variable column in a [`MultipleVariablesTable`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VariableColumn {
    /// Column variable name (`<= MAX_TITLE_BYTES` UTF-8 bytes; addressable by `[Title]` in calculated formulas).
    pub title: String,
    /// Semantic variable role (`Continuous`, `Categorical`, `Label`, or `Calculated`).
    pub role: VariableRole,
    /// `true` when `role` was auto-inferred from entered tokens (muted badge); `false` once
    /// explicitly confirmed by the user (accented badge).
    pub role_inferred: bool,
    /// Whether `cells` are user-entered or derived from a calculated-variable formula.
    pub source: VariableSource,
    /// Ordering specification for distinct categorical levels when `role == VariableRole::Categorical`.
    pub level_order: LevelOrderSpec,
    /// Per-subject cells (`len() == row_count()`).
    pub cells: Vec<Cell>,
}

impl VariableColumn {
    /// Construct an empty `VariableColumn` with auto-inferred `Continuous` role.
    pub fn new(title: impl Into<String>, row_count: usize) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        validate_rows(row_count)?;
        Ok(Self {
            title,
            role: VariableRole::Continuous,
            role_inferred: true,
            source: VariableSource::Entered,
            level_order: LevelOrderSpec::default(),
            cells: vec![Cell::empty(); row_count],
        })
    }

    /// Explicitly assign `role` and lock `role_inferred = false` (`table.setVariableRole`).
    pub fn lock_role(&mut self, role: VariableRole) {
        self.role = role;
        self.role_inferred = false;
    }

    /// Configure this column as an in-table Calculated Variable (`=formula`), stripping any leading `=`.
    pub fn set_calculated_formula(&mut self, raw_formula: &str) {
        let normalized = raw_formula.trim().strip_prefix('=').unwrap_or(raw_formula.trim()).trim().to_owned();
        self.role = VariableRole::Calculated;
        self.role_inferred = false;
        self.source = VariableSource::Calculated { formula: normalized };
    }

    /// Re-infer `Continuous` vs `Categorical` from non-empty cells iff `role_inferred` is `true`
    /// and the column is not `Calculated`.
    pub fn refresh_inferred_role(&mut self) {
        if !self.role_inferred || matches!(self.source, VariableSource::Calculated { .. }) {
            return;
        }
        let mut saw_any = false;
        let mut saw_text = false;
        for cell in &self.cells {
            if cell.is_empty() {
                continue;
            }
            saw_any = true;
            if matches!(cell.value, CellValue::Text(_)) {
                saw_text = true;
                break;
            }
        }
        if saw_any {
            self.role = if saw_text { VariableRole::Categorical } else { VariableRole::Continuous };
        }
    }

    /// Atomically recode categorical tokens across both included and excluded cells
    /// (`table.recodeLevels`; `docs/implementation/data-tables-and-transforms.md` §2.3), returning the number of
    /// rewritten cells.
    pub fn recode_levels(&mut self, mappings: &[(String, String)]) -> usize {
        if mappings.is_empty() {
            return 0;
        }
        let mut changed = 0;
        self.level_order.recode_levels(mappings);
        for cell in &mut self.cells {
            let Some(current) = cell_categorical_token(&cell.value) else {
                continue;
            };
            for (from, to) in mappings {
                let from_trimmed = from.trim();
                let to_trimmed = to.trim();
                if current == from_trimmed && from_trimmed != to_trimmed {
                    cell.value = CellValue::Text(to_trimmed.to_string());
                    changed += 1;
                    break;
                }
            }
        }
        self.refresh_inferred_role();
        changed
    }

    /// Compute distinct categorical levels ordered by this column's [`LevelOrderSpec`].
    #[must_use]
    pub fn ordered_levels(&self) -> Vec<CategoricalLevelSummary> {
        self.ordered_levels_with(&self.level_order)
    }

    /// Compute distinct categorical levels ordered by an arbitrary [`LevelOrderSpec`] (allowing
    /// graphs to override display order without mutating the source table's contrast reference level).
    #[must_use]
    pub fn ordered_levels_with(&self, spec: &LevelOrderSpec) -> Vec<CategoricalLevelSummary> {
        let mut entries: Vec<(usize, CategoricalLevelSummary)> = Vec::new();
        let mut index_by_level: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for cell in &self.cells {
            let Some(tok) = cell_categorical_token(&cell.value) else {
                continue;
            };
            if let Some(&idx) = index_by_level.get(&tok)
                && let Some((_, existing)) = entries.get_mut(idx)
            {
                if cell.excluded {
                    existing.excluded_count += 1;
                } else {
                    existing.included_count += 1;
                }
            } else {
                let rank = entries.len();
                index_by_level.insert(tok.clone(), rank);
                entries.push((
                    rank,
                    CategoricalLevelSummary { level: tok, included_count: usize::from(!cell.excluded), excluded_count: usize::from(cell.excluded) },
                ));
            }
        }

        match &spec.order {
            LevelOrderKind::EntryOrder => {}
            LevelOrderKind::Alphabetical => {
                entries.sort_by(|(ra, a), (rb, b)| natural_case_insensitive_cmp(&a.level, &b.level).then_with(|| ra.cmp(rb)));
            }
            LevelOrderKind::FrequencyDescending => {
                entries.sort_by(|(ra, a), (rb, b)| b.included_count.cmp(&a.included_count).then_with(|| ra.cmp(rb)));
            }
            LevelOrderKind::FrequencyAscending => {
                entries.sort_by(|(ra, a), (rb, b)| a.included_count.cmp(&b.included_count).then_with(|| ra.cmp(rb)));
            }
            LevelOrderKind::Explicit(order_list) => {
                let mut rank_by_level: std::collections::HashMap<&str, usize> = std::collections::HashMap::with_capacity(order_list.len());
                for (pos, level) in order_list.iter().enumerate() {
                    rank_by_level.entry(level.trim()).or_insert(pos);
                }
                entries.sort_by(|(ra, a), (rb, b)| {
                    let pa = rank_by_level.get(a.level.as_str()).copied();
                    let pb = rank_by_level.get(b.level.as_str()).copied();
                    match (pa, pb) {
                        (Some(ia), Some(ib)) => ia.cmp(&ib),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => ra.cmp(rb),
                    }
                });
            }
        }

        let mut out: Vec<CategoricalLevelSummary> = entries.into_iter().map(|(_, s)| s).collect();
        if spec.reversed {
            out.reverse();
        }
        out
    }
}

fn cell_categorical_token(val: &CellValue) -> Option<String> {
    match val {
        CellValue::Empty => None,
        CellValue::Text(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        CellValue::Number(v) if v.is_finite() => Some(format!("{v}")),
        CellValue::Number(_) => None,
    }
}

fn natural_case_insensitive_cmp(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars().peekable();
    let mut ib = b.chars().peekable();
    loop {
        match (ia.peek(), ib.peek()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&ca), Some(&cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_digits(&mut ia);
                let nb = take_digits(&mut ib);
                let sa = na.trim_start_matches('0');
                let sb = nb.trim_start_matches('0');
                let ord = sa.len().cmp(&sb.len()).then_with(|| sa.cmp(sb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(&ca), Some(&cb)) => {
                let _ = ia.next();
                let _ = ib.next();
                let ord = ca.to_ascii_lowercase().cmp(&cb.to_ascii_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn take_digits<I: Iterator<Item = char>>(iter: &mut std::iter::Peekable<I>) -> String {
    let mut s = String::new();
    while let Some(&ch) = iter.peek() {
        if ch.is_ascii_digit() {
            s.push(ch);
            let _ = iter.next();
        } else {
            break;
        }
    }
    s
}

/// Archetype 7: Tidy observational dataframe (`rows = observational units`, `columns = typed variables`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MultipleVariablesTable {
    /// Optional per-subject row stub labels (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Whether the `Row Titles` stub column is visibly expanded in the spreadsheet UI.
    pub row_titles_expanded: bool,
    /// Ordered typed variable columns.
    pub variables: Vec<VariableColumn>,
}

#[derive(serde::Deserialize)]
struct RawMultipleVariablesTable {
    row_titles: Vec<String>,
    row_titles_expanded: bool,
    variables: Vec<VariableColumn>,
}

impl<'de> serde::Deserialize<'de> for MultipleVariablesTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawMultipleVariablesTable::deserialize(deserializer)?;
        let table = Self { row_titles: raw.row_titles, row_titles_expanded: raw.row_titles_expanded, variables: raw.variables };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl MultipleVariablesTable {
    /// Construct a `MultipleVariablesTable` with `variable_count` columns (`"Variable A"`, `"Variable B"`, ...) and `row_count` rows.
    pub fn new(variable_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(variable_count)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, variable_count, 1, 0)?;
        let mut variables = Vec::with_capacity(variable_count);
        for idx in 0..variable_count {
            variables.push(VariableColumn::new(format!("Variable {}", default_column_label(idx)), row_count)?);
        }
        Ok(Self { row_titles: vec![String::new(); row_count], row_titles_expanded: false, variables })
    }

    /// Number of observational rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.variables.first().map_or(self.row_titles.len(), |v| v.cells.len())
    }

    /// Find variable index by exact or trimmed title.
    #[must_use]
    pub fn find_variable(&self, name: &str) -> Option<usize> {
        let needle = name.trim();
        self.variables.iter().position(|v| v.title.trim() == needle)
    }

    /// Validate rectangular row invariants across variables, row titles, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_data_sets(self.variables.len())?;
        validate_cell_grid(row_count, self.variables.len(), 1, 0)?;
        validate_title_vec(TableKind::MultipleVariables, &self.row_titles, row_count, "row title count does not match table row count")?;
        for (col_idx, var) in self.variables.iter().enumerate() {
            validate_title(&var.title)?;
            validate_rows(var.cells.len())?;
            if var.cells.len() != row_count {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::MultipleVariables,
                    row: var.cells.len(),
                    col: col_idx,
                    reason: "variable row count does not match table row count",
                });
            }
        }
        Ok(())
    }
}

// ============================================================================
// 8. Nested Table
// ============================================================================

/// Experimental group (`A`, `B`, ...) containing biological-unit subcolumns (`A:1`, `A:2`, ...)
/// with technical replicates stacked down rows (`docs/implementation/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NestedGroupColumn {
    /// Top-level experimental treatment group title (for example `"Control"` or `"Treated"`).
    pub title: String,
    /// Second-level biological sub-unit labels (`len() == biological_unit_count()`, for example `["A:1", "A:2", "A:3"]`).
    pub subcolumn_titles: Vec<String>,
    /// Technical replicate grid indexed as `cells[tech_replicate_row][bio_unit_subcol]`.
    pub cells: Vec<Vec<Cell>>,
}

impl NestedGroupColumn {
    /// Allocate an empty `NestedGroupColumn` with `biological_unit_count` subcolumns and `row_count` technical-replicate rows.
    pub fn new(title: impl Into<String>, biological_unit_count: usize, row_count: usize) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        validate_subcolumns(biological_unit_count)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, 1, biological_unit_count, 0)?;
        let subcolumn_titles = (1..=biological_unit_count).map(|i| format!("{title}:{i}")).collect();
        Ok(Self { title, subcolumn_titles, cells: vec![vec![Cell::empty(); biological_unit_count]; row_count] })
    }

    /// Number of nested biological experimental units (subcolumns) inside this group.
    #[must_use]
    pub fn biological_unit_count(&self) -> usize {
        self.subcolumn_titles.len()
    }
}

/// Archetype 8: Hierarchical two-level design (experimental groups -> biological units -> technical replicates).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NestedTable {
    /// Optional per-row technical-replicate stub labels (`empty` or `len() == row_count()`).
    pub row_titles: Vec<String>,
    /// Whether the `Row Titles` stub column is visibly expanded in the spreadsheet UI.
    pub row_titles_expanded: bool,
    /// Ordered top-level experimental treatment groups.
    pub groups: Vec<NestedGroupColumn>,
}

#[derive(serde::Deserialize)]
struct RawNestedTable {
    row_titles: Vec<String>,
    row_titles_expanded: bool,
    groups: Vec<NestedGroupColumn>,
}

impl<'de> serde::Deserialize<'de> for NestedTable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawNestedTable::deserialize(deserializer)?;
        let table = Self { row_titles: raw.row_titles, row_titles_expanded: raw.row_titles_expanded, groups: raw.groups };
        table.validate().map_err(serde::de::Error::custom)?;
        Ok(table)
    }
}

impl NestedTable {
    /// Construct a `NestedTable` with `group_count` treatments, `biological_units_per_group` subcolumns per treatment, and `row_count` technical-replicate rows.
    pub fn new(group_count: usize, biological_units_per_group: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(group_count)?;
        validate_subcolumns(biological_units_per_group)?;
        validate_rows(row_count)?;
        validate_cell_grid(row_count, group_count, biological_units_per_group, 0)?;
        let mut groups = Vec::with_capacity(group_count);
        for idx in 0..group_count {
            groups.push(NestedGroupColumn::new(default_column_label(idx), biological_units_per_group, row_count)?);
        }
        Ok(Self { row_titles: vec![String::new(); row_count], row_titles_expanded: false, groups })
    }

    /// Number of technical-replicate rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.groups.first().map_or(self.row_titles.len(), |g| g.cells.len())
    }

    /// Non-excluded finite technical replicates inside `(group_idx, biological_unit_idx)`.
    #[must_use]
    pub fn active_technical_replicates(&self, group_idx: usize, bio_unit_idx: usize) -> Vec<f64> {
        let Some(group) = self.groups.get(group_idx) else {
            return Vec::new();
        };
        group.cells.iter().filter_map(|row| row.get(bio_unit_idx).and_then(Cell::active_number)).collect()
    }

    /// Validate rectangular row/subcolumn invariants across groups, unit headers, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        let row_count = self.row_count();
        validate_rows(row_count)?;
        validate_data_sets(self.groups.len())?;
        validate_title_vec(TableKind::Nested, &self.row_titles, row_count, "row title count does not match table row count")?;
        let mut total_cells: usize = 0;
        for (g_idx, grp) in self.groups.iter().enumerate() {
            validate_title(&grp.title)?;
            let units = grp.biological_unit_count();
            validate_subcolumns(units)?;
            for sub_title in &grp.subcolumn_titles {
                validate_title(sub_title)?;
            }
            validate_rows(grp.cells.len())?;
            if grp.cells.len() != row_count {
                return Err(DocError::InvalidCellValue {
                    archetype: TableKind::Nested,
                    row: grp.cells.len(),
                    col: g_idx,
                    reason: "nested group row count does not match table row count",
                });
            }
            let group_cells = row_count
                .checked_mul(units)
                .and_then(|c| total_cells.checked_add(c))
                .ok_or(DocError::TooManyCells { count: usize::MAX, max: crate::error::MAX_SHEET_CELLS })?;
            validate_sheet_cells(group_cells)?;
            total_cells = group_cells;
            for (row_idx, row) in grp.cells.iter().enumerate() {
                if row.len() > MAX_SUBCOLUMNS {
                    return Err(DocError::TooManySubcolumns { count: row.len(), max: MAX_SUBCOLUMNS });
                }
                if row.len() != units {
                    return Err(DocError::InvalidCellValue {
                        archetype: TableKind::Nested,
                        row: row_idx,
                        col: g_idx,
                        reason: "nested group subcolumn count does not match biological unit count",
                    });
                }
            }
        }
        Ok(())
    }
}

// ============================================================================
// Unified TableArchetype Sum Type
// ============================================================================

/// Strongly typed table payload carried by every [`crate::DataSheet`] (`docs/implementation/data-tables-and-transforms.md` §1.1 & §2).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum TableArchetype {
    /// Continuous *X* vs one or more *Y* response series ([`XyTable`]).
    Xy(XyTable),
    /// One-way group comparison columns ([`ColumnTable`]).
    Column(ColumnTable),
    /// Two-way or `2 × 2 × K` three-way factorial table ([`GroupedTable`]).
    Grouped(GroupedTable),
    /// Non-negative integer count cross-tabulation ([`ContingencyTable`]).
    Contingency(ContingencyTable),
    /// Elapsed-time and event/censoring survival table ([`SurvivalTable`]).
    Survival(SurvivalTable),
    /// Non-negative categorical slice proportions ([`PartsOfWholeTable`]).
    PartsOfWhole(PartsOfWholeTable),
    /// Multivariate tidy observation dataframe ([`MultipleVariablesTable`]).
    MultipleVariables(MultipleVariablesTable),
    /// Hierarchical treatment -> biological unit -> technical replicate table ([`NestedTable`]).
    Nested(NestedTable),
}

impl TableArchetype {
    /// Discriminant [`TableKind`] for this archetype.
    #[must_use]
    pub const fn kind(&self) -> TableKind {
        match self {
            Self::Xy(_) => TableKind::Xy,
            Self::Column(_) => TableKind::Column,
            Self::Grouped(_) => TableKind::Grouped,
            Self::Contingency(_) => TableKind::Contingency,
            Self::Survival(_) => TableKind::Survival,
            Self::PartsOfWhole(_) => TableKind::PartsOfWhole,
            Self::MultipleVariables(_) => TableKind::MultipleVariables,
            Self::Nested(_) => TableKind::Nested,
        }
    }

    /// Validate rectangular row/subcolumn invariants, subcolumn mode rules, and dimension caps.
    pub fn validate(&self) -> Result<(), DocError> {
        match self {
            Self::Xy(t) => t.validate(),
            Self::Column(t) => t.validate(),
            Self::Grouped(t) => t.validate(),
            Self::Contingency(t) => t.validate(),
            Self::Survival(t) => t.validate(),
            Self::PartsOfWhole(t) => t.validate(),
            Self::MultipleVariables(t) => t.validate(),
            Self::Nested(t) => t.validate(),
        }
    }

    /// Number of logical rows in the table.
    #[must_use]
    pub fn row_count(&self) -> usize {
        match self {
            Self::Xy(t) => t.row_count(),
            Self::Column(t) => t.row_count(),
            Self::Grouped(t) => t.row_count(),
            Self::Contingency(t) => t.row_count(),
            Self::Survival(t) => t.row_count(),
            Self::PartsOfWhole(t) => t.row_count(),
            Self::MultipleVariables(t) => t.row_count(),
            Self::Nested(t) => t.row_count(),
        }
    }

    /// Number of primary data-set / cohort / variable / group columns.
    #[must_use]
    pub fn column_count(&self) -> usize {
        match self {
            Self::Xy(t) => t.data_sets.len(),
            Self::Column(t) => t.data_sets.len(),
            Self::Grouped(t) => t.data_sets.len(),
            Self::Contingency(t) => t.data_sets.len(),
            Self::Survival(t) => t.cohorts.len(),
            Self::PartsOfWhole(t) => t.data_sets.len(),
            Self::MultipleVariables(t) => t.variables.len(),
            Self::Nested(t) => t.groups.len(),
        }
    }

    /// Read a data-set / variable cell by `(row, col, subcol)`.
    #[must_use]
    pub fn data_cell(&self, coord: CellCoord) -> Option<&Cell> {
        match self {
            Self::Xy(t) => t.data_sets.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::Column(t) => t.data_sets.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::Grouped(t) => t.data_sets.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::Contingency(t) => t.data_sets.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::Survival(t) => t.cohorts.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::PartsOfWhole(t) => t.data_sets.get(coord.col)?.cell(coord.row, coord.subcol),
            Self::MultipleVariables(t) => (coord.subcol == 0).then(|| t.variables.get(coord.col)?.cells.get(coord.row)).flatten(),
            Self::Nested(t) => t.groups.get(coord.col)?.cells.get(coord.row)?.get(coord.subcol),
        }
    }

    /// Mutably borrow a data-set / variable cell by `(row, col, subcol)`.
    pub fn data_cell_mut(&mut self, coord: CellCoord) -> Option<&mut Cell> {
        match self {
            Self::Xy(t) => t.data_sets.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::Column(t) => t.data_sets.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::Grouped(t) => t.data_sets.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::Contingency(t) => t.data_sets.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::Survival(t) => t.cohorts.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::PartsOfWhole(t) => t.data_sets.get_mut(coord.col)?.cell_mut(coord.row, coord.subcol),
            Self::MultipleVariables(t) => {
                if coord.subcol != 0 {
                    return None;
                }
                t.variables.get_mut(coord.col)?.cells.get_mut(coord.row)
            }
            Self::Nested(t) => t.groups.get_mut(coord.col)?.cells.get_mut(coord.row)?.get_mut(coord.subcol),
        }
    }

    /// Read an independent `X` (`XyTable`) or elapsed-time (`SurvivalTable`, `subcol == 0`) cell.
    #[must_use]
    pub fn x_or_time_cell(&self, row: usize, subcol: usize) -> Option<&Cell> {
        match self {
            Self::Xy(t) => t.x_cells.get(row)?.get(subcol),
            Self::Survival(t) => (subcol == 0).then(|| t.times.get(row)).flatten(),
            Self::Column(_) | Self::Grouped(_) | Self::Contingency(_) | Self::PartsOfWhole(_) | Self::MultipleVariables(_) | Self::Nested(_) => None,
        }
    }

    /// Mutably borrow an independent `X` (`XyTable`) or elapsed-time (`SurvivalTable`, `subcol == 0`) cell.
    pub fn x_or_time_cell_mut(&mut self, row: usize, subcol: usize) -> Option<&mut Cell> {
        match self {
            Self::Xy(t) => t.x_cells.get_mut(row)?.get_mut(subcol),
            Self::Survival(t) => {
                if subcol != 0 {
                    return None;
                }
                t.times.get_mut(row)
            }
            Self::Column(_) | Self::Grouped(_) | Self::Contingency(_) | Self::PartsOfWhole(_) | Self::MultipleVariables(_) | Self::Nested(_) => None,
        }
    }

    /// Exact number of allocated [`Cell`] slots held across all internal buffers of this archetype
    /// (`G10` single source of truth for import budget checks).
    #[must_use]
    pub fn allocated_cell_slots(&self) -> usize {
        fn dataset_slots(cols: &[DataSetColumn]) -> usize {
            cols.iter().map(|c| c.cells.iter().map(Vec::len).sum::<usize>()).sum()
        }
        match self {
            Self::Xy(t) => t.x_cells.iter().map(Vec::len).sum::<usize>() + dataset_slots(&t.data_sets),
            Self::Column(t) => dataset_slots(&t.data_sets),
            Self::Grouped(t) => dataset_slots(&t.data_sets),
            Self::Contingency(t) => dataset_slots(&t.data_sets),
            Self::Survival(t) => t.times.len() + dataset_slots(&t.cohorts),
            Self::PartsOfWhole(t) => dataset_slots(&t.data_sets),
            Self::MultipleVariables(t) => t.variables.iter().map(|v| v.cells.len()).sum(),
            Self::Nested(t) => t.groups.iter().map(|g| g.cells.iter().map(Vec::len).sum::<usize>()).sum(),
        }
    }

    /// Shift existing rows in `start_row..existing_rows` downward by `insert_rows` inside
    /// pre-grown row storage so newly inserted rows occupy `start_row..start_row + insert_rows`.
    ///
    /// Callers must grow the archetype's row storage to at least `existing_rows + insert_rows`
    /// before invoking this method. Any slice shorter than `existing_rows + insert_rows` is left
    /// untouched; if `insert_rows == 0` or `start_row >= existing_rows`, this is a no-op.
    /// Shift existing rows starting at `start_row` down by `insert_rows` when inserting into `existing_rows`.
    ///
    /// Note: this only shifts cells and row metadata owned directly by [`TableArchetype`]. Callers
    /// holding a [`crate::DataSheet`] should call [`crate::DataSheet::shift_rows_down`] so
    /// sheet-level `exclusion_notes` and `linked_regions` stay aligned with moved rows.
    pub fn shift_rows_down(&mut self, start_row: usize, insert_rows: usize, existing_rows: usize) {
        if insert_rows == 0 || start_row >= existing_rows {
            return;
        }
        let Some(end) = existing_rows.checked_add(insert_rows) else {
            return;
        };
        match self {
            Self::Xy(t) => {
                rotate_rows_down(&mut t.x_cells, start_row, insert_rows, end);
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                for ds in &mut t.data_sets {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::Column(t) => {
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                for ds in &mut t.data_sets {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::Grouped(t) => {
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                if let Some(spacing) = &mut t.row_numeric_spacing {
                    rotate_rows_down(spacing, start_row, insert_rows, end);
                }
                for ds in &mut t.data_sets {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::Contingency(t) => {
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                for ds in &mut t.data_sets {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::Survival(t) => {
                rotate_rows_down(&mut t.times, start_row, insert_rows, end);
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                for ds in &mut t.cohorts {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::PartsOfWhole(t) => {
                rotate_rows_down(&mut t.slice_labels, start_row, insert_rows, end);
                for ds in &mut t.data_sets {
                    rotate_rows_down(&mut ds.cells, start_row, insert_rows, end);
                }
            }
            Self::MultipleVariables(mv) => {
                rotate_rows_down(&mut mv.row_titles, start_row, insert_rows, end);
                for var in &mut mv.variables {
                    rotate_rows_down(&mut var.cells, start_row, insert_rows, end);
                }
            }
            Self::Nested(t) => {
                rotate_rows_down(&mut t.row_titles, start_row, insert_rows, end);
                for grp in &mut t.groups {
                    rotate_rows_down(&mut grp.cells, start_row, insert_rows, end);
                }
            }
        }
    }
}

fn rotate_rows_down<T>(rows: &mut [T], start_row: usize, shift: usize, end_row: usize) {
    if let Some(slice) = rows.get_mut(start_row..end_row)
        && shift < slice.len()
    {
        slice.rotate_right(shift);
    }
}

#[cfg(test)]
mod shift_rows_tests {
    use super::{
        ColumnDesignHint, ColumnTable, ContingencyTable, GroupedTable, MultipleVariablesTable, NestedTable, PartsOfWholeTable, SubcolumnMode,
        SurvivalTable, TableArchetype, XyTable,
    };
    use crate::cell::Cell;

    #[test]
    fn shift_rows_down_rotates_all_eight_archetypes() {
        // Xy
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
        xy.x_cells[0][0] = Cell::number(1.0);
        xy.x_cells[1][0] = Cell::number(2.0);
        xy.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        xy.data_sets[0].cells[0][0] = Cell::number(10.0);
        xy.data_sets[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::Xy(xy);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::Xy(t) = arch {
            assert_eq!(t.x_cells[2][0], Cell::number(2.0));
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.data_sets[0].cells[2][0], Cell::number(20.0));
        }

        // Column
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
        col.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        col.data_sets[0].cells[0][0] = Cell::number(10.0);
        col.data_sets[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::Column(col);
        arch.shift_rows_down(0, 1, 2);
        if let TableArchetype::Column(t) = arch {
            assert_eq!(t.row_titles, vec!["", "r0", "r1"]);
            assert_eq!(t.data_sets[0].cells[1][0], Cell::number(10.0));
        }

        // Grouped (including row_numeric_spacing)
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 1, 3).unwrap();
        grp.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        grp.row_numeric_spacing = Some(vec![Some(10.0), Some(20.0), None]);
        grp.data_sets[0].cells[0][0] = Cell::number(10.0);
        grp.data_sets[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::Grouped(grp);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::Grouped(t) = arch {
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.row_numeric_spacing, Some(vec![Some(10.0), None, Some(20.0)]));
            assert_eq!(t.data_sets[0].cells[2][0], Cell::number(20.0));
        }

        // Contingency
        let mut cont = ContingencyTable::new(1, 3).unwrap();
        cont.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        cont.data_sets[0].cells[0][0] = Cell::number(10.0);
        cont.data_sets[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::Contingency(cont);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::Contingency(t) = arch {
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.data_sets[0].cells[2][0], Cell::number(20.0));
        }

        // Survival
        let mut surv = SurvivalTable::new(1, 3).unwrap();
        surv.times = vec![Cell::number(5.0), Cell::number(10.0), Cell::empty()];
        surv.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        surv.cohorts[0].cells[0][0] = Cell::number(1.0);
        surv.cohorts[0].cells[1][0] = Cell::number(0.0);
        let mut arch = TableArchetype::Survival(surv);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::Survival(t) = arch {
            assert_eq!(t.times[2], Cell::number(10.0));
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.cohorts[0].cells[2][0], Cell::number(0.0));
        }

        // PartsOfWhole
        let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
        pow.slice_labels = vec!["s0".into(), "s1".into(), String::new()];
        pow.data_sets[0].cells[0][0] = Cell::number(10.0);
        pow.data_sets[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::PartsOfWhole(pow);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::PartsOfWhole(t) = arch {
            assert_eq!(t.slice_labels, vec!["s0", "", "s1"]);
            assert_eq!(t.data_sets[0].cells[2][0], Cell::number(20.0));
        }

        // MultipleVariables
        let mut mv = MultipleVariablesTable::new(1, 3).unwrap();
        mv.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        mv.variables[0].cells[0] = Cell::number(10.0);
        mv.variables[0].cells[1] = Cell::number(20.0);
        let mut arch = TableArchetype::MultipleVariables(mv);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::MultipleVariables(t) = arch {
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.variables[0].cells[2], Cell::number(20.0));
        }

        // Nested
        let mut nested = NestedTable::new(1, 1, 3).unwrap();
        nested.row_titles = vec!["r0".into(), "r1".into(), String::new()];
        nested.groups[0].cells[0][0] = Cell::number(10.0);
        nested.groups[0].cells[1][0] = Cell::number(20.0);
        let mut arch = TableArchetype::Nested(nested);
        arch.shift_rows_down(1, 1, 2);
        if let TableArchetype::Nested(t) = arch {
            assert_eq!(t.row_titles, vec!["r0", "", "r1"]);
            assert_eq!(t.groups[0].cells[2][0], Cell::number(20.0));
        }
    }

    #[test]
    fn shift_rows_down_noops_on_zero_or_past_end() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
        col.row_titles = vec!["r0".into(), "r1".into()];
        let mut arch = TableArchetype::Column(col);
        arch.shift_rows_down(0, 0, 2);
        arch.shift_rows_down(2, 1, 2);
        arch.shift_rows_down(0, usize::MAX, 2);
        if let TableArchetype::Column(t) = arch {
            assert_eq!(t.row_titles, vec!["r0", "r1"]);
        }
    }
}
