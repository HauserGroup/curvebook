use crate::common::{build_archive_zip, col_sheet, make_array_bomb_json, make_import_link_spec, make_nested_empty_array_json};
use curvebook_doc::{
    CURRENT_SCHEMA_VERSION, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DocError, PlacementMode, ProjectDocument, SubcolumnMode,
    TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_format::{
    CsvOptions, FormatError, ImportBaselineAdjustment, apply_csv_import_link, import_csv_to_datasheet, open_project_bytes, parse_delimited_records,
    save_project_bytes,
};

#[test]
fn r1_s05_05_column_raw_replicates_rejected_on_load_and_csv_import() {
    let mut bad_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    bad_col.subcolumn_mode = SubcolumnMode::RawReplicates(3);
    bad_col.data_sets[0].cells = vec![vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)]; 2];
    assert!(matches!(
        DataSheet::new("col-1", "C", TableArchetype::Column(bad_col)),
        Err(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, .. })
    ));

    let mut doc = ProjectDocument::new("Col Reject").unwrap();
    doc.add_data_sheet(col_sheet("col-1", 1, 2)).unwrap();
    let mut v = serde_json::to_value(&doc).unwrap();
    v["data_sheets"][0]["archetype"]["Column"]["subcolumn_mode"] = serde_json::json!({ "RawReplicates": 3 });
    let zip = build_archive_zip(&serde_json::to_vec(&v).unwrap(), CURRENT_SCHEMA_VERSION, "Project", "Audit Test");
    assert!(matches!(open_project_bytes(&zip), Err(FormatError::Json(_))));

    let csv_res = import_csv_to_datasheet("c1", "Bad", "A,B\n1,2\n", TableKind::Column, SubcolumnMode::RawReplicates(2), &CsvOptions::default());
    assert!(matches!(csv_res, Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, .. }))));
}

#[test]
fn r1_s06_05_document_json_amplification_bounded_and_legacy_v0_loads() {
    for (label, payload, schema_ver) in [
        ("v_curr_array_bomb", make_array_bomb_json(250_000), CURRENT_SCHEMA_VERSION),
        ("v0_dom_bomb", make_nested_empty_array_json(r#"{"schema_version":0,"bomb":["#, 1_010_000, "]}"), 0),
    ] {
        assert!(open_project_bytes(&build_archive_zip(&payload, schema_ver, "Project", "Audit Test")).is_err(), "{label}");
    }

    let mut doc = ProjectDocument::new("Valid Project").unwrap();
    doc.add_data_sheet(col_sheet("s1", 1, 1)).unwrap();
    assert_eq!(open_project_bytes(&save_project_bytes(&doc).unwrap()).unwrap(), doc);

    let mut v0_doc = serde_json::to_value(&doc).unwrap();
    v0_doc.as_object_mut().unwrap().remove("schema_version");
    let migrated = open_project_bytes(&build_archive_zip(&serde_json::to_vec(&v0_doc).unwrap(), 0, "Project", "Audit Test")).unwrap();
    assert_eq!((migrated.schema_version, migrated.data_sheets.len()), (CURRENT_SCHEMA_VERSION, 1));
}

#[test]
fn r1_s06_12_apply_csv_import_link_insert_rows_shifts_existing_rows_and_titles() {
    let mut xy = XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::Single, 2, 3).unwrap();
    for r in 0..3 {
        xy.set_x_cell(r, Cell::number((r as f64 + 1.0) * 10.0)).unwrap();
        xy.data_sets[0].cells[r][0] = Cell::number(r as f64 + 1.0);
        xy.data_sets[1].cells[r][0] = Cell::number(r as f64 + 4.0);
    }
    xy.row_titles = vec![String::from("R0"), String::from("R1"), String::from("R2")];
    xy.row_titles_expanded = true;
    let mut sheet = DataSheet::new("xy-ins", "XY Insert", TableArchetype::Xy(xy)).unwrap();

    let spec = make_import_link_spec(PlacementMode::InsertRows, CellCoord::new(1, 0, 0), false);
    apply_csv_import_link(&mut sheet, "100,200\n300,400\n", &spec, false, ImportBaselineAdjustment::None, "lnk-ins", "Insert two rows").unwrap();

    let TableArchetype::Xy(after) = &sheet.archetype else { panic!("expected Xy") };
    assert_eq!(
        (after.row_count(), &after.row_titles),
        (5, &vec![String::from("R0"), String::new(), String::new(), String::from("R1"), String::from("R2")])
    );
    assert_eq!(after.x_cells.iter().map(|r| r[0].active_number()).collect::<Vec<_>>(), vec![Some(10.0), None, None, Some(20.0), Some(30.0)]);
    assert_eq!(
        after.data_sets[0].cells.iter().map(|r| r[0].active_number()).collect::<Vec<_>>(),
        vec![Some(1.0), Some(100.0), Some(300.0), Some(2.0), Some(3.0)]
    );
    assert_eq!(
        after.data_sets[1].cells.iter().map(|r| r[0].active_number()).collect::<Vec<_>>(),
        vec![Some(4.0), Some(200.0), Some(400.0), Some(5.0), Some(6.0)]
    );
}

#[test]
fn r1_x1_02_wide_comma_bomb_csv_rejected_before_allocation() {
    for (label, payload) in [("single_row_600k", ",".repeat(600_000)), ("cumulative_50k_x_30", format!("{}\n", ",".repeat(50_000)).repeat(30))] {
        assert!(parse_delimited_records(&payload, &CsvOptions::default()).is_err(), "{label}");
    }
    let multi_row_bomb = format!("{}\n", ",".repeat(2_000)).repeat(200);
    assert!(import_csv_to_datasheet("bomb", "Bomb", &multi_row_bomb, TableKind::Xy, SubcolumnMode::Single, &CsvOptions::default()).is_err());
    assert_eq!(
        import_csv_to_datasheet("ok", "OK", "X,Y\n1,2\n3,4\n", TableKind::Xy, SubcolumnMode::Single, &CsvOptions::default())
            .unwrap()
            .archetype
            .row_count(),
        2
    );
}
