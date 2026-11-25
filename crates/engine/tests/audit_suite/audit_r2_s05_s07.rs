//! Permanent Round 2 (`R0.19`) regression tests for Slice `S07` (`crates/engine`),
//! engine half of `R2-S12-02` (`path_policy`), and cross-shard `#357` (`subcol`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::sync::atomic::AtomicBool;

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, AnnotationShapeKind, BlandAltmanSpec, BreakOrnament, Cell, CellCoord, ColumnDesignHint, ConfidenceLevel,
    ContingencyMethod, ContingencySpec, CoxTieMethod, DataSheet, EquationDisplayMode, FramePreset, GraphAnnotation, InfoSheet, LevelOrderKind,
    NonlinearFitSpec, OutlierSpec, PartsOfWholeTable, PowerAnalysisSpec, PowerEffectSpec, PowerSolveTarget, PowerTailMode, PowerTestFamily,
    ProjectDocument, RectPt, ResultCell, ResultSheet, ResultTab, ResultTabKind, SheetId, SubcolumnMode, SurvivalAnalysisSpec, SurvivalTable,
    TableArchetype, TableKind, TransformAnalysisSpec, TwoGroupMethod, TwoGroupSpec, VariableRole, XyTable,
};
use curvebook_engine::methods_report::{ReportFormat, describe_power_analysis_spec, generate_methods_report};
use curvebook_engine::{Caller, ErrorCode, Session};
use curvebook_palette::{LineDashStyle, PaletteId, Rgba8};
use serde_json::{Value, json};

use super::common::{
    TempDir, assert_bad_input, assert_bad_inputs, assert_err_cases, col_sheet, col_sheet_hint, contingency_sheet, ds, grouped_rep_sheet, gs,
    mv_sheet, ok_cmd, rs, run_cmds, scalar, session_with, xy_sheet,
};

fn seed_two_group_col(id: &str) -> DataSheet {
    col_sheet(id, "TwoGroup", &[("Ctrl", &[10.0, 11.2, 9.8, 10.5, 11.0]), ("Drug", &[15.2, 16.0, 14.8, 17.1, 15.9])])
}

fn seed_three_group_col(id: &str) -> DataSheet {
    col_sheet_hint(
        id,
        "ThreeGroup",
        ColumnDesignHint::PairedRepeatedMeasures,
        &[("Baseline", &[10.0, 11.0, 12.0, 13.0, 14.0]), ("Low", &[14.0, 15.1, 16.2, 17.3, 18.4]), ("High", &[22.0, 23.2, 24.4, 25.6, 26.8])],
    )
}

fn seed_mv_survival_sheet(id: &str) -> DataSheet {
    mv_sheet(
        id,
        "CoxMV",
        &[],
        &[
            ("Time", None, &[Cell::number(4.0), Cell::number(6.0), Cell::number(8.0), Cell::number(5.0), Cell::number(10.0), Cell::number(12.0)]),
            ("Status", None, &[Cell::number(1.0), Cell::number(1.0), Cell::number(0.0), Cell::number(1.0), Cell::number(1.0), Cell::number(0.0)]),
            (
                "Arm",
                Some(VariableRole::Categorical),
                &[
                    Cell::text("Control"),
                    Cell::text("Control"),
                    Cell::text("Control"),
                    Cell::text("Treated"),
                    Cell::text("Treated"),
                    Cell::text("Treated"),
                ],
            ),
        ],
    )
}

#[test]
fn r2_s07_01_convert_archetype_preview_validation_and_undo() {
    let mut session = session_with("Convert Policy", vec![seed_two_group_col("col-1")]);
    let cancel = AtomicBool::new(false);

    let preview = session
        .execute_with_caller(
            "table.convertArchetype",
            json!({ "sheet_id": "col-1", "target_kind": "Grouped", "preview_only": true }),
            Caller::Mcp,
            &cancel,
        )
        .unwrap();
    assert!(!preview.mutated);

    for caller in [Caller::Mcp, Caller::Ui] {
        let err = session
            .execute_with_caller(
                "table.convertArchetype",
                json!({ "sheet_id": "col-1", "target_kind": "Grouped", "preview_only": "yes" }),
                caller,
                &cancel,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput);
    }

    let committed =
        session.execute_with_caller("table.convertArchetype", json!({ "sheet_id": "col-1", "target_kind": "Grouped" }), Caller::Ui, &cancel).unwrap();
    assert!(committed.mutated && session.is_dirty() && session.undo_depth() == 1);
    assert_eq!(ds(&session, "col-1").kind(), TableKind::Grouped);
    assert!(session.undo().unwrap());
    assert_eq!(ds(&session, "col-1").kind(), TableKind::Column);
}

#[test]
fn r2_s07_03_and_12_methods_report_accuracy_latex_safety_and_audit_parity() {
    let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
    for (r, v) in [20.0, 30.0, 50.0].into_iter().enumerate() {
        pow.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let mut session = session_with(
        "Methods Accuracy",
        vec![
            seed_two_group_col("col-2g"),
            seed_three_group_col("col-3g"),
            contingency_sheet("cont-3x3", "3x3 Table", &[], &[], &[&[5.0, 6.0, 7.0], &[8.0, 9.0, 10.0], &[11.0, 12.0, 13.0]]),
            DataSheet::new("pow-1", "Parts", TableArchetype::PartsOfWhole(pow)).unwrap(),
            xy_sheet("xy-tr", "Dose", "X", &[1.0, 2.0, 3.0, 4.0], &[("Y", &[20.0, 30.0, 40.0, 50.0])]),
        ],
    );

    // 1. Nonparametric KS/MW, Bland-Altman 99%, Contingency 3x3 & PartsOfWhole report accuracy.
    let mut tmp_doc = session.document().clone();
    let mut ba_spec = AnalysisSpec::new("res-ba", "col-2g", AnalysisKind::BlandAltman(BlandAltmanSpec::default())).unwrap();
    ba_spec.confidence_level = ConfidenceLevel::from_percent(99.0).unwrap();
    for spec in [
        AnalysisSpec::new(
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
        .unwrap(),
        AnalysisSpec::new(
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
        .unwrap(),
        ba_spec,
        AnalysisSpec::new(
            "res-c3",
            "cont-3x3",
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::FisherExact,
                compute_odds_and_risk_ratios: true,
                compute_sensitivity_specificity: true,
                ..ContingencySpec::default()
            }),
        )
        .unwrap(),
        AnalysisSpec::new(
            "res-pow",
            "pow-1",
            AnalysisKind::Contingency(ContingencySpec {
                method: ContingencyMethod::PearsonChiSquare { yates_continuity: false },
                compute_odds_and_risk_ratios: true,
                ..ContingencySpec::default()
            }),
        )
        .unwrap(),
    ] {
        tmp_doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();
    }
    let rep_of = |d: &ProjectDocument, id: &str| generate_methods_report(d, &SheetId::new(id), ReportFormat::PlainText).unwrap();
    let ks_txt = rep_of(&tmp_doc, "res-ks");
    assert!(ks_txt.contains("two-tailed") && !ks_txt.contains("Cohen's d") && !ks_txt.contains("verbatim without numerical rounding overrides"));
    assert!(!rep_of(&tmp_doc, "res-mw").contains("Cohen's d"));
    let ba_rep = rep_of(&tmp_doc, "res-ba");
    assert!(ba_rep.contains("99% limits of agreement") && !ba_rep.contains("95% limits"));
    let c3_rep = rep_of(&tmp_doc, "res-c3");
    assert!(!c3_rep.contains("E >= 5") && !c3_rep.contains("odds ratio") && !c3_rep.contains("Diagnostic accuracy"));
    let pow_rep = rep_of(&tmp_doc, "res-pow");
    assert!(pow_rep.contains("goodness-of-fit") && !pow_rep.contains("odds ratio"));

    // 2. One-way ANOVA coercions & transform axis labels.
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "kw-tuk", "kind": "kruskal_wallis", "multiplicity": "tukey" })),
            ("stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "bf-tuk", "kind": "welch_anova", "multiplicity": "tukey" })),
            ("stats.run", json!({ "source_sheet_id": "col-3g", "sheet_id": "fr-tuk", "kind": "friedman", "multiplicity": "tukey" })),
            (
                "transform.run",
                json!({ "source_sheet_id": "xy-tr", "sheet_id": "tr-xy", "kind": "math", "x_transform": "ln", "y_transform": "log10" }),
            ),
        ],
    );
    let kw_rep = rep_of(session.document(), "kw-tuk");
    assert!(kw_rep.contains("Dunn's nonparametric rank-sum") && kw_rep.contains("Bonferroni") && !kw_rep.contains("Tukey"));
    let bf_rep = rep_of(session.document(), "bf-tuk");
    assert!(bf_rep.contains("Games-Howell") && !bf_rep.contains("Tukey") && !bf_rep.contains("eta-squared"));
    let fr_rep = rep_of(session.document(), "fr-tuk");
    assert!(fr_rep.contains("Post-hoc pairwise comparisons were not computed") && !fr_rep.contains("Tukey"));
    assert!(rep_of(session.document(), "tr-xy").contains("X: ln(X); Y: log10(Y)"));

    // 3. R2-S07-12 LaTeX safety (`custom_dsl`, `[`-prefixed rows, orphaned notes).
    let mut tex_doc = session.document().clone();
    tex_doc.data_sheet_mut(&SheetId::new("xy-tr")).unwrap().exclusion_notes.push(curvebook_doc::CellExclusionNote {
        coord: CellCoord::new(0, 0, 0),
        reason_tag: String::from("orphan"),
        note: String::from("cleared previously"),
        timestamp_utc: String::from("2026-11-16T00:00:00Z"),
    });
    let mut nfit = NonlinearFitSpec::new("custom_dsl");
    nfit.custom_dsl = Some(String::from("Y = a_1 & α\n+ b"));
    let mut fit_res = ResultSheet::new(AnalysisSpec::new("res-tex", "xy-tr", AnalysisKind::NonlinearRegression(nfit)).unwrap()).unwrap();
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
    assert!(!tex_out.contains("\\\\\\relax") && tex_out.contains("{[}Agonist]") && tex_out.contains(" \\\\\n\\hline") && tex_out.contains("\\mbox{"));
    assert!(!tex_out.contains("(1 with recorded audit justification note"));

    // 4. R2-S07-12 table.exclusionAuditReport covers Xy.x_cells and Survival.times.
    let mut xy_aud = XyTable::new(SubcolumnMode::RawReplicates(1), 1, 2).unwrap();
    xy_aud.x_cells[0][0] = Cell { value: curvebook_doc::CellValue::Number(1.0), excluded: true };
    xy_aud.data_sets[0].cells[0][0] = Cell::number(2.0);
    let mut srv_aud = SurvivalTable::new(1, 2).unwrap();
    srv_aud.times[1] = Cell { value: curvebook_doc::CellValue::Number(5.0), excluded: true };
    srv_aud.cohorts[0].cells[0][0] = Cell::number(1.0);
    srv_aud.cohorts[0].cells[1][0] = Cell::number(1.0);
    let mut aud_sess = session_with(
        "AuditParity",
        vec![
            DataSheet::new("xy-aud", "XY Audit", TableArchetype::Xy(xy_aud)).unwrap(),
            DataSheet::new("srv-aud", "Surv Audit", TableArchetype::Survival(srv_aud)).unwrap(),
        ],
    );
    let xy_rep = ok_cmd(&mut aud_sess, "table.exclusionAuditReport", json!({ "sheet_id": "xy-aud" }));
    assert_eq!((xy_rep.payload["total_excluded"].as_u64(), xy_rep.payload["exclusions"][0]["target"].as_str()), (Some(1), Some("x")));
    assert_eq!(ok_cmd(&mut aud_sess, "table.exclusionAuditReport", json!({})).payload["total_excluded"], 2);
}

#[test]
fn r2_s07_04_stats_run_and_update_params_forward_cox_ties_ref_levels_and_inner_variants() {
    let mut session = session_with(
        "Cox UpdateParams",
        vec![seed_mv_survival_sheet("mv-cox"), xy_sheet("xy-1", "XY", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y", &[2.5, 5.0, 7.5, 10.0, 12.5])])],
    );
    ok_cmd(
        &mut session,
        "stats.run",
        json!({ "source_sheet_id": "mv-cox", "sheet_id": "res-cox", "kind": "cox_ph", "time_column": 0, "status_column": 1, "predictor_columns": [2], "reference_levels": [[2, "Control"]], "tie_method": "Breslow" }),
    );
    let check_cox = |s: &Session, exp_tie: CoxTieMethod, exp_ref: &str, foot: &str| {
        let r = rs(s, "res-cox");
        assert!(
            matches!(&r.analysis.kind, AnalysisKind::Survival(SurvivalAnalysisSpec::CoxProportionalHazards { tie_method, reference_levels, .. }) if *tie_method == exp_tie && reference_levels == &vec![(2_usize, exp_ref.to_owned())])
        );
        assert!(r.tabs.iter().any(|t| t.footnotes.iter().any(|f| f.contains(foot))));
    };
    check_cox(&session, CoxTieMethod::Breslow, "Control", "Breslow");

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-cox", "tie_method": "Efron", "reference_levels": [[2, "Treated"]] }));
    check_cox(&session, CoxTieMethod::Efron, "Treated", "Efron");

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-cox", "tie_method": "Exact" }));
    check_cox(&session, CoxTieMethod::Exact, "Treated", "Exact");

    // Inner-variant patching for OutlierDetection and rollback on malformed inner patch.
    ok_cmd(&mut session, "stats.run", json!({ "source_sheet_id": "xy-1", "sheet_id": "res-out", "kind": "outliers" }));
    let before_out = rs(&session, "res-out").analysis.clone();
    assert_bad_input(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-out", "fdr_q_percent": "bad" }));
    assert_eq!(rs(&session, "res-out").analysis, before_out);

    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "res-out", "fdr_q_percent": 5.0 }));
    assert!(
        matches!(rs(&session, "res-out").analysis.kind, AnalysisKind::OutlierDetection(OutlierSpec::Rout { fdr_q_percent }) if (fdr_q_percent - 5.0).abs() < 1e-12)
    );
}

#[test]
fn r2_s07_05_checkpoint_validation_caps_cycles_and_undo_byte_budget() {
    let mut session =
        session_with("Checkpoints Budget", vec![col_sheet("col-cp", "Data", &[("A", &(0..10).map(|r| r as f64).collect::<Vec<_>>()), ("B", &[])])]);

    // 1. Nested checkpoints stripped & corrupt/cyclic snapshots rejected safely.
    ok_cmd(&mut session, "project.createCheckpoint", json!({ "name": "cp-valid" }));
    let snap_val: Value = serde_json::from_str(&session.document().checkpoints[0].snapshot_json).unwrap();
    assert!(snap_val.get("checkpoints").is_none_or(|c| c.as_array().is_none_or(Vec::is_empty)));

    let push_cp = |base: &ProjectDocument, id: &str, name: &str, snap: String| {
        let mut d = base.clone();
        d.checkpoints.push(curvebook_doc::NamedCheckpoint {
            id: id.to_owned(),
            name: name.to_owned(),
            note: String::new(),
            created_utc: String::from("unix:100"),
            snapshot_json: snap,
        });
        Session::from_document(d)
    };
    let mut bad_sess = push_cp(session.document(), "cp-corrupt", "Corrupt", json!({ "schema_version": 9999 }).to_string());
    let err = bad_sess.execute("project.restoreCheckpoint", json!({ "id": "cp-corrupt" })).unwrap_err();
    assert!(
        matches!(err.code, ErrorCode::UnsupportedVersion | ErrorCode::BadInput | ErrorCode::CorruptData)
            && bad_sess.document().checkpoints.len() == 2
    );

    let mut cyclic_snap = serde_json::to_value(session.document()).unwrap();
    cyclic_snap["result_sheets"] =
        json!([ResultSheet::new(AnalysisSpec::new("cyc-res", "cyc-res", AnalysisKind::DescriptiveStats(Default::default())).unwrap()).unwrap()]);
    let mut cyc_sess = push_cp(session.document(), "cp-cyc", "Cyclic", cyclic_snap.to_string());
    assert_eq!(cyc_sess.execute("project.restoreCheckpoint", json!({ "id": "cp-cyc" })).unwrap_err().code, ErrorCode::CycleDetected);

    // 2. Enforce MAX_CHECKPOINTS_PER_PROJECT = 32 cap.
    let mut cap_sess = Session::from_document(ProjectDocument::new("CapTest").unwrap());
    for i in 0..32 {
        ok_cmd(&mut cap_sess, "project.createCheckpoint", json!({ "name": format!("cp-{i}") }));
    }
    assert_eq!(cap_sess.execute("project.createCheckpoint", json!({ "name": "cp-overflow" })).unwrap_err().code, ErrorCode::ResourceLimit);

    // 3. Undo byte budget (~16 MiB) evicts old bloated snapshots cleanly without OOM.
    let mut big_doc = ProjectDocument::new("UndoBytes").unwrap();
    big_doc.add_data_sheet(col_sheet("d-big", "Big", &[("A", &[1.0; 20])])).unwrap();
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
    assert!((1..=3).contains(&undo_sess.undo_depth()), "unexpected undo_depth={}", undo_sess.undo_depth());
}

#[test]
fn r2_s07_06_calculated_columns_protected_across_all_mv_mutations() {
    let mv = mv_sheet(
        "mv-guard",
        "MV",
        &[],
        &[
            ("Dose", None, &[Cell::number(1.0), Cell::number(2.0), Cell::number(3.0), Cell::number(4.0)]),
            ("Label", Some(VariableRole::Categorical), &[Cell::text("Low"), Cell::text("Low"), Cell::text("High"), Cell::text("High")]),
        ],
    );
    let mut session = session_with("CalcGuard", vec![mv, col_sheet("src-1", "Source", &[("S", &[99.0; 4])])]);
    ok_cmd(&mut session, "table.setCalculatedVariable", json!({ "sheet_id": "mv-guard", "title": "Scaled", "formula": "=Dose * 2" }));
    assert_err_cases(
        &mut session,
        &[
            ("table.setVariableRole", json!({ "sheet_id": "mv-guard", "col": 2, "role": "Continuous" }), ErrorCode::Conflict),
            ("table.setVariableRole", json!({ "sheet_id": "mv-guard", "col": 0, "role": "Calculated" }), ErrorCode::BadInput),
            ("table.recodeLevels", json!({ "sheet_id": "mv-guard", "col": 2, "mappings": { "2": "Two" } }), ErrorCode::Conflict),
            (
                "table.paste",
                json!({ "sheet_id": "mv-guard", "start_row": 0, "start_col": 2, "text": "Hijacked", "propagate_title_header": true }),
                ErrorCode::Conflict,
            ),
            (
                "table.importCsv",
                json!({ "sheet_id": "mv-guard", "csv": "9\n9\n", "has_header": false, "replace": false, "anchor": { "row": 0, "col": 2, "subcol": 0 } }),
                ErrorCode::Conflict,
            ),
            ("table.pasteLink", json!({ "source_sheet_id": "src-1", "target_sheet_id": "mv-guard", "start_col": 2 }), ErrorCode::Conflict),
        ],
    );
}

#[test]
fn r2_s07_07_parts_of_whole_rejects_kappa_in_runtime_execution() {
    let mut pow = PartsOfWholeTable::new(1, 3).unwrap();
    for (r, v) in [10.0, 20.0, 30.0].into_iter().enumerate() {
        pow.data_sets[0].cells[r][0] = Cell::number(v);
    }
    let mut session = session_with("PowKappa", vec![DataSheet::new("pow-k", "Parts", TableArchetype::PartsOfWhole(pow)).unwrap()]);
    assert_err_cases(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "pow-k", "sheet_id": "res-cohen_kappa", "kind": "cohen_kappa" }), ErrorCode::Conflict),
            ("stats.run", json!({ "source_sheet_id": "pow-k", "sheet_id": "res-fleiss_kappa", "kind": "fleiss_kappa" }), ErrorCode::Conflict),
        ],
    );
}

#[test]
fn r2_s07_08_and_17_canvas_preset_overflow_guard_and_scales_point_overrides() {
    let mut session = session_with("CanvasPreset", vec![seed_two_group_col("col-c")]);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-c", "data_sheet_id": "col-c", "kind": "column" })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g-c", "spec": { "canvas_size_pt": { "width": 500.0, "height": 400.0 }, "point_overrides": [{ "series_index": 0, "point_index": 0, "size_pt": 10.0, "border_width_pt": 2.0 }] } }),
            ),
        ],
    );
    let res = ok_cmd(
        &mut session,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-c", "preset": "SingleColumn88mm", "resize_mode": "ProportionalScale" }),
    );
    let (w_new, h_new) = (res.payload["width_pt"].as_f64().unwrap(), res.payload["height_pt"].as_f64().unwrap());
    let ratio = (w_new / 500.0).min(h_new / 400.0);
    let ov = &gs(&session, "g-c").spec.point_overrides[0];
    assert!((ov.size_pt.unwrap() - 10.0 * ratio).abs() < 1e-6 && (ov.border_width_pt.unwrap() - 2.0 * ratio).abs() < 1e-6);

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
    assert_bad_input(
        &mut ovf_sess,
        "graph.applyCanvasPreset",
        json!({ "sheet_id": "g-c", "preset": "DoubleColumn174mm", "resize_mode": "ProportionalScale" }),
    );
    assert!((gs(&ovf_sess, "g-c").spec.canvas_size_pt.width - w_new).abs() < 1e-6);
}

#[test]
fn r2_s07_09_graph_apply_style_from_honors_selective_toggles() {
    let mut session = session_with("StyleSelect", vec![seed_two_group_col("d1"), seed_two_group_col("d2")]);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g1", "data_sheet_id": "d1", "kind": "column" })),
            ("graph.create", json!({ "sheet_id": "g2", "data_sheet_id": "d2", "kind": "column" })),
            (
                "graph.setSpec",
                json!({ "sheet_id": "g2", "spec": { "palette": "PaulTolVibrant", "frame_preset": "PlainBoxed", "x_axis": { "stroke_width_pt": 2.75, "break_ornament": "Zigzag" }, "legend": { "visible": false, "bordered": true } } }),
            ),
            (
                "graph.applyStyleFrom",
                json!({ "sheet_id": "g1", "donor_sheet_id": "g2", "clone_palette": false, "clone_frame_and_axes": true, "clone_error_bars_and_legend": false }),
            ),
        ],
    );
    let g1 = &gs(&session, "g1").spec;
    assert_eq!((g1.palette, g1.frame_preset, g1.x_axis.break_ornament), (PaletteId::OkabeIto, FramePreset::PlainBoxed, BreakOrnament::Zigzag));
    assert!(!g1.legend.bordered && g1.legend.visible);
}

#[test]
fn r2_s07_10_power_analysis_spec_methods_report_ready() {
    let text = describe_power_analysis_spec(&PowerAnalysisSpec {
        family: PowerTestFamily::Correlation,
        solve_target: PowerSolveTarget::DetectableEffect,
        tail_mode: PowerTailMode::TwoSided,
        alpha: 0.05,
        target_power: Some(0.85),
        sample_size: Some(64.0),
        effect: PowerEffectSpec::Unspecified,
        ..PowerAnalysisSpec::default()
    });
    assert!(text.contains("minimum detectable effect") && text.contains("Pearson correlation") && text.contains("0.85"));
    for forbidden in ["graphpad", "prism", "dotmatics"] {
        assert!(!text.to_ascii_lowercase().contains(forbidden));
    }
}

#[test]
fn r2_s07_11_extract_result_tab_control_sanitization_ci_cells_and_locked_label() {
    let mut session = session_with("ExtractTab", vec![seed_two_group_col("col-t")]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col-t", "sheet_id": "res-t", "kind": "unpaired_t" })),
            (
                "sheet.extractResultTab",
                json!({ "result_sheet_id": "res-t", "sheet_id": "mv-ext", "title": "Clean\x07 Title", "archetype": "multiple_variables" }),
            ),
        ],
    );
    assert!(!ds(&session, "mv-ext").title.contains('\x07'));
    let check_mv = |s: &Session| {
        let TableArchetype::MultipleVariables(mv) = &ds(s, "mv-ext").archetype else { panic!("expected MV") };
        assert_eq!((mv.variables[0].role, mv.variables[0].role_inferred), (VariableRole::Label, false));
        assert!(mv.variables[0].cells.iter().any(|c| matches!(&c.value, curvebook_doc::CellValue::Text(st) if st.contains('[') && st.contains(','))));
    };
    check_mv(&session);
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-t", "row": 0, "col": 0, "value": 9.5 }));
    check_mv(&session);

    let mut san_doc = session.document().clone();
    san_doc.result_sheet_mut(&SheetId::new("res-t")).unwrap().tabs.push(ResultTab {
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
    let TableArchetype::Column(col_san) = &ds(&san_sess, "col-san").archetype else { panic!("expected Column") };
    assert_eq!(col_san.row_titles[0], "Row One");
}

#[test]
fn r2_s07_13_categorical_order_reference_level_and_randomize_groups_name_alias() {
    let mut session = session_with(
        "CatRef",
        vec![mv_sheet(
            "mv-cat",
            "Cat",
            &[],
            &[("Group", Some(VariableRole::Categorical), &[Cell::text("Low"), Cell::text("Med"), Cell::text("High")])],
        )],
    );
    run_cmds(
        &mut session,
        &[
            ("table.setCategoricalOrder", json!({ "sheet_id": "mv-cat", "col": 0, "reference_level": "Med" })),
            (
                "table.randomizeGroups",
                json!({ "sheet_id": "rand-alias", "title": "Randomized", "groups": [{ "label": "Vehicle", "weight": 1 }, { "label": "Drug", "weight": 2 }], "subject_count": 6, "seed": 11 }),
            ),
        ],
    );
    let TableArchetype::MultipleVariables(updated) = &ds(&session, "mv-cat").archetype else { panic!("expected MV") };
    assert_eq!(
        updated.variables[0].level_order.order,
        LevelOrderKind::Explicit(vec![String::from("Med"), String::from("Low"), String::from("High")])
    );
    assert_eq!(ds(&session, "rand-alias").archetype.row_count(), 6);
}

#[test]
fn r2_s07_14_info_remove_constant_if_exists_noop() {
    let mut doc = ProjectDocument::new("InfoNoop").unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded("info-1", "Study Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);
    let out = ok_cmd(&mut session, "info.removeConstant", json!({ "sheet_id": "info-1", "name": "Missing", "if_exists": true }));
    assert!(!out.mutated && out.payload["recalculated_sheets"] == 0 && !session.is_dirty() && session.undo_depth() == 0);
}

#[test]
fn r2_s07_15_graph_create_accepts_result_sheet() {
    let mut session =
        session_with("GraphResult", vec![xy_sheet("xy-src", "XY", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y", &[3.5, 6.0, 8.5, 11.0, 13.5])])]);
    run_cmds(
        &mut session,
        &[
            ("fit.run", json!({ "source_sheet_id": "xy-src", "sheet_id": "res-lin", "kind": "linear" })),
            ("graph.create", json!({ "sheet_id": "g-from-res", "data_sheet_id": "res-lin" })),
        ],
    );
    assert!(session.document().graph_sheet(&SheetId::new("g-from-res")).is_some());
}

#[test]
fn r2_s07_16_graph_set_spec_and_add_annotation_validate_strictly() {
    let mut session = session_with("GraphValidate", vec![seed_two_group_col("col-gv")]);
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col-gv", "sheet_id": "res-gv", "kind": "unpaired_t" })),
            ("graph.create", json!({ "sheet_id": "g-v", "data_sheet_id": "col-gv", "kind": "column" })),
        ],
    );
    assert_bad_inputs(
        &mut session,
        &[
            ("graph.setSpec", json!({ "sheet_id": "g-v", "spec": { "point_overrides": [{ "series_index": 0, "point_index": 0, "size_pt": 0.0 }] } })),
            (
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
            ),
        ],
    );
    assert!(gs(&session, "g-v").spec.annotations.is_empty());
}

#[test]
fn r2_s07_18_random_sample_seed_validation_and_default_proportional() {
    let mut session = session_with("RandomSampleSeed", vec![seed_two_group_col("col-rs")]);
    assert_bad_inputs(
        &mut session,
        &[
            ("transform.run", json!({ "source_sheet_id": "col-rs", "sheet_id": "res-bad1", "kind": "random_sample", "seed": 1.5 })),
            ("transform.run", json!({ "source_sheet_id": "col-rs", "sheet_id": "res-bad2", "kind": "random_sample", "seed": -3 })),
        ],
    );
    ok_cmd(
        &mut session,
        "transform.run",
        json!({ "source_sheet_id": "col-rs", "sheet_id": "res-rs", "kind": "random_sample", "sample_size": 3, "seed": 7 }),
    );
    assert!(matches!(
        rs(&session, "res-rs").analysis.kind,
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed: 7, proportional_allocation: true, .. })
    ));
}

#[test]
fn r2_s12_02_path_policy_rejects_arbitrary_directories_without_curvebook_d() {
    let scratch = TempDir::new("dir-open-save");
    let plain_dir = scratch.child("plain_folder");
    fs::create_dir_all(&plain_dir).unwrap();

    let mut session = Session::new();
    assert_bad_inputs(
        &mut session,
        &[("project.open", json!({ "path": plain_dir.to_string_lossy() })), ("project.save", json!({ "path": plain_dir.to_string_lossy() }))],
    );

    let unpacked_dir = scratch.child("valid_project.curvebook.d");
    ok_cmd(&mut session, "project.save", json!({ "path": unpacked_dir.to_string_lossy() }));
    assert!(unpacked_dir.join("manifest.json").exists());
    let mut reopened = Session::new();
    ok_cmd(&mut reopened, "project.open", json!({ "path": unpacked_dir.to_string_lossy() }));

    let legacy_dir = scratch.child("legacy_project.lumencraft.d");
    fs::create_dir_all(&legacy_dir).unwrap();
    fs::copy(unpacked_dir.join("manifest.json"), legacy_dir.join("manifest.json")).unwrap();
    assert_bad_input(&mut reopened, "project.open", json!({ "path": legacy_dir.to_string_lossy() }));
    assert_bad_input(&mut session, "project.save", json!({ "path": legacy_dir.to_string_lossy() }));
}

#[test]
fn coord_357_format_selected_points_supports_optional_subcol() {
    let grp = grouped_rep_sheet(
        "grp-pts",
        "Grouped",
        &["R0", "R1"],
        &[("A", &[&[10.0, 11.0, 12.0], &[10.0, 11.0, 12.0]]), ("B", &[&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]])],
    );
    let mut session = session_with("SubcolPts", vec![grp]);
    run_cmds(
        &mut session,
        &[
            ("graph.create", json!({ "sheet_id": "g-pts", "data_sheet_id": "grp-pts", "kind": "grouped" })),
            (
                "graph.formatSelectedPoints",
                json!({ "sheet_id": "g-pts", "points": [{ "series_index": 0, "point_index": 1, "subcol": 2 }], "color": "#D55E00", "size_pt": 9.0 }),
            ),
        ],
    );
    let g = &gs(&session, "g-pts").spec;
    assert_eq!(g.point_override_with_subcol(0, 1, Some(2)).and_then(|o| o.size_pt), Some(9.0));
    assert!(g.point_override_with_subcol(0, 1, Some(0)).is_none());
}

#[test]
fn r2_s05_01_table_paste_and_clear_prune_exclusion_notes() {
    let mut session = session_with("PastePruneNotes", vec![seed_two_group_col("col-pr")]);
    ok_cmd(&mut session, "table.excludeCellsWithReason", json!({ "sheet_id": "col-pr", "reason_tag": "outlier", "coords": [[0, 0], [1, 0]] }));
    assert_eq!(ds(&session, "col-pr").exclusion_notes.len(), 2);
    ok_cmd(&mut session, "table.paste", json!({ "sheet_id": "col-pr", "start_row": 0, "start_col": 0, "text": "42.0" }));
    assert_eq!((ds(&session, "col-pr").exclusion_notes.len(), ds(&session, "col-pr").exclusion_notes[0].coord), (1, CellCoord::new(1, 0, 0)));
    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "col-pr", "all": true }));
    assert!(ds(&session, "col-pr").exclusion_notes.is_empty());
}

#[test]
fn r2_s07_10_power_analysis_runtime_dag_info_hooks_and_methods_report() {
    let mut doc = ProjectDocument::new("PowerIntegration").unwrap();
    doc.add_data_sheet(seed_two_group_col("src-p")).unwrap();
    doc.add_info_sheet(InfoSheet::new_preseeded(SheetId::new("info-p"), "Power Info").unwrap()).unwrap();
    let mut session = Session::from_document(doc);

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
    run_cmds(
        &mut session,
        &[
            ("info.setConstant", json!({ "sheet_id": "info-p", "name": "Alpha", "value": 0.05, "kind": "number" })),
            ("info.setConstant", json!({ "sheet_id": "info-p", "name": "Effect Size", "value": 0.5, "kind": "number" })),
            ("analysis.run", json!({ "source_sheet_id": "src-p", "sheet_id": "res-pwr", "kind": { "PowerAnalysis": p_spec } })),
        ],
    );
    assert!(rs(&session, "res-pwr").analyzable_data.as_ref().map_or(0, |d| d.archetype.row_count()) >= 41);
    let n_at_05 = scalar(&session, "res-pwr", "NPerGroup");
    assert!((n_at_05 - 64.0).abs() < 1e-9);

    ok_cmd(&mut session, "info.setConstant", json!({ "sheet_id": "info-p", "name": "Alpha", "value": 0.01, "kind": "number" }));
    assert!(scalar(&session, "res-pwr", "NPerGroup") > n_at_05);

    ok_cmd(
        &mut session,
        "analysis.run",
        json!({ "source_sheet_id": "src-p", "sheet_id": "res-pwr-short", "kind": "power_analysis", "effect_size": 0.8, "alpha": 0.05 }),
    );
    let report = generate_methods_report(session.document(), &SheetId::new("res-pwr"), ReportFormat::PlainText).unwrap();
    assert!(report.contains("Unpaired t test") && report.contains("required sample size"));

    let dir = TempDir::new("power-archive");
    let archive_path = dir.child("power.curvebook");
    curvebook_format::save_project_file(&archive_path, session.document()).unwrap();
    let reloaded = curvebook_format::open_project_file(&archive_path).unwrap();
    let rs_reloaded = reloaded.result_sheet(&SheetId::new("res-pwr")).unwrap();
    assert!(
        matches!(rs_reloaded.analysis.kind, AnalysisKind::PowerAnalysis(_))
            && rs_reloaded.analyzable_data.as_ref().map_or(0, |d| d.archetype.row_count()) >= 41
    );
}
