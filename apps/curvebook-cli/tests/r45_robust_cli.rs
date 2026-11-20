#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_engine::Session;
use curvebook_format::save_project_file;
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-cli-r45-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_curvebook-cli")
}

fn run_cli(args: &[&str]) -> Output {
    Command::new(cli_bin()).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).output().unwrap()
}

#[test]
fn r45_cli_info_and_convert_render_robust_not_applicable_status() {
    let dir = TempDir::new("info");
    let proj_path = dir.path().join("robust_study.curvebook");

    let mut session = Session::new();
    session
        .execute(
            "sheet.createData",
            json!({
                "sheet_id": "xy-1",
                "title": "Decay",
                "kind": "xy",
                "rows": 7,
                "cols": 1,
                "replicates": 1,
                "auto_graph": false
            }),
        )
        .unwrap();
    session
        .execute(
            "table.setXCells",
            json!({
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "value": 0.0 },
                    { "row": 1, "value": 1.0 },
                    { "row": 2, "value": 2.0 },
                    { "row": 3, "value": 4.0 },
                    { "row": 4, "value": 8.0 },
                    { "row": 5, "value": 12.0 }
                ]
            }),
        )
        .unwrap();
    session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "col": 0, "subcol": 0, "value": 100.0 },
                    { "row": 1, "col": 0, "subcol": 0, "value": 74.1 },
                    { "row": 2, "col": 0, "subcol": 0, "value": 54.9 },
                    { "row": 3, "col": 0, "subcol": 0, "value": 30.1 },
                    { "row": 4, "col": 0, "subcol": 0, "value": 9.1 },
                    { "row": 5, "col": 0, "subcol": 0, "value": 2.7 },
                    { "row": 6, "col": 0, "subcol": 0, "value": 50.0 }
                ]
            }),
        )
        .unwrap();
    session
        .execute(
            "fit.run",
            json!({
                "sheet_id": "fit-robust",
                "source_sheet_id": "xy-1",
                "kind": "one_phase_decay",
                "method": "robust",
                "confidence_bands": true,
                "prediction_bands": true,
                "emit_covariance_tab": true,
                "interpolate_unknowns": true,
                "comparison": { "method": "f_test", "alternative_equation_id": "straight_line" }
            }),
        )
        .unwrap();

    save_project_file(&proj_path, session.document()).unwrap();

    let path_str = proj_path.to_str().unwrap();

    // 1. Human `info` prints explicit "not applicable (Robust mode)" marker.
    let human_out = run_cli(&["info", path_str]);
    assert!(human_out.status.success(), "stderr: {}", String::from_utf8_lossy(&human_out.stderr));
    let human_txt = String::from_utf8(human_out.stdout).unwrap();
    assert!(human_txt.contains("inference: not applicable (Robust mode)"), "human output: {human_txt}");

    // 2. JSON `info --json` reports NotApplicableInRobustMode, RSDR scalar, and "Not applicable" inference cells.
    let json_out = run_cli(&["info", "--json", path_str]);
    assert!(json_out.status.success(), "stderr: {}", String::from_utf8_lossy(&json_out.stderr));
    let parsed: Value = serde_json::from_slice(&json_out.stdout).unwrap();
    let analysis = &parsed["analyses"][0];
    assert_eq!(analysis["inference_availability"], json!("NotApplicableInRobustMode"));
    assert!(analysis["scalars"]["RSDR"].as_f64().is_some_and(f64::is_finite));
    let raw_analysis = serde_json::to_string(analysis).unwrap();
    assert!(raw_analysis.contains("\"Not applicable\""));
    assert!(!analysis["tabs"].as_array().unwrap().iter().any(|t| t["id"] == "nonlinear_covariance"));

    // 3. `convert` to CSV and JSON exports cleanly with "Not applicable" cells.
    let csv_path = dir.path().join("robust_out.csv");
    let conv_csv = run_cli(&["convert", path_str, csv_path.to_str().unwrap(), "--sheet", "fit-robust"]);
    assert!(conv_csv.status.success(), "convert csv failed: {}", String::from_utf8_lossy(&conv_csv.stderr));
    let csv_body = fs::read_to_string(&csv_path).unwrap();
    assert!(csv_body.contains("Not applicable"));
    assert!(csv_body.contains("RSDR"));

    let json_path = dir.path().join("robust_out.json");
    let conv_json = run_cli(&["convert", path_str, json_path.to_str().unwrap(), "--sheet", "fit-robust", "--format", "json"]);
    assert!(conv_json.status.success(), "convert json failed: {}", String::from_utf8_lossy(&conv_json.stderr));
    let json_body = fs::read_to_string(&json_path).unwrap();
    assert!(json_body.contains("\"Not applicable\""));
    assert!(json_body.contains("NotApplicableInRobustMode"));
}
