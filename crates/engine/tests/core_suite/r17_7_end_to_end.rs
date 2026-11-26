//! End-to-end engine verification tests for `R17.7` (Verification Plan §3):
//! - Slice 1 Gate: stacked kinetic `.xlsx` -> `analysis.run` (`auc`) -> merged layout `.xlsx`
//!   -> `table.importPlateMap` (`Column`) -> signed AUC, controls, `well_cell_map`/`plate_well_origins`,
//!   stale-result `Conflict`, reactive `Normalize` + `One-Way ANOVA / Dunnett`, snapshot independence,
//!   mutation lifecycle (`InsertRows`, exclude `G12`, overwrite + Undo/Redo), `.curvebook` v8 round-trip.
//! - Slice 2 Gate: stacked kinetic `.csv` -> `RemoveBaseline` -> `auc` & `x_window_mean` reducer
//!   -> `table.importPlateMap` (`Xy`) -> `G12` exclusion -> `fit.run` (`log_agonist_vs_response_4pl`)
//!   recovering standard (`-6..-13`) and shifted (`-7..-13`) ladders.
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

use super::common::{assert_err_code, ds, encode_base64, info, ok_cmd, rs, scalar_ds};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone)]
enum XlsxCell {
    Empty,
    Str(String),
    Num(f64),
}

fn col_letters(mut col_zero: usize) -> String {
    let mut chars = Vec::new();
    loop {
        chars.push(char::from(b'A' + (col_zero % 26) as u8));
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

fn build_single_sheet_xlsx(sheet_name: &str, rows: &[Vec<XlsxCell>], merges: &[&str]) -> Result<Vec<u8>, Box<dyn Error>> {
    let wb = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{}" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        xml_escape(sheet_name)
    );
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
    let mut ws = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r_idx, row) in rows.iter().enumerate() {
        let r_num = r_idx + 1;
        let _ = write!(ws, r#"<row r="{r_num}">"#);
        for (c_idx, cell) in row.iter().enumerate() {
            let cell_ref = format!("{}{r_num}", col_letters(c_idx));
            match cell {
                XlsxCell::Empty => {}
                XlsxCell::Str(t) => {
                    let _ = write!(ws, r#"<c r="{cell_ref}" t="inlineStr"><is><t>{}</t></is></c>"#, xml_escape(t));
                }
                XlsxCell::Num(v) => {
                    let _ = write!(ws, r#"<c r="{cell_ref}"><v>{v}</v></c>"#);
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
    format!("{}{:02}", char::from(b'A' + (idx / 12) as u8), (idx % 12) + 1)
}

fn build_synthetic_stacked_kinetic_xlsx() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut rows: Vec<Vec<XlsxCell>> = vec![
        vec![XlsxCell::Str("Protocol: Dual-Channel Kinetic Screen".into())],
        vec![XlsxCell::Str("Temperature: 37.0".into())],
        vec![XlsxCell::Str("Equilibrated 10 min before kinetic read.".into())],
    ];
    let mut header = vec![XlsxCell::Str("Channel".into()), XlsxCell::Str("Time (s)".into())];
    header.extend((0..96).map(|i| XlsxCell::Str(well_label_96(i))));
    rows.push(header);

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
    let push_block = |rows: &mut Vec<Vec<XlsxCell>>, ch: &str, scale: f64, off: f64| {
        for &t in &times {
            let mut r = Vec::with_capacity(98);
            r.push(XlsxCell::Str(ch.into()));
            r.push(XlsxCell::Num(t));
            r.extend((0..96).map(|i| XlsxCell::Num(well_y_a(i) * scale + off)));
            rows.push(r);
        }
    };
    push_block(&mut rows, "Channel A", 1.0, 0.0);
    rows.push(Vec::new());
    push_block(&mut rows, "Channel B", 0.5, 1.0);
    build_single_sheet_xlsx("KineticRead", &rows, &[])
}

fn build_synthetic_merged_layout_xlsx() -> Result<Vec<u8>, Box<dyn Error>> {
    let mut rows = vec![vec![XlsxCell::Empty; 13]; 11];
    let set = |rows: &mut [Vec<XlsxCell>], r: usize, c: usize, cell: XlsxCell| {
        if let Some(slot) = rows.get_mut(r).and_then(|row| row.get_mut(c)) {
            *slot = cell;
        }
    };
    set(&mut rows, 0, 0, XlsxCell::Str("Plate Layout Map".into()));
    for col in 1..=12 {
        set(&mut rows, 2, col, XlsxCell::Num(col as f64));
    }
    for pr in 0..8 {
        set(&mut rows, 3 + pr, 0, XlsxCell::Str(char::from(b'A' + (pr as u8)).to_string()));
    }
    for (r, c, label) in
        [(3, 1, "Compound 1"), (3, 5, "Compound 2"), (3, 9, "Compound 3"), (9, 9, "Compound 4"), (10, 1, "Buffer"), (10, 5, "Reference")]
    {
        set(&mut rows, r, c, XlsxCell::Str(label.into()));
    }
    build_single_sheet_xlsx("PlateLayout", &rows, &["B4:E4", "F4:I4", "J4:M4", "J10:M10", "B11:E11", "F11:I11"])
}

fn find_well_coord(payload: &Value, well: &str) -> Result<CellCoord, Box<dyn Error>> {
    let entries = payload.get("well_cell_map").and_then(Value::as_array).ok_or("missing well_cell_map")?;
    for e in entries {
        if e.get("well").and_then(Value::as_str) == Some(well) {
            let u = |k| e.get(k).and_then(Value::as_u64).ok_or(k).map(|v| v as usize);
            return Ok(CellCoord::new(u("row")?, u("col")?, u("subcol")?));
        }
    }
    Err(format!("well `{well}` not found").into())
}

fn imported_ids(payload: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(payload
        .get("imported_sheet_ids")
        .and_then(Value::as_array)
        .ok_or("missing imported_sheet_ids")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}

fn verify_slice1_snapshot_and_v8(session: &mut Session, ch_a_id: &str, a01: CellCoord, g12: CellCoord) -> TestResult {
    ok_cmd(session, "table.setCell", json!({ "sheet_id": ch_a_id, "row": 3, "col": 0, "subcol": 0, "value": 28.0 }));
    let TableArchetype::MultipleVariables(mv) = &rs(session, "auc-ch-a").analyzable_data.as_ref().ok_or("auc analyzable_data")?.archetype else {
        return Err("expected MV analyzable_data".into());
    };
    let net =
        mv.variables.iter().find(|v| v.title == "Net Area").and_then(|v| v.cells.first()).and_then(|c| c.active_number()).ok_or("A01 Net Area")?;
    assert!((net - 520.0).abs() < 1e-9, "recomputed A01 AUC={net}");
    assert_eq!(ds(session, "screen-col").cell(a01).and_then(|c| c.active_number()), Some(320.0));

    ok_cmd(session, "table.paste", json!({ "sheet_id": "screen-col", "text": "999", "mode": "insert_rows", "start_row": 0, "start_col": 0 }));
    let (g12_s, a01_s) = (CellCoord::new(g12.row + 1, g12.col, g12.subcol), CellCoord::new(a01.row + 1, a01.col, a01.subcol));
    assert_eq!((ds(session, "screen-col").plate_well_origin(g12_s), ds(session, "screen-col").plate_well_origin(a01_s)), (Some("G12"), Some("A01")));

    ok_cmd(
        session,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": "screen-col", "row": g12_s.row, "col": g12_s.col, "subcol": g12_s.subcol, "reason_tag": "qc_bubble", "note": "Air bubble observed in well G12" }),
    );
    assert_eq!(ds(session, "screen-col").plate_well_origin(g12_s), Some("G12"));

    ok_cmd(
        session,
        "table.setCell",
        json!({ "sheet_id": "screen-col", "row": a01_s.row, "col": a01_s.col, "subcol": a01_s.subcol, "value": 123.45 }),
    );
    assert_eq!(ds(session, "screen-col").plate_well_origin(a01_s), None);
    assert!(session.undo()? && ds(session, "screen-col").plate_well_origin(a01_s) == Some("A01"));
    assert!(session.redo()? && ds(session, "screen-col").plate_well_origin(a01_s).is_none());

    let reopened = open_project_bytes(&save_project_bytes(session.document())?)?;
    validate_project_document(&reopened)?;
    assert_eq!(reopened.schema_version, 8);
    let r_scr = reopened.data_sheet(&SheetId::new("screen-col")).ok_or("reopened screen-col")?;
    assert_eq!((r_scr.plate_well_origin(g12_s), r_scr.plate_well_origin(a01_s)), (Some("G12"), None));
    assert!(r_scr.cell(g12_s).is_some_and(|c| c.excluded));
    Ok(())
}

#[test]
fn slice1_gate_stacked_xlsx_auc_layout_plate_map_normalize_anova_and_v8_lifecycle() -> TestResult {
    let mut session = Session::new();
    let imp = ok_cmd(
        &mut session,
        "table.importXlsx",
        json!({ "bytes_base64": encode_base64(&build_synthetic_stacked_kinetic_xlsx()?), "skip_rows": 3, "split_by_column": 0, "extract_preamble_info": true }),
    );
    let kinetic_ids = imported_ids(&imp.payload)?;
    assert_eq!(kinetic_ids.len(), 2);
    let info_s = info(&session, imp.payload.get("info_sheet_id").and_then(Value::as_str).ok_or("info_sheet_id")?);
    assert!(
        info_s.linked_data_sheets.len() == 2
            && info_s.get_constant("Protocol").is_some()
            && info_s.get_constant("Temperature").is_some()
            && info_s.notes_markdown.contains("Equilibrated")
    );
    let ch_a = kinetic_ids.first().ok_or("ch A")?.clone();

    let auc = ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": ch_a, "sheet_id": "auc-ch-a", "kind": "auc", "baseline": 0.0, "x_min": 10.0, "x_max": 50.0 }),
    );
    assert!(auc.mutated && rs(&session, "auc-ch-a").status == ResultStatus::UpToDate && rs(&session, "auc-ch-a").analyzable_data.is_some());

    let plain = generate_methods_report(session.document(), &SheetId::new("auc-ch-a"), ReportFormat::PlainText)?;
    assert!(plain.contains("Yeh & Kwan") && plain.contains("Bailer (1988)"));
    for fmt in [ReportFormat::Markdown, ReportFormat::Latex] {
        let r = generate_methods_report(session.document(), &SheetId::new("auc-ch-a"), fmt)?;
        assert!(r.contains("Yeh") && r.contains("Kwan") && r.contains("Bailer"));
    }
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "auc-ch-a", "baseline": "first_point", "x_min": null }));
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "auc-ch-a", "baseline": { "mode": "fixed", "value": 0.0 }, "x_min": 10.0, "x_max": 50.0 }),
    );

    let imp_lay = ok_cmd(
        &mut session,
        "table.importXlsx",
        json!({ "bytes_base64": encode_base64(&build_synthetic_merged_layout_xlsx()?), "merge": true, "has_header": false, "cell_range": "B4:M11", "fill_merged_cells": true }),
    );
    let layout_ids = imported_ids(&imp_lay.payload)?;
    assert_eq!(layout_ids.len(), 1);
    let layout_id = layout_ids.first().ok_or("layout_id")?.clone();
    assert_eq!((ds(&session, &layout_id).archetype.row_count(), ds(&session, &layout_id).archetype.column_count()), (8, 12));

    // Stale-result freshness guard: Dirty ResultSheet refused with ErrorCode::Conflict.
    let mut stale_doc = session.document().clone();
    if let Some(r) = stale_doc.result_sheet_mut(&SheetId::new("auc-ch-a")) {
        r.status = ResultStatus::Dirty;
    }
    assert_err_code(
        &mut Session::from_document(stale_doc),
        "table.importPlateMap",
        json!({ "sheet_id": "screen-stale", "title": "Stale Attempt", "source": { "sheet_id": "auc-ch-a", "value_column": "Net Area" }, "layout_sheet_id": layout_id, "blank_labels": ["Buffer"], "positive_control_labels": ["Reference"], "target_kind": "Column" }),
        ErrorCode::Conflict,
    );

    let plate = ok_cmd(
        &mut session,
        "table.importPlateMap",
        json!({ "sheet_id": "screen-col", "title": "Screen AUC", "source": { "sheet_id": "auc-ch-a", "value_column": "Net Area" }, "layout_sheet_id": layout_id, "blank_labels": ["Buffer"], "positive_control_labels": ["Reference"], "target_kind": "Column", "subtract_blank_mean": false }),
    );
    let scr = ds(&session, "screen-col");
    let TableArchetype::Column(col_tbl) = &scr.archetype else { return Err("expected Column".into()) };
    for (title, exp) in [
        ("Compound 1", vec![320.0; 4]),
        ("Compound 2", vec![-80.0; 4]),
        ("Compound 3", vec![200.0; 4]),
        ("Compound 4", vec![240.0, 240.0, 240.0, 248.0]),
        ("Positive Control", vec![400.0; 4]),
        ("Blank", vec![20.0; 4]),
    ] {
        let got: Vec<f64> = col_tbl
            .data_sets
            .iter()
            .find(|d| d.title == title)
            .ok_or(title)?
            .cells
            .iter()
            .filter_map(|r| r.first().and_then(|c| c.active_number()))
            .collect();
        assert_eq!(got, exp, "{title}");
    }
    assert!(scr.floating_notes.iter().any(|n| n.body_markdown.contains("auc-ch-a") && n.body_markdown.contains("Net Area")));
    let (g12, a01) = (find_well_coord(&plate.payload, "G12")?, find_well_coord(&plate.payload, "A01")?);
    assert_eq!(
        (scr.plate_well_origin(g12), scr.cell(g12).and_then(|c| c.active_number()), scr.plate_well_origin(a01)),
        (Some("G12"), Some(248.0), Some("A01"))
    );

    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": "screen-col", "sheet_id": "norm-screen", "kind": "normalize", "zero_anchor": "ColumnMin", "hundred_anchor": "ColumnMax", "per_data_set": false }),
    );
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": "screen-col", "sheet_id": "anova-screen", "kind": "one_way_anova", "selected_data_sets": [0, 1, 2, 3, 4, 5], "multiplicity": "dunnett", "control_column": 5 }),
    );
    assert_eq!(
        (rs(&session, "norm-screen").status.clone(), rs(&session, "anova-screen").status.clone()),
        (ResultStatus::UpToDate, ResultStatus::UpToDate)
    );
    verify_slice1_snapshot_and_v8(&mut session, &ch_a, a01, g12)
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
    let (mut label_grid, mut x_grid, mut sig) = (vec![vec![None; 12]; 8], vec![vec![None; 12]; 8], vec![0.0_f64; 96]);
    let mut put = |r: usize, c: usize, lbl: &str, x: Option<f64>, val: f64| {
        if let Some(s) = label_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
            *s = Some(lbl.into());
        }
        if let Some(s) = x_grid.get_mut(r).and_then(|row| row.get_mut(c)) {
            *s = x;
        }
        if let Some(s) = sig.get_mut(r * 12 + c) {
            *s = val;
        }
    };
    for r in 0..8 {
        let lx = -6.0 - (r as f64);
        let y = hill_4pl(lx, 10.0, 110.0, -9.0, 1.0);
        for c in 0..3 {
            put(r, c, "Agonist A", Some(lx), y);
        }
    }
    for r in 0..7 {
        let lx = -7.0 - (r as f64);
        let y = hill_4pl(lx, 10.0, 90.0, -10.2, 1.0);
        for c in 3..6 {
            put(r, c, "Agonist B", Some(lx), y);
        }
    }
    put(6, 11, "Agonist A", Some(-12.0), 5000.0);
    for c in 9..12 {
        put(7, c, "Buffer", None, 0.0);
    }

    let mut csv = String::from("Assay: Shifted-Ladder Kinetic CRC,,,,,\nTemperature: 25.5,,,,,\nPlate equilibrated prior to read,,,,,\nChannel,Time");
    for idx in 0..96 {
        let _ = write!(csv, ",{}", well_label_96(idx));
    }
    csv.push('\n');
    for (ch, scale, off) in [("Em460", 1.0, 20.0), ("Em530", 0.5, 5.0)] {
        for t in [0.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0] {
            let _ = write!(csv, "{ch},{t}");
            for &d in &sig {
                let _ = write!(csv, ",{:.6}", off + scale * d);
            }
            csv.push('\n');
        }
        csv.push('\n');
    }
    Slice2Fixture { label_grid, x_grid, csv }
}

fn map_exclude_g12_and_verify_4pl_log_ec50(session: &mut Session, f: &Slice2Fixture, plate_id: &str, fit_id: &str, src: Value) -> TestResult {
    let plate = ok_cmd(
        session,
        "table.importPlateMap",
        json!({ "sheet_id": plate_id, "title": plate_id, "source": src, "label_grid": f.label_grid, "x_grid": f.x_grid, "blank_labels": ["Buffer"], "x_title": "Log[Agonist] (M)", "target_kind": "Xy" }),
    );
    let g12 = find_well_coord(&plate.payload, "G12")?;
    assert_eq!(ds(session, plate_id).plate_well_origin(g12), Some("G12"));
    ok_cmd(
        session,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": plate_id, "row": g12.row, "col": g12.col, "subcol": g12.subcol, "reason_tag": "pipetting_outlier", "note": "Anomalous spike in well G12" }),
    );
    assert_eq!(ds(session, plate_id).plate_well_origin(g12), Some("G12"));
    ok_cmd(session, "fit.run", json!({ "source_sheet_id": plate_id, "sheet_id": fit_id, "kind": "log_agonist_vs_response_4pl" }));
    assert_eq!(rs(session, fit_id).status, ResultStatus::UpToDate);
    assert!((scalar_ds(session, fit_id, "LogEC50", Some(0)).ok_or("A LogEC50")? - (-9.0)).abs() < 1e-3);
    assert!((scalar_ds(session, fit_id, "LogEC50", Some(1)).ok_or("B LogEC50")? - (-10.2)).abs() < 1e-3);
    Ok(())
}

#[test]
fn slice2_gate_stacked_csv_remove_baseline_auc_and_x_window_mean_shifted_ladder_4pl() -> TestResult {
    let f = build_slice2_shifted_ladder_fixture();
    let mut session = Session::new();
    for bad in [
        json!({ "csv": f.csv, "filter": { "row_start": 3, "row_match": { "column": 0, "equals": "Em460" } }, "split_by_column": 0 }),
        json!({ "csv": f.csv, "filter": { "row_start": 3 }, "split_by_column": 0, "link": true }),
    ] {
        assert_err_code(&mut session, "table.importCsv", bad, ErrorCode::BadInput);
    }

    let imp = ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "kin", "title": "Kinetic CRC", "csv": f.csv, "archetype": "xy", "filter": { "row_start": 3 }, "split_by_column": 0, "extract_preamble_info": true }),
    );
    let csv_ids = imported_ids(&imp.payload)?;
    assert!(csv_ids.len() == 2 && imp.payload.get("info_sheet_id").and_then(Value::as_str).is_some());
    let em460 = csv_ids.first().ok_or("Em460")?.clone();
    assert_err_code(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": em460, "csv": f.csv, "filter": { "row_start": 3 }, "split_by_column": 0 }),
        ErrorCode::BadInput,
    );

    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": em460, "sheet_id": "delta-em460", "kind": "remove_baseline", "baseline_columns": [93, 94, 95], "operation": "difference" }),
    );
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": "delta-em460", "sheet_id": "auc-em460", "kind": "auc", "baseline": 0.0, "x_min": 15.0, "x_max": 75.0 }),
    );
    assert_eq!(
        (rs(&session, "delta-em460").status.clone(), rs(&session, "auc-em460").status.clone()),
        (ResultStatus::UpToDate, ResultStatus::UpToDate)
    );

    map_exclude_g12_and_verify_4pl_log_ec50(
        &mut session,
        &f,
        "crc-auc",
        "fit-crc-auc",
        json!({ "sheet_id": "auc-em460", "value_column": "Net Area" }),
    )?;
    map_exclude_g12_and_verify_4pl_log_ec50(
        &mut session,
        &f,
        "crc-win",
        "fit-crc-win",
        json!({ "sheet_id": "delta-em460", "reducer": { "x_window_mean": { "x_min": 15.0, "x_max": 75.0 } } }),
    )?;
    for (sid, red) in [("red-min", json!("min")), ("red-max", json!("max")), ("red-row", json!({ "row_index": 2 }))] {
        assert!(ok_cmd(&mut session, "table.importPlateMap", json!({ "sheet_id": sid, "source": { "sheet_id": "delta-em460", "reducer": red }, "label_grid": f.label_grid, "x_grid": f.x_grid, "blank_labels": ["Buffer"], "target_kind": "Column" })).mutated);
    }
    Ok(())
}

#[test]
fn plate_map_cbplate_v1_forward_refusal_legacy_replicate_default_and_csv_new_sheet_filter() -> TestResult {
    let mut session = Session::new();
    let mut wells = vec![PlateWellRole::Excluded; 96];
    for (idx, rep) in [(0, 0), (1, 1)] {
        if let Some(w) = wells.get_mut(idx) {
            *w = PlateWellRole::Treatment { group_name: "Drug".into(), dilution_x: -6.0, replicate: rep };
        }
    }
    let cbplate_json = serialize_cbplate_json(&PlateMapFile::new("V1 Plate", PlateGeometry::Wells96, "Log[M]", wells)?, false)?;
    let mut values = vec![None; 96];
    for (idx, val) in [(0, 42.0), (1, 48.0)] {
        if let Some(v) = values.get_mut(idx) {
            *v = Some(val);
        }
    }
    let cbplate_obj: Value = serde_json::from_str(&cbplate_json)?;
    assert!(
        ok_cmd(
            &mut session,
            "table.importPlateMap",
            json!({ "sheet_id": "cbplate-str", "values": values, "template": cbplate_json, "target_kind": "Column" })
        )
        .mutated
    );
    assert!(
        ok_cmd(
            &mut session,
            "table.importPlateMap",
            json!({ "sheet_id": "cbplate-obj", "values": values, "template": cbplate_obj, "target_kind": "Column" })
        )
        .mutated
    );

    let mut v2_obj: Value = serde_json::from_str(&cbplate_json)?;
    if let Some(m) = v2_obj.as_object_mut() {
        m.insert("format_version".into(), json!(2));
    }
    assert!(
        session
            .execute(
                "table.importPlateMap",
                json!({ "sheet_id": "cbplate-v2", "values": values, "template": serde_json::to_string(&v2_obj)?, "target_kind": "Column" })
            )
            .is_err()
    );

    let mut legacy_roles = vec![json!("excluded"); 96];
    if let Some(r0) = legacy_roles.first_mut() {
        *r0 = json!({ "role": "treatment", "group_name": "ArmA", "dilution_x": 1.0 });
    }
    ok_cmd(
        &mut session,
        "table.importPlateMap",
        json!({ "sheet_id": "legacy-mv", "geometry": 96, "values": values, "roles": legacy_roles, "target_kind": "MultipleVariables" }),
    );
    let TableArchetype::MultipleVariables(mv) = &ds(&session, "legacy-mv").archetype else { return Err("expected MV".into()) };
    assert_eq!(mv.variables.iter().find(|v| v.title == "Replicate").and_then(|v| v.cells.first()).map(|c| &c.value), Some(&CellValue::Number(1.0)));

    let csv_text = "Meta: Test\nChannel,Time,Signal\nCh1,0,10\nCh2,0,99\nCh1,10,20\nCh1,20,30\nCh1,30,40\n";
    let csv_out = ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "filtered-new", "csv": csv_text, "archetype": "xy", "filter": { "row_start": 1, "row_match": { "column": 0, "equals": "Ch1", "drop_column": true }, "decimate_every_k": 2 } }),
    );
    assert_eq!(csv_out.payload["sheet_id"], "filtered-new");
    let TableArchetype::Xy(xy) = &ds(&session, "filtered-new").archetype else { return Err("expected XY".into()) };
    assert_eq!((xy.row_count(), xy.x_value_at(0), xy.x_value_at(1)), (2, Some(0.0), Some(20.0)));
    assert_eq!(xy.data_sets.first().and_then(|d| d.cells.first()).and_then(|r| r.first()).and_then(|c| c.active_number()), Some(10.0));
    assert_eq!(xy.data_sets.first().and_then(|d| d.cells.get(1)).and_then(|r| r.first()).and_then(|c| c.active_number()), Some(30.0));
    Ok(())
}
