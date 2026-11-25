use crate::common::pzfx_doc;
use curvebook_doc::{MAX_TITLE_BYTES, TableArchetype};
use curvebook_format::{FormatError, ImportWarningKind, XmlLimits, import_pzfx_str, parse_xml, parse_xml_with_limits, validate_project_document};
use curvebook_palette::colormap::MAX_CUSTOM_STOPS;
use curvebook_palette::{ColorRamp, ColorStop, Rgba8};

#[test]
fn r1_s06_10_pzfx_overlong_row_titles_and_pow_slice_labels_truncate_safely() {
    let overlong_row_title: String = std::iter::repeat_n('é', MAX_TITLE_BYTES).collect();
    assert!(overlong_row_title.len() > MAX_TITLE_BYTES);
    let overlong_slice: String = "切片".repeat(MAX_TITLE_BYTES / 2);

    let cases = [
        (
            "oneway_row_title",
            pzfx_doc(&format!(
                r#"<Table Sequence="0" TableType="OneWay"><Title>Row Title Truncation</Title><RowTitlesColumn><Subcolumn><d>{overlong_row_title}</d></Subcolumn></RowTitlesColumn><YColumn><Title>Group A</Title><Subcolumn><d>10.5</d></Subcolumn></YColumn></Table>"#
            )),
        ),
        (
            "pow_slice_label",
            pzfx_doc(&format!(
                r#"<Table Sequence="1" TableType="PartsOfWhole"><Title>PoW Slice Truncation</Title><XColumn><Subcolumn><d>{overlong_slice}</d></Subcolumn></XColumn><YColumn><Title>Counts</Title><Subcolumn><d>42</d></Subcolumn></YColumn></Table>"#
            )),
        ),
    ];

    for (label, xml) in cases {
        let report = import_pzfx_str(&xml).unwrap_or_else(|e| panic!("{label}: should truncate, not fail: {e:?}"));
        assert!(report.warnings.iter().any(|w| w.kind == ImportWarningKind::TruncatedTitle), "{label}: missing TruncatedTitle warning");
        assert!(validate_project_document(&report.document).is_ok(), "{label}: document should validate");
        match &report.document.data_sheets[0].archetype {
            TableArchetype::Column(col) => assert!(!col.row_titles.is_empty() && col.row_titles[0].len() <= MAX_TITLE_BYTES),
            TableArchetype::PartsOfWhole(pow) => assert!(!pow.slice_labels.is_empty() && pow.slice_labels[0].len() <= MAX_TITLE_BYTES),
            other => panic!("{label}: unexpected archetype {other:?}"),
        }
    }
}

#[test]
fn r1_s06_11_xml_counts_text_cdata_attributes_and_rejects_duplicate_attrs() {
    // (label, xml, exact_node_count)
    let limit_cases = [
        ("text_nodes", "<root>a<x/>b<y/>c</root>", 6),
        ("cdata_nodes", "<root><![CDATA[alpha]]><![CDATA[beta]]><x/></root>", 4),
        ("attributes", r#"<root a1="1" a2="2" a3="3" a4="4"/>"#, 5),
    ];
    for (label, xml, exact_nodes) in limit_cases {
        let too_tight = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: exact_nodes - 1 };
        let exact = XmlLimits { max_bytes: 4096, max_depth: 16, max_nodes: exact_nodes };
        assert!(matches!(parse_xml_with_limits(xml, too_tight), Err(FormatError::XmlParse { .. })), "{label} must count toward max_nodes");
        assert!(parse_xml_with_limits(xml, exact).is_ok(), "{label} should pass at exact limit {exact_nodes}");
    }

    let mut distinct_attrs = String::from("<root");
    for i in 0..25 {
        distinct_attrs.push_str(&format!(" attr_{i}=\"{i}\""));
    }
    let dup_large = format!("{distinct_attrs} attr_3=\"dup\"/>");
    for (label, xml) in [("dup_small", r#"<root dup="1" other="2" dup="3"/>"#), ("dup_hashset", dup_large.as_str())] {
        assert!(
            matches!(parse_xml(xml), Err(FormatError::XmlParse { reason, .. }) if reason.contains("duplicate attribute")),
            "{label}: duplicate attribute must be rejected"
        );
    }

    distinct_attrs.push_str("/>");
    assert!(parse_xml(&distinct_attrs).is_ok(), "25 distinct attributes should parse cleanly");
}

#[test]
fn r1_s01_07_color_ramp_serde_json_validation_and_normalization() {
    let stops: Vec<_> = (0..=MAX_CUSTOM_STOPS)
        .map(|i| serde_json::json!({ "position": (i as f64) / (MAX_CUSTOM_STOPS as f64), "color": { "r": 10, "g": 20, "b": 30, "a": 255 } }))
        .collect();
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
    let ColorRamp::Custom { stops: norm } = serde_json::from_str::<ColorRamp>(&valid_unsorted).unwrap() else {
        panic!("expected Custom ColorRamp");
    };
    assert_eq!(
        norm,
        vec![
            ColorStop { position: 0.0, color: Rgba8::BLACK },
            ColorStop { position: 0.5, color: Rgba8::rgb(255, 0, 0) },
            ColorStop { position: 0.5, color: Rgba8::rgb(0, 0, 255) },
            ColorStop { position: 1.0, color: Rgba8::WHITE },
        ]
    );
}
