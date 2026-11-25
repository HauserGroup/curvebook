//! Cross-cutting data-fidelity regression suite for SEC-X2 findings `FA-X2-02`..`FA-X2-08`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CiBound, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ErrorBarDirection,
    ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphicalDataObject, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind,
    LinearRegressionSpec, LinkSource, MultipleVariablesTable, OneWayAnovaSpec, OneWayMethod, ProjectDocument, ResultCell, ResultSheet, ResultStatus,
    ResultTab, ResultTabKind, SequenceProgression, SheetCellRange, SheetId, SubcolumnMode, SummaryMeasure, SummaryPointError, TableArchetype,
    TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_engine::Session;
use curvebook_fit::{HookContext, execute_analysis as execute_fit_analysis};
use curvebook_format::{CsvOptions, ZipArchive, export_datasheet_to_csv, parse_delimited_records, save_project_bytes, save_project_unpacked_dir};
use curvebook_math::ConfidenceLevel;
use curvebook_stats::column::summarize_pre_summarized_points;
use curvebook_stats::run_analysis as execute_stats_analysis;
use curvebook_transform::{AucOptions, CurveSheetOp, DerivativeOrder, apply_curve_calculus_to_sheet, compute_sheet_auc, evaluate_formula_for_sheet};
use serde_json::json;

use super::common::{TempDir, approx_eq, col_sheet, ds, gs, ok_cmd, rs, run_cmds, session_with, xy_sheet};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn fa_x2_02_analyzable_result_sheet_initial_paste_link_matches_dag_refresh() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-02 Analyzable Link Parity")?;
    doc.add_data_sheet(col_sheet("src", "Source Col", &[("Ctrl", &[10.0, 20.0, 30.0]), ("Drug", &[100.0, 200.0, 300.0])]))?;
    doc.add_data_sheet(DataSheet::new(
        "dst",
        "Linked Target",
        TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2)?),
    )?)?;
    doc.add_result_sheet(ResultSheet::new(AnalysisSpec::new(
        "res-tr",
        "src",
        AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }),
    )?)?)?;

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "project.recalculate", json!({}));
    let res_tr = rs(&session, "res-tr");
    let analyzable_snapshot = res_tr.analyzable_data.as_ref().ok_or("analyzable_data")?.clone();
    let (expected_tab_id, tab_cols, tab_rows) =
        (res_tr.tabs[0].id.clone(), res_tr.tabs[0].column_headers.len().max(1), res_tr.tabs[0].rows.len().max(1));

    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "dst", "source_sheet_id": "res-tr", "source_kind": "AnalyzableResultSheet" }));
    let dst_initial = ds(&session, "dst");
    assert_eq!(dst_initial.linked_regions.len(), 1);
    assert!(
        matches!(&dst_initial.linked_regions[0].spec.source, LinkSource::InternalSheet { kind: InternalLinkKind::AnalyzableResultSheet, tab_id: None, range, .. } if *range == SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 2, 0)))
    );
    let (TableArchetype::Column(init_tbl), TableArchetype::Column(ana_tbl)) = (&dst_initial.archetype, &analyzable_snapshot.archetype) else {
        panic!("expected Column")
    };
    assert_eq!(init_tbl.data_sets.iter().map(|d| &d.cells).collect::<Vec<_>>(), ana_tbl.data_sets.iter().map(|d| &d.cells).collect::<Vec<_>>());
    approx_eq(init_tbl.data_sets[1].cells[0][0].active_number().unwrap(), 20.0, 1e-12, "r0c1");
    approx_eq(init_tbl.data_sets[2].cells[1][0].active_number().unwrap(), 300.0, 1e-12, "r1c2");

    let initial_source = dst_initial.linked_regions[0].spec.source.clone();
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 1, "value": 999.0 }));
    let dst_after = ds(&session, "dst");
    assert_eq!(dst_after.linked_regions[0].spec.source, initial_source);
    let TableArchetype::Column(refreshed_tbl) = &dst_after.archetype else { panic!("expected Column") };
    approx_eq(refreshed_tbl.data_sets[2].cells[1][0].active_number().unwrap(), 999.0, 1e-12, "refreshed r1c2");

    let mut doc2 = session.document().clone();
    doc2.add_data_sheet(DataSheet::new(
        "dst-tab",
        "Tab Target",
        TableArchetype::MultipleVariables(MultipleVariablesTable::new(tab_cols, tab_rows)?),
    )?)?;
    let mut s2 = Session::from_document(doc2);
    ok_cmd(
        &mut s2,
        "table.pasteLink",
        json!({ "target_sheet_id": "dst-tab", "source_sheet_id": "res-tr", "source_kind": "AnalyzableResultSheet", "tab_id": expected_tab_id }),
    );
    assert!(matches!(
        ds(&s2, "dst-tab").linked_regions[0].spec.source,
        LinkSource::InternalSheet { tab_id: Some(_), kind: InternalLinkKind::AnalyzableResultSheet, .. }
    ));
    Ok(())
}

#[test]
fn fa_x2_03_parametric_unknown_y_rows_interpolate_without_sd_n_or_with_zero_mean_cv() -> TestResult {
    let spec = AnalysisSpec::new(
        "fit-unk",
        "xy-unk",
        AnalysisKind::LinearRegression(LinearRegressionSpec { interpolate_unknowns: true, ..LinearRegressionSpec::default() }),
    )?;
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, 6)?;
        for (r, x) in [0.0, 1.0, 2.0, 3.0].into_iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x))?;
            xy.data_sets[0].cells[r] = vec![Cell::number(2.0 * x + 10.0), Cell::number(1.5), Cell::number(3.0)];
        }
        xy.data_sets[0].cells[4] = vec![Cell::number(14.0), Cell::empty(), Cell::empty()];
        xy.data_sets[0].cells[5] = vec![Cell::number(0.0), Cell::empty(), Cell::empty()];

        let res =
            execute_fit_analysis(&spec, &DataSheet::new("xy-unk", "Parametric Unknowns", TableArchetype::Xy(xy.clone()))?, &HookContext::default())?;
        assert_eq!(res.status, ResultStatus::UpToDate);
        let tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).ok_or("InterpolatedUnknowns")?;
        let xs_out: Vec<f64> = tab
            .rows
            .iter()
            .map(|r| match r[1] {
                ResultCell::Number { value, .. } => value,
                _ => panic!("expected numeric"),
            })
            .collect();
        assert_eq!(xs_out.len(), 2);
        approx_eq(xs_out[0], 2.0, 1e-6, "Y=14");
        approx_eq(xs_out[1], -5.0, 1e-6, "Y=0");

        let mut bad_cal = xy;
        bad_cal.data_sets[0].cells[3] = vec![Cell::number(16.0), Cell::empty(), Cell::empty()];
        assert!(execute_fit_analysis(&spec, &DataSheet::new("xy-bad-cal", "Bad", TableArchetype::Xy(bad_cal))?, &HookContext::default()).is_err());
    }
    Ok(())
}

#[test]
fn fa_x2_04_x_summary_at_parity_across_doc_transform_stats_and_fit() -> TestResult {
    let mut xy_reps =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) }, SubcolumnMode::Single, 1, 5)?;
    xy_reps.x_cells[0] = vec![Cell::number(1.0), Cell::excluded_number(3.0), Cell::number(5.0)];
    xy_reps.x_cells[1] = vec![Cell::number(5.0), Cell::number(7.0), Cell::number(9.0)];
    xy_reps.x_cells[2] = vec![Cell::number(10.0), Cell::number(12.0), Cell::number(14.0)];
    xy_reps.x_cells[3][0] = Cell::excluded_number(2.0);
    xy_reps.x_cells[4][0] = Cell::number(f64::MAX);
    xy_reps.x_cells[4][1] = Cell::number(f64::MAX);
    assert_eq!(
        (
            xy_reps.x_summary_at(0)?,
            xy_reps.x_summary_at(1)?,
            xy_reps.x_summary_at(2)?,
            xy_reps.x_summary_at(3)?,
            xy_reps.x_summary_at(4)?,
            xy_reps.x_summary_at(99)?
        ),
        (Some(3.0), Some(7.0), Some(12.0), None, Some(f64::MAX), None)
    );

    let mut xy_cv = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanCvN }, SubcolumnMode::Single, 1, 2)?;
    xy_cv.x_cells[0] = vec![Cell::number(10.0), Cell::number(5.0), Cell::number(4.0)];
    xy_cv.x_cells[1] = vec![Cell::number(0.0), Cell::number(5.0), Cell::number(4.0)];
    xy_cv.data_sets[0].cells[0][0] = Cell::number(20.0);
    xy_cv.data_sets[0].cells[1][0] = Cell::number(30.0);
    assert_eq!(xy_cv.x_summary_at(0)?, Some(10.0));
    assert!(matches!(xy_cv.x_summary_at(1), Err(SummaryPointError::MissingSpread)));

    let mut xy_rep = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) }, SubcolumnMode::Single, 1, 4)?;
    for (r, (xs, y)) in
        [([1.0, 3.0, 5.0], 6.0), ([4.0, 6.0, 8.0], 12.0), ([7.0, 9.0, 11.0], 18.0), ([10.0, 12.0, 14.0], 24.0)].into_iter().enumerate()
    {
        xy_rep.x_cells[r] = xs.into_iter().map(Cell::number).collect();
        xy_rep.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds_rep = DataSheet::new("ds-xrep", "RepX", TableArchetype::Xy(xy_rep))?;
    let ds_sgl = xy_sheet("ds-xsingle", "SingleX", "X", &[3.0, 6.0, 9.0, 12.0], &[("A", &[6.0, 12.0, 18.0, 24.0])]);

    let fit_rep = execute_fit_analysis(
        &AnalysisSpec::new("f-rep", "ds-xrep", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
        &ds_rep,
        &HookContext::default(),
    )?;
    let fit_sgl = execute_fit_analysis(
        &AnalysisSpec::new("f-sgl", "ds-xsingle", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
        &ds_sgl,
        &HookContext::default(),
    )?;
    approx_eq(fit_rep.lookup_scalar("Slope", Some(0)).unwrap(), 2.0, 1e-12, "slope");
    approx_eq(fit_rep.lookup_scalar("YIntercept", Some(0)).unwrap(), fit_sgl.lookup_scalar("YIntercept", Some(0)).unwrap(), 1e-12, "intercept");
    approx_eq(
        compute_sheet_auc(&ds_rep, 0, &AucOptions::default())?.net_area,
        compute_sheet_auc(&ds_sgl, 0, &AucOptions::default())?.net_area,
        1e-12,
        "auc",
    );
    assert_eq!(
        apply_curve_calculus_to_sheet(&ds_rep, CurveSheetOp::Integrate { baseline: 0.0 })?.archetype,
        apply_curve_calculus_to_sheet(&ds_sgl, CurveSheetOp::Integrate { baseline: 0.0 })?.archetype
    );
    approx_eq(
        apply_curve_calculus_to_sheet(&ds_rep, CurveSheetOp::Differentiate(DerivativeOrder::First))?
            .cell(CellCoord::new(0, 0, 0))
            .unwrap()
            .active_number()
            .unwrap(),
        2.0,
        1e-12,
        "deriv",
    );
    assert_eq!(
        evaluate_formula_for_sheet("= x * 2", &ds_rep, None)?.iter().filter_map(|v| v.as_finite_f64()).collect::<Vec<_>>(),
        vec![6.0, 12.0, 18.0, 24.0]
    );

    let desc_rep =
        execute_stats_analysis(&ds_rep, &AnalysisSpec::new("d-rep", "ds-xrep", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    let desc_sgl =
        execute_stats_analysis(&ds_sgl, &AnalysisSpec::new("d-sgl", "ds-xsingle", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    approx_eq(
        desc_rep.lookup_scalar("Trapezoidal AUC", Some(0)).unwrap(),
        desc_sgl.lookup_scalar("Trapezoidal AUC", Some(0)).unwrap(),
        1e-12,
        "stats auc",
    );

    let ds_bad = DataSheet::new("ds-bad-cv", "BadCvX", TableArchetype::Xy(xy_cv))?;
    assert!(
        execute_fit_analysis(
            &AnalysisSpec::new("f-bad", "ds-bad-cv", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
            &ds_bad,
            &HookContext::default()
        )
        .is_err()
    );
    assert!(compute_sheet_auc(&ds_bad, 0, &AucOptions::default()).is_err() && evaluate_formula_for_sheet("= x * 2", &ds_bad, None).is_err());
    assert!(
        execute_stats_analysis(&ds_bad, &AnalysisSpec::new("d-bad", "ds-bad-cv", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)
            .is_err()
    );
    Ok(())
}

fn extract_vertical_bar_top_y(scene: &ChartScene) -> Option<f64> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Path { path, fill: Some(_), .. } => path
                .commands
                .iter()
                .filter_map(|c| match c {
                    curvebook_chart::PathCmd::MoveTo(pt) | curvebook_chart::PathCmd::LineTo(pt) => Some(pt.y),
                    _ => None,
                })
                .reduce(f64::min),
            _ => None,
        })
        .reduce(f64::min)
}

#[test]
fn fa_x2_05_column_presummarized_pooling_and_validation_parity_across_doc_stats_chart() -> TestResult {
    let mut col_pre = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 3, 2)?;
    for (c, (t, rows)) in
        [("G1", [[10.0, 0.0, 1.0], [14.0, 0.0, 1.0]]), ("G2", [[20.0, 2.0, 3.0], [26.0, 4.0, 5.0]]), ("G3", [[30.0, 1.0, 2.0], [32.0, 3.0, 2.0]])]
            .into_iter()
            .enumerate()
    {
        col_pre.data_sets[c].title = t.to_owned();
        col_pre.data_sets[c].cells = rows.into_iter().map(|r| r.into_iter().map(Cell::number).collect()).collect();
    }
    let ds_pre = DataSheet::new("col-pre", "PrePool", TableArchetype::Column(col_pre.clone()))?;
    let pool = |c: usize| -> Result<_, Box<dyn std::error::Error>> {
        let pts = vec![col_pre.summarized_point(0, c)?.ok_or("pt0")?, col_pre.summarized_point(1, c)?.ok_or("pt1")?];
        Ok(summarize_pre_summarized_points(&col_pre.data_sets[c].title, &pts, ConfidenceLevel::DEFAULT_95)?)
    };
    let (p0, p1, p2) = (pool(0)?, pool(1)?, pool(2)?);
    approx_eq(p0.total_n, 2.0, 1e-12, "N");
    approx_eq(p0.mean, 12.0, 1e-12, "mean");
    approx_eq(p0.sd.unwrap(), 8.0_f64.sqrt(), 1e-12, "sd");
    approx_eq(p0.sem.unwrap(), 2.0, 1e-12, "sem");

    let ow_ord = execute_stats_analysis(&ds_pre, &AnalysisSpec::new("ow-ord", "col-pre", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?)?;
    let ow_welch = execute_stats_analysis(
        &ds_pre,
        &AnalysisSpec::new(
            "ow-welch",
            "col-pre",
            AnalysisKind::OneWayAnova(OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..Default::default() }),
        )?,
    )?;
    assert_eq!((&ow_ord.status, &ow_welch.status), (&ResultStatus::UpToDate, &ResultStatus::UpToDate));
    let ms_w =
        ((p0.total_n - 1.0) * p0.sd.unwrap().powi(2) + (p1.total_n - 1.0) * p1.sd.unwrap().powi(2) + (p2.total_n - 1.0) * p2.sd.unwrap().powi(2))
            / 11.0;
    approx_eq(ow_ord.lookup_scalar("MS residual", None).ok_or("MS")?, ms_w, 1e-9, "MS residual");
    assert!(ow_welch.lookup_scalar("Welch W", None).unwrap().is_finite());

    let mut col_pre_1 = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 2)?;
    col_pre_1.data_sets[0].title = String::from("A");
    col_pre_1.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(0.0), Cell::number(1.0)], vec![Cell::number(14.0), Cell::number(0.0), Cell::number(1.0)]];
    let ds_p1 = DataSheet::new("c-p1", "SameTitle", TableArchetype::Column(col_pre_1))?;
    let ds_r1 = col_sheet("c-r1", "SameTitle", &[("A", &[10.0, 14.0])]);

    let render_bar = |sheet: &DataSheet| -> Result<ChartScene, Box<dyn std::error::Error>> {
        let mut doc = ProjectDocument::new("Chart")?;
        doc.add_data_sheet(sheet.clone())?;
        let mut g = GraphSheet::new_uninitialized_for_data("g", sheet)?;
        g.initialized = true;
        g.spec.y_axis.segments[0].min = Some(-20.0);
        g.spec.y_axis.segments[0].max = Some(50.0);
        g.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: g.spec.palette.color_at(0),
            border_color: g.spec.palette.color_at(0),
            border_width_pt: 0.75,
            replicate_overlay: None,
        };
        g.spec.series[0].y_error_bars = Some(ErrorBarSpec {
            metric: ErrorBarMetric::ConfidenceInterval95,
            direction: ErrorBarDirection::Both,
            dash_style: curvebook_palette::LineDashStyle::Solid,
            stroke_width_pt: 1.0,
            cap_width_pt: 4.0,
            color: None,
        });
        Ok(compile_graph(&g, &doc)?)
    };
    let (scene_p, scene_r) = (render_bar(&ds_p1)?, render_bar(&ds_r1)?);
    assert!(scene_p.warnings.is_empty() && scene_p.primitives == scene_r.primitives);
    approx_eq(extract_vertical_bar_top_y(&scene_p).unwrap(), extract_vertical_bar_top_y(&scene_r).unwrap(), 1e-6, "bar height");

    let mut col_bad = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanCvN, 2, 1)?;
    col_bad.data_sets[0].cells[0] = vec![Cell::number(0.0), Cell::number(5.0), Cell::number(3.0)];
    col_bad.data_sets[1].cells[0] = vec![Cell::number(10.0), Cell::number(5.0), Cell::number(3.0)];
    assert!(matches!(col_bad.summarized_point(0, 0), Err(SummaryPointError::MissingSpread)));
    let ds_bad = DataSheet::new("c-bad", "BadCV", TableArchetype::Column(col_bad))?;
    assert!(execute_stats_analysis(&ds_bad, &AnalysisSpec::new("ow-bad", "c-bad", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?).is_err());
    let mut doc_bad = ProjectDocument::new("BadChart")?;
    doc_bad.add_data_sheet(ds_bad.clone())?;
    let mut gs_bad = GraphSheet::new_uninitialized_for_data("g-bad", &ds_bad)?;
    gs_bad.initialized = true;
    assert!(compile_graph(&gs_bad, &doc_bad)?.warnings.iter().any(|w| w.code == "invalid_pre_summarized_point"));
    Ok(())
}

#[test]
fn fa_x2_06_override_series_slot_contract_insert_rows_shift_and_prune() -> TestResult {
    let mut session = session_with(
        "FA-X2-06 Overrides",
        vec![
            xy_sheet(
                "xy-main",
                "Main",
                "X",
                &[1.0, 2.0, 3.0, 4.0],
                &[("A", &[10.0, 11.0, 12.0, 13.0]), ("B", &[20.0, 21.0, 22.0, 23.0]), ("C", &[30.0, 31.0, 32.0, 33.0])],
            ),
            col_sheet("col-src", "ColSrc", &[("S", &[10.0, 20.0, 30.0, 40.0])]),
            col_sheet("col-dst", "ColDst", &[("D", &[0.0, 0.0, 0.0, 0.0])]),
        ],
    );
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-main", "kind": "xy" }));
    let base_series = gs(&session, "g-xy").spec.series.clone();
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g-xy", "mode": "replace", "series": [base_series[2], base_series[0]] }));
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-xy", "points": [{ "series_index": 0, "point_index": 1 }], "hidden": true }),
    );
    assert!(
        session
            .execute("graph.formatSelectedPoints", json!({ "sheet_id": "g-xy", "points": [{ "series_index": 2, "point_index": 0 }], "hidden": true }))
            .is_err()
    );

    let scene_xy = compile_graph(gs(&session, "g-xy"), session.document())?;
    assert!(!scene_xy.plotted_points.iter().any(|p| p.source_cell.col == 2 && p.source_cell.row == 1));
    assert_eq!(scene_xy.plotted_points.iter().filter(|p| p.source_cell.col == 0).count(), 4);

    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g-xy", "mode": "replace", "series": [base_series[0], base_series[2]] }));
    assert_eq!(gs(&session, "g-xy").spec.point_overrides[0].series_index, 1);

    run_cmds(
        &mut session,
        &[
            ("graph.formatSelectedPoints", json!({ "sheet_id": "g-xy", "points": [{ "series_index": 1, "point_index": 0 }], "color": "#ff0000" })),
            ("table.paste", json!({ "sheet_id": "xy-main", "mode": "insert_rows", "start_row": 1, "start_col": 0, "text": "99\t100\n98\t101\n" })),
        ],
    );
    let ovs = &gs(&session, "g-xy").spec.point_overrides;
    assert!(
        ovs.iter().any(|o| o.series_index == 1 && o.point_index == 0) && ovs.iter().any(|o| o.series_index == 1 && o.point_index == 3 && o.hidden)
    );

    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "xy-main", "mode": "insert_rows", "start_row": 2, "has_header": false, "csv": "50,51,52,53\n" }),
    );
    assert!(gs(&session, "g-xy").spec.point_overrides.iter().any(|o| o.series_index == 1 && o.point_index == 4 && o.hidden));
    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({ "sheet_id": "xy-main", "replace": true, "archetype": "xy", "csv": "X,A,B,C\n1,10,20,30\n2,11,21,31\n" }),
    );
    assert!(!gs(&session, "g-xy").spec.point_overrides.iter().any(|o| o.point_index >= 2));

    run_cmds(
        &mut session,
        &[
            ("table.pasteLink", json!({ "target_sheet_id": "col-dst", "source_sheet_id": "col-src" })),
            ("graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-dst", "kind": "column" })),
            ("graph.formatSelectedPoints", json!({ "sheet_id": "g-col", "points": [{ "series_index": 0, "point_index": 3 }], "hidden": true })),
        ],
    );
    assert_eq!(gs(&session, "g-col").spec.point_overrides.len(), 1);
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-src", "row": 3, "col": 0, "value": null }));
    assert!(gs(&session, "g-col").spec.point_overrides.is_empty());
    Ok(())
}

#[test]
fn fa_x2_07_generated_sequence_csv_export_and_archive_sidecars_emit_x_and_exclusions() -> TestResult {
    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        3,
    )?;
    xy.x_title = String::from("Time (min)");
    xy.data_sets[0].title = String::from("Signal");
    xy.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(20.0)], vec![Cell::number(30.0)]];
    let mut sheet = DataSheet::new("xy-gen", "Generated X", TableArchetype::Xy(xy))?;
    sheet.set_x_or_time_excluded(1, 0, true)?;

    let csv_out = export_datasheet_to_csv(&sheet, &CsvOptions::csv())?;
    assert_eq!(
        parse_delimited_records(&csv_out, &CsvOptions::csv())?,
        vec![vec!["Time (min)", "Signal"], vec!["1", "10"], vec!["3*", "20"], vec!["5", "30"]]
    );

    let mut session = session_with("FA-X2-07 GenSeq Export", vec![sheet]);
    let bytes = save_project_bytes(session.document())?;
    let archive = ZipArchive::from_bytes(&bytes)?;
    let (_, zip_bytes) = archive.iter().find(|(n, _)| n.starts_with("sheets/") && n.ends_with(".csv")).ok_or("zip csv")?;
    assert_eq!(std::str::from_utf8(zip_bytes)?, csv_out);

    let guard = TempDir::try_new("genseq")?;
    let unpacked_dir = guard.child("project.curvebook.d");
    save_project_unpacked_dir(session.document(), &unpacked_dir)?;
    let entry = fs::read_dir(unpacked_dir.join("sheets"))?.next().ok_or("sidecar")??;
    assert_eq!(fs::read_to_string(entry.path())?, csv_out);
    assert_eq!(ok_cmd(&mut session, "table.exportCsv", json!({ "sheet_id": "xy-gen" })).payload["csv"].as_str(), Some(csv_out.as_str()));
    Ok(())
}

#[test]
fn fa_x2_08_confidence_interval_cells_place_accurately_across_filter_decimate_unstack_and_transpose() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-08 Native CI Placement")?;
    doc.add_data_sheet(col_sheet("dummy", "Dummy", &[("A", &[1.0, 2.0])]))?;
    let mut r = ResultSheet::new(AnalysisSpec::new("res-ci", "dummy", AnalysisKind::DescriptiveStats(Default::default()))?)?;
    r.status = ResultStatus::UpToDate;
    let num = |v: f64| ResultCell::Number { value: v, prefix: None, units: None };
    r.tabs = vec![ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        row_labels: (0..6).map(|i| format!("R{i}")).collect(),
        column_headers: vec![String::from("Mean"), String::from("Group"), String::from("CI")],
        rows: vec![
            vec![
                num(10.0),
                ResultCell::Text(String::from("A")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(8.5), upper: CiBound::Finite(11.5) },
            ],
            vec![
                num(20.0),
                ResultCell::Text(String::from("B")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(17.0), upper: CiBound::Open },
            ],
            vec![ResultCell::Empty; 3],
            vec![ResultCell::Empty; 3],
            vec![
                num(30.0),
                ResultCell::Text(String::from("A")),
                ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(33.25) },
            ],
            vec![
                num(40.0),
                ResultCell::Text(String::from("B")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(38.0), upper: CiBound::Finite(42.0) },
            ],
        ],
        footnotes: Vec::new(),
    }];
    doc.add_result_sheet(r)?;
    for (id, cols, rows) in [("dst-dec", 3, 6), ("dst-tr", 6, 3), ("dst-un", 6, 4)] {
        doc.add_data_sheet(DataSheet::new(id, id, TableArchetype::MultipleVariables(MultipleVariablesTable::new(cols, rows)?))?)?;
    }

    let mut session = Session::from_document(doc);
    let link_src = |end_row| LinkSource::InternalSheet {
        sheet_id: SheetId::new("res-ci"),
        kind: InternalLinkKind::TerminalResultSheet,
        tab_id: Some(String::from("summary")),
        range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(end_row, 2, 0)),
    };
    run_cmds(
        &mut session,
        &[
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst-dec", "link_spec": ImportLinkSpec { source: link_src(5), filter: ImportFilterSpec { decimate_every_k: Some(2), ..Default::default() }, placement: ImportPlacementSpec { skip_contiguous_blanks: true, ..Default::default() } } }),
            ),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst-tr", "link_spec": ImportLinkSpec { source: link_src(1), filter: Default::default(), placement: ImportPlacementSpec { transpose: true, ..Default::default() } } }),
            ),
            (
                "table.pasteLink",
                json!({ "target_sheet_id": "dst-un", "link_spec": ImportLinkSpec { source: link_src(5), filter: ImportFilterSpec { col_window: Some((1, 2)), unstack_group_column: Some(0), ..Default::default() }, placement: Default::default() } }),
            ),
            ("sheet.extractResultTab", json!({ "result_sheet_id": "res-ci", "tab_id": "summary", "sheet_id": "ext-ci" })),
        ],
    );

    let dec = ds(&session, "dst-dec");
    assert_eq!(dec.cell(CellCoord::new(0, 2, 0)).unwrap().value, CellValue::Text(String::from("[8.5, 11.5]")));
    assert_eq!(dec.cell(CellCoord::new(1, 2, 0)).unwrap().value, CellValue::Empty);
    assert_eq!(dec.cell(CellCoord::new(2, 2, 0)).unwrap().value, CellValue::Text(String::from("[-inf, 33.25]")));
    assert!(
        dec.cell(CellCoord::new(4, 2, 0)).unwrap().value == CellValue::Empty && dec.cell(CellCoord::new(5, 2, 0)).unwrap().value == CellValue::Empty
    );

    let tr = ds(&session, "dst-tr");
    assert_eq!(
        (&tr.cell(CellCoord::new(2, 0, 0)).unwrap().value, &tr.cell(CellCoord::new(2, 1, 0)).unwrap().value),
        (&CellValue::Text(String::from("[8.5, 11.5]")), &CellValue::Text(String::from("[17, +inf]")))
    );

    let un = ds(&session, "dst-un");
    let texts: Vec<String> = (0..un.archetype.row_count())
        .flat_map(|r| {
            (0..un.archetype.column_count()).filter_map(move |c| match un.cell(CellCoord::new(r, c, 0)).map(|cell| &cell.value) {
                Some(CellValue::Text(s)) => Some(s.clone()),
                _ => None,
            })
        })
        .collect();
    for expected in ["[8.5, 11.5]", "[-inf, 33.25]", "[17, +inf]", "[38, 42]"] {
        assert!(texts.iter().any(|t| t == expected), "missing {expected} in {texts:?}");
    }
    let ext = ds(&session, "ext-ci");
    assert_eq!(
        (&ext.cell(CellCoord::new(0, 2, 0)).unwrap().value, &ext.cell(CellCoord::new(4, 2, 0)).unwrap().value),
        (&CellValue::Text(String::from("[8.5, 11.5]")), &CellValue::Text(String::from("[-inf, 33.25]")))
    );
    Ok(())
}
