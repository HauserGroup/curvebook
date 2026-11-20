#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use curvebook_mcp::{McpServerState, serve};
use serde_json::{Value, json};

fn frame_cl(val: &Value) -> Vec<u8> {
    let body = serde_json::to_string(val).unwrap();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn decode_cl_frames(raw: &[u8]) -> Vec<Value> {
    let mut cursor = raw;
    let mut out = Vec::new();
    while !cursor.is_empty() {
        let text = std::str::from_utf8(cursor).unwrap();
        let (hdr, rest) = text.split_once("\r\n\r\n").unwrap();
        let len: usize = hdr.strip_prefix("Content-Length: ").unwrap().trim().parse().unwrap();
        let body = &rest.as_bytes()[..len];
        out.push(serde_json::from_slice(body).unwrap());
        cursor = &rest.as_bytes()[len..];
    }
    out
}

fn run_rpc(state: &mut McpServerState, req: &Value) -> Value {
    let input = frame_cl(req);
    let mut output = Vec::new();
    serve(&mut Cursor::new(input), &mut output, state).unwrap();
    let replies = decode_cl_frames(&output);
    replies.into_iter().find(|r| r.get("id") == req.get("id")).unwrap()
}

fn call_tool(state: &mut McpServerState, id: i64, name: &str, arguments: Value) -> Value {
    let resp = run_rpc(
        state,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    );
    assert!(resp.get("error").is_none(), "unexpected JSON-RPC error: {resp}");
    assert_eq!(resp["result"]["isError"], json!(false), "tool `{name}` failed: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[test]
fn r45_mcp_robust_fit_renders_not_applicable_status_and_rsdr() {
    let mut state = McpServerState::new();
    call_tool(&mut state, 1, "enable_toolset", json!({ "groups": ["table", "analysis"] }));

    call_tool(
        &mut state,
        2,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "xy-1",
                "title": "Decay",
                "kind": "xy",
                "rows": 7,
                "cols": 1,
                "replicates": 1,
                "auto_graph": false
            }
        }),
    );
    call_tool(
        &mut state,
        3,
        "command_run",
        json!({
            "command_id": "table.setXCells",
            "params": {
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "value": 0.0 },
                    { "row": 1, "value": 1.0 },
                    { "row": 2, "value": 2.0 },
                    { "row": 3, "value": 4.0 },
                    { "row": 4, "value": 8.0 },
                    { "row": 5, "value": 12.0 }
                ]
            }
        }),
    );
    call_tool(
        &mut state,
        4,
        "command_run",
        json!({
            "command_id": "table.setCells",
            "params": {
                "sheet_id": "xy-1",
                "edits": [
                    { "row": 0, "col": 0, "subcol": 0, "value": 100.0 },
                    { "row": 1, "col": 0, "subcol": 0, "value": 74.1 },
                    { "row": 2, "col": 0, "subcol": 0, "value": 54.9 },
                    { "row": 3, "col": 0, "subcol": 0, "value": 30.1 },
                    { "row": 4, "col": 0, "subcol": 0, "value": 9.1 },
                    { "row": 5, "col": 0, "subcol": 0, "value": 2.7 },
                    { "row": 6, "col": 0, "subcol": 0, "value": 50.0 }
                ]
            }
        }),
    );

    let fit_cmd = call_tool(
        &mut state,
        5,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "fit-robust",
                "source_sheet_id": "xy-1",
                "kind": "one_phase_decay",
                "method": "robust",
                "confidence_bands": true,
                "prediction_bands": true,
                "emit_covariance_tab": true,
                "interpolate_unknowns": true,
                "comparison": { "method": "f_test", "alternative_equation_id": "straight_line" }
            }
        }),
    );
    assert_eq!(fit_cmd["payload"]["inference_availability"], json!("NotApplicableInRobustMode"));
    assert!(fit_cmd["payload"]["scalars"]["RSDR"]["value"].as_f64().is_some_and(f64::is_finite));

    let inspected = call_tool(&mut state, 6, "doc_inspect", json!({ "sheet_id": "fit-robust", "include_cells": true }));
    assert_eq!(inspected["inference_availability"], json!("NotApplicableInRobustMode"));
    let tabs_str = serde_json::to_string(&inspected["tabs"]).unwrap();
    assert!(tabs_str.contains("Not applicable"));
    assert!(tabs_str.contains("NotApplicableInRobustMode"));
    assert!(!inspected["tabs"].as_array().unwrap().iter().any(|t| t["id"] == "nonlinear_covariance"));

    let sheet_read =
        run_rpc(&mut state, &json!({ "jsonrpc": "2.0", "id": 7, "method": "resources/read", "params": { "uri": "sheet://fit-robust" } }));
    let sheet_text = sheet_read["result"]["contents"][0]["text"].as_str().unwrap();
    let sheet_val: Value = serde_json::from_str(sheet_text).unwrap();
    assert_eq!(sheet_val["inference_availability"], json!("NotApplicableInRobustMode"));
    assert!(sheet_val["scalars"].as_array().unwrap().iter().any(|s| s["key"] == "RSDR"));

    let res_read = run_rpc(&mut state, &json!({ "jsonrpc": "2.0", "id": 8, "method": "resources/read", "params": { "uri": "result://fit-robust" } }));
    let res_text = res_read["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(res_text.contains("\"Not applicable\""));
    assert!(res_text.contains("NotApplicableInRobustMode"));
}
