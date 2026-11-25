//! Targeted regressions covering all 16 confirmed `SEC-03` audit findings (`FA-SEC03-01`..`FA-SEC03-16`).

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CellCoord, CiBound, ColumnDesignHint, ColumnTable, DocError, ErrorPropagationMode, HookSource, ImportFilterSpec,
    ImportLinkSpec, ImportPlacementSpec, InternalLinkKind, LinkSource, MultipleVariablesTable, NormalizeAnchor, PlacementMode, ResultCell,
    ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SheetCellRange, SheetId, SimulationNoise, SubcolumnMode, TableArchetype,
    TableKind, TransformAnalysisSpec, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::import::MAX_IMPORT_CELLS;
use curvebook_transform::{
    AucOptions, DistributionSimulationConfig, ImportSourcePayload, MathTransformOp, PlateGeometry, PlateMapTemplate, PlateWellRole, PruneMethod,
    SimulatedColumnSpec, SimulatedDistribution, SmoothMethod, TargetArchetypeKind, TidyColumnMapping, TransformContext, TransformError,
    apply_import_link_pipeline, compute_auc, convert_datasheet_archetype, evaluate_formula_for_sheet, execute_analysis_transform, execute_math_op,
    execute_math_transform, execute_normalize, execute_prune, execute_select, execute_transpose, pivot_tidy_long_to_datasheet,
    simulate_distribution_columns, simulate_xy_sheet, smooth_xy,
};

use crate::common::{
    ExpectVal, add_excl, as_col, as_contingency, as_grouped, as_mv, as_xy, assert_approx, assert_formula_rows, clipboard_spec, col_empty, col_nums,
    col_sheet, contingency_sheet, excl, grouped_sheet, import_csv, info_with_numbers, mv_empty, mv_sheet, mv_table, num, result_sheet_with_tab,
    sheet, str_rows, str_vec, txt, xy_3sub_clean, xy_single,
};

#[test]
fn fa_sec03_01_select_column_to_xy_and_survival_shifts_notes_and_warns() {
    let mut s = col_sheet("col-sel", &[("TimeX", &[excl(1.0), num(2.0)]), ("SignalY", &[num(10.0), excl(20.0)])]);
    add_excl(&mut s, CellCoord::new(0, 0, 0), "qc0", "bad axis point");
    add_excl(&mut s, CellCoord::new(1, 1, 0), "qc1", "bad response");

    let ctx = TransformContext::default();
    for target in [TableKind::Xy, TableKind::Survival] {
        let out = execute_select(&s, &[], target, &ctx).unwrap();
        assert_eq!(out.data_sheet.exclusion_notes.len(), 1, "col-0 axis note must not misalign for {target:?}");
        assert_eq!(
            (out.data_sheet.exclusion_notes[0].coord, out.data_sheet.exclusion_notes[0].reason_tag.as_str()),
            (CellCoord::new(1, 0, 0), "qc1")
        );
        assert!(out.warnings.iter().any(|w| w.contains("could not be retained as cell exclusion notes")), "{target:?}: {:?}", out.warnings);
    }
}

#[test]
fn fa_sec03_02_transpose_guards_notes_dates_and_warnings() {
    let ctx = TransformContext::default();

    // 1. 0-row table rejected cleanly.
    assert!(matches!(execute_transpose(&col_empty("empty-col", 2, 0), true, &ctx), Err(TransformError::InvalidInput { .. })));

    // 2. Column transpose swaps exclusion note (row, ds) -> (ds, row).
    let mut col_s = col_empty("tr-notes", 3, 2);
    crate::common::as_col_mut(&mut col_s).data_sets[2].cells[1][0] = excl(99.0);
    add_excl(&mut col_s, CellCoord::new(1, 2, 0), "qc", "col2 row1");
    let tr_col = execute_transpose(&col_s, true, &ctx).unwrap();
    assert_eq!(tr_col.data_sheet.exclusion_notes.len(), 1);
    assert_eq!(tr_col.data_sheet.exclusion_notes[0].coord, CellCoord::new(2, 1, 0));

    // 3. Xy DateOrElapsedTime preserves text labels & emits X-discard warning when !swap_titles.
    let mut xy_dt =
        XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("day") }, SubcolumnMode::Single, 1, 2).unwrap();
    xy_dt.x_cells = vec![vec![txt("2026-01-01")], vec![txt("2026-01-02")]];
    xy_dt.data_sets[0].cells = vec![vec![num(5.0)], vec![num(6.0)]];
    let dt_sheet = sheet("xy-dt", TableArchetype::Xy(xy_dt));

    let tr_swap = execute_transpose(&dt_sheet, true, &ctx).unwrap();
    let c_swap = as_col(&tr_swap.data_sheet);
    assert_eq!(
        (c_swap.data_sets[0].title.as_str(), c_swap.data_sets[1].title.as_str(), c_swap.row_titles.as_slice()),
        ("2026-01-01", "2026-01-02", [String::from("A")].as_slice())
    );
    let tr_noswap = execute_transpose(&dt_sheet, false, &ctx).unwrap();
    assert!(tr_noswap.warnings.iter().any(|w| w.contains("discarded source X values")), "{:?}", tr_noswap.warnings);

    // 4. X error subcolumns + -0.0 X coordinate normalize "-0" -> "0" and warn of omitted X error subcolumns.
    let mut xy_err = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::Single, 1, 1).unwrap();
    xy_err.x_cells[0] = vec![num(-0.0), num(0.5), num(3.0)];
    xy_err.data_sets[0].cells[0][0] = num(12.0);
    let tr_err = execute_transpose(&sheet("xy-xerr", TableArchetype::Xy(xy_err)), true, &ctx).unwrap();
    assert_eq!(as_col(&tr_err.data_sheet).data_sets[0].title, "0");
    assert!(tr_err.warnings.iter().any(|w| w.contains("X error subcolumns")), "{:?}", tr_err.warnings);
}

#[test]
fn fa_sec03_03_multi_info_sheet_dirty_scalar_and_import_filter_info() {
    let mut info_a = info_with_numbers("info-a", &[("K", 10.0)]);
    let info_b = info_with_numbers("info-b", &[("K", 50.0)]);
    let sheets = [info_a.clone(), info_b];
    let ctx = TransformContext { info_sheet: Some(&info_a), info_sheets: &sheets, ..TransformContext::default() };
    let s = xy_single("s-info", &[1.0, 2.0], &[&[25.0, 50.0]]);

    // Explicit hook targets info-b (K=50) while info_sheet is info-a (K=10).
    let hook_b = NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("info-b")), constant_name: String::from("K") });
    let norm_b = execute_normalize(&s, &NormalizeAnchor::FixedValue(0.0), &hook_b, true, true, &ctx).unwrap();
    let n_xy = as_xy(&norm_b.data_sheet);
    assert_approx(n_xy.data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);
    assert_approx(n_xy.data_sets[0].cells[1][0].active_number().unwrap(), 100.0, 1e-12);

    // Unknown explicit info_sheet_id rejects cleanly instead of silently reading info_a.
    let hook_missing =
        NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id: Some(SheetId::new("missing-info")), constant_name: String::from("K") });
    assert!(matches!(
        execute_normalize(&s, &NormalizeAnchor::FixedValue(0.0), &hook_missing, true, true, &ctx),
        Err(TransformError::MissingHook { .. })
    ));

    // Dirty upstream ResultSheet scalar hook rejects cleanly.
    let spec =
        AnalysisSpec::new("up", "s-info", AnalysisKind::Transform(TransformAnalysisSpec::Transpose { swap_row_and_column_titles: false })).unwrap();
    let mut dirty_res = ResultSheet::new(spec).unwrap();
    dirty_res.status = ResultStatus::Dirty;
    dirty_res.scalars.push(ResultScalarEntry { key: String::from("Ref"), value: 100.0, units: None, data_set_index: None });
    let dirty_ctx = TransformContext { result_sheets: std::slice::from_ref(&dirty_res), ..TransformContext::default() };
    let hook_dirty = NormalizeAnchor::Hooked(HookSource::ResultScalar {
        result_sheet_id: SheetId::new("up"),
        parameter_key: String::from("Ref"),
        data_set_index: None,
    });
    let dirty_err = execute_normalize(&s, &NormalizeAnchor::FixedValue(0.0), &hook_dirty, true, true, &dirty_ctx).unwrap_err();
    assert!(matches!(dirty_err, TransformError::MissingHook { .. }) && dirty_err.to_string().contains("dirty"), "{dirty_err:?}");

    // Import pipeline forwards InfoSheet into filter exclude_row_predicate.
    let mut dst = col_empty("imp-f", 1, 1);
    let link = clipboard_spec(
        PlacementMode::Overwrite,
        CellCoord::new(0, 0, 0),
        ImportFilterSpec { exclude_row_predicate: Some(String::from("col(0) > Info(\"K\")")), ..ImportFilterSpec::default() },
    );
    apply_import_link_pipeline(
        &mut dst,
        &link,
        ImportSourcePayload::DelimitedText { text: "5\n15\n", delimiter: ',', decimal_separator: '.' },
        &Default::default(),
        Some(&mut info_a),
    )
    .unwrap();
    let dst_col = as_col(&dst);
    assert_eq!(dst_col.data_sets[0].cells[0][0].active_number(), Some(5.0));
    assert!(dst_col.data_sets[0].cells[1][0].excluded);
}

#[test]
fn fa_sec03_04_contingency_validation_after_math_norm_and_prune() {
    let s = contingency_sheet("cont", &[&[3.0, 2.0], &[4.0, 5.0]]);
    let ctx = TransformContext::default();
    let results = [
        ("negative math", execute_math_op(&s, &MathTransformOp::AddConstant(-10.0), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx)),
        ("fractional norm", execute_normalize(&s, &NormalizeAnchor::FixedValue(0.0), &NormalizeAnchor::FixedValue(10.0), false, false, &ctx)),
        ("odd-sum AverageBins", execute_prune(&s, PruneMethod::AverageBins(2), &ctx)),
    ];
    for (label, res) in results {
        assert!(matches!(res, Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))), "{label}: {res:?}");
    }
}

#[test]
fn fa_sec03_05_prune_log_spaced_sorts_unsorted_and_descending_x() {
    let ctx = TransformContext::default();
    for xs in [[1000.0, 100.0, 10.0, 1.0], [100.0, 1.0, 1000.0, 10.0]] {
        let ys: Vec<f64> = xs.iter().map(|&x| x * 2.0).collect();
        let out_xy =
            as_xy(&execute_prune(&xy_single("log-xy", &xs, &[&ys]), PruneMethod::LogarithmicSpacing { target_points: 2 }, &ctx).unwrap().data_sheet)
                .clone();
        let retained: Vec<f64> = (0..out_xy.row_count()).map(|r| out_xy.x_value_at(r).unwrap()).collect();
        assert_eq!(retained.len(), 2, "must pick endpoints: {retained:?}");
        assert!(retained.contains(&1.0) && retained.contains(&1000.0), "got {retained:?}");
    }
}

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
    let smoothed =
        smooth_xy(&[(5.0, 10.0), (5.0, 11.0), (5.0, 10.5), (5.0, 100.0)], SmoothMethod::Lowess { span: 1.0, robustness_iters: 3 }).unwrap();
    let unweighted_mean = (10.0 + 11.0 + 10.5 + 100.0) / 4.0;
    assert!(smoothed[0].1 < 15.0 && smoothed[0].1 < unweighted_mean - 15.0, "tied LOWESS ignored robustness weights: {smoothed:?}");
}

#[test]
fn fa_sec03_07_normalize_wide_span_simulate_wide_bounds_and_cv_warnings() {
    let ctx = TransformContext::default();

    // 1. Wide normalization span (-1e308 to +1e308) uses half-scaling without overflowing span.
    let norm_wide = execute_normalize(
        &xy_single("wide-n", &[1.0], &[&[0.0]]),
        &NormalizeAnchor::FixedValue(-1e308),
        &NormalizeAnchor::FixedValue(1e308),
        true,
        true,
        &ctx,
    )
    .unwrap();
    assert_approx(as_xy(&norm_wide.data_sheet).data_sets[0].cells[0][0].active_number().unwrap(), 50.0, 1e-12);

    // 2. MeanCvN zero normalized center blanks CV and appends warning.
    let cv_sheet = xy_3sub_clean("cv-n", SubcolumnMode::MeanCvN, &[1.0], &[("A", &[[Some(10.0), Some(15.0), Some(4.0)]])]);
    let cv_norm = execute_normalize(&cv_sheet, &NormalizeAnchor::FixedValue(10.0), &NormalizeAnchor::FixedValue(20.0), true, true, &ctx).unwrap();
    assert!(as_xy(&cv_norm.data_sheet).data_sets[0].cells[0][1].is_empty());
    assert!(cv_norm.warnings.iter().any(|w| w.contains("CV%") && w.contains("blanked")), "{:?}", cv_norm.warnings);

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
    let sim_xy = as_xy(&sim);
    assert_eq!((sim_xy.x_value_at(0), sim_xy.x_value_at(2)), (Some(-1e308), Some(1e308)));
    assert_approx(sim_xy.x_value_at(1).unwrap(), 0.0, 1e-12);
}

#[test]
fn fa_sec03_08_json_parser_linear_on_large_non_ascii_template() {
    let label = "µg_αβγ_".repeat(16);
    let wells =
        (0..384).map(|i| PlateWellRole::Treatment { group_name: format!("{label}{i}"), dilution_x: (i + 1) as f64, replicate: i % 4 }).collect();
    let tpl = PlateMapTemplate::from_wells("NonASCII Plate", PlateGeometry::Wells384, "Conc (µM)", wells).unwrap();
    let json = tpl.to_cbtemplate_json().unwrap();
    assert!(json.len() > 60_000);
    let parsed = PlateMapTemplate::from_cbtemplate_json(&json).unwrap();
    assert_eq!((parsed.wells.len(), &parsed), (384, &tpl));
}

#[test]
fn fa_sec03_09_tidy_exclusions_decimal_comma_replicates_and_collisions() {
    let headers = str_vec(&["X", "Group", "Rep", "Val"]);
    let rows = str_rows(&[&["*10,0*", "Drug", "1,5", "*42,5*"], &["10,0", "Drug", "1,5", "99,0"], &["20,0", "Drug", "2,0", "50,0"]]);
    let map = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: Some(2), value_col: 3, excluded_col: None };

    let mv_sheet = pivot_tidy_long_to_datasheet("MV Tidy", &headers, &rows, &map, TargetArchetypeKind::MultipleVariables, ',').unwrap();
    let mv = as_mv(&mv_sheet);
    assert!(mv.variables[0].cells[0].excluded && mv.variables[3].cells[0].excluded);
    assert_approx(mv.variables[0].cells[0].value.finite_f64().unwrap(), 10.0, 1e-12);
    assert_approx(mv.variables[3].cells[0].value.finite_f64().unwrap(), 42.5, 1e-12);
    assert_approx(mv.variables[2].cells[0].active_number().unwrap(), 1.5, 1e-12);
    assert!(mv_sheet.exclusion_notes.iter().any(|n| n.coord == CellCoord::new(0, 0, 0)), "{:?}", mv_sheet.exclusion_notes);

    for target in [TargetArchetypeKind::Xy, TargetArchetypeKind::Grouped] {
        let out = pivot_tidy_long_to_datasheet("Collide Tidy", &headers, &rows, &map, target, ',').unwrap();
        assert!(out.floating_notes.iter().any(|n| n.body_markdown.contains("collision")), "{target:?}: {:?}", out.floating_notes);
    }
}

#[test]
fn fa_sec03_10_and_16_archetype_convert_budgets_row_titles_dropped_reps_and_empty_summary() {
    // 1. Grouped -> MV preserves row_titles without breaking non-lossy roundtrip.
    let grp_s = grouped_sheet(
        "grp-rt",
        SubcolumnMode::RawReplicates(2),
        &["Control", "Treated"],
        &[("Batch1", &[&[num(1.0), num(2.0)], &[num(3.0), num(4.0)]])],
    );
    let (mv_from_grp, grp_prev) = convert_datasheet_archetype(&grp_s, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!grp_prev.lossy);
    assert_eq!(as_mv(&mv_from_grp).row_titles, vec!["Control", "Control", "Treated", "Treated"]);

    // 2. XY(RawReplicates) -> MV preserves xy.row_titles.
    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap();
    xy_rep.set_x_cell(0, num(5.0)).unwrap();
    xy_rep.row_titles = vec![String::from("SampleA")];
    xy_rep.row_titles_expanded = true;
    xy_rep.data_sets[0].cells[0] = vec![num(10.0), num(20.0)];
    let (mv_from_xy, _) =
        convert_datasheet_archetype(&sheet("xy-rt", TableArchetype::Xy(xy_rep)), TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert_eq!(as_mv(&mv_from_xy).row_titles, vec!["SampleA", "SampleA"]);

    // 3. MV with non-numeric/excluded Replicate marks dropped_cells + lossy + warning for XY & Grouped pivots.
    let mut mv_bad = mv_table(&[
        ("Time", &[num(1.0), num(1.0)]),
        ("Group", &[txt("G1"), txt("G1")]),
        ("Replicate", &[txt("repA"), num(2.0)]),
        ("Value", &[num(10.0), num(20.0)]),
    ]);
    for v in &mut mv_bad.variables {
        v.refresh_inferred_role();
    }
    mv_bad.variables[2].lock_role(VariableRole::Continuous);
    let (_, xy_prev) =
        convert_datasheet_archetype(&sheet("mv-rep-bad", TableArchetype::MultipleVariables(mv_bad.clone())), TargetArchetypeKind::Xy, true).unwrap();
    assert!(xy_prev.lossy && xy_prev.dropped_cells >= 1 && xy_prev.warnings.iter().any(|w| w.contains("Non-numeric or excluded replicate")));

    mv_bad.variables[0].lock_role(VariableRole::Categorical);
    mv_bad.variables[2].cells[0] = excl(1.0);
    let (_, grp_rep_prev) =
        convert_datasheet_archetype(&sheet("mv-rep-bad-g", TableArchetype::MultipleVariables(mv_bad)), TargetArchetypeKind::Grouped, true).unwrap();
    assert!(
        grp_rep_prev.lossy
            && grp_rep_prev.dropped_cells >= 1
            && grp_rep_prev.warnings.iter().any(|w| w.contains("Non-numeric or excluded replicate"))
    );

    // 4. Empty MV summary Grouped pivot returns Err(InvalidInput).
    let mut mv_esum = MultipleVariablesTable::new(5, 1).unwrap();
    for (i, t) in ["RowF", "ColF", "Mean", "SD", "N"].into_iter().enumerate() {
        mv_esum.variables[i].title = String::from(t);
    }
    mv_esum.variables[0].lock_role(VariableRole::Categorical);
    mv_esum.variables[1].lock_role(VariableRole::Categorical);
    assert!(matches!(
        convert_datasheet_archetype(&sheet("mv-esum", TableArchetype::MultipleVariables(mv_esum)), TargetArchetypeKind::Grouped, true),
        Err(TransformError::InvalidInput { .. })
    ));

    // 5. Xy(-0.0) -> Grouped and Contingency normalize "-0" -> "0".
    let nz_sheet = xy_single("xy-nz", &[-0.0], &[&[4.0], &[5.0]]);
    assert_eq!(as_grouped(&convert_datasheet_archetype(&nz_sheet, TargetArchetypeKind::Grouped, true).unwrap().0).row_titles, vec!["0"]);
    assert_eq!(as_contingency(&convert_datasheet_archetype(&nz_sheet, TargetArchetypeKind::Contingency, true).unwrap().0).row_titles, vec!["0"]);

    // 6. Column -> MultipleVariables rejects exceeding MAX_IMPORT_CELLS before allocation.
    let mut wide_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1_000, 11_000).unwrap();
    wide_col.data_sets[0].cells[0][0] = num(1.0);
    const { assert!(MAX_IMPORT_CELLS < 1_000 * 11_000) };
    assert!(convert_datasheet_archetype(&sheet("col-oom", TableArchetype::Column(wide_col)), TargetArchetypeKind::MultipleVariables, true).is_err());
}

#[test]
fn fa_sec03_11_import_unstack_width_subcol_anchor_and_internal_budget() {
    let mut dst_col = col_empty("dst-un", 1, 1);

    // 1. unstack_by_group rejects 3-column payload instead of dropping col 2.
    let spec_unstack = clipboard_spec(
        PlacementMode::Overwrite,
        CellCoord::new(0, 0, 0),
        ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() },
    );
    assert!(matches!(import_csv(&mut dst_col, "A,1,99\nB,2,88\n", &spec_unstack), Err(TransformError::InvalidSpec { .. })));

    // 2. anchor.subcol >= subs_per_col rejects cleanly.
    let spec_bad_sub = clipboard_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 1), ImportFilterSpec::default());
    assert!(matches!(import_csv(&mut dst_col, "10\n20\n", &spec_bad_sub), Err(TransformError::InvalidSpec { .. })));

    // 3. InternalData extract enforces MAX_IMPORT_CELLS budget before cloning.
    let huge_src = col_empty("huge-src", 1_000, 11_000);
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
    let err_budget =
        apply_import_link_pipeline(&mut dst_col, &internal_spec, ImportSourcePayload::InternalData(&huge_src), &Default::default(), None)
            .unwrap_err();
    assert!(matches!(err_budget, TransformError::InvalidSpec { .. }));
}

#[test]
fn fa_sec03_12_math_antilog_rejects_degenerate_base_one_and_non_positive() {
    let s = xy_single("xy-anti", &[1.0, 2.0], &[&[3.0, 4.0]]);
    let ctx = TransformContext::default();
    for bad_base in [1.0, 0.0, -2.0, f64::NAN] {
        let op = MathTransformOp::Antilog { base: bad_base };
        assert!(
            matches!(
                execute_math_transform(&s, None, Some(&op), ErrorPropagationMode::FirstOrderDeltaMethod, &ctx),
                Err(TransformError::InvalidSpec { .. })
            ),
            "base={bad_base}"
        );
    }
    let ok = execute_math_op(&s, &MathTransformOp::Antilog { base: 2.0 }, ErrorPropagationMode::FirstOrderDeltaMethod, &ctx).unwrap();
    assert_approx(as_xy(&ok.data_sheet).data_sets[0].cells[0][0].active_number().unwrap(), 8.0, 1e-12);
}

#[test]
fn fa_sec03_13_countif_string_true_and_scalar_const_arg_guards() {
    let s = mv_sheet(
        "mv-f13",
        &[
            ("FlagText", &[txt("true"), txt("false"), txt("true"), txt("yes")]),
            ("Y", &[num(10.0), num(20.0), num(30.0), num(40.0)]),
            ("Step", &[num(1.0), num(2.0), num(1.0), num(1.0)]),
        ],
    );
    assert_formula_rows(&s, None, "countif(FlagText, \"true\")", &[(0, ExpectVal::Num(2.0))], 1e-12);

    for bad in ["lag(Y, Step)", "lag(Y, [Step])", "lag(Y, $row)", "quantile(Y, row_number())", "rolling_mean(Y, lag(Y, 1))"] {
        assert!(matches!(evaluate_formula_for_sheet(bad, &s, None), Err(TransformError::InvalidSpec { .. })), "`{bad}` must fail with InvalidSpec");
    }

    let info = info_with_numbers("info-f13", &[("W", 1.0)]);
    let win_expr = "mean(Y) OVER (ORDER BY $index ROWS BETWEEN Info(\"W\") PRECEDING AND CURRENT ROW)";
    assert_formula_rows(&s, Some(&info), win_expr, &[(1, ExpectVal::Num(15.0))], 1e-12);

    let mut empty_mv = MultipleVariablesTable::new(1, 0).unwrap();
    empty_mv.variables[0].title = String::from("Y");
    assert!(evaluate_formula_for_sheet(win_expr, &sheet("mv-f13-0", TableArchetype::MultipleVariables(empty_mv)), Some(&info)).unwrap().is_empty());
}

#[test]
fn fa_sec03_14_randomize_huge_seed_omits_lossy_scalar() {
    let s = col_nums("rs-src", &[("A", &[0.0, 1.0, 2.0, 3.0, 4.0])]);
    let ctx = TransformContext::default();
    for (id, seed, expected_scalar) in [("rs-huge", (1_u64 << 53) + 1, None), ("rs-small", 1234_u64, Some(1234.0))] {
        let spec = AnalysisSpec::new(
            id,
            "rs-src",
            AnalysisKind::Transform(TransformAnalysisSpec::RandomSample {
                seed,
                sample_size: 3,
                stratify_column: None,
                proportional_allocation: true,
            }),
        )
        .unwrap();
        let res = execute_analysis_transform(&s, &spec, &ctx).unwrap();
        assert_eq!(res.lookup_scalar("seed", None), expected_scalar, "seed={seed}");
    }
}

#[test]
fn fa_sec03_15_exponential_overflow_and_ci_import_text() {
    let cfg = DistributionSimulationConfig {
        sheet_id: String::from("exp-inf"),
        title: String::from("Exp Inf"),
        n_rows: 16,
        seed: 42,
        columns: vec![SimulatedColumnSpec { name: String::from("E"), distribution: SimulatedDistribution::Exponential { rate: 1e-320 } }],
        use_multiple_variables: false,
    };
    assert!(matches!(simulate_distribution_columns(&cfg), Err(TransformError::EvalError { .. })));

    let res_sheet = result_sheet_with_tab(
        "res-ci",
        ResultTab {
            id: String::from("summary"),
            title: String::from("Summary"),
            kind: ResultTabKind::MainSummary,
            column_headers: str_vec(&["95% CI"]),
            row_labels: str_vec(&["Param", "Wide"]),
            rows: vec![
                vec![ResultCell::ConfidenceInterval { lower: CiBound::Finite(1.5), upper: CiBound::Finite(4.25) }],
                vec![ResultCell::ConfidenceInterval { lower: CiBound::Open, upper: CiBound::Finite(2.0) }],
            ],
            footnotes: Vec::new(),
        },
    );
    let mut dst = mv_empty("ci-dst", 1, 2);
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
    apply_import_link_pipeline(&mut dst, &link_spec, ImportSourcePayload::InternalResult(&res_sheet), &Default::default(), None).unwrap();
    let dst_mv = as_mv(&dst);
    assert_eq!((dst_mv.variables[0].cells[0].active_text(), dst_mv.variables[0].cells[1].active_text()), (Some("[1.5, 4.25]"), Some("[-inf, 2]")));
}
