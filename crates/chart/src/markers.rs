//! Canonical scientific marker shapes, fill modes, and shared ellipse/polygon geometry.

use curvebook_palette::{MarkerFillMode, MarkerShape, MarkerStyle};

use crate::scene::{FillPaint, Path2D, PathCmd, PointPt, RectPt, ScenePrimitive, StrokePaint};

const ELLIPSE_KAPPA: f64 = 0.552_284_749_830_793_6;

/// Construct a smooth closed cubic-Bezier ellipse inside `r`.
#[must_use]
pub fn ellipse_path(r: RectPt) -> Path2D {
    rotated_ellipse_path(r.x + 0.5 * r.width, r.y + 0.5 * r.height, 0.5 * r.width, 0.5 * r.height, 0.0)
}

/// Construct a smooth closed cubic-Bezier ellipse centered at `(cx, cy)` with semi-axes `(rx, ry)`
/// rotated clockwise by `angle_rad` radians.
#[must_use]
pub fn rotated_ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64, angle_rad: f64) -> Path2D {
    let (sin_a, cos_a) = angle_rad.sin_cos();
    let xf = |lx: f64, ly: f64| PointPt::new(cx + lx * cos_a - ly * sin_a, cy + lx * sin_a + ly * cos_a);
    let ox = rx * ELLIPSE_KAPPA;
    let oy = ry * ELLIPSE_KAPPA;
    Path2D {
        commands: vec![
            PathCmd::MoveTo(xf(rx, 0.0)),
            PathCmd::CubicTo { c1: xf(rx, oy), c2: xf(ox, ry), to: xf(0.0, ry) },
            PathCmd::CubicTo { c1: xf(-ox, ry), c2: xf(-rx, oy), to: xf(-rx, 0.0) },
            PathCmd::CubicTo { c1: xf(-rx, -oy), c2: xf(-ox, -ry), to: xf(0.0, -ry) },
            PathCmd::CubicTo { c1: xf(ox, -ry), c2: xf(rx, -oy), to: xf(rx, 0.0) },
            PathCmd::Close,
        ],
    }
}

/// Construct a closed polygon path from `pts`.
#[must_use]
pub(crate) fn polygon_path(pts: &[PointPt]) -> Path2D {
    let Some(&first) = pts.first() else {
        return Path2D::new();
    };
    let mut commands = Vec::with_capacity(pts.len() + 1);
    commands.push(PathCmd::MoveTo(first));
    for &p in pts.iter().skip(1) {
        commands.push(PathCmd::LineTo(p));
    }
    commands.push(PathCmd::Close);
    Path2D { commands }
}

/// Return the axis-aligned intersection of `a` and `b` (clamped to non-negative width/height).
#[must_use]
pub(crate) fn intersect_rects(a: RectPt, b: RectPt) -> RectPt {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    RectPt::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Build the canonical [`Path2D`] geometry for any of the 11 [`MarkerShape`] variants centered at
/// `center` with outer radius `radius`.
#[must_use]
pub fn build_marker_path(center: PointPt, radius: f64, shape: MarkerShape) -> Path2D {
    let r = radius.max(0.5);
    let c = center;
    match shape {
        MarkerShape::Circle => ellipse_path(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
        MarkerShape::Square => Path2D::rect(RectPt::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)),
        MarkerShape::TriangleUp => polygon_path(&[
            PointPt::new(c.x, c.y - r),
            PointPt::new(c.x + 0.866_025 * r, c.y + 0.5 * r),
            PointPt::new(c.x - 0.866_025 * r, c.y + 0.5 * r),
        ]),
        MarkerShape::TriangleDown => polygon_path(&[
            PointPt::new(c.x, c.y + r),
            PointPt::new(c.x + 0.866_025 * r, c.y - 0.5 * r),
            PointPt::new(c.x - 0.866_025 * r, c.y - 0.5 * r),
        ]),
        MarkerShape::Diamond => {
            polygon_path(&[PointPt::new(c.x, c.y - r), PointPt::new(c.x + r, c.y), PointPt::new(c.x, c.y + r), PointPt::new(c.x - r, c.y)])
        }
        MarkerShape::Hexagon => {
            let mut pts = Vec::with_capacity(6);
            for i in 0..6 {
                let ang = (i as f64) * std::f64::consts::FRAC_PI_3 - std::f64::consts::FRAC_PI_6;
                pts.push(PointPt::new(c.x + r * ang.cos(), c.y + r * ang.sin()));
            }
            polygon_path(&pts)
        }
        MarkerShape::Star => {
            let mut pts = Vec::with_capacity(10);
            for i in 0..10 {
                let ang = (i as f64) * std::f64::consts::PI / 5.0 - std::f64::consts::FRAC_PI_2;
                let rad = if i % 2 == 0 { r } else { r * 0.42 };
                pts.push(PointPt::new(c.x + rad * ang.cos(), c.y + rad * ang.sin()));
            }
            polygon_path(&pts)
        }
        MarkerShape::Cross => {
            let d = r * 0.72;
            Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(c.x - d, c.y - d)),
                    PathCmd::LineTo(PointPt::new(c.x + d, c.y + d)),
                    PathCmd::MoveTo(PointPt::new(c.x - d, c.y + d)),
                    PathCmd::LineTo(PointPt::new(c.x + d, c.y - d)),
                ],
            }
        }
        MarkerShape::Plus => Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(c.x - r, c.y)),
                PathCmd::LineTo(PointPt::new(c.x + r, c.y)),
                PathCmd::MoveTo(PointPt::new(c.x, c.y - r)),
                PathCmd::LineTo(PointPt::new(c.x, c.y + r)),
            ],
        },
        MarkerShape::HorizontalBar => Path2D::line(PointPt::new(c.x - r, c.y), PointPt::new(c.x + r, c.y)),
        MarkerShape::VerticalBar => Path2D::line(PointPt::new(c.x, c.y - r), PointPt::new(c.x, c.y + r)),
    }
}

/// Emit any of the 11 [`MarkerShape`]s and 6 [`MarkerFillMode`]s at `center`, preserving and
/// intersecting any incoming `clip` rectangle for semi-filled markers.
pub fn emit_marker(primitives: &mut Vec<ScenePrimitive>, center: PointPt, style: &MarkerStyle, clip: Option<RectPt>) {
    let r = (style.size_pt * 0.5).max(0.5);
    let path = build_marker_path(center, r, style.shape);
    let stroke = (style.stroke_width_pt > 0.0).then(|| StrokePaint::solid(style.stroke_color, style.stroke_width_pt));

    if is_open_stroke_shape(style.shape) {
        let line_stroke = Some(StrokePaint::solid(style.stroke_color, style.stroke_width_pt.max(1.0)));
        primitives.push(ScenePrimitive::Path { path, fill: None, stroke: line_stroke, clip });
        return;
    }

    match style.fill_mode {
        MarkerFillMode::Filled => {
            primitives.push(ScenePrimitive::Path { path, fill: Some(FillPaint::solid(style.fill_color)), stroke, clip });
        }
        MarkerFillMode::Open => {
            primitives.push(ScenePrimitive::Path {
                path,
                fill: None,
                stroke: stroke.or_else(|| Some(StrokePaint::solid(style.stroke_color, 1.0))),
                clip,
            });
        }
        MarkerFillMode::SemiFilledTop | MarkerFillMode::SemiFilledBottom | MarkerFillMode::SemiFilledLeft | MarkerFillMode::SemiFilledRight => {
            let half_rect = semi_fill_rect(center, r, style.fill_mode);
            let combined_clip = clip.map_or(half_rect, |c| intersect_rects(c, half_rect));
            primitives.push(ScenePrimitive::Path {
                path: path.clone(),
                fill: Some(FillPaint::solid(style.fill_color)),
                stroke: None,
                clip: Some(combined_clip),
            });
            primitives.push(ScenePrimitive::Path {
                path,
                fill: None,
                stroke: stroke.or_else(|| Some(StrokePaint::solid(style.stroke_color, 1.0))),
                clip,
            });
        }
    }
}

const fn is_open_stroke_shape(shape: MarkerShape) -> bool {
    matches!(shape, MarkerShape::Cross | MarkerShape::Plus | MarkerShape::HorizontalBar | MarkerShape::VerticalBar)
}

fn semi_fill_rect(center: PointPt, r: f64, mode: MarkerFillMode) -> RectPt {
    let pad = r + 1.0;
    match mode {
        MarkerFillMode::SemiFilledTop => RectPt::new(center.x - pad, center.y - pad, 2.0 * pad, pad),
        MarkerFillMode::SemiFilledBottom => RectPt::new(center.x - pad, center.y, 2.0 * pad, pad),
        MarkerFillMode::SemiFilledLeft => RectPt::new(center.x - pad, center.y - pad, pad, 2.0 * pad),
        MarkerFillMode::SemiFilledRight => RectPt::new(center.x, center.y - pad, pad, 2.0 * pad),
        MarkerFillMode::Filled | MarkerFillMode::Open => RectPt::new(center.x - pad, center.y - pad, 2.0 * pad, 2.0 * pad),
    }
}

#[cfg(test)]
mod tests {
    use curvebook_palette::{MarkerFillMode, MarkerShape, MarkerStyle, Rgba8};

    use super::{build_marker_path, emit_marker};
    use crate::scene::{PointPt, RectPt, ScenePrimitive};

    #[test]
    fn semi_filled_marker_intersects_incoming_clip_rect() {
        let plot_clip = RectPt::new(50.0, 20.0, 100.0, 80.0);
        // Marker positioned near the top-left corner of `plot_clip` so its half-rect extends past `plot_clip`.
        let edge_center = PointPt::new(51.0, 21.0);
        for fill_mode in
            [MarkerFillMode::SemiFilledTop, MarkerFillMode::SemiFilledBottom, MarkerFillMode::SemiFilledLeft, MarkerFillMode::SemiFilledRight]
        {
            let style = MarkerStyle {
                shape: MarkerShape::Circle,
                size_pt: 10.0,
                fill_mode,
                fill_color: Rgba8::rgb(0, 114, 178),
                stroke_color: Rgba8::BLACK,
                stroke_width_pt: 0.9,
            };
            let mut prims = Vec::new();
            emit_marker(&mut prims, edge_center, &style, Some(plot_clip));
            assert_eq!(prims.len(), 2, "semi-filled marker emits fill + stroke primitives");
            let fill_clip = match prims.first() {
                Some(ScenePrimitive::Path { clip: Some(c), fill: Some(_), .. }) => *c,
                other => {
                    assert!(other.is_none(), "expected clipped fill path as first primitive, got {other:?}");
                    continue;
                }
            };
            assert!(fill_clip.x >= plot_clip.x - 1e-12, "{fill_mode:?}: fill clip left escaped plot_clip");
            assert!(fill_clip.y >= plot_clip.y - 1e-12, "{fill_mode:?}: fill clip top escaped plot_clip");
            assert!(fill_clip.right() <= plot_clip.right() + 1e-12, "{fill_mode:?}: fill clip right escaped plot_clip");
            assert!(fill_clip.bottom() <= plot_clip.bottom() + 1e-12, "{fill_mode:?}: fill clip bottom escaped plot_clip");
        }
    }

    #[test]
    fn all_eleven_marker_shapes_build_non_empty_paths() {
        let shapes = [
            MarkerShape::Circle,
            MarkerShape::Square,
            MarkerShape::TriangleUp,
            MarkerShape::TriangleDown,
            MarkerShape::Diamond,
            MarkerShape::Hexagon,
            MarkerShape::Star,
            MarkerShape::Cross,
            MarkerShape::Plus,
            MarkerShape::HorizontalBar,
            MarkerShape::VerticalBar,
        ];
        for shape in shapes {
            let path = build_marker_path(PointPt::new(10.0, 20.0), 4.0, shape);
            assert!(!path.commands.is_empty(), "{shape:?} produced empty path");
        }
    }
}
