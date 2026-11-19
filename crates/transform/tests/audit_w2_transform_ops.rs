//! Wave 2 audit regression suite for `crates/transform` (#54, #55, #56, #59, #60, #61, #77, #94).

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, ErrorPropagationMode, GroupedTable, HookSource, ImportFilterSpec,
    ImportLinkSpec, ImportPlacementSpec, LinkSource, MathTransformPreset, MultipleVariablesTable, NormalizeAnchor, PlacementMode, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_testkit::assert_approx_eq;
use curvebook_transform::{
    BaselineKind, ImportPipelineOptions, ImportSourcePayload, MathTransformOp, PruneMethod, TargetArchetypeKind, TidyColumnMapping, TransformContext,
    TransformError, apply_import_link_pipeline, convert_datasheet_archetype, execute_math_op, execute_normalize, execute_prune,
    execute_remove_baseline_kind, execute_select, pivot_tidy_long_to_datasheet, preview_datasheet_conversion,
};

#[test]
fn audit_54_negative_slope_swaps_bounds_exclusions_notes_and_partial_limits() {
    // 1. Negative derivative LinearScale(-2, 10) on MeanLowHigh (FirstOrderDeltaMethod):
    //    swaps values, `.excluded` flags, AND `CellExclusionNote` subcolumn 1 <-> 2.
    let mut xy = XyTable::new(SubcolumnMode::MeanLowHigh, 1, 1).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.data_sets[0].cells[0] = vec![Cell::number(5.0), Cell::excluded_number(1.5), Cell::number(3.0)];
    let mut sheet = DataSheet::new("neg-mlh", "Neg MLH", TableArchetype::Xy(xy)).unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(0, 0, 1), "qc", "lower asymmetric tail excluded", "2026-11-19T00:00:00Z").unwrap();

    let ctx = TransformContext::default();
    let op = MathTransformOp::Preset(MathTransformPreset::LinearScale { slope: -2.0, intercept: 10.0 });
    let out = execute_math_op(&sheet, &op, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let TableArchetype::Xy(out_xy) = &out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx_eq(out_xy.data_sets[0].cells[0][0].active_number().unwrap(), 0.0, 1e-12, 0.0, "center");
    assert!(!out_xy.data_sets[0].cells[0][1].excluded);
    assert_approx_eq(out_xy.data_sets[0].cells[0][1].active_number().unwrap(), 6.0, 1e-12, 0.0, "swapped -Err");
    assert!(out_xy.data_sets[0].cells[0][2].excluded);
    assert_eq!(out_xy.data_sets[0].cells[0][2].value.finite_f64(), Some(3.0));
    assert_eq!(out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(out.data_sheet.exclusion_notes[0].coord, CellCoord::new(0, 0, 2));

    // 2. Partial-missing + asymmetric exclusion in UpperLowerLimits under negative baseline Ratio & empty center Reciprocal.
    let mut ul_xy = XyTable::new(SubcolumnMode::UpperLowerLimits, 1, 2).unwrap();
    ul_xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    ul_xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    ul_xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::excluded_number(8.0), Cell::empty()];
    ul_xy.data_sets[0].cells[1] = vec![Cell::empty(), Cell::number(2.0), Cell::empty()];
    let mut lim_sheet = DataSheet::new("neg-ul", "Neg UL", TableArchetype::Xy(ul_xy)).unwrap();
    lim_sheet.set_excluded_with_reason(CellCoord::new(0, 0, 1), "qc", "partial lower excluded", "2026-11-19T00:00:00Z").unwrap();

    let base_out = execute_remove_baseline_kind(&lim_sheet, &BaselineKind::Constant(-2.0), BaselineOperation::Ratio, &ctx).unwrap();
    let TableArchetype::Xy(base_xy) = &base_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx_eq(base_xy.data_sets[0].cells[0][0].active_number().unwrap(), -5.0, 1e-12, 0.0, "center");
    assert!(base_xy.data_sets[0].cells[0][1].is_empty() && !base_xy.data_sets[0].cells[0][1].excluded);
    assert!(base_xy.data_sets[0].cells[0][2].excluded);
    assert_eq!(base_xy.data_sets[0].cells[0][2].value.finite_f64(), Some(-4.0));
    assert_eq!(base_out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(base_out.data_sheet.exclusion_notes[0].coord, CellCoord::new(0, 0, 2));
    // Row 1 empty center + partial lower limit must also swap into upper slot (-1.0)
    assert!(base_xy.data_sets[0].cells[1][1].is_empty());
    assert_approx_eq(base_xy.data_sets[0].cells[1][2].active_number().unwrap(), -1.0, 1e-12, 0.0, "partial swapped");

    let recip_out = execute_math_op(&lim_sheet, &MathTransformOp::Reciprocal, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    let TableArchetype::Xy(rec_xy) = &recip_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert!(rec_xy.data_sets[0].cells[1][1].is_empty());
    assert_approx_eq(rec_xy.data_sets[0].cells[1][2].active_number().unwrap(), 0.5, 1e-12, 0.0, "empty-center partial reciprocal");
}

#[test]
fn audit_55_domain_blanked_spread_emits_explicit_warnings() {
    let ctx = TransformContext::default();

    // MeanCvN with mean_out == 0 emits explicit spread-blanked warning.
    let mut xy_cv = XyTable::new(SubcolumnMode::MeanCvN, 1, 1).unwrap();
    xy_cv.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_cv.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(10.0), Cell::number(5.0)];
    let cv_sheet = DataSheet::new("cv-zero", "CV Zero", TableArchetype::Xy(xy_cv)).unwrap();
    let cv_out = execute_math_op(&cv_sheet, &MathTransformOp::Ln, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    assert_eq!(cv_out.warnings.len(), 1);
    assert!(cv_out.warnings[0].contains("CV% spread blanked"));

    // UpperLowerLimits with one bound outside Ln domain blanks bound and warns.
    let mut xy_ul = XyTable::new(SubcolumnMode::UpperLowerLimits, 1, 1).unwrap();
    xy_ul.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_ul.data_sets[0].cells[0] = vec![Cell::number(2.0), Cell::number(-1.0), Cell::number(4.0)];
    let ul_sheet = DataSheet::new("ul-ood", "UL OOD", TableArchetype::Xy(xy_ul)).unwrap();
    let ul_out = execute_math_op(&ul_sheet, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx).unwrap();
    assert_eq!(ul_out.warnings.len(), 1);
    assert!(ul_out.warnings[0].contains("lower bound -1 outside transform domain"));
}

#[test]
fn audit_56_grouped_xy_mv_roundtrips_preserve_replicate_slots_and_warn_presummarized() {
    // 1. Grouped(RawReplicates(3)) with middle/leading empty replicate slots preserves slots via MV round-trip.
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(3), 1, 2).unwrap();
    grp.row_titles = vec![String::from("R0"), String::from("R1")];
    grp.data_sets[0].title = String::from("Drug");
    grp.data_sets[0].cells[0] = vec![Cell::empty(), Cell::number(42.0), Cell::empty()];
    grp.data_sets[0].cells[1] = vec![Cell::number(11.0), Cell::empty(), Cell::number(33.0)];
    let grp_sheet = DataSheet::new("grp-slots", "Slots", TableArchetype::Grouped(grp)).unwrap();

    let (mv_from_grp, _) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    let (grp_back, _) = convert_datasheet_archetype(&mv_from_grp, TargetArchetypeKind::Grouped, false).unwrap();
    let TableArchetype::Grouped(gb) = &grp_back.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(gb.subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert!(gb.data_sets[0].cells[0][0].is_empty());
    assert_eq!(gb.data_sets[0].cells[0][1].active_number(), Some(42.0));
    assert!(gb.data_sets[0].cells[0][2].is_empty());
    assert_eq!(gb.data_sets[0].cells[1][0].active_number(), Some(11.0));
    assert!(gb.data_sets[0].cells[1][1].is_empty());
    assert_eq!(gb.data_sets[0].cells[1][2].active_number(), Some(33.0));

    // 2. Xy(RawReplicates(3)) with empty leading slot preserves replicate slot identity via MV round-trip.
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(3), 1, 2).unwrap();
    xy.x_title = String::from("Conc");
    xy.data_sets[0].title = String::from("Signal");
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy.data_sets[0].cells[0] = vec![Cell::empty(), Cell::number(15.0), Cell::empty()];
    xy.data_sets[0].cells[1] = vec![Cell::number(5.0), Cell::empty(), Cell::number(25.0)];
    let xy_sheet = DataSheet::new("xy-slots", "XY Slots", TableArchetype::Xy(xy)).unwrap();
    let (mv_from_xy, _) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    let (xy_back, _) = convert_datasheet_archetype(&mv_from_xy, TargetArchetypeKind::Xy, false).unwrap();
    let TableArchetype::Xy(xb) = &xy_back.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xb.y_subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert!(xb.data_sets[0].cells[0][0].is_empty());
    assert_eq!(xb.data_sets[0].cells[0][1].active_number(), Some(15.0));
    assert_eq!(xb.data_sets[0].cells[1][2].active_number(), Some(25.0));

    // 3. MV one-factor -> Grouped keeps same source row paired in same replicate slot across columns.
    let mut mv1 = MultipleVariablesTable::new(3, 2).unwrap();
    mv1.variables[0].title = String::from("Media");
    mv1.variables[1].title = String::from("MeasureA");
    mv1.variables[2].title = String::from("MeasureB");
    mv1.variables[0].cells = vec![Cell::text("DMEM"), Cell::text("DMEM")];
    mv1.variables[1].cells = vec![Cell::number(10.0), Cell::number(20.0)];
    mv1.variables[2].cells = vec![Cell::empty(), Cell::number(200.0)];
    for v in &mut mv1.variables {
        v.refresh_inferred_role();
    }
    let mv1_sheet = DataSheet::new("mv1", "OneFac", TableArchetype::MultipleVariables(mv1)).unwrap();
    let (grp1_sheet, _) = convert_datasheet_archetype(&mv1_sheet, TargetArchetypeKind::Grouped, false).unwrap();
    let TableArchetype::Grouped(g1) = &grp1_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(g1.subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert!(g1.data_sets[1].cells[0][0].is_empty());
    assert_eq!(g1.data_sets[1].cells[0][1].active_number(), Some(200.0));
}

#[test]
fn audit_56_presummarized_grouped_warns_and_sparse_explicit_replicate_slots() {
    // 4. Grouped(MeanSdN) -> MultipleVariables flags schema_lossy + emits warning.
    let mut grp_sum = GroupedTable::new(SubcolumnMode::MeanSdN, 1, 1).unwrap();
    grp_sum.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(1.0), Cell::number(4.0)];
    let grp_sum_sheet = DataSheet::new("grp-sum", "Summary", TableArchetype::Grouped(grp_sum)).unwrap();
    let prev = preview_datasheet_conversion(&grp_sum_sheet, TargetArchetypeKind::MultipleVariables).unwrap();
    assert!(prev.lossy);
    assert!(!prev.warnings.is_empty());
    assert!(matches!(
        convert_datasheet_archetype(&grp_sum_sheet, TargetArchetypeKind::MultipleVariables, false),
        Err(TransformError::InvalidSpec { .. })
    ));
    let (_, conv_prev) = convert_datasheet_archetype(&grp_sum_sheet, TargetArchetypeKind::MultipleVariables, true).unwrap();
    assert!(conv_prev.lossy);

    // 5. Direct MV -> XY and MV two-way -> Grouped with sparse explicit Replicate = 2, 4 preserves slots 1 and 3.
    let mut mv_sparse = MultipleVariablesTable::new(4, 2).unwrap();
    mv_sparse.variables[0].title = String::from("Time");
    mv_sparse.variables[1].title = String::from("Group");
    mv_sparse.variables[2].title = String::from("Replicate");
    mv_sparse.variables[3].title = String::from("Value");
    mv_sparse.variables[0].cells = vec![Cell::number(1.0), Cell::number(1.0)];
    mv_sparse.variables[1].cells = vec![Cell::text("Ctrl"), Cell::text("Ctrl")];
    mv_sparse.variables[2].cells = vec![Cell::number(2.0), Cell::number(4.0)];
    mv_sparse.variables[3].cells = vec![Cell::number(12.0), Cell::number(14.0)];
    for v in &mut mv_sparse.variables {
        v.refresh_inferred_role();
    }
    let mv_sp_sheet = DataSheet::new("mv-sparse-rep", "Sparse Rep", TableArchetype::MultipleVariables(mv_sparse.clone())).unwrap();
    let (xy_sp, _) = convert_datasheet_archetype(&mv_sp_sheet, TargetArchetypeKind::Xy, false).unwrap();
    let TableArchetype::Xy(xsp) = &xy_sp.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xsp.y_subcolumn_mode, SubcolumnMode::RawReplicates(4));
    assert!(xsp.data_sets[0].cells[0][0].is_empty());
    assert_eq!(xsp.data_sets[0].cells[0][1].active_number(), Some(12.0));
    assert!(xsp.data_sets[0].cells[0][2].is_empty());
    assert_eq!(xsp.data_sets[0].cells[0][3].active_number(), Some(14.0));

    mv_sparse.variables[0].lock_role(curvebook_doc::VariableRole::Categorical);
    let mv_grp_sp = DataSheet::new("mv-grp-sparse-rep", "Sparse Grp Rep", TableArchetype::MultipleVariables(mv_sparse)).unwrap();
    let (grp_sp, _) = convert_datasheet_archetype(&mv_grp_sp, TargetArchetypeKind::Grouped, false).unwrap();
    let TableArchetype::Grouped(gsp) = &grp_sp.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(gsp.subcolumn_mode, SubcolumnMode::RawReplicates(4));
    assert!(gsp.data_sets[0].cells[0][0].is_empty());
    assert_eq!(gsp.data_sets[0].cells[0][1].active_number(), Some(12.0));
    assert!(gsp.data_sets[0].cells[0][2].is_empty());
    assert_eq!(gsp.data_sets[0].cells[0][3].active_number(), Some(14.0));
}

#[test]
fn audit_59_tidy_grouped_preserves_blank_numeric_x_records_in_missing_row() {
    let headers = vec![String::from("Dose"), String::from("Group"), String::from("Response")];
    let rows = vec![
        vec![String::from("1.0"), String::from("A"), String::from("10.0")],
        vec![String::from(""), String::from("A"), String::from("42.0")],
        vec![String::from("2.0"), String::from("B"), String::from("20.0")],
    ];
    let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    let sheet = pivot_tidy_long_to_datasheet("Blank Numeric X", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, '.').unwrap();
    let TableArchetype::Grouped(grp) = &sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp.row_titles, vec!["1.0", "2.0", "(Missing)"]);
    assert_eq!(grp.data_sets[0].cells[2][0].active_number(), Some(42.0));
}

#[test]
fn audit_60_select_and_transform_carries_xy_x_and_survival_times() {
    // 1. XY -> Column / MV / Survival carries X as first column (or survival Time), and XY -> Contingency warns.
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 3).unwrap();
    xy.x_title = String::from("Concentration");
    xy.data_sets[0].title = String::from("Y0");
    xy.data_sets[1].title = String::from("Y1");
    for r in 0..3 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(10.0 * (r + 1) as f64);
        xy.data_sets[1].cells[r][0] = Cell::number(20.0 * (r + 1) as f64);
    }
    let mut xy_sheet = DataSheet::new("xy-proj", "XY Proj", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.set_excluded_with_reason(CellCoord::new(1, 1, 0), "qc", "outlier", "2026-11-19T00:00:00Z").unwrap();

    let ctx_one = TransformContext { selected_data_sets: vec![1], ..TransformContext::default() };
    let col_out = execute_select(&xy_sheet, &[], TableKind::Column, &ctx_one).unwrap();
    let TableArchetype::Column(col_tbl) = &col_out.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_tbl.data_sets.len(), 2);
    assert_eq!(col_tbl.data_sets[0].title, "Concentration");
    assert_eq!(col_tbl.data_sets[1].title, "Y1");
    assert_eq!(col_tbl.data_sets[0].cells[0][0].active_number(), Some(1.0));
    assert_eq!(col_out.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(col_out.data_sheet.exclusion_notes[0].coord, CellCoord::new(1, 1, 0));

    let srv_from_xy = execute_select(&xy_sheet, &[], TableKind::Survival, &ctx_one).unwrap();
    let TableArchetype::Survival(s_tbl) = &srv_from_xy.data_sheet.archetype else {
        panic!("expected Survival");
    };
    assert_eq!(s_tbl.time_title, "Concentration");
    assert_eq!(s_tbl.times[2].active_number(), Some(3.0));
    assert_eq!(s_tbl.cohorts.len(), 1);
    assert_eq!(s_tbl.cohorts[0].title, "Y1");

    let cont_warn = execute_select(&xy_sheet, &[], TableKind::Contingency, &TransformContext::default()).unwrap();
    assert!(!cont_warn.warnings.is_empty());

    // 2. Survival -> Xy carries Survival.times into Xy.x_cells.
    let mut srv = SurvivalTable::new(1, 2).unwrap();
    srv.time_title = String::from("Days");
    srv.cohorts[0].title = String::from("Event");
    srv.times = vec![Cell::number(7.0), Cell::number(14.0)];
    srv.cohorts[0].cells[0][0] = Cell::number(1.0);
    srv.cohorts[0].cells[1][0] = Cell::number(0.0);
    let srv_sheet = DataSheet::new("srv-proj", "Srv Proj", TableArchetype::Survival(srv)).unwrap();
    let xy_from_srv = execute_select(&srv_sheet, &[], TableKind::Xy, &TransformContext::default()).unwrap();
    let TableArchetype::Xy(xsrv) = &xy_from_srv.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xsrv.x_title, "Days");
    assert_eq!(xsrv.x_value_at(0), Some(7.0));
    assert_eq!(xsrv.x_value_at(1), Some(14.0));
    assert_eq!(xsrv.data_sets[0].title, "Event");
}

#[test]
fn audit_61_prune_rows_remaps_and_prunes_exclusion_notes() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 6).unwrap();
    for r in 0..6 {
        col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
    }
    let mut sheet = DataSheet::new("prune-notes", "Prune Notes", TableArchetype::Column(col)).unwrap();
    // Mark rows 2 & 3 excluded with notes (in bin 1 = rows 2..4 under AverageBins(2)), plus row 4 (kept by EveryNth(2)).
    sheet.set_excluded_with_reason(CellCoord::new(2, 0, 0), "qc2", "bin 1 first", "2026-11-19T00:00:00Z").unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(3, 0, 0), "qc3", "bin 1 second", "2026-11-19T00:00:00Z").unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(4, 0, 0), "qc4", "row 4 outlier", "2026-11-19T00:00:00Z").unwrap();

    let ctx = TransformContext::default();
    let nth = execute_prune(&sheet, PruneMethod::EveryNth(2), &ctx).unwrap();
    // Retained rows [0, 2, 4] -> old rows 2 and 4 become new rows 1 and 2.
    assert_eq!(nth.data_sheet.exclusion_notes.len(), 2);
    assert_eq!(nth.data_sheet.exclusion_notes[0].coord, CellCoord::new(1, 0, 0));
    assert_eq!(nth.data_sheet.exclusion_notes[0].reason_tag, "qc2");
    assert_eq!(nth.data_sheet.exclusion_notes[1].coord, CellCoord::new(2, 0, 0));
    assert_eq!(nth.data_sheet.exclusion_notes[1].reason_tag, "qc4");

    let binned = execute_prune(&sheet, PruneMethod::AverageBins(2), &ctx).unwrap();
    // Bin 0 (0,1): active; Bin 1 (2,3): both excluded -> excluded bin note at row 1; Bin 2 (4,5): active merged -> row 4 note pruned!
    assert_eq!(binned.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(binned.data_sheet.exclusion_notes[0].coord, CellCoord::new(1, 0, 0));
    assert_eq!(binned.data_sheet.exclusion_notes[0].reason_tag, "qc2");
}

#[test]
fn audit_77_import_insert_rows_shifts_exclusion_notes_down() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 3).unwrap();
    col.data_sets[0].cells[0][0] = Cell::number(1.0);
    col.data_sets[0].cells[1][0] = Cell::excluded_number(99.0);
    col.data_sets[0].cells[2][0] = Cell::number(3.0);
    let mut sheet = DataSheet::new("imp-notes", "Import Notes", TableArchetype::Column(col)).unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(1, 0, 0), "qc", "row 1 artifact", "2026-11-19T00:00:00Z").unwrap();

    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("ins.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::InsertRows,
            anchor: CellCoord::new(1, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    apply_import_link_pipeline(
        &mut sheet,
        &spec,
        ImportSourcePayload::DelimitedText { text: "100\n200\n", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(sheet.exclusion_notes.len(), 1);
    assert_eq!(sheet.exclusion_notes[0].coord, CellCoord::new(3, 0, 0));
    assert!(sheet.cell(CellCoord::new(3, 0, 0)).unwrap().excluded);
    assert_eq!(sheet.cell(CellCoord::new(3, 0, 0)).unwrap().value.finite_f64(), Some(99.0));
}

#[test]
fn audit_94_column_title_numeric_hook_uses_doc_parser() {
    let mut src = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    src.data_sets[0].title = String::from("Signal");
    src.data_sets[0].cells[0][0] = Cell::number(20.0);
    src.data_sets[0].cells[1][0] = Cell::number(30.0);
    let mut src_sheet = DataSheet::new("src-hook", "Source", TableArchetype::Column(src)).unwrap();
    let ctx = TransformContext::default();

    // Accepts strict explicit assignment `[I] = 10 nM`.
    if let TableArchetype::Column(c) = &mut src_sheet.archetype {
        c.data_sets[0].title = String::from("[I] = 10 nM");
    }
    let ok = execute_normalize(
        &src_sheet,
        &NormalizeAnchor::FixedValue(0.0),
        &NormalizeAnchor::Hooked(HookSource::ColumnTitleValue { data_sheet_id: src_sheet.id.clone(), data_set_index: 0 }),
        false,
        false,
        &ctx,
    )
    .unwrap();
    let TableArchetype::Column(oc) = &ok.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_approx_eq(oc.data_sets[0].cells[0][0].active_number().unwrap(), 2.0, 1e-12, 0.0, "hook = 10");

    // Rejects biological identifiers carrying numbers (`Inhibitor-10 nM`, `IL-6 10 ng/mL`, `CO2 5%`).
    for bad_title in ["Inhibitor-10 nM", "IL-6 10 ng/mL", "CO2 5%"] {
        if let TableArchetype::Column(c) = &mut src_sheet.archetype {
            c.data_sets[0].title = String::from(bad_title);
        }
        let err = execute_normalize(
            &src_sheet,
            &NormalizeAnchor::FixedValue(0.0),
            &NormalizeAnchor::Hooked(HookSource::ColumnTitleValue { data_sheet_id: src_sheet.id.clone(), data_set_index: 0 }),
            false,
            false,
            &ctx,
        )
        .unwrap_err();
        assert!(matches!(err, TransformError::MissingHook { .. }), "title `{bad_title}` should reject");
    }
}
