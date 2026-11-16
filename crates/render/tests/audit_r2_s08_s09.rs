//! Regression tests for Round 2 Slice S09 (`R2-S09-01`, `R2-S09-02`): ECMA-234 `.emf` path & text encoding.

use lumencraft_chart::{ChartScene, FillPaint, Path2D, PathCmd, PointPt, PositionedText, ScenePrimitive, StrokePaint, TextAnchor};
use lumencraft_palette::Rgba8;
use lumencraft_render::encode_emf;
use lumencraft_text::RichText;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const EMR_POLYBEZIER: u32 = 2;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_MOVETOEX: u32 = 27;
const EMR_LINETO: u32 = 54;
const EMR_BEGINPATH: u32 = 59;
const EMR_ENDPATH: u32 = 60;
const EMR_CLOSEFIGURE: u32 = 61;
const EMR_EXTTEXTOUTW: u32 = 84;

fn read_u32_le(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn read_i32_le(bytes: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
}

fn collect_records(emf: &[u8]) -> Vec<(u32, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= emf.len() {
        let i_type = read_u32_le(emf, off);
        let n_size = read_u32_le(emf, off + 4) as usize;
        assert!(n_size >= 8 && off + n_size <= emf.len());
        out.push((i_type, off));
        off += n_size;
    }
    assert_eq!(off, emf.len());
    out
}

#[test]
fn r2_s09_01_emf_path_uses_polybezier_to_and_skips_move_only_paths() -> TestResult {
    // Move-only paths (`MoveTo` alone or `MoveTo` + `Close`) must emit zero `EMR_BEGINPATH`/`EMR_MOVETOEX`.
    let mut move_only_scene = ChartScene::new("emf-move", "Move Only", 200.0, 150.0, None);
    let mut p_move = Path2D::new();
    p_move.commands.push(PathCmd::MoveTo(PointPt::new(20.0, 20.0)));
    move_only_scene.primitives.push(ScenePrimitive::Path {
        path: p_move,
        fill: None,
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    let mut p_close = Path2D::new();
    p_close.commands.push(PathCmd::MoveTo(PointPt::new(40.0, 40.0)));
    p_close.commands.push(PathCmd::Close);
    move_only_scene.primitives.push(ScenePrimitive::Path {
        path: p_close,
        fill: Some(FillPaint::solid(Rgba8::BLACK)),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    let move_emf = encode_emf(&move_only_scene)?;
    let move_recs = collect_records(&move_emf);
    assert!(!move_recs.iter().any(|&(t, _)| t == EMR_BEGINPATH), "move-only path must not emit EMR_BEGINPATH");
    assert!(!move_recs.iter().any(|&(t, _)| t == EMR_MOVETOEX), "move-only path must not emit EMR_MOVETOEX");
    assert!(!move_recs.iter().any(|&(t, _)| t == EMR_ENDPATH), "move-only path must not emit EMR_ENDPATH");

    // Drawable Bezier + Line path must use `EMR_POLYBEZIERTO` (5) with multiples of 3 points
    // and never `EMR_POLYBEZIER` (2) inside BeginPath..EndPath, nor inject redundant `EMR_MOVETOEX` before LineTo.
    let mut curve_scene = ChartScene::new("emf-curve", "Curve Path", 240.0, 180.0, None);
    let mut curve = Path2D::new();
    curve.commands.push(PathCmd::MoveTo(PointPt::new(30.0, 120.0)));
    curve.commands.push(PathCmd::CubicTo { c1: PointPt::new(60.0, 60.0), c2: PointPt::new(110.0, 40.0), to: PointPt::new(150.0, 80.0) });
    curve.commands.push(PathCmd::LineTo(PointPt::new(180.0, 130.0)));
    curve.commands.push(PathCmd::QuadTo { control: PointPt::new(100.0, 150.0), to: PointPt::new(30.0, 140.0) });
    curve.commands.push(PathCmd::Close);
    curve_scene.primitives.push(ScenePrimitive::Path {
        path: curve,
        fill: Some(FillPaint::solid(Rgba8::rgb(80, 140, 200))),
        stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
        clip: None,
    });
    let curve_emf = encode_emf(&curve_scene)?;
    let curve_recs = collect_records(&curve_emf);

    let begin_idx = curve_recs.iter().position(|&(t, _)| t == EMR_BEGINPATH).ok_or("missing EMR_BEGINPATH")?;
    let end_idx = curve_recs.iter().position(|&(t, _)| t == EMR_ENDPATH).ok_or("missing EMR_ENDPATH")?;
    let path_seq: Vec<u32> = curve_recs[begin_idx..=end_idx].iter().map(|&(t, _)| t).collect();
    assert_eq!(
        path_seq,
        vec![EMR_BEGINPATH, EMR_MOVETOEX, EMR_POLYBEZIERTO, EMR_LINETO, EMR_POLYBEZIERTO, EMR_CLOSEFIGURE, EMR_ENDPATH],
        "unexpected GDI path sequence: {path_seq:?}"
    );
    assert!(!path_seq.contains(&EMR_POLYBEZIER), "EMR_POLYBEZIER (2) must not appear inside path");
    for &(t, off) in &curve_recs[begin_idx..=end_idx] {
        if t == EMR_POLYBEZIERTO {
            let cptl = read_u32_le(&curve_emf, off + 24);
            assert!(cptl >= 3 && cptl.is_multiple_of(3), "invalid EMR_POLYBEZIERTO count {cptl}");
        }
    }
    Ok(())
}

#[test]
fn r2_s09_02_emf_rotated_text_bounds_and_multiline_newlines() -> TestResult {
    let mut scene = ChartScene::new("emf-text", "Rotated and Multiline", 320.0, 240.0, None);
    // Vertical Y-axis title (-90 deg) near x = 28 pt, middle anchored at y = 120 pt, 12 pt font.
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Vertical Title"),
        anchor_pt: PointPt::new(28.0, 120.0),
        anchor: TextAnchor::Middle,
        font_size_pt: 12.0,
        rotation_deg: -90.0,
        color: Rgba8::BLACK,
    }));
    // Multi-line label with `\n` must split across two EMR_EXTTEXTOUTW records cleanly.
    scene.primitives.push(ScenePrimitive::Text(PositionedText {
        rich: RichText::plain("Line Alpha\nLine Beta"),
        anchor_pt: PointPt::new(160.0, 100.0),
        anchor: TextAnchor::Start,
        font_size_pt: 10.0,
        rotation_deg: 0.0,
        color: Rgba8::BLACK,
    }));

    let emf = encode_emf(&scene)?;
    let recs = collect_records(&emf);

    let mut text_recs: Vec<(String, [i32; 4], i32, i32)> = Vec::new();
    for &(t, off) in &recs {
        if t == EMR_EXTTEXTOUTW {
            let n_size = read_u32_le(&emf, off + 4) as usize;
            let rec = &emf[off..off + n_size];
            let bounds = [read_i32_le(rec, 8), read_i32_le(rec, 12), read_i32_le(rec, 16), read_i32_le(rec, 20)];
            let ref_x = read_i32_le(rec, 36);
            let ref_y = read_i32_le(rec, 40);
            let n_chars = read_u32_le(rec, 44) as usize;
            let off_str = read_u32_le(rec, 48) as usize;
            let units: Vec<u16> = rec[off_str..off_str + n_chars * 2].as_chunks::<2>().0.iter().map(|&p| u16::from_le_bytes(p)).collect();
            let text = String::from_utf16(&units)?;
            text_recs.push((text, bounds, ref_x, ref_y));
        }
    }

    assert_eq!(text_recs.len(), 3, "expected 1 rotated + 2 split multi-line records: {text_recs:?}");
    let (rot_text, rot_bounds, _, _) = &text_recs[0];
    assert_eq!(rot_text, "Vertical Title");
    let width_dev = rot_bounds[2] - rot_bounds[0];
    let height_dev = rot_bounds[3] - rot_bounds[1];
    assert!(width_dev >= 15, "rotated text bounds must span font height horizontally, got bounds={rot_bounds:?}");
    assert!(height_dev >= 40, "rotated text bounds must span text advance vertically, got bounds={rot_bounds:?}");

    assert_eq!(text_recs[1].0, "Line Alpha");
    assert_eq!(text_recs[2].0, "Line Beta");
    for (txt, _, _, _) in &text_recs {
        assert!(!txt.contains('\n') && !txt.contains('\r'), "EMR_EXTTEXTOUTW must never emit raw newline in `{txt:?}`");
    }
    assert!(text_recs[2].3 > text_recs[1].3, "second line baseline Y ({}) must advance below first ({})", text_recs[2].3, text_recs[1].3);
    Ok(())
}
