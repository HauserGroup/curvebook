//! Spreadsheet grid viewport windowing, two-tier `(Group x Subcolumn)` header slicing,
//! pinned row-title geometry, block-edge navigation, rectangular quick statistics, and
//! TSV clipboard / command-intent emitters.

use std::cmp::Ordering;
use std::fmt;

use curvebook_doc::{
    Cell, CellCoord, CellValue, DataSheet, ImportLinkSpec, LinkBorderColor, LinkedDataRegion, MAX_DATA_SETS, MAX_ROWS, MAX_SUBCOLUMNS,
    SheetCellRange, SheetId, SubcolumnMode, TableArchetype, XColumnMode,
};
use curvebook_math::sum::SummaryMoments;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::Cursor;
use crate::canvas::ToolCommandIntent;

const DEFAULT_ROW_HEIGHT_PX: f64 = 24.0;
const DEFAULT_COL_WIDTH_PX: f64 = 96.0;
const ROW_NUMBER_GUTTER_WIDTH_PX: f64 = 48.0;
const ROW_TITLE_DEFAULT_WIDTH_PX: f64 = 112.0;
const MAX_CLIPBOARD_CELLS: usize = MAX_ROWS;

const fn default_decimal_separator() -> char {
    '.'
}

/// Zero-based 3D spreadsheet coordinate `(row, col, subcol)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
pub struct GridCellCoord {
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

impl GridCellCoord {
    /// Construct a cell coordinate.
    #[must_use]
    pub const fn new(row: usize, col: usize, subcol: usize) -> Self {
        Self { row, col, subcol }
    }
}

impl From<CellCoord> for GridCellCoord {
    fn from(coord: CellCoord) -> Self {
        Self { row: coord.row, col: coord.col, subcol: coord.subcol }
    }
}

impl From<GridCellCoord> for CellCoord {
    fn from(coord: GridCellCoord) -> Self {
        Self::new(coord.row, coord.col, coord.subcol)
    }
}

/// Physical/pixel scroll and cell geometry configuration for virtualized grid rendering.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridViewportSpec {
    pub scroll_x_px: f64,
    pub scroll_y_px: f64,
    pub viewport_width_px: f64,
    pub viewport_height_px: f64,
    pub default_col_width_px: f64,
    pub row_height_px: f64,
    pub frozen_header_rows: usize,
    pub frozen_row_title_cols: usize,
}

impl Default for GridViewportSpec {
    fn default() -> Self {
        Self {
            scroll_x_px: 0.0,
            scroll_y_px: 0.0,
            viewport_width_px: 800.0,
            viewport_height_px: 480.0,
            default_col_width_px: DEFAULT_COL_WIDTH_PX,
            row_height_px: DEFAULT_ROW_HEIGHT_PX,
            frozen_header_rows: 1,
            frozen_row_title_cols: 1,
        }
    }
}

impl GridViewportSpec {
    #[must_use]
    pub fn sanitized_row_height_px(&self) -> f64 {
        if self.row_height_px.is_finite() && self.row_height_px > 0.0 { self.row_height_px } else { DEFAULT_ROW_HEIGHT_PX }
    }

    #[must_use]
    pub fn sanitized_col_width_px(&self) -> f64 {
        if self.default_col_width_px.is_finite() && self.default_col_width_px > 0.0 { self.default_col_width_px } else { DEFAULT_COL_WIDTH_PX }
    }

    #[must_use]
    pub fn sanitized_viewport_width_px(&self) -> f64 {
        finite_non_neg(self.viewport_width_px)
    }

    #[must_use]
    pub fn sanitized_viewport_height_px(&self) -> f64 {
        finite_non_neg(self.viewport_height_px)
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn header_height_px(&self) -> f64 {
        let row_h = self.sanitized_row_height_px();
        (self.frozen_header_rows as f64) * row_h
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn default_pinned_left_width_px(&self) -> f64 {
        let col_w = self.sanitized_col_width_px();
        (self.frozen_row_title_cols as f64) * col_w
    }
}

/// Half-open visible `[start, end)` row and column slices computed for the current scroll offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GridVisibleWindow {
    pub start_row: usize,
    pub end_row: usize,
    pub start_col: usize,
    pub end_col: usize,
}

/// Pinned left-side geometry for row numbers, expandable `Row Titles`, and independent X/Time columns.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PinnedRowTitleGeometry {
    pub row_titles_expanded: bool,
    pub row_number_gutter_width_px: f64,
    pub row_title_col_x_px: f64,
    pub row_title_col_width_px: f64,
    pub x_or_time_col_count: usize,
    pub x_or_time_cols_x_px: f64,
    pub x_or_time_cols_width_px: f64,
    pub total_pinned_width_px: f64,
    pub header_height_px: f64,
}

/// Scrollbar bounds and sub-pixel scroll offsets for the scrollable data body.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct GridScrollMetrics {
    pub body_origin_x_px: f64,
    pub body_origin_y_px: f64,
    pub body_width_px: f64,
    pub body_height_px: f64,
    pub total_content_width_px: f64,
    pub total_content_height_px: f64,
    pub max_scroll_x_px: f64,
    pub max_scroll_y_px: f64,
    pub subrow_offset_px: f64,
    pub subcol_offset_px: f64,
}

/// Visible second-tier subcolumn header cell (`Y1`, `Y2`, `Mean`, `SD`, `A:1`, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleSubcolumnHeader {
    pub group_index: usize,
    pub subcol_index: usize,
    pub flat_col_index: usize,
    pub label: String,
    pub x_px: f64,
    pub width_px: f64,
}

/// Visible first-tier group header slice spanning `[subcol_start, subcol_end)` within a group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisibleGroupHeaderSlice {
    pub group_index: usize,
    pub title: String,
    pub flat_col_start: usize,
    pub flat_col_end: usize,
    pub subcol_start: usize,
    pub subcol_end: usize,
    pub x_px: f64,
    pub width_px: f64,
    pub subcolumns: Vec<VisibleSubcolumnHeader>,
}

/// Visible two-tier `(Group x Subcolumn)` header slices for the current horizontal scroll window.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GridTwoTierHeaderWindow {
    pub start_flat_col: usize,
    pub end_flat_col: usize,
    pub total_flat_cols: usize,
    pub groups: Vec<VisibleGroupHeaderSlice>,
}

/// Formatted cell entry produced strictly for the visible viewport slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleCellView {
    pub coord: GridCellCoord,
    pub flat_col: usize,
    pub display_text: String,
    pub excluded: bool,
    pub link_border: Option<LinkBorderColor>,
    pub is_selected: bool,
    pub is_active: bool,
}

/// Complete virtualized frame payload for one 60-fps render pass.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GridVisibleWindowFrame {
    pub window: GridVisibleWindow,
    pub pinned: PinnedRowTitleGeometry,
    pub scroll: GridScrollMetrics,
    pub headers: GridTwoTierHeaderWindow,
    pub cells: Vec<VisibleCellView>,
}

/// Cardinal directions for keyboard cell navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GridDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Keyboard navigation actions supported in the virtualized spreadsheet grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GridNavAction {
    Arrow { dir: GridDirection, extend: bool, jump_block: bool },
    Tab { backward: bool },
    Enter { backward: bool },
    Home { to_table_start: bool, extend: bool },
    End { to_table_end: bool, extend: bool },
    PageUp { extend: bool },
    PageDown { extend: bool },
}

/// Error returned by clipboard parsing or intent helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GridToolError {
    ClipboardTooLarge { cell_count: usize, max: usize },
    EmptyClipboard,
    NoSelection,
}

impl fmt::Display for GridToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardTooLarge { cell_count, max } => {
                write!(f, "clipboard paste contains {cell_count} cells, exceeding cap {max}")
            }
            Self::EmptyClipboard => f.write_str("clipboard payload contains no cells"),
            Self::NoSelection => f.write_str("no active grid selection"),
        }
    }
}

impl std::error::Error for GridToolError {}

/// Live summary statistics displayed in the status bar for the active cell selection.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SelectionQuickStats {
    pub selected_cells: usize,
    pub numeric_count: usize,
    pub excluded_count: usize,
    pub sum: Option<f64>,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
    #[serde(default)]
    pub sem: Option<f64>,
    #[serde(default)]
    pub median: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl SelectionQuickStats {
    /// Compute non-destructive summary statistics across inclusive rectangular range `[a, b]` on `sheet`,
    /// skipping excluded, empty, non-numeric, and non-finite cells without sorting `NaN`s.
    #[must_use]
    pub fn from_data_sheet(sheet: &DataSheet, a: GridCellCoord, b: GridCellCoord) -> Self {
        let Some((start_row, end_row, start_flat, end_flat)) = normalized_flat_rect(sheet, a, b) else {
            return Self::default();
        };

        let mut selected_cells = 0usize;
        let mut excluded_count = 0usize;
        let mut values = Vec::new();
        let flat_cols: Vec<(usize, usize)> = (start_flat..=end_flat).filter_map(|fc| GridToolState::flat_col_to_coord(sheet, fc)).collect();

        for row in start_row..=end_row {
            for &(col, subcol) in &flat_cols {
                selected_cells += 1;
                let Some(cell) = sheet.cell(CellCoord::new(row, col, subcol)) else {
                    continue;
                };
                if cell.excluded {
                    excluded_count += 1;
                    continue;
                }
                if let CellValue::Number(v) = cell.value
                    && v.is_finite()
                {
                    values.push(v);
                }
            }
        }

        Self::from_finite_values(selected_cells, excluded_count, &mut values)
    }

    fn from_finite_values(selected_cells: usize, excluded_count: usize, values: &mut Vec<f64>) -> Self {
        values.retain(|v| v.is_finite());
        if values.is_empty() {
            return Self {
                selected_cells,
                numeric_count: 0,
                excluded_count,
                sum: None,
                mean: None,
                sd: None,
                sem: None,
                median: None,
                min: None,
                max: None,
            };
        }

        let numeric_count = values.len();
        let (sum, mean, sd, sem) = match SummaryMoments::from_slice(values) {
            Ok(moments) => {
                let sd = moments.sample_sd().ok().filter(|v| v.is_finite());
                let sem = moments.sem().ok().filter(|v| v.is_finite());
                (Some(moments.sum), Some(moments.mean), sd, sem)
            }
            Err(_) => (None, None, None, None),
        };

        values.sort_by(f64::total_cmp);
        let min = values.first().copied();
        let max = values.last().copied();
        let median = compute_sorted_median(values);

        Self { selected_cells, numeric_count, excluded_count, sum, mean, sd, sem, median, min, max }
    }
}

fn compute_sorted_median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    if n % 2 == 1 {
        return sorted.get(n / 2).copied();
    }
    let lo = sorted.get(n / 2 - 1).copied()?;
    let hi = sorted.get(n / 2).copied()?;
    let diff = hi - lo;
    let mid = if diff.is_finite() { lo + 0.5 * diff } else { 0.5 * lo + 0.5 * hi };
    mid.is_finite().then_some(mid)
}

/// Interactive state machine for spreadsheet selection, inline cell editing, and viewport scrolling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridToolState {
    pub anchor: Option<GridCellCoord>,
    pub active: Option<GridCellCoord>,
    pub editing_buffer: Option<String>,
    pub viewport: GridViewportSpec,
    pub pending_intents: Vec<ToolCommandIntent>,
    #[serde(default)]
    pub hover_column_divider: Option<usize>,
    #[serde(default = "default_decimal_separator")]
    pub decimal_separator: char,
}

impl Default for GridToolState {
    fn default() -> Self {
        Self {
            anchor: None,
            active: None,
            editing_buffer: None,
            viewport: GridViewportSpec::default(),
            pending_intents: Vec::new(),
            hover_column_divider: None,
            decimal_separator: '.',
        }
    }
}

impl GridToolState {
    /// Cursor shape appropriate for spreadsheet cell editing or column-divider resizing.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        if self.hover_column_divider.is_some() { Cursor::AxisResize } else { Cursor::CellGrid }
    }

    /// Move both selection anchor and active cursor to `coord`.
    pub fn select_single(&mut self, coord: GridCellCoord) {
        self.anchor = Some(coord);
        self.active = Some(coord);
        self.editing_buffer = None;
    }

    /// Extend active selection endpoint to `coord` while preserving `anchor` (or setting `anchor = coord`).
    pub fn extend_selection(&mut self, coord: GridCellCoord) {
        if self.anchor.is_none() {
            self.anchor = Some(coord);
        }
        self.active = Some(coord);
        self.editing_buffer = None;
    }

    /// Return `(min_coord, max_coord)` bounds of the active selection in flat reading order, if any.
    #[must_use]
    pub fn selection_bounds(&self) -> Option<(GridCellCoord, GridCellCoord)> {
        let a = self.active?;
        let b = self.anchor.unwrap_or(a);
        let min_row = a.row.min(b.row);
        let max_row = a.row.max(b.row);
        let (min_col, min_sub, max_col, max_sub) =
            if (a.col, a.subcol) <= (b.col, b.subcol) { (a.col, a.subcol, b.col, b.subcol) } else { (b.col, b.subcol, a.col, a.subcol) };
        Some((GridCellCoord::new(min_row, min_col, min_sub), GridCellCoord::new(max_row, max_col, max_sub)))
    }

    /// Return exact component-wise 3-D bounding box `(min_coord, max_coord)` matching engine box semantics.
    #[must_use]
    pub fn selection_box_bounds(&self) -> Option<(GridCellCoord, GridCellCoord)> {
        let a = self.active?;
        let b = self.anchor.unwrap_or(a);
        Some((
            GridCellCoord::new(a.row.min(b.row), a.col.min(b.col), a.subcol.min(b.subcol)),
            GridCellCoord::new(a.row.max(b.row), a.col.max(b.col), a.subcol.max(b.subcol)),
        ))
    }

    /// Compute the currently visible `[start, end)` row/column window clamped to table bounds in O(1).
    #[must_use]
    pub fn visible_window(&self, total_rows: usize, total_cols: usize) -> GridVisibleWindow {
        self.visible_window_with_chrome(total_rows, total_cols, self.viewport.default_pinned_left_width_px(), self.viewport.header_height_px())
    }

    /// Compute the O(1) visible window using exact sheet-derived pinned geometry.
    #[must_use]
    pub fn visible_window_for_sheet(&self, sheet: &DataSheet) -> GridVisibleWindow {
        let pinned = self.pinned_row_title_geometry(sheet);
        let total_rows = sheet.archetype.row_count();
        let total_cols = Self::sheet_total_flat_cols(sheet);
        self.visible_window_with_chrome(total_rows, total_cols, pinned.total_pinned_width_px, pinned.header_height_px)
    }

    fn visible_window_with_chrome(
        &self,
        total_rows: usize,
        total_cols: usize,
        pinned_left_width_px: f64,
        header_height_px: f64,
    ) -> GridVisibleWindow {
        let row_h = self.viewport.sanitized_row_height_px();
        let col_w = self.viewport.sanitized_col_width_px();
        let scroll_y = finite_non_neg(self.viewport.scroll_y_px);
        let scroll_x = finite_non_neg(self.viewport.scroll_x_px);
        let view_h = self.viewport.sanitized_viewport_height_px();
        let view_w = self.viewport.sanitized_viewport_width_px();

        let body_h = (view_h - finite_non_neg(header_height_px)).max(0.0);
        let body_w = (view_w - finite_non_neg(pinned_left_width_px)).max(0.0);

        let (start_row, end_row) = compute_visible_axis_range(scroll_y, body_h, row_h, total_rows);
        let (start_col, end_col) = compute_visible_axis_range(scroll_x, body_w, col_w, total_cols);

        GridVisibleWindow { start_row, end_row, start_col, end_col }
    }

    /// Total flattened data subcolumns across all groups/variables in `sheet`.
    #[must_use]
    pub fn sheet_total_flat_cols(sheet: &DataSheet) -> usize {
        match &sheet.archetype {
            TableArchetype::Xy(t) => t.data_sets.len().saturating_mul(t.y_subcolumn_mode.subcolumn_count()),
            TableArchetype::Column(t) => t.data_sets.len().saturating_mul(t.subcolumn_mode.subcolumn_count()),
            TableArchetype::Grouped(t) => t.data_sets.len().saturating_mul(t.subcolumn_mode.subcolumn_count()),
            TableArchetype::Contingency(t) => t.data_sets.len(),
            TableArchetype::Survival(t) => t.cohorts.len(),
            TableArchetype::PartsOfWhole(t) => t.data_sets.len(),
            TableArchetype::MultipleVariables(t) => t.variables.len(),
            TableArchetype::Nested(t) => t.groups.iter().fold(0usize, |acc, g| acc.saturating_add(g.biological_unit_count())),
        }
    }

    /// Number of subcolumns inside group `col` of `sheet`.
    #[must_use]
    pub fn sheet_subcolumn_count_for_group(sheet: &DataSheet, col: usize) -> usize {
        if col >= sheet.archetype.column_count() {
            return 0;
        }
        match &sheet.archetype {
            TableArchetype::Xy(t) => t.y_subcolumn_mode.subcolumn_count(),
            TableArchetype::Column(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Grouped(t) => t.subcolumn_mode.subcolumn_count(),
            TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => {
                1
            }
            TableArchetype::Nested(t) => t.groups.get(col).map_or(0, |g| g.biological_unit_count()),
        }
    }

    /// Map `(col, subcol)` on `sheet` to its zero-based flat column index in O(1) for uniform modes.
    #[must_use]
    pub fn coord_to_flat_col(sheet: &DataSheet, coord: GridCellCoord) -> Option<usize> {
        let group_count = sheet.archetype.column_count();
        if coord.col >= group_count {
            return None;
        }
        match &sheet.archetype {
            TableArchetype::Nested(t) => {
                let mut offset = 0usize;
                for idx in 0..coord.col {
                    offset = offset.saturating_add(t.groups.get(idx)?.biological_unit_count());
                }
                let bio_count = t.groups.get(coord.col)?.biological_unit_count();
                (coord.subcol < bio_count).then_some(offset.saturating_add(coord.subcol))
            }
            _ => {
                let subs = Self::sheet_subcolumn_count_for_group(sheet, coord.col);
                (coord.subcol < subs).then_some(coord.col.saturating_mul(subs).saturating_add(coord.subcol))
            }
        }
    }

    /// Map zero-based `flat_col` back to `(col, subcol)` in O(1) for uniform modes.
    #[must_use]
    pub fn flat_col_to_coord(sheet: &DataSheet, flat_col: usize) -> Option<(usize, usize)> {
        match &sheet.archetype {
            TableArchetype::Nested(t) => {
                let mut cursor = 0usize;
                for (g_idx, group) in t.groups.iter().enumerate() {
                    let count = group.biological_unit_count();
                    let next = cursor.saturating_add(count);
                    if flat_col < next {
                        return Some((g_idx, flat_col - cursor));
                    }
                    cursor = next;
                }
                None
            }
            _ => {
                let total_groups = sheet.archetype.column_count();
                if total_groups == 0 {
                    return None;
                }
                let subs = Self::sheet_subcolumn_count_for_group(sheet, 0);
                if subs == 0 {
                    return None;
                }
                let col = flat_col / subs;
                let subcol = flat_col % subs;
                (col < total_groups).then_some((col, subcol))
            }
        }
    }

    /// Compute pinned left row-title / X / Time column geometry and two-tier header height in O(1).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn pinned_row_title_geometry(&self, sheet: &DataSheet) -> PinnedRowTitleGeometry {
        let row_h = self.viewport.sanitized_row_height_px();
        let col_w = self.viewport.sanitized_col_width_px();
        let row_titles_expanded = sheet_row_titles_expanded(sheet);
        let x_or_time_col_count = sheet_x_or_time_col_count(sheet);
        let row_title_col_width_px = if row_titles_expanded { ROW_TITLE_DEFAULT_WIDTH_PX } else { 0.0 };
        let x_or_time_cols_width_px = (x_or_time_col_count as f64) * col_w;
        let row_title_col_x_px = ROW_NUMBER_GUTTER_WIDTH_PX;
        let x_or_time_cols_x_px = row_title_col_x_px + row_title_col_width_px;
        let total_pinned_width_px = x_or_time_cols_x_px + x_or_time_cols_width_px;
        let header_tiers = if sheet_uses_two_tier_headers(sheet) { 2 } else { 1 };
        let header_height_px = (self.viewport.frozen_header_rows.max(header_tiers) as f64) * row_h;

        PinnedRowTitleGeometry {
            row_titles_expanded,
            row_number_gutter_width_px: ROW_NUMBER_GUTTER_WIDTH_PX,
            row_title_col_x_px,
            row_title_col_width_px,
            x_or_time_col_count,
            x_or_time_cols_x_px,
            x_or_time_cols_width_px,
            total_pinned_width_px,
            header_height_px,
        }
    }

    /// Compute scroll clamping limits and sub-cell pixel offsets in O(1).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn scroll_metrics(&self, total_rows: usize, total_flat_cols: usize, pinned_left_width_px: f64, header_height_px: f64) -> GridScrollMetrics {
        let row_h = self.viewport.sanitized_row_height_px();
        let col_w = self.viewport.sanitized_col_width_px();
        let view_w = self.viewport.sanitized_viewport_width_px();
        let view_h = self.viewport.sanitized_viewport_height_px();
        let pinned_w = finite_non_neg(pinned_left_width_px);
        let hdr_h = finite_non_neg(header_height_px);
        let body_width_px = (view_w - pinned_w).max(0.0);
        let body_height_px = (view_h - hdr_h).max(0.0);
        let total_content_width_px = (total_flat_cols as f64) * col_w;
        let total_content_height_px = (total_rows as f64) * row_h;
        let max_scroll_x_px = (total_content_width_px - body_width_px).max(0.0);
        let max_scroll_y_px = (total_content_height_px - body_height_px).max(0.0);

        let scroll_x = finite_non_neg(self.viewport.scroll_x_px).min(max_scroll_x_px);
        let scroll_y = finite_non_neg(self.viewport.scroll_y_px).min(max_scroll_y_px);
        let subcol_offset_px = -(scroll_x % col_w);
        let subrow_offset_px = -(scroll_y % row_h);

        GridScrollMetrics {
            body_origin_x_px: pinned_w,
            body_origin_y_px: hdr_h,
            body_width_px,
            body_height_px,
            total_content_width_px,
            total_content_height_px,
            max_scroll_x_px,
            max_scroll_y_px,
            subrow_offset_px,
            subcol_offset_px,
        }
    }

    /// Set scroll offsets clamped cleanly to `[0.0, max_scroll]`.
    pub fn set_scroll_clamped(&mut self, scroll_x_px: f64, scroll_y_px: f64, total_rows: usize, total_flat_cols: usize) {
        let metrics =
            self.scroll_metrics(total_rows, total_flat_cols, self.viewport.default_pinned_left_width_px(), self.viewport.header_height_px());
        self.viewport.scroll_x_px = finite_non_neg(scroll_x_px).min(metrics.max_scroll_x_px);
        self.viewport.scroll_y_px = finite_non_neg(scroll_y_px).min(metrics.max_scroll_y_px);
    }

    /// Incrementally scroll by `(dx_px, dy_px)` clamped to table bounds.
    pub fn scroll_by(&mut self, dx_px: f64, dy_px: f64, total_rows: usize, total_flat_cols: usize) {
        let next_x = finite_non_neg(self.viewport.scroll_x_px) + finite_or_zero(dx_px);
        let next_y = finite_non_neg(self.viewport.scroll_y_px) + finite_or_zero(dy_px);
        self.set_scroll_clamped(next_x, next_y, total_rows, total_flat_cols);
    }

    /// Incrementally scroll `sheet` using its exact pinned row-title and header geometry.
    pub fn scroll_for_sheet(&mut self, sheet: &DataSheet, dx_px: f64, dy_px: f64) {
        let pinned = self.pinned_row_title_geometry(sheet);
        let total_rows = sheet.archetype.row_count();
        let total_cols = Self::sheet_total_flat_cols(sheet);
        let metrics = self.scroll_metrics(total_rows, total_cols, pinned.total_pinned_width_px, pinned.header_height_px);
        let next_x = finite_non_neg(self.viewport.scroll_x_px) + finite_or_zero(dx_px);
        let next_y = finite_non_neg(self.viewport.scroll_y_px) + finite_or_zero(dy_px);
        self.viewport.scroll_x_px = next_x.max(0.0).min(metrics.max_scroll_x_px);
        self.viewport.scroll_y_px = next_y.max(0.0).min(metrics.max_scroll_y_px);
    }

    /// Compute visible two-tier `(Group x Subcolumn)` header slices in O(visible_cols) without
    /// allocating full subcolumn label arrays across `1_024` subcolumns.
    #[must_use]
    pub fn visible_two_tier_headers(&self, sheet: &DataSheet) -> GridTwoTierHeaderWindow {
        let pinned = self.pinned_row_title_geometry(sheet);
        let total_flat_cols = Self::sheet_total_flat_cols(sheet);
        let win =
            self.visible_window_with_chrome(sheet.archetype.row_count(), total_flat_cols, pinned.total_pinned_width_px, pinned.header_height_px);
        self.build_two_tier_headers(sheet, win.start_col, win.end_col, total_flat_cols, pinned.total_pinned_width_px)
    }

    #[allow(clippy::cast_precision_loss)]
    fn build_two_tier_headers(
        &self,
        sheet: &DataSheet,
        start_flat: usize,
        end_flat: usize,
        total_flat_cols: usize,
        pinned_left_width_px: f64,
    ) -> GridTwoTierHeaderWindow {
        if start_flat >= end_flat || total_flat_cols == 0 {
            return GridTwoTierHeaderWindow { start_flat_col: start_flat, end_flat_col: end_flat, total_flat_cols, groups: Vec::new() };
        }

        let col_w = self.viewport.sanitized_col_width_px();
        let scroll_x = finite_non_neg(self.viewport.scroll_x_px);
        let mut groups = Vec::new();

        match &sheet.archetype {
            TableArchetype::Nested(nested) => {
                let mut offset = 0usize;
                for (g_idx, group) in nested.groups.iter().enumerate() {
                    let count = group.biological_unit_count();
                    let g_end = offset.saturating_add(count);
                    if g_end <= start_flat {
                        offset = g_end;
                        continue;
                    }
                    if offset >= end_flat {
                        break;
                    }
                    let vis_start = start_flat.max(offset);
                    let vis_end = end_flat.min(g_end);
                    let sub_start = vis_start - offset;
                    let sub_end = vis_end - offset;
                    let slice = build_group_slice(
                        sheet,
                        &GroupSliceSpec {
                            group_index: g_idx,
                            vis_start,
                            vis_end,
                            subcol_start: sub_start,
                            subcol_end: sub_end,
                            pinned_left_width_px,
                            scroll_x_px: scroll_x,
                            col_w,
                        },
                    );
                    groups.push(slice);
                    offset = g_end;
                }
            }
            _ => {
                let subs_per_group = Self::sheet_subcolumn_count_for_group(sheet, 0).max(1);
                let start_group = start_flat / subs_per_group;
                let end_group = (end_flat - 1) / subs_per_group + 1;
                for g_idx in start_group..end_group {
                    let g_flat_start = g_idx.saturating_mul(subs_per_group);
                    let g_flat_end = g_flat_start.saturating_add(subs_per_group);
                    let vis_start = start_flat.max(g_flat_start);
                    let vis_end = end_flat.min(g_flat_end);
                    if vis_start >= vis_end {
                        continue;
                    }
                    let slice = build_group_slice(
                        sheet,
                        &GroupSliceSpec {
                            group_index: g_idx,
                            vis_start,
                            vis_end,
                            subcol_start: vis_start - g_flat_start,
                            subcol_end: vis_end - g_flat_start,
                            pinned_left_width_px,
                            scroll_x_px: scroll_x,
                            col_w,
                        },
                    );
                    groups.push(slice);
                }
            }
        }

        GridTwoTierHeaderWindow { start_flat_col: start_flat, end_flat_col: end_flat, total_flat_cols, groups }
    }

    /// Format visible window cells, headers, pinned geometry, and scroll metrics in one O(visible_cells) pass.
    #[must_use]
    pub fn format_visible_window(&self, sheet: &DataSheet) -> GridVisibleWindowFrame {
        let pinned = self.pinned_row_title_geometry(sheet);
        let total_rows = sheet.archetype.row_count();
        let total_flat_cols = Self::sheet_total_flat_cols(sheet);
        let window = self.visible_window_with_chrome(total_rows, total_flat_cols, pinned.total_pinned_width_px, pinned.header_height_px);
        let scroll = self.scroll_metrics(total_rows, total_flat_cols, pinned.total_pinned_width_px, pinned.header_height_px);
        let headers = self.build_two_tier_headers(sheet, window.start_col, window.end_col, total_flat_cols, pinned.total_pinned_width_px);

        let vis_cols = window.end_col.saturating_sub(window.start_col);
        let vis_rows = window.end_row.saturating_sub(window.start_row);
        let mut cells = Vec::with_capacity(vis_rows.saturating_mul(vis_cols));

        let sel_flat_bounds = self.selection_bounds().and_then(|(s, e)| {
            let sf = Self::coord_to_flat_col(sheet, s)?;
            let ef = Self::coord_to_flat_col(sheet, e)?;
            Some((s.row, e.row, sf.min(ef), sf.max(ef)))
        });

        let col_coords: Vec<(usize, usize, usize)> =
            (window.start_col..window.end_col).filter_map(|fc| Self::flat_col_to_coord(sheet, fc).map(|(c, s)| (fc, c, s))).collect();

        for row in window.start_row..window.end_row {
            for &(flat_col, col, subcol) in &col_coords {
                let coord = GridCellCoord::new(row, col, subcol);
                let cell = sheet.cell(CellCoord::from(coord));
                let display_text = cell.map_or_else(String::new, |c| format_cell_display(c, self.decimal_separator));
                let excluded = cell.is_some_and(|c| c.excluded);
                let link_border = find_linked_border(&sheet.linked_regions, coord);
                let is_selected = sel_flat_bounds.is_some_and(|(r0, r1, f0, f1)| row >= r0 && row <= r1 && flat_col >= f0 && flat_col <= f1);
                let is_active = self.active == Some(coord);

                cells.push(VisibleCellView { coord, flat_col, display_text, excluded, link_border, is_selected, is_active });
            }
        }

        GridVisibleWindowFrame { window, pinned, scroll, headers, cells }
    }

    /// Adjust scroll offsets minimally so `coord` lies inside the visible body viewport.
    #[allow(clippy::cast_precision_loss)]
    pub fn ensure_coord_visible(&mut self, sheet: &DataSheet, coord: GridCellCoord) {
        let Some(flat_col) = Self::coord_to_flat_col(sheet, coord) else {
            return;
        };
        let pinned = self.pinned_row_title_geometry(sheet);
        let total_rows = sheet.archetype.row_count();
        let total_cols = Self::sheet_total_flat_cols(sheet);
        let metrics = self.scroll_metrics(total_rows, total_cols, pinned.total_pinned_width_px, pinned.header_height_px);
        let row_h = self.viewport.sanitized_row_height_px();
        let col_w = self.viewport.sanitized_col_width_px();

        let cell_top = (coord.row as f64) * row_h;
        let cell_bottom = cell_top + row_h;
        let mut next_y = finite_non_neg(self.viewport.scroll_y_px);
        if cell_top < next_y {
            next_y = cell_top;
        } else if metrics.body_height_px > 0.0 && cell_bottom > next_y + metrics.body_height_px {
            next_y = cell_bottom - metrics.body_height_px;
        }

        let cell_left = (flat_col as f64) * col_w;
        let cell_right = cell_left + col_w;
        let mut next_x = finite_non_neg(self.viewport.scroll_x_px);
        if cell_left < next_x {
            next_x = cell_left;
        } else if metrics.body_width_px > 0.0 && cell_right > next_x + metrics.body_width_px {
            next_x = cell_right - metrics.body_width_px;
        }

        self.viewport.scroll_x_px = next_x.max(0.0).min(metrics.max_scroll_x_px);
        self.viewport.scroll_y_px = next_y.max(0.0).min(metrics.max_scroll_y_px);
    }

    /// Apply a keyboard navigation action on `sheet`, returning the new active coordinate.
    pub fn navigate(&mut self, sheet: &DataSheet, action: GridNavAction) -> Option<GridCellCoord> {
        let total_rows = sheet.archetype.row_count();
        let total_flat_cols = Self::sheet_total_flat_cols(sheet);
        if total_rows == 0 || total_flat_cols == 0 {
            return None;
        }

        let current = self.active.unwrap_or_else(|| GridCellCoord::new(0, 0, 0));
        let cur_row = current.row.min(total_rows - 1);
        let cur_flat = Self::coord_to_flat_col(sheet, current).unwrap_or(0).min(total_flat_cols - 1);

        let ((next_row, next_flat), extend) = match action {
            GridNavAction::Arrow { dir, extend, jump_block } => {
                let target = if jump_block {
                    jump_block_edge(sheet, cur_row, cur_flat, total_rows, total_flat_cols, dir)
                } else {
                    step_one(cur_row, cur_flat, total_rows, total_flat_cols, dir)
                };
                (target, extend)
            }
            GridNavAction::Tab { backward } => (step_tab(cur_row, cur_flat, total_rows, total_flat_cols, backward), false),
            GridNavAction::Enter { backward } => {
                let dir = if backward { GridDirection::Up } else { GridDirection::Down };
                (step_one(cur_row, cur_flat, total_rows, total_flat_cols, dir), false)
            }
            GridNavAction::Home { to_table_start, extend } => {
                let target = if to_table_start { (0, 0) } else { (cur_row, 0) };
                (target, extend)
            }
            GridNavAction::End { to_table_end, extend } => {
                let target = if to_table_end { (total_rows - 1, total_flat_cols - 1) } else { (cur_row, total_flat_cols - 1) };
                (target, extend)
            }
            GridNavAction::PageUp { extend } => (step_page(self, sheet, cur_row, cur_flat, total_rows, true), extend),
            GridNavAction::PageDown { extend } => (step_page(self, sheet, cur_row, cur_flat, total_rows, false), extend),
        };

        let (next_col, next_subcol) = Self::flat_col_to_coord(sheet, next_flat)?;
        let dest = GridCellCoord::new(next_row, next_col, next_subcol);
        if extend {
            if self.anchor.is_none() {
                let (c, s) = Self::flat_col_to_coord(sheet, cur_flat).unwrap_or((0, 0));
                self.anchor = Some(GridCellCoord::new(cur_row, c, s));
            }
            self.active = Some(dest);
            self.editing_buffer = None;
        } else {
            self.select_single(dest);
        }
        self.ensure_coord_visible(sheet, dest);
        Some(dest)
    }

    /// Emit a `table.setCells` command intent for batch cell edits.
    pub fn emit_set_cells_intent(&mut self, sheet_id: &SheetId, edits: &[(GridCellCoord, Cell)]) -> ToolCommandIntent {
        let edit_values: Vec<serde_json::Value> = edits
            .iter()
            .map(|(coord, cell)| {
                let val_json = match &cell.value {
                    CellValue::Empty => serde_json::Value::Null,
                    CellValue::Number(v) if v.is_finite() => json!(*v),
                    CellValue::Number(_) => serde_json::Value::Null,
                    CellValue::Text(s) => json!(s),
                };
                json!({
                    "row": coord.row,
                    "col": coord.col,
                    "subcol": coord.subcol,
                    "value": val_json,
                    "excluded": cell.excluded,
                })
            })
            .collect();
        let intent = ToolCommandIntent::new(
            "table.setCells",
            json!({
                "sheet_id": sheet_id.as_str(),
                "edits": edit_values,
            }),
        );
        self.pending_intents.push(intent.clone());
        intent
    }

    /// Emit a `table.excludeCells` command intent over the active 3-D box selection, if one exists.
    pub fn emit_exclude_selection_intent(&mut self, sheet_id: &SheetId, excluded: bool) -> Option<ToolCommandIntent> {
        let (start, end) = self.selection_box_bounds()?;
        let intent = build_exclude_box_intent(sheet_id, start, end, excluded);
        self.pending_intents.push(intent.clone());
        Some(intent)
    }

    /// Emit exact per-column-group `table.excludeCells` 3-D box intents matching the visual flat selection on `sheet`.
    pub fn emit_exclude_selection_intents_for_sheet(&mut self, sheet: &DataSheet, excluded: bool) -> Vec<ToolCommandIntent> {
        let Some(a) = self.active else {
            return Vec::new();
        };
        let b = self.anchor.unwrap_or(a);
        let Some((start_row, end_row, start_flat, end_flat)) = normalized_flat_rect(sheet, a, b) else {
            return Vec::new();
        };
        let col_count = sheet.archetype.column_count();
        let mut intents = Vec::new();
        let mut offset = 0usize;
        for col_idx in 0..col_count {
            let subcols = Self::sheet_subcolumn_count_for_group(sheet, col_idx);
            if subcols == 0 {
                continue;
            }
            let lo = start_flat.max(offset);
            let hi = end_flat.min(offset + subcols - 1);
            if lo <= hi {
                let start = GridCellCoord::new(start_row, col_idx, lo - offset);
                let end = GridCellCoord::new(end_row, col_idx, hi - offset);
                let intent = build_exclude_box_intent(&sheet.id, start, end, excluded);
                self.pending_intents.push(intent.clone());
                intents.push(intent);
            }
            offset += subcols;
        }
        intents
    }

    /// Parse TSV clipboard text and emit a `table.paste` command intent anchored at `active`/`anchor`.
    pub fn emit_paste_intent(
        &mut self,
        sheet_id: &SheetId,
        raw_tsv: &str,
        transpose: bool,
        parse_asterisk_exclusions: bool,
    ) -> Result<ToolCommandIntent, GridToolError> {
        let cells = parse_tsv_clipboard(raw_tsv, self.decimal_separator, parse_asterisk_exclusions)?;
        if cells.is_empty() {
            return Err(GridToolError::EmptyClipboard);
        }
        let anchor = self.active.or(self.anchor).unwrap_or_default();
        let serialized_rows = serde_json::to_value(&cells).unwrap_or(serde_json::Value::Null);
        let intent = ToolCommandIntent::new(
            "table.paste",
            json!({
                "sheet_id": sheet_id.as_str(),
                "anchor": { "row": anchor.row, "col": anchor.col, "subcol": anchor.subcol },
                "tsv": raw_tsv,
                "cells": serialized_rows,
                "decimal_separator": self.decimal_separator.to_string(),
                "parse_asterisk_exclusions": parse_asterisk_exclusions,
                "transpose": transpose,
            }),
        );
        self.pending_intents.push(intent.clone());
        Ok(intent)
    }

    /// Emit a `table.pasteLink` command intent for attaching a live [`ImportLinkSpec`].
    pub fn emit_paste_link_intent(&mut self, target_sheet_id: &SheetId, link_spec: &ImportLinkSpec) -> ToolCommandIntent {
        let anchor = self.active.or(self.anchor).unwrap_or_else(|| GridCellCoord::from(link_spec.placement.anchor));
        let spec_json = serde_json::to_value(link_spec).unwrap_or(serde_json::Value::Null);
        let intent = ToolCommandIntent::new(
            "table.pasteLink",
            json!({
                "sheet_id": target_sheet_id.as_str(),
                "anchor": { "row": anchor.row, "col": anchor.col, "subcol": anchor.subcol },
                "link_spec": spec_json,
            }),
        );
        self.pending_intents.push(intent.clone());
        intent
    }

    /// Drain all queued tool command intents in FIFO order.
    pub fn drain_intents(&mut self) -> Vec<ToolCommandIntent> {
        std::mem::take(&mut self.pending_intents)
    }
}

fn build_exclude_box_intent(sheet_id: &SheetId, start: GridCellCoord, end: GridCellCoord, excluded: bool) -> ToolCommandIntent {
    ToolCommandIntent::new(
        "table.excludeCells",
        json!({
            "sheet_id": sheet_id.as_str(),
            "row": start.row,
            "col": start.col,
            "subcol": start.subcol,
            "range": {
                "start": { "row": start.row, "col": start.col, "subcol": start.subcol },
                "end": { "row": end.row, "col": end.col, "subcol": end.subcol },
            },
            "excluded": excluded,
        }),
    )
}

/// Format a single [`Cell`] into its TSV token string, normalizing `-0.0` and marking exclusions with `*`.
#[must_use]
pub fn format_cell_tsv_token(cell: &Cell, decimal_separator: char) -> String {
    let base = format_cell_display(cell, decimal_separator);
    if cell.excluded && !base.is_empty() {
        format!("{base}*")
    } else if base.contains(['\t', '\n', '\r', '"']) {
        let escaped = base.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        base
    }
}

/// Format an arbitrary 2D cell grid into deterministic TSV clipboard text.
#[must_use]
pub fn format_grid_tsv(grid: &[Vec<Cell>], decimal_separator: char) -> String {
    let mut out = String::new();
    for (r_idx, row) in grid.iter().enumerate() {
        if r_idx > 0 {
            out.push('\n');
        }
        for (c_idx, cell) in row.iter().enumerate() {
            if c_idx > 0 {
                out.push('\t');
            }
            let tok = format_cell_tsv_token(cell, decimal_separator);
            if tok.contains(['\t', '\n', '\r', '"']) && !(tok.starts_with('"') && tok.ends_with('"')) {
                let escaped = tok.replace('"', "\"\"");
                out.push('"');
                out.push_str(&escaped);
                out.push('"');
            } else {
                out.push_str(&tok);
            }
        }
    }
    out
}

/// Format rectangular selection `[a, b]` on `sheet` into TSV clipboard text.
#[must_use]
pub fn format_selection_tsv(sheet: &DataSheet, a: GridCellCoord, b: GridCellCoord, decimal_separator: char) -> String {
    let Some((start_row, end_row, start_flat, end_flat)) = normalized_flat_rect(sheet, a, b) else {
        return String::new();
    };
    let coords: Vec<(usize, usize)> = (start_flat..=end_flat).filter_map(|fc| GridToolState::flat_col_to_coord(sheet, fc)).collect();
    let mut out = String::new();
    for (r_idx, row) in (start_row..=end_row).enumerate() {
        if r_idx > 0 {
            out.push('\n');
        }
        for (c_idx, &(col, subcol)) in coords.iter().enumerate() {
            if c_idx > 0 {
                out.push('\t');
            }
            if let Some(cell) = sheet.cell(CellCoord::new(row, col, subcol)) {
                out.push_str(&format_cell_tsv_token(cell, decimal_separator));
            }
        }
    }
    out
}

/// Parse TSV clipboard text (supporting RFC-4180 quotes, CRLF/LF, regional decimals, and `*` exclusions).
pub fn parse_tsv_clipboard(raw: &str, decimal_separator: char, parse_asterisk_exclusions: bool) -> Result<Vec<Vec<Cell>>, GridToolError> {
    let cleaned = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let trimmed_end = cleaned.strip_suffix("\r\n").or_else(|| cleaned.strip_suffix('\n')).unwrap_or(cleaned);
    if trimmed_end.is_empty() {
        return Ok(Vec::new());
    }

    let raw_rows = split_tsv_records(trimmed_end);
    let mut total_cells = 0usize;
    let mut rows = Vec::with_capacity(raw_rows.len());
    for raw_row in raw_rows {
        total_cells = total_cells.saturating_add(raw_row.len());
        if total_cells > MAX_CLIPBOARD_CELLS {
            return Err(GridToolError::ClipboardTooLarge { cell_count: total_cells, max: MAX_CLIPBOARD_CELLS });
        }
        let parsed_row = raw_row.into_iter().map(|tok| Cell::parse_import_token(&tok, decimal_separator, parse_asterisk_exclusions)).collect();
        rows.push(parsed_row);
    }
    Ok(rows)
}

fn split_tsv_records(input: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    let _ = chars.next();
                    field.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }

        match ch {
            '"' if field.is_empty() => in_quotes = true,
            '\t' => {
                current_row.push(std::mem::take(&mut field));
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    let _ = chars.next();
                }
                current_row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut current_row));
            }
            '\n' => {
                current_row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut current_row));
            }
            other => field.push(other),
        }
    }

    current_row.push(field);
    rows.push(current_row);
    rows
}

fn format_cell_display(cell: &Cell, decimal_separator: char) -> String {
    match &cell.value {
        CellValue::Empty => String::new(),
        CellValue::Number(v) if v.is_finite() => {
            let clean = if *v == 0.0 { 0.0 } else { *v };
            let s = format!("{clean}");
            if decimal_separator == ',' { s.replace('.', ",") } else { s }
        }
        CellValue::Number(_) => String::new(),
        CellValue::Text(s) => s.clone(),
    }
}

fn normalized_flat_rect(sheet: &DataSheet, a: GridCellCoord, b: GridCellCoord) -> Option<(usize, usize, usize, usize)> {
    let row_count = sheet.archetype.row_count();
    let flat_count = GridToolState::sheet_total_flat_cols(sheet);
    if row_count == 0 || flat_count == 0 {
        return None;
    }

    let min_r = a.row.min(b.row);
    let max_r = a.row.max(b.row);
    if min_r >= row_count {
        return None;
    }
    let end_row = max_r.min(row_count - 1);

    let (c0, s0, c1, s1) =
        if (a.col, a.subcol) <= (b.col, b.subcol) { (a.col, a.subcol, b.col, b.subcol) } else { (b.col, b.subcol, a.col, a.subcol) };
    let start_flat = clamp_coord_to_flat_start(sheet, c0, s0)?;
    let end_flat = clamp_coord_to_flat_end(sheet, c1, s1, flat_count)?;
    if start_flat > end_flat {
        return None;
    }
    Some((min_r, end_row, start_flat, end_flat))
}

fn clamp_coord_to_flat_start(sheet: &DataSheet, col: usize, subcol: usize) -> Option<usize> {
    let col_count = sheet.archetype.column_count();
    if col >= col_count {
        return None;
    }
    if let Some(fc) = GridToolState::coord_to_flat_col(sheet, GridCellCoord::new(0, col, subcol)) {
        return Some(fc);
    }
    let max_sub = GridToolState::sheet_subcolumn_count_for_group(sheet, col);
    if subcol >= max_sub { None } else { GridToolState::coord_to_flat_col(sheet, GridCellCoord::new(0, col, 0)) }
}

fn clamp_coord_to_flat_end(sheet: &DataSheet, col: usize, subcol: usize, total_flat: usize) -> Option<usize> {
    let col_count = sheet.archetype.column_count();
    if col_count == 0 || total_flat == 0 {
        return None;
    }
    if col >= col_count {
        return Some(total_flat - 1);
    }
    let max_sub = GridToolState::sheet_subcolumn_count_for_group(sheet, col).min(MAX_SUBCOLUMNS);
    if max_sub == 0 {
        return Some(total_flat - 1);
    }
    let clamped_sub = subcol.min(max_sub - 1);
    GridToolState::coord_to_flat_col(sheet, GridCellCoord::new(0, col, clamped_sub)).or(Some(total_flat - 1))
}

struct GroupSliceSpec {
    group_index: usize,
    vis_start: usize,
    vis_end: usize,
    subcol_start: usize,
    subcol_end: usize,
    pinned_left_width_px: f64,
    scroll_x_px: f64,
    col_w: f64,
}

#[allow(clippy::cast_precision_loss)]
fn build_group_slice(sheet: &DataSheet, spec: &GroupSliceSpec) -> VisibleGroupHeaderSlice {
    let title = group_title(&sheet.archetype, spec.group_index).to_owned();
    let x_px = spec.pinned_left_width_px + (spec.vis_start as f64) * spec.col_w - spec.scroll_x_px;
    let width_px = ((spec.vis_end - spec.vis_start) as f64) * spec.col_w;
    let mut subcolumns = Vec::with_capacity(spec.vis_end - spec.vis_start);
    for (offset, flat_col_index) in (spec.vis_start..spec.vis_end).enumerate() {
        let subcol_index = spec.subcol_start + offset;
        let sx = spec.pinned_left_width_px + (flat_col_index as f64) * spec.col_w - spec.scroll_x_px;
        let label = subcolumn_label_at(&sheet.archetype, spec.group_index, subcol_index);
        subcolumns.push(VisibleSubcolumnHeader {
            group_index: spec.group_index,
            subcol_index,
            flat_col_index,
            label,
            x_px: sx,
            width_px: spec.col_w,
        });
    }
    VisibleGroupHeaderSlice {
        group_index: spec.group_index,
        title,
        flat_col_start: spec.vis_start,
        flat_col_end: spec.vis_end,
        subcol_start: spec.subcol_start,
        subcol_end: spec.subcol_end,
        x_px,
        width_px,
        subcolumns,
    }
}

fn group_title(arch: &TableArchetype, col: usize) -> &str {
    match arch {
        TableArchetype::Xy(t) => t.data_sets.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::Column(t) => t.data_sets.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::Grouped(t) => t.data_sets.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::Contingency(t) => t.data_sets.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::Survival(t) => t.cohorts.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or("", |d| d.title.as_str()),
        TableArchetype::MultipleVariables(t) => t.variables.get(col).map_or("", |v| v.title.as_str()),
        TableArchetype::Nested(t) => t.groups.get(col).map_or("", |g| g.title.as_str()),
    }
}

/// O(1) subcolumn label lookup avoiding allocating `Vec<String>` across `RawReplicates(1000)`.
fn subcolumn_label_at(arch: &TableArchetype, col: usize, subcol: usize) -> String {
    match arch {
        TableArchetype::Xy(t) => label_for_subcolumn_mode(t.y_subcolumn_mode, subcol),
        TableArchetype::Column(t) => label_for_subcolumn_mode(t.subcolumn_mode, subcol),
        TableArchetype::Grouped(t) => label_for_subcolumn_mode(t.subcolumn_mode, subcol),
        TableArchetype::Nested(t) => {
            t.groups.get(col).and_then(|g| g.subcolumn_titles.get(subcol)).cloned().unwrap_or_else(|| format!("{col}:{sub}", sub = subcol + 1))
        }
        TableArchetype::Contingency(_) | TableArchetype::Survival(_) | TableArchetype::PartsOfWhole(_) | TableArchetype::MultipleVariables(_) => {
            String::from("Y")
        }
    }
}

fn label_for_subcolumn_mode(mode: SubcolumnMode, subcol: usize) -> String {
    match mode {
        SubcolumnMode::Single => String::from("Y"),
        SubcolumnMode::RawReplicates(_) => format!("Y{}", subcol + 1),
        SubcolumnMode::MeanSdN => match subcol {
            0 => String::from("Mean"),
            1 => String::from("SD"),
            _ => String::from("N"),
        },
        SubcolumnMode::MeanSemN => match subcol {
            0 => String::from("Mean"),
            1 => String::from("SEM"),
            _ => String::from("N"),
        },
        SubcolumnMode::MeanCvN => match subcol {
            0 => String::from("Mean"),
            1 => String::from("%CV"),
            _ => String::from("N"),
        },
        SubcolumnMode::MeanLowHigh => match subcol {
            0 => String::from("Mean"),
            1 => String::from("-Error"),
            _ => String::from("+Error"),
        },
        SubcolumnMode::UpperLowerLimits => match subcol {
            0 => String::from("Mean"),
            1 => String::from("Lower"),
            _ => String::from("Upper"),
        },
        SubcolumnMode::MedianIqr => match subcol {
            0 => String::from("Median"),
            1 => String::from("Q1"),
            _ => String::from("Q3"),
        },
    }
}

fn sheet_row_titles_expanded(sheet: &DataSheet) -> bool {
    match &sheet.archetype {
        TableArchetype::Xy(t) => t.row_titles_expanded,
        TableArchetype::Column(t) => t.row_titles_expanded,
        TableArchetype::Grouped(_) | TableArchetype::Contingency(_) | TableArchetype::PartsOfWhole(_) => true,
        TableArchetype::Survival(t) => t.row_titles_expanded,
        TableArchetype::MultipleVariables(t) => t.row_titles_expanded,
        TableArchetype::Nested(t) => t.row_titles_expanded,
    }
}

fn sheet_x_or_time_col_count(sheet: &DataSheet) -> usize {
    match &sheet.archetype {
        TableArchetype::Xy(t) => match t.x_mode {
            XColumnMode::GeneratedSequence { .. } => 1,
            XColumnMode::DateOrElapsedTime { .. } | XColumnMode::Numbers { .. } => t.x_mode.subcolumn_count(),
        },
        TableArchetype::Survival(_) => 1,
        _ => 0,
    }
}

fn sheet_uses_two_tier_headers(sheet: &DataSheet) -> bool {
    match &sheet.archetype {
        TableArchetype::Xy(t) => !matches!(t.y_subcolumn_mode, SubcolumnMode::Single),
        TableArchetype::Column(t) => !matches!(t.subcolumn_mode, SubcolumnMode::Single),
        TableArchetype::Grouped(_) | TableArchetype::Nested(_) => true,
        _ => false,
    }
}

fn find_linked_border(regions: &[LinkedDataRegion], coord: GridCellCoord) -> Option<LinkBorderColor> {
    for region in regions {
        if range_contains(&region.target_range, coord)
            && let Some(color) = region.border_color()
        {
            return Some(color);
        }
    }
    None
}

fn range_contains(range: &SheetCellRange, coord: GridCellCoord) -> bool {
    let min_row = range.start.row.min(range.end.row);
    let max_row = range.start.row.max(range.end.row);
    if coord.row < min_row || coord.row > max_row {
        return false;
    }
    let a = (range.start.col, range.start.subcol);
    let b = (range.end.col, range.end.subcol);
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let target = (coord.col, coord.subcol);
    target >= lo && target <= hi
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn compute_visible_axis_range(scroll_px: f64, body_extent_px: f64, cell_extent_px: f64, total_items: usize) -> (usize, usize) {
    if total_items == 0 || body_extent_px <= 0.0 {
        return (0, 0);
    }
    let start_raw = (scroll_px / cell_extent_px).floor();
    let start = if !start_raw.is_finite() || start_raw <= 0.0 {
        0
    } else if start_raw >= (total_items as f64) {
        total_items
    } else {
        start_raw as usize
    };
    if start >= total_items {
        return (total_items, total_items);
    }

    let remaining = total_items.saturating_sub(start);
    let vis_raw = (body_extent_px / cell_extent_px).ceil();
    let vis_count = if !vis_raw.is_finite() || vis_raw <= 0.0 {
        1
    } else if vis_raw >= (remaining as f64) {
        remaining
    } else {
        (vis_raw as usize).saturating_add(1).min(remaining)
    };
    (start, start.saturating_add(vis_count).min(total_items))
}

fn step_one(row: usize, flat_col: usize, total_rows: usize, total_flat_cols: usize, dir: GridDirection) -> (usize, usize) {
    match dir {
        GridDirection::Up => (row.saturating_sub(1), flat_col),
        GridDirection::Down => ((row + 1).min(total_rows - 1), flat_col),
        GridDirection::Left => (row, flat_col.saturating_sub(1)),
        GridDirection::Right => (row, (flat_col + 1).min(total_flat_cols - 1)),
    }
}

fn step_tab(row: usize, flat_col: usize, total_rows: usize, total_flat_cols: usize, backward: bool) -> (usize, usize) {
    if backward {
        if flat_col > 0 {
            (row, flat_col - 1)
        } else if row > 0 {
            (row - 1, total_flat_cols - 1)
        } else {
            (0, 0)
        }
    } else if flat_col + 1 < total_flat_cols {
        (row, flat_col + 1)
    } else if row + 1 < total_rows {
        (row + 1, 0)
    } else {
        (total_rows - 1, total_flat_cols - 1)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn step_page(state: &GridToolState, sheet: &DataSheet, row: usize, flat_col: usize, total_rows: usize, up: bool) -> (usize, usize) {
    let pinned = state.pinned_row_title_geometry(sheet);
    let body_h = (state.viewport.sanitized_viewport_height_px() - pinned.header_height_px).max(0.0);
    let row_h = state.viewport.sanitized_row_height_px();
    let page_rows = ((body_h / row_h).floor() as usize).clamp(1, total_rows.max(1));
    let next_row = if up { row.saturating_sub(page_rows) } else { row.saturating_add(page_rows).min(total_rows - 1) };
    (next_row, flat_col)
}

fn jump_block_edge(sheet: &DataSheet, row: usize, flat_col: usize, total_rows: usize, total_flat_cols: usize, dir: GridDirection) -> (usize, usize) {
    let Some((next_r, next_f)) = advance_in_dir(row, flat_col, total_rows, total_flat_cols, dir) else {
        return (row, flat_col);
    };

    let cur_pop = is_flat_cell_populated(sheet, row, flat_col);
    let next_pop = is_flat_cell_populated(sheet, next_r, next_f);

    let (mut r, mut f) = (next_r, next_f);
    if cur_pop && next_pop {
        while let Some((nr, nf)) = advance_in_dir(r, f, total_rows, total_flat_cols, dir) {
            if is_flat_cell_populated(sheet, nr, nf) {
                r = nr;
                f = nf;
            } else {
                break;
            }
        }
        return (r, f);
    }

    while !is_flat_cell_populated(sheet, r, f) {
        let Some((nr, nf)) = advance_in_dir(r, f, total_rows, total_flat_cols, dir) else {
            return (r, f);
        };
        r = nr;
        f = nf;
    }
    (r, f)
}

fn advance_in_dir(row: usize, flat_col: usize, total_rows: usize, total_flat_cols: usize, dir: GridDirection) -> Option<(usize, usize)> {
    match dir {
        GridDirection::Up => (row > 0).then(|| (row - 1, flat_col)),
        GridDirection::Down => (row + 1 < total_rows).then(|| (row + 1, flat_col)),
        GridDirection::Left => (flat_col > 0).then(|| (row, flat_col - 1)),
        GridDirection::Right => (flat_col + 1 < total_flat_cols).then(|| (row, flat_col + 1)),
    }
}

fn is_flat_cell_populated(sheet: &DataSheet, row: usize, flat_col: usize) -> bool {
    let Some((col, subcol)) = GridToolState::flat_col_to_coord(sheet, flat_col) else {
        return false;
    };
    sheet.cell(CellCoord::new(row, col, subcol)).is_some_and(|c| !c.is_empty())
}

fn finite_non_neg(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

#[allow(dead_code)]
const fn ensure_bounds_constants_used() -> usize {
    MAX_DATA_SETS + MAX_ROWS + Ordering::Equal as usize
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec,
        InternalLinkKind, LinkBorderColor, LinkSource, LinkedDataRegion, MAX_ROWS, NestedTable, SheetCellRange, SheetId, SubcolumnMode,
        TableArchetype, XyTable,
    };

    use super::{
        GridCellCoord, GridDirection, GridNavAction, GridToolError, GridToolState, SelectionQuickStats, format_selection_tsv, parse_tsv_clipboard,
    };
    use crate::Cursor;

    fn sample_grouped_sheet() -> DataSheet {
        let grouped = GroupedTable::new(SubcolumnMode::RawReplicates(3), 4, 8).expect("grouped table");
        DataSheet::new(SheetId::new("grp-1"), "Grouped", TableArchetype::Grouped(grouped)).expect("sheet")
    }

    #[test]
    fn quick_stats_computes_sum_mean_sd_sem_median_without_nans() {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 6).expect("col");
        let mut sheet = DataSheet::new("d1", "Stats", TableArchetype::Column(col)).expect("sheet");
        // Col 0: [10, 20, 30, 40], excluded 99, text "ctrl"
        sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(10.0)).expect("c0");
        sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(20.0)).expect("c1");
        sheet.set_cell(CellCoord::new(2, 0, 0), Cell::number(30.0)).expect("c2");
        sheet.set_cell(CellCoord::new(3, 0, 0), Cell::number(40.0)).expect("c3");
        sheet.set_cell(CellCoord::new(4, 0, 0), Cell::excluded_number(99.0)).expect("c4");
        sheet.set_cell(CellCoord::new(5, 0, 0), Cell::text("ctrl")).expect("c5");
        // Col 1: [50]
        sheet.set_cell(CellCoord::new(0, 1, 0), Cell::number(50.0)).expect("c6");

        let stats = SelectionQuickStats::from_data_sheet(&sheet, GridCellCoord::new(0, 0, 0), GridCellCoord::new(5, 1, 0));
        assert_eq!(stats.selected_cells, 12);
        assert_eq!(stats.numeric_count, 5);
        assert_eq!(stats.excluded_count, 1);
        assert_eq!(stats.sum, Some(150.0));
        assert_eq!(stats.mean, Some(30.0));
        assert_eq!(stats.median, Some(30.0));
        assert!((stats.sd.expect("sd") - (250.0_f64).sqrt()).abs() < 1e-12);
        assert!((stats.sem.expect("sem") - (50.0_f64).sqrt()).abs() < 1e-12);
        assert_eq!(stats.min, Some(10.0));
        assert_eq!(stats.max, Some(50.0));

        // Even-count median & reversed coordinates across multi-subcolumns
        let mut grp = sample_grouped_sheet();
        grp.set_cell(CellCoord::new(0, 0, 2), Cell::number(2.0)).expect("g0");
        grp.set_cell(CellCoord::new(0, 1, 0), Cell::number(8.0)).expect("g1");
        grp.set_cell(CellCoord::new(1, 0, 2), Cell::number(4.0)).expect("g2");
        grp.set_cell(CellCoord::new(1, 1, 0), Cell::number(6.0)).expect("g3");
        let g_stats = SelectionQuickStats::from_data_sheet(&grp, GridCellCoord::new(1, 1, 0), GridCellCoord::new(0, 0, 2));
        assert_eq!(g_stats.selected_cells, 4);
        assert_eq!(g_stats.numeric_count, 4);
        assert_eq!(g_stats.median, Some(5.0));
    }

    #[test]
    fn virtualized_o1_window_and_two_tier_headers_at_one_million_rows() {
        let mut grid = GridToolState::default();
        grid.viewport.viewport_width_px = 1280.0;
        grid.viewport.viewport_height_px = 720.0;
        grid.viewport.scroll_y_px = 12_000_000.0; // row 500_000
        grid.viewport.scroll_x_px = 96.0 * 500.0; // col 500
        let win = grid.visible_window(MAX_ROWS, 1_024);
        assert_eq!(win.start_row, 500_000);
        assert!(win.end_row > 500_000 && win.end_row < 500_040);
        assert_eq!(win.start_col, 500);
        assert!(win.end_col > 500 && win.end_col < 525);

        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(4), 256, 100).expect("256x4");
        grp.data_sets[1].title = String::from("Treated");
        let sheet = DataSheet::new("g-wide", "Wide", TableArchetype::Grouped(grp)).expect("sheet");
        assert_eq!(GridToolState::sheet_total_flat_cols(&sheet), 1_024);

        let mut tool = GridToolState::default();
        tool.viewport.viewport_width_px = 900.0;
        tool.viewport.scroll_x_px = 96.0 * 2.0; // starts mid-group A at subcol Y3
        let hdrs = tool.visible_two_tier_headers(&sheet);
        assert_eq!(hdrs.start_flat_col, 2);
        assert_eq!(hdrs.groups[0].group_index, 0);
        assert_eq!(hdrs.groups[0].subcol_start, 2);
        assert_eq!(hdrs.groups[0].subcolumns[0].label, "Y3");
        assert_eq!(hdrs.groups[1].title, "Treated");
        assert_eq!(hdrs.groups[1].subcolumns[0].label, "Y1");
    }

    #[test]
    fn nested_table_header_and_coord_mapping() {
        let mut nested = NestedTable::new(2, 2, 4).expect("nested");
        nested.groups[1].subcolumn_titles.push(String::from("B:3"));
        for row in &mut nested.groups[1].cells {
            row.push(Cell::empty());
        }
        let sheet = DataSheet::new("n1", "Nested", TableArchetype::Nested(nested)).expect("sheet");
        assert_eq!(GridToolState::sheet_total_flat_cols(&sheet), 5);
        assert_eq!(GridToolState::coord_to_flat_col(&sheet, GridCellCoord::new(0, 1, 2)), Some(4));
        assert_eq!(GridToolState::flat_col_to_coord(&sheet, 4), Some((1, 2)));
    }

    #[test]
    fn block_edge_and_range_navigation_semantics() {
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 10).expect("col");
        col.data_sets[0].cells[0][0] = Cell::number(1.0);
        col.data_sets[0].cells[1][0] = Cell::number(2.0);
        col.data_sets[0].cells[2][0] = Cell::number(3.0);
        // rows 3, 4 empty
        col.data_sets[0].cells[5][0] = Cell::number(10.0);
        col.data_sets[0].cells[6][0] = Cell::number(20.0);
        let sheet = DataSheet::new("d-nav", "Nav", TableArchetype::Column(col)).expect("sheet");

        let mut tool = GridToolState::default();
        tool.select_single(GridCellCoord::new(0, 0, 0));

        // Shift+Cmd+Down jumps to last populated cell before empty boundary (row 2) while anchoring at 0
        let pos = tool.navigate(&sheet, GridNavAction::Arrow { dir: GridDirection::Down, extend: true, jump_block: true });
        assert_eq!(pos, Some(GridCellCoord::new(2, 0, 0)));
        assert_eq!(tool.anchor, Some(GridCellCoord::new(0, 0, 0)));

        // Plain Cmd+Down from row 2 crosses empty rows 3..4 to row 5
        let pos2 = tool.navigate(&sheet, GridNavAction::Arrow { dir: GridDirection::Down, extend: false, jump_block: true });
        assert_eq!(pos2, Some(GridCellCoord::new(5, 0, 0)));
        assert_eq!(tool.anchor, Some(GridCellCoord::new(5, 0, 0)));

        // Tab wraps cleanly to next row
        tool.select_single(GridCellCoord::new(0, 2, 0));
        let wrapped = tool.navigate(&sheet, GridNavAction::Tab { backward: false });
        assert_eq!(wrapped, Some(GridCellCoord::new(1, 0, 0)));
        let back = tool.navigate(&sheet, GridNavAction::Tab { backward: true });
        assert_eq!(back, Some(GridCellCoord::new(0, 2, 0)));
    }

    #[test]
    fn clipboard_tsv_roundtrip_and_intent_emitters() {
        let mut sheet = DataSheet::new("s1", "TSV", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 3).expect("xy"))).expect("sheet");
        sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(1.25)).expect("c0");
        sheet.set_cell(CellCoord::new(0, 1, 0), Cell::excluded_number(9.5)).expect("c1");
        sheet.set_cell(CellCoord::new(1, 0, 0), Cell::text("ctrl")).expect("c2");
        sheet
            .add_linked_region(LinkedDataRegion {
                id: String::from("l1"),
                spec: ImportLinkSpec {
                    source: LinkSource::InternalSheet {
                        sheet_id: SheetId::new("up"),
                        kind: InternalLinkKind::DataSheet,
                        tab_id: None,
                        range: SheetCellRange::default(),
                    },
                    filter: ImportFilterSpec::default(),
                    placement: ImportPlacementSpec::default(),
                },
                target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
                provenance_summary: String::from("link"),
            })
            .expect("add link");

        let tsv = format_selection_tsv(&sheet, GridCellCoord::new(0, 0, 0), GridCellCoord::new(1, 1, 0), ',');
        assert_eq!(tsv, "1,25\t9,5*\nctrl\t");

        let parsed = parse_tsv_clipboard(&tsv, ',', true).expect("parse tsv");
        assert_eq!(parsed[0][0], Cell::number(1.25));
        assert_eq!(parsed[0][1], Cell::excluded_number(9.5));
        assert_eq!(parsed[1][0], Cell::text("ctrl"));
        assert_eq!(parsed[1][1], Cell::empty());

        let mut tool = GridToolState { decimal_separator: ',', ..GridToolState::default() };
        tool.select_single(GridCellCoord::new(0, 0, 0));
        tool.extend_selection(GridCellCoord::new(1, 1, 0));

        let frame = tool.format_visible_window(&sheet);
        assert_eq!(frame.cells[0].link_border, Some(LinkBorderColor::Blue));
        assert!(frame.cells[1].excluded);
        assert!(frame.cells[1].is_selected);

        let ex_intent = tool.emit_exclude_selection_intent(&sheet.id, true).expect("exclude intent");
        assert_eq!(ex_intent.command_id, "table.excludeCells");
        let paste_intent = tool.emit_paste_intent(&sheet.id, "1,5\t2,5*", false, true).expect("paste intent");
        assert_eq!(paste_intent.command_id, "table.paste");
        let set_intent = tool.emit_set_cells_intent(&sheet.id, &[(GridCellCoord::new(0, 0, 0), Cell::number(3.0))]);
        assert_eq!(set_intent.command_id, "table.setCells");
        let link_intent = tool.emit_paste_link_intent(&sheet.id, &sheet.linked_regions[0].spec);
        assert_eq!(link_intent.command_id, "table.pasteLink");

        tool.hover_column_divider = Some(0);
        assert_eq!(tool.cursor(), Cursor::AxisResize);
        tool.hover_column_divider = None;
        assert_eq!(tool.cursor(), Cursor::CellGrid);

        let huge = "\t".repeat(MAX_ROWS + 1);
        assert!(matches!(parse_tsv_clipboard(&huge, '.', true), Err(GridToolError::ClipboardTooLarge { .. })));
    }
}
