//! Integration tests for `R10.9` L2 proportional UI text shaping, tabular-numeral alignment,
//! multi-script Unicode coverage, BiDi RTL ordering, HiDPI scaling, `CURVEBOOK_FONTS_DIR`
//! discovery, and `missing_glyph` reporting.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use curvebook_testkit::assert_approx_eq;
use curvebook_text::{
    FontFamilyKind, MeasuredTextMetrics, SpanSlope, SpanWeight, SystemFontSet, TextDirection, TextMeasurer, TrueTypeFont, UiTextStyle,
    build_minimal_truetype_font, has_cleanroom_vector_outline,
};

static TMP_SEQ: AtomicU64 = AtomicU64::new(1);

fn temp_font_dir(label: &str) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("cb-r10-ui-shaping-{label}-{}-{seq}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp font dir");
    dir
}

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

/// Build an SFNT font with custom per-glyph advance widths for testing proportional vs tabular metrics
/// and `CURVEBOOK_FONTS_DIR` override priority.
/// Maps ASCII `' '..='~'` to `gid 1..=95`, where `override_widths` sets `(ch, advance_1000)`.
fn build_custom_advance_sfnt(family_name: &str, override_widths: &[(char, u16)], is_fixed_pitch: bool) -> Vec<u8> {
    let num_glyphs: u16 = 96; // gid 0 (.notdef) + 95 printable ASCII (32..=126)

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&820i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-210i16).to_be_bytes());
    hhea[8..10].copy_from_slice(&90i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    push_u16(&mut hmtx, 550);
    push_i16(&mut hmtx, 40);
    for code in 32u8..=126u8 {
        let ch = char::from(code);
        let default_w = match ch {
            'i' | 'l' | '.' | ',' => 220,
            'W' | 'M' => 920,
            '0'..='9' => 560,
            'E' => 680,
            'e' => 540,
            '-' => 340,
            '+' => 590,
            _ => 500,
        };
        let adv = override_widths.iter().find(|&&(c, _)| c == ch).map_or(default_w, |&(_, w)| w);
        push_u16(&mut hmtx, adv);
        push_i16(&mut hmtx, 30);
    }

    // `cmap` format 4 mapping 32..=126 -> gid 1..=95 (delta = -31), sentinel 0xFFFF -> 0.
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 3);
    push_u16(&mut cmap, 1);
    push_u32(&mut cmap, 12);
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 32);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 4); // segCountX2
    push_u16(&mut cmap, 4);
    push_u16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 126);
    push_u16(&mut cmap, 0xFFFF);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 32);
    push_u16(&mut cmap, 0xFFFF);
    push_i16(&mut cmap, -31);
    push_i16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 0);

    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    if is_fixed_pitch {
        post[12..16].copy_from_slice(&1u32.to_be_bytes());
    }

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
    push_u16(&mut name, u16::try_from(str_buf.len()).unwrap());
    push_u16(&mut name, 0);
    name.extend_from_slice(&str_buf);

    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&4u16.to_be_bytes());
    os2[4..6].copy_from_slice(&400u16.to_be_bytes());
    os2[62..64].copy_from_slice(&0x0040u16.to_be_bytes());
    os2[68..70].copy_from_slice(&820i16.to_be_bytes());
    os2[70..72].copy_from_slice(&(-210i16).to_be_bytes());

    let tables = [
        (*b"OS/2", os2),
        (*b"cmap", cmap),
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"hmtx", hmtx),
        (*b"maxp", maxp),
        (*b"name", name),
        (*b"post", post),
    ];
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, u16::try_from(tables.len()).unwrap());
    push_u16(&mut font, 128);
    push_u16(&mut font, 3);
    push_u16(&mut font, 0);

    let mut offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, u32::try_from(offset).unwrap());
        push_u32(&mut font, u32::try_from(data.len()).unwrap());
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

/// Build an in-memory TrueType Collection (`.ttc`) containing two distinct SFNT sub-fonts.
fn build_synthetic_ttc() -> Vec<u8> {
    let f1 = build_custom_advance_sfnt("Synthetic Sans", &[('A', 660)], false);
    let mono_widths: Vec<(char, u16)> = (32u8..=126u8).map(|c| (char::from(c), 600)).collect();
    let f2 = build_custom_advance_sfnt("Synthetic Mono", &mono_widths, true);

    let header_len = 12 + 8; // ttcf + version + numFonts=2 + 2 u32 offsets
    let off1 = header_len;
    let off2 = header_len + f1.len();

    let mut ttc = Vec::new();
    ttc.extend_from_slice(b"ttcf");
    push_u32(&mut ttc, 0x0001_0000);
    push_u32(&mut ttc, 2);
    push_u32(&mut ttc, u32::try_from(off1).unwrap());
    push_u32(&mut ttc, u32::try_from(off2).unwrap());

    // Shift SFNT table offsets inside f1 and f2 by their starting offset inside the .ttc container.
    fn shift_table_offsets(mut sfnt: Vec<u8>, base: usize) -> Vec<u8> {
        let num_tables = u16::from_be_bytes([sfnt[4], sfnt[5]]);
        for i in 0..usize::from(num_tables) {
            let rec = 12 + i * 16 + 8;
            let raw = u32::from_be_bytes([sfnt[rec], sfnt[rec + 1], sfnt[rec + 2], sfnt[rec + 3]]);
            let shifted = u32::try_from(usize::try_from(raw).unwrap() + base).unwrap();
            sfnt[rec..rec + 4].copy_from_slice(&shifted.to_be_bytes());
        }
        sfnt
    }

    ttc.extend_from_slice(&shift_table_offsets(f1, off1));
    ttc.extend_from_slice(&shift_table_offsets(f2, off2));
    ttc
}

#[test]
fn mixed_case_proportional_widths_and_mono_uniformity() {
    let sets = [SystemFontSet::cleanroom_only(), SystemFontSet::from_env_and_system()];
    for set in &sets {
        for family in [FontFamilyKind::UiSans, FontFamilyKind::ScientificSerif] {
            let style = UiTextStyle::new(family, 14.0);
            let w_i = set.measure(&style, "iiii", 96).width_px;
            let w_a = set.measure(&style, "aaaa", 96).width_px;
            let w_w = set.measure(&style, "WWWW", 96).width_px;
            assert!(w_i > 0.0 && w_i < w_a && w_a < w_w, "{family:?}: expected i ({w_i}) < a ({w_a}) < W ({w_w})");

            // Accented mixed-case proportionality: narrow î < é < wide Ŵ.
            let w_i_hat = set.measure(&style, "îîî", 96).width_px;
            let w_e_acute = set.measure(&style, "ééé", 96).width_px;
            let w_w_hat = set.measure(&style, "ŴŴŴ", 96).width_px;
            assert!(w_i_hat < w_e_acute && w_e_acute < w_w_hat, "{family:?}: expected î < é < Ŵ");

            // Case sensitivity: mixed-case string preserves distinct proportional glyph advances.
            let shaped = set.shape_line(&style, "iW", 96);
            assert_eq!(shaped.glyphs.len(), 2);
            assert!(shaped.glyphs[0].advance_px < shaped.glyphs[1].advance_px);
            assert_approx_eq(shaped.glyphs[1].x_px, shaped.glyphs[0].advance_px, 1e-9, 1e-9, "second glyph x_px");
        }

        // UiMono enforces equal cell widths for narrow 'i' and wide 'W'.
        let mono = UiTextStyle::ui_mono(14.0);
        let mono_i = set.measure(&mono, "iiii", 96).width_px;
        let mono_w = set.measure(&mono, "WWWW", 96).width_px;
        assert_approx_eq(mono_i, mono_w, 1e-6, 1e-6, "UiMono width uniformity");
    }
}

#[test]
fn tabular_numerals_share_uniform_advance_across_cleanroom_custom_and_host_fonts() {
    // Deliberately make '1' narrow and 'E' wide in custom SFNT hmtx to verify tabular override.
    let irregular_sfnt = build_custom_advance_sfnt("Irregular Sans", &[('1', 380), ('9', 580), ('.', 210), ('E', 710), ('e', 520)], false);
    let custom_set = SystemFontSet::from_sfnt_bytes(&[("IrregularSans.ttf", &irregular_sfnt)]);
    let sets = [SystemFontSet::cleanroom_only(), custom_set, SystemFontSet::from_env_and_system()];

    let tabular_chars = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', ',', '-', '+', 'E', 'e', '\u{2212}'];

    for set in &sets {
        for style in [UiTextStyle::grid_tabular(13.0), UiTextStyle::ui_sans(13.0).with_tabular_nums(true)] {
            let base_w = set.measure(&style, "0", 96).width_px;
            assert!(base_w > 0.0);
            for ch in tabular_chars {
                let s = ch.to_string();
                let w = set.measure(&style, &s, 96).width_px;
                assert_approx_eq(w, base_w, 1e-6, 1e-6, &format!("tabular char {ch:?} advance"));
            }

            // Two equal-length scientific values with different digits/signs/exponents align pixel-for-pixel.
            let a = set.measure(&style, "-1,234.56E+07", 144).width_px;
            let b = set.measure(&style, "+9,870.12e-89", 144).width_px;
            assert_approx_eq(a, b, 1e-6, 1e-6, "tabular scientific column alignment");

            // Non-numeric labels in GridTabular still stay proportional ('i' < 'W').
            assert!(set.measure(&style, "iii", 96).width_px < set.measure(&style, "WWW", 96).width_px);
        }
    }

    // Proportional UiSans without tabular_nums preserves natural '.' vs 'W' difference.
    let prop_style = UiTextStyle::ui_sans(13.0);
    let dot_w = sets[0].measure(&prop_style, ".", 96).width_px;
    let zero_w = sets[0].measure(&prop_style, "0", 96).width_px;
    assert!(dot_w < zero_w);
}

#[test]
fn greek_math_sub_super_latin_and_cyrillic_have_zero_missing_glyphs_even_without_system_fonts() {
    let cleanroom = SystemFontSet::cleanroom_only();
    let host = SystemFontSet::from_env_and_system();

    let greek_lower = "αβγδεζηθικλμνξοπρσςτυφχψω";
    let greek_upper = "ΑΒΓΔΕΖΗΘΙΚΛΜΝΞΟΠΡΣΤΥΦΧΨΩ";
    let math_syms = "≤≥±×÷≠≈∞√∑−·°′″→←↔↑↓";
    let sub_super = "⁰¹²³⁴⁵⁶⁷⁸⁹⁻⁺₀₁₂₃₄₅₆₇₈₉₋₊";
    let accented = "ÀÇÉÖÜßàáâãäçèéêëíîñóôõöùúüąćęğİıłńŚśŞşźżČčřšž";
    let cyrillic = "АБВГДЕЁЖЗИЙКЛМНОПРСТУФХЦЧШЩЪЫЬЭЮЯабвгдеёжзийклмнопрстуфхцчшщъыьэюя";

    let style = UiTextStyle::ui_sans(14.0);
    for (label, sample) in [
        ("greek_lower", greek_lower),
        ("greek_upper", greek_upper),
        ("math", math_syms),
        ("sub_super", sub_super),
        ("accented_latin", accented),
        ("cyrillic", cyrillic),
    ] {
        // Every cleanroom glyph reports missing_glyph == false and used_vector_fallback == true.
        let shaped_cr = cleanroom.shape_line(&style, sample, 96);
        assert_eq!(shaped_cr.missing_glyph_count, 0, "cleanroom missing glyph in {label}");
        assert!(shaped_cr.metrics.width_px > 0.0);
        assert!(shaped_cr.glyphs.iter().all(|g| !g.missing_glyph && g.used_vector_fallback && g.glyph_id.is_none()));

        for ch in sample.chars() {
            assert!(has_cleanroom_vector_outline(ch), "cleanroom outline missing for {label} {ch:?}");
        }

        // Host system fonts also cover all of them cleanly with zero missing glyphs.
        let shaped_host = host.shape_line(&style, sample, 96);
        assert_eq!(shaped_host.missing_glyph_count, 0, "host missing glyph in {label}");
        assert!(shaped_host.metrics.width_px > 0.0);
    }

    // Rich markup mode (`rich: true`) handles subscripts/superscripts with vertical offsets and bounds.
    let rich_style = UiTextStyle::ui_sans(14.0).with_rich(true);
    let plain_num = cleanroom.measure(&style, "10", 96);
    let sup_line = cleanroom.shape_line(&rich_style, "10^{−6}", 96);
    let sub_line = cleanroom.shape_line(&rich_style, "<b>IC<sub>50</sub></b>", 96);
    assert_eq!(sup_line.visual_text, "10−6");
    assert!(sup_line.metrics.ascent_px > plain_num.ascent_px);
    assert!(sup_line.glyphs.iter().any(|g| g.ch == '6' && g.y_px < 0.0));
    assert_eq!(sub_line.visual_text, "IC50");
    assert!(sub_line.metrics.descent_px > plain_num.descent_px);
    assert!(sub_line.glyphs.iter().any(|g| g.ch == '5' && g.y_px > 0.0));
}

#[test]
fn cjk_and_bidi_rtl_shape_cleanly_at_all_hidpi_scales() {
    let cleanroom = SystemFontSet::cleanroom_only();
    let host = SystemFontSet::from_env_and_system();
    let style = UiTextStyle::ui_sans(14.0);
    let scales = [(96u16, 1.0_f64), (144, 1.5), (192, 2.0), (288, 3.0)];

    let cjk_samples = [("ja", "日本語 用量反応 IC50"), ("zh-CN", "中文 剂量反应"), ("zh-TW", "繁體中文 劑量反應"), ("ko", "한국어 용량 반응")];
    let rtl_samples = [("ar", "تحليل الجرعة (IC50)"), ("he", "עקומת תגובה (p < 0.05)"), ("he_num", "שלום 2026")];

    for set in [&cleanroom, &host] {
        for (locale, sample) in cjk_samples {
            let base = set.measure(&style, sample, 96);
            assert!(base.width_px > 0.0 && base.ascent_px > 0.0 && base.descent_px > 0.0 && base.line_height_px > 0.0, "{locale}");
            for &(dpi, factor) in &scales {
                let scaled = set.measure(&style, sample, dpi);
                assert_approx_eq(scaled.width_px, base.width_px * factor, 1e-9, 1e-9, &format!("{locale} width @ {factor}x"));
                assert_approx_eq(scaled.line_height_px, base.line_height_px * factor, 1e-9, 1e-9, &format!("{locale} lh @ {factor}x"));
            }
        }

        for (locale, sample) in rtl_samples {
            let shaped_1x = set.shape_line(&style, sample, 96);
            assert_eq!(shaped_1x.direction, TextDirection::Rtl, "{locale}");
            assert!(!shaped_1x.visual_text.is_empty());
            assert!(shaped_1x.metrics.width_px > 0.0);
            if locale == "he_num" {
                assert_eq!(shaped_1x.visual_text, "2026 םולש");
            }
            if locale == "ar" {
                assert!(shaped_1x.visual_text.contains("(IC50)"));
            }
            for &(dpi, factor) in &scales {
                let shaped_n = set.shape_line(&style, sample, dpi);
                assert_eq!(shaped_n.visual_text, shaped_1x.visual_text);
                assert_approx_eq(shaped_n.metrics.width_px, shaped_1x.metrics.width_px * factor, 1e-9, 1e-9, &format!("{locale} rtl @ {factor}x"));
                assert_approx_eq(shaped_n.metrics.ascent_px, shaped_1x.metrics.ascent_px * factor, 1e-9, 1e-9, &format!("{locale} asc @ {factor}x"));
                assert_approx_eq(
                    shaped_n.metrics.descent_px,
                    shaped_1x.metrics.descent_px * factor,
                    1e-9,
                    1e-9,
                    &format!("{locale} desc @ {factor}x"),
                );
            }
        }
    }

    // On macOS hosts with Arial Unicode / SFArabic / SFHebrew, all CJK & RTL samples resolve to SFNT glyph IDs.
    if cfg!(target_os = "macos") && std::path::Path::new("/System/Library/Fonts/Supplemental/Arial Unicode.ttf").is_file() {
        for (locale, sample) in cjk_samples.into_iter().chain(rtl_samples) {
            let shaped = host.shape_line(&style, sample, 96);
            assert_eq!(shaped.missing_glyph_count, 0, "{locale}: expected 0 missing glyphs on macOS host");
            assert!(
                shaped.glyphs.iter().filter(|g| !g.ch.is_whitespace()).all(|g| g.glyph_id.is_some() && !g.used_vector_fallback),
                "{locale}: expected SFNT glyph_ids for all non-whitespace chars: {:?}",
                shaped.glyphs
            );
        }
    }
}

#[test]
fn hidpi_scaling_is_strictly_proportional_for_multiline_and_rich_scripts() {
    let host = SystemFontSet::from_env_and_system();
    let rich_style = UiTextStyle::ui_sans(13.0).with_rich(true);
    let text = "<b>IC<sub>50</sub></b> = 10^{−6} M\n<i>R<sup>2</sup></i> = 0.998";
    let base = host.shape_line(&rich_style, text, 96);

    for (dpi, scale) in [(144u16, 1.5_f64), (192, 2.0), (288, 3.0)] {
        let shaped = host.shape_line(&rich_style, text, dpi);
        assert_approx_eq(shaped.metrics.width_px, base.metrics.width_px * scale, 1e-9, 1e-9, "width scale");
        assert_approx_eq(shaped.metrics.ascent_px, base.metrics.ascent_px * scale, 1e-9, 1e-9, "ascent scale");
        assert_approx_eq(shaped.metrics.descent_px, base.metrics.descent_px * scale, 1e-9, 1e-9, "descent scale");
        assert_approx_eq(shaped.metrics.line_height_px, base.metrics.line_height_px * scale, 1e-9, 1e-9, "line_height scale");
        assert_eq!(shaped.glyphs.len(), base.glyphs.len());
        for (g_s, g_b) in shaped.glyphs.iter().zip(&base.glyphs) {
            assert_eq!(g_s.ch, g_b.ch);
            assert_approx_eq(g_s.x_px, g_b.x_px * scale, 1e-9, 1e-9, "glyph x scale");
            assert_approx_eq(g_s.y_px, g_b.y_px * scale, 1e-9, 1e-9, "glyph y scale");
            assert_approx_eq(g_s.advance_px, g_b.advance_px * scale, 1e-9, 1e-9, "glyph advance scale");
        }
    }
}

#[test]
fn missing_glyph_and_vector_fallback_flags_report_accurately() {
    let style = UiTextStyle::ui_sans(14.0);

    // 1. Cleanroom fallback: ASCII/Greek -> vector fallback; unmapped CJK/PUA -> missing_glyph.
    let cleanroom = SystemFontSet::cleanroom_only();
    let cr_mixed = cleanroom.shape_line(&style, "Aα 中\u{E000}", 96);
    assert_eq!(cr_mixed.glyphs.len(), 5);
    assert!(cr_mixed.glyphs[0].used_vector_fallback && !cr_mixed.glyphs[0].missing_glyph && cr_mixed.glyphs[0].glyph_id.is_none());
    assert!(cr_mixed.glyphs[1].used_vector_fallback && !cr_mixed.glyphs[1].missing_glyph && cr_mixed.glyphs[1].glyph_id.is_none());
    assert!(!cr_mixed.glyphs[2].used_vector_fallback && !cr_mixed.glyphs[2].missing_glyph && cr_mixed.glyphs[2].glyph_id.is_none()); // space
    assert!(!cr_mixed.glyphs[3].used_vector_fallback && cr_mixed.glyphs[3].missing_glyph && cr_mixed.glyphs[3].glyph_id.is_none()); // '中'
    assert!(!cr_mixed.glyphs[4].used_vector_fallback && cr_mixed.glyphs[4].missing_glyph && cr_mixed.glyphs[4].glyph_id.is_none()); // PUA
    assert_eq!(cr_mixed.missing_glyph_count, 2);

    // 2. Minimal SFNT in memory: ASCII + Latin-1 + selected Greek mapped in SFNT; '≤' uses vector fallback; PUA is missing.
    let minimal = build_minimal_truetype_font();
    let min_set = SystemFontSet::from_sfnt_bytes(&[("Minimal.ttf", &minimal)]);
    let min_shaped = min_set.shape_line(&style, "Aα≤\u{E001}", 96);
    assert_eq!(min_shaped.glyphs.len(), 4);
    assert!(min_shaped.glyphs[0].glyph_id.is_some() && !min_shaped.glyphs[0].used_vector_fallback && !min_shaped.glyphs[0].missing_glyph);
    assert!(min_shaped.glyphs[1].glyph_id.is_some() && !min_shaped.glyphs[1].used_vector_fallback && !min_shaped.glyphs[1].missing_glyph);
    assert!(min_shaped.glyphs[2].glyph_id.is_none() && min_shaped.glyphs[2].used_vector_fallback && !min_shaped.glyphs[2].missing_glyph);
    assert!(min_shaped.glyphs[3].glyph_id.is_none() && !min_shaped.glyphs[3].used_vector_fallback && min_shaped.glyphs[3].missing_glyph);
    assert_eq!(min_shaped.missing_glyph_count, 1);

    // 3. Host SFNT set also reports missing_glyph for unmapped Private Use / non-character codepoints.
    let host = SystemFontSet::from_env_and_system();
    let host_pua = host.shape_line(&style, "Ok\u{10FFFD}", 96);
    assert_eq!(host_pua.missing_glyph_count, 1);
    assert!(host_pua.glyphs[2].missing_glyph);
}

#[test]
fn ttc_collection_and_fonts_dir_manifest_discovery_work_and_override_host() {
    // TrueType Collection (.ttc) parsing exposes both embedded sub-faces cleanly.
    let ttc_bytes = build_synthetic_ttc();
    let parsed = TrueTypeFont::parse_collection(&ttc_bytes);
    assert_eq!(parsed.len(), 2);
    let ttc_set = SystemFontSet::from_sfnt_bytes(&[("Family.ttc", &ttc_bytes)]);
    assert_eq!(ttc_set.face_count(), 2);
    let mono_shaped = ttc_set.shape_line(&UiTextStyle::ui_mono(10.0), "iW", 96);
    assert_approx_eq(mono_shaped.glyphs[0].advance_px, 6.0, 1e-6, 1e-6, "ttc mono i");
    assert_approx_eq(mono_shaped.glyphs[1].advance_px, 6.0, 1e-6, 1e-6, "ttc mono W");

    // CURVEBOOK_FONTS_DIR layout with `fonts/manifest.txt`.
    let dir = temp_font_dir("manifest");
    let fonts_sub = dir.join("fonts");
    std::fs::create_dir_all(&fonts_sub).expect("mkdir fonts");
    let custom_bytes = build_custom_advance_sfnt("Override Sans", &[('A', 900)], false);
    std::fs::write(fonts_sub.join("Override-Regular.ttf"), &custom_bytes).expect("write ttf");
    std::fs::write(fonts_sub.join("manifest.txt"), "# pinned test font\nOverride-Regular.ttf\n../escape.ttf\n").expect("write manifest");

    let dir_set = SystemFontSet::from_fonts_dir(&dir);
    assert_eq!(dir_set.face_count(), 1);
    let w = dir_set.measure(&UiTextStyle::ui_sans(10.0), "A", 96).width_px;
    assert_approx_eq(w, 9.0, 1e-6, 1e-6, "dir override advance 900/1000 * 10");

    // Verify CURVEBOOK_FONTS_DIR env override inside a clean child process without unsafe set_var.
    if std::env::var_os("CB_R10_FONT_ENV_CHILD").as_deref() == Some(std::ffi::OsStr::new("1")) {
        let env_set = SystemFontSet::from_env_and_system();
        let env_w = env_set.measure(&UiTextStyle::ui_sans(10.0), "A", 96).width_px;
        assert_approx_eq(env_w, 9.0, 1e-6, 1e-6, "CURVEBOOK_FONTS_DIR env override priority");
    } else {
        let exe = std::env::current_exe().expect("current test exe");
        let status = Command::new(exe)
            .env("CURVEBOOK_FONTS_DIR", &dir)
            .env("CB_R10_FONT_ENV_CHILD", "1")
            .args(["--exact", "ttc_collection_and_fonts_dir_manifest_discovery_work_and_override_host"])
            .status()
            .expect("spawn child test");
        assert!(status.success(), "CURVEBOOK_FONTS_DIR child process check failed");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn edge_cases_trait_objects_combining_marks_and_measure_shape_parity() {
    let set = SystemFontSet::from_env_and_system();
    let measurer_ref: &dyn TextMeasurer = &set;
    let measurer_arc: Arc<dyn TextMeasurer> = Arc::new(set.clone());

    let style = UiTextStyle::ui_sans(13.0);

    // Trait objects & concrete measurer agree identically (`measure == shape_line.metrics`).
    for text in ["", "Hello, World!", "IC50 = 12.4 ± 0.5 μM", "Line 1\r\nLine Two\nThree", "e\u{0301}", "ש\u{05BC}לום"] {
        let m1 = measurer_ref.measure(&style, text, 144);
        let s1 = measurer_arc.shape_line(&style, text, 144);
        assert_eq!(m1, s1.metrics, "measure vs shape_line parity for {text:?}");
    }

    // Empty string has 0 width and positive line metrics.
    let empty = measurer_ref.measure(&style, "", 96);
    assert_eq!(empty.width_px, 0.0);
    assert!(empty.ascent_px > 0.0 && empty.descent_px > 0.0 && empty.line_height_px > 0.0);

    // Combining marks keep zero horizontal advance in both Latin and Hebrew.
    let combined = set.shape_line(&style, "e\u{0301}", 96);
    assert_eq!(combined.glyphs.len(), 2);
    assert_eq!(combined.glyphs[1].advance_px, 0.0);

    let he_combined = set.shape_line(&style, "ש\u{05BC}לום", 96);
    assert!(he_combined.glyphs.iter().any(|g| g.ch == '\u{05BC}' && g.advance_px == 0.0));

    // Invalid size_px returns zero metrics cleanly without NaN or panic.
    for bad_size in [0.0_f32, -10.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let bad = UiTextStyle::ui_sans(bad_size);
        let m = measurer_ref.measure(&bad, "test", 96);
        assert_eq!(m, MeasuredTextMetrics::default());
        let s = measurer_ref.shape_line(&bad, "test", 96);
        assert!(s.glyphs.is_empty());
    }

    // dpi == 0 safely falls back to baseline 96 DPI instead of zeroing metrics.
    let d96 = measurer_ref.measure(&style, "Alpha", 96);
    let d0 = measurer_ref.measure(&style, "Alpha", 0);
    assert_eq!(d0, d96);

    // Rich mode handles empty markup tags cleanly without panicking.
    let rich_empty = measurer_ref.shape_line(&style.clone().with_rich(true), "<b></b>", 96);
    assert_eq!(rich_empty.metrics.width_px, 0.0);
    assert!(rich_empty.metrics.line_height_px > 0.0);
}

#[test]
fn weight_slope_and_family_selection_route_to_matching_sfnt_faces() {
    let reg = build_custom_advance_sfnt("TestSans Regular", &[('A', 667)], false);
    let bold = build_custom_advance_sfnt("TestSans Bold", &[('A', 760)], false);
    let italic = build_custom_advance_sfnt("TestSans Italic", &[('A', 680)], false);
    let serif = build_custom_advance_sfnt("TestSerif Regular", &[('A', 710)], false);

    let set = SystemFontSet::from_sfnt_bytes(&[
        ("TestSans-Regular.ttf", &reg),
        ("TestSans-Bold.ttf", &bold),
        ("TestSans-Italic.ttf", &italic),
        ("TestSerif-Regular.ttf", &serif),
    ]);
    assert!(set.has_sfnt_faces());
    assert_eq!(set.face_names().len(), 4);

    let s_reg = UiTextStyle::ui_sans(10.0);
    let s_bold = s_reg.clone().with_weight(SpanWeight::Bold);
    let s_italic = s_reg.clone().with_slope(SpanSlope::Italic);
    let s_serif = UiTextStyle::scientific_serif(10.0);

    assert_approx_eq(set.measure(&s_reg, "A", 96).width_px, 6.67, 1e-6, 1e-6, "regular face width");
    assert_approx_eq(set.measure(&s_bold, "A", 96).width_px, 7.60, 1e-6, 1e-6, "bold face width");
    assert_approx_eq(set.measure(&s_italic, "A", 96).width_px, 6.80, 1e-6, 1e-6, "italic face width");
    assert_approx_eq(set.measure(&s_serif, "A", 96).width_px, 7.10, 1e-6, 1e-6, "serif face width");

    let (face_idx, gid) = set.resolve_face_for_char(&s_bold, 'A').expect("resolved bold face");
    assert_eq!(face_idx, 1);
    assert!(gid > 0);
    assert!(set.parsed_face(face_idx).is_some());
    assert!(set.face_bytes(face_idx).is_some());
}
