//! Wave 2 audit regression suite for `crates/transform` (#54, #55, #56, #59, #60, #61, #77, #94).

use curvebook_doc::{
    BaselineOperation, CellCoord, ErrorPropagationMode, HookSource, ImportFilterSpec, MathTransformPreset, NormalizeAnchor, PlacementMode,
    SubcolumnMode, TableArchetype, TableKind, VariableRole, XColumnMode, XyTable,
};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    BaselineKind, MathTransformOp, PruneMethod, TargetArchetypeKind, TidyColumnMapping, TransformContext, TransformError,
    convert_datasheet_archetype, execute_math_op, execute_normalize, execute_prune, execute_remove_baseline_kind, execute_select,
    pivot_tidy_long_to_datasheet,
};

use crate::common::{
    add_excl, as_col, as_col_mut, as_contingency, as_grouped, as_xy, assert_lossy_convert, clipboard_spec, col_nums, col_sheet, empty, excl,
    grouped_sheet, import_csv, mv_sheet, mv_sheet_inferred, mv_table, num, sheet, str_rows, str_vec, survival_sheet, txt, xy_3sub_clean, xy_single,
};

#[test]
fn audit_54_negative_slope_swaps_bounds_exclusions_notes_and_partial_limits() {
    let ctx = TransformContext::default();

    // 1. Negative derivative LinearScale(-2, 10) on MeanLowHigh swaps values, `.excluded`, and `CellExclusionNote` subcol 1 <-> 2.
    let mut mlh = xy_3sub_clean("neg-mlh", SubcolumnMode::MeanLowHigh, &[1.0], &[("A", &[[Some(5.0), Some(1.5), Some(3.0)]])]);
    add_excl(&mut mlh, CellCoord::new(0, 0, 1), "qc", "lower asymmetric tail excluded");
    let op = MathTransformOp::Preset(MathTransformPreset::LinearScale { slope: -2.0, intercept: 10.0 });
    let out = execute_math_op(&mlh, &op, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let row = &as_xy(&out.data_sheet).data_sets[0].cells[0];
    assert_approx_eq(row[0].active_number().unwrap(), 0.0, 1e-12, 0.0, "center");
    assert!(!row[1].excluded);
    assert_approx_eq(row[1].active_number().unwrap(), 6.0, 1e-12, 0.0, "swapped -Err");
    assert!(row[2].excluded && row[2].value.finite_f64() == Some(3.0));
    assert_eq!(out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(out.data_sheet.exclusion_notes[0].coord, CellCoord::new(0, 0, 2));

    // 2. Partial-missing + asymmetric exclusion in UpperLowerLimits under negative baseline Ratio & empty-center Reciprocal.
    let mut lim =
        xy_3sub_clean("neg-ul", SubcolumnMode::UpperLowerLimits, &[1.0, 2.0], &[("A", &[[Some(10.0), Some(8.0), None], [None, Some(2.0), None]])]);
    add_excl(&mut lim, CellCoord::new(0, 0, 1), "qc", "partial lower excluded");

    let base_out = execute_remove_baseline_kind(&lim, &BaselineKind::Constant(-2.0), BaselineOperation::Ratio, &ctx).unwrap();
    let base_xy = as_xy(&base_out.data_sheet);
    assert_approx_eq(base_xy.data_sets[0].cells[0][0].active_number().unwrap(), -5.0, 1e-12, 0.0, "center");
    assert!(base_xy.data_sets[0].cells[0][1].is_empty() && !base_xy.data_sets[0].cells[0][1].excluded);
    assert!(base_xy.data_sets[0].cells[0][2].excluded && base_xy.data_sets[0].cells[0][2].value.finite_f64() == Some(-4.0));
    assert_eq!(base_out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(base_out.data_sheet.exclusion_notes[0].coord, CellCoord::new(0, 0, 2));
    assert!(base_xy.data_sets[0].cells[1][1].is_empty());
    assert_approx_eq(base_xy.data_sets[0].cells[1][2].active_number().unwrap(), -1.0, 1e-12, 0.0, "partial swapped");

    let recip_out = execute_math_op(&lim, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let rec_row1 = &as_xy(&recip_out.data_sheet).data_sets[0].cells[1];
    assert!(rec_row1[1].is_empty());
    assert_approx_eq(rec_row1[2].active_number().unwrap(), 0.5, 1e-12, 0.0, "empty-center partial reciprocal");
}

#[test]
fn audit_55_domain_blanked_spread_emits_explicit_warnings() {
    // MeanCvN with mean_out == 0 emits explicit spread-blanked warning.
    // (UpperLowerLimits Ln OOD lower bound warning is covered in `d1_normalization::math_transform_decreasing_monotone_endpoints_preserve_lower_upper_ordering`.)
    let cv_sheet = xy_3sub_clean("cv-zero", SubcolumnMode::MeanCvN, &[1.0], &[("A", &[[Some(1.0), Some(10.0), Some(5.0)]])]);
    let cv_out = execute_math_op(&cv_sheet, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &TransformContext::default()).unwrap();
    assert_eq!(cv_out.warnings.len(), 1);
    assert!(cv_out.warnings[0].contains("CV% spread blanked"));
}

#[test]
fn audit_56_grouped_xy_mv_roundtrips_preserve_replicate_slots_and_warn_presummarized() {
    // 1. Grouped(RawReplicates(3)) preserves leading/middle empty replicate slots via MV round-trip.
    let grp_sheet = grouped_sheet(
        "grp-slots",
        SubcolumnMode::RawReplicates(3),
        &["R0", "R1"],
        &[("Drug", &[&[empty(), num(42.0), empty()], &[num(11.0), empty(), num(33.0)]])],
    );
    let (mv_g, _) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    let (grp_back, _) = convert_datasheet_archetype(&mv_g, TargetArchetypeKind::Grouped, false).unwrap();
    let gb = as_grouped(&grp_back);
    assert_eq!(gb.subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert_eq!(
        gb.data_sets[0].cells.iter().map(|r| r.iter().map(|c| c.active_number()).collect::<Vec<_>>()).collect::<Vec<_>>(),
        vec![vec![None, Some(42.0), None], vec![Some(11.0), None, Some(33.0)]]
    );

    // 2. Xy(RawReplicates(3)) preserves leading empty replicate slots via MV round-trip.
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(3), 1, 2).unwrap();
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Signal");
    xy.set_x_cell(0, num(1.0)).unwrap();
    xy.set_x_cell(1, num(2.0)).unwrap();
    xy.data_sets[0].cells = vec![vec![empty(), num(15.0), empty()], vec![num(5.0), empty(), num(25.0)]];
    let (mv_x, _) = convert_datasheet_archetype(&sheet("xy-slots", TableArchetype::Xy(xy)), TargetArchetypeKind::MultipleVariables, false).unwrap();
    let (xy_back, _) = convert_datasheet_archetype(&mv_x, TargetArchetypeKind::Xy, false).unwrap();
    let xb = as_xy(&xy_back);
    assert_eq!(xb.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert!(xb.data_sets[0].cells[0][0].is_empty());
    assert_eq!((xb.data_sets[0].cells[0][1].active_number(), xb.data_sets[0].cells[1][2].active_number()), (Some(15.0), Some(25.0)));

    // 3. MV one-factor -> Grouped keeps same source row paired in same replicate slot across columns.
    let mv1_sheet = mv_sheet_inferred(
        "mv1",
        &[("Media", &[txt("DMEM"), txt("DMEM")]), ("MeasureA", &[num(10.0), num(20.0)]), ("MeasureB", &[empty(), num(200.0)])],
    );
    let (grp1_sheet, _) = convert_datasheet_archetype(&mv1_sheet, TargetArchetypeKind::Grouped, false).unwrap();
    let g1 = as_grouped(&grp1_sheet);
    assert_eq!(g1.subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert!(g1.data_sets[1].cells[0][0].is_empty());
    assert_eq!(g1.data_sets[1].cells[0][1].active_number(), Some(200.0));
}

#[test]
fn audit_56_presummarized_grouped_warns_and_sparse_explicit_replicate_slots() {
    // 4. Grouped(MeanSdN) -> MultipleVariables flags schema_lossy + emits warning.
    let grp_sum = grouped_sheet("grp-sum", SubcolumnMode::MeanSdN, &[], &[("A", &[&[num(10.0), num(1.0), num(4.0)]])]);
    let (_, conv_prev) = assert_lossy_convert(&grp_sum, TargetArchetypeKind::MultipleVariables);
    assert!(!conv_prev.warnings.is_empty());

    // 5. Direct MV -> XY and MV two-way -> Grouped with sparse explicit Replicate = 2, 4 preserves slots 1 and 3.
    let mut mv_sparse = mv_table(&[
        ("Time", &[num(1.0), num(1.0)]),
        ("Group", &[txt("Ctrl"), txt("Ctrl")]),
        ("Replicate", &[num(2.0), num(4.0)]),
        ("Value", &[num(12.0), num(14.0)]),
    ]);
    for v in &mut mv_sparse.variables {
        v.refresh_inferred_role();
    }
    let expected_slots = vec![None, Some(12.0), None, Some(14.0)];

    let (xy_sp, _) =
        convert_datasheet_archetype(&sheet("mv-sparse-rep", TableArchetype::MultipleVariables(mv_sparse.clone())), TargetArchetypeKind::Xy, false)
            .unwrap();
    let xsp = as_xy(&xy_sp);
    assert_eq!(xsp.y_subcolumn_mode, SubcolumnMode::RawReplicates(4));
    assert_eq!(xsp.data_sets[0].cells[0].iter().map(|c| c.active_number()).collect::<Vec<_>>(), expected_slots);

    mv_sparse.variables[0].lock_role(VariableRole::Categorical);
    let (grp_sp, _) =
        convert_datasheet_archetype(&sheet("mv-grp-sparse-rep", TableArchetype::MultipleVariables(mv_sparse)), TargetArchetypeKind::Grouped, false)
            .unwrap();
    let gsp = as_grouped(&grp_sp);
    assert_eq!(gsp.subcolumn_mode, SubcolumnMode::RawReplicates(4));
    assert_eq!(gsp.data_sets[0].cells[0].iter().map(|c| c.active_number()).collect::<Vec<_>>(), expected_slots);
}

#[test]
fn audit_59_tidy_grouped_preserves_blank_numeric_x_records_in_missing_row() {
    let headers = str_vec(&["Dose", "Group", "Response"]);
    let rows = str_rows(&[&["1.0", "A", "10.0"], &["", "A", "42.0"], &["2.0", "B", "20.0"]]);
    let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    let out = pivot_tidy_long_to_datasheet("Blank Numeric X", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, '.').unwrap();
    let grp = as_grouped(&out);
    assert_eq!(grp.row_titles, vec!["1.0", "2.0", "(Missing)"]);
    assert_eq!(grp.data_sets[0].cells[2][0].active_number(), Some(42.0));
}

#[test]
fn audit_60_select_and_transform_carries_xy_x_and_survival_times() {
    let mut xy_sheet = xy_single("xy-proj", &[1.0, 2.0, 3.0], &[&[10.0, 20.0, 30.0], &[20.0, 40.0, 60.0]]);
    if let TableArchetype::Xy(xy) = &mut xy_sheet.archetype {
        xy.x_title = String::from("Concentration");
        xy.data_sets[0].title = String::from("Y0");
        xy.data_sets[1].title = String::from("Y1");
    }
    add_excl(&mut xy_sheet, CellCoord::new(1, 1, 0), "qc", "outlier");

    let ctx_one = TransformContext { selected_data_sets: vec![1], ..TransformContext::default() };
    let col_out = execute_select(&xy_sheet, &[], TableKind::Column, &ctx_one).unwrap();
    let col_tbl = as_col(&col_out.data_sheet);
    assert_eq!(col_tbl.data_sets.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(), vec!["Concentration", "Y1"]);
    assert_eq!(col_tbl.data_sets[0].cells[0][0].active_number(), Some(1.0));
    assert_eq!(col_out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(col_out.data_sheet.exclusion_notes[0].coord, CellCoord::new(1, 1, 0));

    let srv_out = execute_select(&xy_sheet, &[], TableKind::Survival, &ctx_one).unwrap();
    let s_tbl = crate::common::as_survival(&srv_out.data_sheet);
    assert_eq!(
        (s_tbl.time_title.as_str(), s_tbl.times[2].active_number(), s_tbl.cohorts.len(), s_tbl.cohorts[0].title.as_str()),
        ("Concentration", Some(3.0), 1, "Y1")
    );

    assert!(!execute_select(&xy_sheet, &[], TableKind::Contingency, &TransformContext::default()).unwrap().warnings.is_empty());

    // Survival -> Xy carries Survival.times into Xy.x_cells.
    let srv_sheet = survival_sheet("srv-proj", "Days", "Event", &[(7.0, 1.0), (14.0, 0.0)]);
    let xy_from_srv = execute_select(&srv_sheet, &[], TableKind::Xy, &TransformContext::default()).unwrap();
    let xsrv = as_xy(&xy_from_srv.data_sheet);
    assert_eq!(
        (xsrv.x_title.as_str(), xsrv.x_value_at(0), xsrv.x_value_at(1), xsrv.data_sets[0].title.as_str()),
        ("Days", Some(7.0), Some(14.0), "Event")
    );
}

#[test]
fn audit_61_prune_rows_remaps_and_prunes_exclusion_notes() {
    let mut s = col_nums("prune-notes", &[("A", &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0])]);
    for (r, tag, msg) in [(2, "qc2", "bin 1 first"), (3, "qc3", "bin 1 second"), (4, "qc4", "row 4 outlier")] {
        add_excl(&mut s, CellCoord::new(r, 0, 0), tag, msg);
    }
    let ctx = TransformContext::default();

    let nth = execute_prune(&s, PruneMethod::EveryNth(2), &ctx).unwrap();
    assert_eq!(
        nth.data_sheet.exclusion_notes.iter().map(|n| (n.coord, n.reason_tag.as_str())).collect::<Vec<_>>(),
        vec![(CellCoord::new(1, 0, 0), "qc2"), (CellCoord::new(2, 0, 0), "qc4")]
    );

    let binned = execute_prune(&s, PruneMethod::AverageBins(2), &ctx).unwrap();
    assert_eq!(
        binned.data_sheet.exclusion_notes.iter().map(|n| (n.coord, n.reason_tag.as_str())).collect::<Vec<_>>(),
        vec![(CellCoord::new(1, 0, 0), "qc2")]
    );
}

#[test]
fn audit_77_import_insert_rows_shifts_exclusion_notes_down() {
    let mut s = col_sheet("imp-notes", &[("A", &[num(1.0), excl(99.0), num(3.0)])]);
    add_excl(&mut s, CellCoord::new(1, 0, 0), "qc", "row 1 artifact");
    let spec = clipboard_spec(PlacementMode::InsertRows, CellCoord::new(1, 0, 0), ImportFilterSpec::default());
    import_csv(&mut s, "100\n200\n", &spec).unwrap();
    assert_eq!(s.exclusion_notes.len(), 1);
    assert_eq!(s.exclusion_notes[0].coord, CellCoord::new(3, 0, 0));
    let cell = s.cell(CellCoord::new(3, 0, 0)).unwrap();
    assert!(cell.excluded && cell.value.finite_f64() == Some(99.0));
}

#[test]
fn audit_94_column_title_numeric_hook_uses_doc_parser() {
    let mut src = col_nums("src-hook", &[("[I] = 10 nM", &[20.0, 30.0])]);
    let ctx = TransformContext::default();
    let hook = NormalizeAnchor::Hooked(HookSource::ColumnTitleValue { data_sheet_id: src.id.clone(), data_set_index: 0 });

    let ok = execute_normalize(&src, &NormalizeAnchor::FixedValue(0.0), &hook, false, false, &ctx).unwrap();
    assert_approx_eq(as_col(&ok.data_sheet).data_sets[0].cells[0][0].active_number().unwrap(), 2.0, 1e-12, 0.0, "hook = 10");

    // Rejects biological identifiers carrying numbers.
    for bad_title in ["Inhibitor-10 nM", "IL-6 10 ng/mL", "CO2 5%"] {
        as_col_mut(&mut src).data_sets[0].title = String::from(bad_title);
        let err = execute_normalize(&src, &NormalizeAnchor::FixedValue(0.0), &hook, false, false, &ctx).unwrap_err();
        assert!(matches!(err, TransformError::MissingHook { .. }), "title `{bad_title}` should reject");
    }
}

#[test]
fn wave4_mv_to_contingency_with_zero_label_columns_does_not_panic() {
    // Regression for `archetype_convert.rs` eager `label_cols[0]` evaluation when `label_cols` is empty.
    let s = mv_sheet("mv-cont-nolabel", &[("Alive", &[num(12.0), num(8.0)]), ("Dead", &[num(3.0), num(15.0)])]);
    let (converted, _) = convert_datasheet_archetype(&s, TargetArchetypeKind::Contingency, false).unwrap();
    let cont = as_contingency(&converted);
    assert_eq!((cont.data_sets.len(), cont.row_count()), (2, 2));
    assert_eq!((cont.data_sets[0].cells[0][0].active_number(), cont.data_sets[1].cells[1][0].active_number()), (Some(12.0), Some(15.0)));

    // Non-crosstab 1 Categorical + 2 Numeric MV with 0 Label columns returns a typed error instead of panicking.
    let mut mixed_mv =
        mv_table(&[("Group", &[txt("Control"), txt("Treated")]), ("CountA", &[num(5.0), num(9.0)]), ("CountB", &[num(7.0), num(4.0)])]);
    mixed_mv.variables[0].role = VariableRole::Categorical;
    let mixed_err = convert_datasheet_archetype(
        &sheet("mv-cont-mixed-nolabel", TableArchetype::MultipleVariables(mixed_mv)),
        TargetArchetypeKind::Contingency,
        false,
    )
    .unwrap_err();
    assert!(matches!(mixed_err, TransformError::Doc(_)));
}
