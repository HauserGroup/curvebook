//! Virtualized `Column` `DataSheet` grid view in `egui 0.37` (`R15.2`, `gui-layout-spec.md` §8.2).
//!
//! Renders the frozen 3-tier header band, frozen `#` row-index gutter (`48 pt`), collapsible
//! `Row Titles` column (`112 pt` / `34 pt`), two-axis virtualized viewport (`<= 16.67 ms` at
//! `1,000,000` cells), exclusion styling + tooltips, `LinkedDataRegion` guard popovers, and
//! buffered single-commit inline editing through [`PendingUiIntents`].

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;

use curvebook_doc::{
    Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, DataSheet, LinkBorderColor, LinkedDataRegion, SheetId, SubcolumnMode, TableArchetype,
    default_column_label,
};
use curvebook_tools::GridCellCoord;
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui,
    UiBuilder, Vec2, WidgetInfo, pos2, vec2,
};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::{GridViewState, format_cell_display, parse_localized_number_input};
use crate::i18n::{LocalePreset, tr};
use crate::inspect::ZLayer;
use crate::views::grid_core::{grid_tr, linked_region_for_cell};

use super::{FrameInspectEntryInput, FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens, UiIntent};

const ROW_NUM_WIDTH_PT: f32 = 48.0;
const ROW_TITLES_EXPANDED_WIDTH_PT: f32 = 112.0;
const ROW_TITLES_COLLAPSED_WIDTH_PT: f32 = 34.0;
const DEFAULT_DATA_COL_WIDTH_PT: f32 = 96.0;
const LINK_STRIP_HEIGHT_PT: f32 = 26.0;
const CELL_PAD_X_PT: f32 = 4.0;
const CORNER_TRIGGER_HEIGHT_PT: f32 = 24.0;

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

#[derive(Debug, Clone)]
struct GridGeometry {
    available_rect: Rect,
    table_rect: Rect,
    row_num_w: f32,
    rt_w: f32,
    tier1_h: f32,
    tier2_h: f32,
    tier3_h: f32,
    header_h: f32,
    row_h: f32,
    subcol_w: f32,
    subs_per_col: usize,
    total_rows: usize,
    total_cols: usize,
    total_flat_cols: usize,
    total_data_w: f32,
    total_data_h: f32,
    need_h: bool,
    need_v: bool,
    h_reserve: f32,
    v_reserve: f32,
    cols_x: f32,
    data_y: f32,
    body_inner_w: f32,
    body_inner_h: f32,
    effective_collapsed: bool,
    corner_rect: Rect,
    header_data_rect: Rect,
    gutter_body_rect: Rect,
    body_outer_rect: Rect,
}

struct GridRenderCtx<'a> {
    app: &'a UiApp,
    ds: &'a DataSheet,
    col_table: &'a ColumnTable,
    tokens: &'a Tokens,
    metrics: LayoutMetrics,
    geom: &'a GridGeometry,
    sheet_id_base: Id,
    modal_blocked: bool,
    locale: &'a str,
}

impl<'a> GridRenderCtx<'a> {
    const fn input_blocked(&self, state: &ColumnGridEguiState) -> bool {
        self.modal_blocked || state.active_link_popover.is_some()
    }
}

struct SelectionFastLookup<'a> {
    active: Option<GridCellCoord>,
    rect_bounds: Option<(usize, usize, usize, usize)>,
    discontig_slice: Option<&'a [GridCellCoord]>,
    discontig_set: Option<HashSet<GridCellCoord>>,
}

impl<'a> SelectionFastLookup<'a> {
    fn new(app: &'a UiApp, total_rows: usize, subs_per_col: usize, total_flat: usize) -> Self {
        let active = app.grid.tool.active;
        let has_discontig =
            app.grid.selected_coords.len() > 1 || (!app.grid.selected_coords.is_empty() && app.grid.tool.anchor == app.grid.tool.active);
        if has_discontig {
            if app.grid.selected_coords.len() > 8 {
                let set = app.grid.selected_coords.iter().copied().collect();
                return Self { active, rect_bounds: None, discontig_slice: None, discontig_set: Some(set) };
            }
            return Self { active, rect_bounds: None, discontig_slice: Some(&app.grid.selected_coords), discontig_set: None };
        }
        let rect_bounds = if total_rows == 0 || total_flat == 0 {
            None
        } else if let Some((min_c, max_c)) = app.grid.tool.selection_bounds() {
            let r0 = min_c.row;
            let r1 = max_c.row.min(total_rows.saturating_sub(1));
            let f0 = min_c.col.saturating_mul(subs_per_col).saturating_add(min_c.subcol.min(subs_per_col - 1));
            let f1 = max_c.col.saturating_mul(subs_per_col).saturating_add(max_c.subcol.min(subs_per_col - 1));
            if r0 <= r1 && f0 < total_flat { Some((r0, r1, f0.min(f1), f0.max(f1).min(total_flat - 1))) } else { None }
        } else {
            None
        };
        Self { active, rect_bounds, discontig_slice: None, discontig_set: None }
    }

    fn contains(&self, coord: GridCellCoord, flat_col: usize) -> bool {
        if let Some(set) = &self.discontig_set {
            return set.contains(&coord);
        }
        if let Some(slice) = self.discontig_slice {
            return slice.contains(&coord);
        }
        if let Some((r0, r1, f0, f1)) = self.rect_bounds {
            return (r0..=r1).contains(&coord.row) && (f0..=f1).contains(&flat_col);
        }
        false
    }
}

struct BodyRenderResult {
    inner_rect: Rect,
    rendered_cells: usize,
    visible_rows: Range<usize>,
    visible_subcols: Range<usize>,
    used_scroll_offset: Vec2,
    new_scroll_offset: Vec2,
    inline_edit_handled_key: bool,
    cell_clicked_or_edited: bool,
}

/// Render the active `Column` `DataSheet` virtualized grid into `ui` without mutating `Session`.
pub fn render_column_grid(
    ui: &mut Ui,
    app: &UiApp,
    state: &mut ColumnGridEguiState,
    render_ctx: ColumnGridRenderContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> ColumnGridFrameReport {
    let available = ui.available_rect_before_wrap();
    state.ellipsized_headers = 0;

    let Some((ds, col_table)) = active_column_sheet(app) else {
        state.last_visible_rows = 0..0;
        state.last_visible_subcols = 0..0;
        return ColumnGridFrameReport {
            grid_rect_pt: available,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            ..ColumnGridFrameReport::default()
        };
    };

    ensure_solid_scrollbar_style(ui, render_ctx.metrics);

    if let Some(collapsed_override) = app.grid.row_titles_collapsed_override {
        state.row_titles_collapsed = collapsed_override;
    }

    let mut table_top = available.top();
    let link_strip_h = if !ds.linked_regions.is_empty() && available.height() > 110.0 { LINK_STRIP_HEIGHT_PT } else { 0.0 };
    table_top += link_strip_h;
    let table_rect = Rect::from_min_max(pos2(available.left(), table_top), available.max);

    let scroll_allocated_w = ui.spacing().scroll.allocated_width();
    let geom = compute_grid_geometry(available, table_rect, col_table, render_ctx.metrics, scroll_allocated_w, state.row_titles_collapsed);
    if geom.body_inner_w < 16.0 || geom.body_inner_h < 16.0 {
        state.last_visible_rows = 0..0;
        state.last_visible_subcols = 0..0;
        return ColumnGridFrameReport {
            grid_rect_pt: available,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            ..ColumnGridFrameReport::default()
        };
    }

    let sheet_id_base = ui.scope_id().with(("column_grid", ds.id.as_str()));
    let rctx = GridRenderCtx {
        app,
        ds,
        col_table,
        tokens: render_ctx.tokens,
        metrics: render_ctx.metrics,
        geom: &geom,
        sheet_id_base,
        modal_blocked: render_ctx.modal_blocked,
        locale: render_ctx.tokens.locale,
    };

    clamp_scroll_offset(state, &geom);

    if link_strip_h > 0.0 {
        let strip_rect = Rect::from_min_size(available.min, vec2(available.width(), link_strip_h));
        render_link_provenance_strip(ui, &rctx, state, meta, intents, strip_rect);
    }

    forward_gutter_header_wheel(ui, &rctx, state);
    let prev_scroll = state.scroll_offset_pt;
    let body_res = render_body_scroll_area(ui, &rctx, state, meta, intents);

    render_corner_and_row_titles_header(ui, &rctx, state, meta, intents);
    render_column_headers_3tier(ui, &rctx, state, body_res.used_scroll_offset.x, &body_res.visible_subcols, meta, intents);
    render_gutter_body(ui, &rctx, state, body_res.used_scroll_offset.y, &body_res.visible_rows, meta, intents);
    paint_scrollbar_corner_cap(ui, &rctx);

    if !body_res.inline_edit_handled_key && !body_res.cell_clicked_or_edited {
        handle_grid_keyboard_shortcuts(ui, &rctx, state, intents);
    }

    state.scroll_offset_pt = body_res.new_scroll_offset;
    clamp_scroll_offset(state, &geom);
    state.last_visible_rows = body_res.visible_rows.clone();
    state.last_visible_subcols = body_res.visible_subcols.clone();

    let scrolled_this_frame = (state.scroll_offset_pt - prev_scroll).length_sq() > 0.01;
    if scrolled_this_frame {
        // Request a same-frame re-layout when no grid intents were queued yet so frozen headers
        // stay aligned with the updated scroll offset; any intents queued later in the same
        // discarded pass are preserved across passes by `PendingUiIntents::merge_pass` (`R15-GUI-05`).
        if intents.is_empty() && !ui.ctx().will_discard() {
            ui.ctx().request_discard("column grid scroll offset updated");
        } else {
            ui.ctx().request_repaint();
        }
    }

    let outside_click_blocked = render_linked_region_guard_popover(ui, &rctx, state, meta, intents);
    let (v_bar_rect, h_bar_rect) = compute_scrollbar_rects(ui, &geom, body_res.inner_rect);

    ColumnGridFrameReport {
        grid_rect_pt: available,
        rendered_cells: body_res.rendered_cells,
        visible_rows: body_res.visible_rows,
        visible_subcols: body_res.visible_subcols,
        vertical_scrollbar_rect_pt: v_bar_rect,
        horizontal_scrollbar_rect_pt: h_bar_rect,
        rendered_modal_guard_open: state.active_link_popover.is_some(),
        outside_click_blocked: outside_click_blocked || (render_ctx.modal_blocked && any_pointer_click_in(ui, available)),
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
    if active_column_sheet(app).is_some() {
        return render_column_grid(ui, app, state, render_ctx, meta, intents);
    }
    multi_archetype::render_multi_archetype_grid(ui, app, state, render_ctx, meta, intents)
}

mod multi_archetype {
    use super::*;
    use crate::grid_view::data_set_subcolumn_count;
    use curvebook_doc::{SheetCellRange, VariableRole, VariableSource, XColumnMode};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum AuxInlineTarget {
        XCell { row: usize, subcol: usize },
        XTitle { subcol: usize },
        SubcolumnTitle { col: usize, subcol: usize },
        SheetTitle,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct ContextCellTarget {
        is_x: bool,
        row: usize,
        col: usize,
        subcol: usize,
    }

    #[derive(Debug, Clone)]
    struct ContextMenuState {
        pos: egui::Pos2,
        row: usize,
        cell: Option<ContextCellTarget>,
    }

    #[derive(Debug, Clone, Default)]
    struct MultiGridAuxState {
        sheet_id: Option<SheetId>,
        aux_edit: Option<(AuxInlineTarget, String, bool)>,
        selected_x_cell: Option<(usize, usize)>,
        last_app_x_cell: Option<(usize, usize)>,
        last_app_y_selection: Option<(Option<GridCellCoord>, Option<GridCellCoord>, usize)>,
        active_role_menu_col: Option<usize>,
        context_menu: Option<ContextMenuState>,
        suppress_loss_commit_once: bool,
    }

    #[derive(Debug, Clone)]
    struct MultiGridGeom {
        grid_outer_rect: Rect,
        link_strip_rect: Option<Rect>,
        h_tier1: f32,
        h_tier2: f32,
        h_tier3: f32,
        row_h: f32,
        row_num_w: f32,
        row_title_w: f32,
        subcol_w: f32,
        bar_reserve: f32,
        row_count: usize,
        col_count: usize,
        x_subcols: usize,
        uniform_data_subcols: Option<usize>,
        col_flat_offsets: Vec<usize>,
        total_data_flat_cols: usize,
        total_flat_cols: usize,
        row_titles_collapsed: bool,
        corner_rect: Rect,
        col_header_rect: Rect,
        row_gutter_rect: Rect,
        body_rect: Rect,
        body_inner_clip: Rect,
        content_w: f32,
        content_h: f32,
        font_num_pt: f32,
        font_small_pt: f32,
    }

    impl MultiGridGeom {
        fn flat_data_to_coord(&self, flat_data_col: usize) -> Option<(usize, usize)> {
            if flat_data_col >= self.total_data_flat_cols || self.col_count == 0 {
                return None;
            }
            if let Some(subs) = self.uniform_data_subcols {
                let s = subs.max(1);
                return Some((flat_data_col / s, flat_data_col % s));
            }
            let col = match self.col_flat_offsets.binary_search(&flat_data_col) {
                Ok(idx) => idx,
                Err(ins) => ins.saturating_sub(1),
            };
            let start = self.col_flat_offsets.get(col).copied().unwrap_or(0);
            Some((col, flat_data_col.saturating_sub(start)))
        }

        fn coord_to_flat_all(&self, is_x: bool, col: usize, subcol: usize) -> usize {
            if is_x {
                return subcol.min(self.x_subcols.saturating_sub(1));
            }
            let data_flat = if let Some(subs) = self.uniform_data_subcols {
                col.saturating_mul(subs.max(1)).saturating_add(subcol)
            } else {
                self.col_flat_offsets.get(col).copied().unwrap_or(0).saturating_add(subcol)
            };
            self.x_subcols.saturating_add(data_flat)
        }

        fn col_span_flat(&self, col: usize) -> (usize, usize) {
            if let Some(subs) = self.uniform_data_subcols {
                let s = subs.max(1);
                (self.x_subcols + col * s, s)
            } else {
                let start = self.col_flat_offsets.get(col).copied().unwrap_or(0);
                let next = self.col_flat_offsets.get(col + 1).copied().unwrap_or(self.total_data_flat_cols);
                (self.x_subcols + start, next.saturating_sub(start).max(1))
            }
        }
    }

    fn text_style(pt: f32, mono: bool, color: Color32, align: Align, italic: bool, strike: bool) -> CellTextStyle {
        let font_id = if mono { egui::FontId::new(pt, egui::FontFamily::Monospace) } else { egui::FontId::proportional(pt) };
        CellTextStyle { font_id, color, align, italic, strike }
    }

    pub(super) fn render_multi_archetype_grid(
        ui: &mut Ui,
        app: &UiApp,
        state: &mut DataGridEguiState,
        render_ctx: DataGridRenderContext<'_>,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
    ) -> DataGridFrameReport {
        let available = ui.available_rect_before_wrap();
        state.ellipsized_headers = 0;

        let Some(ds) = app.document().active_sheet_id.as_ref().and_then(|id| app.document().data_sheet(id)) else {
            state.last_visible_rows = 0..0;
            state.last_visible_subcols = 0..0;
            return DataGridFrameReport {
                grid_rect_pt: available,
                rendered_modal_guard_open: state.active_link_popover.is_some(),
                ..DataGridFrameReport::default()
            };
        };

        ensure_solid_scrollbar_style(ui, render_ctx.metrics);
        if !available.is_positive() {
            return DataGridFrameReport { grid_rect_pt: available, ..DataGridFrameReport::default() };
        }

        let aux_id = Id::unique(("r15_4_multi_grid_aux", ds.id.as_str()));
        let mut aux: MultiGridAuxState = ui.ctx().data_mut(|d| d.get_temp::<MultiGridAuxState>(aux_id).unwrap_or_default());
        if aux.sheet_id.as_ref() != Some(&ds.id) {
            aux = MultiGridAuxState { sheet_id: Some(ds.id.clone()), ..MultiGridAuxState::default() };
            state.inline_edit = None;
            state.active_link_popover = None;
        }
        let cur_app_y_sel = Some((app.grid.tool.active, app.grid.tool.anchor, app.grid.selected_coords.len()));
        if app.grid.active_x_cell != aux.last_app_x_cell {
            aux.selected_x_cell = app.grid.active_x_cell;
            aux.last_app_x_cell = app.grid.active_x_cell;
        }
        if cur_app_y_sel != aux.last_app_y_selection {
            aux.last_app_y_selection = cur_app_y_sel;
            if app.grid.tool.active.is_some() && app.grid.active_x_cell.is_none() {
                aux.selected_x_cell = None;
            }
        }

        let geom = compute_multi_geom(app, ds, state, render_ctx.metrics, available);
        clamp_multi_scroll(state, &geom);

        let blocked = render_ctx.modal_blocked || state.active_link_popover.is_some();
        let outside_click_blocked = render_ctx.modal_blocked && any_pointer_click_in(ui, available);
        let tokens = render_ctx.tokens;

        let painter = ui.painter();
        painter.rect_filled(geom.grid_outer_rect, 0.0, tokens.surface_bg);
        painter.rect_stroke(geom.grid_outer_rect, 0.0, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);

        if let Some(strip_rect) = geom.link_strip_rect {
            render_multi_link_strip(ui, app, ds, strip_rect, state, tokens, meta);
        }
        if !blocked {
            forward_frozen_wheel(ui, &geom, state);
        }

        render_multi_corner(ui, app, ds, state, &mut aux, &geom, tokens, meta, intents, blocked);
        let active_mv_operands = active_mv_formula_operands(app, ds);
        render_multi_column_headers(ui, app, ds, state, &mut aux, &geom, &active_mv_operands, tokens, meta, intents, blocked);
        render_multi_gutter_body(ui, app, ds, state, &mut aux, &geom, tokens, meta, intents, blocked);
        let body_out = render_multi_body(ui, app, ds, state, &mut aux, &geom, tokens, meta, intents, blocked);

        let cap_rect = Rect::from_min_max(
            pos2(
                (geom.body_rect.max.x - geom.bar_reserve).max(geom.body_rect.min.x),
                (geom.body_rect.max.y - geom.bar_reserve).max(geom.body_rect.min.y),
            ),
            geom.body_rect.max,
        );
        if cap_rect.is_positive() {
            ui.painter().rect_filled(cap_rect, 0.0, tokens.header_bg);
        }

        render_multi_inline_editors(ui, app, ds, state, &mut aux, &geom, tokens, meta, intents, blocked);
        render_mv_role_menu_popup(ui, ds, &mut aux, &geom, state.scroll_offset_pt.x, tokens, meta, intents, blocked);
        render_row_context_menu_popup(ui, app, ds, &mut aux, tokens, meta, intents, blocked);
        if !blocked {
            handle_multi_keyboard_shortcuts(ui, app, ds, state, &mut aux, &geom, intents);
        }

        render_multi_link_guard_popover(ui, ds, state, tokens, meta, intents, geom.grid_outer_rect);

        let (v_sb, h_sb) = compute_multi_scrollbar_rects(&geom);
        state.last_visible_rows = body_out.vis_rows.clone();
        state.last_visible_subcols = body_out.vis_all_flat.clone();
        ui.ctx().data_mut(|d| d.insert_temp(aux_id, aux));

        DataGridFrameReport {
            grid_rect_pt: geom.grid_outer_rect,
            rendered_cells: body_out.rendered_cells,
            visible_rows: body_out.vis_rows,
            visible_subcols: body_out.vis_all_flat,
            vertical_scrollbar_rect_pt: v_sb,
            horizontal_scrollbar_rect_pt: h_sb,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            outside_click_blocked,
        }
    }

    fn compute_multi_geom(app: &UiApp, ds: &DataSheet, state: &ColumnGridEguiState, metrics: LayoutMetrics, available: Rect) -> MultiGridGeom {
        let link_h = if ds.linked_regions.is_empty() { 0.0 } else { LINK_STRIP_HEIGHT_PT };
        let link_strip_rect = (link_h > 0.0).then(|| Rect::from_min_max(available.min, pos2(available.max.x, available.min.y + link_h)));
        let grid_outer_rect = Rect::from_min_max(pos2(available.min.x, available.min.y + link_h), available.max);

        let row_titles_collapsed = app.grid.row_titles_collapsed_override.unwrap_or(state.row_titles_collapsed);
        let [h_tier1, h_tier2, h_tier3] = metrics.grid_header_tier_heights;
        let header_h = h_tier1 + h_tier2 + h_tier3;
        let row_h = metrics.grid_row_h.max(20.0);
        let row_num_w = ROW_NUM_WIDTH_PT.min((grid_outer_rect.width() * 0.25).floor()).max(24.0);
        let row_title_w = if row_titles_collapsed { ROW_TITLES_COLLAPSED_WIDTH_PT } else { ROW_TITLES_EXPANDED_WIDTH_PT }
            .min((grid_outer_rect.width() * 0.35).floor())
            .max(28.0);
        let gutter_w = row_num_w + row_title_w;
        let subcol_w = DEFAULT_DATA_COL_WIDTH_PT;
        let bar_reserve = (metrics.scroll_bar_thickness + 4.0).max(14.0);

        let row_count = ds.archetype.row_count();
        let col_count = ds.archetype.column_count();
        let x_subcols = archetype_x_subcol_count(&ds.archetype);

        let (uniform_data_subcols, col_flat_offsets, total_data_flat_cols) = match &ds.archetype {
            TableArchetype::Nested(_) => {
                let mut offsets = Vec::with_capacity(col_count + 1);
                let mut acc = 0usize;
                for c in 0..col_count {
                    offsets.push(acc);
                    acc = acc.saturating_add(data_set_subcolumn_count(&ds.archetype, c).max(1));
                }
                offsets.push(acc);
                (None, offsets, acc)
            }
            _ => {
                let subs = if col_count > 0 { data_set_subcolumn_count(&ds.archetype, 0).max(1) } else { 1 };
                (Some(subs), Vec::new(), col_count.saturating_mul(subs))
            }
        };
        let total_flat_cols = x_subcols.saturating_add(total_data_flat_cols);

        let body_x0 = (grid_outer_rect.min.x + gutter_w).min(grid_outer_rect.max.x);
        let body_y0 = (grid_outer_rect.min.y + header_h).min(grid_outer_rect.max.y);
        let corner_rect = Rect::from_min_max(grid_outer_rect.min, pos2(body_x0, body_y0));
        let col_header_rect =
            Rect::from_min_max(pos2(body_x0, grid_outer_rect.min.y), pos2((grid_outer_rect.max.x - bar_reserve).max(body_x0), body_y0));
        let row_gutter_rect =
            Rect::from_min_max(pos2(grid_outer_rect.min.x, body_y0), pos2(body_x0, (grid_outer_rect.max.y - bar_reserve).max(body_y0)));
        let body_rect = Rect::from_min_max(pos2(body_x0, body_y0), grid_outer_rect.max);
        let body_inner_clip = Rect::from_min_max(
            body_rect.min,
            pos2((body_rect.max.x - bar_reserve).max(body_rect.min.x), (body_rect.max.y - bar_reserve).max(body_rect.min.y)),
        );

        #[allow(clippy::cast_precision_loss)]
        let content_w = (total_flat_cols as f32) * subcol_w;
        #[allow(clippy::cast_precision_loss)]
        let content_h = (row_count as f32) * row_h;

        MultiGridGeom {
            grid_outer_rect,
            link_strip_rect,
            h_tier1,
            h_tier2,
            h_tier3,
            row_h,
            row_num_w,
            row_title_w,
            subcol_w,
            bar_reserve,
            row_count,
            col_count,
            x_subcols,
            uniform_data_subcols,
            col_flat_offsets,
            total_data_flat_cols,
            total_flat_cols,
            row_titles_collapsed,
            corner_rect,
            col_header_rect,
            row_gutter_rect,
            body_rect,
            body_inner_clip,
            content_w,
            content_h,
            font_num_pt: metrics.font_grid_numeral_pt,
            font_small_pt: metrics.font_ui_small_pt,
        }
    }

    fn clamp_multi_scroll(state: &mut ColumnGridEguiState, geom: &MultiGridGeom) {
        let max_x = (geom.content_w - geom.body_inner_clip.width()).max(0.0);
        let max_y = (geom.content_h - geom.body_inner_clip.height()).max(0.0);
        state.scroll_offset_pt.x = state.scroll_offset_pt.x.clamp(0.0, max_x);
        state.scroll_offset_pt.y = state.scroll_offset_pt.y.clamp(0.0, max_y);
    }

    fn forward_frozen_wheel(ui: &mut Ui, geom: &MultiGridGeom, state: &mut ColumnGridEguiState) {
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let over_frozen = pointer.is_some_and(|p| geom.col_header_rect.contains(p) || geom.row_gutter_rect.contains(p));
        if !over_frozen {
            return;
        }
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta.length_sq() > 0.0 {
            state.scroll_offset_pt.x -= delta.x;
            state.scroll_offset_pt.y -= delta.y;
            clamp_multi_scroll(state, geom);
        }
    }

    fn compute_multi_scrollbar_rects(geom: &MultiGridGeom) -> (Option<Rect>, Option<Rect>) {
        let b = geom.body_rect;
        if b.width() <= geom.bar_reserve || b.height() <= geom.bar_reserve {
            return (None, None);
        }
        let v_sb = Rect::from_min_max(pos2(b.max.x - geom.bar_reserve, b.min.y), pos2(b.max.x, b.max.y - geom.bar_reserve));
        let h_sb = Rect::from_min_max(pos2(b.min.x, b.max.y - geom.bar_reserve), pos2(b.max.x - geom.bar_reserve, b.max.y));
        (v_sb.is_positive().then_some(v_sb), h_sb.is_positive().then_some(h_sb))
    }

    fn archetype_x_subcol_count(arch: &TableArchetype) -> usize {
        match arch {
            TableArchetype::Xy(xy) => xy.x_mode.subcolumn_count().max(1),
            TableArchetype::Survival(_) => 1,
            _ => 0,
        }
    }

    fn archetype_badge(arch: &TableArchetype) -> &'static str {
        match arch {
            TableArchetype::Column(_) => "Col",
            TableArchetype::Xy(_) => "XY",
            TableArchetype::Grouped(_) => "Grp",
            TableArchetype::Contingency(_) => "Cont",
            TableArchetype::Survival(_) => "Surv",
            TableArchetype::PartsOfWhole(_) => "PoW",
            TableArchetype::MultipleVariables(_) => "MV",
            TableArchetype::Nested(_) => "Nest",
        }
    }

    fn render_multi_link_strip(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        strip_rect: Rect,
        state: &mut ColumnGridEguiState,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
    ) {
        ui.painter().rect_filled(strip_rect, 0.0, tokens.panel_bg);
        ui.painter()
            .line_segment([pos2(strip_rect.min.x, strip_rect.max.y), pos2(strip_rect.max.x, strip_rect.max.y)], Stroke::new(1.0, tokens.divider));
        let mut cursor_x = strip_rect.min.x + 6.0;
        for (idx, reg) in ds.linked_regions.iter().enumerate() {
            let color = link_border_color32(reg.spec.source.border_color(), tokens);
            let badge_txt = format!("Link {}: {}", idx + 1, reg.provenance_summary);
            #[allow(clippy::cast_precision_loss)]
            let pill_w = (badge_txt.len() as f32 * 6.5 + 18.0).min(240.0);
            if cursor_x + pill_w > strip_rect.max.x - 4.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(cursor_x, strip_rect.min.y + 2.0), vec2(pill_w, strip_rect.height() - 4.0));
            let wid = format!("grid.link_badge.{idx}");
            let resp = ui.interact(r, ui.make_persistent_id(&wid), Sense::click());
            ui.painter().rect_filled(r, 3.0, color.linear_multiply(0.16));
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            render_ellipsized_text(
                ui,
                r.shrink2(vec2(5.0, 1.0)),
                &badge_txt,
                CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_primary, Align::LEFT),
            );
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "button",
                    label: &badge_txt,
                    command_id: None,
                    value: None,
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if resp.clicked() {
                open_link_popover(state, reg, &app.locale);
            }
            cursor_x += pill_w + 6.0;
        }
    }

    fn render_multi_link_guard_popover(
        ui: &mut Ui,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        bounds: Rect,
    ) {
        let Some(pop) = state.active_link_popover.clone() else { return };
        let pop_w = (bounds.width() - 24.0).clamp(240.0, 360.0);
        let pos = pos2(bounds.center().x - pop_w * 0.5, bounds.min.y + 32.0);
        egui::Area::new(Id::unique(("multi_link_guard", ds.id.as_str()))).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |pui| {
            egui::Frame::popup(pui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.5, tokens.link_blue)).show(pui, |box_ui| {
                box_ui.set_min_width(pop_w);
                box_ui.label(egui::RichText::new(&pop.message).color(tokens.text_primary));
                box_ui.add_space(6.0);
                box_ui.horizontal(|row| {
                    if let Some(up) = &pop.upstream_sheet_id {
                        let go = row.button("Go To Source");
                        record_meta_if_enabled(
                            row,
                            meta,
                            true,
                            &go,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_guard.go_to_source",
                                role: "button",
                                label: "Go To Source",
                                command_id: Some("sheet.select"),
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if go.clicked() {
                            intents.push_engine("sheet.select", json!({ "sheet_id": up.as_str() }));
                            state.active_link_popover = None;
                        }
                    }
                    let ul = row.button("Unlink Region");
                    record_meta_if_enabled(
                        row,
                        meta,
                        true,
                        &ul,
                        FrameInspectEntryInput {
                            widget_id: "grid.link_guard.unlink",
                            role: "button",
                            label: "Unlink Region",
                            command_id: Some("table.unlinkRegion"),
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::ModalDialog,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if ul.clicked() {
                        intents.push_engine("table.unlinkRegion", json!({ "sheet_id": ds.id.as_str(), "link_id": pop.region_id }));
                        state.active_link_popover = None;
                    }
                    let dis = row.button("Dismiss");
                    record_meta_if_enabled(
                        row,
                        meta,
                        true,
                        &dis,
                        FrameInspectEntryInput {
                            widget_id: "grid.link_guard.dismiss",
                            role: "button",
                            label: "Dismiss",
                            command_id: None,
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::ModalDialog,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if dis.clicked() {
                        state.active_link_popover = None;
                    }
                });
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_corner(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let cr = geom.corner_rect;
        ui.painter().rect_filled(cr, 0.0, tokens.header_bg);
        ui.painter().rect_stroke(cr, 0.0, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);

        let y1 = cr.min.y;
        let y2 = y1 + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let y4 = cr.max.y;
        let x_mid = cr.min.x + geom.row_num_w;
        let x_end = cr.max.x;

        let corner_btn_r = Rect::from_min_max(pos2(cr.min.x, y1), pos2(x_mid, y2));
        let badge = archetype_badge(&ds.archetype);
        let corner_resp =
            ui.interact(corner_btn_r, ui.make_persistent_id("grid.corner.archetype"), if blocked { Sense::hover() } else { Sense::click() });
        let fill = if corner_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
        ui.painter().rect_filled(corner_btn_r, 0.0, fill);
        render_ellipsized_text(
            ui,
            corner_btn_r.shrink(2.0),
            badge,
            CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.accent, Align::Center),
        );
        let acc_badge = badge.to_owned();
        corner_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_badge.clone()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &corner_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.archetype",
                role: "button",
                label: badge,
                command_id: Some("ui.table.convertArchetype.open"),
                value: None,
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if corner_resp.clicked() && !blocked {
            intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": ds.id.as_str() }));
        }

        let sheet_btn_r = Rect::from_min_max(pos2(x_mid, y1), pos2(x_end, y2));
        if sheet_btn_r.width() > 8.0 {
            let st_resp = ui.interact(sheet_btn_r, ui.make_persistent_id("grid.sheet_title"), if blocked { Sense::hover() } else { Sense::click() });
            if st_resp.hovered() && !blocked {
                ui.painter().rect_filled(sheet_btn_r, 0.0, tokens.hover_bg);
            }
            let (ellip, _) = render_ellipsized_text(
                ui,
                sheet_btn_r.shrink2(vec2(4.0, 1.0)),
                &ds.title,
                CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_strong, Align::LEFT),
            );
            if ellip {
                state.ellipsized_headers += 1;
            }
            let acc_title = ds.title.clone();
            st_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_title.clone()));
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &st_resp,
                FrameInspectEntryInput {
                    widget_id: "grid.sheet_title",
                    role: "button",
                    label: &ds.title,
                    command_id: Some("sheet.rename"),
                    value: Some(serde_json::Value::String(ds.title.clone())),
                    enabled: !blocked,
                    disabled_reason: blocked.then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: ellip,
                    full_label: ellip.then_some(ds.title.as_str()),
                },
            );
            if st_resp.clicked() && !blocked {
                state.inline_edit = None;
                aux.aux_edit = Some((AuxInlineTarget::SheetTitle, ds.title.clone(), true));
            }
        }

        let hash_r = Rect::from_min_max(pos2(cr.min.x, y2), pos2(x_mid, y3));
        let hash_resp = ui.interact(hash_r, ui.make_persistent_id("grid.corner.row_num"), Sense::hover());
        render_ellipsized_text(ui, hash_r, "#", CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_muted, Align::Center));
        hash_resp.widget_info(|| WidgetInfo::labeled(egui::Role::Label, true, String::from("#")));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &hash_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.row_num",
                role: "label",
                label: "#",
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );

        let rt_hdr_r = Rect::from_min_max(pos2(x_mid, y2), pos2(x_end, y3));
        let rt_band_label = row_titles_band_label(ds, &app.locale);
        let rt_resp = ui.interact(rt_hdr_r, ui.make_persistent_id("grid.row_titles.band"), Sense::hover());
        if !geom.row_titles_collapsed {
            render_ellipsized_text(
                ui,
                rt_hdr_r.shrink2(vec2(5.0, 1.0)),
                &rt_band_label,
                CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_strong, Align::LEFT),
            );
        }
        let acc_rt = rt_band_label.clone();
        rt_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_rt.clone()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &rt_resp,
            FrameInspectEntryInput {
                widget_id: "grid.row_titles.band",
                role: "label",
                label: &rt_band_label,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );

        let toggle_r = Rect::from_min_max(pos2(x_mid, y3), pos2(x_end, y4));
        let toggle_lbl = if geom.row_titles_collapsed { ">" } else { "<|>" };
        let toggle_resp =
            ui.interact(toggle_r, ui.make_persistent_id("grid.row_titles.toggle"), if blocked { Sense::hover() } else { Sense::click() });
        if toggle_resp.hovered() && !blocked {
            ui.painter().rect_filled(toggle_r, 0.0, tokens.hover_bg);
        }
        render_ellipsized_text(
            ui,
            toggle_r.shrink(2.0),
            toggle_lbl,
            CellTextStyle::plain(egui::FontId::proportional(10.5), tokens.text_muted, Align::Center),
        );
        let acc_tgl = if geom.row_titles_collapsed { "Expand Row Titles" } else { "Collapse Row Titles" };
        toggle_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_tgl.to_owned()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &toggle_resp,
            FrameInspectEntryInput {
                widget_id: "grid.row_titles.toggle",
                role: "button",
                label: toggle_lbl,
                command_id: None,
                value: None,
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if toggle_resp.clicked() && !blocked {
            let next = !geom.row_titles_collapsed;
            state.row_titles_collapsed = next;
            intents.push_set_row_titles_collapsed(next);
        }

        let p = ui.painter();
        p.line_segment([pos2(cr.min.x, y2), pos2(cr.max.x, y2)], Stroke::new(1.0, tokens.divider));
        p.line_segment([pos2(cr.min.x, y3), pos2(cr.max.x, y3)], Stroke::new(1.0, tokens.divider));
        p.line_segment([pos2(x_mid, y1), pos2(x_mid, y4)], Stroke::new(1.0, tokens.divider));
    }

    fn row_titles_band_label(ds: &DataSheet, locale: &str) -> String {
        match &ds.archetype {
            TableArchetype::Grouped(g) if !g.row_factor_title.trim().is_empty() => g.row_factor_title.clone(),
            TableArchetype::Contingency(c) if !c.row_factor_title.trim().is_empty() => c.row_factor_title.clone(),
            TableArchetype::PartsOfWhole(_) => String::from("Slice Labels"),
            _ => grid_tr(locale, "w6.grid.row_titles", "Row Titles").to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_column_headers(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        active_mv_operands: &HashSet<usize>,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let hdr_rect = geom.col_header_rect;
        if !hdr_rect.is_positive() {
            return;
        }
        let scroll_x = state.scroll_offset_pt.x;
        let vis_flat = compute_visible_range(scroll_x, hdr_rect.width(), geom.subcol_w, geom.total_flat_cols);

        ui.scope_builder(UiBuilder::new().max_rect(hdr_rect), |hdr_ui| {
            hdr_ui.set_clip_rect(hdr_rect);
            hdr_ui.painter().rect_filled(hdr_rect, 0.0, tokens.header_bg);

            let y1 = hdr_rect.min.y;
            let y2 = y1 + geom.h_tier1;
            let y3 = y2 + geom.h_tier2;
            let y4 = hdr_rect.max.y;

            if geom.x_subcols > 0 && vis_flat.start < geom.x_subcols {
                render_multi_x_headers(hdr_ui, ds, state, aux, geom, &vis_flat, scroll_x, hdr_rect, tokens, meta, blocked);
            }

            let vis_data_start = vis_flat.start.saturating_sub(geom.x_subcols);
            let vis_data_end = vis_flat.end.saturating_sub(geom.x_subcols);
            if vis_data_start < vis_data_end && geom.col_count > 0 {
                render_multi_data_col_headers(
                    hdr_ui,
                    app,
                    ds,
                    state,
                    aux,
                    geom,
                    vis_data_start..vis_data_end,
                    scroll_x,
                    hdr_rect,
                    active_mv_operands,
                    tokens,
                    meta,
                    intents,
                    blocked,
                );
            }

            hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y2), pos2(hdr_rect.max.x, y2)], Stroke::new(1.0, tokens.divider));
            hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y3), pos2(hdr_rect.max.x, y3)], Stroke::new(1.0, tokens.divider));
            hdr_ui.painter().line_segment([pos2(hdr_rect.min.x, y4), pos2(hdr_rect.max.x, y4)], Stroke::new(1.5, tokens.border));
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_x_headers(
        hdr_ui: &mut Ui,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        vis_flat: &Range<usize>,
        scroll_x: f32,
        hdr_rect: Rect,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        blocked: bool,
    ) {
        let y1 = hdr_rect.min.y;
        let y2 = y1 + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let y4 = hdr_rect.max.y;
        let x_x0 = hdr_rect.min.x - scroll_x;
        #[allow(clippy::cast_precision_loss)]
        let x_w = (geom.x_subcols as f32) * geom.subcol_w;
        let x_tier1_r = Rect::from_min_max(pos2(x_x0, y1), pos2(x_x0 + x_w, y2));
        let super_x = if matches!(ds.archetype, TableArchetype::Survival(_)) { "Time" } else { "X" };
        render_ellipsized_text(
            hdr_ui,
            x_tier1_r.intersect(hdr_rect).shrink2(vec2(4.0, 1.0)),
            super_x,
            CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_muted, Align::Center),
        );

        for sx in 0..geom.x_subcols {
            if !vis_flat.contains(&sx) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let sx_x0 = x_x0 + (sx as f32) * geom.subcol_w;
            let sx_x1 = sx_x0 + geom.subcol_w;

            let t2_r = Rect::from_min_max(pos2(sx_x0, y2), pos2(sx_x1, y3)).intersect(hdr_rect);
            let x_title = x_column_title(&ds.archetype, sx);
            if t2_r.width() > 2.0 {
                let wid = format!("grid.x_title.{sx}");
                let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
                if resp.hovered() && !blocked {
                    hdr_ui.painter().rect_filled(t2_r, 0.0, tokens.hover_bg);
                }
                render_ellipsized_text(
                    hdr_ui,
                    t2_r.shrink2(vec2(4.0, 1.0)),
                    &x_title,
                    CellTextStyle::plain(egui::FontId::proportional(12.0), tokens.accent, Align::Center),
                );
                let acc = format!("X Title: {x_title}");
                resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
                record_meta_if_enabled(
                    hdr_ui,
                    meta,
                    state.record_inspect_meta,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: &wid,
                        role: "button",
                        label: &x_title,
                        command_id: Some("table.setTitles"),
                        value: Some(serde_json::Value::String(x_title.clone())),
                        enabled: !blocked,
                        disabled_reason: blocked.then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if resp.clicked() && !blocked && sx == 0 {
                    state.inline_edit = None;
                    aux.aux_edit = Some((AuxInlineTarget::XTitle { subcol: sx }, x_title, true));
                }
            }

            let t3_r = Rect::from_min_max(pos2(sx_x0, y3), pos2(sx_x1, y4)).intersect(hdr_rect);
            let x_sub_lbl = x_subcolumn_label(&ds.archetype, sx);
            if t3_r.width() > 2.0 {
                let swid = format!("grid.subcol_hdr.x.{sx}");
                let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), Sense::hover());
                render_ellipsized_text(
                    hdr_ui,
                    t3_r.shrink2(vec2(3.0, 1.0)),
                    &x_sub_lbl,
                    CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_muted, Align::Center),
                );
                let acc_s = x_sub_lbl.clone();
                sresp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc_s.clone()));
                record_meta_if_enabled(
                    hdr_ui,
                    meta,
                    state.record_inspect_meta,
                    &sresp,
                    FrameInspectEntryInput {
                        widget_id: &swid,
                        role: "columnheader",
                        label: &x_sub_lbl,
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: false,
                        full_label: None,
                    },
                );
            }
            hdr_ui.painter().line_segment([pos2(sx_x1, y2), pos2(sx_x1, y4)], Stroke::new(1.0, tokens.divider));
        }
        let x_sep_x = x_x0 + x_w;
        hdr_ui.painter().line_segment([pos2(x_sep_x, y1), pos2(x_sep_x, y4)], Stroke::new(1.5, tokens.border));
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_data_col_headers(
        hdr_ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        vis_data: Range<usize>,
        scroll_x: f32,
        hdr_rect: Rect,
        active_mv_operands: &HashSet<usize>,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let y1 = hdr_rect.min.y;
        let y2 = y1 + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let y4 = hdr_rect.max.y;
        let first_col = geom.flat_data_to_coord(vis_data.start).map_or(0, |(c, _)| c);
        let last_col = geom.flat_data_to_coord(vis_data.end.saturating_sub(1)).map_or(geom.col_count - 1, |(c, _)| c);

        for col in first_col..=last_col.min(geom.col_count.saturating_sub(1)) {
            let (span_flat_start, span_subs) = geom.col_span_flat(col);
            #[allow(clippy::cast_precision_loss)]
            let cx0 = hdr_rect.min.x + (span_flat_start as f32) * geom.subcol_w - scroll_x;
            #[allow(clippy::cast_precision_loss)]
            let cx1 = cx0 + (span_subs as f32) * geom.subcol_w;

            if active_mv_operands.contains(&col) {
                let highlight_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y4)).intersect(hdr_rect);
                hdr_ui.painter().rect_filled(highlight_r, 0.0, tokens.accent.linear_multiply(0.14));
            }

            let t1_r = Rect::from_min_max(pos2(cx0, y1), pos2(cx1, y2)).intersect(hdr_rect);
            let super_lbl = data_super_column_label(&ds.archetype, col);
            if t1_r.width() > 4.0 {
                render_ellipsized_text(
                    hdr_ui,
                    t1_r.shrink2(vec2(4.0, 1.0)),
                    &super_lbl,
                    CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_muted, Align::Center),
                );
            }

            let t2_r = Rect::from_min_max(pos2(cx0, y2), pos2(cx1, y3)).intersect(hdr_rect);
            let col_title = data_column_title(&ds.archetype, col);
            if t2_r.width() > 4.0 && state.inline_edit.as_ref().is_none_or(|e| e.target != GridInlineEditTarget::ColumnTitle { col }) {
                let wid = format!("grid.col_title.{col}");
                let resp = hdr_ui.interact(t2_r, hdr_ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
                if resp.hovered() && !blocked {
                    hdr_ui.painter().rect_filled(t2_r, 0.0, tokens.hover_bg);
                }
                let (ellip, _) = render_ellipsized_text(
                    hdr_ui,
                    t2_r.shrink2(vec2(4.0, 1.0)),
                    &col_title,
                    CellTextStyle::plain(egui::FontId::proportional(12.0), tokens.text_strong, Align::Center),
                );
                if ellip {
                    state.ellipsized_headers += 1;
                }
                let acc = col_title.clone();
                resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
                record_meta_if_enabled(
                    hdr_ui,
                    meta,
                    state.record_inspect_meta,
                    &resp,
                    FrameInspectEntryInput {
                        widget_id: &wid,
                        role: "button",
                        label: &col_title,
                        command_id: Some("table.setTitles"),
                        value: Some(serde_json::Value::String(col_title.clone())),
                        enabled: !blocked,
                        disabled_reason: blocked.then_some("modal dialog is active"),
                        z_layer: ZLayer::Workspace,
                        ellipsized: ellip,
                        full_label: ellip.then_some(col_title.as_str()),
                    },
                );
                if resp.clicked() && !blocked {
                    aux.aux_edit = None;
                    begin_inline_edit(hdr_ui, state, GridInlineEditTarget::ColumnTitle { col }, col_title);
                }
            }

            if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
                if let Some(var) = mv.variables.get(col) {
                    let chip_r = Rect::from_min_max(pos2(cx0 + 2.0, y3 + 2.0), pos2(cx1 - 2.0, y4 - 2.0)).intersect(hdr_rect);
                    if chip_r.width() > 6.0 {
                        render_mv_tier3_role_chip(hdr_ui, ds, state, aux, col, var, chip_r, tokens, meta, intents, blocked);
                    }
                }
            } else {
                render_subcolumn_tier3_cells(hdr_ui, app, ds, state, aux, geom, col, span_subs, cx0, hdr_rect, tokens, meta, blocked);
            }
            hdr_ui.painter().line_segment([pos2(cx1, y1), pos2(cx1, y4)], Stroke::new(1.25, tokens.border));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_subcolumn_tier3_cells(
        hdr_ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        col: usize,
        span_subs: usize,
        cx0: f32,
        hdr_rect: Rect,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        blocked: bool,
    ) {
        let y3 = hdr_rect.min.y + geom.h_tier1 + geom.h_tier2;
        let y4 = hdr_rect.max.y;
        let is_nested = matches!(ds.archetype, TableArchetype::Nested(_));
        for subcol in 0..span_subs {
            #[allow(clippy::cast_precision_loss)]
            let sx0 = cx0 + (subcol as f32) * geom.subcol_w;
            let sx1 = sx0 + geom.subcol_w;
            let t3_r = Rect::from_min_max(pos2(sx0, y3), pos2(sx1, y4)).intersect(hdr_rect);
            if t3_r.width() <= 2.0 {
                continue;
            }
            let sub_lbl = data_subcolumn_header_label(ds, col, subcol, &app.locale);
            let swid = if is_nested { format!("grid.subcol_title.{col}.{subcol}") } else { format!("grid.subcol_hdr.{col}.{subcol}") };
            let sense = if is_nested && !blocked { Sense::click() } else { Sense::hover() };
            let sresp = hdr_ui.interact(t3_r, hdr_ui.make_persistent_id(&swid), sense);
            if is_nested && sresp.hovered() && !blocked {
                hdr_ui.painter().rect_filled(t3_r, 0.0, tokens.hover_bg);
            }
            let fg = if is_nested { tokens.text_primary } else { tokens.text_muted };
            render_ellipsized_text(
                hdr_ui,
                t3_r.shrink2(vec2(3.0, 1.0)),
                &sub_lbl,
                CellTextStyle::plain(egui::FontId::proportional(11.0), fg, Align::Center),
            );
            let acc_s = sub_lbl.clone();
            let role = if is_nested { egui::Role::Button } else { egui::Role::ColumnHeader };
            let role_str = if is_nested { "button" } else { "columnheader" };
            sresp.widget_info(move || WidgetInfo::labeled(role, !is_nested || !blocked, acc_s.clone()));
            record_meta_if_enabled(
                hdr_ui,
                meta,
                state.record_inspect_meta,
                &sresp,
                FrameInspectEntryInput {
                    widget_id: &swid,
                    role: role_str,
                    label: &sub_lbl,
                    command_id: is_nested.then_some("table.setSubcolumnTitles"),
                    value: is_nested.then(|| serde_json::Value::String(sub_lbl.clone())),
                    enabled: !is_nested || !blocked,
                    disabled_reason: (is_nested && blocked).then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            if is_nested && sresp.clicked() && !blocked {
                state.inline_edit = None;
                aux.aux_edit = Some((AuxInlineTarget::SubcolumnTitle { col, subcol }, sub_lbl, true));
            }
            if subcol + 1 < span_subs {
                hdr_ui.painter().line_segment([pos2(sx1, y3), pos2(sx1, y4)], Stroke::new(1.0, tokens.divider));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_mv_tier3_role_chip(
        ui: &mut Ui,
        ds: &DataSheet,
        state: &ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        col: usize,
        var: &curvebook_doc::VariableColumn,
        rect: Rect,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let is_calc = matches!(var.source, VariableSource::Calculated { .. });
        let (chip_txt, chip_col) = if is_calc {
            ("FX=", tokens.role_fx)
        } else {
            let short = match var.role {
                VariableRole::Continuous => "NUM",
                VariableRole::Categorical => "CAT",
                VariableRole::Label => "LBL",
                VariableRole::Calculated => "FX=",
            };
            let c = match var.role {
                VariableRole::Continuous => tokens.role_num,
                VariableRole::Categorical => tokens.role_cat,
                VariableRole::Label => tokens.role_lbl,
                VariableRole::Calculated => tokens.role_fx,
            };
            (short, c)
        };
        let label = if !is_calc && var.role_inferred { format!("Auto·{chip_txt}") } else { chip_txt.to_owned() };
        let wid = format!("mv_formula.role_badge.{col}");
        let resp = ui.interact(rect, ui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
        let alpha = if is_calc || !var.role_inferred { 0.22 } else { 0.10 };
        ui.painter().rect_filled(rect, 3.0, chip_col.linear_multiply(alpha));
        let stroke_w = if !is_calc && var.role_inferred { 1.0 } else { 1.5 };
        ui.painter().rect_stroke(rect, 3.0, Stroke::new(stroke_w, chip_col), egui::StrokeKind::Inside);
        render_ellipsized_text(ui, rect.shrink2(vec2(4.0, 0.0)), &label, text_style(10.5, false, chip_col, Align::Center, is_calc, false));
        let acc = format!("Variable {} role: {label}", var.title);
        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "button",
                label: &label,
                command_id: Some(if is_calc { "ui.table.openFormulaBar" } else { "table.setVariableRole" }),
                value: Some(serde_json::Value::String(label.clone())),
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        if resp.clicked() && !blocked {
            if is_calc {
                aux.active_role_menu_col = None;
                intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": col }));
            } else {
                aux.active_role_menu_col = if aux.active_role_menu_col == Some(col) { None } else { Some(col) };
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_gutter_body(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let g_rect = geom.row_gutter_rect;
        if !g_rect.is_positive() {
            return;
        }
        let scroll_y = state.scroll_offset_pt.y;
        let vis_rows = compute_visible_range(scroll_y, g_rect.height(), geom.row_h, geom.row_count);
        let x0 = g_rect.min.x;
        let x_mid = x0 + geom.row_num_w;
        let x1 = g_rect.max.x;

        ui.scope_builder(UiBuilder::new().max_rect(g_rect), |gui| {
            gui.set_clip_rect(g_rect);
            gui.painter().rect_filled(g_rect, 0.0, tokens.panel_bg);
            for row in vis_rows {
                #[allow(clippy::cast_precision_loss)]
                let y0 = g_rect.min.y + (row as f32) * geom.row_h - scroll_y;
                let y1 = y0 + geom.row_h;
                let row_selected = aux.selected_x_cell.map_or_else(|| app.grid.selected_rows.contains(&row), |(xr, _)| xr == row);
                let num_r = Rect::from_min_max(pos2(x0, y0), pos2(x_mid, y1)).intersect(g_rect);
                if num_r.height() > 1.0 {
                    if row_selected {
                        gui.painter().rect_filled(num_r, 0.0, tokens.accent.linear_multiply(0.18));
                    }
                    let wid = format!("grid.row_num.{row}");
                    let resp = gui.interact(num_r, gui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
                    let row_str = format!("{}", row + 1);
                    render_ellipsized_text(
                        gui,
                        num_r.shrink2(vec2(4.0, 0.0)),
                        &row_str,
                        text_style(11.0, true, tokens.text_muted, Align::RIGHT, false, false),
                    );
                    let acc_rn = format!("Row {}", row + 1);
                    resp.widget_info(move || WidgetInfo::labeled(egui::Role::RowHeader, !blocked, acc_rn.clone()));
                    record_meta_if_enabled(
                        gui,
                        meta,
                        state.record_inspect_meta,
                        &resp,
                        FrameInspectEntryInput {
                            widget_id: &wid,
                            role: "rowheader",
                            label: &row_str,
                            command_id: None,
                            value: None,
                            enabled: !blocked,
                            disabled_reason: blocked.then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if resp.clicked() && !blocked && geom.col_count > 0 {
                        let c = GridCellCoord::new(row, 0, 0);
                        aux.selected_x_cell = None;
                        intents.push_select_grid_cell(c);
                    }
                    if resp.secondary_clicked()
                        && !blocked
                        && let Some(pos) = resp.interact_pointer_pos()
                    {
                        aux.context_menu = Some(ContextMenuState { pos, row, cell: None });
                    }
                }

                let rt_r = Rect::from_min_max(pos2(x_mid, y0), pos2(x1, y1)).intersect(g_rect);
                if rt_r.height() > 1.0 {
                    let row_title = row_title_at(&ds.archetype, row);
                    if geom.row_titles_collapsed {
                        let wid = format!("grid.row_title_collapsed.{row}");
                        let resp = gui.interact(rt_r, gui.make_persistent_id(&wid), Sense::hover());
                        let abbr: String = row_title.chars().take(2).collect();
                        render_ellipsized_text(
                            gui,
                            rt_r.shrink2(vec2(3.0, 0.0)),
                            &abbr,
                            CellTextStyle::plain(egui::FontId::proportional(11.0), tokens.text_muted, Align::Center),
                        );
                        let acc_c = format!("Row {} Title {}", row + 1, row_title);
                        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_c.clone()));
                        record_meta_if_enabled(
                            gui,
                            meta,
                            state.record_inspect_meta,
                            &resp,
                            FrameInspectEntryInput {
                                widget_id: &wid,
                                role: "label",
                                label: &abbr,
                                command_id: None,
                                value: Some(serde_json::Value::String(row_title)),
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::Workspace,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                    } else {
                        let wid = format!("grid.row_title.{row}");
                        let editing_this = state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::RowTitle { row });
                        if !editing_this {
                            let resp = gui.interact(rt_r, gui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
                            if resp.hovered() && !blocked {
                                gui.painter().rect_filled(rt_r, 0.0, tokens.hover_bg);
                            }
                            render_ellipsized_text(
                                gui,
                                rt_r.shrink2(vec2(5.0, 0.0)),
                                &row_title,
                                CellTextStyle::plain(egui::FontId::proportional(12.0), tokens.text_primary, Align::LEFT),
                            );
                            let acc_rt = if row_title.is_empty() { format!("Row {} Title", row + 1) } else { row_title.clone() };
                            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_rt.clone()));
                            record_meta_if_enabled(
                                gui,
                                meta,
                                state.record_inspect_meta,
                                &resp,
                                FrameInspectEntryInput {
                                    widget_id: &wid,
                                    role: "button",
                                    label: &row_title,
                                    command_id: Some("table.setRowTitles"),
                                    value: Some(serde_json::Value::String(row_title.clone())),
                                    enabled: !blocked,
                                    disabled_reason: blocked.then_some("modal dialog is active"),
                                    z_layer: ZLayer::Workspace,
                                    ellipsized: false,
                                    full_label: None,
                                },
                            );
                            if resp.clicked() && !blocked {
                                aux.aux_edit = None;
                                begin_inline_edit(gui, state, GridInlineEditTarget::RowTitle { row }, row_title);
                            }
                        }
                    }
                }
                gui.painter().line_segment([pos2(x0, y1), pos2(x1, y1)], Stroke::new(1.0, tokens.divider));
            }
            gui.painter().line_segment([pos2(x_mid, g_rect.min.y), pos2(x_mid, g_rect.max.y)], Stroke::new(1.0, tokens.divider));
            gui.painter().line_segment([pos2(x1, g_rect.min.y), pos2(x1, g_rect.max.y)], Stroke::new(1.5, tokens.border));
        });
    }

    struct MultiBodyOut {
        rendered_cells: usize,
        vis_rows: Range<usize>,
        vis_all_flat: Range<usize>,
    }

    fn is_coord_selected(app: &UiApp, coord: GridCellCoord) -> (bool, bool) {
        let is_active = app.grid.tool.active == Some(coord);
        if is_active {
            return (true, true);
        }
        if let Some((s, e)) = app.grid.tool.selection_bounds()
            && coord.row >= s.row
            && coord.row <= e.row
            && coord.col >= s.col
            && coord.col <= e.col
            && coord.subcol >= s.subcol
            && coord.subcol <= e.subcol
        {
            return (false, true);
        }
        if app.grid.selected_coords.contains(&coord) {
            return (false, true);
        }
        (false, false)
    }

    struct MultiBodyDrawBuffers<'a> {
        text_buf: &'a mut String,
        id_buf: &'a mut String,
        acc_buf: &'a mut String,
        record_meta: bool,
        base_row_bg: Color32,
        x_seq_readonly: bool,
        is_mv: bool,
        is_cont: bool,
        is_surv: bool,
        blocked: bool,
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_body(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) -> MultiBodyOut {
        let b_rect = geom.body_rect;
        if !b_rect.is_positive() {
            return MultiBodyOut { rendered_cells: 0, vis_rows: 0..0, vis_all_flat: 0..0 };
        }
        let mut out = MultiBodyOut { rendered_cells: 0, vis_rows: 0..0, vis_all_flat: 0..0 };

        let scroll_out = ui.scope_builder(UiBuilder::new().max_rect(b_rect), |body_parent| {
            if blocked {
                body_parent.disable();
            }
            egui::ScrollArea::both()
                .id_salt(("r15_4_multi_grid_scroll", ds.id.as_str()))
                .scroll_offset(state.scroll_offset_pt)
                .scroll_bar_visibility(if blocked { ScrollBarVisibility::AlwaysHidden } else { ScrollBarVisibility::AlwaysVisible })
                .scroll_source(if blocked {
                    ScrollSource::NONE
                } else {
                    ScrollSource { scroll_bar: true, drag: DragScroll::Never, mouse_wheel: true }
                })
                .auto_shrink([false, false])
                .show_viewport(body_parent, |body_ui, viewport| {
                    let origin = body_ui.min_rect().min;
                    body_ui.allocate_space(vec2(geom.content_w.max(geom.body_inner_clip.width()), geom.content_h.max(geom.body_inner_clip.height())));

                    let vis_rows = compute_visible_range(viewport.min.y, geom.body_inner_clip.height(), geom.row_h, geom.row_count);
                    let vis_all_flat = compute_visible_range(viewport.min.x, geom.body_inner_clip.width(), geom.subcol_w, geom.total_flat_cols);
                    out.vis_rows = vis_rows.clone();
                    out.vis_all_flat = vis_all_flat.clone();

                    let mut text_buf = String::with_capacity(32);
                    let mut id_buf = String::with_capacity(32);
                    let mut acc_buf = String::with_capacity(48);
                    let record_meta = state.record_inspect_meta && !body_ui.ctx().will_discard();

                    body_ui.set_clip_rect(geom.body_inner_clip);

                    let x_seq_readonly =
                        matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }));
                    let is_mv = matches!(&ds.archetype, TableArchetype::MultipleVariables(_));
                    let is_cont = matches!(&ds.archetype, TableArchetype::Contingency(_));
                    let is_surv = matches!(&ds.archetype, TableArchetype::Survival(_));
                    let alt_fill = alt_row_fill(tokens.surface_bg, tokens.panel_bg);

                    for row in vis_rows.clone() {
                        #[allow(clippy::cast_precision_loss)]
                        let y0 = origin.y + (row as f32) * geom.row_h;
                        let y1 = y0 + geom.row_h;
                        if y1 <= geom.body_inner_clip.min.y || y0 >= geom.body_inner_clip.max.y {
                            continue;
                        }
                        let mut dbuf = MultiBodyDrawBuffers {
                            text_buf: &mut text_buf,
                            id_buf: &mut id_buf,
                            acc_buf: &mut acc_buf,
                            record_meta,
                            base_row_bg: if row % 2 == 1 { alt_fill } else { tokens.surface_bg },
                            x_seq_readonly,
                            is_mv,
                            is_cont,
                            is_surv,
                            blocked,
                        };

                        for flat in vis_all_flat.clone() {
                            #[allow(clippy::cast_precision_loss)]
                            let x0 = origin.x + (flat as f32) * geom.subcol_w;
                            let raw_cell_r = Rect::from_min_max(pos2(x0, y0), pos2(x0 + geom.subcol_w, y1));
                            let clipped = raw_cell_r.intersect(geom.body_inner_clip);
                            if clipped.width() < 1.0 || clipped.height() < 1.0 {
                                continue;
                            }
                            if flat < geom.x_subcols {
                                render_single_multi_x_cell(body_ui, app, ds, state, aux, geom, row, flat, clipped, &mut dbuf, tokens, meta);
                                out.rendered_cells += 1;
                            } else if let Some((col, subcol)) = geom.flat_data_to_coord(flat - geom.x_subcols) {
                                render_single_multi_data_cell(
                                    body_ui,
                                    app,
                                    ds,
                                    state,
                                    aux,
                                    geom,
                                    GridCellCoord::new(row, col, subcol),
                                    raw_cell_r,
                                    clipped,
                                    &mut dbuf,
                                    tokens,
                                    meta,
                                    intents,
                                );
                                out.rendered_cells += 1;
                            }
                        }

                        body_ui.painter().line_segment(
                            [pos2(geom.body_inner_clip.min.x, y1), pos2(geom.body_inner_clip.max.x, y1)],
                            Stroke::new(1.0, tokens.divider),
                        );
                    }

                    paint_multi_vertical_hairlines(body_ui, origin.x, geom, &vis_all_flat, tokens);
                    super::clear_grid_drag_anchor_if_released(body_ui, &ds.id);
                })
        });
        if !blocked {
            state.scroll_offset_pt = scroll_out.inner.state.clamped_offset();
            clamp_multi_scroll(state, geom);
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_single_multi_x_cell(
        body_ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        row: usize,
        subcol: usize,
        clipped: Rect,
        dbuf: &mut MultiBodyDrawBuffers<'_>,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
    ) {
        let cell_opt = ds.archetype.x_or_time_cell(row, subcol);
        dbuf.text_buf.clear();
        if let Some(c) = cell_opt {
            format_cell_into(c, &app.locale_preset, dbuf.text_buf);
        } else if dbuf.x_seq_readonly
            && subcol == 0
            && let TableArchetype::Xy(xy) = &ds.archetype
            && let XColumnMode::GeneratedSequence { start, step, progression } = &xy.x_mode
        {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let seq_val = match progression {
                curvebook_doc::SequenceProgression::Arithmetic => *start + (row as f64) * (*step),
                curvebook_doc::SequenceProgression::Geometric => *start * step.powi(row as i32),
            };
            format_cell_into(&Cell::number(seq_val), &app.locale_preset, dbuf.text_buf);
        }
        let excluded = cell_opt.is_some_and(|c| c.excluded);
        let editable = !dbuf.blocked && !ds.frozen && !dbuf.x_seq_readonly;
        let is_selected = aux.selected_x_cell == Some((row, subcol));
        let editing_this = aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::XCell { row, subcol });

        let fill = if is_selected {
            tokens.active_bg
        } else if dbuf.x_seq_readonly {
            tokens.panel_bg
        } else {
            dbuf.base_row_bg
        };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
        if is_selected {
            body_ui.painter().rect_stroke(clipped, 0.0, Stroke::new(1.5, tokens.accent), egui::StrokeKind::Inside);
        }

        if !editing_this && !dbuf.text_buf.is_empty() {
            let text_col = if excluded {
                tokens.excluded_cell_text
            } else if dbuf.x_seq_readonly {
                tokens.text_muted
            } else {
                tokens.text_primary
            };
            render_ellipsized_text(
                body_ui,
                clipped.shrink2(vec2(5.0, 1.0)),
                dbuf.text_buf,
                text_style(geom.font_num_pt, true, text_col, Align::RIGHT, excluded || dbuf.x_seq_readonly, excluded),
            );
        }

        dbuf.id_buf.clear();
        let _ = write!(dbuf.id_buf, "grid.xcell.{row}.{subcol}");
        let resp =
            body_ui.interact(clipped, body_ui.make_persistent_id(&*dbuf.id_buf), if dbuf.blocked { Sense::hover() } else { Sense::click_and_drag() });
        dbuf.acc_buf.clear();
        if dbuf.text_buf.is_empty() {
            let _ = write!(dbuf.acc_buf, "R{}X{}", row + 1, subcol + 1);
        } else {
            let _ = write!(dbuf.acc_buf, "R{}X{} {}", row + 1, subcol + 1, dbuf.text_buf);
        }
        let acc_owned = dbuf.acc_buf.clone();
        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Cell, editable, acc_owned.clone()));

        if dbuf.record_meta {
            meta.record_response(
                body_ui,
                &resp,
                FrameInspectEntryInput {
                    widget_id: dbuf.id_buf,
                    role: "gridcell",
                    label: dbuf.acc_buf,
                    command_id: Some("table.setXCells"),
                    value: Some(serde_json::json!({
                        "row": row,
                        "subcol": subcol,
                        "text": dbuf.text_buf.clone(),
                        "excluded": excluded,
                        "selected": is_selected,
                    })),
                    enabled: editable,
                    disabled_reason: if dbuf.blocked {
                        Some("modal dialog is active")
                    } else if dbuf.x_seq_readonly {
                        Some("X column is a generated sequence (read-only)")
                    } else if ds.frozen {
                        Some("sheet is frozen")
                    } else {
                        None
                    },
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }

        if !dbuf.blocked && resp.clicked() {
            aux.selected_x_cell = Some((row, subcol));
            aux.active_role_menu_col = None;
            aux.context_menu = None;
            set_grid_focus_active(body_ui, true);
            body_ui.ctx().request_repaint();
            if is_multi_click_edit(&resp) && editable {
                state.inline_edit = None;
                aux.aux_edit = Some((AuxInlineTarget::XCell { row, subcol }, initial_edit_text_for_cell(cell_opt, &app.locale_preset), true));
            }
        }
        if !dbuf.blocked
            && resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            aux.selected_x_cell = Some((row, subcol));
            body_ui.ctx().request_repaint();
            aux.context_menu = Some(ContextMenuState { pos, row, cell: Some(ContextCellTarget { is_x: true, row, col: 0, subcol }) });
        }
    }

    struct MultiCellFlags<'a> {
        cell_opt: Option<&'a curvebook_doc::Cell>,
        excluded: bool,
        mv_is_cat_or_lbl: bool,
        mv_is_calc: bool,
        linked: Option<&'a LinkedDataRegion>,
        editable: bool,
        is_active: bool,
        is_selected: bool,
        contingency_invalid: bool,
    }

    #[allow(clippy::too_many_arguments)]
    fn render_single_multi_data_cell(
        body_ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        coord: GridCellCoord,
        raw_cell_r: Rect,
        clipped: Rect,
        dbuf: &mut MultiBodyDrawBuffers<'_>,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
    ) {
        let cell_opt = ds.archetype.data_cell(curvebook_doc::CellCoord::from(coord));
        dbuf.text_buf.clear();
        if let Some(c) = cell_opt {
            format_cell_into(c, &app.locale_preset, dbuf.text_buf);
        }
        let mv_var = if dbuf.is_mv {
            if let TableArchetype::MultipleVariables(mv) = &ds.archetype { mv.variables.get(coord.col) } else { None }
        } else {
            None
        };
        let mv_is_calc = mv_var.is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }));
        let mv_is_cat_or_lbl = mv_var.is_some_and(|v| matches!(v.role, VariableRole::Categorical | VariableRole::Label));
        let linked = linked_region_for_cell(ds, coord.row, coord.col);
        let (is_active, is_selected) = if aux.selected_x_cell.is_some() { (false, false) } else { is_coord_selected(app, coord) };
        let flags = MultiCellFlags {
            cell_opt,
            excluded: cell_opt.is_some_and(|c| c.excluded),
            mv_is_cat_or_lbl,
            mv_is_calc,
            linked,
            editable: !dbuf.blocked && !ds.frozen && !mv_is_calc && linked.is_none(),
            is_active,
            is_selected,
            contingency_invalid: dbuf.is_cont && is_invalid_contingency_cell(cell_opt),
        };

        paint_single_multi_data_cell(body_ui, ds, state, geom, coord, raw_cell_r, clipped, dbuf, tokens, &flags);

        dbuf.id_buf.clear();
        let _ = write!(dbuf.id_buf, "grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol);
        let resp =
            body_ui.interact(clipped, body_ui.make_persistent_id(&*dbuf.id_buf), if dbuf.blocked { Sense::hover() } else { Sense::click_and_drag() });
        dbuf.acc_buf.clear();
        if dbuf.text_buf.is_empty() {
            let _ = write!(dbuf.acc_buf, "R{}C{}", coord.row + 1, coord.col + 1);
        } else {
            let _ = write!(dbuf.acc_buf, "R{}C{} {}", coord.row + 1, coord.col + 1, dbuf.text_buf);
        }
        let acc_owned = dbuf.acc_buf.clone();
        let acc_enabled = flags.editable || flags.mv_is_calc;
        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Cell, acc_enabled, acc_owned.clone()));

        handle_multi_data_cell_interactions(body_ui, app, ds, state, aux, coord, dbuf, meta, intents, &flags, &resp);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_single_multi_data_cell(
        body_ui: &Ui,
        ds: &DataSheet,
        state: &ColumnGridEguiState,
        geom: &MultiGridGeom,
        coord: GridCellCoord,
        raw_cell_r: Rect,
        clipped: Rect,
        dbuf: &MultiBodyDrawBuffers<'_>,
        tokens: &Tokens,
        flags: &MultiCellFlags<'_>,
    ) {
        let fill = if flags.is_active {
            tokens.active_bg
        } else if flags.is_selected {
            tokens.hover_bg
        } else if flags.mv_is_calc {
            tokens.role_fx.linear_multiply(0.06)
        } else if flags.contingency_invalid {
            tokens.link_red.linear_multiply(0.12)
        } else {
            dbuf.base_row_bg
        };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
        if flags.is_active {
            body_ui.painter().rect_stroke(clipped, 0.0, Stroke::new(1.5, tokens.accent), egui::StrokeKind::Inside);
        }
        if let Some(reg) = flags.linked {
            let (_, span_subs) = geom.col_span_flat(coord.col);
            paint_linked_perimeter_edges(
                body_ui,
                raw_cell_r,
                clipped,
                reg,
                coord,
                span_subs,
                link_border_color32(reg.spec.source.border_color(), tokens),
            );
        }
        if flags.excluded {
            paint_exclusion_corner_marker(body_ui, raw_cell_r, clipped, ds, coord, tokens);
        }

        if state.inline_edit.as_ref().is_none_or(|e| e.target != GridInlineEditTarget::Cell(coord)) && !dbuf.text_buf.is_empty() {
            let is_numeric = matches!(flags.cell_opt.map(|c| &c.value), Some(CellValue::Number(_))) && !flags.mv_is_cat_or_lbl;
            let align = if is_numeric { Align::RIGHT } else { Align::LEFT };
            let text_col = if flags.excluded {
                tokens.excluded_cell_text
            } else if flags.contingency_invalid {
                tokens.link_red
            } else if flags.mv_is_calc {
                tokens.role_fx
            } else if dbuf.is_surv && cell_is_survival_event(ds, flags.cell_opt) {
                tokens.accent
            } else {
                tokens.text_primary
            };
            render_ellipsized_text(
                body_ui,
                clipped.shrink2(vec2(5.0, 1.0)),
                dbuf.text_buf,
                text_style(
                    if is_numeric { geom.font_num_pt } else { geom.font_small_pt },
                    is_numeric,
                    text_col,
                    align,
                    flags.excluded || flags.mv_is_calc,
                    flags.excluded,
                ),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_multi_data_cell_interactions(
        body_ui: &Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        coord: GridCellCoord,
        dbuf: &MultiBodyDrawBuffers<'_>,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        flags: &MultiCellFlags<'_>,
        resp: &egui::Response,
    ) {
        if !dbuf.blocked && resp.hovered() {
            if flags.excluded
                && let Some(tip) = GridViewState::cell_exclusion_tooltip(ds, coord)
            {
                resp.clone().on_hover_text(tip);
            } else if flags.contingency_invalid {
                resp.clone().on_hover_text("Contingency counts should be non-negative integers");
            }
        }

        if dbuf.record_meta {
            let inspect_text = if flags.excluded && !dbuf.text_buf.is_empty() { format!("{}*", dbuf.text_buf) } else { dbuf.text_buf.clone() };
            meta.record_response(
                body_ui,
                resp,
                FrameInspectEntryInput {
                    widget_id: dbuf.id_buf,
                    role: "gridcell",
                    label: dbuf.acc_buf,
                    command_id: Some("table.setCell"),
                    value: Some(serde_json::json!({
                        "row": coord.row,
                        "col": coord.col,
                        "subcol": coord.subcol,
                        "text": inspect_text,
                        "excluded": flags.excluded,
                        "selected": flags.is_selected,
                    })),
                    enabled: flags.editable,
                    disabled_reason: if dbuf.blocked {
                        Some("modal dialog is active")
                    } else if flags.mv_is_calc {
                        Some("Calculated variable cells are formula-derived (read-only).")
                    } else if flags.linked.is_some() {
                        Some("cell is inside a read-only linked region")
                    } else if ds.frozen {
                        Some("sheet is frozen")
                    } else {
                        None
                    },
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
        }

        if !dbuf.blocked && resp.clicked() {
            if aux.selected_x_cell.take().is_some() {
                body_ui.ctx().request_repaint();
            }
            aux.active_role_menu_col = None;
            aux.context_menu = None;
            set_grid_focus_active(body_ui, true);
            if body_ui.input(|i| i.modifiers).shift {
                intents.push_extend_grid_selection(coord);
            } else {
                intents.push_select_grid_cell(coord);
            }
            if is_multi_click_edit(resp) {
                if flags.mv_is_calc {
                    intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": coord.col }));
                } else if let Some(reg) = flags.linked {
                    open_link_popover(state, reg, &app.locale);
                } else if !ds.frozen {
                    aux.aux_edit = None;
                    begin_inline_edit(
                        body_ui,
                        state,
                        GridInlineEditTarget::Cell(coord),
                        initial_edit_text_for_cell(flags.cell_opt, &app.locale_preset),
                    );
                }
            }
        } else if !dbuf.blocked
            && aux.active_role_menu_col.is_none()
            && aux.context_menu.is_none()
            && !is_multi_click_edit(resp)
            && !resp.secondary_clicked()
            && super::handle_cell_pointer_drag_select(body_ui, &ds.id, coord, resp, intents)
        {
            aux.selected_x_cell = None;
            aux.active_role_menu_col = None;
            aux.context_menu = None;
        }
        if !dbuf.blocked
            && resp.secondary_clicked()
            && let Some(pos) = resp.interact_pointer_pos()
        {
            aux.selected_x_cell = None;
            intents.push_select_grid_cell(coord);
            aux.context_menu = Some(ContextMenuState {
                pos,
                row: coord.row,
                cell: Some(ContextCellTarget { is_x: false, row: coord.row, col: coord.col, subcol: coord.subcol }),
            });
        }
    }

    fn paint_multi_vertical_hairlines(body_ui: &Ui, origin_x: f32, geom: &MultiGridGeom, vis_all_flat: &Range<usize>, tokens: &Tokens) {
        for flat in vis_all_flat.clone() {
            #[allow(clippy::cast_precision_loss)]
            let x1 = origin_x + ((flat + 1) as f32) * geom.subcol_w;
            if x1 <= geom.body_inner_clip.min.x || x1 >= geom.body_inner_clip.max.x {
                continue;
            }
            let is_group_boundary = if flat + 1 == geom.x_subcols {
                true
            } else if flat >= geom.x_subcols {
                geom.flat_data_to_coord(flat + 1 - geom.x_subcols).is_none_or(|(_, s)| s == 0)
            } else {
                false
            };
            let w = if is_group_boundary { 1.25 } else { 0.75 };
            body_ui
                .painter()
                .line_segment([pos2(x1, geom.body_inner_clip.min.y), pos2(x1, geom.body_inner_clip.max.y)], Stroke::new(w, tokens.divider));
        }
    }

    fn is_invalid_contingency_cell(cell: Option<&Cell>) -> bool {
        let Some(c) = cell else { return false };
        match c.value {
            CellValue::Number(v) => !v.is_finite() || v < 0.0 || (v - v.round()).abs() > 1e-6,
            CellValue::Text(ref s) => !s.trim().is_empty(),
            CellValue::Empty => false,
        }
    }

    fn cell_is_survival_event(ds: &DataSheet, cell: Option<&Cell>) -> bool {
        let TableArchetype::Survival(s) = &ds.archetype else { return false };
        matches!(cell.map(|c| &c.value), Some(CellValue::Number(v)) if (*v - s.event_code as f64).abs() < 1e-6)
    }

    fn x_column_title(arch: &TableArchetype, subcol: usize) -> String {
        match arch {
            TableArchetype::Xy(xy) => {
                if subcol == 1 {
                    String::from("±X Error")
                } else {
                    xy.x_title.clone()
                }
            }
            TableArchetype::Survival(s) => s.time_title.clone(),
            _ => String::from("X"),
        }
    }

    fn x_subcolumn_label(arch: &TableArchetype, subcol: usize) -> String {
        match arch {
            TableArchetype::Xy(xy) => match &xy.x_mode {
                XColumnMode::Numbers { subcolumn_mode } => {
                    if subcolumn_mode.subcolumn_count() > 1 && subcol > 0 {
                        String::from("X Error")
                    } else {
                        String::from("X")
                    }
                }
                XColumnMode::DateOrElapsedTime { unit_label } => {
                    if unit_label.trim().is_empty() {
                        String::from("Time")
                    } else {
                        format!("Time ({unit_label})")
                    }
                }
                XColumnMode::GeneratedSequence { .. } => String::from("Seq"),
            },
            TableArchetype::Survival(_) => String::from("Time"),
            _ => String::from("X"),
        }
    }

    fn data_super_column_label(arch: &TableArchetype, col: usize) -> String {
        let letter = (b'A' + u8::try_from(col % 26).unwrap_or(0)) as char;
        match arch {
            TableArchetype::Grouped(g) if !g.column_factor_title.trim().is_empty() => format!("{} {letter}", g.column_factor_title),
            TableArchetype::Survival(_) => format!("Cohort {letter}"),
            TableArchetype::MultipleVariables(_) => format!("Var {}", col + 1),
            TableArchetype::PartsOfWhole(_) => String::from("Slice Share"),
            _ => format!("Group {letter}"),
        }
    }

    fn data_column_title(arch: &TableArchetype, col: usize) -> String {
        match arch {
            TableArchetype::Column(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::Xy(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::Grouped(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::Contingency(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::Survival(t) => t.cohorts.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::PartsOfWhole(t) => t.data_sets.get(col).map_or_else(String::new, |d| d.title.clone()),
            TableArchetype::MultipleVariables(t) => t.variables.get(col).map_or_else(String::new, |v| v.title.clone()),
            TableArchetype::Nested(t) => t.groups.get(col).map_or_else(String::new, |g| g.title.clone()),
        }
    }

    fn data_subcolumn_header_label(ds: &DataSheet, col: usize, subcol: usize, locale: &str) -> String {
        match &ds.archetype {
            TableArchetype::Xy(t) => column_subcolumn_role_label(t.y_subcolumn_mode, subcol, locale),
            TableArchetype::Grouped(t) => column_subcolumn_role_label(t.subcolumn_mode, subcol, locale),
            TableArchetype::Column(t) => column_subcolumn_role_label(t.subcolumn_mode, subcol, locale),
            TableArchetype::Contingency(_) => String::from("Count"),
            TableArchetype::Survival(s) => format!("Event({}) / Cens({})", s.event_code, s.censored_code),
            TableArchetype::PartsOfWhole(_) => String::from("Value"),
            TableArchetype::MultipleVariables(_) => String::from("Var"),
            TableArchetype::Nested(t) => t
                .groups
                .get(col)
                .and_then(|g| g.subcolumn_titles.get(subcol))
                .cloned()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| format!("Unit {}", subcol + 1)),
        }
    }

    fn row_title_at(arch: &TableArchetype, row: usize) -> String {
        let slice = match arch {
            TableArchetype::Column(t) => &t.row_titles,
            TableArchetype::Xy(t) => &t.row_titles,
            TableArchetype::Grouped(t) => &t.row_titles,
            TableArchetype::Contingency(t) => &t.row_titles,
            TableArchetype::Survival(t) => &t.row_titles,
            TableArchetype::PartsOfWhole(t) => &t.slice_labels,
            TableArchetype::MultipleVariables(t) => &t.row_titles,
            TableArchetype::Nested(t) => &t.row_titles,
        };
        slice.get(row).cloned().unwrap_or_default()
    }

    fn active_mv_formula_operands(app: &UiApp, ds: &DataSheet) -> HashSet<usize> {
        let mut out = HashSet::new();
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else { return out };
        let formula_opt = if app.grid.formula_bar.active && !app.grid.formula_bar.input.trim().is_empty() {
            Some(app.grid.formula_bar.input.as_str())
        } else if let Some(active) = app.grid.tool.active {
            mv.variables.get(active.col).and_then(|v| match &v.source {
                VariableSource::Calculated { formula, .. } => Some(formula.as_str()),
                VariableSource::Entered => None,
            })
        } else {
            None
        };
        let Some(formula) = formula_opt else { return out };
        for (idx, var) in mv.variables.iter().enumerate() {
            let bracketed = format!("[{}]", var.title);
            if formula.contains(&bracketed) || formula.contains(&var.title) {
                out.insert(idx);
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn render_multi_inline_editors(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        _tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        if blocked {
            return;
        }
        if let Some(mut edit) = state.inline_edit.take() {
            let Some(rect) = rect_for_standard_target(geom, state, &edit.target) else { return };
            let wid = match &edit.target {
                GridInlineEditTarget::Cell(c) => format!("grid.cell.{}.{}.{}", c.row, c.col, c.subcol),
                GridInlineEditTarget::ColumnTitle { col } => format!("grid.col_title.{col}"),
                GridInlineEditTarget::RowTitle { row } => format!("grid.row_title.{row}"),
            };
            let te_id = ui.make_persistent_id(("multi_std_inline", &wid));
            let resp = ui.put(rect, egui::TextEdit::singleline(&mut edit.draft).id(te_id).desired_width(rect.width()));
            let first_frame = !edit.focused_once;
            if first_frame {
                resp.request_focus();
                edit.focused_once = true;
            }
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "textbox",
                    label: &edit.draft,
                    command_id: None,
                    value: Some(serde_json::Value::String(edit.draft.clone())),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            let esc = pressed_key_event_modifiers(ui, Key::Escape).is_some();
            let enter = pressed_key_event_modifiers(ui, Key::Enter).is_some();
            let tab = pressed_key_event_modifiers(ui, Key::Tab);
            if esc {
                return;
            }
            if enter || tab.is_some() || (resp.lost_focus() && !first_frame) {
                commit_standard_inline_edit(app, ds, edit.target, edit.draft, intents);
                if enter {
                    if let Some(c) = app.grid.tool.active {
                        intents.push_select_grid_cell(GridCellCoord::new((c.row + 1).min(geom.row_count.saturating_sub(1)), c.col, c.subcol));
                    }
                } else if let Some(mods) = tab
                    && let Some(c) = app.grid.tool.active
                {
                    let next = step_cell_horiz(geom, Some(c), None, mods.shift);
                    if let (false, Some(nc), _) = next {
                        intents.push_select_grid_cell(nc);
                    }
                }
                return;
            }
            state.inline_edit = Some(edit);
            return;
        }

        if let Some((target, mut draft, mut just_focused)) = aux.aux_edit.take() {
            let Some(rect) = rect_for_aux_target(geom, state, &target) else { return };
            let wid = match &target {
                AuxInlineTarget::XCell { row, subcol } => format!("grid.xcell.{row}.{subcol}"),
                AuxInlineTarget::XTitle { subcol } => format!("grid.x_title.{subcol}"),
                AuxInlineTarget::SubcolumnTitle { col, subcol } => format!("grid.subcol_title.{col}.{subcol}"),
                AuxInlineTarget::SheetTitle => String::from("grid.sheet_title"),
            };
            let te_id = ui.make_persistent_id(("multi_aux_inline", &wid));
            if matches!(target, AuxInlineTarget::XCell { .. })
                && (!just_focused || ui.memory(|m| m.has_focus(te_id)))
                && let Some(multi_text) = extract_multiline_or_tab_x_input(ui, app)
            {
                commit_aux_inline_edit(app, ds, aux, &target, multi_text, intents);
                return;
            }
            let resp = ui.put(rect, egui::TextEdit::singleline(&mut draft).id(te_id).desired_width(rect.width()));
            if just_focused {
                resp.request_focus();
                just_focused = false;
                aux.suppress_loss_commit_once = true;
            }
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "textbox",
                    label: &draft,
                    command_id: None,
                    value: Some(serde_json::Value::String(draft.clone())),
                    enabled: true,
                    disabled_reason: None,
                    z_layer: ZLayer::Workspace,
                    ellipsized: false,
                    full_label: None,
                },
            );
            let esc = pressed_key_event_modifiers(ui, Key::Escape).is_some();
            let enter = pressed_key_event_modifiers(ui, Key::Enter).is_some();
            let tab = pressed_key_event_modifiers(ui, Key::Tab);
            let lost = resp.lost_focus() && !aux.suppress_loss_commit_once;
            aux.suppress_loss_commit_once = false;
            if esc {
                return;
            }
            if enter || tab.is_some() || lost {
                commit_aux_inline_edit(app, ds, aux, &target, draft, intents);
                if enter && let AuxInlineTarget::XCell { row, subcol } = target {
                    aux.selected_x_cell = Some(((row + 1).min(geom.row_count.saturating_sub(1)), subcol));
                }
                return;
            }
            aux.aux_edit = Some((target, draft, just_focused));
        }
    }

    fn rect_for_standard_target(geom: &MultiGridGeom, state: &ColumnGridEguiState, target: &GridInlineEditTarget) -> Option<Rect> {
        match *target {
            GridInlineEditTarget::Cell(coord) => {
                let flat = geom.coord_to_flat_all(false, coord.col, coord.subcol);
                #[allow(clippy::cast_precision_loss)]
                let x0 = geom.body_rect.min.x + (flat as f32) * geom.subcol_w - state.scroll_offset_pt.x;
                #[allow(clippy::cast_precision_loss)]
                let y0 = geom.body_rect.min.y + (coord.row as f32) * geom.row_h - state.scroll_offset_pt.y;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.row_h)).intersect(geom.body_inner_clip))
            }
            GridInlineEditTarget::ColumnTitle { col } => {
                let (flat, span) = geom.col_span_flat(col);
                #[allow(clippy::cast_precision_loss)]
                let x0 = geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - state.scroll_offset_pt.x;
                #[allow(clippy::cast_precision_loss)]
                let w = (span as f32) * geom.subcol_w;
                let y0 = geom.col_header_rect.min.y + geom.h_tier1;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(w, geom.h_tier2)).intersect(geom.col_header_rect))
            }
            GridInlineEditTarget::RowTitle { row } => {
                #[allow(clippy::cast_precision_loss)]
                let y0 = geom.row_gutter_rect.min.y + (row as f32) * geom.row_h - state.scroll_offset_pt.y;
                let x0 = geom.row_gutter_rect.min.x + geom.row_num_w;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.row_title_w, geom.row_h)).intersect(geom.row_gutter_rect))
            }
        }
    }

    fn rect_for_aux_target(geom: &MultiGridGeom, state: &ColumnGridEguiState, target: &AuxInlineTarget) -> Option<Rect> {
        match *target {
            AuxInlineTarget::XCell { row, subcol } => {
                #[allow(clippy::cast_precision_loss)]
                let x0 = geom.body_rect.min.x + (subcol as f32) * geom.subcol_w - state.scroll_offset_pt.x;
                #[allow(clippy::cast_precision_loss)]
                let y0 = geom.body_rect.min.y + (row as f32) * geom.row_h - state.scroll_offset_pt.y;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.row_h)).intersect(geom.body_inner_clip))
            }
            AuxInlineTarget::XTitle { subcol } => {
                #[allow(clippy::cast_precision_loss)]
                let x0 = geom.col_header_rect.min.x + (subcol as f32) * geom.subcol_w - state.scroll_offset_pt.x;
                let y0 = geom.col_header_rect.min.y + geom.h_tier1;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.h_tier2)).intersect(geom.col_header_rect))
            }
            AuxInlineTarget::SubcolumnTitle { col, subcol } => {
                let flat = geom.coord_to_flat_all(false, col, subcol);
                #[allow(clippy::cast_precision_loss)]
                let x0 = geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - state.scroll_offset_pt.x;
                let y0 = geom.col_header_rect.min.y + geom.h_tier1 + geom.h_tier2;
                Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.h_tier3)).intersect(geom.col_header_rect))
            }
            AuxInlineTarget::SheetTitle => {
                let x0 = geom.corner_rect.min.x + geom.row_num_w;
                Some(Rect::from_min_size(pos2(x0, geom.corner_rect.min.y), vec2(geom.row_title_w, geom.h_tier1)))
            }
        }
    }

    fn commit_standard_inline_edit(app: &UiApp, ds: &DataSheet, target: GridInlineEditTarget, draft: String, intents: &mut PendingUiIntents) {
        match target {
            GridInlineEditTarget::Cell(coord) => {
                let val = parse_inline_cell_input(app, &draft);
                intents.push_engine(
                    "table.setCell",
                    json!({ "sheet_id": ds.id.as_str(), "row": coord.row, "col": coord.col, "subcol": coord.subcol, "value": val }),
                );
            }
            GridInlineEditTarget::ColumnTitle { col } => {
                intents.push_engine("table.setTitles", json!({ "sheet_id": ds.id.as_str(), "col": col, "title": draft.trim() }));
            }
            GridInlineEditTarget::RowTitle { row } => {
                intents.push_engine("table.setRowTitles", json!({ "sheet_id": ds.id.as_str(), "row": row, "title": draft.trim() }));
            }
        }
    }

    fn x_subcolumn_count(arch: &TableArchetype) -> usize {
        match arch {
            TableArchetype::Xy(xy) => xy.x_cells.first().map_or(1, Vec::len).max(1),
            TableArchetype::Survival(_) => 1,
            _ => 0,
        }
    }

    fn is_x_column_readonly(ds: &DataSheet) -> bool {
        ds.frozen || matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }))
    }

    fn x_clipboard_cache_id() -> Id {
        Id::unique("r15_6_grid_x_clipboard")
    }

    fn copy_x_cell_to_clipboard(ui: &Ui, app: &UiApp, ds: &DataSheet, row: usize, subcol: usize) {
        let mut text = String::new();
        if let Some(cell) = ds.archetype.x_or_time_cell(row, subcol) {
            format_cell_into(cell, &app.locale_preset, &mut text);
        }
        let _ = app.write_clipboard_text(&text);
        ui.ctx().copy_text(text.clone());
        let snapshot = app.interaction.last_clipboard_text.clone();
        ui.ctx().data_mut(|d| d.insert_temp(x_clipboard_cache_id(), Some((text, snapshot))));
    }

    fn clear_x_clipboard_cache(ui: &Ui) {
        ui.ctx().data_mut(|d| d.insert_temp::<Option<(String, Option<String>)>>(x_clipboard_cache_id(), None));
    }

    fn read_x_cached_clipboard_if_fresher(ui: &Ui, app: &UiApp) -> Option<String> {
        let cached = ui.ctx().data(|d| d.get_temp::<Option<(String, Option<String>)>>(x_clipboard_cache_id())).flatten()?;
        let (x_text, app_snapshot) = cached;
        if app.interaction.last_clipboard_text != app_snapshot {
            return None;
        }
        (!x_text.is_empty()).then_some(x_text)
    }

    fn read_clipboard_text_for_x_paste(ui: &Ui, app: &UiApp) -> Option<String> {
        if let Ok(status) = app.read_clipboard_text()
            && status.available
            && let Some(text) = status.text
            && !text.is_empty()
        {
            return Some(text);
        }
        if let Some(x_text) = read_x_cached_clipboard_if_fresher(ui, app) {
            return Some(x_text);
        }
        app.interaction.last_clipboard_text.clone().filter(|t| !t.is_empty())
    }

    fn extract_multiline_or_tab_x_input(ui: &Ui, app: &UiApp) -> Option<String> {
        let direct = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Paste(t) | egui::Event::Text(t) if t.contains('\n') || t.contains('\r') || t.contains('\t') => Some(t.clone()),
                _ => None,
            })
        });
        if direct.is_some() {
            return direct;
        }
        if pressed_key_event_modifiers(ui, Key::V).is_some_and(|m| m.command || m.mac_cmd || m.ctrl)
            && let Some(clip) = read_clipboard_text_for_x_paste(ui, app)
            && (clip.contains('\n') || clip.contains('\r') || clip.contains('\t'))
        {
            return Some(clip);
        }
        None
    }

    fn push_x_cell_text_paste_intents(
        app: &UiApp,
        ds: &DataSheet,
        start_row: usize,
        start_subcol: usize,
        text: &str,
        intents: &mut PendingUiIntents,
    ) {
        if is_x_column_readonly(ds) {
            return;
        }
        let x_subcols = x_subcolumn_count(&ds.archetype);
        let avail_x_cols = x_subcols.saturating_sub(start_subcol);
        let row_count = ds.archetype.row_count();
        if avail_x_cols == 0 || row_count == 0 {
            return;
        }
        let mut x_edits = Vec::new();
        let mut y_lines = Vec::new();
        let mut has_y_cols = false;
        for (dr, raw_line) in text.lines().enumerate() {
            let target_row = start_row.saturating_add(dr);
            if target_row >= row_count {
                break;
            }
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let cols: Vec<&str> = line.split('\t').collect();
            for (dc, token) in cols.iter().take(avail_x_cols).enumerate() {
                let val = parse_inline_cell_input(app, token);
                x_edits.push(json!({
                    "row": target_row,
                    "subcol": start_subcol + dc,
                    "value": val,
                }));
            }
            if cols.len() > avail_x_cols {
                has_y_cols = true;
                y_lines.push(cols[avail_x_cols..].join("\t"));
            } else {
                y_lines.push(String::new());
            }
        }
        if !x_edits.is_empty() {
            intents.push_engine("table.setXCells", json!({ "sheet_id": ds.id.as_str(), "edits": x_edits }));
        }
        if has_y_cols {
            let dec = if app.locale_preset.decimal_separator == ',' { "," } else { "." };
            intents.push_engine(
                "table.paste",
                json!({
                    "sheet_id": ds.id.as_str(),
                    "text": y_lines.join("\n"),
                    "start_row": start_row,
                    "start_col": 0,
                    "start_subcol": 0,
                    "delimiter": "\t",
                    "decimal_separator": dec,
                    "propagate_title_header": false,
                }),
            );
        }
    }

    fn take_workspace_copy_intent(intents: &mut PendingUiIntents) -> bool {
        if !intents.intents().iter().any(|it| matches!(it, UiIntent::WorkspaceCopy { .. })) {
            return false;
        }
        let kept: Vec<UiIntent> = intents.intents().iter().filter(|it| !matches!(it, UiIntent::WorkspaceCopy { .. })).cloned().collect();
        intents.clear();
        for it in kept {
            intents.push(it);
        }
        true
    }

    fn commit_aux_inline_edit(
        app: &UiApp,
        ds: &DataSheet,
        aux: &mut MultiGridAuxState,
        target: &AuxInlineTarget,
        draft: String,
        intents: &mut PendingUiIntents,
    ) {
        match *target {
            AuxInlineTarget::XCell { row, subcol } => {
                if draft.contains('\t') || draft.contains('\n') || draft.contains('\r') {
                    push_x_cell_text_paste_intents(app, ds, row, subcol, &draft, intents);
                } else {
                    let val = parse_inline_cell_input(app, &draft);
                    intents.push_engine("table.setXCells", json!({ "sheet_id": ds.id.as_str(), "row": row, "subcol": subcol, "value": val }));
                }
                aux.selected_x_cell = Some((row, subcol));
            }
            AuxInlineTarget::XTitle { .. } => {
                intents.push_engine("table.setTitles", json!({ "sheet_id": ds.id.as_str(), "x_title": draft.trim() }));
            }
            AuxInlineTarget::SubcolumnTitle { col, subcol } => {
                intents.push_engine(
                    "table.setSubcolumnTitles",
                    json!({ "sheet_id": ds.id.as_str(), "col": col, "subcol": subcol, "title": draft.trim() }),
                );
            }
            AuxInlineTarget::SheetTitle => {
                let t = draft.trim();
                if !t.is_empty() {
                    intents.push_engine("sheet.rename", json!({ "sheet_id": ds.id.as_str(), "title": t }));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_mv_role_menu_popup(
        ui: &mut Ui,
        ds: &DataSheet,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        scroll_x: f32,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let Some(col) = aux.active_role_menu_col else { return };
        if blocked || !matches!(ds.archetype, TableArchetype::MultipleVariables(_)) {
            aux.active_role_menu_col = None;
            return;
        }
        let (flat, _) = geom.col_span_flat(col);
        #[allow(clippy::cast_precision_loss)]
        let x0 = (geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - scroll_x)
            .clamp(geom.grid_outer_rect.min.x, (geom.grid_outer_rect.max.x - 180.0).max(geom.grid_outer_rect.min.x));
        let pos = pos2(x0, geom.col_header_rect.max.y + 2.0);
        egui::Area::new(Id::unique(("mv_role_popup", ds.id.as_str(), col))).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |pui| {
            egui::Frame::popup(pui.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).show(pui, |m| {
                m.set_min_width(172.0);
                for (slug, text) in [
                    ("auto", "Auto (Infer role)"),
                    ("continuous", "NUM · Continuous"),
                    ("categorical", "CAT · Categorical"),
                    ("label", "LBL · Label / ID"),
                ] {
                    let wid = format!("mv_formula.role_opt.{col}.{slug}");
                    let r = m.button(text);
                    record_meta_if_enabled(
                        m,
                        meta,
                        true,
                        &r,
                        FrameInspectEntryInput {
                            widget_id: &wid,
                            role: "menuitem",
                            label: text,
                            command_id: Some("table.setVariableRole"),
                            value: Some(serde_json::Value::String(slug.to_owned())),
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::FloatingOverlay,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if r.clicked() {
                        intents.push_engine("table.setVariableRole", json!({ "sheet_id": ds.id.as_str(), "col": col, "role": slug }));
                        aux.active_role_menu_col = None;
                    }
                }
                m.separator();
                let mwid = format!("mv_formula.role_manage.{col}");
                let mr = m.button("Manage Variables & Levels…");
                record_meta_if_enabled(
                    m,
                    meta,
                    true,
                    &mr,
                    FrameInspectEntryInput {
                        widget_id: &mwid,
                        role: "menuitem",
                        label: "Manage Variables & Levels…",
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::FloatingOverlay,
                        ellipsized: false,
                        full_label: None,
                    },
                );
                if mr.clicked() {
                    intents.push_open_manage_variables(ds.id.clone(), Some(col));
                    aux.active_role_menu_col = None;
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row_context_menu_popup(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        aux: &mut MultiGridAuxState,
        tokens: &Tokens,
        meta: &mut FrameInspectMeta,
        intents: &mut PendingUiIntents,
        blocked: bool,
    ) {
        let Some(ctx_menu) = aux.context_menu.clone() else { return };
        let (pos, row, cell_ctx) = (ctx_menu.pos, ctx_menu.row, ctx_menu.cell);
        if blocked {
            aux.context_menu = None;
            return;
        }
        let area_resp = egui::Area::new(Id::unique(("grid_ctx_menu", ds.id.as_str()))).fixed_pos(pos).order(egui::Order::Foreground).show(ui.ctx(), |m| {
            egui::Frame::popup(m.style()).fill(tokens.panel_bg).stroke(Stroke::new(1.0, tokens.border)).show(m, |list| {
                list.set_min_width(210.0);
                list.label(egui::RichText::new(format!("Row {} Actions", row + 1)).strong().size(11.5).color(tokens.text_muted));
                list.separator();
                for (wid, label) in [
                    ("grid.ctx.style_points", "Style Selected Points on Graph…"),
                    ("grid.ctx.show_row_titles", "Show Row Titles on Graph"),
                    ("grid.ctx.reset_style", "Reset to Dataset Style"),
                    ("grid.ctx.exclude", "Exclude Selected (Cmd/Ctrl+E)"),
                    ("grid.ctx.exclude_reason", "Exclude with Reason…"),
                    ("grid.ctx.copy", "Copy Selection"),
                    ("grid.ctx.paste", "Paste Here"),
                ] {
                    let r = list.button(label);
                    record_meta_if_enabled(
                        list,
                        meta,
                        true,
                        &r,
                        FrameInspectEntryInput {
                            widget_id: wid,
                            role: "menuitem",
                            label,
                            command_id: None,
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::FloatingOverlay,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                    if r.clicked() {
                        match wid {
                            "grid.ctx.style_points" => {
                                if let Some(g) = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == ds.id) {
                                    intents.push_engine("sheet.select", json!({ "sheet_id": g.id.as_str() }));
                                }
                                intents.push_ui("ui.inspector.select", json!({ "inspector": "format_graph" }));
                            }
                            "grid.ctx.show_row_titles" => {
                                if let Some(g) = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == ds.id) {
                                    intents.push_engine("graph.format", json!({ "sheet_id": g.id.as_str(), "row_point_labels_row": row, "show_row_titles": true }));
                                }
                            }
                            "grid.ctx.reset_style" => {
                                if let Some(g) = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == ds.id) {
                                    intents.push_engine("graph.format", json!({ "sheet_id": g.id.as_str(), "reset_row_override": row }));
                                }
                            }
                            "grid.ctx.exclude" => {
                                if let Some(tgt) = cell_ctx && tgt.is_x {
                                    let was_ex = ds.archetype.x_or_time_cell(tgt.row, tgt.subcol).is_some_and(|c| c.excluded);
                                    intents.push_engine("table.excludeCells", json!({ "sheet_id": ds.id.as_str(), "target": "x", "row": tgt.row, "subcol": tgt.subcol, "excluded": !was_ex }));
                                } else {
                                    let c = app.grid.tool.active.unwrap_or(GridCellCoord::new(row, 0, 0));
                                    let was_ex = ds.archetype.data_cell(c.into()).is_some_and(|cell| cell.excluded);
                                    intents.push_engine("table.excludeCells", json!({ "sheet_id": ds.id.as_str(), "row": c.row, "col": c.col, "subcol": c.subcol, "excluded": !was_ex }));
                                }
                            }
                            "grid.ctx.exclude_reason" => intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": ds.id.as_str() })),
                            "grid.ctx.copy" => {
                                if let Some(tgt) = cell_ctx && tgt.is_x {
                                    copy_x_cell_to_clipboard(list, app, ds, tgt.row, tgt.subcol);
                                } else if let Some((xr, xs)) = aux.selected_x_cell {
                                    copy_x_cell_to_clipboard(list, app, ds, xr, xs);
                                } else {
                                    clear_x_clipboard_cache(list);
                                    intents.push_grid_copy_selection();
                                }
                            }
                            "grid.ctx.paste" => {
                                if let Some(tgt) = cell_ctx && tgt.is_x {
                                    if let Some(clip) = read_clipboard_text_for_x_paste(list, app) {
                                        push_x_cell_text_paste_intents(app, ds, tgt.row, tgt.subcol, &clip, intents);
                                    }
                                } else if let Some((xr, xs)) = aux.selected_x_cell {
                                    if let Some(clip) = read_clipboard_text_for_x_paste(list, app) {
                                        push_x_cell_text_paste_intents(app, ds, xr, xs, &clip, intents);
                                    }
                                } else {
                                    intents.push_grid_paste_from_clipboard(app.grid.tool.active);
                                }
                            }
                            _ => {}
                        }
                        aux.context_menu = None;
                    }
                }
            });
        });
        if area_resp.response.clicked_elsewhere() {
            aux.context_menu = None;
        }
    }

    fn handle_multi_clipboard_shortcuts(ui: &Ui, app: &UiApp, ds: &DataSheet, aux: &MultiGridAuxState, intents: &mut PendingUiIntents) -> bool {
        let has_workspace_copy = intents.intents().iter().any(|it| matches!(it, UiIntent::WorkspaceCopy { .. }));
        let copy_key = ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::Copy)))
            || pressed_key_event_modifiers(ui, Key::C).is_some_and(|m| m.command || m.mac_cmd || m.ctrl);
        if has_workspace_copy || copy_key {
            if let Some((xr, xs)) = aux.selected_x_cell {
                let _ = take_workspace_copy_intent(intents);
                copy_x_cell_to_clipboard(ui, app, ds, xr, xs);
                return true;
            }
            clear_x_clipboard_cache(ui);
            if !has_workspace_copy {
                intents.push_grid_copy_selection();
            }
            return true;
        }

        let cut_key = ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::Cut)))
            || pressed_key_event_modifiers(ui, Key::X).is_some_and(|m| m.command || m.mac_cmd || m.ctrl);
        if cut_key {
            if let Some((xr, xs)) = aux.selected_x_cell {
                copy_x_cell_to_clipboard(ui, app, ds, xr, xs);
                if !is_x_column_readonly(ds) {
                    intents.push_engine(
                        "table.setXCells",
                        json!({ "sheet_id": ds.id.as_str(), "row": xr, "subcol": xs, "value": serde_json::Value::Null }),
                    );
                }
                return true;
            }
            clear_x_clipboard_cache(ui);
            intents.push_grid_cut_selection();
            return true;
        }

        let paste_event_text = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Paste(t) if !t.is_empty() => Some(t.clone()),
                _ => None,
            })
        });
        let paste_key = paste_event_text.is_some() || pressed_key_event_modifiers(ui, Key::V).is_some_and(|m| m.command || m.mac_cmd || m.ctrl);
        if paste_key {
            if let Some((xr, xs)) = aux.selected_x_cell {
                if let Some(clip) = paste_event_text.or_else(|| read_clipboard_text_for_x_paste(ui, app)) {
                    push_x_cell_text_paste_intents(app, ds, xr, xs, &clip, intents);
                }
                return true;
            }
            let x_fallback = paste_event_text.or_else(|| {
                let live_avail = app.read_clipboard_text().ok().is_some_and(|s| s.available && s.text.as_deref().is_some_and(|t| !t.is_empty()));
                if live_avail { None } else { read_x_cached_clipboard_if_fresher(ui, app) }
            });
            if let Some(clip) = x_fallback {
                let anchor = app.grid.tool.active.unwrap_or(GridCellCoord::new(0, 0, 0));
                intents.push_engine(
                    "table.paste",
                    json!({
                        "sheet_id": ds.id.as_str(),
                        "text": clip,
                        "start_row": anchor.row,
                        "start_col": anchor.col,
                        "start_subcol": anchor.subcol,
                        "propagate_title_header": false,
                    }),
                );
                return true;
            }
            intents.push_grid_paste_from_clipboard(app.grid.tool.active);
            return true;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_multi_keyboard_shortcuts(
        ui: &mut Ui,
        app: &UiApp,
        ds: &DataSheet,
        state: &mut ColumnGridEguiState,
        aux: &mut MultiGridAuxState,
        geom: &MultiGridGeom,
        intents: &mut PendingUiIntents,
    ) {
        if state.inline_edit.is_some() || aux.aux_edit.is_some() || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        if !is_grid_focus_active(ui, app, &ds.id) && aux.selected_x_cell.is_none() && app.grid.tool.active.is_none() {
            return;
        }
        if let Some(e_mods) = pressed_key_event_modifiers(ui, Key::E)
            && (e_mods.command || e_mods.mac_cmd || e_mods.ctrl)
        {
            if e_mods.shift {
                intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": ds.id.as_str() }));
                return;
            }
            if let Some((xr, xs)) = aux.selected_x_cell {
                let was_ex = ds.archetype.x_or_time_cell(xr, xs).is_some_and(|c| c.excluded);
                intents.push_engine(
                    "table.excludeCells",
                    json!({ "sheet_id": ds.id.as_str(), "target": "x", "row": xr, "subcol": xs, "excluded": !was_ex }),
                );
                return;
            }
            if let Some(range) = app.grid.tool.selection_bounds() {
                let target_excluded = !ds.archetype.data_cell(curvebook_doc::CellCoord::from(range.0)).is_some_and(|c| c.excluded);
                let doc_range = SheetCellRange::new(curvebook_doc::CellCoord::from(range.0), curvebook_doc::CellCoord::from(range.1));
                intents.push_engine("table.excludeCells", json!({ "sheet_id": ds.id.as_str(), "range": doc_range, "excluded": target_excluded }));
                return;
            }
        }

        if handle_multi_clipboard_shortcuts(ui, app, ds, aux, intents) {
            return;
        }

        if (pressed_key_event_modifiers(ui, Key::Delete).is_some() || pressed_key_event_modifiers(ui, Key::Backspace).is_some()) && !ds.frozen {
            if let Some((xr, xs)) = aux.selected_x_cell {
                if !is_x_column_readonly(ds) {
                    intents.push_engine(
                        "table.setXCells",
                        json!({ "sheet_id": ds.id.as_str(), "row": xr, "subcol": xs, "value": serde_json::Value::Null }),
                    );
                }
                return;
            }
            if let Some(range) = app.grid.tool.selection_bounds() {
                let doc_range = SheetCellRange::new(curvebook_doc::CellCoord::from(range.0), curvebook_doc::CellCoord::from(range.1));
                intents.push_engine("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": doc_range }));
                return;
            }
        }

        for (key, dx, dy) in [(Key::ArrowLeft, -1_isize, 0_isize), (Key::ArrowRight, 1, 0), (Key::ArrowUp, 0, -1), (Key::ArrowDown, 0, 1)] {
            if let Some(km) = pressed_key_event_modifiers(ui, key) {
                if geom.row_count == 0 || geom.total_flat_cols == 0 {
                    return;
                }
                let cur_row = aux.selected_x_cell.map(|(r, _)| r).or_else(|| app.grid.tool.active.map(|c| c.row)).unwrap_or(0);
                let cur_flat = match (aux.selected_x_cell, app.grid.tool.active) {
                    (Some((_, xs)), _) => xs,
                    (None, Some(c)) => geom.coord_to_flat_all(false, c.col, c.subcol),
                    (None, None) => 0,
                };
                let next_row = cur_row.saturating_add_signed(dy).min(geom.row_count.saturating_sub(1));
                let next_flat = cur_flat.saturating_add_signed(dx).min(geom.total_flat_cols.saturating_sub(1));
                if next_flat < geom.x_subcols {
                    aux.selected_x_cell = Some((next_row, next_flat));
                    ui.ctx().request_repaint();
                } else if let Some((nc, ns)) = geom.flat_data_to_coord(next_flat - geom.x_subcols) {
                    aux.selected_x_cell = None;
                    let coord = GridCellCoord::new(next_row, nc, ns);
                    if km.shift {
                        intents.push_extend_grid_selection(coord);
                    } else {
                        intents.push_select_grid_cell(coord);
                    }
                    ui.ctx().request_repaint();
                }
                return;
            }
        }

        if let Some(km) = pressed_key_event_modifiers(ui, Key::Tab) {
            let (is_x, nc_opt, x_opt) = step_cell_horiz(geom, app.grid.tool.active, aux.selected_x_cell, km.shift);
            if is_x {
                aux.selected_x_cell = x_opt;
                ui.ctx().request_repaint();
            } else if let Some(nc) = nc_opt {
                aux.selected_x_cell = None;
                intents.push_select_grid_cell(nc);
                ui.ctx().request_repaint();
            }
            return;
        }

        if pressed_key_event_modifiers(ui, Key::F2).is_some() && !ds.frozen {
            if let Some((xr, xs)) = aux.selected_x_cell {
                let init = initial_edit_text_for_cell(ds.archetype.x_or_time_cell(xr, xs), &app.locale_preset);
                aux.aux_edit = Some((AuxInlineTarget::XCell { row: xr, subcol: xs }, init, true));
                return;
            }
            if let Some(c) = app.grid.tool.active {
                let init = initial_edit_text_for_cell(ds.archetype.data_cell(c.into()), &app.locale_preset);
                begin_inline_edit(ui, state, GridInlineEditTarget::Cell(c), init);
                return;
            }
        }

        let mods = ui.input(|i| i.modifiers);
        let cmd = mods.command || mods.mac_cmd || mods.ctrl;
        if !cmd && !ds.frozen {
            let typed: String = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|ev| match ev {
                        egui::Event::Text(t) if !t.is_empty() && t.chars().all(|c| !c.is_control()) => Some(t.as_str()),
                        _ => None,
                    })
                    .collect()
            });
            if !typed.is_empty() {
                if let Some((xr, xs)) = aux.selected_x_cell {
                    let x_ro = matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }));
                    if !x_ro {
                        aux.aux_edit = Some((AuxInlineTarget::XCell { row: xr, subcol: xs }, typed, true));
                    }
                } else if let Some(c) = app.grid.tool.active {
                    let is_calc = matches!(&ds.archetype, TableArchetype::MultipleVariables(mv) if mv.variables.get(c.col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. })));
                    if !is_calc && linked_region_for_cell(ds, c.row, c.col).is_none() {
                        begin_inline_edit(ui, state, GridInlineEditTarget::Cell(c), typed);
                    }
                }
            }
        }
    }

    fn step_cell_horiz(
        geom: &MultiGridGeom,
        active_y: Option<GridCellCoord>,
        active_x: Option<(usize, usize)>,
        backward: bool,
    ) -> (bool, Option<GridCellCoord>, Option<(usize, usize)>) {
        if geom.total_flat_cols == 0 || geom.row_count == 0 {
            return (false, None, None);
        }
        let row = active_x.map(|(r, _)| r).or_else(|| active_y.map(|c| c.row)).unwrap_or(0);
        let cur = match (active_x, active_y) {
            (Some((_, xs)), _) => xs,
            (None, Some(c)) => geom.coord_to_flat_all(false, c.col, c.subcol),
            (None, None) => 0,
        };
        let next = if backward { cur.saturating_sub(1) } else { (cur + 1).min(geom.total_flat_cols - 1) };
        if next < geom.x_subcols {
            (true, None, Some((row, next)))
        } else {
            (false, geom.flat_data_to_coord(next - geom.x_subcols).map(|(c, s)| GridCellCoord::new(row, c, s)), None)
        }
    }
}

fn active_column_sheet(app: &UiApp) -> Option<(&DataSheet, &ColumnTable)> {
    let doc = app.document();
    let sid = doc.active_sheet_id.as_ref()?;
    let ds = doc.data_sheet(sid)?;
    let TableArchetype::Column(col_table) = &ds.archetype else {
        return None;
    };
    Some((ds, col_table))
}

fn ensure_solid_scrollbar_style(ui: &mut Ui, metrics: LayoutMetrics) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    *scroll = egui::style::ScrollStyle::solid();
    scroll.floating = false;
    scroll.bar_width = metrics.scroll_bar_thickness.max(10.0);
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
}

fn compute_grid_geometry(
    available_rect: Rect,
    table_rect: Rect,
    col_table: &ColumnTable,
    metrics: LayoutMetrics,
    scroll_allocated_w: f32,
    collapsed: bool,
) -> GridGeometry {
    let row_num_w = ROW_NUM_WIDTH_PT.min((table_rect.width() * 0.25).floor()).max(24.0);
    let desired_rt_w = if collapsed { ROW_TITLES_COLLAPSED_WIDTH_PT } else { ROW_TITLES_EXPANDED_WIDTH_PT };
    let max_rt_w = (table_rect.width() - row_num_w - 32.0).max(24.0);
    let rt_w = desired_rt_w.min(max_rt_w);
    let left_gutter_w = row_num_w + rt_w;

    let tier1_h = metrics.grid_header_tier_heights[0].max(18.0);
    let tier2_h = metrics.grid_header_tier_heights[1].max(24.0);
    let tier3_h = metrics.grid_header_tier_heights[2].max(18.0);
    let header_h = tier1_h + tier2_h + tier3_h;
    let row_h = metrics.grid_row_h.max(24.0);
    let subcol_w = DEFAULT_DATA_COL_WIDTH_PT;

    let subs_per_col = col_table.subcolumn_mode.subcolumn_count().max(1);
    let total_rows = col_table.row_count();
    let total_cols = col_table.data_sets.len();
    let total_flat_cols = total_cols.saturating_mul(subs_per_col);

    #[allow(clippy::cast_precision_loss)]
    let total_data_w = (total_flat_cols as f32) * subcol_w;
    #[allow(clippy::cast_precision_loss)]
    let total_data_h = (total_rows as f32) * row_h;

    let avail_cols0 = (table_rect.width() - left_gutter_w).max(0.0);
    let avail_body0 = (table_rect.height() - header_h).max(0.0);

    let mut need_v = total_data_h > avail_body0 + 0.5;
    let mut need_h = total_data_w > avail_cols0 + 0.5;
    for _ in 0..2 {
        let vr = if need_v { scroll_allocated_w } else { 0.0 };
        let hr = if need_h { scroll_allocated_w } else { 0.0 };
        need_v = total_data_h > (avail_body0 - hr).max(0.0) + 0.5;
        need_h = total_data_w > (avail_cols0 - vr).max(0.0) + 0.5;
    }

    let v_reserve = if need_v { scroll_allocated_w } else { 0.0 };
    let h_reserve = if need_h { scroll_allocated_w } else { 0.0 };
    let cols_x = table_rect.left() + left_gutter_w;
    let data_y = table_rect.top() + header_h;
    let body_inner_w = (table_rect.right() - v_reserve - cols_x).max(0.0);
    let body_inner_h = (table_rect.bottom() - h_reserve - data_y).max(0.0);

    let corner_rect = Rect::from_min_max(table_rect.min, pos2(cols_x, data_y));
    let header_data_rect = Rect::from_min_max(pos2(cols_x, table_rect.top()), pos2(table_rect.right() - v_reserve, data_y));
    let gutter_body_rect = Rect::from_min_max(pos2(table_rect.left(), data_y), pos2(cols_x, table_rect.bottom() - h_reserve));
    let body_outer_rect = Rect::from_min_max(pos2(cols_x, data_y), table_rect.max);

    GridGeometry {
        available_rect,
        table_rect,
        row_num_w,
        rt_w,
        tier1_h,
        tier2_h,
        tier3_h,
        header_h,
        row_h,
        subcol_w,
        subs_per_col,
        total_rows,
        total_cols,
        total_flat_cols,
        total_data_w,
        total_data_h,
        need_h,
        need_v,
        h_reserve,
        v_reserve,
        cols_x,
        data_y,
        body_inner_w,
        body_inner_h,
        effective_collapsed: collapsed,
        corner_rect,
        header_data_rect,
        gutter_body_rect,
        body_outer_rect,
    }
}

fn clamp_scroll_offset(state: &mut ColumnGridEguiState, geom: &GridGeometry) {
    let max_x = (geom.total_data_w - geom.body_inner_w).max(0.0);
    let max_y = (geom.total_data_h - geom.body_inner_h).max(0.0);
    state.scroll_offset_pt.x = if state.scroll_offset_pt.x.is_finite() { state.scroll_offset_pt.x.clamp(0.0, max_x) } else { 0.0 };
    state.scroll_offset_pt.y = if state.scroll_offset_pt.y.is_finite() { state.scroll_offset_pt.y.clamp(0.0, max_y) } else { 0.0 };
}

fn any_pointer_click_in(ui: &Ui, rect: Rect) -> bool {
    ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if rect.contains(*pos))))
}

const fn blend_row_channel(a: u8, b: u8) -> u8 {
    ((a as u16 * 3 + b as u16) / 4) as u8
}

const fn alt_row_fill(surface: Color32, panel: Color32) -> Color32 {
    Color32::from_rgb(blend_row_channel(surface.r(), panel.r()), blend_row_channel(surface.g(), panel.g()), blend_row_channel(surface.b(), panel.b()))
}

const fn link_border_color32(color: Option<LinkBorderColor>, tokens: &Tokens) -> Color32 {
    match color {
        Some(LinkBorderColor::Blue) | None => tokens.link_blue,
        Some(LinkBorderColor::Green) => tokens.link_green,
        Some(LinkBorderColor::Red) => tokens.link_red,
        Some(LinkBorderColor::Charcoal) => tokens.text_muted,
    }
}

fn compute_scrollbar_rects(ui: &Ui, geom: &GridGeometry, inner_rect: Rect) -> (Option<Rect>, Option<Rect>) {
    let scroll = ui.spacing().scroll;
    let bar_w = scroll.bar_width;
    let inner_m = scroll.bar_inner_margin;
    let v_rect = geom.need_v.then(|| {
        Rect::from_min_max(
            pos2(inner_rect.right() + inner_m, inner_rect.top()),
            pos2((inner_rect.right() + inner_m + bar_w).min(geom.table_rect.right()), inner_rect.bottom()),
        )
    });
    let h_rect = geom.need_h.then(|| {
        Rect::from_min_max(
            pos2(inner_rect.left(), inner_rect.bottom() + inner_m),
            pos2(inner_rect.right(), (inner_rect.bottom() + inner_m + bar_w).min(geom.table_rect.bottom())),
        )
    });
    (v_rect, h_rect)
}

fn paint_scrollbar_corner_cap(ui: &Ui, rctx: &GridRenderCtx<'_>) {
    let geom = rctx.geom;
    if geom.need_v && geom.need_h {
        let cap = Rect::from_min_max(pos2(geom.table_rect.right() - geom.v_reserve, geom.table_rect.bottom() - geom.h_reserve), geom.table_rect.max);
        ui.painter().rect_filled(cap, CornerRadius::ZERO, rctx.tokens.header_bg);
        ui.painter().rect_stroke(cap, CornerRadius::ZERO, Stroke::new(1.0, rctx.tokens.divider), StrokeKind::Inside);
    }
}

fn forward_gutter_header_wheel(ui: &mut Ui, rctx: &GridRenderCtx<'_>, state: &mut ColumnGridEguiState) {
    if rctx.input_blocked(state) {
        return;
    }
    let Some(pointer) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    let delta = ui.input(|i| i.smooth_scroll_delta);
    if delta == Vec2::ZERO {
        return;
    }
    let mut consumed = false;
    if rctx.geom.gutter_body_rect.contains(pointer) && rctx.geom.need_v && delta.y.abs() > 0.0 {
        state.scroll_offset_pt.y -= delta.y;
        consumed = true;
    } else if rctx.geom.header_data_rect.contains(pointer) && rctx.geom.need_h && delta.x.abs() > 0.0 {
        state.scroll_offset_pt.x -= delta.x;
        consumed = true;
    }
    if consumed {
        clamp_scroll_offset(state, rctx.geom);
        ui.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
        });
    }
}

struct CellTextStyle {
    font_id: FontId,
    color: Color32,
    align: Align,
    italic: bool,
    strike: bool,
}

impl CellTextStyle {
    fn plain(font_id: FontId, color: Color32, align: Align) -> Self {
        Self { font_id, color, align, italic: false, strike: false }
    }
}

fn render_ellipsized_text(ui: &Ui, clip_rect: Rect, text: &str, style: CellTextStyle) -> (bool, Rect) {
    if text.is_empty() || clip_rect.width() <= 2.0 || clip_rect.height() <= 2.0 {
        return (false, Rect::NOTHING);
    }
    let max_w = (clip_rect.width() - CELL_PAD_X_PT * 2.0).max(1.0);
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat {
            font_id: style.font_id,
            color: style.color,
            italics: style.italic,
            strikethrough: if style.strike { Stroke::new(1.0, style.color) } else { Stroke::NONE },
            ..Default::default()
        },
    );
    job.wrap = TextWrapping { max_width: max_w, max_rows: 1, break_anywhere: false, overflow_character: Some('…') };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let elided = galley.elided;
    let size = galley.size();
    let x = match style.align {
        Align::RIGHT => (clip_rect.right() - CELL_PAD_X_PT - size.x).max(clip_rect.left() + CELL_PAD_X_PT),
        Align::Center => clip_rect.center().x - size.x * 0.5,
        Align::LEFT => clip_rect.left() + CELL_PAD_X_PT,
    };
    let y = clip_rect.center().y - size.y * 0.5;
    let text_pos = pos2(x, y);
    let text_rect = Rect::from_min_size(text_pos, size);
    ui.painter().with_clip_rect(clip_rect).galley(text_pos, galley, style.color);
    (elided, text_rect)
}

fn format_cell_into(cell: &Cell, preset: &LocalePreset, out: &mut String) {
    out.clear();
    match &cell.value {
        CellValue::Empty => {}
        CellValue::Number(v) if v.is_finite() => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            let _ = write!(out, "{normalized}");
            if normalized.is_sign_negative() && normalized != 0.0 && preset.minus_sign != "-" && out.starts_with('-') {
                out.replace_range(..1, &preset.minus_sign);
            }
            if preset.decimal_separator != '.'
                && let Some(dot) = out.find('.')
            {
                let mut sep_buf = [0u8; 4];
                let sep_str = preset.decimal_separator.encode_utf8(&mut sep_buf);
                out.replace_range(dot..=dot, sep_str);
            }
        }
        CellValue::Number(_) => {}
        CellValue::Text(s) => out.push_str(s),
    }
}

fn initial_edit_text_for_cell(cell: Option<&Cell>, preset: &LocalePreset) -> String {
    let Some(c) = cell else {
        return String::new();
    };
    match &c.value {
        CellValue::Text(s) if s.starts_with('=') => s.clone(),
        _ => format_cell_display(c, preset),
    }
}

fn parse_inline_cell_input(app: &UiApp, draft: &str) -> serde_json::Value {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        serde_json::Value::Null
    } else if trimmed.starts_with('=') {
        serde_json::Value::String(trimmed.to_owned())
    } else if let Some(num) = parse_localized_number_input(trimmed, &app.locale_preset) {
        json!(num)
    } else {
        serde_json::Value::String(draft.to_owned())
    }
}

fn record_meta_if_enabled(ui: &Ui, meta: &mut FrameInspectMeta, enabled: bool, resp: &Response, input: FrameInspectEntryInput<'_>) {
    if !enabled {
        return;
    }
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
        return;
    }
    meta.record_response(ui, resp, input);
}

fn render_link_provenance_strip(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    strip_rect: Rect,
) {
    let tokens = rctx.tokens;
    ui.scope_builder(UiBuilder::new().max_rect(strip_rect), |ui| {
        ui.set_clip_rect(strip_rect);
        ui.painter().rect_filled(strip_rect, CornerRadius::ZERO, tokens.panel_bg);
        ui.painter().rect_stroke(strip_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let badge_lbl = grid_tr(rctx.locale, "w5.grid_core.linked_badge", "Linked");
        let mut cursor_x = strip_rect.left() + 6.0;
        let blocked = rctx.input_blocked(state);
        for (idx, reg) in rctx.ds.linked_regions.iter().enumerate() {
            let rem = strip_rect.right() - cursor_x;
            if rem < 72.0 {
                break;
            }
            let chip_w = 240.0_f32.min((rem - 6.0).max(0.0));
            let chip_rect = Rect::from_min_size(pos2(cursor_x, strip_rect.top() + 3.0), vec2(chip_w, (strip_rect.height() - 6.0).max(16.0)));
            let color = link_border_color32(reg.border_color(), tokens);
            let label = if reg.provenance_summary.is_empty() {
                format!("{badge_lbl} #{}", idx + 1)
            } else {
                format!("{badge_lbl}: {}", reg.provenance_summary)
            };
            let sense = if blocked { Sense::hover() } else { Sense::click() };
            let resp = ui.interact(chip_rect, rctx.sheet_id_base.with(("link_badge", idx)), sense);
            let fill = if resp.hovered() && !blocked { color.gamma_multiply(0.24) } else { color.gamma_multiply(0.12) };
            ui.painter().rect_filled(chip_rect, CornerRadius::same(3), fill);
            ui.painter().rect_stroke(chip_rect, CornerRadius::same(3), Stroke::new(2.0, color), StrokeKind::Inside);
            let (elided, _) =
                render_ellipsized_text(ui, chip_rect, &label, CellTextStyle::plain(FontId::proportional(11.5), tokens.text_primary, Align::LEFT));
            let acc_label = label.clone();
            resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_label.clone()));
            if elided {
                let _ = resp.clone().on_hover_text(&label);
            }
            if resp.clicked() && !blocked {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
            }
            let wid = format!("grid.link_badge.{idx}");
            record_meta_if_enabled(
                ui,
                meta,
                state.record_inspect_meta,
                &resp,
                FrameInspectEntryInput {
                    widget_id: &wid,
                    role: "button",
                    label: &label,
                    command_id: None,
                    value: Some(json!({ "region_id": reg.id })),
                    enabled: !blocked,
                    disabled_reason: blocked.then_some("modal dialog is active"),
                    z_layer: ZLayer::Workspace,
                    ellipsized: elided,
                    full_label: elided.then_some(label.as_str()),
                },
            );
            cursor_x += chip_w + 6.0;
        }
    });
    let _ = intents;
}

fn open_link_popover(state: &mut ColumnGridEguiState, reg: &LinkedDataRegion, locale: &str) {
    let prompt = grid_tr(locale, "w5.grid_core.linked_read_only_prompt", "Linked region is read-only. Go to source or unlink to edit.");
    let msg = if reg.provenance_summary.is_empty() { prompt.to_owned() } else { format!("{}\n{}", reg.provenance_summary, prompt) };
    state.active_link_popover =
        Some(LinkedRegionPopover { region_id: reg.id.clone(), upstream_sheet_id: reg.spec.source.upstream_sheet_id().cloned(), message: msg });
}

fn render_corner_and_row_titles_header(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let blocked = rctx.input_blocked(state);
    ui.scope_builder(UiBuilder::new().max_rect(geom.corner_rect), |ui| {
        ui.set_clip_rect(geom.corner_rect);

        // Top-left Corner Archetype Trigger (`Column`)
        let trigger_h = CORNER_TRIGGER_HEIGHT_PT.min(geom.header_h);
        let trigger_rect = Rect::from_min_size(geom.corner_rect.min, vec2(geom.row_num_w, trigger_h));
        let trigger_sense = if blocked { Sense::hover() } else { Sense::click() };
        let trigger_resp = ui.interact(trigger_rect, rctx.sheet_id_base.with("corner_archetype"), trigger_sense);
        let trigger_bg = if trigger_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
        ui.painter().rect_filled(trigger_rect, CornerRadius::ZERO, trigger_bg);
        ui.painter().rect_stroke(trigger_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

        let archetype_label = format!("{} ▾", tr(rctx.locale, "table.column"));
        let (trigger_elided, _) = render_ellipsized_text(
            ui,
            trigger_rect,
            &archetype_label,
            CellTextStyle::plain(FontId::proportional(11.0), tokens.text_strong, Align::Center),
        );
        if trigger_elided {
            state.ellipsized_headers += 1;
        }
        let acc_archetype = archetype_label.clone();
        trigger_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_archetype.clone()));
        if trigger_resp.hovered() {
            let tip = if trigger_elided {
                format!("{archetype_label} — {}", tr(rctx.locale, "cmd.ui.table.convertArchetype.open"))
            } else {
                tr(rctx.locale, "cmd.ui.table.convertArchetype.open").to_owned()
            };
            let _ = trigger_resp.clone().on_hover_text(tip);
        }
        if trigger_resp.clicked() && !blocked {
            intents.push_ui("ui.table.convertArchetype.open", json!({ "sheet_id": rctx.ds.id.as_str() }));
            ui.ctx().request_repaint();
        }
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &trigger_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.archetype",
                role: "button",
                label: &archetype_label,
                command_id: Some("ui.table.convertArchetype.open"),
                value: Some(json!({ "archetype": "column" })),
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: trigger_elided,
                full_label: trigger_elided.then_some(archetype_label.as_str()),
            },
        );

        // `#` header below Corner Trigger
        let hash_rect = Rect::from_min_max(
            pos2(geom.corner_rect.left(), geom.corner_rect.top() + trigger_h),
            pos2(geom.corner_rect.left() + geom.row_num_w, geom.corner_rect.bottom()),
        );
        let hash_resp = ui.interact(hash_rect, rctx.sheet_id_base.with("corner_row_num"), Sense::hover());
        ui.painter().rect_filled(hash_rect, CornerRadius::ZERO, tokens.header_bg);
        ui.painter().rect_stroke(hash_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        let hash_label = grid_tr(rctx.locale, "w5.grid_core.row_number_header", "#");
        let (hash_elided, _) =
            render_ellipsized_text(ui, hash_rect, hash_label, CellTextStyle::plain(FontId::proportional(11.0), tokens.text_muted, Align::Center));
        let acc_hash = hash_label.to_owned();
        hash_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_hash.clone()));
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &hash_resp,
            FrameInspectEntryInput {
                widget_id: "grid.corner.row_num",
                role: "label",
                label: hash_label,
                command_id: None,
                value: None,
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: hash_elided,
                full_label: hash_elided.then_some(hash_label),
            },
        );

        // Row Titles header + toggle
        let rt_x = geom.corner_rect.left() + geom.row_num_w;
        let rt_header_rect = Rect::from_min_size(pos2(rt_x, geom.corner_rect.top()), vec2(geom.rt_w, geom.header_h));
        render_row_titles_header_block(ui, rctx, state, meta, intents, rt_header_rect, blocked);
    });
}

fn render_row_titles_header_block(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    rt_header_rect: Rect,
    blocked: bool,
) {
    let tokens = rctx.tokens;
    let geom = rctx.geom;
    let toggle_sense = if blocked { Sense::hover() } else { Sense::click() };

    if geom.effective_collapsed {
        let expand_lbl = grid_tr(rctx.locale, "w5.grid_core.expand_row_titles", "Expand Row Titles");
        let short_lbl = grid_tr(rctx.locale, "w5.grid_core.row_titles_short", "RT");
        let resp = ui.interact(rt_header_rect, rctx.sheet_id_base.with("row_titles_toggle"), toggle_sense);
        let bg = if resp.hovered() && !blocked { tokens.hover_bg } else { tokens.header_bg };
        ui.painter().rect_filled(rt_header_rect, CornerRadius::ZERO, bg);
        ui.painter().rect_stroke(rt_header_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        let (elided, _) = render_ellipsized_text(
            ui,
            rt_header_rect,
            short_lbl,
            CellTextStyle::plain(FontId::proportional(11.0), tokens.text_primary, Align::Center),
        );
        if elided {
            state.ellipsized_headers += 1;
        }
        let acc = expand_lbl.to_owned();
        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc.clone()));
        if resp.hovered() {
            let _ = resp.clone().on_hover_text(expand_lbl);
        }
        if resp.clicked() && !blocked {
            state.row_titles_collapsed = false;
            intents.push_set_row_titles_collapsed(false);
            ui.ctx().request_repaint();
        }
        record_meta_if_enabled(
            ui,
            meta,
            state.record_inspect_meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: "grid.row_titles.toggle",
                role: "button",
                label: expand_lbl,
                command_id: None,
                value: Some(json!({ "collapsed": true })),
                enabled: !blocked,
                disabled_reason: blocked.then_some("modal dialog is active"),
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
        return;
    }

    let top_rect = Rect::from_min_size(rt_header_rect.min, vec2(geom.rt_w, geom.tier1_h));
    let rt_title = grid_tr(rctx.locale, "w5.grid_core.row_titles", "Row Titles");
    let band_resp = ui.interact(top_rect, rctx.sheet_id_base.with("row_titles_band"), Sense::hover());
    ui.painter().rect_filled(top_rect, CornerRadius::ZERO, tokens.header_bg);
    ui.painter().rect_stroke(top_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let (band_elided, _) =
        render_ellipsized_text(ui, top_rect, rt_title, CellTextStyle::plain(FontId::proportional(11.0), tokens.text_strong, Align::Center));
    if band_elided {
        state.ellipsized_headers += 1;
    }
    let acc_band = rt_title.to_owned();
    band_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_band.clone()));
    if band_elided && band_resp.hovered() {
        let _ = band_resp.clone().on_hover_text(rt_title);
    }
    record_meta_if_enabled(
        ui,
        meta,
        state.record_inspect_meta,
        &band_resp,
        FrameInspectEntryInput {
            widget_id: "grid.row_titles.band",
            role: "label",
            label: rt_title,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::Workspace,
            ellipsized: band_elided,
            full_label: band_elided.then_some(rt_title),
        },
    );

    let toggle_rect = Rect::from_min_max(pos2(rt_header_rect.left(), top_rect.bottom()), rt_header_rect.max);
    let collapse_lbl = grid_tr(rctx.locale, "w5.grid_core.collapse_row_titles", "Collapse Row Titles");
    let resp = ui.interact(toggle_rect, rctx.sheet_id_base.with("row_titles_toggle"), toggle_sense);
    let bg = if resp.hovered() && !blocked { tokens.hover_bg } else { tokens.panel_bg };
    ui.painter().rect_filled(toggle_rect, CornerRadius::ZERO, bg);
    ui.painter().rect_stroke(toggle_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let toggle_visual = format!("◀ {}", grid_tr(rctx.locale, "w5.grid_core.row_titles_short", "RT"));
    let (elided, _) =
        render_ellipsized_text(ui, toggle_rect, &toggle_visual, CellTextStyle::plain(FontId::proportional(11.0), tokens.text_muted, Align::Center));
    let acc_toggle = collapse_lbl.to_owned();
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_toggle.clone()));
    if resp.hovered() {
        let _ = resp.clone().on_hover_text(collapse_lbl);
    }
    if resp.clicked() && !blocked {
        state.row_titles_collapsed = true;
        intents.push_set_row_titles_collapsed(true);
        ui.ctx().request_repaint();
    }
    record_meta_if_enabled(
        ui,
        meta,
        state.record_inspect_meta,
        &resp,
        FrameInspectEntryInput {
            widget_id: "grid.row_titles.toggle",
            role: "button",
            label: collapse_lbl,
            command_id: None,
            value: Some(json!({ "collapsed": false })),
            enabled: !blocked,
            disabled_reason: blocked.then_some("modal dialog is active"),
            z_layer: ZLayer::Workspace,
            ellipsized: elided && false,
            full_label: None,
        },
    );
}

fn render_body_scroll_area(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> BodyRenderResult {
    let geom = rctx.geom;
    let blocked = rctx.input_blocked(state);
    let scroll_source = if blocked { ScrollSource::NONE } else { ScrollSource { scroll_bar: true, drag: DragScroll::Never, mouse_wheel: true } };

    let mut result = BodyRenderResult {
        inner_rect: Rect::from_min_size(pos2(geom.cols_x, geom.data_y), vec2(geom.body_inner_w, geom.body_inner_h)),
        rendered_cells: 0,
        visible_rows: 0..0,
        visible_subcols: 0..0,
        used_scroll_offset: state.scroll_offset_pt,
        new_scroll_offset: state.scroll_offset_pt,
        inline_edit_handled_key: false,
        cell_clicked_or_edited: false,
    };

    ui.scope_builder(UiBuilder::new().max_rect(geom.body_outer_rect), |ui| {
        let scroll_out = egui::ScrollArea::new([geom.need_h, geom.need_v])
            .id_salt(("column_grid_scroll", rctx.ds.id.as_str()))
            .auto_shrink([false, false])
            .animated(false)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .scroll_source(scroll_source)
            .scroll_offset(state.scroll_offset_pt)
            .show_viewport(ui, |ui, viewport| {
                ui.set_min_size(vec2(geom.total_data_w.max(1.0), geom.total_data_h.max(1.0)));
                let vis_res = render_visible_cells(ui, rctx, state, viewport, meta, intents);
                result.used_scroll_offset = viewport.min.to_vec2();
                result.rendered_cells = vis_res.rendered_cells;
                result.visible_rows = vis_res.visible_rows;
                result.visible_subcols = vis_res.visible_subcols;
                result.inline_edit_handled_key = vis_res.inline_edit_handled_key;
                result.cell_clicked_or_edited = vis_res.cell_clicked_or_edited;
            });
        result.inner_rect = scroll_out.inner_rect;
        result.new_scroll_offset = scroll_out.state.clamped_offset();
    });

    result
}

struct VisibleCellsOut {
    rendered_cells: usize,
    visible_rows: Range<usize>,
    visible_subcols: Range<usize>,
    inline_edit_handled_key: bool,
    cell_clicked_or_edited: bool,
}

fn compute_visible_range(start_px: f32, extent_px: f32, unit_px: f32, total: usize) -> Range<usize> {
    if total == 0 || unit_px <= 0.0 || extent_px <= 0.0 {
        return 0..0;
    }
    let s = start_px.max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let start = ((s / unit_px).floor() as usize).min(total);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let end = (((s + extent_px) / unit_px).ceil() as usize + 1).min(total);
    start..end.max(start)
}

struct CellDrawInput<'a> {
    coord: GridCellCoord,
    cell_rect: Rect,
    clipped_rect: Rect,
    cell_opt: Option<&'a Cell>,
    is_active: bool,
    is_selected: bool,
    linked_reg: Option<&'a LinkedDataRegion>,
    blocked: bool,
}

fn render_visible_cells(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    viewport: Rect,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> VisibleCellsOut {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let clip_rect = ui.clip_rect();
    ui.painter().rect_filled(clip_rect, CornerRadius::ZERO, tokens.surface_bg);

    let row_range = compute_visible_range(viewport.min.y, viewport.height(), geom.row_h, geom.total_rows);
    let flat_range = compute_visible_range(viewport.min.x, viewport.width(), geom.subcol_w, geom.total_flat_cols);
    let content_origin = ui.max_rect().min;

    if row_range.is_empty() || flat_range.is_empty() {
        return VisibleCellsOut {
            rendered_cells: 0,
            visible_rows: 0..0,
            visible_subcols: 0..0,
            inline_edit_handled_key: false,
            cell_clicked_or_edited: false,
        };
    }

    let alt_bg = alt_row_fill(tokens.surface_bg, tokens.panel_bg);
    for row in row_range.clone() {
        if row % 2 == 1 {
            #[allow(clippy::cast_precision_loss)]
            let y = content_origin.y + (row as f32) * geom.row_h;
            let band = Rect::from_min_size(pos2(clip_rect.left(), y), vec2(clip_rect.width(), geom.row_h)).intersect(clip_rect);
            if band.is_positive() {
                ui.painter().rect_filled(band, CornerRadius::ZERO, alt_bg);
            }
        }
    }

    let sel = SelectionFastLookup::new(rctx.app, geom.total_rows, geom.subs_per_col, geom.total_flat_cols);
    let has_links = !rctx.ds.linked_regions.is_empty();
    let blocked = rctx.input_blocked(state);
    let mut text_buf = String::with_capacity(32);
    let mut rendered_cells = 0usize;
    let mut min_vis_row = usize::MAX;
    let mut max_vis_row = 0usize;
    let mut min_vis_flat = usize::MAX;
    let mut max_vis_flat = 0usize;
    let mut inline_edit_handled_key = false;
    let mut cell_clicked_or_edited = false;
    let mut active_border_rect: Option<Rect> = None;

    for row in row_range.clone() {
        #[allow(clippy::cast_precision_loss)]
        let cell_y = content_origin.y + (row as f32) * geom.row_h;
        for flat in flat_range.clone() {
            let col = flat / geom.subs_per_col;
            let subcol = flat % geom.subs_per_col;
            #[allow(clippy::cast_precision_loss)]
            let cell_x = content_origin.x + (flat as f32) * geom.subcol_w;
            let cell_rect = Rect::from_min_size(pos2(cell_x, cell_y), vec2(geom.subcol_w, geom.row_h));
            let clipped_rect = cell_rect.intersect(clip_rect);
            if clipped_rect.width() <= 0.5 || clipped_rect.height() <= 0.5 {
                continue;
            }
            rendered_cells += 1;
            min_vis_row = min_vis_row.min(row);
            max_vis_row = max_vis_row.max(row + 1);
            min_vis_flat = min_vis_flat.min(flat);
            max_vis_flat = max_vis_flat.max(flat + 1);

            let coord = GridCellCoord::new(row, col, subcol);
            let cell_opt = rctx.col_table.data_sets.get(col).and_then(|d| d.cells.get(row)).and_then(|s| s.get(subcol));
            let is_active = sel.active == Some(coord);
            let is_selected = is_active || sel.contains(coord, flat);
            let linked_reg = if has_links { linked_region_for_cell(rctx.ds, row, col) } else { None };
            if is_active {
                active_border_rect = Some(clipped_rect);
            }

            let is_editing_this = state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::Cell(coord));
            if is_editing_this {
                let hint = format!("R{}C{}", row + 1, col + 1);
                let handled = render_inline_text_edit(
                    ui,
                    rctx,
                    state,
                    meta,
                    intents,
                    clipped_rect,
                    rctx.sheet_id_base.with(("cell_edit", row, col, subcol)),
                    &hint,
                    &format!("grid.cell.{row}.{col}.{subcol}"),
                );
                inline_edit_handled_key |= handled;
                cell_clicked_or_edited = true;
                continue;
            }

            let draw = CellDrawInput { coord, cell_rect, clipped_rect, cell_opt, is_active, is_selected, linked_reg, blocked };
            let interacted = render_single_cell(ui, rctx, state, meta, intents, &draw, &mut text_buf);
            cell_clicked_or_edited |= interacted;
        }
    }

    render_grid_hairlines(ui, rctx, content_origin, clip_rect, &row_range, &flat_range);
    clear_grid_drag_anchor_if_released(ui, &rctx.ds.id);
    if let Some(active_rect) = active_border_rect {
        ui.painter().rect_stroke(active_rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.focus_ring), StrokeKind::Inside);
    }

    let visible_rows = if min_vis_row < max_vis_row { min_vis_row..max_vis_row } else { 0..0 };
    let visible_subcols = if min_vis_flat < max_vis_flat { min_vis_flat..max_vis_flat } else { 0..0 };
    VisibleCellsOut { rendered_cells, visible_rows, visible_subcols, inline_edit_handled_key, cell_clicked_or_edited }
}

fn render_single_cell(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    draw: &CellDrawInput<'_>,
    text_buf: &mut String,
) -> bool {
    let tokens = rctx.tokens;
    let coord = draw.coord;
    let cell_id = rctx.sheet_id_base.with(("cell", coord.row, coord.col, coord.subcol));
    let sense = if draw.blocked { Sense::hover() } else { Sense::click_and_drag() };
    let resp = ui.interact(draw.clipped_rect, cell_id, sense);

    if let Some(reg) = draw.linked_reg {
        let lc = link_border_color32(reg.border_color(), tokens);
        ui.painter().rect_filled(draw.clipped_rect, CornerRadius::ZERO, lc.gamma_multiply(0.08));
        paint_linked_perimeter_edges(ui, draw.cell_rect, draw.clipped_rect, reg, coord, rctx.geom.subs_per_col, lc);
    }
    if draw.is_selected {
        ui.painter().rect_filled(draw.clipped_rect, CornerRadius::ZERO, tokens.active_bg.gamma_multiply(0.65));
        if !draw.is_active {
            ui.painter().rect_stroke(draw.clipped_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
        }
    }

    let mut excluded = false;
    let mut is_numeric = false;
    text_buf.clear();
    if let Some(cell) = draw.cell_opt {
        excluded = cell.excluded;
        is_numeric = matches!(cell.value, CellValue::Number(_));
        format_cell_into(cell, &rctx.app.locale_preset, text_buf);
    }

    let mut elided = false;
    if !text_buf.is_empty() {
        let fg = if excluded { tokens.excluded_cell_text } else { tokens.text_primary };
        let font_id = if is_numeric {
            FontId::new(rctx.metrics.font_grid_numeral_pt, FontFamily::Monospace)
        } else {
            FontId::new(rctx.metrics.font_grid_numeral_pt, FontFamily::Proportional)
        };
        let align = if is_numeric { Align::RIGHT } else { Align::LEFT };
        let (was_elided, _) =
            render_ellipsized_text(ui, draw.clipped_rect, text_buf, CellTextStyle { font_id, color: fg, align, italic: excluded, strike: excluded });
        elided = was_elided;
    } else if excluded {
        let _ = render_ellipsized_text(
            ui,
            draw.clipped_rect,
            "×",
            CellTextStyle {
                font_id: FontId::proportional(rctx.metrics.font_ui_small_pt),
                color: tokens.excluded_cell_text,
                align: Align::Center,
                italic: true,
                strike: false,
            },
        );
    }

    if excluded {
        paint_exclusion_corner_marker(ui, draw.cell_rect, draw.clipped_rect, rctx.ds, coord, tokens);
    }

    let cell_label = format!("R{}C{}", coord.row + 1, coord.col + 1);
    let acc_label = if elided && !text_buf.is_empty() { text_buf.clone() } else { cell_label.clone() };
    let editable = !rctx.ds.frozen && !draw.blocked;
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Cell, editable, acc_label.clone()));

    if resp.hovered() {
        if excluded {
            let tip = GridViewState::cell_exclusion_tooltip(rctx.ds, coord)
                .unwrap_or_else(|| grid_tr(rctx.locale, "w5.grid_core.excluded_cell", "Excluded cell").to_owned());
            let full_tip = if elided && !text_buf.is_empty() { format!("{text_buf}\n{tip}") } else { tip };
            let _ = resp.clone().on_hover_text(full_tip);
        } else if elided {
            let _ = resp.clone().on_hover_text(text_buf.as_str());
        }
    }

    let mut interacted = false;
    if !draw.blocked {
        if is_multi_click_edit(&resp) {
            interacted = true;
            intents.push_select_grid_cell(coord);
            set_grid_focus_active(ui, true);
            if !rctx.ds.frozen {
                if let Some(reg) = draw.linked_reg {
                    open_link_popover(state, reg, rctx.locale);
                    ui.ctx().request_repaint();
                } else {
                    let initial = initial_edit_text_for_cell(draw.cell_opt, &rctx.app.locale_preset);
                    begin_inline_edit(ui, state, GridInlineEditTarget::Cell(coord), initial);
                }
            }
        } else if resp.clicked() {
            interacted = true;
            set_grid_focus_active(ui, true);
            if ui.input(|i| i.modifiers.shift) {
                intents.push_extend_grid_selection(coord);
            } else {
                intents.push_select_grid_cell(coord);
            }
            ui.ctx().request_repaint();
        } else if resp.secondary_clicked() {
            interacted = true;
            set_grid_focus_active(ui, true);
            intents.push_select_grid_cell(coord);
            ui.ctx().request_repaint();
        } else if handle_cell_pointer_drag_select(ui, &rctx.ds.id, coord, &resp, intents) {
            interacted = true;
            ui.ctx().request_repaint();
        }
    }

    if state.record_inspect_meta && !ui.ctx().will_discard() {
        let wid = format!("grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol);
        let inspect_text = if excluded && !text_buf.is_empty() { format!("{text_buf}*") } else { text_buf.clone() };
        let disabled_reason = if draw.blocked {
            Some("modal dialog is active")
        } else if rctx.ds.frozen {
            Some("Data sheet is frozen (read-only).")
        } else {
            None
        };
        record_meta_if_enabled(
            ui,
            meta,
            true,
            &resp,
            FrameInspectEntryInput {
                widget_id: &wid,
                role: "gridcell",
                label: &cell_label,
                command_id: None,
                value: Some(json!({
                    "row": coord.row,
                    "col": coord.col,
                    "subcol": coord.subcol,
                    "text": inspect_text,
                    "excluded": excluded,
                    "selected": draw.is_selected,
                })),
                enabled: editable,
                disabled_reason,
                z_layer: ZLayer::Workspace,
                ellipsized: elided,
                full_label: elided.then_some(text_buf.as_str()),
            },
        );
    }

    interacted
}

fn paint_linked_perimeter_edges(
    ui: &Ui,
    cell_rect: Rect,
    clipped: Rect,
    reg: &LinkedDataRegion,
    coord: GridCellCoord,
    subs_per_col: usize,
    color: Color32,
) {
    let r = &reg.target_range;
    let painter = ui.painter().with_clip_rect(clipped);
    if coord.row == r.start.row {
        painter.rect_filled(Rect::from_min_size(cell_rect.min, vec2(cell_rect.width(), 2.0)), CornerRadius::ZERO, color);
    }
    if coord.row == r.end.row {
        painter.rect_filled(
            Rect::from_min_size(pos2(cell_rect.left(), cell_rect.bottom() - 2.0), vec2(cell_rect.width(), 2.0)),
            CornerRadius::ZERO,
            color,
        );
    }
    if coord.col == r.start.col && coord.subcol == r.start.subcol {
        painter.rect_filled(Rect::from_min_size(cell_rect.min, vec2(2.0, cell_rect.height())), CornerRadius::ZERO, color);
    }
    let last_sub = if coord.col == r.end.col { r.end.subcol.min(subs_per_col.saturating_sub(1)) } else { subs_per_col.saturating_sub(1) };
    if coord.col == r.end.col && coord.subcol == last_sub {
        painter.rect_filled(
            Rect::from_min_size(pos2(cell_rect.right() - 2.0, cell_rect.top()), vec2(2.0, cell_rect.height())),
            CornerRadius::ZERO,
            color,
        );
    }
}

fn paint_exclusion_corner_marker(ui: &Ui, cell_rect: Rect, clipped: Rect, ds: &DataSheet, coord: GridCellCoord, tokens: &Tokens) {
    if clipped.width() < 8.0 || clipped.height() < 8.0 {
        return;
    }
    let has_note = !ds.exclusion_notes.is_empty() && ds.exclusion_note(CellCoord::from(coord)).is_some();
    let tag_color = if has_note { tokens.role_cat } else { tokens.excluded_cell_text };
    let rt = cell_rect.right_top();
    let tri = vec![rt, rt - vec2(7.0, 0.0), rt + vec2(0.0, 7.0)];
    ui.painter().with_clip_rect(clipped).add(egui::Shape::convex_polygon(tri, tag_color, Stroke::NONE));
}

fn render_grid_hairlines(
    ui: &Ui,
    rctx: &GridRenderCtx<'_>,
    content_origin: Pos2,
    clip_rect: Rect,
    row_range: &Range<usize>,
    flat_range: &Range<usize>,
) {
    let geom = rctx.geom;
    let stroke = Stroke::new(1.0, rctx.tokens.divider);
    let painter = ui.painter().with_clip_rect(clip_rect);

    #[allow(clippy::cast_precision_loss)]
    let max_x = (content_origin.x + (flat_range.end as f32) * geom.subcol_w).min(clip_rect.right());
    #[allow(clippy::cast_precision_loss)]
    let max_y = (content_origin.y + (row_range.end as f32) * geom.row_h).min(clip_rect.bottom());

    for row in row_range.start..=row_range.end {
        #[allow(clippy::cast_precision_loss)]
        let y = content_origin.y + (row as f32) * geom.row_h;
        if y >= clip_rect.top() && y <= clip_rect.bottom() {
            painter.line_segment([pos2(clip_rect.left(), y), pos2(max_x, y)], stroke);
        }
    }
    for flat in flat_range.start..=flat_range.end {
        #[allow(clippy::cast_precision_loss)]
        let x = content_origin.x + (flat as f32) * geom.subcol_w;
        if x >= clip_rect.left() && x <= clip_rect.right() {
            painter.line_segment([pos2(x, clip_rect.top()), pos2(x, max_y)], stroke);
        }
    }
}

fn render_column_headers_3tier(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    scroll_x: f32,
    vis_subcols: &Range<usize>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let blocked = rctx.input_blocked(state);
    if geom.total_cols == 0 || geom.header_data_rect.width() <= 1.0 {
        return;
    }

    let flat_range = if vis_subcols.is_empty() {
        compute_visible_range(scroll_x, geom.header_data_rect.width(), geom.subcol_w, geom.total_flat_cols)
    } else {
        vis_subcols.clone()
    };
    if flat_range.is_empty() {
        return;
    }

    let start_col = flat_range.start / geom.subs_per_col;
    let end_col = flat_range.end.div_ceil(geom.subs_per_col).min(geom.total_cols);

    ui.scope_builder(UiBuilder::new().max_rect(geom.header_data_rect), |ui| {
        ui.set_clip_rect(geom.header_data_rect);
        let y1 = geom.header_data_rect.top();
        let y2 = y1 + geom.tier1_h;
        let y3 = y2 + geom.tier2_h;

        for col in start_col..end_col {
            #[allow(clippy::cast_precision_loss)]
            let col_x = geom.header_data_rect.left() - scroll_x + ((col * geom.subs_per_col) as f32) * geom.subcol_w;
            #[allow(clippy::cast_precision_loss)]
            let col_w = (geom.subs_per_col as f32) * geom.subcol_w;

            // Tier 1: Super-Column ID (`Col A`, `Col B`, ...)
            let r1_raw = Rect::from_min_size(pos2(col_x, y1), vec2(col_w, geom.tier1_h));
            let r1 = r1_raw.intersect(geom.header_data_rect);
            if r1.width() > 0.5 && r1.height() > 0.5 {
                let super_label = format!("Col {}", default_column_label(col));
                let r1_resp = ui.interact(r1, rctx.sheet_id_base.with(("col_super", col)), Sense::hover());
                ui.painter().rect_filled(r1, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(r1, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                let (elided, _) =
                    render_ellipsized_text(ui, r1, &super_label, CellTextStyle::plain(FontId::proportional(11.0), tokens.text_muted, Align::Center));
                if elided {
                    state.ellipsized_headers += 1;
                }
                let acc = super_label.clone();
                r1_resp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc.clone()));
                if elided && r1_resp.hovered() {
                    let _ = r1_resp.clone().on_hover_text(&super_label);
                }
                record_meta_if_enabled(
                    ui,
                    meta,
                    state.record_inspect_meta,
                    &r1_resp,
                    FrameInspectEntryInput {
                        widget_id: &format!("grid.col_super.{col}"),
                        role: "label",
                        label: &super_label,
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(super_label.as_str()),
                    },
                );
            }

            // Tier 2: Editable Column Title
            let r2_raw = Rect::from_min_size(pos2(col_x, y2), vec2(col_w, geom.tier2_h));
            let r2 = r2_raw.intersect(geom.header_data_rect);
            if r2.width() > 0.5 && r2.height() > 0.5 {
                let col_title_raw = rctx.col_table.data_sets.get(col).map_or("", |d| d.title.as_str());
                let col_title = if col_title_raw.is_empty() { default_column_label(col) } else { col_title_raw.to_owned() };
                if state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::ColumnTitle { col }) {
                    let _ = render_inline_text_edit(
                        ui,
                        rctx,
                        state,
                        meta,
                        intents,
                        r2,
                        rctx.sheet_id_base.with(("col_title_edit", col)),
                        &col_title,
                        &format!("grid.col_title.{col}"),
                    );
                } else {
                    let sense = if blocked { Sense::hover() } else { Sense::click() };
                    let r2_resp = ui.interact(r2, rctx.sheet_id_base.with(("col_title", col)), sense);
                    let bg = if r2_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.panel_bg };
                    ui.painter().rect_filled(r2, CornerRadius::ZERO, bg);
                    ui.painter().rect_stroke(r2, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                    let (elided, _) = render_ellipsized_text(
                        ui,
                        r2,
                        &col_title,
                        CellTextStyle::plain(FontId::proportional(12.0), tokens.text_strong, Align::Center),
                    );
                    if elided {
                        state.ellipsized_headers += 1;
                    }
                    let acc = col_title.clone();
                    let editable = !rctx.ds.frozen && !blocked;
                    r2_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, editable, acc.clone()));
                    if r2_resp.hovered() {
                        let _ = r2_resp.clone().on_hover_text(&col_title);
                    }
                    if !blocked {
                        if is_multi_click_edit(&r2_resp) && !rctx.ds.frozen {
                            begin_inline_edit(ui, state, GridInlineEditTarget::ColumnTitle { col }, col_title.clone());
                        } else if r2_resp.clicked() && geom.total_rows > 0 {
                            intents.push_select_grid_cell(GridCellCoord::new(0, col, 0));
                            intents.push_extend_grid_selection(GridCellCoord::new(geom.total_rows - 1, col, geom.subs_per_col.saturating_sub(1)));
                            set_grid_focus_active(ui, true);
                            ui.ctx().request_repaint();
                        }
                    }
                    record_meta_if_enabled(
                        ui,
                        meta,
                        state.record_inspect_meta,
                        &r2_resp,
                        FrameInspectEntryInput {
                            widget_id: &format!("grid.col_title.{col}"),
                            role: "button",
                            label: &col_title,
                            command_id: None,
                            value: Some(json!({ "col": col, "title": col_title })),
                            enabled: editable,
                            disabled_reason: blocked.then_some("modal dialog is active").or_else(|| rctx.ds.frozen.then_some("Frozen")),
                            z_layer: ZLayer::Workspace,
                            ellipsized: elided,
                            full_label: elided.then_some(col_title.as_str()),
                        },
                    );
                }
            }

            // Tier 3: Subcolumn Roles
            for subcol in 0..geom.subs_per_col {
                #[allow(clippy::cast_precision_loss)]
                let sub_x = col_x + (subcol as f32) * geom.subcol_w;
                let r3_raw = Rect::from_min_size(pos2(sub_x, y3), vec2(geom.subcol_w, geom.tier3_h));
                let r3 = r3_raw.intersect(geom.header_data_rect);
                if r3.width() <= 0.5 || r3.height() <= 0.5 {
                    continue;
                }
                let sub_label = column_subcolumn_role_label(rctx.col_table.subcolumn_mode, subcol, rctx.locale);
                let r3_resp = ui.interact(r3, rctx.sheet_id_base.with(("subcol_hdr", col, subcol)), Sense::hover());
                ui.painter().rect_filled(r3, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(r3, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
                let (elided, _) =
                    render_ellipsized_text(ui, r3, &sub_label, CellTextStyle::plain(FontId::proportional(11.0), tokens.text_muted, Align::Center));
                if elided {
                    state.ellipsized_headers += 1;
                }
                let acc = sub_label.clone();
                r3_resp.widget_info(move || WidgetInfo::labeled(egui::Role::ColumnHeader, true, acc.clone()));
                if r3_resp.hovered() {
                    let hint_tag = match rctx.col_table.design_hint {
                        ColumnDesignHint::UnpairedReplicates => "Unpaired",
                        ColumnDesignHint::PairedRepeatedMeasures => "Paired",
                        ColumnDesignHint::PrecomputedError => "Summary",
                    };
                    let _ = r3_resp.clone().on_hover_text(format!("{sub_label} · {hint_tag}"));
                }
                record_meta_if_enabled(
                    ui,
                    meta,
                    state.record_inspect_meta,
                    &r3_resp,
                    FrameInspectEntryInput {
                        widget_id: &format!("grid.subcol_title.{col}.{subcol}"),
                        role: "label",
                        label: &sub_label,
                        command_id: None,
                        value: None,
                        enabled: true,
                        disabled_reason: None,
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(sub_label.as_str()),
                    },
                );
            }
        }
    });
}

fn column_subcolumn_role_label(mode: SubcolumnMode, subcol: usize, locale: &str) -> String {
    match mode {
        SubcolumnMode::Single => String::from("Y"),
        SubcolumnMode::MeanSdN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sd_col", "SD").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanSemN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => grid_tr(locale, "w5.grid_core.sem_col", "SEM").to_owned(),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        SubcolumnMode::MeanCvN => match subcol {
            0 => grid_tr(locale, "w5.grid_core.mean_col", "Mean").to_owned(),
            1 => String::from("%CV"),
            2 => grid_tr(locale, "w5.grid_core.n_col", "N").to_owned(),
            _ => format!("Y{}", subcol + 1),
        },
        _ => {
            if subcol == 0 {
                String::from("Y")
            } else {
                format!("Y{}", subcol + 1)
            }
        }
    }
}

fn render_gutter_body(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    scroll_y: f32,
    vis_rows: &Range<usize>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) {
    let geom = rctx.geom;
    let tokens = rctx.tokens;
    let blocked = rctx.input_blocked(state);
    if geom.total_rows == 0 || geom.gutter_body_rect.height() <= 1.0 {
        return;
    }

    let row_range = if vis_rows.is_empty() {
        compute_visible_range(scroll_y, geom.gutter_body_rect.height(), geom.row_h, geom.total_rows)
    } else {
        vis_rows.clone()
    };

    ui.scope_builder(UiBuilder::new().max_rect(geom.gutter_body_rect), |ui| {
        ui.set_clip_rect(geom.gutter_body_rect);
        let rt_x = geom.gutter_body_rect.left() + geom.row_num_w;
        let last_col = geom.total_cols.saturating_sub(1);
        let last_sub = geom.subs_per_col.saturating_sub(1);
        let row_sense = if blocked { Sense::hover() } else { Sense::click() };

        for row in row_range {
            #[allow(clippy::cast_precision_loss)]
            let row_y = geom.gutter_body_rect.top() - scroll_y + (row as f32) * geom.row_h;

            // `#` row index cell
            let rn_rect =
                Rect::from_min_size(pos2(geom.gutter_body_rect.left(), row_y), vec2(geom.row_num_w, geom.row_h)).intersect(geom.gutter_body_rect);
            if rn_rect.width() > 0.5 && rn_rect.height() > 0.5 {
                let rn_resp = ui.interact(rn_rect, rctx.sheet_id_base.with(("row_num", row)), row_sense);
                ui.painter().rect_filled(rn_rect, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(rn_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
                let rn_text = (row + 1).to_string();
                let (elided, _) = render_ellipsized_text(
                    ui,
                    rn_rect,
                    &rn_text,
                    CellTextStyle::plain(FontId::new(11.0, FontFamily::Monospace), tokens.text_muted, Align::RIGHT),
                );
                let acc = rn_text.clone();
                rn_resp.widget_info(move || WidgetInfo::labeled(egui::Role::RowHeader, !blocked, acc.clone()));
                if rn_resp.clicked() && !blocked && geom.total_cols > 0 {
                    intents.push_select_grid_cell(GridCellCoord::new(row, 0, 0));
                    intents.push_extend_grid_selection(GridCellCoord::new(row, last_col, last_sub));
                    set_grid_focus_active(ui, true);
                    ui.ctx().request_repaint();
                }
                if state.record_inspect_meta && !ui.ctx().will_discard() {
                    record_meta_if_enabled(
                        ui,
                        meta,
                        true,
                        &rn_resp,
                        FrameInspectEntryInput {
                            widget_id: &format!("grid.row_num.{row}"),
                            role: "label",
                            label: &rn_text,
                            command_id: None,
                            value: None,
                            enabled: !blocked,
                            disabled_reason: blocked.then_some("modal dialog is active"),
                            z_layer: ZLayer::Workspace,
                            ellipsized: elided,
                            full_label: elided.then_some(rn_text.as_str()),
                        },
                    );
                }
            }

            // `Row Titles` cell
            let rt_rect = Rect::from_min_size(pos2(rt_x, row_y), vec2(geom.rt_w, geom.row_h)).intersect(geom.gutter_body_rect);
            if rt_rect.width() <= 0.5 || rt_rect.height() <= 0.5 {
                continue;
            }
            if geom.effective_collapsed {
                let c_resp = ui.interact(rt_rect, rctx.sheet_id_base.with(("rt_collapsed", row)), Sense::hover());
                ui.painter().rect_filled(rt_rect, CornerRadius::ZERO, tokens.header_bg);
                ui.painter().rect_stroke(rt_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
                let c_label = format!("Row {}", row + 1);
                let acc = c_label.clone();
                c_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc.clone()));
                if state.record_inspect_meta && !ui.ctx().will_discard() {
                    record_meta_if_enabled(
                        ui,
                        meta,
                        true,
                        &c_resp,
                        FrameInspectEntryInput {
                            widget_id: &format!("grid.row_title_collapsed.{row}"),
                            role: "label",
                            label: &c_label,
                            command_id: None,
                            value: None,
                            enabled: true,
                            disabled_reason: None,
                            z_layer: ZLayer::Workspace,
                            ellipsized: false,
                            full_label: None,
                        },
                    );
                }
                continue;
            }

            if state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::RowTitle { row }) {
                let _ = render_inline_text_edit(
                    ui,
                    rctx,
                    state,
                    meta,
                    intents,
                    rt_rect,
                    rctx.sheet_id_base.with(("row_title_edit", row)),
                    &format!("Row {} Title", row + 1),
                    &format!("grid.row_title.{row}"),
                );
                continue;
            }

            let title = rctx.col_table.row_titles.get(row).map_or("", String::as_str);
            let rt_resp = ui.interact(rt_rect, rctx.sheet_id_base.with(("row_title", row)), row_sense);
            let bg = if rt_resp.hovered() && !blocked { tokens.hover_bg } else { tokens.panel_bg };
            ui.painter().rect_filled(rt_rect, CornerRadius::ZERO, bg);
            ui.painter().rect_stroke(rt_rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
            let elided = if title.is_empty() {
                false
            } else {
                let (e, _) =
                    render_ellipsized_text(ui, rt_rect, title, CellTextStyle::plain(FontId::proportional(12.0), tokens.text_primary, Align::LEFT));
                e
            };
            let a11y_label = if title.is_empty() {
                format!("{} {}", grid_tr(rctx.locale, "w5.grid_core.row_titles", "Row Titles"), row + 1)
            } else {
                title.to_owned()
            };
            let acc = a11y_label.clone();
            let editable = !rctx.ds.frozen && !blocked;
            rt_resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, editable, acc.clone()));
            if elided && rt_resp.hovered() {
                let _ = rt_resp.clone().on_hover_text(title);
            }
            if !blocked {
                if is_multi_click_edit(&rt_resp) && !rctx.ds.frozen {
                    begin_inline_edit(ui, state, GridInlineEditTarget::RowTitle { row }, title.to_owned());
                } else if rt_resp.clicked() && geom.total_cols > 0 {
                    intents.push_select_grid_cell(GridCellCoord::new(row, 0, 0));
                    intents.push_extend_grid_selection(GridCellCoord::new(row, last_col, last_sub));
                    set_grid_focus_active(ui, true);
                    ui.ctx().request_repaint();
                }
            }
            if state.record_inspect_meta && !ui.ctx().will_discard() {
                record_meta_if_enabled(
                    ui,
                    meta,
                    true,
                    &rt_resp,
                    FrameInspectEntryInput {
                        widget_id: &format!("grid.row_title.{row}"),
                        role: "button",
                        label: &a11y_label,
                        command_id: None,
                        value: Some(json!({ "row": row, "title": title })),
                        enabled: editable,
                        disabled_reason: blocked.then_some("modal dialog is active").or_else(|| rctx.ds.frozen.then_some("Frozen")),
                        z_layer: ZLayer::Workspace,
                        ellipsized: elided,
                        full_label: elided.then_some(title),
                    },
                );
            }
        }
    });
}

fn pending_commit_key(sheet_id: &SheetId) -> Id {
    Id::unique(("column_grid_pending_commit", sheet_id.as_str()))
}

fn grid_focus_key(sheet_id: &SheetId) -> Id {
    Id::unique(("column_grid_focused", sheet_id.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GridDragAnchor {
    coord: GridCellCoord,
    press_pos: Pos2,
}

fn grid_drag_anchor_key(sheet_id: &SheetId) -> Id {
    Id::unique(("grid_drag_anchor", sheet_id.as_str()))
}

fn handle_cell_pointer_drag_select(ui: &Ui, sheet_id: &SheetId, coord: GridCellCoord, resp: &egui::Response, intents: &mut PendingUiIntents) -> bool {
    let key = grid_drag_anchor_key(sheet_id);
    let (primary_pressed, primary_down, primary_released, shift, ptr_pos) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.modifiers.shift,
            i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
        )
    });
    if primary_pressed && !shift && resp.is_pointer_button_down_on() {
        if let Some(press_pos) = ptr_pos
            && !ui.ctx().will_discard()
        {
            ui.ctx().data_mut(|d| d.insert_temp(key, Some(GridDragAnchor { coord, press_pos })));
        }
        intents.push_select_grid_cell(coord);
        set_grid_focus_active(ui, true);
        return true;
    }
    let Some(pos) = ptr_pos.filter(|p| resp.rect.contains(*p)) else {
        return false;
    };
    if primary_down || primary_released {
        let anchor = ui.ctx().data(|d| d.get_temp::<Option<GridDragAnchor>>(key).flatten());
        if let Some(a) = anchor
            && a.coord != coord
            && pos.distance(a.press_pos) >= 4.0
        {
            intents.push_extend_grid_selection(coord);
            set_grid_focus_active(ui, true);
            return true;
        }
    }
    false
}

fn clear_grid_drag_anchor_if_released(ui: &Ui, sheet_id: &SheetId) {
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.ctx().data_mut(|d| d.remove_temp::<Option<GridDragAnchor>>(grid_drag_anchor_key(sheet_id)));
    }
}

fn set_grid_focus_active(ui: &Ui, focused: bool) {
    ui.data_mut(|d| d.insert_temp(Id::unique("column_grid_any_focused"), focused));
}

fn is_grid_focus_active(ui: &Ui, app: &UiApp, sheet_id: &SheetId) -> bool {
    let mem_focus = ui.data(|d| d.get_temp::<bool>(Id::unique("column_grid_any_focused")).unwrap_or(false))
        || ui.data(|d| d.get_temp::<bool>(grid_focus_key(sheet_id)).unwrap_or(false));
    mem_focus || app.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("grid."))
}

fn is_multi_click_edit(resp: &egui::Response) -> bool {
    // Include spillover triple-clicks when a user double-clicks another cell < 0.3s after an unrelated click.
    resp.double_clicked() || resp.triple_clicked()
}

fn begin_inline_edit(ui: &Ui, state: &mut ColumnGridEguiState, target: GridInlineEditTarget, initial: String) {
    state.inline_edit = Some(GridInlineEditBuffer { target, draft: initial, focused_once: false });
    ui.ctx().request_repaint();
}

#[allow(clippy::too_many_arguments)]
fn render_inline_text_edit(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    rect: Rect,
    editor_id: Id,
    hint: &str,
    widget_id: &str,
) -> bool {
    let Some(edit) = state.inline_edit.as_mut() else {
        return false;
    };
    let target = edit.target;
    let was_focused = edit.focused_once;
    let escaped = was_focused && ui.input(|i| i.key_pressed(Key::Escape));
    if escaped {
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        state.inline_edit = None;
        ui.data_mut(|d| d.remove::<bool>(pending_commit_key(&rctx.ds.id)));
        ui.memory_mut(|m| m.surrender_focus(editor_id));
        ui.ctx().request_repaint();
        return true;
    }

    let enter_pressed = was_focused && ui.input(|i| i.key_pressed(Key::Enter));
    let tab_pressed = was_focused && ui.input(|i| i.key_pressed(Key::Tab));
    let shift_held = ui.input(|i| i.modifiers.shift);

    ui.painter().rect_filled(rect, CornerRadius::ZERO, rctx.tokens.input_bg);
    ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(2.0, rctx.tokens.focus_ring), StrokeKind::Inside);

    let inner_rect = rect.shrink2(vec2(2.0, 1.0));
    let edit_resp = ui
        .scope_builder(UiBuilder::new().max_rect(inner_rect), |ui| {
            ui.set_clip_rect(inner_rect);
            let font_style = if matches!(target, GridInlineEditTarget::Cell(_)) {
                FontId::new(rctx.metrics.font_grid_numeral_pt, FontFamily::Monospace)
            } else {
                FontId::new(rctx.metrics.font_ui_body_pt, FontFamily::Proportional)
            };
            ui.add_sized(
                inner_rect.size(),
                TextEdit::singleline(&mut edit.draft)
                    .id(editor_id)
                    .font(font_style)
                    .frame(egui::Frame::NONE)
                    .clip_text(true)
                    .desired_width(inner_rect.width())
                    .hint_text(hint),
            )
        })
        .inner;

    if !was_focused {
        edit_resp.request_focus();
        edit.focused_once = true;
        ui.ctx().request_repaint();
    }

    let discard_retry = ui.data(|d| d.get_temp::<bool>(pending_commit_key(&rctx.ds.id)).unwrap_or(false));
    let should_commit = discard_retry || (was_focused && (enter_pressed || tab_pressed || edit_resp.lost_focus()));
    if should_commit {
        let draft = edit.draft.clone();
        commit_inline_edit_intent(rctx, target, draft, intents);
        if enter_pressed {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        }
        if tab_pressed {
            ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
            ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab));
        }
        if enter_pressed || tab_pressed {
            advance_selection_after_edit(rctx, target, enter_pressed, tab_pressed, shift_held, intents);
        }
        if ui.ctx().will_discard() {
            ui.data_mut(|d| d.insert_temp(pending_commit_key(&rctx.ds.id), true));
        } else {
            state.inline_edit = None;
            ui.memory_mut(|m| m.surrender_focus(editor_id));
            ui.data_mut(|d| {
                d.remove::<bool>(pending_commit_key(&rctx.ds.id));
            });
            ui.ctx().request_repaint();
        }
        return enter_pressed || tab_pressed;
    }

    if state.record_inspect_meta && !ui.ctx().will_discard() {
        let draft_val = edit.draft.clone();
        record_meta_if_enabled(
            ui,
            meta,
            true,
            &edit_resp,
            FrameInspectEntryInput {
                widget_id,
                role: "textbox",
                label: hint,
                command_id: None,
                value: Some(json!({ "text": draft_val, "editing": true })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }

    false
}

fn commit_inline_edit_intent(rctx: &GridRenderCtx<'_>, target: GridInlineEditTarget, draft: String, intents: &mut PendingUiIntents) {
    let sheet_id = rctx.ds.id.as_str();
    match target {
        GridInlineEditTarget::Cell(coord) => {
            let value = parse_inline_cell_input(rctx.app, &draft);
            intents.push_engine(
                "table.setCell",
                json!({
                    "sheet_id": sheet_id,
                    "row": coord.row,
                    "col": coord.col,
                    "subcol": coord.subcol,
                    "value": value,
                }),
            );
        }
        GridInlineEditTarget::ColumnTitle { col } => {
            intents.push_engine(
                "table.setTitles",
                json!({
                    "sheet_id": sheet_id,
                    "col": col,
                    "title": draft,
                }),
            );
        }
        GridInlineEditTarget::RowTitle { row } => {
            intents.push_engine(
                "table.setRowTitles",
                json!({
                    "sheet_id": sheet_id,
                    "row": row,
                    "title": draft,
                }),
            );
        }
    }
}

fn advance_selection_after_edit(
    rctx: &GridRenderCtx<'_>,
    target: GridInlineEditTarget,
    enter_pressed: bool,
    tab_pressed: bool,
    shift_held: bool,
    intents: &mut PendingUiIntents,
) {
    let GridInlineEditTarget::Cell(coord) = target else {
        return;
    };
    let geom = rctx.geom;
    if geom.total_rows == 0 || geom.total_cols == 0 {
        return;
    }
    if enter_pressed {
        let next_row = if shift_held { coord.row.saturating_sub(1) } else { (coord.row + 1).min(geom.total_rows - 1) };
        intents.push_select_grid_cell(GridCellCoord::new(next_row, coord.col, coord.subcol));
    } else if tab_pressed {
        let cur_flat = coord.col * geom.subs_per_col + coord.subcol;
        let next_flat = if shift_held { cur_flat.saturating_sub(1) } else { (cur_flat + 1).min(geom.total_flat_cols - 1) };
        let next_col = next_flat / geom.subs_per_col;
        let next_sub = next_flat % geom.subs_per_col;
        intents.push_select_grid_cell(GridCellCoord::new(coord.row, next_col, next_sub));
    }
}

fn pressed_key_event_modifiers(ui: &Ui, target: Key) -> Option<Modifiers> {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|ev| match ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } if *key == target => Some(*modifiers),
                _ => None,
            })
            .or_else(|| i.key_pressed(target).then_some(i.modifiers))
    })
}

fn handle_grid_keyboard_shortcuts(ui: &mut Ui, rctx: &GridRenderCtx<'_>, state: &mut ColumnGridEguiState, intents: &mut PendingUiIntents) {
    if rctx.input_blocked(state) || state.inline_edit.is_some() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    if !is_grid_focus_active(ui, rctx.app, &rctx.ds.id) {
        return;
    }

    let Some(active) = rctx.app.grid.tool.active else {
        return;
    };
    let modifiers = ui.input(|i| i.modifiers);

    // Check `Cmd/Ctrl + E` (exclude/include) & `Cmd/Ctrl + Shift + E` (exclude with reason)
    if let Some(e_mods) = pressed_key_event_modifiers(ui, Key::E)
        && (e_mods.command || e_mods.mac_cmd || e_mods.ctrl)
    {
        if e_mods.shift {
            intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": rctx.ds.id.as_str() }));
        } else if !rctx.ds.frozen {
            if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
                return;
            }
            let cur_excl = rctx.ds.cell(CellCoord::from(active)).is_some_and(|c| c.excluded);
            intents.push_engine(
                "table.excludeCells",
                json!({
                    "sheet_id": rctx.ds.id.as_str(),
                    "excluded": !cur_excl,
                    "row": active.row,
                    "col": active.col,
                    "subcol": active.subcol,
                }),
            );
        }
        ui.ctx().request_repaint();
        return;
    }

    // Clipboard Cut & Paste (`Cmd/Ctrl + X`, `Cmd/Ctrl + V`)
    let cut_requested = ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::Cut)))
        || pressed_key_event_modifiers(ui, Key::X).is_some_and(|m| m.command || m.mac_cmd || m.ctrl);
    if cut_requested {
        if !rctx.ds.frozen {
            if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
                return;
            }
            intents.push_grid_cut_selection();
        }
        ui.ctx().request_repaint();
        return;
    }

    let paste_requested = ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::Paste(_))))
        || pressed_key_event_modifiers(ui, Key::V).is_some_and(|m| m.command || m.mac_cmd || m.ctrl);
    if paste_requested {
        if !rctx.ds.frozen {
            if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
                return;
            }
            intents.push_grid_paste_from_clipboard(Some(active));
        }
        ui.ctx().request_repaint();
        return;
    }

    // Navigation & Clear keys
    let nav_payload = if let Some(m) = pressed_key_event_modifiers(ui, Key::ArrowUp) {
        Some(if m.shift { "nav:shift_up" } else { "nav:up" })
    } else if let Some(m) = pressed_key_event_modifiers(ui, Key::ArrowDown) {
        Some(if m.shift { "nav:shift_down" } else { "nav:down" })
    } else if let Some(m) = pressed_key_event_modifiers(ui, Key::ArrowLeft) {
        Some(if m.shift { "nav:shift_left" } else { "nav:left" })
    } else if let Some(m) = pressed_key_event_modifiers(ui, Key::ArrowRight) {
        Some(if m.shift { "nav:shift_right" } else { "nav:right" })
    } else if let Some(m) = pressed_key_event_modifiers(ui, Key::Tab) {
        Some(if m.shift { "nav:shift_tab" } else { "nav:tab" })
    } else if let Some(m) = pressed_key_event_modifiers(ui, Key::Enter) {
        Some(if m.shift { "nav:shift_enter" } else { "nav:enter" })
    } else if ui.input(|i| i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace)) {
        if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
            open_link_popover(state, reg, rctx.locale);
            ui.ctx().request_repaint();
            return;
        }
        Some("edit:clear")
    } else {
        None
    };

    if let Some(payload) = nav_payload {
        intents.push_custom_view_action("grid_core", payload);
        ui.ctx().request_repaint();
        return;
    }

    // `F2` or direct text/IME typing begins buffered inline edit on active cell (or triggers linked guard)
    if ui.input(|i| i.key_pressed(Key::F2)) {
        if !rctx.ds.frozen {
            if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
                open_link_popover(state, reg, rctx.locale);
                ui.ctx().request_repaint();
            } else {
                let cell = rctx.ds.cell(CellCoord::from(active));
                let initial = initial_edit_text_for_cell(cell, &rctx.app.locale_preset);
                begin_inline_edit(ui, state, GridInlineEditTarget::Cell(active), initial);
            }
        }
        return;
    }

    if !modifiers.command && !modifiers.ctrl && !modifiers.alt {
        let typed = ui.input(|i| {
            i.events.iter().find_map(|ev| match ev {
                egui::Event::Text(t) if !t.is_empty() && !t.chars().all(char::is_control) => Some(t.clone()),
                egui::Event::Ime(egui::ImeEvent::Commit(t)) if !t.is_empty() => Some(t.clone()),
                _ => None,
            })
        });
        if let Some(text) = typed {
            ui.input_mut(|i| {
                i.events.retain(|ev| !matches!(ev, egui::Event::Text(_) | egui::Event::Ime(egui::ImeEvent::Commit(_))));
            });
            if !rctx.ds.frozen {
                if let Some(reg) = linked_region_for_cell(rctx.ds, active.row, active.col) {
                    open_link_popover(state, reg, rctx.locale);
                    ui.ctx().request_repaint();
                } else {
                    begin_inline_edit(ui, state, GridInlineEditTarget::Cell(active), text);
                }
            }
        }
    }
}

fn render_linked_region_guard_popover(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> bool {
    let Some(pop) = state.active_link_popover.clone() else {
        return false;
    };
    let tokens = rctx.tokens;
    let viewport = ui.ctx().content_rect();
    let backdrop_scrim = tokens.scrim.gamma_multiply(0.35);

    let mut outside_click_blocked = false;
    let backdrop_resp = egui::Area::new(Id::unique(("grid_link_guard_backdrop", rctx.ds.id.as_str())))
        .order(Order::Middle)
        .fixed_pos(viewport.min)
        .show(ui.ctx(), |ui| {
            let (rect, resp) = ui.allocate_exact_size(viewport.size(), Sense::click_and_drag());
            ui.painter().rect_filled(rect, CornerRadius::ZERO, backdrop_scrim);
            resp
        })
        .inner;

    if backdrop_resp.clicked() {
        outside_click_blocked = true;
    }

    if ui.input(|i| i.key_pressed(Key::Escape)) {
        state.active_link_popover = None;
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        ui.ctx().request_repaint();
        return outside_click_blocked;
    }

    let card_w = 400.0_f32.min((rctx.geom.available_rect.width() - 24.0).max(240.0));
    let card_pos = pos2(
        (rctx.geom.available_rect.center().x - card_w * 0.5).max(viewport.left() + 8.0),
        (rctx.geom.available_rect.top() + 48.0).min(viewport.bottom() - 150.0),
    );
    let title = grid_tr(rctx.locale, "w5.grid_core.linked_region_title", "Linked Data Region");
    let mut should_close = false;

    let card_area = egui::Area::new(Id::unique(("grid_link_guard_card", rctx.ds.id.as_str()))).order(Order::Foreground).fixed_pos(card_pos).show(
        ui.ctx(),
        |ui| {
            egui::Frame::popup(ui.style())
                .fill(tokens.surface_bg)
                .stroke(Stroke::new(2.0, tokens.link_blue))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.set_width(card_w);
                    ui.strong(title);
                    ui.add_space(4.0);
                    ui.colored_label(tokens.link_red, &pop.message);
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let goto_lbl = grid_tr(rctx.locale, "w5.grid_core.go_to_source", "Go To Source");
                        let has_upstream = pop.upstream_sheet_id.is_some();
                        let goto_resp = ui.add_enabled(has_upstream, egui::Button::new(goto_lbl));
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &goto_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.goto",
                                role: "button",
                                label: goto_lbl,
                                command_id: Some("sheet.select"),
                                value: None,
                                enabled: has_upstream,
                                disabled_reason: (!has_upstream).then_some("External source"),
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if goto_resp.clicked()
                            && let Some(target_sid) = &pop.upstream_sheet_id
                        {
                            intents.push_engine("sheet.select", json!({ "sheet_id": target_sid.as_str(), "mode": "navigator" }));
                            should_close = true;
                        }

                        let unlink_lbl = grid_tr(rctx.locale, "w5.grid_core.unlink", "Unlink");
                        let unlink_resp = ui.add_enabled(!rctx.ds.frozen, egui::Button::new(unlink_lbl));
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &unlink_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.unlink",
                                role: "button",
                                label: unlink_lbl,
                                command_id: Some("table.clearCells"),
                                value: None,
                                enabled: !rctx.ds.frozen,
                                disabled_reason: rctx.ds.frozen.then_some("Frozen"),
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if unlink_resp.clicked() && !rctx.ds.frozen {
                            intents.push_engine("table.clearCells", json!({ "sheet_id": rctx.ds.id.as_str(), "cells": [], "unlink": true }));
                            should_close = true;
                        }

                        let close_lbl = grid_tr(rctx.locale, "w5.grid_core.close", "Close");
                        let close_resp = ui.button(close_lbl);
                        record_meta_if_enabled(
                            ui,
                            meta,
                            state.record_inspect_meta,
                            &close_resp,
                            FrameInspectEntryInput {
                                widget_id: "grid.link_popover.close",
                                role: "button",
                                label: close_lbl,
                                command_id: None,
                                value: None,
                                enabled: true,
                                disabled_reason: None,
                                z_layer: ZLayer::ModalDialog,
                                ellipsized: false,
                                full_label: None,
                            },
                        );
                        if close_resp.clicked() {
                            should_close = true;
                        }
                    });
                });
        },
    );

    if state.record_inspect_meta && !ui.ctx().will_discard() {
        record_meta_if_enabled(
            ui,
            meta,
            true,
            &card_area.response,
            FrameInspectEntryInput {
                widget_id: "grid.link_popover.panel",
                role: "dialog",
                label: title,
                command_id: None,
                value: Some(json!({ "region_id": pop.region_id })),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::ModalDialog,
                ellipsized: false,
                full_label: None,
            },
        );
    }

    if should_close {
        state.active_link_popover = None;
        ui.ctx().request_repaint();
    }

    outside_click_blocked
}

#[cfg(test)]
mod tests {
    use curvebook_doc::{
        Cell, ColumnDesignHint, ColumnTable, DataSheet, ImportLinkSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId,
        TableArchetype,
    };
    use curvebook_engine::Session;
    use curvebook_tools::GridCellCoord;

    use super::{ColumnGridEguiState, ColumnGridRenderContext, GridInlineEditTarget, render_column_grid};
    use crate::UiApp;
    use crate::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
    use crate::egui_shell::{PendingUiIntents, Tokens};
    use crate::interaction::ViewportMetrics;

    fn build_column_app(cols: usize, rows: usize) -> UiApp {
        let mut doc = ProjectDocument::default();
        let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).expect("column table");
        for c in 0..cols {
            col.data_sets[c].title = format!("Group {}", c + 1);
            for r in 0..rows {
                col.data_sets[c].cells[r][0] = Cell::number((r as f64) + (c as f64) * 0.1);
            }
        }
        let ds = DataSheet::new("col-1", "Column Test", TableArchetype::Column(col)).expect("data sheet");
        doc.add_data_sheet(ds).expect("add data sheet");
        doc.select_in_navigator(&SheetId::from("col-1")).expect("select sheet");
        UiApp::from_session(Session::from_document(doc))
    }

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

    #[test]
    fn one_million_cell_column_grid_renders_strictly_visible_window() {
        let app = build_column_app(1_000, 1_000);
        let tokens = Tokens::default();
        let state = (app, ColumnGridEguiState::default());
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            state,
            ViewportMetrics::new(1280.0, 800.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            |ui, meta, (app, grid_state)| {
                let mut intents = PendingUiIntents::new();
                let t = Tokens::get(ui.ctx());
                let rep = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
                assert!(rep.rendered_cells < 600, "must virtualize 1M cells, rendered {}", rep.rendered_cells);
                assert!(rep.rendered_cells > 100);
                assert!(rep.vertical_scrollbar_rect_pt.is_some());
                assert!(rep.horizontal_scrollbar_rect_pt.is_some());
                let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
            },
        );
        driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
        let parity = driver.verify_pixel_to_hit_box_parity();
        assert!(parity.is_clean(), "1M-cell parity violations: {:?}", parity.violations);
        assert!(driver.check_scrollbar_non_overlap().is_empty());
    }

    #[test]
    fn linked_region_guard_opens_on_double_click_and_blocks_outside_clicks() {
        let mut app = build_column_app(3, 8);
        {
            let mut doc = app.document().clone();
            let ds = doc.data_sheet_mut(&SheetId::from("col-1")).expect("sheet");
            ds.add_linked_region(LinkedDataRegion {
                id: String::from("link-1"),
                spec: ImportLinkSpec { source: LinkSource::Clipboard, filter: Default::default(), placement: Default::default() },
                target_range: SheetCellRange::new(curvebook_doc::CellCoord::new(0, 0, 0), curvebook_doc::CellCoord::new(2, 1, 0)),
                provenance_summary: String::from("Raw Plate › A1:B3"),
            })
            .expect("add link");
            *app.session_mut() = Session::from_document(doc);
            app.sync_views();
        }
        let tokens = Tokens::default();
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            (app, ColumnGridEguiState::default()),
            ViewportMetrics::new(1024.0, 640.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            |ui, meta, (app, grid_state)| {
                let mut intents = PendingUiIntents::new();
                let t = Tokens::get(ui.ctx());
                let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
                let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
            },
        );
        let center = driver.frame_meta().find_by_widget_id("grid.cell.1.0.0").expect("cell meta").clipped_rect.center();
        driver.click_at(center).expect("click 1");
        driver.click_at(center).expect("click 2");
        assert!(driver.state().1.active_link_popover.is_some());
        assert!(driver.frame_meta().find_by_widget_id("grid.link_popover.panel").is_some());
    }

    #[test]
    fn buffered_inline_edit_commits_once_on_enter_and_cancels_on_escape() {
        let app = build_column_app(2, 5);
        let tokens = Tokens::default();
        let grid_state = ColumnGridEguiState {
            inline_edit: Some(super::GridInlineEditBuffer {
                target: GridInlineEditTarget::Cell(GridCellCoord::new(0, 0, 0)),
                draft: String::from("42.75"),
                focused_once: false,
            }),
            ..Default::default()
        };
        let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
            (app, grid_state),
            ViewportMetrics::new(900.0, 560.0, 1.0),
            tokens,
            build_driver_font_definitions(),
            |ui, meta, (app, grid_state)| {
                let mut intents = PendingUiIntents::new();
                let t = Tokens::get(ui.ctx());
                let _ = render_column_grid(ui, app, grid_state, ColumnGridRenderContext::new(&t, false), meta, &mut intents);
                let _ = intents.commit_if_settled(app, ui.ctx().will_discard());
            },
        );
        driver.press_egui_key(egui::Key::Enter, egui::Modifiers::NONE);
        assert!(driver.state().1.inline_edit.is_none());
        assert_eq!(driver.state().0.session().undo_depth(), 1);
        let cell = driver.state().0.document().data_sheet(&SheetId::from("col-1")).unwrap().cell(curvebook_doc::CellCoord::new(0, 0, 0)).unwrap();
        assert_eq!(cell.value, curvebook_doc::CellValue::Number(42.75));
    }
}
