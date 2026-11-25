#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions for Stage 3A X2 data-correctness fixes (`R1-X2-01` and `R1-X2-02`)
//! across `doc`, `transform`, `fit`, `engine::dag`, and `chart::xy`.

use std::collections::HashSet;

use curvebook_chart::compile_graph;
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, ChecklistItem, ChecklistStatus, CiBound, ConstantValue, CurveSamplePoint, DataSheet,
    EmbeddedTableStyle, ErrorCode, FittedCurveOverlay, FloatingNote, GraphAnnotation, GraphSheet, HookSource, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, InfoSheet, InternalLinkKind, LinkSource, LinkedDataRegion, NonlinearFitSpec, NormalizeAnchor, PairwiseComparisonRecord,
    ProjectDocument, RectPt, RegressionOverlaySpec, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind,
    SheetCellRange, SheetId, SubcolumnMode, TableArchetype, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_engine::dag::recalculate_project_with_report;
use curvebook_fit::HookContext;
use curvebook_testkit::metrics::assert_approx_eq;
use curvebook_transform::TransformContext;
use serde_json::json;

use super::common::{ok_cmd, rs, session_with, xy_sheet};

#[test]
fn r1_x2_01_failed_recompute_clears_stale_outputs_and_consumers_reject_failed() {
    let mut s = session_with("x2-01", vec![xy_sheet("d", "d", "X", &[1.0, 2.0, 3.0, 4.0, 5.0], &[("Y", &[2.0, 4.1, 5.9, 8.2, 9.9])])]);
    ok_cmd(&mut s, "analysis.add", json!({"source_sheet_id": "d", "sheet_id": "lin", "kind": "linear"}));

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

    let mut doc2 = s.document().clone();
    doc2.add_result_sheet(ResultSheet::new(norm_spec).unwrap()).unwrap();
    doc2.add_result_sheet(ResultSheet::new(nl_spec).unwrap()).unwrap();
    assert!(recalculate_project_with_report(&mut doc2).unwrap().failed_result_sheets.is_empty());

    {
        let lin = doc2.result_sheets.iter_mut().find(|r| r.id.as_str() == "lin").unwrap();
        assert!(
            lin.status == ResultStatus::UpToDate
                && lin.lookup_scalar("Slope", None).is_some()
                && !lin.tabs.is_empty()
                && !lin.fitted_curves.is_empty()
        );
        lin.pairwise_comparisons.push(PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: "A".into(),
            label_b: "B".into(),
            estimate_diff: Some(1.0),
            raw_p_or_q: 0.01,
            is_fdr_q: false,
            significant: true,
        });
        lin.checklist.push(ChecklistItem {
            assumption: "Stale item".into(),
            status: ChecklistStatus::Pass,
            diagnostic_metric: None,
            explanation: "Stale detail".into(),
            remediation: None,
        });
        lin.floating_notes.push(FloatingNote::new("n1", RectPt::new(0.0, 0.0, 50.0, 20.0), "stale note"));
    }

    let mut s2 = Session::from_document(doc2);
    for r in 0..3 {
        let _ = s2.execute("table.excludeCells", json!({"sheet_id": "d", "row": r, "col": 0, "subcol": 0}));
    }

    for failed_id in ["lin", "tr", "nl"] {
        let res = rs(&s2, failed_id);
        assert!(matches!(res.status, ResultStatus::Failed { .. }) && res.lookup_scalar("Slope", None).is_none());
        assert!(
            res.scalars.is_empty()
                && res.tabs.is_empty()
                && res.fitted_curves.is_empty()
                && res.pairwise_comparisons.is_empty()
                && res.analyzable_data.is_none()
                && res.checklist.is_empty()
        );
        assert_eq!(res.floating_notes.len(), usize::from(failed_id == "lin"));
    }

    // Defense-in-depth checks on a synthetic Failed ResultSheet retaining stale fields:
    let mut stale_failed =
        ResultSheet::new(AnalysisSpec::new("fake_lin", "d", AnalysisKind::NonlinearRegression(NonlinearFitSpec::new("straight_line"))).unwrap())
            .unwrap();
    stale_failed.status = ResultStatus::Failed { code: ErrorCode::SolverFailed, message: "synthetic solver error".into() };
    stale_failed.scalars.push(ResultScalarEntry { key: "Slope".into(), data_set_index: None, value: 42.0, units: None });
    stale_failed.analyzable_data = Some(xy_sheet("fake_analyzable", "fake_analyzable", "X", &[1.0, 2.0, 3.0], &[("Y", &[9.0, 9.0, 9.0])]));
    stale_failed.tabs.push(ResultTab {
        id: "t0".into(),
        title: "Stale Tab".into(),
        kind: ResultTabKind::MainSummary,
        column_headers: vec!["Col".into()],
        row_labels: vec!["Row".into()],
        rows: vec![vec![ResultCell::Text("STALE_TABLE_TOKEN".into())]],
        footnotes: Vec::new(),
    });
    stale_failed.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: "straight_line".into(),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint { x: 1.0, y: 2.0, ci_lower: None, ci_upper: None, pred_lower: None, pred_upper: None }],
    });
    assert!(stale_failed.lookup_scalar("Slope", None).is_none());

    let dummy_ds = s2.document().data_sheets[0].clone();
    let failed_sheets = [stale_failed.clone()];
    let hook = HookSource::ResultScalar { result_sheet_id: SheetId::new("fake_lin"), parameter_key: "Slope".into(), data_set_index: None };
    let tr_ctx = TransformContext {
        info_sheet: None,
        info_sheets: &[],
        result_sheets: &failed_sheets,
        data_sheets: std::slice::from_ref(&dummy_ds),
        selected_data_sets: Vec::new(),
        output_id: None,
        output_title: None,
    };
    assert!(tr_ctx.resolve_hook(&hook, &dummy_ds).unwrap_err().to_string().contains("failed"));
    let fit_hooks = HookContext {
        info_sheets: &[],
        result_sheets: &failed_sheets,
        data_sheets: std::slice::from_ref(&dummy_ds),
        primary_info_sheet_id: None,
        visited_sheets: HashSet::new(),
    };
    assert!(fit_hooks.resolve(&hook).unwrap_err().to_string().contains("failed"));

    // Chart XY consumers must reject Failed sheets.
    let mut check_doc = ProjectDocument::new("chart-check").unwrap();
    check_doc.add_data_sheet(dummy_ds.clone()).unwrap();
    check_doc.add_result_sheet(stale_failed).unwrap();

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
        style: EmbeddedTableStyle::default(),
    });
    let scene = compile_graph(&gsheet, &check_doc).unwrap();
    assert!(scene.warnings.iter().any(|w| w.code == "failed_regression_result") && !format!("{:?}", scene.primitives).contains("STALE_TABLE_TOKEN"));

    let mut g_src = GraphSheet::new_uninitialized_for_data("g-fail-src", &dummy_ds).unwrap();
    g_src.initialized = true;
    g_src.spec.series[0].source_sheet_id = SheetId::new("fake_lin");
    assert!(compile_graph(&g_src, &check_doc).unwrap().plotted_points.is_empty());

    // Linked DataSheet inside DAG must refuse Failed upstream ResultSheet.
    let mut linked_ds = xy_sheet("d-linked", "d-linked", "X", &[1.0, 2.0], &[("Y", &[0.0, 0.0])]);
    linked_ds.linked_regions.push(LinkedDataRegion {
        id: "link-1".into(),
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
        provenance_summary: "from tr".into(),
    });
    let mut dag_doc = s2.document().clone();
    dag_doc.add_data_sheet(linked_ds).unwrap();
    assert!(recalculate_project_with_report(&mut dag_doc).unwrap_err().to_string().contains("failed"));
}

#[test]
fn r1_x2_02_mean_sd_n_regression_matches_raw_replicates() {
    let reps: [&[f64]; 5] = [&[1.0, 2.0, 3.0], &[3.0, 4.0, 6.0], &[4.0, 5.0, 7.0], &[6.0, 8.0, 9.0], &[7.0, 9.0, 10.0, 11.0, 12.0, 14.0]];
    let mut raw =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(6), 1, 5).unwrap();
    let mut sum = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 5).unwrap();
    for (r, v) in reps.iter().enumerate() {
        let x = (r + 1) as f64;
        raw.set_x_cell(r, Cell::number(x)).unwrap();
        sum.set_x_cell(r, Cell::number(x)).unwrap();
        for (k, &y) in v.iter().enumerate() {
            raw.data_sets[0].cells[r][k] = Cell::number(y);
        }
        let n = v.len() as f64;
        let m = v.iter().sum::<f64>() / n;
        let sd = (v.iter().map(|y| (y - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        sum.data_sets[0].cells[r][0] = Cell::number(m);
        sum.data_sets[0].cells[r][1] = Cell::number(sd);
        sum.data_sets[0].cells[r][2] = Cell::number(n);
    }

    let mut s = session_with(
        "x2-02",
        vec![DataSheet::new("raw", "raw", TableArchetype::Xy(raw)).unwrap(), DataSheet::new("sum", "sum", TableArchetype::Xy(sum)).unwrap()],
    );
    ok_cmd(&mut s, "analysis.add", json!({"source_sheet_id": "raw", "sheet_id": "lraw", "kind": "linear"}));
    ok_cmd(&mut s, "analysis.add", json!({"source_sheet_id": "sum", "sheet_id": "lsum", "kind": "linear"}));

    for key in ["Slope", "YIntercept", "R2"] {
        assert_approx_eq(rs(&s, "lsum").lookup_scalar(key, None).unwrap(), rs(&s, "lraw").lookup_scalar(key, None).unwrap(), 1e-10, 1e-10, key);
    }
    let (raw_tab, sum_tab) = (&rs(&s, "lraw").tabs[0], &rs(&s, "lsum").tabs[0]);
    assert_eq!(sum_tab.rows.len(), raw_tab.rows.len());
    for (ri, (gr, er)) in sum_tab.rows.iter().zip(raw_tab.rows.iter()).take(9).enumerate() {
        for (ci, (g, e)) in gr.iter().zip(er.iter()).enumerate() {
            match (g, e) {
                (ResultCell::Number { value: a, .. }, ResultCell::Number { value: b, .. }) => {
                    assert_approx_eq(*a, *b, 1e-10, 1e-10, &format!("t[{ri}][{ci}]"))
                }
                (
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(a0), upper: CiBound::Finite(a1) },
                    ResultCell::ConfidenceInterval { lower: CiBound::Finite(b0), upper: CiBound::Finite(b1) },
                ) => {
                    assert_approx_eq(*a0, *b0, 1e-10, 1e-10, "lo");
                    assert_approx_eq(*a1, *b1, 1e-10, 1e-10, "hi");
                }
                _ => assert_eq!(g, e),
            }
        }
    }
}

#[test]
fn sec03_03_dag_transform_multi_info_sheet_and_rejects_dirty_result_scalar() {
    let mut ds = xy_sheet("d", "d", "X", &[1.0, 2.0], &[("Y", &[10.0, 20.0])]);
    ds.info_sheet_id = Some(SheetId::new("info-a"));
    let mut info_a = InfoSheet::new_preseeded("info-a", "Info A").unwrap();
    info_a.set_constant("Scale", ConstantValue::Number(10.0), None).unwrap();
    let mut info_b = InfoSheet::new_preseeded("info-b", "Info B").unwrap();
    info_b.set_constant("Scale", ConstantValue::Number(100.0), None).unwrap();

    let mut doc = ProjectDocument::new("sec03-03").unwrap();
    doc.add_info_sheet(info_a).unwrap();
    doc.add_info_sheet(info_b).unwrap();
    doc.add_data_sheet(ds.clone()).unwrap();

    for (rid, info_id) in [("tr-b", Some(SheetId::new("info-b"))), ("tr-a", None)] {
        let spec = AnalysisSpec::new(
            rid,
            "d",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(0.0),
                hundred_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: info_id, constant_name: String::from("Scale") }),
                output_percent: true,
                per_data_set: true,
            }),
        )
        .unwrap();
        doc.add_result_sheet(ResultSheet::new(spec).unwrap()).unwrap();
    }
    recalculate_project_with_report(&mut doc).unwrap();

    for (rid, expected) in [("tr-b", 10.0), ("tr-a", 100.0)] {
        let TableArchetype::Xy(xy) = &doc.result_sheet(&SheetId::new(rid)).unwrap().analyzable_data.as_ref().unwrap().archetype else {
            panic!("expected XY")
        };
        assert_approx_eq(xy.data_sets[0].cells[0][0].active_number().unwrap(), expected, 1e-12, 0.0, rid);
    }

    // Stale Dirty upstream ResultSheet scalar hook must be rejected by TransformContext::resolve_hook.
    let mut dirty_res = ResultSheet::new(
        AnalysisSpec::new("up", "d", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap(),
    )
    .unwrap();
    dirty_res.status = ResultStatus::Dirty;
    dirty_res.scalars.push(ResultScalarEntry { key: String::from("Peak"), value: 42.0, units: None, data_set_index: None });
    let ctx = TransformContext { result_sheets: std::slice::from_ref(&dirty_res), ..TransformContext::default() };
    let dirty_spec = TransformAnalysisSpec::Normalize {
        zero_anchor: NormalizeAnchor::FixedValue(0.0),
        hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("up"),
            parameter_key: String::from("Peak"),
            data_set_index: None,
        }),
        output_percent: true,
        per_data_set: true,
    };
    let err = curvebook_transform::execute_transform(&ds, &dirty_spec, &ctx).unwrap_err();
    assert!(matches!(err, curvebook_transform::TransformError::MissingHook { .. }) && err.to_string().contains("dirty"));
}
