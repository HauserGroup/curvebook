//! Integration tests for `R17.7` microplate grid layout builders, analyzable-sheet plate
//! extraction (well-per-row and kinetic reducers), `ImportFilterSpec::row_match`, and full
//! `plate_well_origins` lifecycle maintenance across `curvebook-transform` operations.

use std::error::Error;

use curvebook_doc::{
    BaselineOperation, Cell, CellCoord, ColumnDesignHint, ColumnEqualsFilter, ColumnTable, DataSheet, DerivedColumnFormula, ErrorPropagationMode,
    GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, LinkSource, MultipleVariablesTable, NormalizeAnchor, PlacementMode,
    PlateCellOrigin, SubcolumnMode, TableArchetype, TableKind, XyTable,
};
use curvebook_transform::{
    BaselineKind, ImportPipelineOptions, ImportSourcePayload, MathTransformOp, PlateGeometry, PlateGridBuildOptions, PlateKineticReducer,
    PlateMapTemplate, PlateSourceExtraction, PlateWellMatchMode, PlateWellRole, PruneMethod, RowSampleConfig, SelectRule, TargetArchetypeKind,
    TransformContext, TransformError, apply_import_link_pipeline, build_plate_template_from_grids, convert_datasheet_archetype, execute_math_op,
    execute_normalize, execute_prune, execute_remove_baseline_kind, execute_select_with_derived, execute_transpose,
    extract_plate_grid_from_datasheet, extract_plate_values_from_analyzable_sheet, extract_plate_x_grid_from_datasheet,
    map_plate_to_datasheet_with_origins, random_sample_rows,
};

type TestResult = Result<(), Box<dyn Error>>;

fn assert_approx(actual: f64, expected: f64, tol: f64) {
    assert!((actual - expected).abs() <= tol, "expected {expected} (+/- {tol}), got {actual}");
}

fn empty_grid(rows: usize, cols: usize) -> Vec<Vec<String>> {
    vec![vec![String::new(); cols]; rows]
}

fn empty_x_grid(rows: usize, cols: usize) -> Vec<Vec<Option<f64>>> {
    vec![vec![None; cols]; rows]
}

// ============================================================================
// 1. ImportFilterSpec::row_match in apply_import_link_pipeline
// ============================================================================

#[test]
fn import_pipeline_row_match_filtering_drop_keep_numeric_and_oob() -> TestResult {
    // Delimited kinetic multi-channel payload with padded discriminator column at col 1 after col_window.
    let csv = "DropMe,Channel,Time,Signal\n\
               x, Ch1 ,0,10.0\n\
               x,Ch2,0,90.0\n\
               x,Ch1,10,20.0\n\
               x,Ch1,20,30.0\n\
               x,Ch2,10,80.0\n\
               x,Ch1,30,40.0";

    let mut target = DataSheet::new("dst-rm", "Filtered", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?;
    let spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec {
            row_window: Some((1, 6)),
            col_window: Some((1, 3)),
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("  Ch1  "), drop_column: true }),
            decimate_every_k: Some(2),
            ..ImportFilterSpec::default()
        },
        placement: ImportPlacementSpec::default(),
    };
    let report = apply_import_link_pipeline(
        &mut target,
        &spec,
        ImportSourcePayload::DelimitedText { text: csv, delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )?;

    // 4 Ch1 rows kept, decimated by 2 -> rows at Time=0 and Time=20; discriminator dropped -> 2 cols.
    assert_eq!(report.rows_placed, 2);
    assert_eq!(report.cols_placed, 2);
    assert_eq!(target.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(0.0));
    assert_eq!(target.cell(CellCoord::new(0, 1, 0)).and_then(Cell::active_number), Some(10.0));
    assert_eq!(target.cell(CellCoord::new(1, 0, 0)).and_then(Cell::active_number), Some(20.0));
    assert_eq!(target.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(30.0));

    // Compact numeric matching + drop_column == false preserves column.
    let num_csv = "450,1.5\n520,2.5\n450,3.5";
    let mut num_target =
        DataSheet::new("dst-num", "NumMatch", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 1)?))?;
    let num_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec {
            row_match: Some(ColumnEqualsFilter { column: 0, equals: String::from("450"), drop_column: false }),
            ..ImportFilterSpec::default()
        },
        placement: ImportPlacementSpec::default(),
    };
    let num_report = apply_import_link_pipeline(
        &mut num_target,
        &num_spec,
        ImportSourcePayload::DelimitedText { text: num_csv, delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )?;
    assert_eq!(num_report.rows_placed, 2);
    assert_eq!(num_report.cols_placed, 2);
    assert_eq!(num_target.cell(CellCoord::new(0, 0, 0)).and_then(Cell::active_number), Some(450.0));
    assert_eq!(num_target.cell(CellCoord::new(1, 1, 0)).and_then(Cell::active_number), Some(3.5));

    // Out-of-bounds column on non-empty grid returns TransformError::InvalidSpec.
    let oob_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec { row_match: Some(ColumnEqualsFilter::new(5, "450")), ..ImportFilterSpec::default() },
        placement: ImportPlacementSpec::default(),
    };
    let err = apply_import_link_pipeline(
        &mut num_target,
        &oob_spec,
        ImportSourcePayload::DelimitedText { text: num_csv, delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    );
    assert!(matches!(err, Err(TransformError::InvalidSpec { .. })));
    Ok(())
}

// ============================================================================
// 2. build_plate_template_from_grids & extract_plate_grid_from_datasheet
// ============================================================================

#[test]
fn build_plate_template_from_grids_validation_zero_based_replicates_and_literal_labels() -> TestResult {
    // 9x13 and 17x25 rejected with explicit header-exclusion hint.
    let grid_9x13 = empty_grid(9, 13);
    let err_9x13 = build_plate_template_from_grids(&grid_9x13, None, &PlateGridBuildOptions::default());
    let Err(TransformError::InvalidSpec { message: msg_9x13 }) = err_9x13 else {
        return Err("expected InvalidSpec for 9x13".into());
    };
    assert!(msg_9x13.contains("9x13") && msg_9x13.contains("header"), "unexpected 9x13 message: {msg_9x13}");

    let grid_17x25 = empty_grid(17, 25);
    let err_17x25 = build_plate_template_from_grids(&grid_17x25, None, &PlateGridBuildOptions::default());
    let Err(TransformError::InvalidSpec { message: msg_17x25 }) = err_17x25 else {
        return Err("expected InvalidSpec for 17x25".into());
    };
    assert!(msg_17x25.contains("17x25") && msg_17x25.contains("header"), "unexpected 17x25 message: {msg_17x25}");

    // 8x12 with literal labels, case-insensitive blanks/controls, empty excluded, zero-based replicates.
    let mut labels = empty_grid(8, 12);
    labels[0][0] = String::from("  BUFFER ");
    labels[0][1] = String::from("buffer");
    labels[0][2] = String::from(" Pos Ctrl ");
    // Literal label containing bracketed concentration syntax must NOT be inline-parsed.
    labels[1][0] = String::from("Compound [10 uM]");
    labels[1][1] = String::from("Compound [10 uM]");
    labels[2][0] = String::from("Compound [10 uM]");

    let opts_col = PlateGridBuildOptions {
        name: String::from("Screen 96"),
        x_title: String::from("LogConc"),
        blank_labels: vec![String::from("Buffer")],
        positive_control_labels: vec![String::from("pos ctrl")],
        target_kind: TargetArchetypeKind::Column,
    };
    let (tpl_col, warns_col) = build_plate_template_from_grids(&labels, None, &opts_col)?;
    assert_eq!(tpl_col.geometry, PlateGeometry::Wells96);
    assert!(warns_col.is_empty());
    assert_eq!(tpl_col.well(0, 0)?, &PlateWellRole::Blank);
    assert_eq!(tpl_col.well(0, 1)?, &PlateWellRole::Blank);
    assert_eq!(tpl_col.well(0, 2)?, &PlateWellRole::PositiveControl);
    assert_eq!(tpl_col.well(0, 3)?, &PlateWellRole::Excluded);
    assert_eq!(tpl_col.well(1, 0)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: 0.0, replicate: 0 });
    assert_eq!(tpl_col.well(1, 1)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: 0.0, replicate: 1 });
    assert_eq!(tpl_col.well(2, 0)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: 0.0, replicate: 2 });

    // Omitting x_grid for Xy/Grouped emits non-blocking structural warning.
    let opts_xy = PlateGridBuildOptions { target_kind: TargetArchetypeKind::Xy, ..opts_col.clone() };
    let (_, warns_xy) = build_plate_template_from_grids(&labels, None, &opts_xy)?;
    assert!(!warns_xy.is_empty());

    // Supplying x_grid: Blank/PositiveControl/Excluded may have None; Treatment with None or NaN fails naming well + label.
    let mut x_grid = empty_x_grid(8, 12);
    x_grid[1][0] = Some(-6.0);
    x_grid[1][1] = Some(-6.0);
    x_grid[2][0] = None; // missing at C01!
    let err_missing_x = build_plate_template_from_grids(&labels, Some(&x_grid), &opts_xy);
    let Err(TransformError::InvalidInput { message: msg_missing }) = err_missing_x else {
        return Err("expected InvalidInput for missing x_grid on Treatment well".into());
    };
    assert!(msg_missing.contains("C01") && msg_missing.contains("Compound [10 uM]"), "unexpected msg: {msg_missing}");

    x_grid[2][0] = Some(f64::NAN);
    let err_nan_x = build_plate_template_from_grids(&labels, Some(&x_grid), &opts_xy);
    let Err(TransformError::InvalidInput { message: msg_nan }) = err_nan_x else {
        return Err("expected InvalidInput for NaN x_grid on Treatment well".into());
    };
    assert!(msg_nan.contains("C01") && msg_nan.contains("Compound [10 uM]"), "unexpected msg: {msg_nan}");

    x_grid[2][0] = Some(-7.0);
    let (tpl_with_x, warns_with_x) = build_plate_template_from_grids(&labels, Some(&x_grid), &opts_xy)?;
    assert!(warns_with_x.is_empty());
    assert_eq!(tpl_with_x.well(1, 0)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: -6.0, replicate: 0 });
    assert_eq!(tpl_with_x.well(1, 1)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: -6.0, replicate: 1 });
    assert_eq!(tpl_with_x.well(2, 0)?, &PlateWellRole::Treatment { group_name: String::from("Compound [10 uM]"), dilution_x: -7.0, replicate: 0 });

    // Geometry mismatch between 8x12 label_grid and 16x24 x_grid is rejected.
    let x_384 = empty_x_grid(16, 24);
    assert!(matches!(build_plate_template_from_grids(&labels, Some(&x_384), &opts_xy), Err(TransformError::InvalidSpec { .. })));

    // 384-well 16x24 grid succeeds cleanly.
    let mut labels_384 = empty_grid(16, 24);
    labels_384[15][23] = String::from("Lead-384");
    let (tpl_384, _) = build_plate_template_from_grids(&labels_384, None, &opts_col)?;
    assert_eq!(tpl_384.geometry, PlateGeometry::Wells384);
    assert_eq!(tpl_384.well(15, 23)?, &PlateWellRole::Treatment { group_name: String::from("Lead-384"), dilution_x: 0.0, replicate: 0 });
    Ok(())
}

#[test]
fn extract_plate_grid_from_datasheet_reads_data_cells_only_and_checks_geometry() -> TestResult {
    // 1. MultipleVariables 8x12 text layout sheet with non-empty column & row titles.
    let mut mv = MultipleVariablesTable::new(12, 8)?;
    for c in 0..12 {
        mv.variables[c].title = format!("ColHeader{}", c + 1);
    }
    mv.row_titles = (0..8).map(|r| format!("RowHeader{r}")).collect();
    mv.variables[0].cells[0] = Cell::text("Buffer");
    mv.variables[11].cells[7] = Cell::text("Compound-Z");
    let mv_sheet = DataSheet::new("layout-mv", "Layout", TableArchetype::MultipleVariables(mv))?;
    let (geom, grid) = extract_plate_grid_from_datasheet(&mv_sheet)?;
    assert_eq!(geom, PlateGeometry::Wells96);
    assert_eq!(grid.len(), 8);
    assert_eq!(grid[0].len(), 12);
    assert_eq!(grid[0][0], "Buffer");
    assert_eq!(grid[7][11], "Compound-Z");
    // Column & row titles must NOT leak into grid cells.
    assert!(!grid.iter().flatten().any(|s| s.contains("Header")));

    // 2. Numeric 8x12 imported as XY (1 X column + 11 Y columns = 12 data columns total).
    let mut xy = XyTable::new(SubcolumnMode::Single, 11, 8)?;
    xy.x_title = String::from("IgnoreTitle");
    for r in 0..8 {
        xy.x_cells[r][0] = Cell::number(-6.0 - (r as f64));
        for c in 0..11 {
            xy.data_sets[c].cells[r][0] = Cell::number(-6.0 - (r as f64) - 0.1 * ((c + 1) as f64));
        }
    }
    let xy_sheet = DataSheet::new("x-grid-xy", "X Grid", TableArchetype::Xy(xy))?;
    let (x_geom, x_grid) = extract_plate_x_grid_from_datasheet(&xy_sheet)?;
    assert_eq!(x_geom, PlateGeometry::Wells96);
    assert_approx(x_grid[0][0].ok_or("x[0][0]")?, -6.0, 1e-12);
    assert_approx(x_grid[7][11].ok_or("x[7][11]")?, -14.1, 1e-12);

    // 3. 9x13 sheet fails with explicit header-exclusion hint.
    let mv_9x13 = MultipleVariablesTable::new(13, 9)?;
    let sheet_9x13 = DataSheet::new("bad-9x13", "With Headers", TableArchetype::MultipleVariables(mv_9x13))?;
    let err_9x13 = extract_plate_grid_from_datasheet(&sheet_9x13);
    let Err(TransformError::InvalidSpec { message }) = err_9x13 else {
        return Err("expected InvalidSpec for 9x13 datasheet".into());
    };
    assert!(message.contains("9x13") && message.contains("header"), "unexpected message: {message}");
    Ok(())
}

// ============================================================================
// 3. extract_plate_values_from_analyzable_sheet (WellPerRow & WellPerColumn)
// ============================================================================

#[test]
fn extract_plate_values_well_per_row_shuffled_missing_excluded_and_collisions() -> TestResult {
    // Simulated AreaUnderCurve analyzable_data MultipleVariables table:
    // shuffled wells, delimiter-prefixed IDs, missing G11, excluded B02, non-well row "Summary".
    let rows = vec![
        ("H12", Some(88.0), false),
        ("G12 - Drug B", Some(77.0), false),
        ("A01 - Sample X1", Some(10.0), false),
        ("A2_rep1", Some(12.0), false),
        ("B01:ch1", Some(21.0), false),
        ("B02", Some(999.0), true),   // excluded -> skipped -> None
        ("C01", None, false),         // unavailable -> None
        ("A100", Some(500.0), false), // NOT a valid 96-well ID -> ignored with warning
        ("Summary", Some(42.0), false),
    ];
    let mut mv = MultipleVariablesTable::new(3, rows.len())?;
    mv.variables[0].title = String::from("Series");
    mv.variables[1].title = String::from("Positive Area");
    mv.variables[2].title = String::from("Net Area");
    for (r, (series, val, excl)) in rows.iter().enumerate() {
        mv.variables[0].cells[r] = Cell::text(*series);
        mv.variables[1].cells[r] = Cell::number(1.0);
        mv.variables[2].cells[r] = match (*val, *excl) {
            (Some(v), true) => Cell::excluded_number(v),
            (Some(v), false) => Cell::number(v),
            (None, _) => Cell::empty(),
        };
    }
    let sheet = DataSheet::new("auc-data", "AUC Output", TableArchetype::MultipleVariables(mv))?;

    let extracted = extract_plate_values_from_analyzable_sheet(
        &sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerRow { value_column: String::from("  net area ") },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(extracted.values.len(), 96);
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(0, 0)?], Some(10.0)); // A01
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(0, 1)?], Some(12.0)); // A02
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(1, 0)?], Some(21.0)); // B01
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(1, 1)?], None); // B02 excluded
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(2, 0)?], None); // C01 empty
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(0, 9)?], None); // A10 must NOT match A100
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(6, 10)?], None); // G11 missing
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(6, 11)?], Some(77.0)); // G12
    assert_eq!(extracted.values[PlateGeometry::Wells96.well_index(7, 11)?], Some(88.0)); // H12
    assert!(extracted.warnings.iter().any(|w| w.contains("missing")));
    assert!(extracted.warnings.iter().any(|w| w.contains("B02")));
    assert!(extracted.warnings.iter().any(|w| w.contains("A100")));

    // Duplicate well-ID collision ("A01" + "A1 - dup") fails cleanly.
    let mut dup_mv = MultipleVariablesTable::new(2, 2)?;
    dup_mv.variables[0].title = String::from("Well");
    dup_mv.variables[1].title = String::from("Net Area");
    dup_mv.variables[0].cells[0] = Cell::text("A01");
    dup_mv.variables[1].cells[0] = Cell::number(1.0);
    dup_mv.variables[0].cells[1] = Cell::text("A1 - dup");
    dup_mv.variables[1].cells[1] = Cell::number(2.0);
    let dup_sheet = DataSheet::new("dup-mv", "Dup", TableArchetype::MultipleVariables(dup_mv))?;
    let dup_err = extract_plate_values_from_analyzable_sheet(
        &dup_sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerRow { value_column: String::from("Net Area") },
        PlateWellMatchMode::ByWellId,
    );
    let Err(TransformError::InvalidInput { message }) = dup_err else {
        return Err("expected duplicate well collision error".into());
    };
    assert!(message.contains("A01"), "unexpected duplicate error: {message}");

    // Column sheet WellPerRow using row_titles + dataset title, and Positional mode.
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    col_tbl.data_sets[0].title = String::from("Signal");
    col_tbl.row_titles = vec![String::from("H12"), String::from("A01")];
    col_tbl.data_sets[0].cells[0][0] = Cell::number(55.0);
    col_tbl.data_sets[0].cells[1][0] = Cell::number(11.0);
    let col_sheet = DataSheet::new("col-wpr", "ColWpr", TableArchetype::Column(col_tbl))?;

    let by_id = extract_plate_values_from_analyzable_sheet(
        &col_sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerRow { value_column: String::from("signal") },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(by_id.values[95], Some(55.0));
    assert_eq!(by_id.values[0], Some(11.0));

    let pos = extract_plate_values_from_analyzable_sheet(
        &col_sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerRow { value_column: String::from("Signal") },
        PlateWellMatchMode::Positional,
    )?;
    assert_eq!(pos.values[0], Some(55.0));
    assert_eq!(pos.values[1], Some(11.0));
    Ok(())
}

#[test]
fn extract_plate_values_well_per_column_kinetic_reducers_skip_excluded_cells() -> TestResult {
    // Kinetic XY sheet with times 0, 10, 20, 30, 40 and shuffled well columns + excluded artifacts.
    let mut xy = XyTable::new(SubcolumnMode::Single, 4, 5)?;
    let times = [0.0, 10.0, 20.0, 30.0, 40.0];
    for (r, &t) in times.iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(t);
    }
    xy.data_sets[0].title = String::from("H12");
    xy.data_sets[0].cells = vec![
        vec![Cell::number(10.0)],
        vec![Cell::number(20.0)],
        vec![Cell::excluded_number(999_999.0)], // excluded spike must be skipped by Mean, Max, RowIndex
        vec![Cell::number(40.0)],
        vec![Cell::excluded_number(-500.0)], // excluded trough must be skipped by Min
    ];
    xy.data_sets[1].title = String::from("A01 - Agonist");
    xy.data_sets[1].cells =
        vec![vec![Cell::number(5.0)], vec![Cell::number(15.0)], vec![Cell::number(25.0)], vec![Cell::number(35.0)], vec![Cell::number(45.0)]];
    xy.data_sets[2].title = String::from("G12_rep");
    xy.data_sets[2].cells =
        vec![vec![Cell::number(2.0)], vec![Cell::number(4.0)], vec![Cell::number(6.0)], vec![Cell::number(8.0)], vec![Cell::number(10.0)]];
    xy.data_sets[3].title = String::from("ReferenceMean"); // non-well column -> ignored with warning
    xy.data_sets[3].cells = vec![vec![Cell::number(1.0)]; 5];

    let sheet = DataSheet::new("kin-xy", "Kinetic", TableArchetype::Xy(xy))?;

    // XWindowMean [10.0, 30.0]: H12 averages (20 + 40) / 2 = 30.0 (row 2 excluded); A01 averages (15 + 25 + 35) / 3 = 25.0.
    let win = extract_plate_values_from_analyzable_sheet(
        &sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::XWindowMean { x_min: 10.0, x_max: 30.0 } },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_approx(win.values[95].ok_or("H12")?, 30.0, 1e-12);
    assert_approx(win.values[0].ok_or("A01")?, 25.0, 1e-12);
    assert_approx(win.values[83].ok_or("G12")?, 6.0, 1e-12);
    assert_eq!(win.values[82], None); // G11 missing

    // RowIndex(2): H12 excluded -> None; A01 -> 25.0.
    let row2 = extract_plate_values_from_analyzable_sheet(
        &sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::RowIndex(2) },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(row2.values[95], None);
    assert_eq!(row2.values[0], Some(25.0));

    // Min & Max skip excluded cells.
    let mins = extract_plate_values_from_analyzable_sheet(
        &sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::Min },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(mins.values[95], Some(10.0));
    assert_eq!(mins.values[0], Some(5.0));

    let maxs = extract_plate_values_from_analyzable_sheet(
        &sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::Max },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(maxs.values[95], Some(40.0));
    assert_eq!(maxs.values[0], Some(45.0));

    // Invalid XWindowMean bounds and XWindowMean on Column table rejected cleanly.
    assert!(matches!(
        extract_plate_values_from_analyzable_sheet(
            &sheet,
            PlateGeometry::Wells96,
            &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::XWindowMean { x_min: 30.0, x_max: 10.0 } },
            PlateWellMatchMode::ByWellId,
        ),
        Err(TransformError::InvalidSpec { .. })
    ));
    let mut col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2)?;
    col_tbl.data_sets[0].title = String::from("A01");
    col_tbl.data_sets[0].cells[0][0] = Cell::number(3.0);
    col_tbl.data_sets[0].cells[1][0] = Cell::number(9.0);
    let col_sheet = DataSheet::new("col-kin", "ColKin", TableArchetype::Column(col_tbl))?;
    assert!(matches!(
        extract_plate_values_from_analyzable_sheet(
            &col_sheet,
            PlateGeometry::Wells96,
            &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::XWindowMean { x_min: 0.0, x_max: 10.0 } },
            PlateWellMatchMode::ByWellId,
        ),
        Err(TransformError::IncompatibleTable { .. })
    ));
    let col_max = extract_plate_values_from_analyzable_sheet(
        &col_sheet,
        PlateGeometry::Wells96,
        &PlateSourceExtraction::WellPerColumn { reducer: PlateKineticReducer::Max },
        PlateWellMatchMode::ByWellId,
    )?;
    assert_eq!(col_max.values[0], Some(9.0));
    Ok(())
}

// ============================================================================
// 4. plate_well_origins across all 4 archetypes + full mutation lifecycle
// ============================================================================

#[test]
fn plate_map_populates_origins_and_well_cell_map_across_all_four_archetypes() -> TestResult {
    let mut tpl = PlateMapTemplate::new("CRC Plate", PlateGeometry::Wells96, "LogConc")?;
    let mut vals = vec![None; 96];

    // Blank at A01, PositiveControl at A02, Excluded with value at A07, Excluded empty at A08.
    tpl.set_well(0, 0, PlateWellRole::Blank)?;
    vals[PlateGeometry::Wells96.well_index(0, 0)?] = Some(10.0);
    tpl.set_well(0, 1, PlateWellRole::PositiveControl)?;
    vals[PlateGeometry::Wells96.well_index(0, 1)?] = Some(200.0);
    tpl.set_well(0, 6, PlateWellRole::Excluded)?;
    vals[PlateGeometry::Wells96.well_index(0, 6)?] = Some(999.0);
    tpl.set_well(0, 7, PlateWellRole::Excluded)?;
    vals[PlateGeometry::Wells96.well_index(0, 7)?] = None;

    // Treatment arm "Agonist": G11 missing (None), G12 populated (rep 1 at x=-6), H12 populated (rep 0 at x=-7).
    tpl.set_well(6, 10, PlateWellRole::Treatment { group_name: String::from("Agonist"), dilution_x: -6.0, replicate: 0 })?;
    vals[PlateGeometry::Wells96.well_index(6, 10)?] = None; // G11 missing!
    tpl.set_well(6, 11, PlateWellRole::Treatment { group_name: String::from("Agonist"), dilution_x: -6.0, replicate: 1 })?;
    vals[PlateGeometry::Wells96.well_index(6, 11)?] = Some(120.0);
    tpl.set_well(7, 11, PlateWellRole::Treatment { group_name: String::from("Agonist"), dilution_x: -7.0, replicate: 0 })?;
    vals[PlateGeometry::Wells96.well_index(7, 11)?] = Some(80.0);

    // 1. XY: rows sorted ascending by X (-7 at row 0, -6 at row 1); G12 at (1, 0, 1), H12 at (0, 0, 0).
    let xy_out = map_plate_to_datasheet_with_origins("XY", PlateGeometry::Wells96, &vals, &tpl, TargetArchetypeKind::Xy, false)?;
    assert_eq!(xy_out.sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("H12"));
    assert_eq!(xy_out.sheet.plate_well_origin(CellCoord::new(1, 0, 1)), Some("G12"));
    assert_eq!(xy_out.sheet.plate_well_origin(CellCoord::new(1, 0, 0)), None); // G11 missing
    assert_eq!(xy_out.well_cell_map.len(), 2);
    xy_out.sheet.validate()?;

    // 2. Grouped: same treatment grid layout as XY.
    let grp_out = map_plate_to_datasheet_with_origins("Grouped", PlateGeometry::Wells96, &vals, &tpl, TargetArchetypeKind::Grouped, false)?;
    assert_eq!(grp_out.sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("H12"));
    assert_eq!(grp_out.sheet.plate_well_origin(CellCoord::new(1, 0, 1)), Some("G12"));
    grp_out.sheet.validate()?;

    // 3. Column (subtract_blank_mean = false): Agonist (col 0: H12, G12), Positive Control (col 1: A02), Blank (col 2: A01), Excluded (col 3: A07).
    let col_out = map_plate_to_datasheet_with_origins("Col", PlateGeometry::Wells96, &vals, &tpl, TargetArchetypeKind::Column, false)?;
    assert_eq!(col_out.sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("H12"));
    assert_eq!(col_out.sheet.plate_well_origin(CellCoord::new(1, 0, 0)), Some("G12"));
    assert_eq!(col_out.sheet.plate_well_origin(CellCoord::new(0, 1, 0)), Some("A02"));
    assert_eq!(col_out.sheet.plate_well_origin(CellCoord::new(0, 2, 0)), Some("A01"));
    assert_eq!(col_out.sheet.plate_well_origin(CellCoord::new(0, 3, 0)), Some("A07"));
    assert_eq!(col_out.well_cell_map.len(), 5);
    col_out.sheet.validate()?;

    // 4. MultipleVariables: Value column col=7 carries origins for every populated/excluded-with-value well (not empty wells).
    let mv_out = map_plate_to_datasheet_with_origins("MV", PlateGeometry::Wells96, &vals, &tpl, TargetArchetypeKind::MultipleVariables, false)?;
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(0, 7, 0)), Some("A01"));
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(1, 7, 0)), Some("A02"));
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(6, 7, 0)), Some("A07"));
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(7, 7, 0)), None); // A08 empty
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(82, 7, 0)), None); // G11 empty
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(83, 7, 0)), Some("G12"));
    assert_eq!(mv_out.sheet.plate_well_origin(CellCoord::new(95, 7, 0)), Some("H12"));
    assert_eq!(mv_out.well_cell_map.len(), 5);
    mv_out.sheet.validate()?;
    Ok(())
}

fn make_origin_column_sheet() -> Result<DataSheet, Box<dyn Error>> {
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4)?;
    col.data_sets[0].title = String::from("DrugA");
    col.data_sets[1].title = String::from("DrugB");
    for r in 0..4 {
        col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 10.0);
        col.data_sets[1].cells[r][0] = Cell::number((r + 1) as f64 * 100.0);
    }
    let mut sheet = DataSheet::new("orig-col", "Plate Col", TableArchetype::Column(col))?;
    sheet.plate_well_origins = vec![
        PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("A01") },
        PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("B01") },
        PlateCellOrigin { coord: CellCoord::new(2, 0, 0), well: String::from("G12") },
        PlateCellOrigin { coord: CellCoord::new(3, 0, 0), well: String::from("H12") },
        PlateCellOrigin { coord: CellCoord::new(0, 1, 0), well: String::from("A02") },
        PlateCellOrigin { coord: CellCoord::new(2, 1, 0), well: String::from("C02") },
    ];
    sheet.plate_well_origins.sort_by_key(|o| o.coord);
    sheet.set_excluded_with_reason(CellCoord::new(2, 0, 0), "outlier", "well G12 bubble", "")?;
    Ok(sheet)
}

#[test]
fn plate_well_origins_lifecycle_import_exclude_and_convert() -> TestResult {
    let sheet = make_origin_column_sheet()?;
    // Excluding G12 preserves its plate well origin.
    assert_eq!(sheet.plate_well_origin(CellCoord::new(2, 0, 0)), Some("G12"));

    // PlacementMode::InsertRows at row 1 (inserts 2 rows): row 0 stays at 0, row 2 (G12) shifts to row 4.
    let mut inserted = sheet.clone();
    let insert_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::InsertRows, anchor: CellCoord::new(1, 0, 0), ..ImportPlacementSpec::default() },
    };
    apply_import_link_pipeline(
        &mut inserted,
        &insert_spec,
        ImportSourcePayload::DelimitedText { text: "1,2\n3,4", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )?;
    assert_eq!(inserted.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    assert_eq!(inserted.plate_well_origin(CellCoord::new(1, 0, 0)), None);
    assert_eq!(inserted.plate_well_origin(CellCoord::new(3, 0, 0)), Some("B01"));
    assert_eq!(inserted.plate_well_origin(CellCoord::new(4, 0, 0)), Some("G12"));
    assert_eq!(inserted.plate_well_origin(CellCoord::new(5, 0, 0)), Some("H12"));
    assert_eq!(inserted.plate_well_origin(CellCoord::new(4, 1, 0)), Some("C02"));
    inserted.validate()?;

    // PlacementMode::Overwrite removes overwritten origin while preserving untouched origins.
    let mut overwritten = sheet.clone();
    let ov_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, anchor: CellCoord::new(0, 0, 0), ..ImportPlacementSpec::default() },
    };
    apply_import_link_pipeline(
        &mut overwritten,
        &ov_spec,
        ImportSourcePayload::DelimitedText { text: "999", delimiter: ',', decimal_separator: '.' },
        &ImportPipelineOptions::default(),
        None,
    )?;
    assert_eq!(overwritten.plate_well_origin(CellCoord::new(0, 0, 0)), None);
    assert_eq!(overwritten.plate_well_origin(CellCoord::new(2, 0, 0)), Some("G12"));
    overwritten.validate()?;

    // convert_datasheet_archetype: Column -> Grouped preserves all origins; Column -> Xy drops col 0 cleanly without inflating lossy.
    let (as_grouped, grp_prev) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Grouped, false)?;
    assert!(!grp_prev.lossy);
    assert_eq!(as_grouped.plate_well_origin(CellCoord::new(2, 0, 0)), Some("G12"));
    assert_eq!(as_grouped.plate_well_origin(CellCoord::new(2, 1, 0)), Some("C02"));
    as_grouped.validate()?;

    let mut clean_col_sheet = sheet.clone();
    clean_col_sheet.set_excluded(CellCoord::new(2, 0, 0), false)?;
    let (as_xy, xy_prev) = convert_datasheet_archetype(&clean_col_sheet, TargetArchetypeKind::Xy, false)?;
    assert!(!xy_prev.lossy, "unmapped plate_well_origins on col 0 moved to X must not inflate dropped_notes/lossy");
    assert_eq!(as_xy.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A02"));
    assert_eq!(as_xy.plate_well_origin(CellCoord::new(2, 0, 0)), Some("C02"));
    assert_eq!(as_xy.plate_well_origins.len(), 2);
    as_xy.validate()?;
    Ok(())
}

#[test]
fn plate_well_origins_lifecycle_transpose_prune_select_and_sample() -> TestResult {
    let sheet = make_origin_column_sheet()?;
    let mut clean_col_sheet = sheet.clone();
    clean_col_sheet.set_excluded(CellCoord::new(2, 0, 0), false)?;
    let (as_xy, _) = convert_datasheet_archetype(&clean_col_sheet, TargetArchetypeKind::Xy, false)?;
    let ctx = TransformContext::default();

    // Transpose: Column (row=2, col=0) -> (row=0, col=2), plus XY RawReplicates(2) transpose.
    let tx = execute_transpose(&sheet, true, &ctx)?;
    assert_eq!(tx.data_sheet.plate_well_origin(CellCoord::new(0, 2, 0)), Some("G12"));
    assert_eq!(tx.data_sheet.plate_well_origin(CellCoord::new(1, 2, 0)), Some("C02"));
    tx.data_sheet.validate()?;

    let mut xy_rep = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 2)?;
    xy_rep.x_cells[0][0] = Cell::number(1.0);
    xy_rep.x_cells[1][0] = Cell::number(2.0);
    xy_rep.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(11.0)];
    xy_rep.data_sets[0].cells[1] = vec![Cell::number(20.0), Cell::number(21.0)];
    let mut xy_rep_sheet = DataSheet::new("xy-rep", "XY Rep", TableArchetype::Xy(xy_rep))?;
    xy_rep_sheet.plate_well_origins = vec![
        PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("A01") },
        PlateCellOrigin { coord: CellCoord::new(0, 0, 1), well: String::from("A02") },
        PlateCellOrigin { coord: CellCoord::new(1, 0, 1), well: String::from("B02") },
    ];
    let tx_xy_rep = execute_transpose(&xy_rep_sheet, true, &ctx)?;
    assert_eq!(tx_xy_rep.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    assert_eq!(tx_xy_rep.data_sheet.plate_well_origin(CellCoord::new(1, 0, 0)), Some("A02"));
    assert_eq!(tx_xy_rep.data_sheet.plate_well_origin(CellCoord::new(1, 1, 0)), Some("B02"));
    tx_xy_rep.data_sheet.validate()?;

    // Prune: EveryNth(2), LogarithmicSpacing, MinMaxBins remap; AverageBins(2) clears origins.
    let nth = execute_prune(&sheet, PruneMethod::EveryNth(2), &ctx)?;
    assert_eq!(nth.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    assert_eq!(nth.data_sheet.plate_well_origin(CellCoord::new(1, 0, 0)), Some("G12"));
    assert_eq!(nth.data_sheet.plate_well_origin(CellCoord::new(1, 1, 0)), Some("C02"));
    assert_eq!(nth.data_sheet.plate_well_origin(CellCoord::new(0, 1, 0)), Some("A02"));
    nth.data_sheet.validate()?;

    let log_pruned = execute_prune(&as_xy, PruneMethod::LogarithmicSpacing { target_points: 2 }, &ctx)?;
    assert_eq!(log_pruned.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A02"));
    log_pruned.data_sheet.validate()?;

    let minmax_pruned = execute_prune(&sheet, PruneMethod::MinMaxBins(2), &ctx)?;
    assert_eq!(minmax_pruned.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    minmax_pruned.data_sheet.validate()?;

    let avg = execute_prune(&sheet, PruneMethod::AverageBins(2), &ctx)?;
    assert!(avg.data_sheet.plate_well_origins.is_empty(), "AverageBins must clear plate_well_origins");
    avg.data_sheet.validate()?;

    // SelectAndTransform: row/col subset + new/overwritten derived column + cross-archetype projection.
    let sel_ctx = TransformContext { selected_data_sets: vec![1, 0], ..TransformContext::default() };
    let sel = execute_select_with_derived(
        &sheet,
        &[SelectRule::RowRange(1, 3)],
        &[DerivedColumnFormula { title: String::from("Calc"), formula: String::from("[DrugA] * 2") }],
        TableKind::Column,
        &sel_ctx,
    )?;
    assert_eq!(sel.data_sheet.plate_well_origin(CellCoord::new(1, 0, 0)), Some("C02"));
    assert_eq!(sel.data_sheet.plate_well_origin(CellCoord::new(1, 1, 0)), Some("G12"));
    assert_eq!(sel.data_sheet.plate_well_origin(CellCoord::new(1, 2, 0)), None);
    sel.data_sheet.validate()?;

    let col_to_xy_sel = execute_select_with_derived(&sheet, &[], &[], TableKind::Xy, &ctx)?;
    assert_eq!(col_to_xy_sel.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A02"));
    assert_eq!(col_to_xy_sel.data_sheet.plate_well_origin(CellCoord::new(2, 0, 0)), Some("C02"));
    assert_eq!(col_to_xy_sel.data_sheet.plate_well_origins.len(), 2);
    col_to_xy_sel.data_sheet.validate()?;

    let xy_to_col_sel = execute_select_with_derived(&as_xy, &[], &[], TableKind::Column, &ctx)?;
    assert_eq!(xy_to_col_sel.data_sheet.plate_well_origin(CellCoord::new(0, 1, 0)), Some("A02"));
    assert_eq!(xy_to_col_sel.data_sheet.plate_well_origin(CellCoord::new(2, 1, 0)), Some("C02"));
    xy_to_col_sel.data_sheet.validate()?;

    let overwrite_derived = execute_select_with_derived(
        &sheet,
        &[],
        &[DerivedColumnFormula { title: String::from("DrugA"), formula: String::from("[DrugA] + 1") }],
        TableKind::Column,
        &ctx,
    )?;
    assert_eq!(overwrite_derived.data_sheet.plate_well_origin(CellCoord::new(2, 0, 0)), None);
    assert_eq!(overwrite_derived.data_sheet.plate_well_origin(CellCoord::new(2, 1, 0)), Some("C02"));
    overwrite_derived.data_sheet.validate()?;

    // RandomSample remaps kept row origins accurately.
    let sampled = random_sample_rows(
        &sheet,
        &RowSampleConfig { seed: 42, sample_size: 2, stratify_column: None, proportional_allocation: false, output_id: None, output_title: None },
    )?;
    assert!(!sampled.plate_well_origins.is_empty());
    assert!(sampled.plate_well_origins.iter().all(|o| o.coord.row < 2));
    sampled.validate()?;
    Ok(())
}

#[test]
fn plate_well_origins_lifecycle_math_normalize_and_baseline_error_swap() -> TestResult {
    let ctx = TransformContext::default();
    let mut err_grp = GroupedTable::new(SubcolumnMode::UpperLowerLimits, 1, 2)?;
    err_grp.data_sets[0].title = String::from("Limits");
    err_grp.data_sets[0].cells[0] = vec![Cell::number(10.0), Cell::number(8.0), Cell::number(12.0)];
    err_grp.data_sets[0].cells[1] = vec![Cell::number(-5.0), Cell::number(-7.0), Cell::number(-3.0)];
    let mut err_sheet = DataSheet::new("err-grp", "ErrBounds", TableArchetype::Grouped(err_grp))?;
    err_sheet.plate_well_origins = vec![
        PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("A01") },
        PlateCellOrigin { coord: CellCoord::new(0, 0, 1), well: String::from("A02") },
        PlateCellOrigin { coord: CellCoord::new(0, 0, 2), well: String::from("A03") },
        PlateCellOrigin { coord: CellCoord::new(1, 0, 0), well: String::from("B01") },
    ];

    // Negating flips lower (8 -> -8) and upper (12 -> -12), swapping subcol 1 <-> 2.
    let neg_math = execute_math_op(&err_sheet, &MathTransformOp::MultiplyByConstant(-1.0), ErrorPropagationMode::ReplicateEndpoints, &ctx)?;
    assert_eq!(neg_math.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    assert_eq!(neg_math.data_sheet.plate_well_origin(CellCoord::new(0, 0, 1)), Some("A03"));
    assert_eq!(neg_math.data_sheet.plate_well_origin(CellCoord::new(0, 0, 2)), Some("A02"));
    neg_math.data_sheet.validate()?;

    // Ln on row 1 (-5.0) blanks cell -> origin for B01 pruned, while A01..A03 preserved.
    let ln_math = execute_math_op(&err_sheet, &MathTransformOp::Ln, ErrorPropagationMode::ReplicateEndpoints, &ctx)?;
    assert_eq!(ln_math.data_sheet.plate_well_origin(CellCoord::new(0, 0, 0)), Some("A01"));
    assert_eq!(ln_math.data_sheet.plate_well_origin(CellCoord::new(1, 0, 0)), None);
    ln_math.data_sheet.validate()?;

    // Decreasing Normalize (zero=100, hundred=0) reverses bounds -> swaps subcol 1 <-> 2 origins.
    let norm = execute_normalize(&err_sheet, &NormalizeAnchor::FixedValue(100.0), &NormalizeAnchor::FixedValue(0.0), true, false, &ctx)?;
    assert_eq!(norm.data_sheet.plate_well_origin(CellCoord::new(0, 0, 1)), Some("A03"));
    assert_eq!(norm.data_sheet.plate_well_origin(CellCoord::new(0, 0, 2)), Some("A02"));
    norm.data_sheet.validate()?;

    // RemoveBaseline Ratio with negative constant baseline (-2.0) reverses bounds -> swaps subcol 1 <-> 2 origins.
    let base = execute_remove_baseline_kind(&err_sheet, &BaselineKind::Constant(-2.0), BaselineOperation::Ratio, &ctx)?;
    assert_eq!(base.data_sheet.plate_well_origin(CellCoord::new(0, 0, 1)), Some("A03"));
    assert_eq!(base.data_sheet.plate_well_origin(CellCoord::new(0, 0, 2)), Some("A02"));
    base.data_sheet.validate()?;
    Ok(())
}
