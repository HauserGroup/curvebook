//! Cross-cutting data-fidelity regression suite for SEC-X2 findings `FA-X2-02`..`FA-X2-08`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use curvebook_chart::{ChartScene, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, CiBound, ColumnDesignHint, ColumnTable, DataSheet, DescriptiveSpec, ErrorBarDirection,
    ErrorBarMetric, ErrorBarSpec, GraphSheet, GraphicalDataObject, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InternalLinkKind,
    LinearRegressionSpec, LinkSource, OneWayAnovaSpec, OneWayMethod, ProjectDocument, ResultCell, ResultSheet, ResultStatus, ResultTab,
    ResultTabKind, SequenceProgression, SheetCellRange, SheetId, SubcolumnMode, SummaryMeasure, SummaryPointError, TableArchetype,
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

use super::common::{TempDir, approx_eq, ok_cmd};

type TestResult = Result<(), Box<dyn std::error::Error>>;

// ============================================================================
// FA-X2-02: Unify AnalyzableResultSheet (tab_id: None) initial paste vs DAG refresh
// ============================================================================

#[test]
fn fa_x2_02_analyzable_result_sheet_initial_paste_link_matches_dag_refresh() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-02 Analyzable Link Parity")?;
    let mut src_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3)?;
    src_col.data_sets[0].title = String::from("Ctrl");
    src_col.data_sets[1].title = String::from("Drug");
    for r in 0..3 {
        src_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
        src_col.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64 * 100.0);
    }
    doc.add_data_sheet(DataSheet::new("src", "Source Col", TableArchetype::Column(src_col))?)?;

    // Target ColumnTable (3 cols x 2 rows) that will live-link from the Transpose ResultSheet (`tab_id: None`).
    let dst_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2)?;
    doc.add_data_sheet(DataSheet::new("dst", "Linked Target", TableArchetype::Column(dst_col))?)?;

    let tr_spec =
        AnalysisSpec::new("res-tr", "src", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false }))?;
    doc.add_result_sheet(ResultSheet::new(tr_spec)?)?;

    let mut session = Session::from_document(doc);
    ok_cmd(&mut session, "project.recalculate", json!({}));

    let res_tr = session.document().result_sheet(&SheetId::new("res-tr")).ok_or("missing res-tr")?;
    let analyzable_snapshot = res_tr.analyzable_data.as_ref().ok_or("expected analyzable_data on Transform ResultSheet")?.clone();
    let expected_tab_id = res_tr.tabs[0].id.clone();
    let tab_cols = res_tr.tabs[0].column_headers.len().max(1);
    let tab_rows = res_tr.tabs[0].rows.len().max(1);

    // Initial pasteLink with `tab_id: None` and no explicit `source_range` must resolve `analyzable_data` (2 rows x 3 cols),
    // NOT `tabs[0]` (`"Transform Summary"` metadata tab!).
    ok_cmd(
        &mut session,
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst",
            "source_sheet_id": "res-tr",
            "source_kind": "AnalyzableResultSheet",
        }),
    );

    let dst_initial = session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst")?;
    assert_eq!(dst_initial.linked_regions.len(), 1);
    let LinkSource::InternalSheet { kind, tab_id, range, .. } = &dst_initial.linked_regions[0].spec.source else {
        panic!("expected InternalSheet link source");
    };
    assert_eq!(*kind, InternalLinkKind::AnalyzableResultSheet);
    assert!(tab_id.is_none());
    assert_eq!(*range, SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 2, 0)));

    let TableArchetype::Column(ref initial_tbl) = dst_initial.archetype else {
        panic!("expected Column dst");
    };
    let TableArchetype::Column(ref ana_tbl) = analyzable_snapshot.archetype else {
        panic!("expected Column analyzable_data");
    };
    assert_eq!(initial_tbl.data_sets.iter().map(|d| &d.cells).collect::<Vec<_>>(), ana_tbl.data_sets.iter().map(|d| &d.cells).collect::<Vec<_>>());
    approx_eq(initial_tbl.data_sets[1].cells[0][0].active_number().unwrap(), 20.0, 1e-12, "initial transposed r0c1");
    approx_eq(initial_tbl.data_sets[2].cells[1][0].active_number().unwrap(), 300.0, 1e-12, "initial transposed r1c2");

    let initial_source = dst_initial.linked_regions[0].spec.source.clone();

    // Mutate upstream source cell -> DAG refresh must keep exact same range and update transposed cell r1c2 to 999.
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "src", "row": 2, "col": 1, "value": 999.0 }));
    let dst_after = session.document().data_sheet(&SheetId::new("dst")).ok_or("missing dst after recalc")?;
    assert_eq!(dst_after.linked_regions[0].spec.source, initial_source);
    let TableArchetype::Column(ref refreshed_tbl) = dst_after.archetype else {
        panic!("expected Column dst");
    };
    approx_eq(refreshed_tbl.data_sets[2].cells[1][0].active_number().unwrap(), 999.0, 1e-12, "refreshed r1c2");

    // Explicitly passing `tab_id: Some(...)` on the same AnalyzableResultSheet resolves the summary ResultTab instead.
    let mut doc2 = session.document().clone();
    doc2.add_data_sheet(DataSheet::new(
        "dst-tab",
        "Tab Target",
        TableArchetype::MultipleVariables(curvebook_doc::MultipleVariablesTable::new(tab_cols, tab_rows)?),
    )?)?;
    let mut s2 = Session::from_document(doc2);
    ok_cmd(
        &mut s2,
        "table.pasteLink",
        json!({
            "target_sheet_id": "dst-tab",
            "source_sheet_id": "res-tr",
            "source_kind": "AnalyzableResultSheet",
            "tab_id": expected_tab_id,
        }),
    );
    let ds_tab = s2.document().data_sheet(&SheetId::new("dst-tab")).ok_or("missing dst-tab")?;
    assert!(matches!(
        ds_tab.linked_regions[0].spec.source,
        LinkSource::InternalSheet { tab_id: Some(_), kind: InternalLinkKind::AnalyzableResultSheet, .. }
    ));
    Ok(())
}

// ============================================================================
// FA-X2-03: Unknown-Y rows with blank X under parametric pre-summarized modes
// ============================================================================

#[test]
fn fa_x2_03_parametric_unknown_y_rows_interpolate_without_sd_n_or_with_zero_mean_cv() -> TestResult {
    for mode in [SubcolumnMode::MeanSdN, SubcolumnMode::MeanSemN, SubcolumnMode::MeanCvN] {
        // Calibration line: Y = 2*X + 10 for X in [0, 1, 2, 3].
        // Row 4: blank X, Y = [14.0, Empty, Empty] -> expected X = 2.0
        // Row 5: blank X, Y = [0.0, Empty, Empty]  -> expected X = -5.0 (previously broke MeanCvN on mean == 0.0!)
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 1, 6)?;
        for (r, x) in [0.0_f64, 1.0, 2.0, 3.0].into_iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x))?;
            let mean = 2.0 * x + 10.0;
            xy.data_sets[0].cells[r] = vec![Cell::number(mean), Cell::number(1.5), Cell::number(3.0)];
        }
        xy.data_sets[0].cells[4] = vec![Cell::number(14.0), Cell::empty(), Cell::empty()];
        xy.data_sets[0].cells[5] = vec![Cell::number(0.0), Cell::empty(), Cell::empty()];

        let sheet = DataSheet::new("xy-unk", "Parametric Unknowns", TableArchetype::Xy(xy.clone()))?;
        let lin_spec = LinearRegressionSpec { interpolate_unknowns: true, ..LinearRegressionSpec::default() };
        let spec = AnalysisSpec::new("fit-unk", "xy-unk", AnalysisKind::LinearRegression(lin_spec))?;
        let res = execute_fit_analysis(&spec, &sheet, &HookContext::default())?;
        assert_eq!(res.status, ResultStatus::UpToDate);
        let interp_tab = res.tabs.iter().find(|t| t.kind == ResultTabKind::InterpolatedUnknowns).ok_or("missing InterpolatedUnknowns tab")?;
        assert_eq!(interp_tab.rows.len(), 2, "{mode:?}: expected 2 interpolated unknown rows");

        let mut xs_out = Vec::new();
        for row in &interp_tab.rows {
            let ResultCell::Number { value, .. } = row[1] else {
                panic!("{mode:?}: expected numeric interpolated X, got {row:?}");
            };
            xs_out.push(value);
        }
        approx_eq(xs_out[0], 2.0, 1e-6, &format!("{mode:?} unknown Y=14 -> X"));
        approx_eq(xs_out[1], -5.0, 1e-6, &format!("{mode:?} unknown Y=0 -> X"));

        // Calibration rows (non-blank X) must STILL reject incomplete SD/N cells.
        let mut xy_bad_cal = xy;
        xy_bad_cal.set_x_cell(3, Cell::number(3.0))?;
        xy_bad_cal.data_sets[0].cells[3] = vec![Cell::number(16.0), Cell::empty(), Cell::empty()];
        let bad_sheet = DataSheet::new("xy-bad-cal", "Bad Calibration", TableArchetype::Xy(xy_bad_cal))?;
        assert!(execute_fit_analysis(&spec, &bad_sheet, &HookContext::default()).is_err(), "{mode:?} calibration row missing SD/N must fail");
    }
    Ok(())
}

// ============================================================================
// FA-X2-04: XyTable::x_summary_at unified across doc/transform/stats/fit
// ============================================================================

#[test]
fn fa_x2_04_x_summary_at_parity_across_doc_transform_stats_and_fit() -> TestResult {
    // 1. Unit contract of XyTable::x_summary_at across XColumnMode variants.
    let mut xy_reps =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) }, SubcolumnMode::Single, 1, 5)?;
    // Row 0: [1, 3*, 5] -> active mean = 3.0
    xy_reps.set_x_subcell(0, 0, Cell::number(1.0))?;
    xy_reps.set_x_subcell(0, 1, Cell::excluded_number(3.0))?;
    xy_reps.set_x_subcell(0, 2, Cell::number(5.0))?;
    // Row 1: [5, 7, 9] -> active mean = 7.0
    xy_reps.set_x_subcell(1, 0, Cell::number(5.0))?;
    xy_reps.set_x_subcell(1, 1, Cell::number(7.0))?;
    xy_reps.set_x_subcell(1, 2, Cell::number(9.0))?;
    // Row 2: [10, 12, 14] -> active mean = 12.0
    xy_reps.set_x_subcell(2, 0, Cell::number(10.0))?;
    xy_reps.set_x_subcell(2, 1, Cell::number(12.0))?;
    xy_reps.set_x_subcell(2, 2, Cell::number(14.0))?;
    // Row 3: all excluded -> None
    xy_reps.set_x_subcell(3, 0, Cell::excluded_number(2.0))?;
    // Row 4: [f64::MAX, f64::MAX] -> overflow-safe Some(f64::MAX)
    xy_reps.set_x_subcell(4, 0, Cell::number(f64::MAX))?;
    xy_reps.set_x_subcell(4, 1, Cell::number(f64::MAX))?;

    assert_eq!(xy_reps.x_summary_at(0)?, Some(3.0));
    assert_eq!(xy_reps.x_summary_at(1)?, Some(7.0));
    assert_eq!(xy_reps.x_summary_at(2)?, Some(12.0));
    assert_eq!(xy_reps.x_summary_at(3)?, None);
    assert_eq!(xy_reps.x_summary_at(4)?, Some(f64::MAX));
    assert_eq!(xy_reps.x_summary_at(99)?, None);

    // Parametric MeanCvN X rejects Mean==0, and validates spread>=0 & N>=1.
    let mut xy_cv = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanCvN }, SubcolumnMode::Single, 1, 2)?;
    xy_cv.set_x_subcell(0, 0, Cell::number(10.0))?;
    xy_cv.set_x_subcell(0, 1, Cell::number(5.0))?;
    xy_cv.set_x_subcell(0, 2, Cell::number(4.0))?;
    xy_cv.set_x_subcell(1, 0, Cell::number(0.0))?;
    xy_cv.set_x_subcell(1, 1, Cell::number(5.0))?;
    xy_cv.set_x_subcell(1, 2, Cell::number(4.0))?;
    xy_cv.data_sets[0].cells[0][0] = Cell::number(20.0);
    xy_cv.data_sets[0].cells[1][0] = Cell::number(30.0);
    assert_eq!(xy_cv.x_summary_at(0)?, Some(10.0));
    assert!(matches!(xy_cv.x_summary_at(1), Err(SummaryPointError::MissingSpread)));

    // 2. Compare RawReplicates(3) X ([3.0, 7.0, 12.0], plus [2*, 4, 6] => 5.0 if we use clean finite 4-row table)
    // across Fit (LinearRegression), Transform (AUC, Smooth/Integrate, Formula `= x * 2`), and Stats (Descriptive AUC & GLM).
    let mut xy_rep_clean =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(3) }, SubcolumnMode::Single, 1, 4)?;
    let mut xy_single = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 4)?;
    let rep_rows = [
        ([1.0, 3.0, 5.0], 6.0),     // mean X = 3
        ([4.0, 6.0, 8.0], 12.0),    // mean X = 6
        ([7.0, 9.0, 11.0], 18.0),   // mean X = 9
        ([10.0, 12.0, 14.0], 24.0), // mean X = 12 -> Y = 2*X exact
    ];
    for (r, (xs, y)) in rep_rows.into_iter().enumerate() {
        for (s, xv) in xs.into_iter().enumerate() {
            xy_rep_clean.set_x_subcell(r, s, Cell::number(xv))?;
        }
        xy_single.set_x_cell(r, Cell::number((r as f64 + 1.0) * 3.0))?;
        xy_rep_clean.data_sets[0].cells[r][0] = Cell::number(y);
        xy_single.data_sets[0].cells[r][0] = Cell::number(y);
    }
    let ds_rep = DataSheet::new("ds-xrep", "RepX", TableArchetype::Xy(xy_rep_clean))?;
    let ds_single = DataSheet::new("ds-xsingle", "SingleX", TableArchetype::Xy(xy_single))?;

    // Fit: linear regression slope must be 2.0 (if X[0] were used, intercept wouldn't be 0.0).
    let fit_rep = execute_fit_analysis(
        &AnalysisSpec::new("f-rep", "ds-xrep", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
        &ds_rep,
        &HookContext::default(),
    )?;
    let fit_sgl = execute_fit_analysis(
        &AnalysisSpec::new("f-sgl", "ds-xsingle", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
        &ds_single,
        &HookContext::default(),
    )?;
    approx_eq(fit_rep.lookup_scalar("Slope", Some(0)).unwrap(), 2.0, 1e-12, "fit RepX slope");
    approx_eq(fit_rep.lookup_scalar("YIntercept", Some(0)).unwrap(), 0.0, 1e-12, "fit RepX intercept");
    approx_eq(
        fit_rep.lookup_scalar("YIntercept", Some(0)).unwrap(),
        fit_sgl.lookup_scalar("YIntercept", Some(0)).unwrap(),
        1e-12,
        "fit RepX == SingleX",
    );

    // Transform: compute_sheet_auc + curve calculus Integrate + Formula `= x * 2`.
    let auc_rep = compute_sheet_auc(&ds_rep, 0, &AucOptions::default())?;
    let auc_sgl = compute_sheet_auc(&ds_single, 0, &AucOptions::default())?;
    approx_eq(auc_rep.net_area, auc_sgl.net_area, 1e-12, "transform AUC RepX == SingleX");
    let integ_rep = apply_curve_calculus_to_sheet(&ds_rep, CurveSheetOp::Integrate { baseline: 0.0 })?;
    let integ_sgl = apply_curve_calculus_to_sheet(&ds_single, CurveSheetOp::Integrate { baseline: 0.0 })?;
    assert_eq!(integ_rep.archetype, integ_sgl.archetype, "curve calculus Integrate must match SingleX");
    let deriv_rep = apply_curve_calculus_to_sheet(&ds_rep, CurveSheetOp::Differentiate(DerivativeOrder::First))?;
    approx_eq(deriv_rep.cell(CellCoord::new(0, 0, 0)).unwrap().active_number().unwrap(), 2.0, 1e-12, "derivative RepX");
    let formula_vals = evaluate_formula_for_sheet("= x * 2", &ds_rep, None)?;
    assert_eq!(formula_vals.iter().filter_map(|v| v.as_finite_f64()).collect::<Vec<_>>(), vec![6.0, 12.0, 18.0, 24.0]);

    // Stats: Descriptive trapezoidal AUC + GLM XY.
    let desc_rep =
        execute_stats_analysis(&ds_rep, &AnalysisSpec::new("d-rep", "ds-xrep", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    let desc_sgl =
        execute_stats_analysis(&ds_single, &AnalysisSpec::new("d-sgl", "ds-xsingle", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?)?;
    approx_eq(
        desc_rep.lookup_scalar("Trapezoidal AUC", Some(0)).unwrap(),
        desc_sgl.lookup_scalar("Trapezoidal AUC", Some(0)).unwrap(),
        1e-12,
        "stats AUC RepX == SingleX",
    );

    // Propagated SummaryPointError on invalid MeanCvN X across fit, transform, and stats.
    let ds_bad_cv = DataSheet::new("ds-bad-cv", "BadCvX", TableArchetype::Xy(xy_cv))?;
    assert!(
        execute_fit_analysis(
            &AnalysisSpec::new("f-bad", "ds-bad-cv", AnalysisKind::LinearRegression(LinearRegressionSpec::default()))?,
            &ds_bad_cv,
            &HookContext::default()
        )
        .is_err()
    );
    assert!(compute_sheet_auc(&ds_bad_cv, 0, &AucOptions::default()).is_err());
    assert!(evaluate_formula_for_sheet("= x * 2", &ds_bad_cv, None).is_err());
    assert!(
        execute_stats_analysis(&ds_bad_cv, &AnalysisSpec::new("d-bad", "ds-bad-cv", AnalysisKind::DescriptiveStats(DescriptiveSpec::default()))?,)
            .is_err()
    );
    Ok(())
}

// ============================================================================
// FA-X2-05: ColumnTable multi-row pre-summarized pooling & validation parity
// ============================================================================

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
                .fold(None, |acc, y| Some(acc.map_or(y, |a: f64| a.min(y)))),
            _ => None,
        })
        .fold(None, |acc, y| Some(acc.map_or(y, |a: f64| a.min(y))))
}

#[test]
fn fa_x2_05_column_presummarized_pooling_and_validation_parity_across_doc_stats_chart() -> TestResult {
    // Column A: two N=1 rows [(10, 0, 1), (14, 0, 1)] -> pooled N=2, mean=12.0, sd=sqrt(8), sem=2.0.
    // Raw equivalent for Column A: [10.0, 14.0].
    // Column B: [(20, 2, 3), (26, 4, 5)] -> pooled N=8.
    // Column C: [(30, 1, 2), (32, 3, 2)] -> pooled N=4.
    let mut col_pre = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 3, 2)?;
    col_pre.data_sets[0].title = String::from("G1");
    col_pre.data_sets[1].title = String::from("G2");
    col_pre.data_sets[2].title = String::from("G3");
    col_pre.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(0.0), Cell::number(1.0)], vec![Cell::number(14.0), Cell::number(0.0), Cell::number(1.0)]];
    col_pre.data_sets[1].cells =
        vec![vec![Cell::number(20.0), Cell::number(2.0), Cell::number(3.0)], vec![Cell::number(26.0), Cell::number(4.0), Cell::number(5.0)]];
    col_pre.data_sets[2].cells =
        vec![vec![Cell::number(30.0), Cell::number(1.0), Cell::number(2.0)], vec![Cell::number(32.0), Cell::number(3.0), Cell::number(2.0)]];
    let ds_pre = DataSheet::new("col-pre", "PrePool", TableArchetype::Column(col_pre.clone()))?;

    // Compute exact pooled stats via canonical stats::column::summarize_pre_summarized_points.
    let mut pts_g0 = Vec::new();
    let mut pts_g1 = Vec::new();
    let mut pts_g2 = Vec::new();
    for r in 0..2 {
        pts_g0.push(col_pre.summarized_point(r, 0)?.ok_or("missing g0")?);
        pts_g1.push(col_pre.summarized_point(r, 1)?.ok_or("missing g1")?);
        pts_g2.push(col_pre.summarized_point(r, 2)?.ok_or("missing g2")?);
    }
    let pool0 = summarize_pre_summarized_points("G1", &pts_g0, ConfidenceLevel::DEFAULT_95)?;
    let pool1 = summarize_pre_summarized_points("G2", &pts_g1, ConfidenceLevel::DEFAULT_95)?;
    let pool2 = summarize_pre_summarized_points("G3", &pts_g2, ConfidenceLevel::DEFAULT_95)?;
    approx_eq(pool0.total_n, 2.0, 1e-12, "G1 pooled N");
    approx_eq(pool0.mean, 12.0, 1e-12, "G1 pooled mean");
    approx_eq(pool0.sd.unwrap(), 8.0_f64.sqrt(), 1e-12, "G1 pooled SD");
    approx_eq(pool0.sem.unwrap(), 2.0, 1e-12, "G1 pooled SEM");

    // ANOVA (both Ordinary and Welch, which requires pooled total_n >= 2 per group) must use multi-row pooled stats!
    let ow_ord = execute_stats_analysis(&ds_pre, &AnalysisSpec::new("ow-ord", "col-pre", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?)?;
    let welch_spec = OneWayAnovaSpec { method: OneWayMethod::BrownForsytheAndWelch, ..OneWayAnovaSpec::default() };
    let ow_welch = execute_stats_analysis(&ds_pre, &AnalysisSpec::new("ow-welch", "col-pre", AnalysisKind::OneWayAnova(welch_spec))?)?;
    assert_eq!(ow_ord.status, ResultStatus::UpToDate);
    assert_eq!(ow_welch.status, ResultStatus::UpToDate);

    // Check MS residual against exact pooled within-group MS across all 3 columns (total N = 2 + 8 + 4 = 14 -> dfw = 11).
    let ss_w_expected = (pool0.total_n - 1.0) * pool0.sd.unwrap().powi(2)
        + (pool1.total_n - 1.0) * pool1.sd.unwrap().powi(2)
        + (pool2.total_n - 1.0) * pool2.sd.unwrap().powi(2);
    let ms_w_expected = ss_w_expected / (14.0 - 3.0);
    approx_eq(ow_ord.lookup_scalar("MS residual", None).ok_or("missing MS residual")?, ms_w_expected, 1e-9, "pooled MS residual matches total_n=14");
    assert!(ow_welch.lookup_scalar("Welch W", None).unwrap().is_finite());

    // Chart parity: single-column N=1 pooled pair [(10,0,1), (14,0,1)] must render identical geometry
    // (bar top height + 95% CI error bar) to raw [10.0, 14.0] under identical fixed axis range!
    let mut col_pre_1 = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanSdN, 1, 2)?;
    col_pre_1.data_sets[0].title = String::from("A");
    col_pre_1.data_sets[0].cells =
        vec![vec![Cell::number(10.0), Cell::number(0.0), Cell::number(1.0)], vec![Cell::number(14.0), Cell::number(0.0), Cell::number(1.0)]];
    let mut col_raw_1 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    col_raw_1.data_sets[0].title = String::from("A");
    col_raw_1.data_sets[0].cells = vec![vec![Cell::number(10.0)], vec![Cell::number(14.0)]];

    let ds_p1 = DataSheet::new("c-p1", "SameTitle", TableArchetype::Column(col_pre_1))?;
    let ds_r1 = DataSheet::new("c-r1", "SameTitle", TableArchetype::Column(col_raw_1))?;
    let mut doc_p = ProjectDocument::new("PreChart")?;
    doc_p.add_data_sheet(ds_p1.clone())?;
    let mut doc_r = ProjectDocument::new("RawChart")?;
    doc_r.add_data_sheet(ds_r1.clone())?;

    let configure_chart = |ds: &DataSheet| -> Result<GraphSheet, Box<dyn std::error::Error>> {
        let mut gs = GraphSheet::new_uninitialized_for_data("g", ds)?;
        gs.initialized = true;
        gs.spec.y_axis.segments[0].min = Some(-20.0);
        gs.spec.y_axis.segments[0].max = Some(50.0);
        gs.spec.series[0].gdo = GraphicalDataObject::BarWithReplicates {
            summary: SummaryMeasure::Mean,
            fill_color: gs.spec.palette.color_at(0),
            border_color: gs.spec.palette.color_at(0),
            border_width_pt: 0.75,
            replicate_overlay: None,
        };
        gs.spec.series[0].y_error_bars = Some(ErrorBarSpec {
            metric: ErrorBarMetric::ConfidenceInterval95,
            direction: ErrorBarDirection::Both,
            dash_style: curvebook_palette::LineDashStyle::Solid,
            stroke_width_pt: 1.0,
            cap_width_pt: 4.0,
            color: None,
        });
        Ok(gs)
    };
    let gs_p = configure_chart(&ds_p1)?;
    let gs_r = configure_chart(&ds_r1)?;
    let scene_p = compile_graph(&gs_p, &doc_p)?;
    let scene_r = compile_graph(&gs_r, &doc_r)?;
    assert!(scene_p.warnings.is_empty(), "pooled N=1+N=1 must emit 0 warnings: {:?}", scene_p.warnings);
    approx_eq(extract_vertical_bar_top_y(&scene_p).unwrap(), extract_vertical_bar_top_y(&scene_r).unwrap(), 1e-6, "pooled bar height matches raw");
    assert_eq!(scene_p.primitives, scene_r.primitives, "pooled [(10,0,1),(14,0,1)] bar+CI geometry must match raw [10,14]");

    // Validation agreement on invalid spread < 0 and MeanCvN mean == 0:
    // Rejected in doc::summarized_point, rejected by ANOVA, and warns `invalid_pre_summarized_point` + omits bar in chart!
    let mut col_bad = ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, SubcolumnMode::MeanCvN, 2, 1)?;
    col_bad.data_sets[0].cells[0] = vec![Cell::number(0.0), Cell::number(5.0), Cell::number(3.0)];
    col_bad.data_sets[1].cells[0] = vec![Cell::number(10.0), Cell::number(5.0), Cell::number(3.0)];
    assert!(matches!(col_bad.summarized_point(0, 0), Err(SummaryPointError::MissingSpread)));
    let ds_bad = DataSheet::new("c-bad", "BadCV", TableArchetype::Column(col_bad))?;
    assert!(execute_stats_analysis(&ds_bad, &AnalysisSpec::new("ow-bad", "c-bad", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))?,).is_err());
    let mut doc_bad = ProjectDocument::new("BadChart")?;
    doc_bad.add_data_sheet(ds_bad.clone())?;
    let mut gs_bad = GraphSheet::new_uninitialized_for_data("g-bad", &ds_bad)?;
    gs_bad.initialized = true;
    let scene_bad = compile_graph(&gs_bad, &doc_bad)?;
    assert!(scene_bad.warnings.iter().any(|w| w.code == "invalid_pre_summarized_point"), "expected invalid_pre_summarized_point warning");
    Ok(())
}

// ============================================================================
// FA-X2-06: PointStyleOverride::series_index contract + InsertRows shift & prune
// ============================================================================

#[test]
fn fa_x2_06_override_series_slot_contract_insert_rows_shift_and_prune() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-06 Overrides")?;
    let mut xy = XyTable::new(SubcolumnMode::Single, 3, 4)?;
    for r in 0..4 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64))?;
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 + r as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(20.0 + r as f64);
        xy.data_sets[2].cells[r][0] = Cell::number(30.0 + r as f64);
    }
    doc.add_data_sheet(DataSheet::new("xy-main", "Main", TableArchetype::Xy(xy))?)?;

    // Column sheet + graph + XY source/linked sheet for DAG shrink test
    let mut src_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4)?;
    let dst_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 4)?;
    for r in 0..4 {
        src_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
    }
    doc.add_data_sheet(DataSheet::new("col-src", "ColSrc", TableArchetype::Column(src_col))?)?;
    doc.add_data_sheet(DataSheet::new("col-dst", "ColDst", TableArchetype::Column(dst_col))?)?;

    let mut session = Session::from_document(doc);

    // 1. XY Graph with reordered/filtered series slots: slot 0 = ds 2, slot 1 = ds 0.
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-xy", "data_sheet_id": "xy-main", "kind": "xy" }));
    let base_series = session.document().graph_sheet(&SheetId::new("g-xy")).ok_or("missing g-xy")?.spec.series.clone();
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g-xy", "mode": "replace", "series": [base_series[2], base_series[0]] }));
    // Target slot 0 (which is data_set_index == 2), point 1.
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-xy", "points": [{ "series_index": 0, "point_index": 1 }], "hidden": true }),
    );
    // Slot 2 is out of bounds (series.len() == 2).
    assert!(
        session
            .execute("graph.formatSelectedPoints", json!({ "sheet_id": "g-xy", "points": [{ "series_index": 2, "point_index": 0 }], "hidden": true }))
            .is_err()
    );

    let scene_xy = compile_graph(session.document().graph_sheet(&SheetId::new("g-xy")).unwrap(), session.document())?;
    // ds 2 has 3 visible points (row 1 hidden), ds 0 keeps all 4 visible points (never polluted by series_index: 0!).
    assert!(!scene_xy.plotted_points.iter().any(|p| p.source_cell.col == 2 && p.source_cell.row == 1));
    assert_eq!(scene_xy.plotted_points.iter().filter(|p| p.source_cell.col == 0).count(), 4);

    // Reordering spec.series via graph.setSeries to [ds 0, ds 2] must remap override series_index 0 -> 1.
    ok_cmd(&mut session, "graph.setSeries", json!({ "sheet_id": "g-xy", "mode": "replace", "series": [base_series[0], base_series[2]] }));
    let g_after_reorder = session.document().graph_sheet(&SheetId::new("g-xy")).unwrap();
    assert_eq!(g_after_reorder.spec.point_overrides[0].series_index, 1);

    // 2. table.paste with InsertRows at row 1 inserting 2 rows shifts point_index 1 -> 3!
    // Also add an override on point 0 to verify point 0 stays 0.
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-xy", "points": [{ "series_index": 1, "point_index": 0 }], "color": "#ff0000" }),
    );
    ok_cmd(
        &mut session,
        "table.paste",
        json!({ "sheet_id": "xy-main", "mode": "insert_rows", "start_row": 1, "start_col": 0, "text": "99\t100\n98\t101\n" }),
    );
    let ovs_after_paste = &session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.point_overrides;
    assert!(ovs_after_paste.iter().any(|o| o.series_index == 1 && o.point_index == 0));
    assert!(
        ovs_after_paste.iter().any(|o| o.series_index == 1 && o.point_index == 3 && o.hidden),
        "point_index 1 must shift to 3: {ovs_after_paste:?}"
    );

    // 3. table.importCsv with InsertRows shifts point_index, and `replace: true` shorter import prunes shortened rows.
    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({
            "sheet_id": "xy-main",
            "mode": "insert_rows",
            "start_row": 2,
            "has_header": false,
            "csv": "50,51,52,53\n"
        }),
    );
    let ovs_after_csv_ins = &session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.point_overrides;
    assert!(
        ovs_after_csv_ins.iter().any(|o| o.series_index == 1 && o.point_index == 4 && o.hidden),
        "point_index 3 must shift to 4: {ovs_after_csv_ins:?}"
    );

    ok_cmd(
        &mut session,
        "table.importCsv",
        json!({
            "sheet_id": "xy-main",
            "replace": true,
            "archetype": "xy",
            "csv": "X,A,B,C\n1,10,20,30\n2,11,21,31\n"
        }),
    );
    let ovs_after_replace = &session.document().graph_sheet(&SheetId::new("g-xy")).unwrap().spec.point_overrides;
    assert!(!ovs_after_replace.iter().any(|o| o.point_index >= 2), "shortened replace import must prune point_index >= 2: {ovs_after_replace:?}");

    // 4. DAG refresh shortening a linked column sheet prunes shortened trailing point overrides.
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "col-dst", "source_sheet_id": "col-src" }));
    ok_cmd(&mut session, "graph.create", json!({ "sheet_id": "g-col", "data_sheet_id": "col-dst", "kind": "column" }));
    ok_cmd(
        &mut session,
        "graph.formatSelectedPoints",
        json!({ "sheet_id": "g-col", "points": [{ "series_index": 0, "point_index": 3 }], "hidden": true }),
    );
    assert_eq!(session.document().graph_sheet(&SheetId::new("g-col")).unwrap().spec.point_overrides.len(), 1);
    ok_cmd(&mut session, "table.setCell", json!({ "sheet_id": "col-src", "row": 3, "col": 0, "value": null }));
    assert!(
        session.document().graph_sheet(&SheetId::new("g-col")).unwrap().spec.point_overrides.is_empty(),
        "DAG refresh shortening linked sheet must prune trailing row 3 override"
    );
    Ok(())
}

// ============================================================================
// FA-X2-07: GeneratedSequence CSV export and .curvebook/.curvebook.d sidecars
// ============================================================================

#[test]
fn fa_x2_07_generated_sequence_csv_export_and_archive_sidecars_emit_x_and_exclusions() -> TestResult {
    let x_mode = XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Arithmetic };
    let mut xy = XyTable::new_with_x_mode(x_mode, SubcolumnMode::Single, 1, 3)?;
    xy.x_title = String::from("Time (min)");
    xy.data_sets[0].title = String::from("Signal");
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[0].cells[1][0] = Cell::number(20.0);
    xy.data_sets[0].cells[2][0] = Cell::number(30.0);
    let mut sheet = DataSheet::new("xy-gen", "Generated X", TableArchetype::Xy(xy))?;
    sheet.set_x_or_time_excluded(1, 0, true)?;

    // 1. Direct export_datasheet_to_csv emits X header + generated sequence values including `3*`.
    let csv_out = export_datasheet_to_csv(&sheet, &CsvOptions::csv())?;
    let records = parse_delimited_records(&csv_out, &CsvOptions::csv())?;
    assert_eq!(records[0], vec!["Time (min)", "Signal"]);
    assert_eq!(records[1], vec!["1", "10"]);
    assert_eq!(records[2], vec!["3*", "20"]);
    assert_eq!(records[3], vec!["5", "30"]);

    // 2. Engine table.exportCsv + .curvebook ZIP sidecar + .curvebook.d unpacked directory sidecar.
    let mut doc = ProjectDocument::new("FA-X2-07 GenSeq Export")?;
    doc.add_data_sheet(sheet)?;

    let bytes = save_project_bytes(&doc)?;
    let archive = ZipArchive::from_bytes(&bytes)?;
    let (_, zip_bytes) = archive.iter().find(|(n, _)| n.starts_with("sheets/") && n.ends_with(".csv")).ok_or("missing ZIP sheet csv")?;
    let zip_csv = std::str::from_utf8(zip_bytes)?;
    assert_eq!(zip_csv, csv_out, ".curvebook archive sheets/*.csv sidecar must match GeneratedSequence CSV");

    let guard = TempDir::try_new("genseq")?;
    let unpacked_dir = guard.path.join("project.curvebook.d");
    save_project_unpacked_dir(&doc, &unpacked_dir)?;
    let sidecar_entries: Vec<_> = fs::read_dir(unpacked_dir.join("sheets"))?.filter_map(Result::ok).collect();
    let dir_csv = fs::read_to_string(sidecar_entries[0].path())?;
    assert_eq!(dir_csv, csv_out, ".curvebook.d sheets/*.csv sidecar must match GeneratedSequence CSV");

    let mut session = Session::from_document(doc);
    let out = ok_cmd(&mut session, "table.exportCsv", json!({ "sheet_id": "xy-gen" }));
    assert_eq!(out.payload["csv"].as_str(), Some(csv_out.as_str()));
    Ok(())
}

// ============================================================================
// FA-X2-08: Native ResultCell::ConfidenceInterval formatting without stale backfill
// ============================================================================

#[test]
fn fa_x2_08_confidence_interval_cells_place_accurately_across_filter_decimate_unstack_and_transpose() -> TestResult {
    let mut doc = ProjectDocument::new("FA-X2-08 Native CI Placement")?;
    let dummy = DataSheet::new("dummy", "Dummy", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?))?;
    doc.add_data_sheet(dummy)?;

    let mut rs = ResultSheet::new(AnalysisSpec::new("res-ci", "dummy", AnalysisKind::DescriptiveStats(Default::default()))?)?;
    rs.status = ResultStatus::UpToDate;
    rs.tabs = vec![ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        row_labels: vec![String::from("R0"), String::from("R1"), String::from("R2"), String::from("R3"), String::from("R4"), String::from("R5")],
        column_headers: vec![String::from("Mean"), String::from("Group"), String::from("CI")],
        rows: vec![
            vec![
                ResultCell::Number { value: 10.0, prefix: None, units: None },
                ResultCell::Text(String::from("A")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(8.5), upper: CiBound::Finite(11.5) },
            ],
            vec![
                ResultCell::Number { value: 20.0, prefix: None, units: None },
                ResultCell::Text(String::from("B")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(17.0), upper: CiBound::Open },
            ],
            vec![ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
            vec![ResultCell::Empty, ResultCell::Empty, ResultCell::Empty],
            vec![
                ResultCell::Number { value: 30.0, prefix: None, units: None },
                ResultCell::Text(String::from("A")),
                ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(33.25) },
            ],
            vec![
                ResultCell::Number { value: 40.0, prefix: None, units: None },
                ResultCell::Text(String::from("B")),
                ResultCell::ConfidenceInterval { lower: CiBound::Finite(38.0), upper: CiBound::Finite(42.0) },
            ],
        ],
        footnotes: Vec::new(),
    }];
    doc.add_result_sheet(rs)?;

    let target_dec = DataSheet::new("dst-dec", "Decimated", TableArchetype::MultipleVariables(curvebook_doc::MultipleVariablesTable::new(3, 6)?))?;
    let target_tr = DataSheet::new("dst-tr", "Transposed", TableArchetype::MultipleVariables(curvebook_doc::MultipleVariablesTable::new(6, 3)?))?;
    let target_un = DataSheet::new("dst-un", "Unstacked", TableArchetype::MultipleVariables(curvebook_doc::MultipleVariablesTable::new(6, 4)?))?;
    doc.add_data_sheet(target_dec)?;
    doc.add_data_sheet(target_tr)?;
    doc.add_data_sheet(target_un)?;

    let mut session = Session::from_document(doc);

    // 1. skip_contiguous_blanks + decimate_every_k = 2:
    // Source rows [0, 1, 2(blank), 3(blank->collapsed), 4, 5] -> collapsed blanks -> [0, 1, 2(blank), 4, 5] (5 rows) -> decimate every 2 -> indices 0, 2, 4 -> placed at rows 0, 1, 2!
    let dec_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-ci"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(5, 2, 0)),
        },
        filter: ImportFilterSpec { decimate_every_k: Some(2), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec { skip_contiguous_blanks: true, ..ImportPlacementSpec::default() },
    };
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "dst-dec", "link_spec": dec_spec }));
    let dec_ds = session.document().data_sheet(&SheetId::new("dst-dec")).ok_or("missing dst-dec")?;
    assert_eq!(dec_ds.cell(CellCoord::new(0, 2, 0)).unwrap().value, CellValue::Text(String::from("[8.5, 11.5]")));
    assert_eq!(dec_ds.cell(CellCoord::new(1, 2, 0)).unwrap().value, CellValue::Empty, "collapsed blank row 1 col 2 must stay empty");
    assert_eq!(dec_ds.cell(CellCoord::new(2, 2, 0)).unwrap().value, CellValue::Text(String::from("[-inf, 33.25]")));
    assert_eq!(dec_ds.cell(CellCoord::new(4, 2, 0)).unwrap().value, CellValue::Empty, "stale backfill must NOT write to raw row 4");
    assert_eq!(dec_ds.cell(CellCoord::new(5, 2, 0)).unwrap().value, CellValue::Empty, "stale backfill must NOT write to raw row 5");

    // 2. Transpose: col 2 (`CI`) becomes row 2 across placed columns 0..=1!
    let tr_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-ci"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 2, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { transpose: true, ..ImportPlacementSpec::default() },
    };
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "dst-tr", "link_spec": tr_spec }));
    let tr_ds = session.document().data_sheet(&SheetId::new("dst-tr")).ok_or("missing dst-tr")?;
    assert_eq!(tr_ds.cell(CellCoord::new(2, 0, 0)).unwrap().value, CellValue::Text(String::from("[8.5, 11.5]")));
    assert_eq!(tr_ds.cell(CellCoord::new(2, 1, 0)).unwrap().value, CellValue::Text(String::from("[17, +inf]")));

    // 3. Unstack 2-column window (cols 1..=2 -> Group, CI): row 4 (`[-inf, 33.25]` for A) stacks cleanly under Group A!
    let un_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-ci"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(5, 2, 0)),
        },
        filter: ImportFilterSpec { col_window: Some((1, 2)), unstack_group_column: Some(0), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec::default(),
    };
    ok_cmd(&mut session, "table.pasteLink", json!({ "target_sheet_id": "dst-un", "link_spec": un_spec }));
    let un_ds = session.document().data_sheet(&SheetId::new("dst-un")).ok_or("missing dst-un")?;
    let all_texts: Vec<String> = (0..un_ds.archetype.row_count())
        .flat_map(|r| {
            (0..un_ds.archetype.column_count()).filter_map(move |c| match un_ds.cell(CellCoord::new(r, c, 0)).map(|cell| &cell.value) {
                Some(CellValue::Text(s)) => Some(s.clone()),
                _ => None,
            })
        })
        .collect();
    assert!(all_texts.contains(&String::from("[8.5, 11.5]")));
    assert!(all_texts.contains(&String::from("[-inf, 33.25]")));
    assert!(all_texts.contains(&String::from("[17, +inf]")));
    assert!(all_texts.contains(&String::from("[38, 42]")));

    // 4. `sheet.extractResultTab` formats CIs natively even after upstream refresh without backfill pass.
    ok_cmd(&mut session, "sheet.extractResultTab", json!({ "result_sheet_id": "res-ci", "tab_id": "summary", "sheet_id": "ext-ci" }));
    let ext_ds = session.document().data_sheet(&SheetId::new("ext-ci")).ok_or("missing ext-ci")?;
    assert_eq!(ext_ds.cell(CellCoord::new(0, 2, 0)).unwrap().value, CellValue::Text(String::from("[8.5, 11.5]")));
    assert_eq!(ext_ds.cell(CellCoord::new(4, 2, 0)).unwrap().value, CellValue::Text(String::from("[-inf, 33.25]")));
    Ok(())
}
