#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
use curvebook_doc::{CellCoord, SheetId};
use curvebook_engine::Session;
use curvebook_format::{JOURNAL_FILE_NAME, LOCKFILE_NAME, SNAPSHOT_FILE_NAME};
use curvebook_ui_egui::ControlAuth;
use curvebook_ui_egui::interaction::ViewportMetrics;
use serde_json::{Value, json};

static APP_TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = APP_TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("curvebook-app-wal-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn now_epoch_ms() -> u64 {
    u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())).unwrap_or(u64::MAX)
}

#[test]
fn r1_s11_01_default_single_instance_control_stream_journals_and_recovers_after_crash() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-stream")?;
    let recovery_dir = dir.path.join("recovery");
    let auth = ControlAuth::from_token("test-recovery-wiring-secret-token")?;
    let viewport = ViewportMetrics::new(800.0, 600.0, 1.0);

    // 1. Simulate default single-instance launch attaching open_dir_with_recovery into DesktopWindowSession.
    {
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "single-inst-1", 2_000, 1_000)?;
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;
        let win = rt.window_mut(win_id)?;
        assert!(win.has_recovery_journal(), "default single-instance window must hold active recovery journal");

        let reqs = [
            json!({
                "id": "r1",
                "method": "engine.execute",
                "token": auth.as_str(),
                "params": {
                    "command_id": "sheet.createData",
                    "params": { "sheet_id": "ctrl-ds", "title": "Control WAL", "archetype": "column", "cols": 2, "rows": 4 }
                }
            }),
            json!({
                "id": "r2",
                "method": "command.run",
                "token": auth.as_str(),
                "command": "table.setCell",
                "params": { "sheet_id": "ctrl-ds", "row": 0, "col": 0, "value": 12.5 }
            }),
            json!({
                "id": "r3",
                "method": "command.run",
                "token": auth.as_str(),
                "command": "table.setCell",
                "params": { "sheet_id": "ctrl-ds", "row": 0, "col": 0, "value": 99.0 }
            }),
            // Atomic failing batch must roll back without poisoning ops.journal.
            json!({
                "id": "r4",
                "method": "command.batch",
                "token": auth.as_str(),
                "params": {
                    "atomic": true,
                    "commands": [
                        { "command": "table.setCell", "params": { "sheet_id": "ctrl-ds", "row": 1, "col": 0, "value": 5.0 } },
                        { "command": "no.such.command", "params": {} }
                    ]
                }
            }),
            // Undo must checkpoint cleanly (R1-S07-06) so post-crash recovery restores 12.5, not 99.0.
            json!({
                "id": "r5",
                "method": "command.run",
                "token": auth.as_str(),
                "command": "edit.undo",
                "params": {}
            }),
            // Follow-up command after undo appends cleanly to ops.journal after checkpoint.
            json!({
                "id": "r6",
                "method": "table.setCell",
                "token": auth.as_str(),
                "params": { "sheet_id": "ctrl-ds", "row": 1, "col": 0, "value": 34.5 }
            }),
        ];
        let mut input = String::new();
        for req in reqs {
            input.push_str(&serde_json::to_string(&req)?);
            input.push('\n');
        }
        let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
        let mut out = Vec::new();
        let quit = win.serve_control_stream_with_quit(&mut reader, &mut out, &auth)?;
        assert!(!quit);

        let lines: Vec<Value> = String::from_utf8(out)?.lines().map(serde_json::from_str).collect::<Result<Vec<Value>, _>>()?;
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0]["ok"], json!(true));
        assert_eq!(lines[1]["ok"], json!(true));
        assert_eq!(lines[2]["ok"], json!(true));
        assert_eq!(lines[3]["ok"], json!(false));
        assert_eq!(lines[4]["ok"], json!(true));
        assert_eq!(lines[5]["ok"], json!(true));

        assert!(recovery_dir.join(SNAPSHOT_FILE_NAME).is_file(), "undo checkpoint must persist snapshot");
        assert!(recovery_dir.join(JOURNAL_FILE_NAME).is_file(), "follow-up --control edit must persist to ops.journal");
        // Simulate SIGKILL / crash: drop runtime WITHOUT calling close_all_force().
        drop(rt);
    }

    // 2. Reopen recovery directory after stale TTL and verify exact live state + clean close on exit.
    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: None, ..DesktopHostConfig::default() });
    let rec_win_id = rec_rt.recover_window_from_dir(recovery_dir.clone(), viewport, 5_000)?;
    {
        let rec_win = rec_rt.window(rec_win_id)?;
        assert!(rec_win.recovered_from_crash());
        let ds = rec_win.app().session().document().data_sheet(&SheetId::new("ctrl-ds")).ok_or("missing recovered ctrl-ds")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(12.5));
        assert_eq!(ds.cell(CellCoord::new(1, 0, 0)).and_then(|c| c.active_number()), Some(34.5));
    }

    rec_rt.close_all_force()?;
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "clean shutdown must release curvebook.lock");
    let leftover_journal = fs::read(recovery_dir.join(JOURNAL_FILE_NAME)).unwrap_or_default();
    assert!(leftover_journal.is_empty(), "clean shutdown must clear ops.journal");
    Ok(())
}

#[test]
fn r1_s11_01_and_s06_03_binary_default_launch_and_control_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let dir = TempDirGuard::new("s11-bin")?;
    let data_dir = dir.path.join("curvebook-data");
    let recovery_dir = data_dir.join("recovery");

    // 1. Clean default single-instance launch (--headless-once --portable-dir) attaches recovery and cleans up on exit.
    let clean_out = Command::new(bin).env("HOME", &dir.path).args(["--headless-once", "--portable-dir"]).arg(&dir.path).output()?;
    assert!(clean_out.status.success(), "clean headless-once failed: {}", String::from_utf8_lossy(&clean_out.stderr));
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "clean exit must release lockfile");

    // 2. R1-S06-03 on app startup: inject an interior corrupt line alongside an intact WAL entry and launch.
    {
        let (mut s, mut j, _) = Session::open_dir_with_recovery(&recovery_dir, "precrash-owner", 1_000, 1)?;
        s.execute_and_record(
            &mut j,
            "sheet.createData",
            json!({ "sheet_id": "surviving-sheet", "title": "Intact Pre-Corrupt", "archetype": "column", "cols": 1, "rows": 2 }),
        )?;
        let raw_journal_path = recovery_dir.join(JOURNAL_FILE_NAME);
        let mut f = fs::OpenOptions::new().append(true).open(&raw_journal_path)?;
        f.write_all(b"{\"format_version\":1,\"seq\":2,\"torn_interior")?;
        f.sync_data()?;
        s.execute_and_record(&mut j, "table.setCell", json!({ "sheet_id": "surviving-sheet", "row": 0, "col": 0, "value": 7.0 }))?;
        drop(j.abandon());
    }
    // Wait briefly so lock TTL (1_000ms) is stale relative to process clock (already true since now_epoch_ms >> 1001).
    let heal_out = Command::new(bin).env("HOME", &dir.path).args(["--headless-once", "--portable-dir"]).arg(&dir.path).output()?;
    assert!(
        heal_out.status.success(),
        "default launch with interior corrupt WAL line must NOT abort startup: {}",
        String::from_utf8_lossy(&heal_out.stderr)
    );
    let stderr_str = String::from_utf8_lossy(&heal_out.stderr);
    assert!(stderr_str.contains("curvebook recovery warning"), "expected recovery warning on stderr, got: {stderr_str}");

    let (reopened_after_heal, heal_journal, _) = Session::open_dir_with_recovery(&recovery_dir, "verify-heal", 2_000, now_epoch_ms() + 10)?;
    let healed_ds = reopened_after_heal.document().data_sheet(&SheetId::new("surviving-sheet")).ok_or("missing surviving-sheet")?;
    assert_eq!(healed_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(7.0));
    let mut reopen_mut = reopened_after_heal;
    let _ = reopen_mut.close_journal_cleanly(heal_journal)?;

    // 3. Default single-instance --control binary crash test (when loopback TCP connect is permitted by host sandbox).
    let probe_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(Box::new(err)),
    };
    let port = probe_listener.local_addr()?.port();
    drop(probe_listener);

    let token_path = data_dir.join(format!("control-{port}.token"));
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let child = Command::new(bin).env("HOME", &dir.path).args(["--portable-dir"]).arg(&dir.path).args(["--control", &port.to_string()]).spawn()?;
    let mut guard = ChildGuard(child);

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !token_path.exists() {
        if let Some(status) = guard.0.try_wait()? {
            return Err(format!("curvebook --control exited unexpectedly with {status}").into());
        }
        if std::time::Instant::now() >= deadline {
            return Err("timed out waiting for control token file".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
            return Ok(());
        }
        Err(err) => return Err(Box::new(err)),
    };
    let token = fs::read_to_string(&token_path)?;
    {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut r = BufReader::new(stream.try_clone()?);
        let mut w = BufWriter::new(stream);
        for req in [
            json!({
                "id": "tcp-1",
                "method": "engine.execute",
                "token": token,
                "params": {
                    "command_id": "sheet.createData",
                    "params": { "sheet_id": "tcp-sheet", "title": "TCP Recovered", "archetype": "column", "cols": 1, "rows": 2 }
                }
            }),
            json!({
                "id": "tcp-2",
                "method": "table.setCell",
                "token": token,
                "params": { "sheet_id": "tcp-sheet", "row": 0, "col": 0, "value": 42.25 }
            }),
        ] {
            w.write_all(serde_json::to_string(&req)?.as_bytes())?;
            w.write_all(b"\n")?;
            w.flush()?;
            let mut line = String::new();
            std::io::BufRead::read_line(&mut r, &mut line)?;
            let resp: Value = serde_json::from_str(&line)?;
            assert_eq!(resp["ok"], json!(true), "control request failed: {resp}");
        }
    }

    assert!(recovery_dir.join(JOURNAL_FILE_NAME).is_file(), "TCP --control mutations must flush to ops.journal immediately");
    // Simulate hard crash (SIGKILL without app.quit or clean shutdown).
    let _ = guard.0.kill();
    let _ = guard.0.wait();

    // Relaunch with expired lease timestamp and confirm TCP mutations survive crash recovery.
    let (mut rec_tcp, rec_tcp_journal, tcp_report) =
        Session::open_dir_with_recovery(&recovery_dir, "post-sigkill-relaunch", 5_000, now_epoch_ms().saturating_add(400_000))?;
    assert!(tcp_report.recovered_from_stale_lock.is_some());
    assert_eq!(tcp_report.recovered_commands.len(), 2);
    let tcp_ds = rec_tcp.document().data_sheet(&SheetId::new("tcp-sheet")).ok_or("missing recovered tcp-sheet")?;
    assert_eq!(tcp_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(42.25));
    let _ = rec_tcp.close_journal_cleanly(rec_tcp_journal)?;
    Ok(())
}

#[test]
fn r1_s11_03_lease_renewal_new_instance_isolation_and_multiwindow_close_all_force() -> Result<(), Box<dyn std::error::Error>> {
    use curvebook_format::{DirectoryJournalBackend, JournalBackend};

    let dir = TempDirGuard::new("s11-03-renewal-and-new-instance")?;
    let recovery_root = dir.path.join("recovery-root");
    let viewport = ViewportMetrics::new(800.0, 520.0, 2.0);

    // 1. Primary instance opens recovery_root and renews its lease timestamp per executed command.
    let mut primary_rt = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: Some(recovery_root.clone()),
        new_instance: false,
        ..DesktopHostConfig::default()
    });
    let w1 = primary_rt.open_window(None, viewport)?;
    let initial_acquired = DirectoryJournalBackend::open_dir(&recovery_root)?.read_lease()?.ok_or("missing primary lease")?.acquired_epoch_ms;

    std::thread::sleep(Duration::from_millis(15));
    let _ = primary_rt
        .window_mut(w1)?
        .execute_and_journal("sheet.createData", json!({ "sheet_id": "renew-ds", "title": "Renew", "archetype": "column", "cols": 1, "rows": 1 }))?;
    let renewed_acquired = DirectoryJournalBackend::open_dir(&recovery_root)?.read_lease()?.ok_or("missing renewed primary lease")?.acquired_epoch_ms;
    assert!(renewed_acquired >= initial_acquired + 10, "lease timestamp should renew per command");

    // 2. Second runtime with --new-instance gets isolated recovery dir without conflicting on primary lock.
    let mut isolated_rt =
        DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_root.clone()), new_instance: true, ..DesktopHostConfig::default() });
    let iso_w = isolated_rt.open_window(None, viewport)?;
    let iso_dir = isolated_rt.window(iso_w)?.recovery_dir().ok_or("missing isolated dir")?.to_path_buf();
    assert_ne!(iso_dir, recovery_root);
    assert!(iso_dir.join(LOCKFILE_NAME).is_file());
    assert!(recovery_root.join(LOCKFILE_NAME).is_file());

    isolated_rt.close_all_force()?;
    assert!(!iso_dir.join(LOCKFILE_NAME).exists());
    assert!(recovery_root.join(LOCKFILE_NAME).is_file(), "closing --new-instance must leave primary lock intact");

    // 3. Multi-window close_all_force cleans remaining windows even if one window lost lock ownership.
    let w2 = primary_rt.open_window(None, viewport)?;
    let w2_dir = primary_rt.window(w2)?.recovery_dir().ok_or("missing w2 dir")?.to_path_buf();
    assert!(w2_dir.join(LOCKFILE_NAME).is_file());
    // Steal w1's lock so closing w1 fails; close_all_force must still release w2's lock cleanly.
    fs::write(
        recovery_root.join(LOCKFILE_NAME),
        serde_json::to_vec(&curvebook_format::JournalLockLease::new("external-owner", now_epoch_ms(), 300_000)?)?,
    )?;
    let close_err = primary_rt.close_all_force().expect_err("stolen lock on w1 must report error");
    assert!(close_err.contains("external-owner"));
    assert!(!w2_dir.join(LOCKFILE_NAME).exists(), "close_all_force must still close healthy window 2");
    assert!(recovery_root.join(LOCKFILE_NAME).is_file(), "external-owner lock on w1 must not be unlinked");

    Ok(())
}

#[test]
fn r1_s11_02_install_window_error_releases_recovery_lock() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-02-install-rollback")?;
    let recovery_root = dir.path.join("recovery-root");
    let viewport = ViewportMetrics::new(800.0, 520.0, 2.0);

    let mut bad_rt = DesktopHostRuntime::new(DesktopHostConfig {
        recovery_root: Some(recovery_root.clone()),
        theme_override: Some(String::from("nonexistent-theme")),
        ..DesktopHostConfig::default()
    });
    assert!(bad_rt.open_window(None, viewport).is_err(), "invalid theme override must abort window open");
    assert!(!recovery_root.join(LOCKFILE_NAME).exists(), "failed window open must release recovery lock");

    Ok(())
}

#[test]
fn r1_s11_05_and_06_binary_corrupt_prefs_config_dir_and_no_prefs() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("s11-05-06-binary")?;
    let cfg_dir = dir.path.join("cfg");
    fs::create_dir_all(&cfg_dir)?;
    fs::write(cfg_dir.join("shortcuts.json"), "{not json")?;
    fs::write(cfg_dir.join("preferences.json"), "{also not json")?;

    // Corrupt shortcuts.json & preferences.json warn and launch cleanly with CURVEBOOK_CONFIG_DIR.
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let ok_corrupt = Command::new(bin).env("CURVEBOOK_CONFIG_DIR", &cfg_dir).args(["--headless-once"]).output()?;
    assert!(ok_corrupt.status.success(), "corrupt prefs must not abort launch: {}", String::from_utf8_lossy(&ok_corrupt.stderr));

    // With CURVEBOOK_NO_PREFS=1, even an --event-script launch avoids reading/writing prefs or recovery_dir.
    let script = dir.path.join("script.jsonl");
    fs::write(&script, "{\"WindowFocused\":{\"window_id\":1}}\n")?;
    let no_prefs = Command::new(bin)
        .env("CURVEBOOK_CONFIG_DIR", &cfg_dir)
        .env("CURVEBOOK_NO_PREFS", "1")
        .args(["--event-script", &script.display().to_string()])
        .output()?;
    assert!(no_prefs.status.success(), "CURVEBOOK_NO_PREFS=1 launch failed: {}", String::from_utf8_lossy(&no_prefs.stderr));
    assert!(!cfg_dir.join("recovery").exists(), "CURVEBOOK_NO_PREFS=1 must skip shared recovery dir");

    Ok(())
}

#[test]
fn audit_2_mixed_batch_undo_and_create_recovers_exact_sheet_count() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit2-mixed-batch")?;
    let recovery_dir = dir.path.join("recovery");
    let auth = ControlAuth::from_token("audit2-mixed-batch-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "audit2-1", 2_000, 1_000)?;
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;
        let win = rt.window_mut(win_id)?;

        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "archetype": "column", "cols": 1, "rows": 2 }))?;
        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t2", "title": "Second", "archetype": "column", "cols": 1, "rows": 2 }))?;

        // Mixed batch: edit.undo removes `t2`, then sheet.createData creates `t3` -> live state has ["t1", "t3"].
        let batch_req = json!({
            "id": "b1",
            "method": "command.batch",
            "token": auth.as_str(),
            "params": {
                "atomic": true,
                "commands": [
                    { "command": "edit.undo", "params": {} },
                    { "command": "sheet.createData", "params": { "sheet_id": "t3", "title": "Third", "archetype": "column", "cols": 1, "rows": 2 } }
                ]
            }
        });
        let input = format!("{batch_req}\n");
        let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
        let mut out = Vec::new();
        let quit = win.serve_control_stream_with_quit(&mut reader, &mut out, &auth)?;
        assert!(!quit);
        let resp: Value = serde_json::from_slice(&out)?;
        assert_eq!(resp["ok"], json!(true), "mixed batch failed: {resp}");

        let live_ids: Vec<&str> = win.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(live_ids, vec!["t1", "t3"]);
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_ids: Vec<&str> = rec_rt.window(rec_id)?.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(rec_ids, vec!["t1", "t3"], "mixed batch recovery must not duplicate post-undo steps");
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_5_gui_edit_after_64_undo_eviction_persists_through_crash_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit5-64-undo")?;
    let recovery_dir = dir.path.join("recovery");
    let viewport = ViewportMetrics::new(1100.0, 720.0, 1.0);

    {
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "audit5-1", 2_000, 1_000)?;
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;
        {
            let win = rt.window_mut(win_id)?;
            win.execute_and_journal(
                "sheet.createData",
                json!({ "sheet_id": "s64", "title": "FullUndo", "archetype": "column", "cols": 2, "rows": 4 }),
            )?;
            // Fill the 64-entry session undo ring completely.
            for i in 0..64 {
                win.execute_and_journal("table.setCell", json!({ "sheet_id": "s64", "row": (i % 3) + 1, "col": 0, "value": f64::from(i + 1) }))?;
            }
            assert_eq!(win.app().session().undo_depth(), 64);
            assert_eq!(win.app().session().redo_depth(), 0);
            assert!(win.app().session().is_dirty());
        }

        // Perform a genuine egui GUI double-click + text commit + Enter edit on cell (0, 0, 0).
        let mut native = curvebook::gui_window::CurveBookNativeApp::new(rt, win_id, None);
        let ctx = eframe::egui::Context::default();
        ctx.set_fonts(curvebook::gui_window::build_egui_font_definitions(None));
        let screen = eframe::egui::Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(1280.0, 800.0));
        let step = |app_host: &mut curvebook::gui_window::CurveBookNativeApp, events: Vec<eframe::egui::Event>| {
            let mut out = ctx.run_ui(eframe::egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| app_host.step_ui(ui));
            out.textures_delta.clear();
        };
        for _ in 0..2 {
            step(&mut native, Vec::new());
        }
        let cell_pos = native.inspect_meta().find_by_widget_id("grid.cell.0.0.0").ok_or("missing grid.cell.0.0.0")?.clipped_rect.center();
        step(
            &mut native,
            vec![
                eframe::egui::Event::PointerMoved(cell_pos),
                eframe::egui::Event::PointerButton {
                    pos: cell_pos,
                    button: eframe::egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: eframe::egui::Modifiers::NONE,
                },
                eframe::egui::Event::PointerButton {
                    pos: cell_pos,
                    button: eframe::egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: eframe::egui::Modifiers::NONE,
                },
                eframe::egui::Event::PointerButton {
                    pos: cell_pos,
                    button: eframe::egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: eframe::egui::Modifiers::NONE,
                },
                eframe::egui::Event::PointerButton {
                    pos: cell_pos,
                    button: eframe::egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: eframe::egui::Modifiers::NONE,
                },
            ],
        );
        step(&mut native, Vec::new());
        step(
            &mut native,
            vec![
                eframe::egui::Event::Text(String::from("777.25")),
                eframe::egui::Event::Key {
                    key: eframe::egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: eframe::egui::Modifiers::NONE,
                },
            ],
        );
        step(&mut native, Vec::new());

        let win = native.runtime().window(win_id)?;
        assert_eq!(win.app().session().undo_depth(), 64, "undo stack stayed capped at 64 after oldest eviction");
        let ds = win.app().session().document().data_sheet(&SheetId::new("s64")).ok_or("missing live s64")?;
        assert_eq!(ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()), Some(777.25));
        drop(native);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_ds = rec_rt.window(rec_id)?.app().session().document().data_sheet(&SheetId::new("s64")).ok_or("missing recovered s64")?;
    assert_eq!(
        rec_ds.cell(CellCoord::new(0, 0, 0)).and_then(|c| c.active_number()),
        Some(777.25),
        "GUI edit at 64-entry undo eviction boundary must checkpoint to crash recovery"
    );
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_118_mixed_batch_gui_menu_then_create_recovers_exact_sheets() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit118-mixed-batch")?;
    let recovery_dir = dir.path.join("recovery");
    let auth = ControlAuth::from_token("audit118-mixed-batch-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "audit118-1", 2_000, 1_000)?;
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;
        let win = rt.window_mut(win_id)?;

        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "archetype": "column", "cols": 1, "rows": 2 }))?;

        // Non-atomic mixed batch: GUI menu action creates `g1`, followed by `sheet.createData` for `t2`.
        let batch_req = json!({
            "id": "b118-1",
            "method": "command.batch",
            "token": auth.as_str(),
            "params": {
                "atomic": false,
                "commands": [
                    { "command": "ui.invoke_menu", "params": { "command_id": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "t1", "kind": "column" } } },
                    { "command": "sheet.createData", "params": { "sheet_id": "t2", "title": "Second", "archetype": "column", "cols": 1, "rows": 2 } }
                ]
            }
        });
        let input = format!("{batch_req}\n");
        let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
        let mut out = Vec::new();
        let quit = win.serve_control_stream_with_quit(&mut reader, &mut out, &auth)?;
        assert!(!quit);
        let resp: Value = serde_json::from_slice(&out)?;
        assert_eq!(resp["ok"], json!(true), "mixed GUI+create batch failed: {resp}");

        let live_data_ids: Vec<&str> = win.app().session().document().data_sheets.iter().map(|s| s.id.as_str()).collect();
        let live_graph_ids: Vec<&str> = win.app().session().document().graph_sheets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(live_data_ids, vec!["t1", "t2"]);
        assert_eq!(live_graph_ids, vec!["g1"]);
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_doc = rec_rt.window(rec_id)?.app().session().document();
    let rec_data_ids: Vec<&str> = rec_doc.data_sheets.iter().map(|s| s.id.as_str()).collect();
    let rec_graph_ids: Vec<&str> = rec_doc.graph_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(rec_data_ids, vec!["t1", "t2"], "mixed batch recovery must retain all data sheets");
    assert_eq!(rec_graph_ids, vec!["g1"], "mixed batch recovery must retain GUI menu-created graph sheet");
    rec_rt.close_all_force()?;
    Ok(())
}

#[test]
fn audit_118_mixed_batch_save_then_gui_mutation_keeps_dirty_and_recovers() -> Result<(), Box<dyn std::error::Error>> {
    let dir = TempDirGuard::new("audit118-save-dirty")?;
    let recovery_dir = dir.path.join("recovery");
    let save_path = dir.path.join("saved-before-gui.curvebook");
    let auth = ControlAuth::from_token("audit118-save-dirty-control-token")?;
    let viewport = ViewportMetrics::new(900.0, 620.0, 1.0);

    {
        let (session, journal, report) = Session::open_dir_with_recovery(&recovery_dir, "audit118-save-1", 2_000, 1_000)?;
        let mut rt = DesktopHostRuntime::new(DesktopHostConfig { recovery_root: Some(recovery_dir.clone()), ..DesktopHostConfig::default() });
        let win_id = rt.open_window_with_recovery(session, viewport, recovery_dir.clone(), journal, Some(report))?;
        let win = rt.window_mut(win_id)?;

        win.execute_and_journal("sheet.createData", json!({ "sheet_id": "t1", "title": "First", "archetype": "column", "cols": 1, "rows": 2 }))?;

        // Mixed batch on live control frame path: save first, then mutate via GUI menu action.
        let batch_req = json!({
            "id": "b118-save",
            "method": "command.batch",
            "params": {
                "atomic": false,
                "commands": [
                    { "command": "project.save", "params": { "path": save_path.display().to_string() } },
                    { "command": "ui.invoke_menu", "params": { "command_id": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "t1", "kind": "column" } } }
                ]
            }
        });
        let raw = format!("{batch_req}\n");
        let frame_out = win.dispatch_live_control_frame_with_hook(raw.as_bytes(), &auth, true, |_, _| Ok(()))?;
        let resp: Value = serde_json::from_slice(&frame_out.response_bytes)?;
        assert_eq!(resp["ok"], json!(true), "save+GUI mixed batch failed: {resp}");

        let saved_doc = curvebook_format::open_project_bytes(&fs::read(&save_path)?)?;
        let saved_data_ids: Vec<&str> = saved_doc.data_sheets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(saved_data_ids, vec!["t1"]);
        assert!(saved_doc.graph_sheets.is_empty(), "pre-GUI saved archive must contain 0 graph sheets");

        let live_graph_ids: Vec<&str> = win.app().session().document().graph_sheets.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(live_graph_ids, vec!["g1"]);
        assert_eq!(win.app().session().file_path(), Some(save_path.as_path()));
        assert!(win.app().session().is_dirty(), "post-save GUI mutation must keep session dirty");
        assert!(win.has_unsaved_changes(), "post-save GUI mutation must keep window marked dirty");
        drop(rt);
    }

    let mut rec_rt = DesktopHostRuntime::new(DesktopHostConfig::default());
    let rec_id = rec_rt.recover_window_from_dir(recovery_dir, viewport, 5_000)?;
    let rec_doc = rec_rt.window(rec_id)?.app().session().document();
    let rec_data_ids: Vec<&str> = rec_doc.data_sheets.iter().map(|s| s.id.as_str()).collect();
    let rec_graph_ids: Vec<&str> = rec_doc.graph_sheets.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(rec_data_ids, vec!["t1"]);
    assert_eq!(rec_graph_ids, vec!["g1"], "crash recovery after save + GUI mutation must recover g1");
    rec_rt.close_all_force()?;
    Ok(())
}
