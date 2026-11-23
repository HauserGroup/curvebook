//! Virtualized single-archetype `Column` grid view (`R15.2`, `gui-layout-spec.md` §8.2).

use std::ops::Range;

use curvebook_doc::{Cell, CellValue, LinkedDataRegion};
use curvebook_tools::GridCellCoord;
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::{Align, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::GridViewState;
use crate::inspect::ZLayer;
use crate::views::grid_core::{grid_tr, linked_region_for_cell};

use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents};
use super::headers::{render_column_headers_3tier, render_corner_and_row_titles_header};
use super::inline_edit::{InlineTextEditSpec, begin_inline_edit, render_inline_text_edit};
use super::linked_regions::{open_link_popover, render_link_provenance_strip, render_linked_region_guard_popover};
use super::multi_archetype::{GridContextMenuHost, render_grid_context_menu_popup};
use super::shared::LinkedEdgePaint;
use super::shared::{
    BodyRenderResult, CellTextStyle, ContextCellTarget, ContextMenuState, GridRenderCtx, LINK_STRIP_HEIGHT_PT, SelectionFastLookup,
    active_column_sheet, alt_row_fill, any_pointer_click_in, clamp_scroll_offset, clear_grid_drag_anchor_if_released, compute_grid_geometry,
    compute_scrollbar_rects, compute_visible_range, ensure_solid_scrollbar_style, format_cell_into_with_decimals, forward_gutter_header_wheel,
    handle_cell_pointer_drag_select, initial_edit_text_for_cell, is_linked_region_broken, is_multi_click_edit, link_border_color32,
    paint_exclusion_corner_marker, paint_linked_perimeter_edges, paint_scrollbar_corner_cap, record_meta_if_enabled,
    record_meta_with_unclipped_if_enabled, render_ellipsized_text, set_grid_focus_active, table_display_decimals,
};
use super::shortcuts::handle_grid_keyboard_shortcuts;
use super::{ColumnGridEguiState, ColumnGridFrameReport, ColumnGridRenderContext, GridInlineEditTarget};

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

    render_grid_context_menu_popup(
        ui,
        &GridContextMenuHost {
            app,
            ds,
            tokens: render_ctx.tokens,
            blocked: rctx.input_blocked(state),
            record_inspect_meta: state.record_inspect_meta,
            selected_x_cell: None,
        },
        &mut state.context_menu,
        meta,
        intents,
    );
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
        if blocked {
            ui.disable();
        }
        let scroll_out = egui::ScrollArea::new([geom.need_h, geom.need_v])
            .id_salt(("column_grid_scroll", rctx.ds.id.as_str()))
            .auto_shrink([false, false])
            .animated(false)
            .scroll_bar_visibility(if blocked { ScrollBarVisibility::AlwaysHidden } else { ScrollBarVisibility::AlwaysVisible })
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

struct CellDrawInput<'a> {
    coord: GridCellCoord,
    cell_rect: Rect,
    clipped_rect: Rect,
    cell_opt: Option<&'a Cell>,
    is_active: bool,
    is_selected: bool,
    linked_reg: Option<&'a LinkedDataRegion>,
    blocked: bool,
    table_decimals: Option<u8>,
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
    let table_decimals = table_display_decimals(ui.ctx());
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
                let widget_id = format!("grid.cell.{row}.{col}.{subcol}");
                let handled = render_inline_text_edit(
                    ui,
                    rctx,
                    state,
                    meta,
                    intents,
                    InlineTextEditSpec {
                        rect: clipped_rect,
                        editor_id: rctx.sheet_id_base.with(("cell_edit", row, col, subcol)),
                        hint: &hint,
                        widget_id: &widget_id,
                    },
                );
                inline_edit_handled_key |= handled;
                cell_clicked_or_edited = true;
                continue;
            }

            let draw = CellDrawInput { coord, cell_rect, clipped_rect, cell_opt, is_active, is_selected, linked_reg, blocked, table_decimals };
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
        let broken = is_linked_region_broken(rctx.app.document(), reg);
        let lc = if broken { tokens.link_red } else { link_border_color32(reg.border_color(), tokens) };
        ui.painter().rect_filled(draw.clipped_rect, CornerRadius::ZERO, lc.gamma_multiply(0.08));
        paint_linked_perimeter_edges(
            ui,
            draw.cell_rect,
            draw.clipped_rect,
            &LinkedEdgePaint { reg, coord, subs_per_col: rctx.geom.subs_per_col, color: lc, broken },
        );
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
        format_cell_into_with_decimals(cell, &rctx.app.locale_preset, draw.table_decimals, text_buf);
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
        } else if handle_column_cell_pointer(ui, rctx, state, intents, &resp, coord) {
            interacted = true;
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
                    CellTextStyle::plain(FontId::new(tokens.metrics.font_ui_small_pt, FontFamily::Monospace), tokens.text_muted, Align::RIGHT),
                );
                let acc = rn_text.clone();
                rn_resp.widget_info(move || WidgetInfo::labeled(egui::Role::RowHeader, !blocked, acc.clone()));
                if rn_resp.clicked() && !blocked && geom.total_cols > 0 {
                    intents.push_select_grid_cell(GridCellCoord::new(row, 0, 0));
                    intents.push_extend_grid_selection(GridCellCoord::new(row, last_col, last_sub));
                    set_grid_focus_active(ui, true);
                    ui.ctx().request_repaint();
                }
                if rn_resp.secondary_clicked()
                    && !blocked
                    && let Some(pos) = rn_resp.interact_pointer_pos()
                {
                    state.context_menu = Some(ContextMenuState { pos, row, cell: None });
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
            let rt_raw = Rect::from_min_size(pos2(rt_x, row_y), vec2(geom.rt_w, geom.row_h));
            let rt_rect = rt_raw.intersect(geom.gutter_body_rect);
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
                let hint = format!("Row {} Title", row + 1);
                let widget_id = format!("grid.row_title.{row}");
                let _ = render_inline_text_edit(
                    ui,
                    rctx,
                    state,
                    meta,
                    intents,
                    InlineTextEditSpec {
                        rect: rt_rect,
                        editor_id: rctx.sheet_id_base.with(("row_title_edit", row)),
                        hint: &hint,
                        widget_id: &widget_id,
                    },
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
                } else if rt_resp.secondary_clicked()
                    && let Some(pos) = rt_resp.interact_pointer_pos()
                {
                    state.context_menu = Some(ContextMenuState { pos, row, cell: None });
                }
            }
            if state.record_inspect_meta && !ui.ctx().will_discard() {
                record_meta_with_unclipped_if_enabled(
                    ui,
                    meta,
                    true,
                    &rt_resp,
                    rt_raw,
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

fn handle_column_cell_pointer(
    ui: &Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    intents: &mut PendingUiIntents,
    resp: &egui::Response,
    coord: GridCellCoord,
) -> bool {
    if resp.clicked() {
        set_grid_focus_active(ui, true);
        if ui.input(|i| i.modifiers.shift) {
            intents.push_extend_grid_selection(coord);
        } else {
            intents.push_select_grid_cell(coord);
        }
        ui.ctx().request_repaint();
        return true;
    }
    if resp.secondary_clicked() {
        set_grid_focus_active(ui, true);
        let already_selected = rctx.app.grid.selected_coords.contains(&coord) || rctx.app.grid.tool.active == Some(coord);
        if !already_selected {
            intents.push_select_grid_cell(coord);
        }
        if let Some(pos) = resp.interact_pointer_pos() {
            state.context_menu = Some(ContextMenuState {
                pos,
                row: coord.row,
                cell: Some(ContextCellTarget { is_x: false, row: coord.row, col: coord.col, subcol: coord.subcol }),
            });
        }
        ui.ctx().request_repaint();
        return true;
    }
    if handle_cell_pointer_drag_select(ui, &rctx.ds.id, coord, resp, intents) {
        ui.ctx().request_repaint();
        return true;
    }
    false
}
