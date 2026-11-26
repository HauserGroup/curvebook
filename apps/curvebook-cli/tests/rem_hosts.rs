//! Wave 2 remediation regression suite for `curvebook-cli` (`fix-hosts`).
//!
//! Covers `AUD-06-11`, `AUD-06-15`, and `AUD-13-23`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype};
use curvebook_format::save_project_file;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!("cb-rem-hosts-cli-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_curvebook-cli")
}

#[test]
fn aud_06_11_cli_info_reports_column_subcolumn_mode() -> TestResult {
    let dir = TempDir::new()?;
    let mut doc = ProjectDocument::new("Column Summary Study")?;
    let mut col = ColumnTable::new(ColumnDesignHint::PrecomputedError, 2, 2)?;
    col.subcolumn_mode = SubcolumnMode::MeanSdN;
    doc.add_data_sheet(DataSheet::new("col-1", "Summary Column", TableArchetype::Column(col))?)?;

    let proj = dir.path().join("column_summary.curvebook");
    save_project_file(&proj, &doc)?;

    let out = Command::new(cli_bin()).args(["info", &proj.display().to_string(), "--json"]).output()?;
    assert!(out.status.success(), "cli info failed: {}", String::from_utf8_lossy(&out.stderr));
    let parsed: Value = serde_json::from_slice(&out.stdout)?;
    assert_eq!(parsed["tables"][0]["subcolumn_mode"], json!("MeanSdN"));
    Ok(())
}

#[test]
fn aud_06_15_cli_run_rejects_unknown_keys_and_batch_propagates_project_warnings() -> TestResult {
    let dir = TempDir::new()?;

    // 1. `curvebook-cli run` rejects unknown keys like `param` typo with exit code 2.
    let bad_script = dir.path().join("bad_script.json");
    fs::write(
        &bad_script,
        serde_json::to_vec(&json!([
            { "cmd": "sheet.createData", "param": { "kind": "Column" } }
        ]))?,
    )?;
    let run_out = Command::new(cli_bin()).args(["run", &bad_script.display().to_string(), "--json"]).output()?;
    assert_eq!(run_out.status.code(), Some(2), "unknown command key must fail with schema exit code 2: {}", String::from_utf8_lossy(&run_out.stdout));

    // 2. `curvebook-cli batch` propagates `loaded.warnings` from base `project` into stderr and JSON `warnings`.
    // Create a `.pzfx` file containing an unsupported/unparsed XML construct that produces an import warning.
    let pzfx_path = dir.path().join("warn_base.pzfx");
    fs::write(
        &pzfx_path,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <TableSequence>
    <Table ID="Table0" TableType="OneWay">
      <Title>Data 1</Title>
      <YColumn Width="81" Decimals="1" Subcolumns="1">
        <Title>Ctrl</Title>
        <Subcolumn><d>1.0</d><d>2.0</d></Subcolumn>
      </YColumn>
    </Table>
  </TableSequence>
  <Graph ID="Graph0"><Title>Unsupported Legacy Graph</Title></Graph>
</PrismFile>"#,
    )?;

    let manifest_path = dir.path().join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "project": "warn_base.pzfx",
            "exports": [{ "format": "curvebook", "out": "out.curvebook" }]
        }))?,
    )?;

    let batch_out = Command::new(cli_bin()).args(["batch", &manifest_path.display().to_string(), "--json"]).output()?;
    assert!(batch_out.status.success(), "batch failed: stderr={}", String::from_utf8_lossy(&batch_out.stderr));
    let batch_json: Value = serde_json::from_slice(&batch_out.stdout)?;
    let warnings = batch_json["warnings"].as_array().ok_or("expected warnings array from base project")?;
    assert!(!warnings.is_empty(), "base project warnings must be propagated to batch JSON output");
    let stderr_text = String::from_utf8_lossy(&batch_out.stderr);
    assert!(stderr_text.contains("warning:"), "base project warnings must also be printed to stderr: {stderr_text}");
    Ok(())
}

#[test]
fn aud_13_23_cli_sandbox_sanitizes_extra_path_keys() -> TestResult {
    let dir = TempDir::new()?;
    let wd = dir.path().join("wd");
    fs::create_dir_all(&wd)?;

    for key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path"] {
        let script = wd.join(format!("escape_{key}.json"));
        let mut params = serde_json::Map::new();
        params.insert(key.to_owned(), json!("../outside.target"));
        fs::write(
            &script,
            serde_json::to_vec(&json!([
                { "cmd": "project.save", "params": Value::Object(params) }
            ]))?,
        )?;
        let out = Command::new(cli_bin()).args(["run", &script.display().to_string(), "--workdir", &wd.display().to_string(), "--json"]).output()?;
        assert_eq!(out.status.code(), Some(2), "extra path key `{key}` escape must fail with exit 2: {}", String::from_utf8_lossy(&out.stdout));
    }
    Ok(())
}
