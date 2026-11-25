//! Engine integration tests for Wave 22c-2 command handlers:
//! - WF-30: Calculated variable manual edit rejection and pre-mutation formula cycle checks.
//! - WF-31: `table.convertArchetype` supporting `"contingency"`.
//! - `graph.setKind` honoring `option_id` while preserving full `kind` backward compatibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    Cell, CellValue, DataSheet, DiagnosticPlotKind, GraphKind, GraphicalDataObject, GroupedArrangement, MultipleVariablesTable, TableArchetype,
    VariableSource,
};
use curvebook_engine::{ErrorCode, Session};
use serde_json::json;

use super::common::{assert_bad_input, assert_err_code, ds, grouped_rep_sheet, gs, mv_sheet, ok_cmd, session_with};

fn make_mv_sheet(id: &str) -> DataSheet {
    mv_sheet(
        id,
        "MV Calc",
        &[],
        &[
            ("A", None, &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)]),
            ("B", None, &[Cell::number(10.0), Cell::number(20.0), Cell::number(30.0), Cell::number(40.0)]),
            ("C", None, &[]),
        ],
    )
}

fn mv_table<'a>(session: &'a Session, id: &str) -> &'a MultipleVariablesTable {
    let TableArchetype::MultipleVariables(mv) = &ds(session, id).archetype else { panic!("expected MultipleVariables") };
    mv
}

#[test]
fn calculated_column_rejects_manual_cell_edits_clear_and_paste() {
    let mut session = session_with("Calc Guards", vec![make_mv_sheet("mv-calc")]);
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-calc", "col": 2, "formula": "=A + B" }));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(11.0));

    // 1-4. Manual setCell, mixed setCells, clearCells, and paste hitting calculated col 2 -> atomic Conflict.
    for (cmd_id, params) in [
        ("table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 2, "value": 999.0 })),
        (
            "table.setCells",
            json!({ "sheet_id": "mv-calc", "edits": [{ "row": 0, "col": 0, "value": 99.0 }, { "row": 0, "col": 2, "value": 777.0 }] }),
        ),
        ("table.clearCells", json!({ "sheet_id": "mv-calc", "row": 1, "col": 2 })),
        ("table.clearCells", json!({ "sheet_id": "mv-calc", "row_start": 0, "row_end": 1, "col_start": 1, "col_end": 2 })),
        ("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "text": "50\t60\n70\t80" })),
        ("table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "rows": [[50.0, 60.0]] })),
    ] {
        let _ = assert_err_code(&mut session, cmd_id, params, ErrorCode::Conflict);
    }
    let mv = mv_table(&session, "mv-calc");
    assert_eq!(
        (mv.variables[0].cells[0].active_number(), mv.variables[1].cells[0].active_number(), mv.variables[2].cells[0].active_number()),
        (Some(1.0), Some(10.0), Some(11.0))
    );

    // 5-6. Editing/pasting entered cols succeeds and recalculates col 2.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "mv-calc", "row": 0, "col": 0, "value": 5.0 }));
    assert_eq!(mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), Some(15.0));

    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 0, "text": "2\n3" }));
    assert_eq!(
        (mv_table(&session, "mv-calc").variables[2].cells[0].active_number(), mv_table(&session, "mv-calc").variables[2].cells[1].active_number()),
        (Some(12.0), Some(23.0))
    );

    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "mv-calc", "start_row": 0, "start_col": 1, "text": "100\n200" }));
    let mv = mv_table(&session, "mv-calc");
    assert_eq!((mv.variables[2].cells[0].active_number(), mv.variables[2].cells[1].active_number()), (Some(102.0), Some(203.0)));
    assert!(matches!(mv.variables[2].source, VariableSource::Calculated { .. }));

    // 7. clearCells(all: true) clears entered cells and keeps calculated variable definition.
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "mv-calc", "all": true }));
    let mv = mv_table(&session, "mv-calc");
    assert!(matches!(mv.variables[2].source, VariableSource::Calculated { .. }));
    assert!(matches!(mv.variables[0].cells[0].value, CellValue::Empty));
}

#[test]
fn calculated_variable_cycle_guard_runs_before_mutation() {
    let mut session = session_with("Cycle Guard", vec![make_mv_sheet("mv-cyc")]);
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-cyc", "col": 1, "formula": "=A * 2" }));

    for params in [
        json!({ "sheet_id": "mv-cyc", "col": 2, "formula": "=C + 1" }),
        json!({ "sheet_id": "mv-cyc", "title": "LoopCol", "formula": "=LoopCol + 1" }),
        json!({ "sheet_id": "mv-cyc", "col": 0, "formula": "=B + 1" }),
        json!({ "sheet_id": "mv-cyc", "col": 1, "formula": "=B + A" }),
    ] {
        let _ = assert_err_code(&mut session, "table.setCalculatedVariable", params, ErrorCode::CycleDetected);
    }

    let mv = mv_table(&session, "mv-cyc");
    assert_eq!(mv.variables.len(), 3);
    assert!(matches!(mv.variables[0].source, VariableSource::Entered));
    assert_eq!(mv.variables[0].cells[0].active_number(), Some(1.0));
    assert!(matches!(mv.variables[2].source, VariableSource::Entered));
    assert!(matches!(&mv.variables[1].source, VariableSource::Calculated { formula } if formula.trim_start_matches('=').trim() == "A * 2"));
    assert_eq!(mv.variables[1].cells[1].active_number(), Some(4.0));
}

#[test]
fn convert_archetype_supports_contingency() {
    let grp = grouped_rep_sheet("grp-1", "2x2 Counts", &["Exposed", "Unexposed"], &[("Case", &[&[18.0], &[6.0]]), ("Control", &[&[12.0], &[24.0]])]);
    let mut session = session_with("Convert Contingency", vec![grp]);
    ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "grp-1", "target_kind": "contingency" }));

    let TableArchetype::Contingency(cont) = &ds(&session, "grp-1").archetype else { panic!("expected Contingency") };
    assert_eq!(cont.row_titles, vec![String::from("Exposed"), String::from("Unexposed")]);
    assert_eq!((cont.data_sets[0].cells[0][0].active_number(), cont.data_sets[1].cells[1][0].active_number()), (Some(18.0), Some(24.0)));
}

#[test]
fn graph_set_kind_honors_option_id_and_preserves_kind_compat() {
    let mut session = session_with("Graph OptionId", vec![make_mv_sheet("mv-1")]);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "mv-1", "kind": "xy" }));

    for (option_id, kind_arg, expected) in [
        ("diag.residual", "xy", GraphKind::Diagnostic(DiagnosticPlotKind::ResidualPlot)),
        ("diag.normal_qq", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::NormalQqPlot)),
        ("diag.heatmap", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)),
        ("diag.volcano", "xy", GraphKind::Diagnostic(DiagnosticPlotKind::Volcano)),
        ("diag.bland_altman", "column", GraphKind::Diagnostic(DiagnosticPlotKind::BlandAltman)),
        ("diag.roc", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::RocCurve)),
        ("diag.forest", "diagnostic", GraphKind::Diagnostic(DiagnosticPlotKind::ForestPlot)),
    ] {
        ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "kind": kind_arg, "option_id": option_id }));
        assert_eq!(gs(&session, "g-1").spec.kind, expected, "mismatch for `{option_id}`");
    }

    ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "kind": "column", "option_id": "col.violin" }));
    assert_eq!(gs(&session, "g-1").spec.kind, GraphKind::Column);
    assert!(matches!(gs(&session, "g-1").spec.series[0].gdo, GraphicalDataObject::Violin { half_violin_with_points: false, .. }));

    ok_cmd(
        &mut session,
        "graph.setKind",
        json!({ "sheet_id": "g-1", "kind": "column", "option_id": "col.box_whisker", "reset_series_style": false }),
    );
    assert!(matches!(gs(&session, "g-1").spec.series[0].gdo, GraphicalDataObject::Violin { .. }));

    for (kind, expected) in [("grouped", GraphKind::Grouped), ("contingency", GraphKind::Contingency)] {
        ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "kind": kind, "option_id": "grouped.stacked" }));
        assert_eq!((gs(&session, "g-1").spec.kind, gs(&session, "g-1").spec.grouped_arrangement), (expected, GroupedArrangement::Stacked));
    }

    ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "kind": "survival" }));
    assert_eq!(gs(&session, "g-1").spec.kind, GraphKind::Survival);
    ok_cmd(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "kind": "xy", "option_id": "xy_points_connecting" }));
    assert_eq!(gs(&session, "g-1").spec.kind, GraphKind::Xy);
    let _ = assert_bad_input(&mut session, "graph.setKind", json!({ "sheet_id": "g-1", "option_id": "diag.residual" }));
}
