//! Serialized live macOS `/usr/bin/pbcopy` + `/usr/bin/pbpaste` pasteboard round-trip test for
//! `de-DE` decimal-comma numbers, `=...` formula preservation, and multi-flavor clipboard bundles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(target_os = "macos")]
#[test]
fn macos_live_pasteboard_de_de_tsv_formula_and_bundle_roundtrip() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::time::{SystemTime, UNIX_EPOCH};

    use lumencraft_doc::{Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GraphKind, GraphSheet, ProjectDocument, TableArchetype};
    use lumencraft_engine::Session;
    use lumencraft_ui_egui::{
        ClipboardBundle, OsAdapterAvailability, UiApp, read_os_clipboard_pdf_bytes, read_os_clipboard_text, read_os_pasteboard,
        write_os_clipboard_bundle, write_os_pasteboard,
    };

    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let sentinel = format!("LC_W7_{}_{}", std::process::id(), nonce);

    let table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).expect("col table");
    let mut sheet = DataSheet::new("data-de", &sentinel, TableArchetype::Column(table)).expect("datasheet");
    sheet.set_cell(CellCoord::new(0, 0, 0), Cell::number(12.75)).expect("num cell");
    sheet.set_cell(CellCoord::new(0, 1, 0), Cell::text("=A1*2")).expect("formula cell");
    sheet.set_cell(CellCoord::new(1, 0, 0), Cell::number(-12.5)).expect("neg cell");
    sheet.set_cell(CellCoord::new(1, 1, 0), Cell::excluded_number(9.75)).expect("excluded cell");

    let tsv_core = ClipboardBundle::tsv_from_datasheet(&sheet, ',').expect("de-DE TSV formatting");
    assert!(tsv_core.contains("12,75"), "expected decimal-comma 12,75 in `{tsv_core}`");
    assert!(tsv_core.contains("=A1*2"), "expected formula =A1*2 in `{tsv_core}`");
    assert!(tsv_core.contains("9,75*"), "expected excluded decimal-comma 9,75* in `{tsv_core}`");
    let payload = format!("{sentinel}\t{tsv_core}");

    // Phase 1: Attempt LIVE `/usr/bin/pbcopy` + `/usr/bin/pbpaste` round-trip directly.
    let mut pbcopy = match Command::new("/usr/bin/pbcopy").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(err) => {
            println!("macos_clipboard_live: SKIPPED (failed to spawn /usr/bin/pbcopy: {err})");
            return;
        }
    };
    if let Some(mut stdin) = pbcopy.stdin.take()
        && let Err(err) = stdin.write_all(payload.as_bytes())
    {
        println!("macos_clipboard_live: SKIPPED (failed writing to /usr/bin/pbcopy: {err})");
        return;
    }
    let copy_out = match pbcopy.wait_with_output() {
        Ok(out) => out,
        Err(err) => {
            println!("macos_clipboard_live: SKIPPED (/usr/bin/pbcopy wait error: {err})");
            return;
        }
    };
    if !copy_out.status.success() {
        let stderr = String::from_utf8_lossy(&copy_out.stderr);
        println!(
            "macos_clipboard_live: SKIPPED (headless macOS pasteboard server unavailable on pbcopy: status={} stderr=`{}`)",
            copy_out.status,
            stderr.trim()
        );
        return;
    }

    let paste_out = match Command::new("/usr/bin/pbpaste").output() {
        Ok(out) => out,
        Err(err) => {
            println!("macos_clipboard_live: SKIPPED (failed running /usr/bin/pbpaste: {err})");
            return;
        }
    };
    if !paste_out.status.success() {
        let stderr = String::from_utf8_lossy(&paste_out.stderr);
        println!(
            "macos_clipboard_live: SKIPPED (headless macOS pasteboard server unavailable on pbpaste: status={} stderr=`{}`)",
            paste_out.status,
            stderr.trim()
        );
        return;
    }

    let pasted = String::from_utf8(paste_out.stdout).expect("pbpaste UTF-8");
    assert_eq!(pasted, payload, "live /usr/bin/pbcopy -> /usr/bin/pbpaste round-trip mismatch");

    // Phase 2: Exercise `write_os_pasteboard` + `read_os_pasteboard` + `write_os_clipboard_bundle`.
    let write_status = write_os_pasteboard(&payload).expect("write_os_pasteboard");
    assert_eq!(write_status.availability, OsAdapterAvailability::Submitted);
    let read_status = read_os_pasteboard().expect("read_os_pasteboard");
    assert_eq!(read_status.availability, OsAdapterAvailability::Read);
    assert_eq!(read_status.text.as_deref(), Some(payload.as_str()));

    let mut doc = ProjectDocument::new("Clipboard Bundle").expect("doc");
    doc.add_data_sheet(sheet.clone()).expect("add sheet");
    let mut graph = GraphSheet::new_uninitialized_for_data("g-clip", &sheet).expect("graph");
    graph.initialized = true;
    graph.spec.kind = GraphKind::Column;
    doc.add_graph_sheet(graph).expect("add graph");
    let session = Session::from_document(doc);
    let read_text = read_os_clipboard_text().expect("read_os_clipboard_text TSV roundtrip");
    assert_eq!(read_text, payload);

    let bundle = ClipboardBundle::from_sheet(&session, &"g-clip".into(), 144).expect("bundle");
    assert!(bundle.emf_bytes.is_some());
    assert!(bundle.pdf_bytes.starts_with(b"%PDF-1.7"));
    let bundle_status = write_os_clipboard_bundle(&bundle).expect("write_os_clipboard_bundle");
    assert!(bundle_status.available, "expected delivered bundle status: {bundle_status:?}");
    assert_eq!(bundle_status.availability, OsAdapterAvailability::Submitted);

    let live_pdf = read_os_clipboard_pdf_bytes().expect("read_os_clipboard_pdf_bytes immediately after write");
    assert!(live_pdf.starts_with(b"%PDF-1.7"), "pasteboard PDF missing %PDF-1.7 header");
    assert_eq!(live_pdf, bundle.pdf_bytes, "exact PDF byte roundtrip failed on macOS NSPasteboard");

    let bundle_text = read_os_clipboard_text().expect("read_os_clipboard_text after bundle write");
    assert_eq!(bundle_text, bundle.svg);

    // Also verify a multi-panel LayoutSheet bundle via UiApp roundtrips exact %PDF-1.7 + text.
    let mut app = UiApp::new();
    app.execute_command("sheet.createData", serde_json::json!({ "sheet_id": "d-ui", "kind": "xy", "title": "Assay", "auto_graph": true }))
        .expect("data");
    app.execute_command("table.setCell", serde_json::json!({ "sheet_id": "d-ui", "row": 0, "col": 0, "value": 1.0 })).expect("x0");
    app.execute_command("table.setCell", serde_json::json!({ "sheet_id": "d-ui", "row": 0, "col": 1, "value": 10.0 })).expect("y0");
    app.execute_command("table.setCell", serde_json::json!({ "sheet_id": "d-ui", "row": 1, "col": 0, "value": 2.0 })).expect("x1");
    app.execute_command("table.setCell", serde_json::json!({ "sheet_id": "d-ui", "row": 1, "col": 1, "value": 25.0 })).expect("y1");
    let graph_id = app.session().document().graph_sheets.first().expect("graph sheet").id.clone();
    app.execute_command("layout.create", serde_json::json!({ "sheet_id": "layout-ui", "title": "Figure Layout", "preset": "double_column" }))
        .expect("create layout");
    app.execute_command(
        "layout.addPanel",
        serde_json::json!({ "sheet_id": "layout-ui", "kind": "graph", "graph_sheet_id": graph_id, "bounds_pt": { "x": 12.0, "y": 12.0, "width": 180.0, "height": 140.0 } }),
    )
    .expect("add layout panel");
    let layout_bundle = ClipboardBundle::from_sheet(app.session(), &"layout-ui".into(), 144).expect("layout bundle");
    let layout_status = write_os_clipboard_bundle(&layout_bundle).expect("write layout bundle");
    assert!(layout_status.available);
    assert_eq!(layout_status.availability, OsAdapterAvailability::Submitted);
    let layout_pdf = read_os_clipboard_pdf_bytes().expect("read layout pdf from pasteboard");
    assert!(layout_pdf.starts_with(b"%PDF-1.7"));
    assert_eq!(layout_pdf, layout_bundle.pdf_bytes);
    assert_eq!(read_os_clipboard_text().expect("layout pasteboard text"), layout_bundle.svg);

    println!(
        "macos_clipboard_live: EXECUTED_LIVE_ROUNDTRIP sentinel={sentinel} tsv_bytes={} emf_bytes={} pdf_bytes={} layout_pdf_bytes={}",
        payload.len(),
        bundle.emf_bytes.as_ref().map_or(0, Vec::len),
        live_pdf.len(),
        layout_pdf.len(),
    );
}
