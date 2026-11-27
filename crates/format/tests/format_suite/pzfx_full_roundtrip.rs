//! Comprehensive tests for Wave 2 `.pzfx` full import (§4.1), deterministic data-only export &
//! preflight (§4.2), round-trip property verification (§4.3), and §6.3 feature-coverage warnings.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use proptest::prelude::*;

use curvebook_doc::{
    AnalysisKind, Cell, CellCoord, CellExclusionNote, ColumnDesignHint, ColumnTable, ConstantValue, ContingencyTable, DataSheet, FloatingNote,
    GraphSheet, GroupedTable, InfoConstant, InfoSheet, LayoutSheet, LayoutSpec, LevelOrderKind, LinkSource, LinkedDataRegion, MultipleVariablesTable,
    NamedCheckpoint, NestedGroupColumn, NestedTable, OneWayAnovaSpec, PartsOfWholeTable, PlateCellOrigin, ProjectDocument, RectPt, ReplicateStacking,
    SequenceProgression, SheetCellRange, SheetId, SheetOrigin, SubcolumnMode, SurvivalTable, TableArchetype, ThreeWayColumnGrouping, VariableRole,
    XColumnMode, XyTable,
};
use curvebook_format::{
    FormatError, ImportWarningKind, PZFX_FEATURE_CORPUS_COVERAGE, PZFX_ROOT_ELEMENT, PzfxDroppedKind, PzfxExportWarningKind,
    PzfxFeatureCoverageStatus, XmlWriter, compare_pzfx_data_layer, export_pzfx_bytes, export_pzfx_file, import_pzfx_bytes, import_pzfx_str,
    ingest_image_asset, lookup_pzfx_feature_coverage, parse_xml, preflight_pzfx_export, validate_project_document,
};
use curvebook_palette::Rgba8;

use super::common::{TempDirGuard, encode_rgba_png, result_sheet_with_kind};

fn wrap_vendor_root(attrs: &str, body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{PZFX_ROOT_ELEMENT} PrismXMLVersion=\"5.00\"{attrs}>{body}</{PZFX_ROOT_ELEMENT}>")
}

fn encode_utf16(s: &str, le: bool, bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2 + 2);
    if bom {
        out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    }
    for unit in s.encode_utf16() {
        out.extend_from_slice(&if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
    }
    out
}

#[test]
fn pzfx_import_detects_utf8_utf16_and_legacy_single_byte_encodings_and_rejects_unknown() {
    let body = "<TableSequence><Ref ID=\"t1\" Selected=\"1\"/></TableSequence>\
         <Table ID=\"t1\" TableType=\"OneWay\" XFormat=\"none\"><Title>Dose \u{00B5}M</Title>\
         <YColumn Width=\"81\" Decimals=\"2\" Subcolumns=\"1\"><Title>Caf\u{00E9}</Title><Subcolumn><d>4.5</d></Subcolumn></YColumn></Table>";
    let utf8_xml = wrap_vendor_root("", body);
    let utf16_xml =
        format!("<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<{PZFX_ROOT_ELEMENT} PrismXMLVersion=\"5.00\">{body}</{PZFX_ROOT_ELEMENT}>");

    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(utf8_xml.as_bytes());
    let utf16_le_bom = encode_utf16(&utf16_xml, true, true);
    let utf16_be_bom = encode_utf16(&utf16_xml, false, true);
    let utf16_le_nobom = encode_utf16(&utf16_xml, true, false);
    let utf16_be_nobom = encode_utf16(&utf16_xml, false, false);

    for (label, payload) in [
        ("utf8-bom", utf8_bom),
        ("utf16-le-bom", utf16_le_bom),
        ("utf16-be-bom", utf16_be_bom),
        ("utf16-le-nobom", utf16_le_nobom),
        ("utf16-be-nobom", utf16_be_nobom),
    ] {
        let rep = import_pzfx_bytes(&payload).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(rep.document.data_sheets[0].title, "Dose \u{00B5}M", "{label}");
        let TableArchetype::Column(ref col) = rep.document.data_sheets[0].archetype else { panic!("{label}: expected Column") };
        assert_eq!(col.data_sets[0].title, "Caf\u{00E9}", "{label}");
        assert_eq!(col.data_sets[0].cell(0, 0).unwrap().active_number(), Some(4.5), "{label}");
    }

    // Windows-1252 (0x80 = Euro, 0xB5 = micro)
    let mut cp1252 = b"<?xml version=\"1.0\" encoding=\"Windows-1252\"?>".to_vec();
    cp1252.extend_from_slice(
        format!("<{PZFX_ROOT_ELEMENT} PrismXMLVersion=\"5.00\"><Table ID=\"w1\" TableType=\"OneWay\" XFormat=\"none\"><Title>Cost ").as_bytes(),
    );
    cp1252.push(0x80); // €
    cp1252.extend_from_slice(b" / ");
    cp1252.push(0xB5); // µ
    cp1252.extend_from_slice(format!("M</Title><YColumn Width=\"81\" Decimals=\"1\" Subcolumns=\"1\"><Title>A</Title><Subcolumn><d>1</d></Subcolumn></YColumn></Table></{PZFX_ROOT_ELEMENT}>").as_bytes());
    let rep_1252 = import_pzfx_bytes(&cp1252).unwrap();
    assert_eq!(rep_1252.document.data_sheets[0].title, "Cost \u{20AC} / \u{00B5}M");

    // MacRoman (0xB5 = micro, 0x8E = é)
    let mut mac = b"<?xml version=\"1.0\" encoding=\"MacRoman\"?>".to_vec();
    mac.extend_from_slice(
        format!("<{PZFX_ROOT_ELEMENT} PrismXMLVersion=\"5.00\"><Table ID=\"m1\" TableType=\"OneWay\" XFormat=\"none\"><Title>Caf").as_bytes(),
    );
    mac.push(0x8E); // é in MacRoman
    mac.extend_from_slice(format!("</Title><YColumn Width=\"81\" Decimals=\"1\" Subcolumns=\"1\"><Title>A</Title><Subcolumn><d>2</d></Subcolumn></YColumn></Table></{PZFX_ROOT_ELEMENT}>").as_bytes());
    let rep_mac = import_pzfx_bytes(&mac).unwrap();
    assert_eq!(rep_mac.document.data_sheets[0].title, "Caf\u{00E9}");

    // Unknown encoding returns typed UnsupportedFormat
    let unknown = format!(
        "<?xml version=\"1.0\" encoding=\"KOI8-R\"?><{PZFX_ROOT_ELEMENT} PrismXMLVersion=\"5.00\"><Table ID=\"u1\" TableType=\"OneWay\"/></{PZFX_ROOT_ELEMENT}>"
    );
    assert!(matches!(import_pzfx_bytes(unknown.as_bytes()), Err(FormatError::UnsupportedFormat { .. })));
}

#[test]
fn pzfx_import_unwraps_xslt_namespaces_skips_opaque_template_and_reports_trailing_bytes() {
    let opaque_payload = "A".repeat(256 * 1024);
    let xslt_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xsl:stylesheet version=\"1.0\" xmlns:xsl=\"http://www.w3.org/1999/XSL/Transform\">\n\
         <{PZFX_ROOT_ELEMENT} xmlns=\"http://example.invalid/ns\" PrismXMLVersion=\"5.00\">\n\
           <TableSequence Selected=\"1\"><Ref ID=\"t-second\" Selected=\"1\"/><Ref ID=\"t-first\"/></TableSequence>\n\
           <Table ID=\"t-first\" TableType=\"OneWay\" XFormat=\"none\"><Title>First in XML</Title>\
             <YColumn Width=\"81\" Decimals=\"2\" Subcolumns=\"1\"><Title>A</Title><Subcolumn><d>1</d></Subcolumn></YColumn></Table>\n\
           <Table ID=\"t-second\" TableType=\"OneWay\" XFormat=\"none\"><Title>Second in XML</Title>\
             <YColumn Width=\"81\" Decimals=\"2\" Subcolumns=\"1\"><Title>B</Title><Subcolumn><d>2</d></Subcolumn></YColumn></Table>\n\
           <Template dt=\"bin\">{opaque_payload}</Template>\n\
         </{PZFX_ROOT_ELEMENT}>\n\
         </xsl:stylesheet>"
    );
    let mut payload = xslt_xml.into_bytes();
    payload.extend_from_slice(b"\x00\xFFtrailing-binary-garbage");

    let report = import_pzfx_bytes(&payload).unwrap();
    let doc = &report.document;
    assert_eq!(doc.data_sheets.len(), 2);
    // Sequence order respected: t-second before t-first, and t-second is active + family anchor.
    assert_eq!(doc.data_sheets[0].id.as_str(), "t-second");
    assert_eq!(doc.data_sheets[1].id.as_str(), "t-first");
    assert_eq!(doc.active_sheet_id, Some(SheetId::new("t-second")));
    assert_eq!(doc.family_anchor_sheet_id, Some(SheetId::new("t-second")));

    // Exactly ONE IgnoredSection warning carrying both opaque Template byte count and trailing byte count.
    let ignored: Vec<_> = report.warnings.iter().filter(|w| w.kind == ImportWarningKind::IgnoredSection).collect();
    assert_eq!(ignored.len(), 1, "expected single IgnoredSection warning, got {:?}", report.warnings);
    assert!(ignored[0].message.contains("262144"), "missing opaque byte count: {}", ignored[0].message);
    assert!(ignored[0].message.contains("25"), "missing trailing byte count: {}", ignored[0].message);
}

#[test]
fn pzfx_import_handles_hugetable_table1024_ext_table_types_mvtype_xformat_time_subcol_titles_per_table_info_and_colors() {
    let xml = wrap_vendor_root(
        "",
        r##"
        <InfoSequence Selected="1">
          <Ref ID="info-global"/>
          <Ref ID="info-embedded" Selected="1"/>
        </InfoSequence>
        <Info ID="info-global">
          <Title>Global Protocol</Title>
          <Notes>Use <Font Face="Symbol">m</Font>M stock<BR/>Line 2</Notes>
          <Constant><Name>Experiment Date</Name><Value>2026-04-15</Value></Constant>
          <Constant><Name>NonIsoDate</Name><Value>04/15/2026</Value></Constant>
        </Info>
        <TableSequence>
          <Ref ID="t-nested"/>
          <Ref ID="t-mv"/>
          <Ref ID="t-time"/>
        </TableSequence>
        <HugeTable ID="t-nested" TableType="TwoWay" ExtTableType="Nested" XFormat="none" YFormat="replicates" Replicates="2" Highlighted="4" EVFormat="AsteriskAfterNumber">
          <Title><b>Nested</b> <Font Face="Symbol">a</Font></Title>
          <Info ID="info-embedded">
            <Title>Nested Table Notes</Title>
            <Notes>Per-table metadata</Notes>
            <Constant><Name>Assay Date</Name><Value>2026-05-01</Value></Constant>
          </Info>
          <FloatingNote ID="fn-1" Color="Blue" Left="24" Top="40" Width="180" Height="90"><Font>Sticky<BR/>line two</Font></FloatingNote>
          <SubColumnTitles OwnSet="1">
            <Subcolumn><d>Mouse A</d></Subcolumn>
            <Subcolumn><d>Mouse B</d></Subcolumn>
            <Subcolumn><d>Mouse C</d></Subcolumn>
            <Subcolumn><d>Mouse D</d></Subcolumn>
          </SubColumnTitles>
          <YColumn Width="81" Decimals="3" Subcolumns="2"><Title>Ctrl</Title><Subcolumn><d>1.1</d></Subcolumn><Subcolumn><d>1.2</d></Subcolumn></YColumn>
          <YColumn Width="81" Decimals="3" Subcolumns="2"><Title>Drug</Title><Subcolumn><d>2.1</d></Subcolumn><Subcolumn><d>2.2</d></Subcolumn></YColumn>
        </HugeTable>
        <Table1024 ID="t-mv" TableType="OneWay" ExtTableType="MultipleVariables" XFormat="none" EVFormat="AsteriskAfterNumber">
          <Title>MultiVar 1024</Title>
          <YColumn Width="81" Decimals="0" Subcolumns="1" MVType="Categorical"><Title>GroupCode</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></YColumn>
          <YColumn Width="81" Decimals="4" Subcolumns="1" MVType="Continues"><Title>Biomarker</Title><Subcolumn><d>12.5</d><d>18.25</d></Subcolumn></YColumn>
        </Table1024>
        <Table ID="t-time" TableType="XY" XFormat="time" YFormat="SD" EVFormat="AsteriskAfterNumber">
          <Title>Time Series</Title>
          <SubColumnTitles OwnSet="0">
            <Subcolumn><d>Avg</d></Subcolumn>
            <Subcolumn><d>StdDev</d></Subcolumn>
          </SubColumnTitles>
          <XColumn Width="81" Decimals="1" Subcolumns="1"><Title>Elapsed (h)</Title><Subcolumn><d>0</d><d>1.5</d></Subcolumn></XColumn>
          <XAdvancedColumn Width="81" Decimals="1" Subcolumns="1" Version="1"><Title>Elapsed (h)</Title><Subcolumn><d>00:00:00</d><d>01:30:00</d></Subcolumn></XAdvancedColumn>
          <YColumn Width="81" Decimals="2" Subcolumns="2"><Title>Signal</Title><Subcolumn><d>10</d><d>25</d></Subcolumn><Subcolumn><d>0.5</d><d>1.2</d></Subcolumn></YColumn>
        </Table>
        "##,
    );

    let report = import_pzfx_str(&xml).unwrap();
    let doc = &report.document;
    assert!(report.warnings.is_empty(), "unexpected warnings: {:?}", report.warnings);

    // InfoSequence Selected="1" + Ref Selected="1" selects info-embedded.
    assert_eq!(doc.info_sheets.len(), 2);
    assert_eq!(doc.info_sheets[0].id.as_str(), "info-global");
    assert_eq!(doc.info_sheets[1].id.as_str(), "info-embedded");
    assert_eq!(doc.active_sheet_id, Some(SheetId::new("info-embedded")));
    assert_eq!(doc.family_anchor_sheet_id, Some(SheetId::new("info-embedded")));

    // Strict ISO-8601 Date vs non-ISO Text + Symbol-font note decoding.
    assert_eq!(doc.info_sheets[0].get_constant("Experiment Date").unwrap().value, ConstantValue::Date(String::from("2026-04-15")));
    assert_eq!(doc.info_sheets[0].get_constant("NonIsoDate").unwrap().value, ConstantValue::Text(String::from("04/15/2026")));
    assert_eq!(doc.info_sheets[0].notes_markdown, "Use μM stock\nLine 2");

    // HugeTable TwoWay+ExtTableType=Nested + embedded per-table Info + FloatingNote color + Highlighted tag + OwnSet=1 subcolumn titles.
    let nested_ds = doc.data_sheet(&SheetId::new("t-nested")).unwrap();
    assert_eq!(nested_ds.title, "<b>Nested</b> α");
    assert_eq!(nested_ds.info_sheet_id, Some(SheetId::new("info-embedded")));
    assert_eq!(doc.info_sheets[1].linked_data_sheets, vec![SheetId::new("t-nested")]);
    assert!(nested_ds.color_tag.is_some());
    assert_eq!(nested_ds.floating_notes.len(), 1);
    assert_eq!(nested_ds.floating_notes[0].fill_color, Rgba8::rgb(0x00, 0x00, 0xFF));
    assert_eq!(nested_ds.floating_notes[0].body_markdown, "Sticky\nline two");
    let TableArchetype::Nested(ref nested) = nested_ds.archetype else { panic!("expected Nested") };
    assert_eq!(nested.groups[0].subcolumn_titles, vec!["Mouse A", "Mouse B"]);
    assert_eq!(nested.groups[1].subcolumn_titles, vec!["Mouse C", "Mouse D"]);

    // Table1024 OneWay+ExtTableType=MultipleVariables + MVType="Categorical"|"Continues" + display_decimals.
    let mv_ds = doc.data_sheet(&SheetId::new("t-mv")).unwrap();
    let TableArchetype::MultipleVariables(ref mv) = mv_ds.archetype else { panic!("expected MultipleVariables") };
    assert_eq!((mv.variables[0].role, mv.variables[0].role_inferred, mv.variables[0].display_decimals), (VariableRole::Categorical, false, Some(0)));
    assert_eq!((mv.variables[1].role, mv.variables[1].role_inferred, mv.variables[1].display_decimals), (VariableRole::Continuous, false, Some(4)));

    // XY XFormat="time" prefers numeric <XColumn>, preserves x_display_decimals + OwnSet=0 subcolumn titles + native MeanSd.
    let time_ds = doc.data_sheet(&SheetId::new("t-time")).unwrap();
    let TableArchetype::Xy(ref xy) = time_ds.archetype else { panic!("expected XY") };
    assert_eq!(xy.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Elapsed (h)") });
    assert_eq!(xy.x_display_decimals, Some(1));
    assert_eq!((xy.x_cells[0][0].active_number(), xy.x_cells[1][0].active_number()), (Some(0.0), Some(1.5)));
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::MeanSd);
    assert_eq!(xy.data_sets[0].display_decimals, Some(2));
    assert_eq!(xy.data_sets[0].subcolumn_titles, vec!["Avg", "StdDev"]);
}

#[test]
fn pzfx_export_preflight_and_report_list_every_dropped_and_unrepresentable_category() {
    let mut doc = ProjectDocument::new("Full Project").unwrap();

    // 1. XY with GeneratedSequence, x_subcolumn_titles, linked_regions, frozen, exclusion_notes, plate_well_origins, origin, minimized note
    let mut xy = XyTable::new_with_x_mode(
        XColumnMode::GeneratedSequence { start: 1.0, step: 2.0, progression: SequenceProgression::Geometric },
        SubcolumnMode::Single,
        1,
        2,
    )
    .unwrap();
    xy.data_sets[0].cells[0][0] = Cell::excluded_number(10.0);
    xy.data_sets[0].cells[1][0] = Cell::number(20.0);
    let mut xy_sheet = DataSheet::new("d-xy", "Dose Curve", TableArchetype::Xy(xy)).unwrap();
    xy_sheet.frozen = true;
    xy_sheet.origin = Some(SheetOrigin {
        source_label: String::from("donor.curvebook"),
        source_format: String::from("curvebook"),
        merged_at: String::from("unix:1700000000"),
    });
    xy_sheet.exclusion_notes.push(CellExclusionNote {
        coord: CellCoord::new(0, 0, 0),
        reason_tag: String::from("outlier"),
        note: String::from("pipette bubble"),
        timestamp_utc: String::from("unix:1700000000"),
    });
    xy_sheet.plate_well_origins.push(PlateCellOrigin { coord: CellCoord::new(0, 0, 0), well: String::from("A01") });
    xy_sheet.linked_regions.push(LinkedDataRegion {
        id: String::from("link-1"),
        spec: curvebook_doc::ImportLinkSpec {
            source: LinkSource::ExternalDelimitedFile { path: String::from("raw.csv"), delimiter: ',', decimal_separator: '.' },
            filter: curvebook_doc::ImportFilterSpec::default(),
            placement: curvebook_doc::ImportPlacementSpec::default(),
        },
        target_range: SheetCellRange::new(CellCoord::new(0, 0, 0), CellCoord::new(1, 0, 0)),
        provenance_summary: String::from("raw.csv"),
    });
    let mut min_note = FloatingNote::new("n1", RectPt::new(10.0, 10.0, 120.0, 40.0), "Hidden note");
    min_note.minimized = true;
    xy_sheet.floating_notes.push(min_note);
    doc.add_data_sheet(xy_sheet).unwrap();

    // 2. MultipleVariables with Calculated formula + Label role + custom level order
    let mut mv = MultipleVariablesTable::new(3, 2).unwrap();
    mv.variables[0].title = String::from("ID");
    mv.variables[0].lock_role(VariableRole::Label);
    mv.variables[0].cells = vec![Cell::text("S1"), Cell::text("S2")];
    mv.variables[1].title = String::from("Arm");
    mv.variables[1].lock_role(VariableRole::Categorical);
    mv.variables[1].level_order.order = LevelOrderKind::Alphabetical;
    mv.variables[1].cells = vec![Cell::text("Ctrl"), Cell::text("Drug")];
    mv.variables[2].title = String::from("LogConc");
    mv.variables[2].set_calculated_formula("log10([Dose])");
    mv.variables[2].cells = vec![Cell::number(0.0), Cell::number(1.0)];
    doc.add_data_sheet(DataSheet::new("d-mv", "MV Table", TableArchetype::MultipleVariables(mv)).unwrap()).unwrap();

    // 3. Grouped with Stacked + row_numeric_spacing + 3-way grouping + custom factor titles
    let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 4, 2).unwrap();
    grp.replicate_stacking = ReplicateStacking::Stacked;
    grp.row_numeric_spacing = Some(vec![Some(1.0), Some(4.0)]);
    grp.row_factor_title = String::from("Genotype");
    grp.column_factor_title = String::from("Drug");
    grp.three_way_grouping = Some(ThreeWayColumnGrouping {
        factor_1_name: String::from("Sex"),
        factor_1_levels: [String::from("F"), String::from("M")],
        factor_2_name: String::from("Diet"),
        factor_2_levels: [String::from("Std"), String::from("HFD")],
    });
    doc.add_data_sheet(DataSheet::new("d-grp", "Factorial", TableArchetype::Grouped(grp)).unwrap()).unwrap();

    // 4. Survival with custom event/censored codes + Column PairedRepeatedMeasures
    let mut surv = SurvivalTable::new(1, 2).unwrap();
    surv.event_code = 2;
    surv.censored_code = 9;
    doc.add_data_sheet(DataSheet::new("d-surv", "Surv", TableArchetype::Survival(surv)).unwrap()).unwrap();
    let col = ColumnTable::new(ColumnDesignHint::PairedRepeatedMeasures, 2, 2).unwrap();
    doc.add_data_sheet(DataSheet::new("d-col", "Paired", TableArchetype::Column(col)).unwrap()).unwrap();

    // 5. InfoSheet with units, floating note, color tag, origin
    let mut info = InfoSheet::new_preseeded("info-1", "Protocol").unwrap();
    info.constants = vec![InfoConstant::new("Temp", ConstantValue::Number(37.0), Some(String::from("C"))).unwrap()];
    info.floating_notes.push(FloatingNote::new("in-1", RectPt::new(0.0, 0.0, 100.0, 40.0), "Info note"));
    info.color_tag = Some(Rgba8::rgb(0xFF, 0x00, 0x00));
    doc.add_info_sheet(info).unwrap();

    // 6. Analyses, Graphs, Layouts, Checkpoints, Image Assets
    doc.add_result_sheet(result_sheet_with_kind("res-1", "d-col", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))).unwrap();
    let mut graph_sheet = GraphSheet::new_uninitialized_for_data("g-1", &doc.data_sheets[0]).unwrap();
    graph_sheet.title = String::from("Main Figure");
    doc.add_graph_sheet(graph_sheet).unwrap();
    doc.add_layout_sheet(LayoutSheet::new("l-1", "Panel Layout", LayoutSpec::default()).unwrap()).unwrap();
    doc.checkpoints.push(NamedCheckpoint {
        id: String::from("cp-1"),
        name: String::from("Baseline"),
        note: String::new(),
        created_utc: String::from("unix:1700000000"),
        snapshot_json: String::from("{}"),
    });
    let png_bytes = encode_rgba_png(2, 2);
    doc.upsert_image_asset(ingest_image_asset(&png_bytes).unwrap()).unwrap();

    let preflight = preflight_pzfx_export(&doc);
    assert!(preflight.has_dropped_items());
    assert_eq!(preflight.data_sheet_count, 5);
    assert_eq!(preflight.info_sheet_count, 1);

    for kind in [
        PzfxDroppedKind::AnalysesAndResults,
        PzfxDroppedKind::Graphs,
        PzfxDroppedKind::Layouts,
        PzfxDroppedKind::LiveLinks,
        PzfxDroppedKind::Images,
        PzfxDroppedKind::Checkpoints,
        PzfxDroppedKind::FormulasWrittenAsValues,
        PzfxDroppedKind::UnrepresentableTableFeatures,
    ] {
        let g = preflight.group(kind).unwrap_or_else(|| panic!("missing dropped group {kind:?}"));
        assert!(g.count >= 1 && !g.items.is_empty(), "{kind:?} should have non-empty items");
    }

    let report = export_pzfx_bytes(&doc).unwrap();
    assert_eq!(report.preflight, preflight, "export report preflight must match pure preflight identically");
    let dropped_warnings: Vec<_> = report.warnings.iter().filter(|w| w.kind == PzfxExportWarningKind::DroppedContent).collect();
    assert_eq!(dropped_warnings.len(), preflight.dropped.len());

    // GeneratedSequence was materialized into explicit numeric X values (1.0, 2.0) and re-imports cleanly.
    let re = import_pzfx_bytes(&report.bytes).unwrap();
    let TableArchetype::Xy(ref re_xy) = re.document.data_sheet(&SheetId::new("d-xy")).unwrap().archetype else {
        panic!("expected XY");
    };
    assert_eq!((re_xy.x_cells[0][0].active_number(), re_xy.x_cells[1][0].active_number()), (Some(1.0), Some(2.0)));
    assert_eq!(re_xy.data_sets[0].cell(0, 0), Some(&Cell::excluded_number(10.0)));
}

#[test]
fn pzfx_export_emits_insufficient_coverage_warnings_for_unverified_features_and_promotes_large_tables() {
    let mut doc = ProjectDocument::new("Coverage And Scale").unwrap();

    // 1. MeanLowHigh (`low-high`, 2/2) + MeanCvN (`CVN`, 0/0) + MedianIqr (`median-iqr`, 0/0) + XEVFormat (`0/0`)
    let mut xy_cov =
        XyTable::new_with_x_mode(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSem }, SubcolumnMode::MeanLowHigh, 1, 1).unwrap();
    xy_cov.x_cells[0][0] = Cell::number(1.0);
    xy_cov.x_cells[0][1] = Cell::number(0.1);
    xy_cov.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy_cov.data_sets[0].cells[0][1] = Cell::number(1.0);
    xy_cov.data_sets[0].cells[0][2] = Cell::number(2.0);
    doc.add_data_sheet(DataSheet::new("t-lh", "LowHigh", TableArchetype::Xy(xy_cov)).unwrap()).unwrap();

    let mut grp_iqr = GroupedTable::new(SubcolumnMode::MedianIqr, 1, 1).unwrap();
    grp_iqr.data_sets[0].cells[0] = vec![Cell::number(5.0), Cell::number(3.0), Cell::number(7.0)];
    doc.add_data_sheet(DataSheet::new("t-iqr", "MedianIQR", TableArchetype::Grouped(grp_iqr)).unwrap()).unwrap();

    // 2. HugeTable (> 104 Y columns) & Table1024 (> 256 Y columns)
    let huge_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 110, 1).unwrap();
    doc.add_data_sheet(DataSheet::new("t-huge", "Wide110", TableArchetype::Column(huge_col)).unwrap()).unwrap();
    let table_1024 = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 260, 1).unwrap();
    doc.add_data_sheet(DataSheet::new("t-1024", "Wide260", TableArchetype::Column(table_1024)).unwrap()).unwrap();

    let report = export_pzfx_bytes(&doc).unwrap();
    let cov_warnings: Vec<_> = report.warnings.iter().filter(|w| w.kind == PzfxExportWarningKind::InsufficientCorpusCoverage).collect();
    assert!(cov_warnings.iter().any(|w| w.source == "t-lh" && w.message.contains("low-high")), "missing low-high coverage warning: {cov_warnings:?}");
    assert!(
        cov_warnings.iter().any(|w| w.source == "t-lh" && w.message.contains("XEVFormat")),
        "missing XEVFormat coverage warning: {cov_warnings:?}"
    );
    assert!(
        cov_warnings.iter().any(|w| w.source == "t-iqr" && w.message.contains("median-iqr")),
        "missing median-iqr coverage warning: {cov_warnings:?}"
    );

    // Verify exact root-child ordering and promoted tag names in output XML.
    let xml = std::str::from_utf8(&report.bytes).unwrap();
    let root = parse_xml(xml).unwrap();
    assert_eq!(root.local_name(), PZFX_ROOT_ELEMENT);
    let child_names: Vec<&str> = root.child_elements().map(|e| e.local_name()).collect();
    assert_eq!(child_names, vec!["Created", "InfoSequence", "TableSequence", "Table", "Table", "HugeTable", "Table1024"]);

    // Verify static coverage table invariants & lookup helper.
    assert_eq!(
        lookup_pzfx_feature_coverage("y_format:low-high|Subcolumns=<missing>|Replicates=<missing>").unwrap().status,
        PzfxFeatureCoverageStatus::InsufficientCoverage
    );
    assert_eq!(
        lookup_pzfx_feature_coverage("y_format:SD|Subcolumns=<missing>|Replicates=<missing>").unwrap().status,
        PzfxFeatureCoverageStatus::Conformant
    );
    assert!(PZFX_FEATURE_CORPUS_COVERAGE.len() >= 40);

    // Round-trip via atomic file write matches data layer identically.
    let dir = TempDirGuard::new("pzfx-export").unwrap();
    let out_path = dir.child("study.pzfx");
    let file_rep = export_pzfx_file(&out_path, &doc).unwrap();
    assert_eq!(file_rep.bytes, report.bytes);
    let on_disk = std::fs::read(&out_path).unwrap();
    assert_eq!(on_disk, report.bytes);

    let re = import_pzfx_bytes(&on_disk).unwrap();
    let diff = compare_pzfx_data_layer(&doc, &re.document);
    assert!(diff.is_identical(), "round-trip diff: {diff:?}");
}

#[test]
fn pzfx_export_and_import_roundtrip_per_table_embedded_info_and_compare_detects_diffs() {
    let mut doc = ProjectDocument::new("Per-Table Info").unwrap();
    let mut info_global = InfoSheet::new_preseeded("info-global", "Shared Info").unwrap();
    info_global.constants = vec![
        InfoConstant::new("Experiment Date", ConstantValue::Date(String::from("2026-06-01")), None).unwrap(),
        InfoConstant::new("Dose", ConstantValue::Number(12.5), None).unwrap(),
    ];
    info_global.notes_markdown = String::from("Shared protocol note");

    let mut info_t2 = InfoSheet::new_preseeded("info-t2", "Table 2 Info").unwrap();
    info_t2.constants = vec![InfoConstant::new("Operator", ConstantValue::Text(String::from("Analyst A")), None).unwrap()];
    info_t2.notes_markdown = String::from("Dedicated table 2 note");

    doc.add_info_sheet(info_global).unwrap();
    doc.add_info_sheet(info_t2).unwrap();

    let mut ds1 =
        DataSheet::new("data-1", "Table 1", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    ds1.info_sheet_id = Some(SheetId::new("info-global"));
    let mut ds2 =
        DataSheet::new("data-2", "Table 2", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 1, 2).unwrap())).unwrap();
    ds2.info_sheet_id = Some(SheetId::new("info-t2"));
    doc.add_data_sheet(ds1).unwrap();
    doc.add_data_sheet(ds2).unwrap();
    doc.select_in_navigator(&SheetId::new("data-2")).unwrap();

    let rep = export_pzfx_bytes(&doc).unwrap();
    assert!(rep.warnings.is_empty(), "unexpected export warnings: {:?}", rep.warnings);
    let re = import_pzfx_bytes(&rep.bytes).unwrap();
    assert!(re.warnings.is_empty(), "unexpected import warnings: {:?}", re.warnings);
    assert!(compare_pzfx_data_layer(&doc, &re.document).is_identical());
    assert_eq!(re.document.data_sheet(&SheetId::new("data-2")).unwrap().info_sheet_id, Some(SheetId::new("info-t2")));
    assert_eq!(re.document.active_sheet_id, Some(SheetId::new("data-2")));

    // Mutate one cell and verify compare_pzfx_data_layer flags the exact difference.
    let mut mutated = re.document.clone();
    if let TableArchetype::Column(ref mut col) = mutated.data_sheets[0].archetype {
        col.data_sets[0].cells[0][0] = Cell::excluded_number(99.0);
    }
    let diff = compare_pzfx_data_layer(&doc, &mutated);
    assert!(!diff.is_identical());
    assert!(diff.differences.iter().any(|d| d.location.contains("data[0:data-1]")));

    // Verify XmlWriter helper stays clean with bare constant (no `<` prefix stored in constant).
    assert!(!PZFX_ROOT_ELEMENT.starts_with('<'));
    let mut w = XmlWriter::new();
    w.empty_element(PZFX_ROOT_ELEMENT, &[("PrismXMLVersion", "5.00")]).unwrap();
    assert!(w.finish().unwrap().contains(PZFX_ROOT_ELEMENT));
}

// ---------------------------------------------------------------------------
// §4.3 Proptest Round-Trip & Preflight Completeness Property Tests
// ---------------------------------------------------------------------------

fn arb_numeric_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        2 => Just(Cell::empty()),
        5 => (-10000_i32..=10000_i32).prop_map(|n| Cell::number(f64::from(n) / 100.0)),
        2 => (-10000_i32..=10000_i32).prop_map(|n| Cell::excluded_number(f64::from(n) / 100.0)),
    ]
}

fn arb_nonneg_int_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        1 => Just(Cell::empty()),
        6 => (0_u16..=500_u16).prop_map(|n| Cell::number(f64::from(n))),
        1 => (0_u16..=500_u16).prop_map(|n| Cell::excluded_number(f64::from(n))),
    ]
}

fn arb_survival_status_cell() -> impl Strategy<Value = Cell> {
    prop_oneof![
        1 => Just(Cell::empty()),
        4 => Just(Cell::number(0.0)),
        4 => Just(Cell::number(1.0)),
        1 => Just(Cell::excluded_number(1.0)),
    ]
}

fn arb_title() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::from("Control")),
        Just(String::from("Dose 10<sup>-6</sup> M")),
        Just(String::from("<b>IC</b><sub>50</sub> (\u{03BC}M)")),
        Just(String::from("<i>In vitro</i> response")),
        "[A-Za-z][A-Za-z0-9 ]{0,12}".prop_map(|s| s.trim().to_owned()),
    ]
}

fn arb_representable_table(idx: usize) -> impl Strategy<Value = DataSheet> {
    let id = format!("tbl-{idx}");
    (0_u8..8_u8, arb_title(), 1_usize..=4_usize, 1_usize..=3_usize, any::<bool>()).prop_flat_map(
        move |(kind_tag, title, rows, cols, has_row_titles)| {
            let id = id.clone();
            let row_titles_strat = prop::collection::vec("[A-Za-z][A-Za-z0-9]{0,6}", rows);
            match kind_tag {
                0 => {
                    // XY across Single / RawReplicates(2) / MeanSd / MeanSem / MeanSdN / MeanSemN / MeanCvN / MeanLowHigh / UpperLowerLimits / MedianIqr
                    let y_mode_strat = prop_oneof![
                        Just(SubcolumnMode::Single),
                        Just(SubcolumnMode::RawReplicates(2)),
                        Just(SubcolumnMode::MeanSd),
                        Just(SubcolumnMode::MeanSem),
                        Just(SubcolumnMode::MeanSdN),
                        Just(SubcolumnMode::MeanSemN),
                        Just(SubcolumnMode::MeanCvN),
                        Just(SubcolumnMode::MeanLowHigh),
                        Just(SubcolumnMode::UpperLowerLimits),
                        Just(SubcolumnMode::MedianIqr),
                    ];
                    let x_mode_strat = prop_oneof![
                        Just(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single }),
                        Just(XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::MeanSd }),
                        Just(XColumnMode::DateOrElapsedTime { unit_label: String::from("Time") }),
                    ];
                    (y_mode_strat, x_mode_strat, row_titles_strat)
                        .prop_flat_map(move |(y_mode, x_mode, rt)| {
                            let x_subs = x_mode.subcolumn_count();
                            let y_subs = y_mode.subcolumn_count();
                            (
                                Just((y_mode, x_mode, rt)),
                                prop::collection::vec(prop::collection::vec(arb_numeric_cell(), x_subs), rows),
                                prop::collection::vec(prop::collection::vec(prop::collection::vec(arb_numeric_cell(), y_subs), rows), cols),
                                prop::option::of(0_u8..=6_u8),
                            )
                        })
                        .prop_map(move |((y_mode, x_mode, rt), x_cells, y_grid, dec)| {
                            let mut xy = XyTable::new_with_x_mode(x_mode.clone(), y_mode, cols, rows).unwrap();
                            xy.x_title = match &x_mode {
                                XColumnMode::DateOrElapsedTime { unit_label } => unit_label.clone(),
                                _ => String::from("X"),
                            };
                            xy.x_display_decimals = dec;
                            xy.x_cells = x_cells;
                            if has_row_titles {
                                xy.row_titles = rt;
                                xy.row_titles_expanded = true;
                            }
                            for (c, col_rows) in y_grid.into_iter().enumerate() {
                                xy.data_sets[c].cells = col_rows;
                                xy.data_sets[c].display_decimals = dec;
                            }
                            DataSheet::new(id.clone(), title.clone(), TableArchetype::Xy(xy)).unwrap()
                        })
                        .boxed()
                }
                1 => {
                    // Column across Single / MeanSd / MeanSem / MeanSdN / MeanSemN / MeanCvN
                    let y_mode_strat = prop_oneof![
                        Just(SubcolumnMode::Single),
                        Just(SubcolumnMode::MeanSd),
                        Just(SubcolumnMode::MeanSem),
                        Just(SubcolumnMode::MeanSdN),
                        Just(SubcolumnMode::MeanSemN),
                        Just(SubcolumnMode::MeanCvN),
                    ];
                    (y_mode_strat, row_titles_strat)
                        .prop_flat_map(move |(y_mode, rt)| {
                            let y_subs = y_mode.subcolumn_count();
                            (
                                Just((y_mode, rt)),
                                prop::collection::vec(prop::collection::vec(prop::collection::vec(arb_numeric_cell(), y_subs), rows), cols),
                            )
                        })
                        .prop_map(move |((y_mode, rt), y_grid)| {
                            let hint =
                                if y_mode.is_pre_summarized() { ColumnDesignHint::PrecomputedError } else { ColumnDesignHint::UnpairedReplicates };
                            let mut col = ColumnTable::new_with_subcolumns(hint, y_mode, cols, rows).unwrap();
                            if has_row_titles {
                                col.row_titles = rt;
                                col.row_titles_expanded = true;
                            }
                            for (c, col_rows) in y_grid.into_iter().enumerate() {
                                col.data_sets[c].cells = col_rows;
                            }
                            DataSheet::new(id.clone(), title.clone(), TableArchetype::Column(col)).unwrap()
                        })
                        .boxed()
                }
                2 => {
                    // Grouped across RawReplicates(2) / MeanSd / MeanSem / MeanSdN / UpperLowerLimits / MeanLowHigh / MedianIqr
                    let y_mode_strat = prop_oneof![
                        Just(SubcolumnMode::RawReplicates(2)),
                        Just(SubcolumnMode::MeanSd),
                        Just(SubcolumnMode::MeanSem),
                        Just(SubcolumnMode::MeanSdN),
                        Just(SubcolumnMode::UpperLowerLimits),
                        Just(SubcolumnMode::MeanLowHigh),
                        Just(SubcolumnMode::MedianIqr),
                    ];
                    (y_mode_strat, row_titles_strat)
                        .prop_flat_map(move |(y_mode, rt)| {
                            let y_subs = y_mode.subcolumn_count();
                            (
                                Just((y_mode, rt)),
                                prop::collection::vec(prop::collection::vec(prop::collection::vec(arb_numeric_cell(), y_subs), rows), cols),
                            )
                        })
                        .prop_map(move |((y_mode, rt), y_grid)| {
                            let mut grp = GroupedTable::new(y_mode, cols, rows).unwrap();
                            if has_row_titles {
                                grp.row_titles = rt;
                            }
                            for (c, col_rows) in y_grid.into_iter().enumerate() {
                                grp.data_sets[c].cells = col_rows;
                            }
                            DataSheet::new(id.clone(), title.clone(), TableArchetype::Grouped(grp)).unwrap()
                        })
                        .boxed()
                }
                3 => (row_titles_strat, prop::collection::vec(prop::collection::vec(arb_nonneg_int_cell(), rows), cols))
                    .prop_map(move |(rt, y_grid)| {
                        let mut cont = ContingencyTable::new(cols, rows).unwrap();
                        if has_row_titles {
                            cont.row_titles = rt;
                        }
                        for (c, col_rows) in y_grid.into_iter().enumerate() {
                            for (r, cell) in col_rows.into_iter().enumerate() {
                                cont.data_sets[c].cells[r][0] = cell;
                            }
                        }
                        DataSheet::new(id.clone(), title.clone(), TableArchetype::Contingency(cont)).unwrap()
                    })
                    .boxed(),
                4 => (
                    row_titles_strat,
                    prop::collection::vec(arb_nonneg_int_cell(), rows),
                    prop::collection::vec(prop::collection::vec(arb_survival_status_cell(), rows), cols),
                )
                    .prop_map(move |(rt, times, cohorts)| {
                        let mut surv = SurvivalTable::new(cols, rows).unwrap();
                        surv.times = times;
                        if has_row_titles {
                            surv.row_titles = rt;
                            surv.row_titles_expanded = true;
                        }
                        for (c, col_rows) in cohorts.into_iter().enumerate() {
                            for (r, cell) in col_rows.into_iter().enumerate() {
                                surv.cohorts[c].cells[r][0] = cell;
                            }
                        }
                        DataSheet::new(id.clone(), title.clone(), TableArchetype::Survival(surv)).unwrap()
                    })
                    .boxed(),
                5 => (row_titles_strat, prop::collection::vec(prop::collection::vec(arb_nonneg_int_cell(), rows), cols))
                    .prop_map(move |(rt, y_grid)| {
                        let mut pow = PartsOfWholeTable::new(cols, rows).unwrap();
                        if has_row_titles {
                            pow.slice_labels = rt;
                        }
                        for (c, col_rows) in y_grid.into_iter().enumerate() {
                            for (r, cell) in col_rows.into_iter().enumerate() {
                                pow.data_sets[c].cells[r][0] = cell;
                            }
                        }
                        DataSheet::new(id.clone(), title.clone(), TableArchetype::PartsOfWhole(pow)).unwrap()
                    })
                    .boxed(),
                6 => (row_titles_strat, prop::collection::vec(prop::collection::vec(arb_numeric_cell(), rows), cols))
                    .prop_map(move |(rt, grid)| {
                        let mut mv = MultipleVariablesTable::new(cols, rows).unwrap();
                        if has_row_titles {
                            mv.row_titles = rt;
                            mv.row_titles_expanded = true;
                        }
                        for (c, col_cells) in grid.into_iter().enumerate() {
                            mv.variables[c].cells = col_cells;
                            mv.variables[c].refresh_inferred_role();
                        }
                        DataSheet::new(id.clone(), title.clone(), TableArchetype::MultipleVariables(mv)).unwrap()
                    })
                    .boxed(),
                _ => (row_titles_strat, prop::collection::vec(prop::collection::vec(prop::collection::vec(arb_numeric_cell(), 2), rows), cols))
                    .prop_map(move |(rt, grid)| {
                        let mut nested = NestedTable::new(cols, 2, rows).unwrap();
                        if has_row_titles {
                            nested.row_titles = rt;
                            nested.row_titles_expanded = true;
                        }
                        for (g, grp_rows) in grid.into_iter().enumerate() {
                            let mut grp = NestedGroupColumn::new(format!("Grp{g}"), 2, rows).unwrap();
                            grp.subcolumn_titles = vec![format!("U{g}A"), format!("U{g}B")];
                            grp.cells = grp_rows;
                            nested.groups[g] = grp;
                        }
                        DataSheet::new(id.clone(), title.clone(), TableArchetype::Nested(nested)).unwrap()
                    })
                    .boxed(),
            }
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn prop_pzfx_roundtrip_preserves_data_layer_across_all_eight_archetypes_and_modes(
        t0 in arb_representable_table(0),
        t1 in arb_representable_table(1),
        select_second in any::<bool>(),
        include_info in any::<bool>(),
        const_num in -500_i32..=500_i32,
    ) {
        let mut doc = ProjectDocument::new("Prop Study").unwrap();
        if include_info {
            let mut info = InfoSheet::new_preseeded("info-0", "Study Info").unwrap();
            info.constants = vec![
                InfoConstant::new("Date", ConstantValue::Date(String::from("2026-03-14")), None).unwrap(),
                InfoConstant::new("Conc", ConstantValue::Number(f64::from(const_num) / 10.0), None).unwrap(),
                InfoConstant::new("Buffer", ConstantValue::Text(String::from("PBS pH 7.4")), None).unwrap(),
                InfoConstant::new("Blank", ConstantValue::Empty, None).unwrap(),
            ];
            info.notes_markdown = String::from("Generated property-test protocol");
            doc.add_info_sheet(info).unwrap();
        }
        doc.add_data_sheet(t0).unwrap();
        doc.add_data_sheet(t1).unwrap();
        if select_second {
            doc.select_in_navigator(&SheetId::new("tbl-1")).unwrap();
        }

        let preflight = preflight_pzfx_export(&doc);
        prop_assert!(!preflight.has_dropped_items(), "representable doc had unexpected dropped items: {preflight:?}");

        let rep1 = export_pzfx_bytes(&doc).unwrap();
        let rep2 = export_pzfx_bytes(&doc).unwrap();
        prop_assert_eq!(&rep1.bytes, &rep2.bytes, "export bytes must be deterministic");
        prop_assert_eq!(&rep1.preflight, &preflight);

        let reimported = import_pzfx_bytes(&rep1.bytes).unwrap();
        prop_assert!(validate_project_document(&reimported.document).is_ok());

        let diff = compare_pzfx_data_layer(&doc, &reimported.document);
        prop_assert!(diff.is_identical(), "round-trip data-layer diff: {diff:?}");
    }

    #[test]
    fn prop_pzfx_preflight_matches_export_report_dropped_counts(
        n_results in 0_usize..=3,
        n_graphs in 0_usize..=3,
        n_layouts in 0_usize..=2,
        n_checkpoints in 0_usize..=2,
        n_images in 0_usize..=2,
        freeze_sheet in any::<bool>(),
    ) {
        let mut doc = ProjectDocument::new("Dropped Prop").unwrap();
        let mut ds = DataSheet::new("d-0", "Data", TableArchetype::Column(ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 2).unwrap())).unwrap();
        ds.frozen = freeze_sheet;
        doc.add_data_sheet(ds).unwrap();

        for i in 0..n_results {
            doc.add_result_sheet(result_sheet_with_kind(&format!("r-{i}"), "d-0", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))).unwrap();
        }
        for i in 0..n_graphs {
            let mut graph = GraphSheet::new_uninitialized_for_data(format!("g-{i}"), &doc.data_sheets[0]).unwrap();
            graph.title = format!("Graph {i}");
            doc.add_graph_sheet(graph).unwrap();
        }
        for i in 0..n_layouts {
            doc.add_layout_sheet(LayoutSheet::new(format!("l-{i}"), format!("Layout {i}"), LayoutSpec::default()).unwrap()).unwrap();
        }
        for i in 0..n_checkpoints {
            doc.checkpoints.push(NamedCheckpoint {
                id: format!("cp-{i}"),
                name: format!("Milestone {i}"),
                note: String::new(),
                created_utc: String::from("unix:1700000000"),
                snapshot_json: String::from("{}"),
            });
        }
        for i in 0..n_images {
            let payload = encode_rgba_png((i + 1) as u32, 1);
            doc.upsert_image_asset(ingest_image_asset(&payload).unwrap()).unwrap();
        }

        let preflight = preflight_pzfx_export(&doc);
        let expected_total = n_results + n_graphs + n_layouts + n_checkpoints + n_images + usize::from(freeze_sheet);
        prop_assert_eq!(preflight.total_dropped_count(), expected_total);
        prop_assert_eq!(preflight.has_dropped_items(), expected_total > 0);
        prop_assert_eq!(preflight.group(PzfxDroppedKind::AnalysesAndResults).map_or(0, |g| g.count), n_results);
        prop_assert_eq!(preflight.group(PzfxDroppedKind::Graphs).map_or(0, |g| g.count), n_graphs);
        prop_assert_eq!(preflight.group(PzfxDroppedKind::Layouts).map_or(0, |g| g.count), n_layouts);
        prop_assert_eq!(preflight.group(PzfxDroppedKind::Checkpoints).map_or(0, |g| g.count), n_checkpoints);
        prop_assert_eq!(preflight.group(PzfxDroppedKind::Images).map_or(0, |g| g.count), n_images);

        let report = export_pzfx_bytes(&doc).unwrap();
        prop_assert_eq!(&report.preflight, &preflight);
        let dropped_warn_count = report.warnings.iter().filter(|w| w.kind == PzfxExportWarningKind::DroppedContent).count();
        prop_assert_eq!(dropped_warn_count, preflight.dropped.len());
    }
}

#[test]
fn pzfx_roundtrip_preserves_tag_wrapped_titles_and_richtext_escapes_without_whitespace_drift() {
    // Tag-only titles/row titles/nested group titles (zero top-level text outside child tags)
    // plus `<Font>`-split triggers, `<BR/>`, and empty trailing `<Font/>` must round-trip with
    // zero whitespace or escape drift through pretty-printed `export_pzfx_bytes`.
    let xml = wrap_vendor_root(
        "",
        r#"
        <TableSequence Selected="1">
          <Ref ID="t-rt" Selected="1"/>
          <Ref ID="t-nest"/>
        </TableSequence>
        <Table ID="t-rt" TableType="XY" XFormat="numbers" YFormat="replicates" Replicates="2" EVFormat="AsteriskAfterNumber">
          <Title><B>Tag Only Table</B></Title>
          <RowTitlesColumn Width="81">
            <Subcolumn>
              <d><B>Row Bold</B></d>
              <d><SUP>**</SUP></d>
              <d><B>Group</B> <Font>*</Font><Font>*</Font></d>
              <d><B>Line1</B>_<BR/>Line2</d>
              <d><B>Item</B> A_<Font> 1</Font><Font/></d>
            </Subcolumn>
          </RowTitlesColumn>
          <SubColumnTitles OwnSet="0">
            <Subcolumn><d><I>Rep One</I></d></Subcolumn>
            <Subcolumn><d><B>Rep</B> <Font>*</Font><Font>*</Font></d></Subcolumn>
          </SubColumnTitles>
          <XColumn Width="81" Decimals="2" Subcolumns="1">
            <Title><I>Time Axis</I></Title>
            <Subcolumn><d>1</d><d>2</d><d>3</d><d>4</d><d>5</d></Subcolumn>
          </XColumn>
          <YColumn Width="81" Decimals="2" Subcolumns="2">
            <Title><B><I>Control</I></B></Title>
            <Subcolumn><d>10</d><d>20</d><d>30</d><d>40</d><d>50</d></Subcolumn>
            <Subcolumn><d>11</d><d>21</d><d>31</d><d>41</d><d>51</d></Subcolumn>
          </YColumn>
        </Table>
        <Table ID="t-nest" TableType="TwoWay" ExtTableType="Nested" XFormat="none" YFormat="replicates" Replicates="2" EVFormat="AsteriskAfterNumber">
          <Title>Nested Rich Titles</Title>
          <YColumn Width="81" Decimals="2" Subcolumns="2">
            <Title><B>WT Group</B></Title>
            <Subcolumn><d>1.5</d></Subcolumn>
            <Subcolumn><d>2.5</d></Subcolumn>
          </YColumn>
        </Table>
        "#,
    );

    let first = import_pzfx_str(&xml).unwrap();
    let exported = export_pzfx_bytes(&first.document).unwrap();
    let exported_xml = std::str::from_utf8(&exported.bytes).unwrap();
    assert!(!exported_xml.contains("</B>\n"), "inline rich-text close tag must not be followed by indentation newline");
    assert!(!exported_xml.contains("</I>\n"), "inline rich-text close tag must not be followed by indentation newline");
    assert!(!exported_xml.contains("</SUP>\n"), "inline rich-text close tag must not be followed by indentation newline");

    let second = import_pzfx_bytes(&exported.bytes).unwrap();
    let diff = compare_pzfx_data_layer(&first.document, &second.document);
    assert!(diff.is_identical(), "expected zero round-trip diffs, got: {diff:?}");

    let TableArchetype::Nested(ref nest) = second.document.data_sheets[1].archetype else { panic!("expected Nested") };
    assert_eq!(nest.groups[0].title, "<b>WT Group</b>");
    assert_eq!(nest.groups[0].subcolumn_titles, vec!["<b>WT Group</b>:1", "<b>WT Group</b>:2"]);
}

#[test]
fn pzfx_roundtrip_preserves_xformat_time_with_empty_title_and_distinct_unit_label() {
    let xml = wrap_vendor_root(
        "",
        r#"
        <TableSequence Selected="1">
          <Ref ID="t-empty-time" Selected="1"/>
          <Ref ID="t-adv-time"/>
        </TableSequence>
        <Table ID="t-empty-time" TableType="XY" XFormat="time" EVFormat="AsteriskAfterNumber">
          <Title>Empty Time Title</Title>
          <XColumn Width="81" Decimals="1" Subcolumns="1"><Title/><Subcolumn><d>0</d><d>5</d></Subcolumn></XColumn>
          <YColumn Width="81" Decimals="2" Subcolumns="1"><Title>Signal</Title><Subcolumn><d>1</d><d>2</d></Subcolumn></YColumn>
        </Table>
        <Table ID="t-adv-time" TableType="XY" XFormat="time" EVFormat="AsteriskAfterNumber">
          <Title>Advanced Time Title</Title>
          <XColumn Width="81" Decimals="1" Subcolumns="1"><Title/><Subcolumn><d>0</d><d>10</d></Subcolumn></XColumn>
          <XAdvancedColumn Width="81" Decimals="1" Subcolumns="1"><Title>Elapsed (min)</Title><Subcolumn><d>00:00</d><d>00:10</d></Subcolumn></XAdvancedColumn>
          <YColumn Width="81" Decimals="2" Subcolumns="1"><Title>Response</Title><Subcolumn><d>3</d><d>4</d></Subcolumn></YColumn>
        </Table>
        "#,
    );

    let first = import_pzfx_str(&xml).unwrap();
    let TableArchetype::Xy(ref xy0) = first.document.data_sheets[0].archetype else { panic!("expected XY") };
    assert_eq!(xy0.x_title, "Time");
    assert_eq!(xy0.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Time") });
    let TableArchetype::Xy(ref xy1) = first.document.data_sheets[1].archetype else { panic!("expected XY") };
    assert_eq!(xy1.x_title, "Elapsed (min)");
    assert_eq!(xy1.x_mode, XColumnMode::DateOrElapsedTime { unit_label: String::from("Elapsed (min)") });

    let exp = export_pzfx_bytes(&first.document).unwrap();
    assert!(exp.warnings.is_empty(), "expected zero export warnings, got {:?}", exp.warnings);
    let re = import_pzfx_bytes(&exp.bytes).unwrap();
    let diff = compare_pzfx_data_layer(&first.document, &re.document);
    assert!(diff.is_identical(), "unexpected time-mode diff: {diff:?}");

    // CurveBook-authored table with distinct `x_title` vs `unit_label`: preflight explains the
    // fold into X column title, and `compare_pzfx_data_layer` accepts the explained fold while
    // still flagging an actual X-mode variant change.
    let mut doc = ProjectDocument::new("Distinct Unit").unwrap();
    let mut xy = XyTable::new_with_x_mode(XColumnMode::DateOrElapsedTime { unit_label: String::from("Hours") }, SubcolumnMode::Single, 1, 2).unwrap();
    xy.x_title = String::from("Duration");
    xy.x_cells[0][0] = Cell::number(1.0);
    xy.x_cells[1][0] = Cell::number(2.0);
    xy.data_sets[0].cells[0][0] = Cell::number(10.0);
    xy.data_sets[0].cells[1][0] = Cell::number(20.0);
    doc.add_data_sheet(DataSheet::new("t-dist", "Distinct", TableArchetype::Xy(xy)).unwrap()).unwrap();

    let exp2 = export_pzfx_bytes(&doc).unwrap();
    assert!(exp2.preflight.group(PzfxDroppedKind::UnrepresentableTableFeatures).is_some());
    let re2 = import_pzfx_bytes(&exp2.bytes).unwrap();
    assert!(compare_pzfx_data_layer(&doc, &re2.document).is_identical());

    let mut wrong_mode = re2.document.clone();
    if let TableArchetype::Xy(ref mut bad_xy) = wrong_mode.data_sheets[0].archetype {
        bad_xy.x_mode = XColumnMode::Numbers { subcolumn_mode: SubcolumnMode::Single };
    }
    assert!(!compare_pzfx_data_layer(&doc, &wrong_mode).is_identical());
}

#[test]
fn pzfx_export_emits_compact_subcolumns_and_trims_sparse_trailing_empty_cells() {
    let mut doc = ProjectDocument::new("Compact Sparse").unwrap();

    // 1. Sparse 20-column x 200-row table: only column 0 has 200 rows; columns 1..20 have 1 value.
    let mut col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 20, 200).unwrap();
    for r in 0..200 {
        col.data_sets[0].cells[r][0] = Cell::number(r as f64);
    }
    for c in 1..20 {
        col.data_sets[c].cells[0][0] = Cell::number(c as f64);
    }
    doc.add_data_sheet(DataSheet::new("t-sparse", "Sparse", TableArchetype::Column(col)).unwrap()).unwrap();

    // 2. All-empty 3-row table must still preserve its row count on round-trip.
    let empty_col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 2, 3).unwrap();
    doc.add_data_sheet(DataSheet::new("t-empty", "EmptyRows", TableArchetype::Column(empty_col)).unwrap()).unwrap();

    let rep1 = export_pzfx_bytes(&doc).unwrap();
    let rep2 = export_pzfx_bytes(&doc).unwrap();
    assert_eq!(rep1.bytes, rep2.bytes, "compact export must remain byte-identical");

    let xml = std::str::from_utf8(&rep1.bytes).unwrap();
    assert!(xml.contains("<Subcolumn><d>0</d><d>1</d>"), "cells inside <Subcolumn> must be emitted compactly without per-cell newlines");
    // Sparse columns 1..20 have 1 populated cell each (not 200 trailing `<d/>` padding nodes).
    let d_count = xml.matches("<d>").count() + xml.matches("<d/>").count();
    assert_eq!(d_count, 200 + 19 + 3, "expected exact populated cell count + 3 empty tail anchor cells, got {d_count}");

    let re = import_pzfx_bytes(&rep1.bytes).unwrap();
    assert!(compare_pzfx_data_layer(&doc, &re.document).is_identical());
    let TableArchetype::Column(ref re_empty) = re.document.data_sheets[1].archetype else { panic!("expected Column") };
    assert_eq!(re_empty.row_count(), 3);
}
