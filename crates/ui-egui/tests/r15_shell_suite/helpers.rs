//! Suite-local helper utilities for `ui_audit_suite`.
#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{ProjectDocument, ResultCell, ResultTab, ResultTabKind, SheetId};
use curvebook_engine::CommandOutcome;
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::control_io::{dispatch_gui_action, dispatch_ui_command};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::{Map, Value};

use crate::common::{ShellTestState, TestRes, ensure, make_shell_driver_with_state, sync_app_to_driver};

pub fn exec(app: &mut UiApp, cmd: &str, params: Value) -> Result<CommandOutcome, String> {
    app.execute_command(cmd, params).map_err(|e| format!("{cmd}: {e}"))
}

pub fn gui(app: &mut UiApp, action: &str, params: Value) -> Result<Value, String> {
    dispatch_gui_action(app, action, params).map_err(|e| format!("{action}: {e}"))
}

pub fn ui_cmd(app: &mut UiApp, cmd: &str, params: Value) -> Result<Option<Value>, String> {
    dispatch_ui_command(app, cmd, &params)
}

pub fn call_egui_control(app: &mut UiApp, req: &Value) -> Result<Value, String> {
    let line = serde_json::to_string(req).map_err(|e| format!("serialize egui control request: {e}"))?;
    let raw = app.dispatch_control_line(&line);
    serde_json::from_str(&raw).map_err(|e| format!("parse egui control response `{raw}`: {e}"))
}

pub fn patch_map(val: Value) -> Map<String, Value> {
    val.as_object().cloned().unwrap_or_default()
}

pub fn make_human_driver(doc: ProjectDocument) -> EguiHumanPathDriver<'static, ShellTestState> {
    make_shell_driver_with_state(ShellTestState::from_doc_human(doc), ViewportMetrics::default(), Tokens::default())
}

pub fn make_merge_driver(app: UiApp, w: f64, h: f64) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_merge_passes(), ViewportMetrics::new(w, h, 1.0), tokens)
}

pub fn run_panel_driver<S, F>(state: S, w: f64, h: f64, render: F) -> EguiHumanPathDriver<'static, S>
where
    S: 'static,
    F: FnMut(&mut egui::Ui, &mut FrameInspectMeta, &mut S) + 'static,
{
    let mut driver = EguiHumanPathDriver::with_tokens_and_fonts(
        state,
        ViewportMetrics::new(w, h, 1.0),
        Tokens::default(),
        build_driver_font_definitions(),
        render,
    );
    driver.set_bleed_policy(TextBleedPolicy::AllowVerticalScrollClip);
    driver.run_settled();
    driver
}

pub fn has_widget<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> bool {
    driver.frame_meta().find_by_widget_id(wid).is_some()
}

pub fn widget_enabled<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<bool, String> {
    Ok(driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?.enabled)
}

pub fn widget_label<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<String, String> {
    Ok(driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?.label.clone())
}

pub fn widget_value<S>(driver: &EguiHumanPathDriver<'_, S>, wid: &str) -> Result<Option<Value>, String> {
    Ok(driver.frame_meta().find_by_widget_id(wid).ok_or_else(|| format!("missing `{wid}`"))?.value.clone())
}

pub fn open_edit_params_driver(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    ensure(driver.state_mut().app.canvas_dialogs.open_edit_parameters(&doc, sheet_id), format!("open_edit_parameters({sheet_id}) failed"))?;
    sync_app_to_driver(driver);
    Ok(())
}

pub fn open_format_graph_driver(driver: &mut EguiHumanPathDriver<'_, ShellTestState>, sheet_id: &SheetId) -> TestRes {
    let doc = driver.state().app.document().clone();
    ensure(driver.state_mut().app.canvas_dialogs.open_format_graph(&doc, sheet_id), format!("open_format_graph({sheet_id}) failed"))?;
    sync_app_to_driver(driver);
    Ok(())
}

pub fn make_result_tab(id: &str, title: &str, kind: ResultTabKind, headers: &[&str], row_labels: &[&str], rows: Vec<Vec<ResultCell>>) -> ResultTab {
    ResultTab {
        id: id.to_owned(),
        title: title.to_owned(),
        kind,
        column_headers: headers.iter().map(|s| (*s).to_owned()).collect(),
        row_labels: row_labels.iter().map(|s| (*s).to_owned()).collect(),
        rows,
        footnotes: Vec::new(),
    }
}
