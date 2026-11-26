//! Wave 1 security regression suite for `FA-X1-01` across the `curvebook-mcp` `--workdir`
//! confinement boundary on a `GuiSessionHost`.
use std::fs;

use curvebook_doc::{
    CellCoord, ErrorCode, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, LinkedDataRegion, SheetCellRange, SheetId,
};
use curvebook_engine::{EngineError, ExternalPathPolicy, Session};
use curvebook_format::save_project_bytes;
use curvebook_mcp::{GuiSessionHost, McpServerState, call_tool};
use serde_json::{Value, json};

use super::common::{TempDir, encode_base64};

type TestRes = Result<(), String>;

/// Real `Session`-backed `GuiSessionHost` modeling `ui.invoke_menu` -> `Session::execute_as_ui`
/// (`Caller::Ui`) so `--workdir` confinement is exercised directly against `Session`.
struct RealSessionGuiHost {
    session: Session,
    invoked_methods: Vec<(String, Value)>,
}

impl RealSessionGuiHost {
    fn new(policy: ExternalPathPolicy) -> Self {
        let session = Session::new().with_path_policy(policy);
        Self { session, invoked_methods: Vec::new() }
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
                Ok(json!({ "data_sheets": sheets }))
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
fn fa_x1_01_gui_host_with_workdir_permits_inside_save_and_rejects_outside_external_links() -> TestRes {
    let temp = TempDir::new("x1-workdir")?;
    let workdir = temp.path.join("wd");
    let outside = temp.path.join("outside");
    fs::create_dir_all(&workdir).map_err(|err| format!("mkdir workdir: {err}"))?;
    fs::create_dir_all(&outside).map_err(|err| format!("mkdir outside: {err}"))?;
    let outside_csv = outside.join("secret.csv");
    fs::write(&outside_csv, "A,B\nOUTSIDE_SECRET,77\n").map_err(|err| format!("write outside csv: {err}"))?;
    let poisoned_b64 = build_poisoned_archive_b64(&outside_csv)?;

    let host = RealSessionGuiHost::new(ExternalPathPolicy::Unconstrained);
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
    assert_mcp_error(&recalc_escape, "path_escape", "external link escape under --workdir")?;

    // Valid save strictly inside workdir succeeds, while parent traversal fails with `path_escape`.
    let save_ok = call_tool(&mut state, "ui_invoke_menu", json!({ "path": ["File", "Save"], "params": { "path": "ok.curvebook" } }))
        .map_err(|err| format!("inside save call err: {err}"))?;
    if save_ok.is_error || !workdir.join("ok.curvebook").is_file() {
        return Err(format!("expected inside save to write ok.curvebook: {:?}", save_ok.structured_content));
    }
    let save_esc =
        call_tool(&mut state, "ui_invoke_menu", json!({ "command_id": "project.save", "params": { "path": "../outside/escape.curvebook" } }))
            .map_err(|err| format!("escape save call err: {err}"))?;
    assert_mcp_error(&save_esc, "path_escape", "project.save escape under --workdir")?;

    Ok(())
}
