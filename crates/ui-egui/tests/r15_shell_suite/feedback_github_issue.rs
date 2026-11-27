//! Human-path and control-channel verification for direct GitHub feedback redirect
//! (`app.feedback`, zero-telemetry design).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use curvebook_ui_egui::os_integration::{StubPlatformClipboardAdapter, StubPlatformUrlOpener};
use curvebook_ui_egui::{ActiveDialog, GITHUB_FEEDBACK_URL, UiApp, menu_bar_tooltips, validate_github_issue_url};
use serde_json::{Value, json};

use crate::common::{TestRes, call_control, make_default_driver};

#[test]
fn help_menu_registers_app_feedback_alongside_about() {
    let app = UiApp::new();
    let sections = menu_bar_tooltips(&app);
    let help = sections.iter().find(|s| s.menu_key == "menu.help").expect("Help menu present");
    assert!(help.items.iter().any(|i| i.command_id == "app.about"));
    let fb = help.items.iter().find(|i| i.command_id == "app.feedback").expect("app.feedback present");
    assert_eq!(fb.label, "Send Feedback");
    assert!(fb.enabled && fb.read_only && !fb.summary.is_empty());
}

#[test]
fn human_path_feedback_opens_github_chooser_directly_from_help_and_about() -> TestRes {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);

    let mut driver = make_default_driver(app);
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.feedback")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert_eq!(url_stub.last_url().as_deref(), Some(GITHUB_FEEDBACK_URL));
    assert!(clip_stub.last_text().is_none(), "feedback redirect must never write to clipboard");

    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);
    driver.click_widget_id("dialog.about.send_feedback")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About, "About modal must stay open");
    assert_eq!(url_stub.captured_urls(), vec![GITHUB_FEEDBACK_URL, GITHUB_FEEDBACK_URL]);
    assert!(clip_stub.last_text().is_none(), "About Send Feedback must never write to clipboard");
    Ok(())
}

#[test]
fn unavailable_and_guarded_openers_record_status_banner_without_clipboard_writes() -> TestRes {
    let unavail = Arc::new(StubPlatformUrlOpener::unavailable());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&unavail) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);

    let resp = call_control(&mut app, &json!({ "id": "fb-unavail", "method": "app.feedback", "params": {} }))?;
    assert_eq!(resp["ok"].as_bool(), Some(true));
    assert_eq!(resp.pointer("/result/url").and_then(Value::as_str), Some(GITHUB_FEEDBACK_URL));
    assert_eq!(resp.pointer("/result/opened").and_then(Value::as_bool), Some(false));
    assert_eq!(resp.pointer("/result/availability").and_then(Value::as_str), Some("unavailable"));
    assert_eq!(
        app.canvas_dialogs.status_banner.as_deref(),
        Some("Could not open browser. Visit https://github.com/HauserGroup/curvebook/issues/new/choose")
    );
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    assert!(clip_stub.last_text().is_none());

    let mut guarded_app = UiApp::new();
    guarded_app.human_path_only = true;
    guarded_app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);
    let guarded_resp = call_control(&mut guarded_app, &json!({ "id": "fb-guarded", "method": "app.feedback", "params": {} }))?;
    assert_eq!(guarded_resp["ok"].as_bool(), Some(true));
    assert_eq!(guarded_resp.pointer("/result/opened").and_then(Value::as_bool), Some(false));
    assert_eq!(guarded_resp.pointer("/result/availability").and_then(Value::as_str), Some("unavailable"));
    assert_eq!(
        guarded_app.canvas_dialogs.status_banner.as_deref(),
        Some("Could not open browser. Visit https://github.com/HauserGroup/curvebook/issues/new/choose")
    );
    assert!(clip_stub.last_text().is_none());
    Ok(())
}

#[test]
fn control_channel_dispatches_app_feedback() -> TestRes {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);

    let resp = call_control(&mut app, &json!({ "id": "fb-1", "method": "app.feedback", "params": {} }))?;
    assert_eq!(resp["ok"].as_bool(), Some(true));
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::None);
    let url = resp.pointer("/result/url").and_then(Value::as_str).ok_or("missing result.url")?;
    assert_eq!(url, GITHUB_FEEDBACK_URL);
    validate_github_issue_url(url)?;
    assert_eq!(resp.pointer("/result/opened").and_then(Value::as_bool), Some(true));
    assert_eq!(resp.pointer("/result/availability").and_then(Value::as_str), Some("submitted"));
    assert_eq!(url_stub.last_url().as_deref(), Some(GITHUB_FEEDBACK_URL));
    Ok(())
}
