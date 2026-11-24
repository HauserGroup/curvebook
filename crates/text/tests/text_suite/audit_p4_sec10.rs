//! Regression tests for audit P4 (`FA-SEC10-04`, `FA-SEC10-05`, `FA-SEC10-09`, `FA-SEC10-10`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_testkit::TempDirGuard;
use curvebook_text::glyph_paths::outline_ttf_glyph;
use curvebook_text::metrics::reorder_rich_text_bidi_with_direction;
use curvebook_text::{RichSpan, RichText, SpanWeight, SystemFontSet, TextDirection, TrueTypeFont, outline_rich_text_with_fonts};

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

/// Build an SFNT TrueType font mapping `codepoint` -> `gid 1`, with either a 3-vertex triangle
/// glyph or a multi-contour glyph having `contours` triangles.
fn build_synthetic_glyf_sfnt(family_name: &str, codepoint: u16, contours: u16) -> Vec<u8> {
    let num_glyphs: u16 = 2;

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    // Short `loca` format (`indexToLocFormat = 0`).
    head[50..52].copy_from_slice(&0i16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[8..10].copy_from_slice(&90i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    let total_points = u16::try_from(usize::from(contours) * 3).expect("points fit u16");
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    maxp[6..8].copy_from_slice(&total_points.to_be_bytes());
    maxp[8..10].copy_from_slice(&contours.to_be_bytes());

    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    push_u16(&mut hmtx, 600);
    push_i16(&mut hmtx, 50);
    push_u16(&mut hmtx, 760);
    push_i16(&mut hmtx, 50);

    // Build simple `glyf` entry for `gid 1`: `contours` distinct triangles.
    let mut glyph1 = Vec::new();
    push_i16(&mut glyph1, i16::try_from(contours).expect("contours fit i16"));
    push_i16(&mut glyph1, 100);
    push_i16(&mut glyph1, 0);
    push_i16(&mut glyph1, 600);
    push_i16(&mut glyph1, 700);
    for c in 0..contours {
        push_u16(&mut glyph1, c * 3 + 2);
    }
    push_u16(&mut glyph1, 0); // instructionLength = 0
    // Flags: 0x01 (on-curve) | 0x02 (u8 x) | 0x10 (positive x) | 0x04 (u8 y) | 0x20 (positive y).
    let n_pts = usize::from(total_points);
    glyph1.resize(glyph1.len() + n_pts, 0x37);
    // x deltas
    for c in 0..contours {
        glyph1.push(if c == 0 { 10 } else { 1 });
        glyph1.push(20);
        glyph1.push(5);
    }
    // y deltas
    for c in 0..contours {
        glyph1.push(if c == 0 { 10 } else { 1 });
        glyph1.push(0);
        glyph1.push(25);
    }
    while !glyph1.len().is_multiple_of(4) {
        glyph1.push(0);
    }

    let glyf = glyph1;
    // Short `loca`: gid 0 empty [0, 0], gid 1 [0, glyf.len()/2].
    let mut loca = Vec::new();
    push_u16(&mut loca, 0);
    push_u16(&mut loca, 0);
    push_u16(&mut loca, u16::try_from(glyf.len() / 2).expect("short loca fits u16"));

    // Format 4 cmap mapping single BMP `codepoint` -> gid 1 (`delta = 1 - codepoint`).
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 3);
    push_u16(&mut cmap, 1);
    push_u32(&mut cmap, 12);
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 32);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 4); // segCountX2 = 4 (2 segments)
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, codepoint);
    push_u16(&mut cmap, 0xFFFF);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, codepoint);
    push_u16(&mut cmap, 0xFFFF);
    let delta = 1_i32 - i32::from(codepoint);
    push_u16(&mut cmap, u16::try_from(delta.rem_euclid(65_536)).expect("idDelta fits u16"));
    push_i16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 0);

    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    let mut str_buf = Vec::new();
    for unit in family_name.encode_utf16() {
        push_u16(&mut str_buf, unit);
    }
    let mut name = Vec::new();
    push_u16(&mut name, 0);
    push_u16(&mut name, 1);
    push_u16(&mut name, 18);
    push_u16(&mut name, 3);
    push_u16(&mut name, 1);
    push_u16(&mut name, 0x0409);
    push_u16(&mut name, 1);
    push_u16(&mut name, u16::try_from(str_buf.len()).expect("name len"));
    push_u16(&mut name, 0);
    name.extend_from_slice(&str_buf);

    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&4u16.to_be_bytes());
    os2[4..6].copy_from_slice(&400u16.to_be_bytes());
    os2[62..64].copy_from_slice(&0x0040u16.to_be_bytes());
    os2[68..70].copy_from_slice(&800i16.to_be_bytes());
    os2[70..72].copy_from_slice(&(-200i16).to_be_bytes());

    let tables = [
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"glyf", glyf),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"loca", loca),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, u16::try_from(tables.len()).expect("table count"));
    push_u16(&mut font, 128);
    push_u16(&mut font, 3);
    push_u16(&mut font, 4);

    let mut offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, u32::try_from(offset).expect("table offset"));
        push_u32(&mut font, u32::try_from(data.len()).expect("table len"));
        offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        font.extend_from_slice(data);
        while !font.len().is_multiple_of(4) {
            font.push(0);
        }
    }
    font
}

#[test]
fn sec10_04_non_latin_outlining_uses_system_font_set_while_cleanroom_stays_deterministic() {
    let dir_guard = TempDirGuard::new("p4-sec10-text-non-latin-outline").expect("create temp font dir");
    let dir = dir_guard.path();
    let fonts_sub = dir.join("fonts");
    std::fs::create_dir_all(&fonts_sub).expect("mkdir fonts");
    let cjk_ttf = build_synthetic_glyf_sfnt("CjkTriangleSans", 0x4E2D, 1);
    std::fs::write(fonts_sub.join("CjkTriangleSans.ttf"), &cjk_ttf).expect("write ttf");
    std::fs::write(fonts_sub.join("manifest.txt"), "CjkTriangleSans.ttf\n").expect("write manifest");

    let custom_fonts = SystemFontSet::from_fonts_dir(dir);
    assert_eq!(custom_fonts.face_count(), 1);

    let cleanroom = SystemFontSet::cleanroom_only();
    let glyphs = outline_rich_text_with_fonts(&RichText::plain("A中"), 0.0, 0.0, 12.0, &custom_fonts);
    let clean_a = outline_rich_text_with_fonts(&RichText::plain("A"), 0.0, 0.0, 12.0, &cleanroom);
    assert_eq!(glyphs.len(), 2);
    // Cleanroom Latin 'A' remains deterministic (matches cleanroom_only identically).
    assert_eq!(glyphs[0].commands, clean_a[0].commands);
    // Non-Latin U+4E2D resolves to the synthetic triangle SFNT glyph (5 cmds) instead of the 10-cmd hollow tofu box.
    assert_eq!(glyphs[1].codepoint, '中');
    assert_eq!(glyphs[1].commands.len(), 5, "expected synthetic triangle SFNT contour: {:?}", glyphs[1].commands);

    // Without host/custom fonts, `cleanroom_only()` falls back deterministically to the 2-contour hollow box (10 cmds).
    let fallback = outline_rich_text_with_fonts(&RichText::plain("中"), 0.0, 0.0, 12.0, &cleanroom);
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].commands.len(), 10, "expected cleanroom hollow tofu box (10 cmds): {:?}", fallback[0].commands);
    assert_ne!(glyphs[1].commands, fallback[0].commands, "custom SFNT triangle must replace cleanroom tofu box");
}

#[test]
fn sec10_05_excess_ttf_contours_are_rejected_early_not_silently_truncated() {
    let valid_at_cap = build_synthetic_glyf_sfnt("AtCap", 0x0041, 256);
    let font_ok = TrueTypeFont::parse(&valid_at_cap).expect("parse 256-contour SFNT");
    let cmds_ok = outline_ttf_glyph(&font_ok, 'A', 0.0, 0.0, 12.0, false).expect("256 contours allowed");
    assert_eq!(cmds_ok.len(), 256 * 5);

    let over_cap = build_synthetic_glyf_sfnt("OverCap", 0x0041, 257);
    let font_over = TrueTypeFont::parse(&over_cap).expect("parse 257-contour SFNT");
    assert!(
        outline_ttf_glyph(&font_over, 'A', 0.0, 0.0, 12.0, false).is_none(),
        "TTF glyphs with > 256 contours must return None rather than truncating to 256"
    );
}

#[test]
fn sec10_09_rtl_directed_multispan_reordering_respects_explicit_paragraph_direction() {
    // Mixed neutral + LTR across multiple spans behaves differently under explicit RTL base direction
    // vs auto-inferred LTR direction (neutral '-' adopts RTL direction and moves to the right of LTR run).
    let multi = RichText::new(vec![RichSpan::plain("- "), RichSpan { weight: SpanWeight::Bold, ..RichSpan::plain("IC50") }]);

    let ltr_visual = reorder_rich_text_bidi_with_direction(&multi, TextDirection::Ltr);
    let ltr_str: String = ltr_visual.spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(ltr_str, "- IC50");

    let rtl_visual = reorder_rich_text_bidi_with_direction(&multi, TextDirection::Rtl);
    let rtl_str: String = rtl_visual.spans.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(rtl_str, "IC50 -", "RTL container base direction must reorder leading neutral across spans: {rtl_str:?}");
}

#[test]
fn sec10_10_manifest_path_containment_allows_double_dots_in_names_and_rejects_traversal() {
    let root_guard = TempDirGuard::new("p4-sec10-text-manifest-containment").expect("create temp font dir");
    let root = root_guard.path();
    let bundle = root.join("bundle");
    let fonts_sub = bundle.join("fonts");
    let nested_sub = fonts_sub.join("sub");
    std::fs::create_dir_all(&nested_sub).expect("mkdir sub");

    let valid_a = build_synthetic_glyf_sfnt("DotName", 0x0041, 1);
    let valid_b = build_synthetic_glyf_sfnt("NestedDot", 0x0042, 1);
    let evil = build_synthetic_glyf_sfnt("EvilEscape", 0x4E2D, 1);

    std::fs::write(fonts_sub.join("Foo..Bar.ttf"), &valid_a).expect("write Foo..Bar.ttf");
    std::fs::write(nested_sub.join("Nested..Font.ttf"), &valid_b).expect("write Nested..Font.ttf");
    // Place outside.ttf in `root` (parent of `bundle`, two levels above `bundle/fonts/`).
    std::fs::write(root.join("outside.ttf"), &evil).expect("write outside.ttf");

    let abs_escape = root.join("outside.ttf").to_string_lossy().into_owned();
    let manifest = format!(
        "# test manifest\n\
         Foo..Bar.ttf\n\
         sub/Nested..Font.ttf\n\
         ../outside.ttf\n\
         ../../outside.ttf\n\
         sub/../Foo..Bar.ttf\n\
         {abs_escape}\n\
         \\escaped.ttf\n"
    );
    std::fs::write(fonts_sub.join("manifest.txt"), manifest).expect("write manifest");

    let loaded = SystemFontSet::from_fonts_dir(&bundle);
    assert_eq!(loaded.face_count(), 2, "Foo..Bar.ttf and sub/Nested..Font.ttf must load while ../, ../../, sub/../, and /abs paths are rejected");
}
