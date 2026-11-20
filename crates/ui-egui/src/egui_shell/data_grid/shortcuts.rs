//! Keyboard navigation, exclusion, typing-to-edit, and clipboard shortcuts for `Column` and
//! multi-archetype data grids.

use curvebook_doc::{CellCoord, DataSheet, SheetCellRange, TableArchetype, VariableSource, XColumnMode};
use curvebook_tools::GridCellCoord;
use egui::{Id, Key, Ui};
use serde_json::json;

use crate::UiApp;
use crate::views::grid_core::linked_region_for_cell;

use super::super::{PendingUiIntents, UiIntent};
use super::inline_edit::begin_inline_edit;
use super::linked_regions::open_link_popover;
use super::shared::{
    AuxInlineTarget, GridRenderCtx, MultiGridAuxState, MultiGridCtx, MultiGridFrameState, MultiGridGeom, format_cell_into,
    initial_edit_text_for_cell, is_grid_focus_active, parse_inline_cell_input, pressed_key_event_modifiers,
};
use super::{ColumnGridEguiState, GridInlineEditTarget};

pub(super) fn handle_grid_keyboard_shortcuts(ui: &mut Ui, rctx: &GridRenderCtx<'_>, state: &mut ColumnGridEguiState, intents: &mut PendingUiIntents) {
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

pub(super) fn copy_x_cell_to_clipboard(ui: &Ui, app: &UiApp, ds: &DataSheet, row: usize, subcol: usize) {
    let mut text = String::new();
    if let Some(cell) = ds.archetype.x_or_time_cell(row, subcol) {
        format_cell_into(cell, &app.locale_preset, &mut text);
    }
    let _ = app.write_clipboard_text(&text);
    ui.ctx().copy_text(text.clone());
    let snapshot = app.interaction.last_clipboard_text.clone();
    ui.ctx().data_mut(|d| d.insert_temp(x_clipboard_cache_id(), Some((text, snapshot))));
}

pub(super) fn clear_x_clipboard_cache(ui: &Ui) {
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

pub(super) fn read_clipboard_text_for_x_paste(ui: &Ui, app: &UiApp) -> Option<String> {
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

pub(super) fn extract_multiline_or_tab_x_input(ui: &Ui, app: &UiApp) -> Option<String> {
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

pub(super) fn push_x_cell_text_paste_intents(
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
                intents
                    .push_engine("table.setXCells", json!({ "sheet_id": ds.id.as_str(), "row": xr, "subcol": xs, "value": serde_json::Value::Null }));
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

pub(super) fn handle_multi_keyboard_shortcuts(ui: &mut Ui, mctx: &MultiGridCtx<'_>, fstate: &mut MultiGridFrameState<'_>) {
    let app = mctx.app;
    let ds = mctx.ds;
    let geom = mctx.geom;
    if fstate.state.inline_edit.is_some() || fstate.aux.aux_edit.is_some() || ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    if !is_grid_focus_active(ui, app, &ds.id) && fstate.aux.selected_x_cell.is_none() && app.grid.tool.active.is_none() {
        return;
    }
    if let Some(e_mods) = pressed_key_event_modifiers(ui, Key::E)
        && (e_mods.command || e_mods.mac_cmd || e_mods.ctrl)
    {
        if e_mods.shift {
            fstate.intents.push_ui("ui.grid.excludeWithReason", json!({ "sheet_id": ds.id.as_str() }));
            return;
        }
        if let Some((xr, xs)) = fstate.aux.selected_x_cell {
            let was_ex = ds.archetype.x_or_time_cell(xr, xs).is_some_and(|c| c.excluded);
            fstate.intents.push_engine(
                "table.excludeCells",
                json!({ "sheet_id": ds.id.as_str(), "target": "x", "row": xr, "subcol": xs, "excluded": !was_ex }),
            );
            return;
        }
        if let Some(range) = app.grid.tool.selection_bounds() {
            let target_excluded = !ds.archetype.data_cell(CellCoord::from(range.0)).is_some_and(|c| c.excluded);
            let doc_range = SheetCellRange::new(CellCoord::from(range.0), CellCoord::from(range.1));
            fstate.intents.push_engine("table.excludeCells", json!({ "sheet_id": ds.id.as_str(), "range": doc_range, "excluded": target_excluded }));
            return;
        }
    }

    if handle_multi_clipboard_shortcuts(ui, app, ds, fstate.aux, fstate.intents) {
        return;
    }

    if (pressed_key_event_modifiers(ui, Key::Delete).is_some() || pressed_key_event_modifiers(ui, Key::Backspace).is_some()) && !ds.frozen {
        if let Some((xr, xs)) = fstate.aux.selected_x_cell {
            if !is_x_column_readonly(ds) {
                fstate
                    .intents
                    .push_engine("table.setXCells", json!({ "sheet_id": ds.id.as_str(), "row": xr, "subcol": xs, "value": serde_json::Value::Null }));
            }
            return;
        }
        if let Some(range) = app.grid.tool.selection_bounds() {
            let doc_range = SheetCellRange::new(CellCoord::from(range.0), CellCoord::from(range.1));
            fstate.intents.push_engine("table.clearCells", json!({ "sheet_id": ds.id.as_str(), "range": doc_range }));
            return;
        }
    }

    for (key, dx, dy) in [(Key::ArrowLeft, -1_isize, 0_isize), (Key::ArrowRight, 1, 0), (Key::ArrowUp, 0, -1), (Key::ArrowDown, 0, 1)] {
        if let Some(km) = pressed_key_event_modifiers(ui, key) {
            if geom.row_count == 0 || geom.total_flat_cols == 0 {
                return;
            }
            let cur_row = fstate.aux.selected_x_cell.map(|(r, _)| r).or_else(|| app.grid.tool.active.map(|c| c.row)).unwrap_or(0);
            let cur_flat = match (fstate.aux.selected_x_cell, app.grid.tool.active) {
                (Some((_, xs)), _) => xs,
                (None, Some(c)) => geom.coord_to_flat_all(false, c.col, c.subcol),
                (None, None) => 0,
            };
            let next_row = cur_row.saturating_add_signed(dy).min(geom.row_count.saturating_sub(1));
            let next_flat = cur_flat.saturating_add_signed(dx).min(geom.total_flat_cols.saturating_sub(1));
            if next_flat < geom.x_subcols {
                fstate.aux.selected_x_cell = Some((next_row, next_flat));
                ui.ctx().request_repaint();
            } else if let Some((nc, ns)) = geom.flat_data_to_coord(next_flat - geom.x_subcols) {
                fstate.aux.selected_x_cell = None;
                let coord = GridCellCoord::new(next_row, nc, ns);
                if km.shift {
                    fstate.intents.push_extend_grid_selection(coord);
                } else {
                    fstate.intents.push_select_grid_cell(coord);
                }
                ui.ctx().request_repaint();
            }
            return;
        }
    }

    if let Some(km) = pressed_key_event_modifiers(ui, Key::Tab) {
        let (is_x, nc_opt, x_opt) = step_cell_horiz(geom, app.grid.tool.active, fstate.aux.selected_x_cell, km.shift);
        if is_x {
            fstate.aux.selected_x_cell = x_opt;
            ui.ctx().request_repaint();
        } else if let Some(nc) = nc_opt {
            fstate.aux.selected_x_cell = None;
            fstate.intents.push_select_grid_cell(nc);
            ui.ctx().request_repaint();
        }
        return;
    }

    if pressed_key_event_modifiers(ui, Key::F2).is_some() && !ds.frozen {
        if let Some((xr, xs)) = fstate.aux.selected_x_cell {
            let init = initial_edit_text_for_cell(ds.archetype.x_or_time_cell(xr, xs), &app.locale_preset);
            fstate.aux.aux_edit = Some((AuxInlineTarget::XCell { row: xr, subcol: xs }, init, true));
            return;
        }
        if let Some(c) = app.grid.tool.active {
            let init = initial_edit_text_for_cell(ds.archetype.data_cell(c.into()), &app.locale_preset);
            begin_inline_edit(ui, fstate.state, GridInlineEditTarget::Cell(c), init);
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
            if let Some((xr, xs)) = fstate.aux.selected_x_cell {
                let x_ro = matches!(&ds.archetype, TableArchetype::Xy(xy) if matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. }));
                if !x_ro {
                    fstate.aux.aux_edit = Some((AuxInlineTarget::XCell { row: xr, subcol: xs }, typed, true));
                }
            } else if let Some(c) = app.grid.tool.active {
                let is_calc = matches!(&ds.archetype, TableArchetype::MultipleVariables(mv) if mv.variables.get(c.col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. })));
                if !is_calc && linked_region_for_cell(ds, c.row, c.col).is_none() {
                    begin_inline_edit(ui, fstate.state, GridInlineEditTarget::Cell(c), typed);
                }
            }
        }
    }
}

pub(super) fn step_cell_horiz(
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
