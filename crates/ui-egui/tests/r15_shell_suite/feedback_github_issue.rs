//! Human-path and control-channel verification for the in-app Feedback -> Prefilled GitHub Issue
//! modal (`app.feedback`, zero-telemetry design).
#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use curvebook_ui_egui::interaction::KeyModifiers;
use curvebook_ui_egui::os_integration::{StubPlatformClipboardAdapter, StubPlatformUrlOpener};
use curvebook_ui_egui::{
    ActiveDialog, FeedbackCategory, GITHUB_ISSUES_NEW_BASE, MAX_GITHUB_ISSUE_URL_BYTES, UiApp, menu_bar_tooltips, validate_github_issue_url,
};
use serde_json::{Value, json};

use crate::common::{TestRes, call_control, make_default_driver};

#[test]
fn help_menu_registers_app_feedback_alongside_about() {
    let app = UiApp::new();
    let sections = menu_bar_tooltips(&app);
    let help = sections.iter().find(|s| s.menu_key == "menu.help").expect("Help menu present");
    assert!(help.items.iter().any(|i| i.command_id == "app.about"));
    let fb = help.items.iter().find(|i| i.command_id == "app.feedback").expect("app.feedback present");
    assert!(fb.enabled && fb.read_only && !fb.summary.is_empty());
}

#[test]
fn human_path_feedback_modal_builds_prefilled_github_url_and_copies_markdown() -> TestRes {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);

    let mut driver = make_default_driver(app);
    driver.click_widget_id("menu.help")?;
    driver.click_widget_id("menu.app.about")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);
    driver.click_widget_id("dialog.about.send_feedback")?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Feedback);

    for (wid, cat) in [("feedback.category.feature", FeedbackCategory::FeatureRequest), ("feedback.category.bug", FeedbackCategory::BugReport)] {
        driver.click_widget_id(wid)?;
        assert_eq!(driver.state().app.canvas_dialogs.feedback.category, cat);
    }

    driver.click_widget_id("feedback.title")?;
    driver.type_text("Legend overlaps axis label")?;
    driver.click_widget_id("feedback.summary")?;
    driver.type_text("Legend text clips on 88mm preset")?;
    driver.click_widget_id("dialog.feedback.open_github")?;

    let opened_url = url_stub.last_url().ok_or("missing captured URL")?;
    validate_github_issue_url(&opened_url)?;
    assert!(opened_url.starts_with(GITHUB_ISSUES_NEW_BASE));
    for needle in [
        "template=bug_report.yml",
        "title=Legend%20overlaps%20axis%20label",
        "what-happened=Legend%20text%20clips%20on%2088mm%20preset",
        "version=CurveBook",
        "surface=Desktop%20app",
    ] {
        assert!(opened_url.contains(needle), "missing `{needle}` in {opened_url}");
    }

    driver.click_widget_id("feedback.include_diagnostics")?;
    assert!(!driver.state().app.canvas_dialogs.feedback.include_diagnostics);
    driver.click_widget_id("dialog.feedback.open_github")?;
    let url_no_diag = url_stub.last_url().ok_or("missing second URL")?;
    validate_github_issue_url(&url_no_diag)?;
    assert!(!url_no_diag.contains("&version=") && !url_no_diag.contains("&platform="));

    driver.click_widget_id("dialog.feedback.copy_markdown")?;
    let md = clip_stub.last_text().ok_or("missing clipboard Markdown")?;
    assert!(md.contains("# Legend overlaps axis label") && md.contains("## What happened") && md.contains("Legend text clips on 88mm preset"));
    assert!(driver.state().app.canvas_dialogs.feedback.copied_markdown);

    driver.click_widget_id("dialog.feedback.reset")?;
    assert!(driver.state().app.canvas_dialogs.feedback.title.is_empty() && driver.state().app.canvas_dialogs.feedback.summary.is_empty());
    driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn oversized_feedback_truncates_url_cleanly_while_markdown_stays_complete() -> TestRes {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());
    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);
    app.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    app.canvas_dialogs.feedback.category = FeedbackCategory::NumericalResult;
    app.canvas_dialogs.feedback.title = "4PL EC50 CI mismatch".into();
    app.canvas_dialogs.feedback.summary = "value=1.234567890 ".repeat(400);
    app.canvas_dialogs.feedback.details = "R drc::drm gives 1.2345".into();
    app.canvas_dialogs.feedback.extra = "Four-parameter logistic fit".into();

    let (built, status) = app.open_feedback_on_github()?;
    assert!(status.available && built.truncated && built.omitted_chars > 0 && built.url.len() <= MAX_GITHUB_ISSUE_URL_BYTES);
    validate_github_issue_url(&built.url)?;
    assert!(built.url.contains("template=numerical_result.yml") && built.url.contains("title=Numerical%3A%204PL%20EC50%20CI%20mismatch"));

    let (md, clip_status) = app.copy_feedback_markdown()?;
    assert!(clip_status.available && md.len() > MAX_GITHUB_ISSUE_URL_BYTES && md.contains("R drc::drm gives 1.2345"));
    assert_eq!(clip_stub.last_text().as_deref(), Some(md.as_str()));
    Ok(())
}

#[test]
fn control_channel_dispatches_app_feedback() -> TestRes {
    let mut app = UiApp::new();
    let resp = call_control(&mut app, &json!({ "id": "fb-1", "method": "app.feedback", "params": { "category": "feature" } }))?;
    assert_eq!(
        (resp["ok"].as_bool(), &app.canvas_dialogs.active_dialog, app.canvas_dialogs.feedback.category),
        (Some(true), &ActiveDialog::Feedback, FeedbackCategory::FeatureRequest)
    );
    assert!(validate_github_issue_url(resp.pointer("/result/preview_url").and_then(Value::as_str).ok_or("preview_url")?).is_ok());

    let bad = call_control(&mut app, &json!({ "id": "fb-2", "method": "app.feedback", "params": { "category": "bogus" } }))?;
    assert_eq!(bad["ok"].as_bool(), Some(false));
    Ok(())
}
