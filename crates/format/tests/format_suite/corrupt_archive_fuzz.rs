#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::common::{
    RawZipEntry, bracket_spec, build_raw_zip, callout_item, col_sheet, live_graph_item, make_import_link_spec, param_rule, result_sheet_with_kind,
};
use curvebook_doc::{
    AnalysisKind, AxisSegment, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, CiBound, ColumnDesignHint,
    ColumnTable, ConstantValue, ContingencyTable, CurveSamplePoint, DataSheet, DocError, FittedCurveOverlay, FloatingNote, GraphAnnotation,
    GraphSheet, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind, JournalWidthPreset,
    LayoutSheet, LayoutSpec, LinkSource, LinkedDataRegion, MAX_DATA_SETS, MAX_SUBCOLUMNS, MAX_TITLE_BYTES, MultipleVariablesTable, NestedTable,
    NonlinearFitSpec, OneWayAnovaSpec, PairwiseComparisonRecord, ParameterScope, PartsOfWholeTable, PlacementMode, ProjectDocument, RectPt,
    ResultCell, ResultScalarEntry, ResultStatus, ResultTab, ResultTabKind, SequenceProgression, SheetCellRange, SheetId, SubcolumnMode,
    SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use curvebook_format::{
    ArchiveKind, CsvOptions, FormatError, ImportBaselineAdjustment, ImportWarningKind, MAX_XML_DEPTH, MAX_ZIP_ENTRIES, XmlLimits, ZipArchive,
    ZipLimits, ZipWriter, apply_csv_import_link, export_datasheet_to_csv, filter_delimited_text, import_csv_to_datasheet, import_prism_bytes,
    import_prism_bytes_with_limits, import_pzfx_bytes, import_pzfx_str, instantiate_template_bytes, open_archive_bytes,
    open_archive_bytes_with_limits, open_project_bytes, open_template_bytes, parse_cell_token, parse_delimited_records, parse_xml,
    parse_xml_with_limits, rearrange_and_baseline_grid, sanitize_zip_path, save_project_bytes, save_template_bytes, validate_project_document,
};
use proptest::collection;
use proptest::prelude::{Just, ProptestConfig, Strategy, TestCaseError, any, prop_assert, prop_assert_eq, prop_oneof, proptest};
use serde_json::{Value, json};

fn build_rich_eight_archetype_project() -> ProjectDocument {
    let mut doc = ProjectDocument::new("Comprehensive Eight-Archetype Study").unwrap();

    let mut info = InfoSheet::new_preseeded("info-main", "Assay Metadata").unwrap();
    info.set_constant("HotNM", ConstantValue::Number(2.5), Some(String::from("nM"))).unwrap();
    info.set_constant("RunDate", ConstantValue::Date(String::from("2026-11-13")), None).unwrap();
    info.set_constant("Buffer", ConstantValue::Text(String::from("HEPES pH 7.4")), None).unwrap();
    info.notes_markdown = String::from("Triplicate ligand binding & factorial plates.");
    doc.add_info_sheet(info).unwrap();

    let mut xy =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }, SubcolumnMode::RawReplicates(2), 2, 3).unwrap();
    xy.x_title = String::from("Log[Agonist]");
    xy.row_titles = vec![String::from("Low"), String::from("Mid"), String::from("High")];
    xy.row_titles_expanded = true;
    xy.data_sets[0].title = String::from("Wildtype");
    xy.data_sets[1].title = String::from("Mutant");
    xy.data_sets[0].cells[0] = vec![Cell::number(10.5), Cell::excluded_number(99.9)];
    xy.data_sets[0].cells[1] = vec![Cell::number(48.2), Cell::number(51.0)];
    xy.data_sets[1].cells[2] = vec![Cell::number(95.0), Cell::number(94.1)];
    for (r, x) in [-9.0, -8.0, -7.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(*x)).unwrap();
    }
    let mut xy_sheet = DataSheet::new("data-xy", "Dose Curve", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.info_sheet_id = Some(SheetId::new("info-main"));
    xy_sheet.floating_notes.push(FloatingNote::new("note-xy", RectPt::new(8.0, 12.0, 120.0, 40.0), "Check outlier"));
    doc.add_data_sheet(xy_sheet.clone()).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3).unwrap();
    col.row_titles = vec![String::from("Subj 1"), String::from("Subj 2"), String::from("Subj 3")];
    (col.data_sets[0].cells[0][0], col.data_sets[1].cells[0][0]) = (Cell::number(4.1), Cell::number(8.4));
    let mut col_sheet = DataSheet::new("data-col", "Paired Groups", TableArchetype::Column(col)).unwrap();
    col_sheet.linked_regions.push(LinkedDataRegion {
        id: String::from("lnk-ext"),
        spec: make_import_link_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), false),
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0)),
        provenance_summary: String::from("Imported from assay.csv"),
    });
    doc.add_data_sheet(col_sheet).unwrap();

    let mut grp = GroupedTable::new(SubcolumnMode::MeanSemN, 2, 2).unwrap();
    grp.row_titles = vec![String::from("24h"), String::from("48h")];
    grp.data_sets[0].cells[0] = vec![Cell::number(20.0), Cell::number(1.5), Cell::number(6.0)];
    doc.add_data_sheet(DataSheet::new("data-grp", "Factorial", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut cont = ContingencyTable::new(2, 2).unwrap();
    cont.row_titles = vec![String::from("Exposed"), String::from("Unexposed")];
    (cont.data_sets[0].cells[0][0], cont.data_sets[1].cells[1][0]) = (Cell::number(14.0), Cell::number(17.0));
    doc.add_data_sheet(DataSheet::new("data-cont", "Responder Counts", TableArchetype::Contingency(cont)).unwrap()).unwrap();

    let mut surv = SurvivalTable::new(2, 2).unwrap();
    surv.times = vec![Cell::number(5.0), Cell::number(12.0)];
    surv.cohorts[0].cells = vec![vec![Cell::number(1.0)], vec![Cell::number(0.0)]];
    doc.add_data_sheet(DataSheet::new("data-surv", "Survival Cohort", TableArchetype::Survival(surv)).unwrap()).unwrap();

    let mut pow = PartsOfWholeTable::new(1, 2).unwrap();
    pow.slice_labels = vec![String::from("CD4"), String::from("CD8")];
    pow.data_sets[0].cells = vec![vec![Cell::number(40.0)], vec![Cell::number(60.0)]];
    doc.add_data_sheet(DataSheet::new("data-pow", "Cell Fractions", TableArchetype::PartsOfWhole(pow)).unwrap()).unwrap();

    let mut mv = MultipleVariablesTable::new(2, 2).unwrap();
    mv.variables[0].cells = vec![Cell::text("Ctrl"), Cell::text("Drug")];
    mv.variables[0].refresh_inferred_role();
    mv.variables[1].cells = vec![Cell::number(12.3), Cell::number(28.7)];
    doc.add_data_sheet(DataSheet::new("data-mv", "Cohort Frame", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    let mut nest = NestedTable::new(2, 2, 2).unwrap();
    nest.groups[0].cells[0] = vec![Cell::number(1.1), Cell::number(1.2)];
    let mut nest_sheet = DataSheet::new("data-nest", "Nested Replicates", TableArchetype::Nested(nest)).unwrap();
    nest_sheet.linked_regions.push(LinkedDataRegion {
        id: String::from("lnk-int"),
        spec: ImportLinkSpec {
            source: LinkSource::InternalSheet {
                sheet_id: SheetId::new("data-col"),
                kind: InternalLinkKind::DataSheet,
                tab_id: None,
                range: SheetCellRange::default(),
            },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::default(),
        provenance_summary: String::from("Linked from data-col"),
    });
    doc.add_data_sheet(nest_sheet).unwrap();

    let mut fit = NonlinearFitSpec::new("four_pl");
    let mut rule = param_rule(
        "HotNM",
        ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-main")),
            constant_name: String::from("HotNM"),
        }),
    );
    rule.lower_bound = Some(0.0);
    fit.parameter_rules.push(rule);
    let mut res = result_sheet_with_kind("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit));
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("EC50"), data_set_index: Some(0), value: 1.25e-8, units: Some(String::from("M")) });
    res.tabs.push(ResultTab {
        id: String::from("params"),
        title: String::from("Best-fit values"),
        kind: ResultTabKind::ParametersAndConfidenceIntervals,
        column_headers: vec![String::from("Value"), String::from("95% CI"), String::from("P value")],
        row_labels: vec![String::from("EC50")],
        rows: vec![vec![
            ResultCell::Number { value: 1.25e-8, prefix: None, units: Some(String::from("M")) },
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(8.0e-9), upper: CiBound::Open },
            ResultCell::PValue { raw: 0.0002, formatted: String::from("0.0002"), stars: String::from("***") },
        ]],
        footnotes: vec![String::from("Asymmetrical profile-likelihood CI")],
    });
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Wildtype"),
        label_b: String::from("Mutant"),
        estimate_diff: Some(4.5),
        raw_p_or_q: 0.0021,
        is_fdr_q: false,
        significant: true,
    });
    res.fitted_curves.push(FittedCurveOverlay {
        data_set_index: 0,
        equation_id: String::from("four_pl"),
        converged: true,
        ambiguous: false,
        points: vec![CurveSamplePoint { x: -8.0, y: 50.0, ci_lower: Some(44.0), ci_upper: Some(56.0), pred_lower: None, pred_upper: None }],
    });
    res.checklist.push(ChecklistItem {
        assumption: String::from("Normality"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("W = 0.99")),
        explanation: String::from("Residuals consistent with Gaussian model"),
        remediation: None,
    });
    doc.add_result_sheet(res).unwrap();
    doc.add_result_sheet(result_sheet_with_kind("anova-1", "data-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))).unwrap();

    let mut gs = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
    gs.spec.x_axis.segments = vec![AxisSegment::default(), AxisSegment::default()];
    gs.spec.comparisons.push(bracket_spec("anova-1"));
    gs.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 90.0, 24.0),
        markdown: String::from("EC50 = {{result:fit-1:EC50}}"),
        bordered: true,
    });
    doc.add_graph_sheet(gs).unwrap();

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    lspec.items.extend([live_graph_item("panel-a", "graph-1"), callout_item("callout-1", "Ligand = {{info:HotNM}}")]);
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).unwrap()).unwrap();
    doc
}

fn check_no_panic<F: FnOnce()>(label: &str, f: F) {
    assert!(catch_unwind(AssertUnwindSafe(f)).is_ok(), "unexpected panic in {label}");
}

#[test]
fn full_eight_archetype_project_and_template_round_trip_determinism() {
    let doc = build_rich_eight_archetype_project();
    let proj_a = save_project_bytes(&doc).unwrap();
    assert_eq!(proj_a, save_project_bytes(&doc).unwrap(), ".curvebook must be deterministic");
    assert_eq!(open_archive_bytes(&proj_a).unwrap(), (ArchiveKind::Project, doc.clone()));
    assert_eq!(open_project_bytes(&proj_a).unwrap(), doc);
    assert!(matches!(open_template_bytes(&proj_a), Err(FormatError::InvalidArchiveKind { expected: "template", found: "project" })));

    let tmpl_a = save_template_bytes(&doc).unwrap();
    assert_eq!(tmpl_a, save_template_bytes(&doc).unwrap(), ".cbtemplate must be deterministic");
    assert!(matches!(open_project_bytes(&tmpl_a), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));

    let tmpl = open_template_bytes(&tmpl_a).unwrap();
    assert_eq!((&tmpl.info_sheets, &tmpl.graph_sheets, &tmpl.layout_sheets), (&doc.info_sheets, &doc.graph_sheets, &doc.layout_sheets));
    assert!(tmpl.result_sheets.iter().all(|r| r.status == ResultStatus::Dirty
        && r.scalars.is_empty()
        && r.tabs.is_empty()
        && r.pairwise_comparisons.is_empty()
        && r.fitted_curves.is_empty()
        && r.checklist.is_empty()));
    for ds in &tmpl.data_sheets {
        for r in 0..ds.archetype.row_count() {
            assert!(ds.x_or_time_cell(r, 0).is_none_or(|x| x.is_empty()));
            for c in 0..ds.archetype.column_count() {
                assert!(ds.cell(CellCoord::new(r, c, 0)).unwrap().is_empty());
            }
        }
    }

    let inst_from_tmpl = instantiate_template_bytes(&tmpl_a, Some("Batch Plate #7")).unwrap();
    assert_eq!((inst_from_tmpl.title.as_str(), inst_from_tmpl.data_sheets.len()), ("Batch Plate #7", 8));
    let inst_from_proj = instantiate_template_bytes(&proj_a, Some("Cloned From Project")).unwrap();
    assert_eq!((inst_from_proj.title.as_str(), &inst_from_proj.result_sheets[0].status), ("Cloned From Project", &ResultStatus::Dirty));
}

#[test]
fn exhaustive_truncation_of_project_and_template_archives_returns_err() {
    let doc = build_rich_eight_archetype_project();
    let (proj_bytes, tmpl_bytes) = (save_project_bytes(&doc).unwrap(), save_template_bytes(&doc).unwrap());

    for len in 0..proj_bytes.len() {
        let prefix = &proj_bytes[..len];
        check_no_panic("exhaustive project truncation", || {
            assert!(
                open_project_bytes(prefix).is_err() && open_archive_bytes(prefix).is_err() && instantiate_template_bytes(prefix, None).is_err(),
                "len {len}"
            );
        });
    }
    for len in 0..tmpl_bytes.len() {
        let prefix = &tmpl_bytes[..len];
        check_no_panic("exhaustive template truncation", || {
            assert!(open_template_bytes(prefix).is_err() && open_archive_bytes(prefix).is_err(), "len {len}");
        });
    }
}

#[test]
fn targeted_zip_header_and_manifest_corruptions_return_clean_err() {
    let valid = build_raw_zip(&[RawZipEntry::stored(b"manifest.json", b"{}")], b"");
    let eocd_pos = valid.len() - 22;
    let cd_off = u32::from_le_bytes(valid[eocd_pos + 16..eocd_pos + 20].try_into().unwrap()) as usize;

    type ZipPatchFn = fn(&mut [u8], usize, usize);
    let patches: &[(&str, ZipPatchFn)] = &[
        ("bad_disk", |b, e, _| b[e + 4] = 1),
        ("bad_cd_disk", |b, e, _| b[e + 6] = 2),
        ("bad_disk_entries", |b, e, _| b[e + 8] = 9),
        ("bad_cd_end", |b, e, _| b[e + 12..e + 16].copy_from_slice(&9999_u32.to_le_bytes())),
        ("bad_comment_len", |b, e, _| b[e + 20..e + 22].copy_from_slice(&5_u16.to_le_bytes())),
        ("bad_local_off", |b, _, cd| b[cd + 42..cd + 46].copy_from_slice(&(u32::MAX - 5).to_le_bytes())),
    ];
    for &(label, patch) in patches {
        let mut buf = valid.clone();
        patch(&mut buf, eocd_pos, cd_off);
        assert!(matches!(ZipArchive::from_bytes(&buf), Err(FormatError::InvalidZip { .. })), "{label}");
    }

    assert!(matches!(
        ZipArchive::from_bytes(&build_raw_zip(&[RawZipEntry { declared_uncompressed: Some(10), ..RawZipEntry::stored(b"a.txt", b"hello") }], b"")),
        Err(FormatError::InvalidZip { .. })
    ));
    assert!(matches!(
        ZipArchive::from_bytes(&build_raw_zip(&[RawZipEntry { flags: 0x0001, ..RawZipEntry::stored(b"secret.txt", b"data") }], b"")),
        Err(FormatError::UnsupportedFormat { .. })
    ));
    assert!(matches!(
        ZipArchive::from_bytes(&build_raw_zip(&[RawZipEntry { method: 12, ..RawZipEntry::stored(b"data.bin", b"BZh9") }], b"")),
        Err(FormatError::UnsupportedCompression { method: 12, .. })
    ));

    let doc_bytes = serde_json::to_vec(&build_rich_eight_archetype_project()).unwrap();
    let bad_magic = json!({ "format": "other-format", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project", "title": "X", "document_crc32": crc32fast::hash(&doc_bytes), "sheets": [] });
    let bad_magic_zip = build_raw_zip(
        &[RawZipEntry::deflated(b"document.json", &doc_bytes), RawZipEntry::deflated(b"manifest.json", &serde_json::to_vec(&bad_magic).unwrap())],
        b"",
    );
    assert!(matches!(open_project_bytes(&bad_magic_zip), Err(FormatError::UnsupportedFormat { .. })));

    let missing_sheet = json!({
        "format": "curvebook-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project", "title": "X",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{ "id": "data-xy", "section": "Data", "title": "Dose Curve", "csv_path": "sheets/missing.csv", "crc32": 12345 }]
    });
    let missing_zip = build_raw_zip(
        &[RawZipEntry::deflated(b"document.json", &doc_bytes), RawZipEntry::deflated(b"manifest.json", &serde_json::to_vec(&missing_sheet).unwrap())],
        b"",
    );
    assert!(matches!(open_project_bytes(&missing_zip), Err(FormatError::MissingArchiveEntry { .. })));
}

#[test]
fn zip_slip_path_traversal_duplicates_and_local_header_spoofing_blocked() {
    let hostile_paths = [
        "../manifest.json",
        "../../etc/passwd",
        "sheets/../../escape.csv",
        "sheets/..",
        "..",
        "../",
        "/manifest.json",
        "/etc/passwd",
        "/",
        "\\Windows\\System32\\cmd.exe",
        "sheets\\..\\evil.csv",
        "C:manifest.json",
        "C:/manifest.json",
        "./manifest.json",
        "sheets/./a.csv",
        "sheets//a.csv",
        "",
        "manifest.json\0.txt",
    ];
    let mut writer = ZipWriter::new();
    for path in hostile_paths {
        assert!(matches!(sanitize_zip_path(path), Err(FormatError::UnsafePath { .. })), "sanitize `{path:?}`");
        assert!(matches!(writer.add_file(path, b"x".to_vec()), Err(FormatError::UnsafePath { .. })), "ZipWriter `{path:?}`");
        let raw_zip = build_raw_zip(&[RawZipEntry::stored(path.as_bytes(), b"payload")], b"");
        assert!(matches!(ZipArchive::from_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "ZipArchive `{path:?}`");
        assert!(matches!(open_archive_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "open_archive `{path:?}`");
        // `.prism` uses `ZipReadOptions::external_interop()`, which normalizes benign `./`
        // segments while keeping strict rejection of traversal, absolute paths, drive prefixes,
        // empty segments, and NUL bytes.
        if curvebook_format::sanitize_zip_path_with_options(path, curvebook_format::ZipReadOptions::external_interop()).is_err() {
            assert!(matches!(import_prism_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "import_prism_bytes `{path:?}`");
        }
    }

    let dup_zip = build_raw_zip(&[RawZipEntry::stored(b"sheets/a.csv", b"1"), RawZipEntry::stored(b"sheets/a.csv", b"2")], b"");
    assert!(matches!(ZipArchive::from_bytes(&dup_zip), Err(FormatError::DuplicateEntry { .. })));

    let spoofed = RawZipEntry { local_name: Some(b"../../../../etc/passwd"), ..RawZipEntry::stored(b"data/ok.csv", b"X,Y\n1,2\n") };
    let archive = ZipArchive::from_bytes(&build_raw_zip(&[spoofed], b"")).unwrap();
    assert_eq!(archive.iter().map(|(k, _)| k).collect::<Vec<_>>(), vec!["data/ok.csv"]);
}

#[test]
fn zip_bomb_declared_sizes_lying_deflate_and_cumulative_budgets_rejected() {
    let huge_zip = build_raw_zip(&[RawZipEntry { declared_uncompressed: Some(u32::MAX), ..RawZipEntry::deflated(b"huge.csv", b"tiny") }], b"");
    assert!(matches!(ZipArchive::from_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(open_archive_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(import_prism_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));

    let relaxed = ZipLimits { max_entries: 16, max_total_uncompressed_bytes: u64::MAX, max_entry_uncompressed_bytes: u64::MAX };
    assert!(
        ZipArchive::from_bytes_with_limits(
            &build_raw_zip(&[RawZipEntry { declared_uncompressed: Some(u32::MAX), ..RawZipEntry::deflated(b"huge2.csv", b"tiny") }], b""),
            relaxed
        )
        .is_err()
    );

    let cum: Vec<_> = [(b"a.bin".as_slice(), b"x".as_slice()), (b"b.bin", b"y"), (b"c.bin", b"z")]
        .into_iter()
        .map(|(n, p)| RawZipEntry { declared_uncompressed: Some(50 * 1024 * 1024), ..RawZipEntry::stored(n, p) })
        .collect();
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&cum, b"")), Err(FormatError::ZipBomb { .. })));

    let zeros = vec![0_u8; 256 * 1024];
    let lying_zip = build_raw_zip(&[RawZipEntry { declared_uncompressed: Some(32), ..RawZipEntry::deflated(b"data/bomb.csv", &zeros) }], b"");
    let tight = ZipLimits { max_entries: 32, max_total_uncompressed_bytes: 8192, max_entry_uncompressed_bytes: 8192 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(open_archive_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(import_prism_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));

    let (part_a, part_b) = (vec![b'A'; 3000], vec![b'B'; 3000]);
    let multi_zip = build_raw_zip(&[RawZipEntry::deflated(b"a.txt", &part_a), RawZipEntry::deflated(b"b.txt", &part_b)], b"");
    let split = ZipLimits { max_entries: 32, max_total_uncompressed_bytes: 4096, max_entry_uncompressed_bytes: 4096 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&multi_zip, split), Err(FormatError::ZipBomb { .. })));

    let names: Vec<String> = (0..=MAX_ZIP_ENTRIES).map(|i| format!("e/{i}.txt")).collect();
    let entries: Vec<_> = names.iter().map(|n| RawZipEntry::stored(n.as_bytes(), b"")).collect();
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&entries, b"")), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn corrupt_crc32_and_invalid_utf8_entries_handled_cleanly() {
    for (label, entry) in [
        ("stored_crc", RawZipEntry { crc: Some(crc32fast::hash(b"{}") ^ 0x1234_5678), ..RawZipEntry::stored(b"manifest.json", b"{}") }),
        ("deflated_crc", RawZipEntry { crc: Some(0xDEAD_BEEF), ..RawZipEntry::deflated(b"manifest.json", b"{ \"hello\": \"world\" }") }),
    ] {
        assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&[entry], b"")), Err(FormatError::CorruptCrc { .. })), "{label}");
    }

    let doc = build_rich_eight_archetype_project();
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let csv_bytes = export_datasheet_to_csv(&doc.data_sheets[0], &CsvOptions::csv()).unwrap().into_bytes();
    let bad_sheet_manifest = json!({
        "format": "curvebook-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project", "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{ "id": doc.data_sheets[0].id, "section": "Data", "title": doc.data_sheets[0].title, "csv_path": "sheets/data-xy.csv", "crc32": crc32fast::hash(&csv_bytes) ^ 1 }]
    });
    let bad_sheet_zip = build_raw_zip(
        &[
            RawZipEntry::deflated(b"document.json", &doc_bytes),
            RawZipEntry::deflated(b"sheets/data-xy.csv", &csv_bytes),
            RawZipEntry::deflated(b"manifest.json", &serde_json::to_vec(&bad_sheet_manifest).unwrap()),
        ],
        b"",
    );
    assert!(matches!(open_project_bytes(&bad_sheet_zip), Err(FormatError::CorruptCrc { .. })));

    assert!(matches!(
        ZipArchive::from_bytes(&build_raw_zip(&[RawZipEntry::stored(&[0xFF, 0xFE, b'x'], b"payload")], b"")),
        Err(FormatError::InvalidZip { .. })
    ));
    assert!(matches!(
        open_archive_bytes(&build_raw_zip(&[RawZipEntry::stored(b"manifest.json", &[0xFF, 0x80, 0xBF])], b"")),
        Err(FormatError::InvalidZip { .. })
    ));

    let manifest_ok = json!({ "format": "curvebook-archive", "schema_version": CURRENT_SCHEMA_VERSION, "kind": "Project", "title": "Bad UTF8 Doc", "document_crc32": crc32fast::hash(&[0xFF, 0x80, 0xFE]), "sheets": [] });
    let bad_doc_utf8 = build_raw_zip(
        &[
            RawZipEntry::stored(b"manifest.json", &serde_json::to_vec(&manifest_ok).unwrap()),
            RawZipEntry::stored(b"document.json", &[0xFF, 0x80, 0xFE]),
        ],
        b"",
    );
    assert!(matches!(open_project_bytes(&bad_doc_utf8), Err(FormatError::Json(_))));

    let prism_bad_csv = build_raw_zip(
        &[RawZipEntry::stored(b"sheets/corrupt.csv", &[0xFF, 0xFE, b',', b'1']), RawZipEntry::stored(b"sheets/good.csv", b"A\n1\n")],
        b"",
    );
    let prism_report = import_prism_bytes(&prism_bad_csv).unwrap();
    assert!(
        prism_report.document.data_sheets.len() == 1
            && prism_report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source == "sheets/corrupt.csv")
    );
    assert!(matches!(import_pzfx_bytes(&[0xFF, 0xFE, b'<', b'P']), Err(FormatError::XmlParse { .. })));
}

#[test]
fn adversarial_pzfx_xml_unclosed_deep_nesting_missing_tables_and_non_finite_tokens() {
    let bad_xml_cases = [
        "",
        "   \r\n\t ",
        "\u{FEFF}",
        "plain text outside root",
        "hello<PrismFile/>",
        "<PrismFile><Table>",
        "<PrismFile><Title>abc</PrismFile>",
        "<PrismFile attr=\"unclosed></PrismFile>",
        "<PrismFile><!-- unclosed comment</PrismFile>",
        "<?xml version=\"1.0\"<PrismFile/>",
        "<PrismFile><![CDATA[unclosed</PrismFile>",
        "<PrismFile><Title>a &amp b</Title></PrismFile>",
        "<PrismFile><Title>&#xZZZZ;</Title></PrismFile>",
        "<PrismFile><Title>&#99999999999;</Title></PrismFile>",
        "<PrismFile><Title>&#xD800;</Title></PrismFile>",
        "<PrismFile><Title>&custom;</Title></PrismFile>",
        "<!DOCTYPE x [<!ENTITY xx SYSTEM \"file:///etc/passwd\">]><PrismFile/>",
        "<!ENTITY x \"y\"><PrismFile/>",
        "<WrongRoot/>",
    ];
    for case in bad_xml_cases {
        check_no_panic("bad xml case", || assert!(import_pzfx_str(case).is_err(), "expected Err for `{case}`"));
    }
    assert!(parse_xml("<PrismFile/><PrismFile/>").is_err(), "strict default XML parser must reject trailing root elements");
    let trailing_rep = import_pzfx_str("<PrismFile/><PrismFile/>").unwrap();
    assert!(
        trailing_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection),
        "stop-after-root .pzfx import must report trailing bytes via IgnoredSection"
    );

    for depth in [MAX_XML_DEPTH + 1, 512, 2_500] {
        let deep = format!("<PrismFile>{}{}</PrismFile>", "<A>".repeat(depth), "</A>".repeat(depth));
        assert!(matches!(parse_xml(&deep), Err(FormatError::XmlDepthExceeded { .. })));
        assert!(matches!(import_pzfx_str(&deep), Err(FormatError::XmlDepthExceeded { .. })));
    }

    let many_nodes = "<PrismFile><A/><A/><A/><A/><A/></PrismFile>";
    for limits in [XmlLimits { max_bytes: 1024, max_depth: 32, max_nodes: 3 }, XmlLimits { max_bytes: 8, max_depth: 32, max_nodes: 100 }] {
        assert!(matches!(parse_xml_with_limits(many_nodes, limits), Err(FormatError::XmlParse { .. })));
    }

    let empty_rep = import_pzfx_str("<PrismFile><Title>Empty Study</Title><TableSequence/></PrismFile>").unwrap();
    assert!(empty_rep.document.title == "Empty Study" && empty_rep.document.data_sheets.is_empty());

    let long_utf8 = "é".repeat(MAX_TITLE_BYTES);
    let weird_pzfx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><PrismFile><Title>{long_utf8}</Title>
  <InfoSequence><Info ID=""><Title>Info</Title><Constant><Name>NanVal</Name><Value>NaN</Value></Constant><Constant><Name>InfVal</Name><Value>Inf</Value></Constant><Constant><Name>NegInfVal</Name><Value>-Infinity</Value></Constant><Constant><Name>Huge</Name><Value>1e500</Value></Constant><Constant><Name></Name><Value>123</Value></Constant></Info></InfoSequence>
  <TableSequence>
    <Table ID="   " TableType="XY" XFormat="start-step" X0="NaN" DeltaX="Inf" Replicates="0"><Title>{long_utf8}</Title><XColumn><Subcolumn><d>NaN</d><d>Inf</d><d>-Inf</d><d>1e500</d></Subcolumn></XColumn><YColumn><Subcolumn><d>NaN</d><d>*NaN</d><d>Inf*</d><d>*</d></Subcolumn></YColumn></Table>
    <Table ID="dup" TableType="Column" Replicates="999999999999999999999999"><YColumn/></Table>
    <Table ID="dup" TableType="PartsOfWhole"/>
  </TableSequence>
  <GraphSequence><Graph ID="g1"/></GraphSequence><LayoutSequence><Layout ID="l1"/></LayoutSequence><ScriptSequence><Script ID="s1"/></ScriptSequence>
</PrismFile>"#
    );
    let rep = import_pzfx_str(&weird_pzfx).unwrap();
    assert!(rep.document.title.len() <= MAX_TITLE_BYTES && rep.document.data_sheets.len() == 3);
    assert!(
        rep.warnings.iter().any(|w| w.kind == ImportWarningKind::TruncatedTitle)
            && rep.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection)
    );
    validate_project_document(&rep.document).unwrap();
    assert_eq!(open_project_bytes(&save_project_bytes(&rep.document).unwrap()).unwrap(), rep.document);

    assert!(matches!(
        import_pzfx_str(&format!("<PrismFile><Table TableType=\"XY\" Replicates=\"{}\"/></PrismFile>", MAX_SUBCOLUMNS + 1)),
        Err(FormatError::Doc(DocError::TooManySubcolumns { .. }))
    ));
}

#[test]
fn adversarial_prism_json_structural_cases_handled_cleanly() {
    let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
    let deep_rep = import_prism_bytes(&build_raw_zip(
        &[RawZipEntry::stored(b"manifest.json", deep.as_bytes()), RawZipEntry::stored(b"sheets/a.csv", b"A\n1\n")],
        b"",
    ))
    .unwrap();
    assert!(deep_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata));

    let deep_doc = import_prism_bytes(&build_raw_zip(
        &[
            RawZipEntry::stored(b"document.json", deep.as_bytes()),
            RawZipEntry::stored(b"manifest.json", deep.as_bytes()),
            RawZipEntry::stored(b"info/info.json", deep.as_bytes()),
        ],
        b"",
    ))
    .unwrap();
    assert!(deep_doc.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk));

    let nan_doc = br#"{"schema_version":1,"title":"NaN","data_sheets":[{"id":"d1","title":"D","archetype":{"Column":{"design_hint":"UnpairedReplicates","subcolumn_mode":"Single","row_titles":[],"row_titles_expanded":false,"data_sets":[{"title":"A","cells":[[{"value":{"Number":NaN},"excluded":false}]]}]}},"frozen":false,"info_sheet_id":null,"linked_regions":[],"floating_notes":[],"color_tag":null}],"info_sheets":[],"result_sheets":[],"graph_sheets":[],"layout_sheets":[],"active_sheet_id":null,"family_anchor_sheet_id":null}"#;
    assert!(
        import_prism_bytes(&build_raw_zip(&[RawZipEntry::stored(b"document.json", nan_doc)], b""))
            .unwrap()
            .warnings
            .iter()
            .any(|w| w.kind == ImportWarningKind::UnsupportedChunk)
    );

    let ver_zip = |v: Value| {
        let bytes = serde_json::to_vec(&json!({ "schema_version": v, "title": "V", "data_sheets": [], "info_sheets": [], "result_sheets": [], "graph_sheets": [], "layout_sheets": [], "active_sheet_id": null, "family_anchor_sheet_id": null })).unwrap();
        build_raw_zip(&[RawZipEntry::stored(b"document.json", &bytes)], b"")
    };
    assert!(matches!(import_prism_bytes(&ver_zip(json!(CURRENT_SCHEMA_VERSION + 42))), Err(FormatError::UnsupportedSchemaVersion { .. })));
    assert!(matches!(
        import_prism_bytes(&ver_zip(json!(9_999_999_999_999_u64))),
        Err(FormatError::UnsupportedSchemaVersion { version: u32::MAX, max_supported: CURRENT_SCHEMA_VERSION })
    ));
    assert!(matches!(import_prism_bytes(&ver_zip(json!("not-an-int"))), Err(FormatError::Json(_))));

    let mut cycle_doc = ProjectDocument::new("Cycle").unwrap();
    cycle_doc.add_result_sheet(result_sheet_with_kind("r1", "r1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))).unwrap();
    assert!(matches!(
        import_prism_bytes(&build_raw_zip(&[RawZipEntry::stored(b"document.json", &serde_json::to_vec(&cycle_doc).unwrap())], b"")),
        Err(FormatError::DependencyCycle { .. })
    ));

    let bad_meta = serde_json::to_vec(
        &json!({ "title": "Bad Reps", "kind": "XY", "subcolumn_mode": "replicates", "replicates": MAX_SUBCOLUMNS + 10, "csv_path": "sheets/xy.csv" }),
    )
    .unwrap();
    assert!(matches!(
        import_prism_bytes(&build_raw_zip(
            &[RawZipEntry::stored(b"sheets/xy.json", &bad_meta), RawZipEntry::stored(b"sheets/xy.csv", b"X,Y\n1,2\n")],
            b""
        )),
        Err(FormatError::Doc(DocError::TooManySubcolumns { .. }))
    ));
}

#[test]
fn adversarial_rfc4180_csv_tsv_edge_cases() {
    let opts = CsvOptions::csv();
    for bad in ["\"unclosed", "a,\"unclosed", "\"escaped\"\"quote", "a,b\n\"c,d", "a\"b,c", "\"ab\"c,d"] {
        assert!(matches!(parse_delimited_records(bad, &opts), Err(FormatError::CsvParse { .. })), "{bad:?}");
    }

    let mixed = "\u{FEFF}\"H,1\"\t\"H\r\n2\"\r\n1\t2\r3\t\"line1\nline2\rline3\"\n\u{FEFF}4\t5\r";
    assert_eq!(
        parse_delimited_records(mixed, &CsvOptions::tsv()).unwrap(),
        vec![vec!["H,1", "H\r\n2"], vec!["1", "2"], vec!["3", "line1\nline2\rline3"], vec!["\u{FEFF}4", "5"]]
    );
    assert!(parse_delimited_records("\u{FEFF}", &opts).unwrap().is_empty());

    let jagged = "A,B,C\n1\n2,3,4,5,6\n\n7,8\n";
    for (kind, mode) in [
        (TableKind::Xy, SubcolumnMode::Single),
        (TableKind::Column, SubcolumnMode::Single),
        (TableKind::Grouped, SubcolumnMode::RawReplicates(2)),
        (TableKind::Contingency, SubcolumnMode::Single),
        (TableKind::Survival, SubcolumnMode::Single),
        (TableKind::PartsOfWhole, SubcolumnMode::Single),
        (TableKind::MultipleVariables, SubcolumnMode::Single),
        (TableKind::Nested, SubcolumnMode::RawReplicates(1)),
    ] {
        assert!(
            !export_datasheet_to_csv(&import_csv_to_datasheet("jagged", "Jagged", jagged, kind, mode, &opts).unwrap(), &opts).unwrap().is_empty()
        );
    }

    let euro_text = "\u{FEFF}Group A;Group B\r\n1.234,5;Inf\r\n-0,25*;NaN\r\n*;-Infinity\r\n";
    let det = CsvOptions::detect(euro_text);
    assert_eq!((det.delimiter, det.decimal_separator), (';', ','));
    let euro = CsvOptions { thousands_separator: Some('.'), ..CsvOptions::european_csv() };
    for (tok, exp) in [
        ("1.234,5", Cell::number(1234.5)),
        ("-0,25*", Cell::excluded_number(-0.25)),
        ("NaN", Cell::empty()),
        ("Inf", Cell::text("Inf")),
        ("-Infinity", Cell::text("-Infinity")),
        ("1e500", Cell::text("1e500")),
        ("*NaN", Cell { value: CellValue::Empty, excluded: true }),
    ] {
        assert_eq!(parse_cell_token(tok, &euro), exp, "tok={tok:?}");
    }

    assert_eq!(parse_delimited_records("H\0A,H\0B\n\"v\x001\",2\0x\n", &opts).unwrap(), vec![vec!["H\0A", "H\0B"], vec!["v\x001", "2\0x"]]);
    assert!(matches!(parse_delimited_records(&",".repeat(MAX_DATA_SETS * MAX_SUBCOLUMNS + 4), &opts), Err(FormatError::CsvParse { .. })));
}

#[test]
fn adversarial_csv_import_link_pipeline_and_frozen_sheets() {
    for (label, text, filter) in [
        ("row_win", "A\n1\n2\n", ImportFilterSpec { row_window: Some((5, 2)), ..Default::default() }),
        ("col_win", "A,B\n1,2\n", ImportFilterSpec { col_window: Some((3, 1)), ..Default::default() }),
        ("zero_dec", "A\n1\n", ImportFilterSpec { decimate_every_k: Some(0), ..Default::default() }),
        ("oob_unstack", "A,B\n1,2\n", ImportFilterSpec { unstack_group_column: Some(99), ..Default::default() }),
    ] {
        assert!(matches!(filter_delimited_text(text, ',', '.', true, &filter), Err(FormatError::InvalidPipeline { .. })), "{label}");
    }

    let grid = vec![vec![Cell::number(0.0), Cell::number(10.0)], vec![Cell::number(5.0), Cell::number(20.0)]];
    for row in rearrange_and_baseline_grid(&grid, &ImportPlacementSpec::default(), ImportBaselineAdjustment::PercentOfFirstRow) {
        assert!(row.iter().all(|c| match c.value {
            CellValue::Number(v) => v.is_finite(),
            _ => true,
        }));
    }

    let mut frozen = col_sheet("frozen-1", 1, 1);
    frozen.frozen = true;
    let spec = make_import_link_spec(PlacementMode::Overwrite, CellCoord::new(0, 0, 0), false);
    assert!(matches!(
        apply_csv_import_link(&mut frozen, "A\n1\n", &spec, true, ImportBaselineAdjustment::None, "l1", "summary"),
        Err(FormatError::Doc(DocError::SheetFrozen { .. }))
    ));
}

#[test]
fn hardened_import_cell_budget_and_anchor_overflow_regressions() {
    let opts = CsvOptions::csv();
    for (label, csv, kind, mode) in [
        ("jagged_col", format!("{}\n{}", ",".repeat(1_000), "\n".repeat(10_005)), TableKind::Column, SubcolumnMode::Single),
        ("xy_rep16", format!("{}\n{}", ",".repeat(1_600), "\n".repeat(7_000)), TableKind::Xy, SubcolumnMode::RawReplicates(16)),
    ] {
        assert!(matches!(import_csv_to_datasheet(label, label, &csv, kind, mode, &opts), Err(FormatError::CsvParse { .. })), "{label}");
    }

    let mut unstack_oom = String::from("Group,Val\n");
    for g in 0..1_001 {
        unstack_oom.push_str(&format!("G{g},1\n"));
    }
    unstack_oom.push_str(&"G0,2\n".repeat(10_005));
    assert!(matches!(
        filter_delimited_text(&unstack_oom, ',', '.', false, &ImportFilterSpec { unstack_group_column: Some(0), ..Default::default() }),
        Err(FormatError::InvalidPipeline { .. })
    ));

    for bad_anchor in [CellCoord::new(usize::MAX, 0, 0), CellCoord::new(0, usize::MAX, 0), CellCoord::new(0, 0, usize::MAX)] {
        let mut target = DataSheet::new("tgt", "Target", TableArchetype::Xy(XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap())).unwrap();
        let spec = make_import_link_spec(PlacementMode::Overwrite, bad_anchor, false);
        assert!(matches!(
            apply_csv_import_link(&mut target, "1\n2\n", &spec, false, ImportBaselineAdjustment::None, "l1", "summary"),
            Err(FormatError::InvalidPipeline { .. })
        ));
    }

    let mut wide_target = col_sheet("wide", 25, 1);
    let growth_spec = make_import_link_spec(PlacementMode::Overwrite, CellCoord::new(500_000, 0, 0), false);
    assert!(matches!(
        apply_csv_import_link(&mut wide_target, "1\n", &growth_spec, false, ImportBaselineAdjustment::None, "l1", "summary"),
        Err(FormatError::InvalidPipeline { .. })
    ));

    let row_ds = "<d/>".repeat(1_000);
    let table_6m = format!(
        "<Table TableType=\"XY\" Replicates=\"500\"><RowTitlesColumn><Subcolumn>{row_ds}</Subcolumn></RowTitlesColumn>{}</Table>",
        "<YColumn><Subcolumn/></YColumn>".repeat(12)
    );
    for (label, xml) in [
        (
            "single_xy",
            format!(
                "<PrismFile><Table TableType=\"XY\" Replicates=\"512\"><RowTitlesColumn><Subcolumn>{row_ds}</Subcolumn></RowTitlesColumn>{}</Table></PrismFile>",
                "<YColumn><Subcolumn/></YColumn>".repeat(20)
            ),
        ),
        ("cumulative_xy", format!("<PrismFile>{table_6m}{table_6m}</PrismFile>")),
        (
            "nested",
            format!(
                "<PrismFile><Table TableType=\"Nested\"><RowTitlesColumn><Subcolumn>{row_ds}</Subcolumn></RowTitlesColumn>{}</Table></PrismFile>",
                format!("<YColumn>{}</YColumn>", "<Subcolumn/>".repeat(500)).repeat(25)
            ),
        ),
    ] {
        assert!(matches!(import_pzfx_str(&xml), Err(FormatError::XmlParse { offset: 0, .. })), "{label}");
    }

    let mut near_u32_max = build_raw_zip(&[RawZipEntry::stored(b"a", b"x")], b"");
    near_u32_max[74..78].copy_from_slice(&(u32::MAX - 10).to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&near_u32_max), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn hardened_result_sheet_non_finite_float_validation() {
    let base = build_rich_eight_archetype_project();
    assert!(validate_project_document(&base).is_ok());

    type DocMutFn = fn(&mut ProjectDocument);
    let mutators: &[(&str, DocMutFn)] = &[
        ("tab_num", |d| d.result_sheets[0].tabs[0].rows[0][0] = ResultCell::Number { value: f64::NAN, prefix: None, units: None }),
        ("tab_ci", |d| {
            d.result_sheets[0].tabs[0].rows[0][1] = ResultCell::ConfidenceInterval { lower: CiBound::Finite(f64::NEG_INFINITY), upper: CiBound::Open }
        }),
        ("pairwise", |d| d.result_sheets[0].pairwise_comparisons[0].estimate_diff = Some(f64::INFINITY)),
        ("curve_ci", |d| d.result_sheets[0].fitted_curves[0].points[0].ci_lower = Some(f64::NAN)),
    ];
    for &(label, mutate) in mutators {
        let mut bad = base.clone();
        mutate(&mut bad);
        assert!(matches!(validate_project_document(&bad), Err(FormatError::NonFiniteFloat { .. })), "{label}: validate");
        assert!(matches!(save_project_bytes(&bad), Err(FormatError::NonFiniteFloat { .. })), "{label}: save");
    }
}

// ============================================================================
// Proptest Property / Fuzz Harnesses
// ============================================================================

fn arb_fuzzed_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        Just(Cell::empty()),
        (-1e5_f64..1e5_f64).prop_map(|v| Cell::number((v * 1000.0).round() / 1000.0)),
        (-1e5_f64..1e5_f64).prop_map(|v| Cell::excluded_number((v * 1000.0).round() / 1000.0)),
        "[A-Za-z0-9 _-]{1,12}".prop_map(Cell::text),
        "[A-Za-z0-9 _-]{1,12}".prop_map(Cell::excluded_text),
    ]
}

fn arb_xml_fragment() -> impl Strategy<Value = String> {
    const TOKENS: &[&str] = &[
        "<PrismFile>",
        "</PrismFile>",
        "<Table TableType=\"XY\" Replicates=\"2\">",
        "<Table TableType=\"Column\">",
        "<Table TableType=\"Nested\" Replicates=\"3\">",
        "</Table>",
        "<Title>Test <sup>2</sup></Title>",
        "<InfoSequence><Info><Constant><Name>K</Name><Value>NaN</Value></Constant></Info></InfoSequence>",
        "<XColumn><Subcolumn><d>1.0</d><d>Inf</d></Subcolumn></XColumn>",
        "<YColumn><Subcolumn><d Excluded=\"1\">42.5</d><d>NaN</d><d>*</d></Subcolumn></YColumn>",
        "<RowTitlesColumn><Subcolumn><d>R1</d></Subcolumn></RowTitlesColumn>",
        "<A>",
        "</A>",
        "<!-- comment -->",
        "<!-- unclosed",
        "<![CDATA[<tag> & &#65;]]>",
        "<![CDATA[unclosed",
        "<!DOCTYPE foo>",
        "&lt;&gt;&amp;&#x3B1;&bad;",
        "NaN",
        "-Infinity",
        "\u{FEFF}",
    ];
    collection::vec(prop_oneof![proptest::sample::select(TOKENS).prop_map(String::from), "\\PC{0,24}"], 0..32).prop_map(|parts| parts.concat())
}

fn arb_csv_adversarial_text() -> impl Strategy<Value = String> {
    const PIECES: &[&str] =
        &[",", ";", "\t", "\"", "\"\"", "\n", "\r\n", "\r", "\u{FEFF}", "\0", "*", "NA", "NaN", "Inf", "-Inf", "12,34", "12.34", "45,6*", "1.234,5"];
    collection::vec(prop_oneof![proptest::sample::select(PIECES).prop_map(String::from), "\\PC{0,12}"], 0..36).prop_map(|parts| parts.concat())
}

fn arb_unsafe_zip_path() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just(String::from("..")),
        Just(String::from(".")),
        Just(String::from("/")),
        "[A-Za-z0-9_]{1,8}".prop_map(|s| format!("../{s}")),
        ("[A-Za-z0-9_]{1,8}", "[A-Za-z0-9_]{1,8}").prop_map(|(a, b)| format!("{a}/../../{b}")),
        "[A-Za-z0-9_]{1,8}".prop_map(|s| format!("/{s}/manifest.json")),
        ("[A-Za-z0-9_]{1,8}", "[A-Za-z0-9_]{1,8}").prop_map(|(a, b)| format!("{a}\\{b}")),
        ("[A-Za-z]", "[A-Za-z0-9_]{1,8}").prop_map(|(d, f)| format!("{d}:{f}")),
        ("[A-Za-z0-9_]{1,8}", "[A-Za-z0-9_]{1,8}").prop_map(|(a, b)| format!("{a}\0{b}")),
        ("[A-Za-z0-9_]{1,8}", "[A-Za-z0-9_]{1,8}").prop_map(|(a, b)| format!("{a}//{b}")),
        "[A-Za-z0-9_]{1,8}".prop_map(|s| format!("./{s}")),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn well_formed_projects_and_templates_round_trip_deterministically(
        proj_title in "[A-Za-z0-9 _\\-]{1,24}",
        info_val in -1e4_f64..1e4_f64,
        use_geom_x in any::<bool>(),
        col_cells in collection::vec(collection::vec(arb_fuzzed_cell(), 3), 2),
        nest_cells in collection::vec(collection::vec(collection::vec(arb_fuzzed_cell(), 2), 2), 2),
    ) {
        let mut doc = ProjectDocument::new(proj_title).unwrap();
        let mut info = InfoSheet::new_preseeded("info-1", "Metadata").unwrap();
        info.set_constant("Dose", ConstantValue::Number(info_val), Some(String::from("uM"))).unwrap();
        doc.add_info_sheet(info).unwrap();

        let mut xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 3).unwrap();
        if use_geom_x {
            xy.x_mode = XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Geometric };
            xy.x_cells = vec![Vec::new(); 3];
        } else {
            for r in 0..3 {
                xy.set_x_cell(r, Cell::number(r as f64)).unwrap();
            }
        }
        for (slot, (c0, c1)) in xy.data_sets[0].cells.iter_mut().zip(col_cells[0].iter().zip(&col_cells[1])) {
            *slot = vec![c0.clone(), c1.clone()];
        }
        let xy_sheet = DataSheet::new("data-xy", "XY", TableArchetype::Xy(xy)).unwrap();
        doc.add_data_sheet(xy_sheet.clone()).unwrap();

        let mut nest = NestedTable::new(2, 2, 2).unwrap();
        for (g, g_rows) in nest_cells.iter().enumerate() {
            for (r, row) in g_rows.iter().enumerate() {
                nest.groups[g].cells[r] = row.clone();
            }
        }
        doc.add_data_sheet(DataSheet::new("data-nest", "Nested", TableArchetype::Nested(nest)).unwrap()).unwrap();
        doc.add_graph_sheet(GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap()).unwrap();

        let p1 = save_project_bytes(&doc).unwrap();
        prop_assert_eq!(&p1, &save_project_bytes(&doc).unwrap());
        prop_assert_eq!(open_project_bytes(&p1).unwrap(), doc.clone());

        let t1 = save_template_bytes(&doc).unwrap();
        prop_assert_eq!(&t1, &save_template_bytes(&doc).unwrap());
        let tmpl = open_template_bytes(&t1).unwrap();
        prop_assert_eq!(tmpl.info_sheets, doc.info_sheets);
        for ds in &tmpl.data_sheets {
            for r in 0..ds.archetype.row_count() {
                if let Some(x) = ds.x_or_time_cell(r, 0) {
                    prop_assert!(x.is_empty());
                }
                for c in 0..ds.archetype.column_count() {
                    prop_assert!(ds.cell(CellCoord::new(r, c, 0)).unwrap().is_empty());
                }
            }
        }
    }

    #[test]
    fn random_truncated_and_bitflipped_archives_never_panic(
        is_template in any::<bool>(),
        cut_ratio in 0.0_f64..=1.0_f64,
        trailing in collection::vec(any::<u8>(), 0..64),
        flips in collection::vec((any::<usize>(), 1_u8..=255_u8), 1..24),
    ) {
        let doc = build_rich_eight_archetype_project();
        let base = if is_template { save_template_bytes(&doc).unwrap() } else { save_project_bytes(&doc).unwrap() };

        let cut = (((base.len() as f64) * cut_ratio).floor() as usize).min(base.len());
        let mut truncated = base[..cut].to_vec();
        if cut < base.len() {
            let trunc_res = catch_unwind(AssertUnwindSafe(|| {
                let _ = (open_archive_bytes(&truncated), open_project_bytes(&truncated), open_template_bytes(&truncated), instantiate_template_bytes(&truncated, Some("Retry")));
            }));
            prop_assert!(trunc_res.is_ok(), "panic on truncated archive");
            prop_assert!(open_archive_bytes(&truncated).is_err());
        }
        truncated.extend_from_slice(&trailing);
        prop_assert!(catch_unwind(AssertUnwindSafe(|| open_archive_bytes(&truncated))).is_ok(), "panic on truncated+trailing");

        let mut mutated = base;
        let len = mutated.len();
        for (pos, mask) in flips {
            mutated[pos % len] ^= mask;
        }
        let flip_res = catch_unwind(AssertUnwindSafe(|| (open_archive_bytes(&mutated), open_project_bytes(&mutated), open_template_bytes(&mutated), instantiate_template_bytes(&mutated, None))));
        match flip_res {
            Err(_) => return Err(TestCaseError::fail("panic on bit-flipped archive")),
            Ok((Ok((_, parsed_doc)), _, _, _)) => prop_assert!(validate_project_document(&parsed_doc).is_ok()),
            Ok((Err(_), _, _, _)) => {}
        }
    }

    #[test]
    fn unsafe_zip_paths_always_rejected_in_raw_archives(path in arb_unsafe_zip_path()) {
        prop_assert!(matches!(sanitize_zip_path(&path), Err(FormatError::UnsafePath { .. })), "sanitize {path:?}");
        let raw_zip = build_raw_zip(&[RawZipEntry::stored(path.as_bytes(), b"data")], b"");
        prop_assert!(matches!(ZipArchive::from_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "ZipArchive {path:?}");
        prop_assert!(matches!(open_archive_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "open_archive {path:?}");
        if curvebook_format::sanitize_zip_path_with_options(&path, curvebook_format::ZipReadOptions::external_interop()).is_err() {
            prop_assert!(matches!(import_prism_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "import_prism_bytes {path:?}");
        }
    }

    #[test]
    fn pzfx_xml_structural_fuzz_never_panics(xml in arb_xml_fragment()) {
        match catch_unwind(AssertUnwindSafe(|| { let _ = parse_xml(&xml); import_pzfx_str(&xml) })) {
            Err(_) => return Err(TestCaseError::fail("panic in XML / .pzfx parser")),
            Ok(Ok(report)) => prop_assert!(validate_project_document(&report.document).is_ok()),
            Ok(Err(_)) => {}
        }
    }

    #[test]
    fn prism_container_structural_fuzz_never_panics(
        manifest_body in "\\PC{0,160}",
        doc_body in "\\PC{0,200}",
        info_body in "\\PC{0,160}",
        csv_body in arb_csv_adversarial_text(),
        raw_chunk in collection::vec(any::<u8>(), 0..64),
        include_doc in any::<bool>(),
        replicates in 0_usize..600_usize,
    ) {
        let meta_bytes = serde_json::to_vec(&json!({ "title": "Fuzzed Sheet", "kind": "XY", "subcolumn_mode": "replicates", "replicates": replicates, "csv_path": "sheets/fuzzed.csv" })).unwrap();
        let mut entries = vec![
            RawZipEntry::deflated(b"manifest.json", manifest_body.as_bytes()),
            RawZipEntry::deflated(b"info/info.json", info_body.as_bytes()),
            RawZipEntry::deflated(b"sheets/fuzzed.json", &meta_bytes),
            RawZipEntry::deflated(b"sheets/fuzzed.csv", csv_body.as_bytes()),
            RawZipEntry::stored(b"chunks/blob.bin", &raw_chunk),
        ];
        if include_doc {
            entries.push(RawZipEntry::deflated(b"document.json", doc_body.as_bytes()));
        }
        match catch_unwind(AssertUnwindSafe(|| import_prism_bytes(&build_raw_zip(&entries, b"")))) {
            Err(_) => return Err(TestCaseError::fail("panic in .prism importer")),
            Ok(Ok(report)) => prop_assert!(validate_project_document(&report.document).is_ok()),
            Ok(Err(_)) => {}
        }
    }

    #[test]
    fn adversarial_csv_tsv_fuzz_never_panics(
        text in arb_csv_adversarial_text(),
        delim in prop_oneof![Just(','), Just(';'), Just('\t'), Just('\0')],
        decimal in prop_oneof![Just('.'), Just(',')],
        has_row_titles in any::<bool>(),
        header_rows in 0_usize..3_usize,
        kind_idx in 0_usize..8_usize,
    ) {
        let kind = TableKind::ALL[kind_idx];
        let mode = match kind { TableKind::Grouped | TableKind::Nested => SubcolumnMode::RawReplicates(2), _ => SubcolumnMode::Single };
        let opts = CsvOptions { delimiter: delim, decimal_separator: decimal, header_rows, has_row_titles, ..CsvOptions::csv() };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = (CsvOptions::detect(&text), parse_delimited_records(&text, &opts), parse_cell_token(&text, &opts));
            if let Ok(sheet) = import_csv_to_datasheet("fuzz-csv", "Fuzz", &text, kind, mode, &opts) {
                let _ = export_datasheet_to_csv(&sheet, &opts);
            }
            let filter = ImportFilterSpec { row_window: Some((0, 10)), col_window: Some((0, 8)), decimate_every_k: Some(1), parse_asterisk_exclusions: true, ..Default::default() };
            if let Ok(filtered) = filter_delimited_text(&text, delim, decimal, header_rows > 0, &filter) {
                let _ = rearrange_and_baseline_grid(&filtered.cells, &ImportPlacementSpec { transpose: true, skip_contiguous_blanks: true, ..Default::default() }, ImportBaselineAdjustment::PercentOfFirstRow);
            }
        }));
        prop_assert!(outcome.is_ok(), "panic on adversarial CSV/TSV input");
    }
}
