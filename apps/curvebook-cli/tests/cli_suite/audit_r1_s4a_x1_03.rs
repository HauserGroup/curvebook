//! Permanent regression tests for `R1-X1-03` across `curvebook-cli`:
//! - `convert`, `run`, and `batch` reject FIFOs, symlinks to FIFOs, and oversized inputs without
//!   blocking or OOM.
use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES};
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let f = File::create(path)?;
    f.set_len(byte_len)?;
    Ok(())
}

fn run_cli_bounded(args: Vec<String>, timeout: Duration) -> Result<Output, String> {
    CliHarness::run_bounded(&args, timeout)
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
    assert_eq!(
        parsed.get("error").and_then(|e| e.get("code")).and_then(Value::as_str),
        Some(expected_code),
        "stdout={parsed}, stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn r1_x1_03_cli_rejects_fifos_and_symlinks_without_blocking() -> TestResult {
    let dir = TempDir::new("fifo")?;
    let fifo_csv = dir.child("pipe.csv");
    assert!(Command::new("mkfifo").arg(&fifo_csv).status()?.success());
    let link_csv = dir.child("link.csv");
    std::os::unix::fs::symlink(&fifo_csv, &link_csv)?;

    let out_svg = dir.child("out.svg").display().to_string();
    let timeout = Duration::from_secs(20);

    for target in [&fifo_csv, &link_csv] {
        let out = run_cli_bounded(vec![String::from("convert"), target.display().to_string(), out_svg.clone(), String::from("--json")], timeout)?;
        assert!(!out.status.success());
    }

    let script_out = run_cli_bounded(vec![String::from("run"), fifo_csv.display().to_string(), String::from("--json")], timeout)?;
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
    let batch_out = run_cli_bounded(vec![String::from("batch"), manifest_path.display().to_string(), String::from("--json")], timeout)?;
    assert!(!batch_out.status.success());

    Ok(())
}

#[test]
fn r1_x1_03_cli_rejects_oversized_inputs_with_resource_limit() -> TestResult {
    let dir = TempDir::new("oversized")?;
    let timeout = Duration::from_secs(20);

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
