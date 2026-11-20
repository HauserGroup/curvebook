//! Pure-Rust clean-room ECMA-234 Windows Enhanced Metafile (`.emf`) binary encoder.
//!
//! Emits little-endian 4-byte-aligned Enhanced Metafile records with exact header byte counts,
//! record counts, 0.01 mm `rclFrame` bounds, `MM_ANISOTROPIC` coordinate mapping, styled vector
//! paths (lines and `EMR_POLYBEZIERTO` cubic segments), clip stacks, Unicode `EMR_EXTTEXTOUTW`
//! labels, optional bottom-up BGRA `EMR_STRETCHDIBITS` bitmaps, and clean `EMR_DELETEOBJECT`
//! handle lifecycle management.

use curvebook_chart::{
    ChartScene, FillPaint, LineCap, LineJoin, Path2D, PathCmd, PointPt, PositionedText, RectPt, ScenePrimitive, SceneRasterImage, StrokePaint,
    TextAnchor,
};
use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_text::{
    RichSpan, RichText, SpanSlope, SpanWeight, effective_span_scale, measure_text_pt,
    metrics::{char_advance_pt, split_rich_text_lines},
    script_baseline_offset_pt,
};

use crate::rasterizer::{MAX_RASTER_DIM_PX, MAX_RASTER_PIXELS, RasterImage, RenderError};

/// Scale factor from PostScript points (`1/72` in) into integer EMF logical twips (`1/20` pt).
const LOGICAL_SCALE: f64 = 20.0;

/// Reference display resolution in dots per inch used for EMF device metrics.
const REF_DPI: f64 = 96.0;

/// Exact device pixels along each axis of the 100-inch reference device (`100 * 96`).
const REF_DEVICE_PX: i32 = 9_600;

/// Exact millimeters along each axis of the 100-inch reference device (`100 * 25.4`).
const REF_DEVICE_MM: i32 = 2_540;

// ECMA-234 / Enhanced Metafile record identifiers (`iType`).
const EMR_HEADER: u32 = 1;
const EMR_POLYBEZIERTO: u32 = 5;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETWINDOWORGEX: u32 = 10;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_SETVIEWPORTORGEX: u32 = 12;
const EMR_EOF: u32 = 14;
const EMR_SETMAPMODE: u32 = 17;
const EMR_SETBKMODE: u32 = 18;
const EMR_SETPOLYFILLMODE: u32 = 19;
const EMR_SETTEXTALIGN: u32 = 22;
const EMR_SETTEXTCOLOR: u32 = 24;
const EMR_MOVETOEX: u32 = 27;
const EMR_INTERSECTCLIPRECT: u32 = 30;
const EMR_SAVEDC: u32 = 33;
const EMR_RESTOREDC: u32 = 34;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_DELETEOBJECT: u32 = 40;
const EMR_LINETO: u32 = 54;
const EMR_BEGINPATH: u32 = 59;
const EMR_ENDPATH: u32 = 60;
const EMR_CLOSEFIGURE: u32 = 61;
const EMR_FILLPATH: u32 = 62;
const EMR_STROKEANDFILLPATH: u32 = 63;
const EMR_STROKEPATH: u32 = 64;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_EXTCREATEPEN: u32 = 95;

// Header / mapping / drawing mode constants.
const ENHMETA_SIGNATURE: u32 = 0x464D_4520;
const ENHMETA_VERSION: u32 = 0x0001_0000;
const EMR_HEADER_BASE_BYTES: usize = 88;
const MM_ANISOTROPIC: u32 = 8;
const BKMODE_TRANSPARENT: u32 = 1;
const POLYFILL_WINDING: u32 = 2;
const TA_LEFT_BASELINE: u32 = 24; // TA_LEFT (0) | TA_BASELINE (24)
const GM_COMPATIBLE: u32 = 1;

// Stock object indices (MS-EMF / GDI `ENHMETA_STOCK_OBJECT` flag `0x8000_0000`).
const STOCK_NULL_BRUSH: u32 = 0x8000_0005;
const STOCK_BLACK_PEN: u32 = 0x8000_0007;
const STOCK_NULL_PEN: u32 = 0x8000_0008;
const STOCK_ANSI_VAR_FONT: u32 = 0x8000_000C;

// Pen / brush / DIB constants.
const PS_SOLID: u32 = 0;
const PS_USERSTYLE: u32 = 7;
const PS_ENDCAP_ROUND: u32 = 0x0000_0000;
const PS_ENDCAP_SQUARE: u32 = 0x0000_0100;
const PS_ENDCAP_FLAT: u32 = 0x0000_0200;
const PS_JOIN_ROUND: u32 = 0x0000_0000;
const PS_JOIN_BEVEL: u32 = 0x0000_1000;
const PS_JOIN_MITER: u32 = 0x0000_2000;
const PS_GEOMETRIC: u32 = 0x0001_0000;
const BS_SOLID: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const SRCCOPY: u32 = 0x00CC_0020;
const BI_RGB: u32 = 0;

/// Encode a compiled [`ChartScene`] into an ECMA-234 Enhanced Metafile (`.emf`) byte stream.
///
/// # Errors
/// Returns [`RenderError::InvalidTarget`] when scene dimensions are non-positive or non-finite, or
/// if encoded size exceeds the 32-bit EMF `nBytes` limit.
pub fn encode_emf(scene: &ChartScene) -> Result<Vec<u8>, RenderError> {
    encode_emf_with_raster(scene, None)
}

/// Encode a [`RasterImage`] into an ECMA-234 `.emf` wrapper containing a bottom-up BGRA
/// `EMR_STRETCHDIBITS` bitmap record.
///
/// # Errors
/// Returns [`RenderError::InvalidTarget`] on invalid target geometry or
/// [`RenderError::BufferSizeMismatch`] if `image.rgba` length does not match `width * height * 4`.
pub fn encode_raster_emf(image: &RasterImage) -> Result<Vec<u8>, RenderError> {
    let dpi = f64::from(image.dpi.max(1));
    let width_pt = f64::from(image.width_px) * 72.0 / dpi;
    let height_pt = f64::from(image.height_px) * 72.0 / dpi;
    let scene = ChartScene::new(String::from("raster"), String::from("raster"), width_pt, height_pt, None);
    let dest_rect = RectPt::new(0.0, 0.0, width_pt, height_pt);
    encode_emf_with_raster(&scene, Some((image, dest_rect)))
}

/// Encode a [`ChartScene`] together with an optional raster layer emitted as a bottom-up BGRA
/// `EMR_STRETCHDIBITS` record.
///
/// # Errors
/// Returns [`RenderError`] if dimensions/coordinates are invalid, buffer sizes mismatch, or output
/// size exceeds 32-bit EMF bounds.
pub fn encode_emf_with_raster(scene: &ChartScene, raster: Option<(&RasterImage, RectPt)>) -> Result<Vec<u8>, RenderError> {
    let metrics = SceneMetrics::from_scene(scene)?;
    let mut writer = EmfWriter::new(metrics, scene.background.unwrap_or(Rgba8::WHITE))?;

    writer.emit_setup_records()?;
    if let Some(bg) = scene.background
        && bg.a > 0
    {
        let bg_rect = Path2D::rect(RectPt::new(0.0, 0.0, scene.width_pt, scene.height_pt));
        let already_present = matches!(
            scene.primitives.first(),
            Some(ScenePrimitive::Path { path, fill: Some(_), stroke: None, clip: None }) if *path == bg_rect
        );
        if !already_present {
            writer.emit_path(&bg_rect, Some(FillPaint::solid(bg)), None, None)?;
        }
    }
    if let Some((image, dest_rect)) = raster {
        writer.emit_stretch_dibits(image, dest_rect)?;
    }

    for primitive in &scene.primitives {
        writer.emit_primitive(primitive)?;
    }
    writer.restore_unclosed_clips()?;
    writer.emit_eof()?;
    writer.finalize()
}

#[derive(Debug, Clone, Copy)]
struct SceneMetrics {
    win_w: i32,
    win_h: i32,
    vp_w: i32,
    vp_h: i32,
    frame_w_hmm: i32,
    frame_h_hmm: i32,
}

impl SceneMetrics {
    fn from_scene(scene: &ChartScene) -> Result<Self, RenderError> {
        if !scene.width_pt.is_finite() || !scene.height_pt.is_finite() || scene.width_pt <= 0.0 || scene.height_pt <= 0.0 {
            return Err(RenderError::InvalidTarget {
                message: format!("EMF scene dimensions must be positive and finite (got {}x{})", scene.width_pt, scene.height_pt),
            });
        }
        let max_pt = (i32::MAX as f64) / (LOGICAL_SCALE * 2.0);
        if scene.width_pt > max_pt || scene.height_pt > max_pt {
            return Err(RenderError::InvalidTarget {
                message: format!("EMF scene dimensions {}x{} pt exceed coordinate range", scene.width_pt, scene.height_pt),
            });
        }
        let win_w = scale_logical(scene.width_pt).max(1);
        let win_h = scale_logical(scene.height_pt).max(1);
        let vp_w = pt_to_dev_px(scene.width_pt).max(1);
        let vp_h = pt_to_dev_px(scene.height_pt).max(1);
        let frame_w_hmm = pt_to_himetric(scene.width_pt).max(1);
        let frame_h_hmm = pt_to_himetric(scene.height_pt).max(1);
        Ok(Self { win_w, win_h, vp_w, vp_h, frame_w_hmm, frame_h_hmm })
    }
}

#[derive(Debug, Clone, Copy)]
struct BoundsRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    empty: bool,
}

impl BoundsRect {
    const fn empty() -> Self {
        Self { left: 0, top: 0, right: 0, bottom: 0, empty: true }
    }

    fn include_pt(&mut self, x_dev: i32, y_dev: i32) {
        if self.empty {
            self.left = x_dev;
            self.top = y_dev;
            self.right = x_dev;
            self.bottom = y_dev;
            self.empty = false;
        } else {
            self.left = self.left.min(x_dev);
            self.top = self.top.min(y_dev);
            self.right = self.right.max(x_dev);
            self.bottom = self.bottom.max(y_dev);
        }
    }

    fn include_rect(&mut self, other: Self) {
        if !other.empty {
            self.include_pt(other.left, other.top);
            self.include_pt(other.right, other.bottom);
        }
    }

    const fn as_ltrb(self, fallback_w: i32, fallback_h: i32) -> [i32; 4] {
        if self.empty { [0, 0, fallback_w, fallback_h] } else { [self.left, self.top, self.right, self.bottom] }
    }
}

struct HandlePool {
    free_slots: Vec<u32>,
    next_slot: u32,
    max_slot_used: u16,
}

impl HandlePool {
    const fn new() -> Self {
        // Handle slot 0 is reserved by the ECMA-234 Enhanced Metafile specification.
        Self { free_slots: Vec::new(), next_slot: 1, max_slot_used: 0 }
    }

    fn alloc(&mut self) -> Result<u32, RenderError> {
        let slot = if let Some(reused) = self.free_slots.pop() {
            reused
        } else {
            let s = self.next_slot;
            self.next_slot =
                self.next_slot.checked_add(1).ok_or_else(|| RenderError::EncodingFailed { message: String::from("EMF handle table overflow") })?;
            s
        };
        let slot_u16 = u16::try_from(slot).map_err(|_| RenderError::EncodingFailed { message: String::from("EMF handle count exceeds u16") })?;
        if slot_u16 > self.max_slot_used {
            self.max_slot_used = slot_u16;
        }
        Ok(slot)
    }

    fn free(&mut self, slot: u32) {
        self.free_slots.push(slot);
    }

    const fn n_handles(&self) -> u16 {
        self.max_slot_used.saturating_add(1)
    }
}

struct EmfWriter {
    buf: Vec<u8>,
    n_records: u32,
    metrics: SceneMetrics,
    bg_color: Rgba8,
    handles: HandlePool,
    bounds: BoundsRect,
    clip_depth: usize,
}

impl EmfWriter {
    fn new(metrics: SceneMetrics, bg_color: Rgba8) -> Result<Self, RenderError> {
        let mut buf = Vec::with_capacity(4096);
        buf.resize(EMR_HEADER_BASE_BYTES, 0);
        // Header counts as record #1; finalized once all records and bounds are known.
        let mut writer = Self { buf, n_records: 1, metrics, bg_color, handles: HandlePool::new(), bounds: BoundsRect::empty(), clip_depth: 0 };
        writer.patch_header(0, EMR_HEADER_BASE_BYTES as u32, 1, 1)?;
        Ok(writer)
    }

    fn emit_setup_records(&mut self) -> Result<(), RenderError> {
        self.emit_u32_record(EMR_SETMAPMODE, &[MM_ANISOTROPIC])?;
        self.emit_i32_pair_record(EMR_SETWINDOWORGEX, 0, 0)?;
        self.emit_i32_pair_record(EMR_SETWINDOWEXTEX, self.metrics.win_w, self.metrics.win_h)?;
        self.emit_i32_pair_record(EMR_SETVIEWPORTORGEX, 0, 0)?;
        self.emit_i32_pair_record(EMR_SETVIEWPORTEXTEX, self.metrics.vp_w, self.metrics.vp_h)?;
        self.emit_u32_record(EMR_SETBKMODE, &[BKMODE_TRANSPARENT])?;
        self.emit_u32_record(EMR_SETPOLYFILLMODE, &[POLYFILL_WINDING])?;
        self.emit_u32_record(EMR_SETTEXTALIGN, &[TA_LEFT_BASELINE])?;
        Ok(())
    }

    fn emit_primitive(&mut self, primitive: &ScenePrimitive) -> Result<(), RenderError> {
        match primitive {
            ScenePrimitive::Path { path, fill, stroke, clip } => self.emit_path(path, *fill, *stroke, *clip),
            ScenePrimitive::Text(text) => self.emit_text(text),
            ScenePrimitive::RasterImage(raster) => self.emit_raster_image(raster),
            ScenePrimitive::PushClip { rect } => self.push_clip(*rect),
            ScenePrimitive::PopClip => self.pop_clip(),
        }
    }

    fn push_clip(&mut self, rect: RectPt) -> Result<(), RenderError> {
        if !rect_is_finite(rect) {
            return Ok(());
        }
        self.emit_empty_record(EMR_SAVEDC)?;
        self.emit_rect_record(EMR_INTERSECTCLIPRECT, rect_to_logical(rect))?;
        self.clip_depth += 1;
        Ok(())
    }

    fn pop_clip(&mut self) -> Result<(), RenderError> {
        if self.clip_depth > 0 {
            self.emit_i32_record(EMR_RESTOREDC, -1)?;
            self.clip_depth -= 1;
        }
        Ok(())
    }

    fn restore_unclosed_clips(&mut self) -> Result<(), RenderError> {
        while self.clip_depth > 0 {
            self.pop_clip()?;
        }
        Ok(())
    }

    fn emit_path(&mut self, path: &Path2D, fill: Option<FillPaint>, stroke: Option<StrokePaint>, clip: Option<RectPt>) -> Result<(), RenderError> {
        if path.commands.is_empty() {
            return Ok(());
        }
        let active_fill = fill.filter(|f| f.color.a > 0);
        let active_stroke = stroke.filter(|s| s.color.a > 0 && s.width_pt.is_finite() && s.width_pt > 0.0);
        if active_fill.is_none() && active_stroke.is_none() {
            return Ok(());
        }

        let mut pushed_local_clip = false;
        if let Some(clip_rect) = clip
            && rect_is_finite(clip_rect)
        {
            self.push_clip(clip_rect)?;
            pushed_local_clip = true;
        }

        let brush_handle = if let Some(fill_paint) = active_fill {
            let color = blend_over_bg(fill_paint.color, self.bg_color);
            let h = self.create_solid_brush(color)?;
            self.select_object(h)?;
            Some(h)
        } else {
            self.select_object(STOCK_NULL_BRUSH)?;
            None
        };

        let pen_handle = if let Some(stroke_paint) = active_stroke {
            let h = self.create_pen(stroke_paint)?;
            self.select_object(h)?;
            Some(h)
        } else {
            self.select_object(STOCK_NULL_PEN)?;
            None
        };

        if let Some(path_bounds) = self.emit_path_geometry(path, active_stroke.map(|s| s.width_pt))? {
            self.bounds.include_rect(path_bounds);
            let action = match (brush_handle.is_some(), pen_handle.is_some()) {
                (true, true) => EMR_STROKEANDFILLPATH,
                (true, false) => EMR_FILLPATH,
                (false, true) => EMR_STROKEPATH,
                (false, false) => 0,
            };
            if action != 0 {
                let b = path_bounds.as_ltrb(self.metrics.vp_w, self.metrics.vp_h);
                self.emit_rect_record(action, b)?;
            }
        }

        if let Some(h) = pen_handle {
            self.select_object(STOCK_BLACK_PEN)?;
            self.delete_object(h)?;
        }
        if let Some(h) = brush_handle {
            self.select_object(STOCK_NULL_BRUSH)?;
            self.delete_object(h)?;
        }
        if pushed_local_clip {
            self.pop_clip()?;
        }
        Ok(())
    }

    fn emit_path_geometry(&mut self, path: &Path2D, stroke_width_pt: Option<f64>) -> Result<Option<BoundsRect>, RenderError> {
        let mut dev_bounds = BoundsRect::empty();
        let mut cur_pt: Option<PointPt> = None;
        let mut subpath_start: Option<PointPt> = None;
        let mut pending_moveto: Option<PointPt> = None;
        let mut bezier_start: Option<PointPt> = None;
        let mut pending_beziers: Vec<PointPt> = Vec::new();
        let mut has_segments = false;
        let mut began = false;

        let commit_moveto_if_pending =
            |writer: &mut Self, began: &mut bool, pending: &mut Option<PointPt>, bounds: &mut BoundsRect| -> Result<(), RenderError> {
                if let Some(mpt) = pending.take() {
                    if !*began {
                        writer.emit_empty_record(EMR_BEGINPATH)?;
                        *began = true;
                    }
                    let (lx, ly) = point_to_logical(mpt);
                    writer.emit_i32_pair_record(EMR_MOVETOEX, lx, ly)?;
                    bounds.include_pt(pt_to_dev_px(mpt.x), pt_to_dev_px(mpt.y));
                } else if !*began {
                    writer.emit_empty_record(EMR_BEGINPATH)?;
                    *began = true;
                }
                Ok(())
            };

        for cmd in &path.commands {
            match *cmd {
                PathCmd::MoveTo(pt) if point_is_finite(pt) => {
                    if !pending_beziers.is_empty() {
                        commit_moveto_if_pending(self, &mut began, &mut pending_moveto, &mut dev_bounds)?;
                        self.flush_polybezier_to(bezier_start.take(), &mut pending_beziers, &mut dev_bounds)?;
                        has_segments = true;
                    }
                    pending_moveto = Some(pt);
                    cur_pt = Some(pt);
                    subpath_start = Some(pt);
                }
                PathCmd::LineTo(to) if point_is_finite(to) => {
                    if cur_pt.is_none() {
                        continue;
                    }
                    if !pending_beziers.is_empty() {
                        commit_moveto_if_pending(self, &mut began, &mut pending_moveto, &mut dev_bounds)?;
                        self.flush_polybezier_to(bezier_start.take(), &mut pending_beziers, &mut dev_bounds)?;
                    }
                    commit_moveto_if_pending(self, &mut began, &mut pending_moveto, &mut dev_bounds)?;
                    let (lx, ly) = point_to_logical(to);
                    self.emit_i32_pair_record(EMR_LINETO, lx, ly)?;
                    dev_bounds.include_pt(pt_to_dev_px(to.x), pt_to_dev_px(to.y));
                    cur_pt = Some(to);
                    has_segments = true;
                }
                PathCmd::QuadTo { control, to } if point_is_finite(control) && point_is_finite(to) => {
                    let Some(p0) = cur_pt else {
                        continue;
                    };
                    // Exact quadratic-to-cubic Bezier degree elevation.
                    let c1 = PointPt::new(p0.x + (2.0 / 3.0) * (control.x - p0.x), p0.y + (2.0 / 3.0) * (control.y - p0.y));
                    let c2 = PointPt::new(to.x + (2.0 / 3.0) * (control.x - to.x), to.y + (2.0 / 3.0) * (control.y - to.y));
                    if pending_beziers.is_empty() {
                        bezier_start = Some(p0);
                    }
                    pending_beziers.extend_from_slice(&[c1, c2, to]);
                    cur_pt = Some(to);
                }
                PathCmd::CubicTo { c1, c2, to } if point_is_finite(c1) && point_is_finite(c2) && point_is_finite(to) => {
                    let Some(p0) = cur_pt else {
                        continue;
                    };
                    if pending_beziers.is_empty() {
                        bezier_start = Some(p0);
                    }
                    pending_beziers.extend_from_slice(&[c1, c2, to]);
                    cur_pt = Some(to);
                }
                PathCmd::Close => {
                    if !pending_beziers.is_empty() {
                        commit_moveto_if_pending(self, &mut began, &mut pending_moveto, &mut dev_bounds)?;
                        self.flush_polybezier_to(bezier_start.take(), &mut pending_beziers, &mut dev_bounds)?;
                        has_segments = true;
                    }
                    if began && pending_moveto.is_none() && subpath_start.is_some() {
                        self.emit_empty_record(EMR_CLOSEFIGURE)?;
                        cur_pt = subpath_start;
                        has_segments = true;
                    }
                }
                _ => {}
            }
        }

        if !pending_beziers.is_empty() {
            commit_moveto_if_pending(self, &mut began, &mut pending_moveto, &mut dev_bounds)?;
            self.flush_polybezier_to(bezier_start.take(), &mut pending_beziers, &mut dev_bounds)?;
            has_segments = true;
        }
        if !began || !has_segments {
            return Ok(None);
        }
        self.emit_empty_record(EMR_ENDPATH)?;

        if let Some(w_pt) = stroke_width_pt
            && !dev_bounds.empty
        {
            let pad_dev = pt_to_dev_px(w_pt * 0.5).max(1);
            dev_bounds.left = dev_bounds.left.saturating_sub(pad_dev);
            dev_bounds.top = dev_bounds.top.saturating_sub(pad_dev);
            dev_bounds.right = dev_bounds.right.saturating_add(pad_dev);
            dev_bounds.bottom = dev_bounds.bottom.saturating_add(pad_dev);
        }
        Ok(Some(dev_bounds))
    }

    fn flush_polybezier_to(&mut self, start_pt: Option<PointPt>, pts: &mut Vec<PointPt>, dev_bounds: &mut BoundsRect) -> Result<(), RenderError> {
        if pts.len() < 3 || !pts.len().is_multiple_of(3) {
            pts.clear();
            return Ok(());
        }
        let mut poly_bounds = BoundsRect::empty();
        if let Some(s) = start_pt {
            let dx = pt_to_dev_px(s.x);
            let dy = pt_to_dev_px(s.y);
            poly_bounds.include_pt(dx, dy);
            dev_bounds.include_pt(dx, dy);
        }
        for pt in pts.iter() {
            let dx = pt_to_dev_px(pt.x);
            let dy = pt_to_dev_px(pt.y);
            poly_bounds.include_pt(dx, dy);
            dev_bounds.include_pt(dx, dy);
        }
        let rcl = poly_bounds.as_ltrb(self.metrics.vp_w, self.metrics.vp_h);
        let cptl =
            u32::try_from(pts.len()).map_err(|_| RenderError::EncodingFailed { message: String::from("EMR_POLYBEZIERTO point count overflow") })?;
        let mut payload = Vec::with_capacity(20 + pts.len() * 8);
        push_rect_bytes(&mut payload, rcl);
        payload.extend_from_slice(&cptl.to_le_bytes());
        for pt in pts.iter() {
            let (lx, ly) = point_to_logical(*pt);
            payload.extend_from_slice(&lx.to_le_bytes());
            payload.extend_from_slice(&ly.to_le_bytes());
        }
        pts.clear();
        self.emit_record(EMR_POLYBEZIERTO, &payload)
    }

    fn emit_text(&mut self, text: &PositionedText) -> Result<(), RenderError> {
        if !point_is_finite(text.anchor_pt) || !text.font_size_pt.is_finite() || text.font_size_pt <= 0.0 || text.rich.spans.is_empty() {
            return Ok(());
        }
        let lines = split_rich_text_lines(&text.rich);
        if lines.is_empty() {
            return Ok(());
        }
        let line_step = text.font_size_pt * 1.20;
        let rad = if text.rotation_deg.is_finite() { text.rotation_deg.to_radians() } else { 0.0 };
        let (sin_a, cos_a) = rad.sin_cos();
        let escapement_tenths = if text.rotation_deg.is_finite() { (-text.rotation_deg * 10.0).round() as i32 } else { 0 };

        for (line_idx, line) in lines.iter().enumerate() {
            if line.spans.is_empty() {
                continue;
            }
            let total_w = measure_text_pt(line, text.font_size_pt).width_pt;
            let anchor_dx = match text.anchor {
                TextAnchor::Start => 0.0,
                TextAnchor::Middle => -0.5 * total_w,
                TextAnchor::End => -total_w,
            };
            let line_y = (line_idx as f64) * line_step;
            let mut cursor_x = anchor_dx;
            for span in &line.spans {
                if span.text.is_empty() {
                    continue;
                }
                let span_size_pt = text.font_size_pt * effective_span_scale(span);
                let baseline_dy = line_y + script_baseline_offset_pt(span.script, text.font_size_pt);
                let span_w = measure_text_pt(&RichText::new(vec![span.clone()]), text.font_size_pt).width_pt;
                let raw_color = span.color.unwrap_or(text.color);
                if raw_color.a > 0 && span_size_pt > 0.0 {
                    let x_pt = text.anchor_pt.x + cursor_x * cos_a - baseline_dy * sin_a;
                    let y_pt = text.anchor_pt.y + cursor_x * sin_a + baseline_dy * cos_a;
                    let color = blend_over_bg(raw_color, self.bg_color);
                    self.emit_text_span(span, PointPt::new(x_pt, y_pt), span_size_pt, rad, escapement_tenths, color)?;
                }
                cursor_x += span_w;
            }
        }
        Ok(())
    }

    fn emit_text_span(
        &mut self,
        span: &RichSpan,
        origin_pt: PointPt,
        size_pt: f64,
        rad: f64,
        escapement_tenths: i32,
        color: Rgba8,
    ) -> Result<(), RenderError> {
        let mut utf16_units: Vec<u16> = Vec::new();
        let mut dx_logical: Vec<u32> = Vec::new();
        let mut total_adv_pt = 0.0_f64;

        for ch in span.text.chars() {
            if ch == '\n' || ch == '\r' {
                continue;
            }
            let adv_pt = char_advance_pt(ch, size_pt, span.weight, span.slope);
            let adv_log = scale_logical(adv_pt.max(0.0)).max(0) as u32;
            let mut u16_buf = [0u16; 2];
            let encoded = ch.encode_utf16(&mut u16_buf);
            if let Some(&first) = encoded.first() {
                utf16_units.push(first);
                dx_logical.push(adv_log);
            }
            if let Some(&second) = encoded.get(1) {
                utf16_units.push(second);
                dx_logical.push(0);
            }
            total_adv_pt += adv_pt.max(0.0);
        }
        if utf16_units.is_empty() {
            return Ok(());
        }

        let family = span.font_family.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or("Arial");
        let font_handle = self.create_font(family, size_pt, escapement_tenths, span.weight, span.slope)?;
        self.select_object(font_handle)?;
        self.emit_u32_record(EMR_SETTEXTCOLOR, &[colorref(color)])?;

        let (lx, ly) = point_to_logical(origin_pt);
        let (sin_a, cos_a) = rad.sin_cos();
        let mut span_bounds = BoundsRect::empty();
        for u in [0.0_f64, total_adv_pt] {
            for v in [-size_pt, 0.25 * size_pt] {
                let px = origin_pt.x + u * cos_a - v * sin_a;
                let py = origin_pt.y + u * sin_a + v * cos_a;
                span_bounds.include_pt(pt_to_dev_px(px), pt_to_dev_px(py));
            }
        }
        self.bounds.include_rect(span_bounds);

        let n_chars =
            u32::try_from(utf16_units.len()).map_err(|_| RenderError::EncodingFailed { message: String::from("EMF text length overflow") })?;
        let str_bytes = utf16_units.len() * 2;
        let padded_str_bytes = (str_bytes + 3) & !3;
        let off_string: u32 = 76; // 28 (EMR_EXTTEXTOUTW fields before EMRTEXT) + 40 (EMRTEXT) + 8 (EMR header)
        let off_dx = off_string + padded_str_bytes as u32;
        let ex_scale = (2540.0_f64 / (72.0 * LOGICAL_SCALE)) as f32;

        let mut payload = Vec::with_capacity((off_dx as usize) + dx_logical.len() * 4 - 8);
        push_rect_bytes(&mut payload, span_bounds.as_ltrb(self.metrics.vp_w, self.metrics.vp_h));
        payload.extend_from_slice(&GM_COMPATIBLE.to_le_bytes());
        payload.extend_from_slice(&ex_scale.to_le_bytes());
        payload.extend_from_slice(&ex_scale.to_le_bytes());
        // EMRTEXT structure:
        payload.extend_from_slice(&lx.to_le_bytes());
        payload.extend_from_slice(&ly.to_le_bytes());
        payload.extend_from_slice(&n_chars.to_le_bytes());
        payload.extend_from_slice(&off_string.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // fOptions
        push_rect_bytes(&mut payload, [0, 0, -1, -1]); // rcl (unused when fOptions == 0)
        payload.extend_from_slice(&off_dx.to_le_bytes());
        for unit in &utf16_units {
            payload.extend_from_slice(&unit.to_le_bytes());
        }
        payload.resize(payload.len() + (padded_str_bytes - str_bytes), 0);
        for dx in &dx_logical {
            payload.extend_from_slice(&dx.to_le_bytes());
        }

        self.emit_record(EMR_EXTTEXTOUTW, &payload)?;
        self.select_object(STOCK_ANSI_VAR_FONT)?;
        self.delete_object(font_handle)?;
        Ok(())
    }

    fn emit_stretch_dibits(&mut self, image: &RasterImage, dest_rect: RectPt) -> Result<(), RenderError> {
        if image.width_px == 0
            || image.height_px == 0
            || image.width_px > MAX_RASTER_DIM_PX
            || image.height_px > MAX_RASTER_DIM_PX
            || u64::from(image.width_px).saturating_mul(u64::from(image.height_px)) > MAX_RASTER_PIXELS
        {
            return Err(RenderError::InvalidTarget {
                message: format!("invalid raster dimensions {}x{} for EMF DIB", image.width_px, image.height_px),
            });
        }
        let expected_len = image
            .expected_byte_len()
            .ok_or_else(|| RenderError::InvalidTarget { message: String::from("raster dimensions overflow byte length") })?;
        if image.rgba.len() != expected_len {
            return Err(RenderError::BufferSizeMismatch { expected: expected_len, actual: image.rgba.len() });
        }
        if !rect_is_finite(dest_rect) || dest_rect.width <= 0.0 || dest_rect.height <= 0.0 {
            return Err(RenderError::InvalidTarget { message: String::from("EMF raster destination rectangle must be positive and finite") });
        }

        let [x0, y0, x1, y1] = rect_to_logical(dest_rect);
        let cx_dest = x1.saturating_sub(x0).max(1);
        let cy_dest = y1.saturating_sub(y0).max(1);
        let cx_src = i32::try_from(image.width_px).map_err(|_| RenderError::InvalidTarget { message: String::from("raster width exceeds i32") })?;
        let cy_src = i32::try_from(image.height_px).map_err(|_| RenderError::InvalidTarget { message: String::from("raster height exceeds i32") })?;
        let cb_bits = u32::try_from(expected_len)
            .map_err(|_| RenderError::InvalidTarget { message: String::from("raster byte size exceeds EMF 32-bit DIB") })?;

        let mut dev_bounds = BoundsRect::empty();
        dev_bounds.include_pt(pt_to_dev_px(dest_rect.x), pt_to_dev_px(dest_rect.y));
        dev_bounds.include_pt(pt_to_dev_px(dest_rect.x + dest_rect.width), pt_to_dev_px(dest_rect.y + dest_rect.height));
        self.bounds.include_rect(dev_bounds);

        let off_bmi: u32 = 80;
        let cb_bmi: u32 = 40;
        let off_bits: u32 = off_bmi + cb_bmi;
        let ppm = ((f64::from(image.dpi.max(1)) / 25.4) * 1000.0).round() as i32;

        let mut payload = Vec::with_capacity(72 + 40 + expected_len);
        push_rect_bytes(&mut payload, dev_bounds.as_ltrb(self.metrics.vp_w, self.metrics.vp_h));
        payload.extend_from_slice(&x0.to_le_bytes()); // xDest
        payload.extend_from_slice(&y0.to_le_bytes()); // yDest
        payload.extend_from_slice(&0i32.to_le_bytes()); // xSrc
        payload.extend_from_slice(&0i32.to_le_bytes()); // ySrc
        payload.extend_from_slice(&cx_src.to_le_bytes()); // cxSrc
        payload.extend_from_slice(&cy_src.to_le_bytes()); // cySrc
        payload.extend_from_slice(&off_bmi.to_le_bytes());
        payload.extend_from_slice(&cb_bmi.to_le_bytes());
        payload.extend_from_slice(&off_bits.to_le_bytes());
        payload.extend_from_slice(&cb_bits.to_le_bytes());
        payload.extend_from_slice(&DIB_RGB_COLORS.to_le_bytes());
        payload.extend_from_slice(&SRCCOPY.to_le_bytes());
        payload.extend_from_slice(&cx_dest.to_le_bytes());
        payload.extend_from_slice(&cy_dest.to_le_bytes());

        // BITMAPINFOHEADER (40 bytes): positive biHeight encodes bottom-up BGRA scanlines.
        payload.extend_from_slice(&cb_bmi.to_le_bytes()); // biSize
        payload.extend_from_slice(&cx_src.to_le_bytes()); // biWidth
        payload.extend_from_slice(&cy_src.to_le_bytes()); // biHeight (+ = bottom-up)
        payload.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
        payload.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
        payload.extend_from_slice(&BI_RGB.to_le_bytes()); // biCompression
        payload.extend_from_slice(&cb_bits.to_le_bytes()); // biSizeImage
        payload.extend_from_slice(&ppm.to_le_bytes()); // biXPelsPerMeter
        payload.extend_from_slice(&ppm.to_le_bytes()); // biYPelsPerMeter
        payload.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
        payload.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

        let row_bytes = (image.width_px as usize) * 4;
        for row in (0..image.height_px as usize).rev() {
            let start = row * row_bytes;
            let row_slice = image
                .rgba
                .get(start..start + row_bytes)
                .ok_or(RenderError::BufferSizeMismatch { expected: expected_len, actual: image.rgba.len() })?;
            for &[r, g, b, a] in row_slice.as_chunks::<4>().0 {
                payload.extend_from_slice(&[b, g, r, a]);
            }
        }
        self.emit_record(EMR_STRETCHDIBITS, &payload)
    }

    fn emit_raster_image(&mut self, prim: &SceneRasterImage) -> Result<(), RenderError> {
        let (src_w, src_h) = prim.pixel_size;
        let expected_len = (src_w as usize).checked_mul(src_h as usize).and_then(|p| p.checked_mul(4));
        if src_w == 0
            || src_h == 0
            || expected_len != Some(prim.rgba.len())
            || !rect_is_finite(prim.bounds_pt)
            || prim.bounds_pt.width <= 0.0
            || prim.bounds_pt.height <= 0.0
            || !prim.rotation_deg.is_finite()
        {
            return Ok(());
        }
        let Some(vis_doc) = prim.crop.visible_rect(curvebook_doc::RectPt::from(prim.bounds_pt)) else {
            return Ok(());
        };
        let vis = RectPt::from(vis_doc);
        let bounds = prim.bounds_pt;
        let cx = bounds.x + 0.5 * bounds.width;
        let cy = bounds.y + 0.5 * bounds.height;
        let (sin_t, cos_t) = prim.rotation_deg.to_radians().sin_cos();

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for vx in [vis.x, vis.right()] {
            for vy in [vis.y, vis.bottom()] {
                let dx = vx - cx;
                let dy = vy - cy;
                let wx = cx + dx * cos_t - dy * sin_t;
                let wy = cy + dx * sin_t + dy * cos_t;
                min_x = min_x.min(wx);
                max_x = max_x.max(wx);
                min_y = min_y.min(wy);
                max_y = max_y.max(wy);
            }
        }
        let aabb = RectPt::new(min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0));
        if !rect_is_finite(aabb) || aabb.width <= 0.0 || aabb.height <= 0.0 {
            return Ok(());
        }

        let nat_scale = (f64::from(src_w) / bounds.width).max(f64::from(src_h) / bounds.height).max(REF_DPI / 72.0);
        let max_scale = (2048.0 / aabb.width.max(1e-3)).min(2048.0 / aabb.height.max(1e-3));
        let scale = nat_scale.min(max_scale).max(1e-3);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bw = (aabb.width * scale).ceil().clamp(1.0, 2048.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bh = (aabb.height * scale).ceil().clamp(1.0, 2048.0) as u32;
        let sx = f64::from(bw) / aabb.width;
        let sy = f64::from(bh) / aabb.height;

        let opaque_bg = if self.bg_color.a == u8::MAX {
            self.bg_color
        } else if self.bg_color.a > 0 {
            Rgba8::rgb(self.bg_color.r, self.bg_color.g, self.bg_color.b)
        } else {
            Rgba8::WHITE
        };
        let mut baked = RasterImage::new(bw, bh, 96, opaque_bg)?;
        let mut shifted = prim.clone();
        shifted.bounds_pt.x -= aabb.x;
        shifted.bounds_pt.y -= aabb.y;
        shifted.clip = None;
        crate::rasterizer::render_raster_image(&mut baked, &shifted, sx, sy, crate::rasterizer::ClipRectPx::from_canvas(bw, bh));

        let mut pushed_clip = false;
        if let Some(clip_rect) = prim.clip
            && rect_is_finite(clip_rect)
        {
            self.push_clip(clip_rect)?;
            pushed_clip = true;
        }
        let res = self.emit_stretch_dibits(&baked, aabb);
        if pushed_clip {
            self.pop_clip()?;
        }
        res
    }

    fn create_solid_brush(&mut self, color: Rgba8) -> Result<u32, RenderError> {
        let handle = self.handles.alloc()?;
        let mut payload = Vec::with_capacity(16);
        payload.extend_from_slice(&handle.to_le_bytes());
        payload.extend_from_slice(&BS_SOLID.to_le_bytes());
        payload.extend_from_slice(&colorref(color).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // lbHatch
        self.emit_record(EMR_CREATEBRUSHINDIRECT, &payload)?;
        Ok(handle)
    }

    fn create_pen(&mut self, stroke: StrokePaint) -> Result<u32, RenderError> {
        let handle = self.handles.alloc()?;
        let width_log = scale_logical(stroke.width_pt).max(1);
        let color = blend_over_bg(stroke.color, self.bg_color);
        let use_ext = stroke.dash != LineDashStyle::Solid || stroke.cap != LineCap::Butt || stroke.join != LineJoin::Miter;

        if !use_ext {
            let mut payload = Vec::with_capacity(20);
            payload.extend_from_slice(&handle.to_le_bytes());
            payload.extend_from_slice(&PS_SOLID.to_le_bytes());
            payload.extend_from_slice(&width_log.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&colorref(color).to_le_bytes());
            self.emit_record(EMR_CREATEPEN, &payload)?;
            return Ok(handle);
        }

        let cap_bits = match stroke.cap {
            LineCap::Butt => PS_ENDCAP_FLAT,
            LineCap::Round => PS_ENDCAP_ROUND,
            LineCap::Square => PS_ENDCAP_SQUARE,
        };
        let join_bits = match stroke.join {
            LineJoin::Miter => PS_JOIN_MITER,
            LineJoin::Round => PS_JOIN_ROUND,
            LineJoin::Bevel => PS_JOIN_BEVEL,
        };
        let dash_mults = stroke.dash.dash_multipliers();
        let style_bits = if dash_mults.is_empty() { PS_SOLID } else { PS_USERSTYLE };
        let pen_style = PS_GEOMETRIC | style_bits | cap_bits | join_bits;

        let style_entries: Vec<u32> = dash_mults.iter().map(|m| ((m * f64::from(width_log)).round() as i64).max(1) as u32).collect();
        let n_entries = u32::try_from(style_entries.len()).unwrap_or(0);

        let mut payload = Vec::with_capacity(48 + style_entries.len().max(1) * 4);
        payload.extend_from_slice(&handle.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // offBmi
        payload.extend_from_slice(&0u32.to_le_bytes()); // cbBmi
        payload.extend_from_slice(&0u32.to_le_bytes()); // offBits
        payload.extend_from_slice(&0u32.to_le_bytes()); // cbBits
        // EXTLOGPEN fields:
        payload.extend_from_slice(&pen_style.to_le_bytes());
        payload.extend_from_slice(&(width_log as u32).to_le_bytes());
        payload.extend_from_slice(&BS_SOLID.to_le_bytes());
        payload.extend_from_slice(&colorref(color).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // elpHatch
        payload.extend_from_slice(&n_entries.to_le_bytes());
        if style_entries.is_empty() {
            payload.extend_from_slice(&0u32.to_le_bytes()); // trailing StyleEntry[0] padding per Win32 EXTLOGPEN
        } else {
            for entry in &style_entries {
                payload.extend_from_slice(&entry.to_le_bytes());
            }
        }
        self.emit_record(EMR_EXTCREATEPEN, &payload)?;
        Ok(handle)
    }

    fn create_font(&mut self, family: &str, size_pt: f64, escapement_tenths: i32, weight: SpanWeight, slope: SpanSlope) -> Result<u32, RenderError> {
        let handle = self.handles.alloc()?;
        let lf_height = -scale_logical(size_pt).max(1);
        let lf_weight: i32 = match weight {
            SpanWeight::Regular => 400,
            SpanWeight::Bold => 700,
        };
        let lf_italic: u8 = u8::from(slope == SpanSlope::Italic);

        // Emit full 332-byte `EMR_EXTCREATEFONTINDIRECTW` (`EXTLOGFONTW` = 320 bytes).
        let mut payload = Vec::with_capacity(324);
        payload.extend_from_slice(&handle.to_le_bytes());
        // LOGFONTW (92 bytes):
        payload.extend_from_slice(&lf_height.to_le_bytes());
        payload.extend_from_slice(&0i32.to_le_bytes()); // lfWidth
        payload.extend_from_slice(&escapement_tenths.to_le_bytes());
        payload.extend_from_slice(&escapement_tenths.to_le_bytes()); // lfOrientation
        payload.extend_from_slice(&lf_weight.to_le_bytes());
        payload.push(lf_italic);
        payload.push(0); // lfUnderline
        payload.push(0); // lfStrikeOut
        payload.push(1); // DEFAULT_CHARSET
        payload.push(0); // OUT_DEFAULT_PRECIS
        payload.push(0); // CLIP_DEFAULT_PRECIS
        payload.push(4); // ANTIALIASED_QUALITY
        payload.push(0x22); // VARIABLE_PITCH | FF_SWISS
        push_utf16_fixed(&mut payload, family, 32); // lfFaceName[32] (64 bytes)
        // EXTLOGFONTW extension (228 bytes):
        push_utf16_fixed(&mut payload, family, 64); // elfFullName[64] (128 bytes)
        let style_name = match (weight, slope) {
            (SpanWeight::Regular, SpanSlope::Upright) => "Regular",
            (SpanWeight::Bold, SpanSlope::Upright) => "Bold",
            (SpanWeight::Regular, SpanSlope::Italic) => "Italic",
            (SpanWeight::Bold, SpanSlope::Italic) => "Bold Italic",
        };
        push_utf16_fixed(&mut payload, style_name, 32); // elfStyle[32] (64 bytes)
        payload.resize(payload.len() + 36, 0); // elfVersion..elfPanose + 2-byte struct alignment
        self.emit_record(EMR_EXTCREATEFONTINDIRECTW, &payload)?;
        Ok(handle)
    }

    fn select_object(&mut self, handle: u32) -> Result<(), RenderError> {
        self.emit_u32_record(EMR_SELECTOBJECT, &[handle])
    }

    fn delete_object(&mut self, handle: u32) -> Result<(), RenderError> {
        self.emit_u32_record(EMR_DELETEOBJECT, &[handle])?;
        self.handles.free(handle);
        Ok(())
    }

    fn emit_eof(&mut self) -> Result<(), RenderError> {
        // EMR_EOF (20 bytes): iType=14, nSize=20, nPalEntries=0, offPalEntries=16, nSizeLast=20.
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&16u32.to_le_bytes());
        payload.extend_from_slice(&20u32.to_le_bytes());
        self.emit_record(EMR_EOF, &payload)
    }

    fn finalize(mut self) -> Result<Vec<u8>, RenderError> {
        let n_bytes =
            u32::try_from(self.buf.len()).map_err(|_| RenderError::EncodingFailed { message: String::from("EMF stream exceeds 4 GiB limit") })?;
        self.patch_header(0, n_bytes, self.n_records, self.handles.n_handles())?;
        Ok(self.buf)
    }

    fn patch_header(&mut self, _reserved: usize, n_bytes: u32, n_records: u32, n_handles: u16) -> Result<(), RenderError> {
        let rcl_bounds = self.bounds.as_ltrb(self.metrics.vp_w, self.metrics.vp_h);
        let rcl_frame = [0, 0, self.metrics.frame_w_hmm, self.metrics.frame_h_hmm];
        let mut hdr = Vec::with_capacity(EMR_HEADER_BASE_BYTES);
        hdr.extend_from_slice(&EMR_HEADER.to_le_bytes());
        hdr.extend_from_slice(&(EMR_HEADER_BASE_BYTES as u32).to_le_bytes());
        push_rect_bytes(&mut hdr, rcl_bounds);
        push_rect_bytes(&mut hdr, rcl_frame);
        hdr.extend_from_slice(&ENHMETA_SIGNATURE.to_le_bytes());
        hdr.extend_from_slice(&ENHMETA_VERSION.to_le_bytes());
        hdr.extend_from_slice(&n_bytes.to_le_bytes());
        hdr.extend_from_slice(&n_records.to_le_bytes());
        hdr.extend_from_slice(&n_handles.to_le_bytes());
        hdr.extend_from_slice(&0u16.to_le_bytes()); // sReserved
        hdr.extend_from_slice(&0u32.to_le_bytes()); // nDescription
        hdr.extend_from_slice(&0u32.to_le_bytes()); // offDescription
        hdr.extend_from_slice(&0u32.to_le_bytes()); // nPalEntries
        hdr.extend_from_slice(&REF_DEVICE_PX.to_le_bytes());
        hdr.extend_from_slice(&REF_DEVICE_PX.to_le_bytes());
        hdr.extend_from_slice(&REF_DEVICE_MM.to_le_bytes());
        hdr.extend_from_slice(&REF_DEVICE_MM.to_le_bytes());

        let dst = self
            .buf
            .get_mut(..EMR_HEADER_BASE_BYTES)
            .ok_or_else(|| RenderError::EncodingFailed { message: String::from("EMF header buffer truncated") })?;
        dst.copy_from_slice(&hdr);
        Ok(())
    }

    fn emit_empty_record(&mut self, i_type: u32) -> Result<(), RenderError> {
        self.emit_record(i_type, &[])
    }

    fn emit_u32_record(&mut self, i_type: u32, words: &[u32]) -> Result<(), RenderError> {
        let mut payload = Vec::with_capacity(words.len() * 4);
        for word in words {
            payload.extend_from_slice(&word.to_le_bytes());
        }
        self.emit_record(i_type, &payload)
    }

    fn emit_i32_record(&mut self, i_type: u32, val: i32) -> Result<(), RenderError> {
        self.emit_record(i_type, &val.to_le_bytes())
    }

    fn emit_i32_pair_record(&mut self, i_type: u32, a: i32, b: i32) -> Result<(), RenderError> {
        let mut payload = Vec::with_capacity(8);
        payload.extend_from_slice(&a.to_le_bytes());
        payload.extend_from_slice(&b.to_le_bytes());
        self.emit_record(i_type, &payload)
    }

    fn emit_rect_record(&mut self, i_type: u32, rect: [i32; 4]) -> Result<(), RenderError> {
        let mut payload = Vec::with_capacity(16);
        push_rect_bytes(&mut payload, rect);
        self.emit_record(i_type, &payload)
    }

    fn emit_record(&mut self, i_type: u32, payload: &[u8]) -> Result<(), RenderError> {
        let padded_payload = (payload.len() + 3) & !3;
        let record_len = 8usize.saturating_add(padded_payload);
        let n_size = u32::try_from(record_len).map_err(|_| RenderError::EncodingFailed { message: String::from("EMF record exceeds u32 length") })?;
        self.buf.extend_from_slice(&i_type.to_le_bytes());
        self.buf.extend_from_slice(&n_size.to_le_bytes());
        self.buf.extend_from_slice(payload);
        self.buf.resize(self.buf.len() + (padded_payload - payload.len()), 0);
        self.n_records =
            self.n_records.checked_add(1).ok_or_else(|| RenderError::EncodingFailed { message: String::from("EMF record count overflow") })?;
        Ok(())
    }
}

fn push_rect_bytes(out: &mut Vec<u8>, ltrb: [i32; 4]) {
    for coord in ltrb {
        out.extend_from_slice(&coord.to_le_bytes());
    }
}

fn push_utf16_fixed(out: &mut Vec<u8>, text: &str, max_units: usize) {
    let mut written = 0usize;
    for unit in text.encode_utf16() {
        if written + 1 >= max_units {
            break;
        }
        out.extend_from_slice(&unit.to_le_bytes());
        written += 1;
    }
    while written < max_units {
        out.extend_from_slice(&0u16.to_le_bytes());
        written += 1;
    }
}

fn scale_logical(val_pt: f64) -> i32 {
    if !val_pt.is_finite() {
        return 0;
    }
    let scaled = (val_pt * LOGICAL_SCALE).round();
    scaled.clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32
}

fn point_to_logical(pt: PointPt) -> (i32, i32) {
    (scale_logical(pt.x), scale_logical(pt.y))
}

fn rect_to_logical(rect: RectPt) -> [i32; 4] {
    let x0 = rect.x.min(rect.x + rect.width);
    let y0 = rect.y.min(rect.y + rect.height);
    let x1 = rect.x.max(rect.x + rect.width);
    let y1 = rect.y.max(rect.y + rect.height);
    [scale_logical(x0), scale_logical(y0), scale_logical(x1), scale_logical(y1)]
}

fn pt_to_dev_px(pt: f64) -> i32 {
    if !pt.is_finite() {
        return 0;
    }
    let dev = (pt * REF_DPI / 72.0).round();
    dev.clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32
}

fn pt_to_himetric(pt: f64) -> i32 {
    if !pt.is_finite() {
        return 0;
    }
    let hmm = (pt * 2540.0 / 72.0).round();
    hmm.clamp(f64::from(i32::MIN / 2), f64::from(i32::MAX / 2)) as i32
}

const fn point_is_finite(pt: PointPt) -> bool {
    pt.x.is_finite() && pt.y.is_finite()
}

const fn rect_is_finite(rect: RectPt) -> bool {
    rect.x.is_finite() && rect.y.is_finite() && rect.width.is_finite() && rect.height.is_finite()
}

const fn colorref(c: Rgba8) -> u32 {
    ((c.b as u32) << 16) | ((c.g as u32) << 8) | (c.r as u32)
}

fn blend_over_bg(fg: Rgba8, bg: Rgba8) -> Rgba8 {
    if fg.a == 255 || bg.a == 0 {
        return fg;
    }
    let alpha = u16::from(fg.a);
    let inv = 255 - alpha;
    let r = ((u16::from(fg.r) * alpha + u16::from(bg.r) * inv + 127) / 255) as u8;
    let g = ((u16::from(fg.g) * alpha + u16::from(bg.g) * inv + 127) / 255) as u8;
    let b = ((u16::from(fg.b) * alpha + u16::from(bg.b) * inv + 127) / 255) as u8;
    Rgba8 { r, g, b, a: 255 }
}
