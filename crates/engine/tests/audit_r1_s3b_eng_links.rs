#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, ErrorCode, FitMethod, GroupedTable,
    HookSource, InfoSheet, NonlinearFitSpec, NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, ResultSheet, ResultStatus,
    SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, WeightingScheme, XColumnMode, XyTable,
};
use curvebook_engine::{Session, dag};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let id = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("curvebook-s3b-eng-links-{label}-{}-{id}", std::process::id()));
        if path.exists() {
            let _ = fs::remove_dir_all(&path);
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn import_csv(session: &mut Session, sheet_id: &str, csv: &str) -> TestResult {
    session.execute(
        "table.importCsv",
        json!({
            "sheet_id": sheet_id,
            "csv": csv,
            "has_header": false,
            "archetype": "column",
        }),
    )?;
    Ok(())
}

fn require_ds<'a>(session: &'a Session, sheet_id: &str) -> Result<&'a DataSheet, Box<dyn std::error::Error>> {
    session.document().data_sheet(&SheetId::new(sheet_id)).ok_or_else(|| format!("missing data sheet `{sheet_id}`").into())
}

fn col_nums(session: &Session, sheet_id: &str, col: usize) -> Result<Vec<Option<f64>>, Box<dyn std::error::Error>> {
    let ds = require_ds(session, sheet_id)?;
    let rows = ds.archetype.row_count();
    Ok((0..rows).map(|r| ds.cell(CellCoord::new(r, col, 0)).and_then(|c| c.active_number())).collect())
}

fn subcol_nums(session: &Session, sheet_id: &str, col: usize, subcol: usize) -> Result<Vec<Option<f64>>, Box<dyn std::error::Error>> {
    let ds = require_ds(session, sheet_id)?;
    let rows = ds.archetype.row_count();
    Ok((0..rows).map(|r| ds.cell(CellCoord::new(r, col, subcol)).and_then(|c| c.active_number())).collect())
}

#[test]
fn r1_s07_01_append_rows_link_preserves_user_rows_on_create_refresh_grow_and_shrink() -> TestResult {
    let mut s = Session::new();
    import_csv(&mut s, "dst", "1\n2\n3\n4\n5")?;
    import_csv(&mut s, "src", "100\n200\n300")?;

    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_sheet_id": "src",
            "mode": "append",
        }),
    )?;

    let initial = col_nums(&s, "dst", 0)?;
    assert_eq!(initial, vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(200.0), Some(300.0)]);
    let dst_after_paste = require_ds(&s, "dst")?;
    assert_eq!(dst_after_paste.linked_regions.len(), 1);
    assert_eq!(dst_after_paste.linked_regions[0].target_range, SheetCellRange::new(CellCoord::new(5, 0, 0), CellCoord::new(7, 0, 0)));

    // Source update refreshes only the linked block at rows 5..8.
    s.execute("table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 0, "value": 250.0 }))?;
    assert_eq!(col_nums(&s, "dst", 0)?, vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(250.0), Some(300.0)]);
    assert_eq!(require_ds(&s, "dst")?.linked_regions.len(), 1);

    // Shrinking linked range in source clears trailing linked cells (row 7) without touching user rows 0..5.
    s.execute("table.clearCells", json!({ "sheet_id": "src", "cells": [{ "row": 2, "col": 0 }] }))?;
    let shrunk = col_nums(&s, "dst", 0)?;
    assert_eq!(&shrunk[0..5], &[Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0)]);
    assert_eq!(&shrunk[5..8], &[Some(100.0), Some(250.0), None]);
    assert_eq!(require_ds(&s, "dst")?.linked_regions[0].target_range, SheetCellRange::new(CellCoord::new(5, 0, 0), CellCoord::new(7, 0, 0)));

    // Refilling cleared row in source grows the appended block back in place without clobbering user rows 0..5.
    s.execute("table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 0, "value": 350.0 }))?;
    assert_eq!(col_nums(&s, "dst", 0)?, vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(250.0), Some(350.0)]);
    assert_eq!(require_ds(&s, "dst")?.linked_regions[0].target_range, SheetCellRange::new(CellCoord::new(5, 0, 0), CellCoord::new(7, 0, 0)));
    Ok(())
}

#[test]
fn r1_s07_01b_append_columns_and_grouped_subcol_shrink_refresh_in_place() -> TestResult {
    let mut doc = ProjectDocument::new("AppendColumns & Grouped")?;

    let mut dst_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    dst_tbl.data_sets[0].title = String::from("Baseline");
    dst_tbl.data_sets[0].cells[0][0] = Cell::number(10.0);
    dst_tbl.data_sets[0].cells[1][0] = Cell::number(20.0);
    doc.add_data_sheet(DataSheet::new("dst-col", "Target", TableArchetype::Column(dst_tbl))?)?;

    let mut src_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2)?;
    src_tbl.data_sets[0].title = String::from("TreatA");
    src_tbl.data_sets[1].title = String::from("TreatB");
    src_tbl.data_sets[0].cells[0][0] = Cell::number(1.0);
    src_tbl.data_sets[0].cells[1][0] = Cell::number(2.0);
    src_tbl.data_sets[1].cells[0][0] = Cell::number(3.0);
    src_tbl.data_sets[1].cells[1][0] = Cell::number(4.0);
    doc.add_data_sheet(DataSheet::new("src-col", "Source", TableArchetype::Column(src_tbl))?)?;

    let mut s = Session::from_document(doc);
    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst-col",
            "source_sheet_id": "src-col",
            "mode": "append_columns",
            "propagate_title_header": true,
        }),
    )?;

    assert_eq!(col_nums(&s, "dst-col", 0)?, vec![Some(10.0), Some(20.0)]);
    assert_eq!(col_nums(&s, "dst-col", 1)?, vec![Some(1.0), Some(2.0)]);
    assert_eq!(col_nums(&s, "dst-col", 2)?, vec![Some(3.0), Some(4.0)]);
    let TableArchetype::Column(tbl) = &require_ds(&s, "dst-col")?.archetype else {
        return Err("expected Column table".into());
    };
    assert_eq!(tbl.data_sets[0].title, "Baseline");
    assert_eq!(tbl.data_sets[1].title, "TreatA");
    assert_eq!(tbl.data_sets[2].title, "TreatB");

    s.execute("table.setCell", json!({ "sheet_id": "src-col", "row": 0, "col": 0, "value": 11.0 }))?;
    s.execute("table.setGroupTitles", json!({ "sheet_id": "src-col", "col": 0, "title": "TreatA_v2" }))?;

    assert_eq!(col_nums(&s, "dst-col", 0)?, vec![Some(10.0), Some(20.0)]);
    assert_eq!(col_nums(&s, "dst-col", 1)?, vec![Some(11.0), Some(2.0)]);
    assert_eq!(require_ds(&s, "dst-col")?.linked_regions.len(), 1);
    let TableArchetype::Column(tbl) = &require_ds(&s, "dst-col")?.archetype else {
        return Err("expected Column table".into());
    };
    assert_eq!(tbl.data_sets[0].title, "Baseline");
    assert_eq!(tbl.data_sets[1].title, "TreatA_v2");
    assert_eq!(tbl.data_sets[2].title, "TreatB");

    // Grouped multi-subcolumn shrink clears all columns and subcolumns in the old target_range.
    let mut gdoc = ProjectDocument::new("Grouped Shrink")?;
    let mut g_src = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?;
    for r in 0..2 {
        for c in 0..2 {
            for sub in 0..3 {
                g_src.data_sets[c].cells[r][sub] = Cell::number(((r * 10 + c * 3 + sub) + 1) as f64);
            }
        }
    }
    gdoc.add_data_sheet(DataSheet::new("g-src", "G Source", TableArchetype::Grouped(g_src))?)?;
    let g_dst = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?;
    gdoc.add_data_sheet(DataSheet::new("g-dst", "G Dest", TableArchetype::Grouped(g_dst))?)?;

    let mut gs = Session::from_document(gdoc);
    gs.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "g-dst",
            "source_sheet_id": "g-src",
            "mode": "overwrite",
            "start_row": 0,
            "start_col": 0,
        }),
    )?;
    assert_eq!(subcol_nums(&gs, "g-dst", 1, 2)?[1], Some(16.0));

    // Clear dataset 1 and row 1 from g-src so linked range shrinks to 1 row x 1 col x 3 subcols.
    gs.execute(
        "table.clearCells",
        json!({
            "sheet_id": "g-src",
            "cells": [
                { "row": 1, "col": 0, "subcol": 0 },
                { "row": 1, "col": 0, "subcol": 1 },
                { "row": 1, "col": 0, "subcol": 2 },
                { "row": 0, "col": 1, "subcol": 0 },
                { "row": 0, "col": 1, "subcol": 1 },
                { "row": 0, "col": 1, "subcol": 2 },
                { "row": 1, "col": 1, "subcol": 0 },
                { "row": 1, "col": 1, "subcol": 1 },
                { "row": 1, "col": 1, "subcol": 2 },
            ],
        }),
    )?;

    assert_eq!(subcol_nums(&gs, "g-dst", 0, 0)?, vec![Some(1.0), None]);
    assert_eq!(subcol_nums(&gs, "g-dst", 0, 2)?, vec![Some(3.0), None]);
    assert_eq!(subcol_nums(&gs, "g-dst", 1, 0)?, vec![None, None]);
    assert_eq!(subcol_nums(&gs, "g-dst", 1, 2)?, vec![None, None]);
    Ok(())
}

#[test]
fn r1_s07_03_external_delimited_link_clears_stale_rows_on_shrink_and_append() -> TestResult {
    let dir = TempDirGuard::new("external-refresh")?;
    let plate_path = dir.path().join("plate.csv");
    fs::write(&plate_path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")?;

    let mut s = Session::new();
    import_csv(&mut s, "d", "0")?;
    s.execute(
        "table.pasteLink",
        json!({
            "target_sheet_id": "d",
            "path": plate_path.to_string_lossy(),
            "delimiter": ",",
        }),
    )?;
    assert_eq!(col_nums(&s, "d", 0)?, (1..=10).map(|v| Some(f64::from(v))).collect::<Vec<_>>());

    // Rewrite shorter CSV (8 rows instead of 10): trailing rows 8..10 must clear on recalculate.
    fs::write(&plate_path, "11\n12\n13\n14\n15\n16\n17\n18\n")?;
    let recalc = s.execute("project.recalculate", json!({}))?;
    assert_eq!(recalc.payload["refreshed_links"], json!(1));
    let after = col_nums(&s, "d", 0)?;
    assert_eq!(&after[0..8], &[Some(11.0), Some(12.0), Some(13.0), Some(14.0), Some(15.0), Some(16.0), Some(17.0), Some(18.0)]);
    assert_eq!(&after[8..10], &[None, None]);
    let ds = require_ds(&s, "d")?;
    assert_eq!(ds.linked_regions.len(), 1);
    assert_eq!(ds.linked_regions[0].target_range, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(7, 0, 0)));

    // Missing external CSV fails atomically without clearing previously loaded rows.
    fs::remove_file(&plate_path)?;
    assert!(s.execute("project.recalculate", json!({})).is_err());
    assert_eq!(&col_nums(&s, "d", 0)?[0..8], &[Some(11.0), Some(12.0), Some(13.0), Some(14.0), Some(15.0), Some(16.0), Some(17.0), Some(18.0)]);

    // External CSV appended via table.importCsv { link: true, mode: "append" } preserves user rows when growing & shrinking.
    let append_path = dir.path().join("plate_append.csv");
    fs::write(&append_path, "100\n200\n300\n")?;
    let mut s_app = Session::new();
    import_csv(&mut s_app, "app", "1\n2")?;
    s_app.execute(
        "table.importCsv",
        json!({
            "sheet_id": "app",
            "path": append_path.to_string_lossy(),
            "has_header": false,
            "mode": "append",
            "link": true,
        }),
    )?;
    assert_eq!(col_nums(&s_app, "app", 0)?, vec![Some(1.0), Some(2.0), Some(100.0), Some(200.0), Some(300.0)]);

    fs::write(&append_path, "105\n205\n305\n405\n")?;
    s_app.execute("project.recalculate", json!({}))?;
    assert_eq!(col_nums(&s_app, "app", 0)?, vec![Some(1.0), Some(2.0), Some(105.0), Some(205.0), Some(305.0), Some(405.0)]);

    fs::write(&append_path, "110\n220\n")?;
    s_app.execute("project.recalculate", json!({}))?;
    assert_eq!(col_nums(&s_app, "app", 0)?, vec![Some(1.0), Some(2.0), Some(110.0), Some(220.0), None, None]);
    assert_eq!(require_ds(&s_app, "app")?.linked_regions.len(), 1);
    assert_eq!(require_ds(&s_app, "app")?.linked_regions[0].target_range, SheetCellRange::new(CellCoord::new(2, 0, 0), CellCoord::new(3, 0, 0)));
    Ok(())
}

#[test]
fn r1_s07_08a_calculated_variables_never_fall_back_to_unrelated_info_sheet() -> TestResult {
    let mut s = Session::new();
    // Unrelated Info sheet added FIRST so `info_sheets.first()` would return K=10 if fallback existed.
    s.execute("sheet.createData", json!({ "sheet_id": "plate2", "info_sheet_id": "plate2-info", "auto_info": true }))?;
    s.execute("info.setConstant", json!({ "sheet_id": "plate2-info", "name": "K", "value": 10.0 }))?;

    s.execute(
        "sheet.createData",
        json!({ "sheet_id": "mv", "archetype": "multiple_variables", "cols": 2, "rows": 3, "info_sheet_id": "mv-info", "auto_info": true }),
    )?;
    s.execute("table.setGroupTitles", json!({ "sheet_id": "mv", "col": 0, "title": "Raw" }))?;
    s.execute("info.setConstant", json!({ "sheet_id": "mv-info", "name": "K", "value": 3.0 }))?;
    s.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv", "title": "C", "formula": "=[Raw] * Info(\"K\")" }))?;
    s.execute("table.setCell", json!({ "sheet_id": "mv", "row": 0, "col": 0, "value": 1.0 }))?;
    assert_eq!(col_nums(&s, "mv", 2)?[0], Some(3.0));

    // `cascade: false` rejects deleting referenced InfoSheet with Conflict.
    let err_no_cascade = s.execute("sheet.delete", json!({ "sheet_id": "mv-info", "cascade": false })).err().ok_or("cascade=false must refuse")?;
    assert_eq!(err_no_cascade.code, ErrorCode::Conflict);
    assert!(s.document().info_sheet(&SheetId::new("mv-info")).is_some());
    assert_eq!(col_nums(&s, "mv", 2)?[0], Some(3.0));

    // Default `sheet.delete` on `mv-info` while calculated variable requires `Info("K")` refuses cleanly rather than rebinding to `plate2-info`.
    let err_delete = s.execute("sheet.delete", json!({ "sheet_id": "mv-info" })).err().ok_or("delete breaking calc formula must refuse")?;
    assert!(err_delete.message().contains("failed"));
    assert!(s.document().info_sheet(&SheetId::new("mv-info")).is_some());
    assert_eq!(col_nums(&s, "mv", 2)?[0], Some(3.0));

    // Even if `mv.info_sheet_id` becomes None directly, DAG recalculation fails instead of silently reading `plate2-info`.
    let mut broken_doc = s.document().clone();
    broken_doc.data_sheet_mut(&SheetId::new("mv")).ok_or("missing mv")?.info_sheet_id = None;
    let err_recalc = dag::recalculate_project(&mut broken_doc).err().ok_or("unbound Info constant must fail")?;
    assert!(err_recalc.message().contains("failed"));

    // Deleting an InfoSheet whose paired DataSheet has NO Info(...) formula succeeds and leaves info_sheet_id unbound.
    let mut s_plain = Session::new();
    s_plain.execute("sheet.createData", json!({ "sheet_id": "other", "info_sheet_id": "other-info", "auto_info": true }))?;
    s_plain.execute("info.setConstant", json!({ "sheet_id": "other-info", "name": "K", "value": 99.0 }))?;
    s_plain.execute(
        "sheet.createData",
        json!({ "sheet_id": "mv-no-info", "archetype": "multiple_variables", "cols": 1, "rows": 2, "info_sheet_id": "mv-plain-info", "auto_info": true }),
    )?;
    s_plain.execute("table.setGroupTitles", json!({ "sheet_id": "mv-no-info", "col": 0, "title": "Raw" }))?;
    s_plain.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-no-info", "title": "Dbl", "formula": "=[Raw] * 2" }))?;
    s_plain.execute("table.setCell", json!({ "sheet_id": "mv-no-info", "row": 0, "col": 0, "value": 4.0 }))?;
    s_plain.execute("sheet.delete", json!({ "sheet_id": "mv-plain-info" }))?;
    assert!(s_plain.document().info_sheet(&SheetId::new("mv-plain-info")).is_none());
    assert_eq!(require_ds(&s_plain, "mv-no-info")?.info_sheet_id, None);
    assert_eq!(col_nums(&s_plain, "mv-no-info", 1)?[0], Some(8.0));
    Ok(())
}

#[test]
fn r1_s07_08b_fit_and_transform_hooks_strictly_require_bound_info_sheet() -> TestResult {
    let mut doc = ProjectDocument::new("Hook Binding Regression")?;

    // Unrelated foreign InfoSheet added FIRST so `info_sheets.first()` would yield Top/Scale = 1000.0 if fallback existed.
    let mut plate2_info = InfoSheet::new_preseeded("plate2-info", "Plate 2 Info")?;
    plate2_info.set_constant("Scale", ConstantValue::Number(1000.0), None)?;
    plate2_info.set_constant("Top", ConstantValue::Number(1000.0), None)?;
    doc.add_info_sheet(plate2_info)?;

    let mut curve_info = InfoSheet::new_preseeded("curve-info", "Curve Info")?;
    curve_info.set_constant("Scale", ConstantValue::Number(50.0), None)?;
    curve_info.set_constant("Top", ConstantValue::Number(50.0), None)?;
    doc.add_info_sheet(curve_info)?;

    // Clean log10(dose) -> 4PL response data scaled around Top = 50.0.
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 6)?;
    let doses = [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    let resp = [2.0, 5.0, 15.0, 35.0, 46.0, 49.0];
    for (i, (&x, &y)) in doses.iter().zip(resp.iter()).enumerate() {
        xy.set_x_cell(i, Cell::number(x))?;
        xy.data_sets[0].cells[i][0] = Cell::number(y);
    }
    let mut ds = DataSheet::new("xy-1", "Dose Response", TableArchetype::Xy(xy))?;
    ds.info_sheet_id = Some(SheetId::new("curve-info"));
    doc.add_data_sheet(ds)?;

    // Second sheet has no bound info_sheet_id and hosts an explicit InfoConstant { info_sheet_id: Some("curve-info") } fit.
    let mut xy2 = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 6)?;
    for (i, (&x, &y)) in doses.iter().zip(resp.iter()).enumerate() {
        xy2.set_x_cell(i, Cell::number(x))?;
        xy2.data_sets[0].cells[i][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("xy-2", "Dose Response 2", TableArchetype::Xy(xy2))?)?;

    // 1. Transform using unbound InfoConstant { info_sheet_id: None } (inherits xy-1's curve-info).
    let xform_spec = AnalysisSpec::new(
        "res-xform",
        "xy-1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Scale") }),
            output_percent: true,
            per_data_set: false,
        }),
    )?;
    doc.add_result_sheet(ResultSheet::new(xform_spec)?)?;

    // 2. Direct NonlinearRegression using unbound InfoConstant { info_sheet_id: None }.
    let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl.method = FitMethod::LeastSquares;
    nl.weighting = WeightingScheme::None;
    nl.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Top") }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let fit_spec = AnalysisSpec::new("res-fit", "xy-1", AnalysisKind::NonlinearRegression(nl.clone()))?;
    doc.add_result_sheet(ResultSheet::new(fit_spec)?)?;

    // 3. Chained NonlinearRegression downstream of `res-xform` walking back to `xy-1`'s `curve-info`.
    let chain_spec = AnalysisSpec::new("res-chain", "res-xform", AnalysisKind::NonlinearRegression(nl))?;
    doc.add_result_sheet(ResultSheet::new(chain_spec)?)?;

    // 4. Explicit InfoConstant { info_sheet_id: Some("curve-info") } on `xy-2` (which has info_sheet_id: None).
    let mut nl_explicit = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    nl_explicit.method = FitMethod::LeastSquares;
    nl_explicit.weighting = WeightingScheme::None;
    nl_explicit.parameter_rules.push(ParameterRule {
        name: String::from("Top"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("curve-info")),
            constant_name: String::from("Top"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let exp_spec = AnalysisSpec::new("res-explicit", "xy-2", AnalysisKind::NonlinearRegression(nl_explicit))?;
    doc.add_result_sheet(ResultSheet::new(exp_spec)?)?;

    dag::recalculate_project(&mut doc)?;
    for rid in ["res-xform", "res-fit", "res-chain", "res-explicit"] {
        let res = doc.result_sheet(&SheetId::new(rid)).ok_or_else(|| format!("missing {rid}"))?;
        assert_eq!(res.status, ResultStatus::UpToDate, "{rid} initially UpToDate");
    }

    let mut s = Session::from_document(doc);

    // `cascade: false` blocks deleting `curve-info` when hooked by fits/transforms.
    let err_no_cascade =
        s.execute("sheet.delete", json!({ "sheet_id": "curve-info", "cascade": false })).err().ok_or("cascade=false must conflict")?;
    assert_eq!(err_no_cascade.code, ErrorCode::Conflict);

    // Default delete leaves `xy-1.info_sheet_id` unbound and recalculates all dependent hooks to ResultStatus::Failed.
    s.execute("sheet.delete", json!({ "sheet_id": "curve-info" }))?;
    assert!(s.document().info_sheet(&SheetId::new("curve-info")).is_none());
    assert_eq!(require_ds(&s, "xy-1")?.info_sheet_id, None);
    for rid in ["res-xform", "res-fit", "res-chain", "res-explicit"] {
        let res = s.document().result_sheet(&SheetId::new(rid)).ok_or_else(|| format!("missing {rid}"))?;
        assert!(matches!(res.status, ResultStatus::Failed { .. }), "expected {rid} to fail after bound InfoSheet deletion, got {:?}", res.status);
        assert!(res.fitted_curves.is_empty());
    }

    // Subsequent edits on `xy-1` must keep those analyses Failed rather than falling back to `plate2-info`.
    s.execute("table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 2.5 }))?;
    for rid in ["res-xform", "res-fit", "res-chain", "res-explicit"] {
        let res = s.document().result_sheet(&SheetId::new(rid)).ok_or_else(|| format!("missing {rid}"))?;
        assert!(matches!(res.status, ResultStatus::Failed { .. }), "{rid} must remain Failed after edit, got {:?}", res.status);
    }
    Ok(())
}
