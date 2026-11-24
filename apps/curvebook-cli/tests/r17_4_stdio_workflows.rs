//! End-to-end stdio JSON-RPC workflows exercising the real `curvebook-cli mcp` binary across:
//! 1) messy delimited import & cleanup,
//! 2) statistical group comparison + fuzzy error recovery,
//! 3) nonlinear dose-response fit with ROUT outlier detection & identifiability diagnostics,
//! 4) publication-quality figure preflight, render_preview, and SVG/PDF/PNG export.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempWorkdir {
    path: PathBuf,
}

impl TempWorkdir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = env::temp_dir().join(format!("cb-cli-r17-4-{label}-{}-{seq}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|err| format!("create workdir {}: {err}", dir.display()))?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempWorkdir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct StdioMcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl StdioMcp {
    fn spawn(workdir: &Path) -> Result<Self, String> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_curvebook-cli"))
            .arg("mcp")
            .arg("--workdir")
            .arg(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("spawn curvebook-cli mcp: {err}"))?;
        let stdin = child.stdin.take().ok_or("missing child stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("missing child stdout")?);
        let mut client = Self { child, stdin, stdout, next_id: 1 };
        let _ = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "r17-4-e2e-test", "version": "1.0" }
            }),
        )?;
        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|err| format!("encode rpc: {err}"))?;
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).map_err(|err| format!("write header: {err}"))?;
        self.stdin.write_all(&body).map_err(|err| format!("write body: {err}"))?;
        self.stdin.flush().map_err(|err| format!("flush: {err}"))
    }

    fn read_frame(&mut self) -> Result<(usize, Value), String> {
        let mut content_len: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).map_err(|err| format!("read header: {err}"))?;
            if n == 0 {
                return Err(String::from("unexpected EOF from curvebook-cli mcp stdout"));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some((k, v)) = trimmed.split_once(':')
                && k.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_len = v.trim().parse::<usize>().ok();
            }
        }
        let len = content_len.ok_or("missing Content-Length header")?;
        let mut buf = vec![0_u8; len];
        self.stdout.read_exact(&mut buf).map_err(|err| format!("read body: {err}"))?;
        let val: Value = serde_json::from_slice(&buf).map_err(|err| format!("parse rpc json: {err}"))?;
        Ok((buf.len(), val))
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<(usize, Value), String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let (bytes, msg) = self.read_frame()?;
            if msg.get("id") == Some(&json!(id)) {
                return Ok((bytes, msg));
            }
        }
    }

    fn call_ok(&mut self, name: &str, args: Value) -> Result<(usize, Value), String> {
        let (bytes, resp) = self.request("tools/call", json!({ "name": name, "arguments": args }))?;
        let res = &resp["result"];
        if res["isError"] == json!(true) || resp.get("error").is_some() {
            return Err(format!("tool `{name}` failed: {resp}"));
        }
        let sc = res.get("structuredContent").cloned().ok_or_else(|| format!("missing structuredContent on `{name}`"))?;
        Ok((bytes, sc))
    }

    fn call_err(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let (_, resp) = self.request("tools/call", json!({ "name": name, "arguments": args }))?;
        if resp["result"]["isError"] != json!(true) {
            return Err(format!("expected isError=true for `{name}`, got {resp}"));
        }
        Ok(resp["result"]["structuredContent"].clone())
    }
}

impl Drop for StdioMcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_workflows_1_through_4_end_to_end_compact_and_accurate() -> Result<(), String> {
    let dir = TempWorkdir::new("wf14")?;
    let messy_csv = dir.path.join("elisa.csv");
    fs::write(
        &messy_csv,
        "Dose_nM;Ctrl_1;Ctrl_2;Drug_1;Drug_2;\n0.1;101.2;99.1;100.4;98.8;\n1.0;98.5;ND;83.2;81.9;\n10.0;100.1;99.4;49.8;9999.0;\n100.0;99.6;;19.1;18.4;\n",
    )
    .map_err(|e| e.to_string())?;

    let mut mcp = StdioMcp::spawn(&dir.path)?;

    // Discovery budget sanity checks
    let (ts_bytes, ts_json) = mcp.call_ok("list_toolsets", json!({ "include_tools": false }))?;
    if ts_bytes > 4096 || ts_json["total_engine_commands"].as_u64().unwrap_or(0) < 100 {
        return Err(format!("unexpected list_toolsets(include_tools=false) bytes={ts_bytes}: {ts_json}"));
    }
    let (cl_bytes, _) = mcp.call_ok("command_list", json!({ "filter": "fit", "include_ui": false }))?;
    if cl_bytes > 4096 {
        return Err(format!("filtered compact command_list exceeded budget: {cl_bytes}B"));
    }
    let (_, fit_schema) = mcp.call_ok("command_list", json!({ "command_id": "fit.run" }))?;
    if fit_schema.pointer("/commands/0/input_schema/properties/method").is_none() {
        return Err(format!("fit.run schema missing method parameter: {fit_schema}"));
    }

    // Workflow 1: messy CSV import, warning surface, exclusion cleanup, rename
    let (_, opened) = mcp.call_ok(
        "open_file",
        json!({
            "path": messy_csv.display().to_string(),
            "target_kind": "xy",
            "subcolumn_mode": "replicates:2",
            "delimiter": ";",
            "sheet_id": "elisa-1"
        }),
    )?;
    if opened["warnings"].as_array().is_none_or(Vec::is_empty) {
        return Err(format!("expected ND text placeholder warning from open_file: {opened}"));
    }
    let missing_reason = mcp.call_err(
        "command_run",
        json!({ "command_id": "table.excludeCellsWithReason", "params": { "sheet_id": "elisa-1", "row": 2, "col": 1, "subcol": 1 } }),
    )?;
    if !missing_reason["remediation"].as_str().unwrap_or("").contains("command_list") {
        return Err(format!("missing reason_tag remediation hint: {missing_reason}"));
    }
    mcp.call_ok(
        "command_run",
        json!({
            "command_id": "table.excludeCellsWithReason",
            "params": { "sheet_id": "elisa-1", "coords": [{ "row": 2, "col": 1, "subcol": 1 }], "reason_tag": "pipetting_spike" }
        }),
    )?;
    mcp.call_ok("command_run", json!({ "command_id": "sheet.rename", "params": { "sheet_id": "elisa-1", "title": "Clean ELISA" } }))?;

    // Workflow 2: 3-group statistical comparison + fuzzy error recovery
    let bad_kind = mcp.call_err(
        "command_run",
        json!({ "command_id": "stats.run", "params": { "sheet_id": "st-1", "source_sheet_id": "elisa-1", "kind": "anova_one_way" } }),
    )?;
    if !bad_kind["remediation"].as_str().unwrap_or("").contains("one_way_anova") {
        return Err(format!("bad kind missing one_way_anova remediation: {bad_kind}"));
    }
    mcp.call_ok(
        "command_run",
        json!({ "command_id": "sheet.createData", "params": { "sheet_id": "col-1", "title": "Cohort", "kind": "column", "rows": 6, "cols": 3 } }),
    )?;
    let groups = [[10.2, 10.8, 9.9, 11.1, 10.4, 10.6], [8.4, 8.9, 8.1, 8.7, 9.0, 8.5], [4.2, 4.8, 4.5, 4.9, 4.3, 4.6]];
    let col_edits: Vec<Value> = groups
        .iter()
        .enumerate()
        .flat_map(|(c, vals)| vals.iter().enumerate().map(move |(r, v)| json!({ "row": r, "col": c, "subcol": 0, "value": v })))
        .collect();
    mcp.call_ok("command_run", json!({ "command_id": "table.setCells", "params": { "sheet_id": "col-1", "edits": col_edits } }))?;
    mcp.call_ok(
        "command_run",
        json!({ "command_id": "stats.run", "params": { "sheet_id": "anova-1", "source_sheet_id": "col-1", "kind": "one_way_anova" } }),
    )?;
    let (_, anova_tab) = mcp.call_ok("doc_inspect", json!({ "sheet_id": "anova-1", "include_cells": true, "tab_id": "anova-table" }))?;
    if anova_tab["tabs"].as_array().map(Vec::len) != Some(1) {
        return Err(format!("expected filtered anova_table tab: {anova_tab}"));
    }

    // Workflow 3: nonlinear 4PL fit + ROUT outlier + identifiability + compact result resource
    mcp.call_ok(
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": {
                "sheet_id": "dr-xy",
                "title": "IC50 Curve",
                "kind": "xy",
                "rows": 8,
                "cols": 1,
                "replicates": 3,
                "auto_family": true,
                "graph_sheet_id": "dr-graph"
            }
        }),
    )?;
    let xs = [-9.0_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.0];
    mcp.call_ok("command_run", json!({ "command_id": "table.setXCells", "params": { "sheet_id": "dr-xy", "values": xs } }))?;
    let mut dr_edits = Vec::new();
    for (r, x) in xs.iter().enumerate() {
        let y = 5.0 + 95.0 / (1.0 + 10.0_f64.powf(x - (-7.0)));
        for s in 0..3 {
            let val = if r == 5 && s == 1 { 195.0 } else { y + ((s as f64) - 1.0) * 1.2 };
            dr_edits.push(json!({ "row": r, "col": 0, "subcol": s, "value": val }));
        }
    }
    mcp.call_ok("command_run", json!({ "command_id": "table.setCells", "params": { "sheet_id": "dr-xy", "edits": dr_edits } }))?;
    mcp.call_ok(
        "command_run",
        json!({
            "command_id": "fit.run",
            "params": {
                "sheet_id": "dr-fit",
                "source_sheet_id": "dr-xy",
                "equation_id": "log_inhibitor_vs_response_4pl",
                "method": "rout",
                "identify_unstable_parameters": true
            }
        }),
    )?;
    let (_, fit_params) = mcp.call_ok("doc_inspect", json!({ "sheet_id": "dr-fit", "include_cells": true, "tab_id": "nonlinear_parameters" }))?;
    let log_ic50 = fit_params["scalars"]
        .as_array()
        .ok_or("missing scalars")?
        .iter()
        .find(|s| s["key"] == json!("LogIC50"))
        .and_then(|s| s["value"].as_f64())
        .ok_or("missing LogIC50")?;
    if (log_ic50 - (-7.0)).abs() > 0.1 {
        return Err(format!("ROUT fit LogIC50 drifted: {log_ic50}"));
    }
    let (res_full_bytes, _) = mcp.request("resources/read", json!({ "uri": "result://dr-fit" }))?;
    let (res_trim_bytes, res_trim_resp) = mcp.request("resources/read", json!({ "uri": "result://dr-fit?include_curves=false" }))?;
    if res_trim_bytes * 2 >= res_full_bytes {
        return Err(format!("expected include_curves=false to cut >50%: full={res_full_bytes}B trim={res_trim_bytes}B"));
    }
    let trim_val: Value =
        serde_json::from_str(res_trim_resp["result"]["contents"][0]["text"].as_str().ok_or("missing text")?).map_err(|e| e.to_string())?;
    if trim_val["fitted_curves"][0]["points_omitted"] != json!(true) {
        return Err(format!("expected points_omitted=true in trimmed result resource: {trim_val}"));
    }

    // Workflow 4: publication graph preflight, compact graph metadata, render_preview, SVG/PDF/PNG export
    mcp.call_ok(
        "command_run",
        json!({ "command_id": "graph.setRegressionOverlay", "params": { "sheet_id": "dr-graph", "result_sheet_id": "dr-fit", "all_series": true } }),
    )?;
    mcp.call_ok("command_run", json!({ "command_id": "graph.preflightCheck", "params": { "sheet_id": "dr-graph" } }))?;
    let (gmeta_bytes, gmeta) = mcp.call_ok("doc_inspect", json!({ "sheet_id": "dr-graph", "include_cells": false }))?;
    if gmeta.get("spec").is_some() || gmeta_bytes > 2048 {
        return Err(format!("graph include_cells=false bloated ({gmeta_bytes}B): {gmeta}"));
    }
    let (_, preview_resp) = mcp.request(
        "tools/call",
        json!({ "name": "render_preview", "arguments": { "sheet_id": "dr-graph", "width_px": 320, "height_px": 240, "dpi": 144 } }),
    )?;
    if preview_resp["result"]["isError"] != json!(false) || preview_resp["result"]["content"][0]["mimeType"] != json!("image/png") {
        return Err(format!("render_preview failed: {preview_resp}"));
    }
    for (fmt, ext, magic) in [("svg", "svg", b"<svg".as_slice()), ("pdf", "pdf", b"%PDF".as_slice()), ("png", "png", b"\x89PNG".as_slice())] {
        let out = dir.path.join(format!("figure.{ext}"));
        mcp.call_ok("export", json!({ "sheet_id": "dr-graph", "format": fmt, "output_path": out.display().to_string(), "dpi": 300 }))?;
        let data = fs::read(&out).map_err(|e| format!("read {ext}: {e}"))?;
        if !data.windows(magic.len()).any(|w| w == magic) {
            return Err(format!("exported `{ext}` missing expected signature"));
        }
    }
    Ok(())
}
