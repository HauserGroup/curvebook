//! Toolbar and context menu view shard state and clipboard/cell-action dispatchers (`R10.6`).

use curvebook_doc::{DataSheet, ProjectDocument, ResultSheet, SheetCellRange, SheetId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::ResultSheetViewState;
use crate::interaction::UiEventOutcome;
use crate::os_integration::ClipboardBundle;

/// Transient view state owned by the toolbar / menus / status bar shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolbarMenusViewState {
    pub overflow_open: bool,
    pub active_context_menu_id: Option<String>,
    #[serde(default)]
    pub context_menu_anchor_px: Option<[i32; 2]>,
    #[serde(default)]
    pub context_sheet_id: Option<String>,
    #[serde(default)]
    pub text_bold: bool,
    #[serde(default)]
    pub text_italic: bool,
    #[serde(default)]
    pub text_font_size_pt: u16,
    #[serde(default)]
    pub last_toolbar_notice: Option<String>,
}

fn active_data_sheet(doc: &ProjectDocument) -> Option<&DataSheet> {
    doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id))
}

fn active_sheet_title(doc: &ProjectDocument) -> Option<&str> {
    let id = doc.active_sheet_id.as_ref()?;
    doc.data_sheet(id)
        .map(|s| s.title.as_str())
        .or_else(|| doc.result_sheet(id).map(ResultSheet::title))
        .or_else(|| doc.graph_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.layout_sheet(id).map(|s| s.title.as_str()))
        .or_else(|| doc.info_sheet(id).map(|s| s.title.as_str()))
}

fn active_linked_region<'a>(app: &UiApp, ds: &'a DataSheet) -> Option<&'a curvebook_doc::LinkedDataRegion> {
    let probe = app.grid.tool.active.or_else(|| app.grid.selected_coords.first().copied());
    if let Some(c) = probe
        && let Some(found) = ds.linked_regions.iter().find(|reg| {
            let r0 = reg.target_range.start.row.min(reg.target_range.end.row);
            let r1 = reg.target_range.start.row.max(reg.target_range.end.row);
            let cs0 = (reg.target_range.start.col, reg.target_range.start.subcol).min((reg.target_range.end.col, reg.target_range.end.subcol));
            let cs1 = (reg.target_range.start.col, reg.target_range.start.subcol).max((reg.target_range.end.col, reg.target_range.end.subcol));
            let cs = (c.col, c.subcol);
            c.row >= r0 && c.row <= r1 && cs >= cs0 && cs <= cs1
        })
    {
        return Some(found);
    }
    ds.linked_regions.first()
}

enum PreparedClipboardCopy {
    Text(String),
    RenderVisual { sid: SheetId, fallback_title: String },
}

fn prepare_active_sheet_copy(app: &UiApp) -> Result<PreparedClipboardCopy, String> {
    let doc = app.session.document();
    let Some(sid) = doc.active_sheet_id.as_ref() else {
        return Err(String::from("no active sheet to copy"));
    };
    let dec = app.grid.decimal_separator;
    if let Some(ds) = doc.data_sheet(sid) {
        Ok(PreparedClipboardCopy::Text(app.grid.prepare_clipboard_selection(ds, dec)?.tsv))
    } else if let Some(rs) = doc.result_sheet(sid) {
        let mut lines = Vec::new();
        if let Some(tab) = rs.tabs.first() {
            if !tab.column_headers.is_empty() {
                lines.push(tab.column_headers.join("\t"));
            }
            for row in &tab.rows {
                let cols: Vec<String> = row.iter().map(|cell| ResultSheetViewState::format_result_cell(cell, rs.analysis.p_value_format)).collect();
                lines.push(cols.join("\t"));
            }
        } else {
            lines.push(rs.title().to_owned());
        }
        Ok(PreparedClipboardCopy::Text(lines.join("\n")))
    } else if let Some(info) = doc.info_sheet(sid) {
        let mut lines = vec![info.title.clone()];
        if !info.notes_markdown.is_empty() {
            lines.push(info.notes_markdown.clone());
        }
        for c in &info.constants {
            lines.push(format!("{}\t{:?}", c.name, c.value));
        }
        Ok(PreparedClipboardCopy::Text(lines.join("\n")))
    } else if doc.graph_sheet(sid).is_some() || doc.layout_sheet(sid).is_some() {
        Ok(PreparedClipboardCopy::RenderVisual { sid: sid.clone(), fallback_title: active_sheet_title(doc).unwrap_or("Figure").to_owned() })
    } else {
        Err(String::from("active sheet is not copyable"))
    }
}

fn copy_active_sheet_to_clipboard(app: &mut UiApp) -> Result<String, String> {
    let payload = match prepare_active_sheet_copy(app)? {
        PreparedClipboardCopy::Text(text) => text,
        PreparedClipboardCopy::RenderVisual { sid, fallback_title } => match app.session().render_sheet_scene(&sid) {
            Ok(scene) => {
                let bundle = ClipboardBundle::from_scene(&scene, 144)?;
                let _ = app.write_clipboard_bundle(&bundle);
                let text = if !bundle.svg.is_empty() { bundle.svg.clone() } else { fallback_title };
                let line_count = text.lines().count().max(1);
                app.interaction.last_clipboard_text = Some(text);
                app.interaction.last_clipboard_bundle = Some(bundle);
                return Ok(format!("Copied {line_count} line(s)"));
            }
            Err(_) => fallback_title,
        },
    };
    let line_count = payload.lines().count().max(1);
    let _ = app.write_clipboard_text(&payload);
    app.interaction.last_clipboard_bundle = None;
    app.interaction.last_clipboard_text = Some(payload);
    Ok(format!("Copied {line_count} line(s)"))
}

fn range_to_json(range: SheetCellRange) -> Value {
    json!({
        "start": { "row": range.start.row, "col": range.start.col, "subcol": range.start.subcol },
        "end": { "row": range.end.row, "col": range.end.col, "subcol": range.end.subcol },
    })
}

enum ExcludePlan {
    SingleCommand { command_id: &'static str, params: Value, notice: String },
    MultipleCommands { intents: Vec<curvebook_tools::ToolCommandIntent>, notice: String },
}

fn prepare_exclude_plan(app: &mut UiApp, exclude: bool) -> Result<ExcludePlan, String> {
    let doc = app.session.document();
    let Some(ds) = active_data_sheet(doc) else {
        return Err(String::from("no active data sheet"));
    };
    if ds.frozen {
        return Err(format!("sheet `{}` is frozen", ds.title));
    }
    let sheet_id = ds.id.clone();

    if app.grid.all_selected_coords(ds).is_empty()
        && let Some((row, subcol)) = app.grid.active_x_cell
    {
        return Ok(ExcludePlan::SingleCommand {
            command_id: "table.excludeCells",
            params: json!({ "sheet_id": sheet_id.as_str(), "target": "x", "excluded": exclude, "row": row, "subcol": subcol }),
            notice: String::from(if exclude { "Excluded X cell" } else { "Included X cell" }),
        });
    }

    app.grid.tool.pending_intents.clear();
    let count = app.grid.emit_toggle_exclude_intents(ds, exclude);
    let intents = std::mem::take(&mut app.grid.tool.pending_intents);
    if !intents.is_empty() {
        return Ok(ExcludePlan::MultipleCommands { intents, notice: format!("{} {count} range(s)", if exclude { "Excluded" } else { "Included" }) });
    }

    let Some(c) = app.grid.tool.active else {
        return Err(String::from("no cell selected for exclude/include"));
    };
    let single = SheetCellRange::new(curvebook_doc::CellCoord::from(c), curvebook_doc::CellCoord::from(c));
    Ok(ExcludePlan::SingleCommand {
        command_id: "table.excludeCells",
        params: json!({ "sheet_id": sheet_id.as_str(), "excluded": exclude, "range": range_to_json(single) }),
        notice: String::from(if exclude { "Excluded cell" } else { "Included cell" }),
    })
}

fn run_exclude_action(app: &mut UiApp, exclude: bool) -> Result<String, String> {
    match prepare_exclude_plan(app, exclude)? {
        ExcludePlan::SingleCommand { command_id, params, notice } => {
            app.execute_command(command_id, params).map_err(|e| e.to_string())?;
            app.grid.refresh_selection_stats(app.session.document());
            Ok(notice)
        }
        ExcludePlan::MultipleCommands { intents, notice } => {
            for intent in intents {
                app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
            }
            app.grid.refresh_selection_stats(app.session.document());
            Ok(notice)
        }
    }
}

fn dismiss_context_overlays(app: &mut UiApp, state: &mut ToolbarMenusViewState) {
    state.active_context_menu_id = None;
    state.context_menu_anchor_px = None;
    state.context_sheet_id = None;
    state.overflow_open = false;
    app.interaction.context_menu_target = None;
    app.interaction.open_menu = None;
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `toolbar_menus` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut ToolbarMenusViewState, payload: &str) -> Result<UiEventOutcome, String> {
    dismiss_context_overlays(app, state);
    match payload {
        "clipboard.copy" => {
            let msg = copy_active_sheet_to_clipboard(app)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "clipboard.cut" => {
            let msg = copy_active_sheet_to_clipboard(app)?;
            let clear_requests = {
                let doc = app.session.document();
                active_data_sheet(doc).filter(|ds| !ds.frozen).map(|ds| app.grid.clipboard_target(ds).clear_cells_requests(ds)).unwrap_or_default()
            };
            if !clear_requests.is_empty() {
                for req in clear_requests {
                    app.execute_command("table.clearCells", req).map_err(|e| e.to_string())?;
                }
                app.grid.refresh_selection_stats(app.session.document());
            }
            state.last_toolbar_notice = Some(format!("Cut ({msg})"));
            Ok(UiEventOutcome { handled: true, action: String::from("clipboard.cut"), ..Default::default() })
        }
        "ctx.exclude" => {
            let msg = run_exclude_action(app, true)?;
            state.last_toolbar_notice = Some(msg.clone());
            Ok(UiEventOutcome { handled: true, action: String::from(payload), command_result: Some(json!({ "notice": msg })), ..Default::default() })
        }
        "ctx.clear_cells" => {
            let (frozen, requests) = {
                let doc = app.session.document();
                let ds = active_data_sheet(doc).ok_or_else(|| String::from("no active data sheet"))?;
                if ds.frozen { (true, Vec::new()) } else { (false, app.grid.clipboard_target(ds).clear_cells_requests(ds)) }
            };
            if frozen {
                state.last_toolbar_notice = Some(String::from(crate::i18n::tr(&app.locale, "w7.mv.frozen_clear_blocked")));
                return Ok(UiEventOutcome { handled: true, action: String::from("grid_core.frozen_read_only"), ..Default::default() });
            }
            if requests.is_empty() {
                state.last_toolbar_notice = Some(String::from("No cells selected to clear"));
                return Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() });
            }
            let mut cleared_total = 0_u64;
            for req in requests {
                let out = app.execute_command("table.clearCells", req).map_err(|e| e.to_string())?;
                cleared_total += out.payload.get("cleared_count").and_then(Value::as_u64).unwrap_or(0);
            }
            app.grid.refresh_selection_stats(app.session.document());
            state.last_toolbar_notice = Some(format!("Cleared {cleared_total} cell(s)"));
            Ok(UiEventOutcome { handled: true, action: String::from(payload), ..Default::default() })
        }
        "ctx.unlink_region" => {
            let (sheet_id, region_id) = {
                let doc = app.session.document();
                let ds = active_data_sheet(doc).ok_or_else(|| String::from("no active data sheet"))?;
                let region_id =
                    active_linked_region(app, ds).map(|reg| reg.id.clone()).ok_or_else(|| String::from("no linked region on active data sheet"))?;
                (ds.id.clone(), region_id)
            };
            app.execute_command("table.clearCells", json!({ "sheet_id": sheet_id.as_str(), "unlink": true, "region_id": region_id, "cells": [] }))
                .map_err(|e| e.to_string())?;
            state.last_toolbar_notice = Some(String::from("Detached linked region"));
            Ok(UiEventOutcome {
                handled: true,
                action: String::from(payload),
                command_id: Some(String::from("table.clearCells")),
                ..Default::default()
            })
        }
        _ => Ok(UiEventOutcome::default()),
    }
}
