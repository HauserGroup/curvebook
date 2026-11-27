//! Unit, fuzz, and property tests for §2.1 XML and §2.2 ZIP codec upgrades.

use std::collections::BTreeMap;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};

use curvebook_format::xml::{is_valid_xml10_char, lookup_named_xml_entity};
use curvebook_format::{
    FormatError, MAX_XML_BYTES, MAX_XML_OPAQUE_BYTES, SkippedOpaqueElement, XmlElement, XmlEncoding, XmlLimits, XmlNode, XmlParseOptions, XmlWriter,
    ZipArchive, ZipEntryClass, ZipIndex, ZipLimits, ZipReadOptions, classify_zip_entry_name, decode_cp437, detect_xml_encoding, parse_xml,
    parse_xml_bytes, parse_xml_bytes_with_options, parse_xml_with_limits, parse_xml_with_options, sanitize_zip_path, sanitize_zip_path_with_options,
};
use flate2::Compression;
use flate2::write::DeflateEncoder;
use proptest::collection;
use proptest::prelude::{ProptestConfig, any, prop_assert, prop_assert_eq, proptest};

fn encode_utf16(text: &str, big_endian: bool, with_bom: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if with_bom {
        out.extend_from_slice(if big_endian { &[0xFE, 0xFF] } else { &[0xFF, 0xFE] });
    }
    for unit in text.encode_utf16() {
        out.extend_from_slice(&if big_endian { unit.to_be_bytes() } else { unit.to_le_bytes() });
    }
    out
}

#[test]
fn xml_encoding_detection_utf8_and_utf16() {
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice("<?xml version=\"1.0\" encoding=\"UTF-8\"?><Root>café µM ± 2°</Root>".as_bytes());
    assert_eq!(detect_xml_encoding(&utf8_bom).unwrap(), (XmlEncoding::Utf8, 3));
    let parsed = parse_xml_bytes(&utf8_bom).unwrap();
    assert_eq!(parsed.encoding, XmlEncoding::Utf8);
    assert_eq!(parsed.root.plain_text(), "café µM ± 2°");
    assert_eq!(parsed.trailing_bytes, 0);

    let sample_unicode = "<?xml version=\"1.0\" encoding=\"UTF-16\"?><Root attr=\"αβγ\">Δ = 10 \u{1D6C2} \u{1F9EA}</Root>";
    for (big_endian, with_bom, expected_enc, expected_bom) in [
        (false, true, XmlEncoding::Utf16Le, 2),
        (false, false, XmlEncoding::Utf16Le, 0),
        (true, true, XmlEncoding::Utf16Be, 2),
        (true, false, XmlEncoding::Utf16Be, 0),
    ] {
        let bytes = encode_utf16(sample_unicode, big_endian, with_bom);
        assert_eq!(detect_xml_encoding(&bytes).unwrap(), (expected_enc, expected_bom));
        let res = parse_xml_bytes(&bytes).unwrap();
        assert_eq!(res.encoding, expected_enc);
        assert_eq!(res.root.attr("attr"), Some("αβγ"));
        assert_eq!(res.root.plain_text(), "Δ = 10 \u{1D6C2} \u{1F9EA}");
    }

    for (big_endian, expected_enc) in [(false, XmlEncoding::Utf16Le), (true, XmlEncoding::Utf16Be)] {
        let bytes = encode_utf16(" \n\t<Root>ok</Root>", big_endian, false);
        assert_eq!(detect_xml_encoding(&bytes).unwrap(), (expected_enc, 0));
        let res = parse_xml_bytes(&bytes).unwrap();
        assert_eq!(res.encoding, expected_enc);
        assert_eq!(res.root.plain_text(), "ok");
    }
}

#[test]
fn xml_encoding_single_byte_tables() {
    let mut iso_bytes = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><Root>caf\xE9 \xB5M \xB110\xB0 \xD7\xF7\xFF</Root>".to_vec();
    assert_eq!(detect_xml_encoding(&iso_bytes).unwrap(), (XmlEncoding::Iso8859_1, 0));
    let iso_res = parse_xml_bytes(&iso_bytes).unwrap();
    assert_eq!(iso_res.encoding, XmlEncoding::Iso8859_1);
    assert_eq!(iso_res.root.plain_text(), "café µM ±10° ×÷ÿ");
    iso_bytes = b"<?xml version='1.0' encoding='latin1'?><Root>\xC0\xDF\xE9</Root>".to_vec();
    assert_eq!(parse_xml_bytes(&iso_bytes).unwrap().root.plain_text(), "Àßé");

    let cp1252_bytes =
        b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><Root>\x80\x85\x86\x87\x89\x91a\x92\x93b\x94\x95\x96\x97\x99\xB1\xB5</Root>";
    assert_eq!(detect_xml_encoding(cp1252_bytes).unwrap(), (XmlEncoding::Windows1252, 0));
    let cp1252_res = parse_xml_bytes(cp1252_bytes).unwrap();
    assert_eq!(cp1252_res.encoding, XmlEncoding::Windows1252);
    assert_eq!(
        cp1252_res.root.plain_text(),
        "\u{20AC}\u{2026}\u{2020}\u{2021}\u{2030}\u{2018}a\u{2019}\u{201C}b\u{201D}\u{2022}\u{2013}\u{2014}\u{2122}\u{00B1}\u{00B5}"
    );
    for undef in [0x81_u8, 0x8D, 0x8F, 0x90, 0x9D] {
        let mut bad = b"<?xml version=\"1.0\" encoding=\"cp1252\"?><Root>".to_vec();
        bad.push(undef);
        bad.extend_from_slice(b"</Root>");
        assert!(matches!(parse_xml_bytes(&bad), Err(FormatError::XmlParse { .. })), "CP1252 0x{undef:02X} must fail");
    }

    let mac_bytes = b"<?xml version=\"1.0\" encoding=\"macintosh\"?><Root>\x80\x8E\xA1\xA5\xB0\xB1\xB2\xB3\xB5\xB9\xBD\xC3\xC5\xD0\xD1\xDB</Root>";
    assert_eq!(detect_xml_encoding(mac_bytes).unwrap(), (XmlEncoding::MacRoman, 0));
    let mac_res = parse_xml_bytes(mac_bytes).unwrap();
    assert_eq!(mac_res.encoding, XmlEncoding::MacRoman);
    assert_eq!(
        mac_res.root.plain_text(),
        "\u{00C4}\u{00E9}\u{00B0}\u{2022}\u{221E}\u{00B1}\u{2264}\u{2265}\u{00B5}\u{03C0}\u{03A9}\u{221A}\u{2248}\u{2013}\u{2014}\u{20AC}"
    );
}

#[test]
fn xml_encoding_rejects_unknown_utf32_conflicts_and_malformed_utf16() {
    for bad_enc in ["Shift_JIS", "ISO-8859-2", "EUC-JP", "KOI8-R", "bogus-charset", ""] {
        let raw = format!("<?xml version=\"1.0\" encoding=\"{bad_enc}\"?><Root/>");
        assert!(
            matches!(parse_xml_bytes(raw.as_bytes()), Err(FormatError::UnsupportedFormat { .. })),
            "unknown encoding `{bad_enc}` must fail with UnsupportedFormat"
        );
    }
    assert!(matches!(parse_xml_bytes(&[0x00, 0x00, 0xFE, 0xFF, 0, 0, 0, b'<']), Err(FormatError::UnsupportedFormat { .. })));
    assert!(matches!(parse_xml_bytes(&[0xFF, 0xFE, 0x00, 0x00, b'<', 0, 0, 0]), Err(FormatError::UnsupportedFormat { .. })));

    let mut bom_conflict = vec![0xEF, 0xBB, 0xBF];
    bom_conflict.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><Root/>");
    assert!(matches!(parse_xml_bytes(&bom_conflict), Err(FormatError::XmlParse { .. })));

    let utf16le_decl_be = encode_utf16("<?xml version=\"1.0\" encoding=\"UTF-16BE\"?><Root/>", false, true);
    assert!(matches!(parse_xml_bytes(&utf16le_decl_be), Err(FormatError::XmlParse { .. })));

    let mut lone_surrogate = encode_utf16("<Root>", false, true);
    lone_surrogate.extend_from_slice(&0xD800_u16.to_le_bytes());
    lone_surrogate.extend_from_slice(&encode_utf16("</Root>", false, false));
    assert!(matches!(parse_xml_bytes(&lone_surrogate), Err(FormatError::XmlParse { .. })));

    let mut odd_utf16 = encode_utf16("<Root>x", false, true);
    odd_utf16.push(0x41);
    assert!(matches!(parse_xml_bytes(&odd_utf16), Err(FormatError::XmlParse { .. })));
}

#[test]
fn xml_stop_after_root_reports_trailing_bytes_without_decoding_tail() {
    let opts = XmlParseOptions::default().with_stop_after_root(true);
    let binary_tail: &[u8] = &[0xFF, 0xFE, 0x81, 0x00, 0xD8, 0x00, 0x01]; // invalid UTF-8, undefined CP1252 0x81, odd len for UTF-16!

    // UTF-8 with binary tail
    let mut utf8 = b"<Root><Item>42</Item></Root>".to_vec();
    utf8.extend_from_slice(binary_tail);
    let res = parse_xml_bytes_with_options(&utf8, &opts).unwrap();
    assert_eq!(res.root.first_child("Item").unwrap().plain_text(), "42");
    assert_eq!(res.trailing_bytes, binary_tail.len());

    // Self-closing root + binary tail starting with '\n'
    let mut self_closing = b"<Root/>".to_vec();
    self_closing.extend_from_slice(b"\n\xFF\x00");
    let res_sc = parse_xml_bytes_with_options(&self_closing, &opts).unwrap();
    assert_eq!(res_sc.trailing_bytes, 3);

    // Trailing XML whitespace only -> trailing_bytes == 0
    let ws_only = b"<Root><A/></Root>\r\n  \t ";
    assert_eq!(parse_xml_bytes_with_options(ws_only, &opts).unwrap().trailing_bytes, 0);

    // UTF-16LE with invalid odd-length binary tail (tail never decoded!)
    let mut utf16le = encode_utf16("<Root><V>9</V></Root>", false, true);
    utf16le.extend_from_slice(binary_tail);
    let res16 = parse_xml_bytes_with_options(&utf16le, &opts).unwrap();
    assert_eq!(res16.encoding, XmlEncoding::Utf16Le);
    assert_eq!(res16.root.first_child("V").unwrap().plain_text(), "9");
    assert_eq!(res16.trailing_bytes, binary_tail.len());

    // UTF-16LE with UTF-16LE whitespace only -> trailing_bytes == 0
    let utf16le_ws = encode_utf16("<Root/>\r\n", false, true);
    assert_eq!(parse_xml_bytes_with_options(&utf16le_ws, &opts).unwrap().trailing_bytes, 0);

    // Windows-1252 with undefined CP1252 byte (0x81) in tail -> never decoded!
    let mut cp1252 = b"<?xml version=\"1.0\" encoding=\"windows-1252\"?><Root>\x80</Root>".to_vec();
    cp1252.extend_from_slice(binary_tail);
    let res_cp = parse_xml_bytes_with_options(&cp1252, &opts).unwrap();
    assert_eq!(res_cp.root.plain_text(), "€");
    assert_eq!(res_cp.trailing_bytes, binary_tail.len());

    // Unclosed root remains an error even with stop_after_root = true
    assert!(matches!(parse_xml_bytes_with_options(b"<Root><Child/>", &opts), Err(FormatError::XmlParse { .. })));

    // Strict default (stop_after_root = false) still rejects second root / trailing garbage
    assert!(matches!(parse_xml_bytes(b"<A/><B/>"), Err(FormatError::XmlParse { .. })));
    assert!(matches!(parse_xml_bytes(&utf8), Err(FormatError::XmlParse { .. })));
}

#[test]
fn xml_opaque_bodies_are_configurable_counted_and_enforce_caps() {
    let inner_body = "<Nested a=\"1\"><Deep>payload-text</Deep></Nested>".repeat(50);
    let xml = format!("<Doc><CustomBlob id=\"b1\">{inner_body}</CustomBlob><EmptyBlob/><Data>ok</Data></Doc>");

    // Without configuring `CustomBlob`, it is parsed normally into DOM children.
    let normal = parse_xml(&xml).unwrap();
    assert_eq!(normal.first_child("CustomBlob").unwrap().child_elements().count(), 50);

    // With configurable opaque elements, body is skipped with exact byte count and 0 child DOM nodes.
    let opts = XmlParseOptions::default().with_opaque_elements(["CustomBlob", "EmptyBlob"]).with_limits(XmlLimits {
        max_bytes: MAX_XML_BYTES,
        max_depth: 16,
        max_nodes: 10,
    });
    let parsed = parse_xml_with_options(&xml, &opts).unwrap();
    assert_eq!(parsed.root.first_child("Data").unwrap().plain_text(), "ok");
    let blob = parsed.root.first_child("CustomBlob").unwrap();
    assert_eq!(blob.attr("id"), Some("b1"));
    assert!(blob.children.is_empty());
    assert_eq!(
        parsed.skipped_opaque,
        vec![
            SkippedOpaqueElement { name: String::from("CustomBlob"), body_bytes: inner_body.len() },
            SkippedOpaqueElement { name: String::from("EmptyBlob"), body_bytes: 0 },
        ]
    );
    assert_eq!(parsed.total_opaque_bytes(), inner_body.len());

    // Exact opaque byte count in UTF-16LE (raw UTF-16 bytes!)
    let u16_xml = encode_utf16("<Doc><Blob>HELLO</Blob></Doc>", false, true);
    let u16_opts = XmlParseOptions::default().with_opaque_elements(["Blob"]);
    let u16_parsed = parse_xml_bytes_with_options(&u16_xml, &u16_opts).unwrap();
    assert_eq!(u16_parsed.skipped_opaque, vec![SkippedOpaqueElement { name: String::from("Blob"), body_bytes: 10 }]);

    // Skipped opaque bytes still count against raw `max_bytes`.
    let tight_bytes =
        XmlParseOptions::default().with_opaque_elements(["CustomBlob"]).with_limits(XmlLimits { max_bytes: 128, max_depth: 16, max_nodes: 100 });
    assert!(matches!(parse_xml_with_options(&xml, &tight_bytes), Err(FormatError::XmlParse { .. })));

    // Unclosed opaque element, DOCTYPE inside opaque element, and excessive depth inside opaque element all fail.
    let opaque_opts =
        XmlParseOptions::default().with_opaque_elements(["Blob"]).with_limits(XmlLimits { max_bytes: 4096, max_depth: 3, max_nodes: 100 });
    assert!(matches!(parse_xml_with_options("<Doc><Blob>unclosed</Doc>", &opaque_opts), Err(FormatError::XmlParse { .. })));
    assert!(matches!(parse_xml_with_options("<Doc><Blob><!DOCTYPE x></Blob></Doc>", &opaque_opts), Err(FormatError::XmlParse { .. })));
    assert!(matches!(
        parse_xml_with_options("<Doc><Blob><A><B/></A></Blob></Doc>", &opaque_opts),
        Err(FormatError::XmlDepthExceeded { depth: 4, max: 3 })
    ));

    // Raw cap ceiling: > 64 MiB rejected unless opaque skipping enabled; <= 128 MiB allowed with opaque; > 128 MiB rejected always.
    let over_64_no_opaque = XmlParseOptions::default().with_limits(XmlLimits { max_bytes: MAX_XML_OPAQUE_BYTES, ..XmlLimits::default() });
    assert!(matches!(parse_xml_bytes_with_options(b"<Root/>", &over_64_no_opaque), Err(FormatError::XmlParse { .. })));

    let at_128_with_opaque =
        XmlParseOptions::default().with_opaque_elements(["Blob"]).with_limits(XmlLimits { max_bytes: MAX_XML_OPAQUE_BYTES, ..XmlLimits::default() });
    assert!(parse_xml_bytes_with_options(b"<Root/>", &at_128_with_opaque).is_ok());

    let over_128_with_opaque = XmlParseOptions::default()
        .with_opaque_elements(["Blob"])
        .with_limits(XmlLimits { max_bytes: MAX_XML_OPAQUE_BYTES + 1, ..XmlLimits::default() });
    assert!(matches!(parse_xml_bytes_with_options(b"<Root/>", &over_128_with_opaque), Err(FormatError::XmlParse { .. })));
}

#[test]
fn xml_whitespace_only_text_nodes_exempt_from_max_nodes_but_count_bytes_and_preserve_mixed_content() {
    // Pretty-printed XML: 1 Root + 20 Item elements + 20 non-whitespace text nodes = 41 nodes.
    // Indentation introduces 41 additional whitespace-only text nodes that must NOT bump max_nodes.
    let mut pretty = String::from("<Root>\n");
    for i in 0..20 {
        pretty.push_str(&format!("    <Item>{i}</Item>\n"));
    }
    pretty.push_str("</Root>");

    let exact_limits = XmlLimits { max_bytes: pretty.len(), max_depth: 8, max_nodes: 41 };
    let root = parse_xml_with_limits(&pretty, exact_limits).unwrap();
    assert_eq!(root.children_named("Item").count(), 20);

    // One fewer node limit fails (proving non-whitespace text + elements still count).
    let too_few_nodes = XmlLimits { max_bytes: pretty.len(), max_depth: 8, max_nodes: 40 };
    assert!(matches!(parse_xml_with_limits(&pretty, too_few_nodes), Err(FormatError::XmlParse { .. })));

    // One fewer byte limit fails (proving whitespace bytes still count against max_bytes).
    let too_few_bytes = XmlLimits { max_bytes: pretty.len() - 1, max_depth: 8, max_nodes: 100 };
    assert!(matches!(parse_xml_with_limits(&pretty, too_few_bytes), Err(FormatError::XmlParse { .. })));

    // Mixed-content inter-element space is still preserved!
    let mixed = parse_xml("<Title><b>Alpha</b> <i>Beta</i></Title>").unwrap();
    assert_eq!(mixed.plain_text(), "Alpha Beta");
}

#[test]
fn xml_html4_entities_and_doctype_rejection() {
    // Spot-check W3C HTML 4 Latin-1, currency, punctuation, Greek, and math entities.
    let cases = [
        ("Agrave", '\u{00C0}'),
        ("Eacute", '\u{00C9}'),
        ("ntilde", '\u{00F1}'),
        ("ccedil", '\u{00E7}'),
        ("szlig", '\u{00DF}'),
        ("yuml", '\u{00FF}'),
        ("OElig", '\u{0152}'),
        ("oelig", '\u{0153}'),
        ("Scaron", '\u{0160}'),
        ("micro", '\u{00B5}'),
        ("mu", '\u{03BC}'),
        ("euro", '\u{20AC}'),
        ("cent", '\u{00A2}'),
        ("pound", '\u{00A3}'),
        ("curren", '\u{00A4}'),
        ("yen", '\u{00A5}'),
        ("ndash", '\u{2013}'),
        ("mdash", '\u{2014}'),
        ("lsquo", '\u{2018}'),
        ("rsquo", '\u{2019}'),
        ("sbquo", '\u{201A}'),
        ("ldquo", '\u{201C}'),
        ("rdquo", '\u{201D}'),
        ("bdquo", '\u{201E}'),
        ("lsaquo", '\u{2039}'),
        ("rsaquo", '\u{203A}'),
        ("hellip", '\u{2026}'),
        ("dagger", '\u{2020}'),
        ("Dagger", '\u{2021}'),
        ("permil", '\u{2030}'),
        ("ensp", '\u{2002}'),
        ("emsp", '\u{2003}'),
        ("thinsp", '\u{2009}'),
        ("frasl", '\u{2044}'),
        ("oline", '\u{203E}'),
        ("forall", '\u{2200}'),
        ("nabla", '\u{2207}'),
        ("oplus", '\u{2295}'),
        ("otimes", '\u{2297}'),
        ("sigmaf", '\u{03C2}'),
        ("thetasym", '\u{03D1}'),
    ];
    for (name, expected) in cases {
        assert_eq!(lookup_named_xml_entity(name), Some(expected), "entity &{name};");
        let xml = format!("<R a=\"&{name};\">&{name};</R>");
        let root = parse_xml(&xml).unwrap();
        let expected_str = expected.to_string();
        assert_eq!(root.attr("a"), Some(expected_str.as_str()));
        assert_eq!(root.plain_text(), expected_str);
    }

    // DOCTYPE and ENTITY remain strictly rejected across encodings.
    for bad in [
        "<!DOCTYPE root [<!ENTITY x \"y\">]><root>&x;</root>",
        "<!DOCTYPE root SYSTEM \"http://example.invalid/a.dtd\"><root/>",
        "<!ENTITY x \"y\"><root/>",
    ] {
        assert!(matches!(parse_xml(bad), Err(FormatError::XmlParse { .. })));
        let u16_bad = encode_utf16(bad, false, true);
        assert!(matches!(parse_xml_bytes(&u16_bad), Err(FormatError::XmlParse { .. })));
    }
}

#[test]
fn xml_writer_is_deterministic_preserves_attr_order_and_escapes() {
    let mut w = XmlWriter::with_indent("  ").unwrap();
    w.write_xml_declaration().unwrap();
    w.start_element("Doc", &[("z", "last"), ("a", "1 & 2 < 3 > 4 \"q\" 's'\t\n\r"), ("m", "mid")]).unwrap();
    w.start_inline_element("Title", &[("lang", "en")]).unwrap();
    w.text("H").unwrap();
    w.text_element("sub", &[], "2").unwrap();
    w.text("O & CO").unwrap();
    w.end_element_named("Title").unwrap();
    w.text_element("Note", &[("k", "v")], "a < b & c > d \"e\" 'f'\rg").unwrap();
    w.empty_element("Mark", &[("b", "2"), ("a", "1")]).unwrap();
    w.end_element_named("Doc").unwrap();
    let xml = w.finish().unwrap();

    assert!(xml.contains("<Doc z=\"last\" a=\"1 &amp; 2 &lt; 3 &gt; 4 &quot;q&quot; &apos;s&apos;&#x9;&#xA;&#xD;\" m=\"mid\">"));
    assert!(xml.contains("<Mark b=\"2\" a=\"1\"/>"));
    assert!(xml.contains("<Title lang=\"en\">H<sub>2</sub>O &amp; CO</Title>"));
    assert!(xml.contains("<Note k=\"v\">a &lt; b &amp; c &gt; d &quot;e&quot; &apos;f&apos;&#xD;g</Note>"));

    let parsed = parse_xml(&xml).unwrap();
    assert_eq!(parsed.attrs[0], (String::from("z"), String::from("last")));
    assert_eq!(parsed.attrs[1], (String::from("a"), String::from("1 & 2 < 3 > 4 \"q\" 's'\t\n\r")));
    assert_eq!(parsed.attrs[2], (String::from("m"), String::from("mid")));
    assert_eq!(parsed.first_child("Title").unwrap().plain_text(), "H2O & CO");
    assert_eq!(parsed.first_child("Note").unwrap().plain_text(), "a < b & c > d \"e\" 'f'\rg");

    let mut w2 = XmlWriter::new();
    w2.write_element(&parsed).unwrap();
    let compact = w2.finish().unwrap();
    assert_eq!(parse_xml(&compact).unwrap().attrs, parsed.attrs);
}

#[test]
fn xml_writer_rejects_control_chars_and_structural_errors() {
    let forbidden: Vec<char> =
        (0x00_u32..=0x1F).filter(|c| !matches!(c, 0x09 | 0x0A | 0x0D)).chain([0xFFFE, 0xFFFF]).filter_map(char::from_u32).collect();
    for ch in forbidden {
        let bad = format!("ok{ch}end");
        let mut wt = XmlWriter::new();
        wt.start_element("R", &[]).unwrap();
        assert!(matches!(wt.text(&bad), Err(FormatError::XmlParse { .. })), "text must reject U+{:04X}", ch as u32);
        assert!(matches!(wt.cdata(&bad), Err(FormatError::XmlParse { .. })), "cdata must reject U+{:04X}", ch as u32);
        assert!(matches!(wt.comment(&bad), Err(FormatError::XmlParse { .. })), "comment must reject U+{:04X}", ch as u32);
        assert!(matches!(wt.text_element("T", &[], &bad), Err(FormatError::XmlParse { .. })));

        let mut wa = XmlWriter::new();
        assert!(matches!(wa.empty_element("R", &[("a", &bad)]), Err(FormatError::XmlParse { .. })), "attr must reject U+{:04X}", ch as u32);
    }

    assert!(matches!(XmlWriter::new().empty_element("R", &[("a", "1"), ("a", "2")]), Err(FormatError::XmlParse { .. })));
    assert!(matches!(XmlWriter::new().empty_element("1Bad", &[]), Err(FormatError::XmlParse { .. })));
    assert!(matches!(XmlWriter::new().empty_element("R", &[("bad name", "1")]), Err(FormatError::XmlParse { .. })));
    let mut unclosed = XmlWriter::new();
    unclosed.start_element("R", &[]).unwrap();
    assert!(matches!(unclosed.finish(), Err(FormatError::XmlParse { .. })));
    let mut two_roots = XmlWriter::new();
    two_roots.empty_element("A", &[]).unwrap();
    assert!(matches!(two_roots.empty_element("B", &[]), Err(FormatError::XmlParse { .. })));
    let mut bad_cdata = XmlWriter::new();
    bad_cdata.start_element("R", &[]).unwrap();
    assert!(matches!(bad_cdata.cdata("a]]>b"), Err(FormatError::XmlParse { .. })));
    assert!(matches!(bad_cdata.comment("a--b"), Err(FormatError::XmlParse { .. })));
    assert!(matches!(bad_cdata.comment("trail-"), Err(FormatError::XmlParse { .. })));
}

// ---------------------------------------------------------------------------
// Synthetic ZIP / ZIP64 test builder (PKWARE APPNOTE §4.3.14–4.3.16 & §4.5.3)
// ---------------------------------------------------------------------------

struct RawEntrySpec {
    name: Vec<u8>,
    payload: Vec<u8>,
    method: u16,
    flags: u16,
    crc_override: Option<u32>,
    uncompressed_override: Option<u64>,
    zip64_uncomp_sentinel: bool,
    zip64_comp_sentinel: bool,
    zip64_offset_sentinel: bool,
    zip64_disk_sentinel: bool,
    disk_start_override: u32,
    omit_zip64_extra: bool,
    truncate_zip64_extra: bool,
}

impl RawEntrySpec {
    fn stored(name: &[u8], payload: &[u8]) -> Self {
        Self {
            name: name.to_vec(),
            payload: payload.to_vec(),
            method: 0,
            flags: 0x0800,
            crc_override: None,
            uncompressed_override: None,
            zip64_uncomp_sentinel: false,
            zip64_comp_sentinel: false,
            zip64_offset_sentinel: false,
            zip64_disk_sentinel: false,
            disk_start_override: 0,
            omit_zip64_extra: false,
            truncate_zip64_extra: false,
        }
    }

    fn deflated(name: &[u8], raw_payload: &[u8]) -> Self {
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
        enc.write_all(raw_payload).unwrap();
        let compressed = enc.finish().unwrap();
        Self {
            name: name.to_vec(),
            payload: compressed,
            method: 8,
            flags: 0x0800,
            crc_override: Some(crc32fast::hash(raw_payload)),
            uncompressed_override: Some(raw_payload.len() as u64),
            zip64_uncomp_sentinel: false,
            zip64_comp_sentinel: false,
            zip64_offset_sentinel: false,
            zip64_disk_sentinel: false,
            disk_start_override: 0,
            omit_zip64_extra: false,
            truncate_zip64_extra: false,
        }
    }
}

struct Zip64EocdSpec {
    write_zip64_eocd: bool,
    use_eocd_sentinels: bool,
    total_entries_override: Option<u64>,
    entries_disk_override: Option<u64>,
    cd_size_override: Option<u64>,
    cd_offset_override: Option<u64>,
    total_disks: u32,
    z64_disk: u32,
    std_eocd_total_override: Option<u16>,
}

impl Default for Zip64EocdSpec {
    fn default() -> Self {
        Self {
            write_zip64_eocd: true,
            use_eocd_sentinels: true,
            total_entries_override: None,
            entries_disk_override: None,
            cd_size_override: None,
            cd_offset_override: None,
            total_disks: 1,
            z64_disk: 0,
            std_eocd_total_override: None,
        }
    }
}

fn build_zip_with_zip64(entries: &[RawEntrySpec], eocd_spec: &Zip64EocdSpec) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd_records: Vec<Vec<u8>> = Vec::new();

    for entry in entries {
        let local_offset = out.len() as u64;
        let crc = entry.crc_override.unwrap_or_else(|| crc32fast::hash(&entry.payload));
        let comp_size = entry.payload.len() as u64;
        let uncomp_size = entry.uncompressed_override.unwrap_or(comp_size);

        // Local file header
        let local_comp_32 = if entry.zip64_comp_sentinel { u32::MAX } else { comp_size as u32 };
        let local_uncomp_32 = if entry.zip64_uncomp_sentinel { u32::MAX } else { uncomp_size as u32 };
        let mut local_extra = Vec::new();
        if entry.zip64_comp_sentinel || entry.zip64_uncomp_sentinel {
            local_extra.extend_from_slice(&0x0001_u16.to_le_bytes());
            local_extra.extend_from_slice(&16_u16.to_le_bytes());
            local_extra.extend_from_slice(&uncomp_size.to_le_bytes());
            local_extra.extend_from_slice(&comp_size.to_le_bytes());
        }
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        out.extend_from_slice(&45_u16.to_le_bytes());
        out.extend_from_slice(&entry.flags.to_le_bytes());
        out.extend_from_slice(&entry.method.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0x0021_u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&local_comp_32.to_le_bytes());
        out.extend_from_slice(&local_uncomp_32.to_le_bytes());
        out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&(local_extra.len() as u16).to_le_bytes());
        out.extend_from_slice(&entry.name);
        out.extend_from_slice(&local_extra);
        out.extend_from_slice(&entry.payload);

        // Central directory extra field (0x0001)
        let mut z64_body = Vec::new();
        if entry.zip64_uncomp_sentinel {
            z64_body.extend_from_slice(&uncomp_size.to_le_bytes());
        }
        if entry.zip64_comp_sentinel {
            z64_body.extend_from_slice(&comp_size.to_le_bytes());
        }
        if entry.zip64_offset_sentinel {
            z64_body.extend_from_slice(&local_offset.to_le_bytes());
        }
        if entry.zip64_disk_sentinel {
            z64_body.extend_from_slice(&entry.disk_start_override.to_le_bytes());
        }
        if entry.truncate_zip64_extra && !z64_body.is_empty() {
            z64_body.truncate(z64_body.len().saturating_sub(4));
        }

        let mut cd_extra = Vec::new();
        if !entry.omit_zip64_extra && !z64_body.is_empty() {
            // Prefix with an unrelated dummy extra field (0x5455) to verify scanning past other IDs.
            cd_extra.extend_from_slice(&0x5455_u16.to_le_bytes());
            cd_extra.extend_from_slice(&4_u16.to_le_bytes());
            cd_extra.extend_from_slice(&[1, 2, 3, 4]);
            cd_extra.extend_from_slice(&0x0001_u16.to_le_bytes());
            cd_extra.extend_from_slice(&(z64_body.len() as u16).to_le_bytes());
            cd_extra.extend_from_slice(&z64_body);
        }

        let cd_comp_32 = if entry.zip64_comp_sentinel { u32::MAX } else { comp_size as u32 };
        let cd_uncomp_32 = if entry.zip64_uncomp_sentinel { u32::MAX } else { uncomp_size as u32 };
        let cd_offset_32 = if entry.zip64_offset_sentinel { u32::MAX } else { local_offset as u32 };
        let cd_disk_16 = if entry.zip64_disk_sentinel { u16::MAX } else { entry.disk_start_override as u16 };

        let mut rec = Vec::new();
        rec.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        rec.extend_from_slice(&45_u16.to_le_bytes());
        rec.extend_from_slice(&45_u16.to_le_bytes());
        rec.extend_from_slice(&entry.flags.to_le_bytes());
        rec.extend_from_slice(&entry.method.to_le_bytes());
        rec.extend_from_slice(&0_u16.to_le_bytes());
        rec.extend_from_slice(&0x0021_u16.to_le_bytes());
        rec.extend_from_slice(&crc.to_le_bytes());
        rec.extend_from_slice(&cd_comp_32.to_le_bytes());
        rec.extend_from_slice(&cd_uncomp_32.to_le_bytes());
        rec.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        rec.extend_from_slice(&(cd_extra.len() as u16).to_le_bytes());
        rec.extend_from_slice(&0_u16.to_le_bytes());
        rec.extend_from_slice(&cd_disk_16.to_le_bytes());
        rec.extend_from_slice(&0_u16.to_le_bytes());
        rec.extend_from_slice(&0_u32.to_le_bytes());
        rec.extend_from_slice(&cd_offset_32.to_le_bytes());
        rec.extend_from_slice(&entry.name);
        rec.extend_from_slice(&cd_extra);
        cd_records.push(rec);
    }

    let cd_offset = out.len() as u64;
    for rec in &cd_records {
        out.extend_from_slice(rec);
    }
    let cd_size = (out.len() as u64) - cd_offset;
    let total_entries = eocd_spec.total_entries_override.unwrap_or(cd_records.len() as u64);
    let entries_disk = eocd_spec.entries_disk_override.unwrap_or(total_entries);
    let final_cd_size = eocd_spec.cd_size_override.unwrap_or(cd_size);
    let final_cd_offset = eocd_spec.cd_offset_override.unwrap_or(cd_offset);

    if eocd_spec.write_zip64_eocd {
        let z64_eocd_offset = out.len() as u64;
        // ZIP64 EOCD record (APPNOTE §4.3.14)
        out.extend_from_slice(&0x0606_4b50_u32.to_le_bytes());
        out.extend_from_slice(&44_u64.to_le_bytes());
        out.extend_from_slice(&45_u16.to_le_bytes());
        out.extend_from_slice(&45_u16.to_le_bytes());
        out.extend_from_slice(&eocd_spec.z64_disk.to_le_bytes());
        out.extend_from_slice(&0_u32.to_le_bytes());
        out.extend_from_slice(&entries_disk.to_le_bytes());
        out.extend_from_slice(&total_entries.to_le_bytes());
        out.extend_from_slice(&final_cd_size.to_le_bytes());
        out.extend_from_slice(&final_cd_offset.to_le_bytes());

        // ZIP64 EOCD locator (APPNOTE §4.3.15)
        out.extend_from_slice(&0x0706_4b50_u32.to_le_bytes());
        out.extend_from_slice(&eocd_spec.z64_disk.to_le_bytes());
        out.extend_from_slice(&z64_eocd_offset.to_le_bytes());
        out.extend_from_slice(&eocd_spec.total_disks.to_le_bytes());
    }

    // Standard EOCD (APPNOTE §4.3.16)
    let std_total = eocd_spec.std_eocd_total_override.unwrap_or(if eocd_spec.use_eocd_sentinels { u16::MAX } else { total_entries as u16 });
    let std_cd_size = if eocd_spec.use_eocd_sentinels { u32::MAX } else { final_cd_size as u32 };
    let std_cd_offset = if eocd_spec.use_eocd_sentinels { u32::MAX } else { final_cd_offset as u32 };

    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&std_total.to_le_bytes());
    out.extend_from_slice(&std_total.to_le_bytes());
    out.extend_from_slice(&std_cd_size.to_le_bytes());
    out.extend_from_slice(&std_cd_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out
}

#[test]
fn zip64_reading_supports_all_sentinel_combinations_and_validates_limits_before_allocating() {
    // 1. Valid ZIP64 archive exercising every 0x0001 extra field sentinel combination + DEFLATE & STORED.
    let e1 = RawEntrySpec {
        zip64_uncomp_sentinel: true,
        zip64_comp_sentinel: true,
        zip64_offset_sentinel: true,
        zip64_disk_sentinel: true,
        ..RawEntrySpec::deflated(b"data/sheet1.csv", b"x,y\n1,10\n2,20\n3,30\n4,40\n")
    };
    let e2 = RawEntrySpec { zip64_uncomp_sentinel: true, ..RawEntrySpec::stored(b"document.json", br#"{"sheets":{"data":["s1"]}}"#) };
    let e3 = RawEntrySpec { zip64_offset_sentinel: true, ..RawEntrySpec::stored(b"meta/info.txt", b"zip64-offset-only") };

    for use_eocd_sentinels in [true, false] {
        let zip_bytes = build_zip_with_zip64(
            &[RawEntrySpec::stored(b"a.txt", b"alpha")],
            &Zip64EocdSpec { write_zip64_eocd: true, use_eocd_sentinels, ..Zip64EocdSpec::default() },
        );
        let arc = ZipArchive::from_bytes(&zip_bytes).unwrap();
        assert_eq!(arc.get_utf8("a.txt").unwrap(), Some("alpha"));
    }

    let z64_bytes = build_zip_with_zip64(&[e1, e2, e3], &Zip64EocdSpec::default());
    let arc = ZipArchive::from_bytes(&z64_bytes).unwrap();
    assert_eq!(arc.get_utf8("data/sheet1.csv").unwrap(), Some("x,y\n1,10\n2,20\n3,30\n4,40\n"));
    assert_eq!(arc.get_utf8("document.json").unwrap(), Some(r#"{"sheets":{"data":["s1"]}}"#));
    assert_eq!(arc.get_utf8("meta/info.txt").unwrap(), Some("zip64-offset-only"));

    let mut idx = ZipIndex::from_bytes(&z64_bytes).unwrap();
    assert_eq!(idx.len(), 3);
    assert_eq!(idx.read_entry_utf8("meta/info.txt").unwrap().as_deref(), Some("zip64-offset-only"));

    // Single entry with 0x0001 extra field inside a standard non-ZIP64 EOCD archive
    let hybrid = build_zip_with_zip64(
        &[RawEntrySpec { zip64_uncomp_sentinel: true, zip64_comp_sentinel: true, ..RawEntrySpec::stored(b"h.txt", b"hybrid") }],
        &Zip64EocdSpec { write_zip64_eocd: false, use_eocd_sentinels: false, ..Zip64EocdSpec::default() },
    );
    assert_eq!(ZipArchive::from_bytes(&hybrid).unwrap().get_utf8("h.txt").unwrap(), Some("hybrid"));

    // 2. Security validation BEFORE allocating: forged u64::MAX entry count -> ZipBomb (zero OOM).
    let forged_count = build_zip_with_zip64(
        &[RawEntrySpec::stored(b"x.txt", b"x")],
        &Zip64EocdSpec { total_entries_override: Some(u64::MAX), ..Zip64EocdSpec::default() },
    );
    assert!(matches!(ZipArchive::from_bytes(&forged_count), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(ZipIndex::from_bytes(&forged_count), Err(FormatError::ZipBomb { .. })));

    // Forged entry count within max_entries (100) but CD size too small for 100 headers -> InvalidZip (zero over-alloc).
    let forged_cd_ratio = build_zip_with_zip64(
        &[RawEntrySpec::stored(b"x.txt", b"x")],
        &Zip64EocdSpec { total_entries_override: Some(100), ..Zip64EocdSpec::default() },
    );
    assert!(matches!(ZipArchive::from_bytes(&forged_cd_ratio), Err(FormatError::InvalidZip { .. })));

    // Forged 5 GiB uncompressed size in ZIP64 0x0001 extra field -> ZipBomb before allocating.
    let forged_5gib = build_zip_with_zip64(
        &[RawEntrySpec {
            uncompressed_override: Some(5 * 1024 * 1024 * 1024),
            zip64_uncomp_sentinel: true,
            ..RawEntrySpec::stored(b"huge.bin", b"tiny")
        }],
        &Zip64EocdSpec::default(),
    );
    assert!(matches!(ZipArchive::from_bytes(&forged_5gib), Err(FormatError::ZipBomb { .. })));

    // Forged CD offset / CD size overflow -> InvalidZip
    let forged_cd_size = build_zip_with_zip64(
        &[RawEntrySpec::stored(b"x.txt", b"x")],
        &Zip64EocdSpec { cd_size_override: Some(u64::MAX), ..Zip64EocdSpec::default() },
    );
    assert!(matches!(ZipArchive::from_bytes(&forged_cd_size), Err(FormatError::InvalidZip { .. })));

    // Missing or truncated 0x0001 extra field -> InvalidZip
    let missing_extra = build_zip_with_zip64(
        &[RawEntrySpec { zip64_uncomp_sentinel: true, omit_zip64_extra: true, ..RawEntrySpec::stored(b"m.txt", b"m") }],
        &Zip64EocdSpec::default(),
    );
    assert!(matches!(ZipArchive::from_bytes(&missing_extra), Err(FormatError::InvalidZip { .. })));

    let trunc_extra = build_zip_with_zip64(
        &[RawEntrySpec { zip64_uncomp_sentinel: true, truncate_zip64_extra: true, ..RawEntrySpec::stored(b"t.txt", b"t") }],
        &Zip64EocdSpec::default(),
    );
    assert!(matches!(ZipArchive::from_bytes(&trunc_extra), Err(FormatError::InvalidZip { .. })));

    // Multi-disk ZIP64 and inconsistent standard/ZIP64 EOCD -> InvalidZip
    let multi_disk = build_zip_with_zip64(&[RawEntrySpec::stored(b"x.txt", b"x")], &Zip64EocdSpec { total_disks: 2, ..Zip64EocdSpec::default() });
    assert!(matches!(ZipArchive::from_bytes(&multi_disk), Err(FormatError::InvalidZip { .. })));

    let inconsistent_eocd = build_zip_with_zip64(
        &[RawEntrySpec::stored(b"x.txt", b"x")],
        &Zip64EocdSpec { use_eocd_sentinels: false, std_eocd_total_override: Some(9), ..Zip64EocdSpec::default() },
    );
    assert!(matches!(ZipArchive::from_bytes(&inconsistent_eocd), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn zip_external_interop_normalizes_paths_and_rejects_hostile_paths() {
    let interop = ZipReadOptions::external_interop();

    assert_eq!(sanitize_zip_path_with_options(r"dir\sub\file.csv", interop).unwrap(), "dir/sub/file.csv");
    assert_eq!(sanitize_zip_path_with_options("./dir/./file.csv", interop).unwrap(), "dir/file.csv");
    assert_eq!(sanitize_zip_path_with_options(r".\dir\.\sub\file.csv", interop).unwrap(), "dir/sub/file.csv");
    assert_eq!(sanitize_zip_path_with_options(r"dir\sub\", interop).unwrap(), "dir/sub/");
    assert_eq!(sanitize_zip_path_with_options("./dir/./", interop).unwrap(), "dir/");

    assert!(matches!(sanitize_zip_path(r"dir\file.csv"), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(sanitize_zip_path("./file.csv"), Err(FormatError::UnsafePath { .. })));

    for bad in [
        "",
        ".",
        "./",
        "dir/.",
        "..",
        "../secret.txt",
        "dir/../secret.txt",
        r"dir\..\secret.txt",
        "./../secret.txt",
        "/etc/passwd",
        r"\Windows\System32",
        r"\\server\share\file",
        "C:/Windows/win.ini",
        r"C:\Windows\win.ini",
        "./C:/Windows/win.ini",
        "a//b",
        r"a\\b",
        "a\0b",
    ] {
        assert!(matches!(sanitize_zip_path_with_options(bad, interop), Err(FormatError::UnsafePath { .. })), "interop must reject `{bad:?}`");
    }
}

#[test]
fn zip_external_interop_rejects_collisions_and_decodes_cp437() {
    let interop = ZipReadOptions::external_interop();
    let plain_eocd = Zip64EocdSpec { write_zip64_eocd: false, use_eocd_sentinels: false, ..Zip64EocdSpec::default() };

    let dup_slash = build_zip_with_zip64(&[RawEntrySpec::stored(br"a\b.csv", b"1"), RawEntrySpec::stored(b"a/b.csv", b"2")], &plain_eocd);
    assert!(matches!(ZipArchive::from_bytes(&dup_slash), Err(FormatError::UnsafePath { .. })));
    assert!(matches!(ZipArchive::from_bytes_with_options(&dup_slash, ZipLimits::default(), interop), Err(FormatError::DuplicateEntry { .. })));
    assert!(matches!(ZipIndex::from_bytes_with_options(&dup_slash, ZipLimits::default(), interop), Err(FormatError::DuplicateEntry { .. })));

    let dup_dot = build_zip_with_zip64(&[RawEntrySpec::stored(b"./a/b.csv", b"1"), RawEntrySpec::stored(b"a/b.csv", b"2")], &plain_eocd);
    assert!(matches!(ZipArchive::from_bytes_with_options(&dup_dot, ZipLimits::default(), interop), Err(FormatError::DuplicateEntry { .. })));

    let case_dup =
        build_zip_with_zip64(&[RawEntrySpec::stored(b"Data/Sheet.json", b"1"), RawEntrySpec::stored(br"data\sheet.json", b"2")], &plain_eocd);
    assert!(ZipArchive::from_bytes_with_options(&case_dup, ZipLimits::default(), interop).is_ok());
    let ci_opts = interop.with_reject_case_folded_duplicates(true);
    assert!(matches!(ZipArchive::from_bytes_with_options(&case_dup, ZipLimits::default(), ci_opts), Err(FormatError::DuplicateEntry { .. })));
    assert!(matches!(ZipIndex::from_bytes_with_options(&case_dup, ZipLimits::default(), ci_opts), Err(FormatError::DuplicateEntry { .. })));

    assert_eq!(decode_cp437(b"caf\x82_\xE6M_\xF11_\x8E\x99\x9A.csv"), "café_µM_±1_ÄÖÜ.csv");
    let cp437_entry = RawEntrySpec { flags: 0, ..RawEntrySpec::stored(b"data\\caf\x82_\xE6M.csv", b"1,2\n") };
    let utf8_no_flag = RawEntrySpec { flags: 0, ..RawEntrySpec::stored("data/naïve.csv".as_bytes(), b"3,4\n") };
    let cp437_zip = build_zip_with_zip64(&[cp437_entry, utf8_no_flag], &plain_eocd);
    assert!(matches!(ZipArchive::from_bytes(&cp437_zip), Err(FormatError::InvalidZip { .. })));
    let arc = ZipArchive::from_bytes_with_options(&cp437_zip, ZipLimits::default(), interop).unwrap();
    assert_eq!(arc.get_utf8("data/café_µM.csv").unwrap(), Some("1,2\n"));
    assert_eq!(arc.get_utf8("data/naïve.csv").unwrap(), Some("3,4\n"));

    let bad_utf8_flag = build_zip_with_zip64(&[RawEntrySpec { flags: 0x0800, ..RawEntrySpec::stored(b"caf\x82.csv", b"1") }], &plain_eocd);
    assert!(matches!(ZipArchive::from_bytes_with_options(&bad_utf8_flag, ZipLimits::default(), interop), Err(FormatError::InvalidZip { .. })));
}

#[test]
fn zip_index_is_lazy_and_inspects_unread_classes() {
    let huge_unread_payload = vec![b'P'; 20_000];
    let lying_bomb_raw = vec![b'Z'; 50_000];
    let mut bomb_enc = DeflateEncoder::new(Vec::new(), Compression::best());
    bomb_enc.write_all(&lying_bomb_raw).unwrap();
    let bomb_compressed = bomb_enc.finish().unwrap();

    let entries = [
        RawEntrySpec::deflated(b"document.json", br#"{"title":"Assay"}"#),
        RawEntrySpec::stored(b"data/s1.csv", b"1,2\n3,4\n"),
        RawEntrySpec::deflated(b"previews/graph1.png", &huge_unread_payload),
        RawEntrySpec { crc_override: Some(0xDEAD_BEEF), ..RawEntrySpec::stored(b"cache/render.bin", b"corrupt") },
        RawEntrySpec { method: 99, ..RawEntrySpec::stored(b"db/store.sqlite3", b"sqlite") },
        RawEntrySpec {
            name: b"bomb.bin".to_vec(),
            payload: bomb_compressed,
            method: 8,
            flags: 0x0800,
            crc_override: Some(crc32fast::hash(&lying_bomb_raw)),
            uncompressed_override: Some(10),
            zip64_uncomp_sentinel: false,
            zip64_comp_sentinel: false,
            zip64_offset_sentinel: false,
            zip64_disk_sentinel: false,
            disk_start_override: 0,
            omit_zip64_extra: false,
            truncate_zip64_extra: false,
        },
    ];
    let zip_bytes = build_zip_with_zip64(&entries, &Zip64EocdSpec::default());

    let tight = ZipLimits { max_entries: 16, max_total_uncompressed_bytes: 100, max_entry_uncompressed_bytes: 64 };
    assert!(matches!(ZipArchive::from_bytes_with_limits(&zip_bytes, tight), Err(FormatError::ZipBomb { .. })));

    let mut idx = ZipIndex::from_bytes_with_options(&zip_bytes, tight, ZipReadOptions::external_interop()).unwrap();
    assert_eq!((idx.len(), idx.bytes_read(), idx.remaining_budget()), (6, 0, 100));
    assert_eq!(idx.read_entry_utf8("document.json").unwrap().as_deref(), Some(r#"{"title":"Assay"}"#));
    assert_eq!(idx.read_entry_utf8("data/s1.csv").unwrap().as_deref(), Some("1,2\n3,4\n"));
    assert!(idx.is_read("document.json") && idx.is_read("data/s1.csv") && !idx.is_read("previews/graph1.png"));
    assert_eq!((idx.bytes_read(), idx.remaining_budget()), (25, 75));

    let unread_names: Vec<&str> = idx.unread_entry_names().collect();
    assert_eq!(unread_names, vec!["bomb.bin", "cache/render.bin", "db/store.sqlite3", "previews/graph1.png"]);
    assert_eq!(
        idx.unread_entry_counts_by_class(),
        BTreeMap::from([(ZipEntryClass::Preview, 1), (ZipEntryClass::Cache, 1), (ZipEntryClass::Database, 1), (ZipEntryClass::Other, 1)])
    );
    assert_eq!(classify_zip_entry_name("thumbs/fig.jpg"), ZipEntryClass::Preview);
    assert_eq!(classify_zip_entry_name("foo/data.sqlite"), ZipEntryClass::Database);
    assert_eq!(classify_zip_entry_name("tmp/calc.cache"), ZipEntryClass::Cache);

    assert!(matches!(idx.read_entry("previews/graph1.png"), Err(FormatError::ZipBomb { .. })));
    assert!(matches!(idx.read_entry("cache/render.bin"), Err(FormatError::CorruptCrc { .. })));
    assert!(matches!(idx.read_entry("db/store.sqlite3"), Err(FormatError::UnsupportedCompression { method: 99, .. })));
    assert!(matches!(idx.read_entry("bomb.bin"), Err(FormatError::ZipBomb { .. })));
}

#[test]
fn zip_index_enforces_cumulative_read_budget_and_entry_count_cap() {
    let plain_eocd = Zip64EocdSpec { write_zip64_eocd: false, use_eocd_sentinels: false, ..Zip64EocdSpec::default() };
    let budget_zip = build_zip_with_zip64(
        &[
            RawEntrySpec::stored(b"a.txt", b"0123456789"),
            RawEntrySpec::stored(b"b.txt", b"0123456789"),
            RawEntrySpec::stored(b"c.txt", b"0123456789"),
        ],
        &plain_eocd,
    );
    let mut budget_idx = ZipIndex::from_bytes_with_limits(
        &budget_zip,
        ZipLimits { max_entries: 8, max_total_uncompressed_bytes: 15, max_entry_uncompressed_bytes: 15 },
    )
    .unwrap();
    assert_eq!(budget_idx.read_entry("a.txt").unwrap().unwrap().len(), 10);
    assert!(matches!(budget_idx.read_entry("b.txt"), Err(FormatError::ZipBomb { .. })));

    let tiny_count = ZipLimits { max_entries: 2, max_total_uncompressed_bytes: 1024, max_entry_uncompressed_bytes: 1024 };
    assert!(matches!(ZipIndex::from_bytes_with_limits(&budget_zip, tiny_count), Err(FormatError::ZipBomb { .. })));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn prop_xml_writer_roundtrips_valid_xml10_text_and_attr_order(
        text in "[a-zA-Z0-9 &<>\"'\\t\\n\\r\\u{00A0}-\\u{03CF}]{0,80}",
        v1 in "[a-zA-Z0-9 &<>\"'\\t\\n\\r]{0,30}",
        v2 in "[a-zA-Z0-9 &<>\"'\\t\\n\\r]{0,30}",
    ) {
        prop_assert!(text.chars().all(is_valid_xml10_char));
        let mut w = XmlWriter::new();
        w.write_xml_declaration().unwrap();
        w.start_element("Root", &[("zeta", &v1), ("alpha", &v2)]).unwrap();
        w.text(&text).unwrap();
        w.end_element().unwrap();
        let serialized = w.finish().unwrap();

        let parsed = parse_xml(&serialized).unwrap();
        prop_assert_eq!(parsed.attrs.len(), 2);
        prop_assert_eq!(&parsed.attrs[0].0, "zeta");
        prop_assert_eq!(&parsed.attrs[0].1, &v1);
        prop_assert_eq!(&parsed.attrs[1].0, "alpha");
        prop_assert_eq!(&parsed.attrs[1].1, &v2);
        // Normalize bare `\r\n` to `\n` only if any; XmlWriter escapes `\r` as `&#xD;` so exact match holds!
        let expected_children = if text.is_empty() { vec![] } else { vec![XmlNode::Text(text.clone())] };
        prop_assert_eq!(&parsed.children, &expected_children);
        prop_assert_eq!(parsed.plain_text(), text);
    }

    #[test]
    fn prop_codecs_never_panic_on_arbitrary_bytes(raw in collection::vec(any::<u8>(), 0..512)) {
        let stop_opts = XmlParseOptions::default().with_stop_after_root(true).with_opaque_elements(["Blob"]);
        let interop_zip = ZipReadOptions::external_interop().with_reject_case_folded_duplicates(true);
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = detect_xml_encoding(&raw);
            let _ = parse_xml_bytes(&raw);
            let _ = parse_xml_bytes_with_options(&raw, &stop_opts);
            let _ = ZipArchive::from_bytes(&raw);
            let _ = ZipArchive::from_bytes_with_options(&raw, ZipLimits::default(), interop_zip);
            if let Ok(mut idx) = ZipIndex::from_bytes_with_options(&raw, ZipLimits::default(), interop_zip) {
                let first = idx.entry_names().next().map(str::to_owned);
                if let Some(name) = first {
                    let _ = idx.read_entry(&name);
                }
                let _ = idx.unread_entry_counts_by_class();
            }
        }));
        prop_assert!(outcome.is_ok(), "XML/ZIP codecs panicked on arbitrary byte input");
    }

    #[test]
    fn prop_zip_interop_never_accepts_traversal_absolute_drive_or_nul(
        prefix in "[a-zA-Z0-9._-]{0,8}",
        suffix in "[a-zA-Z0-9._-]{1,8}",
    ) {
        let interop = ZipReadOptions::external_interop();
        let hostile = [
            format!("/{suffix}"),
            format!("\\{suffix}"),
            format!("C:/{suffix}"),
            format!("./C:/{suffix}"),
            format!("{prefix}/../{suffix}"),
            format!("{prefix}\\..\\{suffix}"),
            format!("{prefix}\0{suffix}"),
        ];
        for path in hostile {
            prop_assert!(matches!(sanitize_zip_path(&path), Err(FormatError::UnsafePath { .. })), "strict {path:?}");
            prop_assert!(matches!(sanitize_zip_path_with_options(&path, interop), Err(FormatError::UnsafePath { .. })), "interop {path:?}");
        }
    }
}

#[test]
fn xml_writer_write_element_handles_empty_and_mixed_dom() {
    let dom = XmlElement {
        name: String::from("Root"),
        attrs: vec![(String::from("b"), String::from("2")), (String::from("a"), String::from("1"))],
        children: vec![
            XmlNode::Text(String::from("lead ")),
            XmlNode::Element(XmlElement { name: String::from("Tag"), attrs: vec![], children: vec![] }),
            XmlNode::Text(String::from(" tail")),
        ],
    };
    let mut w = XmlWriter::with_indent("  ").unwrap();
    w.write_element(&dom).unwrap();
    let out = w.finish().unwrap();
    assert_eq!(out, "<Root b=\"2\" a=\"1\">lead <Tag/> tail</Root>");
    assert_eq!(parse_xml(&out).unwrap(), dom);
}
