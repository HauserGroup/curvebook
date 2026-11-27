//! Unified multi-archetype data grid (`Column`, `XY`, `Grouped`, `Contingency`, `Survival`,
//! `PartsOfWhole`, `MultipleVariables`, `Nested`) body, gutter, cell rendering, and context menu (`R15.2`, `R15.4`).

use std::fmt::Write as _;
use std::ops::Range;

use curvebook_doc::{Cell, CellCoord, CellValue, DataSheet, LinkedDataRegion, SheetId, TableArchetype, VariableRole, VariableSource, XColumnMode};
use curvebook_tools::GridCellCoord;
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::{Align, Color32, FontId, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::linked_region_for_cell;
use crate::inspect::ZLayer;

use super::super::ui_kit::{ActionMenuEntry, ActionMenuItem, ActionMenuSpec, TooltipSpec, action_menu_popup, show_tooltip_for_control};
use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens, UiIntent};
use super::headers::{MultiColHeaderView, is_factor_title_editing, render_multi_column_headers, render_multi_corner, row_title_at};
use super::inline_edit::{begin_inline_edit, render_multi_inline_editors};
use super::linked_regions::{open_link_popover, render_link_provenance_strip, render_linked_region_guard_popover};
use super::mv_roles::{active_mv_formula_operands, render_mv_role_menu_popup};
use super::shared::LinkedEdgePaint;
use super::shared::{
    AuxInlineTarget, CellTextStyle, ContextCellTarget, ContextMenuState, GridDragOutcome, MultiGridAuxState, MultiGridCtx, MultiGridFrameState,
    MultiGridGeom, SelectionFastLookup, alt_row_fill, any_pointer_click_in, apply_pending_scroll_target, clamp_multi_scroll, compute_multi_geom,
    compute_multi_scrollbar_rects, compute_visible_range, ensure_solid_scrollbar_style, format_cell_into_with_decimals, forward_frozen_wheel,
    initial_edit_text_for_cell, is_linked_region_broken, is_multi_click_edit, link_border_color32, multi_text_style, paint_exclusion_corner_marker,
    paint_find_highlight, paint_linked_perimeter_edges, paint_scrollbar_corner_cap, paint_selection_perimeter_overlay,
    prepare_grid_drag_and_autoscroll, record_meta_if_enabled, record_meta_with_unclipped_if_enabled, render_ellipsized_text, set_grid_focus_active,
    table_display_decimals,
};
use super::shortcuts::{
    clear_x_clipboard_cache, copy_x_range_to_clipboard, first_linked_region_in_selection, handle_multi_keyboard_shortcuts,
    push_exclude_selection_intent, push_x_cell_text_paste_intents, push_x_range_clear_intents, push_x_range_exclude_intent,
    read_clipboard_text_for_x_paste, resolve_grid_selection,
};
use super::{DataGridEguiState, DataGridFrameReport, DataGridRenderContext, GridInlineEditTarget};

pub(super) fn render_multi_archetype_grid(
    ui: &mut Ui,
    app: &UiApp,
    target_sheet_id: Option<&SheetId>,
    state: &mut DataGridEguiState,
    render_ctx: DataGridRenderContext<'_>,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> DataGridFrameReport {
    let available = ui.available_rect_before_wrap();
    state.ellipsized_headers = 0;

    let Some(ds) = target_sheet_id.and_then(|id| app.document().data_sheet(id)) else {
        state.last_visible_rows = 0..0;
        state.last_visible_subcols = 0..0;
        return DataGridFrameReport {
            grid_rect_pt: available,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            ..DataGridFrameReport::default()
        };
    };

    ensure_solid_scrollbar_style(ui, render_ctx.metrics);
    if let Some(collapsed_override) = app.grid.row_titles_collapsed_override {
        state.row_titles_collapsed = collapsed_override;
    }
    if !available.is_positive() {
        return DataGridFrameReport { grid_rect_pt: available, ..DataGridFrameReport::default() };
    }

    let last_sheet_key = Id::unique(("r15_4_multi_grid_last_sheet", render_ctx.pane_salt));
    let prev_sheet = ui.ctx().data_mut(|d| {
        let prev = d.get_temp::<SheetId>(last_sheet_key);
        d.insert_temp(last_sheet_key, ds.id.clone());
        prev
    });
    let aux_id = Id::unique(("r15_4_multi_grid_aux", render_ctx.pane_salt, ds.id.as_str()));
    let mut aux: MultiGridAuxState = ui.ctx().data_mut(|d| d.get_temp::<MultiGridAuxState>(aux_id).unwrap_or_default());
    if aux.sheet_id.as_ref() != Some(&ds.id) {
        aux = MultiGridAuxState { sheet_id: Some(ds.id.clone()), ..MultiGridAuxState::default() };
    }
    if prev_sheet.as_ref().is_some_and(|prev| prev != &ds.id) {
        state.inline_edit = None;
        state.active_link_popover = None;
        state.context_menu = None;
    }
    let cur_app_y_sel = app.grid.selection_belongs_to(&ds.id).then_some((app.grid.tool.active, app.grid.tool.anchor, app.grid.selected_coords.len()));
    let app_x_cell = app.grid.active_x_cell_for_sheet(&ds.id);
    if app_x_cell != aux.last_app_x_cell {
        if let Some(cell) = app_x_cell {
            if aux.selected_x_cell != Some(cell) {
                aux.set_single_x(cell);
            }
        } else {
            aux.clear_x_selection();
        }
        aux.last_app_x_cell = app_x_cell;
    }
    if cur_app_y_sel != aux.last_app_y_selection {
        aux.last_app_y_selection = cur_app_y_sel;
        if app.grid.active_cell_for_sheet(&ds.id).is_some() && app_x_cell.is_none() {
            aux.clear_x_selection();
        }
    }

    let scroll_allocated_w = ui.spacing().scroll.allocated_width();
    let geom = compute_multi_geom(app, ds, state, render_ctx.metrics, scroll_allocated_w, available);
    if geom.body_inner_clip.width() < 16.0 || geom.body_inner_clip.height() < 16.0 {
        state.last_visible_rows = 0..0;
        state.last_visible_subcols = 0..0;
        return DataGridFrameReport {
            grid_rect_pt: available,
            rendered_modal_guard_open: state.active_link_popover.is_some(),
            ..DataGridFrameReport::default()
        };
    }
    apply_pending_scroll_target(state, &geom);
    clamp_multi_scroll(state, &geom);

    let blocked = render_ctx.modal_blocked || state.active_link_popover.is_some();
    let outside_click_blocked = render_ctx.modal_blocked && any_pointer_click_in(ui, available);
    let tokens = render_ctx.tokens;

    let painter = ui.painter();
    painter.rect_filled(geom.grid_outer_rect, 0.0, tokens.surface_bg);
    painter.rect_stroke(geom.grid_outer_rect, 0.0, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    let mctx = MultiGridCtx {
        app,
        ds,
        geom: &geom,
        tokens,
        metrics: render_ctx.metrics,
        blocked,
        pane_focused: render_ctx.pane_focused,
        pane_salt: render_ctx.pane_salt,
        locale: tokens.locale,
    };
    let factor_title_editing = is_factor_title_editing(ui.ctx(), &ds.id);
    let (body_out, link_outside_click) = {
        let mut fstate = MultiGridFrameState { state, aux: &mut aux, meta, intents };
        if let Some(strip_rect) = geom.link_strip_rect {
            render_link_provenance_strip(ui, &mctx, &mut fstate, strip_rect);
        }
        if !blocked {
            forward_frozen_wheel(ui, &geom, fstate.state);
            clamp_multi_scroll(fstate.state, &geom);
        }
        let drag_outcome = prepare_grid_drag_and_autoscroll(ui, &mctx, &mut fstate, factor_title_editing);
        let prev_scroll = fstate.state.scroll_offset_pt;
        fstate.state.last_available_rect = geom.available_rect;
        let body_out = render_multi_body(ui, &mctx, &mut fstate, &drag_outcome);
        render_multi_corner(ui, &mctx, &mut fstate);
        let active_mv_operands = active_mv_formula_operands(app, ds);
        let header_sel = SelectionFastLookup::new_with_preview(app, &ds.id, &geom, drag_outcome.preview_data_range);
        render_multi_column_headers(
            ui,
            &mctx,
            &mut fstate,
            &MultiColHeaderView {
                scroll_x: body_out.used_scroll_offset.x,
                vis_flat_hint: &body_out.vis_all_flat,
                active_mv_operands: &active_mv_operands,
                sel: &header_sel,
                drag_outcome: &drag_outcome,
            },
        );
        render_multi_gutter_body(ui, &mctx, &mut fstate, body_out.used_scroll_offset.y, &body_out.vis_rows, &header_sel, &drag_outcome);
        paint_scrollbar_corner_cap(ui, &geom, tokens);

        let inline_handled = render_multi_inline_editors(ui, &mctx, &mut fstate, body_out.used_scroll_offset);
        render_mv_role_menu_popup(ui, &mctx, &mut fstate);
        render_row_context_menu_popup(ui, &mctx, &mut fstate);
        if !blocked && !inline_handled && !body_out.cell_clicked_or_edited {
            handle_multi_keyboard_shortcuts(ui, &mctx, &mut fstate);
        }

        if !blocked {
            fstate.state.scroll_offset_pt = body_out.new_scroll_offset;
            clamp_multi_scroll(fstate.state, &geom);
        }
        let scrolled_this_frame = (fstate.state.scroll_offset_pt - prev_scroll).length_sq() > 0.01;
        if scrolled_this_frame {
            if fstate.intents.is_empty() && !ui.ctx().will_discard() {
                ui.ctx().request_discard("data grid scroll offset updated");
            } else {
                ui.ctx().request_repaint();
            }
        }
        let link_outside_click = render_linked_region_guard_popover(ui, &mctx, &mut fstate);
        (body_out, link_outside_click)
    };

    let (v_sb, h_sb) = compute_multi_scrollbar_rects(ui, &geom, body_out.inner_rect);
    state.last_visible_rows = body_out.vis_rows.clone();
    state.last_visible_subcols = body_out.vis_all_flat.clone();
    ui.ctx().data_mut(|d| d.insert_temp(aux_id, aux));

    DataGridFrameReport {
        grid_rect_pt: available,
        rendered_cells: body_out.rendered_cells,
        visible_rows: body_out.vis_rows,
        visible_subcols: body_out.vis_all_flat,
        vertical_scrollbar_rect_pt: v_sb,
        horizontal_scrollbar_rect_pt: h_sb,
        rendered_modal_guard_open: state.active_link_popover.is_some(),
        outside_click_blocked: outside_click_blocked || link_outside_click,
    }
}

fn render_multi_gutter_body(
    ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    scroll_y: f32,
    vis_rows_hint: &Range<usize>,
    sel: &SelectionFastLookup<'_>,
    drag_outcome: &GridDragOutcome,
) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let g_rect = geom.row_gutter_rect;
    if !g_rect.is_positive() {
        return;
    }
    let vis_rows =
        if vis_rows_hint.is_empty() { compute_visible_range(scroll_y, g_rect.height(), geom.row_h, geom.row_count) } else { vis_rows_hint.clone() };
    let x0 = g_rect.min.x;
    let x_mid = x0 + geom.row_num_w;
    let x1 = g_rect.max.x;
    let last_col = geom.col_count.saturating_sub(1);
    let last_sub = geom.col_span_flat(last_col).1.saturating_sub(1);

    ui.scope_builder(UiBuilder::new().max_rect(g_rect), |gui| {
        gui.set_clip_rect(g_rect);
        gui.painter().rect_filled(g_rect, 0.0, tokens.panel_bg);
        for row in vis_rows {
            #[allow(clippy::cast_precision_loss)]
            let y0 = g_rect.min.y + (row as f32) * geom.row_h - scroll_y;
            let y1 = y0 + geom.row_h;
            let row_selected = fstate.aux.selected_x_bounds().map_or_else(|| sel.contains_row(row), |(xr0, xr1, _, _)| (xr0..=xr1).contains(&row));
            let num_r = Rect::from_min_max(pos2(x0, y0), pos2(x_mid, y1)).intersect(g_rect);
            if num_r.height() > 1.0 {
                if row_selected {
                    let row_fill = if mctx.pane_focused { tokens.active_bg } else { tokens.header_bg };
                    gui.painter().rect_filled(num_r, 0.0, row_fill);
                }
                let wid = format!("grid.row_num.{row}");
                let resp = gui.interact(num_r, gui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click_and_drag() });
                let row_str = format!("{}", row + 1);
                let row_text_col = if row_selected && mctx.pane_focused { tokens.text_strong } else { tokens.text_muted };
                render_ellipsized_text(
                    gui,
                    num_r.shrink2(vec2(4.0, 0.0)),
                    &row_str,
                    multi_text_style(tokens.metrics.font_ui_small_pt, true, row_text_col, Align::RIGHT, false, false),
                );
                let acc_rn = format!("Row {}", row + 1);
                resp.widget_info(move || WidgetInfo::labeled(egui::Role::RowHeader, !blocked, acc_rn.clone()));
                record_meta_if_enabled(
                    gui,
                    fstate.meta,
                    fstate.state.record_inspect_meta,
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
                if !drag_outcome.suppress_click_this_frame && !drag_outcome.press_selected_data && resp.clicked() && !blocked && geom.col_count > 0 {
                    fstate.aux.clear_x_selection();
                    let shift_held = gui.input(|i| i.modifiers.shift);
                    if shift_held {
                        let anchor_row = app.grid.tool.anchor.map_or(row, |a| a.row).min(geom.row_count.saturating_sub(1));
                        fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord: GridCellCoord::new(anchor_row, 0, 0) });
                        fstate.intents.push(UiIntent::ExtendGridSelection {
                            sheet_id: Some(ds.id.clone()),
                            coord: GridCellCoord::new(row, last_col, last_sub),
                        });
                    } else {
                        fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord: GridCellCoord::new(row, 0, 0) });
                        fstate.intents.push(UiIntent::ExtendGridSelection {
                            sheet_id: Some(ds.id.clone()),
                            coord: GridCellCoord::new(row, last_col, last_sub),
                        });
                    }
                    set_grid_focus_active(gui, true);
                    gui.ctx().request_repaint();
                }
                if resp.secondary_clicked()
                    && !blocked
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    fstate.state.context_menu = Some(ContextMenuState { pos, row, cell: None });
                }
            }

            let rt_raw = Rect::from_min_max(pos2(x_mid, y0), pos2(x1, y1));
            let rt_r = rt_raw.intersect(g_rect);
            if rt_r.height() > 1.0 {
                let row_title = row_title_at(&ds.archetype, row);
                let rt_find_active = fstate.state.active_find_matches.as_ref().is_some_and(|h| h.active_row_title == Some(row));
                let rt_find_match = rt_find_active || fstate.state.active_find_matches.as_ref().is_some_and(|h| h.matching_row_titles.contains(&row));
                if rt_find_match {
                    paint_find_highlight(gui, rt_r, tokens, rt_find_active);
                }
                if geom.row_titles_collapsed {
                    let wid = format!("grid.row_title_collapsed.{row}");
                    let resp = gui.interact(rt_r, gui.make_persistent_id(&wid), Sense::hover());
                    let abbr: String = row_title.chars().take(2).collect();
                    render_ellipsized_text(
                        gui,
                        rt_r.shrink2(vec2(3.0, 0.0)),
                        &abbr,
                        CellTextStyle::plain(FontId::proportional(tokens.metrics.font_ui_small_pt), tokens.text_muted, Align::Center),
                    );
                    let acc_c = format!("Row {} Title {}", row + 1, row_title);
                    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Label, true, acc_c.clone()));
                    record_meta_if_enabled(
                        gui,
                        fstate.meta,
                        fstate.state.record_inspect_meta,
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
                    let editing_this = fstate.state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::RowTitle { row });
                    if !editing_this {
                        let rt_editable = !blocked && !ds.frozen;
                        let resp = gui.interact(rt_r, gui.make_persistent_id(&wid), if rt_editable { Sense::click() } else { Sense::hover() });
                        if resp.hovered() && rt_editable {
                            gui.painter().rect_filled(rt_r, 0.0, tokens.hover_bg);
                        }
                        let (rt_elided, _) = render_ellipsized_text(
                            gui,
                            rt_r.shrink2(vec2(5.0, 0.0)),
                            &row_title,
                            CellTextStyle::plain(FontId::proportional(12.0), tokens.text_primary, Align::LEFT),
                        );
                        if rt_elided && resp.hovered() {
                            let _ = resp.clone().on_hover_text(&row_title);
                        }
                        let acc_rt = if row_title.is_empty() { format!("Row {} Title", row + 1) } else { row_title.clone() };
                        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, rt_editable, acc_rt.clone()));
                        record_meta_with_unclipped_if_enabled(
                            gui,
                            fstate.meta,
                            fstate.state.record_inspect_meta,
                            &resp,
                            rt_raw,
                            FrameInspectEntryInput {
                                widget_id: &wid,
                                role: "button",
                                label: &row_title,
                                command_id: Some("table.setRowTitles"),
                                value: Some(serde_json::Value::String(row_title.clone())),
                                enabled: rt_editable,
                                disabled_reason: if blocked {
                                    Some("modal dialog is active")
                                } else if ds.frozen {
                                    Some("Data sheet is frozen (read-only).")
                                } else {
                                    None
                                },
                                z_layer: ZLayer::Workspace,
                                ellipsized: rt_elided,
                                full_label: rt_elided.then_some(row_title.as_str()),
                            },
                        );
                        if resp.clicked() && rt_editable {
                            fstate.aux.aux_edit = None;
                            begin_inline_edit(gui, fstate.state, GridInlineEditTarget::RowTitle { row }, row_title);
                        } else if resp.secondary_clicked()
                            && !blocked
                            && let Some(pos) = resp.interact_pointer_pos()
                        {
                            fstate.state.context_menu = Some(ContextMenuState { pos, row, cell: None });
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
    inner_rect: Rect,
    rendered_cells: usize,
    vis_rows: Range<usize>,
    vis_all_flat: Range<usize>,
    used_scroll_offset: Vec2,
    new_scroll_offset: Vec2,
    cell_clicked_or_edited: bool,
}

struct MultiBodyDrawBuffers<'a> {
    text_buf: &'a mut String,
    id_buf: &'a mut String,
    acc_buf: &'a mut String,
    sel: &'a SelectionFastLookup<'a>,
    has_links: bool,
    record_meta: bool,
    table_decimals: Option<u8>,
    base_row_bg: Color32,
    x_seq_readonly: bool,
    is_mv: bool,
    is_cont: bool,
    is_surv: bool,
    blocked: bool,
}

#[derive(Clone, Copy)]
struct MultiXCellDraw {
    row: usize,
    subcol: usize,
    clipped: Rect,
}

#[derive(Clone, Copy)]
struct MultiDataCellDraw {
    coord: GridCellCoord,
    flat_data_col: usize,
    raw_cell_r: Rect,
    clipped: Rect,
}

fn render_multi_body(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, drag_outcome: &GridDragOutcome) -> MultiBodyOut {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let table_decimals = table_display_decimals(ui.ctx());
    let b_rect = geom.body_rect;
    let scroll_offset = fstate.state.scroll_offset_pt;
    if !b_rect.is_positive() {
        return MultiBodyOut {
            inner_rect: geom.body_inner_clip,
            rendered_cells: 0,
            vis_rows: 0..0,
            vis_all_flat: 0..0,
            used_scroll_offset: scroll_offset,
            new_scroll_offset: scroll_offset,
            cell_clicked_or_edited: false,
        };
    }
    let mut out = MultiBodyOut {
        inner_rect: geom.body_inner_clip,
        rendered_cells: 0,
        vis_rows: 0..0,
        vis_all_flat: 0..0,
        used_scroll_offset: scroll_offset,
        new_scroll_offset: scroll_offset,
        cell_clicked_or_edited: drag_outcome.press_selected_data || drag_outcome.press_selected_x,
    };

    let scroll_out =
        ui.scope_builder(UiBuilder::new().id_salt(("r15_4_multi_grid_body", mctx.pane_salt, ds.id.as_str())).max_rect(b_rect), |body_parent| {
            if blocked {
                body_parent.disable();
            }
            egui::ScrollArea::new([geom.need_h, geom.need_v])
                .id_salt(("r15_4_multi_grid_scroll", mctx.pane_salt, ds.id.as_str()))
                .scroll_offset(scroll_offset)
                .animated(false)
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
                    out.used_scroll_offset = viewport.min.to_vec2();

                    let vis_rows = compute_visible_range(viewport.min.y, geom.body_inner_clip.height(), geom.row_h, geom.row_count);
                    let vis_all_flat = compute_visible_range(viewport.min.x, geom.body_inner_clip.width(), geom.subcol_w, geom.total_flat_cols);
                    out.vis_rows = vis_rows.clone();
                    out.vis_all_flat = vis_all_flat.clone();

                    let sel = SelectionFastLookup::new_with_preview(mctx.app, &mctx.ds.id, geom, drag_outcome.preview_data_range);
                    let has_links = !ds.linked_regions.is_empty();
                    let mut text_buf = String::with_capacity(32);
                    let mut id_buf = String::with_capacity(32);
                    let mut acc_buf = String::with_capacity(48);
                    let record_meta = fstate.state.record_inspect_meta && !body_ui.ctx().will_discard();

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
                            sel: &sel,
                            has_links,
                            record_meta,
                            table_decimals,
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
                                if render_single_multi_x_cell(
                                    body_ui,
                                    mctx,
                                    fstate,
                                    MultiXCellDraw { row, subcol: flat, clipped },
                                    &mut dbuf,
                                    drag_outcome,
                                ) {
                                    out.cell_clicked_or_edited = true;
                                }
                                out.rendered_cells += 1;
                            } else if let Some((col, subcol)) = geom.flat_data_to_coord(flat - geom.x_subcols) {
                                let coord = GridCellCoord::new(row, col, subcol);
                                if render_single_multi_data_cell(
                                    body_ui,
                                    mctx,
                                    fstate,
                                    MultiDataCellDraw { coord, flat_data_col: flat - geom.x_subcols, raw_cell_r, clipped },
                                    &mut dbuf,
                                    drag_outcome,
                                ) {
                                    out.cell_clicked_or_edited = true;
                                }
                                out.rendered_cells += 1;
                            }
                        }

                        body_ui.painter().line_segment(
                            [pos2(geom.body_inner_clip.min.x, y1), pos2(geom.body_inner_clip.max.x, y1)],
                            Stroke::new(1.0, tokens.divider),
                        );
                    }

                    paint_multi_vertical_hairlines(body_ui, origin.x, geom, &vis_all_flat, tokens);
                    paint_selection_perimeter_overlay(body_ui, mctx, origin, &vis_rows, &vis_all_flat, &sel, fstate.aux);
                })
        });
    out.inner_rect = scroll_out.inner.inner_rect;
    out.new_scroll_offset = scroll_out.inner.state.clamped_offset();
    out
}

fn render_single_multi_x_cell(
    body_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    cell: MultiXCellDraw,
    dbuf: &mut MultiBodyDrawBuffers<'_>,
    drag_outcome: &GridDragOutcome,
) -> bool {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let MultiXCellDraw { row, subcol, clipped } = cell;
    let editing_this = fstate.aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::XCell { row, subcol });
    if editing_this {
        return true;
    }
    let cell_opt = ds.archetype.x_or_time_cell(row, subcol);
    dbuf.text_buf.clear();
    if let Some(c) = cell_opt {
        format_cell_into_with_decimals(c, &app.locale_preset, dbuf.table_decimals, dbuf.text_buf);
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
        format_cell_into_with_decimals(&Cell::number(seq_val), &app.locale_preset, dbuf.table_decimals, dbuf.text_buf);
    }
    let excluded = cell_opt.is_some_and(|c| c.excluded);
    let editable = !dbuf.blocked && !ds.frozen && !dbuf.x_seq_readonly;
    let is_active = fstate.aux.selected_x_cell == Some((row, subcol));
    let is_selected = is_active || fstate.aux.x_contains(row, subcol);

    let base_fill = if dbuf.x_seq_readonly { tokens.panel_bg } else { dbuf.base_row_bg };
    body_ui.painter().rect_filled(clipped, 0.0, base_fill);
    if is_active {
        let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.selection_unemphasized_bg };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
    } else if is_selected {
        let fill = if mctx.pane_focused { tokens.active_bg.linear_multiply(0.72) } else { tokens.selection_unemphasized_bg };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
    }
    let x_find_coord = GridCellCoord::new(row, crate::egui_shell::sheet_find_bar::X_OR_TIME_COL_SENTINEL, subcol);
    let x_find_active = fstate.state.active_find_matches.as_ref().is_some_and(|h| h.active_cell == Some(x_find_coord));
    let x_find_match = x_find_active || fstate.state.active_find_matches.as_ref().is_some_and(|h| h.matching_cells.contains(&x_find_coord));
    if x_find_match {
        paint_find_highlight(body_ui, clipped, tokens, x_find_active);
    }

    let mut x_elided = false;
    if !dbuf.text_buf.is_empty() {
        let text_col = if excluded {
            tokens.excluded_cell_text
        } else if dbuf.x_seq_readonly {
            tokens.text_muted
        } else {
            tokens.text_primary
        };
        let (was_elided, _) = render_ellipsized_text(
            body_ui,
            clipped.shrink2(vec2(5.0, 1.0)),
            dbuf.text_buf,
            multi_text_style(geom.font_num_pt, true, text_col, Align::RIGHT, excluded || dbuf.x_seq_readonly, excluded),
        );
        x_elided = was_elided;
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
        fstate.meta.record_response(
            body_ui,
            &resp,
            FrameInspectEntryInput {
                widget_id: dbuf.id_buf,
                role: "gridcell",
                label: dbuf.acc_buf,
                command_id: Some("table.setXCells"),
                value: Some(json!({
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

    if !dbuf.blocked && resp.hovered() && (excluded || x_elided) {
        let elided_text = (x_elided && !dbuf.text_buf.is_empty()).then_some(dbuf.text_buf.as_str());
        if let Some(tip) = super::shared::build_data_cell_tooltip(ds, x_find_coord, excluded, false, elided_text, mctx.locale) {
            let mut spec = TooltipSpec::new(&tip.title);
            if let Some(body) = tip.summary.as_deref() {
                spec = spec.with_summary(body);
            }
            let _ = show_tooltip_for_control(&resp, tokens, dbuf.text_buf, x_elided, spec);
        }
    }

    let mut interacted = false;
    if !dbuf.blocked && !drag_outcome.suppress_click_this_frame && resp.clicked() {
        interacted = true;
        if !drag_outcome.press_selected_x {
            let shift_held = body_ui.input(|i| i.modifiers.shift);
            if shift_held {
                fstate.aux.extend_x_selection((row, subcol));
            } else {
                fstate.aux.set_single_x((row, subcol));
            }
        }
        super::shared::push_select_x_if_needed(app, row, subcol, fstate.intents);
        fstate.aux.active_role_menu_col = None;
        fstate.state.context_menu = None;
        set_grid_focus_active(body_ui, true);
        body_ui.ctx().request_repaint();
        if is_multi_click_edit(&resp) && editable {
            fstate.state.inline_edit = None;
            fstate.aux.aux_edit = Some((AuxInlineTarget::XCell { row, subcol }, initial_edit_text_for_cell(cell_opt, &app.locale_preset), true));
        }
    }
    if !dbuf.blocked
        && resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        interacted = true;
        if !fstate.aux.x_contains(row, subcol) {
            fstate.aux.set_single_x((row, subcol));
            super::shared::push_select_x_if_needed(app, row, subcol, fstate.intents);
        } else if app.grid.active_cell_for_sheet(&ds.id).is_some() || app.grid.active_x_cell_for_sheet(&ds.id).is_none() {
            let (xr, xs) = fstate.aux.selected_x_cell.unwrap_or((row, subcol));
            super::shared::push_select_x_if_needed(app, xr, xs, fstate.intents);
        }
        body_ui.ctx().request_repaint();
        fstate.state.context_menu = Some(ContextMenuState { pos, row, cell: Some(ContextCellTarget { is_x: true, row, col: 0, subcol }) });
    }
    interacted
}

struct MultiCellFlags<'a> {
    cell_opt: Option<&'a Cell>,
    excluded: bool,
    elided: bool,
    mv_is_cat_or_lbl: bool,
    mv_is_calc: bool,
    linked: Option<&'a LinkedDataRegion>,
    editable: bool,
    is_active: bool,
    is_selected: bool,
    contingency_invalid: bool,
    find_match: bool,
    find_active: bool,
}

fn render_single_multi_data_cell(
    body_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    cell: MultiDataCellDraw,
    dbuf: &mut MultiBodyDrawBuffers<'_>,
    drag_outcome: &GridDragOutcome,
) -> bool {
    let app = mctx.app;
    let ds = mctx.ds;
    let coord = cell.coord;
    let editing_this = fstate.state.inline_edit.as_ref().is_some_and(|e| e.target == GridInlineEditTarget::Cell(coord));
    if editing_this {
        return true;
    }
    let cell_opt = ds.archetype.data_cell(CellCoord::from(coord));
    dbuf.text_buf.clear();
    if let Some(c) = cell_opt {
        format_cell_into_with_decimals(c, &app.locale_preset, dbuf.table_decimals, dbuf.text_buf);
    }
    let mv_var =
        if dbuf.is_mv { if let TableArchetype::MultipleVariables(mv) = &ds.archetype { mv.variables.get(coord.col) } else { None } } else { None };
    let mv_is_calc = mv_var.is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }));
    let mv_is_cat_or_lbl = mv_var.is_some_and(|v| matches!(v.role, VariableRole::Categorical | VariableRole::Label));
    let linked = if dbuf.has_links { linked_region_for_cell(ds, coord.row, coord.col) } else { None };
    let is_active = fstate.aux.selected_x_cell.is_none() && dbuf.sel.active == Some(coord);
    let is_selected = is_active || (fstate.aux.selected_x_cell.is_none() && dbuf.sel.contains(coord, cell.flat_data_col));
    let find_active = fstate.state.active_find_matches.as_ref().is_some_and(|h| h.active_cell == Some(coord));
    let find_match = find_active || fstate.state.active_find_matches.as_ref().is_some_and(|h| h.matching_cells.contains(&coord));
    let mut flags = MultiCellFlags {
        cell_opt,
        excluded: cell_opt.is_some_and(|c| c.excluded),
        elided: false,
        mv_is_cat_or_lbl,
        mv_is_calc,
        linked,
        editable: !dbuf.blocked && !ds.frozen && !mv_is_calc,
        is_active,
        is_selected,
        contingency_invalid: dbuf.is_cont && is_invalid_contingency_cell(cell_opt),
        find_match,
        find_active,
    };

    flags.elided = paint_single_multi_data_cell(body_ui, mctx, cell, dbuf, &flags);

    dbuf.id_buf.clear();
    let _ = write!(dbuf.id_buf, "grid.cell.{}.{}.{}", coord.row, coord.col, coord.subcol);
    let resp = body_ui.interact(
        cell.clipped,
        body_ui.make_persistent_id(&*dbuf.id_buf),
        if dbuf.blocked { Sense::hover() } else { Sense::click_and_drag() },
    );
    dbuf.acc_buf.clear();
    if dbuf.text_buf.is_empty() {
        let _ = write!(dbuf.acc_buf, "R{}C{}", coord.row + 1, coord.col + 1);
    } else {
        let _ = write!(dbuf.acc_buf, "R{}C{} {}", coord.row + 1, coord.col + 1, dbuf.text_buf);
    }
    let acc_owned = if flags.elided && !dbuf.text_buf.is_empty() { dbuf.text_buf.clone() } else { dbuf.acc_buf.clone() };
    let acc_enabled = flags.editable || flags.mv_is_calc;
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Cell, acc_enabled, acc_owned.clone()));

    handle_multi_data_cell_interactions(body_ui, mctx, fstate, coord, dbuf, &flags, &resp, drag_outcome)
}

fn paint_single_multi_data_cell(
    body_ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    cell: MultiDataCellDraw,
    dbuf: &MultiBodyDrawBuffers<'_>,
    flags: &MultiCellFlags<'_>,
) -> bool {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let MultiDataCellDraw { coord, raw_cell_r, clipped, .. } = cell;
    let base_fill = if flags.mv_is_calc {
        tokens.role_fx.linear_multiply(0.06)
    } else if flags.contingency_invalid {
        tokens.link_red.linear_multiply(0.12)
    } else {
        dbuf.base_row_bg
    };
    body_ui.painter().rect_filled(clipped, 0.0, base_fill);
    if flags.is_active {
        let fill = if mctx.pane_focused { tokens.active_bg } else { tokens.selection_unemphasized_bg };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
    } else if flags.is_selected {
        let fill = if mctx.pane_focused { tokens.active_bg.linear_multiply(0.72) } else { tokens.selection_unemphasized_bg };
        body_ui.painter().rect_filled(clipped, 0.0, fill);
    }
    if flags.find_match {
        paint_find_highlight(body_ui, clipped, tokens, flags.find_active);
    }
    if let Some(reg) = flags.linked {
        let (_, span_subs) = geom.col_span_flat(coord.col);
        let broken = is_linked_region_broken(mctx.app.document(), reg);
        let lc = if broken { tokens.link_red } else { link_border_color32(reg.spec.source.border_color(), tokens) };
        body_ui.painter().rect_filled(clipped, 0.0, lc.gamma_multiply(0.08));
        paint_linked_perimeter_edges(body_ui, raw_cell_r, clipped, &LinkedEdgePaint { reg, coord, subs_per_col: span_subs, color: lc, broken });
    }
    if flags.excluded {
        paint_exclusion_corner_marker(body_ui, raw_cell_r, clipped, ds, coord, tokens);
    }

    let mut elided = false;
    if !dbuf.text_buf.is_empty() {
        let is_numeric = matches!(flags.cell_opt.map(|c| &c.value), Some(CellValue::Number(_))) && !flags.mv_is_cat_or_lbl;
        let align = if is_numeric { Align::RIGHT } else { Align::LEFT };
        let text_col = if flags.excluded {
            tokens.excluded_cell_text
        } else if flags.contingency_invalid {
            tokens.link_red
        } else if flags.mv_is_calc {
            tokens.role_fx
        } else if dbuf.is_surv && cell_is_survival_event(ds, flags.cell_opt) {
            tokens.text_strong
        } else {
            tokens.text_primary
        };
        let (was_elided, _) = render_ellipsized_text(
            body_ui,
            clipped.shrink2(vec2(5.0, 1.0)),
            dbuf.text_buf,
            multi_text_style(
                if is_numeric { geom.font_num_pt } else { geom.font_small_pt },
                is_numeric,
                text_col,
                align,
                flags.excluded || flags.mv_is_calc,
                flags.excluded,
            ),
        );
        elided = was_elided;
    } else if flags.excluded {
        let _ = render_ellipsized_text(
            body_ui,
            clipped,
            "×",
            CellTextStyle {
                font_id: FontId::proportional(geom.font_small_pt),
                color: tokens.excluded_cell_text,
                align: Align::Center,
                italic: true,
                strike: false,
            },
        );
    }
    elided
}

#[allow(clippy::too_many_arguments)]
fn handle_multi_data_cell_interactions(
    body_ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    coord: GridCellCoord,
    dbuf: &MultiBodyDrawBuffers<'_>,
    flags: &MultiCellFlags<'_>,
    resp: &Response,
    drag_outcome: &GridDragOutcome,
) -> bool {
    let app = mctx.app;
    let ds = mctx.ds;
    if !dbuf.blocked && resp.hovered() {
        let elided_text = (flags.elided && !dbuf.text_buf.is_empty()).then_some(dbuf.text_buf.as_str());
        if let Some(tip) = super::shared::build_data_cell_tooltip(ds, coord, flags.excluded, flags.contingency_invalid, elided_text, mctx.locale) {
            let mut spec = TooltipSpec::new(&tip.title);
            if let Some(body) = tip.summary.as_deref() {
                spec = spec.with_summary(body);
            }
            let _ = show_tooltip_for_control(resp, mctx.tokens, dbuf.text_buf, flags.elided, spec);
        }
    }

    if dbuf.record_meta {
        let inspect_text = if flags.excluded && !dbuf.text_buf.is_empty() { format!("{}*", dbuf.text_buf) } else { dbuf.text_buf.clone() };
        let source_well = ds.plate_well_origin(CellCoord::from(coord));
        let mut cell_val = json!({
            "row": coord.row,
            "col": coord.col,
            "subcol": coord.subcol,
            "text": inspect_text,
            "excluded": flags.excluded,
            "selected": flags.is_selected,
        });
        if let (Some(well), Some(obj)) = (source_well, cell_val.as_object_mut()) {
            obj.insert(String::from("source_well"), json!(well));
        }
        fstate.meta.record_response(
            body_ui,
            resp,
            FrameInspectEntryInput {
                widget_id: dbuf.id_buf,
                role: "gridcell",
                label: dbuf.acc_buf,
                command_id: Some("table.setCell"),
                value: Some(cell_val),
                enabled: flags.editable,
                disabled_reason: if dbuf.blocked {
                    Some("modal dialog is active")
                } else if flags.mv_is_calc {
                    Some("Calculated variable cells are formula-derived (read-only).")
                } else if ds.frozen {
                    Some("sheet is frozen")
                } else {
                    None
                },
                z_layer: ZLayer::Workspace,
                ellipsized: flags.elided,
                full_label: flags.elided.then_some(dbuf.text_buf.as_str()),
            },
        );
    }

    let mut interacted = false;
    if !dbuf.blocked && !drag_outcome.suppress_click_this_frame && resp.clicked() {
        interacted = true;
        if fstate.aux.selected_x_cell.is_some() {
            fstate.aux.clear_x_selection();
            body_ui.ctx().request_repaint();
        }
        fstate.aux.active_role_menu_col = None;
        fstate.state.context_menu = None;
        set_grid_focus_active(body_ui, true);
        let (shift_held, cmd_held) = body_ui.input(|i| (i.modifiers.shift, i.modifiers.command || i.modifiers.mac_cmd || i.modifiers.ctrl));
        if shift_held {
            fstate.intents.push(UiIntent::ExtendGridSelection { sheet_id: Some(ds.id.clone()), coord });
        } else if cmd_held {
            fstate.intents.push(UiIntent::ToggleGridCell { sheet_id: Some(ds.id.clone()), coord });
        } else if drag_outcome.click_already_selected_data != Some(coord)
            && !(app.grid.active_cell_for_sheet(&ds.id) == Some(coord)
                && app.grid.tool.anchor == Some(coord)
                && app.grid.selected_coords.len() <= 1
                && app.grid.active_x_cell_for_sheet(&ds.id).is_none())
        {
            fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord });
        }
        if is_multi_click_edit(resp) {
            if flags.mv_is_calc {
                fstate.intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": coord.col }));
            } else if let Some(reg) = flags.linked {
                open_link_popover(fstate.state, reg, mctx.locale);
                body_ui.ctx().request_repaint();
            } else if !ds.frozen {
                fstate.aux.aux_edit = None;
                begin_inline_edit(
                    body_ui,
                    fstate.state,
                    GridInlineEditTarget::Cell(coord),
                    initial_edit_text_for_cell(flags.cell_opt, &app.locale_preset),
                );
            }
        }
    }
    if !dbuf.blocked
        && resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        interacted = true;
        fstate.aux.clear_x_selection();
        let already_selected = app.grid.selection_belongs_to(&ds.id) && flags.is_selected;
        if !already_selected {
            fstate.intents.push(UiIntent::SelectGridCell { sheet_id: Some(ds.id.clone()), coord });
        }
        fstate.state.context_menu = Some(ContextMenuState {
            pos,
            row: coord.row,
            cell: Some(ContextCellTarget { is_x: false, row: coord.row, col: coord.col, subcol: coord.subcol }),
        });
    }
    interacted
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
        body_ui.painter().line_segment([pos2(x1, geom.body_inner_clip.min.y), pos2(x1, geom.body_inner_clip.max.y)], Stroke::new(w, tokens.divider));
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

fn find_linked_graph_for_data_sheet<'a>(app: &'a UiApp, ds_id: &curvebook_doc::SheetId) -> Option<&'a curvebook_doc::GraphSheet> {
    let doc = app.document();
    if let Some(active) = doc.active_sheet_id.as_ref().and_then(|id| doc.graph_sheet(id))
        && (active.spec.primary_source_sheet_id == *ds_id
            || doc.result_sheet(&active.spec.primary_source_sheet_id).is_some_and(|r| r.analysis.source_sheet_id == *ds_id))
    {
        return Some(active);
    }
    doc.graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == *ds_id).or_else(|| {
        doc.graph_sheets.iter().find(|g| doc.result_sheet(&g.spec.primary_source_sheet_id).is_some_and(|r| r.analysis.source_sheet_id == *ds_id))
    })
}

fn row_title_or_default(ds: &DataSheet, row: usize) -> String {
    let raw = row_title_at(&ds.archetype, row);
    if raw.trim().is_empty() { format!("Row {}", row + 1) } else { raw }
}

fn render_row_context_menu_popup(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let app = mctx.app;
    let ds = mctx.ds;
    let tokens = mctx.tokens;
    let Some(ctx_menu) = fstate.state.context_menu.clone() else { return };
    let (pos, row, cell_ctx) = (ctx_menu.pos, ctx_menu.row, ctx_menu.cell);
    if mctx.blocked {
        fstate.state.context_menu = None;
        return;
    }
    let selected_x_bounds = fstate.aux.selected_x_bounds();
    let record_inspect_meta = fstate.state.record_inspect_meta;
    let mut close_menu = false;
    let mut linked_guard_to_open: Option<LinkedDataRegion> = None;
    let meta = &mut *fstate.meta;
    let intents = &mut *fstate.intents;

    let linked_graph = find_linked_graph_for_data_sheet(app, &ds.id);
    let has_graph = linked_graph.is_some();
    let row_override_points: Vec<serde_json::Value> = linked_graph
        .map(|g| {
            g.spec
                .point_overrides
                .iter()
                .filter(|ov| ov.point_index == row)
                .map(|ov| match ov.subcol {
                    Some(sc) => json!({ "series_index": ov.series_index, "point_index": ov.point_index, "subcol": sc }),
                    None => json!({ "series_index": ov.series_index, "point_index": ov.point_index }),
                })
                .collect()
        })
        .unwrap_or_default();

    let is_parts_of_whole = matches!(ds.archetype, TableArchetype::PartsOfWhole(_));
    let at_col_cap = ds.archetype.column_count() >= curvebook_doc::MAX_DATA_SETS;
    let right_clicked_x = cell_ctx.is_some_and(|t| t.is_x);
    let target_dup_col = cell_ctx
        .filter(|t| !t.is_x)
        .map(|t| t.col)
        .or_else(|| app.grid.active_cell_for_sheet(&ds.id).map(|c| c.col))
        .unwrap_or(0)
        .min(ds.archetype.column_count().saturating_sub(1));
    let unequal_nested_units = nested_group_has_unequal_units(ds, target_dup_col);
    let dup_enabled = !ds.frozen && !right_clicked_x && ds.archetype.column_count() > 0 && !is_parts_of_whole && !at_col_cap && !unequal_nested_units;
    let dup_disabled_reason = if ds.frozen {
        Some("Data sheet is frozen (read-only).")
    } else if right_clicked_x {
        Some("Right-click a data column cell (not X/Time) to duplicate.")
    } else if is_parts_of_whole {
        Some("Parts-of-Whole tables contain a single column.")
    } else if at_col_cap {
        Some("Maximum data column limit reached.")
    } else if ds.archetype.column_count() == 0 {
        Some("No data columns available to duplicate.")
    } else if unequal_nested_units {
        Some("Nested group biological unit count differs from Group 1 and cannot be duplicated.")
    } else {
        None
    };

    let header_title = format!("Row {} Actions", row + 1);
    type GridCtxItemTuple<'a> = (&'a str, &'a str, Option<&'a str>, bool, Option<&'a str>);
    let items: [GridCtxItemTuple<'_>; 9] = [
        (
            "grid.ctx.style_points",
            "Style Selected Points on Graph",
            Some("ui.formatPoints.open"),
            has_graph,
            (!has_graph).then_some("No graph linked to this data table yet."),
        ),
        (
            "grid.ctx.show_row_titles",
            "Show Row Titles on Graph",
            Some("graph.addAnnotation"),
            has_graph,
            (!has_graph).then_some("No graph linked to this data table yet."),
        ),
        (
            "grid.ctx.reset_style",
            "Reset to Dataset Style",
            Some("graph.formatSelectedPoints"),
            has_graph && !row_override_points.is_empty(),
            if !has_graph {
                Some("No graph linked to this data table yet.")
            } else if row_override_points.is_empty() {
                Some("No per-point style overrides exist on this row.")
            } else {
                None
            },
        ),
        (
            "grid.ctx.exclude",
            "Exclude Selected (Cmd/Ctrl+E)",
            Some("table.excludeCells"),
            !ds.frozen,
            ds.frozen.then_some("Data sheet is frozen (read-only)."),
        ),
        (
            "grid.ctx.exclude_reason",
            "Exclude with Reason",
            Some("ui.grid.excludeWithReason"),
            !ds.frozen,
            ds.frozen.then_some("Data sheet is frozen (read-only)."),
        ),
        ("grid.ctx.cut", "Cut Selection", Some("edit.cut"), !ds.frozen, ds.frozen.then_some("Data sheet is frozen (read-only).")),
        ("grid.ctx.copy", "Copy Selection", Some("edit.copy"), true, None),
        ("grid.ctx.paste", "Paste Here", Some("edit.paste"), !ds.frozen, ds.frozen.then_some("Data sheet is frozen (read-only).")),
        ("grid.ctx.duplicate_column", "Duplicate Column", Some("table.paste"), dup_enabled, dup_disabled_reason),
    ];

    let mut entries = Vec::with_capacity(items.len() + 2);
    entries.push(ActionMenuEntry::Header(&header_title));
    entries.push(ActionMenuEntry::Separator);
    for (wid, label, command_id, enabled, disabled_reason) in items {
        entries.push(ActionMenuEntry::Item(
            ActionMenuItem::menu_item(wid, label).command_id(command_id).enabled(enabled).disabled_reason(disabled_reason),
        ));
    }

    let out = action_menu_popup(
        ui.ctx(),
        tokens,
        record_inspect_meta.then_some(meta),
        ActionMenuSpec::context(Id::unique(("grid_ctx_menu", ds.id.as_str())), "popup.grid.ctx", "Row Actions", pos),
        &entries,
    );

    if let Some(hit) = out.clicked_entry
        && let Some(item_idx) = hit.checked_sub(2)
        && let Some((wid, _, _, enabled, _)) = items.get(item_idx)
        && *enabled
    {
        match *wid {
            "grid.ctx.style_points" => {
                if let Some(g) = linked_graph {
                    let points: Vec<serde_json::Value> = if let Some(tgt) = cell_ctx
                        && !tgt.is_x
                        && tgt.col < g.spec.series.len()
                    {
                        vec![json!({ "series_index": tgt.col, "point_index": row })]
                    } else if !g.spec.series.is_empty() {
                        vec![json!({ "series_index": 0, "point_index": row })]
                    } else {
                        Vec::new()
                    };
                    intents.push_ui("ui.formatPoints.open", json!({ "sheet_id": g.id.as_str(), "points": points }));
                }
            }
            "grid.ctx.show_row_titles" => {
                if let Some(g) = linked_graph {
                    let text = row_title_or_default(ds, row);
                    #[allow(clippy::cast_precision_loss)]
                    let x_pt = 72.0 + f64::from((row % 3) as u32) * 124.0;
                    #[allow(clippy::cast_precision_loss)]
                    let y_pt = 48.0 + f64::from((row / 3) as u32) * 30.0;
                    intents.push_engine(
                        "graph.addAnnotation",
                        json!({
                            "sheet_id": g.id.as_str(),
                            "kind": "text",
                            "text": text,
                            "x_pt": x_pt,
                            "y_pt": y_pt,
                        }),
                    );
                }
            }
            "grid.ctx.reset_style" => {
                if let Some(g) = linked_graph
                    && !row_override_points.is_empty()
                {
                    intents.push_engine(
                        "graph.formatSelectedPoints",
                        json!({
                            "sheet_id": g.id.as_str(),
                            "mode": "clear",
                            "points": row_override_points,
                        }),
                    );
                }
            }
            "grid.ctx.exclude" => {
                if let Some(reg) = handle_grid_ctx_exclude(app, ds, row, cell_ctx, selected_x_bounds, intents) {
                    linked_guard_to_open = Some(reg);
                }
            }
            "grid.ctx.exclude_reason" => intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": ds.id.as_str() })),
            "grid.ctx.cut" => {
                if let Some(reg) = handle_grid_ctx_cut(ui, app, ds, cell_ctx, selected_x_bounds, intents) {
                    linked_guard_to_open = Some(reg);
                }
            }
            "grid.ctx.copy" => {
                if let Some(tgt) = cell_ctx
                    && tgt.is_x
                {
                    if let Some((r0, r1, s0, s1)) = selected_x_bounds
                        && (r0..=r1).contains(&tgt.row)
                        && (s0..=s1).contains(&tgt.subcol)
                    {
                        copy_x_range_to_clipboard(ui, app, ds, r0, r1, s0, s1);
                    } else {
                        copy_x_range_to_clipboard(ui, app, ds, tgt.row, tgt.row, tgt.subcol, tgt.subcol);
                    }
                } else if let Some((xr0, xr1, xs0, xs1)) = selected_x_bounds {
                    copy_x_range_to_clipboard(ui, app, ds, xr0, xr1, xs0, xs1);
                } else {
                    clear_x_clipboard_cache(ui);
                    intents.push(UiIntent::GridCopySelection);
                }
            }
            "grid.ctx.paste" => {
                if let Some(tgt) = cell_ctx
                    && tgt.is_x
                {
                    if let Some(clip) = read_clipboard_text_for_x_paste(ui, app) {
                        push_x_cell_text_paste_intents(app, ds, tgt.row, tgt.subcol, &clip, intents);
                    }
                } else if let Some((xr0, _, xs0, _)) = selected_x_bounds {
                    if let Some(clip) = read_clipboard_text_for_x_paste(ui, app) {
                        push_x_cell_text_paste_intents(app, ds, xr0, xs0, &clip, intents);
                    }
                } else {
                    intents.push(UiIntent::GridPasteFromClipboard { start_coord: app.grid.active_cell_for_sheet(&ds.id) });
                }
            }
            "grid.ctx.duplicate_column" => {
                if let Some(intent) = build_duplicate_column_intent(app, ds, cell_ctx) {
                    intents.push(intent);
                }
            }
            _ => {}
        }
        close_menu = true;
    }

    if close_menu || out.dismissed {
        fstate.state.context_menu = None;
    }
    if let Some(reg) = linked_guard_to_open {
        open_link_popover(fstate.state, &reg, mctx.locale);
        ui.ctx().request_repaint();
    }
}

fn nested_group_has_unequal_units(ds: &DataSheet, src_col: usize) -> bool {
    let TableArchetype::Nested(nt) = &ds.archetype else {
        return false;
    };
    let default_units = nt.groups.first().map_or(1, curvebook_doc::NestedGroupColumn::biological_unit_count).max(1);
    let src_units = nt.groups.get(src_col).map_or(default_units, curvebook_doc::NestedGroupColumn::biological_unit_count).max(1);
    src_units != default_units
}

fn build_duplicate_column_intent(app: &UiApp, ds: &DataSheet, cell_ctx: Option<ContextCellTarget>) -> Option<UiIntent> {
    let col_count = ds.archetype.column_count();
    if col_count == 0 {
        return None;
    }
    let src_col = cell_ctx
        .filter(|t| !t.is_x)
        .map(|t| t.col)
        .or_else(|| app.grid.active_cell_for_sheet(&ds.id).map(|c| c.col))
        .unwrap_or(0)
        .min(col_count.saturating_sub(1));
    build_duplicate_column_intent_for_col(ds, src_col)
}

/// Build an atomic `UiIntent` duplicating `src_col` in `ds` (preserving exclusions, Nested
/// subcolumn titles, and MV calculated/categorical roles in a single Undo step, and refusing
/// unequal-unit Nested groups that `table.paste` cannot append without splitting).
#[must_use]
pub fn build_duplicate_column_intent_for_col(ds: &DataSheet, src_col: usize) -> Option<UiIntent> {
    let col_count = ds.archetype.column_count();
    if ds.frozen
        || col_count == 0
        || src_col >= col_count
        || col_count >= curvebook_doc::MAX_DATA_SETS
        || matches!(ds.archetype, TableArchetype::PartsOfWhole(_))
        || nested_group_has_unequal_units(ds, src_col)
    {
        return None;
    }
    let src_title = crate::dialogs_canvas::data_column_title(ds, src_col)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| curvebook_doc::default_column_label(src_col), str::to_owned);
    let max_base_len = curvebook_doc::MAX_TITLE_BYTES.saturating_sub(" (Copy)".len());
    let truncated_base: String = src_title.char_indices().take_while(|(idx, ch)| idx + ch.len_utf8() <= max_base_len).map(|(_, ch)| ch).collect();
    let new_title = format!("{truncated_base} (Copy)");

    if let TableArchetype::MultipleVariables(mv) = &ds.archetype
        && let Some(var) = mv.variables.get(src_col)
        && let VariableSource::Calculated { formula } = &var.source
    {
        return Some(UiIntent::EngineCommand {
            command_id: String::from("table.setCalculatedVariable"),
            params: json!({
                "sheet_id": ds.id.as_str(),
                "title": new_title,
                "formula": formula,
            }),
        });
    }

    let n_rows = ds.archetype.row_count().max(1);
    let n_sub = crate::grid_view::data_set_subcolumn_count(&ds.archetype, src_col).max(1);
    let mut rows_json = Vec::with_capacity(n_rows);
    let mut excluded_cells = Vec::new();
    for r in 0..n_rows {
        let mut row_vals = Vec::with_capacity(n_sub);
        for s in 0..n_sub {
            let cell_opt = ds.archetype.data_cell(CellCoord::new(r, src_col, s));
            let val = match cell_opt.map(|c| &c.value) {
                Some(CellValue::Number(v)) => json!(v),
                Some(CellValue::Text(t)) => json!(t),
                Some(CellValue::Empty) | None => serde_json::Value::Null,
            };
            let is_excl = cell_opt.is_some_and(|c| c.excluded);
            if is_excl {
                excluded_cells.push(json!({ "row": r, "col": col_count, "subcol": s }));
            }
            row_vals.push(json!({ "value": val, "excluded": is_excl, "parse_asterisk_exclusion": false }));
        }
        rows_json.push(row_vals);
    }

    let mut commands = vec![
        (
            String::from("table.paste"),
            json!({
                "sheet_id": ds.id.as_str(),
                "mode": "append_columns",
                "start_row": 0,
                "rows": rows_json,
            }),
        ),
        (
            String::from("table.setTitles"),
            json!({
                "sheet_id": ds.id.as_str(),
                "col": col_count,
                "title": new_title,
            }),
        ),
    ];
    if !excluded_cells.is_empty() {
        commands.push((
            String::from("table.excludeCells"),
            json!({
                "sheet_id": ds.id.as_str(),
                "cells": excluded_cells,
                "excluded": true,
            }),
        ));
    }
    if let TableArchetype::Nested(nt) = &ds.archetype
        && let Some(grp) = nt.groups.get(src_col)
    {
        for (s, sub_title) in grp.subcolumn_titles.iter().enumerate() {
            if !sub_title.is_empty() {
                commands.push((
                    String::from("table.setSubcolumnTitles"),
                    json!({
                        "sheet_id": ds.id.as_str(),
                        "col": col_count,
                        "subcol": s,
                        "title": sub_title,
                    }),
                ));
            }
        }
    }
    if let TableArchetype::MultipleVariables(mv) = &ds.archetype
        && let Some(var) = mv.variables.get(src_col)
        && var.role != VariableRole::Calculated
    {
        let role_slug = match var.role {
            VariableRole::Continuous => "continuous",
            VariableRole::Categorical => "categorical",
            VariableRole::Label => "label",
            VariableRole::Calculated => "continuous",
        };
        commands.push((
            String::from("table.setVariableRole"),
            json!({
                "sheet_id": ds.id.as_str(),
                "col": col_count,
                "role": role_slug,
            }),
        ));
    }
    Some(UiIntent::EngineCommandGroup { commands })
}

fn handle_grid_ctx_exclude(
    app: &UiApp,
    ds: &DataSheet,
    row: usize,
    cell_ctx: Option<ContextCellTarget>,
    selected_x_bounds: Option<(usize, usize, usize, usize)>,
    intents: &mut PendingUiIntents,
) -> Option<LinkedDataRegion> {
    if let Some(tgt) = cell_ctx
        && tgt.is_x
    {
        if let Some((r0, r1, s0, s1)) = selected_x_bounds
            && (r0..=r1).contains(&tgt.row)
            && (s0..=s1).contains(&tgt.subcol)
        {
            push_x_range_exclude_intent(ds, r0, r1, s0, s1, intents);
        } else {
            push_x_range_exclude_intent(ds, tgt.row, tgt.row, tgt.subcol, tgt.subcol, intents);
        }
        return None;
    }
    if cell_ctx.is_none()
        && let Some((xr0, xr1, xs0, xs1)) = selected_x_bounds
    {
        push_x_range_exclude_intent(ds, xr0, xr1, xs0, xs1, intents);
        return None;
    }
    if let Some(sel) =
        resolve_grid_selection(app, ds).filter(|sel| cell_ctx.is_none_or(|t| sel.contains_coord(GridCellCoord::new(t.row, t.col, t.subcol))))
    {
        if let Some(reg) = first_linked_region_in_selection(ds, &sel) {
            return Some(reg.clone());
        }
        let target_excluded = !sel.first_coord().and_then(|c| ds.archetype.data_cell(c)).is_some_and(|c| c.excluded);
        push_exclude_selection_intent(ds, &sel, target_excluded, intents);
        return None;
    }
    let c = cell_ctx
        .filter(|t| !t.is_x)
        .map(|t| GridCellCoord::new(t.row, t.col, t.subcol))
        .or_else(|| app.grid.active_cell_for_sheet(&ds.id))
        .unwrap_or(GridCellCoord::new(row, 0, 0));
    if let Some(reg) = linked_region_for_cell(ds, c.row, c.col) {
        return Some(reg.clone());
    }
    let was_ex = ds.archetype.data_cell(c.into()).is_some_and(|cell| cell.excluded);
    intents.push_engine(
        "table.excludeCells",
        json!({ "sheet_id": ds.id.as_str(), "row": c.row, "col": c.col, "subcol": c.subcol, "excluded": !was_ex }),
    );
    None
}

fn handle_grid_ctx_cut(
    ui: &Ui,
    app: &UiApp,
    ds: &DataSheet,
    cell_ctx: Option<ContextCellTarget>,
    selected_x_bounds: Option<(usize, usize, usize, usize)>,
    intents: &mut PendingUiIntents,
) -> Option<LinkedDataRegion> {
    let x_target_bounds = if let Some(tgt) = cell_ctx.filter(|t| t.is_x) {
        if let Some((r0, r1, s0, s1)) = selected_x_bounds
            && (r0..=r1).contains(&tgt.row)
            && (s0..=s1).contains(&tgt.subcol)
        {
            Some((r0, r1, s0, s1))
        } else {
            Some((tgt.row, tgt.row, tgt.subcol, tgt.subcol))
        }
    } else {
        selected_x_bounds
    };
    if let Some((xr0, xr1, xs0, xs1)) = x_target_bounds {
        copy_x_range_to_clipboard(ui, app, ds, xr0, xr1, xs0, xs1);
        push_x_range_clear_intents(ds, xr0, xr1, xs0, xs1, intents);
        return None;
    }
    if ds.frozen {
        return None;
    }
    let linked = resolve_grid_selection(app, ds)
        .and_then(|sel| first_linked_region_in_selection(ds, &sel))
        .or_else(|| app.grid.active_cell_for_sheet(&ds.id).and_then(|active| linked_region_for_cell(ds, active.row, active.col)));
    if let Some(reg) = linked {
        return Some(reg.clone());
    }
    clear_x_clipboard_cache(ui);
    intents.push(UiIntent::GridCutSelection);
    None
}
