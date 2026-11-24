#![forbid(unsafe_code)]
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Shared test helpers for `crates/mcp` integration suites.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_mcp::{ContentBlock, McpServerState, ToolCallResult, serve};
use serde_json::{Value, json};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(1);

pub struct TempDir {
    pub path: PathBuf,
}

impl TempDir {
    fn create(prefix: &str, canonicalize: bool) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb-mcp-{prefix}-{}-{seq}", std::process::id()));
        if dir.exists() {
            let _ = fs::remove_dir_all(&dir);
        }
        fs::create_dir_all(&dir).map_err(|err| format!("create temp dir `{}`: {err}", dir.display()))?;
        let path = if canonicalize { fs::canonicalize(&dir).unwrap_or(dir) } else { dir };
        Ok(Self { path })
    }

    pub fn new(prefix: &str) -> Result<Self, String> {
        Self::create(prefix, false)
    }

    pub fn new_canonical(prefix: &str) -> Result<Self, String> {
        Self::create(prefix, true)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn child(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn env_truthy(var: &str) -> bool {
    std::env::var(var).is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

pub fn loopback_required() -> bool {
    env_truthy("CI") || env_truthy("CURVEBOOK_REQUIRE_LOOPBACK")
}

pub fn frame_cl(body: &Value) -> Result<Vec<u8>, String> {
    let raw = serde_json::to_string(body).map_err(|err| format!("serialize request: {err}"))?;
    Ok(format!("Content-Length: {}\r\n\r\n{raw}", raw.len()).into_bytes())
}

pub fn decode_cl_frames(raw: &[u8]) -> Result<Vec<Value>, String> {
    let mut cursor = raw;
    let mut out = Vec::new();
    while !cursor.is_empty() {
        let text = std::str::from_utf8(cursor).map_err(|err| format!("valid utf8: {err}"))?;
        let (headers, rest) = text.split_once("\r\n\r\n").ok_or_else(|| String::from("header separator"))?;
        let len_str = headers.strip_prefix("Content-Length: ").ok_or_else(|| String::from("content-length header"))?.trim();
        let len: usize = len_str.parse().map_err(|err| format!("valid integer: {err}"))?;
        let Some((body, tail)) = rest.as_bytes().split_at_checked(len) else {
            return Err(format!("truncated body: expected {len} bytes, found {}", rest.len()));
        };
        out.push(serde_json::from_slice(body).map_err(|err| format!("json frame: {err}"))?);
        cursor = tail;
    }
    Ok(out)
}

pub fn run_cl_rpc(state: &mut McpServerState, req: &Value) -> Result<Value, String> {
    let input = frame_cl(req)?;
    let mut output = Vec::new();
    serve(&mut Cursor::new(input), &mut output, state).map_err(|err| format!("serve: {err}"))?;
    let mut replies = decode_cl_frames(&output)?;
    if replies.len() != 1 {
        return Err(format!("expected 1 response for request {req}, got {}", replies.len()));
    }
    Ok(replies.remove(0))
}

pub fn rpc_roundtrip(state: &mut McpServerState, id: i64, method: &str, params: Value) -> Result<Value, String> {
    let framed = frame_cl(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    }))?;
    let mut out = Vec::new();
    serve(&mut Cursor::new(framed), &mut out, state).map_err(|err| format!("serve {method}: {err}"))?;
    decode_cl_frames(&out)?.into_iter().next().ok_or_else(|| format!("expected response frame for {method}"))
}

pub fn parse_result_json(result: &ToolCallResult) -> Result<Value, String> {
    if let Some(structured) = &result.structured_content {
        return Ok(structured.clone());
    }
    let text = result
        .content
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Image { .. } => None,
        })
        .ok_or_else(|| String::from("tool result missing text block"))?;
    serde_json::from_str(text).map_err(|err| format!("invalid JSON tool payload: {err}"))
}

pub fn encode_base64(bytes: &[u8]) -> String {
    curvebook_platform::base64::encode(bytes)
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    curvebook_platform::base64::decode(input).map_err(|err| err.to_string())
}
