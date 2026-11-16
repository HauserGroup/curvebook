use curvebook_doc::{
    Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource,
    MultipleVariablesTable, PlacementMode, SubcolumnMode, SurvivalTable, TableArchetype, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::{
    ImportPipelineOptions, ImportSourcePayload, PruneMethod, TransformContext, TransformError, apply_import_link_pipeline,
    evaluate_calculated_variables, evaluate_formula_for_sheet, execute_prune, execute_transpose, parse_formula, validate_no_formula_cycle,
};

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} +/- {tol}, got {actual}");
}

#[test]
fn r1_s04_02_long_left_assoc_chain_evaluates_without_stack_overflow() {
    let expr_text = vec!["1"; 8_000].join("+");
    let handle = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
            col.data_sets[0].cells[0][0] = Cell::number(42.0);
            let sheet = DataSheet::new("s-chain", "Chain", TableArchetype::Column(col)).unwrap();
            let ast = parse_formula(&expr_text).unwrap();
            drop(ast);
            let vals = evaluate_formula_for_sheet(&expr_text, &sheet, None).unwrap();
            vals[0].as_finite_f64().unwrap()
        })
        .unwrap();
    let total = handle.join().expect("2 MiB worker thread must not stack overflow");
    assert_approx(total, 8_000.0, 1e-12);
}

#[test]
fn r1_s04_04_builtin_constants_not_shadowed_by_column_letters() {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 5, 1).unwrap();
    for c in 0..5 {
        col.data_sets[c].title = format!("Series{c}");
        col.data_sets[c].cells[0][0] = Cell::number(100.0 + c as f64);
    }
    let sheet = DataSheet::new("s-const", "Constants", TableArchetype::Column(col)).unwrap();
    let e_sq = std::f64::consts::E * std::f64::consts::E;

    // Bare `e` must resolve to mathematical constant `e`, not default column letter `E` (index 4).
    let res = evaluate_formula_for_sheet("exp(1) * e", &sheet, None).unwrap();
    assert_approx(res[0].as_finite_f64().unwrap(), e_sq, 1e-12);

    let pi_res = evaluate_formula_for_sheet("pi", &sheet, None).unwrap();
    assert_approx(pi_res[0].as_finite_f64().unwrap(), std::f64::consts::PI, 1e-12);

    // Bracketed / explicit column ref `[E]` must still read column index 4 (104.0).
    let bracket_res = evaluate_formula_for_sheet("[E]", &sheet, None).unwrap();
    assert_approx(bracket_res[0].as_finite_f64().unwrap(), 104.0, 1e-12);

    // Calculated variable at index 4 (`E`) using `=exp(1) * e` must neither falsely self-cycle nor read col 4.
    let mut mv = MultipleVariablesTable::new(5, 2).unwrap();
    for i in 0..5 {
        mv.variables[i].title = format!("V{i}");
        mv.variables[i].cells[0] = Cell::number(99.0);
        mv.variables[i].cells[1] = Cell::number(99.0);
    }
    mv.variables[4].title = String::from("E");
    validate_no_formula_cycle(&mv, 4, "=exp(1) * e").unwrap();
    assert!(matches!(validate_no_formula_cycle(&mv, 4, "=[E] + 1"), Err(TransformError::CircularDependency { .. })));

    mv.variables[4].set_calculated_formula("=exp(1) * e");
    evaluate_calculated_variables(&mut mv, None).unwrap();
    assert_approx(mv.variables[4].cells[0].active_number().unwrap(), e_sq, 1e-12);
}

#[test]
fn r1_s04_05_place_grid_grows_columns_across_overwrite_insert_and_append_rows() {
    let csv = "10,20,30,40\n11,21,31,41\n";
    for mode in [PlacementMode::Overwrite, PlacementMode::InsertRows, PlacementMode::AppendRows] {
        let mut target =
            DataSheet::new("grow", "Grow", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap())).unwrap();
        let spec = ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("wide.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec {
                mode,
                anchor: CellCoord::new(0, 0, 0),
                propagate_title_header: false,
                skip_contiguous_blanks: false,
                transpose: false,
            },
        };
        let report = apply_import_link_pipeline(
            &mut target,
            &spec,
            ImportSourcePayload::DelimitedText { text: csv, delimiter: ',', decimal_separator: '.' },
            &ImportPipelineOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(report.cols_placed, 4, "expected 4 columns placed under {mode:?}");
        assert_eq!(target.archetype.column_count(), 4);
        let row_offset = if mode == PlacementMode::AppendRows { 2 } else { 0 };
        let TableArchetype::Column(col) = &target.archetype else {
            panic!("expected Column");
        };
        assert_approx(col.data_sets[3].cells[row_offset][0].active_number().unwrap(), 40.0, 1e-12);
        assert_approx(col.data_sets[3].cells[row_offset + 1][0].active_number().unwrap(), 41.0, 1e-12);
    }
}

#[test]
fn r1_s04_06_insert_rows_shifts_existing_data_x_times_titles_and_spacing() {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 1, 3).unwrap();
    for r in 0..3 {
        xy.set_x_cell(r, Cell::number((r + 1) as f64 * 10.0)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64);
    }
    xy.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
    xy.row_titles_expanded = true;
    let mut xy_sheet = DataSheet::new("xy-ins", "XY", TableArchetype::Xy(xy)).unwrap();

    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("d.csv"), delimiter: ',', decimal_separator: '.' },
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
        &mut xy_sheet,
        &spec,
        ImportSourcePayload::DelimitedText { text: "99\n100\n", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    let TableArchetype::Xy(after_xy) = &xy_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(after_xy.row_count(), 5);
    assert_eq!(after_xy.row_titles, vec!["R0", "", "", "R1", "R2"]);
    assert_eq!(after_xy.x_cells[0][0].active_number(), Some(10.0));
    assert!(after_xy.x_cells[1][0].is_empty());
    assert!(after_xy.x_cells[2][0].is_empty());
    assert_eq!(after_xy.x_cells[3][0].active_number(), Some(20.0));
    assert_eq!(after_xy.x_cells[4][0].active_number(), Some(30.0));
    assert_eq!(after_xy.data_sets[0].cells[1][0].active_number(), Some(99.0));
    assert_eq!(after_xy.data_sets[0].cells[3][0].active_number(), Some(2.0));

    // Verify Survival.times and Grouped.row_numeric_spacing also shift on InsertRows.
    let mut srv = SurvivalTable::new(1, 2).unwrap();
    srv.times = vec![Cell::number(5.0), Cell::number(15.0)];
    srv.cohorts[0].cells[0][0] = Cell::number(1.0);
    srv.cohorts[0].cells[1][0] = Cell::number(0.0);
    let mut srv_sheet = DataSheet::new("srv", "Srv", TableArchetype::Survival(srv)).unwrap();
    apply_import_link_pipeline(
        &mut srv_sheet,
        &spec,
        ImportSourcePayload::DelimitedText { text: "1\n", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    let TableArchetype::Survival(after_srv) = &srv_sheet.archetype else {
        panic!("expected Survival");
    };
    assert_eq!(after_srv.times.len(), 3);
    assert_eq!(after_srv.times[0].active_number(), Some(5.0));
    assert!(after_srv.times[1].is_empty());
    assert_eq!(after_srv.times[2].active_number(), Some(15.0));
    assert_eq!(after_srv.cohorts[0].cells[2][0].active_number(), Some(0.0));

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(2.0)]);
    grp.data_sets[0].cells[0][0] = Cell::number(10.0);
    grp.data_sets[0].cells[0][1] = Cell::number(11.0);
    grp.data_sets[0].cells[1][0] = Cell::number(20.0);
    grp.data_sets[0].cells[1][1] = Cell::number(21.0);
    let mut grp_sheet = DataSheet::new("grp", "Grp", TableArchetype::Grouped(grp)).unwrap();
    apply_import_link_pipeline(
        &mut grp_sheet,
        &spec,
        ImportSourcePayload::DelimitedText { text: "15,16\n", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    let TableArchetype::Grouped(after_grp) = &grp_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(after_grp.row_numeric_spacing, Some(vec![Some(1.0), None, Some(2.0)]));
    assert_eq!(after_grp.data_sets[0].cells[1][0].active_number(), Some(15.0));
    assert_eq!(after_grp.data_sets[0].cells[1][1].active_number(), Some(16.0));
    assert_eq!(after_grp.data_sets[0].cells[2][0].active_number(), Some(20.0));
    assert_eq!(after_grp.data_sets[0].cells[2][1].active_number(), Some(21.0));
}

#[test]
fn r1_s04_07_average_bins_handles_presummarized_intervals_and_text() {
    let ctx = TransformContext::default();

    // 1. Pre-summarized MeanSdN pools moments accurately (unbiased between-group variance).
    let mut xy_sd = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy_sd.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_sd.set_x_cell(1, Cell::number(3.0)).unwrap();
    // Group 1: mean=10, sd=0, n=2; Group 2: mean=14, sd=0, n=2 -> pooled mean=12, n=4, SS=2*(4)+2*(4)=16 -> sd=sqrt(16/3).
    xy_sd.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(0.0), Cell::number(2.0)];
    xy_sd.data_sets[0].cells[1] = vec![Cell::number(14.0), Cell::number(0.0), Cell::number(2.0)];
    let sd_sheet = DataSheet::new("sd", "SD", TableArchetype::Xy(xy_sd)).unwrap();
    let sd_out = execute_prune(&sd_sheet, PruneMethod::AverageBins(2), &ctx).unwrap();
    let TableArchetype::Xy(binned_sd) = &sd_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx(binned_sd.x_cells[0][0].active_number().unwrap(), 2.0, 1e-12);
    assert_approx(binned_sd.data_sets[0].cells[0][0].active_number().unwrap(), 12.0, 1e-12);
    assert_approx(binned_sd.data_sets[0].cells[0][1].active_number().unwrap(), (16.0_f64 / 3.0).sqrt(), 1e-12);
    assert_approx(binned_sd.data_sets[0].cells[0][2].active_number().unwrap(), 4.0, 1e-12);

    // 2. Unpoolable interval mode MeanLowHigh clears interval bounds and warns cleanly.
    let mut xy_lh =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanLowHigh, 1, 2).unwrap();
    xy_lh.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy_lh.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy_lh.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(2.0), Cell::number(3.0)];
    xy_lh.data_sets[0].cells[1] = vec![Cell::number(20.0), Cell::number(4.0), Cell::number(5.0)];
    let lh_out = execute_prune(&DataSheet::new("lh", "LH", TableArchetype::Xy(xy_lh)).unwrap(), PruneMethod::AverageBins(2), &ctx).unwrap();
    assert!(!lh_out.warnings.is_empty());
    let TableArchetype::Xy(binned_lh) = &lh_out.data_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_approx(binned_lh.data_sets[0].cells[0][0].active_number().unwrap(), 15.0, 1e-12);
    assert!(binned_lh.data_sets[0].cells[0][1].is_empty());
    assert!(binned_lh.data_sets[0].cells[0][2].is_empty());

    // 3. MultipleVariables uniform text preserves label; conflicting text warns cleanly.
    let mut mv = MultipleVariablesTable::new(2, 4).unwrap();
    mv.variables[0].role = VariableRole::Categorical;
    mv.variables[0].role_inferred = false;
    mv.variables[0].cells = vec![Cell::text("ctrl"), Cell::text("ctrl"), Cell::text("ctrl"), Cell::text("trt")];
    mv.variables[1].role = VariableRole::Continuous;
    mv.variables[1].cells = vec![Cell::number(10.0), Cell::number(20.0), Cell::number(30.0), Cell::text("oops")];
    let mv_out =
        execute_prune(&DataSheet::new("mv", "MV", TableArchetype::MultipleVariables(mv)).unwrap(), PruneMethod::AverageBins(2), &ctx).unwrap();
    assert!(!mv_out.warnings.is_empty());
    let TableArchetype::MultipleVariables(binned_mv) = &mv_out.data_sheet.archetype else {
        panic!("expected MV");
    };
    assert_eq!(binned_mv.variables[0].cells[0], Cell::text("ctrl"));
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
            xy.data_sets[0].cells[r][s] = Cell::number((r * 10 + s + 1) as f64);
            xy.data_sets[1].cells[r][s] = Cell::number((100 + r * 10 + s + 1) as f64);
        }
    }
    let sheet = DataSheet::new("xy-rep", "Replicates", TableArchetype::Xy(xy)).unwrap();
    let out = execute_transpose(&sheet, true, &ctx).unwrap();
    assert!(!out.warnings.is_empty(), "transposing multi-replicate XY should emit a warning");

    let TableArchetype::Column(col) = &out.data_sheet.archetype else {
        panic!("expected Column output");
    };
    // 2 original rows * 3 replicates = 6 rows stacked; 2 original datasets = 2 output columns? No: transposed -> 2 original datasets become 2 rows? No, unrolled replicates: 2 cols * 3 reps = 6 rows, 2 cols!
    assert_eq!(col.row_count(), 6);
    assert_eq!(col.data_sets.len(), 2);
    // All 12 values preserved!
    let mut flat: Vec<f64> = col.data_sets.iter().flat_map(|ds| ds.cells.iter().filter_map(|r| r[0].active_number())).collect();
    flat.sort_by(f64::total_cmp);
    assert_eq!(flat, vec![1.0, 2.0, 3.0, 11.0, 12.0, 13.0, 101.0, 102.0, 103.0, 111.0, 112.0, 113.0]);

    // Pre-summarized MeanSdN preserves subcolumns into ColumnTable cleanly.
    let mut xy_msd = XyTable::new(SubcolumnMode::MeanSdN, 1, 2).unwrap();
    xy_msd.data_sets[0].cells[0] = vec![Cell::number(5.0), Cell::number(0.5), Cell::number(4.0)];
    xy_msd.data_sets[0].cells[1] = vec![Cell::number(9.0), Cell::number(0.8), Cell::number(6.0)];
    let msd_out = execute_transpose(&DataSheet::new("xy-msd", "MSD", TableArchetype::Xy(xy_msd)).unwrap(), false, &ctx).unwrap();
    let TableArchetype::Column(col_msd) = &msd_out.data_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_msd.subcolumn_mode, SubcolumnMode::MeanSdN);
    assert_eq!(col_msd.data_sets[1].cells[0][0].active_number(), Some(9.0));
    assert_eq!(col_msd.data_sets[1].cells[0][2].active_number(), Some(6.0));
}

#[test]
fn r1_x2_04_import_parse_delimited_text_handles_rfc4180_quotes() {
    let csv = "\"Boston, MA\",\"He said \"\"OK\"\"\",\"9.5\"\r\n\"Austin, TX\",\"3\",10.0\n";
    let mut target = DataSheet::new("mv-q", "Quotes", TableArchetype::MultipleVariables(MultipleVariablesTable::new(3, 1).unwrap())).unwrap();
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("q.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    let report = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )
    .unwrap();
    assert_eq!(report.cols_placed, 3);
    assert_eq!(report.rows_placed, 2);

    let TableArchetype::MultipleVariables(mv) = &target.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mv.variables[0].cells[0], Cell::text("Boston, MA"));
    assert_eq!(mv.variables[1].cells[0], Cell::text("He said \"OK\""));
    assert_approx(mv.variables[2].cells[0].active_number().unwrap(), 9.5, 1e-12);
    assert_eq!(mv.variables[0].cells[1], Cell::text("Austin, TX"));
    assert_approx(mv.variables[1].cells[1].active_number().unwrap(), 3.0, 1e-12);
    assert_approx(mv.variables[2].cells[1].active_number().unwrap(), 10.0, 1e-12);
}
