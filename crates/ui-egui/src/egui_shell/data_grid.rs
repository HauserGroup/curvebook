//! Virtualized `Column` `DataSheet` grid view in `egui 0.37` (`R15.2`, `gui-layout-spec.md` §8.2).
//!
//! Renders the frozen 3-tier header band, frozen `#` row-index gutter (`48 pt`), collapsible
//! `Row Titles` column (`112 pt` / `34 pt`), two-axis virtualized viewport (`<= 16.67 ms` at
//! `1,000,000` cells), exclusion styling + tooltips, `LinkedDataRegion` guard popovers, and
//! buffered single-commit inline editing through [`PendingUiIntents`].

use std::ops::Range;

use curvebook_doc::SheetId;
use curvebook_tools::GridCellCoord;
use egui::{Rect, Ui, Vec2};

use crate::UiApp;

use super::{FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens};

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

/// Render the active `Column` `DataSheet` virtualized grid into `ui` without mutating `Session`.
///
/// Wave 2 base stub: reserves the available center-stage rectangle cleanly; full implementation
/// lands in `r15-grid-col` (`R15.2`).
pub fn render_column_grid(
    ui: &mut Ui,
    _app: &UiApp,
    state: &mut ColumnGridEguiState,
    render_ctx: ColumnGridRenderContext<'_>,
    _meta: &mut FrameInspectMeta,
    _intents: &mut PendingUiIntents,
) -> ColumnGridFrameReport {
    let available = ui.available_rect_before_wrap();
    let _ = (render_ctx.tokens, render_ctx.metrics, render_ctx.modal_blocked);
    state.last_visible_rows = 0..0;
    state.last_visible_subcols = 0..0;
    ColumnGridFrameReport {
        grid_rect_pt: available,
        rendered_modal_guard_open: state.active_link_popover.is_some(),
        ..ColumnGridFrameReport::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{ColumnGridEguiState, ColumnGridRenderContext};
    use crate::egui_shell::Tokens;

    #[test]
    fn default_grid_state_enables_inspect_meta_and_inherits_metrics() {
        let state = ColumnGridEguiState::default();
        assert!(state.record_inspect_meta);
        assert!(!state.row_titles_collapsed);
        let tokens = Tokens::default();
        let ctx = ColumnGridRenderContext::new(&tokens, false);
        assert_eq!(ctx.metrics.grid_row_h, 24.0);
        assert!(!ctx.modal_blocked);
    }
}
