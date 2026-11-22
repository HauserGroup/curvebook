#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_doc::{
    AnalysisKind, AnalysisSpec, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellValue, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable,
    DataSheet, DocError, ErrorCode, FloatingNote, GraphAnnotation, GraphSheet, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec,
    InfoConstant, InfoSheet, LayoutItem, LayoutSheet, LayoutSpec, LinearRegressionSpec, LinkSource, MAX_TITLE_BYTES, MultipleVariablesTable,
    NestedGroupColumn, NestedTable, NonlinearFitSpec, NormalizeAnchor, OneWayAnovaSpec, ParameterRule, ParameterScope, PartsOfWholeTable,
    PlacementMode, ProjectDocument, RectPt, ResultCell, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SequenceProgression, SheetId,
    SourceCellId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, TransformAnalysisSpec, XColumnMode, XyTable,
};
use curvebook_format::recovery::{CrashRecoveryJournal, DirectoryJournalBackend, JOURNAL_FILE_NAME};
use curvebook_format::{
    CsvOptions, ExportOptions, ExportSource, FormatError, FormatRegistry, ImportBaselineAdjustment, ZipWriter, apply_csv_import_link,
    ensure_result_sheet_csv_exportable, export_datasheet_to_csv, export_result_sheet_json, export_result_sheet_tidy_csv, export_result_tab_csv,
    import_csv_to_datasheet, import_prism_bytes, import_pzfx_str, open_project_bytes, parse_delimited_records, save_project_bytes,
    try_export_result_sheet_tidy_csv, try_export_result_tab_csv, validate_project_document,
};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("cb_audit_p3_sec04_{prefix}_{}_{}", std::process::id(), seq));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { path: dir }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn rnum(value: f64) -> ResultCell {
    ResultCell::Number { value, prefix: None, units: None }
}

fn excl_text(text: &str) -> Cell {
    Cell { value: CellValue::Text(String::from(text)), excluded: true }
}

fn excl_number(value: f64) -> Cell {
    Cell { value: CellValue::Number(value), excluded: true }
}

fn make_failed_result_sheet(id: &str, source_id: &str) -> ResultSheet {
    let mut rs = ResultSheet::new(AnalysisSpec::new(id, source_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    rs.status = ResultStatus::Failed { code: ErrorCode::BadInput, message: String::from("synthetic failed analysis") };
    rs.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("R1")],
        rows: vec![vec![ResultCell::Text(String::from("stale")), rnum(42.0)]],
        footnotes: Vec::new(),
    });
    rs
}

fn make_valid_result_sheet(id: &str, source_id: &str, dirty: bool) -> ResultSheet {
    let mut rs = ResultSheet::new(AnalysisSpec::new(id, source_id, AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    rs.status = if dirty { ResultStatus::Dirty } else { ResultStatus::UpToDate };
    rs.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("Summary"),
        kind: ResultTabKind::MainSummary,
        column_headers: vec![String::from("Metric"), String::from("Value")],
        row_labels: vec![String::from("Mean")],
        rows: vec![vec![ResultCell::Text(String::from("Alpha")), rnum(3.25)]],
        footnotes: Vec::new(),
    });
    rs
}

fn build_zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new();
    for (name, bytes) in entries {
        writer.add_file(name, bytes.to_vec()).unwrap();
    }
    writer.finish().unwrap()
}

// ============================================================================
// FA-SEC04-04: Public result CSV re-exports & FormatRegistry Failed refusal
// ============================================================================

#[test]
fn fa_sec04_04_result_csv_reexports_and_registry_refuse_failed_status() {
    let mut doc = ProjectDocument::new("Failed Export Check").unwrap();
    let ds = DataSheet::new("d1", "Data 1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap())).unwrap();
    doc.add_data_sheet(ds).unwrap();

    let failed = make_failed_result_sheet("r_fail", "d1");
    doc.add_result_sheet(failed.clone()).unwrap();
    doc.active_sheet_id = Some(SheetId::from("r_fail"));

    assert!(matches!(ensure_result_sheet_csv_exportable(&failed), Err(FormatError::InvalidPipeline { .. })));
    assert!(matches!(try_export_result_tab_csv(&failed, &failed.tabs[0], &CsvOptions::csv()), Err(FormatError::InvalidPipeline { .. })));
    assert!(matches!(try_export_result_sheet_tidy_csv(&failed, &CsvOptions::csv()), Err(FormatError::InvalidPipeline { .. })));

    let mut rs_only_doc = ProjectDocument::new("Failed Only").unwrap();
    rs_only_doc.result_sheets.push(failed.clone());

    let reg = FormatRegistry::default_registry();
    for fmt in ["csv", "tsv"] {
        let exporter = reg.exporter_by_format(fmt).unwrap();
        assert!(
            matches!(exporter.export_bytes(ExportSource::ResultSheet(&failed), &ExportOptions::default()), Err(FormatError::InvalidPipeline { .. })),
            "{fmt} ExportSource::ResultSheet must refuse Failed result sheet"
        );
        assert!(
            matches!(
                exporter.export_bytes(
                    ExportSource::Document(&doc),
                    &ExportOptions { sheet_id: Some(String::from("r_fail")), ..ExportOptions::default() }
                ),
                Err(FormatError::InvalidPipeline { .. })
            ),
            "{fmt} ExportSource::Document with explicit sheet_id must refuse Failed result sheet"
        );
        assert!(
            matches!(
                exporter.export_bytes(ExportSource::Document(&rs_only_doc), &ExportOptions::default()),
                Err(FormatError::InvalidPipeline { .. })
            ),
            "{fmt} ExportSource::Document fallback to first result_sheet must refuse Failed result sheet"
        );
    }

    // Positive controls: UpToDate & Dirty export cleanly via try_* helpers, registry, and legacy helpers
    for (idx, dirty) in [false, true].into_iter().enumerate() {
        let ok_rs = make_valid_result_sheet(&format!("r_ok_{idx}"), "d1", dirty);
        ensure_result_sheet_csv_exportable(&ok_rs).unwrap();
        let tab_csv = try_export_result_tab_csv(&ok_rs, &ok_rs.tabs[0], &CsvOptions::csv()).unwrap();
        assert!(tab_csv.contains("Alpha,3.25"));
        let tidy_csv = try_export_result_sheet_tidy_csv(&ok_rs, &CsvOptions::csv()).unwrap();
        assert!(
            tidy_csv.contains("summary")
                && tidy_csv.contains("Summary")
                && tidy_csv.contains("Mean")
                && tidy_csv.contains("Alpha")
                && tidy_csv.contains("3.25")
        );
        // Legacy un-deprecated helpers remain callable and consistent
        assert_eq!(export_result_tab_csv(&ok_rs.tabs[0], &CsvOptions::csv()), tab_csv);
        assert_eq!(export_result_sheet_tidy_csv(&ok_rs, &CsvOptions::csv()), tidy_csv);

        let mut ok_doc = ProjectDocument::new("Valid Export Check").unwrap();
        let ds_ok =
            DataSheet::new("d1", "Data 1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap())).unwrap();
        ok_doc.add_data_sheet(ds_ok).unwrap();
        ok_doc.add_result_sheet(ok_rs.clone()).unwrap();
        for fmt in ["csv", "tsv"] {
            let exporter = reg.exporter_by_format(fmt).unwrap();
            assert!(exporter.export_bytes(ExportSource::ResultSheet(&ok_rs), &ExportOptions::default()).is_ok());
            assert!(exporter.export_bytes(ExportSource::Document(&ok_doc), &ExportOptions::default()).is_ok());
        }
    }
}

// ============================================================================
// FA-SEC04-05: Streaming migration/import bypasses 1M DOM limit on valid >250k
// ============================================================================

#[test]
fn fa_sec04_05_large_v0_v6_and_prism_projects_load_while_bombs_fail() {
    // 140_000 rows x 2 cols = 280_000 cells (> 1_120_000 JSON DOM nodes if materialized as Value)
    // plus a ResultSheet with 120_000 ResultTab rows x 3 cols = 360_000 result cells (> 1M nodes)
    let mut big_doc = ProjectDocument::new("Large Legacy Study").unwrap();
    let mut xy = XyTable::new(SubcolumnMode::Single, 1, 140_000).unwrap();
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[139_999][0] = Cell::number(140_000.0);
    xy.data_sets[0].cells[0][0] = Cell::number(2.5);
    xy.data_sets[0].cells[139_999][0] = Cell::number(99.5);
    let ds = DataSheet::new("d1", "Big XY", TableArchetype::Xy(xy)).unwrap();
    big_doc.add_data_sheet(ds).unwrap();

    let mut rs = ResultSheet::new(AnalysisSpec::new("r1", "d1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    rs.tabs.push(ResultTab {
        id: String::from("big_tab"),
        title: String::from("Big Tab"),
        kind: ResultTabKind::Residuals,
        column_headers: vec![String::from("A"), String::from("B"), String::from("C")],
        row_labels: (0..120_000).map(|i| format!("r{i}")).collect(),
        rows: vec![vec![rnum(1.0), rnum(2.0), rnum(3.0)]; 120_000],
        footnotes: Vec::new(),
    });
    big_doc.add_result_sheet(rs).unwrap();

    for legacy_ver in [0_u32, 6_u32] {
        let mut doc_json = serde_json::to_value(&big_doc).unwrap();
        doc_json["schema_version"] = serde_json::json!(legacy_ver);
        let doc_bytes = serde_json::to_vec(&doc_json).unwrap();
        let manifest_bytes = serde_json::to_vec(&serde_json::json!({
            "format": "curvebook-archive",
            "schema_version": legacy_ver,
            "kind": "Project",
            "title": "Large Legacy Study",
            "document_crc32": crc32fast::hash(&doc_bytes),
            "sheets": []
        }))
        .unwrap();
        let zip = build_zip_entries(&[("manifest.json", &manifest_bytes), ("document.json", &doc_bytes)]);
        let opened = open_project_bytes(&zip).unwrap();
        assert_eq!(opened.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(opened.data_sheets[0].archetype.row_count(), 140_000);
        assert_eq!(opened.result_sheets[0].tabs[0].rows.len(), 120_000);
    }

    // Embedded v6 document.json inside .prism ZIP
    let mut prism_json = serde_json::to_value(&big_doc).unwrap();
    prism_json["schema_version"] = serde_json::json!(6);
    let prism_doc_bytes = serde_json::to_vec(&prism_json).unwrap();
    let prism_zip = build_zip_entries(&[("document.json", &prism_doc_bytes)]);
    let report = import_prism_bytes(&prism_zip).unwrap();
    assert_eq!(report.document.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(report.document.data_sheets[0].archetype.row_count(), 140_000);
    assert_eq!(report.document.result_sheets[0].tabs[0].rows.len(), 120_000);

    // Malicious JSON amplification bombs still rejected cleanly
    let mut top_bomb = String::from("{\"schema_version\":0,\"title\":\"Bomb\",\"bomb\":[");
    for i in 0..1_005_000_usize {
        if i > 0 {
            top_bomb.push(',');
        }
        top_bomb.push_str("[]");
    }
    top_bomb.push_str("],\"data_sheets\":[],\"info_sheets\":[],\"result_sheets\":[],\"graph_sheets\":[],\"layout_sheets\":[]}");
    let bomb_bytes = top_bomb.as_bytes();
    let manifest_v0 = serde_json::to_vec(&serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": 0,
        "kind": "Project",
        "title": "Bomb",
        "document_crc32": crc32fast::hash(bomb_bytes),
        "sheets": []
    }))
    .unwrap();
    let bomb_zip = build_zip_entries(&[("manifest.json", &manifest_v0), ("document.json", bomb_bytes)]);
    assert!(matches!(open_project_bytes(&bomb_zip), Err(FormatError::Json(_))));

    // Embedded .prism top-level array bomb > 1M nodes
    let mut array_bomb = String::from("[");
    for i in 0..1_005_000_usize {
        if i > 0 {
            array_bomb.push(',');
        }
        array_bomb.push_str("[]");
    }
    array_bomb.push(']');
    let prism_bomb_zip = build_zip_entries(&[("document.json", array_bomb.as_bytes())]);
    assert!(matches!(import_prism_bytes(&prism_bomb_zip), Err(FormatError::Json(_))));

    // Sequence amplification inside data_sheets (> MAX_ROWS entries in x_cells) rejected
    let mut seq_bomb = String::from(
        "{\"schema_version\":0,\"title\":\"SeqBomb\",\"data_sheets\":[{\"id\":\"d1\",\"title\":\"D1\",\"archetype\":{\"Xy\":{\"x_title\":\"X\",\"x_mode\":\"Numeric\",\"y_subcolumn_mode\":\"Single\",\"x_cells\":[",
    );
    for i in 0..1_000_005_usize {
        if i > 0 {
            seq_bomb.push(',');
        }
        seq_bomb.push_str("[]");
    }
    seq_bomb.push_str("],\"row_titles\":[],\"data_sets\":[]}}}],\"info_sheets\":[],\"result_sheets\":[],\"graph_sheets\":[],\"layout_sheets\":[]}");
    let seq_manifest = serde_json::to_vec(&serde_json::json!({
        "format": "curvebook-archive",
        "schema_version": 0,
        "kind": "Project",
        "title": "SeqBomb",
        "document_crc32": crc32fast::hash(seq_bomb.as_bytes()),
        "sheets": []
    }))
    .unwrap();
    let seq_bomb_zip = build_zip_entries(&[("manifest.json", &seq_manifest), ("document.json", seq_bomb.as_bytes())]);
    assert!(open_project_bytes(&seq_bomb_zip).is_err());
}

// ============================================================================
// FA-SEC04-06: GeneratedSequence x_cells carrier rectangularity & budget
// ============================================================================

#[test]
fn fa_sec04_06_generated_sequence_x_cells_rectangular_and_budgeted() {
    let x_mode = XColumnMode::GeneratedSequence { start: 0.0, step: 1.0, progression: SequenceProgression::Arithmetic };
    let xy = XyTable::new_with_x_mode(x_mode.clone(), SubcolumnMode::Single, 1, 2).unwrap();
    let mut sheet = DataSheet::new("d_gen", "GenSeq", TableArchetype::Xy(xy)).unwrap();
    sheet.set_x_or_time_excluded(0, 0, true).unwrap();

    // AppendRows keeps x_cells rectangular (len == row_count, each inner len == 1)
    let append_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::AppendRows, anchor: CellCoord::new(0, 0, 0), ..ImportPlacementSpec::default() },
    };
    apply_csv_import_link(&mut sheet, "10\n20\n", &append_spec, false, ImportBaselineAdjustment::None, "l1", "append").unwrap();
    sheet.validate().unwrap();
    if let TableArchetype::Xy(ref t) = sheet.archetype {
        assert_eq!(t.x_cells.len(), 4);
        assert!(t.x_cells.iter().all(|row| row.len() == 1));
        assert!(t.x_cells[0][0].excluded);
        assert!(!t.x_cells[2][0].excluded);
    } else {
        panic!("expected XyTable");
    }

    // InsertRows shifts x_cells down and keeps row widths == 1
    let insert_spec = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::InsertRows, anchor: CellCoord::new(1, 0, 0), ..ImportPlacementSpec::default() },
    };
    apply_csv_import_link(&mut sheet, "5\n", &insert_spec, false, ImportBaselineAdjustment::None, "l2", "insert").unwrap();
    sheet.validate().unwrap();
    if let TableArchetype::Xy(ref t) = sheet.archetype {
        assert_eq!(t.x_cells.len(), 5);
        assert!(t.x_cells.iter().all(|row| row.len() == 1));
    }

    let mut doc = ProjectDocument::new("GenSeq Save").unwrap();
    doc.add_data_sheet(sheet).unwrap();
    let bytes = save_project_bytes(&doc).unwrap();
    let reopened = open_project_bytes(&bytes).unwrap();
    reopened.data_sheets[0].validate().unwrap();

    // Budget accounting includes x_width == 1 when GeneratedSequence carries an exclusion vector:
    // 20 datasets * 50 replicates = 1_000 Y cells/row + 1 X cell/row = 1_001 cells/row.
    // At 9_990 rows: 9_990 * 1_001 = 9_999_990 <= MAX_IMPORT_CELLS (10_000_000).
    // Growing by 2 rows yields 9_992 * 1_000 = 9_992_000 (without x_width) vs 9_992 * 1_001 = 10_001_992 (> 10_000_000).
    let xy_big = XyTable::new_with_x_mode(x_mode, SubcolumnMode::RawReplicates(50), 20, 9_990).unwrap();
    let mut big_sheet = DataSheet::new("d_big", "BigGen", TableArchetype::Xy(xy_big)).unwrap();
    big_sheet.set_x_or_time_excluded(0, 0, true).unwrap();
    let before = big_sheet.clone();
    let err = apply_csv_import_link(&mut big_sheet, "1\n2\n", &append_spec, false, ImportBaselineAdjustment::None, "l_oom", "oom").unwrap_err();
    assert!(matches!(err, FormatError::InvalidPipeline { .. }));
    assert_eq!(big_sheet, before);
}

// ============================================================================
// FA-SEC04-07: Nested biological_unit_count traversal + subcol bounds
// ============================================================================

#[test]
fn fa_sec04_07_nested_biological_units_and_subcolumn_bounds() {
    let nested = NestedTable {
        groups: vec![NestedGroupColumn::new("Control", 2, 2).unwrap(), NestedGroupColumn::new("Treated", 3, 2).unwrap()],
        row_titles: vec![String::new(); 2],
        row_titles_expanded: false,
    };
    let mut sheet = DataSheet::new("n1", "Nested", TableArchetype::Nested(nested)).unwrap();

    // Importing 6 columns at (0,0,0) fills group 0 (subcols 0..2), group 1 (subcols 0..3), and adds group 2 (1 unit)
    let spec_fill = ImportLinkSpec { source: LinkSource::Clipboard, filter: ImportFilterSpec::default(), placement: ImportPlacementSpec::default() };
    let range = apply_csv_import_link(&mut sheet, "1,2,3,4,5,6\n", &spec_fill, false, ImportBaselineAdjustment::None, "l1", "fill").unwrap();
    assert_eq!(range.end, CellCoord::new(0, 2, 0));
    if let TableArchetype::Nested(ref t) = sheet.archetype {
        assert_eq!(t.groups.len(), 3);
        assert_eq!(t.groups[0].cells[0], vec![Cell::number(1.0), Cell::number(2.0)]);
        assert_eq!(t.groups[1].cells[0], vec![Cell::number(3.0), Cell::number(4.0), Cell::number(5.0)]);
        assert_eq!(t.groups[2].biological_unit_count(), 1);
        assert_eq!(t.groups[2].cells[0], vec![Cell::number(6.0)]);
    } else {
        panic!("expected NestedTable");
    }

    // Starting at (row=1, col=0, subcol=1) writes sequentially into (0,1) then (1,0)
    let spec_mid = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { anchor: CellCoord::new(1, 0, 1), ..ImportPlacementSpec::default() },
    };
    apply_csv_import_link(&mut sheet, "10,20\n", &spec_mid, false, ImportBaselineAdjustment::None, "l2", "mid").unwrap();
    if let TableArchetype::Nested(ref t) = sheet.archetype {
        assert_eq!(t.groups[0].cells[1][1], Cell::number(10.0));
        assert_eq!(t.groups[1].cells[1][0], Cell::number(20.0));
    }

    // Out-of-bounds anchor.subcol must fail with CellOutOfBounds across Nested + Single/Fixed archetypes
    let oob_spec = |subcol: usize| ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { anchor: CellCoord::new(0, 0, subcol), ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_csv_import_link(&mut sheet, "99\n", &oob_spec(2), false, ImportBaselineAdjustment::None, "oob", "oob"),
        Err(FormatError::Doc(DocError::CellOutOfBounds { col: 0, subcol: 2, .. }))
    ));

    let single_sheets = [
        DataSheet::new("xy1", "XY", TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 2).unwrap())).unwrap(),
        DataSheet::new("col1", "Col", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap())).unwrap(),
        DataSheet::new("con1", "Con", TableArchetype::Contingency(ContingencyTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("sur1", "Sur", TableArchetype::Survival(SurvivalTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("pow1", "Pow", TableArchetype::PartsOfWhole(PartsOfWholeTable::new(2, 2).unwrap())).unwrap(),
        DataSheet::new("mv1", "MV", TableArchetype::MultipleVariables(MultipleVariablesTable::new(2, 2).unwrap())).unwrap(),
    ];
    for mut s in single_sheets {
        let before = s.clone();
        let err = apply_csv_import_link(&mut s, "99\n", &oob_spec(1), false, ImportBaselineAdjustment::None, "oob", "oob").unwrap_err();
        assert!(matches!(err, FormatError::Doc(DocError::CellOutOfBounds { subcol: 1, .. })), "unexpected err for {}: {err:?}", s.id);
        assert_eq!(s, before);
    }
}

// ============================================================================
// FA-SEC04-08: apply_csv_import_link error atomicity
// ============================================================================

#[test]
fn fa_sec04_08_apply_csv_import_link_is_atomic_on_error() {
    let mut xy = XyTable::new(SubcolumnMode::Single, 2, 2).unwrap();
    xy.data_sets[0].title = String::from("OriginalA");
    xy.data_sets[1].title = String::from("OriginalB");
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[1].cells[1][0] = Cell::number(20.0);
    let mut sheet = DataSheet::new("d_atomic", "Atomic", TableArchetype::Xy(xy)).unwrap();
    let before = sheet.clone();

    let too_long_title = "X".repeat(MAX_TITLE_BYTES + 10);
    let csv_bad_hdr = format!("ValidNewA,{too_long_title}\n100,200\n");
    let spec_insert_hdr = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec {
            mode: PlacementMode::InsertRows,
            anchor: CellCoord::new(0, 0, 0),
            propagate_title_header: true,
            ..ImportPlacementSpec::default()
        },
    };
    let err = apply_csv_import_link(&mut sheet, &csv_bad_hdr, &spec_insert_hdr, true, ImportBaselineAdjustment::None, "l_err", "err").unwrap_err();
    assert!(matches!(err, FormatError::Doc(DocError::TitleTooLong { .. })));
    assert_eq!(sheet, before, "InsertRows + bad second header must not mutate DataSheet");

    // Same atomicity guarantee on subcolumn-out-of-bounds during InsertRows
    let spec_insert_oob = ImportLinkSpec {
        source: LinkSource::Clipboard,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::InsertRows, anchor: CellCoord::new(0, 0, 1), ..ImportPlacementSpec::default() },
    };
    assert!(apply_csv_import_link(&mut sheet, "100\n", &spec_insert_oob, false, ImportBaselineAdjustment::None, "l_oob", "oob").is_err());
    assert_eq!(sheet, before);
}

// ============================================================================
// FA-SEC04-09: Excluded CellValue::Text CSV round-tripping
// ============================================================================

#[test]
fn fa_sec04_09_excluded_text_cells_round_trip_losslessly_in_valid_csv() {
    let cases = [
        ("ctrl", "ctrl"),
        ("0012", "0012"),
        ("1.5", "1.5"),
        ("-42", "'-42"),
        ("NA", "NA"),
        ("NaN", "NaN"),
        ("NULL", "NULL"),
        (".", "."),
        (" a ", " a "),
        ("", ""),
        ("a,b", "a,b"),
        ("line1\nline2", "line1\nline2"),
        ("say \"hi\"", "say \"hi\""),
        ("a*", "a*"),
        ("*a", "*a"),
        ("*", "*"),
        ("\"", "\""),
        ("=cmd", "'=cmd"),
        ("1,5", "1,5"),
    ];

    let mut tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, cases.len()).unwrap();
    tbl.data_sets[0].title = String::from("ExcludedText");
    tbl.data_sets[1].title = String::from("ExcludedNumber");
    tbl.data_sets[2].title = String::from("NormalText");
    for (r, &(txt, _)) in cases.iter().enumerate() {
        tbl.data_sets[0].cells[r][0] = excl_text(txt);
        tbl.data_sets[1].cells[r][0] = excl_number((r as f64) + 1.5);
        tbl.data_sets[2].cells[r][0] = Cell::text(txt);
    }
    let sheet = DataSheet::new("d_ex", "Excluded", TableArchetype::Column(tbl)).unwrap();

    for (label, opts) in [("us_csv", CsvOptions::csv()), ("eu_semicolon", CsvOptions { delimiter: ';', decimal_separator: ',', ..CsvOptions::csv() })]
    {
        let exported = export_datasheet_to_csv(&sheet, &opts).unwrap();
        // Every exported CSV must remain 100% RFC-4180 valid!
        parse_delimited_records(&exported, &opts).unwrap_or_else(|e| panic!("{label} exported invalid CSV: {e}\n{exported}"));

        let reimported =
            import_csv_to_datasheet("d_re", "Re", &exported, TableKind::Column, SubcolumnMode::Single, &CsvOptions { header_rows: 1, ..opts })
                .unwrap();
        for (r, &(_, expected_after_formula_neutralize)) in cases.iter().enumerate() {
            assert_eq!(
                reimported.cell(CellCoord::new(r, 0, 0)),
                Some(&excl_text(expected_after_formula_neutralize)),
                "{label} excluded text mismatch at row {r} ({expected_after_formula_neutralize:?})\ncsv={exported}"
            );
            assert_eq!(reimported.cell(CellCoord::new(r, 1, 0)), Some(&excl_number((r as f64) + 1.5)), "{label} excluded number mismatch at row {r}");
            assert_eq!(
                reimported.cell(CellCoord::new(r, 2, 0)),
                Some(&Cell::text(expected_after_formula_neutralize)),
                "{label} unexcluded text mismatch at row {r}"
            );
        }
    }
}

// ============================================================================
// FA-SEC04-10: export_result_sheet_json rejects non-finite sheet.analysis
// ============================================================================

#[test]
fn fa_sec04_10_export_result_sheet_json_rejects_non_finite_analysis_floats() {
    let mut bad_lin =
        ResultSheet::new(AnalysisSpec::new("r_lin", "d1", AnalysisKind::LinearRegression(LinearRegressionSpec::default())).unwrap()).unwrap();
    if let AnalysisKind::LinearRegression(ref mut lin) = bad_lin.analysis.kind {
        lin.plot_x_min = Some(f64::NAN);
    }

    let mut bad_norm = ResultSheet::new(
        AnalysisSpec::new(
            "r_norm",
            "d1",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::FixedValue(0.0),
                hundred_anchor: NormalizeAnchor::ColumnMax,
                output_percent: true,
                per_data_set: true,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    if let AnalysisKind::Transform(TransformAnalysisSpec::Normalize { ref mut zero_anchor, .. }) = bad_norm.analysis.kind {
        *zero_anchor = NormalizeAnchor::FixedValue(f64::INFINITY);
    }

    let mut nl_spec = NonlinearFitSpec::new("one_site_binding");
    nl_spec.parameter_rules.push(ParameterRule {
        name: String::from("Bmax"),
        scope: ParameterScope::Constant(1.0),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let mut bad_nl = ResultSheet::new(AnalysisSpec::new("r_nl", "d1", AnalysisKind::NonlinearRegression(nl_spec)).unwrap()).unwrap();
    if let AnalysisKind::NonlinearRegression(ref mut spec) = bad_nl.analysis.kind {
        spec.parameter_rules[0].scope = ParameterScope::Constant(f64::NEG_INFINITY);
    }

    let reg = FormatRegistry::default_registry();
    let json_exporter = reg.exporter_by_format("json").unwrap();
    for bad in [&bad_lin, &bad_norm, &bad_nl] {
        assert!(matches!(export_result_sheet_json(bad, false), Err(FormatError::NonFiniteFloat { .. })), "failed to reject {}", bad.id);
        assert!(
            matches!(json_exporter.export_bytes(ExportSource::ResultSheet(bad), &ExportOptions::default()), Err(FormatError::NonFiniteFloat { .. })),
            "registry json failed to reject {}",
            bad.id
        );
    }

    let good = make_valid_result_sheet("r_good", "d1", false);
    assert!(export_result_sheet_json(&good, false).is_ok());
}

// ============================================================================
// FA-SEC04-11: NestedTable .pzfx-origin CSV export/import round-trip
// ============================================================================

#[test]
fn fa_sec04_11_nested_pzfx_csv_round_trip_preserves_groups_and_subcolumns() {
    let pzfx_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<GraphPadPrismFile PrismXMLVersion="5.00">
  <Table ID="Table0" TableType="Nested" Replicates="2">
    <Title>Nested Study</Title>
    <RowTitlesColumn><Subcolumn><d>R1</d><d>R2</d></Subcolumn></RowTitlesColumn>
    <YColumn Width="81" Decimals="1" Subcolumns="2">
      <Title>Control</Title>
      <Subcolumn><Title>Mouse 1</Title><d>10.5</d><d Excluded="true">11.0</d></Subcolumn>
      <Subcolumn><Title>Mouse 2</Title><d>12.5</d><d>13.0</d></Subcolumn>
    </YColumn>
    <YColumn Width="81" Decimals="1" Subcolumns="2">
      <Title>Treated</Title>
      <Subcolumn><Title>Mouse 3</Title><d>20.5</d><d>21.0</d></Subcolumn>
      <Subcolumn><Title>Mouse 4</Title><d>22.5</d><d>23.0</d></Subcolumn>
    </YColumn>
  </Table>
</GraphPadPrismFile>"#;

    let report = import_pzfx_str(pzfx_xml).unwrap();
    assert_eq!(report.document.data_sheets.len(), 1);
    let orig = &report.document.data_sheets[0];
    let exported = export_datasheet_to_csv(orig, &CsvOptions::csv()).unwrap();
    let header_line = exported.lines().next().unwrap();
    assert_eq!(header_line, "Row Title,Control:Mouse 1,Control:Mouse 2,Treated:Mouse 3,Treated:Mouse 4");

    let reimported = import_csv_to_datasheet(
        "re_nested",
        "Nested Reimported",
        &exported,
        TableKind::Nested,
        SubcolumnMode::RawReplicates(2),
        &CsvOptions { header_rows: 1, has_row_titles: true, ..CsvOptions::csv() },
    )
    .unwrap();
    if let (TableArchetype::Nested(a), TableArchetype::Nested(b)) = (&orig.archetype, &reimported.archetype) {
        assert_eq!(a.groups.len(), b.groups.len());
        assert_eq!(b.groups[0].title, "Control");
        assert_eq!(b.groups[0].biological_unit_count(), 2);
        assert_eq!(b.groups[0].subcolumn_titles, vec!["Control:Mouse 1", "Control:Mouse 2"]);
        assert_eq!(b.groups[0].cells, a.groups[0].cells);
        assert!(b.groups[0].cells[1][0].excluded);
        assert_eq!(b.groups[1].title, "Treated");
        assert_eq!(b.groups[1].biological_unit_count(), 2);
        assert_eq!(b.groups[1].subcolumn_titles, vec!["Treated:Mouse 3", "Treated:Mouse 4"]);
        assert_eq!(b.groups[1].cells, a.groups[1].cells);
        assert_eq!(a.row_titles, b.row_titles);
    } else {
        panic!("expected NestedTable");
    }

    // Also test unequal biological_unit_count groups [2, 1] directly
    let mut g0 = NestedGroupColumn::new("Control", 2, 2).unwrap();
    g0.subcolumn_titles = vec![String::from("M1"), String::from("M2")];
    g0.cells[0] = vec![Cell::number(1.0), excl_number(2.0)];
    let mut g1 = NestedGroupColumn::new("Treated", 1, 2).unwrap();
    g1.subcolumn_titles = vec![String::from("M3")];
    g1.cells[0] = vec![Cell::number(3.0)];
    let unequal = DataSheet::new(
        "n_unequal",
        "Unequal",
        TableArchetype::Nested(NestedTable { groups: vec![g0.clone(), g1.clone()], row_titles: vec![String::new(); 2], row_titles_expanded: false }),
    )
    .unwrap();
    let unequal_csv = export_datasheet_to_csv(&unequal, &CsvOptions::csv()).unwrap();
    assert_eq!(unequal_csv.lines().next().unwrap(), "Control:M1,Control:M2,Treated:M3");
    let unequal_re =
        import_csv_to_datasheet("n_u2", "Unequal2", &unequal_csv, TableKind::Nested, SubcolumnMode::RawReplicates(1), &CsvOptions::csv()).unwrap();
    if let TableArchetype::Nested(t) = unequal_re.archetype {
        assert_eq!(t.groups.len(), 2);
        assert_eq!(t.groups[0].biological_unit_count(), 2);
        assert_eq!(t.groups[1].biological_unit_count(), 1);
        assert_eq!(t.groups[0].cells[0], g0.cells[0]);
        assert_eq!(t.groups[1].cells[0], g1.cells[0]);
    } else {
        panic!("expected NestedTable");
    }
}

// ============================================================================
// FA-SEC04-12: .prism SheetId remapping covers hooks, outliers, notes, tokens
// ============================================================================

#[test]
fn fa_sec04_12_prism_sheet_id_remap_covers_hooks_outliers_notes_and_inline_tokens() {
    let xy = XyTable::new(SubcolumnMode::Single, 1, 3).unwrap();
    let ds = DataSheet::new("Data 1", "Data 1", TableArchetype::Xy(xy)).unwrap();

    let mut info = InfoSheet::new_preseeded("Info 1", "Info 1").unwrap();
    info.linked_data_sheets.push(SheetId::from("Data 1"));
    info.constants.push(InfoConstant::new("Conc", ConstantValue::Number(10.0), None).unwrap());

    let rs_src = ResultSheet::new(AnalysisSpec::new("Result 1", "Data 1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();

    let mut rs_norm = ResultSheet::new(
        AnalysisSpec::new(
            "res_norm",
            "Data 1",
            AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
                zero_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant {
                    info_sheet_id: Some(SheetId::from("Info 1")),
                    constant_name: String::from("Conc"),
                }),
                hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar {
                    result_sheet_id: SheetId::from("Result 1"),
                    parameter_key: String::from("EC50"),
                    data_set_index: Some(0),
                }),
                output_percent: true,
                per_data_set: true,
            }),
        )
        .unwrap(),
    )
    .unwrap();
    rs_norm.eliminated_outliers.push(SourceCellId { sheet_id: SheetId::from("Data 1"), row: 1, col: 0, subcol: 0 });

    let mut nl_spec = NonlinearFitSpec::new("one_site_binding");
    nl_spec.parameter_rules.push(ParameterRule {
        name: String::from("Bmax"),
        scope: ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::from("Info 1")),
            constant_name: String::from("Conc"),
        }),
        initial_override: None,
        lower_bound: None,
        upper_bound: None,
    });
    let rs_nl = ResultSheet::new(AnalysisSpec::new("res_nl", "Data 1", AnalysisKind::NonlinearRegression(nl_spec)).unwrap()).unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data("Graph 1", &ds).unwrap();
    gs.spec.title = String::from("Title {{info:Info 1:Conc}} / {{result:Result 1:0:EC50}}");
    gs.spec.x_axis.title = String::from("X {{info:Info 1:Conc}}");
    gs.spec.y_axis.title = String::from("Y {{result:Result 1:EC50}}");
    if let Some(ref mut y2) = gs.spec.y2_axis {
        y2.title = String::from("Y2 {{info:Info 1:Conc}}");
    }
    gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 100.0, 40.0),
        markdown: String::from("Box {{info:Info 1:Conc}}"),
        bordered: true,
    });
    gs.floating_notes.push(FloatingNote::new("n1", RectPt::new(5.0, 5.0, 50.0, 20.0), "Note {{result:Result 1:0:EC50}}"));

    let mut layout_spec = LayoutSpec::default();
    layout_spec.items.push(LayoutItem::TextCallout {
        id: String::from("tc1"),
        markdown: String::from("Layout {{info:Info 1:Conc}} + {{result:Result 1:EC50}}"),
        bounds_pt: RectPt::new(0.0, 0.0, 120.0, 40.0),
        font_size_pt: 10.0,
    });
    let ls = LayoutSheet::new("Layout 1", "Layout 1", layout_spec).unwrap();

    let mut doc = ProjectDocument::new("Remap Test").unwrap();
    doc.data_sheets.push(ds);
    doc.info_sheets.push(info);
    doc.result_sheets.push(rs_src);
    doc.result_sheets.push(rs_norm);
    doc.result_sheets.push(rs_nl);
    doc.graph_sheets.push(gs);
    doc.layout_sheets.push(ls);

    let zip = build_zip_entries(&[("document.json", &serde_json::to_vec(&doc).unwrap())]);
    let report = import_prism_bytes(&zip).unwrap();
    let imported = &report.document;

    imported.validate().unwrap();
    validate_project_document(imported).unwrap();

    let new_data_id = imported.data_sheets[0].id.clone();
    let new_info_id = imported.info_sheets[0].id.clone();
    let new_rs_src_id = imported.result_sheets[0].id.clone();
    assert_ne!(new_data_id.as_str(), "Data 1");
    assert_ne!(new_info_id.as_str(), "Info 1");
    assert_ne!(new_rs_src_id.as_str(), "Result 1");

    let norm = &imported.result_sheets[1];
    assert_eq!(norm.eliminated_outliers[0].sheet_id, new_data_id);
    match &norm.analysis.kind {
        AnalysisKind::Transform(TransformAnalysisSpec::Normalize {
            zero_anchor: NormalizeAnchor::Hooked(HookSource::InfoConstant { info_sheet_id, .. }),
            hundred_anchor: NormalizeAnchor::Hooked(HookSource::ResultScalar { result_sheet_id, .. }),
            ..
        }) => {
            assert_eq!(info_sheet_id.as_ref(), Some(&new_info_id));
            assert_eq!(result_sheet_id, &new_rs_src_id);
        }
        other => panic!("unexpected normalize kind: {other:?}"),
    }

    match &imported.result_sheets[2].analysis.kind {
        AnalysisKind::NonlinearRegression(spec) => {
            let ParameterScope::HookedConstant(HookSource::InfoConstant { info_sheet_id, .. }) = &spec.parameter_rules[0].scope else {
                panic!("expected HookedConstant");
            };
            assert_eq!(info_sheet_id.as_ref(), Some(&new_info_id));
        }
        other => panic!("unexpected nl kind: {other:?}"),
    }

    let g = &imported.graph_sheets[0];
    assert!(g.spec.title.contains(&format!("{{{{info:{new_info_id}:Conc}}}}")));
    assert!(g.spec.title.contains(&format!("{{{{result:{new_rs_src_id}:0:EC50}}}}")));
    assert!(g.spec.x_axis.title.contains(&format!("{{{{info:{new_info_id}:Conc}}}}")));
    assert!(g.spec.y_axis.title.contains(&format!("{{{{result:{new_rs_src_id}:EC50}}}}")));
    assert!(g.floating_notes[0].body_markdown.contains(&format!("{{{{result:{new_rs_src_id}:0:EC50}}}}")));
    let LayoutItem::TextCallout { markdown, .. } = &imported.layout_sheets[0].spec.items[0] else {
        panic!("expected TextCallout");
    };
    assert!(markdown.contains(&format!("{{{{info:{new_info_id}:Conc}}}}")));
    assert!(markdown.contains(&format!("{{{{result:{new_rs_src_id}:EC50}}}}")));
}

// ============================================================================
// FA-SEC04-13: .prism embedded document.json uses clamp_peek_schema_version
// ============================================================================

#[test]
fn fa_sec04_13_prism_schema_version_clamps_u64_and_rejects_non_u64() {
    let base_payload = |extra: Option<(&str, serde_json::Value)>| {
        let mut obj = serde_json::json!({
            "title": "Schema Peek",
            "data_sheets": [],
            "info_sheets": [],
            "result_sheets": [],
            "graph_sheets": [],
            "layout_sheets": [],
            "active_sheet_id": null,
            "family_anchor_sheet_id": null
        });
        if let Some((k, v)) = extra {
            obj.as_object_mut().unwrap().insert(k.to_owned(), v);
        }
        let bytes = serde_json::to_vec(&obj).unwrap();
        build_zip_entries(&[("document.json", &bytes)])
    };

    assert!(matches!(
        import_prism_bytes(&base_payload(Some(("schema_version", serde_json::json!(5_000_000_000_u64))))),
        Err(FormatError::UnsupportedSchemaVersion { version: u32::MAX, max_supported: CURRENT_SCHEMA_VERSION })
    ));

    for bad in [serde_json::json!("7"), serde_json::json!(-1), serde_json::json!(1.5), serde_json::json!(true)] {
        assert!(
            matches!(import_prism_bytes(&base_payload(Some(("schema_version", bad.clone())))), Err(FormatError::Json(_))),
            "expected FormatError::Json for schema_version={bad}"
        );
    }

    // Omitted or null schema_version migrates from v0 cleanly
    assert_eq!(import_prism_bytes(&base_payload(None)).unwrap().document.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(
        import_prism_bytes(&base_payload(Some(("schema_version", serde_json::Value::Null)))).unwrap().document.schema_version,
        CURRENT_SCHEMA_VERSION
    );
}

// ============================================================================
// FA-SEC04-14: Recovery journal rewrite emits no duplicate blank newline
// ============================================================================

#[test]
fn fa_sec04_14_recovery_journal_rewrite_has_no_duplicate_trailing_newline() {
    let dir = TempDirGuard::new("journal_nl");
    {
        let backend = DirectoryJournalBackend::open_dir(&dir.path).unwrap();
        let (mut journal, _) = CrashRecoveryJournal::open_or_recover(backend, "w1", 60_000, 1_000).unwrap();
        journal.record_command(serde_json::json!({"op": "noop", "v": 1})).unwrap();
    }

    // Append a torn tail lacking terminating JSON validity so open_or_recover rewrites ops.journal
    let journal_path = dir.path.join(JOURNAL_FILE_NAME);
    {
        use std::io::Write as _;
        let mut f = OpenOptions::new().append(true).open(&journal_path).unwrap();
        f.write_all(b"{\"format_version\":1,\"seq\":2,\"torn").unwrap();
    }

    let backend = DirectoryJournalBackend::open_dir(&dir.path).unwrap();
    let (mut reopened, report) = CrashRecoveryJournal::open_or_recover(backend, "w2", 60_000, 65_000).unwrap();
    assert_eq!(report.discarded_corrupt_tail_lines, 1);

    let after_bytes = fs::read_to_string(&journal_path).unwrap();
    assert!(!after_bytes.contains("\n\n"), "rewritten journal must not contain duplicate newlines: {after_bytes:?}");
    assert!(after_bytes.ends_with('\n'));
    assert_eq!(after_bytes.lines().count(), 1);

    // Subsequent writes remain contiguous single-newline NDJSON without corrupt-line warnings
    reopened.record_command(serde_json::json!({"op": "noop", "v": 2})).unwrap();
    drop(reopened);

    let backend2 = DirectoryJournalBackend::open_dir(&dir.path).unwrap();
    let (_, report2) = CrashRecoveryJournal::open_or_recover(backend2, "w3", 60_000, 130_000).unwrap();
    assert_eq!(report2.discarded_corrupt_tail_lines, 0);
    assert!(report2.warnings.is_empty(), "unexpected warnings: {:?}", report2.warnings);
}
