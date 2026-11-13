//! The eight typed scientific data-table archetypes (`XyTable`, `ColumnTable`, `GroupedTable`,
//! `ContingencyTable`, `SurvivalTable`, `PartsOfWholeTable`, `MultipleVariablesTable`,
//! `NestedTable`) and their unified [`TableArchetype`] sum type (`docs/data-tables-and-transforms.md` §2).

use std::cmp::Ordering;

use crate::TableKind;
use crate::cell::{Cell, CellValue, ReplicateStacking, SubcolumnMode, SummarizedPoint};
use crate::error::{DocError, SummaryPointError, validate_data_sets, validate_rows, validate_subcolumns, validate_title};

/// Zero-based 3D cell address `(row, col, subcol)` inside a data table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CellCoord {
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

impl CellCoord {
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
#[derive(Debug, Clone, PartialEq)]
pub struct DataSetColumn {
    pub title: String,
    pub cells: Vec<Vec<Cell>>,
}

impl DataSetColumn {
    /// Allocate an empty `DataSetColumn` validated against dimension and title caps.
    pub fn new(title: impl Into<String>, row_count: usize, subcolumn_count: usize) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        validate_rows(row_count)?;
        validate_subcolumns(subcolumn_count)?;
        Ok(Self { title, cells: vec![vec![Cell::empty(); subcolumn_count]; row_count] })
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.cells.len()
    }

    #[must_use]
    pub fn subcolumn_count(&self) -> usize {
        self.cells.first().map_or(0, Vec::len)
    }

    #[must_use]
    pub fn cell(&self, row: usize, subcol: usize) -> Option<&Cell> {
        self.cells.get(row).and_then(|r| r.get(subcol))
    }

    pub fn cell_mut(&mut self, row: usize, subcol: usize) -> Option<&mut Cell> {
        self.cells.get_mut(row).and_then(|r| r.get_mut(subcol))
    }

    /// Grow row count to at least `min_rows` using `subcolumn_count` empty cells per added row.
    pub fn ensure_rows(&mut self, min_rows: usize, subcolumn_count: usize) -> Result<(), DocError> {
        validate_rows(min_rows)?;
        validate_subcolumns(subcolumn_count)?;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SequenceProgression {
    /// `X_r = start + r * step`.
    #[default]
    Arithmetic,
    /// `X_r = start * step^r`.
    Geometric,
}

/// Independent X-column configuration for an [`XyTable`] (`docs/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, PartialEq)]
pub enum XColumnMode {
    /// Explicit numeric X coordinates with optional error subcolumns (`Single`, `MeanSdN`, etc.).
    Numbers { subcolumn_mode: SubcolumnMode },
    /// Elapsed time or date coordinates accompanied by a unit label.
    DateOrElapsedTime { unit_label: String },
    /// Automatically generated arithmetic or geometric series (`X0`, `Delta X`).
    GeneratedSequence { start: f64, step: f64, progression: SequenceProgression },
}

impl Default for XColumnMode {
    fn default() -> Self {
        Self::Numbers { subcolumn_mode: SubcolumnMode::Single }
    }
}

impl XColumnMode {
    #[must_use]
    pub const fn subcolumn_count(&self) -> usize {
        match self {
            Self::Numbers { subcolumn_mode } => subcolumn_mode.subcolumn_count(),
            Self::DateOrElapsedTime { .. } => 1,
            Self::GeneratedSequence { .. } => 0,
        }
    }
}

/// Archetype 1: Continuous independent X variable paired with one or more Y response series.
#[derive(Debug, Clone, PartialEq)]
pub struct XyTable {
    pub x_title: String,
    pub x_mode: XColumnMode,
    pub x_cells: Vec<Vec<Cell>>,
    pub row_titles: Vec<String>,
    pub row_titles_expanded: bool,
    pub y_subcolumn_mode: SubcolumnMode,
    pub data_sets: Vec<DataSetColumn>,
}

impl XyTable {
    pub fn new(y_subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        Self::new_with_x_mode(XColumnMode::default(), y_subcolumn_mode, data_set_count, row_count)
    }

    /// Construct an `XyTable` with explicit X-column mode (including optional X error subcolumns).
    pub fn new_with_x_mode(x_mode: XColumnMode, y_subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        if let XColumnMode::Numbers { subcolumn_mode } = x_mode {
            subcolumn_mode.validate()?;
        }
        y_subcolumn_mode.validate()?;
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let x_subs = x_mode.subcolumn_count();
        let y_subs = y_subcolumn_mode.subcolumn_count();
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
}

// ============================================================================
// 2. Column Table
// ============================================================================

/// Experimental row-index semantics for a [`ColumnTable`] (`docs/data-tables-and-transforms.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnTable {
    pub design_hint: ColumnDesignHint,
    pub subcolumn_mode: SubcolumnMode,
    pub row_titles: Vec<String>,
    pub row_titles_expanded: bool,
    pub data_sets: Vec<DataSetColumn>,
}

impl ColumnTable {
    pub fn new(design_hint: ColumnDesignHint, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let subcolumn_mode = match design_hint {
            ColumnDesignHint::UnpairedReplicates | ColumnDesignHint::PairedRepeatedMeasures => SubcolumnMode::Single,
            ColumnDesignHint::PrecomputedError => SubcolumnMode::MeanSdN,
        };
        let subs = subcolumn_mode.subcolumn_count();
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

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data_sets.first().map_or(self.row_titles.len(), DataSetColumn::row_count)
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
        if cols.is_empty() {
            return Vec::new();
        }
        let rows = self.row_count();
        let mut blocks = Vec::new();
        for r in 0..rows {
            let mut row_vals = Vec::with_capacity(cols.len());
            for &c in cols {
                let Some(val) = self.data_sets.get(c).and_then(|ds| ds.cell(r, 0)).and_then(Cell::active_number) else {
                    row_vals.clear();
                    break;
                };
                row_vals.push(val);
            }
            if row_vals.len() == cols.len() {
                blocks.push(row_vals);
            }
        }
        blocks
    }
}

// ============================================================================
// 3. Grouped Table
// ============================================================================

/// Optional `2 × 2 × K` Three-Way ANOVA column factor metadata mapping 4 data-set columns
/// `((F1_0, F2_0), (F1_0, F2_1), (F1_1, F2_0), (F1_1, F2_1))` (`docs/data-tables-and-transforms.md` §2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeWayColumnGrouping {
    pub factor_1_name: String,
    pub factor_1_levels: [String; 2],
    pub factor_2_name: String,
    pub factor_2_levels: [String; 2],
}

/// Archetype 3: Two-factor (or `2 × 2 × K` three-factor) tensor `(Row r × Column c × Subcolumn s)`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupedTable {
    pub row_factor_title: String,
    pub column_factor_title: String,
    pub row_titles: Vec<String>,
    pub row_numeric_spacing: Option<Vec<Option<f64>>>,
    pub subcolumn_mode: SubcolumnMode,
    pub replicate_stacking: ReplicateStacking,
    pub three_way_grouping: Option<ThreeWayColumnGrouping>,
    pub data_sets: Vec<DataSetColumn>,
}

impl GroupedTable {
    pub fn new(subcolumn_mode: SubcolumnMode, data_set_count: usize, row_count: usize) -> Result<Self, DocError> {
        subcolumn_mode.validate()?;
        if matches!(subcolumn_mode, SubcolumnMode::Single) {
            return Err(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Grouped, mode: subcolumn_mode });
        }
        validate_data_sets(data_set_count)?;
        validate_rows(row_count)?;
        let subs = subcolumn_mode.subcolumn_count();
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
}

// ============================================================================
// 4. Contingency Table
// ============================================================================

/// Archetype 4: Categorical cross-tabulation of non-negative integer counts (`R × C`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContingencyTable {
    pub row_factor_title: String,
    pub column_factor_title: String,
    pub row_titles: Vec<String>,
    pub data_sets: Vec<DataSetColumn>,
}

impl ContingencyTable {
    pub fn new(column_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(column_count)?;
        validate_rows(row_count)?;
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
}

// ============================================================================
// 5. Survival Table
// ============================================================================

/// Subject endpoint status in a [`SurvivalTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SurvivalStatus {
    Event,
    Censored,
}

/// Validated time-to-event record for a single subject.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurvivalObservation {
    pub row: usize,
    pub time: f64,
    pub status: SurvivalStatus,
}

/// Archetype 5: Time-to-event survival table (`t >= 0`, configurable event/censored status codes).
#[derive(Debug, Clone, PartialEq)]
pub struct SurvivalTable {
    pub time_title: String,
    pub times: Vec<Cell>,
    pub event_code: i64,
    pub censored_code: i64,
    pub row_titles: Vec<String>,
    pub row_titles_expanded: bool,
    pub cohorts: Vec<DataSetColumn>,
}

impl SurvivalTable {
    pub fn new(cohort_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(cohort_count)?;
        validate_rows(row_count)?;
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
}

// ============================================================================
// 6. Parts of Whole Table
// ============================================================================

/// Validated non-negative slice observation and normalized fraction of total.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceFraction {
    pub row: usize,
    pub label: String,
    pub value: f64,
    pub fraction: f64,
}

/// Archetype 6: Mutually exclusive categorical slices summing to a whole.
#[derive(Debug, Clone, PartialEq)]
pub struct PartsOfWholeTable {
    pub slice_labels: Vec<String>,
    pub data_sets: Vec<DataSetColumn>,
}

impl PartsOfWholeTable {
    pub fn new(series_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(series_count)?;
        validate_rows(row_count)?;
        let mut data_sets = Vec::with_capacity(series_count);
        for idx in 0..series_count {
            data_sets.push(DataSetColumn::new(default_column_label(idx), row_count, 1)?);
        }
        Ok(Self { slice_labels: vec![String::new(); row_count], data_sets })
    }

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
}

// ============================================================================
// 7. Multiple Variables Table
// ============================================================================

/// Typed semantic role of a column in a [`MultipleVariablesTable`] (`docs/data-tables-and-transforms.md` §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VariableRole {
    #[default]
    Continuous,
    Categorical,
    Label,
    Calculated,
}

/// Origin of values in a [`VariableColumn`] (`docs/data-tables-and-transforms.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VariableSource {
    #[default]
    Entered,
    /// Formula stored normalized without a leading `=` character.
    Calculated { formula: String },
}

/// Ordering rule for distinct levels of a `Categorical` variable (`docs/data-tables-and-transforms.md` §2.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LevelOrderSpec {
    pub order: LevelOrderKind,
    pub reversed: bool,
}

/// Frequency summary for a single categorical level in a [`VariableColumn`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoricalLevelSummary {
    pub level: String,
    pub included_count: usize,
    pub excluded_count: usize,
}

/// Single named variable column in a [`MultipleVariablesTable`].
#[derive(Debug, Clone, PartialEq)]
pub struct VariableColumn {
    pub title: String,
    pub role: VariableRole,
    /// `true` when `role` was auto-inferred from entered tokens (muted badge); `false` once
    /// explicitly confirmed by the user (accented badge).
    pub role_inferred: bool,
    pub source: VariableSource,
    pub level_order: LevelOrderSpec,
    pub cells: Vec<Cell>,
}

impl VariableColumn {
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
    /// (`table.recodeLevels`; `docs/data-tables-and-transforms.md` §2.3), returning the number of
    /// rewritten cells.
    pub fn recode_levels(&mut self, mappings: &[(String, String)]) -> usize {
        if mappings.is_empty() {
            return 0;
        }
        let mut changed = 0;
        for cell in &mut self.cells {
            let Some(current) = cell_categorical_token(&cell.value) else {
                continue;
            };
            for (from, to) in mappings {
                if current == *from && from != to {
                    cell.value = CellValue::Text(to.clone());
                    changed += 1;
                    break;
                }
            }
        }
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
        for cell in &self.cells {
            let Some(tok) = cell_categorical_token(&cell.value) else {
                continue;
            };
            if let Some((_, existing)) = entries.iter_mut().find(|(_, item)| item.level == tok) {
                if cell.excluded {
                    existing.excluded_count += 1;
                } else {
                    existing.included_count += 1;
                }
            } else {
                let rank = entries.len();
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
                entries.sort_by(|(ra, a), (rb, b)| {
                    let pa = order_list.iter().position(|x| x == &a.level);
                    let pb = order_list.iter().position(|x| x == &b.level);
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
#[derive(Debug, Clone, PartialEq)]
pub struct MultipleVariablesTable {
    pub row_titles: Vec<String>,
    pub row_titles_expanded: bool,
    pub variables: Vec<VariableColumn>,
}

impl MultipleVariablesTable {
    pub fn new(variable_count: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(variable_count)?;
        validate_rows(row_count)?;
        let mut variables = Vec::with_capacity(variable_count);
        for idx in 0..variable_count {
            variables.push(VariableColumn::new(format!("Variable {}", default_column_label(idx)), row_count)?);
        }
        Ok(Self { row_titles: vec![String::new(); row_count], row_titles_expanded: false, variables })
    }

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
}

// ============================================================================
// 8. Nested Table
// ============================================================================

/// Experimental group (`A`, `B`, ...) containing biological-unit subcolumns (`A:1`, `A:2`, ...)
/// with technical replicates stacked down rows (`docs/data-tables-and-transforms.md` §2).
#[derive(Debug, Clone, PartialEq)]
pub struct NestedGroupColumn {
    pub title: String,
    pub subcolumn_titles: Vec<String>,
    pub cells: Vec<Vec<Cell>>,
}

impl NestedGroupColumn {
    pub fn new(title: impl Into<String>, biological_unit_count: usize, row_count: usize) -> Result<Self, DocError> {
        let title = title.into();
        validate_title(&title)?;
        validate_subcolumns(biological_unit_count)?;
        validate_rows(row_count)?;
        let subcolumn_titles = (1..=biological_unit_count).map(|i| format!("{title}:{i}")).collect();
        Ok(Self { title, subcolumn_titles, cells: vec![vec![Cell::empty(); biological_unit_count]; row_count] })
    }

    #[must_use]
    pub fn biological_unit_count(&self) -> usize {
        self.subcolumn_titles.len()
    }
}

/// Archetype 8: Hierarchical two-level design (experimental groups -> biological units -> technical replicates).
#[derive(Debug, Clone, PartialEq)]
pub struct NestedTable {
    pub row_titles: Vec<String>,
    pub row_titles_expanded: bool,
    pub groups: Vec<NestedGroupColumn>,
}

impl NestedTable {
    pub fn new(group_count: usize, biological_units_per_group: usize, row_count: usize) -> Result<Self, DocError> {
        validate_data_sets(group_count)?;
        validate_subcolumns(biological_units_per_group)?;
        validate_rows(row_count)?;
        let mut groups = Vec::with_capacity(group_count);
        for idx in 0..group_count {
            groups.push(NestedGroupColumn::new(default_column_label(idx), biological_units_per_group, row_count)?);
        }
        Ok(Self { row_titles: vec![String::new(); row_count], row_titles_expanded: false, groups })
    }

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
}

// ============================================================================
// Unified TableArchetype Sum Type
// ============================================================================

/// Strongly typed table payload carried by every [`crate::DataSheet`] (`docs/data-tables-and-transforms.md` §1.1 & §2).
#[derive(Debug, Clone, PartialEq)]
pub enum TableArchetype {
    Xy(XyTable),
    Column(ColumnTable),
    Grouped(GroupedTable),
    Contingency(ContingencyTable),
    Survival(SurvivalTable),
    PartsOfWhole(PartsOfWholeTable),
    MultipleVariables(MultipleVariablesTable),
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
}
