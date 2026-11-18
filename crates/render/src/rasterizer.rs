//! Deterministic software rasterizer converting [`curvebook_chart::ChartScene`] into RGBA pixels.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};

use curvebook_chart::{ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PositionedText, RectPt, ScenePrimitive, StrokePaint, TextAnchor};
use curvebook_palette::{CanvasPreviewFilter, LineDashStyle, Rgba8, simulate_preview_filter};
use curvebook_text::metrics::split_rich_text_lines;
use curvebook_text::{GlyphOutlineCmd, outline_rich_text};
use serde::{Deserialize, Serialize};

use crate::RenderTargetSpec;

/// Maximum supported raster width or height in pixels (`16384 x 16384`).
pub const MAX_RASTER_DIM_PX: u32 = 16_384;

/// Maximum supported total raster pixel budget (`8192 x 8192` = 67,108,864 pixels).
pub const MAX_RASTER_PIXELS: u64 = 67_108_864;

/// Alias for uncompressed RGBA pixel buffers compatible with canvas preview filters.
pub type RgbaPixmap = RasterImage;

const BEZIER_FLATNESS_TOL_PX: f64 = 0.15;
const MAX_BEZIER_DEPTH: u8 = 12;
const MITER_LIMIT: f64 = 4.0;
const MAX_DASH_SEGMENTS: usize = 65_536;
const SUBPIXEL_OFFSETS: [f64; 4] = [0.125, 0.375, 0.625, 0.875];
const CIRCLE_KAPPA: f64 = 0.552_284_749_830_793_6;
const STAMP_PHASE_STEPS: f64 = 8.0;
const STAMP_DIM_STEPS: f64 = 64.0;
const MAX_STAMP_RADIUS_PX: f64 = 24.0;
const MAX_SMALL_POLY_EXTENT_PX: f64 = 32.0;
const MAX_CACHED_ELLIPSE_MASKS: usize = 256;
const MAX_CACHED_POLY_MASKS: usize = 512;
const MAX_CACHED_BAKED_STAMPS: usize = 1_024;

/// Uncompressed RGBA pixel buffer paired with resolution and physical DPI metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterImage {
    pub width_px: u32,
    pub height_px: u32,
    pub dpi: u16,
    pub rgba: Vec<u8>,
}

/// Validate target raster dimensions, DPI, [`MAX_RASTER_DIM_PX`], and [`MAX_RASTER_PIXELS`],
/// returning total pixel count.
pub(crate) fn validate_raster_target_dims(width_px: u32, height_px: u32, dpi: u16, label: &str) -> Result<u64, RenderError> {
    if width_px == 0 || height_px == 0 || dpi == 0 {
        return Err(RenderError::InvalidTarget { message: format!("{label} width, height, and dpi must be positive") });
    }
    if width_px > MAX_RASTER_DIM_PX || height_px > MAX_RASTER_DIM_PX {
        return Err(RenderError::InvalidTarget {
            message: format!("{label} dimensions {width_px}x{height_px} exceed maximum {MAX_RASTER_DIM_PX}x{MAX_RASTER_DIM_PX}"),
        });
    }
    let total_pixels = u64::from(width_px).saturating_mul(u64::from(height_px));
    if total_pixels > MAX_RASTER_PIXELS {
        return Err(RenderError::InvalidTarget {
            message: format!("{label} pixel count {total_pixels} ({width_px}x{height_px}) exceeds budget {MAX_RASTER_PIXELS}"),
        });
    }
    Ok(total_pixels)
}

impl RenderTargetSpec {
    /// Resolve uniform pixel dimensions for `scene` at `dpi` without independently clamping axes.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidTarget`] when `dpi == 0`, scene bounds are non-positive or
    /// non-finite, any scaled dimension rounds below `1` pixel, or uniform dimensions exceed
    /// [`MAX_RASTER_DIM_PX`] or [`MAX_RASTER_PIXELS`].
    pub fn from_scene_dpi(scene: &ChartScene, dpi: u16) -> Result<Self, RenderError> {
        if dpi == 0 {
            return Err(RenderError::InvalidTarget { message: String::from("raster DPI must be positive") });
        }
        if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
            return Err(RenderError::InvalidTarget { message: String::from("scene dimensions must be finite and positive") });
        }
        let scale = f64::from(dpi) / 72.0;
        let w_round = (scene.width_pt * scale).round();
        let h_round = (scene.height_pt * scale).round();
        if !w_round.is_finite() || !h_round.is_finite() {
            return Err(RenderError::InvalidTarget { message: format!("scaled raster dimensions at {dpi} DPI overflow numeric bounds") });
        }
        if w_round < 1.0 || h_round < 1.0 {
            return Err(RenderError::InvalidTarget {
                message: format!("scaled raster dimensions ({w_round}x{h_round}) at {dpi} DPI are smaller than 1 pixel"),
            });
        }
        let max_dim_f = f64::from(MAX_RASTER_DIM_PX);
        if w_round > max_dim_f || h_round > max_dim_f {
            return Err(RenderError::InvalidTarget {
                message: format!("uniform raster dimensions {w_round}x{h_round} at {dpi} DPI exceed maximum {MAX_RASTER_DIM_PX}x{MAX_RASTER_DIM_PX}"),
            });
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let width_px = w_round as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let height_px = h_round as u32;
        let _ = validate_raster_target_dims(width_px, height_px, dpi, "raster")?;
        Ok(Self { width_px, height_px, dpi })
    }
}

impl RasterImage {
    /// Allocate a pixel buffer validated against zero dimensions, max bounds, and size overflows.
    ///
    /// # Errors
    /// Returns [`RenderError::InvalidTarget`] on zero/oversized dimensions, zero DPI, or overflow.
    pub fn new(width_px: u32, height_px: u32, dpi: u16, fill: Rgba8) -> Result<Self, RenderError> {
        let total_pixels = validate_raster_target_dims(width_px, height_px, dpi, "raster")?;
        let pixel_count = usize::try_from(total_pixels)
            .map_err(|_| RenderError::InvalidTarget { message: String::from("raster dimensions overflow addressable pixel count") })?;
        let byte_len =
            pixel_count.checked_mul(4).ok_or_else(|| RenderError::InvalidTarget { message: String::from("raster byte buffer size overflow") })?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(byte_len).map_err(|err| RenderError::InvalidTarget {
            message: format!("failed to reserve {byte_len} bytes for raster image ({width_px}x{height_px}): {err}"),
        })?;
        if fill.r == fill.g && fill.g == fill.b && fill.b == fill.a {
            rgba.resize(byte_len, fill.r);
        } else {
            rgba.resize(byte_len, 0);
            let fill_px = [fill.r, fill.g, fill.b, fill.a];
            for px in rgba.as_chunks_mut::<4>().0 {
                *px = fill_px;
            }
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
        let idx = ((y as usize) * (self.width_px as usize) + (x as usize)) * 4;
        let Some(px) = self.rgba.get_mut(idx..idx + 4) else {
            return;
        };
        blend_pixel_slice(px, src, coverage.clamp(0.0, 1.0));
    }
}

#[inline]
fn blend_pixel_slice(px: &mut [u8], src: Rgba8, cov: f64) {
    if px.len() < 4 || src.a == 0 || cov <= 0.0 {
        return;
    }
    if src.a == u8::MAX && cov >= 1.0 {
        px[0] = src.r;
        px[1] = src.g;
        px[2] = src.b;
        px[3] = u8::MAX;
        return;
    }

    let sa = (f64::from(src.a) / 255.0) * cov.min(1.0);
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

/// Errors produced by CPU rasterization or PNG/TIFF encoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderError {
    InvalidTarget { message: String },
    BufferSizeMismatch { expected: usize, actual: usize },
    EncodingFailed { message: String },
}

/// Apply a canvas CVD or grayscale print simulation filter across every pixel of `pixmap`.
pub fn apply_canvas_preview_filter(pixmap: &mut RgbaPixmap, filter: CanvasPreviewFilter) {
    for [r, g, b, a] in pixmap.rgba.as_chunks_mut::<4>().0 {
        let filtered = simulate_preview_filter(Rgba8 { r: *r, g: *g, b: *b, a: *a }, filter);
        *r = filtered.r;
        *g = filtered.g;
        *b = filtered.b;
        *a = filtered.a;
    }
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
    let canvas_opaque = initial_bg.a == u8::MAX;
    let skip_first_bg = canvas_opaque && is_matching_full_bg_rect(scene, initial_bg);
    let mut img = RasterImage::new(target.width_px, target.height_px, target.dpi, initial_bg)?;
    let canvas_clip = ClipRectPx::from_canvas(target.width_px, target.height_px);
    let mut clip_stack = vec![canvas_clip];
    let mut scratch = RasterScratch::new(canvas_opaque);

    let start_idx = usize::from(skip_first_bg);
    for prim in scene.primitives.iter().skip(start_idx) {
        let active_clip = clip_stack.last().copied().unwrap_or(canvas_clip);
        match prim {
            ScenePrimitive::Path { path, fill, stroke, clip } => {
                let effective_clip = match clip.and_then(|r| ClipRectPx::from_rect_pt(r, sx, sy)) {
                    Some(local) => active_clip.intersect(local),
                    None if clip.is_some() => ClipRectPx::empty(),
                    None => active_clip,
                };
                if !effective_clip.is_empty() && !path.is_empty() {
                    let env = PathStyleEnv { sx, sy, stroke_scale, clip: effective_clip };
                    rasterize_path(&mut img, path, *fill, *stroke, env, &mut scratch);
                }
            }
            ScenePrimitive::Text(text) => {
                if !active_clip.is_empty() {
                    render_text(&mut img, text, sx, sy, active_clip, &mut scratch);
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

fn is_matching_full_bg_rect(scene: &ChartScene, bg: Rgba8) -> bool {
    let expected_bg_path = Path2D::rect(RectPt::new(0.0, 0.0, scene.width_pt, scene.height_pt));
    matches!(
        scene.primitives.first(),
        Some(ScenePrimitive::Path { path, fill: Some(fill), stroke: None, clip: None })
            if fill.color == bg && *path == expected_bg_path
    )
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
    if is_matching_full_bg_rect(scene, bg) {
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
        !self.min_x.is_finite()
            || !self.max_x.is_finite()
            || !self.min_y.is_finite()
            || !self.max_y.is_finite()
            || self.max_x <= self.min_x
            || self.max_y <= self.min_y
    }

    fn expand(self, margin: f64) -> Self {
        let m = if margin.is_finite() { margin.max(0.0) } else { 0.0 };
        Self { min_x: self.min_x - m, min_y: self.min_y - m, max_x: self.max_x + m, max_y: self.max_y + m }
    }
}

struct Subpath {
    points: Vec<PointPx>,
    closed: bool,
    has_segment: bool,
}

#[derive(Default)]
struct FastHasher(u64);

impl Hasher for FastHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.write_u64(u64::from(i));
    }

    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.write_u64(u64::from(i.cast_unsigned()));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.write_u64(u64::from(i));
    }

    #[inline]
    fn write_i16(&mut self, i: i16) {
        self.write_u64(u64::from(i.cast_unsigned()));
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.write_u64(u64::from(i));
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64);
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EllipseMaskKey {
    rx_q: u32,
    ry_q: u32,
    sr_q: u32,
    qx: u8,
    qy: u8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct SmallPolyCmdQ {
    op: u8,
    dx: i16,
    dy: i16,
}

#[derive(Clone, Copy)]
struct SmallPolyKey {
    len: u8,
    cmds: [SmallPolyCmdQ; 24],
    qx: u8,
    qy: u8,
    sr_q: u16,
    cap: u8,
    join: u8,
}

impl PartialEq for SmallPolyKey {
    fn eq(&self, other: &Self) -> bool {
        let len = usize::from(self.len);
        self.len == other.len
            && self.qx == other.qx
            && self.qy == other.qy
            && self.sr_q == other.sr_q
            && self.cap == other.cap
            && self.join == other.join
            && self.cmds.get(..len) == other.cmds.get(..len)
    }
}

impl Eq for SmallPolyKey {}

impl Hash for SmallPolyKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len.hash(state);
        self.qx.hash(state);
        self.qy.hash(state);
        self.sr_q.hash(state);
        self.cap.hash(state);
        self.join.hash(state);
        if let Some(slice) = self.cmds.get(..usize::from(self.len)) {
            slice.hash(state);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct BakedStampKey {
    mask_id: usize,
    fill: Option<[u8; 4]>,
    stroke: Option<[u8; 4]>,
}

#[derive(Clone, Copy)]
struct MaskRowSpan {
    x_off: u16,
    len: u16,
    offset: u32,
}

struct CoverageMask {
    min_dx: i32,
    min_dy: i32,
    rows: Vec<MaskRowSpan>,
    cells: Vec<[u8; 2]>,
}

#[derive(Clone, Copy)]
enum BakedPixel {
    Empty,
    Opaque([u8; 4]),
    Blend { straight: [u8; 4], pr: f32, pg: f32, pb: f32, sa: f32, inv_sa: f32 },
}

struct BakedStamp {
    min_dx: i32,
    min_dy: i32,
    rows: Vec<MaskRowSpan>,
    pixels: Vec<BakedPixel>,
}

#[derive(Default)]
struct MarkerStampCache {
    ellipse_masks: FastMap<EllipseMaskKey, usize>,
    poly_masks: FastMap<SmallPolyKey, usize>,
    masks: Vec<CoverageMask>,
    baked: FastMap<BakedStampKey, BakedStamp>,
}

#[derive(Default)]
struct ScanScratch {
    edges: Vec<PolyEdge>,
    row_cov: Vec<u8>,
    crossings: Vec<(f64, i32)>,
    active: Vec<usize>,
    clean_points: Vec<PointPx>,
    dirs: Vec<PointPx>,
    pieces: Vec<StrokePiece>,
    row_masks: Vec<u16>,
    dash_pattern: Vec<f64>,
    dash_pts: Vec<PointPx>,
}

struct RasterScratch {
    canvas_opaque: bool,
    subpaths: Vec<Subpath>,
    vec_pool: Vec<Vec<PointPx>>,
    scan: ScanScratch,
    stamps: MarkerStampCache,
}

impl RasterScratch {
    fn new(canvas_opaque: bool) -> Self {
        Self {
            canvas_opaque,
            subpaths: Vec::with_capacity(8),
            vec_pool: Vec::with_capacity(8),
            scan: ScanScratch::default(),
            stamps: MarkerStampCache::default(),
        }
    }
}

enum ScanTarget<'a> {
    Image { img: &'a mut RasterImage, color: Rgba8 },
    FillMask { min_dx: i32, min_dy: i32, width: u32, cells: &'a mut [[u8; 2]] },
    StrokeMask { min_dx: i32, min_dy: i32, width: u32, cells: &'a mut [[u8; 2]] },
}

impl ScanTarget<'_> {
    fn bounds_px(&self) -> (u32, u32) {
        match self {
            Self::Image { img, .. } => (img.width_px, img.height_px),
            Self::FillMask { width, cells, .. } | Self::StrokeMask { width, cells, .. } => {
                let w = (*width).max(1);
                let h = u32::try_from(cells.len() / (w as usize)).unwrap_or(0);
                (w, h)
            }
        }
    }

    fn write_row(&mut self, py: u32, x_start: u32, row_cov: &[u8]) {
        match self {
            Self::Image { img, color } => {
                let Some(row_start) =
                    (py as usize).checked_mul(img.width_px as usize).and_then(|r| r.checked_add(x_start as usize)).and_then(|p| p.checked_mul(4))
                else {
                    return;
                };
                let color = *color;
                for (offset, &hits) in row_cov.iter().enumerate() {
                    if hits > 0 {
                        let idx = row_start + offset * 4;
                        if let Some(px) = img.rgba.get_mut(idx..idx + 4) {
                            blend_pixel_slice(px, color, f64::from(hits) / 16.0);
                        }
                    }
                }
            }
            Self::FillMask { min_dx, min_dy, width, cells } => {
                let mut dst = MaskRowTarget { min_dx: *min_dx, min_dy: *min_dy, width: *width, cells, channel: 0 };
                write_mask_row(py, x_start, row_cov, &mut dst);
            }
            Self::StrokeMask { min_dx, min_dy, width, cells } => {
                let mut dst = MaskRowTarget { min_dx: *min_dx, min_dy: *min_dy, width: *width, cells, channel: 1 };
                write_mask_row(py, x_start, row_cov, &mut dst);
            }
        }
    }

    fn write_stroke_row(&mut self, py: u32, x_start: u32, row_masks: &[u16]) {
        match self {
            Self::Image { img, color } => {
                let Some(row_start) =
                    (py as usize).checked_mul(img.width_px as usize).and_then(|r| r.checked_add(x_start as usize)).and_then(|p| p.checked_mul(4))
                else {
                    return;
                };
                let color = *color;
                for (offset, &mask) in row_masks.iter().enumerate() {
                    let hits = mask.count_ones();
                    if hits > 0 {
                        let idx = row_start + offset * 4;
                        if let Some(px) = img.rgba.get_mut(idx..idx + 4) {
                            blend_pixel_slice(px, color, f64::from(hits) / 16.0);
                        }
                    }
                }
            }
            Self::FillMask { min_dx, min_dy, width, cells } => {
                let mut dst = MaskRowTarget { min_dx: *min_dx, min_dy: *min_dy, width: *width, cells, channel: 0 };
                write_stroke_mask_row(py, x_start, row_masks, &mut dst);
            }
            Self::StrokeMask { min_dx, min_dy, width, cells } => {
                let mut dst = MaskRowTarget { min_dx: *min_dx, min_dy: *min_dy, width: *width, cells, channel: 1 };
                write_stroke_mask_row(py, x_start, row_masks, &mut dst);
            }
        }
    }
}

struct MaskRowTarget<'a> {
    min_dx: i32,
    min_dy: i32,
    width: u32,
    cells: &'a mut [[u8; 2]],
    channel: usize,
}

fn write_mask_row(py: u32, x_start: u32, row_cov: &[u8], dst: &mut MaskRowTarget<'_>) {
    let Ok(ly) = i32::try_from(py).map(|v| v - dst.min_dy) else { return };
    if ly < 0 {
        return;
    }
    let row_base = (ly as usize) * (dst.width as usize);
    for (offset, &hits) in row_cov.iter().enumerate() {
        if hits == 0 {
            continue;
        }
        let Ok(lx) = i32::try_from(x_start + (offset as u32)).map(|v| v - dst.min_dx) else { continue };
        if lx >= 0
            && (lx as u32) < dst.width
            && let Some(cell) = dst.cells.get_mut(row_base + (lx as usize))
        {
            cell[dst.channel] = hits.min(16);
        }
    }
}

fn write_stroke_mask_row(py: u32, x_start: u32, row_masks: &[u16], dst: &mut MaskRowTarget<'_>) {
    let Ok(ly) = i32::try_from(py).map(|v| v - dst.min_dy) else { return };
    if ly < 0 {
        return;
    }
    let row_base = (ly as usize) * (dst.width as usize);
    for (offset, &mask) in row_masks.iter().enumerate() {
        let hits = mask.count_ones();
        if hits == 0 {
            continue;
        }
        let Ok(lx) = i32::try_from(x_start + (offset as u32)).map(|v| v - dst.min_dx) else { continue };
        if lx >= 0
            && (lx as u32) < dst.width
            && let Some(cell) = dst.cells.get_mut(row_base + (lx as usize))
        {
            cell[dst.channel] = (hits as u8).min(16);
        }
    }
}

#[inline]
fn take_point_vec(pool: &mut Vec<Vec<PointPx>>, first: PointPx) -> Vec<PointPx> {
    let mut v = pool.pop().unwrap_or_else(|| Vec::with_capacity(16));
    v.clear();
    v.push(first);
    v
}

#[inline]
fn reclaim_subpaths(subpaths: &mut Vec<Subpath>, pool: &mut Vec<Vec<PointPx>>) {
    for mut sub in subpaths.drain(..) {
        if pool.len() < 16 {
            sub.points.clear();
            pool.push(sub.points);
        }
    }
}

#[derive(Clone, Copy)]
struct EllipsePx {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
}

#[inline]
fn unit_axis_delta(pt: PointPx, cx: f64, cy: f64, rx: f64, ry: f64) -> Option<(f64, f64)> {
    let ux = (pt.x - cx) / rx;
    let uy = (pt.y - cy) / ry;
    let horiz = (ux.abs() - 1.0).abs() <= 1e-3 && uy.abs() <= 1e-3;
    let vert = (uy.abs() - 1.0).abs() <= 1e-3 && ux.abs() <= 1e-3;
    if horiz {
        Some((ux.signum(), 0.0))
    } else if vert {
        Some((0.0, uy.signum()))
    } else {
        None
    }
}

fn try_extract_axis_aligned_ellipse(path: &Path2D, sx: f64, sy: f64) -> Option<EllipsePx> {
    let [
        PathCmd::MoveTo(raw_p0),
        PathCmd::CubicTo { c1: raw_a1, c2: raw_a2, to: raw_p1 },
        PathCmd::CubicTo { c1: raw_b1, c2: raw_b2, to: raw_p2 },
        PathCmd::CubicTo { c1: raw_c1, c2: raw_c2, to: raw_p3 },
        PathCmd::CubicTo { c1: raw_d1, c2: raw_d2, to: raw_p4 },
        PathCmd::Close,
    ] = path.commands.as_slice()
    else {
        return None;
    };

    let scale = |pt: curvebook_chart::PointPt| PointPx::new(pt.x * sx, pt.y * sy);
    let (p0, p1, p2, p3, p4) = (scale(*raw_p0), scale(*raw_p1), scale(*raw_p2), scale(*raw_p3), scale(*raw_p4));
    if !p0.is_finite() || !p1.is_finite() || !p2.is_finite() || !p3.is_finite() || !p4.is_finite() {
        return None;
    }
    if (p4.x - p0.x).abs() > 1e-3 || (p4.y - p0.y).abs() > 1e-3 {
        return None;
    }

    let cx = 0.5 * (p0.x + p2.x);
    let cy = 0.5 * (p0.y + p2.y);
    if (0.5 * (p1.x + p3.x) - cx).abs() > 1e-3 || (0.5 * (p1.y + p3.y) - cy).abs() > 1e-3 {
        return None;
    }

    let (rx, ry) = if (p0.y - cy).abs() <= 1e-3 && (p1.x - cx).abs() <= 1e-3 {
        ((p0.x - cx).abs(), (p1.y - cy).abs())
    } else if (p0.x - cx).abs() <= 1e-3 && (p1.y - cy).abs() <= 1e-3 {
        ((p1.x - cx).abs(), (p0.y - cy).abs())
    } else {
        return None;
    };
    if !rx.is_finite() || !ry.is_finite() || rx <= 1e-6 || ry <= 1e-6 {
        return None;
    }

    let tol_x = (0.005 * rx).max(1e-3);
    let tol_y = (0.005 * ry).max(1e-3);
    let segs = [
        (p0, scale(*raw_a1), scale(*raw_a2), p1),
        (p1, scale(*raw_b1), scale(*raw_b2), p2),
        (p2, scale(*raw_c1), scale(*raw_c2), p3),
        (p3, scale(*raw_d1), scale(*raw_d2), p4),
    ];
    for (q0, c1, c2, q1) in segs {
        if !c1.is_finite() || !c2.is_finite() {
            return None;
        }
        let u = unit_axis_delta(q0, cx, cy, rx, ry)?;
        let v = unit_axis_delta(q1, cx, cy, rx, ry)?;
        if (u.0 * v.0 + u.1 * v.1).abs() > 1e-3 {
            return None;
        }
        let exp_c1x = cx + (u.0 + CIRCLE_KAPPA * v.0) * rx;
        let exp_c1y = cy + (u.1 + CIRCLE_KAPPA * v.1) * ry;
        let exp_c2x = cx + (v.0 + CIRCLE_KAPPA * u.0) * rx;
        let exp_c2y = cy + (v.1 + CIRCLE_KAPPA * u.1) * ry;
        if (c1.x - exp_c1x).abs() > tol_x || (c1.y - exp_c1y).abs() > tol_y || (c2.x - exp_c2x).abs() > tol_x || (c2.y - exp_c2y).abs() > tol_y {
            return None;
        }
    }

    Some(EllipsePx { cx, cy, rx, ry })
}

#[derive(Clone, Copy)]
struct PathStyleEnv {
    sx: f64,
    sy: f64,
    stroke_scale: f64,
    clip: ClipRectPx,
}

#[derive(Clone, Copy)]
struct EllipseEvalGeom {
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    has_stroke: bool,
    r: f64,
    r_sq: f64,
    r_out: f64,
    r_out_sq: f64,
    r_in: f64,
    r_in_sq: f64,
    inv_rx_sq: f64,
    inv_ry_sq: f64,
    max_outer: f64,
}

impl EllipseEvalGeom {
    fn new(cx: f64, cy: f64, rx: f64, ry: f64, sr: f64, has_stroke: bool) -> Self {
        let r = 0.5 * (rx + ry);
        let r_out = r + sr;
        let r_in = (r - sr).max(0.0);
        Self {
            cx,
            cy,
            rx,
            ry,
            has_stroke,
            r,
            r_sq: r * r,
            r_out,
            r_out_sq: r_out * r_out,
            r_in,
            r_in_sq: r_in * r_in,
            inv_rx_sq: 1.0 / (rx * rx),
            inv_ry_sq: 1.0 / (ry * ry),
            max_outer: if has_stroke { r_out } else { rx.max(ry) },
        }
    }

    #[inline]
    fn eval_unclipped_pixel(&self, px_f: f64, py_f: f64) -> (u8, u8) {
        let cdx = (px_f + 0.5) - self.cx;
        let cdy = (py_f + 0.5) - self.cy;
        let dist_c = cdx.hypot(cdy);
        if dist_c - 0.55 > self.max_outer {
            return (0, 0);
        }
        if !self.has_stroke && (self.rx - self.ry).abs() <= 1e-6 && dist_c + 0.55 < self.r {
            return (16, 0);
        }
        if self.has_stroke && dist_c + 0.55 < self.r && self.r_in > 0.0 && dist_c + 0.55 < self.r_in {
            return (16, 0);
        }
        if self.has_stroke && dist_c + 0.55 < self.r_out && (self.r_in <= 0.0 || dist_c - 0.55 > self.r_in) {
            let fill_hits = if dist_c + 0.55 < self.r {
                16
            } else if dist_c - 0.55 > self.r {
                0
            } else {
                count_circle_subpixels(px_f - self.cx, py_f - self.cy, self.r_sq)
            };
            return (fill_hits, 16);
        }

        let mut f_hits = 0u8;
        let mut s_hits = 0u8;
        for &dy_off in &SUBPIXEL_OFFSETS {
            let sy_d = (py_f + dy_off) - self.cy;
            for &dx_off in &SUBPIXEL_OFFSETS {
                let sx_d = (px_f + dx_off) - self.cx;
                if self.has_stroke {
                    let d2 = sx_d * sx_d + sy_d * sy_d;
                    if d2 <= self.r_sq {
                        f_hits += 1;
                    }
                    if d2 <= self.r_out_sq && (self.r_in <= 0.0 || d2 >= self.r_in_sq) {
                        s_hits += 1;
                    }
                } else if sx_d * sx_d * self.inv_rx_sq + sy_d * sy_d * self.inv_ry_sq <= 1.0 {
                    f_hits += 1;
                }
            }
        }
        (f_hits, s_hits)
    }

    #[inline]
    fn eval_clipped_pixel(&self, px_f: f64, py_f: f64, clip: ClipRectPx) -> (u8, u8) {
        let mut f_hits = 0u8;
        let mut s_hits = 0u8;
        for &dy_off in &SUBPIXEL_OFFSETS {
            let y_sub = py_f + dy_off;
            if y_sub < clip.min_y || y_sub >= clip.max_y {
                continue;
            }
            let sy_d = y_sub - self.cy;
            for &dx_off in &SUBPIXEL_OFFSETS {
                let x_sub = px_f + dx_off;
                if x_sub < clip.min_x || x_sub >= clip.max_x {
                    continue;
                }
                let sx_d = x_sub - self.cx;
                if self.has_stroke {
                    let d2 = sx_d * sx_d + sy_d * sy_d;
                    if d2 <= self.r_sq {
                        f_hits += 1;
                    }
                    if d2 <= self.r_out_sq && (self.r_in <= 0.0 || d2 >= self.r_in_sq) {
                        s_hits += 1;
                    }
                } else if sx_d * sx_d * self.inv_rx_sq + sy_d * sy_d * self.inv_ry_sq <= 1.0 {
                    f_hits += 1;
                }
            }
        }
        (f_hits, s_hits)
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn try_rasterize_ellipse_path(
    img: &mut RasterImage,
    ellipse: EllipsePx,
    fill_color: Option<Rgba8>,
    stroke_paint: Option<StrokePaint>,
    env: PathStyleEnv,
    scratch: &mut RasterScratch,
) -> bool {
    let is_near_circle = (ellipse.rx - ellipse.ry).abs() <= 0.05 * ellipse.rx.max(ellipse.ry).max(1.0);
    if let Some(sp) = stroke_paint
        && (sp.dash != LineDashStyle::Solid || !is_near_circle)
    {
        return false;
    }

    let sr = stroke_paint.map_or(0.0, |s| 0.5 * (s.width_pt * env.stroke_scale).max(0.5));
    let stroke_color = stroke_paint.map(|s| s.color);
    let half_w = ellipse.rx + sr;
    let half_h = ellipse.ry + sr;

    let c_min_x = env.clip.min_x.max(0.0);
    let c_max_x = env.clip.max_x.min(f64::from(img.width_px));
    let c_min_y = env.clip.min_y.max(0.0);
    let c_max_y = env.clip.max_y.min(f64::from(img.height_px));
    if ellipse.cx + half_w <= c_min_x || ellipse.cx - half_w >= c_max_x || ellipse.cy + half_h <= c_min_y || ellipse.cy - half_h >= c_max_y {
        return true;
    }

    if half_w <= MAX_STAMP_RADIUS_PX && half_h <= MAX_STAMP_RADIUS_PX {
        let (rx_eff_raw, ry_eff_raw) = if stroke_color.is_some() {
            let r = 0.5 * (ellipse.rx + ellipse.ry);
            (r, r)
        } else {
            (ellipse.rx, ellipse.ry)
        };
        let rx_q = (rx_eff_raw * STAMP_DIM_STEPS).round() as u32;
        let ry_q = (ry_eff_raw * STAMP_DIM_STEPS).round() as u32;
        let sr_q = (sr * STAMP_DIM_STEPS).round() as u32;
        if rx_q > 0 && ry_q > 0 {
            let cx_q = (ellipse.cx * STAMP_PHASE_STEPS).round() as i64;
            let cy_q = (ellipse.cy * STAMP_PHASE_STEPS).round() as i64;
            let anchor_x = cx_q.div_euclid(8) as i32;
            let anchor_y = cy_q.div_euclid(8) as i32;
            let qx = cx_q.rem_euclid(8) as u8;
            let qy = cy_q.rem_euclid(8) as u8;
            let key = EllipseMaskKey { rx_q, ry_q, sr_q, qx, qy };
            let (min_dx, max_dx, min_dy, max_dy) = ellipse_mask_bounds(key);
            let ix0 = anchor_x + min_dx;
            let ix1 = anchor_x + max_dx;
            let iy0 = anchor_y + min_dy;
            let iy1 = anchor_y + max_dy;
            if f64::from(ix0) >= c_min_x && f64::from(ix1) <= c_max_x && f64::from(iy0) >= c_min_y && f64::from(iy1) <= c_max_y {
                let mask_id = get_or_build_ellipse_mask(key, &mut scratch.stamps);
                if let Some(id) = mask_id {
                    blit_mask_or_baked(img, id, anchor_x, anchor_y, fill_color, stroke_color, scratch);
                    return true;
                }
            }
        }
    }

    rasterize_ellipse_direct(img, ellipse, fill_color, stroke_color, sr, env.clip);
    true
}

#[allow(clippy::cast_possible_truncation)]
fn ellipse_mask_bounds(key: EllipseMaskKey) -> (i32, i32, i32, i32) {
    let rx = f64::from(key.rx_q) / STAMP_DIM_STEPS;
    let ry = f64::from(key.ry_q) / STAMP_DIM_STEPS;
    let sr = f64::from(key.sr_q) / STAMP_DIM_STEPS;
    let half_w = rx + sr;
    let half_h = ry + sr;
    let local_cx = f64::from(key.qx) / STAMP_PHASE_STEPS;
    let local_cy = f64::from(key.qy) / STAMP_PHASE_STEPS;
    let min_dx = (local_cx - half_w).floor() as i32;
    let max_dx = (local_cx + half_w).ceil() as i32;
    let min_dy = (local_cy - half_h).floor() as i32;
    let max_dy = (local_cy + half_h).ceil() as i32;
    (min_dx, max_dx.max(min_dx), min_dy, max_dy.max(min_dy))
}

fn get_or_build_ellipse_mask(key: EllipseMaskKey, stamps: &mut MarkerStampCache) -> Option<usize> {
    if let Some(&id) = stamps.ellipse_masks.get(&key) {
        return Some(id);
    }
    if stamps.ellipse_masks.len() >= MAX_CACHED_ELLIPSE_MASKS {
        return None;
    }
    let mask = build_ellipse_mask(key);
    let id = stamps.masks.len();
    stamps.masks.push(mask);
    stamps.ellipse_masks.insert(key, id);
    Some(id)
}

fn build_ellipse_mask(key: EllipseMaskKey) -> CoverageMask {
    let (min_dx, max_dx, min_dy, max_dy) = ellipse_mask_bounds(key);
    let width = u32::try_from(max_dx - min_dx).unwrap_or(0);
    let height = u32::try_from(max_dy - min_dy).unwrap_or(0);
    let geom = EllipseEvalGeom::new(
        f64::from(key.qx) / STAMP_PHASE_STEPS,
        f64::from(key.qy) / STAMP_PHASE_STEPS,
        f64::from(key.rx_q) / STAMP_DIM_STEPS,
        f64::from(key.ry_q) / STAMP_DIM_STEPS,
        f64::from(key.sr_q) / STAMP_DIM_STEPS,
        key.sr_q > 0,
    );

    let mut raw_cells = vec![[0u8; 2]; (width as usize) * (height as usize)];
    for row_idx in 0..(height as usize) {
        let py = f64::from(min_dy + (row_idx as i32));
        for col_idx in 0..(width as usize) {
            let px = f64::from(min_dx + (col_idx as i32));
            let (f_hits, s_hits) = geom.eval_unclipped_pixel(px, py);
            raw_cells[row_idx * (width as usize) + col_idx] = [f_hits, s_hits];
        }
    }

    compact_coverage_mask(min_dx, min_dy, width, height, &raw_cells)
}

#[inline]
fn count_circle_subpixels(base_dx: f64, base_dy: f64, r_sq: f64) -> u8 {
    let mut hits = 0u8;
    for &dy_off in &SUBPIXEL_OFFSETS {
        let dy = base_dy + dy_off;
        let dy2 = dy * dy;
        for &dx_off in &SUBPIXEL_OFFSETS {
            let dx = base_dx + dx_off;
            if dx * dx + dy2 <= r_sq {
                hits += 1;
            }
        }
    }
    hits
}

fn compact_coverage_mask(min_dx: i32, min_dy: i32, width: u32, height: u32, raw_cells: &[[u8; 2]]) -> CoverageMask {
    let w = width as usize;
    let h = height as usize;
    let mut rows = Vec::with_capacity(h);
    let mut cells = Vec::with_capacity(raw_cells.len());
    for row_idx in 0..h {
        let row = &raw_cells[row_idx * w..(row_idx + 1) * w];
        let first = row.iter().position(|&c| c[0] > 0 || c[1] > 0);
        let last = row.iter().rposition(|&c| c[0] > 0 || c[1] > 0);
        match (first, last) {
            (Some(lo), Some(hi)) => {
                let offset = u32::try_from(cells.len()).unwrap_or(0);
                let len = u16::try_from(hi - lo + 1).unwrap_or(0);
                let x_off = u16::try_from(lo).unwrap_or(0);
                cells.extend_from_slice(&row[lo..=hi]);
                rows.push(MaskRowSpan { x_off, len, offset });
            }
            _ => rows.push(MaskRowSpan { x_off: 0, len: 0, offset: 0 }),
        }
    }
    CoverageMask { min_dx, min_dy, rows, cells }
}

fn bake_pixel(fill_hits: u8, stroke_hits: u8, fill_color: Option<Rgba8>, stroke_color: Option<Rgba8>) -> BakedPixel {
    let sa_f = match fill_color {
        Some(fc) if fill_hits > 0 => (f64::from(fc.a) / 255.0) * (f64::from(fill_hits) / 16.0),
        _ => 0.0,
    };
    let sa_s = match stroke_color {
        Some(sc) if stroke_hits > 0 => (f64::from(sc.a) / 255.0) * (f64::from(stroke_hits) / 16.0),
        _ => 0.0,
    };
    if sa_f <= 0.0 && sa_s <= 0.0 {
        return BakedPixel::Empty;
    }

    let (mut pr, mut pg, mut pb, mut a) = match fill_color {
        Some(fc) if sa_f > 0.0 => (f64::from(fc.r) * sa_f, f64::from(fc.g) * sa_f, f64::from(fc.b) * sa_f, sa_f),
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    if let Some(sc) = stroke_color
        && sa_s > 0.0
    {
        let inv_s = 1.0 - sa_s;
        pr = f64::from(sc.r) * sa_s + pr * inv_s;
        pg = f64::from(sc.g) * sa_s + pg * inv_s;
        pb = f64::from(sc.b) * sa_s + pb * inv_s;
        a = sa_s + a * inv_s;
    }

    if a <= 1e-12 {
        return BakedPixel::Empty;
    }
    if a >= 1.0 - 1e-9 {
        return BakedPixel::Opaque([clamp_u8(pr), clamp_u8(pg), clamp_u8(pb), u8::MAX]);
    }
    let straight = [clamp_u8(pr / a), clamp_u8(pg / a), clamp_u8(pb / a), clamp_u8(a * 255.0)];
    BakedPixel::Blend { straight, pr: pr as f32, pg: pg as f32, pb: pb as f32, sa: a as f32, inv_sa: (1.0 - a) as f32 }
}

fn bake_stamp_from_mask(mask: &CoverageMask, fill_color: Option<Rgba8>, stroke_color: Option<Rgba8>) -> BakedStamp {
    let mut pixels = Vec::with_capacity(mask.cells.len());
    for &[fh, sh] in &mask.cells {
        pixels.push(bake_pixel(fh, sh, fill_color, stroke_color));
    }
    BakedStamp { min_dx: mask.min_dx, min_dy: mask.min_dy, rows: mask.rows.clone(), pixels }
}

fn blit_mask_or_baked(
    img: &mut RasterImage,
    mask_id: usize,
    anchor_x: i32,
    anchor_y: i32,
    fill_color: Option<Rgba8>,
    stroke_color: Option<Rgba8>,
    scratch: &mut RasterScratch,
) {
    let key = BakedStampKey { mask_id, fill: fill_color.map(|c| [c.r, c.g, c.b, c.a]), stroke: stroke_color.map(|c| [c.r, c.g, c.b, c.a]) };
    if let Some(stamp) = scratch.stamps.baked.get(&key) {
        blit_baked_stamp(img, stamp, anchor_x, anchor_y, scratch.canvas_opaque);
        return;
    }
    let Some(mask) = scratch.stamps.masks.get(mask_id) else {
        return;
    };
    if scratch.stamps.baked.len() < MAX_CACHED_BAKED_STAMPS {
        let stamp = bake_stamp_from_mask(mask, fill_color, stroke_color);
        blit_baked_stamp(img, &stamp, anchor_x, anchor_y, scratch.canvas_opaque);
        scratch.stamps.baked.insert(key, stamp);
    } else {
        blit_mask_uncached(img, mask, anchor_x, anchor_y, fill_color, stroke_color);
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
#[inline]
fn clamp_u8_f32(val: f32) -> u8 {
    val.round().clamp(0.0, 255.0) as u8
}

#[inline]
fn blend_baked_translucent_dst(dst: &mut [u8], pr: f32, pg: f32, pb: f32, sa: f32, inv_sa: f32) {
    let da = f64::from(dst[3]) / 255.0;
    let da_inv = da * f64::from(inv_sa);
    let out_a = f64::from(sa) + da_inv;
    if out_a <= 1e-12 {
        dst[0] = 0;
        dst[1] = 0;
        dst[2] = 0;
        dst[3] = 0;
        return;
    }
    dst[0] = clamp_u8((f64::from(pr) + f64::from(dst[0]) * da_inv) / out_a);
    dst[1] = clamp_u8((f64::from(pg) + f64::from(dst[1]) * da_inv) / out_a);
    dst[2] = clamp_u8((f64::from(pb) + f64::from(dst[2]) * da_inv) / out_a);
    dst[3] = clamp_u8(out_a * 255.0);
}

fn blit_baked_stamp(img: &mut RasterImage, stamp: &BakedStamp, anchor_x: i32, anchor_y: i32, canvas_opaque: bool) {
    let Ok(dst_x0) = usize::try_from(anchor_x + stamp.min_dx) else { return };
    let Ok(dst_y0) = usize::try_from(anchor_y + stamp.min_dy) else { return };
    let img_w = img.width_px as usize;
    for (row_idx, span) in stamp.rows.iter().enumerate() {
        if span.len == 0 {
            continue;
        }
        let span_len = usize::from(span.len);
        let src_start = span.offset as usize;
        let Some(src_row) = stamp.pixels.get(src_start..src_start + span_len) else { continue };
        let row_byte_start = ((dst_y0 + row_idx) * img_w + dst_x0 + usize::from(span.x_off)) * 4;
        let Some(dst_row) = img.rgba.get_mut(row_byte_start..row_byte_start + span_len * 4) else { continue };

        if canvas_opaque {
            for (dst, &px) in dst_row.as_chunks_mut::<4>().0.iter_mut().zip(src_row.iter()) {
                match px {
                    BakedPixel::Empty => {}
                    BakedPixel::Opaque(rgba) => *dst = rgba,
                    BakedPixel::Blend { pr, pg, pb, inv_sa, .. } => {
                        dst[0] = clamp_u8_f32(pr + f32::from(dst[0]) * inv_sa);
                        dst[1] = clamp_u8_f32(pg + f32::from(dst[1]) * inv_sa);
                        dst[2] = clamp_u8_f32(pb + f32::from(dst[2]) * inv_sa);
                    }
                }
            }
        } else {
            for (dst, &px) in dst_row.as_chunks_mut::<4>().0.iter_mut().zip(src_row.iter()) {
                match px {
                    BakedPixel::Empty => {}
                    BakedPixel::Opaque(rgba) => *dst = rgba,
                    BakedPixel::Blend { straight, pr, pg, pb, sa, inv_sa } => {
                        if dst[3] == 0 {
                            *dst = straight;
                        } else if dst[3] == u8::MAX {
                            dst[0] = clamp_u8_f32(pr + f32::from(dst[0]) * inv_sa);
                            dst[1] = clamp_u8_f32(pg + f32::from(dst[1]) * inv_sa);
                            dst[2] = clamp_u8_f32(pb + f32::from(dst[2]) * inv_sa);
                        } else {
                            blend_baked_translucent_dst(dst, pr, pg, pb, sa, inv_sa);
                        }
                    }
                }
            }
        }
    }
}

fn blit_mask_uncached(
    img: &mut RasterImage,
    mask: &CoverageMask,
    anchor_x: i32,
    anchor_y: i32,
    fill_color: Option<Rgba8>,
    stroke_color: Option<Rgba8>,
) {
    let Ok(dst_x0) = usize::try_from(anchor_x + mask.min_dx) else { return };
    let Ok(dst_y0) = usize::try_from(anchor_y + mask.min_dy) else { return };
    let img_w = img.width_px as usize;
    for (row_idx, span) in mask.rows.iter().enumerate() {
        if span.len == 0 {
            continue;
        }
        let span_len = usize::from(span.len);
        let src_start = span.offset as usize;
        let Some(src_row) = mask.cells.get(src_start..src_start + span_len) else { continue };
        let row_byte_start = ((dst_y0 + row_idx) * img_w + dst_x0 + usize::from(span.x_off)) * 4;
        let Some(dst_row) = img.rgba.get_mut(row_byte_start..row_byte_start + span_len * 4) else { continue };
        for (dst, &[fh, sh]) in dst_row.as_chunks_mut::<4>().0.iter_mut().zip(src_row.iter()) {
            if let Some(fc) = fill_color
                && fh > 0
            {
                blend_pixel_slice(dst, fc, f64::from(fh) / 16.0);
            }
            if let Some(sc) = stroke_color
                && sh > 0
            {
                blend_pixel_slice(dst, sc, f64::from(sh) / 16.0);
            }
        }
    }
}

fn rasterize_ellipse_direct(
    img: &mut RasterImage,
    ellipse: EllipsePx,
    fill_color: Option<Rgba8>,
    stroke_color: Option<Rgba8>,
    sr: f64,
    clip: ClipRectPx,
) {
    let half_w = ellipse.rx + sr;
    let half_h = ellipse.ry + sr;
    let Some((x_start, x_end, y_start, y_end)) =
        clipped_pixel_bounds(ellipse.cx - half_w, ellipse.cy - half_h, ellipse.cx + half_w, ellipse.cy + half_h, clip, img.width_px, img.height_px)
    else {
        return;
    };

    let geom = EllipseEvalGeom::new(ellipse.cx, ellipse.cy, ellipse.rx, ellipse.ry, sr, stroke_color.is_some() && sr > 0.0);
    let img_w = img.width_px as usize;

    for py in y_start..y_end {
        let py_f = f64::from(py);
        let row_full_y = py_f + 0.125 >= clip.min_y && py_f + 0.875 < clip.max_y;
        let row_base = (py as usize) * img_w;
        for px in x_start..x_end {
            let px_f = f64::from(px);
            let (f_hits, s_hits) = if row_full_y && px_f + 0.125 >= clip.min_x && px_f + 0.875 < clip.max_x {
                geom.eval_unclipped_pixel(px_f, py_f)
            } else {
                geom.eval_clipped_pixel(px_f, py_f, clip)
            };
            if f_hits == 0 && s_hits == 0 {
                continue;
            }
            let idx = (row_base + (px as usize)) * 4;
            let Some(dst) = img.rgba.get_mut(idx..idx + 4) else { continue };
            if let Some(fc) = fill_color
                && f_hits > 0
            {
                blend_pixel_slice(dst, fc, f64::from(f_hits) / 16.0);
            }
            if let Some(sc) = stroke_color
                && s_hits > 0
            {
                blend_pixel_slice(dst, sc, f64::from(s_hits) / 16.0);
            }
        }
    }
}

#[allow(clippy::cast_possible_truncation)]
fn try_rasterize_small_poly_stamp(
    img: &mut RasterImage,
    path: &Path2D,
    fill_color: Option<Rgba8>,
    stroke_paint: Option<StrokePaint>,
    env: PathStyleEnv,
    scratch: &mut RasterScratch,
) -> bool {
    let n = path.commands.len();
    if !(4..=24).contains(&n) {
        return false;
    }
    if let Some(sp) = stroke_paint
        && sp.dash != LineDashStyle::Solid
    {
        return false;
    }
    let Some(PathCmd::MoveTo(raw_p0)) = path.commands.first() else {
        return false;
    };
    let p0 = PointPx::new(raw_p0.x * env.sx, raw_p0.y * env.sy);
    if !p0.is_finite() {
        return false;
    }

    let cx_q = (p0.x * STAMP_PHASE_STEPS).round() as i64;
    let cy_q = (p0.y * STAMP_PHASE_STEPS).round() as i64;
    let anchor_x = cx_q.div_euclid(8) as i32;
    let anchor_y = cy_q.div_euclid(8) as i32;
    let qx = cx_q.rem_euclid(8) as u8;
    let qy = cy_q.rem_euclid(8) as u8;

    let mut cmds = [SmallPolyCmdQ { op: 0, dx: 0, dy: 0 }; 24];
    let mut min_dx_f = 0.0f64;
    let mut max_dx_f = 0.0f64;
    let mut min_dy_f = 0.0f64;
    let mut max_dy_f = 0.0f64;
    let mut has_close = false;
    let mut move_count = 0u8;

    for (idx, cmd) in path.commands.iter().enumerate() {
        let (op, pt_opt) = match *cmd {
            PathCmd::MoveTo(pt) => {
                move_count += 1;
                (0u8, Some(pt))
            }
            PathCmd::LineTo(pt) => (1u8, Some(pt)),
            PathCmd::Close => {
                has_close = true;
                (2u8, None)
            }
            PathCmd::QuadTo { .. } | PathCmd::CubicTo { .. } => return false,
        };
        if let Some(pt) = pt_opt {
            let p = PointPx::new(pt.x * env.sx, pt.y * env.sy);
            if !p.is_finite() {
                return false;
            }
            let rdx = p.x - p0.x;
            let rdy = p.y - p0.y;
            if rdx.abs() > MAX_SMALL_POLY_EXTENT_PX || rdy.abs() > MAX_SMALL_POLY_EXTENT_PX {
                return false;
            }
            let Ok(dx_q) = i16::try_from((rdx * STAMP_DIM_STEPS).round() as i64) else { return false };
            let Ok(dy_q) = i16::try_from((rdy * STAMP_DIM_STEPS).round() as i64) else { return false };
            let qdx_f = f64::from(dx_q) / STAMP_DIM_STEPS;
            let qdy_f = f64::from(dy_q) / STAMP_DIM_STEPS;
            min_dx_f = min_dx_f.min(qdx_f);
            max_dx_f = max_dx_f.max(qdx_f);
            min_dy_f = min_dy_f.min(qdy_f);
            max_dy_f = max_dy_f.max(qdy_f);
            cmds[idx] = SmallPolyCmdQ { op, dx: dx_q, dy: dy_q };
        } else {
            cmds[idx] = SmallPolyCmdQ { op, dx: 0, dy: 0 };
        }
    }

    if !has_close && move_count < 2 {
        return false;
    }
    if (max_dx_f - min_dx_f) > MAX_SMALL_POLY_EXTENT_PX || (max_dy_f - min_dy_f) > MAX_SMALL_POLY_EXTENT_PX {
        return false;
    }

    let sr = stroke_paint.map_or(0.0, |s| 0.5 * (s.width_pt * env.stroke_scale).max(0.5));
    let Ok(sr_q) = u16::try_from((sr * STAMP_DIM_STEPS).round() as i64) else { return false };
    let sr_eff = f64::from(sr_q) / STAMP_DIM_STEPS;
    let stroke_pad = if sr_q > 0 { (MITER_LIMIT * sr_eff).max(sr_eff * 2.0) + 1.0 } else { 1.0 };

    let local_cx = f64::from(qx) / STAMP_PHASE_STEPS;
    let local_cy = f64::from(qy) / STAMP_PHASE_STEPS;
    let min_dx = (local_cx + min_dx_f - stroke_pad).floor() as i32;
    let max_dx = (local_cx + max_dx_f + stroke_pad).ceil() as i32;
    let min_dy = (local_cy + min_dy_f - stroke_pad).floor() as i32;
    let max_dy = (local_cy + max_dy_f + stroke_pad).ceil() as i32;
    let width = u32::try_from(max_dx - min_dx).unwrap_or(0);
    let height = u32::try_from(max_dy - min_dy).unwrap_or(0);
    if width == 0 || height == 0 || width > 48 || height > 48 {
        return false;
    }

    let ix0 = anchor_x + min_dx;
    let ix1 = anchor_x + max_dx;
    let iy0 = anchor_y + min_dy;
    let iy1 = anchor_y + max_dy;
    let c_min_x = env.clip.min_x.max(0.0);
    let c_max_x = env.clip.max_x.min(f64::from(img.width_px));
    let c_min_y = env.clip.min_y.max(0.0);
    let c_max_y = env.clip.max_y.min(f64::from(img.height_px));
    if f64::from(ix0) < c_min_x || f64::from(ix1) > c_max_x || f64::from(iy0) < c_min_y || f64::from(iy1) > c_max_y {
        return false;
    }

    let (cap_u8, join_u8) = match stroke_paint {
        Some(sp) => (line_cap_code(sp.cap), line_join_code(sp.join)),
        None => (0, 0),
    };
    let key = SmallPolyKey { len: n as u8, cmds, qx, qy, sr_q, cap: cap_u8, join: join_u8 };
    let mask_id = if let Some(&id) = scratch.stamps.poly_masks.get(&key) {
        Some(id)
    } else if scratch.stamps.poly_masks.len() < MAX_CACHED_POLY_MASKS {
        let mask = build_small_poly_mask(&key, min_dx, min_dy, width, height, stroke_paint, scratch);
        let id = scratch.stamps.masks.len();
        scratch.stamps.masks.push(mask);
        scratch.stamps.poly_masks.insert(key, id);
        Some(id)
    } else {
        None
    };

    if let Some(id) = mask_id {
        blit_mask_or_baked(img, id, anchor_x, anchor_y, fill_color, stroke_paint.map(|s| s.color), scratch);
        true
    } else {
        false
    }
}

fn line_cap_code(cap: LineCap) -> u8 {
    match cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    }
}

fn line_join_code(join: LineJoin) -> u8 {
    match join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    }
}

fn build_small_poly_mask(
    key: &SmallPolyKey,
    min_dx: i32,
    min_dy: i32,
    width: u32,
    height: u32,
    stroke_paint: Option<StrokePaint>,
    scratch: &mut RasterScratch,
) -> CoverageMask {
    let origin_x = f64::from(key.qx) / STAMP_PHASE_STEPS - f64::from(min_dx);
    let origin_y = f64::from(key.qy) / STAMP_PHASE_STEPS - f64::from(min_dy);
    let mut local_path = Path2D::new();
    for cmd in &key.cmds[..usize::from(key.len)] {
        let pt = curvebook_chart::PointPt::new(origin_x + f64::from(cmd.dx) / STAMP_DIM_STEPS, origin_y + f64::from(cmd.dy) / STAMP_DIM_STEPS);
        match cmd.op {
            0 => local_path.commands.push(PathCmd::MoveTo(pt)),
            1 => local_path.commands.push(PathCmd::LineTo(pt)),
            _ => local_path.commands.push(PathCmd::Close),
        }
    }

    flatten_path_into(&local_path, 1.0, 1.0, &mut scratch.subpaths, &mut scratch.vec_pool);
    let mut raw_cells = vec![[0u8; 2]; (width as usize) * (height as usize)];
    let local_clip = ClipRectPx::from_canvas(width, height);
    if !scratch.subpaths.is_empty() {
        fill_subpaths_target(
            ScanTarget::FillMask { min_dx: 0, min_dy: 0, width, cells: &mut raw_cells },
            &scratch.subpaths,
            local_clip,
            &mut scratch.scan,
        );
        if let Some(sp) = stroke_paint {
            let mut local_sp = sp;
            local_sp.width_pt = (f64::from(key.sr_q) * 2.0) / STAMP_DIM_STEPS;
            stroke_subpaths_target(
                ScanTarget::StrokeMask { min_dx: 0, min_dy: 0, width, cells: &mut raw_cells },
                &scratch.subpaths,
                &local_sp,
                1.0,
                local_clip,
                &mut scratch.scan,
            );
        }
    }
    reclaim_subpaths(&mut scratch.subpaths, &mut scratch.vec_pool);
    compact_coverage_mask(min_dx, min_dy, width, height, &raw_cells)
}

fn try_extract_axis_aligned_rect(path: &Path2D, sx: f64, sy: f64) -> Option<(f64, f64, f64, f64)> {
    let [PathCmd::MoveTo(r0), PathCmd::LineTo(r1), PathCmd::LineTo(r2), PathCmd::LineTo(r3), PathCmd::Close] = path.commands.as_slice() else {
        return None;
    };
    let p0 = PointPx::new(r0.x * sx, r0.y * sy);
    let p1 = PointPx::new(r1.x * sx, r1.y * sy);
    let p2 = PointPx::new(r2.x * sx, r2.y * sy);
    let p3 = PointPx::new(r3.x * sx, r3.y * sy);
    if !p0.is_finite() || !p1.is_finite() || !p2.is_finite() || !p3.is_finite() {
        return None;
    }
    let horiz_first = (p0.y - p1.y).abs() <= 1e-9 && (p1.x - p2.x).abs() <= 1e-9 && (p2.y - p3.y).abs() <= 1e-9 && (p3.x - p0.x).abs() <= 1e-9;
    let vert_first = (p0.x - p1.x).abs() <= 1e-9 && (p1.y - p2.y).abs() <= 1e-9 && (p2.x - p3.x).abs() <= 1e-9 && (p3.y - p0.y).abs() <= 1e-9;
    if !horiz_first && !vert_first {
        return None;
    }
    let min_x = p0.x.min(p2.x);
    let max_x = p0.x.max(p2.x);
    let min_y = p0.y.min(p2.y);
    let max_y = p0.y.max(p2.y);
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    Some((min_x, min_y, max_x, max_y))
}

#[inline]
fn subpixel_interval_hits(px: u32, lo: f64, hi: f64) -> u8 {
    let px_f = f64::from(px);
    if lo <= px_f + 0.125 && px_f + 0.875 < hi {
        return 4;
    }
    let mut hits = 0u8;
    for &off in &SUBPIXEL_OFFSETS {
        let s = px_f + off;
        if lo <= s && s < hi {
            hits += 1;
        }
    }
    hits
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn fill_rect_fast(img: &mut RasterImage, rect: (f64, f64, f64, f64), color: Rgba8, clip: ClipRectPx, canvas_opaque: bool) {
    let lo_x = rect.0.max(clip.min_x).max(0.0);
    let hi_x = rect.2.min(clip.max_x).min(f64::from(img.width_px));
    let lo_y = rect.1.max(clip.min_y).max(0.0);
    let hi_y = rect.3.min(clip.max_y).min(f64::from(img.height_px));
    if !lo_x.is_finite() || !hi_x.is_finite() || !lo_y.is_finite() || !hi_y.is_finite() || hi_x <= lo_x || hi_y <= lo_y {
        return;
    }
    let x_start = (lo_x.floor() as u32).min(img.width_px);
    let x_end = (hi_x.ceil() as u32).clamp(x_start, img.width_px);
    let y_start = (lo_y.floor() as u32).min(img.height_px);
    let y_end = (hi_y.ceil() as u32).clamp(y_start, img.height_px);
    if x_end <= x_start || y_end <= y_start {
        return;
    }

    let left_hits = subpixel_interval_hits(x_start, lo_x, hi_x);
    let full_x_lo = x_start + u32::from(left_hits < 4);
    let (right_hits, full_x_hi) = if x_end > full_x_lo {
        let rh = subpixel_interval_hits(x_end - 1, lo_x, hi_x);
        (rh, x_end - u32::from(rh < 4))
    } else {
        (0, full_x_lo)
    };

    let img_w = img.width_px as usize;
    let src_arr = [color.r, color.g, color.b, u8::MAX];
    for py in y_start..y_end {
        let y_hits = subpixel_interval_hits(py, lo_y, hi_y);
        if y_hits == 0 {
            continue;
        }
        let row_base = (py as usize) * img_w;
        if left_hits < 4 && left_hits > 0 {
            let idx = (row_base + (x_start as usize)) * 4;
            if let Some(px) = img.rgba.get_mut(idx..idx + 4) {
                blend_pixel_slice(px, color, f64::from(left_hits * y_hits) / 16.0);
            }
        }
        if full_x_hi > full_x_lo {
            let start_b = (row_base + (full_x_lo as usize)) * 4;
            let end_b = (row_base + (full_x_hi as usize)) * 4;
            if let Some(span) = img.rgba.get_mut(start_b..end_b) {
                if y_hits == 4 && color.a == u8::MAX {
                    for px in span.as_chunks_mut::<4>().0 {
                        *px = src_arr;
                    }
                } else if canvas_opaque {
                    let sa = (f64::from(color.a) / 255.0) * (f64::from(y_hits) / 4.0);
                    let inv = (1.0 - sa) as f32;
                    let pr = (f64::from(color.r) * sa) as f32;
                    let pg = (f64::from(color.g) * sa) as f32;
                    let pb = (f64::from(color.b) * sa) as f32;
                    for px in span.as_chunks_mut::<4>().0 {
                        px[0] = clamp_u8_f32(pr + f32::from(px[0]) * inv);
                        px[1] = clamp_u8_f32(pg + f32::from(px[1]) * inv);
                        px[2] = clamp_u8_f32(pb + f32::from(px[2]) * inv);
                    }
                } else {
                    let cov = f64::from(y_hits) / 4.0;
                    for px in span.as_chunks_mut::<4>().0 {
                        blend_pixel_slice(px, color, cov);
                    }
                }
            }
        }
        if x_end > full_x_lo && right_hits < 4 && right_hits > 0 {
            let idx = (row_base + ((x_end - 1) as usize)) * 4;
            if let Some(px) = img.rgba.get_mut(idx..idx + 4) {
                blend_pixel_slice(px, color, f64::from(right_hits * y_hits) / 16.0);
            }
        }
    }
}

fn rasterize_path(
    img: &mut RasterImage,
    path: &Path2D,
    fill: Option<FillPaint>,
    stroke: Option<StrokePaint>,
    env: PathStyleEnv,
    scratch: &mut RasterScratch,
) {
    let needs_fill = fill.is_some_and(|f| f.color.a > 0);
    let needs_stroke = stroke.is_some_and(|s| s.color.a > 0 && s.width_pt.is_finite() && s.width_pt > 0.0);
    if !needs_fill && !needs_stroke {
        return;
    }

    let fill_color = fill.filter(|_| needs_fill).map(|f| f.color);
    let stroke_paint = stroke.filter(|_| needs_stroke);

    if let Some(ellipse) = try_extract_axis_aligned_ellipse(path, env.sx, env.sy)
        && try_rasterize_ellipse_path(img, ellipse, fill_color, stroke_paint, env, scratch)
    {
        return;
    }

    if try_rasterize_small_poly_stamp(img, path, fill_color, stroke_paint, env, scratch) {
        return;
    }

    if let Some(rect) = try_extract_axis_aligned_rect(path, env.sx, env.sy) {
        if let Some(color) = fill_color {
            fill_rect_fast(img, rect, color, env.clip, scratch.canvas_opaque);
        }
        if let Some(sp) = stroke_paint {
            flatten_path_into(path, env.sx, env.sy, &mut scratch.subpaths, &mut scratch.vec_pool);
            if !scratch.subpaths.is_empty() {
                stroke_subpaths_target(
                    ScanTarget::Image { img, color: sp.color },
                    &scratch.subpaths,
                    &sp,
                    env.stroke_scale,
                    env.clip,
                    &mut scratch.scan,
                );
            }
            reclaim_subpaths(&mut scratch.subpaths, &mut scratch.vec_pool);
        }
        return;
    }

    flatten_path_into(path, env.sx, env.sy, &mut scratch.subpaths, &mut scratch.vec_pool);
    if scratch.subpaths.is_empty() {
        return;
    }

    if let Some(color) = fill_color {
        fill_subpaths_target(ScanTarget::Image { img, color }, &scratch.subpaths, env.clip, &mut scratch.scan);
    }
    if let Some(sp) = stroke_paint {
        stroke_subpaths_target(ScanTarget::Image { img, color: sp.color }, &scratch.subpaths, &sp, env.stroke_scale, env.clip, &mut scratch.scan);
    }
    reclaim_subpaths(&mut scratch.subpaths, &mut scratch.vec_pool);
}

fn flatten_path_into(path: &Path2D, sx: f64, sy: f64, subpaths: &mut Vec<Subpath>, pool: &mut Vec<Vec<PointPx>>) {
    reclaim_subpaths(subpaths, pool);
    let mut current: Option<Subpath> = None;
    let mut start_pt: Option<PointPx> = None;

    for cmd in &path.commands {
        match *cmd {
            PathCmd::MoveTo(pt) => {
                if let Some(mut sub) = current.take() {
                    if sub.has_segment {
                        subpaths.push(sub);
                    } else if pool.len() < 16 {
                        sub.points.clear();
                        pool.push(sub.points);
                    }
                }
                let p = PointPx::new(pt.x * sx, pt.y * sy);
                if p.is_finite() {
                    start_pt = Some(p);
                    current = Some(Subpath { points: take_point_vec(pool, p), closed: false, has_segment: false });
                } else {
                    start_pt = None;
                }
            }
            PathCmd::LineTo(pt) => {
                let p = PointPx::new(pt.x * sx, pt.y * sy);
                if p.is_finite() {
                    push_line_point(&mut current, &mut start_pt, p, pool);
                }
            }
            PathCmd::QuadTo { control, to } => {
                let c = PointPx::new(control.x * sx, control.y * sy);
                let p = PointPx::new(to.x * sx, to.y * sy);
                if c.is_finite() && p.is_finite() {
                    push_quad_points(&mut current, &mut start_pt, c, p, pool);
                }
            }
            PathCmd::CubicTo { c1, c2, to } => {
                let p1 = PointPx::new(c1.x * sx, c1.y * sy);
                let p2 = PointPx::new(c2.x * sx, c2.y * sy);
                let p = PointPx::new(to.x * sx, to.y * sy);
                if p1.is_finite() && p2.is_finite() && p.is_finite() {
                    push_cubic_points(&mut current, &mut start_pt, p1, p2, p, pool);
                }
            }
            PathCmd::Close => {
                if let Some(mut sub) = current.take() {
                    sub.closed = true;
                    if !sub.points.is_empty() {
                        subpaths.push(sub);
                    } else if pool.len() < 16 {
                        sub.points.clear();
                        pool.push(sub.points);
                    }
                }
                if let Some(st) = start_pt {
                    current = Some(Subpath { points: take_point_vec(pool, st), closed: false, has_segment: false });
                }
            }
        }
    }

    if let Some(mut sub) = current {
        if sub.has_segment {
            subpaths.push(sub);
        } else if pool.len() < 16 {
            sub.points.clear();
            pool.push(sub.points);
        }
    }
}

fn push_line_point(current: &mut Option<Subpath>, start_pt: &mut Option<PointPx>, p: PointPx, pool: &mut Vec<Vec<PointPx>>) {
    if let Some(sub) = current.as_mut() {
        sub.has_segment = true;
        if sub.points.last().is_none_or(|prev| prev.dist_sq(p) > 1e-18) {
            sub.points.push(p);
        }
    } else {
        *start_pt = Some(p);
        *current = Some(Subpath { points: take_point_vec(pool, p), closed: false, has_segment: false });
    }
}

fn push_quad_points(current: &mut Option<Subpath>, start_pt: &mut Option<PointPx>, c: PointPx, to: PointPx, pool: &mut Vec<Vec<PointPx>>) {
    let Some(sub) = current.as_mut() else {
        *start_pt = Some(to);
        *current = Some(Subpath { points: take_point_vec(pool, to), closed: false, has_segment: false });
        return;
    };
    sub.has_segment = true;
    if let Some(&p0) = sub.points.last() {
        flatten_quad(p0, c, to, 0, &mut sub.points);
    }
}

fn push_cubic_points(
    current: &mut Option<Subpath>,
    start_pt: &mut Option<PointPx>,
    c1: PointPx,
    c2: PointPx,
    to: PointPx,
    pool: &mut Vec<Vec<PointPx>>,
) {
    let Some(sub) = current.as_mut() else {
        *start_pt = Some(to);
        *current = Some(Subpath { points: take_point_vec(pool, to), closed: false, has_segment: false });
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

fn fill_subpaths_target(mut target: ScanTarget<'_>, subpaths: &[Subpath], clip: ClipRectPx, scan: &mut ScanScratch) {
    scan.edges.clear();
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
            scan.edges.push(PolyEdge { a, b, y_min, y_max, dir });
        }
    }

    if scan.edges.is_empty() {
        return;
    }

    let (bound_w, bound_h) = target.bounds_px();
    let Some((x_start, x_end, y_start, y_end)) = clipped_pixel_bounds(min_x, min_y, max_x, max_y, clip, bound_w, bound_h) else {
        return;
    };
    let row_len = (x_end - x_start) as usize;
    scan.row_cov.resize(row_len, 0);
    scan.edges.sort_by(|e0, e1| e0.y_min.total_cmp(&e1.y_min));
    scan.active.clear();
    let mut next_edge = 0usize;

    for py in y_start..y_end {
        let row_top = f64::from(py);
        let row_bot = row_top + 1.0;
        while let Some(edge) = scan.edges.get(next_edge) {
            if edge.y_min > row_bot {
                break;
            }
            scan.active.push(next_edge);
            next_edge += 1;
        }
        scan.active.retain(|&idx| scan.edges.get(idx).is_some_and(|e| e.y_max >= row_top));
        if scan.active.is_empty() {
            continue;
        }

        scan.row_cov.fill(0);
        for &dy_off in &SUBPIXEL_OFFSETS {
            let y_sub = row_top + dy_off;
            if y_sub < clip.min_y || y_sub >= clip.max_y {
                continue;
            }
            collect_scanline_crossings(&scan.edges, &scan.active, y_sub, &mut scan.crossings);
            accumulate_winding_spans(&scan.crossings, x_start, x_end, clip, &mut scan.row_cov);
        }

        target.write_row(py, x_start, &scan.row_cov);
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

struct StrokeEmitBufs<'a> {
    dash_pattern: &'a mut Vec<f64>,
    dash_pts: &'a mut Vec<PointPx>,
    dirs: &'a mut Vec<PointPx>,
    pieces: &'a mut Vec<StrokePiece>,
}

fn stroke_subpaths_target(
    mut target: ScanTarget<'_>,
    subpaths: &[Subpath],
    stroke: &StrokePaint,
    stroke_scale: f64,
    clip: ClipRectPx,
    scan: &mut ScanScratch,
) {
    if clip.is_empty() {
        return;
    }
    let width_px = (stroke.width_pt * stroke_scale).max(0.5);
    let radius = 0.5 * width_px;
    let clip_margin = (MITER_LIMIT * radius).max(radius * 2.0) + 2.0;
    let dash_clip = clip.expand(clip_margin);
    scan.pieces.clear();
    let mut dash_segments = 0usize;

    for sub in subpaths {
        normalize_subpath_points_into(&sub.points, sub.closed, &mut scan.clean_points);
        if scan.clean_points.is_empty() {
            continue;
        }
        if stroke.dash == LineDashStyle::Solid {
            emit_solid_subpath_pieces(&scan.clean_points, sub.closed, radius, stroke.cap, stroke.join, &mut scan.dirs, &mut scan.pieces);
        } else {
            let mut bufs =
                StrokeEmitBufs { dash_pattern: &mut scan.dash_pattern, dash_pts: &mut scan.dash_pts, dirs: &mut scan.dirs, pieces: &mut scan.pieces };
            emit_dashed_subpath_pieces(&scan.clean_points, sub.closed, width_px, radius, stroke, dash_clip, &mut dash_segments, &mut bufs);
        }
    }

    if scan.pieces.is_empty() {
        return;
    }

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for p in &scan.pieces {
        min_x = min_x.min(p.x_min);
        max_x = max_x.max(p.x_max);
        min_y = min_y.min(p.y_min);
        max_y = max_y.max(p.y_max);
    }

    let (bound_w, bound_h) = target.bounds_px();
    let Some((x_start, x_end, y_start, y_end)) = clipped_pixel_bounds(min_x, min_y, max_x, max_y, clip, bound_w, bound_h) else {
        return;
    };

    scan.pieces.sort_by(|a, b| a.y_min.total_cmp(&b.y_min));
    let row_len = (x_end - x_start) as usize;
    scan.row_masks.resize(row_len, 0);
    scan.active.clear();
    let mut next_piece = 0usize;

    for py in y_start..y_end {
        let row_top = f64::from(py);
        let row_bot = row_top + 1.0;
        while let Some(piece) = scan.pieces.get(next_piece) {
            if piece.y_min > row_bot {
                break;
            }
            scan.active.push(next_piece);
            next_piece += 1;
        }
        scan.active.retain(|&idx| scan.pieces.get(idx).is_some_and(|p| p.y_max >= row_top));
        if scan.active.is_empty() {
            continue;
        }

        scan.row_masks.fill(0);
        for (ky, &dy_off) in SUBPIXEL_OFFSETS.iter().enumerate() {
            let y_sub = row_top + dy_off;
            if y_sub < clip.min_y || y_sub >= clip.max_y {
                continue;
            }
            for &idx in &scan.active {
                if let Some((x_lo, x_hi)) = scan.pieces.get(idx).and_then(|p| p.x_interval_at(y_sub)) {
                    set_stroke_row_bits(x_lo, x_hi, ky, x_start, x_end, clip, &mut scan.row_masks);
                }
            }
        }

        target.write_stroke_row(py, x_start, &scan.row_masks);
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

fn normalize_subpath_points_into(points: &[PointPx], closed: bool, clean: &mut Vec<PointPx>) {
    clean.clear();
    for &p in points {
        if clean.last().is_none_or(|prev| prev.dist_sq(p) > 1e-18) {
            clean.push(p);
        }
    }
    if closed && clean.len() > 1 && clean.first().zip(clean.last()).is_some_and(|(a, b)| a.dist_sq(*b) <= 1e-18) {
        let _ = clean.pop();
    }
}

fn emit_solid_subpath_pieces(
    points: &[PointPx],
    closed: bool,
    r: f64,
    cap: LineCap,
    join: LineJoin,
    dirs: &mut Vec<PointPx>,
    out: &mut Vec<StrokePiece>,
) {
    if points.len() == 1 {
        if let Some(&pt) = points.first() {
            emit_zero_length_cap(pt, r, cap, out);
        }
        return;
    }

    let seg_count = if closed { points.len() } else { points.len() - 1 };
    dirs.clear();
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

#[derive(Debug, Clone, Copy)]
enum ClipEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy)]
struct ClippedSegment {
    start: PointPx,
    end: PointPx,
    entered: bool,
    exited: bool,
    skip_start: f64,
    skip_end: f64,
}

fn point_on_clip_edge(a: PointPx, b: PointPx, dx: f64, dy: f64, edge: ClipEdge, rect: ClipRectPx) -> PointPx {
    match edge {
        ClipEdge::Left | ClipEdge::Right => {
            let x = match edge {
                ClipEdge::Left => rect.min_x,
                _ => rect.max_x,
            };
            let raw_y = if (x - a.x).abs() <= (x - b.x).abs() { a.y + (x - a.x) * (dy / dx) } else { b.y + (x - b.x) * (dy / dx) };
            let y = if raw_y.is_finite() { raw_y.clamp(rect.min_y, rect.max_y) } else { a.y };
            PointPx::new(x, y)
        }
        ClipEdge::Top | ClipEdge::Bottom => {
            let y = match edge {
                ClipEdge::Top => rect.min_y,
                _ => rect.max_y,
            };
            let raw_x = if (y - a.y).abs() <= (y - b.y).abs() { a.x + (y - a.y) * (dx / dy) } else { b.x + (y - b.y) * (dx / dy) };
            let x = if raw_x.is_finite() { raw_x.clamp(rect.min_x, rect.max_x) } else { a.x };
            PointPx::new(x, y)
        }
    }
}

fn clip_axis_aligned_1d(a0: f64, b0: f64, lo: f64, hi: f64) -> Option<(f64, f64, bool, bool)> {
    let d = b0 - a0;
    if d > 0.0 {
        let c0 = a0.max(lo);
        let c1 = b0.min(hi);
        if c1 - c0 <= 1e-9 {
            return None;
        }
        Some((c0, c1, c0 > a0, c1 < b0))
    } else if d < 0.0 {
        let c0 = a0.min(hi);
        let c1 = b0.max(lo);
        if c0 - c1 <= 1e-9 {
            return None;
        }
        Some((c0, c1, c0 < a0, c1 > b0))
    } else {
        None
    }
}

/// Clip segment `(a, b)` against `rect` cleanly even at extreme coordinate magnitudes.
fn clip_segment_to_rect(a: PointPx, b: PointPx, rect: ClipRectPx) -> Option<ClippedSegment> {
    if rect.is_empty() || !a.is_finite() || !b.is_finite() {
        return None;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    if !dx.is_finite() || !dy.is_finite() {
        return None;
    }
    if dy == 0.0 {
        if a.y < rect.min_y || a.y > rect.max_y {
            return None;
        }
        let (x0, x1, entered, exited) = clip_axis_aligned_1d(a.x, b.x, rect.min_x, rect.max_x)?;
        return Some(ClippedSegment {
            start: PointPx::new(x0, a.y),
            end: PointPx::new(x1, a.y),
            entered,
            exited,
            skip_start: (x0 - a.x).abs(),
            skip_end: (b.x - x1).abs(),
        });
    }
    if dx == 0.0 {
        if a.x < rect.min_x || a.x > rect.max_x {
            return None;
        }
        let (y0, y1, entered, exited) = clip_axis_aligned_1d(a.y, b.y, rect.min_y, rect.max_y)?;
        return Some(ClippedSegment {
            start: PointPx::new(a.x, y0),
            end: PointPx::new(a.x, y1),
            entered,
            exited,
            skip_start: (y0 - a.y).abs(),
            skip_end: (b.y - y1).abs(),
        });
    }

    let checks = [
        (-dx, a.x - rect.min_x, ClipEdge::Left),
        (dx, rect.max_x - a.x, ClipEdge::Right),
        (-dy, a.y - rect.min_y, ClipEdge::Top),
        (dy, rect.max_y - a.y, ClipEdge::Bottom),
    ];
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    let mut enter_edge: Option<ClipEdge> = None;
    let mut exit_edge: Option<ClipEdge> = None;

    for (p, q, edge) in checks {
        let r = q / p;
        if !r.is_finite() {
            return None;
        }
        if p < 0.0 {
            if r > t0 {
                t0 = r;
                enter_edge = Some(edge);
            }
        } else if r < t1 {
            t1 = r;
            exit_edge = Some(edge);
        }
    }

    if t0 - t1 > 1e-12 || t1 < 0.0 || t0 > 1.0 {
        return None;
    }

    let start = match enter_edge {
        Some(edge) => point_on_clip_edge(a, b, dx, dy, edge, rect),
        None => a,
    };
    let end = match exit_edge {
        Some(edge) => point_on_clip_edge(a, b, dx, dy, edge, rect),
        None => b,
    };
    if (end.x - start.x).hypot(end.y - start.y) <= 1e-6 {
        return None;
    }

    Some(ClippedSegment {
        start,
        end,
        entered: enter_edge.is_some(),
        exited: exit_edge.is_some(),
        skip_start: (start.x - a.x).hypot(start.y - a.y),
        skip_end: (b.x - end.x).hypot(b.y - end.y),
    })
}

/// Advance dash phase analytically modulo `period` without stepping individual dashes.
fn advance_dash_state(pattern: &[f64], period: f64, pat_idx: &mut usize, pat_rem: &mut f64, advance_len: f64) {
    if period <= 0.0 || !period.is_finite() || !advance_len.is_finite() || advance_len <= 0.0 || pattern.is_empty() {
        return;
    }
    let mut cur_phase = 0.0f64;
    for (i, &seg) in pattern.iter().enumerate() {
        match i.cmp(pat_idx) {
            std::cmp::Ordering::Less => cur_phase += seg,
            std::cmp::Ordering::Equal => {
                cur_phase += (seg - *pat_rem).max(0.0);
                break;
            }
            std::cmp::Ordering::Greater => break,
        }
    }
    let mut phase = (cur_phase + (advance_len % period)) % period;
    if !phase.is_finite() || phase < 0.0 || phase >= period {
        phase = 0.0;
    }
    for (i, &seg) in pattern.iter().enumerate() {
        if phase < seg - 1e-9 {
            *pat_idx = i;
            *pat_rem = (seg - phase).max(0.0);
            return;
        }
        phase = (phase - seg).max(0.0);
    }
    *pat_idx = 0;
    *pat_rem = pattern.first().copied().unwrap_or(0.25);
}

#[allow(clippy::too_many_arguments)]
fn emit_dashed_subpath_pieces(
    points: &[PointPx],
    closed: bool,
    width_px: f64,
    r: f64,
    stroke: &StrokePaint,
    clip: ClipRectPx,
    dash_segments: &mut usize,
    bufs: &mut StrokeEmitBufs<'_>,
) {
    let mults = stroke.dash.dash_multipliers();
    if mults.is_empty() || points.len() < 2 {
        emit_solid_subpath_pieces(points, closed, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
        return;
    }
    bufs.dash_pattern.clear();
    for &m in mults {
        bufs.dash_pattern.push((m * width_px).max(0.25));
    }
    let dash_period: f64 = bufs.dash_pattern.iter().copied().sum();
    let mut pat_idx = 0usize;
    let mut pat_rem = bufs.dash_pattern.first().copied().unwrap_or(width_px);
    bufs.dash_pts.clear();
    let seg_count = if closed { points.len() } else { points.len() - 1 };

    for i in 0..seg_count {
        if *dash_segments >= MAX_DASH_SEGMENTS {
            break;
        }
        let Some(&a) = points.get(i) else { continue };
        let Some(&b) = points.get((i + 1) % points.len()) else { continue };
        let Some((_, seg_len)) = unit_dir(a, b) else { continue };
        let Some(clipped) = clip_segment_to_rect(a, b, clip) else {
            if !bufs.dash_pts.is_empty() {
                emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                bufs.dash_pts.clear();
            }
            advance_dash_state(bufs.dash_pattern, dash_period, &mut pat_idx, &mut pat_rem, seg_len);
            continue;
        };
        let Some((dir, vis_len)) = unit_dir(clipped.start, clipped.end) else {
            if !bufs.dash_pts.is_empty() {
                emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                bufs.dash_pts.clear();
            }
            advance_dash_state(bufs.dash_pattern, dash_period, &mut pat_idx, &mut pat_rem, seg_len);
            continue;
        };

        if clipped.entered {
            if !bufs.dash_pts.is_empty() {
                emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                bufs.dash_pts.clear();
            }
            advance_dash_state(bufs.dash_pattern, dash_period, &mut pat_idx, &mut pat_rem, clipped.skip_start);
        }

        let mut u = 0.0f64;
        while u < vis_len - 1e-9 {
            if *dash_segments >= MAX_DASH_SEGMENTS {
                if !bufs.dash_pts.is_empty() {
                    emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                    bufs.dash_pts.clear();
                }
                return;
            }
            let take = (vis_len - u).min(pat_rem);
            let next_u = u + take;
            if next_u <= u {
                break;
            }
            *dash_segments += 1;
            let p_start = PointPx::new(clipped.start.x + dir.x * u, clipped.start.y + dir.y * u);
            let p_end = PointPx::new(clipped.start.x + dir.x * next_u, clipped.start.y + dir.y * next_u);
            if pat_idx.is_multiple_of(2) {
                if bufs.dash_pts.is_empty() {
                    bufs.dash_pts.push(p_start);
                }
                bufs.dash_pts.push(p_end);
            }
            pat_rem -= take;
            u = next_u;
            if pat_rem <= 1e-9 {
                if pat_idx.is_multiple_of(2) && !bufs.dash_pts.is_empty() {
                    emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                    bufs.dash_pts.clear();
                }
                pat_idx = (pat_idx + 1) % bufs.dash_pattern.len();
                pat_rem = bufs.dash_pattern.get(pat_idx).copied().unwrap_or(width_px);
            }
        }

        if clipped.exited {
            if !bufs.dash_pts.is_empty() {
                emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
                bufs.dash_pts.clear();
            }
            advance_dash_state(bufs.dash_pattern, dash_period, &mut pat_idx, &mut pat_rem, clipped.skip_end);
        }
    }

    if !bufs.dash_pts.is_empty() {
        emit_solid_subpath_pieces(bufs.dash_pts, false, r, stroke.cap, stroke.join, bufs.dirs, bufs.pieces);
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

fn render_text(img: &mut RasterImage, text: &PositionedText, sx: f64, sy: f64, clip: ClipRectPx, scratch: &mut RasterScratch) {
    if !text.font_size_pt.is_finite() || text.font_size_pt <= 0.0 || !text.anchor_pt.x.is_finite() || !text.anchor_pt.y.is_finite() {
        return;
    }
    let lines = split_rich_text_lines(&text.rich);
    if lines.is_empty() {
        return;
    }

    let line_step = text.font_size_pt * 1.20;
    let rad = if text.rotation_deg.is_finite() { text.rotation_deg.to_radians() } else { 0.0 };
    let (sin_a, cos_a) = rad.sin_cos();

    for (line_idx, line) in lines.iter().enumerate() {
        let y_line = (line_idx as f64) * line_step;
        let outlines = outline_rich_text(line, 0.0, y_line, text.font_size_pt);
        if outlines.is_empty() {
            continue;
        }

        let total_advance: f64 = outlines.iter().map(|g| if g.advance_x_pt.is_finite() { g.advance_x_pt } else { 0.0 }).sum();
        let anchor_dx = match text.anchor {
            TextAnchor::Start => 0.0,
            TextAnchor::Middle => -0.5 * total_advance,
            TextAnchor::End => -total_advance,
        };
        let xform = TextTransform { anchor_dx, anchor_x: text.anchor_pt.x, anchor_y: text.anchor_pt.y, cos_a, sin_a, sx, sy };

        for glyph in &outlines {
            if glyph.commands.is_empty() {
                continue;
            }
            let color = glyph.color.unwrap_or(text.color);
            if color.a == 0 {
                continue;
            }
            flatten_glyph_commands_into(&glyph.commands, xform, &mut scratch.subpaths, &mut scratch.vec_pool);
            if !scratch.subpaths.is_empty() {
                fill_subpaths_target(ScanTarget::Image { img, color }, &scratch.subpaths, clip, &mut scratch.scan);
            }
            reclaim_subpaths(&mut scratch.subpaths, &mut scratch.vec_pool);
        }
    }
}

fn flatten_glyph_commands_into(cmds: &[GlyphOutlineCmd], xform: TextTransform, subpaths: &mut Vec<Subpath>, pool: &mut Vec<Vec<PointPx>>) {
    reclaim_subpaths(subpaths, pool);
    let mut current: Option<Subpath> = None;
    let mut start_pt: Option<PointPx> = None;

    for cmd in cmds {
        match *cmd {
            GlyphOutlineCmd::MoveTo { x_pt, y_pt } => {
                if let Some(mut sub) = current.take() {
                    if !sub.points.is_empty() {
                        subpaths.push(sub);
                    } else if pool.len() < 16 {
                        sub.points.clear();
                        pool.push(sub.points);
                    }
                }
                let p = xform.map_pt(x_pt, y_pt);
                if p.is_finite() {
                    start_pt = Some(p);
                    current = Some(Subpath { points: take_point_vec(pool, p), closed: true, has_segment: false });
                } else {
                    start_pt = None;
                }
            }
            GlyphOutlineCmd::LineTo { x_pt, y_pt } => {
                let p = xform.map_pt(x_pt, y_pt);
                if p.is_finite() {
                    push_line_point(&mut current, &mut start_pt, p, pool);
                }
            }
            GlyphOutlineCmd::QuadTo { cx_pt, cy_pt, x_pt, y_pt } => {
                let c = xform.map_pt(cx_pt, cy_pt);
                let p = xform.map_pt(x_pt, y_pt);
                if c.is_finite() && p.is_finite() {
                    push_quad_points(&mut current, &mut start_pt, c, p, pool);
                }
            }
            GlyphOutlineCmd::CubicTo { c1x_pt, c1y_pt, c2x_pt, c2y_pt, x_pt, y_pt } => {
                let c1 = xform.map_pt(c1x_pt, c1y_pt);
                let c2 = xform.map_pt(c2x_pt, c2y_pt);
                let p = xform.map_pt(x_pt, y_pt);
                if c1.is_finite() && c2.is_finite() && p.is_finite() {
                    push_cubic_points(&mut current, &mut start_pt, c1, c2, p, pool);
                }
            }
            GlyphOutlineCmd::Close => {
                if let Some(mut sub) = current.take() {
                    sub.closed = true;
                    if !sub.points.is_empty() {
                        subpaths.push(sub);
                    } else if pool.len() < 16 {
                        sub.points.clear();
                        pool.push(sub.points);
                    }
                }
                if let Some(st) = start_pt {
                    current = Some(Subpath { points: take_point_vec(pool, st), closed: true, has_segment: false });
                }
            }
        }
    }

    if let Some(mut sub) = current {
        if sub.points.len() > 2 {
            subpaths.push(sub);
        } else if pool.len() < 16 {
            sub.points.clear();
            pool.push(sub.points);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curvebook_chart::{ChartScene, Path2D, PathCmd, PointPt, ScenePrimitive, StrokePaint};
    use curvebook_palette::{LineDashStyle, Rgba8};

    fn dashed_scene_for_line(x0: f64, y0: f64, x1: f64, y1: f64, dash: LineDashStyle) -> ChartScene {
        let mut scene = ChartScene::new("dash", "Dash", 200.0, 100.0, Some(Rgba8::WHITE));
        let path = Path2D::line(PointPt::new(x0, y0), PointPt::new(x1, y1));
        scene.primitives.push(ScenePrimitive::Path {
            path,
            fill: None,
            stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 2.0, dash, cap: LineCap::Butt, join: LineJoin::Miter }),
            clip: None,
        });
        scene
    }

    #[test]
    fn off_canvas_dashed_lines_render_fast_and_preserve_phase() {
        let target = RenderTargetSpec { width_px: 200, height_px: 100, dpi: 72 };
        let short_scene = dashed_scene_for_line(-26.0, 50.0, 226.0, 50.0, LineDashStyle::Dashed);
        let img_short = render_scene(&short_scene, target).expect("short render");
        assert!(img_short.rgba.as_chunks::<4>().0.iter().any(|px| px[0] < 128));

        let t0 = std::time::Instant::now();
        let huge_scene = dashed_scene_for_line(-1e16, 50.0, 1e16, 50.0, LineDashStyle::Dashed);
        let img_huge = render_scene(&huge_scene, target).expect("huge horizontal render");
        assert!(t0.elapsed().as_secs_f64() < 0.25);
        assert!(img_huge.rgba.as_chunks::<4>().0.iter().any(|px| px[0] < 128));

        let t1 = std::time::Instant::now();
        let huge_diag = dashed_scene_for_line(-1e6, -1e6 + 50.0, 1e6, 1e6 + 50.0, LineDashStyle::Dashed);
        let img_diag = render_scene(&huge_diag, target).expect("huge diagonal render");
        assert!(t1.elapsed().as_secs_f64() < 0.25);
        assert!(img_diag.rgba.as_chunks::<4>().0.iter().any(|px| px[0] < 128));

        let mut s_1e5 = dashed_scene_for_line(10.0, 10.0, 10.0 + 1e5, 10.0, LineDashStyle::Dotted);
        if let Some(ScenePrimitive::Path { stroke: Some(sp), .. }) = s_1e5.primitives.first_mut() {
            sp.width_pt = 1.25;
            sp.cap = LineCap::Round;
            sp.join = LineJoin::Round;
        }
        let mut s_1e6 = dashed_scene_for_line(10.0, 10.0, 10.0 + 1e6, 10.0, LineDashStyle::Dotted);
        if let Some(ScenePrimitive::Path { stroke: Some(sp), .. }) = s_1e6.primitives.first_mut() {
            sp.width_pt = 1.25;
            sp.cap = LineCap::Round;
            sp.join = LineJoin::Round;
        }
        let t_a = std::time::Instant::now();
        let _ = render_scene(&s_1e5, target).expect("1e5 dotted");
        let dur_a = t_a.elapsed().as_secs_f64();
        let t_b = std::time::Instant::now();
        let _ = render_scene(&s_1e6, target).expect("1e6 dotted");
        let dur_b = t_b.elapsed().as_secs_f64();
        assert!(dur_b < 0.05 + 2.0 * dur_a);

        let mut phase_pattern = vec![8.0, 5.0];
        let period: f64 = phase_pattern.iter().copied().sum();
        let mut idx_a = 0usize;
        let mut rem_a = phase_pattern[0];
        advance_dash_state(&phase_pattern, period, &mut idx_a, &mut rem_a, 10.0 * period + 3.0);
        assert_eq!(idx_a, 0);
        assert!((rem_a - 5.0).abs() < 1e-9);
        phase_pattern.clear();
        advance_dash_state(&phase_pattern, 0.0, &mut idx_a, &mut rem_a, 10.0);
    }

    #[test]
    fn off_canvas_horizontal_dashed_line_matches_exact_phase_modulo() {
        // Horizontal shift by an integer number of dash periods must produce identical visible pixels.
        let target = RenderTargetSpec { width_px: 200, height_px: 100, dpi: 72 };
        // width_pt = 2.0 at 72 dpi (scale 1.0): Dashed multipliers [4.0, 2.5] -> period = (4.0 + 2.5) * 2.0 = 13.0 px.
        let period_px = 13.0;
        let shift_periods = 100_000.0;
        let s1 = dashed_scene_for_line(-130.0, 50.0, 260.0, 50.0, LineDashStyle::Dashed);
        let s2 = dashed_scene_for_line(-130.0 - shift_periods * period_px, 50.0, 260.0 + 13_000.0, 50.0, LineDashStyle::Dashed);
        let r1 = render_scene(&s1, target).expect("r1");
        let r2 = render_scene(&s2, target).expect("r2");
        assert_eq!(r1.rgba, r2.rgba);
    }

    #[test]
    fn dense_in_canvas_dashed_polyline_hits_segment_cap_cleanly() {
        let mut scene = ChartScene::new("dense", "Dense", 200.0, 100.0, Some(Rgba8::WHITE));
        let mut commands = vec![PathCmd::MoveTo(PointPt::new(10.0, 10.0))];
        for i in 1..=200 {
            let x = if i % 2 == 0 { 10.0 } else { 190.0 };
            let y = 10.0 + f64::from(i % 80);
            commands.push(PathCmd::LineTo(PointPt::new(x, y)));
        }
        scene.primitives.push(ScenePrimitive::Path {
            path: Path2D { commands },
            fill: None,
            stroke: Some(StrokePaint { color: Rgba8::BLACK, width_pt: 0.1, dash: LineDashStyle::Dotted, cap: LineCap::Butt, join: LineJoin::Miter }),
            clip: None,
        });
        let target = RenderTargetSpec { width_px: 200, height_px: 100, dpi: 72 };
        let t0 = std::time::Instant::now();
        let img = render_scene(&scene, target).expect("dense render");
        assert!(t0.elapsed().as_secs_f64() < 1.0);
        assert!(img.rgba.as_chunks::<4>().0.iter().any(|px| px[0] < 128));
    }

    fn cubic_circle_path(cx: f64, cy: f64, r: f64, start_top: bool) -> Path2D {
        let k = CIRCLE_KAPPA * r;
        if start_top {
            Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(cx, cy - r)),
                    PathCmd::CubicTo { c1: PointPt::new(cx + k, cy - r), c2: PointPt::new(cx + r, cy - k), to: PointPt::new(cx + r, cy) },
                    PathCmd::CubicTo { c1: PointPt::new(cx + r, cy + k), c2: PointPt::new(cx + k, cy + r), to: PointPt::new(cx, cy + r) },
                    PathCmd::CubicTo { c1: PointPt::new(cx - k, cy + r), c2: PointPt::new(cx - r, cy + k), to: PointPt::new(cx - r, cy) },
                    PathCmd::CubicTo { c1: PointPt::new(cx - r, cy - k), c2: PointPt::new(cx - k, cy - r), to: PointPt::new(cx, cy - r) },
                    PathCmd::Close,
                ],
            }
        } else {
            Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(cx + r, cy)),
                    PathCmd::CubicTo { c1: PointPt::new(cx + r, cy + k), c2: PointPt::new(cx + k, cy + r), to: PointPt::new(cx, cy + r) },
                    PathCmd::CubicTo { c1: PointPt::new(cx - k, cy + r), c2: PointPt::new(cx - r, cy + k), to: PointPt::new(cx - r, cy) },
                    PathCmd::CubicTo { c1: PointPt::new(cx - r, cy - k), c2: PointPt::new(cx - k, cy - r), to: PointPt::new(cx, cy - r) },
                    PathCmd::CubicTo { c1: PointPt::new(cx + k, cy - r), c2: PointPt::new(cx + r, cy - k), to: PointPt::new(cx + r, cy) },
                    PathCmd::Close,
                ],
            }
        }
    }

    #[test]
    fn circle_stamp_cache_preserves_aa_determinism_clipping_and_non_ellipse_rejection() {
        // Both start-top (xy) and start-right (survival_cat) circles must be detected and render identically.
        let p_top = cubic_circle_path(32.3, 32.6, 4.0, true);
        let p_right = cubic_circle_path(32.3, 32.6, 4.0, false);
        assert!(try_extract_axis_aligned_ellipse(&p_top, 1.5, 1.5).is_some());
        assert!(try_extract_axis_aligned_ellipse(&p_right, 1.5, 1.5).is_some());

        // A 4-cubic non-ellipse squircle whose endpoints sit on axes must NOT trigger the ellipse fast-path.
        let squircle = Path2D {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(20.0, 10.0)),
                PathCmd::CubicTo { c1: PointPt::new(29.0, 10.0), c2: PointPt::new(30.0, 11.0), to: PointPt::new(30.0, 20.0) },
                PathCmd::CubicTo { c1: PointPt::new(30.0, 29.0), c2: PointPt::new(29.0, 30.0), to: PointPt::new(20.0, 30.0) },
                PathCmd::CubicTo { c1: PointPt::new(11.0, 30.0), c2: PointPt::new(10.0, 29.0), to: PointPt::new(10.0, 20.0) },
                PathCmd::CubicTo { c1: PointPt::new(10.0, 11.0), c2: PointPt::new(11.0, 10.0), to: PointPt::new(20.0, 10.0) },
                PathCmd::Close,
            ],
        };
        assert!(try_extract_axis_aligned_ellipse(&squircle, 1.0, 1.0).is_none());

        let target = RenderTargetSpec { width_px: 120, height_px: 120, dpi: 72 };
        let mut s_top = ChartScene::new("c-top", "Top", 120.0, 120.0, Some(Rgba8::WHITE));
        let mut s_right = ChartScene::new("c-right", "Right", 120.0, 120.0, Some(Rgba8::WHITE));
        let fill = Some(FillPaint::solid(Rgba8::rgba(0, 114, 178, 210)));
        let stroke = Some(StrokePaint::solid(Rgba8::WHITE, 0.8));
        s_top.primitives.push(ScenePrimitive::Path { path: p_top, fill, stroke, clip: None });
        s_right.primitives.push(ScenePrimitive::Path { path: p_right, fill, stroke, clip: None });
        let img_top = render_scene(&s_top, target).expect("top circle");
        let img_right = render_scene(&s_right, target).expect("right circle");
        assert_eq!(img_top.rgba, img_right.rgba);

        // Edge & clip-straddling circles over transparent background (`None`) must respect clips and blend cleanly.
        let mut clip_scene = ChartScene::new("c-clip", "Clip", 80.0, 80.0, None);
        let clip_rect = RectPt::new(10.0, 10.0, 30.0, 30.0);
        clip_scene.primitives.push(ScenePrimitive::Path {
            path: cubic_circle_path(10.0, 25.0, 5.0, false),
            fill: Some(FillPaint::solid(Rgba8::rgba(200, 40, 40, 128))),
            stroke: Some(StrokePaint::solid(Rgba8::BLACK, 1.0)),
            clip: Some(clip_rect),
        });
        clip_scene.primitives.push(ScenePrimitive::Path {
            path: cubic_circle_path(0.5, 0.5, 4.0, true),
            fill: Some(FillPaint::solid(Rgba8::rgb(20, 120, 200))),
            stroke: None,
            clip: None,
        });
        let img_clip_a = render_scene(&clip_scene, RenderTargetSpec { width_px: 80, height_px: 80, dpi: 72 }).expect("clip a");
        let img_clip_b = render_scene(&clip_scene, RenderTargetSpec { width_px: 80, height_px: 80, dpi: 72 }).expect("clip b");
        assert_eq!(img_clip_a.rgba, img_clip_b.rgba);
        for y in 15..=35 {
            for x in 0..=9 {
                let idx = (y * 80 + x) * 4;
                assert_eq!(&img_clip_a.rgba[idx..idx + 4], &[0, 0, 0, 0], "pixel ({x},{y}) leaked left of clip");
            }
        }

        // Verify crisp AA edges (intermediate blended pixels) and solid center on filled+stroked circle.
        let center_idx = (32 * 120 + 32) * 4;
        let center_px = &img_top.rgba[center_idx..center_idx + 4];
        assert_ne!(center_px, &[255, 255, 255, 255], "circle center must not stay white");
        let aa_edges = img_top.rgba.as_chunks::<4>().0.iter().filter(|px| px[0] > 0 && px[0] < 255 && px[2] > 0 && px[2] < 255).count();
        assert!(aa_edges >= 8, "expected smooth AA rim on stamped circle, got {aa_edges}");

        // Subpixel axis-aligned rect fast path must match general scanline polygon coverage exactly.
        let mut fast_rect_scene = ChartScene::new("rect-fast", "RectFast", 60.0, 60.0, Some(Rgba8::WHITE));
        fast_rect_scene.primitives.push(ScenePrimitive::Path {
            path: Path2D::rect(RectPt::new(10.25, 10.6, 15.5, 12.25)),
            fill: Some(FillPaint::solid(Rgba8::rgba(20, 110, 190, 180))),
            stroke: None,
            clip: Some(RectPt::new(12.1, 11.2, 12.4, 10.5)),
        });
        let img_rect_fast = render_scene(&fast_rect_scene, RenderTargetSpec { width_px: 60, height_px: 60, dpi: 72 }).expect("fast rect");
        let mut ref_img = RasterImage::new(60, 60, 72, Rgba8::WHITE).expect("ref img");
        let mut ref_scratch = RasterScratch::new(true);
        flatten_path_into(&Path2D::rect(RectPt::new(10.25, 10.6, 15.5, 12.25)), 1.0, 1.0, &mut ref_scratch.subpaths, &mut ref_scratch.vec_pool);
        fill_subpaths_target(
            ScanTarget::Image { img: &mut ref_img, color: Rgba8::rgba(20, 110, 190, 180) },
            &ref_scratch.subpaths,
            ClipRectPx::from_rect_pt(RectPt::new(12.1, 11.2, 12.4, 10.5), 1.0, 1.0).expect("clip"),
            &mut ref_scratch.scan,
        );
        assert_eq!(img_rect_fast.rgba, ref_img.rgba, "fast rect path must match scanline fill bit-for-bit");
    }

    #[test]
    fn small_poly_marker_stamps_and_cache_overflow_fallback_remain_deterministic() {
        // Test repeated diamond/triangle/cross markers plus > MAX_CACHED_BAKED_STAMPS unique colors.
        let mut scene = ChartScene::new("poly-markers", "Markers", 400.0, 200.0, Some(Rgba8::WHITE));
        for i in 0..1_200u32 {
            let x = 10.0 + f64::from(i % 75) * 5.0;
            let y = 10.0 + f64::from(i / 75) * 10.0;
            let r = 2.25;
            let diamond = Path2D {
                commands: vec![
                    PathCmd::MoveTo(PointPt::new(x, y - r)),
                    PathCmd::LineTo(PointPt::new(x + r, y)),
                    PathCmd::LineTo(PointPt::new(x, y + r)),
                    PathCmd::LineTo(PointPt::new(x - r, y)),
                    PathCmd::Close,
                ],
            };
            let color = Rgba8::rgb((i % 251) as u8, ((i * 7) % 251) as u8, ((i * 13) % 251) as u8);
            scene.primitives.push(ScenePrimitive::Path {
                path: diamond,
                fill: Some(FillPaint::solid(color)),
                stroke: Some(StrokePaint::solid(Rgba8::BLACK, 0.75)),
                clip: Some(RectPt::new(4.0, 4.0, 392.0, 192.0)),
            });
            let circle = cubic_circle_path(x + 1.5, y + 1.5, 2.0, false);
            scene.primitives.push(ScenePrimitive::Path {
                path: circle,
                fill: Some(FillPaint::solid(color)),
                stroke: Some(StrokePaint::solid(Rgba8::WHITE, 0.5)),
                clip: Some(RectPt::new(4.0, 4.0, 392.0, 192.0)),
            });
        }
        let target = RenderTargetSpec { width_px: 400, height_px: 200, dpi: 72 };
        let img_1 = render_scene(&scene, target).expect("render 1");
        let img_2 = render_scene(&scene, target).expect("render 2");
        assert_eq!(img_1.rgba, img_2.rgba);
        let non_white = img_1.rgba.as_chunks::<4>().0.iter().filter(|px| *px != &[255, 255, 255, 255]).count();
        assert!(non_white > 10_000, "expected rasterized marker ink, got {non_white}");
    }
}
