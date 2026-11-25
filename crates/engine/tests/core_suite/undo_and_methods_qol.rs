//! Integration tests for `G12` (read-only dispatch + bounded undo stack) and `QOL-6`
//! (clean-room ICMJE/ARRIVE methods & results reporting generator).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, ColumnDesignHint, ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet,
    FactorialAnovaSpec, FitMethod, LinearRegressionSpec, ModelComparisonSpec, MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec,
    MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod, PValueStackMethod, PValueStackSpec, ParameterRule, ParameterScope,
    PassingBablokSpec, ProjectDocument, ResultStatus, SheetId, SurvivalAnalysisSpec, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole,
    WeightingScheme,
};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{MAX_UNDO_CELL_BUDGET, MAX_UNDO_ENTRIES, Session};
use serde_json::{Value, json};

use super::common::{
    TempDir, cell_num, col_sheet, col_sheet_hint, contingency_sheet, ds, err_cmd, grouped_rep_sheet, mv_sheet, ok_cmd, rs, run_cmds, session_with,
    survival_sheet, xy_rep_sheet, xy_sheet,
};

fn assert_clean_room_prose(report: &str) {
    let lower = report.to_ascii_lowercase();
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!lower.contains(forbidden), "found forbidden brand string `{forbidden}` in report:\n{report}");
    }
}

fn sample_4pl_two_dataset_xy(id: &str, title: &str, relative_spread: bool) -> DataSheet {
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mk = |ec50: f64| -> Vec<[f64; 2]> {
        xs.iter()
            .map(|&x| {
                let y = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((ec50 - x) * 1.1));
                if relative_spread { [y * 0.985, y * 1.015] } else { [y - 0.6, y + 0.6] }
            })
            .collect()
    };
    let (r0, r1) = (mk(-6.5), mk(-5.8));
    let (s0, s1): (Vec<&[f64]>, Vec<&[f64]>) = (r0.iter().map(|a| a.as_slice()).collect(), r1.iter().map(|a| a.as_slice()).collect());
    xy_rep_sheet(id, title, "Log10 [Agonist] (M)", &xs, &[("Wildtype_A", &s0), ("Mutant_B", &s1)])
}

fn sample_mv(id: &str) -> DataSheet {
    let rows = [
        (1.2, 10.5, 24.0, 4.0, 1.0, "Ctrl"),
        (2.1, 14.2, 31.0, 7.0, 1.0, "Ctrl"),
        (3.4, 18.8, 39.0, 10.0, 0.0, "Treat"),
        (4.5, 22.1, 46.0, 13.0, 1.0, "Treat"),
        (5.8, 27.9, 54.0, 18.0, 0.0, "Treat"),
        (6.9, 31.3, 62.0, 22.0, 1.0, "Ctrl"),
        (7.4, 34.0, 67.0, 26.0, 0.0, "Treat"),
        (8.2, 37.5, 73.0, 30.0, 1.0, "Ctrl"),
    ];
    let rts: Vec<String> = (1..=8).map(|i| format!("Pt_{i}")).collect();
    let rt_refs: Vec<&str> = rts.iter().map(String::as_str).collect();
    type MvRow = (f64, f64, f64, f64, f64, &'static str);
    let col = |f: fn(&MvRow) -> f64| rows.iter().map(|r| Cell::number(f(r))).collect::<Vec<_>>();
    let arm: Vec<Cell> = rows.iter().map(|r| Cell::text(r.5)).collect();
    mv_sheet(
        id,
        "Multivariate Cohort",
        &rt_refs,
        &[
            ("Marker_X", None, &col(|r| r.0)),
            ("Marker_Y", None, &col(|r| r.1)),
            ("Marker_Z", None, &col(|r| r.2)),
            ("Time_Months", None, &col(|r| r.3)),
            ("Event_Status", None, &col(|r| r.4)),
            ("Arm", Some(VariableRole::Categorical), &arm),
        ],
    )
}

#[test]
fn g12_read_only_commands_preserve_redo_stack_and_undo_depth() {
    let mut session = session_with("G12 ReadOnly Check", vec![sample_4pl_two_dataset_xy("xy-1", "Dose Assay", false)]);
    run_cmds(
        &mut session,
        &[
            ("sheet.rename", json!({ "sheet_id": "xy-1", "title": "Dose Assay v2" })),
            ("fit.run", json!({ "sheet_id": "fit-1", "source_sheet_id": "xy-1", "kind": "four_parameter_logistic" })),
            ("graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" })),
            ("layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" })),
            ("sheet.rename", json!({ "sheet_id": "xy-1", "title": "Dose Assay v3" })),
        ],
    );
    assert_eq!(session.undo_depth(), 5);
    assert!(session.undo().unwrap());
    assert_eq!((session.undo_depth(), session.redo_depth()), (4, 1));
    assert_eq!(ds(&session, "xy-1").title, "Dose Assay v2");

    let dir = TempDir::new("g12-readonly");
    let ro_calls: Vec<(&str, Value)> = vec![
        ("project.inspect", json!({})),
        ("project.save", json!({ "path": dir.child("ro.curvebook").to_string_lossy() })),
        ("project.saveTemplate", json!({})),
        ("project.listCheckpoints", json!({})),
        ("table.exportCsv", json!({ "sheet_id": "xy-1", "path": dir.child("ro.csv").to_string_lossy() })),
        ("table.exclusionAuditReport", json!({ "sheet_id": "xy-1" })),
        ("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "preview_only": true })),
        ("analysis.exportJson", json!({ "sheet_id": "fit-1", "path": dir.child("ro.json").to_string_lossy() })),
        ("analysis.exportCsv", json!({ "sheet_id": "fit-1" })),
        ("result.generateReport", json!({ "result_sheet_id": "fit-1", "format": "markdown" })),
        ("graph.preflightCheck", json!({ "sheet_id": "g-1" })),
        ("export.svg", json!({ "sheet_id": "g-1" })),
        ("export.pdf", json!({ "sheet_id": "g-1" })),
        ("export.png", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("export.tiff", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("graph.exportSvg", json!({ "sheet_id": "g-1" })),
        ("graph.exportPdf", json!({ "sheet_id": "g-1" })),
        ("graph.exportPng", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("graph.exportEmf", json!({ "sheet_id": "g-1", "path": dir.child("g1.emf").to_string_lossy() })),
        ("layout.exportEmf", json!({ "sheet_id": "lay-1", "path": dir.child("lay1.emf").to_string_lossy() })),
    ];
    for (cmd, params) in ro_calls {
        let out = ok_cmd(&mut session, cmd, params);
        assert!(!out.mutated && session.undo_depth() == 4 && session.redo_depth() == 1, "`{cmd}` broke read-only invariants");
    }

    assert!(session.redo().unwrap());
    assert_eq!((ds(&session, "xy-1").title.as_str(), session.undo_depth(), session.redo_depth()), ("Dose Assay v3", 5, 0));

    assert!(session.undo().unwrap() && session.redo_depth() == 1);
    let conv = ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true }));
    assert!(conv.mutated && session.undo_depth() == 5 && session.redo_depth() == 0);
}

#[test]
fn g12_one_million_cell_read_only_execution_avoids_snapshot_clone() {
    let sheet = DataSheet::new(
        SheetId::new("mv-1m"),
        "One Million Cells",
        TableArchetype::MultipleVariables(MultipleVariablesTable::new(1, 1_000_000).unwrap()),
    )
    .unwrap();
    assert_eq!(sheet.archetype.allocated_cell_slots(), 1_000_000);
    let mut session = session_with("1M Cell Benchmark", vec![sheet]);

    run_cmds(
        &mut session,
        &[
            ("sheet.rename", json!({ "sheet_id": "mv-1m", "title": "1M Step 1" })),
            ("sheet.rename", json!({ "sheet_id": "mv-1m", "title": "1M Step 2" })),
        ],
    );
    assert!(session.undo().unwrap());
    assert_eq!((session.undo_depth(), session.redo_depth(), session.undo_cell_slots()), (1, 1, 1_000_000));

    let start = Instant::now();
    for _ in 0..25 {
        assert!(!ok_cmd(&mut session, "project.inspect", json!({})).mutated);
    }
    assert!(start.elapsed().as_secs_f64() < 2.0, "25 read-only calls on 1M-cell doc took too long");
    assert_eq!((session.undo_depth(), session.redo_depth(), session.undo_cell_slots()), (1, 1, 1_000_000));
}

#[test]
fn g12_undo_stack_bounds_entries_at_64() {
    assert_eq!(MAX_UNDO_ENTRIES, 64);
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 1, "rows": 1 }));
    for step in 1_u32..=80 {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-1", "row": 0, "col": 0, "value": f64::from(step) }));
    }
    assert_eq!(session.undo_depth(), MAX_UNDO_ENTRIES);
    for _ in 0..MAX_UNDO_ENTRIES {
        assert!(session.undo().unwrap());
    }
    assert_eq!((session.undo_depth(), session.redo_depth()), (0, MAX_UNDO_ENTRIES));
    assert!(!session.undo().unwrap());
    assert_eq!(cell_num(&session, "col-1", 0, 0, 0), Some(16.0));
}

#[test]
fn g12_undo_cell_budget_evicts_oldest_and_keeps_oversized_newest() {
    assert_eq!(MAX_UNDO_CELL_BUDGET, 25_000_000);

    // Phase 1: 10M-slot snapshots cap at 2 entries (20M <= 25M), evicting oldest on 3rd edit.
    {
        let sheet = DataSheet::new(
            SheetId::new("mv-10m"),
            "Initial 10M",
            TableArchetype::MultipleVariables(MultipleVariablesTable::new(100, 100_000).unwrap()),
        )
        .unwrap();
        assert_eq!(sheet.archetype.allocated_cell_slots(), 10_000_000);
        let mut session = session_with("Budget Eviction", vec![sheet]);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 1" }));
        assert_eq!((session.undo_depth(), session.undo_cell_slots()), (1, 10_000_000));
        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 2" }));
        assert_eq!((session.undo_depth(), session.undo_cell_slots()), (2, 20_000_000));
        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 3" }));
        assert_eq!((session.undo_depth(), session.undo_cell_slots()), (2, 20_000_000));

        assert!(session.undo().unwrap() && ds(&session, "mv-10m").title == "Rev 2");
        assert!(session.undo().unwrap() && ds(&session, "mv-10m").title == "Rev 1");
        assert!(!session.undo().unwrap());
    }

    // Phase 2: single snapshot > 25M slots keeps only the newest snapshot (`len == 1`).
    {
        let sheet = DataSheet::new(
            SheetId::new("mv-26m"),
            "Oversized Init",
            TableArchetype::MultipleVariables(MultipleVariablesTable::new(260, 100_000).unwrap()),
        )
        .unwrap();
        assert_eq!(sheet.archetype.allocated_cell_slots(), 26_000_000);
        let mut session = session_with("Oversized Newest Retained", vec![sheet]);

        for rev in ["Oversized Rev 1", "Oversized Rev 2"] {
            ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-26m", "title": rev }));
            assert_eq!((session.undo_depth(), session.undo_cell_slots()), (1, 26_000_000));
        }
        assert!(session.undo().unwrap() && ds(&session, "mv-26m").title == "Oversized Rev 1");
        assert!(!session.undo().unwrap());
    }
}

#[test]
fn qol6_format_parsing_and_error_handling() {
    for (name, expected) in [
        ("plain", Some(ReportFormat::PlainText)),
        ("PLAINTEXT", Some(ReportFormat::PlainText)),
        ("text", Some(ReportFormat::PlainText)),
        ("plain_text", Some(ReportFormat::PlainText)),
        ("plain-text", Some(ReportFormat::PlainText)),
        ("markdown", Some(ReportFormat::Markdown)),
        ("md", Some(ReportFormat::Markdown)),
        ("latex", Some(ReportFormat::Latex)),
        ("TeX", Some(ReportFormat::Latex)),
        ("html", None),
    ] {
        assert_eq!(ReportFormat::from_name(name), expected);
    }
    assert_eq!((ReportFormat::PlainText.as_str(), ReportFormat::Markdown.as_str(), ReportFormat::Latex.as_str()), ("plain", "markdown", "latex"));
    let doc = ProjectDocument::new("Empty Doc").unwrap();
    assert!(generate_methods_report(&doc, &SheetId::new(""), ReportFormat::PlainText).is_err());
    assert!(generate_methods_report(&doc, &SheetId::new("missing-result"), ReportFormat::Markdown).is_err());
}

#[test]
fn qol6_t_test_and_anova_reports_all_formats_and_preserve_stored_numbers() {
    let two_col =
        col_sheet("two-col", "Two-Group Study", &[("Control", &[10.1, 11.4, 9.8, 10.7, 11.2]), ("Treatment_Alpha", &[16.2, 17.5, 15.9, 18.1, 16.8])]);
    let rm_col = col_sheet_hint(
        "rm-col",
        "Longitudinal Biomarker",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("Baseline", &[12.0, 13.5, 11.8, 14.1, 12.6]), ("Week_4", &[16.4, 17.8, 15.9, 18.5, 17.0]), ("Week_8", &[21.2, 23.0, 20.5, 24.1, 22.4])],
    );
    let mut session = session_with("Parametric & Multiplicity Study", vec![two_col, rm_col]);

    let welch_spec = AnalysisSpec::new(
        "res-welch",
        "two-col",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..Default::default() }),
    )
    .unwrap();
    let anova_spec = AnalysisSpec::new(
        "res-rm-anova",
        "rm-col",
        AnalysisKind::OneWayAnova(OneWayAnovaSpec {
            method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
            compute_effect_sizes: true,
            multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..Default::default() },
            ..Default::default()
        }),
    )
    .unwrap();
    run_cmds(&mut session, &[("analysis.run", json!({ "spec": welch_spec })), ("analysis.run", json!({ "spec": anova_spec }))]);

    let doc_snap = session.document();
    for res_id in ["res-welch", "res-rm-anova"] {
        let sid = SheetId::new(res_id);
        let original = doc_snap.result_sheet(&sid).unwrap().clone();
        assert_eq!(original.status, ResultStatus::UpToDate);
        let plain = generate_methods_report(doc_snap, &sid, ReportFormat::PlainText).unwrap();
        assert_eq!(plain, generate_methods_report(doc_snap, &sid, ReportFormat::PlainText).unwrap());
        let md = generate_methods_report(doc_snap, &sid, ReportFormat::Markdown).unwrap();
        let tex = generate_methods_report(doc_snap, &sid, ReportFormat::Latex).unwrap();
        for r in [&plain, &md, &tex] {
            assert_clean_room_prose(r);
        }
        assert!(
            plain.contains("Statistical and Mathematical Methods")
                && md.starts_with("# ")
                && md.contains("## Statistical and Mathematical Methods")
                && md.contains("| ")
        );
        assert!(
            tex.contains("\\section*{")
                && tex.contains("\\subsection*{Statistical and Mathematical Methods}")
                && tex.contains("$p < \\alpha$")
                && tex.contains("$p \\ge \\alpha$")
                && tex.contains("\\begin{tabular}")
        );
        assert_eq!(doc_snap.result_sheet(&sid).unwrap(), &original);
    }

    let welch_plain = generate_methods_report(doc_snap, &SheetId::new("res-welch"), ReportFormat::PlainText).unwrap();
    assert!(welch_plain.contains("Welch's unequal-variance t-test") && welch_plain.contains("Cohen's d"));
    let anova_plain = generate_methods_report(doc_snap, &SheetId::new("res-rm-anova"), ReportFormat::PlainText).unwrap();
    assert!(
        anova_plain.contains("repeated-measures one-way ANOVA")
            && anova_plain.contains("Geisser-Greenhouse")
            && anova_plain.contains("Tukey's honestly significant difference")
    );
}

#[test]
fn qol6_survival_km_and_cox_reports() {
    let surv = survival_sheet(
        "surv-1",
        "Survival Cohort",
        &["Placebo", "Active Drug"],
        &[
            (4.0, &[Some(1.0), None]),
            (7.0, &[Some(1.0), None]),
            (10.0, &[Some(0.0), None]),
            (13.0, &[Some(1.0), None]),
            (18.0, &[Some(1.0), None]),
            (6.0, &[None, Some(1.0)]),
            (11.0, &[None, Some(0.0)]),
            (16.0, &[None, Some(1.0)]),
            (22.0, &[None, Some(0.0)]),
            (28.0, &[None, Some(1.0)]),
        ],
    );
    let mut session = session_with("Oncology Time-to-Event Study", vec![surv, sample_mv("mv-1")]);
    let km_spec = AnalysisSpec::new(
        "res-km",
        "surv-1",
        AnalysisKind::Survival(SurvivalAnalysisSpec::KaplanMeier {
            log_rank_mantel_cox: true,
            gehan_breslow_wilcoxon: false,
            trend_test: false,
            asymmetrical_greenwood_ci: true,
            multiplicity: MultiplicitySpec::default(),
            mv_time_column: None,
            mv_status_column: None,
            mv_group_column: None,
        }),
    )
    .unwrap();
    let cox_spec = AnalysisSpec::new(
        "res-cox",
        "mv-1",
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards {
            time_column: 3,
            status_column: 4,
            predictor_columns: vec![0, 1],
            reference_levels: Vec::new(),
            tie_method: CoxTieMethod::Efron,
        }),
    )
    .unwrap();
    run_cmds(&mut session, &[("analysis.run", json!({ "spec": km_spec })), ("analysis.run", json!({ "spec": cox_spec }))]);

    let km_md = generate_methods_report(session.document(), &SheetId::new("res-km"), ReportFormat::Markdown).unwrap();
    assert!(km_md.contains("Kaplan-Meier product-limit") && km_md.contains("Log-rank") && km_md.contains("hazard ratio"));
    assert_clean_room_prose(&km_md);

    let cox_tex = generate_methods_report(session.document(), &SheetId::new("res-cox"), ReportFormat::Latex).unwrap();
    assert!(
        cox_tex.contains("Cox proportional hazards")
            && cox_tex.contains("Efron")
            && cox_tex.contains("Time\\_Months")
            && cox_tex.contains("Event\\_Status")
    );
    assert_clean_room_prose(&cox_tex);
}

#[test]
fn qol6_nonlinear_regression_formula_rout_weighting_global_comparison_and_latex_escaping() {
    let mut session = session_with("IC50_Project & 100% $Test$", vec![sample_4pl_two_dataset_xy("xy-4pl", "Dose_Response_100% & $Assay$", true)]);
    let mut fit_f = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_f.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    fit_f.weighting = WeightingScheme::RelativeSquare;
    fit_f.profile_likelihood_ci = true;
    fit_f.parameter_rules.push(ParameterRule {
        name: String::from("HillSlope"),
        scope: ParameterScope::SharedGlobal,
        initial_override: None,
        lower_bound: Some(0.1),
        upper_bound: Some(10.0),
    });
    fit_f.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") });

    let mut fit_aicc = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_aicc.comparison = Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") });

    run_cmds(
        &mut session,
        &[
            ("analysis.run", json!({ "spec": AnalysisSpec::new("res-4pl-f", "xy-4pl", AnalysisKind::NonlinearRegression(fit_f)).unwrap() })),
            ("analysis.run", json!({ "spec": AnalysisSpec::new("res-4pl-aicc", "xy-4pl", AnalysisKind::NonlinearRegression(fit_aicc)).unwrap() })),
        ],
    );

    let doc_snap = session.document();
    let sid_f = SheetId::new("res-4pl-f");
    let plain = generate_methods_report(doc_snap, &sid_f, ReportFormat::PlainText).unwrap();
    let md = generate_methods_report(doc_snap, &sid_f, ReportFormat::Markdown).unwrap();
    let tex = generate_methods_report(doc_snap, &sid_f, ReportFormat::Latex).unwrap();

    assert!(plain.contains("Bottom + (Top - Bottom)") && md.contains("Bottom + (Top - Bottom)"));
    assert!(tex.contains("\\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{(\\mathrm{LogEC50} - X)\\cdot \\mathrm{HillSlope}}}"));
    for report in [&plain, &md, &tex] {
        assert_clean_room_prose(report);
        for needle in [
            "ROUT",
            "Venzon-Moolgavkar profile-likelihood",
            "HillSlope",
            "shared globally across all datasets",
            "extra-sum-of-squares F-test",
            "1 / Y^2",
        ] {
            assert!(report.contains(needle), "missing `{needle}`");
        }
    }
    for needle in [
        "$Q = 1\\%$",
        "$\\ge 0.1$",
        "$\\le 10$",
        "$p < \\alpha$",
        "$p \\ge \\alpha$",
        "IC50\\_Project \\& 100\\% \\$Test\\$",
        "Dose\\_Response\\_100\\% \\& \\$Assay\\$",
    ] {
        assert!(tex.contains(needle), "missing LaTeX `{needle}`");
    }
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-4pl-aicc"), ReportFormat::Markdown).unwrap().contains("AICc"));
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-4pl-aicc"), ReportFormat::Latex).unwrap().contains("$\\mathrm{AIC}_c$"));
}

#[test]
fn qol6_contingency_regression_and_multivariate_methods_reports() {
    let xs = [10.0, 20.0, 30.0, 40.0, 50.0, 60.0];
    let (ma, mb): (Vec<f64>, Vec<f64>) = (xs.iter().map(|x| x * 1.04 + 0.5).collect(), xs.iter().map(|x| x * 0.98 - 0.3).collect());
    let grp = grouped_rep_sheet(
        "grp-1",
        "Two-Way Assay",
        &["Low", "High"],
        &[("Placebo", &[&[10.0, 10.4, 10.8], &[15.0, 15.4, 15.8]]), ("Drug", &[&[14.0, 14.4, 14.8], &[19.0, 19.4, 19.8]])],
    );
    let mut mv_mi = sample_mv("mv-mi");
    if let TableArchetype::MultipleVariables(ref mut tbl) = mv_mi.archetype {
        tbl.variables[0].cells[1] = Cell::empty();
        tbl.variables[2].cells[4] = Cell::empty();
    }
    let mut session = session_with(
        "Comprehensive Analytics",
        vec![
            contingency_sheet("cont-1", "2x2 Responder Matrix", &[], &["Responder", "Nonresponder"], &[&[18.0, 4.0], &[5.0, 15.0]]),
            xy_sheet("xy-method", "Method Comparison", "X", &xs, &[("Method A", &ma), ("Method B", &mb)]),
            sample_mv("mv-1"),
            grp,
            mv_mi,
        ],
    );

    let mut pca_spec = AnalysisSpec::new(
        "res-pca",
        "mv-1",
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42), pcr_outcome_column: None }),
    )
    .unwrap();
    pca_spec.selected_data_sets = vec![0, 1, 2];
    let mut mi_spec = AnalysisSpec::new(
        "res-mi",
        "mv-mi",
        AnalysisKind::Multivariate(MultivariateSpec::MultipleImputation {
            imputations_m: 5,
            max_iters: 10,
            seed: 42,
            response_column: Some(1),
            predictor_columns: vec![0, 2],
        }),
    )
    .unwrap();
    mi_spec.selected_data_sets = vec![0, 1, 2];

    for spec in [
        AnalysisSpec::new(
            "res-cont",
            "cont-1",
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::FisherExact,
                compute_odds_and_risk_ratios: true,
                compute_sensitivity_specificity: true,
                compute_phi_and_cramers_v: true,
                ..Default::default()
            }),
        )
        .unwrap(),
        AnalysisSpec::new(
            "res-barnard",
            "cont-1",
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::BarnardExact,
                compute_odds_and_risk_ratios: true,
                compute_sensitivity_specificity: false,
                compute_phi_and_cramers_v: false,
                ..Default::default()
            }),
        )
        .unwrap(),
        AnalysisSpec::new(
            "res-ba",
            "xy-method",
            AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, ..Default::default() }),
        )
        .unwrap(),
        AnalysisSpec::new("res-pb", "xy-method", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 })).unwrap(),
        AnalysisSpec::new(
            "res-deming",
            "xy-method",
            AnalysisKind::LinearRegression(LinearRegressionSpec {
                force_through_origin: false,
                confidence_bands: false,
                prediction_bands: false,
                deming_model_ii: true,
                ..Default::default()
            }),
        )
        .unwrap(),
        pca_spec,
        mi_spec,
        AnalysisSpec::new("res-2way", "grp-1", AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap(),
    ] {
        ok_cmd(&mut session, "analysis.run", json!({ "spec": spec }));
    }

    let doc_snap = session.document();
    for (id, needles) in [
        ("res-cont", &["Fisher's exact conditional hypergeometric test", "Koopman asymptotic score CI"][..]),
        ("res-barnard", &["Barnard's unconditional exact score test"]),
        ("res-ba", &["Bland-Altman"]),
        ("res-pb", &["Passing-Bablok"]),
        ("res-deming", &["Deming"]),
        ("res-pca", &["Principal component analysis (PCA)"]),
        ("res-mi", &["Multiple Imputation by Chained Equations"]),
        ("res-2way", &["two-way factorial ANOVA"]),
    ] {
        let rep = generate_methods_report(doc_snap, &SheetId::new(id), ReportFormat::PlainText).unwrap();
        for needle in needles {
            assert!(rep.contains(needle), "`{id}` missing `{needle}`");
        }
    }
}

#[test]
fn r10_10_pvalue_stack_executes_updates_undoes_and_generates_methods_report() {
    let pvals = [0.001_f64, 0.015, 0.032, 0.045, 0.21, 0.80];
    let sec: Vec<f64> = pvals.iter().map(|&p| (p * 1.5).min(1.0)).collect();
    let mut session =
        session_with("Differential Screen P Values", vec![col_sheet("p-col", "Raw P Stack", &[("Screen_P", &pvals), ("Secondary_P", &sec)])]);

    let run_out = ok_cmd(
        &mut session,
        "stats.pvalueStack",
        json!({ "sheet_id": "res-pstack", "source_sheet_id": "p-col", "method": "benjamini_hochberg", "threshold": 0.05, "column": 0 }),
    );
    assert_eq!(run_out.payload["sheet_id"], "res-pstack");
    let rs_bh = rs(&session, "res-pstack");
    assert_eq!(rs_bh.status, ResultStatus::UpToDate);
    assert_eq!(rs_bh.scalars.iter().find(|s| s.key == "TotalInputCount").map(|s| s.value), Some(6.0));
    assert_eq!(rs_bh.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(2.0));
    assert!(rs_bh.tabs.iter().any(|t| t.id == "pvalue-stack-summary") && rs_bh.tabs.iter().any(|t| t.id == "pvalue-stack-adjusted"));

    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "p-col", "row": 5, "col": 1, "value": 1.25 }));
    let err = err_cmd(&mut session, "stats.pvalueStack", json!({ "sheet_id": "res-bad", "source_sheet_id": "p-col", "column": 1 })).to_string();
    assert!(err.contains("Domain") || err.contains("[0, 1]") || err.contains("failed"), "got: {err}");

    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({ "sheet_id": "res-pstack", "kind": AnalysisKind::PValueStack(PValueStackSpec { method: PValueStackMethod::HolmBonferroni, threshold: 0.05, column: Some(0) }) }),
    );
    let discoveries = |s: &Session| rs(s, "res-pstack").scalars.iter().find(|e| e.key == "TotalDiscoveries").map(|e| e.value);
    assert_eq!(discoveries(&session), Some(1.0));
    assert!(session.undo().unwrap() && discoveries(&session) == Some(2.0));
    assert!(session.redo().unwrap() && discoveries(&session) == Some(1.0));

    for fmt in ["plain", "markdown", "latex"] {
        let rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-pstack", "format": fmt }));
        let text = rep.payload["report"].as_str().unwrap();
        assert_clean_room_prose(text);
        assert!(text.contains("Holm") || text.contains("FWER"), "expected Holm text in {fmt}:\n{text}");
    }
}
