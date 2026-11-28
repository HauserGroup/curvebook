//! `R19.4`: the methods report must describe the observations an analysis actually used across
//! paired analyses, listwise/REML ANOVA, regression (linear, Deming, nonlinear, GLM, simple
//! logistic, Passing–Bablok), survival (Kaplan–Meier, Cox PH), and unpaired analyses with ignored
//! non-numeric or excluded cells.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, CoxTieMethod, DataSheet, FactorialAnovaSpec, GlmFamily, GlmSpec, GroupedTable,
    LinearRegressionSpec, MultipleVariablesTable, NonlinearFitSpec, ObservationAccounting, ObservationAccountingKind, OneWayAnovaSpec, OneWayMethod,
    PairedObservationAccounting, ProjectDocument, RegressionCiMethod, RepeatedMeasuresDesign, ResultStatus, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, SurvivalOtherValuePolicy, SurvivalTable, TableArchetype, VariableRole, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::methods_report::{ReportFormat, ReportScope, generate_methods_report, generate_methods_report_with_scope};
use serde_json::json;

use super::common::{col_sheet, ok_cmd, rs, session_with};

fn run_spec(session: &mut Session, cmd: &str, id: &str, source_id: &str, kind: AnalysisKind) {
    let spec = AnalysisSpec::new(id, source_id, kind).unwrap();
    ok_cmd(session, cmd, json!({ "spec": spec }));
}

const OVERCLAIM: &str = "all finite active observations were included";
const EXPECTED: PairedObservationAccounting =
    PairedObservationAccounting { pairs_analyzed: 4, omitted_excluded: 1, omitted_non_numeric: 1, omitted_missing_partner: 1 };

/// Paired table with 4 complete pairs and one omitted row per reason (missing partner, excluded, non-numeric, empty).
fn paired_doc() -> ProjectDocument {
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 8).unwrap();
    col.data_sets[0].title = String::from("Before");
    col.data_sets[1].title = String::from("After");
    for (r, (a, b)) in [
        (Cell::number(10.0), Cell::number(12.0)),
        (Cell::number(11.0), Cell::number(13.5)),
        (Cell::number(9.0), Cell::number(11.0)),
        (Cell::number(12.0), Cell::empty()),
        (Cell::number(10.5), Cell::excluded_number(20.0)),
        (Cell::text("n/a"), Cell::number(14.0)),
        (Cell::empty(), Cell::empty()),
        (Cell::number(13.0), Cell::number(15.2)),
    ]
    .into_iter()
    .enumerate()
    {
        col.data_sets[0].cells[r][0] = a;
        col.data_sets[1].cells[r][0] = b;
    }
    let mut doc = ProjectDocument::new("Paired accounting").unwrap();
    doc.add_data_sheet(DataSheet::new("paired", "Paired data", TableArchetype::Column(col)).unwrap()).unwrap();
    doc
}

fn run(kind: &str) -> (Session, String) {
    let mut session = Session::from_document(paired_doc());
    let id = format!("res-{kind}");
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "paired", "sheet_id": id, "kind": kind }));
    (session, id)
}

#[test]
fn paired_analyses_report_pairs_analyzed_and_omitted_by_reason() {
    for kind in ["paired_t", "ratio_paired_t", "wilcoxon", "paired_sign_test", "bland_altman"] {
        let (session, id) = run(kind);
        let res = rs(&session, &id);
        assert_eq!((&res.status, res.paired_observation_accounting()), (&ResultStatus::UpToDate, Some(EXPECTED)), "{kind}");
        let checklist =
            res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap_or_else(|| panic!("{kind}: no checklist row"));
        assert_eq!(checklist.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 4 retained"), "{kind}");

        let sid = SheetId::new(&id);
        for format in [ReportFormat::Markdown, ReportFormat::PlainText, ReportFormat::Latex] {
            let report = generate_methods_report(session.document(), &sid, format).unwrap();
            assert!(!report.contains(OVERCLAIM), "{kind} {format:?}:\n{report}");
            assert!(
                report.contains(
                    "4 complete pair(s) analyzed; 3 row(s) omitted by complete-case pairing (1 missing a partner value, 1 containing an excluded cell, 1 containing a non-numeric cell)."
                ),
                "{kind} {format:?}:\n{report}"
            );
        }
    }
}

#[test]
fn complete_pairs_report_no_omissions() {
    let mut doc = paired_doc();
    let Some(TableArchetype::Column(col)) = doc.data_sheet_mut(&SheetId::new("paired")).map(|ds| &mut ds.archetype) else {
        panic!("paired column table");
    };
    for r in 3..=6 {
        col.data_sets[0].cells[r][0] = Cell::empty();
        col.data_sets[1].cells[r][0] = Cell::empty();
    }
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "paired", "sheet_id": "res", "kind": "paired_t" }));
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(
        report.contains("4 complete pair(s) analyzed; no rows omitted.")
            && report.contains("0 cells masked as excluded prior to analysis.")
            && !report.contains(OVERCLAIM),
        "{report}"
    );
}

#[test]
fn stale_result_does_not_report_pairing_counts() {
    let (session, id) = run("paired_t");
    let sid = SheetId::new(id);
    let mut doc = session.document().clone();
    doc.result_sheet_mut(&sid).unwrap().status = ResultStatus::Dirty;
    let report = generate_methods_report(&doc, &sid, ReportFormat::PlainText).unwrap();
    assert!(!report.contains("Paired observations"), "{report}");
}

#[test]
fn unpaired_analysis_makes_no_inclusion_claim() {
    let mut sheet = col_sheet("unpaired", "Unpaired data", &[("", &[10.0, 11.0, 12.0, 13.0]), ("", &[14.0, 15.0, 16.0, 17.0])]);
    if let TableArchetype::Column(col) = &mut sheet.archetype {
        col.data_sets[1].cells[3][0] = Cell::text("lost");
    }
    let mut session = session_with("Unpaired", vec![sheet]);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "unpaired", "sheet_id": "res", "kind": "unpaired_t" }));
    let res = rs(&session, "res");
    assert_eq!(res.paired_observation_accounting(), None);
    assert_eq!(
        res.observation_accounting(),
        Some(&ObservationAccounting {
            ignored_non_numeric_cells: 1,
            excluded_cells: 0,
            ..ObservationAccounting::new(ObservationAccountingKind::Unpaired, 7)
        })
    );
    let report = generate_methods_report(session.document(), &SheetId::new("res"), ReportFormat::PlainText).unwrap();
    assert!(
        !report.contains(OVERCLAIM)
            && report.contains("0 cells masked as excluded prior to analysis.")
            && report.contains("7 active numeric observation(s) analyzed; 1 non-numeric cell(s) ignored."),
        "{report}"
    );
}

fn assert_reports_contain(session: &Session, sheet_id: &str, expected_substring: &str, context: &str) {
    let sid = SheetId::new(sheet_id);
    for fmt in [ReportFormat::PlainText, ReportFormat::Markdown, ReportFormat::Latex] {
        for scope in [ReportScope::FullReport, ReportScope::Methods] {
            let report = generate_methods_report_with_scope(session.document(), &sid, fmt, scope).unwrap();
            assert!(!report.contains(OVERCLAIM), "{context} {fmt:?} {scope:?}:\n{report}");
            assert!(report.contains(expected_substring), "{context} {fmt:?} {scope:?}: expected `{expected_substring}` in:\n{report}");
        }
    }
}

#[test]
fn listwise_anova_and_multifactor_report_exact_observation_accounting() {
    // 1-way RM ANOVA (listwise Friedman + classical RM with use_reml_on_missing: false + REML mixed effects).
    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 8).unwrap();
    for (idx, title) in ["Baseline", "Low", "High"].into_iter().enumerate() {
        col.data_sets[idx].title = String::from(title);
    }
    let rm_rows = [
        [Cell::number(10.0), Cell::number(14.0), Cell::number(20.0)],
        [Cell::number(11.0), Cell::number(15.5), Cell::number(21.0)],
        [Cell::number(9.5), Cell::number(13.5), Cell::number(19.0)],
        [Cell::number(12.0), Cell::number(16.0), Cell::number(22.0)],
        [Cell::number(10.5), Cell::number(15.0), Cell::number(20.5)],
        [Cell::number(11.0), Cell::empty(), Cell::number(19.5)],
        [Cell::number(10.0), Cell::excluded_number(50.0), Cell::number(20.0)],
        [Cell::text("bad"), Cell::number(14.5), Cell::number(21.5)],
    ];
    for (r, row) in rm_rows.into_iter().enumerate() {
        for (c, cell) in row.into_iter().enumerate() {
            col.data_sets[c].cells[r][0] = cell;
        }
    }
    let mut session = session_with("RM ANOVA accounting", vec![DataSheet::new("rm1", "RM 1-Way", TableArchetype::Column(col)).unwrap()]);

    // Friedman uses complete-case listwise filtering: 5 complete blocks, 3 omitted (1 missing, 1 excluded, 1 non-numeric).
    run_spec(
        &mut session,
        "stats.run",
        "res-friedman",
        "rm1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::Friedman, ..OneWayAnovaSpec::default() }),
    );
    let friedman_res = rs(&session, "res-friedman");
    let friedman_acc = friedman_res.observation_accounting().unwrap();
    assert_eq!(friedman_acc.kind, ObservationAccountingKind::ListwiseAnova);
    assert_eq!(friedman_acc.analyzed, 5);
    assert_eq!((friedman_acc.omitted_missing, friedman_acc.omitted_excluded, friedman_acc.omitted_non_numeric), (1, 1, 1));
    assert_eq!(friedman_acc.rows_omitted(), 3);
    assert_eq!(friedman_acc.ignored_non_numeric_cells, 1);
    assert_eq!(friedman_acc.excluded_cells, 1);
    let chk = friedman_res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
    assert_eq!(chk.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 5 retained"));
    assert_reports_contain(
        &session,
        "res-friedman",
        "5 complete row(s)/block(s) analyzed; 3 row(s) omitted by listwise filtering (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell). 1 non-numeric cell(s) ignored.",
        "res-friedman",
    );

    // 1-way RM with missing/excluded/non-numeric cells engages mixed-effects REML and retains all 21 active measurements.
    run_spec(
        &mut session,
        "stats.run",
        "res-rm-reml",
        "rm1",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            mixed_effects_reml: false,
            ..OneWayAnovaSpec::default()
        }),
    );
    let reml_res = rs(&session, "res-rm-reml");
    let reml_acc = reml_res.observation_accounting().unwrap();
    assert_eq!(reml_acc.kind, ObservationAccountingKind::MixedEffectsAnova);
    assert_eq!(reml_acc.analyzed, 21);
    assert_eq!(reml_acc.ignored_non_numeric_cells, 1);
    assert_eq!(reml_acc.excluded_cells, 1);
    assert_reports_contain(
        &session,
        "res-rm-reml",
        "21 active observation(s) analyzed by mixed-effects REML (no listwise subject deletion). 1 non-numeric cell(s) ignored.",
        "res-rm-reml",
    );

    // Complete 1-way RM ANOVA uses classical listwise accounting with 0 omitted rows.
    let complete_col = col_sheet(
        "rm1-complete",
        "RM Complete",
        &[("A", &[10.0, 11.0, 9.5, 12.0, 10.5]), ("B", &[14.0, 15.5, 13.5, 16.0, 15.0]), ("C", &[20.0, 21.0, 19.0, 22.0, 20.5])],
    );
    let mut complete_rm_session = session_with("RM Complete", vec![complete_col]);
    run_spec(
        &mut complete_rm_session,
        "stats.run",
        "res-rm-complete",
        "rm1-complete",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            mixed_effects_reml: false,
            ..OneWayAnovaSpec::default()
        }),
    );
    let comp_acc = rs(&complete_rm_session, "res-rm-complete").observation_accounting().unwrap();
    assert_eq!(comp_acc.kind, ObservationAccountingKind::ListwiseAnova);
    assert_eq!(comp_acc.analyzed, 5);
    assert_eq!(comp_acc.rows_omitted(), 0);
    assert_reports_contain(&complete_rm_session, "res-rm-complete", "5 complete row(s)/block(s) analyzed; no rows omitted.", "res-rm-complete");

    // 2-way RM ANOVA on Grouped table (classical balanced when complete, REML when a cell is excluded/missing).
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    grp.row_titles = vec![String::from("T1"), String::from("T2")];
    grp.data_sets[0].title = String::from("Ctrl");
    grp.data_sets[1].title = String::from("Drug");
    let gvals = [[[10.0, 11.0, 12.0], [14.0, 15.0, 16.0]], [[18.0, 19.0, 20.0], [24.0, 25.0, 26.0]]];
    for (r, row_groups) in gvals.iter().enumerate() {
        for (c, reps) in row_groups.iter().enumerate() {
            for (s, &v) in reps.iter().enumerate() {
                grp.data_sets[c].cells[r][s] = Cell::number(v);
            }
        }
    }
    grp.data_sets[1].cells[1][2] = Cell::text("err");
    let mut grp_session = session_with("2-way RM", vec![DataSheet::new("grp2", "Grouped 2-Way", TableArchetype::Grouped(grp)).unwrap()]);
    run_spec(
        &mut grp_session,
        "stats.run",
        "res-rm2",
        "grp2",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            repeated_measures: RepeatedMeasuresDesign::RowFactorMatched,
            fit_reml_on_missing: true,
            ..FactorialAnovaSpec::default()
        }),
    );
    let rm2_acc = rs(&grp_session, "res-rm2").observation_accounting().unwrap();
    assert_eq!(rm2_acc.kind, ObservationAccountingKind::MixedEffectsAnova);
    assert_eq!(rm2_acc.analyzed, 11);
    assert_eq!(rm2_acc.ignored_non_numeric_cells, 1);
    assert_reports_contain(
        &grp_session,
        "res-rm2",
        "11 active observation(s) analyzed by mixed-effects REML (no listwise subject deletion). 1 non-numeric cell(s) ignored.",
        "res-rm2",
    );
}

#[test]
fn multifactor_anova_reports_exact_listwise_observation_accounting() {
    // Multifactor ANOVA on MultipleVariablesTable with 1 missing factor, 1 excluded response, 1 non-numeric response.
    let mut mv = MultipleVariablesTable::new(3, 15).unwrap();
    mv.variables[0].title = String::from("Dose");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Sex");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[2].title = String::from("Outcome");
    mv.variables[2].lock_role(VariableRole::Continuous);
    let complete_mv = [
        ("Low", "F", 10.0),
        ("Low", "F", 11.0),
        ("Low", "F", 12.0),
        ("Low", "M", 13.0),
        ("Low", "M", 14.0),
        ("Low", "M", 15.0),
        ("High", "F", 20.0),
        ("High", "F", 21.0),
        ("High", "F", 22.0),
        ("High", "M", 25.0),
        ("High", "M", 26.0),
        ("High", "M", 27.0),
    ];
    for (r, (d, s, y)) in complete_mv.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(d);
        mv.variables[1].cells[r] = Cell::text(s);
        mv.variables[2].cells[r] = Cell::number(y);
    }
    // Row 12: missing factor
    mv.variables[0].cells[12] = Cell::text("Low");
    mv.variables[1].cells[12] = Cell::empty();
    mv.variables[2].cells[12] = Cell::number(11.5);
    // Row 13: excluded response
    mv.variables[0].cells[13] = Cell::text("High");
    mv.variables[1].cells[13] = Cell::text("F");
    mv.variables[2].cells[13] = Cell::excluded_number(99.0);
    // Row 14: non-numeric response
    mv.variables[0].cells[14] = Cell::text("High");
    mv.variables[1].cells[14] = Cell::text("M");
    mv.variables[2].cells[14] = Cell::text("invalid");

    let mut mv_session =
        session_with("Multifactor ANOVA", vec![DataSheet::new("mv-fac", "MV Factorial", TableArchetype::MultipleVariables(mv)).unwrap()]);
    run_spec(
        &mut mv_session,
        "stats.run",
        "res-mv-fac",
        "mv-fac",
        AnalysisKind::FactorialAnova(FactorialAnovaSpec {
            response_column: Some(2),
            multifactor_columns: vec![0, 1],
            ..FactorialAnovaSpec::default()
        }),
    );
    let mv_res = rs(&mv_session, "res-mv-fac");
    let mv_acc = mv_res.observation_accounting().unwrap();
    assert_eq!(mv_acc.kind, ObservationAccountingKind::ListwiseAnova);
    assert_eq!(mv_acc.analyzed, 12);
    assert_eq!((mv_acc.omitted_missing, mv_acc.omitted_excluded, mv_acc.omitted_non_numeric), (1, 1, 1));
    assert_eq!(mv_acc.ignored_non_numeric_cells, 1);
    assert_eq!(mv_acc.excluded_cells, 1);
    let mv_chk = mv_res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
    assert_eq!(mv_chk.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 12 retained"));
    let mv_num_chk = mv_res.checklist.iter().find(|c| c.assumption == "Numeric input validation").unwrap();
    assert_eq!(mv_num_chk.diagnostic_metric.as_deref(), Some("1 non-numeric cell(s) ignored"));
    assert_reports_contain(
        &mv_session,
        "res-mv-fac",
        "12 complete row(s)/block(s) analyzed; 3 row(s) omitted by listwise filtering (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell). 1 non-numeric cell(s) ignored.",
        "res-mv-fac",
    );
}

#[test]
fn regression_families_report_exact_observation_accounting() {
    // XY table for OLS linear, Deming, and nonlinear regression (with missing, excluded, non-numeric, and range-filtered rows).
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 10).unwrap();
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Signal");
    let xy_rows = [
        (Cell::number(1.0), Cell::number(3.1)),
        (Cell::number(2.0), Cell::number(5.0)),
        (Cell::number(3.0), Cell::number(7.2)),
        (Cell::number(4.0), Cell::number(8.9)),
        (Cell::number(5.0), Cell::number(11.1)),
        (Cell::number(6.0), Cell::number(13.0)),
        // Out of X range when filtered to [1, 6]
        (Cell::number(20.0), Cell::number(41.0)),
        // Missing Y
        (Cell::number(7.0), Cell::empty()),
        // Excluded Y
        (Cell::number(8.0), Cell::excluded_number(99.0)),
        // Non-numeric Y
        (Cell::number(9.0), Cell::text("n/a")),
    ];
    for (r, (xc, yc)) in xy_rows.into_iter().enumerate() {
        xy.x_cells[r][0] = xc;
        xy.data_sets[0].cells[r][0] = yc;
    }
    let mut session = session_with("Regression accounting", vec![DataSheet::new("xy-reg", "XY Reg", TableArchetype::Xy(xy)).unwrap()]);

    let mut nl_spec = NonlinearFitSpec::new("straight_line");
    nl_spec.x_range_filter = Some((Some(1.0), Some(6.0)));
    for (id, kind) in [
        (
            "res-lin",
            AnalysisKind::LinearRegression(LinearRegressionSpec { x_range_filter: Some((Some(1.0), Some(6.0))), ..LinearRegressionSpec::default() }),
        ),
        (
            "res-deming",
            AnalysisKind::LinearRegression(LinearRegressionSpec {
                deming_model_ii: true,
                x_range_filter: Some((Some(1.0), Some(6.0))),
                ..LinearRegressionSpec::default()
            }),
        ),
        ("res-nl", AnalysisKind::NonlinearRegression(nl_spec)),
    ] {
        run_spec(&mut session, "fit.run", id, "xy-reg", kind);
        let res = rs(&session, id);
        let acc = res.observation_accounting().unwrap_or_else(|| panic!("{id}: missing observation_accounting"));
        assert_eq!(acc.kind, ObservationAccountingKind::Regression, "{id}");
        assert_eq!(acc.analyzed, 6, "{id}");
        assert_eq!((acc.omitted_missing, acc.omitted_excluded, acc.omitted_non_numeric, acc.omitted_out_of_range), (1, 1, 1, 1), "{id}");
        assert_eq!(acc.rows_omitted(), 4, "{id}");
        assert_eq!(acc.ignored_non_numeric_cells, 1, "{id}");
        assert_eq!(acc.excluded_cells, 1, "{id}");
        let chk = res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
        assert_eq!(chk.diagnostic_metric.as_deref(), Some("4 row(s) omitted; 6 retained"), "{id}");
        let num_chk = res.checklist.iter().find(|c| c.assumption == "Numeric input validation").unwrap();
        assert_eq!(num_chk.diagnostic_metric.as_deref(), Some("1 non-numeric cell(s) ignored"), "{id}");
        assert_reports_contain(
            &session,
            id,
            "6 observation(s) analyzed; 4 row(s)/observation(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell, 1 outside configured X/Y range bounds). 1 non-numeric cell(s) ignored.",
            id,
        );
    }

    // Passing-Bablok on XY table (same rows without range filter -> 7 analyzed, 3 omitted: missing, excluded, non-numeric).
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy-reg", "sheet_id": "res-pb", "kind": "passing_bablok" }));
    let pb_acc = rs(&session, "res-pb").observation_accounting().unwrap();
    assert_eq!(pb_acc.kind, ObservationAccountingKind::Regression);
    assert_eq!(pb_acc.analyzed, 7);
    assert_eq!((pb_acc.omitted_missing, pb_acc.omitted_excluded, pb_acc.omitted_non_numeric), (1, 1, 1));
    assert_reports_contain(
        &session,
        "res-pb",
        "7 observation(s) analyzed; 3 row(s)/observation(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell). 1 non-numeric cell(s) ignored.",
        "res-pb",
    );

    // Simple logistic regression on binary XY table with excluded Y, non-numeric Y, and missing X.
    let mut sl_xy = XyTable::new(SubcolumnMode::Single, 1, 13).unwrap();
    sl_xy.x_title = String::from("Dose");
    sl_xy.data_sets[0].title = String::from("Response");
    for (r, (x, y)) in [(1.0, 0.0), (2.0, 0.0), (3.0, 0.0), (4.0, 0.0), (5.0, 1.0), (6.0, 0.0), (7.0, 1.0), (8.0, 1.0), (9.0, 1.0), (10.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        sl_xy.x_cells[r][0] = Cell::number(x);
        sl_xy.data_sets[0].cells[r][0] = Cell::number(y);
    }
    sl_xy.x_cells[10][0] = Cell::empty();
    sl_xy.data_sets[0].cells[10][0] = Cell::number(1.0);
    sl_xy.x_cells[11][0] = Cell::number(11.0);
    sl_xy.data_sets[0].cells[11][0] = Cell::excluded_number(1.0);
    sl_xy.x_cells[12][0] = Cell::number(12.0);
    sl_xy.data_sets[0].cells[12][0] = Cell::text("err");
    let mut sl_session = session_with("Simple Logistic", vec![DataSheet::new("xy-sl", "XY SL", TableArchetype::Xy(sl_xy)).unwrap()]);
    ok_cmd(&mut sl_session, "stats.run", json!({ "source_sheet_id": "xy-sl", "sheet_id": "res-sl", "kind": "simple_logistic" }));
    let sl_acc = rs(&sl_session, "res-sl").observation_accounting().unwrap();
    assert_eq!(sl_acc.kind, ObservationAccountingKind::Regression);
    assert_eq!(sl_acc.analyzed, 10);
    assert_eq!((sl_acc.omitted_missing, sl_acc.omitted_excluded, sl_acc.omitted_non_numeric), (1, 1, 1));
    assert_eq!(sl_acc.ignored_non_numeric_cells, 1);
    assert_eq!(sl_acc.excluded_cells, 1);
    assert_reports_contain(
        &sl_session,
        "res-sl",
        "10 observation(s) analyzed; 3 row(s)/observation(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell). 1 non-numeric cell(s) ignored.",
        "res-sl",
    );

    // GLM (Multiple Linear, Logistic, Poisson) on MV table with missing, excluded, non-numeric rows.
    let mut mv = MultipleVariablesTable::new(4, 13).unwrap();
    mv.variables[0].title = String::from("X");
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].title = String::from("Y_lin");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("Y_bin");
    mv.variables[2].lock_role(VariableRole::Continuous);
    mv.variables[3].title = String::from("Y_cnt");
    mv.variables[3].lock_role(VariableRole::Continuous);
    let glm_rows = [
        (1.0, 2.1, 0.0, 1.0),
        (2.0, 4.0, 0.0, 2.0),
        (3.0, 6.2, 0.0, 3.0),
        (4.0, 7.9, 0.0, 4.0),
        (5.0, 10.1, 1.0, 6.0),
        (6.0, 12.0, 0.0, 7.0),
        (7.0, 14.2, 1.0, 10.0),
        (8.0, 16.0, 1.0, 13.0),
        (9.0, 18.1, 1.0, 16.0),
        (10.0, 20.0, 1.0, 21.0),
    ];
    for (r, (x, yl, yb, yc)) in glm_rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(x);
        mv.variables[1].cells[r] = Cell::number(yl);
        mv.variables[2].cells[r] = Cell::number(yb);
        mv.variables[3].cells[r] = Cell::number(yc);
    }
    // Row 10: missing X
    mv.variables[0].cells[10] = Cell::empty();
    mv.variables[1].cells[10] = Cell::number(5.0);
    mv.variables[2].cells[10] = Cell::number(1.0);
    mv.variables[3].cells[10] = Cell::number(5.0);
    // Row 11: excluded X
    mv.variables[0].cells[11] = Cell::excluded_number(11.0);
    mv.variables[1].cells[11] = Cell::number(22.0);
    mv.variables[2].cells[11] = Cell::number(1.0);
    mv.variables[3].cells[11] = Cell::number(25.0);
    // Row 12: non-numeric X
    mv.variables[0].cells[12] = Cell::text("bad_x");
    mv.variables[1].cells[12] = Cell::number(24.0);
    mv.variables[2].cells[12] = Cell::number(1.0);
    mv.variables[3].cells[12] = Cell::number(28.0);

    let mut glm_session = session_with("GLM accounting", vec![DataSheet::new("mv-glm", "MV GLM", TableArchetype::MultipleVariables(mv)).unwrap()]);
    for (id, family, y_col) in
        [("res-glm-lin", GlmFamily::Linear, 1), ("res-glm-log", GlmFamily::Logistic, 2), ("res-glm-pois", GlmFamily::Poisson, 3)]
    {
        run_spec(
            &mut glm_session,
            "stats.run",
            id,
            "mv-glm",
            AnalysisKind::GeneralizedLinearModel(GlmSpec { family, response_column: y_col, predictor_columns: vec![0], ..GlmSpec::default() }),
        );
        let res = rs(&glm_session, id);
        let acc = res.observation_accounting().unwrap_or_else(|| panic!("{id}: missing observation_accounting"));
        assert_eq!(acc.kind, ObservationAccountingKind::Regression, "{id}");
        assert_eq!(acc.analyzed, 10, "{id}");
        assert_eq!((acc.omitted_missing, acc.omitted_excluded, acc.omitted_non_numeric), (1, 1, 1), "{id}");
        assert_eq!(acc.ignored_non_numeric_cells, 1, "{id}");
        assert_eq!(acc.excluded_cells, 1, "{id}");
        let chk = res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
        assert_eq!(chk.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 10 retained"), "{id}");
        assert_reports_contain(
            &glm_session,
            id,
            "10 observation(s) analyzed; 3 row(s)/observation(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell). 1 non-numeric cell(s) ignored.",
            id,
        );
    }
}

#[test]
fn survival_analyses_report_subjects_events_censored_and_omitted_rows() {
    // 1. Kaplan-Meier on SurvivalTable with excluded cell, missing time, and excluded/invalid status code.
    let mut st = SurvivalTable::new(2, 11).unwrap();
    st.time_title = String::from("Days");
    st.cohorts[0].title = String::from("Placebo");
    st.cohorts[1].title = String::from("Drug");
    let st_rows: [(Cell, Cell, Cell); 11] = [
        (Cell::number(5.0), Cell::number(1.0), Cell::empty()),
        (Cell::number(10.0), Cell::number(1.0), Cell::empty()),
        (Cell::number(12.0), Cell::number(0.0), Cell::empty()),
        (Cell::number(18.0), Cell::number(1.0), Cell::empty()),
        (Cell::number(8.0), Cell::empty(), Cell::number(1.0)),
        (Cell::number(15.0), Cell::empty(), Cell::number(0.0)),
        (Cell::number(22.0), Cell::empty(), Cell::number(1.0)),
        (Cell::number(30.0), Cell::empty(), Cell::number(0.0)),
        // Row 8: excluded status cell
        (Cell::number(25.0), Cell::excluded_number(1.0), Cell::empty()),
        // Row 9: missing time cell
        (Cell::empty(), Cell::empty(), Cell::number(1.0)),
        // Row 10: unrecognized status code excluded by policy
        (Cell::number(28.0), Cell::number(9.0), Cell::empty()),
    ];
    for (r, (t, c0, c1)) in st_rows.into_iter().enumerate() {
        st.times[r] = t;
        st.cohorts[0].cells[r][0] = c0;
        st.cohorts[1].cells[r][0] = c1;
    }
    let mut km_session = session_with("KM Survival", vec![DataSheet::new("surv-1", "Survival Table", TableArchetype::Survival(st)).unwrap()]);
    run_spec(
        &mut km_session,
        "stats.run",
        "res-km-table",
        "surv-1",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: curvebook_doc::MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
            event_code: Some(1.0),
            censored_code: Some(0.0),
            treat_other_as: SurvivalOtherValuePolicy::Exclude,
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        }),
    );
    let km_res = rs(&km_session, "res-km-table");
    let km_acc = km_res.observation_accounting().unwrap();
    assert_eq!(km_acc.kind, ObservationAccountingKind::Survival);
    assert_eq!(km_acc.analyzed, 8);
    assert_eq!((km_acc.events, km_acc.censored), (Some(5), Some(3)));
    assert_eq!((km_acc.omitted_missing, km_acc.omitted_excluded, km_acc.omitted_invalid), (1, 1, 1));
    assert_eq!(km_acc.excluded_cells, 1);
    let km_chk = km_res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
    assert_eq!(km_chk.diagnostic_metric.as_deref(), Some("3 row(s) omitted; 8 retained"));
    assert_reports_contain(
        &km_session,
        "res-km-table",
        "8 subject(s) analyzed (5 event(s), 3 censored); 3 row(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 with an excluded or invalid domain value).",
        "res-km-table",
    );

    // 2. Kaplan-Meier & Cox PH on MultipleVariablesTable with events, censored, missing, excluded, non-numeric, and invalid status rows.
    let mut mv = MultipleVariablesTable::new(4, 14).unwrap();
    mv.variables[0].title = String::from("Time");
    mv.variables[0].lock_role(VariableRole::Continuous);
    mv.variables[1].title = String::from("Status");
    mv.variables[1].lock_role(VariableRole::Continuous);
    mv.variables[2].title = String::from("Arm");
    mv.variables[2].lock_role(VariableRole::Categorical);
    mv.variables[3].title = String::from("Age");
    mv.variables[3].lock_role(VariableRole::Continuous);
    let surv_mv_rows = [
        (4.0, 1.0, "Ctrl", 65.0),
        (7.0, 1.0, "Ctrl", 70.0),
        (10.0, 0.0, "Ctrl", 58.0),
        (12.0, 1.0, "Ctrl", 63.0),
        (15.0, 0.0, "Ctrl", 55.0),
        (9.0, 1.0, "Drug", 61.0),
        (14.0, 1.0, "Drug", 66.0),
        (18.0, 0.0, "Drug", 52.0),
        (22.0, 1.0, "Drug", 60.0),
        (26.0, 0.0, "Drug", 49.0),
    ];
    for (r, (t, s, arm, age)) in surv_mv_rows.into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::number(t);
        mv.variables[1].cells[r] = Cell::number(s);
        mv.variables[2].cells[r] = Cell::text(arm);
        mv.variables[3].cells[r] = Cell::number(age);
    }
    // Row 10: missing time
    mv.variables[0].cells[10] = Cell::empty();
    mv.variables[1].cells[10] = Cell::number(1.0);
    mv.variables[2].cells[10] = Cell::text("Ctrl");
    mv.variables[3].cells[10] = Cell::number(64.0);
    // Row 11: excluded time
    mv.variables[0].cells[11] = Cell::excluded_number(20.0);
    mv.variables[1].cells[11] = Cell::number(1.0);
    mv.variables[2].cells[11] = Cell::text("Drug");
    mv.variables[3].cells[11] = Cell::number(62.0);
    // Row 12: non-numeric status
    mv.variables[0].cells[12] = Cell::number(11.0);
    mv.variables[1].cells[12] = Cell::text("lost");
    mv.variables[2].cells[12] = Cell::text("Ctrl");
    mv.variables[3].cells[12] = Cell::number(59.0);
    // Row 13: excluded/invalid status code (9)
    mv.variables[0].cells[13] = Cell::number(16.0);
    mv.variables[1].cells[13] = Cell::number(9.0);
    mv.variables[2].cells[13] = Cell::text("Drug");
    mv.variables[3].cells[13] = Cell::number(57.0);

    let mut mv_surv = session_with("MV Survival", vec![DataSheet::new("mv-surv", "MV Survival", TableArchetype::MultipleVariables(mv)).unwrap()]);

    run_spec(
        &mut mv_surv,
        "stats.run",
        "res-km-mv",
        "mv-surv",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: curvebook_doc::MultiplicitySpec::default(),
            mv_time_column: Some(0),
            mv_status_column: Some(1),
            mv_group_column: Some(2),
            event_code: Some(1.0),
            censored_code: Some(0.0),
            treat_other_as: SurvivalOtherValuePolicy::Exclude,
            ratio_median_ci: false,
            mv_status_columns: Vec::new(),
            mv_group_columns: Vec::new(),
        }),
    );
    run_spec(
        &mut mv_surv,
        "stats.run",
        "res-cox",
        "mv-surv",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 0,
            status_column: 1,
            predictor_columns: vec![3],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
            formula: None,
            covariate_profiles: Vec::new(),
            nested_comparison: None,
            emit_cox_snell_residuals: false,
            ci_method: RegressionCiMethod::default(),
            event_code: Some(1.0),
            censored_code: Some(0.0),
            treat_other_as: SurvivalOtherValuePolicy::Exclude,
        }),
    );
    for id in ["res-km-mv", "res-cox"] {
        let res = rs(&mv_surv, id);
        let acc = res.observation_accounting().unwrap();
        assert_eq!(acc.kind, ObservationAccountingKind::Survival, "{id}");
        assert_eq!(acc.analyzed, 10, "{id}");
        assert_eq!((acc.events, acc.censored), (Some(6), Some(4)), "{id}");
        assert_eq!((acc.omitted_missing, acc.omitted_excluded, acc.omitted_non_numeric, acc.omitted_invalid), (1, 1, 1, 1), "{id}");
        assert_eq!(acc.ignored_non_numeric_cells, 1, "{id}");
        assert_eq!(acc.excluded_cells, 1, "{id}");
        let chk = res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
        assert_eq!(chk.diagnostic_metric.as_deref(), Some("4 row(s) omitted; 10 retained"), "{id}");
        assert_reports_contain(
            &mv_surv,
            id,
            "10 subject(s) analyzed (6 event(s), 4 censored); 4 row(s) omitted (1 missing a required value, 1 containing an excluded cell, 1 containing a non-numeric cell, 1 with an excluded or invalid domain value). 1 non-numeric cell(s) ignored.",
            id,
        );
    }

    // Stale (Dirty) survival result suppresses observation accounting in report.
    let mut dirty_doc = mv_surv.document().clone();
    dirty_doc.result_sheet_mut(&SheetId::new("res-cox")).unwrap().status = ResultStatus::Dirty;
    let dirty_report = generate_methods_report(&dirty_doc, &SheetId::new("res-cox"), ReportFormat::PlainText).unwrap();
    assert!(!dirty_report.contains("Survival observation accounting"), "{dirty_report}");
}

#[test]
fn unpaired_analyses_report_ignored_non_numeric_and_excluded_cells() {
    let mut sheet = col_sheet(
        "unpaired-multi",
        "Unpaired Multi",
        &[("Ctrl", &[10.0, 11.0, 12.0, 13.0, 14.0]), ("Low", &[15.0, 16.0, 17.0, 18.0, 19.0]), ("High", &[22.0, 23.0, 24.0, 25.0, 26.0])],
    );
    if let TableArchetype::Column(col) = &mut sheet.archetype {
        col.data_sets[0].cells[4][0] = Cell::text("n/a");
        col.data_sets[1].cells[4][0] = Cell::text("missing_lab");
        col.data_sets[1].cells[3][0] = Cell::excluded_number(99.0);
    }
    let mut session = session_with("Unpaired accounting", vec![sheet]);

    // Two-group unpaired analyses on first two columns: 4 + 3 = 7 active numeric obs, 2 non-numeric, 1 excluded.
    for kind in ["unpaired_t", "welch_t", "mann_whitney", "kolmogorov_smirnov"] {
        let id = format!("res-{kind}");
        ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "unpaired-multi", "sheet_id": id, "kind": kind, "selected_data_sets": [0, 1] }));
        let res = rs(&session, &id);
        assert_eq!(res.paired_observation_accounting(), None, "{kind}");
        let acc = res.observation_accounting().unwrap();
        assert_eq!(acc.kind, ObservationAccountingKind::Unpaired, "{kind}");
        assert_eq!(acc.analyzed, 7, "{kind}");
        assert_eq!(acc.ignored_non_numeric_cells, 2, "{kind}");
        assert_eq!(acc.excluded_cells, 1, "{kind}");
        let chk = res.checklist.iter().find(|c| c.assumption == "Numeric input validation").unwrap();
        assert_eq!(chk.diagnostic_metric.as_deref(), Some("2 non-numeric cell(s) ignored"), "{kind}");
        assert_reports_contain(&session, &id, "7 active numeric observation(s) analyzed; 2 non-numeric cell(s) ignored.", kind);
    }

    // Multi-group unpaired analyses across all 3 columns: 4 + 3 + 5 = 12 active numeric obs, 2 non-numeric, 1 excluded.
    for kind in ["one_way_anova", "welch_anova", "kruskal_wallis", "descriptive"] {
        let id = format!("res-{kind}");
        ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "unpaired-multi", "sheet_id": id, "kind": kind }));
        let res = rs(&session, &id);
        assert_eq!(res.paired_observation_accounting(), None, "{kind}");
        let acc = res.observation_accounting().unwrap();
        assert_eq!(acc.kind, ObservationAccountingKind::Unpaired, "{kind}");
        assert_eq!(acc.analyzed, 12, "{kind}");
        assert_eq!(acc.ignored_non_numeric_cells, 2, "{kind}");
        assert_eq!(acc.excluded_cells, 1, "{kind}");
        let chk = res.checklist.iter().find(|c| c.assumption == "Numeric input validation").unwrap();
        assert_eq!(chk.diagnostic_metric.as_deref(), Some("2 non-numeric cell(s) ignored"), "{kind}");
        assert_reports_contain(&session, &id, "12 active numeric observation(s) analyzed; 2 non-numeric cell(s) ignored.", kind);
    }
}

#[test]
fn simple_logistic_multi_y_avoids_shared_x_double_counting_and_syncs_x_checklist() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 13).unwrap();
    xy.x_title = String::from("Dose");
    xy.data_sets[0].title = String::from("ArmA");
    xy.data_sets[1].title = String::from("ArmB");
    for (r, (x, ya, yb)) in [
        (1.0, 0.0, 0.0),
        (2.0, 0.0, 0.0),
        (3.0, 0.0, 0.0),
        (4.0, 0.0, 0.0),
        (5.0, 1.0, 0.0),
        (6.0, 0.0, 1.0),
        (7.0, 1.0, 1.0),
        (8.0, 1.0, 1.0),
        (9.0, 1.0, 1.0),
        (10.0, 1.0, 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        xy.x_cells[r][0] = Cell::number(x);
        xy.data_sets[0].cells[r][0] = Cell::number(ya);
        xy.data_sets[1].cells[r][0] = Cell::number(yb);
    }
    // Row 10: shared X masked as excluded (must count once in excluded_cells, omit 1 observation per Y arm).
    xy.x_cells[10][0] = Cell::excluded_number(11.0);
    xy.data_sets[0].cells[10][0] = Cell::number(1.0);
    xy.data_sets[1].cells[10][0] = Cell::number(0.0);
    // Row 11: shared X non-numeric (must count once in ignored_non_numeric_cells and appear in checklist, omit 1 obs per Y arm).
    xy.x_cells[11][0] = Cell::text("bad_x");
    xy.data_sets[0].cells[11][0] = Cell::number(1.0);
    xy.data_sets[1].cells[11][0] = Cell::number(0.0);
    // Row 12: valid X, non-numeric Y in ArmA, valid Y in ArmB (ArmB analyzed, ArmA omitted).
    xy.x_cells[12][0] = Cell::number(12.0);
    xy.data_sets[0].cells[12][0] = Cell::text("bad_y");
    xy.data_sets[1].cells[12][0] = Cell::number(1.0);

    let mut session = session_with("Multi-Y Simple Logistic", vec![DataSheet::new("xy-sl-multi", "XY SL Multi", TableArchetype::Xy(xy)).unwrap()]);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy-sl-multi", "sheet_id": "res-sl-multi", "kind": "simple_logistic" }));
    let res = rs(&session, "res-sl-multi");
    let acc = res.observation_accounting().unwrap();
    assert_eq!(acc.kind, ObservationAccountingKind::Regression);
    assert_eq!(acc.analyzed, 21);
    assert_eq!((acc.omitted_excluded, acc.omitted_non_numeric, acc.omitted_missing), (2, 3, 0));
    assert_eq!(acc.excluded_cells, 1);
    assert_eq!(acc.ignored_non_numeric_cells, 2);

    let align_chk = res.checklist.iter().find(|c| c.assumption == "Complete-case row alignment").unwrap();
    assert_eq!(align_chk.diagnostic_metric.as_deref(), Some("5 row(s) omitted; 21 retained"));
    let num_chk = res.checklist.iter().find(|c| c.assumption == "Numeric input validation").unwrap();
    assert_eq!(num_chk.diagnostic_metric.as_deref(), Some("2 non-numeric cell(s) ignored"));
    assert!(num_chk.explanation.contains("Dose: 1") && num_chk.explanation.contains("ArmA: 1"), "unexpected explanation: {}", num_chk.explanation);

    assert_reports_contain(
        &session,
        "res-sl-multi",
        "21 observation(s) analyzed; 5 row(s)/observation(s) omitted (2 containing an excluded cell, 3 containing a non-numeric cell). 2 non-numeric cell(s) ignored.",
        "res-sl-multi",
    );
}

#[test]
fn two_group_unpaired_ignores_non_numeric_in_unanalyzed_third_column() {
    let mut sheet = col_sheet(
        "three-col-unpaired",
        "Three Col Unpaired",
        &[("GroupA", &[10.0, 11.0, 12.0, 13.0]), ("GroupB", &[15.0, 16.0, 17.0, 18.0]), ("UnusedC", &[20.0, 21.0, 22.0, 23.0])],
    );
    if let TableArchetype::Column(col) = &mut sheet.archetype {
        col.data_sets[2].cells[3][0] = Cell::text("ignored_in_col3");
    }
    let mut session = session_with("Two-Group 3-Col Scope", vec![sheet]);

    for kind in ["unpaired_t", "welch_t", "lognormal_unpaired_t", "lognormal_welch_t", "mann_whitney", "kolmogorov_smirnov"] {
        let id = format!("res-3col-{kind}");
        ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "three-col-unpaired", "sheet_id": id, "kind": kind }));
        let res = rs(&session, &id);
        let acc = res.observation_accounting().unwrap();
        assert_eq!(acc.kind, ObservationAccountingKind::Unpaired, "{kind}");
        assert_eq!(acc.analyzed, 8, "{kind}");
        assert_eq!(acc.ignored_non_numeric_cells, 0, "{kind}");
        assert!(
            res.checklist.iter().all(|c| c.assumption != "Numeric input validation"),
            "{kind}: unanalyzed 3rd column must not trigger Numeric input validation checklist item"
        );
        assert_reports_contain(&session, &id, "8 active numeric observation(s) analyzed; 0 non-numeric cells ignored.", kind);
    }
}
