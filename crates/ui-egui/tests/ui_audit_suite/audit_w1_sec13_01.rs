//! Wave 1 security regression suite (`FA-SEC13-01` host path containment for `render.pdf` and `ui.screenshot`).
//!
//! Verifies:
//! - `render.pdf`, semantic `ui.screenshot`, and `egui`-backend `ui.screenshot` enforce
//!   [`ExternalPathPolicy::RejectExternal`] and [`ExternalPathPolicy::Workdir`] containment
//!   (including relative traversal, sibling prefix confusion, and symlink escapes) before writing
//!   to the host filesystem.
#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use curvebook_doc::ErrorCode;
use curvebook_engine::ExternalPathPolicy;
use curvebook_platform::WorkdirGuard;
use curvebook_ui_egui::{UiApp, dispatch_gui_action, handle_control_line};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

use crate::common::temp_dir;

fn build_sample_app(policy: ExternalPathPolicy) -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.session_mut().set_path_policy(policy);
    app.execute_command(
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": true, "graph_sheet_id": "g1" }),
    )
    .map_err(|err| format!("create sample sheet: {err}"))?;
    app.execute_command("table.setXCells", json!({ "sheet_id": "d1", "start_row": 0, "values": [1.0, 2.0, 3.0] }))
        .map_err(|err| format!("seed X: {err}"))?;
    app.execute_command("table.paste", json!({ "sheet_id": "d1", "start_row": 0, "start_col": 0, "text": "10\n20\n30" }))
        .map_err(|err| format!("seed Y: {err}"))?;
    app.sync_views();
    Ok(app)
}

fn call_control(app: &mut UiApp, req: &Value) -> Result<Value, String> {
    let line = serde_json::to_string(req).map_err(|err| format!("serialize control request: {err}"))?;
    let resp_line = handle_control_line(app, &line);
    serde_json::from_str(&resp_line).map_err(|err| format!("parse control response `{resp_line}`: {err}"))
}

fn call_egui_control(app: &mut UiApp, req: &Value) -> Result<Value, String> {
    let line = serde_json::to_string(req).map_err(|err| format!("serialize egui control request: {err}"))?;
    let resp_line = app.dispatch_control_line(&line);
    serde_json::from_str(&resp_line).map_err(|err| format!("parse egui control response `{resp_line}`: {err}"))
}

fn assert_control_error_code(resp: &Value, expected_code: &str, context: &str) -> TestRes {
    if resp["ok"] != json!(false) {
        return Err(format!("{context}: expected failure, got {resp}"));
    }
    let code = resp.pointer("/error_detail/code").and_then(Value::as_str);
    if code != Some(expected_code) {
        return Err(format!("{context}: expected error_detail.code `{expected_code}`, got {resp}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_01_reject_external_blocks_pdf_and_both_screenshot_paths() -> TestRes {
    let root = temp_dir("reject-external")?;
    let outside_pdf = root.path().join("outside.pdf");
    let outside_sem_png = root.path().join("outside_sem.png");
    let outside_egui_png = root.path().join("outside_egui.png");

    let mut app = build_sample_app(ExternalPathPolicy::RejectExternal)?;

    let pdf_resp = call_control(
        &mut app,
        &json!({ "id": "p1", "method": "render.pdf", "params": { "sheet_id": "g1", "path": outside_pdf.to_string_lossy() } }),
    )?;
    assert_control_error_code(&pdf_resp, "path_escape", "render.pdf under RejectExternal")?;
    if outside_pdf.exists() {
        return Err(format!("render.pdf must not write `{}` under RejectExternal", outside_pdf.display()));
    }

    let sem_err = dispatch_gui_action(
        &mut app,
        "ui.screenshot",
        json!({ "target": "workspace", "width_px": 240, "height_px": 160, "path": outside_sem_png.to_string_lossy() }),
    )
    .err()
    .ok_or_else(|| String::from("semantic ui.screenshot with path must fail under RejectExternal"))?;
    if sem_err.code != ErrorCode::PathEscape {
        return Err(format!("expected PathEscape on semantic ui.screenshot, got {:?}", sem_err.code));
    }
    if outside_sem_png.exists() {
        return Err(String::from("semantic ui.screenshot wrote file under RejectExternal"));
    }

    let egui_resp = call_egui_control(
        &mut app,
        &json!({ "id": "s-egui", "method": "ui.screenshot", "params": { "target": "workspace", "width_px": 240, "height_px": 160, "path": outside_egui_png.to_string_lossy() } }),
    )?;
    assert_control_error_code(&egui_resp, "path_escape", "egui ui.screenshot under RejectExternal")?;
    if outside_egui_png.exists() {
        return Err(String::from("egui ui.screenshot wrote file under RejectExternal"));
    }

    // Omitting `path` still succeeds with inline payloads under RejectExternal.
    let pdf_mem = call_control(&mut app, &json!({ "id": "p2", "method": "render.pdf", "params": { "sheet_id": "g1" } }))?;
    if pdf_mem["ok"] != json!(true) || pdf_mem["result"]["data_base64"].as_str().is_none_or(str::is_empty) {
        return Err(format!("in-memory render.pdf failed under RejectExternal: {pdf_mem}"));
    }
    let shot_mem = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "target": "workspace", "width_px": 240, "height_px": 160 }))
        .map_err(|err| format!("in-memory ui.screenshot failed under RejectExternal: {err}"))?;
    if shot_mem["data_base64"].as_str().is_none_or(|b| !b.starts_with("iVBORw0K")) {
        return Err(format!("unexpected in-memory ui.screenshot payload: {shot_mem}"));
    }
    Ok(())
}

#[test]
fn fa_sec13_01_workdir_blocks_escapes_and_allows_atomic_writes_inside_workdir() -> TestRes {
    let root = temp_dir("workdir")?;
    let workdir = root.path().join("wd");
    let sibling = root.path().join("wd-sibling");
    let outside = root.path().join("outside");
    fs::create_dir_all(&workdir).map_err(|err| format!("mkdir workdir: {err}"))?;
    fs::create_dir_all(&sibling).map_err(|err| format!("mkdir sibling: {err}"))?;
    fs::create_dir_all(&outside).map_err(|err| format!("mkdir outside: {err}"))?;

    #[cfg(unix)]
    {
        let link = workdir.join("escape_link");
        std::os::unix::fs::symlink(&outside, &link).map_err(|err| format!("symlink escape_link: {err}"))?;
    }

    let guard = WorkdirGuard::new(&workdir).map_err(|err| format!("WorkdirGuard::new: {err}"))?;
    let canon_workdir = guard.canonical_root().to_path_buf();
    let mut app = build_sample_app(ExternalPathPolicy::from_guard(guard))?;

    let outside_abs_pdf = outside.join("abs.pdf");
    let outside_abs_png = outside.join("abs.png");
    let mut escape_targets =
        vec![String::from("../outside/rel.out"), String::from("../wd-sibling/prefix_trap.out"), outside_abs_pdf.to_string_lossy().into_owned()];
    #[cfg(unix)]
    escape_targets.push(String::from("escape_link/via_symlink.out"));

    for target in &escape_targets {
        let pdf_err = call_control(&mut app, &json!({ "id": "pe", "method": "render.pdf", "params": { "sheet_id": "g1", "path": target } }))?;
        assert_control_error_code(&pdf_err, "path_escape", &format!("render.pdf `{target}`"))?;

        let sem_err =
            dispatch_gui_action(&mut app, "ui.screenshot", json!({ "target": "workspace", "width_px": 200, "height_px": 140, "path": target }))
                .err()
                .ok_or_else(|| format!("dispatch_gui_action ui.screenshot `{target}` should fail"))?;
        if sem_err.code != ErrorCode::PathEscape {
            return Err(format!("dispatch_gui_action ui.screenshot `{target}` expected PathEscape, got {:?}", sem_err.code));
        }

        let egui_err = call_egui_control(
            &mut app,
            &json!({ "id": "se", "method": "ui.screenshot", "params": { "target": "workspace", "width_px": 200, "height_px": 140, "path": target } }),
        )?;
        assert_control_error_code(&egui_err, "path_escape", &format!("dispatch_control_line ui.screenshot `{target}`"))?;
    }

    let outside_abs_png_err = dispatch_gui_action(
        &mut app,
        "ui.screenshot",
        json!({ "target": "workspace", "width_px": 200, "height_px": 140, "path": outside_abs_png.to_string_lossy() }),
    )
    .err()
    .ok_or_else(|| String::from("outside abs png should fail"))?;
    if outside_abs_png_err.code != ErrorCode::PathEscape {
        return Err(format!("expected PathEscape, got {:?}", outside_abs_png_err.code));
    }

    let nul_err =
        dispatch_gui_action(&mut app, "ui.screenshot", json!({ "path": "bad\0shot.png" })).err().ok_or_else(|| String::from("NUL path must fail"))?;
    if nul_err.code != ErrorCode::BadInput {
        return Err(format!("expected BadInput on NUL path, got {:?}", nul_err.code));
    }

    // Valid relative paths strictly inside workdir succeed atomically across all three write paths.
    let pdf_ok =
        call_control(&mut app, &json!({ "id": "pok", "method": "render.pdf", "params": { "sheet_id": "g1", "path": "exports/chart.pdf" } }))?;
    if pdf_ok["ok"] != json!(true) {
        return Err(format!("render.pdf inside workdir failed: {pdf_ok}"));
    }
    let pdf_bytes = fs::read(canon_workdir.join("exports/chart.pdf")).map_err(|err| format!("read chart.pdf: {err}"))?;
    if !pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("written chart.pdf missing %PDF- header"));
    }

    let sem_ok =
        dispatch_gui_action(&mut app, "ui.screenshot", json!({ "target": "workspace", "width_px": 200, "height_px": 140, "path": "shots/sem.png" }))
            .map_err(|err| format!("dispatch_gui_action ui.screenshot inside workdir failed: {err}"))?;
    let sem_ret = sem_ok["path"].as_str().ok_or("missing path in gui screenshot response")?;
    if !Path::new(sem_ret).starts_with(&canon_workdir) {
        return Err(format!("gui screenshot path `{sem_ret}` escaped `{}`", canon_workdir.display()));
    }
    let sem_bytes = fs::read(canon_workdir.join("shots/sem.png")).map_err(|err| format!("read sem.png: {err}"))?;
    if !sem_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("written sem.png missing PNG signature"));
    }

    let egui_ok = call_egui_control(
        &mut app,
        &json!({ "id": "sok", "method": "ui.screenshot", "params": { "target": "workspace", "width_px": 200, "height_px": 140, "path": "shots/egui.png" } }),
    )?;
    if egui_ok["ok"] != json!(true) {
        return Err(format!("dispatch_control_line ui.screenshot inside workdir failed: {egui_ok}"));
    }
    let egui_ret = egui_ok.pointer("/result/path").and_then(Value::as_str).ok_or("missing result.path in egui screenshot response")?;
    if !Path::new(egui_ret).starts_with(&canon_workdir) {
        return Err(format!("egui screenshot path `{egui_ret}` escaped `{}`", canon_workdir.display()));
    }
    let egui_bytes = fs::read(canon_workdir.join("shots/egui.png")).map_err(|err| format!("read egui.png: {err}"))?;
    if !egui_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("written egui.png missing PNG signature"));
    }

    if outside.read_dir().map_err(|err| format!("readdir outside: {err}"))?.next().is_some()
        || sibling.read_dir().map_err(|err| format!("readdir sibling: {err}"))?.next().is_some()
    {
        return Err(String::from("escaping write left files in outside or sibling directory"));
    }
    Ok(())
}
