//! Regression tests for 2-subcolumn `SubcolumnMode::MeanSd` and `SubcolumnMode::MeanSem`
//! across math, normalize, remove-baseline, transpose, and archetype conversion
//! (schema v9 §3.5).

use curvebook_doc::{
    BaselineOperation, Cell, ColumnDesignHint, ColumnTable, DataSheet, ErrorPropagationMode, GroupedTable, MathTransformPreset, NormalizeAnchor,
    SubcolumnMode, TableArchetype, XyTable,
};
use curvebook_transform::{
    MathTransformOp, TargetArchetypeKind, TransformContext, convert_datasheet_archetype, execute_math_transform, execute_normalize,
    execute_remove_baseline, execute_transpose,
};

#[test]
fn v9_mean_sd_and_mean_sem_transforms_and_archetype_conversions_preserve_two_subcolumns() {
    let ctx = TransformContext::default();

    for mode in [SubcolumnMode::MeanSd, SubcolumnMode::MeanSem] {
        // 1. Math transform (delta method & linear scale) transforms BOTH Mean and SD/SEM subcolumns.
        let mut xy = XyTable::new(mode, 1, 2).unwrap();
        xy.x_cells = vec![vec![Cell::number(1.0)], vec![Cell::number(2.0)]];
        xy.data_sets[0].cells = vec![vec![Cell::number(10.0), Cell::number(2.0)], vec![Cell::number(100.0), Cell::number(10.0)]];
        let xy_sheet = DataSheet::new("xy-m", "XY", TableArchetype::Xy(xy)).unwrap();

        let linear_op = MathTransformOp::Preset(MathTransformPreset::LinearScale { slope: 2.0, intercept: 5.0 });
        let linear_out = execute_math_transform(&xy_sheet, None, Some(&linear_op), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
        let TableArchetype::Xy(lin_xy) = &linear_out.data_sheet.archetype else {
            panic!("expected XY output");
        };
        assert_eq!(lin_xy.y_subcolumn_mode, mode);
        assert_eq!(lin_xy.data_sets[0].cells[0][0].active_number(), Some(25.0));
        assert_eq!(lin_xy.data_sets[0].cells[0][1].active_number(), Some(4.0));

        // 2. Normalize scales Mean and SD/SEM consistently.
        let norm_out =
            execute_normalize(&xy_sheet, &NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::FixedValue(100.0), true, true, &ctx).unwrap();
        let TableArchetype::Xy(norm_xy) = &norm_out.data_sheet.archetype else {
            panic!("expected XY output");
        };
        assert_eq!(norm_xy.data_sets[0].cells[0][0].active_number(), Some(10.0));
        assert_eq!(norm_xy.data_sets[0].cells[0][1].active_number(), Some(2.0));

        // 3. RemoveBaseline (Difference) on 2-column Grouped MeanSd/MeanSem preserves 2 subcolumns.
        let mut grp = GroupedTable::new(mode, 2, 2).unwrap();
        grp.row_titles = vec![String::from("R1"), String::from("R2")];
        grp.data_sets[0].title = String::from("Base");
        grp.data_sets[1].title = String::from("Drug");
        grp.data_sets[0].cells = vec![vec![Cell::number(10.0), Cell::number(1.0)], vec![Cell::number(20.0), Cell::number(2.0)]];
        grp.data_sets[1].cells = vec![vec![Cell::number(15.0), Cell::number(3.0)], vec![Cell::number(30.0), Cell::number(4.0)]];
        let grp_sheet = DataSheet::new("grp-m", "Grouped", TableArchetype::Grouped(grp)).unwrap();

        let base_out = execute_remove_baseline(&grp_sheet, &[0], BaselineOperation::Difference, &ctx).unwrap();
        let TableArchetype::Grouped(base_grp) = &base_out.data_sheet.archetype else {
            panic!("expected Grouped output");
        };
        assert_eq!(base_grp.subcolumn_mode, mode);
        assert_eq!(base_grp.data_sets[1].cells[0][0].active_number(), Some(5.0));

        // 4. Transpose swaps rows & datasets while keeping 2 subcolumns intact.
        let tr_out = execute_transpose(&grp_sheet, true, &ctx).unwrap();
        let TableArchetype::Grouped(tr_grp) = &tr_out.data_sheet.archetype else {
            panic!("expected transposed Grouped output");
        };
        assert_eq!(tr_grp.subcolumn_mode, mode);
        assert_eq!(tr_grp.data_sets[0].cells[1][0].active_number(), Some(15.0));
        assert_eq!(tr_grp.data_sets[0].cells[1][1].active_number(), Some(3.0));

        // 5. Grouped <-> Column conversion preserves MeanSd / MeanSem cleanly.
        let (conv_col, _) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Column, false).unwrap();
        let TableArchetype::Column(col_t) = &conv_col.archetype else {
            panic!("expected Column output");
        };
        assert_eq!(col_t.subcolumn_mode, mode);
        assert_eq!(col_t.design_hint, ColumnDesignHint::PrecomputedError);

        let col_orig = DataSheet::new(
            "col-orig",
            "Col",
            TableArchetype::Column(ColumnTable::new_with_subcolumns(ColumnDesignHint::PrecomputedError, mode, 2, 2).unwrap()),
        )
        .unwrap();
        let (conv_grp, _) = convert_datasheet_archetype(&col_orig, TargetArchetypeKind::Grouped, false).unwrap();
        let TableArchetype::Grouped(g_back) = &conv_grp.archetype else {
            panic!("expected Grouped output");
        };
        assert_eq!(g_back.subcolumn_mode, mode);
    }
}
