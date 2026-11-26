//! MCP protocol and resource-template audit regression tests.

use std::collections::VecDeque;
use std::io::{BufReader, Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use curvebook_doc::ErrorCode;
use curvebook_engine::EngineError;
use curvebook_mcp::{
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, GuiSessionHost, McpServerState, ResourceTemplateDescriptor, dispatch_json_value,
    list_resource_templates, serve, serve_concurrent,
};
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, String>;

fn rpc(state: &mut McpServerState, id: i64, method: &str, params: Value) -> TestResult<Value> {
    dispatch_json_value(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }),
        state,
    )
    .ok_or_else(|| format!("expected JSON-RPC reply for {method}"))
}

fn assert_canonical_templates(reply: &Value) -> TestResult {
    let raw = reply.pointer("/result/resourceTemplates").ok_or_else(|| format!("missing result.resourceTemplates in {reply}"))?;
    let parsed: Vec<ResourceTemplateDescriptor> =
        serde_json::from_value(raw.clone()).map_err(|err| format!("failed to decode resourceTemplates: {err}"))?;
    assert_eq!(parsed, list_resource_templates());
    assert_eq!(parsed.len(), 2, "expected exactly sheet:// and result:// templates: {raw}");

    let arr = raw.as_array().ok_or_else(|| format!("resourceTemplates must be an array: {raw}"))?;
    let uris: Vec<&str> = arr.iter().filter_map(|t| t.get("uriTemplate").and_then(Value::as_str)).collect();
    assert_eq!(uris, vec!["sheet://{id}", "result://{id}"]);
    for tmpl in arr {
        assert_eq!(tmpl.get("mimeType"), Some(&json!("application/json")));
        assert!(tmpl.get("name").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
        assert!(tmpl.get("description").and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
    }
    Ok(())
}

#[test]
fn resources_templates_list_headless() -> TestResult {
    let mut normal = McpServerState::new();
    let reply = rpc(&mut normal, 1, "resources/templates/list", json!({}))?;
    assert_canonical_templates(&reply)?;

    // Omitting `params` defaults to an empty object and also succeeds.
    let omitted = dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/templates/list" }), &mut normal)
        .ok_or_else(|| String::from("expected reply when params omitted"))?;
    assert_canonical_templates(&omitted)?;
    Ok(())
}

#[test]
fn resources_templates_list_live_attach() -> TestResult {
    let mut live_normal = McpServerState::with_control_port(49601).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(
        |method, _params| match method {
            "ping" => Ok(json!({ "pong": true })),
            other => Err(format!("unexpected remote method {other}")),
        },
    );
    let normal_reply = rpc(&mut live_normal, 1, "resources/templates/list", json!({}))?;
    assert_canonical_templates(&normal_reply)?;
    Ok(())
}

#[test]
fn resources_templates_list_rejects_non_object_params() -> TestResult {
    let mut state = McpServerState::new();
    let reply = rpc(&mut state, 1, "resources/templates/list", json!([]))?;
    assert_eq!(reply.pointer("/error/code"), Some(&json!(ERR_INVALID_PARAMS)));
    Ok(())
}

#[test]
fn premature_notifications_initialized_does_not_break_initialize_handshake() -> TestResult {
    // 1. Default session: premature `notifications/initialized` (and legacy `initialized`)
    //    must neither mark initialized nor break the subsequent `initialize` handshake.
    let mut state = McpServerState::new();
    let script = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
    );
    let mut out = Vec::new();
    serve(&mut Cursor::new(script.as_bytes()), &mut out, &mut state).map_err(|e| format!("serve: {e}"))?;
    let text = String::from_utf8(out).map_err(|e| format!("utf8: {e}"))?;
    let replies: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).map_err(|e| format!("json line: {e}"))).collect::<TestResult<Vec<_>>>()?;

    assert_eq!(replies.len(), 2, "notifications must not emit responses: {replies:?}");
    assert_eq!(replies.first().and_then(|r| r.pointer("/result/protocolVersion")), Some(&json!("2024-11-05")));
    assert!(state.is_initialized());
    assert_eq!(replies.get(1).and_then(|r| r.pointer("/error/code")), Some(&json!(ERR_INVALID_REQUEST)));

    // 2. Strict `require_initialize` session: premature `notifications/initialized` must NOT
    //    satisfy `require_initialize`; operational calls stay rejected until `initialize` runs.
    let mut strict = McpServerState::new().with_require_initialize(true);
    let strict_script = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"tools/list\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":13,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\"}}\n",
    );
    let mut strict_out = Vec::new();
    serve(&mut Cursor::new(strict_script.as_bytes()), &mut strict_out, &mut strict).map_err(|e| format!("serve strict: {e}"))?;
    let strict_text = String::from_utf8(strict_out).map_err(|e| format!("utf8 strict: {e}"))?;
    let strict_replies: Vec<Value> =
        strict_text.lines().map(|l| serde_json::from_str(l).map_err(|e| format!("json line: {e}"))).collect::<TestResult<Vec<_>>>()?;

    assert_eq!(strict_replies.len(), 4);
    assert_eq!(strict_replies.first().and_then(|r| r.pointer("/error/code")), Some(&json!(ERR_INVALID_REQUEST)));
    assert_eq!(strict_replies.get(1).and_then(|r| r.pointer("/result/protocolVersion")), Some(&json!("2024-11-05")));
    assert!(strict_replies.get(2).and_then(|r| r.pointer("/result/tools")).is_some_and(Value::is_array));
    assert_eq!(strict_replies.get(3).and_then(|r| r.pointer("/error/code")), Some(&json!(ERR_INVALID_REQUEST)));
    Ok(())
}

#[test]
fn serve_concurrent_delivers_cancel_while_request_in_flight() -> TestResult {
    struct ChanRead(mpsc::Receiver<Vec<u8>>, VecDeque<u8>);
    impl Read for ChanRead {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            while self.1.is_empty() {
                match self.0.recv() {
                    Ok(c) if !c.is_empty() => self.1.extend(c),
                    Ok(_) => continue,
                    Err(_) => return Ok(0),
                }
            }
            let n = out.len().min(self.1.len());
            for dst in &mut out[..n] {
                *dst = self.1.pop_front().unwrap_or(0);
            }
            Ok(n)
        }
    }
    struct SlotCancelHost {
        reg_slot: Arc<curvebook_platform::sync::Mutex<Option<curvebook_mcp::CancellationRegistry>>>,
        started: Option<mpsc::Sender<()>>,
        observed: Arc<AtomicBool>,
    }
    impl GuiSessionHost for SlotCancelHost {
        fn call_gui(&mut self, _: &str, _: Value) -> Result<Value, EngineError> {
            if let Some(tx) = self.started.take() {
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

    let (in_tx, in_rx) = mpsc::channel::<Vec<u8>>();
    let reg_slot = Arc::new(curvebook_platform::sync::Mutex::named("test.reg_slot", None));
    let (s_tx, s_rx) = mpsc::channel::<()>();
    let observed = Arc::new(AtomicBool::new(false));
    let mut state = McpServerState::with_gui_host(Box::new(SlotCancelHost {
        reg_slot: Arc::clone(&reg_slot),
        started: Some(s_tx),
        observed: Arc::clone(&observed),
    }));
    if let Ok(mut g) = reg_slot.lock() {
        *g = Some(state.cancellation_registry());
    }
    let sender = thread::spawn(move || {
        let _ = in_tx.send(
            format!("{}\n", json!({ "jsonrpc": "2.0", "id": 42, "method": "tools/call", "params": { "name": "ui_inspect", "arguments": {} } }))
                .into_bytes(),
        );
        if s_rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            let _ = in_tx
                .send(format!("{}\n", json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 42 } })).into_bytes());
        }
        thread::sleep(Duration::from_millis(50));
    });
    let mut output = Vec::new();
    serve_concurrent(BufReader::new(ChanRead(in_rx, VecDeque::new())), &mut output, &mut state, 1024 * 1024).map_err(|e| e.to_string())?;
    let _ = sender.join();
    assert!(observed.load(Ordering::Acquire));
    let reply: Value = serde_json::from_str(String::from_utf8(output).map_err(|e| e.to_string())?.trim()).map_err(|e| e.to_string())?;
    assert_eq!(
        (&reply["id"], &reply["result"]["isError"], &reply["result"]["structuredContent"]["code"]),
        (&json!(42), &json!(true), &json!("cancelled"))
    );
    Ok(())
}
