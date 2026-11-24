use curvebook_doc::{
    AnalysisKind, AnalysisSpec, ChecklistItem, ChecklistStatus, CiBound, CurveSamplePoint, FittedCurveOverlay, OneWayAnovaSpec,
    PairwiseComparisonRecord, ResultCell, ResultScalarEntry, ResultSheet, ResultStatus, ResultTab, ResultTabKind, SubcolumnMode, TableArchetype,
    TableKind, VariableRole,
};
use curvebook_format::{
    CsvOptions, FormatError, ImportWarningKind, XmlLimits, ZipLimits, ZipWriter, export_result_sheet_json, export_result_sheet_tidy_csv,
    export_result_tab_csv, import_prism_bytes, import_prism_bytes_with_limits, import_pzfx_str, parse_delimited_records, parse_xml,
    parse_xml_with_limits,
};

#[test]
fn xml_parser_handles_bom_comments_cdata_entities_and_security_bounds() {
    let xml = "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- lead comment -->\n<Root attr=\"a &amp; b\">\n  <Title>Conc (10<sup>-6</sup> M)</Title>\n  <Raw><![CDATA[<tag> & &#65;]]></Raw>\n  <Glyph>&#x3B1; &lt; 0.05 &apos;ok&apos; &quot;1&quot;</Glyph>\n</Root>";
    let root = parse_xml(xml).unwrap();
    assert_eq!(root.attr("attr"), Some("a & b"));
    assert_eq!(root.first_child("Title").unwrap().plain_text(), "Conc (10-6 M)");
    assert_eq!(root.first_child("Raw").unwrap().plain_text(), "<tag> & &#65;");
    assert_eq!(root.first_child("Glyph").unwrap().plain_text(), "α < 0.05 'ok' \"1\"");

    // Reject DOCTYPE / ENTITY (XXE guard)
    let xxe = "<!DOCTYPE foo [<!ENTITY bar \"baz\">]><Root/>";
    assert!(matches!(parse_xml(xxe), Err(FormatError::XmlParse { .. })));

    // Depth limit guard
    let deep = "<A><B><C><D/></C></B></A>";
    let limits = XmlLimits { max_bytes: 4096, max_depth: 3, max_nodes: 100 };
    assert!(matches!(parse_xml_with_limits(deep, limits), Err(FormatError::XmlDepthExceeded { depth: 4, max: 3 })));
}

#[test]
fn pzfx_importer_covers_all_eight_table_types_exclusions_and_info_constants() {
    // Uses neutral <PrismFile ...> root tag cleanly without vendor copyright signatures.
    let pzfx = r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile SchemaVersion="2">
  <Title>Multi-Table Assay</Title>
  <InfoSequence>
    <Info ID="info-main">
      <Title>Experiment Metadata</Title>
      <Constant><Name>HotNM</Name><Value>2.5</Value><Units>nM</Units></Constant>
      <Constant><Name>Buffer</Name><Value>HEPES pH 7.4</Value></Constant>
      <Notes>All assays performed in triplicate.</Notes>
    </Info>
  </InfoSequence>
  <TableSequence>
    <Table ID="t-xy" TableType="XY" XFormat="numbers" YFormat="replicates" Replicates="2">
      <Title>Dose Response (10<sup>-6</sup> M)</Title>
      <RowTitlesColumn><Subcolumn><d>Low</d><d>High</d></Subcolumn></RowTitlesColumn>
      <XColumn><Title>Conc</Title><Subcolumn><d>1.0</d><d>10.0</d></Subcolumn></XColumn>
      <YColumn><Title>Wildtype</Title><Subcolumn><d>12.5</d><d>80.0</d></Subcolumn><Subcolumn><d Excluded="1">999.0</d><d>82.0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-col" TableType="OneWay">
      <Title>Treatment Groups</Title>
      <YColumn><Title>Control</Title><Subcolumn><d>5.1</d><d>5.3</d></Subcolumn></YColumn>
      <YColumn><Title>Drug</Title><Subcolumn><d>9.4</d><d Excluded="true">99.0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-grp" TableType="TwoWay" YFormat="SD">
      <Title>Factorial Assay</Title>
      <RowTitlesColumn><Subcolumn><d>24h</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Vehicle</Title><Subcolumn><d>10</d></Subcolumn><Subcolumn><d>1.2</d></Subcolumn><Subcolumn><d>6</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-cont" TableType="Contingency">
      <Title>Responder Counts</Title>
      <RowTitlesColumn><Subcolumn><d>Exposed</d><d>Unexposed</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Positive</Title><Subcolumn><d>14</d><d>3</d></Subcolumn></YColumn>
      <YColumn><Title>Negative</Title><Subcolumn><d>6</d><d>17</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-surv" TableType="Survival">
      <Title>Kaplan-Meier</Title>
      <XColumn><Title>Days</Title><Subcolumn><d>5</d><d>12</d></Subcolumn></XColumn>
      <YColumn><Title>Cohort A</Title><Subcolumn><d>1</d><d>0</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-pow" TableType="PartsOfWhole">
      <Title>Cell Fractions</Title>
      <RowTitlesColumn><Subcolumn><d>CD4+</d><d>CD8+</d></Subcolumn></RowTitlesColumn>
      <YColumn><Title>Count</Title><Subcolumn><d>45</d><d>55</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-mv" TableType="MultipleVariables">
      <Title>Patient Cohort</Title>
      <YColumn><Title>Cohort</Title><Subcolumn><d>A</d><d>B</d></Subcolumn></YColumn>
      <YColumn><Title>Age</Title><Subcolumn><d>42</d><d>58</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-nest" TableType="Nested" Replicates="2">
      <Title>Animal Replicates</Title>
      <YColumn><Title>WT</Title><Subcolumn><d>1.1</d><d>1.2</d></Subcolumn><Subcolumn><d>1.3</d><d>1.4</d></Subcolumn></YColumn>
    </Table>
    <Table ID="t-unk" TableType="CustomFuture">
      <Title>Fallback</Title>
      <YColumn><Title>A</Title><Subcolumn><d>1</d></Subcolumn></YColumn>
    </Table>
  </TableSequence>
  <AnalysisSequence>
    <Analysis ID="an-1"/>
  </AnalysisSequence>
</PrismFile>"#;

    let report = import_pzfx_str(pzfx).unwrap();
    let doc = &report.document;
    assert_eq!(doc.title, "Multi-Table Assay");
    assert_eq!(doc.info_sheets.len(), 1);
    assert_eq!(doc.info_sheets[0].get_constant("HotNM").unwrap().value.as_finite_f64(), Some(2.5));
    assert_eq!(doc.data_sheets.len(), 9);

    assert_eq!(doc.data_sheets[0].title, "Dose Response (10-6 M)");
    let TableArchetype::Xy(xy) = &doc.data_sheets[0].archetype else {
        panic!("expected XY");
    };
    assert_eq!(xy.y_subcolumn_mode, SubcolumnMode::RawReplicates(2));
    assert_eq!(xy.data_sets[0].cell(0, 1).unwrap().active_number(), None);
    assert!(xy.data_sets[0].cell(0, 1).unwrap().excluded);
    assert_eq!(xy.data_sets[0].cell(0, 1).unwrap().value.finite_f64(), Some(999.0));

    assert_eq!(doc.data_sheets[1].kind(), TableKind::Column);
    assert_eq!(doc.data_sheets[2].kind(), TableKind::Grouped);
    assert_eq!(doc.data_sheets[3].kind(), TableKind::Contingency);
    assert_eq!(doc.data_sheets[4].kind(), TableKind::Survival);
    assert_eq!(doc.data_sheets[5].kind(), TableKind::PartsOfWhole);
    let TableArchetype::MultipleVariables(mv) = &doc.data_sheets[6].archetype else {
        panic!("expected MultipleVariables");
    };
    assert_eq!(mv.variables[0].role, VariableRole::Categorical);
    assert_eq!(mv.variables[1].role, VariableRole::Continuous);
    assert_eq!(doc.data_sheets[7].kind(), TableKind::Nested);

    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::UnsupportedTableType));
    assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::IgnoredSection));
}

#[test]
fn prism_zip_importer_reads_csv_json_and_warns_on_unknown_binary_chunks() {
    let mut writer = ZipWriter::new();
    let manifest = serde_json::json!({
        "title": "External Container Assay",
        "sheets": [
            {
                "id": "sheet-xy",
                "title": "Kinetic Trace",
                "kind": "XY",
                "subcolumn_mode": "Single",
                "csv_path": "data/kinetics.csv"
            }
        ]
    });
    writer.add_file("manifest.json", serde_json::to_vec(&manifest).unwrap()).unwrap();
    writer.add_file("data/kinetics.csv", b"Time,Signal\n0,1.0\n5,4.2*\n10,9.5\n".to_vec()).unwrap();
    writer
        .add_file(
            "info/info.json",
            serde_json::to_vec(&serde_json::json!({
                "id": "info-1",
                "title": "Notes",
                "constants": { "Km": 12.5, "Operator": "A. Researcher" },
                "notes": "Imported cleanly"
            }))
            .unwrap(),
        )
        .unwrap();
    writer.add_file("chunks/raw.arrow", vec![0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    writer.add_file("scripts/plot.py", b"print('hi')".to_vec()).unwrap();

    let zip_bytes = writer.finish().unwrap();
    let report = import_prism_bytes(&zip_bytes).unwrap();
    assert_eq!(report.document.title, "External Container Assay");
    assert_eq!(report.document.info_sheets.len(), 1);
    assert_eq!(report.document.info_sheets[0].get_constant("Km").unwrap().value.as_finite_f64(), Some(12.5));
    assert_eq!(report.document.data_sheets.len(), 1);
    assert_eq!(report.document.data_sheets[0].kind(), TableKind::Xy);
    let TableArchetype::Xy(xy) = &report.document.data_sheets[0].archetype else {
        panic!("expected XY");
    };
    assert!(xy.data_sets[0].cell(1, 0).unwrap().excluded);
    assert_eq!(report.warnings.len(), 2);
    assert!(report.warnings.iter().all(|w| w.kind == ImportWarningKind::UnsupportedChunk));

    // Bounded ZIP limits enforced for `.prism`
    let tight = ZipLimits { max_entries: 2, max_total_uncompressed_bytes: 1024 * 1024, max_entry_uncompressed_bytes: 1024 * 1024 };
    assert!(matches!(import_prism_bytes_with_limits(&zip_bytes, tight), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn result_sheet_tidy_csv_and_structured_json_export() {
    let spec = AnalysisSpec::new("anova-1", "data-1", AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).unwrap();
    let mut res = ResultSheet::new(spec).unwrap();
    res.status = ResultStatus::UpToDate;
    res.scalars.push(ResultScalarEntry { key: String::from("R2"), data_set_index: None, value: 0.942, units: None });
    res.tabs.push(ResultTab {
        id: String::from("summary"),
        title: String::from("ANOVA Summary"),
        kind: ResultTabKind::AnovaTable,
        column_headers: vec![String::from("F"), String::from("P value"), String::from("95% CI"), String::from("Significant")],
        row_labels: vec![String::from("Treatment")],
        rows: vec![vec![
            ResultCell::Number { value: 18.4, prefix: None, units: None },
            ResultCell::PValue { raw: 0.000_032, formatted: String::from("< 0.0001"), stars: String::from("****") },
            ResultCell::ConfidenceInterval { lower: CiBound::Finite(2.1), upper: CiBound::Open },
            ResultCell::Boolean(true),
        ]],
        footnotes: vec![],
    });
    res.pairwise_comparisons.push(PairwiseComparisonRecord {
        group_a: 0,
        group_b: 1,
        label_a: String::from("Control"),
        label_b: String::from("Treated"),
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
        points: vec![CurveSamplePoint { x: -7.0, y: 50.0, ci_lower: Some(45.0), ci_upper: Some(55.0), pred_lower: None, pred_upper: None }],
    });
    res.checklist.push(ChecklistItem {
        assumption: String::from("Normality of residuals"),
        status: ChecklistStatus::Pass,
        diagnostic_metric: Some(String::from("W = 0.98")),
        explanation: String::from("Residuals consistent with Gaussian distribution"),
        remediation: None,
    });

    let opts = CsvOptions::csv();
    let tab_csv = export_result_tab_csv(&res.tabs[0], &opts);
    let tab_rows = parse_delimited_records(&tab_csv, &opts).unwrap();
    assert_eq!(tab_rows.len(), 2);
    assert_eq!(tab_rows[1][0], "Treatment");
    assert_eq!(tab_rows[1][2], "< 0.0001 (****)");
    assert_eq!(tab_rows[1][3], "[2.1, Open]");

    let tidy_csv = export_result_sheet_tidy_csv(&res, &opts);
    let tidy_rows = parse_delimited_records(&tidy_csv, &opts).unwrap();
    // 1 header + 4 tab cells + 1 scalar + 1 comparison + 1 curve point + 1 checklist = 9 rows
    assert_eq!(tidy_rows.len(), 9);
    assert!(tidy_rows.iter().any(|r| r[2] == "tab_cell" && r[12] == "0.000032"));
    assert!(tidy_rows.iter().any(|r| r[2] == "scalar" && r[6] == "R2" && r[9] == "0.942"));
    assert!(tidy_rows.iter().any(|r| r[2] == "pairwise_comparison" && r[16] == "true"));
    assert!(tidy_rows.iter().any(|r| r[2] == "curve_point" && r[14] == "45"));
    assert!(tidy_rows.iter().any(|r| r[2] == "checklist" && r[17] == "Pass"));

    let json = export_result_sheet_json(&res, true).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["id"], "anova-1");
    assert_eq!(parsed["scalars"][0]["key"], "R2");
}

#[test]
fn prism_zip_importer_uses_neutral_default_title_when_manifest_title_missing() {
    let mut writer = ZipWriter::new();
    writer.add_file("manifest.json", br#"{"title":"   "}"#.to_vec()).unwrap();
    writer.add_file("data/values.csv", b"Group,Response\nA,1.5\nB,2.5\n".to_vec()).unwrap();
    let zip_bytes = writer.finish().unwrap();
    let report = import_prism_bytes(&zip_bytes).unwrap();
    assert_eq!(report.document.title, "Imported Project");
}
