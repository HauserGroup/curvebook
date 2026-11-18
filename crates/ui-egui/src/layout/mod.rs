//! Single retained UI layout tree, canonical coordinates, paint primitives, and verification (`#248`, `R10.1`, `R10.2`, `R10.15`, `R10.16`, `R10.20`).

pub mod a11y;
pub mod canvas_viewport;
pub mod hit_test;
pub mod paint;
pub mod scroll;
pub mod verify;
pub mod widgets;

use std::collections::{BTreeMap, BTreeSet};

use curvebook_doc::{SheetId, SheetSection};
use curvebook_palette::Rgba8;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::UiApp;
use crate::compositor::{FramePalette, WorkspaceFrameLayout, WorkspacePixelRect};
use crate::dialogs_canvas::ActiveInspector;
use crate::interaction::{GridHitGeometry, InfoRowHitRect, NavigatorEntryHitRect, ScenePaperMapping, ViewportMetrics, WorkspaceLayoutMap};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::{DesktopTextMeasurer, ShapedTextLine, TextMeasurer, UiTextStyle};

pub use a11y::{AccessibilityMeta, FocusScope, WidgetRole, cycle_focus_in_scope, ordered_focus_targets};
pub use canvas_viewport::{
    CanvasViewportGeometry, compute_aspect_ratio_paper_mapping, should_show_graph_series_chip, should_show_layout_panel_chrome,
};
pub use hit_test::{find_scroll_container_at, hit_test_logical, hit_test_physical};
pub use paint::{PaintOp, PaintOpKind, paint_layout_tree};
pub use scroll::{ScrollAxes, ScrollMeta, ScrollStateStore};
pub use verify::{ParityReport, ParityViolation, verify_click_action_parity, verify_pixel_to_hit_box_parity};

/// Unique retained widget identifier (`LayoutTree::finalize` checks uniqueness).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WidgetId(pub String);

impl WidgetId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for WidgetId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for WidgetId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// Deterministic paint/hit-test Z-layer ordering (`Workspace` < ... < `ModalBackdrop` < `ModalDialog` < `Tooltip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ZLayer {
    Workspace = 0,
    FloatingOverlay = 10,
    SplitHandle = 20,
    DropdownMenu = 30,
    ModalBackdrop = 40,
    ModalDialog = 50,
    ModalDropdown = 60,
    ContextMenu = 70,
    Tooltip = 80,
}

/// Physical pixel rectangle inside the retained layout framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct LayoutRectPx {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl LayoutRectPx {
    #[must_use]
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self { x, y, width, height }
    }

    #[must_use]
    pub const fn from_workspace_rect(r: WorkspacePixelRect) -> Self {
        Self {
            x: if r.x > i32::MAX as u32 { i32::MAX } else { r.x as i32 },
            y: if r.y > i32::MAX as u32 { i32::MAX } else { r.y as i32 },
            width: r.width,
            height: r.height,
        }
    }

    #[must_use]
    pub const fn to_workspace_rect(self) -> WorkspacePixelRect {
        let x = if self.x <= 0 { 0 } else { self.x as u32 };
        let y = if self.y <= 0 { 0 } else { self.y as u32 };
        WorkspacePixelRect::new(x, y, self.width, self.height)
    }

    #[must_use]
    pub const fn right(self) -> i32 {
        self.x.saturating_add(if self.width > i32::MAX as u32 { i32::MAX } else { self.width as i32 })
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.y.saturating_add(if self.height > i32::MAX as u32 { i32::MAX } else { self.height as i32 })
    }

    #[must_use]
    pub const fn center(self) -> [i32; 2] {
        [
            self.x.saturating_add(if self.width / 2 > i32::MAX as u32 { i32::MAX } else { (self.width / 2) as i32 }),
            self.y.saturating_add(if self.height / 2 > i32::MAX as u32 { i32::MAX } else { (self.height / 2) as i32 }),
        ]
    }

    #[must_use]
    pub const fn contains_px(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    #[must_use]
    pub fn intersect(self, other: Self) -> Self {
        let x0 = i64::from(self.x).max(i64::from(other.x));
        let y0 = i64::from(self.y).max(i64::from(other.y));
        let x1 = (i64::from(self.x) + i64::from(self.width)).min(i64::from(other.x) + i64::from(other.width));
        let y1 = (i64::from(self.y) + i64::from(self.height)).min(i64::from(other.y) + i64::from(other.height));
        let w = u32::try_from((x1 - x0).max(0)).unwrap_or(0);
        let h = u32::try_from((y1 - y0).max(0)).unwrap_or(0);
        Self {
            x: i32::try_from(x0.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
            y: i32::try_from(y0.clamp(i64::from(i32::MIN), i64::from(i32::MAX))).unwrap_or(0),
            width: w,
            height: h,
        }
    }

    #[must_use]
    pub const fn inset(self, pad: u32) -> Self {
        let p = if pad > i32::MAX as u32 { i32::MAX } else { pad as i32 };
        Self {
            x: self.x.saturating_add(p),
            y: self.y.saturating_add(p),
            width: self.width.saturating_sub(pad.saturating_mul(2)),
            height: self.height.saturating_sub(pad.saturating_mul(2)),
        }
    }
}

/// Canonical physical + logical bounds for a single [`WidgetNode`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NodeBounds {
    pub rect_px: LayoutRectPx,
    pub visible_rect_px: LayoutRectPx,
    pub rect_logical: [f64; 4],
}

impl NodeBounds {
    /// Convert physical `rect_px` and active clip `clip_rect` into canonical logical + physical bounds.
    #[must_use]
    pub fn from_physical(rect_px: LayoutRectPx, clip_rect: Option<LayoutRectPx>, scale_factor: f64) -> Self {
        let s = if scale_factor.is_finite() && scale_factor > 0.0 { scale_factor } else { 1.0 };
        let visible_rect_px = clip_rect.map_or(rect_px, |c| rect_px.intersect(c));
        let rect_logical = [f64::from(rect_px.x) / s, f64::from(rect_px.y) / s, f64::from(rect_px.width) / s, f64::from(rect_px.height) / s];
        Self { rect_px, visible_rect_px, rect_logical }
    }
}

/// Index into [`LayoutTree::clips`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipId(pub usize);

/// Visual interaction states carried on every [`WidgetNode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetVisualState {
    pub hovered: bool,
    pub active: bool,
    pub focused: bool,
    pub enablement: ControlEnablement,
    pub ellipsized: bool,
}

impl Default for WidgetVisualState {
    fn default() -> Self {
        Self { hovered: false, active: false, focused: false, enablement: ControlEnablement::enabled(), ellipsized: false }
    }
}

/// Semantic action executed when a user activates a widget via click or keyboard.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum WidgetAction {
    #[default]
    NoOp,
    MenuOpen(String),
    InvokeCommand {
        command_id: String,
        params: Value,
    },
    SwitchSheet {
        sheet_id: SheetId,
        section: SheetSection,
    },
    ToggleFolder(SheetSection),
    ToggleFamilyPane,
    SetSplitRatio(f64),
    SwapSplitPanes,
    SelectGridCell {
        row: u32,
        col: u32,
        subcol: u16,
    },
    StartInlineGridEdit {
        row: u32,
        col: u32,
        subcol: u16,
    },
    FocusWidget(String),
    ScrollBy {
        container_id: String,
        delta_x: f64,
        delta_y: f64,
    },
    ScrollbarTrackJump {
        container_id: String,
        vertical: bool,
        ratio: f64,
    },
    SelectResultTab(String),
    SelectAnalyzeCategory(String),
    SelectAnalyzeEntry(String),
    SelectEditParamsTab(String),
    ToggleDialogCheckbox(String),
    SelectDialogRadio {
        group: String,
        value: String,
    },
    OpenDropdown(String),
    SelectDropdownOption {
        dropdown_id: String,
        value: String,
    },
    StepSpinner {
        field_id: String,
        delta: f64,
    },
    SetSlider {
        field_id: String,
        normalized: f64,
    },
    DialogConfirm,
    DialogCancel,
    InvokeDialogButton(String),
    ClickDialogControl(String),
    ToggleInspectorTab(ActiveInspector),
    SetCanvasZoomFit,
    CycleCanvasZoomPreset,
    SelectCanvasTool(String),
    SelectLayoutPanel(String),
    ClosePopupOrModal,
    Custom {
        domain: String,
        payload: String,
    },
}

/// Single retained node in [`LayoutTree`], driving paint, hit-testing, accessibility, and `inspect.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetNode {
    pub id: WidgetId,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub paint_order: u32,
    pub clip: Option<ClipId>,
    pub scroll: Option<ScrollMeta>,
    pub bounds: NodeBounds,
    pub a11y: AccessibilityMeta,
    pub state: WidgetVisualState,
    pub action: WidgetAction,
    pub drawn_in_frame: bool,
}

/// Single rendered text run recorded for `R10.9` missing-glyph and `R10.2` clipping/overlap verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutTextRun {
    pub owner_widget_id: WidgetId,
    pub text: String,
    pub style: UiTextStyle,
    pub z_layer: ZLayer,
    pub clip: Option<ClipId>,
    pub scroll_container_id: Option<String>,
    pub origin_px: [f64; 2],
    pub baseline_y_px: f64,
    pub bounds_px: LayoutRectPx,
    pub shaped: ShapedTextLine,
    pub ellipsized: bool,
}

/// Cached [`LayoutTree`] keyed by viewport metrics and state fingerprint on `UiApp`.
#[derive(Debug, Clone, PartialEq)]
pub struct RetainedLayoutFrame {
    pub metrics: ViewportMetrics,
    pub state_hash: u64,
    pub tree: LayoutTree,
}

/// Single retained workspace + modal layout tree powering rendering, hit-testing, and `inspect_ui_tree`.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree {
    pub metrics: ViewportMetrics,
    pub is_rtl: bool,
    pub locale: String,
    pub clips: Vec<LayoutRectPx>,
    pub nodes: Vec<WidgetNode>,
    pub index_by_id: BTreeMap<String, usize>,
    pub paint_ops: Vec<PaintOp>,
    pub text_runs: Vec<LayoutTextRun>,
    pub scene_papers: Vec<ScenePaperMapping>,
    pub active_focus_scope: FocusScope,
    pub frame: WorkspaceFrameLayout,
    pub navigator_entries: Vec<NavigatorEntryHitRect>,
    pub split_primary: Option<WorkspacePixelRect>,
    pub split_divider: Option<WorkspacePixelRect>,
    pub split_secondary: Option<WorkspacePixelRect>,
    pub grid_geometry: Option<GridHitGeometry>,
    pub result_banner_rect: Option<WorkspacePixelRect>,
    pub result_banner_sheet_id: Option<SheetId>,
    pub canvas_title_rect: Option<WorkspacePixelRect>,
    pub x_axis_title_rect: Option<WorkspacePixelRect>,
    pub y_axis_title_rect: Option<WorkspacePixelRect>,
    pub info_constant_rows: Vec<InfoRowHitRect>,
    pub info_notes_rect: Option<WorkspacePixelRect>,
    pub info_sheet_id: Option<SheetId>,
}

impl LayoutTree {
    /// Construct an empty tree prepared for `metrics`.
    #[must_use]
    pub fn empty(app: &UiApp, metrics: ViewportMetrics) -> Self {
        Self {
            metrics,
            is_rtl: crate::i18n::is_rtl_locale(&app.locale),
            locale: app.locale.clone(),
            clips: Vec::new(),
            nodes: Vec::new(),
            index_by_id: BTreeMap::new(),
            paint_ops: Vec::new(),
            text_runs: Vec::new(),
            scene_papers: Vec::new(),
            active_focus_scope: FocusScope::Workspace,
            frame: WorkspaceFrameLayout::default(),
            navigator_entries: Vec::new(),
            split_primary: None,
            split_divider: None,
            split_secondary: None,
            grid_geometry: None,
            result_banner_rect: None,
            result_banner_sheet_id: None,
            canvas_title_rect: None,
            x_axis_title_rect: None,
            y_axis_title_rect: None,
            info_constant_rows: Vec::new(),
            info_notes_rect: None,
            info_sheet_id: None,
        }
    }

    /// Build and finalize the complete [`LayoutTree`] for `(app, metrics)`.
    #[must_use]
    pub fn build(app: &UiApp, metrics: ViewportMetrics) -> Self {
        let measurer = DesktopTextMeasurer::global();
        let palette = FramePalette::from_theme(&app.theme);
        crate::views::build_workspace_layout_tree(app, metrics, measurer, &palette)
    }

    /// Lookup a widget node by exact `WidgetId` string.
    #[must_use]
    pub fn node(&self, id: &str) -> Option<&WidgetNode> {
        self.index_by_id.get(id).and_then(|&idx| self.nodes.get(idx))
    }

    /// Validate unique `WidgetId`s, sort `nodes` and `paint_ops` by `(z_layer, paint_order)`,
    /// compute `drawn_in_frame` from emitted geometry, and rebuild [`Self::index_by_id`].
    ///
    /// # Errors
    /// Returns duplicate widget IDs if any builder registered two nodes with identical [`WidgetId`].
    pub fn finalize(&mut self) -> Result<(), Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut duplicates = Vec::new();
        for node in &self.nodes {
            if !seen.insert(node.id.0.clone()) {
                duplicates.push(node.id.0.clone());
            }
        }

        self.nodes.sort_by_key(|n| (n.z_layer, n.paint_order));
        self.paint_ops.sort_by_key(|op| (op.z_layer, op.paint_order));

        let viewport =
            LayoutRectPx::new(0, 0, i32::try_from(self.metrics.physical_width_px()).unwrap_or(i32::MAX) as u32, self.metrics.physical_height_px());
        let mut drawn_owners: BTreeSet<String> = BTreeSet::new();
        for op in &self.paint_ops {
            if let Some(owner) = &op.owner_widget_id
                && op.emits_visible_geometry(&self.clips, viewport)
            {
                drawn_owners.insert(owner.0.clone());
            }
        }

        self.index_by_id.clear();
        for (idx, node) in self.nodes.iter_mut().enumerate() {
            node.drawn_in_frame = drawn_owners.contains(node.id.as_str());
            self.index_by_id.insert(node.id.0.clone(), idx);
        }

        if duplicates.is_empty() { Ok(()) } else { Err(duplicates) }
    }

    /// Project the legacy [`WorkspaceLayoutMap`] surface directly from this single layout tree.
    #[must_use]
    pub fn to_workspace_layout_map(&self) -> WorkspaceLayoutMap {
        WorkspaceLayoutMap {
            metrics: self.metrics,
            is_rtl: self.is_rtl,
            frame: self.frame,
            navigator_entries: self.navigator_entries.clone(),
            split_primary: self.split_primary,
            split_divider: self.split_divider,
            split_secondary: self.split_secondary,
            grid_geometry: self.grid_geometry.clone(),
            scene_paper: self.scene_papers.first().cloned(),
            result_banner_rect: self.result_banner_rect,
            result_banner_sheet_id: self.result_banner_sheet_id.clone(),
            canvas_title_rect: self.canvas_title_rect,
            x_axis_title_rect: self.x_axis_title_rect,
            y_axis_title_rect: self.y_axis_title_rect,
            info_constant_rows: self.info_constant_rows.clone(),
            info_notes_rect: self.info_notes_rect,
            info_sheet_id: self.info_sheet_id.clone(),
        }
    }
}

/// Reusable mutable builder used by `views/mod.rs` and Wave 5 view modules to emit widgets & paint ops.
pub struct LayoutBuilder<'a> {
    pub app: &'a UiApp,
    pub measurer: &'a DesktopTextMeasurer,
    pub palette: &'a FramePalette,
    pub tree: LayoutTree,
    clip_stack: Vec<ClipId>,
    next_paint_order: u32,
}

/// Complete descriptor passed into [`LayoutBuilder::push_node`].
#[derive(Debug, Clone)]
pub struct NodeSpec {
    pub id: WidgetId,
    pub parent: Option<WidgetId>,
    pub z_layer: ZLayer,
    pub rect_px: LayoutRectPx,
    pub a11y: AccessibilityMeta,
    pub state: WidgetVisualState,
    pub action: WidgetAction,
}

/// Complete descriptor passed into [`LayoutBuilder::paint_text`].
#[derive(Debug, Clone)]
pub struct TextRunSpec<'b> {
    pub z_layer: ZLayer,
    pub owner: &'b WidgetId,
    pub scroll_container_id: Option<&'b str>,
    pub pos_px: [f64; 2],
    pub text: &'b str,
    pub style: &'b UiTextStyle,
    pub color: Rgba8,
    pub ellipsized: bool,
}

impl<'a> LayoutBuilder<'a> {
    #[must_use]
    pub fn new(app: &'a UiApp, metrics: ViewportMetrics, measurer: &'a DesktopTextMeasurer, palette: &'a FramePalette) -> Self {
        let mut tree = LayoutTree::empty(app, metrics);
        let root_clip = LayoutRectPx::new(0, 0, metrics.physical_width_px(), metrics.physical_height_px());
        tree.clips.push(root_clip);
        Self { app, measurer, palette, tree, clip_stack: vec![ClipId(0)], next_paint_order: 1 }
    }

    /// Push a clipped region intersected with the current top clip onto the clip stack.
    pub fn push_clip(&mut self, rect: LayoutRectPx) -> ClipId {
        let parent_clip = self.active_clip_rect();
        let combined = rect.intersect(parent_clip);
        let id = ClipId(self.tree.clips.len());
        self.tree.clips.push(combined);
        self.clip_stack.push(id);
        id
    }

    /// Pop the active clip region (preserving root viewport clip `0`).
    pub fn pop_clip(&mut self) {
        if self.clip_stack.len() > 1 {
            self.clip_stack.pop();
        }
    }

    #[must_use]
    pub fn active_clip_id(&self) -> ClipId {
        self.clip_stack.last().copied().unwrap_or(ClipId(0))
    }

    #[must_use]
    pub fn active_clip_rect(&self) -> LayoutRectPx {
        let id = self.active_clip_id();
        self.tree.clips.get(id.0).copied().unwrap_or_default()
    }

    #[must_use]
    pub fn scale(&self) -> f64 {
        self.tree.metrics.clamped_scale_factor()
    }

    #[must_use]
    pub fn scroll_offset(&self, container_id: &str) -> [f64; 2] {
        self.app.interaction.scroll_offsets.offset(container_id)
    }

    pub fn next_order(&mut self) -> u32 {
        let cur = self.next_paint_order;
        self.next_paint_order = self.next_paint_order.saturating_add(1);
        cur
    }

    /// Record a `WidgetNode` computing `bounds` from `rect_px` and active clip.
    pub fn push_node(&mut self, spec: NodeSpec) -> WidgetId {
        let clip_id = self.active_clip_id();
        let clip_rect = self.active_clip_rect();
        let order = self.next_order();
        let bounds = NodeBounds::from_physical(spec.rect_px, Some(clip_rect), self.scale());
        self.tree.nodes.push(WidgetNode {
            id: spec.id.clone(),
            parent: spec.parent,
            z_layer: spec.z_layer,
            paint_order: order,
            clip: Some(clip_id),
            scroll: None,
            bounds,
            a11y: spec.a11y,
            state: spec.state,
            action: spec.action,
            drawn_in_frame: false,
        });
        spec.id
    }

    /// Attach `ScrollMeta` to an already-pushed widget node by ID.
    pub fn attach_scroll(&mut self, id: &WidgetId, scroll: ScrollMeta) {
        if let Some(node) = self.tree.nodes.iter_mut().rev().find(|n| n.id == *id) {
            node.scroll = Some(scroll);
        }
    }

    /// Emit a solid rectangle fill primitive.
    pub fn paint_fill(&mut self, z_layer: ZLayer, owner: Option<&WidgetId>, rect: LayoutRectPx, color: Rgba8) {
        let order = self.next_order();
        self.tree.paint_ops.push(PaintOp {
            z_layer,
            paint_order: order,
            clip: Some(self.active_clip_id()),
            owner_widget_id: owner.cloned(),
            kind: PaintOpKind::FillRect { rect, color },
        });
    }

    /// Emit a stroked rectangle border primitive.
    pub fn paint_stroke(&mut self, z_layer: ZLayer, owner: Option<&WidgetId>, rect: LayoutRectPx, color: Rgba8, thickness: u32) {
        let order = self.next_order();
        self.tree.paint_ops.push(PaintOp {
            z_layer,
            paint_order: order,
            clip: Some(self.active_clip_id()),
            owner_widget_id: owner.cloned(),
            kind: PaintOpKind::StrokeRect { rect, color, thickness },
        });
    }

    /// Emit a line segment primitive.
    pub fn paint_line(&mut self, z_layer: ZLayer, owner: Option<&WidgetId>, p0: [i32; 2], p1: [i32; 2], color: Rgba8, thickness: u32) {
        let order = self.next_order();
        self.tree.paint_ops.push(PaintOp {
            z_layer,
            paint_order: order,
            clip: Some(self.active_clip_id()),
            owner_widget_id: owner.cloned(),
            kind: PaintOpKind::LineSegment { p0, p1, color, thickness },
        });
    }

    /// Shape, record in `tree.text_runs`, and emit a `PaintOp::TextRun` anchored at top-left `pos_px = [x, top_y]`.
    pub fn paint_text(&mut self, spec: TextRunSpec<'_>) -> ShapedTextLine {
        let shaped = self.measurer.shape_line(spec.text, spec.style, &self.tree.locale);
        let baseline_y = spec.pos_px[1] + shaped.metrics.ascent_px;
        #[allow(clippy::cast_possible_truncation)]
        let bx = spec.pos_px[0].floor() as i32;
        #[allow(clippy::cast_possible_truncation)]
        let by = spec.pos_px[1].floor() as i32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bw = shaped.metrics.width_px.ceil().max(1.0) as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bh = shaped.metrics.line_height_px.ceil().max(1.0) as u32;
        let clip_id = self.active_clip_id();

        if !spec.text.is_empty() {
            self.tree.text_runs.push(LayoutTextRun {
                owner_widget_id: spec.owner.clone(),
                text: spec.text.to_owned(),
                style: spec.style.clone(),
                z_layer: spec.z_layer,
                clip: Some(clip_id),
                scroll_container_id: spec.scroll_container_id.map(str::to_owned),
                origin_px: [spec.pos_px[0], baseline_y],
                baseline_y_px: baseline_y,
                bounds_px: LayoutRectPx::new(bx, by, bw, bh),
                shaped: shaped.clone(),
                ellipsized: spec.ellipsized,
            });
        }

        let order = self.next_order();
        self.tree.paint_ops.push(PaintOp {
            z_layer: spec.z_layer,
            paint_order: order,
            clip: Some(clip_id),
            owner_widget_id: Some(spec.owner.clone()),
            kind: PaintOpKind::TextRun {
                text: spec.text.to_owned(),
                style: spec.style.clone(),
                locale: self.tree.locale.clone(),
                origin_px: [spec.pos_px[0], baseline_y],
                color: spec.color,
                shaped: shaped.clone(),
            },
        });
        shaped
    }

    /// Finish building and finalize `LayoutTree`.
    #[must_use]
    pub fn finish(mut self) -> LayoutTree {
        let _ = self.tree.finalize();
        self.tree
    }
}

/// Deterministic fast FNV-1a state hash for [`RetainedLayoutFrame`] cache invalidation.
#[must_use]
pub fn compute_ui_state_fingerprint(app: &UiApp, metrics: ViewportMetrics) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed_bytes = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    feed_bytes(&metrics.logical_width.to_bits().to_le_bytes());
    feed_bytes(&metrics.logical_height.to_bits().to_le_bytes());
    feed_bytes(&metrics.scale_factor.to_bits().to_le_bytes());
    feed_bytes(&[
        u8::from(app.presentation_mode),
        u8::from(app.nav.navigator_open),
        u8::from(app.nav.inspector_open),
        u8::from(app.family_open),
        u8::from(app.canvas_dialogs.rulers_visible),
        u8::from(app.session().is_dirty()),
        u8::from(app.interaction.last_clipboard_text.as_deref().is_some_and(|t| !t.is_empty())),
        u8::from(app.interaction.last_clipboard_bundle.is_some()),
    ]);
    feed_bytes(app.locale.as_bytes());
    feed_bytes(&[app.theme.mode as u8]);
    let doc = app.session.document();
    feed_bytes(&(doc.data_sheets.len() as u64).to_le_bytes());
    feed_bytes(&(doc.result_sheets.len() as u64).to_le_bytes());
    feed_bytes(&(doc.graph_sheets.len() as u64).to_le_bytes());
    feed_bytes(&(doc.layout_sheets.len() as u64).to_le_bytes());
    feed_bytes(&(app.session().undo_depth() as u64).to_le_bytes());
    feed_bytes(&(app.session().redo_depth() as u64).to_le_bytes());
    if let Some(id) = &doc.active_sheet_id {
        feed_bytes(id.as_str().as_bytes());
        if let Some(ds) = doc.data_sheet(id) {
            feed_bytes(ds.title.as_bytes());
            feed_bytes(&[u8::from(ds.frozen)]);
        } else if let Some(rs) = doc.result_sheet(id) {
            feed_bytes(rs.title().as_bytes());
            feed_bytes(&(rs.tabs.len() as u64).to_le_bytes());
        }
    }
    if let Some(f) = &app.focus_widget_id {
        feed_bytes(f.as_bytes());
    }
    if let Some(hov) = &app.interaction.hovered_widget_id {
        feed_bytes(hov.as_bytes());
    }
    if let Some(hcmd) = &app.hover_command_id {
        feed_bytes(hcmd.as_bytes());
    }
    if let Some(htip) = &app.grid.hover_exclusion_tooltip {
        feed_bytes(htip.as_bytes());
    }
    feed_bytes(&(app.canvas_dialogs.canvas.zoom.to_bits()).to_le_bytes());
    feed_bytes(&(app.canvas_dialogs.canvas.pan_pt.x.to_bits()).to_le_bytes());
    feed_bytes(&(app.canvas_dialogs.canvas.pan_pt.y.to_bits()).to_le_bytes());
    feed_bytes(&app.mutation_seq().to_le_bytes());
    feed_bytes(&[u8::from(app.inline_editor.active), u8::from(app.grid.formula_bar.active)]);
    feed_bytes(app.inline_editor.buffer.as_bytes());
    feed_bytes(app.grid.formula_bar.input.as_bytes());
    feed_bytes(&(app.interaction.scroll_offsets.offsets.len() as u64).to_le_bytes());
    for (k, v) in &app.interaction.scroll_offsets.offsets {
        feed_bytes(k.as_bytes());
        feed_bytes(&v[0].to_bits().to_le_bytes());
        feed_bytes(&v[1].to_bits().to_le_bytes());
    }
    feed_bytes(&(app.grid.selected_rows.len() as u64).to_le_bytes());
    if let Some(first) = app.grid.selected_rows.first() {
        feed_bytes(&(*first as u64).to_le_bytes());
    }
    if let Some(last) = app.grid.selected_rows.last() {
        feed_bytes(&(*last as u64).to_le_bytes());
    }
    if let Ok(s) = serde_json::to_vec(&(
        &app.canvas_dialogs.active_dialog,
        &app.canvas_dialogs.analyze,
        &app.canvas_dialogs.edit_params,
        &app.canvas_dialogs.power_wizard.stage,
        &app.canvas_dialogs.active_inspector,
        &app.canvas_dialogs.result_view,
        &app.canvas_dialogs.canvas.selected_object_id,
        &app.canvas_dialogs.preflight_issues.len(),
        &app.split,
        &app.interaction.open_dropdown,
    )) {
        feed_bytes(&s);
    }
    if let Ok(s2) = serde_json::to_vec(&(
        &app.interaction.open_menu,
        &app.interaction.pressed_widget_id,
        &app.interaction.context_menu_target,
        &app.interaction.canvas_drag,
        &app.canvas_dialogs.canvas.active_snap_guides,
        &app.interaction.text_field_buffers,
        &app.brushed_cell,
        &app.brushed_points,
        &app.preview_filter,
        &app.grid.tool.viewport,
    )) {
        feed_bytes(&s2);
    }
    if let Ok(s3) = serde_json::to_vec(&(
        &app.grid.tool.active,
        &app.grid.quick_stats,
        &app.nav.collapsed_sections,
        &app.nav.gallery,
        &app.nav.active_pane,
        &app.inline_editor.target,
        &app.view_shards,
    )) {
        feed_bytes(&s3);
    }
    if let Ok(s4) = serde_json::to_vec(&(
        &app.nav.welcome,
        &app.nav.floating_notes_collapsed,
        &app.nav.search_query,
        &app.nav.navigator_drag,
        &app.power_wizards_by_sheet,
        &app.grid.import_reshape,
    )) {
        feed_bytes(&s4);
    }
    if let Ok(s5) = serde_json::to_vec(&(
        &app.canvas_dialogs.convert_archetype,
        &app.canvas_dialogs.export_figure,
        &app.canvas_dialogs.change_graph_type,
        &app.canvas_dialogs.hook_picker,
        &app.canvas_dialogs.format_graph,
        &app.canvas_dialogs.format_axes,
        &app.canvas_dialogs.bubble_plot,
        &app.canvas_dialogs.pairwise,
        &app.canvas_dialogs.page_layout,
        &app.canvas_dialogs.format_points,
        &app.canvas_dialogs.print,
    )) {
        feed_bytes(&s5);
    }
    h
}
