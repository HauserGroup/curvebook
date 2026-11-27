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
    let cite = help.items.iter().find(|i| i.command_id == "app.cite").expect("app.cite present");
    assert_eq!(cite.label, "Cite CurveBook");
    assert!(cite.enabled && cite.read_only && !cite.summary.is_empty());
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

#[test]
fn human_path_cite_dialog_opens_from_help_about_and_methods_copies_and_opens_link_on_click() -> TestRes {
    use curvebook_doc::CURVEBOOK_CITATION;

    let version = env!("CARGO_PKG_VERSION");
    let expected_ref = CURVEBOOK_CITATION.plain_text_reference(Some(version));
    let expected_bib = CURVEBOOK_CITATION.bibtex(Some(version));
    let expected_url = CURVEBOOK_CITATION.primary_citation_url();

    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);
    app.execute_command("sheet.createData", json!({ "sheet_id": "ds-cite", "kind": "column", "cols": 2, "rows": 3 })).map_err(|e| e.to_string())?;
    for (col, vals) in [(0usize, [1.0, 2.0, 3.0]), (1usize, [4.0, 5.0, 6.0])] {
        for (row, v) in vals.iter().enumerate() {
            app.execute_command("table.setCell", json!({ "sheet_id": "ds-cite", "row": row, "col": col, "value": v })).map_err(|e| e.to_string())?;
        }
    }
    app.execute_command("stats.run", json!({ "source_sheet_id": "ds-cite", "sheet_id": "res-cite", "kind": "descriptive_stats" }))
        .map_err(|e| e.to_string())?;
    app.execute_command("sheet.select", json!({ "sheet_id": "res-cite" })).map_err(|e| e.to_string())?;
    app.sync_views();

    let mut driver = make_default_driver(app);

    // 1. Open from Help menu.
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.cite")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Cite);
    assert!(url_stub.captured_urls().is_empty(), "opening Cite dialog must trigger zero automatic URL opens");
    let ref_val = driver
        .frame_meta()
        .find_by_widget_id("dialog.cite.reference_text")
        .and_then(|e| e.value.as_ref())
        .and_then(Value::as_str)
        .ok_or("missing dialog.cite.reference_text")?;
    assert_eq!(ref_val, expected_ref);
    let bib_val = driver
        .frame_meta()
        .find_by_widget_id("dialog.cite.bibtex_text")
        .and_then(|e| e.value.as_ref())
        .and_then(Value::as_str)
        .ok_or("missing dialog.cite.bibtex_text")?;
    assert_eq!(bib_val, expected_bib);
    assert!(driver.verify_pixel_to_hit_box_parity().is_clean(), "Cite dialog parity must be clean");

    // Copy reference & BibTeX.
    driver.click_widget_id("dialog.cite.copy_reference")?;
    assert_eq!(clip_stub.last_text().as_deref(), Some(expected_ref.as_str()));
    assert_eq!(driver.frame_meta().find_by_widget_id("dialog.cite.copy_reference").ok_or("missing copy_reference")?.label, "Copied");
    driver.click_widget_id("dialog.cite.copy_bibtex")?;
    assert_eq!(clip_stub.last_text().as_deref(), Some(expected_bib.as_str()));
    assert_eq!(driver.frame_meta().find_by_widget_id("dialog.cite.copy_bibtex").ok_or("missing copy_bibtex")?.label, "Copied");

    // External link opens strictly on click and keeps dialog open.
    driver.click_widget_id("dialog.cite.external_link")?;
    assert_eq!(url_stub.captured_urls(), vec![expected_url.clone()]);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Cite);

    // Close via Escape.
    driver.press_egui_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 2. Open from About CurveBook dialog.
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);
    driver.click_widget_id("dialog.about.cite")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Cite);
    driver.click_widget_id("dialog.cite.close")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 3. Open from Result Methods panel `Cite` button.
    driver.click_widget_id("inspector.results.tab.methods")?;
    driver.click_widget_id("inspector.cite_curvebook")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Cite);
    driver.click_widget_id("dialog.cite.close")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);

    // 4. Control-channel `app.cite` payload + unavailable URL opener banner fallback.
    let unavail = Arc::new(StubPlatformUrlOpener::unavailable());
    let mut ctl_app = UiApp::new();
    ctl_app.set_url_opener(Arc::clone(&unavail) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    let resp = call_control(&mut ctl_app, &json!({ "id": "cite-1", "method": "app.cite", "params": {} }))?;
    assert_eq!(resp["ok"].as_bool(), Some(true));
    assert_eq!(ctl_app.canvas_dialogs.active_dialog, ActiveDialog::Cite);
    assert_eq!(resp.pointer("/result/version").and_then(Value::as_str), Some(version));
    assert_eq!(resp.pointer("/result/reference").and_then(Value::as_str), Some(expected_ref.as_str()));
    assert_eq!(resp.pointer("/result/bibtex").and_then(Value::as_str), Some(expected_bib.as_str()));
    assert_eq!(resp.pointer("/result/url").and_then(Value::as_str), Some(expected_url.as_str()));
    let open_status = ctl_app.open_citation_url()?;
    assert!(!open_status.available);
    assert_eq!(ctl_app.canvas_dialogs.status_banner.as_deref(), Some(format!("Could not open browser. Visit {expected_url}").as_str()));
    Ok(())
}
