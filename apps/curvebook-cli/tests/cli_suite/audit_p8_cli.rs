use std::fs;
use std::io::{BufReader, Cursor};
use std::path::Path;
use std::process::Output;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, DataSheet, DescriptiveSpec, ErrorCode, LayoutSheet, LayoutSpec, PhysicalUnit,
    ProjectDocument, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::Session;
use curvebook_format::save_project_file;
use curvebook_ui_egui::{ControlAuth, UiApp, serve_control_stream_with_quit};
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

fn run_cli(args: &[&str]) -> Output {
    CliHarness::run(args).expect("execute curvebook-cli")
}

fn build_large_inspect_doc() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Control Inspect Parity Doc").expect("new doc");

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 250).expect("xy table");
    if let Some(ds0) = xy.data_sets.first_mut() {
        ds0.title = format!("LongSeries_{}", "X".repeat(320));
        for (r, (x_row, ds_row)) in xy.x_cells.iter_mut().zip(ds0.cells.iter_mut()).enumerate().take(250) {
            if let Some(x_cell) = x_row.first_mut() {
                *x_cell = Cell::number(r as f64);
            }
            if let Some(y_cell) = ds_row.first_mut() {
                *y_cell = Cell::text(format!("val_{r:03}_{}", "V".repeat(400)));
            }
        }
    }
    let big_ds = DataSheet::new("big_data", "Big Data Table", TableArchetype::Xy(xy)).expect("big_ds");
    doc.add_data_sheet(big_ds).expect("add big_data");

    let analysis = AnalysisSpec::new("big_result", "big_data", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis spec");
    let mut result = ResultSheet::new(analysis).expect("result sheet");
    result.status = ResultStatus::UpToDate;

    let tab0 = ResultTab {
        id: String::from("tab_rows"),
        title: String::from("Many Rows"),
        kind: ResultTabKind::MainSummary,
        column_headers: (0..10).map(|c| format!("C{c}")).collect(),
        row_labels: (0..180).map(|r| format!("R{r}")).collect(),
        rows: (0..180).map(|r| (0..10).map(|c| ResultCell::Number { value: (r * 10 + c) as f64, prefix: None, units: None }).collect()).collect(),
        footnotes: vec![],
    };
    result.tabs.push(tab0);

    let tab1 = ResultTab {
        id: String::from("tab_cols"),
        title: String::from("Many Cols"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: (0..80).map(|c| format!("WideCol{c}")).collect(),
        row_labels: (0..25).map(|r| format!("Row{r}")).collect(),
        rows: (0..25).map(|r| (0..80).map(|c| ResultCell::Number { value: (r * 80 + c) as f64, prefix: None, units: None }).collect()).collect(),
        footnotes: vec![],
    };
    result.tabs.push(tab1);

    for t in 2..35 {
        result.tabs.push(ResultTab {
            id: format!("tab_{t}"),
            title: format!("Extra {t}"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("K"), String::from("V")],
            row_labels: vec![format!("row_{t}")],
            rows: vec![vec![ResultCell::Text(format!("k{t}")), ResultCell::Number { value: t as f64, prefix: None, units: None }]],
            footnotes: vec![],
        });
    }
    for c in 0..150 {
        result.checklist.push(ChecklistItem {
            assumption: format!("Check #{c}"),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: String::from("ok"),
            remediation: None,
        });
    }
    for s in 0..2200 {
        result.scalars.push(ResultScalarEntry { key: format!("scalar_{s:04}"), data_set_index: None, value: s as f64, units: None });
    }
    doc.add_result_sheet(result).expect("add big_result");
    doc
}

#[test]
fn fa_sec21_03_control_io_doc_inspect_bounds_data_and_results() {
    let mut app = UiApp::new();
    *app.session_mut() = Session::from_document(build_large_inspect_doc());

    let token = "audit-sec21-03-control-token-abcdef012345";
    let auth = ControlAuth::from_token(token).expect("valid token");

    let reqs = [
        json!({ "id": "auth", "method": "auth", "token": token }),
        json!({ "id": "data", "method": "doc.inspect", "params": { "sheet_id": "big_data", "include_cells": true } }),
        json!({ "id": "result", "method": "doc.inspect", "params": { "sheet_id": "big_result", "include_cells": true } }),
        json!({ "id": "summary", "method": "doc.inspect", "params": {} }),
        json!({ "id": "quit", "method": "app.quit", "params": {} }),
    ];
    let mut input = String::new();
    for req in reqs {
        input.push_str(&serde_json::to_string(&req).expect("serialize req"));
        input.push('\n');
    }

    let mut reader = BufReader::new(Cursor::new(input.into_bytes()));
    let mut output = Vec::new();
    let quit = serve_control_stream_with_quit(&mut app, &mut reader, &mut output, &auth).expect("serve control stream");
    assert!(quit, "expected app.quit to stop control stream");

    let out_str = String::from_utf8(output).expect("valid UTF-8 control responses");
    let responses: Vec<Value> =
        out_str.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str::<Value>(l).expect("parse response JSON")).collect();
    assert_eq!(responses.len(), 5, "expected 5 control responses");

    let data_resp = &responses[1];
    assert_eq!(data_resp["ok"], json!(true), "data response: {data_resp}");
    let data_res = &data_resp["result"];
    assert_eq!(data_res["rows"], json!(250));
    assert_eq!(data_res["csv_preview_truncated"], json!(true));
    let csv_preview = data_res["csv_preview"].as_str().expect("csv_preview");
    assert!(csv_preview.len() <= 65_536, "control_io csv_preview exceeded 64 KiB cap: {}", csv_preview.len());
    assert!(csv_preview.lines().count() <= 101, "control_io csv_preview exceeded 100 data rows: {}", csv_preview.lines().count());

    let result_resp = &responses[2];
    assert_eq!(result_resp["ok"], json!(true), "result response: {result_resp}");
    let res_obj = &result_resp["result"];
    assert_eq!(res_obj["tabs"], json!(35));
    assert_eq!(res_obj["tabs_truncated"], json!(true));
    let tabs = res_obj["tabs_data"].as_array().expect("tabs_data array");
    assert_eq!(tabs.len(), 32);
    assert!(tabs[0]["rows"].as_array().map(Vec::len).unwrap_or(999) <= 100);
    assert!(tabs[1]["column_headers"].as_array().map(Vec::len).unwrap_or(999) <= 64);
    assert_eq!(res_obj["checklist"], json!(150));
    assert_eq!(res_obj["checklist_truncated"], json!(true));
    assert_eq!(res_obj["checklist_entries"].as_array().map(Vec::len), Some(128));
    assert_eq!(res_obj["scalars"], json!(2200));
    assert_eq!(res_obj["scalars_truncated"], json!(true));
    assert_eq!(res_obj["scalars_data"].as_array().map(Vec::len), Some(2048));

    let summary_resp = &responses[3];
    assert_eq!(summary_resp["ok"], json!(true), "summary response: {summary_resp}");
    let result_sheets = summary_resp["result"]["result_sheets"].as_array().expect("result_sheets summary");
    assert_eq!(result_sheets.len(), 1);
    assert_eq!(result_sheets[0]["tabs"], json!(35));
    assert_eq!(result_sheets[0]["scalars"], json!(2200));
}

#[test]
fn fa_sec21_06_filesystem_write_commands_do_not_precreate_empty_dirs_on_failure() {
    let dir = TempDir::new("sec21-06").unwrap();
    let wd = dir.path().join("workdir");
    fs::create_dir_all(&wd).expect("create workdir");
    let wd_str = wd.display().to_string();

    // 1. Failing `run` command (`result.generateReport` on non-existent sheet_id) must NOT create `never_run/`.
    let script_path = dir.path().join("fail_run.ndjson");
    let script_line = json!({
        "command": "result.generateReport",
        "params": {
            "result_sheet_id": "missing_result",
            "path": "never_run/nested/report.md"
        }
    });
    fs::write(&script_path, format!("{script_line}\n")).expect("write run script");
    let script_str = script_path.display().to_string();
    let run_fail = run_cli(&["run", &script_str, "--workdir", &wd_str]);
    assert!(!run_fail.status.success(), "failing `run` must exit non-zero");
    assert!(!wd.join("never_run").exists(), "`run` prematurely created parent dir on failed command");

    // 2. Failing `batch` command (`export.svg` on non-existent graph) must NOT create `never_batch/`.
    let batch_fail_path = dir.path().join("fail_batch.json");
    let batch_fail = json!({
        "schema": "curvebook.batch.v1",
        "commands": [
            {
                "cmd": "export.svg",
                "params": {
                    "sheet_id": "missing_graph",
                    "path": "never_batch/nested/fig.svg"
                }
            }
        ],
        "exports": []
    });
    fs::write(&batch_fail_path, serde_json::to_vec_pretty(&batch_fail).expect("json")).expect("write fail batch");
    let batch_fail_str = batch_fail_path.display().to_string();
    let batch_out = run_cli(&["batch", &batch_fail_str, "--workdir", &wd_str]);
    assert!(!batch_out.status.success(), "failing `batch` must exit non-zero");
    assert!(!wd.join("never_batch").exists(), "`batch` prematurely created parent dir on failed command");

    // 3. Failing direct CLI command (`export.svg`) must NOT create `never_direct/`.
    let direct_fail = run_cli(&["export.svg", "--workdir", &wd_str, "--sheet-id", "missing_graph", "--path", "never_direct/nested/fig.svg"]);
    assert!(!direct_fail.status.success(), "failing direct CLI command must exit non-zero");
    assert!(!wd.join("never_direct").exists(), "direct command prematurely created parent dir on failed command");

    // 4. Successful `project.save` with alias `file_path` inside `--workdir` creates nested dir atomically on success.
    let batch_ok_path = dir.path().join("ok_batch.json");
    let batch_ok = json!({
        "schema": "curvebook.batch.v1",
        "commands": [
            {
                "cmd": "sheet.createData",
                "params": { "sheet_id": "tbl_ok", "title": "OK", "kind": "xy" }
            },
            {
                "cmd": "project.save",
                "params": { "file_path": "saved_ok/sub/out.curvebook" }
            }
        ],
        "exports": []
    });
    fs::write(&batch_ok_path, serde_json::to_vec_pretty(&batch_ok).expect("json")).expect("write ok batch");
    let batch_ok_str = batch_ok_path.display().to_string();
    let ok_out = run_cli(&["batch", &batch_ok_str, "--workdir", &wd_str]);
    assert!(
        ok_out.status.success(),
        "valid batch with nested project.save failed: stdout={}, stderr={}",
        String::from_utf8_lossy(&ok_out.stdout),
        String::from_utf8_lossy(&ok_out.stderr)
    );
    assert!(wd.join("saved_ok/sub/out.curvebook").is_file(), "valid project.save must atomically write nested archive");
}

#[test]
fn fa_x1_05_raster_target_alignment_and_dpi_bounds() {
    let dir = TempDir::new("x1-05").unwrap();

    // 1. Valid normal layout archive to test DPI lower/upper bounds (18..=2400).
    let mut normal_doc = ProjectDocument::new("Normal Layout").expect("normal doc");
    let normal_layout = LayoutSheet::new("lay_ok", "Normal Page", LayoutSpec::default()).expect("normal layout");
    normal_doc.add_layout_sheet(normal_layout).expect("add normal layout");
    let normal_archive = dir.path().join("normal.curvebook");
    save_project_file(&normal_archive, &normal_doc).expect("save normal archive");
    let normal_str = normal_archive.display().to_string();

    for bad_dpi in ["17", "10", "0", "2401"] {
        let out_png = dir.path().join(format!("bad_{bad_dpi}.png"));
        let out_str = out_png.display().to_string();
        let res = run_cli(&["convert", &normal_str, &out_str, "--sheet", "lay_ok", "--dpi", bad_dpi]);
        assert!(!res.status.success(), "convert accepted out-of-range --dpi {bad_dpi}");
        assert!(!out_png.exists(), "failed dpi {bad_dpi} must not create output PNG");
    }

    let ok_png = dir.path().join("ok_18.png");
    let ok_png_str = ok_png.display().to_string();
    let ok_res = run_cli(&["convert", &normal_str, &ok_png_str, "--sheet", "lay_ok", "--dpi", "18"]);
    assert!(ok_res.status.success(), "convert at --dpi 18 failed: stderr={}", String::from_utf8_lossy(&ok_res.stderr));
    assert!(ok_png.is_file(), "expected valid 18 DPI PNG output");

    // Batch manifest export with DPI 17 must also fail cleanly.
    let bad_dpi_manifest = dir.path().join("bad_dpi_batch.json");
    let manifest = json!({
        "schema": "curvebook.batch.v1",
        "project": &normal_str,
        "exports": [
            { "sheet": "lay_ok", "format": "png", "dpi": 17, "out": "never_dpi17.png" }
        ]
    });
    fs::write(&bad_dpi_manifest, serde_json::to_vec_pretty(&manifest).expect("json")).expect("write bad dpi manifest");
    let wd_str = dir.path().display().to_string();
    let bad_dpi_manifest_str = bad_dpi_manifest.display().to_string();
    let batch_res = run_cli(&["batch", &bad_dpi_manifest_str, "--workdir", &wd_str]);
    assert!(!batch_res.status.success(), "batch accepted out-of-range dpi 17");
    assert!(!Path::new(&dir.path().join("never_dpi17.png")).exists());

    // 2. Extreme aspect-ratio layout (72000 x 720 pt = 1000 x 10 in) must fail rather than distorting.
    let mut extreme_doc = ProjectDocument::new("Extreme Aspect Doc").expect("extreme doc");
    let extreme_spec = LayoutSpec { width: 72_000.0, height: 720.0, unit: PhysicalUnit::Points, ..LayoutSpec::default() };
    let extreme_layout = LayoutSheet::new("lay_extreme", "Extreme Banner", extreme_spec).expect("extreme layout");
    extreme_doc.add_layout_sheet(extreme_layout).expect("add extreme layout");
    let extreme_archive = dir.path().join("extreme.curvebook");
    save_project_file(&extreme_archive, &extreme_doc).expect("save extreme archive");
    let extreme_str = extreme_archive.display().to_string();

    let extreme_png = dir.path().join("extreme.png");
    let extreme_png_str = extreme_png.display().to_string();
    let conv_extreme = run_cli(&["convert", &extreme_str, &extreme_png_str, "--sheet", "lay_extreme", "--dpi", "72"]);
    assert!(!conv_extreme.status.success(), "convert must reject unrenderable extreme raster target instead of distorting");
    assert!(!extreme_png.exists(), "rejected extreme raster target must not write distorted PNG");

    let bench_extreme = run_cli(&["bench", &extreme_str]);
    assert!(!bench_extreme.status.success(), "bench must reject extreme raster target instead of clamping independently");
}

#[test]
fn fa_sec04_04_cli_convert_and_export_refuse_failed_result_sheet_csv_and_tsv() {
    let dir = TempDir::new("sec04-04-cli").unwrap();
    let mut doc = ProjectDocument::new("Failed Result Export Doc").expect("new doc");
    let xy = XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy");
    let ds = DataSheet::new("d1", "Data 1", TableArchetype::Xy(xy)).expect("ds");
    doc.add_data_sheet(ds).expect("add data");

    let analysis = AnalysisSpec::new("res_fail", "d1", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis");
    let mut rs = ResultSheet::new(analysis).expect("result");
    rs.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("singular matrix") };
    rs.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Mean"), String::from("SD")],
        row_labels: vec![String::from("Row1")],
        rows: vec![vec![ResultCell::Number { value: 3.0, prefix: None, units: None }, ResultCell::Number { value: 0.5, prefix: None, units: None }]],
        footnotes: vec![],
    });
    doc.add_result_sheet(rs).expect("add failed result");

    let archive = dir.path().join("failed_result.curvebook");
    save_project_file(&archive, &doc).expect("save archive");
    let archive_str = archive.display().to_string();

    let out_csv = dir.path().join("stale_export.csv");
    let out_csv_str = out_csv.display().to_string();
    let res_csv = run_cli(&["convert", &archive_str, &out_csv_str, "--sheet", "res_fail"]);
    assert!(!res_csv.status.success(), "CLI convert CSV must refuse Failed ResultSheet");
    assert!(!out_csv.exists(), "refused convert CSV must not create output file");

    let out_tsv = dir.path().join("stale_export.tsv");
    let out_tsv_str = out_tsv.display().to_string();
    let res_tsv = run_cli(&["export", &archive_str, &out_tsv_str, "--format", "tsv", "--sheet", "res_fail"]);
    assert!(!res_tsv.status.success(), "CLI export TSV must refuse Failed ResultSheet");
    assert!(!out_tsv.exists(), "refused export TSV must not create output file");
}
