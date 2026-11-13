//! Deterministic software rasterizer converting [`lumencraft_chart::ChartScene`] into RGBA pixels.

use lumencraft_chart::{ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};
use lumencraft_palette::{LineDashStyle, Rgba8};
use lumencraft_text::{GlyphOutlineCmd, outline_rich_text};
use serde::{Deserialize, Serialize};

use crate::RenderTargetSpec;

/// Maximum supported raster width or height in pixels (`16384 x 16384`).
pub const MAX_RASTER_DIM_PX: u32 = 16_384;

const BEZIER_FLATNESS_TOL_PX: f64 = 0.15;
const MAX_BEZIER_DEPTH: u8 = 12;
const MITER_LIMIT: f64 = 4.0;
const SUBPIXEL_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];

/// Uncompressed RGBA pixel buffer paired with resolution and physical DPI metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterImage {
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
    pub rgba: Vec<u8>,
}

impl RasterImage {
    /// Allocate a pixel buffer validated against zero dimensions, max bounds, and size overflows.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidTarget`] on zero/oversized dimensions, zero DPI, or overflow.
    pub fn new(width_px: u32, height_px: u32, dpi: u16, fill: Rgba8) -> Result<Self, RenderError> {
        if width_px == 0 || height_px == 0 || dpi == 0 {
            return Err(RenderError::InvalidTarget { message: String::from("raster width, height, and dpi must be positive") });
        }
        if width_px > MAX_RASTER_DIM_PX || height_px > MAX_RASTER_DIM_PX {
            return Err(RenderError::InvalidTarget {
                message: format!("raster dimensions {width_px}x{height_px} exceed maximum {MAX_RASTER_DIM_PX}x{MAX_RASTER_DIM_PX}"),
            });
        }
        let pixel_count = (width_px as usize)
            .checked_mul(height_px as usize)
            .ok_or_else(|| RenderError::InvalidTarget { message: String::from("raster dimensions overflow addressable pixel count") })?;
        let byte_len =
            pixel_count.checked_mul(4).ok_or_else(|| RenderError::InvalidTarget { message: String::from("raster byte buffer size overflow") })?;
        let mut rgba = Vec::with_capacity(byte_len);
        for _ in 0..pixel_count {
            rgba.extend_from_slice(&[fill.r, fill.g, fill.b, fill.a]);
        }
        Ok(Self { width_px, height_px, dpi, rgba })
    }

    /// Expected RGBA byte buffer length (`width_px * height_px * 4`).
    #[must_use]
    pub fn expected_byte_len(&self) -> Option<usize> {
        (self.width_px as usize).checked_mul(self.height_px as usize).and_then(|p| p.checked_mul(4))
    }

    /// Composite a pixel at `(x, y)` using straight-alpha Porter-Duff `source-over`.
    pub fn blend_pixel(&mut self, x: u32, y: u32, src: Rgba8) {
        self.blend_pixel_coverage(x, y, src, 1.0);
    }

    /// Composite a pixel at `(x, y)` with fractional geometric coverage in `[0.0, 1.0]`.
    pub(crate) fn blend_pixel_coverage(&mut self, x: u32, y: u32, src: Rgba8, coverage: f64) {
        if x >= self.width_px || y >= self.height_px || src.a == 0 || !coverage.is_finite() || coverage <= 0.0 {
            return;
        }
        let cov = coverage.clamp(0.0, 1.0);
        let idx = ((y as usize) * (self.width_px as usize) + (x as usize)) * 4;
        let Some(px) = self.rgba.get_mut(idx..idx + 4) else {
            return;
        };

        if src.a == u8::MAX && cov >= 1.0 {
            px[0] = src.r;
            px[1] = src.g;
            px[2] = src.b;
            px[3] = u8::MAX;
            return;
        }

        let sa = (f64::from(src.a) / 255.0) * cov;
        if sa <= 0.0 {
            return;
        }
        if px[3] == 0 {
            px[0] = src.r;
            px[1] = src.g;
            px[2] = src.b;
            px[3] = clamp_u8(sa * 255.0);
            return;
        }
        if px[3] == u8::MAX {
            let inv = 1.0 - sa;
            px[0] = clamp_u8(f64::from(src.r) * sa + f64::from(px[0]) * inv);
            px[1] = clamp_u8(f64::from(src.g) * sa + f64::from(px[1]) * inv);
            px[2] = clamp_u8(f64::from(src.b) * sa + f64::from(px[2]) * inv);
            px[3] = u8::MAX;
            return;
        }

        let da = f64::from(px[3]) / 255.0;
        let da_inv = da * (1.0 - sa);
        let out_a = sa + da_inv;
        if out_a <= 1e-12 {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
            px[3] = 0;
            return;
        }
        px[0] = clamp_u8((f64::from(src.r) * sa + f64::from(px[0]) * da_inv) / out_a);
        px[1] = clamp_u8((f64::from(src.g) * sa + f64::from(px[1]) * da_inv) / out_a);
        px[2] = clamp_u8((f64::from(src.b) * sa + f64::from(px[2]) * da_inv) / out_a);
        px[3] = clamp_u8(out_a * 255.0);
    }
}

/// Errors produced by CPU rasterization or PNG/TIFF encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderError {
    InvalidTarget { message: String },
    BufferSizeMismatch { expected: usize, actual: usize },
    EncodingFailed { message: String },
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTarget { message } => write!(f, "invalid raster target: {message}"),
            Self::BufferSizeMismatch { expected, actual } => {
                write!(f, "RGBA buffer length {actual} does not match expected {expected}")
            }
            Self::EncodingFailed { message } => write!(f, "image encoding failed: {message}"),
        }
    }
}

impl std::error::Error for RenderError {}

/// Rasterize a [`ChartScene`] onto a deterministic RGBA buffer matching `target`.
///
/// # Errors
/// Returns [`RenderError::InvalidTarget`] when `target` or scene bounds are invalid.
pub fn render_scene(scene: &ChartScene, target: RenderTargetSpec) -> Result<RasterImage, RenderError> {
    if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
        return Err(RenderError::InvalidTarget { message: String::from("scene dimensions must be finite and positive") });
    }
    let sx = f64::from(target.width_px) / scene.width_pt;
    let sy = f64::from(target.height_px) / scene.height_pt;
    if !sx.is_finite() || !sy.is_finite() || sx <= 0.0 || sy <= 0.0 {
        return Err(RenderError::InvalidTarget { message: String::from("scene scale factors must be finite and positive") });
    }
    let stroke_scale = (sx * sy).sqrt();

    let initial_bg = initial_background_fill(scene);
    let mut img = RasterImage::new(target.width_px, target.height_px, target.dpi, initial_bg)?;
    let canvas_clip = ClipRectPx::from_canvas(target.width_px, target.height_px);
    let mut clip_stack = vec![canvas_clip];

    for prim in &scene.primitives {
        let active_clip = clip_stack.last().copied().unwrap_or(canvas_clip);
        match prim {
            ScenePrimitive::Path { path, fill, stroke, clip } => {
                let effective_clip = match clip.and_then(|r| ClipRectPx::from_rect_pt(r, sx, sy)) {
                    Some(local) => active_clip.intersect(local),
                    None if clip.is_some() => ClipRectPx::empty(),
                    None => active_clip,
                };
                if !effective_clip.is_empty() && !path.is_empty() {
                    rasterize_path(&mut img, path, *fill, *stroke, sx, sy, stroke_scale, effective_clip);
                }
            }
            ScenePrimitive::Text(text) => {
                if !active_clip.is_empty() {
                    render_text(&mut img, text, sx, sy, active_clip);
                }
            }
            ScenePrimitive::PushClip { rect } => {
                let next = match ClipRectPx::from_rect_pt(*rect, sx, sy) {
                    Some(local) => active_clip.intersect(local),
                    None => ClipRectPx::empty(),
                };
                clip_stack.push(next);
            }
            ScenePrimitive::PopClip => {
                if clip_stack.len() > 1 {
                    let _ = clip_stack.pop();
                }
            }
        }
    }

    Ok(img)
}

fn initial_background_fill(scene: &ChartScene) -> Rgba8 {
    let Some(bg) = scene.background else {
        return Rgba8::TRANSPARENT;
    };
    if bg.a == u8::MAX {
        return bg;
    }
    // Avoid double-blending translucent backgrounds when `ChartScene::new` already pushed the
    // matching full-canvas background rect as its first primitive.
    let expected_bg_path = Path2D::rect(RectPt::new(0.0, 0.0, scene.width_pt, scene.height_pt));
    if let Some(ScenePrimitive::Path { path, fill: Some(fill), stroke: None, clip: None }) = scene.primitives.first()
        && fill.color == bg
        && *path == expected_bg_path
    {
        return Rgba8::TRANSPARENT;
    }
    bg
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clamp_u8(val: f64) -> u8 {
    if !val.is_finite() {
        return 0;
    }
    val.round().clamp(0.0, 255.0) as u8
}

#[derive(Debug, Clone, Copy)]
struct PointPx {
    x: f64,
    y: f64,
}

impl PointPx {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    fn dist_sq(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone, Copy)]
struct ClipRectPx {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl ClipRectPx {
    fn empty() -> Self {
        Self { min_x: 0.0, min_y: 0.0, max_x: 0.0, max_y: 0.0 }
    }

    fn from_canvas(width_px: u32, height_px: u32) -> Self {
        Self { min_x: 0.0, min_y: 0.0, max_x: f64::from(width_px), max_y: f64::from(height_px) }
    }

    fn from_rect_pt(rect: RectPt, sx: f64, sy: f64) -> Option<Self> {
        if !rect.x.is_finite() || !rect.y.is_finite() || !rect.width.is_finite() || !rect.height.is_finite() {
            return None;
        }
        let min_x = rect.x * sx;
        let min_y = rect.y * sy;
        let max_x = (rect.x + rect.width.max(0.0)) * sx;
        let max_y = (rect.y + rect.height.max(0.0)) * sy;
        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            return None;
        }
        Some(Self { min_x, min_y, max_x, max_y })
    }

    fn intersect(self, other: Self) -> Self {
        let min_x = self.min_x.max(other.min_x);
        let min_y = self.min_y.max(other.min_y);
        let max_x = self.max_x.min(other.max_x);
        let max_y = self.max_y.min(other.max_y);
        if max_x <= min_x || max_y <= min_y { Self::empty() } else { Self { min_x, min_y, max_x, max_y } }
    }

    fn is_empty(self) -> bool {
        self.max_x <= self.min_x || self.max_y <= self.min_y
    }
}

struct Subpath {
    points: Vec<PointPx>,
    closed: bool,
    has_segment: bool,
}

#[allow(clippy::too_many_arguments)]
fn rasterize_path(
    img: &mut RasterImage,
    path: &Path2D,
    fill: Option<FillPaint>,
    stroke: Option<StrokePaint>,
    sx: f64,
    sy: f64,
    stroke_scale: f64,
    clip: ClipRectPx,
) {
    let needs_fill = fill.is_some_and(|f| f.color.a > 0);
    let needs_stroke = stroke.is_some_and(|s| s.color.a > 0 && s.width_pt.is_finite() && s.width_pt > 0.0);
    if !needs_fill && !needs_stroke {
        return;
    }

    let subpaths = flatten_path(path, sx, sy);
    if subpaths.is_empty() {
        return;
    }

    if let Some(fill_paint) = fill.filter(|_| needs_fill) {
        fill_subpaths(img, &subpaths, fill_paint.color, clip);
    }
    if let Some(stroke_paint) = stroke.filter(|_| needs_stroke) {
        stroke_subpaths(img, &subpaths, &stroke_paint, stroke_scale, clip);
    }
}

fn flatten_path(path: &Path2D, sx: f64, sy: f64) -> Vec<Subpath> {
    let mut subpaths = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut start_pt: Option<PointPx> = None;

    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(pt) => {
                if let Some(sub) = current.take()
                    && sub.has_segment
                {
                    subpaths.push(sub);
                }
                let p = PointPx::new(pt.x * sx, pt.y * sy);
                if p.is_finite() {
                    start_pt = Some(p);
                    current = Some(Subpath { points: vec![p], closed: false, has_segment: false });
                } else {
                    start_pt = None;
                }
            }
            PathCmd::LineTo(pt) => {
                let p = PointPx::new(pt.x * sx, pt.y * sy);
                if p.is_finite() {
                    push_line_point(&mut current, &mut start_pt, p);
                }
            }
            PathCmd::QuadTo { control, to } => {
                let c = PointPx::new(control.x * sx, control.y * sy);
                let p = PointPx::new(to.x * sx, to.y * sy);
                if c.is_finite() && p.is_finite() {
                    push_quad_points(&mut current, &mut start_pt, c, p);
                }
            }
            PathCmd::CubicTo { c1, c2, to } => {
                let p1 = PointPx::new(c1.x * sx, c1.y * sy);
                let p2 = PointPx::new(c2.x * sx, c2.y * sy);
                let p = PointPx::new(to.x * sx, to.y * sy);
                if p1.is_finite() && p2.is_finite() && p.is_finite() {
                    push_cubic_points(&mut current, &mut start_pt, p1, p2, p);
                }
            }
            PathCmd::Close => {
                if let Some(mut sub) = current.take() {
                    sub.closed = true;
                    if !sub.points.is_empty() {
                        subpaths.push(sub);
                    }
                }
                if let Some(st) = start_pt {
                    current = Some(Subpath { points: vec![st], closed: false, has_segment: false });
                }
            }
        }
    }

    if let Some(sub) = current
        && sub.has_segment
    {
        subpaths.push(sub);
    }

    subpaths
}

fn push_line_point(current: &mut Option<Subpath>, start_pt: &mut Option<PointPx>, p: PointPx) {
    if let Some(sub) = current.as_mut() {
        sub.has_segment = true;
        if sub.points.last().is_none_or(|prev| prev.dist_sq(p) > 1e-18) {
            sub.points.push(p);
        }
    } else {
        *start_pt = Some(p);
        *current = Some(Subpath { points: vec![p], closed: false, has_segment: false });
    }
}

fn push_quad_points(current: &mut Option<Subpath>, start_pt: &mut Option<PointPx>, c: PointPx, to: PointPx) {
    let Some(sub) = current.as_mut() else {
        *start_pt = Some(to);
        *current = Some(Subpath { points: vec![to], closed: false, has_segment: false });
        return;
    };
    sub.has_segment = true;
    if let Some(&p0) = sub.points.last() {
        flatten_quad(p0, c, to, 0, &mut sub.points);
    }
}

fn push_cubic_points(current: &mut Option<Subpath>, start_pt: &mut Option<PointPx>, c1: PointPx, c2: PointPx, to: PointPx) {
    let Some(sub) = current.as_mut() else {
        *start_pt = Some(to);
        *current = Some(Subpath { points: vec![to], closed: false, has_segment: false });
        return;
    };
    sub.has_segment = true;
    if let Some(&p0) = sub.points.last() {
        flatten_cubic(p0, c1, c2, to, 0, &mut sub.points);
    }
}

fn flatten_quad(p0: PointPx, p1: PointPx, p2: PointPx, depth: u8, out: &mut Vec<PointPx>) {
    let mid_x = 0.5 * (p0.x + p2.x);
    let mid_y = 0.5 * (p0.y + p2.y);
    let dx = p1.x - mid_x;
    let dy = p1.y - mid_y;
    let err_sq = dx * dx + dy * dy;
    let max_dev_sq = 4.0 * BEZIER_FLATNESS_TOL_PX * BEZIER_FLATNESS_TOL_PX;
    if depth >= MAX_BEZIER_DEPTH || err_sq <= max_dev_sq {
        if out.last().is_none_or(|prev| prev.dist_sq(p2) > 1e-18) {
            out.push(p2);
        }
        return;
    }
    let q0 = PointPx::new(0.5 * (p0.x + p1.x), 0.5 * (p0.y + p1.y));
    let q1 = PointPx::new(0.5 * (p1.x + p2.x), 0.5 * (p1.y + p2.y));
    let r = PointPx::new(0.5 * (q0.x + q1.x), 0.5 * (q0.y + q1.y));
    flatten_quad(p0, q0, r, depth + 1, out);
    flatten_quad(r, q1, p2, depth + 1, out);
}

fn flatten_cubic(p0: PointPx, p1: PointPx, p2: PointPx, p3: PointPx, depth: u8, out: &mut Vec<PointPx>) {
    let ux = 3.0 * p1.x - 2.0 * p0.x - p3.x;
    let uy = 3.0 * p1.y - 2.0 * p0.y - p3.y;
    let vx = 3.0 * p2.x - p0.x - 2.0 * p3.x;
    let vy = 3.0 * p2.y - p0.y - 2.0 * p3.y;
    let max_uv_sq = (ux * ux + uy * uy).max(vx * vx + vy * vy);
    let limit_sq = 16.0 * BEZIER_FLATNESS_TOL_PX * BEZIER_FLATNESS_TOL_PX;
    if depth >= MAX_BEZIER_DEPTH || max_uv_sq <= limit_sq {
        if out.last().is_none_or(|prev| prev.dist_sq(p3) > 1e-18) {
            out.push(p3);
        }
        return;
    }
    let q0 = PointPx::new(0.5 * (p0.x + p1.x), 0.5 * (p0.y + p1.y));
    let q1 = PointPx::new(0.5 * (p1.x + p2.x), 0.5 * (p1.y + p2.y));
    let q2 = PointPx::new(0.5 * (p2.x + p3.x), 0.5 * (p2.y + p3.y));
    let r0 = PointPx::new(0.5 * (q0.x + q1.x), 0.5 * (q0.y + q1.y));
    let r1 = PointPx::new(0.5 * (q1.x + q2.x), 0.5 * (q1.y + q2.y));
    let s = PointPx::new(0.5 * (r0.x + r1.x), 0.5 * (r0.y + r1.y));
    flatten_cubic(p0, q0, r0, s, depth + 1, out);
    flatten_cubic(s, r1, q2, p3, depth + 1, out);
}

#[derive(Debug, Clone, Copy)]
struct PolyEdge {
    a: PointPx,
    b: PointPx,
    y_min: f64,
    y_max: f64,
    dir: i32,
}

fn fill_subpaths(img: &mut RasterImage, subpaths: &[Subpath], color: Rgba8, clip: ClipRectPx) {
    let mut edges = Vec::new();
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for sub in subpaths {
        if sub.points.len() < 3 {
            continue;
        }
        for i in 0..sub.points.len() {
            let Some(&a) = sub.points.get(i) else { continue };
            let Some(&b) = sub.points.get((i + 1) % sub.points.len()) else { continue };
            min_x = min_x.min(a.x).min(b.x);
            max_x = max_x.max(a.x).max(b.x);
            min_y = min_y.min(a.y).min(b.y);
            max_y = max_y.max(a.y).max(b.y);
            if (a.y - b.y).abs() <= 1e-12 {
                continue;
            }
            let (y_min, y_max, dir) = if a.y < b.y { (a.y, b.y, 1) } else { (b.y, a.y, -1) };
            edges.push(PolyEdge { a, b, y_min, y_max, dir });
        }
    }

    if edges.is_empty() {
        return;
    }

    let Some((x_start, x_end, y_start, y_end)) = clipped_pixel_bounds(min_x, min_y, max_x, max_y, clip, img.width_px, img.height_px) else {
        return;
    };
    let row_len = (x_end - x_start) as usize;
    let mut row_cov = vec![0u8; row_len];
    let mut crossings: Vec<(f64, i32)> = Vec::with_capacity(16);

    edges.sort_by(|e0, e1| e0.y_min.total_cmp(&e1.y_min));
    let mut active: Vec<usize> = Vec::with_capacity(32);
    let mut next_edge = 0usize;

    for py in y_start..y_end {
        let row_top = f64::from(py);
        let row_bot = row_top + 1.0;
        while let Some(edge) = edges.get(next_edge) {
            if edge.y_min > row_bot {
                break;
            }
            active.push(next_edge);
            next_edge += 1;
        }
        active.retain(|&idx| edges.get(idx).is_some_and(|e| e.y_max >= row_top));
        if active.is_empty() {
            continue;
        }

        row_cov.fill(0);
        for &dy_off in &SUBPIXEL_OFFSETS {
            let y_sub = row_top + dy_off;
            if y_sub < clip.min_y || y_sub >= clip.max_y {
                continue;
            }
            collect_scanline_crossings(&edges, &active, y_sub, &mut crossings);
            accumulate_winding_spans(&crossings, x_start, x_end, clip, &mut row_cov);
        }

        for (offset, &hits) in row_cov.iter().enumerate() {
            if hits > 0 {
                img.blend_pixel_coverage(x_start + (offset as u32), py, color, f64::from(hits) / 16.0);
            }
        }
    }
}

fn collect_scanline_crossings(edges: &[PolyEdge], active: &[usize], y_sub: f64, out: &mut Vec<(f64, i32)>) {
    out.clear();
    for &idx in active {
        let Some(edge) = edges.get(idx) else { continue };
        if edge.y_min <= y_sub && y_sub < edge.y_max {
            let denom = edge.b.y - edge.a.y;
            if denom.abs() > 1e-12 {
                let t = (y_sub - edge.a.y) / denom;
                let x_cross = (1.0 - t) * edge.a.x + t * edge.b.x;
                if x_cross.is_finite() {
                    out.push((x_cross, edge.dir));
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
}

fn accumulate_winding_spans(crossings: &[(f64, i32)], x_start: u32, x_end: u32, clip: ClipRectPx, row_cov: &mut [u8]) {
    let mut winding = 0i32;
    let mut span_start = 0.0f64;

    for &(x_cross, dir) in crossings {
        let prev = winding;
        winding += dir;
        if prev == 0 && winding != 0 {
            span_start = x_cross;
        } else if prev != 0 && winding == 0 {
            fill_interval_subpixels(span_start, x_cross, x_start, x_end, clip, row_cov);
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fill_interval_subpixels(x0: f64, x1: f64, x_start: u32, x_end: u32, clip: ClipRectPx, row_cov: &mut [u8]) {
    let lo = x0.max(clip.min_x);
    let hi = x1.min(clip.max_x);
    if hi <= lo {
        return;
    }
    let p_start = (lo.floor().max(f64::from(x_start)) as u32).min(x_end);
    let p_end = (hi.ceil().min(f64::from(x_end)) as u32).max(p_start);
    for px in p_start..p_end {
        let idx = (px - x_start) as usize;
        let Some(slot) = row_cov.get_mut(idx) else { continue };
        let px_f = f64::from(px);
        if lo <= px_f + 0.125 && px_f + 0.875 < hi {
            *slot = slot.saturating_add(4);
        } else {
            let mut hits = 0u8;
            for &dx_off in &SUBPIXEL_OFFSETS {
                let x_sub = px_f + dx_off;
                if lo <= x_sub && x_sub < hi {
                    hits += 1;
                }
            }
            *slot = slot.saturating_add(hits);
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn clipped_pixel_bounds(
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    clip: ClipRectPx,
    width_px: u32,
    height_px: u32,
) -> Option<(u32, u32, u32, u32)> {
    let x0 = min_x.max(clip.min_x).max(0.0);
    let y0 = min_y.max(clip.min_y).max(0.0);
    let x1 = max_x.min(clip.max_x).min(f64::from(width_px));
    let y1 = max_y.min(clip.max_y).min(f64::from(height_px));
    if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() || x1 <= x0 || y1 <= y0 {
        return None;
    }
    let xs = (x0.floor() as u32).min(width_px);
    let ys = (y0.floor() as u32).min(height_px);
    let xe = (x1.ceil() as u32).clamp(xs, width_px);
    let ye = (y1.ceil() as u32).clamp(ys, height_px);
    if xe <= xs || ye <= ys { None } else { Some((xs, xe, ys, ye)) }
}

enum StrokePieceKind {
    ConvexPoly { verts: [PointPx; 4], len: usize },
    Disk { cx: f64, cy: f64, r: f64 },
}

struct StrokePiece {
    kind: StrokePieceKind,
    y_min: f64,
    y_max: f64,
    x_min: f64,
    x_max: f64,
}

impl StrokePiece {
    fn poly(pts: &[PointPx]) -> Option<Self> {
        if pts.len() < 3 || pts.len() > 4 {
            return None;
        }
        let mut verts = [PointPx::new(0.0, 0.0); 4];
        let mut x_min = f64::INFINITY;
        let mut x_max = f64::NEG_INFINITY;
        let mut y_min = f64::INFINITY;
        let mut y_max = f64::NEG_INFINITY;
        for (slot, &pt) in verts.iter_mut().zip(pts.iter()) {
            *slot = pt;
            x_min = x_min.min(pt.x);
            x_max = x_max.max(pt.x);
            y_min = y_min.min(pt.y);
            y_max = y_max.max(pt.y);
        }
        Some(Self { kind: StrokePieceKind::ConvexPoly { verts, len: pts.len() }, y_min, y_max, x_min, x_max })
    }

    fn disk(cx: f64, cy: f64, r: f64) -> Self {
        Self { kind: StrokePieceKind::Disk { cx, cy, r }, y_min: cy - r, y_max: cy + r, x_min: cx - r, x_max: cx + r }
    }

    fn x_interval_at(&self, y_sub: f64) -> Option<(f64, f64)> {
        if y_sub < self.y_min || y_sub > self.y_max {
            return None;
        }
        match self.kind {
            StrokePieceKind::Disk { cx, cy, r } => {
                let dy = y_sub - cy;
                let rem = r * r - dy * dy;
                if rem <= 0.0 {
                    None
                } else {
                    let dx = rem.sqrt();
                    Some((cx - dx, cx + dx))
                }
            }
            StrokePieceKind::ConvexPoly { verts, len } => convex_poly_interval(&verts[..len], y_sub),
        }
    }
}

fn convex_poly_interval(verts: &[PointPx], y_sub: f64) -> Option<(f64, f64)> {
    let mut x_lo = f64::INFINITY;
    let mut x_hi = f64::NEG_INFINITY;
    let n = verts.len();
    for i in 0..n {
        let Some(&a) = verts.get(i) else { continue };
        let Some(&b) = verts.get((i + 1) % n) else { continue };
        let crosses = (a.y <= y_sub && b.y >= y_sub) || (b.y <= y_sub && a.y >= y_sub);
        if !crosses {
            continue;
        }
        let dy = b.y - a.y;
        if dy.abs() <= 1e-12 {
            if (a.y - y_sub).abs() <= 1e-12 {
                x_lo = x_lo.min(a.x).min(b.x);
                x_hi = x_hi.max(a.x).max(b.x);
            }
        } else {
            let t = (y_sub - a.y) / dy;
            let x = (1.0 - t) * a.x + t * b.x;
            x_lo = x_lo.min(x);
            x_hi = x_hi.max(x);
        }
    }
    if x_lo.is_finite() && x_hi.is_finite() && x_hi > x_lo { Some((x_lo, x_hi)) } else { None }
}

fn stroke_subpaths(img: &mut RasterImage, subpaths: &[Subpath], stroke: &StrokePaint, stroke_scale: f64, clip: ClipRectPx) {
    let width_px = (stroke.width_pt * stroke_scale).max(0.5);
    let radius = 0.5 * width_px;
    let mut pieces: Vec<StrokePiece> = Vec::new();

    for sub in subpaths {
        let clean = normalize_subpath_points(&sub.points, sub.closed);
        if clean.is_empty() {
            continue;
        }
        if stroke.dash == LineDashStyle::Solid {
            emit_solid_subpath_pieces(&clean, sub.closed, radius, stroke.cap, stroke.join, &mut pieces);
        } else {
            emit_dashed_subpath_pieces(&clean, sub.closed, width_px, radius, stroke, &mut pieces);
        }
    }

    if pieces.is_empty() {
        return;
    }

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in &pieces {
        min_x = min_x.min(p.x_min);
        max_x = max_x.max(p.x_max);
        min_y = min_y.min(p.y_min);
        max_y = max_y.max(p.y_max);
    }

    let Some((x_start, x_end, y_start, y_end)) = clipped_pixel_bounds(min_x, min_y, max_x, max_y, clip, img.width_px, img.height_px) else {
        return;
    };

    pieces.sort_by(|a, b| a.y_min.total_cmp(&b.y_min));
    let row_len = (x_end - x_start) as usize;
    let mut row_masks = vec![0u16; row_len];
    let mut active: Vec<usize> = Vec::with_capacity(32);
    let mut next_piece = 0usize;

    for py in y_start..y_end {
        let row_top = f64::from(py);
        let row_bot = row_top + 1.0;
        while let Some(piece) = pieces.get(next_piece) {
            if piece.y_min > row_bot {
                break;
            }
            active.push(next_piece);
            next_piece += 1;
        }
        active.retain(|&idx| pieces.get(idx).is_some_and(|p| p.y_max >= row_top));
        if active.is_empty() {
            continue;
        }

        row_masks.fill(0);
        for (ky, &dy_off) in SUBPIXEL_OFFSETS.iter().enumerate() {
            let y_sub = row_top + dy_off;
            if y_sub < clip.min_y || y_sub >= clip.max_y {
                continue;
            }
            for &idx in &active {
                if let Some((x_lo, x_hi)) = pieces.get(idx).and_then(|p| p.x_interval_at(y_sub)) {
                    set_stroke_row_bits(x_lo, x_hi, ky, x_start, x_end, clip, &mut row_masks);
                }
            }
        }

        for (offset, &mask) in row_masks.iter().enumerate() {
            let hits = mask.count_ones();
            if hits > 0 {
                img.blend_pixel_coverage(x_start + (offset as u32), py, stroke.color, f64::from(hits) / 16.0);
            }
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn set_stroke_row_bits(x0: f64, x1: f64, ky: usize, x_start: u32, x_end: u32, clip: ClipRectPx, row_masks: &mut [u16]) {
    let lo = x0.max(clip.min_x);
    let hi = x1.min(clip.max_x);
    if hi <= lo {
        return;
    }
    let shift = ky * 4;
    let p_start = (lo.floor().max(f64::from(x_start)) as u32).min(x_end);
    let p_end = (hi.ceil().min(f64::from(x_end)) as u32).max(p_start);
    for px in p_start..p_end {
        let idx = (px - x_start) as usize;
        let Some(slot) = row_masks.get_mut(idx) else { continue };
        let px_f = f64::from(px);
        if lo <= px_f + 0.125 && px_f + 0.875 < hi {
            *slot |= 0x0Fu16 << shift;
        } else {
            let mut nibble = 0u16;
            for (kx, &dx_off) in SUBPIXEL_OFFSETS.iter().enumerate() {
                let x_sub = px_f + dx_off;
                if lo <= x_sub && x_sub < hi {
                    nibble |= 1u16 << kx;
                }
            }
            *slot |= nibble << shift;
        }
    }
}

fn normalize_subpath_points(points: &[PointPx], closed: bool) -> Vec<PointPx> {
    let mut clean: Vec<PointPx> = Vec::with_capacity(points.len());
    for &p in points {
        if clean.last().is_none_or(|prev| prev.dist_sq(p) > 1e-18) {
            clean.push(p);
        }
    }
    if closed && clean.len() > 1 && clean.first().zip(clean.last()).is_some_and(|(a, b)| a.dist_sq(*b) <= 1e-18) {
        let _ = clean.pop();
    }
    clean
}

fn emit_solid_subpath_pieces(points: &[PointPx], closed: bool, r: f64, cap: LineCap, join: LineJoin, out: &mut Vec<StrokePiece>) {
    if points.len() == 1 {
        if let Some(&pt) = points.first() {
            emit_zero_length_cap(pt, r, cap, out);
        }
        return;
    }

    let seg_count = if closed { points.len() } else { points.len() - 1 };
    let mut dirs = Vec::with_capacity(seg_count);
    for i in 0..seg_count {
        let Some(&a) = points.get(i) else { continue };
        let Some(&b) = points.get((i + 1) % points.len()) else { continue };
        if let Some((dir, _len)) = unit_dir(a, b) {
            emit_segment_quad(a, b, dir, r, out);
            dirs.push(dir);
        }
    }

    if dirs.is_empty() {
        return;
    }

    let join_count = if closed { dirs.len() } else { dirs.len().saturating_sub(1) };
    for i in 0..join_count {
        let vertex_idx = if closed { (i + 1) % points.len() } else { i + 1 };
        let Some(&v) = points.get(vertex_idx) else { continue };
        let Some(&d0) = dirs.get(i) else { continue };
        let Some(&d1) = dirs.get((i + 1) % dirs.len()) else { continue };
        emit_join_piece(v, d0, d1, r, join, out);
    }

    if !closed && let (Some(&first), Some(&last), Some(&d_first), Some(&d_last)) = (points.first(), points.last(), dirs.first(), dirs.last()) {
        emit_endpoint_cap(first, PointPx::new(-d_first.x, -d_first.y), r, cap, out);
        emit_endpoint_cap(last, d_last, r, cap, out);
    }
}

fn emit_dashed_subpath_pieces(points: &[PointPx], closed: bool, width_px: f64, r: f64, stroke: &StrokePaint, out: &mut Vec<StrokePiece>) {
    let mults = stroke.dash.dash_multipliers();
    if mults.is_empty() || points.len() < 2 {
        emit_solid_subpath_pieces(points, closed, r, stroke.cap, stroke.join, out);
        return;
    }
    let pattern: Vec<f64> = mults.iter().map(|&m| (m * width_px).max(0.25)).collect();
    let mut pat_idx = 0usize;
    let mut pat_rem = pattern.first().copied().unwrap_or(width_px);
    let mut dash_pts: Vec<PointPx> = Vec::new();
    let seg_count = if closed { points.len() } else { points.len() - 1 };

    for i in 0..seg_count {
        let Some(&a) = points.get(i) else { continue };
        let Some(&b) = points.get((i + 1) % points.len()) else { continue };
        let Some((dir, seg_len)) = unit_dir(a, b) else { continue };
        let mut pos = 0.0f64;
        while pos < seg_len - 1e-9 {
            let take = (seg_len - pos).min(pat_rem);
            let next_pos = pos + take;
            let p_start = PointPx::new(a.x + dir.x * pos, a.y + dir.y * pos);
            let p_end = PointPx::new(a.x + dir.x * next_pos, a.y + dir.y * next_pos);
            if pat_idx.is_multiple_of(2) {
                if dash_pts.is_empty() {
                    dash_pts.push(p_start);
                }
                dash_pts.push(p_end);
            }
            pat_rem -= take;
            pos = next_pos;
            if pat_rem <= 1e-9 {
                if pat_idx.is_multiple_of(2) && !dash_pts.is_empty() {
                    emit_solid_subpath_pieces(&dash_pts, false, r, stroke.cap, stroke.join, out);
                    dash_pts.clear();
                }
                pat_idx = (pat_idx + 1) % pattern.len();
                pat_rem = pattern.get(pat_idx).copied().unwrap_or(width_px);
            }
        }
    }

    if !dash_pts.is_empty() {
        emit_solid_subpath_pieces(&dash_pts, false, r, stroke.cap, stroke.join, out);
    }
}

fn unit_dir(a: PointPx, b: PointPx) -> Option<(PointPx, f64)> {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = dx.hypot(dy);
    if !len.is_finite() || len <= 1e-9 { None } else { Some((PointPx::new(dx / len, dy / len), len)) }
}

fn emit_segment_quad(a: PointPx, b: PointPx, dir: PointPx, r: f64, out: &mut Vec<StrokePiece>) {
    let nx = -dir.y * r;
    let ny = dir.x * r;
    let verts =
        [PointPx::new(a.x + nx, a.y + ny), PointPx::new(b.x + nx, b.y + ny), PointPx::new(b.x - nx, b.y - ny), PointPx::new(a.x - nx, a.y - ny)];
    if let Some(piece) = StrokePiece::poly(&verts) {
        out.push(piece);
    }
}

fn emit_endpoint_cap(pt: PointPx, out_dir: PointPx, r: f64, cap: LineCap, out: &mut Vec<StrokePiece>) {
    match cap {
        LineCap::Butt => {}
        LineCap::Round => out.push(StrokePiece::disk(pt.x, pt.y, r)),
        LineCap::Square => {
            let end = PointPx::new(pt.x + out_dir.x * r, pt.y + out_dir.y * r);
            emit_segment_quad(pt, end, out_dir, r, out);
        }
    }
}

fn emit_zero_length_cap(pt: PointPx, r: f64, cap: LineCap, out: &mut Vec<StrokePiece>) {
    match cap {
        LineCap::Butt => {}
        LineCap::Round => out.push(StrokePiece::disk(pt.x, pt.y, r)),
        LineCap::Square => {
            let verts = [
                PointPx::new(pt.x - r, pt.y - r),
                PointPx::new(pt.x + r, pt.y - r),
                PointPx::new(pt.x + r, pt.y + r),
                PointPx::new(pt.x - r, pt.y + r),
            ];
            if let Some(piece) = StrokePiece::poly(&verts) {
                out.push(piece);
            }
        }
    }
}

fn emit_join_piece(v: PointPx, d0: PointPx, d1: PointPx, r: f64, join: LineJoin, out: &mut Vec<StrokePiece>) {
    let cross = d0.x * d1.y - d0.y * d1.x;
    let dot = d0.x * d1.x + d0.y * d1.y;
    if cross.abs() <= 1e-9 && dot > 0.0 {
        return;
    }
    if join == LineJoin::Round {
        out.push(StrokePiece::disk(v.x, v.y, r));
        return;
    }
    if cross.abs() <= 1e-9 {
        return;
    }

    let sign = if cross > 0.0 { 1.0 } else { -1.0 };
    let n0 = PointPx::new(sign * d0.y, -sign * d0.x);
    let n1 = PointPx::new(sign * d1.y, -sign * d1.x);
    let p_a = PointPx::new(v.x + r * n0.x, v.y + r * n0.y);
    let p_b = PointPx::new(v.x + r * n1.x, v.y + r * n1.y);

    if join == LineJoin::Miter {
        let bx = n0.x + n1.x;
        let by = n0.y + n1.y;
        let b_len_sq = bx * bx + by * by;
        // L / r = 2 / sqrt(b_len_sq); L / r <= MITER_LIMIT <=> b_len_sq >= (2 / MITER_LIMIT)^2
        let min_b_sq = (2.0 / MITER_LIMIT) * (2.0 / MITER_LIMIT);
        if b_len_sq >= min_b_sq {
            let scale = (2.0 * r) / b_len_sq;
            let miter = PointPx::new(v.x + scale * bx, v.y + scale * by);
            if let Some(piece) = StrokePiece::poly(&[v, p_a, miter, p_b]) {
                out.push(piece);
            }
            return;
        }
    }

    if let Some(piece) = StrokePiece::poly(&[v, p_a, p_b]) {
        out.push(piece);
    }
}

#[derive(Debug, Clone, Copy)]
struct TextTransform {
    anchor_dx: f64,
    anchor_x: f64,
    anchor_y: f64,
    cos_a: f64,
    sin_a: f64,
    sx: f64,
    sy: f64,
}

impl TextTransform {
    fn map_pt(self, x_pt: f64, y_pt: f64) -> PointPx {
        let lx = x_pt + self.anchor_dx;
        let rx = self.anchor_x + lx * self.cos_a - y_pt * self.sin_a;
        let ry = self.anchor_y + lx * self.sin_a + y_pt * self.cos_a;
        PointPx::new(rx * self.sx, ry * self.sy)
    }
}

fn render_text(img: &mut RasterImage, text: &PositionedText, sx: f64, sy: f64, clip: ClipRectPx) {
    if !text.font_size_pt.is_finite() || text.font_size_pt <= 0.0 || !text.anchor_pt.x.is_finite() || !text.anchor_pt.y.is_finite() {
        return;
    }
    let outlines = outline_rich_text(&text.rich, 0.0, 0.0, text.font_size_pt);
    if outlines.is_empty() {
        return;
    }

    let total_advance: f64 = outlines.iter().map(|g| if g.advance_x_pt.is_finite() { g.advance_x_pt } else { 0.0 }).sum();
    let anchor_dx = match text.anchor {
        TextAnchor::Start => 0.0,
        TextAnchor::Middle => -0.5 * total_advance,
        TextAnchor::End => -total_advance,
    };
    let rad = if text.rotation_deg.is_finite() { text.rotation_deg.to_radians() } else { 0.0 };
    let xform = TextTransform { anchor_dx, anchor_x: text.anchor_pt.x, anchor_y: text.anchor_pt.y, cos_a: rad.cos(), sin_a: rad.sin(), sx, sy };

    for glyph in &outlines {
        if glyph.commands.is_empty() {
            continue;
        }
        let color = glyph.color.unwrap_or(text.color);
        if color.a == 0 {
            continue;
        }
        let subpaths = flatten_glyph_commands(&glyph.commands, xform);
        if !subpaths.is_empty() {
            fill_subpaths(img, &subpaths, color, clip);
        }
    }
}

fn flatten_glyph_commands(cmds: &[GlyphOutlineCmd], xform: TextTransform) -> Vec<Subpath> {
    let mut subpaths = Vec::new();
    let mut current: Option<Subpath> = None;
    let mut start_pt: Option<PointPx> = None;

    for cmd in cmds {
        match *cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                if let Some(sub) = current.take()
                    && !sub.points.is_empty()
                {
                    subpaths.push(sub);
                }
                let p = xform.map_pt(x_pt, y_pt);
                if p.is_finite() {
                    start_pt = Some(p);
                    current = Some(Subpath { points: vec![p], closed: true, has_segment: false });
                } else {
                    start_pt = None;
                }
            }
            GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                let p = xform.map_pt(x_pt, y_pt);
                if p.is_finite() {
                    push_line_point(&mut current, &mut start_pt, p);
                }
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                let c = xform.map_pt(cx_pt, cy_pt);
                let p = xform.map_pt(x_pt, y_pt);
                if c.is_finite() && p.is_finite() {
                    push_quad_points(&mut current, &mut start_pt, c, p);
                }
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                let c1 = xform.map_pt(c1x_pt, c1y_pt);
                let c2 = xform.map_pt(c2x_pt, c2y_pt);
                let p = xform.map_pt(x_pt, y_pt);
                if c1.is_finite() && c2.is_finite() && p.is_finite() {
                    push_cubic_points(&mut current, &mut start_pt, c1, c2, p);
                }
            }
            GlyphOutlineCmd::Close => {
                if let Some(mut sub) = current.take() {
                    sub.closed = true;
                    if !sub.points.is_empty() {
                        subpaths.push(sub);
                    }
                }
                if let Some(st) = start_pt {
                    current = Some(Subpath { points: vec![st], closed: true, has_segment: false });
                }
            }
        }
    }

    if let Some(sub) = current
        && sub.points.len() > 2
    {
        subpaths.push(sub);
    }

    subpaths
}
