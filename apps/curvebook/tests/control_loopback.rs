//! Integration tests for CurveBook `--control` JSON-lines protocol, UI parity commands,
//! hardware clipboard/print adapters, loopback TCP enforcement, and binary CLI semantics.

#[path = "common/mod.rs"]
mod common;
#[path = "support/control_fixture.rs"]
mod control_fixture;

use std::io::{BufReader, BufWriter, Cursor, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

use curvebook_doc::{CellCoord, SheetCellRange, SheetId};
use curvebook_format::{open_project_file, save_project_file};
use curvebook_ui_egui::{
    CONTROL_HTTP_FORBIDDEN_RESPONSE, ClipboardBundle, ControlAuth, PrintJobSpec, UiApp, bind_control_tcp_listener, parse_control_port,
    serve_control_listener, serve_control_stream, validate_loopback_addr,
};
use serde_json::{Value, json};

use common::{
    ChildGuard, TempDirGuard, bind_loopback_probe_listener, reserve_ephemeral_port, send_json_line, tcp_connect_loopback, wait_for_token_file,
};

fn connect_with_timeout(addr: SocketAddr, io_timeout: Duration) -> Result<(BufReader<TcpStream>, BufWriter<TcpStream>), String> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(io_timeout));
    let _ = stream.set_write_timeout(Some(io_timeout));
    let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    Ok((reader, BufWriter::new(stream)))
}

#[test]
fn jsonl_control_protocol_end_to_end_over_in_memory_stream() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    let temp_dir = TempDirGuard::new("ctrl-test")?;
    let shot_path = temp_dir.child("workspace-shot.png");

    let requests = [
        json!({"id": "c-1", "method": "ping"}),
        json!({"id": "c-2", "method": "command.list"}),
        json!({"id": "c-3", "op": "command.run", "command": "sheet.createData", "params": {"sheet_id": "xy-dose", "graph_sheet_id": "graph-dose", "title": "Dose Response", "kind": "xy", "cols": 1, "rows": 6, "auto_graph": true}}),
        json!({"id": "c-4", "method": "table.setXCells", "params": {"sheet_id": "xy-dose", "values": [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0]}}),
        json!({"id": "c-5", "method": "table.setCells", "params": {"sheet_id": "xy-dose", "rows": [[10.0], [18.0], [45.0], [78.0], [95.0], [99.0]]}}),
        json!({"id": "c-6", "method": "fit.run", "params": {"source_sheet_id": "xy-dose", "sheet_id": "fit-4pl", "kind": "four_parameter_logistic"}}),
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
        json!({"id": "c-22", "method": "ui.screenshot", "params": {"width_px": 320, "height_px": 200, "dpi": 144, "path": shot_path.to_string_lossy()}}),
        json!({"id": "c-23", "method": "ui.state"}),
        json!({"id": "c-24", "method": "command.batch", "params": {"commands": [{"command": "ui.locale.set", "params": {"locale": "pt-BR"}}, {"command": "ui.theme.set", "params": {"mode": "light"}}]}}),
        json!({"id": "c-25", "op": "app.quit"}),
    ];

    let auth = ControlAuth::from_token("sample-stream-loopback-token")?;
    let mut payload = format!("{{\"id\":\"c-0\",\"method\":\"auth\",\"params\":{{\"token\":\"{}\"}}}}\n", auth.as_str());
    for item in &requests {
        payload.push_str(&serde_json::to_string(item)?);
        payload.push('\n');
    }
    payload.push_str("{\"id\":\"c-after\",\"method\":\"ping\"}\n");

    let mut output = Vec::new();
    serve_control_stream(&mut app, &mut BufReader::new(Cursor::new(payload.as_bytes())), &mut output, &auth)?;

    let parsed: Vec<Value> = String::from_utf8(output)?.lines().map(serde_json::from_str).collect::<Result<_, _>>()?;
    assert_eq!(parsed.len(), requests.len() + 1, "stream must exit cleanly on app.quit");
    assert!(parsed.iter().all(|r| r["ok"] == json!(true)));
    let replies = &parsed[1..];

    let listed = replies[1]["result"]["commands"].as_array().ok_or("commands array missing")?;
    assert!(listed.iter().any(|e| e["id"] == "ui.screenshot" && e["ui"] == true));
    assert!(listed.iter().any(|e| e["id"] == "fit.run" && e["ui"] == false));
    assert_eq!(replies[6]["result"]["counts"], json!({"data": 1, "results": 1, "graphs": 1, "layouts": 0, "info": 0, "total": 3}));
    assert!(replies[8]["result"]["svg"].as_str().is_some_and(|s| s.contains("<svg")));
    assert_eq!(replies[9]["result"]["encoding"], json!("base64"));
    assert!(replies[9]["result"]["bytes_len"].as_u64().is_some_and(|n| n > 100));
    assert_eq!((&replies[21]["result"]["width_px"], &replies[21]["result"]["height_px"]), (&json!(320), &json!(200)));
    assert!(replies[21]["result"]["data_base64"].as_str().is_some_and(|b| b.starts_with("iVBORw0K")));
    assert!(std::fs::read(&shot_path)?.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(
        (
            &replies[22]["result"]["locale"],
            &replies[22]["result"]["decimal_separator"],
            &replies[22]["result"]["theme_mode"],
            &replies[22]["result"]["wcag_aa"],
            &replies[22]["result"]["family_open"]
        ),
        (&json!("fr"), &json!(","), &json!("high_contrast"), &json!(true), &json!(false))
    );
    assert_eq!(app.locale, "pt-BR");
    Ok(())
}

#[test]
fn clipboard_bundle_and_print_spool_hardware_adapters() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({"sheet_id": "xy-hw", "graph_sheet_id": "graph-hw", "title": "Receptor Binding", "kind": "xy", "cols": 1, "rows": 4, "auto_graph": true}))?;
    app.execute_command("table.setXCells", json!({"sheet_id": "xy-hw", "values": [1.0, 2.0, 3.0, 4.0]}))?;
    app.execute_command("table.setCells", json!({"sheet_id": "xy-hw", "rows": [[12.5], [24.0], [36.5], [48.0]]}))?;
    app.execute_command("table.excludeCells", json!({"sheet_id": "xy-hw", "range": {"start": {"row": 2, "col": 0, "subcol": 0}, "end": {"row": 2, "col": 0, "subcol": 0}}, "excluded": true}))?;

    let graph_id = SheetId::new("graph-hw");
    let bundle = ClipboardBundle::from_sheet(app.session(), &graph_id, 300)?;
    assert!(
        bundle.pdf_bytes.starts_with(b"%PDF-")
            && bundle.svg.contains("<svg")
            && bundle.html.contains("<!DOCTYPE html>")
            && bundle.png_bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            && bundle.dpi == 300
    );

    let data_sheet = app.document().data_sheet(&SheetId::new("xy-hw")).ok_or("missing xy-hw sheet")?;
    let tsv_fr = ClipboardBundle::tsv_from_datasheet(data_sheet, ',')?;
    assert!(tsv_fr.contains('\t') && tsv_fr.contains("12,5") && tsv_fr.contains("36,5*"));
    let block = ClipboardBundle::tsv_from_datasheet_range(data_sheet, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 0, 0)), '.');
    assert_eq!(block.lines().collect::<Vec<_>>(), vec!["12.5", "24", "36.5*"]);

    let scene = app.session().render_sheet_scene(&graph_id)?;
    let exact_print = PrintJobSpec::publication_100_percent().spool_pdf(app.session())?;
    assert!(
        exact_print.pdf_bytes.starts_with(b"%PDF-")
            && exact_print.page_count == 1
            && exact_print.page_sizes_pt == vec![(scene.width_pt, scene.height_pt)]
    );

    let bleed_print = PrintJobSpec::publication_100_percent().with_crop_and_bleed(9.0).spool_pdf(app.session())?;
    let pdf_text = String::from_utf8_lossy(&bleed_print.pdf_bytes);
    assert!(bleed_print.pdf_bytes.starts_with(b"%PDF-") && pdf_text.contains("/TrimBox") && pdf_text.contains("/BleedBox"));
    assert!(bleed_print.page_sizes_pt[0].0 > scene.width_pt && bleed_print.page_sizes_pt[0].1 > scene.height_pt);
    Ok(())
}

#[test]
fn loopback_validation_and_ephemeral_tcp_check() -> Result<(), Box<dyn std::error::Error>> {
    control_fixture::run("loopback_validation_and_ephemeral_tcp_check", || {
        for bad in ["0", "65536", "-1", "abc"] {
            assert!(parse_control_port(bad).is_err());
        }
        for (raw, port) in [("1", 1), ("9001", 9001), ("65535", 65535)] {
            assert_eq!(parse_control_port(raw).ok(), Some(port));
        }
        for bad_addr in ["0.0.0.0:9001", "192.168.1.42:9001", "[::]:9125"] {
            let addr: SocketAddr = bad_addr.parse()?;
            assert!(validate_loopback_addr(addr).is_err() && bind_control_tcp_listener(addr).is_err());
        }
        for good_addr in ["127.0.0.1:0", "[::1]:0"] {
            assert!(validate_loopback_addr(good_addr.parse()?).is_ok());
        }

        let Some((listener, local_addr)) = bind_loopback_probe_listener()? else {
            return Ok(());
        };
        let auth = ControlAuth::generate()?;
        let token_owned = auth.as_str().to_owned();
        let mut app = UiApp::new();
        let client = std::thread::spawn(move || -> Result<Vec<Value>, String> {
            let mut cleanup = control_fixture::QuitOnFailure::new(local_addr, &token_owned);
            let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(2))?;
            let cmds = [
                json!({"id": "t0", "method": "auth", "params": {"token": token_owned}}),
                json!({"id": "t1", "method": "ping"}),
                json!({"id": "t2", "method": "sheet.createData", "params": {"sheet_id": "tcp-1", "title": "TCP"}}),
                json!({"id": "t3", "method": "app.quit"}),
            ];
            let mut out = Vec::with_capacity(cmds.len());
            for cmd in cmds {
                out.push(send_json_line(&mut w, &mut r, &cmd).map_err(|e| e.to_string())?);
            }
            cleanup.disarm();
            Ok(out)
        });
        serve_control_listener(&mut app, &listener, &auth)?;
        let resps = client.join().map_err(|_| "client thread panicked")??;
        assert_eq!(resps.len(), 4);
        assert!(resps.iter().all(|r| r["ok"] == json!(true)) && resps[3].pointer("/result/quit") == Some(&json!(true)));
        assert_eq!(app.document().data_sheets.len(), 1);
        Ok(())
    })
}

#[test]
fn cli_binary_options_and_headless_startup_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let ver = Command::new(bin).arg("--version").output()?;
    assert!(ver.status.success() && String::from_utf8_lossy(&ver.stdout).contains("CurveBook"));

    let help = Command::new(bin).arg("--help").output()?;
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.status.success()
            && help_text.contains("--control <port>")
            && help_text.contains("--window")
            && help_text.contains("CURVEBOOK_CONTROL_PORT")
    );

    for bad_args in [
        vec!["--control", "0"],
        vec!["--nonexistent-flag"],
        vec!["--control", "9125", "--window", "--headless-once"],
        vec!["--window", "--headless-once"],
        vec!["--control", "0", "--window"],
    ] {
        assert!(!Command::new(bin).args(&bad_args).output()?.status.success(), "{bad_args:?} must fail");
    }
    assert!(!Command::new(bin).env("CURVEBOOK_CONTROL_PORT", "9125").args(["--window", "--headless-once"]).output()?.status.success());

    let temp_dir = TempDirGuard::new("bin-smoke")?;
    let archive_path = temp_dir.child("startup.curvebook");
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", json!({"sheet_id": "s1", "title": "Init", "auto_graph": true}))?;
    save_project_file(&archive_path, app.document())?;

    let headless = Command::new(bin).arg("--headless-once").arg(&archive_path).output()?;
    assert!(headless.status.success(), "headless startup failed: {}", String::from_utf8_lossy(&headless.stderr));
    Ok(())
}

#[test]
fn bad_control_connection_does_not_stop_the_server() -> Result<(), Box<dyn std::error::Error>> {
    control_fixture::run("bad_control_connection_does_not_stop_the_server", || {
        let Some((listener, local_addr)) = bind_loopback_probe_listener()? else {
            return Ok(());
        };
        let auth = ControlAuth::generate()?;
        let token_owned = auth.as_str().to_owned();
        let mut app = UiApp::new();
        let client = std::thread::spawn(move || -> Result<Value, String> {
            let mut cleanup = control_fixture::QuitOnFailure::new(local_addr, &token_owned);
            {
                let mut bad = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                bad.write_all(b"\xff\xfe{\"id\":\"bad\"}\n").map_err(|e| e.to_string())?;
                bad.flush().map_err(|e| e.to_string())?;
                let _ = bad.shutdown(std::net::Shutdown::Both);
            }
            let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(5))?;
            let ping = send_json_line(&mut w, &mut r, &json!({"id": "p", "method": "ping", "token": token_owned})).map_err(|e| e.to_string())?;
            let quit = send_json_line(&mut w, &mut r, &json!({"id": "q", "method": "app.quit", "token": token_owned})).map_err(|e| e.to_string())?;
            if quit.pointer("/result/quit") != Some(&json!(true)) {
                return Err(format!("app.quit not acknowledged: {quit}"));
            }
            cleanup.disarm();
            Ok(ping)
        });
        serve_control_listener(&mut app, &listener, &auth)?;
        assert_eq!(client.join().map_err(|_| "client thread panicked")??["ok"], json!(true));
        Ok(())
    })
}

#[test]
fn tcp_control_socket_rejects_unauthenticated_and_browser_http_post_before_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    control_fixture::run("tcp_control_socket_rejects_unauthenticated_and_browser_http_post_before_dispatch", || {
        let Some((listener, local_addr)) = bind_loopback_probe_listener()? else {
            return Ok(());
        };
        let temp_dir = TempDirGuard::new("ctrl-http-auth")?;
        let forbidden_shot = temp_dir.child("csrf-shot.png");
        let forbidden_shot_str = forbidden_shot.to_string_lossy().into_owned();
        let auth = ControlAuth::generate()?;
        let token_owned = auth.as_str().to_owned();
        let mut app = UiApp::new();

        let client = std::thread::spawn(move || -> Result<(Value, Value, String, Value), String> {
            let mut cleanup = control_fixture::QuitOnFailure::new(local_addr, &token_owned);
            let unauth_resp = {
                let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(2))?;
                send_json_line(&mut w, &mut r, &json!({"id": "unauth-1", "method": "sheet.createData", "params": {"sheet_id": "bad-1"}}))
                    .map_err(|e| e.to_string())?
            };
            let wrong_resp = {
                let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(2))?;
                send_json_line(
                    &mut w,
                    &mut r,
                    &json!({"id": "wrong-1", "method": "sheet.createData", "token": "wrong-secret", "params": {"sheet_id": "bad-2"}}),
                )
                .map_err(|e| e.to_string())?
            };
            let http_reply = {
                let mut stream = TcpStream::connect_timeout(&local_addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let body = format!(
                    "{{\"id\":\"http-pwn\",\"method\":\"ui.screenshot\",\"params\":{{\"width_px\":320,\"height_px\":200,\"path\":{}}}}}\n",
                    serde_json::to_string(&forbidden_shot_str).map_err(|e| e.to_string())?
                );
                let http_req = format!(
                    "POST / HTTP/1.1\r\nHost: {local_addr}\r\nOrigin: https://evil.example\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(http_req.as_bytes()).map_err(|e| e.to_string())?;
                stream.flush().map_err(|e| e.to_string())?;
                let mut raw = String::new();
                match stream.read_to_string(&mut raw) {
                    Ok(_) => {}
                    Err(err) if err.kind() == ErrorKind::ConnectionReset && raw == CONTROL_HTTP_FORBIDDEN_RESPONSE => {}
                    Err(err) => return Err(err.to_string()),
                }
                raw
            };
            let ok_resp = {
                let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(2))?;
                let resp = send_json_line(
                    &mut w,
                    &mut r,
                    &json!({"id": "ok-1", "method": "sheet.createData", "token": token_owned, "params": {"sheet_id": "good-1", "title": "Valid"}}),
                )
                .map_err(|e| e.to_string())?;
                let quit = send_json_line(&mut w, &mut r, &json!({"id": "quit-1", "method": "app.quit", "token": token_owned}))
                    .map_err(|e| e.to_string())?;
                if quit.pointer("/result/quit") != Some(&json!(true)) {
                    return Err(format!("app.quit not acknowledged: {quit}"));
                }
                resp
            };
            cleanup.disarm();
            Ok((unauth_resp, wrong_resp, http_reply, ok_resp))
        });

        serve_control_listener(&mut app, &listener, &auth)?;
        let (unauth_resp, wrong_resp, http_reply, ok_resp) = client.join().map_err(|_| "client thread panicked")??;
        for bad in [&unauth_resp, &wrong_resp] {
            assert_eq!((&bad["ok"], bad.pointer("/error_detail/code")), (&json!(false), Some(&json!("unauthorized"))));
            assert!(bad["error"].as_str().is_some_and(|e| e.contains("unauthorized")));
        }
        assert_eq!(http_reply, CONTROL_HTTP_FORBIDDEN_RESPONSE);
        assert!(!forbidden_shot.exists() && ok_resp["ok"] == json!(true));
        assert_eq!(app.document().data_sheets.first().map(|s| s.id.as_str()), Some("good-1"));
        Ok(())
    })
}

#[test]
fn control_server_survives_small_viewport_dialog_screenshot_and_continues() -> Result<(), Box<dyn std::error::Error>> {
    control_fixture::run("control_server_survives_small_viewport_dialog_screenshot_and_continues", || {
        let Some((listener, local_addr)) = bind_loopback_probe_listener()? else {
            return Ok(());
        };
        let auth = ControlAuth::generate()?;
        let token_owned = auth.as_str().to_owned();
        let mut app = UiApp::new();
        let client = std::thread::spawn(move || -> Result<(Value, Value, Value), String> {
            let mut cleanup = control_fixture::QuitOnFailure::new(local_addr, &token_owned);
            let (mut r, mut w) = connect_with_timeout(local_addr, Duration::from_secs(5))?;
            let _ =
                send_json_line(&mut w, &mut r, &json!({"id": 0, "method": "auth", "params": {"token": token_owned}})).map_err(|e| e.to_string())?;
            let _ = send_json_line(&mut w, &mut r, &json!({"id": 1, "method": "ui.palette.open"})).map_err(|e| e.to_string())?;
            let small_shot =
                send_json_line(&mut w, &mut r, &json!({"id": 2, "method": "ui.screenshot", "params": {"width_px": 64, "height_px": 64}}))
                    .map_err(|e| e.to_string())?;
            let next_ping = send_json_line(&mut w, &mut r, &json!({"id": 3, "method": "ping"})).map_err(|e| e.to_string())?;
            let quit_resp = send_json_line(&mut w, &mut r, &json!({"id": 4, "method": "app.quit"})).map_err(|e| e.to_string())?;
            cleanup.disarm();
            Ok((small_shot, next_ping, quit_resp))
        });

        serve_control_listener(&mut app, &listener, &auth)?;
        let (small_shot, next_ping, quit_resp) = client.join().map_err(|_| "client thread panicked")??;
        if small_shot.get("ok") == Some(&json!(false)) {
            assert_eq!(small_shot.pointer("/error_detail/code"), Some(&json!("internal_panic")));
        }
        assert_eq!((&next_ping["ok"], quit_resp.pointer("/result/quit")), (&json!(true), Some(&json!(true))));
        Ok(())
    })
}

#[test]
fn cli_control_token_file_created_0600_enforced_and_cleaned_up_on_quit() -> Result<(), Box<dyn std::error::Error>> {
    let Some(port) = reserve_ephemeral_port()? else {
        return Ok(());
    };
    let bin = env!("CARGO_BIN_EXE_curvebook");
    let temp_dir = TempDirGuard::new("cli-ctrl-tok")?;
    let token_path = temp_dir.child("curvebook-data").join(format!("control-{port}.token"));
    let save_path = temp_dir.child("headless-saved.curvebook");

    let mut guard = ChildGuard(
        Command::new(bin)
            .env("HOME", temp_dir.path())
            .args(["--portable-dir"])
            .arg(temp_dir.path())
            .args(["--control", &port.to_string()])
            .spawn()?,
    );
    let token = wait_for_token_file(&mut guard.0, &token_path, Duration::from_secs(10))?;
    assert_eq!(token.len(), curvebook_ui_egui::CONTROL_AUTH_TOKEN_BYTES * 2);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&token_path)?.permissions().mode() & 0o777, 0o600);
    }

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Some((mut r1, mut w1)) = tcp_connect_loopback(addr, Duration::from_secs(2))? else {
        return Ok(());
    };
    let unauth = send_json_line(&mut w1, &mut r1, &json!({"id": "c1", "method": "ping"}))?;
    assert_eq!(unauth["ok"], json!(false));
    assert!(unauth["error"].as_str().is_some_and(|e| e.contains("unauthorized")));
    drop((r1, w1));

    let (mut r2, mut w2) = tcp_connect_loopback(addr, Duration::from_secs(5))?.ok_or("loopback connect denied")?;
    for req in [
        json!({"id": "c2", "method": "auth", "params": {"token": token}}),
        json!({"id": "c3", "method": "ping"}),
        json!({"id": "c4", "method": "sheet.createData", "params": {"sheet_id": "hl-1", "title": "Headless", "kind": "column", "cols": 1, "rows": 2}}),
        json!({"id": "c5", "method": "table.setCell", "params": {"sheet_id": "hl-1", "row": 0, "col": 0, "value": 42.5}}),
        json!({"id": "c6", "method": "project.save", "params": {"path": save_path.display().to_string()}}),
    ] {
        assert_eq!(send_json_line(&mut w2, &mut r2, &req)?["ok"], json!(true));
    }
    assert_eq!(send_json_line(&mut w2, &mut r2, &json!({"id": "c7", "method": "app.quit"}))?["result"]["quit"], json!(true));
    drop((r2, w2));

    assert!(guard.0.wait()?.success() && !token_path.exists());
    let doc = open_project_file(&save_path)?;
    assert_eq!(doc.data_sheet(&SheetId::new("hl-1")).and_then(|d| d.cell(CellCoord::new(0, 0, 0))).and_then(|c| c.active_number()), Some(42.5));

    // Also verify CURVEBOOK_CONTROL_PORT without --window runs headlessly and cleans up token.
    if let Some(env_port) = reserve_ephemeral_port()? {
        let env_tok_path = temp_dir.child("curvebook-data").join(format!("control-{env_port}.token"));
        let mut env_guard = ChildGuard(
            Command::new(bin)
                .env("HOME", temp_dir.path())
                .env("CURVEBOOK_CONTROL_PORT", env_port.to_string())
                .args(["--portable-dir"])
                .arg(temp_dir.path())
                .spawn()?,
        );
        let env_tok = wait_for_token_file(&mut env_guard.0, &env_tok_path, Duration::from_secs(10))?;
        let (mut er, mut ew) = tcp_connect_loopback(SocketAddr::from(([127, 0, 0, 1], env_port)), Duration::from_secs(5))?.ok_or("connect denied")?;
        assert_eq!(send_json_line(&mut ew, &mut er, &json!({"id": "eq", "method": "app.quit", "token": env_tok}))?["result"]["quit"], json!(true));
        assert!(env_guard.0.wait()?.success() && !env_tok_path.exists());
    }
    Ok(())
}

#[test]
fn tcp_fixture_client_error_and_panic_both_stop_the_listener() -> control_fixture::TestResult {
    control_fixture::run("tcp_fixture_client_error_and_panic_both_stop_the_listener", || {
        for panic_client in [false, true] {
            let Some((listener, address)) = bind_loopback_probe_listener()? else {
                return Ok(());
            };
            let auth = ControlAuth::generate()?;
            let token = auth.as_str().to_owned();
            let mut app = UiApp::new();
            let client = std::thread::spawn(move || -> Result<(), String> {
                let _cleanup = control_fixture::QuitOnFailure::new(address, &token);
                if panic_client {
                    panic!("injected TCP fixture client panic");
                }
                Err(String::from("injected TCP fixture client error"))
            });
            serve_control_listener(&mut app, &listener, &auth)?;
            let outcome = client.join();
            if panic_client {
                assert!(outcome.is_err());
            } else {
                assert_eq!(outcome.unwrap(), Err(String::from("injected TCP fixture client error")));
            }
        }
        Ok(())
    })
}

#[test]
fn tcp_fixture_deadline_kills_a_stalled_child() {
    const NAME: &str = "tcp_fixture_deadline_kills_a_stalled_child";
    if control_fixture::is_child(NAME) {
        loop {
            std::thread::park();
        }
    }
    let error = control_fixture::run_child(NAME, Duration::from_millis(250)).expect_err("a stalled child must time out");
    assert!(matches!(error.downcast_ref::<curvebook_platform::PlatformError>(), Some(curvebook_platform::PlatformError::TimedOut { .. })));
}
