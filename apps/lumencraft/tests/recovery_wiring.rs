#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lumencraft::desktop_host::{DesktopHostConfig, DesktopHostRuntime};
use lumencraft_doc::{CellCoord, SheetId};
use lumencraft_engine::Session;
use lumencraft_format::{JOURNAL_FILE_NAME, LOCKFILE_NAME, SNAPSHOT_FILE_NAME};
use lumencraft_ui_egui::ControlAuth;
use lumencraft_ui_egui::interaction::ViewportMetrics;
use serde_json::{Value, json};

static APP_TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = APP_TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("lumencraft-app-wal-{label}-{}-{id}", std::process::id()));
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
    assert!(!recovery_dir.join(LOCKFILE_NAME).exists(), "clean shutdown must release lumencraft.lock");
    let leftover_journal = fs::read(recovery_dir.join(JOURNAL_FILE_NAME)).unwrap_or_default();
    assert!(leftover_journal.is_empty(), "clean shutdown must clear ops.journal");
    Ok(())
}

#[test]
fn r1_s11_01_and_s06_03_binary_default_launch_and_control_recovery() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_lumencraft");
    let dir = TempDirGuard::new("s11-bin")?;
    let data_dir = dir.path.join("lumencraft-data");
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
    assert!(stderr_str.contains("lumencraft recovery warning"), "expected recovery warning on stderr, got: {stderr_str}");

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
            return Err(format!("lumencraft --control exited unexpectedly with {status}").into());
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
