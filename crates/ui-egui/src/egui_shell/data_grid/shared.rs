//! Shared geometry, scrolling, render contexts, formatting, and pointer/focus helpers for the
//! unified virtualized data grid across all 8 table archetypes.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::ops::Range;

use curvebook_doc::{
    Cell, CellCoord, CellValue, DataSheet, LinkBorderColor, LinkedDataRegion, ProjectDocument, ResultStatus, SheetId, TableArchetype,
};
use curvebook_tools::GridCellCoord;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Color32, CornerRadius, FontId, Id, Key, Modifiers, Pos2, Rect, Response, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::{data_set_subcolumn_count, format_cell_display, parse_localized_number_input};
use crate::i18n::LocalePreset;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, LayoutMetrics, PendingUiIntents, Tokens, UiIntent};
use super::{ColumnGridEguiState, DataGridScrollTarget};

pub(super) const ROW_NUM_WIDTH_PT: f32 = 48.0;
pub(super) const ROW_TITLES_EXPANDED_WIDTH_PT: f32 = 112.0;
pub(super) const ROW_TITLES_COLLAPSED_WIDTH_PT: f32 = 34.0;
pub(super) const DEFAULT_DATA_COL_WIDTH_PT: f32 = 96.0;
pub(super) const LINK_STRIP_HEIGHT_PT: f32 = 26.0;
pub(super) const CELL_PAD_X_PT: f32 = 4.0;

pub(super) struct SelectionFastLookup<'a> {
    pub(super) active: Option<GridCellCoord>,
    rect_bounds: Option<(usize, usize, usize, usize)>,
    discontig_slice: Option<&'a [GridCellCoord]>,
    discontig_set: Option<HashSet<GridCellCoord>>,
}

impl<'a> SelectionFastLookup<'a> {
    pub(super) fn new_with_preview(
        app: &'a UiApp,
        sheet_id: &SheetId,
        geom: &MultiGridGeom,
        preview_range: Option<(GridCellCoord, GridCellCoord)>,
    ) -> Self {
        if let Some((anchor, active)) = preview_range {
            let rect_bounds = if geom.row_count == 0 || geom.total_data_flat_cols == 0 {
                None
            } else {
                let r0 = anchor.row.min(active.row).min(geom.row_count - 1);
                let r1 = anchor.row.max(active.row).min(geom.row_count - 1);
                let (min_c, max_c) = if (anchor.col, anchor.subcol) <= (active.col, active.subcol) { (anchor, active) } else { (active, anchor) };
                let f0 = geom.coord_to_flat_all(false, min_c.col, min_c.subcol).saturating_sub(geom.x_subcols);
                let f1 = geom.coord_to_flat_all(false, max_c.col, max_c.subcol).saturating_sub(geom.x_subcols);
                if r0 <= r1 && f0.min(f1) < geom.total_data_flat_cols {
                    Some((r0, r1, f0.min(f1), f0.max(f1).min(geom.total_data_flat_cols - 1)))
                } else {
                    None
                }
            };
            return Self { active: Some(active), rect_bounds, discontig_slice: None, discontig_set: None };
        }
        if !app.grid.selection_belongs_to(sheet_id) {
            return Self { active: None, rect_bounds: None, discontig_slice: None, discontig_set: None };
        }
        let active = app.grid.tool.active;
        let has_discontig =
            app.grid.selected_coords.len() > 1 || (app.grid.selected_coords.len() == 1 && app.grid.tool.anchor != app.grid.tool.active);
        if has_discontig {
            if app.grid.selected_coords.len() > 8 {
                let set = app.grid.selected_coords.iter().copied().collect();
                return Self { active, rect_bounds: None, discontig_slice: None, discontig_set: Some(set) };
            }
            return Self { active, rect_bounds: None, discontig_slice: Some(&app.grid.selected_coords), discontig_set: None };
        }
        let rect_bounds = if geom.row_count == 0 || geom.total_data_flat_cols == 0 {
            None
        } else if let Some((min_c, max_c)) = app.grid.tool.selection_bounds() {
            let r0 = min_c.row;
            let r1 = max_c.row.min(geom.row_count.saturating_sub(1));
            let f0 = geom.coord_to_flat_all(false, min_c.col, min_c.subcol).saturating_sub(geom.x_subcols);
            let f1 = geom.coord_to_flat_all(false, max_c.col, max_c.subcol).saturating_sub(geom.x_subcols);
            if r0 <= r1 && f0 < geom.total_data_flat_cols { Some((r0, r1, f0.min(f1), f0.max(f1).min(geom.total_data_flat_cols - 1))) } else { None }
        } else if app.grid.selected_coords.len() == 1 {
            let c = app.grid.selected_coords[0];
            let f = geom.coord_to_flat_all(false, c.col, c.subcol).saturating_sub(geom.x_subcols);
            (c.row < geom.row_count && f < geom.total_data_flat_cols).then_some((c.row, c.row, f, f))
        } else {
            None
        };
        Self { active, rect_bounds, discontig_slice: None, discontig_set: None }
    }

    pub(super) fn contains(&self, coord: GridCellCoord, flat_data_col: usize) -> bool {
        if let Some(set) = &self.discontig_set {
            return set.contains(&coord);
        }
        if let Some(slice) = self.discontig_slice {
            return slice.contains(&coord);
        }
        if let Some((r0, r1, f0, f1)) = self.rect_bounds {
            return (r0..=r1).contains(&coord.row) && (f0..=f1).contains(&flat_data_col);
        }
        false
    }

    pub(super) fn contains_flat_cell(&self, geom: &MultiGridGeom, row: usize, flat_data_col: usize) -> bool {
        if let Some((r0, r1, f0, f1)) = self.rect_bounds {
            return (r0..=r1).contains(&row) && (f0..=f1).contains(&flat_data_col);
        }
        geom.flat_data_to_coord(flat_data_col).is_some_and(|(col, subcol)| self.contains(GridCellCoord::new(row, col, subcol), flat_data_col))
    }

    pub(super) fn contiguous_rect(&self) -> Option<(usize, usize, usize, usize)> {
        self.rect_bounds
    }

    pub(super) fn has_discontiguous(&self) -> bool {
        self.discontig_slice.is_some() || self.discontig_set.is_some()
    }

    pub(super) fn contains_row(&self, row: usize) -> bool {
        if let Some((r0, r1, _, _)) = self.rect_bounds {
            return (r0..=r1).contains(&row);
        }
        if let Some(set) = &self.discontig_set {
            return set.iter().any(|c| c.row == row);
        }
        if let Some(slice) = self.discontig_slice {
            return slice.iter().any(|c| c.row == row);
        }
        false
    }

    pub(super) fn contains_full_flat_col(&self, geom: &MultiGridGeom, flat_data_col: usize) -> bool {
        self.rect_bounds
            .is_some_and(|(r0, r1, f0, f1)| geom.row_count > 0 && r0 == 0 && r1 + 1 >= geom.row_count && (f0..=f1).contains(&flat_data_col))
    }

    pub(super) fn contains_flat_col(&self, geom: &MultiGridGeom, flat_data_col: usize) -> bool {
        if let Some((_, _, f0, f1)) = self.rect_bounds {
            return (f0..=f1).contains(&flat_data_col);
        }
        geom.flat_data_to_coord(flat_data_col).is_some_and(|(col, subcol)| {
            self.discontig_set.as_ref().is_some_and(|s| s.iter().any(|c| c.col == col && c.subcol == subcol))
                || self.discontig_slice.is_some_and(|s| s.iter().any(|c| c.col == col && c.subcol == subcol))
        })
    }

    pub(super) fn contains_col(&self, geom: &MultiGridGeom, col: usize) -> bool {
        let (start_all, span_subs) = geom.col_span_flat(col);
        let col_f0 = start_all.saturating_sub(geom.x_subcols);
        let col_f1 = col_f0 + span_subs.saturating_sub(1);
        if let Some((_, _, f0, f1)) = self.rect_bounds {
            return f0 <= col_f1 && col_f0 <= f1;
        }
        if let Some(set) = &self.discontig_set {
            return set.iter().any(|c| c.col == col);
        }
        if let Some(slice) = self.discontig_slice {
            return slice.iter().any(|c| c.col == col);
        }
        false
    }

    pub(super) fn contains_full_col(&self, geom: &MultiGridGeom, col: usize) -> bool {
        if geom.row_count == 0 {
            return false;
        }
        let (start_all, span_subs) = geom.col_span_flat(col);
        let col_f0 = start_all.saturating_sub(geom.x_subcols);
        let col_f1 = col_f0 + span_subs.saturating_sub(1);
        self.rect_bounds.is_some_and(|(r0, r1, f0, f1)| r0 == 0 && r1 + 1 >= geom.row_count && f0 <= col_f0 && col_f1 <= f1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AuxInlineTarget {
    XCell { row: usize, subcol: usize },
    XTitle { subcol: usize },
    SubcolumnTitle { col: usize, subcol: usize },
    SheetTitle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::egui_shell) struct ContextCellTarget {
    pub(super) is_x: bool,
    pub(super) row: usize,
    pub(super) col: usize,
    pub(super) subcol: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::egui_shell) enum ColHeaderSortTarget {
    X { subcol: Option<usize> },
    RowTitle,
    Column { col: usize, subcol: Option<usize> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::egui_shell) enum GridContextMenuKind {
    RowCell { row: usize, cell: Option<ContextCellTarget> },
    ColumnHeader { target: ColHeaderSortTarget, header_label: String },
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::egui_shell) struct ContextMenuState {
    pub(super) pos: Pos2,
    pub(super) kind: GridContextMenuKind,
}

pub(super) fn open_col_header_context_menu(fstate: &mut MultiGridFrameState<'_>, pos: Pos2, target: ColHeaderSortTarget, header_label: String) {
    fstate.aux.active_role_menu_col = None;
    fstate.state.context_menu = Some(ContextMenuState { pos, kind: GridContextMenuKind::ColumnHeader { target, header_label } });
}

#[derive(Debug, Clone, Default)]
pub(super) struct MultiGridAuxState {
    pub(super) sheet_id: Option<SheetId>,
    pub(super) aux_edit: Option<(AuxInlineTarget, String, bool)>,
    pub(super) selected_x_cell: Option<(usize, usize)>,
    pub(super) selected_x_anchor: Option<(usize, usize)>,
    pub(super) last_app_x_cell: Option<(usize, usize)>,
    pub(super) last_app_y_selection: Option<(Option<GridCellCoord>, Option<GridCellCoord>, usize)>,
    pub(super) active_role_menu_col: Option<usize>,
    pub(super) suppress_loss_commit_once: bool,
}

impl MultiGridAuxState {
    pub(super) fn set_single_x(&mut self, cell: (usize, usize)) {
        self.selected_x_cell = Some(cell);
        self.selected_x_anchor = Some(cell);
    }

    pub(super) fn extend_x_selection(&mut self, cell: (usize, usize)) {
        if self.selected_x_anchor.is_none() {
            self.selected_x_anchor = self.selected_x_cell.or(Some(cell));
        }
        self.selected_x_cell = Some(cell);
    }

    pub(super) fn clear_x_selection(&mut self) {
        self.selected_x_cell = None;
        self.selected_x_anchor = None;
    }

    pub(super) fn selected_x_bounds(&self) -> Option<(usize, usize, usize, usize)> {
        let cur = self.selected_x_cell?;
        let anchor = self.selected_x_anchor.unwrap_or(cur);
        Some((anchor.0.min(cur.0), anchor.0.max(cur.0), anchor.1.min(cur.1), anchor.1.max(cur.1)))
    }

    pub(super) fn x_contains(&self, row: usize, subcol: usize) -> bool {
        self.selected_x_bounds().is_some_and(|(r0, r1, s0, s1)| (r0..=r1).contains(&row) && (s0..=s1).contains(&subcol))
    }

    pub(super) fn x_contains_subcol(&self, subcol: usize) -> bool {
        self.selected_x_bounds().is_some_and(|(_, _, s0, s1)| (s0..=s1).contains(&subcol))
    }

    pub(super) fn x_full_subcol_selected(&self, geom: &MultiGridGeom, subcol: usize) -> bool {
        geom.row_count > 0
            && self.selected_x_bounds().is_some_and(|(r0, r1, s0, s1)| r0 == 0 && r1 + 1 >= geom.row_count && (s0..=s1).contains(&subcol))
    }
}

#[derive(Debug, Clone)]
pub(super) struct MultiGridGeom {
    pub(super) available_rect: Rect,
    pub(super) grid_outer_rect: Rect,
    pub(super) link_strip_rect: Option<Rect>,
    pub(super) h_tier1: f32,
    pub(super) h_tier2: f32,
    pub(super) h_tier3: f32,
    pub(super) row_h: f32,
    pub(super) row_num_w: f32,
    pub(super) row_title_w: f32,
    pub(super) subcol_w: f32,
    pub(super) need_h: bool,
    pub(super) need_v: bool,
    pub(super) h_reserve: f32,
    pub(super) v_reserve: f32,
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
            let s = subs.max(1);
            col.saturating_mul(s).saturating_add(subcol.min(s - 1))
        } else {
            let start = self.col_flat_offsets.get(col).copied().unwrap_or(0);
            let next = self.col_flat_offsets.get(col + 1).copied().unwrap_or(self.total_data_flat_cols);
            let span = next.saturating_sub(start).max(1);
            start.saturating_add(subcol.min(span - 1))
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

/// Immutable per-frame context shared across unified grid helpers (`R15-GUI-08`).
pub(super) struct MultiGridCtx<'a> {
    pub(super) app: &'a UiApp,
    pub(super) ds: &'a DataSheet,
    pub(super) geom: &'a MultiGridGeom,
    pub(super) tokens: &'a Tokens,
    pub(super) metrics: LayoutMetrics,
    pub(super) blocked: bool,
    pub(super) pane_focused: bool,
    pub(super) pane_salt: &'static str,
    pub(super) locale: &'a str,
}

/// Mutable per-frame state & output handles threaded through unified grid helpers (`R15-GUI-08`).
pub(super) struct MultiGridFrameState<'a> {
    pub(super) state: &'a mut ColumnGridEguiState,
    pub(super) aux: &'a mut MultiGridAuxState,
    pub(super) meta: &'a mut FrameInspectMeta,
    pub(super) intents: &'a mut PendingUiIntents,
}

pub(super) fn ensure_solid_scrollbar_style(ui: &mut Ui, metrics: LayoutMetrics) {
    let scroll = &mut ui.style_mut().spacing.scroll;
    *scroll = egui::style::ScrollStyle::solid();
    scroll.floating = false;
    scroll.bar_width = metrics.scroll_bar_thickness.max(10.0);
    scroll.bar_inner_margin = 2.0;
    scroll.bar_outer_margin = 2.0;
}

pub(super) fn compute_multi_geom(
    app: &UiApp,
    ds: &DataSheet,
    state: &ColumnGridEguiState,
    metrics: LayoutMetrics,
    scroll_allocated_w: f32,
    available: Rect,
) -> MultiGridGeom {
    let link_h = if !ds.linked_regions.is_empty() && available.height() > 110.0 { LINK_STRIP_HEIGHT_PT } else { 0.0 };
    let link_strip_rect = (link_h > 0.0).then(|| Rect::from_min_max(available.min, pos2(available.max.x, available.min.y + link_h)));
    let grid_outer_rect = Rect::from_min_max(pos2(available.min.x, available.min.y + link_h), available.max);

    let row_titles_collapsed = app.grid.row_titles_collapsed_override.unwrap_or(state.row_titles_collapsed);
    let [h_tier1, h_tier2, h_tier3] = metrics.grid_header_tier_heights;
    let header_h = h_tier1 + h_tier2 + h_tier3;
    let row_h = metrics.grid_row_h.max(24.0);
    let row_num_w = ROW_NUM_WIDTH_PT.min((grid_outer_rect.width() * 0.25).floor()).max(24.0);
    let desired_rt_w = if row_titles_collapsed { ROW_TITLES_COLLAPSED_WIDTH_PT } else { ROW_TITLES_EXPANDED_WIDTH_PT };
    let max_rt_w = (grid_outer_rect.width() - row_num_w - 32.0).max(24.0);
    let row_title_w = desired_rt_w.min(max_rt_w);
    let gutter_w = row_num_w + row_title_w;
    let subcol_w = DEFAULT_DATA_COL_WIDTH_PT;

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

    #[allow(clippy::cast_precision_loss)]
    let content_w = (total_flat_cols as f32) * subcol_w;
    #[allow(clippy::cast_precision_loss)]
    let content_h = (row_count as f32) * row_h;

    let avail_cols0 = (grid_outer_rect.width() - gutter_w).max(0.0);
    let avail_body0 = (grid_outer_rect.height() - header_h).max(0.0);
    let mut need_v = content_h > avail_body0 + 0.5;
    let mut need_h = content_w > avail_cols0 + 0.5;
    for _ in 0..2 {
        let vr = if need_v { scroll_allocated_w } else { 0.0 };
        let hr = if need_h { scroll_allocated_w } else { 0.0 };
        need_v = content_h > (avail_body0 - hr).max(0.0) + 0.5;
        need_h = content_w > (avail_cols0 - vr).max(0.0) + 0.5;
    }
    let v_reserve = if need_v { scroll_allocated_w } else { 0.0 };
    let h_reserve = if need_h { scroll_allocated_w } else { 0.0 };

    let body_x0 = (grid_outer_rect.min.x + gutter_w).min(grid_outer_rect.max.x);
    let body_y0 = (grid_outer_rect.min.y + header_h).min(grid_outer_rect.max.y);
    let corner_rect = Rect::from_min_max(grid_outer_rect.min, pos2(body_x0, body_y0));
    let col_header_rect = Rect::from_min_max(pos2(body_x0, grid_outer_rect.min.y), pos2((grid_outer_rect.max.x - v_reserve).max(body_x0), body_y0));
    let row_gutter_rect = Rect::from_min_max(pos2(grid_outer_rect.min.x, body_y0), pos2(body_x0, (grid_outer_rect.max.y - h_reserve).max(body_y0)));
    let body_rect = Rect::from_min_max(pos2(body_x0, body_y0), grid_outer_rect.max);
    let body_inner_clip = Rect::from_min_max(
        body_rect.min,
        pos2((body_rect.max.x - v_reserve).max(body_rect.min.x), (body_rect.max.y - h_reserve).max(body_rect.min.y)),
    );

    MultiGridGeom {
        available_rect: available,
        grid_outer_rect,
        link_strip_rect,
        h_tier1,
        h_tier2,
        h_tier3,
        row_h,
        row_num_w,
        row_title_w,
        subcol_w,
        need_h,
        need_v,
        h_reserve,
        v_reserve,
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

pub(super) fn clamp_multi_scroll(state: &mut ColumnGridEguiState, geom: &MultiGridGeom) {
    let max_x = (geom.content_w - geom.body_inner_clip.width()).max(0.0);
    let max_y = (geom.content_h - geom.body_inner_clip.height()).max(0.0);
    state.scroll_offset_pt.x = if state.scroll_offset_pt.x.is_finite() { state.scroll_offset_pt.x.clamp(0.0, max_x) } else { 0.0 };
    state.scroll_offset_pt.y = if state.scroll_offset_pt.y.is_finite() { state.scroll_offset_pt.y.clamp(0.0, max_y) } else { 0.0 };
}

#[derive(Debug, Clone, Copy)]
pub(in crate::egui_shell) struct ScrollCellTarget {
    pub is_x: bool,
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

pub(in crate::egui_shell) fn scroll_cell_into_view(
    app: &UiApp,
    ds: &DataSheet,
    state: &mut ColumnGridEguiState,
    metrics: LayoutMetrics,
    viewport_size_pt: (f32, f32),
    target: ScrollCellTarget,
) -> bool {
    let ScrollCellTarget { is_x, row, col, subcol } = target;
    let (viewport_w_pt, viewport_h_pt) = viewport_size_pt;
    if row >= ds.archetype.row_count() {
        return false;
    }
    if is_x {
        if subcol >= archetype_x_subcol_count(&ds.archetype) {
            return false;
        }
    } else if col >= ds.archetype.column_count() || subcol >= data_set_subcolumn_count(&ds.archetype, col) {
        return false;
    }
    let est_w = (viewport_w_pt - 500.0).max(240.0);
    let est_h = (viewport_h_pt - 140.0).max(200.0);
    let avail = if state.last_available_rect.is_positive() { state.last_available_rect } else { Rect::from_min_size(Pos2::ZERO, vec2(est_w, est_h)) };
    let geom = compute_multi_geom(app, ds, state, metrics, metrics.scroll_bar_thickness, avail);
    let flat_all = geom.coord_to_flat_all(is_x, col, subcol);
    #[allow(clippy::cast_precision_loss)]
    let target_x = (flat_all as f32) * geom.subcol_w - geom.body_inner_clip.width() * 0.4;
    #[allow(clippy::cast_precision_loss)]
    let target_y = (row as f32) * geom.row_h - geom.body_inner_clip.height() * 0.4;
    state.scroll_offset_pt.x = target_x.max(0.0);
    state.scroll_offset_pt.y = target_y.max(0.0);
    clamp_multi_scroll(state, &geom);
    true
}

pub(super) fn apply_pending_scroll_target(state: &mut ColumnGridEguiState, geom: &MultiGridGeom) {
    let Some(target) = state.pending_scroll_target.take() else {
        return;
    };
    let vis_w = geom.body_inner_clip.width().max(geom.subcol_w);
    let vis_h = geom.body_inner_clip.height().max(geom.row_h);
    match target {
        DataGridScrollTarget::Cell(coord) => {
            if coord.col == crate::egui_shell::sheet_find_bar::X_OR_TIME_COL_SENTINEL {
                state.scroll_offset_pt.x = 0.0;
            } else {
                let flat = geom.coord_to_flat_all(false, coord.col, coord.subcol);
                #[allow(clippy::cast_precision_loss)]
                let x0 = (flat as f32) * geom.subcol_w;
                let x1 = x0 + geom.subcol_w;
                if x0 < state.scroll_offset_pt.x || x1 > state.scroll_offset_pt.x + vis_w {
                    state.scroll_offset_pt.x = (x0 - vis_w * 0.35).max(0.0);
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let y0 = (coord.row as f32) * geom.row_h;
            let y1 = y0 + geom.row_h;
            if y0 < state.scroll_offset_pt.y || y1 > state.scroll_offset_pt.y + vis_h {
                state.scroll_offset_pt.y = (y0 - vis_h * 0.35).max(0.0);
            }
        }
        DataGridScrollTarget::ColumnTitle { col } => {
            if col == crate::egui_shell::sheet_find_bar::X_OR_TIME_COL_SENTINEL {
                state.scroll_offset_pt.x = 0.0;
            } else {
                let (flat_start, _) = geom.col_span_flat(col);
                #[allow(clippy::cast_precision_loss)]
                let x0 = (flat_start as f32) * geom.subcol_w;
                let x1 = x0 + geom.subcol_w;
                if x0 < state.scroll_offset_pt.x || x1 > state.scroll_offset_pt.x + vis_w {
                    state.scroll_offset_pt.x = (x0 - vis_w * 0.35).max(0.0);
                }
            }
        }
        DataGridScrollTarget::RowTitle { row } => {
            #[allow(clippy::cast_precision_loss)]
            let y0 = (row as f32) * geom.row_h;
            let y1 = y0 + geom.row_h;
            if y0 < state.scroll_offset_pt.y || y1 > state.scroll_offset_pt.y + vis_h {
                state.scroll_offset_pt.y = (y0 - vis_h * 0.35).max(0.0);
            }
        }
    }
}

pub(super) fn paint_find_highlight(ui: &Ui, rect: Rect, tokens: &Tokens, is_active: bool) {
    let tint = tokens.warning.linear_multiply(if is_active { 0.38 } else { 0.22 });
    ui.painter().rect_filled(rect, CornerRadius::ZERO, tint);
    if is_active {
        ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.accent), StrokeKind::Inside);
    }
}

pub(super) fn forward_frozen_wheel(ui: &mut Ui, geom: &MultiGridGeom, state: &mut ColumnGridEguiState) {
    let Some(pointer) = ui.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.latest_pos())) else {
        return;
    };
    let delta = ui.input(|i| i.smooth_scroll_delta);
    if delta == Vec2::ZERO {
        return;
    }
    let mut consumed = false;
    if geom.row_gutter_rect.contains(pointer) && geom.need_v && delta.y.abs() > 0.0 {
        state.scroll_offset_pt.y -= delta.y;
        consumed = true;
    } else if geom.col_header_rect.contains(pointer) && geom.need_h && delta.x.abs() > 0.0 {
        state.scroll_offset_pt.x -= delta.x;
        consumed = true;
    }
    if consumed {
        clamp_multi_scroll(state, geom);
        ui.input_mut(|i| {
            i.smooth_scroll_delta = Vec2::ZERO;
        });
    }
}

pub(super) fn compute_multi_scrollbar_rects(ui: &Ui, geom: &MultiGridGeom, inner_rect: Rect) -> (Option<Rect>, Option<Rect>) {
    let scroll = ui.spacing().scroll;
    let bar_w = scroll.bar_width;
    let inner_m = scroll.bar_inner_margin;
    let v_rect = geom.need_v.then(|| {
        Rect::from_min_max(
            pos2(inner_rect.right() + inner_m, inner_rect.top()),
            pos2((inner_rect.right() + inner_m + bar_w).min(geom.grid_outer_rect.right()), inner_rect.bottom()),
        )
    });
    let h_rect = geom.need_h.then(|| {
        Rect::from_min_max(
            pos2(inner_rect.left(), inner_rect.bottom() + inner_m),
            pos2(inner_rect.right(), (inner_rect.bottom() + inner_m + bar_w).min(geom.grid_outer_rect.bottom())),
        )
    });
    (v_rect.filter(Rect::is_positive), h_rect.filter(Rect::is_positive))
}

pub(super) fn paint_scrollbar_corner_cap(ui: &Ui, geom: &MultiGridGeom, tokens: &Tokens) {
    if geom.need_v && geom.need_h {
        let cap = Rect::from_min_max(
            pos2(geom.grid_outer_rect.right() - geom.v_reserve, geom.grid_outer_rect.bottom() - geom.h_reserve),
            geom.grid_outer_rect.max,
        );
        if cap.is_positive() {
            ui.painter().rect_filled(cap, CornerRadius::ZERO, tokens.header_bg);
            ui.painter().rect_stroke(cap, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        }
    }
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
        TableArchetype::Column(_) => "COL",
        TableArchetype::Xy(_) => "XY",
        TableArchetype::Grouped(_) => "GRP",
        TableArchetype::Contingency(_) => "CONT",
        TableArchetype::Survival(_) => "SURV",
        TableArchetype::PartsOfWhole(_) => "POW",
        TableArchetype::MultipleVariables(_) => "MV",
        TableArchetype::Nested(_) => "NEST",
    }
}

pub(super) fn any_pointer_click_in(ui: &Ui, rect: Rect) -> bool {
    ui.input(|i| i.events.iter().any(|ev| matches!(ev, egui::Event::PointerButton { pos, pressed: false, .. } if rect.contains(*pos))))
}

pub(super) fn alt_row_fill(surface: Color32, panel: Color32) -> Color32 {
    let mixed = curvebook_palette::lerp_oklab(super::super::color32_to_rgba8(surface), super::super::color32_to_rgba8(panel), 0.25);
    super::super::rgba8_to_color32(mixed)
}

pub(super) const fn link_border_color32(color: Option<LinkBorderColor>, tokens: &Tokens) -> Color32 {
    match color {
        Some(LinkBorderColor::Blue) | None => tokens.link_blue,
        Some(LinkBorderColor::Green) => tokens.link_green,
        Some(LinkBorderColor::Red) => tokens.link_red,
        Some(LinkBorderColor::Charcoal) => tokens.text_muted,
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
    let font_id = if mono {
        egui::FontId::new(pt, egui::FontFamily::Name(std::sync::Arc::from(super::super::fonts::ALIGNED_NUMERAL_FONT_FAMILY)))
    } else {
        egui::FontId::proportional(pt)
    };
    CellTextStyle { font_id, color, align, italic, strike }
}

pub(super) fn render_ellipsized_text(ui: &Ui, clip_rect: Rect, text: &str, style: CellTextStyle) -> (bool, Rect) {
    let max_w = clip_rect.width() - CELL_PAD_X_PT * 2.0;
    if text.is_empty() || max_w <= 1.0 || clip_rect.height() <= 2.0 {
        return (false, Rect::NOTHING);
    }
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
    if !elided && (text_rect.left() < clip_rect.left() - 0.5 || text_rect.right() > clip_rect.right() + 0.5) {
        return (false, Rect::NOTHING);
    }
    ui.painter().with_clip_rect(clip_rect).galley(text_pos, galley, style.color);
    (elided, text_rect)
}

pub(super) fn table_display_decimals(ctx: &egui::Context) -> Option<u8> {
    crate::egui_shell::trailing_studio::studio_table_decimals(ctx)
}

pub(in crate::egui_shell) fn format_cell_into_with_decimals(cell: &Cell, preset: &LocalePreset, decimals: Option<u8>, out: &mut String) {
    out.clear();
    match &cell.value {
        CellValue::Empty => {}
        CellValue::Number(v) if v.is_finite() => {
            let normalized = if *v == 0.0 { 0.0 } else { *v };
            if let Some(dec) = decimals {
                let d = usize::from(dec.min(12));
                let _ = write!(out, "{normalized:.d$}");
                if let Some(rest) = out.strip_prefix('-')
                    && rest.bytes().all(|b| b == b'0' || b == b'.')
                {
                    out.remove(0);
                }
            } else {
                let _ = write!(out, "{normalized}");
            }
            if out.starts_with('-') && preset.minus_sign != "-" {
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

pub(super) fn format_cell_into(cell: &Cell, preset: &LocalePreset, out: &mut String) {
    format_cell_into_with_decimals(cell, preset, None, out);
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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedInlineCell {
    pub(super) value: serde_json::Value,
    pub(super) excluded: bool,
}

pub(super) fn parse_inline_cell_with_exclusion(app: &UiApp, draft: &str) -> ParsedInlineCell {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return ParsedInlineCell { value: serde_json::Value::Null, excluded: false };
    }
    if trimmed.starts_with('=') {
        return ParsedInlineCell { value: serde_json::Value::String(trimmed.to_owned()), excluded: false };
    }
    if let Some(stem) = trimmed.strip_suffix('*') {
        let inner = stem.trim();
        if !inner.is_empty() {
            if let Some(num) = parse_localized_number_input(inner, &app.locale_preset) {
                return ParsedInlineCell { value: json!(num), excluded: true };
            }
            return ParsedInlineCell { value: serde_json::Value::String(inner.to_owned()), excluded: true };
        }
    }
    if let Some(num) = parse_localized_number_input(trimmed, &app.locale_preset) {
        ParsedInlineCell { value: json!(num), excluded: false }
    } else {
        ParsedInlineCell { value: serde_json::Value::String(draft.to_owned()), excluded: false }
    }
}

pub(super) fn record_meta_if_enabled(ui: &Ui, meta: &mut FrameInspectMeta, enabled: bool, resp: &Response, input: FrameInspectEntryInput<'_>) {
    record_meta_with_unclipped_if_enabled(ui, meta, enabled, resp, resp.rect, input);
}

pub(super) fn record_meta_with_unclipped_if_enabled(
    ui: &Ui,
    meta: &mut FrameInspectMeta,
    enabled: bool,
    resp: &Response,
    unclipped_rect: Rect,
    input: FrameInspectEntryInput<'_>,
) {
    if !enabled {
        return;
    }
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() <= 0.5 || clipped.height() <= 0.5 {
        return;
    }
    meta.record_response_with_unclipped(ui, resp, unclipped_rect, input);
}

pub(super) struct LinkedEdgePaint<'a> {
    pub(super) reg: &'a LinkedDataRegion,
    pub(super) coord: GridCellCoord,
    pub(super) subs_per_col: usize,
    pub(super) color: Color32,
    pub(super) broken: bool,
}

#[must_use]
pub(in crate::egui_shell) fn is_linked_region_broken(doc: &ProjectDocument, reg: &LinkedDataRegion) -> bool {
    reg.spec.source.upstream_sheet_id().is_some_and(|up_id| {
        doc.sheet_section(up_id).is_none() || doc.result_sheet(up_id).is_some_and(|rs| matches!(rs.status, ResultStatus::Failed { .. }))
    })
}

pub(super) fn paint_broken_link_glyph(painter: &egui::Painter, rect: Rect, color: Color32) {
    let inner = rect.shrink(1.0);
    if !inner.is_positive() {
        return;
    }
    let c = inner.center();
    let s = (inner.width().min(inner.height()) * 0.28).max(2.2);
    let stroke = Stroke::new(1.4, color);
    painter.add(egui::Shape::line(
        vec![
            pos2(c.x - s * 1.15, c.y + s * 0.3),
            pos2(c.x - s * 1.55, c.y - s * 0.1),
            pos2(c.x - s * 0.85, c.y - s * 0.8),
            pos2(c.x - s * 0.3, c.y - s * 0.4),
        ],
        stroke,
    ));
    painter.add(egui::Shape::line(
        vec![
            pos2(c.x + s * 0.3, c.y + s * 0.4),
            pos2(c.x + s * 0.85, c.y + s * 0.8),
            pos2(c.x + s * 1.55, c.y + s * 0.1),
            pos2(c.x + s * 1.15, c.y - s * 0.3),
        ],
        stroke,
    ));
    painter.line_segment([pos2(c.x - s * 0.25, c.y - s * 0.85), pos2(c.x + s * 0.25, c.y + s * 0.85)], Stroke::new(1.2, color));
}

pub(super) fn paint_dashed_segment(painter: &egui::Painter, start: Pos2, end: Pos2, stroke: Stroke) {
    let delta = end - start;
    let total = delta.length();
    if total <= 0.5 {
        return;
    }
    let dir = delta / total;
    let dash = 4.0_f32;
    let gap = 3.0_f32;
    let mut offset = 0.0_f32;
    while offset < total {
        let seg_end = (offset + dash).min(total);
        painter.line_segment([start + dir * offset, start + dir * seg_end], stroke);
        offset += dash + gap;
    }
}

pub(super) fn paint_dashed_rect_stroke(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let r = rect.shrink(stroke.width * 0.5);
    paint_dashed_segment(painter, r.left_top(), r.right_top(), stroke);
    paint_dashed_segment(painter, r.right_top(), r.right_bottom(), stroke);
    paint_dashed_segment(painter, r.right_bottom(), r.left_bottom(), stroke);
    paint_dashed_segment(painter, r.left_bottom(), r.left_top(), stroke);
}

pub(super) fn paint_linked_perimeter_edges(ui: &Ui, cell_rect: Rect, clipped: Rect, spec: &LinkedEdgePaint<'_>) {
    let LinkedEdgePaint { reg, coord, subs_per_col, color, broken } = *spec;
    let r = &reg.target_range;
    let painter = ui.painter().with_clip_rect(clipped);
    let last_sub = if coord.col == r.end.col { r.end.subcol.min(subs_per_col.saturating_sub(1)) } else { subs_per_col.saturating_sub(1) };

    if broken {
        let stroke = Stroke::new(2.0, color);
        if coord.row == r.start.row {
            paint_dashed_segment(&painter, pos2(cell_rect.left(), cell_rect.top() + 1.0), pos2(cell_rect.right(), cell_rect.top() + 1.0), stroke);
        }
        if coord.row == r.end.row {
            paint_dashed_segment(
                &painter,
                pos2(cell_rect.left(), cell_rect.bottom() - 1.0),
                pos2(cell_rect.right(), cell_rect.bottom() - 1.0),
                stroke,
            );
        }
        if coord.col == r.start.col && coord.subcol == r.start.subcol {
            paint_dashed_segment(&painter, pos2(cell_rect.left() + 1.0, cell_rect.top()), pos2(cell_rect.left() + 1.0, cell_rect.bottom()), stroke);
        }
        if coord.col == r.end.col && coord.subcol == last_sub {
            paint_dashed_segment(&painter, pos2(cell_rect.right() - 1.0, cell_rect.top()), pos2(cell_rect.right() - 1.0, cell_rect.bottom()), stroke);
        }
        if coord.row == r.start.row
            && coord.col == r.start.col
            && coord.subcol == r.start.subcol
            && clipped.width() >= 14.0
            && clipped.height() >= 14.0
        {
            paint_broken_link_glyph(&painter, Rect::from_min_size(cell_rect.min + vec2(2.0, 2.0), vec2(10.0, 10.0)), color);
        }
        return;
    }

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

/// Structured tooltip content (`Band 1` title + optional `Band 2` summary) for a data cell (`R20.21`).
pub(super) struct StructuredCellTooltip {
    pub(super) title: String,
    pub(super) summary: Option<String>,
}

/// Build structured tooltip data for a grid cell combining elided text, microplate source well
/// provenance (`Source well: <well>`), `cell_exclusion_tooltip`, and contingency validation hints (`R17.7` / `R20.21`).
#[must_use]
pub(super) fn build_data_cell_tooltip(
    ds: &DataSheet,
    coord: GridCellCoord,
    excluded: bool,
    contingency_invalid: bool,
    elided_text: Option<&str>,
    locale: &str,
) -> Option<StructuredCellTooltip> {
    let full_elided = elided_text.map(str::trim).filter(|s| !s.is_empty());
    let well_line = ds.plate_well_origin(CellCoord::from(coord)).map(|well| {
        let prefix = crate::i18n::tr_or_fallback(locale, "grid.tooltip.source_well", "Source well");
        format!("{prefix}: {well}")
    });
    let excl_line = excluded.then(|| {
        crate::grid_view::GridViewState::cell_exclusion_tooltip(ds, coord)
            .unwrap_or_else(|| crate::i18n::tr_or_fallback(locale, "w5.grid_core.excluded_cell", "Excluded from analyses").to_owned())
    });
    let invalid_line = (!excluded && contingency_invalid).then(|| String::from("Contingency counts should be non-negative integers"));

    if let Some(full) = full_elided {
        let mut details = Vec::new();
        if let Some(w) = well_line {
            details.push(w);
        }
        if let Some(ex) = excl_line {
            details.push(ex);
        } else if let Some(inv) = invalid_line {
            details.push(inv);
        }
        return Some(StructuredCellTooltip { title: full.to_owned(), summary: (!details.is_empty()).then(|| details.join("\n")) });
    }
    if let Some(excl) = excl_line {
        return Some(StructuredCellTooltip { title: excl, summary: well_line });
    }
    if let Some(inv) = invalid_line {
        return Some(StructuredCellTooltip {
            title: String::from("Invalid Contingency Count"),
            summary: Some(well_line.map_or(inv.clone(), |w| format!("{w}\n{inv}"))),
        });
    }
    well_line.map(|well| StructuredCellTooltip { title: well, summary: None })
}

/// Format the hover tooltip string for a data cell (`format_data_cell_hover_tooltip` helper preserved for callers/tests).
#[allow(dead_code)]
#[must_use]
pub(super) fn format_data_cell_hover_tooltip(
    ds: &DataSheet,
    coord: GridCellCoord,
    excluded: bool,
    contingency_invalid: bool,
    elided_text: Option<&str>,
    locale: &str,
) -> Option<String> {
    let tip = build_data_cell_tooltip(ds, coord, excluded, contingency_invalid, elided_text, locale)?;
    Some(match tip.summary {
        Some(body) => format!("{}\n{body}", tip.title),
        None => tip.title,
    })
}

pub(super) fn pending_commit_key(sheet_id: &SheetId) -> Id {
    Id::unique(("column_grid_pending_commit", sheet_id.as_str()))
}

pub(super) fn grid_focus_key(sheet_id: &SheetId) -> Id {
    Id::unique(("column_grid_focused", sheet_id.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GridDragOrigin {
    DataCells { anchor: GridCellCoord },
    XCells { anchor: (usize, usize) },
    RowHeaders { anchor_row: usize },
    ColHeaders { anchor_col: usize, anchor_sub: usize, anchor_subs: usize, whole_col: bool },
    XHeaders { anchor_sub: usize, whole_x: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct GridDragState {
    origin: GridDragOrigin,
    press_pos: Pos2,
    drag_activated: bool,
    defer_select_until_drag: bool,
    shift_started: bool,
    last_emitted_pair: Option<(GridCellCoord, GridCellCoord)>,
    last_emitted_x: Option<((usize, usize), (usize, usize))>,
    last_commit_time_s: f64,
    last_autoscroll_frame: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct GridDragOutcome {
    pub(super) preview_data_range: Option<(GridCellCoord, GridCellCoord)>,
    pub(super) edge_scrolled: bool,
    pub(super) suppress_click_this_frame: bool,
    pub(super) press_selected_data: bool,
    pub(super) press_selected_x: bool,
    pub(super) click_already_selected_data: Option<GridCellCoord>,
}

fn grid_drag_state_key(pane_salt: &str, sheet_id: &SheetId) -> Id {
    Id::unique(("grid_drag_state", pane_salt, sheet_id.as_str()))
}

fn clamp_row_at_y(geom: &MultiGridGeom, scroll_y: f32, y: f32) -> usize {
    let min_y = geom.body_inner_clip.min.y;
    let max_y = (geom.body_inner_clip.max.y - 0.5).max(min_y);
    let origin_y = geom.body_inner_clip.min.y - scroll_y;
    let rel_y = (y.clamp(min_y, max_y) - origin_y).max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let row = (rel_y / geom.row_h).floor().max(0.0) as usize;
    row.min(geom.row_count.saturating_sub(1))
}

fn clamp_flat_all_at_x(geom: &MultiGridGeom, scroll_x: f32, x: f32) -> usize {
    let min_x = geom.body_inner_clip.min.x;
    let max_x = (geom.body_inner_clip.max.x - 0.5).max(min_x);
    let origin_x = geom.body_inner_clip.min.x - scroll_x;
    let rel_x = (x.clamp(min_x, max_x) - origin_x).max(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let flat = (rel_x / geom.subcol_w).floor().max(0.0) as usize;
    flat.min(geom.total_flat_cols.saturating_sub(1))
}

fn edge_autoscroll_axis_delta(pos: f32, min_edge: f32, max_edge: f32, band: f32, dt: f32) -> f32 {
    let span = (max_edge - min_edge).max(0.0);
    if span <= 0.0 {
        return 0.0;
    }
    let inside_band = band.min(span * 0.25);
    let min_speed = 96.0_f32;
    let max_speed = 720.0_f32;
    let ramp_span = 64.0_f32;
    if pos < min_edge + inside_band {
        let t = ((min_edge + inside_band - pos) / ramp_span).clamp(0.0, 1.0);
        -(min_speed + t * (max_speed - min_speed)) * dt
    } else if pos > max_edge - inside_band {
        let t = ((pos - (max_edge - inside_band)) / ramp_span).clamp(0.0, 1.0);
        (min_speed + t * (max_speed - min_speed)) * dt
    } else {
        0.0
    }
}

fn classify_drag_start(
    ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    state: &ColumnGridEguiState,
    aux: &MultiGridAuxState,
    press_pos: Pos2,
    shift: bool,
) -> Option<(GridDragOrigin, bool)> {
    let geom = mctx.geom;
    let ds = mctx.ds;
    if geom.row_count == 0 || geom.total_flat_cols == 0 {
        return None;
    }
    if ui.ctx().layer_id_at(press_pos).is_some_and(|l| l.order > ui.layer_id().order) {
        return None;
    }
    let scroll = state.scroll_offset_pt;
    if geom.body_inner_clip.contains(press_pos) {
        let origin_x = geom.body_inner_clip.min.x - scroll.x;
        let origin_y = geom.body_inner_clip.min.y - scroll.y;
        let rel_x = press_pos.x - origin_x;
        let rel_y = press_pos.y - origin_y;
        if rel_x < 0.0 || rel_x >= geom.content_w || rel_y < 0.0 || rel_y >= geom.content_h {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let row = ((rel_y / geom.row_h).floor() as usize).min(geom.row_count - 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let flat_all = ((rel_x / geom.subcol_w).floor() as usize).min(geom.total_flat_cols - 1);
        if flat_all < geom.x_subcols {
            let anchor = if shift { aux.selected_x_anchor.or(aux.selected_x_cell).unwrap_or((row, flat_all)) } else { (row, flat_all) };
            return Some((GridDragOrigin::XCells { anchor }, false));
        }
        if let Some((col, subcol)) = geom.flat_data_to_coord(flat_all - geom.x_subcols) {
            let coord = GridCellCoord::new(row, col, subcol);
            let anchor = if shift { mctx.app.grid.tool.anchor.unwrap_or(coord) } else { coord };
            return Some((GridDragOrigin::DataCells { anchor }, false));
        }
        return None;
    }

    let row_num_rect = Rect::from_min_max(geom.row_gutter_rect.min, pos2(geom.row_gutter_rect.min.x + geom.row_num_w, geom.row_gutter_rect.max.y))
        .intersect(geom.row_gutter_rect);
    if row_num_rect.contains(press_pos) && geom.col_count > 0 {
        let rel_y = press_pos.y - (geom.row_gutter_rect.min.y - scroll.y);
        if rel_y < 0.0 || rel_y >= geom.content_h {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let row = ((rel_y / geom.row_h).floor() as usize).min(geom.row_count - 1);
        let anchor_row = if shift { mctx.app.grid.tool.anchor.map_or(row, |a| a.row).min(geom.row_count - 1) } else { row };
        return Some((GridDragOrigin::RowHeaders { anchor_row }, false));
    }

    if geom.col_header_rect.contains(press_pos) {
        let rel_x = press_pos.x - (geom.col_header_rect.min.x - scroll.x);
        if rel_x < 0.0 || rel_x >= geom.content_w {
            return None;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let flat_all = ((rel_x / geom.subcol_w).floor() as usize).min(geom.total_flat_cols - 1);
        let y2 = geom.col_header_rect.min.y + geom.h_tier1;
        let y3 = y2 + geom.h_tier2;
        let is_tier1 = press_pos.y < y2;
        let is_tier2 = !is_tier1 && press_pos.y < y3;
        if flat_all < geom.x_subcols {
            let defer = is_tier2 && flat_all == 0 && !ds.frozen;
            return Some((GridDragOrigin::XHeaders { anchor_sub: if is_tier1 { 0 } else { flat_all }, whole_x: is_tier1 }, defer));
        }
        if let Some((col, subcol)) = geom.flat_data_to_coord(flat_all - geom.x_subcols) {
            let is_mv = matches!(ds.archetype, TableArchetype::MultipleVariables(_));
            if !is_tier1 && !is_tier2 && is_mv {
                return None;
            }
            let (_, span_subs) = geom.col_span_flat(col);
            if is_tier1 || is_tier2 {
                let anchor_col = if shift && is_tier1 { mctx.app.grid.tool.anchor.map_or(col, |a| a.col).min(geom.col_count - 1) } else { col };
                let (_, anchor_subs) = geom.col_span_flat(anchor_col);
                return Some((GridDragOrigin::ColHeaders { anchor_col, anchor_sub: 0, anchor_subs, whole_col: true }, is_tier2 && !ds.frozen));
            }
            let is_nested = matches!(ds.archetype, TableArchetype::Nested(_));
            return Some((
                GridDragOrigin::ColHeaders { anchor_col: col, anchor_sub: subcol, anchor_subs: span_subs, whole_col: false },
                is_nested && !ds.frozen,
            ));
        }
    }
    None
}

fn apply_drag_edge_autoscroll(
    ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    drag: &mut GridDragState,
    pos: Pos2,
    frame_nr: u64,
) -> bool {
    let geom = mctx.geom;
    let prev_scroll = fstate.state.scroll_offset_pt;
    let band_y = 12.0_f32;
    let band_x = 16.0_f32;
    let raw_dt = ui.input(|i| i.stable_dt);
    let dt = if raw_dt > 0.0 { raw_dt.clamp(1.0 / 120.0, 0.05) } else { 1.0 / 60.0 };
    let mut dx = 0.0_f32;
    let mut dy = 0.0_f32;
    match drag.origin {
        GridDragOrigin::DataCells { .. } => {
            if geom.need_v {
                dy = edge_autoscroll_axis_delta(pos.y, geom.body_inner_clip.min.y, geom.body_inner_clip.max.y, band_y, dt);
            }
            if geom.need_h {
                dx = edge_autoscroll_axis_delta(pos.x, geom.body_inner_clip.min.x, geom.body_inner_clip.max.x, band_x, dt);
            }
        }
        GridDragOrigin::XCells { .. } => {
            if geom.need_v {
                dy = edge_autoscroll_axis_delta(pos.y, geom.body_inner_clip.min.y, geom.body_inner_clip.max.y, band_y, dt);
            }
            if geom.need_h && pos.x < geom.body_inner_clip.min.x + band_x {
                dx = edge_autoscroll_axis_delta(pos.x, geom.body_inner_clip.min.x, geom.body_inner_clip.max.x, band_x, dt).min(0.0);
            }
        }
        GridDragOrigin::RowHeaders { .. } => {
            if geom.need_v {
                dy = edge_autoscroll_axis_delta(pos.y, geom.row_gutter_rect.min.y, geom.row_gutter_rect.max.y, band_y, dt);
            }
        }
        GridDragOrigin::ColHeaders { .. } => {
            if geom.need_h {
                dx = edge_autoscroll_axis_delta(pos.x, geom.col_header_rect.min.x, geom.col_header_rect.max.x, band_x, dt);
            }
        }
        GridDragOrigin::XHeaders { .. } => {
            if geom.need_h && pos.x < geom.col_header_rect.min.x + band_x {
                dx = edge_autoscroll_axis_delta(pos.x, geom.col_header_rect.min.x, geom.col_header_rect.max.x, band_x, dt).min(0.0);
            }
        }
    }
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    fstate.state.scroll_offset_pt.x += dx;
    fstate.state.scroll_offset_pt.y += dy;
    clamp_multi_scroll(fstate.state, geom);
    drag.last_autoscroll_frame = frame_nr;
    let scrolled = (fstate.state.scroll_offset_pt - prev_scroll).length_sq() > 0.01;
    if scrolled && !ui.ctx().will_discard() {
        ui.ctx().request_repaint();
    }
    scrolled
}

struct DragDataCommit {
    anchor: GridCellCoord,
    target: GridCellCoord,
    span_cells: usize,
    primary_released: bool,
    time_s: f64,
}

fn commit_data_drag_selection(
    ui: &Ui,
    sheet_id: &SheetId,
    fstate: &mut MultiGridFrameState<'_>,
    outcome: &mut GridDragOutcome,
    drag: &mut GridDragState,
    c: DragDataCommit,
) {
    fstate.aux.clear_x_selection();
    fstate.aux.active_role_menu_col = None;
    fstate.state.context_menu = None;
    set_grid_focus_active(ui, true);
    outcome.preview_data_range = Some((c.anchor, c.target));
    outcome.press_selected_data = true;
    let pair = (c.anchor, c.target);
    if drag.last_emitted_pair == Some(pair) {
        return;
    }
    let anchor_changed = drag.last_emitted_pair.is_none_or(|(prev_a, _)| prev_a != c.anchor);
    let throttled_ok = c.primary_released || c.span_cells <= 1024 || (c.time_s - drag.last_commit_time_s) >= 0.05;
    if anchor_changed {
        if drag.shift_started && matches!(drag.origin, GridDragOrigin::DataCells { .. }) {
            fstate.intents.push(UiIntent::ExtendGridSelection { sheet_id: Some(sheet_id.clone()), coord: c.target });
        } else {
            fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(sheet_id.clone()), coord: c.anchor });
            if c.target != c.anchor || !matches!(drag.origin, GridDragOrigin::DataCells { .. }) {
                fstate.intents.push(UiIntent::ExtendGridSelection { sheet_id: Some(sheet_id.clone()), coord: c.target });
            }
        }
        drag.last_emitted_pair = Some(pair);
        drag.last_commit_time_s = c.time_s;
    } else if throttled_ok {
        fstate.intents.push(UiIntent::ExtendGridSelection { sheet_id: Some(sheet_id.clone()), coord: c.target });
        drag.last_emitted_pair = Some(pair);
        drag.last_commit_time_s = c.time_s;
    }
}

struct DragStepInput<'a> {
    sheet_id: &'a SheetId,
    cur_pos: Pos2,
    primary_pressed: bool,
    primary_released: bool,
    time_s: f64,
}

fn step_active_drag_selection(
    ui: &Ui,
    app: &UiApp,
    geom: &MultiGridGeom,
    fstate: &mut MultiGridFrameState<'_>,
    outcome: &mut GridDragOutcome,
    drag: &mut GridDragState,
    input: DragStepInput<'_>,
) {
    let DragStepInput { sheet_id, cur_pos, primary_pressed, primary_released, time_s } = input;
    let scroll = fstate.state.scroll_offset_pt;
    let last_row = geom.row_count.saturating_sub(1);
    match drag.origin {
        GridDragOrigin::XCells { anchor } => {
            let row = clamp_row_at_y(geom, scroll.y, cur_pos.y);
            let sx = clamp_flat_all_at_x(geom, scroll.x, cur_pos.x).min(geom.x_subcols.saturating_sub(1));
            let pair = (anchor, (row, sx));
            fstate.aux.selected_x_anchor = Some(anchor);
            fstate.aux.selected_x_cell = Some((row, sx));
            fstate.aux.active_role_menu_col = None;
            fstate.state.context_menu = None;
            set_grid_focus_active(ui, true);
            outcome.press_selected_x = true;
            if drag.last_emitted_x != Some(pair) {
                drag.last_emitted_x = Some(pair);
                push_select_x_if_needed(app, row, sx, fstate.intents);
                if !ui.ctx().will_discard() {
                    ui.ctx().request_repaint();
                }
            }
        }
        GridDragOrigin::XHeaders { anchor_sub, whole_x } => {
            let sx = clamp_flat_all_at_x(geom, scroll.x, cur_pos.x).min(geom.x_subcols.saturating_sub(1));
            let pair = if whole_x { ((0, 0), (last_row, geom.x_subcols.saturating_sub(1))) } else { ((0, anchor_sub), (last_row, sx)) };
            fstate.aux.selected_x_anchor = Some(pair.0);
            fstate.aux.selected_x_cell = Some(pair.1);
            fstate.aux.active_role_menu_col = None;
            fstate.state.context_menu = None;
            set_grid_focus_active(ui, true);
            outcome.press_selected_x = true;
            if drag.last_emitted_x != Some(pair) {
                drag.last_emitted_x = Some(pair);
                push_select_x_if_needed(app, pair.1.0, pair.1.1, fstate.intents);
                if !ui.ctx().will_discard() {
                    ui.ctx().request_repaint();
                }
            }
        }
        GridDragOrigin::DataCells { anchor } if geom.total_data_flat_cols > 0 => {
            let row = clamp_row_at_y(geom, scroll.y, cur_pos.y);
            let flat_data = clamp_flat_all_at_x(geom, scroll.x, cur_pos.x).saturating_sub(geom.x_subcols).min(geom.total_data_flat_cols - 1);
            if let Some((col, subcol)) = geom.flat_data_to_coord(flat_data) {
                let target = GridCellCoord::new(row, col, subcol);
                if primary_pressed && !drag.shift_started && target == anchor {
                    outcome.click_already_selected_data = Some(anchor);
                }
                let fa = geom.coord_to_flat_all(false, anchor.col, anchor.subcol).saturating_sub(geom.x_subcols);
                let span_cells = (anchor.row.abs_diff(row) + 1).saturating_mul(fa.abs_diff(flat_data) + 1);
                commit_data_drag_selection(
                    ui,
                    sheet_id,
                    fstate,
                    outcome,
                    drag,
                    DragDataCommit { anchor, target, span_cells, primary_released, time_s },
                );
            }
        }
        GridDragOrigin::RowHeaders { anchor_row } if geom.col_count > 0 => {
            let row = clamp_row_at_y(geom, scroll.y, cur_pos.y);
            let last_col = geom.col_count - 1;
            let last_sub = geom.col_span_flat(last_col).1.saturating_sub(1);
            let anchor = GridCellCoord::new(anchor_row, 0, 0);
            let target = GridCellCoord::new(row, last_col, last_sub);
            let span_cells = (anchor_row.abs_diff(row) + 1).saturating_mul(geom.total_data_flat_cols.max(1));
            commit_data_drag_selection(ui, sheet_id, fstate, outcome, drag, DragDataCommit { anchor, target, span_cells, primary_released, time_s });
        }
        GridDragOrigin::ColHeaders { anchor_col, anchor_sub, anchor_subs, whole_col } if geom.total_data_flat_cols > 0 => {
            let flat_data = clamp_flat_all_at_x(geom, scroll.x, cur_pos.x).saturating_sub(geom.x_subcols).min(geom.total_data_flat_cols - 1);
            if let Some((t_col, t_sub)) = geom.flat_data_to_coord(flat_data) {
                let (anchor, target) = if whole_col {
                    let (_, t_subs) = geom.col_span_flat(t_col);
                    if t_col >= anchor_col {
                        (GridCellCoord::new(0, anchor_col, 0), GridCellCoord::new(last_row, t_col, t_subs.saturating_sub(1)))
                    } else {
                        (GridCellCoord::new(0, anchor_col, anchor_subs.saturating_sub(1)), GridCellCoord::new(last_row, t_col, 0))
                    }
                } else {
                    (GridCellCoord::new(0, anchor_col, anchor_sub), GridCellCoord::new(last_row, t_col, t_sub))
                };
                let fa = geom.coord_to_flat_all(false, anchor.col, anchor.subcol).saturating_sub(geom.x_subcols);
                let fb = geom.coord_to_flat_all(false, target.col, target.subcol).saturating_sub(geom.x_subcols);
                let span_cells = geom.row_count.saturating_mul(fa.abs_diff(fb) + 1);
                commit_data_drag_selection(
                    ui,
                    sheet_id,
                    fstate,
                    outcome,
                    drag,
                    DragDataCommit { anchor, target, span_cells, primary_released, time_s },
                );
            }
        }
        _ => {}
    }
}

pub(super) fn prepare_grid_drag_and_autoscroll(
    ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    factor_title_editing: bool,
) -> GridDragOutcome {
    let geom = mctx.geom;
    let ds = mctx.ds;
    let key = grid_drag_state_key(mctx.pane_salt, &ds.id);
    let mut outcome = GridDragOutcome::default();

    let frame_nr = ui.ctx().cumulative_frame_nr();
    let (primary_pressed, primary_down, primary_released, shift, cmd_mod, ptr_pos, press_event_pos, time_s) = ui.input(|i| {
        let ev_press_pos = i.events.iter().find_map(|ev| match ev {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.modifiers.shift,
            i.modifiers.command || i.modifiers.mac_cmd || i.modifiers.ctrl,
            i.pointer.interact_pos().or_else(|| i.pointer.latest_pos()),
            ev_press_pos,
            i.time,
        )
    });

    if mctx.blocked || geom.row_count == 0 || geom.total_flat_cols == 0 {
        ui.ctx().data_mut(|d| d.remove_temp::<Option<GridDragState>>(key));
        return outcome;
    }

    let can_start_drag = fstate.state.inline_edit.is_none()
        && fstate.aux.aux_edit.is_none()
        && !factor_title_editing
        && fstate.state.context_menu.is_none()
        && fstate.aux.active_role_menu_col.is_none()
        && !egui::Popup::is_any_open(ui.ctx());

    let mut drag_opt = ui.ctx().data(|d| d.get_temp::<Option<GridDragState>>(key).flatten());

    if primary_pressed
        && !cmd_mod
        && can_start_drag
        && let Some(start_pos) = press_event_pos.or(ptr_pos)
        && let Some((origin, defer_select_until_drag)) = classify_drag_start(ui, mctx, fstate.state, fstate.aux, start_pos, shift)
    {
        drag_opt = Some(GridDragState {
            origin,
            press_pos: start_pos,
            drag_activated: false,
            defer_select_until_drag,
            shift_started: shift,
            last_emitted_pair: None,
            last_emitted_x: None,
            last_commit_time_s: -1.0,
            last_autoscroll_frame: 0,
        });
    }

    let Some(mut drag) = drag_opt else {
        return outcome;
    };
    if !primary_down && !primary_released {
        ui.ctx().data_mut(|d| d.remove_temp::<Option<GridDragState>>(key));
        return outcome;
    }
    if !drag.drag_activated && ptr_pos.is_some_and(|pos| pos.distance(drag.press_pos) >= 4.0) {
        drag.drag_activated = true;
    }
    if primary_down
        && !primary_released
        && drag.drag_activated
        && drag.last_autoscroll_frame != frame_nr
        && let Some(pos) = ptr_pos
    {
        outcome.edge_scrolled = apply_drag_edge_autoscroll(ui, mctx, fstate, &mut drag, pos, frame_nr);
    }
    if !drag.defer_select_until_drag || drag.drag_activated {
        let cur_pos = ptr_pos.unwrap_or(drag.press_pos);
        step_active_drag_selection(
            ui,
            mctx.app,
            geom,
            fstate,
            &mut outcome,
            &mut drag,
            DragStepInput { sheet_id: &ds.id, cur_pos, primary_pressed, primary_released, time_s },
        );
    }
    if primary_released {
        if drag.drag_activated {
            outcome.suppress_click_this_frame = true;
        }
        ui.ctx().data_mut(|d| d.remove_temp::<Option<GridDragState>>(key));
    } else {
        ui.ctx().data_mut(|d| d.insert_temp(key, Some(drag)));
    }
    outcome
}

pub(super) fn push_select_x_if_needed(app: &UiApp, row: usize, subcol: usize, intents: &mut PendingUiIntents) {
    if app.grid.active_x_cell != Some((row, subcol)) || app.grid.tool.active.is_some() || !app.grid.selected_coords.is_empty() {
        intents.push(UiIntent::SelectXCell { row, subcol });
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_discontig_selection_perimeter(
    body_ui: &Ui,
    geom: &MultiGridGeom,
    tokens: &Tokens,
    origin: Pos2,
    vis_rows: &Range<usize>,
    vis_all_flat: &Range<usize>,
    sel: &SelectionFastLookup<'_>,
    pane_focused: bool,
    high_contrast: bool,
) {
    let (stroke, dashed) = if pane_focused {
        (Stroke::new(1.5, tokens.accent), false)
    } else if high_contrast {
        (Stroke::new(1.0, tokens.border), true)
    } else {
        (Stroke::new(1.0, tokens.divider), false)
    };
    let draw_edge = |p0: Pos2, p1: Pos2| {
        if dashed {
            paint_dashed_segment(body_ui.painter(), p0, p1, stroke);
        } else {
            body_ui.painter().line_segment([p0, p1], stroke);
        }
    };
    let inset = 0.75_f32;
    for row in vis_rows.clone() {
        #[allow(clippy::cast_precision_loss)]
        let y0 = origin.y + (row as f32) * geom.row_h;
        let y1 = y0 + geom.row_h;
        for flat in vis_all_flat.clone() {
            let Some(flat_data) = flat.checked_sub(geom.x_subcols) else { continue };
            if !sel.contains_flat_cell(geom, row, flat_data) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let x0 = origin.x + (flat as f32) * geom.subcol_w;
            let x1 = x0 + geom.subcol_w;
            if row == 0 || !sel.contains_flat_cell(geom, row - 1, flat_data) {
                draw_edge(pos2(x0, y0 + inset), pos2(x1, y0 + inset));
            }
            if row + 1 >= geom.row_count || !sel.contains_flat_cell(geom, row + 1, flat_data) {
                draw_edge(pos2(x0, y1 - inset), pos2(x1, y1 - inset));
            }
            if flat_data == 0 || !sel.contains_flat_cell(geom, row, flat_data - 1) {
                draw_edge(pos2(x0 + inset, y0), pos2(x0 + inset, y1));
            }
            if flat_data + 1 >= geom.total_data_flat_cols || !sel.contains_flat_cell(geom, row, flat_data + 1) {
                draw_edge(pos2(x1 - inset, y0), pos2(x1 - inset, y1));
            }
        }
    }
}

pub(super) fn paint_selection_perimeter_overlay(
    body_ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    origin: Pos2,
    vis_rows: &Range<usize>,
    vis_all_flat: &Range<usize>,
    sel: &SelectionFastLookup<'_>,
    aux: &MultiGridAuxState,
) {
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let focused = mctx.pane_focused;
    let high_contrast = tokens.mode == crate::theme::UiThemeMode::HighContrast;
    let vis_clamp = geom.body_inner_clip.expand(8.0);
    let safe_clipped_rect = |x0: f32, y0: f32, x1: f32, y1: f32| -> Option<Rect> {
        let r = Rect::from_min_max(pos2(x0.max(vis_clamp.min.x), y0.max(vis_clamp.min.y)), pos2(x1.min(vis_clamp.max.x), y1.min(vis_clamp.max.y)));
        (r.width() > 0.0 && r.height() > 0.0).then_some(r)
    };
    let paint_range_perim = |rect: Rect| {
        if focused {
            body_ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.5, tokens.accent), StrokeKind::Inside);
        } else if high_contrast {
            super::super::ui_kit::paint_dashed_rect_stroke(body_ui.painter(), rect, Stroke::new(1.0, tokens.border));
        } else {
            body_ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.divider), StrokeKind::Inside);
        }
    };
    let paint_active_cell = |rect: Rect| {
        if focused {
            body_ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(2.0, tokens.focus_ring), StrokeKind::Inside);
        } else if high_contrast {
            super::super::ui_kit::paint_dashed_rect_stroke(body_ui.painter(), rect, Stroke::new(1.0, tokens.border));
        } else {
            body_ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
        }
    };

    if let Some((xr0, xr1, xs0, xs1)) = aux.selected_x_bounds() {
        let is_multi_x = xr1 > xr0 || xs1 > xs0;
        #[allow(clippy::cast_precision_loss)]
        let x0 = origin.x + (xs0 as f32) * geom.subcol_w;
        #[allow(clippy::cast_precision_loss)]
        let x1 = origin.x + ((xs1 + 1) as f32) * geom.subcol_w;
        #[allow(clippy::cast_precision_loss)]
        let y0 = origin.y + (xr0 as f32) * geom.row_h;
        #[allow(clippy::cast_precision_loss)]
        let y1 = origin.y + ((xr1 + 1) as f32) * geom.row_h;
        if is_multi_x && let Some(perim_r) = safe_clipped_rect(x0, y0, x1, y1) {
            paint_range_perim(perim_r);
        }
        if let Some((ax_r, ax_s)) = aux.selected_x_cell
            && vis_rows.contains(&ax_r)
            && vis_all_flat.contains(&ax_s)
        {
            #[allow(clippy::cast_precision_loss)]
            let ax_x0 = origin.x + (ax_s as f32) * geom.subcol_w;
            #[allow(clippy::cast_precision_loss)]
            let ax_y0 = origin.y + (ax_r as f32) * geom.row_h;
            let raw_r = Rect::from_min_max(pos2(ax_x0, ax_y0), pos2(ax_x0 + geom.subcol_w, ax_y0 + geom.row_h));
            let clipped = raw_r.intersect(geom.body_inner_clip);
            if clipped.width() >= 1.0
                && clipped.height() >= 1.0
                && let Some(ax_rect) = safe_clipped_rect(raw_r.min.x, raw_r.min.y, raw_r.max.x, raw_r.max.y)
            {
                paint_active_cell(ax_rect);
            }
        }
        return;
    }

    if let Some((r0, r1, f0, f1)) = sel.contiguous_rect() {
        let all_f0 = geom.x_subcols + f0;
        let all_f1 = geom.x_subcols + f1;
        #[allow(clippy::cast_precision_loss)]
        let x0 = origin.x + (all_f0 as f32) * geom.subcol_w;
        #[allow(clippy::cast_precision_loss)]
        let x1 = origin.x + ((all_f1 + 1) as f32) * geom.subcol_w;
        #[allow(clippy::cast_precision_loss)]
        let y0 = origin.y + (r0 as f32) * geom.row_h;
        #[allow(clippy::cast_precision_loss)]
        let y1 = origin.y + ((r1 + 1) as f32) * geom.row_h;
        if (r1 > r0 || f1 > f0)
            && let Some(perim_r) = safe_clipped_rect(x0, y0, x1, y1)
        {
            paint_range_perim(perim_r);
        }
    } else if sel.has_discontiguous() {
        paint_discontig_selection_perimeter(body_ui, geom, tokens, origin, vis_rows, vis_all_flat, sel, focused, high_contrast);
    }

    if let Some(active) = sel.active {
        let flat_all = geom.coord_to_flat_all(false, active.col, active.subcol);
        if vis_rows.contains(&active.row) && vis_all_flat.contains(&flat_all) {
            #[allow(clippy::cast_precision_loss)]
            let ax0 = origin.x + (flat_all as f32) * geom.subcol_w;
            #[allow(clippy::cast_precision_loss)]
            let ay0 = origin.y + (active.row as f32) * geom.row_h;
            let raw_r = Rect::from_min_max(pos2(ax0, ay0), pos2(ax0 + geom.subcol_w, ay0 + geom.row_h));
            let clipped = raw_r.intersect(geom.body_inner_clip);
            if clipped.width() >= 1.0
                && clipped.height() >= 1.0
                && let Some(active_r) = safe_clipped_rect(raw_r.min.x, raw_r.min.y, raw_r.max.x, raw_r.max.y)
            {
                paint_active_cell(active_r);
            }
        }
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
