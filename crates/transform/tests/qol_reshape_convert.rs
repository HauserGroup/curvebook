use curvebook_doc::{
    Cell, CellCoord, CellExclusionNote, CellValue, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DocError, FloatingNote, GroupedTable,
    MultipleVariablesTable, RectPt, SheetId, SubcolumnMode, TableArchetype, TableKind, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::{
    PlateGeometry, PlateMapTemplate, PlateWellRole, TargetArchetypeKind, TidyColumnMapping, TransformError, convert_datasheet_archetype,
    map_plate_to_datasheet, pivot_tidy_long_to_datasheet, preview_datasheet_conversion,
};

fn raw_number(cell: &Cell) -> Option<f64> {
    match cell.value {
        CellValue::Number(v) => Some(v),
        _ => None,
    }
}

#[test]
fn qol1_column_grouped_roundtrip_preserves_metadata_exclusions_and_notes() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, 3, 4).expect("column");
    col.row_titles = vec!["Baseline".into(), "1h".into(), "4h".into(), "24h".into()];
    col.data_sets[0].title = "Vehicle".into();
    col.data_sets[1].title = "LowDose".into();
    col.data_sets[2].title = "HighDose".into();
    for r in 0..4 {
        for c in 0..3 {
            col.data_sets[c].cells[r][0] = Cell::number((r * 10 + c + 1) as f64);
        }
    }

    let mut sheet = DataSheet::new("sheet-col-grp", "Kinetic Response", TableArchetype::Column(col)).expect("sheet");
    sheet.info_sheet_id = Some(SheetId::new("info-study-42"));
    sheet.color_tag = Some(FloatingNote::DEFAULT_FILL);
    sheet.floating_notes.push(FloatingNote::new("note-1", RectPt::new(10.0, 20.0, 120.0, 60.0), "Sample kinetic plate"));
    sheet.set_excluded_with_reason(CellCoord::new(2, 1, 0), "bubble", "Optical bubble at 4h LowDose", "2026-11-14T08:00:00Z").expect("exclude");

    let preview = preview_datasheet_conversion(&sheet, TargetArchetypeKind::Grouped).expect("preview");
    assert_eq!(preview.source_kind, TableKind::Column);
    assert_eq!(preview.target_kind, TargetArchetypeKind::Grouped);
    assert!(!preview.lossy);
    assert_eq!(preview.dropped_cells, 0);
    assert_eq!(preview.preserved_exclusions, 1);

    let (grouped_sheet, conv_preview) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Grouped, false).expect("col->grouped");
    assert!(!conv_preview.lossy);
    assert_eq!(grouped_sheet.id, SheetId::new("sheet-col-grp"));
    assert_eq!(grouped_sheet.title, "Kinetic Response");
    assert_eq!(grouped_sheet.info_sheet_id, sheet.info_sheet_id);
    assert_eq!(grouped_sheet.color_tag, sheet.color_tag);
    assert_eq!(grouped_sheet.floating_notes, sheet.floating_notes);
    assert_eq!(grouped_sheet.exclusion_notes.len(), 1);
    assert_eq!(grouped_sheet.exclusion_notes[0].coord, CellCoord::new(2, 1, 0));
    assert_eq!(grouped_sheet.exclusion_notes[0].reason_tag, "bubble");
    assert!(grouped_sheet.cell(CellCoord::new(2, 1, 0)).expect("cell").excluded);

    let (back_col, back_preview) = convert_datasheet_archetype(&grouped_sheet, TargetArchetypeKind::Column, false).expect("grouped->col");
    assert!(!back_preview.lossy);
    assert_eq!(back_col.exclusion_notes, sheet.exclusion_notes);
    assert_eq!(back_col.cell(CellCoord::new(2, 1, 0)).expect("cell"), sheet.cell(CellCoord::new(2, 1, 0)).expect("orig"));
}

#[test]
fn qol1_grouped_replicates_to_column_flattens_and_remaps_exclusion_note() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).expect("grouped");
    grp.row_titles = vec!["1nM".into(), "10nM".into(), "100nM".into()];
    grp.data_sets[0].title = "WT".into();
    grp.data_sets[1].title = "KO".into();
    for r in 0..3 {
        for c in 0..2 {
            for s in 0..2 {
                grp.data_sets[c].cells[r][s] = Cell::number((r * 100 + c * 10 + s + 1) as f64);
            }
        }
    }

    let mut sheet = DataSheet::new("grp-reps", "Replicate Assay", TableArchetype::Grouped(grp)).expect("sheet");
    sheet.set_excluded_with_reason(CellCoord::new(1, 1, 1), "outlier", "KO 10nM rep2 pipetting error", "2026-11-14T09:00:00Z").expect("exclude");

    let (col_sheet, preview) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Column, false).expect("grouped->col");
    assert!(!preview.lossy);
    assert_eq!(preview.preserved_exclusions, 1);

    let TableArchetype::Column(col_tbl) = &col_sheet.archetype else {
        panic!("expected Column archetype");
    };
    assert_eq!(col_tbl.data_sets.len(), 6);
    assert_eq!(col_tbl.data_sets[4].title, "KO - 10nM");
    assert_eq!(col_sheet.exclusion_notes.len(), 1);
    let remapped_coord = col_sheet.exclusion_notes[0].coord;
    assert_eq!(remapped_coord, CellCoord::new(1, 4, 0));
    let remapped_cell = col_sheet.cell(remapped_coord).expect("remapped cell");
    assert!(remapped_cell.excluded);
    assert_eq!(raw_number(remapped_cell), Some(112.0));
}

#[test]
fn qol1_column_xy_bidirectional_and_exclusion_coord_remap() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 3, 3).expect("column");
    col.data_sets[0].title = "Conc_uM".into();
    col.data_sets[1].title = "Control".into();
    col.data_sets[2].title = "Treated".into();
    for (r, x) in [0.1, 1.0, 10.0].into_iter().enumerate() {
        col.data_sets[0].cells[r][0] = Cell::number(x);
        col.data_sets[1].cells[r][0] = Cell::number(100.0 - x * 2.0);
        col.data_sets[2].cells[r][0] = Cell::number(95.0 - x * 6.0);
    }

    let mut sheet = DataSheet::new("col-xy-1", "DoseCurve", TableArchetype::Column(col)).expect("sheet");
    sheet
        .set_excluded_with_reason(CellCoord::new(2, 2, 0), "precipitation", "Compound precipitated at 10 uM", "2026-11-14T09:10:00Z")
        .expect("exclude");

    let (xy_sheet, xy_prev) = convert_datasheet_archetype(&sheet, TargetArchetypeKind::Xy, false).expect("col->xy");
    assert!(!xy_prev.lossy);
    assert_eq!(xy_prev.preserved_exclusions, 1);

    let TableArchetype::Xy(xy) = &xy_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy.x_title, "Conc_uM");
    assert_eq!(xy.data_sets.len(), 2);
    assert_eq!(xy_sheet.exclusion_notes.len(), 1);
    assert_eq!(xy_sheet.exclusion_notes[0].coord, CellCoord::new(2, 1, 0));
    assert!(xy_sheet.cell(CellCoord::new(2, 1, 0)).expect("y cell").excluded);

    let (col_back, back_prev) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::Column, false).expect("xy->col");
    assert!(!back_prev.lossy);
    assert_eq!(col_back.exclusion_notes.len(), 1);
    assert_eq!(col_back.exclusion_notes[0].coord, CellCoord::new(2, 2, 0));
    assert!(col_back.cell(CellCoord::new(2, 2, 0)).expect("col cell").excluded);

    // Single-column Column -> Xy synthesizes X cleanly
    let mut single_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 1, 3).expect("single");
    single_col.data_sets[0].title = "Signal".into();
    for r in 0..3 {
        single_col.data_sets[0].cells[r][0] = Cell::number((r + 1) as f64 * 5.0);
    }
    let single_sheet = DataSheet::new("single-col", "Single", TableArchetype::Column(single_col)).expect("sheet");
    let (single_xy, single_prev) = convert_datasheet_archetype(&single_sheet, TargetArchetypeKind::Xy, false).expect("single->xy");
    assert!(!single_prev.lossy);
    let TableArchetype::Xy(sxy) = &single_xy.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(sxy.x_cells[0][0].active_number(), Some(1.0));
    assert_eq!(sxy.data_sets[0].cells[2][0].active_number(), Some(15.0));
}

#[test]
fn qol1_grouped_and_xy_with_multiple_variables_roundtrip_and_remap_notes() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).expect("grouped");
    grp.row_factor_title = "Media".into();
    grp.column_factor_title = "Genotype".into();
    grp.row_titles = vec!["DMEM".into(), "RPMI".into()];
    grp.data_sets[0].title = "Control".into();
    grp.data_sets[1].title = "Mutant".into();
    grp.data_sets[0].cells[0][0] = Cell::number(11.0);
    grp.data_sets[0].cells[0][1] = Cell::number(12.0);
    grp.data_sets[0].cells[1][0] = Cell::number(21.0);
    grp.data_sets[1].cells[0][0] = Cell::number(31.0);
    grp.data_sets[1].cells[1][0] = Cell::number(41.0);
    grp.data_sets[1].cells[1][1] = Cell::number(42.0);

    let mut grp_sheet = DataSheet::new("grp-mv", "Media x Genotype", TableArchetype::Grouped(grp)).expect("sheet");
    grp_sheet.set_excluded_with_reason(CellCoord::new(1, 1, 1), "qc", "RPMI Mutant rep 2 contaminated", "2026-11-14T09:15:00Z").expect("exclude");

    let (mv_sheet, mv_prev) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::MultipleVariables, false).expect("grp->mv");
    assert!(!mv_prev.lossy);
    assert_eq!(mv_prev.preserved_exclusions, 1);
    assert_eq!(mv_sheet.exclusion_notes.len(), 1);
    let mv_excl_coord = mv_sheet.exclusion_notes[0].coord;
    let mv_excl_cell = mv_sheet.cell(mv_excl_coord).expect("mv cell");
    assert!(mv_excl_cell.excluded);
    assert_eq!(raw_number(mv_excl_cell), Some(42.0));

    let (grp_back, back_prev) = convert_datasheet_archetype(&mv_sheet, TargetArchetypeKind::Grouped, false).expect("mv->grp");
    assert!(!back_prev.lossy);
    assert_eq!(grp_back.exclusion_notes.len(), 1);
    let back_coord = grp_back.exclusion_notes[0].coord;
    let back_cell = grp_back.cell(back_coord).expect("back cell");
    assert!(back_cell.excluded);
    assert_eq!(raw_number(back_cell), Some(42.0));

    // Xy <-> MultipleVariables round-trip
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 1, 3).expect("xy");
    xy.x_title = "Time_min".into();
    xy.data_sets[0].title = "Fluorescence".into();
    for (r, t) in [0.0, 5.0, 10.0].into_iter().enumerate() {
        xy.x_cells[r][0] = Cell::number(t);
        xy.data_sets[0].cells[r][0] = Cell::number(100.0 + t);
        xy.data_sets[0].cells[r][1] = Cell::number(102.0 + t);
    }
    let mut xy_sheet = DataSheet::new("xy-mv", "Fluorescence Trace", TableArchetype::Xy(xy)).expect("sheet");
    xy_sheet
        .set_excluded_with_reason(CellCoord::new(1, 0, 1), "spike", "Detector saturation at 5 min rep 2", "2026-11-14T09:20:00Z")
        .expect("exclude");

    let (xy_to_mv, p1) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::MultipleVariables, false).expect("xy->mv");
    assert!(!p1.lossy);
    assert_eq!(xy_to_mv.exclusion_notes.len(), 1);
    let mv_c = xy_to_mv.exclusion_notes[0].coord;
    let mv_c_cell = xy_to_mv.cell(mv_c).expect("mv cell");
    assert_eq!(raw_number(mv_c_cell), Some(107.0));
    assert!(mv_c_cell.excluded);

    let (mv_to_xy, p2) = convert_datasheet_archetype(&xy_to_mv, TargetArchetypeKind::Xy, false).expect("mv->xy");
    assert!(!p2.lossy);
    assert_eq!(mv_to_xy.exclusion_notes.len(), 1);
    assert_eq!(mv_to_xy.exclusion_notes[0].coord, CellCoord::new(1, 0, 1));
    let xy_c_cell = mv_to_xy.cell(CellCoord::new(1, 0, 1)).expect("xy cell");
    assert_eq!(raw_number(xy_c_cell), Some(107.0));
    assert!(xy_c_cell.excluded);
}

#[test]
fn qol1_lossy_guard_and_unsupported_archetype() {
    // Column with text inside column 0 -> Xy is lossy (text X cannot become numeric X)
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 2, 2).expect("col");
    col.data_sets[0].title = "LabelX".into();
    col.data_sets[1].title = "Measure".into();
    col.data_sets[0].cells[0][0] = Cell::text("Alpha");
    col.data_sets[0].cells[1][0] = Cell::text("Beta");
    col.data_sets[1].cells[0][0] = Cell::number(5.0);
    col.data_sets[1].cells[1][0] = Cell::number(9.0);
    let col_sheet = DataSheet::new("lossy-col", "Lossy", TableArchetype::Column(col)).expect("sheet");

    let preview = preview_datasheet_conversion(&col_sheet, TargetArchetypeKind::Xy).expect("preview");
    assert!(preview.lossy);
    assert_eq!(preview.dropped_cells, 2);
    assert!(!preview.warnings.is_empty());

    let err = convert_datasheet_archetype(&col_sheet, TargetArchetypeKind::Xy, false).expect_err("must refuse lossy");
    assert!(matches!(err, TransformError::InvalidSpec { .. }));

    let (forced, forced_prev) = convert_datasheet_archetype(&col_sheet, TargetArchetypeKind::Xy, true).expect("allow lossy");
    assert!(forced_prev.lossy);
    assert_eq!(forced.kind(), TableKind::Xy);

    // MultipleVariables with extra factor/numeric columns -> Grouped lossy guard
    let mut mv = MultipleVariablesTable::new(5, 2).expect("mv");
    for (i, title) in ["Factor1", "Factor2", "Factor3", "MeasureA", "MeasureB"].iter().enumerate() {
        mv.variables[i].title = (*title).into();
    }
    for r in 0..2 {
        mv.variables[0].cells[r] = Cell::text("Row");
        mv.variables[1].cells[r] = Cell::text("Col");
        mv.variables[2].cells[r] = Cell::text("Extra");
        mv.variables[3].cells[r] = Cell::number((r + 1) as f64);
        mv.variables[4].cells[r] = Cell::number((r + 10) as f64);
    }
    for v in &mut mv.variables {
        v.refresh_inferred_role();
    }
    let mv_sheet = DataSheet::new("lossy-mv", "WideMV", TableArchetype::MultipleVariables(mv)).expect("mv sheet");
    let mv_err = convert_datasheet_archetype(&mv_sheet, TargetArchetypeKind::Grouped, false).expect_err("must refuse lossy MV");
    assert!(matches!(mv_err, TransformError::InvalidSpec { .. }));
    let (_, mv_forced) = convert_datasheet_archetype(&mv_sheet, TargetArchetypeKind::Grouped, true).expect("allow lossy MV");
    assert!(mv_forced.lossy);
    assert!(mv_forced.dropped_cells > 0);

    // Unsupported source archetype -> IncompatibleTable
    let cont = ContingencyTable::new(2, 2).expect("cont");
    let cont_sheet = DataSheet::new("cont", "Contingency", TableArchetype::Contingency(cont)).expect("sheet");
    let compat_err = convert_datasheet_archetype(&cont_sheet, TargetArchetypeKind::Xy, true).expect_err("unsupported");
    assert!(matches!(compat_err, TransformError::IncompatibleTable { kind: TableKind::Contingency, .. }));
}

#[test]
fn qol2_plate_96_ic50_to_xy_blank_subtraction_excluded_and_cbtemplate_roundtrip() {
    let mut template = PlateMapTemplate::new("96-Well IC50", PlateGeometry::Wells96, "Concentration_nM").expect("template");
    let mut values = vec![None; PlateGeometry::Wells96.well_count()];

    // Row A: Blank (cols 0..4), PositiveControl (cols 4..6), Excluded spiked wells (cols 6..8)
    let blanks = [100.0, 102.0, 98.0, 100.0];
    for (c, v) in blanks.into_iter().enumerate() {
        template.set_well(0, c, PlateWellRole::Blank).expect("blank");
        values[PlateGeometry::Wells96.well_index(0, c).expect("idx")] = Some(v);
    }
    for (offset, v) in [1200.0, 1220.0].into_iter().enumerate() {
        let c = 4 + offset;
        template.set_well(0, c, PlateWellRole::PositiveControl).expect("pos ctrl");
        values[PlateGeometry::Wells96.well_index(0, c).expect("idx")] = Some(v);
    }
    for (offset, v) in [99_999.0, -500.0].into_iter().enumerate() {
        let c = 6 + offset;
        template.set_well(0, c, PlateWellRole::Excluded).expect("excluded");
        values[PlateGeometry::Wells96.well_index(0, c).expect("idx")] = Some(v);
    }

    // 2 compounds across 8 dilutions (cols 0..8), 2 replicates each:
    // Compound-A in rows B, C; Compound-B in rows D, E.
    let concs = [0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0, 10_000.0];
    for (col, &conc) in concs.iter().enumerate() {
        for rep in 0..2 {
            let row_a = 1 + rep;
            template
                .set_well(row_a, col, PlateWellRole::Treatment { group_name: "Compound-A".into(), dilution_x: conc, replicate: rep + 1 })
                .expect("set A");
            values[PlateGeometry::Wells96.well_index(row_a, col).expect("idx")] = Some(1000.0 - (col as f64) * 90.0 + (rep as f64) * 4.0);

            let row_b = 3 + rep;
            template
                .set_well(row_b, col, PlateWellRole::Treatment { group_name: "Compound-B".into(), dilution_x: conc, replicate: rep + 1 })
                .expect("set B");
            values[PlateGeometry::Wells96.well_index(row_b, col).expect("idx")] = Some(950.0 - (col as f64) * 75.0 + (rep as f64) * 6.0);
        }
    }

    // `.cbtemplate` JSON round-trip
    let json = template.to_cbtemplate_json().expect("to json");
    let reloaded = PlateMapTemplate::from_cbtemplate_json(&json).expect("from json");
    assert_eq!(reloaded, template);

    let xy_sheet = map_plate_to_datasheet("IC50 Screen", PlateGeometry::Wells96, &values, &reloaded, TargetArchetypeKind::Xy, true).expect("map Xy");
    let TableArchetype::Xy(xy) = &xy_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy.x_title, "Concentration_nM");
    assert_eq!(xy.data_sets.len(), 2);
    assert_eq!(xy.data_sets[0].title, "Compound-A");
    assert_eq!(xy.data_sets[1].title, "Compound-B");
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert_eq!(xy.x_cells.len(), 8);
    // Blank mean = 100.0 even with Excluded well = 99999.0; Compound-A row 0 rep 1 = 1000.0 - 100.0 = 900.0
    assert!((xy.data_sets[0].cells[0][0].active_number().expect("y") - 900.0).abs() < 1e-10);
    assert!((xy.data_sets[0].cells[0][1].active_number().expect("y") - 904.0).abs() < 1e-10);
    // QC floating note records blank mean and excluded wells without corrupting curves
    assert!(!xy_sheet.floating_notes.is_empty());
    let qc_body = &xy_sheet.floating_notes[0].body_markdown;
    assert!(qc_body.contains("Blank mean subtracted: 100"));
    assert!(qc_body.contains("A07=99999"));

    // Column projection preserves Excluded wells as Cell::excluded_number + CellExclusionNote
    let col_sheet =
        map_plate_to_datasheet("IC50 Column", PlateGeometry::Wells96, &values, &reloaded, TargetArchetypeKind::Column, true).expect("map Column");
    assert!(col_sheet.exclusion_notes.iter().any(|n| n.note.contains("A07")));
    let TableArchetype::Column(col_tbl) = &col_sheet.archetype else {
        panic!("expected Column");
    };
    let excl_col = col_tbl.data_sets.iter().find(|ds| ds.title == "Excluded").expect("Excluded col");
    assert!(excl_col.cells[0][0].excluded);
    assert_eq!(raw_number(&excl_col.cells[0][0]), Some(99_999.0));
}

#[test]
fn qol2_plate_384_dimensions_grouped_and_multiple_variables() {
    assert_eq!(PlateGeometry::Wells384.rows(), 16);
    assert_eq!(PlateGeometry::Wells384.cols(), 24);
    assert_eq!(PlateGeometry::Wells384.well_count(), 384);
    assert_eq!(PlateGeometry::Wells384.well_label(15, 23).expect("P24"), "P24");

    let mut template = PlateMapTemplate::new("384 Assay", PlateGeometry::Wells384, "Dose").expect("384 template");
    let mut values = vec![Some(10.0); 384];

    for c in 0..4 {
        template.set_well(0, c, PlateWellRole::Blank).expect("blank");
        values[PlateGeometry::Wells384.well_index(0, c).expect("idx")] = Some(5.0);
    }
    template.set_well(15, 23, PlateWellRole::Excluded).expect("P24 excluded");
    values[PlateGeometry::Wells384.well_index(15, 23).expect("idx")] = Some(777.0);

    for (r, conc) in [1.0, 10.0, 100.0].into_iter().enumerate() {
        let plate_r = 1 + r;
        for rep in 0..3 {
            template
                .set_well(plate_r, rep, PlateWellRole::Treatment { group_name: "Lead-1".into(), dilution_x: conc, replicate: rep + 1 })
                .expect("lead");
            values[PlateGeometry::Wells384.well_index(plate_r, rep).expect("idx")] = Some(50.0 + (r * 10 + rep) as f64);
        }
    }

    let mv_sheet = map_plate_to_datasheet("384 Long", PlateGeometry::Wells384, &values, &template, TargetArchetypeKind::MultipleVariables, true)
        .expect("384 MV");
    let TableArchetype::MultipleVariables(mv) = &mv_sheet.archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables.len(), 8);
    assert_eq!(mv.variables[0].cells.len(), 384);
    assert_eq!(mv.variables[0].cells[383].active_text(), Some("P24"));
    assert!(mv.variables[7].cells[383].excluded);
    assert!(mv_sheet.exclusion_notes.iter().any(|n| n.coord == CellCoord::new(383, 7, 0)));

    let grp_sheet =
        map_plate_to_datasheet("384 Grouped", PlateGeometry::Wells384, &values, &template, TargetArchetypeKind::Grouped, true).expect("384 Grouped");
    let TableArchetype::Grouped(grp) = &grp_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp.subcolumn_mode, SubcolumnMode::RawReplicates(3));
    assert_eq!(grp.data_sets[0].title, "Lead-1");
    assert_eq!(grp.row_titles, vec!["1", "10", "100"]);
    // 50.0 - blank_mean(5.0) = 45.0
    assert_eq!(grp.data_sets[0].cells[0][0].active_number(), Some(45.0));
}

#[test]
fn qol2_tidy_long_pivots_column_grouped_xy_with_decimal_comma_and_exclusions() {
    let headers = vec!["Condition".into(), "Dose".into(), "Rep".into(), "Signal".into(), "Flag".into()];
    let rows = vec![
        vec!["Ctrl".into(), "0,5".into(), "1".into(), "12,5".into(), "false".into()],
        vec!["Ctrl".into(), "0,5".into(), "2".into(), "13,25*".into(), "".into()],
        vec!["Ctrl".into(), "2,0".into(), "1".into(), "20,0".into(), "0".into()],
        vec!["Drug".into(), "0,5".into(), "1".into(), "8,5".into(), "true".into()],
        vec!["Drug".into(), "0,5".into(), "2".into(), "9,0".into(), "no".into()],
        vec!["Drug".into(), "2,0".into(), "1".into(), "4,75".into(), "hemolyzed".into()],
    ];
    let mapping = TidyColumnMapping { x_col: Some(1), group_col: 0, replicate_col: Some(2), value_col: 3, excluded_col: Some(4) };

    // Column pivot
    let col_sheet = pivot_tidy_long_to_datasheet("Tidy Col", &headers, &rows, &mapping, TargetArchetypeKind::Column, ',').expect("tidy->col");
    let TableArchetype::Column(col_tbl) = &col_sheet.archetype else {
        panic!("expected Column");
    };
    assert_eq!(col_tbl.data_sets.len(), 2);
    assert_eq!(col_tbl.data_sets[0].cells[0][0].active_number(), Some(12.5));
    assert!(col_tbl.data_sets[0].cells[1][0].excluded);
    assert_eq!(raw_number(&col_tbl.data_sets[0].cells[1][0]), Some(13.25));
    assert!(col_tbl.data_sets[1].cells[0][0].excluded);
    assert!(col_tbl.data_sets[1].cells[2][0].excluded);
    assert_eq!(col_sheet.exclusion_notes.len(), 3);
    assert!(col_sheet.exclusion_notes.iter().any(|n| n.note == "hemolyzed"));

    // Grouped pivot with replicate subcolumns
    let grp_sheet = pivot_tidy_long_to_datasheet("Tidy Grouped", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, ',').expect("tidy->grp");
    let TableArchetype::Grouped(grp_tbl) = &grp_sheet.archetype else {
        panic!("expected Grouped");
    };
    assert_eq!(grp_tbl.subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert_eq!(grp_tbl.row_titles, vec!["0,5", "2,0"]);
    assert_eq!(grp_tbl.data_sets[0].title, "Ctrl");
    assert_eq!(grp_tbl.data_sets[1].title, "Drug");
    assert_eq!(grp_tbl.data_sets[0].cells[0][0].active_number(), Some(12.5));
    assert!(grp_tbl.data_sets[0].cells[0][1].excluded);
    assert_eq!(raw_number(&grp_tbl.data_sets[0].cells[0][1]), Some(13.25));
    assert_eq!(grp_tbl.data_sets[1].cells[0][1].active_number(), Some(9.0));
    assert_eq!(grp_sheet.exclusion_notes.len(), 3);

    // Xy pivot with numeric X + replicate subcolumns
    let xy_sheet = pivot_tidy_long_to_datasheet("Tidy XY", &headers, &rows, &mapping, TargetArchetypeKind::Xy, ',').expect("tidy->xy");
    let TableArchetype::Xy(xy_tbl) = &xy_sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy_tbl.x_title, "Dose");
    assert_eq!(xy_tbl.x_cells[0][0].active_number(), Some(0.5));
    assert_eq!(xy_tbl.x_cells[1][0].active_number(), Some(2.0));
    assert_eq!(xy_tbl.data_sets[0].cells[0][0].active_number(), Some(12.5));
    assert!(xy_tbl.data_sets[1].cells[1][0].excluded);
    assert_eq!(xy_sheet.exclusion_notes.len(), 3);
}

#[test]
fn qol2_tidy_long_ragged_and_missing_inputs_do_not_panic() {
    let headers = vec!["Group".into(), "X".into(), "Value".into()];
    let ragged_rows = vec![
        vec![],
        vec!["   ".into()],
        vec!["GroupA".into()],
        vec!["GroupA".into(), "1.0".into(), "10.0".into()],
        vec!["GroupA".into(), "".into(), "20.0".into()],
        vec!["GroupA".into(), "2.0".into(), "NA".into()],
        vec!["GroupB".into(), "1.0".into(), "15.5".into()],
        vec!["GroupB".into(), "2.0".into()],
    ];
    let mapping = TidyColumnMapping { x_col: Some(1), group_col: 0, replicate_col: None, value_col: 2, excluded_col: None };

    let col = pivot_tidy_long_to_datasheet("Ragged Col", &headers, &ragged_rows, &mapping, TargetArchetypeKind::Column, '.').expect("ragged column");
    assert_eq!(col.kind(), TableKind::Column);

    let grp =
        pivot_tidy_long_to_datasheet("Ragged Grouped", &headers, &ragged_rows, &mapping, TargetArchetypeKind::Grouped, '.').expect("ragged grouped");
    assert_eq!(grp.kind(), TableKind::Grouped);

    let xy = pivot_tidy_long_to_datasheet("Ragged XY", &headers, &ragged_rows, &mapping, TargetArchetypeKind::Xy, '.').expect("ragged xy");
    let TableArchetype::Xy(xy_tbl) = &xy.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(xy_tbl.x_cells.len(), 2);
    assert_eq!(xy_tbl.data_sets[0].cells[0][0].active_number(), Some(10.0));
    assert!(xy_tbl.data_sets[0].cells[1][0].is_empty());

    // Pivoting to Xy without x_col returns InvalidSpec cleanly
    let no_x = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 2, excluded_col: None };
    let err = pivot_tidy_long_to_datasheet("Missing XCol", &headers, &ragged_rows, &no_x, TargetArchetypeKind::Xy, '.').expect_err("requires x_col");
    assert!(matches!(err, TransformError::InvalidSpec { .. }));
}

#[test]
fn qol1_column_and_grouped_to_contingency_preserve_labels_and_exclusions() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 2, 2).expect("col");
    col.data_sets[0].title = "Exposed".into();
    col.data_sets[1].title = "Unexposed".into();
    col.row_titles = vec!["Outcome+".into(), "Outcome-".into()];
    col.data_sets[0].cells[0][0] = Cell::number(18.0);
    col.data_sets[0].cells[1][0] = Cell::number(7.0);
    col.data_sets[1].cells[0][0] = Cell::number(4.0);
    col.data_sets[1].cells[1][0] = Cell { value: CellValue::Number(21.0), excluded: true };
    let mut col_sheet = DataSheet::new("col-cnt", "Counts", TableArchetype::Column(col)).expect("sheet");
    col_sheet.exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(1, 1, 0),
        reason_tag: "Audit".into(),
        note: "Pending recount".into(),
        timestamp_utc: "2026-01-01T00:00:00Z".into(),
    });

    let prev = preview_datasheet_conversion(&col_sheet, TargetArchetypeKind::Contingency).expect("preview");
    assert!(!prev.lossy);
    assert_eq!(prev.target_kind, TargetArchetypeKind::Contingency);
    assert_eq!(prev.preserved_cells, 4);
    assert_eq!(prev.preserved_exclusions, 1);

    let (cont_sheet, conv_prev) = convert_datasheet_archetype(&col_sheet, TargetArchetypeKind::Contingency, false).expect("col->cont");
    assert!(!conv_prev.lossy);
    assert_eq!(cont_sheet.kind(), TableKind::Contingency);
    assert_eq!(cont_sheet.exclusion_notes.len(), 1);
    let TableArchetype::Contingency(cont) = &cont_sheet.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(cont.row_titles, vec!["Outcome+", "Outcome-"]);
    assert_eq!(cont.data_sets[0].title, "Exposed");
    assert_eq!(cont.data_sets[1].title, "Unexposed");
    assert_eq!(cont.count_at(0, 0).unwrap(), Some(18));
    assert_eq!(cont.count_at(1, 1).unwrap(), None);

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).expect("grp");
    grp.row_factor_title = "Response".into();
    grp.column_factor_title = "Arm".into();
    grp.row_titles = vec!["Responder".into(), "NonResponder".into()];
    grp.data_sets[0].title = "Drug".into();
    grp.data_sets[1].title = "Placebo".into();
    grp.data_sets[0].cells[0] = vec![Cell::number(12.0), Cell::number(99.0)];
    grp.data_sets[0].cells[1] = vec![Cell::number(8.0), Cell::empty()];
    grp.data_sets[1].cells[0] = vec![Cell::number(5.0), Cell::empty()];
    grp.data_sets[1].cells[1] = vec![Cell::number(15.0), Cell::empty()];
    let grp_sheet = DataSheet::new("grp-cnt", "Grouped", TableArchetype::Grouped(grp)).expect("grp");
    assert!(matches!(convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Contingency, false), Err(TransformError::InvalidSpec { .. })));
    let (grp_cont, grp_prev) = convert_datasheet_archetype(&grp_sheet, TargetArchetypeKind::Contingency, true).expect("grp->cont");
    assert!(grp_prev.lossy);
    assert_eq!(grp_prev.dropped_cells, 1);
    let TableArchetype::Contingency(g_cont) = &grp_cont.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(g_cont.row_factor_title, "Response");
    assert_eq!(g_cont.column_factor_title, "Arm");
    assert_eq!(g_cont.count_at(0, 0).unwrap(), Some(12));
}

#[test]
fn qol1_mv_to_contingency_and_invalid_count_rejection() {
    let mut mv_counts = MultipleVariablesTable::new(3, 2).expect("mv counts");
    mv_counts.variables[0].title = "Genotype".into();
    mv_counts.variables[0].role = VariableRole::Label;
    mv_counts.variables[0].role_inferred = false;
    mv_counts.variables[1].title = "Cases".into();
    mv_counts.variables[2].title = "Controls".into();
    mv_counts.variables[0].cells = vec![Cell::text("WildType"), Cell::text("Mutant")];
    mv_counts.variables[1].cells = vec![Cell::number(10.0), Cell::number(25.0)];
    mv_counts.variables[2].cells = vec![Cell::number(30.0), Cell::number(15.0)];
    let mv_counts_sheet = DataSheet::new("mv-counts", "MVCounts", TableArchetype::MultipleVariables(mv_counts)).expect("sheet");
    let (mv_cont_sheet, mv_prev) = convert_datasheet_archetype(&mv_counts_sheet, TargetArchetypeKind::Contingency, false).expect("mv->cont");
    assert!(!mv_prev.lossy);
    let TableArchetype::Contingency(mv_cont) = &mv_cont_sheet.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(mv_cont.row_factor_title, "Genotype");
    assert_eq!(mv_cont.row_titles, vec!["WildType", "Mutant"]);
    assert_eq!(mv_cont.data_sets[0].title, "Cases");
    assert_eq!(mv_cont.data_sets[1].title, "Controls");
    assert_eq!(mv_cont.count_at(1, 0).unwrap(), Some(25));

    let mut mv_cat = MultipleVariablesTable::new(2, 4).expect("mv cat");
    mv_cat.variables[0].title = "Treatment".into();
    mv_cat.variables[0].role = VariableRole::Categorical;
    mv_cat.variables[0].role_inferred = false;
    mv_cat.variables[1].title = "Status".into();
    mv_cat.variables[1].role = VariableRole::Categorical;
    mv_cat.variables[1].role_inferred = false;
    for (r, (t, s)) in [("Drug", "Cured"), ("Drug", "Cured"), ("Drug", "Relapse"), ("Ctrl", "Relapse")].into_iter().enumerate() {
        mv_cat.variables[0].cells[r] = Cell::text(t);
        mv_cat.variables[1].cells[r] = Cell::text(s);
    }
    let mv_cat_sheet = DataSheet::new("mv-cat", "MVCat", TableArchetype::MultipleVariables(mv_cat)).expect("sheet");
    let (xt_sheet, xt_prev) = convert_datasheet_archetype(&mv_cat_sheet, TargetArchetypeKind::Contingency, false).expect("mv cat->cont");
    assert!(!xt_prev.lossy);
    let TableArchetype::Contingency(xt) = &xt_sheet.archetype else {
        panic!("expected Contingency");
    };
    assert_eq!(xt.row_titles, vec!["Drug", "Ctrl"]);
    assert_eq!(xt.data_sets[0].title, "Cured");
    assert_eq!(xt.data_sets[1].title, "Relapse");
    assert_eq!(xt.count_at(0, 0).unwrap(), Some(2));
    assert_eq!(xt.count_at(1, 0).unwrap(), Some(0));

    for bad in [Cell::number(-1.0), Cell::number(2.5), Cell::text("bad")] {
        let mut bad_col = ColumnTable::new_with_subcolumns(ColumnDesignHint::UnpairedReplicates, SubcolumnMode::Single, 2, 2).expect("bad");
        bad_col.data_sets[0].cells[0][0] = Cell::number(5.0);
        bad_col.data_sets[0].cells[1][0] = bad;
        bad_col.data_sets[1].cells[0][0] = Cell::number(3.0);
        bad_col.data_sets[1].cells[1][0] = Cell::number(4.0);
        let bad_sheet = DataSheet::new("bad", "Bad", TableArchetype::Column(bad_col)).expect("sheet");
        let err = preview_datasheet_conversion(&bad_sheet, TargetArchetypeKind::Contingency).expect_err("must reject invalid count");
        assert!(matches!(err, TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. })));
    }
}
