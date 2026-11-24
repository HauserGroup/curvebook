use std::fs;
use std::sync::Arc;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ChecklistItem, ChecklistStatus, DataSheet, DescriptiveSpec, ErrorCode, ProjectDocument, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_engine::{EngineError, Session};
use curvebook_format::ZipWriter;
use curvebook_mcp::{GUI_TOOL_NAMES, GuiSessionHost, McpServerState, call_tool, dispatch_json_value, list_gui_tools, list_tools_for_state};
use curvebook_platform::sync::Mutex;
use serde_json::{Value, json};

use super::common::TempDir;

type RecordedCalls = Arc<Mutex<Vec<(String, Value)>>>;

struct RecordingGuiHost {
    gui_only: bool,
    calls: RecordedCalls,
}

impl RecordingGuiHost {
    fn new() -> (Self, RecordedCalls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (Self { gui_only: false, calls: Arc::clone(&calls) }, calls)
    }
}

impl GuiSessionHost for RecordingGuiHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        self.calls.lock().expect("lock calls").push((method.to_string(), params));
        Ok(json!({ "ok": true, "method": method }))
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.gui_only = gui_only;
    }

    fn is_gui_only(&self) -> bool {
        self.gui_only
    }
}

fn call_tool_json(state: &mut McpServerState, id: u64, name: &str, args: Value) -> Value {
    dispatch_json_value(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": args
            }
        }),
        state,
    )
    .expect("jsonrpc tools/call reply")
}

fn structured_error_code(resp: &Value) -> Option<String> {
    if resp.pointer("/result/isError") != Some(&json!(true)) {
        return None;
    }
    resp.pointer("/result/structuredContent/code").and_then(Value::as_str).map(str::to_owned)
}

fn result_payload_json(resp: &Value) -> Value {
    assert_ne!(resp.pointer("/result/isError"), Some(&json!(true)), "unexpected error tool response: {resp}");
    if let Some(sc) = resp.pointer("/result/structuredContent")
        && !sc.is_null()
    {
        return sc.clone();
    }
    let text = resp.pointer("/result/content/0/text").and_then(Value::as_str).expect("tool text content");
    serde_json::from_str(text).expect("tool JSON content")
}

#[test]
fn fa_sec21_01_and_02_ui_metrics_schema_and_normalization() {
    let gui_tools = list_gui_tools();
    for tool_name in ["ui_pointer_gesture", "ui_inspect"] {
        let def = gui_tools.iter().find(|t| t.name == tool_name).expect("gui tool definition");
        let metrics = &def.input_schema["properties"]["metrics"];
        assert_eq!(metrics["type"], json!("object"), "{tool_name} metrics schema must be object");
        assert_eq!(metrics["additionalProperties"], json!(false), "{tool_name} metrics must set additionalProperties: false");
        let props = metrics["properties"].as_object().expect("metrics properties object");
        for key in ["logical_width", "logical_height", "scale_factor", "width_px", "height_px"] {
            assert!(props.contains_key(key), "{tool_name} missing metrics property `{key}`");
        }
    }

    let (host, calls) = RecordingGuiHost::new();
    let mut state = McpServerState::with_gui_host(Box::new(host)).with_gui_only(true);

    let gesture_px = call_tool(
        &mut state,
        "ui_pointer_gesture",
        json!({
            "action": "click",
            "x": 120.0,
            "y": 80.0,
            "metrics": { "width_px": 1200.0, "height_px": 800.0, "scale_factor": 2.0 }
        }),
    )
    .expect("ui_pointer_gesture px normalization");
    assert!(!gesture_px.is_error);

    let inspect_px = call_tool(
        &mut state,
        "ui_inspect",
        json!({
            "scope": "workspace",
            "include_bounds": true,
            "metrics": { "width_px": 1024.0, "height_px": 768.0 }
        }),
    )
    .expect("ui_inspect px normalization");
    assert!(!inspect_px.is_error);

    let inspect_canonical = call_tool(
        &mut state,
        "ui_inspect",
        json!({
            "metrics": { "logical_width": 1440.0, "logical_height": 900.0, "scale_factor": 1.5 }
        }),
    )
    .expect("ui_inspect canonical metrics");
    assert!(!inspect_canonical.is_error);

    let recorded = calls.lock().expect("lock recorded");
    assert_eq!(recorded.len(), 3);
    assert_eq!(
        recorded[0],
        (
            String::from("ui.pointer_gesture"),
            json!({
                "action": "click",
                "x": 120.0,
                "y": 80.0,
                "metrics": {
                    "logical_width": 1200.0,
                    "logical_height": 800.0,
                    "scale_factor": 2.0
                }
            })
        )
    );
    assert_eq!(
        recorded[1],
        (
            String::from("ui.inspect"),
            json!({
                "scope": "workspace",
                "include_bounds": true,
                "metrics": {
                    "logical_width": 1024.0,
                    "logical_height": 768.0
                }
            })
        )
    );
    assert_eq!(
        recorded[2],
        (
            String::from("ui.inspect"),
            json!({
                "metrics": {
                    "logical_width": 1440.0,
                    "logical_height": 900.0,
                    "scale_factor": 1.5
                }
            })
        )
    );
    drop(recorded);

    for bad_metrics in [
        json!("not-an-object"),
        json!({ "logical_width": 1200.0, "width_px": 900.0, "logical_height": 800.0 }),
        json!({ "width_px": "1200" }),
        json!({ "logical_width": 0.0, "logical_height": 800.0 }),
        json!({ "logical_width": 1200.0, "logical_height": -1.0 }),
        json!({ "logical_width": 1200.0, "logical_height": 800.0, "scale_factor": 0.0 }),
        json!({ "logical_width": 1200.0, "logical_height": 800.0, "dpi": 150 }),
    ] {
        let gesture_res =
            call_tool(&mut state, "ui_pointer_gesture", json!({ "action": "click", "x": 10.0, "y": 10.0, "metrics": bad_metrics.clone() }));
        let gesture_rejected = match &gesture_res {
            Err(curvebook_mcp::ToolCallError::InvalidParams(_)) => true,
            Ok(ok) => ok.is_error,
            Err(_) => false,
        };
        assert!(gesture_rejected, "gesture accepted invalid metrics: {bad_metrics} -> {gesture_res:?}");

        let inspect_res = call_tool(&mut state, "ui_inspect", json!({ "metrics": bad_metrics.clone() }));
        let inspect_rejected = match &inspect_res {
            Err(curvebook_mcp::ToolCallError::InvalidParams(_)) => true,
            Ok(ok) => ok.is_error,
            Err(_) => false,
        };
        assert!(inspect_rejected, "inspect accepted invalid metrics: {bad_metrics} -> {inspect_res:?}");
    }

    assert_eq!(calls.lock().expect("lock calls").len(), 3, "invalid metrics must not forward to GUI host");
}

#[test]
fn fa_sec21_03_headless_doc_inspect_and_resource_bounding() {
    let mut doc = ProjectDocument::new("Large Sheet Preview Audit").expect("new doc");

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 250).expect("xy table");
    xy.data_sets[0].title = format!("LongSeries_{}", "X".repeat(320));
    for r in 0..250 {
        xy.x_cells[r][0] = Cell::number(r as f64);
        xy.data_sets[0].cells[r][0] = Cell::text(format!("val_{r:03}_{}", "V".repeat(400)));
    }
    let big_ds = DataSheet::new("big_data", "Big Data Table", TableArchetype::Xy(xy)).expect("big_ds");
    doc.add_data_sheet(big_ds).expect("add big_data");

    let analysis = AnalysisSpec::new("big_result", "big_data", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis spec");
    let mut result = ResultSheet::new(analysis).expect("result sheet");
    result.status = ResultStatus::UpToDate;

    let tab0 = ResultTab {
        id: String::from("tab_rows"),
        title: String::from("Many Rows"),
        kind: ResultTabKind::MainSummary,
        column_headers: (0..10).map(|c| format!("C{c}")).collect(),
        row_labels: (0..180).map(|r| format!("R{r}")).collect(),
        rows: (0..180).map(|r| (0..10).map(|c| ResultCell::Number { value: (r * 10 + c) as f64, prefix: None, units: None }).collect()).collect(),
        footnotes: vec![],
    };
    result.tabs.push(tab0);

    let tab1 = ResultTab {
        id: String::from("tab_cols"),
        title: String::from("Many Cols"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: (0..80).map(|c| format!("WideCol{c}")).collect(),
        row_labels: (0..25).map(|r| format!("Row{r}")).collect(),
        rows: (0..25).map(|r| (0..80).map(|c| ResultCell::Number { value: (r * 80 + c) as f64, prefix: None, units: None }).collect()).collect(),
        footnotes: vec![],
    };
    result.tabs.push(tab1);

    for t in 2..35 {
        result.tabs.push(ResultTab {
            id: format!("tab_{t}"),
            title: format!("Extra {t}"),
            kind: ResultTabKind::MainSummary,
            column_headers: vec![String::from("K"), String::from("V")],
            row_labels: vec![format!("row_{t}")],
            rows: vec![vec![ResultCell::Text(format!("k{t}")), ResultCell::Number { value: t as f64, prefix: None, units: None }]],
            footnotes: vec![],
        });
    }
    for c in 0..150 {
        result.checklist.push(ChecklistItem {
            assumption: format!("Check #{c}"),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: String::from("ok"),
            remediation: None,
        });
    }
    for s in 0..2200 {
        result.scalars.push(ResultScalarEntry { key: format!("scalar_{s:04}"), data_set_index: None, value: s as f64, units: None });
    }
    doc.add_result_sheet(result).expect("add big_result");

    let mut state = McpServerState::from_session(Session::from_document(doc));

    let data_resp = call_tool_json(&mut state, 1, "doc_inspect", json!({ "sheet_id": "big_data", "include_cells": true }));
    let data_json = result_payload_json(&data_resp);
    assert_eq!(data_json["rows"], json!(250), "data_json={data_json}");
    assert_eq!(data_json["csv_preview_truncated"], json!(true));
    let csv_preview = data_json["csv_preview"].as_str().expect("csv_preview");
    assert!(csv_preview.len() <= 65_536, "csv_preview exceeded 64 KiB cap: {}", csv_preview.len());
    assert!(csv_preview.lines().count() <= 101, "csv_preview exceeded 100 data rows: {}", csv_preview.lines().count());

    let res_resp = call_tool_json(&mut state, 2, "doc_inspect", json!({ "sheet_id": "big_result", "include_cells": true }));
    let res_json = result_payload_json(&res_resp);
    assert_eq!(res_json["tabs_truncated"], json!(true), "res_json={res_json}");
    assert_eq!(res_json["tabs_count"], json!(35));
    let tabs = res_json["tabs"].as_array().expect("tabs array");
    assert_eq!(tabs.len(), 32);
    assert!(tabs[0]["rows"].as_array().map(Vec::len).unwrap_or(999) <= 100);
    assert!(tabs[1]["column_headers"].as_array().map(Vec::len).unwrap_or(999) <= 64);
    assert_eq!(res_json["checklist_truncated"], json!(true));
    assert_eq!(res_json["checklist"].as_array().map(Vec::len), Some(128));
    assert_eq!(res_json["scalars_truncated"], json!(true));
    assert_eq!(res_json["scalars"].as_array().map(Vec::len), Some(2048));

    let res_read =
        dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": { "uri": "sheet://big_result" } }), &mut state)
            .expect("jsonrpc resources/read reply");
    let res_text = res_read["result"]["contents"][0]["text"].as_str().expect("resource result text");
    let res_res_json: Value = serde_json::from_str(res_text).expect("resource json");
    assert_eq!(res_res_json["tabs_truncated"], json!(true));
    assert_eq!(res_res_json["tabs"].as_array().map(Vec::len), Some(32));
    assert!(res_res_json["tabs"][0]["rows"].as_array().map(Vec::len).unwrap_or(999) <= 100);
    assert_eq!(res_res_json["checklist_truncated"], json!(true));
    assert_eq!(res_res_json["checklist"].as_array().map(Vec::len), Some(128));
    assert_eq!(res_res_json["scalars_truncated"], json!(true));
    assert_eq!(res_res_json["scalars"].as_array().map(Vec::len), Some(2048));

    let summary_resp = call_tool_json(&mut state, 4, "doc_inspect", json!({}));
    let summary_json = result_payload_json(&summary_resp);
    let summaries = summary_json["analyses"].as_array().expect("analyses summary");
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0]["tabs_count"], json!(35));
    assert_eq!(summaries[0]["scalars_truncated"], json!(true));
    assert_eq!(summaries[0]["scalars"].as_array().map(Vec::len), Some(2048));
}

#[test]
fn fa_sec21_04_live_report_import_uses_path_for_prism_and_pzfx() {
    let dir = TempDir::new("live-import-path").unwrap();
    let pzfx_path = dir.path.join("assay.pzfx");
    let archive_file = dir.path.join("bundle.prism");

    let pzfx_xml = r#"<PrismFile>
  <Title>Cleanroom Enzyme Assay</Title>
  <TableSequence>
    <Table ID="tbl1" TableType="XY">
      <Title>Michaelis Data</Title>
      <XColumn><Title>Substrate</Title><Subcolumn><d>1</d><d>5</d><d>10</d></Subcolumn></XColumn>
      <YColumn><Title>Velocity</Title><Subcolumn><d>4.2</d><d>14.1</d><d>21.3</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
</PrismFile>"#;
    fs::write(&pzfx_path, pzfx_xml).expect("write pzfx");

    let mut zw = ZipWriter::new();
    let manifest = br#"{"title":"Archive","sheets":[{"id":"dose","title":"Dose","kind":"xy","csv_path":"tables/dose.csv"}]}"#;
    let table_csv = b"Dose,Response\n-8,5\n-6,50\n-4,95\n";
    zw.add_file("manifest.json", manifest.to_vec()).expect("zip manifest");
    zw.add_file("tables/dose.csv", table_csv.to_vec()).expect("zip table");
    fs::write(&archive_file, zw.finish().expect("finish zip")).expect("write zip archive");

    let recorded: RecordedCalls = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&recorded);
    let mut live_state = McpServerState::with_control_port(49777).expect("with_control_port").with_control_handler(move |method, params| {
        rec.lock().expect("lock").push((method.to_string(), params.clone()));
        match method {
            "ping" => Ok(json!({ "pong": true, "gui_only": false })),
            "engine.execute" => Ok(json!({
                "command_id": params["command_id"],
                "ok": true
            })),
            "command.batch" => Ok(json!({
                "ok": true,
                "results": params["commands"].as_array().map(|c| c.len()).unwrap_or(0)
            })),
            other => Err(format!("unexpected method {other}")),
        }
    });

    let pzfx_resp = call_tool_json(
        &mut live_state,
        1,
        "open_file",
        json!({ "path": pzfx_path.display().to_string(), "sheet_id": "custom_pzfx", "append": true }),
    );
    let _ = result_payload_json(&pzfx_resp);

    let archive_resp = call_tool_json(&mut live_state, 2, "open_file", json!({ "path": archive_file.display().to_string() }));
    let _ = result_payload_json(&archive_resp);

    let calls = recorded.lock().expect("lock recorded");
    let exec_calls: Vec<&Value> = calls.iter().filter(|(m, _)| m == "engine.execute").map(|(_, p)| p).collect();
    let batch_calls: Vec<&Value> = calls.iter().filter(|(m, _)| m == "command.batch").map(|(_, p)| p).collect();
    assert_eq!(exec_calls.len(), 1, "expected 1 live engine.execute call, got {calls:?}");
    assert_eq!(batch_calls.len(), 1, "expected 1 live command.batch call, got {calls:?}");

    let pzfx_cmd = exec_calls[0];
    assert_eq!(pzfx_cmd["command_id"], json!("table.importPzfx"));
    assert!(pzfx_cmd["params"].get("path").and_then(Value::as_str).is_some(), "pzfx import must forward params.path");
    assert!(pzfx_cmd["params"].get("xml").is_none(), "pzfx import must not serialize full XML inline");
    assert_eq!(pzfx_cmd["params"]["merge"], json!(true));

    let archive_step = &batch_calls[0]["commands"][1];
    assert_eq!(archive_step["command"], json!("table.importPrism"));
    assert!(archive_step["params"].get("path").and_then(Value::as_str).is_some(), "archive import must forward params.path");
    assert_eq!(archive_step["params"]["merge"], json!(false));
    assert!(archive_step["params"].get("archive_base64").is_none(), "archive import must not allocate base64 archive inline");
}

#[test]
fn fa_sec21_05_reject_unknown_args_on_all_18_builtin_tools() {
    let dir = TempDir::new("unknown-args").unwrap();
    let sample_csv = dir.path.join("sample.csv");
    fs::write(&sample_csv, "X,Y\n1,2\n3,4\n").expect("write csv");

    let (gui_host, gui_calls) = RecordingGuiHost::new();
    let mut state = McpServerState::with_workdir(&dir.path).expect("with_workdir");
    state.set_gui_host(Some(Box::new(gui_host)));
    let _ = call_tool(&mut state, "enable_toolset", json!({ "group": "gui" })).expect("enable gui toolset");

    let all_tools = list_tools_for_state(&state);
    let core_names = [
        "doc_inspect",
        "open_file",
        "save_file",
        "render_preview",
        "export",
        "command_list",
        "command_run",
        "command_batch",
        "list_toolsets",
        "enable_toolset",
    ];
    for name in core_names.into_iter().chain(GUI_TOOL_NAMES) {
        assert!(all_tools.iter().any(|t| t.name == name), "missing tool `{name}`");
    }

    let cases: [(&str, Value); 18] = [
        ("doc_inspect", json!({ "unexpected": true })),
        ("open_file", json!({ "path": sample_csv.display().to_string(), "unexpected": 1 })),
        ("save_file", json!({ "path": "out.curvebook", "unexpected": 1 })),
        ("render_preview", json!({ "target": "document", "unexpected": 1 })),
        ("export", json!({ "path": "out.csv", "format": "csv", "unexpected": 1 })),
        ("command_list", json!({ "unexpected": "x" })),
        ("command_run", json!({ "command_id": "sheet.select", "unexpected": true })),
        ("command_batch", json!({ "commands": [], "unexpected": true })),
        ("list_toolsets", json!({ "unexpected": true })),
        ("enable_toolset", json!({ "group": "stats", "unexpected": true })),
        ("ui_inspect", json!({ "scope": "window", "unexpected": true })),
        ("ui_screenshot", json!({ "target": "workspace", "format": "png", "unexpected": true })),
        ("ui_pointer_gesture", json!({ "action": "click", "x": 1.0, "y": 2.0, "unexpected": true })),
        ("ui_type_text", json!({ "text": "abc", "unexpected": true })),
        ("ui_press_key", json!({ "key": "Enter", "unexpected": true })),
        ("ui_invoke_menu", json!({ "command_id": "file.new", "unexpected": true })),
        ("ui_switch_tab", json!({ "tab": "graphs", "unexpected": true })),
        ("ui_answer_dialog", json!({ "action": "confirm", "unexpected": true })),
    ];

    for (idx, (name, args)) in cases.iter().enumerate() {
        let resp = call_tool_json(&mut state, (idx + 1) as u64, name, args.clone());
        let is_rejected = resp["error"]["code"] == json!(-32602) || structured_error_code(&resp).as_deref() == Some("bad_input");
        assert!(is_rejected, "tool `{name}` did not reject unknown args with -32602/bad_input: {resp}");
    }

    let modifier_resp = call_tool_json(&mut state, 99, "ui_press_key", json!({ "key": "Enter", "modifiers": { "shift": true, "bogus": true } }));
    assert!(modifier_resp["error"]["code"] == json!(-32602) || structured_error_code(&modifier_resp).as_deref() == Some("bad_input"));

    let (hidden_host, _) = RecordingGuiHost::new();
    let mut non_gui = McpServerState::with_gui_host(Box::new(hidden_host));
    let hidden_gui = call_tool_json(&mut non_gui, 100, "ui_inspect", json!({ "bogus": 1 }));
    assert!(hidden_gui["error"].is_object() || structured_error_code(&hidden_gui).is_some());

    assert!(gui_calls.lock().expect("lock gui_calls").is_empty(), "rejected GUI requests must never reach host");
}

#[test]
fn fa_sec04_04_csv_export_fails_closed_on_failed_result_sheet() {
    let dir = TempDir::new("failed-result-csv").unwrap();
    let mut doc = ProjectDocument::new("Failed Result Sheet Audit").expect("new doc");
    let xy = XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy table");
    let ds = DataSheet::new("data_tbl", "Data", TableArchetype::Xy(xy)).expect("data sheet");
    doc.add_data_sheet(ds).expect("add data");

    let analysis = AnalysisSpec::new("failed_res", "data_tbl", AnalysisKind::DescriptiveStats(DescriptiveSpec::default())).expect("analysis");
    let mut failed_res = ResultSheet::new(analysis).expect("failed_res");
    failed_res.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("insufficient replicates") };
    failed_res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Mean"), String::from("SD")],
        row_labels: vec![String::from("Row1")],
        rows: vec![vec![ResultCell::Number { value: 1.5, prefix: None, units: None }, ResultCell::Number { value: 0.7, prefix: None, units: None }]],
        footnotes: vec![],
    });
    doc.add_result_sheet(failed_res).expect("add failed_res");

    let mut state = McpServerState::from_session(Session::from_document(doc));
    state.set_workdir(Some(&dir.path)).expect("workdir");

    for (id, file, args) in [
        (1, "default.csv", json!({ "path": "default.csv", "format": "csv", "sheet_id": "failed_res" })),
        (2, "tab.csv", json!({ "path": "tab.csv", "format": "csv", "sheet_id": "failed_res", "tab_id": "summary" })),
        (3, "tidy.csv", json!({ "path": "tidy.csv", "format": "csv", "sheet_id": "failed_res", "tidy": true })),
        (4, "explicit.csv", json!({ "path": "explicit.csv", "format": "csv", "sheet_id": "failed_res", "tab_id": "summary", "tidy": false })),
    ] {
        let resp = call_tool_json(&mut state, id, "export", args);
        assert_eq!(resp["result"]["isError"], json!(true), "export to {file} must fail on Failed ResultSheet: {resp}");
        assert!(!dir.path.join(file).exists(), "failed ResultSheet export must not create {file}");
    }
}
