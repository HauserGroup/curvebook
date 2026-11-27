//! Integration tests for Wave 3A (`R23.1`–`R23.6`) headless `curvebook-cli` parity:
//! - Command registry discovery (`table.unlinkCalculatedVariable`, `fit.*UserEquation*`, `fit.cloneBuiltinEquation`)
//! - Direct CLI execution of atomic User Equation Library commands under `--workdir` confinement
//! - End-to-end `curvebook-cli run` script covering R23 analyses, unlink, Chained Monte Carlo, and Methods Report

use std::fs;

use serde_json::{Value, json};

use super::common::{CliHarness, TempDir};

fn run_json_ok(args: &[&str]) -> Result<Value, String> {
    let stdout = CliHarness::run_ok(args, None)?;
    serde_json::from_str(&stdout).map_err(|e| format!("parse cli JSON for {args:?}: {e}\nstdout:\n{stdout}"))
}

#[test]
fn cli_commands_lists_r23_unlink_and_user_equation_commands() -> Result<(), String> {
    let payload = run_json_ok(&["commands", "--json"])?;
    let cmds = payload["commands"].as_array().ok_or("missing commands array")?;
    for expected in [
        "table.unlinkCalculatedVariable",
        "fit.listUserEquations",
        "fit.getUserEquation",
        "fit.saveUserEquation",
        "fit.deleteUserEquation",
        "fit.cloneBuiltinEquation",
    ] {
        assert!(cmds.iter().any(|c| c["id"] == json!(expected)), "expected `{expected}` in `curvebook-cli commands --json`");
    }
    Ok(())
}

#[test]
fn cli_direct_user_equation_library_workflow_respects_workdir() -> Result<(), String> {
    let work = TempDir::new("r23-cli-eq")?;
    let work_str = work.path_str();

    // 1. Clone built-in equation directly via CLI.
    let cloned = run_json_ok(&[
        "fit.cloneBuiltinEquation",
        "--workdir",
        &work_str,
        "--builtin-id",
        "one_phase_decay",
        "--new-id",
        "cli_decay_custom",
        "--json",
    ])?;
    assert_eq!(cloned["ok"], json!(true));
    assert_eq!(cloned["result"]["saved"], json!(true));
    assert_eq!(cloned["result"]["equation"]["id"], json!("cli_decay_custom"));
    assert!(work.child("user_equations.json").exists(), "user_equations.json must reside inside --workdir");

    // 2. Save a custom equation with [Constraints] and [Transforms].
    let saved = run_json_ok(&[
        "fit.saveUserEquation",
        "--workdir",
        &work_str,
        "--id",
        "cli_mm_custom",
        "--name",
        "CLI Michaelis-Menten",
        "--dsl-source",
        "Y = Vmax * X / (Km + X)\n[Constraints]\nKm >= 0\n[Transforms]\nHalfVmax = Vmax / 2",
        "--overwrite",
        "true",
        "--json",
    ])?;
    assert_eq!(saved["ok"], json!(true));
    assert_eq!(saved["result"]["equation"]["transforms_count"], json!(1));

    // 3. List & Get.
    let listed = run_json_ok(&["fit.listUserEquations", "--workdir", &work_str, "--json"])?;
    assert_eq!(listed["result"]["count"], json!(2));
    let got = run_json_ok(&["fit.getUserEquation", "--workdir", &work_str, "--id", "cli_decay_custom", "--json"])?;
    assert_eq!(got["result"]["equation"]["cloned_from_builtin"], json!("one_phase_decay"));

    // 4. Path escape rejected with exit code 2.
    let esc_out = CliHarness::run(&["fit.listUserEquations", "--workdir", &work_str, "--dir", "../outside", "--json"])?;
    assert_eq!(esc_out.status.code(), Some(2), "expected exit code 2 for --workdir escape");

    // 5. Delete.
    let deleted = run_json_ok(&["fit.deleteUserEquation", "--workdir", &work_str, "--id", "cli_mm_custom", "--json"])?;
    assert_eq!(deleted["result"]["deleted"], json!(true));
    Ok(())
}

#[test]
fn cli_run_script_executes_r23_analyses_unlink_and_chained_monte_carlo() -> Result<(), String> {
    let work = TempDir::new("r23-cli-run")?;
    let work_str = work.path_str();
    let script_path = work.child("r23_pipeline.json");
    let out_proj = work.child("r23_out.cbproj");
    let report_md = work.child("mc_methods.md");

    let script = json!([
        {
            "cmd": "sheet.createData",
                "params": {
                    "sheet_id": "mv-1",
                    "title": "Cohort",
                    "kind": "multiple_variables",
                    "rows": 3,
                    "cols": 2,
                    "variables": [
                        { "title": "Input", "role": "Continuous" },
                        { "title": "Derived", "role": "Continuous" }
                    ]
                }
            },
            {
                "cmd": "table.setCells",
                "params": {
                    "sheet_id": "mv-1",
                    "edits": [
                        { "row": 0, "col": 0, "value": 2.0 },
                        { "row": 1, "col": 0, "value": 4.0 },
                        { "row": 2, "col": 0, "value": 6.0 }
                    ]
                }
            },
            {
                "cmd": "table.setCalculatedVariable",
                "params": { "sheet_id": "mv-1", "col": 1, "formula": "= [Input] + 10" }
            },
            {
                "cmd": "table.unlinkCalculatedVariable",
                "params": { "sheet_id": "mv-1", "col": 1 }
            },
            {
                "cmd": "table.importCsv",
                "params": {
                    "sheet_id": "xy-1",
                    "title": "Decay",
                    "archetype": "xy",
                    "csv": "X,Y\n0,100\n1,61\n2,37\n3,23\n4,14\n5,9\n"
                }
            },
            {
                "cmd": "transform.run",
                "params": { "sheet_id": "res-deriv", "source_sheet_id": "xy-1", "kind": "curve_calculus", "operation": "first_derivative" }
            },
            {
                "cmd": "fit.run",
                "params": {
                    "sheet_id": "res-dist",
                    "source_sheet_id": "xy-1",
                    "kind": "nonlinear",
                    "plot_distribution": {
                        "family": "StudentT",
                        "parameters": [10.0],
                        "tail_shade": { "TwoTailed": { "lower": -2.228, "upper": 2.228 } }
                    }
                }
            },
            {
                "cmd": "transform.run",
                "params": {
                    "sheet_id": "sim-xy",
                    "source_sheet_id": "xy-1",
                    "kind": "simulate",
                    "seed": 42,
                    "row_count": 7,
                    "replicates": 1,
                    "x_min": 0.0,
                    "x_max": 6.0,
                    "equation_id": "one_phase_decay",
                    "parameters": { "Y0": 100.0, "Plateau": 10.0, "K": 0.5 },
                    "noise": { "Gaussian": { "sd": 2.0 } }
                }
            },
            {
                "cmd": "fit.run",
                "params": {
                    "sheet_id": "fit-sim",
                    "source_sheet_id": "sim-xy",
                    "equation_id": "one_phase_decay"
                }
            },
            {
                "cmd": "stats.run",
                "params": {
                    "sheet_id": "res-mc",
                    "source_sheet_id": "sim-xy",
                    "kind": "monte_carlo",
                    "seed": 99,
                    "iterations": 12,
                    "downstream_result_sheet_id": "fit-sim",
                    "tracked_scalars": [
                        { "label": "RateK", "parameter_key": "K", "data_set_index": 0 }
                    ],
                    "hit_rules": [
                        { "scalar_index": 0, "op": "Between", "threshold": 0.3, "upper_threshold": 0.7 }
                    ],
                    "exclude_flagged_fits": true
                }
            },
            {
                "cmd": "result.generateReport",
                "params": { "result_sheet_id": "res-mc", "format": "markdown", "path": "mc_methods.md" }
            },
            {
                "cmd": "project.save",
                "params": { "path": "r23_out.cbproj" }
            }
    ]);
    fs::write(&script_path, serde_json::to_string_pretty(&script).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;

    let run_out = run_json_ok(&["run", script_path.to_str().ok_or("script path")?, "--workdir", &work_str, "--json"])?;
    assert_eq!(run_out["ok"], json!(true));
    assert_eq!(run_out["commands_executed"], json!(12));
    assert!(out_proj.exists(), "expected output project file");
    let report = fs::read_to_string(&report_md).map_err(|e| e.to_string())?;
    assert!(report.contains("Monte Carlo"), "expected Monte Carlo prose in methods report:\n{report}");
    assert!(!report.contains("MonteCarloSimulation("), "methods report must not emit Debug formatting");
    Ok(())
}
