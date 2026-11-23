#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Stage 3A X2 data-correctness fixes (`R1-X2-01` and `R1-X2-02`)
//! across `doc`, `transform`, `fit`, `engine::dag`, and `chart::xy`.

use std::collections::HashSet;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistItem, ChecklistStatus, CiBound, DataSheet, ErrorCode, FittedCurveOverlay, FloatingNote,
    GraphAnnotation, GraphSheet, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkSource, NonlinearFitSpec,
    NormalizeAnchor, PairwiseComparisonRecord, ProjectDocument, RectPt, RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet,
    ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::dag::recalculate_project_with_report;
use curvebook_fit::HookContext;
use curvebook_testkit::metrics::assert_approx_eq;
use curvebook_transform::TransformContext;
use serde_json::json;

fn xy_single(id: &str, xs: &[f64], ys: &[f64]) -> DataSheet {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, xs.len()).unwrap();
    for (r, (&x, &y)) in xs.iter().zip(ys).enumerate() {
        xy.set_x_cell(r, Cell::number(x)).unwrap();
        if let Some(cell) = xy.data_sets.first_mut().and_then(|ds| ds.cells.get_mut(r)).and_then(|row| row.first_mut()) {
            *cell = Cell::number(y);
        }
    }
    DataSheet::new(id, id, TableArchetype::Xy(xy)).unwrap()
}

fn scalar(s: &Session, res: &str, key: &str) -> Option<f64> {
    s.document().result_sheet(&SheetId::new(res)).unwrap().lookup_scalar(key, None)
}

#[test]
fn r1_x2_01_failed_recompute_clears_stale_outputs_and_consumers_reject_failed() {
    let ds = xy_single("d", &[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.1, 5.9, 8.2, 9.9]);
    let mut doc = ProjectDocument::new("x2-01").unwrap();
    doc.add_data_sheet(ds).unwrap();

    // Pre-existing downstream Transform hooking `lin:Slope` AND downstream Nonlinear fit sourcing `tr`.
    let norm_spec = AnalysisSpec::new(
        "tr",
        "d",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                result_sheet_id: SheetId::new("lin"),
                parameter_key: String::from("Slope"),
                data_set_index: None,
            }),
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    let nl_spec = AnalysisSpec::new("nl", "tr", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap();

    let mut s = Session::from_document(doc);
    s.execute("analysis.add", json!({"source_sheet": "d", "sheet_id": "lin", "kind": "linear"})).unwrap();

    // Attach pre-populated checklist/notes/comparisons onto `lin` and add downstream `tr` and `nl`,
    // then recalculate cleanly first.
    let mut doc2 = s.document().clone();
    doc2.add_result_sheet(ResultSheet::new(norm_spec).unwrap()).unwrap();
    doc2.add_result_sheet(ResultSheet::new(nl_spec).unwrap()).unwrap();
    let report_ok = recalculate_project_with_report(&mut doc2).unwrap();
    assert!(report_ok.failed_result_sheets.is_empty());

    // Populate derived stale fields plus user `floating_notes` onto `lin`: failure must clear derived fields while preserving user notes.
    {
        let lin_mut = doc2.result_sheets.iter_mut().find(|r| r.id.as_str() == "lin").unwrap();
        assert_eq!(lin_mut.status, ResultStatus::UpToDate);
        assert!(lin_mut.lookup_scalar("Slope", None).is_some());
        assert!(!lin_mut.tabs.is_empty());
        assert!(!lin_mut.fitted_curves.is_empty());
        lin_mut.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: "A".to_owned(),
            label_b: "B".to_owned(),
            estimate_diff: Some(1.0),
            raw_p_or_q: 0.01,
            is_fdr_q: false,
            significant: true,
        });
        lin_mut.checklist.push(ChecklistItem {
            assumption: "Stale item".to_owned(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: "Stale detail".to_owned(),
            remediation: None,
        });
        lin_mut.floating_notes.push(FloatingNote::new("n1", RectPt::new(0.0, 0.0, 50.0, 20.0), "stale note"));
    }

    let mut s2 = Session::from_document(doc2);
    // Exclude 3 of 5 points so only 2 remain -> `lin` recompute fails, cascading to `tr` and `nl`.
    for r in 0..3 {
        let _ = s2.execute("table.excludeCells", json!({"sheet_id": "d", "row": r, "col": 0, "subcol": 0}));
    }

    let doc_after = s2.document();
    for failed_id in ["lin", "tr", "nl"] {
        let res = doc_after.result_sheet(&SheetId::new(failed_id)).unwrap();
        assert!(matches!(res.status, ResultStatus::Failed { .. }), "{failed_id} should be Failed, got {:?}", res.status);
        assert!(res.lookup_scalar("Slope", None).is_none(), "{failed_id} must not expose Slope");
        assert!(res.scalars.is_empty(), "{failed_id} stale scalars must be cleared");
        assert!(res.tabs.is_empty(), "{failed_id} stale tabs must be cleared");
        assert!(res.fitted_curves.is_empty(), "{failed_id} stale fitted curves must be cleared");
        assert!(res.pairwise_comparisons.is_empty(), "{failed_id} stale pairwise comparisons must be cleared");
        assert!(res.analyzable_data.is_none(), "{failed_id} stale analyzable_data must be cleared");
        assert!(res.checklist.is_empty(), "{failed_id} stale checklist must be cleared");
        if failed_id == "lin" {
            assert_eq!(res.floating_notes.len(), 1, "lin user floating notes must survive transient recompute failure");
        } else {
            assert!(res.floating_notes.is_empty());
        }
    }

    // Defense-in-depth checks even if a Failed ResultSheet still artificially retains stale fields:
    let mut stale_failed =
        ResultSheet::new(AnalysisSpec::new("fake_lin", "d", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap())
            .unwrap();
    stale_failed.status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "synthetic solver error".to_owned() };
    stale_failed.scalars.push(ResultScalarEntry { key: "Slope".to_owned(), data_set_index: None, value: 42.0, units: None });
    stale_failed.analyzable_data = Some(xy_single("fake_analyzable", &[1.0, 2.0, 3.0], &[9.0, 9.0, 9.0]));
    stale_failed.tabs.push(ResultTab {
        id: "t0".to_owned(),
        title: "Stale Tab".to_owned(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec!["Col".to_owned()],
        row_labels: vec!["Row".to_owned()],
        rows: vec![vec![ResultCell::Text("STALE_TABLE_TOKEN".to_owned())]],
        footnotes: Vec::new(),
    });
    stale_failed.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: "straight_line".to_owned(),
        converged: true,
        ambiguous: false,
        points: vec![curvebook_doc::CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
    });

    assert!(stale_failed.lookup_scalar("Slope", None).is_none(), "Failed ResultSheet::lookup_scalar must return None");

    let dummy_ds = doc_after.data_sheets[0].clone();
    let failed_sheets = [stale_failed.clone()];
    let tr_ctx = TransformContext {
        info_sheet: None,
        info_sheets: &[],
        result_sheets: &failed_sheets,
        data_sheets: std::slice::from_ref(&dummy_ds),
        selected_data_sets: Vec::new(),
        output_id: None,
        output_title: None,
    };
    let hook = HookSource::ResultScalar { result_sheet_id: SheetId::new("fake_lin"), parameter_key: "Slope".to_owned(), data_set_index: None };
    let tr_err = tr_ctx.resolve_hook(&hook, &dummy_ds).unwrap_err();
    assert!(tr_err.to_string().contains("failed"), "TransformContext must reject Failed sheet: {tr_err}");

    let fit_hooks = HookContext {
        info_sheets: &[],
        result_sheets: &failed_sheets,
        data_sheets: std::slice::from_ref(&dummy_ds),
        primary_info_sheet_id: None,
        visited_sheets: HashSet::new(),
    };
    let fit_err = fit_hooks.resolve(&hook).unwrap_err();
    assert!(fit_err.to_string().contains("failed"), "HookContext must reject Failed sheet: {fit_err}");

    // Chart XY consumers (overlay, analyzable_data source, embedded result table) must reject Failed sheets.
    let mut check_doc = ProjectDocument::new("chart-check").unwrap();
    check_doc.add_data_sheet(dummy_ds.clone()).unwrap();
    check_doc.add_result_sheet(stale_failed.clone()).unwrap();

    let mut gsheet = GraphSheet::new_uninitialized_for_data("g-fail", &dummy_ds).unwrap();
    gsheet.initialized = true;
    gsheet.spec.series[0].regression_overlay = Some(RegressionOverlaySpec {
        result_sheet_id: SheetId::new("fake_lin"),
        sample_segments: 32,
        show_confidence_band: false,
        show_prediction_band: false,
        shaded_ribbon: false,
    });
    gsheet.spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
        result_sheet_id: SheetId::new("fake_lin"),
        tab_id: None,
        bounds_pt: RectPt::new(10.0, 10.0, 140.0, 80.0),
        selected_rows: None,
        selected_cols: None,
        style: curvebook_doc::EmbeddedTableStyle::default(),
    });
    let scene = compile_graph(&gsheet, &check_doc).unwrap();
    assert!(
        scene.warnings.iter().any(|w| w.code == "failed_regression_result"),
        "expected failed_regression_result warning, got {:?}",
        scene.warnings
    );
    let scene_dbg = format!("{:?}", scene.primitives);
    assert!(!scene_dbg.contains("STALE_TABLE_TOKEN"), "EmbeddedResultTable must not render stale text from Failed ResultSheet");

    // Graph series directly targeting analyzable_data on a Failed ResultSheet must not plot stale points.
    let mut g_src = GraphSheet::new_uninitialized_for_data("g-fail-src", &dummy_ds).unwrap();
    g_src.initialized = true;
    g_src.spec.series[0].source_sheet_id = SheetId::new("fake_lin");
    let src_scene = compile_graph(&g_src, &check_doc).unwrap();
    assert!(src_scene.plotted_points.is_empty(), "resolve_data_sheet must refuse Failed analyzable ResultSheet");

    // Linked DataSheet inside DAG must refuse non-UpToDate ResultSheet instead of copying stale analyzable_data.
    let mut linked_ds = xy_single("d-linked", &[1.0, 2.0], &[0.0, 0.0]);
    linked_ds.linked_regions.push(curvebook_doc::LinkedDataRegion {
        id: "link-1".to_owned(),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("tr"),
                kind: InternalLinkKind::AnalyzableResultSheet,
                tab_id: None,
                range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0)),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 1, 0)),
        provenance_summary: "from tr".to_owned(),
    });
    let mut dag_doc = doc_after.clone();
    dag_doc.add_data_sheet(linked_ds).unwrap();
    let dag_err = recalculate_project_with_report(&mut dag_doc).unwrap_err();
    assert!(dag_err.to_string().contains("failed"), "DAG linked region refresh must refuse Failed upstream ResultSheet: {dag_err}");
}

fn replicates() -> Vec<Vec<f64>> {
    vec![vec![1.0, 2.0, 3.0], vec![3.0, 4.0, 6.0], vec![4.0, 5.0, 7.0], vec![6.0, 8.0, 9.0], vec![7.0, 9.0, 10.0, 11.0, 12.0, 14.0]]
}

fn rep_stats(v: &[f64]) -> (f64, f64, f64) {
    let n = v.len() as f64;
    let m = v.iter().sum::<f64>() / n;
    let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    (m, sd, n)
}

#[test]
fn r1_x2_02_mean_sd_n_regression_matches_raw_replicates() {
    let reps = replicates();
    let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
    let mut raw =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(6), 1, 5).unwrap();
    let mut sum = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 5).unwrap();
    for (r, v) in reps.iter().enumerate() {
        raw.set_x_cell(r, Cell::number(xs[r])).unwrap();
        sum.set_x_cell(r, Cell::number(xs[r])).unwrap();
        for (k, &y) in v.iter().enumerate() {
            raw.data_sets[0].cells[r][k] = Cell::number(y);
        }
        let (m, sd, n) = rep_stats(v);
        sum.data_sets[0].cells[r][0] = Cell::number(m);
        sum.data_sets[0].cells[r][1] = Cell::number(sd);
        sum.data_sets[0].cells[r][2] = Cell::number(n);
    }

    let mut doc = ProjectDocument::new("x2-02").unwrap();
    doc.add_data_sheet(DataSheet::new("raw", "raw", TableArchetype::Xy(raw)).unwrap()).unwrap();
    doc.add_data_sheet(DataSheet::new("sum", "sum", TableArchetype::Xy(sum)).unwrap()).unwrap();
    let mut s = Session::from_document(doc);

    s.execute("analysis.add", json!({"source_sheet": "raw", "sheet_id": "lraw", "kind": "linear"})).unwrap();
    s.execute("analysis.add", json!({"source_sheet": "sum", "sheet_id": "lsum", "kind": "linear"})).unwrap();

    for key in ["Slope", "YIntercept", "R2"] {
        let a = scalar(&s, "lraw", key).unwrap();
        let b = scalar(&s, "lsum", key).unwrap();
        assert_approx_eq(b, a, 1e-10, 1e-10, &format!("Session linear {key}"));
    }
    let raw_tab = &s.document().result_sheet(&SheetId::new("lraw")).unwrap().tabs[0];
    let sum_tab = &s.document().result_sheet(&SheetId::new("lsum")).unwrap().tabs[0];
    assert_eq!(sum_tab.rows.len(), raw_tab.rows.len());
    for (row_idx, (got_row, exp_row)) in sum_tab.rows.iter().zip(raw_tab.rows.iter()).take(9).enumerate() {
        for (col_idx, (got, exp)) in got_row.iter().zip(exp_row.iter()).enumerate() {
            match (got, exp) {
                (ResultCell::Number { value: a, .. }, ResultCell::Number { value: b, .. }) => {
                    assert_approx_eq(*a, *b, 1e-10, 1e-10, &format!("Session linear tab0[{row_idx}][{col_idx}]"));
                }
                (
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(a_lo), upper: CiBound::Finite(a_hi) },
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(b_lo), upper: CiBound::Finite(b_hi) },
                ) => {
                    assert_approx_eq(*a_lo, *b_lo, 1e-10, 1e-10, &format!("Session linear CI lower[{row_idx}]"));
                    assert_approx_eq(*a_hi, *b_hi, 1e-10, 1e-10, &format!("Session linear CI upper[{row_idx}]"));
                }
                _ => assert_eq!(got, exp, "Session linear tab0 cell[{row_idx}][{col_idx}]"),
            }
        }
    }
}

#[test]
fn sec03_03_dag_transform_multi_info_sheet_and_rejects_dirty_result_scalar() {
    let mut ds = xy_single("d", &[1.0, 2.0], &[10.0, 20.0]);
    ds.info_sheet_id = Some(SheetId::new("info-a"));

    let mut info_a = curvebook_doc::InfoSheet::new_preseeded("info-a", "Info A").unwrap();
    info_a.set_constant("Scale", curvebook_doc::ConstantValue::Number(10.0), None).unwrap();

    let mut info_b = curvebook_doc::InfoSheet::new_preseeded("info-b", "Info B").unwrap();
    info_b.set_constant("Scale", curvebook_doc::ConstantValue::Number(100.0), None).unwrap();

    let mut doc = ProjectDocument::new("sec03-03").unwrap();
    doc.add_info_sheet(info_a).unwrap();
    doc.add_info_sheet(info_b).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    // Explicit info_sheet_id ("info-b") and default paired primary ("info-a") must BOTH resolve in DAG.
    let explicit_spec = AnalysisSpec::new(
        "tr-b",
        "d",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                info_sheet_id: Some(SheetId::new("info-b")),
                constant_name: String::from("Scale"),
            }),
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    let default_spec = AnalysisSpec::new(
        "tr-a",
        "d",
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::FixedValue(0.0),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: None, constant_name: String::from("Scale") }),
            output_percent: true,
            per_data_set: true,
        }),
    )
    .unwrap();
    doc.add_result_sheet(ResultSheet::new(explicit_spec).unwrap()).unwrap();
    doc.add_result_sheet(ResultSheet::new(default_spec).unwrap()).unwrap();

    recalculate_project_with_report(&mut doc).unwrap();

    let tr_b = doc.result_sheet(&SheetId::new("tr-b")).unwrap().analyzable_data.as_ref().unwrap();
    let TableArchetype::Xy(xy_b) = &tr_b.archetype else {
        panic!("expected XY");
    };
    assert_approx_eq(xy_b.data_sets[0].cells[0][0].active_number().unwrap(), 10.0, 1e-12, 0.0, "explicit info-b Scale=100");

    let tr_a = doc.result_sheet(&SheetId::new("tr-a")).unwrap().analyzable_data.as_ref().unwrap();
    let TableArchetype::Xy(xy_a) = &tr_a.archetype else {
        panic!("expected XY");
    };
    assert_approx_eq(xy_a.data_sets[0].cells[0][0].active_number().unwrap(), 100.0, 1e-12, 0.0, "primary info-a Scale=10");

    // Stale Dirty upstream ResultSheet scalar hook must be rejected by TransformContext::resolve_hook.
    let upstream_spec =
        AnalysisSpec::new("up", "d", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap();
    let mut dirty_res = ResultSheet::new(upstream_spec).unwrap();
    dirty_res.status = ResultStatus::Dirty;
    dirty_res.scalars.push(ResultScalarEntry { key: String::from("Peak"), value: 42.0, units: None, data_set_index: None });
    let ctx = TransformContext { result_sheets: std::slice::from_ref(&dirty_res), ..TransformContext::default() };
    let dirty_hook_spec = TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::FixedValue(0.0),
        hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("up"),
            parameter_key: String::from("Peak"),
            data_set_index: None,
        }),
        output_percent: true,
        per_data_set: true,
    };
    let err = curvebook_transform::execute_transform(&ds, &dirty_hook_spec, &ctx).unwrap_err();
    assert!(
        matches!(err, curvebook_transform::TransformError::MissingHook { .. }) && err.to_string().contains("dirty"),
        "Dirty ResultScalar must reject: {err:?}"
    );
}
