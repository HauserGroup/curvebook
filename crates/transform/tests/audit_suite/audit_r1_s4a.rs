use curvebook_doc::{
    Cell, CellCoord, GroupedTable, ImportFilterSpec, MultipleVariablesTable, PlacementMode, SubcolumnMode, TableArchetype, VariableRole, XyTable,
};
use curvebook_transform::{
    PruneMethod, TransformContext, TransformError, evaluate_calculated_variables, evaluate_formula_for_sheet, execute_prune, execute_transpose,
    parse_formula, validate_no_formula_cycle,
};

use crate::common::{
    ExpectVal, SubExp, as_col, as_grouped, as_mv, as_survival, as_xy, assert_approx, assert_cells_3, assert_formula_first, clipboard_spec, col_empty,
    col_nums, import_csv, mv_empty, mv_sheet_roles, num, sheet, survival_sheet, txt, xy_3sub_clean, xy_single,
};

#[test]
fn r1_s04_02_long_left_assoc_chain_evaluates_without_stack_overflow() {
    let expr_text = vec!["1"; 8_000].join("+");
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let s = col_nums("s-chain", &[("A", &[42.0])]);
            drop(parse_formula(&expr_text).unwrap());
            evaluate_formula_for_sheet(&expr_text, &s, None).unwrap()[0].as_finite_f64().unwrap()
        })
        .unwrap();
    assert_approx(handle.join().expect("2 MiB worker thread must not stack overflow"), 8_000.0, 1e-12);
}

#[test]
fn r1_s04_04_builtin_constants_not_shadowed_by_column_letters() {
    let s = col_nums("s-const", &[("Series0", &[100.0]), ("Series1", &[101.0]), ("Series2", &[102.0]), ("Series3", &[103.0]), ("Series4", &[104.0])]);
    let e_sq = std::f64::consts::E * std::f64::consts::E;

    // Bare `e`/`pi` resolve to math constants while explicit `[E]` reads column index 4 (104.0).
    for (expr, expected) in [("exp(1) * e", e_sq), ("pi", std::f64::consts::PI), ("[E]", 104.0)] {
        assert_formula_first(&s, None, expr, ExpectVal::Num(expected), 1e-12);
    }

    // Calculated variable at index 4 (`E`) using `=exp(1) * e` must neither falsely self-cycle nor read col 4.
    let mut mv = MultipleVariablesTable::new(5, 2).unwrap();
    for i in 0..5 {
        mv.variables[i].title = if i == 4 { String::from("E") } else { format!("V{i}") };
        mv.variables[i].cells = vec![num(99.0), num(99.0)];
    }
    validate_no_formula_cycle(&mv, 4, "=exp(1) * e").unwrap();
    assert!(matches!(validate_no_formula_cycle(&mv, 4, "=[E] + 1"), Err(TransformError::CircularDependency { .. })));

    mv.variables[4].set_calculated_formula("=exp(1) * e");
    evaluate_calculated_variables(&mut mv, None).unwrap();
    assert_approx(mv.variables[4].cells[0].active_number().unwrap(), e_sq, 1e-12);
}

#[test]
fn r1_s04_05_place_grid_grows_columns_across_overwrite_insert_and_append_rows() {
    let csv = "10,20,30,40\n11,21,31,41\n";
    for (mode, row_offset) in [(PlacementMode::Overwrite, 0), (PlacementMode::InsertRows, 0), (PlacementMode::AppendRows, 2)] {
        let mut target = col_empty("grow", 2, 2);
        let spec = clipboard_spec(mode, CellCoord::new(0, 0, 0), ImportFilterSpec::default());
        let report = import_csv(&mut target, csv, &spec).unwrap();
        assert_eq!(report.cols_placed, 4, "expected 4 columns placed under {mode:?}");
        assert_eq!(target.archetype.column_count(), 4);
        let col = as_col(&target);
        assert_approx(col.data_sets[3].cells[row_offset][0].active_number().unwrap(), 40.0, 1e-12);
        assert_approx(col.data_sets[3].cells[row_offset + 1][0].active_number().unwrap(), 41.0, 1e-12);
    }
}

#[test]
fn r1_s04_06_insert_rows_shifts_existing_data_x_times_titles_and_spacing() {
    let spec = clipboard_spec(PlacementMode::InsertRows, CellCoord::new(1, 0, 0), ImportFilterSpec::default());

    let mut xy_sheet = xy_single("xy-ins", &[10.0, 20.0, 30.0], &[&[1.0, 2.0, 3.0]]);
    if let TableArchetype::Xy(xy) = &mut xy_sheet.archetype {
        xy.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
        xy.row_titles_expanded = true;
    }
    import_csv(&mut xy_sheet, "99\n100\n", &spec).unwrap();
    let after_xy = as_xy(&xy_sheet);
    assert_eq!(after_xy.row_count(), 5);
    assert_eq!(after_xy.row_titles, vec!["R0", "", "", "R1", "R2"]);
    assert_eq!((0..5).map(|r| after_xy.x_cells[r][0].active_number()).collect::<Vec<_>>(), vec![Some(10.0), None, None, Some(20.0), Some(30.0)]);
    assert_eq!(after_xy.data_sets[0].cells[1][0].active_number(), Some(99.0));
    assert_eq!(after_xy.data_sets[0].cells[3][0].active_number(), Some(2.0));

    // Survival.times and Grouped.row_numeric_spacing also shift on InsertRows.
    let mut srv_sheet = survival_sheet("srv", "", "", &[(5.0, 1.0), (15.0, 0.0)]);
    import_csv(&mut srv_sheet, "1\n", &spec).unwrap();
    let after_srv = as_survival(&srv_sheet);
    assert_eq!(after_srv.times.iter().map(Cell::active_number).collect::<Vec<_>>(), vec![Some(5.0), None, Some(15.0)]);
    assert_eq!(after_srv.cohorts[0].cells[2][0].active_number(), Some(0.0));

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0)]);
    grp.data_sets[0].cells = vec![vec![num(10.0), num(11.0)], vec![num(20.0), num(21.0)]];
    let mut grp_sheet = sheet("grp", TableArchetype::Grouped(grp));
    import_csv(&mut grp_sheet, "15,16\n", &spec).unwrap();
    let after_grp = as_grouped(&grp_sheet);
    assert_eq!(after_grp.row_numeric_spacing, Some(vec![Some(1.0), None, Some(2.0)]));
    assert_eq!(
        after_grp.data_sets[0].cells[1..3].iter().map(|r| (r[0].active_number(), r[1].active_number())).collect::<Vec<_>>(),
        vec![(Some(15.0), Some(16.0)), (Some(20.0), Some(21.0))]
    );
}

#[test]
fn r1_s04_07_average_bins_handles_presummarized_intervals_and_text() {
    let ctx = TransformContext::default();
    let tol = (1e-12, 0.0);

    // 1. Pre-summarized MeanSdN pools moments accurately (unbiased between-group variance: SS=16 -> sd=sqrt(16/3)).
    let sd_sheet =
        xy_3sub_clean("sd", SubcolumnMode::MeanSdN, &[1.0, 3.0], &[("A", &[[Some(10.0), Some(0.0), Some(2.0)], [Some(14.0), Some(0.0), Some(2.0)]])]);
    let sd_out = execute_prune(&sd_sheet, PruneMethod::AverageBins(2), &ctx).unwrap();
    let binned_sd = as_xy(&sd_out.data_sheet);
    assert_approx(binned_sd.x_cells[0][0].active_number().unwrap(), 2.0, 1e-12);
    assert_cells_3(&binned_sd.data_sets[0].cells[0], [SubExp::Num(12.0), SubExp::Num((16.0_f64 / 3.0).sqrt()), SubExp::Num(4.0)], tol, "MeanSdN bin");

    // 2. Unpoolable interval mode MeanLowHigh clears interval bounds and warns cleanly.
    let lh_sheet = xy_3sub_clean(
        "lh",
        SubcolumnMode::MeanLowHigh,
        &[1.0, 2.0],
        &[("A", &[[Some(10.0), Some(2.0), Some(3.0)], [Some(20.0), Some(4.0), Some(5.0)]])],
    );
    let lh_out = execute_prune(&lh_sheet, PruneMethod::AverageBins(2), &ctx).unwrap();
    assert!(!lh_out.warnings.is_empty());
    assert_cells_3(&as_xy(&lh_out.data_sheet).data_sets[0].cells[0], [SubExp::Num(15.0), SubExp::Empty, SubExp::Empty], tol, "MeanLowHigh bin");

    // 3. MultipleVariables uniform text preserves label; conflicting text warns cleanly.
    let mv_sheet = mv_sheet_roles(
        "mv",
        &[
            ("Cat", VariableRole::Categorical, &[txt("ctrl"), txt("ctrl"), txt("ctrl"), txt("trt")]),
            ("Val", VariableRole::Continuous, &[num(10.0), num(20.0), num(30.0), txt("oops")]),
        ],
    );
    let mv_out = execute_prune(&mv_sheet, PruneMethod::AverageBins(2), &ctx).unwrap();
    assert!(!mv_out.warnings.is_empty());
    let binned_mv = as_mv(&mv_out.data_sheet);
    assert_eq!(binned_mv.variables[0].cells[0], txt("ctrl"));
    assert!(binned_mv.variables[0].cells[1].is_empty());
    assert_approx(binned_mv.variables[1].cells[0].active_number().unwrap(), 15.0, 1e-12);
    assert_approx(binned_mv.variables[1].cells[1].active_number().unwrap(), 30.0, 1e-12);
}

#[test]
fn r1_s04_09_transpose_xy_preserves_replicates_and_presummarized_with_warning() {
    let ctx = TransformContext::default();
    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(3), 2, 2).unwrap();
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    for r in 0..2 {
        for s in 0..3 {
            xy.data_sets[0].cells[r][s] = num((r * 10 + s + 1) as f64);
            xy.data_sets[1].cells[r][s] = num((100 + r * 10 + s + 1) as f64);
        }
    }
    let out = execute_transpose(&sheet("xy-rep", TableArchetype::Xy(xy)), true, &ctx).unwrap();
    assert!(!out.warnings.is_empty(), "transposing multi-replicate XY should emit a warning");
    let col = as_col(&out.data_sheet);
    assert_eq!((col.row_count(), col.data_sets.len()), (6, 2));
    let mut flat: Vec<f64> = col.data_sets.iter().flat_map(|ds| ds.cells.iter().filter_map(|r| r[0].active_number())).collect();
    flat.sort_by(f64::total_cmp);
    assert_eq!(flat, vec![1.0, 2.0, 3.0, 11.0, 12.0, 13.0, 101.0, 102.0, 103.0, 111.0, 112.0, 113.0]);

    // Pre-summarized MeanSdN preserves subcolumns into ColumnTable cleanly.
    let msd_sheet = xy_3sub_clean(
        "xy-msd",
        SubcolumnMode::MeanSdN,
        &[1.0, 2.0],
        &[("A", &[[Some(5.0), Some(0.5), Some(4.0)], [Some(9.0), Some(0.8), Some(6.0)]])],
    );
    let msd_out = execute_transpose(&msd_sheet, false, &ctx).unwrap();
    let col_msd = as_col(&msd_out.data_sheet);
    assert_eq!(col_msd.subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!((col_msd.data_sets[1].cells[0][0].active_number(), col_msd.data_sets[1].cells[0][2].active_number()), (Some(9.0), Some(6.0)));
}

#[test]
fn r1_x2_04_import_parse_delimited_text_handles_rfc4180_quotes() {
    let csv = "\"Boston, MA\",\"He said \"\"OK\"\"\",\"9.5\"\r\n\"Austin, TX\",\"3\",10.0\n";
    let mut target = mv_empty("mv-q", 3, 1);
    let spec = clipboard_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), ImportFilterSpec::default());
    let report = import_csv(&mut target, csv, &spec).unwrap();
    assert_eq!((report.cols_placed, report.rows_placed), (3, 2));

    let mv = as_mv(&target);
    assert_eq!((&mv.variables[0].cells[0], &mv.variables[1].cells[0]), (&txt("Boston, MA"), &txt("He said \"OK\"")));
    assert_approx(mv.variables[2].cells[0].active_number().unwrap(), 9.5, 1e-12);
    assert_eq!(mv.variables[0].cells[1], txt("Austin, TX"));
    assert_approx(mv.variables[1].cells[1].active_number().unwrap(), 3.0, 1e-12);
    assert_approx(mv.variables[2].cells[1].active_number().unwrap(), 10.0, 1e-12);
}
