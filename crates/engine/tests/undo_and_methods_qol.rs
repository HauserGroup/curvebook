//! Integration tests for `G12` (read-only dispatch + bounded undo stack) and `QOL-6`
//! (clean-room ICMJE/ARRIVE methods & results reporting generator).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, BlandAltmanSpec, Cell, CellCoord, ColumnDesignHint, ColumnTable, ContingencyMethod, ContingencySpec,
    ContingencyTable, CoxTieMethod, DataSheet, FactorialAnovaSpec, FitMethod, GroupedTable, LinearRegressionSpec, ModelComparisonSpec,
    MultipleVariablesTable, MultiplicityMethod, MultiplicitySpec, MultivariateSpec, NonlinearFitSpec, OneWayAnovaSpec, OneWayMethod,
    PValueStackMethod, PValueStackSpec, ParameterRule, ParameterScope, PassingBablokSpec, ProjectDocument, ResultStatus, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, SurvivalTable, TableArchetype, TwoGroupMethod, TwoGroupSpec, VariableRole, WeightingScheme, XyTable,
};
use curvebook_engine::methods_report::{ReportFormat, generate_methods_report};
use curvebook_engine::{CommandOutcome, MAX_UNDO_CELL_BUDGET, MAX_UNDO_ENTRIES, Session};
use serde_json::{Value, json};

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn assert_clean_room_prose(report: &str) {
    let lower = report.to_ascii_lowercase();
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!lower.contains(forbidden), "found forbidden brand string `{forbidden}` in report:\n{report}");
    }
}

fn sample_two_group_column_sheet(id: &str) -> DataSheet {
    let a = [10.1_f64, 11.4, 9.8, 10.7, 11.2];
    let b = [16.2_f64, 17.5, 15.9, 18.1, 16.8];
    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, a.len()).unwrap();
    table.data_sets[0].title = String::from("Control");
    table.data_sets[1].title = String::from("Treatment_Alpha");
    for r in 0..a.len() {
        table.data_sets[0].cells[r][0] = Cell::number(a[r]);
        table.data_sets[1].cells[r][0] = Cell::number(b[r]);
    }
    DataSheet::new(SheetId::new(id), "Two-Group Study", TableArchetype::Column(table)).unwrap()
}

fn sample_rm_anova_column_sheet(id: &str) -> DataSheet {
    let groups =
        [("Baseline", [12.0_f64, 13.5, 11.8, 14.1, 12.6]), ("Week_4", [16.4, 17.8, 15.9, 18.5, 17.0]), ("Week_8", [21.2, 23.0, 20.5, 24.1, 22.4])];
    let mut table = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, groups.len(), 5).unwrap();
    for (c, (name, vals)) in groups.iter().enumerate() {
        table.data_sets[c].title = (*name).to_owned();
        for (r, &v) in vals.iter().enumerate() {
            table.data_sets[c].cells[r][0] = Cell::number(v);
        }
    }
    DataSheet::new(SheetId::new(id), "Longitudinal Biomarker", TableArchetype::Column(table)).unwrap()
}

fn sample_4pl_two_dataset_xy_sheet(id: &str, title: &str) -> DataSheet {
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(2), 2, xs.len()).unwrap();
    table.x_title = String::from("Log10 [Agonist] (M)");
    table.data_sets[0].title = String::from("Wildtype_A");
    table.data_sets[1].title = String::from("Mutant_B");
    for (r, &x) in xs.iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        let y0 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.1));
        let y1 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-5.8 - x) * 1.1));
        table.data_sets[0].cells[r][0] = Cell::number(y0 - 0.6);
        table.data_sets[0].cells[r][1] = Cell::number(y0 + 0.6);
        table.data_sets[1].cells[r][0] = Cell::number(y1 - 0.6);
        table.data_sets[1].cells[r][1] = Cell::number(y1 + 0.6);
    }
    DataSheet::new(SheetId::new(id), title, TableArchetype::Xy(table)).unwrap()
}

fn sample_survival_sheet(id: &str) -> DataSheet {
    let rows = [
        (4.0_f64, Some(1.0_f64), None),
        (7.0, Some(1.0), None),
        (10.0, Some(0.0), None),
        (13.0, Some(1.0), None),
        (18.0, Some(1.0), None),
        (6.0, None, Some(1.0)),
        (11.0, None, Some(0.0)),
        (16.0, None, Some(1.0)),
        (22.0, None, Some(0.0)),
        (28.0, None, Some(1.0)),
    ];
    let mut table = SurvivalTable::new(2, rows.len()).unwrap();
    table.cohorts[0].title = String::from("Placebo");
    table.cohorts[1].title = String::from("Active Drug");
    for (r, &(t, c0, c1)) in rows.iter().enumerate() {
        table.times[r] = Cell::number(t);
        if let Some(ev) = c0 {
            table.cohorts[0].cells[r][0] = Cell::number(ev);
        }
        if let Some(ev) = c1 {
            table.cohorts[1].cells[r][0] = Cell::number(ev);
        }
    }
    DataSheet::new(SheetId::new(id), "Survival Cohort", TableArchetype::Survival(table)).unwrap()
}

fn sample_multivariate_sheet(id: &str) -> DataSheet {
    let rows = [
        (1.2_f64, 10.5_f64, 24.0_f64, 4.0_f64, 1.0_f64, "Ctrl"),
        (2.1, 14.2, 31.0, 7.0, 1.0, "Ctrl"),
        (3.4, 18.8, 39.0, 10.0, 0.0, "Treat"),
        (4.5, 22.1, 46.0, 13.0, 1.0, "Treat"),
        (5.8, 27.9, 54.0, 18.0, 0.0, "Treat"),
        (6.9, 31.3, 62.0, 22.0, 1.0, "Ctrl"),
        (7.4, 34.0, 67.0, 26.0, 0.0, "Treat"),
        (8.2, 37.5, 73.0, 30.0, 1.0, "Ctrl"),
    ];
    let mut table = MultipleVariablesTable::new(6, rows.len()).unwrap();
    let titles = ["Marker_X", "Marker_Y", "Marker_Z", "Time_Months", "Event_Status", "Arm"];
    for (c, title) in titles.iter().enumerate() {
        table.variables[c].title = (*title).to_owned();
    }
    table.variables[5].role = VariableRole::Categorical;
    table.variables[5].role_inferred = false;
    for (r, &(x, y, z, t, st, arm)) in rows.iter().enumerate() {
        table.row_titles[r] = format!("Pt_{}", r + 1);
        table.variables[0].cells[r] = Cell::number(x);
        table.variables[1].cells[r] = Cell::number(y);
        table.variables[2].cells[r] = Cell::number(z);
        table.variables[3].cells[r] = Cell::number(t);
        table.variables[4].cells[r] = Cell::number(st);
        table.variables[5].cells[r] = Cell::text(arm);
    }
    DataSheet::new(SheetId::new(id), "Multivariate Cohort", TableArchetype::MultipleVariables(table)).unwrap()
}

#[test]
fn g12_read_only_commands_preserve_redo_stack_and_undo_depth() {
    let mut doc = ProjectDocument::new("G12 ReadOnly Check").unwrap();
    doc.add_data_sheet(sample_4pl_two_dataset_xy_sheet("xy-1", "Dose Assay")).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "xy-1", "title": "Dose Assay v2" }));
    ok_cmd(&mut session, "fit.run", json!({ "sheet_id": "fit-1", "source_sheet": "xy-1", "kind": "four_parameter_logistic" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-1", "data_sheet_id": "xy-1", "kind": "xy" }));
    ok_cmd(&mut session, "layout.create", json!({ "sheet_id": "lay-1", "title": "Figure 1" }));
    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "xy-1", "title": "Dose Assay v3" }));

    assert_eq!(session.undo_depth(), 5);
    assert!(session.undo().unwrap());
    assert_eq!(session.undo_depth(), 4);
    assert_eq!(session.redo_depth(), 1);
    assert_eq!(session.document().data_sheet(&SheetId::new("xy-1")).unwrap().title, "Dose Assay v2");

    let tmp_dir = std::env::temp_dir().join("curvebook-g12-readonly-tests");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let archive_path = tmp_dir.join("ro_check.curvebook");
    let csv_path = tmp_dir.join("ro_check.csv");
    let json_path = tmp_dir.join("ro_check.json");
    let g_emf_path = tmp_dir.join("g1.emf");
    let l_emf_path = tmp_dir.join("lay1.emf");

    let ro_calls: Vec<(&str, Value)> = vec![
        ("project.inspect", json!({})),
        ("project.save", json!({ "path": archive_path.to_string_lossy() })),
        ("project.saveTemplate", json!({})),
        ("project.listCheckpoints", json!({})),
        ("table.exportCsv", json!({ "sheet_id": "xy-1", "path": csv_path.to_string_lossy() })),
        ("table.exclusionAuditReport", json!({ "sheet_id": "xy-1" })),
        ("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "preview_only": true })),
        ("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "previewOnly": true })),
        ("table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true, "dry_run": true })),
        ("analysis.exportJson", json!({ "sheet_id": "fit-1", "path": json_path.to_string_lossy() })),
        ("analysis.exportCsv", json!({ "sheet_id": "fit-1" })),
        ("result.generateReport", json!({ "sheet_id": "fit-1", "format": "markdown" })),
        ("graph.preflightCheck", json!({ "sheet_id": "g-1" })),
        ("export.svg", json!({ "sheet_id": "g-1" })),
        ("export.pdf", json!({ "sheet_id": "g-1" })),
        ("export.png", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("export.tiff", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("graph.exportSvg", json!({ "sheet_id": "g-1" })),
        ("graph.exportPdf", json!({ "sheet_id": "g-1" })),
        ("graph.exportPng", json!({ "sheet_id": "g-1", "dpi": 96 })),
        ("graph.exportEmf", json!({ "sheet_id": "g-1", "path": g_emf_path.to_string_lossy() })),
        ("layout.exportEmf", json!({ "sheet_id": "lay-1", "path": l_emf_path.to_string_lossy() })),
    ];

    for (cmd, params) in ro_calls {
        let outcome = ok_cmd(&mut session, cmd, params);
        assert!(!outcome.mutated, "`{cmd}` unexpectedly reported mutated=true");
        assert_eq!(session.undo_depth(), 4, "`{cmd}` modified undo_depth");
        assert_eq!(session.redo_depth(), 1, "`{cmd}` cleared redo_stack");
    }

    // Undo stack entry before redo still holds the original state and redo restores v3.
    assert!(session.redo().unwrap());
    assert_eq!(session.document().data_sheet(&SheetId::new("xy-1")).unwrap().title, "Dose Assay v3");
    assert_eq!(session.undo_depth(), 5);
    assert_eq!(session.redo_depth(), 0);

    // Non-preview table.convertArchetype mutates document and clears redo_stack after undo.
    assert!(session.undo().unwrap());
    assert_eq!(session.redo_depth(), 1);
    let conv = ok_cmd(&mut session, "table.convertArchetype", json!({ "sheet_id": "xy-1", "target_kind": "Column", "allow_lossy": true }));
    assert!(conv.mutated);
    assert_eq!(session.undo_depth(), 5);
    assert_eq!(session.redo_depth(), 0);

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

#[test]
fn g12_one_million_cell_read_only_execution_avoids_snapshot_clone() {
    let table = MultipleVariablesTable::new(1, 1_000_000).unwrap();
    let ds = DataSheet::new(SheetId::new("mv-1m"), "One Million Cells", TableArchetype::MultipleVariables(table)).unwrap();
    assert_eq!(ds.archetype.allocated_cell_slots(), 1_000_000);

    let mut doc = ProjectDocument::new("1M Cell Benchmark").unwrap();
    doc.add_data_sheet(ds).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-1m", "title": "1M Step 1" }));
    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-1m", "title": "1M Step 2" }));
    assert!(session.undo().unwrap());
    assert_eq!(session.undo_depth(), 1);
    assert_eq!(session.redo_depth(), 1);
    assert_eq!(session.undo_cell_slots(), 1_000_000);

    let start = Instant::now();
    for _ in 0..25 {
        let out = ok_cmd(&mut session, "project.inspect", json!({}));
        assert!(!out.mutated);
    }
    let elapsed = start.elapsed();
    assert!(elapsed.as_secs_f64() < 2.0, "25 read-only calls on a 1M-cell doc took too long ({elapsed:?}), suggesting unexpected doc cloning");
    assert_eq!(session.undo_depth(), 1);
    assert_eq!(session.redo_depth(), 1);
    assert_eq!(session.undo_cell_slots(), 1_000_000);
}

#[test]
fn g12_undo_stack_bounds_entries_at_64() {
    assert_eq!(MAX_UNDO_ENTRIES, 64);
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "col-1", "kind": "column", "cols": 1, "rows": 1 }));

    for step in 1_u32..=80 {
        ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-1", "row": 0, "col": 0, "value": f64::from(step) }));
    }
    // 1 (sheet.createData) + 80 (table.setCell) = 81 mutating steps, capped at MAX_UNDO_ENTRIES (64).
    assert_eq!(session.undo_depth(), MAX_UNDO_ENTRIES);

    for _ in 0..MAX_UNDO_ENTRIES {
        assert!(session.undo().unwrap());
    }
    assert_eq!(session.undo_depth(), 0);
    assert_eq!(session.redo_depth(), MAX_UNDO_ENTRIES);
    assert!(!session.undo().unwrap());

    // After undoing steps 80 down through 17 (64 steps), step 16's value remains active.
    let val = session.document().data_sheet(&SheetId::new("col-1")).unwrap().archetype.data_cell(CellCoord::new(0, 0, 0)).unwrap().value.finite_f64();
    assert_eq!(val, Some(16.0));
}

#[test]
fn g12_undo_cell_budget_evicts_oldest_and_keeps_oversized_newest() {
    assert_eq!(MAX_UNDO_CELL_BUDGET, 25_000_000);

    // Phase 1: 10M-slot snapshots cap at 2 entries (20M <= 25M), evicting oldest on the 3rd edit.
    {
        let table = MultipleVariablesTable::new(100, 100_000).unwrap();
        let ds = DataSheet::new(SheetId::new("mv-10m"), "Initial 10M", TableArchetype::MultipleVariables(table)).unwrap();
        assert_eq!(ds.archetype.allocated_cell_slots(), 10_000_000);

        let mut doc = ProjectDocument::new("Budget Eviction").unwrap();
        doc.add_data_sheet(ds).unwrap();
        let mut session = Session::from_document(doc);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 1" }));
        assert_eq!(session.undo_depth(), 1);
        assert_eq!(session.undo_cell_slots(), 10_000_000);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 2" }));
        assert_eq!(session.undo_depth(), 2);
        assert_eq!(session.undo_cell_slots(), 20_000_000);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-10m", "title": "Rev 3" }));
        assert_eq!(session.undo_depth(), 2);
        assert_eq!(session.undo_cell_slots(), 20_000_000);
        assert!(session.undo_cell_slots() <= MAX_UNDO_CELL_BUDGET);

        // Undoing twice returns to "Rev 1", confirming "Initial 10M" (oldest) was evicted.
        assert!(session.undo().unwrap());
        assert_eq!(session.document().data_sheet(&SheetId::new("mv-10m")).unwrap().title, "Rev 2");
        assert!(session.undo().unwrap());
        assert_eq!(session.document().data_sheet(&SheetId::new("mv-10m")).unwrap().title, "Rev 1");
        assert!(!session.undo().unwrap());
    }

    // Phase 2: single snapshot > 25M slots keeps only the newest snapshot (`len == 1`).
    {
        let table = MultipleVariablesTable::new(260, 100_000).unwrap();
        let ds = DataSheet::new(SheetId::new("mv-26m"), "Oversized Init", TableArchetype::MultipleVariables(table)).unwrap();
        assert_eq!(ds.archetype.allocated_cell_slots(), 26_000_000);

        let mut doc = ProjectDocument::new("Oversized Newest Retained").unwrap();
        doc.add_data_sheet(ds).unwrap();
        let mut session = Session::from_document(doc);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-26m", "title": "Oversized Rev 1" }));
        assert_eq!(session.undo_depth(), 1);
        assert_eq!(session.undo_cell_slots(), 26_000_000);

        ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "mv-26m", "title": "Oversized Rev 2" }));
        assert_eq!(session.undo_depth(), 1);
        assert_eq!(session.undo_cell_slots(), 26_000_000);

        assert!(session.undo().unwrap());
        assert_eq!(session.document().data_sheet(&SheetId::new("mv-26m")).unwrap().title, "Oversized Rev 1");
        assert!(!session.undo().unwrap());
    }
}

#[test]
fn qol6_format_parsing_and_error_handling() {
    assert_eq!(ReportFormat::from_name("plain"), Some(ReportFormat::PlainText));
    assert_eq!(ReportFormat::from_name("PLAINTEXT"), Some(ReportFormat::PlainText));
    assert_eq!(ReportFormat::from_name("text"), Some(ReportFormat::PlainText));
    assert_eq!(ReportFormat::from_name("plain_text"), Some(ReportFormat::PlainText));
    assert_eq!(ReportFormat::from_name("plain-text"), Some(ReportFormat::PlainText));
    assert_eq!(ReportFormat::from_name("markdown"), Some(ReportFormat::Markdown));
    assert_eq!(ReportFormat::from_name("md"), Some(ReportFormat::Markdown));
    assert_eq!(ReportFormat::from_name("latex"), Some(ReportFormat::Latex));
    assert_eq!(ReportFormat::from_name("TeX"), Some(ReportFormat::Latex));
    assert_eq!(ReportFormat::from_name("html"), None);

    assert_eq!(ReportFormat::PlainText.as_str(), "plain");
    assert_eq!(ReportFormat::Markdown.as_str(), "markdown");
    assert_eq!(ReportFormat::Latex.as_str(), "latex");

    let doc = ProjectDocument::new("Empty Doc").unwrap();
    assert!(generate_methods_report(&doc, &SheetId::new(""), ReportFormat::PlainText).is_err());
    assert!(generate_methods_report(&doc, &SheetId::new("missing-result"), ReportFormat::Markdown).is_err());
}

#[test]
fn qol6_t_test_and_anova_reports_all_formats_and_preserve_stored_numbers() {
    let mut doc = ProjectDocument::new("Parametric & Multiplicity Study").unwrap();
    doc.add_data_sheet(sample_two_group_column_sheet("two-col")).unwrap();
    doc.add_data_sheet(sample_rm_anova_column_sheet("rm-col")).unwrap();
    let mut session = Session::from_document(doc);

    let welch_spec = AnalysisSpec::new(
        "res-welch",
        "two-col",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec { method: TwoGroupMethod::WelchT, ..TwoGroupSpec::default() }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": welch_spec }));

    let rm_spec = OneWayAnovaSpec {
        method: OneWayMethod::RepeatedMeasures { geisser_greenhouse: true },
        compute_effect_sizes: true,
        multiplicity: MultiplicitySpec { method: MultiplicityMethod::TukeyHsd, ..MultiplicitySpec::default() },
        ..OneWayAnovaSpec::default()
    };
    let anova_spec = AnalysisSpec::new("res-rm-anova", "rm-col", AnalysisKind::OneWayAnova(rm_spec)).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": anova_spec }));

    let doc_snap = session.document();
    for res_id in ["res-welch", "res-rm-anova"] {
        let sid = SheetId::new(res_id);
        let original_sheet = doc_snap.result_sheet(&sid).unwrap().clone();
        assert_eq!(original_sheet.status, ResultStatus::UpToDate);

        let plain_a = generate_methods_report(doc_snap, &sid, ReportFormat::PlainText).unwrap();
        let plain_b = generate_methods_report(doc_snap, &sid, ReportFormat::PlainText).unwrap();
        assert_eq!(plain_a, plain_b, "plain report must be deterministic");

        let md = generate_methods_report(doc_snap, &sid, ReportFormat::Markdown).unwrap();
        let tex = generate_methods_report(doc_snap, &sid, ReportFormat::Latex).unwrap();

        assert_clean_room_prose(&plain_a);
        assert_clean_room_prose(&md);
        assert_clean_room_prose(&tex);

        assert!(plain_a.contains("Statistical and Mathematical Methods"));
        assert!(md.starts_with("# "));
        assert!(md.contains("## Statistical and Mathematical Methods"));
        assert!(md.contains("| "));
        assert!(tex.contains("\\section*{"));
        assert!(tex.contains("\\subsection*{Statistical and Mathematical Methods}"));
        assert!(tex.contains("$p < \\alpha$"));
        assert!(tex.contains("$p \\ge \\alpha$"));
        assert!(tex.contains("\\begin{tabular}"));

        // Verify stored result sheet numbers are completely untouched.
        assert_eq!(doc_snap.result_sheet(&sid).unwrap(), &original_sheet);
    }

    let welch_plain = generate_methods_report(doc_snap, &SheetId::new("res-welch"), ReportFormat::PlainText).unwrap();
    assert!(welch_plain.contains("Welch's unequal-variance t-test"));
    assert!(welch_plain.contains("Cohen's d"));

    let anova_plain = generate_methods_report(doc_snap, &SheetId::new("res-rm-anova"), ReportFormat::PlainText).unwrap();
    assert!(anova_plain.contains("repeated-measures one-way ANOVA"));
    assert!(anova_plain.contains("Geisser-Greenhouse"));
    assert!(anova_plain.contains("Tukey's honestly significant difference"));
}

#[test]
fn qol6_survival_km_and_cox_reports() {
    let mut doc = ProjectDocument::new("Oncology Time-to-Event Study").unwrap();
    doc.add_data_sheet(sample_survival_sheet("surv-1")).unwrap();
    doc.add_data_sheet(sample_multivariate_sheet("mv-1")).unwrap();
    let mut session = Session::from_document(doc);

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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": km_spec }));

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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": cox_spec }));

    let doc_snap = session.document();
    let km_md = generate_methods_report(doc_snap, &SheetId::new("res-km"), ReportFormat::Markdown).unwrap();
    assert!(km_md.contains("Kaplan-Meier product-limit"));
    assert!(km_md.contains("Log-rank"));
    assert!(km_md.contains("hazard ratio"));
    assert_clean_room_prose(&km_md);

    let cox_tex = generate_methods_report(doc_snap, &SheetId::new("res-cox"), ReportFormat::Latex).unwrap();
    assert!(cox_tex.contains("Cox proportional hazards"));
    assert!(cox_tex.contains("Efron"));
    assert!(cox_tex.contains("Time\\_Months"));
    assert!(cox_tex.contains("Event\\_Status"));
    assert_clean_room_prose(&cox_tex);
}

#[test]
fn qol6_nonlinear_regression_formula_rout_weighting_global_comparison_and_latex_escaping() {
    let mut doc = ProjectDocument::new("IC50_Project & 100% $Test$").unwrap();
    // Use multiplicative replicate spread (±1.5%) matching RelativeSquare (1/Y^2) weighting so
    // homoscedastic-relative ROUT eliminates the same observations under both 4PL and 3PL (RV-99).
    let xs = [-9.5_f64, -8.5, -8.0, -7.5, -7.0, -6.5, -6.0, -5.5, -5.0, -4.0];
    let mut table = XyTable::new(SubcolumnMode::RawReplicates(2), 2, xs.len()).unwrap();
    table.x_title = String::from("Log10 [Agonist] (M)");
    table.data_sets[0].title = String::from("Wildtype_A");
    table.data_sets[1].title = String::from("Mutant_B");
    for (r, &x) in xs.iter().enumerate() {
        table.x_cells[r][0] = Cell::number(x);
        let y0 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-6.5 - x) * 1.1));
        let y1 = 10.0 + 90.0 / (1.0 + 10.0_f64.powf((-5.8 - x) * 1.1));
        table.data_sets[0].cells[r][0] = Cell::number(y0 * 0.985);
        table.data_sets[0].cells[r][1] = Cell::number(y0 * 1.015);
        table.data_sets[1].cells[r][0] = Cell::number(y1 * 0.985);
        table.data_sets[1].cells[r][1] = Cell::number(y1 * 1.015);
    }
    doc.add_data_sheet(DataSheet::new(SheetId::new("xy-4pl"), "Dose_Response_100% & $Assay$", TableArchetype::Xy(table)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    let mut fit_spec_f = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_spec_f.method = FitMethod::RoutOutlierElimination { fdr_q_percent: 1.0 };
    fit_spec_f.weighting = WeightingScheme::RelativeSquare;
    fit_spec_f.profile_likelihood_ci = true;
    fit_spec_f.parameter_rules.push(ParameterRule {
        name: String::from("HillSlope"),
        scope: ParameterScope::SharedGlobal,
        initial_override: None,
        lower_bound: Some(0.1),
        upper_bound: Some(10.0),
    });
    fit_spec_f.comparison = Some(ModelComparisonSpec::ExtraSumOfSquaresF { alternative_equation_id: String::from("log_agonist_vs_response_3pl") });

    let spec_f = AnalysisSpec::new("res-4pl-f", "xy-4pl", AnalysisKind::NonlinearRegression(fit_spec_f)).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": spec_f }));

    let mut fit_spec_aicc = NonlinearFitSpec::new("log_agonist_vs_response_4pl");
    fit_spec_aicc.comparison = Some(ModelComparisonSpec::Aicc { alternative_equation_id: String::from("log_agonist_vs_response_3pl") });
    let spec_aicc = AnalysisSpec::new("res-4pl-aicc", "xy-4pl", AnalysisKind::NonlinearRegression(fit_spec_aicc)).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": spec_aicc }));

    let doc_snap = session.document();
    let sid_f = SheetId::new("res-4pl-f");

    let plain = generate_methods_report(doc_snap, &sid_f, ReportFormat::PlainText).unwrap();
    let md = generate_methods_report(doc_snap, &sid_f, ReportFormat::Markdown).unwrap();
    let tex = generate_methods_report(doc_snap, &sid_f, ReportFormat::Latex).unwrap();

    for report in [&plain, &md] {
        assert!(report.contains("Bottom + (Top - Bottom)"));
    }
    assert!(tex.contains("\\mathrm{Bottom} + \\frac{\\mathrm{Top} - \\mathrm{Bottom}}{1 + 10^{(\\mathrm{LogEC50} - X)\\cdot \\mathrm{HillSlope}}}"));

    for report in [&plain, &md, &tex] {
        assert_clean_room_prose(report);
        assert!(report.contains("ROUT"));
        assert!(report.contains("Venzon-Moolgavkar profile-likelihood"));
        assert!(report.contains("HillSlope"));
        assert!(report.contains("shared globally across all datasets"));
        assert!(report.contains("extra-sum-of-squares F-test"));
        assert!(report.contains("1 / Y^2"));
    }

    // Verify LaTeX proper `$...$` math spans, `\ge` / `\le` / `\alpha`, and literal character escaping.
    assert!(tex.contains("$Q = 1\\%$"));
    assert!(tex.contains("$\\ge 0.1$"));
    assert!(tex.contains("$\\le 10$"));
    assert!(tex.contains("$p < \\alpha$"));
    assert!(tex.contains("$p \\ge \\alpha$"));
    assert!(tex.contains("IC50\\_Project \\& 100\\% \\$Test\\$"));
    assert!(tex.contains("Dose\\_Response\\_100\\% \\& \\$Assay\\$"));

    // Also verify AICc comparison formatting in LaTeX & Markdown.
    let aicc_md = generate_methods_report(doc_snap, &SheetId::new("res-4pl-aicc"), ReportFormat::Markdown).unwrap();
    let aicc_tex = generate_methods_report(doc_snap, &SheetId::new("res-4pl-aicc"), ReportFormat::Latex).unwrap();
    assert!(aicc_md.contains("AICc"));
    assert!(aicc_tex.contains("$\\mathrm{AIC}_c$"));
}

#[test]
fn qol6_contingency_regression_and_multivariate_methods_reports() {
    let mut doc = ProjectDocument::new("Comprehensive Analytics").unwrap();

    let mut cont = ContingencyTable::new(2, 2).unwrap();
    cont.data_sets[0].title = String::from("Responder");
    cont.data_sets[1].title = String::from("Nonresponder");
    cont.data_sets[0].cells[0][0] = Cell::number(18.0);
    cont.data_sets[1].cells[0][0] = Cell::number(4.0);
    cont.data_sets[0].cells[1][0] = Cell::number(5.0);
    cont.data_sets[1].cells[1][0] = Cell::number(15.0);
    doc.add_data_sheet(DataSheet::new(SheetId::new("cont-1"), "2x2 Responder Matrix", TableArchetype::Contingency(cont)).unwrap()).unwrap();

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 6).unwrap();
    xy.data_sets[0].title = String::from("Method A");
    xy.data_sets[1].title = String::from("Method B");
    for (r, x) in [10.0_f64, 20.0, 30.0, 40.0, 50.0, 60.0].iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(*x);
        xy.data_sets[0].cells[r][0] = Cell::number(x * 1.04 + 0.5);
        xy.data_sets[1].cells[r][0] = Cell::number(x * 0.98 - 0.3);
    }
    doc.add_data_sheet(DataSheet::new(SheetId::new("xy-method"), "Method Comparison", TableArchetype::Xy(xy)).unwrap()).unwrap();
    doc.add_data_sheet(sample_multivariate_sheet("mv-1")).unwrap();

    let mut grouped = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    grouped.row_titles = vec![String::from("Low"), String::from("High")];
    grouped.data_sets[0].title = String::from("Placebo");
    grouped.data_sets[1].title = String::from("Drug");
    for r in 0..2 {
        for c in 0..2 {
            for s in 0..3 {
                grouped.data_sets[c].cells[r][s] = Cell::number(10.0 + (r as f64) * 5.0 + (c as f64) * 4.0 + (s as f64) * 0.4);
            }
        }
    }
    doc.add_data_sheet(DataSheet::new(SheetId::new("grp-1"), "Two-Way Assay", TableArchetype::Grouped(grouped)).unwrap()).unwrap();

    let mut mv_mi_sheet = sample_multivariate_sheet("mv-mi");
    if let TableArchetype::MultipleVariables(ref mut tbl) = mv_mi_sheet.archetype {
        tbl.variables[0].cells[1] = Cell::empty();
        tbl.variables[2].cells[4] = Cell::empty();
    }
    doc.add_data_sheet(mv_mi_sheet).unwrap();

    let mut session = Session::from_document(doc);

    let cont_spec = AnalysisSpec::new(
        "res-cont",
        "cont-1",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            compute_phi_and_cramers_v: true,
            ..ContingencySpec::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": cont_spec }));

    let barnard_spec = AnalysisSpec::new(
        "res-barnard",
        "cont-1",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::BarnardExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: false,
            compute_phi_and_cramers_v: false,
            ..ContingencySpec::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": barnard_spec }));

    let ba_spec = AnalysisSpec::new(
        "res-ba",
        "xy-method",
        AnalysisKind::BlandAltman(BlandAltmanSpec { column_a: 0, column_b: 1, percent_difference: false, ..BlandAltmanSpec::default() }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": ba_spec }));

    let pb_spec = AnalysisSpec::new("res-pb", "xy-method", AnalysisKind::PassingBablok(PassingBablokSpec { x_column: 0, y_column: 1 })).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": pb_spec }));

    let deming_spec = AnalysisSpec::new(
        "res-deming",
        "xy-method",
        AnalysisKind::LinearRegression(LinearRegressionSpec {
            force_through_origin: false,
            confidence_bands: false,
            prediction_bands: false,
            deming_model_ii: true,
            ..LinearRegressionSpec::default()
        }),
    )
    .unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": deming_spec }));

    let mut pca_spec = AnalysisSpec::new(
        "res-pca",
        "mv-1",
        AnalysisKind::Multivariate(MultivariateSpec::Pca { standardize: true, parallel_analysis_seed: Some(42) }),
    )
    .unwrap();
    pca_spec.selected_data_sets = vec![0, 1, 2];
    ok_cmd(&mut session, "analysis.run", json!({ "spec": pca_spec }));

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
    ok_cmd(&mut session, "analysis.run", json!({ "spec": mi_spec }));

    let two_way_spec = AnalysisSpec::new("res-2way", "grp-1", AnalysisKind::FactorialAnova(FactorialAnovaSpec::default())).unwrap();
    ok_cmd(&mut session, "analysis.run", json!({ "spec": two_way_spec }));

    let doc_snap = session.document();
    let cont_report = generate_methods_report(doc_snap, &SheetId::new("res-cont"), ReportFormat::PlainText).unwrap();
    assert!(cont_report.contains("Fisher's exact conditional hypergeometric test"));
    assert!(cont_report.contains("Koopman asymptotic score CI"));
    assert!(
        generate_methods_report(doc_snap, &SheetId::new("res-barnard"), ReportFormat::PlainText)
            .unwrap()
            .contains("Barnard's unconditional exact score test")
    );
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-ba"), ReportFormat::PlainText).unwrap().contains("Bland-Altman"));
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-pb"), ReportFormat::PlainText).unwrap().contains("Passing-Bablok"));
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-deming"), ReportFormat::PlainText).unwrap().contains("Deming"));
    assert!(
        generate_methods_report(doc_snap, &SheetId::new("res-pca"), ReportFormat::PlainText).unwrap().contains("Principal component analysis (PCA)")
    );
    assert!(
        generate_methods_report(doc_snap, &SheetId::new("res-mi"), ReportFormat::PlainText)
            .unwrap()
            .contains("Multiple Imputation by Chained Equations")
    );
    assert!(generate_methods_report(doc_snap, &SheetId::new("res-2way"), ReportFormat::PlainText).unwrap().contains("two-way factorial ANOVA"));
}

#[test]
fn r10_10_pvalue_stack_executes_updates_undoes_and_generates_methods_report() {
    let mut doc = ProjectDocument::new("Differential Screen P Values").unwrap();
    let pvals = [0.001_f64, 0.015, 0.032, 0.045, 0.21, 0.80];
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, pvals.len()).unwrap();
    col.data_sets[0].title = String::from("Screen_P");
    col.data_sets[1].title = String::from("Secondary_P");
    for (r, &p) in pvals.iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(p);
        col.data_sets[1].cells[r][0] = Cell::number((p * 1.5).min(1.0));
    }
    doc.add_data_sheet(DataSheet::new(SheetId::new("p-col"), "Raw P Stack", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // 1. Run via dedicated `stats.pvalueStack` command selecting column 0 with Benjamini-Hochberg at Q = 0.05.
    let run_out = ok_cmd(
        &mut session,
        "stats.pvalueStack",
        json!({
            "sheet_id": "res-pstack",
            "source_sheet_id": "p-col",
            "method": "benjamini_hochberg",
            "threshold": 0.05,
            "column": 0
        }),
    );
    assert_eq!(run_out.payload["sheet_id"], json!("res-pstack"));

    let rs_bh = session.document().result_sheet(&SheetId::new("res-pstack")).unwrap();
    assert_eq!(rs_bh.status, ResultStatus::UpToDate);
    assert_eq!(rs_bh.scalars.iter().find(|s| s.key == "TotalInputCount").map(|s| s.value), Some(6.0));
    assert_eq!(rs_bh.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(2.0));
    assert!(rs_bh.tabs.iter().any(|t| t.id == "pvalue-stack-summary"));
    assert!(rs_bh.tabs.iter().any(|t| t.id == "pvalue-stack-adjusted"));

    // 2. Verify domain error refusal on out-of-range P values (> 1.0) without silent clamping.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "p-col", "row": 5, "col": 1, "value": 1.25 }));
    let err = session.execute("stats.pvalueStack", json!({ "sheet_id": "res-bad", "source_sheet_id": "p-col", "column": 1 })).unwrap_err();
    assert!(err.to_string().contains("Domain") || err.to_string().contains("[0, 1]") || err.to_string().contains("failed"), "got: {err}");

    // 3. Update parameters to Holm-Bonferroni (`analysis.updateParams`), verify fewer discoveries, then undo/redo.
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({
            "sheet_id": "res-pstack",
            "kind": AnalysisKind::PValueStack(PValueStackSpec {
                method: PValueStackMethod::HolmBonferroni,
                threshold: 0.05,
                column: Some(0),
            })
        }),
    );
    let rs_holm = session.document().result_sheet(&SheetId::new("res-pstack")).unwrap();
    assert_eq!(rs_holm.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(1.0));

    assert!(session.undo().unwrap());
    let rs_undone = session.document().result_sheet(&SheetId::new("res-pstack")).unwrap();
    assert_eq!(rs_undone.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(2.0));
    assert!(session.redo().unwrap());
    let rs_redone = session.document().result_sheet(&SheetId::new("res-pstack")).unwrap();
    assert_eq!(rs_redone.scalars.iter().find(|s| s.key == "TotalDiscoveries").map(|s| s.value), Some(1.0));

    // 4. Verify methods report generation across all 3 formats.
    for fmt in ["plain", "markdown", "latex"] {
        let rep = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "res-pstack", "format": fmt }));
        let text = rep.payload["report"].as_str().unwrap();
        assert_clean_room_prose(text);
        assert!(text.contains("Holm") || text.contains("FWER"), "expected Holm methods text in {fmt}:\n{text}");
    }
}
