//! Regression tests for verified Delta Audit Section DA-02 findings (`DA-02-01`..`DA-02-07`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::AtomicBool;

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, DataSheet, ErrorCode, FitMethod, NestedTable, OneWayMethod, SheetId, TableArchetype, TwoGroupMethod, VariableRole,
};
use curvebook_engine::services::{ExportArtifactFormat, ExportArtifactRequest, ExportTarget, RasterSizingPolicy, export_document_artifact};
use curvebook_engine::{BatchCommandStep, Caller, ExternalPathPolicy, Session};
use curvebook_render::{MAX_CAPPED_RASTER_DIM_PX, MAX_RASTER_PIXELS};
use serde_json::{Value, json};

use super::common::{
    TempWorkspace, assert_bad_input, assert_bad_inputs, assert_err_cases, col_sheet, ds, err_cmd, gs, mv_sheet, ok_cmd, rs, run_cmds, session_with,
    xy_sheet,
};

fn make_da02_col(id: &str, cols: usize, rows: usize) -> DataSheet {
    let data: Vec<(String, Vec<f64>)> =
        (0..cols).map(|c| (format!("G{c}"), (0..rows).map(|r| 10.0 + (c as f64) * 3.0 + (r as f64) * 0.5).collect())).collect();
    let refs: Vec<(&str, &[f64])> = data.iter().map(|(n, v)| (n.as_str(), v.as_slice())).collect();
    col_sheet(id, "Columns", &refs)
}

fn make_dose_xy(id: &str) -> DataSheet {
    xy_sheet(id, "Dose", "X", &[-9.0, -8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0], &[("Y", &[10.0, 12.0, 22.0, 52.0, 84.0, 96.0, 99.0, 100.0])])
}

fn make_da02_mv(id: &str) -> DataSheet {
    let t: Vec<Cell> = (0..8).map(|r| Cell::number((r + 1) as f64 * 2.0)).collect();
    let st: Vec<Cell> = (0..8).map(|r| Cell::number(if r % 2 == 0 { 1.0 } else { 0.0 })).collect();
    let x1: Vec<Cell> = (0..8).map(|r| Cell::number(5.0 + (r as f64) * 1.3 + if r % 3 == 0 { 0.4 } else { -0.2 })).collect();
    let grp: Vec<Cell> = (0..8).map(|r| Cell::text(if r < 4 { "A" } else { "B" })).collect();
    mv_sheet(
        id,
        "MV",
        &[],
        &[
            ("Time", Some(VariableRole::Continuous), &t),
            ("Status", Some(VariableRole::Continuous), &st),
            ("X1", Some(VariableRole::Continuous), &x1),
            ("Group", Some(VariableRole::Categorical), &grp),
        ],
    )
}

#[test]
fn da02_01_retired_aliases_rejected_and_overrides_applied() {
    let mut session = session_with(
        "DA-02-01",
        vec![
            make_da02_col("col3", 3, 6),
            col_sheet("pvals", "PValues", &[("P", &[0.001, 0.015, 0.04, 0.2, 0.6])]),
            make_dose_xy("xy1"),
            make_da02_mv("mv1"),
        ],
    );

    // 1. Pruned retired aliases across analysis families must fail with BadInput instead of silently defaulting.
    assert_bad_inputs(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "threshold": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "post_hoc": "tukey" })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_way_anova", "multiplicity_method": "tukey" })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "alpha": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "q": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "fdr_q": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_threshold": 0.01 })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_method": "bonferroni" })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "multiplicity_method": "bonferroni" })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "multiplicity": "bonferroni" })),
            ("stats.run", json!({ "source_sheet_id": "pvals", "kind": "pvalue_stack", "pvalue_stack_column": 0 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "one_sample_t", "hypothetical_mean": 5.0 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "unpaired_t", "group_column": 3 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "unpaired_t", "response_column": 2 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "log_rank": false })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "gehan": true })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "time_column": 0 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "status_column": 1 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "kaplan_meier", "group_column": 3 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "cox_ph", "cox_covariate_columns": [2] })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "glm", "glm_family": "linear" })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "n": 20 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "n2": 20 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "standardized_effect": 0.5 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "df": 2 })),
            ("stats.run", json!({ "source_sheet_id": "col3", "kind": "power", "anova_groups": 3 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "correlation_matrix", "correlation_all_pairs": true })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "correlation_matrix", "correlation_heatmap": true })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "clustering", "distance_metric": "euclidean" })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "clustering", "linkage_method": "ward" })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "m": 5 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "num_imputations": 5 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "iterations": 5 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "target_column": 0 })),
            ("stats.run", json!({ "source_sheet_id": "mv1", "kind": "multiple_imputation", "predictors": [2] })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "fit_method": "rout" })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "model_comparison": "aicc" })),
            ("fit.run", json!({ "source_sheet_id": "xy1", "kind": "4pl", "normality_tests": true })),
            (
                "fit.run",
                json!({ "source_sheet_id": "xy1", "kind": "4pl", "comparison": "aicc", "alternative_equation_id": "log_agonist_vs_response_3pl", "alternative_model": "3pl" }),
            ),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "preset": "log10" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "operation": "log10" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "math", "propagate_error": "delta" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "zero": "smallest" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "normalize", "hundred": "largest" })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "prune_rows", "step": 2 })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "prune_rows", "average_dropped": true })),
            ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "derived": [] })),
            ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "column_formulas": [] })),
            ("transform.run", json!({ "source_sheet_id": "mv1", "kind": "select_and_transform", "formulas": [] })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "simulate", "n_points": 20 })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "simulate", "params": { "Top": 100.0 } })),
            ("transform.run", json!({ "source_sheet_id": "xy1", "kind": "random_sample", "n_samples": 4 })),
        ],
    );

    // 2. Nonlinear ROUT Q: both `fdr_q_percent` and `q_percent` apply; conflicts fail.
    ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "nl-rout", "source_sheet_id": "xy1", "kind": "4pl", "method": "rout", "fdr_q_percent": 5.0 }),
    );
    assert!(
        matches!(rs(&session, "nl-rout").analysis.kind, AnalysisKind::NonlinearRegression(ref nl) if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 5.0).abs() < 1e-12))
    );
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "nl-rout", "fdr_q_percent": 3.5 }));
    assert!(
        matches!(rs(&session, "nl-rout").analysis.kind, AnalysisKind::NonlinearRegression(ref nl) if matches!(nl.method, FitMethod::RoutOutlierElimination { fdr_q_percent } if (fdr_q_percent - 3.5).abs() < 1e-12))
    );
    assert_bad_input(&mut session, "analysis.updateParams", json!({ "sheet_id": "nl-rout", "q_percent": 2.0, "fdr_q_percent": 4.0 }));

    // 3. `analysis.updateParams` updates `hypothetical` and `geisser_greenhouse`, rejects retired metadata inside `params`.
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "os-t", "source_sheet_id": "col3", "kind": "one_sample_t", "hypothetical": 1.0 })),
            ("analysis.updateParams", json!({ "sheet_id": "os-t", "hypothetical": 12.5 })),
            ("stats.run", json!({ "sheet_id": "rm-anova", "source_sheet_id": "col3", "kind": "rm_one_way_anova", "geisser_greenhouse": true })),
            ("analysis.updateParams", json!({ "sheet_id": "rm-anova", "geisser_greenhouse": false })),
            ("stats.run", json!({ "sheet_id": "corr-1", "source_sheet_id": "mv1", "kind": "correlation_matrix" })),
            ("analysis.updateParams", json!({ "sheet_id": "corr-1", "correlation_all_pairs": true, "correlation_heatmap": false })),
        ],
    );
    assert!(
        matches!(rs(&session, "os-t").analysis.kind, AnalysisKind::TwoGroupComparison(ref tg) if matches!(tg.method, TwoGroupMethod::OneSampleT { hypothetical } if (hypothetical - 12.5).abs() < 1e-12))
    );
    assert!(
        matches!(rs(&session, "rm-anova").analysis.kind, AnalysisKind::OneWayAnova(ref spec) if spec.method == OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
    );

    assert_bad_inputs(
        &mut session,
        &[
            ("analysis.updateParams", json!({ "sheet_id": "os-t", "params": { "confidence_level": 90.0 } })),
            ("analysis.updateParams", json!({ "sheet_id": "os-t", "params": { "custom_title": "X" } })),
            ("analysis.updateParams", json!({ "sheet_id": "os-t", "params": { "selected_columns": [0] } })),
            ("analysis.updateParams", json!({ "sheet_id": "os-t", "params": { "columns": [0] } })),
            ("analysis.updateParams", json!({ "sheet_id": "corr-1", "correlation_heatmap": "yes" })),
            ("analysis.updateParams", json!({ "sheet_id": "corr-1", "params": { "correlation_heatmap": true } })),
        ],
    );

    // 4. Error/remediation text references canonical `source_sheet_id`, never retired `source_sheet`.
    let missing_res = err_cmd(&mut session, "analysis.run", json!({ "sheet_id": "does-not-exist" }));
    assert!(missing_res.message().contains("source_sheet_id") && !missing_res.message().contains("`source_sheet`"));
}

#[test]
fn da02_02_nested_subcolumn_count_uses_biological_units() {
    let make_nested = |id: &str, title: &str, bio: usize, rows: usize| {
        let mut nt = NestedTable::new(2, bio, rows).unwrap();
        for g in &mut nt.groups {
            for r in 0..rows {
                for s in 0..bio {
                    g.cells[r][s] = Cell::number((r * 100 + s) as f64);
                }
            }
        }
        DataSheet::new(id, title, TableArchetype::Nested(nt)).unwrap()
    };
    let mut session = session_with("DA-02-02 Nested", vec![make_nested("nest-tall", "Tall", 3, 12), make_nested("nest-wide", "Wide", 5, 2)]);

    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "nest-tall", "all": true }));
    let tall_after = ds(&session, "nest-tall");
    for col in 0..2 {
        for row in 0..12 {
            for sub in 0..3 {
                assert!(tall_after.cell(CellCoord::new(row, col, sub)).unwrap().is_empty());
            }
        }
    }
    assert_bad_input(
        &mut session,
        "table.excludeCells",
        json!({ "sheet_id": "nest-tall", "range": { "start_row": 0, "end_row": 0, "start_col": 0, "end_col": 0, "start_subcol": 3, "end_subcol": 3 }, "excluded": true }),
    );

    ok_cmd(
        &mut session,
        "table.excludeCellsWithReason",
        json!({ "sheet_id": "nest-wide", "coords": [{ "row": 1, "col": 0, "subcol": 4 }], "reason_tag": "outlier" }),
    );
    assert_eq!(
        ok_cmd(&mut session, "table.exclusionAuditReport", json!({ "sheet_id": "nest-wide" })).payload.get("total_excluded").and_then(Value::as_u64),
        Some(1)
    );

    ok_cmd(&mut session, "table.clearCells", json!({ "sheet_id": "nest-wide", "all": true }));
    let wide_after = ds(&session, "nest-wide");
    for col in 0..2 {
        for row in 0..2 {
            for sub in 0..5 {
                assert!(wide_after.cell(CellCoord::new(row, col, sub)).unwrap().is_empty());
            }
        }
    }
}

#[test]
fn da02_03_atomic_batch_selection_save_open_and_undo_redo() {
    let tmp = TempWorkspace::new("da02-batch");
    let (save_path, save_path_2) = (tmp.workdir.join("project.curvebook"), tmp.workdir.join("project-2.curvebook"));
    let cancel = AtomicBool::new(false);

    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "s1", "title": "First", "kind": "column", "cols": 1, "rows": 2 })),
            ("sheet.createData", json!({ "sheet_id": "s2", "title": "Second", "kind": "column", "cols": 1, "rows": 2 })),
            ("sheet.rename", json!({ "sheet_id": "s2", "title": "Second v2" })),
            ("edit.undo", json!({})),
            ("project.save", json!({ "path": save_path.to_str().unwrap() })),
        ],
    );
    let (undo_before, redo_before) = (session.undo_depth(), session.redo_depth());
    assert!(!session.is_dirty() && undo_before > 0 && redo_before == 1);

    // (1) Read-only `sheet.select` inside batch does not push undo, clear redo, or dirty session.
    session.execute_batch_atomic(&[BatchCommandStep::new("sheet.select", json!({ "sheet_id": "s1" }))], Caller::Ui, &cancel).unwrap();
    assert_eq!(session.document().active_sheet_id.as_ref().map(SheetId::as_str), Some("s1"));
    assert!(!session.is_dirty() && session.undo_depth() == undo_before && session.redo_depth() == redo_before);

    // (2a) Mutating batch ending with `project.save` collapses undo AND stays clean.
    session
        .execute_batch_atomic(
            &[
                BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "First A" })),
                BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "First B" })),
                BatchCommandStep::new("project.save", json!({ "path": save_path.to_str().unwrap() })),
            ],
            Caller::Ui,
            &cancel,
        )
        .unwrap();
    assert!(!session.is_dirty() && session.undo_depth() == undo_before + 1 && session.redo_depth() == 0);

    // (2b-2e) Save-only, Save-As, edit+open, and save+edit dirty/stack semantics.
    ok_cmd(&mut session, "sheet.rename", json!({ "sheet_id": "s1", "title": "First Dirty" }));
    let dirty_undo = session.undo_depth();
    for path in [&save_path, &save_path_2] {
        session
            .execute_batch_atomic(&[BatchCommandStep::new("project.save", json!({ "path": path.to_str().unwrap() }))], Caller::Ui, &cancel)
            .unwrap();
        assert!(!session.is_dirty() && session.undo_depth() == dirty_undo && session.file_path().is_some());
    }
    session
        .execute_batch_atomic(
            &[
                BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "Unsaved Edit" })),
                BatchCommandStep::new("project.open", json!({ "path": save_path.to_str().unwrap() })),
            ],
            Caller::Ui,
            &cancel,
        )
        .unwrap();
    assert!(!session.is_dirty());

    session
        .execute_batch_atomic(
            &[
                BatchCommandStep::new("project.save", json!({ "path": save_path.to_str().unwrap() })),
                BatchCommandStep::new("sheet.rename", json!({ "sheet_id": "s1", "title": "After Save" })),
            ],
            Caller::Ui,
            &cancel,
        )
        .unwrap();
    assert!(session.is_dirty());

    // (3) `edit.undo` & `edit.redo` inside batch preserve stacks cleanly.
    let doc_before_undo = session.document().clone();
    let undo_depth_pre = session.undo_depth();
    session.execute_batch_atomic(&[BatchCommandStep::new("edit.undo", json!({}))], Caller::Ui, &cancel).unwrap();
    assert_eq!((session.undo_depth(), session.redo_depth()), (undo_depth_pre - 1, 1));
    session.execute_batch_atomic(&[BatchCommandStep::new("edit.redo", json!({}))], Caller::Ui, &cancel).unwrap();
    assert_eq!((session.undo_depth(), session.redo_depth()), (undo_depth_pre, 0));
    assert_eq!(session.document(), &doc_before_undo);
}

#[test]
fn da02_04_capped_raster_export_bounds_and_zero_validation() {
    let mut session = Session::new();
    run_cmds(
        &mut session,
        &[
            ("sheet.createData", json!({ "sheet_id": "xy1", "title": "Dose", "kind": "xy", "rows": 3, "cols": 1, "auto_graph": true })),
            ("table.setXCells", json!({ "sheet_id": "xy1", "values": [1.0, 2.0, 3.0] })),
            ("table.setCells", json!({ "sheet_id": "xy1", "rows": [[1.0], [2.0], [3.0]] })),
        ],
    );
    let graph_id = session.document().graph_sheets[0].id.clone();

    let make_req = |w, h, dpi, default_dpi| ExportArtifactRequest {
        target: ExportTarget::Explicit(graph_id.clone()),
        format: ExportArtifactFormat::Png {
            width_px: w,
            height_px: h,
            dpi,
            sizing: RasterSizingPolicy::CapToLimits { default_dpi },
            transparent_bg: false,
        },
    };
    for (w, h) in [(Some(10_000), Some(10_000)), (Some(20_000), Some(1_000)), (Some(12_000), None), (None, Some(12_000))] {
        let art = export_document_artifact(session.document(), &make_req(w, h, Some(300), 150), None, "test.da02_04").unwrap();
        let meta = art.raster.unwrap();
        assert!(
            art.bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                && meta.width_px <= MAX_CAPPED_RASTER_DIM_PX
                && meta.height_px <= MAX_CAPPED_RASTER_DIM_PX
                && u64::from(meta.width_px) * u64::from(meta.height_px) <= MAX_RASTER_PIXELS
        );
    }
    for (w, h, dpi, default_dpi) in
        [(Some(0), Some(200), Some(150), 150), (Some(200), Some(0), Some(150), 150), (None, None, Some(0), 150), (None, None, None, 0)]
    {
        assert_eq!(
            export_document_artifact(session.document(), &make_req(w, h, dpi, default_dpi), None, "test.da02_04.zero").unwrap_err().code,
            ErrorCode::BadInput
        );
    }
}

#[test]
fn da02_05_patch_analysis_kind_fields_handles_schema_nullable_non_option_fields() {
    let mut session = session_with(
        "DA-02-05",
        vec![make_dose_xy("xy1"), col_sheet("pvals", "PValues", &[("P", &[0.001, 0.015, 0.04, 0.2, 0.6])]), make_da02_col("col2", 2, 5)],
    );
    run_cmds(
        &mut session,
        &[
            (
                "fit.run",
                json!({ "sheet_id": "fit-null", "source_sheet_id": "xy1", "kind": "4pl", "identify_unstable_parameters": null, "equation_id": null }),
            ),
            (
                "stats.pvalueStack",
                json!({ "sheet_id": "pstack-null", "source_sheet_id": "pvals", "method": "benjamini_hochberg", "threshold": null }),
            ),
            (
                "stats.run",
                json!({ "sheet_id": "desc-1", "source_sheet_id": "col2", "kind": "descriptive", "hypothetical_value": 10.0, "custom_upper_percentile": 90.0 }),
            ),
            ("analysis.updateParams", json!({ "sheet_id": "desc-1", "hypothetical_value": null, "custom_upper_percentile": null })),
        ],
    );
    assert!(matches!(rs(&session, "pstack-null").analysis.kind, AnalysisKind::PValueStack(ref ps) if (ps.threshold - 0.05).abs() < 1e-12));
    assert!(
        matches!(rs(&session, "desc-1").analysis.kind, AnalysisKind::DescriptiveStats(ref d) if d.hypothetical_value.is_none() && d.custom_upper_percentile.is_none())
    );
    assert_bad_input(&mut session, "stats.run", json!({ "source_sheet_id": "col2", "kind": "bland_altman", "column_a": null }));
}

#[test]
fn da02_06_broken_axis_segments_do_not_leak_segment_zero() {
    let mut session = Session::new();
    ok_cmd(&mut session, "sheet.createData", json!({ "sheet_id": "xy1", "title": "Data", "kind": "xy", "rows": 3, "cols": 1, "auto_graph": true }));
    let gid = session.document().graph_sheets[0].id.as_str().to_owned();
    run_cmds(
        &mut session,
        &[
            ("graph.setAxes", json!({ "sheet_id": gid, "y_axis": { "min": 0.0, "max": 10.0, "major_tick_interval": 2.0 } })),
            (
                "graph.setAxes",
                json!({ "sheet_id": gid, "y_axis": { "segments": [{ "min": 0.0, "max": 5.0, "length_share": 0.4 }, { "min": 50.0, "length_share": 0.6 }] } }),
            ),
        ],
    );
    let segs = &gs(&session, &gid).spec.y_axis.segments;
    assert_eq!(segs.len(), 2);
    assert_eq!((segs[0].min, segs[0].max, segs[1].min, segs[1].max, segs[1].major_tick_interval), (Some(0.0), Some(5.0), Some(50.0), None, None));
}

#[test]
fn da02_07_analysis_exports_enforce_path_policy() {
    let tmp = TempWorkspace::new("da02-export-policy");
    let mut session = session_with("DA-02-07", vec![make_da02_col("col2", 2, 5)])
        .with_path_policy(ExternalPathPolicy::workdir(&tmp.workdir).expect("workdir policy"));
    run_cmds(
        &mut session,
        &[
            ("stats.run", json!({ "sheet_id": "res1", "source_sheet_id": "col2", "kind": "descriptive" })),
            ("analysis.exportJson", json!({ "sheet_id": "res1", "path": "out.json" })),
            ("analysis.exportCsv", json!({ "sheet_id": "res1", "path": "out.csv" })),
            ("result.generateReport", json!({ "result_sheet_id": "res1", "path": "report.md" })),
        ],
    );
    assert!(tmp.workdir.join("out.json").is_file() && tmp.workdir.join("out.csv").is_file() && tmp.workdir.join("report.md").is_file());

    let (out_json, out_csv, out_md) = (tmp.outside.join("escape.json"), tmp.outside.join("escape.csv"), tmp.outside.join("escape.md"));
    assert_err_cases(
        &mut session,
        &[
            ("analysis.exportJson", json!({ "sheet_id": "res1", "path": out_json.to_str().unwrap() }), ErrorCode::PathEscape),
            ("analysis.exportCsv", json!({ "sheet_id": "res1", "path": "../outside/escape.csv" }), ErrorCode::PathEscape),
            ("result.generateReport", json!({ "result_sheet_id": "res1", "path": out_md.to_str().unwrap() }), ErrorCode::PathEscape),
        ],
    );
    assert!(!out_json.exists() && !out_csv.exists() && !out_md.exists());
}

#[test]
fn descriptive_parity_options_methods_report_and_raw_x_scale_warning() {
    let raw_molar_xy = xy_sheet(
        "xy-raw-molar",
        "Raw Molar Dose",
        "Concentration (M)",
        &[1e-9, 1e-8, 1e-7, 1e-6, 1e-5],
        &[("Response", &[5.0, 18.0, 52.0, 86.0, 96.0])],
    );
    let mut session = session_with("R19-1-Engine", vec![make_da02_col("col2", 2, 10), raw_molar_xy]);

    // 1) Descriptive stats additive options (`visible_metrics`, `significant_digits`, `save_as_default`) + methods report.
    ok_cmd(
        &mut session,
        "stats.run",
        json!({
            "sheet_id": "desc-parity",
            "source_sheet_id": "col2",
            "kind": "descriptive",
            "custom_upper_percentile": 90.0,
            "significant_digits": 4,
            "save_as_default": true,
            "visible_metrics": { "harmonic_mean": false, "sum": false }
        }),
    );
    let desc_rs = rs(&session, "desc-parity");
    assert!(matches!(
        &desc_rs.analysis.kind,
        AnalysisKind::DescriptiveStats(d)
            if d.significant_digits == Some(4)
                && d.save_as_default
                && !d.visible_metrics.harmonic_mean
                && !d.visible_metrics.sum
                && d.visible_metrics.mean_sd_sem
    ));
    let tab = desc_rs.tabs.iter().find(|t| t.id == "desc-summary").expect("desc-summary tab");
    assert!(tab.row_labels.iter().any(|l| l == "10% Percentile (Type 6)"));
    assert!(tab.row_labels.iter().any(|l| l == "90% Percentile (Type 6)"));
    assert!(!tab.row_labels.iter().any(|l| l == "Harmonic mean" || l == "Sum"));

    let report = ok_cmd(&mut session, "result.generateReport", json!({ "result_sheet_id": "desc-parity", "format": "markdown" }));
    let md = report.payload["report"].as_str().unwrap_or("");
    assert!(md.contains("Hyndman-Fan Type 6"), "methods report should cite Type 6 custom percentiles: {md}");
    assert!(md.contains("4 significant digits"), "methods report should note significant_digits: {md}");
    assert!(md.contains("2 metric group(s) hidden"), "methods report should note hidden metric groups: {md}");

    // `analysis.updateParams` supports resetting `significant_digits` to `null` and rejects out-of-range values.
    ok_cmd(&mut session, "analysis.updateParams", json!({ "sheet_id": "desc-parity", "significant_digits": null }));
    assert!(matches!(&rs(&session, "desc-parity").analysis.kind, AnalysisKind::DescriptiveStats(d) if d.significant_digits.is_none()));
    assert_bad_inputs(
        &mut session,
        &[
            ("stats.run", json!({ "source_sheet_id": "col2", "kind": "descriptive", "significant_digits": 0 })),
            ("stats.run", json!({ "source_sheet_id": "col2", "kind": "descriptive", "significant_digits": 16 })),
            ("analysis.updateParams", json!({ "sheet_id": "desc-parity", "significant_digits": 0 })),
        ],
    );

    // 2) Nonlinear `log_*` equation run on raw molar X values returns non-crashing scale warning offering non-log counterpart.
    let fit_out = ok_cmd(
        &mut session,
        "fit.run",
        json!({ "sheet_id": "fit-raw-molar", "source_sheet_id": "xy-raw-molar", "equation_id": "log_agonist_vs_response_4pl" }),
    );
    assert!(
        fit_out.warnings.iter().any(|w| w.contains("agonist_vs_response_4pl")),
        "expected outcome.warnings to offer non-log counterpart, got {:?}",
        fit_out.warnings
    );
    let scale_warnings = fit_out.payload["scale_warnings"].as_array().expect("scale_warnings array");
    assert!(scale_warnings.iter().any(|w| w.as_str().is_some_and(|s| s.contains("agonist_vs_response_4pl"))));
}
