#![allow(clippy::unwrap_used, clippy::expect_used)]

//! End-to-end MCP + `D1` nonlinear regression oracle test for GitHub issue #59:
//! `open_file` on a decimal-comma `.pzfx` file, `resources/read` (`sheet://Table0`),
//! and `command_run` (`fit.run` with `log_agonist_vs_response_3pl`).
//!
//! Reference (`D1`):
//! - Motulsky, H. J., & Christopoulos, A. (2004). *Fitting Models to Biological Data Using
//!   Linear and Nonlinear Regression: A Practical Guide to Curve Fitting*. Oxford University
//!   Press, Ch. 43 (three-parameter Hill / log-agonist vs. response model:
//!   `Y = Bottom + (Top - Bottom) / (1 + 10^(LogEC50 - X))`).
//! - Exact ordinary least-squares stationary point (`||grad SSR||_inf < 2e-16` via full
//!   Newton–Raphson / Gauss–Newton on the 7-point dataset `X = [-9, -8.5, -8, -7.5, -7, -6.5, -6]`,
//!   `Y = [0.10, 0.15, 0.30, 0.55, 0.80, 0.92, 0.97]`):
//!   `Bottom = 0.0622802171852402`, `Top = 1.0001991840935882`,
//!   `LogEC50 = -7.5393275696570097`, `EC50 = 2.8885003888554543e-8`,
//!   `SSR = 1.995300310913576e-4`. Numerical tolerance: `1e-6` absolute on `Bottom`, `Top`,
//!   and `LogEC50` (`1e-12` relative parity against the identical dot-decimal `.pzfx` import).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{ResultStatus, SheetId};
use curvebook_mcp::{McpServerState, dispatch_json_value};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempWorkdir {
    path: PathBuf,
}

impl TempWorkdir {
    fn new(label: &str) -> Self {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb_issue_59_{label}_{}_{seq}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp workdir");
        Self { path }
    }
}

impl Drop for TempWorkdir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn rpc(state: &mut McpServerState, req: Value) -> Value {
    dispatch_json_value(req, state).expect("JSON-RPC response")
}

fn tool_json(state: &mut McpServerState, id: u64, name: &str, arguments: Value) -> Value {
    let resp = rpc(
        state,
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments,
            }
        }),
    );
    assert_ne!(resp.pointer("/result/isError"), Some(&json!(true)), "unexpected MCP tool error for `{name}`: {resp}");
    if let Some(sc) = resp.pointer("/result/structuredContent")
        && !sc.is_null()
    {
        return sc.clone();
    }
    let text = resp.pointer("/result/content/0/text").and_then(Value::as_str).expect("MCP text block");
    serde_json::from_str(text).expect("MCP JSON payload")
}

fn run_mcp_3pl_workflow(pzfx_xml: &str, label: &str) -> (String, Vec<(String, f64)>) {
    let dir = TempWorkdir::new(label);
    fs::write(dir.path.join("comma.pzfx"), pzfx_xml).expect("write comma.pzfx");

    let mut state = McpServerState::with_workdir(&dir.path).expect("MCP state with workdir");
    let _init = rpc(
        &mut state,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2024-11-05" }
        }),
    );

    let opened = tool_json(&mut state, 2, "open_file", json!({ "path": "comma.pzfx" }));
    assert_eq!(opened["opened"], json!(true));
    assert_eq!(opened["warnings"], json!([]), "expected zero warnings on clean decimal-comma import: {opened}");

    let sheet_resp = rpc(
        &mut state,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "resources/read",
            "params": { "uri": "sheet://Table0" }
        }),
    );
    let sheet_text = sheet_resp.pointer("/result/contents/0/text").and_then(Value::as_str).expect("sheet://Table0 resource JSON");
    let sheet_json: Value = serde_json::from_str(sheet_text).expect("parse sheet://Table0 JSON");
    let csv_preview = sheet_json["csv_preview"].as_str().expect("csv_preview string").to_owned();

    let _fit = tool_json(
        &mut state,
        4,
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "fit",
                "source_sheet_id": "Table0",
                "equation_id": "log_agonist_vs_response_3pl"
            }
        }),
    );

    let rs = state.session().document().result_sheet(&SheetId::new("fit")).expect("fit result sheet");
    assert_eq!(rs.status, ResultStatus::UpToDate, "fit must converge cleanly");
    let scalars: Vec<(String, f64)> = rs.scalars.iter().map(|s| (s.key.clone(), s.value)).collect();
    (csv_preview, scalars)
}

fn scalar_val(scalars: &[(String, f64)], key: &str) -> f64 {
    scalars.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| *v).expect("expected fit scalar key")
}

#[test]
fn issue_59_mcp_open_pzfx_decimal_comma_and_3pl_fit_oracle() {
    let comma_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
<TableSequence><Ref ID="Table0" Selected="1"></Ref></TableSequence>
<Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY">
<Title>comma decimals</Title>
<XColumn Width="60" Subcolumns="1" Decimals="1"><Title></Title><Subcolumn>
<d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d>
</Subcolumn></XColumn>
<YColumn Width="60" Decimals="3" Subcolumns="1"><Title>Response</Title><Subcolumn>
<d>0,10</d><d>0,15</d><d>0,30</d><d>0,55</d><d>0,80</d><d>0,92</d><d>0,97</d>
</Subcolumn></YColumn>
</Table>
</PrismFile>"#;

    let partial_comma_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
<TableSequence><Ref ID="Table0" Selected="1"></Ref></TableSequence>
<Table ID="Table0" XFormat="numbers" YFormat="replicates" Replicates="1" TableType="XY">
<Title>comma decimals</Title>
<XColumn Width="60" Subcolumns="1" Decimals="1"><Title></Title><Subcolumn>
<d>-9</d><d>-8,5</d><d>-8</d><d>-7,5</d><d>-7</d><d>-6,5</d><d>-6</d>
</Subcolumn></XColumn>
<YColumn Width="60" Decimals="3" Subcolumns="1"><Title>Response</Title><Subcolumn>
<d>0.10</d><d>0.15</d><d>0.30</d><d>0.55</d><d>0.80</d><d>0.92</d><d>0.97</d>
</Subcolumn></YColumn>
</Table>
</PrismFile>"#;

    let dot_xml = comma_xml.replace(',', ".");

    let (comma_csv, comma_scalars) = run_mcp_3pl_workflow(comma_xml, "full_comma");
    let (partial_csv, partial_scalars) = run_mcp_3pl_workflow(partial_comma_xml, "partial_comma");
    let (dot_csv, dot_scalars) = run_mcp_3pl_workflow(&dot_xml, "dot_ref");

    // 1. `sheet://Table0` CSV preview stores numeric values rather than quoted strings like `"'-8,5","0,15"`.
    assert_eq!(comma_csv, "X,Response\n-9,0.1\n-8.5,0.15\n-8,0.3\n-7.5,0.55\n-7,0.8\n-6.5,0.92\n-6,0.97");
    assert_eq!(partial_csv, dot_csv);
    assert_eq!(comma_csv, dot_csv);

    // 2. D1 Oracle (Motulsky & Christopoulos 2004, 3PL log-agonist vs. response OLS stationary point, tol = 1e-6).
    let ref_bottom = 0.062_280_217_185_240_2_f64;
    let ref_top = 1.000_199_184_093_588_2_f64;
    let ref_log_ec50 = -7.539_327_569_657_01_f64;
    let ref_ec50 = 2.888_500_388_855_454_3e-8_f64;
    let tol = 1e-6_f64;

    for (mode, scalars) in [("full_comma", &comma_scalars), ("partial_comma", &partial_scalars)] {
        let bottom = scalar_val(scalars, "Bottom");
        let top = scalar_val(scalars, "Top");
        let log_ec50 = scalar_val(scalars, "LogEC50");
        let ec50 = scalar_val(scalars, "EC50");

        assert!((bottom - ref_bottom).abs() <= tol, "{mode}: Bottom={bottom} vs oracle {ref_bottom}");
        assert!((top - ref_top).abs() <= tol, "{mode}: Top={top} vs oracle {ref_top}");
        assert!((log_ec50 - ref_log_ec50).abs() <= tol, "{mode}: LogEC50={log_ec50} vs oracle {ref_log_ec50}");
        assert!((ec50 - ref_ec50).abs() <= 1e-13, "{mode}: EC50={ec50} vs oracle {ref_ec50}");

        // Exact parity against dot-decimal `.pzfx` import (`tol = 1e-12`).
        assert!((bottom - scalar_val(&dot_scalars, "Bottom")).abs() <= 1e-12);
        assert!((top - scalar_val(&dot_scalars, "Top")).abs() <= 1e-12);
        assert!((log_ec50 - scalar_val(&dot_scalars, "LogEC50")).abs() <= 1e-12);
    }
}
