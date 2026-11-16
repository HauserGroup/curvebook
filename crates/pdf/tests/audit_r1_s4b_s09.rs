//! Regression tests for Stage 4B S09 typography & layout fixes in `curvebook-pdf`.

use curvebook_chart::{ChartScene, PointPt, PositionedText, ScenePrimitive, TextAnchor};
use curvebook_palette::Rgba8;
use curvebook_pdf::{PdfExportOptions, render_pdf};
use curvebook_text::{RichSpan, RichText, SpanWeight, measure_text_pt};

fn inflate_stream(pdf: &[u8]) -> String {
    let text = String::from_utf8_lossy(pdf);
    let mut all = String::new();
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("stream\n") {
        let after = &rest[pos + 7..];
        if let Some(end) = after.find("\nendstream") {
            all.push_str(&after[..end]);
            all.push('\n');
            rest = &after[end + 10..];
        } else {
            break;
        }
    }
    all
}

fn extract_first_tm_xy(stream: &str) -> Option<(f64, f64)> {
    for line in stream.lines() {
        let trimmed = line.trim();
        if let Some(body) = trimmed.strip_suffix(" Tm") {
            let parts: Vec<&str> = body.split_whitespace().collect();
            if parts.len() == 6
                && let (Ok(x), Ok(y)) = (parts[4].parse::<f64>(), parts[5].parse::<f64>())
            {
                return Some((x, y));
            }
        }
    }
    None
}

fn extract_all_tms(stream: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for line in stream.lines() {
        let trimmed = line.trim();
        if let Some(body) = trimmed.strip_suffix(" Tm") {
            let parts: Vec<&str> = body.split_whitespace().collect();
            if parts.len() == 6
                && let (Ok(x), Ok(y)) = (parts[4].parse::<f64>(), parts[5].parse::<f64>())
            {
                out.push((x, y));
            }
        }
    }
    out
}

fn pdf_opts(outline_text: bool) -> PdfExportOptions {
    PdfExportOptions { compress_streams: false, outline_text, ..PdfExportOptions::default() }
}

#[test]
fn r1_s09_05_end_and_middle_anchor_offsets_match_text_crate_widths() {
    for label in ["Vehicle + inhibitor", "Lipid (mg/dl) @ 4 °C"] {
        let mut scene = ChartScene::new("g", "Width", 400.0, 200.0, Some(Rgba8::WHITE));
        let rich = RichText::plain(label);
        scene.primitives.push(ScenePrimitive::Text(PositionedText {
            rich: rich.clone(),
            anchor_pt: PointPt::new(300.0, 100.0),
            font_size_pt: 10.0,
            anchor: TextAnchor::End,
            rotation_deg: 0.0,
            color: Rgba8::BLACK,
        }));
        let pdf = render_pdf(&scene, &pdf_opts(false)).expect("render pdf");
        let stream = inflate_stream(&pdf);
        let (tm_x, _) = extract_first_tm_xy(&stream).expect("Tm missing");
        let expected = 300.0 - measure_text_pt(&rich, 10.0).width_pt;
        assert!((tm_x - expected).abs() < 0.02, "label={label:?}: pdf tm_x={tm_x:.3} expected={expected:.3}");
    }

    // Bold ASCII widths must also match authoritative Core-14 bold metrics.
    let mut bold_scene = ChartScene::new("g", "BoldWidth", 400.0, 200.0, Some(Rgba8::WHITE));
    let bold_rich = RichText::new(vec![RichSpan { weight: SpanWeight::Bold, ..RichSpan::plain("Mean +/- SD @ 2026") }]);
    bold_scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: bold_rich.clone(),
        anchor_pt: PointPt::new(200.0, 100.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Middle,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));
    let bold_pdf = render_pdf(&bold_scene, &pdf_opts(false)).expect("render bold pdf");
    let bold_stream = inflate_stream(&bold_pdf);
    let (bold_tm_x, _) = extract_first_tm_xy(&bold_stream).expect("bold Tm");
    let expected_mid = 200.0 - 0.5 * measure_text_pt(&bold_rich, 12.0).width_pt;
    assert!((bold_tm_x - expected_mid).abs() < 0.02, "bold tm_x={bold_tm_x:.3} expected={expected_mid:.3}");
}

#[test]
fn r1_s09_06_pdf_multiline_text_splits_into_lines_in_selectable_and_outlined_modes() {
    let mut scene = ChartScene::new("g", "Multiline", 400.0, 240.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Line one\nLine two"),
        anchor_pt: PointPt::new(200.0, 100.0),
        font_size_pt: 10.0,
        anchor: TextAnchor::End,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let selectable = render_pdf(&scene, &pdf_opts(false)).expect("selectable multiline pdf");
    let stream = inflate_stream(&selectable);
    assert!(!stream.contains("(Line one Line two) Tj"), "newlines must not collapse to spaces: {stream}");
    assert!(stream.contains("(Line one) Tj") && stream.contains("(Line two) Tj"));

    let tms = extract_all_tms(&stream);
    assert_eq!(tms.len(), 2);
    // Page height is 240: line 0 at y_pdf = 240 - 100 = 140, line 1 at y_pdf = 240 - (100 + 12) = 128.
    assert!((tms[0].1 - 140.0).abs() < 0.01);
    assert!((tms[1].1 - 128.0).abs() < 0.01);

    let expected_x0 = 200.0 - measure_text_pt(&RichText::plain("Line one"), 10.0).width_pt;
    let expected_x1 = 200.0 - measure_text_pt(&RichText::plain("Line two"), 10.0).width_pt;
    assert!((tms[0].0 - expected_x0).abs() < 0.02);
    assert!((tms[1].0 - expected_x1).abs() < 0.02);

    // Outlined mode produces non-empty distinct paths across two vertical rows.
    let outlined = render_pdf(&scene, &pdf_opts(true)).expect("outlined multiline pdf");
    let out_stream = inflate_stream(&outlined);
    assert!(out_stream.contains("f\nQ\n") || out_stream.contains("S\nQ\n"), "outlined glyph paths missing: {out_stream}");
    let move_ys: Vec<f64> =
        out_stream.lines().filter_map(|line| line.strip_suffix(" m")).filter_map(|xy| xy.split_whitespace().nth(1)?.parse::<f64>().ok()).collect();
    assert!(move_ys.iter().any(|&y| y > 139.0), "expected line 0 glyph y >= 139: {move_ys:?}");
    assert!(move_ys.iter().any(|&y| y < 137.0), "expected line 1 glyph y < 137: {move_ys:?}");
}

#[test]
fn r1_s09_08_pdf_outlined_and_selectable_bidi_avoid_double_reversal() {
    let mut scene = ChartScene::new("g", "BiDi", 400.0, 200.0, Some(Rgba8::WHITE));
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("שלום 2026"),
        anchor_pt: PointPt::new(100.0, 100.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let mut digits_only = ChartScene::new("g", "Digits", 400.0, 200.0, Some(Rgba8::WHITE));
    digits_only.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("2026"),
        anchor_pt: PointPt::new(100.0, 100.0),
        font_size_pt: 12.0,
        anchor: TextAnchor::Start,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let out_bidi = inflate_stream(&render_pdf(&scene, &pdf_opts(true)).expect("bidi outlined"));
    let out_digits = inflate_stream(&render_pdf(&digits_only, &pdf_opts(true)).expect("digits outlined"));
    let digits_core = out_digits.trim_end_matches('\n');
    assert!(!digits_core.is_empty());
    assert!(
        out_bidi.starts_with(digits_core),
        "outlined PDF must render '2026' first visually without double reversal:\nbidi:\n{out_bidi}\ndigits:\n{digits_core}"
    );

    let sel_bidi = inflate_stream(&render_pdf(&scene, &pdf_opts(false)).expect("bidi selectable"));
    let bt_start = sel_bidi.find("BT\n").unwrap_or_else(|| panic!("BT block missing: {sel_bidi}"));
    let pos_2026 = sel_bidi.find("(2026) Tj").unwrap_or_else(|| panic!("selectable digits missing: {sel_bidi}"));
    let pos_space =
        sel_bidi[pos_2026..].find("( ) Tj").map(|off| pos_2026 + off).unwrap_or_else(|| panic!("RTL space after digits missing: {sel_bidi}"));
    assert!(bt_start < pos_2026, "digits should appear in first BT block: {sel_bidi}");
    assert!(!sel_bidi[bt_start..pos_2026].contains(" m\n"), "Hebrew outlines must not precede '2026': {sel_bidi}");
    assert!(sel_bidi[pos_space..].contains(" m\n"), "Hebrew outlines must follow '2026 ': {sel_bidi}");
}
