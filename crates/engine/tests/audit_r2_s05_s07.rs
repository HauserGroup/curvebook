//! Permanent Round 2 (`R0.19`) regression tests for Slice `S07` (`crates/engine`),
//! engine half of `R2-S12-02` (`path_policy`), and cross-shard `#357` (`subcol`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, BlandAltmanSpec, BreakOrnament, Cell, CellCoord, ColumnDesignHint, ColumnTable, ConfidenceLevel,
    ContingencyMethod, ContingencySpec, ContingencyTable, CoxTieMethod, DataSheet, EquationDisplayMode, FramePreset, GraphAnnotation, GroupedTable,
    LevelOrderKind, MultipleVariablesTable, NonlinearFitSpec, OutlierSpec, PartsOfWholeTable, PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget,
    PowerTailMode, PowerTestFamily, ProjectDocument, RectPt, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnMode,
    SurvivalAnalysisSpec, SurvivalTable, TableArchetype, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::methods_report::{ReportFormat, describe_power_analysis_spec, generate_methods_report};
use curvebook_engine::{Caller, ErrorCode, GuiOnlyPolicy, Session};
use curvebook_palette::{LineDashStyle, PaletteId, Rgba8};
use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(1);

struct ScratchDir {
    root: PathBuf,
}

impl ScratchDir {
    fn new(label: &str) -> Self {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("cb-r2-s05s07-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn ok_cmd(session: &mut Session, id: &str, params: Value) -> curvebook_engine::CommandOutcome {
    session.execute(id, params).unwrap_or_else(|err| panic!("command `{id}` failed: {err}"))
}

fn seed_two_group_col(id: &str) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 5).unwrap();
    if let [ds0, ds1] = t.data_sets.as_mut_slice() {
        ds0.title = String::from("Ctrl");
        ds1.title = String::from("Drug");
        for (r, (&a, &b)) in [10.0, 11.2, 9.8, 10.5, 11.0].iter().zip([15.2, 16.0, 14.8, 17.1, 15.9].iter()).enumerate() {
            if let Some(c0) = ds0.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *c0 = Cell::number(a);
            }
            if let Some(c1) = ds1.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *c1 = Cell::number(b);
            }
        }
    }
    DataSheet::new(id, "TwoGroup", TableArchetype::Column(t)).unwrap()
}

fn seed_three_group_col(id: &str) -> DataSheet {
    let mut t = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 3, 5).unwrap();
    if let [ds0, ds1, ds2] = t.data_sets.as_mut_slice() {
        ds0.title = String::from("Baseline");
        ds1.title = String::from("Low");
        ds2.title = String::from("High");
        for r in 0..5 {
            if let Some(c0) = ds0.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *c0 = Cell::number(10.0 + r as f64);
            }
            if let Some(c1) = ds1.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *c1 = Cell::number(14.0 + r as f64 * 1.1);
            }
            if let Some(c2) = ds2.cells.get_mut(r).and_then(|row| row.first_mut()) {
                *c2 = Cell::number(22.0 + r as f64 * 1.2);
            }
        }
    }
    DataSheet::new(id, "ThreeGroup", TableArchetype::Column(t)).unwrap()
}

fn seed_mv_survival_sheet(id: &str) -> DataSheet {
    let mut mv = MultipleVariablesTable::new(3, 6).unwrap();
    if let [v0, v1, v2] = mv.variables.as_mut_slice() {
        v0.title = String::from("Time");
        v1.title = String::from("Status");
        v2.title = String::from("Arm");
        v2.role = VariableRole::Categorical;
        let rows = [
            (4.0, 1.0, "Control"),
            (6.0, 1.0, "Control"),
            (8.0, 0.0, "Control"),
            (5.0, 1.0, "Treated"),
            (10.0, 1.0, "Treated"),
            (12.0, 0.0, "Treated"),
        ];
        for (r, (tm, st, arm)) in rows.into_iter().enumerate() {
            if let Some(c0) = v0.cells.get_mut(r) {
                *c0 = Cell::number(tm);
            }
            if let Some(c1) = v1.cells.get_mut(r) {
                *c1 = Cell::number(st);
            }
            if let Some(c2) = v2.cells.get_mut(r) {
                *c2 = Cell::text(arm);
            }
        }
    }
    DataSheet::new(id, "CoxMV", TableArchetype::MultipleVariables(mv)).unwrap()
}

#[test]
fn r2_s07_01_convert_archetype_strict_policy_aliases_preview_and_undo() {
    let mut doc = ProjectDocument::new("Convert Policy").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-1")).unwrap();
    let mut session = Session::from_document(doc).with_gui_only(GuiOnlyPolicy::Strict);
    let cancel = AtomicBool::new(false);

    // Mutating convertArchetype blocked under Strict for non-UI callers.
    let err_strict = session
        .execute_with_caller("table.convertArchetype", json!({ "sheet_id": "col-1", "target_kind": "Grouped" }), Caller::Mcp, &cancel)
        .unwrap_err();
    assert_eq!(err_strict.code, ErrorCode::GuiOnlyViolation);

    // Read-only preview (`preview_only: true`) permitted under Strict; retired aliases rejected.
    let preview = session
        .execute_with_caller(
            "table.convertArchetype",
            json!({ "sheet_id": "col-1", "target_kind": "Grouped", "preview_only": true }),
            Caller::Mcp,
            &cancel,
        )
        .unwrap();
    assert!(!preview.mutated);

    // Non-boolean `preview_only` rejected cleanly instead of mis-routing.
    let non_bool_mcp = session
        .execute_with_caller(
            "table.convertArchetype",
            json!({ "sheet_id": "col-1", "target_kind": "Grouped", "preview_only": "yes" }),
            Caller::Mcp,
            &cancel,
        )
        .unwrap_err();
    assert_eq!(non_bool_mcp.code, ErrorCode::GuiOnlyViolation);
    let non_bool_ui = session
        .execute_with_caller(
            "table.convertArchetype",
            json!({ "sheet_id": "col-1", "target_kind": "Grouped", "preview_only": "yes" }),
            Caller::Ui,
            &cancel,
        )
        .unwrap_err();
    assert_eq!(non_bool_ui.code, ErrorCode::BadInput);

    // UI caller can mutate cleanly with undo rollback.
    let committed =
        session.execute_with_caller("table.convertArchetype", json!({ "sheet_id": "col-1", "target_kind": "Grouped" }), Caller::Ui, &cancel).unwrap();
    assert!(committed.mutated);
    assert!(session.is_dirty());
    assert_eq!(session.undo_depth(), 1);
    assert_eq!(session.document().data_sheet(&SheetId::new("col-1")).unwrap().kind(), curvebook_doc::TableKind::Grouped);

    assert!(session.undo().unwrap());
    assert_eq!(session.document().data_sheet(&SheetId::new("col-1")).unwrap().kind(), curvebook_doc::TableKind::Column);
}

#[test]
fn r2_s07_03_and_12_methods_report_accuracy_latex_safety_and_audit_parity() {
    let mut doc = ProjectDocument::new("Methods Accuracy").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-2g")).unwrap();
    doc.add_data_sheet(seed_three_group_col("col-3g")).unwrap();

    let mut cont_3x3 = ContingencyTable::new(3, 3).unwrap();
    for r in 0..3 {
        for c in 0..3 {
            cont_3x3.data_sets[c].cells[r][0] = Cell::number((r * 3 + c + 5) as f64);
        }
    }
    doc.add_data_sheet(DataSheet::new("cont-3x3", "3x3 Table", TableArchetype::Contingency(cont_3x3)).unwrap()).unwrap();

    let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
    pow.data_sets[0].cells[0][0] = Cell::number(20.0);
    pow.data_sets[0].cells[1][0] = Cell::number(30.0);
    pow.data_sets[0].cells[2][0] = Cell::number(50.0);
    doc.add_data_sheet(DataSheet::new("pow-1", "Parts", TableArchetype::PartsOfWhole(pow)).unwrap()).unwrap();

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 4).unwrap();
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number((r + 2) as f64 * 10.0);
    }
    let xy_sheet = DataSheet::new("xy-tr", "Dose", TableArchetype::Xy(xy)).unwrap();
    doc.add_data_sheet(xy_sheet).unwrap();

    let mut session = Session::from_document(doc);

    // 1. Two-group nonparametric KS one-tailed flag + compute_effect_sizes=true -> two-tailed, no Cohen's d.
    let ks_spec = AnalysisSpec::new(
        "res-ks",
        "col-2g",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::KolmogorovSmirnov,
            two_tailed: false,
            compute_effect_sizes: true,
            mv_group_column: None,
            mv_response_column: None,
        }),
    )
    .unwrap();
    let mw_spec = AnalysisSpec::new(
        "res-mw",
        "col-2g",
        AnalysisKind::TwoGroupComparison(TwoGroupSpec {
            method: TwoGroupMethod::MannWhitneyU,
            two_tailed: true,
            compute_effect_sizes: true,
            mv_group_column: None,
            mv_response_column: None,
        }),
    )
    .unwrap();
    let mut tmp_doc = session.document().clone();
    tmp_doc.add_result_sheet(ResultSheet::new(ks_spec).unwrap()).unwrap();
    tmp_doc.add_result_sheet(ResultSheet::new(mw_spec).unwrap()).unwrap();
    let ks_txt = generate_methods_report(&tmp_doc, &SheetId::new("res-ks"), ReportFormat::PlainText).unwrap();
    assert!(ks_txt.contains("two-tailed"), "KS report must indicate two-tailed: {ks_txt}");
    assert!(!ks_txt.contains("Cohen's d"), "KS report must not claim Cohen's d: {ks_txt}");
    assert!(!ks_txt.contains("verbatim without numerical rounding overrides"));
    let mw_txt = generate_methods_report(&tmp_doc, &SheetId::new("res-mw"), ReportFormat::PlainText).unwrap();
    assert!(!mw_txt.contains("Cohen's d"), "Mann-Whitney must not claim Cohen's d: {mw_txt}");

    // 2. One-way ANOVA coercions: KruskalWallis + TukeyHsd -> Dunn/Bonferroni (no Tukey), Welch -> Games-Howell, Friedman -> no post-hoc.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "col-3g", "sheet_id": "kw-tuk", "kind": "kruskal_wallis", "multiplicity": "tukey" }),
    );
    let kw_rep = generate_methods_report(session.document(), &SheetId::new("kw-tuk"), ReportFormat::PlainText).unwrap();
    assert!(kw_rep.contains("Dunn's nonparametric rank-sum") && kw_rep.contains("Bonferroni"), "{kw_rep}");
    assert!(!kw_rep.contains("Tukey"), "{kw_rep}");

    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "bf-tuk", "kind": "welch_anova", "multiplicity": "tukey" }));
    let bf_rep = generate_methods_report(session.document(), &SheetId::new("bf-tuk"), ReportFormat::PlainText).unwrap();
    assert!(bf_rep.contains("Games-Howell"), "{bf_rep}");
    assert!(!bf_rep.contains("Tukey") && !bf_rep.contains("eta-squared"), "{bf_rep}");

    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "fr-tuk", "kind": "friedman", "multiplicity": "tukey" }));
    let fr_rep = generate_methods_report(session.document(), &SheetId::new("fr-tuk"), ReportFormat::PlainText).unwrap();
    assert!(fr_rep.contains("Post-hoc pairwise comparisons were not computed"), "{fr_rep}");
    assert!(!fr_rep.contains("Tukey"), "{fr_rep}");

    // 3. Bland-Altman reflects ConfidenceLevel (99% vs 95%).
    let mut ba_doc = session.document().clone();
    let mut ba_spec = AnalysisSpec::new("res-ba", "col-2g", AnalysisKind::BlandAltman(BlandAltmanSpec::default())).unwrap();
    ba_spec.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    ba_doc.add_result_sheet(ResultSheet::new(ba_spec).unwrap()).unwrap();
    let ba_rep = generate_methods_report(&ba_doc, &SheetId::new("res-ba"), ReportFormat::PlainText).unwrap();
    assert!(ba_rep.contains("99% limits of agreement") && !ba_rep.contains("95% limits"), "{ba_rep}");

    // 4. Contingency 3x3 & PartsOfWhole & FisherExact E>=5 gating.
    let mut cat_doc = session.document().clone();
    let cont_spec = AnalysisSpec::new(
        "res-c3",
        "cont-3x3",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::FisherExact,
            compute_odds_and_risk_ratios: true,
            compute_sensitivity_specificity: true,
            ..ContingencySpec::default()
        }),
    )
    .unwrap();
    cat_doc.add_result_sheet(ResultSheet::new(cont_spec).unwrap()).unwrap();
    let c3_rep = generate_methods_report(&cat_doc, &SheetId::new("res-c3"), ReportFormat::PlainText).unwrap();
    assert!(!c3_rep.contains("E >= 5") && !c3_rep.contains("odds ratio") && !c3_rep.contains("Diagnostic accuracy"), "{c3_rep}");

    let pow_spec = AnalysisSpec::new(
        "res-pow",
        "pow-1",
        AnalysisKind::Contingency(ContingencySpec {
            method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
            compute_odds_and_risk_ratios: true,
            ..ContingencySpec::default()
        }),
    )
    .unwrap();
    cat_doc.add_result_sheet(ResultSheet::new(pow_spec).unwrap()).unwrap();
    let pow_rep = generate_methods_report(&cat_doc, &SheetId::new("res-pow"), ReportFormat::PlainText).unwrap();
    assert!(pow_rep.contains("goodness-of-fit") && !pow_rep.contains("odds ratio"), "{pow_rep}");

    // 5. Transform axis labels (ln(X), log10(Y)) + R2-S07-12 LaTeX safety (`custom_dsl`, `[`-prefixed rows, orphaned notes).
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "source_sheet_id": "xy-tr", "sheet_id": "tr-xy", "kind": "math", "x_transform": "ln", "y_transform": "log10" }),
    );
    let tr_rep = generate_methods_report(session.document(), &SheetId::new("tr-xy"), ReportFormat::PlainText).unwrap();
    assert!(tr_rep.contains("X: ln(X); Y: log10(Y)"), "{tr_rep}");

    let mut tex_doc = session.document().clone();
    // Inject orphaned exclusion note directly into the isolated report doc (simulating historical project before R2-S05-01).
    tex_doc.data_sheet_mut(&SheetId::new("xy-tr")).unwrap().exclusion_notes.push(curvebook_doc::CellExclusionNote {
        coord: CellCoord::new(0, 0, 0),
        reason_tag: String::from("orphan"),
        note: String::from("cleared previously"),
        timestamp_utc: String::from("2026-11-16T00:00:00Z"),
    });
    let mut nfit = NonlinearFitSpec::new("custom_dsl");
    nfit.custom_dsl = Some(String::from("Y = a_1 & α\n+ b"));
    let fit_spec = AnalysisSpec::new("res-tex", "xy-tr", AnalysisKind::NonlinearRegression(nfit)).unwrap();
    let mut fit_res = ResultSheet::new(fit_spec).unwrap();
    fit_res.tabs.push(ResultTab {
        id: String::from("t1"),
        title: String::from("Params"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Value")],
        row_labels: vec![String::from("[Agonist]")],
        rows: vec![vec![ResultCell::Number { value: 1.25, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    tex_doc.add_result_sheet(fit_res).unwrap();
    let tex_out = generate_methods_report(&tex_doc, &SheetId::new("res-tex"), ReportFormat::Latex).unwrap();
    assert!(!tex_out.contains("\\\\\\relax"), "LaTeX tabular rows must not emit \\\\\\relax before \\hline: {tex_out}");
    assert!(tex_out.contains("{[}Agonist]"), "LaTeX tabular rows must guard leading `[` with `{{[}}`: {tex_out}");
    assert!(tex_out.contains(" \\\\\n\\hline"), "LaTeX tabular rows must end with plain `\\\\` before `\\hline`: {tex_out}");
    assert!(tex_out.contains("\\mbox{"), "custom DSL should render via single-line mbox: {tex_out}");
    assert!(!tex_out.contains("(1 with recorded audit justification note"), "orphaned exclusion note must not be counted");

    // 6. R2-S07-12 table.exclusionAuditReport covers Xy.x_cells and Survival.times.
    let mut aud_doc = ProjectDocument::new("AuditParity").unwrap();
    let mut xy_aud = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 2).unwrap();
    xy_aud.x_cells[0][0] = Cell { value: curvebook_doc::CellValue::Number(1.0), excluded: true };
    xy_aud.data_sets[0].cells[0][0] = Cell::number(2.0);
    aud_doc.add_data_sheet(DataSheet::new("xy-aud", "XY Audit", TableArchetype::Xy(xy_aud)).unwrap()).unwrap();

    let mut srv_aud = SurvivalTable::new(1, 2).unwrap();
    srv_aud.times[1] = Cell { value: curvebook_doc::CellValue::Number(5.0), excluded: true };
    srv_aud.cohorts[0].cells[0][0] = Cell::number(1.0);
    srv_aud.cohorts[0].cells[1][0] = Cell::number(1.0);
    aud_doc.add_data_sheet(DataSheet::new("srv-aud", "Surv Audit", TableArchetype::Survival(srv_aud)).unwrap()).unwrap();

    let mut aud_sess = Session::from_document(aud_doc);
    let xy_rep = ok_cmd(&mut aud_sess, "table.exclusionAuditReport", json!({ "sheet_id": "xy-aud" }));
    assert_eq!(xy_rep.payload["total_excluded"], 1);
    assert_eq!(xy_rep.payload["exclusions"][0]["target"], "x");

    let all_rep = ok_cmd(&mut aud_sess, "table.exclusionAuditReport", json!({}));
    assert_eq!(all_rep.payload["total_excluded"], 2);
}

#[test]
fn r2_s07_04_stats_run_and_update_params_forward_cox_ties_ref_levels_and_inner_variants() {
    let mut doc = ProjectDocument::new("Cox UpdateParams").unwrap();
    doc.add_data_sheet(seed_mv_survival_sheet("mv-cox")).unwrap();
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 2.5);
    }
    doc.add_data_sheet(DataSheet::new("xy-1", "XY", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "source_sheet_id": "mv-cox",
            "sheet_id": "res-cox",
            "kind": "cox_ph",
            "time_column": 0,
            "status_column": 1,
            "predictor_columns": [2],
            "reference_levels": [[2, "Control"]],
            "tie_method": "Breslow"
        }),
    );

    let res0 = session.document().result_sheet(&SheetId::new("res-cox")).unwrap();
    match &res0.analysis.kind {
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, reference_levels, .. }) => {
            assert_eq!(*tie_method, CoxTieMethod::Breslow);
            assert_eq!(reference_levels, &vec![(2_usize, String::from("Control"))]);
        }
        other => panic!("expected CoxProportionalHazards, got {other:?}"),
    }
    assert!(res0.tabs.iter().any(|t| t.footnotes.iter().any(|f| f.contains("Breslow"))));

    // analysis.updateParams forwards updated Cox tie_method & reference_levels.
    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({
            "sheet_id": "res-cox",
            "tie_method": "Efron",
            "reference_levels": [[2, "Treated"]]
        }),
    );
    let res1 = session.document().result_sheet(&SheetId::new("res-cox")).unwrap();
    match &res1.analysis.kind {
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, reference_levels, .. }) => {
            assert_eq!(*tie_method, CoxTieMethod::Efron);
            assert_eq!(reference_levels, &vec![(2_usize, String::from("Treated"))]);
        }
        other => panic!("expected CoxProportionalHazards, got {other:?}"),
    }
    assert!(res1.tabs.iter().any(|t| t.footnotes.iter().any(|f| f.contains("Efron"))));

    ok_cmd(
        &mut session,
        "analysis.updateParams",
        json!({
            "sheet_id": "res-cox",
            "tie_method": "Exact"
        }),
    );
    let res2 = session.document().result_sheet(&SheetId::new("res-cox")).unwrap();
    match &res2.analysis.kind {
        AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, .. }) => {
            assert_eq!(*tie_method, CoxTieMethod::Exact);
        }
        other => panic!("expected CoxProportionalHazards, got {other:?}"),
    }
    assert!(res2.tabs.iter().any(|t| t.footnotes.iter().any(|f| f.contains("Exact"))));

    // Inner-variant patching works for OutlierDetection (changing fdr_q_percent inside nested enum) and rejects malformed inner patch.
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy-1", "sheet_id": "res-out", "kind": "outliers" }));
    let before_out = session.document().result_sheet(&SheetId::new("res-out")).unwrap().clone();
    let bad_patch = session.execute("analysis.updateParams", json!({ "sheet_id": "res-out", "fdr_q_percent": "bad" })).unwrap_err();
    assert_eq!(bad_patch.code, ErrorCode::BadInput);
    assert_eq!(session.document().result_sheet(&SheetId::new("res-out")).unwrap().analysis, before_out.analysis);

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-out", "fdr_q_percent": 5.0 }));
    assert!(matches!(
        session.document().result_sheet(&SheetId::new("res-out")).unwrap().analysis.kind,
        AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) if (fdr_q_percent - 5.0).abs() < 1e-12
    ));
}

#[test]
fn r2_s07_05_checkpoint_validation_caps_cycles_and_undo_byte_budget() {
    let mut doc = ProjectDocument::new("Checkpoints Budget").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 10).unwrap();
    for r in 0..10 {
        col.data_sets[0].cells[r][0] = Cell::number(r as f64);
    }
    doc.add_data_sheet(DataSheet::new("col-cp", "Data", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    // 1. Nested checkpoints stripped on create & corrupt/cyclic snapshot rejected safely on restore.
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "cp-valid" }));
    let snap_val: Value = serde_json::from_str(&session.document().checkpoints[0].snapshot_json).unwrap();
    assert!(snap_val.get("checkpoints").is_none_or(|c| c.as_array().is_none_or(|a| a.is_empty())));

    let mut bad_doc = session.document().clone();
    bad_doc.checkpoints.push(curvebook_doc::NamedCheckpoint {
        id: String::from("cp-corrupt"),
        name: String::from("Corrupt"),
        note: String::new(),
        created_utc: String::from("unix:100"),
        snapshot_json: json!({ "schema_version": 9999 }).to_string(),
    });
    let mut bad_sess = Session::from_document(bad_doc);
    let err = bad_sess.execute("project.restoreCheckpoint", json!({ "id": "cp-corrupt" })).unwrap_err();
    assert!(matches!(err.code, ErrorCode::UnsupportedVersion | ErrorCode::BadInput | ErrorCode::CorruptData), "got {err}");
    assert_eq!(bad_sess.document().checkpoints.len(), 2, "failed restore must not wipe checkpoints");

    // 2. Cyclic analysis DAG inside restored checkpoint must be rejected/sanitized safely.
    let mut cyclic_snapshot = serde_json::to_value(session.document()).unwrap();
    let bad_spec = AnalysisSpec::new("cyc-res", "cyc-res", AnalysisKind::DescriptiveStats(Default::default())).unwrap();
    let bad_res = ResultSheet::new(bad_spec).unwrap();
    cyclic_snapshot["result_sheets"] = json!([bad_res]);
    let mut cyc_doc = session.document().clone();
    cyc_doc.checkpoints.push(curvebook_doc::NamedCheckpoint {
        id: String::from("cp-cyc"),
        name: String::from("Cyclic"),
        note: String::new(),
        created_utc: String::from("unix:101"),
        snapshot_json: cyclic_snapshot.to_string(),
    });
    let mut cyc_sess = Session::from_document(cyc_doc);
    let cyc_err = cyc_sess.execute("project.restoreCheckpoint", json!({ "id": "cp-cyc" })).unwrap_err();
    assert_eq!(cyc_err.code, ErrorCode::CycleDetected);

    // 3. Enforce MAX_CHECKPOINTS_PER_PROJECT = 32 cap.
    let mut cap_sess = Session::from_document(ProjectDocument::new("CapTest").unwrap());
    for i in 0..32 {
        ok_cmd(&mut cap_sess, "project.createCheckpoint", json!({ "name": format!("cp-{i}") }));
    }
    let cap_err = cap_sess.execute("project.createCheckpoint", json!({ "name": "cp-overflow" })).unwrap_err();
    assert_eq!(cap_err.code, ErrorCode::ResourceLimit);

    // 4. Undo byte budget (~16 MiB) evicts old bloated snapshots cleanly without OOM.
    let mut big_doc = ProjectDocument::new("UndoBytes").unwrap();
    let mut big_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 20).unwrap();
    big_col.data_sets[0].cells[0][0] = Cell::number(1.0);
    big_doc.add_data_sheet(DataSheet::new("d-big", "Big", TableArchetype::Column(big_col)).unwrap()).unwrap();
    let mut pad_doc = big_doc.clone();
    pad_doc.data_sheet_mut(&SheetId::new("d-big")).unwrap().set_cell(CellCoord::new(1, 0, 0), Cell::text("x".repeat(2 * 1024 * 1024))).unwrap();
    let big_payload = serde_json::to_string(&pad_doc).unwrap();
    for i in 0..3 {
        big_doc.checkpoints.push(curvebook_doc::NamedCheckpoint {
            id: format!("pad-{i}"),
            name: format!("Pad {i}"),
            note: String::new(),
            created_utc: String::from("unix:1"),
            snapshot_json: big_payload.clone(),
        });
    }
    let mut undo_sess = Session::from_document(big_doc);
    for r in 0..5 {
        ok_cmd(&mut undo_sess, "table.setCell", json!({ "sheet_id": "d-big", "row": 0, "col": 0, "value": r as f64 + 2.0 }));
    }
    assert!(
        undo_sess.undo_depth() <= 3,
        "undo byte budget (16 MiB) should have evicted older ~6 MiB snapshots, got depth={}",
        undo_sess.undo_depth()
    );
    assert!(undo_sess.undo_depth() >= 1);
}

#[test]
fn r2_s07_06_calculated_columns_protected_across_all_mv_mutations() {
    let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Label");
    mv.variables[1].role = VariableRole::Categorical;
    for r in 0..4 {
        mv.variables[0].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[1].cells[r] = Cell::text(if r < 2 { "Low" } else { "High" });
    }
    let mut doc = ProjectDocument::new("CalcGuard").unwrap();
    doc.add_data_sheet(DataSheet::new("mv-guard", "MV", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    let mut src_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4).unwrap();
    for r in 0..4 {
        src_col.data_sets[0].cells[r][0] = Cell::number(99.0);
    }
    doc.add_data_sheet(DataSheet::new("src-1", "Source", TableArchetype::Column(src_col)).unwrap()).unwrap();

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-guard", "title": "Scaled", "formula": "=Dose * 2" }));
    // Calculated column is at index 2.
    assert_eq!(
        session.execute("table.setVariableRole", json!({ "sheet_id": "mv-guard", "col": 2, "role": "Continuous" })).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        session.execute("table.setVariableRole", json!({ "sheet_id": "mv-guard", "col": 0, "role": "Calculated" })).unwrap_err().code,
        ErrorCode::BadInput
    );
    assert_eq!(
        session.execute("table.recodeLevels", json!({ "sheet_id": "mv-guard", "col": 2, "mappings": { "2": "Two" } })).unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(
        session
            .execute(
                "table.paste",
                json!({ "sheet_id": "mv-guard", "start_row": 0, "start_col": 2, "text": "Hijacked", "propagate_title_header": true })
            )
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        session
            .execute(
                "table.importCsv",
                json!({ "sheet_id": "mv-guard", "csv": "9\n9\n", "has_header": false, "replace": false, "anchor": { "row": 0, "col": 2, "subcol": 0 } })
            )
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(
        session.execute("table.pasteLink", json!({ "source_sheet_id": "src-1", "target_sheet_id": "mv-guard", "start_col": 2 })).unwrap_err().code,
        ErrorCode::Conflict
    );
}

#[test]
fn r2_s07_07_parts_of_whole_rejects_kappa_in_runtime_execution() {
    let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
    pow.data_sets[0].cells[0][0] = Cell::number(10.0);
    pow.data_sets[0].cells[1][0] = Cell::number(20.0);
    pow.data_sets[0].cells[2][0] = Cell::number(30.0);
    let mut doc = ProjectDocument::new("PowKappa").unwrap();
    doc.add_data_sheet(DataSheet::new("pow-k", "Parts", TableArchetype::PartsOfWhole(pow)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    for kind in ["cohen_kappa", "fleiss_kappa"] {
        let err = session.execute("stats.run", json!({ "source_sheet_id": "pow-k", "sheet_id": format!("res-{kind}"), "kind": kind })).unwrap_err();
        assert_eq!(err.code, ErrorCode::Conflict, "{kind} must be rejected on PartsOfWhole");
    }
}

#[test]
fn r2_s07_08_and_17_canvas_preset_overflow_guard_and_scales_point_overrides() {
    let mut doc = ProjectDocument::new("CanvasPreset").unwrap();
    let ds = seed_two_group_col("col-c");
    doc.add_data_sheet(ds).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-c", "data_sheet_id": "col-c", "kind": "column" }));
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g-c",
            "spec": {
                "canvas_size_pt": { "width": 500.0, "height": 400.0 },
                "point_overrides": [{
                    "series_index": 0,
                    "point_index": 0,
                    "size_pt": 10.0,
                    "border_width_pt": 2.0
                }]
            }
        }),
    );

    // R2-S07-17: ProportionalScale scales PointStyleOverride dimensions proportionally.
    let res = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-c", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    let w_new = res.payload["width_pt"].as_f64().unwrap();
    let h_new = res.payload["height_pt"].as_f64().unwrap();
    let expected_ratio = (w_new / 500.0).min(h_new / 400.0);
    let ov = &session.document().graph_sheet(&SheetId::new("g-c")).unwrap().spec.point_overrides[0];
    assert!((ov.size_pt.unwrap() - 10.0 * expected_ratio).abs() < 1e-6);
    assert!((ov.border_width_pt.unwrap() - 2.0 * expected_ratio).abs() < 1e-6);

    // R2-S07-08: Extreme coordinate causing f64 overflow during ProportionalScale returns BadInput without corrupting spec.
    let mut ovf_doc = session.document().clone();
    ovf_doc.graph_sheet_mut(&SheetId::new("g-c")).unwrap().spec.annotations.push(GraphAnnotation::Shape {
        kind: AnnotationShapeKind::Rectangle,
        bounds_pt: RectPt::new(1e308, 0.0, 10.0, 10.0),
        stroke_color: Rgba8::BLACK,
        stroke_width_pt: 1.0,
        dash: LineDashStyle::Solid,
        fill_color: None,
    });
    let mut ovf_sess = Session::from_document(ovf_doc);
    let err = ovf_sess
        .execute("graph.applyCanvasPreset", json!({ "sheet_id": "g-c", "preset": "DoubleColumn174mm", "resize_mode": "ProportionalScale" }))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::BadInput);
    assert!((ovf_sess.document().graph_sheet(&SheetId::new("g-c")).unwrap().spec.canvas_size_pt.width - w_new).abs() < 1e-6);
}

#[test]
fn r2_s07_09_graph_apply_style_from_honors_selective_toggles() {
    let mut doc = ProjectDocument::new("StyleSelect").unwrap();
    doc.add_data_sheet(seed_two_group_col("d1")).unwrap();
    doc.add_data_sheet(seed_two_group_col("d2")).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "d2", "kind": "column" }));
    ok_cmd(
        &mut session,
        "graph.setSpec",
        json!({
            "sheet_id": "g2",
            "spec": {
                "palette": "PaulTolVibrant",
                "frame_preset": "PlainBoxed",
                "x_axis": { "stroke_width_pt": 2.75, "break_ornament": "Zigzag" },
                "legend": { "visible": false, "bordered": true }
            }
        }),
    );

    ok_cmd(
        &mut session,
        "graph.applyStyleFrom",
        json!({
            "sheet_id": "g1",
            "donor_sheet_id": "g2",
            "clone_palette": false,
            "clone_frame_and_axes": true,
            "clone_error_bars_and_legend": false
        }),
    );
    let g1 = &session.document().graph_sheet(&SheetId::new("g1")).unwrap().spec;
    assert_eq!(g1.palette, PaletteId::OkabeIto);
    assert_eq!(g1.frame_preset, FramePreset::PlainBoxed);
    assert_eq!(g1.x_axis.break_ornament, BreakOrnament::Zigzag);
    assert!(!g1.legend.bordered && g1.legend.visible);
}

#[test]
fn r2_s07_10_power_analysis_spec_methods_report_ready() {
    let spec = PowerAnalysisSpec {
        family: PowerTestFamily::Correlation,
        solve_target: PowerSolveTarget::DetectableEffect,
        tail_mode: PowerTailMode::TwoSided,
        alpha: 0.05,
        target_power: Some(0.85),
        sample_size: Some(64.0),
        effect: PowerEffectSpec::Unspecified,
        ..PowerAnalysisSpec::default()
    };
    let text = describe_power_analysis_spec(&spec);
    assert!(text.contains("minimum detectable effect") && text.contains("Pearson correlation") && text.contains("0.85"), "{text}");
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!text.to_ascii_lowercase().contains(forbidden));
    }
}

#[test]
fn r2_s07_11_extract_result_tab_control_sanitization_ci_cells_and_locked_label() {
    let mut doc = ProjectDocument::new("ExtractTab").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-t")).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-t", "sheet_id": "res-t", "kind": "unpaired_t" }));

    ok_cmd(
        &mut session,
        "sheet.extractResultTab",
        json!({
            "result_sheet_id": "res-t",
            "sheet_id": "mv-ext",
            "title": "Clean\x07 Title",
            "archetype": "multiple_variables"
        }),
    );
    let ext = session.document().data_sheet(&SheetId::new("mv-ext")).unwrap();
    assert!(!ext.title.contains('\x07'));
    let TableArchetype::MultipleVariables(mv) = &ext.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables[0].role, VariableRole::Label);
    assert!(!mv.variables[0].role_inferred);
    // ConfidenceInterval cell populated as readable interval string instead of empty.
    assert!(mv.variables[0].cells.iter().any(|c| matches!(&c.value, curvebook_doc::CellValue::Text(s) if s.contains('[') && s.contains(','))));
    // Reactive upstream edit + recalculation preserves locked Label role and formatted CI cells.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-t", "row": 0, "col": 0, "value": 9.5 }));
    let TableArchetype::MultipleVariables(mv_after) = &session.document().data_sheet(&SheetId::new("mv-ext")).unwrap().archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv_after.variables[0].role, VariableRole::Label);
    assert!(!mv_after.variables[0].role_inferred);
    assert!(mv_after.variables[0].cells.iter().any(|c| matches!(&c.value, curvebook_doc::CellValue::Text(s) if s.contains('[') && s.contains(','))));

    // Control characters inside ResultTab.row_labels are sanitized into row_titles.
    let mut san_doc = session.document().clone();
    let res_mut = san_doc.result_sheet_mut(&SheetId::new("res-t")).unwrap();
    res_mut.tabs.push(ResultTab {
        id: String::from("ctrl-tab"),
        title: String::from("Ctrl Tab"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Val")],
        row_labels: vec![String::from("Row\x07\nOne")],
        rows: vec![vec![ResultCell::Number { value: 4.2, prefix: None, units: None }]],
        footnotes: Vec::new(),
    });
    let mut san_sess = Session::from_document(san_doc);
    ok_cmd(
        &mut san_sess,
        "sheet.extractResultTab",
        json!({ "result_sheet_id": "res-t", "tab_id": "ctrl-tab", "sheet_id": "col-san", "archetype": "column" }),
    );
    let TableArchetype::Column(col_san) = &san_sess.document().data_sheet(&SheetId::new("col-san")).unwrap().archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_san.row_titles[0], "Row One");
}

#[test]
fn r2_s07_13_categorical_order_reference_level_and_randomize_groups_name_alias() {
    let mut mv = MultipleVariablesTable::new(1, 3).unwrap();
    mv.variables[0].title = String::from("Group");
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].cells = vec![Cell::text("Low"), Cell::text("Med"), Cell::text("High")];
    let mut doc = ProjectDocument::new("CatRef").unwrap();
    doc.add_data_sheet(DataSheet::new("mv-cat", "Cat", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "table.setCategoricalOrder", json!({ "sheet_id": "mv-cat", "col": 0, "reference_level": "Med" }));
    let TableArchetype::MultipleVariables(updated) = &session.document().data_sheet(&SheetId::new("mv-cat")).unwrap().archetype else {
        panic!("expected MV");
    };
    assert_eq!(
        updated.variables[0].level_order.order,
        LevelOrderKind::Explicit(vec![String::from("Med"), String::from("Low"), String::from("High")])
    );

    ok_cmd(
        &mut session,
        "table.randomizeGroups",
        json!({
            "sheet_id": "rand-alias",
            "title": "Randomized",
            "groups": [{ "label": "Vehicle", "weight": 1 }, { "label": "Drug", "weight": 2 }],
            "subject_count": 6,
            "seed": 11
        }),
    );
    let rand_ds = session.document().data_sheet(&SheetId::new("rand-alias")).unwrap();
    assert_eq!(rand_ds.archetype.row_count(), 6);
}

#[test]
fn r2_s07_14_info_remove_constant_if_exists_noop() {
    let mut doc = ProjectDocument::new("InfoNoop").unwrap();
    doc.add_info_sheet(curvebook_doc::InfoSheet::new_preseeded("info-1", "Study Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    let out = ok_cmd(&mut session, "info.removeConstant", json!({ "sheet_id": "info-1", "name": "Missing", "if_exists": true }));
    assert!(!out.mutated);
    assert_eq!(out.payload["recalculated_sheets"], 0);
    assert!(!session.is_dirty());
    assert_eq!(session.undo_depth(), 0);
}

#[test]
fn r2_s07_15_graph_create_accepts_result_sheet() {
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 5).unwrap();
    for r in 0..5 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(2.0 * (r + 1) as f64 + 1.0);
    }
    let mut doc = ProjectDocument::new("GraphResult").unwrap();
    doc.add_data_sheet(DataSheet::new("xy-src", "XY", TableArchetype::Xy(xy)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "fit.run", json!({ "source_sheet_id": "xy-src", "sheet_id": "res-lin", "kind": "linear" }));

    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-from-res", "data_sheet_id": "res-lin" }));
    assert!(session.document().graph_sheet(&SheetId::new("g-from-res")).is_some());
}

#[test]
fn r2_s07_16_graph_set_spec_and_add_annotation_validate_strictly() {
    let mut doc = ProjectDocument::new("GraphValidate").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-gv")).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "col-gv", "sheet_id": "res-gv", "kind": "unpaired_t" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-v", "data_sheet_id": "col-gv", "kind": "column" }));

    let err_spec = session
        .execute(
            "graph.setSpec",
            json!({ "sheet_id": "g-v", "spec": { "point_overrides": [{ "series_index": 0, "point_index": 0, "size_pt": 0.0 }] } }),
        )
        .unwrap_err();
    assert_eq!(err_spec.code, ErrorCode::BadInput);

    let err_ann = session
        .execute(
            "graph.addAnnotation",
            json!({
                "sheet_id": "g-v",
                "annotation": GraphAnnotation::ModelEquation {
                    result_sheet_id: SheetId::new("res-gv"),
                    data_set_index: 0,
                    mode: EquationDisplayMode::Substituted,
                    bounds_pt: RectPt::new(10.0, 10.0, 100.0, 30.0),
                    font_size_pt: 0.0,
                    bordered: false,
                }
            }),
        )
        .unwrap_err();
    assert_eq!(err_ann.code, ErrorCode::BadInput);
    assert!(session.document().graph_sheet(&SheetId::new("g-v")).unwrap().spec.annotations.is_empty());
}

#[test]
fn r2_s07_18_random_sample_seed_validation_and_default_proportional() {
    let mut doc = ProjectDocument::new("RandomSampleSeed").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-rs")).unwrap();
    let mut session = Session::from_document(doc);

    let frac_err = session
        .execute("transform.run", json!({ "source_sheet_id": "col-rs", "sheet_id": "res-bad1", "kind": "random_sample", "seed": 1.5 }))
        .unwrap_err();
    assert_eq!(frac_err.code, ErrorCode::BadInput);

    let neg_err = session
        .execute("transform.run", json!({ "source_sheet_id": "col-rs", "sheet_id": "res-bad2", "kind": "random_sample", "seed": -3 }))
        .unwrap_err();
    assert_eq!(neg_err.code, ErrorCode::BadInput);

    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "source_sheet_id": "col-rs", "sheet_id": "res-rs", "kind": "random_sample", "sample_size": 3, "seed": 7 }),
    );
    match &session.document().result_sheet(&SheetId::new("res-rs")).unwrap().analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, proportional_allocation, .. }) => {
            assert_eq!(*seed, 7);
            assert!(*proportional_allocation, "proportional_allocation must default to true");
        }
        other => panic!("expected RandomSample, got {other:?}"),
    }
}

#[test]
fn r2_s12_02_path_policy_rejects_arbitrary_directories_without_curvebook_d() {
    let scratch = ScratchDir::new("dir-open-save");
    let plain_dir = scratch.root.join("plain_folder");
    fs::create_dir_all(&plain_dir).unwrap();

    let mut session = Session::new();
    let open_err = session.execute("project.open", json!({ "path": plain_dir.to_string_lossy() })).unwrap_err();
    assert_eq!(open_err.code, ErrorCode::BadInput, "arbitrary directory without .curvebook.d must be rejected on open: {open_err}");

    let save_err = session.execute("project.save", json!({ "path": plain_dir.to_string_lossy() })).unwrap_err();
    assert_eq!(save_err.code, ErrorCode::BadInput, "arbitrary directory without .curvebook.d / unpacked:true must be rejected on save: {save_err}");

    let unpacked_dir = scratch.root.join("valid_project.curvebook.d");
    ok_cmd(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(unpacked_dir.join("manifest.json").exists());
    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }));

    let legacy_dir = scratch.root.join("legacy_project.lumencraft.d");
    fs::create_dir_all(&legacy_dir).expect("create legacy dir");
    fs::copy(unpacked_dir.join("manifest.json"), legacy_dir.join("manifest.json")).expect("copy manifest");
    let legacy_open = reopened.execute("project.open", json!({ "path": legacy_dir.to_string_lossy() })).expect_err("reject legacy open");
    assert_eq!(legacy_open.code, ErrorCode::BadInput, "legacy .lumencraft.d directory must be rejected on open: {legacy_open}");
    let legacy_save = session.execute("project.save", json!({ "path": legacy_dir.to_string_lossy() })).expect_err("reject legacy save");
    assert_eq!(legacy_save.code, ErrorCode::BadInput, "legacy .lumencraft.d directory must be rejected on save: {legacy_save}");
}

#[test]
fn coord_357_format_selected_points_supports_optional_subcol() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    for r in 0..2 {
        for sc in 0..3 {
            grp.data_sets[0].cells[r][sc] = Cell::number(10.0 + sc as f64);
        }
    }
    let mut doc = ProjectDocument::new("SubcolPts").unwrap();
    doc.add_data_sheet(DataSheet::new("grp-pts", "Grouped", TableArchetype::Grouped(grp)).unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-pts", "data_sheet_id": "grp-pts", "kind": "grouped" }));

    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({
            "sheet_id": "g-pts",
            "points": [{ "series_index": 0, "point_index": 1, "subcol": 2 }],
            "color": "#D55E00",
            "size_pt": 9.0
        }),
    );
    let g = &session.document().graph_sheet(&SheetId::new("g-pts")).unwrap().spec;
    assert_eq!(g.point_override_with_subcol(0, 1, Some(2)).and_then(|o| o.size_pt), Some(9.0));
    assert!(g.point_override_with_subcol(0, 1, Some(0)).is_none(), "replicate 0 should not inherit subcol=2 override");
}

#[test]
fn r2_s05_01_table_paste_and_clear_prune_exclusion_notes() {
    let mut doc = ProjectDocument::new("PastePruneNotes").unwrap();
    doc.add_data_sheet(seed_two_group_col("col-pr")).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "table.excludeCellsWithReason", json!({ "sheet_id": "col-pr", "reason_tag": "outlier", "coords": [[0, 0], [1, 0]] }));
    assert_eq!(session.document().data_sheet(&SheetId::new("col-pr")).unwrap().exclusion_notes.len(), 2);

    // Overwriting [0, 0] via table.paste prunes its exclusion note while leaving [1, 0] intact.
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col-pr", "start_row": 0, "start_col": 0, "text": "42.0" }));
    let after_paste = session.document().data_sheet(&SheetId::new("col-pr")).unwrap();
    assert_eq!(after_paste.exclusion_notes.len(), 1);
    assert_eq!(after_paste.exclusion_notes[0].coord, CellCoord::new(1, 0, 0));

    // Clearing all cells via table.clearCells also prunes remaining exclusion notes.
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "col-pr", "all": true }));
    let after_clear = session.document().data_sheet(&SheetId::new("col-pr")).unwrap();
    assert!(after_clear.exclusion_notes.is_empty());
}

#[test]
fn r2_s07_10_power_analysis_runtime_dag_info_hooks_and_methods_report() {
    let mut doc = ProjectDocument::new("PowerIntegration").unwrap();
    doc.add_data_sheet(seed_two_group_col("src-p")).unwrap();
    doc.add_info_sheet(curvebook_doc::InfoSheet::new_preseeded(SheetId::new("info-p"), "Power Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-p", "name": "Alpha", "value": 0.05, "kind": "number" }));
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-p", "name": "Effect Size", "value": 0.5, "kind": "number" }));

    let p_spec = PowerAnalysisSpec {
        family: PowerTestFamily::TwoSampleUnpairedT,
        solve_target: PowerSolveTarget::SampleSize,
        tail_mode: PowerTailMode::TwoSided,
        alpha: 0.05,
        target_power: Some(0.80),
        effect: PowerEffectSpec::Standardized(0.5),
        info_sheet_id: Some(SheetId::new("info-p")),
        ..PowerAnalysisSpec::default()
    };
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "source_sheet_id": "src-p",
            "sheet_id": "res-pwr",
            "kind": { "PowerAnalysis": p_spec }
        }),
    );

    let rs = session.document().result_sheet(&SheetId::new("res-pwr")).unwrap();
    assert!(rs.analyzable_data.as_ref().map_or(0, |d| d.archetype.row_count()) >= 41);
    let n_at_05 = rs.scalars.iter().find(|s| s.key == "NPerGroup").map(|s| s.value).unwrap();
    assert!((n_at_05 - 64.0).abs() < 1e-9, "expected N=64 per group for d=0.5, alpha=0.05, power=0.80, got {n_at_05}");

    // Linked InfoSheet constant edit must reactively recompute the downstream PowerAnalysis ResultSheet.
    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-p", "name": "Alpha", "value": 0.01, "kind": "number" }));
    let rs_after_alpha = session.document().result_sheet(&SheetId::new("res-pwr")).unwrap();
    let n_at_01 = rs_after_alpha.scalars.iter().find(|s| s.key == "NPerGroup").map(|s| s.value).unwrap();
    assert!(n_at_01 > n_at_05, "tighter alpha=0.01 must raise required N (before={n_at_05}, after={n_at_01})");

    // Shorthand `power_analysis` creates a valid PowerAnalysis ResultSheet and methods report describes it.
    ok_cmd(
        &mut session,
        "analysis.run",
        json!({
            "source_sheet_id": "src-p",
            "sheet_id": "res-pwr-short",
            "kind": "power_analysis",
            "effect_size": 0.8,
            "alpha": 0.05
        }),
    );
    let report = generate_methods_report(session.document(), &SheetId::new("res-pwr"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("Unpaired t test") && report.contains("required sample size"), "{report}");

    // `.curvebook` archive roundtrip preserves PowerAnalysis ResultSheet + analyzable curve data.
    let dir = ScratchDir::new("power-archive");
    let archive_path = dir.root.join("power.curvebook");
    curvebook_format::save_project_file(&archive_path, session.document()).unwrap();
    let reloaded = curvebook_format::open_project_file(&archive_path).unwrap();
    let rs_reloaded = reloaded.result_sheet(&SheetId::new("res-pwr")).unwrap();
    assert!(matches!(rs_reloaded.analysis.kind, AnalysisKind::PowerAnalysis(_)));
    assert!(rs_reloaded.analyzable_data.as_ref().map_or(0, |d| d.archetype.row_count()) >= 41);
}
