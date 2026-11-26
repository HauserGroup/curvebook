//! End-to-end engine verification tests for `R17.7` (Verification Plan §3):
//! - Slice 1 Gate: stacked kinetic `.xlsx` (`skip_rows`, `split_by_column`, `extract_preamble_info`)
//!   -> `analysis.run` (`kind: "auc"`, explicit `x_min`/`x_max`)
//!   -> merged layout `.xlsx` (`merge: true`, `has_header: false`, `cell_range: "B4:M11"`, `fill_merged_cells: true`)
//!   -> `table.importPlateMap(source={sheet_id, value_column: "Net Area"}, layout_sheet_id, target_kind: "Column")`
//!   -> signed AUC, `Positive Control`/`Blank`, `well_cell_map` & `plate_well_origins`, stale-result `Conflict`,
//!   reactive `Normalize` + `One-Way ANOVA / Dunnett`, snapshot independence, mutation lifecycle (`InsertRows`,
//!   exclude `G12`, overwrite + Undo/Redo), and `.curvebook` v8 archive save/reopen.
//! - Slice 2 Gate: stacked kinetic `.csv` (`filter.row_start`, `split_by_column`, `extract_preamble_info`)
//!   -> `RemoveBaseline` (Buffer)
//!   -> `analysis.run` (`kind: "auc"`) + parallel `x_window_mean` reducer
//!   -> `table.importPlateMap(..., label_grid, x_grid, blank_labels: ["Buffer"], target_kind: "Xy")`
//!   -> lookup `G12` via `plate_well_origins`/`well_cell_map` -> `table.excludeCellsWithReason`
//!   -> `fit.run` (`log_agonist_vs_response_4pl`) recovering accurate logEC50 on standard (`-6..-13`)
//!   and shifted (`-7..-13`) ladders.
//! - `.cbplate` v1 / forward refusal, legacy `roles` `replicate = 1` default, and `table.importCsv` new-sheet filter.
#![forbid(unsafe_code)]
#![deny(clippy::indexing_slicing)]

use std::error::Error;
use std::fmt::Write as _;

use curvebook_doc::{CellCoord, CellValue, ErrorCode, PlateGeometry, PlateWellRole, ResultStatus, SheetId, TableArchetype};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_format::zip::ZipWriter;
use curvebook_format::{PlateMapFile, open_project_bytes, save_project_bytes, serialize_cbplate_json, validate_project_document};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = *chunk.first().unwrap_or(&0) as usize;
        let b1 = *chunk.get(1).unwrap_or(&0) as usize;
        let b2 = *chunk.get(2).unwrap_or(&0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        if let Some(&c0) = TABLE.get((n >> 18) & 0x3F) {
            out.push(char::from(c0));
        }
        if let Some(&c1) = TABLE.get((n >> 12) & 0x3F) {
            out.push(char::from(c1));
        }
        if chunk.len() > 1 {
            if let Some(&c2) = TABLE.get((n >> 6) & 0x3F) {
                out.push(char::from(c2));
            }
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            if let Some(&c3) = TABLE.get(n & 0x3F) {
                out.push(char::from(c3));
            }
        } else {
            out.push('=');
        }
    }
    out
}

fn col_letters(mut col_zero: usize) -> String {
    let mut chars = Vec::new();
    loop {
        let rem = (col_zero % 26) as u8;
        chars.push(char::from(b'A' + rem));
        if col_zero < 26 {
            break;
        }
        col_zero = col_zero / 26 - 1;
    }
    chars.into_iter().rev().collect()
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

enum XlsxCell {
    Empty,
    Str(String),
    Num(f64),
}

fn build_single_sheet_xlsx(sheet_name: &str, rows: &[Vec<XlsxCell>], merges: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let wb = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#,
        xml_escape(sheet_name)
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>"#;

    let mut ws = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r_idx, row) in rows.iter().enumerate() {
        let r_num = r_idx + 1;
        let _ = write!(ws, r#"<row r="{r_num}">"#);
        for (c_idx, cell) in row.iter().enumerate() {
            let cell_ref = format!("{}{}", col_letters(c_idx), r_num);
            match cell {
                XlsxCell::Empty => {}
                XlsxCell::Str(text) => {
                    let _ = write!(ws, r#"<c r="{cell_ref}" t="inlineStr"><is><t>{}</t></is></c>"#, xml_escape(text));
                }
                XlsxCell::Num(val) => {
                    let _ = write!(ws, r#"<c r="{cell_ref}"><v>{val}</v></c>"#);
                }
            }
        }
        ws.push_str("</row>");
    }
    ws.push_str("</sheetData>");
    if !merges.is_empty() {
        let _ = write!(ws, r#"<mergeCells count="{}">"#, merges.len());
        for m in merges {
            let _ = write!(ws, r#"<mergeCell ref="{m}"/>"#);
        }
        ws.push_str("</mergeCells>");
    }
    ws.push_str("</worksheet>");

    let mut zw = ZipWriter::new();
    zw.add_file("xl/workbook.xml", wb.into_bytes())?;
    zw.add_file("xl/_rels/workbook.xml.rels", rels.as_bytes().to_vec())?;
    zw.add_file("xl/worksheets/sheet1.xml", ws.into_bytes())?;
    Ok(zw.finish()?)
}

fn well_label_96(idx: usize) -> String {
    let row = (idx / 12) as u8;
    let col = (idx % 12) + 1;
    format!("{}{col:02}", char::from(b'A' + row))
}

fn build_synthetic_stacked_kinetic_xlsx() -> Result<Vec<u8>, Box<dyn Error>> {
    // Preamble (3 rows) + header (row 4) + 7 rows Channel A + blank separator + 7 rows Channel B.
    let mut rows: Vec<Vec<XlsxCell>> = Vec::new();
    rows.push(vec![XlsxCell::Str(String::from("Protocol: Dual-Channel Kinetic Screen"))]);
    rows.push(vec![XlsxCell::Str(String::from("Temperature: 37.0"))]);
    rows.push(vec![XlsxCell::Str(String::from("Equilibrated 10 min before kinetic read."))]);

    let mut header = vec![XlsxCell::Str(String::from("Channel")), XlsxCell::Str(String::from("Time (s)"))];
    for idx in 0..96 {
        header.push(XlsxCell::Str(well_label_96(idx)));
    }
    rows.push(header);

    // Per-well constant heights for Channel A over t = [0, 10, 20, 30, 40, 50, 60] (span on [10, 50] is 40 s):
    // - A01..A04 (0..4):   Compound 1 -> y = 8.0   -> Net Area = 320.0
    // - A05..A08 (4..8):   Compound 2 -> y = -2.0  -> Net Area = -80.0 (signed negative AUC!)
    // - A09..A12 (8..12):  Compound 3 -> y = 5.0   -> Net Area = 200.0
    // - G09..G11 (80..83): Compound 4 -> y = 6.0   -> Net Area = 240.0
    // - G12      (83):     Compound 4 -> y = 6.2   -> Net Area = 248.0 (distinct QC well)
    // - H01..H04 (84..88): Buffer     -> y = 0.5   -> Net Area = 20.0
    // - H05..H08 (88..92): Reference  -> y = 10.0  -> Net Area = 400.0
    // - other wells: 0.0
    let well_y_a = |idx: usize| -> f64 {
        match idx {
            0..=3 => 8.0,
            4..=7 => -2.0,
            8..=11 => 5.0,
            80..=82 => 6.0,
            83 => 6.2,
            84..=87 => 0.5,
            88..=91 => 10.0,
            _ => 0.0,
        }
    };
    let times = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    for &t in &times {
        let mut r = Vec::with_capacity(98);
        r.push(XlsxCell::Str(String::from("Channel A")));
        r.push(XlsxCell::Num(t));
        for idx in 0..96 {
            r.push(XlsxCell::Num(well_y_a(idx)));
        }
        rows.push(r);
    }
    // Blank separator row between stacked blocks
    rows.push(Vec::new());
    for &t in &times {
        let mut r = Vec::with_capacity(98);
        r.push(XlsxCell::Str(String::from("Channel B")));
        r.push(XlsxCell::Num(t));
        for idx in 0..96 {
            r.push(XlsxCell::Num(well_y_a(idx) * 0.5 + 1.0));
        }
        rows.push(r);
    }

    build_single_sheet_xlsx("KineticRead", &rows, &[])
}

fn build_synthetic_merged_layout_xlsx() -> Result<Vec<u8>, Box<dyn Error>> {
    // Place an 8x12 plate layout at B4:M11 (rows 4..=11, cols B..=M) surrounded by row/column headers in row 3 and col A.
    // Row 4 (plate row A): B4:E4 merged = "Compound 1", F4:I4 merged = "Compound 2", J4:M4 merged = "Compound 3"
    // Row 10 (plate row G): J10:M10 merged = "Compound 4" (covers G09..G12)
    // Row 11 (plate row H): B11:E11 merged = "Buffer", F11:I11 merged = "Reference", J11..M11 left unmerged empty!
    let mut rows: Vec<Vec<XlsxCell>> = Vec::new();
    for _ in 0..11 {
        let mut row = Vec::with_capacity(13);
        for _ in 0..13 {
            row.push(XlsxCell::Empty);
        }
        rows.push(row);
    }
    // Title row 1 & column headers row 3
    if let Some(r0) = rows.first_mut()
        && let Some(c0) = r0.first_mut()
    {
        *c0 = XlsxCell::Str(String::from("Plate Layout Map"));
    }
    if let Some(r2) = rows.get_mut(2) {
        for col in 1..=12 {
            if let Some(slot) = r2.get_mut(col) {
                *slot = XlsxCell::Num(col as f64);
            }
        }
    }
    for pr in 0..8 {
        if let Some(row) = rows.get_mut(3 + pr)
            && let Some(slot) = row.first_mut()
        {
            *slot = XlsxCell::Str(char::from(b'A' + (pr as u8)).to_string());
        }
    }
    // Top-left cells of merged ranges only (fill_merged_cells will expand them!)
    if let Some(row_a) = rows.get_mut(3) {
        if let Some(c) = row_a.get_mut(1) {
            *c = XlsxCell::Str(String::from("Compound 1"));
        }
        if let Some(c) = row_a.get_mut(5) {
            *c = XlsxCell::Str(String::from("Compound 2"));
        }
        if let Some(c) = row_a.get_mut(9) {
            *c = XlsxCell::Str(String::from("Compound 3"));
        }
    }
    if let Some(row_g) = rows.get_mut(9)
        && let Some(c) = row_g.get_mut(9)
    {
        *c = XlsxCell::Str(String::from("Compound 4"));
    }
    if let Some(row_h) = rows.get_mut(10) {
        if let Some(c) = row_h.get_mut(1) {
            *c = XlsxCell::Str(String::from("Buffer"));
        }
        if let Some(c) = row_h.get_mut(5) {
            *c = XlsxCell::Str(String::from("Reference"));
        }
    }

    let merges = ["B4:E4", "F4:I4", "J4:M4", "J10:M10", "B11:E11", "F11:I11"];
    build_single_sheet_xlsx("PlateLayout", &rows, &merges)
}

fn find_well_coord(payload: &Value, well: &str) -> Result<CellCoord, Box<dyn Error>> {
    let entries = payload.get("well_cell_map").and_then(Value::as_array).ok_or("missing well_cell_map array")?;
    for entry in entries {
        if entry.get("well").and_then(Value::as_str) == Some(well) {
            let row = entry.get("row").and_then(Value::as_u64).ok_or("missing row")? as usize;
            let col = entry.get("col").and_then(Value::as_u64).ok_or("missing col")? as usize;
            let subcol = entry.get("subcol").and_then(Value::as_u64).ok_or("missing subcol")? as usize;
            return Ok(CellCoord::new(row, col, subcol));
        }
    }
    Err(format!("well `{well}` not found in well_cell_map").into())
}

fn verify_slice1_snapshot_independence_and_v8_lifecycle(
    session: &mut Session,
    ch_a_id: &str,
    a01_coord: CellCoord,
    g12_coord: CellCoord,
) -> TestResult {
    // 7. Snapshot independence: editing upstream kinetic Channel A recomputes AUC while static plate sheet retains original 320.0!
    session.execute("table.setCell", json!({ "sheet_id": ch_a_id, "row": 3, "col": 0, "subcol": 0, "value": 28.0 }))?;
    let auc_after = session.document().result_sheet(&SheetId::new("auc-ch-a")).ok_or("auc-ch-a after edit")?;
    let auc_ad = auc_after.analyzable_data.as_ref().ok_or("auc analyzable_data")?;
    let TableArchetype::MultipleVariables(mv) = &auc_ad.archetype else {
        return Err("expected MV analyzable_data from AUC".into());
    };
    let net_col = mv.variables.iter().find(|v| v.title == "Net Area").ok_or("Net Area column missing")?;
    let new_a01_auc = net_col.cells.first().and_then(|c| c.active_number()).ok_or("A01 new Net Area missing")?;
    assert!((new_a01_auc - 520.0).abs() < 1e-9, "expected recomputed A01 AUC = 520.0, got {new_a01_auc}");
    let screen_static = session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col still present")?;
    assert_eq!(
        screen_static.cell(a01_coord).and_then(|c| c.active_number()),
        Some(320.0),
        "static plate snapshot must retain original value after upstream kinetic edit"
    );

    // 8. Mutation lifecycle + actual `.curvebook` v8 archive save/reopen:
    session.execute(
        "table.paste",
        json!({
            "sheet_id": "screen-col",
            "text": "999",
            "mode": "insert_rows",
            "start_row": 0,
            "start_col": 0,
        }),
    )?;
    let g12_shifted = CellCoord::new(g12_coord.row + 1, g12_coord.col, g12_coord.subcol);
    let a01_shifted = CellCoord::new(a01_coord.row + 1, a01_coord.col, a01_coord.subcol);
    let ds_shifted = session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col after shift")?;
    assert_eq!(ds_shifted.plate_well_origin(g12_shifted), Some("G12"), "InsertRows must shift G12 origin down by 1");
    assert_eq!(ds_shifted.plate_well_origin(a01_shifted), Some("A01"), "InsertRows must shift A01 origin down by 1");

    session.execute(
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": "screen-col",
            "row": g12_shifted.row,
            "col": g12_shifted.col,
            "subcol": g12_shifted.subcol,
            "reason_tag": "qc_bubble",
            "note": "Air bubble observed in well G12",
        }),
    )?;
    let ds_excl = session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col after exclude")?;
    assert_eq!(ds_excl.plate_well_origin(g12_shifted), Some("G12"), "exclusion must preserve plate_well_origin");

    // Overwrite shifted A01 cell -> clears origin; Undo restores; Redo re-clears.
    session.execute(
        "table.setCell",
        json!({
            "sheet_id": "screen-col",
            "row": a01_shifted.row,
            "col": a01_shifted.col,
            "subcol": a01_shifted.subcol,
            "value": 123.45,
        }),
    )?;
    assert_eq!(
        session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col")?.plate_well_origin(a01_shifted),
        None,
        "overwriting value must clear plate_well_origin"
    );
    assert!(session.undo()?);
    assert_eq!(
        session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col")?.plate_well_origin(a01_shifted),
        Some("A01"),
        "Undo must restore cleared plate_well_origin"
    );
    assert!(session.redo()?);
    assert_eq!(
        session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col")?.plate_well_origin(a01_shifted),
        None,
        "Redo must re-clear plate_well_origin"
    );

    // Save & reopen actual `.curvebook` v8 archive and verify origins survive streaming deserialization.
    let archive_bytes = save_project_bytes(session.document())?;
    let reopened = open_project_bytes(&archive_bytes)?;
    validate_project_document(&reopened)?;
    assert_eq!(reopened.schema_version, 8);
    let reopened_screen = reopened.data_sheet(&SheetId::new("screen-col")).ok_or("reopened screen-col missing")?;
    assert_eq!(reopened_screen.plate_well_origin(g12_shifted), Some("G12"), "G12 origin must survive v8 archive round-trip");
    assert_eq!(reopened_screen.plate_well_origin(a01_shifted), None, "cleared A01 origin must remain cleared after round-trip");
    assert!(reopened_screen.cell(g12_shifted).is_some_and(|c| c.excluded), "G12 exclusion must survive archive round-trip");
    Ok(())
}

fn verify_slice1_stale_refusal(session: &Session, layout_id: &str) -> TestResult {
    let mut stale_doc = session.document().clone();
    if let Some(res) = stale_doc.result_sheet_mut(&SheetId::new("auc-ch-a")) {
        res.status = ResultStatus::Dirty;
    }
    let mut stale_session = Session::from_document(stale_doc);
    let stale_err = stale_session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "screen-stale",
            "title": "Stale Attempt",
            "source": { "sheet_id": "auc-ch-a", "value_column": "Net Area" },
            "layout_sheet_id": layout_id,
            "blank_labels": ["Buffer"],
            "positive_control_labels": ["Reference"],
            "target_kind": "Column",
        }),
    );
    let err = stale_err.err().ok_or("expected stale ResultSheet refusal")?;
    assert_eq!(err.code, ErrorCode::Conflict, "Dirty ResultSheet must fail with ErrorCode::Conflict");
    Ok(())
}

fn verify_slice1_mapped_columns_and_downstream(session: &mut Session, plate_payload: &Value) -> Result<(CellCoord, CellCoord), Box<dyn Error>> {
    let screen_ds = session.document().data_sheet(&SheetId::new("screen-col")).ok_or("screen-col missing")?;
    let TableArchetype::Column(col_tbl) = &screen_ds.archetype else {
        return Err("expected Column archetype for screen-col".into());
    };
    let find_col_vals = |title: &str| -> Result<Vec<f64>, Box<dyn Error>> {
        let ds = col_tbl.data_sets.iter().find(|d| d.title == title).ok_or_else(|| format!("missing column `{title}`"))?;
        Ok(ds.cells.iter().filter_map(|row| row.first().and_then(|c| c.active_number())).collect())
    };
    assert_eq!(find_col_vals("Compound 1")?, vec![320.0, 320.0, 320.0, 320.0]);
    assert_eq!(find_col_vals("Compound 2")?, vec![-80.0, -80.0, -80.0, -80.0], "signed negative AUC must be preserved");
    assert_eq!(find_col_vals("Compound 3")?, vec![200.0, 200.0, 200.0, 200.0]);
    assert_eq!(find_col_vals("Compound 4")?, vec![240.0, 240.0, 240.0, 248.0]);
    assert_eq!(find_col_vals("Positive Control")?, vec![400.0, 400.0, 400.0, 400.0]);
    assert_eq!(find_col_vals("Blank")?, vec![20.0, 20.0, 20.0, 20.0]);

    assert!(
        screen_ds.floating_notes.iter().any(|n| n.body_markdown.contains("auc-ch-a") && n.body_markdown.contains("Net Area")),
        "FloatingNote must record upstream source_sheet_id and value_column"
    );
    let g12_coord = find_well_coord(plate_payload, "G12")?;
    let a01_coord = find_well_coord(plate_payload, "A01")?;
    assert_eq!(screen_ds.plate_well_origin(g12_coord), Some("G12"));
    assert_eq!(screen_ds.cell(g12_coord).and_then(|c| c.active_number()), Some(248.0));
    assert_eq!(screen_ds.plate_well_origin(a01_coord), Some("A01"));

    // Reactive Transform -> Normalize + One-Way ANOVA / Dunnett on the mapped Column sheet.
    session.execute(
        "analysis.run",
        json!({
            "source_sheet_id": "screen-col",
            "sheet_id": "norm-screen",
            "kind": "normalize",
            "zero_anchor": "ColumnMin",
            "hundred_anchor": "ColumnMax",
            "per_data_set": false,
        }),
    )?;
    assert_eq!(session.document().result_sheet(&SheetId::new("norm-screen")).ok_or("norm-screen missing")?.status, ResultStatus::UpToDate);

    session.execute(
        "analysis.run",
        json!({
            "source_sheet_id": "screen-col",
            "sheet_id": "anova-screen",
            "kind": "one_way_anova",
            "selected_data_sets": [0, 1, 2, 3, 4, 5],
            "multiplicity": "dunnett",
            "control_column": 5,
        }),
    )?;
    assert_eq!(session.document().result_sheet(&SheetId::new("anova-screen")).ok_or("anova-screen missing")?.status, ResultStatus::UpToDate);
    Ok((a01_coord, g12_coord))
}

#[test]
fn slice1_gate_stacked_xlsx_auc_layout_plate_map_normalize_anova_and_v8_lifecycle() -> TestResult {
    let mut session = Session::new();

    // 1. Import synthetic stacked kinetic `.xlsx` with skip_rows + split_by_column + extract_preamble_info.
    let kinetic_xlsx = build_synthetic_stacked_kinetic_xlsx()?;
    let imp_kinetic = session.execute(
        "table.importXlsx",
        json!({
            "bytes_base64": encode_base64(&kinetic_xlsx),
            "skip_rows": 3,
            "split_by_column": 0,
            "extract_preamble_info": true,
        }),
    )?;
    let kinetic_ids: Vec<String> = imp_kinetic
        .payload
        .get("imported_sheet_ids")
        .and_then(Value::as_array)
        .ok_or("missing imported_sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert_eq!(kinetic_ids.len(), 2, "expected 2 split kinetic sheets");
    let info_id_str = imp_kinetic.payload.get("info_sheet_id").and_then(Value::as_str).ok_or("expected shared info_sheet_id")?;
    let info_sheet = session.document().info_sheet(&SheetId::new(info_id_str)).ok_or("shared InfoSheet missing")?;
    assert_eq!(info_sheet.linked_data_sheets.len(), 2, "shared InfoSheet must link to both split sheets");
    assert!(info_sheet.get_constant("Protocol").is_some(), "expected Protocol constant");
    assert!(info_sheet.get_constant("Temperature").is_some(), "expected Temperature constant");
    assert!(info_sheet.notes_markdown.contains("Equilibrated"), "expected plain-text preamble in notes_markdown");

    let ch_a_id = kinetic_ids.first().ok_or("missing Channel A sheet ID")?.clone();

    // 2. Run first-class `analysis.run` (`kind: "auc"`, explicit `x_min`, `x_max`).
    let auc_out = session.execute(
        "analysis.run",
        json!({
            "source_sheet_id": ch_a_id,
            "sheet_id": "auc-ch-a",
            "kind": "auc",
            "baseline": 0.0,
            "x_min": 10.0,
            "x_max": 50.0,
        }),
    )?;
    assert!(auc_out.mutated);
    let auc_res = session.document().result_sheet(&SheetId::new("auc-ch-a")).ok_or("auc-ch-a missing")?;
    assert_eq!(auc_res.status, ResultStatus::UpToDate);
    assert!(auc_res.analyzable_data.is_some(), "AUC must emit analyzable_data");

    // Verify methods report cites Yeh & Kwan (1978) and Bailer (1988) across formats.
    let methods_plain = generate_methods_report(session.document(), &SheetId::new("auc-ch-a"), ReportFormat::PlainText)?;
    assert!(methods_plain.contains("Yeh & Kwan"), "plain methods report must cite Yeh & Kwan (1978)");
    assert!(methods_plain.contains("Bailer (1988)"), "plain methods report must cite Bailer (1988)");
    for fmt in [ReportFormat::Markdown, ReportFormat::Latex] {
        let rendered = generate_methods_report(session.document(), &SheetId::new("auc-ch-a"), fmt)?;
        assert!(
            rendered.contains("Yeh") && rendered.contains("Kwan") && rendered.contains("Bailer"),
            "methods report ({fmt:?}) must cite both papers"
        );
    }

    // Verify `analysis.updateParams` supports `baseline` modes and clearing `x_min` via null, then restore.
    session.execute("analysis.updateParams", json!({ "sheet_id": "auc-ch-a", "baseline": "first_point", "x_min": null }))?;
    session.execute(
        "analysis.updateParams",
        json!({ "sheet_id": "auc-ch-a", "baseline": { "mode": "fixed", "value": 0.0 }, "x_min": 10.0, "x_max": 50.0 }),
    )?;

    // 3. Merge synthetic merged 8x12 layout `.xlsx` (`merge: true`, `has_header: false`, `cell_range: "B4:M11"`, `fill_merged_cells: true`).
    let layout_xlsx = build_synthetic_merged_layout_xlsx()?;
    let imp_layout = session.execute(
        "table.importXlsx",
        json!({
            "bytes_base64": encode_base64(&layout_xlsx),
            "merge": true,
            "has_header": false,
            "cell_range": "B4:M11",
            "fill_merged_cells": true,
        }),
    )?;
    let layout_ids: Vec<String> = imp_layout
        .payload
        .get("imported_sheet_ids")
        .and_then(Value::as_array)
        .ok_or("missing layout imported_sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert_eq!(layout_ids.len(), 1);
    let layout_id = layout_ids.first().ok_or("missing layout sheet ID")?.clone();
    let layout_ds = session.document().data_sheet(&SheetId::new(&layout_id)).ok_or("layout sheet missing")?;
    assert_eq!(layout_ds.archetype.row_count(), 8);
    assert_eq!(layout_ds.archetype.column_count(), 12);

    // 4. Stale-result freshness guard: Dirty ResultSheet must be refused with ErrorCode::Conflict.
    verify_slice1_stale_refusal(&session, &layout_id)?;

    // 5. Map UpToDate AUC ResultSheet -> Column screening table with controls.
    let plate_out = session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "screen-col",
            "title": "Screen AUC",
            "source": { "sheet_id": "auc-ch-a", "value_column": "Net Area" },
            "layout_sheet_id": layout_id,
            "blank_labels": ["Buffer"],
            "positive_control_labels": ["Reference"],
            "target_kind": "Column",
            "subtract_blank_mean": false,
        }),
    )?;
    let (a01_coord, g12_coord) = verify_slice1_mapped_columns_and_downstream(&mut session, &plate_out.payload)?;
    verify_slice1_snapshot_independence_and_v8_lifecycle(&mut session, &ch_a_id, a01_coord, g12_coord)
}

fn hill_4pl(log_x: f64, bottom: f64, top: f64, log_ec50: f64, hill: f64) -> f64 {
    bottom + (top - bottom) / (1.0 + 10.0_f64.powf((log_ec50 - log_x) * hill))
}

struct Slice2Fixture {
    label_grid: Vec<Vec<Option<String>>>,
    x_grid: Vec<Vec<Option<f64>>>,
    csv: String,
}

fn build_slice2_shifted_ladder_fixture() -> Slice2Fixture {
    let mut label_grid: Vec<Vec<Option<String>>> = vec![vec![None; 12]; 8];
    let mut x_grid: Vec<Vec<Option<f64>>> = vec![vec![None; 12]; 8];
    let mut well_delta_signal = vec![0.0_f64; 96];

    for r in 0..8 {
        let log_x_a = -6.0 - (r as f64);
        let sig_a = hill_4pl(log_x_a, 10.0, 110.0, -9.0, 1.0);
        for c in 0..3 {
            if let Some(slot) = label_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = Some(String::from("Agonist A"));
            }
            if let Some(slot) = x_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = Some(log_x_a);
            }
            if let Some(slot) = well_delta_signal.get_mut(r * 12 + c) {
                *slot = sig_a;
            }
        }
    }
    for r in 0..7 {
        let log_x_b = -7.0 - (r as f64);
        let sig_b = hill_4pl(log_x_b, 10.0, 90.0, -10.2, 1.0);
        for c in 3..6 {
            if let Some(slot) = label_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = Some(String::from("Agonist B"));
            }
            if let Some(slot) = x_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
                *slot = Some(log_x_b);
            }
            if let Some(slot) = well_delta_signal.get_mut(r * 12 + c) {
                *slot = sig_b;
            }
        }
    }
    // Anomalous outlier well G12 (row 6, col 11, idx 83)
    if let Some(slot) = label_grid.get_mut(6).and_then(|row| row.get_mut(11)) {
        *slot = Some(String::from("Agonist A"));
    }
    if let Some(slot) = x_grid.get_mut(6).and_then(|row| row.get_mut(11)) {
        *slot = Some(-12.0);
    }
    if let Some(slot) = well_delta_signal.get_mut(83) {
        *slot = 5000.0;
    }
    // Buffer wells H10..H12 (row 7, cols 9..=11, idx 93..=95)
    for c in 9..12 {
        if let Some(slot) = label_grid.get_mut(7).and_then(|row| row.get_mut(c)) {
            *slot = Some(String::from("Buffer"));
        }
        if let Some(slot) = well_delta_signal.get_mut(7 * 12 + c) {
            *slot = 0.0;
        }
    }

    let buffer_offset = 20.0;
    let mut csv = String::new();
    csv.push_str("Assay: Shifted-Ladder Kinetic CRC,,,,,\n");
    csv.push_str("Temperature: 25.5,,,,,\n");
    csv.push_str("Plate equilibrated prior to read,,,,,\n");
    csv.push_str("Channel,Time");
    for idx in 0..96 {
        let _ = write!(csv, ",{}", well_label_96(idx));
    }
    csv.push('\n');

    let times = [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0];
    for &t in &times {
        let _ = write!(csv, "Em460,{t}");
        for &delta in &well_delta_signal {
            let _ = write!(csv, ",{:.6}", buffer_offset + delta);
        }
        csv.push('\n');
    }
    csv.push('\n');
    for &t in &times {
        let _ = write!(csv, "Em530,{t}");
        for &delta in &well_delta_signal {
            let _ = write!(csv, ",{:.6}", 5.0 + 0.5 * delta);
        }
        csv.push('\n');
    }
    Slice2Fixture { label_grid, x_grid, csv }
}

fn map_exclude_g12_and_verify_4pl_log_ec50(
    session: &mut Session,
    fixture: &Slice2Fixture,
    plate_sheet_id: &str,
    fit_sheet_id: &str,
    source_json: Value,
) -> TestResult {
    let plate_out = session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": plate_sheet_id,
            "title": plate_sheet_id,
            "source": source_json,
            "label_grid": fixture.label_grid,
            "x_grid": fixture.x_grid,
            "blank_labels": ["Buffer"],
            "x_title": "Log[Agonist] (M)",
            "target_kind": "Xy",
        }),
    )?;
    let g12_coord = find_well_coord(&plate_out.payload, "G12")?;
    let ds = session.document().data_sheet(&SheetId::new(plate_sheet_id)).ok_or("mapped CRC sheet missing")?;
    assert_eq!(ds.plate_well_origin(g12_coord), Some("G12"));

    session.execute(
        "table.excludeCellsWithReason",
        json!({
            "sheet_id": plate_sheet_id,
            "row": g12_coord.row,
            "col": g12_coord.col,
            "subcol": g12_coord.subcol,
            "reason_tag": "pipetting_outlier",
            "note": "Anomalous spike in well G12",
        }),
    )?;
    assert_eq!(
        session.document().data_sheet(&SheetId::new(plate_sheet_id)).ok_or("mapped CRC sheet")?.plate_well_origin(g12_coord),
        Some("G12"),
        "excluding G12 must preserve plate_well_origin"
    );

    session.execute(
        "fit.run",
        json!({
            "source_sheet_id": plate_sheet_id,
            "sheet_id": fit_sheet_id,
            "kind": "log_agonist_vs_response_4pl",
        }),
    )?;
    let fit = session.document().result_sheet(&SheetId::new(fit_sheet_id)).ok_or("fit sheet missing")?;
    assert_eq!(fit.status, ResultStatus::UpToDate);
    let log_ec50_a =
        fit.scalars.iter().find(|s| s.data_set_index == Some(0) && s.key == "LogEC50").map(|s| s.value).ok_or("missing LogEC50 for Agonist A")?;
    let log_ec50_b =
        fit.scalars.iter().find(|s| s.data_set_index == Some(1) && s.key == "LogEC50").map(|s| s.value).ok_or("missing LogEC50 for Agonist B")?;
    assert!((log_ec50_a - (-9.0)).abs() < 1e-3, "Agonist A standard-ladder logEC50 mismatch: {log_ec50_a}");
    assert!((log_ec50_b - (-10.2)).abs() < 1e-3, "Agonist B shifted-ladder logEC50 mismatch: {log_ec50_b}");
    Ok(())
}

#[test]
fn slice2_gate_stacked_csv_remove_baseline_auc_and_x_window_mean_shifted_ladder_4pl() -> TestResult {
    let fixture = build_slice2_shifted_ladder_fixture();
    let mut session = Session::new();

    // Verify mutual exclusions on `table.importCsv` first.
    let bad_combo = session.execute(
        "table.importCsv",
        json!({
            "csv": fixture.csv,
            "filter": { "row_start": 3, "row_match": { "column": 0, "equals": "Em460" } },
            "split_by_column": 0,
        }),
    );
    assert_eq!(bad_combo.err().ok_or("expected split + row_match error")?.code, ErrorCode::BadInput);

    let bad_link = session.execute(
        "table.importCsv",
        json!({
            "csv": fixture.csv,
            "filter": { "row_start": 3 },
            "split_by_column": 0,
            "link": true,
        }),
    );
    assert_eq!(bad_link.err().ok_or("expected split + link error")?.code, ErrorCode::BadInput);

    // 1. Import stacked CSV -> 2 XY sheets + 1 shared preamble InfoSheet.
    let imp_csv = session.execute(
        "table.importCsv",
        json!({
            "sheet_id": "kin",
            "title": "Kinetic CRC",
            "csv": fixture.csv,
            "archetype": "xy",
            "filter": { "row_start": 3 },
            "split_by_column": 0,
            "extract_preamble_info": true,
        }),
    )?;
    let csv_ids: Vec<String> = imp_csv
        .payload
        .get("imported_sheet_ids")
        .and_then(Value::as_array)
        .ok_or("missing imported_sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    assert_eq!(csv_ids.len(), 2);
    assert!(imp_csv.payload.get("info_sheet_id").and_then(Value::as_str).is_some(), "expected shared preamble info_sheet_id");
    let em460_id = csv_ids.first().ok_or("missing Em460 sheet id")?.clone();

    // Verify rejecting split_by_column when targeting an existing sheet.
    let bad_existing = session.execute(
        "table.importCsv",
        json!({
            "sheet_id": em460_id,
            "csv": fixture.csv,
            "filter": { "row_start": 3 },
            "split_by_column": 0,
        }),
    );
    assert_eq!(bad_existing.err().ok_or("expected split on existing sheet error")?.code, ErrorCode::BadInput);

    // 2. RemoveBaseline using Buffer columns (H10..H12 -> data_set indices 93, 94, 95).
    session.execute(
        "analysis.run",
        json!({
            "source_sheet_id": em460_id,
            "sheet_id": "delta-em460",
            "kind": "remove_baseline",
            "baseline_columns": [93, 94, 95],
            "operation": "difference",
        }),
    )?;
    assert_eq!(session.document().result_sheet(&SheetId::new("delta-em460")).ok_or("delta-em460 missing")?.status, ResultStatus::UpToDate);

    // 3. Run standalone AUC over [15.0, 75.0] on delta-em460 (span = 60.0 s).
    session.execute(
        "analysis.run",
        json!({
            "source_sheet_id": "delta-em460",
            "sheet_id": "auc-em460",
            "kind": "auc",
            "baseline": 0.0,
            "x_min": 15.0,
            "x_max": 75.0,
        }),
    )?;
    assert_eq!(session.document().result_sheet(&SheetId::new("auc-em460")).ok_or("auc-em460 missing")?.status, ResultStatus::UpToDate);

    // 4. Map AUC -> XY shifted-ladder table, exclude G12 via well_cell_map, fit 4PL.
    map_exclude_g12_and_verify_4pl_log_ec50(
        &mut session,
        &fixture,
        "crc-auc",
        "fit-crc-auc",
        json!({ "sheet_id": "auc-em460", "value_column": "Net Area" }),
    )?;

    // 5. Parallel Slice 2 path: kinetic `x_window_mean` reducer directly on delta-em460 + exclude G12 + 4PL fit.
    map_exclude_g12_and_verify_4pl_log_ec50(
        &mut session,
        &fixture,
        "crc-win",
        "fit-crc-win",
        json!({ "sheet_id": "delta-em460", "reducer": { "x_window_mean": { "x_min": 15.0, "x_max": 75.0 } } }),
    )?;

    // Also verify `min`, `max`, and `row_index` reducers succeed cleanly via engine command JSON contract.
    for (sid, reducer_json) in [("red-min", json!("min")), ("red-max", json!("max")), ("red-row", json!({ "row_index": 2 }))] {
        let out = session.execute(
            "table.importPlateMap",
            json!({
                "sheet_id": sid,
                "source": { "sheet_id": "delta-em460", "reducer": reducer_json },
                "label_grid": fixture.label_grid,
                "x_grid": fixture.x_grid,
                "blank_labels": ["Buffer"],
                "target_kind": "Column",
            }),
        )?;
        assert!(out.mutated);
    }

    Ok(())
}

#[test]
fn plate_map_cbplate_v1_forward_refusal_legacy_replicate_default_and_csv_new_sheet_filter() -> TestResult {
    let mut session = Session::new();

    // 1. `.cbplate` v1 JSON accepted as string and object; v2 cleanly rejected.
    let mut wells = vec![PlateWellRole::Excluded; 96];
    if let Some(w0) = wells.first_mut() {
        *w0 = PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: -6.0, replicate: 0 };
    }
    if let Some(w1) = wells.get_mut(1) {
        *w1 = PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: -6.0, replicate: 1 };
    }
    let cbplate = PlateMapFile::new("V1 Plate", PlateGeometry::Wells96, "Log[M]", wells)?;
    let cbplate_json = serialize_cbplate_json(&cbplate, false)?;
    let mut values = vec![None; 96];
    if let Some(v0) = values.first_mut() {
        *v0 = Some(42.0);
    }
    if let Some(v1) = values.get_mut(1) {
        *v1 = Some(48.0);
    }

    let out_str = session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "cbplate-str",
            "values": values,
            "template": cbplate_json,
            "target_kind": "Column",
        }),
    )?;
    assert!(out_str.mutated);

    let cbplate_obj: Value = serde_json::from_str(&cbplate_json)?;
    let out_obj = session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "cbplate-obj",
            "values": values,
            "template": cbplate_obj,
            "target_kind": "Column",
        }),
    )?;
    assert!(out_obj.mutated);

    let mut v2_obj: Value = serde_json::from_str(&cbplate_json)?;
    if let Some(map) = v2_obj.as_object_mut() {
        map.insert(String::from("format_version"), json!(2));
    }
    let v2_str = serde_json::to_string(&v2_obj)?;
    let v2_err = session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "cbplate-v2",
            "values": values,
            "template": v2_str,
            "target_kind": "Column",
        }),
    );
    assert!(v2_err.is_err(), "future `.cbplate` v2 must be rejected");

    // 2. Legacy `roles` without `replicate` MUST preserve `replicate = 1` default in MultipleVariables output.
    let mut legacy_roles = vec![json!("excluded"); 96];
    if let Some(r0) = legacy_roles.first_mut() {
        *r0 = json!({ "role": "treatment", "group_name": "ArmA", "dilution_x": 1.0 });
    }
    session.execute(
        "table.importPlateMap",
        json!({
            "sheet_id": "legacy-mv",
            "geometry": 96,
            "values": values,
            "roles": legacy_roles,
            "target_kind": "MultipleVariables",
        }),
    )?;
    let legacy_ds = session.document().data_sheet(&SheetId::new("legacy-mv")).ok_or("legacy-mv missing")?;
    let TableArchetype::MultipleVariables(mv) = &legacy_ds.archetype else {
        return Err("expected MultipleVariables sheet".into());
    };
    let rep_var = mv.variables.iter().find(|v| v.title == "Replicate").ok_or("missing Replicate column")?;
    assert_eq!(rep_var.cells.first().map(|c| &c.value), Some(&CellValue::Number(1.0)), "legacy roles replicate default must remain 1");

    // 3. `table.importCsv` honors `filter` (`row_start`, `row_match`, `decimate_every_k`) on NEW sheets!
    let csv_text = "Meta: Test\nChannel,Time,Signal\nCh1,0,10\nCh2,0,99\nCh1,10,20\nCh1,20,30\nCh1,30,40\n";
    let csv_out = session.execute(
        "table.importCsv",
        json!({
            "sheet_id": "filtered-new",
            "csv": csv_text,
            "archetype": "xy",
            "filter": {
                "row_start": 1,
                "row_match": { "column": 0, "equals": "Ch1", "drop_column": true },
                "decimate_every_k": 2,
            },
        }),
    )?;
    assert_eq!(csv_out.payload["sheet_id"], "filtered-new");
    let filtered_ds = session.document().data_sheet(&SheetId::new("filtered-new")).ok_or("filtered-new missing")?;
    let TableArchetype::Xy(xy) = &filtered_ds.archetype else {
        return Err("expected XY sheet for filtered-new".into());
    };
    assert_eq!(xy.row_count(), 2, "row_match + decimate_every_k=2 must keep rows at t=0 and t=20");
    assert_eq!(xy.x_value_at(0), Some(0.0));
    assert_eq!(xy.x_value_at(1), Some(20.0));
    assert_eq!(xy.data_sets.first().and_then(|d| d.cells.first()).and_then(|r| r.first()).and_then(|c| c.active_number()), Some(10.0));
    assert_eq!(xy.data_sets.first().and_then(|d| d.cells.get(1)).and_then(|r| r.first()).and_then(|c| c.active_number()), Some(30.0));

    Ok(())
}
