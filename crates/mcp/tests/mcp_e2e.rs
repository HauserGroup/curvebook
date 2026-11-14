use std::fs;
use std::io::{BufRead, BufReader, Cursor, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use lumencraft_format::ZipWriter;
use lumencraft_mcp::ui_catalog::UI_COMMANDS;
use lumencraft_mcp::{McpServerState, serve};
use lumencraft_svg::validate_svg_structure;
use serde_json::{Value, json};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("lumencraft-{prefix}-{}-{nanos}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        Self { path: dir }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn frame_cl(val: &Value) -> Result<Vec<u8>, String> {
    let body = serde_json::to_string(val).map_err(|err| format!("serialize request: {err}"))?;
    Ok(format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes())
}

fn decode_cl_frames(raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut cursor = raw;
    let mut out = Vec::new();
    while !cursor.is_empty() {
        let text = std::str::from_utf8(cursor).map_err(|err| format!("valid utf8: {err}"))?;
        let (hdr, rest) = text.split_once("\r\n\r\n").ok_or_else(|| String::from("header separator"))?;
        let len_str = hdr.strip_prefix("Content-Length: ").ok_or_else(|| String::from("content-length header"))?.trim();
        let len: usize = len_str.parse().map_err(|err| format!("valid integer: {err}"))?;
        if rest.len() < len {
            return Err(format!("truncated body: expected {len} bytes, found {}", rest.len()));
        }
        let body = &rest.as_bytes()[..len];
        out.push(serde_json::from_slice(body).map_err(|err| format!("json body: {err}"))?);
        cursor = &rest.as_bytes()[len..];
    }
    Ok(out)
}

fn run_cl_rpc(state: &mut McpServerState, req: &Value) -> Result<Value, String> {
    let input = frame_cl(req)?;
    let mut output = Vec::new();
    serve(&mut Cursor::new(input), &mut output, state).map_err(|err| format!("serve: {err}"))?;
    let mut replies = decode_cl_frames(&output)?;
    if replies.len() != 1 {
        return Err(format!("expected 1 response for request {req}, got {}", replies.len()));
    }
    Ok(replies.remove(0))
}

fn call_tool_rpc(state: &mut McpServerState, id: i64, name: &str, arguments: Value) -> Result<Value, String> {
    let resp = run_cl_rpc(
        state,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            }
        }),
    )?;
    if resp.get("error").is_some() {
        return Err(format!("unexpected JSON-RPC error for {name}: {resp}"));
    }
    Ok(resp["result"].clone())
}

fn first_text_json(tool_res: &Value) -> Result<Value, String> {
    if tool_res["isError"] != json!(false) {
        return Err(format!("tool failed: {tool_res}"));
    }
    let text = tool_res["content"][0]["text"].as_str().ok_or_else(|| String::from("first block text missing"))?;
    serde_json::from_str(text).map_err(|err| format!("valid JSON in tool text block: {err}"))
}

fn decode_base64_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(format!("invalid base64 byte {byte}")),
    }
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let clean: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let (chunks, rem) = clean.as_chunks::<4>();
    if !rem.is_empty() {
        return Err(String::from("base64 length must be divisible by 4"));
    }
    let mut out = Vec::with_capacity(chunks.len() * 3);
    for chunk in chunks {
        let a = decode_base64_nibble(chunk[0])?;
        let b = decode_base64_nibble(chunk[1])?;
        let pad_c = chunk[2] == b'=';
        let pad_d = chunk[3] == b'=';
        let c = if pad_c { 0 } else { decode_base64_nibble(chunk[2])? };
        let d = if pad_d { 0 } else { decode_base64_nibble(chunk[3])? };
        let triple = (u32::from(a) << 18) | (u32::from(b) << 12) | (u32::from(c) << 6) | u32::from(d);
        out.push(((triple >> 16) & 0xFF) as u8);
        if !pad_c {
            out.push(((triple >> 8) & 0xFF) as u8);
        }
        if !pad_d {
            out.push((triple & 0xFF) as u8);
        }
    }
    Ok(out)
}

fn slice0_handshake_and_catalog(state: &mut McpServerState) -> Result<(), String> {
    let mut handshake = Vec::new();
    handshake.extend(frame_cl(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "clientInfo": { "name": "e2e-test", "version": "1.0" } }
    }))?);
    handshake.extend(frame_cl(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?);
    handshake.extend(frame_cl(&json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }))?);
    handshake.extend(frame_cl(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }))?);

    let mut hs_out = Vec::new();
    serve(&mut Cursor::new(handshake), &mut hs_out, state).map_err(|err| format!("handshake serve: {err}"))?;
    let hs_replies = decode_cl_frames(&hs_out)?;
    if hs_replies.len() != 3 {
        return Err(format!("expected 3 handshake replies, got {}", hs_replies.len()));
    }
    if hs_replies[0]["result"]["protocolVersion"] != json!("2024-11-05")
        || hs_replies[0]["result"]["serverInfo"]["name"] != json!("lumencraft")
        || !hs_replies[0]["result"]["capabilities"]["tools"].is_object()
    {
        return Err(format!("unexpected initialize reply: {}", hs_replies[0]));
    }
    if hs_replies[2]["result"]["tools"].as_array().map(Vec::len) != Some(10) || !state.is_initialized() {
        return Err(format!("unexpected tools/list or init state: {}", hs_replies[2]));
    }

    let cmd_list = first_text_json(&call_tool_rpc(state, 4, "command_list", json!({}))?)?;
    let entries = cmd_list["commands"].as_array().ok_or_else(|| String::from("missing commands array"))?;
    for spec in UI_COMMANDS {
        let hit = entries.iter().find(|item| item["id"] == spec.id).ok_or_else(|| format!("missing UI command {}", spec.id))?;
        if hit["ui"] != json!(true) {
            return Err(format!("expected ui=true on {}: {hit}", spec.id));
        }
    }
    Ok(())
}

fn slice0_populate_and_fit(state: &mut McpServerState) -> Result<(), String> {
    let create_res = first_text_json(&call_tool_rpc(
        state,
        5,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-1",
                "title": "Agonist Dose-Response",
                "archetype": "xy",
                "rows": 7,
                "cols": 1,
                "replicates": 2,
                "auto_family": true,
                "info_sheet_id": "info-1",
                "graph_sheet_id": "graph-1"
            }
        }),
    )?)?;
    if create_res["mutated"] != json!(true) {
        return Err(format!("expected mutated=true: {create_res}"));
    }

    let x_vals = [-9.0_f64, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0];
    call_tool_rpc(
        state,
        6,
        "command_run",
        json!({
            "command_id": "table.setXCells",
            "params": { "sheet_id": "xy-1", "values": x_vals }
        }),
    )?;

    let mut edits = Vec::new();
    for (row, &x) in x_vals.iter().enumerate() {
        let y_exact = 10.0 + 80.0 / (1.0 + 10.0_f64.powf(-6.0 - x));
        edits.push(json!({ "row": row, "col": 0, "subcol": 0, "value": y_exact - 0.4 }));
        edits.push(json!({ "row": row, "col": 0, "subcol": 1, "value": y_exact + 0.4 }));
    }
    call_tool_rpc(
        state,
        7,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": { "sheet_id": "xy-1", "edits": edits }
        }),
    )?;

    let fit_res = first_text_json(&call_tool_rpc(
        state,
        8,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "fit-1",
                "source_sheet_id": "xy-1",
                "equation_id": "log_agonist_vs_response_4pl"
            }
        }),
    )?)?;
    if fit_res["payload"]["sheet_id"] != json!("fit-1") {
        return Err(format!("unexpected fit result: {fit_res}"));
    }

    call_tool_rpc(
        state,
        9,
        "command_run",
        json!({
            "command_id": "graph.setRegressionOverlay",
            "params": { "sheet_id": "graph-1", "result_sheet_id": "fit-1", "all_series": true }
        }),
    )?;

    let inspect = first_text_json(&call_tool_rpc(state, 10, "doc_inspect", json!({}))?)?;
    let counts = (
        inspect["sheets"]["data"].as_array().map(Vec::len),
        inspect["sheets"]["info"].as_array().map(Vec::len),
        inspect["analyses"].as_array().map(Vec::len),
        inspect["graphs"].as_array().map(Vec::len),
    );
    if counts != (Some(1), Some(1), Some(1), Some(1)) {
        return Err(format!("unexpected sheet counts: {counts:?}"));
    }

    let result_scalars = inspect["variables"]["result_scalars"].as_array().ok_or_else(|| String::from("missing result scalars"))?;
    let log_ec50 = result_scalars
        .iter()
        .find(|s| s["key"] == "LogEC50")
        .and_then(|s| s["value"].as_f64())
        .ok_or_else(|| String::from("LogEC50 scalar missing"))?;
    if (log_ec50 - (-6.0)).abs() >= 0.05 {
        return Err(format!("unexpected LogEC50: {log_ec50}"));
    }
    Ok(())
}

fn slice0_render_and_exports(state: &mut McpServerState, dir: &std::path::Path) -> Result<(), String> {
    let svg_path = dir.join("dose_response.svg");
    let pdf_path = dir.join("dose_response.pdf");
    let tiff_path = dir.join("dose_response.tiff");
    let csv_path = dir.join("fit_tidy.csv");

    let preview = call_tool_rpc(state, 11, "render_preview", json!({ "sheet_id": "graph-1", "width_px": 640, "height_px": 480, "dpi": 96 }))?;
    if preview["isError"] != json!(false)
        || preview["content"][0]["type"] != json!("image")
        || preview["content"][0]["mimeType"] != json!("image/png")
    {
        return Err(format!("invalid render_preview envelope: {preview}"));
    }
    let png_b64 = preview["content"][0]["data"].as_str().ok_or_else(|| String::from("png base64 missing"))?;
    let png_raw = decode_base64(png_b64)?;
    if !png_raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("invalid PNG signature"));
    }

    let meta_text = preview["content"][1]["text"].as_str().ok_or_else(|| String::from("metadata text missing"))?;
    let preview_meta: Value = serde_json::from_str(meta_text).map_err(|err| format!("meta json: {err}"))?;
    if preview_meta["sheet_id"] != json!("graph-1") || preview_meta["width_px"] != json!(640) || preview_meta["height_px"] != json!(480) {
        return Err(format!("unexpected preview meta: {preview_meta}"));
    }
    if preview_meta["primitives_count"].as_u64().unwrap_or(0) == 0 {
        return Err(String::from("expected non-zero primitives count"));
    }

    let svg_inline = call_tool_rpc(state, 12, "export", json!({ "format": "svg", "sheet_id": "graph-1" }))?;
    let svg_text = svg_inline["content"][0]["text"].as_str().ok_or_else(|| String::from("svg text missing"))?;
    validate_svg_structure(svg_text).map_err(|err| format!("valid inline SVG: {err}"))?;

    call_tool_rpc(state, 13, "export", json!({ "format": "svg", "sheet_id": "graph-1", "output_path": svg_path.display().to_string() }))?;
    let saved_svg = fs::read_to_string(&svg_path).map_err(|err| format!("read svg: {err}"))?;
    validate_svg_structure(&saved_svg).map_err(|err| format!("valid disk SVG: {err}"))?;

    call_tool_rpc(state, 14, "export", json!({ "format": "pdf", "sheet_id": "graph-1", "output_path": pdf_path.display().to_string() }))?;
    let pdf_bytes = fs::read(&pdf_path).map_err(|err| format!("read pdf: {err}"))?;
    if !pdf_bytes.starts_with(b"%PDF-") {
        return Err(String::from("invalid PDF header"));
    }

    call_tool_rpc(
        state,
        15,
        "export",
        json!({ "format": "tiff", "sheet_id": "graph-1", "width_px": 320, "height_px": 240, "output_path": tiff_path.display().to_string() }),
    )?;
    let tiff_bytes = fs::read(&tiff_path).map_err(|err| format!("read tiff: {err}"))?;
    if !tiff_bytes.starts_with(b"II*\0") && !tiff_bytes.starts_with(b"MM\0*") {
        return Err(String::from("invalid TIFF header"));
    }

    call_tool_rpc(state, 16, "export", json!({ "format": "csv", "sheet_id": "fit-1", "tidy": true, "output_path": csv_path.display().to_string() }))?;
    let csv_text = fs::read_to_string(&csv_path).map_err(|err| format!("read csv: {err}"))?;
    if !csv_text.contains("LogEC50") {
        return Err(String::from("expected LogEC50 in CSV export"));
    }

    let json_inline = call_tool_rpc(state, 17, "export", json!({ "format": "json", "sheet_id": "fit-1" }))?;
    let fit_text = json_inline["content"][0]["text"].as_str().ok_or_else(|| String::from("fit json missing"))?;
    let exported_fit: Value = serde_json::from_str(fit_text).map_err(|err| format!("parse fit json: {err}"))?;
    if exported_fit["id"] != json!("fit-1") {
        return Err(format!("unexpected fit export id: {exported_fit}"));
    }
    Ok(())
}

fn slice0_save_and_reopen(state: &mut McpServerState, dir: &std::path::Path) -> Result<(), String> {
    let project_path = dir.join("dose_response.lumencraft");
    let template_path = dir.join("dose_response.lctemplate");

    let save_res = first_text_json(&call_tool_rpc(state, 18, "save_file", json!({ "path": project_path.display().to_string() }))?)?;
    if save_res["saved"] != json!(true) {
        return Err(format!("expected saved=true: {save_res}"));
    }

    first_text_json(&call_tool_rpc(state, 19, "save_file", json!({ "path": template_path.display().to_string() }))?)?;

    let mut reopened_state = McpServerState::new();
    let open_res = first_text_json(&call_tool_rpc(&mut reopened_state, 20, "open_file", json!({ "path": project_path.display().to_string() }))?)?;
    if open_res["opened"] != json!(true) || open_res["total_sheet_count"] != json!(4) {
        return Err(format!("unexpected reopen result: {open_res}"));
    }

    let reopened_inspect = first_text_json(&call_tool_rpc(&mut reopened_state, 21, "doc_inspect", json!({}))?)?;
    if reopened_inspect["analyses"].as_array().map(Vec::len) != Some(1) || reopened_inspect["graphs"].as_array().map(Vec::len) != Some(1) {
        return Err(format!("unexpected reopened inspect: {reopened_inspect}"));
    }

    let single_xy = first_text_json(&call_tool_rpc(&mut reopened_state, 22, "doc_inspect", json!({ "sheet_id": "xy-1", "include_cells": true }))?)?;
    if !single_xy["csv_preview"].as_str().is_some_and(|s| s.contains("-9")) {
        return Err(format!("missing -9 in xy csv preview: {single_xy}"));
    }

    let mut tpl_state = McpServerState::new();
    first_text_json(&call_tool_rpc(
        &mut tpl_state,
        23,
        "open_file",
        json!({ "path": template_path.display().to_string(), "title": "From Template" }),
    )?)?;
    let tpl_inspect = first_text_json(&call_tool_rpc(&mut tpl_state, 24, "doc_inspect", json!({}))?)?;
    if tpl_inspect["title"] != json!("From Template") {
        return Err(format!("unexpected template title: {tpl_inspect}"));
    }
    Ok(())
}

#[test]
fn slice0_end_to_end_via_mcp_jsonrpc() {
    let dir = TempDirGuard::new("mcp-slice0");
    let mut state = McpServerState::new();
    slice0_handshake_and_catalog(&mut state).expect("slice0 handshake");
    slice0_populate_and_fit(&mut state).expect("slice0 populate and fit");
    slice0_render_and_exports(&mut state, &dir.path).expect("slice0 render and export");
    slice0_save_and_reopen(&mut state, &dir.path).expect("slice0 save and reopen");
}

#[test]
fn command_batch_atomic_rolls_back_all_mutations_on_failure() {
    let mut state = McpServerState::new();
    call_tool_rpc(
        &mut state,
        1,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": { "sheet_id": "base-xy", "archetype": "xy", "rows": 3, "cols": 1 }
        }),
    )
    .expect("create base-xy");

    let batch_fail = call_tool_rpc(
        &mut state,
        2,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                {
                    "command_id": "sheet.createData",
                    "params": { "sheet_id": "ephemeral-1", "archetype": "column", "rows": 3, "cols": 2 }
                },
                {
                    "command_id": "table.setCell",
                    "params": { "sheet_id": "ephemeral-1", "row": 0, "col": 0, "value": 42.0 }
                },
                {
                    "command_id": "nonexistent.command.boom",
                    "params": {}
                }
            ]
        }),
    )
    .expect("batch rpc envelope");
    assert_eq!(batch_fail["isError"], json!(true));
    let err_payload: Value = serde_json::from_str(batch_fail["content"][0]["text"].as_str().expect("error payload")).expect("json");
    assert_eq!(err_payload["rolled_back"], json!(true));
    assert_eq!(err_payload["failed_index"], json!(2));

    let inspect = first_text_json(&call_tool_rpc(&mut state, 3, "doc_inspect", json!({})).expect("inspect")).expect("inspect json");
    let data_sheets = inspect["sheets"]["data"].as_array().expect("data sheets");
    assert_eq!(data_sheets.len(), 1);
    assert_eq!(data_sheets[0]["id"], json!("base-xy"));
}

#[test]
fn open_file_supports_csv_tsv_pzfx_and_prism_containers() {
    let dir = TempDirGuard::new("mcp-import");
    let csv_file = dir.path.join("kinetics.csv");
    let tsv_file = dir.path.join("groups.tsv");
    let pzfx_file = dir.path.join("assay.pzfx");
    let prism_file = dir.path.join("bundle.prism");

    fs::write(&csv_file, "X,Control,Treated\n0,10,10\n5,25,60\n10,40,95\n").expect("write csv");
    fs::write(&tsv_file, "Ctrl\tDrug\n1.2\t3.4\n1.5\t3.9\n").expect("write tsv");

    let pzfx_xml = r#"<PrismFile>
  <Title>Cleanroom Enzyme Assay</Title>
  <InfoSequence>
    <Info ID="info1">
      <Title>Assay Notes</Title>
      <Constant><Name>SubstrateKm</Name><Value>12.5</Value></Constant>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="tbl1" TableType="XY">
      <Title>Michaelis Data</Title>
      <XColumn><Title>Substrate</Title><Subcolumn><d>1</d><d>5</d><d>10</d></Subcolumn></XColumn>
      <YColumn><Title>Velocity</Title><Subcolumn><d>4.2</d><d>14.1</d><d>21.3</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    fs::write(&pzfx_file, pzfx_xml).expect("write pzfx");

    let mut zw = ZipWriter::new();
    let manifest = br#"{"title":"Cleanroom Prism Archive","sheets":[{"id":"dose","title":"Dose","kind":"xy","csv_path":"tables/dose.csv"}]}"#;
    let table_csv = b"Dose,Response\n-8,5\n-6,50\n-4,95\n";
    zw.add_file("manifest.json", manifest.to_vec()).expect("zip manifest");
    zw.add_file("tables/dose.csv", table_csv.to_vec()).expect("zip table");
    let prism_bytes = zw.finish().expect("finish zip");
    fs::write(&prism_file, prism_bytes).expect("write prism zip");

    let mut state = McpServerState::new();

    let csv_res = first_text_json(
        &call_tool_rpc(&mut state, 1, "open_file", json!({ "path": csv_file.display().to_string(), "target_kind": "xy" })).expect("open csv"),
    )
    .expect("csv json");
    assert_eq!(csv_res["format"], json!("csv"));
    assert_eq!(csv_res["rows"], json!(3));

    let tsv_res = first_text_json(
        &call_tool_rpc(
            &mut state,
            2,
            "open_file",
            json!({ "path": tsv_file.display().to_string(), "target_kind": "column", "append": true, "sheet_id": "grp-tsv" }),
        )
        .expect("open tsv"),
    )
    .expect("tsv json");
    assert_eq!(tsv_res["format"], json!("tsv"));
    let doc_state = first_text_json(&call_tool_rpc(&mut state, 3, "doc_inspect", json!({})).expect("inspect")).expect("inspect json");
    assert_eq!(doc_state["sheets"]["data"].as_array().expect("data").len(), 2);

    let pzfx_res =
        first_text_json(&call_tool_rpc(&mut state, 4, "open_file", json!({ "path": pzfx_file.display().to_string() })).expect("open pzfx"))
            .expect("pzfx json");
    assert_eq!(pzfx_res["format"], json!("pzfx"));
    assert_eq!(pzfx_res["title"], json!("Cleanroom Enzyme Assay"));

    let prism_res =
        first_text_json(&call_tool_rpc(&mut state, 5, "open_file", json!({ "path": prism_file.display().to_string() })).expect("open prism"))
            .expect("prism json");
    assert_eq!(prism_res["format"], json!("prism"));
    assert_eq!(prism_res["title"], json!("Cleanroom Prism Archive"));
}

fn run_ndjson_batch(state: &mut McpServerState, requests: &[Value]) -> Result<Vec<Value>, String> {
    let mut wire = String::new();
    for req in requests {
        let encoded = serde_json::to_string(req).map_err(|err| format!("serialize request: {err}"))?;
        wire.push_str(&encoded);
        wire.push('\n');
    }
    let mut output = Vec::new();
    serve(&mut Cursor::new(wire.as_bytes()), &mut output, state).map_err(|err| format!("serve ndjson: {err}"))?;
    let text = String::from_utf8(output).map_err(|err| format!("valid utf8: {err}"))?;
    text.lines().map(|line| serde_json::from_str(line).map_err(|err| format!("json response: {err}"))).collect()
}

#[test]
fn newline_transport_supports_layout_rendering_and_multipage_pdf() {
    let mut state = McpServerState::new();
    let batch_replies = run_ndjson_batch(
        &mut state,
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05" }
            }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "command_batch",
                    "arguments": {
                        "commands": [
                            {
                                "command_id": "sheet.createData",
                                "params": { "sheet_id": "d1", "archetype": "column", "rows": 4, "cols": 2, "auto_graph": true, "graph_sheet_id": "g1" }
                            },
                            {
                                "command_id": "table.setCells",
                                "params": { "sheet_id": "d1", "rows": [[10.0, 20.0], [12.0, 24.0], [11.0, 22.0]] }
                            },
                            {
                                "command_id": "layout.create",
                                "params": { "sheet_id": "lay-1", "title": "Figure 1", "preset": "double_column" }
                            },
                            {
                                "command_id": "layout.addPanel",
                                "params": { "sheet_id": "lay-1", "graph_sheet_id": "g1" }
                            }
                        ]
                    }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "render_preview", "arguments": { "sheet_id": "lay-1" } }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "export", "arguments": { "format": "pdf", "all_graphs": true } }
            }),
        ],
    )
    .expect("run ndjson batch");
    assert_eq!(batch_replies.len(), 4);
    assert_eq!(batch_replies[0]["id"], json!(1));
    assert_eq!(batch_replies[1]["result"]["isError"], json!(false));

    let preview = &batch_replies[2]["result"];
    assert_eq!(preview["isError"], json!(false));
    let png_bytes = decode_base64(preview["content"][0]["data"].as_str().expect("layout png")).expect("decode layout png");
    assert!(png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let multi_pdf = first_text_json(&batch_replies[3]["result"]).expect("multi pdf json");
    assert_eq!(multi_pdf["format"], json!("pdf"));
    let pdf_bytes = decode_base64(multi_pdf["data"].as_str().expect("pdf b64")).expect("decode pdf b64");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_control_port_attach_dispatches_ui_and_engine_and_atomic_undo() {
    use std::io::ErrorKind;
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    assert!(McpServerState::with_control_port(0).is_err());

    // Probe whether loopback TCP connect is permitted in the active execution environment.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            let mut state = McpServerState::with_control_port(59999).expect("valid port");
            let res = call_tool_rpc(&mut state, 1, "doc_inspect", json!({})).expect("rpc reply");
            assert_eq!(res["isError"], json!(true));
            return;
        }
        Err(err) => panic!("unexpected loopback bind error: {err}"),
    };
    let port = listener.local_addr().expect("local addr").port();

    let probe_addr = listener.local_addr().expect("probe addr");
    let probe_connect = TcpStream::connect_timeout(&probe_addr, Duration::from_millis(200));
    match probe_connect {
        Ok(stream) => {
            drop(stream);
            let _ = listener.accept();
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            let mut state = McpServerState::with_control_port(port).expect("valid port");
            let res = call_tool_rpc(&mut state, 1, "doc_inspect", json!({})).expect("rpc reply");
            assert_eq!(res["isError"], json!(true));
            assert!(res["content"][0]["text"].as_str().is_some_and(|t| t.contains("failed to connect")));
            return;
        }
        Err(err) => panic!("unexpected loopback probe error: {err}"),
    }

    let (methods_tx, methods_rx) = mpsc::channel::<String>();

    let handle = thread::spawn(move || {
        for stream_res in listener.incoming().take(5) {
            let mut stream = stream_res.expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("read request line");
            let req: Value = serde_json::from_str(&line).expect("json req");
            let id = req["id"].as_str().unwrap_or("mcp-1").to_owned();
            let method = req["method"].as_str().unwrap_or("").to_owned();
            methods_tx.send(method.clone()).expect("record method");

            let resp = match method.as_str() {
                "state.snapshot" => json!({ "id": id, "ok": true, "result": { "title": "Live Desktop Window", "active_sheet_id": "xy-live" } }),
                "ui.navigator.toggle" => json!({ "id": id, "ok": true, "result": { "navigator_open": false } }),
                "engine.execute" => {
                    let cmd_id = req["params"]["command_id"].as_str().unwrap_or("");
                    if cmd_id == "fail.step" {
                        json!({ "id": id, "ok": false, "error": "simulated engine failure" })
                    } else {
                        json!({ "id": id, "ok": true, "result": { "command_id": cmd_id, "mutated": cmd_id != "edit.undo" } })
                    }
                }
                _ => json!({ "id": id, "ok": false, "error": "unknown method" }),
            };

            let wire = format!("{resp}\n");
            stream.write_all(wire.as_bytes()).expect("write resp");
            stream.flush().expect("flush");
        }
    });

    let mut state = McpServerState::with_control_port(port).expect("with_control_port");

    // 1. doc_inspect -> state.snapshot
    let inspect = first_text_json(&call_tool_rpc(&mut state, 1, "doc_inspect", json!({})).expect("inspect rpc")).expect("inspect json");
    assert_eq!(inspect["mode"], json!("live_attach"));
    assert_eq!(inspect["snapshot"]["title"], json!("Live Desktop Window"));

    // 2. command_run ui.navigator.toggle
    let ui_res =
        first_text_json(&call_tool_rpc(&mut state, 2, "command_run", json!({ "command_id": "ui.navigator.toggle", "params": {} })).expect("ui rpc"))
            .expect("ui json");
    assert_eq!(ui_res["live_attach"], json!(true));
    assert_eq!(ui_res["result"]["navigator_open"], json!(false));

    // 3. atomic command_batch triggering failure -> triggers edit.undo rollback (requests 3, 4, 5)
    let batch = call_tool_rpc(
        &mut state,
        3,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "live-1" } },
                { "command_id": "fail.step", "params": {} }
            ]
        }),
    )
    .expect("batch rpc");
    assert_eq!(batch["isError"], json!(true));
    let batch_json: Value = serde_json::from_str(batch["content"][0]["text"].as_str().expect("text")).expect("json");
    assert_eq!(batch_json["rolled_back"], json!(true));

    handle.join().expect("server thread finished");
    let methods: Vec<String> = methods_rx.iter().collect();
    assert_eq!(methods, vec!["state.snapshot", "ui.navigator.toggle", "engine.execute", "engine.execute", "engine.execute"]);
}
