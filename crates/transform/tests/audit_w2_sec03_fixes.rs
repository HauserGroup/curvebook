#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Targeted regressions covering all 16 confirmed `SEC-03` audit findings (`FA-SEC03-01`..`FA-SEC03-16`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CiBound, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, DocError,
    ErrorPropagationMode, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, LinkSource,
    MultipleVariablesTable, NormalizeAnchor, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange,
    SheetId, SimulationNoise, SubcolumnMode, TableArchetype, TableKind, TransformAnalysisSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::import::MAX_IMPORT_CELLS;
use curvebook_transform::{
    AucOptions, DistributionSimulationConfig, MathTransformOp, PlateGeometry, PlateMapTemplate, PlateWellRole, PruneMethod, SimulatedColumnSpec,
    SimulatedDistribution, SmoothMethod, TargetArchetypeKind, TidyColumnMapping, TransformContext, TransformError, apply_import_link_pipeline,
    compute_auc, convert_datasheet_archetype, evaluate_formula_for_sheet, execute_analysis_transform, execute_math_op, execute_math_transform,
    execute_normalize, execute_prune, execute_select, execute_transpose, pivot_tidy_long_to_datasheet, simulate_distribution_columns,
    simulate_xy_sheet, smooth_xy,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} ± {tol}, got {actual}");
}

// ============================================================================
// FA-SEC03-01: Non-axis Column -> Xy / Survival shifts exclusion notes by -1
//              and drops col-0 notes with an explicit warning.
// ============================================================================

#[test]
fn fa_sec03_01_select_column_to_xy_and_survival_shifts_notes_and_warns() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap();
    col.data_sets[0].title = String::from("TimeX");
    col.data_sets[1].title = String::from("SignalY");
    col.data_sets[0].cells[0][0] = Cell::excluded_number(1.0);
    col.data_sets[0].cells[1][0] = Cell::number(2.0);
    col.data_sets[1].cells[0][0] = Cell::number(10.0);
    col.data_sets[1].cells[1][0] = Cell::excluded_number(20.0);

    let mut sheet = DataSheet::new("col-sel", "Column Select", TableArchetype::Column(col)).unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(0, 0, 0), "qc0", "bad axis point", "2026-11-22T00:00:00Z").unwrap();
    sheet.set_excluded_with_reason(CellCoord::new(1, 1, 0), "qc1", "bad response", "2026-11-22T00:00:00Z").unwrap();

    let ctx = TransformContext::default();
    for target in [TableKind::Xy, TableKind::Survival] {
        let out = execute_select(&sheet, &[], target, &ctx).unwrap();
        assert_eq!(out.data_sheet.exclusion_notes.len(), 1, "col-0 axis note must not misalign into Y col 0 for {target:?}");
        assert_eq!(out.data_sheet.exclusion_notes[0].coord, CellCoord::new(1, 0, 0));
        assert_eq!(out.data_sheet.exclusion_notes[0].reason_tag, "qc1");
        assert!(
            out.warnings.iter().any(|w| w.contains("could not be retained as cell exclusion notes")),
            "expected col-0 note drop warning for {target:?}, got {:?}",
            out.warnings
        );
    }
}

// ============================================================================
// FA-SEC03-02: Transpose rejects empty tables, remaps exclusion notes, preserves
//              DateOrElapsedTime labels & -0.0 normalization, and warns on drops.
// ============================================================================

#[test]
fn fa_sec03_02_transpose_guards_notes_dates_and_warnings() {
    let ctx = TransformContext::default();

    // 1. 0-row table rejected cleanly instead of constructing invalid ColumnTable.
    let empty_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 0).unwrap();
    let empty_sheet = DataSheet::new("empty-col", "Empty", TableArchetype::Column(empty_col)).unwrap();
    assert!(matches!(execute_transpose(&empty_sheet, true, &ctx), Err(TransformError::InvalidInput { .. })));

    // 2. Column transpose swaps exclusion note (row, ds) -> (ds, row).
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 2).unwrap();
    col.data_sets[2].cells[1][0] = Cell::excluded_number(99.0);
    let mut col_sheet = DataSheet::new("tr-notes", "Notes", TableArchetype::Column(col)).unwrap();
    col_sheet.set_excluded_with_reason(CellCoord::new(1, 2, 0), "qc", "col2 row1", "2026-11-22T00:00:00Z").unwrap();
    let tr_col = execute_transpose(&col_sheet, true, &ctx).unwrap();
    assert_eq!(tr_col.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(tr_col.data_sheet.exclusion_notes[0].coord, CellCoord::new(2, 1, 0));

    // 3. Xy DateOrElapsedTime preserves text labels & emits X-discard warning when !swap_titles.
    let mut xy_dt =
        XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("day") }, SubcolumnMode::Single, 1, 2).unwrap();
    xy_dt.x_cells[0] = vec![Cell::text("2026-01-01")];
    xy_dt.x_cells[1] = vec![Cell::text("2026-01-02")];
    xy_dt.data_sets[0].cells[0][0] = Cell::number(5.0);
    xy_dt.data_sets[0].cells[1][0] = Cell::number(6.0);
    let dt_sheet = DataSheet::new("xy-dt", "Dates", TableArchetype::Xy(xy_dt)).unwrap();

    let tr_swap = execute_transpose(&dt_sheet, true, &ctx).unwrap();
    let TableArchetype::Column(c_swap) = &tr_swap.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(c_swap.data_sets[0].title, "2026-01-01");
    assert_eq!(c_swap.data_sets[1].title, "2026-01-02");
    assert_eq!(c_swap.row_titles, vec!["A"]);

    let tr_noswap = execute_transpose(&dt_sheet, false, &ctx).unwrap();
    assert!(tr_noswap.warnings.iter().any(|w| w.contains("discarded source X values")), "expected X discard warning: {:?}", tr_noswap.warnings);

    // 4. X error subcolumns + -0.0 X coordinate normalize "-0" -> "0" and warn of omitted X error subcolumns.
    let mut xy_err = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::Single, 1, 1).unwrap();
    xy_err.x_cells[0] = vec![Cell::number(-0.0), Cell::number(0.5), Cell::number(3.0)];
    xy_err.data_sets[0].cells[0][0] = Cell::number(12.0);
    let err_sheet = DataSheet::new("xy-xerr", "X Err", TableArchetype::Xy(xy_err)).unwrap();
    let tr_err = execute_transpose(&err_sheet, true, &ctx).unwrap();
    let TableArchetype::Column(c_err) = &tr_err.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(c_err.data_sets[0].title, "0");
    assert!(tr_err.warnings.iter().any(|w| w.contains("X error subcolumns")), "expected X error warning: {:?}", tr_err.warnings);
}

// ============================================================================
// FA-SEC03-03: Multi-InfoSheet TransformContext, Dirty ResultScalar rejection,
//              and ImportFilterSpec::exclude_row_predicate Info() lookup.
// ============================================================================

#[test]
fn fa_sec03_03_multi_info_sheet_dirty_scalar_and_import_filter_info() {
    let mut info_a = InfoSheet::new_preseeded("info-a", "A").unwrap();
    info_a.set_constant("K", ConstantValue::Number(10.0), None).unwrap();
    let mut info_b = InfoSheet::new_preseeded("info-b", "B").unwrap();
    info_b.set_constant("K", ConstantValue::Number(50.0), None).unwrap();

    let sheets = [info_a.clone(), info_b];
    let ctx = TransformContext { info_sheet: Some(&info_a), info_sheets: &sheets, ..TransformContext::default() };

    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy.data_sets[0].cells[0][0] = Cell::number(25.0);
    xy.data_sets[0].cells[1][0] = Cell::number(50.0);
    let sheet = DataSheet::new("s-info", "S", TableArchetype::Xy(xy)).unwrap();

    // Explicit hook targets info-b (K=50) while info_sheet is info-a (K=10).
    let norm_b = execute_normalize(
        &sheet,
        &NormalizeAnchor::FixedValue(0.0),
        &NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-b")), constant_name: String::from("K") }),
        true,
        true,
        &ctx,
    )
    .unwrap();
    let TableArchetype::Xy(n_xy) = &norm_b.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(n_xy.data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);
    assert_approx(n_xy.data_sets[0].cells[1][0].active_number().unwrap(), 100.0, 1e-12);

    // Unknown explicit info_sheet_id rejects cleanly instead of silently reading info_a.
    let err_missing = execute_normalize(
        &sheet,
        &NormalizeAnchor::FixedValue(0.0),
        &NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("missing-info")), constant_name: String::from("K") }),
        true,
        true,
        &ctx,
    )
    .unwrap_err();
    assert!(matches!(err_missing, TransformError::MissingHook { .. }));

    // Dirty upstream ResultSheet scalar hook rejects cleanly.
    let spec =
        AnalysisSpec::new("up", "s-info", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap();
    let mut dirty_res = ResultSheet::new(spec).unwrap();
    dirty_res.status = ResultStatus::Dirty;
    dirty_res.scalars.push(ResultScalarEntry { key: String::from("Ref"), value: 100.0, units: None, data_set_index: None });
    let dirty_ctx = TransformContext { result_sheets: std::slice::from_ref(&dirty_res), ..TransformContext::default() };
    let dirty_err = execute_normalize(
        &sheet,
        &NormalizeAnchor::FixedValue(0.0),
        &NormalizeAnchor::Hooked(HookSource::ResultScalar {
            result_sheet_id: SheetId::new("up"),
            parameter_key: String::from("Ref"),
            data_set_index: None,
        }),
        true,
        true,
        &dirty_ctx,
    )
    .unwrap_err();
    assert!(matches!(dirty_err, TransformError::MissingHook { .. }) && dirty_err.to_string().contains("dirty"), "{dirty_err:?}");

    // Import pipeline forwards InfoSheet into filter exclude_row_predicate.
    let mut dst =
        DataSheet::new("imp-f", "Import Filter", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap()))
            .unwrap();
    let link = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { exclude_row_predicate: Some(String::from("col(0) > Info(\"K\")")), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec::default(),
    };
    let mut info_a_mut = info_a;
    apply_import_link_pipeline(
        &mut dst,
        &link,
        curvebook_transform::ImportSourcePayload::DelimitedText { text: "5\n15\n", delimiter: ',', decimal_separator: '.' },
        &Default::default(),
        Some(&mut info_a_mut),
    )
    .unwrap();
    let TableArchetype::Column(dst_col) = &dst.archetype else {
        panic!("expected Column");
    };
    assert_eq!(dst_col.data_sets[0].cells[0][0].active_number(), Some(5.0));
    assert!(dst_col.data_sets[0].cells[1][0].excluded);
}

// ============================================================================
// FA-SEC03-04: Output archetype invariant validation (Contingency counts)
// ============================================================================

#[test]
fn fa_sec03_04_contingency_validation_after_math_norm_and_prune() {
    let mut cont = ContingencyTable::new(2, 2).unwrap();
    cont.data_sets[0].cells[0][0] = Cell::number(3.0);
    cont.data_sets[1].cells[0][0] = Cell::number(4.0);
    cont.data_sets[0].cells[1][0] = Cell::number(2.0);
    cont.data_sets[1].cells[1][0] = Cell::number(5.0);
    let sheet = DataSheet::new("cont", "Contingency", TableArchetype::Contingency(cont)).unwrap();
    let ctx = TransformContext::default();

    // Negative shift makes counts negative -> DocError::InvalidCellValue
    assert!(matches!(
        execute_math_op(&sheet, &MathTransformOp::AddConstant(-10.0), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx),
        Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))
    ));

    // Fractional normalization makes counts non-integer -> DocError::InvalidCellValue
    assert!(matches!(
        execute_normalize(&sheet, &NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::FixedValue(10.0), false, false, &ctx),
        Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))
    ));

    // AverageBins on odd-sum Contingency rows yields 2.5 -> rejected cleanly
    assert!(matches!(
        execute_prune(&sheet, PruneMethod::AverageBins(2), &ctx),
        Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))
    ));
}

// ============================================================================
// FA-SEC03-05: LogarithmicSpacing pruning handles unsorted & descending X
// ============================================================================

#[test]
fn fa_sec03_05_prune_log_spaced_sorts_unsorted_and_descending_x() {
    let ctx = TransformContext::default();
    for xs in [vec![1000.0, 100.0, 10.0, 1.0], vec![100.0, 1.0, 1000.0, 10.0]] {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, xs.len()).unwrap();
        for (r, &x) in xs.iter().enumerate() {
            xy.set_x_cell(r, Cell::number(x)).unwrap();
            xy.data_sets[0].cells[r][0] = Cell::number(x * 2.0);
        }
        let sheet = DataSheet::new("log-xy", "Log XY", TableArchetype::Xy(xy)).unwrap();
        let pruned = execute_prune(&sheet, PruneMethod::LogarithmicSpacing { target_points: 2 }, &ctx).unwrap();
        let TableArchetype::Xy(out_xy) = &pruned.data_sheet.archetype else {
            panic!("expected XY");
        };
        let retained_xs: Vec<f64> = (0..out_xy.row_count()).map(|r| out_xy.x_value_at(r).unwrap()).collect();
        assert_eq!(retained_xs.len(), 2, "must pick both min=1 and max=1000 endpoints: {retained_xs:?}");
        assert!(retained_xs.contains(&1.0) && retained_xs.contains(&1000.0), "got {retained_xs:?}");
    }
}

// ============================================================================
// FA-SEC03-06: AUC option bounds + tied-X LOWESS robustness weights
// ============================================================================

#[test]
fn fa_sec03_06_curve_auc_options_and_tied_lowess_weights() {
    let pts = [(0.0, 0.0), (1.0, 2.0), (2.0, 0.0)];
    for bad in [
        AucOptions { min_peak_height: -0.1, ..AucOptions::default() },
        AucOptions { min_peak_height: f64::NAN, ..AucOptions::default() },
        AucOptions { min_area_fraction: -0.05, ..AucOptions::default() },
        AucOptions { min_area_fraction: 1.5, ..AucOptions::default() },
        AucOptions { min_area_fraction: f64::INFINITY, ..AucOptions::default() },
    ] {
        assert!(matches!(compute_auc(&pts, &bad), Err(TransformError::InvalidSpec { .. })));
    }

    // All tied X values (max_d == 0): robustness iterations must downweight the outlier at y=100.0.
    let tied = vec![(5.0, 10.0), (5.0, 11.0), (5.0, 10.5), (5.0, 100.0)];
    let smoothed = smooth_xy(&tied, SmoothMethod::Lowess { span: 1.0, robustness_iters: 3 }).unwrap();
    let unweighted_mean = (10.0 + 11.0 + 10.5 + 100.0) / 4.0;
    assert!(smoothed[0].1 < 15.0 && smoothed[0].1 < unweighted_mean - 15.0, "tied LOWESS ignored robustness weights: {:?}", smoothed);
}

// ============================================================================
// FA-SEC03-07: Normalize wide spans & %CV warnings + Simulate wide X bounds
// ============================================================================

#[test]
fn fa_sec03_07_normalize_wide_span_simulate_wide_bounds_and_cv_warnings() {
    let ctx = TransformContext::default();

    // 1. Wide normalization span (-1e308 to +1e308) uses half-scaling without overflowing span.
    let mut wide_xy = XyTable::new(SubcolumnMode::Single, 1, 1).unwrap();
    wide_xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    wide_xy.data_sets[0].cells[0][0] = Cell::number(0.0);
    let wide_sheet = DataSheet::new("wide-n", "Wide", TableArchetype::Xy(wide_xy)).unwrap();
    let norm_wide =
        execute_normalize(&wide_sheet, &NormalizeAnchor::FixedValue(-1e308), &NormalizeAnchor::FixedValue(1e308), true, true, &ctx).unwrap();
    let TableArchetype::Xy(nw) = &norm_wide.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(nw.data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);

    // 2. MeanCvN zero normalized center blanks CV and appends warning.
    let mut cv_xy = XyTable::new(SubcolumnMode::MeanCvN, 1, 1).unwrap();
    cv_xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    cv_xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(15.0), Cell::number(4.0)];
    let cv_sheet = DataSheet::new("cv-n", "CV", TableArchetype::Xy(cv_xy)).unwrap();
    let cv_norm = execute_normalize(&cv_sheet, &NormalizeAnchor::FixedValue(10.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    let TableArchetype::Xy(cv_out) = &cv_norm.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert!(cv_out.data_sets[0].cells[0][1].is_empty());
    assert!(cv_norm.warnings.iter().any(|w| w.contains("CV%") && w.contains("blanked")), "expected CV warning: {:?}", cv_norm.warnings);

    // 3. Simulate wide x_min = -1e308, x_max = +1e308 avoids overflow in x_max - x_min.
    let sim = simulate_xy_sheet(
        "sim-w",
        "Wide Sim",
        1,
        3,
        1,
        -1e308,
        1e308,
        "linear",
        &[(String::from("slope"), 0.0), (String::from("intercept"), 1.0)],
        SimulationNoise::None,
    )
    .unwrap();
    let TableArchetype::Xy(sim_xy) = &sim.archetype else {
        panic!("expected XY");
    };
    assert_eq!(sim_xy.x_value_at(0), Some(-1e308));
    assert_approx(sim_xy.x_value_at(1).unwrap(), 0.0, 1e-12);
    assert_eq!(sim_xy.x_value_at(2), Some(1e308));
}

// ============================================================================
// FA-SEC03-08: JsonParser O(N) on large non-ASCII .cbtemplate JSON
// ============================================================================

#[test]
fn fa_sec03_08_json_parser_linear_on_large_non_ascii_template() {
    let mut wells = Vec::with_capacity(384);
    let label = "µg_αβγ_".repeat(16); // multi-byte UTF-8 under MAX_TITLE_BYTES (255)
    for i in 0..384 {
        wells.push(PlateWellRole::Treatment { group_name: format!("{label}{i}"), dilution_x: (i + 1) as f64, replicate: i % 4 });
    }
    let tpl = PlateMapTemplate::from_wells("NonASCII Plate", PlateGeometry::Wells384, "Conc (µM)", wells).unwrap();
    let json = tpl.to_cbtemplate_json().unwrap();
    assert!(json.len() > 60_000);
    let parsed = PlateMapTemplate::from_cbtemplate_json(&json).unwrap();
    assert_eq!(parsed.wells.len(), 384);
    assert_eq!(parsed, tpl);
}

// ============================================================================
// FA-SEC03-09: Tidy exclusions (`*...*`), MV X exclusion notes, decimal_sep
//              replicates, and slot-collision FloatingNote warnings
// ============================================================================

#[test]
fn fa_sec03_09_tidy_exclusions_decimal_comma_replicates_and_collisions() {
    // 1. Symmetric `*...*` stripping, excluded X -> CellExclusionNote in MV, and decimal_sep ',' replicate parse
    let headers = vec![String::from("X"), String::from("Group"), String::from("Rep"), String::from("Val")];
    let rows = vec![
        vec![String::from("*10,0*"), String::from("Drug"), String::from("1,5"), String::from("*42,5*")],
        vec![String::from("10,0"), String::from("Drug"), String::from("1,5"), String::from("99,0")],
        vec![String::from("20,0"), String::from("Drug"), String::from("2,0"), String::from("50,0")],
    ];
    let map = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: Some(2), value_col: 3, excluded_col: None };

    let mv_sheet = pivot_tidy_long_to_datasheet("MV Tidy", &headers, &rows, &map, TargetArchetypeKind::MultipleVariables, ',').unwrap();
    let TableArchetype::MultipleVariables(mv) = &mv_sheet.archetype else {
        panic!("expected MV");
    };
    assert!(mv.variables[0].cells[0].excluded);
    assert_approx(mv.variables[0].cells[0].value.finite_f64().unwrap(), 10.0, 1e-12);
    assert!(mv.variables[3].cells[0].excluded);
    assert_approx(mv.variables[3].cells[0].value.finite_f64().unwrap(), 42.5, 1e-12);
    assert_approx(mv.variables[2].cells[0].active_number().unwrap(), 1.5, 1e-12);
    assert!(mv_sheet.exclusion_notes.iter().any(|n| n.coord == CellCoord::new(0, 0, 0)), "excluded X note missing: {:?}", mv_sheet.exclusion_notes);

    // 2. Xy and Grouped attach collision warning FloatingNote when replicate slots collide
    let xy_sheet = pivot_tidy_long_to_datasheet("XY Tidy", &headers, &rows, &map, TargetArchetypeKind::Xy, ',').unwrap();
    assert!(
        xy_sheet.floating_notes.iter().any(|n| n.body_markdown.contains("collision")),
        "expected collision FloatingNote in XY: {:?}",
        xy_sheet.floating_notes
    );

    let grp_sheet = pivot_tidy_long_to_datasheet("Grp Tidy", &headers, &rows, &map, TargetArchetypeKind::Grouped, ',').unwrap();
    assert!(
        grp_sheet.floating_notes.iter().any(|n| n.body_markdown.contains("collision")),
        "expected collision FloatingNote in Grouped: {:?}",
        grp_sheet.floating_notes
    );
}

// ============================================================================
// FA-SEC03-10 & FA-SEC03-16: Archetype conversion budgets, row_titles,
//              dropped replicate metrics/warnings, -0.0 labels, empty summary
// ============================================================================

#[test]
fn fa_sec03_10_and_16_archetype_convert_budgets_row_titles_dropped_reps_and_empty_summary() {
    // 1. Grouped -> MV preserves row_titles without breaking non-lossy roundtrip
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    grp.row_titles = vec![String::from("Control"), String::from("Treated")];
    grp.data_sets[0].title = String::from("Batch1");
    grp.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(2.0)];
    grp.data_sets[0].cells[1] = vec![Cell::number(3.0), Cell::number(4.0)];
    let grp_sheet = DataSheet::new("grp-rt", "Grp RT", TableArchetype::Grouped(grp)).unwrap();
    let (mv_from_grp, grp_prev) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!grp_prev.lossy);
    let TableArchetype::MultipleVariables(mv_grp) = &mv_from_grp.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mv_grp.row_titles, vec!["Control", "Control", "Treated", "Treated"]);

    // 2. XY(RawReplicates) -> MV preserves xy.row_titles
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap();
    xy_rep.set_x_cell(0, Cell::number(5.0)).unwrap();
    xy_rep.row_titles = vec![String::from("SampleA")];
    xy_rep.row_titles_expanded = true;
    xy_rep.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(20.0)];
    let xy_sheet = DataSheet::new("xy-rt", "XY RT", TableArchetype::Xy(xy_rep)).unwrap();
    let (mv_from_xy, _) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    let TableArchetype::MultipleVariables(mv_xy) = &mv_from_xy.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mv_xy.row_titles, vec!["SampleA", "SampleA"]);

    // 3. MV with non-numeric/excluded Replicate marks dropped_cells + schema_lossy + warning for Grouped & XY pivots
    let mut mv_bad_rep = MultipleVariablesTable::new(4, 2).unwrap();
    mv_bad_rep.variables[0].title = String::from("Time");
    mv_bad_rep.variables[0].cells = vec![Cell::number(1.0), Cell::number(1.0)];
    mv_bad_rep.variables[1].title = String::from("Group");
    mv_bad_rep.variables[1].cells = vec![Cell::text("G1"), Cell::text("G1")];
    mv_bad_rep.variables[2].title = String::from("Replicate");
    mv_bad_rep.variables[2].cells = vec![Cell::text("repA"), Cell::number(2.0)];
    mv_bad_rep.variables[3].title = String::from("Value");
    mv_bad_rep.variables[3].cells = vec![Cell::number(10.0), Cell::number(20.0)];
    for v in &mut mv_bad_rep.variables {
        v.refresh_inferred_role();
    }
    mv_bad_rep.variables[2].lock_role(VariableRole::Continuous);
    let bad_rep_sheet = DataSheet::new("mv-rep-bad", "Bad Rep", TableArchetype::MultipleVariables(mv_bad_rep.clone())).unwrap();
    let (_, xy_rep_prev) = convert_datasheet_archetype(&bad_rep_sheet, TargetArchetypeKind::Xy, true).unwrap();
    assert!(xy_rep_prev.lossy && xy_rep_prev.dropped_cells >= 1);
    assert!(
        xy_rep_prev.warnings.iter().any(|w| w.contains("Non-numeric or excluded replicate")),
        "expected replicate drop warning: {:?}",
        xy_rep_prev.warnings
    );

    mv_bad_rep.variables[0].lock_role(VariableRole::Categorical);
    mv_bad_rep.variables[2].cells[0] = Cell::excluded_number(1.0);
    let bad_rep_grp = DataSheet::new("mv-rep-bad-g", "Bad Rep G", TableArchetype::MultipleVariables(mv_bad_rep)).unwrap();
    let (_, grp_rep_prev) = convert_datasheet_archetype(&bad_rep_grp, TargetArchetypeKind::Grouped, true).unwrap();
    assert!(grp_rep_prev.lossy && grp_rep_prev.dropped_cells >= 1);
    assert!(
        grp_rep_prev.warnings.iter().any(|w| w.contains("Non-numeric or excluded replicate")),
        "expected replicate drop warning: {:?}",
        grp_rep_prev.warnings
    );

    // 4. Empty MV summary Grouped pivot returns Err(InvalidInput) instead of 0-dataset GroupedTable
    let mut mv_empty_sum = MultipleVariablesTable::new(5, 1).unwrap();
    mv_empty_sum.variables[0].title = String::from("RowF");
    mv_empty_sum.variables[0].lock_role(VariableRole::Categorical);
    mv_empty_sum.variables[1].title = String::from("ColF");
    mv_empty_sum.variables[1].lock_role(VariableRole::Categorical);
    mv_empty_sum.variables[2].title = String::from("Mean");
    mv_empty_sum.variables[3].title = String::from("SD");
    mv_empty_sum.variables[4].title = String::from("N");
    let empty_sum_sheet = DataSheet::new("mv-esum", "Empty Sum", TableArchetype::MultipleVariables(mv_empty_sum)).unwrap();
    assert!(matches!(convert_datasheet_archetype(&empty_sum_sheet, TargetArchetypeKind::Grouped, true), Err(TransformError::InvalidInput { .. })));

    // 5. Xy(-0.0) -> Grouped and Contingency normalize "-0" -> "0"
    let mut neg_zero_xy = XyTable::new(SubcolumnMode::Single, 2, 1).unwrap();
    neg_zero_xy.set_x_cell(0, Cell::number(-0.0)).unwrap();
    neg_zero_xy.data_sets[0].cells[0][0] = Cell::number(4.0);
    neg_zero_xy.data_sets[1].cells[0][0] = Cell::number(5.0);
    let nz_sheet = DataSheet::new("xy-nz", "NegZero", TableArchetype::Xy(neg_zero_xy)).unwrap();
    let (grp_nz, _) = convert_datasheet_archetype(&nz_sheet, TargetArchetypeKind::Grouped, true).unwrap();
    let TableArchetype::Grouped(gnz) = &grp_nz.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(gnz.row_titles, vec!["0"]);
    let (cont_nz, _) = convert_datasheet_archetype(&nz_sheet, TargetArchetypeKind::Contingency, true).unwrap();
    let TableArchetype::Contingency(cnz) = &cont_nz.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(cnz.row_titles, vec!["0"]);

    // 6. Column -> MultipleVariables rejects exceeding MAX_IMPORT_CELLS before allocation
    let mut wide_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1_000, 11_000).unwrap();
    wide_col.data_sets[0].cells[0][0] = Cell::number(1.0);
    let wide_col_sheet = DataSheet::new("col-oom", "Too Big", TableArchetype::Column(wide_col)).unwrap();
    const { assert!(MAX_IMPORT_CELLS < 1_000 * 11_000) };
    assert!(convert_datasheet_archetype(&wide_col_sheet, TargetArchetypeKind::MultipleVariables, true).is_err());
}

// ============================================================================
// FA-SEC03-11: Import unstack width == 2, -0.0 group, subcol bounds,
//              and InternalData budget check
// ============================================================================

#[test]
fn fa_sec03_11_import_unstack_width_subcol_anchor_and_internal_budget() {
    // 1. unstack_by_group rejects 3-column payload instead of dropping col 2
    let mut dst_col =
        DataSheet::new("dst-un", "Unstack", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap())).unwrap();
    let spec_unstack = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec::default(),
    };
    let err_3col = apply_import_link_pipeline(
        &mut dst_col,
        &spec_unstack,
        curvebook_transform::ImportSourcePayload::DelimitedText { text: "A,1,99\nB,2,88\n", delimiter: ',', decimal_separator: '.' },
        &Default::default(),
        None,
    )
    .unwrap_err();
    assert!(matches!(err_3col, TransformError::InvalidSpec { .. }));

    // 2. anchor.subcol >= subs_per_col rejects cleanly instead of silently ignoring extra columns
    let spec_bad_sub = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { anchor: CellCoord::new(0, 0, 1), ..ImportPlacementSpec::default() },
    };
    let err_sub = apply_import_link_pipeline(
        &mut dst_col,
        &spec_bad_sub,
        curvebook_transform::ImportSourcePayload::DelimitedText { text: "10\n20\n", delimiter: ',', decimal_separator: '.' },
        &Default::default(),
        None,
    )
    .unwrap_err();
    assert!(matches!(err_sub, TransformError::InvalidSpec { .. }));

    // 3. InternalData extract enforces MAX_IMPORT_CELLS budget before cloning
    let huge_src =
        DataSheet::new("huge-src", "Huge", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1_000, 11_000).unwrap()))
            .unwrap();
    let internal_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("huge-src"),
            kind: InternalLinkKind::DataSheet,
            tab_id: None,
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(10_999, 999, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec::default(),
    };
    let err_budget = apply_import_link_pipeline(
        &mut dst_col,
        &internal_spec,
        curvebook_transform::ImportSourcePayload::InternalData(&huge_src),
        &Default::default(),
        None,
    )
    .unwrap_err();
    assert!(matches!(err_budget, TransformError::InvalidSpec { .. }));
}

// ============================================================================
// FA-SEC03-12: MathTransformOp::Antilog rejects base == 1.0 or base <= 0.0
// ============================================================================

#[test]
fn fa_sec03_12_math_antilog_rejects_degenerate_base_one_and_non_positive() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy.data_sets[0].cells[0][0] = Cell::number(3.0);
    xy.data_sets[0].cells[1][0] = Cell::number(4.0);
    let sheet = DataSheet::new("xy-anti", "Antilog", TableArchetype::Xy(xy)).unwrap();
    let ctx = TransformContext::default();

    for bad_base in [1.0, 0.0, -2.0, f64::NAN] {
        let op = MathTransformOp::Antilog { base: bad_base };
        assert!(
            matches!(
                execute_math_transform(&sheet, None, Some(&op), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx),
                Err(TransformError::InvalidSpec { .. })
            ),
            "base={bad_base} must be rejected"
        );
    }

    let ok = execute_math_op(&sheet, &MathTransformOp::Antilog { base: 2.0 }, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    let TableArchetype::Xy(ok_xy) = &ok.data_sheet.archetype else {
        panic!("expected XY");
    };
    assert_approx(ok_xy.data_sets[0].cells[0][0].active_number().unwrap(), 8.0, 1e-12);
}

// ============================================================================
// FA-SEC03-13: countif(col, "true") + eval_scalar_const_arg rejects column/row/window
// ============================================================================

#[test]
fn fa_sec03_13_countif_string_true_and_scalar_const_arg_guards() {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].title = String::from("FlagText");
    mv.variables[1].title = String::from("Y");
    mv.variables[2].title = String::from("Step");
    for (r, (flag, y, step)) in [("true", 10.0, 1.0), ("false", 20.0, 2.0), ("true", 30.0, 1.0), ("yes", 40.0, 1.0)].into_iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(flag);
        mv.variables[1].cells[r] = Cell::number(y);
        mv.variables[2].cells[r] = Cell::number(step);
    }
    let sheet = DataSheet::new("mv-f13", "Formula", TableArchetype::MultipleVariables(mv)).unwrap();

    // countif with text literal "true" counts rows whose text equals "true"
    let cnt = evaluate_formula_for_sheet("countif(FlagText, \"true\")", &sheet, None).unwrap();
    assert_approx(cnt[0].as_finite_f64().unwrap(), 2.0, 1e-12);

    // lag / quantile / rolling_mean scalar parameters must reject column/row/window refs
    for bad in ["lag(Y, Step)", "lag(Y, [Step])", "lag(Y, $row)", "quantile(Y, row_number())", "rolling_mean(Y, lag(Y, 1))"] {
        assert!(
            matches!(evaluate_formula_for_sheet(bad, &sheet, None), Err(TransformError::InvalidSpec { .. })),
            "`{bad}` must fail with InvalidSpec"
        );
    }

    // Info("W") inside window frame scalar parameter still works cleanly, even on a 0-row table
    let mut info = InfoSheet::new_preseeded("info-f13", "F13").unwrap();
    info.set_constant("W", ConstantValue::Number(1.0), None).unwrap();
    let info_res =
        evaluate_formula_for_sheet("mean(Y) OVER (ORDER BY $index ROWS BETWEEN Info(\"W\") PRECEDING AND CURRENT ROW)", &sheet, Some(&info)).unwrap();
    assert_approx(info_res[1].as_finite_f64().unwrap(), 15.0, 1e-12);

    let mut empty_mv = MultipleVariablesTable::new(1, 0).unwrap();
    empty_mv.variables[0].title = String::from("Y");
    let empty_sheet = DataSheet::new("mv-f13-0", "Empty Formula", TableArchetype::MultipleVariables(empty_mv)).unwrap();
    let empty_res =
        evaluate_formula_for_sheet("mean(Y) OVER (ORDER BY $index ROWS BETWEEN Info(\"W\") PRECEDING AND CURRENT ROW)", &empty_sheet, Some(&info))
            .unwrap();
    assert!(empty_res.is_empty());
}

// ============================================================================
// FA-SEC03-14: RandomSample omits lossy `"seed"` scalar when seed > 2^53
// ============================================================================

#[test]
fn fa_sec03_14_randomize_huge_seed_omits_lossy_scalar() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 5).unwrap();
    for r in 0..5 {
        col.data_sets[0].cells[r][0] = Cell::number(r as f64);
    }
    let sheet = DataSheet::new("rs-src", "Source", TableArchetype::Column(col)).unwrap();
    let ctx = TransformContext::default();

    let huge_seed = (1_u64 << 53) + 1;
    let spec_huge = AnalysisSpec::new(
        "rs-huge",
        "rs-src",
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: huge_seed,
            sample_size: 3,
            stratify_column: None,
            proportional_allocation: true,
        }),
    )
    .unwrap();
    let res_huge = execute_analysis_transform(&sheet, &spec_huge, &ctx).unwrap();
    assert_eq!(res_huge.lookup_scalar("seed", None), None, "seed > 2^53 must not emit precision-corrupted f64 scalar");

    let spec_small = AnalysisSpec::new(
        "rs-small",
        "rs-src",
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
            seed: 1234,
            sample_size: 3,
            stratify_column: None,
            proportional_allocation: true,
        }),
    )
    .unwrap();
    let res_small = execute_analysis_transform(&sheet, &spec_small, &ctx).unwrap();
    assert_eq!(res_small.lookup_scalar("seed", None), Some(1234.0));
}

// ============================================================================
// FA-SEC03-15: Exponential overflow returns EvalError, and ResultTab import
//              renders ResultCell::ConfidenceInterval text instead of blank
// ============================================================================

#[test]
fn fa_sec03_15_exponential_overflow_and_ci_import_text() {
    // 1. Exponential overflow with extreme tiny rate fails with EvalError instead of emitting Infinity.
    let cfg = DistributionSimulationConfig {
        sheet_id: String::from("exp-inf"),
        title: String::from("Exp Inf"),
        n_rows: 16,
        seed: 42,
        columns: vec![SimulatedColumnSpec { name: String::from("E"), distribution: SimulatedDistribution::Exponential { rate: 1e-320 } }],
        use_multiple_variables: false,
    };
    assert!(matches!(simulate_distribution_columns(&cfg), Err(TransformError::EvalError { .. })));

    // 2. Import from ResultTab converts ResultCell::ConfidenceInterval into `[lo, hi]` text cell.
    let analysis =
        AnalysisSpec::new("res-ci", "src", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap();
    let tab = ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("95% CI")],
        row_labels: vec![String::from("Param"), String::from("Wide")],
        rows: vec![
            vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.5), upper: CiBound::Finite(4.25) }],
            vec![ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) }],
        ],
        footnotes: Vec::new(),
    };
    let mut res_sheet = ResultSheet::new(analysis).unwrap();
    res_sheet.tabs.push(tab);

    let mut dst = DataSheet::new("ci-dst", "CI Dst", TableArchetype::MultipleVariables(MultipleVariablesTable::new(1, 2).unwrap())).unwrap();
    let link_spec = ImportLinkSpec {
        source: LinkSource::InternalSheet {
            sheet_id: SheetId::new("res-ci"),
            kind: InternalLinkKind::TerminalResultSheet,
            tab_id: Some(String::from("summary")),
            range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec::default(),
    };
    apply_import_link_pipeline(&mut dst, &link_spec, curvebook_transform::ImportSourcePayload::InternalResult(&res_sheet), &Default::default(), None)
        .unwrap();
    let TableArchetype::MultipleVariables(dst_mv) = &dst.archetype else {
        panic!("expected MV");
    };
    assert_eq!(dst_mv.variables[0].cells[0].active_text(), Some("[1.5, 4.25]"));
    assert_eq!(dst_mv.variables[0].cells[1].active_text(), Some("[-inf, 2]"));
}
