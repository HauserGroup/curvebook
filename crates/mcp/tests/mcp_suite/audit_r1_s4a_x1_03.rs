//! Permanent regression tests for `R1-X1-03` across `curvebook-mcp`:
//! - `open_file` rejects FIFOs, symlinks to FIFOs, directories, and `/dev/zero` without blocking.
//! - `open_file` enforces per-format size limits before reading or allocating.
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use curvebook_format::{MAX_ARCHIVE_FILE_BYTES, MAX_CSV_FILE_BYTES, MAX_XML_BYTES};
use curvebook_mcp::{McpServerState, call_tool};
use serde_json::json;

use super::common::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn create_sparse_file(path: &Path, byte_len: u64) -> TestResult {
    let f = File::create(path)?;
    f.set_len(byte_len)?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn r1_x1_03_mcp_open_file_rejects_fifo_and_devices_without_blocking() -> TestResult {
    let dir = TempDir::new("fifo")?;
    let fifo = dir.child("stream.csv");
    assert!(Command::new("mkfifo").arg(&fifo).status()?.success());
    let link = dir.child("stream_link.csv");
    std::os::unix::fs::symlink(&fifo, &link)?;

    for target in [fifo.clone(), link] {
        let path_str = target.display().to_string();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut state = McpServerState::new();
            let res = call_tool(&mut state, "open_file", json!({ "path": path_str }));
            let _ = tx.send(res);
        });
        let outcome = match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(res) => res.map_err(|err| format!("MCP open_file failed: {err:?}"))?,
            Err(err) => {
                if let Ok(f) = fs::OpenOptions::new().write(true).open(&fifo) {
                    drop(f);
                }
                return Err(format!("MCP open_file blocked on FIFO: {err}").into());
            }
        };
        assert!(outcome.is_error, "expected MCP open_file on FIFO to fail");
        let code = outcome.structured_content.as_ref().and_then(|v| v.get("code")).and_then(|v| v.as_str());
        assert_eq!(code, Some("bad_input"), "expected bad_input for FIFO, got {:?}", outcome.structured_content);
    }

    let subdir = dir.child("sub");
    fs::create_dir_all(&subdir)?;
    let mut state = McpServerState::new();
    let dir_res =
        call_tool(&mut state, "open_file", json!({ "path": subdir.display().to_string(), "format": "csv" })).map_err(|err| format!("{err:?}"))?;
    assert!(dir_res.is_error);
    let dir_code = dir_res.structured_content.as_ref().and_then(|v| v.get("code")).and_then(|v| v.as_str());
    assert_eq!(dir_code, Some("bad_input"));

    let dev_zero = Path::new("/dev/zero");
    if dev_zero.exists() {
        let zero_res = call_tool(&mut state, "open_file", json!({ "path": "/dev/zero", "format": "csv" })).map_err(|err| format!("{err:?}"))?;
        assert!(zero_res.is_error);
        let zero_code = zero_res.structured_content.as_ref().and_then(|v| v.get("code")).and_then(|v| v.as_str());
        assert_eq!(zero_code, Some("bad_input"));
    }

    Ok(())
}

#[test]
fn r1_x1_03_mcp_open_file_rejects_oversized_inputs_before_allocation() -> TestResult {
    let dir = TempDir::new("oversized")?;
    let mut state = McpServerState::new();

    for (name, cap) in [
        ("huge.csv", MAX_CSV_FILE_BYTES),
        ("huge.tsv", MAX_CSV_FILE_BYTES),
        ("huge.pzfx", MAX_XML_BYTES),
        ("huge.curvebook", MAX_ARCHIVE_FILE_BYTES),
        ("huge.cbtemplate", MAX_ARCHIVE_FILE_BYTES),
        ("huge.prism", MAX_ARCHIVE_FILE_BYTES),
    ] {
        let p = dir.child(name);
        create_sparse_file(&p, (cap as u64).saturating_add(1))?;
        let res = call_tool(&mut state, "open_file", json!({ "path": p.display().to_string() })).map_err(|err| format!("{err:?}"))?;
        assert!(res.is_error, "expected oversized {name} to fail");
        let code = res.structured_content.as_ref().and_then(|v| v.get("code")).and_then(|v| v.as_str());
        assert_eq!(code, Some("resource_limit"), "expected resource_limit for {name}, got {:?}", res.structured_content);
    }

    Ok(())
}
