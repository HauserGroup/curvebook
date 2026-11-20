//! Virtualized `DataSheet` grid view in `egui 0.37` (`R15.2`, `R15.4`, `gui-layout-spec.md` §8.2).
//!
//! Renders the frozen 3-tier header band, frozen `#` row-index gutter (`48 pt`), collapsible
//! `Row Titles` column (`112 pt` / `34 pt`), two-axis virtualized viewport (`<= 16.67 ms` at
//! `1,000,000` cells), exclusion styling + tooltips, `LinkedDataRegion` guard popovers, and
//! buffered single-commit inline editing through [`PendingUiIntents`].

mod column;
mod headers;
mod inline_edit;
mod linked_regions;
mod multi_archetype;
mod mv_roles;
mod shared;
mod shortcuts;
#[cfg(test)]
mod tests;

use std::ops::Range;

use curvebook_doc::SheetId;
use curvebook_tools::GridCellCoord;
use egui::{Rect, Ui, Vec2};

use crate::UiApp;

use super::{FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens};

pub use column::render_column_grid;

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
        }
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
    if shared::active_column_sheet(app).is_some() {
        return render_column_grid(ui, app, state, render_ctx, meta, intents);
    }
    multi_archetype::render_multi_archetype_grid(ui, app, state, render_ctx, meta, intents)
}
