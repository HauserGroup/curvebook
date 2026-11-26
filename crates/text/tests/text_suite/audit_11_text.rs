//! Regression tests for AUD-11 findings in `crates/text`:
//! `AUD-11-02`, `AUD-11-03`, `AUD-11-04`, `AUD-11-05`, `AUD-11-13`, `AUD-11-14`, `AUD-11-18`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_testkit::TempDirGuard;
use curvebook_text::glyph_paths::{outline_ttf_glyph, outline_visual_rich_text_rotated_with_fonts_and_ttf};
use curvebook_text::{RichText, SystemFontSet, TextMeasurer, TrueTypeFont, UiTextStyle, expand_template_hooks, parse_markup};

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn assemble_sfnt(sfnt_version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|(tag, _)| *tag);
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, sfnt_version);
    push_u16(&mut font, u16::try_from(tables.len()).expect("table count"));
    push_u16(&mut font, 128);
    push_u16(&mut font, 3);
    push_u16(&mut font, 0);

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

fn build_name_table(family: &str, postscript: &str) -> Vec<u8> {
    let mut str_buf = Vec::new();
    let fam_off = 0u16;
    for u in family.encode_utf16() {
        push_u16(&mut str_buf, u);
    }
    let fam_len = u16::try_from(str_buf.len()).expect("family len");
    let ps_off = fam_len;
    for u in postscript.encode_utf16() {
        push_u16(&mut str_buf, u);
    }
    let ps_len = u16::try_from(str_buf.len()).expect("ps len") - ps_off;

    let mut name = Vec::new();
    push_u16(&mut name, 0);
    push_u16(&mut name, 2);
    push_u16(&mut name, 6 + 2 * 12);
    for (name_id, len, off) in [(1u16, fam_len, fam_off), (6u16, ps_len, ps_off)] {
        push_u16(&mut name, 3);
        push_u16(&mut name, 1);
        push_u16(&mut name, 0x0409);
        push_u16(&mut name, name_id);
        push_u16(&mut name, len);
        push_u16(&mut name, off);
    }
    name.extend_from_slice(&str_buf);
    name
}

fn build_single_bmp_cmap4(codepoint: u16, gid: u16) -> Vec<u8> {
    let mut sub = Vec::new();
    push_u16(&mut sub, 4);
    push_u16(&mut sub, 32);
    push_u16(&mut sub, 0);
    push_u16(&mut sub, 4); // segCountX2 = 4 (2 segments)
    push_u16(&mut sub, 4);
    push_u16(&mut sub, 1);
    push_u16(&mut sub, 0);
    push_u16(&mut sub, codepoint);
    push_u16(&mut sub, 0xFFFF);
    push_u16(&mut sub, 0);
    push_u16(&mut sub, codepoint);
    push_u16(&mut sub, 0xFFFF);
    let delta = i32::from(gid) - i32::from(codepoint);
    push_u16(&mut sub, u16::try_from(delta.rem_euclid(65_536)).expect("idDelta"));
    push_i16(&mut sub, 1);
    push_u16(&mut sub, 0);
    push_u16(&mut sub, 0);
    sub
}

fn wrap_cmap_subtables(entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut cmap = Vec::new();
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, u16::try_from(entries.len()).expect("n subtables"));
    let mut off = 4 + entries.len() * 8;
    for (plat, enc, data) in entries {
        push_u16(&mut cmap, *plat);
        push_u16(&mut cmap, *enc);
        push_u32(&mut cmap, u32::try_from(off).expect("cmap sub offset"));
        off += data.len();
    }
    for (_, _, data) in entries {
        cmap.extend_from_slice(data);
    }
    cmap
}

fn build_glyf_font_custom(family: &str, postscript: &str, descender: i16, codepoint: u16, empty_glyph1: bool) -> Vec<u8> {
    let num_glyphs: u16 = 2;

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&0i16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&descender.to_be_bytes());
    hhea[8..10].copy_from_slice(&90i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    maxp[6..8].copy_from_slice(&3u16.to_be_bytes());
    maxp[8..10].copy_from_slice(&1u16.to_be_bytes());

    let mut hmtx = Vec::new();
    push_u16(&mut hmtx, 500);
    push_i16(&mut hmtx, 0);
    push_u16(&mut hmtx, 600);
    push_i16(&mut hmtx, 20);

    let (glyf, loca) = if empty_glyph1 {
        (vec![0u8; 4], vec![0u8; 6])
    } else {
        let mut g = Vec::new();
        push_i16(&mut g, 1);
        push_i16(&mut g, 10);
        push_i16(&mut g, 0);
        push_i16(&mut g, 100);
        push_i16(&mut g, 100);
        push_u16(&mut g, 2);
        push_u16(&mut g, 0);
        g.extend_from_slice(&[0x37, 0x37, 0x37, 10, 40, 10, 10, 0, 50]);
        while !g.len().is_multiple_of(4) {
            g.push(0);
        }
        let mut l = Vec::new();
        push_u16(&mut l, 0);
        push_u16(&mut l, 0);
        push_u16(&mut l, u16::try_from(g.len() / 2).expect("loca"));
        (g, l)
    };

    let cmap = wrap_cmap_subtables(&[(3, 1, build_single_bmp_cmap4(codepoint, 1))]);
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    assemble_sfnt(
        0x0001_0000,
        vec![
            (*b"cmap", cmap),
            (*b"glyf", glyf),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"loca", loca),
            (*b"maxp", maxp),
            (*b"name", build_name_table(family, postscript)),
            (*b"post", post),
        ],
    )
}

fn cff_encode_index(items: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    push_u16(&mut out, u16::try_from(items.len()).expect("cff index count"));
    if items.is_empty() {
        return out;
    }
    let total_data: usize = items.iter().map(|s| s.len()).sum();
    let off_size: u8 = if total_data < 255 {
        1
    } else if total_data < 65_535 {
        2
    } else {
        4
    };
    out.push(off_size);
    let mut offset: u32 = 1;
    let push_off = |buf: &mut Vec<u8>, v: u32, sz: u8| match sz {
        1 => buf.push(u8::try_from(v).expect("u8 off")),
        2 => buf.extend_from_slice(&u16::try_from(v).expect("u16 off").to_be_bytes()),
        _ => buf.extend_from_slice(&v.to_be_bytes()),
    };
    push_off(&mut out, offset, off_size);
    for item in items {
        offset += u32::try_from(item.len()).expect("item len");
        push_off(&mut out, offset, off_size);
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn cff_push_int(buf: &mut Vec<u8>, v: i32) {
    if (-107..=107).contains(&v) {
        buf.push(u8::try_from(v + 139).expect("cff int1"));
    } else if (108..=1131).contains(&v) {
        let d = v - 108;
        buf.push(u8::try_from((d / 256) + 247).expect("cff int2+"));
        buf.push(u8::try_from(d % 256).expect("cff int2+ lo"));
    } else if (-1131..=-108).contains(&v) {
        let d = -v - 108;
        buf.push(u8::try_from((d / 256) + 251).expect("cff int2-"));
        buf.push(u8::try_from(d % 256).expect("cff int2- lo"));
    } else {
        buf.push(29);
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

/// Build a minimal valid OpenType CFF (`OTTO`) font mapping `'A'` (`0x0041`) to `gid 1`.
fn build_synthetic_cff_otf(charstring_a: &[u8], global_subrs: &[&[u8]]) -> Vec<u8> {
    let num_glyphs: u16 = 2;
    let name_index = cff_encode_index(&[b"SynthCFF"]);
    let string_index = cff_encode_index(&[]);
    let gsubr_index = cff_encode_index(global_subrs);
    // Charset format 0 with 1 glyph (gid 1 -> SID 34 'A').
    let charset = [0u8, 0, 34];
    let notdef_cs = [14u8]; // endchar
    let charstrings_index = cff_encode_index(&[&notdef_cs, charstring_a]);
    let private_dict = [139u8, 20]; // defaultWidthX = 0 (arbitrary valid private dict)

    // Compute offsets iteratively using 5-byte integer encodings (tag 29 + i32) so Top DICT size is fixed.
    let encode_top_dict = |charset_off: i32, cs_off: i32, priv_len: i32, priv_off: i32| -> Vec<u8> {
        let mut d = Vec::new();
        d.push(29);
        d.extend_from_slice(&charset_off.to_be_bytes());
        d.push(15); // charset
        d.push(29);
        d.extend_from_slice(&cs_off.to_be_bytes());
        d.push(17); // CharStrings
        d.push(29);
        d.extend_from_slice(&priv_len.to_be_bytes());
        d.push(29);
        d.extend_from_slice(&priv_off.to_be_bytes());
        d.push(18); // Private
        d
    };
    let dummy_top = encode_top_dict(0, 0, 0, 0);
    let top_index_len = cff_encode_index(&[&dummy_top]).len();
    let header_len = 4usize;
    let charset_off = header_len + name_index.len() + top_index_len + string_index.len() + gsubr_index.len();
    let cs_off = charset_off + charset.len();
    let priv_off = cs_off + charstrings_index.len();

    let real_top = encode_top_dict(
        i32::try_from(charset_off).expect("charset off"),
        i32::try_from(cs_off).expect("cs off"),
        i32::try_from(private_dict.len()).expect("priv len"),
        i32::try_from(priv_off).expect("priv off"),
    );
    let top_index = cff_encode_index(&[&real_top]);
    assert_eq!(top_index.len(), top_index_len);

    let mut cff = Vec::new();
    cff.extend_from_slice(&[1, 0, 4, 4]); // CFF v1.0 header
    cff.extend_from_slice(&name_index);
    cff.extend_from_slice(&top_index);
    cff.extend_from_slice(&string_index);
    cff.extend_from_slice(&gsubr_index);
    cff.extend_from_slice(&charset);
    cff.extend_from_slice(&charstrings_index);
    cff.extend_from_slice(&private_dict);

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());

    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());

    let mut hmtx = Vec::new();
    push_u16(&mut hmtx, 500);
    push_i16(&mut hmtx, 0);
    push_u16(&mut hmtx, 600);
    push_i16(&mut hmtx, 0);

    let cmap = wrap_cmap_subtables(&[(3, 1, build_single_bmp_cmap4(0x0041, 1))]);
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    assemble_sfnt(
        u32::from_be_bytes(*b"OTTO"),
        vec![
            (*b"CFF ", cff),
            (*b"cmap", cmap),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"maxp", maxp),
            (*b"name", build_name_table("SynthCFF", "SynthCFF")),
            (*b"post", post),
        ],
    )
}

#[test]
fn aud11_02_i16_min_descender_does_not_panic_or_wrap() {
    let font_bytes = build_glyf_font_custom("MinDesc", "MinDesc", i16::MIN, 0x0041, false);
    let ttf = TrueTypeFont::parse_checked(&font_bytes).expect("parse i16::MIN descender font");
    assert_eq!(ttf.descent(), i16::MIN);

    // `shaping::build_loaded_face` must clamp to 0.65 (not wrap negative to 0.1 or panic).
    let set = SystemFontSet::from_sfnt_bytes(&[("MinDesc", &font_bytes)]);
    assert_eq!(set.face_count(), 1);
    let ui_metrics = set.measure(&UiTextStyle::ui_sans(10.0), "A", 96);
    assert!((ui_metrics.descent_px - 6.5).abs() < 1e-6, "expected clamped 0.65*10 = 6.5px descent, got {}", ui_metrics.descent_px);
}

#[test]
fn aud11_03_unmatched_markup_and_template_delimiters_stay_linear_time() {
    let lt_input = "<".repeat(100_000);
    let amp_input = "&".repeat(100_000);
    let hook_input = "{{".repeat(50_000);

    let start = Instant::now();
    let parsed_lt = parse_markup(&lt_input);
    let parsed_amp = parse_markup(&amp_input);
    let expanded = expand_template_hooks(&hook_input, |_| Some(String::from("X")));
    let elapsed = start.elapsed();

    assert_eq!(parsed_lt.plain_text(), lt_input);
    assert_eq!(parsed_amp.plain_text(), amp_input);
    assert_eq!(expanded, hook_input);
    assert!(elapsed.as_secs_f64() < 2.0, "expected linear-time parse/expand, took {elapsed:?}");

    // Valid tags, boundary-length entities, and valid template hooks continue to work.
    assert_eq!(parse_markup("<b>x</b> &alpha; &#x00000000000041;").plain_text(), "x \u{03B1} A");
    assert_eq!(expand_template_hooks("a={{k}} tail {{unclosed", |k| (k == "k").then(|| String::from("1"))), "a=1 tail {{unclosed");
}

#[test]
fn aud11_04_manifest_dedup_count_cap_and_symlink_containment() {
    let root_guard = TempDirGuard::new("aud11-04-font-dir").expect("temp dir");
    let root = root_guard.path();
    let bundle = root.join("bundle");
    let fonts_sub = bundle.join("fonts");
    std::fs::create_dir_all(&fonts_sub).expect("mkdir fonts");

    let valid_a = build_glyf_font_custom("RepeatFont", "RepeatFont", -200, 0x0041, false);
    std::fs::write(fonts_sub.join("a.ttf"), &valid_a).expect("write a.ttf");

    // 1. 1,000 repeated lines in manifest.txt must deduplicate to 1 loaded face (no OOM amplification).
    let repeated_manifest = "a.ttf\n".repeat(1_000);
    std::fs::write(fonts_sub.join("manifest.txt"), &repeated_manifest).expect("write repeated manifest");
    let dedup_set = SystemFontSet::from_fonts_dir(&bundle);
    assert_eq!(dedup_set.face_count(), 1, "repeated manifest entries must deduplicate to 1 face");

    // 2. More than MAX_DIR_FONTS (64) distinct files in manifest.txt must be capped at 64.
    let cap_dir = root.join("cap_bundle");
    let cap_fonts = cap_dir.join("fonts");
    std::fs::create_dir_all(&cap_fonts).expect("mkdir cap_fonts");
    let mut cap_manifest = String::new();
    for idx in 0..72u16 {
        let name = format!("f{idx:03}.ttf");
        let bytes = build_glyf_font_custom(&format!("CapFont{idx}"), &format!("CapFont{idx}"), -200, 0x0041, false);
        std::fs::write(cap_fonts.join(&name), &bytes).expect("write cap font");
        cap_manifest.push_str(&name);
        cap_manifest.push('\n');
    }
    std::fs::write(cap_fonts.join("manifest.txt"), &cap_manifest).expect("write cap manifest");
    let capped_set = SystemFontSet::from_fonts_dir(&cap_dir);
    assert_eq!(capped_set.face_count(), 64, "manifest font loading must respect MAX_DIR_FONTS=64");

    // 3. Symlinked font file, symlinked subdirectory, and symlinked manifest must all be rejected.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let outside_dir = root.join("outside_dir");
        std::fs::create_dir_all(&outside_dir).expect("mkdir outside_dir");
        let evil_font = build_glyf_font_custom("EvilSymlink", "EvilSymlink", -200, 0x4E2D, false);
        let outside_font = outside_dir.join("secret.ttf");
        std::fs::write(&outside_font, &evil_font).expect("write outside font");

        let sym_bundle = root.join("sym_bundle");
        let sym_fonts = sym_bundle.join("fonts");
        std::fs::create_dir_all(&sym_fonts).expect("mkdir sym_fonts");
        symlink(&outside_font, sym_fonts.join("escape.ttf")).expect("symlink escape.ttf");
        symlink(&outside_dir, sym_fonts.join("sub")).expect("symlink sub dir");
        std::fs::write(sym_fonts.join("manifest.txt"), "escape.ttf\nsub/secret.ttf\n").expect("write sym manifest");

        let sym_set = SystemFontSet::from_fonts_dir(&sym_bundle);
        assert_eq!(sym_set.face_count(), 0, "symlinked files and directories must never load outside CURVEBOOK_FONTS_DIR");

        // Symlinked manifest.txt pointing outside dir must also be ignored.
        let outside_manifest = root.join("outside_manifest.txt");
        std::fs::write(&outside_manifest, "a.ttf\n").expect("write outside manifest");
        let sym_manifest_bundle = root.join("sym_manifest_bundle");
        std::fs::create_dir_all(sym_manifest_bundle.join("fonts")).expect("mkdir");
        symlink(&outside_manifest, sym_manifest_bundle.join("fonts").join("manifest.txt")).expect("symlink manifest");
        // No font files in sym_manifest_bundle; verify symlinked manifest is rejected cleanly.
        assert_eq!(SystemFontSet::from_fonts_dir(&sym_manifest_bundle).face_count(), 0);
    }
}

#[test]
fn aud11_05_cff_outline_enforces_contour_subr_and_finiteness_budgets() {
    // 1. Valid 1-contour CFF triangle outlines cleanly.
    let mut tri_cs = Vec::new();
    cff_push_int(&mut tri_cs, 100);
    cff_push_int(&mut tri_cs, 100);
    tri_cs.push(21); // rmoveto
    cff_push_int(&mut tri_cs, 200);
    cff_push_int(&mut tri_cs, 0);
    tri_cs.push(5); // rlineto
    cff_push_int(&mut tri_cs, -100);
    cff_push_int(&mut tri_cs, 200);
    tri_cs.push(5); // rlineto
    tri_cs.push(14); // endchar
    let ok_otf = build_synthetic_cff_otf(&tri_cs, &[]);
    let ok_ttf = TrueTypeFont::parse_checked(&ok_otf).expect("parse valid CFF OTF");
    let ok_cmds = outline_ttf_glyph(&ok_ttf, 'A', 0.0, 0.0, 12.0, false).expect("valid CFF triangle must outline");
    assert!(!ok_cmds.is_empty());
    // Non-finite origin or extreme size overflowing coordinates must return None.
    assert!(outline_ttf_glyph(&ok_ttf, 'A', f64::NAN, 0.0, 12.0, false).is_none());
    // 1.5e308 + 300 * (1e308 / 1000) = 1.8e308 > f64::MAX -> map_pt overflows to +inf -> None.
    assert!(outline_ttf_glyph(&ok_ttf, 'A', 1.5e308, 0.0, 1e308, false).is_none());

    // 2. CFF glyph with 257 contours (> MAX_TTF_CONTOURS=256) must return None.
    let mut many_contours = Vec::new();
    for _ in 0..257 {
        cff_push_int(&mut many_contours, 1);
        cff_push_int(&mut many_contours, 1);
        many_contours.push(21); // rmoveto
        cff_push_int(&mut many_contours, 2);
        cff_push_int(&mut many_contours, 0);
        many_contours.push(5); // rlineto
        cff_push_int(&mut many_contours, 0);
        cff_push_int(&mut many_contours, 2);
        many_contours.push(5); // rlineto
    }
    many_contours.push(14);
    let over_otf = build_synthetic_cff_otf(&many_contours, &[]);
    let over_ttf = TrueTypeFont::parse_checked(&over_otf).expect("parse over-contour CFF OTF");
    assert!(outline_ttf_glyph(&over_ttf, 'A', 0.0, 0.0, 12.0, false).is_none(), "CFF glyph with >256 contours must be rejected");

    // 3. CFF Type 2 subroutine amplification bomb (8 nested levels, 4x fanout = 4^8 = 65,536 calls) must abort rapidly.
    // Global subr bias for < 1240 entries is 107, so encoded arg `idx - 107` calls gsubr `idx`.
    let mut subrs: Vec<Vec<u8>> = Vec::new();
    // Leaf gsubr 0: emit one tiny move+line and return (11).
    let mut leaf = Vec::new();
    cff_push_int(&mut leaf, 1);
    cff_push_int(&mut leaf, 1);
    leaf.push(5); // rlineto
    leaf.push(11); // return
    subrs.push(leaf);
    for level in 1..=8i32 {
        let mut s = Vec::new();
        for _ in 0..4 {
            cff_push_int(&mut s, (level - 1) - 107);
            s.push(29); // callgsubr
        }
        s.push(11); // return
        subrs.push(s);
    }
    let mut root_cs = Vec::new();
    cff_push_int(&mut root_cs, 10);
    cff_push_int(&mut root_cs, 10);
    root_cs.push(21); // rmoveto
    cff_push_int(&mut root_cs, 8 - 107);
    root_cs.push(29); // callgsubr level 8
    root_cs.push(14); // endchar

    let subr_refs: Vec<&[u8]> = subrs.iter().map(Vec::as_slice).collect();
    let bomb_otf = build_synthetic_cff_otf(&root_cs, &subr_refs);
    let bomb_ttf = TrueTypeFont::parse_checked(&bomb_otf).expect("parse CFF subr bomb");
    let t0 = Instant::now();
    assert!(outline_ttf_glyph(&bomb_ttf, 'A', 0.0, 0.0, 12.0, false).is_none(), "CFF subr amplification bomb must be rejected");
    assert!(t0.elapsed().as_secs_f64() < 1.0, "CFF subr bomb must reject rapidly, took {:?}", t0.elapsed());
}

#[test]
fn aud11_13_cmap_collection_prioritizes_format12_and_avoids_truncation_or_drift() {
    let num_glyphs: u16 = 65_200;

    // Format 4 mapping BMP 0x0041..=0xFE40 (65,024 entries) where 'A' (0x0041) -> gid 1.
    let mut sub4 = Vec::new();
    push_u16(&mut sub4, 4);
    push_u16(&mut sub4, 32);
    push_u16(&mut sub4, 0);
    push_u16(&mut sub4, 4);
    push_u16(&mut sub4, 4);
    push_u16(&mut sub4, 1);
    push_u16(&mut sub4, 0);
    push_u16(&mut sub4, 0xFE40);
    push_u16(&mut sub4, 0xFFFF);
    push_u16(&mut sub4, 0);
    push_u16(&mut sub4, 0x0041);
    push_u16(&mut sub4, 0xFFFF);
    let delta4 = (1i32 - 0x0041i32).rem_euclid(65_536);
    push_u16(&mut sub4, u16::try_from(delta4).expect("delta4"));
    push_i16(&mut sub4, 1);
    push_u16(&mut sub4, 0);
    push_u16(&mut sub4, 0);

    // Format 12 (Windows 3,10 — higher rank than Format 4) mapping:
    //   - 0x0041 ('A') -> gid 2 (deliberately different from Format 4's gid 1!)
    //   - 0x10400..=0x10410 -> gid 500..=516 (supplementary plane)
    let mut sub12 = Vec::new();
    push_u16(&mut sub12, 12);
    push_u16(&mut sub12, 0);
    push_u32(&mut sub12, 16 + 2 * 12);
    push_u32(&mut sub12, 0);
    push_u32(&mut sub12, 2); // nGroups
    // Group 0: 'A' -> gid 2
    push_u32(&mut sub12, 0x0041);
    push_u32(&mut sub12, 0x0041);
    push_u32(&mut sub12, 2);
    // Group 1: U+10400..=U+10410 -> gid 500..=516
    push_u32(&mut sub12, 0x10400);
    push_u32(&mut sub12, 0x10410);
    push_u32(&mut sub12, 500);

    let cmap = wrap_cmap_subtables(&[(3, 1, sub4), (3, 10, sub12)]);
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes()); // numOfHMetrics = 1
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    let mut hmtx = vec![0u8; 4 + usize::from(num_glyphs - 1) * 2];
    hmtx[0..2].copy_from_slice(&600u16.to_be_bytes());
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    let bytes = assemble_sfnt(
        0x0001_0000,
        vec![
            (*b"cmap", cmap),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"maxp", maxp),
            (*b"name", build_name_table("DualCmap", "DualCmap")),
            (*b"post", post),
        ],
    );
    let ttf = TrueTypeFont::parse_checked(&bytes).expect("parse dual-cmap font");
    let info = ttf.inspect_info().expect("inspect dual-cmap font");

    // Both `TrueTypeFont::glyph_id` and `TrueTypeFontInfo::glyph_id` must agree on Format 12 for 'A' and U+10400.
    assert_eq!(ttf.glyph_id('A'), Some(2));
    assert_eq!(info.glyph_id('A'), 2, "Format 12 primary subtable must deterministically win over Format 4");
    assert_eq!(ttf.glyph_id('\u{10400}'), Some(500));
    assert_eq!(info.glyph_id('\u{10400}'), 500, "supplementary Format 12 mappings must not be truncated behind Format 4");
}

#[test]
fn aud11_14_combining_mark_falls_back_when_preferred_ttf_glyph_is_empty() {
    // Build a preferred TTF that maps U+0304 (combining macron) to an empty zero-contour glyph.
    let empty_macron_bytes = build_glyf_font_custom("EmptyMacron", "EmptyMacron", -200, 0x0304, true);
    let empty_ttf = TrueTypeFont::parse_checked(&empty_macron_bytes).expect("parse empty macron font");
    let raw_cmds = outline_ttf_glyph(&empty_ttf, '\u{0304}', 0.0, 0.0, 12.0, false).expect("mapped glyph returns Some(empty)");
    assert!(raw_cmds.is_empty(), "precondition: preferred_ttf maps U+0304 to zero contours");

    let cleanroom = SystemFontSet::cleanroom_only();
    // 1. Combining mark attached to base 'x' (`x̄`).
    let attached =
        outline_visual_rich_text_rotated_with_fonts_and_ttf(&RichText::plain("x\u{0304}"), 0.0, 0.0, 12.0, 0.0, &cleanroom, Some(&empty_ttf));
    assert_eq!(attached.len(), 2);
    assert_eq!(attached[1].codepoint, '\u{0304}');
    assert!(!attached[1].commands.is_empty(), "combining macron must fall back to cleanroom vector outline instead of empty preferred_ttf glyph");

    // 2. Standalone combining mark (no preceding base).
    let standalone =
        outline_visual_rich_text_rotated_with_fonts_and_ttf(&RichText::plain("\u{0304}"), 0.0, 0.0, 12.0, 0.0, &cleanroom, Some(&empty_ttf));
    assert_eq!(standalone.len(), 1);
    assert!(!standalone[0].commands.is_empty(), "standalone combining macron must also fall back when preferred_ttf glyph is empty");
}

#[test]
fn aud11_18_sanitize_postscript_name_strips_hash_delimiter() {
    let bytes = build_glyf_font_custom("HashFont", "MyFont#20Name/(Bad)%[]", -200, 0x0041, false);
    let ttf = TrueTypeFont::parse_checked(&bytes).expect("parse hash ps name font");
    let info = ttf.inspect_info().expect("inspect hash ps name font");
    assert!(!info.postscript_name.contains('#'), "postscript_name must not contain PDF Name escape '#': {:?}", info.postscript_name);
    assert_eq!(info.postscript_name, "MyFont20NameBad");
}
