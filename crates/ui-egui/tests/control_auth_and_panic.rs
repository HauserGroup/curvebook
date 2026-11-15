//! Regression tests for `R1-S10-02` (HTTP/auth enforcement) and `R1-S10-03` (control panic guard).

use std::io::{BufReader, Cursor};

use lumencraft_ui_egui::{
    CONTROL_HTTP_FORBIDDEN_RESPONSE, UiApp, handle_control_line, is_http_request_line, serve_control_stream_with_quit_and_token,
};
use serde_json::{Value, json};

#[test]
fn http_methods_and_markers_return_403_and_execute_no_body_lines() -> Result<(), Box<dyn std::error::Error>> {
    let methods = ["POST", "GET", "PUT", "DELETE", "HEAD", "OPTIONS", "PATCH", "TRACE", "CONNECT", "post"];
    for method in methods {
        let mut app = UiApp::new();
        let req = format!(
            "{method} / HTTP/1.1\r\nHost: 127.0.0.1:9001\r\nContent-Type: text/plain\r\n\r\n{{\"id\":1,\"method\":\"sheet.createData\",\"token\":\"sample-tok\",\"params\":{{\"sheet_id\":\"evil\"}}}}\n"
        );
        let first_line = req.lines().next().unwrap_or("");
        assert!(is_http_request_line(first_line), "expected HTTP match for {method}");

        let mut reader = BufReader::new(Cursor::new(req.as_bytes()));
        let mut out = Vec::new();
        let quit = serve_control_stream_with_quit_and_token(&mut app, &mut reader, &mut out, "sample-tok")?;
        assert!(!quit);
        assert_eq!(String::from_utf8(out)?, CONTROL_HTTP_FORBIDDEN_RESPONSE);
        assert!(app.document().data_sheets.is_empty(), "{method} HTTP body must never execute");
    }

    assert!(is_http_request_line("CUSTOM /path HTTP/1.0"));
    assert!(is_http_request_line("FOO /bar HTTP/2.0"));
    assert!(!is_http_request_line(r#"{"id":1,"method":"ping"}"#));
    Ok(())
}

#[test]
fn top_level_token_fields_are_stripped_from_strict_command_params() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let payload = concat!(
        "{\"id\":1,\"method\":\"sheet.createData\",\"token\":\"sample-strip-tok\",\"auth_token\":\"sample-strip-tok\",\"params\":{\"sheet_id\":\"s-1\",\"title\":\"Dose\"}}\n",
        "{\"id\":2,\"method\":\"ui.locale.set\",\"token\":\"sample-strip-tok\",\"locale\":\"fr\"}\n",
        "{\"id\":3,\"method\":\"app.quit\",\"token\":\"sample-strip-tok\"}\n"
    );
    let mut reader = BufReader::new(Cursor::new(payload.as_bytes()));
    let mut out = Vec::new();
    let quit = serve_control_stream_with_quit_and_token(&mut app, &mut reader, &mut out, "sample-strip-tok")?;
    assert!(quit);

    let text = String::from_utf8(out)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    for line in lines {
        let parsed: Value = serde_json::from_str(line)?;
        assert_eq!(parsed["ok"], json!(true), "unexpected error: {parsed}");
    }
    assert_eq!(app.locale, "fr");
    assert_eq!(app.document().data_sheets.len(), 1);
    Ok(())
}

#[test]
fn in_process_control_dispatch_catches_small_viewport_panic_without_unwinding() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let palette_resp: Value = serde_json::from_str(&handle_control_line(&mut app, r#"{"id":"d1","method":"ui.palette.open"}"#))?;
    assert_eq!(palette_resp["ok"], json!(true));

    let shot_resp: Value =
        serde_json::from_str(&handle_control_line(&mut app, r#"{"id":"d2","method":"ui.screenshot","params":{"width_px":64,"height_px":64}}"#))?;
    if shot_resp["ok"] == json!(false) {
        assert_eq!(shot_resp["error_detail"]["code"], json!("internal_panic"));
    }

    let ping_resp: Value = serde_json::from_str(&app.dispatch_control_line(r#"{"id":"d3","method":"ping"}"#))?;
    assert_eq!(ping_resp["ok"], json!(true));
    Ok(())
}
