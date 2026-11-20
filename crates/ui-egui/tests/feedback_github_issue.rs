#![forbid(unsafe_code)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Human-path and control-channel verification for the in-app Feedback -> Prefilled GitHub Issue
//! modal (`app.feedback`, Option 1 zero-telemetry design).

use std::sync::Arc;

use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::inspect_bridge::render_egui_shell_frame;
use curvebook_ui_egui::egui_shell::{EguiShellViewState, PendingUiIntents, Tokens};
use curvebook_ui_egui::interaction::{KeyModifiers, ViewportMetrics};
use curvebook_ui_egui::os_integration::{StubPlatformClipboardAdapter, StubPlatformUrlOpener};
use curvebook_ui_egui::{
    ActiveDialog, FeedbackCategory, GITHUB_ISSUES_NEW_BASE, MAX_GITHUB_ISSUE_URL_BYTES, UiApp, handle_control_line, menu_bar_tooltips,
    validate_github_issue_url,
};
use egui::Pos2;
use serde_json::{Value, json};

struct FeedbackHarnessState {
    app: UiApp,
    views: EguiShellViewState,
    intents: PendingUiIntents,
}

impl FeedbackHarnessState {
    fn new(app: UiApp) -> Self {
        let views = app.egui_views.clone();
        Self { app, views, intents: PendingUiIntents::new() }
    }
}

fn make_driver(app: UiApp) -> EguiHumanPathDriver<'static, FeedbackHarnessState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    let fonts = build_driver_font_definitions();
    let metrics = ViewportMetrics::new(1280.0, 800.0, 1.0);
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(FeedbackHarnessState::new(app), metrics, tokens, fonts, |ui, meta, st| {
        let _ = render_egui_shell_frame(ui, &st.app, &mut st.views, meta, &mut st.intents);
    });
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.set_post_frame_hook(|st, settled| {
        if settled {
            st.app.egui_views = st.views.clone();
            let _ = st.intents.commit_if_settled(&mut st.app, false);
            st.views = st.app.egui_views.clone();
        } else {
            st.intents.clear();
        }
    });
    driver.run_settled();
    driver
}

fn widget_center(driver: &EguiHumanPathDriver<'_, FeedbackHarnessState>, widget_id: &str) -> Result<Pos2, String> {
    let entry = driver.frame_meta().find_by_widget_id(widget_id).ok_or_else(|| format!("missing widget `{widget_id}`"))?;
    if !entry.clipped_rect.is_positive() {
        return Err(format!("widget `{widget_id}` has empty rect {:?}", entry.clipped_rect));
    }
    Ok(entry.clipped_rect.center())
}

#[test]
fn help_menu_registers_app_feedback_alongside_about() {
    let app = UiApp::new();
    let sections = menu_bar_tooltips(&app);
    let help = sections.iter().find(|s| s.menu_key == "menu.help").expect("Help menu section present");
    assert!(help.items.iter().any(|item| item.command_id == "app.about"), "Help menu must include app.about");
    let feedback_item = help.items.iter().find(|item| item.command_id == "app.feedback").expect("Help menu must include app.feedback");
    assert!(feedback_item.enabled);
    assert!(feedback_item.read_only);
    assert!(!feedback_item.summary.is_empty());
}

#[test]
fn human_path_feedback_modal_builds_prefilled_github_url_and_copies_markdown() -> Result<(), String> {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());

    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);

    let mut driver = make_driver(app);

    // 1. Open About dialog via visible top-bar About button, then click Send Feedback... inside About.
    let about_pos = widget_center(&driver, "toolbar.help.about")?;
    driver.click_at(about_pos)?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::About);

    let send_fb_pos = widget_center(&driver, "dialog.about.send_feedback")?;
    driver.click_at(send_fb_pos)?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::Feedback);

    // 2. Switch category to Feature request and back to Bug report via real egui tab clicks.
    let feat_pos = widget_center(&driver, "feedback.category.feature")?;
    driver.click_at(feat_pos)?;
    assert_eq!(driver.state().app.canvas_dialogs.feedback.category, FeedbackCategory::FeatureRequest);

    let bug_pos = widget_center(&driver, "feedback.category.bug")?;
    driver.click_at(bug_pos)?;
    assert_eq!(driver.state().app.canvas_dialogs.feedback.category, FeedbackCategory::BugReport);

    // 3. Type into the Title and What happened fields via real egui focus + text input.
    let title_pos = widget_center(&driver, "feedback.title")?;
    driver.click_at(title_pos)?;
    driver.type_text("Legend overlaps axis label")?;

    let summary_pos = widget_center(&driver, "feedback.summary")?;
    driver.click_at(summary_pos)?;
    driver.type_text("Legend text clips on 88mm preset")?;

    // 4. Click "Open on GitHub..." and verify the allowlisted prefilled URL is captured.
    let open_pos = widget_center(&driver, "dialog.feedback.open_github")?;
    driver.click_at(open_pos)?;

    let opened_url = url_stub.last_url().ok_or_else(|| String::from("expected StubPlatformUrlOpener to capture URL"))?;
    validate_github_issue_url(&opened_url)?;
    assert!(opened_url.starts_with(GITHUB_ISSUES_NEW_BASE));
    assert!(opened_url.contains("template=bug_report.yml"));
    assert!(opened_url.contains("title=Legend%20overlaps%20axis%20label"));
    assert!(opened_url.contains("what-happened=Legend%20text%20clips%20on%2088mm%20preset"));
    assert!(opened_url.contains("version=CurveBook"));
    assert!(opened_url.contains("surface=Desktop%20app"));

    // 5. Toggle diagnostics off and verify platform/version are omitted from the next launch.
    let diag_pos = widget_center(&driver, "feedback.include_diagnostics")?;
    driver.click_at(diag_pos)?;
    assert!(!driver.state().app.canvas_dialogs.feedback.include_diagnostics);

    let open_pos_2 = widget_center(&driver, "dialog.feedback.open_github")?;
    driver.click_at(open_pos_2)?;
    let url_no_diag = url_stub.last_url().ok_or_else(|| String::from("expected second captured URL"))?;
    validate_github_issue_url(&url_no_diag)?;
    assert!(!url_no_diag.contains("&version="), "version must be omitted when diagnostics unchecked: {url_no_diag}");
    assert!(!url_no_diag.contains("&platform="), "platform must be omitted when diagnostics unchecked: {url_no_diag}");

    // 6. Click "Copy Markdown" and verify full Markdown lands in the clipboard adapter.
    let copy_pos = widget_center(&driver, "dialog.feedback.copy_markdown")?;
    driver.click_at(copy_pos)?;
    let md = clip_stub.last_text().ok_or_else(|| String::from("expected Markdown report in StubPlatformClipboardAdapter"))?;
    assert!(md.contains("# Legend overlaps axis label"));
    assert!(md.contains("## What happened"));
    assert!(md.contains("Legend text clips on 88mm preset"));
    assert!(driver.state().app.canvas_dialogs.feedback.copied_markdown);

    // 7. Click "Clear" to reset fields, then press Escape to close the modal.
    let clear_pos = widget_center(&driver, "dialog.feedback.reset")?;
    driver.click_at(clear_pos)?;
    assert!(driver.state().app.canvas_dialogs.feedback.title.is_empty());
    assert!(driver.state().app.canvas_dialogs.feedback.summary.is_empty());

    driver.press_key("Escape", KeyModifiers::default())?;
    assert_eq!(driver.state().app.canvas_dialogs.active_dialog, ActiveDialog::None);
    Ok(())
}

#[test]
fn oversized_feedback_truncates_url_cleanly_while_markdown_stays_complete() -> Result<(), String> {
    let url_stub = Arc::new(StubPlatformUrlOpener::new());
    let clip_stub = Arc::new(StubPlatformClipboardAdapter::new());

    let mut app = UiApp::new();
    app.set_url_opener(Arc::clone(&url_stub) as Arc<dyn curvebook_ui_egui::PlatformUrlOpener>);
    app.set_clipboard_adapter(Arc::clone(&clip_stub) as Arc<dyn curvebook_ui_egui::PlatformClipboardAdapter>);
    app.canvas_dialogs.active_dialog = ActiveDialog::Feedback;
    app.canvas_dialogs.feedback.category = FeedbackCategory::NumericalResult;
    app.canvas_dialogs.feedback.title = String::from("4PL EC50 CI mismatch");
    app.canvas_dialogs.feedback.summary = "value=1.234567890 ".repeat(400);
    app.canvas_dialogs.feedback.details = String::from("R drc::drm gives 1.2345");
    app.canvas_dialogs.feedback.extra = String::from("Four-parameter logistic fit");

    let (built, status) = app.open_feedback_on_github()?;
    assert!(status.available);
    assert!(built.truncated);
    assert!(built.omitted_chars > 0);
    assert!(built.url.len() <= MAX_GITHUB_ISSUE_URL_BYTES);
    validate_github_issue_url(&built.url)?;
    assert!(built.url.contains("template=numerical_result.yml"));
    assert!(built.url.contains("title=Numerical%3A%204PL%20EC50%20CI%20mismatch"));

    let (md, clip_status) = app.copy_feedback_markdown()?;
    assert!(clip_status.available);
    assert!(md.len() > MAX_GITHUB_ISSUE_URL_BYTES);
    assert!(md.contains("R drc::drm gives 1.2345"));
    assert_eq!(clip_stub.last_text().as_deref(), Some(md.as_str()));
    Ok(())
}

#[test]
fn control_channel_dispatches_app_feedback_and_respects_gui_only() {
    let mut app = UiApp::new();
    let resp_raw = handle_control_line(&mut app, &json!({ "id": "fb-1", "method": "app.feedback", "params": { "category": "feature" } }).to_string());
    let resp: Value = serde_json::from_str(&resp_raw).expect("valid JSON");
    assert_eq!(resp.get("ok").and_then(Value::as_bool), Some(true), "unexpected response: {resp}");
    assert_eq!(app.canvas_dialogs.active_dialog, ActiveDialog::Feedback);
    assert_eq!(app.canvas_dialogs.feedback.category, FeedbackCategory::FeatureRequest);
    let preview_url = resp.pointer("/result/preview_url").and_then(Value::as_str).expect("preview_url");
    assert!(validate_github_issue_url(preview_url).is_ok());

    let bad_raw = handle_control_line(&mut app, &json!({ "id": "fb-2", "method": "app.feedback", "params": { "category": "bogus" } }).to_string());
    let bad: Value = serde_json::from_str(&bad_raw).expect("valid JSON");
    assert_eq!(bad.get("ok").and_then(Value::as_bool), Some(false));

    app.set_gui_only(true);
    let blocked_raw = handle_control_line(&mut app, &json!({ "id": "fb-3", "method": "app.feedback", "params": {} }).to_string());
    let blocked: Value = serde_json::from_str(&blocked_raw).expect("valid JSON");
    assert_eq!(blocked.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(blocked.pointer("/error_detail/code").and_then(Value::as_str), Some("gui_only_violation"));
}
