//! Stage 4C audit regression tests for `R1-S09-10` in `curvebook-text`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Instant;

use curvebook_text::TrueTypeFont;
use curvebook_text::glyph_paths::outline_ttf_glyph;

use super::sfnt_fixture::{
    assemble_sfnt, build_cmap4_subtable, build_head, build_hhea, build_synthetic_cff_otf, cff_push_int, push_i16, push_u16, push_u32,
    wrap_cmap_subtables,
};

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
    let end_code = 0x0041_u16 + (num_glyphs - 2);
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&num_glyphs.to_be_bytes());
    let mut hmtx = Vec::with_capacity(usize::from(num_glyphs) * 4);
    for _ in 0..num_glyphs {
        push_u16(&mut hmtx, 600);
        push_i16(&mut hmtx, 0);
    }
    let cmap = wrap_cmap_subtables(&[(3, 1, build_cmap4_subtable(&[(0x0041, end_code, 1 - 0x0041_i16, 0), (0xFFFF, 0xFFFF, 1, 0)], &[]))]);
    let mut glyf = Vec::new();
    let mut loca = Vec::with_capacity((usize::from(num_glyphs) + 1) * 4);
    for blob in glyph_blobs {
        push_u32(&mut loca, u32::try_from(glyf.len()).expect("glyf offset fits u32"));
        glyf.extend_from_slice(blob);
    }
    push_u32(&mut loca, u32::try_from(glyf.len()).expect("glyf end fits u32"));
    assemble_sfnt(
        0x0001_0000,
        vec![
            (*b"cmap", cmap),
            (*b"glyf", glyf),
            (*b"head", build_head(1)),
            (*b"hhea", build_hhea(800, -200, 0, num_glyphs)),
            (*b"hmtx", hmtx),
            (*b"loca", loca),
            (*b"maxp", maxp),
        ],
    )
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

#[test]
fn cff_outline_enforces_contour_subr_and_finiteness_budgets() {
    let push_op = |buf: &mut Vec<u8>, dx: i32, dy: i32, op: u8| {
        cff_push_int(buf, dx);
        cff_push_int(buf, dy);
        buf.push(op);
    };

    // 1. Valid 1-contour CFF triangle outlines cleanly; NaN origin & overflowing coords return None.
    let mut tri_cs = Vec::new();
    push_op(&mut tri_cs, 100, 100, 21); // rmoveto
    push_op(&mut tri_cs, 200, 0, 5); // rlineto
    push_op(&mut tri_cs, -100, 200, 5);
    tri_cs.push(14); // endchar
    let ok_otf = build_synthetic_cff_otf(&tri_cs, &[]);
    let ok_ttf = TrueTypeFont::parse_checked(&ok_otf).expect("parse valid CFF OTF");
    assert!(!outline_ttf_glyph(&ok_ttf, 'A', 0.0, 0.0, 12.0, false).expect("valid CFF triangle").is_empty());
    assert!(outline_ttf_glyph(&ok_ttf, 'A', f64::NAN, 0.0, 12.0, false).is_none());
    assert!(outline_ttf_glyph(&ok_ttf, 'A', 1.5e308, 0.0, 1e308, false).is_none());

    // 2. CFF glyph with 257 contours (> MAX_TTF_CONTOURS=256) returns None.
    let mut many_contours = Vec::new();
    for _ in 0..257 {
        push_op(&mut many_contours, 1, 1, 21);
        push_op(&mut many_contours, 2, 0, 5);
        push_op(&mut many_contours, 0, 2, 5);
    }
    many_contours.push(14);
    let over_otf = build_synthetic_cff_otf(&many_contours, &[]);
    let over_ttf = TrueTypeFont::parse_checked(&over_otf).expect("parse over-contour CFF OTF");
    assert!(outline_ttf_glyph(&over_ttf, 'A', 0.0, 0.0, 12.0, false).is_none(), "CFF glyph with >256 contours must be rejected");

    // 3. CFF Type 2 subroutine amplification bomb (8 nested levels, 4x fanout = 4^8 calls) aborts rapidly.
    let mut leaf = Vec::new();
    push_op(&mut leaf, 1, 1, 5);
    leaf.push(11); // return
    let mut subrs = vec![leaf];
    for level in 1..=8i32 {
        let mut s = Vec::new();
        for _ in 0..4 {
            cff_push_int(&mut s, (level - 1) - 107);
            s.push(29); // callgsubr
        }
        s.push(11);
        subrs.push(s);
    }
    let mut root_cs = Vec::new();
    push_op(&mut root_cs, 10, 10, 21);
    cff_push_int(&mut root_cs, 8 - 107);
    root_cs.push(29);
    root_cs.push(14);
    let subr_refs: Vec<&[u8]> = subrs.iter().map(Vec::as_slice).collect();
    let bomb_otf = build_synthetic_cff_otf(&root_cs, &subr_refs);
    let bomb_ttf = TrueTypeFont::parse_checked(&bomb_otf).expect("parse CFF subr bomb");
    let t0 = Instant::now();
    assert!(outline_ttf_glyph(&bomb_ttf, 'A', 0.0, 0.0, 12.0, false).is_none(), "CFF subr bomb must be rejected");
    assert!(t0.elapsed().as_secs_f64() < 1.0, "CFF subr bomb must reject rapidly, took {:?}", t0.elapsed());
}
