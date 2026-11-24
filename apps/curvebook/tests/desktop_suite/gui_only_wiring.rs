//! Integration tests for `--gui-only` policy wiring in the desktop application.
#![forbid(unsafe_code)]

use std::process::Command;

use curvebook::configure_gui_only;
use curvebook::desktop_host::{DesktopHostConfig, DesktopHostEvent, DesktopHostRuntime};
use curvebook_ui_egui::{ViewportMetrics, dispatch_gui_action};
use serde_json::{Value, json};

#[test]
fn desktop_binary_documents_gui_only_and_starts_headless_strict() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");

    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success(), "--help failed: {:?}", String::from_utf8_lossy(&help.stderr));
    let help_stdout = String::from_utf8(help.stdout)?;
    assert!(help_stdout.contains("--gui-only"), "missing --gui-only in desktop --help:\n{help_stdout}");

    let portable = std::env::temp_dir().join(format!("cb-desktop-guionly-{}", std::process::id()));
    std::fs::create_dir_all(&portable)?;

    let headless = Command::new(bin).args(["--headless-once", "--gui-only", "--portable-dir"]).arg(&portable).output()?;
    let _ = std::fs::remove_dir_all(&portable);
    assert!(headless.status.success(), "--headless-once --gui-only failed: {:?}", String::from_utf8_lossy(&headless.stderr));

    Ok(())
}

#[test]
fn desktop_host_gui_only_blocks_direct_control_and_preserves_policy_across_windows() -> Result<(), Box<dyn std::error::Error>> {
    let mut rt =
        DesktopHostRuntime::new(DesktopHostConfig { in_window_menus: true, new_instance: false, gui_only: true, ..DesktopHostConfig::default() });
    let w1 = rt.open_window(None, ViewportMetrics::new(1024.0, 640.0, 1.0))?;
    assert!(rt.window(w1)?.app().is_gui_only());

    // Direct engine/command mutations over control channel must fail with `gui_only_violation`.
    for request in [
        json!({"id": 1, "method": "engine.execute", "params": {"command_id": "sheet.createData", "params": {"kind": "xy", "title": "Bypass"}}}),
        json!({"id": 2, "method": "command.run", "params": {"command": "sheet.createData", "params": {"kind": "xy"}}}),
        json!({"id": 3, "method": "command.batch", "params": {"commands": [{"command": "sheet.createData", "params": {"kind": "xy"}}]}}),
        json!({"id": 4, "method": "sheet.createData", "params": {"kind": "xy", "title": "Direct"}}),
        json!({"id": 5, "method": "ui.theme.set", "params": {"mode": "dark"}}),
    ] {
        let line = serde_json::to_string(&request)?;
        let resp: Value = serde_json::from_str(&rt.window_mut(w1)?.app_mut().dispatch_control_line(&line))?;
        assert_eq!(resp["ok"], json!(false), "expected gui_only_violation for {request}");
        assert_eq!(resp["error_detail"]["code"], json!("gui_only_violation"), "unexpected error_detail for {request}: {resp}");
    }

    // Read-only control inspection continues to work.
    let ping: Value = serde_json::from_str(&rt.window_mut(w1)?.app_mut().dispatch_control_line(r#"{"id":10,"method":"ping","params":{}}"#))?;
    assert_eq!(ping["ok"], json!(true));
    let ui_state: Value = serde_json::from_str(&rt.window_mut(w1)?.app_mut().dispatch_control_line(r#"{"id":11,"method":"ui.state","params":{}}"#))?;
    assert_eq!(ui_state["ok"], json!(true));

    // Interactive GUI dispatch remains allowed and drives the real UiApp under gui_only.
    let app = rt.window_mut(w1)?.app_mut();
    let inspected = dispatch_gui_action(app, "ui.inspect", json!({ "scope": "all" }))?;
    assert_eq!(inspected["gui_only"], json!(true));

    let opened = dispatch_gui_action(app, "ui.invoke_menu", json!({ "path": "Analyze/Power and Sample Size..." }))?;
    assert!(opened.is_object(), "expected object response from ui.invoke_menu: {opened}");
    let dialog_state = dispatch_gui_action(app, "ui.inspect", json!({ "scope": "dialog" }))?;
    assert_eq!(dialog_state["active_dialog"], json!("power_and_sample_size"));

    for _ in 0..3 {
        let next = dispatch_gui_action(app, "ui.answer_dialog", json!({ "action": "next" }))?;
        assert!(next.is_object(), "expected object response from ui.answer_dialog next: {next}");
    }
    let created = dispatch_gui_action(app, "ui.answer_dialog", json!({ "action": "create" }))?;
    assert!(created.is_object(), "expected object response from ui.answer_dialog create: {created}");

    let post_wizard = dispatch_gui_action(app, "ui.inspect", json!({ "scope": "all" }))?;
    assert_eq!(post_wizard["active_dialog"], json!("none"), "expected dialog closed after create: {post_wizard}");
    let sheets = post_wizard["navigator_sheets"].as_array().ok_or("missing navigator_sheets")?;
    assert!(sheets.len() >= 3, "expected power wizard to create data/result/graph sheets, got {sheets:?}");

    let shot = dispatch_gui_action(app, "ui.screenshot", json!({ "target": "full_window" }))?;
    assert!(shot["width_px"].as_u64().is_some_and(|w| w > 0), "missing width_px: {shot}");
    assert!(shot["height_px"].as_u64().is_some_and(|h| h > 0), "missing height_px: {shot}");
    let png_b64 = shot["data_base64"].as_str().ok_or("missing screenshot data_base64 payload")?;
    assert!(png_b64.starts_with("iVBORw0KGgo"), "expected PNG base64 header, got: {shot}");

    // Opening a second window inherits DesktopHostConfig.gui_only = true, and toggle helper works.
    let opened_second = rt.dispatch(DesktopHostEvent::WindowOpened { initial_file: None, viewport: ViewportMetrics::new(960.0, 600.0, 1.0) })?;
    let w2 = opened_second.opened_window_id.ok_or("expected opened second window")?;
    assert!(rt.window(w2)?.app().is_gui_only());

    configure_gui_only(rt.window_mut(w2)?.app_mut(), false);
    assert!(!rt.window(w2)?.app().is_gui_only());
    configure_gui_only(rt.window_mut(w2)?.app_mut(), true);
    assert!(rt.window(w2)?.app().is_gui_only());

    Ok(())
}
