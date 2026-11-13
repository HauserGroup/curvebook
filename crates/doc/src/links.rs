//! Provenance-tracked `LinkedDataRegion` live links, four-stage `ImportLinkSpec` pipeline, and
//! [`DataSheet`] (with `frozen: bool` mutation guards) (`docs/data-tables-and-transforms.md` §1.4 & §6).

use lumencraft_palette::Rgba8;

use crate::TableKind;
use crate::archetypes::{CellCoord, TableArchetype, default_column_label};
use crate::cell::{Cell, SubcolumnMode};
use crate::error::{DocError, MAX_SUBCOLUMNS, validate_subcolumns, validate_title};
use crate::ids::{FloatingNote, SheetId};

/// Inclusive rectangular sub-grid range `(start ..= end)` inside a data or result sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub struct SheetCellRange {
    pub start: CellCoord,
    pub end: CellCoord,
}

impl SheetCellRange {
    #[must_use]
    pub const fn new(start: CellCoord, end: CellCoord) -> Self {
        Self { start, end }
    }
}

/// Classification of an internal upstream sheet referenced by a live [`LinkedDataRegion`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum InternalLinkKind {
    /// Upstream `DataSheet` cell range (rendered with a blue provenance border).
    DataSheet,
    /// Upstream analyzable `ResultSheet` such as Transform, Normalize, or Transpose (green border).
    AnalyzableResultSheet,
    /// Terminal statistical or regression `ResultSheet` parameter cell (red border).
    TerminalResultSheet,
}

/// Provenance border colour rendered around a [`LinkedDataRegion`] in the spreadsheet grid
/// (`docs/data-tables-and-transforms.md` §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum LinkBorderColor {
    /// Live link to an internal `DataSheet` cell range.
    Blue,
    /// Live link to an analyzable `ResultSheet` (`Transform`, `Normalize`, `Transpose`).
    Green,
    /// Live link to a terminal statistical / regression `ResultSheet` cell.
    Red,
    /// Live link to an external delimited (`CSV` / `TSV`) file on disk.
    Charcoal,
}

/// Stage 1 of [`ImportLinkSpec`]: data origin (`docs/data-tables-and-transforms.md` §6.1).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LinkSource {
    Clipboard,
    InternalSheet { sheet_id: SheetId, kind: InternalLinkKind, tab_id: Option<String>, range: SheetCellRange },
    ExternalDelimitedFile { path: String, delimiter: char, decimal_separator: char },
}

impl LinkSource {
    /// Provenance border badge colour (`None` for one-shot unlinked clipboard paste).
    #[must_use]
    pub const fn border_color(&self) -> Option<LinkBorderColor> {
        match self {
            Self::Clipboard => None,
            Self::InternalSheet { kind: InternalLinkKind::DataSheet, .. } => Some(LinkBorderColor::Blue),
            Self::InternalSheet { kind: InternalLinkKind::AnalyzableResultSheet, .. } => Some(LinkBorderColor::Green),
            Self::InternalSheet { kind: InternalLinkKind::TerminalResultSheet, .. } => Some(LinkBorderColor::Red),
            Self::ExternalDelimitedFile { .. } => Some(LinkBorderColor::Charcoal),
        }
    }

    /// Upstream internal `SheetId` when this link participates in the project DAG.
    #[must_use]
    pub const fn upstream_sheet_id(&self) -> Option<&SheetId> {
        match self {
            Self::InternalSheet { sheet_id, .. } => Some(sheet_id),
            Self::Clipboard | Self::ExternalDelimitedFile { .. } => None,
        }
    }
}

/// Stage 2 of [`ImportLinkSpec`]: slicing, periodic decimation, unstacking, and `*` exclusion parsing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportFilterSpec {
    pub row_window: Option<(usize, usize)>,
    pub col_window: Option<(usize, usize)>,
    pub decimate_every_k: Option<usize>,
    pub unstack_group_column: Option<usize>,
    pub parse_asterisk_exclusions: bool,
}

impl Default for ImportFilterSpec {
    fn default() -> Self {
        Self { row_window: None, col_window: None, decimate_every_k: None, unstack_group_column: None, parse_asterisk_exclusions: true }
    }
}

/// Placement mode at anchor cell `(row, col, subcol)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum PlacementMode {
    #[default]
    Overwrite,
    InsertRows,
    AppendRows,
    AppendColumns,
}

/// Stage 3 of [`ImportLinkSpec`]: placement anchor, blank-skipping, header promotion, and reactive
/// transposition (`Grouped` <-> `Column`; `docs/data-tables-and-transforms.md` §6.3).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ImportPlacementSpec {
    pub mode: PlacementMode,
    pub anchor: CellCoord,
    pub propagate_title_header: bool,
    pub skip_contiguous_blanks: bool,
    pub transpose: bool,
}

/// Complete four-stage `Import & Paste Special` / `Paste Link` specification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportLinkSpec {
    pub source: LinkSource,
    pub filter: ImportFilterSpec,
    pub placement: ImportPlacementSpec,
}

/// Live provenance-tracked cell region embedded inside a [`DataSheet`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LinkedDataRegion {
    pub id: String,
    pub spec: ImportLinkSpec,
    pub target_range: SheetCellRange,
    pub provenance_summary: String,
}

impl LinkedDataRegion {
    /// Grid border colour derived from the link source.
    #[must_use]
    pub const fn border_color(&self) -> Option<LinkBorderColor> {
        self.spec.source.border_color()
    }
}

/// Project data-table sheet pairing metadata, freeze state, live linked regions, and floating notes
/// with one of the eight [`TableArchetype`] schemas (`docs/data-tables-and-transforms.md` §1).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DataSheet {
    pub id: SheetId,
    pub title: String,
    pub archetype: TableArchetype,
    /// When `true`, cell edits, pastes, structural edits, and formula changes return
    /// [`DocError::SheetFrozen`] while read-only inspection, analyses, and graphs remain active.
    pub frozen: bool,
    pub info_sheet_id: Option<SheetId>,
    pub linked_regions: Vec<LinkedDataRegion>,
    pub floating_notes: Vec<FloatingNote>,
    pub color_tag: Option<Rgba8>,
}

impl DataSheet {
    /// Construct a validated, unfrozen `DataSheet`.
    pub fn new(id: impl Into<SheetId>, title: impl Into<String>, archetype: TableArchetype) -> Result<Self, DocError> {
        let id = id.into();
        if id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        let title = title.into();
        validate_title(&title)?;
        Ok(Self { id, title, archetype, frozen: false, info_sheet_id: None, linked_regions: Vec::new(), floating_notes: Vec::new(), color_tag: None })
    }

    #[must_use]
    pub const fn kind(&self) -> TableKind {
        self.archetype.kind()
    }

    /// Guard helper returning `Err(DocError::SheetFrozen)` when `self.frozen` is `true`.
    pub fn ensure_mutable(&self) -> Result<(), DocError> {
        if self.frozen {
            return Err(DocError::SheetFrozen { sheet_id: self.id.clone() });
        }
        Ok(())
    }

    /// Rename this data sheet (permitted only when unfrozen and within [`crate::MAX_TITLE_BYTES`]).
    pub fn set_title(&mut self, title: impl Into<String>) -> Result<(), DocError> {
        self.ensure_mutable()?;
        let title = title.into();
        validate_title(&title)?;
        self.title = title;
        Ok(())
    }

    /// Read a data cell at `coord`.
    #[must_use]
    pub fn cell(&self, coord: CellCoord) -> Option<&Cell> {
        self.archetype.data_cell(coord)
    }

    /// Mutate a data cell at `coord` (enforces freeze state and bounds; refreshes inferred role on
    /// `MultipleVariables` columns).
    pub fn set_cell(&mut self, coord: CellCoord, cell: Cell) -> Result<(), DocError> {
        self.ensure_mutable()?;
        let Some(slot) = self.archetype.data_cell_mut(coord) else {
            return Err(DocError::CellOutOfBounds { row: coord.row, col: coord.col, subcol: coord.subcol });
        };
        *slot = cell;
        if let TableArchetype::MultipleVariables(mv) = &mut self.archetype
            && let Some(var) = mv.variables.get_mut(coord.col)
        {
            var.refresh_inferred_role();
        }
        Ok(())
    }

    /// Non-destructively set or clear the exclusion mask at `coord` (`table.excludeCells`).
    pub fn set_excluded(&mut self, coord: CellCoord, excluded: bool) -> Result<(), DocError> {
        self.ensure_mutable()?;
        let Some(slot) = self.archetype.data_cell_mut(coord) else {
            return Err(DocError::CellOutOfBounds { row: coord.row, col: coord.col, subcol: coord.subcol });
        };
        slot.excluded = excluded;
        Ok(())
    }

    /// Read an independent `X` (`Xy`) or elapsed-time (`Survival`, `subcol == 0`) cell.
    #[must_use]
    pub fn x_or_time_cell(&self, row: usize, subcol: usize) -> Option<&Cell> {
        self.archetype.x_or_time_cell(row, subcol)
    }

    /// Mutate an independent `X` (`Xy`) or elapsed-time (`Survival`, `subcol == 0`) cell
    /// (enforces freeze state and bounds).
    pub fn set_x_or_time_cell(&mut self, row: usize, subcol: usize, cell: Cell) -> Result<(), DocError> {
        self.ensure_mutable()?;
        let Some(slot) = self.archetype.x_or_time_cell_mut(row, subcol) else {
            return Err(DocError::CellOutOfBounds { row, col: 0, subcol });
        };
        *slot = cell;
        Ok(())
    }

    /// Non-destructively set or clear the exclusion mask on an independent `X` (`Xy`) or
    /// elapsed-time (`Survival`, `subcol == 0`) cell.
    pub fn set_x_or_time_excluded(&mut self, row: usize, subcol: usize, excluded: bool) -> Result<(), DocError> {
        self.ensure_mutable()?;
        let Some(slot) = self.archetype.x_or_time_cell_mut(row, subcol) else {
            return Err(DocError::CellOutOfBounds { row, col: 0, subcol });
        };
        slot.excluded = excluded;
        Ok(())
    }

    /// Attach a live [`LinkedDataRegion`] to this sheet (enforces freeze state).
    pub fn add_linked_region(&mut self, region: LinkedDataRegion) -> Result<(), DocError> {
        self.ensure_mutable()?;
        self.linked_regions.push(region);
        Ok(())
    }

    /// Extract a 2D slice of cells `[row][flat_subcol]` across inclusive `range` (`start ..= end`),
    /// clamped safely against sheet dimensions and preserving raw values and `excluded` masks
    /// losslessly for cross-table `Paste Link`.
    #[must_use]
    pub fn extract_cell_grid(&self, range: SheetCellRange) -> Vec<Vec<Cell>> {
        let row_count = self.archetype.row_count();
        let col_count = self.archetype.column_count();
        if row_count == 0
            || col_count == 0
            || range.end.row < range.start.row
            || range.end.col < range.start.col
            || range.start.row >= row_count
            || range.start.col >= col_count
        {
            return Vec::new();
        }
        let end_row = range.end.row.min(row_count - 1);
        let end_col = range.end.col.min(col_count - 1);
        let mut out = Vec::with_capacity(end_row - range.start.row + 1);
        for r in range.start.row..=end_row {
            let mut row_cells = Vec::new();
            for c in range.start.col..=end_col {
                let max_sub = subcolumn_upper_index(&self.archetype, c);
                let s_min = if c == range.start.col { range.start.subcol } else { 0 };
                let s_max = if c == range.end.col { range.end.subcol.min(max_sub) } else { max_sub };
                if s_max < s_min {
                    continue;
                }
                for s in s_min..=s_max {
                    row_cells.push(self.cell(CellCoord::new(r, c, s)).cloned().unwrap_or_default());
                }
            }
            if row_cells.is_empty() {
                return Vec::new();
            }
            out.push(row_cells);
        }
        out
    }

    /// Clone this sheet's archetype structure for `sheet.clonePipeline` ("Clone Family"), optionally
    /// clearing Y values, titles, or overriding raw replicate count (`docs/data-tables-and-transforms.md` §7).
    pub fn clone_structure(&self, new_id: impl Into<SheetId>, opts: &CloneDataSheetOptions) -> Result<Self, DocError> {
        let new_id = new_id.into();
        if new_id.as_str().trim().is_empty() {
            return Err(DocError::EmptyIdentifier);
        }
        validate_title(&opts.title)?;
        if let Some(n) = opts.override_replicates {
            validate_subcolumns(n)?;
        }

        let mut cloned = self.clone();
        cloned.id = new_id;
        cloned.title = opts.title.clone();
        cloned.frozen = false;
        cloned.linked_regions.clear();
        cloned.floating_notes.clear();
        apply_clone_options(&mut cloned.archetype, opts)?;
        Ok(cloned)
    }
}

/// Options for [`DataSheet::clone_structure`] (`sheet.clonePipeline`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CloneDataSheetOptions {
    pub title: String,
    pub keep_x_values: bool,
    pub clear_y_values: bool,
    pub clear_column_titles: bool,
    pub clear_row_titles: bool,
    pub override_replicates: Option<usize>,
}

/// Losslessly transpose a 2D cell grid (`rows × cols` -> `cols × rows`) while preserving `excluded` flags.
#[must_use]
pub fn transpose_cell_grid(grid: &[Vec<Cell>]) -> Vec<Vec<Cell>> {
    let max_cols = grid.iter().map(Vec::len).max().unwrap_or(0);
    if max_cols == 0 {
        return Vec::new();
    }
    let mut out = vec![vec![Cell::empty(); grid.len()]; max_cols];
    for (r, row) in grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(target_row) = out.get_mut(c)
                && let Some(slot) = target_row.get_mut(r)
            {
                *slot = cell.clone();
            }
        }
    }
    out
}

fn subcolumn_upper_index(arch: &TableArchetype, col: usize) -> usize {
    let count = match arch {
        TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
        TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
        TableArchetype::Nested(t) => t.groups.get(col).map_or(0, |g| g.biological_unit_count()),
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => 1,
    };
    count.min(MAX_SUBCOLUMNS).saturating_sub(1)
}

fn apply_clone_options(arch: &mut TableArchetype, opts: &CloneDataSheetOptions) -> Result<(), DocError> {
    match arch {
        TableArchetype::Xy(t) => {
            if !opts.keep_x_values {
                clear_2d_cells(&mut t.x_cells);
            }
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            if let Some(n) = opts.override_replicates {
                t.y_subcolumn_mode = SubcolumnMode::RawReplicates(n);
            }
            let subs = t.y_subcolumn_mode.subcolumn_count();
            reset_data_sets(&mut t.data_sets, subs, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::Column(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            let subs = t.subcolumn_mode.subcolumn_count();
            reset_data_sets(&mut t.data_sets, subs, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::Grouped(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            if let Some(n) = opts.override_replicates {
                t.subcolumn_mode = SubcolumnMode::RawReplicates(n);
            }
            let subs = t.subcolumn_mode.subcolumn_count();
            reset_data_sets(&mut t.data_sets, subs, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::Contingency(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            reset_data_sets(&mut t.data_sets, 1, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::Survival(t) => {
            if !opts.keep_x_values {
                for c in &mut t.times {
                    *c = Cell::empty();
                }
            }
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            reset_data_sets(&mut t.cohorts, 1, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::PartsOfWhole(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.slice_labels);
            }
            reset_data_sets(&mut t.data_sets, 1, opts.clear_y_values, opts.clear_column_titles)?;
        }
        TableArchetype::MultipleVariables(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            for (idx, var) in t.variables.iter_mut().enumerate() {
                if opts.clear_column_titles {
                    var.title = format!("Variable {}", default_column_label(idx));
                }
                if opts.clear_y_values {
                    for c in &mut var.cells {
                        *c = Cell::empty();
                    }
                }
            }
        }
        TableArchetype::Nested(t) => {
            if opts.clear_row_titles {
                clear_strings(&mut t.row_titles);
            }
            for (idx, g) in t.groups.iter_mut().enumerate() {
                if opts.clear_column_titles {
                    g.title = default_column_label(idx);
                }
                let units = opts.override_replicates.unwrap_or_else(|| g.biological_unit_count());
                g.subcolumn_titles = (1..=units).map(|i| format!("{}:{i}", g.title)).collect();
                if opts.clear_y_values || opts.override_replicates.is_some() {
                    g.cells = vec![vec![Cell::empty(); units]; g.cells.len()];
                }
            }
        }
    }
    Ok(())
}

fn clear_2d_cells(grid: &mut [Vec<Cell>]) {
    for row in grid {
        for cell in row {
            *cell = Cell::empty();
        }
    }
}

fn clear_strings(titles: &mut [String]) {
    for s in titles {
        s.clear();
    }
}

fn reset_data_sets(
    data_sets: &mut [crate::archetypes::DataSetColumn],
    subcolumn_count: usize,
    clear_values: bool,
    clear_titles: bool,
) -> Result<(), DocError> {
    validate_subcolumns(subcolumn_count)?;
    for (idx, ds) in data_sets.iter_mut().enumerate() {
        if clear_titles {
            ds.title = default_column_label(idx);
        }
        if clear_values || ds.subcolumn_count() != subcolumn_count {
            ds.cells = vec![vec![Cell::empty(); subcolumn_count]; ds.row_count()];
        }
    }
    Ok(())
}
