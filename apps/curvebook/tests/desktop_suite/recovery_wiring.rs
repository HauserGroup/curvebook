#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::process::Command;
use std::time::Duration;

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime, DesktopWindowId};
use curvebook::gui_window::CurveBookNativeApp;
use curvebook_doc::{CellCoord, SheetId};
use curvebook_engine::Session;
use curvebook_format::{DirectoryJournalBackend, JOURNAL_FILE_NAME, JournalBackend, LOCKFILE_NAME, SNAPSHOT_FILE_NAME};
use curvebook_ui_egui::ControlAuth;
use curvebook_ui_egui::interaction::ViewportMetrics;
use eframe::egui;
use serde_json::{Value, json};

use crate::common::{
    ChildGuard, NativeHarness, TempDirGuard, now_epoch_ms, reserve_ephemeral_port, send_control_lines, send_json_line, tcp_connect_loopback,
    wait_for_token_file,
};

fn open_recovered_window(
    recovery_dir: &std::path::Path,
    owner: &str,
    viewport: ViewportMetrics,
) -> Result<(DesktopHostRuntime, DesktopWindowId), Box<dyn std::error::Error>> {
    let (session, journal, report) = Session::open_dir_with_recovery(recovery_dir, owner, 2_000, 1_000)?;
    let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.to_path_buf()), ..DesktopHostConfig::default() });
    let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.to_path_buf(), journal, Some(report))?;
    Ok((rt, win_id))
}

#[test]
fn r1_s11_01_default_single_instance_control_stream_journals_and_recovers_after_crash() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-stream")?;
    let recovery_dir = dir.child("recovery");
    let auth = ControlAuth::from_token("test-recovery-wiring-secret-token")?;
    let viewport = ViewportMetrics::new(800.0, 600.0, 1.0);

    {
        let (mut rt, win_id) = open_recovered_window(&recovery_dir, "single-inst-1", viewport)?;
        let win = rt.window_mut(win_id)?;
        assert!(win.has_recovery_journal());

        let reqs = [
            json!({ "id": "r1", "method": "engine.execute", "token": auth.as_str(), "params": { "command_id": "sheet.createData", "params": { "sheet_id": "ctrl-ds", "title": "Control WAL", "kind": "column", "cols": 2, "rows": 4 } } }),
            json!({ "id": "r2", "method": "command.run", "token": auth.as_str(), "command": "table.setCell", "params": { "sheet_id": "ctrl-ds", "row": 0, "col": 0, "value": 12.5 } }),
            json!({ "id": "r3", "method": "command.run", "token": auth.as_str(), "command": "table.setCell", "params": { "sheet_id": "ctrl-ds", "row": 0, "col": 0, "value": 99.0 } }),
            json!({ "id": "r4", "method": "command.batch", "token": auth.as_str(), "params": { "atomic": true, "commands": [
                { "command": "table.setCell", "params": { "sheet_id": "ctrl-ds", "row": 1, "col": 0, "value": 5.0 } },
                { "command": "no.such.command", "params": {} }
            ] } }),
            json!({ "id": "r5", "method": "command.run", "token": auth.as_str(), "command": "edit.undo", "params": {} }),
            json!({ "id": "r6", "method": "table.setCell", "token": auth.as_str(), "params": { "sheet_id": "ctrl-ds", "row": 1, "col": 0, "value": 34.5 } }),
        ];
        let (lines, quit) = send_control_lines(win, &auth, &reqs)?;
        assert!(!quit && lines.len() == 6);
        assert_eq!(
            lines.iter().map(|v| v["ok"].as_bool()).collect::<Vec<_>>(),
            vec![Some(true), Some(true), Some(true), Some(false), Some(true), Some(true)]
        );
        assert!(recovery_dir.join(SNAPSHOT_FILE_NAME).is_file() && recovery_dir.join(JOURNAL_FILE_NAME).is_file());
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_win_id = rec_rt.recover_window_from_dir(recovery_dir.clone(), viewport, 5_000)?;
    {
        let rec_win = rec_rt.window(rec_win_id)?;
        assert!(rec_win.recovered_from_crash());
        let ds = rec_win.app().session().document().data_sheet(&SheetId::new("ctrl-ds")).ok_or("missing recovered ctrl-ds")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(12.5));
        assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(34.5));
    }
    rec_rt.close_all_force()?;
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists());
    assert!(fs::read(recovery_dir.join(JOURNAL_FILE_NAME)).unwrap_or_default().is_empty());
    Ok(())
}

#[test]
fn r1_s11_01_and_s06_03_binary_default_launch_and_control_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let dir = TempDirGuard::new("s11-bin")?;
    let data_dir = dir.child("curvebook-data");
    let recovery_dir = data_dir.join("recovery");

    // 1. Clean default launch releases lockfile.
    let clean_out = Command::new(bin).env("HOME", dir.path()).args(["--headless-once", "--portable-dir"]).arg(dir.path()).output()?;
    assert!(clean_out.status.success() && !recovery_dir.join(LOCKFILE_NAME).exists());

    // 2. R1-S06-03: interior corrupt WAL line heals on launch without aborting.
    {
        let (mut s, mut j, _) = Session::open_dir_with_recovery(&recovery_dir, "precrash-owner", 1_000, 1)?;
        s.execute_and_record(
            &mut j,
            "sheet.createData",
            json!({ "sheet_id": "surviving-sheet", "title": "Intact Pre-Corrupt", "kind": "column", "cols": 1, "rows": 2 }),
        )?;
        let mut f = fs::OpenOptions::new().append(true).open(recovery_dir.join(JOURNAL_FILE_NAME))?;
        f.write_all(b"{\"format_version\":1,\"seq\":2,\"torn_interior")?;
        f.sync_data()?;
        s.execute_and_record(&mut j, "table.setCell", json!({ "sheet_id": "surviving-sheet", "row": 0, "col": 0, "value": 7.0 }))?;
        drop(j.abandon());
    }
    let heal_out = Command::new(bin).env("HOME", dir.path()).args(["--headless-once", "--portable-dir"]).arg(dir.path()).output()?;
    assert!(heal_out.status.success() && String::from_utf8_lossy(&heal_out.stderr).contains("curvebook recovery warning"));

    let (mut reopened, heal_journal, _) = Session::open_dir_with_recovery(&recovery_dir, "verify-heal", 2_000, now_epoch_ms() + 10)?;
    let healed_ds = reopened.document().data_sheet(&SheetId::new("surviving-sheet")).ok_or("missing surviving-sheet")?;
    assert_eq!(healed_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(7.0));
    let _ = reopened.close_journal_cleanly(heal_journal)?;

    // 3. Default single-instance --control SIGKILL crash recovery over loopback TCP.
    let Some(port) = reserve_ephemeral_port()? else {
        return Ok(());
    };
    let token_path = data_dir.join(format!("control-{port}.token"));
    let mut guard = ChildGuard(
        Command::new(bin).env("HOME", dir.path()).args(["--portable-dir"]).arg(dir.path()).args(["--control", &port.to_string()]).spawn()?,
    );
    let token = wait_for_token_file(&mut guard.0, &token_path, Duration::from_secs(10))?;

    let Some((mut r, mut w)) = tcp_connect_loopback(SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_secs(2))? else {
        return Ok(());
    };
    for req in [
        json!({ "id": "tcp-1", "method": "engine.execute", "token": token, "params": { "command_id": "sheet.createData", "params": { "sheet_id": "tcp-sheet", "title": "TCP Recovered", "kind": "column", "cols": 1, "rows": 2 } } }),
        json!({ "id": "tcp-2", "method": "table.setCell", "token": token, "params": { "sheet_id": "tcp-sheet", "row": 0, "col": 0, "value": 42.25 } }),
    ] {
        assert_eq!(send_json_line(&mut w, &mut r, &req)?["ok"], json!(true));
    }
    drop((r, w));

    assert!(recovery_dir.join(JOURNAL_FILE_NAME).is_file());
    let _ = guard.0.kill();
    let _ = guard.0.wait();

    let (mut rec_tcp, rec_tcp_journal, tcp_report) =
        Session::open_dir_with_recovery(&recovery_dir, "post-sigkill-relaunch", 5_000, now_epoch_ms().saturating_add(400_000))?;
    assert!(tcp_report.recovered_from_stale_lock.is_some() && tcp_report.recovered_commands.len() == 2);
    let tcp_ds = rec_tcp.document().data_sheet(&SheetId::new("tcp-sheet")).ok_or("missing recovered tcp-sheet")?;
    assert_eq!(tcp_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(42.25));
    let _ = rec_tcp.close_journal_cleanly(rec_tcp_journal)?;
    Ok(())
}

#[test]
fn r1_s11_03_lease_renewal_new_instance_isolation_and_multiwindow_close_all_force() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-03-renewal-and-new-instance")?;
    let recovery_root = dir.child("recovery-root");
    let viewport = ViewportMetrics::new(800.0, 520.0, 2.0);

    let mut primary_rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_root.clone()), ..DesktopHostConfig::default() });
    let w1 = primary_rt.open_window(None, viewport)?;
    let initial_acquired = DirectoryJournalBackend::open_dir(&recovery_root)?.read_lease()?.ok_or("missing primary lease")?.acquired_epoch_ms;
    std::thread::sleep(Duration::from_millis(15));
    let _ = primary_rt
        .window_mut(w1)?
        .execute_and_journal("sheet.createData", json!({ "sheet_id": "renew-ds", "title": "Renew", "kind": "column", "cols": 1, "rows": 1 }))?;
    let renewed = DirectoryJournalBackend::open_dir(&recovery_root)?.read_lease()?.ok_or("missing renewed lease")?.acquired_epoch_ms;
    assert!(renewed >= initial_acquired + 10);

    let mut isolated_rt =
        DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_root.clone()), new_instance: true, ..DesktopHostConfig::default() });
    let iso_w = isolated_rt.open_window(None, viewport)?;
    let iso_dir = isolated_rt.window(iso_w)?.recovery_dir().ok_or("missing isolated dir")?.to_path_buf();
    assert!(iso_dir != recovery_root && iso_dir.join(LOCKFILE_NAME).is_file() && recovery_root.join(LOCKFILE_NAME).is_file());
    isolated_rt.close_all_force()?;
    assert!(!iso_dir.join(LOCKFILE_NAME).exists() && recovery_root.join(LOCKFILE_NAME).is_file());

    let w2 = primary_rt.open_window(None, viewport)?;
    let w2_dir = primary_rt.window(w2)?.recovery_dir().ok_or("missing w2 dir")?.to_path_buf();
    assert!(w2_dir.join(LOCKFILE_NAME).is_file());
    fs::write(
        recovery_root.join(LOCKFILE_NAME),
        serde_json::to_vec(&curvebook_format::JournalLockLease::new("external-owner", now_epoch_ms(), 300_000)?)?,
    )?;
    let close_err = primary_rt.close_all_force().expect_err("stolen lock on w1 must report error");
    assert!(close_err.contains("external-owner") && !w2_dir.join(LOCKFILE_NAME).exists() && recovery_root.join(LOCKFILE_NAME).is_file());
    Ok(())
}

#[test]
fn r1_s11_02_install_window_error_releases_recovery_lock() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-02-install-rollback")?;
    let recovery_root = dir.child("recovery-root");
    let mut bad_rt = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: Some(recovery_root.clone()),
        theme_override: Some(String::from("nonexistent-theme")),
        ..DesktopHostConfig::default()
    });
    assert!(bad_rt.open_window(None, ViewportMetrics::new(800.0, 520.0, 2.0)).is_err());
    assert!(!recovery_root.join(LOCKFILE_NAME).exists());
    Ok(())
}

#[test]
fn r1_s11_05_and_06_binary_corrupt_prefs_config_dir_and_no_prefs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-05-06-binary")?;
    let cfg_dir = dir.child("cfg");
    fs::create_dir_all(&cfg_dir)?;
    fs::write(cfg_dir.join("shortcuts.json"), "{not json")?;
    fs::write(cfg_dir.join("preferences.json"), "{also not json")?;

    let bin = env!("CARGO_BIN_EXE_curvebook");
    assert!(Command::new(bin).env("CURVEBOOK_CONFIG_DIR", &cfg_dir).args(["--headless-once"]).output()?.status.success());

    let script = dir.child("script.jsonl");
    fs::write(&script, "{\"WindowFocused\":{\"window_id\":1}}\n")?;
    let no_prefs = Command::new(bin)
        .env("CURVEBOOK_CONFIG_DIR", &cfg_dir)
        .env("CURVEBOOK_NO_PREFS", "1")
        .args(["--event-script", &script.display().to_string()])
        .output()?;
    assert!(no_prefs.status.success() && !cfg_dir.join("recovery").exists());
    Ok(())
}

#[test]
fn audit_2_mixed_batch_undo_and_create_recovers_exact_sheet_count() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit2-mixed-batch")?;
    let recovery_dir = dir.child("recovery");
    let auth = ControlAuth::from_token("audit2-mixed-batch-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (mut rt, win_id) = open_recovered_window(&recovery_dir, "audit2-1", viewport)?;
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "kind": "column", "cols": 1, "rows": 2 }))?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t2", "title": "Second", "kind": "column", "cols": 1, "rows": 2 }))?;
        let (resps, quit) = send_control_lines(
            win,
            &auth,
            &[json!({
                "id": "b1", "method": "command.batch", "token": auth.as_str(),
                "params": { "atomic": true, "commands": [
                    { "command": "edit.undo", "params": {} },
                    { "command": "sheet.createData", "params": { "sheet_id": "t3", "title": "Third", "kind": "column", "cols": 1, "rows": 2 } }
                ] }
            })],
        )?;
        assert!(!quit && resps[0]["ok"] == json!(true));
        let live_ids: Vec<&str> = win.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(live_ids, vec!["t1", "t3"]);
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_ids: Vec<&str> = rec_rt.window(rec_id)?.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(rec_ids, vec!["t1", "t3"]);
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_5_gui_edit_after_64_undo_eviction_persists_through_crash_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit5-64-undo")?;
    let recovery_dir = dir.child("recovery");
    let viewport = ViewportMetrics::new(1100.0, 720.0, 1.0);

    {
        let (mut rt, win_id) = open_recovered_window(&recovery_dir, "audit5-1", viewport)?;
        {
            let win = rt.window_mut(win_id)?;
            win.execute_and_journal("sheet.createData", json!({ "sheet_id": "s64", "title": "FullUndo", "kind": "column", "cols": 2, "rows": 4 }))?;
            for i in 0..64 {
                win.execute_and_journal("table.setCell", json!({ "sheet_id": "s64", "row": (i % 3) + 1, "col": 0, "value": f64::from(i + 1) }))?;
            }
            assert_eq!((win.app().session().undo_depth(), win.app().session().redo_depth(), win.app().session().is_dirty()), (64, 0, true));
        }

        let mut h = NativeHarness::new(CurveBookNativeApp::new(rt, win_id, None));
        h.settle(2);
        let cell_pos = h.app.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        h.double_click_pos(cell_pos);
        let _ = h.step_events(vec![
            egui::Event::Text(String::from("777.25")),
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        ]);
        let _ = h.step_events(Vec::new());

        let win = h.app.runtime().window(win_id)?;
        assert_eq!(win.app().session().undo_depth(), 64);
        assert_eq!(
            win.app()
                .session()
                .document()
                .data_sheet(&SheetId::new("s64"))
                .and_then(|d| d.cell(CellCoord::new(0, 0, 0)))
                .and_then(|c| c.active_number()),
            Some(777.25)
        );
        drop(h);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_ds = rec_rt.window(rec_id)?.app().session().document().data_sheet(&SheetId::new("s64")).ok_or("missing recovered s64")?;
    assert_eq!(rec_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(777.25));
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_118_mixed_batch_gui_menu_then_create_recovers_exact_sheets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit118-mixed-batch")?;
    let recovery_dir = dir.child("recovery");
    let auth = ControlAuth::from_token("audit118-mixed-batch-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (mut rt, win_id) = open_recovered_window(&recovery_dir, "audit118-1", viewport)?;
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "kind": "column", "cols": 1, "rows": 2 }))?;
        let (resps, quit) = send_control_lines(
            win,
            &auth,
            &[json!({
                "id": "b118-1", "method": "command.batch", "token": auth.as_str(),
                "params": { "atomic": false, "commands": [
                    { "command": "ui.invoke_menu", "params": { "command_id": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "t1", "kind": "column" } } },
                    { "command": "sheet.createData", "params": { "sheet_id": "t2", "title": "Second", "kind": "column", "cols": 1, "rows": 2 } }
                ] }
            })],
        )?;
        assert!(!quit && resps[0]["ok"] == json!(true));
        assert_eq!(win.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2"]);
        assert_eq!(win.app().session().document().graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g1"]);
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_doc = rec_rt.window(rec_id)?.app().session().document();
    assert_eq!(rec_doc.data_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2"]);
    assert_eq!(rec_doc.graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g1"]);
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_118_mixed_batch_save_then_gui_mutation_keeps_dirty_and_recovers() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit118-save-dirty")?;
    let recovery_dir = dir.child("recovery");
    let save_path = dir.child("saved-before-gui.curvebook");
    let auth = ControlAuth::from_token("audit118-save-dirty-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (mut rt, win_id) = open_recovered_window(&recovery_dir, "audit118-save-1", viewport)?;
        let win = rt.window_mut(win_id)?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "kind": "column", "cols": 1, "rows": 2 }))?;

        let raw = format!(
            "{}\n",
            json!({
                "id": "b118-save", "method": "command.batch",
                "params": { "atomic": false, "commands": [
                    { "command": "project.save", "params": { "path": save_path.display().to_string() } },
                    { "command": "ui.invoke_menu", "params": { "command_id": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "t1", "kind": "column" } } }
                ] }
            })
        );
        let frame_out = win.dispatch_live_control_frame_with_hook(raw.as_bytes(), &auth, true, |_, _| Ok(()))?;
        let resp: Value = serde_json::from_slice(&frame_out.response_bytes)?;
        assert_eq!(resp["ok"], json!(true));

        let saved_doc = curvebook_format::open_project_bytes(&fs::read(&save_path)?)?;
        assert_eq!(saved_doc.data_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["t1"]);
        assert!(saved_doc.graph_sheets.is_empty());
        assert_eq!(win.app().session().document().graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g1"]);
        assert_eq!(win.app().session().file_path(), Some(save_path.as_path()));
        assert!(win.app().session().is_dirty() && win.has_unsaved_changes());
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_doc = rec_rt.window(rec_id)?.app().session().document();
    assert_eq!(rec_doc.data_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["t1"]);
    assert_eq!(rec_doc.graph_sheets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["g1"]);
    rec_rt.close_all_force()?;
    Ok(())
}
