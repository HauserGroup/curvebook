//! Project Navigator, Family lineage tree, and Info / Floating Notes view state & live actions (`R10.17`).

use std::collections::BTreeMap;

use curvebook_doc::{FloatingNote, ProjectDocument, SheetId};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::UiEventOutcome;
use crate::workspace_nav::ActiveWorkspacePane;

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
    let parts: Vec<&str> = payload.split('|').collect();
    match parts.as_slice() {
        ["note.toggle", sheet_id, note_id] => handle_note_toggle(app, state, sheet_id, note_id),
        ["note.set_pos", sheet_id, note_id, x_raw, y_raw] => handle_note_set_pos(app, state, sheet_id, note_id, x_raw, y_raw),
        _ => Ok(UiEventOutcome::default()),
    }
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
