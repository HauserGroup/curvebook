//! Permanent Round 2 (`R0.19`) Slice `S04` (`crates/transform`) audit regression tests.

use lumencraft_doc::{
    Cell, ColumnDesignHint, ColumnTable, DataSheet, DocError, GroupedTable, MAX_TITLE_BYTES, MultipleVariablesTable, SequenceProgression, SheetId,
    SubcolumnMode, TableArchetype, TableKind, VariableRole, VariableSource, XColumnMode, XyTable,
};
use lumencraft_transform::import::MAX_IMPORT_CELLS;
use lumencraft_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, PlateGeometry, PlateMapTemplate, PlateWellRole, RowSampleConfig,
    SelectRule, SimulatedColumnSpec, SimulatedDistribution, SubjectRandomizationConfig, TargetArchetypeKind, TidyColumnMapping, TransformContext,
    TransformError, convert_datasheet_archetype, evaluate_calculated_variables, map_plate_to_datasheet, pivot_tidy_long_to_datasheet,
    preview_datasheet_conversion, random_sample_rows, randomize_subjects_to_groups, sample_row_indices, simulate_distribution_columns,
};

fn mean_and_sd(values: &[f64]) -> (f64, f64) {
    let mut n = 0.0_f64;
    let mut mean = 0.0_f64;
    let mut m2 = 0.0_f64;
    for &x in values {
        n += 1.0;
        let delta = x - mean;
        mean += delta / n;
        let delta2 = x - mean;
        m2 += delta * delta2;
    }
    let sd = (m2 / (n - 1.0)).sqrt();
    (mean, sd)
}

fn raw_xy_datasheet(id: &str, title: &str, xy: XyTable) -> DataSheet {
    DataSheet {
        id: SheetId::new(id),
        title: String::from(title),
        archetype: TableArchetype::Xy(xy),
        frozen: false,
        info_sheet_id: None,
        linked_regions: Vec::new(),
        floating_notes: Vec::new(),
        color_tag: None,
        exclusion_notes: Vec::new(),
    }
}

fn set_mv_cell(mv: &mut MultipleVariablesTable, col: usize, row: usize, cell: Cell) {
    mv.variables[col].cells[row] = cell;
}

// ============================================================================
// R2-S04-01: Binomial trials upper bound (<= 2^53) & cancel-free BTRS log-PMF
// ============================================================================

#[test]
fn r2_s04_01_binomial_trials_bound_and_btrs_precision() {
    let max_exact: u64 = 1_u64 << 53;
    for bad_trials in [max_exact + 1, 10_000_000_000_000_000_000_u64, u64::MAX] {
        let cfg = DistributionSimulationConfig {
            sheet_id: String::from("sim_bad"),
            title: String::from("Bad Binomial"),
            n_rows: 16,
            seed: 7,
            columns: vec![SimulatedColumnSpec {
                name: String::from("B"),
                distribution: SimulatedDistribution::Binomial { trials: bad_trials, p: 0.5 },
            }],
            use_multiple_variables: false,
        };
        let err = simulate_distribution_columns(&cfg).unwrap_err();
        assert!(matches!(err, TransformError::InvalidSpec { .. }), "trials={bad_trials} must fail validation: {err:?}");
    }

    let cases = [(max_exact, 0.5_f64), (1_000_000_000_000_000_u64, 0.27_f64), (2_000_000_000_000_u64, 1e-11_f64)];
    let n_rows = 4_000_usize;
    let cfg = DistributionSimulationConfig {
        sheet_id: String::from("sim_ok"),
        title: String::from("Large Binomial"),
        n_rows,
        seed: 2026,
        columns: cases
            .iter()
            .enumerate()
            .map(|(i, &(trials, p))| SimulatedColumnSpec { name: format!("B{i}"), distribution: SimulatedDistribution::Binomial { trials, p } })
            .collect(),
        use_multiple_variables: false,
    };

    let sheet = simulate_distribution_columns(&cfg).unwrap();
    let TableArchetype::Column(col_table) = &sheet.archetype else {
        panic!("expected Column archetype, found {:?}", sheet.kind());
    };
    for (col_idx, &(trials, p)) in cases.iter().enumerate() {
        let vals = col_table.active_column_values(col_idx);
        assert_eq!(vals.len(), n_rows);
        for &v in &vals {
            assert!(v.is_finite() && v >= 0.0 && v <= trials as f64 && v.fract() == 0.0, "invalid binomial draw {v} for trials={trials}");
        }
        let expected_mean = (trials as f64) * p;
        let expected_sd = ((trials as f64) * p * (1.0 - p)).sqrt();
        let se_mean = expected_sd / (n_rows as f64).sqrt();
        let (emp_mean, emp_sd) = mean_and_sd(&vals);
        assert!(
            (emp_mean - expected_mean).abs() <= 5.0 * se_mean,
            "col {col_idx}: empirical mean {emp_mean} drifted from {expected_mean} (SE={se_mean})"
        );
        let sd_ratio = emp_sd / expected_sd;
        assert!(
            (0.90..=1.10).contains(&sd_ratio),
            "col {col_idx}: empirical SD {emp_sd} vs theoretical {expected_sd} ratio={sd_ratio} out of [0.90, 1.10]"
        );
    }
}

// ============================================================================
// R2-S04-02: MV level tokens must respect Cell::excluded across pivots
// ============================================================================

#[test]
fn r2_s04_02_mv_to_column_and_grouped_respect_excluded() {
    let mut mv_one = MultipleVariablesTable::new(2, 3).unwrap();
    mv_one.variables[0].title = String::from("Group");
    mv_one.variables[1].title = String::from("Value");
    set_mv_cell(&mut mv_one, 0, 0, Cell::text("Ctrl"));
    set_mv_cell(&mut mv_one, 1, 0, Cell::number(10.0));
    set_mv_cell(&mut mv_one, 0, 1, Cell::excluded_text("Ctrl"));
    set_mv_cell(&mut mv_one, 1, 1, Cell::number(99.0));
    set_mv_cell(&mut mv_one, 0, 2, Cell::text("Drug"));
    set_mv_cell(&mut mv_one, 1, 2, Cell::number(20.0));
    let sheet_one = DataSheet::new("mv1", "MV1", TableArchetype::MultipleVariables(mv_one)).unwrap();

    for target in [TargetArchetypeKind::Column, TargetArchetypeKind::Grouped] {
        let prev = preview_datasheet_conversion(&sheet_one, target).unwrap();
        assert!(prev.lossy, "excluded factor cell with active numeric value must be marked lossy for {target:?}");
        assert_eq!(prev.dropped_cells, 2);
        assert!(convert_datasheet_archetype(&sheet_one, target, false).is_err());

        let (converted, _) = convert_datasheet_archetype(&sheet_one, target, true).unwrap();
        match &converted.archetype {
            TableArchetype::Column(col) => {
                assert_eq!(col.data_sets.len(), 2);
                assert_eq!(col.active_column_values(0), vec![10.0]);
                assert_eq!(col.active_column_values(1), vec![20.0]);
            }
            TableArchetype::Grouped(grp) => {
                let active: Vec<f64> = grp.data_sets.iter().flat_map(|ds| ds.cells.iter().flatten().filter_map(Cell::active_number)).collect();
                assert_eq!(active, vec![10.0, 20.0]);
            }
            other => panic!("unexpected archetype {other:?}"),
        }
    }
}

#[test]
fn r2_s04_02_mv_two_way_and_xy_respect_excluded_and_negative_zero() {
    let mut mv_two = MultipleVariablesTable::new(3, 3).unwrap();
    mv_two.variables[0].title = String::from("Dose");
    mv_two.variables[1].title = String::from("Arm");
    mv_two.variables[2].title = String::from("Response");
    set_mv_cell(&mut mv_two, 0, 0, Cell::text("1mg"));
    set_mv_cell(&mut mv_two, 1, 0, Cell::text("A"));
    set_mv_cell(&mut mv_two, 2, 0, Cell::number(5.0));
    set_mv_cell(&mut mv_two, 0, 1, Cell::text("1mg"));
    set_mv_cell(&mut mv_two, 1, 1, Cell::excluded_text("A"));
    set_mv_cell(&mut mv_two, 2, 1, Cell::number(99.0));
    set_mv_cell(&mut mv_two, 0, 2, Cell::text("2mg"));
    set_mv_cell(&mut mv_two, 1, 2, Cell::text("B"));
    set_mv_cell(&mut mv_two, 2, 2, Cell::number(8.0));
    let sheet_two = DataSheet::new("mv2", "MV2", TableArchetype::MultipleVariables(mv_two)).unwrap();

    assert!(preview_datasheet_conversion(&sheet_two, TargetArchetypeKind::Grouped).unwrap().lossy);
    assert!(convert_datasheet_archetype(&sheet_two, TargetArchetypeKind::Grouped, false).is_err());
    let (grp_two, _) = convert_datasheet_archetype(&sheet_two, TargetArchetypeKind::Grouped, true).unwrap();
    let TableArchetype::Grouped(gt) = &grp_two.archetype else {
        panic!("expected Grouped");
    };
    let active_two: Vec<f64> = gt.data_sets.iter().flat_map(|ds| ds.cells.iter().flatten().filter_map(Cell::active_number)).collect();
    assert_eq!(active_two, vec![5.0, 8.0]);

    let mut mv_xy = MultipleVariablesTable::new(3, 3).unwrap();
    mv_xy.variables[0].title = String::from("X");
    mv_xy.variables[1].title = String::from("Series");
    mv_xy.variables[2].title = String::from("Y");
    set_mv_cell(&mut mv_xy, 0, 0, Cell::number(1.0));
    set_mv_cell(&mut mv_xy, 1, 0, Cell::text("S1"));
    set_mv_cell(&mut mv_xy, 2, 0, Cell::number(10.0));
    set_mv_cell(&mut mv_xy, 0, 1, Cell::number(1.0));
    set_mv_cell(&mut mv_xy, 1, 1, Cell::excluded_text("S1"));
    set_mv_cell(&mut mv_xy, 2, 1, Cell::number(99.0));
    set_mv_cell(&mut mv_xy, 0, 2, Cell::number(-0.0));
    set_mv_cell(&mut mv_xy, 1, 2, Cell::text("S1"));
    set_mv_cell(&mut mv_xy, 2, 2, Cell::number(15.0));
    let sheet_xy = DataSheet::new("mv_xy", "MVXY", TableArchetype::MultipleVariables(mv_xy)).unwrap();
    assert!(convert_datasheet_archetype(&sheet_xy, TargetArchetypeKind::Xy, false).is_err());
    let (xy_out, _) = convert_datasheet_archetype(&sheet_xy, TargetArchetypeKind::Xy, true).unwrap();
    let TableArchetype::Xy(xy) = &xy_out.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy.active_xy_pairs(0), vec![(0.0, 15.0), (1.0, 10.0)]);
}

// ============================================================================
// R2-S04-03: Excluded X isolated per observation in Long-to-XY pivots
// ============================================================================

#[test]
fn r2_s04_03_long_to_xy_x_exclusion_isolated_per_observation() {
    let mut mv = MultipleVariablesTable::new(3, 4).unwrap();
    mv.variables[0].title = String::from("Dose");
    mv.variables[1].title = String::from("Compound");
    mv.variables[2].title = String::from("Response");

    set_mv_cell(&mut mv, 0, 0, Cell::excluded_number(10.0));
    set_mv_cell(&mut mv, 1, 0, Cell::text("CompoundA"));
    set_mv_cell(&mut mv, 2, 0, Cell::number(50.0));

    set_mv_cell(&mut mv, 0, 1, Cell::number(10.0));
    set_mv_cell(&mut mv, 1, 1, Cell::text("CompoundB"));
    set_mv_cell(&mut mv, 2, 1, Cell::number(85.0));

    set_mv_cell(&mut mv, 0, 2, Cell::excluded_number(20.0));
    set_mv_cell(&mut mv, 1, 2, Cell::text("CompoundA"));
    set_mv_cell(&mut mv, 2, 2, Cell::number(40.0));

    set_mv_cell(&mut mv, 0, 3, Cell::excluded_number(20.0));
    set_mv_cell(&mut mv, 1, 3, Cell::text("CompoundB"));
    set_mv_cell(&mut mv, 2, 3, Cell::number(60.0));

    let sheet = DataSheet::new("mv_x_ex", "MV X Ex", TableArchetype::MultipleVariables(mv)).unwrap();
    let (xy_sheet, preview) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!preview.lossy);
    let TableArchetype::Xy(xy) = &xy_sheet.archetype else {
        panic!("expected Xy");
    };
    assert!(!xy.x_cells[0][0].excluded, "shared X=10 has active CompoundB so X must stay active");
    assert!(xy.x_cells[1][0].excluded, "all records at X=20 have excluded X so shared X=20 must be excluded");
    assert!(xy.data_sets[0].cells[0][0].excluded, "CompoundA's Y at X=10 must inherit observation exclusion");
    assert!(!xy.data_sets[1].cells[0][0].excluded, "CompoundB's Y at X=10 must stay active");
    assert_eq!(xy.active_xy_pairs(0), vec![]);
    assert_eq!(xy.active_xy_pairs(1), vec![(10.0, 85.0)]);

    let headers = vec![String::from("X"), String::from("Group"), String::from("Value")];
    let rows = vec![
        vec![String::from("*10"), String::from("CompoundA"), String::from("50")],
        vec![String::from("10"), String::from("CompoundB"), String::from("85")],
        vec![String::from("*20"), String::from("CompoundA"), String::from("40")],
        vec![String::from("*20"), String::from("CompoundB"), String::from("60")],
    ];
    let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    let tidy_xy = pivot_tidy_long_to_datasheet("Tidy X Ex", &headers, &rows, &mapping, TargetArchetypeKind::Xy, '.').unwrap();
    let TableArchetype::Xy(txy) = &tidy_xy.archetype else {
        panic!("expected Xy");
    };
    assert!(!txy.x_cells[0][0].excluded);
    assert!(txy.x_cells[1][0].excluded);
    assert!(txy.data_sets[0].cells[0][0].excluded);
    assert!(!txy.data_sets[1].cells[0][0].excluded);
    assert_eq!(txy.active_xy_pairs(1), vec![(10.0, 85.0)]);
}

// ============================================================================
// R2-S04-04: Grouped <-> MV roles locked Categorical & row_numeric_spacing
// ============================================================================

#[test]
fn r2_s04_04_grouped_mv_roundtrip_roles_and_row_numeric_spacing() {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    xy.data_sets[0].title = String::new();
    xy.data_sets[1].title = String::from("Treated");
    for (r, &x_val) in [1.0, 10.0, 100.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(x_val)).unwrap();
        xy.data_sets[0].cells[r] = vec![Cell::number(10.0 + x_val), Cell::number(11.0 + x_val)];
        xy.data_sets[1].cells[r] = vec![Cell::number(20.0 + x_val), Cell::number(21.0 + x_val)];
    }
    let xy_sheet = DataSheet::new("xy_src", "Dose Curves", TableArchetype::Xy(xy)).unwrap();

    let (grp_sheet, prev_xg) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!prev_xg.lossy);

    let (mv_sheet, prev_gm) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!prev_gm.lossy);
    let TableArchetype::MultipleVariables(mv) = &mv_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables[0].role, VariableRole::Categorical);
    assert!(!mv.variables[0].role_inferred);
    assert_eq!(mv.variables[1].role, VariableRole::Categorical);
    assert!(!mv.variables[1].role_inferred);
    assert!(mv.variables[1].cells.iter().any(|c| c.active_text() == Some("A")));

    let (grp_rt_sheet, prev_mg) = convert_datasheet_archetype(&mv_sheet, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!prev_mg.lossy);
    let TableArchetype::Grouped(grp_rt) = &grp_rt_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp_rt.row_numeric_spacing, Some(vec![Some(1.0), Some(10.0), Some(100.0)]));

    let (xy_rt_sheet, prev_gx) = convert_datasheet_archetype(&grp_rt_sheet, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!prev_gx.lossy);
    let TableArchetype::Xy(xy_rt) = &xy_rt_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy_rt.active_xy_pairs(0), vec![(1.0, 11.0), (1.0, 12.0), (10.0, 20.0), (10.0, 21.0), (100.0, 110.0), (100.0, 111.0)]);
    assert_eq!(xy_rt.active_xy_pairs(1), vec![(1.0, 21.0), (1.0, 22.0), (10.0, 30.0), (10.0, 31.0), (100.0, 120.0), (100.0, 121.0)]);
}

// ============================================================================
// R2-S04-05: Pre-summarized conversions report lossy flag & valid schemas
// ============================================================================

#[test]
fn r2_s04_05_presummarized_conversions_lossy_and_valid_schema() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::MeanSemN, 1, 2).unwrap();
    col.data_sets[0].title = String::from("Drug");
    col.data_sets[0].cells[0] = vec![Cell::number(12.0), Cell::number(0.8), Cell::number(6.0)];
    col.data_sets[0].cells[1] = vec![Cell::number(18.0), Cell::number(1.2), Cell::number(6.0)];
    let col_sheet = DataSheet::new("col_sem", "Col SEM", TableArchetype::Column(col)).unwrap();

    let col_prev = preview_datasheet_conversion(&col_sheet, TargetArchetypeKind::MultipleVariables).unwrap();
    assert!(col_prev.lossy);
    assert!(!col_prev.warnings.is_empty());
    assert!(convert_datasheet_archetype(&col_sheet, TargetArchetypeKind::MultipleVariables, false).is_err());
    let (mv_col, _) = convert_datasheet_archetype(&col_sheet, TargetArchetypeKind::MultipleVariables, true).unwrap();
    let TableArchetype::MultipleVariables(mvc) = &mv_col.archetype else {
        panic!("expected MV");
    };
    assert_eq!(mvc.variables.len(), 3);
    assert_eq!(mvc.variables[0].cells[0].active_number(), Some(12.0));
    assert_eq!(mvc.variables[1].cells[0].active_number(), Some(0.8));
    assert_eq!(mvc.variables[2].cells[0].active_number(), Some(6.0));

    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::MeanSemN, 2, 2).unwrap();
    xy.set_x_cell(0, Cell::number(1.0)).unwrap();
    xy.set_x_cell(1, Cell::number(2.0)).unwrap();
    xy.data_sets[0].title = String::from("A");
    xy.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(0.5), Cell::number(3.0)];
    xy.data_sets[0].cells[1] = vec![Cell::number(20.0), Cell::number(1.0), Cell::number(3.0)];
    xy.data_sets[1].title = String::from("B");
    xy.data_sets[1].cells[0] = vec![Cell::number(30.0), Cell::number(1.5), Cell::number(4.0)];
    xy.data_sets[1].cells[1] = vec![Cell::number(40.0), Cell::number(2.0), Cell::number(4.0)];
    let xy_sheet = DataSheet::new("xy_sem", "XY SEM", TableArchetype::Xy(xy)).unwrap();

    assert!(preview_datasheet_conversion(&xy_sheet, TargetArchetypeKind::MultipleVariables).unwrap().lossy);
    assert!(convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::MultipleVariables, false).is_err());
    assert!(convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::MultipleVariables, true).is_ok());

    let xy_col_prev = preview_datasheet_conversion(&xy_sheet, TargetArchetypeKind::Column).unwrap();
    assert!(xy_col_prev.lossy);
    assert!(convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::Column, false).is_err());

    let (col_from_xy, _) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::Column, true).unwrap();
    let TableArchetype::Column(cxy) = &col_from_xy.archetype else {
        panic!("expected Column");
    };
    cxy.validate().unwrap();
    assert_eq!(cxy.data_sets.len(), 2, "scalar X must move to row_titles rather than padding a broken MeanSemN col 0");
    assert_eq!(cxy.row_titles, vec!["1", "2"]);
    assert!(cxy.summarized_point(0, 0).unwrap().is_some());
    assert!(cxy.summarized_point(1, 1).unwrap().is_some());

    let mut xy_match =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::MeanSdN, 1, 1).unwrap();
    xy_match.x_cells[0] = vec![Cell::number(5.0), Cell::number(0.2), Cell::number(3.0)];
    xy_match.data_sets[0].cells[0] = vec![Cell::number(15.0), Cell::number(1.0), Cell::number(3.0)];
    let match_sheet = DataSheet::new("xy_sd", "XY SD", TableArchetype::Xy(xy_match)).unwrap();
    let (match_col, match_prev) = convert_datasheet_archetype(&match_sheet, TargetArchetypeKind::Column, false).unwrap();
    assert!(!match_prev.lossy);
    let TableArchetype::Column(mc) = &match_col.archetype else {
        panic!("expected Column");
    };
    assert_eq!(mc.data_sets.len(), 2);
    assert!(mc.summarized_point(0, 0).unwrap().is_some());
    assert!(mc.summarized_point(0, 1).unwrap().is_some());
}

// ============================================================================
// R2-S04-06: parse_exclusion_flag rejects "0.0", "null", "n/a", "nil", "nan"
// ============================================================================

#[test]
fn r2_s04_06_parse_exclusion_flag_sentinels_and_zeros() {
    let headers = vec![String::from("Group"), String::from("Value"), String::from("Excl")];
    let non_excluded = ["0", "0.0", "-0.0", "0,0", "+0.00", "", "   ", "null", "NULL", "n/a", "N/A", "na", "NaN", "nil", "false", "no"];
    let excluded = ["1", "true", "yes", "x", "outlier", "hemolyzed"];

    let mut rows = Vec::new();
    for (i, token) in non_excluded.iter().enumerate() {
        rows.push(vec![String::from("Active"), format!("{}", i + 1), (*token).to_string()]);
    }
    for (i, token) in excluded.iter().enumerate() {
        rows.push(vec![String::from("Flagged"), format!("{}", 100 + i), (*token).to_string()]);
    }

    let mapping = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 1, excluded_col: Some(2) };
    let sheet = pivot_tidy_long_to_datasheet("Excl Flags", &headers, &rows, &mapping, TargetArchetypeKind::Column, '.').unwrap();
    let TableArchetype::Column(col) = &sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col.data_sets[0].cells.iter().filter(|r| r[0].excluded).count(), 0, "all sentinel/zero tokens must remain unexcluded");
    assert_eq!(col.active_column_values(0).len(), non_excluded.len());
    assert_eq!(col.data_sets[1].cells[..excluded.len()].iter().filter(|r| !r[0].excluded).count(), 0, "all truthy/reason flags must exclude");
}

// ============================================================================
// R2-S04-07: Checked cell budgets before allocation in pivots & simulation
// ============================================================================

#[test]
fn r2_s04_07_simulation_and_tidy_cell_budgets() {
    let sim_cfg = DistributionSimulationConfig {
        sheet_id: String::from("sim_oom"),
        title: String::from("Over Budget"),
        n_rows: 1_000_000,
        seed: 1,
        columns: (0..11)
            .map(|i| SimulatedColumnSpec { name: format!("C{i}"), distribution: SimulatedDistribution::Uniform { min: 0.0, max: 1.0 } })
            .collect(),
        use_multiple_variables: false,
    };
    assert!(MAX_IMPORT_CELLS < sim_cfg.n_rows * sim_cfg.columns.len());
    assert!(simulate_distribution_columns(&sim_cfg).is_err());

    let tall_rows = 10_000_usize;
    let extra_groups = 1_005_usize;
    let tidy_headers = vec![String::from("Group"), String::from("Value")];
    let mut tidy_rows = Vec::with_capacity(tall_rows + extra_groups);
    for _ in 0..tall_rows {
        tidy_rows.push(vec![String::from("G0"), String::from("1.0")]);
    }
    for g in 0..extra_groups {
        tidy_rows.push(vec![format!("G{}", g + 1), String::from("1.0")]);
    }
    let tidy_map = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 1, excluded_col: None };
    assert!(pivot_tidy_long_to_datasheet("Tidy Budget", &tidy_headers, &tidy_rows, &tidy_map, TargetArchetypeKind::Column, '.').is_err());
}

#[test]
fn r2_s04_07_mv_wide_pivot_cell_budgets() {
    let tall_rows = 10_000_usize;
    let extra_groups = 1_005_usize;
    let mut mv_unstack = MultipleVariablesTable::new(2, tall_rows + extra_groups).unwrap();
    mv_unstack.variables[0].title = String::from("Group");
    mv_unstack.variables[1].title = String::from("Value");
    for r in 0..tall_rows {
        set_mv_cell(&mut mv_unstack, 0, r, Cell::text("G0"));
        set_mv_cell(&mut mv_unstack, 1, r, Cell::number(1.0));
    }
    for g in 0..extra_groups {
        let r = tall_rows + g;
        set_mv_cell(&mut mv_unstack, 0, r, Cell::text(format!("G{}", g + 1)));
        set_mv_cell(&mut mv_unstack, 1, r, Cell::number(1.0));
    }
    let mv_unstack_sheet = DataSheet::new("mv_big_col", "Big Col", TableArchetype::MultipleVariables(mv_unstack)).unwrap();
    assert!(convert_datasheet_archetype(&mv_unstack_sheet, TargetArchetypeKind::Column, true).is_err());

    let mut mv_2way = MultipleVariablesTable::new(3, 2_010).unwrap();
    mv_2way.variables[0].title = String::from("RowF");
    mv_2way.variables[1].title = String::from("ColF");
    mv_2way.variables[2].title = String::from("Val");
    for r in 0..1_000 {
        set_mv_cell(&mut mv_2way, 0, r, Cell::text(format!("R{r}")));
        set_mv_cell(&mut mv_2way, 1, r, Cell::text("C0"));
        set_mv_cell(&mut mv_2way, 2, r, Cell::number(1.0));
    }
    for c in 0..1_000 {
        let row = 1_000 + c;
        set_mv_cell(&mut mv_2way, 0, row, Cell::text("R0"));
        set_mv_cell(&mut mv_2way, 1, row, Cell::text(format!("C{c}")));
        set_mv_cell(&mut mv_2way, 2, row, Cell::number(2.0));
    }
    for rep in 0..10 {
        let row = 2_000 + rep;
        set_mv_cell(&mut mv_2way, 0, row, Cell::text("R0"));
        set_mv_cell(&mut mv_2way, 1, row, Cell::text("C0"));
        set_mv_cell(&mut mv_2way, 2, row, Cell::number(3.0));
    }
    let mv_2way_sheet = DataSheet::new("mv_big_2way", "Big 2Way", TableArchetype::MultipleVariables(mv_2way)).unwrap();
    assert!(convert_datasheet_archetype(&mv_2way_sheet, TargetArchetypeKind::Grouped, true).is_err());
}

// ============================================================================
// R2-S04-08: MAX_TITLE_BYTES enforcement across plate, tidy, and convert
// ============================================================================

#[test]
fn r2_s04_08_title_length_validation_across_plate_tidy_and_convert() {
    let over = "A".repeat(MAX_TITLE_BYTES + 1);
    let err_new = PlateMapTemplate::new(&over, PlateGeometry::Wells96, "Conc").unwrap_err();
    assert!(matches!(err_new, TransformError::Doc(DocError::TitleTooLong { .. })));

    let err_x = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, &over).unwrap_err();
    assert!(matches!(err_x, TransformError::Doc(DocError::TitleTooLong { .. })));

    let mut tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    let err_set = tpl.set_well(0, 0, PlateWellRole::Treatment { group_name: over.clone(), dilution_x: 1.0, replicate: 0 }).unwrap_err();
    assert!(matches!(err_set, TransformError::Doc(DocError::TitleTooLong { .. })));

    let mut bad_wells = vec![PlateWellRole::Blank; 96];
    bad_wells[0] = PlateWellRole::Treatment { group_name: over.clone(), dilution_x: 1.0, replicate: 0 };
    assert!(matches!(
        PlateMapTemplate::from_wells("Valid", PlateGeometry::Wells96, "Conc", bad_wells.clone()),
        Err(TransformError::Doc(DocError::TitleTooLong { .. }))
    ));
    let raw_over_role =
        PlateMapTemplate { name: String::from("Valid"), geometry: PlateGeometry::Wells96, x_title: String::from("Conc"), wells: bad_wells };
    assert!(matches!(raw_over_role.validate(), Err(TransformError::Doc(DocError::TitleTooLong { .. }))));
    let json_over_role = raw_over_role.to_lctemplate_json().unwrap();
    assert!(matches!(PlateMapTemplate::from_lctemplate_json(&json_over_role), Err(TransformError::Doc(DocError::TitleTooLong { .. }))));

    let ok_tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    let values = vec![Some(1.0); 96];
    let err_plate = map_plate_to_datasheet(&over, PlateGeometry::Wells96, &values, &ok_tpl, TargetArchetypeKind::Column, false).unwrap_err();
    assert!(matches!(err_plate, TransformError::Doc(DocError::TitleTooLong { .. })));

    let headers = vec![String::from("X"), String::from("Group"), String::from("Val")];
    let map = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    let base_rows = vec![vec![String::from("cond"), String::from("Ctrl"), String::from("1.0")]];
    assert!(matches!(
        pivot_tidy_long_to_datasheet(&over, &headers, &base_rows, &map, TargetArchetypeKind::Column, '.'),
        Err(TransformError::Doc(DocError::TitleTooLong { .. }))
    ));
    let long_grp_rows = vec![vec![String::from("cond"), over.clone(), String::from("1.0")]];
    assert!(matches!(
        pivot_tidy_long_to_datasheet("Valid", &headers, &long_grp_rows, &map, TargetArchetypeKind::Column, '.'),
        Err(TransformError::Doc(DocError::TitleTooLong { .. }))
    ));
    let long_row_rows = vec![vec![over.clone(), String::from("Ctrl"), String::from("1.0")]];
    assert!(matches!(
        pivot_tidy_long_to_datasheet("Valid", &headers, &long_row_rows, &map, TargetArchetypeKind::Grouped, '.'),
        Err(TransformError::Doc(DocError::TitleTooLong { .. }))
    ));

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).unwrap();
    grp.data_sets[0].title = "D".repeat(2_500);
    grp.row_titles[0] = "R".repeat(2_500);
    grp.row_titles[1] = String::from("R2");
    grp.data_sets[0].cells[0] = vec![Cell::number(1.0), Cell::number(2.0)];
    grp.data_sets[0].cells[1] = vec![Cell::number(3.0), Cell::number(4.0)];
    let grp_sheet = DataSheet::new("grp_long", "Long Join", TableArchetype::Grouped(grp)).unwrap();
    assert!(matches!(preview_datasheet_conversion(&grp_sheet, TargetArchetypeKind::Column), Err(TransformError::Doc(DocError::TitleTooLong { .. }))));
    assert!(matches!(
        convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Column, true),
        Err(TransformError::Doc(DocError::TitleTooLong { .. }))
    ));
}

// ============================================================================
// R2-S04-09: Stratified sampling ignores excluded & empty stratum cells
// ============================================================================

#[test]
fn r2_s04_09_stratified_sampling_ignores_excluded_and_missing_strata() {
    let mut mv = MultipleVariablesTable::new(2, 10).unwrap();
    mv.variables[0].title = String::from("Stratum");
    mv.variables[1].title = String::from("Measurement");

    for r in 0..3 {
        set_mv_cell(&mut mv, 0, r, Cell::text("Ctrl"));
        set_mv_cell(&mut mv, 1, r, Cell::number((r + 1) as f64));
    }
    for r in 3..6 {
        set_mv_cell(&mut mv, 0, r, Cell::text("Drug"));
        set_mv_cell(&mut mv, 1, r, Cell::number((r + 1) as f64));
    }
    set_mv_cell(&mut mv, 0, 6, Cell::excluded_text("Ctrl"));
    set_mv_cell(&mut mv, 1, 6, Cell::number(900.0));
    set_mv_cell(&mut mv, 0, 7, Cell::excluded_text("Drug"));
    set_mv_cell(&mut mv, 1, 7, Cell::number(901.0));
    set_mv_cell(&mut mv, 0, 8, Cell::empty());
    set_mv_cell(&mut mv, 1, 8, Cell::number(902.0));
    set_mv_cell(&mut mv, 0, 9, Cell::text("   "));
    set_mv_cell(&mut mv, 1, 9, Cell::number(903.0));

    let sheet = DataSheet::new("mv_strat", "Stratified", TableArchetype::MultipleVariables(mv)).unwrap();

    for proportional in [true, false] {
        let cfg = RowSampleConfig {
            seed: 99,
            sample_size: 4,
            stratify_column: Some(0),
            proportional_allocation: proportional,
            output_id: Some(String::from("samp_ok")),
            output_title: Some(String::from("Sampled")),
        };
        let indices = sample_row_indices(&sheet, &cfg).unwrap();
        assert_eq!(indices.len(), 4);
        assert!(indices.iter().all(|&idx| idx < 6), "sampled excluded/blank stratum indices: {indices:?}");

        let out = random_sample_rows(&sheet, &cfg).unwrap();
        let TableArchetype::MultipleVariables(out_mv) = &out.archetype else {
            panic!("expected MV");
        };
        assert_eq!(out_mv.row_count(), 4);
        assert!(out_mv.variables[1].cells.iter().all(|c| c.active_number().unwrap() < 100.0));
    }

    let over_cfg =
        RowSampleConfig { seed: 99, sample_size: 7, stratify_column: Some(0), proportional_allocation: true, output_id: None, output_title: None };
    assert!(matches!(sample_row_indices(&sheet, &over_cfg), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(random_sample_rows(&sheet, &over_cfg), Err(TransformError::InvalidSpec { .. })));

    let mut empty_mv = MultipleVariablesTable::new(1, 2).unwrap();
    set_mv_cell(&mut empty_mv, 0, 0, Cell::excluded_text("Ctrl"));
    set_mv_cell(&mut empty_mv, 0, 1, Cell::empty());
    let empty_sheet = DataSheet::new("mv_empty_s", "Empty Strata", TableArchetype::MultipleVariables(empty_mv)).unwrap();
    let cfg_one =
        RowSampleConfig { seed: 1, sample_size: 1, stratify_column: Some(0), proportional_allocation: true, output_id: None, output_title: None };
    assert!(matches!(random_sample_rows(&empty_sheet, &cfg_one), Err(TransformError::InvalidSpec { .. })));
}

// ============================================================================
// R2-S04-10: Subject randomization rejects duplicate IDs & group labels
// ============================================================================

#[test]
fn r2_s04_10_randomize_subjects_rejects_duplicate_ids_and_labels() {
    let dup_subjects = SubjectRandomizationConfig {
        sheet_id: String::from("rand_dup_subj"),
        title: String::from("Dup Subj"),
        subject_ids: vec![String::from("S1"), String::from("S2"), String::from(" S1 ")],
        groups: vec![GroupAllocationWeight::new("Ctrl"), GroupAllocationWeight::new("Drug")],
        scheme: AllocationScheme::CompleteBalanced,
        seed: 1,
    };
    assert!(matches!(randomize_subjects_to_groups(&dup_subjects), Err(TransformError::InvalidSpec { .. })));

    let dup_groups = SubjectRandomizationConfig {
        sheet_id: String::from("rand_dup_grp"),
        title: String::from("Dup Grp"),
        subject_ids: vec![String::from("S1"), String::from("S2"), String::from("S3"), String::from("S4")],
        groups: vec![GroupAllocationWeight::new("Ctrl"), GroupAllocationWeight::new(" Ctrl ")],
        scheme: AllocationScheme::CompleteBalanced,
        seed: 1,
    };
    assert!(matches!(randomize_subjects_to_groups(&dup_groups), Err(TransformError::InvalidSpec { .. })));

    let valid = SubjectRandomizationConfig {
        sheet_id: String::from("rand_trim"),
        title: String::from("Trimmed Valid"),
        subject_ids: vec![String::from(" S1 "), String::from("S2 "), String::from(" S3"), String::from("S4")],
        groups: vec![GroupAllocationWeight::new(" Ctrl "), GroupAllocationWeight::new("Drug ")],
        scheme: AllocationScheme::CompleteBalanced,
        seed: 42,
    };
    let out = randomize_subjects_to_groups(&valid).unwrap();
    let TableArchetype::MultipleVariables(mv) = &out.data_sheet.archetype else {
        panic!("expected MV");
    };
    let mut ids: Vec<&str> = mv.variables[0].cells.iter().filter_map(Cell::active_text).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["S1", "S2", "S3", "S4"]);
    assert!(mv.variables[4].cells.iter().filter_map(Cell::active_text).all(|g| g == "Ctrl" || g == "Drug"));
}

// ============================================================================
// R2-S04-11: Ragged XyTable bounds safety (zero OOB panic) on convert
// ============================================================================

#[test]
fn r2_s04_11_ragged_xy_table_no_oob_panic_on_convert() {
    for mode in [SubcolumnMode::Single, SubcolumnMode::RawReplicates(2), SubcolumnMode::MeanSdN, SubcolumnMode::MeanLowHigh] {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 2, 5).unwrap();
        let k = mode.subcolumn_count().max(1);
        for r in 0..5 {
            xy.set_x_cell(r, Cell::number((r + 1) as f64)).unwrap();
            xy.data_sets[0].cells[r] = vec![Cell::number(10.0 * (r + 1) as f64); k];
            xy.data_sets[1].cells[r] = vec![Cell::number(100.0 + r as f64); k];
        }
        xy.data_sets[0].title = String::from("Short");
        xy.data_sets[1].title = String::from("Long");
        xy.data_sets[0].cells.truncate(2);
        xy.row_titles = vec![String::from("r0"), String::from("r1"), String::from("r2")];
        let sheet = raw_xy_datasheet("ragged_xy", "Ragged XY", xy);

        let (col_sheet, _) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Column, true).unwrap();
        let TableArchetype::Column(col) = &col_sheet.archetype else {
            panic!("expected Column");
        };
        assert_eq!(col.row_count(), 5);
        assert_eq!(col.data_sets.last().unwrap().cells[4][0].active_number(), Some(104.0));

        let (grp_sheet, _) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Grouped, true).unwrap();
        let TableArchetype::Grouped(grp) = &grp_sheet.archetype else {
            panic!("expected Grouped");
        };
        assert_eq!(grp.row_count(), 5);
        assert_eq!(grp.data_sets[1].cells[4][0].active_number(), Some(104.0));
        assert_eq!(grp.row_titles[4], "5");
    }

    let mut seq_xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 0.0, step: 10.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        2,
        5,
    )
    .unwrap();
    for r in 0..5 {
        seq_xy.data_sets[0].cells[r] = vec![Cell::number(1.0)];
        seq_xy.data_sets[1].cells[r] = vec![Cell::number((r * 10 + 1) as f64)];
    }
    seq_xy.data_sets[0].cells.truncate(2);
    seq_xy.x_cells.clear();
    let seq_sheet = raw_xy_datasheet("ragged_seq", "Ragged Seq", seq_xy);
    let (seq_col, _) = convert_datasheet_archetype(&seq_sheet, TargetArchetypeKind::Column, true).unwrap();
    let TableArchetype::Column(sc) = &seq_col.archetype else {
        panic!("expected Column");
    };
    assert_eq!(sc.active_column_values(0), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    assert_eq!(sc.data_sets[2].cells[4][0].active_number(), Some(41.0));
}

// ============================================================================
// R2-S04-12: execute_select clears VariableRole::Calculated when snapshotting
// ============================================================================

#[test]
fn r2_s04_12_execute_select_clears_calculated_role_on_snapshot() {
    let mut mv = MultipleVariablesTable::new(3, 3).unwrap();
    mv.variables[0].title = String::from("Raw");
    for r in 0..3 {
        set_mv_cell(&mut mv, 0, r, Cell::number((r + 1) as f64));
    }
    mv.variables[1].title = String::from("Doubled");
    mv.variables[1].set_calculated_formula("=Raw * 2");
    mv.variables[2].title = String::from("Tag");
    mv.variables[2].set_calculated_formula("=if(Raw > 1.5, \"High\", \"Low\")");
    evaluate_calculated_variables(&mut mv, None).unwrap();

    let sheet = DataSheet::new("mv_calc", "Calculated MV", TableArchetype::MultipleVariables(mv)).unwrap();

    let subset_ctx = TransformContext { selected_data_sets: vec![1, 2], ..TransformContext::default() };
    let subset_out = lumencraft_transform::execute_select(&sheet, &[], TableKind::MultipleVariables, &subset_ctx).unwrap();
    let TableArchetype::MultipleVariables(sub_mv) = &subset_out.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    sub_mv.validate().unwrap();
    assert_eq!(sub_mv.variables[0].source, VariableSource::Entered);
    assert_eq!(sub_mv.variables[0].role, VariableRole::Continuous);
    assert!(sub_mv.variables[0].role_inferred);
    assert_eq!(sub_mv.variables[1].source, VariableSource::Entered);
    assert_eq!(sub_mv.variables[1].role, VariableRole::Categorical);
    assert!(sub_mv.variables[1].role_inferred);
    assert_eq!(sub_mv.variables[0].cells[2].active_number(), Some(6.0));
    assert_eq!(sub_mv.variables[1].cells[0].active_text(), Some("Low"));
    assert_eq!(sub_mv.variables[1].cells[1].active_text(), Some("High"));

    let full_ctx = TransformContext::default();
    let full_out = lumencraft_transform::execute_select(&sheet, &[SelectRule::RowRange(0, 1)], TableKind::MultipleVariables, &full_ctx).unwrap();
    let TableArchetype::MultipleVariables(full_mv) = &full_out.data_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert!(matches!(full_mv.variables[1].source, VariableSource::Calculated { .. }));
    assert_eq!(full_mv.variables[1].role, VariableRole::Calculated);
}

// ============================================================================
// R2-S04-13: Plate arm collision separation & template empty-name/JSON guards
// ============================================================================

#[test]
fn r2_s04_13_plate_arm_collision_and_template_empty_name_json_guards() {
    let mut tpl = PlateMapTemplate::new("Collision Plate", PlateGeometry::Wells96, "Conc").unwrap();
    tpl.set_well(0, 0, PlateWellRole::Treatment { group_name: String::from("Blank"), dilution_x: 1.0, replicate: 0 }).unwrap();
    tpl.set_well(0, 1, PlateWellRole::Treatment { group_name: String::from("Positive Control"), dilution_x: 1.0, replicate: 0 }).unwrap();
    tpl.set_well(0, 2, PlateWellRole::Treatment { group_name: String::from("Excluded"), dilution_x: 1.0, replicate: 0 }).unwrap();
    tpl.set_well(0, 3, PlateWellRole::Blank).unwrap();
    tpl.set_well(0, 4, PlateWellRole::PositiveControl).unwrap();
    tpl.set_well(0, 5, PlateWellRole::Excluded).unwrap();

    let mut values = vec![None; 96];
    values[0] = Some(100.0);
    values[1] = Some(200.0);
    values[2] = Some(300.0);
    values[3] = Some(10.0);
    values[4] = Some(50.0);
    values[5] = Some(999.0);

    let sheet_sub = map_plate_to_datasheet("Plate", PlateGeometry::Wells96, &values, &tpl, TargetArchetypeKind::Column, true).unwrap();
    let TableArchetype::Column(col_sub) = &sheet_sub.archetype else {
        panic!("expected Column");
    };
    let titles_sub: Vec<&str> = col_sub.data_sets.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles_sub, vec!["Blank", "Positive Control", "Excluded", "Plate Positive Control", "Plate Excluded"]);
    assert_eq!(col_sub.active_column_values(0), vec![90.0]);
    assert_eq!(col_sub.active_column_values(1), vec![190.0]);
    assert_eq!(col_sub.active_column_values(2), vec![290.0]);
    assert_eq!(col_sub.active_column_values(3), vec![40.0]);
    assert!(col_sub.data_sets[4].cells[0][0].excluded);

    let sheet_raw = map_plate_to_datasheet("Plate Raw", PlateGeometry::Wells96, &values, &tpl, TargetArchetypeKind::Column, false).unwrap();
    let TableArchetype::Column(col_raw) = &sheet_raw.archetype else {
        panic!("expected Column");
    };
    let titles_raw: Vec<&str> = col_raw.data_sets.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles_raw, vec!["Blank", "Positive Control", "Excluded", "Plate Positive Control", "Plate Blank", "Plate Excluded"]);
    assert_eq!(col_raw.active_column_values(0), vec![100.0]);
    assert_eq!(col_raw.active_column_values(4), vec![10.0]);
    assert!(col_raw.data_sets[5].cells[0][0].excluded);

    let mut clean_tpl = PlateMapTemplate::new("Clean Plate", PlateGeometry::Wells96, "Conc").unwrap();
    clean_tpl.set_well(0, 0, PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: 1.0, replicate: 0 }).unwrap();
    clean_tpl.set_well(0, 1, PlateWellRole::PositiveControl).unwrap();
    let mut clean_vals = vec![None; 96];
    clean_vals[0] = Some(25.0);
    clean_vals[1] = Some(50.0);
    let clean_sheet = map_plate_to_datasheet("Clean", PlateGeometry::Wells96, &clean_vals, &clean_tpl, TargetArchetypeKind::Column, false).unwrap();
    let TableArchetype::Column(clean_col) = &clean_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(clean_col.data_sets[1].title, "Positive Control");

    assert!(matches!(PlateMapTemplate::new("  ", PlateGeometry::Wells96, "Conc"), Err(TransformError::InvalidSpec { .. })));
    let blank_wells = vec![PlateWellRole::Blank; 96];
    assert!(matches!(PlateMapTemplate::from_wells("", PlateGeometry::Wells96, "Conc", blank_wells), Err(TransformError::InvalidSpec { .. })));
    let mut raw_tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    raw_tpl.name = String::from("  ");
    assert!(matches!(raw_tpl.validate(), Err(TransformError::InvalidSpec { .. })));
    let bad_json = raw_tpl.to_lctemplate_json().unwrap();
    assert!(matches!(PlateMapTemplate::from_lctemplate_json(&bad_json), Err(TransformError::InvalidSpec { .. })));

    let huge_wells_json = format!(
        "{{\"name\":\"Huge\",\"geometry\":\"Wells96\",\"x_title\":\"X\",\"wells\":[{}]}}",
        std::iter::repeat_n("\"Blank\"", 385).collect::<Vec<_>>().join(",")
    );
    assert!(PlateMapTemplate::from_lctemplate_json(&huge_wells_json).is_err());

    let huge_json = " ".repeat((4 * 1024 * 1024) + 1);
    assert!(matches!(PlateMapTemplate::from_lctemplate_json(&huge_json), Err(TransformError::InvalidInput { .. })));
}
