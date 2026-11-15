use lumencraft_doc::{
    CURRENT_SCHEMA_VERSION, Cell, CellCoord, ColumnDesignHint, ColumnTable, DataSheet, DocError, ImportFilterSpec, ImportLinkSpec,
    ImportPlacementSpec, LinkSource, PlacementMode, ProjectDocument, SubcolumnMode, TableArchetype, TableKind, XColumnMode, XyTable,
};
use lumencraft_format::{
    CsvOptions, FormatError, ImportBaselineAdjustment, ZipWriter, apply_csv_import_link, import_csv_to_datasheet, open_project_bytes,
    parse_delimited_records, save_project_bytes, validate_project_document,
};

#[allow(clippy::unwrap_used)]
fn build_zip_with_doc_bytes(doc_bytes: Vec<u8>, manifest_schema_version: u32) -> Vec<u8> {
    let manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": manifest_schema_version,
        "kind": "Project",
        "title": "Audit Test",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let mut writer = ZipWriter::new();
    writer.add_file("document.json", doc_bytes).unwrap();
    writer.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    writer.finish().unwrap()
}

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
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap();
    doc.add_data_sheet(DataSheet::new("col-1", "C", TableArchetype::Column(col)).unwrap()).unwrap();

    let mut mutated_doc = doc.clone();
    if let TableArchetype::Column(ref mut tbl) = mutated_doc.data_sheets[0].archetype {
        tbl.subcolumn_mode = SubcolumnMode::RawReplicates(3);
        tbl.data_sets[0].cells = vec![vec![Cell::number(1.0), Cell::number(2.0), Cell::number(3.0)]; 2];
    }
    assert!(matches!(
        validate_project_document(&mutated_doc),
        Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, .. }))
    ));

    let mut v = serde_json::to_value(&doc).unwrap();
    let col_obj = v["data_sheets"][0]["archetype"]["Column"].as_object_mut().unwrap();
    col_obj.insert(String::from("subcolumn_mode"), serde_json::json!({ "RawReplicates": 3 }));
    let zip = build_zip_with_doc_bytes(serde_json::to_vec(&v).unwrap(), CURRENT_SCHEMA_VERSION);
    assert!(matches!(open_project_bytes(&zip), Err(FormatError::Json(_))));

    let csv_res = import_csv_to_datasheet("c1", "Bad", "A,B\n1,2\n", TableKind::Column, SubcolumnMode::RawReplicates(2), &CsvOptions::default());
    assert!(matches!(csv_res, Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { archetype: TableKind::Column, .. }))));
}

#[test]
fn r1_s06_05_document_json_amplification_bounded_and_legacy_v0_loads() {
    // Array bomb `[[],[],...]` must reject immediately without unbounded DOM allocation.
    let mut array_bomb = Vec::with_capacity(900_000);
    array_bomb.push(b'[');
    for i in 0..250_000 {
        if i > 0 {
            array_bomb.push(b',');
        }
        array_bomb.extend_from_slice(b"[]");
    }
    array_bomb.push(b']');
    let bomb_zip = build_zip_with_doc_bytes(array_bomb, CURRENT_SCHEMA_VERSION);
    assert!(open_project_bytes(&bomb_zip).is_err());

    // Legacy v0 JSON with > 1,000,000 nested DOM nodes must hit migration DOM budget cleanly.
    let mut doc = ProjectDocument::new("Valid Project").unwrap();
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    doc.add_data_sheet(DataSheet::new("s1", "S1", TableArchetype::Column(col)).unwrap()).unwrap();
    let mut v0_bomb = Vec::with_capacity(3_100_000);
    v0_bomb.extend_from_slice(br#"{"schema_version":0,"bomb":["#);
    for i in 0..1_010_000 {
        if i > 0 {
            v0_bomb.push(b',');
        }
        v0_bomb.extend_from_slice(b"[]");
    }
    v0_bomb.extend_from_slice(b"]}");
    let v0_bomb_zip = build_zip_with_doc_bytes(v0_bomb, 0);
    assert!(open_project_bytes(&v0_bomb_zip).is_err());

    // Valid v1 roundtrip & valid v0 legacy migration both succeed.
    let v1_zip = save_project_bytes(&doc).unwrap();
    assert_eq!(open_project_bytes(&v1_zip).unwrap(), doc);

    let mut v0_doc_json = serde_json::to_value(&doc).unwrap();
    v0_doc_json.as_object_mut().unwrap().remove("schema_version");
    let v0_zip = build_zip_with_doc_bytes(serde_json::to_vec(&v0_doc_json).unwrap(), 0);
    let migrated = open_project_bytes(&v0_zip).unwrap();
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(migrated.data_sheets.len(), 1);
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

    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("rows.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::InsertRows,
            anchor: CellCoord::new(1, 0, 0),
            propagate_title_header: false,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };

    apply_csv_import_link(&mut sheet, "100,200\n300,400\n", &spec, false, ImportBaselineAdjustment::None, "lnk-ins", "Insert two rows").unwrap();

    let TableArchetype::Xy(after) = &sheet.archetype else {
        panic!("expected Xy");
    };
    assert_eq!(after.row_count(), 5);
    assert_eq!(after.x_cells[0][0].active_number(), Some(10.0));
    assert!(after.x_cells[1][0].is_empty());
    assert!(after.x_cells[2][0].is_empty());
    assert_eq!(after.x_cells[3][0].active_number(), Some(20.0));
    assert_eq!(after.x_cells[4][0].active_number(), Some(30.0));

    assert_eq!(after.row_titles, vec!["R0", "", "", "R1", "R2"]);
    assert_eq!(after.data_sets[0].cells[0][0].active_number(), Some(1.0));
    assert_eq!(after.data_sets[0].cells[1][0].active_number(), Some(100.0));
    assert_eq!(after.data_sets[0].cells[2][0].active_number(), Some(300.0));
    assert_eq!(after.data_sets[0].cells[3][0].active_number(), Some(2.0));
    assert_eq!(after.data_sets[0].cells[4][0].active_number(), Some(3.0));
    assert_eq!(after.data_sets[1].cells[1][0].active_number(), Some(200.0));
    assert_eq!(after.data_sets[1].cells[3][0].active_number(), Some(5.0));
    assert_eq!(after.data_sets[1].cells[4][0].active_number(), Some(6.0));
}

#[test]
fn r1_x1_02_wide_comma_bomb_csv_rejected_before_allocation() {
    // Column-width cap during tokenization stops single-row comma bombs before allocating unbounded Vecs.
    let single_row_bomb = ",".repeat(600_000);
    assert!(parse_delimited_records(&single_row_bomb, &CsvOptions::default()).is_err());

    // Cumulative field cap stops multi-row field amplification before allocating unbounded Vec<Vec<String>>.
    let field_bomb = format!("{}\n", ",".repeat(50_000)).repeat(30);
    assert!(parse_delimited_records(&field_bomb, &CsvOptions::default()).is_err());

    // Target-aware import enforces tight table column cap early on row 1.
    let multi_row_bomb = format!("{}\n", ",".repeat(2_000)).repeat(200);
    assert!(import_csv_to_datasheet("bomb", "Bomb", &multi_row_bomb, TableKind::Xy, SubcolumnMode::Single, &CsvOptions::default()).is_err());

    // Normal bounded CSV continues to import cleanly.
    let normal = import_csv_to_datasheet("ok", "OK", "X,Y\n1,2\n3,4\n", TableKind::Xy, SubcolumnMode::Single, &CsvOptions::default()).unwrap();
    assert_eq!(normal.archetype.row_count(), 2);
}
