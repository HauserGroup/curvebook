//! Wave 1 security regression suite for `FA-X1-01` across the `curvebook-mcp` `--gui-only`
//! boundary (`ui_invoke_menu` -> `Caller::Ui` escalation and external link exfiltration).
//!
//! Tests:
//! - Poisoned inline archives (`project.open` / `project.instantiateTemplate` `bytes_base64`),
//!   `table.importPrism` / `table.importPzfx`, `table.pasteLink` (`ExternalDelimitedFile`),
//!   `project.recalculate` (`include_external_links: true`), persistent CSV links, and unconstrained
//!   host path keys via `ui_invoke_menu` are rejected fail-closed with `gui_only_violation` under
//!   `--gui-only` without `--workdir`, via both `McpServerState::with_gui_host` (backed by a real
//!   `Session` calling `execute_as_ui`) and loopback `--control-port`.
//! - Legitimate in-memory `--gui-only` menu commands (`sheet.createData`, parameterless
//!   `Analyze/Recalculate All`, inline `layout.insertImage` `bytes_base64`, inline
//!   `table.importTidyLong`) and `--workdir`-confined operations continue to function cleanly.
#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use curvebook_doc::{
    CellCoord, ErrorCode, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, SheetCellRange, SheetId,
};
use curvebook_engine::{EngineError, ExternalPathPolicy, GuiOnlyPolicy, Session};
use curvebook_format::save_project_bytes;
use curvebook_mcp::{GuiSessionHost, McpServerState, call_tool, dispatch_json_value};
use serde_json::{Value, json};

type TestRes = Result<(), String>;

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cb-audit-w1-x1-{label}-{}-{seq}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path).map_err(|err| format!("create temp dir `{}`: {err}", path.display()))?;
        Ok(Self { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(char::from(TABLE[((n >> 18) & 0x3f) as usize]));
        out.push(char::from(TABLE[((n >> 12) & 0x3f) as usize]));
        out.push(if chunk.len() > 1 { char::from(TABLE[((n >> 6) & 0x3f) as usize]) } else { '=' });
        out.push(if chunk.len() > 2 { char::from(TABLE[(n & 0x3f) as usize]) } else { '=' });
    }
    out
}

/// Real `Session`-backed `GuiSessionHost` modeling `ui.invoke_menu` -> `Session::execute_as_ui`
/// (`Caller::Ui`) so any bypass of MCP filtering executes directly against `Session`.
struct RealSessionGuiHost {
    session: Session,
    gui_only: bool,
    invoked_methods: Vec<(String, Value)>,
}

impl RealSessionGuiHost {
    fn new(gui_only: bool, policy: ExternalPathPolicy) -> Self {
        let gui_policy = if gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled };
        let session = Session::new().with_gui_only(gui_policy).with_path_policy(policy);
        Self { session, gui_only, invoked_methods: Vec::new() }
    }

    fn resolve_menu_cmd(params: &Value) -> Result<String, EngineError> {
        if let Some(cid) = params.get("command_id").and_then(Value::as_str) {
            return Ok(cid.to_owned());
        }
        let path = match params.get("path") {
            Some(Value::String(s)) => s.to_owned(),
            Some(Value::Array(arr)) => arr.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("/"),
            _ => String::new(),
        };
        let norm: String = path.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect();
        match norm.as_str() {
            "fileopen" | "fileopenproject" | "open" => Ok(String::from("project.open")),
            "filenewfromtemplate" | "newfromtemplate" => Ok(String::from("project.instantiateTemplate")),
            "analyzerecalculateall" | "recalculateall" => Ok(String::from("project.recalculate")),
            "editpastelink" | "pastelink" => Ok(String::from("table.pasteLink")),
            "fileimporttidylongtable" | "importtidylongtable" => Ok(String::from("table.importTidyLong")),
            "filesave" => Ok(String::from("project.save")),
            _ => Err(EngineError::with_code(ErrorCode::NotFound, format!("unknown menu path `{path}`"))),
        }
    }
}

impl GuiSessionHost for RealSessionGuiHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        self.invoked_methods.push((method.to_owned(), params.clone()));
        match method {
            "ui.inspect" => {
                let sheets: Vec<Value> = self
                    .session
                    .document()
                    .data_sheets
                    .iter()
                    .map(|ds| json!({ "sheet_id": ds.id.as_str(), "title": ds.title, "archetype": &ds.archetype }))
                    .collect();
                Ok(json!({ "gui_only": self.gui_only, "data_sheets": sheets }))
            }
            "ui.invoke_menu" => {
                let cmd_id = Self::resolve_menu_cmd(&params)?;
                let extra = params.get("params").cloned().unwrap_or_else(|| json!({}));
                let outcome = self.session.execute_as_ui(&cmd_id, extra)?;
                Ok(json!({ "invoked": true, "command_id": cmd_id, "mutated": outcome.mutated, "payload": outcome.payload }))
            }
            other => Err(EngineError::with_code(ErrorCode::NotFound, format!("unsupported mock method `{other}`"))),
        }
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.gui_only = gui_only;
        self.session.set_gui_only(if gui_only { GuiOnlyPolicy::Strict } else { GuiOnlyPolicy::Disabled });
    }

    fn is_gui_only(&self) -> bool {
        self.gui_only
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(&self.session)
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(&mut self.session)
    }
}

fn build_poisoned_archive_b64(secret_csv_path: &std::path::Path) -> Result<String, String> {
    let mut builder = Session::new();
    builder
        .execute("sheet.createData", json!({ "sheet_id": "exfil", "title": "Exfil", "kind": "xy", "cols": 1, "rows": 2 }))
        .map_err(|err| format!("create exfil sheet: {err}"))?;
    let mut doc = builder.document().clone();
    doc.data_sheet_mut(&SheetId::new("exfil")).ok_or_else(|| String::from("missing exfil sheet"))?.linked_regions.push(LinkedDataRegion {
        id: String::from("ext-link"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile {
                path: secret_csv_path.to_string_lossy().into_owned(),
                delimiter: ',',
                decimal_separator: '.',
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(0, 0, 0)),
        provenance_summary: String::from("external"),
    });
    let bytes = save_project_bytes(&doc).map_err(|err| format!("save poisoned archive: {err}"))?;
    Ok(encode_base64(&bytes))
}

fn assert_mcp_error(result: &curvebook_mcp::ToolCallResult, expected_code: &str, context: &str) -> TestRes {
    if !result.is_error {
        return Err(format!("{context}: expected MCP error, got {:?}", result.structured_content));
    }
    let code = result.structured_content.as_ref().and_then(|v| v.get("code")).and_then(Value::as_str);
    if code != Some(expected_code) {
        return Err(format!("{context}: expected code `{expected_code}`, got {:?}", result.structured_content));
    }
    Ok(())
}

#[test]
fn fa_x1_01_mcp_gui_only_without_workdir_blocks_ui_invoke_menu_escalation() -> TestRes {
    let temp = TempDir::new("x1-mcp")?;
    let secret_csv = temp.path.join("secret_host.csv");
    fs::write(&secret_csv, "HeaderA,HeaderB\nCANARY_HOST_READ,31415\n").map_err(|err| format!("write secret csv: {err}"))?;
    let poisoned_b64 = build_poisoned_archive_b64(&secret_csv)?;

    let host = RealSessionGuiHost::new(true, ExternalPathPolicy::Unconstrained);
    let mut state = McpServerState::with_gui_host(Box::new(host));

    // Seed an in-memory table cleanly first via legitimate ui_invoke_menu.
    let seeded =
        call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "m1", "kind": "Column" } }))
            .map_err(|err| format!("seed column table failed: {err}"))?;
    if seeded.is_error {
        return Err(format!("expected legitimate sheet.createData to succeed: {:?}", seeded.structured_content));
    }

    // All FA-X1-01 escalation probes must fail closed with `gui_only_violation`.
    for (label, args) in [
        ("project.open bytes_base64", json!({ "command_id": "project.open", "params": { "bytes_base64": poisoned_b64 } })),
        ("File/Open... bytes_base64", json!({ "path": "File/Open...", "params": { "bytes_base64": poisoned_b64 } })),
        ("File > Open array bytesBase64", json!({ "path": ["File", "Open..."], "params": { "bytesBase64": poisoned_b64 } })),
        (
            "project.instantiateTemplate bytes_base64",
            json!({ "command_id": "project.instantiateTemplate", "params": { "bytes_base64": poisoned_b64 } }),
        ),
        ("File/New From Template... bytes_base64", json!({ "path": "File/New From Template...", "params": { "bytes_base64": poisoned_b64 } })),
        ("table.importPrism archive_base64", json!({ "command_id": "table.importPrism", "params": { "archive_base64": poisoned_b64 } })),
        ("table.importPzfx xml", json!({ "command_id": "table.importPzfx", "params": { "xml": "<XmlProject/>" } })),
        (
            "table.pasteLink ExternalDelimitedFile",
            json!({
                "command_id": "table.pasteLink",
                "params": {
                    "target_sheet_id": "m1",
                    "link_spec": {
                        "source": {
                            "ExternalDelimitedFile": {
                                "path": secret_csv.to_string_lossy(),
                                "delimiter": ",",
                                "decimal_separator": "."
                            }
                        }
                    }
                }
            }),
        ),
        (
            "Edit/Paste Link path",
            json!({ "path": ["Edit", "Paste Link"], "params": { "target_sheet_id": "m1", "path": secret_csv.to_string_lossy() } }),
        ),
        (
            "project.recalculate include_external_links true",
            json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }),
        ),
        (
            "Analyze/Recalculate All includeExternalLinks true",
            json!({ "path": ["Analyze", "Recalculate All"], "params": { "includeExternalLinks": true } }),
        ),
        ("table.importCsv link true", json!({ "command_id": "table.importCsv", "params": { "sheet_id": "m1", "csv": "A\n1\n", "link": true } })),
        ("path alias file", json!({ "command_id": "graph.insertImage", "params": { "file": secret_csv.to_string_lossy() } })),
        ("path alias out", json!({ "command_id": "graph.exportSvg", "params": { "out": secret_csv.to_string_lossy() } })),
        ("table.setCell blocked", json!({ "command_id": "table.setCell", "params": { "sheet_id": "m1", "row": 0, "col": 0, "value": 42.0 } })),
        ("nested ui.print.open submit_to_os", json!({ "command_id": "ui.print.open", "params": { "print": { "submit_to_os": true } } })),
    ] {
        let res = call_tool(&mut state, "ui_invoke_menu", args.clone()).map_err(|err| format!("call `{label}` err: {err}"))?;
        assert_mcp_error(&res, "gui_only_violation", label)?;

        // Also confirm JSON-RPC wire dispatch rejects identically.
        let rpc = dispatch_json_value(
            json!({ "jsonrpc": "2.0", "id": 99, "method": "tools/call", "params": { "name": "ui_invoke_menu", "arguments": args } }),
            &mut state,
        )
        .ok_or_else(|| format!("missing JSON-RPC response for `{label}`"))?;
        if rpc.pointer("/result/isError") != Some(&json!(true)) || rpc.pointer("/result/structuredContent/code") != Some(&json!("gui_only_violation"))
        {
            return Err(format!("JSON-RPC `{label}` did not fail closed with gui_only_violation: {rpc}"));
        }
    }

    // Secret must never appear in inspected sheets.
    let inspect = call_tool(&mut state, "ui_inspect", json!({ "scope": "all" })).map_err(|err| format!("ui_inspect failed: {err}"))?;
    let serialized = serde_json::to_string(&inspect).unwrap_or_default();
    if serialized.contains("CANARY_HOST_READ") || serialized.contains("31415") {
        return Err(format!("host secret exfiltrated into ui_inspect: {serialized}"));
    }

    // Legitimate in-memory `Analyze/Recalculate All` (no params) and `table.importTidyLong` inline CSV succeed.
    let recalc_ok = call_tool(&mut state, "ui_invoke_menu", json!({ "path": ["Analyze", "Recalculate All"] }))
        .map_err(|err| format!("Analyze/Recalculate All failed: {err}"))?;
    if recalc_ok.is_error {
        return Err(format!("in-memory Analyze/Recalculate All failed: {:?}", recalc_ok.structured_content));
    }
    let tidy_ok = call_tool(
        &mut state,
        "ui_invoke_menu",
        json!({
            "path": "File/Import/Tidy Long Table...",
            "params": { "sheet_id": "tidy-ok", "csv": "Group,Y\nCtrl,1.5\nDrug,4.5\n", "group_col": "Group", "value_col": "Y", "target_kind": "Column" }
        }),
    )
    .map_err(|err| format!("inline tidy import failed: {err}"))?;
    if tidy_ok.is_error {
        return Err(format!("inline File/Import/Tidy Long Table... failed: {:?}", tidy_ok.structured_content));
    }

    // Remote `--control-port --gui-only` attach: `ping` handshake succeeds, while dangerous `ui_invoke_menu`
    // probes fail closed BEFORE touching the remote control wire (`ui.invoke_menu` is never emitted).
    let saw_menu_wire = Arc::new(AtomicBool::new(false));
    let saw_flag = Arc::clone(&saw_menu_wire);
    let mut attached = McpServerState::with_control_port_and_gui_only(49404, true)
        .map_err(|err| format!("attach control port: {err}"))?
        .with_control_handler(move |method, _params| {
            if method == "ping" {
                return Ok(json!({ "pong": true, "gui_only": true }));
            }
            if method == "ui.invoke_menu" {
                saw_flag.store(true, Ordering::Relaxed);
            }
            Ok(json!({ "gui_only": true, "method": method }))
        });
    let wire_blocked =
        call_tool(&mut attached, "ui_invoke_menu", json!({ "command_id": "project.open", "params": { "bytes_base64": poisoned_b64 } }))
            .map_err(|err| format!("remote ui_invoke_menu error: {err}"))?;
    assert_mcp_error(&wire_blocked, "gui_only_violation", "control-port ui_invoke_menu block")?;
    let recalc_blocked =
        call_tool(&mut attached, "ui_invoke_menu", json!({ "path": ["Analyze", "Recalculate All"], "params": { "include_external_links": true } }))
            .map_err(|err| format!("remote recalc error: {err}"))?;
    assert_mcp_error(&recalc_blocked, "gui_only_violation", "control-port recalc block")?;
    if saw_menu_wire.load(Ordering::Relaxed) {
        return Err(String::from("blocked ui_invoke_menu payloads must not be forwarded over --control-port"));
    }
    Ok(())
}

#[test]
fn fa_x1_01_gui_only_with_workdir_permits_inside_save_and_rejects_outside_external_links() -> TestRes {
    let temp = TempDir::new("x1-workdir")?;
    let workdir = temp.path.join("wd");
    let outside = temp.path.join("outside");
    fs::create_dir_all(&workdir).map_err(|err| format!("mkdir workdir: {err}"))?;
    fs::create_dir_all(&outside).map_err(|err| format!("mkdir outside: {err}"))?;
    let outside_csv = outside.join("secret.csv");
    fs::write(&outside_csv, "A,B\nOUTSIDE_SECRET,77\n").map_err(|err| format!("write outside csv: {err}"))?;
    let poisoned_b64 = build_poisoned_archive_b64(&outside_csv)?;

    let host = RealSessionGuiHost::new(true, ExternalPathPolicy::Unconstrained);
    let mut state = McpServerState::with_gui_host(Box::new(host));
    state.set_workdir(Some(&workdir)).map_err(|err| format!("set_workdir: {err}"))?;

    // Confined archive load + recalculation MUST reject outside external link with `path_escape`.
    let open_ok = call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "project.open", "params": { "bytes_base64": poisoned_b64 } }))
        .map_err(|err| format!("open under workdir failed unexpectedly: {err}"))?;
    if open_ok.is_error {
        return Err(format!("open archive under --workdir should load doc before recalc check: {:?}", open_ok.structured_content));
    }
    let recalc_escape =
        call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "project.recalculate", "params": { "include_external_links": true } }))
            .map_err(|err| format!("recalc escape call err: {err}"))?;
    assert_mcp_error(&recalc_escape, "path_escape", "external link escape under --gui-only --workdir")?;

    // Valid save strictly inside workdir succeeds, while parent traversal fails with `path_escape`.
    let save_ok = call_tool(&mut state, "ui_invoke_menu", json!({ "path": ["File", "Save"], "params": { "path": "ok.curvebook" } }))
        .map_err(|err| format!("inside save call err: {err}"))?;
    if save_ok.is_error || !workdir.join("ok.curvebook").is_file() {
        return Err(format!("expected inside save to write ok.curvebook: {:?}", save_ok.structured_content));
    }
    let save_esc =
        call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "project.save", "params": { "path": "../outside/escape.curvebook" } }))
            .map_err(|err| format!("escape save call err: {err}"))?;
    assert_mcp_error(&save_esc, "path_escape", "project.save escape under --gui-only --workdir")?;

    Ok(())
}
