//! CLI integration tests for Wave 3 Track 3B (`§4.2`, `§6.1`, `§6.3`):
//! - `convert` / `export` `.pzfx` data-only export (atomic write, stderr + JSON warnings, round-trip)
//! - `convert` / `export` / `batch` experimental `.prism` gating (`--experimental` / `experimental: true`)
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ConstantValue, DataSheet, GraphSheet, InfoSheet, ProjectDocument, ResultSheet, SheetId, SubcolumnMode,
    TableArchetype, XyTable,
};
use curvebook_format::registry::DESC_PRISM;
use curvebook_format::{compare_pzfx_data_layer, import_pzfx_bytes, save_project_file};
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

fn build_seed_project(dir: &TempDir) -> (ProjectDocument, std::path::PathBuf) {
    let mut doc = ProjectDocument::new("CLI Export Surface Study").unwrap();
    let mut info = InfoSheet::new_preseeded("info-1", "Assay Notes").unwrap();
    info.set_constant("Date", ConstantValue::Date(String::from("2026-08-15")), None).unwrap();
    doc.add_info_sheet(info).unwrap();

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Response");
    for (r, (x, y)) in [(1.0, 10.0), (10.0, 50.0), (100.0, 90.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds = DataSheet::new("xy-1", "Dose Curve", TableArchetype::Xy(xy)).unwrap();
    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("gr-1"), &ds).unwrap();
    gs.initialized = true;
    doc.add_data_sheet(ds).unwrap();
    doc.add_graph_sheet(gs).unwrap();
    let res = ResultSheet::new(AnalysisSpec::new("res-1", "xy-1", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap();
    doc.add_result_sheet(res).unwrap();

    let cb_path = dir.child("study.curvebook");
    save_project_file(&cb_path, &doc).unwrap();
    (doc, cb_path)
}

#[test]
fn cli_export_and_convert_write_pzfx_with_stderr_and_json_warnings_and_roundtrip() {
    let dir = TempDir::new("cli-pzfx-export").unwrap();
    let (doc, cb_path) = build_seed_project(&dir);
    let pzfx_text_out = dir.child("text_out.pzfx");
    let pzfx_json_out = dir.child("json_out.pzfx");

    // 1. Human mode prints ExportReport warnings to stderr and writes deterministically.
    let out_text = CliHarness::run(&["export", cb_path.to_str().unwrap(), pzfx_text_out.to_str().unwrap()]).unwrap();
    assert!(out_text.status.success(), "stderr={}", String::from_utf8_lossy(&out_text.stderr));
    let stderr_text = String::from_utf8_lossy(&out_text.stderr);
    assert!(stderr_text.contains("warning:"), "expected ExportReport warnings on stderr, got: {stderr_text}");
    assert!(stderr_text.contains("DroppedContent"), "expected dropped-content warning on stderr, got: {stderr_text}");

    // 2. JSON mode surfaces ExportReport warnings in `warnings` array and writes identical bytes.
    let out_json = CliHarness::run(&["convert", cb_path.to_str().unwrap(), pzfx_json_out.to_str().unwrap(), "--json"]).unwrap();
    assert!(out_json.status.success(), "stderr={}", String::from_utf8_lossy(&out_json.stderr));
    let payload: Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(payload["ok"], json!(true));
    assert_eq!(payload["format"], json!("pzfx"));
    let warnings = payload["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().is_some_and(|s| s.contains("DroppedContent"))),
        "expected DroppedContent warning in JSON warnings: {warnings:?}"
    );

    let bytes_a = fs::read(&pzfx_text_out).unwrap();
    let bytes_b = fs::read(&pzfx_json_out).unwrap();
    assert_eq!(bytes_a, bytes_b, ".pzfx CLI exports must be byte-deterministic");
    let reimported = import_pzfx_bytes(&bytes_a).unwrap();
    let diff = compare_pzfx_data_layer(&doc, &reimported.document);
    assert!(diff.is_identical(), "round-trip data-layer diff: {diff:?}");
}

#[test]
fn cli_export_and_batch_gate_experimental_archive_behind_explicit_flag() {
    let dir = TempDir::new("cli-exp-archive").unwrap();
    let (_doc, cb_path) = build_seed_project(&dir);
    let refused_path = dir.child("refused.prism");
    let allowed_path = dir.child("allowed.prism");

    // 1. Without `--experimental`, export fails cleanly pointing to `.pzfx` and writes nothing.
    let refused = CliHarness::run(&["export", cb_path.to_str().unwrap(), refused_path.to_str().unwrap(), "--json"]).unwrap();
    assert!(!refused.status.success(), "expected refusal without --experimental");
    assert!(!refused_path.exists());
    let err_json: Value = serde_json::from_slice(&refused.stdout).unwrap();
    assert!(err_json["error"]["message"].as_str().unwrap_or("").contains(".pzfx"), "refusal message should guide users to .pzfx: {err_json}");

    // 2. With `--experimental`, export succeeds, emits warnings, and `info` reads the output cleanly.
    let ok = CliHarness::run(&["export", cb_path.to_str().unwrap(), allowed_path.to_str().unwrap(), "--experimental", "--json"]).unwrap();
    assert!(ok.status.success(), "stderr={}", String::from_utf8_lossy(&ok.stderr));
    assert!(allowed_path.is_file());
    let ok_json: Value = serde_json::from_slice(&ok.stdout).unwrap();
    assert_eq!(ok_json["format"], json!(DESC_PRISM.id));
    assert!(!ok_json["warnings"].as_array().unwrap().is_empty());

    let info_out = CliHarness::run(&["info", allowed_path.to_str().unwrap(), "--json"]).unwrap();
    assert!(info_out.status.success(), "info failed on exported archive: {}", String::from_utf8_lossy(&info_out.stderr));

    // 3. Batch manifest: `.pzfx` export succeeds without flag; `.prism` fails without `experimental: true` and succeeds with it.
    let bad_manifest = dir.child("bad_batch.json");
    fs::write(
        &bad_manifest,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "project": cb_path.file_name().unwrap().to_str().unwrap(),
            "exports": [{ "out": "batch_refused.prism" }]
        }))
        .unwrap(),
    )
    .unwrap();
    let bad_batch = CliHarness::run(&["batch", bad_manifest.to_str().unwrap(), "--json"]).unwrap();
    assert!(!bad_batch.status.success(), "batch archive export without experimental: true must fail");
    assert!(!dir.child("batch_refused.prism").exists());

    let ok_manifest = dir.child("ok_batch.json");
    fs::write(
        &ok_manifest,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "project": cb_path.file_name().unwrap().to_str().unwrap(),
            "exports": [
                { "out": "batch_out.pzfx", "format": "pzfx" },
                { "out": "batch_out.prism", "experimental": true }
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    let good_batch = CliHarness::run(&["batch", ok_manifest.to_str().unwrap(), "--json"]).unwrap();
    assert!(good_batch.status.success(), "stderr={}", String::from_utf8_lossy(&good_batch.stderr));
    assert!(dir.child("batch_out.pzfx").is_file());
    assert!(dir.child("batch_out.prism").is_file());
    let batch_json: Value = serde_json::from_slice(&good_batch.stdout).unwrap();
    assert_eq!(batch_json["exports_written"], json!(2));
    assert!(!batch_json["warnings"].as_array().unwrap().is_empty());
}
