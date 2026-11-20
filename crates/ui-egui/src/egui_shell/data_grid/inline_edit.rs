//! Buffered single-commit inline text editors and commit intent dispatch for `Column` and
//! multi-archetype data grids.

use curvebook_doc::DataSheet;
use curvebook_tools::GridCellCoord;
use egui::{CornerRadius, FontFamily, FontId, Id, Key, Modifiers, Rect, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::inspect::ZLayer;

use super::super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents};
use super::shared::{
    AuxInlineTarget, GridRenderCtx, MultiGridAuxState, MultiGridCtx, MultiGridFrameState, MultiGridGeom, parse_inline_cell_input, pending_commit_key,
    pressed_key_event_modifiers, record_meta_if_enabled,
};
use super::shortcuts::{extract_multiline_or_tab_x_input, push_x_cell_text_paste_intents, step_cell_horiz};
use super::{ColumnGridEguiState, GridInlineEditBuffer, GridInlineEditTarget};

/// Per-editor placement and inspection metadata for [`render_inline_text_edit`] (`R15-GUI-08`).
pub(super) struct InlineTextEditSpec<'a> {
    pub(super) rect: Rect,
    pub(super) editor_id: Id,
    pub(super) hint: &'a str,
    pub(super) widget_id: &'a str,
}

pub(super) fn begin_inline_edit(ui: &Ui, state: &mut ColumnGridEguiState, target: GridInlineEditTarget, initial: String) {
    state.inline_edit = Some(GridInlineEditBuffer { target, draft: initial, focused_once: false });
    ui.ctx().request_repaint();
}

pub(super) fn render_inline_text_edit(
    ui: &mut Ui,
    rctx: &GridRenderCtx<'_>,
    state: &mut ColumnGridEguiState,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
    spec: InlineTextEditSpec<'_>,
) -> bool {
    let InlineTextEditSpec { rect, editor_id, hint, widget_id } = spec;
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

pub(super) fn render_multi_inline_editors(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    if mctx.blocked {
        return;
    }
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    if let Some(mut edit) = fstate.state.inline_edit.take() {
        let Some(rect) = rect_for_standard_target(geom, fstate.state, &edit.target) else { return };
        let wid = match &edit.target {
            GridInlineEditTarget::Cell(c) => format!("grid.cell.{}.{}.{}", c.row, c.col, c.subcol),
            GridInlineEditTarget::ColumnTitle { col } => format!("grid.col_title.{col}"),
            GridInlineEditTarget::RowTitle { row } => format!("grid.row_title.{row}"),
        };
        let te_id = ui.make_persistent_id(("multi_std_inline", &wid));
        let resp = ui.put(rect, TextEdit::singleline(&mut edit.draft).id(te_id).desired_width(rect.width()));
        let first_frame = !edit.focused_once;
        if first_frame {
            resp.request_focus();
            edit.focused_once = true;
        }
        record_meta_if_enabled(
            ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
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
            commit_standard_inline_edit(app, ds, edit.target, edit.draft, fstate.intents);
            if enter {
                if let Some(c) = app.grid.tool.active {
                    fstate.intents.push_select_grid_cell(GridCellCoord::new((c.row + 1).min(geom.row_count.saturating_sub(1)), c.col, c.subcol));
                }
            } else if let Some(mods) = tab
                && let Some(c) = app.grid.tool.active
            {
                let next = step_cell_horiz(geom, Some(c), None, mods.shift);
                if let (false, Some(nc), _) = next {
                    fstate.intents.push_select_grid_cell(nc);
                }
            }
            return;
        }
        fstate.state.inline_edit = Some(edit);
        return;
    }

    if let Some((target, mut draft, mut just_focused)) = fstate.aux.aux_edit.take() {
        let Some(rect) = rect_for_aux_target(geom, fstate.state, &target) else { return };
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
            commit_aux_inline_edit(app, ds, fstate.aux, &target, multi_text, fstate.intents);
            return;
        }
        let resp = ui.put(rect, TextEdit::singleline(&mut draft).id(te_id).desired_width(rect.width()));
        if just_focused {
            resp.request_focus();
            just_focused = false;
            fstate.aux.suppress_loss_commit_once = true;
        }
        record_meta_if_enabled(
            ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
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
        let lost = resp.lost_focus() && !fstate.aux.suppress_loss_commit_once;
        fstate.aux.suppress_loss_commit_once = false;
        if esc {
            return;
        }
        if enter || tab.is_some() || lost {
            commit_aux_inline_edit(app, ds, fstate.aux, &target, draft, fstate.intents);
            if enter && let AuxInlineTarget::XCell { row, subcol } = target {
                fstate.aux.selected_x_cell = Some(((row + 1).min(geom.row_count.saturating_sub(1)), subcol));
            }
            return;
        }
        fstate.aux.aux_edit = Some((target, draft, just_focused));
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
            intents
                .push_engine("table.setSubcolumnTitles", json!({ "sheet_id": ds.id.as_str(), "col": col, "subcol": subcol, "title": draft.trim() }));
        }
        AuxInlineTarget::SheetTitle => {
            let t = draft.trim();
            if !t.is_empty() {
                intents.push_engine("sheet.rename", json!({ "sheet_id": ds.id.as_str(), "title": t }));
            }
        }
    }
}
