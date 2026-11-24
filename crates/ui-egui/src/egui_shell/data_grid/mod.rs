//! Virtualized `DataSheet` grid view in `egui 0.37` (`R15.2`, `R15.4`, `gui-layout-spec.md` §8.2).
//!
//! Renders the frozen 3-tier header band, frozen `#` row-index gutter (`48 pt`), collapsible
//! `Row Titles` column (`112 pt` / `34 pt`), two-axis virtualized viewport (`<= 16.67 ms` at
//! `1,000,000` cells), exclusion styling + tooltips, `LinkedDataRegion` guard popovers, and
//! buffered single-commit inline editing through [`PendingUiIntents`].

mod headers;
mod inline_edit;
mod linked_regions;
mod multi_archetype;
mod mv_roles;
mod shared;
mod shortcuts;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::ops::Range;

use curvebook_doc::{SheetId, TableArchetype};
use curvebook_tools::GridCellCoord;
use egui::{Rect, Ui, Vec2};

use crate::UiApp;

use super::{FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens};

pub(in crate::egui_shell) use shared::is_linked_region_broken;

/// Target inside the `Column` grid currently undergoing buffered inline text editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridInlineEditTarget {
    Cell(GridCellCoord),
    ColumnTitle { col: usize },
    RowTitle { row: usize },
}

/// Local `TextEdit` draft buffered in UI state until `Enter`, `Tab`, or focus-loss commits once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridInlineEditBuffer {
    pub target: GridInlineEditTarget,
    pub draft: String,
    pub focused_once: bool,
}

/// Active read-only guard popover displayed when attempting to edit a `LinkedDataRegion` cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedRegionPopover {
    pub region_id: String,
    pub upstream_sheet_id: Option<SheetId>,
    pub message: String,
}

/// Optional find-in-sheet highlight overlay painted on visible matching cells, column titles,
/// and row titles (`R15`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataGridFindHighlight {
    pub active_cell: Option<GridCellCoord>,
    pub active_col_title: Option<usize>,
    pub active_row_title: Option<usize>,
    pub matching_cells: HashSet<GridCellCoord>,
    pub matching_col_titles: HashSet<usize>,
    pub matching_row_titles: HashSet<usize>,
}

/// Deferred scroll target resolved on the next [`render_data_grid`] frame once layout geometry is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataGridScrollTarget {
    Cell(GridCellCoord),
    ColumnTitle { col: usize },
    RowTitle { row: usize },
}

/// Persistent transient UI state for [`render_column_grid`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnGridEguiState {
    pub scroll_offset_pt: Vec2,
    pub row_titles_collapsed: bool,
    pub inline_edit: Option<GridInlineEditBuffer>,
    pub active_link_popover: Option<LinkedRegionPopover>,
    pub(super) context_menu: Option<shared::ContextMenuState>,
    pub last_visible_rows: Range<usize>,
    pub last_visible_subcols: Range<usize>,
    pub ellipsized_headers: usize,
    pub record_inspect_meta: bool,
    pub active_find_matches: Option<DataGridFindHighlight>,
    pub pending_scroll_target: Option<DataGridScrollTarget>,
}

impl Default for ColumnGridEguiState {
    fn default() -> Self {
        Self {
            scroll_offset_pt: Vec2::ZERO,
            row_titles_collapsed: false,
            inline_edit: None,
            active_link_popover: None,
            context_menu: None,
            last_visible_rows: 0..0,
            last_visible_subcols: 0..0,
            ellipsized_headers: 0,
            record_inspect_meta: true,
            active_find_matches: None,
            pending_scroll_target: None,
        }
    }
}

impl ColumnGridEguiState {
    /// Request scrolling the grid viewport on the next render so `(row, col, subcol)` is brought into view.
    pub fn scroll_to_cell(&mut self, row: usize, col: usize, subcol: usize) {
        self.pending_scroll_target = Some(DataGridScrollTarget::Cell(GridCellCoord::new(row, col, subcol)));
    }

    /// Request scrolling the grid viewport horizontally on the next render so column `col` is brought into view.
    pub fn scroll_to_column_title(&mut self, col: usize) {
        self.pending_scroll_target = Some(DataGridScrollTarget::ColumnTitle { col });
    }

    /// Request scrolling the grid viewport vertically on the next render so row `row` is brought into view.
    pub fn scroll_to_row_title(&mut self, row: usize) {
        self.pending_scroll_target = Some(DataGridScrollTarget::RowTitle { row });
    }

    /// Request scrolling the grid viewport on the next render to bring `target` into view.
    pub fn scroll_to_target(&mut self, target: DataGridScrollTarget) {
        self.pending_scroll_target = Some(target);
    }
}

/// Per-frame layout/rendering options passed into [`render_column_grid`].
#[derive(Debug, Clone, Copy)]
pub struct ColumnGridRenderContext<'a> {
    pub tokens: &'a Tokens,
    pub metrics: LayoutMetrics,
    pub modal_blocked: bool,
}

impl<'a> ColumnGridRenderContext<'a> {
    /// Construct render options inheriting `tokens.metrics`.
    #[must_use]
    pub const fn new(tokens: &'a Tokens, modal_blocked: bool) -> Self {
        Self { tokens, metrics: tokens.metrics, modal_blocked }
    }
}

/// Frame geometry and inspection diagnostics returned by [`render_column_grid`].
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnGridFrameReport {
    pub grid_rect_pt: Rect,
    pub rendered_cells: usize,
    pub visible_rows: Range<usize>,
    pub visible_subcols: Range<usize>,
    pub vertical_scrollbar_rect_pt: Option<Rect>,
    pub horizontal_scrollbar_rect_pt: Option<Rect>,
    pub rendered_modal_guard_open: bool,
    pub outside_click_blocked: bool,
}

impl Default for ColumnGridFrameReport {
    fn default() -> Self {
        Self {
            grid_rect_pt: Rect::NOTHING,
            rendered_cells: 0,
            visible_rows: 0..0,
            visible_subcols: 0..0,
            vertical_scrollbar_rect_pt: None,
            horizontal_scrollbar_rect_pt: None,
            rendered_modal_guard_open: false,
            outside_click_blocked: false,
        }
    }
}

/// Canonical alias for unified multi-archetype grid state (`R15.4`).
pub type DataGridEguiState = ColumnGridEguiState;
/// Canonical alias for unified multi-archetype grid render context (`R15.4`).
pub type DataGridRenderContext<'a> = ColumnGridRenderContext<'a>;
/// Canonical alias for unified multi-archetype grid frame report (`R15.4`).
pub type DataGridFrameReport = ColumnGridFrameReport;

/// Render the active `Column` `DataSheet` virtualized grid into `ui` without mutating `Session`.
///
/// Delegates to the unified multi-archetype grid renderer (`x_subcols == 0`), returning an empty
/// report if the active sheet is not a `Column` table.
pub fn render_column_grid(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ColumnGridEguiState,
    render_ctx: ColumnGridRenderContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ColumnGridFrameReport {
    let is_column = app
        .document()
        .active_sheet_id
        .as_ref()
        .and_then(|id| app.document().data_sheet(id))
        .is_some_and(|ds| matches!(ds.archetype, TableArchetype::Column(_)));
    if !is_column {
        let available = ui.available_rect_before_wrap();
        state.ellipsized_headers = 0;
        state.last_visible_rows = 0..0;
        state.last_visible_subcols = 0..0;
        return ColumnGridFrameReport {
            grid_rect_pt: available,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            ..ColumnGridFrameReport::default()
        };
    }
    multi_archetype::render_multi_archetype_grid(ui, app, state, render_ctx, meta, intents)
}

/// Render the active `DataSheet` natively for all 8 table archetypes (`Column`, `XY`, `Grouped`,
/// `Contingency`, `Survival`, `PartsOfWhole`, `MultipleVariables`, `Nested`) without mutating
/// `Session` during layout (`R15.4`, `gui-layout-spec.md` §8.2).
pub fn render_data_grid(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut DataGridEguiState,
    render_ctx: DataGridRenderContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> DataGridFrameReport {
    multi_archetype::render_multi_archetype_grid(ui, app, state, render_ctx, meta, intents)
}
