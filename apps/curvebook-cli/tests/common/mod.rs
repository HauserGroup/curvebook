#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Shared test harness (`CliHarness`, `TempDir`, `TempDirPair`, `McpChild`, Base64 via
//! `curvebook_platform::base64`, `assert_close`, and R73 GUI-only workflow helpers) for
//! `apps/curvebook-cli` integration tests.

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct TempDir {
    pub dir: PathBuf,
}

pub type TempWorkdir = TempDir;

impl TempDir {
    pub fn new(prefix: &str) -> Result<Self, String> {
        let seq = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb_{prefix}_{}_{}", std::process::id(), seq));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| format!("create temp dir {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    pub fn child(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    pub fn path_str(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

pub struct TempDirPair {
    pub root: PathBuf,
    pub workdir: PathBuf,
    pub outside: PathBuf,
}

impl TempDirPair {
    pub fn new(label: &str) -> Self {
        let seq = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-cli-policy-{label}-{}-{seq}", std::process::id()));
        let workdir = root.join("workdir");
        let outside = root.join("outside");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&workdir).expect("create workdir");
        fs::create_dir_all(&outside).expect("create outside");
        Self { root, workdir, outside }
    }
}

impl Drop for TempDirPair {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct CliHarness;

impl CliHarness {
    pub fn bin() -> &'static str {
        env!("CARGO_BIN_EXE_curvebook-cli")
    }

    pub fn path_str(p: &Path) -> Result<&str, String> {
        p.to_str().ok_or_else(|| format!("non-UTF-8 path: {}", p.display()))
    }

    pub fn run(args: &[&str]) -> Result<Output, String> {
        Self::run_with_stdin(args, None)
    }

    pub fn run_with_stdin(args: &[&str], stdin_bytes: Option<&[u8]>) -> Result<Output, String> {
        let mut cmd = Command::new(Self::bin());
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

    pub fn run_bounded(args: &[String], timeout: Duration) -> Result<Output, String> {
        let mut child = Command::new(Self::bin())
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("spawn curvebook-cli failed: {err}"))?;

        let mut stdout = child.stdout.take().ok_or_else(|| String::from("missing child stdout"))?;
        let mut stderr = child.stderr.take().ok_or_else(|| String::from("missing child stderr"))?;
        let out_handle = std::thread::spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            buf
        });
        let err_handle = std::thread::spawn(move || -> Vec<u8> {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf);
            buf
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait().map_err(|err| format!("try_wait failed: {err}"))? {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("curvebook-cli hung beyond {timeout:?} for args {args:?}"));
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        };
        let stdout_bytes = out_handle.join().unwrap_or_default();
        let stderr_bytes = err_handle.join().unwrap_or_default();
        Ok(Output { status, stdout: stdout_bytes, stderr: stderr_bytes })
    }

    pub fn run_ok(args: &[&str], stdin_bytes: Option<&[u8]>) -> Result<String, String> {
        let out = Self::run_with_stdin(args, stdin_bytes)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.code() != Some(0) {
            return Err(format!("CLI failed for {args:?} (code={:?})\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
        }
        Ok(stdout)
    }

    pub fn run_ok_json(args: &[&str]) -> Result<Value, String> {
        let stdout = Self::run_ok(args, None)?;
        serde_json::from_str(&stdout).map_err(|e| format!("parse stdout JSON for {args:?}: {e}\nstdout:\n{stdout}"))
    }

    pub fn run_checked_json(args: &[&str]) -> Result<(Value, String), String> {
        let out = Self::run(args)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.code() != Some(0) {
            return Err(format!("expected exit 0 for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
        }
        let val: Value = serde_json::from_str(&stdout).map_err(|e| format!("parse JSON stdout for {args:?}: {e}\nstdout:\n{stdout}"))?;
        Ok((val, stderr))
    }

    pub fn assert_json_error(args: &[&str], expected_exit: i32, expected_code: &str, expected_category: &str) -> Result<Value, String> {
        let out = Self::run(args)?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.code() != Some(expected_exit) {
            return Err(format!("expected exit {expected_exit} for {args:?}, got {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status.code()));
        }
        let val: Value =
            serde_json::from_str(&stdout).map_err(|e| format!("parse error JSON for {args:?}: {e}\nstdout:\n{stdout}\nstderr:\n{stderr}"))?;
        let err = val.get("error").ok_or_else(|| format!("missing `error` envelope in {val}"))?;
        if err.get("code").and_then(Value::as_str) != Some(expected_code) {
            return Err(format!("unexpected error.code in {val} (expected {expected_code})"));
        }
        if err.get("category").and_then(Value::as_str) != Some(expected_category) {
            return Err(format!("unexpected error.category in {val} (expected {expected_category})"));
        }
        if err.get("message").and_then(Value::as_str).is_none_or(str::is_empty) {
            return Err(format!("missing error.message in {val}"));
        }
        if err.get("retryable").and_then(Value::as_bool).is_none() {
            return Err(format!("missing boolean error.retryable in {val}"));
        }
        if err.get("remediation").is_none() {
            return Err(format!("missing error.remediation key in {val}"));
        }
        if err.get("details").is_none() {
            return Err(format!("missing error.details key in {val}"));
        }
        Ok(val)
    }
}

pub fn env_truthy(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

pub fn loopback_required() -> bool {
    env_truthy("CI") || env_truthy("CURVEBOOK_REQUIRE_LOOPBACK")
}

pub fn frame_content_length(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

pub fn parse_content_length_frames(mut raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut frames = Vec::new();
    while !raw.is_empty() {
        let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
            return Err(format!("truncated Content-Length header in remaining {} byte(s)", raw.len()));
        };
        let (header_bytes, rest) = raw.split_at_checked(header_end).ok_or_else(|| String::from("truncated Content-Length header"))?;
        let header = std::str::from_utf8(header_bytes).map_err(|e| format!("header UTF-8: {e}"))?;
        let mut content_len: Option<usize> = None;
        for line in header.split("\r\n") {
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("content-length")
            {
                content_len = Some(v.trim().parse::<usize>().map_err(|e| format!("parse Content-Length `{v}`: {e}"))?);
            }
        }
        let len = content_len.ok_or_else(|| format!("missing Content-Length header in `{header}`"))?;
        let body_and_rest = rest.get(4..).ok_or_else(|| String::from("missing CRLFCRLF separator"))?;
        let Some((body, tail)) = body_and_rest.split_at_checked(len) else {
            return Err(format!("truncated frame body: expected {len} bytes, got {}", body_and_rest.len()));
        };
        frames.push(serde_json::from_slice(body).map_err(|e| format!("parse frame JSON: {e}"))?);
        raw = tail;
    }
    Ok(frames)
}

pub fn write_framed_rpc(writer: &mut impl Write, payload: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(payload).map_err(|e| format!("serialize rpc: {e}"))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).map_err(|e| format!("write header: {e}"))?;
    writer.write_all(&body).map_err(|e| format!("write body: {e}"))?;
    writer.flush().map_err(|e| format!("flush stdin: {e}"))
}

pub fn read_framed_rpc(reader: &mut BufReader<impl Read>) -> Result<Value, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(|e| format!("read header: {e}"))?;
        if read == 0 {
            return Err(String::from("unexpected EOF while reading MCP headers"));
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            content_length = Some(value.trim().parse::<usize>().map_err(|e| format!("bad Content-Length `{trimmed}`: {e}"))?);
        }
    }
    let len = content_length.ok_or_else(|| String::from("missing Content-Length header in MCP response"))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| format!("read {len}-byte MCP body: {e}"))?;
    serde_json::from_slice(&buf).map_err(|e| format!("parse MCP JSON body: {e}"))
}

pub struct McpChild {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    notifications: VecDeque<Value>,
    finished: bool,
}

impl McpChild {
    fn spawn_inner(args: &[&str], stderr: Stdio) -> Result<Self, String> {
        let mut child = Command::new(CliHarness::bin())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("spawn curvebook-cli {:?}: {e}", args))?;
        let stdin = child.stdin.take().ok_or_else(|| String::from("missing child stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| String::from("missing child stdout"))?);
        Ok(Self { child, stdin: Some(stdin), stdout, next_id: 1, notifications: VecDeque::new(), finished: false })
    }

    pub fn spawn(args: &[&str]) -> Result<Self, String> {
        Self::spawn_inner(args, Stdio::inherit())
    }

    pub fn spawn_piped(args: &[&str]) -> Result<Self, String> {
        Self::spawn_inner(args, Stdio::piped())
    }

    pub fn next_req_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn write_content_length(&mut self, msg: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or_else(|| String::from("child stdin closed"))?;
        write_framed_rpc(stdin, msg)
    }

    pub fn write_ndjson_line(&mut self, msg: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or_else(|| String::from("child stdin closed"))?;
        let mut line = serde_json::to_string(msg).map_err(|e| format!("serialize ndjson: {e}"))?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).map_err(|e| format!("write ndjson: {e}"))?;
        stdin.flush().map_err(|e| format!("flush ndjson: {e}"))
    }

    pub fn read_message(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.stdout.read_line(&mut line).map_err(|e| format!("read stdout: {e}"))?;
            if n == 0 {
                return Err(String::from("unexpected EOF from curvebook-cli mcp stdout"));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('{') || trimmed.starts_with('[') {
                return serde_json::from_str(trimmed).map_err(|e| format!("parse ndjson response `{trimmed}`: {e}"));
            }
            let mut content_len: Option<usize> = None;
            let mut cur = line.clone();
            loop {
                let t = cur.trim();
                if t.is_empty() {
                    break;
                }
                if let Some((name, val)) = t.split_once(':')
                    && name.trim().eq_ignore_ascii_case("content-length")
                {
                    content_len = Some(val.trim().parse::<usize>().map_err(|e| format!("invalid Content-Length `{t}`: {e}"))?);
                }
                cur.clear();
                let next_n = self.stdout.read_line(&mut cur).map_err(|e| format!("read header line: {e}"))?;
                if next_n == 0 {
                    return Err(String::from("unexpected EOF inside Content-Length headers"));
                }
            }
            let len = content_len.ok_or_else(|| format!("missing Content-Length header near `{trimmed}`"))?;
            let mut buf = vec![0u8; len];
            self.stdout.read_exact(&mut buf).map_err(|e| format!("read {len}-byte MCP body: {e}"))?;
            return serde_json::from_slice(&buf).map_err(|e| format!("parse MCP JSON body: {e}"));
        }
    }

    pub fn send_notification(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.write_content_length(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    pub fn request_with_id(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        self.write_content_length(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.read_until_id(id)
    }

    pub fn request_ndjson(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_req_id();
        self.write_ndjson_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;
        self.read_until_id(id)
    }

    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_req_id();
        self.request_with_id(id, method, params)
    }

    pub fn call_tool_envelope(&mut self, id: u64, name: &str, arguments: Value) -> Result<Value, String> {
        self.request_with_id(id, "tools/call", json!({ "name": name, "arguments": arguments }))
    }

    fn read_until_id(&mut self, expected_id: u64) -> Result<Value, String> {
        loop {
            let msg = self.read_message()?;
            if msg.get("id").is_none_or(Value::is_null) {
                self.notifications.push_back(msg);
                continue;
            }
            if msg.get("id") != Some(&json!(expected_id)) {
                return Err(format!("JSON-RPC id mismatch: expected {expected_id}, got {msg}"));
            }
            return Ok(msg);
        }
    }

    pub fn take_notifications(&mut self) -> Vec<Value> {
        self.notifications.drain(..).collect()
    }

    pub fn initialize(&mut self) -> Result<Value, String> {
        let resp = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "r73-stdio-acceptance", "version": "1.0.0" }
            }),
        )?;
        if let Some(err) = resp.get("error") {
            return Err(format!("initialize failed: {err}"));
        }
        self.send_notification("notifications/initialized", json!({}))?;
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn finish(mut self) -> Result<(), String> {
        self.finished = true;
        drop(self.stdin.take());
        let status = self.child.wait().map_err(|e| format!("wait curvebook-cli mcp child: {e}"))?;
        if !status.success() {
            return Err(format!("curvebook-cli mcp exited non-zero: {status}"));
        }
        Ok(())
    }

    pub fn call_tool_raw(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let resp = self.request("tools/call", json!({ "name": name, "arguments": arguments }))?;
        if let Some(err) = resp.get("error") {
            return Err(format!("JSON-RPC protocol error calling `{name}`: {err}"));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn call_tool_ok(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self.call_tool_raw(name, arguments)?;
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(format!("tool `{name}` returned isError=true: {result}"));
        }
        Ok(result)
    }

    pub fn call_tool_payload(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self.call_tool_ok(name, arguments)?;
        if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
            return Ok(sc.clone());
        }
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().find_map(|b| b.get("text").and_then(Value::as_str)))
            .ok_or_else(|| format!("tool `{name}` missing text/structuredContent: {result}"))?;
        serde_json::from_str(text).map_err(|e| format!("parse tool `{name}` text JSON `{text}`: {e}"))
    }

    pub fn call_tool_err(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let result = self.call_tool_raw(name, arguments)?;
        if result.get("isError").and_then(Value::as_bool) != Some(true) {
            return Err(format!("expected tool `{name}` to return isError=true, got: {result}"));
        }
        if let Some(sc) = result.get("structuredContent").filter(|v| !v.is_null()) {
            return Ok(sc.clone());
        }
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // 8 GUI MCP tool wrappers over stdio JSON-RPC
    // -----------------------------------------------------------------------

    pub fn ui_inspect(&mut self, scope: &str, include_bounds: bool) -> Result<Value, String> {
        self.call_tool_payload("ui_inspect", json!({ "scope": scope, "include_bounds": include_bounds }))
    }

    pub fn ui_screenshot(&mut self, target: &str, width_px: u32, height_px: u32) -> Result<Vec<u8>, String> {
        let result = self.call_tool_ok(
            "ui_screenshot",
            json!({
                "target": target,
                "width_px": width_px,
                "height_px": height_px,
                "dpi": 144,
            }),
        )?;
        let img = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().find(|b| b.get("type").and_then(Value::as_str) == Some("image")))
            .ok_or_else(|| format!("ui_screenshot missing image block: {result}"))?;
        if img.get("mimeType").and_then(Value::as_str) != Some("image/png") {
            return Err(format!("ui_screenshot unexpected mimeType: {img}"));
        }
        let b64 = img.get("data").and_then(Value::as_str).ok_or_else(|| format!("ui_screenshot missing base64 data: {img}"))?;
        let bytes = decode_base64(b64)?;
        if bytes.len() < 128 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(format!("ui_screenshot invalid PNG bytes (len={})", bytes.len()));
        }
        Ok(bytes)
    }

    pub fn ui_invoke(&mut self, command_id: &str, params: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_invoke_menu", json!({ "command_id": command_id, "params": params }))
    }

    pub fn ui_invoke_path(&mut self, path: &[&str], params: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_invoke_menu", json!({ "path": path, "params": params }))
    }

    pub fn ui_invoke_err(&mut self, command_id: &str, params: Value) -> Result<Value, String> {
        self.call_tool_err("ui_invoke_menu", json!({ "command_id": command_id, "params": params }))
    }

    pub fn ui_switch_sheet(&mut self, sheet_id: &str) -> Result<Value, String> {
        self.call_tool_payload("ui_switch_tab", json!({ "sheet_id": sheet_id }))
    }

    pub fn ui_switch_tab(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_switch_tab", args)
    }

    pub fn ui_switch_tab_err(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_err("ui_switch_tab", args)
    }

    pub fn ui_answer_dialog(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_answer_dialog", args)
    }

    pub fn ui_answer_ok(&mut self, fields: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_answer_dialog", json!({ "fields": fields, "action": "ok" }))
    }

    pub fn ui_answer_err(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_err("ui_answer_dialog", args)
    }

    pub fn ui_type(&mut self, widget_id: Option<&str>, text: &str, clear_existing: bool, commit_enter: bool) -> Result<Value, String> {
        let mut args = serde_json::Map::new();
        args.insert(String::from("text"), Value::String(text.to_owned()));
        if let Some(wid) = widget_id {
            args.insert(String::from("widget_id"), Value::String(wid.to_owned()));
        }
        args.insert(String::from("clear_existing"), Value::Bool(clear_existing));
        args.insert(String::from("commit_enter"), Value::Bool(commit_enter));
        self.call_tool_payload("ui_type_text", Value::Object(args))
    }

    pub fn ui_type_err(&mut self, widget_id: Option<&str>, text: &str) -> Result<Value, String> {
        let mut args = serde_json::Map::new();
        args.insert(String::from("text"), Value::String(text.to_owned()));
        if let Some(wid) = widget_id {
            args.insert(String::from("widget_id"), Value::String(wid.to_owned()));
        }
        self.call_tool_err("ui_type_text", Value::Object(args))
    }

    pub fn ui_press(&mut self, key: &str, modifiers: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_press_key", json!({ "key": key, "modifiers": modifiers }))
    }

    pub fn ui_press_err(&mut self, key: &str, modifiers: Value) -> Result<Value, String> {
        self.call_tool_err("ui_press_key", json!({ "key": key, "modifiers": modifiers }))
    }

    pub fn ui_pointer(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_payload("ui_pointer_gesture", args)
    }

    pub fn ui_pointer_err(&mut self, args: Value) -> Result<Value, String> {
        self.call_tool_err("ui_pointer_gesture", args)
    }

    // -----------------------------------------------------------------------
    // Higher-level UI workflow helpers (strictly via the 8 ui_* MCP tools)
    // -----------------------------------------------------------------------

    pub fn create_table(&mut self, params: Value) -> Result<String, String> {
        let res = self.ui_invoke("sheet.createData", params)?;
        res.get("payload")
            .and_then(|p| p.get("sheet_id"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("sheet.createData missing sheet_id: {res}"))
    }

    pub fn paste_tsv(&mut self, sheet_id: &str, row: usize, col: usize, subcol: usize, tsv: &str) -> Result<(), String> {
        self.ui_switch_sheet(sheet_id)?;
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.ui_type(Some(&wid), tsv, true, false)?;
        Ok(())
    }

    pub fn set_x_cell(&mut self, sheet_id: &str, row: usize, subcol: usize, text: &str) -> Result<(), String> {
        self.ui_switch_sheet(sheet_id)?;
        let wid = format!("grid.xcell.{row}.{subcol}");
        self.ui_type(Some(&wid), text, true, true)?;
        Ok(())
    }

    pub fn set_cell(&mut self, sheet_id: &str, row: usize, col: usize, subcol: usize, text: &str) -> Result<(), String> {
        self.ui_switch_sheet(sheet_id)?;
        let wid = format!("grid.cell.{row}.{col}.{subcol}");
        self.ui_type(Some(&wid), text, true, true)?;
        Ok(())
    }

    pub fn exclude_cell_cmd_shift_e(
        &mut self,
        sheet_id: &str,
        row: usize,
        col: usize,
        subcol: usize,
        reason_tag: &str,
        note: &str,
    ) -> Result<(), String> {
        self.ui_switch_sheet(sheet_id)?;
        self.ui_pointer(json!({ "action": "click", "widget_id": format!("grid.cell.{row}.{col}.{subcol}") }))?;
        self.ui_press("e", json!({ "meta": true, "shift": true }))?;
        self.ui_answer_ok(json!({ "reason_tag": reason_tag, "note": note }))?;
        Ok(())
    }

    pub fn navigator_result_ids(&mut self) -> Result<Vec<String>, String> {
        let nav = self.ui_inspect("navigator", false)?;
        let arr = nav.get("navigator_sheets").and_then(Value::as_array).ok_or_else(|| format!("missing navigator_sheets in ui_inspect: {nav}"))?;
        Ok(arr
            .iter()
            .filter(|s| s.get("section").and_then(Value::as_str) == Some("results"))
            .filter_map(|s| s.get("sheet_id").and_then(Value::as_str).map(ToOwned::to_owned))
            .collect())
    }

    pub fn run_analyze(&mut self, sheet_id: &str, fields: Value) -> Result<String, String> {
        let before = self.navigator_result_ids()?;
        self.ui_switch_sheet(sheet_id)?;
        self.ui_invoke("ui.analyze.open", json!({ "sheet_id": sheet_id }))?;
        let out = self.ui_answer_dialog(json!({
            "fields": fields,
            "force_create_new": true,
            "action": "ok",
        }))?;
        if let Some(sid) = out.get("payload").and_then(|p| p.get("sheet_id")).and_then(Value::as_str) {
            return Ok(sid.to_owned());
        }
        let after = self.navigator_result_ids()?;
        after.into_iter().rev().find(|id| !before.contains(id)).ok_or_else(|| format!("analyze did not produce a new ResultSheet: {out}"))
    }

    pub fn edit_analysis_cmd_t(&mut self, result_sheet_id: &str, fields: Value) -> Result<Value, String> {
        self.ui_switch_sheet(result_sheet_id)?;
        self.ui_press("t", json!({ "meta": true }))?;
        self.ui_answer_ok(fields)
    }

    pub fn graph_for_source(&mut self, source_id: &str) -> Result<String, String> {
        let nav = self.ui_inspect("navigator", false)?;
        let arr = nav.get("navigator_sheets").and_then(Value::as_array).ok_or_else(|| format!("missing navigator_sheets: {nav}"))?;
        let expected_prefix = format!("{source_id}-graph");
        if let Some(found) = arr
            .iter()
            .filter(|s| s.get("section").and_then(Value::as_str) == Some("graphs"))
            .filter_map(|s| s.get("sheet_id").and_then(Value::as_str))
            .find(|id| id.starts_with(&expected_prefix) || *id == source_id)
        {
            return Ok(found.to_owned());
        }
        arr.iter()
            .rev()
            .filter(|s| s.get("section").and_then(Value::as_str) == Some("graphs"))
            .filter_map(|s| s.get("sheet_id").and_then(Value::as_str))
            .next()
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("no graph sheet found near `{source_id}` in navigator: {nav}"))
    }
}

impl Drop for McpChild {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.stdin.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn encode_base64(input: &[u8]) -> String {
    curvebook_platform::base64::encode(input)
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|e| e.to_string())
}

pub fn find_node<'a>(nodes: &'a Value, widget_id: &str) -> Option<&'a Value> {
    nodes.as_array()?.iter().find(|n| n.get("widget_id").and_then(Value::as_str) == Some(widget_id))
}

pub fn assert_close(actual: f64, expected: f64, tol: f64, label: &str) -> Result<(), String> {
    if !actual.is_finite() || (actual - expected).abs() > tol {
        return Err(format!("{label}: expected {expected} +/- {tol}, got {actual}"));
    }
    Ok(())
}

pub fn assert_approx(actual: f64, expected: f64, tol: f64, label: &str) -> Result<(), String> {
    assert_close(actual, expected, tol, label)
}

/// Save current GUI project via `"File/Save"` into `workdir/archive_rel`, then spawn a fresh
/// headless `curvebook-cli mcp --workdir` child process, call `open_file`, and run verification.
pub fn save_and_verify_headless(
    gui: &mut McpChild,
    workdir: &Path,
    archive_rel: &str,
    verify: impl FnOnce(&mut McpChild) -> Result<(), String>,
) -> Result<(), String> {
    let save_out = gui.ui_invoke_path(&["File", "Save"], json!({ "path": archive_rel }))?;
    if save_out.get("invoked").and_then(Value::as_bool) != Some(true) {
        return Err(format!("File/Save unexpected output: {save_out}"));
    }
    let archive_path = workdir.join(archive_rel);
    let meta = fs::metadata(&archive_path).map_err(|e| format!("saved archive missing at {}: {e}", archive_path.display()))?;
    if meta.len() < 16 {
        return Err(format!("saved archive too small ({} bytes) at {}", meta.len(), archive_path.display()));
    }

    let wd_str = workdir.to_string_lossy().into_owned();
    let mut headless = McpChild::spawn(&["mcp", "--workdir", &wd_str])?;
    headless.initialize()?;
    let opened = headless.call_tool_payload("open_file", json!({ "path": archive_rel }))?;
    if opened.get("total_sheet_count").and_then(Value::as_u64).or_else(|| opened.get("total_sheets").and_then(Value::as_u64)).unwrap_or(0) == 0 {
        return Err(format!("reopened archive had zero sheets: {opened}"));
    }
    verify(&mut headless)
}

pub fn read_result_json(headless: &mut McpChild, result_sheet_id: &str) -> Result<Value, String> {
    let uri = format!("result://{result_sheet_id}");
    let resp = headless.request("resources/read", json!({ "uri": uri }))?;
    if let Some(err) = resp.get("error") {
        return Err(format!("resources/read `{result_sheet_id}` error: {err}"));
    }
    let text = resp
        .pointer("/result/contents/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing text in resources/read `{result_sheet_id}`: {resp}"))?;
    serde_json::from_str(text).map_err(|e| format!("parse result JSON `{result_sheet_id}`: {e}"))
}

pub fn result_scalar_f64(rs_json: &Value, key: &str) -> Result<f64, String> {
    if let Some(arr) = rs_json.get("scalars").and_then(Value::as_array)
        && let Some(item) = arr.iter().find(|s| s.get("key").and_then(Value::as_str) == Some(key))
        && let Some(v) = item.get("value").and_then(Value::as_f64)
    {
        return Ok(v);
    }
    if let Some(obj) = rs_json.get("scalars").and_then(Value::as_object)
        && let Some(v) = obj.get(key).and_then(Value::as_f64)
    {
        return Ok(v);
    }
    Err(format!("missing scalar `{key}` in result JSON: {}", rs_json.get("scalars").unwrap_or(&Value::Null)))
}

pub fn result_tab<'a>(rs_json: &'a Value, tab_id: &str) -> Result<&'a Value, String> {
    rs_json
        .get("tabs")
        .and_then(Value::as_array)
        .and_then(|arr| arr.iter().find(|t| t.get("id").and_then(Value::as_str) == Some(tab_id)))
        .ok_or_else(|| format!("missing tab `{tab_id}` in result JSON"))
}

pub fn tab_cell_f64(tab: &Value, row_label: &str, col_name: &str) -> Result<f64, String> {
    let rows_lbl = tab.get("row_labels").and_then(Value::as_array).ok_or("tab missing row_labels")?;
    let cols_hdr = tab.get("column_headers").and_then(Value::as_array).ok_or("tab missing column_headers")?;
    let r_idx =
        rows_lbl.iter().position(|v| v.as_str() == Some(row_label)).ok_or_else(|| format!("row `{row_label}` missing in tab rows={rows_lbl:?}"))?;
    let c_idx =
        cols_hdr.iter().position(|v| v.as_str() == Some(col_name)).ok_or_else(|| format!("col `{col_name}` missing in tab cols={cols_hdr:?}"))?;
    let cell = tab
        .get("rows")
        .and_then(|r| r.get(r_idx))
        .and_then(|row| row.get(c_idx))
        .ok_or_else(|| format!("missing cell at ({row_label}, {col_name})"))?;
    extract_result_cell_f64(cell).ok_or_else(|| format!("non-numeric cell at ({row_label}, {col_name}): {cell}"))
}

pub fn extract_result_cell_f64(cell: &Value) -> Option<f64> {
    if let Some(v) = cell.as_f64() {
        return Some(v);
    }
    cell.get("value")
        .and_then(Value::as_f64)
        .or_else(|| cell.get("raw").and_then(Value::as_f64))
        .or_else(|| cell.get("Number").and_then(|n| n.get("value")).and_then(Value::as_f64))
        .or_else(|| cell.get("PValue").and_then(|p| p.get("raw")).and_then(Value::as_f64))
}
