//! Integration tests for CLI v2 exit codes, structured `--json` errors, `commands`,
//! direct `<command-id>` schema-typed dispatch, `mcp --control-port` validation,
//! and a live stdio JSON-RPC smoke test against the compiled `lumencraft-cli` binary.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

type TestResult = Result<(), String>;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("{prefix}_{}_{}", std::process::id(), seq));
        fs::create_dir_all(&dir).map_err(|e| format!("create temp dir `{}`: {e}", dir.display()))?;
        Ok(Self { path: dir })
    }

    fn path(&self) -> &Path {
        &self.path
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

fn path_str(p: &Path) -> Result<&str, String> {
    p.to_str().ok_or_else(|| format!("non-UTF-8 path: {}", p.display()))
}

fn run_cli(args: &[&str], stdin_bytes: Option<&[u8]>) -> Result<Output, String> {
    let mut cmd = Command::new(cli_bin());
    cmd.args(args);
    if stdin_bytes.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn cli {args:?}: {e}"))?;
    if let Some(bytes) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(bytes).map_err(|e| format!("write cli stdin: {e}"))?;
    }
    child.wait_with_output().map_err(|e| format!("wait cli {args:?}: {e}"))
}

fn run_cli_ok_json(args: &[&str]) -> Result<Value, String> {
    let out = run_cli(args, None)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.code() != Some(0) {
        return Err(format!("expected exit 0 for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
    }
    serde_json::from_str(&stdout).map_err(|e| format!("parse stdout JSON for {args:?}: {e}\nstdout:\n{stdout}"))
}

fn assert_structured_json_error(args: &[&str], expected_exit: i32, expected_code: &str, expected_category: &str) -> Result<Value, String> {
    let out = run_cli(args, None)?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.code() != Some(expected_exit) {
        return Err(format!("expected exit {expected_exit} for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
    }
    let val: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("parse error JSON for {args:?}: {e}\nstdout:\n{stdout}\nstderr:\n{stderr}"))?;
    let err = val.get("error").ok_or_else(|| format!("missing `error` envelope in {val}"))?;
    assert_eq!(err.get("code").and_then(Value::as_str), Some(expected_code), "unexpected error.code in {val}");
    assert_eq!(err.get("category").and_then(Value::as_str), Some(expected_category), "unexpected error.category in {val}");
    assert!(err.get("message").and_then(Value::as_str).is_some_and(|m| !m.is_empty()), "missing error.message in {val}");
    assert!(err.get("retryable").and_then(Value::as_bool).is_some(), "missing boolean error.retryable in {val}");
    assert!(err.get("remediation").is_some(), "missing error.remediation key in {val}");
    assert!(err.get("details").is_some(), "missing error.details key in {val}");
    Ok(val)
}

fn frame_content_length(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

fn parse_content_length_frames(mut raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut frames = Vec::new();
    while !raw.is_empty() {
        let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
            return Err(format!("truncated Content-Length header in remaining {} byte(s)", raw.len()));
        };
        let header = std::str::from_utf8(&raw[..header_end]).map_err(|e| format!("header UTF-8: {e}"))?;
        let mut content_len: Option<usize> = None;
        for line in header.split("\r\n") {
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                content_len = Some(v.trim().parse::<usize>().map_err(|e| format!("invalid Content-Length `{v}`: {e}"))?);
            }
        }
        let len = content_len.ok_or_else(|| format!("missing Content-Length header in `{header}`"))?;
        let body_start = header_end + 4;
        let body_end = body_start + len;
        if raw.len() < body_end {
            return Err(format!("truncated Content-Length body: expected {len} bytes, got {}", raw.len() - body_start));
        }
        let val: Value = serde_json::from_slice(&raw[body_start..body_end]).map_err(|e| format!("parse framed JSON: {e}"))?;
        frames.push(val);
        raw = &raw[body_end..];
    }
    Ok(frames)
}

#[test]
fn exit_codes_and_json_structured_errors_work_end_to_end() -> TestResult {
    let work = TempDir::new("lc_cli_v2_err")?;

    // Exit 0
    let ok_out = run_cli(&["--version"], None)?;
    assert_eq!(ok_out.status.code(), Some(0));

    // Exit 1 with --json -> structured io_error
    let missing_file = work.path().join("does-not-exist.lumencraft");
    let err1 = assert_structured_json_error(&["info", path_str(&missing_file)?, "--json"], 1, "io_error", "io")?;
    assert_eq!(err1["error"]["retryable"], json!(true));

    // Exit 1 without --json -> backward-compatible plain stderr text, empty stdout
    let plain_err1 = run_cli(&["info", path_str(&missing_file)?], None)?;
    assert_eq!(plain_err1.status.code(), Some(1));
    assert!(plain_err1.stdout.is_empty());
    let stderr1 = String::from_utf8_lossy(&plain_err1.stderr);
    assert!(stderr1.contains("lumencraft-cli:"), "expected plain stderr banner, got `{stderr1}`");

    // Exit 1 from engine command preserving structured envelope metadata in --json
    let engine_err = assert_structured_json_error(&["sheet.delete", "--sheet-id", "no_such_sheet", "--json"], 1, "not_found", "state")?;
    assert!(engine_err["error"].get("remediation").is_some());
    assert!(engine_err["error"].get("details").is_some());
    assert!(engine_err["error"]["message"].as_str().is_some_and(|m| m.contains("no_such_sheet")));

    // Exit 2 usage error with --json includes actionable remediation
    let usage_err = assert_structured_json_error(&["no-such-command", "--json"], 2, "bad_input", "input")?;
    assert!(usage_err["error"]["remediation"].as_str().is_some());

    // Exit 2 usage error without --json -> backward-compatible stderr text
    let plain_err2 = run_cli(&["no-such-command"], None)?;
    assert_eq!(plain_err2.status.code(), Some(2));
    assert!(plain_err2.stdout.is_empty());
    assert!(String::from_utf8_lossy(&plain_err2.stderr).contains("usage error:"));

    // Exit 2 workdir path escape with --json
    let _ = assert_structured_json_error(
        &["sheet.createData", "--sheet-id", "d1", "--workdir", path_str(work.path())?, "--out", "../outside.lumencraft", "--json"],
        2,
        "path_escape",
        "policy",
    )?;
    assert!(!work.path().join("../outside.lumencraft").exists());

    Ok(())
}

#[test]
fn commands_discovery_lists_plain_grouped_and_json_metadata() -> TestResult {
    let plain = run_cli(&["commands"], None)?;
    assert_eq!(plain.status.code(), Some(0));
    let plain_stdout = String::from_utf8_lossy(&plain.stdout);
    assert!(plain_stdout.contains("sheet.createData"));
    assert!(plain_stdout.contains("project.inspect"));

    let all_json = run_cli_ok_json(&["commands", "--json"])?;
    assert_eq!(all_json["ok"], json!(true));
    let cmds = all_json["commands"].as_array().ok_or("commands must be an array")?;
    assert!(cmds.len() >= 90, "expected >= 90 commands, got {}", cmds.len());
    for entry in cmds.iter().take(10) {
        assert!(entry.get("group").and_then(Value::as_str).is_some());
        assert!(entry.get("id").and_then(Value::as_str).is_some());
        assert!(entry.get("label").and_then(Value::as_str).is_some());
        assert!(entry.get("shortcut").and_then(Value::as_str).is_some());
        assert!(entry.get("effects").and_then(Value::as_array).is_some());
        assert!(entry.get("summary").and_then(Value::as_str).is_some());
        assert!(entry.get("beginner").and_then(Value::as_str).is_some());
        assert!(entry.get("input_schema").and_then(Value::as_object).is_some());
    }

    let sheet_json = run_cli_ok_json(&["commands", "--group", "sheet", "--json"])?;
    assert_eq!(sheet_json["group"], json!("sheet"));
    let sheet_cmds = sheet_json["commands"].as_array().ok_or("sheet commands array")?;
    assert!(!sheet_cmds.is_empty());
    assert!(sheet_cmds.iter().all(|c| c["group"] == json!("sheet")));

    let _ = assert_structured_json_error(&["commands", "--group", "unknown_group", "--json"], 2, "bad_input", "input")?;
    Ok(())
}

#[test]
fn direct_command_execution_supports_schema_typed_flags_and_atomic_save() -> TestResult {
    let work = TempDir::new("lc_cli_v2_direct")?;
    let wd_str = path_str(work.path())?;

    let created = run_cli_ok_json(&[
        "sheet.createData",
        "--sheet-id",
        "dose_1",
        "--title",
        "Dose Response",
        "--archetype",
        "XY",
        "--rows",
        "4",
        "--cols",
        "2",
        "--auto-graph",
        "--variables",
        "Concentration,Response",
        "--x-mode",
        "numbers",
        "--workdir",
        wd_str,
        "--out",
        "nested/study.lumencraft",
        "--json",
    ])?;
    assert_eq!(created["ok"], json!(true));
    assert_eq!(created["command_id"], json!("sheet.createData"));
    assert_eq!(created["mutated"], json!(true));
    assert_eq!(created["result"]["sheet_id"], json!("dose_1"));

    let saved_file = work.path().join("nested/study.lumencraft");
    assert!(saved_file.is_file(), "direct command --out must persist archive atomically");

    let edited = run_cli_ok_json(&[
        "table.setCell",
        "--workdir",
        wd_str,
        "--in",
        "nested/study.lumencraft",
        "--out",
        "nested/study.lumencraft",
        "--sheet_id",
        "dose_1",
        "--row",
        "0",
        "--col",
        "0",
        "--value",
        "12.5",
        "--json",
    ])?;
    assert_eq!(edited["ok"], json!(true));
    assert_eq!(edited["mutated"], json!(true));

    let info = run_cli_ok_json(&["info", path_str(&saved_file)?, "--json"])?;
    assert_eq!(info["tables"][0]["id"], json!("dose_1"));
    assert_eq!(info["graphs"].as_array().map(Vec::len), Some(1));

    // Invalid typed flag value returns usage exit 2
    let _ = assert_structured_json_error(&["sheet.createData", "--rows", "abc", "--json"], 2, "bad_input", "input")?;
    let _ = assert_structured_json_error(&["sheet.createData", "--no-such-flag", "--json"], 2, "bad_input", "input")?;
    Ok(())
}

#[test]
fn mcp_control_port_and_token_flag_validation_reject_invalid_and_unexpected_args() -> TestResult {
    for args in [
        &["mcp", "--control-port", "0"][..],
        &["mcp", "--control-port=0"][..],
        &["mcp", "--control-port", "65536"][..],
        &["mcp", "--control-port", "not-a-port"][..],
        &["mcp", "--control-port"][..],
        &["mcp", "--control-token", ""][..],
        &["mcp", "--control-token="][..],
        &["mcp", "--control-token", "has space"][..],
        &["mcp", "--control-token"][..],
        &["mcp", "--unexpected-flag"][..],
        &["mcp", "extra-arg"][..],
    ] {
        let out = run_cli(args, None)?;
        assert_eq!(out.status.code(), Some(2), "expected exit 2 for {args:?}, got {:?}", out.status.code());
    }

    let _ = assert_structured_json_error(&["mcp", "--control-port", "0", "--json"], 2, "bad_input", "input")?;
    let _ = assert_structured_json_error(&["mcp", "--control-token", "bad token", "--json"], 2, "bad_input", "input")?;

    let help = run_cli(&["--help"], None)?;
    assert!(help.status.success());
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("mcp [--control-port <port>] [--gui-only] [--workdir <dir>] [--control-token <token>]"));
    Ok(())
}

#[test]
fn live_mcp_stdio_recovers_after_oversized_content_length_request() -> TestResult {
    let mut script = Vec::new();
    script.extend(frame_content_length(
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"cli-drain","version":"0.1.0"}}}"#,
    ));
    let pad = "x".repeat(16 * 1024 * 1024 + 512);
    let huge = format!(r#"{{"jsonrpc":"2.0","id":"huge-1","method":"ping","params":{{"blob":"{pad}"}}}}"#);
    script.extend(frame_content_length(&huge));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":2,"method":"ping","params":{}}"#));

    let out = run_cli(&["mcp"], Some(&script))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "expected clean exit 0 after bounded frame drain, stderr:\n{stderr}");

    let responses = parse_content_length_frames(&out.stdout)?;
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], json!(0));
    assert_eq!(responses[1]["id"], json!("huge-1"));
    assert_eq!(responses[1]["error"]["code"], json!(-32600));
    assert_eq!(responses[2]["id"], json!(2));
    assert_eq!(responses[2]["result"], json!({}));
    Ok(())
}

#[test]
fn live_mcp_stdio_smoke_test_initialize_tools_prompts_resources_and_call() -> TestResult {
    let mut script = Vec::new();
    script.extend(frame_content_length(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"cli-smoke","version":"0.1.0"}}}"#,
    ));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":3,"method":"prompts/list","params":{}}"#));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":4,"method":"resources/list","params":{}}"#));
    script.extend(frame_content_length(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"doc_inspect","arguments":{}}}"#));

    let out = run_cli(&["mcp"], Some(&script))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "expected clean EOF exit 0 from `lumencraft-cli mcp`, stderr:\n{stderr}");

    let responses = parse_content_length_frames(&out.stdout)?;
    assert_eq!(responses.len(), 5, "expected 5 JSON-RPC responses (notification produces no reply), got {responses:?}");

    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], json!("lumencraft"));

    assert_eq!(responses[1]["id"], json!(2));
    let tools = responses[1]["result"]["tools"].as_array().ok_or("tools/list must return tools array")?;
    assert!(tools.iter().any(|t| t["name"] == json!("doc_inspect")), "expected `doc_inspect` in tools/list");

    assert_eq!(responses[2]["id"], json!(3));
    let prompts = responses[2]["result"]["prompts"].as_array().ok_or("prompts/list must return prompts array")?;
    assert!(!prompts.is_empty(), "expected non-empty prompts list");

    assert_eq!(responses[3]["id"], json!(4));
    assert!(responses[3]["result"]["resources"].is_array());

    assert_eq!(responses[4]["id"], json!(5));
    assert!(responses[4].get("error").is_none(), "doc_inspect tools/call returned error: {}", responses[4]);
    assert_ne!(responses[4]["result"]["isError"], json!(true));
    Ok(())
}
