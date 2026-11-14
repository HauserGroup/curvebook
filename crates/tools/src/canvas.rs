//! Graph and layout canvas interaction state machines emitting declarative engine command intents.

use lumencraft_chart::{ChartScene, PointPt, RectPt};
use lumencraft_doc::{
    AxisSpec, BracketLabelMode, BracketStyle, GraphAnnotation, GraphSheet, GraphSpec, LayoutItem, LayoutSpec, PairwiseBracketSpec,
    RectPt as DocRectPt, SheetFolderKind, SheetId,
};
use lumencraft_palette::{LineDashStyle, Rgba8};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Cursor;

/// Which axis (`X`, `Y`, or secondary `Y2`) is referenced on a graph scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisKind {
    X,
    Y,
    Y2,
}

const DEFAULT_SNAP_GRID_PT: f64 = 6.0;
const DEFAULT_SNAP_TOLERANCE_PT: f64 = 5.0;
const DEFAULT_PAGE_MARGIN_PT: f64 = 36.0;
const MIN_ZOOM: f64 = 0.1;
const MAX_ZOOM: f64 = 16.0;
const DEFAULT_CALLOUT_WIDTH_PT: f64 = 120.0;
const DEFAULT_CALLOUT_HEIGHT_PT: f64 = 44.0;
const MIN_HANDLE_SPAN_PT: f64 = 18.0;
const INERTIA_MIN_ZOOM: f64 = 0.1;
const INERTIA_MAX_ZOOM: f64 = 32.0;
const DEFAULT_PAN_FRICTION_PER_SEC: f64 = 8.0;
const INERTIA_STOP_SPEED_PT_PER_SEC: f64 = 0.5;
const PAN_VELOCITY_SMOOTHING_RATE: f64 = 45.0;
const MIN_PAN_DT_SEC: f64 = 1e-4;
const PT_PER_INCH: f64 = 72.0;
const PT_PER_MM: f64 = 72.0 / 25.4;
const MIN_MAJOR_TICK_PX: f64 = 48.0;
const MIN_MINOR_TICK_PX: f64 = 8.0;
const MAX_RULER_SPAN_PT: f64 = 1_000_000.0;
const MAX_RULER_MAJOR_TICKS: usize = 512;
const MAX_RULER_TICKS: usize = 2_048;

const fn default_snap_grid_pt() -> f64 {
    DEFAULT_SNAP_GRID_PT
}

const fn default_snap_tolerance_pt() -> f64 {
    DEFAULT_SNAP_TOLERANCE_PT
}

/// Serialize any `Serialize` value without panicking on fallback.
fn serialize_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Declarative intent emitted by a UI/canvas tool to be executed by `lumencraft-engine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCommandIntent {
    pub command_id: String,
    pub params: Value,
}

impl ToolCommandIntent {
    /// Construct a new command intent.
    #[must_use]
    pub fn new(command_id: impl Into<String>, params: Value) -> Self {
        Self { command_id: command_id.into(), params }
    }
}

/// Active interaction mode on a graph or publication layout canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CanvasToolMode {
    #[default]
    Select,
    Bracket,
    AxisBreakResize,
    TextCallout,
    ArrowCallout,
    Pan,
    /// Legacy alias for text callout mode preserved for compatibility.
    Callout,
    /// Legacy alias for axis break/endpoint adjustment mode preserved for compatibility.
    AxisAdjust,
}

/// Endpoint of a cartesian plot axis line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisEndpointKind {
    Min,
    Max,
}

/// Handle location on a layout panel bounding box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResizeHandleCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Left,
    Right,
    Top,
    Bottom,
}

/// Interactive hit-test target inside a graph scene or publication layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CanvasHitTarget {
    PlotArea,
    Axis { kind: AxisKind },
    AxisEndpoint { kind: AxisKind, end: AxisEndpointKind },
    AxisBreak { kind: AxisKind, segment_index: usize },
    Annotation { index: usize },
    EmbeddedTable { index: usize },
    LayoutPanel { index: usize },
    PanelResizeHandle { index: usize, corner: ResizeHandleCorner },
}

/// Anchor recorded on the first click of two-click comparison bracket placement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BracketAnchor {
    pub pt: PointPt,
    pub group_index: Option<usize>,
    pub data_value: Option<f64>,
    #[serde(default)]
    pub result_sheet_id: Option<SheetId>,
}

/// Orientation of a visual alignment guide during layout drag operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapOrientation {
    Vertical,
    Horizontal,
}

/// Reason a candidate bound snapped to a layout guide coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SnapSource {
    PageEdge,
    PageCenter,
    PageMargin,
    Grid,
    SiblingEdge,
    SiblingCenter,
    PlotOrigin,
}

/// Visual guide line rendered while snapping layout panels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapGuide {
    pub orientation: SnapOrientation,
    pub position_pt: f64,
    pub span_start_pt: f64,
    pub span_end_pt: f64,
    pub source: SnapSource,
}

/// Result of snapping a candidate layout panel rectangle against page and sibling guides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSnapResult {
    pub snapped_bounds_pt: RectPt,
    pub guides: Vec<SnapGuide>,
}

/// Interactive pan/zoom/object-placement state for graph and layout viewports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasToolState {
    pub mode: CanvasToolMode,
    pub zoom: f64,
    pub pan_pt: PointPt,
    pub selected_object_id: Option<String>,
    pub drag_start_pt: Option<PointPt>,
    pub snap_to_grid: bool,
    #[serde(default = "default_snap_grid_pt")]
    pub snap_grid_pt: f64,
    #[serde(default = "default_snap_tolerance_pt")]
    pub snap_tolerance_pt: f64,
    #[serde(default)]
    pub hover_target: Option<CanvasHitTarget>,
    #[serde(default)]
    pub pending_bracket_start: Option<BracketAnchor>,
    #[serde(default)]
    pub active_snap_guides: Vec<SnapGuide>,
}

impl Default for CanvasToolState {
    fn default() -> Self {
        Self {
            mode: CanvasToolMode::Select,
            zoom: 1.0,
            pan_pt: PointPt::default(),
            selected_object_id: None,
            drag_start_pt: None,
            snap_to_grid: true,
            snap_grid_pt: DEFAULT_SNAP_GRID_PT,
            snap_tolerance_pt: DEFAULT_SNAP_TOLERANCE_PT,
            hover_target: None,
            pending_bracket_start: None,
            active_snap_guides: Vec::new(),
        }
    }
}

impl CanvasToolState {
    /// Switch the active tool mode, clearing incomplete multi-step placement state when leaving a mode.
    pub fn set_mode(&mut self, mode: CanvasToolMode) {
        if self.mode != mode {
            self.pending_bracket_start = None;
            self.drag_start_pt = None;
            self.active_snap_guides.clear();
            self.mode = mode;
        }
    }

    /// Logical cursor corresponding to the active canvas tool mode and hovered target.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        match self.mode {
            CanvasToolMode::Pan => Cursor::HandGrab,
            CanvasToolMode::Bracket | CanvasToolMode::TextCallout | CanvasToolMode::ArrowCallout | CanvasToolMode::Callout => Cursor::Crosshair,
            CanvasToolMode::AxisBreakResize | CanvasToolMode::AxisAdjust => Cursor::AxisResize,
            CanvasToolMode::Select => match self.hover_target {
                Some(CanvasHitTarget::EmbeddedTable { .. }) => Cursor::CellGrid,
                Some(
                    CanvasHitTarget::Axis { .. }
                    | CanvasHitTarget::AxisEndpoint { .. }
                    | CanvasHitTarget::AxisBreak { .. }
                    | CanvasHitTarget::PanelResizeHandle { .. },
                ) => Cursor::AxisResize,
                _ => Cursor::Select,
            },
        }
    }

    /// Transform a scene coordinate (points) into screen/viewport coordinates (pixels).
    #[must_use]
    pub fn scene_to_screen(&self, scene_pt: PointPt) -> PointPt {
        let zoom = sanitized_zoom(self.zoom);
        PointPt::new(scene_pt.x * zoom + self.pan_pt.x, scene_pt.y * zoom + self.pan_pt.y)
    }

    /// Transform a screen/viewport coordinate (pixels) back into scene coordinates (points).
    #[must_use]
    pub fn screen_to_scene(&self, screen_pt: PointPt) -> PointPt {
        let zoom = sanitized_zoom(self.zoom);
        PointPt::new((screen_pt.x - self.pan_pt.x) / zoom, (screen_pt.y - self.pan_pt.y) / zoom)
    }

    /// Transform a scene rectangle (points) into viewport coordinates (pixels).
    #[must_use]
    pub fn scene_rect_to_screen(&self, rect: RectPt) -> RectPt {
        let zoom = sanitized_zoom(self.zoom);
        let origin = self.scene_to_screen(PointPt::new(rect.x, rect.y));
        RectPt::new(origin.x, origin.y, rect.width * zoom, rect.height * zoom)
    }

    /// Pan the canvas viewport by `(dx_pt, dy_pt)`.
    pub fn pan_by(&mut self, dx_pt: f64, dy_pt: f64) {
        if dx_pt.is_finite() {
            self.pan_pt.x += dx_pt;
        }
        if dy_pt.is_finite() {
            self.pan_pt.y += dy_pt;
        }
    }

    /// Clamp and update viewport zoom multiplier in `[0.1, 16.0]`.
    pub fn set_zoom(&mut self, zoom: f64) {
        if zoom.is_finite() && zoom > 0.0 {
            self.zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        }
    }

    /// Scale zoom multiplicatively while keeping `anchor_screen_pt` pinned to the same scene point.
    pub fn zoom_by_at(&mut self, factor: f64, anchor_screen_pt: PointPt) {
        if !factor.is_finite() || factor <= 0.0 || !anchor_screen_pt.x.is_finite() || !anchor_screen_pt.y.is_finite() {
            return;
        }
        let world_before = self.screen_to_scene(anchor_screen_pt);
        let next_zoom = (sanitized_zoom(self.zoom) * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = next_zoom;
        self.pan_pt = PointPt::new(anchor_screen_pt.x - world_before.x * next_zoom, anchor_screen_pt.y - world_before.y * next_zoom);
    }

    /// Reset pan and zoom back to `1.0x` at the origin.
    pub fn reset_viewport(&mut self) {
        self.zoom = 1.0;
        self.pan_pt = PointPt::default();
    }

    /// Fit a scene of `scene_size_pt` centered inside `viewport_size_px` with uniform padding.
    pub fn fit_scene(&mut self, scene_size_pt: PointPt, viewport_size_px: PointPt, padding_px: f64) {
        if scene_size_pt.x <= 0.0 || scene_size_pt.y <= 0.0 || viewport_size_px.x <= 0.0 || viewport_size_px.y <= 0.0 || !padding_px.is_finite() {
            return;
        }
        let pad = padding_px.max(0.0);
        let avail_w = (viewport_size_px.x - pad * 2.0).max(1.0);
        let avail_h = (viewport_size_px.y - pad * 2.0).max(1.0);
        let fit_zoom = (avail_w / scene_size_pt.x).min(avail_h / scene_size_pt.y).clamp(MIN_ZOOM, MAX_ZOOM);
        self.zoom = fit_zoom;
        self.pan_pt = PointPt::new((viewport_size_px.x - scene_size_pt.x * fit_zoom) * 0.5, (viewport_size_px.y - scene_size_pt.y * fit_zoom) * 0.5);
    }

    /// Update hovered target against a rendered graph scene and spec, returning the resolved target.
    pub fn update_hover_graph(&mut self, scene: &ChartScene, spec: &GraphSpec, pt: PointPt) -> Option<CanvasHitTarget> {
        let hit = hit_test_graph(scene, spec, pt, self.snap_tolerance_pt.max(2.0));
        self.hover_target = hit;
        hit
    }

    /// Update hovered target against a publication layout spec, returning the resolved target.
    pub fn update_hover_layout(&mut self, layout: &LayoutSpec, pt: PointPt) -> Option<CanvasHitTarget> {
        let hit = hit_test_layout(layout, pt, self.snap_tolerance_pt.max(2.0));
        self.hover_target = hit;
        hit
    }

    /// Record the starting point of a pointer drag gesture.
    pub fn begin_drag(&mut self, pt: PointPt) {
        self.drag_start_pt = Some(pt);
    }

    /// Cancel an in-progress two-click bracket placement.
    pub fn cancel_bracket(&mut self) {
        self.pending_bracket_start = None;
    }

    /// Advance two-click comparison bracket placement; click 1 arms, click 2 emits `graph.addComparisonBrackets`.
    pub fn click_bracket(&mut self, sheet_id: &SheetId, anchor: BracketAnchor, default_label: BracketLabelMode) -> Option<ToolCommandIntent> {
        let Some(first) = self.pending_bracket_start.take() else {
            self.pending_bracket_start = Some(anchor);
            return None;
        };
        let (group_a, group_b) = resolve_bracket_group_pair(&first, &anchor);
        let result_sheet_id = anchor.result_sheet_id.or(first.result_sheet_id).unwrap_or_else(|| sheet_id.clone());
        let step_pt = (first.pt.y - anchor.pt.y).abs().clamp(10.0, 48.0);
        let spec = PairwiseBracketSpec {
            result_sheet_id,
            group_a,
            group_b,
            style: BracketStyle::Square,
            label_mode: default_label,
            hide_if_non_significant: false,
            stroke_width_pt: 1.0,
            font_size_pt: 9.0,
            bracket_step_pt: step_pt,
        };
        Some(build_add_comparison_brackets_intent(sheet_id, &[spec]))
    }

    /// Finalize a pointer drag gesture on `sheet_id`, yielding an engine intent when applicable.
    pub fn end_drag(&mut self, sheet_id: &SheetId, end_pt: PointPt) -> Option<ToolCommandIntent> {
        let start = self.drag_start_pt.take()?;
        match self.mode {
            CanvasToolMode::Pan => {
                self.pan_by(end_pt.x - start.x, end_pt.y - start.y);
                None
            }
            CanvasToolMode::Bracket => {
                let left_anchor = BracketAnchor { pt: start, group_index: None, data_value: Some(start.y.min(end_pt.y)), result_sheet_id: None };
                let right_anchor = BracketAnchor { pt: end_pt, group_index: None, data_value: Some(start.y.min(end_pt.y)), result_sheet_id: None };
                let (group_a, group_b) = resolve_bracket_group_pair(&left_anchor, &right_anchor);
                let spec = PairwiseBracketSpec {
                    result_sheet_id: sheet_id.clone(),
                    group_a,
                    group_b,
                    style: BracketStyle::Square,
                    label_mode: BracketLabelMode::SummaryStars,
                    hide_if_non_significant: false,
                    stroke_width_pt: 1.0,
                    font_size_pt: 9.0,
                    bracket_step_pt: 12.0,
                };
                Some(build_add_comparison_brackets_intent(sheet_id, &[spec]))
            }
            CanvasToolMode::TextCallout | CanvasToolMode::Callout => {
                let bounds = normalized_rect_from_drag(start, end_pt, DEFAULT_CALLOUT_WIDTH_PT, DEFAULT_CALLOUT_HEIGHT_PT);
                Some(build_add_text_callout_intent(sheet_id, bounds, "Callout", true))
            }
            CanvasToolMode::ArrowCallout => Some(build_add_arrow_callout_intent(sheet_id, start, end_pt, true)),
            CanvasToolMode::Select | CanvasToolMode::AxisBreakResize | CanvasToolMode::AxisAdjust => None,
        }
    }

    /// Snap a candidate layout rectangle to page edges/centers, margins, plot origins, siblings, and grid.
    #[must_use]
    pub fn compute_layout_snap(
        &self,
        layout: &LayoutSpec,
        moving_index: Option<usize>,
        candidate_bounds_pt: RectPt,
        plot_origins_pt: &[PointPt],
    ) -> LayoutSnapResult {
        let page_size = layout.page_size_pt();
        let (page_w, page_h) = (page_size.width, page_size.height);
        let mut x_candidates = Vec::new();
        let mut y_candidates = Vec::new();
        collect_page_snap_lines(page_w, page_h, &mut x_candidates, &mut y_candidates);
        collect_sibling_snap_lines(layout, moving_index, page_w, page_h, &mut x_candidates, &mut y_candidates);
        if layout.align_plot_origins {
            collect_plot_origin_snap_lines(plot_origins_pt, page_w, page_h, &mut x_candidates, &mut y_candidates);
        }

        let tolerance = self.snap_tolerance_pt.max(0.0);
        let x_cfg = SnapAxisParams {
            lines: &x_candidates,
            snap_to_grid: self.snap_to_grid,
            grid_pt: self.snap_grid_pt,
            tolerance_pt: tolerance,
            cross_span_pt: page_h,
        };
        let y_cfg = SnapAxisParams {
            lines: &y_candidates,
            snap_to_grid: self.snap_to_grid,
            grid_pt: self.snap_grid_pt,
            tolerance_pt: tolerance,
            cross_span_pt: page_w,
        };
        let x_snap = snap_axis(SnapOrientation::Vertical, candidate_bounds_pt.x, candidate_bounds_pt.width, &x_cfg);
        let y_snap = snap_axis(SnapOrientation::Horizontal, candidate_bounds_pt.y, candidate_bounds_pt.height, &y_cfg);

        let snapped_bounds_pt = RectPt::new(x_snap.snapped_origin, y_snap.snapped_origin, candidate_bounds_pt.width, candidate_bounds_pt.height);
        let mut guides = x_snap.guides;
        guides.extend(y_snap.guides);
        LayoutSnapResult { snapped_bounds_pt, guides }
    }

    /// Snap a dragged layout item to active guides, update `active_snap_guides`, and emit `layout.setSpec`.
    pub fn commit_layout_panel_bounds(
        &mut self,
        sheet_id: &SheetId,
        layout: &LayoutSpec,
        item_index: usize,
        candidate_bounds_pt: RectPt,
        plot_origins_pt: &[PointPt],
    ) -> Option<ToolCommandIntent> {
        if item_index >= layout.items.len() {
            return None;
        }
        let snapped = self.compute_layout_snap(layout, Some(item_index), candidate_bounds_pt, plot_origins_pt);
        self.active_snap_guides = snapped.guides;
        let mut updated = layout.clone();
        if let Some(item) = updated.items.get_mut(item_index) {
            set_layout_item_bounds(item, snapped.snapped_bounds_pt.into());
        }
        Some(build_set_layout_spec_intent(sheet_id, &updated))
    }

    /// Resize a discontinuous axis break segment on `sheet` and emit `graph.setAxes`.
    #[must_use]
    pub fn apply_axis_break_resize(
        &self,
        sheet: &GraphSheet,
        kind: AxisKind,
        segment_index: usize,
        new_share: f64,
        new_gap_pt: Option<f64>,
    ) -> Option<ToolCommandIntent> {
        let mut x_axis = sheet.spec.x_axis.clone();
        let mut y_axis = sheet.spec.y_axis.clone();
        let mut y2_axis = sheet.spec.y2_axis.clone();
        let target_axis = match kind {
            AxisKind::X => &mut x_axis,
            AxisKind::Y => &mut y_axis,
            AxisKind::Y2 => y2_axis.as_mut()?,
        };
        if !adjust_axis_segment_share(target_axis, segment_index, new_share, new_gap_pt) {
            return None;
        }
        Some(build_set_axes_intent(&sheet.id, &x_axis, &y_axis, y2_axis.as_ref()))
    }
}

/// Build an intent for `graph.addComparisonBrackets`.
#[must_use]
pub fn build_add_comparison_brackets_intent(sheet_id: &SheetId, brackets: &[PairwiseBracketSpec]) -> ToolCommandIntent {
    let brackets_val = serialize_value(&brackets);
    ToolCommandIntent::new("graph.addComparisonBrackets", json!({ "sheet_id": sheet_id.as_str(), "brackets": brackets_val }))
}

/// Build an intent for `graph.setAxes`.
#[must_use]
pub fn build_set_axes_intent(sheet_id: &SheetId, x_axis: &AxisSpec, y_axis: &AxisSpec, y2_axis: Option<&AxisSpec>) -> ToolCommandIntent {
    let x_val = serialize_value(x_axis);
    let y_val = serialize_value(y_axis);
    let y2_val = y2_axis.map(serialize_value).unwrap_or(Value::Null);
    ToolCommandIntent::new(
        "graph.setAxes",
        json!({
            "sheet_id": sheet_id.as_str(),
            "x_axis": x_val,
            "y_axis": y_val,
            "y2_axis": y2_val,
        }),
    )
}

/// Build an intent for `graph.addAnnotation`.
#[must_use]
pub fn build_add_annotation_intent(sheet_id: &SheetId, annotation: &GraphAnnotation) -> ToolCommandIntent {
    let ann_val = serialize_value(annotation);
    ToolCommandIntent::new("graph.addAnnotation", json!({ "sheet_id": sheet_id.as_str(), "annotation": ann_val }))
}

/// Build a text callout `graph.addAnnotation` intent.
#[must_use]
pub fn build_add_text_callout_intent(sheet_id: &SheetId, bounds_pt: RectPt, markdown: impl Into<String>, bordered: bool) -> ToolCommandIntent {
    let annotation = GraphAnnotation::TextBox { bounds_pt: bounds_pt.into(), markdown: markdown.into(), bordered };
    build_add_annotation_intent(sheet_id, &annotation)
}

/// Build an arrow callout `graph.addAnnotation` intent.
#[must_use]
pub fn build_add_arrow_callout_intent(sheet_id: &SheetId, start_pt: PointPt, end_pt: PointPt, arrowhead: bool) -> ToolCommandIntent {
    let annotation = GraphAnnotation::LineOrArrow {
        start_pt: (start_pt.x, start_pt.y),
        end_pt: (end_pt.x, end_pt.y),
        arrowhead,
        dash: LineDashStyle::Solid,
        color: Rgba8::BLACK,
    };
    build_add_annotation_intent(sheet_id, &annotation)
}

/// Build an intent for `layout.setSpec`.
#[must_use]
pub fn build_set_layout_spec_intent(sheet_id: &SheetId, spec: &LayoutSpec) -> ToolCommandIntent {
    let spec_val = serialize_value(spec);
    ToolCommandIntent::new("layout.setSpec", json!({ "sheet_id": sheet_id.as_str(), "spec": spec_val }))
}

/// Hit-test basic plot geometry (`PlotArea`, X/Y axes, and endpoints) in a `ChartScene`.
#[must_use]
pub fn hit_test_scene(scene: &ChartScene, pt: PointPt, tolerance_pt: f64) -> Option<CanvasHitTarget> {
    hit_test_plot_axes(scene.plot_rect_pt, false, pt, tolerance_pt.max(1.0))
        .or_else(|| rect_contains_with_margin(scene.plot_rect_pt, pt, 0.0).then_some(CanvasHitTarget::PlotArea))
}

/// Hit-test annotations, axis breaks, axes/endpoints, and plot interior on a graph scene + spec.
#[must_use]
pub fn hit_test_graph(scene: &ChartScene, spec: &GraphSpec, pt: PointPt, tolerance_pt: f64) -> Option<CanvasHitTarget> {
    let tol = tolerance_pt.max(1.0);
    for (idx, ann) in spec.annotations.iter().enumerate().rev() {
        if hit_test_annotation(ann, pt, tol) {
            return Some(match ann {
                GraphAnnotation::EmbeddedResultTable { .. } => CanvasHitTarget::EmbeddedTable { index: idx },
                _ => CanvasHitTarget::Annotation { index: idx },
            });
        }
    }
    if let Some(hit) = hit_test_axis_breaks(scene.plot_rect_pt, &spec.x_axis, &spec.y_axis, spec.y2_axis.as_ref(), pt, tol) {
        return Some(hit);
    }
    if let Some(hit) = hit_test_plot_axes(scene.plot_rect_pt, spec.y2_axis.is_some(), pt, tol) {
        return Some(hit);
    }
    rect_contains_with_margin(scene.plot_rect_pt, pt, 0.0).then_some(CanvasHitTarget::PlotArea)
}

/// Hit-test panel boxes, resize handles, and live tables on a `LayoutSpec`.
#[must_use]
pub fn hit_test_layout(layout: &LayoutSpec, pt: PointPt, tolerance_pt: f64) -> Option<CanvasHitTarget> {
    let tol = tolerance_pt.max(1.0);
    for (idx, item) in layout.items.iter().enumerate().rev() {
        let bounds: RectPt = layout_item_bounds(item).into();
        if let Some(corner) = hit_test_resize_handles(bounds, pt, tol) {
            return Some(CanvasHitTarget::PanelResizeHandle { index: idx, corner });
        }
        if rect_contains_with_margin(bounds, pt, 0.0) {
            return Some(match item {
                LayoutItem::LiveTable { .. } => CanvasHitTarget::EmbeddedTable { index: idx },
                LayoutItem::LiveGraph { .. } | LayoutItem::TextCallout { .. } => CanvasHitTarget::LayoutPanel { index: idx },
            });
        }
    }
    None
}

/// Compute screen-space axis break boundary X or Y coordinates inside `plot_rect`.
#[must_use]
pub fn axis_break_positions_pt(plot_rect: RectPt, kind: AxisKind, axis: &AxisSpec) -> Vec<f64> {
    if axis.segments.len() <= 1 {
        return Vec::new();
    }
    let span = match kind {
        AxisKind::X => plot_rect.width,
        AxisKind::Y | AxisKind::Y2 => plot_rect.height,
    };
    let origin = match kind {
        AxisKind::X => plot_rect.x,
        AxisKind::Y | AxisKind::Y2 => plot_rect.bottom(),
    };
    let dir = match kind {
        AxisKind::X => 1.0,
        AxisKind::Y | AxisKind::Y2 => -1.0,
    };

    let gap = axis.gap_width_pt.max(0.0);
    let break_count = axis.segments.len().saturating_sub(1);
    let total_gap = gap * (break_count as f64);
    let usable_span = (span - total_gap).max(0.0);
    let total_share: f64 = axis.segments.iter().map(|seg| seg.length_share.max(0.0)).sum();
    if total_share <= 0.0 {
        return Vec::new();
    }

    let mut boundaries = Vec::with_capacity(break_count);
    let mut cursor = 0.0;
    for (idx, seg) in axis.segments.iter().enumerate() {
        if idx > 0 {
            boundaries.push(origin + dir * (cursor + gap * 0.5));
            cursor += gap;
        }
        cursor += usable_span * (seg.length_share.max(0.0) / total_share);
    }
    boundaries
}

fn sanitized_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() && zoom > 0.0 { zoom.clamp(MIN_ZOOM, MAX_ZOOM) } else { 1.0 }
}

fn resolve_bracket_group_pair(first: &BracketAnchor, second: &BracketAnchor) -> (usize, usize) {
    match (first.group_index, second.group_index) {
        (Some(a), Some(b)) if a != b => (a.min(b), a.max(b)),
        (Some(a), Some(_)) => (a, a.saturating_add(1)),
        _ => {
            if first.pt.x <= second.pt.x {
                let left = ((first.pt.x.max(0.0) / 48.0).floor() as usize).min(1022);
                let right = ((second.pt.x.max(0.0) / 48.0).floor() as usize).max(left.saturating_add(1)).min(1023);
                (left, right)
            } else {
                let left = ((second.pt.x.max(0.0) / 48.0).floor() as usize).min(1022);
                let right = ((first.pt.x.max(0.0) / 48.0).floor() as usize).max(left.saturating_add(1)).min(1023);
                (left, right)
            }
        }
    }
}

fn normalized_rect_from_drag(start: PointPt, end: PointPt, fallback_w: f64, fallback_h: f64) -> RectPt {
    let x = start.x.min(end.x);
    let y = start.y.min(end.y);
    let raw_w = (end.x - start.x).abs();
    let raw_h = (end.y - start.y).abs();
    let width = if raw_w < 8.0 { fallback_w } else { raw_w };
    let height = if raw_h < 8.0 { fallback_h } else { raw_h };
    RectPt::new(x, y, width, height)
}

fn adjust_axis_segment_share(axis: &mut AxisSpec, segment_index: usize, new_share: f64, new_gap_pt: Option<f64>) -> bool {
    if !new_share.is_finite() || axis.segments.len() <= 1 || segment_index >= axis.segments.len() {
        return false;
    }
    let neighbor_idx = if segment_index + 1 < axis.segments.len() { segment_index + 1 } else { segment_index - 1 };
    let Some(curr_share) = axis.segments.get(segment_index).map(|seg| seg.length_share) else {
        return false;
    };
    let Some(neighbor_share) = axis.segments.get(neighbor_idx).map(|seg| seg.length_share) else {
        return false;
    };
    let pair_total = curr_share + neighbor_share;
    if pair_total <= 0.1 {
        return false;
    }
    let clamped = new_share.clamp(0.05, pair_total - 0.05);
    if let Some(gap) = new_gap_pt.filter(|val| val.is_finite() && *val >= 0.0) {
        axis.gap_width_pt = gap;
    }
    if let Some(seg) = axis.segments.get_mut(segment_index) {
        seg.length_share = clamped;
    }
    if let Some(neighbor) = axis.segments.get_mut(neighbor_idx) {
        neighbor.length_share = (pair_total - clamped).max(0.05);
    }
    true
}

fn hit_test_axis_breaks(
    plot_rect: RectPt,
    x_axis: &AxisSpec,
    y_axis: &AxisSpec,
    y2_axis: Option<&AxisSpec>,
    pt: PointPt,
    tol: f64,
) -> Option<CanvasHitTarget> {
    for (idx, pos_x) in axis_break_positions_pt(plot_rect, AxisKind::X, x_axis).into_iter().enumerate() {
        if (pt.x - pos_x).abs() <= tol && (pt.y - plot_rect.bottom()).abs() <= tol * 1.5 {
            return Some(CanvasHitTarget::AxisBreak { kind: AxisKind::X, segment_index: idx + 1 });
        }
    }
    for (idx, pos_y) in axis_break_positions_pt(plot_rect, AxisKind::Y, y_axis).into_iter().enumerate() {
        if (pt.y - pos_y).abs() <= tol && (pt.x - plot_rect.x).abs() <= tol * 1.5 {
            return Some(CanvasHitTarget::AxisBreak { kind: AxisKind::Y, segment_index: idx + 1 });
        }
    }
    if let Some(y2) = y2_axis {
        for (idx, pos_y) in axis_break_positions_pt(plot_rect, AxisKind::Y2, y2).into_iter().enumerate() {
            if (pt.y - pos_y).abs() <= tol && (pt.x - plot_rect.right()).abs() <= tol * 1.5 {
                return Some(CanvasHitTarget::AxisBreak { kind: AxisKind::Y2, segment_index: idx + 1 });
            }
        }
    }
    None
}

fn hit_test_plot_axes(plot_rect: RectPt, has_y2: bool, pt: PointPt, tol: f64) -> Option<CanvasHitTarget> {
    let endpoints = [
        (PointPt::new(plot_rect.x, plot_rect.bottom()), AxisKind::X, AxisEndpointKind::Min),
        (PointPt::new(plot_rect.right(), plot_rect.bottom()), AxisKind::X, AxisEndpointKind::Max),
        (PointPt::new(plot_rect.x, plot_rect.y), AxisKind::Y, AxisEndpointKind::Max),
    ];
    for (anchor, kind, end) in endpoints {
        if distance(pt, anchor) <= tol * 1.5 {
            return Some(CanvasHitTarget::AxisEndpoint { kind, end });
        }
    }
    if has_y2 {
        let y2_endpoints = [
            (PointPt::new(plot_rect.right(), plot_rect.bottom()), AxisEndpointKind::Min),
            (PointPt::new(plot_rect.right(), plot_rect.y), AxisEndpointKind::Max),
        ];
        for (anchor, end) in y2_endpoints {
            if distance(pt, anchor) <= tol * 1.5 {
                return Some(CanvasHitTarget::AxisEndpoint { kind: AxisKind::Y2, end });
            }
        }
    }

    if point_to_segment_distance(pt, PointPt::new(plot_rect.x, plot_rect.bottom()), PointPt::new(plot_rect.right(), plot_rect.bottom())) <= tol {
        return Some(CanvasHitTarget::Axis { kind: AxisKind::X });
    }
    if point_to_segment_distance(pt, PointPt::new(plot_rect.x, plot_rect.y), PointPt::new(plot_rect.x, plot_rect.bottom())) <= tol {
        return Some(CanvasHitTarget::Axis { kind: AxisKind::Y });
    }
    if has_y2
        && point_to_segment_distance(pt, PointPt::new(plot_rect.right(), plot_rect.y), PointPt::new(plot_rect.right(), plot_rect.bottom())) <= tol
    {
        return Some(CanvasHitTarget::Axis { kind: AxisKind::Y2 });
    }
    None
}

fn hit_test_annotation(annotation: &GraphAnnotation, pt: PointPt, tol: f64) -> bool {
    match annotation {
        GraphAnnotation::TextBox { bounds_pt, .. }
        | GraphAnnotation::EmbeddedResultTable { bounds_pt, .. }
        | GraphAnnotation::Shape { bounds_pt, .. } => rect_contains_with_margin((*bounds_pt).into(), pt, tol),
        GraphAnnotation::LineOrArrow { start_pt, end_pt, .. } => {
            point_to_segment_distance(pt, PointPt::new(start_pt.0, start_pt.1), PointPt::new(end_pt.0, end_pt.1)) <= tol
        }
    }
}

fn hit_test_resize_handles(bounds: RectPt, pt: PointPt, tol: f64) -> Option<ResizeHandleCorner> {
    let corners = [
        (PointPt::new(bounds.x, bounds.y), ResizeHandleCorner::TopLeft),
        (PointPt::new(bounds.right(), bounds.y), ResizeHandleCorner::TopRight),
        (PointPt::new(bounds.x, bounds.bottom()), ResizeHandleCorner::BottomLeft),
        (PointPt::new(bounds.right(), bounds.bottom()), ResizeHandleCorner::BottomRight),
    ];
    for (anchor, corner) in corners {
        if (pt.x - anchor.x).abs() <= tol && (pt.y - anchor.y).abs() <= tol {
            return Some(corner);
        }
    }
    if bounds.width >= MIN_HANDLE_SPAN_PT && bounds.height >= MIN_HANDLE_SPAN_PT {
        let mid_x = bounds.x + bounds.width * 0.5;
        let mid_y = bounds.y + bounds.height * 0.5;
        let edge_mids = [
            (PointPt::new(mid_x, bounds.y), ResizeHandleCorner::Top),
            (PointPt::new(mid_x, bounds.bottom()), ResizeHandleCorner::Bottom),
            (PointPt::new(bounds.x, mid_y), ResizeHandleCorner::Left),
            (PointPt::new(bounds.right(), mid_y), ResizeHandleCorner::Right),
        ];
        for (anchor, corner) in edge_mids {
            if (pt.x - anchor.x).abs() <= tol && (pt.y - anchor.y).abs() <= tol {
                return Some(corner);
            }
        }
    }
    None
}

fn layout_item_bounds(item: &LayoutItem) -> DocRectPt {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => *bounds_pt,
    }
}

fn set_layout_item_bounds(item: &mut LayoutItem, next: DocRectPt) {
    match item {
        LayoutItem::LiveGraph { bounds_pt, .. } | LayoutItem::LiveTable { bounds_pt, .. } | LayoutItem::TextCallout { bounds_pt, .. } => {
            *bounds_pt = next
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SnapLine {
    coord: f64,
    span_start: f64,
    span_end: f64,
    source: SnapSource,
}

struct AxisSnapResolution {
    snapped_origin: f64,
    guides: Vec<SnapGuide>,
}

fn collect_page_snap_lines(page_w: f64, page_h: f64, x_lines: &mut Vec<SnapLine>, y_lines: &mut Vec<SnapLine>) {
    x_lines.push(SnapLine { coord: 0.0, span_start: 0.0, span_end: page_h, source: SnapSource::PageEdge });
    x_lines.push(SnapLine { coord: page_w * 0.5, span_start: 0.0, span_end: page_h, source: SnapSource::PageCenter });
    x_lines.push(SnapLine { coord: page_w, span_start: 0.0, span_end: page_h, source: SnapSource::PageEdge });
    y_lines.push(SnapLine { coord: 0.0, span_start: 0.0, span_end: page_w, source: SnapSource::PageEdge });
    y_lines.push(SnapLine { coord: page_h * 0.5, span_start: 0.0, span_end: page_w, source: SnapSource::PageCenter });
    y_lines.push(SnapLine { coord: page_h, span_start: 0.0, span_end: page_w, source: SnapSource::PageEdge });

    if page_w > DEFAULT_PAGE_MARGIN_PT * 2.5 {
        x_lines.push(SnapLine { coord: DEFAULT_PAGE_MARGIN_PT, span_start: 0.0, span_end: page_h, source: SnapSource::PageMargin });
        x_lines.push(SnapLine { coord: page_w - DEFAULT_PAGE_MARGIN_PT, span_start: 0.0, span_end: page_h, source: SnapSource::PageMargin });
    }
    if page_h > DEFAULT_PAGE_MARGIN_PT * 2.5 {
        y_lines.push(SnapLine { coord: DEFAULT_PAGE_MARGIN_PT, span_start: 0.0, span_end: page_w, source: SnapSource::PageMargin });
        y_lines.push(SnapLine { coord: page_h - DEFAULT_PAGE_MARGIN_PT, span_start: 0.0, span_end: page_w, source: SnapSource::PageMargin });
    }
}

fn collect_sibling_snap_lines(
    layout: &LayoutSpec,
    moving_index: Option<usize>,
    page_w: f64,
    page_h: f64,
    x_lines: &mut Vec<SnapLine>,
    y_lines: &mut Vec<SnapLine>,
) {
    for (idx, item) in layout.items.iter().enumerate() {
        if moving_index == Some(idx) {
            continue;
        }
        let rect: RectPt = layout_item_bounds(item).into();
        x_lines.push(SnapLine { coord: rect.x, span_start: 0.0, span_end: page_h, source: SnapSource::SiblingEdge });
        x_lines.push(SnapLine { coord: rect.x + rect.width * 0.5, span_start: 0.0, span_end: page_h, source: SnapSource::SiblingCenter });
        x_lines.push(SnapLine { coord: rect.right(), span_start: 0.0, span_end: page_h, source: SnapSource::SiblingEdge });
        y_lines.push(SnapLine { coord: rect.y, span_start: 0.0, span_end: page_w, source: SnapSource::SiblingEdge });
        y_lines.push(SnapLine { coord: rect.y + rect.height * 0.5, span_start: 0.0, span_end: page_w, source: SnapSource::SiblingCenter });
        y_lines.push(SnapLine { coord: rect.bottom(), span_start: 0.0, span_end: page_w, source: SnapSource::SiblingEdge });
    }
}

fn collect_plot_origin_snap_lines(plot_origins_pt: &[PointPt], page_w: f64, page_h: f64, x_lines: &mut Vec<SnapLine>, y_lines: &mut Vec<SnapLine>) {
    for origin in plot_origins_pt {
        if origin.x.is_finite() {
            x_lines.push(SnapLine { coord: origin.x, span_start: 0.0, span_end: page_h, source: SnapSource::PlotOrigin });
        }
        if origin.y.is_finite() {
            y_lines.push(SnapLine { coord: origin.y, span_start: 0.0, span_end: page_w, source: SnapSource::PlotOrigin });
        }
    }
}

struct SnapAxisParams<'a> {
    lines: &'a [SnapLine],
    snap_to_grid: bool,
    grid_pt: f64,
    tolerance_pt: f64,
    cross_span_pt: f64,
}

fn snap_axis(orientation: SnapOrientation, origin: f64, extent: f64, cfg: &SnapAxisParams<'_>) -> AxisSnapResolution {
    let anchors = [0.0, extent * 0.5, extent];
    let mut best_delta: Option<f64> = None;
    for offset in anchors {
        let candidate = origin + offset;
        for line in cfg.lines {
            let delta = line.coord - candidate;
            if delta.abs() <= cfg.tolerance_pt && best_delta.is_none_or(|prev| delta.abs() < prev.abs()) {
                best_delta = Some(delta);
            }
        }
    }

    let mut guides = Vec::new();
    let snapped_origin = if let Some(delta) = best_delta {
        let snapped = origin + delta;
        for offset in anchors {
            let aligned_coord = snapped + offset;
            for line in cfg.lines {
                if (line.coord - aligned_coord).abs() <= 0.01 {
                    guides.push(SnapGuide {
                        orientation,
                        position_pt: line.coord,
                        span_start_pt: line.span_start,
                        span_end_pt: line.span_end,
                        source: line.source,
                    });
                }
            }
        }
        snapped
    } else if cfg.snap_to_grid && cfg.grid_pt.is_finite() && cfg.grid_pt > 0.5 {
        let grid_coord = (origin / cfg.grid_pt).round() * cfg.grid_pt;
        if (grid_coord - origin).abs() <= cfg.tolerance_pt {
            guides.push(SnapGuide {
                orientation,
                position_pt: grid_coord,
                span_start_pt: 0.0,
                span_end_pt: cfg.cross_span_pt,
                source: SnapSource::Grid,
            });
            grid_coord
        } else {
            origin
        }
    } else {
        origin
    };

    AxisSnapResolution { snapped_origin, guides }
}

fn rect_contains_with_margin(rect: RectPt, pt: PointPt, margin: f64) -> bool {
    pt.x >= rect.x - margin && pt.x <= rect.right() + margin && pt.y >= rect.y - margin && pt.y <= rect.bottom() + margin
}

fn distance(a: PointPt, b: PointPt) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn point_to_segment_distance(pt: PointPt, a: PointPt, b: PointPt) -> f64 {
    let abx = b.x - a.x;
    let aby = b.y - a.y;
    let len_sq = abx * abx + aby * aby;
    if len_sq <= f64::EPSILON {
        return distance(pt, a);
    }
    let t = (((pt.x - a.x) * abx + (pt.y - a.y) * aby) / len_sq).clamp(0.0, 1.0);
    let proj = PointPt::new(a.x + t * abx, a.y + t * aby);
    distance(pt, proj)
}

const fn default_inertia_zoom() -> f64 {
    1.0
}

const fn default_pan_friction() -> f64 {
    DEFAULT_PAN_FRICTION_PER_SEC
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_non_neg(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 { value } else { 0.0 }
}

fn sanitized_inertia_zoom(zoom: f64) -> f64 {
    if zoom.is_finite() && zoom > 0.0 { zoom.clamp(INERTIA_MIN_ZOOM, INERTIA_MAX_ZOOM) } else { 1.0 }
}

/// Smooth trackpad pinch-to-zoom and exponential inertial pan state (`L4`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanZoomInertiaState {
    #[serde(default = "default_inertia_zoom")]
    pub zoom: f64,
    #[serde(default)]
    pub pan_pt: [f64; 2],
    #[serde(default)]
    pub vel_pt_per_sec: [f64; 2],
    #[serde(default = "default_pan_friction")]
    pub friction_per_sec: f64,
}

impl Default for PanZoomInertiaState {
    fn default() -> Self {
        Self { zoom: 1.0, pan_pt: [0.0, 0.0], vel_pt_per_sec: [0.0, 0.0], friction_per_sec: DEFAULT_PAN_FRICTION_PER_SEC }
    }
}

impl PanZoomInertiaState {
    /// Scale `zoom` multiplicatively within `[0.1, 32.0]` while keeping `anchor_view_pt` stationary.
    pub fn apply_pinch_zoom(&mut self, factor: f64, anchor_view_pt: [f64; 2]) {
        if !factor.is_finite() || factor <= 0.0 || !anchor_view_pt[0].is_finite() || !anchor_view_pt[1].is_finite() {
            return;
        }
        let old_zoom = sanitized_inertia_zoom(self.zoom);
        let next_zoom = (old_zoom * factor).clamp(INERTIA_MIN_ZOOM, INERTIA_MAX_ZOOM);
        let pan_x = finite_or_zero(self.pan_pt[0]);
        let pan_y = finite_or_zero(self.pan_pt[1]);
        if (next_zoom - old_zoom) == 0.0 {
            self.zoom = next_zoom;
            self.pan_pt = [pan_x, pan_y];
            return;
        }
        let content_x = (anchor_view_pt[0] - pan_x) / old_zoom;
        let content_y = (anchor_view_pt[1] - pan_y) / old_zoom;
        let next_pan_x = anchor_view_pt[0] - content_x * next_zoom;
        let next_pan_y = anchor_view_pt[1] - content_y * next_zoom;
        if next_pan_x.is_finite() && next_pan_y.is_finite() {
            self.zoom = next_zoom;
            self.pan_pt = [next_pan_x, next_pan_y];
        }
    }

    /// Apply a trackpad pan delta `(dx_pt, dy_pt)` over `dt_sec`, updating pan and smoothing velocity.
    pub fn apply_pan_delta(&mut self, delta_pt: [f64; 2], dt_sec: f64) {
        if !delta_pt[0].is_finite() || !delta_pt[1].is_finite() || !dt_sec.is_finite() || dt_sec < 0.0 {
            return;
        }
        let next_pan_x = finite_or_zero(self.pan_pt[0]) + delta_pt[0];
        let next_pan_y = finite_or_zero(self.pan_pt[1]) + delta_pt[1];
        if next_pan_x.is_finite() && next_pan_y.is_finite() {
            self.pan_pt = [next_pan_x, next_pan_y];
        }
        if dt_sec == 0.0 {
            return;
        }
        let effective_dt = dt_sec.max(MIN_PAN_DT_SEC);
        let inst_vx = delta_pt[0] / effective_dt;
        let inst_vy = delta_pt[1] / effective_dt;
        if !inst_vx.is_finite() || !inst_vy.is_finite() {
            return;
        }
        let alpha = (1.0 - (-dt_sec * PAN_VELOCITY_SMOOTHING_RATE).exp()).clamp(0.25, 1.0);
        let prev_vx = finite_or_zero(self.vel_pt_per_sec[0]);
        let prev_vy = finite_or_zero(self.vel_pt_per_sec[1]);
        let next_vx = prev_vx + (inst_vx - prev_vx) * alpha;
        let next_vy = prev_vy + (inst_vy - prev_vy) * alpha;
        if next_vx.is_finite() && next_vy.is_finite() {
            self.vel_pt_per_sec = [next_vx, next_vy];
        }
    }

    /// Advance inertial coasting by `dt_sec`, decaying exponentially until speed drops below `0.5 pt/s`.
    /// Returns `true` while still moving and `false` once snapped cleanly to zero velocity.
    pub fn step_inertia(&mut self, dt_sec: f64) -> bool {
        let vx = finite_or_zero(self.vel_pt_per_sec[0]);
        let vy = finite_or_zero(self.vel_pt_per_sec[1]);
        let pan_x = finite_or_zero(self.pan_pt[0]);
        let pan_y = finite_or_zero(self.pan_pt[1]);
        self.pan_pt = [pan_x, pan_y];

        if !dt_sec.is_finite() || dt_sec <= 0.0 {
            let speed = vx.hypot(vy);
            if !speed.is_finite() || speed < INERTIA_STOP_SPEED_PT_PER_SEC {
                self.vel_pt_per_sec = [0.0, 0.0];
            } else {
                self.vel_pt_per_sec = [vx, vy];
            }
            return false;
        }

        let start_speed = vx.hypot(vy);
        if !start_speed.is_finite() || start_speed < INERTIA_STOP_SPEED_PT_PER_SEC {
            self.vel_pt_per_sec = [0.0, 0.0];
            return false;
        }

        let friction = if self.friction_per_sec.is_finite() && self.friction_per_sec > 0.0 {
            self.friction_per_sec.max(1e-3)
        } else {
            DEFAULT_PAN_FRICTION_PER_SEC
        };
        let decay = (-friction * dt_sec).exp();
        let disp_scale = -((-friction * dt_sec).exp_m1()) / friction;
        let next_pan_x = pan_x + vx * disp_scale;
        let next_pan_y = pan_y + vy * disp_scale;
        if next_pan_x.is_finite() && next_pan_y.is_finite() {
            self.pan_pt = [next_pan_x, next_pan_y];
        }

        let next_vx = vx * decay;
        let next_vy = vy * decay;
        let next_speed = next_vx.hypot(next_vy);
        if !next_speed.is_finite() || next_speed < INERTIA_STOP_SPEED_PT_PER_SEC {
            self.vel_pt_per_sec = [0.0, 0.0];
            false
        } else {
            self.vel_pt_per_sec = [next_vx, next_vy];
            true
        }
    }

    /// Fit `content_size_pt` centered inside `viewport_size_pt`, clearing residual velocity.
    pub fn zoom_to_fit(&mut self, content_size_pt: [f64; 2], viewport_size_pt: [f64; 2]) {
        let [cw, ch] = content_size_pt;
        let [vw, vh] = viewport_size_pt;
        if !cw.is_finite() || !ch.is_finite() || !vw.is_finite() || !vh.is_finite() || cw <= 0.0 || ch <= 0.0 || vw <= 0.0 || vh <= 0.0 {
            return;
        }
        let fit_zoom = (vw / cw).min(vh / ch).clamp(INERTIA_MIN_ZOOM, INERTIA_MAX_ZOOM);
        if !fit_zoom.is_finite() || fit_zoom <= 0.0 {
            return;
        }
        let pan_x = (vw - cw * fit_zoom) * 0.5;
        let pan_y = (vh - ch * fit_zoom) * 0.5;
        if pan_x.is_finite() && pan_y.is_finite() {
            self.zoom = fit_zoom;
            self.pan_pt = [pan_x, pan_y];
            self.vel_pt_per_sec = [0.0, 0.0];
        }
    }

    /// Update zoom from a percentage (`100.0` = `1.0x`), clamped to `[0.1, 32.0]`.
    pub fn set_zoom_percent(&mut self, pct: f64) {
        if !pct.is_finite() || pct <= 0.0 {
            return;
        }
        self.zoom = (pct / 100.0).clamp(INERTIA_MIN_ZOOM, INERTIA_MAX_ZOOM);
    }

    /// Transform a view-space point into content-space points.
    #[must_use]
    pub fn view_to_content(&self, view_pt: [f64; 2]) -> [f64; 2] {
        let zoom = sanitized_inertia_zoom(self.zoom);
        [(view_pt[0] - finite_or_zero(self.pan_pt[0])) / zoom, (view_pt[1] - finite_or_zero(self.pan_pt[1])) / zoom]
    }

    /// Transform a content-space point into view-space points.
    #[must_use]
    pub fn content_to_view(&self, content_pt: [f64; 2]) -> [f64; 2] {
        let zoom = sanitized_inertia_zoom(self.zoom);
        [content_pt[0] * zoom + finite_or_zero(self.pan_pt[0]), content_pt[1] * zoom + finite_or_zero(self.pan_pt[1])]
    }
}

/// Physical measurement unit used by page layout rulers (`L4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum PhysicalUnit {
    #[default]
    Millimeters,
    Inches,
    Points,
}

impl PhysicalUnit {
    /// Exact typographic points per physical unit (`1 in = 72 pt`, `1 mm = 72.0 / 25.4 pt`).
    #[must_use]
    pub const fn pt_per_unit(self) -> f64 {
        match self {
            Self::Millimeters => PT_PER_MM,
            Self::Inches => PT_PER_INCH,
            Self::Points => 1.0,
        }
    }

    /// Convert `value` in this physical unit into typographic points (`0.0` on non-finite input).
    #[must_use]
    pub fn to_points(self, value: f64) -> f64 {
        if value.is_finite() { value * self.pt_per_unit() } else { 0.0 }
    }

    /// Convert typographic `points` into this physical unit (`0.0` on non-finite input).
    #[must_use]
    pub fn from_points(self, points: f64) -> f64 {
        if points.is_finite() { points / self.pt_per_unit() } else { 0.0 }
    }
}

/// Standard journal figure-width and page presets in points (`L4`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum JournalWidthPreset {
    SingleColumn85Mm,
    OneHalfColumn114Mm,
    DoubleColumn174Mm,
    LetterHalf,
    CustomPt(f64, f64),
}

impl JournalWidthPreset {
    /// Return `(width_pt, height_pt)` in exact typographic points (`1 in = 72 pt`, `1 mm = 72.0 / 25.4 pt`).
    #[must_use]
    pub fn size_pt(self) -> (f64, f64) {
        match self {
            Self::SingleColumn85Mm => (85.0 * PT_PER_MM, 85.0 * PT_PER_MM),
            Self::OneHalfColumn114Mm => (114.0 * PT_PER_MM, 90.0 * PT_PER_MM),
            Self::DoubleColumn174Mm => (174.0 * PT_PER_MM, 120.0 * PT_PER_MM),
            Self::LetterHalf => (5.5 * PT_PER_INCH, 8.5 * PT_PER_INCH),
            Self::CustomPt(w, h) => (finite_non_neg(w), finite_non_neg(h)),
        }
    }
}

/// Single tick entry produced by [`build_ruler_ticks`] for a horizontal or vertical page ruler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RulerTick {
    pub pos_pt: f64,
    pub view_px: f64,
    pub major: bool,
    pub label: Option<String>,
}

/// Build zoom-adaptive ruler ticks across `[0, doc_span_pt]` in `unit`, preventing overlapping labels
/// and capping against zero, non-finite, or oversized inputs.
#[must_use]
pub fn build_ruler_ticks(unit: PhysicalUnit, doc_span_pt: f64, zoom: f64, pan_pt: f64, px_per_pt: f64) -> Vec<RulerTick> {
    if !doc_span_pt.is_finite()
        || doc_span_pt <= 0.0
        || !zoom.is_finite()
        || zoom <= 0.0
        || !pan_pt.is_finite()
        || !px_per_pt.is_finite()
        || px_per_pt <= 0.0
    {
        return Vec::new();
    }

    let span_pt = doc_span_pt.min(MAX_RULER_SPAN_PT);
    let pt_per_unit = unit.pt_per_unit();
    let px_per_unit = pt_per_unit * zoom * px_per_pt;
    if !px_per_unit.is_finite() || px_per_unit <= 0.0 {
        return Vec::new();
    }

    let span_units = span_pt / pt_per_unit;
    let min_major_for_zoom = MIN_MAJOR_TICK_PX / px_per_unit;
    #[allow(clippy::cast_precision_loss)]
    let min_major_for_cap = span_units / (MAX_RULER_MAJOR_TICKS as f64);
    let min_major_units = min_major_for_zoom.max(min_major_for_cap);
    if !min_major_units.is_finite() || min_major_units <= 0.0 {
        return Vec::new();
    }

    let (major_step_units, subdivisions) = select_ruler_step(unit, min_major_units, px_per_unit);
    let minor_step_units = major_step_units / f64::from(subdivisions);
    let decimals = step_decimal_places(major_step_units);

    let max_steps_f = (span_units / minor_step_units).floor();
    if !max_steps_f.is_finite() || max_steps_f < 0.0 {
        return Vec::new();
    }
    #[allow(clippy::cast_possible_truncation)]
    let cap_u32 = MAX_RULER_TICKS.saturating_sub(1) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let max_steps = if max_steps_f >= f64::from(cap_u32) { cap_u32 } else { max_steps_f as u32 };

    let mut ticks = Vec::with_capacity((max_steps as usize).saturating_add(1).min(MAX_RULER_TICKS));
    for step_idx in 0..=max_steps {
        let is_major = step_idx % subdivisions == 0;
        let value_units = f64::from(step_idx) * minor_step_units;
        let pos_pt = value_units * pt_per_unit;
        if pos_pt > span_pt * (1.0 + 1e-9) {
            break;
        }
        let view_px = (pos_pt * zoom + pan_pt) * px_per_pt;
        if !view_px.is_finite() {
            break;
        }
        let label = is_major.then(|| format_tick_label(value_units, decimals));
        ticks.push(RulerTick { pos_pt: pos_pt.min(span_pt), view_px, major: is_major, label });
    }
    ticks
}

fn select_ruler_step(unit: PhysicalUnit, min_major_units: f64, px_per_unit: f64) -> (f64, u32) {
    let (major_step, mut subdivisions) = match unit {
        PhysicalUnit::Inches if min_major_units < 1.0 - 1e-9 => select_binary_inch_step(min_major_units),
        PhysicalUnit::Inches => select_decimal_step(min_major_units, true),
        PhysicalUnit::Millimeters | PhysicalUnit::Points => select_decimal_step(min_major_units, false),
    };
    while subdivisions > 1 && (major_step / f64::from(subdivisions)) * px_per_unit < MIN_MINOR_TICK_PX {
        subdivisions = match subdivisions {
            5 | 4 => 2,
            _ => 1,
        };
    }
    (major_step, subdivisions)
}

fn select_binary_inch_step(min_major_units: f64) -> (f64, u32) {
    const INCH_STEPS: &[(f64, u32)] = &[(0.015_625, 2), (0.031_25, 2), (0.062_5, 4), (0.125, 4), (0.25, 4), (0.5, 4), (1.0, 4)];
    for &(step, subs) in INCH_STEPS {
        if step >= min_major_units * (1.0 - 1e-9) {
            return (step, subs);
        }
    }
    (1.0, 4)
}

fn select_decimal_step(min_major_units: f64, quarter_ones: bool) -> (f64, u32) {
    if min_major_units > 1e30 {
        return (min_major_units, 1);
    }
    let clamped = min_major_units.clamp(1e-6, 1e30);
    #[allow(clippy::cast_possible_truncation)]
    let exp = clamped.log10().floor() as i32;
    let base = 10.0_f64.powi(exp);
    let ones_subs = if quarter_ones && exp == 0 { 4 } else { 5 };
    let tens_subs = if quarter_ones && exp == -1 { 4 } else { 5 };
    let candidates = [(1.0, ones_subs), (2.0, 4), (5.0, 5), (10.0, tens_subs), (20.0, 4), (50.0, 5)];
    for (mult, subs) in candidates {
        let step = mult * base;
        if step >= clamped * (1.0 - 1e-9) {
            return (step, subs);
        }
    }
    (10.0 * base, 5)
}

fn step_decimal_places(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 || step >= 1.0 - 1e-9 {
        return 0;
    }
    let mut scaled = step;
    for d in 1..=6 {
        scaled *= 10.0;
        if (scaled - scaled.round()).abs() < 1e-6 {
            return d;
        }
    }
    4
}

fn format_tick_label(value: f64, decimals: usize) -> String {
    let clean = if value.abs() < 1e-12 { 0.0 } else { value };
    if decimals == 0 {
        return format!("{clean:.0}");
    }
    let mut s = format!("{clean:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" { String::from("0") } else { s }
}

/// Drag-and-drop sheet reorder state within a Project Navigator folder (`L4`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NavigatorDragState {
    pub active_drag: Option<(SheetFolderKind, String, usize)>,
    pub hover_target_index: Option<usize>,
}

impl NavigatorDragState {
    /// Start dragging `sheet_id` from `from_index` inside `folder`.
    pub fn begin_drag(&mut self, folder: SheetFolderKind, sheet_id: impl Into<String>, from_index: usize) {
        self.active_drag = Some((folder, sheet_id.into(), from_index));
        self.hover_target_index = None;
    }

    /// Update the hovered insertion index while a drag is active.
    pub fn update_hover(&mut self, to_index: usize) {
        if self.active_drag.is_some() {
            self.hover_target_index = Some(to_index);
        }
    }

    /// Commit an active drag-reorder drop, returning `(folder, from_index, to_index)` when moved.
    pub fn commit_drop(&mut self) -> Option<(SheetFolderKind, usize, usize)> {
        let (folder, _sheet_id, from_index) = self.active_drag.take()?;
        let to_index = self.hover_target_index.take()?;
        if from_index == to_index {
            return None;
        }
        Some((folder, from_index, to_index))
    }

    /// Cancel any active drag operation and clear hover state.
    pub fn cancel(&mut self) {
        self.active_drag = None;
        self.hover_target_index = None;
    }
}

#[cfg(test)]
mod tests {
    use lumencraft_doc::{AxisSegment, BreakOrnament, ColumnDesignHint, ColumnTable, DataSheet, GraphAxisScale, PageOrientation, TableArchetype};

    use super::*;

    fn sample_scene() -> ChartScene {
        ChartScene {
            sheet_id: "g-1".into(),
            title: "Sample".into(),
            width_pt: 400.0,
            height_pt: 300.0,
            background: Some(Rgba8::WHITE),
            plot_rect_pt: RectPt::new(60.0, 40.0, 280.0, 200.0),
            primitives: Vec::new(),
            warnings: Vec::new(),
            plotted_points: Vec::new(),
        }
    }

    fn sample_data_sheet() -> DataSheet {
        let col = ColumnTable::new(ColumnDesignHint::UnpairedReplicates, 3, 5).expect("col");
        DataSheet::new("t-1", "Sample Data", TableArchetype::Column(col)).expect("data sheet")
    }

    fn broken_y_graph_spec() -> GraphSpec {
        let ds = sample_data_sheet();
        let mut spec = GraphSpec::default_for_data_sheet(&ds);
        let mut y_axis = AxisSpec::linear("Response");
        y_axis.gap_width_pt = 8.0;
        y_axis.break_ornament = BreakOrnament::DoubleSlash;
        y_axis.segments = vec![
            AxisSegment { min: Some(0.0), max: Some(50.0), scale: GraphAxisScale::Linear, length_share: 0.4, ..AxisSegment::default() },
            AxisSegment { min: Some(100.0), max: Some(500.0), scale: GraphAxisScale::Linear, length_share: 0.6, ..AxisSegment::default() },
        ];
        spec.y_axis = y_axis;
        spec.annotations.push(GraphAnnotation::EmbeddedResultTable {
            result_sheet_id: SheetId::new("r-1"),
            tab_id: Some("summary".into()),
            bounds_pt: DocRectPt { x: 200.0, y: 60.0, width: 90.0, height: 50.0 },
        });
        spec
    }

    #[test]
    fn cursor_maps_all_five_variants_accurately() {
        let mut state = CanvasToolState::default();
        assert_eq!(state.cursor(), Cursor::Select);

        state.hover_target = Some(CanvasHitTarget::EmbeddedTable { index: 0 });
        assert_eq!(state.cursor(), Cursor::CellGrid);

        state.hover_target = Some(CanvasHitTarget::AxisBreak { kind: AxisKind::Y, segment_index: 1 });
        assert_eq!(state.cursor(), Cursor::AxisResize);

        state.set_mode(CanvasToolMode::Bracket);
        assert_eq!(state.cursor(), Cursor::Crosshair);
        state.set_mode(CanvasToolMode::TextCallout);
        assert_eq!(state.cursor(), Cursor::Crosshair);
        state.set_mode(CanvasToolMode::ArrowCallout);
        assert_eq!(state.cursor(), Cursor::Crosshair);

        state.set_mode(CanvasToolMode::AxisBreakResize);
        assert_eq!(state.cursor(), Cursor::AxisResize);

        state.set_mode(CanvasToolMode::Pan);
        assert_eq!(state.cursor(), Cursor::HandGrab);
    }

    #[test]
    fn graph_and_layout_hit_testing_detects_targets() {
        let scene = sample_scene();
        let spec = broken_y_graph_spec();

        assert_eq!(hit_test_graph(&scene, &spec, PointPt::new(220.0, 80.0), 4.0), Some(CanvasHitTarget::EmbeddedTable { index: 0 }));
        assert_eq!(hit_test_graph(&scene, &spec, PointPt::new(100.0, 100.0), 4.0), Some(CanvasHitTarget::PlotArea));
        assert_eq!(hit_test_graph(&scene, &spec, PointPt::new(140.0, 240.0), 4.0), Some(CanvasHitTarget::Axis { kind: AxisKind::X }));
        assert_eq!(
            hit_test_graph(&scene, &spec, PointPt::new(60.0, 240.0), 4.0),
            Some(CanvasHitTarget::AxisEndpoint { kind: AxisKind::X, end: AxisEndpointKind::Min })
        );

        let breaks = axis_break_positions_pt(scene.plot_rect_pt, AxisKind::Y, &spec.y_axis);
        assert_eq!(breaks.len(), 1);
        assert_eq!(
            hit_test_graph(&scene, &spec, PointPt::new(60.0, breaks[0]), 4.0),
            Some(CanvasHitTarget::AxisBreak { kind: AxisKind::Y, segment_index: 1 })
        );

        let layout = LayoutSpec {
            orientation: PageOrientation::Landscape,
            align_plot_origins: true,
            items: vec![
                LayoutItem::LiveGraph {
                    id: "p1".into(),
                    graph_sheet_id: SheetId::new("g-1"),
                    bounds_pt: DocRectPt { x: 40.0, y: 40.0, width: 200.0, height: 150.0 },
                    scale_percent: 100.0,
                    badge_override: None,
                },
                LayoutItem::LiveTable {
                    id: "t1".into(),
                    source_sheet_id: SheetId::new("t-1"),
                    tab_id: Some("summary".into()),
                    bounds_pt: DocRectPt { x: 300.0, y: 60.0, width: 120.0, height: 80.0 },
                },
            ],
            ..LayoutSpec::default()
        };
        assert_eq!(
            hit_test_layout(&layout, PointPt::new(240.0, 190.0), 4.0),
            Some(CanvasHitTarget::PanelResizeHandle { index: 0, corner: ResizeHandleCorner::BottomRight })
        );
        assert_eq!(hit_test_layout(&layout, PointPt::new(340.0, 100.0), 4.0), Some(CanvasHitTarget::EmbeddedTable { index: 1 }));
        assert_eq!(hit_test_layout(&layout, PointPt::new(100.0, 100.0), 4.0), Some(CanvasHitTarget::LayoutPanel { index: 0 }));
    }

    #[test]
    fn two_click_bracket_and_callout_and_break_intents() {
        let mut state = CanvasToolState { mode: CanvasToolMode::Bracket, ..CanvasToolState::default() };
        let g1 = SheetId::new("g-1");

        assert!(
            state
                .click_bracket(
                    &g1,
                    BracketAnchor { pt: PointPt::new(220.0, 90.0), group_index: Some(2), data_value: Some(15.0), result_sheet_id: None },
                    BracketLabelMode::SummaryStars,
                )
                .is_none()
        );
        assert!(state.pending_bracket_start.is_some());

        let intent = state
            .click_bracket(
                &g1,
                BracketAnchor {
                    pt: PointPt::new(120.0, 95.0),
                    group_index: Some(0),
                    data_value: Some(18.0),
                    result_sheet_id: Some(SheetId::new("res-1")),
                },
                BracketLabelMode::SummaryStars,
            )
            .expect("second click emits intent");
        assert!(state.pending_bracket_start.is_none());
        assert_eq!(intent.command_id, "graph.addComparisonBrackets");
        assert_eq!(intent.params["brackets"][0]["group_a"], 0);
        assert_eq!(intent.params["brackets"][0]["group_b"], 2);
        assert_eq!(intent.params["brackets"][0]["result_sheet_id"], "res-1");

        state.set_mode(CanvasToolMode::ArrowCallout);
        state.begin_drag(PointPt::new(50.0, 50.0));
        let arrow = state.end_drag(&g1, PointPt::new(120.0, 90.0)).expect("arrow callout intent");
        assert_eq!(arrow.command_id, "graph.addAnnotation");

        let ds = sample_data_sheet();
        let mut sheet = GraphSheet::new_uninitialized_for_data(g1.clone(), &ds).expect("graph sheet");
        sheet.spec = broken_y_graph_spec();
        let axes_intent = state.apply_axis_break_resize(&sheet, AxisKind::Y, 1, 0.7, Some(10.0)).expect("axis intent");
        assert_eq!(axes_intent.command_id, "graph.setAxes");
        assert_eq!(axes_intent.params["y_axis"]["gap_width_pt"], 10.0);
    }

    #[test]
    fn layout_snap_guides_and_viewport_helpers() {
        let mut state = CanvasToolState::default();
        let layout = LayoutSpec {
            orientation: PageOrientation::Portrait,
            align_plot_origins: true,
            items: vec![
                LayoutItem::LiveGraph {
                    id: "left".into(),
                    graph_sheet_id: SheetId::new("g-1"),
                    bounds_pt: DocRectPt { x: 72.0, y: 72.0, width: 200.0, height: 140.0 },
                    scale_percent: 100.0,
                    badge_override: None,
                },
                LayoutItem::LiveGraph {
                    id: "right".into(),
                    graph_sheet_id: SheetId::new("g-2"),
                    bounds_pt: DocRectPt { x: 300.0, y: 75.0, width: 200.0, height: 140.0 },
                    scale_percent: 100.0,
                    badge_override: None,
                },
            ],
            ..LayoutSpec::default()
        };
        let intent = state
            .commit_layout_panel_bounds(&SheetId::new("l-1"), &layout, 1, RectPt::new(300.0, 75.0, 200.0, 140.0), &[PointPt::new(72.0, 72.0)])
            .expect("layout intent");
        assert_eq!(intent.command_id, "layout.setSpec");
        let snapped_spec: LayoutSpec = serde_json::from_value(intent.params["spec"].clone()).expect("deserialize layout spec");
        assert_eq!(layout_item_bounds(&snapped_spec.items[1]).y, 72.0);
        assert!(!state.active_snap_guides.is_empty());

        let anchor = PointPt::new(300.0, 200.0);
        let world_before = state.screen_to_scene(anchor);
        state.zoom_by_at(2.0, anchor);
        let world_after = state.screen_to_scene(anchor);
        assert!((world_before.x - world_after.x).abs() < 1e-9);
        assert!((world_before.y - world_after.y).abs() < 1e-9);

        state.fit_scene(PointPt::new(400.0, 300.0), PointPt::new(820.0, 620.0), 10.0);
        assert!((state.zoom - 2.0).abs() < 1e-9);
        state.reset_viewport();
        assert_eq!(state.zoom, 1.0);
        assert_eq!(state.pan_pt, PointPt::default());
    }

    #[test]
    fn wave0_json_roundtrip_backward_compatible() {
        let wave0_json = r#"{
            "mode": "Bracket",
            "zoom": 1.5,
            "pan_pt": { "x": 12.0, "y": -4.0 },
            "selected_object_id": null,
            "drag_start_pt": null,
            "snap_to_grid": true
        }"#;
        let state: CanvasToolState = serde_json::from_str(wave0_json).expect("deserialize wave0");
        assert_eq!(state.mode, CanvasToolMode::Bracket);
        assert_eq!(state.cursor(), Cursor::Crosshair);
        assert_eq!(state.snap_grid_pt, DEFAULT_SNAP_GRID_PT);
    }
}
