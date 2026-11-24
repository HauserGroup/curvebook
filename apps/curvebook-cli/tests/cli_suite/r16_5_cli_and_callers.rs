//! Integration tests for `R16.5` Seam B & Seam C:
//! - Deprecated CLI direct flags & legacy `curvebook.batch.v1` manifests succeed while emitting
//!   deprecation warnings on `stderr` and `--json`.
//! - Canonical CLI direct & batch invocations emit zero warnings (`stderr` clean, `"warnings"` omitted).
//! - Representative `UiApp` (`Caller::Ui`), `curvebook-mcp` (`Caller::Mcp`), and `curvebook-cli`
//!   (`Caller::Cli`) canonical flows emit zero warnings (`outcome.warnings.is_empty()`).

use std::fs;
use std::path::Path;
use std::process::Output;
use std::sync::atomic::AtomicBool;

use curvebook_engine::{Caller, Session};
use curvebook_mcp::{McpServerState, call_tool};
use curvebook_ui_egui::UiApp;
use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

type TestResult = Result<(), String>;

fn path_str(p: &Path) -> Result<&str, String> {
    CliHarness::path_str(p)
}

fn run_cli(args: &[&str]) -> Result<Output, String> {
    CliHarness::run(args)
}

fn run_cli_checked(args: &[&str]) -> Result<(Value, String), String> {
    CliHarness::run_checked_json(args)
}

fn string_array(val: &Value, key: &str) -> Result<Vec<String>, String> {
    let arr = val.get(key).and_then(Value::as_array).ok_or_else(|| format!("expected array at `{key}` in {val}"))?;
    arr.iter().map(|item| item.as_str().map(str::to_owned).ok_or_else(|| format!("non-string item in `{key}`: {item}"))).collect()
}

#[test]
fn cli_direct_deprecated_flags_succeed_with_stderr_and_json_warnings() -> TestResult {
    let work = TempDir::new("cb_r165_cli_dep_direct")?;
    let wd = path_str(work.path())?;

    let (created_json, created_err) = run_cli_checked(&[
        "sheet.createData",
        "--sheet-id",
        "d1",
        "--archetype",
        "xy",
        "--rows",
        "4",
        "--cols",
        "1",
        "--workdir",
        wd,
        "--out",
        "study.curvebook",
        "--json",
    ])?;
    assert_eq!(created_json["ok"], json!(true));
    assert!(created_err.contains("warning:"), "expected deprecation warning on stderr: {created_err}");
    assert!(created_err.contains("`archetype`") && created_err.contains("`kind`"), "stderr: {created_err}");
    let created_warnings = string_array(&created_json, "warnings")?;
    assert!(
        created_warnings.iter().any(|w| w.contains("`archetype`") && w.contains("`kind`") && w.contains("sheet.createData")),
        "unexpected json warnings: {created_warnings:?}"
    );

    let (renamed_json, renamed_err) = run_cli_checked(&[
        "sheet.rename",
        "--workdir",
        wd,
        "--in",
        "study.curvebook",
        "--out",
        "study.curvebook",
        "--id",
        "d1",
        "--new-title",
        "Renamed Dose",
        "--json",
    ])?;
    assert_eq!(renamed_json["ok"], json!(true));
    let rename_warnings = string_array(&renamed_json, "warnings")?;
    assert_eq!(rename_warnings.len(), 2, "expected 2 warnings, got {rename_warnings:?}");
    assert!(rename_warnings.iter().any(|w| w.contains("`id`") && w.contains("`sheet_id`")));
    assert!(rename_warnings.iter().any(|w| w.contains("`new_title`") && w.contains("`title`")));
    assert!(renamed_err.contains("warning:") && renamed_err.contains("`new_title`"));

    // Plain (non-JSON) direct execution also prints `warning: ...` to stderr and succeeds.
    let plain = run_cli(&["sheet.rename", "--workdir", wd, "--in", "study.curvebook", "--sheet-id", "d1", "--new-title", "Plain Renamed"])?;
    assert_eq!(plain.status.code(), Some(0));
    let plain_err = String::from_utf8_lossy(&plain.stderr);
    assert!(plain_err.contains("warning:") && plain_err.contains("`new_title`"), "plain stderr: {plain_err}");
    Ok(())
}

#[test]
fn cli_batch_legacy_manifest_and_script_emit_warnings_while_canonical_cli_emits_zero_warnings() -> TestResult {
    let work = TempDir::new("cb_r165_cli_batch")?;
    let wd = path_str(work.path())?;
    let csv_path = work.path().join("dose.csv");
    fs::write(&csv_path, "x,y\n1,2\n2,4\n3,6\n4,8\n").map_err(|e| e.to_string())?;

    // 1. Legacy `curvebook.batch.v1` manifest with deprecated input/command/export keys,
    // plus `graph.addAnnotation` combining `"type": "shape"` and `"color"`.
    let legacy_manifest_path = work.path().join("legacy_batch.json");
    let legacy_manifest = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{
            "csv": "dose.csv",
            "sheetId": "xy1",
            "tableKind": "xy",
            "subcolumnMode": "single",
            "decimalSeparator": "."
        }],
        "commands": [
            { "command": "sheet.rename", "params": { "id": "xy1", "new_title": "Assay" } },
            { "method": "fit.run", "params": { "id": "fit1", "source_sheet": "xy1", "kind": "linear", "deming": true } },
            { "cmd": "graph.create", "params": { "id": "g1", "source_sheet_id": "xy1", "kind": "xy" } },
            {
                "cmd": "graph.addAnnotation",
                "params": {
                    "id": "g1",
                    "type": "shape",
                    "color": "#112233",
                    "rect_pt": { "x": 12.0, "y": 14.0, "width": 50.0, "height": 24.0 }
                }
            }
        ],
        "exports": [{ "out": "legacy_fit.csv", "format": "csv", "sheetId": "fit1", "tabIndex": 0 }]
    });
    fs::write(&legacy_manifest_path, serde_json::to_string_pretty(&legacy_manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let (batch_json, batch_err) = run_cli_checked(&["batch", path_str(&legacy_manifest_path)?, "--workdir", wd, "--json"])?;
    assert_eq!(batch_json["ok"], json!(true));
    assert_eq!(batch_json["inputs_loaded"], json!(1));
    assert_eq!(batch_json["commands_executed"], json!(4));
    assert_eq!(batch_json["exports_written"], json!(1));
    assert!(work.path().join("legacy_fit.csv").is_file());
    assert!(batch_err.contains("warning:"), "expected deprecation warnings on stderr: {batch_err}");

    let top_warnings = string_array(&batch_json, "warnings")?;
    assert!(top_warnings.iter().any(|w| w.contains("inputs[0].sheetId")), "missing sheetId warning: {top_warnings:?}");
    assert!(top_warnings.iter().any(|w| w.contains("inputs[0].tableKind")), "missing tableKind warning: {top_warnings:?}");
    assert!(top_warnings.iter().any(|w| w.contains("commands[0]") && w.contains("`new_title`")), "missing command warning: {top_warnings:?}");
    assert!(
        top_warnings.iter().any(|w| w.contains("graph.addAnnotation") && w.contains("`type`") && w.contains("`kind`")),
        "missing type->kind warning: {top_warnings:?}"
    );
    assert!(
        top_warnings.iter().any(|w| w.contains("graph.addAnnotation") && w.contains("`color`") && w.contains("`stroke_color`")),
        "missing color->stroke_color warning: {top_warnings:?}"
    );
    assert!(top_warnings.iter().any(|w| w.contains("exports[0].sheetId")), "missing export sheetId warning: {top_warnings:?}");

    // 2. `run <script>` executes commands with deprecated aliases directly against the engine,
    // surfacing per-entry `results[i].warnings` and top-level `warnings` alongside stderr.
    let script_path = work.path().join("legacy_script.jsonl");
    fs::write(
        &script_path,
        concat!(
            "{\"cmd\":\"sheet.createData\",\"params\":{\"sheet_id\":\"d1\",\"kind\":\"xy\",\"rows\":3,\"cols\":1}}\n",
            "{\"cmd\":\"sheet.rename\",\"params\":{\"id\":\"d1\",\"new_title\":\"Script Renamed\"}}\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let (script_json, script_err) = run_cli_checked(&["run", path_str(&script_path)?, "--workdir", wd, "--json"])?;
    assert_eq!(script_json["ok"], json!(true));
    assert!(script_err.contains("warning:") && script_err.contains("`new_title`"));
    assert!(script_json["results"][0].get("warnings").is_none(), "canonical step 0 must omit warnings");
    let step1_warnings = string_array(&script_json["results"][1], "warnings")?;
    assert_eq!(step1_warnings.len(), 2);
    assert_eq!(string_array(&script_json, "warnings")?.len(), 2);

    // 3. Canonical CLI direct command emits ZERO warnings (empty stderr, no `warnings` key in JSON).
    let (canon_direct, canon_direct_err) = run_cli_checked(&[
        "sheet.createData",
        "--sheet-id",
        "canon_1",
        "--title",
        "Canonical Direct",
        "--kind",
        "xy",
        "--rows",
        "4",
        "--cols",
        "1",
        "--workdir",
        wd,
        "--out",
        "canon.curvebook",
        "--json",
    ])?;
    assert_eq!(canon_direct["ok"], json!(true));
    assert!(canon_direct_err.trim().is_empty(), "canonical direct CLI wrote unexpected stderr: {canon_direct_err}");
    assert!(canon_direct.get("warnings").is_none(), "canonical direct JSON must omit `warnings`: {canon_direct}");

    // 4. Canonical `curvebook.batch.v1` manifest emits ZERO warnings everywhere.
    let canon_manifest_path = work.path().join("canon_batch.json");
    let canon_manifest = json!({
        "schema": "curvebook.batch.v1",
        "inputs": [{
            "csv": "dose.csv",
            "sheet": "xy1",
            "kind": "xy",
            "subcolumn_mode": "single",
            "decimal_separator": "."
        }],
        "commands": [
            { "cmd": "sheet.rename", "params": { "sheet_id": "xy1", "title": "Canonical Assay" } },
            { "cmd": "fit.run", "params": { "sheet_id": "fit1", "source_sheet_id": "xy1", "kind": "linear", "deming": true } },
            { "cmd": "graph.create", "params": { "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy" } },
            {
                "cmd": "graph.addAnnotation",
                "params": {
                    "sheet_id": "g1",
                    "kind": "shape",
                    "stroke_color": "#112233",
                    "bounds_pt": { "x": 12.0, "y": 14.0, "width": 50.0, "height": 24.0 }
                }
            }
        ],
        "exports": [{ "out": "canon_fit.csv", "format": "csv", "sheet": "fit1", "tab_index": 0 }]
    });
    fs::write(&canon_manifest_path, serde_json::to_string_pretty(&canon_manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let (canon_batch, canon_batch_err) = run_cli_checked(&["batch", path_str(&canon_manifest_path)?, "--workdir", wd, "--json"])?;
    assert_eq!(canon_batch["ok"], json!(true));
    assert!(canon_batch_err.trim().is_empty(), "canonical batch CLI wrote unexpected stderr: {canon_batch_err}");
    assert!(canon_batch.get("warnings").is_none(), "canonical batch JSON must omit `warnings`: {canon_batch}");
    let batch_results = canon_batch["results"].as_array().ok_or("missing batch results array")?;
    for entry in batch_results {
        assert!(entry.get("warnings").is_none(), "canonical batch entry must omit `warnings`: {entry}");
    }
    Ok(())
}

#[test]
fn canonical_ui_mcp_and_cli_callers_emit_zero_warnings() -> TestResult {
    let cancel = AtomicBool::new(false);

    // 1. UiApp (`Caller::Ui` & `Caller::Control`) canonical flow emits zero warnings.
    let mut app = UiApp::new();
    let ui_steps: [(&str, Value); 8] = [
        ("sheet.createData", json!({ "sheet_id": "xy1", "title": "Dose", "kind": "xy", "rows": 4, "cols": 1, "auto_graph": false })),
        ("table.setXCells", json!({ "sheet_id": "xy1", "values": [1.0, 2.0, 3.0, 4.0] })),
        (
            "table.setCells",
            json!({
                "sheet_id": "xy1",
                "edits": [
                    { "row": 0, "col": 0, "subcol": 0, "value": 2.0 },
                    { "row": 1, "col": 0, "subcol": 0, "value": 4.1 },
                    { "row": 2, "col": 0, "subcol": 0, "value": 5.9 },
                    { "row": 3, "col": 0, "subcol": 0, "value": 8.0 }
                ]
            }),
        ),
        ("fit.run", json!({ "sheet_id": "fit1", "source_sheet_id": "xy1", "kind": "linear", "deming": false })),
        ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "xy1", "kind": "xy", "title": "Dose Plot" })),
        (
            "graph.addAnnotation",
            json!({ "sheet_id": "g1", "kind": "shape", "stroke_color": "#000000", "bounds_pt": { "x": 10.0, "y": 10.0, "width": 50.0, "height": 25.0 } }),
        ),
        ("layout.create", json!({ "sheet_id": "lay1", "title": "Figure 1", "preset": "SingleColumn85Mm" })),
        ("layout.addPanel", json!({ "sheet_id": "lay1", "kind": "graph", "graph_sheet_id": "g1", "bounds_pt": [18.0, 18.0, 180.0, 140.0] })),
    ];
    for (cmd_id, params) in ui_steps {
        let outcome = app.execute_command(cmd_id, params).map_err(|e| format!("UiApp `{cmd_id}` failed: {e}"))?;
        assert!(outcome.warnings.is_empty(), "UiApp `{cmd_id}` emitted unexpected warnings: {:?}", outcome.warnings);
    }

    let ctrl_clean = app
        .execute_control_command("sheet.rename", json!({ "sheet_id": "xy1", "title": "Dose Clean" }))
        .map_err(|e| format!("control rename failed: {e}"))?;
    assert!(ctrl_clean.warnings.is_empty(), "canonical Control emitted warnings: {:?}", ctrl_clean.warnings);
    let ctrl_dep = app
        .execute_control_command("sheet.rename", json!({ "id": "xy1", "new_title": "Dose Deprecated" }))
        .map_err(|e| format!("control deprecated rename failed: {e}"))?;
    assert_eq!(ctrl_dep.warnings.len(), 2, "expected 2 warnings on deprecated Control call: {:?}", ctrl_dep.warnings);

    // 2. `curvebook-mcp` (`Caller::Mcp`) canonical flow emits zero warnings & omits `"warnings"` from structuredContent.
    let mut mcp = McpServerState::new();
    let _ = call_tool(&mut mcp, "enable_toolset", json!({ "group": "sheet" })).map_err(|e| e.to_string())?;
    let mcp_direct = call_tool(
        &mut mcp,
        "sheet_create_data",
        json!({ "sheet_id": "col1", "title": "Cohorts", "kind": "column", "rows": 3, "cols": 2, "auto_graph": false }),
    )
    .map_err(|e| e.to_string())?;
    assert!(!mcp_direct.is_error);
    let mcp_direct_sc = mcp_direct.structured_content.as_ref().ok_or("missing structuredContent for sheet_create_data")?;
    assert!(mcp_direct_sc.get("warnings").is_none(), "canonical MCP tool must omit `warnings`: {mcp_direct_sc}");

    let mcp_batch = call_tool(
        &mut mcp,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "table.paste", "params": { "sheet_id": "col1", "text": "10\t20\n11\t22\n12\t24" } },
                { "command_id": "stats.run", "params": { "sheet_id": "anova1", "source_sheet_id": "col1", "kind": "one_way_anova", "multiplicity": "tukey", "alpha": 0.05 } }
            ]
        }),
    )
    .map_err(|e| e.to_string())?;
    assert!(!mcp_batch.is_error);
    let mcp_batch_sc = mcp_batch.structured_content.as_ref().ok_or("missing structuredContent for command_batch")?;
    let mcp_batch_results = mcp_batch_sc["results"].as_array().ok_or("missing MCP batch results")?;
    for entry in mcp_batch_results {
        assert!(entry["outcome"].get("warnings").is_none(), "canonical MCP batch outcome must omit `warnings`: {entry}");
    }

    let mcp_deprecated =
        call_tool(&mut mcp, "command_run", json!({ "command_id": "sheet.rename", "params": { "id": "col1", "new_title": "Deprecated Rename" } }))
            .map_err(|e| e.to_string())?;
    assert!(!mcp_deprecated.is_error);
    let mcp_dep_sc = mcp_deprecated.structured_content.as_ref().ok_or("missing structuredContent for deprecated MCP command")?;
    let mcp_dep_warnings = string_array(mcp_dep_sc, "warnings")?;
    assert_eq!(mcp_dep_warnings.len(), 2, "expected 2 MCP deprecation warnings: {mcp_dep_warnings:?}");

    // 3. Direct `Session::execute_with_caller` across `Caller::Ui`, `Caller::Mcp`, and `Caller::Cli`.
    for caller in [Caller::Ui, Caller::Mcp, Caller::Cli] {
        let mut session = Session::new();
        let out1 = session
            .execute_with_caller(
                "sheet.createData",
                json!({ "sheet_id": "s1", "title": "Study", "kind": "column", "rows": 3, "cols": 2, "auto_graph": false }),
                caller,
                &cancel,
            )
            .map_err(|e| format!("{caller:?} createData failed: {e}"))?;
        assert!(out1.warnings.is_empty(), "{caller:?} createData warnings: {:?}", out1.warnings);

        let out2 = session
            .execute_with_caller("table.paste", json!({ "sheet_id": "s1", "text": "10\t20\n11\t22\n12\t24" }), caller, &cancel)
            .map_err(|e| format!("{caller:?} paste failed: {e}"))?;
        assert!(out2.warnings.is_empty(), "{caller:?} paste warnings: {:?}", out2.warnings);

        let out3 = session
            .execute_with_caller("stats.run", json!({ "sheet_id": "res1", "source_sheet_id": "s1", "kind": "welch_t" }), caller, &cancel)
            .map_err(|e| format!("{caller:?} stats.run failed: {e}"))?;
        assert!(out3.warnings.is_empty(), "{caller:?} stats.run warnings: {:?}", out3.warnings);
    }

    Ok(())
}
