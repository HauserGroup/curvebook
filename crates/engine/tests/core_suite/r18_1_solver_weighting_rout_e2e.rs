//! End-to-end engine command, reactive `Cleaned data` chaining, and methods report tests for R18.1.

use curvebook_doc::{
    AnalysisKind, Cell, ConvergenceStrictness, OutlierHandling, ReplicateFitMode, ResultStatus, SheetId, VariableRole, WeightingScheme,
    WeightingSource,
};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use serde_json::json;

use super::common::{mv_sheet, ok_cmd, rs, session_with, xy_sheet};

#[test]
fn r18_1_engine_commands_cleaned_data_chaining_and_methods_report() {
    let jitter = [0.1, -0.1, 0.05, -0.08, 0.12, -0.05, 0.07, -0.09, 0.06, -0.04, 0.08, -0.07, 0.03, 40.0];
    let xs: Vec<f64> = (1..=14).map(f64::from).collect();
    let ys: Vec<f64> = xs.iter().zip(jitter).map(|(&x, e)| 2.0 * x + 5.0 + e).collect();
    let mut session = session_with("R18.1 E2E", vec![xy_sheet("xy1", "ROUT Decay", "", &xs, &[("", &ys)])]);

    // Create nonlinear fit with PredictedY, ROUT Eliminate, emit_cleaned_data_tab, RowMeansOnly, and Medium strictness.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "nl1", "source_sheet_id": "xy1", "kind": "straight_line", "method": "rout", "fdr_q_percent": 1.0,
            "weighting": "1/y2", "weighting_source": "PredictedY", "outlier_handling": "Eliminate",
            "emit_cleaned_data_tab": true, "replicate_fit_mode": "RowMeansOnly", "convergence_strictness": "Medium", "profile_likelihood_ci": false
        }),
    );

    let rs1 = rs(&session, "nl1");
    assert_eq!(rs1.status, ResultStatus::UpToDate);
    assert!(rs1.tabs.iter().any(|t| t.id == "cleaned_data" && t.title == "Cleaned data") && rs1.analyzable_data.is_some());
    assert!(matches!(
        &rs1.analysis.kind,
        AnalysisKind::NonlinearRegression(nl)
            if nl.weighting == WeightingScheme::RelativeSquare
                && nl.weighting_source == WeightingSource::PredictedY
                && nl.outlier_handling == OutlierHandling::Eliminate
                && nl.emit_cleaned_data_tab
                && nl.replicate_fit_mode == ReplicateFitMode::RowMeansOnly
                && nl.convergence_strictness == ConvergenceStrictness::Medium
    ));

    // Downstream analysis binds directly to the analyzable Cleaned data ResultSheet.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "sheet_id": "nl_downstream", "source_sheet_id": "nl1", "kind": "straight_line", "profile_likelihood_ci": false }),
    );
    let rs_down = rs(&session, "nl_downstream");
    assert_eq!(rs_down.status, ResultStatus::UpToDate);
    let slope_clean = rs_down.lookup_scalar("Slope", Some(0)).unwrap();
    assert!((slope_clean - 1.998_956).abs() < 1e-3, "downstream cleaned slope={slope_clean}");

    // Methods report describes PredictedY IRLS, Medium strictness, RowMeansOnly, and Cleaned data.
    let report = generate_methods_report(session.document(), &SheetId::new("nl1"), ReportFormat::PlainText).unwrap();
    for needle in ["1 / Y_curve^2", "Medium", "row means", "Cleaned data"] {
        assert!(report.contains(needle), "missing `{needle}` in report:\n{report}");
    }

    // Update to IdentifyAndCountOnly + Quick strictness via analysis.updateParams.
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "nl1", "outlier_handling": "IdentifyAndCountOnly", "convergence_strictness": "Quick" }),
    );
    let rs_updated = rs(&session, "nl1");
    assert_eq!(
        (&rs_updated.status, rs_updated.eliminated_outliers.is_empty(), rs_updated.lookup_scalar("ROUT outliers", None)),
        (&ResultStatus::UpToDate, true, Some(1.0))
    );
}

#[test]
fn r18_1_engine_multivar_multi_response_mv_y_columns() {
    let groups: Vec<Cell> = (0..6).map(|_| Cell::text("A")).chain((0..6).map(|_| Cell::text("B"))).collect();
    let xs: Vec<Cell> = (1..=6).chain(1..=6).map(|i| Cell::number(f64::from(i))).collect();
    let ra: Vec<Cell> =
        (1..=6).map(|i| Cell::number(1.0 + 2.0 * f64::from(i))).chain((1..=6).map(|i| Cell::number(3.0 + 4.0 * f64::from(i)))).collect();
    let rb: Vec<Cell> =
        (1..=6).map(|i| Cell::number(10.0 - 0.5 * f64::from(i))).chain((1..=6).map(|i| Cell::number(20.0 - 1.5 * f64::from(i)))).collect();
    let mut session = session_with(
        "R18.1 MV",
        vec![mv_sheet(
            "mv1",
            "MV MultiY",
            &[],
            &[
                ("Group", Some(VariableRole::Categorical), &groups),
                ("X", Some(VariableRole::Continuous), &xs),
                ("RespA", Some(VariableRole::Continuous), &ra),
                ("RespB", Some(VariableRole::Continuous), &rb),
            ],
        )],
    );

    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "sheet_id": "nl_mv", "source_sheet_id": "mv1", "kind": "straight_line",
            "mv_group_columns": [0], "mv_x_columns": [1], "mv_y_columns": [2, 3], "profile_likelihood_ci": false
        }),
    );
    let r = rs(&session, "nl_mv");
    assert_eq!((&r.status, r.fitted_curves.len()), (&ResultStatus::UpToDate, 4));
    for (idx, exp) in [(0, 2.0), (1, 4.0), (2, -0.5), (3, -1.5)] {
        assert!((r.lookup_scalar("Slope", Some(idx)).unwrap() - exp).abs() < 1e-8);
    }
}
