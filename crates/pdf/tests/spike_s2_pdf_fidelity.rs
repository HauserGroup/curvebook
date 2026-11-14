//! Risk Spike S2 Proof: Pure-Rust PDF 1.4+ scientific typography, Core-14 WinAnsi/Symbol
//! encoding, superscript/subscript baseline shifts, arbitrary rotation matrices, alpha
//! transparency `/ExtGState`, `TrimBox`/`BleedBox`, sRGB/CMYK/Grayscale colour spaces,
//! outlined glyph fallback, xref/FlateDecode structural verification, and byte determinism.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Instant;

use flate2::read::ZlibDecoder;
use lumencraft_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
};
use lumencraft_palette::{LineDashStyle, OKABE_ITO_BLUE, OKABE_ITO_ORANGE, OKABE_ITO_VERMILION, Rgba8};
use lumencraft_pdf::{PdfColorSpace, PdfError, PdfExportOptions, render_pdf, render_pdf_pages};
use lumencraft_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, parse_markup};

struct ParsedPdf {
    objects: BTreeMap<usize, Vec<u8>>,
    xref_offsets: Vec<(usize, usize, char)>,
    trailer: String,
    streams: Vec<(usize, String, Vec<u8>)>,
}

fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn parse_and_verify_pdf(bytes: &[u8]) -> Result<ParsedPdf, String> {
    assert!(bytes.starts_with(b"%PDF-1.4\n") || bytes.starts_with(b"%PDF-1.7\n"), "missing %PDF-1.4/%PDF-1.7 header");
    assert!(bytes.ends_with(b"%%EOF\n"), "missing %%EOF\\n footer");

    let startxref_pos = rfind_subslice(bytes, b"startxref\n").ok_or("missing startxref")?;
    let after_startxref = std::str::from_utf8(&bytes[startxref_pos + b"startxref\n".len()..]).map_err(|e| format!("ASCII startxref: {e}"))?;
    let xref_offset_str = after_startxref.lines().next().ok_or("missing xref offset line")?.trim();
    let xref_offset: usize = xref_offset_str.parse().map_err(|e| format!("invalid xref offset: {e}"))?;

    assert!(xref_offset < startxref_pos, "xref offset out of bounds");
    assert!(bytes[xref_offset..].starts_with(b"xref\n"), "xref offset does not point to xref\\n");

    let trailer_rel = find_subslice(&bytes[xref_offset..startxref_pos], b"trailer\n").ok_or("missing trailer after xref")?;
    let xref_section = std::str::from_utf8(&bytes[xref_offset..xref_offset + trailer_rel]).map_err(|e| format!("ASCII xref table: {e}"))?;
    let trailer_section = std::str::from_utf8(&bytes[xref_offset + trailer_rel..startxref_pos]).map_err(|e| format!("ASCII trailer: {e}"))?;

    let mut xref_lines = xref_section.lines();
    assert_eq!(xref_lines.next(), Some("xref"));
    let header_parts: Vec<&str> = xref_lines.next().ok_or("missing xref range line")?.split_whitespace().collect();
    assert_eq!(header_parts.len(), 2);
    assert_eq!(header_parts[0], "0");
    let total_entries: usize = header_parts[1].parse().map_err(|e| format!("invalid xref count: {e}"))?;

    let mut xref_offsets = Vec::with_capacity(total_entries);
    for (idx, line) in xref_lines.enumerate() {
        assert_eq!(line.len(), 20 - 1, "each xref entry line before \\n must be 19 chars: `{line}`");
        let off: usize = line[0..10].parse().map_err(|e| format!("10-digit xref offset: {e}"))?;
        let generation: usize = line[11..16].parse().map_err(|e| format!("5-digit xref generation: {e}"))?;
        let kind = line.chars().nth(17).ok_or("xref f/n flag")?;
        if idx == 0 {
            assert_eq!((off, generation, kind), (0, 65535, 'f'));
        } else {
            assert_eq!(generation, 0);
            assert_eq!(kind, 'n');
            assert!(off < xref_offset, "object {idx} offset {off} >= xref {xref_offset}");
        }
        xref_offsets.push((off, generation, kind));
    }
    assert_eq!(xref_offsets.len(), total_entries);

    let mut objects = BTreeMap::new();
    let mut streams = Vec::new();
    for (obj_id, &(off, _, kind)) in xref_offsets.iter().enumerate() {
        if kind != 'n' {
            continue;
        }
        let expected_prefix = format!("{obj_id} 0 obj\n");
        assert!(bytes[off..].starts_with(expected_prefix.as_bytes()), "object {obj_id} missing `{expected_prefix}` at offset {off}");
        let body_start = off + expected_prefix.len();
        let endobj_rel = find_subslice(&bytes[body_start..xref_offset], b"endobj\n").ok_or("missing endobj\\n")?;
        let body = &bytes[body_start..body_start + endobj_rel];
        objects.insert(obj_id, body.to_vec());

        if let Some(stream_rel) = find_subslice(body, b"\nstream\n") {
            let dict_bytes = &body[..stream_rel];
            let dict_str = String::from_utf8_lossy(dict_bytes);
            let data_start = stream_rel + b"\nstream\n".len();
            let endstream_rel = find_subslice(&body[data_start..], b"\nendstream").ok_or("missing \\nendstream")?;
            let raw_stream = &body[data_start..data_start + endstream_rel];

            let declared_len = extract_pdf_int(&dict_str, "/Length ").ok_or("stream missing /Length")?;
            assert_eq!(raw_stream.len(), declared_len, "object {obj_id} /Length mismatch");

            let decoded = if dict_str.contains("/Filter /FlateDecode") {
                let mut decoder = ZlibDecoder::new(raw_stream);
                let mut out = Vec::new();
                decoder.read_to_end(&mut out).map_err(|e| format!("zlib FlateDecode: {e}"))?;
                out
            } else {
                raw_stream.to_vec()
            };
            streams.push((obj_id, dict_str.into_owned(), decoded));
        }
    }

    Ok(ParsedPdf { objects, xref_offsets, trailer: trailer_section.to_owned(), streams })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn extract_pdf_int(dict: &str, key: &str) -> Option<usize> {
    let idx = dict.find(key)?;
    let rest = &dict[idx + key.len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn build_spike_s2_scenes() -> Vec<ChartScene> {
    let mut p1 = ChartScene::new("graph-1", "Dose-Response & Thermodynamics (ΔG)", 480.0, 340.0, Some(Rgba8::WHITE));
    p1.plot_rect_pt = RectPt::new(64.0, 36.0, 380.0, 240.0);

    // Clipped plot frame + translucent 95% CI ribbon (alpha = 76) with opaque stroke (alpha = 255)
    p1.primitives.push(ScenePrimitive::PushClip { rect: p1.plot_rect_pt });

    let mut ribbon = Path2D::new();
    ribbon.commands.push(PathCmd::MoveTo(PointPt::new(70.0, 220.0)));
    ribbon.commands.push(PathCmd::QuadTo { control: PointPt::new(180.0, 180.0), to: PointPt::new(260.0, 120.0) });
    ribbon.commands.push(PathCmd::CubicTo { c1: PointPt::new(310.0, 90.0), c2: PointPt::new(370.0, 72.0), to: PointPt::new(430.0, 68.0) });
    ribbon.commands.push(PathCmd::LineTo(PointPt::new(430.0, 98.0)));
    ribbon.commands.push(PathCmd::CubicTo { c1: PointPt::new(370.0, 102.0), c2: PointPt::new(310.0, 125.0), to: PointPt::new(260.0, 155.0) });
    ribbon.commands.push(PathCmd::QuadTo { control: PointPt::new(180.0, 210.0), to: PointPt::new(70.0, 250.0) });
    ribbon.commands.push(PathCmd::Close);

    p1.primitives.push(ScenePrimitive::Path {
        path: ribbon,
        fill: Some(FillPaint::solid(OKABE_ITO_BLUE.with_alpha(76))),
        stroke: Some(StrokePaint { color: OKABE_ITO_BLUE, width_pt: 1.5, dash: LineDashStyle::Solid, cap: LineCap::Round, join: LineJoin::Round }),
        clip: None,
    });

    // Dashed threshold line inside nested clip
    p1.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(64.0, 140.0), PointPt::new(444.0, 140.0)),
        fill: None,
        stroke: Some(StrokePaint {
            color: OKABE_ITO_VERMILION.with_alpha(180),
            width_pt: 1.25,
            dash: LineDashStyle::DashDot,
            cap: LineCap::Square,
            join: LineJoin::Bevel,
        }),
        clip: Some(RectPt::new(64.0, 40.0, 380.0, 230.0)),
    });

    p1.primitives.push(ScenePrimitive::PopClip);

    // Scientific labels covering superscripts, subscripts, Greek, relational symbols, bold, italic, and rotations
    p1.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("10<sup>−6</sup> mol·L<sup>−1</sup>"),
        anchor_pt: PointPt::new(254.0, 310.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    p1.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("IC<sub>50</sub> = 2.4 ± 0.3 µM"),
        anchor_pt: PointPt::new(82.0, 74.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: OKABE_ITO_BLUE,
    }));

    p1.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("α, β, γ, ΔG, χ² · ≤ ≥ × ≈ ∞ °"),
        anchor_pt: PointPt::new(140.0, 286.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 45.0,
        color: Rgba8::rgb(35, 35, 35),
    }));

    p1.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>p < 0.0001</b> · <i>K</i><sub>d</sub>"),
        anchor_pt: PointPt::new(428.0, 120.0),
        anchor: TextAnchor::End,
        font_size_pt: 10.0,
        rotation_deg: 90.0,
        color: OKABE_ITO_VERMILION,
    }));

    p1.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![
            RichSpan { text: String::from("V"), weight: SpanWeight::Bold, slope: SpanSlope::Italic, ..RichSpan::default() },
            RichSpan { text: String::from("max"), script: SpanScript::Subscript, scale: 1.0, ..RichSpan::default() },
        ]),
        anchor_pt: PointPt::new(82.0, 96.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: OKABE_ITO_ORANGE,
    }));

    // Page 2: Landscape survival summary with rotated Y-axis label and full Greek alphabet check
    let mut p2 = ChartScene::new("graph-2", "Survival & Complete Greek Symbol Audit", 540.0, 360.0, Some(Rgba8::WHITE));
    p2.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(80.0, 60.0)),
                PathCmd::LineTo(PointPt::new(200.0, 60.0)),
                PathCmd::LineTo(PointPt::new(200.0, 150.0)),
                PathCmd::LineTo(PointPt::new(360.0, 150.0)),
                PathCmd::LineTo(PointPt::new(360.0, 240.0)),
                PathCmd::LineTo(PointPt::new(480.0, 240.0)),
            ],
        },
        fill: None,
        stroke: Some(StrokePaint::solid(OKABE_ITO_BLUE, 2.0)),
        clip: None,
    });
    p2.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("Survival Probability (%)"),
        anchor_pt: PointPt::new(28.0, 180.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 11.0,
        rotation_deg: -90.0,
        color: Rgba8::BLACK,
    }));
    p2.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ αβγδεζηθικλμνξοπρςστυφχψω"),
        anchor_pt: PointPt::new(80.0, 315.0),
        anchor: TextAnchor::Start,
        font_size_pt: 9.5,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    vec![p1, p2]
}

fn verify_document_objects(parsed: &ParsedPdf) {
    assert!(parsed.trailer.contains("/Root 1 0 R"));
    assert!(parsed.trailer.contains("/Info 8 0 R"));

    let catalog = String::from_utf8_lossy(&parsed.objects[&1]);
    assert!(catalog.contains("/Type /Catalog"));
    assert!(catalog.contains("/Pages 2 0 R"));

    let pages_obj = String::from_utf8_lossy(&parsed.objects[&2]);
    assert!(pages_obj.contains("/Count 2"));

    assert!(String::from_utf8_lossy(&parsed.objects[&3]).contains("/BaseFont /Helvetica /Encoding /WinAnsiEncoding"));
    assert!(String::from_utf8_lossy(&parsed.objects[&4]).contains("/BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding"));
    assert!(String::from_utf8_lossy(&parsed.objects[&5]).contains("/BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding"));
    assert!(String::from_utf8_lossy(&parsed.objects[&6]).contains("/BaseFont /Symbol >>"));
    assert!(String::from_utf8_lossy(&parsed.objects[&7]).contains("/BaseFont /Helvetica-BoldOblique /Encoding /WinAnsiEncoding"));

    let info = String::from_utf8_lossy(&parsed.objects[&8]);
    assert!(info.contains("/Title <FEFF"));
    assert!(info.contains("/Creator (LumenCraft Spike S2)"));

    let all_obj_text: String = parsed.objects.values().map(|b| String::from_utf8_lossy(b).into_owned()).collect::<Vec<_>>().join("\n");
    assert!(all_obj_text.contains("/Type /ExtGState /ca 0.298 /CA 0.298"));
    assert!(all_obj_text.contains("/Type /ExtGState /ca 0.706 /CA 0.706"));
    assert!(all_obj_text.contains("/Type /ExtGState /CA 1.000"));
    assert!(all_obj_text.contains("/MediaBox [0.000 0.000 480.000 340.000]"));
    assert!(all_obj_text.contains("/MediaBox [0.000 0.000 540.000 360.000]"));
    assert!(all_obj_text.contains("/TrimBox [12.000 10.000 468.000 330.000]"));
    assert!(all_obj_text.contains("/BleedBox [4.000 4.000 476.000 336.000]"));
}

fn verify_page1_geometry_and_text(page1_stream: &str) {
    // Ribbon first QuadTo: P0=(70, 220), P1=(180, 180), P2=(260, 120) in Y-down on H=340 ->
    // C1=(143.333, 193.333)->y_pdf=146.667, C2=(206.667, 160.000)->y_pdf=180.000, P2=(260.000, 220.000)
    let checks: &[(&str, &str)] = &[
        ("143.333 146.667 206.667 180.000 260.000 220.000 c", "exact quad-to-cubic elevation missing"),
        ("/GSa076 gs", "translucent ribbon ExtGState missing"),
        ("/GSs255 gs", "opaque stroke ExtGState reset missing"),
        ("1 J 1 j", "round cap/join missing"),
        ("[5.625 2.500 1.250 2.500] 0 d", "dash-dot pattern missing"),
        ("re W n", "rectangular clip operator missing"),
        (" rg\n", "sRGB fill operator missing"),
        (" RG ", "sRGB stroke operator missing"),
        ("BT\n", "PDF text block open missing"),
        ("ET\n", "PDF text block close missing"),
        ("/F1 12.00 Tf", "Helvetica 12pt Tf missing"),
        ("(10) Tj", "base mantissa `10` missing"),
        ("4.200 Ts", "superscript baseline shift missing"),
        ("/F4 8.40 Tf", "superscript Symbol switch for Unicode minus missing"),
        ("(-) Tj", "Symbol minus sign missing"),
        ("/F1 8.40 Tf", "superscript Helvetica 8.40pt Tf missing"),
        ("(6) Tj", "superscript exponent `6` missing"),
        ("0.000 Ts", "baseline reset Ts missing"),
        ("( mol\\267L) Tj", "WinAnsi middle-dot escape missing"),
        ("-2.200 Ts", "subscript baseline shift missing"),
        ("(50) Tj", "subscript `50` missing"),
        ("(\\261) Tj", "Symbol ± (\\261) missing"),
        ("(m) Tj", "Symbol µ (m) missing"),
        ("0.707107 -0.707107 0.707107 0.707107 ", "45-degree rotation Tm missing"),
        ("(a) Tj", "Symbol alpha missing"),
        ("(b) Tj", "Symbol beta missing"),
        ("(g) Tj", "Symbol gamma missing"),
        ("(D) Tj", "Symbol Delta missing"),
        ("(c) Tj", "Symbol chi missing"),
        ("(\\262 \\267 ) Tj", "WinAnsi superscript-2 and middle-dot missing"),
        ("(\\243) Tj", "Symbol <= missing"),
        ("(\\263) Tj", "Symbol >= missing"),
        ("(\\264) Tj", "Symbol multiply missing"),
        ("(\\273) Tj", "Symbol approx missing"),
        ("(\\245) Tj", "Symbol infinity missing"),
        ("(\\260) Tj", "Symbol degree missing"),
        ("0.000000 -1.000000 1.000000 0.000000 ", "90-degree rotation Tm missing"),
        ("/F2 10.00 Tf\n(p < 0.0001) Tj", "Bold Helvetica p-value label missing"),
        ("/F3 10.00 Tf\n(K) Tj", "Italic Helvetica K label missing"),
        ("/F5 10.00 Tf\n(V) Tj", "BoldOblique Helvetica V label missing"),
        ("-2.000 Ts\n/F1 7.00 Tf\n(max) Tj", "manual RichSpan subscript scale/rise missing"),
    ];
    for &(needle, msg) in checks {
        assert!(page1_stream.contains(needle), "{msg}: expected `{needle}`");
    }
}

#[test]
fn spike_s2_selectable_vector_pdf_greek_super_sub_rotation_and_xref_fidelity() -> Result<(), String> {
    let scenes = build_spike_s2_scenes();
    let opts = PdfExportOptions {
        color_space: PdfColorSpace::Srgb,
        compress_streams: true,
        title: Some(String::from("Spike S2 — ΔG & IC50")),
        creator: String::from("LumenCraft Spike S2"),
        outline_text: false,
        trim_pt: Some(RectPt::new(12.0, 10.0, 456.0, 320.0)),
        bleed_pt: Some(RectPt::new(4.0, 4.0, 472.0, 332.0)),
        ..PdfExportOptions::default()
    };

    let t0 = Instant::now();
    let pdf_a = render_pdf_pages(&scenes, &opts).map_err(|e| format!("render multipage sRGB PDF: {e}"))?;
    let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let pdf_b = render_pdf_pages(&scenes, &opts).map_err(|e| format!("re-render determinism: {e}"))?;
    assert_eq!(pdf_a, pdf_b, "PDF output must be byte-for-byte deterministic across runs");

    let parsed = parse_and_verify_pdf(&pdf_a)?;
    verify_document_objects(&parsed);

    assert_eq!(parsed.streams.len(), 2, "expected 2 compressed page streams");
    let page1_stream = String::from_utf8(parsed.streams[0].2.clone()).map_err(|e| format!("UTF-8 stream 1: {e}"))?;
    let page2_stream = String::from_utf8(parsed.streams[1].2.clone()).map_err(|e| format!("UTF-8 stream 2: {e}"))?;

    verify_page1_geometry_and_text(&page1_stream);

    assert!(page2_stream.contains("0.000000 1.000000 -1.000000 0.000000 "), "-90-degree rotation Tm missing");
    assert!(page2_stream.contains("(ABGDEZHQIKLMNXOPRSTUFCYW) Tj"), "full uppercase Greek Symbol mapping mismatch");
    assert!(page2_stream.contains("(abgdezhqiklmnxoprVstufcyw) Tj"), "full lowercase Greek Symbol mapping mismatch");

    let uncompressed_opts = PdfExportOptions { compress_streams: false, ..opts };
    let pdf_raw = render_pdf_pages(&scenes, &uncompressed_opts).map_err(|e| format!("uncompressed multipage PDF: {e}"))?;
    let parsed_raw = parse_and_verify_pdf(&pdf_raw)?;
    assert_eq!(parsed_raw.streams[0].2, parsed.streams[0].2);
    assert_eq!(parsed_raw.streams[1].2, parsed.streams[1].2);

    println!(
        "SPIKE_S2_METRICS: pages={}, total_objs={}, xref_entries={}, compressed_bytes={}, uncompressed_bytes={}, p1_stream_bytes={}, p2_stream_bytes={}, render_ms={:.3}",
        scenes.len(),
        parsed.objects.len(),
        parsed.xref_offsets.len(),
        pdf_a.len(),
        pdf_raw.len(),
        parsed.streams[0].2.len(),
        parsed.streams[1].2.len(),
        elapsed_ms
    );
    Ok(())
}

#[test]
fn spike_s2_cmyk_and_grayscale_color_spaces() -> Result<(), String> {
    let scenes = build_spike_s2_scenes();

    let cmyk_opts = PdfExportOptions { color_space: PdfColorSpace::Cmyk, ..PdfExportOptions::default() };
    let cmyk_bytes = render_pdf_pages(&scenes, &cmyk_opts).map_err(|e| format!("CMYK PDF: {e}"))?;
    assert_eq!(cmyk_bytes, render_pdf_pages(&scenes, &cmyk_opts).map_err(|e| format!("CMYK det: {e}"))?);
    let cmyk_parsed = parse_and_verify_pdf(&cmyk_bytes)?;
    let cmyk_stream = String::from_utf8(cmyk_parsed.streams[0].2.clone()).map_err(|e| format!("UTF-8 CMYK: {e}"))?;
    assert!(cmyk_stream.contains(" k\n"), "CMYK fill operator `k` missing");
    assert!(cmyk_stream.contains(" K "), "CMYK stroke operator `K` missing");
    assert!(!cmyk_stream.contains(" rg\n"), "CMYK stream must not emit sRGB `rg`");
    assert!(!cmyk_stream.contains(" RG "), "CMYK stream must not emit sRGB `RG`");
    assert!(!cmyk_stream.contains(" g\n"), "CMYK stream must not emit Grayscale `g`");

    let gray_opts = PdfExportOptions { color_space: PdfColorSpace::Grayscale, ..PdfExportOptions::default() };
    let gray_bytes = render_pdf_pages(&scenes, &gray_opts).map_err(|e| format!("Grayscale PDF: {e}"))?;
    assert_eq!(gray_bytes, render_pdf_pages(&scenes, &gray_opts).map_err(|e| format!("Grayscale det: {e}"))?);
    let gray_parsed = parse_and_verify_pdf(&gray_bytes)?;
    let gray_stream = String::from_utf8(gray_parsed.streams[0].2.clone()).map_err(|e| format!("UTF-8 Grayscale: {e}"))?;
    assert!(gray_stream.contains(" g\n"), "Grayscale fill operator `g` missing");
    assert!(gray_stream.contains(" G "), "Grayscale stroke operator `G` missing");
    assert!(!gray_stream.contains(" rg\n"), "Grayscale stream must not emit sRGB `rg`");
    assert!(!gray_stream.contains(" k\n"), "Grayscale stream must not emit CMYK `k`");
    Ok(())
}

#[test]
fn spike_s2_outline_text_mode_emits_vector_paths_without_text_operators() -> Result<(), String> {
    let scenes = build_spike_s2_scenes();
    let outline_opts =
        PdfExportOptions { color_space: PdfColorSpace::Srgb, compress_streams: true, outline_text: true, ..PdfExportOptions::default() };

    let pdf_a = render_pdf_pages(&scenes, &outline_opts).map_err(|e| format!("outlined PDF: {e}"))?;
    let pdf_b = render_pdf_pages(&scenes, &outline_opts).map_err(|e| format!("outlined PDF det: {e}"))?;
    assert_eq!(pdf_a, pdf_b, "outlined PDF must be deterministic");

    let parsed = parse_and_verify_pdf(&pdf_a)?;
    let s1 = String::from_utf8(parsed.streams[0].2.clone()).map_err(|e| format!("UTF-8 s1: {e}"))?;
    let s2 = String::from_utf8(parsed.streams[1].2.clone()).map_err(|e| format!("UTF-8 s2: {e}"))?;

    for s in [&s1, &s2] {
        assert!(!s.contains("\nBT\n") && !s.starts_with("BT\n"), "outline_text=true must not emit BT");
        assert!(!s.contains(" Tj\n"), "outline_text=true must not emit Tj");
        assert!(!s.contains(" Tm\n"), "outline_text=true must not emit Tm");
        assert!(s.contains(" m\n") && s.contains(" l\n") && s.contains("f\n"), "outlined glyphs must emit vector path operators");
    }
    Ok(())
}

#[test]
fn spike_s2_error_guards_and_unbalanced_clip_safety() -> Result<(), String> {
    assert_eq!(render_pdf_pages(&[], &PdfExportOptions::default()), Err(PdfError::EmptyDocument));

    let bad_scene = ChartScene { width_pt: 0.0, ..ChartScene::new("bad", "Bad", 100.0, 100.0, None) };
    assert!(matches!(render_pdf(&bad_scene, &PdfExportOptions::default()), Err(PdfError::InvalidScene { .. })));

    let bad_trim = PdfExportOptions { trim_pt: Some(RectPt::new(0.0, 0.0, 0.0, 100.0)), ..PdfExportOptions::default() };
    let ok_scene = ChartScene::new("ok", "OK", 200.0, 150.0, Some(Rgba8::WHITE));
    assert!(matches!(render_pdf(&ok_scene, &bad_trim), Err(PdfError::InvalidScene { .. })));

    let mut unbal = ChartScene::new("clip", "Unbalanced Clip", 200.0, 150.0, Some(Rgba8::WHITE));
    unbal.primitives.push(ScenePrimitive::PushClip { rect: RectPt::new(10.0, 10.0, 100.0, 80.0) });
    unbal.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("α + β"),
        anchor_pt: PointPt::new(20.0, 40.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let bytes = render_pdf(&unbal, &PdfExportOptions::default()).map_err(|e| format!("unbalanced clip: {e}"))?;
    let parsed = parse_and_verify_pdf(&bytes)?;
    let stream = String::from_utf8(parsed.streams[0].2.clone()).map_err(|e| format!("UTF-8 stream: {e}"))?;
    let q_count = stream.lines().filter(|l| *l == "q").count();
    let big_q_count = stream.lines().filter(|l| *l == "Q").count();
    assert_eq!(q_count, big_q_count, "PDF q/Q graphics state stack must always balance");
    Ok(())
}
