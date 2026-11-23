//! MCP protocol and resource-template audit regression tests.

use std::io::Cursor;

use curvebook_mcp::{
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, McpServerState, ResourceTemplateDescriptor, call_tool, dispatch_json_value, list_resource_templates,
    serve,
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
fn resources_templates_list_headless_normal_and_gui_only() -> TestResult {
    let mut normal = McpServerState::new();
    let reply = rpc(&mut normal, 1, "resources/templates/list", json!({}))?;
    assert_canonical_templates(&reply)?;

    // Omitting `params` defaults to an empty object and also succeeds.
    let omitted = dispatch_json_value(json!({ "jsonrpc": "2.0", "id": 2, "method": "resources/templates/list" }), &mut normal)
        .ok_or_else(|| String::from("expected reply when params omitted"))?;
    assert_canonical_templates(&omitted)?;

    let mut gui_only = McpServerState::new().with_gui_only(true);
    let gui_reply = rpc(&mut gui_only, 3, "resources/templates/list", json!({}))?;
    assert_eq!(gui_reply.pointer("/result/resourceTemplates"), Some(&json!([])));
    Ok(())
}

#[test]
fn resources_templates_list_live_attach_normal_and_gui_only() -> TestResult {
    let mut live_normal = McpServerState::with_control_port(49601).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(
        |method, _params| match method {
            "ping" => Ok(json!({ "pong": true, "gui_only": false })),
            other => Err(format!("unexpected remote method {other}")),
        },
    );
    let normal_reply = rpc(&mut live_normal, 1, "resources/templates/list", json!({}))?;
    assert_canonical_templates(&normal_reply)?;

    let mut live_remote_gui_only = McpServerState::with_control_port(49602).map_err(|e| format!("with_control_port: {e}"))?.with_control_handler(
        |method, _params| match method {
            "ping" => Ok(json!({ "pong": true, "gui_only": true })),
            other => Err(format!("unexpected remote method {other}")),
        },
    );
    let remote_gui_reply = rpc(&mut live_remote_gui_only, 2, "resources/templates/list", json!({}))?;
    assert!(live_remote_gui_only.is_gui_only(), "should adopt remote --gui-only policy");
    assert_eq!(remote_gui_reply.pointer("/result/resourceTemplates"), Some(&json!([])));

    let mut live_mismatch = McpServerState::with_control_port_and_gui_only(49603, true)
        .map_err(|e| format!("with_control_port_and_gui_only: {e}"))?
        .with_control_handler(|method, _params| match method {
            "ping" => Ok(json!({ "pong": true, "gui_only": false })),
            other => Err(format!("unexpected remote method {other}")),
        });
    let mismatch_reply = rpc(&mut live_mismatch, 3, "resources/templates/list", json!({}))?;
    assert_eq!(mismatch_reply.pointer("/error/code"), Some(&json!(ERR_INVALID_REQUEST)));
    assert_eq!(mismatch_reply.pointer("/error/data/code"), Some(&json!("gui_only_violation")));
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
fn ui_screenshot_path_under_gui_only_remediation_references_inline_image_png() -> TestResult {
    let mut state = McpServerState::new().with_gui_only(true);
    let result = call_tool(&mut state, "ui_screenshot", json!({ "path": "shot.png" }))
        .map_err(|e| format!("expected structured gui_only_violation tool error: {e}"))?;
    assert!(result.is_error, "ui_screenshot with path must fail under --gui-only");
    let structured = result.structured_content.as_ref().ok_or_else(|| String::from("missing structured_content"))?;
    assert_eq!(structured.get("code"), Some(&json!("gui_only_violation")));
    let remediation = structured.get("remediation").and_then(Value::as_str).ok_or_else(|| format!("missing remediation: {structured}"))?;
    assert!(remediation.contains("image/png"), "remediation should reference inline `image/png` content block, got: {remediation}");
    assert!(!remediation.contains("data_base64"), "remediation must not reference `data_base64`, got: {remediation}");
    Ok(())
}
