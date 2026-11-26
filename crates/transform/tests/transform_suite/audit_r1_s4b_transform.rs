use curvebook_doc::{BaselineOperation, Cell, CellValue, SubcolumnMode, TransformAnalysisSpec};
use curvebook_transform::{CurveSheetOp, DerivativeOrder, SmoothMethod, TransformContext, apply_curve_calculus_to_sheet, execute_transform};

use crate::common::{SubExp, as_col, as_grouped, as_xy, assert_cells_3, col_sheet, empty, excl, grouped_sheet, num, xy_opt};

#[test]
fn r1_s04_10_curve_calculus_aligns_sparse_and_unsorted_datasets() {
    // Dataset A has points only at x = 0, 2, 4; Dataset B has points at x = 0..=4.
    let sparse = xy_opt(
        "sparse",
        &[0.0, 1.0, 2.0, 3.0, 4.0],
        &[("A", &[Some(0.0), None, Some(4.0), None, Some(8.0)]), ("B", &[Some(10.0), Some(13.0), Some(16.0), Some(19.0), Some(22.0)])],
    );
    let diff_sheet = apply_curve_calculus_to_sheet(&sparse, CurveSheetOp::Differentiate(DerivativeOrder::First)).unwrap();
    let diff_xy = as_xy(&diff_sheet);
    assert_eq!((0..5).map(|r| diff_xy.x_value_at(r)).collect::<Vec<_>>(), vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0), Some(4.0)]);
    assert_eq!(
        (0..5).map(|r| diff_xy.data_sets[0].cells[r][0].active_number()).collect::<Vec<_>>(),
        vec![Some(2.0), None, Some(2.0), None, Some(2.0)]
    );
    assert!((0..5).all(|r| diff_xy.data_sets[1].cells[r][0].active_number() == Some(3.0)));

    // Unsorted X rows with ragged presence + Integrate & Smooth.
    let unsorted = xy_opt(
        "unsorted",
        &[4.0, 1.0, 3.0, 0.0],
        &[("A", &[Some(4.0), None, Some(3.0), Some(0.0)]), ("B", &[Some(8.0), Some(2.0), None, Some(0.0)])],
    );
    let integ_sheet = apply_curve_calculus_to_sheet(&unsorted, CurveSheetOp::Integrate { baseline: 0.0 }).unwrap();
    let integ_xy = as_xy(&integ_sheet);
    assert_eq!((0..4).map(|r| integ_xy.x_value_at(r)).collect::<Vec<_>>(), vec![Some(0.0), Some(1.0), Some(3.0), Some(4.0)]);
    assert_eq!(integ_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_eq!(integ_xy.data_sets[1].cells[1][0].active_number(), Some(1.0));
    assert_eq!(integ_xy.data_sets[0].cells[2][0].active_number(), Some(4.5));
    assert_eq!(integ_xy.data_sets[1].cells[2][0].value, CellValue::Empty);

    let smooth_sheet = apply_curve_calculus_to_sheet(&unsorted, CurveSheetOp::Smooth(SmoothMethod::MovingAverage { window: 3 })).unwrap();
    let smooth_xy = as_xy(&smooth_sheet);
    assert_eq!(smooth_xy.row_count(), 4);
    assert_eq!(smooth_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_eq!(smooth_xy.data_sets[1].cells[2][0].value, CellValue::Empty);
    assert!(smooth_xy.data_sets[0].cells[2][0].active_number().is_some());
}

#[test]
fn r1_s04_11_remove_baseline_warns_on_single_and_excluded_cells() {
    let ctx = TransformContext::default();
    let col = col_sheet("col_rb", &[("B", &[num(2.0), empty(), num(0.0), excl(5.0)]), ("T", &[num(10.0), num(20.0), num(30.0), excl(40.0)])]);
    let out =
        execute_transform(&col, &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Ratio }, &ctx)
            .unwrap();
    assert_eq!(out.warnings.len(), 3, "expected warnings on row 2 col 0, row 1 col 1, row 2 col 1 only: {:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w.contains("row 3")), "excluded row 3 must not warn: {:?}", out.warnings);
    let res = as_col(&out.data_sheet);
    assert_eq!(res.data_sets[1].cells[0][0].active_number(), Some(5.0));
    assert_eq!(res.data_sets[1].cells[1][0].value, CellValue::Empty);
    assert_eq!(res.data_sets[1].cells[2][0].value, CellValue::Empty);
}

#[test]
fn r1_s04_11_remove_baseline_warns_across_presummarized_subcolumn_modes() {
    struct Case<'a> {
        id: &'a str,
        mode: SubcolumnMode,
        op: BaselineOperation,
        base_rows: &'a [&'a [Cell]],
        trt_rows: &'a [&'a [Cell]],
        min_warnings: usize,
        exact_warnings: bool,
        expected_trt: &'a [[SubExp; 3]],
    }
    let e = empty();
    let cases = [
        Case {
            id: "grp_sd",
            mode: SubcolumnMode::MeanSdN,
            op: BaselineOperation::Difference,
            base_rows: &[&[e.clone(), e.clone(), num(3.0)]],
            trt_rows: &[&[num(15.0), num(2.5), num(3.0)]],
            min_warnings: 1,
            exact_warnings: false,
            expected_trt: &[[SubExp::Empty, SubExp::Empty, SubExp::Num(3.0)]],
        },
        Case {
            id: "grp_cv",
            mode: SubcolumnMode::MeanCvN,
            op: BaselineOperation::Difference,
            base_rows: &[&[e.clone(), e.clone(), num(3.0)], &[num(10.0), num(2.0), num(3.0)]],
            trt_rows: &[&[num(12.0), num(5.0), num(3.0)], &[num(10.0), num(4.0), num(3.0)]],
            min_warnings: 3,
            exact_warnings: false,
            expected_trt: &[[SubExp::Empty, SubExp::Empty, SubExp::Any], [SubExp::Num(0.0), SubExp::Empty, SubExp::Any]],
        },
        Case {
            id: "grp_lh",
            mode: SubcolumnMode::MeanLowHigh,
            op: BaselineOperation::PercentChange,
            base_rows: &[&[e.clone(), e.clone(), e]],
            trt_rows: &[&[num(8.0), num(1.0), num(2.0)]],
            min_warnings: 3,
            exact_warnings: true,
            expected_trt: &[[SubExp::Empty, SubExp::Empty, SubExp::Empty]],
        },
    ];

    let ctx = TransformContext::default();
    for c in cases {
        let s = grouped_sheet(c.id, c.mode, &[], &[("Base", c.base_rows), ("Trt", c.trt_rows)]);
        let out = execute_transform(&s, &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: c.op }, &ctx).unwrap();
        if c.exact_warnings {
            assert_eq!(out.warnings.len(), c.min_warnings, "{}: {:?}", c.id, out.warnings);
        } else {
            assert!(out.warnings.len() >= c.min_warnings, "{}: {:?}", c.id, out.warnings);
        }
        let res = as_grouped(&out.data_sheet);
        for (r, &exp) in c.expected_trt.iter().enumerate() {
            assert_cells_3(&res.data_sets[1].cells[r], exp, (1e-12, 0.0), &format!("{} r{r}", c.id));
        }
    }
}
