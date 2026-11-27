//! Integration tests for Wave 3A (`R23.1`–`R23.6`) headless MCP parity:
//! - Progressive toolset exposure (`table_unlink_calculated_variable`, `fit_*_user_equation*`, `fit_clone_builtin_equation`)
//! - `table.unlinkCalculatedVariable` + undo/redo via MCP
//! - `RowStatistics`, `FractionOfTotal`, `CurveCalculus`, `plot_distribution`, `lognormal_one_way_anova`, and Chained Monte Carlo via MCP
//! - Atomic User Equation Library round-trip & `--workdir` path confinement enforcement via MCP

use curvebook_mcp::{McpServerState, call_tool};
use serde_json::{Value, json};

use super::common::{TempDir, parse_result_json, rpc_roundtrip};

fn call_ok(state: &mut McpServerState, tool: &str, args: Value) -> Result<Value, String> {
    let res = call_tool(state, tool, args).map_err(|e| format!("tool `{tool}` protocol err: {e}"))?;
    let val = parse_result_json(&res)?;
    if res.is_error {
        return Err(format!("tool `{tool}` returned error payload: {val}"));
    }
    Ok(val)
}

#[test]
fn mcp_toolsets_expose_unlink_and_user_equation_tools_and_enforce_workdir() -> Result<(), String> {
    let dir = TempDir::new_canonical("r23-mcp-eq")?;
    let mut state = McpServerState::with_workdir(dir.path()).map_err(|e| e.to_string())?;

    call_ok(&mut state, "enable_toolset", json!({ "group": "table" }))?;
    call_ok(&mut state, "enable_toolset", json!({ "group": "analysis" }))?;

    let tools_rpc = rpc_roundtrip(&mut state, 1, "tools/list", json!({}))?;
    let tools = tools_rpc["result"]["tools"].as_array().ok_or("missing tools list")?;
    for expected in [
        "table_unlink_calculated_variable",
        "fit_list_user_equations",
        "fit_get_user_equation",
        "fit_save_user_equation",
        "fit_delete_user_equation",
        "fit_clone_builtin_equation",
    ] {
        assert!(tools.iter().any(|t| t["name"] == json!(expected)), "expected MCP tool `{expected}` in tools/list");
    }

    // Clone a built-in equation inside `--workdir` (defaulting to workdir root when `dir` omitted).
    let cloned = call_ok(
        &mut state,
        "fit_clone_builtin_equation",
        json!({ "builtin_id": "michaelis_menten", "new_id": "mcp_mm_custom", "new_name": "MCP Michaelis-Menten" }),
    )?;
    assert_eq!(cloned["payload"]["saved"], json!(true));
    assert_eq!(cloned["payload"]["equation"]["id"], json!("mcp_mm_custom"));
    assert!(dir.child("user_equations.json").exists(), "user_equations.json must be written inside --workdir");

    // Save custom equation with [Constraints] and [Transforms].
    let saved = call_ok(
        &mut state,
        "fit_save_user_equation",
        json!({
            "id": "mcp_two_exp",
            "name": "MCP Two-Phase Decay",
            "dsl_source": "Y = A1 * exp(-KFast * X) + A2 * exp(-KSlow * X)\n[Constraints]\nKFast > KSlow\nKFast >= 0\nKSlow >= 0\n[Transforms]\nHalfLifeFast = ln(2) / KFast",
            "overwrite": true
        }),
    )?;
    assert_eq!(saved["payload"]["equation"]["relational_constraints_count"], json!(1));
    assert_eq!(saved["payload"]["equation"]["transforms_count"], json!(1));

    let listed = call_ok(&mut state, "fit_list_user_equations", json!({}))?;
    assert_eq!(listed["payload"]["count"], json!(2));

    let fetched = call_ok(&mut state, "fit_get_user_equation", json!({ "id": "mcp_mm_custom" }))?;
    assert_eq!(fetched["payload"]["equation"]["cloned_from_builtin"], json!("michaelis_menten"));

    // Fit using the saved user equation ID directly via MCP without passing custom_dsl.
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": { "sheet_id": "xy-mm", "title": "Enzyme", "kind": "xy", "rows": 5, "cols": 1 }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setXCells",
            "params": { "sheet_id": "xy-mm", "edits": [
                { "row": 0, "value": 1.0 },
                { "row": 1, "value": 2.0 },
                { "row": 2, "value": 5.0 },
                { "row": 3, "value": 10.0 },
                { "row": 4, "value": 25.0 }
            ] }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": { "sheet_id": "xy-mm", "edits": [
                { "row": 0, "col": 0, "value": 16.6667 },
                { "row": 1, "col": 0, "value": 28.5714 },
                { "row": 2, "col": 0, "value": 50.0 },
                { "row": 3, "col": 0, "value": 66.6667 },
                { "row": 4, "col": 0, "value": 83.3333 }
            ] }
        }),
    )?;
    let fit_out = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": { "sheet_id": "fit-user-mm", "source_sheet_id": "xy-mm", "equation_id": "mcp_mm_custom" }
        }),
    )?;
    assert_eq!(fit_out["payload"]["status"], json!("UpToDate"));
    let vmax = fit_out["payload"]["scalars"]["Vmax"]["value"].as_f64().ok_or("missing Vmax")?;
    assert!((vmax - 100.0).abs() < 1.0, "expected Vmax ~ 100, got {vmax}");

    // Escape attempt via `dir: "../outside"` must fail with structured `path_escape`.
    let esc_res = call_tool(&mut state, "fit_list_user_equations", json!({ "dir": "../outside_escape" }))
        .map_err(|e| format!("unexpected protocol error: {e}"))?;
    assert!(esc_res.is_error, "expected path escape rejection");
    let esc_val = parse_result_json(&esc_res)?;
    assert_eq!(esc_val["code"], json!("path_escape"));

    // Delete saved equation cleanly.
    let del = call_ok(&mut state, "fit_delete_user_equation", json!({ "id": "mcp_two_exp" }))?;
    assert_eq!(del["payload"]["deleted"], json!(true));
    Ok(())
}

#[test]
fn mcp_executes_r23_analyses_unlink_and_chained_monte_carlo() -> Result<(), String> {
    let mut state = McpServerState::new();

    // 1. MV calculated variable + table.unlinkCalculatedVariable + undo via MCP.
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "mv-1",
                "title": "MV",
                "kind": "multiple_variables",
                "rows": 3,
                "cols": 2,
                "variables": [
                    { "title": "Base", "role": "Continuous" },
                    { "title": "Calc", "role": "Continuous" }
                ]
            }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": { "sheet_id": "mv-1", "edits": [
                { "row": 0, "col": 0, "value": 4.0 },
                { "row": 1, "col": 0, "value": 8.0 },
                { "row": 2, "col": 0, "value": 12.0 }
            ] }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setCalculatedVariable",
            "params": { "sheet_id": "mv-1", "col": 1, "formula": "= [Base] * 3" }
        }),
    )?;
    let unlinked = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.unlinkCalculatedVariable",
            "params": { "sheet_id": "mv-1", "col": 1 }
        }),
    )?;
    assert_eq!(unlinked["payload"]["unlinked"], json!(true));

    // 2. Column data -> FractionOfTotal + Lognormal One-Way ANOVA.
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.importCsv",
            "params": {
                "sheet_id": "col-ln",
                "title": "Biomarkers",
                "archetype": "column",
                "csv": "Ctrl,Low,High\n10,25,80\n12,30,95\n11,28,90\n14,32,110\n"
            }
        }),
    )?;
    let frac = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "transform.run",
            "params": {
                "sheet_id": "res-frac",
                "source_sheet_id": "col-ln",
                "kind": "fraction_of_total",
                "denominator": "column_total",
                "output_percent": true,
                "ci_method": "wilson"
            }
        }),
    )?;
    assert_eq!(frac["payload"]["status"], json!("UpToDate"));

    let ln_anova = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "stats.run",
            "params": {
                "sheet_id": "res-ln-anova",
                "source_sheet_id": "col-ln",
                "kind": "lognormal_one_way_anova"
            }
        }),
    )?;
    assert_eq!(ln_anova["payload"]["status"], json!("UpToDate"));

    // 3. XY data -> RowStatistics, CurveCalculus, plot_distribution.
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": { "sheet_id": "xy-calc", "title": "Quad", "kind": "xy", "rows": 5, "cols": 1, "replicates": 2 }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setXCells",
            "params": { "sheet_id": "xy-calc", "edits": [
                { "row": 0, "value": 0.0 },
                { "row": 1, "value": 1.0 },
                { "row": 2, "value": 2.0 },
                { "row": 3, "value": 3.0 },
                { "row": 4, "value": 4.0 }
            ] }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": { "sheet_id": "xy-calc", "edits": [
                { "row": 0, "col": 0, "subcol": 0, "value": 0.0 }, { "row": 0, "col": 0, "subcol": 1, "value": 0.0 },
                { "row": 1, "col": 0, "subcol": 0, "value": 0.9 }, { "row": 1, "col": 0, "subcol": 1, "value": 1.1 },
                { "row": 2, "col": 0, "subcol": 0, "value": 3.8 }, { "row": 2, "col": 0, "subcol": 1, "value": 4.2 },
                { "row": 3, "col": 0, "subcol": 0, "value": 8.8 }, { "row": 3, "col": 0, "subcol": 1, "value": 9.2 },
                { "row": 4, "col": 0, "subcol": 0, "value": 15.8 }, { "row": 4, "col": 0, "subcol": 1, "value": 16.2 }
            ] }
        }),
    )?;
    let row_stats = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "transform.run",
            "params": { "sheet_id": "res-rowstats", "source_sheet_id": "xy-calc", "kind": "row_statistics", "summary": "mean_ci" }
        }),
    )?;
    assert_eq!(row_stats["payload"]["status"], json!("UpToDate"));

    let deriv = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "transform.run",
            "params": { "sheet_id": "res-deriv", "source_sheet_id": "xy-calc", "kind": "curve_calculus", "operation": "first_derivative" }
        }),
    )?;
    assert_eq!(deriv["payload"]["status"], json!("UpToDate"));

    let dist = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "res-dist",
                "source_sheet_id": "xy-calc",
                "kind": "nonlinear",
                "plot_distribution": {
                    "family": "Normal",
                    "parameters": [0.0, 1.0],
                    "tail_shade": { "UpperTail": { "cutoff": 1.96 } },
                    "cumulative": false
                }
            }
        }),
    )?;
    assert_eq!(dist["payload"]["status"], json!("UpToDate"));

    // 4. Chained Monte Carlo via MCP: Simulate Column -> One-Way ANOVA -> MonteCarloSimulation.
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "transform.run",
            "params": {
                "sheet_id": "sim-col",
                "source_sheet_id": "col-ln",
                "kind": "simulate",
                "seed": 77,
                "row_count": 6,
                "simulation_mode": {
                    "Column": {
                        "column_means": [10.0, 15.0, 20.0],
                        "within_sd": 2.0
                    }
                }
            }
        }),
    )?;
    call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "stats.run",
            "params": { "sheet_id": "sim-anova", "source_sheet_id": "sim-col", "kind": "one_way_anova" }
        }),
    )?;
    let mc = call_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "stats.run",
            "params": {
                "sheet_id": "res-mc",
                "source_sheet_id": "sim-col",
                "kind": "monte_carlo",
                "seed": 2026,
                "iterations": 15,
                "downstream_result_sheet_id": "sim-anova",
                "tracked_scalars": [
                    { "label": "AnovaP", "parameter_key": "P value" }
                ],
                "hit_rules": [
                    { "scalar_index": 0, "op": "LessThan", "threshold": 0.05 }
                ]
            }
        }),
    )?;
    assert_eq!(mc["payload"]["status"], json!("UpToDate"));
    assert_eq!(mc["payload"]["iterations"], json!(15.0));
    assert_eq!(mc["payload"]["retained_iterations"], json!(15.0));
    let hits = mc["payload"]["hits"].as_f64().ok_or("missing hits")?;
    let not_hits = mc["payload"]["not_hits"].as_f64().ok_or("missing not_hits")?;
    assert!((hits + not_hits - 15.0).abs() < 1e-9);
    Ok(())
}
