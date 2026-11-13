#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

use flate2::Compression;
use flate2::write::DeflateEncoder;
use lumencraft_doc::{
    AnalysisKind, AnalysisSpec, AxisSegment, CURRENT_SCHEMA_VERSION, Cell, CellCoord, CellValue, ChecklistItem, ChecklistStatus, CiBound,
    ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, CurveSamplePoint, DataSheet, DocError, FittedCurveOverlay, FloatingNote,
    GraphAnnotation, GraphSheet, GroupedTable, HookSource, ImportFilterSpec, ImportLinkSpec, ImportPlacementSpec, InfoSheet, InternalLinkKind,
    JournalWidthPreset, LayoutItem, LayoutSheet, LayoutSpec, LinkSource, LinkedDataRegion, MAX_DATA_SETS, MAX_SUBCOLUMNS, MAX_TITLE_BYTES,
    MultipleVariablesTable, NestedTable, NonlinearFitSpec, OneWayAnovaSpec, PairwiseBracketSpec, PairwiseComparisonRecord, PartsOfWholeTable,
    PlacementMode, ProjectDocument, RectPt, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SequenceProgression,
    SheetCellRange, SheetId, SubcolumnMode, SurvivalTable, TableArchetype, TableKind, XColumnMode, XyTable,
};
use lumencraft_format::{
    ArchiveKind, CsvOptions, FormatError, ImportBaselineAdjustment, ImportWarningKind, MAX_XML_DEPTH, MAX_ZIP_ENTRIES, XmlLimits, ZipArchive,
    ZipLimits, ZipWriter, apply_csv_import_link, export_datasheet_to_csv, filter_delimited_text, import_csv_to_datasheet, import_prism_bytes,
    import_prism_bytes_with_limits, import_pzfx_bytes, import_pzfx_str, instantiate_template_bytes, open_archive_bytes,
    open_archive_bytes_with_limits, open_project_bytes, open_template_bytes, parse_cell_token, parse_delimited_records, parse_xml,
    parse_xml_with_limits, rearrange_and_baseline_grid, sanitize_zip_path, save_project_bytes, save_template_bytes, validate_project_document,
};
use proptest::collection;
use proptest::prelude::{Just, ProptestConfig, Strategy, TestCaseError, any, prop_assert, prop_assert_eq, prop_oneof, proptest};

// ============================================================================
// Raw ZIP Crafting & Test Document Fixtures
// ============================================================================

struct RawZipEntry<'a> {
    cd_name: &'a [u8],
    local_name: Option<&'a [u8]>,
    flags: u16,
    method: u16,
    crc: Option<u32>,
    declared_compressed: Option<u32>,
    declared_uncompressed: Option<u32>,
    payload: &'a [u8],
    compressed_override: Option<Vec<u8>>,
}

impl<'a> RawZipEntry<'a> {
    fn stored(name: &'a [u8], payload: &'a [u8]) -> Self {
        Self {
            cd_name: name,
            local_name: None,
            flags: 0x0800,
            method: 0,
            crc: None,
            declared_compressed: None,
            declared_uncompressed: None,
            payload,
            compressed_override: None,
        }
    }

    fn deflated(name: &'a [u8], payload: &'a [u8]) -> Self {
        Self {
            cd_name: name,
            local_name: None,
            flags: 0x0800,
            method: 8,
            crc: None,
            declared_compressed: None,
            declared_uncompressed: None,
            payload,
            compressed_override: None,
        }
    }
}

fn deflate_raw(data: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn build_raw_zip(entries: &[RawZipEntry<'_>], eocd_comment: &[u8]) -> Vec<u8> {
    struct CdMeta<'a> {
        name: &'a [u8],
        flags: u16,
        method: u16,
        crc: u32,
        comp_len: u32,
        uncomp_len: u32,
        local_offset: u32,
    }

    let mut out = Vec::new();
    let mut cd_records = Vec::with_capacity(entries.len());

    for entry in entries {
        let local_offset = u32::try_from(out.len()).unwrap();
        let compressed = match &entry.compressed_override {
            Some(buf) => buf.clone(),
            None if entry.method == 8 => deflate_raw(entry.payload),
            None => entry.payload.to_vec(),
        };
        let crc = entry.crc.unwrap_or_else(|| crc32fast::hash(entry.payload));
        let comp_len = entry.declared_compressed.unwrap_or_else(|| u32::try_from(compressed.len()).unwrap());
        let uncomp_len = entry.declared_uncompressed.unwrap_or_else(|| u32::try_from(entry.payload.len()).unwrap());
        let local_name = entry.local_name.unwrap_or(entry.cd_name);
        let local_name_len = u16::try_from(local_name.len()).unwrap();

        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&entry.flags.to_le_bytes());
        out.extend_from_slice(&entry.method.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0x0021_u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&comp_len.to_le_bytes());
        out.extend_from_slice(&uncomp_len.to_le_bytes());
        out.extend_from_slice(&local_name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(local_name);
        out.extend_from_slice(&compressed);

        cd_records.push(CdMeta { name: entry.cd_name, flags: entry.flags, method: entry.method, crc, comp_len, uncomp_len, local_offset });
    }

    let cd_offset = u32::try_from(out.len()).unwrap();
    for rec in &cd_records {
        let name_len = u16::try_from(rec.name.len()).unwrap();
        out.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        out.extend_from_slice(&0x0314_u16.to_le_bytes());
        out.extend_from_slice(&20_u16.to_le_bytes());
        out.extend_from_slice(&rec.flags.to_le_bytes());
        out.extend_from_slice(&rec.method.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0x0021_u16.to_le_bytes());
        out.extend_from_slice(&rec.crc.to_le_bytes());
        out.extend_from_slice(&rec.comp_len.to_le_bytes());
        out.extend_from_slice(&rec.uncomp_len.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&rec.local_offset.to_le_bytes());
        out.extend_from_slice(rec.name);
    }
    let cd_size = u32::try_from(out.len()).unwrap() - cd_offset;
    let count = u16::try_from(cd_records.len()).unwrap();
    let comment_len = u16::try_from(eocd_comment.len()).unwrap();

    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&comment_len.to_le_bytes());
    out.extend_from_slice(eocd_comment);
    out
}

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
    for (r, x) in [-9.0, -8.0, -7.0].iter().enumerate() {
        xy.set_x_cell(r, Cell::number(*x)).unwrap();
    }
    xy.data_sets[0].cells[0] = vec![Cell::number(10.5), Cell::excluded_number(99.9)];
    xy.data_sets[0].cells[1] = vec![Cell::number(48.2), Cell::number(51.0)];
    xy.data_sets[1].cells[2] = vec![Cell::number(95.0), Cell::number(94.1)];
    let mut xy_sheet = DataSheet::new("data-xy", "Dose Curve", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.info_sheet_id = Some(SheetId::new("info-main"));
    xy_sheet.floating_notes.push(FloatingNote::new("note-xy", RectPt::new(8.0, 12.0, 120.0, 40.0), "Check outlier"));
    doc.add_data_sheet(xy_sheet.clone()).unwrap();

    let mut col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 3).unwrap();
    col.row_titles = vec![String::from("Subj 1"), String::from("Subj 2"), String::from("Subj 3")];
    col.data_sets[0].cells[0][0] = Cell::number(4.1);
    col.data_sets[1].cells[0][0] = Cell::number(8.4);
    let mut col_sheet = DataSheet::new("data-col", "Paired Groups", TableArchetype::Column(col)).unwrap();
    col_sheet.linked_regions.push(LinkedDataRegion {
        id: String::from("lnk-ext"),
        spec: ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("assay.csv"), delimiter: ',', decimal_separator: '.' },
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(2, 1, 0)),
        provenance_summary: String::from("Imported from assay.csv"),
    });
    doc.add_data_sheet(col_sheet.clone()).unwrap();

    let mut grp = GroupedTable::new(SubcolumnMode::MeanSemN, 2, 2).unwrap();
    grp.row_titles = vec![String::from("24h"), String::from("48h")];
    grp.data_sets[0].cells[0] = vec![Cell::number(20.0), Cell::number(1.5), Cell::number(6.0)];
    doc.add_data_sheet(DataSheet::new("data-grp", "Factorial", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    let mut cont = ContingencyTable::new(2, 2).unwrap();
    cont.row_titles = vec![String::from("Exposed"), String::from("Unexposed")];
    cont.data_sets[0].cells[0][0] = Cell::number(14.0);
    cont.data_sets[1].cells[1][0] = Cell::number(17.0);
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
    fit.parameter_rules.push(lumencraft_doc::ParameterRule {
        name: String::from("HotNM"),
        scope: lumencraft_doc::ParameterScope::HookedConstant(HookSource::InfoConstant {
            info_sheet_id: Some(SheetId::new("info-main")),
            constant_name: String::from("HotNM"),
        }),
        initial_override: None,
        lower_bound: Some(0.0),
        upper_bound: None,
    });
    let mut res = ResultSheet::new(AnalysisSpec::new("fit-1", "data-xy", AnalysisKind::NonlinearRegression(fit)).unwrap()).unwrap();
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
    });
    doc.add_result_sheet(res).unwrap();

    let anova_res =
        ResultSheet::new(AnalysisSpec::new("anova-1", "data-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    doc.add_result_sheet(anova_res).unwrap();

    let mut gsheet = GraphSheet::new_uninitialized_for_data("graph-1", &xy_sheet).unwrap();
    gsheet.spec.x_axis.segments = vec![AxisSegment::default(), AxisSegment::default()];
    gsheet.spec.comparisons.push(PairwiseBracketSpec {
        result_sheet_id: SheetId::new("anova-1"),
        group_a: 0,
        group_b: 1,
        style: lumencraft_doc::BracketStyle::Square,
        label_mode: lumencraft_doc::BracketLabelMode::SummaryStars,
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 10.0,
        bracket_step_pt: 12.0,
    });
    gsheet.spec.annotations.push(GraphAnnotation::TextBox {
        bounds_pt: RectPt::new(10.0, 10.0, 90.0, 24.0),
        markdown: String::from("EC50 = {{result:fit-1:EC50}}"),
        bordered: true,
    });
    doc.add_graph_sheet(gsheet).unwrap();

    let mut lspec = LayoutSpec::from_preset(JournalWidthPreset::DoubleColumn174Mm);
    lspec.items.push(LayoutItem::LiveGraph {
        id: String::from("panel-a"),
        graph_sheet_id: SheetId::new("graph-1"),
        bounds_pt: RectPt::new(10.0, 10.0, 200.0, 160.0),
        scale_percent: 100.0,
        badge_override: None,
    });
    lspec.items.push(LayoutItem::TextCallout {
        id: String::from("callout-1"),
        markdown: String::from("Ligand = {{info:HotNM}}"),
        bounds_pt: RectPt::new(220.0, 10.0, 120.0, 40.0),
        font_size_pt: 9.0,
    });
    doc.add_layout_sheet(LayoutSheet::new("layout-1", "Figure 1", lspec).unwrap()).unwrap();

    doc
}

fn check_no_panic<F: FnOnce()>(label: &str, f: F) {
    let outcome = catch_unwind(AssertUnwindSafe(f));
    assert!(outcome.is_ok(), "unexpected panic in {label}");
}

// ============================================================================
// 1. Deterministic Project & Template Round-Trips
// ============================================================================

#[test]
fn full_eight_archetype_project_and_template_round_trip_determinism() {
    let doc = build_rich_eight_archetype_project();

    let proj_a = save_project_bytes(&doc).unwrap();
    let proj_b = save_project_bytes(&doc).unwrap();
    assert_eq!(proj_a, proj_b, ".lumencraft save output must be byte-for-byte deterministic");

    let (kind, decoded) = open_archive_bytes(&proj_a).unwrap();
    assert_eq!(kind, ArchiveKind::Project);
    assert_eq!(decoded, doc);
    assert_eq!(open_project_bytes(&proj_a).unwrap(), doc);
    assert!(matches!(open_template_bytes(&proj_a), Err(FormatError::InvalidArchiveKind { expected: "template", found: "project" })));

    let tmpl_a = save_template_bytes(&doc).unwrap();
    let tmpl_b = save_template_bytes(&doc).unwrap();
    assert_eq!(tmpl_a, tmpl_b, ".lctemplate save output must be byte-for-byte deterministic");
    assert!(matches!(open_project_bytes(&tmpl_a), Err(FormatError::InvalidArchiveKind { expected: "project", found: "template" })));

    let tmpl = open_template_bytes(&tmpl_a).unwrap();
    assert_eq!(tmpl.info_sheets, doc.info_sheets);
    assert_eq!(tmpl.graph_sheets, doc.graph_sheets);
    assert_eq!(tmpl.layout_sheets, doc.layout_sheets);
    for res in &tmpl.result_sheets {
        assert_eq!(res.status, ResultStatus::Dirty);
        assert!(res.scalars.is_empty());
        assert!(res.tabs.is_empty());
        assert!(res.pairwise_comparisons.is_empty());
        assert!(res.fitted_curves.is_empty());
        assert!(res.checklist.is_empty());
    }
    for ds in &tmpl.data_sheets {
        for r in 0..ds.archetype.row_count() {
            if let Some(x) = ds.x_or_time_cell(r, 0) {
                assert!(x.is_empty());
            }
            for c in 0..ds.archetype.column_count() {
                assert!(ds.cell(CellCoord::new(r, c, 0)).unwrap().is_empty());
            }
        }
    }

    let inst_from_tmpl = instantiate_template_bytes(&tmpl_a, Some("Batch Plate #7")).unwrap();
    assert_eq!(inst_from_tmpl.title, "Batch Plate #7");
    assert_eq!(inst_from_tmpl.data_sheets.len(), 8);

    let inst_from_proj = instantiate_template_bytes(&proj_a, Some("Cloned From Project")).unwrap();
    assert_eq!(inst_from_proj.title, "Cloned From Project");
    assert_eq!(inst_from_proj.result_sheets[0].status, ResultStatus::Dirty);
}

// ============================================================================
// 2. Truncated & Bit-Flipped .lumencraft / .lctemplate Streams
// ============================================================================

#[test]
fn exhaustive_truncation_of_project_and_template_archives_returns_err() {
    let doc = build_rich_eight_archetype_project();
    let proj_bytes = save_project_bytes(&doc).unwrap();
    let tmpl_bytes = save_template_bytes(&doc).unwrap();

    for len in 0..proj_bytes.len() {
        let prefix = &proj_bytes[..len];
        check_no_panic("exhaustive project truncation", || {
            assert!(open_project_bytes(prefix).is_err(), "truncated project at len {len} must fail");
            assert!(open_archive_bytes(prefix).is_err(), "truncated archive at len {len} must fail");
            assert!(instantiate_template_bytes(prefix, None).is_err(), "truncated instantiate at len {len} must fail");
        });
    }

    for len in 0..tmpl_bytes.len() {
        let prefix = &tmpl_bytes[..len];
        check_no_panic("exhaustive template truncation", || {
            assert!(open_template_bytes(prefix).is_err(), "truncated template at len {len} must fail");
            assert!(open_archive_bytes(prefix).is_err(), "truncated archive at len {len} must fail");
        });
    }
}

#[test]
fn targeted_zip_header_and_manifest_corruptions_return_clean_err() {
    // 1. Multi-disk EOCD & mismatched entry counts
    let valid = build_raw_zip(&[RawZipEntry::stored(b"manifest.json", b"{}")], b"");
    let eocd_pos = valid.len() - 22;

    let mut bad_disk = valid.clone();
    bad_disk[eocd_pos + 4] = 1;
    assert!(matches!(ZipArchive::from_bytes(&bad_disk), Err(FormatError::InvalidZip { .. })));

    let mut bad_cd_disk = valid.clone();
    bad_cd_disk[eocd_pos + 6] = 2;
    assert!(matches!(ZipArchive::from_bytes(&bad_cd_disk), Err(FormatError::InvalidZip { .. })));

    let mut bad_disk_entries = valid.clone();
    bad_disk_entries[eocd_pos + 8] = 9;
    assert!(matches!(ZipArchive::from_bytes(&bad_disk_entries), Err(FormatError::InvalidZip { .. })));

    let mut bad_cd_end = valid.clone();
    bad_cd_end[eocd_pos + 12..eocd_pos + 16].copy_from_slice(&9999_u32.to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&bad_cd_end), Err(FormatError::InvalidZip { .. })));

    let mut bad_comment_len = valid.clone();
    bad_comment_len[eocd_pos + 20..eocd_pos + 22].copy_from_slice(&5_u16.to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&bad_comment_len), Err(FormatError::InvalidZip { .. })));

    // 2. Central directory local_offset near u32::MAX (must fail cleanly without overflow/panic)
    let mut bad_local_off = valid.clone();
    let cd_off = u32::from_le_bytes(bad_local_off[eocd_pos + 16..eocd_pos + 20].try_into().unwrap()) as usize;
    bad_local_off[cd_off + 42..cd_off + 46].copy_from_slice(&(u32::MAX - 5).to_le_bytes());
    assert!(matches!(ZipArchive::from_bytes(&bad_local_off), Err(FormatError::InvalidZip { .. })));

    // 3. Stored entry compressed_size != uncompressed_size without data descriptor flag
    let mut stored_mismatch = RawZipEntry::stored(b"a.txt", b"hello");
    stored_mismatch.declared_uncompressed = Some(10);
    let mismatch_zip = build_raw_zip(&[stored_mismatch], b"");
    assert!(matches!(ZipArchive::from_bytes(&mismatch_zip), Err(FormatError::InvalidZip { .. })));

    // 4. Encrypted entry flag & unsupported compression method
    let mut encrypted = RawZipEntry::stored(b"secret.txt", b"data");
    encrypted.flags = 0x0001;
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&[encrypted], b"")), Err(FormatError::UnsupportedFormat { .. })));

    let mut bzip2_entry = RawZipEntry::stored(b"data.bin", b"BZh9");
    bzip2_entry.method = 12;
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&[bzip2_entry], b"")), Err(FormatError::UnsupportedCompression { method: 12, .. })));

    // 5. Wrong manifest format magic & missing sheet CSV referenced by manifest
    let doc = build_rich_eight_archetype_project();
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let bad_magic = serde_json::json!({
        "format": "other-format",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": "X",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": []
    });
    let bad_magic_zip = build_raw_zip(
        &[RawZipEntry::deflated(b"document.json", &doc_bytes), RawZipEntry::deflated(b"manifest.json", &serde_json::to_vec(&bad_magic).unwrap())],
        b"",
    );
    assert!(matches!(open_project_bytes(&bad_magic_zip), Err(FormatError::UnsupportedFormat { .. })));

    let missing_sheet_csv = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": "X",
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{
            "id": "data-xy",
            "section": "Data",
            "title": "Dose Curve",
            "csv_path": "sheets/missing.csv",
            "crc32": 12345
        }]
    });
    let missing_csv_zip = build_raw_zip(
        &[
            RawZipEntry::deflated(b"document.json", &doc_bytes),
            RawZipEntry::deflated(b"manifest.json", &serde_json::to_vec(&missing_sheet_csv).unwrap()),
        ],
        b"",
    );
    assert!(matches!(open_project_bytes(&missing_csv_zip), Err(FormatError::MissingArchiveEntry { .. })));
}

// ============================================================================
// 3. Zip-Slip Path Traversal, Zip Bombs, Corrupt CRC32, Invalid UTF-8
// ============================================================================

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
        assert!(matches!(sanitize_zip_path(path), Err(FormatError::UnsafePath { .. })), "sanitize_zip_path accepted `{path:?}`");
        assert!(matches!(writer.add_file(path, b"x".to_vec()), Err(FormatError::UnsafePath { .. })), "ZipWriter accepted `{path:?}`");

        let raw_zip = build_raw_zip(&[RawZipEntry::stored(path.as_bytes(), b"payload")], b"");
        assert!(matches!(ZipArchive::from_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "ZipArchive accepted `{path:?}`");
        assert!(matches!(open_archive_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "open_archive_bytes accepted `{path:?}`");
        assert!(matches!(import_prism_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })), "import_prism_bytes accepted `{path:?}`");
    }

    // Duplicate entry in raw Central Directory must be rejected.
    let dup_zip = build_raw_zip(&[RawZipEntry::stored(b"sheets/a.csv", b"1"), RawZipEntry::stored(b"sheets/a.csv", b"2")], b"");
    assert!(matches!(ZipArchive::from_bytes(&dup_zip), Err(FormatError::DuplicateEntry { .. })));

    // Local header filename spoofing `../../../../etc/passwd` while CD says `data/ok.csv`:
    // ZipArchive ignores the local header path and exposes only the sanitized CD path.
    let mut spoofed = RawZipEntry::stored(b"data/ok.csv", b"X,Y\n1,2\n");
    spoofed.local_name = Some(b"../../../../etc/passwd");
    let spoof_zip = build_raw_zip(&[spoofed], b"");
    let archive = ZipArchive::from_bytes(&spoof_zip).unwrap();
    let keys: Vec<&str> = archive.iter().map(|(k, _)| k).collect();
    assert_eq!(keys, vec!["data/ok.csv"]);
    assert!(keys.iter().all(|k| !k.contains("..")));
}

#[test]
fn zip_bomb_declared_sizes_lying_deflate_and_cumulative_budgets_rejected() {
    // 1. Huge declared uncompressed size (4 GiB - 1) on tiny payload: rejected immediately without allocating 4 GiB.
    let mut huge_declared = RawZipEntry::deflated(b"huge.csv", b"tiny");
    huge_declared.declared_uncompressed = Some(u32::MAX);
    let huge_zip = build_raw_zip(&[huge_declared], b"");
    assert!(matches!(ZipArchive::from_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(open_archive_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(import_prism_bytes(&huge_zip), Err(FormatError::ZipBomb { .. })));

    // Even with custom large limits, preallocation is capped and header mismatch fails cleanly without OOM.
    let relaxed = ZipLimits { max_entries: 16, max_total_uncompressed_bytes: u64::MAX, max_entry_uncompressed_bytes: u64::MAX };
    let mut huge_declared_2 = RawZipEntry::deflated(b"huge2.csv", b"tiny");
    huge_declared_2.declared_uncompressed = Some(u32::MAX);
    assert!(ZipArchive::from_bytes_with_limits(&build_raw_zip(&[huge_declared_2], b""), relaxed).is_err());

    // 2. Cumulative declared uncompressed sizes across entries exceeding archive total cap (each entry individually < entry cap).
    let mut e1 = RawZipEntry::stored(b"a.bin", b"x");
    e1.declared_uncompressed = Some(50 * 1024 * 1024);
    let mut e2 = RawZipEntry::stored(b"b.bin", b"y");
    e2.declared_uncompressed = Some(50 * 1024 * 1024);
    let mut e3 = RawZipEntry::stored(b"c.bin", b"z");
    e3.declared_uncompressed = Some(50 * 1024 * 1024);
    let cum_zip = build_raw_zip(&[e1, e2, e3], b"");
    assert!(matches!(ZipArchive::from_bytes(&cum_zip), Err(FormatError::ZipBomb { .. })));

    // 3. Lying deflate header claiming uncompressed_size = 32 bytes while stream expands to 256 KiB of zeros.
    let zeros = vec![0_u8; 256 * 1024];
    let mut lying = RawZipEntry::deflated(b"data/bomb.csv", &zeros);
    lying.declared_uncompressed = Some(32);
    let lying_zip = build_raw_zip(&[lying], b"");
    let tight = ZipLimits { max_entries: 32, max_total_uncompressed_bytes: 8192, max_entry_uncompressed_bytes: 8192 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(open_archive_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(import_prism_bytes_with_limits(&lying_zip, tight), Err(FormatError::ZipBomb { .. })));

    // 4. Cumulative actual decompression budget across multiple entries.
    let part_a = vec![b'A'; 3000];
    let part_b = vec![b'B'; 3000];
    let multi_zip = build_raw_zip(&[RawZipEntry::deflated(b"a.txt", &part_a), RawZipEntry::deflated(b"b.txt", &part_b)], b"");
    let split_limits = ZipLimits { max_entries: 32, max_total_uncompressed_bytes: 4096, max_entry_uncompressed_bytes: 4096 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&multi_zip, split_limits), Err(FormatError::ZipBomb { .. })));

    // 5. Entry count exceeding MAX_ZIP_ENTRIES (4096 + 1)
    let mut entries = Vec::with_capacity(MAX_ZIP_ENTRIES + 1);
    let names: Vec<String> = (0..=MAX_ZIP_ENTRIES).map(|i| format!("e/{i}.txt")).collect();
    for name in &names {
        entries.push(RawZipEntry::stored(name.as_bytes(), b""));
    }
    let too_many_zip = build_raw_zip(&entries, b"");
    assert!(matches!(ZipArchive::from_bytes(&too_many_zip), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn corrupt_crc32_and_invalid_utf8_entries_handled_cleanly() {
    // 1. Corrupt ZIP entry CRC32 (both stored and deflated)
    let mut bad_stored = RawZipEntry::stored(b"manifest.json", b"{}");
    bad_stored.crc = Some(crc32fast::hash(b"{}") ^ 0x1234_5678);
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&[bad_stored], b"")), Err(FormatError::CorruptCrc { .. })));

    let mut bad_deflated = RawZipEntry::deflated(b"manifest.json", b"{ \"hello\": \"world\" }");
    bad_deflated.crc = Some(0xDEAD_BEEF);
    assert!(matches!(ZipArchive::from_bytes(&build_raw_zip(&[bad_deflated], b"")), Err(FormatError::CorruptCrc { .. })));

    // 2. Corrupt sheet CSV CRC32 inside valid ZIP & valid manifest.document_crc32
    let doc = build_rich_eight_archetype_project();
    let doc_bytes = serde_json::to_vec(&doc).unwrap();
    let csv_bytes = export_datasheet_to_csv(&doc.data_sheets[0], &CsvOptions::csv()).unwrap().into_bytes();
    let bad_sheet_manifest = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": doc.title,
        "document_crc32": crc32fast::hash(&doc_bytes),
        "sheets": [{
            "id": doc.data_sheets[0].id,
            "section": "Data",
            "title": doc.data_sheets[0].title,
            "csv_path": "sheets/data-xy.csv",
            "crc32": crc32fast::hash(&csv_bytes) ^ 1
        }]
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

    // 3. Invalid UTF-8 entry filename in raw ZIP
    let bad_name_zip = build_raw_zip(&[RawZipEntry::stored(&[0xFF, 0xFE, b'x'], b"payload")], b"");
    assert!(matches!(ZipArchive::from_bytes(&bad_name_zip), Err(FormatError::InvalidZip { .. })));

    // 4. Invalid UTF-8 in manifest.json & document.json
    let bad_manifest_utf8 = build_raw_zip(&[RawZipEntry::stored(b"manifest.json", &[0xFF, 0x80, 0xBF])], b"");
    assert!(matches!(open_archive_bytes(&bad_manifest_utf8), Err(FormatError::InvalidZip { .. })));

    let manifest_ok = serde_json::json!({
        "format": "lumencraft-archive",
        "schema_version": CURRENT_SCHEMA_VERSION,
        "kind": "Project",
        "title": "Bad UTF8 Doc",
        "document_crc32": crc32fast::hash(&[0xFF, 0x80, 0xFE]),
        "sheets": []
    });
    let bad_doc_utf8 = build_raw_zip(
        &[
            RawZipEntry::stored(b"manifest.json", &serde_json::to_vec(&manifest_ok).unwrap()),
            RawZipEntry::stored(b"document.json", &[0xFF, 0x80, 0xFE]),
        ],
        b"",
    );
    assert!(matches!(open_project_bytes(&bad_doc_utf8), Err(FormatError::Json(_))));

    // 5. Invalid UTF-8 CSV inside `.prism` archive emits UnsupportedChunk warning instead of panicking.
    let prism_bad_csv = build_raw_zip(
        &[RawZipEntry::stored(b"sheets/corrupt.csv", &[0xFF, 0xFE, b',', b'1']), RawZipEntry::stored(b"sheets/good.csv", b"A\n1\n")],
        b"",
    );
    let prism_report = import_prism_bytes(&prism_bad_csv).unwrap();
    assert_eq!(prism_report.document.data_sheets.len(), 1);
    assert!(prism_report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk && w.source == "sheets/corrupt.csv"));

    // 6. Invalid UTF-8 in `.pzfx` bytes
    assert!(matches!(import_pzfx_bytes(&[0xFF, 0xFE, b'<', b'P']), Err(FormatError::XmlParse { .. })));
}

// ============================================================================
// 4. Adversarial .pzfx XML & .prism JSON Structural Fuzzing
// ============================================================================

#[test]
fn adversarial_pzfx_xml_unclosed_deep_nesting_missing_tables_and_non_finite_tokens() {
    // 1. Structural XML syntax failures
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
        "<PrismFile/><PrismFile/>",
        "<WrongRoot/>",
    ];
    for case in bad_xml_cases {
        check_no_panic("bad xml case", || {
            assert!(import_pzfx_str(case).is_err(), "expected Err for malformed XML `{case}`");
        });
    }

    // 2. Huge XML nesting depth (> MAX_XML_DEPTH = 128 and > 2000 levels) must fail cleanly without stack overflow.
    for depth in [MAX_XML_DEPTH + 1, 512, 2_500] {
        let mut deep = String::from("<PrismFile>");
        for _ in 0..depth {
            deep.push_str("<A>");
        }
        for _ in 0..depth {
            deep.push_str("</A>");
        }
        deep.push_str("</PrismFile>");
        assert!(matches!(parse_xml(&deep), Err(FormatError::XmlDepthExceeded { .. })));
        assert!(matches!(import_pzfx_str(&deep), Err(FormatError::XmlDepthExceeded { .. })));
    }

    // Tight node & byte caps
    let many_nodes = "<PrismFile><A/><A/><A/><A/><A/></PrismFile>";
    assert!(matches!(
        parse_xml_with_limits(many_nodes, XmlLimits { max_bytes: 1024, max_depth: 32, max_nodes: 3 }),
        Err(FormatError::XmlParse { .. })
    ));
    assert!(matches!(
        parse_xml_with_limits(many_nodes, XmlLimits { max_bytes: 8, max_depth: 32, max_nodes: 100 }),
        Err(FormatError::XmlParse { .. })
    ));

    // 3. Missing tables, empty elements, weird Replicates, NaN/Inf tokens, and oversized UTF-8 titles
    let empty_pzfx = "<PrismFile><Title>Empty Study</Title><TableSequence/></PrismFile>";
    let empty_rep = import_pzfx_str(empty_pzfx).unwrap();
    assert_eq!(empty_rep.document.title, "Empty Study");
    assert!(empty_rep.document.data_sheets.is_empty());

    let long_utf8_title = "é".repeat(MAX_TITLE_BYTES); // 2 bytes per char -> 8192 bytes, straddles MAX_TITLE_BYTES=4096
    let weird_pzfx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile>
  <Title>{long_utf8_title}</Title>
  <InfoSequence>
    <Info ID="">
      <Title>Info</Title>
      <Constant><Name>NanVal</Name><Value>NaN</Value></Constant>
      <Constant><Name>InfVal</Name><Value>Inf</Value></Constant>
      <Constant><Name>NegInfVal</Name><Value>-Infinity</Value></Constant>
      <Constant><Name>Huge</Name><Value>1e500</Value></Constant>
      <Constant><Name></Name><Value>123</Value></Constant>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="   " TableType="XY" XFormat="start-step" X0="NaN" DeltaX="Inf" Replicates="0">
      <Title>{long_utf8_title}</Title>
      <XColumn><Subcolumn><d>NaN</d><d>Inf</d><d>-Inf</d><d>1e500</d></Subcolumn></XColumn>
      <YColumn><Subcolumn><d>NaN</d><d>*NaN</d><d>Inf*</d><d>*</d></Subcolumn></YColumn>
    </Table>
    <Table ID="dup" TableType="Column" Replicates="999999999999999999999999">
      <YColumn/>
    </Table>
    <Table ID="dup" TableType="PartsOfWhole"/>
  </TableSequence>
  <GraphSequence><Graph ID="g1"/></GraphSequence>
  <LayoutSequence><Layout ID="l1"/></LayoutSequence>
  <ScriptSequence><Script ID="s1"/></ScriptSequence>
</PrismFile>"#
    );
    let rep = import_pzfx_str(&weird_pzfx).unwrap();
    assert!(rep.document.title.len() <= MAX_TITLE_BYTES);
    assert_eq!(rep.document.data_sheets.len(), 3);
    assert!(rep.warnings.iter().any(|w| w.kind == ImportWarningKind::TruncatedTitle));
    assert!(rep.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection));
    validate_project_document(&rep.document).unwrap();
    let round_bytes = save_project_bytes(&rep.document).unwrap();
    assert_eq!(open_project_bytes(&round_bytes).unwrap(), rep.document);

    // Replicates > MAX_SUBCOLUMNS rejected cleanly
    let too_many_reps = format!("<PrismFile><Table TableType=\"XY\" Replicates=\"{}\"/></PrismFile>", MAX_SUBCOLUMNS + 1);
    assert!(matches!(import_pzfx_str(&too_many_reps), Err(FormatError::Doc(DocError::TooManySubcolumns { .. }))));
}

#[test]
fn adversarial_prism_json_structural_cases_handled_cleanly() {
    // 1. Deeply nested JSON (300 levels) hits serde_json recursion cap cleanly (no stack overflow)
    let mut deep_json = "[".repeat(300);
    deep_json.push_str(&"]".repeat(300));
    let deep_manifest_zip =
        build_raw_zip(&[RawZipEntry::stored(b"manifest.json", deep_json.as_bytes()), RawZipEntry::stored(b"sheets/a.csv", b"A\n1\n")], b"");
    let deep_rep = import_prism_bytes(&deep_manifest_zip).unwrap();
    assert!(deep_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::MissingMetadata));

    let deep_doc_zip = build_raw_zip(
        &[
            RawZipEntry::stored(b"document.json", deep_json.as_bytes()),
            RawZipEntry::stored(b"manifest.json", deep_json.as_bytes()),
            RawZipEntry::stored(b"info/info.json", deep_json.as_bytes()),
        ],
        b"",
    );
    let deep_doc_rep = import_prism_bytes(&deep_doc_zip).unwrap();
    assert!(deep_doc_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk));

    // 2. Embedded document.json with NaN/Inf literal tokens, bad schema versions, cycles, duplicate IDs, oversized titles
    let nan_doc_json = br#"{"schema_version":1,"title":"NaN","data_sheets":[{"id":"d1","title":"D","archetype":{"Column":{"design_hint":"UnpairedReplicates","subcolumn_mode":"Single","row_titles":[],"row_titles_expanded":false,"data_sets":[{"title":"A","cells":[[{"value":{"Number":NaN},"excluded":false}]]}]}},"frozen":false,"info_sheet_id":null,"linked_regions":[],"floating_notes":[],"color_tag":null}],"info_sheets":[],"result_sheets":[],"graph_sheets":[],"layout_sheets":[],"active_sheet_id":null,"family_anchor_sheet_id":null}"#;
    let nan_prism = build_raw_zip(&[RawZipEntry::stored(b"document.json", nan_doc_json)], b"");
    // serde_json rejects literal `NaN`, falling back to chunk warning with empty valid project
    let nan_rep = import_prism_bytes(&nan_prism).unwrap();
    assert!(nan_rep.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedChunk));

    let future_doc = serde_json::json!({
        "schema_version": CURRENT_SCHEMA_VERSION + 42,
        "title": "Future",
        "data_sheets": [],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": [],
        "active_sheet_id": null,
        "family_anchor_sheet_id": null
    });
    assert!(matches!(
        import_prism_bytes(&build_raw_zip(&[RawZipEntry::stored(b"document.json", &serde_json::to_vec(&future_doc).unwrap())], b"")),
        Err(FormatError::UnsupportedSchemaVersion { .. })
    ));

    // schema_version > u32::MAX inside document.json
    let u64_ver_doc = serde_json::json!({
        "schema_version": 9_999_999_999_999_u64,
        "title": "Huge Ver",
        "data_sheets": [],
        "info_sheets": [],
        "result_sheets": [],
        "graph_sheets": [],
        "layout_sheets": [],
        "active_sheet_id": null,
        "family_anchor_sheet_id": null
    });
    assert!(matches!(
        import_prism_bytes(&build_raw_zip(&[RawZipEntry::stored(b"document.json", &serde_json::to_vec(&u64_ver_doc).unwrap())], b"")),
        Err(FormatError::Json(_))
    ));

    // Dependency cycle embedded in document.json
    let mut cycle_doc = ProjectDocument::new("Cycle").unwrap();
    let res_self = ResultSheet::new(AnalysisSpec::new("r1", "r1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap()).unwrap();
    cycle_doc.add_result_sheet(res_self).unwrap();
    let cycle_zip = build_raw_zip(&[RawZipEntry::stored(b"document.json", &serde_json::to_vec(&cycle_doc).unwrap())], b"");
    assert!(matches!(import_prism_bytes(&cycle_zip), Err(FormatError::DependencyCycle { .. })));

    // Oversized replicates in PrismTableMeta rejected cleanly
    let bad_meta = serde_json::json!({
        "title": "Bad Reps",
        "kind": "XY",
        "subcolumn_mode": "replicates",
        "replicates": MAX_SUBCOLUMNS + 10,
        "csv_path": "sheets/xy.csv"
    });
    let bad_meta_zip = build_raw_zip(
        &[RawZipEntry::stored(b"sheets/xy.json", &serde_json::to_vec(&bad_meta).unwrap()), RawZipEntry::stored(b"sheets/xy.csv", b"X,Y\n1,2\n")],
        b"",
    );
    assert!(matches!(import_prism_bytes(&bad_meta_zip), Err(FormatError::Doc(DocError::TooManySubcolumns { .. }))));
}

// ============================================================================
// 5. Adversarial RFC-4180 CSV/TSV Edge Cases
// ============================================================================

#[test]
fn adversarial_rfc4180_csv_tsv_edge_cases() {
    let opts = CsvOptions::csv();

    // 1. Unclosed quotes & malformed quoting
    for bad in ["\"unclosed", "a,\"unclosed", "\"escaped\"\"quote", "a,b\n\"c,d", "a\"b,c", "\"ab\"c,d"] {
        assert!(matches!(parse_delimited_records(bad, &opts), Err(FormatError::CsvParse { .. })), "expected CsvParse error for `{bad:?}`");
    }

    // 2. BOM handling, CRLF / LF / CR mixes, and quoted embedded newlines
    let mixed = "\u{FEFF}\"H,1\"\t\"H\r\n2\"\r\n1\t2\r3\t\"line1\nline2\rline3\"\n\u{FEFF}4\t5\r";
    let tsv_opts = CsvOptions::tsv();
    let recs = parse_delimited_records(mixed, &tsv_opts).unwrap();
    assert_eq!(recs, vec![vec!["H,1", "H\r\n2"], vec!["1", "2"], vec!["3", "line1\nline2\rline3"], vec!["\u{FEFF}4", "5"]]);
    assert!(parse_delimited_records("\u{FEFF}", &opts).unwrap().is_empty());

    // 3. Uneven / jagged row lengths across all 8 TableKinds
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
        let sheet = import_csv_to_datasheet("jagged", "Jagged", jagged, kind, mode, &opts).unwrap();
        let exported = export_datasheet_to_csv(&sheet, &opts).unwrap();
        assert!(!exported.is_empty());
    }

    // 4. Comma-decimal vs dot-decimal, sentinels, NaN/Inf, asterisks
    let euro_text = "\u{FEFF}Group A;Group B\r\n1.234,5;Inf\r\n-0,25*;NaN\r\n*;-Infinity\r\n";
    let detected = CsvOptions::detect(euro_text);
    assert_eq!(detected.delimiter, ';');
    assert_eq!(detected.decimal_separator, ',');
    let mut euro = CsvOptions::european_csv();
    euro.thousands_separator = Some('.');
    assert_eq!(parse_cell_token("1.234,5", &euro), Cell::number(1234.5));
    assert_eq!(parse_cell_token("-0,25*", &euro), Cell::excluded_number(-0.25));
    assert_eq!(parse_cell_token("NaN", &euro), Cell::empty());
    assert_eq!(parse_cell_token("Inf", &euro), Cell::text("Inf"));
    assert_eq!(parse_cell_token("-Infinity", &euro), Cell::text("-Infinity"));
    assert_eq!(parse_cell_token("1e500", &euro), Cell::text("1e500"));
    assert_eq!(parse_cell_token("*NaN", &euro), Cell { value: CellValue::Empty, excluded: true });

    // 5. NUL bytes inside unquoted & quoted CSV/TSV fields
    let nul_csv = "H\0A,H\0B\n\"v\x001\",2\0x\n";
    let nul_recs = parse_delimited_records(nul_csv, &opts).unwrap();
    assert_eq!(nul_recs, vec![vec!["H\0A", "H\0B"], vec!["v\x001", "2\0x"]]);

    // 6. Column count exceeding MAX_DATA_SETS * MAX_SUBCOLUMNS + 4 rejected cleanly
    let max_cols = MAX_DATA_SETS * MAX_SUBCOLUMNS + 4;
    let too_many_cols_csv = ",".repeat(max_cols);
    assert!(matches!(parse_delimited_records(&too_many_cols_csv, &opts), Err(FormatError::CsvParse { .. })));
}

#[test]
fn adversarial_csv_import_link_pipeline_and_frozen_sheets() {
    // Adversarial ImportLinkSpec pipeline bounds & zero-baseline safety
    let bad_row_win = ImportFilterSpec { row_window: Some((5, 2)), ..ImportFilterSpec::default() };
    assert!(matches!(filter_delimited_text("A\n1\n2\n", ',', '.', true, &bad_row_win), Err(FormatError::InvalidPipeline { .. })));

    let bad_col_win = ImportFilterSpec { col_window: Some((3, 1)), ..ImportFilterSpec::default() };
    assert!(matches!(filter_delimited_text("A,B\n1,2\n", ',', '.', true, &bad_col_win), Err(FormatError::InvalidPipeline { .. })));

    let zero_decimate = ImportFilterSpec { decimate_every_k: Some(0), ..ImportFilterSpec::default() };
    assert!(matches!(filter_delimited_text("A\n1\n", ',', '.', true, &zero_decimate), Err(FormatError::InvalidPipeline { .. })));

    let oob_unstack = ImportFilterSpec { unstack_group_column: Some(99), ..ImportFilterSpec::default() };
    assert!(matches!(filter_delimited_text("A,B\n1,2\n", ',', '.', true, &oob_unstack), Err(FormatError::InvalidPipeline { .. })));

    // PercentOfFirstRow with 0.0 in row 0 must never produce NaN/Inf
    let grid = vec![vec![Cell::number(0.0), Cell::number(10.0)], vec![Cell::number(5.0), Cell::number(20.0)]];
    let norm = rearrange_and_baseline_grid(&grid, &ImportPlacementSpec::default(), ImportBaselineAdjustment::PercentOfFirstRow);
    for row in &norm {
        for cell in row {
            if let CellValue::Number(v) = cell.value {
                assert!(v.is_finite());
            }
        }
    }

    // Frozen target sheet rejects apply_csv_import_link
    let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 1).unwrap();
    let mut frozen_sheet = DataSheet::new("frozen-1", "Frozen", TableArchetype::Column(col)).unwrap();
    frozen_sheet.frozen = true;
    let spec = ImportLinkSpec {
        source: LinkSource::ExternalDelimitedFile { path: String::from("in.csv"), delimiter: ',', decimal_separator: '.' },
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { mode: PlacementMode::Overwrite, ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_csv_import_link(&mut frozen_sheet, "A\n1\n", &spec, true, ImportBaselineAdjustment::None, "l1", "summary"),
        Err(FormatError::Doc(DocError::SheetFrozen { .. }))
    ));
}

#[test]
fn hardened_import_cell_budget_and_anchor_overflow_regressions() {
    let opts = CsvOptions::csv();

    // 1. Small jagged CSV (~30 KB) whose dense grid exceeds MAX_IMPORT_CELLS must fail cleanly (no OOM).
    let mut jagged_oom_csv = format!("{}\n", ",".repeat(1_000));
    jagged_oom_csv.push_str(&"\n".repeat(10_005));
    assert!(matches!(
        import_csv_to_datasheet("oom-col", "OOM Column", &jagged_oom_csv, TableKind::Column, SubcolumnMode::Single, &opts),
        Err(FormatError::CsvParse { .. })
    ));

    // 2. Xy with RawReplicates(16), 100 datasets, and 7_000 rows -> > 10M dense cells cleanly rejected.
    let mut xy_oom_csv = format!("{}\n", ",".repeat(1_600));
    xy_oom_csv.push_str(&"\n".repeat(7_000));
    assert!(matches!(
        import_csv_to_datasheet("oom-xy", "OOM XY", &xy_oom_csv, TableKind::Xy, SubcolumnMode::RawReplicates(16), &opts),
        Err(FormatError::CsvParse { .. })
    ));

    // 3. Long CSV unstacking 1_001 groups where one group has 10_005 rows (> MAX_IMPORT_CELLS) -> InvalidPipeline.
    let mut unstack_oom = String::from("Group,Val\n");
    for g in 0..1_001 {
        unstack_oom.push_str(&format!("G{g},1\n"));
    }
    for _ in 0..10_005 {
        unstack_oom.push_str("G0,2\n");
    }
    let unstack_filter = ImportFilterSpec { unstack_group_column: Some(0), ..ImportFilterSpec::default() };
    assert!(matches!(filter_delimited_text(&unstack_oom, ',', '.', false, &unstack_filter), Err(FormatError::InvalidPipeline { .. })));

    // 4. ImportPlacementSpec.anchor overflow (`usize::MAX` row/col/subcol) & growth budget cap must never panic or OOM.
    let make_xy_target = || {
        let xy = XyTable::new(SubcolumnMode::RawReplicates(2), 1, 1).unwrap();
        DataSheet::new("tgt", "Target", TableArchetype::Xy(xy)).unwrap()
    };
    let ext_source = LinkSource::ExternalDelimitedFile { path: String::from("in.csv"), delimiter: ',', decimal_separator: '.' };
    for bad_anchor in [CellCoord::new(usize::MAX, 0, 0), CellCoord::new(0, usize::MAX, 0), CellCoord::new(0, 0, usize::MAX)] {
        let mut target = make_xy_target();
        let spec = ImportLinkSpec {
            source: ext_source.clone(),
            filter: ImportFilterSpec::default(),
            placement: ImportPlacementSpec { anchor: bad_anchor, mode: PlacementMode::Overwrite, ..ImportPlacementSpec::default() },
        };
        assert!(matches!(
            apply_csv_import_link(&mut target, "1\n2\n", &spec, false, ImportBaselineAdjustment::None, "l1", "summary"),
            Err(FormatError::InvalidPipeline { .. })
        ));
    }

    let col_tbl = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 25, 1).unwrap();
    let mut wide_target = DataSheet::new("wide", "Wide", TableArchetype::Column(col_tbl)).unwrap();
    let growth_spec = ImportLinkSpec {
        source: ext_source,
        filter: ImportFilterSpec::default(),
        placement: ImportPlacementSpec { anchor: CellCoord::new(500_000, 0, 0), mode: PlacementMode::Overwrite, ..ImportPlacementSpec::default() },
    };
    assert!(matches!(
        apply_csv_import_link(&mut wide_target, "1\n", &growth_spec, false, ImportBaselineAdjustment::None, "l1", "summary"),
        Err(FormatError::InvalidPipeline { .. })
    ));

    // 5. Small `.pzfx` single-table & cumulative multi-table & Nested cell-budget enforcement.
    let row_ds_1000 = "<d/>".repeat(1_000);
    let xy_ycols_20 = "<YColumn><Subcolumn/></YColumn>".repeat(20);
    let pzfx_single_oom = format!(
        "<PrismFile><Table TableType=\"XY\" Replicates=\"512\"><RowTitlesColumn><Subcolumn>{row_ds_1000}</Subcolumn></RowTitlesColumn>{xy_ycols_20}</Table></PrismFile>"
    );
    assert!(matches!(import_pzfx_str(&pzfx_single_oom), Err(FormatError::XmlParse { offset: 0, .. })));

    let xy_ycols_12 = "<YColumn><Subcolumn/></YColumn>".repeat(12);
    let table_6m = format!(
        "<Table TableType=\"XY\" Replicates=\"500\"><RowTitlesColumn><Subcolumn>{row_ds_1000}</Subcolumn></RowTitlesColumn>{xy_ycols_12}</Table>"
    );
    let pzfx_cumulative_oom = format!("<PrismFile>{table_6m}{table_6m}</PrismFile>");
    assert!(matches!(import_pzfx_str(&pzfx_cumulative_oom), Err(FormatError::XmlParse { offset: 0, .. })));

    let nested_subs_500 = "<Subcolumn/>".repeat(500);
    let nested_groups_25 = format!("<YColumn>{nested_subs_500}</YColumn>").repeat(25);
    let pzfx_nested_oom = format!(
        "<PrismFile><Table TableType=\"Nested\"><RowTitlesColumn><Subcolumn>{row_ds_1000}</Subcolumn></RowTitlesColumn>{nested_groups_25}</Table></PrismFile>"
    );
    assert!(matches!(import_pzfx_str(&pzfx_nested_oom), Err(FormatError::XmlParse { offset: 0, .. })));

    // 6. Central Directory local_offset near u32::MAX (wasm32 checked_add regression) returns clean Err.
    let mut near_u32_max_zip = build_raw_zip(&[RawZipEntry::stored(b"a", b"x")], b"");
    let bogus_offset = (u32::MAX - 10).to_le_bytes();
    near_u32_max_zip[74..78].copy_from_slice(&bogus_offset);
    assert!(matches!(ZipArchive::from_bytes(&near_u32_max_zip), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn hardened_result_sheet_non_finite_float_validation() {
    let base_doc = build_rich_eight_archetype_project();
    assert!(validate_project_document(&base_doc).is_ok());

    let mut bad_tab_num = base_doc.clone();
    bad_tab_num.result_sheets[0].tabs[0].rows[0][0] = ResultCell::Number { value: f64::NAN, prefix: None, units: None };
    assert!(matches!(validate_project_document(&bad_tab_num), Err(FormatError::NonFiniteFloat { .. })));
    assert!(matches!(save_project_bytes(&bad_tab_num), Err(FormatError::NonFiniteFloat { .. })));

    let mut bad_tab_ci = base_doc.clone();
    bad_tab_ci.result_sheets[0].tabs[0].rows[0][1] =
        ResultCell::ConfidenceInterval { lower: CiBound::Finite(f64::NEG_INFINITY), upper: CiBound::Open };
    assert!(matches!(validate_project_document(&bad_tab_ci), Err(FormatError::NonFiniteFloat { .. })));

    let mut bad_pairwise = base_doc.clone();
    bad_pairwise.result_sheets[0].pairwise_comparisons[0].estimate_diff = Some(f64::INFINITY);
    assert!(matches!(validate_project_document(&bad_pairwise), Err(FormatError::NonFiniteFloat { .. })));

    let mut bad_curve = base_doc;
    bad_curve.result_sheets[0].fitted_curves[0].points[0].ci_lower = Some(f64::NAN);
    assert!(matches!(save_project_bytes(&bad_curve), Err(FormatError::NonFiniteFloat { .. })));
}

// ============================================================================
// 6. Proptest Property / Fuzz Harnesses
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
    let token = prop_oneof![
        Just(String::from("<PrismFile>")),
        Just(String::from("</PrismFile>")),
        Just(String::from("<Table TableType=\"XY\" Replicates=\"2\">")),
        Just(String::from("<Table TableType=\"Column\">")),
        Just(String::from("<Table TableType=\"Nested\" Replicates=\"3\">")),
        Just(String::from("</Table>")),
        Just(String::from("<Title>Test <sup>2</sup></Title>")),
        Just(String::from("<InfoSequence><Info><Constant><Name>K</Name><Value>NaN</Value></Constant></Info></InfoSequence>")),
        Just(String::from("<XColumn><Subcolumn><d>1.0</d><d>Inf</d></Subcolumn></XColumn>")),
        Just(String::from("<YColumn><Subcolumn><d Excluded=\"1\">42.5</d><d>NaN</d><d>*</d></Subcolumn></YColumn>")),
        Just(String::from("<RowTitlesColumn><Subcolumn><d>R1</d></Subcolumn></RowTitlesColumn>")),
        Just(String::from("<A>")),
        Just(String::from("</A>")),
        Just(String::from("<!-- comment -->")),
        Just(String::from("<!-- unclosed")),
        Just(String::from("<![CDATA[<tag> & &#65;]]>")),
        Just(String::from("<![CDATA[unclosed")),
        Just(String::from("<!DOCTYPE foo>")),
        Just(String::from("&lt;&gt;&amp;&#x3B1;&bad;")),
        Just(String::from("NaN")),
        Just(String::from("-Infinity")),
        Just(String::from("\u{FEFF}")),
        "\\PC{0,24}",
    ];
    collection::vec(token, 0..32).prop_map(|parts| parts.concat())
}

fn arb_csv_adversarial_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just(String::from(",")),
        Just(String::from(";")),
        Just(String::from("\t")),
        Just(String::from("\"")),
        Just(String::from("\"\"")),
        Just(String::from("\n")),
        Just(String::from("\r\n")),
        Just(String::from("\r")),
        Just(String::from("\u{FEFF}")),
        Just(String::from("\0")),
        Just(String::from("*")),
        Just(String::from("NA")),
        Just(String::from("NaN")),
        Just(String::from("Inf")),
        Just(String::from("-Inf")),
        Just(String::from("12,34")),
        Just(String::from("12.34")),
        Just(String::from("45,6*")),
        Just(String::from("1.234,5")),
        "\\PC{0,12}",
    ];
    collection::vec(piece, 0..36).prop_map(|parts| parts.concat())
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
        let p2 = save_project_bytes(&doc).unwrap();
        prop_assert_eq!(&p1, &p2);
        prop_assert_eq!(open_project_bytes(&p1).unwrap(), doc.clone());

        let t1 = save_template_bytes(&doc).unwrap();
        let t2 = save_template_bytes(&doc).unwrap();
        prop_assert_eq!(&t1, &t2);
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

        // 1. Truncation (+ optional trailing garbage)
        let cut = ((base.len() as f64) * cut_ratio).floor() as usize;
        let cut = cut.min(base.len());
        let mut truncated = base[..cut].to_vec();
        if cut < base.len() {
            let trunc_res = catch_unwind(AssertUnwindSafe(|| {
                let _ = open_archive_bytes(&truncated);
                let _ = open_project_bytes(&truncated);
                let _ = open_template_bytes(&truncated);
                let _ = instantiate_template_bytes(&truncated, Some("Retry"));
            }));
            prop_assert!(trunc_res.is_ok(), "panic on truncated archive");
            prop_assert!(open_archive_bytes(&truncated).is_err());
        }
        truncated.extend_from_slice(&trailing);
        let trailing_res = catch_unwind(AssertUnwindSafe(|| open_archive_bytes(&truncated)));
        prop_assert!(trailing_res.is_ok(), "panic on truncated+trailing archive");

        // 2. Multi-byte XOR bit flips across ZIP stream
        let mut mutated = base;
        let len = mutated.len();
        for (pos, mask) in flips {
            mutated[pos % len] ^= mask;
        }
        let flip_res = catch_unwind(AssertUnwindSafe(|| (
            open_archive_bytes(&mutated),
            open_project_bytes(&mutated),
            open_template_bytes(&mutated),
            instantiate_template_bytes(&mutated, None),
        )));
        match flip_res {
            Err(_) => return Err(TestCaseError::fail("panic on bit-flipped archive")),
            Ok((Ok((_, parsed_doc)), _, _, _)) => {
                prop_assert!(validate_project_document(&parsed_doc).is_ok());
            }
            Ok((Err(_), _, _, _)) => {}
        }
    }

    #[test]
    fn unsafe_zip_paths_always_rejected_in_raw_archives(path in arb_unsafe_zip_path()) {
        prop_assert!(
            matches!(sanitize_zip_path(&path), Err(FormatError::UnsafePath { .. })),
            "expected UnsafePath from sanitize_zip_path for {path:?}"
        );
        let raw_zip = build_raw_zip(&[RawZipEntry::stored(path.as_bytes(), b"data")], b"");
        prop_assert!(
            matches!(ZipArchive::from_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })),
            "expected UnsafePath from ZipArchive::from_bytes for {path:?}"
        );
        prop_assert!(
            matches!(open_archive_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })),
            "expected UnsafePath from open_archive_bytes for {path:?}"
        );
        prop_assert!(
            matches!(import_prism_bytes(&raw_zip), Err(FormatError::UnsafePath { .. })),
            "expected UnsafePath from import_prism_bytes for {path:?}"
        );
    }

    #[test]
    fn pzfx_xml_structural_fuzz_never_panics(xml in arb_xml_fragment()) {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = parse_xml(&xml);
            import_pzfx_str(&xml)
        }));
        match outcome {
            Err(_) => return Err(TestCaseError::fail("panic in XML / .pzfx parser")),
            Ok(Ok(report)) => {
                prop_assert!(validate_project_document(&report.document).is_ok());
            }
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
        let meta = serde_json::json!({
            "title": "Fuzzed Sheet",
            "kind": "XY",
            "subcolumn_mode": "replicates",
            "replicates": replicates,
            "csv_path": "sheets/fuzzed.csv"
        });
        let meta_bytes = serde_json::to_vec(&meta).unwrap();
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
        let zip_bytes = build_raw_zip(&entries, b"");
        let outcome = catch_unwind(AssertUnwindSafe(|| import_prism_bytes(&zip_bytes)));
        match outcome {
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
        let mode = match kind {
            TableKind::Grouped | TableKind::Nested => SubcolumnMode::RawReplicates(2),
            _ => SubcolumnMode::Single,
        };
        let opts = CsvOptions {
            delimiter: delim,
            decimal_separator: decimal,
            header_rows,
            has_row_titles,
            ..CsvOptions::csv()
        };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = CsvOptions::detect(&text);
            let _ = parse_delimited_records(&text, &opts);
            let _ = parse_cell_token(&text, &opts);
            if let Ok(sheet) = import_csv_to_datasheet("fuzz-csv", "Fuzz", &text, kind, mode, &opts) {
                let _ = export_datasheet_to_csv(&sheet, &opts);
            }
            let filter = ImportFilterSpec {
                row_window: Some((0, 10)),
                col_window: Some((0, 8)),
                decimate_every_k: Some(1),
                unstack_group_column: None,
                parse_asterisk_exclusions: true,
            };
            if let Ok(filtered) = filter_delimited_text(&text, delim, decimal, header_rows > 0, &filter) {
                let _ = rearrange_and_baseline_grid(
                    &filtered.cells,
                    &ImportPlacementSpec { transpose: true, skip_contiguous_blanks: true, ..ImportPlacementSpec::default() },
                    ImportBaselineAdjustment::PercentOfFirstRow,
                );
            }
        }));
        prop_assert!(outcome.is_ok(), "panic on adversarial CSV/TSV input");
    }
}
