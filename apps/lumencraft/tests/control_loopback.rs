//! Integration tests for LumenCraft `--control` JSON-lines protocol, UI parity commands,
//! hardware clipboard/print adapters, loopback TCP enforcement, and binary CLI semantics.

use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lumencraft_doc::{CellCoord, SheetCellRange, SheetId};
use lumencraft_format::save_project_file;
use lumencraft_ui_egui::{
    ClipboardBundle, PrintJobSpec, UiApp, bind_control_tcp_listener, parse_control_port, serve_control_listener, serve_control_stream,
    validate_loopback_addr,
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

    let mut payload = String::new();
    for item in &requests {
        payload.push_str(&serde_json::to_string(item)?);
        payload.push('\n');
    }
    // Must never be reached after app.quit:
    payload.push_str("{\"id\":\"c-after\",\"method\":\"ping\"}\n");

    let mut reader = BufReader::new(Cursor::new(payload.as_bytes()));
    let mut output = Vec::new();
    serve_control_stream(&mut app, &mut reader, &mut output)?;

    let text = String::from_utf8(output)?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), requests.len(), "stream must exit cleanly on app.quit");

    for (idx, line) in lines.iter().enumerate() {
        let resp: Value = serde_json::from_str(line)?;
        assert_eq!(resp["ok"], json!(true), "step {idx} failed: {resp}");
    }

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

            let client = std::thread::spawn(move || -> Result<Vec<Value>, String> {
                let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
                let mut writer = BufWriter::new(stream);
                let mut reader = BufReader::new(reader_stream);

                let cmds = [
                    r#"{"id":"t1","method":"ping"}"#,
                    r#"{"id":"t2","method":"sheet.createData","params":{"sheet_id":"tcp-1","title":"TCP"}}"#,
                    r#"{"id":"t3","method":"app.quit"}"#,
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
            serve_control_listener(&mut app, &listener)?;
            let responses = client.join().map_err(|_| "client thread panicked")??;
            assert_eq!(responses.len(), 3);
            assert_eq!(responses[0]["ok"], json!(true));
            assert_eq!(responses[1]["ok"], json!(true));
            assert_eq!(responses[2]["result"]["quit"], json!(true));
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

    let client = std::thread::spawn(move || -> Result<Value, String> {
        // First connection sends a line that is not valid UTF-8, which fails that connection.
        {
            let mut bad = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
            bad.write_all(b"\xff\xfe{\"id\":\"bad\"}\n").map_err(|e| e.to_string())?;
            bad.flush().map_err(|e| e.to_string())?;
            let _ = bad.shutdown(std::net::Shutdown::Both);
        }
        // A second connection must still be served.
        let stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut writer = BufWriter::new(stream);
        writer.write_all(b"{\"id\":\"p\",\"method\":\"ping\"}\n").map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).map_err(|e| e.to_string())?;
        writer.write_all(b"{\"id\":\"q\",\"method\":\"app.quit\"}\n").map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    });

    let mut app = UiApp::new();
    serve_control_listener(&mut app, &listener)?;
    let ping = client.join().map_err(|_| "client thread panicked")??;
    assert_eq!(ping["ok"], json!(true));
    Ok(())
}
