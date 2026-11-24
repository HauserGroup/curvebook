//! Integration tests for CurveBook CLI startup environment variables, `--` option
//! termination for `-`-prefixed file paths, and `--new-instance` process isolation (`R1-S11-13`).

use std::path::Path;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook::desktop_host::{DesktopHostEvent, DesktopWindowId};
use curvebook_doc::{DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
use curvebook_engine::Session;
use curvebook_format::{LOCKFILE_NAME, save_project_file};

use crate::common::TempDirGuard;

fn run_bin(args: &[&str], cwd: Option<&Path>, extra_env: &[(&str, &str)]) -> Result<Output, Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_curvebook"));
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.env_remove("CURVEBOOK_CONTROL_PORT");
    cmd.env_remove("CURVEBOOK_CONTROL_TOKEN");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    Ok(cmd.output()?)
}

fn now_epoch_ms() -> u64 {
    u64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis())).unwrap_or(u64::MAX)
}

#[test]
fn invalid_env_control_port_warns_and_does_not_abort_uncontrolled_launch() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-env-port")?;
    let portable = tmp.path.to_str().ok_or("non-UTF-8 temp path")?;

    for bad_port in ["abc", "0", "70000", "-1"] {
        let out = run_bin(&["--portable-dir", portable, "--headless-once"], None, &[("CURVEBOOK_CONTROL_PORT", bad_port)])?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "uncontrolled launch with CURVEBOOK_CONTROL_PORT={bad_port} failed: {stderr}");
        assert!(stderr.contains("curvebook warning:") && stderr.contains("CURVEBOOK_CONTROL_PORT"), "expected warning on stderr, got: {stderr}");
    }

    // Invalid CURVEBOOK_CONTROL_TOKEN together with invalid CURVEBOOK_CONTROL_PORT also warns and does not abort.
    let bad_both = run_bin(
        &["--portable-dir", portable, "--headless-once"],
        None,
        &[("CURVEBOOK_CONTROL_PORT", "abc"), ("CURVEBOOK_CONTROL_TOKEN", "bad token")],
    )?;
    assert!(bad_both.status.success(), "unexpected failure: {}", String::from_utf8_lossy(&bad_both.stderr));

    // `--version` and `--help` remain quiet and succeed even if CURVEBOOK_CONTROL_PORT is malformed.
    let ver = run_bin(&["--version"], None, &[("CURVEBOOK_CONTROL_PORT", "abc")])?;
    assert!(ver.status.success());
    assert!(ver.stderr.is_empty(), "expected no stderr warning on --version, got: {}", String::from_utf8_lossy(&ver.stderr));

    // Explicit CLI `--control` with invalid port still aborts.
    for bad_cli in ["0", "abc"] {
        let out = run_bin(&["--portable-dir", portable, "--control", bad_cli, "--headless-once"], None, &[])?;
        assert!(!out.status.success(), "explicit `--control {bad_cli}` must fail");
    }

    Ok(())
}

#[test]
fn double_dash_terminator_allows_dash_prefixed_files_and_rejects_without_separator() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-dash-files")?;
    let portable_dir = tmp.path.join("portable");
    std::fs::create_dir_all(&portable_dir)?;
    let portable = portable_dir.to_str().ok_or("non-UTF-8 portable path")?;

    let dash_csv = tmp.path.join("-data.csv");
    std::fs::write(&dash_csv, "Dose,Response\n1,10\n2,20\n")?;

    let dash_help_csv = tmp.path.join("--help.csv");
    std::fs::write(&dash_help_csv, "A,B\n3,4\n")?;

    let dash_proj = tmp.path.join("-study.curvebook");
    let mut doc = ProjectDocument::new("Dash Study")?;
    doc.add_data_sheet(DataSheet::new("ds-1", "Dose", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2)?))?)?;
    save_project_file(&dash_proj, &doc)?;

    // Without `--`, `-data.csv` is still rejected as an unknown option (with hint to use `--`).
    let without_sep = run_bin(&["--portable-dir", portable, "--headless-once", "-data.csv"], Some(&tmp.path), &[])?;
    assert!(!without_sep.status.success(), "bare `-data.csv` without `--` must be rejected");
    let err_text = String::from_utf8_lossy(&without_sep.stderr);
    assert!(err_text.contains("unknown option"), "expected unknown option error, got: {err_text}");
    assert!(err_text.contains("--"), "expected `--` hint in error, got: {err_text}");

    // With `--`, `-data.csv` and `-study.curvebook` and `--help.csv` open cleanly.
    let csv_ok = run_bin(&["--portable-dir", portable, "--headless-once", "--", "-data.csv"], Some(&tmp.path), &[])?;
    assert!(csv_ok.status.success(), "opening `-data.csv` after `--` failed: {}", String::from_utf8_lossy(&csv_ok.stderr));

    let merged_ok = run_bin(&["--portable-dir", portable, "--headless-once", "--", "-study.curvebook", "-data.csv"], Some(&tmp.path), &[])?;
    assert!(merged_ok.status.success(), "opening `-study.curvebook` + `-data.csv` after `--` failed: {}", String::from_utf8_lossy(&merged_ok.stderr));

    let flag_named_csv_ok = run_bin(&["--portable-dir", portable, "--headless-once", "--", "--help.csv"], Some(&tmp.path), &[])?;
    assert!(flag_named_csv_ok.status.success(), "opening `--help.csv` after `--` failed: {}", String::from_utf8_lossy(&flag_named_csv_ok.stderr));

    // `-- --help` must NOT print usage and exit 0; `--help` is treated as a file path and fails to open.
    let help_as_file = run_bin(&["--portable-dir", portable, "--headless-once", "--", "--help"], Some(&tmp.path), &[])?;
    assert!(!help_as_file.status.success(), "`-- --help` must treat `--help` as a missing file rather than printing help");
    assert!(!String::from_utf8_lossy(&help_as_file.stdout).contains("usage: curvebook"));

    // Bare trailing `--` with no files succeeds cleanly.
    let bare_sep = run_bin(&["--portable-dir", portable, "--headless-once", "--"], Some(&tmp.path), &[])?;
    assert!(bare_sep.status.success(), "bare `--` failed: {}", String::from_utf8_lossy(&bare_sep.stderr));

    Ok(())
}

#[test]
fn new_instance_isolates_recovery_lock_and_opens_window_separately() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-new-inst")?;
    let portable_dir = tmp.path.join("portable");
    let recovery_dir = portable_dir.join("curvebook-data").join("recovery");
    std::fs::create_dir_all(&recovery_dir)?;
    let portable = portable_dir.to_str().ok_or("non-UTF-8 portable path")?;

    // Hold primary recovery lock to simulate an already-running primary instance.
    let (_primary_session, primary_journal, _report) =
        Session::open_dir_with_recovery(&recovery_dir, "primary-instance-holder", 300_000, now_epoch_ms())?;

    // Launching without `--new-instance` is blocked by the active primary recovery lock.
    let blocked = run_bin(&["--portable-dir", portable, "--headless-once"], None, &[])?;
    assert!(!blocked.status.success(), "default launch must fail when primary lock is actively held");

    // Launching WITH `--new-instance` succeeds using an isolated journal directory and leaves primary lock intact.
    let dash_csv = tmp.path.join("-new-inst-data.csv");
    std::fs::write(&dash_csv, "X,Y\n1,2\n")?;
    let isolated = run_bin(&["--portable-dir", portable, "--new-instance", "--headless-once", "--", "-new-inst-data.csv"], Some(&tmp.path), &[])?;
    assert!(isolated.status.success(), "`--new-instance` launch failed: {}", String::from_utf8_lossy(&isolated.stderr));
    assert!(recovery_dir.join(LOCKFILE_NAME).is_file(), "primary lockfile must remain untouched");

    // `--new-instance` also routes dropped project files on a clean window into a second window (window_id 2),
    // whereas without `--new-instance` dropping onto a clean window replaces window 1 in place (no window 2).
    let drop_proj = tmp.path.join("dropped.curvebook");
    let mut drop_doc = ProjectDocument::new("Dropped Project")?;
    drop_doc.add_data_sheet(DataSheet::new("drop-1", "Drop", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 1, 2)?))?)?;
    save_project_file(&drop_proj, &drop_doc)?;

    let script_path = tmp.path.join("drop-check.jsonl");
    let events = [
        DesktopHostEvent::FileDropped { window_id: DesktopWindowId(1), path: drop_proj },
        DesktopHostEvent::RedrawRequested { window_id: DesktopWindowId(2) },
    ];
    let mut jsonl = String::new();
    for ev in &events {
        jsonl.push_str(&serde_json::to_string(ev)?);
        jsonl.push('\n');
    }
    std::fs::write(&script_path, jsonl)?;
    let script_str = script_path.to_str().ok_or("non-UTF-8 script path")?;

    let fresh_portable = tmp.path.join("fresh-portable");
    let fresh_str = fresh_portable.to_str().ok_or("non-UTF-8 fresh portable path")?;
    let multi_win = run_bin(&["--portable-dir", fresh_str, "--new-instance", "--headless-once", "--event-script", script_str], None, &[])?;
    assert!(multi_win.status.success(), "`--new-instance` FileDropped must open window 2: {}", String::from_utf8_lossy(&multi_win.stderr));

    // Contrast: without `--new-instance` (and distinct portable dir so primary lock doesn't block), window 2 is not opened.
    let single_portable = tmp.path.join("single-portable");
    let single_str = single_portable.to_str().ok_or("non-UTF-8 single portable path")?;
    let single_win = run_bin(&["--portable-dir", single_str, "--headless-once", "--event-script", script_str], None, &[])?;
    assert!(!single_win.status.success(), "without `--new-instance`, clean FileDropped should replace window 1 rather than open window 2");

    drop(primary_journal.abandon());
    Ok(())
}
