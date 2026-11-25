//! Regression tests for ledger items RV-59, RV-60, RV-63, and RV-64 (`crates/transform/src/ops/**`).

use curvebook_doc::{
    BaselineOperation, Cell, CellValue, ErrorPropagationMode, MathTransformPreset, NormalizeAnchor, SequenceProgression, SubcolumnMode,
    SurvivalStatus, TableArchetype, TableKind, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_transform::{
    BaselineKind, MathTransformOp, PruneMethod, TransformContext, TransformError, execute_math_transform, execute_normalize, execute_prune,
    execute_remove_baseline_kind, execute_select, execute_transform,
};

use crate::common::{SubExp, as_survival, as_xy, assert_approx, assert_cells_3, excl, num, sheet, survival_sheet, xy_3sub_clean, xy_single};

#[test]
fn rv59_duplicate_selected_data_sets_deduplicate_preserving_order() {
    let mut s = xy_single("xy-dup-sel", &[1.0, 2.0], &[&[100.0, 1000.0], &[10.0, 20.0], &[50.0, 100.0]]);
    if let TableArchetype::Xy(xy) = &mut s.archetype {
        for (i, t) in ["ColA", "ColB", "ColC"].into_iter().enumerate() {
            xy.data_sets[i].title = String::from(t);
        }
    }

    // 1. Math Log10 with [0, 0] applies log10 once (100 -> 2, 1000 -> 3), leaving col 1 untouched.
    let dup0 = TransformContext { selected_data_sets: vec![0, 0], ..TransformContext::default() };
    let math_out = execute_math_transform(
        &s,
        None,
        Some(&MathTransformOp::Preset(MathTransformPreset::Log10)),
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &dup0,
    )
    .expect("math log10");
    let math_xy = as_xy(&math_out.data_sheet);
    assert_approx(math_xy.data_sets[0].cells[0][0].active_number().unwrap(), 2.0, 1e-12);
    assert_approx(math_xy.data_sets[0].cells[1][0].active_number().unwrap(), 3.0, 1e-12);
    assert_approx(math_xy.data_sets[1].cells[0][0].active_number().unwrap(), 10.0, 1e-12);

    // 2. Baseline subtraction with [0, 0] subtracts constant 25 once (100 -> 75, not 50).
    let base_out = execute_remove_baseline_kind(&s, &BaselineKind::Constant(25.0), BaselineOperation::Difference, &dup0).expect("baseline");
    let base_xy = as_xy(&base_out.data_sheet);
    assert_approx(base_xy.data_sets[0].cells[0][0].active_number().unwrap(), 75.0, 1e-12);
    assert_approx(base_xy.data_sets[0].cells[1][0].active_number().unwrap(), 975.0, 1e-12);

    // 3. Normalize per-dataset with [2, 0, 2] normalizes each unique selected column once (4 anchor scalars).
    let order_ctx = TransformContext { selected_data_sets: vec![2, 0, 2], ..TransformContext::default() };
    let norm_out = execute_normalize(&s, &NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::FixedValue(50.0), true, true, &order_ctx).unwrap();
    assert_eq!(norm_out.scalars.len(), 4);
    let norm_xy = as_xy(&norm_out.data_sheet);
    assert_approx(norm_xy.data_sets[2].cells[0][0].active_number().unwrap(), 100.0, 1e-12);
    assert_approx(norm_xy.data_sets[0].cells[0][0].active_number().unwrap(), 200.0, 1e-12);
    assert_approx(norm_xy.data_sets[1].cells[0][0].active_number().unwrap(), 10.0, 1e-12);

    // 4. SelectAndTransform with [2, 0, 2, 0] preserves first-occurrence order [ColC, ColA] and rejects OOB.
    let sel_ctx = TransformContext { selected_data_sets: vec![2, 0, 2, 0], ..TransformContext::default() };
    let sel_out = execute_select(&s, &[], TableKind::Xy, &sel_ctx).unwrap();
    assert_eq!(as_xy(&sel_out.data_sheet).data_sets.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), vec!["ColC", "ColA"]);

    let oob_ctx = TransformContext { selected_data_sets: vec![0, 3], ..TransformContext::default() };
    assert!(matches!(execute_select(&s, &[], TableKind::Xy, &oob_ctx), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn rv60_baseline_negative_divisor_reorders_upper_lower_and_iqr_bounds() {
    let ctx = TransformContext::default();
    let tol = (1e-12, 0.0);

    // 1. UpperLowerLimits [Mean=10, Lower=8, Upper=16] with negative baseline -2.0 sorts bounds cleanly.
    let ul_sheet = xy_3sub_clean("xy-ul", SubcolumnMode::UpperLowerLimits, &[1.0], &[("A", &[[Some(10.0), Some(8.0), Some(16.0)]])]);
    for (op, exp) in [
        (BaselineOperation::Ratio, [-5.0, -8.0, -4.0]),
        (BaselineOperation::PercentChange, [-600.0, -900.0, -500.0]),
        (BaselineOperation::FractionalChange, [-6.0, -9.0, -5.0]),
    ] {
        let out = execute_remove_baseline_kind(&ul_sheet, &BaselineKind::Constant(-2.0), op, &ctx).unwrap();
        let row = &as_xy(&out.data_sheet).data_sets[0].cells[0];
        assert_cells_3(row, exp.map(SubExp::Num), tol, &format!("UL {op:?}"));
        let bounds = SubcolumnMode::UpperLowerLimits.explicit_bounds(row).expect("ordered explicit bounds");
        assert!(bounds.lower <= bounds.center && bounds.center <= bounds.upper);
    }

    // 2. MedianIqr [Median=12, Q1=6, Q3=18] divided by -3.0 -> [-4, -6, -2].
    let iqr_sheet = xy_3sub_clean("xy-iqr", SubcolumnMode::MedianIqr, &[1.0], &[("A", &[[Some(12.0), Some(6.0), Some(18.0)]])]);
    let iqr_out = execute_remove_baseline_kind(&iqr_sheet, &BaselineKind::Constant(-3.0), BaselineOperation::Ratio, &ctx).unwrap();
    assert_cells_3(&as_xy(&iqr_out.data_sheet).data_sets[0].cells[0], [-4.0, -6.0, -2.0].map(SubExp::Num), tol, "MedianIqr ratio");
}

#[test]
fn rv63_materialize_x_preserves_excluded_value_and_mask_across_math_and_prune() {
    let ctx = TransformContext::default();

    // 1. DateOrElapsedTime with excluded X cells preserves transformed values & masks across Math and Prune.
    let mut xy_time =
        XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("min") }, SubcolumnMode::Single, 1, 4).unwrap();
    xy_time.x_cells = vec![vec![num(10.0)], vec![excl(100.0)], vec![excl(200.0)], vec![num(1000.0)]];
    xy_time.data_sets[0].cells = vec![vec![num(50.0)], vec![excl(500.0)], vec![num(600.0)], vec![num(5000.0)]];
    let time_sheet = sheet("xy-time", TableArchetype::Xy(xy_time));

    let math_out = execute_math_transform(
        &time_sheet,
        Some(&MathTransformOp::Preset(MathTransformPreset::Log10)),
        Some(&MathTransformOp::UserExpression(String::from("Y / X"))),
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &ctx,
    )
    .unwrap();
    let math_xy = as_xy(&math_out.data_sheet);
    assert_eq!((&math_xy.x_cells[0][0], &math_xy.x_cells[1][0], &math_xy.x_cells[3][0]), (&num(1.0), &excl(2.0), &num(3.0)));
    assert_eq!(math_xy.data_sets[0].cells[1][0], excl(5.0));

    let bin1_xy = as_xy(&execute_prune(&time_sheet, PruneMethod::AverageBins(1), &ctx).unwrap().data_sheet).clone();
    assert_eq!((&bin1_xy.x_cells[0][0], &bin1_xy.x_cells[1][0], &bin1_xy.x_cells[2][0]), (&num(10.0), &excl(100.0), &excl(200.0)));

    let mut xy_all_excl =
        XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("s") }, SubcolumnMode::Single, 1, 2).unwrap();
    xy_all_excl.x_cells = vec![vec![excl(100.0)], vec![excl(200.0)]];
    xy_all_excl.data_sets[0].cells = vec![vec![num(10.0)], vec![num(30.0)]];
    let bin2_out = execute_prune(&sheet("xy-excl", TableArchetype::Xy(xy_all_excl)), PruneMethod::AverageBins(2), &ctx).unwrap();
    let bin2_xy = as_xy(&bin2_out.data_sheet);
    assert_eq!((&bin2_xy.x_cells[0][0], &bin2_xy.data_sets[0].cells[0][0]), (&excl(150.0), &num(20.0)));

    // 2. GeneratedSequence with an excluded X slot preserves both sequence value and exclusion mask.
    let mut xy_seq = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 10.0, step: 10.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        1,
        3,
    )
    .unwrap();
    xy_seq.x_cells = vec![
        vec![Cell { value: CellValue::Empty, excluded: false }],
        vec![Cell { value: CellValue::Empty, excluded: true }],
        vec![Cell { value: CellValue::Empty, excluded: false }],
    ];
    xy_seq.data_sets[0].cells = vec![vec![num(1.0)], vec![num(2.0)], vec![num(3.0)]];
    let seq_math = execute_math_transform(
        &sheet("xy-seq", TableArchetype::Xy(xy_seq)),
        Some(&MathTransformOp::UserExpression(String::from("X * 2"))),
        None,
        ErrorPropagationMode::FirstOrderDeltaMethod,
        &ctx,
    )
    .unwrap();
    let seq_xy = as_xy(&seq_math.data_sheet);
    assert_eq!((&seq_xy.x_cells[0][0], &seq_xy.x_cells[1][0], &seq_xy.x_cells[2][0]), (&num(20.0), &excl(40.0), &num(60.0)));
}

#[test]
fn rv64_average_bins_rejects_survival_table_while_row_decimation_succeeds() {
    let ctx = TransformContext::default();
    let s = survival_sheet("srv-1", "", "", &[(2.0, 1.0), (4.0, 0.0), (6.0, 1.0), (8.0, 0.0)]);

    assert!(matches!(execute_prune(&s, PruneMethod::AverageBins(2), &ctx), Err(TransformError::IncompatibleTable { kind: TableKind::Survival, .. })));
    assert!(matches!(
        execute_transform(&s, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: true }, &ctx),
        Err(TransformError::IncompatibleTable { kind: TableKind::Survival, .. })
    ));

    let decimated = execute_transform(&s, &TransformAnalysisSpec::PruneRows { every_k_rows: 2, average_window: false }, &ctx).unwrap();
    let obs = as_survival(&decimated.data_sheet).cohort_observations(0).unwrap();
    assert_eq!(obs.iter().map(|o| o.status).collect::<Vec<_>>(), vec![SurvivalStatus::Event, SurvivalStatus::Event]);
}
