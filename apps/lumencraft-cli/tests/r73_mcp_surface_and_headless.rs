mod r73_common;

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use lumencraft_doc::{
    CellCoord, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, ProjectDocument, SheetCellRange, SheetId,
};
use lumencraft_engine::Session;
use lumencraft_format::{ZipWriter, save_project_file};
use lumencraft_svg::validate_svg_structure;
use lumencraft_ui_egui::UiApp;
use r73_common::{McpChild, TempWorkdir, assert_approx, decode_base64, read_result_json, result_scalar_f64, result_tab, tab_cell_f64};
use serde_json::{Value, json};

fn assert_structured_error_shape(err_sc: &Value, expected_code: &str, context: &str) -> Result<(), String> {
    if err_sc["code"] != json!(expected_code) {
        return Err(format!("{context}: expected error code `{expected_code}`, got {err_sc}"));
    }
    if err_sc["category"].as_str().is_none_or(str::is_empty) {
        return Err(format!("{context}: missing `category` in {err_sc}"));
    }
    if err_sc["message"].as_str().is_none_or(str::is_empty) {
        return Err(format!("{context}: missing `message` in {err_sc}"));
    }
    if err_sc["retryable"].as_bool().is_none() {
        return Err(format!("{context}: missing `retryable` bool in {err_sc}"));
    }
    if err_sc.get("remediation").is_none() {
        return Err(format!("{context}: missing `remediation` field in {err_sc}"));
    }
    if matches!(expected_code, "path_escape" | "gui_only_violation" | "conflict") && err_sc["remediation"].as_str().is_none_or(str::is_empty) {
        return Err(format!("{context}: expected non-empty remediation for `{expected_code}` in {err_sc}"));
    }
    if err_sc.get("details").is_none() {
        return Err(format!("{context}: missing `details` field in {err_sc}"));
    }
    Ok(())
}

#[test]
fn r73_stdio_initialize_ping_ndjson_and_content_length_both_modes() -> Result<(), String> {
    for gui_only in [false, true] {
        let args: &[&str] = if gui_only { &["mcp", "--gui-only"] } else { &["mcp"] };
        let mut child = McpChild::spawn(args)?;
        let init = child.initialize()?;
        if init["serverInfo"]["name"] != json!("lumencraft") {
            return Err(format!("unexpected serverInfo: {init}"));
        }
        if init["capabilities"]["tools"]["listChanged"] != json!(true) {
            return Err(format!("expected tools.listChanged=true: {init}"));
        }
        if init["capabilities"]["resources"]["listChanged"] != json!(false) || init["capabilities"]["prompts"]["listChanged"] != json!(false) {
            return Err(format!("unexpected resource/prompt capabilities: {init}"));
        }
        let instr = init["instructions"].as_str().ok_or("missing initialize instructions")?;
        if gui_only {
            if !instr.contains("--gui-only") || !instr.contains("ui_inspect") || !instr.contains("ui_screenshot") {
                return Err(format!("--gui-only instructions missing GUI contract: {instr}"));
            }
        } else if !instr.contains("doc_inspect") || !instr.contains("list_toolsets") || !instr.contains("enable_toolset") {
            return Err(format!("headless instructions missing workflow hints: {instr}"));
        }

        // Ping over Content-Length and raw NDJSON line framing on the same session.
        let ping_cl = child.request("ping", json!({}))?;
        if ping_cl["result"] != json!({}) {
            return Err(format!("ping Content-Length mismatch: {ping_cl}"));
        }
        let ping_ndjson = child.request_ndjson("ping", json!({}))?;
        if ping_ndjson["result"] != json!({}) {
            return Err(format!("ping NDJSON mismatch: {ping_ndjson}"));
        }
    }
    Ok(())
}

#[test]
fn r73_stdio_progressive_toolsets_and_list_changed_notification() -> Result<(), String> {
    let mut child = McpChild::spawn(&["mcp"])?;
    child.initialize()?;

    let initial_tools = child.request("tools/list", json!({}))?["result"]["tools"].as_array().cloned().ok_or("tools/list missing array")?;
    let initial_names: Vec<&str> = initial_tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for core in [
        "doc_inspect",
        "open_file",
        "save_file",
        "export",
        "render_preview",
        "command_run",
        "command_batch",
        "command_list",
        "list_toolsets",
        "enable_toolset",
    ] {
        if !initial_names.contains(&core) {
            return Err(format!("core tool `{core}` missing from default tools/list: {initial_names:?}"));
        }
    }
    if initial_names.iter().any(|n| n.starts_with("stats_") || n.starts_with("ui_")) {
        return Err(format!("progressive toolsets leaked before enable_toolset: {initial_names:?}"));
    }

    let toolsets = child.call_tool_payload("list_toolsets", json!({}))?;
    let ts_arr = toolsets["toolsets"].as_array().ok_or("list_toolsets missing array")?;
    let groups = ["project", "edit", "sheet", "table", "info", "analysis", "graph", "layout", "export"];
    for group in groups {
        if !ts_arr.iter().any(|t| t["group"] == json!(group)) {
            return Err(format!("toolset `{group}` missing from list_toolsets: {toolsets}"));
        }
    }
    if toolsets["gui_toolset"]["group"] != json!("gui") {
        return Err(format!("missing gui_toolset in list_toolsets: {toolsets}"));
    }

    // Enable every engine command group one by one and confirm list_changed + exposed schemas.
    for group in groups {
        let en = child.call_tool_payload("enable_toolset", json!({ "group": group, "enabled": true }))?;
        if en["changed"] != json!(true) || en["enabled"] != json!(true) {
            return Err(format!("enable_toolset({group}) unexpected result: {en}"));
        }
        let note = child.read_message()?;
        if note["method"] != json!("notifications/tools/list_changed") {
            return Err(format!("enable_toolset({group}) expected notifications/tools/list_changed, got {note}"));
        }
    }

    let expanded = child.request("tools/list", json!({}))?["result"]["tools"].as_array().cloned().ok_or("expanded tools/list missing array")?;
    let exp_names: Vec<&str> = expanded.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected_tool in ["sheet_create_data", "stats_run", "stats_pvalue_stack", "fit_run", "graph_create", "layout_create"] {
        if !exp_names.contains(&expected_tool) {
            return Err(format!("missing enabled tool `{expected_tool}`"));
        }
    }
    for tool in &expanded {
        if tool["inputSchema"]["type"] != json!("object") || !tool["annotations"].is_object() {
            return Err(format!("tool missing valid inputSchema or annotations: {tool}"));
        }
    }

    // Invoke enabled direct tool directly over stdio JSON-RPC.
    let created = child.call_tool_payload("sheet_create_data", json!({ "sheet_id": "ts-col", "kind": "Column", "rows": 4, "cols": 2 }))?;
    if created["payload"]["sheet_id"] != json!("ts-col") {
        return Err(format!("direct sheet_create_data failed: {created}"));
    }

    // Disable `analysis` and confirm stats_run disappears + notification emitted.
    child.call_tool_payload("enable_toolset", json!({ "group": "analysis", "enabled": false }))?;
    let dis_note = child.read_message()?;
    if dis_note["method"] != json!("notifications/tools/list_changed") {
        return Err(format!("disabling toolset expected list_changed notification, got {dis_note}"));
    }
    let after_dis = child.request("tools/list", json!({}))?["result"]["tools"].as_array().cloned().ok_or("tools/list after disable missing array")?;
    if after_dis.iter().any(|t| t["name"] == json!("stats_run")) {
        return Err("stats_run still listed after disabling `analysis`".to_owned());
    }

    // Enable `gui` toolset in normal headless mode -> exposes all 8 ui_* tools, which return Conflict without host/port.
    child.call_tool_payload("enable_toolset", json!({ "group": "gui", "enabled": true }))?;
    let gui_err = child.call_tool_err("ui_inspect", json!({ "scope": "all" }))?;
    assert_structured_error_shape(&gui_err, "conflict", "unattached ui_inspect in headless mode")?;
    Ok(())
}

#[test]
fn r73_stdio_resources_and_prompts_complete_surface() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_res_prompts")?;
    let wd_str = wd.path_str();
    let mut child = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    child.initialize()?;

    child.call_tool_payload(
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "res-col", "title": "Group Assay", "kind": "Column", "rows": 4, "cols": 3 } },
                { "command_id": "table.setTitles", "params": { "sheet_id": "res-col", "titles": ["Ctrl", "Low", "High"] } },
                { "command_id": "table.paste", "params": { "sheet_id": "res-col", "start_row": 0, "start_col": 0, "text": "10\t15\t24\n11\t16\t26\n9\t14\t25\n10\t15\t27" } },
                { "command_id": "stats.run", "params": { "source_sheet_id": "res-col", "sheet_id": "res-anova", "kind": "one_way_anova" } },
                { "command_id": "graph.create", "params": { "data_sheet_id": "res-col", "sheet_id": "res-graph" } },
                { "command_id": "layout.create", "params": { "sheet_id": "res-layout", "title": "Fig 1" } }
            ]
        }),
    )?;

    let res_list = child.request("resources/list", json!({}))?;
    let resources = res_list["result"]["resources"].as_array().ok_or("resources/list missing array")?;
    let uris: Vec<&str> = resources.iter().filter_map(|r| r["uri"].as_str()).collect();
    for expected in ["sheet://res-col", "sheet://res-anova", "result://res-anova", "sheet://res-graph", "sheet://res-layout"] {
        if !uris.contains(&expected) {
            return Err(format!("missing resource URI `{expected}` in {uris:?}"));
        }
    }

    let sheet_read = child.request("resources/read", json!({ "uri": "sheet://res-col" }))?;
    let sheet_text = sheet_read["result"]["contents"][0]["text"].as_str().ok_or("missing sheet:// text")?;
    if !sheet_text.contains("Ctrl") || !sheet_text.contains("27") {
        return Err(format!("sheet://res-col content incomplete: {sheet_text}"));
    }

    let rs_json = read_result_json(&mut child, "res-anova")?;
    if rs_json["id"] != json!("res-anova") || result_scalar_f64(&rs_json, "P value")? >= 0.001 {
        return Err(format!("unexpected result://res-anova JSON: {rs_json}"));
    }

    // Invalid URI -> -32602, Missing resource -> -32002.
    let bad_uri = child.request("resources/read", json!({ "uri": "http://bad/uri" }))?;
    if bad_uri["error"]["code"] != json!(-32602) {
        return Err(format!("expected -32602 for bad URI, got {bad_uri}"));
    }
    let missing_uri = child.request("resources/read", json!({ "uri": "result://nonexistent-rs" }))?;
    if missing_uri["error"]["code"] != json!(-32002) {
        return Err(format!("expected -32002 for missing resource, got {missing_uri}"));
    }

    // Prompts discovery & invocation.
    let plist = child.request("prompts/list", json!({}))?;
    let prompts = plist["result"]["prompts"].as_array().ok_or("prompts/list missing array")?;
    let pnames: Vec<&str> = prompts.iter().filter_map(|p| p["name"].as_str()).collect();
    for expected in ["dose_response_curve_fit", "choose_two_group_test", "publication_figure_checklist"] {
        if !pnames.contains(&expected) {
            return Err(format!("missing prompt `{expected}` in {pnames:?}"));
        }
    }

    let p1 = child
        .request("prompts/get", json!({ "name": "dose_response_curve_fit", "arguments": { "sheet_id": "xy-1", "response_type": "inhibitor" } }))?;
    let p1_txt = p1["result"]["messages"][0]["content"]["text"].as_str().ok_or("missing prompt message text")?;
    if !p1_txt.contains("xy-1") || !p1_txt.contains("log_inhibitor_vs_response_4pl") {
        return Err(format!("dose_response_curve_fit content mismatch: {p1_txt}"));
    }
    let p2 = child.request(
        "prompts/get",
        json!({ "name": "choose_two_group_test", "arguments": { "sheet_id": "res-col", "paired": "true", "parametric": "false" } }),
    )?;
    let p2_txt = p2["result"]["messages"][0]["content"]["text"].as_str().ok_or("missing choose_two_group_test text")?;
    if !p2_txt.contains("Wilcoxon") {
        return Err(format!("choose_two_group_test expected Wilcoxon recommendation: {p2_txt}"));
    }
    let p3 = child.request(
        "prompts/get",
        json!({ "name": "publication_figure_checklist", "arguments": { "graph_sheet_id": "res-graph", "target_format": "pdf" } }),
    )?;
    let p3_txt = p3["result"]["messages"][0]["content"]["text"].as_str().ok_or("missing publication_figure_checklist text")?;
    if !p3_txt.contains("res-graph") || !p3_txt.contains("PDF") {
        return Err(format!("publication_figure_checklist mismatch: {p3_txt}"));
    }

    let bad_prompt = child.request("prompts/get", json!({ "name": "unknown_prompt" }))?;
    if bad_prompt["error"]["code"] != json!(-32602) {
        return Err(format!("expected -32602 for unknown prompt: {bad_prompt}"));
    }
    let bad_arg = child.request("prompts/get", json!({ "name": "choose_two_group_test", "arguments": { "paired": 123 } }))?;
    if bad_arg["error"]["code"] != json!(-32602) {
        return Err(format!("expected -32602 for non-string prompt arg: {bad_arg}"));
    }
    Ok(())
}

#[test]
fn r73_stdio_headless_workflows_command_run_batch_save_reopen_export() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_headless_wf")?;
    let wd_str = wd.path_str();
    let mut child = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    child.initialize()?;

    // 1. atomic batch rollback on mid-batch error
    child.call_tool_payload(
        "command_run",
        json!({ "command_id": "sheet.createData", "params": { "sheet_id": "base-col", "kind": "Column", "rows": 3, "cols": 2 } }),
    )?;
    let atomic_err = child.call_tool_err(
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "transient-sheet", "kind": "Column", "rows": 2, "cols": 2 } },
                { "command_id": "stats.run", "params": { "source_sheet_id": "missing-sheet-xyz", "kind": "one_way_anova" } }
            ]
        }),
    )?;
    assert_structured_error_shape(&atomic_err, "not_found", "atomic batch rollback")?;
    let doc_after_rollback = child.call_tool_payload("doc_inspect", json!({}))?;
    if doc_after_rollback["data_sheets"].as_array().is_some_and(|arr| arr.iter().any(|s| s["id"] == json!("transient-sheet"))) {
        return Err("atomic rollback leaked transient-sheet".to_owned());
    }

    // 2. End-to-end headless scientific workflows via command_run + command_batch:
    // Column ANOVA, P-value stack BH, XY 4PL dose-response + constrained fit, MV PCA + GLM, Survival KM, Graph + Layout.
    child.call_tool_payload(
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "table.setTitles", "params": { "sheet_id": "base-col", "titles": ["Ctrl", "DrugA"] } },
                { "command_id": "table.paste", "params": { "sheet_id": "base-col", "start_row": 0, "start_col": 0, "text": "10.0\t22.0\n11.0\t24.0\n12.0\t26.0" } },
                { "command_id": "stats.run", "params": { "source_sheet_id": "base-col", "sheet_id": "rs-tg", "kind": "welch_t" } },
                { "command_id": "sheet.createData", "params": { "sheet_id": "pval-col", "kind": "Column", "rows": 4, "cols": 1 } },
                { "command_id": "table.paste", "params": { "sheet_id": "pval-col", "start_row": 0, "start_col": 0, "text": "0.001\n0.012\n0.045\n0.400" } },
                { "command_id": "stats.pvalueStack", "params": { "source_sheet_id": "pval-col", "sheet_id": "rs-pstack", "method": "benjamini_hochberg", "threshold": 0.05 } },
                { "command_id": "sheet.createData", "params": { "sheet_id": "xy-4pl", "kind": "Xy", "rows": 7, "cols": 1 } },
                { "command_id": "table.setXCells", "params": { "sheet_id": "xy-4pl", "values": [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0] } },
                { "command_id": "table.paste", "params": { "sheet_id": "xy-4pl", "start_row": 0, "start_col": 0, "text": "98\n95\n82\n50\n18\n5\n2" } },
                { "command_id": "fit.run", "params": { "source_sheet_id": "xy-4pl", "sheet_id": "rs-4pl", "equation_id": "log_inhibitor_vs_response_4pl", "profile_likelihood_ci": true } },
                { "command_id": "graph.create", "params": { "data_sheet_id": "xy-4pl", "sheet_id": "g-4pl" } },
                { "command_id": "graph.setRegressionOverlay", "params": { "sheet_id": "g-4pl", "result_sheet_id": "rs-4pl", "series_index": 0, "show_confidence_band": true } },
                { "command_id": "layout.create", "params": { "sheet_id": "lay-main", "title": "Headless Plate" } },
                { "command_id": "layout.addPanel", "params": { "sheet_id": "lay-main", "kind": "graph", "graph_sheet_id": "g-4pl", "bounds_pt": [20.0, 20.0, 240.0, 160.0] } },
                { "command_id": "sheet.createData", "params": { "sheet_id": "anova-col", "kind": "Column", "rows": 4, "cols": 3 } },
                { "command_id": "table.paste", "params": { "sheet_id": "anova-col", "start_row": 0, "start_col": 0, "text": "10\t15\t25\n11\t16\t26\n9\t14\t24\n10\t15\t27" } },
                { "command_id": "stats.run", "params": { "source_sheet_id": "anova-col", "sheet_id": "rs-anova", "kind": "one_way_anova" } }
            ]
        }),
    )?;

    // Also verify non-atomic command_batch continues past error and preserves prior successful mutation.
    let non_atomic = child.call_tool_err(
        "command_batch",
        json!({
            "atomic": false,
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "kept-non-atomic", "kind": "Column", "rows": 2, "cols": 2 } },
                { "command_id": "stats.run", "params": { "source_sheet_id": "nonexistent-na", "kind": "one_way_anova" } }
            ]
        }),
    )?;
    if non_atomic["atomic"] != json!(false) || non_atomic["completed"] != json!(1) {
        return Err(format!("expected non-atomic batch with completed=1: {non_atomic}"));
    }
    let na_check = child.call_tool_payload("doc_inspect", json!({ "sheet_id": "kept-non-atomic" }))?;
    if na_check["id"] != json!("kept-non-atomic") {
        return Err(format!("non-atomic batch should retain kept-non-atomic: {na_check}"));
    }

    // Save `.lumencraft`, `.lumencraft.d`, and `.lctemplate` inside --workdir.
    child.call_tool_payload("save_file", json!({ "path": "headless.lumencraft" }))?;
    child.call_tool_payload("save_file", json!({ "path": "headless.lumencraft.d", "format": "lumencraft.d" }))?;
    child.call_tool_payload("save_file", json!({ "path": "headless.lctemplate", "format": "lctemplate" }))?;

    // Spawn a fresh headless `lumencraft-cli mcp` child process and reopen `headless.lumencraft`.
    let mut reopened = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    reopened.initialize()?;
    reopened.call_tool_payload("open_file", json!({ "path": "headless.lumencraft" }))?;

    let rs_4pl = read_result_json(&mut reopened, "rs-4pl")?;
    let logic50 = tab_cell_f64(result_tab(&rs_4pl, "nonlinear_parameters")?, "LogIC50", "Best-fit value")?;
    assert_approx(logic50, -6.0, 0.15, "reopened headless 4PL LogIC50")?;
    let rs_tg = read_result_json(&mut reopened, "rs-tg")?;
    if result_scalar_f64(&rs_tg, "PValue")? >= 0.01 {
        return Err("reopened Welch two-group PValue unexpectedly >= 0.01".to_owned());
    }
    let rs_pstack = read_result_json(&mut reopened, "rs-pstack")?;
    if (result_scalar_f64(&rs_pstack, "TotalDiscoveries")? - 2.0).abs() > 1e-9 {
        return Err(format!("reopened p-value stack TotalDiscoveries mismatch: {rs_pstack}"));
    }

    // Export all publication formats (SVG, PDF 1.7, PNG, TIFF, CSV tab/tidy, JSON) to disk inside --workdir.
    reopened.call_tool_payload("export", json!({ "format": "svg", "sheet_id": "g-4pl", "path": "exports/fig.svg" }))?;
    reopened.call_tool_payload("export", json!({ "format": "pdf", "sheet_id": "lay-main", "path": "exports/fig.pdf" }))?;
    reopened.call_tool_payload(
        "export",
        json!({ "format": "png", "sheet_id": "g-4pl", "dpi": 144, "width_px": 640, "height_px": 420, "path": "exports/fig.png" }),
    )?;
    reopened.call_tool_payload(
        "export",
        json!({ "format": "tiff", "sheet_id": "g-4pl", "dpi": 144, "width_px": 320, "height_px": 240, "path": "exports/fig.tiff" }),
    )?;
    reopened.call_tool_payload(
        "export",
        json!({ "format": "csv", "sheet_id": "rs-4pl", "tab_id": "nonlinear_parameters", "path": "exports/params.csv" }),
    )?;
    reopened.call_tool_payload("export", json!({ "format": "csv", "sheet_id": "rs-pstack", "tidy": true, "path": "exports/pstack_tidy.csv" }))?;
    reopened.call_tool_payload("export", json!({ "format": "json", "sheet_id": "rs-4pl", "pretty": true, "path": "exports/rs4pl.json" }))?;

    let svg_str = std::fs::read_to_string(wd.dir.join("exports/fig.svg")).map_err(|e| format!("read fig.svg: {e}"))?;
    validate_svg_structure(&svg_str).map_err(|e| format!("invalid fig.svg: {e}"))?;
    let pdf_bytes = std::fs::read(wd.dir.join("exports/fig.pdf")).map_err(|e| format!("read fig.pdf: {e}"))?;
    if !pdf_bytes.starts_with(b"%PDF-1.7") || !pdf_bytes.windows(5).any(|w| w == b"%%EOF") {
        return Err("exported fig.pdf missing %PDF-1.7 or %%EOF".to_owned());
    }
    let png_bytes = std::fs::read(wd.dir.join("exports/fig.png")).map_err(|e| format!("read fig.png: {e}"))?;
    if !png_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("exported fig.png missing PNG magic bytes".to_owned());
    }
    let tiff_bytes = std::fs::read(wd.dir.join("exports/fig.tiff")).map_err(|e| format!("read fig.tiff: {e}"))?;
    if !(tiff_bytes.starts_with(b"II*\0") || tiff_bytes.starts_with(b"MM\0*")) {
        return Err("exported fig.tiff missing TIFF magic bytes".to_owned());
    }
    let csv_params = std::fs::read_to_string(wd.dir.join("exports/params.csv")).map_err(|e| format!("read params.csv: {e}"))?;
    if !csv_params.contains("LogIC50") {
        return Err(format!("params.csv missing LogIC50: {csv_params}"));
    }

    // Also exercise render_preview and multi-format open_file (.lumencraft.d, .lctemplate, .csv, .tsv, append=true).
    let preview = reopened.call_tool_ok("render_preview", json!({ "sheet_id": "g-4pl", "width_px": 320, "height_px": 240 }))?;
    let img_b64 = preview["content"]
        .as_array()
        .and_then(|arr| arr.iter().find(|b| b["type"] == json!("image")))
        .and_then(|b| b["data"].as_str())
        .ok_or("render_preview missing PNG image content")?;
    if !decode_base64(img_b64)?.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("render_preview PNG magic mismatch".to_owned());
    }

    std::fs::write(wd.dir.join("sample.csv"), "A,B\n1,2\n3,4\n").map_err(|e| format!("write sample.csv: {e}"))?;
    std::fs::write(wd.dir.join("sample.tsv"), "A\tB\n5\t6\n7\t8\n").map_err(|e| format!("write sample.tsv: {e}"))?;
    let pzfx_xml = r#"<PrismFile>
  <Title>Headless Enzyme Assay</Title>
  <TableSequence>
    <Table ID="tbl1" TableType="XY">
      <Title>Velocity</Title>
      <XColumn><Title>X</Title><Subcolumn><d>1</d><d>5</d><d>10</d></Subcolumn></XColumn>
      <YColumn><Title>Y</Title><Subcolumn><d>4.2</d><d>14.1</d><d>21.3</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    std::fs::write(wd.dir.join("assay.pzfx"), pzfx_xml).map_err(|e| format!("write pzfx: {e}"))?;
    let mut zw = ZipWriter::new();
    zw.add_file(
        "manifest.json",
        br#"{"title":"Headless Prism","sheets":[{"id":"dose","title":"Dose","kind":"xy","csv_path":"tables/dose.csv"}]}"#.to_vec(),
    )
    .map_err(|e| format!("zip manifest: {e}"))?;
    zw.add_file("tables/dose.csv", b"Dose,Response\n-8,5\n-6,50\n-4,95\n".to_vec()).map_err(|e| format!("zip table: {e}"))?;
    std::fs::write(wd.dir.join("bundle.prism"), zw.finish().map_err(|e| format!("zip finish: {e}"))?).map_err(|e| format!("write prism: {e}"))?;

    reopened.call_tool_payload("open_file", json!({ "path": "headless.lumencraft.d" }))?;
    reopened.call_tool_payload("open_file", json!({ "path": "headless.lctemplate" }))?;
    let opened_pzfx = reopened.call_tool_payload("open_file", json!({ "path": "assay.pzfx" }))?;
    if opened_pzfx["format"] != json!("pzfx") {
        return Err(format!("unexpected pzfx open response: {opened_pzfx}"));
    }
    let opened_prism = reopened.call_tool_payload("open_file", json!({ "path": "bundle.prism" }))?;
    if opened_prism["format"] != json!("prism") {
        return Err(format!("unexpected prism open response: {opened_prism}"));
    }
    reopened.call_tool_payload("open_file", json!({ "path": "sample.csv" }))?;
    reopened.call_tool_payload("open_file", json!({ "path": "sample.tsv", "append": true, "sheet_id": "appended-tsv" }))?;
    let doc_final = reopened.call_tool_payload("doc_inspect", json!({ "sheet_id": "appended-tsv", "include_cells": true }))?;
    if doc_final["id"] != json!("appended-tsv") || doc_final["rows"] != json!(2) {
        return Err(format!("append tsv missing in doc_inspect: {doc_final}"));
    }
    Ok(())
}

#[test]
fn r73_stdio_workdir_guard_and_structured_errors() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_wd_guard")?;
    let outside = TempWorkdir::new("r73_wd_outside")?;
    let wd_str = wd.path_str();
    let outside_file = outside.dir.join("secret.lumencraft");
    std::fs::write(&outside_file, b"not-a-project").map_err(|e| format!("write outside file: {e}"))?;

    #[cfg(unix)]
    {
        let link_path = wd.dir.join("sym-escape.lumencraft");
        std::os::unix::fs::symlink(&outside_file, &link_path).map_err(|e| format!("create symlink: {e}"))?;
    }

    let mut headless = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    headless.initialize()?;
    headless.call_tool_payload(
        "command_run",
        json!({ "command_id": "sheet.createData", "params": { "sheet_id": "wd-s1", "kind": "Column", "rows": 2, "cols": 2 } }),
    )?;

    let outside_str = outside_file.to_string_lossy().into_owned();
    for (tool, args) in [
        ("save_file", json!({ "path": "../escape.lumencraft" })),
        ("open_file", json!({ "path": &outside_str })),
        ("export", json!({ "format": "svg", "path": "../escape.svg" })),
        ("command_run", json!({ "command_id": "project.save", "params": { "path": "../cmd_escape.lumencraft" } })),
    ] {
        let err = headless.call_tool_err(tool, args)?;
        assert_structured_error_shape(&err, "path_escape", &format!("headless {tool}"))?;
    }
    #[cfg(unix)]
    {
        let sym_err = headless.call_tool_err("open_file", json!({ "path": "sym-escape.lumencraft" }))?;
        assert_structured_error_shape(&sym_err, "path_escape", "symlink escape")?;
    }

    // Stored external link escape inside an archive opened under --workdir is blocked on project.recalculate.
    let escape_csv = outside.dir.join("external_link.csv");
    std::fs::write(&escape_csv, "A,B\n1,2\n").map_err(|e| format!("write external csv: {e}"))?;
    let mut link_builder = Session::from_document(ProjectDocument::new("Linked").map_err(|e| e.to_string())?);
    link_builder
        .execute("sheet.createData", json!({ "sheet_id": "linked-ds", "title": "Linked", "kind": "Column", "rows": 2, "cols": 1 }))
        .map_err(|e| e.to_string())?;
    let mut link_doc = link_builder.document().clone();
    link_doc.data_sheet_mut(&SheetId::new("linked-ds")).ok_or("missing linked-ds")?.linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: escape_csv.to_string_lossy().into_owned(), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("external link"),
    });
    let linked_archive = wd.dir.join("with_external_link.lumencraft");
    save_project_file(&linked_archive, &link_doc).map_err(|e| format!("save linked archive: {e}"))?;

    headless.call_tool_payload("open_file", json!({ "path": "with_external_link.lumencraft" }))?;
    let recalc_err =
        headless.call_tool_err("command_run", json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }))?;
    assert_structured_error_shape(&recalc_err, "path_escape", "stored external link escape")?;

    // Strict --gui-only --workdir enforces WorkdirGuard on File/Save & graph.exportSvg too!
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;
    gui.create_table(json!({ "sheet_id": "gui-wd-ds", "kind": "Column", "rows": 2, "cols": 2, "auto_graph": true, "graph_sheet_id": "gui-wd-g" }))?;
    let gui_save_esc = gui.ui_invoke_err("project.save", json!({ "path": "../gui_escape.lumencraft" }))?;
    assert_structured_error_shape(&gui_save_esc, "path_escape", "--gui-only File/Save escape")?;
    let gui_open_esc = gui.ui_invoke_err("project.open", json!({ "path": &outside_str }))?;
    assert_structured_error_shape(&gui_open_esc, "path_escape", "--gui-only File/Open escape")?;
    let gui_svg_esc = gui.ui_invoke_err("graph.exportSvg", json!({ "sheet_id": "gui-wd-g", "path": "../gui_escape.svg" }))?;
    assert_structured_error_shape(&gui_svg_esc, "path_escape", "--gui-only exportSvg escape")?;

    gui.ui_invoke_path(&["File", "Save"], json!({ "path": "inside_ok.lumencraft" }))?;
    if !wd.dir.join("inside_ok.lumencraft").is_file() {
        return Err("expected --gui-only --workdir File/Save inside workdir to write file".to_owned());
    }
    Ok(())
}

#[test]
fn r73_stdio_strict_gui_only_rejections_and_control_port_parity() -> Result<(), String> {
    let wd = TempWorkdir::new("r73_gui_reject")?;
    let wd_str = wd.path_str();
    let mut gui = McpChild::spawn(&["mcp", "--gui-only", "--workdir", &wd_str])?;
    gui.initialize()?;

    let tools_resp = gui.request("tools/list", json!({}))?;
    let tools = tools_resp["result"]["tools"].as_array().ok_or("tools/list missing array")?;
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    if names
        != [
            "ui_inspect",
            "ui_screenshot",
            "ui_pointer_gesture",
            "ui_type_text",
            "ui_press_key",
            "ui_invoke_menu",
            "ui_switch_tab",
            "ui_answer_dialog",
        ]
    {
        return Err(format!("unexpected --gui-only tools/list: {names:?}"));
    }

    // Resources disabled in --gui-only
    let res_list = gui.request("resources/list", json!({}))?;
    if res_list["result"]["resources"] != json!([]) {
        return Err(format!("expected empty resources/list in --gui-only: {res_list}"));
    }
    let res_read = gui.request("resources/read", json!({ "uri": "sheet://data-1" }))?;
    if res_read["error"]["code"] != json!(-32600) || res_read["error"]["data"]["code"] != json!("gui_only_violation") {
        return Err(format!("expected -32600 gui_only_violation for resources/read: {res_read}"));
    }

    // All direct L5 / inspection / toolset tools strictly rejected
    for (tool, args) in [
        ("doc_inspect", json!({})),
        ("open_file", json!({ "path": "a.lumencraft" })),
        ("save_file", json!({ "path": "b.lumencraft" })),
        ("export", json!({ "format": "csv" })),
        ("render_preview", json!({})),
        ("command_run", json!({ "command_id": "sheet.createData", "params": { "kind": "Column" } })),
        ("command_batch", json!({ "commands": [] })),
        ("command_list", json!({})),
        ("list_toolsets", json!({})),
        ("enable_toolset", json!({ "toolset": "stats", "enabled": true })),
        ("sheet_create_data", json!({ "kind": "Column" })),
        ("stats_run", json!({ "sheet_id": "data-1" })),
        ("fit_run", json!({ "sheet_id": "data-1" })),
    ] {
        let err = gui.call_tool_err(tool, args)?;
        assert_structured_error_shape(&err, "gui_only_violation", &format!("--gui-only reject {tool}"))?;
    }

    // Disk ui_screenshot `path` strictly rejected with gui_only_violation and writes nothing to disk.
    let shot_err = gui.call_tool_err("ui_screenshot", json!({ "target": "workspace", "path": "forbidden.png" }))?;
    assert_structured_error_shape(&shot_err, "gui_only_violation", "--gui-only ui_screenshot path")?;
    if wd.dir.join("forbidden.png").exists() {
        return Err("ui_screenshot path wrote file under --gui-only".to_owned());
    }

    // Loopback `--control-port <N> --gui-only` attaches cleanly and rejects L5 / disk bypasses.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(format!("bind control TcpListener: {err}")),
    };
    let addr = listener.local_addr().map_err(|e| format!("local_addr: {e}"))?;
    let port = addr.port();
    if let Err(err) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        if err.kind() == ErrorKind::PermissionDenied {
            return Ok(());
        }
        return Err(format!("probe control connect: {err}"));
    }
    let _ = listener.accept();
    listener.set_nonblocking(true).map_err(|e| format!("set_nonblocking: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);

    let server_thread = thread::spawn(move || -> Result<(), String> {
        let mut app = UiApp::new();
        app.set_gui_only(true);
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            let stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(err) => return Err(err.to_string()),
            };
            stream.set_nonblocking(false).map_err(|e| e.to_string())?;
            let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
            let reader = BufReader::new(stream);
            for line_res in reader.lines() {
                let line = line_res.map_err(|e| e.to_string())?;
                if line.trim().is_empty() {
                    continue;
                }
                let reply = app.dispatch_control_line(&line);
                writer.write_all(reply.as_bytes()).map_err(|e| e.to_string())?;
                writer.write_all(b"\n").map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
            }
        }
    });

    let port_str = port.to_string();
    let mut attached = McpChild::spawn(&["mcp", "--control-port", &port_str, "--gui-only"])?;
    attached.initialize()?;
    let insp_raw = attached.call_tool_raw("ui_inspect", json!({ "scope": "all", "include_bounds": true }))?;
    if insp_raw["isError"] == json!(true)
        && insp_raw["structuredContent"]["code"] == json!("io_error")
        && insp_raw["structuredContent"]["message"].as_str().is_some_and(|m| m.contains("Operation not permitted") || m.contains("Permission denied"))
    {
        stop.store(true, Ordering::Relaxed);
        drop(attached);
        server_thread.join().map_err(|_| String::from("TCP server thread panicked"))??;
        return Ok(());
    }
    if insp_raw["isError"] == json!(true) {
        stop.store(true, Ordering::Relaxed);
        let _ = server_thread.join();
        return Err(format!("attached ui_inspect failed: {insp_raw}"));
    }
    let insp_text = insp_raw["content"][0]["text"].as_str().ok_or("missing attached ui_inspect text")?;
    let insp: Value = serde_json::from_str(insp_text).map_err(|e| format!("parse attached inspect: {e}"))?;
    if insp["gui_only"] != json!(true) {
        return Err(format!("expected live-control gui_only=true, got {insp}"));
    }

    let shot = attached.ui_screenshot("full_window", 480, 320)?;
    if shot.len() < 128 {
        return Err("expected valid PNG bytes over --control-port --gui-only".to_owned());
    }
    attached.create_table(json!({ "sheet_id": "tcp-col", "kind": "Column", "rows": 3, "cols": 2 }))?;
    attached.paste_tsv("tcp-col", 0, 0, 0, "10\t20\n12\t22\n14\t24")?;
    let tcp_doc_err = attached.call_tool_err("doc_inspect", json!({}))?;
    assert_structured_error_shape(&tcp_doc_err, "gui_only_violation", "TCP --gui-only doc_inspect")?;
    let tcp_shot_err = attached.call_tool_err("ui_screenshot", json!({ "target": "workspace", "path": "tcp_forbidden.png" }))?;
    assert_structured_error_shape(&tcp_shot_err, "gui_only_violation", "TCP --gui-only ui_screenshot path")?;

    // Verify raw loopback TCP control protocol rejects mutating/L5 & disk writes while keeping read-only inspection.
    {
        let forbidden_shot = wd.dir.join("direct_tcp_forbidden.png");
        let mut raw_sock = TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("connect direct TCP socket: {e}"))?;
        let mut reader = BufReader::new(raw_sock.try_clone().map_err(|e| format!("clone tcp: {e}"))?);
        let mut send_tcp = |req_id: &str, method: &str, params: Value| -> Result<Value, String> {
            let line =
                serde_json::to_string(&json!({ "id": req_id, "method": method, "params": params })).map_err(|e| format!("serialize tcp req: {e}"))?;
            raw_sock.write_all(line.as_bytes()).map_err(|e| format!("write tcp: {e}"))?;
            raw_sock.write_all(b"\n").map_err(|e| format!("write tcp nl: {e}"))?;
            raw_sock.flush().map_err(|e| format!("flush tcp: {e}"))?;
            let mut reply_line = String::new();
            reader.read_line(&mut reply_line).map_err(|e| format!("read tcp reply: {e}"))?;
            serde_json::from_str(&reply_line).map_err(|e| format!("parse tcp reply `{reply_line}`: {e}"))
        };

        for (req_id, method, params) in [
            ("b1", "sheet.createData", json!({ "kind": "Column" })),
            ("b2", "engine.execute", json!({ "command_id": "sheet.createData", "params": { "kind": "Column" } })),
            ("b3", "command.run", json!({ "command_id": "sheet.createData", "params": { "kind": "Column" } })),
            ("b4", "command.batch", json!({ "commands": [{ "command": "sheet.createData", "params": { "kind": "Column" } }] })),
            ("b5", "ui.theme.set", json!({ "mode": "dark" })),
            ("b6", "doc.export", json!({ "format": "csv" })),
            ("b7", "render.preview", json!({})),
            ("b8", "ui.screenshot", json!({ "target": "full_window", "path": forbidden_shot })),
        ] {
            let reply = send_tcp(req_id, method, params)?;
            if reply["ok"] != json!(false) || reply["error_detail"]["code"] != json!("gui_only_violation") {
                return Err(format!("expected direct TCP `{method}` gui_only_violation, got {reply}"));
            }
        }
        if forbidden_shot.exists() {
            return Err("direct TCP ui.screenshot path wrote file under --gui-only".to_owned());
        }

        for (req_id, method, expect_gui_only) in [("r1", "ping", true), ("r2", "doc.inspect", false), ("r3", "state.snapshot", true)] {
            let reply = send_tcp(req_id, method, json!({}))?;
            if reply["ok"] != json!(true) {
                return Err(format!("expected read-only direct TCP `{method}` ok=true, got {reply}"));
            }
            if expect_gui_only && reply["result"]["gui_only"] != json!(true) {
                return Err(format!("expected read-only direct TCP `{method}` gui_only=true, got {reply}"));
            }
        }
    }

    stop.store(true, Ordering::Relaxed);
    drop(attached);
    server_thread.join().map_err(|_| String::from("TCP server thread panicked"))??;
    Ok(())
}
