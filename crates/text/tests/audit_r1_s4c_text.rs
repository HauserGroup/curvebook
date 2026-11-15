//! Stage 4C audit regression tests for `R1-S09-10` in `lumencraft-text`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use lumencraft_text::TrueTypeFont;
use lumencraft_text::glyph_paths::outline_ttf_glyph;

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(buf: &mut Vec<u8>, v: i16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn make_simple_square_glyph() -> Vec<u8> {
    let mut g = Vec::new();
    push_i16(&mut g, 1); // numberOfContours
    push_i16(&mut g, 100);
    push_i16(&mut g, 100);
    push_i16(&mut g, 500);
    push_i16(&mut g, 500);
    push_u16(&mut g, 3); // endPtsOfContours[0]
    push_u16(&mut g, 0); // instructionLength
    g.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
    for dx in [100_i16, 400, 0, -400] {
        push_i16(&mut g, dx);
    }
    for dy in [100_i16, 0, 400, 0] {
        push_i16(&mut g, dy);
    }
    while !g.len().is_multiple_of(4) {
        g.push(0);
    }
    g
}

fn make_simple_many_point_glyph(n_points: usize) -> Vec<u8> {
    let mut g = Vec::new();
    push_i16(&mut g, 1); // numberOfContours
    push_i16(&mut g, 0);
    push_i16(&mut g, 0);
    push_i16(&mut g, 500);
    push_i16(&mut g, 500);
    push_u16(&mut g, u16::try_from(n_points - 1).expect("n_points fits in u16"));
    push_u16(&mut g, 0); // instructionLength
    // on-curve (0x01) + x_same (0x10) + y_same (0x20) => 0 delta bytes per point.
    g.resize(g.len() + n_points, 0x31);
    while !g.len().is_multiple_of(4) {
        g.push(0);
    }
    g
}

fn make_compound_glyph(components: &[(u16, i16, i16)]) -> Vec<u8> {
    let mut g = Vec::new();
    push_i16(&mut g, -1); // numberOfContours < 0 => compound
    push_i16(&mut g, 0);
    push_i16(&mut g, 0);
    push_i16(&mut g, 500);
    push_i16(&mut g, 500);
    for (idx, &(comp_gid, dx, dy)) in components.iter().enumerate() {
        let mut flags: u16 = 0x0003; // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES
        if idx + 1 < components.len() {
            flags |= 0x0020; // MORE_COMPONENTS
        }
        push_u16(&mut g, flags);
        push_u16(&mut g, comp_gid);
        push_i16(&mut g, dx);
        push_i16(&mut g, dy);
    }
    while !g.len().is_multiple_of(4) {
        g.push(0);
    }
    g
}

/// Build an SFNT font whose `gid 1..=N` map sequentially to `'A', 'B', ...`.
fn build_ttf_from_glyphs(glyph_blobs: &[Vec<u8>]) -> Vec<u8> {
    let num_glyphs = u16::try_from(glyph_blobs.len()).expect("glyph count fits u16");
    assert!((2..=27).contains(&num_glyphs));
    let mapped_count = num_glyphs - 1;
    let end_code = 0x0041_u16 + (mapped_count - 1);
    let id_delta: i16 = 1 - 0x0041_i16;

    // `head` (indexToLocFormat = 1: long 32-bit `loca` offsets)
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5_u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000_u16.to_be_bytes());
    head[50..52].copy_from_slice(&1_i16.to_be_bytes());

    // `hhea`
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    hhea[4..6].copy_from_slice(&800_i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200_i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&num_glyphs.to_be_bytes());

    // `maxp`
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());

    // `hmtx`
    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    for _ in 0..num_glyphs {
        push_u16(&mut hmtx, 600);
        push_i16(&mut hmtx, 0);
    }

    // `cmap` format 4 mapping 'A'..=end_code -> 1..=mapped_count
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
    push_u16(&mut cmap, end_code);
    push_u16(&mut cmap, 0xFFFF);
    push_u16(&mut cmap, 0); // reservedPad
    push_u16(&mut cmap, 0x0041);
    push_u16(&mut cmap, 0xFFFF);
    push_i16(&mut cmap, id_delta);
    push_i16(&mut cmap, 1);
    push_u16(&mut cmap, 0);
    push_u16(&mut cmap, 0);

    // `glyf` + long `loca`
    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity((usize::from(num_glyphs) + 1) * 4);
    for blob in glyph_blobs {
        push_u32(&mut loca, u32::try_from(glyf.len()).expect("glyf offset fits u32"));
        glyf.extend_from_slice(blob);
    }
    push_u32(&mut loca, u32::try_from(glyf.len()).expect("glyf end fits u32"));

    let tables: [([u8; 4], Vec<u8>); 7] =
        [(*b"cmap", cmap), (*b"glyf", glyf), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"loca", loca), (*b"maxp", maxp)];
    let num_tables = u16::try_from(tables.len()).expect("table count");
    let dir_size = 12 + tables.len() * 16;
    let mut font = Vec::new();
    push_u32(&mut font, 0x0001_0000);
    push_u16(&mut font, num_tables);
    push_u16(&mut font, 64);
    push_u16(&mut font, 2);
    push_u16(&mut font, 48);

    let mut cur_offset = dir_size;
    for (tag, data) in &tables {
        font.extend_from_slice(tag);
        push_u32(&mut font, 0);
        push_u32(&mut font, u32::try_from(cur_offset).expect("table offset"));
        push_u32(&mut font, u32::try_from(data.len()).expect("table len"));
        cur_offset += (data.len() + 3) & !3;
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
fn r1_s09_10_exponential_64_pow_8_compound_tree_stops_within_budget() {
    // Acyclic 8-level 64-way fanout tree:
    // gid 1 ('A') = simple square
    // gid 2 ('B') = 64 x gid 1 (valid 1-level 64-component glyph: 65 components, 256 points)
    // gid 3 ('C') = 64 x gid 2 (4161 components -> exceeds total component budget)
    // ...
    // gid 9 ('I') = 64 x gid 8 (64^7 leaf instances)
    let mut glyphs = Vec::with_capacity(10);
    glyphs.push(Vec::new());
    glyphs.push(make_simple_square_glyph());
    for parent_gid in 1_u16..=8 {
        let comps = [(parent_gid, 10, 0); 64];
        glyphs.push(make_compound_glyph(&comps));
    }

    let sfnt = build_ttf_from_glyphs(&glyphs);
    let ttf = TrueTypeFont::parse(&sfnt).expect("parse synthetic exponential TTF");

    let simple_cmds = outline_ttf_glyph(&ttf, 'A', 0.0, 20.0, 12.0, false).expect("simple glyph 'A' must decode");
    assert_eq!(simple_cmds.len(), 6);

    let one_level_cmds = outline_ttf_glyph(&ttf, 'B', 0.0, 20.0, 12.0, false).expect("1-level 64-component glyph 'B' must remain valid");
    assert_eq!(one_level_cmds.len(), 64 * 6);

    let t0 = Instant::now();
    assert!(outline_ttf_glyph(&ttf, 'C', 0.0, 20.0, 12.0, false).is_none(), "2-level 64x64 compound glyph must abort cleanly");
    assert!(outline_ttf_glyph(&ttf, 'I', 0.0, 20.0, 12.0, false).is_none(), "64^7 exponential compound tree must abort cleanly");
    assert!(t0.elapsed().as_secs_f64() < 0.1, "budget rejection took too long: {:?}", t0.elapsed());
}

#[test]
fn r1_s09_10_self_referential_and_cyclic_compound_glyphs_return_none() {
    // gid 1 ('A') -> self-loop (1 component)
    // gid 2 ('B') -> self-loop (64 components)
    // gid 3 ('C') <-> gid 4 ('D') mutual cycle
    let glyphs = vec![
        Vec::new(),
        make_compound_glyph(&[(1, 0, 0)]),
        make_compound_glyph(&[(2, 0, 0); 64]),
        make_compound_glyph(&[(4, 10, 0)]),
        make_compound_glyph(&[(3, 0, 10)]),
    ];
    let sfnt = build_ttf_from_glyphs(&glyphs);
    let ttf = TrueTypeFont::parse(&sfnt).expect("parse cyclic TTF");

    let t0 = Instant::now();
    for ch in ['A', 'B', 'C', 'D'] {
        assert!(outline_ttf_glyph(&ttf, ch, 0.0, 20.0, 12.0, false).is_none(), "cyclic glyph {ch:?} must return None");
    }
    assert!(t0.elapsed().as_secs_f64() < 0.1, "cycle rejection took too long: {:?}", t0.elapsed());
}

#[test]
fn r1_s09_10_compound_point_budget_caps_total_points_across_components() {
    // gid 1 ('A') = 1000-point simple glyph
    // gid 2 ('B') = 4 x gid 1 (4000 points total, 5 components -> within total budget)
    // gid 3 ('C') = 20 x gid 1 (20_000 points total, 21 components -> exceeds total point budget)
    let glyphs = vec![Vec::new(), make_simple_many_point_glyph(1000), make_compound_glyph(&[(1, 0, 0); 4]), make_compound_glyph(&[(1, 0, 0); 20])];
    let sfnt = build_ttf_from_glyphs(&glyphs);
    let ttf = TrueTypeFont::parse(&sfnt).expect("parse point-budget TTF");

    let ok_cmds = outline_ttf_glyph(&ttf, 'B', 0.0, 20.0, 12.0, false).expect("4x1000-point compound glyph must succeed");
    assert_eq!(ok_cmds.len(), 4 * 1002);

    assert!(outline_ttf_glyph(&ttf, 'C', 0.0, 20.0, 12.0, false).is_none(), "20x1000-point compound glyph must exceed total point budget");
}

#[test]
fn r1_s09_10_valid_diamond_dag_and_depth_limit_behave_cleanly() {
    // gid 1 ('A') = simple square
    // gid 2 ('B') = 2 x gid 1 (sibling reuse / diamond DAG)
    // gid 3 ('C') = 2 x gid 2 (2-level diamond DAG -> 4 leaf squares)
    // gid 4..=10 ('D'..='J') = linear chain up to depth 7 (valid)
    // gid 11 ('K') = depth 8 chain (exceeds MAX_TTF_COMPONENT_DEPTH)
    let mut glyphs =
        vec![Vec::new(), make_simple_square_glyph(), make_compound_glyph(&[(1, 0, 0), (1, 100, 0)]), make_compound_glyph(&[(2, 0, 0), (2, 0, 100)])];
    glyphs.push(make_compound_glyph(&[(1, 10, 0)])); // gid 4 ('D'): depth 1
    for prev_gid in 4_u16..=10 {
        glyphs.push(make_compound_glyph(&[(prev_gid, 10, 0)]));
    }
    // gid 10 ('J') has depth 7; gid 11 ('K') has depth 8.

    let sfnt = build_ttf_from_glyphs(&glyphs);
    let ttf = TrueTypeFont::parse(&sfnt).expect("parse DAG & depth TTF");

    let dag_cmds = outline_ttf_glyph(&ttf, 'C', 0.0, 20.0, 12.0, false).expect("valid diamond DAG must decode");
    assert_eq!(dag_cmds.len(), 4 * 6);

    let depth7_cmds = outline_ttf_glyph(&ttf, 'J', 0.0, 20.0, 12.0, false).expect("depth-7 chain must decode");
    assert_eq!(depth7_cmds.len(), 6);

    assert!(outline_ttf_glyph(&ttf, 'K', 0.0, 20.0, 12.0, false).is_none(), "depth-8 chain must abort cleanly");
}
