use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, DataSheet,
    DocError, FloatingNote, GraphSheet, GroupedTable, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LinkBorderColor, LinkSource, LinkedDataRegion, MAX_SUBCOLUMNS, MAX_TITLE_BYTES,
    MultipleVariablesTable, NestedTable, NonlinearFitSpec, OneWayAnovaSpec, ParameterRule, ParameterScope, PartsOfWholeTable, PlacementMode,
    ProjectDocument, RectPt, ResultScalarEntry, ResultSheet, ResultStatus, SequenceProgression, SheetCellRange, SheetId, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use lumencraft_format::{
    ArchiveKind, CsvOptions, FormatError, ImportBaselineAdjustment, ZipLimits, ZipWriter, apply_csv_import_link, export_datasheet_to_csv,
    filter_delimited_text, import_csv_to_datasheet, instantiate_template_bytes, open_archive_bytes, open_project_bytes, open_template_bytes,
    parse_cell_token, parse_delimited_records, rearrange_and_baseline_grid, save_project_bytes, save_template_bytes, validate_project_document,
};

fn build_sample_project() -> Result<ProjectDocument, FormatError> {
    let mut doc = ProjectDocument::new("Dose Response Assay")?;

    let mut info = InfoSheet::new_preseeded("info-1", "Assay Metadata")?;
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM")))?;
    info.notes_markdown = String::from("Room temperature incubation 30 min.");
    doc.add_info_sheet(info)?;

    let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 2, 3)?;
    xy.x_title = String::from("Log[Agonist]");
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    for (r, x) in [-9.0, -8.0, -7.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(*x))?;
    }
    xy.data_sets[0].cells[0] = vec![Cell::number(10.5), Cell::excluded_number(99.9)];
    xy.data_sets[0].cells[1] = vec![Cell::number(48.2), Cell::number(51.0)];
    xy.data_sets[1].cells[0] = vec![Cell::number(12.0), Cell::number(11.4)];
    let mut ds = DataSheet::new("data-xy", "Binding Curve", TableArchetype::Xy(xy))?;
    ds.info_sheet_id = Some(SheetId::new("info-1"));
    ds.floating_notes.push(FloatingNote::new("note-1", RectPt::new(10.0, 10.0, 120.0, 50.0), "Outlier flagged"));
    doc.add_data_sheet(ds.clone())?;

    let mut fit = NonlinearFitSpec::new("four_pl");
    fit.parameter_rules.push(ParameterRule {
        name: String::from("HotNM"),
        scope: ParameterScope::HookedConstant(lumencraft_doc::HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-1")),
            constant_name: String::from("HotNM"),
        }),
        initial_override: None,
        lower_bound: Some(0.0),
        upper_bound: None,
    });
    let spec = AnalysisSpec::new("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit))?;
    let mut res = ResultSheet::new(spec)?;
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 1.25e-8, units: Some(String::from("M")) });
    doc.add_result_sheet(res)?;

    let gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &ds)?;
    doc.add_graph_sheet(gsheet)?;

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::SingleColumn85Mm);
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(12.0, 12.0, 200.0, 150.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec)?)?;
    Ok(doc)
}

#[test]
fn lumencraft_archive_is_deterministic_and_round_trips() {
    let doc = build_sample_project().unwrap();
    let bytes1 = save_project_bytes(&doc).unwrap();
    let bytes2 = save_project_bytes(&doc).unwrap();
    assert_eq!(bytes1, bytes2);

    let (kind, decoded) = open_archive_bytes(&bytes1).unwrap();
    assert_eq!(kind, ArchiveKind::Project);
    assert_eq!(decoded, doc);

    // Template loader rejects project kind cleanly.
    assert!(matches!(open_template_bytes(&bytes1), Err(FormatError::InvalidArchiveKind { expected: "template", found: "project" })));
}

#[test]
fn lctemplate_strips_raw_cells_and_cached_results_while_preserving_schemas() {
    let doc = build_sample_project().unwrap();
    let tmpl_bytes = save_template_bytes(&doc).unwrap();
    assert!(matches!(open_project_bytes(&tmpl_bytes), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));

    let tmpl = open_template_bytes(&tmpl_bytes).unwrap();
    let TableArchetype::Xy(xy) = &tmpl.data_sheets[0].archetype else {
        panic!("expected XY table");
    };
    assert_eq!(xy.x_title, "Log[Agonist]");
    assert_eq!(xy.data_sets[0].title, "Wildtype");
    assert!(xy.x_cells[0][0].is_empty());
    assert!(xy.data_sets[0].cell(0, 0).unwrap().is_empty());
    assert_eq!(tmpl.result_sheets[0].status, ResultStatus::Dirty);
    assert!(tmpl.result_sheets[0].scalars.is_empty());
    assert_eq!(tmpl.info_sheets[0].get_constant("HotNM").unwrap().value.as_finite_f64(), Some(2.5));
    assert_eq!(tmpl.graph_sheets.len(), 1);
    assert_eq!(tmpl.layout_sheets.len(), 1);

    let inst = instantiate_template_bytes(&tmpl_bytes, Some("Replicate Run #2")).unwrap();
    assert_eq!(inst.title, "Replicate Run #2");
    assert_eq!(inst.result_sheets[0].status, ResultStatus::Dirty);
}

#[test]
fn archive_detects_crc_corruption_and_missing_entries() {
    let doc = build_sample_project().unwrap();
    let mut writer = ZipWriter::new();
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    writer.add_file("document.json", doc_bytes.clone()).unwrap();
    let bad_manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes) ^ 0xFFFF_FFFF,
        "sheets": []
    });
    writer.add_file("manifest.json", serde_json::to_vec(&bad_manifest).unwrap()).unwrap();
    let corrupt_zip = writer.finish().unwrap();
    assert!(matches!(open_project_bytes(&corrupt_zip), Err(FormatError::CorruptCrc { .. })));

    // Missing manifest.json
    let mut no_manifest = ZipWriter::new();
    no_manifest.add_file("document.json", doc_bytes).unwrap();
    let missing_zip = no_manifest.finish().unwrap();
    assert!(matches!(open_project_bytes(&missing_zip), Err(FormatError::MissingArchiveEntry { .. })));
}

#[test]
fn zip_rejects_path_traversal_duplicates_and_zip_bombs() {
    let mut w = ZipWriter::new();
    assert!(matches!(w.add_file("../escape.txt", b"x".to_vec()), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(w.add_file("/abs/path.txt", b"x".to_vec()), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(w.add_file("dir\\win.txt", b"x".to_vec()), Err(FormatError::UnsafePath { .. })));
    w.add_file("a.txt", b"1".to_vec()).unwrap();
    assert!(matches!(w.add_file("a.txt", b"2".to_vec()), Err(FormatError::DuplicateEntry { .. })));

    // Zip bomb check via custom small limits on valid archive
    let doc = build_sample_project().unwrap();
    let bytes = save_project_bytes(&doc).unwrap();
    let tiny_limits = ZipLimits { max_entries: 1, max_total_uncompressed_bytes: 1024 * 1024, max_entry_uncompressed_bytes: 1024 * 1024 };
    assert!(matches!(lumencraft_format::open_archive_bytes_with_limits(&bytes, tiny_limits), Err(FormatError::ZipBomb { .. })));

    let tiny_bytes_limits = ZipLimits { max_entries: 100, max_total_uncompressed_bytes: 64, max_entry_uncompressed_bytes: 64 };
    assert!(matches!(lumencraft_format::open_archive_bytes_with_limits(&bytes, tiny_bytes_limits), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn schema_migration_v0_to_v1_and_future_version_rejection() {
    let v0_doc = serde_json::json!({
        "schema_version": 0,
        "title": "Legacy Study",
        "data_sheets": [],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": []
    });
    let doc_bytes = serde_json::to_vec(&v0_doc).unwrap();
    let manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": 0,
        "kind": "Project",
        "title": "Legacy Study",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let mut w = ZipWriter::new();
    w.add_file("document.json", doc_bytes).unwrap();
    w.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    let migrated = open_project_bytes(&w.finish().unwrap()).unwrap();
    assert_eq!(migrated.title, "Legacy Study");
    assert_eq!(migrated.schema_version, CURRENT_SCHEMA_VERSION);

    // Future schema version rejected cleanly
    let future_doc = serde_json::json!({
        "schema_version": CURRENT_SCHEMA_VERSION + 5,
        "title": "Future Study",
        "data_sheets": [],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": [],
        "active_sheet_id": null,
        "family_anchor_sheet_id": null
    });
    let f_bytes = serde_json::to_vec(&future_doc).unwrap();
    let f_manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION + 5,
        "kind": "Project",
        "title": "Future Study",
        "document_crc32": crc32fast::hash(&f_bytes),
        "sheets": []
    });
    let mut w2 = ZipWriter::new();
    w2.add_file("document.json", f_bytes).unwrap();
    w2.add_file("manifest.json", serde_json::to_vec(&f_manifest).unwrap()).unwrap();
    assert!(matches!(open_project_bytes(&w2.finish().unwrap()), Err(FormatError::UnsupportedSchemaVersion { .. })));
}

#[test]
fn detects_circular_hook_and_import_link_cycles_and_bounds() {
    let mut doc = ProjectDocument::new("Cycle Test").unwrap();
    let mut ds_a =
        DataSheet::new("data-a", "A", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    let mut ds_b =
        DataSheet::new("data-b", "B", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    ds_a.linked_regions.push(LinkedDataRegion {
        id: String::from("l-ab"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("data-b"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::default(),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::default(),
        provenance_summary: String::from("B->A"),
    });
    ds_b.linked_regions.push(LinkedDataRegion {
        id: String::from("l-ba"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("data-a"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::default(),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::default(),
        provenance_summary: String::from("A->B"),
    });
    doc.add_data_sheet(ds_a).unwrap();
    doc.add_data_sheet(ds_b).unwrap();
    assert!(matches!(save_project_bytes(&doc), Err(FormatError::DependencyCycle { .. })));

    // Self-referential ResultSheet cycle
    let mut doc2 = ProjectDocument::new("Self Loop").unwrap();
    let res = ResultSheet::new(AnalysisSpec::new("res-1", "res-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    doc2.add_result_sheet(res).unwrap();
    assert!(matches!(save_project_bytes(&doc2), Err(FormatError::DependencyCycle { .. })));

    // Non-finite float & oversized title bounds
    let mut doc3 = ProjectDocument::new("Bounds").unwrap();
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    col.data_sets[0].cells[0][0].value = CellValue::Number(f64::NAN);
    doc3.add_data_sheet(DataSheet::new("d1", "D1", TableArchetype::Column(col)).unwrap()).unwrap();
    assert!(matches!(save_project_bytes(&doc3), Err(FormatError::NonFiniteFloat { .. })));

    let mut doc4 = ProjectDocument::new("Title Bounds").unwrap();
    doc4.title = "X".repeat(MAX_TITLE_BYTES + 1);
    assert!(matches!(save_project_bytes(&doc4), Err(FormatError::Doc(DocError::TitleTooLong { .. }))));
}

#[test]
fn csv_rfc4180_regional_sentinels_and_exclusions() {
    let csv = "\u{FEFF}\"Group, A\";\"Group \"\"B\"\"\"\r\n1,234;45,6*\r\nNA;*\r\nNULL;.\r\n\"line1\nline2\";*7,5\r";
    let detected = CsvOptions::detect(csv);
    assert_eq!(detected.delimiter, ';');
    assert_eq!(detected.decimal_separator, ',');

    let mut opts = CsvOptions::european_csv();
    opts.thousands_separator = Some('.');
    let records = parse_delimited_records(csv, &opts).unwrap();
    assert_eq!(records.len(), 5);
    assert_eq!(records[0], vec!["Group, A", "Group \"B\""]);
    assert_eq!(records[4][0], "line1\nline2");

    assert_eq!(parse_cell_token("1.234,5", &opts), Cell::number(1234.5));
    assert_eq!(parse_cell_token("45,6*", &opts), Cell::excluded_number(45.6));
    assert_eq!(parse_cell_token("*", &opts), Cell { value: CellValue::Empty, excluded: true });
    assert_eq!(parse_cell_token("NA", &opts), Cell::empty());
    assert_eq!(parse_cell_token("NULL", &opts), Cell::empty());
    assert_eq!(parse_cell_token(".", &opts), Cell::empty());

    let sheet = import_csv_to_datasheet("csv-1", "Regional Import", csv, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    let round_csv = export_datasheet_to_csv(&sheet, &opts).unwrap();
    let reimported = import_csv_to_datasheet("csv-2", "Regional Import", &round_csv, TableKind::Column, SubcolumnMode::Single, &opts).unwrap();
    assert_eq!(sheet.archetype, reimported.archetype);
}

#[test]
fn multi_stage_import_link_pipeline_filter_transpose_baseline_and_freeze() {
    let raw_csv = "Dose,RespA,RespB\n0,10,20\n1,15,25*\n2,20,30\n3,25,35\n";
    let filter = ImportFilterSpec {
        row_window: Some((0, 3)),
        col_window: Some((1, 2)),
        decimate_every_k: Some(2),
        unstack_group_column: None,
        parse_asterisk_exclusions: true,
    };
    let filtered = filter_delimited_text(raw_csv, ',', '.', true, &filter).unwrap();
    // Rows 0 and 2 kept: [[10, 20], [20, 30]]
    assert_eq!(filtered.headers, vec!["RespA", "RespB"]);
    assert_eq!(filtered.cells.len(), 2);
    assert_eq!(filtered.cells[0][0].active_number(), Some(10.0));
    assert_eq!(filtered.cells[1][1].active_number(), Some(30.0));

    let placement = ImportPlacementSpec {
        mode: PlacementMode::Overwrite,
        anchor: CellCoord::new(0, 0, 0),
        propagate_title_header: true,
        skip_contiguous_blanks: true,
        transpose: true,
    };
    let adjusted = rearrange_and_baseline_grid(&filtered.cells, &placement, ImportBaselineAdjustment::SubtractFirstRow);
    // Transposed: [[10, 20], [20, 30]] -> SubtractFirstRow: [[0, 0], [10, 10]]
    assert_eq!(adjusted[0][0].active_number(), Some(0.0));
    assert_eq!(adjusted[1][1].active_number(), Some(10.0));

    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap();
    let mut target = DataSheet::new("tgt-1", "Linked Target", TableArchetype::Column(col)).unwrap();
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("assay.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::Overwrite,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: true,
            skip_contiguous_blanks: false,
            transpose: false,
        },
    };
    let range = apply_csv_import_link(
        &mut target,
        "ColA,ColB\n10,20\n15,25*\n",
        &spec,
        true,
        ImportBaselineAdjustment::SubtractFirstRow,
        "lnk-csv",
        "From assay.csv",
    )
    .unwrap();
    assert_eq!(range.start, CellCoord::new(0, 0, 0));
    assert_eq!(range.end, CellCoord::new(1, 1, 0));
    assert_eq!(target.linked_regions[0].border_color(), Some(LinkBorderColor::Charcoal));
    assert_eq!(target.cell(CellCoord::new(1, 0, 0)).unwrap().active_number(), Some(5.0));
    assert!(target.cell(CellCoord::new(1, 1, 0)).unwrap().excluded);

    // Unstack long-to-wide test
    let long_csv = "Group,Value\nCtrl,10\nTreat,20\nCtrl,12\nTreat,24*\n";
    let unstack_filter = ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() };
    let unstacked = filter_delimited_text(long_csv, ',', '.', true, &unstack_filter).unwrap();
    assert_eq!(unstacked.headers, vec!["Ctrl", "Treat"]);
    assert_eq!(unstacked.cells[0][0].active_number(), Some(10.0));
    assert_eq!(unstacked.cells[1][0].active_number(), Some(12.0));
    assert!(unstacked.cells[1][1].excluded);

    // Frozen target rejects import link
    target.frozen = true;
    assert!(matches!(
        apply_csv_import_link(&mut target, "A\n1\n", &spec, true, ImportBaselineAdjustment::None, "lnk-2", "Blocked"),
        Err(FormatError::Doc(DocError::SheetFrozen { .. }))
    ));
}

#[test]
fn forged_zip_header_inflating_past_budget_is_blocked_as_zip_bomb() {
    use std::io::Write;
    let payload = vec![b'A'; 4096];
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(&payload).unwrap();
    let deflated = enc.finish().unwrap();
    let crc = crc32fast::hash(&payload);

    // Craft a ZIP claiming uncompressed_size = 16 while stream inflates to 4096 bytes.
    let name = b"bomb.txt";
    let mut out = Vec::new();
    out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&8_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0x0021_u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&deflated);

    let cd_offset = out.len() as u32;
    out.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0x0314_u16.to_le_bytes());
    out.extend_from_slice(&20_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&8_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0x0021_u16.to_le_bytes());
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(name);
    let cd_size = (out.len() as u32) - cd_offset;

    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());

    let limits = ZipLimits { max_entries: 16, max_total_uncompressed_bytes: 512, max_entry_uncompressed_bytes: 512 };
    assert!(matches!(lumencraft_format::ZipArchive::from_bytes_with_limits(&out, limits), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn tsv_and_csv_round_trip_multiple_archetypes() {
    let tsv_opts = CsvOptions::tsv();
    let xy_tsv = "Conc\tResp:Y1\tResp:Y2\n1\t10.5\t11.0\n10\t52.3*\t54.1\n";
    let xy_sheet = import_csv_to_datasheet("xy-1", "XY TSV", xy_tsv, TableKind::Xy, SubcolumnMode::RawReplicates(2), &tsv_opts).unwrap();
    assert_eq!(xy_sheet.cell(CellCoord::new(1, 0, 0)).unwrap().active_number(), None);
    assert!(xy_sheet.cell(CellCoord::new(1, 0, 0)).unwrap().excluded);
    let exported = export_datasheet_to_csv(&xy_sheet, &tsv_opts).unwrap();
    let reimported = import_csv_to_datasheet("xy-2", "XY TSV", &exported, TableKind::Xy, SubcolumnMode::RawReplicates(2), &tsv_opts).unwrap();
    assert_eq!(xy_sheet.archetype, reimported.archetype);

    let mv_csv = "Subject,Dose,Outcome\nS1,10,Responder\nS2,20,NonResponder\n";
    let mv_sheet =
        import_csv_to_datasheet("mv-1", "Cohort", mv_csv, TableKind::MultipleVariables, SubcolumnMode::Single, &CsvOptions::csv()).unwrap();
    let mv_out = export_datasheet_to_csv(&mv_sheet, &CsvOptions::csv()).unwrap();
    let mv_back =
        import_csv_to_datasheet("mv-2", "Cohort", &mv_out, TableKind::MultipleVariables, SubcolumnMode::Single, &CsvOptions::csv()).unwrap();
    assert_eq!(mv_sheet.archetype, mv_back.archetype);
}

fn raw_datasheet(id: &str, archetype: TableArchetype) -> DataSheet {
    DataSheet {
        id: SheetId::new(id),
        title: format!("Sheet {id}"),
        archetype,
        frozen: false,
        info_sheet_id: None,
        linked_regions: Vec::new(),
        floating_notes: Vec::new(),
        color_tag: None,
        exclusion_notes: Vec::new(),
    }
}

fn wrap_project_zip(doc_json: &serde_json::Value) -> Result<Vec<u8>, FormatError> {
    let doc_bytes = serde_json::to_vec(doc_json)?;
    let manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": "Forged",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let mut w = ZipWriter::new();
    w.add_file("document.json", doc_bytes)?;
    w.add_file("manifest.json", serde_json::to_vec(&manifest)?)?;
    w.finish()
}

#[test]
fn r1_s05_01_archive_validation_and_open_reject_ragged_and_disallowed_tables() {
    let mut mv = MultipleVariablesTable::new(2, 4).expect("mv");
    mv.variables[1].cells.truncate(2);
    let mut doc = ProjectDocument::new("Ragged").expect("doc");
    doc.data_sheets.push(raw_datasheet("mv-1", TableArchetype::MultipleVariables(mv)));
    assert!(matches!(validate_project_document(&doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 1).expect("xy");
    xy.data_sets[1].cells.push(vec![Cell::number(3.0)]);
    let mut xy_doc = ProjectDocument::new("RaggedXY").expect("doc");
    xy_doc.data_sheets.push(raw_datasheet("xy-1", TableArchetype::Xy(xy)));
    assert!(matches!(validate_project_document(&xy_doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 1, 2).expect("grp");
    grp.data_sets[0].cells[1].pop();
    let mut grp_doc = ProjectDocument::new("RaggedGrp").expect("doc");
    grp_doc.data_sheets.push(raw_datasheet("grp-1", TableArchetype::Grouped(grp)));
    assert!(matches!(validate_project_document(&grp_doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut nested = NestedTable::new(1, 2, 2).expect("nested");
    nested.groups[0].cells[0].pop();
    let mut nest_doc = ProjectDocument::new("RaggedNest").expect("doc");
    nest_doc.data_sheets.push(raw_datasheet("nest-1", TableArchetype::Nested(nested)));
    assert!(matches!(validate_project_document(&nest_doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut surv = SurvivalTable::new(1, 2).expect("surv");
    surv.times.pop();
    let mut surv_doc = ProjectDocument::new("RaggedSurv").expect("doc");
    surv_doc.data_sheets.push(raw_datasheet("surv-1", TableArchetype::Survival(surv)));
    assert!(matches!(validate_project_document(&surv_doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut pow = PartsOfWholeTable::new(1, 2).expect("pow");
    pow.slice_labels.pop();
    let mut pow_doc = ProjectDocument::new("RaggedPow").expect("doc");
    pow_doc.data_sheets.push(raw_datasheet("pow-1", TableArchetype::PartsOfWhole(pow)));
    assert!(matches!(validate_project_document(&pow_doc), Err(FormatError::Doc(DocError::InvalidCellValue { .. }))));

    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).expect("col");
    col.subcolumn_mode = SubcolumnMode::RawReplicates(3);
    let mut col_doc = ProjectDocument::new("BadColMode").expect("doc");
    col_doc.data_sheets.push(raw_datasheet("col-1", TableArchetype::Column(col)));
    assert!(matches!(validate_project_document(&col_doc), Err(FormatError::Doc(DocError::IncompatibleSubcolumnMode { .. }))));

    let mut xy_nan_seq = XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy");
    xy_nan_seq.x_mode = XColumnMode::GeneratedSequence { start: f64::NAN, step: 1.0, progression: SequenceProgression::Arithmetic };
    let mut seq_doc = ProjectDocument::new("NanSeq").expect("doc");
    seq_doc.data_sheets.push(raw_datasheet("seq-1", TableArchetype::Xy(xy_nan_seq)));
    assert!(matches!(validate_project_document(&seq_doc), Err(FormatError::NonFiniteFloat { .. })));

    // Crafted archive JSON with ragged MultipleVariables must be rejected by open_project_bytes.
    let forged_mv_bytes = wrap_project_zip(&serde_json::json!({
        "schema_version": CURRENT_SCHEMA_VERSION,
        "title": "Forged",
        "data_sheets": [{
            "id": "d1",
            "title": "MV",
            "archetype": {
                "MultipleVariables": {
                    "row_titles": ["a", "b"],
                    "row_titles_expanded": false,
                    "variables": [
                        { "title": "Y", "role": "Continuous", "source": "Entered", "cells": [{"value":"Empty","excluded":false}, {"value":"Empty","excluded":false}], "category_order": null },
                        { "title": "G", "role": "Categorical", "source": "Entered", "cells": [{"value":"Empty","excluded":false}], "category_order": null }
                    ]
                }
            }
        }],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": []
    }))
    .expect("wrap mv zip");
    assert!(open_project_bytes(&forged_mv_bytes).is_err());
}

#[test]
fn r1_s05_03_x_column_mode_rejected_by_archive_csv_export_and_placement() {
    for bad_sub in [0_usize, MAX_SUBCOLUMNS + 1, usize::MAX] {
        let mut xy = XyTable::new(SubcolumnMode::Single, 1, 2).expect("xy");
        xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::RawReplicates(bad_sub) };
        let mut sheet = raw_datasheet("xy-bad-x", TableArchetype::Xy(xy));

        let mut doc = ProjectDocument::new("BadXMode").expect("doc");
        doc.data_sheets.push(sheet.clone());
        assert!(matches!(validate_project_document(&doc), Err(FormatError::Doc(_))));
        assert!(matches!(export_datasheet_to_csv(&sheet, &CsvOptions::csv()), Err(FormatError::Doc(_))));

        let spec = ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("x.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        };
        assert!(matches!(
            apply_csv_import_link(&mut sheet, "X,Y\n1,2\n3,4\n", &spec, true, ImportBaselineAdjustment::None, "lnk", "test"),
            Err(FormatError::Doc(_))
        ));
    }

    // Archive JSON containing RawReplicates(usize::MAX) in x_mode is rejected cleanly on open_project_bytes.
    let forged_xmode_bytes = wrap_project_zip(&serde_json::json!({
        "schema_version": CURRENT_SCHEMA_VERSION,
        "title": "ForgedX",
        "data_sheets": [{
            "id": "xy1",
            "title": "XY",
            "archetype": {
                "Xy": {
                    "x_title": "X",
                    "x_mode": { "Numbers": { "subcolumn_mode": { "RawReplicates": usize::MAX } } },
                    "y_subcolumn_mode": "Single",
                    "row_titles": ["", ""],
                    "row_titles_expanded": false,
                    "x_cells": [[{"value":"Empty","excluded":false}], [{"value":"Empty","excluded":false}]],
                    "data_sets": [{ "title": "A", "cells": [[{"value":"Empty","excluded":false}], [{"value":"Empty","excluded":false}]] }]
                }
            }
        }],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": []
    }))
    .expect("wrap xmode zip");
    assert!(open_project_bytes(&forged_xmode_bytes).is_err());
}
