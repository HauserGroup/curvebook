//! Suite-local helper utilities for `ui_audit_suite`.
#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, MultipleVariablesTable, ProjectDocument, ResultCell, ResultTab, ResultTabKind, SheetId,
    SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::{CommandOutcome, Session};
use curvebook_ui_egui::UiApp;
use curvebook_ui_egui::control_io::{dispatch_gui_action, dispatch_ui_command};
use curvebook_ui_egui::egui_shell::driver::{EguiHumanPathDriver, TextBleedPolicy, build_driver_font_definitions};
use curvebook_ui_egui::egui_shell::{FrameInspectMeta, Tokens};
use curvebook_ui_egui::interaction::ViewportMetrics;
use egui::{Pos2, Rect, Vec2};
use serde_json::Value;

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

pub fn seed_xy_app(xs: &[f64], ys: &[f64]) -> (UiApp, SheetId) {
    let mut tbl = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
    tbl.x_title = String::from("Conc");
    tbl.data_sets[0].title = String::from("Response");
    for (r, (&x, &y)) in xs.iter().zip(ys.iter()).enumerate() {
        tbl.x_cells[r][0] = Cell::number(x);
        tbl.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let sid = SheetId::new("xy-1");
    let mut doc = ProjectDocument::new("XY Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "Dose Response", TableArchetype::Xy(tbl)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

pub fn seed_column_app(cols: &[(&str, &[f64])], hint: ColumnDesignHint) -> (UiApp, SheetId) {
    let rows = cols.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut tbl = ColumnTable::new(hint, cols.len(), rows).unwrap();
    for (ci, (title, vals)) in cols.iter().enumerate() {
        tbl.data_sets[ci].title = (*title).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            tbl.data_sets[ci].cells[r][0] = Cell::number(v);
        }
    }
    let sid = SheetId::new("col-1");
    let mut doc = ProjectDocument::new("Column Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "Groups", TableArchetype::Column(tbl)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

pub fn seed_mv_app(col_count: usize, rows: usize) -> (UiApp, SheetId) {
    let mut mv = MultipleVariablesTable::new(col_count, rows).unwrap();
    for (c, vc) in mv.variables.iter_mut().enumerate() {
        vc.title = format!("V{}", c + 1);
        if c == 0 {
            vc.role = VariableRole::Categorical;
            for r in 0..rows {
                vc.cells[r] = Cell::text(if r % 2 == 0 { "Ctrl" } else { "Tx" });
            }
        } else {
            vc.role = VariableRole::Continuous;
            for r in 0..rows {
                vc.cells[r] = Cell::number(((r + 1) * (c + 1)) as f64);
            }
        }
    }
    let sid = SheetId::new("mv-1");
    let mut doc = ProjectDocument::new("MV Test").unwrap();
    doc.data_sheets.push(DataSheet::new(sid.clone(), "MV Sheet", TableArchetype::MultipleVariables(mv)).unwrap());
    doc.active_sheet_id = Some(sid.clone());
    let mut app = UiApp::from_session(Session::from_document(doc));
    app.sync_views();
    (app, sid)
}

pub fn make_commit_driver(app: UiApp) -> EguiHumanPathDriver<'static, ShellTestState> {
    let tokens = Tokens::from_theme_spec(&app.theme, &app.locale);
    make_shell_driver_with_state(ShellTestState::new(app).with_commit_all().with_merge_passes(), ViewportMetrics::new(1280.0, 800.0, 1.0), tokens)
}

pub fn run_egui_pass_with_ctx<R>(
    ctx: &egui::Context,
    size: Vec2,
    input: egui::RawInput,
    central: bool,
    mut draw: impl FnMut(&egui::Context, &mut egui::Ui) -> R,
) -> Result<R, String> {
    let mut raw = input;
    raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
    let mut out = None;
    let mut full = ctx.run_ui(raw, |ui| {
        if central {
            egui::CentralPanel::default().show(ui, |ui| {
                let c = ui.ctx().clone();
                out = Some(draw(&c, ui));
            });
        } else {
            let c = ui.ctx().clone();
            out = Some(draw(&c, ui));
        }
    });
    full.textures_delta.clear();
    out.ok_or_else(|| String::from("egui pass did not execute"))
}

pub fn click_input(pos: Pos2) -> egui::RawInput {
    egui::RawInput {
        events: vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        ],
        ..egui::RawInput::default()
    }
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
