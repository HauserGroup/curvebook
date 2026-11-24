//! Engine integration tests for Wave 22c-2 command handlers:
//! - WF-30: Calculated variable manual edit rejection (`setCell`, `setCells`, `clearCells`, `paste`)
//!   and `table.setCalculatedVariable` pre-mutation formula cycle checks.
//! - WF-31: `table.convertArchetype` supporting `"contingency"` (`TargetArchetypeKind::Contingency`).
//! - `graph.setKind` honoring `option_id` while preserving full `kind` backward compatibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, CellValue, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject, GroupedArrangement, GroupedTable, MultipleVariablesTable,
    ProjectDocument, SheetId, SubcolumnMode, TableArchetype, VariableSource,
};
use curvebook_engine::{ErrorCode, Session};
use serde_json::json;

fn make_mv_sheet(id: &str) -> DataSheet {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    if let [v0, v1, v2] = mv.variables.as_mut_slice() {
        v0.title = String::from("A");
        v1.title = String::from("B");
        v2.title = String::from("C");
        for (r, (c0, c1)) in v0.cells.iter_mut().zip(v1.cells.iter_mut()).enumerate().take(4) {
            *c0 = Cell::number((r + 1) as f64);
            *c1 = Cell::number(((r + 1) * 10) as f64);
        }
    }
    DataSheet::new(SheetId::new(id), "MV Calc", TableArchetype::MultipleVariables(mv)).unwrap()
}

fn mv_table<'a>(session: &'a Session, id: &str) -> &'a MultipleVariablesTable {
    let ds = session.document().data_sheet(&SheetId::new(id)).unwrap();
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        panic!("expected MultipleVariables");
    };
    mv
}

#[test]
fn calculated_column_rejects_manual_cell_edits_clear_and_paste() {
    let mut doc = ProjectDocument::new("Calc Guards").unwrap();
    doc.add_data_sheet(make_mv_sheet("mv-calc")).unwrap();
    let mut session = Session::from_document(doc);

    session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 2, "formula": "=A + B" })).unwrap();
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(11.0));

    // 1. table.setCell targeting calculated col 2 -> ErrorCode::Conflict, value preserved.
    let err_set_cell = session.execute("table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 2, "value": 999.0 })).unwrap_err();
    assert_eq!(err_set_cell.code, ErrorCode::Conflict);
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(11.0));

    // 2. table.setCells mixed batch hitting entered col 0 + calculated col 2 -> atomic Conflict, neither edited.
    let err_set_cells = session
        .execute(
            "table.setCells",
            json!({
                "sheet_id": "mv-calc",
                "edits": [
                    { "row": 0, "col": 0, "value": 99.0 },
                    { "row": 0, "col": 2, "value": 777.0 }
                ]
            }),
        )
        .unwrap_err();
    assert_eq!(err_set_cells.code, ErrorCode::Conflict);
    assert_eq!(mv_table(&session, "mv-calc").variables[0].cells[0].active_number(), Some(1.0));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(11.0));

    // 3. table.clearCells targeting single cell or range intersecting calculated col 2 -> Conflict.
    let err_clear_cell = session.execute("table.clearCells", json!({ "sheet_id": "mv-calc", "row": 1, "col": 2 })).unwrap_err();
    assert_eq!(err_clear_cell.code, ErrorCode::Conflict);

    let err_clear_range = session
        .execute("table.clearCells", json!({ "sheet_id": "mv-calc", "row_start": 0, "row_end": 1, "col_start": 1, "col_end": 2 }))
        .unwrap_err();
    assert_eq!(err_clear_range.code, ErrorCode::Conflict);
    assert_eq!(mv_table(&session, "mv-calc").variables[1].cells[0].active_number(), Some(10.0));

    // 4. table.paste overlapping calculated col 2 (`text` and `rows` payloads) -> Conflict, col 1 preserved.
    let err_paste =
        session.execute("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "text": "50\t60\n70\t80" })).unwrap_err();
    assert_eq!(err_paste.code, ErrorCode::Conflict);
    assert_eq!(mv_table(&session, "mv-calc").variables[1].cells[0].active_number(), Some(10.0));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(11.0));

    let err_paste_rows =
        session.execute("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "rows": [[50.0, 60.0]] })).unwrap_err();
    assert_eq!(err_paste_rows.code, ErrorCode::Conflict);
    assert_eq!(mv_table(&session, "mv-calc").variables[1].cells[0].active_number(), Some(10.0));

    // 5. Editing entered col 0 via setCell and single-column paste succeeds and triggers formula recalculation.
    session.execute("table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 0, "value": 5.0 })).unwrap();
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(15.0));

    session.execute("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 0, "text": "2\n3" })).unwrap();
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(12.0));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[1].active_number(), Some(23.0));

    // 6. Pasting onto entered col 1 updates it and recalculates col 2 without error.
    session.execute("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "text": "100\n200" })).unwrap();
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(102.0));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[1].active_number(), Some(203.0));
    assert!(matches!(mv_table(&session, "mv-calc").variables[2].source, VariableSource::Calculated { .. }));

    // 7. table.clearCells with all: true clears entered cells and keeps calculated variable definition.
    session.execute("table.clearCells", json!({ "sheet_id": "mv-calc", "all": true })).unwrap();
    assert!(matches!(mv_table(&session, "mv-calc").variables[2].source, VariableSource::Calculated { .. }));
    assert!(matches!(mv_table(&session, "mv-calc").variables[0].cells[0].value, CellValue::Empty));
}

#[test]
fn calculated_variable_cycle_guard_runs_before_mutation() {
    let mut doc = ProjectDocument::new("Cycle Guard").unwrap();
    doc.add_data_sheet(make_mv_sheet("mv-cyc")).unwrap();
    let mut session = Session::from_document(doc);

    session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "col": 1, "formula": "=A * 2" })).unwrap();

    // Direct self-reference on existing entered col C -> CycleDetected without mutating C to Calculated.
    let err_self = session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "col": 2, "formula": "=C + 1" })).unwrap_err();
    assert_eq!(err_self.code, ErrorCode::CycleDetected);
    assert!(matches!(mv_table(&session, "mv-cyc").variables[2].source, VariableSource::Entered));

    // Direct self-reference on brand-new named variable -> CycleDetected without adding column.
    let err_new_self =
        session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "title": "LoopCol", "formula": "=LoopCol + 1" })).unwrap_err();
    assert_eq!(err_new_self.code, ErrorCode::CycleDetected);
    assert_eq!(mv_table(&session, "mv-cyc").variables.len(), 3);

    // Mutual 2-hop cycle: A = B + 1 when B = A * 2 -> CycleDetected without mutating A.
    let err_mutual = session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "col": 0, "formula": "=B + 1" })).unwrap_err();
    assert_eq!(err_mutual.code, ErrorCode::CycleDetected);
    assert!(matches!(mv_table(&session, "mv-cyc").variables[0].source, VariableSource::Entered));
    assert_eq!(mv_table(&session, "mv-cyc").variables[0].cells[0].active_number(), Some(1.0));

    // Self-cycle updating existing calculated column B -> preserves original formula.
    let err_update_self = session.execute("table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "col": 1, "formula": "=B + A" })).unwrap_err();
    assert_eq!(err_update_self.code, ErrorCode::CycleDetected);
    assert!(matches!(
        &mv_table(&session, "mv-cyc").variables[1].source,
        VariableSource::Calculated { formula } if formula.trim_start_matches('=').trim() == "A * 2"
    ));
    assert_eq!(mv_table(&session, "mv-cyc").variables[1].cells[1].active_number(), Some(4.0));
}

#[test]
fn convert_archetype_supports_contingency() {
    let mut doc = ProjectDocument::new("Convert Contingency").unwrap();
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(1), 2, 2).unwrap();
    grp.row_titles = vec![String::from("Exposed"), String::from("Unexposed")];
    grp.data_sets[0].title = String::from("Case");
    grp.data_sets[1].title = String::from("Control");
    grp.data_sets[0].cells[0][0] = Cell::number(18.0);
    grp.data_sets[1].cells[0][0] = Cell::number(12.0);
    grp.data_sets[0].cells[1][0] = Cell::number(6.0);
    grp.data_sets[1].cells[1][0] = Cell::number(24.0);
    doc.add_data_sheet(DataSheet::new(SheetId::new("grp-1"), "2x2 Counts", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    session.execute("table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "contingency" })).unwrap();

    let converted = session.document().data_sheet(&SheetId::new("grp-1")).unwrap();
    let TableArchetype::Contingency(cont) = &converted.archetype else {
        panic!("expected Contingency archetype, got {:?}", converted.archetype);
    };
    assert_eq!(cont.row_titles, vec![String::from("Exposed"), String::from("Unexposed")]);
    assert_eq!(cont.data_sets[0].cells[0][0].active_number(), Some(18.0));
    assert_eq!(cont.data_sets[1].cells[1][0].active_number(), Some(24.0));
}

#[test]
fn graph_set_kind_honors_option_id_and_preserves_kind_compat() {
    let mut doc = ProjectDocument::new("Graph OptionId").unwrap();
    doc.add_data_sheet(make_mv_sheet("mv-1")).unwrap();
    let mut session = Session::from_document(doc);

    session.execute("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "mv-1", "kind": "xy" })).unwrap();

    for (option_id, kind_arg, expected) in [
        ("diag.residual", "xy", GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot)),
        ("diag.normal_qq", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot)),
        ("diag.heatmap", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)),
        ("diag.volcano", "xy", GraphKind::Diagnostic(DiagnosticPlotKind::Volcano)),
        ("diag.bland_altman", "column", GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman)),
        ("diag.roc", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve)),
        ("diag.forest", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot)),
    ] {
        session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": kind_arg, "option_id": option_id })).unwrap();
        let g = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
        assert_eq!(g.spec.kind, expected, "mismatch for option_id `{option_id}`");
    }

    // Column GDO presets update series styling when reset_series_style is true (default),
    // and respect reset_series_style: false.
    session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "column", "option_id": "col.violin" })).unwrap();
    let g_violin = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    assert_eq!(g_violin.spec.kind, GraphKind::Column);
    assert!(matches!(g_violin.spec.series[0].gdo, GraphicalDataObject::Violin { half_violin_with_points: false, .. }));

    session
        .execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "column", "option_id": "col.box_whisker", "reset_series_style": false }))
        .unwrap();
    let g_keep = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    assert!(matches!(g_keep.spec.series[0].gdo, GraphicalDataObject::Violin { .. }));

    // Grouped stacked sets GroupedArrangement::Stacked; contingency + grouped.stacked preserves Contingency.
    session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "grouped", "option_id": "grouped.stacked" })).unwrap();
    let g_stacked = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    assert_eq!(g_stacked.spec.kind, GraphKind::Grouped);
    assert_eq!(g_stacked.spec.grouped_arrangement, GroupedArrangement::Stacked);

    session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "contingency", "option_id": "grouped.stacked" })).unwrap();
    let g_cont = session.document().graph_sheet(&SheetId::new("g-1")).unwrap();
    assert_eq!(g_cont.spec.kind, GraphKind::Contingency);
    assert_eq!(g_cont.spec.grouped_arrangement, GroupedArrangement::Stacked);

    // Backward-compatible kind without option_id and with unrecognized legacy option_id.
    session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "survival" })).unwrap();
    assert_eq!(session.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.kind, GraphKind::Survival);

    session.execute("graph.setKind", json!({ "sheet_id": "g-1", "kind": "xy", "option_id": "xy_points_connecting" })).unwrap();
    assert_eq!(session.document().graph_sheet(&SheetId::new("g-1")).unwrap().spec.kind, GraphKind::Xy);

    // Missing `kind` still rejects at `/kind`.
    let err_missing_kind = session.execute("graph.setKind", json!({ "sheet_id": "g-1", "option_id": "diag.residual" })).unwrap_err();
    assert_eq!(err_missing_kind.code, ErrorCode::BadInput);
}
