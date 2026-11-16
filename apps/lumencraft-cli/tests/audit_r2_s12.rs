//! Permanent Round 2 (`R0.19`) Slice `S12` audit regression suite for `lumencraft-cli`.
//!
//! Covers:
//! - `R2-S12-01`: `--control-port` CLI/MCP handshake & bidirectional `--gui-only` policy enforcement.
//! - `R2-S12-02`: `is_unpacked_project_path`, `convert`, and `<cmd> --out` never classify `.` or
//!   plain directories without `.lumencraft.d` suffix as unpacked projects or delete `<dir>/sheets/`.
//! - `R2-S12-03`: In-process `lumencraft-cli mcp --gui-only` `ui_invoke_menu` works on parameterless
//!   non-`ui.*` menu items without `expected object, got null` and blocks `ui.result.copyMethodsReport`.
//! - `R2-S12-05`: `cmd_convert` fails closed on invalid `.lctemplate` archive kind without falling back
//!   to `open_archive_bytes`.
//! - `R2-S12-07`: Hosted `UiApp` + `McpServerState::with_gui_host` shares a single `Session` & `resources/*`.
//! - `R2-S12-08`: Direct CLI `PropKind::Integer` parses full `u64` seeds `> i64::MAX`,
//!   `ensure_param_output_parents` ignores read-only commands, and `--out` export runs inside `run_guarded`.
//! - `R2-S12-09`: In-process `--gui-only` MCP serves GUI-only prompts and preserves `"wheel"` action.
#![forbid(unsafe_code)]

use std::fs;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use lumencraft_doc::{ColumnDesignHint, ColumnTable, DataSheet, ProjectDocument, SheetId, TableArchetype};
use lumencraft_engine::{EngineError, Session};
use lumencraft_format::save_project_file;
use lumencraft_mcp::{GUI_ONLY_SERVER_INSTRUCTIONS, GuiSessionHost, McpServerState, call_tool, dispatch_json_value};
use lumencraft_ui_egui::{ControlAuth, UiApp, dispatch_gui_action, serve_control_stream_with_quit};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TEMP_SEQ: AtomicUsize = AtomicUsize::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("lc-r2-s12-cli-{label}-{}-{seq}", std::process::id()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir)?;
        Ok(Self { path: dir })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_lumencraft-cli")
}

struct McpChild {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl McpChild {
    fn spawn(args: &[&str]) -> Result<Self, String> {
        let mut child = Command::new(cli_bin())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn lumencraft-cli {args:?}: {e}"))?;
        let stdin = child.stdin.take().ok_or_else(|| String::from("missing child stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| String::from("missing child stdout"))?;
        Ok(Self { child, stdin: Some(stdin), stdout: BufReader::new(stdout), next_id: 1 })
    }

    fn send_json(&mut self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_vec(msg).map_err(|e| format!("serialize JSON: {e}"))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        let stdin = self.stdin.as_mut().ok_or_else(|| String::from("child stdin closed"))?;
        stdin.write_all(header.as_bytes()).map_err(|e| format!("write header: {e}"))?;
        stdin.write_all(&body).map_err(|e| format!("write body: {e}"))?;
        stdin.flush().map_err(|e| format!("flush stdin: {e}"))?;
        Ok(())
    }

    fn read_json(&mut self) -> Result<Value, String> {
        let mut content_length = None::<usize>;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).map_err(|e| format!("read header: {e}"))?;
            if n == 0 {
                return Err(String::from("unexpected EOF from MCP child stdout"));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some(len_str) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(len_str.trim().parse::<usize>().map_err(|e| format!("bad Content-Length `{trimmed}`: {e}"))?);
            }
        }
        let len = content_length.ok_or_else(|| String::from("missing Content-Length header"))?;
        let mut buf = vec![0_u8; len];
        self.stdout.read_exact(&mut buf).map_err(|e| format!("read body: {e}"))?;
        serde_json::from_slice(&buf).map_err(|e| format!("parse JSON body: {e}"))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send_json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        self.read_json()
    }

    fn initialize(&mut self) -> Result<Value, String> {
        let init = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "audit-r2-s12-cli", "version": "0.1.0" }
            }),
        )?;
        self.send_json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} }))?;
        Ok(init)
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.request("tools/call", json!({ "name": name, "arguments": arguments }))
    }

    fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| format!("wait child: {e}"))?;
        if !status.success() {
            return Err(format!("MCP child exited non-zero: {status}"));
        }
        Ok(())
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct SharedUiAppHost {
    app: UiApp,
}

impl SharedUiAppHost {
    fn new(gui_only: bool) -> Self {
        let mut app = UiApp::new();
        app.set_gui_only(gui_only);
        Self { app }
    }
}

impl GuiSessionHost for SharedUiAppHost {
    fn call_gui(&mut self, method: &str, params: Value) -> Result<Value, EngineError> {
        dispatch_gui_action(&mut self.app, method, params)
    }

    fn set_gui_only(&mut self, gui_only: bool) {
        self.app.set_gui_only(gui_only);
    }

    fn is_gui_only(&self) -> bool {
        self.app.is_gui_only()
    }

    fn hosted_session(&self) -> Option<&Session> {
        Some(self.app.session())
    }

    fn hosted_session_mut(&mut self) -> Option<&mut Session> {
        Some(self.app.session_mut())
    }

    fn sync_after_session_mutation(&mut self) {
        self.app.sync_views();
    }
}

#[test]
fn r2_s12_01_cli_control_port_handshake_enforces_gui_only_in_both_directions() -> TestResult {
    use std::io::Cursor;

    // In-process control stream test (works even inside strict sandbox without TCP loopback).
    let token = "audit-r2-s12-token";
    let auth = ControlAuth::from_token(token)?;
    let mut locked_app = UiApp::new();
    locked_app.set_gui_only(true);
    let auth_clone = auth.clone();
    let mut client = McpServerState::with_control_port(49410)?.with_control_token(token)?.with_control_handler(move |method, params| {
        let auth_line =
            serde_json::to_string(&json!({ "id": 0, "method": "auth", "params": { "token": auth_clone.as_str() } })).map_err(|e| e.to_string())?;
        let req_line = serde_json::to_string(&json!({ "id": "mcp-1", "method": method, "params": params })).map_err(|e| e.to_string())?;
        let input = format!("{auth_line}\n{req_line}\n");
        let mut reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut out = Vec::new();
        let _ = serve_control_stream_with_quit(&mut locked_app, &mut reader, &mut out, &auth_clone).map_err(|e| e.to_string())?;
        let text = String::from_utf8(out).map_err(|e| e.to_string())?;
        let reply_line = text.lines().nth(1).ok_or_else(|| format!("missing control reply in `{text}`"))?;
        let parsed: Value = serde_json::from_str(reply_line).map_err(|e| e.to_string())?;
        if parsed["ok"] == json!(true) { Ok(parsed["result"].clone()) } else { Err(parsed["error"].as_str().unwrap_or("control error").to_owned()) }
    });

    let init = dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }), &mut client)
        .ok_or("missing initialize reply")?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));
    assert!(client.is_gui_only());

    // Live TCP loopback binary test when loopback bind+connect is permitted in the active environment.
    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let addr = listener.local_addr()?;
    if let Err(err) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
        if err.kind() == ErrorKind::PermissionDenied {
            return Ok(());
        }
        return Err(err.into());
    }
    let _ = listener.accept();
    listener.set_nonblocking(true)?;
    let port = addr.port();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let tcp_auth = ControlAuth::from_token(token)?;
    let server = thread::spawn(move || -> Result<(), String> {
        let mut app = UiApp::new();
        app.set_gui_only(true);
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                return Ok(());
            }
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
                    let mut reader = BufReader::new(stream);
                    let quit = serve_control_stream_with_quit(&mut app, &mut reader, &mut writer, &tcp_auth).map_err(|e| e.to_string())?;
                    if quit {
                        return Ok(());
                    }
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    });

    // Spawn CLI MCP WITHOUT `--gui-only` pointing at the `--gui-only` desktop control port.
    let port_str = port.to_string();
    let mut attached = McpChild::spawn(&["mcp", "--control-port", &port_str, "--control-token", token])?;
    let tcp_init = attached.initialize()?;
    assert_eq!(tcp_init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));
    let listed = attached.request("tools/list", json!({}))?;
    assert_eq!(listed["result"]["tools"].as_array().map(Vec::len), Some(8));
    let blocked = attached.call_tool("doc_inspect", json!({}))?;
    assert_eq!(blocked["result"]["isError"], json!(true));
    assert_eq!(blocked["result"]["structuredContent"]["code"], json!("gui_only_violation"));

    attached.finish()?;
    stop.store(true, Ordering::Relaxed);
    server.join().map_err(|_| "TCP server thread panicked")??;
    Ok(())
}

#[test]
fn r2_s12_02_unpacked_dir_check_never_classifies_dot_or_plain_dirs() -> TestResult {
    let dir = TempDir::new("unpacked-dir")?;
    let ws = dir.path.join("workspace");
    let sheets_dir = ws.join("sheets");
    fs::create_dir_all(&sheets_dir)?;
    let input_csv = dir.path.join("input.csv");
    fs::write(&input_csv, "X,Y\n1,10\n2,20\n")?;
    let assay_file = sheets_dir.join("raw_assay.csv");
    let notes_file = sheets_dir.join("notes.txt");
    fs::write(&assay_file, "A,B\n1,2\n")?;
    fs::write(&notes_file, "keep me\n")?;

    // `lumencraft-cli convert input.csv ./workspace` must fail cleanly without touching `workspace/sheets/`.
    let convert_out: Output = Command::new(cli_bin()).args(["convert", &input_csv.display().to_string(), &ws.display().to_string()]).output()?;
    assert!(!convert_out.status.success(), "convert into non-.lumencraft.d dir must fail");
    assert!(!ws.join("manifest.json").exists(), "manifest.json must not be created in plain dir");
    assert!(assay_file.is_file(), "sheets/raw_assay.csv must never be deleted");
    assert!(notes_file.is_file(), "sheets/notes.txt must never be deleted");

    // `lumencraft-cli project.new --out .` inside `workspace` must fail cleanly without deleting `sheets/`.
    let direct_out: Output = Command::new(cli_bin()).current_dir(&ws).args(["project.new", "--out", "."]).output()?;
    assert!(!direct_out.status.success(), "project.new --out . must fail");
    assert!(!ws.join("manifest.json").exists(), "manifest.json must not be created in `.`");
    assert!(assay_file.is_file(), "sheets/raw_assay.csv must survive `project.new --out .`");

    // Explicit `.lumencraft.d` suffix still succeeds cleanly.
    let valid_dir = dir.path.join("study.lumencraft.d");
    let ok_out = Command::new(cli_bin()).args(["convert", &input_csv.display().to_string(), &valid_dir.display().to_string()]).output()?;
    assert!(ok_out.status.success(), "convert to .lumencraft.d failed: {}", String::from_utf8_lossy(&ok_out.stderr));
    assert!(valid_dir.join("manifest.json").is_file());
    Ok(())
}

#[test]
fn r2_s12_03_and_09_in_process_gui_only_menu_prompts_and_wheel() -> TestResult {
    let mut mcp = McpChild::spawn(&["mcp", "--gui-only"])?;
    let init = mcp.initialize()?;
    assert_eq!(init["result"]["instructions"], json!(GUI_ONLY_SERVER_INSTRUCTIONS));

    // Create a sheet first via `ui_invoke_menu` so standard non-`ui.*` menu items are enabled.
    let new_sheet = mcp.call_tool("ui_invoke_menu", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "m1", "kind": "Column" } }))?;
    assert_ne!(new_sheet["result"]["isError"], json!(true), "sheet.createData failed: {new_sheet}");

    // Standard non-`ui.*` zero-arg menu item WITHOUT `params` must pass root object `{}` (never null).
    let recalc = mcp.call_tool("ui_invoke_menu", json!({ "path": ["Analyze", "Recalculate All"] }))?;
    assert_ne!(recalc["result"]["isError"], json!(true), "Recalculate All without params failed: {recalc}");

    let undo = mcp.call_tool("ui_invoke_menu", json!({ "path": ["Edit", "Undo"] }))?;
    assert_ne!(undo["result"]["isError"], json!(true), "Edit > Undo without params failed: {undo}");

    // OS clipboard helper blocked under `--gui-only`.
    let copy_blocked = mcp.call_tool("ui_invoke_menu", json!({ "command_id": "ui.result.copyMethodsReport" }))?;
    assert_eq!(copy_blocked["result"]["isError"], json!(true));
    assert_eq!(copy_blocked["result"]["structuredContent"]["code"], json!("gui_only_violation"));

    // `prompts/list` + `prompts/get` never expose headless-only tools/resources under `--gui-only`.
    let plist = mcp.request("prompts/list", json!({}))?;
    let prompts = plist["result"]["prompts"].as_array().ok_or("prompts/list missing array")?;
    for entry in prompts {
        let name = entry["name"].as_str().ok_or("missing prompt name")?;
        let got = mcp.request("prompts/get", json!({ "name": name }))?;
        let text = got["result"]["messages"][0]["content"]["text"].as_str().unwrap_or("");
        for forbidden in ["doc_inspect", "enable_toolset", "command_run", "render_preview", "sheet://", "result://"] {
            assert!(!text.contains(forbidden), "CLI --gui-only prompt `{name}` contains `{forbidden}`:\n{text}");
        }
    }

    // `ui_pointer_gesture { action: "wheel" }` returns `"action": "wheel"` matching `--control-port`.
    let wheel = mcp.call_tool("ui_pointer_gesture", json!({ "action": "wheel", "x": 200.0, "y": 200.0, "delta_y": -120.0 }))?;
    assert_ne!(wheel["result"]["isError"], json!(true), "wheel failed: {wheel}");
    assert_eq!(wheel["result"]["structuredContent"]["action"], json!("wheel"));

    mcp.finish()?;
    Ok(())
}

#[test]
fn r2_s12_05_cmd_convert_fails_closed_on_wrong_lctemplate_archive_kind() -> TestResult {
    let dir = TempDir::new("lctemplate-kind")?;
    let mut doc = ProjectDocument::new("Archive With Data").map_err(|e| e.to_string())?;
    let ds = DataSheet::new("d1", "Table", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?;
    doc.data_sheets.push(ds);

    // Save a Project archive, then copy its bytes to a `.lctemplate` extension.
    let real_archive = dir.path.join("real.lumencraft");
    save_project_file(&real_archive, &doc)?;
    let fake_tpl = dir.path.join("spoofed.lctemplate");
    fs::copy(&real_archive, &fake_tpl)?;

    let out_csv = dir.path.join("out.csv");
    let res = Command::new(cli_bin()).args(["convert", &fake_tpl.display().to_string(), &out_csv.display().to_string()]).output()?;
    assert!(!res.status.success(), "converting a Project archive disguised as .lctemplate must fail closed");
    assert!(!out_csv.exists(), "must not emit output CSV after archive-kind mismatch");
    Ok(())
}

#[test]
fn r2_s12_07_shared_ui_app_host_and_cli_resources_parity() -> TestResult {
    let dir = TempDir::new("uiapp-host")?;
    let mut state = McpServerState::with_gui_host(Box::new(SharedUiAppHost::new(false)));
    state.set_workdir(Some(&dir.path))?;

    let created =
        call_tool(&mut state, "command_run", json!({ "command_id": "sheet.createData", "params": { "sheet_id": "gui-shared", "kind": "Xy" } }))
            .map_err(|e| format!("{e}"))?;
    assert!(!created.is_error);
    let insp = call_tool(&mut state, "ui_inspect", json!({ "scope": "navigator" })).map_err(|e| format!("{e}"))?;
    assert!(!insp.is_error);
    let insp_sc = insp.structured_content.ok_or("missing structuredContent")?;
    let nav = insp_sc["navigator_sheets"].as_array().ok_or("missing navigator_sheets")?;
    assert!(nav.iter().any(|s| s["sheet_id"] == json!("gui-shared")), "UiApp did not see shared mutation: {insp_sc}");
    assert!(state.session().document().data_sheet(&SheetId::new("gui-shared")).is_some());

    let res =
        dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": { "uri": "sheet://gui-shared" } }), &mut state)
            .ok_or("missing resources/read reply")?;
    assert!(res.get("error").is_none(), "resources/read failed on UiApp host: {res}");
    Ok(())
}

#[test]
fn r2_s12_08_cli_direct_u64_seed_readonly_workdir_parents_and_out_guard() -> TestResult {
    let dir = TempDir::new("direct-cmd")?;
    let wd = dir.path.join("wd");
    fs::create_dir_all(&wd)?;
    let wd_str = wd.display().to_string();

    // 1. Valid `u64` seed > `i64::MAX` (`18446744073709551615 == u64::MAX`) succeeds on direct CLI command.
    let seed_out = Command::new(cli_bin())
        .args(["table.randomizeGroups", "--groups", "[\"Ctrl\",\"Tx\"]", "--subject-count", "6", "--seed", "18446744073709551615", "--json"])
        .output()?;
    assert!(seed_out.status.success(), "table.randomizeGroups with u64::MAX seed failed: stderr={}", String::from_utf8_lossy(&seed_out.stderr));
    let seed_json: Value = serde_json::from_slice(&seed_out.stdout)?;
    assert_eq!(seed_json["result"]["seed"], json!(u64::MAX));

    // 2. Non-writing `--workdir` command must NOT create missing parent directories from path params.
    let read_only_err =
        Command::new(cli_bin()).args(["project.open", "--path", "missing_parent/study.lumencraft", "--workdir", &wd_str, "--json"]).output()?;
    assert!(!read_only_err.status.success());
    assert!(!wd.join("missing_parent").exists(), "read-only project.open under --workdir must never create missing_parent/");

    // 3. Direct `--out` export runs cleanly and produces valid archive inside `--workdir`.
    let out_res = Command::new(cli_bin())
        .args(["sheet.createData", "--kind", "Column", "--workdir", &wd_str, "--out", "nested_out/direct.lumencraft", "--json"])
        .output()?;
    assert!(out_res.status.success(), "direct --out failed: {}", String::from_utf8_lossy(&out_res.stderr));
    assert!(wd.join("nested_out/direct.lumencraft").is_file());
    Ok(())
}
