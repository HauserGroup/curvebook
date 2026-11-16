//! Project Navigator, Family lineage tree, and Info / Floating Notes view builders (`R10.17`).

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{FloatingNote, InfoSheet, ProjectDocument, SheetFolderKind, SheetId, SheetSection, TableKind};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::UiApp;
use crate::interaction::{InfoRowHitRect, InlineEditorTarget, NavigatorEntryHitRect, UiEventOutcome};
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta, scrollbar_thickness_px};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, emit_button, emit_scrollbars_for_meta, emit_text_field, paint_text_fitted, widget_state,
};
use crate::layout::{LayoutBuilder, LayoutRectPx, NodeSpec, TextRunSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{TextMeasurer, UiTextStyle, ellipsize_text};
use crate::workspace_nav::{ActiveWorkspacePane, FamilyPaneNode, FamilyRelation, NavigatorEntry, live_sheet_index_in_folder};

use super::a11y::{disclosure_meta, labeled_meta, selectable_meta};

const SEARCH_WIDGET_ID: &str = "navigator.search_input";
const SEARCH_CLEAR_ID: &str = "navigator.search_clear";
const NAV_SCROLL_ID: &str = "scroll.navigator";
const FAMILY_SCROLL_ID: &str = "scroll.family";
const INFO_CONST_SCROLL_ID: &str = "scroll.info_constants";

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

/// Build the Project Navigator folder tree and bottom Family pane inside `rect`.
pub fn build_navigator_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width < 24 || rect.height < 24 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, rect, builder.palette.panel);
    builder.paint_stroke(ZLayer::Workspace, None, rect, builder.palette.border, 1);

    let fam_h = if builder.app.family_open && rect.height >= 160 { rect.height / 3 } else { 24u32.min(rect.height) };
    let tree_h = rect.height.saturating_sub(fam_h);
    let tree_rect = LayoutRectPx::new(rect.x, rect.y, rect.width, tree_h);
    let fam_rect = LayoutRectPx::new(rect.x, rect.y + i32::try_from(tree_h).unwrap_or(0), rect.width, fam_h);

    build_navigator_tree_section(builder, tree_rect);
    build_family_pane(builder, fam_rect);

    if !builder.app.presentation_mode && !builder.app.nav.gallery.open {
        let canvas_rect = LayoutRectPx::from_workspace_rect(builder.tree.frame.canvas);
        if let Some(active_id) = builder.app.document().active_sheet_id.clone() {
            let notes = sheet_floating_notes(builder.app.document(), &active_id).to_vec();
            if !notes.is_empty() {
                build_floating_notes_overlay(builder, &active_id, &notes, canvas_rect);
            }
        }
    }
}

fn build_navigator_tree_section(builder: &mut LayoutBuilder<'_>, tree_rect: LayoutRectPx) {
    if tree_rect.width < 24 || tree_rect.height < 24 {
        return;
    }
    let scale = builder.scale();
    let search_h = round_u32(28.0 * scale).clamp(24, 48);
    let show_search = tree_rect.width >= 90 && tree_rect.height >= search_h + 24;
    let active_query = active_search_query(builder).to_owned();

    let list_rect = if show_search {
        let bar_rect = LayoutRectPx::new(tree_rect.x + 4, tree_rect.y + 4, tree_rect.width.saturating_sub(8), search_h.saturating_sub(4));
        build_navigator_search_bar(builder, bar_rect, &active_query);
        let used_h = search_h + 4;
        LayoutRectPx::new(tree_rect.x, tree_rect.y + i32::try_from(used_h).unwrap_or(32), tree_rect.width, tree_rect.height.saturating_sub(used_h))
    } else {
        tree_rect
    };

    if list_rect.width < 24 || list_rect.height < 20 {
        return;
    }

    let row_h = round_u32(22.0 * scale).clamp(20, 40);
    let step_y = row_h + 2;
    let needle = active_query.trim().to_lowercase();
    let rows_plan = count_navigator_rows(builder.app, &needle);
    let content_h = (rows_plan as u32).saturating_mul(step_y) + 8;

    let scroll_off = builder.scroll_offset(NAV_SCROLL_ID);
    let scroll_meta = ScrollMeta::new(String::from(NAV_SCROLL_ID), list_rect, [list_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let mut tree_a11y = labeled_meta(WidgetRole::TreeItem, "Project Navigator", FocusScope::Workspace, "Cmd+Shift+N");
    tree_a11y.expanded = Some(true);
    let tree_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("navigator.tree"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: list_rect,
        a11y: tree_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&tree_wid, scroll_meta.clone());
    builder.paint_stroke(ZLayer::Workspace, Some(&tree_wid), list_rect, builder.palette.border, 1);

    let sb_reserve = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) + 2 } else { 2 };
    let inner = list_rect.inset(3);
    if inner.width < 18 || inner.height < 12 {
        return;
    }
    let content_local_x = if builder.tree.is_rtl { sb_reserve } else { 0 };
    let avail_w = inner.width.saturating_sub(sb_reserve).max(16);

    builder.push_clip(inner);
    let y_scroll = round_i32(scroll_meta.scroll_offset_px[1]);
    let mut cur_y = inner.y + 2 - y_scroll;
    let mut matched_any = false;

    for (f_idx, folder) in builder.app.nav.folders.iter().enumerate() {
        let collapsed = folder.collapsed || builder.app.nav.collapsed_sections.contains(&folder.section);
        let folder_kind = folder_kind_for_section(folder.section);
        let filtered: Vec<&NavigatorEntry> = folder.entries.iter().filter(|e| entry_matches_query(e, &needle)).collect();
        if !filtered.is_empty() {
            matched_any = true;
        }

        let f_rect = LayoutRectPx::new(inner.x + i32::try_from(content_local_x).unwrap_or(0), cur_y, avail_w, row_h);
        if is_row_visible(f_rect, inner) {
            build_folder_header_row(
                builder,
                &tree_wid,
                FolderHeaderCtx { f_idx, section: folder.section, label: &folder.label, collapsed, visible_count: filtered.len(), rect: f_rect },
            );
        }
        cur_y += i32::try_from(step_y).unwrap_or(24);

        if collapsed {
            continue;
        }

        let doc_folder_count = folder_sheet_count(builder.app.document(), folder_kind);
        for (e_idx, entry) in filtered.iter().enumerate() {
            let live_idx = live_sheet_index_in_folder(builder.app.document(), folder_kind, entry.sheet_id.as_str()).unwrap_or(e_idx);
            let row_rect = LayoutRectPx::new(inner.x + i32::try_from(content_local_x).unwrap_or(0), cur_y, avail_w, row_h);
            if is_row_visible(row_rect, inner) {
                build_sheet_entry_row(
                    builder,
                    &tree_wid,
                    EntryRowCtx { entry, folder_kind, live_idx, doc_folder_count, filter_active: !needle.is_empty(), rect: row_rect },
                );
            }
            cur_y += i32::try_from(step_y).unwrap_or(24);
        }

        if needle.is_empty() {
            let create_rect = LayoutRectPx::new(inner.x + i32::try_from(content_local_x).unwrap_or(0), cur_y, avail_w, row_h);
            if is_row_visible(create_rect, inner) {
                build_inline_create_row(builder, &tree_wid, folder.section, create_rect);
            }
            cur_y += i32::try_from(step_y).unwrap_or(24);
        }
    }

    if !needle.is_empty() && !matched_any {
        let empty_rect = LayoutRectPx::new(inner.x + i32::try_from(content_local_x).unwrap_or(0), cur_y, avail_w, row_h);
        if is_row_visible(empty_rect, inner) {
            paint_muted_hint(
                builder,
                &tree_wid,
                empty_rect,
                nav_tr(&builder.tree.locale, "nav.no_matches", "No matching sheets"),
                Some(NAV_SCROLL_ID),
            );
        }
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(tree_wid));
}

fn active_search_query<'a>(builder: &'a LayoutBuilder<'_>) -> &'a str {
    builder.app.interaction.text_field_buffers.get(SEARCH_WIDGET_ID).map_or(builder.app.nav.search_query.as_str(), |buf| buf.text.as_str())
}

fn entry_matches_query(entry: &NavigatorEntry, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    entry.title.to_lowercase().contains(needle_lower)
        || entry.sheet_id.as_str().to_lowercase().contains(needle_lower)
        || entry.archetype_label.as_deref().is_some_and(|lbl| lbl.to_lowercase().contains(needle_lower))
}

fn count_navigator_rows(app: &UiApp, needle_lower: &str) -> usize {
    let mut rows = 0usize;
    let mut matched_any = false;
    for folder in &app.nav.folders {
        rows += 1;
        let collapsed = folder.collapsed || app.nav.collapsed_sections.contains(&folder.section);
        if collapsed {
            continue;
        }
        let count = folder.entries.iter().filter(|e| entry_matches_query(e, needle_lower)).count();
        if count > 0 {
            matched_any = true;
        }
        rows += count;
        if needle_lower.is_empty() {
            rows += 1;
        }
    }
    if !needle_lower.is_empty() && !matched_any {
        rows += 1;
    }
    rows
}

fn build_navigator_search_bar(builder: &mut LayoutBuilder<'_>, bar_rect: LayoutRectPx, active_query: &str) {
    let clear_w = bar_rect.height.clamp(18, 28);
    let gap = 4u32;
    let field_w = bar_rect.width.saturating_sub(clear_w + gap);
    if field_w < 36 {
        return;
    }
    let is_rtl = builder.tree.is_rtl;
    let field_local_x = crate::i18n::mirror_x_for_rtl(0, field_w, bar_rect.width, is_rtl);
    let clear_local_x = crate::i18n::mirror_x_for_rtl(field_w + gap, clear_w, bar_rect.width, is_rtl);

    let field_rect = LayoutRectPx::new(bar_rect.x + i32::try_from(field_local_x).unwrap_or(0), bar_rect.y, field_w, bar_rect.height);
    let clear_rect = LayoutRectPx::new(bar_rect.x + i32::try_from(clear_local_x).unwrap_or(0), bar_rect.y, clear_w, bar_rect.height);
    let label = nav_tr(&builder.tree.locale, "nav.search_label", "Search sheets");

    emit_text_field(
        builder,
        TextFieldSpec {
            id: SEARCH_WIDGET_ID,
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: field_rect,
            label,
            fallback_text: active_query,
            keyboard_path: SEARCH_WIDGET_ID,
            focus_scope: FocusScope::Workspace,
            tab_order: Some(12),
            multiline: false,
            enablement: ControlEnablement::enabled(),
        },
    );

    let clear_en =
        if active_query.trim().is_empty() { ControlEnablement::disabled("Type a filter query to clear") } else { ControlEnablement::enabled() };
    let clear_label = nav_tr(&builder.tree.locale, "nav.search_clear", "Clear search");
    let st = widget_state(builder, SEARCH_CLEAR_ID, clear_en);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(SEARCH_CLEAR_ID),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: clear_rect,
        a11y: labeled_meta(WidgetRole::Button, clear_label, FocusScope::Workspace, SEARCH_CLEAR_ID),
        state: st.clone(),
        action: if st.enablement.enabled {
            WidgetAction::Custom { domain: String::from("nav_notes"), payload: String::from("search.clear") }
        } else {
            WidgetAction::NoOp
        },
    });
    let bg = if st.hovered && st.enablement.enabled { builder.palette.header } else { builder.palette.surface };
    builder.paint_fill(ZLayer::Workspace, Some(&wid), clear_rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), clear_rect, builder.palette.border, 1);
    let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    paint_cross_icon(builder, ZLayer::Workspace, &wid, clear_rect, fg);
}

struct FolderHeaderCtx<'a> {
    f_idx: usize,
    section: SheetSection,
    label: &'a str,
    collapsed: bool,
    visible_count: usize,
    rect: LayoutRectPx,
}

fn build_folder_header_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, ctx: FolderHeaderCtx<'_>) {
    let localized = crate::i18n::sheet_section_label(&builder.tree.locale, ctx.section);
    let folder_title = if localized.is_empty() { ctx.label } else { localized };
    let fid = format!("nav.folder.{}", ctx.f_idx);
    let fst = widget_state(builder, &fid, ControlEnablement::enabled());
    let fa = disclosure_meta(folder_title, FocusScope::Workspace, format!("nav.folder.{}", ctx.f_idx), !ctx.collapsed);
    let fwid = builder.push_node(NodeSpec {
        id: WidgetId::from(fid),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: ctx.rect,
        a11y: fa,
        state: fst.clone(),
        action: WidgetAction::ToggleFolder(ctx.section),
    });
    let bg = if fst.hovered { builder.palette.header } else { builder.palette.panel };
    builder.paint_fill(ZLayer::Workspace, Some(&fwid), ctx.rect, bg);

    let is_rtl = builder.tree.is_rtl;
    let chev_w = 14u32;
    let icon_w = 14u32;
    let gap = 4u32;
    let chev_x = crate::i18n::mirror_x_for_rtl(2, chev_w, ctx.rect.width, is_rtl);
    let icon_x = crate::i18n::mirror_x_for_rtl(2 + chev_w + 2, icon_w, ctx.rect.width, is_rtl);
    let lead_used = 2 + chev_w + 2 + icon_w + gap;
    let text_w = ctx.rect.width.saturating_sub(lead_used + 2);
    let text_x = crate::i18n::mirror_x_for_rtl(lead_used, text_w, ctx.rect.width, is_rtl);

    let chev_rect = LayoutRectPx::new(ctx.rect.x + i32::try_from(chev_x).unwrap_or(0), ctx.rect.y, chev_w, ctx.rect.height);
    let icon_rect = LayoutRectPx::new(ctx.rect.x + i32::try_from(icon_x).unwrap_or(0), ctx.rect.y, icon_w, ctx.rect.height);
    let text_rect = LayoutRectPx::new(ctx.rect.x + i32::try_from(text_x).unwrap_or(0), ctx.rect.y, text_w, ctx.rect.height);

    paint_disclosure_chevron(builder, ZLayer::Workspace, &fwid, chev_rect, !ctx.collapsed, builder.palette.muted);
    paint_section_icon(builder, ZLayer::Workspace, &fwid, icon_rect, ctx.section, None, builder.palette.accent);

    let size_px = (12.0 * builder.scale()).clamp(9.0, 22.0);
    let bold_style = UiTextStyle::ui_bold(size_px);
    let txt = format!("{folder_title} ({})", ctx.visible_count);
    paint_text_fitted(
        builder,
        &fwid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: text_rect,
            pad_x: 2,
            text: &txt,
            style: &bold_style,
            color: builder.palette.text,
            scroll_container: Some(NAV_SCROLL_ID),
        },
    );
}

struct EntryRowCtx<'a> {
    entry: &'a NavigatorEntry,
    folder_kind: SheetFolderKind,
    live_idx: usize,
    doc_folder_count: usize,
    filter_active: bool,
    rect: LayoutRectPx,
}

fn build_sheet_entry_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, ctx: EntryRowCtx<'_>) {
    let entry = ctx.entry;
    let scale = builder.scale();
    let is_rtl = builder.tree.is_rtl;
    let indent = round_u32(8.0 * scale).saturating_add((entry.nesting_depth.min(4) as u32).saturating_mul(round_u32(10.0 * scale)));
    let show_reorder = !ctx.filter_active && ctx.doc_folder_count > 1 && ctx.rect.width >= indent + 96;
    let btn_w = round_u32(15.0 * scale).clamp(14, 24);
    let reorder_w = if show_reorder { btn_w * 2 + 4 } else { 0 };
    let main_w = ctx.rect.width.saturating_sub(indent + reorder_w + 2);
    if main_w < 24 {
        return;
    }

    let main_local_x = crate::i18n::mirror_x_for_rtl(indent, main_w, ctx.rect.width, is_rtl);
    let row_rect = LayoutRectPx::new(ctx.rect.x + i32::try_from(main_local_x).unwrap_or(0), ctx.rect.y, main_w, ctx.rect.height);

    let eid = format!("nav.sheet.{}", entry.sheet_id.as_str());
    let selected = entry.is_active || builder.app.document().active_sheet_id.as_ref() == Some(&entry.sheet_id);
    let est = widget_state(builder, &eid, ControlEnablement::enabled());
    let raw_title =
        if entry.title.trim().is_empty() { nav_tr(&builder.tree.locale, "nav.untitled_sheet", "Untitled Sheet") } else { entry.title.as_str() };
    let mut ea =
        selectable_meta(WidgetRole::TreeItem, raw_title, FocusScope::Workspace, format!("nav.{:?}.{}", ctx.folder_kind, ctx.live_idx), selected);
    if entry.is_frozen {
        ea.value_text = Some(String::from(nav_tr(&builder.tree.locale, "nav.frozen_badge", "Frozen")));
    }

    let ewid = builder.push_node(NodeSpec {
        id: WidgetId::from(eid),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: row_rect,
        a11y: ea,
        state: est.clone(),
        action: WidgetAction::SwitchSheet { sheet_id: entry.sheet_id.clone(), section: entry.section },
    });

    let being_dragged = builder.app.nav.navigator_drag.active_sheet_id() == Some(entry.sheet_id.as_str());
    let bg = if selected {
        builder.palette.accent
    } else if est.hovered || being_dragged {
        builder.palette.header
    } else {
        builder.palette.panel
    };
    let fg = if selected { Rgba8::WHITE } else { builder.palette.text };
    let icon_col = if selected { Rgba8::WHITE } else { builder.palette.accent };
    builder.paint_fill(ZLayer::Workspace, Some(&ewid), row_rect, bg);

    if being_dragged {
        builder.paint_stroke(ZLayer::Workspace, Some(&ewid), row_rect, builder.palette.accent, 2);
    } else if builder.app.nav.navigator_drag.active_folder() == Some(ctx.folder_kind)
        && builder.app.nav.navigator_drag.hover_target_index == Some(ctx.live_idx)
    {
        builder.paint_line(ZLayer::Workspace, Some(&ewid), [row_rect.x, row_rect.y], [row_rect.right(), row_rect.y], builder.palette.accent, 2);
    }

    let mut lead = 3u32;
    if let Some(tag_col) = entry.color_tag {
        let bar_x = crate::i18n::mirror_x_for_rtl(lead, 4, row_rect.width, is_rtl);
        let bar_rect = LayoutRectPx::new(row_rect.x + i32::try_from(bar_x).unwrap_or(0), row_rect.y + 3, 4, row_rect.height.saturating_sub(6));
        builder.paint_fill(ZLayer::Workspace, Some(&ewid), bar_rect, tag_col);
        lead += 6;
    }

    if ctx.doc_folder_count > 1 {
        let grip_x = crate::i18n::mirror_x_for_rtl(lead, 7, row_rect.width, is_rtl);
        let grip_rect = LayoutRectPx::new(row_rect.x + i32::try_from(grip_x).unwrap_or(0), row_rect.y, 7, row_rect.height);
        paint_drag_grip(builder, ZLayer::Workspace, &ewid, grip_rect, if selected { Rgba8::WHITE } else { builder.palette.muted });
        lead += 9;
    }

    let icon_w = 14u32;
    let icon_x = crate::i18n::mirror_x_for_rtl(lead, icon_w, row_rect.width, is_rtl);
    let icon_rect = LayoutRectPx::new(row_rect.x + i32::try_from(icon_x).unwrap_or(0), row_rect.y, icon_w, row_rect.height);
    let table_kind = builder.app.document().data_sheet(&entry.sheet_id).map(|ds| ds.kind());
    paint_section_icon(builder, ZLayer::Workspace, &ewid, icon_rect, entry.section, table_kind, icon_col);
    lead += icon_w + 3;

    let badges_w = paint_entry_badges(builder, &ewid, entry, row_rect, selected);
    let text_w = row_rect.width.saturating_sub(lead + badges_w + 2);
    let text_local_x = crate::i18n::mirror_x_for_rtl(lead, text_w, row_rect.width, is_rtl);
    let text_rect = LayoutRectPx::new(row_rect.x + i32::try_from(text_local_x).unwrap_or(0), row_rect.y, text_w, row_rect.height);

    let size_px = (12.0 * scale).clamp(9.0, 22.0);
    let style = if selected || entry.is_bold { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
    paint_text_fitted(
        builder,
        &ewid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: text_rect,
            pad_x: 2,
            text: raw_title,
            style: &style,
            color: fg,
            scroll_container: Some(NAV_SCROLL_ID),
        },
    );

    builder.tree.navigator_entries.push(NavigatorEntryHitRect {
        sheet_id: entry.sheet_id.clone(),
        folder: ctx.folder_kind,
        index_in_folder: ctx.live_idx,
        rect: row_rect.to_workspace_rect(),
    });

    if show_reorder {
        let up_local_x = crate::i18n::mirror_x_for_rtl(indent + main_w + 2, btn_w, ctx.rect.width, is_rtl);
        let down_local_x = crate::i18n::mirror_x_for_rtl(indent + main_w + 2 + btn_w + 2, btn_w, ctx.rect.width, is_rtl);
        let up_rect =
            LayoutRectPx::new(ctx.rect.x + i32::try_from(up_local_x).unwrap_or(0), ctx.rect.y + 1, btn_w, ctx.rect.height.saturating_sub(2));
        let down_rect =
            LayoutRectPx::new(ctx.rect.x + i32::try_from(down_local_x).unwrap_or(0), ctx.rect.y + 1, btn_w, ctx.rect.height.saturating_sub(2));
        emit_reorder_button(builder, parent, &ctx, true, up_rect);
        emit_reorder_button(builder, parent, &ctx, false, down_rect);
    }
}

fn paint_entry_badges(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, entry: &NavigatorEntry, row_rect: LayoutRectPx, selected: bool) -> u32 {
    let is_rtl = builder.tree.is_rtl;
    let mut trail = 2u32;

    if entry.is_frozen {
        let w = 12u32;
        let lx = crate::i18n::mirror_x_for_rtl(row_rect.width.saturating_sub(trail + w), w, row_rect.width, is_rtl);
        let br = LayoutRectPx::new(row_rect.x + i32::try_from(lx).unwrap_or(0), row_rect.y + 3, w, row_rect.height.saturating_sub(6));
        let lock_col = if selected { Rgba8::WHITE } else { builder.palette.accent };
        paint_lock_badge(builder, ZLayer::Workspace, owner, br, lock_col);
        trail += w + 3;
    }

    if entry.is_dirty {
        let r = 3u32;
        let lx = crate::i18n::mirror_x_for_rtl(row_rect.width.saturating_sub(trail + r * 2), r * 2, row_rect.width, is_rtl);
        let cy = row_rect.center()[1];
        let cx = row_rect.x + i32::try_from(lx + r).unwrap_or(0);
        push_circle_op(builder, ZLayer::Workspace, owner, [cx, cy], r, Rgba8::rgb(220, 38, 38), None);
        trail += r * 2 + 3;
    }

    if entry.floating_note_count > 0 {
        let w = 10u32;
        let lx = crate::i18n::mirror_x_for_rtl(row_rect.width.saturating_sub(trail + w), w, row_rect.width, is_rtl);
        let nr = LayoutRectPx::new(row_rect.x + i32::try_from(lx).unwrap_or(0), row_rect.y + 4, w, row_rect.height.saturating_sub(8));
        builder.paint_fill(ZLayer::Workspace, Some(owner), nr, FloatingNote::DEFAULT_FILL);
        builder.paint_stroke(ZLayer::Workspace, Some(owner), nr, Rgba8::rgb(120, 95, 20), 1);
        trail += w + 3;
    }

    for link_col in entry.link_colors.iter().take(2) {
        let w = 4u32;
        let lx = crate::i18n::mirror_x_for_rtl(row_rect.width.saturating_sub(trail + w), w, row_rect.width, is_rtl);
        let pr = LayoutRectPx::new(row_rect.x + i32::try_from(lx).unwrap_or(0), row_rect.y + 4, w, row_rect.height.saturating_sub(8));
        let rgba = crate::workspace_nav::NavigatorBadgeColor::from_link_border(*link_col).rgba();
        builder.paint_fill(ZLayer::Workspace, Some(owner), pr, rgba);
        trail += w + 2;
    }

    trail
}

fn emit_reorder_button(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, ctx: &EntryRowCtx<'_>, move_up: bool, rect: LayoutRectPx) {
    let entry = ctx.entry;
    let can_move = if move_up { ctx.live_idx > 0 } else { ctx.live_idx + 1 < ctx.doc_folder_count };
    let en = if can_move {
        ControlEnablement::enabled()
    } else if move_up {
        ControlEnablement::disabled("Already first in folder")
    } else {
        ControlEnablement::disabled("Already last in folder")
    };
    let dir_tag = if move_up { "up" } else { "down" };
    let wid_str = format!("nav.reorder_{dir_tag}.{}", entry.sheet_id.as_str());
    let label = if move_up {
        nav_tr(&builder.tree.locale, "nav.move_up", "Move sheet up")
    } else {
        nav_tr(&builder.tree.locale, "nav.move_down", "Move sheet down")
    };
    let st = widget_state(builder, &wid_str, en);
    let delta = if move_up { -1 } else { 1 };
    let payload = format!("nav.reorder|{}|{}|{delta}", folder_slug(ctx.folder_kind), entry.sheet_id.as_str());
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(wid_str.clone()),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: rect,
        a11y: labeled_meta(WidgetRole::Button, format!("{label}: {}", entry.title), FocusScope::Workspace, wid_str),
        state: st.clone(),
        action: if st.enablement.enabled { WidgetAction::Custom { domain: String::from("nav_notes"), payload } } else { WidgetAction::NoOp },
    });
    let bg = if st.hovered && st.enablement.enabled { builder.palette.header } else { builder.palette.surface };
    let fg = if st.enablement.enabled { builder.palette.text } else { builder.palette.muted };
    builder.paint_fill(ZLayer::Workspace, Some(&wid), rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), rect, builder.palette.border, 1);
    paint_vertical_arrow(builder, ZLayer::Workspace, &wid, rect, move_up, fg);
}

fn build_inline_create_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, section: SheetSection, rect: LayoutRectPx) {
    let scale = builder.scale();
    let indent = round_u32(8.0 * scale);
    let btn_w = rect.width.saturating_sub(indent + 4);
    if btn_w < 30 {
        return;
    }
    let local_x = crate::i18n::mirror_x_for_rtl(indent, btn_w, rect.width, builder.tree.is_rtl);
    let btn_rect = LayoutRectPx::new(rect.x + i32::try_from(local_x).unwrap_or(0), rect.y + 1, btn_w, rect.height.saturating_sub(2));
    let doc = builder.app.document();
    let locale = builder.tree.locale.clone();

    let (slug, label, enablement) = match section {
        SheetSection::Data => ("data", nav_tr(&locale, "menu.insert.new_data_table", "New Data Table..."), ControlEnablement::enabled()),
        SheetSection::Info => {
            let en = if doc.info_sheets.is_empty() {
                ControlEnablement::disabled("Create a Data Table with paired Info first")
            } else {
                ControlEnablement::enabled()
            };
            ("info", nav_tr(&locale, "menu.insert.new_info_sheet", "New Info Sheet"), en)
        }
        SheetSection::Analyses => {
            let en = if doc.data_sheets.is_empty() {
                ControlEnablement::disabled("Create a Data Table first to run an analysis")
            } else {
                ControlEnablement::enabled()
            };
            ("results", nav_tr(&locale, "nav.new_analysis", "New Analysis..."), en)
        }
        SheetSection::Graphs => {
            let en = if doc.data_sheets.is_empty() {
                ControlEnablement::disabled("Create a Data Table first to create a graph")
            } else {
                ControlEnablement::enabled()
            };
            ("graphs", nav_tr(&locale, "menu.insert.new_graph", "New Graph"), en)
        }
        SheetSection::Layouts => ("layouts", nav_tr(&locale, "menu.insert.new_layout", "New Page Layout"), ControlEnablement::enabled()),
    };

    let wid_str = format!("nav.create.{slug}");
    let st = widget_state(builder, &wid_str, enablement);
    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(wid_str.clone()),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: btn_rect,
        a11y: labeled_meta(WidgetRole::Button, label, FocusScope::Workspace, wid_str),
        state: st.clone(),
        action: if st.enablement.enabled {
            WidgetAction::Custom { domain: String::from("nav_notes"), payload: format!("nav.create|{slug}") }
        } else {
            WidgetAction::NoOp
        },
    });

    let bg = if st.hovered && st.enablement.enabled { builder.palette.header } else { builder.palette.surface };
    let fg = if st.enablement.enabled { builder.palette.accent } else { builder.palette.muted };
    builder.paint_fill(ZLayer::Workspace, Some(&wid), btn_rect, bg);
    builder.paint_stroke(ZLayer::Workspace, Some(&wid), btn_rect, builder.palette.border, 1);

    let plus_w = 14u32;
    let plus_x = crate::i18n::mirror_x_for_rtl(4, plus_w, btn_rect.width, builder.tree.is_rtl);
    let plus_rect = LayoutRectPx::new(btn_rect.x + i32::try_from(plus_x).unwrap_or(0), btn_rect.y, plus_w, btn_rect.height);
    paint_plus_icon(builder, ZLayer::Workspace, &wid, plus_rect, fg);

    let text_w = btn_rect.width.saturating_sub(plus_w + 8);
    let text_x = crate::i18n::mirror_x_for_rtl(plus_w + 6, text_w, btn_rect.width, builder.tree.is_rtl);
    let text_rect = LayoutRectPx::new(btn_rect.x + i32::try_from(text_x).unwrap_or(0), btn_rect.y, text_w, btn_rect.height);
    let size_px = (11.5 * scale).clamp(9.0, 20.0);
    let style = UiTextStyle::ui_sans(size_px);
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: text_rect,
            pad_x: 2,
            text: label,
            style: &style,
            color: if st.enablement.enabled { builder.palette.text } else { builder.palette.muted },
            scroll_container: Some(NAV_SCROLL_ID),
        },
    );
}

/// Build the Family lineage strip inside `rect`.
pub fn build_family_pane(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    if rect.width < 24 || rect.height < 16 {
        return;
    }
    builder.paint_line(ZLayer::Workspace, None, [rect.x, rect.y], [rect.right(), rect.y], builder.palette.border, 1);
    let scale = builder.scale();
    let hdr_h = round_u32(22.0 * scale).clamp(18, 36).min(rect.height);
    let hdr = LayoutRectPx::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), hdr_h);
    let fam_title = nav_tr(&builder.tree.locale, "nav.family", "Family");
    let fam_label = if builder.app.family_open { format!("▾ {fam_title}") } else { format!("▸ {fam_title}") };

    emit_button(
        builder,
        ButtonWidgetSpec {
            id: "navigator.family_toggle",
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: hdr,
            label: &fam_label,
            keyboard_path: "Cmd+Shift+F",
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            primary: false,
            selected: builder.app.family_open,
            shortcut: Some("Cmd+Shift+F"),
            enablement: ControlEnablement::enabled(),
            action: WidgetAction::ToggleFamilyPane,
        },
    );

    if !builder.app.family_open || rect.height <= hdr_h + 18 {
        return;
    }

    let list_rect = LayoutRectPx::new(rect.x + 2, hdr.bottom() + 2, rect.width.saturating_sub(4), rect.height.saturating_sub(hdr_h + 5));
    if list_rect.width < 20 || list_rect.height < 16 {
        return;
    }

    let ordered = order_family_tree(builder.app.document(), &builder.app.nav.family_nodes);
    let row_h = round_u32(20.0 * scale).clamp(18, 36);
    let step_y = row_h + 2;
    let total_rows = ordered.len().max(1);
    let content_h = (total_rows as u32).saturating_mul(step_y) + 6;
    let scroll_off = builder.scroll_offset(FAMILY_SCROLL_ID);
    let scroll_meta = ScrollMeta::new(String::from(FAMILY_SCROLL_ID), list_rect, [list_rect.width, content_h], scroll_off, ScrollAxes::Vertical);

    let mut fam_a11y = labeled_meta(WidgetRole::TreeItem, fam_title, FocusScope::Workspace, "Cmd+Shift+F");
    fam_a11y.expanded = Some(true);
    let fam_wid = builder.push_node(NodeSpec {
        id: WidgetId::from("family.tree"),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: list_rect,
        a11y: fam_a11y,
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.attach_scroll(&fam_wid, scroll_meta.clone());
    builder.paint_stroke(ZLayer::Workspace, Some(&fam_wid), list_rect, builder.palette.border, 1);

    let sb_reserve = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) + 2 } else { 2 };
    let clip = list_rect.inset(2);
    let avail_w = clip.width.saturating_sub(sb_reserve).max(16);
    let local_x = if builder.tree.is_rtl { sb_reserve } else { 0 };
    builder.push_clip(clip);

    let y_scroll = round_i32(scroll_meta.scroll_offset_px[1]);
    let mut cy = clip.y + 2 - y_scroll;

    if ordered.is_empty() {
        let r = LayoutRectPx::new(clip.x + i32::try_from(local_x).unwrap_or(0), cy, avail_w, row_h);
        if is_row_visible(r, clip) {
            paint_muted_hint(builder, &fam_wid, r, nav_tr(&builder.tree.locale, "nav.family_empty", "No linked sheets"), Some(FAMILY_SCROLL_ID));
        }
    } else {
        for (idx, item) in ordered.iter().enumerate() {
            let r = LayoutRectPx::new(clip.x + i32::try_from(local_x).unwrap_or(0), cy, avail_w, row_h);
            if is_row_visible(r, clip) {
                build_family_row(builder, &fam_wid, idx, item, r);
            }
            cy += i32::try_from(step_y).unwrap_or(22);
        }
    }

    builder.pop_clip();
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(fam_wid));
}

#[derive(Debug, Clone)]
struct OrderedFamilyNode<'a> {
    node: &'a FamilyPaneNode,
    depth: usize,
}

fn order_family_tree<'a>(doc: &ProjectDocument, nodes: &'a [FamilyPaneNode]) -> Vec<OrderedFamilyNode<'a>> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let id_set: BTreeSet<&SheetId> = nodes.iter().map(|n| &n.sheet_id).collect();
    let mut by_id: BTreeMap<&SheetId, usize> = BTreeMap::new();
    for (idx, n) in nodes.iter().enumerate() {
        by_id.insert(&n.sheet_id, idx);
    }

    // Build parent -> children mapping inside the family subgraph.
    let mut parent_of: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];

    for (idx, node) in nodes.iter().enumerate() {
        let ups: Vec<usize> = doc
            .upstream_dependencies(&node.sheet_id)
            .into_iter()
            .filter(|u| id_set.contains(u))
            .filter_map(|u| by_id.get(&u).copied())
            .filter(|&u_idx| u_idx != idx)
            .collect();
        if let Some(best_parent) = pick_primary_family_parent(nodes, &ups) {
            parent_of[idx] = Some(best_parent);
            children_of[best_parent].push(idx);
        }
    }

    let mut roots: Vec<usize> = parent_of.iter().enumerate().filter_map(|(i, p)| p.is_none().then_some(i)).collect();
    roots.sort_by_key(|&i| family_sort_key(&nodes[i]));
    for kids in &mut children_of {
        kids.sort_by_key(|&i| family_sort_key(&nodes[i]));
    }

    let mut out = Vec::with_capacity(nodes.len());
    let mut visited = vec![false; nodes.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &root in roots.iter().rev() {
        stack.push((root, 0));
    }
    while let Some((idx, depth)) = stack.pop() {
        if visited[idx] {
            continue;
        }
        visited[idx] = true;
        out.push(OrderedFamilyNode { node: &nodes[idx], depth });
        for &child in children_of[idx].iter().rev() {
            if !visited[child] {
                stack.push((child, depth + 1));
            }
        }
    }

    // Fallback for any cycle-disconnected items.
    for (idx, node) in nodes.iter().enumerate() {
        if !visited[idx] {
            out.push(OrderedFamilyNode { node, depth: 0 });
        }
    }
    out
}

fn pick_primary_family_parent(nodes: &[FamilyPaneNode], candidates: &[usize]) -> Option<usize> {
    candidates.iter().copied().max_by_key(|&idx| {
        let n = &nodes[idx];
        let rel_score = match n.relation {
            FamilyRelation::Upstream => 0usize,
            FamilyRelation::Anchor => 1,
            FamilyRelation::Downstream => 2,
        };
        (section_order_rank(n.section), rel_score, std::cmp::Reverse(idx))
    })
}

fn family_sort_key(node: &FamilyPaneNode) -> (u8, usize, &str) {
    let rel = match node.relation {
        FamilyRelation::Upstream => 0u8,
        FamilyRelation::Anchor => 1,
        FamilyRelation::Downstream => 2,
    };
    (section_order_rank(node.section), if rel == 0 { usize::MAX - node.hop_distance } else { node.hop_distance }, node.title.as_str())
}

const fn section_order_rank(section: SheetSection) -> u8 {
    match section {
        SheetSection::Info => 0,
        SheetSection::Data => 1,
        SheetSection::Analyses => 2,
        SheetSection::Graphs => 3,
        SheetSection::Layouts => 4,
    }
}

fn build_family_row(builder: &mut LayoutBuilder<'_>, parent: &WidgetId, idx: usize, item: &OrderedFamilyNode<'_>, rect: LayoutRectPx) {
    let node = item.node;
    let scale = builder.scale();
    let is_rtl = builder.tree.is_rtl;
    let indent_step = round_u32(11.0 * scale);
    let indent = 4 + (item.depth.min(5) as u32).saturating_mul(indent_step);
    let row_w = rect.width.saturating_sub(indent).max(24);
    let row_local_x = crate::i18n::mirror_x_for_rtl(indent, row_w, rect.width, is_rtl);
    let row_rect = LayoutRectPx::new(rect.x + i32::try_from(row_local_x).unwrap_or(0), rect.y, row_w, rect.height);

    let title = if node.title.trim().is_empty() { nav_tr(&builder.tree.locale, "nav.untitled_sheet", "Untitled Sheet") } else { node.title.as_str() };
    let active = node.is_active || builder.app.document().active_sheet_id.as_ref() == Some(&node.sheet_id);
    let id_str = format!("family.item.{idx}");
    let st = widget_state(builder, &id_str, ControlEnablement::enabled());
    let mut a11y = selectable_meta(WidgetRole::TreeItem, title, FocusScope::Workspace, format!("family.{idx}"), active);
    a11y.value_text = Some(format!("depth:{}", item.depth));

    let wid = builder.push_node(NodeSpec {
        id: WidgetId::from(id_str),
        parent: Some(parent.clone()),
        z_layer: ZLayer::Workspace,
        rect_px: row_rect,
        a11y,
        state: st.clone(),
        action: WidgetAction::Custom { domain: String::from("nav_notes"), payload: format!("family.select|{}", node.sheet_id.as_str()) },
    });

    let bg = if active {
        builder.palette.accent
    } else if st.hovered || node.is_anchor {
        builder.palette.header
    } else {
        builder.palette.panel
    };
    let fg = if active { Rgba8::WHITE } else { builder.palette.text };
    let icon_col = if active { Rgba8::WHITE } else { builder.palette.accent };
    builder.paint_fill(ZLayer::Workspace, Some(&wid), row_rect, bg);

    if node.is_anchor && !active {
        let bar_x = crate::i18n::mirror_x_for_rtl(0, 3, row_rect.width, is_rtl);
        builder.paint_fill(
            ZLayer::Workspace,
            Some(&wid),
            LayoutRectPx::new(row_rect.x + i32::try_from(bar_x).unwrap_or(0), row_rect.y + 2, 3, row_rect.height.saturating_sub(4)),
            builder.palette.accent,
        );
    }

    if item.depth > 0 {
        let branch_x = if is_rtl { row_rect.right() + 4 } else { row_rect.x - 6 };
        let cy = row_rect.center()[1];
        builder.paint_line(ZLayer::Workspace, Some(&wid), [branch_x, row_rect.y], [branch_x, cy], builder.palette.border, 1);
        builder.paint_line(
            ZLayer::Workspace,
            Some(&wid),
            [branch_x, cy],
            [if is_rtl { row_rect.right() } else { row_rect.x }, cy],
            builder.palette.border,
            1,
        );
    }

    let icon_w = 13u32;
    let lead = if node.is_anchor && !active { 5 } else { 3 };
    let icon_local_x = crate::i18n::mirror_x_for_rtl(lead, icon_w, row_rect.width, is_rtl);
    let icon_rect = LayoutRectPx::new(row_rect.x + i32::try_from(icon_local_x).unwrap_or(0), row_rect.y, icon_w, row_rect.height);
    let table_kind = builder.app.document().data_sheet(&node.sheet_id).map(|ds| ds.kind());
    paint_section_icon(builder, ZLayer::Workspace, &wid, icon_rect, node.section, table_kind, icon_col);

    let text_lead = lead + icon_w + 3;
    let text_w = row_rect.width.saturating_sub(text_lead + 2);
    let text_local_x = crate::i18n::mirror_x_for_rtl(text_lead, text_w, row_rect.width, is_rtl);
    let text_rect = LayoutRectPx::new(row_rect.x + i32::try_from(text_local_x).unwrap_or(0), row_rect.y, text_w, row_rect.height);
    let size_px = (11.5 * scale).clamp(9.0, 20.0);
    let style = if active || node.is_anchor { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
    paint_text_fitted(
        builder,
        &wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: text_rect,
            pad_x: 2,
            text: title,
            style: &style,
            color: fg,
            scroll_container: Some(FAMILY_SCROLL_ID),
        },
    );
}

/// Build the Info Sheet constants + notes editor view inside `rect`, preserving `R2-S10-09`
/// primary-pane geometry guards and overlaying interactive `FloatingNote` callout cards.
pub fn build_info_sheet_view(builder: &mut LayoutBuilder<'_>, info: &InfoSheet, rect: LayoutRectPx, slot_tag: &str) {
    let box_rect = rect.inset(8);
    if box_rect.width < 60 || box_rect.height < 60 {
        return;
    }
    builder.paint_fill(ZLayer::Workspace, None, box_rect, builder.palette.surface);
    builder.paint_stroke(ZLayer::Workspace, None, box_rect, builder.palette.border, 1);

    let scale = builder.scale();
    let half_w = box_rect.width / 2;
    let const_x = box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(0, half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let notes_x =
        box_rect.x + i32::try_from(crate::i18n::mirror_x_for_rtl(half_w, box_rect.width - half_w, box_rect.width, builder.tree.is_rtl)).unwrap_or(0);
    let hdr_y = box_rect.y + 4;
    let body_top = box_rect.y + 26;
    let notes_rect = LayoutRectPx::new(notes_x + 6, body_top, box_rect.width.saturating_sub(half_w + 12), box_rect.height.saturating_sub(32));

    if slot_tag != "secondary" || builder.tree.info_notes_rect.is_none() {
        builder.tree.info_notes_rect = Some(notes_rect.to_workspace_rect());
        builder.tree.info_sheet_id = Some(info.id.clone());
    }

    let size_px = (12.0 * scale).clamp(9.0, 22.0);
    let bold = UiTextStyle::ui_bold(size_px);
    let st = UiTextStyle::ui_sans(size_px);
    let const_hdr = LayoutRectPx::new(const_x + 6, hdr_y, half_w.saturating_sub(12), 18);
    let notes_hdr = LayoutRectPx::new(notes_x + 6, hdr_y, box_rect.width.saturating_sub(half_w + 12), 18);
    let const_heading_txt = nav_tr(&builder.tree.locale, "info.constants_heading", "Constants");
    let notes_heading_txt = nav_tr(&builder.tree.locale, "info.notes_heading", "Experiment Notes");

    let ch_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("info.{slot_tag}.const_hdr")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: const_hdr,
        a11y: AccessibilityMeta::new(WidgetRole::Label, const_heading_txt, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &ch_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: const_hdr,
            pad_x: 2,
            text: const_heading_txt,
            style: &bold,
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let nh_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(format!("info.{slot_tag}.notes_hdr")),
        parent: None,
        z_layer: ZLayer::Workspace,
        rect_px: notes_hdr,
        a11y: AccessibilityMeta::new(WidgetRole::Label, notes_heading_txt, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    paint_text_fitted(
        builder,
        &nh_wid,
        FittedTextSpec {
            z_layer: ZLayer::Workspace,
            rect: notes_hdr,
            pad_x: 2,
            text: notes_heading_txt,
            style: &bold,
            color: builder.palette.text,
            scroll_container: None,
        },
    );

    let const_list_rect = LayoutRectPx::new(const_x + 6, body_top, half_w.saturating_sub(12), box_rect.height.saturating_sub(32));
    if const_list_rect.width >= 24 && const_list_rect.height >= 20 {
        let row_h = round_u32(20.0 * scale).clamp(18, 34);
        let step = row_h + 2;
        let content_h = (info.constants.len() as u32).saturating_mul(step) + 4;
        let scroll_id = format!("{INFO_CONST_SCROLL_ID}.{slot_tag}");
        let scroll_off = builder.scroll_offset(&scroll_id);
        let scroll_meta = ScrollMeta::new(scroll_id.clone(), const_list_rect, [const_list_rect.width, content_h], scroll_off, ScrollAxes::Vertical);
        let sb_reserve = if scroll_meta.has_vertical_overflow() { scrollbar_thickness_px(scale) + 2 } else { 0 };
        let row_w = const_list_rect.width.saturating_sub(sb_reserve).max(16);
        let row_lx = if builder.tree.is_rtl { i32::try_from(sb_reserve).unwrap_or(0) } else { 0 };

        let cont_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(format!("info.{slot_tag}.constants_list")),
            parent: None,
            z_layer: ZLayer::Workspace,
            rect_px: const_list_rect,
            a11y: AccessibilityMeta::new(WidgetRole::Label, const_heading_txt, FocusScope::Workspace),
            state: WidgetVisualState::default(),
            action: WidgetAction::NoOp,
        });
        builder.attach_scroll(&cont_wid, scroll_meta.clone());
        builder.push_clip(const_list_rect);

        let y_scroll = round_i32(scroll_meta.scroll_offset_px[1]);
        let mut y = const_list_rect.y - y_scroll;
        for (idx, c) in info.constants.iter().enumerate() {
            let r = LayoutRectPx::new(const_list_rect.x + row_lx, y, row_w, row_h);
            if is_row_visible(r, const_list_rect) {
                let cid = if slot_tag == "secondary" { format!("info.secondary.const.{idx}") } else { format!("info.const.{idx}") };
                let wid = builder.push_node(NodeSpec {
                    id: WidgetId::from(cid.clone()),
                    parent: Some(cont_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    rect_px: r,
                    a11y: labeled_meta(WidgetRole::TextBox, c.name.clone(), FocusScope::Workspace, cid),
                    state: WidgetVisualState::default(),
                    action: WidgetAction::FocusWidget(format!("info.const.{idx}")),
                });
                builder.paint_fill(ZLayer::Workspace, Some(&wid), r, builder.palette.panel);
                let unit_suffix = c.units.as_deref().filter(|u| !u.is_empty()).map_or(String::new(), |u| format!(" {u}"));
                let txt = format!("{}: {}{unit_suffix}", c.name, c.value.display_text());
                paint_text_fitted(
                    builder,
                    &wid,
                    FittedTextSpec {
                        z_layer: ZLayer::Workspace,
                        rect: r,
                        pad_x: 4,
                        text: &txt,
                        style: &st,
                        color: builder.palette.text,
                        scroll_container: Some(&scroll_id),
                    },
                );
                if slot_tag != "secondary" || builder.tree.info_constant_rows.is_empty() {
                    builder.tree.info_constant_rows.push(InfoRowHitRect {
                        sheet_id: info.id.clone(),
                        name: c.name.clone(),
                        value_text: c.value.display_text(),
                        rect: r.to_workspace_rect(),
                    });
                }
            }
            y += i32::try_from(step).unwrap_or(22);
        }
        builder.pop_clip();
        emit_scrollbars_for_meta(builder, ZLayer::Workspace, &scroll_meta, Some(cont_wid));
    }

    let notes_id = if slot_tag == "secondary" { "info.secondary.notes" } else { "info.notes" };
    let active_notes_text = match &builder.app.inline_editor.target {
        Some(InlineEditorTarget::InfoNotes { sheet_id }) if builder.app.inline_editor.active && sheet_id == &info.id => {
            builder.app.inline_editor.buffer.as_str()
        }
        _ => info.notes_markdown.as_str(),
    };
    let _ = emit_text_field(
        builder,
        TextFieldSpec {
            id: notes_id,
            parent: None,
            z_layer: ZLayer::Workspace,
            rect: notes_rect,
            label: notes_heading_txt,
            fallback_text: active_notes_text,
            keyboard_path: notes_id,
            focus_scope: FocusScope::Workspace,
            tab_order: None,
            multiline: true,
            enablement: ControlEnablement::enabled(),
        },
    );

    if !info.floating_notes.is_empty() {
        build_floating_notes_overlay(builder, &info.id, &info.floating_notes, box_rect);
    }
}

/// Render interactive, collapsible, draggable `FloatingNote` callout cards overlaid within `bounds_rect`.
pub fn build_floating_notes_overlay(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, notes: &[FloatingNote], bounds_rect: LayoutRectPx) {
    if notes.is_empty() || bounds_rect.width < 120 || bounds_rect.height < 48 {
        return;
    }
    builder.push_clip(bounds_rect);
    for (idx, note) in notes.iter().enumerate() {
        let card_id = format!("floating_note.{}.{}.card", sheet_id.as_str(), note.id.as_str());
        if builder.tree.nodes.iter().any(|n| n.id.as_str() == card_id) {
            continue;
        }
        build_floating_note_card(builder, sheet_id, note, idx, bounds_rect);
    }
    builder.pop_clip();
}

fn build_floating_note_card(builder: &mut LayoutBuilder<'_>, sheet_id: &SheetId, note: &FloatingNote, idx: usize, bounds_rect: LayoutRectPx) {
    let scale = builder.scale();
    let pt_to_px = (96.0 / 72.0) * scale;
    let note_id = note.id.as_str();
    let key = note_state_key(sheet_id, note_id);
    let shard = &builder.app.view_shards.nav_notes;

    let collapsed = builder.app.nav.floating_notes_collapsed.get(note_id).copied().unwrap_or(note.minimized);
    let [x_pt, y_pt] = shard.note_positions_pt.get(&key).copied().unwrap_or([note.bounds_pt.x, note.bounds_pt.y]);

    let hdr_h = round_u32(22.0 * scale).clamp(18, 38);
    let min_w = round_u32(196.0 * scale);
    let max_w = bounds_rect.width.saturating_sub(12).max(110);
    let card_w = round_u32(note.bounds_pt.width * pt_to_px).clamp(min_w.min(max_w), max_w);
    let min_body_h = round_u32(54.0 * scale);
    let max_h = bounds_rect.height.saturating_sub(12).max(hdr_h + 4);
    let expanded_h = (hdr_h + round_u32(note.bounds_pt.height * pt_to_px).max(min_body_h)).clamp(hdr_h + 24, max_h);
    let card_h = if collapsed { hdr_h + 4 } else { expanded_h };

    let stagger = if shard.note_positions_pt.contains_key(&key) { 0 } else { i32::try_from(idx * 18).unwrap_or(0) };
    let raw_x = bounds_rect.x + 6 + round_i32(x_pt * pt_to_px) + stagger;
    let raw_y = bounds_rect.y + 6 + round_i32(y_pt * pt_to_px) + stagger;
    let max_x = (bounds_rect.right() - 6 - i32::try_from(card_w).unwrap_or(180)).max(bounds_rect.x + 6);
    let max_y = (bounds_rect.bottom() - 6 - i32::try_from(card_h).unwrap_or(24)).max(bounds_rect.y + 6);
    let card_rect = LayoutRectPx::new(raw_x.clamp(bounds_rect.x + 6, max_x), raw_y.clamp(bounds_rect.y + 6, max_y), card_w, card_h);

    let fill = if note.fill_color.a == 0 { FloatingNote::DEFAULT_FILL } else { note.fill_color };
    let is_light = fill.relative_luminance() >= 0.42;
    let ink = if is_light { Rgba8::rgb(24, 28, 36) } else { Rgba8::WHITE };
    let hdr_fill = shift_color(fill, if is_light { -18 } else { 18 });
    let is_active = shard.active_note_id.as_deref() == Some(note_id) || shard.grabbed_note_id.as_deref() == Some(key.as_str());
    let border_col = if is_active { builder.palette.accent } else { shift_color(fill, if is_light { -72 } else { 64 }) };

    let shadow_rect = LayoutRectPx::new(card_rect.x + 2, card_rect.y + 2, card_rect.width, card_rect.height);
    builder.paint_fill(ZLayer::FloatingOverlay, None, shadow_rect, Rgba8::rgba(15, 23, 42, 40));

    let card_id = format!("floating_note.{}.{}.card", sheet_id.as_str(), note_id);
    let card_title = nav_tr(&builder.tree.locale, "menu.insert.floating_note", "Floating Note");
    let card_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(card_id),
        parent: None,
        z_layer: ZLayer::FloatingOverlay,
        rect_px: card_rect,
        a11y: AccessibilityMeta::new(WidgetRole::Label, card_title, FocusScope::Workspace),
        state: WidgetVisualState::default(),
        action: WidgetAction::NoOp,
    });
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&card_wid), card_rect, fill);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&card_wid), card_rect, border_col, if is_active { 2 } else { 1 });

    let hdr_rect = LayoutRectPx::new(card_rect.x + 2, card_rect.y + 2, card_rect.width.saturating_sub(4), hdr_h);
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&card_wid), hdr_rect, hdr_fill);
    builder.paint_line(
        ZLayer::FloatingOverlay,
        Some(&card_wid),
        [hdr_rect.x, hdr_rect.bottom()],
        [hdr_rect.right(), hdr_rect.bottom()],
        border_col,
        1,
    );

    build_note_card_header(
        builder,
        &card_wid,
        NoteHeaderCtx {
            sheet_id,
            note_id,
            collapsed,
            is_grabbed: shard.grabbed_note_id.as_deref() == Some(key.as_str()),
            hdr_rect,
            hdr_fill,
            ink,
            border_col,
        },
    );

    if !collapsed && card_rect.bottom() > hdr_rect.bottom() + 14 {
        let body_rect = LayoutRectPx::new(
            card_rect.x + 6,
            hdr_rect.bottom() + 4,
            card_rect.width.saturating_sub(12),
            u32::try_from((card_rect.bottom() - hdr_rect.bottom() - 8).max(0)).unwrap_or(0),
        );
        let body_text = if note.body_markdown.trim().is_empty() {
            nav_tr(&builder.tree.locale, "nav.note_empty", "(Empty note)")
        } else {
            note.body_markdown.as_str()
        };
        paint_wrapped_note_body(builder, &card_wid, body_rect, body_text, ink);
    }
}

struct NoteHeaderCtx<'a> {
    sheet_id: &'a SheetId,
    note_id: &'a str,
    collapsed: bool,
    is_grabbed: bool,
    hdr_rect: LayoutRectPx,
    hdr_fill: Rgba8,
    ink: Rgba8,
    border_col: Rgba8,
}

fn build_note_card_header(builder: &mut LayoutBuilder<'_>, card_wid: &WidgetId, ctx: NoteHeaderCtx<'_>) {
    let scale = builder.scale();
    let is_rtl = builder.tree.is_rtl;
    let locale = builder.tree.locale.clone();
    let btn_h = ctx.hdr_rect.height.saturating_sub(4).max(14);
    let btn_y = ctx.hdr_rect.y + i32::try_from((ctx.hdr_rect.height.saturating_sub(btn_h)) / 2).unwrap_or(0);
    let collapse_w = round_u32(18.0 * scale).clamp(16, 26);
    let move_w = round_u32(15.0 * scale).clamp(14, 22);
    let gap = 2u32;

    let show_four_dirs = ctx.hdr_rect.width >= 180;
    let move_specs: &[(&str, f64, f64, [i32; 2])] = if show_four_dirs {
        &[("move_left", -24.0, 0.0, [-1, 0]), ("move_up", 0.0, -18.0, [0, -1]), ("move_down", 0.0, 18.0, [0, 1]), ("move_right", 24.0, 0.0, [1, 0])]
    } else {
        &[("move_right", 24.0, 18.0, [1, 0])]
    };

    let controls_w = collapse_w + (move_specs.len() as u32) * (move_w + gap) + 2;
    let drag_w = ctx.hdr_rect.width.saturating_sub(controls_w + 2);
    if drag_w < 36 {
        return;
    }

    let drag_local_x = crate::i18n::mirror_x_for_rtl(0, drag_w, ctx.hdr_rect.width, is_rtl);
    let drag_rect = LayoutRectPx::new(
        ctx.hdr_rect.x + i32::try_from(drag_local_x).unwrap_or(0),
        ctx.hdr_rect.y + 1,
        drag_w,
        ctx.hdr_rect.height.saturating_sub(2),
    );
    let drag_id = format!("floating_note.{}.{}.drag_handle", ctx.sheet_id.as_str(), ctx.note_id);
    let drag_st = widget_state(builder, &drag_id, ControlEnablement::enabled());
    let mut drag_a11y = labeled_meta(WidgetRole::Button, nav_tr(&locale, "nav.note_drag", "Drag note"), FocusScope::Workspace, drag_id.clone());
    drag_a11y.selected = Some(ctx.is_grabbed);
    let drag_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(drag_id),
        parent: Some(card_wid.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: drag_rect,
        a11y: drag_a11y,
        state: drag_st.clone(),
        action: WidgetAction::Custom { domain: String::from("nav_notes"), payload: format!("note.drag|{}|{}", ctx.sheet_id.as_str(), ctx.note_id) },
    });
    let drag_bg = if drag_st.hovered || ctx.is_grabbed { shift_color(ctx.hdr_fill, -14) } else { ctx.hdr_fill };
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&drag_wid), drag_rect, drag_bg);
    if ctx.is_grabbed {
        builder.paint_stroke(ZLayer::FloatingOverlay, Some(&drag_wid), drag_rect, builder.palette.accent, 1);
    }

    let grip_x = crate::i18n::mirror_x_for_rtl(3, 7, drag_rect.width, is_rtl);
    let grip_rect = LayoutRectPx::new(drag_rect.x + i32::try_from(grip_x).unwrap_or(0), drag_rect.y, 7, drag_rect.height);
    paint_drag_grip(builder, ZLayer::FloatingOverlay, &drag_wid, grip_rect, ctx.ink);

    let title_w = drag_rect.width.saturating_sub(14);
    let title_x = crate::i18n::mirror_x_for_rtl(12, title_w, drag_rect.width, is_rtl);
    let title_rect = LayoutRectPx::new(drag_rect.x + i32::try_from(title_x).unwrap_or(0), drag_rect.y, title_w, drag_rect.height);
    let note_hdr_lbl = nav_tr(&locale, "menu.insert.floating_note", "Floating Note");
    let size_px = (11.0 * scale).clamp(9.0, 19.0);
    paint_text_fitted(
        builder,
        &drag_wid,
        FittedTextSpec {
            z_layer: ZLayer::FloatingOverlay,
            rect: title_rect,
            pad_x: 2,
            text: note_hdr_lbl,
            style: &UiTextStyle::ui_bold(size_px),
            color: ctx.ink,
            scroll_container: None,
        },
    );

    let mut trail = collapse_w;
    let col_local_x = crate::i18n::mirror_x_for_rtl(ctx.hdr_rect.width.saturating_sub(trail), collapse_w, ctx.hdr_rect.width, is_rtl);
    let col_rect = LayoutRectPx::new(ctx.hdr_rect.x + i32::try_from(col_local_x).unwrap_or(0), btn_y, collapse_w, btn_h);
    let col_id = format!("floating_note.{}.{}.collapse", ctx.sheet_id.as_str(), ctx.note_id);
    let col_lbl =
        if ctx.collapsed { nav_tr(&locale, "nav.note_expand", "Expand note") } else { nav_tr(&locale, "nav.note_collapse", "Collapse note") };
    let mut col_a11y = labeled_meta(WidgetRole::Button, col_lbl, FocusScope::Workspace, col_id.clone());
    col_a11y.expanded = Some(!ctx.collapsed);
    let col_st = widget_state(builder, &col_id, ControlEnablement::enabled());
    let col_wid = builder.push_node(NodeSpec {
        id: WidgetId::from(col_id),
        parent: Some(card_wid.clone()),
        z_layer: ZLayer::FloatingOverlay,
        rect_px: col_rect,
        a11y: col_a11y,
        state: col_st.clone(),
        action: WidgetAction::Custom { domain: String::from("nav_notes"), payload: format!("note.toggle|{}|{}", ctx.sheet_id.as_str(), ctx.note_id) },
    });
    let col_bg = if col_st.hovered { shift_color(ctx.hdr_fill, -18) } else { ctx.hdr_fill };
    builder.paint_fill(ZLayer::FloatingOverlay, Some(&col_wid), col_rect, col_bg);
    builder.paint_stroke(ZLayer::FloatingOverlay, Some(&col_wid), col_rect, ctx.border_col, 1);
    paint_disclosure_chevron(builder, ZLayer::FloatingOverlay, &col_wid, col_rect, !ctx.collapsed, ctx.ink);

    for &(suffix, dx, dy, dir) in move_specs.iter().rev() {
        trail += gap + move_w;
        let mx = crate::i18n::mirror_x_for_rtl(ctx.hdr_rect.width.saturating_sub(trail), move_w, ctx.hdr_rect.width, is_rtl);
        let m_rect = LayoutRectPx::new(ctx.hdr_rect.x + i32::try_from(mx).unwrap_or(0), btn_y, move_w, btn_h);
        let m_id = format!("floating_note.{}.{}.{suffix}", ctx.sheet_id.as_str(), ctx.note_id);
        let m_st = widget_state(builder, &m_id, ControlEnablement::enabled());
        let m_wid = builder.push_node(NodeSpec {
            id: WidgetId::from(m_id.clone()),
            parent: Some(card_wid.clone()),
            z_layer: ZLayer::FloatingOverlay,
            rect_px: m_rect,
            a11y: labeled_meta(
                WidgetRole::Button,
                format!("{} ({suffix})", nav_tr(&locale, "nav.note_move_step", "Move note")),
                FocusScope::Workspace,
                m_id,
            ),
            state: m_st.clone(),
            action: WidgetAction::Custom {
                domain: String::from("nav_notes"),
                payload: format!("note.move|{}|{}|{dx}|{dy}", ctx.sheet_id.as_str(), ctx.note_id),
            },
        });
        let m_bg = if m_st.hovered { shift_color(ctx.hdr_fill, -18) } else { ctx.hdr_fill };
        builder.paint_fill(ZLayer::FloatingOverlay, Some(&m_wid), m_rect, m_bg);
        builder.paint_stroke(ZLayer::FloatingOverlay, Some(&m_wid), m_rect, ctx.border_col, 1);
        paint_cardinal_arrow(builder, ZLayer::FloatingOverlay, &m_wid, m_rect, dir, ctx.ink);
    }
}

fn paint_wrapped_note_body(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, text: &str, color: Rgba8) {
    if rect.width < 20 || rect.height < 12 {
        return;
    }
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let style = UiTextStyle::ui_sans(size_px);
    let max_w = f64::from(rect.width);
    let line_h = builder.measurer.measure("Ag", &style, &builder.tree.locale).line_height_px.ceil() as i32;
    let step_y = (line_h + 2).max(14);
    let mut lines = wrap_note_lines(builder, text, &style, max_w);
    let max_lines = usize::try_from(rect.height as i32 / step_y).unwrap_or(1).max(1);
    if lines.len() > max_lines {
        lines.truncate(max_lines);
    }
    for (idx, line) in lines.into_iter().enumerate() {
        let y = rect.y + i32::try_from(idx).unwrap_or(0) * step_y;
        if y + step_y - 2 > rect.bottom() {
            break;
        }
        let (display, ellipsized) = ellipsize_text(builder.measurer, &line, &style, &builder.tree.locale, max_w);
        builder.paint_text(TextRunSpec {
            z_layer: ZLayer::FloatingOverlay,
            owner,
            scroll_container_id: None,
            pos_px: [f64::from(rect.x), f64::from(y)],
            text: &display,
            style: &style,
            color,
            ellipsized,
        });
    }
}

fn wrap_note_lines(builder: &LayoutBuilder<'_>, text: &str, style: &UiTextStyle, max_w: f64) -> Vec<String> {
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut current = String::new();
        for word in trimmed.split_whitespace() {
            let candidate = if current.is_empty() { word.to_owned() } else { format!("{current} {word}") };
            let w = builder.measurer.measure(&candidate, style, &builder.tree.locale).width_px;
            if w <= max_w || current.is_empty() {
                current = candidate;
            } else {
                out.push(current);
                current = word.to_owned();
            }
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

fn sheet_floating_notes<'a>(doc: &'a ProjectDocument, id: &SheetId) -> &'a [FloatingNote] {
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

fn paint_muted_hint(builder: &mut LayoutBuilder<'_>, owner: &WidgetId, rect: LayoutRectPx, text: &str, scroll_id: Option<&str>) {
    let size_px = (11.5 * builder.scale()).clamp(9.0, 20.0);
    let style = UiTextStyle::ui_sans(size_px).italic();
    paint_text_fitted(
        builder,
        owner,
        FittedTextSpec { z_layer: ZLayer::Workspace, rect, pad_x: 6, text, style: &style, color: builder.palette.muted, scroll_container: scroll_id },
    );
}

fn paint_disclosure_chevron(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, expanded: bool, color: Rgba8) {
    let [cx, cy] = rect.center();
    if expanded {
        builder.paint_line(z, Some(owner), [cx - 3, cy - 2], [cx, cy + 2], color, 2);
        builder.paint_line(z, Some(owner), [cx, cy + 2], [cx + 3, cy - 2], color, 2);
    } else {
        builder.paint_line(z, Some(owner), [cx - 2, cy - 3], [cx + 2, cy], color, 2);
        builder.paint_line(z, Some(owner), [cx + 2, cy], [cx - 2, cy + 3], color, 2);
    }
}

fn paint_section_icon(
    builder: &mut LayoutBuilder<'_>,
    z: ZLayer,
    owner: &WidgetId,
    rect: LayoutRectPx,
    section: SheetSection,
    table_kind: Option<TableKind>,
    color: Rgba8,
) {
    let [cx, cy] = rect.center();
    let box_rect = LayoutRectPx::new(cx - 5, cy - 5, 10, 10);
    match section {
        SheetSection::Data => {
            builder.paint_stroke(z, Some(owner), box_rect, color, 1);
            builder.paint_line(z, Some(owner), [box_rect.x, cy], [box_rect.right(), cy], color, 1);
            builder.paint_line(z, Some(owner), [cx, box_rect.y], [cx, box_rect.bottom()], color, 1);
            if matches!(table_kind, Some(TableKind::Xy | TableKind::Survival)) {
                push_circle_op(builder, z, owner, [cx - 2, cy + 2], 1, color, None);
            }
        }
        SheetSection::Info => {
            builder.paint_stroke(z, Some(owner), LayoutRectPx::new(cx - 4, cy - 5, 8, 10), color, 1);
            push_circle_op(builder, z, owner, [cx, cy - 2], 1, color, None);
            builder.paint_line(z, Some(owner), [cx, cy], [cx, cy + 3], color, 1);
        }
        SheetSection::Analyses => {
            builder.paint_stroke(z, Some(owner), box_rect, color, 1);
            builder.paint_line(z, Some(owner), [cx - 3, cy + 1], [cx - 1, cy + 3], color, 1);
            builder.paint_line(z, Some(owner), [cx - 1, cy + 3], [cx + 3, cy - 2], color, 1);
        }
        SheetSection::Graphs => {
            builder.paint_line(z, Some(owner), [cx - 4, cy - 5], [cx - 4, cy + 4], color, 1);
            builder.paint_line(z, Some(owner), [cx - 4, cy + 4], [cx + 5, cy + 4], color, 1);
            builder.paint_line(z, Some(owner), [cx - 2, cy + 2], [cx + 1, cy - 1], color, 1);
            builder.paint_line(z, Some(owner), [cx + 1, cy - 1], [cx + 4, cy - 3], color, 1);
        }
        SheetSection::Layouts => {
            builder.paint_stroke(z, Some(owner), box_rect, color, 1);
            builder.paint_fill(z, Some(owner), LayoutRectPx::new(cx - 3, cy - 3, 3, 3), color);
            builder.paint_fill(z, Some(owner), LayoutRectPx::new(cx + 1, cy - 3, 3, 6), color);
        }
    }
}

fn paint_lock_badge(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, color: Rgba8) {
    let [cx, cy] = rect.center();
    builder.paint_stroke(z, Some(owner), LayoutRectPx::new(cx - 2, cy - 4, 5, 4), color, 1);
    builder.paint_fill(z, Some(owner), LayoutRectPx::new(cx - 3, cy - 1, 7, 5), color);
}

fn paint_drag_grip(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, color: Rgba8) {
    let [cx, cy] = rect.center();
    for dy in [-3, 0, 3] {
        for dx in [-2, 1] {
            builder.paint_fill(z, Some(owner), LayoutRectPx::new(cx + dx, cy + dy, 2, 2), color);
        }
    }
}

fn paint_vertical_arrow(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, up: bool, color: Rgba8) {
    let [cx, cy] = rect.center();
    if up {
        builder.paint_line(z, Some(owner), [cx - 3, cy + 1], [cx, cy - 2], color, 1);
        builder.paint_line(z, Some(owner), [cx, cy - 2], [cx + 3, cy + 1], color, 1);
    } else {
        builder.paint_line(z, Some(owner), [cx - 3, cy - 1], [cx, cy + 2], color, 1);
        builder.paint_line(z, Some(owner), [cx, cy + 2], [cx + 3, cy - 1], color, 1);
    }
}

fn paint_cardinal_arrow(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, dir: [i32; 2], color: Rgba8) {
    let [cx, cy] = rect.center();
    match dir {
        [-1, 0] => {
            builder.paint_line(z, Some(owner), [cx + 2, cy - 3], [cx - 2, cy], color, 1);
            builder.paint_line(z, Some(owner), [cx - 2, cy], [cx + 2, cy + 3], color, 1);
        }
        [1, 0] => {
            builder.paint_line(z, Some(owner), [cx - 2, cy - 3], [cx + 2, cy], color, 1);
            builder.paint_line(z, Some(owner), [cx + 2, cy], [cx - 2, cy + 3], color, 1);
        }
        [0, -1] => paint_vertical_arrow(builder, z, owner, rect, true, color),
        _ => paint_vertical_arrow(builder, z, owner, rect, false, color),
    }
}

fn paint_plus_icon(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, color: Rgba8) {
    let [cx, cy] = rect.center();
    builder.paint_line(z, Some(owner), [cx - 3, cy], [cx + 3, cy], color, 1);
    builder.paint_line(z, Some(owner), [cx, cy - 3], [cx, cy + 3], color, 1);
}

fn paint_cross_icon(builder: &mut LayoutBuilder<'_>, z: ZLayer, owner: &WidgetId, rect: LayoutRectPx, color: Rgba8) {
    let [cx, cy] = rect.center();
    builder.paint_line(z, Some(owner), [cx - 3, cy - 3], [cx + 3, cy + 3], color, 1);
    builder.paint_line(z, Some(owner), [cx - 3, cy + 3], [cx + 3, cy - 3], color, 1);
}

fn push_circle_op(
    builder: &mut LayoutBuilder<'_>,
    z_layer: ZLayer,
    owner: &WidgetId,
    center: [i32; 2],
    radius: u32,
    fill: Rgba8,
    stroke: Option<Rgba8>,
) {
    let order = builder.next_order();
    builder.tree.paint_ops.push(PaintOp {
        z_layer,
        paint_order: order,
        clip: Some(builder.active_clip_id()),
        owner_widget_id: Some(owner.clone()),
        kind: PaintOpKind::CircleIndicator { center, radius, fill, stroke },
    });
}

fn shift_color(color: Rgba8, delta: i16) -> Rgba8 {
    let shift = |ch: u8| -> u8 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let c = i16::from(ch).saturating_add(delta).clamp(0, 255) as u8;
        c
    };
    Rgba8::rgba(shift(color.r), shift(color.g), shift(color.b), color.a)
}

fn is_row_visible(row: LayoutRectPx, clip: LayoutRectPx) -> bool {
    row.bottom() > clip.y + 2 && row.y < clip.bottom() - 2
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

const fn folder_slug(folder: SheetFolderKind) -> &'static str {
    match folder {
        SheetFolderKind::Data => "data",
        SheetFolderKind::Info => "info",
        SheetFolderKind::Results => "results",
        SheetFolderKind::Graphs => "graphs",
        SheetFolderKind::Layouts => "layouts",
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

fn round_u32(v: f64) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let res = v.round().clamp(0.0, f64::from(u32::MAX)) as u32;
    res
}

fn round_i32(v: f64) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let res = v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    res
}

fn nav_tr<'a>(locale: &str, key: &str, fallback: &'a str) -> &'a str {
    crate::i18n::try_tr(locale, key).unwrap_or(fallback)
}

/// Map [`SheetSection`] onto [`SheetFolderKind`].
#[must_use]
pub const fn folder_kind_for_section(section: SheetSection) -> SheetFolderKind {
    match section {
        SheetSection::Data => SheetFolderKind::Data,
        SheetSection::Info => SheetFolderKind::Info,
        SheetSection::Analyses => SheetFolderKind::Results,
        SheetSection::Graphs => SheetFolderKind::Graphs,
        SheetSection::Layouts => SheetFolderKind::Layouts,
    }
}

/// Map [`SheetFolderKind`] onto [`SheetSection`].
#[must_use]
pub const fn section_for_folder_kind(folder: SheetFolderKind) -> SheetSection {
    match folder {
        SheetFolderKind::Data => SheetSection::Data,
        SheetFolderKind::Info => SheetSection::Info,
        SheetFolderKind::Results => SheetSection::Analyses,
        SheetFolderKind::Graphs => SheetSection::Graphs,
        SheetFolderKind::Layouts => SheetSection::Layouts,
    }
}
