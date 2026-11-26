//! Wave 1 security regression suite (`FA-SEC13-01` host path containment for `render.pdf` and `ui.screenshot`).
#![forbid(unsafe_code)]

use std::fs;
use std::path::Path;

use curvebook_doc::ErrorCode;
use curvebook_engine::ExternalPathPolicy;
use curvebook_platform::WorkdirGuard;
use curvebook_ui_egui::{UiApp, dispatch_gui_action};
use serde_json::{Value, json};

use crate::common::{PNG_SIGNATURE, TestRes, assert_control_error_code, call_control, ensure, temp_dir};
use crate::helpers::{call_egui_control, exec, gui};

fn build_sample_app(policy: ExternalPathPolicy) -> Result<UiApp, String> {
    let mut app = UiApp::new();
    app.session_mut().set_path_policy(policy);
    exec(
        &mut app,
        "sheet.createData",
        json!({ "sheet_id": "d1", "title": "Dose", "kind": "xy", "cols": 1, "rows": 3, "auto_graph": true, "graph_sheet_id": "g1" }),
    )?;
    exec(&mut app, "table.setXCells", json!({ "sheet_id": "d1", "start_row": 0, "values": [1.0, 2.0, 3.0] }))?;
    exec(&mut app, "table.paste", json!({ "sheet_id": "d1", "start_row": 0, "start_col": 0, "text": "10\n20\n30" }))?;
    app.sync_views();
    Ok(app)
}

fn assert_all_three_paths_blocked(app: &mut UiApp, target: &str, ctx: &str) -> TestRes {
    let pdf = call_control(app, &json!({ "id": "pe", "method": "render.pdf", "params": { "sheet_id": "g1", "path": target } }))?;
    assert_control_error_code(&pdf, "path_escape", &format!("render.pdf {ctx}"))?;

    let sem = dispatch_gui_action(app, "ui.screenshot", json!({ "target": "workspace", "width_px": 200, "height_px": 140, "path": target }))
        .err()
        .ok_or_else(|| format!("semantic ui.screenshot {ctx} must fail"))?;
    ensure(sem.code == ErrorCode::PathEscape, format!("semantic ui.screenshot {ctx}: expected PathEscape, got {:?}", sem.code))?;

    let egui = call_egui_control(
        app,
        &json!({ "id": "se", "method": "ui.screenshot", "params": { "target": "workspace", "width_px": 200, "height_px": 140, "path": target } }),
    )?;
    assert_control_error_code(&egui, "path_escape", &format!("egui ui.screenshot {ctx}"))
}

#[test]
fn fa_sec13_01_reject_external_blocks_pdf_and_both_screenshot_paths() -> TestRes {
    let root = temp_dir("reject-external")?;
    let mut app = build_sample_app(ExternalPathPolicy::RejectExternal)?;
    let outside = root.path().join("outside.out");
    assert_all_three_paths_blocked(&mut app, &outside.to_string_lossy(), "under RejectExternal")?;
    ensure(!outside.exists(), "must not create file under RejectExternal")?;

    // Omitting `path` still succeeds with inline payloads under RejectExternal.
    let pdf_mem = call_control(&mut app, &json!({ "id": "p2", "method": "render.pdf", "params": { "sheet_id": "g1" } }))?;
    ensure(
        pdf_mem["ok"] == json!(true) && pdf_mem["result"]["data_base64"].as_str().is_some_and(|s| !s.is_empty()),
        format!("in-memory pdf: {pdf_mem}"),
    )?;
    let shot_mem = gui(&mut app, "ui.screenshot", json!({ "target": "workspace", "width_px": 240, "height_px": 160 }))?;
    ensure(shot_mem["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")), format!("in-memory shot: {shot_mem}"))
}

#[test]
fn fa_sec13_01_workdir_blocks_escapes_and_allows_atomic_writes_inside_workdir() -> TestRes {
    let root = temp_dir("workdir")?;
    let (workdir, sibling, outside) = (root.path().join("wd"), root.path().join("wd-sibling"), root.path().join("outside"));
    for dir in [&workdir, &sibling, &outside] {
        fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, workdir.join("escape_link")).map_err(|e| format!("symlink: {e}"))?;

    let guard = WorkdirGuard::new(&workdir).map_err(|e| e.to_string())?;
    let canon = guard.canonical_root().to_path_buf();
    let mut app = build_sample_app(ExternalPathPolicy::from_guard(guard))?;

    let mut escape_targets = vec![
        String::from("../outside/rel.out"),
        String::from("../wd-sibling/prefix_trap.out"),
        outside.join("abs.out").to_string_lossy().into_owned(),
    ];
    #[cfg(unix)]
    escape_targets.push(String::from("escape_link/via_symlink.out"));
    for target in &escape_targets {
        assert_all_three_paths_blocked(&mut app, target, target)?;
    }

    let nul_err = dispatch_gui_action(&mut app, "ui.screenshot", json!({ "path": "bad\0shot.png" })).err().ok_or("NUL path must fail")?;
    ensure(nul_err.code == ErrorCode::BadInput, format!("expected BadInput on NUL path, got {:?}", nul_err.code))?;

    // Valid relative paths inside workdir succeed atomically across all three write paths.
    let pdf_ok =
        call_control(&mut app, &json!({ "id": "pok", "method": "render.pdf", "params": { "sheet_id": "g1", "path": "exports/chart.pdf" } }))?;
    ensure(pdf_ok["ok"] == json!(true), format!("pdf inside workdir: {pdf_ok}"))?;
    ensure(fs::read(canon.join("exports/chart.pdf")).map_err(|e| e.to_string())?.starts_with(b"%PDF-"), "missing %PDF- header")?;

    let sem_ok = gui(&mut app, "ui.screenshot", json!({ "target": "workspace", "width_px": 200, "height_px": 140, "path": "shots/sem.png" }))?;
    ensure(Path::new(sem_ok["path"].as_str().ok_or("missing path")?).starts_with(&canon), "sem.png escaped workdir")?;
    ensure(fs::read(canon.join("shots/sem.png")).map_err(|e| e.to_string())?.starts_with(PNG_SIGNATURE), "sem.png missing PNG signature")?;

    let egui_ok = call_egui_control(
        &mut app,
        &json!({ "id": "sok", "method": "ui.screenshot", "params": { "target": "workspace", "width_px": 200, "height_px": 140, "path": "shots/egui.png" } }),
    )?;
    let egui_ret = egui_ok.pointer("/result/path").and_then(Value::as_str).ok_or("missing result.path")?;
    ensure(egui_ok["ok"] == json!(true) && Path::new(egui_ret).starts_with(&canon), format!("egui.png: {egui_ok}"))?;
    ensure(fs::read(canon.join("shots/egui.png")).map_err(|e| e.to_string())?.starts_with(PNG_SIGNATURE), "egui.png missing PNG signature")?;

    for dir in [&outside, &sibling] {
        ensure(dir.read_dir().map_err(|e| e.to_string())?.next().is_none(), "escaping write left files in outside/sibling dir")?;
    }
    Ok(())
}
