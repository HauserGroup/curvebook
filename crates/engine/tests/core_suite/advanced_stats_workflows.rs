#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

//! End-to-end engine workflow tests for one-way MANOVA, competing-risks cumulative incidence
//! (Aalen–Johansen CIF + Gray's test), Scheffé post-hoc comparisons, stratified 2×2×K
//! Cochran–Mantel–Haenszel analysis, and dual-axis Gardner–Altman estimation plots.

use curvebook_doc::{
    AnalysisKind, Cell, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DiagnosticPlotKind, GraphKind, MultipleVariablesTable,
    MultiplicityMethod, ResultStatus, SheetId, SurvivalTable, TableArchetype, VariableRole,
};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_testkit::assert_approx_eq;
use serde_json::json;

use super::common::{col_sheet, ok_cmd, rs, scalar_ds, session_with};

#[test]
fn manova_competing_risks_cmh_scheffe_and_estimation_plot_end_to_end() {
    // 1. MultipleVariables table for MANOVA (3 groups x 3 responses).
    let groups = ["A", "A", "A", "A", "A", "B", "B", "B", "B", "B", "C", "C", "C", "C", "C"];
    let y1 = [5.1, 4.9, 4.7, 5.4, 5.0, 6.4, 6.9, 6.5, 6.2, 6.8, 7.1, 7.6, 7.3, 6.9, 7.4];
    let y2 = [3.5, 3.0, 3.2, 3.9, 3.6, 3.2, 3.1, 2.8, 2.9, 3.0, 3.0, 3.4, 2.9, 3.1, 3.2];
    let y3 = [1.4, 1.4, 1.3, 1.7, 1.5, 4.5, 4.9, 4.6, 4.3, 4.8, 5.9, 6.6, 6.3, 5.1, 6.1];
    let mut mv = MultipleVariablesTable::new(4, 15).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].lock_role(VariableRole::Categorical);
    mv.variables[1].title = String::from("Y1");
    mv.variables[2].title = String::from("Y2");
    mv.variables[3].title = String::from("Y3");
    for r in 0..15 {
        mv.variables[0].cells[r] = Cell::text(groups[r]);
        mv.variables[1].cells[r] = Cell::number(y1[r]);
        mv.variables[2].cells[r] = Cell::number(y2[r]);
        mv.variables[3].cells[r] = Cell::number(y3[r]);
    }
    let ds_manova = DataSheet::new(SheetId::new("mv-manova"), "MANOVA Data", TableArchetype::MultipleVariables(mv)).unwrap();

    // 2. Survival table for Competing Risks (0=censored, 1=primary, 2=competing).
    let mut surv = SurvivalTable::new(2, 8).unwrap();
    surv.cohorts[0].title = String::from("ArmA");
    surv.cohorts[1].title = String::from("ArmB");
    let times = [1., 2., 2., 3., 4., 5., 6., 8.];
    let ca = [1., 1., 2., 0., 1., 2., 1., 0.];
    let cb = [2., 2., 1., 0., 2., 1., 2., 0.];
    for r in 0..8 {
        surv.times[r] = Cell::number(times[r]);
        surv.cohorts[0].cells[r][0] = Cell::number(ca[r]);
        surv.cohorts[1].cells[r][0] = Cell::number(cb[r]);
    }
    let ds_cr = DataSheet::new(SheetId::new("surv-cr"), "Competing Risks", TableArchetype::Survival(surv)).unwrap();

    // 3. Contingency table (K x 4) for Stratified 2x2xK CMH.
    let mut ct = ContingencyTable::new(4, 3).unwrap();
    ct.row_titles = vec!["Center 1".into(), "Center 2".into(), "Center 3".into()];
    let strata_rows = [[10., 15., 20., 40.], [18., 10., 12., 25.], [8., 12., 14., 30.]];
    for (r, vals) in strata_rows.iter().enumerate() {
        for (c, &v) in vals.iter().enumerate() {
            ct.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    let ds_cmh = DataSheet::new(SheetId::new("ct-cmh"), "CMH Strata", TableArchetype::Contingency(ct)).unwrap();

    // 4. Column table for Scheffé ANOVA + Estimation Plot.
    let ds_col = col_sheet(
        "col-groups",
        "Groups",
        &[
            ("G1", &[10.0, 11.0, 12.0, 10.5, 11.5]),
            ("G2", &[12.0, 13.0, 12.5, 13.5]),
            ("G3", &[15.0, 16.0, 14.5, 15.5, 16.5, 15.0]),
            ("G4", &[20.0, 19.0, 21.0, 20.5, 19.5]),
        ],
    );

    let mut pair_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    pair_col.data_sets[0].title = String::from("Ctrl");
    pair_col.data_sets[1].title = String::from("Drug");
    for (r, (a, b)) in [(10.0, 14.0), (11.0, 15.0), (12.0, 16.0), (10.5, 14.5), (11.5, 15.5)].into_iter().enumerate() {
        pair_col.data_sets[0].cells[r][0] = Cell::number(a);
        pair_col.data_sets[1].cells[r][0] = Cell::number(b);
    }
    let ds_est = DataSheet::new(SheetId::new("col-est"), "Est Data", TableArchetype::Column(pair_col)).unwrap();

    let mut s = session_with("Advanced Stats", vec![ds_manova, ds_cr, ds_cmh, ds_col, ds_est]);

    // --- MANOVA via stats.run + updateParams + methods report ---
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-manova", "source_sheet_id": "mv-manova", "kind": "manova" }));
    assert_eq!(rs(&s, "res-manova").status, ResultStatus::UpToDate);
    assert!(matches!(rs(&s, "res-manova").analysis.kind, AnalysisKind::Manova(_)));
    assert_approx_eq(scalar_ds(&s, "res-manova", "Wilks' Lambda", None).unwrap(), 0.018_586_984_644, 1e-10, 1e-10, "Wilks");
    assert_approx_eq(scalar_ds(&s, "res-manova", "Pillai's trace", None).unwrap(), 1.184_770_310_520, 1e-10, 1e-10, "Pillai");
    ok_cmd(&mut s, "analysis.updateParams", json!({ "sheet_id": "res-manova", "response_columns": [1, 3] }));
    assert_eq!(rs(&s, "res-manova").status, ResultStatus::UpToDate);
    let manova_rpt = generate_methods_report(s.document(), &SheetId::new("res-manova"), ReportFormat::Markdown).unwrap();
    assert!(manova_rpt.contains("Pillai") && manova_rpt.contains("Wilks") && manova_rpt.contains("Rencher 2002"));

    // --- Competing Risks via stats.run + updateParams + methods report ---
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-cr", "source_sheet_id": "surv-cr", "kind": "competing_risks" }));
    assert_eq!(rs(&s, "res-cr").status, ResultStatus::UpToDate);
    assert!(matches!(rs(&s, "res-cr").analysis.kind, AnalysisKind::CompetingRisks(_)));
    ok_cmd(&mut s, "analysis.updateParams", json!({ "sheet_id": "res-cr", "gray_rho": 1.0, "log_log_ci": false }));
    assert_eq!(rs(&s, "res-cr").status, ResultStatus::UpToDate);
    let cr_rpt = generate_methods_report(s.document(), &SheetId::new("res-cr"), ReportFormat::Markdown).unwrap();
    assert!(cr_rpt.contains("Aalen-Johansen") && cr_rpt.contains("Gray 1988"));

    // --- CMH via stats.run + updateParams + methods report ---
    ok_cmd(&mut s, "stats.run", json!({ "sheet_id": "res-cmh", "source_sheet_id": "ct-cmh", "kind": "cmh" }));
    assert_eq!(rs(&s, "res-cmh").status, ResultStatus::UpToDate);
    assert_approx_eq(scalar_ds(&s, "res-cmh", "Common OR (Mantel-Haenszel)", None).unwrap(), 1.922_234_004_949, 1e-10, 1e-10, "CMH OR");
    assert_approx_eq(scalar_ds(&s, "res-cmh", "CMH chi-square (continuity-corrected)", None).unwrap(), 4.204_763_118_628, 1e-10, 1e-10, "CMH cc");
    ok_cmd(&mut s, "analysis.updateParams", json!({ "sheet_id": "res-cmh", "continuity_correction": false }));
    assert_eq!(rs(&s, "res-cmh").status, ResultStatus::UpToDate);
    let cmh_rpt = generate_methods_report(s.document(), &SheetId::new("res-cmh"), ReportFormat::Markdown).unwrap();
    assert!(cmh_rpt.contains("Mantel & Haenszel 1959") && cmh_rpt.contains("Tarone 1985"));

    // --- One-Way ANOVA with Scheffé post-hoc ---
    ok_cmd(
        &mut s,
        "stats.run",
        json!({ "sheet_id": "res-scheffe", "source_sheet_id": "col-groups", "kind": "one_way_anova", "multiplicity": "scheffe" }),
    );
    assert!(matches!(
        rs(&s, "res-scheffe").analysis.kind,
        AnalysisKind::OneWayAnova(ref a) if a.multiplicity.method == MultiplicityMethod::Scheffe
    ));
    assert_approx_eq(rs(&s, "res-scheffe").pairwise_comparisons[0].raw_p_or_q, 2.550_570_644_870_986e-2, 1e-10, 1e-10, "Scheffe G1-G2 p");
    let scheffe_rpt = generate_methods_report(s.document(), &SheetId::new("res-scheffe"), ReportFormat::Markdown).unwrap();
    assert!(scheffe_rpt.contains("Scheff") && scheffe_rpt.contains("1953"));

    // Welch/Brown-Forsythe ANOVA + Scheffe reports per-pair Welch t + Scheffe (not Games-Howell) and honors control_column.
    ok_cmd(
        &mut s,
        "stats.run",
        json!({
            "sheet_id": "res-welch-scheffe",
            "source_sheet_id": "col-groups",
            "kind": "welch_anova",
            "multiplicity": "scheffe",
            "control_column": 0
        }),
    );
    assert_eq!(rs(&s, "res-welch-scheffe").pairwise_comparisons.len(), 3);
    let welch_scheffe_rpt = generate_methods_report(s.document(), &SheetId::new("res-welch-scheffe"), ReportFormat::Markdown).unwrap();
    assert!(
        welch_scheffe_rpt.contains("Welch-Satterthwaite")
            && welch_scheffe_rpt.contains("Scheff")
            && welch_scheffe_rpt.contains("Control column for post-hoc comparisons")
            && !welch_scheffe_rpt.contains("Games-Howell"),
        "unexpected Welch Scheffe methods report:\n{welch_scheffe_rpt}"
    );

    // --- Gardner-Altman Estimation Plot via graph.create + graph.setKind + graph.setSpec + preflight ---
    ok_cmd(&mut s, "graph.create", json!({ "sheet_id": "g-est", "data_sheet_id": "col-est", "title": "Estimation" }));
    ok_cmd(&mut s, "graph.setKind", json!({ "sheet_id": "g-est", "option_id": "diag.estimation_plot", "kind": { "Diagnostic": "EstimationPlot" } }));
    assert_eq!(s.document().graph_sheet(&SheetId::new("g-est")).unwrap().spec.kind, GraphKind::Diagnostic(DiagnosticPlotKind::EstimationPlot));
    ok_cmd(&mut s, "graph.setSpec", json!({ "sheet_id": "g-est", "estimation_paired_lines": true }));
    assert!(s.document().graph_sheet(&SheetId::new("g-est")).unwrap().spec.estimation_paired_lines);
    ok_cmd(&mut s, "graph.preflightCheck", json!({ "sheet_id": "g-est" }));
}
