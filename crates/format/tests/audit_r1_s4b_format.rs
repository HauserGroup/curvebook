use lumencraft_doc::{MAX_TITLE_BYTES, TableArchetype};
use lumencraft_format::{FormatError, ImportWarningKind, XmlLimits, import_pzfx_str, parse_xml, parse_xml_with_limits, validate_project_document};
use lumencraft_palette::colormap::MAX_CUSTOM_STOPS;
use lumencraft_palette::{ColorRamp, ColorStop, Rgba8};

#[test]
fn r1_s06_10_pzfx_overlong_row_titles_and_pow_slice_labels_truncate_safely() {
    // Include multi-byte UTF-8 characters crossing the MAX_TITLE_BYTES boundary.
    let overlong_row_title: String = std::iter::repeat_n('é', MAX_TITLE_BYTES).collect();
    assert!(overlong_row_title.len() > MAX_TITLE_BYTES);

    let pzfx_row_titles = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile PrismXMLVersion="5.00">
  <Table Sequence="0" TableType="OneWay">
    <Title>Row Title Truncation</Title>
    <RowTitlesColumn>
      <Subcolumn><d>{overlong_row_title}</d></Subcolumn>
    </RowTitlesColumn>
    <YColumn>
      <Title>Group A</Title>
      <Subcolumn><d>10.5</d></Subcolumn>
    </YColumn>
  </Table>
</PrismFile>"#
    );
    let report = import_pzfx_str(&pzfx_row_titles);
    assert!(report.is_ok(), "import with overlong row title should truncate, not fail: {report:?}");
    let Ok(report) = report else { return };
    assert!(
        report.warnings.iter().any(|w| w.kind == ImportWarningKind::TruncatedTitle),
        "expected TruncatedTitle warning, got {:?}",
        report.warnings
    );
    assert!(validate_project_document(&report.document).is_ok(), "imported document should pass validation");
    if let Some(sheet) = report.document.data_sheets.first()
        && let TableArchetype::Column(col) = &sheet.archetype
    {
        assert!(!col.row_titles.is_empty());
        assert!(col.row_titles[0].len() <= MAX_TITLE_BYTES);
    }

    // Also test PartsOfWhole slice labels derived from XColumn when RowTitlesColumn is omitted.
    let overlong_slice: String = "切片".repeat(MAX_TITLE_BYTES / 2);
    let pzfx_pow = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<PrismFile PrismXMLVersion="5.00">
  <Table Sequence="1" TableType="PartsOfWhole">
    <Title>PoW Slice Truncation</Title>
    <XColumn>
      <Subcolumn><d>{overlong_slice}</d></Subcolumn>
    </XColumn>
    <YColumn>
      <Title>Counts</Title>
      <Subcolumn><d>42</d></Subcolumn>
    </YColumn>
  </Table>
</PrismFile>"#
    );
    let pow_report = import_pzfx_str(&pzfx_pow);
    assert!(pow_report.is_ok(), "import PoW with overlong slice label should truncate: {pow_report:?}");
    let Ok(pow_report) = pow_report else { return };
    assert!(
        pow_report.warnings.iter().any(|w| w.kind == ImportWarningKind::TruncatedTitle),
        "expected TruncatedTitle warning for PoW slice label: {:?}",
        pow_report.warnings
    );
    assert!(validate_project_document(&pow_report.document).is_ok(), "PoW document should validate after truncation");
    if let Some(sheet) = pow_report.document.data_sheets.first()
        && let TableArchetype::PartsOfWhole(pow) = &sheet.archetype
    {
        assert!(!pow.slice_labels.is_empty());
        assert!(pow.slice_labels[0].len() <= MAX_TITLE_BYTES);
    }
}

#[test]
fn r1_s06_11_xml_counts_text_cdata_attributes_and_rejects_duplicate_attrs() {
    let text_heavy = "<root>a<x/>b<y/>c</root>";
    // 3 elements (<root>, <x>, <y>) + 3 non-empty text nodes (a, b, c) = 6 nodes total.
    let limits_five = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: 5 };
    assert!(matches!(parse_xml_with_limits(text_heavy, limits_five), Err(FormatError::XmlParse { .. })), "text nodes must count toward max_nodes");
    let limits_six = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: 6 };
    assert!(parse_xml_with_limits(text_heavy, limits_six).is_ok());

    let cdata_heavy = "<root><![CDATA[alpha]]><![CDATA[beta]]><x/></root>";
    // 2 elements + 2 non-empty CDATA nodes = 4 nodes total.
    let limits_three = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: 3 };
    assert!(matches!(parse_xml_with_limits(cdata_heavy, limits_three), Err(FormatError::XmlParse { .. })), "CDATA nodes must count toward max_nodes");
    let limits_four = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: 4 };
    assert!(parse_xml_with_limits(cdata_heavy, limits_four).is_ok());

    let attr_heavy = r#"<root a1="1" a2="2" a3="3" a4="4"/>"#;
    // 1 element + 4 attributes = 5 nodes total.
    assert!(matches!(parse_xml_with_limits(attr_heavy, limits_four), Err(FormatError::XmlParse { .. })), "attributes must count toward max_nodes");
    assert!(parse_xml_with_limits(attr_heavy, limits_five).is_ok());

    // Duplicate attributes (small count linear scan & large count HashSet path).
    let dup_small = r#"<root dup="1" other="2" dup="3"/>"#;
    assert!(
        matches!(parse_xml(dup_small), Err(FormatError::XmlParse { reason, .. }) if reason.contains("duplicate attribute")),
        "duplicate attribute must be rejected"
    );

    let mut distinct_attrs = String::from("<root");
    for i in 0..25 {
        distinct_attrs.push_str(&format!(" attr_{i}=\"{i}\""));
    }
    let dup_large = format!("{distinct_attrs} attr_3=\"dup\"/>");
    assert!(
        matches!(parse_xml(&dup_large), Err(FormatError::XmlParse { reason, .. }) if reason.contains("duplicate attribute")),
        "duplicate attribute past HashSet switch threshold must be rejected"
    );

    distinct_attrs.push_str("/>");
    assert!(parse_xml(&distinct_attrs).is_ok(), "25 distinct attributes should parse cleanly");
}

#[test]
fn r1_s01_07_color_ramp_serde_json_validation_and_normalization() {
    let mut stops = Vec::with_capacity(MAX_CUSTOM_STOPS + 1);
    for i in 0..=MAX_CUSTOM_STOPS {
        stops.push(serde_json::json!({
            "position": (i as f64) / (MAX_CUSTOM_STOPS as f64),
            "color": { "r": 10, "g": 20, "b": 30, "a": 255 }
        }));
    }
    let too_many_json = serde_json::json!({ "Custom": { "stops": stops } }).to_string();
    assert!(serde_json::from_str::<ColorRamp>(&too_many_json).is_err(), "deserializing > MAX_CUSTOM_STOPS must fail");

    let valid_unsorted = serde_json::json!({
        "Custom": {
            "stops": [
                { "position": 1.5, "color": { "r": 255, "g": 255, "b": 255, "a": 255 } },
                { "position": 0.5, "color": { "r": 255, "g": 0, "b": 0, "a": 255 } },
                { "position": 0.5, "color": { "r": 0, "g": 255, "b": 0, "a": 255 } },
                { "position": 0.5, "color": { "r": 0, "g": 0, "b": 255, "a": 255 } },
                { "position": -0.5, "color": { "r": 0, "g": 0, "b": 0, "a": 255 } }
            ]
        }
    })
    .to_string();
    let parsed = serde_json::from_str::<ColorRamp>(&valid_unsorted);
    assert!(parsed.is_ok(), "valid bounded stops should deserialize & normalize: {parsed:?}");
    if let Ok(ColorRamp::Custom { stops: norm }) = parsed {
        assert_eq!(norm.len(), 4);
        assert_eq!(norm[0], ColorStop { position: 0.0, color: Rgba8::BLACK });
        assert_eq!(norm[1], ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) });
        assert_eq!(norm[2], ColorStop { position: 0.5, color: Rgba8::rgb(0, 0, 255) });
        assert_eq!(norm[3], ColorStop { position: 1.0, color: Rgba8::WHITE });
    }
}
