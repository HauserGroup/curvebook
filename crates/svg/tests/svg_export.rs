#![allow(clippy::unwrap_used, clippy::expect_used)]

use curvebook_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor,
    compile_graph, compile_layout,
};
use curvebook_doc::{DataSheet, GraphSheet, LayoutSheet, LayoutSpec, ProjectDocument, SheetId, SubcolumnMode, TableArchetype, XyTable};
use curvebook_palette::{LineDashStyle, OKABE_ITO_BLUE, OKABE_ITO_VERMILION, Rgba8};
use curvebook_svg::{SVG_XMLNS, SvgError, SvgExportOptions, render_svg, validate_svg_structure};
use curvebook_text::{RichSpan, RichText, SpanScript, SpanSlope, SpanWeight, parse_markup};

fn sample_rich_scene() -> ChartScene {
    let mut scene = ChartScene::new("graph-dose", "IC50 & α-Curve <Test> \"1\" 'A'", 320.0, 240.0, Some(Rgba8::WHITE));
    let plot = RectPt::new(40.0, 24.0, 250.0, 176.0);

    scene.primitives.push(ScenePrimitive::PushClip { rect: plot });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(40.0, 160.0)),
                PathCmd::LineTo(PointPt::new(140.0, 120.0)),
                PathCmd::LineTo(PointPt::new(290.0, 60.0)),
                PathCmd::LineTo(PointPt::new(290.0, 180.0)),
                PathCmd::LineTo(PointPt::new(40.0, 180.0)),
                PathCmd::Close,
            ],
        },
        fill: Some(FillPaint::solid(OKABE_ITO_BLUE.with_alpha(85))),
        stroke: None,
        clip: None,
    });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(40.0, 140.0)),
                PathCmd::QuadTo { control: PointPt::new(150.0, 40.0), to: PointPt::new(290.0, 80.0) },
            ],
        },
        fill: None,
        stroke: Some(StrokePaint {
            color: OKABE_ITO_VERMILION,
            width_pt: 1.5,
            dash: LineDashStyle::Dashed,
            cap: LineCap::Round,
            join: LineJoin::Round,
        }),
        clip: Some(plot),
    });
    scene.primitives.push(ScenePrimitive::PopClip);

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<b>IC<sub>50</sub></b> = 10<sup>-6</sup> µM (α, β, ΔG, χ²)"),
        anchor_pt: PointPt::new(165.0, 224.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: parse_markup("<i>Response</i> (%)"),
        anchor_pt: PointPt::new(14.0, 112.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 10.0,
        rotation_deg: -90.0,
        color: Rgba8::BLACK,
    }));

    scene
}

#[test]
fn byte_determinism_across_repeated_exports_and_negative_zero() {
    let mut scene = sample_rich_scene();
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(-0.0, -0.0), PointPt::new(10.0, -1e-7)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: Some(RectPt { x: -0.0, y: -0.0, width: 100.0, height: 100.0 }),
    });
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Origin"),
        anchor_pt: PointPt::new(-0.0, -0.0),
        anchor: TextAnchor::Start,
        font_size_pt: 9.0,
        rotation_deg: -0.0,
        color: Rgba8::BLACK,
    }));

    for outline in [false, true] {
        for pretty in [true, false] {
            let opts = SvgExportOptions { pretty, include_xml_declaration: true, outline_text: outline, ..SvgExportOptions::default() };
            let first = render_svg(&scene, &opts).expect("first render");
            let second = render_svg(&scene, &opts).expect("second render");
            assert_eq!(first, second, "SVG output must be byte-for-byte deterministic (outline={outline}, pretty={pretty})");
            assert!(!first.contains("-0.000"), "negative zero leaked into SVG: {first}");
            assert!(!first.contains("\"-0\""), "negative zero integer leaked into SVG: {first}");
            validate_svg_structure(&first).expect("valid deterministic SVG");
        }
    }

    let prec0_opts = SvgExportOptions { precision: 0, ..SvgExportOptions::default() };
    let prec0_svg = render_svg(&scene, &prec0_opts).expect("precision 0 render");
    assert!(prec0_svg.contains("width=\"320pt\" height=\"240pt\" viewBox=\"0 0 320 240\""));
    assert!(!prec0_svg.contains("\"-0\""), "precision 0 must normalize -0.0 to 0: {prec0_svg}");
    assert!(!prec0_svg.contains("M-0 "), "precision 0 path must normalize -0.0 to 0: {prec0_svg}");
    assert!(!prec0_svg.contains(" -0 "), "precision 0 path arg must normalize -0.0 to 0: {prec0_svg}");
    validate_svg_structure(&prec0_svg).expect("precision 0 valid");
}

#[test]
fn svg11_root_metadata_and_path_geometry() {
    let mut scene = ChartScene::new("s1", "Cubic & Quad Curves", 200.0, 150.0, None);
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(10.0, 20.0)),
                PathCmd::CubicTo { c1: PointPt::new(40.0, 10.0), c2: PointPt::new(80.0, 90.0), to: PointPt::new(120.0, 50.0) },
                PathCmd::QuadTo { control: PointPt::new(150.0, 20.0), to: PointPt::new(180.0, 80.0) },
                PathCmd::Close,
            ],
        },
        fill: Some(FillPaint::solid(Rgba8::rgb(0x1B, 0x9E, 0x77))),
        stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 2.0, dash: LineDashStyle::Solid, cap: LineCap::Square, join: LineJoin::Bevel }),
        clip: None,
    });

    let opts = SvgExportOptions { include_xml_declaration: true, include_metadata: true, precision: 2, ..SvgExportOptions::default() };
    let svg = render_svg(&scene, &opts).expect("render svg");
    validate_svg_structure(&svg).expect("valid svg");

    assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
    assert!(svg.contains(&format!("xmlns=\"{SVG_XMLNS}\"")));
    assert!(svg.contains("version=\"1.1\""));
    assert!(svg.contains("width=\"200.00pt\" height=\"150.00pt\" viewBox=\"0 0 200.00 150.00\""));
    assert!(svg.contains("<title>Cubic &amp; Quad Curves</title>"));
    assert!(svg.contains("d=\"M10.00 20.00 C40.00 10.00 80.00 90.00 120.00 50.00 Q150.00 20.00 180.00 80.00 Z\""));
    assert!(svg.contains("stroke-linecap=\"square\" stroke-linejoin=\"bevel\""));
    assert!(!svg.contains("stroke-dasharray="), "solid stroke must omit dasharray");

    let no_meta = SvgExportOptions { include_metadata: false, ..opts };
    let svg_no_meta = render_svg(&scene, &no_meta).expect("render without metadata");
    assert!(!svg_no_meta.contains("<title>"));
}

#[test]
fn editable_text_supports_rich_tspans_greek_anchors_and_rotation() {
    let mut scene = ChartScene::new("t1", "Typography", 300.0, 200.0, None);
    let rich = RichText::new(vec![
        RichSpan { text: String::from("IC"), weight: SpanWeight::Bold, ..RichSpan::plain("") },
        RichSpan { text: String::from("50"), script: SpanScript::Subscript, scale: 0.7, ..RichSpan::plain("") },
        RichSpan { text: String::from(" (10"), ..RichSpan::plain("") },
        RichSpan { text: String::from("-6"), script: SpanScript::Superscript, scale: 0.7, ..RichSpan::plain("") },
        RichSpan { text: String::from(" µM α/β/γ/Δ/χ)"), slope: SpanSlope::Italic, color: Some(OKABE_ITO_BLUE), ..RichSpan::plain("") },
    ]);

    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich,
        anchor_pt: PointPt::new(150.0, 100.0),
        anchor: TextAnchor::End,
        font_size_pt: 12.0,
        rotation_deg: 45.0,
        color: Rgba8::BLACK,
    }));

    let svg = render_svg(&scene, &SvgExportOptions { outline_text: false, ..SvgExportOptions::default() }).expect("render editable text");
    validate_svg_structure(&svg).expect("valid editable text svg");

    assert!(svg.contains("<text "));
    assert!(svg.contains("text-anchor=\"end\""));
    assert!(svg.contains("transform=\"rotate(45.000, 150.000, 100.000)\""));
    assert!(svg.contains("font-weight=\"bold\""));
    assert!(svg.contains("font-style=\"italic\""));
    assert!(svg.contains("fill=\"#0072B2\""));
    assert!(svg.contains("µM α/β/γ/Δ/χ"));
    assert!(svg.contains("font-size=\"8.400\""), "sub/sup scale 0.7 * 12 = 8.400");
    assert!(svg.contains("dy=\"2.400\""), "subscript shifts down by 0.20 * 12 = 2.400");
    assert!(svg.contains("dy=\"-2.400\""), "baseline span after subscript restores baseline with dy=-2.400");
    assert!(svg.contains("dy=\"-4.200\""), "superscript shifts up by -0.35 * 12 = -4.200");
    assert!(svg.contains("dy=\"4.200\""), "baseline span after superscript restores baseline with dy=+4.200");
    assert!(!svg.contains("</tspan>\n"), "must not inject whitespace between <tspan> elements");
    assert!(!svg.contains("</tspan> <tspan"), "must not inject spaces between <tspan> elements");
}

#[test]
fn outlined_glyph_mode_emits_paths_without_text_elements_and_honors_anchor_and_rotation() {
    let make_scene = |anchor: TextAnchor, rotation_deg: f64| {
        let mut s = ChartScene::new("o1", "", 240.0, 180.0, None);
        s.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: parse_markup("IC<sub>50</sub> αβ"),
            anchor_pt: PointPt::new(120.0, 90.0),
            anchor,
            font_size_pt: 12.0,
            rotation_deg,
            color: Rgba8::rgba(10, 20, 30, 200),
        }));
        s
    };

    let opts = SvgExportOptions { outline_text: true, ..SvgExportOptions::default() };
    let svg_start = render_svg(&make_scene(TextAnchor::Start, 0.0), &opts).expect("outlined start");
    let svg_end = render_svg(&make_scene(TextAnchor::End, 0.0), &opts).expect("outlined end");
    let svg_rot = render_svg(&make_scene(TextAnchor::Middle, 90.0), &opts).expect("outlined rotated");

    for svg in [&svg_start, &svg_end, &svg_rot] {
        validate_svg_structure(svg).expect("valid outlined SVG");
        assert!(!svg.contains("<text"), "outlined mode must not emit <text>");
        assert!(!svg.contains("<tspan"), "outlined mode must not emit <tspan>");
        assert!(svg.contains("<path d=\""), "outlined mode must emit <path> glyph outlines");
        assert!(svg.contains("fill=\"#0A141E\" fill-opacity=\"0.784\" fill-rule=\"nonzero\" stroke=\"none\""));
    }
    assert_ne!(svg_start, svg_end, "TextAnchor::End must shift glyph positions relative to Start");
    assert_ne!(svg_start, svg_rot, "90 deg rotation must transform glyph path coordinates");
}

#[test]
fn clip_rects_nesting_and_deduplication() {
    let mut scene = ChartScene::new("clip", "Clipped", 200.0, 200.0, None);
    let outer = RectPt::new(10.0, 10.0, 180.0, 180.0);
    let inner = RectPt::new(30.0, 30.0, 100.0, 100.0);

    // Extra PopClip at depth 0 should be safely ignored; unclosed PushClip should auto-close.
    scene.primitives.push(ScenePrimitive::PopClip);
    scene.primitives.push(ScenePrimitive::PushClip { rect: outer });
    scene.primitives.push(ScenePrimitive::PushClip { rect: inner });
    scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(0.0, 0.0), PointPt::new(200.0, 200.0)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: Some(outer),
    });
    scene.primitives.push(ScenePrimitive::PopClip);
    // Intentionally omit second PopClip to verify clean auto-closing.

    let svg = render_svg(&scene, &SvgExportOptions::default()).expect("render clipped");
    validate_svg_structure(&svg).expect("balanced clip SVG");

    assert!(svg.contains("<defs>"));
    assert!(svg.contains("<clipPath id=\"clip-0\">"));
    assert!(svg.contains("<clipPath id=\"clip-1\">"));
    assert_eq!(svg.matches("<clipPath ").count(), 2, "identical outer clip rect must be deduplicated");
    assert!(svg.contains("<g clip-path=\"url(#clip-0)\">"));
    assert!(svg.contains("<g clip-path=\"url(#clip-1)\">"));
    assert!(svg.contains("clip-path=\"url(#clip-0)\"/>"));
    assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count());
}

#[test]
fn dash_arrays_and_translucent_fills_and_strokes() {
    let mut scene = ChartScene::new("styles", "Styles", 200.0, 120.0, None);
    let styles = [
        LineDashStyle::Solid,
        LineDashStyle::Dashed,
        LineDashStyle::Dotted,
        LineDashStyle::DashDot,
        LineDashStyle::DashDotDot,
        LineDashStyle::LongDash,
    ];

    for (idx, dash) in styles.into_iter().enumerate() {
        let y = 15.0 + (idx as f64) * 15.0;
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::line(PointPt::new(10.0, y), PointPt::new(190.0, y)),
            fill: Some(FillPaint::solid(Rgba8::rgba(0x00, 0x72, 0xB2, 128))),
            stroke: Some(StrokePaint { color: Rgba8::rgba(0xD5, 0x5E, 0x00, 64), width_pt: 2.0, dash, cap: LineCap::Round, join: LineJoin::Miter }),
            clip: None,
        });
    }

    let svg = render_svg(&scene, &SvgExportOptions { precision: 0, ..SvgExportOptions::default() }).expect("render styles");
    validate_svg_structure(&svg).expect("valid styles SVG");

    assert!(svg.contains("fill=\"#0072B2\" fill-opacity=\"0.502\""));
    assert!(svg.contains("stroke=\"#D55E00\" stroke-opacity=\"0.251\""));
    assert!(svg.contains("stroke-dasharray=\"8,5\""), "Dashed with width 2 -> 4*2, 2.5*2");
    assert!(svg.contains("stroke-dasharray=\"2,4\""), "Dotted with width 2 -> 1*2, 2*2");
    assert!(svg.contains("stroke-dasharray=\"9,4,2,4\""), "DashDot");
    assert!(svg.contains("stroke-dasharray=\"9,4,2,4,2,4\""), "DashDotDot");
    assert!(svg.contains("stroke-dasharray=\"14,6\""), "LongDash");
    assert_eq!(svg.matches("stroke-dasharray=").count(), 5, "Solid must not emit dasharray");
}

#[test]
fn xml_escaping_and_entity_preservation() {
    let mut scene = ChartScene::new("esc", "p < 0.001 & \"Group A's\" > B", 240.0, 100.0, None);
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![RichSpan {
            text: String::from("A < B & C > D \"q\" 's'"),
            font_family: Some(String::from("Noto & \"Sans\"")),
            ..RichSpan::plain("")
        }]),
        anchor_pt: PointPt::new(20.0, 50.0),
        anchor: TextAnchor::Start,
        font_size_pt: 11.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let svg = render_svg(&scene, &SvgExportOptions::default()).expect("render escaped");
    validate_svg_structure(&svg).expect("valid escaped SVG");
    assert!(svg.contains("<title>p &lt; 0.001 &amp; &quot;Group A&apos;s&quot; &gt; B</title>"));
    assert!(svg.contains(">A &lt; B &amp; C &gt; D &quot;q&quot; &apos;s&apos;</tspan>"));
    assert!(svg.contains("font-family=\"Noto &amp; &quot;Sans&quot;\""));
}

#[test]
fn validator_rejects_malformed_and_non_finite_svgs_and_invalid_scenes() {
    let mut bad_scene = ChartScene::new("bad", "Bad", 100.0, 100.0, None);
    bad_scene.primitives.push(ScenePrimitive::Path {
        path: Path2D::line(PointPt::new(0.0, f64::NAN), PointPt::new(10.0, 10.0)),
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    assert!(matches!(render_svg(&bad_scene, &SvgExportOptions::default()), Err(SvgError::InvalidScene { .. })));

    let mut bad_dim = ChartScene::new("bad-dim", "Bad", 100.0, 100.0, None);
    bad_dim.width_pt = 0.0;
    assert!(matches!(render_svg(&bad_dim, &SvgExportOptions::default()), Err(SvgError::InvalidScene { .. })));

    let invalid_svgs = [
        "",
        "<g xmlns=\"http://www.w3.org/2000/svg\"></g>",
        "<svg viewBox=\"0 0 100 100\"></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><g></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"></svg><g/>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 NaN 100\"></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><path d=\"M0 inf L10 10\"/></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text transform=\"rotate(NaN, 0, 0)\"></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A & B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A\u{0000}B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A\u{000B}B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A\u{0001}B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A\u{FFFE}B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>A\u{FFFF}B</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>&#0;</tspan></text></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\"><text><tspan>&#x0B;</tspan></text></svg>",
    ];
    for bad in invalid_svgs {
        assert!(validate_svg_structure(bad).is_err(), "expected validation failure for: {bad}");
    }

    // Valid scientific text content containing literal "NaN" or "Infinity" must still pass.
    let valid_nan_label =
        format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 100 100\"><text x=\"10\" y=\"20\"><tspan>NaN / Infinity</tspan></text></svg>");
    validate_svg_structure(&valid_nan_label).expect("literal text 'NaN' must be allowed");

    // Compact scientific path syntax must validate cleanly.
    let compact_path = format!("<svg xmlns=\"{SVG_XMLNS}\" viewBox=\"0 0 100 100\"><path d=\"M1e-2-3.5L10,20Z\" stroke-width=\"1pt\"/></svg>");
    validate_svg_structure(&compact_path).expect("compact SVG path syntax must validate");

    let defaults = SvgExportOptions::default();
    assert!(!defaults.outline_text);
    assert!(defaults.pretty);
    assert_eq!(defaults.precision, 3);
}

#[test]
fn compiled_chart_and_layout_scenes_export_valid_svg() {
    let mut doc = ProjectDocument::default();
    let archetype = TableArchetype::Xy(XyTable::new(SubcolumnMode::Single, 2, 6).expect("xy table"));
    let data = DataSheet::new(SheetId::new("data-1"), "Kinetics", archetype).expect("data sheet");
    let graph = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-1"), &data).expect("graph sheet");
    let layout = LayoutSheet::new(SheetId::new("layout-1"), "Figure 1", LayoutSpec::default()).expect("layout sheet");
    doc.add_data_sheet(data).expect("add data");
    doc.add_graph_sheet(graph.clone()).expect("add graph");
    doc.add_layout_sheet(layout.clone()).expect("add layout");

    let g_scene = compile_graph(&graph, &doc).expect("compile graph");
    let l_scene = compile_layout(&layout, &doc).expect("compile layout");

    for scene in [&g_scene, &l_scene] {
        for outline_text in [false, true] {
            let opts = SvgExportOptions { outline_text, ..SvgExportOptions::default() };
            let svg = render_svg(scene, &opts).expect("export compiled scene");
            validate_svg_structure(&svg).expect("validate compiled scene SVG");
        }
    }
}

#[test]
fn render_svg_strips_xml_forbidden_control_and_noncharacter_codepoints() {
    let mut scene = ChartScene::new("xml-ctrl", "Dose\u{0000}\u{000B}\u{FFFE}Curve\u{FFFF}", 240.0, 120.0, Some(Rgba8::WHITE));
    let mut span = RichSpan::plain("Group\u{000B}A\u{0001}B\u{000C}C\u{001F}D\u{FFFE}\u{FFFF}\t\n");
    span.font_family = Some(String::from("Helv\u{0007}etica"));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::new(vec![span]),
        anchor_pt: PointPt::new(20.0, 40.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        color: Rgba8::BLACK,
        rotation_deg: 0.0,
    }));

    let svg = render_svg(&scene, &SvgExportOptions::default()).expect("render SVG");
    let invalid: Vec<char> =
        svg.chars().filter(|&c| !matches!(c as u32, 0x09 | 0x0A | 0x0D | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10_FFFF)).collect();
    assert!(invalid.is_empty(), "SVG output contains forbidden XML 1.0 chars: {invalid:?}");
    assert!(svg.contains("<title>DoseCurve</title>"));
    assert!(svg.contains("font-family=\"Helvetica\""));
    validate_svg_structure(&svg).expect("sanitized SVG must validate");
}

fn cld_sample_document() -> (ProjectDocument, GraphSheet) {
    use curvebook_doc::{
        AnalysisKind, AnalysisSpec, BracketLabelMode, BracketStyle, Cell, ColumnDesignHint, ColumnTable, CompactLetterDisplaySpec, OneWayAnovaSpec,
        PairwiseBracketSpec, PairwiseComparisonRecord, PairwiseDisplayMode, ResultSheet, ResultStatus,
    };

    let mut table = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 4).expect("column table");
    for (col, (title, base)) in [("Ctrl", 10.0), ("Mid", 14.0), ("High", 22.0)].into_iter().enumerate() {
        let ds = table.data_sets.get_mut(col).expect("column dataset");
        ds.title = String::from(title);
        ds.cells = vec![vec![Cell::number(base - 1.0)], vec![Cell::number(base)], vec![Cell::number(base + 1.0)], vec![Cell::number(base)]];
    }
    let ds = DataSheet::new(SheetId::new("col-cld"), "Treatment", TableArchetype::Column(table)).expect("data sheet");
    let spec = AnalysisSpec::new(SheetId::new("res-cld"), SheetId::new("col-cld"), AnalysisKind::OneWayAnova(OneWayAnovaSpec::default()))
        .expect("anova spec");
    let mut res = ResultSheet::new(spec).expect("result sheet");
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

    let mut gs = GraphSheet::new_uninitialized_for_data(SheetId::new("graph-cld"), &ds).expect("graph sheet");
    gs.spec.title = String::from("CLD Export Title");
    gs.spec.compact_letter_display = Some(CompactLetterDisplaySpec::new(SheetId::new("res-cld")));
    gs.spec.pairwise_display_mode = PairwiseDisplayMode::CompactLetterDisplayOnly;
    gs.spec.comparisons = vec![PairwiseBracketSpec {
        result_sheet_id: SheetId::new("res-cld"),
        group_a: 0,
        group_b: 2,
        style: BracketStyle::Square,
        label_mode: BracketLabelMode::Custom(String::from("**sig**")),
        hide_if_non_significant: false,
        stroke_width_pt: 1.0,
        font_size_pt: 8.5,
        bracket_step_pt: 14.0,
    }];

    let mut doc = ProjectDocument::default();
    doc.add_data_sheet(ds).expect("add data");
    doc.add_result_sheet(res).expect("add result");
    doc.add_graph_sheet(gs.clone()).expect("add graph");
    (doc, gs)
}

fn extract_svg_text_xy(svg: &str, tspan_text: &str) -> (f64, f64) {
    let needle = format!(">{tspan_text}</tspan>");
    let line = svg.lines().find(|l| l.contains(&needle)).expect("SVG text line for CLD label");
    let parse_attr = |attr: &str| -> f64 {
        let key = format!("{attr}=\"");
        let (_, after_key) = line.split_once(&key).expect("SVG coordinate attribute");
        let (val, _) = after_key.split_once('"').expect("closing quote");
        val.parse::<f64>().expect("numeric SVG coordinate")
    };
    (parse_attr("x"), parse_attr("y"))
}

#[test]
fn cld_compiled_graph_exports_deterministic_editable_and_outlined_svg() {
    use curvebook_doc::PairwiseDisplayMode;

    let (doc, gs_cld) = cld_sample_document();
    let mut gs_ctl = gs_cld.clone();
    gs_ctl.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsOnly;
    gs_ctl.spec.compact_letter_display = None;
    gs_ctl.spec.comparisons.clear();

    let mut gs_both = gs_cld.clone();
    gs_both.spec.pairwise_display_mode = PairwiseDisplayMode::BracketsAndCompactLetterDisplay;

    let scene_cld = compile_graph(&gs_cld, &doc).expect("compile cld scene");
    let scene_ctl = compile_graph(&gs_ctl, &doc).expect("compile control scene");
    let scene_both = compile_graph(&gs_both, &doc).expect("compile cld+bracket scene");

    for outline_text in [false, true] {
        let opts = SvgExportOptions { outline_text, ..SvgExportOptions::default() };
        let svg_a = render_svg(&scene_cld, &opts).expect("first cld svg");
        let svg_b = render_svg(&scene_cld, &opts).expect("second cld svg");
        assert_eq!(svg_a, svg_b, "CLD SVG must be byte-deterministic (outline={outline_text})");
        validate_svg_structure(&svg_a).expect("valid CLD SVG");

        let svg_ctl = render_svg(&scene_ctl, &opts).expect("control svg");
        validate_svg_structure(&svg_ctl).expect("valid control SVG");
        assert_ne!(svg_a, svg_ctl, "CLD letters must change SVG geometry/text (outline={outline_text})");

        if outline_text {
            assert!(!svg_a.contains("<text"), "outlined SVG must emit glyph paths, not <text>");
            assert!(svg_a.contains("<path d="));
        } else {
            assert!(svg_a.contains(">a</tspan>"), "missing CLD letter `a` in SVG:\n{svg_a}");
            assert!(svg_a.contains(">ab</tspan>"), "missing CLD letter `ab` in SVG:\n{svg_a}");
            assert!(svg_a.contains(">b</tspan>"), "missing CLD letter `b` in SVG:\n{svg_a}");
            assert!(!svg_a.contains(">sig</tspan>"), "CompactLetterDisplayOnly must suppress bracket label");
            assert!(!svg_ctl.contains(">ab</tspan>"), "control SVG must omit CLD letters");

            let (xb, _) = extract_svg_text_xy(&svg_a, "b");
            let (xab, _) = extract_svg_text_xy(&svg_a, "ab");
            let (xa, _) = extract_svg_text_xy(&svg_a, "a");
            assert!(xb < xab && xab < xa, "expected Ctrl('b') < Mid('ab') < High('a') horizontally: {xb}, {xab}, {xa}");

            let svg_both = render_svg(&scene_both, &opts).expect("cld+bracket svg");
            validate_svg_structure(&svg_both).expect("valid combined SVG");
            let (_, y_sig) = extract_svg_text_xy(&svg_both, "sig");
            let (_, y_ab) = extract_svg_text_xy(&svg_both, "ab");
            assert!(y_sig + 8.0 < y_ab, "bracket label y={y_sig} must clear CLD label y={y_ab} without overlap");
        }
    }
}
