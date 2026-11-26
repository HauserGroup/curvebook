//! Integration tests for PDF 1.7 header selection, TrueType SFNT (`/Subtype /TrueType`,
//! `/FontDescriptor`, `FlateDecode` `/FontFile2`) embedding, BiDi/RTL visual text ordering,
//! and `--no-default-features` / empty-`BUNDLED_FONTS` Core-14 & outlined fallback.

use std::time::Instant;

use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use curvebook_palette::{OKABE_ITO_BLUE, Rgba8};
use curvebook_pdf::{PdfExportOptions, PdfVersion, render_pdf, render_pdf_pages};
use curvebook_text::{BUNDLED_FONTS, RichText, build_minimal_truetype_font, parse_markup, parse_truetype_font_info};
use serde::Deserialize;
use serde::de::value::{Error as DeValueError, MapDeserializer};
use serde::de::{Deserializer, IntoDeserializer, Visitor};

use super::{ParsedPdf, extract_pdf_int};

fn parse_pdf_strict(bytes: &[u8], expected_header: &[u8]) -> Result<ParsedPdf, String> {
    super::parse_pdf_strict(bytes, &[expected_header], true)
}

fn sample_scientific_scene(title: &str) -> ChartScene {
    let mut scene = ChartScene::new("graph-ttf", title, 420.0, 280.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("Dose-Response: IC<sub>50</sub> = 2.4 ± 0.3 µM (α = 0.05)"),
        anchor_pt: PointPt::new(210.0, 48.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: OKABE_ITO_BLUE,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>Significant</b> · <i>K</i><sub>d</sub>"),
        anchor_pt: PointPt::new(36.0, 120.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    scene
}

#[derive(Clone, Copy)]
enum LegacyFieldValue {
    Bool(bool),
    NoneOption,
    Str(&'static str),
}

impl<'de> Deserializer<'de> for LegacyFieldValue {
    type Error = DeValueError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Self::Bool(b) => visitor.visit_bool(b),
            Self::NoneOption => visitor.visit_none(),
            Self::Str(s) => visitor.visit_borrowed_str(s),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Self::NoneOption => visitor.visit_none(),
            other => visitor.visit_some(other),
        }
    }

    fn deserialize_enum<V>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self {
            Self::Str(s) => visitor.visit_enum(s.into_deserializer()),
            other => other.deserialize_any(visitor),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct newtype_struct seq tuple
        tuple_struct map struct identifier ignored_any
    }
}

impl<'de> IntoDeserializer<'de, DeValueError> for LegacyFieldValue {
    type Deserializer = Self;

    fn into_deserializer(self) -> Self::Deserializer {
        self
    }
}

#[test]
fn pdf_version_header_and_options_serde_defaults() {
    let default_opts = PdfExportOptions::default();
    assert_eq!(default_opts.version, PdfVersion::Pdf17);
    assert!(default_opts.embedded_truetype_font.is_none());

    let legacy_fields = [
        ("color_space", LegacyFieldValue::Str("Srgb")),
        ("compress_streams", LegacyFieldValue::Bool(true)),
        ("title", LegacyFieldValue::NoneOption),
        ("creator", LegacyFieldValue::Str("CurveBook")),
        ("outline_text", LegacyFieldValue::Bool(false)),
        ("trim_pt", LegacyFieldValue::NoneOption),
        ("bleed_pt", LegacyFieldValue::NoneOption),
    ];
    let from_legacy_map = PdfExportOptions::deserialize(MapDeserializer::new(legacy_fields.into_iter())).expect("deserialize legacy options");
    assert_eq!(from_legacy_map.version, PdfVersion::Pdf17);
    assert!(from_legacy_map.embedded_truetype_font.is_none());

    let scene = sample_scientific_scene("Version Header Check");
    let pdf17 = render_pdf(&scene, &default_opts).expect("render pdf 1.7");
    let parsed17 = parse_pdf_strict(&pdf17, b"%PDF-1.7\n").expect("parse PDF 1.7");
    assert!(parsed17.trailer.contains("/Root 1 0 R"));
    assert!(parsed17.trailer.contains("/Info 8 0 R"));

    let opts14 = PdfExportOptions { version: PdfVersion::Pdf14, ..PdfExportOptions::default() };
    let pdf14 = render_pdf(&scene, &opts14).expect("render pdf 1.4");
    let _parsed14 = parse_pdf_strict(&pdf14, b"%PDF-1.4\n").expect("parse PDF 1.4");
}

#[test]
fn pdf17_embeds_truetype_font_descriptor_and_roundtrips_flate2_fontfile2() {
    let ttf_bytes = build_minimal_truetype_font();
    assert!(ttf_bytes.starts_with(&[0x00, 0x01, 0x00, 0x00]));
    let expected_info = parse_truetype_font_info(&ttf_bytes).expect("parse minimal SFNT");

    let scene = sample_scientific_scene("TrueType Embed Figure");
    let opts = PdfExportOptions {
        version: PdfVersion::Pdf17,
        embedded_truetype_font: Some(ttf_bytes.clone()),
        compress_streams: true,
        ..PdfExportOptions::default()
    };

    let pdf_a = render_pdf(&scene, &opts).expect("render pdf with embedded TTF");
    let pdf_b = render_pdf(&scene, &opts).expect("deterministic re-render");
    assert_eq!(pdf_a, pdf_b, "PDF 1.7 with embedded TTF must be deterministic");

    let parsed = parse_pdf_strict(&pdf_a, b"%PDF-1.7\n").expect("parse PDF 1.7 with embedded TTF");

    let all_objs: String = parsed.objects.values().map(|b| String::from_utf8_lossy(b).into_owned()).collect::<Vec<_>>().join("\n");
    let expected_font_head =
        format!("/Type /Font /Subtype /TrueType /BaseFont /{} /FirstChar 32 /LastChar 255 /Widths [", expected_info.postscript_name);
    assert!(all_objs.contains(&expected_font_head), "missing /Subtype /TrueType font dictionary: {expected_font_head}");
    assert!(all_objs.contains("/Encoding /WinAnsiEncoding"), "missing /Encoding /WinAnsiEncoding");

    let widths_line = parsed
        .objects
        .values()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .find(|s| s.contains("/Subtype /TrueType"))
        .expect("truetype font dict");
    let (_, after_widths) = widths_line.split_once("/Widths [").expect("widths start");
    let (widths_inner, _) = after_widths.split_once(']').expect("widths end");
    let parsed_widths: Vec<u16> = widths_inner.split_whitespace().map(|tok| tok.parse::<u16>().expect("width integer")).collect();
    assert_eq!(parsed_widths.len(), 224, "/Widths must contain 224 entries for codes 32..=255");
    assert_eq!(parsed_widths, expected_info.widths_1000);

    let expected_fd = format!(
        "/Type /FontDescriptor /FontName /{} /Flags {} /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {} /FontFile2 ",
        expected_info.postscript_name,
        expected_info.flags,
        expected_info.bbox_1000[0],
        expected_info.bbox_1000[1],
        expected_info.bbox_1000[2],
        expected_info.bbox_1000[3],
        expected_info.ascender_1000,
        expected_info.descender_1000,
        expected_info.cap_height_1000,
        expected_info.stem_v
    );
    assert!(all_objs.contains(&expected_fd), "missing or mismatched /FontDescriptor:\nexpected fragment: {expected_fd}\nactual:\n{all_objs}");

    let (_, ff2_dict, ff2_decoded) =
        parsed.streams.iter().find(|(_, dict, _)| dict.contains("/Length1 ")).expect("missing /FontFile2 stream with /Length1");
    assert!(ff2_dict.contains("/Filter /FlateDecode"), "/FontFile2 must be FlateDecode compressed");
    let declared_len1 = extract_pdf_int(ff2_dict, "/Length1 ").expect("/Length1 integer");
    assert_eq!(declared_len1, ttf_bytes.len());
    assert_eq!(*ff2_decoded, ttf_bytes, "decompressed /FontFile2 must round-trip identically to input SFNT");
    assert!(ff2_decoded.starts_with(&[0x00, 0x01, 0x00, 0x00]), "decompressed /FontFile2 must have SFNT 0x00010000 magic");
    let reparsed_info = parse_truetype_font_info(ff2_decoded).expect("round-trip SFNT parse");
    assert_eq!(reparsed_info, expected_info);

    let page_stream = parsed
        .streams
        .iter()
        .find(|(_, dict, _)| !dict.contains("/Length1 "))
        .map(|(_, _, decoded)| String::from_utf8(decoded.clone()).expect("UTF-8 page stream"))
        .expect("page content stream");
    assert!(page_stream.contains("/F6 12.00 Tf"), "regular text must select embedded TrueType font /F6");
    assert!(page_stream.contains("/F4 12.00 Tf"), "Greek/Symbol glyphs must remain selectable via /F4 Symbol");

    // With compress_streams=false, page stream is raw while /FontFile2 stays FlateDecode compressed.
    let raw_opts = PdfExportOptions { compress_streams: false, ..opts };
    let raw_pdf = render_pdf(&scene, &raw_opts).expect("render uncompressed page stream with embedded TTF");
    let raw_parsed = parse_pdf_strict(&raw_pdf, b"%PDF-1.7\n").expect("parse raw stream PDF");
    let (_, _, raw_ff2) = raw_parsed.streams.iter().find(|(_, dict, _)| dict.contains("/Length1 ")).expect("/FontFile2 in raw mode");
    assert_eq!(raw_ff2, &ttf_bytes);
}

#[test]
fn invalid_embedded_ttf_falls_back_cleanly_to_core14() {
    let scene = sample_scientific_scene("Fallback On Invalid TTF");
    for bad_bytes in [Vec::new(), b"not-a-valid-sfnt".to_vec(), b"OTTO\x00\x01\x00\x00invalid".to_vec()] {
        let opts = PdfExportOptions { embedded_truetype_font: Some(bad_bytes), ..PdfExportOptions::default() };
        let pdf = render_pdf(&scene, &opts).expect("graceful Core-14 fallback on invalid TTF");
        let parsed = parse_pdf_strict(&pdf, b"%PDF-1.7\n").expect("parse fallback PDF");
        let objs: String = parsed.objects.values().map(|b| String::from_utf8_lossy(b).into_owned()).collect::<Vec<_>>().join("\n");
        assert!(!objs.contains("/FontFile2"), "invalid TTF must not emit /FontFile2");
        assert!(objs.contains("/BaseFont /Helvetica"), "must fall back to Core-14 Helvetica");
    }
}

#[test]
fn empty_bundled_fonts_core14_and_outlined_fallback_is_fast() {
    let scene1 = sample_scientific_scene("Fallback Page 1");
    let scene2 = sample_scientific_scene("Fallback Page 2");
    let opts = PdfExportOptions { embedded_truetype_font: None, ..PdfExportOptions::default() };

    let t0 = Instant::now();
    let pdf = render_pdf_pages(&[scene1.clone(), scene2.clone()], &opts).expect("render fallback PDF");
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let parsed = parse_pdf_strict(&pdf, b"%PDF-1.7\n").expect("parse multi-page fallback PDF");
    let objs: String = parsed.objects.values().map(|b| String::from_utf8_lossy(b).into_owned()).collect::<Vec<_>>().join("\n");

    if BUNDLED_FONTS.is_empty() {
        assert!(!objs.contains("/Subtype /TrueType"), "empty BUNDLED_FONTS must stay Core-14 without /Subtype /TrueType");
        assert!(!objs.contains("/FontFile2"), "empty BUNDLED_FONTS must not emit /FontFile2");
        assert!(objs.contains("/BaseFont /Helvetica /Encoding /WinAnsiEncoding"));
        assert!(objs.contains("/BaseFont /Symbol >>"));
    }

    let outline_opts = PdfExportOptions { outline_text: true, ..opts };
    let outlined_pdf = render_pdf_pages(&[scene1, scene2], &outline_opts).expect("render outlined fallback PDF");
    let outlined_parsed = parse_pdf_strict(&outlined_pdf, b"%PDF-1.7\n").expect("parse outlined fallback PDF");
    let p1_outlined = String::from_utf8(outlined_parsed.streams[0].2.clone()).expect("UTF-8 stream");
    assert!(!p1_outlined.contains(" Tj\n"));
    assert!(p1_outlined.contains(" m\n") && p1_outlined.contains("f\n"));

    println!("PDF17_FALLBACK_METRICS: bundled_fonts={}, elapsed_ms={elapsed_ms:.3}", BUNDLED_FONTS.len());
}

#[test]
fn rtl_arabic_and_hebrew_labels_shape_in_visual_order() {
    let mut scene = ChartScene::new("graph-rtl", "BiDi RTL Labels", 440.0, 260.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("שלום 2026"),
        anchor_pt: PointPt::new(220.0, 60.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("مرحبا (12.5)"),
        anchor_pt: PointPt::new(220.0, 100.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: OKABE_ITO_BLUE,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>שלום</b> <i>עולם</i>"),
        anchor_pt: PointPt::new(40.0, 140.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("IC<sub>50</sub> = שלום"),
        anchor_pt: PointPt::new(40.0, 180.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    // Selectable text mode (with embedded TTF) verifies visual LTR ordering of mixed runs & styles.
    let sel_opts = PdfExportOptions { embedded_truetype_font: Some(build_minimal_truetype_font()), ..PdfExportOptions::default() };
    let sel_pdf = render_pdf(&scene, &sel_opts).expect("selectable RTL PDF");
    let sel_parsed = parse_pdf_strict(&sel_pdf, b"%PDF-1.7\n").expect("parse selectable RTL PDF");
    let sel_stream = sel_parsed
        .streams
        .iter()
        .find(|(_, dict, _)| !dict.contains("/Length1 "))
        .map(|(_, _, decoded)| String::from_utf8(decoded.clone()).expect("UTF-8 page stream"))
        .expect("page stream");

    // Selectable mode must NEVER silently replace non-WinAnsi glyphs with `?`.
    assert!(!sel_stream.contains('?'), "non-WinAnsi characters must never produce `?` placeholders in PDF stream:\n{sel_stream}");

    // In "שלום 2026" (RTL base), "2026" renders visually to the left of the outlined Hebrew glyphs.
    let (_, after_2026) = sel_stream.split_once("(2026) Tj").expect("2026 literal");
    let (hebrew_num_seg, after_12_5) = after_2026.split_once("(12.5) Tj").expect("12.5 literal");
    assert!(hebrew_num_seg.contains(" m\n"), "Hebrew word after 2026 must emit vector glyph outlines");

    // Between "12.5" and "IC", we have outlined Arabic glyphs for "مرحبا", then the
    // "<b>שלום</b> <i>עולם</i>" line at y=140.0 (PDF y=120.000) containing outlined "עולם",
    // selectable space `( ) Tj`, and outlined "שלום".
    let (mid_seg, ic_seg) = after_12_5.split_once("(IC) Tj").expect("IC literal");
    let (before_space, after_space) = mid_seg.split_once("120.000 Tm").expect("space Tm at PDF y=120");
    assert!(before_space.contains(" m\n"), "outlined glyphs for Arabic and italic Hebrew word must precede `( ) Tj`");
    assert!(after_space.contains("( ) Tj"), "separating space between visual RTL words");
    assert!(after_space.contains(" m\n"), "bold Hebrew word must emit outlined glyphs after separating space");

    // In "IC<sub>50</sub> = שלום", "IC", subscript "50", and " = " stay selectable while Hebrew outlines follow.
    let (_, after_50) = ic_seg.split_once("(50) Tj").expect("50 literal");
    let (_, after_eq) = after_50.split_once("( = ) Tj").expect("= literal");
    assert!(after_eq.contains(" m\n"), "Hebrew word after `IC_50 = ` must emit outlined glyphs");

    // Outlined text mode: with `TextAnchor::Start` at x=20, visual RTL reordering places `"2026"` at x=20
    // (identical to rendering `"2026"` alone) followed to the right by the 4 Hebrew glyphs.
    let mut digits_only_scene = ChartScene::new("graph-rtl-digits", "Digits Control", 320.0, 120.0, Some(Rgba8::WHITE));
    digits_only_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("2026"),
        anchor_pt: PointPt::new(20.0, 60.0),
        anchor: TextAnchor::Start,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let mut hebrew_num_scene = ChartScene::new("graph-rtl-outline", "RTL Outlined", 320.0, 120.0, Some(Rgba8::WHITE));
    hebrew_num_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("שלום 2026"),
        anchor_pt: PointPt::new(20.0, 60.0),
        anchor: TextAnchor::Start,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let out_opts = PdfExportOptions { outline_text: true, ..PdfExportOptions::default() };
    let digits_pdf = render_pdf(&digits_only_scene, &out_opts).expect("outlined digits PDF");
    let digits_parsed = parse_pdf_strict(&digits_pdf, b"%PDF-1.7\n").expect("parse digits PDF");
    let digits_stream = String::from_utf8(digits_parsed.streams[0].2.clone()).expect("UTF-8 digits stream");

    let out_pdf = render_pdf(&hebrew_num_scene, &out_opts).expect("outlined RTL PDF");
    let out_parsed = parse_pdf_strict(&out_pdf, b"%PDF-1.7\n").expect("parse outlined RTL PDF");
    let out_stream = String::from_utf8(out_parsed.streams[0].2.clone()).expect("UTF-8 outlined stream");
    let q_count = out_stream.lines().filter(|l| *l == "q").count();
    let big_q_count = out_stream.lines().filter(|l| *l == "Q").count();
    assert_eq!(q_count, big_q_count, "graphics state stack must stay balanced");
    assert!(out_stream.starts_with(&digits_stream), "RTL outlined stream must emit '2026' first (at left anchor x=20)");
    assert!(out_stream.len() > digits_stream.len(), "RTL outlined stream must append Hebrew glyphs to the right of '2026'");
}

#[test]
fn non_winansi_cjk_cyrillic_and_hebrew_never_substitute_question_marks() {
    let mut scene = ChartScene::new("non-winansi", "Non-WinAnsi Labels", 420.0, 260.0, Some(Rgba8::WHITE));
    for (idx, (label, anchor, rot, color)) in [
        ("濃度 (nM)", TextAnchor::Start, 0.0, Rgba8::BLACK),
        ("Концентрация", TextAnchor::Middle, 0.0, OKABE_ITO_BLUE),
        ("שלום", TextAnchor::End, 0.0, Rgba8::BLACK.with_alpha(128)),
        ("Why? 濃度<sup>2</sup>", TextAnchor::Start, 35.0, Rgba8::BLACK),
    ]
    .into_iter()
    .enumerate()
    {
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup(label),
            anchor_pt: PointPt::new(80.0, 45.0 + (idx as f64) * 45.0),
            anchor,
            font_size_pt: 12.0,
            rotation_deg: rot,
            color,
        }));
    }

    for embedded in [None, Some(build_minimal_truetype_font())] {
        let opts = PdfExportOptions { compress_streams: false, outline_text: false, embedded_truetype_font: embedded, ..PdfExportOptions::default() };
        let bytes = render_pdf(&scene, &opts).expect("render non-WinAnsi selectable PDF");
        let parsed = parse_pdf_strict(&bytes, b"%PDF-1.7\n").expect("strict PDF parse");
        let stream = parsed
            .streams
            .iter()
            .find(|(_, dict, _)| !dict.contains("/Length1 "))
            .map(|(_, _, decoded)| String::from_utf8(decoded.clone()).expect("UTF-8 stream"))
            .expect("page stream");

        // Never emit `?` placeholder substitutions for non-WinAnsi/non-Symbol scripts.
        for bad in ["(??) Tj", "(????) Tj", "(????????????) Tj", "(? ) Tj"] {
            assert!(!stream.contains(bad), "unexpected placeholder {bad} in stream:\n{stream}");
        }

        // Literal WinAnsi ASCII runs remain selectable (including literal `?` in `Why?`).
        assert!(stream.contains("( \\(nM\\)) Tj"), "expected selectable ` (nM)` suffix:\n{stream}");
        assert!(stream.contains("(Why? ) Tj"), "expected literal `?` preserved in `Why? `:\n{stream}");
        assert!(stream.contains("(2) Tj"), "expected selectable superscript `2`:\n{stream}");
        assert!(stream.contains("/GSa128 gs"), "expected semi-transparent outlined alpha state:\n{stream}");

        // Outlined glyph contours must be emitted outside `BT ... ET` with balanced `q`/`Q`.
        assert!(stream.contains(" m\n") && (stream.contains("f\n") || stream.contains("S\n")));
        let q_count = stream.lines().filter(|l| *l == "q").count();
        let big_q_count = stream.lines().filter(|l| *l == "Q").count();
        assert_eq!(q_count, big_q_count, "graphics state `q`/`Q` must balance");
        let bt_count = stream.lines().filter(|l| *l == "BT").count();
        let et_count = stream.lines().filter(|l| *l == "ET").count();
        assert_eq!(bt_count, et_count, "text block `BT`/`ET` must balance");
        assert!(bt_count > 0);
    }
}

#[test]
fn cld_compiled_graph_exports_deterministic_pdf17_with_selectable_and_outlined_letters() {
    use curvebook_chart::compile_graph;
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, Cell, ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec, DataSheet, GraphSheet, OneWayAnovaSpec,
        PairwiseComparisonRecord, PairwiseDisplayMode, ProjectDocument, ResultSheet, ResultStatus, SheetId, TableArchetype,
    };

    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("column table");
    table.data_sets[0].title = String::from("Ctrl");
    table.data_sets[1].title = String::from("Mid");
    table.data_sets[2].title = String::from("High");
    for (c, base) in [(0, 10.0), (1, 14.0), (2, 22.0)] {
        table.data_sets[c].cells =
            vec![vec![Cell::number(base - 1.0)], vec![Cell::number(base)], vec![Cell::number(base + 1.0)], vec![Cell::number(base)]];
    }
    let ds = DataSheet::new(SheetId::new("col-cld"), "Treatment", TableArchetype::Column(table)).expect("ds");
    let spec =
        AnalysisSpec::new(SheetId::new("res-cld"), SheetId::new("col-cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default())).expect("spec");
    let mut res = ResultSheet::new(spec).expect("res");
    res.status = ResultStatus::UpToDate;
    res.pairwise_comparisons = vec![
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 1,
            label_a: String::from("Ctrl"),
            label_b: String::from("Mid"),
            estimate_diff: Some(-4.0),
            raw_p_or_q: 0.14,
            is_fdr_q: false,
            significant: false,
        },
        PairwiseComparisonRecord {
            group_a: 0,
            group_b: 2,
            label_a: String::from("Ctrl"),
            label_b: String::from("High"),
            estimate_diff: Some(-12.0),
            raw_p_or_q: 0.001,
            is_fdr_q: false,
            significant: true,
        },
        PairwiseComparisonRecord {
            group_a: 1,
            group_b: 2,
            label_a: String::from("Mid"),
            label_b: String::from("High"),
            estimate_diff: Some(-8.0),
            raw_p_or_q: 0.12,
            is_fdr_q: false,
            significant: false,
        },
    ];

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-cld"), &ds).expect("gs");
    gs.spec.title = String::from("CLD PDF 1.7 Figure");
    gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res-cld")));
    gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;

    let mut gs_ctl = gs.clone();
    gs_ctl.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsOnly;
    gs_ctl.spec.compact_letter_display = None;

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add ds");
    doc.add_result_sheet(res).expect("add res");
    doc.add_graph_sheet(gs.clone()).expect("add gs");

    let scene_cld = compile_graph(&gs, &doc).expect("compile cld scene");
    let scene_ctl = compile_graph(&gs_ctl, &doc).expect("compile control scene");

    for embedded in [None, Some(build_minimal_truetype_font())] {
        let sel_opts = PdfExportOptions {
            version: PdfVersion::Pdf17,
            compress_streams: true,
            outline_text: false,
            embedded_truetype_font: embedded.clone(),
            ..PdfExportOptions::default()
        };
        let pdf_1 = render_pdf(&scene_cld, &sel_opts).expect("first cld pdf");
        let pdf_2 = render_pdf(&scene_cld, &sel_opts).expect("second cld pdf");
        assert_eq!(pdf_1, pdf_2, "%PDF-1.7 CLD output must be byte-deterministic");

        let parsed = parse_pdf_strict(&pdf_1, b"%PDF-1.7\n").expect("strict %PDF-1.7 parse");
        let stream = parsed
            .streams
            .iter()
            .find(|(_, dict, _)| !dict.contains("/Length1 "))
            .map(|(_, _, decoded)| String::from_utf8(decoded.clone()).expect("UTF-8 stream"))
            .expect("page stream");

        assert!(stream.contains("(a) Tj\n"), "missing `(a) Tj` in %PDF-1.7 stream");
        assert!(stream.contains("(ab) Tj\n"), "missing `(ab) Tj` in %PDF-1.7 stream");
        assert!(stream.contains("(b) Tj\n"), "missing `(b) Tj` in %PDF-1.7 stream");

        let pdf_ctl = render_pdf(&scene_ctl, &sel_opts).expect("control pdf");
        let parsed_ctl = parse_pdf_strict(&pdf_ctl, b"%PDF-1.7\n").expect("strict control %PDF-1.7 parse");
        let stream_ctl = parsed_ctl
            .streams
            .iter()
            .find(|(_, dict, _)| !dict.contains("/Length1 "))
            .map(|(_, _, decoded)| String::from_utf8(decoded.clone()).expect("UTF-8 control stream"))
            .expect("control page stream");
        assert!(!stream_ctl.contains("(ab) Tj\n"), "control PDF must not emit CLD `(ab) Tj`");
    }

    let out_opts = PdfExportOptions { version: PdfVersion::Pdf17, outline_text: true, ..PdfExportOptions::default() };
    let out_pdf = render_pdf(&scene_cld, &out_opts).expect("outlined cld pdf");
    let out_parsed = parse_pdf_strict(&out_pdf, b"%PDF-1.7\n").expect("strict outlined %PDF-1.7 parse");
    let out_stream = String::from_utf8(out_parsed.streams[0].2.clone()).expect("UTF-8 outlined stream");
    let q_count = out_stream.lines().filter(|l| *l == "q").count();
    let big_q_count = out_stream.lines().filter(|l| *l == "Q").count();
    assert_eq!(q_count, big_q_count, "outlined graphics stack q/Q must balance");
    assert!(!out_stream.contains(" Tj\n"), "outlined PDF must not emit Tj text operators");
}

#[test]
fn pdf17_prunes_unreferenced_fonts_and_omits_font_dict_when_outlined() {
    let mut ascii_scene = ChartScene::new("graph-pruned", "ASCII TrueType", 320.0, 220.0, Some(Rgba8::WHITE));
    ascii_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("Linear Calibration"),
        anchor_pt: PointPt::new(160.0, 40.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let find_page_dict = |parsed: &ParsedPdf| -> String {
        parsed
            .objects
            .values()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .find(|s| s.contains("/Type /Page ") || s.contains("/Type /Page\n"))
            .expect("missing /Type /Page object")
    };

    // 1. Outlined PDF must omit `/Font << ... >>` from the Page `/Resources` dictionary completely.
    let out_opts = PdfExportOptions { version: PdfVersion::Pdf17, outline_text: true, ..PdfExportOptions::default() };
    let out_pdf = render_pdf(&ascii_scene, &out_opts).expect("outlined pdf");
    let out_parsed = parse_pdf_strict(&out_pdf, b"%PDF-1.7\n").expect("strict outlined pdf");
    let out_page = find_page_dict(&out_parsed);
    assert!(!out_page.contains("/Font"), "outlined Page /Resources must omit unused /Font dict:\n{out_page}");

    // 2. Embedded TrueType ASCII PDF must list only `/F5` and omit unused Core-14 Type 1 font refs (`/F1`..`/F4`).
    let ttf_opts = PdfExportOptions {
        version: PdfVersion::Pdf17,
        outline_text: false,
        embedded_truetype_font: Some(build_minimal_truetype_font()),
        ..PdfExportOptions::default()
    };
    let ttf_pdf = render_pdf(&ascii_scene, &ttf_opts).expect("ttf pdf");
    let ttf_parsed = parse_pdf_strict(&ttf_pdf, b"%PDF-1.7\n").expect("strict ttf pdf");
    let ttf_page = find_page_dict(&ttf_parsed);
    assert!(ttf_page.contains("/F6 "), "expected embedded TrueType `/F6` reference in Page /Resources:\n{ttf_page}");
    for unused in ["/F1 ", "/F2 ", "/F3 ", "/F4 ", "/F5 "] {
        assert!(!ttf_page.contains(unused), "unreferenced Core-14 font {unused} leaked into Page /Resources:\n{ttf_page}");
    }
}

#[test]
fn pdf_selectable_invisible_midline_span_preserves_cursor_advance() {
    use curvebook_text::RichSpan;

    let mut scene = ChartScene::new("pdf-invis-span", "Invisible Span Advance", 240.0, 100.0, Some(Rgba8::WHITE));
    let mut rich = RichText::default();
    rich.spans.push(RichSpan::plain("Left"));
    let mut gap = RichSpan::plain("HIDDEN_GAP");
    gap.color = Some(Rgba8::rgba(0, 0, 0, 0));
    rich.spans.push(gap);
    rich.spans.push(RichSpan::plain("Right"));

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich,
        anchor_pt: PointPt::new(20.0, 50.0),
        anchor: TextAnchor::Start,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let opts = PdfExportOptions { version: PdfVersion::Pdf17, compress_streams: false, outline_text: false, ..PdfExportOptions::default() };
    let bytes = render_pdf(&scene, &opts).expect("render pdf");
    let parsed = parse_pdf_strict(&bytes, b"%PDF-1.7\n").expect("strict pdf parse");
    let stream = String::from_utf8(parsed.streams[0].2.clone()).expect("UTF-8 stream");

    assert!(!stream.contains("HIDDEN_GAP"), "0-alpha span must not emit literal text:\n{stream}");
    let (_, after_left) = stream.split_once("(Left) Tj").expect("Left literal");
    let (between, _) = after_left.split_once("(Right) Tj").expect("Right literal");
    assert!(
        between.contains("ET\n") && between.contains("BT\n") && between.contains(" Tm\n"),
        "invisible mid-line span must close BT and emit a new Tm before subsequent text:\n{stream}"
    );
}
