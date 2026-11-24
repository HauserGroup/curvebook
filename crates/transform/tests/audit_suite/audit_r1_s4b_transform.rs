use std::error::Error;

use curvebook_doc::{
    BaselineOperation, Cell, CellValue, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, SubcolumnMode, TableArchetype, TransformAnalysisSpec,
    XColumnMode, XyTable,
};
use curvebook_transform::{CurveSheetOp, DerivativeOrder, SmoothMethod, TransformContext, apply_curve_calculus_to_sheet, execute_transform};

fn build_xy(id: &str, xs: &[f64], ys: &[&[Option<f64>]]) -> Result<DataSheet, Box<dyn Error>> {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, ys.len(), xs.len())?;
    for (r, &x) in xs.iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x))?;
    }
    for (c, col) in ys.iter().enumerate() {
        for (r, &opt_y) in col.iter().enumerate() {
            xy.data_sets[c].cells[r][0] = opt_y.map_or_else(Cell::empty, Cell::number);
        }
    }
    Ok(DataSheet::new(id, format!("Sheet {id}"), TableArchetype::Xy(xy))?)
}

fn as_xy(sheet: &DataSheet) -> Result<&XyTable, Box<dyn Error>> {
    match &sheet.archetype {
        TableArchetype::Xy(xy) => Ok(xy),
        other => Err(format!("expected XY archetype, got {other:?}").into()),
    }
}

#[test]
fn r1_s04_10_curve_calculus_aligns_sparse_and_unsorted_datasets() -> Result<(), Box<dyn Error>> {
    // Dataset A has points only at x = 0, 2, 4; Dataset B has points at x = 0, 1, 2, 3, 4.
    let sheet = build_xy(
        "sparse",
        &[0.0, 1.0, 2.0, 3.0, 4.0],
        &[&[Some(0.0), None, Some(4.0), None, Some(8.0)], &[Some(10.0), Some(13.0), Some(16.0), Some(19.0), Some(22.0)]],
    )?;

    let diff_sheet = apply_curve_calculus_to_sheet(&sheet, CurveSheetOp::Differentiate(DerivativeOrder::First))?;
    let diff_xy = as_xy(&diff_sheet)?;
    assert_eq!(diff_xy.row_count(), 5);
    for (r, expected_x) in [0.0, 1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        assert_eq!(diff_xy.x_value_at(r), Some(*expected_x), "missing or misaligned X at row {r}");
    }
    // Dataset A: d/dx (2x) = 2 at x = 0, 2, 4; rows x = 1, 3 must stay Empty.
    assert_eq!(diff_xy.data_sets[0].cells[0][0].active_number(), Some(2.0));
    assert_eq!(diff_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_eq!(diff_xy.data_sets[0].cells[2][0].active_number(), Some(2.0));
    assert_eq!(diff_xy.data_sets[0].cells[3][0].value, CellValue::Empty);
    assert_eq!(diff_xy.data_sets[0].cells[4][0].active_number(), Some(2.0));
    for r in 0..5 {
        assert_eq!(diff_xy.data_sets[1].cells[r][0].active_number(), Some(3.0), "dataset B derivative at row {r}");
    }

    // Unsorted X rows with ragged presence + Smooth & Integrate.
    let unsorted =
        build_xy("unsorted", &[4.0, 1.0, 3.0, 0.0], &[&[Some(4.0), None, Some(3.0), Some(0.0)], &[Some(8.0), Some(2.0), None, Some(0.0)]])?;
    let integ_sheet = apply_curve_calculus_to_sheet(&unsorted, CurveSheetOp::Integrate { baseline: 0.0 })?;
    let integ_xy = as_xy(&integ_sheet)?;
    assert_eq!(integ_xy.row_count(), 4);
    for (r, expected_x) in [0.0, 1.0, 3.0, 4.0].iter().enumerate() {
        assert_eq!(integ_xy.x_value_at(r), Some(*expected_x));
    }
    assert_eq!(integ_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_eq!(integ_xy.data_sets[1].cells[1][0].active_number(), Some(1.0));
    assert_eq!(integ_xy.data_sets[0].cells[2][0].active_number(), Some(4.5));
    assert_eq!(integ_xy.data_sets[1].cells[2][0].value, CellValue::Empty);

    let smooth_sheet = apply_curve_calculus_to_sheet(&unsorted, CurveSheetOp::Smooth(SmoothMethod::MovingAverage { window: 3 }))?;
    let smooth_xy = as_xy(&smooth_sheet)?;
    assert_eq!(smooth_xy.row_count(), 4);
    assert_eq!(smooth_xy.data_sets[0].cells[1][0].value, CellValue::Empty);
    assert_eq!(smooth_xy.data_sets[1].cells[2][0].value, CellValue::Empty);
    assert!(smooth_xy.data_sets[0].cells[2][0].active_number().is_some());
    Ok(())
}

#[test]
fn r1_s04_11_remove_baseline_warns_on_single_and_excluded_cells() -> Result<(), Box<dyn Error>> {
    let ctx = TransformContext::default();
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    col_tbl.data_sets[0].cells[0][0] = Cell::number(2.0);
    col_tbl.data_sets[0].cells[1][0] = Cell::empty();
    col_tbl.data_sets[0].cells[2][0] = Cell::number(0.0);
    col_tbl.data_sets[0].cells[3][0] = Cell { value: CellValue::Number(5.0), excluded: true };

    col_tbl.data_sets[1].cells[0][0] = Cell::number(10.0);
    col_tbl.data_sets[1].cells[1][0] = Cell::number(20.0);
    col_tbl.data_sets[1].cells[2][0] = Cell::number(30.0);
    col_tbl.data_sets[1].cells[3][0] = Cell { value: CellValue::Number(40.0), excluded: true };

    let col_sheet = DataSheet::new("col_rb", "RB", TableArchetype::Column(col_tbl))?;
    let out = execute_transform(
        &col_sheet,
        &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Ratio },
        &ctx,
    )?;
    assert_eq!(out.warnings.len(), 3, "expected warnings on row 2 col 0, row 1 col 1, row 2 col 1 only: {:?}", out.warnings);
    assert!(!out.warnings.iter().any(|w| w.contains("row 3")), "excluded row 3 must not warn: {:?}", out.warnings);
    if let TableArchetype::Column(res) = &out.data_sheet.archetype {
        assert_eq!(res.data_sets[1].cells[0][0].active_number(), Some(5.0));
        assert_eq!(res.data_sets[1].cells[1][0].value, CellValue::Empty);
        assert_eq!(res.data_sets[1].cells[2][0].value, CellValue::Empty);
    }
    Ok(())
}

#[test]
fn r1_s04_11_remove_baseline_warns_across_presummarized_subcolumn_modes() -> Result<(), Box<dyn Error>> {
    let ctx = TransformContext::default();

    // MeanSdN missing baseline blanks Mean & SD AND warns.
    let mut grp_sd = GroupedTable::new(SubcolumnMode::MeanSdN, 2, 1)?;
    grp_sd.data_sets[0].cells[0] = vec![Cell::empty(), Cell::empty(), Cell::number(3.0)];
    grp_sd.data_sets[1].cells[0] = vec![Cell::number(15.0), Cell::number(2.5), Cell::number(3.0)];
    let grp_sd_sheet = DataSheet::new("grp_sd", "SD", TableArchetype::Grouped(grp_sd))?;
    let out_sd = execute_transform(
        &grp_sd_sheet,
        &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference },
        &ctx,
    )?;
    assert!(!out_sd.warnings.is_empty(), "MeanSdN missing baseline should warn");
    if let TableArchetype::Grouped(res) = &out_sd.data_sheet.archetype {
        assert_eq!(res.data_sets[1].cells[0][0].value, CellValue::Empty);
        assert_eq!(res.data_sets[1].cells[0][1].value, CellValue::Empty);
        assert_eq!(res.data_sets[1].cells[0][2].active_number(), Some(3.0));
    }

    // MeanCvN missing baseline AND zero m_out in Difference must blank & warn.
    let mut grp_cv = GroupedTable::new(SubcolumnMode::MeanCvN, 2, 2)?;
    grp_cv.data_sets[0].cells[0] = vec![Cell::empty(), Cell::empty(), Cell::number(3.0)];
    grp_cv.data_sets[1].cells[0] = vec![Cell::number(12.0), Cell::number(5.0), Cell::number(3.0)];
    grp_cv.data_sets[0].cells[1] = vec![Cell::number(10.0), Cell::number(2.0), Cell::number(3.0)];
    grp_cv.data_sets[1].cells[1] = vec![Cell::number(10.0), Cell::number(4.0), Cell::number(3.0)];
    let grp_cv_sheet = DataSheet::new("grp_cv", "CV", TableArchetype::Grouped(grp_cv))?;
    let out_cv = execute_transform(
        &grp_cv_sheet,
        &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::Difference },
        &ctx,
    )?;
    assert!(out_cv.warnings.len() >= 3, "expected warnings for MeanCvN row 0 blanking + row 1 undefined %CV: {:?}", out_cv.warnings);
    if let TableArchetype::Grouped(res) = &out_cv.data_sheet.archetype {
        assert_eq!(res.data_sets[1].cells[0][0].value, CellValue::Empty);
        assert_eq!(res.data_sets[1].cells[0][1].value, CellValue::Empty);
        assert_eq!(res.data_sets[1].cells[1][0].active_number(), Some(0.0));
        assert_eq!(res.data_sets[1].cells[1][1].value, CellValue::Empty);
    }

    // MeanLowHigh missing baseline warns and blanks Mean/Low/High.
    let mut grp_lh = GroupedTable::new(SubcolumnMode::MeanLowHigh, 2, 1)?;
    grp_lh.data_sets[0].cells[0] = vec![Cell::empty(), Cell::empty(), Cell::empty()];
    grp_lh.data_sets[1].cells[0] = vec![Cell::number(8.0), Cell::number(1.0), Cell::number(2.0)];
    let grp_lh_sheet = DataSheet::new("grp_lh", "LH", TableArchetype::Grouped(grp_lh))?;
    let out_lh = execute_transform(
        &grp_lh_sheet,
        &TransformAnalysisSpec::RemoveBaseline { baseline_columns: vec![0], operation: BaselineOperation::PercentChange },
        &ctx,
    )?;
    assert_eq!(out_lh.warnings.len(), 3, "Mean, Low, High blanked should emit 3 warnings: {:?}", out_lh.warnings);
    Ok(())
}
