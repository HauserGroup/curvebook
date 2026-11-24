//! Regression tests for `R1-S12-10` (Stage 4C S4):
//! - `docs/implementation/scripting-and-automation.md` matches the shipped `curvebook-cli` subcommands,
//!   `curvebook.batch.v1` manifest schema (`docs/schemas/batch-v1.schema.json`), and `CliError::PathEscape`.
//! - `curvebook-cli batch` executes the documented manifest format end-to-end (replacing matching
//!   data sheets by title and exporting `json`, `csv`, `pdf`, and `curvebook`) and rejects stale
//!   legacy guide keys (`exports[].kind`, `inputs[].mode`) and unknown fields with `CliError::Schema`
//!   (exit code `2`).
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use curvebook_format::schemas::batch_v1_schema;
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repo_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.parent().and_then(Path::parent).ok_or("failed to resolve workspace root from CARGO_MANIFEST_DIR")?;
    Ok(root.to_path_buf())
}

fn run_cli(args: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    Ok(CliHarness::run(args)?)
}

fn run_cli_ok_json(args: &[&str]) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(CliHarness::run_ok_json(args)?)
}

fn assert_json_cli_error(
    args: &[&str],
    expected_exit: i32,
    expected_code: &str,
    expected_category: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(CliHarness::assert_json_error(args, expected_exit, expected_code, expected_category)?)
}

fn extract_first_json_code_block(markdown: &str) -> Result<&str, Box<dyn std::error::Error>> {
    let marker = "```json\n";
    let (_, rest) = markdown.split_once(marker).ok_or("missing ```json fenced block in docs/implementation/scripting-and-automation.md")?;
    let (block, _) = rest.split_once("```").ok_or("unclosed ```json fenced block in docs/implementation/scripting-and-automation.md")?;
    Ok(block)
}

fn object_property_keys(schema_node: &Value) -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let props = schema_node.get("properties").and_then(Value::as_object).ok_or("missing `properties` object in schema node")?;
    Ok(props.keys().cloned().collect())
}

#[test]
fn r1_s12_10_scripting_guide_matches_cli_manifest_and_error_contract() -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root()?;
    let guide = fs::read_to_string(root.join("docs/implementation/scripting-and-automation.md"))?;
    let main_src = fs::read_to_string(root.join("apps/curvebook-cli/src/main.rs"))?;
    let error_src = fs::read_to_string(root.join("apps/curvebook-cli/src/error.rs"))?;
    let schema_disk: Value = serde_json::from_str(&fs::read_to_string(root.join("docs/schemas/batch-v1.schema.json"))?)?;
    let schema_live = batch_v1_schema();
    assert_eq!(schema_disk, schema_live, "docs/schemas/batch-v1.schema.json must match batch_v1_schema()");

    // 1. Stale legacy guide drift must be gone.
    for forbidden in ["PathOutsideWorkdir", "results_json", "results_csv", "\"mode\": \"replace_data\"", "\"kind\": \"pdf\"", "\"kind\": \"project\""]
    {
        assert!(!guide.contains(forbidden), "docs/implementation/scripting-and-automation.md still contains stale `{forbidden}`");
    }
    for removed_subcmd in ["`import`", "`analyze`", "`verify`"] {
        assert!(
            !guide.contains(removed_subcmd),
            "docs/implementation/scripting-and-automation.md still documents removed subcommand {removed_subcmd}"
        );
    }

    // 2. Documented subcommands and error variant match main.rs & error.rs.
    for subcmd in ["run", "batch", "convert", "export", "info", "commands", "bench", "perf", "mcp"] {
        assert!(guide.contains(&format!("`{subcmd}`")), "guide should document `{subcmd}` subcommand");
        assert!(main_src.contains(&format!("Some(\"{subcmd}\")")), "main.rs dispatch missing `{subcmd}`");
    }
    assert!(error_src.contains("PathEscape(String)"), "error.rs should define CliError::PathEscape");
    assert!(!error_src.contains("PathOutsideWorkdir"));
    assert!(guide.contains("CliError::PathEscape"), "guide must reference CliError::PathEscape");
    assert!(guide.contains("CliError::Schema"), "guide must reference CliError::Schema");

    // 3. Example manifest in docs/implementation/scripting-and-automation.md conforms strictly to batch-v1.schema.json.
    let example_raw = extract_first_json_code_block(&guide)?;
    let example: Value = serde_json::from_str(example_raw)?;
    let root_obj = example.as_object().ok_or("guide example manifest must be a JSON object")?;
    let root_allowed = object_property_keys(&schema_disk)?;
    for key in root_obj.keys() {
        assert!(root_allowed.contains(key), "guide example root key `{key}` is not in batch-v1.schema.json");
    }
    assert_eq!(example["schema"].as_str(), Some("curvebook.batch.v1"));
    assert!(!(root_obj.contains_key("template") && root_obj.contains_key("project")));

    let input_allowed = object_property_keys(&schema_disk["$defs"]["BatchInput"])?;
    for input in example["inputs"].as_array().ok_or("guide example missing `inputs` array")? {
        let obj = input.as_object().ok_or("guide example input must be an object")?;
        for key in obj.keys() {
            assert!(input_allowed.contains(key), "guide example input key `{key}` not in BatchInput schema");
        }
    }

    let cmd_allowed = object_property_keys(&schema_disk["$defs"]["BatchCommand"])?;
    for cmd in example["commands"].as_array().ok_or("guide example missing `commands` array")? {
        let obj = cmd.as_object().ok_or("guide example command must be an object")?;
        for key in obj.keys() {
            assert!(cmd_allowed.contains(key), "guide example command key `{key}` not in BatchCommand schema");
        }
    }

    let export_allowed = object_property_keys(&schema_disk["$defs"]["BatchExport"])?;
    let format_enum: BTreeSet<String> = schema_disk["$defs"]["BatchExport"]["properties"]["format"]["enum"]
        .as_array()
        .ok_or("BatchExport format enum missing")?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for exp in example["exports"].as_array().ok_or("guide example missing `exports` array")? {
        let obj = exp.as_object().ok_or("guide example export must be an object")?;
        for key in obj.keys() {
            assert!(export_allowed.contains(key), "guide example export key `{key}` not in BatchExport schema");
        }
        if let Some(fmt) = obj.get("format").and_then(Value::as_str) {
            assert!(format_enum.contains(fmt), "guide example export format `{fmt}` not in BatchExport format enum");
        }
    }

    Ok(())
}

#[test]
fn r1_s12_10_documented_batch_manifest_executes_and_replaces_sheet_end_to_end() -> TestResult {
    let dir = TempDir::new("e2e")?;
    let workdir_str = dir.path().to_str().ok_or("non-UTF-8 temp dir")?;

    // 1. Create a reusable template matching the guide's `"Dose Response"` and `"Summary Layout"` titles.
    let tpl_script = dir.child("setup_template.jsonl");
    let tpl_lines = [
        json!({
            "cmd": "sheet.createData",
            "params": {
                "sheet_id": "dose_1",
                "title": "Dose Response",
                "kind": "xy",
                "rows": 4,
                "cols": 1,
                "auto_info": true,
                "info_sheet_id": "info_1"
            }
        })
        .to_string(),
        json!({ "cmd": "graph.create", "params": { "sheet_id": "g_1", "data_sheet_id": "dose_1", "title": "Response Graph" } }).to_string(),
        json!({ "cmd": "graph.initialize", "params": { "sheet_id": "g_1", "kind": "xy" } }).to_string(),
        json!({ "cmd": "layout.create", "params": { "sheet_id": "layout_1", "title": "Summary Layout" } }).to_string(),
        json!({ "cmd": "project.saveTemplate", "params": { "path": "assay-4pl.cbtemplate" } }).to_string(),
    ]
    .join("\n");
    fs::write(&tpl_script, format!("{tpl_lines}\n"))?;
    let tpl_script_str = tpl_script.to_str().ok_or("non-UTF-8 script path")?;
    let setup_res = run_cli_ok_json(&["run", tpl_script_str, "--workdir", workdir_str, "--json"])?;
    assert_eq!(setup_res["ok"], json!(true));

    // 2. Write plate CSV inside `plates/plate_01.csv`.
    fs::create_dir_all(dir.child("plates"))?;
    fs::write(dir.child("plates/plate_01.csv"), "LogConc,Response\n-9.0,10.2\n-8.0,12.5\n-7.0,32.0\n-6.5,55.0\n-6.0,78.0\n-5.0,96.5\n-4.0,99.1\n")?;

    // 3. Execute a `curvebook.batch.v1` manifest following the documented schema.
    let manifest_path = dir.child("manifest.json");
    let manifest = json!({
        "schema": "curvebook.batch.v1",
        "template": "assay-4pl.cbtemplate",
        "inputs": [
            { "sheet": "Dose Response", "csv": "plates/plate_01.csv", "kind": "xy" }
        ],
        "commands": [
            { "cmd": "info.setConstant", "params": { "name": "PlateID", "value": "P01" } },
            { "cmd": "fit.run", "params": { "source_sheet_id": "dose_1", "sheet_id": "fit_1", "title": "4PL Fit", "equation_id": "log_agonist_vs_response_4pl" } }
        ],
        "exports": [
            { "format": "json", "sheet": "4PL Fit", "out": "out/plate_01_fit.json" },
            { "format": "csv", "sheet": "4PL Fit", "out": "out/plate_01_params.csv", "tab_index": 0 },
            { "format": "pdf", "sheet": "Summary Layout", "out": "out/plate_01_figure.pdf" },
            { "format": "curvebook", "out": "out/plate_01.curvebook" }
        ]
    });
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    let manifest_str = manifest_path.to_str().ok_or("non-UTF-8 manifest path")?;
    let batch_res = run_cli_ok_json(&["batch", manifest_str, "--workdir", workdir_str, "--json"])?;
    assert_eq!(batch_res["ok"], json!(true));
    assert_eq!(batch_res["inputs_loaded"], json!(1));
    assert_eq!(batch_res["exports_written"], json!(4));

    let fit_json: Value = serde_json::from_slice(&fs::read(dir.child("out/plate_01_fit.json"))?)?;
    assert!(fit_json.is_object(), "exported result JSON must be a JSON object");

    let params_csv = fs::read_to_string(dir.child("out/plate_01_params.csv"))?;
    assert!(!params_csv.trim().is_empty(), "exported result tab CSV must be non-empty");

    let pdf_bytes = fs::read(dir.child("out/plate_01_figure.pdf"))?;
    assert!(pdf_bytes.starts_with(b"%PDF-"), "exported figure must start with %PDF-");

    // 4. Verify `"Dose Response"` was replaced in place (1 table, not 2) and test `curvebook-cli export`.
    let out_project = dir.child("out/plate_01.curvebook");
    let out_project_str = out_project.to_str().ok_or("non-UTF-8 project path")?;
    let info_res = run_cli_ok_json(&["info", out_project_str, "--json"])?;
    assert_eq!(info_res["counts"]["tables"], json!(1), "matching sheet title must replace existing data sheet rather than appending");
    assert_eq!(info_res["tables"][0]["rows"], json!(7));

    let exported_svg = dir.child("out/graph_via_export.svg");
    let exported_svg_str = exported_svg.to_str().ok_or("non-UTF-8 svg path")?;
    let export_res = run_cli_ok_json(&["export", out_project_str, "--out", exported_svg_str, "--sheet", "g_1", "--json"])?;
    assert_eq!(export_res["ok"], json!(true));
    assert_eq!(export_res["format"], json!("svg"));
    assert!(fs::read_to_string(&exported_svg)?.contains("<svg"));

    Ok(())
}

#[test]
fn r1_s12_10_rejects_legacy_manifest_keys_removed_subcommands_and_path_escape() -> TestResult {
    let dir = TempDir::new("errors")?;
    let outside = TempDir::new("outside")?;
    let workdir_str = dir.path().to_str().ok_or("non-UTF-8 temp dir")?;

    fs::write(dir.child("data.csv"), "X,Y\n1,2\n3,4\n")?;

    // 1. Legacy `exports[].kind` (with or without recognizable extension) is rejected as a schema error (exit 2).
    for (name, out_rel) in [("legacy_kind_txt.json", "out/params.txt"), ("legacy_kind_csv.json", "out/params.csv")] {
        let path = dir.child(name);
        let doc = json!({
            "schema": "curvebook.batch.v1",
            "inputs": [{ "sheet": "d1", "csv": "data.csv", "kind": "xy" }],
            "exports": [{ "kind": "results_csv", "sheet": "d1", "out": out_rel }]
        });
        fs::write(&path, serde_json::to_vec(&doc)?)?;
        let path_str = path.to_str().ok_or("non-UTF-8 path")?;
        let err = assert_json_cli_error(&["batch", path_str, "--workdir", workdir_str, "--json"], 2, "bad_input", "input")?;
        let msg = err["error"]["message"].as_str().unwrap_or("");
        assert!(msg.contains("`kind`") && msg.contains("`format`"), "expected actionable hint for exports[].kind, got `{msg}`");
    }

    // 2. Legacy `inputs[].mode` and unknown root/command fields are rejected with exit 2.
    let mode_manifest = dir.child("legacy_mode.json");
    fs::write(
        &mode_manifest,
        serde_json::to_vec(&json!({
            "schema": "curvebook.batch.v1",
            "inputs": [{ "sheet": "d1", "csv": "data.csv", "mode": "replace_data" }]
        }))?,
    )?;
    let mode_err = assert_json_cli_error(
        &["batch", mode_manifest.to_str().ok_or("non-UTF-8 path")?, "--workdir", workdir_str, "--json"],
        2,
        "bad_input",
        "input",
    )?;
    assert!(mode_err["error"]["message"].as_str().is_some_and(|m| m.contains("`mode`")));

    let bad_cmd_manifest = dir.child("bad_cmd.json");
    fs::write(
        &bad_cmd_manifest,
        serde_json::to_vec(&json!({
            "schema": "curvebook.batch.v1",
            "commands": [{ "cmd": "project.new", "args": {} }]
        }))?,
    )?;
    assert_json_cli_error(
        &["batch", bad_cmd_manifest.to_str().ok_or("non-UTF-8 path")?, "--workdir", workdir_str, "--json"],
        2,
        "bad_input",
        "input",
    )?;

    let bad_pretty_manifest = dir.child("bad_pretty.json");
    fs::write(
        &bad_pretty_manifest,
        serde_json::to_vec(&json!({
            "schema": "curvebook.batch.v1",
            "exports": [{ "format": "json", "out": "out/doc.json", "pretty": "false" }]
        }))?,
    )?;
    assert_json_cli_error(
        &["batch", bad_pretty_manifest.to_str().ok_or("non-UTF-8 path")?, "--workdir", workdir_str, "--json"],
        2,
        "bad_input",
        "input",
    )?;

    // 3. Path traversal / outside path fails with CliError::PathEscape (`path_escape`, category `policy`, exit 2).
    let escape_manifest = dir.child("escape.json");
    let outside_out = outside.child("escaped.json");
    fs::write(
        &escape_manifest,
        serde_json::to_vec(&json!({
            "schema": "curvebook.batch.v1",
            "exports": [{ "format": "json", "out": outside_out }]
        }))?,
    )?;
    assert_json_cli_error(
        &["batch", escape_manifest.to_str().ok_or("non-UTF-8 path")?, "--workdir", workdir_str, "--json"],
        2,
        "path_escape",
        "policy",
    )?;
    assert!(!outside_out.exists(), "escaping export must not write outside workdir");

    // 4. Unimplemented legacy CLI subcommands are rejected as usage errors (exit 2).
    for legacy_cmd in ["import", "analyze", "verify"] {
        assert_json_cli_error(&[legacy_cmd, "--json"], 2, "bad_input", "input")?;
    }

    Ok(())
}

#[test]
fn audit_71_batch_fails_on_failed_result_sheets_without_writing_exports() -> TestResult {
    use std::sync::atomic::AtomicBool;

    use curvebook_doc::{Cell, DataSheet, ProjectDocument, SubcolumnMode, TableArchetype, XyTable};
    use curvebook_engine::Session;
    use curvebook_engine::cmd::Caller;
    use curvebook_format::save_template_file;

    // Build a valid template with a Linear Regression ResultSheet via engine dispatch.
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 4)?;
    for (r, (x, y)) in [(1.0, 2.0), (2.0, 4.0), (3.0, 6.0), (4.0, 8.0)].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut doc = ProjectDocument::new("Template Base")?;
    doc.add_data_sheet(DataSheet::new("plate", "Dose Plate", TableArchetype::Xy(xy))?)?;
    let mut session = Session::from_document(doc);
    session.execute_with_caller(
        "analysis.run",
        json!({
            "source_sheet_id": "plate",
            "sheet_id": "fit_blank_y",
            "title": "Blank-Y Fit",
            "kind": "linear_regression"
        }),
        Caller::Cli,
        &AtomicBool::new(false),
    )?;

    // Scenario A: blank Y CSV loaded into a template whose existing ResultSheet fails recalculation.
    let dir_a = TempDir::new("audit71-blank-y")?;
    let wd_a = dir_a.path().to_str().ok_or("non-UTF-8 temp dir")?;
    save_template_file(&dir_a.child("linear.cbtemplate"), session.document())?;
    fs::write(dir_a.child("blank_y.csv"), "X,Y\n1,\n2,\n3,\n4,\n")?;
    let manifest_a = dir_a.child("manifest_a.json");
    fs::write(
        &manifest_a,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "template": "linear.cbtemplate",
            "inputs": [{ "sheet": "Dose Plate", "csv": "blank_y.csv", "kind": "xy" }],
            "exports": [
                { "format": "json", "sheet": "Blank-Y Fit", "out": "out/fit.json" },
                { "format": "csv", "sheet": "Dose Plate", "out": "out/data.csv" },
                { "format": "curvebook", "out": "out/project.curvebook" }
            ]
        }))?,
    )?;
    let out_a = run_cli(&["batch", manifest_a.to_str().ok_or("non-UTF-8 path")?, "--workdir", wd_a, "--json"])?;
    assert_eq!(out_a.status.code(), Some(1), "batch with blank-Y failed result sheet must exit 1: {}", String::from_utf8_lossy(&out_a.stdout));
    let err_a: Value = serde_json::from_slice(&out_a.stdout)?;
    let msg_a = err_a["error"]["message"].as_str().unwrap_or("");
    assert!(msg_a.contains("Blank-Y Fit") && msg_a.contains("fit_blank_y"), "error must name failed result sheet: {msg_a}");
    assert_eq!(err_a["error"]["details"]["failed_count"], json!(1));
    assert_eq!(err_a["error"]["details"]["failed_result_sheets"][0]["sheet_id"], json!("fit_blank_y"));
    assert_eq!(err_a["error"]["details"]["failed_result_sheets"][0]["title"], json!("Blank-Y Fit"));
    for rel in ["out/fit.json", "out/data.csv", "out/project.curvebook"] {
        assert!(!dir_a.child(rel).exists(), "failed batch must write zero export files ({rel} exists)");
    }

    // Scenario B: manifest adds two analyses on valid data then overwrites Y cells with text -> both ResultSheets fail.
    let dir_b = TempDir::new("audit71-multi-fail")?;
    let wd_b = dir_b.path().to_str().ok_or("non-UTF-8 temp dir")?;
    fs::write(dir_b.child("valid.csv"), "X,Y\n1,2\n2,4\n3,6\n4,8\n")?;
    let manifest_b = dir_b.child("manifest_b.json");
    fs::write(
        &manifest_b,
        serde_json::to_vec_pretty(&json!({
            "schema": "curvebook.batch.v1",
            "commands": [
                { "cmd": "sheet.createData", "params": { "sheet_id": "d1", "title": "Assay", "kind": "xy", "rows": 7, "cols": 1 } },
                { "cmd": "table.setXCells", "params": { "sheet_id": "d1", "values": [-9.0, -8.0, -7.0, -6.5, -6.0, -5.0, -4.0] } },
                { "cmd": "table.setCells", "params": { "sheet_id": "d1", "rows": [[10.2], [12.5], [32.0], [55.0], [78.0], [96.5], [99.1]] } },
                { "cmd": "analysis.run", "params": { "source_sheet_id": "d1", "sheet_id": "fit_alpha", "title": "Alpha Fit", "kind": "linear_regression" } },
                { "cmd": "fit.run", "params": { "source_sheet_id": "d1", "sheet_id": "fit_beta", "title": "Beta Fit", "equation_id": "log_agonist_vs_response_4pl" } },
                { "cmd": "table.setCells", "params": { "sheet_id": "d1", "rows": [["bad"], ["text"], ["n/a"], ["err"], ["bad"], ["bad"], ["bad"]] } }
            ],
            "exports": [
                { "format": "json", "sheet": "Alpha Fit", "out": "out/alpha.json" },
                { "format": "json", "sheet": "Beta Fit", "out": "out/beta.json" },
                { "format": "csv", "sheet": "Assay", "out": "out/assay.csv" },
                { "format": "curvebook", "out": "out/multi.curvebook" }
            ]
        }))?,
    )?;
    let out_b = run_cli(&["batch", manifest_b.to_str().ok_or("non-UTF-8 path")?, "--workdir", wd_b, "--json"])?;
    assert_eq!(out_b.status.code(), Some(1), "batch with multiple failed result sheets must exit 1: {}", String::from_utf8_lossy(&out_b.stdout));
    let err_b: Value = serde_json::from_slice(&out_b.stdout)?;
    let msg_b = err_b["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg_b.contains("Alpha Fit") && msg_b.contains("fit_alpha") && msg_b.contains("Beta Fit") && msg_b.contains("fit_beta"),
        "error message must name every failed result sheet: {msg_b}"
    );
    assert_eq!(err_b["error"]["details"]["failed_count"], json!(2));
    for rel in ["out/alpha.json", "out/beta.json", "out/assay.csv", "out/multi.curvebook"] {
        assert!(!dir_b.child(rel).exists(), "failed batch must write zero export files ({rel} exists)");
    }

    Ok(())
}
