use std::sync::mpsc;
use std::time::Duration;

use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, compile_graph};
use curvebook_doc::{GraphSheet, ProjectDocument};

#[allow(dead_code)]
pub fn compile_guarded(doc: ProjectDocument, graph: GraphSheet) -> ChartScene {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| compile_graph(&graph, &doc)));
        let _ = tx.send(res);
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Err(_) => panic!("compile_graph hung beyond 10s"),
        Ok(Err(_)) => panic!("compile_graph panicked"),
        Ok(Ok(res)) => res.expect("compile_graph returned Err"),
    }
}

#[allow(dead_code)]
pub fn assert_all_primitives_finite(scene: &ChartScene) {
    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::Path { path, clip, .. } => {
                for cmd in &path.commands {
                    match *cmd {
                        PathCmd::MoveTo(p) | PathCmd::LineTo(p) => {
                            assert!(p.x.is_finite() && p.y.is_finite(), "non-finite path point: {p:?}");
                        }
                        PathCmd::QuadTo { control, to } => {
                            assert!(control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite());
                        }
                        PathCmd::CubicTo { c1, c2, to } => {
                            assert!(
                                c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
                            );
                        }
                        PathCmd::Close => {}
                    }
                }
                if let Some(c) = clip {
                    assert!(c.x.is_finite() && c.y.is_finite() && c.width.is_finite() && c.height.is_finite());
                }
            }
            ScenePrimitive::Text(t) => {
                assert!(t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite() && t.font_size_pt.is_finite());
            }
            ScenePrimitive::RasterImage(img) => {
                assert!(
                    img.bounds_pt.x.is_finite()
                        && img.bounds_pt.y.is_finite()
                        && img.bounds_pt.width.is_finite()
                        && img.bounds_pt.height.is_finite()
                        && img.rotation_deg.is_finite()
                );
            }
            ScenePrimitive::PushClip { rect } => {
                assert!(rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite());
            }
            ScenePrimitive::PopClip => {}
        }
    }
}

#[allow(dead_code)]
pub fn scene_texts(scene: &ChartScene) -> Vec<String> {
    scene
        .primitives
        .iter()
        .filter_map(|p| match p {
            ScenePrimitive::Text(t) => Some(t.rich.plain_text()),
            _ => None,
        })
        .collect()
}

#[allow(dead_code)]
pub fn all_points_finite(scene: &ChartScene) -> bool {
    scene.primitives.iter().all(|p| match p {
        ScenePrimitive::Path { path, clip, .. } => {
            path.commands.iter().all(|c| match *c {
                PathCmd::MoveTo(pt) | PathCmd::LineTo(pt) => pt.x.is_finite() && pt.y.is_finite(),
                PathCmd::QuadTo { control, to } => control.x.is_finite() && control.y.is_finite() && to.x.is_finite() && to.y.is_finite(),
                PathCmd::CubicTo { c1, c2, to } => {
                    c1.x.is_finite() && c1.y.is_finite() && c2.x.is_finite() && c2.y.is_finite() && to.x.is_finite() && to.y.is_finite()
                }
                PathCmd::Close => true,
            }) && clip.is_none_or(|r| r.x.is_finite() && r.y.is_finite() && r.width.is_finite() && r.height.is_finite())
        }
        ScenePrimitive::Text(t) => t.anchor_pt.x.is_finite() && t.anchor_pt.y.is_finite() && t.font_size_pt.is_finite() && t.rotation_deg.is_finite(),
        ScenePrimitive::RasterImage(img) => {
            img.bounds_pt.x.is_finite()
                && img.bounds_pt.y.is_finite()
                && img.bounds_pt.width.is_finite()
                && img.bounds_pt.height.is_finite()
                && img.rotation_deg.is_finite()
        }
        ScenePrimitive::PushClip { rect } => rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite(),
        ScenePrimitive::PopClip => true,
    })
}

#[allow(dead_code)]
pub fn assert_point_bits_eq(a: PointPt, b: PointPt, ctx: &str) {
    assert_eq!(a.x.to_bits(), b.x.to_bits(), "x bit mismatch at {ctx}");
    assert_eq!(a.y.to_bits(), b.y.to_bits(), "y bit mismatch at {ctx}");
}

#[allow(dead_code)]
pub fn assert_rect_bits_eq(a: RectPt, b: RectPt, ctx: &str) {
    assert_eq!(a.x.to_bits(), b.x.to_bits(), "rect.x bit mismatch at {ctx}");
    assert_eq!(a.y.to_bits(), b.y.to_bits(), "rect.y bit mismatch at {ctx}");
    assert_eq!(a.width.to_bits(), b.width.to_bits(), "rect.width bit mismatch at {ctx}");
    assert_eq!(a.height.to_bits(), b.height.to_bits(), "rect.height bit mismatch at {ctx}");
}

#[allow(dead_code)]
pub fn assert_paths_bit_identical(p1: &Path2D, p2: &Path2D, idx: usize) {
    assert_eq!(p1.commands.len(), p2.commands.len(), "path command count mismatch at {idx}");
    for (cmd_idx, (ca, cb)) in p1.commands.iter().zip(&p2.commands).enumerate() {
        let loc = format!("prim {idx} cmd {cmd_idx}");
        match (ca, cb) {
            (PathCmd::MoveTo(u), PathCmd::MoveTo(v)) | (PathCmd::LineTo(u), PathCmd::LineTo(v)) => assert_point_bits_eq(*u, *v, &loc),
            (PathCmd::QuadTo { control: c1a, to: t1 }, PathCmd::QuadTo { control: c1b, to: t2 }) => {
                assert_point_bits_eq(*c1a, *c1b, &loc);
                assert_point_bits_eq(*t1, *t2, &loc);
            }
            (PathCmd::CubicTo { c1: a1, c2: a2, to: t1 }, PathCmd::CubicTo { c1: b1, c2: b2, to: t2 }) => {
                assert_point_bits_eq(*a1, *b1, &loc);
                assert_point_bits_eq(*a2, *b2, &loc);
                assert_point_bits_eq(*t1, *t2, &loc);
            }
            (PathCmd::Close, PathCmd::Close) => {}
            _ => assert_eq!(std::mem::discriminant(ca), std::mem::discriminant(cb), "mismatched path cmd at {loc}: {ca:?} vs {cb:?}"),
        }
    }
}

#[allow(dead_code)]
pub fn assert_scenes_bit_identical(a: &ChartScene, b: &ChartScene) {
    assert_eq!(a.sheet_id, b.sheet_id);
    assert_eq!(a.title, b.title);
    assert_eq!(a.width_pt.to_bits(), b.width_pt.to_bits());
    assert_eq!(a.height_pt.to_bits(), b.height_pt.to_bits());
    assert_eq!(a.background, b.background);
    assert_rect_bits_eq(a.plot_rect_pt, b.plot_rect_pt, "plot_rect_pt");
    assert_eq!(a.primitives.len(), b.primitives.len(), "primitive count mismatch");
    for (idx, (pa, pb)) in a.primitives.iter().zip(&b.primitives).enumerate() {
        match (pa, pb) {
            (ScenePrimitive::Text(ta), ScenePrimitive::Text(tb)) => {
                assert_eq!(ta.rich, tb.rich, "rich text mismatch at {idx}");
                assert_point_bits_eq(ta.anchor_pt, tb.anchor_pt, &format!("text {idx}"));
                assert_eq!(ta.font_size_pt.to_bits(), tb.font_size_pt.to_bits());
                assert_eq!(ta.rotation_deg.to_bits(), tb.rotation_deg.to_bits());
                assert_eq!(ta.anchor, tb.anchor);
                assert_eq!(ta.color, tb.color);
            }
            (
                ScenePrimitive::Path { path: p1, fill: f1, stroke: s1, clip: c1 },
                ScenePrimitive::Path { path: p2, fill: f2, stroke: s2, clip: c2 },
            ) => {
                assert_eq!(f1, f2, "fill mismatch at {idx}");
                assert_eq!(c1.is_some(), c2.is_some(), "clip presence mismatch at {idx}");
                if let (Some(ra), Some(rb)) = (c1, c2) {
                    assert_rect_bits_eq(*ra, *rb, &format!("path clip {idx}"));
                }
                assert_paths_bit_identical(p1, p2, idx);
                match (s1, s2) {
                    (Some(sa), Some(sb)) => {
                        assert_eq!(sa.color, sb.color);
                        assert_eq!(sa.width_pt.to_bits(), sb.width_pt.to_bits());
                        assert_eq!(sa.dash, sb.dash);
                        assert_eq!(sa.cap, sb.cap);
                        assert_eq!(sa.join, sb.join);
                    }
                    (None, None) => {}
                    _ => assert_eq!(s1.is_some(), s2.is_some(), "stroke presence mismatch at {idx}"),
                }
            }
            (ScenePrimitive::RasterImage(ia), ScenePrimitive::RasterImage(ib)) => {
                assert_eq!((&ia.asset_id, ia.pixel_size, &ia.rgba), (&ib.asset_id, ib.pixel_size, &ib.rgba));
                assert_rect_bits_eq(ia.bounds_pt, ib.bounds_pt, &format!("raster {idx}"));
                assert_eq!(ia.rotation_deg.to_bits(), ib.rotation_deg.to_bits());
            }
            (ScenePrimitive::PushClip { rect: r1 }, ScenePrimitive::PushClip { rect: r2 }) => {
                assert_rect_bits_eq(*r1, *r2, &format!("push_clip {idx}"));
            }
            (ScenePrimitive::PopClip, ScenePrimitive::PopClip) => {}
            _ => assert_eq!(std::mem::discriminant(pa), std::mem::discriminant(pb), "primitive variant mismatch at {idx}"),
        }
    }
}
