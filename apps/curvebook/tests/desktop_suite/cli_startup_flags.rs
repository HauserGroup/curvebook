//! Integration tests for CurveBook CLI startup environment variables, `--` option
//! termination for `-`-prefixed file paths, and `--new-instance` process isolation (`R1-S11-13`).

use std::path::Path;
use std::process::{Command, Output};

use curvebook::desktop_host::{DesktopHostEvent, DesktopWindowId};
use curvebook_engine::Session;
use curvebook_format::{LOCKFILE_NAME, save_project_file};

use crate::common::{TempDirGuard, build_xy_doc, now_epoch_ms};

fn run_bin(args: &[&str], cwd: Option<&Path>, extra_env: &[(&str, &str)]) -> Result<Output, Box<dyn std::error::Error>> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_curvebook"));
    cmd.args(args).env_remove("CURVEBOOK_CONTROL_PORT").env_remove("CURVEBOOK_CONTROL_TOKEN");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    Ok(cmd.output()?)
}

#[test]
fn invalid_env_control_port_warns_and_does_not_abort_uncontrolled_launch() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-env-port")?;
    let portable = tmp.path.to_str().ok_or("non-UTF-8 temp path")?;

    for bad_port in ["abc", "0", "70000", "-1"] {
        let out = run_bin(&["--portable-dir", portable, "--headless-once"], None, &[("CURVEBOOK_CONTROL_PORT", bad_port)])?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "uncontrolled launch with CURVEBOOK_CONTROL_PORT={bad_port} failed: {stderr}");
        assert!(stderr.contains("curvebook warning:") && stderr.contains("CURVEBOOK_CONTROL_PORT"), "expected warning, got: {stderr}");
    }

    let bad_both = run_bin(
        &["--portable-dir", portable, "--headless-once"],
        None,
        &[("CURVEBOOK_CONTROL_PORT", "abc"), ("CURVEBOOK_CONTROL_TOKEN", "bad token")],
    )?;
    assert!(bad_both.status.success());

    let ver = run_bin(&["--version"], None, &[("CURVEBOOK_CONTROL_PORT", "abc")])?;
    assert!(ver.status.success() && ver.stderr.is_empty());

    for bad_cli in ["0", "abc"] {
        assert!(!run_bin(&["--portable-dir", portable, "--control", bad_cli, "--headless-once"], None, &[])?.status.success());
    }
    Ok(())
}

#[test]
fn double_dash_terminator_allows_dash_prefixed_files_and_rejects_without_separator() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-dash-files")?;
    let portable_dir = tmp.child("portable");
    std::fs::create_dir_all(&portable_dir)?;
    let portable = portable_dir.to_str().ok_or("non-UTF-8 portable path")?;

    std::fs::write(tmp.child("-data.csv"), "Dose,Response\n1,10\n2,20\n")?;
    std::fs::write(tmp.child("--help.csv"), "A,B\n3,4\n")?;
    save_project_file(&tmp.child("-study.curvebook"), &build_xy_doc("ds-1", "Dose", &[(1.0, 2.0)])?)?;

    let without_sep = run_bin(&["--portable-dir", portable, "--headless-once", "-data.csv"], Some(tmp.path()), &[])?;
    let err_text = String::from_utf8_lossy(&without_sep.stderr);
    assert!(!without_sep.status.success() && err_text.contains("unknown option") && err_text.contains("--"));

    for tail in [vec!["--", "-data.csv"], vec!["--", "-study.curvebook", "-data.csv"], vec!["--", "--help.csv"], vec!["--"]] {
        let mut args = vec!["--portable-dir", portable, "--headless-once"];
        args.extend(tail);
        let out = run_bin(&args, Some(tmp.path()), &[])?;
        assert!(out.status.success(), "{args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let help_as_file = run_bin(&["--portable-dir", portable, "--headless-once", "--", "--help"], Some(tmp.path()), &[])?;
    assert!(!help_as_file.status.success() && !String::from_utf8_lossy(&help_as_file.stdout).contains("usage: curvebook"));
    Ok(())
}

#[test]
fn new_instance_isolates_recovery_lock_and_opens_window_separately() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = TempDirGuard::new("s11-13-new-inst")?;
    let portable_dir = tmp.child("portable");
    let recovery_dir = portable_dir.join("curvebook-data").join("recovery");
    std::fs::create_dir_all(&recovery_dir)?;
    let portable = portable_dir.to_str().ok_or("non-UTF-8 portable path")?;

    let (_primary_session, primary_journal, _) = Session::open_dir_with_recovery(&recovery_dir, "primary-instance-holder", 300_000, now_epoch_ms())?;
    assert!(!run_bin(&["--portable-dir", portable, "--headless-once"], None, &[])?.status.success());

    std::fs::write(tmp.child("-new-inst-data.csv"), "X,Y\n1,2\n")?;
    let isolated = run_bin(&["--portable-dir", portable, "--new-instance", "--headless-once", "--", "-new-inst-data.csv"], Some(tmp.path()), &[])?;
    assert!(isolated.status.success() && recovery_dir.join(LOCKFILE_NAME).is_file());

    let drop_proj = tmp.child("dropped.curvebook");
    save_project_file(&drop_proj, &build_xy_doc("drop-1", "Drop", &[(1.0, 2.0)])?)?;

    let script_path = tmp.child("drop-check.jsonl");
    let events = [
        DesktopHostEvent::FileDropped { window_id: DesktopWindowId(1), path: drop_proj },
        DesktopHostEvent::RedrawRequested { window_id: DesktopWindowId(2) },
    ];
    let jsonl: String = events.iter().map(|e| serde_json::to_string(e).map(|s| format!("{s}\n"))).collect::<Result<_, _>>()?;
    std::fs::write(&script_path, jsonl)?;
    let script_str = script_path.to_str().ok_or("non-UTF-8 script path")?;

    let fresh_str = tmp.child("fresh-portable").to_str().ok_or("non-UTF-8")?.to_owned();
    let single_str = tmp.child("single-portable").to_str().ok_or("non-UTF-8")?.to_owned();
    assert!(run_bin(&["--portable-dir", &fresh_str, "--new-instance", "--headless-once", "--event-script", script_str], None, &[])?.status.success());
    assert!(!run_bin(&["--portable-dir", &single_str, "--headless-once", "--event-script", script_str], None, &[])?.status.success());

    drop(primary_journal.abandon());
    Ok(())
}
