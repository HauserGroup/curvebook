#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, ConstantValue, ErrorCode, FitMethod, GroupedTable, HookSource, InfoSheet, NonlinearFitSpec,
    NormalizeAnchor, ParameterRule, ParameterScope, ProjectDocument, ResultSheet, ResultStatus, SheetCellRange, SheetId, SubcolumnMode,
    TableArchetype, TransformAnalysisSpec, WeightingScheme,
};
use curvebook_engine::{Session, dag};
use serde_json::json;

use super::common::{TempDir, assert_err_code, col_sheet, ds, grouped_rep_sheet, ok_cmd, rs, run_cmds, session_with, xy_sheet};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn import_csv(s: &mut Session, id: &str, csv: &str) {
    ok_cmd(s, "table.importCsv", json!({ "sheet_id": id, "csv": csv, "has_header": false, "archetype": "column" }));
}

fn col_opt_nums(s: &Session, id: &str, col: usize) -> Vec<Option<f64>> {
    subcol_opt_nums(s, id, col, 0)
}

fn subcol_opt_nums(s: &Session, id: &str, col: usize, sub: usize) -> Vec<Option<f64>> {
    let sheet = ds(s, id);
    (0..sheet.archetype.row_count()).map(|r| sheet.cell(CellCoord::new(r, col, sub)).and_then(|c| c.active_number())).collect()
}

#[test]
fn r1_s07_01_append_rows_link_preserves_user_rows_on_create_refresh_grow_and_shrink() -> TestResult {
    let mut s = Session::new();
    import_csv(&mut s, "dst", "1\n2\n3\n4\n5");
    import_csv(&mut s, "src", "100\n200\n300");
    ok_cmd(&mut s, "table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "src", "mode": "append" }));

    let exp_range = SheetCellRange::new(CellCoord::new(5, 0, 0), CellCoord::new(7, 0, 0));
    assert_eq!(col_opt_nums(&s, "dst", 0), vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(200.0), Some(300.0)]);
    assert_eq!((ds(&s, "dst").linked_regions.len(), ds(&s, "dst").linked_regions[0].target_range), (1, exp_range));

    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "src", "row": 1, "col": 0, "value": 250.0 }));
    assert_eq!(col_opt_nums(&s, "dst", 0), vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(250.0), Some(300.0)]);

    ok_cmd(&mut s, "table.clearCells", json!({ "sheet_id": "src", "cells": [{ "row": 2, "col": 0 }] }));
    assert_eq!(col_opt_nums(&s, "dst", 0), vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(250.0), None]);
    assert_eq!(ds(&s, "dst").linked_regions[0].target_range, exp_range);

    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 0, "value": 350.0 }));
    assert_eq!(col_opt_nums(&s, "dst", 0), vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(100.0), Some(250.0), Some(350.0)]);
    assert_eq!(ds(&s, "dst").linked_regions[0].target_range, exp_range);
    Ok(())
}

#[test]
fn r1_s07_01b_append_columns_and_grouped_subcol_shrink_refresh_in_place() -> TestResult {
    let mut s = session_with(
        "AppendColumns",
        vec![
            col_sheet("dst-col", "Target", &[("Baseline", &[10.0, 20.0])]),
            col_sheet("src-col", "Source", &[("TreatA", &[1.0, 2.0]), ("TreatB", &[3.0, 4.0])]),
        ],
    );
    ok_cmd(
        &mut s,
        "table.pasteLink",
        json!({ "target_sheet_id": "dst-col", "source_sheet_id": "src-col", "mode": "append_columns", "propagate_title_header": true }),
    );
    assert_eq!(
        (col_opt_nums(&s, "dst-col", 0), col_opt_nums(&s, "dst-col", 1), col_opt_nums(&s, "dst-col", 2)),
        (vec![Some(10.0), Some(20.0)], vec![Some(1.0), Some(2.0)], vec![Some(3.0), Some(4.0)])
    );
    let titles = |sess: &Session| match &ds(sess, "dst-col").archetype {
        TableArchetype::Column(t) => t.data_sets.iter().map(|d| d.title.clone()).collect::<Vec<_>>(),
        _ => panic!("expected Column"),
    };
    assert_eq!(titles(&s), vec!["Baseline", "TreatA", "TreatB"]);

    run_cmds(
        &mut s,
        &[
            ("table.setCell", json!({ "sheet_id": "src-col", "row": 0, "col": 0, "value": 11.0 })),
            ("table.setGroupTitles", json!({ "sheet_id": "src-col", "col": 0, "title": "TreatA_v2" })),
        ],
    );
    assert_eq!(
        (col_opt_nums(&s, "dst-col", 0), col_opt_nums(&s, "dst-col", 1), ds(&s, "dst-col").linked_regions.len()),
        (vec![Some(10.0), Some(20.0)], vec![Some(11.0), Some(2.0)], 1)
    );
    assert_eq!(titles(&s), vec!["Baseline", "TreatA_v2", "TreatB"]);

    let g_src = grouped_rep_sheet(
        "g-src",
        "G Source",
        &["R0", "R1"],
        &[("A", &[&[1.0, 2.0, 3.0], &[11.0, 12.0, 13.0]]), ("B", &[&[4.0, 5.0, 6.0], &[14.0, 15.0, 16.0]])],
    );
    let g_dst = curvebook_doc::DataSheet::new("g-dst", "G Dest", TableArchetype::Grouped(GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2)?))?;
    let mut gs = session_with("Grouped Shrink", vec![g_src, g_dst]);
    ok_cmd(
        &mut gs,
        "table.pasteLink",
        json!({ "target_sheet_id": "g-dst", "source_sheet_id": "g-src", "mode": "overwrite", "start_row": 0, "start_col": 0 }),
    );
    assert_eq!(subcol_opt_nums(&gs, "g-dst", 1, 2)[1], Some(16.0));

    let clear_cells: Vec<_> =
        [(1, 0), (0, 1), (1, 1)].into_iter().flat_map(|(r, c)| (0..3).map(move |sc| json!({ "row": r, "col": c, "subcol": sc }))).collect();
    ok_cmd(&mut gs, "table.clearCells", json!({ "sheet_id": "g-src", "cells": clear_cells }));
    assert_eq!(
        (
            subcol_opt_nums(&gs, "g-dst", 0, 0),
            subcol_opt_nums(&gs, "g-dst", 0, 2),
            subcol_opt_nums(&gs, "g-dst", 1, 0),
            subcol_opt_nums(&gs, "g-dst", 1, 2)
        ),
        (vec![Some(1.0), None], vec![Some(3.0), None], vec![None, None], vec![None, None])
    );
    Ok(())
}

#[test]
fn r1_s07_03_external_delimited_link_clears_stale_rows_on_shrink_and_append() -> TestResult {
    let dir = TempDir::try_new("external-refresh")?;
    let (plate_path, append_path) = (dir.child("plate.csv"), dir.child("plate_append.csv"));
    fs::write(&plate_path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n")?;

    let mut s = Session::new();
    import_csv(&mut s, "d", "0");
    ok_cmd(&mut s, "table.pasteLink", json!({ "target_sheet_id": "d", "source_path": plate_path.to_string_lossy(), "delimiter": "," }));
    assert_eq!(col_opt_nums(&s, "d", 0), (1..=10).map(|v| Some(f64::from(v))).collect::<Vec<_>>());

    fs::write(&plate_path, "11\n12\n13\n14\n15\n16\n17\n18\n")?;
    assert_eq!(ok_cmd(&mut s, "project.recalculate", json!({})).payload["refreshed_links"], json!(1));
    let expected_8: Vec<_> = (11..=18).map(|v| Some(f64::from(v))).collect();
    let after = col_opt_nums(&s, "d", 0);
    assert_eq!((&after[0..8], &after[8..10]), (expected_8.as_slice(), [None, None].as_slice()));
    assert_eq!(
        (ds(&s, "d").linked_regions.len(), ds(&s, "d").linked_regions[0].target_range),
        (1, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(7, 0, 0)))
    );

    fs::remove_file(&plate_path)?;
    assert!(s.execute("project.recalculate", json!({})).is_err());
    assert_eq!(&col_opt_nums(&s, "d", 0)[0..8], expected_8.as_slice());

    fs::write(&append_path, "100\n200\n300\n")?;
    let mut s_app = Session::new();
    import_csv(&mut s_app, "app", "1\n2");
    ok_cmd(
        &mut s_app,
        "table.importCsv",
        json!({ "sheet_id": "app", "path": append_path.to_string_lossy(), "has_header": false, "mode": "append", "link": true }),
    );
    assert_eq!(col_opt_nums(&s_app, "app", 0), vec![Some(1.0), Some(2.0), Some(100.0), Some(200.0), Some(300.0)]);

    fs::write(&append_path, "105\n205\n305\n405\n")?;
    ok_cmd(&mut s_app, "project.recalculate", json!({}));
    assert_eq!(col_opt_nums(&s_app, "app", 0), vec![Some(1.0), Some(2.0), Some(105.0), Some(205.0), Some(305.0), Some(405.0)]);

    fs::write(&append_path, "110\n220\n")?;
    ok_cmd(&mut s_app, "project.recalculate", json!({}));
    assert_eq!(col_opt_nums(&s_app, "app", 0), vec![Some(1.0), Some(2.0), Some(110.0), Some(220.0), None, None]);
    assert_eq!(
        (ds(&s_app, "app").linked_regions.len(), ds(&s_app, "app").linked_regions[0].target_range),
        (1, SheetCellRange::new(CellCoord::new(2, 0, 0), CellCoord::new(3, 0, 0)))
    );
    Ok(())
}

#[test]
fn r1_s07_08a_calculated_variables_never_fall_back_to_unrelated_info_sheet() -> TestResult {
    let mut s = Session::new();
    run_cmds(
        &mut s,
        &[
            ("sheet.createData", json!({ "sheet_id": "plate2", "info_sheet_id": "plate2-info", "auto_info": true })),
            ("info.setConstant", json!({ "sheet_id": "plate2-info", "name": "K", "value": 10.0 })),
            (
                "sheet.createData",
                json!({ "sheet_id": "mv", "archetype": "multiple_variables", "cols": 2, "rows": 3, "info_sheet_id": "mv-info", "auto_info": true }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "mv", "col": 0, "title": "Raw" })),
            ("info.setConstant", json!({ "sheet_id": "mv-info", "name": "K", "value": 3.0 })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv", "title": "C", "formula": "=[Raw] * Info(\"K\")" })),
            ("table.setCell", json!({ "sheet_id": "mv", "row": 0, "col": 0, "value": 1.0 })),
        ],
    );
    assert_eq!(col_opt_nums(&s, "mv", 2)[0], Some(3.0));

    assert_err_code(&mut s, "sheet.delete", json!({ "sheet_id": "mv-info", "cascade": false }), ErrorCode::Conflict);
    let err_del = s.execute("sheet.delete", json!({ "sheet_id": "mv-info" })).unwrap_err();
    assert!(
        err_del.message().contains("failed")
            && s.document().info_sheet(&SheetId::new("mv-info")).is_some()
            && col_opt_nums(&s, "mv", 2)[0] == Some(3.0)
    );

    let mut broken_doc = s.document().clone();
    broken_doc.data_sheet_mut(&SheetId::new("mv")).unwrap().info_sheet_id = None;
    assert!(dag::recalculate_project(&mut broken_doc).unwrap_err().message().contains("failed"));

    let mut s_plain = Session::new();
    run_cmds(
        &mut s_plain,
        &[
            ("sheet.createData", json!({ "sheet_id": "other", "info_sheet_id": "other-info", "auto_info": true })),
            ("info.setConstant", json!({ "sheet_id": "other-info", "name": "K", "value": 99.0 })),
            (
                "sheet.createData",
                json!({ "sheet_id": "mv-no-info", "archetype": "multiple_variables", "cols": 1, "rows": 2, "info_sheet_id": "mv-plain-info", "auto_info": true }),
            ),
            ("table.setGroupTitles", json!({ "sheet_id": "mv-no-info", "col": 0, "title": "Raw" })),
            ("table.setCalculatedVariable", json!({ "sheet_id": "mv-no-info", "title": "Dbl", "formula": "=[Raw] * 2" })),
            ("table.setCell", json!({ "sheet_id": "mv-no-info", "row": 0, "col": 0, "value": 4.0 })),
            ("sheet.delete", json!({ "sheet_id": "mv-plain-info" })),
        ],
    );
    assert!(
        s_plain.document().info_sheet(&SheetId::new("mv-plain-info")).is_none()
            && ds(&s_plain, "mv-no-info").info_sheet_id.is_none()
            && col_opt_nums(&s_plain, "mv-no-info", 1)[0] == Some(8.0)
    );
    Ok(())
}

#[test]
fn r1_s07_08b_fit_and_transform_hooks_strictly_require_bound_info_sheet() -> TestResult {
    let mut doc = ProjectDocument::new("Hook Binding Regression")?;
    for (id, val) in [("plate2-info", 1000.0), ("curve-info", 50.0)] {
        let mut info = InfoSheet::new_preseeded(id, id)?;
        info.set_constant("Scale", ConstantValue::Number(val), None)?;
        info.set_constant("Top", ConstantValue::Number(val), None)?;
        doc.add_info_sheet(info)?;
    }
    let (doses, resp) = ([-9.0, -8.0, -7.0, -6.0, -5.0, -4.0], [2.0, 5.0, 15.0, 35.0, 46.0, 49.0]);
    let mut d1 = xy_sheet("xy-1", "Dose Response", "X", &doses, &[("Y", &resp)]);
    d1.info_sheet_id = Some(SheetId::new("curve-info"));
    doc.add_data_sheet(d1)?;
    doc.add_data_sheet(xy_sheet("xy-2", "Dose Response 2", "X", &doses, &[("Y", &resp)]))?;

    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "res-xform",
        "xy-1",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Scale") }),
            output_percent: true,
            per_data_set: false,
        }),
    )?)?)?;

    let mk_nl = |info_id: Option<&str>| {
        let mut nl = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
        nl.method = FitMethod::LeastSquares;
        nl.weighting = WeightingScheme::None;
        nl.parameter_rules.push(ParameterRule {
            name: String::from("Top"),
            scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
                info_sheet_id: info_id.map(SheetId::new),
                constant_name: String::from("Top"),
            }),
            initial_override: None,
            lower_bound: None,
            upper_bound: None,
        });
        nl
    };
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-fit", "xy-1", AnalysisKind::NonlinearRegression(mk_nl(None)))?)?)?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new("res-chain", "res-xform", AnalysisKind::NonlinearRegression(mk_nl(None)))?)?)?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "res-explicit",
        "xy-2",
        AnalysisKind::NonlinearRegression(mk_nl(Some("curve-info"))),
    )?)?)?;

    dag::recalculate_project(&mut doc)?;
    let rids = ["res-xform", "res-fit", "res-chain", "res-explicit"];
    let mut s = Session::from_document(doc);
    for rid in rids {
        assert_eq!(rs(&s, rid).status, ResultStatus::UpToDate);
    }

    assert_err_code(&mut s, "sheet.delete", json!({ "sheet_id": "curve-info", "cascade": false }), ErrorCode::Conflict);
    ok_cmd(&mut s, "sheet.delete", json!({ "sheet_id": "curve-info" }));
    assert!(s.document().info_sheet(&SheetId::new("curve-info")).is_none() && ds(&s, "xy-1").info_sheet_id.is_none());
    for rid in rids {
        assert!(matches!(rs(&s, rid).status, ResultStatus::Failed { .. }) && rs(&s, rid).fitted_curves.is_empty());
    }
    ok_cmd(&mut s, "table.setCell", json!({ "sheet_id": "xy-1", "row": 0, "col": 0, "value": 2.5 }));
    for rid in rids {
        assert!(matches!(rs(&s, rid).status, ResultStatus::Failed { .. }));
    }
    Ok(())
}
