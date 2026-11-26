//! Permanent regression tests for Stage 4C finding `R1-S10-09` (sub-parts a–d).

use std::error::Error;
use std::io::{self, Cursor};
use std::path::Path;

use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::control_io::{ControlAuth, MAX_CONTROL_LINE_BYTES, serve_control_stream};
use curvebook_ui_egui::menus::ui_commands::UI_COMMANDS;
use serde_json::{Value, json};

use crate::common::call_control;

#[test]
fn r1_s10_09_a_non_utf8_line_returns_error_without_disconnecting() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let auth = ControlAuth::from_token("audit-r1-s10-09-token")?;
    let mut input = Vec::new();
    for line in [
        &br#"{"id":0,"method":"auth","params":{"token":"audit-r1-s10-09-token"}}"#[..],
        br#"{"id":1,"method":"ping"}"#,
        b"\xff\xfe",
        br#"{"id":2,"method":"ping"}"#,
        b"\x80\x81bad",
        br#"{"id":3,"method":"ui.state"}"#,
    ] {
        input.extend_from_slice(line);
        input.push(b'\n');
    }

    let mut out = Vec::new();
    serve_control_stream(&mut app, &mut Cursor::new(input), &mut out, &auth)?;
    let replies: Vec<Value> = String::from_utf8(out)?.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
    assert_eq!(replies.len(), 6, "expected 6 replies without disconnect");

    for (idx, expect_ok, expect_id) in
        [(0, true, None), (1, true, Some(1)), (2, false, None), (3, true, Some(2)), (4, false, None), (5, true, Some(3))]
    {
        assert_eq!(replies[idx]["ok"], json!(expect_ok), "reply[{idx}]: {}", replies[idx]);
        if let Some(id) = expect_id {
            assert_eq!(replies[idx]["id"], json!(id));
        }
        if !expect_ok {
            assert!(replies[idx]["error"].as_str().is_some_and(|e| e.contains("UTF-8")));
        }
    }
    Ok(())
}

#[test]
fn r1_s10_09_a_oversized_frame_emits_error_before_closing() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let auth = ControlAuth::from_token("audit-r1-s10-09-token")?;
    let mut oversized = vec![b'a'; MAX_CONTROL_LINE_BYTES + 16];
    oversized.push(b'\n');

    let mut out = Vec::new();
    let err = serve_control_stream(&mut app, &mut Cursor::new(oversized), &mut out, &auth).err().ok_or("expected InvalidData")?;
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);

    let text = String::from_utf8(out)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1);
    let reply: Value = serde_json::from_str(lines[0])?;
    assert_eq!(reply["ok"], json!(false));
    let max_text = MAX_CONTROL_LINE_BYTES.to_string();
    assert!(reply["error"].as_str().is_some_and(|m| m.contains("exceeds") && m.contains(&max_text)));
    Ok(())
}

#[test]
fn r1_s10_09_b_docs_and_params_doc_cover_handler_parameters() -> Result<(), Box<dyn Error>> {
    for spec in UI_COMMANDS {
        let parsed: Value = serde_json::from_str(spec.params_doc).map_err(|e| format!("`{}` non-JSON params_doc: {e}", spec.id))?;
        assert!(parsed.is_object(), "`{}` params_doc must be JSON object", spec.id);
    }
    for (id, required, forbidden) in [
        ("ui.gallery.toggle", &["\"open\"", "all|"][..], &[][..]),
        ("ui.rulers.toggle", &["\"visible\"", "\"unit\""], &[]),
        ("ui.screenshot", &["\"target\"", "64..=4096", "72..=600", "\"path\""], &["--gui-only"]),
    ] {
        let doc = UI_COMMANDS.iter().find(|s| s.id == id).ok_or_else(|| format!("missing `{id}`"))?.params_doc;
        for needle in required {
            assert!(doc.contains(needle), "`{id}` missing `{needle}`: {doc}");
        }
        for bad in forbidden {
            assert!(!doc.contains(bad), "`{id}` contains `{bad}`: {doc}");
        }
    }

    let proto_doc = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/control-protocol.md"))?;
    assert!(!proto_doc.contains("the `32` synced"));
    for needle in [
        "\"auth\"",
        "\"doc.export\"",
        "\"render.preview\"",
        "`all`: optional `bool`",
        "`path`: optional host filesystem destination",
        "Oversized frames",
        "HTTP request lines",
        "TCP idle timeout",
    ] {
        assert!(proto_doc.contains(needle), "control-protocol.md missing `{needle}`");
    }
    Ok(())
}

#[test]
fn r1_s10_09_c_locale_set_omits_tautological_preset_applied() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    for (id, locale, dec, csv) in [(1, "de", ",", ";"), (2, "he", ".", ",")] {
        let reply = call_control(&mut app, &json!({ "id": id, "method": "ui.locale.set", "params": { "locale": locale } }))?;
        assert_eq!(reply["ok"], json!(true), "{locale}: {reply}");
        let res = reply["result"].as_object().ok_or("result must be object")?;
        assert_eq!(
            (res.get("locale"), res.get("decimal_separator"), res.get("csv_delimiter")),
            (Some(&json!(locale)), Some(&json!(dec)), Some(&json!(csv)))
        );
        assert!(res.get("display_name").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
        assert!(!res.contains_key("preset_applied"));
    }
    Ok(())
}

#[test]
fn r1_s10_09_d_command_palette_tracks_live_shortcuts() -> Result<(), Box<dyn Error>> {
    let mut app = UiApp::default();
    let palette_default = app.shortcuts.active_combo("ui.palette.open").ok_or("missing ui.palette.open")?.to_owned();
    assert!(!palette_default.is_empty());

    assert_eq!(
        call_control(
            &mut app,
            &json!({ "id": 1, "method": "ui.shortcut.set", "params": { "command_id": "ui.gallery.toggle", "shortcut": "Ctrl+Shift+G" } })
        )?["ok"],
        json!(true)
    );
    assert_eq!(call_control(&mut app, &json!({ "id": 2, "method": "ui.palette.open" }))?["ok"], json!(true));
    let list_reply = call_control(&mut app, &json!({ "id": 3, "method": "command.list" }))?;
    let list_sc = list_reply["result"]["commands"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["id"] == "ui.gallery.toggle"))
        .and_then(|e| e["shortcut"].as_str());
    assert_eq!(list_sc, Some("Ctrl+Shift+G"));

    app.canvas_dialogs.command_palette.query = "Ctrl+Shift+G".into();
    for hits in [
        app.canvas_dialogs.command_palette.matches(),
        app.canvas_dialogs.command_palette.matches_for_session(app.session()),
        app.canvas_dialogs.command_palette.matches_with_shortcuts(Some(app.session()), &app.shortcuts),
    ] {
        let entry = hits.iter().find(|e| e.id == "ui.gallery.toggle").ok_or("gallery missing")?;
        assert_eq!(entry.shortcut, "Ctrl+Shift+G");
        assert!(entry.format_compact_row().contains("[Ctrl+Shift+G]"));
    }
    let preview = app.canvas_dialogs.command_palette.preview_rows_for_locale(Some(app.session()), &app.locale, 5);
    assert!(preview.iter().any(|r| r.contains("ui.gallery.toggle") && r.contains("[Ctrl+Shift+G]")));

    assert_eq!(
        call_control(&mut app, &json!({ "id": 4, "method": "ui.shortcut.set", "params": { "command_id": "ui.palette.open", "shortcut": "" } }))?["ok"],
        json!(true)
    );
    app.canvas_dialogs.open_command_palette();
    app.canvas_dialogs.command_palette.query = "ui.palette.open".into();
    let pe = app
        .canvas_dialogs
        .command_palette
        .matches_for_session(app.session())
        .into_iter()
        .find(|e| e.id == "ui.palette.open")
        .ok_or("missing palette")?;
    assert!(pe.shortcut.is_empty() && !pe.format_compact_row().contains(&format!("[{palette_default}]")));

    assert_eq!(
        call_control(&mut app, &json!({ "id": 5, "method": "ui.shortcut.reset", "params": { "command_id": "ui.gallery.toggle" } }))?["ok"],
        json!(true)
    );
    app.canvas_dialogs.command_palette.query = "ui.gallery.toggle".into();
    assert_eq!(
        app.canvas_dialogs
            .command_palette
            .matches_for_session(app.session())
            .into_iter()
            .find(|e| e.id == "ui.gallery.toggle")
            .ok_or("gallery")?
            .shortcut,
        ""
    );

    assert_eq!(call_control(&mut app, &json!({ "id": 6, "method": "ui.shortcut.reset", "params": {} }))?["ok"], json!(true));
    app.canvas_dialogs.command_palette.query = "ui.palette.open".into();
    let restored =
        app.canvas_dialogs.command_palette.matches_for_session(app.session()).into_iter().find(|e| e.id == "ui.palette.open").ok_or("palette")?;
    assert_eq!(restored.shortcut, palette_default);
    assert!(restored.format_compact_row().contains(&format!("[{palette_default}]")));
    Ok(())
}
