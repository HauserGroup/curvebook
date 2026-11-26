use curvebook_doc::{
    CellCoord, ColumnDesignHint, ColumnTable, ContingencyTable, DataSheet, DocError, FloatingNote, GroupedTable, RectPt, SheetId, SubcolumnMode,
    TableArchetype, TableKind, VariableRole, XColumnMode, XyTable,
};
use curvebook_transform::{
    PlateGeometry, PlateMapTemplate, PlateWellRole, TargetArchetypeKind, TidyColumnMapping, TransformError, convert_datasheet_archetype,
    map_plate_to_datasheet, pivot_tidy_long_to_datasheet, preview_datasheet_conversion,
};

use crate::common::{
    add_excl, as_col, as_contingency, as_grouped, as_mv, as_xy, assert_lossy_convert, col_nums, col_sheet, empty, excl, excl_note, grouped_sheet,
    mv_sheet_inferred, mv_sheet_roles, num, raw_number, sheet, str_rows, str_vec, txt,
};

#[test]
fn qol1_column_grouped_roundtrip_preserves_metadata_exclusions_and_notes() {
    let mut col = ColumnTable::new_with_subcolumns(ColumnDesignHint::PairedRepeatedMeasures, SubcolumnMode::Single, 3, 4).unwrap();
    col.row_titles = str_vec(&["Baseline", "1h", "4h", "24h"]);
    for (c, t) in ["Vehicle", "LowDose", "HighDose"].into_iter().enumerate() {
        col.data_sets[c].title = t.into();
        for r in 0..4 {
            col.data_sets[c].cells[r][0] = num((r * 10 + c + 1) as f64);
        }
    }
    let mut s = DataSheet::new("sheet-col-grp", "Kinetic Response", TableArchetype::Column(col)).unwrap();
    s.info_sheet_id = Some(SheetId::new("info-study-42"));
    s.color_tag = Some(FloatingNote::DEFAULT_FILL);
    s.floating_notes.push(FloatingNote::new("note-1", RectPt::new(10.0, 20.0, 120.0, 60.0), "Sample kinetic plate"));
    add_excl(&mut s, CellCoord::new(2, 1, 0), "bubble", "Optical bubble at 4h LowDose");

    let preview = preview_datasheet_conversion(&s, TargetArchetypeKind::Grouped).unwrap();
    assert_eq!(
        (preview.source_kind, preview.target_kind, preview.lossy, preview.dropped_cells, preview.preserved_exclusions),
        (TableKind::Column, TargetArchetypeKind::Grouped, false, 0, 1)
    );

    let (grouped_sheet, conv_prev) = convert_datasheet_archetype(&s, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!conv_prev.lossy);
    assert_eq!(
        (&grouped_sheet.id, grouped_sheet.title.as_str(), &grouped_sheet.info_sheet_id, &grouped_sheet.color_tag, &grouped_sheet.floating_notes),
        (&SheetId::new("sheet-col-grp"), "Kinetic Response", &s.info_sheet_id, &s.color_tag, &s.floating_notes)
    );
    assert_eq!(grouped_sheet.exclusion_notes.len(), 1);
    assert_eq!((grouped_sheet.exclusion_notes[0].coord, grouped_sheet.exclusion_notes[0].reason_tag.as_str()), (CellCoord::new(2, 1, 0), "bubble"));
    assert!(grouped_sheet.cell(CellCoord::new(2, 1, 0)).unwrap().excluded);

    let (back_col, back_prev) = convert_datasheet_archetype(&grouped_sheet, TargetArchetypeKind::Column, false).unwrap();
    assert!(!back_prev.lossy);
    assert_eq!(back_col.exclusion_notes, s.exclusion_notes);
    assert_eq!(back_col.cell(CellCoord::new(2, 1, 0)).unwrap(), s.cell(CellCoord::new(2, 1, 0)).unwrap());
}

#[test]
fn qol1_grouped_replicates_to_column_flattens_and_remaps_exclusion_note() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    grp.row_titles = str_vec(&["1nM", "10nM", "100nM"]);
    (grp.data_sets[0].title, grp.data_sets[1].title) = ("WT".into(), "KO".into());
    for r in 0..3 {
        for c in 0..2 {
            for s in 0..2 {
                grp.data_sets[c].cells[r][s] = num((r * 100 + c * 10 + s + 1) as f64);
            }
        }
    }
    let mut s = sheet("grp-reps", TableArchetype::Grouped(grp));
    add_excl(&mut s, CellCoord::new(1, 1, 1), "outlier", "KO 10nM rep2 pipetting error");

    let (col_sheet, preview) = convert_datasheet_archetype(&s, TargetArchetypeKind::Column, false).unwrap();
    assert!(!preview.lossy && preview.preserved_exclusions == 1);
    let col_tbl = as_col(&col_sheet);
    assert_eq!((col_tbl.data_sets.len(), col_tbl.data_sets[4].title.as_str()), (6, "KO - 10nM"));
    assert_eq!(col_sheet.exclusion_notes.len(), 1);
    let coord = col_sheet.exclusion_notes[0].coord;
    assert_eq!(coord, CellCoord::new(1, 4, 0));
    let cell = col_sheet.cell(coord).unwrap();
    assert!(cell.excluded && raw_number(cell) == Some(112.0));
}

#[test]
fn qol1_column_xy_bidirectional_and_exclusion_coord_remap() {
    let mut s = col_nums("col-xy-1", &[("Conc_uM", &[0.1, 1.0, 10.0]), ("Control", &[99.8, 98.0, 80.0]), ("Treated", &[94.4, 89.0, 35.0])]);
    add_excl(&mut s, CellCoord::new(2, 2, 0), "precipitation", "Compound precipitated at 10 uM");

    let (xy_sheet, xy_prev) = convert_datasheet_archetype(&s, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!xy_prev.lossy && xy_prev.preserved_exclusions == 1);
    let xy = as_xy(&xy_sheet);
    assert_eq!((xy.x_title.as_str(), xy.data_sets.len(), xy_sheet.exclusion_notes.len()), ("Conc_uM", 2, 1));
    assert_eq!(xy_sheet.exclusion_notes[0].coord, CellCoord::new(2, 1, 0));
    assert!(xy_sheet.cell(CellCoord::new(2, 1, 0)).unwrap().excluded);

    let (col_back, back_prev) = convert_datasheet_archetype(&xy_sheet, TargetArchetypeKind::Column, false).unwrap();
    assert!(!back_prev.lossy && col_back.exclusion_notes.len() == 1);
    assert_eq!(col_back.exclusion_notes[0].coord, CellCoord::new(2, 2, 0));
    assert!(col_back.cell(CellCoord::new(2, 2, 0)).unwrap().excluded);

    // Single-column Column -> Xy synthesizes X cleanly.
    let single = col_nums("single-col", &[("Signal", &[5.0, 10.0, 15.0])]);
    let (single_xy, sp) = convert_datasheet_archetype(&single, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!sp.lossy);
    let sxy = as_xy(&single_xy);
    assert_eq!((sxy.x_cells[0][0].active_number(), sxy.data_sets[0].cells[2][0].active_number()), (Some(1.0), Some(15.0)));
}

#[test]
fn qol1_grouped_and_xy_with_multiple_variables_roundtrip_and_remap_notes() {
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).unwrap();
    (grp.row_factor_title, grp.column_factor_title) = ("Media".into(), "Genotype".into());
    grp.row_titles = str_vec(&["DMEM", "RPMI"]);
    (grp.data_sets[0].title, grp.data_sets[1].title) = ("Control".into(), "Mutant".into());
    grp.data_sets[0].cells = vec![vec![num(11.0), num(12.0)], vec![num(21.0), empty()]];
    grp.data_sets[1].cells = vec![vec![num(31.0), empty()], vec![num(41.0), num(42.0)]];
    let mut grp_s = sheet("grp-mv", TableArchetype::Grouped(grp));
    add_excl(&mut grp_s, CellCoord::new(1, 1, 1), "qc", "RPMI Mutant rep 2 contaminated");

    let (mv_s, mv_p) = convert_datasheet_archetype(&grp_s, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!mv_p.lossy && mv_p.preserved_exclusions == 1 && mv_s.exclusion_notes.len() == 1);
    let mv_c = mv_s.cell(mv_s.exclusion_notes[0].coord).unwrap();
    assert!(mv_c.excluded && raw_number(mv_c) == Some(42.0));

    let (grp_back, bp) = convert_datasheet_archetype(&mv_s, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!bp.lossy && grp_back.exclusion_notes.len() == 1);
    let bc = grp_back.cell(grp_back.exclusion_notes[0].coord).unwrap();
    assert!(bc.excluded && raw_number(bc) == Some(42.0));

    // Xy <-> MultipleVariables round-trip.
    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 1, 3).unwrap();
    (xy.x_title, xy.data_sets[0].title) = ("Time_min".into(), "Fluorescence".into());
    for (r, t) in [0.0, 5.0, 10.0].into_iter().enumerate() {
        xy.x_cells[r][0] = num(t);
        xy.data_sets[0].cells[r] = vec![num(100.0 + t), num(102.0 + t)];
    }
    let mut xy_s = sheet("xy-mv", TableArchetype::Xy(xy));
    add_excl(&mut xy_s, CellCoord::new(1, 0, 1), "spike", "Detector saturation at 5 min rep 2");

    let (xy_to_mv, p1) = convert_datasheet_archetype(&xy_s, TargetArchetypeKind::MultipleVariables, false).unwrap();
    assert!(!p1.lossy && xy_to_mv.exclusion_notes.len() == 1);
    let mc = xy_to_mv.cell(xy_to_mv.exclusion_notes[0].coord).unwrap();
    assert!(mc.excluded && raw_number(mc) == Some(107.0));

    let (mv_to_xy, p2) = convert_datasheet_archetype(&xy_to_mv, TargetArchetypeKind::Xy, false).unwrap();
    assert!(!p2.lossy && mv_to_xy.exclusion_notes.len() == 1 && mv_to_xy.exclusion_notes[0].coord == CellCoord::new(1, 0, 1));
    let xc = mv_to_xy.cell(CellCoord::new(1, 0, 1)).unwrap();
    assert!(xc.excluded && raw_number(xc) == Some(107.0));
}

#[test]
fn qol1_lossy_guard_and_unsupported_archetype() {
    let col_s = col_sheet("lossy-col", &[("LabelX", &[txt("Alpha"), txt("Beta")]), ("Measure", &[num(5.0), num(9.0)])]);
    let (forced, fp) = assert_lossy_convert(&col_s, TargetArchetypeKind::Xy);
    assert_eq!((fp.dropped_cells, forced.kind(), fp.warnings.is_empty()), (2, TableKind::Xy, false));

    let mv_s = mv_sheet_inferred(
        "lossy-mv",
        &[
            ("Factor1", &[txt("Row"), txt("Row")]),
            ("Factor2", &[txt("Col"), txt("Col")]),
            ("Factor3", &[txt("Extra"), txt("Extra")]),
            ("MeasureA", &[num(1.0), num(2.0)]),
            ("MeasureB", &[num(10.0), num(11.0)]),
        ],
    );
    let (_, mv_fp) = assert_lossy_convert(&mv_s, TargetArchetypeKind::Grouped);
    assert!(mv_fp.dropped_cells > 0);

    let cont_s = sheet("cont", TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap()));
    assert!(matches!(
        convert_datasheet_archetype(&cont_s, TargetArchetypeKind::Xy, true),
        Err(TransformError::IncompatibleTable { kind: TableKind::Contingency, .. })
    ));
}

#[test]
#[allow(deprecated)]
fn qol2_plate_96_ic50_to_xy_blank_subtraction_excluded_and_cbtemplate_roundtrip() {
    let mut tpl = PlateMapTemplate::new("96-Well IC50", PlateGeometry::Wells96, "Concentration_nM").unwrap();
    let mut values = vec![None; PlateGeometry::Wells96.well_count()];

    for (c, v, role) in [
        (0, 100.0, PlateWellRole::Blank),
        (1, 102.0, PlateWellRole::Blank),
        (2, 98.0, PlateWellRole::Blank),
        (3, 100.0, PlateWellRole::Blank),
        (4, 1200.0, PlateWellRole::PositiveControl),
        (5, 1220.0, PlateWellRole::PositiveControl),
        (6, 99_999.0, PlateWellRole::Excluded),
        (7, -500.0, PlateWellRole::Excluded),
    ] {
        tpl.set_well(0, c, role).unwrap();
        values[PlateGeometry::Wells96.well_index(0, c).unwrap()] = Some(v);
    }

    let concs = [0.001, 0.01, 0.1, 1.0, 10.0, 100.0, 1000.0, 10_000.0];
    for (col, &conc) in concs.iter().enumerate() {
        for rep in 0..2 {
            for (base_r, name, y0, step, rep_step) in [(1, "Compound-A", 1000.0, 90.0, 4.0), (3, "Compound-B", 950.0, 75.0, 6.0)] {
                tpl.set_well(base_r + rep, col, PlateWellRole::Treatment { group_name: name.into(), dilution_x: conc, replicate: rep + 1 }).unwrap();
                values[PlateGeometry::Wells96.well_index(base_r + rep, col).unwrap()] = Some(y0 - (col as f64) * step + (rep as f64) * rep_step);
            }
        }
    }

    let reloaded = PlateMapTemplate::from_cbtemplate_json(&tpl.to_cbtemplate_json().unwrap()).unwrap();
    assert_eq!(reloaded, tpl);

    let xy_sheet = map_plate_to_datasheet("IC50 Screen", PlateGeometry::Wells96, &values, &reloaded, TargetArchetypeKind::Xy, true).unwrap();
    let xy = as_xy(&xy_sheet);
    assert_eq!(
        (xy.x_title.as_str(), xy.data_sets.len(), xy.y_subcolumn_mode, xy.x_cells.len()),
        ("Concentration_nM", 2, SubcolumnMode::RawReplicates(2), 8)
    );
    assert_eq!((xy.data_sets[0].title.as_str(), xy.data_sets[1].title.as_str()), ("Compound-A", "Compound-B"));
    assert!((xy.data_sets[0].cells[0][0].active_number().unwrap() - 900.0).abs() < 1e-10);
    assert!((xy.data_sets[0].cells[0][1].active_number().unwrap() - 904.0).abs() < 1e-10);
    let qc_body = &xy_sheet.floating_notes[0].body_markdown;
    assert!(qc_body.contains("Blank mean subtracted: 100") && qc_body.contains("A07=99999"));

    let col_sheet = map_plate_to_datasheet("IC50 Column", PlateGeometry::Wells96, &values, &reloaded, TargetArchetypeKind::Column, true).unwrap();
    assert!(col_sheet.exclusion_notes.iter().any(|n| n.note.contains("A07")));
    let excl_col = as_col(&col_sheet).data_sets.iter().find(|ds| ds.title == "Excluded").unwrap();
    assert!(excl_col.cells[0][0].excluded && raw_number(&excl_col.cells[0][0]) == Some(99_999.0));
}

#[test]
fn qol2_plate_384_dimensions_grouped_and_multiple_variables() {
    let g = PlateGeometry::Wells384;
    assert_eq!((g.rows(), g.cols(), g.well_count(), g.well_label(15, 23).unwrap().as_str()), (16, 24, 384, "P24"));

    let mut tpl = PlateMapTemplate::new("384 Assay", PlateGeometry::Wells384, "Dose").unwrap();
    let mut values = vec![Some(10.0); 384];
    for c in 0..4 {
        tpl.set_well(0, c, PlateWellRole::Blank).unwrap();
        values[PlateGeometry::Wells384.well_index(0, c).unwrap()] = Some(5.0);
    }
    tpl.set_well(15, 23, PlateWellRole::Excluded).unwrap();
    values[PlateGeometry::Wells384.well_index(15, 23).unwrap()] = Some(777.0);
    for (r, conc) in [1.0, 10.0, 100.0].into_iter().enumerate() {
        for rep in 0..3 {
            tpl.set_well(1 + r, rep, PlateWellRole::Treatment { group_name: "Lead-1".into(), dilution_x: conc, replicate: rep + 1 }).unwrap();
            values[PlateGeometry::Wells384.well_index(1 + r, rep).unwrap()] = Some(50.0 + (r * 10 + rep) as f64);
        }
    }

    let mv_sheet = map_plate_to_datasheet("384 Long", PlateGeometry::Wells384, &values, &tpl, TargetArchetypeKind::MultipleVariables, true).unwrap();
    let mv = as_mv(&mv_sheet);
    assert_eq!((mv.variables.len(), mv.variables[0].cells.len(), mv.variables[0].cells[383].active_text()), (8, 384, Some("P24")));
    assert!(mv.variables[7].cells[383].excluded && mv_sheet.exclusion_notes.iter().any(|n| n.coord == CellCoord::new(383, 7, 0)));

    let grp_sheet = map_plate_to_datasheet("384 Grouped", PlateGeometry::Wells384, &values, &tpl, TargetArchetypeKind::Grouped, true).unwrap();
    let grp = as_grouped(&grp_sheet);
    assert_eq!(
        (grp.subcolumn_mode, grp.data_sets[0].title.as_str(), grp.row_titles.as_slice(), grp.data_sets[0].cells[0][0].active_number()),
        (SubcolumnMode::RawReplicates(3), "Lead-1", str_vec(&["1", "10", "100"]).as_slice(), Some(45.0))
    );
}

#[test]
fn qol2_tidy_long_pivots_column_grouped_xy_with_decimal_comma_and_exclusions() {
    let headers = str_vec(&["Condition", "Dose", "Rep", "Signal", "Flag"]);
    let rows = str_rows(&[
        &["Ctrl", "0,5", "1", "12,5", "false"],
        &["Ctrl", "0,5", "2", "13,25*", ""],
        &["Ctrl", "2,0", "1", "20,0", "0"],
        &["Drug", "0,5", "1", "8,5", "true"],
        &["Drug", "0,5", "2", "9,0", "no"],
        &["Drug", "2,0", "1", "4,75", "hemolyzed"],
    ]);
    let mapping = TidyColumnMapping { x_col: Some(1), group_col: 0, replicate_col: Some(2), value_col: 3, excluded_col: Some(4) };

    let col_sheet = pivot_tidy_long_to_datasheet("Tidy Col", &headers, &rows, &mapping, TargetArchetypeKind::Column, ',').unwrap();
    let col = as_col(&col_sheet);
    assert_eq!(
        (col.data_sets.len(), col.data_sets[0].cells[0][0].active_number(), raw_number(&col.data_sets[0].cells[1][0])),
        (2, Some(12.5), Some(13.25))
    );
    assert!(col.data_sets[0].cells[1][0].excluded && col.data_sets[1].cells[0][0].excluded && col.data_sets[1].cells[2][0].excluded);
    assert_eq!(col_sheet.exclusion_notes.len(), 3);
    assert!(col_sheet.exclusion_notes.iter().any(|n| n.note == "hemolyzed"));

    let grp_sheet = pivot_tidy_long_to_datasheet("Tidy Grouped", &headers, &rows, &mapping, TargetArchetypeKind::Grouped, ',').unwrap();
    let grp = as_grouped(&grp_sheet);
    assert_eq!(
        (grp.subcolumn_mode, grp.row_titles.as_slice(), grp.data_sets[0].title.as_str(), grp.data_sets[1].title.as_str()),
        (SubcolumnMode::RawReplicates(2), str_vec(&["0,5", "2,0"]).as_slice(), "Ctrl", "Drug")
    );
    assert_eq!(
        (grp.data_sets[0].cells[0][0].active_number(), raw_number(&grp.data_sets[0].cells[0][1]), grp.data_sets[1].cells[0][1].active_number()),
        (Some(12.5), Some(13.25), Some(9.0))
    );
    assert!(grp.data_sets[0].cells[0][1].excluded && grp_sheet.exclusion_notes.len() == 3);

    let xy_sheet = pivot_tidy_long_to_datasheet("Tidy XY", &headers, &rows, &mapping, TargetArchetypeKind::Xy, ',').unwrap();
    let xy = as_xy(&xy_sheet);
    assert_eq!(
        (xy.x_title.as_str(), xy.x_cells[0][0].active_number(), xy.x_cells[1][0].active_number(), xy.data_sets[0].cells[0][0].active_number()),
        ("Dose", Some(0.5), Some(2.0), Some(12.5))
    );
    assert!(xy.data_sets[1].cells[1][0].excluded && xy_sheet.exclusion_notes.len() == 3);
}

#[test]
fn qol2_tidy_long_ragged_and_missing_inputs_do_not_panic() {
    let headers = str_vec(&["Group", "X", "Value"]);
    let ragged = str_rows(&[
        &[],
        &["   "],
        &["GroupA"],
        &["GroupA", "1.0", "10.0"],
        &["GroupA", "", "20.0"],
        &["GroupA", "2.0", "NA"],
        &["GroupB", "1.0", "15.5"],
        &["GroupB", "2.0"],
    ]);
    let mapping = TidyColumnMapping { x_col: Some(1), group_col: 0, replicate_col: None, value_col: 2, excluded_col: None };

    assert_eq!(
        pivot_tidy_long_to_datasheet("Ragged Col", &headers, &ragged, &mapping, TargetArchetypeKind::Column, '.').unwrap().kind(),
        TableKind::Column
    );
    assert_eq!(
        pivot_tidy_long_to_datasheet("Ragged Grouped", &headers, &ragged, &mapping, TargetArchetypeKind::Grouped, '.').unwrap().kind(),
        TableKind::Grouped
    );
    let xy_sheet = pivot_tidy_long_to_datasheet("Ragged XY", &headers, &ragged, &mapping, TargetArchetypeKind::Xy, '.').unwrap();
    let xy = as_xy(&xy_sheet);
    assert_eq!((xy.x_cells.len(), xy.data_sets[0].cells[0][0].active_number(), xy.data_sets[0].cells[1][0].is_empty()), (2, Some(10.0), true));

    let no_x = TidyColumnMapping { x_col: None, group_col: 0, replicate_col: None, value_col: 2, excluded_col: None };
    assert!(matches!(
        pivot_tidy_long_to_datasheet("Missing XCol", &headers, &ragged, &no_x, TargetArchetypeKind::Xy, '.'),
        Err(TransformError::InvalidSpec { .. })
    ));
}

#[test]
fn qol1_column_and_grouped_to_contingency_preserve_labels_and_exclusions() {
    let mut col_s = col_sheet("col-cnt", &[("Exposed", &[num(18.0), num(7.0)]), ("Unexposed", &[num(4.0), excl(21.0)])]);
    crate::common::as_col_mut(&mut col_s).row_titles = str_vec(&["Outcome+", "Outcome-"]);
    col_s.exclusion_notes.push(excl_note(CellCoord::new(1, 1, 0), "Audit", "Pending recount"));

    let prev = preview_datasheet_conversion(&col_s, TargetArchetypeKind::Contingency).unwrap();
    assert_eq!((prev.lossy, prev.target_kind, prev.preserved_cells, prev.preserved_exclusions), (false, TargetArchetypeKind::Contingency, 4, 1));
    let (cont_s, conv_prev) = convert_datasheet_archetype(&col_s, TargetArchetypeKind::Contingency, false).unwrap();
    assert!(!conv_prev.lossy && cont_s.kind() == TableKind::Contingency && cont_s.exclusion_notes.len() == 1);
    let cont = as_contingency(&cont_s);
    assert_eq!(
        (cont.row_titles.as_slice(), cont.data_sets[0].title.as_str(), cont.data_sets[1].title.as_str()),
        (str_vec(&["Outcome+", "Outcome-"]).as_slice(), "Exposed", "Unexposed")
    );
    assert_eq!((cont.count_at(0, 0).unwrap(), cont.count_at(1, 1).unwrap()), (Some(18), None));

    let mut grp_s = grouped_sheet(
        "grp-cnt",
        SubcolumnMode::RawReplicates(2),
        &["Responder", "NonResponder"],
        &[("Drug", &[&[num(12.0), num(99.0)], &[num(8.0), empty()]]), ("Placebo", &[&[num(5.0), empty()], &[num(15.0), empty()]])],
    );
    if let TableArchetype::Grouped(g) = &mut grp_s.archetype {
        (g.row_factor_title, g.column_factor_title) = ("Response".into(), "Arm".into());
    }
    let (grp_cont, grp_prev) = assert_lossy_convert(&grp_s, TargetArchetypeKind::Contingency);
    assert_eq!(grp_prev.dropped_cells, 1);
    let g_cont = as_contingency(&grp_cont);
    assert_eq!(
        (g_cont.row_factor_title.as_str(), g_cont.column_factor_title.as_str(), g_cont.count_at(0, 0).unwrap()),
        ("Response", "Arm", Some(12))
    );
}

#[test]
fn qol1_mv_to_contingency_and_invalid_count_rejection() {
    let mv_counts = mv_sheet_roles(
        "mv-counts",
        &[
            ("Genotype", VariableRole::Label, &[txt("WildType"), txt("Mutant")]),
            ("Cases", VariableRole::Continuous, &[num(10.0), num(25.0)]),
            ("Controls", VariableRole::Continuous, &[num(30.0), num(15.0)]),
        ],
    );
    let (mv_cont_s, mv_prev) = convert_datasheet_archetype(&mv_counts, TargetArchetypeKind::Contingency, false).unwrap();
    assert!(!mv_prev.lossy);
    let mv_cont = as_contingency(&mv_cont_s);
    assert_eq!((mv_cont.row_factor_title.as_str(), mv_cont.row_titles.as_slice()), ("Genotype", str_vec(&["WildType", "Mutant"]).as_slice()));
    assert_eq!(
        (mv_cont.data_sets[0].title.as_str(), mv_cont.data_sets[1].title.as_str(), mv_cont.count_at(1, 0).unwrap()),
        ("Cases", "Controls", Some(25))
    );

    let mv_cat = mv_sheet_roles(
        "mv-cat",
        &[
            ("Treatment", VariableRole::Categorical, &[txt("Drug"), txt("Drug"), txt("Drug"), txt("Ctrl")]),
            ("Status", VariableRole::Categorical, &[txt("Cured"), txt("Cured"), txt("Relapse"), txt("Relapse")]),
        ],
    );
    let (xt_s, xt_prev) = convert_datasheet_archetype(&mv_cat, TargetArchetypeKind::Contingency, false).unwrap();
    assert!(!xt_prev.lossy);
    let xt = as_contingency(&xt_s);
    assert_eq!(
        (xt.row_titles.as_slice(), xt.data_sets[0].title.as_str(), xt.data_sets[1].title.as_str()),
        (str_vec(&["Drug", "Ctrl"]).as_slice(), "Cured", "Relapse")
    );
    assert_eq!((xt.count_at(0, 0).unwrap(), xt.count_at(1, 0).unwrap()), (Some(2), Some(0)));

    for bad in [num(-1.0), num(2.5), txt("bad")] {
        let bad_s = col_sheet("bad", &[("A", &[num(5.0), bad]), ("B", &[num(3.0), num(4.0)])]);
        assert!(matches!(
            preview_datasheet_conversion(&bad_s, TargetArchetypeKind::Contingency),
            Err(TransformError::Doc(DocError::InvalidCellValue { archetype: TableKind::Contingency, .. }))
        ));
    }
}

#[test]
#[allow(deprecated)]
fn qol2_cbtemplate_json_float_roundtrip_and_guards() {
    let exact_floats = [
        0.0_f64,
        -0.0_f64,
        0.1_f64,
        0.3_f64,
        1.0 / 3.0,
        std::f64::consts::PI,
        std::f64::consts::E,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        1e-300_f64,
        -1.234_567_890_123_456_7e-17_f64,
        9_007_199_254_740_993.0_f64,
        f64::MAX,
        f64::MIN,
    ];
    let mut tpl = PlateMapTemplate::new("Float Roundtrip", PlateGeometry::Wells96, "Conc").unwrap();
    for (idx, &val) in exact_floats.iter().enumerate() {
        tpl.set_well(idx / 12, idx % 12, PlateWellRole::Treatment { group_name: format!("G{idx}"), dilution_x: val, replicate: idx }).unwrap();
    }
    let json = tpl.to_cbtemplate_json().unwrap();
    let parsed = PlateMapTemplate::from_cbtemplate_json(&json).unwrap();
    assert_eq!(parsed, tpl);
    for (a, b) in parsed.wells.iter().zip(tpl.wells.iter()) {
        if let (PlateWellRole::Treatment { dilution_x: xa, .. }, PlateWellRole::Treatment { dilution_x: xb, .. }) = (a, b)
            && *xb != 0.0
        {
            assert_eq!(xa.to_bits(), xb.to_bits(), "bit-exact float roundtrip mismatch for {xb}");
        }
    }

    let mut nan_tpl = PlateMapTemplate::new("NaN", PlateGeometry::Wells96, "X").unwrap();
    nan_tpl.wells[0] = PlateWellRole::Treatment { group_name: String::from("Bad"), dilution_x: f64::NAN, replicate: 0 };
    assert!(matches!(nan_tpl.to_cbtemplate_json(), Err(TransformError::InvalidInput { .. })));
    assert!(matches!(PlateMapTemplate::from_cbtemplate_json(&format!("{json} {{}}")), Err(TransformError::InvalidInput { .. })));
    assert!(matches!(
        PlateMapTemplate::from_cbtemplate_json(&format!("{}{}", "[".repeat(256), "]".repeat(256))),
        Err(TransformError::InvalidInput { .. })
    ));
}

#[test]
fn qol1_mv_summary_to_grouped_ignores_blank_trailing_rows_without_marking_lossy() {
    // DA-01-06 regression: blank trailing rows must not mark summary pivot lossy.
    let s = mv_sheet_roles(
        "mv-sum",
        &[
            ("Dose", VariableRole::Categorical, &[txt("Low"), txt("High"), empty()]),
            ("Arm", VariableRole::Categorical, &[txt("Drug"), txt("Drug"), empty()]),
            ("Mean", VariableRole::Continuous, &[num(10.0), num(20.0), empty()]),
            ("SD", VariableRole::Continuous, &[num(1.5), num(2.5), empty()]),
            ("N", VariableRole::Continuous, &[num(5.0), num(6.0), empty()]),
        ],
    );
    let (converted, prev) = convert_datasheet_archetype(&s, TargetArchetypeKind::Grouped, false).unwrap();
    assert!(!prev.lossy && prev.dropped_cells == 0);
    let grp = as_grouped(&converted);
    assert_eq!(
        (grp.subcolumn_mode, grp.row_titles.as_slice(), grp.data_sets.len(), grp.data_sets[0].title.as_str()),
        (SubcolumnMode::MeanSdN, str_vec(&["Low", "High"]).as_slice(), 1, "Drug")
    );

    // Conversely, a row missing a factor token with an occupied summary cell IS genuinely lossy.
    let mut lossy_s = s;
    crate::common::as_mv_mut(&mut lossy_s).variables[2].cells[2] = num(99.0);
    let (_, lossy_prev) = assert_lossy_convert(&lossy_s, TargetArchetypeKind::Grouped);
    assert_eq!(lossy_prev.dropped_cells, 1);
}
