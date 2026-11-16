//! Regression tests for ledger items RV-59, RV-60, RV-63, and RV-64 (`crates/transform/src/ops/**`).

use curvebook_doc::{
    BaselineOperation, Cell, CellValue, DataSheet, ErrorPropagationMode, MathTransformPreset, NormalizeAnchor, SequenceProgression, SubcolumnMode,
    SurvivalStatus, SurvivalTable, TableArchetype, TableKind, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_transform::{
    BaselineKind, MathTransformOp, PruneMethod, TransformContext, TransformError, execute_math_transform, execute_normalize, execute_prune,
    execute_remove_baseline_kind, execute_select, execute_transform,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} +/- {tol}, got {actual}");
}

#[test]
fn rv59_duplicate_selected_data_sets_deduplicate_preserving_order() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 3, 2).expect("valid xy");
    xy.set_x_cell(0, Cell::number(1.0)).expect("x0");
    xy.set_x_cell(1, Cell::number(2.0)).expect("x1");
    xy.data_sets[0].title = String::from("ColA");
    xy.data_sets[1].title = String::from("ColB");
    xy.data_sets[2].title = String::from("ColC");
    xy.data_sets[0].cells[0][0] = Cell::number(100.0);
    xy.data_sets[0].cells[1][0] = Cell::number(1000.0);
    xy.data_sets[1].cells[0][0] = Cell::number(10.0);
    xy.data_sets[1].cells[1][0] = Cell::number(20.0);
    xy.data_sets[2].cells[0][0] = Cell::number(50.0);
    xy.data_sets[2].cells[1][0] = Cell::number(100.0);
    let sheet = DataSheet::new("xy-dup-sel", "Dup Sel", TableArchetype::Xy(xy)).expect("sheet");

    // 1. Math Log10 with [0, 0] applies log10 exactly once (100 -> 2, 1000 -> 3), never log10(log10(x)).
    let dup_col0_ctx = TransformContext { selected_data_sets: vec![0, 0], ..TransformContext::default() };
    let math_out = execute_math_transform(
        &sheet,
        None,
        Some(&MathTransformOp::Preset(MathTransformPreset::Log10)),
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &dup_col0_ctx,
    )
    .expect("math log10");
    let TableArchetype::Xy(math_xy) = &math_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx(math_xy.data_sets[0].cells[0][0].active_number().expect("r0"), 2.0, 1e-12);
    assert_approx(math_xy.data_sets[0].cells[1][0].active_number().expect("r1"), 3.0, 1e-12);
    assert_approx(math_xy.data_sets[1].cells[0][0].active_number().expect("untouched col1"), 10.0, 1e-12);

    // 2. Baseline subtraction with [0, 0] subtracts constant 25 once (100 -> 75, not 50).
    let base_out =
        execute_remove_baseline_kind(&sheet, &BaselineKind::Constant(25.0), BaselineOperation::Difference, &dup_col0_ctx).expect("baseline");
    let TableArchetype::Xy(base_xy) = &base_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx(base_xy.data_sets[0].cells[0][0].active_number().expect("r0"), 75.0, 1e-12);
    assert_approx(base_xy.data_sets[0].cells[1][0].active_number().expect("r1"), 975.0, 1e-12);

    // 3. Normalize per-dataset with [2, 0, 2] normalizes each selected column once and emits 4 scalar anchors (2 per unique col).
    let order_ctx = TransformContext { selected_data_sets: vec![2, 0, 2], ..TransformContext::default() };
    let norm_out =
        execute_normalize(&sheet, &NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::FixedValue(50.0), true, true, &order_ctx).expect("normalize");
    assert_eq!(norm_out.scalars.len(), 4);
    let TableArchetype::Xy(norm_xy) = &norm_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx(norm_xy.data_sets[2].cells[0][0].active_number().expect("col2 r0"), 100.0, 1e-12);
    assert_approx(norm_xy.data_sets[0].cells[0][0].active_number().expect("col0 r0"), 200.0, 1e-12);
    assert_approx(norm_xy.data_sets[1].cells[0][0].active_number().expect("col1 untouched"), 10.0, 1e-12);

    // 4. SelectAndTransform with [2, 0, 2, 0] preserves first-occurrence order [ColC, ColA] without duplicates.
    let sel_ctx = TransformContext { selected_data_sets: vec![2, 0, 2, 0], ..TransformContext::default() };
    let sel_out = execute_select(&sheet, &[], TableKind::Xy, &sel_ctx).expect("select");
    let TableArchetype::Xy(sel_xy) = &sel_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    let titles: Vec<&str> = sel_xy.data_sets.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles, vec!["ColC", "ColA"]);

    // Out-of-bounds indices are still rejected cleanly.
    let oob_ctx = TransformContext { selected_data_sets: vec![0, 3], ..TransformContext::default() };
    assert!(matches!(execute_select(&sheet, &[], TableKind::Xy, &oob_ctx), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn rv60_baseline_negative_divisor_reorders_upper_lower_and_iqr_bounds() {
    let ctx = TransformContext::default();

    // 1. UpperLowerLimits divided by a negative baseline (-2.0) across Ratio, PercentChange, and FractionalChange.
    let mut xy_ul = XyTable::new(SubcolumnMode::UpperLowerLimits, 1, 1).expect("ul xy");
    xy_ul.set_x_cell(0, Cell::number(1.0)).expect("x0");
    // [Mean = 10, Lower = 8, Upper = 16]
    xy_ul.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(8.0), Cell::number(16.0)];
    let ul_sheet = DataSheet::new("xy-ul", "UL", TableArchetype::Xy(xy_ul)).expect("sheet");

    // Ratio: v / (-2) maps [10, 8, 16] -> [-5, -4, -8], which must sort bounds to [-8, -4].
    let ratio_out = execute_remove_baseline_kind(&ul_sheet, &BaselineKind::Constant(-2.0), BaselineOperation::Ratio, &ctx).expect("ratio");
    let TableArchetype::Xy(ratio_xy) = &ratio_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    let ratio_row = &ratio_xy.data_sets[0].cells[0];
    assert_approx(ratio_row[0].active_number().expect("mean"), -5.0, 1e-12);
    assert_approx(ratio_row[1].active_number().expect("lower"), -8.0, 1e-12);
    assert_approx(ratio_row[2].active_number().expect("upper"), -4.0, 1e-12);
    let bounds = SubcolumnMode::UpperLowerLimits.explicit_bounds(ratio_row).expect("ordered explicit bounds");
    assert!(bounds.lower <= bounds.center && bounds.center <= bounds.upper);

    // PercentChange: 100 * (v - (-2)) / (-2) maps [10, 8, 16] -> [-600, -500, -900] -> sorted [-900, -500].
    let pct_out = execute_remove_baseline_kind(&ul_sheet, &BaselineKind::Constant(-2.0), BaselineOperation::PercentChange, &ctx).expect("pct");
    let TableArchetype::Xy(pct_xy) = &pct_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    let pct_row = &pct_xy.data_sets[0].cells[0];
    assert_approx(pct_row[0].active_number().expect("mean"), -600.0, 1e-12);
    assert_approx(pct_row[1].active_number().expect("lower"), -900.0, 1e-12);
    assert_approx(pct_row[2].active_number().expect("upper"), -500.0, 1e-12);

    // FractionalChange: (v - (-2)) / (-2) maps [10, 8, 16] -> [-6, -5, -9] -> sorted [-9, -5].
    let frac_out = execute_remove_baseline_kind(&ul_sheet, &BaselineKind::Constant(-2.0), BaselineOperation::FractionalChange, &ctx).expect("frac");
    let TableArchetype::Xy(frac_xy) = &frac_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    let frac_row = &frac_xy.data_sets[0].cells[0];
    assert_approx(frac_row[0].active_number().expect("mean"), -6.0, 1e-12);
    assert_approx(frac_row[1].active_number().expect("lower"), -9.0, 1e-12);
    assert_approx(frac_row[2].active_number().expect("upper"), -5.0, 1e-12);

    // 2. MedianIqr: [Median = 12, Q1 = 6, Q3 = 18] divided by -3.0 -> [-4, -6, -2] with Q1 <= Q3.
    let mut xy_iqr = XyTable::new(SubcolumnMode::MedianIqr, 1, 1).expect("iqr xy");
    xy_iqr.set_x_cell(0, Cell::number(1.0)).expect("x0");
    xy_iqr.data_sets[0].cells[0] = vec![Cell::number(12.0), Cell::number(6.0), Cell::number(18.0)];
    let iqr_sheet = DataSheet::new("xy-iqr", "IQR", TableArchetype::Xy(xy_iqr)).expect("sheet");
    let iqr_out = execute_remove_baseline_kind(&iqr_sheet, &BaselineKind::Constant(-3.0), BaselineOperation::Ratio, &ctx).expect("iqr ratio");
    let TableArchetype::Xy(iqr_xy) = &iqr_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    let iqr_row = &iqr_xy.data_sets[0].cells[0];
    assert_approx(iqr_row[0].active_number().expect("median"), -4.0, 1e-12);
    assert_approx(iqr_row[1].active_number().expect("q1"), -6.0, 1e-12);
    assert_approx(iqr_row[2].active_number().expect("q3"), -2.0, 1e-12);
}

#[test]
fn rv63_materialize_x_preserves_excluded_value_and_mask_across_math_and_prune() {
    let ctx = TransformContext::default();

    // 1. DateOrElapsedTime with an excluded X cell (100.0*) at row 1.
    let mut xy_time = XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("min") }, SubcolumnMode::Single, 1, 4)
        .expect("elapsed xy");
    xy_time.set_x_cell(0, Cell::number(10.0)).expect("x0");
    xy_time.set_x_cell(1, Cell::excluded_number(100.0)).expect("x1 excluded");
    xy_time.set_x_cell(2, Cell::excluded_number(200.0)).expect("x2 excluded");
    xy_time.set_x_cell(3, Cell::number(1000.0)).expect("x3");
    xy_time.data_sets[0].cells[0][0] = Cell::number(50.0);
    xy_time.data_sets[0].cells[1][0] = Cell::excluded_number(500.0);
    xy_time.data_sets[0].cells[2][0] = Cell::number(600.0);
    xy_time.data_sets[0].cells[3][0] = Cell::number(5000.0);
    let time_sheet = DataSheet::new("xy-time", "Elapsed", TableArchetype::Xy(xy_time)).expect("sheet");

    // Math transform on X (Log10) and Y (Y / X) preserves both transformed numbers and excluded flags.
    let math_out = execute_math_transform(
        &time_sheet,
        Some(&MathTransformOp::Preset(MathTransformPreset::Log10)),
        Some(&MathTransformOp::UserExpression(String::from("Y / X"))),
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &ctx,
    )
    .expect("math transform");
    let TableArchetype::Xy(math_xy) = &math_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(math_xy.x_cells[0][0], Cell::number(1.0));
    assert_eq!(math_xy.x_cells[1][0], Cell::excluded_number(2.0));
    assert_eq!(math_xy.x_cells[3][0], Cell::number(3.0));
    assert_eq!(math_xy.data_sets[0].cells[1][0], Cell::excluded_number(5.0));

    // Prune AverageBins(1) preserves excluded X cells intact (not blanked).
    let bin1_out = execute_prune(&time_sheet, PruneMethod::AverageBins(1), &ctx).expect("bin1");
    let TableArchetype::Xy(bin1_xy) = &bin1_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(bin1_xy.x_cells[0][0], Cell::number(10.0));
    assert_eq!(bin1_xy.x_cells[1][0], Cell::excluded_number(100.0));
    assert_eq!(bin1_xy.x_cells[2][0], Cell::excluded_number(200.0));

    // Prune AverageBins(2):
    // - Bin 0 has [10.0, 100.0*] -> active X = 10.0 (excluded value does not contaminate active average).
    // - Middle test with all-excluded bin [100.0*, 200.0*]: build 2-row all-excluded slice to verify 150.0*.
    let mut xy_all_excl = XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("s") }, SubcolumnMode::Single, 1, 2)
        .expect("2-row elapsed");
    xy_all_excl.set_x_cell(0, Cell::excluded_number(100.0)).expect("x0");
    xy_all_excl.set_x_cell(1, Cell::excluded_number(200.0)).expect("x1");
    xy_all_excl.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy_all_excl.data_sets[0].cells[1][0] = Cell::number(30.0);
    let excl_sheet = DataSheet::new("xy-excl", "AllExclBin", TableArchetype::Xy(xy_all_excl)).expect("sheet");
    let bin2_out = execute_prune(&excl_sheet, PruneMethod::AverageBins(2), &ctx).expect("bin2");
    let TableArchetype::Xy(bin2_xy) = &bin2_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(bin2_xy.x_cells[0][0], Cell::excluded_number(150.0));
    assert_eq!(bin2_xy.data_sets[0].cells[0][0], Cell::number(20.0));

    // 2. GeneratedSequence with an excluded X cell slot preserves both sequence value and exclusion mask.
    let mut xy_seq = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 10.0, step: 10.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        3,
    )
    .expect("seq xy");
    xy_seq.x_cells = vec![
        vec![Cell { value: CellValue::Empty, excluded: false }],
        vec![Cell { value: CellValue::Empty, excluded: true }],
        vec![Cell { value: CellValue::Empty, excluded: false }],
    ];
    xy_seq.data_sets[0].cells[0][0] = Cell::number(1.0);
    xy_seq.data_sets[0].cells[1][0] = Cell::number(2.0);
    xy_seq.data_sets[0].cells[2][0] = Cell::number(3.0);
    let seq_sheet = DataSheet::new("xy-seq", "Seq", TableArchetype::Xy(xy_seq)).expect("sheet");
    let seq_math = execute_math_transform(
        &seq_sheet,
        Some(&MathTransformOp::UserExpression(String::from("X * 2"))),
        None,
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &ctx,
    )
    .expect("seq math");
    let TableArchetype::Xy(seq_xy) = &seq_math.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(seq_xy.x_cells[0][0], Cell::number(20.0));
    assert_eq!(seq_xy.x_cells[1][0], Cell::excluded_number(40.0));
    assert_eq!(seq_xy.x_cells[2][0], Cell::number(60.0));
}

#[test]
fn rv64_average_bins_rejects_survival_table_while_row_decimation_succeeds() {
    let ctx = TransformContext::default();
    let mut srv = SurvivalTable::new(1, 4).expect("survival");
    for (r, (t, code)) in [(2.0, 1.0), (4.0, 0.0), (6.0, 1.0), (8.0, 0.0)].into_iter().enumerate() {
        srv.times[r] = Cell::number(t);
        srv.cohorts[0].cells[r][0] = Cell::number(code);
    }
    let sheet = DataSheet::new("srv-1", "Survival", TableArchetype::Survival(srv)).expect("sheet");

    // Bin-averaging a Survival table must refuse cleanly rather than fabricating fractional 0.5 status codes.
    let err_direct = execute_prune(&sheet, PruneMethod::AverageBins(2), &ctx).expect_err("must reject survival bin-averaging");
    assert!(matches!(err_direct, TransformError::IncompatibleTable { kind: TableKind::Survival, .. }));

    let err_spec = execute_transform(&sheet, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }, &ctx)
        .expect_err("TransformAnalysisSpec::PruneRows with average_window=true must reject Survival");
    assert!(matches!(err_spec, TransformError::IncompatibleTable { kind: TableKind::Survival, .. }));

    // Row-decimation (average_window = false / EveryNth) preserves valid integer event/censoring observations.
    let decimated = execute_transform(&sheet, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false }, &ctx)
        .expect("EveryNth decimation on Survival");
    let TableArchetype::Survival(dec_srv) = &decimated.data_sheet.archetype else {
        panic!("expected Survival");
    };
    let obs = dec_srv.cohort_observations(0).expect("valid survival observations");
    assert_eq!(obs.len(), 2);
    assert_eq!(obs[0].status, SurvivalStatus::Event);
    assert_eq!(obs[1].status, SurvivalStatus::Event);
}
