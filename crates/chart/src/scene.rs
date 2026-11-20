//! Device-independent vector scene intermediate representation (`ChartScene`) emitted by
//! `curvebook-chart` and consumed by SVG, PDF, CPU raster, and canvas backends.

use std::fmt;

use curvebook_palette::{LineDashStyle, Rgba8};
use curvebook_text::{RichText, effective_span_scale, measure_text_pt, metrics::split_rich_text_lines};
use serde::{Deserialize, Serialize};

/// Two-dimensional point in PostScript points (`1 pt = 1/72 inch`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PointPt {
    pub x: f64,
    pub y: f64,
}

impl PointPt {
    /// Construct a point in PostScript points.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Axis-aligned rectangle in PostScript points (`1 pt = 1/72 inch`).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RectPt {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl RectPt {
    /// Construct a rectangle with non-negative extents in PostScript points.
    #[must_use]
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x: if x.is_finite() { x } else { 0.0 },
            y: if y.is_finite() { y } else { 0.0 },
            width: if width.is_finite() { width.max(0.0) } else { 0.0 },
            height: if height.is_finite() { height.max(0.0) } else { 0.0 },
        }
    }

    /// Right-edge X coordinate (`x + width`).
    #[must_use]
    pub fn right(self) -> f64 {
        self.x + self.width
    }

    /// Bottom-edge Y coordinate (`y + height`).
    #[must_use]
    pub fn bottom(self) -> f64 {
        self.y + self.height
    }

    /// Geometric center point of the rectangle.
    #[must_use]
    pub fn center(self) -> PointPt {
        PointPt::new(self.x + 0.5 * self.width, self.y + 0.5 * self.height)
    }
}

impl From<curvebook_doc::RectPt> for RectPt {
    fn from(rect: curvebook_doc::RectPt) -> Self {
        Self::new(rect.x, rect.y, rect.width, rect.height)
    }
}

impl From<RectPt> for curvebook_doc::RectPt {
    fn from(rect: RectPt) -> Self {
        Self::new(rect.x, rect.y, rect.width, rect.height)
    }
}

/// Single vector subpath command in a [`Path2D`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    MoveTo(PointPt),
    LineTo(PointPt),
    QuadTo { control: PointPt, to: PointPt },
    CubicTo { c1: PointPt, c2: PointPt, to: PointPt },
    Close,
}

/// Sequence of vector path commands representing a closed or open shape.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Path2D {
    pub commands: Vec<PathCmd>,
}

impl Path2D {
    /// Empty path with no commands.
    #[must_use]
    pub const fn new() -> Self {
        Self { commands: Vec::new() }
    }

    /// Two-point straight line segment.
    #[must_use]
    pub fn line(from: PointPt, to: PointPt) -> Self {
        Self { commands: vec![PathCmd::MoveTo(from), PathCmd::LineTo(to)] }
    }

    /// Closed axis-aligned rectangle path.
    #[must_use]
    pub fn rect(r: RectPt) -> Self {
        Self {
            commands: vec![
                PathCmd::MoveTo(PointPt::new(r.x, r.y)),
                PathCmd::LineTo(PointPt::new(r.right(), r.y)),
                PathCmd::LineTo(PointPt::new(r.right(), r.bottom())),
                PathCmd::LineTo(PointPt::new(r.x, r.bottom())),
                PathCmd::Close,
            ],
        }
    }

    /// True if the path contains no commands.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

/// Fill style attached to a vector [`ScenePrimitive::Path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillPaint {
    pub color: Rgba8,
}

impl FillPaint {
    /// Construct a solid RGBA fill.
    #[must_use]
    pub const fn solid(color: Rgba8) -> Self {
        Self { color }
    }
}

/// Stroke endpoint cap style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// Stroke corner join style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Stroke style attached to a vector [`ScenePrimitive::Path`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrokePaint {
    pub color: Rgba8,
    pub width_pt: f64,
    pub dash: LineDashStyle,
    pub cap: LineCap,
    pub join: LineJoin,
}

impl StrokePaint {
    /// Construct a solid stroke with square/butt caps and miter joins.
    #[must_use]
    pub const fn solid(color: Rgba8, width_pt: f64) -> Self {
        Self { color, width_pt, dash: LineDashStyle::Solid, cap: LineCap::Butt, join: LineJoin::Miter }
    }
}

impl Default for StrokePaint {
    fn default() -> Self {
        Self::solid(Rgba8::BLACK, 1.0)
    }
}

/// Horizontal text alignment anchor relative to [`PositionedText::anchor_pt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextAnchor {
    #[default]
    Start,
    Middle,
    End,
}

/// Positioned rich-text label in PostScript point coordinates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionedText {
    pub rich: RichText,
    pub anchor_pt: PointPt,
    pub anchor: TextAnchor,
    pub font_size_pt: f64,
    pub rotation_deg: f64,
    pub color: Rgba8,
}

/// Individual drawing operation inside a compiled [`ChartScene`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScenePrimitive {
    Path { path: Path2D, fill: Option<FillPaint>, stroke: Option<StrokePaint>, clip: Option<RectPt> },
    Text(PositionedText),
    PushClip { rect: RectPt },
    PopClip,
}

/// Non-fatal compilation warning emitted alongside a compiled scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartWarning {
    pub code: String,
    pub message: String,
}

impl ChartWarning {
    /// Construct a structured compiler warning.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }
}

/// Failure returned when compiling a graph or layout specification into a scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartError {
    SheetNotFound { sheet_id: String },
    InvalidSpec { message: String },
    ResourceLimit { message: String },
    CompileFailed { message: String },
}

impl fmt::Display for ChartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SheetNotFound { sheet_id } => write!(f, "sheet not found: {sheet_id}"),
            Self::InvalidSpec { message } => write!(f, "invalid graph or layout spec: {message}"),
            Self::ResourceLimit { message } => write!(f, "chart resource limit exceeded: {message}"),
            Self::CompileFailed { message } => write!(f, "chart compilation failed: {message}"),
        }
    }
}

impl std::error::Error for ChartError {}

pub use curvebook_doc::SourceCellId;

/// Interactive metadata for an individual plotted marker/point inside [`ChartScene`] (`QOL-5`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlottedPointMeta {
    pub center_pt: PointPt,
    pub radius_pt: f64,
    pub source_cell: SourceCellId,
    pub series_label: String,
    pub value_summary: String,
}

/// Publication preflight diagnostic finding emitted by [`preflight_check_scene`] (`QOL-8`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightIssue {
    pub code: String,
    pub severity: String,
    pub message: String,
}

/// Complete compiled figure or layout scene ready for SVG, PDF, raster, and canvas rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartScene {
    pub sheet_id: String,
    pub title: String,
    pub width_pt: f64,
    pub height_pt: f64,
    pub background: Option<Rgba8>,
    pub plot_rect_pt: RectPt,
    pub primitives: Vec<ScenePrimitive>,
    pub warnings: Vec<ChartWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plotted_points: Vec<PlottedPointMeta>,
}

impl ChartScene {
    /// Create an empty scene container with optional background fill primitive.
    #[must_use]
    pub fn new(sheet_id: impl Into<String>, title: impl Into<String>, width_pt: f64, height_pt: f64, background: Option<Rgba8>) -> Self {
        let w = if width_pt.is_finite() { width_pt.max(1.0) } else { 360.0 };
        let h = if height_pt.is_finite() { height_pt.max(1.0) } else { 260.0 };
        let plot_rect_pt = RectPt::new((w * 0.14).min(48.0), (h * 0.12).min(36.0), (w * 0.78).max(1.0), (h * 0.72).max(1.0));
        let mut primitives = Vec::new();
        if let Some(bg) = background {
            primitives.push(ScenePrimitive::Path {
                path: Path2D::rect(RectPt::new(0.0, 0.0, w, h)),
                fill: Some(FillPaint::solid(bg)),
                stroke: None,
                clip: None,
            });
        }
        Self {
            sheet_id: sheet_id.into(),
            title: title.into(),
            width_pt: w,
            height_pt: h,
            background,
            plot_rect_pt,
            primitives,
            warnings: Vec::new(),
            plotted_points: Vec::new(),
        }
    }

    /// Hit-test individual plotted points in scene coordinates (`pt` in PostScript points) (`QOL-5`).
    #[must_use]
    pub fn hit_test_point(&self, pt: PointPt, tolerance_pt: f64) -> Option<&PlottedPointMeta> {
        let tol = if tolerance_pt.is_finite() { tolerance_pt.max(0.0) } else { 0.0 };
        let mut best: Option<(f64, &PlottedPointMeta)> = None;
        for meta in &self.plotted_points {
            let dist = (meta.center_pt.x - pt.x).hypot(meta.center_pt.y - pt.y);
            let max_r = meta.radius_pt.max(tol);
            if dist <= max_r && best.is_none_or(|(best_d, _)| dist < best_d) {
                best = Some((dist, meta));
            }
        }
        best.map(|(_, m)| m)
    }
}

fn intersect_clip_rect(a: RectPt, b: RectPt) -> RectPt {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = a.right().min(b.right());
    let y1 = a.bottom().min(b.bottom());
    RectPt::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Compute axis-aligned bounding box of a [`PositionedText`] in PostScript points.
#[must_use]
pub(crate) fn text_primitive_bounds_pt(t: &PositionedText) -> Option<RectPt> {
    if !t.anchor_pt.x.is_finite() || !t.anchor_pt.y.is_finite() || !t.font_size_pt.is_finite() || t.font_size_pt <= 0.0 {
        return None;
    }
    let lines = split_rich_text_lines(&t.rich);
    let line_step = t.font_size_pt * 1.20;
    let rad = if t.rotation_deg.is_finite() { t.rotation_deg.to_radians() } else { 0.0 };
    let (sin_a, cos_a) = rad.sin_cos();

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for (line_idx, line) in lines.iter().enumerate() {
        if line.plain_text().trim().is_empty() {
            continue;
        }
        let m = measure_text_pt(line, t.font_size_pt);
        let anchor_dx = match t.anchor {
            TextAnchor::Start => 0.0,
            TextAnchor::Middle => -0.5 * m.width_pt,
            TextAnchor::End => -m.width_pt,
        };
        let baseline_y = (line_idx as f64) * line_step;
        let x0 = anchor_dx;
        let x1 = anchor_dx + m.width_pt;
        let y0 = baseline_y - m.ascent_pt;
        let y1 = baseline_y + m.descent_pt;

        for lx in [x0, x1] {
            for ly in [y0, y1] {
                let wx = t.anchor_pt.x + lx * cos_a - ly * sin_a;
                let wy = t.anchor_pt.y + lx * sin_a + ly * cos_a;
                min_x = min_x.min(wx);
                max_x = max_x.max(wx);
                min_y = min_y.min(wy);
                max_y = max_y.max(wy);
            }
        }
    }

    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some(RectPt::new(min_x, min_y, (max_x - min_x).max(0.0), (max_y - min_y).max(0.0)))
    } else {
        None
    }
}

/// Inspect a compiled [`ChartScene`] for typography or stroke violations at print dimensions (`QOL-8`).
#[must_use]
pub fn preflight_check_scene(scene: &ChartScene) -> Vec<PreflightIssue> {
    let mut issues = Vec::new();
    let mut min_font_found: Option<f64> = None;
    let mut min_stroke_found: Option<f64> = None;
    let mut clipped_text_count = 0_usize;
    let mut clip_stack: Vec<RectPt> = Vec::new();

    for prim in &scene.primitives {
        match prim {
            ScenePrimitive::PushClip { rect } => {
                let effective = clip_stack.last().copied().map_or(*rect, |prev| intersect_clip_rect(prev, *rect));
                clip_stack.push(effective);
            }
            ScenePrimitive::PopClip => {
                clip_stack.pop();
            }
            ScenePrimitive::Text(t) if !t.rich.plain_text().trim().is_empty() => {
                if t.font_size_pt > 0.0 {
                    if t.font_size_pt < 5.0 {
                        min_font_found = Some(min_font_found.map_or(t.font_size_pt, |m| m.min(t.font_size_pt)));
                    }
                    for span in &t.rich.spans {
                        if !span.text.trim().is_empty() {
                            let eff_pt = t.font_size_pt * effective_span_scale(span);
                            if eff_pt > 0.0 && eff_pt < 5.0 {
                                min_font_found = Some(min_font_found.map_or(eff_pt, |m| m.min(eff_pt)));
                            }
                        }
                    }
                }
                let anchor_outside =
                    t.anchor_pt.x < -2.0 || t.anchor_pt.y < -2.0 || t.anchor_pt.x > scene.width_pt + 2.0 || t.anchor_pt.y > scene.height_pt + 2.0;
                let bbox_clipped = text_primitive_bounds_pt(t).is_some_and(|tb| {
                    let outside_canvas = tb.x < -2.0 || tb.y < -2.0 || tb.right() > scene.width_pt + 2.0 || tb.bottom() > scene.height_pt + 2.0;
                    let outside_clip = clip_stack
                        .last()
                        .is_some_and(|c| tb.x < c.x - 1.5 || tb.y < c.y - 1.5 || tb.right() > c.right() + 1.5 || tb.bottom() > c.bottom() + 1.5);
                    outside_canvas || outside_clip
                });
                if anchor_outside || bbox_clipped {
                    clipped_text_count += 1;
                }
            }
            ScenePrimitive::Path { stroke: Some(s), .. } if s.width_pt > 0.0 && s.width_pt < 0.25 => {
                min_stroke_found = Some(min_stroke_found.map_or(s.width_pt, |m| m.min(s.width_pt)));
            }
            _ => {}
        }
    }

    if let Some(sz) = min_font_found {
        issues.push(PreflightIssue {
            code: String::from("min_font_size"),
            severity: String::from("warn"),
            message: format!("visible text size {sz:.2} pt is below recommended 5.0 pt journal print minimum"),
        });
    }
    if let Some(w) = min_stroke_found {
        issues.push(PreflightIssue {
            code: String::from("hairline_stroke"),
            severity: String::from("warn"),
            message: format!("stroke width {w:.3} pt is below recommended 0.25 pt hairline minimum"),
        });
    }
    if clipped_text_count > 0 {
        issues.push(PreflightIssue {
            code: String::from("text_outside_canvas"),
            severity: String::from("warn"),
            message: format!(
                "{clipped_text_count} label(s) clip or extend outside canvas/clip bounds ({:.1}x{:.1} pt)",
                scene.width_pt, scene.height_pt
            ),
        });
    }
    issues
}
