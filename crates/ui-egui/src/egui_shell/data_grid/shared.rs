//! Shared geometry, scrolling, render contexts, formatting, and pointer/focus helpers for the
//! data grid (`Column` and multi-archetype views).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;

use curvebook_doc::{Cell, CellCoord, CellValue, ColumnTable, DataSheet, LinkBorderColor, LinkedDataRegion, SheetId, TableArchetype};
use curvebook_tools::GridCellCoord;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, Response, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::{data_set_subcolumn_count, format_cell_display, parse_localized_number_input};
use crate::i18n::LocalePreset;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens};
use super::ColumnGridEguiState;

pub(super) const ROW_NUM_WIDTH_PT: f32 = 48.0;
pub(super) const ROW_TITLES_EXPANDED_WIDTH_PT: f32 = 112.0;
pub(super) const ROW_TITLES_COLLAPSED_WIDTH_PT: f32 = 34.0;
pub(super) const DEFAULT_DATA_COL_WIDTH_PT: f32 = 96.0;
pub(super) const LINK_STRIP_HEIGHT_PT: f32 = 26.0;
pub(super) const CELL_PAD_X_PT: f32 = 4.0;
pub(super) const CORNER_TRIGGER_HEIGHT_PT: f32 = 24.0;

#[derive(Debug, Clone)]
pub(super) struct GridGeometry {
    pub(super) available_rect: Rect,
    pub(super) table_rect: Rect,
    pub(super) row_num_w: f32,
    pub(super) rt_w: f32,
    pub(super) tier1_h: f32,
    pub(super) tier2_h: f32,
    pub(super) tier3_h: f32,
    pub(super) header_h: f32,
    pub(super) row_h: f32,
    pub(super) subcol_w: f32,
    pub(super) subs_per_col: usize,
    pub(super) total_rows: usize,
    pub(super) total_cols: usize,
    pub(super) total_flat_cols: usize,
    pub(super) total_data_w: f32,
    pub(super) total_data_h: f32,
    pub(super) need_h: bool,
    pub(super) need_v: bool,
    pub(super) h_reserve: f32,
    pub(super) v_reserve: f32,
    pub(super) cols_x: f32,
    pub(super) data_y: f32,
    pub(super) body_inner_w: f32,
    pub(super) body_inner_h: f32,
    pub(super) effective_collapsed: bool,
    pub(super) corner_rect: Rect,
    pub(super) header_data_rect: Rect,
    pub(super) gutter_body_rect: Rect,
    pub(super) body_outer_rect: Rect,
}

pub(super) struct GridRenderCtx<'a> {
    pub(super) app: &'a UiApp,
    pub(super) ds: &'a DataSheet,
    pub(super) col_table: &'a ColumnTable,
    pub(super) tokens: &'a Tokens,
    pub(super) metrics: LayoutMetrics,
    pub(super) geom: &'a GridGeometry,
    pub(super) sheet_id_base: Id,
    pub(super) modal_blocked: bool,
    pub(super) locale: &'a str,
}

impl<'a> GridRenderCtx<'a> {
    pub(super) const fn input_blocked(&self, state: &ColumnGridEguiState) -> bool {
        self.modal_blocked || state.active_link_popover.is_some()
    }
}

pub(super) struct SelectionFastLookup<'a> {
    pub(super) active: Option<GridCellCoord>,
    rect_bounds: Option<(usize, usize, usize, usize)>,
    discontig_slice: Option<&'a [GridCellCoord]>,
    discontig_set: Option<HashSet<GridCellCoord>>,
}

impl<'a> SelectionFastLookup<'a> {
    pub(super) fn new(app: &'a UiApp, total_rows: usize, subs_per_col: usize, total_flat: usize) -> Self {
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

    pub(super) fn contains(&self, coord: GridCellCoord, flat_col: usize) -> bool {
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

pub(super) struct BodyRenderResult {
    pub(super) inner_rect: Rect,
    pub(super) rendered_cells: usize,
    pub(super) visible_rows: Range<usize>,
    pub(super) visible_subcols: Range<usize>,
    pub(super) used_scroll_offset: Vec2,
    pub(super) new_scroll_offset: Vec2,
    pub(super) inline_edit_handled_key: bool,
    pub(super) cell_clicked_or_edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AuxInlineTarget {
    XCell { row: usize, subcol: usize },
    XTitle { subcol: usize },
    SubcolumnTitle { col: usize, subcol: usize },
    SheetTitle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ContextCellTarget {
    pub(super) is_x: bool,
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) subcol: usize,
}

#[derive(Debug, Clone)]
pub(super) struct ContextMenuState {
    pub(super) pos: Pos2,
    pub(super) row: usize,
    pub(super) cell: Option<ContextCellTarget>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct MultiGridAuxState {
    pub(super) sheet_id: Option<SheetId>,
    pub(super) aux_edit: Option<(AuxInlineTarget, String, bool)>,
    pub(super) selected_x_cell: Option<(usize, usize)>,
    pub(super) last_app_x_cell: Option<(usize, usize)>,
    pub(super) last_app_y_selection: Option<(Option<GridCellCoord>, Option<GridCellCoord>, usize)>,
    pub(super) active_role_menu_col: Option<usize>,
    pub(super) context_menu: Option<ContextMenuState>,
    pub(super) suppress_loss_commit_once: bool,
}

#[derive(Debug, Clone)]
pub(super) struct MultiGridGeom {
    pub(super) grid_outer_rect: Rect,
    pub(super) link_strip_rect: Option<Rect>,
    pub(super) h_tier1: f32,
    pub(super) h_tier2: f32,
    pub(super) h_tier3: f32,
    pub(super) row_h: f32,
    pub(super) row_num_w: f32,
    pub(super) row_title_w: f32,
    pub(super) subcol_w: f32,
    pub(super) bar_reserve: f32,
    pub(super) row_count: usize,
    pub(super) col_count: usize,
    pub(super) x_subcols: usize,
    pub(super) uniform_data_subcols: Option<usize>,
    pub(super) col_flat_offsets: Vec<usize>,
    pub(super) total_data_flat_cols: usize,
    pub(super) total_flat_cols: usize,
    pub(super) row_titles_collapsed: bool,
    pub(super) corner_rect: Rect,
    pub(super) col_header_rect: Rect,
    pub(super) row_gutter_rect: Rect,
    pub(super) body_rect: Rect,
    pub(super) body_inner_clip: Rect,
    pub(super) content_w: f32,
    pub(super) content_h: f32,
    pub(super) font_num_pt: f32,
    pub(super) font_small_pt: f32,
}

impl MultiGridGeom {
    pub(super) fn flat_data_to_coord(&self, flat_data_col: usize) -> Option<(usize, usize)> {
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

    pub(super) fn coord_to_flat_all(&self, is_x: bool, col: usize, subcol: usize) -> usize {
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

    pub(super) fn col_span_flat(&self, col: usize) -> (usize, usize) {
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

/// Immutable per-frame context shared across multi-archetype grid helpers (`R15-GUI-08`).
pub(super) struct MultiGridCtx<'a> {
    pub(super) app: &'a UiApp,
    pub(super) ds: &'a DataSheet,
    pub(super) geom: &'a MultiGridGeom,
    pub(super) tokens: &'a Tokens,
    pub(super) blocked: bool,
}

/// Mutable per-frame state & output handles threaded through multi-archetype grid helpers (`R15-GUI-08`).
pub(super) struct MultiGridFrameState<'a> {
    pub(super) state: &'a mut ColumnGridEguiState,
    pub(super) aux: &'a mut MultiGridAuxState,
    pub(super) meta: &'a mut FrameInspectMeta,
    pub(super) intents: &'a mut PendingUiIntents,
}

pub(super) fn active_column_sheet(app: &UiApp) -> Option<(&DataSheet, &ColumnTable)> {
    let doc = app.document();
    let sid = doc.active_sheet_id.as_ref()?;
    let ds = doc.data_sheet(sid)?;
    let TableArchetype::Column(col_table) = &ds.archetype else {
        return None;
    };
    Some((ds, col_table))
}

pub(super) fn ensure_solid_scrollbar_style(ui: &mut Ui, metrics: LayoutMetrics) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    *scroll = egui::style::ScrollStyle::solid();
    scroll.floating = false;
    scroll.bar_width = metrics.scroll_bar_thickness.max(10.0);
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
}

pub(super) fn compute_grid_geometry(
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

pub(super) fn compute_multi_geom(app: &UiApp, ds: &DataSheet, state: &ColumnGridEguiState, metrics: LayoutMetrics, available: Rect) -> MultiGridGeom {
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
    let col_header_rect = Rect::from_min_max(pos2(body_x0, grid_outer_rect.min.y), pos2((grid_outer_rect.max.x - bar_reserve).max(body_x0), body_y0));
    let row_gutter_rect = Rect::from_min_max(pos2(grid_outer_rect.min.x, body_y0), pos2(body_x0, (grid_outer_rect.max.y - bar_reserve).max(body_y0)));
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

pub(super) fn clamp_scroll_offset(state: &mut ColumnGridEguiState, geom: &GridGeometry) {
    let max_x = (geom.total_data_w - geom.body_inner_w).max(0.0);
    let max_y = (geom.total_data_h - geom.body_inner_h).max(0.0);
    state.scroll_offset_pt.x = if state.scroll_offset_pt.x.is_finite() { state.scroll_offset_pt.x.clamp(0.0, max_x) } else { 0.0 };
    state.scroll_offset_pt.y = if state.scroll_offset_pt.y.is_finite() { state.scroll_offset_pt.y.clamp(0.0, max_y) } else { 0.0 };
}

pub(super) fn clamp_multi_scroll(state: &mut ColumnGridEguiState, geom: &MultiGridGeom) {
    let max_x = (geom.content_w - geom.body_inner_clip.width()).max(0.0);
    let max_y = (geom.content_h - geom.body_inner_clip.height()).max(0.0);
    state.scroll_offset_pt.x = state.scroll_offset_pt.x.clamp(0.0, max_x);
    state.scroll_offset_pt.y = state.scroll_offset_pt.y.clamp(0.0, max_y);
}

pub(super) fn forward_frozen_wheel(ui: &mut Ui, geom: &MultiGridGeom, state: &mut ColumnGridEguiState) {
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

pub(super) fn compute_multi_scrollbar_rects(geom: &MultiGridGeom) -> (Option<Rect>, Option<Rect>) {
    let b = geom.body_rect;
    if b.width() <= geom.bar_reserve || b.height() <= geom.bar_reserve {
        return (None, None);
    }
    let v_sb = Rect::from_min_max(pos2(b.max.x - geom.bar_reserve, b.min.y), pos2(b.max.x, b.max.y - geom.bar_reserve));
    let h_sb = Rect::from_min_max(pos2(b.min.x, b.max.y - geom.bar_reserve), pos2(b.max.x - geom.bar_reserve, b.max.y));
    (v_sb.is_positive().then_some(v_sb), h_sb.is_positive().then_some(h_sb))
}

pub(super) fn archetype_x_subcol_count(arch: &TableArchetype) -> usize {
    match arch {
        TableArchetype::Xy(xy) => xy.x_mode.subcolumn_count().max(1),
        TableArchetype::Survival(_) => 1,
        _ => 0,
    }
}

pub(super) fn archetype_badge(arch: &TableArchetype) -> &'static str {
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

pub(super) fn any_pointer_click_in(ui: &Ui, rect: Rect) -> bool {
    ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if rect.contains(*pos))))
}

const fn blend_row_channel(a: u8, b: u8) -> u8 {
    ((a as u16 * 3 + b as u16) / 4) as u8
}

pub(super) const fn alt_row_fill(surface: Color32, panel: Color32) -> Color32 {
    Color32::from_rgb(blend_row_channel(surface.r(), panel.r()), blend_row_channel(surface.g(), panel.g()), blend_row_channel(surface.b(), panel.b()))
}

pub(super) const fn link_border_color32(color: Option<LinkBorderColor>, tokens: &Tokens) -> Color32 {
    match color {
        Some(LinkBorderColor::Blue) | None => tokens.link_blue,
        Some(LinkBorderColor::Green) => tokens.link_green,
        Some(LinkBorderColor::Red) => tokens.link_red,
        Some(LinkBorderColor::Charcoal) => tokens.text_muted,
    }
}

pub(super) fn compute_scrollbar_rects(ui: &Ui, geom: &GridGeometry, inner_rect: Rect) -> (Option<Rect>, Option<Rect>) {
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

pub(super) fn paint_scrollbar_corner_cap(ui: &Ui, rctx: &GridRenderCtx<'_>) {
    let geom = rctx.geom;
    if geom.need_v && geom.need_h {
        let cap = Rect::from_min_max(pos2(geom.table_rect.right() - geom.v_reserve, geom.table_rect.bottom() - geom.h_reserve), geom.table_rect.max);
        ui.painter().rect_filled(cap, CornerRadius::ZERO, rctx.tokens.header_bg);
        ui.painter().rect_stroke(cap, CornerRadius::ZERO, Stroke::new(1.0, rctx.tokens.divider), StrokeKind::Inside);
    }
}

pub(super) fn forward_gutter_header_wheel(ui: &mut Ui, rctx: &GridRenderCtx<'_>, state: &mut ColumnGridEguiState) {
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

pub(super) fn compute_visible_range(start_px: f32, extent_px: f32, unit_px: f32, total: usize) -> Range<usize> {
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

pub(super) struct CellTextStyle {
    pub(super) font_id: FontId,
    pub(super) color: Color32,
    pub(super) align: Align,
    pub(super) italic: bool,
    pub(super) strike: bool,
}

impl CellTextStyle {
    pub(super) fn plain(font_id: FontId, color: Color32, align: Align) -> Self {
        Self { font_id, color, align, italic: false, strike: false }
    }
}

pub(super) fn multi_text_style(pt: f32, mono: bool, color: Color32, align: Align, italic: bool, strike: bool) -> CellTextStyle {
    let font_id = if mono { egui::FontId::new(pt, egui::FontFamily::Monospace) } else { egui::FontId::proportional(pt) };
    CellTextStyle { font_id, color, align, italic, strike }
}

pub(super) fn render_ellipsized_text(ui: &Ui, clip_rect: Rect, text: &str, style: CellTextStyle) -> (bool, Rect) {
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

pub(super) fn format_cell_into(cell: &Cell, preset: &LocalePreset, out: &mut String) {
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

pub(super) fn initial_edit_text_for_cell(cell: Option<&Cell>, preset: &LocalePreset) -> String {
    let Some(c) = cell else {
        return String::new();
    };
    match &c.value {
        CellValue::Text(s) if s.starts_with('=') => s.clone(),
        _ => format_cell_display(c, preset),
    }
}

pub(super) fn parse_inline_cell_input(app: &UiApp, draft: &str) -> serde_json::Value {
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

pub(super) fn record_meta_if_enabled(ui: &Ui, meta: &mut FrameInspectMeta, enabled: bool, resp: &Response, input: FrameInspectEntryInput<'_>) {
    if !enabled {
        return;
    }
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
        return;
    }
    meta.record_response(ui, resp, input);
}

pub(super) fn paint_linked_perimeter_edges(
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

pub(super) fn paint_exclusion_corner_marker(ui: &Ui, cell_rect: Rect, clipped: Rect, ds: &DataSheet, coord: GridCellCoord, tokens: &Tokens) {
    if clipped.width() < 8.0 || clipped.height() < 8.0 {
        return;
    }
    let has_note = !ds.exclusion_notes.is_empty() && ds.exclusion_note(CellCoord::from(coord)).is_some();
    let tag_color = if has_note { tokens.role_cat } else { tokens.excluded_cell_text };
    let rt = cell_rect.right_top();
    let tri = vec![rt, rt - vec2(7.0, 0.0), rt + vec2(0.0, 7.0)];
    ui.painter().with_clip_rect(clipped).add(egui::Shape::convex_polygon(tri, tag_color, Stroke::NONE));
}

pub(super) fn pending_commit_key(sheet_id: &SheetId) -> Id {
    Id::unique(("column_grid_pending_commit", sheet_id.as_str()))
}

pub(super) fn grid_focus_key(sheet_id: &SheetId) -> Id {
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

pub(super) fn handle_cell_pointer_drag_select(
    ui: &Ui,
    sheet_id: &SheetId,
    coord: GridCellCoord,
    resp: &egui::Response,
    intents: &mut PendingUiIntents,
) -> bool {
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

pub(super) fn clear_grid_drag_anchor_if_released(ui: &Ui, sheet_id: &SheetId) {
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.ctx().data_mut(|d| d.remove_temp::<Option<GridDragAnchor>>(grid_drag_anchor_key(sheet_id)));
    }
}

pub(super) fn set_grid_focus_active(ui: &Ui, focused: bool) {
    ui.data_mut(|d| d.insert_temp(Id::unique("column_grid_any_focused"), focused));
}

pub(super) fn is_grid_focus_active(ui: &Ui, app: &UiApp, sheet_id: &SheetId) -> bool {
    let mem_focus = ui.data(|d| d.get_temp::<bool>(Id::unique("column_grid_any_focused")).unwrap_or(false))
        || ui.data(|d| d.get_temp::<bool>(grid_focus_key(sheet_id)).unwrap_or(false));
    mem_focus || app.focus_widget_id.as_deref().is_some_and(|id| id.starts_with("grid."))
}

pub(super) fn is_multi_click_edit(resp: &egui::Response) -> bool {
    // Include spillover triple-clicks when a user double-clicks another cell < 0.3s after an unrelated click.
    resp.double_clicked() || resp.triple_clicked()
}

pub(super) fn pressed_key_event_modifiers(ui: &Ui, target: Key) -> Option<Modifiers> {
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
