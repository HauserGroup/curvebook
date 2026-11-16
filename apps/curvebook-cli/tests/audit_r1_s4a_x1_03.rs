//! Permanent regression tests for `R1-X1-03` across `curvebook-cli`:
//! - `convert`, `run`, and `batch` reject FIFOs, symlinks to FIFOs, and oversized inputs without
//!   blocking or OOM.
#![forbid(unsafe_code)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TEST_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn create(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-s4a-x103-cli-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let f = File::create(path)?;
    f.set_len(byte_len)?;
    Ok(())
}

fn run_cli_bounded(args: Vec<String>, timeout: Duration) -> Result<Output, String> {
    let child = Command::new(env!("CARGO_BIN_EXE_curvebook-cli"))
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn curvebook-cli failed: {err}"))?;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(err)) => Err(format!("wait_with_output failed: {err}")),
        Err(err) => Err(format!("curvebook-cli hung beyond {timeout:?} for args {args:?}: {err}")),
    }
}

fn assert_json_error_code(out: &Output, expected_code: &str) -> TestResult {
    assert!(
        !out.status.success(),
        "expected non-zero exit code, stdout={}, stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: Value = serde_json::from_slice(&out.stdout).map_err(|err| {
        format!(
            "failed to parse --json stdout error ({err}): stdout={}, stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })?;
    assert_eq!(parsed["error"]["code"].as_str(), Some(expected_code), "stdout={parsed}, stderr={}", String::from_utf8_lossy(&out.stderr));
    Ok(())
}

#[cfg(unix)]
#[test]
fn r1_x1_03_cli_rejects_fifos_and_symlinks_without_blocking() -> TestResult {
    let dir = TempDirGuard::create("fifo")?;
    let fifo_csv = dir.child("pipe.csv");
    assert!(Command::new("mkfifo").arg(&fifo_csv).status()?.success());
    let link_csv = dir.child("link.csv");
    std::os::unix::fs::symlink(&fifo_csv, &link_csv)?;

    let out_svg = dir.child("out.svg").display().to_string();
    let timeout = Duration::from_secs(5);

    for target in [&fifo_csv, &link_csv] {
        let res = run_cli_bounded(vec![String::from("convert"), target.display().to_string(), out_svg.clone(), String::from("--json")], timeout);
        let out = match res {
            Ok(out) => out,
            Err(msg) => {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo_csv) {
                    drop(f);
                }
                return Err(msg.into());
            }
        };
        assert!(!out.status.success());
    }

    let script_out = match run_cli_bounded(vec![String::from("run"), fifo_csv.display().to_string(), String::from("--json")], timeout) {
        Ok(out) => out,
        Err(msg) => {
            if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo_csv) {
                drop(f);
            }
            return Err(msg.into());
        }
    };
    assert!(!script_out.status.success());

    let manifest_path = dir.child("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "inputs": [{ "path": fifo_csv.display().to_string(), "sheet": "d1" }],
            "exports": []
        }))?,
    )?;
    let batch_out = match run_cli_bounded(vec![String::from("batch"), manifest_path.display().to_string(), String::from("--json")], timeout) {
        Ok(out) => out,
        Err(msg) => {
            if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo_csv) {
                drop(f);
            }
            return Err(msg.into());
        }
    };
    assert!(!batch_out.status.success());

    Ok(())
}

#[test]
fn r1_x1_03_cli_rejects_oversized_inputs_with_resource_limit() -> TestResult {
    let dir = TempDirGuard::create("oversized")?;
    let timeout = Duration::from_secs(5);

    let big_csv = dir.child("huge.csv");
    create_sparse_file(&big_csv, (MAX_CSV_FILE_BYTES as u64).saturating_add(1))?;
    let out_svg = dir.child("out.svg").display().to_string();
    let convert_out = run_cli_bounded(vec![String::from("convert"), big_csv.display().to_string(), out_svg, String::from("--json")], timeout)?;
    assert_json_error_code(&convert_out, "resource_limit")?;

    let big_script = dir.child("huge.jsonl");
    create_sparse_file(&big_script, (64 * 1024 * 1024_u64).saturating_add(1))?;
    let run_out = run_cli_bounded(vec![String::from("run"), big_script.display().to_string(), String::from("--json")], timeout)?;
    assert_json_error_code(&run_out, "resource_limit")?;

    let big_tpl = dir.child("huge.cbtemplate");
    create_sparse_file(&big_tpl, (MAX_ARCHIVE_FILE_BYTES as u64).saturating_add(1))?;
    let manifest_path = dir.child("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "template": big_tpl.display().to_string(),
            "exports": []
        }))?,
    )?;
    let batch_out = run_cli_bounded(vec![String::from("batch"), manifest_path.display().to_string(), String::from("--json")], timeout)?;
    assert_json_error_code(&batch_out, "resource_limit")?;

    Ok(())
}
