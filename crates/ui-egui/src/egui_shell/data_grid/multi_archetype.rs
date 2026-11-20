//! Unified multi-archetype data grid (`XY`, `Grouped`, `Contingency`, `Survival`, `PartsOfWhole`,
//! `MultipleVariables`, `Nested`) body, gutter, cell rendering, and context menu (`R15.4`).

use std::fmt::Write as _;
use std::ops::Range;

use curvebook_doc::{Cell, CellCoord, CellValue, DataSheet, LinkedDataRegion, TableArchetype, VariableRole, VariableSource, XColumnMode};
use curvebook_tools::GridCellCoord;
use egui::scroll_area::{DragScroll, ScrollBarVisibility, ScrollSource};
use egui::{Align, Color32, FontId, Id, Order, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, WidgetInfo, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::grid_view::GridViewState;
use crate::inspect::ZLayer;
use crate::views::grid_core::linked_region_for_cell;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};
use super::headers::{render_multi_column_headers, render_multi_corner, row_title_at};
use super::inline_edit::{begin_inline_edit, render_multi_inline_editors};
use super::linked_regions::{open_link_popover, render_multi_link_guard_popover, render_multi_link_strip};
use super::mv_roles::{active_mv_formula_operands, render_mv_role_menu_popup};
use super::shared::{
    AuxInlineTarget, CellTextStyle, ContextCellTarget, ContextMenuState, MultiGridAuxState, MultiGridCtx, MultiGridFrameState, MultiGridGeom,
    alt_row_fill, any_pointer_click_in, clamp_multi_scroll, clear_grid_drag_anchor_if_released, compute_multi_geom, compute_multi_scrollbar_rects,
    compute_visible_range, ensure_solid_scrollbar_style, format_cell_into, forward_frozen_wheel, handle_cell_pointer_drag_select,
    initial_edit_text_for_cell, is_multi_click_edit, link_border_color32, multi_text_style, paint_exclusion_corner_marker,
    paint_linked_perimeter_edges, record_meta_if_enabled, render_ellipsized_text, set_grid_focus_active,
};
use super::shortcuts::{
    clear_x_clipboard_cache, copy_x_cell_to_clipboard, handle_multi_keyboard_shortcuts, push_x_cell_text_paste_intents,
    read_clipboard_text_for_x_paste,
};
use super::{ColumnGridEguiState, DataGridEguiState, DataGridFrameReport, DataGridRenderContext, GridInlineEditTarget};

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
    painter.rect_stroke(geom.grid_outer_rect, 0.0, Stroke::new(1.0, tokens.border), StrokeKind::Inside);

    if let Some(strip_rect) = geom.link_strip_rect {
        render_multi_link_strip(ui, app, ds, strip_rect, state, tokens, meta);
    }
    if !blocked {
        forward_frozen_wheel(ui, &geom, state);
    }

    let mctx = MultiGridCtx { app, ds, geom: &geom, tokens, blocked };
    let body_out = {
        let mut fstate = MultiGridFrameState { state, aux: &mut aux, meta, intents };
        render_multi_corner(ui, &mctx, &mut fstate);
        let active_mv_operands = active_mv_formula_operands(app, ds);
        render_multi_column_headers(ui, &mctx, &mut fstate, &active_mv_operands);
        render_multi_gutter_body(ui, &mctx, &mut fstate);
        let body_out = render_multi_body(ui, &mctx, &mut fstate);

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

        render_multi_inline_editors(ui, &mctx, &mut fstate);
        render_mv_role_menu_popup(ui, &mctx, &mut fstate);
        render_row_context_menu_popup(ui, &mctx, &mut fstate);
        if !blocked {
            handle_multi_keyboard_shortcuts(ui, &mctx, &mut fstate);
        }
        body_out
    };

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

fn render_multi_gutter_body(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let g_rect = geom.row_gutter_rect;
    if !g_rect.is_positive() {
        return;
    }
    let scroll_y = fstate.state.scroll_offset_pt.y;
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
            let row_selected = fstate.aux.selected_x_cell.map_or_else(|| app.grid.selected_rows.contains(&row), |(xr, _)| xr == row);
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
                    multi_text_style(11.0, true, tokens.text_muted, Align::RIGHT, false, false),
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
                if resp.clicked() && !blocked && geom.col_count > 0 {
                    let c = GridCellCoord::new(row, 0, 0);
                    fstate.aux.selected_x_cell = None;
                    fstate.intents.push_select_grid_cell(c);
                }
                if resp.secondary_clicked()
                    && !blocked
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    fstate.aux.context_menu = Some(ContextMenuState { pos, row, cell: None });
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
                        CellTextStyle::plain(FontId::proportional(11.0), tokens.text_muted, Align::Center),
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
                        let resp = gui.interact(rt_r, gui.make_persistent_id(&wid), if blocked { Sense::hover() } else { Sense::click() });
                        if resp.hovered() && !blocked {
                            gui.painter().rect_filled(rt_r, 0.0, tokens.hover_bg);
                        }
                        render_ellipsized_text(
                            gui,
                            rt_r.shrink2(vec2(5.0, 0.0)),
                            &row_title,
                            CellTextStyle::plain(FontId::proportional(12.0), tokens.text_primary, Align::LEFT),
                        );
                        let acc_rt = if row_title.is_empty() { format!("Row {} Title", row + 1) } else { row_title.clone() };
                        resp.widget_info(move || WidgetInfo::labeled(egui::Role::Button, !blocked, acc_rt.clone()));
                        record_meta_if_enabled(
                            gui,
                            fstate.meta,
                            fstate.state.record_inspect_meta,
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
                            fstate.aux.aux_edit = None;
                            begin_inline_edit(gui, fstate.state, GridInlineEditTarget::RowTitle { row }, row_title);
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

#[derive(Clone, Copy)]
struct MultiXCellDraw {
    row: usize,
    subcol: usize,
    clipped: Rect,
}

#[derive(Clone, Copy)]
struct MultiDataCellDraw {
    coord: GridCellCoord,
    raw_cell_r: Rect,
    clipped: Rect,
}

fn render_multi_body(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) -> MultiBodyOut {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let blocked = mctx.blocked;
    let b_rect = geom.body_rect;
    if !b_rect.is_positive() {
        return MultiBodyOut { rendered_cells: 0, vis_rows: 0..0, vis_all_flat: 0..0 };
    }
    let mut out = MultiBodyOut { rendered_cells: 0, vis_rows: 0..0, vis_all_flat: 0..0 };
    let scroll_offset = fstate.state.scroll_offset_pt;

    let scroll_out = ui.scope_builder(UiBuilder::new().max_rect(b_rect), |body_parent| {
        if blocked {
            body_parent.disable();
        }
        egui::ScrollArea::both()
            .id_salt(("r15_4_multi_grid_scroll", ds.id.as_str()))
            .scroll_offset(scroll_offset)
            .scroll_bar_visibility(if blocked { ScrollBarVisibility::AlwaysHidden } else { ScrollBarVisibility::AlwaysVisible })
            .scroll_source(if blocked { ScrollSource::NONE } else { ScrollSource { scroll_bar: true, drag: DragScroll::Never, mouse_wheel: true } })
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
                let record_meta = fstate.state.record_inspect_meta && !body_ui.ctx().will_discard();

                body_ui.set_clip_rect(geom.body_inner_clip);

                let x_seq_readonly = matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }));
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
                            render_single_multi_x_cell(body_ui, mctx, fstate, MultiXCellDraw { row, subcol: flat, clipped }, &mut dbuf);
                            out.rendered_cells += 1;
                        } else if let Some((col, subcol)) = geom.flat_data_to_coord(flat - geom.x_subcols) {
                            render_single_multi_data_cell(
                                body_ui,
                                mctx,
                                fstate,
                                MultiDataCellDraw { coord: GridCellCoord::new(row, col, subcol), raw_cell_r, clipped },
                                &mut dbuf,
                            );
                            out.rendered_cells += 1;
                        }
                    }

                    body_ui
                        .painter()
                        .line_segment([pos2(geom.body_inner_clip.min.x, y1), pos2(geom.body_inner_clip.max.x, y1)], Stroke::new(1.0, tokens.divider));
                }

                paint_multi_vertical_hairlines(body_ui, origin.x, geom, &vis_all_flat, tokens);
                clear_grid_drag_anchor_if_released(body_ui, &ds.id);
            })
    });
    if !blocked {
        fstate.state.scroll_offset_pt = scroll_out.inner.state.clamped_offset();
        clamp_multi_scroll(fstate.state, geom);
    }
    out
}

fn render_single_multi_x_cell(
    body_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    cell: MultiXCellDraw,
    dbuf: &mut MultiBodyDrawBuffers<'_>,
) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let MultiXCellDraw { row, subcol, clipped } = cell;
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
    let is_selected = fstate.aux.selected_x_cell == Some((row, subcol));
    let editing_this = fstate.aux.aux_edit.as_ref().is_some_and(|(t, _, _)| *t == AuxInlineTarget::XCell { row, subcol });

    let fill = if is_selected {
        tokens.active_bg
    } else if dbuf.x_seq_readonly {
        tokens.panel_bg
    } else {
        dbuf.base_row_bg
    };
    body_ui.painter().rect_filled(clipped, 0.0, fill);
    if is_selected {
        body_ui.painter().rect_stroke(clipped, 0.0, Stroke::new(1.5, tokens.accent), StrokeKind::Inside);
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
            multi_text_style(geom.font_num_pt, true, text_col, Align::RIGHT, excluded || dbuf.x_seq_readonly, excluded),
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

    if !dbuf.blocked && resp.clicked() {
        fstate.aux.selected_x_cell = Some((row, subcol));
        fstate.aux.active_role_menu_col = None;
        fstate.aux.context_menu = None;
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
        fstate.aux.selected_x_cell = Some((row, subcol));
        body_ui.ctx().request_repaint();
        fstate.aux.context_menu = Some(ContextMenuState { pos, row, cell: Some(ContextCellTarget { is_x: true, row, col: 0, subcol }) });
    }
}

struct MultiCellFlags<'a> {
    cell_opt: Option<&'a Cell>,
    excluded: bool,
    mv_is_cat_or_lbl: bool,
    mv_is_calc: bool,
    linked: Option<&'a LinkedDataRegion>,
    editable: bool,
    is_active: bool,
    is_selected: bool,
    contingency_invalid: bool,
}

fn render_single_multi_data_cell(
    body_ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    cell: MultiDataCellDraw,
    dbuf: &mut MultiBodyDrawBuffers<'_>,
) {
    let app = mctx.app;
    let ds = mctx.ds;
    let coord = cell.coord;
    let cell_opt = ds.archetype.data_cell(CellCoord::from(coord));
    dbuf.text_buf.clear();
    if let Some(c) = cell_opt {
        format_cell_into(c, &app.locale_preset, dbuf.text_buf);
    }
    let mv_var =
        if dbuf.is_mv { if let TableArchetype::MultipleVariables(mv) = &ds.archetype { mv.variables.get(coord.col) } else { None } } else { None };
    let mv_is_calc = mv_var.is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }));
    let mv_is_cat_or_lbl = mv_var.is_some_and(|v| matches!(v.role, VariableRole::Categorical | VariableRole::Label));
    let linked = linked_region_for_cell(ds, coord.row, coord.col);
    let (is_active, is_selected) = if fstate.aux.selected_x_cell.is_some() { (false, false) } else { is_coord_selected(app, coord) };
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

    paint_single_multi_data_cell(body_ui, mctx, fstate.state, cell, dbuf, &flags);

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
    let acc_owned = dbuf.acc_buf.clone();
    let acc_enabled = flags.editable || flags.mv_is_calc;
    resp.widget_info(move || WidgetInfo::labeled(egui::Role::Cell, acc_enabled, acc_owned.clone()));

    handle_multi_data_cell_interactions(body_ui, mctx, fstate, coord, dbuf, &flags, &resp);
}

fn paint_single_multi_data_cell(
    body_ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    state: &ColumnGridEguiState,
    cell: MultiDataCellDraw,
    dbuf: &MultiBodyDrawBuffers<'_>,
    flags: &MultiCellFlags<'_>,
) {
    let ds = mctx.ds;
    let geom = mctx.geom;
    let tokens = mctx.tokens;
    let MultiDataCellDraw { coord, raw_cell_r, clipped } = cell;
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
        body_ui.painter().rect_stroke(clipped, 0.0, Stroke::new(1.5, tokens.accent), StrokeKind::Inside);
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
            multi_text_style(
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

fn handle_multi_data_cell_interactions(
    body_ui: &Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    coord: GridCellCoord,
    dbuf: &MultiBodyDrawBuffers<'_>,
    flags: &MultiCellFlags<'_>,
    resp: &Response,
) {
    let app = mctx.app;
    let ds = mctx.ds;
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
        fstate.meta.record_response(
            body_ui,
            resp,
            FrameInspectEntryInput {
                widget_id: dbuf.id_buf,
                role: "gridcell",
                label: dbuf.acc_buf,
                command_id: Some("table.setCell"),
                value: Some(json!({
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
        if fstate.aux.selected_x_cell.take().is_some() {
            body_ui.ctx().request_repaint();
        }
        fstate.aux.active_role_menu_col = None;
        fstate.aux.context_menu = None;
        set_grid_focus_active(body_ui, true);
        if body_ui.input(|i| i.modifiers).shift {
            fstate.intents.push_extend_grid_selection(coord);
        } else {
            fstate.intents.push_select_grid_cell(coord);
        }
        if is_multi_click_edit(resp) {
            if flags.mv_is_calc {
                fstate.intents.push_ui("ui.table.openFormulaBar", json!({ "sheet_id": ds.id.as_str(), "col": coord.col }));
            } else if let Some(reg) = flags.linked {
                open_link_popover(fstate.state, reg, &app.locale);
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
    } else if !dbuf.blocked
        && fstate.aux.active_role_menu_col.is_none()
        && fstate.aux.context_menu.is_none()
        && !is_multi_click_edit(resp)
        && !resp.secondary_clicked()
        && handle_cell_pointer_drag_select(body_ui, &ds.id, coord, resp, fstate.intents)
    {
        fstate.aux.selected_x_cell = None;
        fstate.aux.active_role_menu_col = None;
        fstate.aux.context_menu = None;
    }
    if !dbuf.blocked
        && resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        fstate.aux.selected_x_cell = None;
        fstate.intents.push_select_grid_cell(coord);
        fstate.aux.context_menu = Some(ContextMenuState {
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

fn render_row_context_menu_popup(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let app = mctx.app;
    let ds = mctx.ds;
    let tokens = mctx.tokens;
    let Some(ctx_menu) = fstate.aux.context_menu.clone() else { return };
    let (pos, row, cell_ctx) = (ctx_menu.pos, ctx_menu.row, ctx_menu.cell);
    if mctx.blocked {
        fstate.aux.context_menu = None;
        return;
    }
    let area_resp = egui::Area::new(Id::unique(("grid_ctx_menu", ds.id.as_str()))).fixed_pos(pos).order(Order::Foreground).show(ui.ctx(), |m| {
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
                    fstate.meta,
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
                                fstate.intents.push_engine("sheet.select", json!({ "sheet_id": g.id.as_str() }));
                            }
                            fstate.intents.push_ui("ui.inspector.select", json!({ "inspector": "format_graph" }));
                        }
                        "grid.ctx.show_row_titles" => {
                            if let Some(g) = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == ds.id) {
                                fstate.intents.push_engine(
                                    "graph.format",
                                    json!({ "sheet_id": g.id.as_str(), "row_point_labels_row": row, "show_row_titles": true }),
                                );
                            }
                        }
                        "grid.ctx.reset_style" => {
                            if let Some(g) = app.document().graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == ds.id) {
                                fstate.intents.push_engine("graph.format", json!({ "sheet_id": g.id.as_str(), "reset_row_override": row }));
                            }
                        }
                        "grid.ctx.exclude" => {
                            if let Some(tgt) = cell_ctx
                                && tgt.is_x
                            {
                                let was_ex = ds.archetype.x_or_time_cell(tgt.row, tgt.subcol).is_some_and(|c| c.excluded);
                                fstate.intents.push_engine(
                                    "table.excludeCells",
                                    json!({ "sheet_id": ds.id.as_str(), "target": "x", "row": tgt.row, "subcol": tgt.subcol, "excluded": !was_ex }),
                                );
                            } else {
                                let c = app.grid.tool.active.unwrap_or(GridCellCoord::new(row, 0, 0));
                                let was_ex = ds.archetype.data_cell(c.into()).is_some_and(|cell| cell.excluded);
                                fstate.intents.push_engine(
                                    "table.excludeCells",
                                    json!({ "sheet_id": ds.id.as_str(), "row": c.row, "col": c.col, "subcol": c.subcol, "excluded": !was_ex }),
                                );
                            }
                        }
                        "grid.ctx.exclude_reason" => fstate.intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": ds.id.as_str() })),
                        "grid.ctx.copy" => {
                            if let Some(tgt) = cell_ctx
                                && tgt.is_x
                            {
                                copy_x_cell_to_clipboard(list, app, ds, tgt.row, tgt.subcol);
                            } else if let Some((xr, xs)) = fstate.aux.selected_x_cell {
                                copy_x_cell_to_clipboard(list, app, ds, xr, xs);
                            } else {
                                clear_x_clipboard_cache(list);
                                fstate.intents.push_grid_copy_selection();
                            }
                        }
                        "grid.ctx.paste" => {
                            if let Some(tgt) = cell_ctx
                                && tgt.is_x
                            {
                                if let Some(clip) = read_clipboard_text_for_x_paste(list, app) {
                                    push_x_cell_text_paste_intents(app, ds, tgt.row, tgt.subcol, &clip, fstate.intents);
                                }
                            } else if let Some((xr, xs)) = fstate.aux.selected_x_cell {
                                if let Some(clip) = read_clipboard_text_for_x_paste(list, app) {
                                    push_x_cell_text_paste_intents(app, ds, xr, xs, &clip, fstate.intents);
                                }
                            } else {
                                fstate.intents.push_grid_paste_from_clipboard(app.grid.tool.active);
                            }
                        }
                        _ => {}
                    }
                    fstate.aux.context_menu = None;
                }
            }
        });
    });
    if area_resp.response.clicked_elsewhere() {
        fstate.aux.context_menu = None;
    }
}
