//! Project Navigator, Family lineage tree, and Info / Floating Notes view builders (`R10.17`).

use std::collections::BTreeMap;

use curvebook_doc::{FloatingNote, ProjectDocument, SheetFolderKind, SheetId, SheetSection};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::workspace_nav::{ActiveWorkspacePane, live_sheet_index_in_folder};

const SEARCH_WIDGET_ID: &str = "navigator.search_input";

/// Transient view state owned by the navigator / notes shard.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NavNotesViewState {
    pub filter_query: String,
    pub active_note_id: Option<String>,
    #[serde(default)]
    pub grabbed_note_id: Option<String>,
    #[serde(default)]
    pub note_positions_pt: BTreeMap<String, [f64; 2]>,
    #[serde(default)]
    pub note_collapsed_overrides: BTreeMap<String, bool>,
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `nav_notes` domain.
///
/// # Errors
/// Returns an error if the custom payload is malformed or a triggered engine/UI command fails.
pub fn handle_custom_action(app: &mut UiApp, state: &mut NavNotesViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if payload == "search.clear" {
        return handle_search_clear(app, state);
    }
    let parts: Vec<&str> = payload.split('|').collect();
    match parts.as_slice() {
        ["nav.create", kind] => handle_nav_create(app, kind),
        ["nav.reorder", folder_raw, sheet_id, delta_raw] => handle_nav_reorder(app, folder_raw, sheet_id, delta_raw),
        ["family.select", sheet_id] => handle_family_select(app, sheet_id),
        ["note.toggle", sheet_id, note_id] => handle_note_toggle(app, state, sheet_id, note_id),
        ["note.drag", sheet_id, note_id] => handle_note_drag_handle(app, state, sheet_id, note_id),
        ["note.move", sheet_id, note_id, dx_raw, dy_raw] => handle_note_move(app, state, sheet_id, note_id, dx_raw, dy_raw),
        ["note.set_pos", sheet_id, note_id, x_raw, y_raw] => handle_note_set_pos(app, state, sheet_id, note_id, x_raw, y_raw),
        _ => Ok(UiEventOutcome::default()),
    }
}

fn handle_search_clear(app: &mut UiApp, state: &mut NavNotesViewState) -> Result<UiEventOutcome, String> {
    app.interaction.text_field_buffers.remove(SEARCH_WIDGET_ID);
    state.filter_query.clear();
    let doc = app.document().clone();
    app.nav.set_search_query("", &doc);
    Ok(UiEventOutcome { handled: true, action: String::from("nav.search_cleared"), ..Default::default() })
}

fn handle_nav_create(app: &mut UiApp, kind: &str) -> Result<UiEventOutcome, String> {
    app.nav.active_pane = ActiveWorkspacePane::Navigator;
    match kind {
        "data" => {
            let outcome = app.execute_command("sheet.createData", json!({})).map_err(|e| e.to_string())?;
            let val = serde_json::to_value(&outcome).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("nav.create:data"),
                command_id: Some(String::from("sheet.createData")),
                command_result: Some(val),
            })
        }
        "info" => {
            let Some(base_info) = app.document().info_sheets.first().map(|s| s.id.clone()) else {
                return Err(String::from("no existing Info sheet available to template a new Info sheet"));
            };
            let title = format!("Info {}", app.document().info_sheets.len() + 1);
            let dup = app.execute_command("sheet.duplicate", json!({ "sheet_id": base_info.as_str(), "title": title })).map_err(|e| e.to_string())?;
            if let Some(new_id) = dup.payload.get("sheet_id").and_then(serde_json::Value::as_str) {
                let _ = app.execute_command("info.setNote", json!({ "sheet_id": new_id, "text": "" }));
                let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
            }
            let val = serde_json::to_value(&dup).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("nav.create:info"),
                command_id: Some(String::from("sheet.duplicate")),
                command_result: Some(val),
            })
        }
        "results" => {
            let res = crate::control_io::dispatch_ui_command(app, "ui.analyze.open", &json!({}))?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("nav.create:results"),
                command_id: Some(String::from("ui.analyze.open")),
                command_result: res,
            })
        }
        "graphs" => {
            let src_id = resolve_graph_source_sheet_id(app.document()).ok_or_else(|| String::from("no Data Table available to create a graph"))?;
            let outcome = app.execute_command("graph.create", json!({ "data_sheet_id": src_id.as_str() })).map_err(|e| e.to_string())?;
            if let Some(new_id) = outcome.payload.get("sheet_id").and_then(serde_json::Value::as_str) {
                let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
            }
            let val = serde_json::to_value(&outcome).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("nav.create:graphs"),
                command_id: Some(String::from("graph.create")),
                command_result: Some(val),
            })
        }
        "layouts" => {
            let outcome = app.execute_command("layout.create", json!({})).map_err(|e| e.to_string())?;
            if let Some(new_id) = outcome.payload.get("sheet_id").and_then(serde_json::Value::as_str) {
                let _ = app.execute_command("sheet.select", json!({ "sheet_id": new_id }));
            }
            let val = serde_json::to_value(&outcome).map_err(|e| e.to_string())?;
            Ok(UiEventOutcome {
                handled: true,
                action: String::from("nav.create:layouts"),
                command_id: Some(String::from("layout.create")),
                command_result: Some(val),
            })
        }
        other => Err(format!("unknown navigator creation target `{other}`")),
    }
}

fn resolve_graph_source_sheet_id(doc: &ProjectDocument) -> Option<SheetId> {
    if let Some(active_id) = &doc.active_sheet_id {
        if doc.data_sheet(active_id).is_some() {
            return Some(active_id.clone());
        }
        if let Some(rs) = doc.result_sheet(active_id)
            && rs.analyzable_data.is_some()
        {
            return Some(rs.id.clone());
        }
    }
    doc.data_sheets.first().map(|s| s.id.clone())
}

fn handle_nav_reorder(app: &mut UiApp, folder_raw: &str, sheet_id: &str, delta_raw: &str) -> Result<UiEventOutcome, String> {
    let folder = parse_folder_kind(folder_raw).ok_or_else(|| format!("unknown navigator folder `{folder_raw}`"))?;
    let delta: isize = delta_raw.parse().map_err(|_| format!("invalid reorder delta `{delta_raw}`"))?;
    let doc = app.document();
    let count = folder_sheet_count(doc, folder);
    let Some(cur_idx) = live_sheet_index_in_folder(doc, folder, sheet_id) else {
        return Err(format!("sheet `{sheet_id}` not found in folder `{folder_raw}`"));
    };
    let max_idx = count.saturating_sub(1);
    let target_idx = cur_idx.saturating_add_signed(delta).min(max_idx);
    if target_idx == cur_idx {
        return Ok(UiEventOutcome { handled: true, action: String::from("nav.reorder_noop"), ..Default::default() });
    }
    let outcome = app.execute_command("sheet.reorder", json!({ "sheet_id": sheet_id, "index": target_idx })).map_err(|e| e.to_string())?;
    let val = serde_json::to_value(&outcome).map_err(|e| e.to_string())?;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("nav.reorder:{sheet_id}->{target_idx}"),
        command_id: Some(String::from("sheet.reorder")),
        command_result: Some(val),
    })
}

fn handle_family_select(app: &mut UiApp, sheet_id: &str) -> Result<UiEventOutcome, String> {
    let sid = SheetId::try_new(sheet_id).map_err(|e| e.to_string())?;
    let mode = if app.document().sheet_section(&sid) == Some(SheetSection::Data) { "navigator" } else { "family" };
    let outcome = app.execute_command("sheet.select", json!({ "sheet_id": sid.as_str(), "mode": mode })).map_err(|e| e.to_string())?;
    crate::interaction::sync_dialogs_on_sheet_switch(app, &sid);
    app.nav.active_pane = ActiveWorkspacePane::FamilyStrip;
    let val = serde_json::to_value(&outcome).map_err(|e| e.to_string())?;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("family.select:{sheet_id}"),
        command_id: Some(String::from("sheet.select")),
        command_result: Some(val),
    })
}

fn handle_note_toggle(app: &mut UiApp, state: &mut NavNotesViewState, sheet_id: &str, note_id: &str) -> Result<UiEventOutcome, String> {
    let sid = SheetId::new(sheet_id);
    let doc_min = sheet_floating_notes(app.document(), &sid).iter().find(|n| n.id.as_str() == note_id).is_some_and(|n| n.minimized);
    let cur = app.nav.floating_notes_collapsed.get(note_id).copied().unwrap_or(doc_min);
    let next = !cur;
    app.nav.floating_notes_collapsed.insert(note_id.to_owned(), next);
    state.note_collapsed_overrides.insert(note_state_key(&sid, note_id), next);
    state.active_note_id = Some(note_id.to_owned());
    app.nav.active_pane = ActiveWorkspacePane::FloatingNotes;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("floating_note.toggle:{note_id}={next}"),
        command_result: Some(json!({ "sheet_id": sheet_id, "note_id": note_id, "collapsed": next })),
        ..Default::default()
    })
}

fn handle_note_drag_handle(app: &mut UiApp, state: &mut NavNotesViewState, sheet_id: &str, note_id: &str) -> Result<UiEventOutcome, String> {
    state.active_note_id = Some(note_id.to_owned());
    let key = note_state_key(&SheetId::new(sheet_id), note_id);
    let next_grabbed = if state.grabbed_note_id.as_deref() == Some(key.as_str()) { None } else { Some(key) };
    state.grabbed_note_id = next_grabbed.clone();
    app.nav.active_pane = ActiveWorkspacePane::FloatingNotes;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("floating_note.drag_handle:{note_id}"),
        command_result: Some(json!({ "sheet_id": sheet_id, "note_id": note_id, "grabbed": next_grabbed.is_some() })),
        ..Default::default()
    })
}

fn handle_note_move(
    app: &mut UiApp,
    state: &mut NavNotesViewState,
    sheet_id: &str,
    note_id: &str,
    dx_raw: &str,
    dy_raw: &str,
) -> Result<UiEventOutcome, String> {
    let dx: f64 = dx_raw.parse().map_err(|_| format!("invalid note dx `{dx_raw}`"))?;
    let dy: f64 = dy_raw.parse().map_err(|_| format!("invalid note dy `{dy_raw}`"))?;
    if !dx.is_finite() || !dy.is_finite() {
        return Err(String::from("floating note delta must be finite"));
    }
    let sid = SheetId::new(sheet_id);
    let key = note_state_key(&sid, note_id);
    let base = state.note_positions_pt.get(&key).copied().unwrap_or_else(|| {
        sheet_floating_notes(app.document(), &sid).iter().find(|n| n.id.as_str() == note_id).map_or([12.0, 12.0], |n| [n.bounds_pt.x, n.bounds_pt.y])
    });
    let next = [(base[0] + dx).clamp(0.0, 2400.0), (base[1] + dy).clamp(0.0, 2400.0)];
    state.note_positions_pt.insert(key, next);
    state.active_note_id = Some(note_id.to_owned());
    app.nav.active_pane = ActiveWorkspacePane::FloatingNotes;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("floating_note.move:{note_id}"),
        command_result: Some(json!({ "sheet_id": sheet_id, "note_id": note_id, "x_pt": next[0], "y_pt": next[1] })),
        ..Default::default()
    })
}

fn handle_note_set_pos(
    app: &mut UiApp,
    state: &mut NavNotesViewState,
    sheet_id: &str,
    note_id: &str,
    x_raw: &str,
    y_raw: &str,
) -> Result<UiEventOutcome, String> {
    let x: f64 = x_raw.parse().map_err(|_| format!("invalid note x `{x_raw}`"))?;
    let y: f64 = y_raw.parse().map_err(|_| format!("invalid note y `{y_raw}`"))?;
    if !x.is_finite() || !y.is_finite() {
        return Err(String::from("floating note position must be finite"));
    }
    let pos = [x.clamp(0.0, 2400.0), y.clamp(0.0, 2400.0)];
    state.note_positions_pt.insert(note_state_key(&SheetId::new(sheet_id), note_id), pos);
    state.active_note_id = Some(note_id.to_owned());
    state.grabbed_note_id = None;
    app.nav.active_pane = ActiveWorkspacePane::FloatingNotes;
    Ok(UiEventOutcome {
        handled: true,
        action: format!("floating_note.set_pos:{note_id}"),
        command_result: Some(json!({ "sheet_id": sheet_id, "note_id": note_id, "x_pt": pos[0], "y_pt": pos[1] })),
        ..Default::default()
    })
}

pub(crate) fn sheet_floating_notes<'a>(doc: &'a ProjectDocument, id: &SheetId) -> &'a [FloatingNote] {
    if let Some(ds) = doc.data_sheet(id) {
        &ds.floating_notes
    } else if let Some(info) = doc.info_sheet(id) {
        &info.floating_notes
    } else if let Some(rs) = doc.result_sheet(id) {
        &rs.floating_notes
    } else if let Some(gs) = doc.graph_sheet(id) {
        &gs.floating_notes
    } else if let Some(ls) = doc.layout_sheet(id) {
        &ls.floating_notes
    } else {
        &[]
    }
}

fn note_state_key(sheet_id: &SheetId, note_id: &str) -> String {
    format!("{}::{note_id}", sheet_id.as_str())
}

fn folder_sheet_count(doc: &ProjectDocument, folder: SheetFolderKind) -> usize {
    match folder {
        SheetFolderKind::Data => doc.data_sheets.len(),
        SheetFolderKind::Info => doc.info_sheets.len(),
        SheetFolderKind::Results => doc.result_sheets.len(),
        SheetFolderKind::Graphs => doc.graph_sheets.len(),
        SheetFolderKind::Layouts => doc.layout_sheets.len(),
    }
}

fn parse_folder_kind(raw: &str) -> Option<SheetFolderKind> {
    match raw {
        "data" | "Data" => Some(SheetFolderKind::Data),
        "info" | "Info" => Some(SheetFolderKind::Info),
        "results" | "Results" => Some(SheetFolderKind::Results),
        "graphs" | "Graphs" => Some(SheetFolderKind::Graphs),
        "layouts" | "Layouts" => Some(SheetFolderKind::Layouts),
        _ => None,
    }
}
