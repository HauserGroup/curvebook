//! Integration tests for LumenCraft `--control` JSON-lines protocol, UI parity commands,
//! hardware clipboard/print adapters, loopback TCP enforcement, and binary CLI semantics.

use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lumencraft_doc::{CellCoord, SheetCellRange, SheetId};
use lumencraft_format::save_project_file;
use lumencraft_ui_egui::{
    CONTROL_HTTP_FORBIDDEN_RESPONSE, ClipboardBundle, ControlAuth, PrintJobSpec, UiApp, bind_control_tcp_listener, parse_control_port,
    serve_control_listener, serve_control_stream, validate_loopback_addr,
};
use serde_json::{Value, json};

#[test]
fn jsonl_control_protocol_end_to_end_over_in_memory_stream() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("lc-ctrl-test-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let shot_path = temp_dir.join("workspace-shot.png");

    let requests = [
        json!({"id": "c-1", "method": "ping"}),
        json!({"id": "c-2", "method": "command.list"}),
        json!({
            "id": "c-3",
            "op": "command.run",
            "command": "sheet.createData",
            "params": {
                "sheet_id": "xy-dose",
                "graph_sheet_id": "graph-dose",
                "title": "Dose Response",
                "kind": "xy",
                "data_sets": 1,
                "rows": 6,
                "auto_graph": true
            }
        }),
        json!({
            "id": "c-4",
            "method": "table.setXCells",
            "params": {"sheet_id": "xy-dose", "values": [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0]}
        }),
        json!({
            "id": "c-5",
            "method": "table.setCells",
            "params": {
                "sheet_id": "xy-dose",
                "rows": [[10.0], [18.0], [45.0], [78.0], [95.0], [99.0]]
            }
        }),
        json!({
            "id": "c-6",
            "method": "fit.run",
            "params": {
                "source_sheet": "xy-dose",
                "sheet_id": "fit-4pl",
                "kind": "four_parameter_logistic"
            }
        }),
        json!({"id": "c-7", "op": "doc.inspect"}),
        json!({"id": "c-8", "op": "doc.inspect", "sheetId": "fit-4pl"}),
        json!({"id": "c-9", "op": "render.svg", "sheetId": "graph-dose"}),
        json!({"id": "c-10", "op": "render.pdf", "sheetId": "graph-dose"}),
        json!({"id": "c-11", "method": "ui.locale.set", "params": {"locale": "fr"}}),
        json!({"id": "c-12", "method": "ui.theme.set", "params": {"mode": "high_contrast"}}),
        json!({"id": "c-13", "method": "ui.navigator.toggle"}),
        json!({"id": "c-14", "method": "ui.inspector.toggle"}),
        json!({"id": "c-15", "method": "ui.family.toggle"}),
        json!({"id": "c-16", "method": "ui.palette.open"}),
        json!({"id": "c-17", "method": "ui.analyze.open", "params": {"sheet_id": "xy-dose"}}),
        json!({"id": "c-18", "method": "ui.changeGraphType.open", "params": {"sheet_id": "graph-dose"}}),
        json!({"id": "c-19", "method": "ui.clonePipeline.open", "params": {"sheet_id": "xy-dose"}}),
        json!({"id": "c-20", "method": "ui.zoom.fitSheet"}),
        json!({"id": "c-21", "method": "app.about"}),
        json!({
            "id": "c-22",
            "method": "ui.screenshot",
            "params": {
                "width_px": 320,
                "height_px": 200,
                "dpi": 144,
                "path": shot_path.to_string_lossy()
            }
        }),
        json!({"id": "c-23", "method": "ui.state"}),
        json!({
            "id": "c-24",
            "method": "command.batch",
            "params": {
                "commands": [
                    {"command": "ui.locale.set", "params": {"locale": "pt-BR"}},
                    {"command": "ui.theme.set", "params": {"mode": "light"}}
                ]
            }
        }),
        json!({"id": "c-25", "op": "app.quit"}),
    ];

    let auth = ControlAuth::from_token("sample-stream-loopback-token")?;
    let mut payload = format!("{{\"id\":\"c-0\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n", auth.as_str());
    for item in &requests {
        payload.push_str(&serde_json::to_string(item)?);
        payload.push('\n');
    }
    // Must never be reached after app.quit:
    payload.push_str("{\"id\":\"c-after\",\"method\":\"ping\"}\n");

    let mut reader = BufReader::new(Cursor::new(payload.as_bytes()));
    let mut output = Vec::new();
    serve_control_stream(&mut app, &mut reader, &mut output, &auth)?;

    let text = String::from_utf8(output)?;
    let all_lines: Vec<&str> = text.lines().collect();
    assert_eq!(all_lines.len(), requests.len() + 1, "stream must exit cleanly on app.quit");

    for (idx, line) in all_lines.iter().enumerate() {
        let resp: Value = serde_json::from_str(line)?;
        assert_eq!(resp["ok"], json!(true), "step {idx} failed: {resp}");
    }
    let lines = &all_lines[1..];

    let cmd_list: Value = serde_json::from_str(lines[1])?;
    let listed = cmd_list["result"]["commands"].as_array().ok_or("commands array missing")?;
    assert!(listed.iter().any(|entry| entry["id"] == "ui.screenshot" && entry["ui"] == true));
    assert!(listed.iter().any(|entry| entry["id"] == "fit.run" && entry["ui"] == false));

    let doc_inspect: Value = serde_json::from_str(lines[6])?;
    assert_eq!(doc_inspect["result"]["counts"]["data"], json!(1));
    assert_eq!(doc_inspect["result"]["counts"]["results"], json!(1));
    assert_eq!(doc_inspect["result"]["counts"]["graphs"], json!(1));

    let svg_resp: Value = serde_json::from_str(lines[8])?;
    assert!(svg_resp["result"]["svg"].as_str().is_some_and(|s| s.contains("<svg")));

    let pdf_resp: Value = serde_json::from_str(lines[9])?;
    assert_eq!(pdf_resp["result"]["encoding"], json!("base64"));
    assert!(pdf_resp["result"]["bytes_len"].as_u64().is_some_and(|n| n > 100));

    let shot_resp: Value = serde_json::from_str(lines[21])?;
    assert_eq!(shot_resp["result"]["width_px"], json!(320));
    assert_eq!(shot_resp["result"]["height_px"], json!(200));
    assert!(shot_resp["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")));
    let disk_png = std::fs::read(&shot_path)?;
    assert!(disk_png.starts_with(b"\x89PNG\r\n\x1a\n"));

    let state_resp: Value = serde_json::from_str(lines[22])?;
    assert_eq!(state_resp["result"]["locale"], json!("fr"));
    assert_eq!(state_resp["result"]["decimal_separator"], json!(","));
    assert_eq!(state_resp["result"]["theme_mode"], json!("high_contrast"));
    assert_eq!(state_resp["result"]["wcag_aa"], json!(true));
    assert_eq!(state_resp["result"]["family_open"], json!(false));

    assert_eq!(app.locale, "pt-BR");

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

#[test]
fn clipboard_bundle_and_print_spool_hardware_adapters() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command(
        "sheet.createData",
        json!({
            "sheet_id": "xy-hw",
            "graph_sheet_id": "graph-hw",
            "title": "Receptor Binding",
            "kind": "xy",
            "data_sets": 1,
            "rows": 4,
            "auto_graph": true
        }),
    )?;
    app.execute_command("table.setXCells", json!({"sheet_id": "xy-hw", "values": [1.0, 2.0, 3.0, 4.0]}))?;
    app.execute_command("table.setCells", json!({"sheet_id": "xy-hw", "rows": [[12.5], [24.0], [36.5], [48.0]]}))?;
    app.execute_command(
        "table.excludeCells",
        json!({
            "sheet_id": "xy-hw",
            "range": {
                "start": {"row": 2, "col": 0, "subcol": 0},
                "end": {"row": 2, "col": 0, "subcol": 0}
            },
            "excluded": true
        }),
    )?;

    let graph_id = SheetId::new("graph-hw");
    let bundle = ClipboardBundle::from_sheet(app.session(), &graph_id, 300)?;
    assert!(bundle.pdf_bytes.starts_with(b"%PDF-"));
    assert!(bundle.svg.contains("<svg"));
    assert!(bundle.html.contains("<!DOCTYPE html>"));
    assert!(bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(bundle.dpi, 300);

    let data_sheet = app.document().data_sheet(&SheetId::new("xy-hw")).ok_or("missing xy-hw sheet")?;
    let tsv_fr = ClipboardBundle::tsv_from_datasheet(data_sheet, ',')?;
    assert!(tsv_fr.contains('\t'));
    assert!(tsv_fr.contains("12,5"));
    assert!(tsv_fr.contains("36,5*"));

    let block = ClipboardBundle::tsv_from_datasheet_range(data_sheet, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0)), '.');
    assert_eq!(block.lines().collect::<Vec<_>>(), vec!["12.5", "24", "36.5*"]);

    let scene = app.session().render_sheet_scene(&graph_id)?;
    let exact_print = PrintJobSpec::publication_100_percent().spool_pdf(app.session())?;
    assert!(exact_print.pdf_bytes.starts_with(b"%PDF-"));
    assert_eq!(exact_print.page_count, 1);
    assert_eq!(exact_print.page_sizes_pt, vec![(scene.width_pt, scene.height_pt)]);

    let bleed_print = PrintJobSpec::publication_100_percent().with_crop_and_bleed(9.0).spool_pdf(app.session())?;
    assert!(bleed_print.pdf_bytes.starts_with(b"%PDF-"));
    let pdf_text = String::from_utf8_lossy(&bleed_print.pdf_bytes);
    assert!(pdf_text.contains("/TrimBox"));
    assert!(pdf_text.contains("/BleedBox"));
    assert!(bleed_print.page_sizes_pt[0].0 > scene.width_pt);
    assert!(bleed_print.page_sizes_pt[0].1 > scene.height_pt);
    Ok(())
}

#[test]
fn loopback_validation_and_ephemeral_tcp_check() -> Result<(), Box<dyn std::error::Error>> {
    assert!(parse_control_port("0").is_err());
    assert!(parse_control_port("65536").is_err());
    assert!(parse_control_port("-1").is_err());
    assert!(parse_control_port("abc").is_err());
    assert_eq!(parse_control_port("1").ok(), Some(1));
    assert_eq!(parse_control_port("9001").ok(), Some(9001));
    assert_eq!(parse_control_port("65535").ok(), Some(65535));

    let non_loopback: SocketAddr = "0.0.0.0:9001".parse()?;
    assert!(validate_loopback_addr(non_loopback).is_err());
    assert!(bind_control_tcp_listener(non_loopback).is_err());

    let lan_ip: SocketAddr = "192.168.1.42:9001".parse()?;
    assert!(validate_loopback_addr(lan_ip).is_err());
    assert!(bind_control_tcp_listener(lan_ip).is_err());

    let loopback_ephemeral = SocketAddr::from(([127, 0, 0, 1], 0));
    match bind_control_tcp_listener(loopback_ephemeral) {
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            // Restricted sandboxes may deny TCP socket bind; in-memory stream test covers full protocol.
        }
        Err(err) => return Err(Box::new(err)),
        Ok(listener) => {
            let local_addr = listener.local_addr()?;
            match TcpStream::connect_timeout(&local_addr, Duration::from_millis(250)) {
                Ok(probe) => {
                    drop(probe);
                    let _ = listener.accept();
                }
                Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
                    return Ok(());
                }
                Err(err) => return Err(Box::new(err)),
            }

            let auth = ControlAuth::generate()?;
            let token_owned = auth.as_str().to_owned();
            let client = std::thread::spawn(move || -> Result<Vec<Value>, String> {
                let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
                let mut writer = BufWriter::new(stream);
                let mut reader = BufReader::new(reader_stream);

                let cmds = [
                    format!(r#"{{"id":"t0","method":"auth","params":{{"token":"{token_owned}"}}}}"#),
                    String::from(r#"{"id":"t1","method":"ping"}"#),
                    String::from(r#"{"id":"t2","method":"sheet.createData","params":{"sheet_id":"tcp-1","title":"TCP"}}"#),
                    String::from(r#"{"id":"t3","method":"app.quit"}"#),
                ];
                let mut responses = Vec::new();
                for cmd in cmds {
                    writer.write_all(cmd.as_bytes()).map_err(|e| e.to_string())?;
                    writer.write_all(b"\n").map_err(|e| e.to_string())?;
                    writer.flush().map_err(|e| e.to_string())?;

                    let mut line = String::new();
                    std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
                    let parsed: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
                    responses.push(parsed);
                }
                Ok(responses)
            });

            let mut app = UiApp::new();
            serve_control_listener(&mut app, &listener, &auth)?;
            let responses = client.join().map_err(|_| "client thread panicked")??;
            assert_eq!(responses.len(), 4);
            assert_eq!(responses[0]["ok"], json!(true));
            assert_eq!(responses[1]["ok"], json!(true));
            assert_eq!(responses[2]["ok"], json!(true));
            assert_eq!(responses[3]["result"]["quit"], json!(true));
            assert_eq!(app.document().data_sheets.len(), 1);
        }
    }
    Ok(())
}

#[test]
fn cli_binary_options_and_headless_startup_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_lumencraft");

    let ver = Command::new(bin).arg("--version").output()?;
    assert!(ver.status.success());
    assert!(String::from_utf8_lossy(&ver.stdout).contains("LumenCraft"));

    let help = Command::new(bin).arg("--help").output()?;
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--control <port>"));

    let bad_port = Command::new(bin).args(["--control", "0"]).output()?;
    assert!(!bad_port.status.success());

    let bad_flag = Command::new(bin).arg("--nonexistent-flag").output()?;
    assert!(!bad_flag.status.success());

    let unique = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("lc-bin-smoke-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let archive_path = temp_dir.join("startup.lumencraft");

    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({"sheet_id": "s1", "title": "Init", "auto_graph": true}))?;
    save_project_file(&archive_path, app.document())?;

    let headless = Command::new(bin).arg("--headless-once").arg(&archive_path).output()?;
    assert!(headless.status.success(), "headless startup failed: {}", String::from_utf8_lossy(&headless.stderr));

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

#[test]
fn bad_control_connection_does_not_stop_the_server() -> Result<(), Box<dyn std::error::Error>> {
    let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        other => other?,
    };
    let local_addr = listener.local_addr()?;
    match TcpStream::connect_timeout(&local_addr, Duration::from_millis(250)) {
        Ok(probe) => {
            drop(probe);
            let _ = listener.accept();
        }
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
            return Ok(());
        }
        Err(err) => return Err(Box::new(err)),
    }

    let auth = ControlAuth::generate()?;
    let token_owned = auth.as_str().to_owned();
    let client = std::thread::spawn(move || -> Result<Value, String> {
        // First connection sends a line that is not valid UTF-8, which fails that connection.
        {
            let mut bad = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            bad.write_all(b"\xff\xfe{\"id\":\"bad\"}\n").map_err(|e| e.to_string())?;
            bad.flush().map_err(|e| e.to_string())?;
            let _ = bad.shutdown(std::net::Shutdown::Both);
        }
        // A second connection must still be served when authenticated.
        let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut writer = BufWriter::new(stream);
        writer.write_all(format!("{{\"id\":\"p\",\"method\":\"ping\",\"token\":\"{token_owned}\"}}\n").as_bytes()).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
        writer.write_all(format!("{{\"id\":\"q\",\"method\":\"app.quit\",\"token\":\"{token_owned}\"}}\n").as_bytes()).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    });

    let mut app = UiApp::new();
    serve_control_listener(&mut app, &listener, &auth)?;
    let ping = client.join().map_err(|_| "client thread panicked")??;
    assert_eq!(ping["ok"], json!(true));
    Ok(())
}

#[test]
fn tcp_control_socket_rejects_unauthenticated_and_browser_http_post_before_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Read;

    let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        other => other?,
    };
    let local_addr = listener.local_addr()?;
    match TcpStream::connect_timeout(&local_addr, Duration::from_millis(250)) {
        Ok(probe) => {
            drop(probe);
            let _ = listener.accept();
        }
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
            return Ok(());
        }
        Err(err) => return Err(Box::new(err)),
    }

    let unique = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("lc-ctrl-http-auth-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let forbidden_shot = temp_dir.join("csrf-shot.png");
    let forbidden_shot_str = forbidden_shot.to_string_lossy().into_owned();

    let auth = ControlAuth::generate()?;
    let token_owned = auth.as_str().to_owned();

    let client = std::thread::spawn(move || -> Result<(Value, Value, String, Value), String> {
        // 1. Raw TCP unauthenticated command must be rejected with "unauthorized".
        let unauth_resp: Value = {
            let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
            let mut writer = BufWriter::new(stream);
            writer
                .write_all(b"{\"id\":\"unauth-1\",\"method\":\"sheet.createData\",\"params\":{\"sheet_id\":\"bad-1\"}}\n")
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())?
        };

        // 2. Wrong token must be rejected with "unauthorized".
        let wrong_resp: Value = {
            let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
            let mut writer = BufWriter::new(stream);
            writer
                .write_all(b"{\"id\":\"wrong-1\",\"method\":\"sheet.createData\",\"token\":\"wrong-secret\",\"params\":{\"sheet_id\":\"bad-2\"}}\n")
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())?
        };

        // 3. Browser-style cross-origin HTTP POST with JSON-RPC body must receive 403 and execute NOTHING.
        let http_reply = {
            let mut stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let body = format!(
                "{{\"id\":\"http-pwn\",\"method\":\"ui.screenshot\",\"params\":{{\"width_px\":320,\"height_px\":200,\"path\":{}}}}}\n",
                serde_json::to_string(&forbidden_shot_str).map_err(|e| e.to_string())?
            );
            let http_req = format!(
                "POST / HTTP/1.1\r\nHost: {local_addr}\r\nOrigin: https://evil.example\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(http_req.as_bytes()).map_err(|e| e.to_string())?;
            stream.flush().map_err(|e| e.to_string())?;
            let mut raw = String::new();
            stream.read_to_string(&mut raw).map_err(|e| e.to_string())?;
            raw
        };

        // 4. Valid token succeeds and shuts server down cleanly.
        let ok_resp: Value = {
            let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
            let mut writer = BufWriter::new(stream);
            writer
                .write_all(
                    format!("{{\"id\":\"ok-1\",\"method\":\"sheet.createData\",\"token\":\"{token_owned}\",\"params\":{{\"sheet_id\":\"good-1\",\"title\":\"Valid\"}}}}\n")
                        .as_bytes(),
                )
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
            writer
                .write_all(format!("{{\"id\":\"quit-1\",\"method\":\"app.quit\",\"token\":\"{token_owned}\"}}\n").as_bytes())
                .map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())?
        };

        Ok((unauth_resp, wrong_resp, http_reply, ok_resp))
    });

    let mut app = UiApp::new();
    serve_control_listener(&mut app, &listener, &auth)?;
    let (unauth_resp, wrong_resp, http_reply, ok_resp) = client.join().map_err(|_| "client thread panicked")??;

    assert_eq!(unauth_resp["ok"], json!(false));
    assert!(unauth_resp["error"].as_str().is_some_and(|e| e.contains("unauthorized")));
    assert_eq!(unauth_resp["error_detail"]["code"], json!("unauthorized"));

    assert_eq!(wrong_resp["ok"], json!(false));
    assert!(wrong_resp["error"].as_str().is_some_and(|e| e.contains("unauthorized")));
    assert_eq!(wrong_resp["error_detail"]["code"], json!("unauthorized"));

    assert_eq!(http_reply, CONTROL_HTTP_FORBIDDEN_RESPONSE);
    assert!(!forbidden_shot.exists(), "HTTP POST attack must never write screenshot file");

    assert_eq!(ok_resp["ok"], json!(true));
    assert_eq!(app.document().data_sheets.len(), 1);
    assert_eq!(app.document().data_sheets[0].id.as_str(), "good-1");

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}

#[test]
fn control_server_survives_small_viewport_dialog_screenshot_and_continues() -> Result<(), Box<dyn std::error::Error>> {
    let listener = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        other => other?,
    };
    let local_addr = listener.local_addr()?;
    match TcpStream::connect_timeout(&local_addr, Duration::from_millis(250)) {
        Ok(probe) => {
            drop(probe);
            let _ = listener.accept();
        }
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
            return Ok(());
        }
        Err(err) => return Err(Box::new(err)),
    }

    let auth = ControlAuth::generate()?;
    let token_owned = auth.as_str().to_owned();
    let client = std::thread::spawn(move || -> Result<(Value, Value, Value), String> {
        let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut writer = BufWriter::new(stream);

        let send = |writer: &mut BufWriter<TcpStream>, reader: &mut BufReader<TcpStream>, payload: &str| -> Result<Value, String> {
            writer.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
            writer.flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            std::io::BufRead::read_line(reader, &mut line).map_err(|e| e.to_string())?;
            serde_json::from_str(&line).map_err(|e| e.to_string())
        };

        let _auth_ok = send(&mut writer, &mut reader, &format!(r#"{{"id":0,"method":"auth","params":{{"token":"{token_owned}"}}}}"#))?;
        let _palette = send(&mut writer, &mut reader, r#"{"id":1,"method":"ui.palette.open"}"#)?;
        let small_shot = send(&mut writer, &mut reader, r#"{"id":2,"method":"ui.screenshot","params":{"width_px":64,"height_px":64}}"#)?;
        let next_ping = send(&mut writer, &mut reader, r#"{"id":3,"method":"ping"}"#)?;
        let quit_resp = send(&mut writer, &mut reader, r#"{"id":4,"method":"app.quit"}"#)?;
        Ok((small_shot, next_ping, quit_resp))
    });

    let mut app = UiApp::new();
    serve_control_listener(&mut app, &listener, &auth)?;
    let (small_shot, next_ping, quit_resp) = client.join().map_err(|_| "client thread panicked")??;
    if small_shot["ok"] == json!(false) {
        assert_eq!(small_shot["error_detail"]["code"], json!("internal_panic"));
    }
    assert_eq!(next_ping["ok"], json!(true));
    assert_eq!(quit_resp["result"]["quit"], json!(true));
    Ok(())
}

#[test]
fn cli_control_token_file_created_0600_enforced_and_cleaned_up_on_quit() -> Result<(), Box<dyn std::error::Error>> {
    let probe = match bind_control_tcp_listener(SocketAddr::from(([127, 0, 0, 1], 0))) {
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        other => other?,
    };
    let port = probe.local_addr()?.port();
    drop(probe);

    let bin = env!("CARGO_BIN_EXE_lumencraft");
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let temp_dir = std::env::temp_dir().join(format!("lc-cli-ctrl-tok-{}-{unique}", std::process::id()));
    std::fs::create_dir_all(&temp_dir)?;
    let token_path = temp_dir.join("lumencraft-data").join(format!("control-{port}.token"));

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let child = Command::new(bin).env("HOME", &temp_dir).args(["--portable-dir"]).arg(&temp_dir).args(["--control", &port.to_string()]).spawn()?;
    let mut guard = ChildGuard(child);

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !token_path.exists() {
        if let Some(status) = guard.0.try_wait()? {
            return Err(format!("lumencraft --control exited early with {status}").into());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for token file `{}`", token_path.display()).into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let token = std::fs::read_to_string(&token_path)?;
    assert_eq!(token.len(), lumencraft_ui_egui::CONTROL_AUTH_TOKEN_BYTES * 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&token_path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "control token file must have 0600 Unix permissions");
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    // Restricted sandboxes may permit bind() in probe while denying outbound connect().
    let first_stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(stream) => stream,
        Err(err) if matches!(err.kind(), ErrorKind::PermissionDenied | ErrorKind::TimedOut) => {
            let _ = guard.0.kill();
            let _ = guard.0.wait();
            let _ = std::fs::remove_dir_all(&temp_dir);
            return Ok(());
        }
        Err(err) => return Err(Box::new(err)),
    };

    // Unauthenticated request rejected.
    {
        let _ = first_stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut r = BufReader::new(first_stream.try_clone()?);
        let mut w = BufWriter::new(first_stream);
        w.write_all(b"{\"id\":\"c1\",\"method\":\"ping\"}\n")?;
        w.flush()?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut r, &mut line)?;
        let resp: Value = serde_json::from_str(&line)?;
        assert_eq!(resp["ok"], json!(false));
        assert!(resp["error"].as_str().is_some_and(|e| e.contains("unauthorized")));
    }

    // Authenticated request succeeds and shuts process down cleanly, cleaning up token file.
    {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut r = BufReader::new(stream.try_clone()?);
        let mut w = BufWriter::new(stream);
        w.write_all(format!("{{\"id\":\"c2\",\"method\":\"ping\",\"token\":\"{token}\"}}\n").as_bytes())?;
        w.flush()?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut r, &mut line)?;
        let ping_resp: Value = serde_json::from_str(&line)?;
        assert_eq!(ping_resp["ok"], json!(true));

        w.write_all(format!("{{\"id\":\"c3\",\"method\":\"app.quit\",\"token\":\"{token}\"}}\n").as_bytes())?;
        w.flush()?;
        line.clear();
        std::io::BufRead::read_line(&mut r, &mut line)?;
        let quit_resp: Value = serde_json::from_str(&line)?;
        assert_eq!(quit_resp["result"]["quit"], json!(true));
    }

    let status = guard.0.wait()?;
    assert!(status.success(), "lumencraft --control exited with {status}");
    assert!(!token_path.exists(), "token file must be removed on clean shutdown");

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}
