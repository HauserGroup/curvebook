use std::sync::mpsc;
use std::time::Duration;

use curvebook_chart::{ChartScene, PathCmd, ScenePrimitive, compile_graph};
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
