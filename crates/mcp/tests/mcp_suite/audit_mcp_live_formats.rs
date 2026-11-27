//! Integration tests for MCP format parity, live-attach `doc_inspect` enrichment,
//! live-attach `command_batch` delegation, direct desktop `doc.export` writes,
//! and control-channel auth response ID validation.

use std::fs;
use std::sync::Arc;

use curvebook_doc::{CellCoord, CellValue, SheetId};
use curvebook_engine::{Caller, Session};
use curvebook_format::{ZipWriter, save_project_file};
use curvebook_mcp::{ContentBlock, McpServerState, call_tool, list_tools};
use curvebook_platform::sync::Mutex;
use serde_json::{Value, json};

use super::common::{TempDir, parse_result_json};

type TestResult<T = ()> = Result<T, String>;

fn call_ok_json(state: &mut McpServerState, name: &str, args: Value) -> TestResult<Value> {
    let result = call_tool(state, name, args).map_err(|err| format!("{name} protocol error: {err:?}"))?;
    if result.is_error {
        return Err(format!("expected {name} to succeed, got error: {:?}", result.structured_content));
    }
    parse_result_json(&result)
}

fn call_err_json(state: &mut McpServerState, name: &str, args: Value) -> TestResult<Value> {
    let result = call_tool(state, name, args).map_err(|err| format!("{name} protocol error: {err:?}"))?;
    if !result.is_error {
        return Err(format!("expected {name} to fail, got ok: {:?}", result.structured_content));
    }
    parse_result_json(&result)
}

fn read_y_cell(state: &McpServerState, sheet_id: &str, row: usize, col: usize) -> Option<f64> {
    let sheet = state.session().document().data_sheet(&SheetId::new(sheet_id))?;
    let cell = sheet.archetype.data_cell(CellCoord::new(row, col, 0))?;
    match cell.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

fn push_u16_be(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16_be(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32_be(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn write_bytes_at(buf: &mut [u8], offset: usize, src: &[u8]) {
    if let Some(slot) = buf.get_mut(offset..offset.saturating_add(src.len())) {
        slot.copy_from_slice(src);
    }
}

fn build_test_glyf_loca(num_glyphs: u16) -> (Vec<u8>, Vec<u8>) {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for gid in 0..num_glyphs {
        push_u16_be(&mut loca, u16::try_from(glyf.len() / 2).unwrap_or(u16::MAX));
        if gid != 1 {
            push_i16_be(&mut glyf, 1);
            push_i16_be(&mut glyf, 50);
            push_i16_be(&mut glyf, 0);
            push_i16_be(&mut glyf, 500);
            push_i16_be(&mut glyf, 700);
            push_u16_be(&mut glyf, 3);
            push_u16_be(&mut glyf, 0);
            glyf.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
            for dx in [50i16, 0, 450, 0] {
                push_i16_be(&mut glyf, dx);
            }
            for dy in [0i16, 700, 0, -700] {
                push_i16_be(&mut glyf, dy);
            }
            if !glyf.len().is_multiple_of(2) {
                glyf.push(0);
            }
        }
    }
    push_u16_be(&mut loca, u16::try_from(glyf.len() / 2).unwrap_or(u16::MAX));
    (glyf, loca)
}

struct TestMetricsTables {
    hmtx: Vec<u8>,
    head: Vec<u8>,
    hhea: Vec<u8>,
    maxp: Vec<u8>,
    os2: Vec<u8>,
}

fn build_test_metrics_tables(num_glyphs: u16) -> TestMetricsTables {
    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    for _ in 0..num_glyphs {
        push_u16_be(&mut hmtx, 556);
        push_i16_be(&mut hmtx, 50);
    }

    let mut head = vec![0u8; 54];
    write_bytes_at(&mut head, 0, &0x0001_0000u32.to_be_bytes());
    write_bytes_at(&mut head, 4, &0x0001_0000u32.to_be_bytes());
    write_bytes_at(&mut head, 12, &0x5F0F_3CF5u32.to_be_bytes());
    write_bytes_at(&mut head, 16, &0x000Bu16.to_be_bytes());
    write_bytes_at(&mut head, 18, &1000u16.to_be_bytes());
    write_bytes_at(&mut head, 36, &[0, 50, 0, 0, 1, 244, 2, 188]);
    write_bytes_at(&mut head, 46, &8u16.to_be_bytes());
    write_bytes_at(&mut head, 48, &2i16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    write_bytes_at(&mut hhea, 0, &0x0001_0000u32.to_be_bytes());
    write_bytes_at(&mut hhea, 4, &800i16.to_be_bytes());
    write_bytes_at(&mut hhea, 6, &(-200i16).to_be_bytes());
    write_bytes_at(&mut hhea, 8, &100i16.to_be_bytes());
    write_bytes_at(&mut hhea, 10, &556u16.to_be_bytes());
    write_bytes_at(&mut hhea, 18, &1i16.to_be_bytes());
    write_bytes_at(&mut hhea, 34, &num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 32];
    write_bytes_at(&mut maxp, 0, &0x0001_0000u32.to_be_bytes());
    write_bytes_at(&mut maxp, 4, &num_glyphs.to_be_bytes());
    write_bytes_at(&mut maxp, 6, &4u16.to_be_bytes());
    write_bytes_at(&mut maxp, 8, &1u16.to_be_bytes());

    let mut os2 = vec![0u8; 96];
    write_bytes_at(&mut os2, 0, &4u16.to_be_bytes());
    write_bytes_at(&mut os2, 2, &556i16.to_be_bytes());
    write_bytes_at(&mut os2, 4, &400u16.to_be_bytes());
    write_bytes_at(&mut os2, 6, &5u16.to_be_bytes());
    write_bytes_at(&mut os2, 62, &0x0040u16.to_be_bytes());
    write_bytes_at(&mut os2, 64, &32u16.to_be_bytes());
    write_bytes_at(&mut os2, 66, &126u16.to_be_bytes());
    write_bytes_at(&mut os2, 68, &800i16.to_be_bytes());
    write_bytes_at(&mut os2, 70, &(-200i16).to_be_bytes());
    write_bytes_at(&mut os2, 88, &700i16.to_be_bytes());

    TestMetricsTables { hmtx, head, hhea, maxp, os2 }
}

fn build_test_cmap_and_name(ps_name: &str) -> (Vec<u8>, Vec<u8>) {
    let segments: [(u16, u16, i16); 2] = [(32, 126, -31), (0xFFFF, 0xFFFF, 1)];
    let mut sub = Vec::new();
    push_u16_be(&mut sub, 4);
    push_u16_be(&mut sub, 32);
    push_u16_be(&mut sub, 0);
    push_u16_be(&mut sub, 4);
    push_u16_be(&mut sub, 4);
    push_u16_be(&mut sub, 1);
    push_u16_be(&mut sub, 0);
    for &(_, end, _) in &segments {
        push_u16_be(&mut sub, end);
    }
    push_u16_be(&mut sub, 0);
    for &(start, _, _) in &segments {
        push_u16_be(&mut sub, start);
    }
    for &(_, _, delta) in &segments {
        push_i16_be(&mut sub, delta);
    }
    for _ in 0..segments.len() {
        push_u16_be(&mut sub, 0);
    }
    let mut cmap = Vec::new();
    push_u16_be(&mut cmap, 0);
    push_u16_be(&mut cmap, 1);
    push_u16_be(&mut cmap, 3);
    push_u16_be(&mut cmap, 1);
    push_u32_be(&mut cmap, 12);
    cmap.extend_from_slice(&sub);

    let entries: [(u16, &str); 2] = [(1, "AuditEmbed"), (6, ps_name)];
    let mut str_buf = Vec::new();
    let mut records = Vec::new();
    for (name_id, text) in entries {
        let start = u16::try_from(str_buf.len()).unwrap_or(0);
        for unit in text.encode_utf16() {
            push_u16_be(&mut str_buf, unit);
        }
        let len = u16::try_from(str_buf.len()).unwrap_or(0).saturating_sub(start);
        records.push((name_id, len, start));
    }
    let str_off = 6u16 + u16::try_from(records.len()).unwrap_or(0) * 12;
    let mut name = Vec::new();
    push_u16_be(&mut name, 0);
    push_u16_be(&mut name, u16::try_from(records.len()).unwrap_or(0));
    push_u16_be(&mut name, str_off);
    for (name_id, len, off) in records {
        push_u16_be(&mut name, 3);
        push_u16_be(&mut name, 1);
        push_u16_be(&mut name, 0x0409);
        push_u16_be(&mut name, name_id);
        push_u16_be(&mut name, len);
        push_u16_be(&mut name, off);
    }
    name.extend_from_slice(&str_buf);
    (cmap, name)
}

/// Build a minimal valid TrueType SFNT (`0x00010000`) covering printable ASCII (`32..=126`)
/// with a custom PostScript font name for deterministic PDF embedding tests.
fn build_minimal_test_ttf(ps_name: &str) -> Vec<u8> {
    let num_glyphs: u16 = 96;
    let (glyf, loca) = build_test_glyf_loca(num_glyphs);
    let TestMetricsTables { hmtx, head, hhea, maxp, os2 } = build_test_metrics_tables(num_glyphs);
    let (cmap, name) = build_test_cmap_and_name(ps_name);
    let mut post = vec![0u8; 32];
    write_bytes_at(&mut post, 0, &0x0003_0000u32.to_be_bytes());

    let tables: [([u8; 4], Vec<u8>); 10] = [
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];

    let num_tables = u16::try_from(tables.len()).unwrap_or(10);
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32_be(&mut font, 0x0001_0000);
    push_u16_be(&mut font, num_tables);
    push_u16_be(&mut font, 128);
    push_u16_be(&mut font, 3);
    push_u16_be(&mut font, num_tables * 16 - 128);

    let mut cur_offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32_be(&mut font, 0);
        push_u32_be(&mut font, u32::try_from(cur_offset).unwrap_or(0));
        push_u32_be(&mut font, u32::try_from(data.len()).unwrap_or(0));
        cur_offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }
    font
}

#[test]
fn schema_open_file_docs_url_append_and_export_parity() -> TestResult {
    let tools = list_tools();
    let open_file = tools.iter().find(|t| t.name == "open_file").ok_or("missing open_file tool")?;
    let expected_open_docs =
        format!("manual/v{}.{}/registry/mcp-tools.html#mcp-tool-open_file", env!("CARGO_PKG_VERSION_MAJOR"), env!("CARGO_PKG_VERSION_MINOR"));
    assert!(open_file.description.contains(&expected_open_docs), "open_file must link to {expected_open_docs}, got: {}", open_file.description);
    let append_desc =
        open_file.input_schema.pointer("/properties/append/description").and_then(Value::as_str).ok_or("missing open_file.append description")?;
    for needle in ["CSV", "TSV", ".pzfx", ".prism"] {
        assert!(append_desc.contains(needle), "open_file.append description missing '{needle}': {append_desc}");
    }

    let export = tools.iter().find(|t| t.name == "export").ok_or("missing export tool")?;
    let format_enum: Vec<&str> = export
        .input_schema
        .pointer("/properties/format/enum")
        .and_then(Value::as_array)
        .ok_or("missing export.format enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for expected in ["svg", "pdf", "emf", "png", "tiff", "csv", "tsv", "json"] {
        assert!(format_enum.contains(&expected), "export.format enum missing '{expected}': {format_enum:?}");
    }
    let pdf_ver_enum: Vec<&str> = export
        .input_schema
        .pointer("/properties/pdf_version/enum")
        .and_then(Value::as_array)
        .ok_or("missing export.pdf_version enum")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(pdf_ver_enum, vec!["1.4", "1.7"]);
    assert_eq!(export.input_schema.pointer("/properties/ttf_font/type"), Some(&json!("string")));

    let tab_id_desc =
        export.input_schema.pointer("/properties/tab_id/description").and_then(Value::as_str).ok_or("missing export.tab_id description")?;
    assert!(tab_id_desc.contains("when supplied"), "export.tab_id description must say 'when supplied', got: {tab_id_desc}");
    assert!(!tab_id_desc.contains("when omitted"), "export.tab_id description must not say 'when omitted': {tab_id_desc}");
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_control_auth_resp_id_mismatch_rejected_over_tcp() -> TestResult {
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(format!("bind loopback: {err}")),
    };
    let port = listener.local_addr().map_err(|e| format!("local_addr: {e}"))?.port();
    let probe_addr = listener.local_addr().map_err(|e| format!("probe addr: {e}"))?;
    match TcpStream::connect_timeout(&probe_addr, Duration::from_millis(200)) {
        Ok(stream) => {
            drop(stream);
            let _ = listener.accept();
        }
        Err(err) if err.kind() == ErrorKind::PermissionDenied => return Ok(()),
        Err(err) => return Err(format!("probe connect: {err}")),
    }

    let handle = thread::spawn(move || {
        // Large >32 KiB `engine.execute` triggers `mcp-auth`; reply with mismatched id.
        if let Ok((mut stream, _)) = listener.accept() {
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            let _ = reader.read_line(&mut line);
            let resp = json!({ "id": "wrong-auth-id", "ok": true, "result": { "authenticated": true } });
            let _ = stream.write_all(format!("{resp}\n").as_bytes());
            let _ = stream.flush();
        }
    });

    let mut state = McpServerState::with_control_port(port)?.with_control_token("test-token-123")?;
    let err = call_err_json(
        &mut state,
        "command_run",
        json!({
            "command_id": "sheet.createData",
            "params": { "sheet_id": "big", "title": "x".repeat(40 * 1024) }
        }),
    )?;
    assert_eq!(err["code"], json!("corrupt_data"), "expected corrupt_data on auth_resp.id mismatch: {err}");
    assert!(err["message"].as_str().is_some_and(|m| m.contains("mismatched control auth response id")), "unexpected error message: {err}");
    let _ = handle.join();
    Ok(())
}

#[test]
fn headless_open_file_append_prism_and_pzfx_preserves_sheets_file_path_and_undo() -> TestResult {
    let dir = TempDir::new_canonical("cb_audit_append_prism_pzfx")?;
    let seed_csv = dir.child("base.csv");
    let proj_path = dir.child("study.curvebook");
    let pzfx_path = dir.child("extra.pzfx");
    let prism_path = dir.child("bundle.prism");

    fs::write(&seed_csv, "X,Y\n1,10\n2,20\n").map_err(|e| format!("write csv: {e}"))?;
    let pzfx_xml = r#"<PrismFile>
  <Title>PZFX Append</Title>
  <Table ID="pzfx_tbl" TableType="XY">
    <Title>Dose Curve</Title>
    <XColumn><Title>X</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></XColumn>
    <YColumn><Title>Y</Title><Subcolumn><d>100</d><d>200</d></Subcolumn></YColumn>
  </Table>
</PrismFile>"#;
    fs::write(&pzfx_path, pzfx_xml).map_err(|e| format!("write pzfx: {e}"))?;

    let mut zip = ZipWriter::new();
    zip.add_file("imported_table.csv", b"X,Y\n5,50\n6,60\n".to_vec()).map_err(|e| format!("zip add: {e}"))?;
    let zip_bytes = zip.finish().map_err(|e| format!("zip finish: {e}"))?;
    fs::write(&prism_path, zip_bytes).map_err(|e| format!("write prism: {e}"))?;

    // Save baseline project with 1 sheet ("base")
    let mut seed = McpServerState::new();
    call_ok_json(&mut seed, "open_file", json!({ "path": seed_csv.display().to_string() }))?;
    save_project_file(&proj_path, seed.session().document()).map_err(|e| format!("save project: {e}"))?;

    let mut state = McpServerState::new();
    call_ok_json(&mut state, "enable_toolset", json!({ "groups": ["table", "edit"] }))?;
    call_ok_json(&mut state, "open_file", json!({ "path": proj_path.display().to_string() }))?;
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));
    assert!(!state.session().is_dirty());

    // Push a prior edit onto the undo stack before appending .pzfx/.prism
    call_ok_json(&mut state, "table_set_cell", json!({ "sheet_id": "base", "row": 0, "col": 0, "value": 99.0 }))?;
    assert_eq!(read_y_cell(&state, "base", 0, 0), Some(99.0));

    // Append .pzfx -> keeps "base", preserves file_path, appends PZFX sheet, preserves undo stack
    let pzfx_res = call_ok_json(&mut state, "open_file", json!({ "path": pzfx_path.display().to_string(), "append": true }))?;
    assert_eq!(pzfx_res["opened"], json!(true));
    assert_eq!(pzfx_res["format"], json!("pzfx"));
    assert_eq!(pzfx_res["total_sheet_count"], json!(2));
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));
    assert!(state.session().is_dirty());
    assert_eq!(read_y_cell(&state, "base", 0, 0), Some(99.0));

    // Append .prism -> keeps "base" + pzfx sheet + adds imported sheet, preserves file_path
    let prism_res = call_ok_json(&mut state, "open_file", json!({ "path": prism_path.display().to_string(), "append": true }))?;
    assert_eq!(prism_res["opened"], json!(true));
    assert_eq!(prism_res["format"], json!("prism"));
    assert_eq!(prism_res["total_sheet_count"], json!(3));
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));
    assert!(state.session().is_dirty());

    // Undo #1 removes appended .prism sheet
    call_ok_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(state.session().document().total_sheet_count(), 2);
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));

    // Undo #2 removes appended .pzfx sheet while keeping prior "base" cell edit (99.0)
    call_ok_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(state.session().document().total_sheet_count(), 1);
    assert_eq!(read_y_cell(&state, "base", 0, 0), Some(99.0));
    assert_eq!(state.session().file_path(), Some(proj_path.as_path()));

    // Undo #3 reverts prior "base" cell edit back to original 10.0
    call_ok_json(&mut state, "edit_undo", json!({}))?;
    assert_eq!(read_y_cell(&state, "base", 0, 0), Some(10.0));
    Ok(())
}

#[test]
fn export_emf_tsv_pdf_version_and_ttf_font_with_workdir_guard() -> TestResult {
    let dir = TempDir::new_canonical("cb_audit_export_parity")?;
    let outside = TempDir::new_canonical("cb_audit_export_outside")?;

    let inside_csv = dir.child("data.csv");
    fs::write(&inside_csv, "X,Y\n1,10\n2,25\n3,40\n").map_err(|e| format!("write csv: {e}"))?;

    let fonts_dir = dir.child("fonts");
    fs::create_dir_all(&fonts_dir).map_err(|e| format!("mkdir fonts: {e}"))?;
    let valid_ttf = fonts_dir.join("audit.ttf");
    fs::write(&valid_ttf, build_minimal_test_ttf("AuditEmbed-Regular")).map_err(|e| format!("write ttf: {e}"))?;
    let bad_ttf = fonts_dir.join("corrupt.ttf");
    fs::write(&bad_ttf, b"OTTO_not_truetype_outlines").map_err(|e| format!("write bad ttf: {e}"))?;
    let outside_ttf = outside.child("escape.ttf");
    fs::write(&outside_ttf, build_minimal_test_ttf("Outside-Regular")).map_err(|e| format!("write outside ttf: {e}"))?;

    let mut state = McpServerState::with_workdir(&dir.path).map_err(|e| format!("with_workdir: {e}"))?;
    call_ok_json(&mut state, "open_file", json!({ "path": "data.csv", "sheet_id": "ds-1" }))?;
    call_ok_json(
        &mut state,
        "command_run",
        json!({ "command_id": "graph.create", "params": { "data_sheet_id": "ds-1", "sheet_id": "g-1", "kind": "xy", "title": "Audit Figure" } }),
    )?;

    // 1. EMF inline + disk export
    let emf_inline = call_ok_json(&mut state, "export", json!({ "format": "emf", "sheet_id": "g-1" }))?;
    assert_eq!(emf_inline["format"], json!("emf"));
    assert_eq!(emf_inline["encoding"], json!("base64"));
    let emf_b64 = emf_inline["data"].as_str().ok_or("missing emf data")?;
    let emf_bytes = curvebook_platform::base64::decode(emf_b64).map_err(|e| format!("decode emf b64: {e}"))?;
    assert!(emf_bytes.len() > 44 && emf_bytes.get(0..4) == Some(&[1, 0, 0, 0]) && emf_bytes.get(40..44) == Some(b" EMF"));

    let emf_disk = call_ok_json(&mut state, "export", json!({ "format": "emf", "sheet_id": "g-1", "output_path": "out/fig.emf" }))?;
    assert_eq!(emf_disk["exported"], json!(true));
    assert_eq!(emf_disk["format"], json!("emf"));
    let disk_emf_bytes = fs::read(dir.child("out/fig.emf")).map_err(|e| format!("read fig.emf: {e}"))?;
    assert_eq!(disk_emf_bytes, emf_bytes);

    // 2. TSV inline + explicit delimiter override + disk export
    let tsv_res = call_tool(&mut state, "export", json!({ "format": "tsv", "sheet_id": "ds-1" })).map_err(|e| format!("tsv export: {e:?}"))?;
    assert!(!tsv_res.is_error, "tsv export failed: {tsv_res:?}");
    let tsv_text = match tsv_res.content.first() {
        Some(ContentBlock::Text { text }) => text.clone(),
        other => return Err(format!("expected text block for tsv, got {other:?}")),
    };
    assert!(tsv_text.contains('\t'), "expected tab separator in TSV output: {tsv_text:?}");

    let tsv_override = call_tool(&mut state, "export", json!({ "format": "tsv", "sheet_id": "ds-1", "csv_delimiter": ";" }))
        .map_err(|e| format!("tsv override: {e:?}"))?;
    assert!(!tsv_override.is_error);
    let override_text = match tsv_override.content.first() {
        Some(ContentBlock::Text { text }) => text.clone(),
        other => return Err(format!("expected text block for override, got {other:?}")),
    };
    assert!(override_text.contains(';'), "expected explicit semicolon delimiter override: {override_text:?}");

    // 3. PDF version 1.4 vs default 1.7 + embedded TrueType font
    let pdf14 =
        call_ok_json(&mut state, "export", json!({ "format": "pdf", "sheet_id": "g-1", "pdf_version": "1.4", "ttf_font": "fonts/audit.ttf" }))?;
    assert_eq!(pdf14["format"], json!("pdf"));
    let pdf14_bytes =
        curvebook_platform::base64::decode(pdf14["data"].as_str().ok_or("missing pdf data")?).map_err(|e| format!("decode pdf: {e}"))?;
    assert!(pdf14_bytes.starts_with(b"%PDF-1.4\n"), "expected %PDF-1.4 header");
    let pdf14_latin1: String = pdf14_bytes.iter().map(|&b| char::from(b)).collect();
    assert!(pdf14_latin1.contains("/Subtype /TrueType"), "expected embedded /Subtype /TrueType in PDF");
    assert!(pdf14_latin1.contains("/FontFile2"), "expected embedded /FontFile2 stream in PDF");
    assert!(pdf14_latin1.contains("/BaseFont /AuditEmbed-Regular"), "expected custom embedded font name in PDF");

    let pdf17 = call_ok_json(&mut state, "export", json!({ "format": "pdf", "sheet_id": "g-1" }))?;
    let pdf17_bytes =
        curvebook_platform::base64::decode(pdf17["data"].as_str().ok_or("missing pdf17 data")?).map_err(|e| format!("decode pdf17: {e}"))?;
    assert!(pdf17_bytes.starts_with(b"%PDF-1.7\n"), "expected default %PDF-1.7 header");

    // 4. Corrupt font bytes rejected cleanly
    let bad_font_err = call_err_json(&mut state, "export", json!({ "format": "pdf", "sheet_id": "g-1", "ttf_font": "fonts/corrupt.ttf" }))?;
    assert_eq!(bad_font_err["code"], json!("bad_input"));

    // 5. WorkdirGuard blocks ttf_font path escape (both relative traversal and absolute outside path)
    for bad_path in [String::from("../outside.ttf"), outside_ttf.display().to_string()] {
        let esc_err = call_err_json(&mut state, "export", json!({ "format": "pdf", "sheet_id": "g-1", "ttf_font": bad_path }))?;
        assert_eq!(esc_err["code"], json!("path_escape"), "expected path_escape for ttf_font escape: {esc_err}");
    }

    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_attach_doc_inspect_enriches_summary_for_default_and_include_cells() -> TestResult {
    let seen_calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::named("test.live_doc_inspect.seen", Vec::new()));
    let seen_clone = Arc::clone(&seen_calls);

    let mut state = McpServerState::with_control_port(49301)?.with_control_handler(move |method, params| {
        if let Ok(mut guard) = seen_clone.lock_or_recover() {
            guard.push((method.to_owned(), params.clone()));
        }
        match method {
            "ping" => Ok(json!({ "pong": true })),
            "state.snapshot" => Ok(json!({
                "title": "Live Study",
                "active_sheet_id": "ds-1",
                "navigator_open": true,
                "zoom": 1.25
            })),
            "doc.inspect" => {
                if let Some(sheet_id) = params.get("sheet_id").and_then(Value::as_str) {
                    Ok(json!({
                        "section": "Data",
                        "id": sheet_id,
                        "title": "Live Data",
                        "include_cells_echo": params.get("include_cells").cloned().unwrap_or(Value::Null),
                    }))
                } else {
                    Ok(json!({
                        "title": "Live Study",
                        "file_path": "/work/live_study.curvebook",
                        "dirty": true,
                        "counts": { "total": 2, "data": 1, "info": 0, "results": 0, "graphs": 1, "layouts": 0 },
                        "data_sheets": [{ "id": "ds-1", "title": "Live Data", "kind": "xy", "rows": 3, "cols": 2, "frozen": false }],
                        "info_sheets": [],
                        "result_sheets": [],
                        "graph_sheets": [{ "id": "g-1", "title": "Live Graph", "initialized": true }],
                        "layout_sheets": [],
                    }))
                }
            }
            other => Err(format!("unexpected method {other}")),
        }
    });

    // 1. doc_inspect({}) merges both snapshot and document summary fields
    let res_default = call_ok_json(&mut state, "doc_inspect", json!({}))?;
    assert_eq!(res_default["mode"], json!("live_attach"));
    assert_eq!(res_default["control_port"], json!(49301));
    assert_eq!(res_default["snapshot"]["zoom"], json!(1.25));
    assert_eq!(res_default["title"], json!("Live Study"));
    assert_eq!(res_default["file_path"], json!("/work/live_study.curvebook"));
    assert_eq!(res_default["dirty"], json!(true));
    assert_eq!(res_default["counts"]["total"], json!(2));
    assert_eq!(res_default["data_sheets"][0]["id"], json!("ds-1"));
    assert_eq!(res_default["graph_sheets"][0]["id"], json!("g-1"));
    assert!(res_default["info_sheets"].is_array());
    assert!(res_default["result_sheets"].is_array());
    assert!(res_default["layout_sheets"].is_array());

    // 2. doc_inspect({ include_cells: true }) with sheet_id omitted matches headless semantics and returns both snapshot + summary
    let res_cells = call_ok_json(&mut state, "doc_inspect", json!({ "include_cells": true }))?;
    assert_eq!(res_cells["mode"], json!("live_attach"));
    assert_eq!(res_cells["snapshot"]["navigator_open"], json!(true));
    assert_eq!(res_cells["counts"]["total"], json!(2));
    assert_eq!(res_cells["data_sheets"][0]["id"], json!("ds-1"));

    // 3. doc_inspect({ sheet_id: "ds-1", include_cells: true }) inspects the single sheet directly
    let res_single = call_ok_json(&mut state, "doc_inspect", json!({ "sheet_id": "ds-1", "include_cells": true }))?;
    assert_eq!(res_single["mode"], json!("live_attach"));
    assert_eq!(res_single["section"], json!("data"));
    assert_eq!(res_single["id"], json!("ds-1"));
    assert_eq!(res_single["include_cells_echo"], json!(true));

    let calls = seen_calls.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
    let method_names: Vec<&str> = calls.iter().map(|(m, _)| m.as_str()).collect();
    assert_eq!(method_names, vec!["state.snapshot", "doc.inspect", "state.snapshot", "doc.inspect", "doc.inspect"]);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_attach_command_batch_delegates_to_desktop_command_batch_with_full_rollback() -> TestResult {
    struct MockDesktop {
        session: Session,
        rulers_visible: bool,
        methods: Vec<String>,
    }

    let desktop =
        Arc::new(Mutex::named("test.live_batch.desktop", MockDesktop { session: Session::new(), rulers_visible: false, methods: Vec::new() }));
    {
        let mut guard = desktop.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
        let cancel = std::sync::atomic::AtomicBool::new(false);
        guard
            .session
            .execute_with_caller("sheet.createData", json!({ "sheet_id": "seed", "title": "Seed", "cols": 2, "rows": 3 }), Caller::Control, &cancel)
            .map_err(|e| format!("seed create: {e}"))?;
        guard
            .session
            .execute_with_caller("table.setCell", json!({ "sheet_id": "seed", "row": 0, "col": 0, "value": 7.0 }), Caller::Control, &cancel)
            .map_err(|e| format!("seed setCell: {e}"))?;
    }

    let desktop_cb = Arc::clone(&desktop);
    let mut state = McpServerState::with_control_port(49302)?.with_control_handler(move |method, params| {
        let mut guard = desktop_cb.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
        guard.methods.push(method.to_owned());
        match method {
            "ping" => Ok(json!({ "pong": true })),
            "command.batch" => {
                let atomic = params.get("atomic").and_then(Value::as_bool).unwrap_or(false);
                let commands = params.get("commands").and_then(Value::as_array).cloned().unwrap_or_default();
                let baseline_session = atomic.then(|| guard.session.clone());
                let baseline_rulers = guard.rulers_visible;
                let cancel = std::sync::atomic::AtomicBool::new(false);
                let mut results = Vec::new();
                for (idx, step) in commands.iter().enumerate() {
                    let cmd_id = step.get("command_id").and_then(Value::as_str).unwrap_or("");
                    let step_params = step.get("params").cloned().unwrap_or_else(|| json!({}));
                    let step_res: Result<Value, String> = if cmd_id == "ui.rulers.toggle" {
                        guard.rulers_visible = step_params.get("visible").and_then(Value::as_bool).unwrap_or(!guard.rulers_visible);
                        Ok(json!({ "rulers_visible": guard.rulers_visible }))
                    } else if cmd_id == "export.svg" {
                        Ok(json!({ "command_id": cmd_id, "mutated": false, "payload": { "path": step_params.get("path") } }))
                    } else {
                        guard
                            .session
                            .execute_with_caller(cmd_id, step_params, Caller::Control, &cancel)
                            .and_then(|out| {
                                serde_json::to_value(out)
                                    .map_err(|e| curvebook_engine::EngineError::with_code(curvebook_doc::ErrorCode::InternalError, e.to_string()))
                            })
                            .map_err(|e| e.to_string())
                    };
                    match step_res {
                        Ok(val) => results.push(json!({ "index": idx, "command": cmd_id, "result": val })),
                        Err(msg) => {
                            if let Some(prev) = baseline_session {
                                guard.session = prev;
                                guard.rulers_visible = baseline_rulers;
                            }
                            return Err(format!("batch step {idx} ({cmd_id}) failed (rolled_back={atomic}): {msg}"));
                        }
                    }
                }
                Ok(json!({ "atomic": atomic, "count": results.len(), "results": results }))
            }
            other => Err(format!("unexpected live method {other}")),
        }
    });

    // 1. Successful mixed engine + UI batch returns MCP's { atomic, count, results: [{ index, ok, outcome }] }
    let ok_batch = call_ok_json(
        &mut state,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "sheet.createData", "params": { "sheet_id": "added", "title": "Added", "cols": 2, "rows": 2 } },
                { "command_id": "ui.rulers.toggle", "params": { "visible": true } }
            ]
        }),
    )?;
    assert_eq!(ok_batch["atomic"], json!(true));
    assert_eq!(ok_batch["count"], json!(2));
    assert_eq!(ok_batch["results"][0]["ok"], json!(true));
    assert_eq!(ok_batch["results"][1]["ok"], json!(true));
    assert_eq!(ok_batch["results"][1]["outcome"]["live_attach"], json!(true));
    assert_eq!(ok_batch["results"][1]["outcome"]["result"]["rulers_visible"], json!(true));

    // 2. Atomic batch with file.new + edit.undo + UI change + failure rolls back completely via single command.batch call
    let pre_fail_count = desktop.lock_or_recover().map_err(|e| format!("lock: {e}"))?.session.document().total_sheet_count();
    let pre_fail_undo = desktop.lock_or_recover().map_err(|e| format!("lock: {e}"))?.session.undo_depth();
    assert_eq!(pre_fail_count, 2);

    let fail_batch = call_err_json(
        &mut state,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "edit.undo", "params": {} },
                { "command_id": "file.new", "params": { "title": "Wiped" } },
                { "command_id": "ui.rulers.toggle", "params": { "visible": false } },
                { "command_id": "nonexistent.command", "params": {} }
            ]
        }),
    )?;
    assert_eq!(fail_batch["atomic"], json!(true));
    assert_eq!(fail_batch["failed_index"], json!(3));
    assert_eq!(fail_batch["failed_command_id"], json!("nonexistent.command"));
    assert_eq!(fail_batch["rolled_back"], json!(true));
    assert_eq!(fail_batch["session_rolled_back"], json!(true));
    assert_eq!(fail_batch["filesystem_side_effects_persisted"], json!(false));

    {
        let guard = desktop.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
        assert_eq!(guard.session.document().total_sheet_count(), pre_fail_count, "file.new must be rolled back");
        assert_eq!(guard.session.undo_depth(), pre_fail_undo, "undo stack must be intact after rollback");
        assert!(guard.rulers_visible, "UI rulers state must be rolled back");
    }

    // 3. Atomic batch with prior filesystem write reports filesystem_side_effects_persisted=true, rolled_back=false, session_rolled_back=true
    let fs_fail = call_err_json(
        &mut state,
        "command_batch",
        json!({
            "atomic": true,
            "commands": [
                { "command_id": "export.svg", "params": { "sheet_id": "g-1", "path": "/work/already_written.svg" } },
                { "command_id": "nonexistent.after.export", "params": {} }
            ]
        }),
    )?;
    assert_eq!(fs_fail["atomic"], json!(true));
    assert_eq!(fs_fail["failed_index"], json!(1));
    assert_eq!(fs_fail["failed_command_id"], json!("nonexistent.after.export"));
    assert_eq!(fs_fail["session_rolled_back"], json!(true));
    assert_eq!(fs_fail["filesystem_side_effects_persisted"], json!(true));
    assert_eq!(fs_fail["rolled_back"], json!(false));
    assert_eq!(fs_fail["persisted_filesystem_steps"][0]["command_id"], json!("export.svg"));

    // Confirm ZERO engine.execute / edit.undo replay calls were sent; only 3 command.batch calls.
    let methods = desktop.lock_or_recover().map_err(|e| format!("lock: {e}"))?.methods.clone();
    assert_eq!(methods, vec!["command.batch", "command.batch", "command.batch"]);
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn live_attach_export_writes_directly_without_base64_and_inline_decodes_base64() -> TestResult {
    let dir = TempDir::new_canonical("cb_audit_live_export")?;
    let out_pdf = dir.child("direct_desktop.pdf");
    let ttf_path = dir.child("custom.ttf");
    fs::write(&ttf_path, build_minimal_test_ttf("LiveEmbed-Regular")).map_err(|e| format!("write ttf: {e}"))?;

    let captured_params: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::named("test.live_export.captured", Vec::new()));
    let captured_clone = Arc::clone(&captured_params);

    let mut state = McpServerState::with_control_port(49303)?.with_control_handler(move |method, params| match method {
        "ping" => Ok(json!({ "pong": true })),
        "doc.export" => {
            if let Ok(mut guard) = captured_clone.lock_or_recover() {
                guard.push(params.clone());
            }
            if let Some(dest) = params.get("path").and_then(Value::as_str) {
                let bytes = b"%PDF-1.4\n%direct-desktop-write\n";
                fs::write(dest, bytes).map_err(|e| format!("write dest: {e}"))?;
                // Deliberately omit data_base64 to prove MCP does not require base64 round-trip.
                Ok(json!({
                    "exported": true,
                    "format": params.get("format").cloned().unwrap_or(json!("pdf")),
                    "sheet_id": "g-live",
                    "path": dest,
                    "bytes_written": bytes.len(),
                }))
            } else {
                let fmt = params.get("format").and_then(Value::as_str).unwrap_or("tsv");
                let raw: &[u8] = if fmt == "emf" { b"\x01\x00\x00\x00fake-emf" } else { b"X\tY\n1\t2\n" };
                Ok(json!({
                    "format": fmt,
                    "sheet_id": "ds-live",
                    "bytes_len": raw.len(),
                    "encoding": "base64",
                    "data_base64": curvebook_platform::base64::encode(raw),
                }))
            }
        }
        other => Err(format!("unexpected method {other}")),
    });

    // 1. Live export with output_path passes path, pdf_version, and ttf_font to desktop doc.export without data_base64
    let direct_res = call_ok_json(
        &mut state,
        "export",
        json!({
            "format": "pdf",
            "sheet_id": "g-live",
            "pdf_version": "1.4",
            "ttf_font": ttf_path.display().to_string(),
            "output_path": out_pdf.display().to_string()
        }),
    )?;
    assert_eq!(direct_res["exported"], json!(true));
    assert_eq!(direct_res["mode"], json!("live_attach"));
    assert_eq!(direct_res["control_port"], json!(49303));
    assert_eq!(direct_res["format"], json!("pdf"));
    assert_eq!(direct_res["sheet_id"], json!("g-live"));
    assert_eq!(direct_res["path"], json!(out_pdf.display().to_string()));
    assert_eq!(direct_res["bytes_written"], json!(b"%PDF-1.4\n%direct-desktop-write\n".len()));
    assert!(out_pdf.is_file());

    // 2. Inline live export (no output_path) still decodes data_base64 for emf & tsv
    let inline_emf = call_ok_json(&mut state, "export", json!({ "format": "emf", "sheet_id": "g-live" }))?;
    assert_eq!(inline_emf["format"], json!("emf"));
    assert_eq!(inline_emf["encoding"], json!("base64"));

    let inline_tsv = call_tool(&mut state, "export", json!({ "format": "tsv", "sheet_id": "ds-live" })).map_err(|e| format!("inline tsv: {e:?}"))?;
    assert!(!inline_tsv.is_error);
    let tsv_text = match inline_tsv.content.first() {
        Some(ContentBlock::Text { text }) => text.as_str(),
        other => return Err(format!("expected text block, got {other:?}")),
    };
    assert_eq!(tsv_text, "X\tY\n1\t2\n");

    let calls = captured_params.lock_or_recover().map_err(|e| format!("lock: {e}"))?;
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["path"], json!(out_pdf.display().to_string()));
    assert_eq!(calls[0]["pdf_version"], json!("1.4"));
    assert_eq!(calls[0]["ttf_font"], json!(ttf_path.display().to_string()));
    assert!(calls[1].get("path").is_none());
    assert!(calls[2].get("path").is_none());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn da_06_02_live_attach_absolutizes_relative_paths_against_mcp_cwd() -> TestResult {
    let cwd = std::env::current_dir().map_err(|err| format!("current_dir: {err}"))?;
    let expected_save = cwd.join("relative_dir/out_project.curvebook");
    let expected_export = cwd.join("relative_dir/out_figure.pdf");
    let expected_font = cwd.join("relative_dir/custom_font.ttf");
    let expected_shot = cwd.join("relative_dir/screen.png");

    let captured: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::named("test.da_06_02.captured", Vec::new()));
    let captured_clone = Arc::clone(&captured);

    let mut state = McpServerState::with_control_port(49304)?.with_control_handler(move |method, params| {
        if let Ok(mut guard) = captured_clone.lock_or_recover() {
            guard.push((method.to_owned(), params.clone()));
        }
        match method {
            "ping" => Ok(json!({ "pong": true })),
            "engine.execute" => Ok(json!({ "mutated": false, "payload": {} })),
            "doc.inspect" => Ok(json!({ "schema_version": 2, "counts": { "total": 1 } })),
            "doc.export" => Ok(json!({
                "exported": true,
                "format": "pdf",
                "sheet_id": "g-1",
                "path": params.get("path").cloned().unwrap_or(Value::Null),
                "bytes_written": 16,
            })),
            "ui.screenshot" => Ok(json!({
                "format": "png",
                "path": params.get("path").cloned().unwrap_or(Value::Null),
                "data_base64": curvebook_platform::base64::encode(b"\x89PNG\r\n\x1a\n"),
            })),
            other => Err(format!("unexpected method {other}")),
        }
    });

    // 1. save_file with relative path forwards cwd-absolutized path to desktop engine.execute.
    let save_res = call_ok_json(&mut state, "save_file", json!({ "path": "relative_dir/out_project.curvebook" }))?;
    assert_eq!(save_res.get("path").and_then(Value::as_str), Some(expected_save.to_string_lossy().as_ref()));

    // 2. export with relative output_path & ttf_font forwards cwd-absolutized paths to desktop doc.export.
    let export_res = call_ok_json(
        &mut state,
        "export",
        json!({
            "format": "pdf",
            "sheet_id": "g-1",
            "output_path": "relative_dir/out_figure.pdf",
            "ttf_font": "relative_dir/custom_font.ttf"
        }),
    )?;
    assert_eq!(export_res.get("path").and_then(Value::as_str), Some(expected_export.to_string_lossy().as_ref()));

    // 3. ui_screenshot with relative path forwards cwd-absolutized path to desktop ui.screenshot.
    let shot_res = call_ok_json(&mut state, "ui_screenshot", json!({ "path": "relative_dir/screen.png" }))?;
    assert_eq!(shot_res.get("path").and_then(Value::as_str), Some(expected_shot.to_string_lossy().as_ref()));

    let calls = captured.lock_or_recover().map_err(|err| format!("lock: {err}"))?;
    let engine_exec = calls.iter().find(|(m, _)| m == "engine.execute").ok_or_else(|| String::from("missing engine.execute call"))?;
    assert_eq!(engine_exec.1.pointer("/params/path").and_then(Value::as_str), Some(expected_save.to_string_lossy().as_ref()));

    let doc_export = calls.iter().find(|(m, _)| m == "doc.export").ok_or_else(|| String::from("missing doc.export call"))?;
    assert_eq!(doc_export.1.get("path").and_then(Value::as_str), Some(expected_export.to_string_lossy().as_ref()));
    assert_eq!(doc_export.1.get("ttf_font").and_then(Value::as_str), Some(expected_font.to_string_lossy().as_ref()));

    let shot_call = calls.iter().find(|(m, _)| m == "ui.screenshot").ok_or_else(|| String::from("missing ui.screenshot call"))?;
    assert_eq!(shot_call.1.get("path").and_then(Value::as_str), Some(expected_shot.to_string_lossy().as_ref()));
    Ok(())
}
