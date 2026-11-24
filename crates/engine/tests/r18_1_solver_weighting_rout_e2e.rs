//! End-to-end engine command, reactive `Cleaned data` chaining, and methods report tests for R18.1.

use curvebook_doc::{
    AnalysisKind, Cell, ConvergenceStrictness, DataSheet, MultipleVariablesTable, OutlierHandling, ProjectDocument, ReplicateFitMode, ResultStatus,
    SheetId, SubcolumnMode, TableArchetype, VariableRole, WeightingScheme, WeightingSource, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

fn ok(session: &mut Session, cmd: &str, params: serde_json::Value) {
    let res = session.execute(cmd, params);
    assert!(res.is_ok(), "command `{cmd}` failed: {:?}", res.err());
}

#[test]
fn r18_1_engine_commands_cleaned_data_chaining_and_methods_report() {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 14).unwrap();
    let jitter = [0.1, -0.1, 0.05, -0.08, 0.12, -0.05, 0.07, -0.09, 0.06, -0.04, 0.08, -0.07, 0.03, 40.0];
    for (r, &e) in jitter.iter().enumerate() {
        let x = (r + 1) as f64;
        let y = 2.0 * x + 5.0 + e;
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let mut doc = ProjectDocument::new("R18.1 E2E").unwrap();
    doc.data_sheets.push(DataSheet::new(SheetId::new("xy1"), "ROUT Decay", TableArchetype::Xy(xy)).unwrap());
    let mut session = Session::from_document(doc);

    // Create nonlinear fit with PredictedY, ROUT Eliminate, emit_cleaned_data_tab, RowMeansOnly, and Medium strictness.
    ok(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "nl1",
            "source_sheet_id": "xy1",
            "kind": "straight_line",
            "method": "rout",
            "fdr_q_percent": 1.0,
            "weighting": "1/y2",
            "weighting_source": "PredictedY",
            "outlier_handling": "Eliminate",
            "emit_cleaned_data_tab": true,
            "replicate_fit_mode": "RowMeansOnly",
            "convergence_strictness": "Medium",
            "profile_likelihood_ci": false
        }),
    );

    let rs1 = session.document().result_sheet(&SheetId::new("nl1")).expect("nl1");
    assert_eq!(rs1.status, ResultStatus::UpToDate);
    assert!(rs1.tabs.iter().any(|t| t.id == "cleaned_data" && t.title == "Cleaned data"));
    assert!(rs1.analyzable_data.is_some(), "emit_cleaned_data_tab must populate analyzable_data");
    assert!(
        matches!(
            &rs1.analysis.kind,
            AnalysisKind::NonlinearRegression(nl)
                if nl.weighting == WeightingScheme::RelativeSquare
                    && nl.weighting_source == WeightingSource::PredictedY
                    && nl.outlier_handling == OutlierHandling::Eliminate
                    && nl.emit_cleaned_data_tab
                    && nl.replicate_fit_mode == ReplicateFitMode::RowMeansOnly
                    && nl.convergence_strictness == ConvergenceStrictness::Medium
        ),
        "unexpected NonlinearFitSpec: {:?}",
        rs1.analysis.kind
    );

    // Downstream analysis binds directly to the analyzable Cleaned data ResultSheet.
    ok(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "nl_downstream",
            "source_sheet_id": "nl1",
            "kind": "straight_line",
            "profile_likelihood_ci": false
        }),
    );
    let rs_down = session.document().result_sheet(&SheetId::new("nl_downstream")).expect("nl_downstream");
    assert_eq!(rs_down.status, ResultStatus::UpToDate);
    let slope_clean = rs_down.lookup_scalar("Slope", Some(0)).unwrap();
    assert!((slope_clean - 1.998_956).abs() < 1e-3, "downstream cleaned slope={slope_clean}");

    // Methods report describes PredictedY IRLS, Medium strictness, RowMeansOnly, and Cleaned data.
    let report = generate_methods_report(session.document(), &SheetId::new("nl1"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("1 / Y_curve^2"), "missing PredictedY IRLS in report:\n{report}");
    assert!(report.contains("Medium"), "missing convergence strictness in report:\n{report}");
    assert!(report.contains("row means"), "missing RowMeansOnly in report:\n{report}");
    assert!(report.contains("Cleaned data"), "missing Cleaned data mention in report:\n{report}");

    // Update to IdentifyAndCountOnly + Quick strictness via analysis.updateParams.
    ok(
        &mut session,
        "analysis.updateParams",
        json!({
            "sheet_id": "nl1",
            "outlier_handling": "IdentifyAndCountOnly",
            "convergence_strictness": "Quick"
        }),
    );
    let rs_updated = session.document().result_sheet(&SheetId::new("nl1")).expect("nl1 updated");
    assert_eq!(rs_updated.status, ResultStatus::UpToDate);
    assert!(rs_updated.eliminated_outliers.is_empty());
    assert_eq!(rs_updated.lookup_scalar("ROUT outliers", None), Some(1.0));
}

#[test]
fn r18_1_engine_multivar_multi_response_mv_y_columns() {
    let mut mv = MultipleVariablesTable::new(4, 12).unwrap();
    mv.variables[0].title = "Group".to_owned();
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = "X".to_owned();
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = "RespA".to_owned();
    mv.variables[2].lock_role(VariableRole::Continuous);
    mv.variables[3].title = "RespB".to_owned();
    mv.variables[3].lock_role(VariableRole::Continuous);
    for i in 0..6 {
        let x = (i + 1) as f64;
        mv.variables[0].cells[i] = Cell::text("A");
        mv.variables[1].cells[i] = Cell::number(x);
        mv.variables[2].cells[i] = Cell::number(1.0 + 2.0 * x);
        mv.variables[3].cells[i] = Cell::number(10.0 - 0.5 * x);

        mv.variables[0].cells[6 + i] = Cell::text("B");
        mv.variables[1].cells[6 + i] = Cell::number(x);
        mv.variables[2].cells[6 + i] = Cell::number(3.0 + 4.0 * x);
        mv.variables[3].cells[6 + i] = Cell::number(20.0 - 1.5 * x);
    }
    let mut doc = ProjectDocument::new("R18.1 MV").unwrap();
    doc.data_sheets.push(DataSheet::new(SheetId::new("mv1"), "MV MultiY", TableArchetype::MultipleVariables(mv)).unwrap());
    let mut session = Session::from_document(doc);

    ok(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "nl_mv",
            "source_sheet_id": "mv1",
            "kind": "straight_line",
            "mv_group_columns": [0],
            "mv_x_columns": [1],
            "mv_y_columns": [2, 3],
            "profile_likelihood_ci": false
        }),
    );
    let rs = session.document().result_sheet(&SheetId::new("nl_mv")).expect("nl_mv");
    assert_eq!(rs.status, ResultStatus::UpToDate);
    assert_eq!(rs.fitted_curves.len(), 4);
    assert!((rs.lookup_scalar("Slope", Some(0)).unwrap() - 2.0).abs() < 1e-8);
    assert!((rs.lookup_scalar("Slope", Some(1)).unwrap() - 4.0).abs() < 1e-8);
    assert!((rs.lookup_scalar("Slope", Some(2)).unwrap() - (-0.5)).abs() < 1e-8);
    assert!((rs.lookup_scalar("Slope", Some(3)).unwrap() - (-1.5)).abs() < 1e-8);
}
