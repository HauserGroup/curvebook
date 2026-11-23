use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use curvebook_mcp::{
    ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND, ERR_PARSE_ERROR, MAX_BATCH_LEN, McpServerState, call_tool, list_tools, serve,
    serve_with_max_bytes,
};
use proptest::{collection, prop_assert, prop_assert_eq, proptest, strategy::Strategy};
use serde_json::{Map, Value, json};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = std::env::temp_dir().join(format!("curvebook-{prefix}-{}-{nanos}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        Self { path: dir }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn frame_cl(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!("{headers}\r\n\r\n").into_bytes();
    out.extend_from_slice(body);
    out
}

fn parse_first_cl_response(output: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(output).map_err(|err| format!("response must be valid UTF-8: {err}"))?;
    let (hdr, rest) = text.split_once("\r\n\r\n").ok_or_else(|| String::from("expected Content-Length terminator"))?;
    let len_str = hdr.strip_prefix("Content-Length: ").ok_or_else(|| String::from("expected Content-Length prefix"))?.trim();
    let len: usize = len_str.parse().map_err(|err| format!("valid integer length: {err}"))?;
    if rest.len() < len {
        return Err(format!("truncated response body: expected {len} bytes, found {}", rest.len()));
    }
    let (body, _) = rest.as_bytes().split_at_checked(len).ok_or_else(|| format!("truncated response body: expected {len} bytes"))?;
    serde_json::from_slice(body).map_err(|err| format!("response body must be valid JSON: {err}"))
}

fn verify_stream_output_is_well_formed(output: &[u8]) -> Result<(), String> {
    if output.is_empty() {
        return Ok(());
    }
    let mut rest = std::str::from_utf8(output).map_err(|err| format!("non-UTF8 output: {err}"))?;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        if rest.is_empty() {
            break;
        }
        if let Some(after_prefix) = rest.strip_prefix("Content-Length: ") {
            let Some((hdr, after)) = after_prefix.split_once("\r\n\r\n") else {
                return Err(String::from("incomplete Content-Length output frame"));
            };
            let len: usize = hdr.trim().parse().map_err(|_| String::from("invalid Content-Length header in server reply"))?;
            let Some((body, tail)) = after.as_bytes().split_at_checked(len) else {
                return Err(String::from("truncated Content-Length output body"));
            };
            let _: Value = serde_json::from_slice(body).map_err(|err| format!("invalid JSON in CL reply: {err}"))?;
            rest = std::str::from_utf8(tail).map_err(|err| format!("non-UTF8 trailing output: {err}"))?;
        } else {
            let (line, tail) = rest.split_once('\n').unwrap_or((rest, ""));
            let _: Value = serde_json::from_str(line.trim()).map_err(|err| format!("invalid JSON in NDJSON reply: {err}"))?;
            rest = tail;
        }
    }
    Ok(())
}

#[test]
fn rejects_malformed_and_giant_content_length_headers() {
    let cases: &[(&str, i64)] = &[
        ("Content-Length: 99999999999999999999999999", ERR_INVALID_REQUEST),
        ("Content-Length: 10000000", ERR_INVALID_REQUEST),
        ("Content-Length: -5", ERR_INVALID_REQUEST),
        ("Content-Length: abc", ERR_INVALID_REQUEST),
        ("Content-Length: 12 34", ERR_INVALID_REQUEST),
        ("Content-Length: 10\r\nContent-Length: 10", ERR_INVALID_REQUEST),
        ("Content-Type: application/json", ERR_INVALID_REQUEST),
        ("Content-Length: 2\r\nBad Header Name: 10", ERR_INVALID_REQUEST),
        ("Content-Length: 2\r\nNoColonHeader", ERR_INVALID_REQUEST),
    ];

    for &(headers, expected_code) in cases {
        let input = frame_cl(headers, b"{}");
        let mut reader = Cursor::new(input);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 4096).expect("serve");
        let reply = parse_first_cl_response(&output).expect("valid CL reply");
        assert_eq!(reply["error"]["code"], json!(expected_code), "headers: {headers}");
    }
}

#[test]
fn handles_lf_headers_extra_headers_and_case_insensitivity() {
    let body = br#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;
    let mut input = Vec::new();
    input.extend_from_slice(b"content-type: application/vscode-jsonrpc; charset=utf-8\n");
    input.extend_from_slice(format!("cOnTeNt-LeNgTh: {}\n\n", body.len()).as_bytes());
    input.extend_from_slice(body);

    let mut reader = Cursor::new(input);
    let mut output = Vec::new();
    let mut state = McpServerState::new();
    serve(&mut reader, &mut output, &mut state).expect("serve");
    let reply = parse_first_cl_response(&output).expect("valid CL reply");
    assert_eq!(reply["id"], json!(7));
    assert_eq!(reply["result"], json!({}));
}

#[test]
fn handles_truncated_cl_body_excess_headers_and_oversized_header_line() {
    let truncated = frame_cl("Content-Length: 200", b"{\"jsonrpc\":\"2.0\"");
    let mut out = Vec::new();
    let mut state = McpServerState::new();
    serve(&mut Cursor::new(truncated), &mut out, &mut state).expect("serve");
    assert_eq!(parse_first_cl_response(&out).expect("valid CL reply")["error"]["code"], json!(ERR_PARSE_ERROR));

    let too_many_headers = format!("{}Content-Length: 2\r\n\r\n{{}}", "X-Pad: 1\r\n".repeat(40));
    let mut out = Vec::new();
    serve(&mut Cursor::new(too_many_headers.as_bytes()), &mut out, &mut state).expect("serve");
    assert_eq!(parse_first_cl_response(&out).expect("valid CL reply")["error"]["code"], json!(ERR_INVALID_REQUEST));

    let long_header = format!("X-Huge: {}\r\nContent-Length: 2\r\n\r\n{{}}", "a".repeat(9000));
    let mut out = Vec::new();
    serve(&mut Cursor::new(long_header.as_bytes()), &mut out, &mut state).expect("serve");
    assert_eq!(parse_first_cl_response(&out).expect("valid CL reply")["error"]["code"], json!(ERR_INVALID_REQUEST));
}

#[test]
fn recovers_after_oversized_ndjson_line_and_bad_utf8() {
    let mut input = Vec::new();
    input.extend_from_slice(b"{\"big\":\"");
    input.extend_from_slice(&vec![b'x'; 2048]);
    input.extend_from_slice(b"\"}\n");
    input.extend_from_slice(b"\xff\xfe\xfd\x80\n");
    input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"ping\"}\n");

    let mut output = Vec::new();
    let mut state = McpServerState::new();
    serve_with_max_bytes(&mut Cursor::new(input), &mut output, &mut state, 512).expect("serve");

    let text = String::from_utf8(output).expect("utf8 replies");
    let replies: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).expect("json")).collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
    assert_eq!(replies[1]["error"]["code"], json!(ERR_PARSE_ERROR));
    assert_eq!(replies[2]["id"], json!(42));
    assert_eq!(replies[2]["result"], json!({}));
}

#[test]
fn enforces_jsonrpc_shape_notifications_batch_and_param_validation() {
    let script = concat!(
        "{\"jsonrpc\":\"2.0\",\"method\":\"unknown/notify\",\"params\":{\"a\":1}}\n",
        "[{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}]\n",
        "{\"jsonrpc\":\"2.0\",\"id\":true,\"method\":\"ping\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"params\":123}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"does/not/exist\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"render_preview\",\"arguments\":{\"width_px\":0}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"render_preview\",\"arguments\":{\"dpi\":5}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"open_file\",\"arguments\":{\"path\":\"   \"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"command_batch\",\"arguments\":{\"commands\":\"bad\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\",\"params\":{\"name\":\"save_file\",\"arguments\":{}}}\n",
    );

    let mut output = Vec::new();
    let mut state = McpServerState::new();
    serve(&mut Cursor::new(script.as_bytes()), &mut output, &mut state).expect("serve");

    let replies: Vec<Value> = String::from_utf8(output).expect("utf8").lines().map(|l| serde_json::from_str(l).expect("json")).collect();
    // Both notifications (single & notification-only batch) emit zero responses, so 8 replies remain.
    assert_eq!(replies.len(), 8);
    assert_eq!(replies[0]["error"]["code"], json!(ERR_INVALID_REQUEST));
    assert_eq!(replies[1]["error"]["code"], json!(ERR_INVALID_REQUEST));
    assert_eq!(replies[2]["error"]["code"], json!(ERR_METHOD_NOT_FOUND));
    assert_eq!(replies[3]["error"]["code"], json!(ERR_INVALID_PARAMS));
    assert_eq!(replies[4]["error"]["code"], json!(ERR_INVALID_PARAMS));
    assert_eq!(replies[5]["error"]["code"], json!(ERR_INVALID_PARAMS));
    assert_eq!(replies[6]["error"]["code"], json!(ERR_INVALID_PARAMS));
    assert_eq!(replies[7]["error"]["code"], json!(ERR_INVALID_PARAMS));
}

#[test]
fn oversized_batch_returns_invalid_request() {
    let items: Vec<Value> = (0..=MAX_BATCH_LEN).map(|i| json!({"jsonrpc": "2.0", "id": i, "method": "ping"})).collect();
    let line = format!("{}\n", serde_json::to_string(&items).expect("batch json"));
    let mut out = Vec::new();
    let mut state = McpServerState::new();
    serve(&mut Cursor::new(line.as_bytes()), &mut out, &mut state).expect("serve");
    let reply: Value = serde_json::from_slice(&out).expect("json");
    assert_eq!(reply["error"]["code"], json!(ERR_INVALID_REQUEST));
}

fn arb_ident_string(min_len: usize, max_len: usize) -> impl Strategy<Value = String> {
    let ch = proptest::prop_oneof![proptest::char::range('a', 'z'), proptest::char::range('0', '9'), proptest::strategy::Just('_'),];
    collection::vec(ch, min_len..=max_len).prop_map(|chars| chars.into_iter().collect())
}

fn arb_json_value() -> impl Strategy<Value = Value> {
    let leaf = proptest::prop_oneof![
        proptest::strategy::Just(Value::Null),
        proptest::arbitrary::any::<bool>().prop_map(Value::Bool),
        (-10_000i64..10_000i64).prop_map(|n| json!(n)),
        arb_ident_string(0, 16).prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 6, |inner| {
        proptest::prop_oneof![
            collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
            collection::vec((arb_ident_string(1, 12), inner), 0..5).prop_map(|pairs| {
                let mut map = Map::new();
                for (k, v) in pairs {
                    map.insert(k, v);
                }
                Value::Object(map)
            })
        ]
    })
}

fn sanitize_paths_in_tempdir(mut args: Value, temp_file: &std::path::Path) -> Value {
    if let Some(obj) = args.as_object_mut() {
        let safe = Value::String(temp_file.display().to_string());
        for key in ["path", "output_path"] {
            if obj.contains_key(key) {
                obj.insert(key.to_owned(), safe.clone());
            }
        }
    }
    args
}

proptest! {
    #[test]
    fn arbitrary_byte_streams_never_panic_and_reply_valid_frames(raw in collection::vec(proptest::arbitrary::any::<u8>(), 0..1024)) {
        let mut reader = Cursor::new(raw);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut reader, &mut output, &mut state, 2048).expect("serve");
        prop_assert!(verify_stream_output_is_well_formed(&output).is_ok(), "malformed reply: {:?}", verify_stream_output_is_well_formed(&output));
    }

    #[test]
    fn adversarial_content_length_headers_never_oom_or_panic(
        digits in "[0-9\\-+ a-zA-Z]{0,28}",
        sep in proptest::prop_oneof!["\r\n\r\n", "\n\n", "\r\n", ""],
        body in collection::vec(proptest::arbitrary::any::<u8>(), 0..128),
    ) {
        let mut frame = format!("Content-Length: {digits}{sep}").into_bytes();
        frame.extend_from_slice(&body);
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut Cursor::new(frame), &mut output, &mut state, 1024).expect("serve");
        prop_assert!(verify_stream_output_is_well_formed(&output).is_ok());
    }

    #[test]
    fn unknown_methods_with_random_params_never_panic(
        method in "[a-zA-Z0-9_./-]{0,32}",
        params in arb_json_value(),
        has_id in proptest::arbitrary::any::<bool>(),
    ) {
        if matches!(
            method.as_str(),
            "initialize"
                | "shutdown"
                | "ping"
                | "tools/list"
                | "tools/call"
                | "resources/list"
                | "resources/templates/list"
                | "resources/read"
                | "prompts/list"
                | "prompts/get"
                | "notifications/initialized"
                | "initialized"
                | "notifications/cancelled"
        ) {
            return Ok(());
        }
        let mut req = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if has_id {
            req.as_object_mut().expect("obj").insert(String::from("id"), json!(1));
        }
        let structured_params = matches!(&req["params"], Value::Object(_) | Value::Array(_) | Value::Null);
        let line = format!("{req}\n");
        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve(&mut Cursor::new(line.as_bytes()), &mut output, &mut state).expect("serve");
        if has_id {
            let reply: Value = serde_json::from_slice(&output).expect("json");
            let code = reply["error"]["code"].as_i64().unwrap_or(0);
            prop_assert!(code == ERR_METHOD_NOT_FOUND || code == ERR_INVALID_REQUEST);
        } else if structured_params {
            prop_assert!(output.is_empty());
        } else {
            let reply: Value = serde_json::from_slice(&output).expect("json");
            prop_assert_eq!(reply["error"]["code"].as_i64(), Some(ERR_INVALID_REQUEST));
        }
    }

    #[test]
    fn arbitrary_tool_calls_never_panic(
        tool_idx in 0usize..12,
        random_name in "[a-z_]{1,16}",
        args in arb_json_value(),
    ) {
        let dir = TempDirGuard::new("prop-tools");
        let safe_file = dir.path.join("fuzz_artifact.bin");
        let tools = list_tools();
        let name = tools.get(tool_idx).map_or_else(|| random_name.as_str(), |t| t.name.as_str());
        let safe_args = sanitize_paths_in_tempdir(args, &safe_file);

        let mut state = McpServerState::new();
        let _ = call_tool(&mut state, name, safe_args);
    }

    #[test]
    fn mixed_ndjson_fuzz_followed_by_ping_keeps_connection_alive(
        noise_lines in collection::vec(collection::vec(proptest::arbitrary::any::<u8>().prop_filter("no newline", |&b| b != b'\n'), 1..128), 1..6),
    ) {
        let mut stream = Vec::new();
        for line in &noise_lines {
            // Avoid accidental Content-Length headers that could enter header mode cleanly.
            stream.push(b'{');
            stream.extend_from_slice(line);
            stream.push(b'\n');
        }
        stream.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":999,\"method\":\"ping\"}\n");

        let mut output = Vec::new();
        let mut state = McpServerState::new();
        serve_with_max_bytes(&mut Cursor::new(stream), &mut output, &mut state, 1024).expect("serve");
        let out_str = String::from_utf8(output).expect("utf8");
        let last_line = out_str.lines().last().expect("trailing ping reply");
        let parsed: Value = serde_json::from_str(last_line).expect("json");
        prop_assert_eq!(&parsed["id"], &json!(999));
        prop_assert_eq!(&parsed["result"], &json!({}));
    }
}
