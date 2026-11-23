//! End-to-end `curvebook-engine` tests for `R6.3`: `SelectAndTransform` derived column
//! formulas (`transform.run`, `analysis.run`, `analysis.updateParams`, reactive DAG recompute,
//! methods report, and error diagnostics) using windowed & conditional formula composition.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, Cell, DataSheet, DerivedColumnFormula, MultipleVariablesTable, ProjectDocument, ResultStatus, SheetId, TableArchetype, TableKind,
    TransformAnalysisSpec,
};
use curvebook_engine::{
    CommandOutcome, Session,
    methods_report::{ReportFormat, generate_methods_report},
};
use serde_json::{Value, json};

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn make_mv_session() -> Session {
    let mut mv = MultipleVariablesTable::new(4, 6).unwrap();
    if let [v0, v1, v2, v3] = mv.variables.as_mut_slice() {
        v0.title = String::from("Group");
        v1.title = String::from("Role");
        v2.title = String::from("Time");
        v3.title = String::from("Response");

        let rows = [
            ("G1", "Ctrl", 1.0, 10.0),
            ("G1", "Ctrl", 2.0, 20.0),
            ("G1", "Trt", 3.0, 45.0),
            ("G2", "Ctrl", 1.0, 100.0),
            ("G2", "Trt", 2.0, 150.0),
            ("G2", "Trt", 3.0, 250.0),
        ];
        for (r, (g, role, t, resp)) in rows.iter().enumerate() {
            if let Some(c0) = v0.cells.get_mut(r) {
                *c0 = Cell::text(*g);
            }
            if let Some(c1) = v1.cells.get_mut(r) {
                *c1 = Cell::text(*role);
            }
            if let Some(c2) = v2.cells.get_mut(r) {
                *c2 = Cell::number(*t);
            }
            if let Some(c3) = v3.cells.get_mut(r) {
                *c3 = Cell::number(*resp);
            }
        }
    }

    let mut doc = ProjectDocument::new("R6.3 SelectAndTransform").unwrap();
    doc.add_data_sheet(DataSheet::new("mv1", "Assay", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    Session::from_document(doc)
}

#[test]
fn r63_engine_select_and_transform_derived_columns_update_params_and_dag() {
    let mut session = make_mv_session();

    // 1. Run `transform.run` with `select_and_transform`, `filter_formula`, and windowed/conditional `derived_columns`.
    ok_cmd(
        &mut session,
        "transform.run",
        json!({
            "sheet_id": "tr-sel",
            "source_sheet_id": "mv1",
            "kind": "select_and_transform",
            "target_archetype": "MultipleVariables",
            "filter_formula": "Time >= 2",
            "selected_data_sets": [0, 2, 3],
            "derived_columns": [
                {
                    "title": "CtrlMean",
                    "formula": "mean(Response) FILTER (WHERE Role == \"Ctrl\") OVER (PARTITION BY Group)"
                },
                {
                    "title": "RollSum",
                    "formula": "sum(Response) OVER (PARTITION BY Group ORDER BY Time ROWS BETWEEN 1 PRECEDING AND CURRENT ROW)"
                },
                {
                    "title": "GroupSdTrt",
                    "formula": "stdevif(Response, Role == \"Trt\") OVER (PARTITION BY Group)"
                }
            ]
        }),
    );

    let rs = session.document().result_sheet(&SheetId::new("tr-sel")).unwrap();
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert!(matches!(
        rs.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            target_archetype: TableKind::MultipleVariables,
            ref derived_columns,
            ..
        }) if derived_columns.len() == 3
    ));

    let analyzable = rs.analyzable_data.as_ref().unwrap();
    let TableArchetype::MultipleVariables(out_mv) = &analyzable.archetype else {
        panic!("expected MultipleVariables analyzable output");
    };
    // Time >= 2 retains 4 rows: (G1,Ctrl,2,20), (G1,Trt,3,45), (G2,Trt,2,150), (G2,Trt,3,250)
    assert_eq!(out_mv.row_count(), 4);
    assert_eq!(out_mv.variables.len(), 6); // Group, Time, Response, CtrlMean, RollSum, GroupSdTrt
    assert_eq!(out_mv.variables[3].title, "CtrlMean");
    assert_eq!(out_mv.variables[4].title, "RollSum");
    assert_eq!(out_mv.variables[5].title, "GroupSdTrt");

    // G1 has Ctrl=[20] -> CtrlMean=20.0; G2 has no Ctrl with Time>=2 -> Missing.
    assert!((out_mv.variables[3].cells[0].active_number().unwrap() - 20.0).abs() < 1e-12);
    assert!((out_mv.variables[3].cells[1].active_number().unwrap() - 20.0).abs() < 1e-12);
    assert!(out_mv.variables[3].cells[2].is_empty());

    // RollSum in G1: row 0 = 20, row 1 = 20+45=65; in G2: row 2 = 150, row 3 = 150+250=400.
    assert!((out_mv.variables[4].cells[0].active_number().unwrap() - 20.0).abs() < 1e-12);
    assert!((out_mv.variables[4].cells[1].active_number().unwrap() - 65.0).abs() < 1e-12);
    assert!((out_mv.variables[4].cells[2].active_number().unwrap() - 150.0).abs() < 1e-12);
    assert!((out_mv.variables[4].cells[3].active_number().unwrap() - 400.0).abs() < 1e-12);

    // GroupSdTrt in G2 (150, 250 -> sd = sqrt(5000)); G1 has only 1 Trt -> Missing.
    assert!(out_mv.variables[5].cells[0].is_empty());
    assert!((out_mv.variables[5].cells[2].active_number().unwrap() - 5000.0_f64.sqrt()).abs() < 1e-10);

    // Methods report mentions derived column names:
    let report = generate_methods_report(session.document(), &SheetId::new("tr-sel"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("CtrlMean") && report.contains("RollSum") && report.contains("GroupSdTrt"));

    // 2. Reactive DAG recomputation when upstream source cell mutates:
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "mv1", "row": 2, "col": 3, "value": 80.0 }));
    let rs_after_edit = session.document().result_sheet(&SheetId::new("tr-sel")).unwrap();
    let TableArchetype::MultipleVariables(mv_after_edit) = &rs_after_edit.analyzable_data.as_ref().unwrap().archetype else {
        panic!("expected MV");
    };
    assert!((mv_after_edit.variables[4].cells[1].active_number().unwrap() - 100.0).abs() < 1e-12); // 20 + 80

    // 3. `analysis.updateParams` with `Title = formula` assignment string syntax & `target_archetype: Xy`:
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({
            "sheet_id": "tr-sel",
            "selected_data_sets": [2],
            "params": {
                "filter_formula": null,
                "target_archetype": "Xy",
                "derived_columns": [
                    "Centered = center(Response) OVER (PARTITION BY Group)",
                    "RankInGroup = rank(Response) OVER (PARTITION BY Group ORDER BY Response DESC)"
                ]
            }
        }),
    );
    let rs_updated = session.document().result_sheet(&SheetId::new("tr-sel")).unwrap();
    assert_eq!(rs_updated.status, ResultStatus::UpToDate);
    assert_eq!(
        rs_updated.analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::SelectAndTransform {
            filter_formula: None,
            target_archetype: TableKind::Xy,
            derived_columns: vec![
                DerivedColumnFormula::new("Centered", "center(Response) OVER (PARTITION BY Group)"),
                DerivedColumnFormula::new("RankInGroup", "rank(Response) OVER (PARTITION BY Group ORDER BY Response DESC)"),
            ],
        })
    );
    let TableArchetype::Xy(out_xy) = &rs_updated.analyzable_data.as_ref().unwrap().archetype else {
        panic!("expected Xy");
    };
    assert_eq!(out_xy.x_title, "Time");
    assert_eq!(out_xy.data_sets.len(), 2);
    assert_eq!(out_xy.data_sets[0].title, "Centered");
    assert_eq!(out_xy.data_sets[1].title, "RankInGroup");
    assert_eq!(out_xy.row_count(), 6);

    // 4. Malformed derived_columns rejects with structured error:
    let bad = session.execute(
        "transform.run",
        json!({
            "source_sheet_id": "mv1",
            "kind": "select_and_transform",
            "derived_columns": ["NoEqualsSign"]
        }),
    );
    assert!(bad.is_err());
}
