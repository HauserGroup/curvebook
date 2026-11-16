use curvebook_doc::{
    AnalysisKind, AnalysisSpec, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet, GraphSheet, InfoSheet,
    JournalWidthPreset, LayoutSheet, LayoutSpec, OneWayAnovaSpec, ProjectDocument, ResultStatus, SheetId, SubcolumnMode, TableArchetype, TableKind,
    XyTable,
};
use curvebook_format::{
    CsvOptions, export_datasheet_to_csv, import_csv_to_datasheet, import_prism_bytes, import_pzfx_bytes, instantiate_template_bytes,
    open_project_bytes, open_template_bytes, parse_delimited_records, parse_xml, save_project_bytes, save_template_bytes,
};
use proptest::prelude::*;

fn arb_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        Just(Cell::empty()),
        (-1e6_f64..1e6_f64).prop_map(|v| {
            let rounded = (v * 1000.0).round() / 1000.0;
            Cell::number(rounded)
        }),
        (-1e6_f64..1e6_f64).prop_map(|v| {
            let rounded = (v * 1000.0).round() / 1000.0;
            Cell::excluded_number(rounded)
        }),
        "[A-Za-z0-9 _-]{1,12}".prop_map(Cell::text),
    ]
}

fn arb_csv_safe_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        Just(Cell::empty()),
        (-1e5_f64..1e5_f64).prop_map(|v| {
            let rounded = (v * 1000.0).round() / 1000.0;
            Cell::number(rounded)
        }),
        (-1e5_f64..1e5_f64).prop_map(|v| {
            let rounded = (v * 1000.0).round() / 1000.0;
            Cell::excluded_number(rounded)
        }),
        "[A-Za-z][A-Za-z0-9 _-]{1,10}".prop_map(|s| {
            // Avoid missing sentinels like NA/NaN/NULL when testing round-trip text fidelity
            if s.eq_ignore_ascii_case("na") || s.eq_ignore_ascii_case("nan") || s.eq_ignore_ascii_case("null") {
                Cell::text("Label")
            } else {
                Cell::text(s)
            }
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn curvebook_archive_round_trip(
        proj_title in "[A-Za-z0-9 _-]{1,24}",
        info_val in -1e4_f64..1e4_f64,
        col_cells in prop::collection::vec(prop::collection::vec(arb_cell(), 4), 2),
        xy_x in prop::collection::vec(-100.0_f64..100.0_f64, 3),
        xy_y in prop::collection::vec(arb_cell(), 3),
    ) {
        let mut doc = ProjectDocument::new(proj_title).unwrap();
        let mut info = InfoSheet::new_preseeded("info-1", "Metadata").unwrap();
        info.set_constant("Concentration", ConstantValue::Number(info_val), Some(String::from("uM"))).unwrap();
        doc.add_info_sheet(info).unwrap();

        let col_table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 4).unwrap();
        let mut col_sheet = DataSheet::new("data-col", "Groups", TableArchetype::Column(col_table)).unwrap();
        col_sheet.info_sheet_id = Some(SheetId::new("info-1"));
        for (c, col_vec) in col_cells.iter().enumerate() {
            for (r, cell) in col_vec.iter().enumerate() {
                col_sheet.set_cell(CellCoord::new(r, c, 0), cell.clone()).unwrap();
            }
        }
        doc.add_data_sheet(col_sheet.clone()).unwrap();

        let xy_table = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
        let mut xy_sheet = DataSheet::new("data-xy", "Curve", TableArchetype::Xy(xy_table)).unwrap();
        for (r, (&x, y)) in xy_x.iter().zip(xy_y.iter()).enumerate() {
            xy_sheet.set_x_or_time_cell(r, 0, Cell::number(x)).unwrap();
            xy_sheet.set_cell(CellCoord::new(r, 0, 0), y.clone()).unwrap();
        }
        doc.add_data_sheet(xy_sheet).unwrap();

        let spec = AnalysisSpec::new("anova-1", "data-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
        doc.add_result_sheet(curvebook_doc::ResultSheet::new(spec).unwrap()).unwrap();
        doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &col_sheet).unwrap()).unwrap();
        doc.add_layout_sheet(LayoutSheet::new("layout-1", "Fig 1", LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm)).unwrap()).unwrap();

        let bytes = save_project_bytes(&doc).unwrap();
        let decoded = open_project_bytes(&bytes).unwrap();
        prop_assert_eq!(&decoded, &doc);

        let tmpl_bytes = save_template_bytes(&doc).unwrap();
        let tmpl_doc = open_template_bytes(&tmpl_bytes).unwrap();
        let inst_doc = instantiate_template_bytes(&tmpl_bytes, Some("Instantiated")).unwrap();
        prop_assert_eq!(inst_doc.title, "Instantiated");
        prop_assert_eq!(tmpl_doc.info_sheets, doc.info_sheets);
        prop_assert_eq!(&tmpl_doc.result_sheets[0].status, &ResultStatus::Dirty);
        for ds in &inst_doc.data_sheets {
            for r in 0..ds.archetype.row_count() {
                for c in 0..ds.archetype.column_count() {
                    prop_assert!(ds.cell(CellCoord::new(r, c, 0)).unwrap().is_empty());
                }
            }
        }
    }

    #[test]
    fn csv_datasheet_write_then_read_round_trips(
        cells in prop::collection::vec(prop::collection::vec(arb_csv_safe_cell(), 5), 3),
        use_euro in any::<bool>(),
    ) {
        let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).unwrap();
        for (c, col) in cells.iter().enumerate() {
            table.data_sets[c].title = format!("Group_{c}");
            for (r, cell) in col.iter().enumerate() {
                table.data_sets[c].cells[r][0] = cell.clone();
            }
        }
        let sheet = DataSheet::new("col-1", "CSV Roundtrip", TableArchetype::Column(table)).unwrap();
        let opts = if use_euro { CsvOptions::european_csv() } else { CsvOptions::csv() };
        let csv_text = export_datasheet_to_csv(&sheet, &opts).unwrap();
        let round = import_csv_to_datasheet("col-1", "CSV Roundtrip", &csv_text, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();

        let (TableArchetype::Column(orig), TableArchetype::Column(parsed)) = (&sheet.archetype, &round.archetype) else {
            return Err(TestCaseError::fail("expected ColumnTable"));
        };
        prop_assert_eq!(orig.data_sets.len(), parsed.data_sets.len());
        for (a, b) in orig.data_sets.iter().zip(parsed.data_sets.iter()) {
            prop_assert_eq!(&a.title, &b.title);
            for (ca, cb) in a.cells.iter().zip(b.cells.iter()) {
                prop_assert_eq!(ca[0].excluded, cb[0].excluded);
                match (&ca[0].value, &cb[0].value) {
                    (CellValue::Number(x), CellValue::Number(y)) => prop_assert!((x - y).abs() < 1e-9),
                    (CellValue::Text(x), CellValue::Text(y)) => prop_assert_eq!(x.trim(), y.trim()),
                    (CellValue::Empty, CellValue::Empty) => {}
                    other => return Err(TestCaseError::fail(format!("cell mismatch: {other:?}"))),
                }
            }
        }
    }

    #[test]
    fn malformed_bytes_and_strings_never_panic(
        raw in prop::collection::vec(any::<u8>(), 0..512),
        text in "\\PC{0,256}",
    ) {
        let _ = open_project_bytes(&raw);
        let _ = import_prism_bytes(&raw);
        let _ = import_pzfx_bytes(&raw);
        let _ = parse_delimited_records(&text, &CsvOptions::csv());
        let _ = parse_xml(&text);
    }
}
