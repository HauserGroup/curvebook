//! Unified buffered single-commit inline text editors and commit intent dispatch for all 8 data
//! grid archetypes.

use curvebook_doc::DataSheet;
use curvebook_tools::GridCellCoord;
use egui::{CornerRadius, FontFamily, FontId, Id, Key, Modifiers, Rect, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, pos2, vec2};
use serde_json::json;

use crate::UiApp;
use crate::inspect::ZLayer;

use super::super::{FrameInspectEntryInput, PendingUiIntents, UiIntent};
use super::shared::{
    AuxInlineTarget, MultiGridAuxState, MultiGridCtx, MultiGridFrameState, MultiGridGeom, parse_inline_cell_input, pending_commit_key,
    pressed_key_event_modifiers, record_meta_if_enabled,
};
use super::shortcuts::{extract_multiline_or_tab_x_input, push_x_cell_text_paste_intents, step_cell_horiz};
use super::{ColumnGridEguiState, GridInlineEditBuffer, GridInlineEditTarget};

pub(super) fn begin_inline_edit(ui: &Ui, state: &mut ColumnGridEguiState, target: GridInlineEditTarget, initial: String) {
    state.inline_edit = Some(GridInlineEditBuffer { target, draft: initial, focused_once: false });
    ui.ctx().request_repaint();
}

fn pending_aux_commit_key(sheet_id: &curvebook_doc::SheetId) -> Id {
    Id::unique(("r15_aux_inline_pending_commit", sheet_id.as_str()))
}

fn put_styled_inline_text_edit(
    ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    rect: Rect,
    te_id: Id,
    draft: &mut String,
    is_numeric_cell: bool,
) -> egui::Response {
    ui.painter().rect_filled(rect, CornerRadius::ZERO, mctx.tokens.input_bg);
    ui.painter().rect_stroke(rect, CornerRadius::ZERO, Stroke::new(2.0, mctx.tokens.focus_ring), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(2.0, 1.0));
    let font_style = if is_numeric_cell {
        super::super::fonts::aligned_numeral_font_id(ui.ctx(), mctx.tokens)
    } else {
        FontId::new(mctx.metrics.font_ui_body_pt, FontFamily::Proportional)
    };
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(inner);
        ui.add_sized(
            inner.size(),
            TextEdit::singleline(draft)
                .id(te_id)
                .font(font_style)
                .frame(egui::Frame::NONE)
                .clip_text(true)
                .lock_focus(true)
                .desired_width(inner.width()),
        )
    })
    .inner
}

pub(super) fn render_multi_inline_editors(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, scroll_offset: Vec2) -> bool {
    if mctx.blocked {
        return false;
    }
    if fstate.state.inline_edit.is_some() {
        return render_standard_inline_editor(ui, mctx, fstate, scroll_offset);
    }
    if fstate.aux.aux_edit.is_some() {
        return render_aux_inline_editor(ui, mctx, fstate, scroll_offset);
    }
    false
}

struct InlineEditShellSpec<'a> {
    rect: Rect,
    widget_id: &'a str,
    te_id: Id,
    pending_key: Id,
    is_cell: bool,
    first_frame: bool,
    suppress_loss_commit: bool,
}

enum InlineEditOutcome {
    Cancel,
    Commit { enter: Option<Modifiers>, tab: Option<Modifiers>, retry_discard: bool },
    KeepOpen,
}

fn run_inline_editor_shell(
    ui: &mut Ui,
    mctx: &MultiGridCtx<'_>,
    fstate: &mut MultiGridFrameState<'_>,
    spec: InlineEditShellSpec<'_>,
    draft: &mut String,
) -> InlineEditOutcome {
    if !spec.first_frame && pressed_key_event_modifiers(ui, Key::Escape).is_some() {
        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        ui.data_mut(|d| d.remove::<bool>(spec.pending_key));
        ui.memory_mut(|m| m.surrender_focus(spec.te_id));
        ui.ctx().request_repaint();
        return InlineEditOutcome::Cancel;
    }
    let enter = if spec.first_frame { None } else { pressed_key_event_modifiers(ui, Key::Enter) };
    let tab = if spec.first_frame { None } else { pressed_key_event_modifiers(ui, Key::Tab) };
    if tab.is_some() {
        // Consume Tab/Shift+Tab before `TextEdit` (with `lock_focus(true)`) so it never inserts `\t`.
        ui.input_mut(|i| {
            i.consume_key(Modifiers::NONE, Key::Tab);
            i.consume_key(Modifiers::SHIFT, Key::Tab);
        });
    }
    let resp = put_styled_inline_text_edit(ui, mctx, spec.rect, spec.te_id, draft, spec.is_cell);
    if spec.first_frame {
        resp.request_focus();
        ui.ctx().request_repaint();
    }
    if !ui.ctx().will_discard() {
        record_meta_if_enabled(
            ui,
            fstate.meta,
            fstate.state.record_inspect_meta,
            &resp,
            FrameInspectEntryInput {
                widget_id: spec.widget_id,
                role: "textbox",
                label: draft,
                command_id: None,
                value: Some(serde_json::Value::String(draft.clone())),
                enabled: true,
                disabled_reason: None,
                z_layer: ZLayer::Workspace,
                ellipsized: false,
                full_label: None,
            },
        );
    }
    let lost = resp.lost_focus() && !spec.first_frame && !spec.suppress_loss_commit;
    let discard_retry = ui.data(|d| d.get_temp::<bool>(spec.pending_key).unwrap_or(false));
    if discard_retry || enter.is_some() || tab.is_some() || lost {
        if enter.is_some() {
            ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Enter);
                i.consume_key(Modifiers::SHIFT, Key::Enter);
            });
        } else if tab.is_some() {
            ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Tab);
                i.consume_key(Modifiers::SHIFT, Key::Tab);
            });
        }
        let retry_discard = ui.ctx().will_discard();
        if retry_discard {
            ui.data_mut(|d| d.insert_temp(spec.pending_key, true));
        } else {
            ui.data_mut(|d| d.remove::<bool>(spec.pending_key));
            ui.memory_mut(|m| m.surrender_focus(spec.te_id));
            ui.ctx().request_repaint();
        }
        return InlineEditOutcome::Commit { enter, tab, retry_discard };
    }
    InlineEditOutcome::KeepOpen
}

fn render_standard_inline_editor(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, scroll_offset: Vec2) -> bool {
    let Some(mut edit) = fstate.state.inline_edit.take() else { return false };
    let Some(rect) = rect_for_standard_target(mctx.geom, scroll_offset, &edit.target) else { return false };
    if !rect.is_positive() {
        fstate.state.inline_edit = Some(edit);
        return false;
    }
    let wid = match &edit.target {
        GridInlineEditTarget::Cell(c) => format!("grid.cell.{}.{}.{}", c.row, c.col, c.subcol),
        GridInlineEditTarget::ColumnTitle { col } => format!("grid.col_title.{col}"),
        GridInlineEditTarget::RowTitle { row } => format!("grid.row_title.{row}"),
    };
    let first_frame = !edit.focused_once;
    edit.focused_once = true;
    let spec = InlineEditShellSpec {
        rect,
        widget_id: &wid,
        te_id: ui.make_persistent_id(("multi_std_inline", mctx.ds.id.as_str(), &wid)),
        pending_key: pending_commit_key(&mctx.ds.id),
        is_cell: matches!(edit.target, GridInlineEditTarget::Cell(_)),
        first_frame,
        suppress_loss_commit: false,
    };
    match run_inline_editor_shell(ui, mctx, fstate, spec, &mut edit.draft) {
        InlineEditOutcome::Cancel => true,
        InlineEditOutcome::Commit { enter, tab, retry_discard } => {
            commit_standard_inline_edit(mctx.app, mctx.ds, edit.target, edit.draft.clone(), fstate.intents);
            if let GridInlineEditTarget::Cell(coord) = edit.target {
                if let Some(mods) = enter
                    && mctx.geom.row_count > 0
                {
                    let next_row = if mods.shift { coord.row.saturating_sub(1) } else { (coord.row + 1).min(mctx.geom.row_count - 1) };
                    fstate.intents.push(UiIntent::SelectGridCell { coord: GridCellCoord::new(next_row, coord.col, coord.subcol) });
                } else if let Some(mods) = tab {
                    let (is_x, nc_opt, nx_opt) = step_cell_horiz(mctx.geom, Some(coord), None, mods.shift);
                    if is_x {
                        fstate.aux.selected_x_cell = nx_opt;
                    } else if let Some(nc) = nc_opt {
                        fstate.aux.selected_x_cell = None;
                        fstate.intents.push(UiIntent::SelectGridCell { coord: nc });
                    }
                }
            }
            if retry_discard {
                fstate.state.inline_edit = Some(edit);
            }
            enter.is_some() || tab.is_some()
        }
        InlineEditOutcome::KeepOpen => {
            fstate.state.inline_edit = Some(edit);
            false
        }
    }
}

fn render_aux_inline_editor(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>, scroll_offset: Vec2) -> bool {
    let Some((target, mut draft, just_focused)) = fstate.aux.aux_edit.take() else { return false };
    let Some(rect) = rect_for_aux_target(mctx.geom, scroll_offset, &target) else { return false };
    if !rect.is_positive() {
        fstate.aux.aux_edit = Some((target, draft, just_focused));
        return false;
    }
    let wid = match &target {
        AuxInlineTarget::XCell { row, subcol } => format!("grid.xcell.{row}.{subcol}"),
        AuxInlineTarget::XTitle { subcol } => format!("grid.x_title.{subcol}"),
        AuxInlineTarget::SubcolumnTitle { col, subcol } => format!("grid.subcol_title.{col}.{subcol}"),
        AuxInlineTarget::SheetTitle => String::from("grid.sheet_title"),
    };
    let te_id = ui.make_persistent_id(("multi_aux_inline", mctx.ds.id.as_str(), &wid));
    let pending_key = pending_aux_commit_key(&mctx.ds.id);
    if matches!(target, AuxInlineTarget::XCell { .. })
        && (!just_focused || ui.memory(|m| m.has_focus(te_id)))
        && let Some(multi_text) = extract_multiline_or_tab_x_input(ui, mctx.app)
    {
        commit_aux_inline_edit(mctx.app, mctx.ds, fstate.aux, &target, multi_text, fstate.intents);
        ui.data_mut(|d| d.remove::<bool>(pending_key));
        return true;
    }
    let suppress_loss_commit = if just_focused {
        fstate.aux.suppress_loss_commit_once = true;
        true
    } else {
        std::mem::take(&mut fstate.aux.suppress_loss_commit_once)
    };
    let spec = InlineEditShellSpec {
        rect,
        widget_id: &wid,
        te_id,
        pending_key,
        is_cell: matches!(target, AuxInlineTarget::XCell { .. }),
        first_frame: just_focused,
        suppress_loss_commit,
    };
    match run_inline_editor_shell(ui, mctx, fstate, spec, &mut draft) {
        InlineEditOutcome::Cancel => true,
        InlineEditOutcome::Commit { enter, tab, retry_discard } => {
            commit_aux_inline_edit(mctx.app, mctx.ds, fstate.aux, &target, draft.clone(), fstate.intents);
            if let AuxInlineTarget::XCell { row, subcol } = target {
                if let Some(mods) = enter
                    && mctx.geom.row_count > 0
                {
                    let next_row = if mods.shift { row.saturating_sub(1) } else { (row + 1).min(mctx.geom.row_count - 1) };
                    fstate.aux.selected_x_cell = Some((next_row, subcol));
                } else if let Some(mods) = tab {
                    let (is_x, nc_opt, nx_opt) = step_cell_horiz(mctx.geom, None, Some((row, subcol)), mods.shift);
                    if is_x {
                        fstate.aux.selected_x_cell = nx_opt;
                    } else if let Some(nc) = nc_opt {
                        fstate.aux.selected_x_cell = None;
                        fstate.intents.push(UiIntent::SelectGridCell { coord: nc });
                    }
                }
            }
            if retry_discard {
                fstate.aux.aux_edit = Some((target, draft, false));
            }
            enter.is_some() || tab.is_some()
        }
        InlineEditOutcome::KeepOpen => {
            fstate.aux.aux_edit = Some((target, draft, false));
            false
        }
    }
}

fn rect_for_standard_target(geom: &MultiGridGeom, scroll_offset: Vec2, target: &GridInlineEditTarget) -> Option<Rect> {
    match *target {
        GridInlineEditTarget::Cell(coord) => {
            let flat = geom.coord_to_flat_all(false, coord.col, coord.subcol);
            #[allow(clippy::cast_precision_loss)]
            let x0 = geom.body_rect.min.x + (flat as f32) * geom.subcol_w - scroll_offset.x;
            #[allow(clippy::cast_precision_loss)]
            let y0 = geom.body_rect.min.y + (coord.row as f32) * geom.row_h - scroll_offset.y;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.row_h)).intersect(geom.body_inner_clip))
        }
        GridInlineEditTarget::ColumnTitle { col } => {
            let (flat, span) = geom.col_span_flat(col);
            #[allow(clippy::cast_precision_loss)]
            let x0 = geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - scroll_offset.x;
            #[allow(clippy::cast_precision_loss)]
            let w = (span as f32) * geom.subcol_w;
            let y0 = geom.col_header_rect.min.y + geom.h_tier1;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(w, geom.h_tier2)).intersect(geom.col_header_rect))
        }
        GridInlineEditTarget::RowTitle { row } => {
            #[allow(clippy::cast_precision_loss)]
            let y0 = geom.row_gutter_rect.min.y + (row as f32) * geom.row_h - scroll_offset.y;
            let x0 = geom.row_gutter_rect.min.x + geom.row_num_w;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.row_title_w, geom.row_h)).intersect(geom.row_gutter_rect))
        }
    }
}

fn rect_for_aux_target(geom: &MultiGridGeom, scroll_offset: Vec2, target: &AuxInlineTarget) -> Option<Rect> {
    match *target {
        AuxInlineTarget::XCell { row, subcol } => {
            #[allow(clippy::cast_precision_loss)]
            let x0 = geom.body_rect.min.x + (subcol as f32) * geom.subcol_w - scroll_offset.x;
            #[allow(clippy::cast_precision_loss)]
            let y0 = geom.body_rect.min.y + (row as f32) * geom.row_h - scroll_offset.y;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.row_h)).intersect(geom.body_inner_clip))
        }
        AuxInlineTarget::XTitle { subcol } => {
            #[allow(clippy::cast_precision_loss)]
            let x0 = geom.col_header_rect.min.x + (subcol as f32) * geom.subcol_w - scroll_offset.x;
            let y0 = geom.col_header_rect.min.y + geom.h_tier1;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.h_tier2)).intersect(geom.col_header_rect))
        }
        AuxInlineTarget::SubcolumnTitle { col, subcol } => {
            let flat = geom.coord_to_flat_all(false, col, subcol);
            #[allow(clippy::cast_precision_loss)]
            let x0 = geom.col_header_rect.min.x + (flat as f32) * geom.subcol_w - scroll_offset.x;
            let y0 = geom.col_header_rect.min.y + geom.h_tier1 + geom.h_tier2;
            Some(Rect::from_min_size(pos2(x0, y0), vec2(geom.subcol_w, geom.h_tier3)).intersect(geom.col_header_rect))
        }
        AuxInlineTarget::SheetTitle => {
            let x0 = geom.corner_rect.min.x + geom.row_num_w;
            Some(Rect::from_min_size(pos2(x0, geom.corner_rect.min.y), vec2(geom.row_title_w, geom.h_tier1)))
        }
    }
}

fn cell_value_matches_json(cell: Option<&curvebook_doc::Cell>, val: &serde_json::Value) -> bool {
    match (cell.map(|c| &c.value), val) {
        (None | Some(curvebook_doc::CellValue::Empty), serde_json::Value::Null) => true,
        (Some(curvebook_doc::CellValue::Number(cur)), serde_json::Value::Number(n)) => n.as_f64().is_some_and(|v| v.to_bits() == cur.to_bits()),
        (Some(curvebook_doc::CellValue::Text(cur)), serde_json::Value::String(s)) => cur == s,
        _ => false,
    }
}

fn commit_standard_inline_edit(app: &UiApp, ds: &DataSheet, target: GridInlineEditTarget, draft: String, intents: &mut PendingUiIntents) {
    match target {
        GridInlineEditTarget::Cell(coord) => {
            let val = parse_inline_cell_input(app, &draft);
            if cell_value_matches_json(ds.cell(curvebook_doc::CellCoord::new(coord.row, coord.col, coord.subcol)), &val) {
                return;
            }
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
            let selecting_y_grid = intents
                .intents()
                .iter()
                .any(|it| matches!(it, UiIntent::SelectGridCell { .. } | UiIntent::ExtendGridSelection { .. } | UiIntent::ToggleGridCell { .. }));
            if !selecting_y_grid {
                aux.selected_x_cell = Some((row, subcol));
            }
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
