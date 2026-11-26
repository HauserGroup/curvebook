//! R17.4 regression tests for MCP token compaction, selective `doc_inspect` / `command_list` /
//! `list_toolsets` / `resources` controls, schema clarity, and actionable error remediations.

#![forbid(unsafe_code)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CurveSamplePoint, FittedCurveOverlay, LinearRegressionSpec, ProjectDocument, ResultCell, ResultScalarEntry,
    ResultSheet, ResultStatus, ResultTab, ResultTabKind,
};
use curvebook_mcp::{ERR_INVALID_PARAMS, McpServerState, ResourceError, call_tool, dispatch_json_value, list_prompts, read_resource};
use serde_json::{Value, json};

use crate::common::{TempDir as TempWorkdir, encode_base64, rpc_roundtrip};

fn rpc(state: &mut McpServerState, id: i64, method: &str, params: Value) -> Result<Value, String> {
    rpc_roundtrip(state, id, method, params)
}

fn call_json_ok(state: &mut McpServerState, name: &str, args: Value) -> Result<Value, String> {
    let res = call_tool(state, name, args).map_err(|err| format!("call_tool({name}): {err}"))?;
    if res.is_error {
        return Err(format!("expected success for `{name}`, got error: {:?}", res.structured_content));
    }
    let text = res
        .content
        .first()
        .map(|b| match b {
            curvebook_mcp::ContentBlock::Text { text } => text.as_str(),
            curvebook_mcp::ContentBlock::Image { .. } => "",
        })
        .unwrap_or("");
    if text.contains('\n') {
        return Err(format!("expected compact single-line JSON in content[0].text for `{name}`, got newlines"));
    }
    res.structured_content.ok_or_else(|| format!("missing structuredContent on `{name}`"))
}

#[test]
fn doc_inspect_compact_sections_limits_and_single_sheet_graph_trimming() -> Result<(), String> {
    let mut state = McpServerState::new();
    call_json_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-1",
                "title": "Dose Series",
                "kind": "xy",
                "rows": 8,
                "cols": 1,
                "replicates": 2,
                "auto_family": true,
                "info_sheet_id": "info-1",
                "graph_sheet_id": "graph-1"
            }
        }),
    )?;
    let xs = [-9.0_f64, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0];
    call_json_ok(&mut state, "command_run", json!({ "command_id": "table.setXCells", "params": { "sheet_id": "xy-1", "values": xs } }))?;
    let mut edits = Vec::new();
    for (r, x) in xs.iter().enumerate() {
        let y = 10.0 + 90.0 / (1.0 + 10.0_f64.powf(x - (-6.0)));
        edits.push(json!({ "row": r, "col": 0, "subcol": 0, "value": y - 0.5 }));
        edits.push(json!({ "row": r, "col": 0, "subcol": 1, "value": y + 0.5 }));
    }
    call_json_ok(&mut state, "command_run", json!({ "command_id": "table.setCells", "params": { "sheet_id": "xy-1", "edits": edits } }))?;
    call_json_ok(
        &mut state,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "fit-1",
                "source_sheet_id": "xy-1",
                "equation_id": "log_inhibitor_vs_response_4pl",
                "identify_unstable_parameters": true
            }
        }),
    )?;

    // Root `standard` avoids duplicating `analyses`/`graphs`/`layouts` under `sheets`.
    let standard = call_json_ok(&mut state, "doc_inspect", json!({}))?;
    if standard.pointer("/sheets/analyses").is_some() || standard.pointer("/sheets/graphs").is_some() || standard.pointer("/sheets/layouts").is_some()
    {
        return Err(format!("root doc_inspect should not duplicate analyses/graphs/layouts inside sheets: {standard}"));
    }
    if standard.pointer("/sheets/data").is_none() || standard["analyses"].as_array().map(Vec::len) != Some(1) {
        return Err(format!("root doc_inspect missing sheets.data or analyses: {standard}"));
    }

    // `detail: "compact"` omits large scalar arrays and result_scalars duplication.
    let compact = call_json_ok(&mut state, "doc_inspect", json!({ "detail": "compact" }))?;
    if compact["analyses"][0].get("scalars").is_some() {
        return Err(format!("compact detail must omit analyses[0].scalars: {compact}"));
    }
    if compact["analyses"][0]["scalars_count"].as_u64().unwrap_or(0) == 0 {
        return Err(format!("compact detail missing scalars_count: {compact}"));
    }
    if compact.pointer("/variables/result_scalars").is_some() || compact.pointer("/variables/result_scalars_count").is_none() {
        return Err(format!("compact detail must replace result_scalars with result_scalars_count: {compact}"));
    }

    // `sections` scopes root summary strictly to requested keys.
    let graphs_only = call_json_ok(&mut state, "doc_inspect", json!({ "detail": "compact", "sections": ["graphs"] }))?;
    if graphs_only.get("sheets").is_some() || graphs_only.get("analyses").is_some() || graphs_only["graphs"].as_array().map(Vec::len) != Some(1) {
        return Err(format!("sections=['graphs'] leaked unrequested sections: {graphs_only}"));
    }

    // Single GraphSheet without `include_cells` omits bloated `spec`, while `detail: "full"` restores it.
    let graph_meta = call_json_ok(&mut state, "doc_inspect", json!({ "sheet_id": "graph-1", "include_cells": false }))?;
    if graph_meta.get("spec").is_some() || graph_meta.get("series_summary").is_none() {
        return Err(format!("graph include_cells=false should omit spec and provide series_summary: {graph_meta}"));
    }
    let graph_full = call_json_ok(&mut state, "doc_inspect", json!({ "sheet_id": "graph-1", "detail": "full" }))?;
    if graph_full.get("spec").is_none() {
        return Err(format!("graph detail=full missing spec: {graph_full}"));
    }

    // `max_rows` bounds Data CSV preview; `tab_id` + compact cell rendering trims ResultSheet inspect.
    let data_rows_3 = call_json_ok(&mut state, "doc_inspect", json!({ "sheet_id": "xy-1", "include_cells": true, "max_rows": 3 }))?;
    let csv = data_rows_3["csv_preview"].as_str().ok_or("missing csv_preview")?;
    if data_rows_3["csv_preview_truncated"] != json!(true) || csv.lines().count() > 4 {
        return Err(format!("expected max_rows=3 truncation, got lines={} payload={data_rows_3}", csv.lines().count()));
    }

    let fit_tab = call_json_ok(
        &mut state,
        "doc_inspect",
        json!({ "sheet_id": "fit-1", "include_cells": true, "tab_id": "nonlinear_parameters", "max_rows": 2 }),
    )?;
    let tabs = fit_tab["tabs"].as_array().ok_or("missing tabs")?;
    if tabs.len() != 1 || tabs[0]["id"] != json!("nonlinear_parameters") {
        return Err(format!("expected single nonlinear_parameters tab, got {fit_tab}"));
    }
    if tabs[0]["rows"].as_array().map(Vec::len) != Some(2) {
        return Err(format!("expected max_rows=2 on ResultTab, got {tabs:?}"));
    }
    if !tabs[0]["rows"][0][0].is_number() {
        return Err(format!("expected compact numeric ResultCell in doc_inspect, got {}", tabs[0]["rows"][0][0]));
    }
    if fit_tab["tab_ids"].as_array().is_none_or(|ids| !ids.contains(&json!("nonlinear_residuals"))) {
        return Err(format!("expected tab_ids discovery list, got {fit_tab}"));
    }

    // Invalid option values return ERR_INVALID_PARAMS (-32602).
    assert!(call_tool(&mut state, "doc_inspect", json!({ "detail": "verbose" })).is_err());
    assert!(call_tool(&mut state, "doc_inspect", json!({ "sections": ["bogus"] })).is_err());
    assert!(call_tool(&mut state, "doc_inspect", json!({ "max_rows": 0 })).is_err());
    Ok(())
}

#[test]
fn command_list_list_toolsets_and_compacted_tool_schemas() -> Result<(), String> {
    let mut state = McpServerState::new();

    // Default `command_list({})` omits 106 raw schemas and empty boilerplate fields.
    let list_default = call_json_ok(&mut state, "command_list", json!({}))?;
    let cmds = list_default["commands"].as_array().ok_or("missing commands")?;
    let fit_entry = cmds.iter().find(|c| c["id"] == json!("fit.run")).ok_or("missing fit.run")?;
    if fit_entry.get("input_schema").is_some() || fit_entry.get("beginner").is_some() {
        return Err(format!("default command_list should omit input_schema/beginner: {fit_entry}"));
    }

    // Exact `command_id` lookup defaults `include_schemas: true`, and normalizes tool names too.
    let by_tool_name = call_json_ok(&mut state, "command_list", json!({ "command_id": "table_exclude_cells_with_reason" }))?;
    let single = by_tool_name["commands"].as_array().ok_or("missing commands")?;
    if single.len() != 1 || single[0]["id"] != json!("table.excludeCellsWithReason") {
        return Err(format!("expected single command_id match, got {by_tool_name}"));
    }
    let schema_val = &single[0]["input_schema"];
    let schema_str = serde_json::to_string(schema_val).map_err(|e| e.to_string())?;
    if schema_str.contains("\"$schema\"") {
        return Err(format!("expected compacted schema without $schema boilerplate: {schema_str}"));
    }
    if schema_val.pointer("/properties/note/anyOf").is_some() || schema_val.pointer("/properties/note/type") != Some(&json!(["string", "null"])) {
        return Err(format!("expected nullable `note` collapsed to [\"string\", \"null\"]: {schema_val}"));
    }

    // `list_toolsets` supports `include_tools: false` and `group` filter cleanly.
    let slim_ts = call_json_ok(&mut state, "list_toolsets", json!({ "include_tools": false }))?;
    let ts_arr = slim_ts["toolsets"].as_array().ok_or("missing toolsets")?;
    if ts_arr.iter().any(|g| g.get("tools").is_some()) {
        return Err(format!("include_tools=false should omit tools arrays: {slim_ts}"));
    }

    // Enabled `fit_run` exposes `method` (ROUT / robust / least_squares) and `stats_run` documents kinds.
    call_json_ok(&mut state, "enable_toolset", json!({ "group": "analysis" }))?;
    let tools_rpc = rpc(&mut state, 1, "tools/list", json!({}))?;
    let tools = tools_rpc["result"]["tools"].as_array().ok_or("tools array")?;
    let fit_tool = tools.iter().find(|t| t["name"] == json!("fit_run")).ok_or("missing fit_run")?;
    if fit_tool.pointer("/inputSchema/properties/method").is_none() {
        return Err(format!("fit_run missing `method` property in MCP schema: {fit_tool}"));
    }
    let stats_tool = tools.iter().find(|t| t["name"] == json!("stats_run")).ok_or("missing stats_run")?;
    let stats_kind_desc = stats_tool.pointer("/inputSchema/properties/kind/description").and_then(Value::as_str).unwrap_or("");
    if !stats_kind_desc.contains("one_way_anova") || !stats_kind_desc.contains("mann_whitney") {
        return Err(format!("stats_run kind description missing shorthand examples: {stats_kind_desc}"));
    }
    Ok(())
}

#[test]
fn actionable_error_hints_and_messy_csv_open_warnings() -> Result<(), String> {
    let dir = TempWorkdir::new("hints")?;
    let messy = dir.path.join("messy.csv");
    fs::write(&messy, "Run 1 Preamble;;\nDose;Ctrl;Drug\n1;100.5;ND\n10;99.2;42.1\n").map_err(|e| e.to_string())?;

    let mut state = McpServerState::with_workdir(&dir.path).map_err(|e| e.to_string())?;
    let opened = call_json_ok(
        &mut state,
        "open_file",
        json!({ "path": messy.display().to_string(), "target_kind": "xy", "delimiter": ";", "sheet_id": "messy-1" }),
    )?;
    let warnings = opened["warnings"].as_array().ok_or_else(|| format!("expected open_file messy-CSV warnings: {opened}"))?;
    if warnings.is_empty() || opened.get("next_step_hint").and_then(Value::as_str).is_none() {
        return Err(format!("expected non-empty warnings + next_step_hint: {opened}"));
    }

    // Missing required parameter gets command_list schema hint.
    let excl_err = call_tool(
        &mut state,
        "command_run",
        json!({ "command_id": "table.excludeCellsWithReason", "params": { "sheet_id": "messy-1", "row": 1, "col": 0 } }),
    )
    .map_err(|e| e.to_string())?;
    if !excl_err.is_error {
        return Err(String::from("expected missing reason_tag error"));
    }
    let excl_sc = excl_err.structured_content.ok_or("missing excl structuredContent")?;
    if !excl_sc["remediation"].as_str().unwrap_or("").contains("command_list") {
        return Err(format!("expected command_list hint on missing required field: {excl_sc}"));
    }

    // Unknown command ID suggests closest registry commands.
    let unk_err = call_tool(&mut state, "command_run", json!({ "command_id": "table.exclude", "params": {} })).map_err(|e| e.to_string())?;
    let unk_sc = unk_err.structured_content.ok_or("missing unknown command structuredContent")?;
    if !unk_sc["remediation"].as_str().unwrap_or("").contains("table.excludeCells") {
        return Err(format!("expected fuzzy table.excludeCells suggestion: {unk_sc}"));
    }

    // Unknown stats kind suggests `one_way_anova` for `"anova_one_way"`.
    let kind_err = call_tool(
        &mut state,
        "command_run",
        json!({ "command_id": "stats.run", "params": { "source_sheet_id": "messy-1", "kind": "anova_one_way" } }),
    )
    .map_err(|e| e.to_string())?;
    let kind_sc = kind_err.structured_content.ok_or("missing bad kind structuredContent")?;
    if !kind_sc["remediation"].as_str().unwrap_or("").contains("one_way_anova") {
        return Err(format!("expected fuzzy one_way_anova suggestion: {kind_sc}"));
    }

    // Disabled toolset RPC error points directly to `enable_toolset` + `command_run`.
    let disabled_rpc = rpc(&mut state, 7, "tools/call", json!({ "name": "sheet_create_data", "arguments": {} }))?;
    if disabled_rpc["error"]["code"] != json!(ERR_INVALID_PARAMS) {
        return Err(format!("expected ERR_INVALID_PARAMS for disabled tool: {disabled_rpc}"));
    }
    let msg = disabled_rpc["error"]["message"].as_str().unwrap_or("");
    if !msg.contains("enable_toolset") || !msg.contains("sheet.createData") {
        return Err(format!("disabled tool error missing actionable recovery hint: {msg}"));
    }

    // Prompt catalog exposes all 3 built-in scientific prompts after prompt enrichment.
    let prompts = list_prompts();
    if prompts.len() != 3 {
        return Err(String::from("expected 3 prompts"));
    }
    Ok(())
}

#[test]
fn resources_read_supports_compact_query_controls() -> Result<(), String> {
    let mut state = McpServerState::new();
    call_json_ok(
        &mut state,
        "command_run",
        json!({ "command_id": "sheet.createData", "params": { "sheet_id": "xy-q", "title": "XY Query", "kind": "xy", "rows": 12, "cols": 1 } }),
    )?;
    let xs: Vec<f64> = (-11..=0).map(f64::from).collect();
    call_json_ok(&mut state, "command_run", json!({ "command_id": "table.setXCells", "params": { "sheet_id": "xy-q", "values": xs } }))?;
    let edits: Vec<Value> = xs
        .iter()
        .enumerate()
        .map(|(r, x)| json!({ "row": r, "col": 0, "subcol": 0, "value": 5.0 + 95.0 / (1.0 + 10.0_f64.powf(x - (-6.0))) }))
        .collect();
    call_json_ok(&mut state, "command_run", json!({ "command_id": "table.setCells", "params": { "sheet_id": "xy-q", "edits": edits } }))?;
    call_json_ok(
        &mut state,
        "command_run",
        json!({ "command_id": "fit.run", "params": { "sheet_id": "fit-q", "source_sheet_id": "xy-q", "equation_id": "log_inhibitor_vs_response_4pl" } }),
    )?;

    let doc = state.session().document();
    let sheet_rows = read_resource(doc, "sheet://xy-q?max_rows=5").map_err(|e| format!("{e}"))?;
    let sv: Value = serde_json::from_str(&sheet_rows[0].text).map_err(|e| e.to_string())?;
    if sv["csv_preview_truncated"] != json!(true) || sv["csv_preview"].as_str().is_none_or(|c| c.lines().count() > 6) {
        return Err(format!("expected sheet max_rows=5 truncation: {sv}"));
    }

    let res_no_curves = read_resource(doc, "result://fit-q?include_curves=false&tab_id=nonlinear_parameters").map_err(|e| format!("{e}"))?;
    let rv: Value = serde_json::from_str(&res_no_curves[0].text).map_err(|e| e.to_string())?;
    if rv["fitted_curves"][0]["points_omitted"] != json!(true) || rv["fitted_curves"][0].get("points").is_some() {
        return Err(format!("expected dense curve points omitted: {rv}"));
    }
    if rv["tabs"].as_array().map(Vec::len) != Some(1) || rv["tabs"][0]["id"] != json!("nonlinear_parameters") {
        return Err(format!("expected tab_id filter on result:// resource: {rv}"));
    }

    assert!(read_resource(doc, "result://fit-q?bogus=1").is_err());
    assert!(read_resource(doc, "result://fit-q?tab_id=nonexistent").is_err());

    // Dense 500-point result projection omits points, slices tab rows + row_labels, and retains eliminated_outliers.
    let mut dense = ProjectDocument::new("Fit Study").map_err(|e| e.to_string())?;
    let mut rs =
        ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    rs.status = ResultStatus::UpToDate;
    rs.scalars.push(ResultScalarEntry { key: String::from("Slope"), data_set_index: Some(0), value: 2.5, units: None });
    let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
    rs.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("R1"), String::from("R2"), String::from("R3")],
        rows: vec![vec![num(1.0)], vec![num(2.0)], vec![num(3.0)]],
        footnotes: vec![String::from("note")],
    });
    rs.tabs.push(ResultTab {
        id: String::from("extra"),
        title: String::from("Extra"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("E1")],
        rows: vec![vec![num(9.0)]],
        footnotes: Vec::new(),
    });
    rs.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("linear"),
        converged: true,
        ambiguous: false,
        points: (0..500)
            .map(|i| CurveSamplePoint { x: f64::from(i), y: f64::from(i) * 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None })
            .collect(),
    });
    dense.add_result_sheet(rs).map_err(|e| e.to_string())?;
    let parsed: Value = serde_json::from_str(
        &read_resource(&dense, "result://r1?include_curves=false&tab_id=summary&max_rows=2").map_err(|e| format!("{e}"))?[0].text,
    )
    .map_err(|e| e.to_string())?;
    assert_eq!(
        (
            parsed["tabs"].as_array().map(Vec::len),
            &parsed["tabs"][0]["id"],
            parsed["tabs"][0]["rows"].as_array().map(Vec::len),
            parsed["tabs"][0]["row_labels"].as_array().map(Vec::len)
        ),
        (Some(1), &json!("summary"), Some(2), Some(2))
    );
    assert_eq!(
        (
            &parsed["fitted_curves"][0]["points_count"],
            &parsed["fitted_curves"][0]["points_omitted"],
            parsed["fitted_curves"][0].get("points").is_none(),
            parsed.get("eliminated_outliers").is_some()
        ),
        (&json!(500), &json!(true), true, true)
    );
    assert!(matches!(read_resource(&dense, "result://r1?tab_id=does_not_exist"), Err(ResourceError::NotFound(_))));

    // Live-attach resources/read query stripping + doc_inspect sections/detail/tab_id/max_rows/max_scalars.
    let res_b64 = encode_base64(serde_json::to_string(&json!({
        "id": "r1", "title": "Fit", "status": "UpToDate",
        "tabs": [
            { "id": "summary", "title": "Summary", "column_headers": ["V"], "row_labels": ["R1", "R2", "R3"], "rows": [[1], [2], [3]], "footnotes": [] },
            { "id": "residuals", "title": "Residuals", "column_headers": ["Res"], "row_labels": ["R1"], "rows": [[0.1]], "footnotes": [] }
        ],
        "fitted_curves": [{ "data_set_index": 0, "equation_id": "linear", "converged": true, "ambiguous": false, "points": [{ "x": 0.0, "y": 1.0 }, { "x": 1.0, "y": 2.0 }] }]
    })).map_err(|e| e.to_string())?.as_bytes());
    let mut live = McpServerState::with_control_port(49808).map_err(|e| e.to_string())?.with_control_handler(move |method, params| match method {
        "state.snapshot" => Ok(json!({ "active_sheet_id": "d1" })),
        "doc.inspect" => match params.get("sheet_id").and_then(Value::as_str) {
            None => Ok(json!({ "title": "Live", "counts": { "total": 2 }, "data_sheets": [{ "id": "d1" }], "result_sheets": [{ "id": "r1", "scalars": [{ "key": "a" }] }], "graph_sheets": [{ "id": "g1", "spec": { "heavy": true } }] })),
            Some("d1") => Ok(json!({ "id": "d1", "section": "Data", "csv_preview": "X,Y\n1,10\n2,20\n3,30\n4,40" })),
            Some("r1") => Ok(json!({ "id": "r1", "section": "Results", "tabs_data": [{ "id": "summary", "rows": [[1], [2], [3]], "row_labels": ["a", "b", "c"] }, { "id": "residuals", "rows": [[0.1]], "row_labels": ["a"] }], "scalars_data": [{ "key": "k1" }, { "key": "k2" }, { "key": "k3" }] })),
            Some(other) => Err(format!("sheet `{other}` not found")),
        },
        "doc.export" => Ok(json!({ "data_base64": res_b64 })),
        _ => Err(format!("unexpected {method}")),
    });
    let sheet_r = dispatch_json_value(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": { "uri": "sheet://d1?max_rows=2" } }),
        &mut live,
    )
    .ok_or("sheet reply")?;
    let sv2: Value = serde_json::from_str(sheet_r["result"]["contents"][0]["text"].as_str().ok_or("text")?).map_err(|e| e.to_string())?;
    assert_eq!((&sv2["csv_preview"], &sv2["csv_preview_truncated"]), (&json!("X,Y\n1,10\n2,20"), &json!(true)));
    let res_r = dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": { "uri": "result://r1?include_curves=false&tab_id=summary&max_rows=1" } }), &mut live).ok_or("res reply")?;
    let rv2: Value = serde_json::from_str(res_r["result"]["contents"][0]["text"].as_str().ok_or("text")?).map_err(|e| e.to_string())?;
    assert_eq!(
        (
            rv2["tabs"].as_array().map(Vec::len),
            rv2["tabs"][0]["rows"].as_array().map(Vec::len),
            &rv2["fitted_curves"][0]["points_omitted"],
            &rv2["fitted_curves"][0]["points_count"]
        ),
        (Some(1), Some(1), &json!(true), &json!(2))
    );
    let bad_tab = dispatch_json_value(
        json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": "result://r1?tab_id=missing" } }),
        &mut live,
    )
    .ok_or("bad tab")?;
    assert_eq!(bad_tab["error"]["code"], json!(-32002));
    let ov = call_json_ok(&mut live, "doc_inspect", json!({ "sections": ["data"], "detail": "compact" }))?;
    assert!(ov.get("data_sheets").is_some() && ov.get("graph_sheets").is_none() && ov.get("result_sheets").is_none());
    let sf = call_json_ok(&mut live, "doc_inspect", json!({ "sheet_id": "r1", "tab_id": "summary", "max_rows": 2, "max_scalars": 1 }))?;
    assert_eq!(
        (
            sf["tabs_data"].as_array().map(Vec::len),
            sf["tabs_data"][0]["rows"].as_array().map(Vec::len),
            sf["scalars_data"].as_array().map(Vec::len),
            &sf["scalars_truncated"]
        ),
        (Some(1), Some(2), Some(1), &json!(true))
    );
    Ok(())
}
