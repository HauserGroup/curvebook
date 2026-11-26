//! Permanent Round 2 (`R0.19`) Slice `S04` (`crates/transform`) audit regression tests.

use curvebook_doc::{
    Cell, ColumnDesignHint, ColumnTable, DocError, MAX_TITLE_BYTES, MultipleVariablesTable, SequenceProgression, SubcolumnMode, TableArchetype,
    TableKind, VariableRole, VariableSource, XColumnMode, XyTable,
};
use curvebook_transform::import::MAX_IMPORT_CELLS;
use curvebook_transform::{
    AllocationScheme, DistributionSimulationConfig, GroupAllocationWeight, PlateGeometry, PlateMapTemplate, PlateWellRole, RowSampleConfig,
    SelectRule, SimulatedColumnSpec, SimulatedDistribution, SubjectRandomizationConfig, TargetArchetypeKind, TidyColumnMapping, TransformContext,
    TransformError, convert_datasheet_archetype, evaluate_calculated_variables, execute_select, map_plate_to_datasheet, pivot_tidy_long_to_datasheet,
    preview_datasheet_conversion, random_sample_rows, randomize_subjects_to_groups, sample_row_indices, simulate_distribution_columns,
};

use crate::common::{
    as_col, as_grouped, as_mv, as_xy, assert_lossy_convert, empty, excl, excl_txt, grouped_sheet, mv_sheet, mv_table, num, raw_sheet, sheet,
    str_rows, str_vec, txt, xy_3sub_clean,
};

fn mean_and_sd(values: &[f64]) -> (f64, f64) {
    let (mut n, mut mean, mut m2) = (0.0_f64, 0.0_f64, 0.0_f64);
    for &x in values {
        n += 1.0;
        let delta = x - mean;
        mean += delta / n;
        m2 += delta * (x - mean);
    }
    (mean, (m2 / (n - 1.0)).sqrt())
}

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
        assert!(matches!(simulate_distribution_columns(&cfg), Err(TransformError::InvalidSpec { .. })), "trials={bad_trials}");
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

    let col_table = as_col(&simulate_distribution_columns(&cfg).unwrap()).clone();
    for (col_idx, &(trials, p)) in cases.iter().enumerate() {
        let vals = col_table.active_column_values(col_idx);
        assert_eq!(vals.len(), n_rows);
        assert!(vals.iter().all(|&v| v.is_finite() && v >= 0.0 && v <= trials as f64 && v.fract() == 0.0), "invalid draw for trials={trials}");
        let expected_mean = (trials as f64) * p;
        let expected_sd = ((trials as f64) * p * (1.0 - p)).sqrt();
        let se_mean = expected_sd / (n_rows as f64).sqrt();
        let (emp_mean, emp_sd) = mean_and_sd(&vals);
        assert!((emp_mean - expected_mean).abs() <= 5.0 * se_mean, "col {col_idx}: mean {emp_mean} vs {expected_mean} (SE={se_mean})");
        let sd_ratio = emp_sd / expected_sd;
        assert!((0.90..=1.10).contains(&sd_ratio), "col {col_idx}: SD ratio {sd_ratio} out of [0.90, 1.10]");
    }
}

#[test]
fn r2_s04_02_mv_to_column_and_grouped_respect_excluded() {
    let s = mv_sheet("mv1", &[("Group", &[txt("Ctrl"), excl_txt("Ctrl"), txt("Drug")]), ("Value", &[num(10.0), num(99.0), num(20.0)])]);
    for target in [TargetArchetypeKind::Column, TargetArchetypeKind::Grouped] {
        let (converted, prev) = assert_lossy_convert(&s, target);
        assert_eq!(prev.dropped_cells, 2, "{target:?}: {prev:?}");
        match &converted.archetype {
            TableArchetype::Column(col) => {
                assert_eq!((col.data_sets.len(), col.active_column_values(0), col.active_column_values(1)), (2, vec![10.0], vec![20.0]));
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
    let sheet_two = mv_sheet(
        "mv2",
        &[
            ("Dose", &[txt("1mg"), txt("1mg"), txt("2mg")]),
            ("Arm", &[txt("A"), excl_txt("A"), txt("B")]),
            ("Response", &[num(5.0), num(99.0), num(8.0)]),
        ],
    );
    let (grp_two, _) = assert_lossy_convert(&sheet_two, TargetArchetypeKind::Grouped);
    let active_two: Vec<f64> =
        as_grouped(&grp_two).data_sets.iter().flat_map(|ds| ds.cells.iter().flatten().filter_map(Cell::active_number)).collect();
    assert_eq!(active_two, vec![5.0, 8.0]);

    let sheet_xy = mv_sheet(
        "mv_xy",
        &[("X", &[num(1.0), num(1.0), num(-0.0)]), ("Series", &[txt("S1"), excl_txt("S1"), txt("S1")]), ("Y", &[num(10.0), num(99.0), num(15.0)])],
    );
    let (xy_out, _) = assert_lossy_convert(&sheet_xy, TargetArchetypeKind::Xy);
    assert_eq!(as_xy(&xy_out).active_xy_pairs(0), vec![(0.0, 15.0), (1.0, 10.0)]);
}

#[test]
fn r2_s04_03_long_to_xy_x_exclusion_isolated_per_observation() {
    let s = mv_sheet(
        "mv_x_ex",
        &[
            ("Dose", &[excl(10.0), num(10.0), excl(20.0), excl(20.0)]),
            ("Compound", &[txt("CompoundA"), txt("CompoundB"), txt("CompoundA"), txt("CompoundB")]),
            ("Response", &[num(50.0), num(85.0), num(40.0), num(60.0)]),
        ],
    );
    let (xy_sheet, preview) = convert_datasheet_archetype(&s, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!preview.lossy);

    let headers = str_vec(&["X", "Group", "Value"]);
    let rows = str_rows(&[&["*10", "CompoundA", "50"], &["10", "CompoundB", "85"], &["*20", "CompoundA", "40"], &["*20", "CompoundB", "60"]]);
    let mapping = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    let tidy_xy = pivot_tidy_long_to_datasheet("Tidy X Ex", &headers, &rows, &mapping, TargetArchetypeKind::Xy, '.').unwrap();

    for (label, out_sheet) in [("MV->XY", &xy_sheet), ("Tidy->XY", &tidy_xy)] {
        let xy = as_xy(out_sheet);
        assert!(!xy.x_cells[0][0].excluded && xy.x_cells[1][0].excluded, "{label} X exclusions");
        assert!(xy.data_sets[0].cells[0][0].excluded && !xy.data_sets[1].cells[0][0].excluded, "{label} Y exclusions");
        assert_eq!(xy.active_xy_pairs(0), vec![], "{label} series 0");
        assert_eq!(xy.active_xy_pairs(1), vec![(10.0, 85.0)], "{label} series 1");
    }
}

#[test]
fn r2_s04_04_grouped_mv_roundtrip_roles_and_row_numeric_spacing() {
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    xy.data_sets[0].title = String::new();
    xy.data_sets[1].title = String::from("Treated");
    for (r, x) in [1.0, 10.0, 100.0].into_iter().enumerate() {
        xy.set_x_cell(r, num(x)).unwrap();
        xy.data_sets[0].cells[r] = vec![num(10.0 + x), num(11.0 + x)];
        xy.data_sets[1].cells[r] = vec![num(20.0 + x), num(21.0 + x)];
    }
    let (grp_sheet, p1) = convert_datasheet_archetype(&sheet("xy_src", TableArchetype::Xy(xy)), TargetArchetypeKind::Grouped, false).unwrap();
    let (mv_sheet, p2) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!p1.lossy && !p2.lossy);

    let mv = as_mv(&mv_sheet);
    assert_eq!(
        (mv.variables[0].role, mv.variables[0].role_inferred, mv.variables[1].role, mv.variables[1].role_inferred),
        (VariableRole::Categorical, false, VariableRole::Categorical, false)
    );
    assert!(mv.variables[1].cells.iter().any(|c| c.active_text() == Some("A")));

    let (grp_rt, p3) = convert_datasheet_archetype(&mv_sheet, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!p3.lossy);
    assert_eq!(as_grouped(&grp_rt).row_numeric_spacing, Some(vec![Some(1.0), Some(10.0), Some(100.0)]));

    let (xy_rt, p4) = convert_datasheet_archetype(&grp_rt, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!p4.lossy);
    let xrt = as_xy(&xy_rt);
    assert_eq!(xrt.active_xy_pairs(0), vec![(1.0, 11.0), (1.0, 12.0), (10.0, 20.0), (10.0, 21.0), (100.0, 110.0), (100.0, 111.0)]);
    assert_eq!(xrt.active_xy_pairs(1), vec![(1.0, 21.0), (1.0, 22.0), (10.0, 30.0), (10.0, 31.0), (100.0, 120.0), (100.0, 121.0)]);
}

#[test]
fn r2_s04_05_presummarized_conversions_lossy_and_valid_schema() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::MeanSemN, 1, 2).unwrap();
    col.data_sets[0].title = String::from("Drug");
    col.data_sets[0].cells = vec![vec![num(12.0), num(0.8), num(6.0)], vec![num(18.0), num(1.2), num(6.0)]];
    let col_sheet = sheet("col_sem", TableArchetype::Column(col));

    let (mv_col, col_prev) = assert_lossy_convert(&col_sheet, TargetArchetypeKind::MultipleVariables);
    assert!(!col_prev.warnings.is_empty());
    let mvc = as_mv(&mv_col);
    assert_eq!(mvc.variables.len(), 3);
    assert_eq!((0..3).map(|c| mvc.variables[c].cells[0].active_number()).collect::<Vec<_>>(), vec![Some(12.0), Some(0.8), Some(6.0)]);

    let xy_sheet = xy_3sub_clean(
        "xy_sem",
        SubcolumnMode::MeanSemN,
        &[1.0, 2.0],
        &[
            ("A", &[[Some(10.0), Some(0.5), Some(3.0)], [Some(20.0), Some(1.0), Some(3.0)]]),
            ("B", &[[Some(30.0), Some(1.5), Some(4.0)], [Some(40.0), Some(2.0), Some(4.0)]]),
        ],
    );
    let _ = assert_lossy_convert(&xy_sheet, TargetArchetypeKind::MultipleVariables);
    let (col_from_xy, _) = assert_lossy_convert(&xy_sheet, TargetArchetypeKind::Column);
    let cxy = as_col(&col_from_xy);
    cxy.validate().unwrap();
    assert_eq!((cxy.data_sets.len(), cxy.row_titles.as_slice()), (2, [String::from("1"), String::from("2")].as_slice()));
    assert!(cxy.summarized_point(0, 0).unwrap().is_some() && cxy.summarized_point(1, 1).unwrap().is_some());

    let mut xy_match =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSdN }, SubcolumnMode::MeanSdN, 1, 1).unwrap();
    xy_match.x_cells[0] = vec![num(5.0), num(0.2), num(3.0)];
    xy_match.data_sets[0].cells[0] = vec![num(15.0), num(1.0), num(3.0)];
    let (match_col, match_prev) =
        convert_datasheet_archetype(&sheet("xy_sd", TableArchetype::Xy(xy_match)), TargetArchetypeKind::Column, false).unwrap();
    assert!(!match_prev.lossy);
    let mc = as_col(&match_col);
    assert_eq!(mc.data_sets.len(), 2);
    assert!(mc.summarized_point(0, 0).unwrap().is_some() && mc.summarized_point(0, 1).unwrap().is_some());
}

#[test]
fn r2_s04_06_parse_exclusion_flag_sentinels_and_zeros() {
    let headers = str_vec(&["Group", "Value", "Excl"]);
    let non_excluded = ["0", "0.0", "-0.0", "0,0", "+0.00", "", "   ", "null", "NULL", "n/a", "N/A", "na", "NaN", "nil", "false", "no"];
    let excluded = ["1", "true", "yes", "x", "outlier", "hemolyzed"];
    let mut rows: Vec<Vec<String>> =
        non_excluded.iter().enumerate().map(|(i, t)| vec![String::from("Active"), format!("{}", i + 1), (*t).to_owned()]).collect();
    rows.extend(excluded.iter().enumerate().map(|(i, t)| vec![String::from("Flagged"), format!("{}", 100 + i), (*t).to_owned()]));

    let mapping = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 1, excluded_col: Some(2) };
    let out = pivot_tidy_long_to_datasheet("Excl Flags", &headers, &rows, &mapping, TargetArchetypeKind::Column, '.').unwrap();
    let col = as_col(&out);
    assert_eq!(col.data_sets[0].cells.iter().filter(|r| r[0].excluded).count(), 0);
    assert_eq!(col.active_column_values(0).len(), non_excluded.len());
    assert_eq!(col.data_sets[1].cells[..excluded.len()].iter().filter(|r| !r[0].excluded).count(), 0);
}

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

    let (tall_rows, extra_groups) = (10_000_usize, 1_005_usize);
    let mut tidy_rows: Vec<Vec<String>> = (0..tall_rows).map(|_| str_vec(&["G0", "1.0"])).collect();
    tidy_rows.extend((0..extra_groups).map(|g| vec![format!("G{}", g + 1), String::from("1.0")]));
    let tidy_map = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 1, excluded_col: None };
    assert!(
        pivot_tidy_long_to_datasheet("Tidy Budget", &str_vec(&["Group", "Value"]), &tidy_rows, &tidy_map, TargetArchetypeKind::Column, '.').is_err()
    );
}

#[test]
fn r2_s04_07_mv_wide_pivot_cell_budgets() {
    let (tall_rows, extra_groups) = (10_000_usize, 1_005_usize);
    let mut mv_unstack = MultipleVariablesTable::new(2, tall_rows + extra_groups).unwrap();
    mv_unstack.variables[0].title = String::from("Group");
    mv_unstack.variables[1].title = String::from("Value");
    for r in 0..tall_rows {
        mv_unstack.variables[0].cells[r] = txt("G0");
        mv_unstack.variables[1].cells[r] = num(1.0);
    }
    for g in 0..extra_groups {
        mv_unstack.variables[0].cells[tall_rows + g] = txt(&format!("G{}", g + 1));
        mv_unstack.variables[1].cells[tall_rows + g] = num(1.0);
    }
    assert!(
        convert_datasheet_archetype(&sheet("mv_big_col", TableArchetype::MultipleVariables(mv_unstack)), TargetArchetypeKind::Column, true).is_err()
    );

    let mut mv_2way = MultipleVariablesTable::new(3, 2_010).unwrap();
    for (i, t) in ["RowF", "ColF", "Val"].into_iter().enumerate() {
        mv_2way.variables[i].title = String::from(t);
    }
    for i in 0..1_000 {
        mv_2way.variables[0].cells[i] = txt(&format!("R{i}"));
        mv_2way.variables[1].cells[i] = txt("C0");
        mv_2way.variables[2].cells[i] = num(1.0);
        mv_2way.variables[0].cells[1_000 + i] = txt("R0");
        mv_2way.variables[1].cells[1_000 + i] = txt(&format!("C{i}"));
        mv_2way.variables[2].cells[1_000 + i] = num(2.0);
    }
    for rep in 0..10 {
        mv_2way.variables[0].cells[2_000 + rep] = txt("R0");
        mv_2way.variables[1].cells[2_000 + rep] = txt("C0");
        mv_2way.variables[2].cells[2_000 + rep] = num(3.0);
    }
    assert!(
        convert_datasheet_archetype(&sheet("mv_big_2way", TableArchetype::MultipleVariables(mv_2way)), TargetArchetypeKind::Grouped, true).is_err()
    );
}

#[test]
#[allow(deprecated)]
fn r2_s04_08_title_length_validation_across_plate_tidy_and_convert() {
    let over = "A".repeat(MAX_TITLE_BYTES + 1);
    let is_too_long = |err: TransformError| matches!(err, TransformError::Doc(DocError::TitleTooLong { .. }));

    assert!(is_too_long(PlateMapTemplate::new(&over, PlateGeometry::Wells96, "Conc").unwrap_err()));
    assert!(is_too_long(PlateMapTemplate::new("Valid", PlateGeometry::Wells96, &over).unwrap_err()));

    let mut tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    assert!(is_too_long(tpl.set_well(0, 0, PlateWellRole::Treatment { group_name: over.clone(), dilution_x: 1.0, replicate: 0 }).unwrap_err()));

    let mut bad_wells = vec![PlateWellRole::Blank; 96];
    bad_wells[0] = PlateWellRole::Treatment { group_name: over.clone(), dilution_x: 1.0, replicate: 0 };
    assert!(is_too_long(PlateMapTemplate::from_wells("Valid", PlateGeometry::Wells96, "Conc", bad_wells.clone()).unwrap_err()));
    let raw_over =
        PlateMapTemplate { name: String::from("Valid"), geometry: PlateGeometry::Wells96, x_title: String::from("Conc"), wells: bad_wells };
    assert!(is_too_long(raw_over.validate().unwrap_err()));
    assert!(is_too_long(PlateMapTemplate::from_cbtemplate_json(&raw_over.to_cbtemplate_json().unwrap()).unwrap_err()));

    let ok_tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    assert!(is_too_long(
        map_plate_to_datasheet(&over, PlateGeometry::Wells96, &vec![Some(1.0); 96], &ok_tpl, TargetArchetypeKind::Column, false).unwrap_err()
    ));

    let headers = str_vec(&["X", "Group", "Val"]);
    let map = TidyColumnMapping { x_col: Some(0), group_col: 1, replicate_col: None, value_col: 2, excluded_col: None };
    for (title, row, target) in [
        (over.as_str(), ["cond", "Ctrl", "1.0"], TargetArchetypeKind::Column),
        ("Valid", ["cond", over.as_str(), "1.0"], TargetArchetypeKind::Column),
        ("Valid", [over.as_str(), "Ctrl", "1.0"], TargetArchetypeKind::Grouped),
    ] {
        assert!(is_too_long(pivot_tidy_long_to_datasheet(title, &headers, &str_rows(&[&row]), &map, target, '.').unwrap_err()));
    }

    let d_long = "D".repeat(2_500);
    let r_long = "R".repeat(2_500);
    let grp_sheet =
        grouped_sheet("grp_long", SubcolumnMode::RawReplicates(2), &[&r_long, "R2"], &[(&d_long, &[&[num(1.0), num(2.0)], &[num(3.0), num(4.0)]])]);
    assert!(is_too_long(preview_datasheet_conversion(&grp_sheet, TargetArchetypeKind::Column).unwrap_err()));
    assert!(is_too_long(convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Column, true).unwrap_err()));
}

#[test]
fn r2_s04_09_stratified_sampling_ignores_excluded_and_missing_strata() {
    let s = mv_sheet(
        "mv_strat",
        &[
            (
                "Stratum",
                &[
                    txt("Ctrl"),
                    txt("Ctrl"),
                    txt("Ctrl"),
                    txt("Drug"),
                    txt("Drug"),
                    txt("Drug"),
                    excl_txt("Ctrl"),
                    excl_txt("Drug"),
                    empty(),
                    txt("   "),
                ],
            ),
            ("Measurement", &[num(1.0), num(2.0), num(3.0), num(4.0), num(5.0), num(6.0), num(900.0), num(901.0), num(902.0), num(903.0)]),
        ],
    );

    for proportional in [true, false] {
        let cfg = RowSampleConfig {
            seed: 99,
            sample_size: 4,
            stratify_column: Some(0),
            proportional_allocation: proportional,
            output_id: Some(String::from("samp_ok")),
            output_title: Some(String::from("Sampled")),
        };
        let indices = sample_row_indices(&s, &cfg).unwrap();
        assert_eq!(indices.len(), 4);
        assert!(indices.iter().all(|&idx| idx < 6), "sampled excluded/blank indices: {indices:?}");

        let out_mv = as_mv(&random_sample_rows(&s, &cfg).unwrap()).clone();
        assert_eq!(out_mv.row_count(), 4);
        assert!(out_mv.variables[1].cells.iter().all(|c| c.active_number().unwrap() < 100.0));
    }

    let over_cfg =
        RowSampleConfig { seed: 99, sample_size: 7, stratify_column: Some(0), proportional_allocation: true, output_id: None, output_title: None };
    assert!(matches!(sample_row_indices(&s, &over_cfg), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(random_sample_rows(&s, &over_cfg), Err(TransformError::InvalidSpec { .. })));

    let empty_s = mv_sheet("mv_empty_s", &[("A", &[excl_txt("Ctrl"), empty()])]);
    let cfg_one =
        RowSampleConfig { seed: 1, sample_size: 1, stratify_column: Some(0), proportional_allocation: true, output_id: None, output_title: None };
    assert!(matches!(random_sample_rows(&empty_s, &cfg_one), Err(TransformError::InvalidSpec { .. })));
}

#[test]
fn r2_s04_10_randomize_subjects_rejects_duplicate_ids_and_labels() {
    for (id, subjects, groups) in [
        ("rand_dup_subj", &["S1", "S2", " S1 "][..], &["Ctrl", "Drug"][..]),
        ("rand_dup_grp", &["S1", "S2", "S3", "S4"][..], &["Ctrl", " Ctrl "][..]),
    ] {
        let cfg = SubjectRandomizationConfig {
            sheet_id: id.to_owned(),
            title: id.to_owned(),
            subject_ids: str_vec(subjects),
            groups: groups.iter().map(|&g| GroupAllocationWeight::new(g)).collect(),
            scheme: AllocationScheme::CompleteBalanced,
            seed: 1,
        };
        assert!(matches!(randomize_subjects_to_groups(&cfg), Err(TransformError::InvalidSpec { .. })), "{id}");
    }

    let valid = SubjectRandomizationConfig {
        sheet_id: String::from("rand_trim"),
        title: String::from("Trimmed Valid"),
        subject_ids: str_vec(&[" S1 ", "S2 ", " S3", "S4"]),
        groups: vec![GroupAllocationWeight::new(" Ctrl "), GroupAllocationWeight::new("Drug ")],
        scheme: AllocationScheme::CompleteBalanced,
        seed: 42,
    };
    let out = randomize_subjects_to_groups(&valid).unwrap();
    let mv = as_mv(&out.data_sheet);
    let mut ids: Vec<&str> = mv.variables[0].cells.iter().filter_map(Cell::active_text).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["S1", "S2", "S3", "S4"]);
    assert!(mv.variables[4].cells.iter().filter_map(Cell::active_text).all(|g| g == "Ctrl" || g == "Drug"));
}

#[test]
fn r2_s04_11_ragged_xy_table_no_oob_panic_on_convert() {
    for mode in [SubcolumnMode::Single, SubcolumnMode::RawReplicates(2), SubcolumnMode::MeanSdN, SubcolumnMode::MeanLowHigh] {
        let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, mode, 2, 5).unwrap();
        let k = mode.subcolumn_count().max(1);
        for r in 0..5 {
            xy.set_x_cell(r, num((r + 1) as f64)).unwrap();
            xy.data_sets[0].cells[r] = vec![num(10.0 * (r + 1) as f64); k];
            xy.data_sets[1].cells[r] = vec![num(100.0 + r as f64); k];
        }
        xy.data_sets[0].title = String::from("Short");
        xy.data_sets[1].title = String::from("Long");
        xy.data_sets[0].cells.truncate(2);
        xy.row_titles = str_vec(&["r0", "r1", "r2"]);
        let s = raw_sheet("ragged_xy", TableArchetype::Xy(xy));

        let (col_sheet, _) = convert_datasheet_archetype(&s, TargetArchetypeKind::Column, true).unwrap();
        let col = as_col(&col_sheet);
        assert_eq!((col.row_count(), col.data_sets.last().unwrap().cells[4][0].active_number()), (5, Some(104.0)));

        let (grp_sheet, _) = convert_datasheet_archetype(&s, TargetArchetypeKind::Grouped, true).unwrap();
        let grp = as_grouped(&grp_sheet);
        assert_eq!((grp.row_count(), grp.data_sets[1].cells[4][0].active_number(), grp.row_titles[4].as_str()), (5, Some(104.0), "5"));
    }

    let mut seq_xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 0.0, step: 10.0, progression: SequenceProgression::Arithmetic },
        SubcolumnMode::Single,
        2,
        5,
    )
    .unwrap();
    for r in 0..5 {
        seq_xy.data_sets[0].cells[r] = vec![num(1.0)];
        seq_xy.data_sets[1].cells[r] = vec![num((r * 10 + 1) as f64)];
    }
    seq_xy.data_sets[0].cells.truncate(2);
    seq_xy.x_cells.clear();
    let (seq_col, _) = convert_datasheet_archetype(&raw_sheet("ragged_seq", TableArchetype::Xy(seq_xy)), TargetArchetypeKind::Column, true).unwrap();
    let sc = as_col(&seq_col);
    assert_eq!(sc.active_column_values(0), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    assert_eq!(sc.data_sets[2].cells[4][0].active_number(), Some(41.0));
}

#[test]
fn r2_s04_12_execute_select_clears_calculated_role_on_snapshot() {
    let mut mv =
        mv_table(&[("Raw", &[num(1.0), num(2.0), num(3.0)]), ("Doubled", &[empty(), empty(), empty()]), ("Tag", &[empty(), empty(), empty()])]);
    mv.variables[1].set_calculated_formula("=Raw * 2");
    mv.variables[2].set_calculated_formula("=if(Raw > 1.5, \"High\", \"Low\")");
    evaluate_calculated_variables(&mut mv, None).unwrap();
    let s = sheet("mv_calc", TableArchetype::MultipleVariables(mv));

    let subset_ctx = TransformContext { selected_data_sets: vec![1, 2], ..TransformContext::default() };
    let sub_mv = as_mv(&execute_select(&s, &[], TableKind::MultipleVariables, &subset_ctx).unwrap().data_sheet).clone();
    sub_mv.validate().unwrap();
    assert_eq!(
        (&sub_mv.variables[0].source, sub_mv.variables[0].role, sub_mv.variables[0].role_inferred),
        (&VariableSource::Entered, VariableRole::Continuous, true)
    );
    assert_eq!(
        (&sub_mv.variables[1].source, sub_mv.variables[1].role, sub_mv.variables[1].role_inferred),
        (&VariableSource::Entered, VariableRole::Categorical, true)
    );
    assert_eq!(
        (sub_mv.variables[0].cells[2].active_number(), sub_mv.variables[1].cells[0].active_text(), sub_mv.variables[1].cells[1].active_text()),
        (Some(6.0), Some("Low"), Some("High"))
    );

    let full_mv =
        as_mv(&execute_select(&s, &[SelectRule::RowRange(0, 1)], TableKind::MultipleVariables, &TransformContext::default()).unwrap().data_sheet)
            .clone();
    assert!(matches!(full_mv.variables[1].source, VariableSource::Calculated { .. }) && full_mv.variables[1].role == VariableRole::Calculated);
}

#[test]
#[allow(deprecated)]
fn r2_s04_13_plate_arm_collision_and_template_empty_name_json_guards() {
    let mut tpl = PlateMapTemplate::new("Collision Plate", PlateGeometry::Wells96, "Conc").unwrap();
    for (c, role) in [
        PlateWellRole::Treatment { group_name: String::from("Blank"), dilution_x: 1.0, replicate: 0 },
        PlateWellRole::Treatment { group_name: String::from("Positive Control"), dilution_x: 1.0, replicate: 0 },
        PlateWellRole::Treatment { group_name: String::from("Excluded"), dilution_x: 1.0, replicate: 0 },
        PlateWellRole::Blank,
        PlateWellRole::PositiveControl,
        PlateWellRole::Excluded,
    ]
    .into_iter()
    .enumerate()
    {
        tpl.set_well(0, c, role).unwrap();
    }
    let mut values = vec![None; 96];
    for (i, v) in [100.0, 200.0, 300.0, 10.0, 50.0, 999.0].into_iter().enumerate() {
        values[i] = Some(v);
    }

    let col_sub = as_col(&map_plate_to_datasheet("Plate", PlateGeometry::Wells96, &values, &tpl, TargetArchetypeKind::Column, true).unwrap()).clone();
    assert_eq!(
        col_sub.data_sets.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
        vec!["Blank", "Positive Control", "Excluded", "Plate Positive Control", "Plate Excluded"]
    );
    assert_eq!((0..4).map(|c| col_sub.active_column_values(c)).collect::<Vec<_>>(), vec![vec![90.0], vec![190.0], vec![290.0], vec![40.0]]);
    assert!(col_sub.data_sets[4].cells[0][0].excluded);

    let col_raw =
        as_col(&map_plate_to_datasheet("Plate Raw", PlateGeometry::Wells96, &values, &tpl, TargetArchetypeKind::Column, false).unwrap()).clone();
    assert_eq!(
        col_raw.data_sets.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
        vec!["Blank", "Positive Control", "Excluded", "Plate Positive Control", "Plate Blank", "Plate Excluded"]
    );
    assert_eq!((col_raw.active_column_values(0), col_raw.active_column_values(4)), (vec![100.0], vec![10.0]));
    assert!(col_raw.data_sets[5].cells[0][0].excluded);

    let mut clean_tpl = PlateMapTemplate::new("Clean Plate", PlateGeometry::Wells96, "Conc").unwrap();
    clean_tpl.set_well(0, 0, PlateWellRole::Treatment { group_name: String::from("Drug"), dilution_x: 1.0, replicate: 0 }).unwrap();
    clean_tpl.set_well(0, 1, PlateWellRole::PositiveControl).unwrap();
    let mut clean_vals = vec![None; 96];
    (clean_vals[0], clean_vals[1]) = (Some(25.0), Some(50.0));
    let clean_col =
        as_col(&map_plate_to_datasheet("Clean", PlateGeometry::Wells96, &clean_vals, &clean_tpl, TargetArchetypeKind::Column, false).unwrap())
            .clone();
    assert_eq!(clean_col.data_sets[1].title, "Positive Control");

    assert!(matches!(PlateMapTemplate::new("  ", PlateGeometry::Wells96, "Conc"), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(
        PlateMapTemplate::from_wells("", PlateGeometry::Wells96, "Conc", vec![PlateWellRole::Blank; 96]),
        Err(TransformError::InvalidSpec { .. })
    ));
    let mut raw_tpl = PlateMapTemplate::new("Valid", PlateGeometry::Wells96, "Conc").unwrap();
    raw_tpl.name = String::from("  ");
    assert!(matches!(raw_tpl.validate(), Err(TransformError::InvalidSpec { .. })));
    assert!(matches!(PlateMapTemplate::from_cbtemplate_json(&raw_tpl.to_cbtemplate_json().unwrap()), Err(TransformError::InvalidSpec { .. })));

    let huge_wells = format!(
        "{{\"name\":\"Huge\",\"geometry\":\"Wells96\",\"x_title\":\"X\",\"wells\":[{}]}}",
        std::iter::repeat_n("\"Blank\"", 385).collect::<Vec<_>>().join(",")
    );
    assert!(PlateMapTemplate::from_cbtemplate_json(&huge_wells).is_err());
    assert!(matches!(PlateMapTemplate::from_cbtemplate_json(&" ".repeat((4 * 1024 * 1024) + 1)), Err(TransformError::InvalidInput { .. })));
}
