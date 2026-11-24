//! Regression tests for Wave 3 engine analysis & validation audit findings
//! (#4, #7, #32, #58, #78, #79, #80, #93, #113, #114, #120).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, BlandAltmanSpec, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorCode, FitMethod, GroupedTable, MultipleVariablesTable,
    ProjectDocument, ResultStatus, SheetId, SubcolumnMode, SurvivalAnalysisSpec, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use serde_json::json;

use super::common::{err_cmd, ok_cmd};

fn scalar_val(session: &Session, sheet_id: &str, key: &str) -> Option<f64> {
    let rs = session.document().result_sheet(&SheetId::new(sheet_id))?;
    rs.scalars.iter().find(|s| s.key == key).map(|s| s.value)
}

fn make_single_xy_sheet(id: &str) -> DataSheet {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 5).unwrap();
    xy.x_title = String::from("X");
    if let Some(ds0) = xy.data_sets.first_mut() {
        ds0.title = String::from("Y0");
        for (r, (x_row, ds_row)) in xy.x_cells.iter_mut().zip(ds0.cells.iter_mut()).enumerate().take(5) {
            let x = (r + 1) as f64;
            if let Some(x_cell) = x_row.first_mut() {
                *x_cell = Cell::number(x);
            }
            if let Some(y_cell) = ds_row.first_mut() {
                *y_cell = Cell::number(x + 3.0);
            }
        }
    }
    DataSheet::new(id, "Single XY", TableArchetype::Xy(xy)).unwrap()
}

fn make_two_xy_sheet(id: &str) -> DataSheet {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 5).unwrap();
    xy.x_title = String::from("X");
    if let [ds0, ds1] = xy.data_sets.as_mut_slice() {
        ds0.title = String::from("Y0");
        ds1.title = String::from("Y1");
        for ((r, x_row), (row0, row1)) in xy.x_cells.iter_mut().enumerate().zip(ds0.cells.iter_mut().zip(ds1.cells.iter_mut())).take(5) {
            let x = (r + 1) as f64;
            if let Some(x_cell) = x_row.first_mut() {
                *x_cell = Cell::number(x);
            }
            if let Some(c0) = row0.first_mut() {
                *c0 = Cell::number(x + 3.0);
            }
            if let Some(c1) = row1.first_mut() {
                *c1 = Cell::number(200.0 + x);
            }
        }
    }
    DataSheet::new(id, "Two XY", TableArchetype::Xy(xy)).unwrap()
}

fn make_col_sheet(id: &str, cols: usize, rows: usize) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, cols, rows).unwrap();
    for (c, ds) in t.data_sets.iter_mut().enumerate().take(cols) {
        ds.title = format!("G{c}");
        for (r, row) in ds.cells.iter_mut().enumerate().take(rows) {
            if let Some(cell) = row.first_mut() {
                *cell = Cell::number((c * 10 + r + 1) as f64 + 0.1 * (r as f64));
            }
        }
    }
    DataSheet::new(id, "Col Sheet", TableArchetype::Column(t)).unwrap()
}

#[test]
fn audit_4_bland_altman_xy_mode_persists_across_column_additions_updates_and_retarget() {
    let mut doc = ProjectDocument::new("Audit #4 Bland-Altman XY").unwrap();
    doc.add_data_sheet(make_single_xy_sheet("xy1")).unwrap();
    doc.add_data_sheet(make_two_xy_sheet("xy2")).unwrap();
    doc.add_data_sheet(make_col_sheet("col2", 2, 5)).unwrap();
    let mut session = Session::from_document(doc);

    // 1-dataset XY defaults explicitly to xy_compare_x_vs_y = Some(true).
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "xy1", "kind": "bland_altman" }));
    let bias_before = scalar_val(&session, "ba-xy1", "Bias").unwrap();
    assert!((bias_before.abs() - 3.0).abs() < 1e-9, "expected X vs Y0 bias magnitude 3.0, got {bias_before}");
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy1")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), column_a: 0, column_b: 1, .. })
    ));

    // Pasting a 2nd Y data set onto xy1 must NOT retarget the existing X-vs-Y0 Bland-Altman sheet.
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "xy1", "start_row": 0, "start_col": 1, "text": "200\n201\n202\n203\n204" }));
    let bias_after_paste = scalar_val(&session, "ba-xy1", "Bias").unwrap();
    assert!((bias_after_paste - bias_before).abs() < 1e-9, "pasting Y1 retargeted existing Bland-Altman bias: {bias_after_paste}");

    // Unrelated updateParams preserves xy_compare_x_vs_y = Some(true).
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "ba-xy1", "confidence_percent": 99.0 }));
    let bias_after_update = scalar_val(&session, "ba-xy1", "Bias").unwrap();
    assert!((bias_after_update - bias_before).abs() < 1e-9);
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy1")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), .. })
    ));

    // Fresh Bland-Altman on 2-dataset XY defaults to Some(false) (Y0 vs Y1), unless xy_compare_x_vs_y: true.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "ba-xy2", "source_sheet_id": "xy2", "kind": "bland_altman" }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy2")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(false), .. })
    ));
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "ba-xy2-x", "source_sheet_id": "xy2", "kind": "bland_altman", "xy_compare_x_vs_y": true }),
    );
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy2-x")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(true), .. })
    ));
    let bias_xy2_x = scalar_val(&session, "ba-xy2-x", "Bias").unwrap();
    assert!((bias_xy2_x.abs() - 3.0).abs() < 1e-9);

    // Invalid X-vs-Y column index is rejected cleanly.
    let bad_x_vs_y =
        err_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy2", "kind": "bland_altman", "xy_compare_x_vs_y": true, "column_b": 2 }));
    assert_eq!(bad_x_vs_y.code, ErrorCode::BadInput);

    // Retargeting between XY tables recomputes xy_compare_x_vs_y, and retargeting to Column clears it to None.
    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "xy2" }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy1")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: Some(false), .. })
    ));
    ok_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "ba-xy1", "source_sheet_id": "col2" }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("ba-xy1")).unwrap().analysis.kind,
        AnalysisKind::BlandAltman(BlandAltmanSpec { xy_compare_x_vs_y: None, .. })
    ));
}

#[test]
fn audit_7_paired_multiple_row_tests_return_bad_input() {
    let mut doc = ProjectDocument::new("Audit #7 Paired Row Tests").unwrap();

    // 3-dataset Grouped with 3 replicates per dataset (non-constant diffs for paired t-test).
    let mut grp3 = GroupedTable::new(SubcolumnMode::RawReplicates(3), 3, 4).unwrap();
    let offsets = [0.1, 0.7, -0.3];
    for c in 0..3 {
        grp3.data_sets[c].title = format!("Arm{c}");
        for r in 0..4 {
            grp3.row_titles[r] = format!("R{r}");
            for s in 0..3 {
                grp3.data_sets[c].cells[r][s] = Cell::number((c * 6 + r + 1) as f64 + offsets[s] * (c as f64 + 1.0));
            }
        }
    }
    doc.add_data_sheet(DataSheet::new("grp3", "Grp3", TableArchetype::Grouped(grp3)).unwrap()).unwrap();

    // 3-dataset XY with 2 replicates per dataset.
    let mut xy3 = XyTable::new(SubcolumnMode::RawReplicates(2), 3, 4).unwrap();
    for r in 0..4 {
        xy3.x_cells[r][0] = Cell::number((r + 1) as f64);
        for c in 0..3 {
            xy3.data_sets[c].cells[r][0] = Cell::number((c * 5 + r + 1) as f64);
            xy3.data_sets[c].cells[r][1] = Cell::number((c * 5 + r + 2) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("xy3", "Xy3", TableArchetype::Xy(xy3)).unwrap()).unwrap();

    // 3-column and 4-column Column tables + 3-numeric MV table.
    doc.add_data_sheet(make_col_sheet("col3", 3, 4)).unwrap();
    doc.add_data_sheet(make_col_sheet("col4", 4, 4)).unwrap();
    let mut mv3 = MultipleVariablesTable::new(3, 4).unwrap();
    for c in 0..3 {
        for r in 0..4 {
            mv3.variables[c].cells[r] = Cell::number((c * 4 + r + 1) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("mv3", "Mv3", TableArchetype::MultipleVariables(mv3)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    for (src, extra) in [
        ("grp3", json!({ "paired": true })),
        ("grp3", json!({ "paired": true, "selected_data_sets": [0, 1, 2] })),
        ("grp3", json!({ "paired": false, "selected_data_sets": [0, 1, 2] })),
        ("xy3", json!({ "paired": true })),
        ("col3", json!({ "paired": true })),
        ("mv3", json!({ "paired": true })),
    ] {
        let mut req = json!({ "source_sheet_id": src, "kind": "multiple_row_t_tests" });
        req.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let err = err_cmd(&mut session, "stats.run", req);
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for {src} with {extra}, got {err}");
    }

    // Valid 2-dataset subset on grp3 succeeds.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "mr-ok", "source_sheet_id": "grp3", "kind": "multiple_row_t_tests", "paired": true, "selected_data_sets": [0, 1] }),
    );
    assert_eq!(session.document().result_sheet(&SheetId::new("mr-ok")).unwrap().status, ResultStatus::UpToDate);

    // Delayed recalculation after table expansion on a Column sheet maps `paired_selected_columns` to BadInput.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "mr-col", "source_sheet_id": "col4", "kind": "multiple_row_t_tests", "paired": true }));
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col4", "start_row": 0, "start_col": 4, "text": "9\n10\n11\n12" }));
    let recalc_err = err_cmd(&mut session, "analysis.run", json!({ "sheet_id": "mr-col" }));
    assert_eq!(recalc_err.code, ErrorCode::BadInput);
}

#[test]
fn audit_32_selected_data_sets_rejects_out_of_range_and_duplicates() {
    let mut doc = ProjectDocument::new("Audit #32 selected_data_sets").unwrap();
    doc.add_data_sheet(make_col_sheet("col2", 2, 5)).unwrap();
    doc.add_data_sheet(make_two_xy_sheet("xy2")).unwrap();

    // MV sheet with Categorical Group (col 0), X (col 1), Y (col 2), 2 group levels.
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    mv.variables[0].title = String::from("Arm");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].role_inferred = false;
    mv.variables[1].title = String::from("LogConc");
    mv.variables[2].title = String::from("Resp");
    for (r, x) in [-8.0, -7.0, -6.0, -8.0, -7.0, -6.0].into_iter().enumerate() {
        let arm = if r < 3 { "Vehicle" } else { "Drug" };
        let base = if r < 3 { 10.0 } else { 20.0 };
        mv.variables[0].cells[r] = Cell::text(arm);
        mv.variables[1].cells[r] = Cell::number(x);
        mv.variables[2].cells[r] = Cell::number(base + (r % 3) as f64 * 25.0);
    }
    doc.add_data_sheet(DataSheet::new("mv-nl", "MV Nonlinear", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // Reject duplicates & out-of-range across creation commands.
    for (cmd, src, kind) in [
        ("stats.run", "col2", "descriptive"),
        ("analysis.run", "col2", "one_way_anova"),
        ("fit.run", "xy2", "linear"),
        ("transform.run", "xy2", "normalize"),
    ] {
        let dup_err = err_cmd(&mut session, cmd, json!({ "source_sheet_id": src, "kind": kind, "selected_data_sets": [0, 0] }));
        assert_eq!(dup_err.code, ErrorCode::BadInput, "expected BadInput for duplicate on {cmd}");
        let oob_err = err_cmd(&mut session, cmd, json!({ "source_sheet_id": src, "kind": kind, "selected_data_sets": [0, 5] }));
        assert_eq!(oob_err.code, ErrorCode::BadInput, "expected BadInput for OOB on {cmd}");
    }

    // Also reject via updateParams & retargetData.
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "desc-1", "source_sheet_id": "col2", "kind": "descriptive" }));
    assert_eq!(
        err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "desc-1", "selected_data_sets": [1, 1] })).code,
        ErrorCode::BadInput
    );
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "desc-1", "selected_data_sets": [99] })).code, ErrorCode::BadInput);
    assert_eq!(
        err_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy2", "selected_data_sets": [0, 0] })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "analysis.retargetData", json!({ "sheet_id": "desc-1", "source_sheet_id": "xy2", "selected_data_sets": [4] })).code,
        ErrorCode::BadInput
    );

    // Explicit Nonlinear MV group selection allows valid group index 0/1 and rejects OOB group index 2.
    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "nl-grp-ok",
            "source_sheet_id": "mv-nl",
            "kind": "straight_line",
            "mv_group_columns": [0],
            "mv_x_columns": [1],
            "mv_y_column": 2,
            "selected_data_sets": [1]
        }),
    );
    let nl_grp_oob = err_cmd(
        &mut session,
        "fit.run",
        json!({
            "source_sheet_id": "mv-nl",
            "kind": "straight_line",
            "mv_group_columns": [0],
            "mv_x_columns": [1],
            "mv_y_column": 2,
            "selected_data_sets": [2]
        }),
    );
    assert_eq!(nl_grp_oob.code, ErrorCode::BadInput);
}

#[test]
fn audit_58_transform_rejects_unknown_operation_and_anchors() {
    let mut doc = ProjectDocument::new("Audit #58 Transform Enums").unwrap();
    doc.add_data_sheet(make_two_xy_sheet("xy2")).unwrap();
    let mut session = Session::from_document(doc);

    assert_eq!(
        err_cmd(&mut session, "transform.run", json!({ "source_sheet_id": "xy2", "kind": "normalize", "hundred_anchor": "maximum" })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "transform.run", json!({ "source_sheet_id": "xy2", "kind": "normalize", "zero_anchor": "bogus" })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "transform.run", json!({ "source_sheet_id": "xy2", "kind": "remove_baseline", "operation": "multiply" })).code,
        ErrorCode::BadInput
    );

    // Valid documented aliases stay accepted.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "norm-ok", "source_sheet_id": "xy2", "kind": "normalize", "zero_anchor": "smallest", "hundred_anchor": "largest" }),
    );
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "base-ok", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "pct_change" }),
    );
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "sheet_id": "base-ok2", "source_sheet_id": "xy2", "kind": "remove_baseline", "baseline_columns": [0], "operation": "pctchange" }),
    );

    // updateParams also rejects typos.
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "norm-ok", "zero_anchor": "minimum" })).code, ErrorCode::BadInput);
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "base-ok", "operation": "add" })).code, ErrorCode::BadInput);
}

#[test]
fn audit_78_confidence_level_fraction_rejected_across_all_commands() {
    let mut doc = ProjectDocument::new("Audit #78 Confidence").unwrap();
    doc.add_data_sheet(make_col_sheet("col2", 2, 5)).unwrap();
    doc.add_data_sheet(make_single_xy_sheet("xy1")).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "res-base", "source_sheet_id": "col2", "kind": "unpaired_t" }));

    for (cmd, payload) in [
        ("analysis.run", json!({ "source_sheet_id": "col2", "kind": "unpaired_t", "confidence_percent": 0.95 })),
        ("analysis.run", json!({ "source_sheet_id": "col2", "kind": "unpaired_t", "confidence_percent": 1.0 })),
        ("stats.run", json!({ "source_sheet_id": "col2", "kind": "descriptive", "confidence_percent": 0.99 })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "confidence_percent": 0.95 })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "confidence_percent": 0.95 })),
        ("analysis.updateParams", json!({ "sheet_id": "res-base", "confidence_percent": 0.95 })),
        ("analysis.updateParams", json!({ "sheet_id": "res-base", "params": { "confidence_percent": 0.95 } })),
    ] {
        let err = err_cmd(&mut session, cmd, payload.clone());
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for {cmd} {payload}");
        let combined = format!("{} {}", err.message(), err.remediation.as_deref().unwrap_or_default());
        assert!(combined.contains("95") && combined.contains("percentage"), "expected percentage remediation in: {combined}");
    }

    // Raw spec JSON containing 0.95 confidence_level also rejected.
    let spec_err = err_cmd(
        &mut session,
        "analysis.run",
        json!({
            "spec": {
                "id": "bad-conf-spec",
                "source_sheet_id": "col2",
                "selected_data_sets": [],
                "auto_title": "Unpaired t test",
                "custom_title": null,
                "confidence_level": 0.95,
                "p_value_format": "FourDecimal",
                "kind": { "DescriptiveStats": { "aggregation": "RowMeans", "custom_upper_percentile": null, "run_normality_battery": true, "hypothetical_value": null } }
            }
        }),
    );
    assert_eq!(spec_err.code, ErrorCode::BadInput);
}

#[test]
fn audit_79_rejects_unknown_and_inapplicable_keys_per_kind() {
    let mut doc = ProjectDocument::new("Audit #79 Strict Keys").unwrap();
    doc.add_data_sheet(make_col_sheet("col3", 3, 5)).unwrap();
    doc.add_data_sheet(make_single_xy_sheet("xy1")).unwrap();
    let mut session = Session::from_document(doc);

    let invalid_cases = [
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "alpha": 0.01 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "hypothetical": 5.0 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "unpaired_t", "bogus_param": true })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "descriptive", "hypothetical_mean": 1.0 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "geisser_greenhouse": true })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity": "tukey", "control_column": 0 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "grubbs", "q_percent": 1.0 })),
        ("stats.run", json!({ "source_sheet_id": "col3", "kind": "rout", "alpha": 0.05 })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "equation_id": "4pl" })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "alpha": 0.05 })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "q_percent": 2.0 })),
        ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "alternative_equation_id": "log_agonist_vs_response_3pl" })),
        ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "every_k_rows": 2 })),
        (
            "analysis.run",
            json!({
                "source_sheet_id": "col3",
                "kind": { "DescriptiveStats": { "aggregation": "RowMeans", "custom_upper_percentile": null, "run_normality_battery": true, "hypothetical_value": null, "bogus": 1 } }
            }),
        ),
        ("analysis.run", json!({ "sheet_id": "nonexistent", "bogus": 1 })),
    ];
    for (cmd, payload) in invalid_cases {
        let err = err_cmd(&mut session, cmd, payload.clone());
        assert_eq!(err.code, ErrorCode::BadInput, "expected BadInput for {cmd} {payload}, got {err}");
    }

    // updateParams rejects unknown keys both top-level and inside `params`.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "rm-ok", "source_sheet_id": "col3", "kind": "rm_one_way_anova", "multiplicity": "tukey", "geisser_greenhouse": false }),
    );
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rm-ok", "bogus": 1 })).code, ErrorCode::BadInput);
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rm-ok", "params": { "bogus": 1 } })).code, ErrorCode::BadInput);
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rm-ok", "control_column": 0 })).code, ErrorCode::BadInput);

    // Valid documented Dunnett + control_column works cleanly.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "dun-ok", "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity": "dunnett", "control_column": 0 }),
    );

    // LinearRegression rejects `parameter_rules` on creation and nested `params`, while allowing
    // top-level validated `parameter_rules` on `analysis.updateParams` for ConstantHookPicker compat.
    let bad_hook_rule = json!([{
        "name": "ConstraintA",
        "scope": { "HookedConstant": { "InfoConstant": { "info_sheet_id": "info-1", "constant_name": "DoseRef" } } }
    }]);
    assert_eq!(
        err_cmd(&mut session, "fit.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "parameter_rules": bad_hook_rule })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "analysis.run", json!({ "source_sheet_id": "xy1", "kind": "linear", "parameter_rules": bad_hook_rule })).code,
        ErrorCode::BadInput
    );
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "lin-hook", "source_sheet_id": "xy1", "kind": "linear" }));
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "lin-hook", "parameter_rules": bad_hook_rule }));
    assert_eq!(
        err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "lin-hook", "parameter_rules": "invalid" })).code,
        ErrorCode::BadInput
    );
    assert_eq!(
        err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "lin-hook", "params": { "parameter_rules": bad_hook_rule } })).code,
        ErrorCode::BadInput
    );
}

#[test]
fn audit_80_nonlinear_rout_q_override_and_bounds() {
    let mut doc = ProjectDocument::new("Audit #80 ROUT Q").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 8).unwrap();
    for (r, (&x, &y)) in
        [-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0].iter().zip([10.0, 12.0, 22.0, 52.0, 84.0, 96.0, 99.0, 100.0].iter()).enumerate()
    {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("xy-dr", "Dose", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // q_percent & fdr_q_percent override works with method: "rout".
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "rout-5", "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": 5.0 }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("rout-5")).unwrap().analysis.kind,
        AnalysisKind::NonlinearRegression(ref nl)
            if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 5.0).abs() < 1e-12)
    ));

    // Boundary Q = 50.0 allowed; <= 0.0 or > 50.0 rejected with BadInput.
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "rout-50", "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": 50.0 }));
    for bad_q in [0.0, -1.0, 50.1] {
        assert_eq!(
            err_cmd(&mut session, "fit.run", json!({ "source_sheet_id": "xy-dr", "kind": "4pl", "method": "rout", "q_percent": bad_q })).code,
            ErrorCode::BadInput,
            "expected BadInput for q_percent={bad_q}"
        );
        assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rout-5", "q_percent": bad_q })).code, ErrorCode::BadInput);
    }

    // Valid updateParams updates fdr_q_percent on existing ROUT fit.
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "rout-5", "q_percent": 2.5 }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("rout-5")).unwrap().analysis.kind,
        AnalysisKind::NonlinearRegression(ref nl)
            if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 2.5).abs() < 1e-12)
    ));
}

#[test]
fn audit_93_mv_default_columns_persisted_before_appending_calculated_var() {
    let mut doc = ProjectDocument::new("Audit #93 MV Persistence").unwrap();

    // MV table 1: Categorical Group (0), Dose (1), Response (2).
    let mut mv = MultipleVariablesTable::new(3, 10).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].role_inferred = false;
    mv.variables[1].title = String::from("Dose");
    mv.variables[2].title = String::from("Resp");
    let doses = [-8.0, -7.0, -6.0, -5.0, -4.0, -8.0, -7.0, -6.0, -5.0, -4.0];
    for (r, &d) in doses.iter().enumerate() {
        let g = if r < 5 { "A" } else { "B" };
        let y = if r < 5 { 10.0 + 90.0 / (1.0 + 10.0_f64.powf(-6.0 - d)) } else { 15.0 + 80.0 / (1.0 + 10.0_f64.powf(-5.5 - d)) };
        mv.variables[0].cells[r] = Cell::text(g);
        mv.variables[1].cells[r] = Cell::number(d);
        mv.variables[2].cells[r] = Cell::number(y);
    }
    doc.add_data_sheet(DataSheet::new("mv-bio", "MV Bio", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    // MV table 2: All continuous columns (no label/categorical) for PCA/CorrelationMatrix & Linear.
    let mut mv_cont = MultipleVariablesTable::new(3, 6).unwrap();
    mv_cont.variables[0].title = String::from("A");
    mv_cont.variables[1].title = String::from("B");
    mv_cont.variables[2].title = String::from("C");
    for r in 0..6 {
        let x = (r + 1) as f64;
        mv_cont.variables[0].cells[r] = Cell::number(x);
        mv_cont.variables[1].cells[r] = Cell::number(2.0 * x + 1.0);
        mv_cont.variables[2].cells[r] = Cell::number(0.5 * x + (r % 2) as f64);
    }
    doc.add_data_sheet(DataSheet::new("mv-cont", "MV Cont", TableArchetype::MultipleVariables(mv_cont)).unwrap()).unwrap();

    // MV table 3: Kaplan-Meier Survival (Arm col 0, Time col 1, Status col 2).
    let mut mv_km = MultipleVariablesTable::new(3, 8).unwrap();
    mv_km.variables[0].title = String::from("Arm");
    mv_km.variables[0].role = VariableRole::Categorical;
    mv_km.variables[0].role_inferred = false;
    mv_km.variables[1].title = String::from("FollowupTime");
    mv_km.variables[2].title = String::from("EventStatus");
    for r in 0..8 {
        mv_km.variables[0].cells[r] = Cell::text(if r < 4 { "Placebo" } else { "Active" });
        mv_km.variables[1].cells[r] = Cell::number((r * 2 + 5) as f64);
        mv_km.variables[2].cells[r] = Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 });
    }
    doc.add_data_sheet(DataSheet::new("mv-km", "MV KM", TableArchetype::MultipleVariables(mv_km)).unwrap()).unwrap();

    // MV table 4: 4 all-continuous columns (unchanged by Calculated var) for PCA/PCR UI compat check.
    let mut mv_all_cont = MultipleVariablesTable::new(4, 6).unwrap();
    for c in 0..4 {
        mv_all_cont.variables[c].title = format!("V{c}");
        for r in 0..6 {
            mv_all_cont.variables[c].cells[r] = Cell::number((r + 1) as f64 * (c + 1) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("mv-all-cont", "MV All Cont", TableArchetype::MultipleVariables(mv_all_cont)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "tg-mv", "source_sheet_id": "mv-bio", "kind": "unpaired_t" }));
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "nl-mv", "source_sheet_id": "mv-bio", "kind": "4pl" }));
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "lin-mv", "source_sheet_id": "mv-cont", "kind": "linear" }));
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "pca-mv", "source_sheet_id": "mv-cont", "kind": "pca" }));
    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "km-mv", "source_sheet_id": "mv-km", "kind": "kaplan_meier" }));

    // Assert default bindings were materialized into AnalysisSpec right at creation.
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("tg-mv")).unwrap().analysis.kind,
        AnalysisKind::TwoGroupComparison(ref tg) if tg.mv_group_column == Some(0) && tg.mv_response_column == Some(2)
    ));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("nl-mv")).unwrap().analysis.kind,
        AnalysisKind::NonlinearRegression(ref nl)
            if nl.mv_group_columns == vec![0] && nl.mv_x_columns == vec![1] && nl.mv_y_column == Some(2)
    ));
    assert_eq!(session.document().result_sheet(&SheetId::new("lin-mv")).unwrap().analysis.selected_data_sets, vec![0, 2]);
    assert_eq!(session.document().result_sheet(&SheetId::new("pca-mv")).unwrap().analysis.selected_data_sets, vec![0, 1, 2]);
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("km-mv")).unwrap().analysis.kind,
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            mv_time_column: Some(1),
            mv_status_column: Some(2),
            mv_group_column: Some(0),
            ..
        })
    ));

    let tg_before = session.document().result_sheet(&SheetId::new("tg-mv")).unwrap().scalars.clone();
    assert!(!tg_before.is_empty());
    let nl_before = session.document().result_sheet(&SheetId::new("nl-mv")).unwrap().scalars.clone();
    let lin_before = scalar_val(&session, "lin-mv", "Slope").unwrap();
    let pca_vars_before = session.document().result_sheet(&SheetId::new("pca-mv")).unwrap().tabs[0].row_labels.len();
    let km_before = session.document().result_sheet(&SheetId::new("km-mv")).unwrap().scalars.clone();

    // Appending a Calculated column with extreme values must NOT retarget existing MV analyses.
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-bio", "title": "Spike", "formula": "= [Resp] + 10000" }));
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-cont", "title": "Spike", "formula": "= [C] + 10000" }));
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-km", "title": "StatusAlt", "formula": "= 1 - [EventStatus]" }));

    let tg_after = &session.document().result_sheet(&SheetId::new("tg-mv")).unwrap().scalars;
    let nl_after = &session.document().result_sheet(&SheetId::new("nl-mv")).unwrap().scalars;
    let lin_after = scalar_val(&session, "lin-mv", "Slope").unwrap();
    let pca_vars_after = session.document().result_sheet(&SheetId::new("pca-mv")).unwrap().tabs[0].row_labels.len();
    let km_after = &session.document().result_sheet(&SheetId::new("km-mv")).unwrap().scalars;

    assert_eq!(tg_after, &tg_before, "two-group retargeted after calculated var");
    assert_eq!(nl_after, &nl_before, "nonlinear MV retargeted after calculated var");
    assert!((lin_after - lin_before).abs() < 1e-12, "linear MV retargeted after calculated var: before={lin_before}, after={lin_after}");
    assert_eq!(pca_vars_after, pca_vars_before, "PCA included new calculated var unexpectedly");
    assert_eq!(km_after, &km_before, "Kaplan-Meier MV retargeted after calculated var");

    // Explicit `"selected_data_sets": []` on all-continuous MV PCA preserves `[]` (UI EditParams WF-23 compat),
    // whereas mixed-role MV tables still persist continuous defaults.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "pca-explicit-empty",
            "source_sheet_id": "mv-all-cont",
            "selected_data_sets": [],
            "kind": { "Multivariate": { "Pca": { "standardize": true, "parallel_analysis_seed": 7 } } }
        }),
    );
    assert!(session.document().result_sheet(&SheetId::new("pca-explicit-empty")).unwrap().analysis.selected_data_sets.is_empty());
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "pca-mixed-explicit-empty",
            "source_sheet_id": "mv-bio",
            "selected_data_sets": [],
            "kind": { "Multivariate": { "Pca": { "standardize": true, "parallel_analysis_seed": 7 } } }
        }),
    );
    assert_eq!(session.document().result_sheet(&SheetId::new("pca-mixed-explicit-empty")).unwrap().analysis.selected_data_sets, vec![1, 2]);
}

#[test]
fn audit_113_multiple_imputation_m_and_iters_bounds() {
    let mut doc = ProjectDocument::new("Audit #113 MI Bounds").unwrap();
    let mut mv = MultipleVariablesTable::new(3, 8).unwrap();
    for r in 0..8 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = if r == 2 { Cell::empty() } else { Cell::number((r * 2 + 1) as f64) };
        mv.variables[2].cells[r] = Cell::number((r * 3 + 2) as f64);
    }
    doc.add_data_sheet(DataSheet::new("mv-mi", "MV MI", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    for bad_m in [0_usize, 1, 51] {
        assert_eq!(
            err_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "mv-mi", "kind": "mice", "imputations_m": bad_m })).code,
            ErrorCode::BadInput,
            "expected BadInput for m={bad_m}"
        );
    }
    for bad_iters in [0_usize, 101] {
        assert_eq!(
            err_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "mv-mi", "kind": "mice", "max_iters": bad_iters })).code,
            ErrorCode::BadInput,
            "expected BadInput for max_iters={bad_iters}"
        );
    }

    ok_cmd(&mut session, "stats.run", json!({ "sheet_id": "mi-ok", "source_sheet_id": "mv-mi", "kind": "mice", "imputations_m": 2, "max_iters": 1 }));
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "mi-ok", "imputations_m": 1 })).code, ErrorCode::BadInput);
    assert_eq!(err_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "mi-ok", "max_iters": 101 })).code, ErrorCode::BadInput);
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "mi-ok", "imputations_m": 50, "max_iters": 100 }));
}

#[test]
fn audit_114_nonlinear_parameter_rules_unknown_and_duplicate_rejected() {
    let mut doc = ProjectDocument::new("Audit #114 Parameter Rules").unwrap();
    doc.add_data_sheet(make_single_xy_sheet("xy1")).unwrap();
    let mut session = Session::from_document(doc);

    // Unknown parameter rule name rejected with BadInput.
    let unk = err_cmd(
        &mut session,
        "fit.run",
        json!({
            "source_sheet_id": "xy1",
            "kind": "4pl",
            "parameter_rules": [{ "name": "BogusParam", "scope": "shared" }]
        }),
    );
    assert_eq!(unk.code, ErrorCode::BadInput);

    // Duplicate parameter rules (case-insensitive) rejected with BadInput.
    let dup = err_cmd(
        &mut session,
        "fit.run",
        json!({
            "source_sheet_id": "xy1",
            "kind": "straight_line",
            "parameter_rules": [
                { "name": "Slope", "scope": "constant", "value": 1.0 },
                { "name": "slope", "scope": "local" }
            ]
        }),
    );
    assert_eq!(dup.code, ErrorCode::BadInput);

    // Valid rule on custom DSL passes, unknown fails; updateParams also validates.
    let dsl_unk = err_cmd(
        &mut session,
        "fit.run",
        json!({
            "source_sheet_id": "xy1",
            "equation_id": "custom_line",
            "custom_dsl": "y = a * x + b",
            "parameter_rules": [{ "name": "c", "scope": "constant", "value": 0.0 }]
        }),
    );
    assert_eq!(dsl_unk.code, ErrorCode::BadInput);

    ok_cmd(
        &mut session,
        "fit.run",
        json!({
            "sheet_id": "fit-dsl",
            "source_sheet_id": "xy1",
            "equation_id": "custom_line",
            "custom_dsl": "y = a * x + b",
            "parameter_rules": [{ "name": "a", "scope": "constant", "value": 1.0 }]
        }),
    );
    assert_eq!(
        err_cmd(
            &mut session,
            "analysis.updateParams",
            json!({ "sheet_id": "fit-dsl", "parameter_rules": [{ "name": "missing", "scope": "local" }] })
        )
        .code,
        ErrorCode::BadInput
    );
}

#[test]
fn audit_120_xy_glm_raw_replicates_uses_all_replicates_and_rejects_presummarized() {
    let mut doc = ProjectDocument::new("Audit #120 XY GLM").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 5).unwrap();
    xy.x_title = String::from("X");
    xy.data_sets[0].title = String::from("Control");
    xy.data_sets[1].title = String::from("Extreme");
    for r in 0..5 {
        xy.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
        xy.data_sets[0].cells[r][1] = Cell::number((r + 1) as f64 + 0.2);
        xy.data_sets[1].cells[r][0] = Cell::number(10.0 + (r as f64));
        xy.data_sets[1].cells[r][1] = Cell::number(9000.0 + (r as f64));
    }
    doc.add_data_sheet(DataSheet::new("xy-rep", "XY RawReplicates", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut xy_sum = XyTable::new(SubcolumnMode::MeanSdN, 1, 5).unwrap();
    for r in 0..5 {
        xy_sum.x_cells[r][0] = Cell::number((r + 1) as f64);
        xy_sum.data_sets[0].cells[r][0] = Cell::number(10.0 + (r as f64));
        xy_sum.data_sets[0].cells[r][1] = Cell::number(1.0);
        xy_sum.data_sets[0].cells[r][2] = Cell::number(3.0);
    }
    doc.add_data_sheet(DataSheet::new("xy-sum", "XY Summary", TableArchetype::Xy(xy_sum)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);

    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "sheet_id": "glm-rep", "source_sheet_id": "xy-rep", "kind": "linear_glm", "predictor_columns": [0], "response_column": 1 }),
    );
    let res = session.document().result_sheet(&SheetId::new("glm-rep")).unwrap();
    assert_eq!(res.status, ResultStatus::UpToDate);
    let intercept = match res.tabs[0].rows[0][0] {
        curvebook_doc::ResultCell::Number { value, .. } => value,
        ref other => panic!("expected numeric Intercept cell, got {other:?}"),
    };
    assert!(intercept > 4000.0, "replicate-2 values (~9000) must enter XY GLM, got Intercept={intercept}");
    let resid_tab = res.tabs.iter().find(|t| t.row_labels.iter().any(|l| l.contains("rep 2"))).expect("replicate 2 rows must be present");
    assert_eq!(resid_tab.row_labels.len(), 10, "all 5*2 raw replicate observations must be present");

    // Pre-summarized XY table remains rejected with BadInput.
    assert_eq!(
        err_cmd(
            &mut session,
            "stats.run",
            json!({ "source_sheet_id": "xy-sum", "kind": "linear_glm", "predictor_columns": [0], "response_column": 0 })
        )
        .code,
        ErrorCode::BadInput
    );
}
