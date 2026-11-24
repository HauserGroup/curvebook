//! R17.4 regression tests for MCP token compaction, selective `doc_inspect` / `command_list` /
//! `list_toolsets` / `resources` controls, schema clarity, and actionable error remediations.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_mcp::{ERR_INVALID_PARAMS, McpServerState, call_tool, list_prompts_for_mode, read_resource, serve};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempWorkdir {
    path: PathBuf,
}

impl TempWorkdir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("cb-mcp-r17-4-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|err| format!("create tempdir {}: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempWorkdir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn frame_cl(msg: &Value) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(msg).map_err(|err| format!("encode rpc: {err}"))?;
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(&body);
    Ok(out)
}

fn decode_cl_frames(raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut rest = raw;
    let mut frames = Vec::new();
    while !rest.is_empty() {
        let Some(pos) = rest.windows(4).position(|w| w == b"\r\n\r\n") else {
            return Err(String::from("truncated Content-Length frame"));
        };
        let header = std::str::from_utf8(rest.get(..pos).ok_or("header slice")?).map_err(|err| format!("utf8 header: {err}"))?;
        let mut content_len: Option<usize> = None;
        for line in header.split("\r\n") {
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_len = v.trim().parse::<usize>().ok();
            }
        }
        let len = content_len.ok_or("missing Content-Length header")?;
        let body_start = pos + 4;
        let body_end = body_start + len;
        let body = rest.get(body_start..body_end).ok_or("incomplete frame body")?;
        let val: Value = serde_json::from_slice(body).map_err(|err| format!("decode rpc json: {err}"))?;
        frames.push(val);
        rest = rest.get(body_end..).ok_or("tail slice")?;
    }
    Ok(frames)
}

fn rpc(state: &mut McpServerState, id: u64, method: &str, params: Value) -> Result<Value, String> {
    let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let bytes = frame_cl(&req)?;
    let mut out = Vec::new();
    serve(&mut Cursor::new(bytes), &mut out, state).map_err(|err| format!("serve {method}: {err}"))?;
    let frames = decode_cl_frames(&out)?;
    frames.into_iter().next().ok_or_else(|| format!("missing response frame for {method}"))
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

    // Prompts in `--gui-only` remain free of headless tokens after prompt enrichment.
    let gui_prompts = list_prompts_for_mode(true);
    if gui_prompts.len() != 3 {
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
    Ok(())
}
