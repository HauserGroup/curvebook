//! Wave 2 remediation regression suite for `curvebook-mcp` (`fix-hosts`).
//!
//! Covers `AUD-06-08`, `AUD-06-09`, `AUD-06-16`, `AUD-13-03`, `AUD-13-10`, `AUD-13-11`,
//! `AUD-13-12`, `AUD-13-13`, `AUD-13-14`, `AUD-13-22`, and `AUD-13-23`.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CurveSamplePoint, ErrorCode, FittedCurveOverlay, LinearRegressionSpec, ProjectDocument, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
};
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
use curvebook_mcp::{
    GuiSessionHost, McpServerState, ResourceError, ToolCallError, call_tool, dispatch_json_value, list_tools, read_resource, serve_concurrent,
};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(1);
        let dir = std::env::temp_dir().join(format!("cb-rem-hosts-mcp-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

type CallLog = Rc<RefCell<Vec<(String, Value)>>>;
type NextReplySlot = Rc<RefCell<Option<Value>>>;

struct RecordingGuiHost {
    session: Session,
    calls: CallLog,
    next_reply: NextReplySlot,
}

impl RecordingGuiHost {
    fn new(session: Session) -> (Self, CallLog, NextReplySlot) {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let next_reply = Rc::new(RefCell::new(None));
        (Self { session, calls: Rc::clone(&calls), next_reply: Rc::clone(&next_reply) }, calls, next_reply)
    }
}

impl GuiSessionHost for RecordingGuiHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        self.calls.borrow_mut().push((method.to_owned(), params));
        Ok(self.next_reply.borrow_mut().take().unwrap_or_else(|| json!({ "ok": true })))
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(&self.session)
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(&mut self.session)
    }

    fn sync_after_session_mutation(&mut self) {}
}

fn build_sample_xlsx_bytes() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Minimal valid OOXML .xlsx workbook with 1 worksheet ("Assay") containing 2 numeric columns.
    let mut zw = curvebook_format::zip::ZipWriter::new();
    zw.add_file(
        "[Content_Types].xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>"#
            .to_vec(),
    )?;
    zw.add_file(
        "_rels/.rels",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/workbook.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Assay" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/_rels/workbook.xml.rels",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#
            .to_vec(),
    )?;
    zw.add_file(
        "xl/worksheets/sheet1.xml",
        br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1" t="inlineStr"><is><t>Dose</t></is></c><c r="B1" t="inlineStr"><is><t>Response</t></is></c></row>
    <row r="2"><c r="A2"><v>1</v></c><c r="B2"><v>10</v></c></row>
    <row r="3"><c r="A3"><v>2</v></c><c r="B3"><v>25</v></c></row>
  </sheetData>
</worksheet>"#
            .to_vec(),
    )?;
    Ok(zw.finish()?)
}

fn encode_test_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(TABLE[((n >> 18) & 0x3F) as usize]));
        out.push(char::from(TABLE[((n >> 12) & 0x3F) as usize]));
        out.push(if chunk.len() > 1 { char::from(TABLE[((n >> 6) & 0x3F) as usize]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[(n & 0x3F) as usize]) } else { '=' });
    }
    out
}

#[test]
fn aud_06_08_live_resource_query_and_doc_inspect_filters() -> TestResult {
    let result_json = json!({
        "id": "r1",
        "title": "Fit",
        "status": "UpToDate",
        "tabs": [
            { "id": "summary", "title": "Summary", "column_headers": ["V"], "row_labels": ["R1", "R2", "R3"], "rows": [[1], [2], [3]], "footnotes": [] },
            { "id": "residuals", "title": "Residuals", "column_headers": ["Res"], "row_labels": ["R1"], "rows": [[0.1]], "footnotes": [] }
        ],
        "fitted_curves": [
            { "data_set_index": 0, "equation_id": "linear", "converged": true, "ambiguous": false, "points": [{ "x": 0.0, "y": 1.0 }, { "x": 1.0, "y": 2.0 }] }
        ]
    });
    let result_b64 = encode_test_base64(serde_json::to_string(&result_json)?.as_bytes());

    let mut state = McpServerState::with_control_port(49808)?.with_control_handler(move |method, params| match method {
        "state.snapshot" => Ok(json!({ "active_sheet_id": "d1" })),
        "doc.inspect" => match params.get("sheet_id").and_then(Value::as_str) {
            None => Ok(json!({
                "title": "Live Study",
                "counts": { "total": 2 },
                "data_sheets": [{ "id": "d1", "title": "Data 1" }],
                "result_sheets": [{ "id": "r1", "title": "Fit", "scalars": [{ "key": "a" }, { "key": "b" }] }],
                "graph_sheets": [{ "id": "g1", "title": "Graph 1", "spec": { "heavy": true } }]
            })),
            Some("d1") => Ok(json!({
                "id": "d1",
                "section": "Data",
                "csv_preview": "X,Y\n1,10\n2,20\n3,30\n4,40"
            })),
            Some("r1") => Ok(json!({
                "id": "r1",
                "section": "Results",
                "tabs_data": [
                    { "id": "summary", "rows": [[1], [2], [3]], "row_labels": ["a", "b", "c"] },
                    { "id": "residuals", "rows": [[0.1]], "row_labels": ["a"] }
                ],
                "scalars_data": [{ "key": "k1" }, { "key": "k2" }, { "key": "k3" }]
            })),
            Some(other) => Err(format!("sheet `{other}` not found")),
        },
        "doc.export" => Ok(json!({ "data_base64": result_b64 })),
        _ => Err(format!("unexpected method {method}")),
    });

    // 1. Live `resources/read` with `?max_rows=2` strips query before sending `sheet_id`.
    let sheet_reply = dispatch_json_value(
        json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": { "uri": "sheet://d1?max_rows=2" } }),
        &mut state,
    )
    .ok_or("missing sheet reply")?;
    assert!(sheet_reply.get("error").is_none(), "live sheet query failed: {sheet_reply}");
    let sheet_text = sheet_reply["result"]["contents"][0]["text"].as_str().ok_or("missing text")?;
    let sheet_val: Value = serde_json::from_str(sheet_text)?;
    assert_eq!(sheet_val["csv_preview"], json!("X,Y\n1,10\n2,20"));
    assert_eq!(sheet_val["csv_preview_truncated"], json!(true));

    // 2. Live `result://r1?include_curves=false&tab_id=summary&max_rows=1` filters tabs and curves.
    let res_reply = dispatch_json_value(
        json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": { "uri": "result://r1?include_curves=false&tab_id=summary&max_rows=1" } }),
        &mut state,
    )
    .ok_or("missing result reply")?;
    assert!(res_reply.get("error").is_none(), "live result query failed: {res_reply}");
    let res_val: Value = serde_json::from_str(res_reply["result"]["contents"][0]["text"].as_str().ok_or("text")?)?;
    assert_eq!(res_val["tabs"].as_array().map(Vec::len), Some(1));
    assert_eq!(res_val["tabs"][0]["rows"].as_array().map(Vec::len), Some(1));
    assert_eq!(res_val["fitted_curves"][0]["points_omitted"], json!(true));
    assert_eq!(res_val["fitted_curves"][0]["points_count"], json!(2));

    // 3. Unknown `tab_id` returns -32002 ResourceNotFound.
    let bad_tab = dispatch_json_value(
        json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": "result://r1?tab_id=missing" } }),
        &mut state,
    )
    .ok_or("missing bad tab reply")?;
    assert_eq!(bad_tab["error"]["code"], json!(-32002));

    // 4. Live `doc_inspect` honors `sections` + `detail: compact` on overview and `tab_id`/`max_rows`/`max_scalars` on sheets.
    let ov = call_tool(&mut state, "doc_inspect", json!({ "sections": ["data"], "detail": "compact" })).map_err(|e| format!("{e}"))?;
    let ov_sc = ov.structured_content.ok_or("ov structured")?;
    assert!(ov_sc.get("data_sheets").is_some());
    assert!(ov_sc.get("graph_sheets").is_none());
    assert!(ov_sc.get("result_sheets").is_none());

    let sheet_filt = call_tool(&mut state, "doc_inspect", json!({ "sheet_id": "r1", "tab_id": "summary", "max_rows": 2, "max_scalars": 1 }))
        .map_err(|e| format!("{e}"))?;
    assert!(!sheet_filt.is_error);
    let sf = sheet_filt.structured_content.ok_or("sf structured")?;
    assert_eq!(sf["tabs_data"].as_array().map(Vec::len), Some(1));
    assert_eq!(sf["tabs_data"][0]["rows"].as_array().map(Vec::len), Some(2));
    assert_eq!(sf["scalars_data"].as_array().map(Vec::len), Some(1));
    assert_eq!(sf["scalars_truncated"], json!(true));
    Ok(())
}

#[test]
fn aud_06_09_open_file_subcolumn_mode_schema_and_none_alias() -> TestResult {
    let tools = list_tools();
    let open_def = tools.iter().find(|t| t.name == "open_file").ok_or("missing open_file")?;
    let desc = open_def.input_schema["properties"]["subcolumn_mode"]["description"].as_str().ok_or("missing desc")?;
    assert!(desc.contains("single"), "schema must mention 'single': {desc}");
    assert!(desc.contains("upper_lower_limits"), "schema must mention 'upper_lower_limits': {desc}");
    assert!(!desc.contains("'mean_sd'"), "schema must not advertise invalid 'mean_sd': {desc}");
    assert!(!desc.contains("'mean_sem'"), "schema must not advertise invalid 'mean_sem': {desc}");

    let td = TempDir::new()?;
    let csv_path = td.path().join("data.csv");
    fs::write(&csv_path, "X,Y\n1,10\n2,20\n")?;

    let mut state = McpServerState::new();
    let res = call_tool(&mut state, "open_file", json!({ "path": csv_path.display().to_string(), "subcolumn_mode": "none" }))
        .map_err(|e| format!("{e}"))?;
    assert!(!res.is_error, "subcolumn_mode 'none' must be accepted as Single alias");

    let bad = call_tool(&mut state, "open_file", json!({ "path": csv_path.display().to_string(), "subcolumn_mode": "mean_sd" }));
    assert!(matches!(bad, Err(ToolCallError::InvalidParams(ref m)) if m.contains("mean_sd_n")));
    Ok(())
}

#[test]
fn aud_06_16_result_resource_filtered_projection_omits_curves_and_slices_tabs() -> TestResult {
    let mut doc = ProjectDocument::new("Fit Study")?;
    let mut rs = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?)?;
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
    doc.add_result_sheet(rs)?;

    let contents = read_resource(&doc, "result://r1?include_curves=false&tab_id=summary&max_rows=2").map_err(|e| format!("{e}"))?;
    let parsed: Value = serde_json::from_str(&contents[0].text)?;
    assert_eq!(parsed["tabs"].as_array().map(Vec::len), Some(1));
    assert_eq!(parsed["tabs"][0]["id"], json!("summary"));
    assert_eq!(parsed["tabs"][0]["rows"].as_array().map(Vec::len), Some(2));
    assert_eq!(parsed["tabs"][0]["row_labels"].as_array().map(Vec::len), Some(2));
    assert_eq!(parsed["fitted_curves"][0]["points_count"], json!(500));
    assert_eq!(parsed["fitted_curves"][0]["points_omitted"], json!(true));
    assert!(parsed["fitted_curves"][0].get("points").is_none());
    assert!(parsed.get("eliminated_outliers").is_some());

    let missing = read_resource(&doc, "result://r1?tab_id=does_not_exist");
    assert!(matches!(missing, Err(ResourceError::NotFound(_))));
    Ok(())
}

#[test]
fn aud_13_03_and_23_reject_external_and_extra_path_keys_enforced() -> TestResult {
    let td = TempDir::new()?;
    let csv = td.path().join("in.csv");
    fs::write(&csv, "X,Y\n1,2\n")?;
    let out_cb = td.path().join("out.curvebook");
    let out_csv = td.path().join("out.csv");

    let mut state = McpServerState::new();
    state.session_mut().set_path_policy(ExternalPathPolicy::RejectExternal);

    for (tool, args) in [
        ("open_file", json!({ "path": csv.display().to_string() })),
        ("save_file", json!({ "path": out_cb.display().to_string() })),
        ("export", json!({ "format": "csv", "output_path": out_csv.display().to_string() })),
        ("command_run", json!({ "command_id": "project.save", "params": { "path": out_cb.display().to_string() } })),
        ("command_run", json!({ "command_id": "project.save", "params": { "destination_path": out_cb.display().to_string() } })),
        ("command_batch", json!({ "commands": [{ "command_id": "project.save", "params": { "output_path": out_cb.display().to_string() } }] })),
    ] {
        let res = call_tool(&mut state, tool, args).map_err(|e| format!("{tool}: {e}"))?;
        assert!(res.is_error, "{tool} must fail under RejectExternal");
        let sc = res.structured_content.ok_or("missing error structuredContent")?;
        assert_eq!(sc["code"], json!("path_escape"), "{tool} must report path_escape, got {sc}");
    }
    assert!(!out_cb.exists());
    assert!(!out_csv.exists());

    // Workdir mode must also reject traversal on extra path keys (`dir`, `output_path`, `unpacked_dir`).
    let wd = td.path().join("wd");
    fs::create_dir_all(&wd)?;
    let mut sandboxed = McpServerState::with_workdir(&wd)?;
    for key in ["dir", "directory", "destination_path", "unpacked_dir", "output_path"] {
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_owned(), json!("../escape.out"));
        let res = call_tool(&mut sandboxed, "command_run", json!({ "command_id": "project.save", "params": Value::Object(obj) }))
            .map_err(|e| format!("{key}: {e}"))?;
        assert!(res.is_error, "extra path key `{key}` traversal must fail");
        let sc = res.structured_content.ok_or("missing structured")?;
        assert_eq!(sc["code"], json!("path_escape"), "extra path key `{key}` must fail with path_escape: {sc}");
    }
    Ok(())
}

#[test]
fn aud_13_11_and_12_gui_tools_choice_tab_and_path_only_screenshot() -> TestResult {
    let (host, calls, next_reply) = RecordingGuiHost::new(Session::new());
    let mut state = McpServerState::with_gui_host(Box::new(host));

    // `ui_switch_tab` normalizes integer index to string.
    let sw = call_tool(&mut state, "ui_switch_tab", json!({ "tab": 1 })).map_err(|e| format!("{e}"))?;
    assert!(!sw.is_error);

    // `ui_answer_dialog` mirrors top-level `choice` into `fields` when no fields map was provided.
    let ans = call_tool(&mut state, "ui_answer_dialog", json!({ "action": "ok", "choice": "modify_existing" })).map_err(|e| format!("{e}"))?;
    assert!(!ans.is_error);

    // `ui_screenshot` succeeds when backend omits inline base64 because it wrote to `path`.
    *next_reply.borrow_mut() = Some(json!({
        "path": "/tmp/shot.png",
        "width_px": 2400,
        "height_px": 1800,
        "dpi": 300,
        "byte_len": 13_000_000,
        "inline_omitted": true
    }));
    let shot = call_tool(&mut state, "ui_screenshot", json!({ "target": "full_window" })).map_err(|e| format!("{e}"))?;
    assert!(!shot.is_error, "path-only screenshot must succeed cleanly: {shot:?}");
    assert_eq!(shot.structured_content.as_ref().and_then(|v| v.get("inline_omitted")), Some(&json!(true)));

    let recorded = calls.borrow();
    assert_eq!(recorded[0].1["tab"], json!("1"));
    assert_eq!(recorded[1].1["choice"], json!("modify_existing"));
    assert_eq!(recorded[1].1["fields"]["choice"], json!("modify_existing"));
    Ok(())
}

#[test]
fn aud_13_13_live_command_batch_step_cap_1000_and_overflow_reporting() -> TestResult {
    // 1. Step cap aligned to 1_000 (1_001 rejected as InvalidParams).
    let mut headless = McpServerState::new();
    let too_many: Vec<Value> = (0..1_001).map(|_| json!({ "command_id": "project.inspect", "params": {} })).collect();
    let err = call_tool(&mut headless, "command_batch", json!({ "commands": too_many }));
    assert!(matches!(err, Err(ToolCallError::InvalidParams(ref m)) if m.contains("1000")));

    // 2. Live batch 16 MiB response overflow after a `project.save` step must report `filesystem_side_effects_persisted: true`.
    let mut live = McpServerState::with_control_port(49813)?.with_control_handler(|method, params| {
        if method == "command.batch" {
            let count = params["commands"].as_array().map_or(0, Vec::len);
            if count == 2 {
                return Err(String::from("control response exceeds maximum control line size (16777216 bytes)"));
            }
            return Err(String::from("command.batch exceeds maximum step limit (1000)"));
        }
        Ok(json!({}))
    });

    let overflow = call_tool(
        &mut live,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "project.save", "params": { "path": "saved.curvebook" } },
                { "command_id": "project.inspect", "params": {} }
            ]
        }),
    )
    .map_err(|e| format!("{e}"))?;
    assert!(overflow.is_error);
    let sc = overflow.structured_content.ok_or("missing sc")?;
    assert_eq!(sc["filesystem_side_effects_persisted"], json!(true), "overflow after save must flag persisted fs effects: {sc}");
    assert_eq!(sc["failed_index"], json!(1));

    // 3. Pre-execution rejection (before step 0 runs) must report `rolled_back: true` and `filesystem_side_effects_persisted: false`.
    let pre_rej = call_tool(
        &mut live,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [{ "command_id": "project.save", "params": { "path": "saved.curvebook" } }]
        }),
    )
    .map_err(|e| format!("{e}"))?;
    assert!(pre_rej.is_error);
    let pre_sc = pre_rej.structured_content.ok_or("missing pre_sc")?;
    assert_eq!(pre_sc["rolled_back"], json!(true), "pre-execution rejection must report rolled_back=true: {pre_sc}");
    assert_eq!(pre_sc["filesystem_side_effects_persisted"], json!(false));
    Ok(())
}

#[test]
fn aud_13_14_standalone_xlsx_title_and_append_and_live_csv_escape_fallback() -> TestResult {
    let td = TempDir::new()?;
    let xlsx_path = td.path().join("dose_study.xlsx");
    fs::write(&xlsx_path, build_sample_xlsx_bytes()?)?;

    // Standalone open_file preserves filename stem when title omitted, and preserves explicit title override.
    let mut state = McpServerState::new();
    let res_stem = call_tool(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string() })).map_err(|e| format!("{e}"))?;
    assert!(!res_stem.is_error);
    assert_eq!(state.session().document().title, "dose_study");

    let res_override = call_tool(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string(), "title": "Explicit Study Title" }))
        .map_err(|e| format!("{e}"))?;
    assert!(!res_override.is_error);
    assert_eq!(state.session().document().title, "Explicit Study Title");

    // Standalone append merges cleanly via path without wiping existing sheet.
    let res_append =
        call_tool(&mut state, "open_file", json!({ "path": xlsx_path.display().to_string(), "append": true })).map_err(|e| format!("{e}"))?;
    assert!(!res_append.is_error);
    assert!(state.session().document().data_sheets.len() >= 2);

    // Live-attach: quote-heavy ~4 MiB CSV whose JSON-escaped envelope exceeds 16 MiB must fall back to `path` mode,
    // and `.xlsx` with explicit `title` sends `file.new {title}` + `table.importXlsx {merge: true}`.
    let seen_batches = Arc::new(curvebook_platform::sync::Mutex::named("test.aud1314", Vec::<Value>::new()));
    let seen_clone = Arc::clone(&seen_batches);
    let mut live = McpServerState::with_control_port(49814)?.with_control_handler(move |method, params| {
        if method == "command.batch" {
            if let Ok(mut g) = seen_clone.lock() {
                g.push(params.clone());
            }
            return Ok(json!({ "results": [{ "ok": true }, { "ok": true }] }));
        }
        if method == "doc.inspect" {
            return Ok(json!({ "title": "Live Title", "counts": { "total": 1 }, "kind": "xy", "rows": 2, "cols": 2 }));
        }
        Ok(json!({}))
    });

    // Control-char-heavy TSV (~3.5 MiB raw -> ~21 MiB JSON-escaped `\u000b`!).
    let mut heavy_csv = String::from("X\tY\n");
    let cell_chunk = "\u{000b}".repeat(1024);
    for _ in 0..3_200 {
        heavy_csv.push_str("1\t");
        heavy_csv.push_str(&cell_chunk);
        heavy_csv.push('\n');
    }
    assert!(heavy_csv.len() < 8 * 1024 * 1024, "raw CSV should stay under naive 8 MiB threshold");
    let heavy_path = td.path().join("heavy_escaped.tsv");
    fs::write(&heavy_path, &heavy_csv)?;

    let live_csv_res = call_tool(&mut live, "open_file", json!({ "path": heavy_path.display().to_string() })).map_err(|e| format!("{e}"))?;
    assert!(!live_csv_res.is_error, "live heavy TSV open_file failed: {live_csv_res:?}");

    let live_xlsx_res = call_tool(&mut live, "open_file", json!({ "path": xlsx_path.display().to_string(), "title": "Live Custom Title" }))
        .map_err(|e| format!("{e}"))?;
    assert!(!live_xlsx_res.is_error, "live xlsx open_file failed: {live_xlsx_res:?}");

    let recorded = seen_batches.lock().map_err(|e| format!("{e}"))?;
    assert_eq!(recorded.len(), 2);
    // Batch 0: heavy TSV fell back to `"path"` instead of overflowing 16 MiB with inline `"csv"`.
    assert!(recorded[0]["commands"][1]["params"].get("path").is_some(), "expected path fallback for heavy escaped TSV: {}", recorded[0]);
    assert!(recorded[0]["commands"][1]["params"].get("csv").is_none());
    // Batch 1: xlsx explicit title passed via `file.new` + `merge: true`.
    assert_eq!(recorded[1]["commands"][0]["params"]["title"], json!("Live Custom Title"));
    assert_eq!(recorded[1]["commands"][1]["params"]["merge"], json!(true));
    Ok(())
}

struct ChannelBufReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: VecDeque<u8>,
}

impl Read for ChannelBufReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.buf.is_empty() {
            match self.rx.recv() {
                Ok(chunk) if !chunk.is_empty() => self.buf.extend(chunk),
                Ok(_) => continue,
                Err(_) => return Ok(0),
            }
        }
        let mut n = 0;
        while n < out.len() {
            if let Some(b) = self.buf.pop_front() {
                out[n] = b;
                n += 1;
            } else {
                break;
            }
        }
        Ok(n)
    }
}

#[test]
fn aud_13_22_serve_concurrent_delivers_cancel_while_request_in_flight() -> TestResult {
    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>();
    let reg_slot: Arc<curvebook_platform::sync::Mutex<Option<curvebook_mcp::CancellationRegistry>>> =
        Arc::new(curvebook_platform::sync::Mutex::named("test.reg_slot", None));
    let reg_slot_host = Arc::clone(&reg_slot);
    let (s_tx, s_rx) = mpsc::channel::<()>();
    let observed_final = Arc::new(AtomicBool::new(false));
    let observed_final_host = Arc::clone(&observed_final);

    struct SlotCancelHost {
        reg_slot: Arc<curvebook_platform::sync::Mutex<Option<curvebook_mcp::CancellationRegistry>>>,
        started_tx: Option<mpsc::Sender<()>>,
        observed: Arc<AtomicBool>,
    }
    impl GuiSessionHost for SlotCancelHost {
        fn call_gui(&mut self, _method: &str, _params: Value) -> Result<Value, EngineError> {
            if let Some(tx) = self.started_tx.take() {
                let _ = tx.send(());
            }
            let reg = self.reg_slot.lock().ok().and_then(|g| g.clone()).ok_or_else(|| EngineError::with_code(ErrorCode::InternalError, "no reg"))?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if reg.is_cancelled(&json!(42)) {
                    self.observed.store(true, Ordering::Release);
                    return Err(EngineError::with_code(ErrorCode::Cancelled, "cancelled in flight"));
                }
                thread::sleep(Duration::from_millis(5));
            }
            Ok(json!({ "timed_out": true }))
        }
    }

    let mut state =
        McpServerState::with_gui_host(Box::new(SlotCancelHost { reg_slot: reg_slot_host, started_tx: Some(s_tx), observed: observed_final_host }));
    if let Ok(mut g) = reg_slot.lock() {
        *g = Some(state.cancellation_registry());
    }

    let sender_handle = thread::spawn(move || {
        let req_line =
            format!("{}\n", json!({ "jsonrpc": "2.0", "id": 42, "method": "tools/call", "params": { "name": "ui_inspect", "arguments": {} } }));
        let _ = in_tx.send(req_line.into_bytes());
        if s_rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            let cancel_line = format!("{}\n", json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 42 } }));
            let _ = in_tx.send(cancel_line.into_bytes());
        }
        thread::sleep(Duration::from_millis(50));
        drop(in_tx);
    });

    let reader = BufReader::new(ChannelBufReader { rx: in_rx, buf: VecDeque::new() });
    let mut output = Vec::new();
    serve_concurrent(reader, &mut output, &mut state, 1024 * 1024)?;
    let _ = sender_handle.join();

    assert!(observed_final.load(Ordering::Acquire), "in-flight request #42 must observe cancellation from concurrent reader thread");
    let reply_str = String::from_utf8(output)?;
    let reply: Value = serde_json::from_str(reply_str.trim())?;
    assert_eq!(reply["id"], json!(42));
    assert_eq!(reply["result"]["isError"], json!(true));
    assert_eq!(reply["result"]["structuredContent"]["code"], json!("cancelled"));
    Ok(())
}
