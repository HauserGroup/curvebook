//! Permanent regression tests for Stage 4C finding `R1-S10-09` (sub-parts a–d).

use std::error::Error;
use std::io::{self, Cursor};
use std::path::Path;

use lumencraft_ui_egui::UiApp;
use lumencraft_ui_egui::control_io::{ControlAuth, MAX_CONTROL_LINE_BYTES, handle_control_line, serve_control_stream};
use lumencraft_ui_egui::menus::ui_commands::UI_COMMANDS;
use serde_json::{Value, json};

#[test]
fn r1_s10_09_a_non_utf8_line_returns_error_without_disconnecting() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let auth = ControlAuth::from_token("audit-r1-s10-09-token")?;
    let mut input = Vec::new();
    input.extend_from_slice(br#"{"id":0,"method":"auth","params":{"token":"audit-r1-s10-09-token"}}"#);
    input.push(b'\n');
    input.extend_from_slice(br#"{"id":1,"method":"ping"}"#);
    input.push(b'\n');
    input.extend_from_slice(b"\xff\xfe\n");
    input.extend_from_slice(br#"{"id":2,"method":"ping"}"#);
    input.push(b'\n');
    input.extend_from_slice(b"\x80\x81bad\n");
    input.extend_from_slice(br#"{"id":3,"method":"ui.state"}"#);
    input.push(b'\n');

    let mut out = Vec::new();
    serve_control_stream(&mut app, &mut Cursor::new(input), &mut out, &auth)?;

    let text = String::from_utf8(out)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 6, "expected 6 replies without disconnect, got: {lines:?}");

    let r0: Value = serde_json::from_str(lines[0])?;
    let r1: Value = serde_json::from_str(lines[1])?;
    let r2: Value = serde_json::from_str(lines[2])?;
    let r3: Value = serde_json::from_str(lines[3])?;
    let r4: Value = serde_json::from_str(lines[4])?;
    let r5: Value = serde_json::from_str(lines[5])?;

    assert_eq!(r0["ok"], json!(true));
    assert_eq!(r1["ok"], json!(true));
    assert_eq!(r1["id"], json!(1));
    assert_eq!(r2["ok"], json!(false));
    assert!(r2["error"].as_str().is_some_and(|e| e.contains("UTF-8")), "unexpected UTF-8 error: {r2}");
    assert_eq!(r3["ok"], json!(true));
    assert_eq!(r3["id"], json!(2));
    assert_eq!(r4["ok"], json!(false));
    assert!(r4["error"].as_str().is_some_and(|e| e.contains("UTF-8")), "unexpected UTF-8 error: {r4}");
    assert_eq!(r5["ok"], json!(true));
    assert_eq!(r5["id"], json!(3));
    Ok(())
}

#[test]
fn r1_s10_09_a_oversized_frame_emits_error_before_closing() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let auth = ControlAuth::from_token("audit-r1-s10-09-token")?;
    let mut oversized = vec![b'a'; MAX_CONTROL_LINE_BYTES + 16];
    oversized.push(b'\n');

    let mut out = Vec::new();
    let err =
        serve_control_stream(&mut app, &mut Cursor::new(oversized), &mut out, &auth).err().ok_or("expected InvalidData error for oversized frame")?;
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let text = String::from_utf8(out)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1, "expected best-effort JSON error frame before close, got: {lines:?}");
    let reply: Value = serde_json::from_str(lines[0])?;
    assert_eq!(reply["ok"], json!(false));
    let max_bytes_text = MAX_CONTROL_LINE_BYTES.to_string();
    assert!(
        reply["error"].as_str().is_some_and(|msg| msg.contains("exceeds") && msg.contains(&max_bytes_text)),
        "expected size limit explanation in reply: {reply}"
    );
    Ok(())
}

#[test]
fn r1_s10_09_b_docs_and_params_doc_cover_handler_parameters() -> Result<(), Box<dyn Error>> {
    for spec in UI_COMMANDS {
        let parsed: Value =
            serde_json::from_str(spec.params_doc).map_err(|err| format!("UI_COMMANDS `{}` has non-JSON params_doc: {err}", spec.id))?;
        assert!(parsed.is_object(), "UI_COMMANDS `{}` params_doc must be a JSON object", spec.id);
    }

    let find_doc = |id: &str| -> Result<&'static str, String> {
        UI_COMMANDS.iter().find(|spec| spec.id == id).map(|spec| spec.params_doc).ok_or_else(|| format!("missing UI command `{id}`"))
    };

    let gallery_doc = find_doc("ui.gallery.toggle")?;
    assert!(gallery_doc.contains("\"open\""), "ui.gallery.toggle missing open param: {gallery_doc}");
    assert!(gallery_doc.contains("all|"), "ui.gallery.toggle missing folder 'all': {gallery_doc}");

    let rulers_doc = find_doc("ui.rulers.toggle")?;
    assert!(rulers_doc.contains("\"visible\""), "ui.rulers.toggle missing visible param: {rulers_doc}");
    assert!(rulers_doc.contains("\"unit\""), "ui.rulers.toggle missing unit param: {rulers_doc}");

    let screenshot_doc = find_doc("ui.screenshot")?;
    assert!(screenshot_doc.contains("\"target\""), "ui.screenshot missing target param: {screenshot_doc}");
    assert!(screenshot_doc.contains("64..=4096"), "ui.screenshot missing pixel bounds: {screenshot_doc}");
    assert!(screenshot_doc.contains("72..=600"), "ui.screenshot missing DPI bounds: {screenshot_doc}");
    assert!(screenshot_doc.contains("--gui-only"), "ui.screenshot missing --gui-only restriction note: {screenshot_doc}");

    let proto_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/control-protocol.md");
    let proto_doc = std::fs::read_to_string(&proto_path)?;
    assert!(!proto_doc.contains("the `32` synced"), "control-protocol.md retains stale 32 command count");
    assert!(proto_doc.contains("\"auth\"") && proto_doc.contains("\"doc.export\"") && proto_doc.contains("\"render.preview\""));
    assert!(proto_doc.contains("`all`: optional `bool`"), "control-protocol.md missing render.pdf `all` documentation");
    assert!(proto_doc.contains("`path`: optional host filesystem destination"), "control-protocol.md missing render.pdf `path` documentation");
    assert!(proto_doc.contains("Oversized frames") && proto_doc.contains("HTTP request lines") && proto_doc.contains("TCP idle timeout"));
    Ok(())
}

#[test]
fn r1_s10_09_c_locale_set_omits_tautological_preset_applied() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();

    for (id, locale, expected_decimal, expected_csv) in [(1, "de", ",", ";"), (2, "he", ".", ",")] {
        let req = json!({
            "id": id,
            "method": "ui.locale.set",
            "params": { "locale": locale },
        });
        let raw_reply = handle_control_line(&mut app, &req.to_string());
        let reply: Value = serde_json::from_str(&raw_reply)?;
        assert_eq!(reply["ok"], json!(true), "locale set failed for {locale}: {reply}");
        let result = reply["result"].as_object().ok_or("ui.locale.set result must be object")?;
        assert_eq!(result.get("locale"), Some(&json!(locale)));
        assert_eq!(result.get("decimal_separator"), Some(&json!(expected_decimal)));
        assert_eq!(result.get("csv_delimiter"), Some(&json!(expected_csv)));
        assert!(result.get("display_name").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
        assert!(!result.contains_key("preset_applied"), "tautological preset_applied must be omitted: {reply}");
    }
    Ok(())
}

#[test]
fn r1_s10_09_d_command_palette_tracks_live_shortcuts() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let palette_default = app.shortcuts.active_combo("ui.palette.open").ok_or("missing ui.palette.open shortcut")?.to_owned();
    assert!(!palette_default.is_empty());

    // 1. Bind a custom shortcut to `ui.gallery.toggle` (default "").
    let set_reply: Value = serde_json::from_str(&handle_control_line(
        &mut app,
        &json!({
            "id": 1,
            "method": "ui.shortcut.set",
            "params": { "command_id": "ui.gallery.toggle", "shortcut": "Ctrl+Shift+G" }
        })
        .to_string(),
    ))?;
    assert_eq!(set_reply["ok"], json!(true), "{set_reply}");

    let open_reply: Value = serde_json::from_str(&handle_control_line(&mut app, &json!({"id": 2, "method": "ui.palette.open"}).to_string()))?;
    assert_eq!(open_reply["ok"], json!(true), "{open_reply}");

    let list_reply: Value = serde_json::from_str(&handle_control_line(&mut app, &json!({"id": 3, "method": "command.list"}).to_string()))?;
    let list_gallery_shortcut = list_reply["result"]["commands"]
        .as_array()
        .and_then(|arr| arr.iter().find(|entry| entry["id"] == "ui.gallery.toggle"))
        .and_then(|entry| entry["shortcut"].as_str())
        .ok_or("missing ui.gallery.toggle in command.list")?;
    assert_eq!(list_gallery_shortcut, "Ctrl+Shift+G");

    // 2. Search by the live shortcut combo and verify matches + compact/preview rows reflect it.
    app.canvas_dialogs.command_palette.query = String::from("Ctrl+Shift+G");
    let hits_default = app.canvas_dialogs.command_palette.matches();
    let hits_session = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    let hits_direct = app.canvas_dialogs.command_palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts);

    for hits in [&hits_default, &hits_session, &hits_direct] {
        let entry = hits.iter().find(|e| e.id == "ui.gallery.toggle").ok_or("gallery missing when searching live shortcut")?;
        assert_eq!(entry.shortcut, "Ctrl+Shift+G");
        assert!(entry.format_compact_row().contains("[Ctrl+Shift+G]"), "compact row missing shortcut: {}", entry.format_compact_row());
    }

    let preview = app.canvas_dialogs.command_palette.preview_rows_for_locale(Some(app.session()), &app.locale, 5);
    assert!(
        preview.iter().any(|row| row.contains("ui.gallery.toggle") && row.contains("[Ctrl+Shift+G]")),
        "preview_rows missing live shortcut: {preview:?}"
    );

    // 3. Unbind `ui.palette.open` and verify the stale default combo disappears even after `open_command_palette()`.
    let unbind_reply: Value = serde_json::from_str(&handle_control_line(
        &mut app,
        &json!({
            "id": 4,
            "method": "ui.shortcut.set",
            "params": { "command_id": "ui.palette.open", "shortcut": "" }
        })
        .to_string(),
    ))?;
    assert_eq!(unbind_reply["ok"], json!(true), "{unbind_reply}");

    app.canvas_dialogs.open_command_palette();
    app.canvas_dialogs.command_palette.query = String::from("ui.palette.open");
    let palette_hits = app.canvas_dialogs.command_palette.matches_for_session(app.session());
    let palette_entry = palette_hits.iter().find(|e| e.id == "ui.palette.open").ok_or("missing ui.palette.open")?;
    assert_eq!(palette_entry.shortcut, "");
    assert!(!palette_entry.format_compact_row().contains(&format!("[{palette_default}]")));

    // 4. Reset single shortcut, then reset all, confirming palette stays aligned with ShortcutRegistry.
    let reset_one: Value = serde_json::from_str(&handle_control_line(
        &mut app,
        &json!({"id": 5, "method": "ui.shortcut.reset", "params": {"command_id": "ui.gallery.toggle"}}).to_string(),
    ))?;
    assert_eq!(reset_one["ok"], json!(true), "{reset_one}");
    app.canvas_dialogs.command_palette.query = String::from("ui.gallery.toggle");
    let reset_gallery = app
        .canvas_dialogs
        .command_palette
        .matches_for_session(app.session())
        .into_iter()
        .find(|e| e.id == "ui.gallery.toggle")
        .ok_or("missing gallery after reset")?;
    assert_eq!(reset_gallery.shortcut, "");

    let reset_all: Value =
        serde_json::from_str(&handle_control_line(&mut app, &json!({"id": 6, "method": "ui.shortcut.reset", "params": {}}).to_string()))?;
    assert_eq!(reset_all["ok"], json!(true), "{reset_all}");
    app.canvas_dialogs.command_palette.query = String::from("ui.palette.open");
    let restored_palette = app
        .canvas_dialogs
        .command_palette
        .matches_for_session(app.session())
        .into_iter()
        .find(|e| e.id == "ui.palette.open")
        .ok_or("missing ui.palette.open after reset_all")?;
    assert_eq!(restored_palette.shortcut, palette_default);
    assert!(restored_palette.format_compact_row().contains(&format!("[{palette_default}]")));
    Ok(())
}
