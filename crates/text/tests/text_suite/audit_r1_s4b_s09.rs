//! Regression tests for Stage 4B S09 typography & layout fixes in `curvebook-text`.

use std::time::Instant;

use curvebook_palette::Rgba8;
use curvebook_text::glyph_paths::{GlyphOutlineCmd, outline_rich_text, outline_rich_text_rotated, outline_rich_text_with_fonts};
use curvebook_text::metrics::{core14_advance_em, is_combining_mark, reorder_rich_text_bidi, rich_contains_rtl, split_rich_text_lines};
use curvebook_text::{BUNDLED_FONTS, RichSpan, RichText, SystemFontSet, measure_text_pt, parse_markup, parse_truetype_font_info};

use super::sfnt_fixture::{assemble_sfnt, build_cmap4_subtable, build_head, build_hhea, build_name_table, build_post_v3, wrap_cmap_subtables};

fn bbox_of_cmds(cmds: &[GlyphOutlineCmd]) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    let mut any = false;
    for cmd in cmds {
        let pts: &[(f64, f64)] = match *cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } | GlyphOutlineCmd::LineTo { x_pt, y_pt } => &[(x_pt, y_pt)],
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => &[(cx_pt, cy_pt), (x_pt, y_pt)],
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => &[(c1x_pt, c1y_pt), (c2x_pt, c2y_pt), (x_pt, y_pt)],
            GlyphOutlineCmd::Close => &[],
        };
        for &(x, y) in pts {
            any = true;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    any.then_some((min_x, max_x, min_y, max_y))
}

fn build_synthetic_sfnt_with_cmap4(num_glyphs: u16, segments: &[(u16, u16, i16, u16)], extra_glyph_ids: &[u16]) -> Option<Vec<u8>> {
    let cmap = wrap_cmap_subtables(&[(3, 1, build_cmap4_subtable(segments, extra_glyph_ids))]);
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    let mut hmtx = vec![0u8; usize::from(num_glyphs.max(1)) * 4];
    hmtx[0..2].copy_from_slice(&500u16.to_be_bytes());
    let mut os2 = vec![0u8; 78];
    os2[4..6].copy_from_slice(&400u16.to_be_bytes());
    Some(assemble_sfnt(
        0x0001_0000,
        vec![
            (*b"OS/2", os2),
            (*b"cmap", cmap),
            (*b"head", build_head(0)),
            (*b"hhea", build_hhea(800, -200, 0, 1)),
            (*b"hmtx", hmtx),
            (*b"maxp", maxp),
            (*b"name", build_name_table("Audit", "Audit")),
            (*b"post", build_post_v3()),
        ],
    ))
}

#[test]
fn r1_s09_09_cmap4_sparse_and_sentinel_segments_complete_fast() {
    let mut segments = Vec::with_capacity(2001);
    for i in 0..2000u16 {
        let start = i * 32;
        let end = (start + 31).min(0xFFFE);
        segments.push((start, end, 0i16, 0u16));
    }
    segments.push((0xFFFF, 0xFFFF, 1i16, 0u16));
    let sfnt = build_synthetic_sfnt_with_cmap4(1, &segments, &[]).expect("build synthetic sfnt");

    let t0 = Instant::now();
    let info = parse_truetype_font_info(&sfnt).expect("valid sparse SFNT");
    let elapsed = t0.elapsed();
    assert!(elapsed.as_secs_f64() < 0.05, "collect_cmap_format4 took too long: {elapsed:?}");
    assert!(info.cmap_basic.is_empty());

    // Valid mappings with negative wrapping idDelta still resolve accurately.
    let seg_valid = [(0x0041, 0x0043, -64i16, 0u16), (0xFFFF, 0xFFFF, 1i16, 0u16)];
    let sfnt_valid = build_synthetic_sfnt_with_cmap4(4, &seg_valid, &[]).expect("build valid sfnt");
    let valid_info = parse_truetype_font_info(&sfnt_valid).expect("valid mapped SFNT");
    assert_eq!(valid_info.cmap_basic, vec![(0x41, 1), (0x42, 2), (0x43, 3)]);
}

#[test]
fn r1_s09_07_accented_latin_and_combining_marks_do_not_collapse_to_fallback_box() {
    if !BUNDLED_FONTS.is_empty() {
        return;
    }
    let box_cmds = outline_rich_text_with_fonts(&RichText::plain("\u{4E2D}"), 0.0, 20.0, 12.0, &SystemFontSet::cleanroom_only())[0].commands.len();
    for ch in ['é', 'ü', 'ñ', 'ç', 'ß', 'ö', 'Á', 'Č', 'ğ', 'Ł', 'ł', 'Œ', 'œ', 'ř', 'š', 'ž', 'ð', 'þ', 'ø', 'Æ', 'æ'] {
        let s = ch.to_string();
        let glyphs = outline_rich_text(&RichText::plain(&s), 0.0, 20.0, 12.0);
        assert_eq!(glyphs.len(), 1, "single glyph for {ch}");
        assert!(!glyphs[0].commands.is_empty(), "non-empty contour for {ch}");
        assert_ne!(glyphs[0].commands.len(), box_cmds, "glyph {ch} ({s}) must not use 4-corner fallback box");
        assert!(matches!(glyphs[0].commands.last(), Some(GlyphOutlineCmd::Close)), "closed contour for {ch}");
    }

    // Combining marks must carry zero advance and sit above their base glyph rather than after advance.
    assert!(is_combining_mark('\u{0301}'));
    assert_eq!(core14_advance_em('\u{0301}', curvebook_text::SpanWeight::Regular, curvebook_text::SpanSlope::Upright), 0.0);
    let combined = outline_rich_text(&RichText::plain("e\u{0301}"), 0.0, 20.0, 12.0);
    assert_eq!(combined.len(), 2);
    assert_eq!(combined[1].advance_x_pt, 0.0);
    let Some(base_box) = bbox_of_cmds(&combined[0].commands) else {
        panic!("base glyph bbox");
    };
    let Some(mark_box) = bbox_of_cmds(&combined[1].commands) else {
        panic!("combining mark bbox");
    };
    assert!(mark_box.0 >= base_box.0 - 1.0 && mark_box.1 <= base_box.1 + 1.0, "mark {mark_box:?} must align horizontally over base {base_box:?}");
    assert!(mark_box.2 < base_box.2, "above combining mark {mark_box:?} must sit above base x-height {base_box:?}");
    assert_eq!(measure_text_pt(&RichText::plain("e\u{0301}"), 12.0).width_pt, measure_text_pt(&RichText::plain("é"), 12.0).width_pt);

    // Combining mark over uppercase sits cleanly above cap height.
    let cap_combined = outline_rich_text(&RichText::plain("E\u{0301}"), 0.0, 20.0, 12.0);
    let Some(cap_box) = bbox_of_cmds(&cap_combined[0].commands) else {
        panic!("cap E bbox");
    };
    let Some(cap_mark) = bbox_of_cmds(&cap_combined[1].commands) else {
        panic!("cap E mark bbox");
    };
    assert!(cap_mark.3 <= cap_box.2 + 0.5, "cap accent {cap_mark:?} must clear cap height {cap_box:?}");

    // Standalone combining mark renders with finite closed contour without collapsing to degenerate sliver.
    let solo = outline_rich_text(&RichText::plain("\u{0308}"), 10.0, 20.0, 12.0);
    let Some(solo_box) = bbox_of_cmds(&solo[0].commands) else {
        panic!("standalone mark bbox");
    };
    assert!(solo_box.0 >= 10.0 && (solo_box.1 - solo_box.0) > 0.5);
}

#[test]
fn r1_s09_08_and_06_shared_helpers_reorder_bidi_and_split_lines() {
    let rich = RichText::new(vec![
        RichSpan { color: Some(Rgba8::rgb(200, 0, 0)), ..RichSpan::plain("ש") },
        RichSpan { color: Some(Rgba8::rgb(0, 0, 200)), ..RichSpan::plain("לום 2026\r\nבוקר") },
    ]);
    assert!(rich_contains_rtl(&rich));
    let lines = split_rich_text_lines(&rich);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].plain_text(), "שלום 2026");
    assert_eq!(lines[1].plain_text(), "בוקר");

    let visual_line0 = reorder_rich_text_bidi(&lines[0]);
    assert_eq!(visual_line0.plain_text(), "2026 םולש");

    // outline_rich_text_rotated applies BiDi reordering automatically.
    let outlines = outline_rich_text_rotated(&RichText::plain("שלום 2026"), 0.0, 20.0, 12.0, 0.0);
    let chars: Vec<char> = outlines.iter().map(|g| g.codepoint).collect();
    assert_eq!(chars, vec!['2', '0', '2', '6', ' ', 'ם', 'ו', 'ל', 'ש']);

    // Multi-line markup maintains line count and ignores CR without breaking span styles.
    let ml = parse_markup("<b>Alpha</b>\r\n<i>Beta</i>");
    let ml_lines = split_rich_text_lines(&ml);
    assert_eq!(ml_lines.len(), 2);
    assert_eq!(ml_lines[0].plain_text(), "Alpha");
    assert_eq!(ml_lines[1].plain_text(), "Beta");
}
