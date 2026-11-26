//! Regression tests for audit P4 (`FA-SEC10-04`, `FA-SEC10-05`, `FA-SEC10-09`, `FA-SEC10-10`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use curvebook_testkit::TempDirGuard;
use curvebook_text::glyph_paths::{outline_ttf_glyph, outline_visual_rich_text_rotated_with_fonts_and_ttf};
use curvebook_text::metrics::reorder_rich_text_bidi_with_direction;
use curvebook_text::{RichSpan, RichText, SpanWeight, SystemFontSet, TextDirection, TrueTypeFont, outline_rich_text_with_fonts};

use super::sfnt_fixture::build_glyf_font;

fn build_synthetic_glyf_sfnt(family_name: &str, codepoint: u16, contours: u16) -> Vec<u8> {
    build_glyf_font(family_name, family_name, -200, codepoint, contours)
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

    // Combining mark mapped to an empty zero-contour glyph in preferred TTF must fall back to cleanroom vector outline.
    let empty_macron = build_glyf_font("EmptyMacron", "EmptyMacron", -200, 0x0304, 0);
    let empty_ttf = TrueTypeFont::parse_checked(&empty_macron).expect("parse empty macron font");
    assert!(outline_ttf_glyph(&empty_ttf, '\u{0304}', 0.0, 0.0, 12.0, false).expect("mapped empty").is_empty());
    let attached =
        outline_visual_rich_text_rotated_with_fonts_and_ttf(&RichText::plain("x\u{0304}"), 0.0, 0.0, 12.0, 0.0, &cleanroom, Some(&empty_ttf));
    assert_eq!(attached.len(), 2);
    assert_eq!(attached[1].codepoint, '\u{0304}');
    assert!(!attached[1].commands.is_empty(), "combining macron must fall back to cleanroom outline");
    let standalone =
        outline_visual_rich_text_rotated_with_fonts_and_ttf(&RichText::plain("\u{0304}"), 0.0, 0.0, 12.0, 0.0, &cleanroom, Some(&empty_ttf));
    assert_eq!(standalone.len(), 1);
    assert!(!standalone[0].commands.is_empty(), "standalone combining macron must fall back when TTF glyph is empty");
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

    // Repeated manifest entries deduplicate to 1 loaded face (no OOM amplification).
    let dedup_dir = root.join("dedup_bundle");
    let dedup_fonts = dedup_dir.join("fonts");
    std::fs::create_dir_all(&dedup_fonts).expect("mkdir dedup_fonts");
    std::fs::write(dedup_fonts.join("a.ttf"), &valid_a).expect("write a.ttf");
    std::fs::write(dedup_fonts.join("manifest.txt"), "a.ttf\n".repeat(1_000)).expect("write repeated manifest");
    assert_eq!(SystemFontSet::from_fonts_dir(&dedup_dir).face_count(), 1, "repeated manifest entries must deduplicate");

    // > MAX_DIR_FONTS (64) distinct manifest entries are capped at 64.
    let cap_dir = root.join("cap_bundle");
    let cap_fonts = cap_dir.join("fonts");
    std::fs::create_dir_all(&cap_fonts).expect("mkdir cap_fonts");
    let mut cap_manifest = String::new();
    for idx in 0..72u16 {
        let name = format!("f{idx:03}.ttf");
        std::fs::write(cap_fonts.join(&name), build_synthetic_glyf_sfnt(&format!("CapFont{idx}"), 0x0041, 1)).expect("write cap font");
        cap_manifest.push_str(&name);
        cap_manifest.push('\n');
    }
    std::fs::write(cap_fonts.join("manifest.txt"), &cap_manifest).expect("write cap manifest");
    assert_eq!(SystemFontSet::from_fonts_dir(&cap_dir).face_count(), 64, "manifest font loading must respect MAX_DIR_FONTS=64");

    // Symlinked font file, symlinked subdirectory, and symlinked manifest must all be rejected.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let outside_dir = root.join("outside_dir");
        std::fs::create_dir_all(&outside_dir).expect("mkdir outside_dir");
        let outside_font = outside_dir.join("secret.ttf");
        std::fs::write(&outside_font, &evil).expect("write outside font");

        let sym_bundle = root.join("sym_bundle");
        let sym_fonts = sym_bundle.join("fonts");
        std::fs::create_dir_all(&sym_fonts).expect("mkdir sym_fonts");
        symlink(&outside_font, sym_fonts.join("escape.ttf")).expect("symlink escape.ttf");
        symlink(&outside_dir, sym_fonts.join("sub")).expect("symlink sub dir");
        std::fs::write(sym_fonts.join("manifest.txt"), "escape.ttf\nsub/secret.ttf\n").expect("write sym manifest");
        assert_eq!(SystemFontSet::from_fonts_dir(&sym_bundle).face_count(), 0, "symlinked files/dirs must never load");

        let outside_manifest = root.join("outside_manifest.txt");
        std::fs::write(&outside_manifest, "a.ttf\n").expect("write outside manifest");
        let sym_manifest_bundle = root.join("sym_manifest_bundle");
        std::fs::create_dir_all(sym_manifest_bundle.join("fonts")).expect("mkdir");
        symlink(&outside_manifest, sym_manifest_bundle.join("fonts").join("manifest.txt")).expect("symlink manifest");
        assert_eq!(SystemFontSet::from_fonts_dir(&sym_manifest_bundle).face_count(), 0);
    }
}
