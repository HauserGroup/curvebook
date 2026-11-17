//! Live right-hand inspector view builder (`R10.18`).
//!
//! Renders 5 responsive sub-tabs (`Format Graph`, `Format Axes`, `Multivariable Channels`,
//! `Pairwise & CLD`, `Page Layout`) and interactive `Point & Drill-Back` controls,
//! dispatching instant undoable engine commands and reflecting live state on undo/redo.

use super::mv_formula::{LevelMoveOp, apply_explicit_level_move, resolve_selected_level_idx};
use crate::i18n::w5_inspector as w5_i18n;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use curvebook_chart::{RectPt, SourceCellId};
use curvebook_doc::{
    AnalysisKind, AxisScale, AxisSegment, AxisSpec, BracketLabelMode, BreakOrnament, CanvasResizeMode, CldOrdering, ColorChannelMapping,
    CompactLetterDisplaySpec, ConnectingLineStyle, ErrorBarDirection, ErrorBarMetric, ErrorBarSpec, FigureCanvasPreset, GraphKind, GraphSheet,
    GraphicalDataObject, GroupedArrangement, LayoutItem, LayoutSheet, LayoutSpec, LevelOrderKind, LevelOrderSpec, LineConnectionKind,
    MultivariateGraphEncoding, MultivariatePlotMode, PairwiseDisplayMode, PhysicalUnit, ProjectDocument, ReplicateOverlayStyle, SeriesSpec, SheetId,
    SizeChannelMapping, SummaryMeasure, TableArchetype, VariableRole, VariableSource, WhiskerMethod,
};
use curvebook_math::LogitDomain;
use curvebook_palette::{CanvasPreviewFilter, ColorRamp, ColormapId, LineDashStyle, MarkerShape, MarkerStyle, PaletteId, Rgba8};
use curvebook_tools::GridCellCoord;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveInspector, data_column_title};
use crate::interaction::UiEventOutcome;
use crate::layout::a11y::{AccessibilityMeta, FocusScope, WidgetRole};
use crate::layout::paint::{PaintOp, PaintOpKind};
use crate::layout::scroll::{ScrollAxes, ScrollMeta};
use crate::layout::widgets::{
    ButtonWidgetSpec, FittedTextSpec, TextFieldSpec, emit_button, emit_scrollbars_for_meta, emit_tab, emit_text_field, paint_text_fitted,
};
use crate::layout::{ClipId, LayoutBuilder, LayoutRectPx, NodeSpec, WidgetAction, WidgetId, WidgetVisualState, ZLayer};
use crate::modal_predicates::ControlEnablement;
use crate::text_bridge::UiTextStyle;
use crate::workspace_nav::ActiveWorkspacePane;

const SCROLL_ID: &str = "scroll.inspector";
const SCALE_BAR_ITEM_ID: &str = "inspector-scale-bar";
const AXIS_MIN_INPUT_ID: &str = "inspector.axes.min.input";
const AXIS_MAX_INPUT_ID: &str = "inspector.axes.max.input";
const AXIS_INTERVAL_INPUT_ID: &str = "inspector.axes.interval.input";
const AXIS_TITLE_INPUT_ID: &str = "inspector.axes.title.input";
const MV_CLAMP_MIN_INPUT_ID: &str = "inspector.mv.color_clamp_min.input";
const MV_CLAMP_MAX_INPUT_ID: &str = "inspector.mv.color_clamp_max.input";

const COLOR_SWATCHES: [(&str, &str); 8] = [
    ("Black", "#000000"),
    ("Orange", "#E69F00"),
    ("Sky", "#56B4E9"),
    ("Green", "#009E73"),
    ("Yellow", "#F0E442"),
    ("Blue", "#0072B2"),
    ("Vermilion", "#D55E00"),
    ("Purple", "#CC79A7"),
];

const SCALE_BAR_STEPS_MM: [u16; 4] = [5, 10, 25, 50];

/// Transient view state owned by the right-hand inspector shard.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InspectorViewState {
    pub collapsed_sections: Vec<String>,
    #[serde(default)]
    pub expanded_sections: Vec<String>,
    #[serde(default)]
    pub open_popup: Option<String>,
    #[serde(default)]
    pub selected_point_cursor: usize,
    #[serde(default)]
    pub selected_point_summary: String,
    #[serde(default)]
    pub page_margin_mm: u16,
    #[serde(default)]
    pub scale_bar_mm: u16,
    #[serde(default)]
    pub target_graph_id: Option<String>,
    #[serde(default)]
    pub target_layout_id: Option<String>,
    #[serde(default)]
    pub regression_result_sheet_id: Option<String>,
    #[serde(default)]
    pub regression_all_series: bool,
    #[serde(default)]
    pub regression_confidence_band: bool,
    #[serde(default)]
    pub regression_shaded_ribbon: bool,
    #[serde(default)]
    pub last_canvas_preset: Option<String>,
    #[serde(default)]
    pub selected_fg_level_name: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SectionKind {
    DrillBack,
    FormatGraph,
    FormatAxes,
    Multivariable,
    PairwiseCld,
    PageLayout,
}

impl SectionKind {
    const fn id(self) -> &'static str {
        match self {
            Self::DrillBack => "drill_back",
            Self::FormatGraph => "format_graph",
            Self::FormatAxes => "format_axes",
            Self::Multivariable => "multivariable",
            Self::PairwiseCld => "pairwise",
            Self::PageLayout => "page_layout",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum AxisWhich {
    X,
    Y,
    Y2,
}

#[derive(Clone, Debug)]
struct PopupOptionSpec {
    label: String,
    payload: String,
    active: bool,
}

#[derive(Clone, Debug)]
struct PendingPopup {
    kind: String,
    anchor: LayoutRectPx,
    options: Vec<PopupOptionSpec>,
}

fn wid(id: &str) -> WidgetId {
    WidgetId(String::from(id))
}

fn enablement_flag(enabled: bool) -> ControlEnablement {
    if enabled { ControlEnablement::enabled() } else { ControlEnablement::disabled("Unavailable for the active sheet") }
}

struct BtnArgs<'a> {
    id: &'a str,
    rect: LayoutRectPx,
    label: &'a str,
    a11y_label: &'a str,
    action: WidgetAction,
    parent: Option<WidgetId>,
    z_layer: ZLayer,
    selected: bool,
    enabled: bool,
    as_tab: bool,
}

fn push_btn_widget(builder: &mut LayoutBuilder<'_>, args: BtnArgs<'_>) {
    let kbd_path = format!("Inspector > {}", args.a11y_label);
    let spec = ButtonWidgetSpec {
        id: args.id,
        rect: args.rect,
        label: args.label,
        keyboard_path: &kbd_path,
        tab_order: Some(0),
        primary: false,
        selected: args.selected,
        action: args.action,
        parent: args.parent,
        focus_scope: FocusScope::Workspace,
        z_layer: args.z_layer,
        shortcut: None,
        enablement: enablement_flag(args.enabled),
    };
    if args.as_tab {
        emit_tab(builder, spec);
    } else {
        emit_button(builder, spec);
    }
}

fn insp_u32(base: u32, scale: f64) -> u32 {
    let s = if scale.is_finite() { scale.clamp(0.75, 4.0) } else { 1.0 };
    (f64::from(base) * s).round().clamp(1.0, 4096.0) as u32
}

fn insp_i32(base: i32, scale: f64) -> i32 {
    let s = if scale.is_finite() { scale.clamp(0.75, 4.0) } else { 1.0 };
    (f64::from(base) * s).round().clamp(-4096.0, 4096.0) as i32
}

fn insp_pt(base: f64, scale: f64) -> f64 {
    let s = if scale.is_finite() { scale.clamp(0.75, 4.0) } else { 1.0 };
    (base * s).clamp(8.5, 28.0)
}

struct SectionDrawCtx<'a, 'b> {
    builder: &'a mut LayoutBuilder<'b>,
    scroll_clip: LayoutRectPx,
    scroll_clip_id: ClipId,
    parent_wid: &'a WidgetId,
    panel_x: i32,
    panel_w: u32,
    cursor_y: i32,
    pending_popup: Option<PendingPopup>,
}

struct DropdownArgs<'a> {
    id: &'a str,
    y: i32,
    title: &'a str,
    current_val: &'a str,
    popup_kind: &'a str,
    open_popup: Option<&'a str>,
    options: Vec<PopupOptionSpec>,
    enabled: bool,
}

struct StepperArgs<'a> {
    y: i32,
    title: &'a str,
    value_display: &'a str,
    dec_id: &'a str,
    dec_payload: &'a str,
    inc_id: &'a str,
    inc_payload: &'a str,
    enabled: bool,
}

const fn rtl_row_x(row_x: i32, row_w: u32, rel_x: i32, item_w: u32, is_rtl: bool) -> i32 {
    if is_rtl { row_x + (row_w as i32) - rel_x - (item_w as i32) } else { row_x + rel_x }
}

impl<'a, 'b> SectionDrawCtx<'a, 'b> {
    fn is_rtl(&self) -> bool {
        self.builder.tree.is_rtl
    }

    fn mirrored_x(&self, rel_x: i32, item_w: u32) -> i32 {
        rtl_row_x(self.panel_x, self.panel_w, rel_x, item_w, self.is_rtl())
    }

    fn scale(&self) -> f64 {
        self.builder.scale()
    }

    fn su(&self, base: u32) -> u32 {
        insp_u32(base, self.scale())
    }

    fn si(&self, base: i32) -> i32 {
        insp_i32(base, self.scale())
    }

    fn sp(&self, base: f64) -> f64 {
        insp_pt(base, self.scale())
    }

    fn is_row_visible(&self, rect: LayoutRectPx) -> bool {
        rect.height > 0 && rect.width > 0 && rect.y >= self.scroll_clip.y && rect.bottom() <= self.scroll_clip.bottom()
    }

    fn paint_label(&mut self, text: &str, y: i32, h: u32, size_px: f64, color: Rgba8, bold: bool) {
        let scaled_h = self.su(h);
        let rect = LayoutRectPx::new(self.panel_x, y, self.panel_w, scaled_h);
        if !self.is_row_visible(rect) {
            return;
        }
        let style = if bold { UiTextStyle::ui_bold(self.sp(size_px)) } else { UiTextStyle::ui_sans(self.sp(size_px)) };
        paint_text_fitted(
            self.builder,
            self.parent_wid,
            FittedTextSpec { rect, text, style: &style, pad_x: 2, color, z_layer: ZLayer::Workspace, scroll_container: Some(SCROLL_ID) },
        );
    }

    fn emit_action_btn(&mut self, id: &str, rect: LayoutRectPx, labels: (&str, &str), payload: &str, active: bool, enabled: bool) {
        if !self.is_row_visible(rect) {
            return;
        }
        let action =
            if enabled { WidgetAction::Custom { domain: String::from("inspector"), payload: String::from(payload) } } else { WidgetAction::NoOp };
        push_btn_widget(
            self.builder,
            BtnArgs {
                id,
                rect,
                label: labels.0,
                a11y_label: labels.1,
                action,
                parent: Some(self.parent_wid.clone()),
                z_layer: ZLayer::Workspace,
                selected: active,
                enabled,
                as_tab: false,
            },
        );
    }

    fn emit_checkbox_row(&mut self, id: &str, y: i32, label: &str, checked: bool, payload: &str, enabled: bool) -> i32 {
        let rect = LayoutRectPx::new(self.panel_x, y, self.panel_w, self.su(24));
        if self.is_row_visible(rect) {
            let box_text = if checked { format!("[x] {label}") } else { format!("[ ] {label}") };
            self.emit_action_btn(id, rect, (&box_text, label), payload, checked, enabled);
        }
        y + self.si(28)
    }

    fn emit_dropdown_trigger(&mut self, args: DropdownArgs<'_>) -> i32 {
        let btn_rect = LayoutRectPx::new(self.panel_x, args.y + self.si(16), self.panel_w, self.su(24));
        let is_open = args.open_popup == Some(args.popup_kind);
        if self.is_row_visible(btn_rect) {
            let muted = self.builder.app.theme.text_muted;
            self.paint_label(args.title, args.y, 15, 10.5, muted, false);
            let marker = if is_open { " ^" } else { " v" };
            let label = format!("{}{}", args.current_val, marker);
            let popup_payload = format!("popup:{}", args.popup_kind);
            self.emit_action_btn(args.id, btn_rect, (&label, args.title), &popup_payload, is_open, args.enabled);
            if is_open && args.enabled && !args.options.is_empty() {
                self.pending_popup = Some(PendingPopup { kind: String::from(args.popup_kind), anchor: btn_rect, options: args.options });
            }
        }
        args.y + self.si(44)
    }

    fn emit_chip_row(&mut self, y: i32, title: Option<&str>, items: &[(&str, &str, &str, bool)], enabled: bool) -> i32 {
        self.emit_chip_row_capped(y, title, items, 4, enabled)
    }

    fn emit_chip_row_capped(
        &mut self,
        y: i32,
        title: Option<&str>,
        items: &[(&str, &str, &str, bool)],
        single_row_limit: usize,
        enabled: bool,
    ) -> i32 {
        let mut cy = y;
        if let Some(t) = title {
            let muted = self.builder.app.theme.text_muted;
            self.paint_label(t, cy, 15, 10.5, muted, false);
            cy += self.si(16);
        }
        let n = items.len().max(1);
        let max_per_row = if self.panel_w < self.su(210) {
            2
        } else if n > single_row_limit.max(1) {
            3
        } else {
            n
        };
        let gap = self.su(4);
        let btn_h = self.su(22);
        let step_y = self.si(26);
        let rows = n.div_ceil(max_per_row);
        for row_idx in 0..rows {
            let start = row_idx * max_per_row;
            let end = (start + max_per_row).min(items.len());
            let slice = &items[start..end];
            let cols = slice.len().max(1) as u32;
            let total_gaps = gap * (cols.saturating_sub(1));
            let btn_w = (self.panel_w.saturating_sub(total_gaps) / cols).max(self.su(24));
            for (idx, (id, lbl, payload, active)) in slice.iter().enumerate() {
                let bx = self.mirrored_x(((idx as u32) * (btn_w + gap)) as i32, btn_w);
                let rect = LayoutRectPx::new(bx, cy, btn_w, btn_h);
                self.emit_action_btn(id, rect, (lbl, lbl), payload, *active, enabled);
            }
            cy += step_y;
        }
        cy
    }

    fn emit_stepper_row(&mut self, args: StepperArgs<'_>) -> i32 {
        let fg = self.builder.app.theme.text_primary;
        let btn_w = self.su(27);
        let btn_h = self.su(22);
        let gap = self.su(4);
        let stepper_w = btn_w.saturating_mul(2).saturating_add(gap);
        let lbl_w = self.panel_w.saturating_sub(stepper_w.saturating_add(gap)).max(self.su(60));
        let lbl_rect = LayoutRectPx::new(self.mirrored_x(0, lbl_w), args.y + self.si(3), lbl_w, self.su(18));
        if self.is_row_visible(lbl_rect) {
            let txt = format!("{}: {}", args.title, args.value_display);
            let txt_style = UiTextStyle::ui_sans(self.sp(10.5));
            paint_text_fitted(
                self.builder,
                self.parent_wid,
                FittedTextSpec {
                    rect: lbl_rect,
                    text: &txt,
                    style: &txt_style,
                    pad_x: 2,
                    color: fg,
                    z_layer: ZLayer::Workspace,
                    scroll_container: Some(SCROLL_ID),
                },
            );
        }
        let step_rel = i32::try_from(self.panel_w.saturating_sub(stepper_w)).unwrap_or(0);
        let dec_rel = step_rel;
        let inc_rel = step_rel + i32::try_from(btn_w + gap).unwrap_or(31);
        let dec_rect = LayoutRectPx::new(self.mirrored_x(dec_rel, btn_w), args.y, btn_w, btn_h);
        let inc_rect = LayoutRectPx::new(self.mirrored_x(inc_rel, btn_w), args.y, btn_w, btn_h);
        self.emit_action_btn(args.dec_id, dec_rect, ("-", &format!("Decrease {}", args.title)), args.dec_payload, false, args.enabled);
        self.emit_action_btn(args.inc_id, inc_rect, ("+", &format!("Increase {}", args.title)), args.inc_payload, false, args.enabled);
        args.y + self.si(26)
    }
}

fn insp_text(app: &UiApp, key: &str, fallback: &'static str) -> &'static str {
    let norm = crate::i18n::normalize_locale(&app.locale);
    crate::i18n::try_tr(norm, key).or_else(|| crate::i18n::try_tr_in_slices(&[w5_i18n::ENTRIES], norm, key)).unwrap_or(fallback)
}

fn outcome_ok(payload: &str, cmd_id: Option<&str>) -> UiEventOutcome {
    UiEventOutcome { handled: true, action: format!("inspector.{payload}"), command_id: cmd_id.map(String::from), command_result: None }
}

fn section_for_active_inspector(app: &UiApp, state: &InspectorViewState) -> SectionKind {
    match app.canvas_dialogs.active_inspector {
        ActiveInspector::FormatGraph => SectionKind::FormatGraph,
        ActiveInspector::FormatAxes => SectionKind::FormatAxes,
        ActiveInspector::BubblePlot => SectionKind::Multivariable,
        ActiveInspector::PairwiseComparisons => SectionKind::PairwiseCld,
        ActiveInspector::PageLayout => SectionKind::PageLayout,
        ActiveInspector::ResultDetails => SectionKind::DrillBack,
        ActiveInspector::None => {
            let doc = app.document();
            if let Some(active_id) = &doc.active_sheet_id {
                if doc.layout_sheet(active_id).is_some() {
                    return SectionKind::PageLayout;
                }
                if let Some(gs) = doc.graph_sheet(active_id) {
                    if gs.spec.multivariate.is_some() {
                        return SectionKind::Multivariable;
                    }
                    return SectionKind::FormatGraph;
                }
            }
            if resolve_graph_sheet(doc, state).is_some() {
                SectionKind::FormatGraph
            } else if resolve_layout_sheet(doc, state).is_some() {
                SectionKind::PageLayout
            } else {
                SectionKind::FormatGraph
            }
        }
    }
}

fn is_section_expanded(app: &UiApp, state: &InspectorViewState, sec: SectionKind) -> bool {
    let id = sec.id();
    if state.collapsed_sections.iter().any(|s| s == id) {
        return false;
    }
    let default_sec = section_for_active_inspector(app, state);
    sec == default_sec || sec == SectionKind::DrillBack || state.expanded_sections.iter().any(|s| s == id)
}

fn resolve_graph_sheet<'a>(doc: &'a ProjectDocument, state: &InspectorViewState) -> Option<&'a GraphSheet> {
    if let Some(active) = &doc.active_sheet_id
        && let Some(gs) = doc.graph_sheet(active)
    {
        return Some(gs);
    }
    if let Some(tid) = &state.target_graph_id {
        let sid = SheetId::new(tid);
        if let Some(gs) = doc.graph_sheet(&sid) {
            return Some(gs);
        }
    }
    doc.graph_sheets.first()
}

fn resolve_layout_sheet<'a>(doc: &'a ProjectDocument, state: &InspectorViewState) -> Option<&'a LayoutSheet> {
    if let Some(active) = &doc.active_sheet_id
        && let Some(ls) = doc.layout_sheet(active)
    {
        return Some(ls);
    }
    if let Some(tid) = &state.target_layout_id {
        let sid = SheetId::new(tid);
        if let Some(ls) = doc.layout_sheet(&sid) {
            return Some(ls);
        }
    }
    doc.layout_sheets.first()
}

fn parse_hex_rgba8(hex: &str) -> Option<Rgba8> {
    let raw = hex.trim().trim_start_matches('#');
    if raw.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&raw[0..2], 16).ok()?;
    let g = u8::from_str_radix(&raw[2..4], 16).ok()?;
    let b = u8::from_str_radix(&raw[4..6], 16).ok()?;
    Some(Rgba8::rgb(r, g, b))
}

fn rgba_to_hex(c: Rgba8) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

fn palette_label(p: PaletteId) -> &'static str {
    match p {
        PaletteId::OkabeIto => "Okabe-Ito",
        PaletteId::PaulTolBright => "Tol Bright",
        PaletteId::PaulTolMuted => "Tol Muted",
        PaletteId::PaulTolHighContrast => "Tol High Contrast",
        PaletteId::PaulTolVibrant => "Tol Vibrant",
        PaletteId::MonochromePrint => "Monochrome",
    }
}

fn marker_shape_label(s: MarkerShape) -> &'static str {
    match s {
        MarkerShape::Circle => "Circle",
        MarkerShape::Square => "Square",
        MarkerShape::TriangleUp => "Triangle Up",
        MarkerShape::TriangleDown => "Triangle Down",
        MarkerShape::Diamond => "Diamond",
        MarkerShape::Cross => "Cross",
        MarkerShape::Plus => "Plus",
        MarkerShape::Star => "Star",
        MarkerShape::Hexagon => "Hexagon",
        MarkerShape::HorizontalBar => "H-Bar",
        MarkerShape::VerticalBar => "V-Bar",
    }
}

fn all_marker_shapes() -> &'static [MarkerShape] {
    &[
        MarkerShape::Circle,
        MarkerShape::Square,
        MarkerShape::TriangleUp,
        MarkerShape::TriangleDown,
        MarkerShape::Diamond,
        MarkerShape::Cross,
        MarkerShape::Plus,
        MarkerShape::Star,
    ]
}

fn series_primary_color(s: &SeriesSpec) -> Rgba8 {
    match &s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            marker.as_ref().map_or_else(|| line.as_ref().map_or(Rgba8::BLACK, |l| l.color), |m| m.fill_color)
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            if border_color.a > 0 {
                *border_color
            } else {
                *fill_color
            }
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, .. } => *fill_color,
        GraphicalDataObject::Violin { fill_color, .. } => *fill_color,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } => *line_color,
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => *line_color,
        GraphicalDataObject::PartsOfWhole { .. } => Rgba8::BLACK,
    }
}

fn series_marker_style(s: &SeriesSpec) -> MarkerStyle {
    match &s.gdo {
        GraphicalDataObject::ScatterAndLine { marker: Some(m), .. } | GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => *m,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => *marker,
        _ => {
            let c = series_primary_color(s);
            MarkerStyle { fill_color: c, stroke_color: c, ..MarkerStyle::default() }
        }
    }
}

fn set_series_primary_color(s: &mut SeriesSpec, rgba: Rgba8) {
    match &mut s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill } => {
            if let Some(m) = marker {
                m.fill_color = rgba;
                m.stroke_color = rgba;
            }
            if let Some(l) = line {
                l.color = rgba;
            }
            if let Some(af) = area_fill {
                af.fill_color = rgba.with_alpha(64);
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, border_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, border_color, .. } => {
            *fill_color = rgba.with_alpha(200);
            *border_color = rgba;
        }
        GraphicalDataObject::BoxAndWhisker { fill_color, .. } => {
            *fill_color = rgba.with_alpha(190);
        }
        GraphicalDataObject::Violin { fill_color, stroke_color, .. } => {
            *fill_color = rgba.with_alpha(190);
            *stroke_color = rgba;
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.fill_color = rgba;
            marker.stroke_color = rgba;
        }
        GraphicalDataObject::BeforeAfterLines { line_color, marker, .. } => {
            *line_color = rgba;
            if let Some(m) = marker {
                m.fill_color = rgba;
                m.stroke_color = rgba;
            }
        }
        GraphicalDataObject::KaplanMeierStep { line_color, .. } => {
            *line_color = rgba;
        }
        GraphicalDataObject::PartsOfWhole { .. } => {}
    }
}

fn set_series_marker_style(s: &mut SeriesSpec, next_m: MarkerStyle) {
    match &mut s.gdo {
        GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } => {
            *marker = Some(next_m);
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            *marker = next_m;
        }
        _ => {}
    }
}

fn topology_label(gdo: &GraphicalDataObject) -> &'static str {
    match gdo {
        GraphicalDataObject::BarWithReplicates { .. } | GraphicalDataObject::FloatingBar { .. } => "Bar",
        GraphicalDataObject::BoxAndWhisker { .. } => "Box",
        GraphicalDataObject::Violin { half_violin_with_points: true, .. } => "Raincloud",
        GraphicalDataObject::Violin { half_violin_with_points: false, .. } => "Violin",
        GraphicalDataObject::AlignedDotPlot { .. } => "Dot Plot",
        GraphicalDataObject::ScatterAndLine { .. } => "Scatter+Line",
        GraphicalDataObject::BeforeAfterLines { .. } => "Before/After",
        GraphicalDataObject::KaplanMeierStep { .. } => "Survival",
        GraphicalDataObject::PartsOfWhole { .. } => "Parts",
    }
}

fn axis_scale_label(s: AxisScale) -> &'static str {
    match s {
        AxisScale::Linear => "Linear",
        AxisScale::Log10 => "Log10",
        AxisScale::Log2 => "Log2",
        AxisScale::Ln => "Ln (Nonlin)",
        AxisScale::Logit(_) => "Logit",
        AxisScale::Probit => "Probit",
        AxisScale::Reciprocal => "1/Y",
    }
}

fn format_opt_num(val: Option<f64>) -> String {
    match val {
        Some(v) if v.is_finite() => {
            if v.fract().abs() < 1e-6 {
                format!("{}", v.round() as i64)
            } else {
                format!("{v:.3}")
            }
        }
        _ => String::from("Auto"),
    }
}

fn default_connecting_line(color: Rgba8) -> ConnectingLineStyle {
    ConnectingLineStyle { kind: LineConnectionKind::default(), color, dash_style: LineDashStyle::Solid, stroke_width_pt: 1.5 }
}

fn retopologize_series(series: &[SeriesSpec], topo: &str) -> Vec<SeriesSpec> {
    series
        .iter()
        .map(|s| {
            let mut next = s.clone();
            let col = series_primary_color(s);
            let cur_marker = series_marker_style(s);
            next.gdo = match topo {
                "bar" => GraphicalDataObject::BarWithReplicates {
                    summary: SummaryMeasure::Mean,
                    fill_color: col.with_alpha(200),
                    border_color: col,
                    border_width_pt: 1.0,
                    replicate_overlay: Some(ReplicateOverlayStyle::DeterministicBeeswarm),
                },
                "box" => GraphicalDataObject::BoxAndWhisker {
                    whiskers: WhiskerMethod::Tukey1_5Iqr,
                    fill_color: col.with_alpha(190),
                    stroke_color: Rgba8::BLACK,
                    show_outliers: true,
                    show_all_points: false,
                },
                "violin" | "raincloud" => GraphicalDataObject::Violin {
                    fill_color: col.with_alpha(190),
                    stroke_color: col,
                    clip_to_observed_range: true,
                    show_median_and_quartiles: true,
                    half_violin_with_points: topo == "raincloud",
                },
                "dot" => GraphicalDataObject::AlignedDotPlot {
                    summary: SummaryMeasure::Mean,
                    marker: MarkerStyle { fill_color: col, stroke_color: col, ..cur_marker },
                    show_crossbar: true,
                },
                "scatter" => GraphicalDataObject::ScatterAndLine {
                    marker: Some(MarkerStyle { fill_color: col, stroke_color: col, ..cur_marker }),
                    line: Some(default_connecting_line(col)),
                    area_fill: None,
                },
                _ => s.gdo.clone(),
            };
            next
        })
        .collect()
}

fn resolve_selected_series_index(app: &UiApp, gs: &GraphSheet) -> usize {
    let max_idx = gs.spec.series.len().saturating_sub(1);
    app.canvas_dialogs.format_graph.selected_series_index.min(max_idx)
}

struct MvLevelTarget {
    is_x: bool,
    levels: Vec<String>,
}

fn resolve_mv_level_target(doc: &ProjectDocument, gs: &GraphSheet) -> Option<MvLevelTarget> {
    let enc = gs.spec.multivariate.as_ref()?;
    let ds = doc.data_sheet(&gs.spec.primary_source_sheet_id)?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return None;
    };
    if let Some(col) = mv
        .variables
        .get(enc.x_column)
        .filter(|col| col.role == VariableRole::Categorical || matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }))
    {
        let spec = enc.x_level_order_override.as_ref().unwrap_or(&col.level_order);
        let levels: Vec<String> = col.ordered_levels_with(spec).into_iter().map(|l| l.level).collect();
        if !levels.is_empty() {
            return Some(MvLevelTarget { is_x: true, levels });
        }
    }
    if let ColorChannelMapping::Categorical { column, level_order_override, .. } = &enc.color
        && let Some(col) = mv.variables.get(*column)
    {
        let spec = level_order_override.as_ref().unwrap_or(&col.level_order);
        let levels: Vec<String> = col.ordered_levels_with(spec).into_iter().map(|l| l.level).collect();
        if !levels.is_empty() {
            return Some(MvLevelTarget { is_x: false, levels });
        }
    }
    None
}

fn mv_column_role_info(doc: &ProjectDocument, gs: &GraphSheet, col_idx: usize) -> Option<(VariableRole, bool)> {
    let ds = doc.data_sheet(&gs.spec.primary_source_sheet_id)?;
    let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
        return None;
    };
    let var = mv.variables.get(col_idx)?;
    Some((var.role, matches!(var.source, VariableSource::Calculated { .. }) || ds.frozen))
}

fn role_display_name(loc: &str, role: VariableRole) -> &'static str {
    match role {
        VariableRole::Continuous => crate::i18n::tr(loc, "w7.mv.var_role_continuous"),
        VariableRole::Categorical => crate::i18n::tr(loc, "w7.mv.var_role_categorical"),
        VariableRole::Label => crate::i18n::tr(loc, "w7.mv.var_role_label"),
        VariableRole::Calculated => "Calculated",
    }
}

fn dispatch_format_graph_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let series_count = gs.spec.series.len();
    let sel_idx = resolve_selected_series_index(app, &gs);

    if let Some(target) = sub.strip_prefix("dataset:") {
        if series_count == 0 {
            return Ok(UiEventOutcome::default());
        }
        let next_idx = match target {
            "prev" => (sel_idx + series_count - 1) % series_count,
            "next" => (sel_idx + 1) % series_count,
            raw => raw.parse::<usize>().unwrap_or(sel_idx).min(series_count - 1),
        };
        app.canvas_dialogs.format_graph.selected_series_index = next_idx;
        if let Some(live_gs) = app.session.document().graph_sheet(&gs.id) {
            app.canvas_dialogs.format_graph.sync_from_graph(live_gs);
        }
        state.open_popup = None;
        return Ok(outcome_ok(sub, None));
    }

    if let Some(pal_key) = sub.strip_prefix("palette:") {
        let mut spec = gs.spec.clone();
        let n = spec.series.len().max(1);
        match pal_key {
            "Viridis" => {
                for (i, s) in spec.series.iter_mut().enumerate() {
                    let t = if n <= 1 { 0.5 } else { (i as f64) / ((n - 1) as f64) };
                    set_series_primary_color(s, ColormapId::Viridis.sample(t));
                }
            }
            _ => {
                let pal = match pal_key {
                    "PaulTolBright" => PaletteId::PaulTolBright,
                    "PaulTolMuted" => PaletteId::PaulTolMuted,
                    "PaulTolHighContrast" => PaletteId::PaulTolHighContrast,
                    "PaulTolVibrant" => PaletteId::PaulTolVibrant,
                    "MonochromePrint" => PaletteId::MonochromePrint,
                    _ => PaletteId::OkabeIto,
                };
                spec.palette = pal;
                for (i, s) in spec.series.iter_mut().enumerate() {
                    set_series_primary_color(s, pal.color_at(i));
                }
            }
        }
        app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "palette": spec.palette, "series": spec.series } }))
            .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSpec")));
    }

    if let Some(hex) = sub.strip_prefix("color:") {
        let Some(rgba) = parse_hex_rgba8(hex) else {
            return Ok(UiEventOutcome::default());
        };
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        set_series_primary_color(&mut series, rgba);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(shape_raw) = sub.strip_prefix("shape:") {
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        let mut marker = series_marker_style(&series);
        let next_shape = if shape_raw == "next" {
            let all = all_marker_shapes();
            let pos = all.iter().position(|s| *s == marker.shape).unwrap_or(0);
            all[(pos + 1) % all.len()]
        } else {
            match shape_raw {
                "Circle" => MarkerShape::Circle,
                "Square" => MarkerShape::Square,
                "TriangleUp" => MarkerShape::TriangleUp,
                "TriangleDown" => MarkerShape::TriangleDown,
                "Diamond" => MarkerShape::Diamond,
                "Cross" => MarkerShape::Cross,
                "Plus" => MarkerShape::Plus,
                "Star" => MarkerShape::Star,
                _ => return Ok(UiEventOutcome::default()),
            }
        };
        marker.shape = next_shape;
        set_series_marker_style(&mut series, marker);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(size_dir) = sub.strip_prefix("size:") {
        let Some(mut series) = gs.spec.series.get(sel_idx).cloned() else {
            return Ok(UiEventOutcome::default());
        };
        let delta = match size_dir {
            "dec" => -1.0,
            "inc" => 1.0,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut marker = series_marker_style(&series);
        marker.size_pt = (marker.size_pt + delta).clamp(1.5, 16.0);
        set_series_marker_style(&mut series, marker);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "gdo": series.gdo }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(topo) = sub.strip_prefix("topology:") {
        let next_series = retopologize_series(&gs.spec.series, topo);
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series": next_series })).map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if sub == "err_toggle" {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let next_eb = if series.y_error_bars.is_some() { Value::Null } else { json!(app.canvas_dialogs.format_graph.error_bar_spec) };
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": next_eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(metric_raw) = sub.strip_prefix("err_metric:") {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let metric = match metric_raw {
            "Sd" => ErrorBarMetric::Sd,
            "Sem" => ErrorBarMetric::Sem,
            "ConfidenceInterval95" => ErrorBarMetric::ConfidenceInterval95,
            "Range" => ErrorBarMetric::Range,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut eb = series.y_error_bars.unwrap_or(app.canvas_dialogs.format_graph.error_bar_spec);
        eb.metric = metric;
        app.canvas_dialogs.format_graph.error_bar_spec = eb;
        app.canvas_dialogs.format_graph.error_bar_enabled = true;
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(dir_raw) = sub.strip_prefix("err_dir:") {
        let Some(series) = gs.spec.series.get(sel_idx) else {
            return Ok(UiEventOutcome::default());
        };
        let dir = match dir_raw {
            "Both" => ErrorBarDirection::Both,
            "UpperOnly" => ErrorBarDirection::UpperOnly,
            "LowerOnly" => ErrorBarDirection::LowerOnly,
            _ => return Ok(UiEventOutcome::default()),
        };
        let mut eb = series.y_error_bars.unwrap_or(app.canvas_dialogs.format_graph.error_bar_spec);
        eb.direction = dir;
        app.canvas_dialogs.format_graph.error_bar_spec = eb;
        app.canvas_dialogs.format_graph.error_bar_enabled = true;
        app.execute_command("graph.setSeries", json!({ "sheet_id": gs.id.as_str(), "series_index": sel_idx, "y_error_bars": eb }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSeries")));
    }

    if let Some(arr_raw) = sub.strip_prefix("arrangement:") {
        let arr = match arr_raw {
            "Interleaved" => GroupedArrangement::Interleaved,
            "Separated" => GroupedArrangement::Separated,
            "Stacked" => GroupedArrangement::Stacked,
            "Superimposed" => GroupedArrangement::Superimposed,
            _ => return Ok(UiEventOutcome::default()),
        };
        app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "grouped_arrangement": arr } }))
            .map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setSpec")));
    }

    if sub == "apply_style_from" {
        let doc = app.document().clone();
        app.canvas_dialogs.open_apply_style_from(&doc, &gs.id);
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("ui.applyStyleFrom.open")));
    }

    if let Some(raw_idx) = sub.strip_prefix("level:select:")
        && let Ok(idx) = raw_idx.parse::<usize>()
    {
        if let Some(target) = resolve_mv_level_target(app.document(), &gs)
            && let Some(lvl) = target.levels.get(idx)
        {
            state.selected_fg_level_name = Some(lvl.clone());
        }
        return Ok(outcome_ok(sub, None));
    }

    if let Some(move_raw) = sub.strip_prefix("level:move:") {
        let op = match move_raw {
            "top" => LevelMoveOp::Top,
            "up" => LevelMoveOp::Up,
            "down" => LevelMoveOp::Down,
            "end" => LevelMoveOp::End,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(mut target) = resolve_mv_level_target(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let mut enc = current_mv_encoding(app.document(), &gs);
        let len = target.levels.len();
        if apply_explicit_level_move(&mut target.levels, &mut state.selected_fg_level_name, op) {
            let new_idx = resolve_selected_level_idx(&target.levels, state.selected_fg_level_name.as_deref());
            let explicit_spec = LevelOrderSpec { order: LevelOrderKind::Explicit(target.levels), reversed: false };
            if target.is_x {
                enc.x_level_order_override = Some(explicit_spec);
            } else if let ColorChannelMapping::Categorical { level_order_override, .. } = &mut enc.color {
                *level_order_override = Some(explicit_spec);
            }
            app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "multivariate": enc })).map_err(|e| e.to_string())?;
            let doc_snap = app.document().clone();
            if let Some(live_gs) = doc_snap.graph_sheet(&gs.id) {
                app.canvas_dialogs.format_graph.sync_from_graph_with_doc(live_gs, Some(&doc_snap));
            }
            if len > 1 {
                match (app.focus_widget_id.as_deref(), new_idx) {
                    (Some("inspector.fg.level_top"), Some(0)) => app.focus_widget_id = Some(String::from("inspector.fg.level_end")),
                    (Some("inspector.fg.level_up"), Some(0)) => app.focus_widget_id = Some(String::from("inspector.fg.level_dn")),
                    (Some("inspector.fg.level_dn"), Some(i)) if i + 1 >= len => app.focus_widget_id = Some(String::from("inspector.fg.level_up")),
                    (Some("inspector.fg.level_end"), Some(i)) if i + 1 >= len => app.focus_widget_id = Some(String::from("inspector.fg.level_top")),
                    _ => {}
                }
            }
            return Ok(outcome_ok(sub, Some("graph.setSpec")));
        }
        return Ok(outcome_ok(sub, None));
    }

    Ok(UiEventOutcome::default())
}

fn dispatch_regression_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let sel_idx = resolve_selected_series_index(app, &gs);
    let cur_ov = gs
        .spec
        .series
        .get(sel_idx)
        .and_then(|s| s.regression_overlay.as_ref())
        .or_else(|| gs.spec.series.iter().find_map(|s| s.regression_overlay.as_ref()));
    let candidates: Vec<String> =
        app.document().result_sheets.iter().filter(|rs| !rs.fitted_curves.is_empty()).map(|rs| rs.id.as_str().to_string()).collect();

    let mut res_id = cur_ov.map(|o| o.result_sheet_id.as_str().to_string()).or_else(|| state.regression_result_sheet_id.clone());
    let mut all_series = if cur_ov.is_some() && !gs.spec.series.is_empty() {
        gs.spec.series.iter().all(|s| s.regression_overlay.is_some()) || state.regression_all_series
    } else {
        state.regression_all_series
    };
    let mut conf_band = cur_ov.map_or(state.regression_confidence_band, |o| o.show_confidence_band);
    let mut shaded_ribbon = cur_ov.map_or(state.regression_shaded_ribbon, |o| o.shaded_ribbon);

    let apply_now = match sub {
        "cycle_result" => {
            let mut ring: Vec<Option<String>> = candidates.iter().cloned().map(Some).collect();
            ring.push(None);
            let cur_pos = ring.iter().position(|entry| entry.as_deref() == res_id.as_deref()).unwrap_or(ring.len().saturating_sub(1));
            res_id = ring[(cur_pos + 1) % ring.len()].clone();
            state.regression_result_sheet_id = res_id.clone();
            true
        }
        "all_series" => {
            all_series = !all_series;
            state.regression_all_series = all_series;
            res_id.is_some()
        }
        "confidence_band" => {
            conf_band = !conf_band;
            state.regression_confidence_band = conf_band;
            res_id.is_some()
        }
        "shaded_ribbon" => {
            shaded_ribbon = !shaded_ribbon;
            state.regression_shaded_ribbon = shaded_ribbon;
            res_id.is_some()
        }
        "apply" => {
            if res_id.is_none() {
                res_id = candidates.first().cloned();
                state.regression_result_sheet_id = res_id.clone();
            }
            true
        }
        _ => return Ok(UiEventOutcome::default()),
    };

    if !apply_now || gs.spec.series.is_empty() {
        return Ok(outcome_ok(sub, None));
    }
    let payload = if let Some(rid) = res_id {
        json!({
            "sheet_id": gs.id.as_str(),
            "result_sheet_id": rid,
            "series_index": sel_idx,
            "all_series": all_series,
            "show_confidence_band": conf_band,
            "shaded_ribbon": shaded_ribbon,
        })
    } else {
        json!({
            "sheet_id": gs.id.as_str(),
            "clear": true,
            "series_index": sel_idx,
            "all_series": all_series,
        })
    };
    app.execute_command("graph.setRegressionOverlay", payload).map_err(|e| e.to_string())?;
    Ok(outcome_ok(sub, Some("graph.setRegressionOverlay")))
}

fn current_axis_which(app: &UiApp) -> AxisWhich {
    use crate::dialogs_canvas::AxisTarget;
    match app.canvas_dialogs.format_axes.target_axis {
        AxisTarget::X => AxisWhich::X,
        AxisTarget::Y => AxisWhich::Y,
        AxisTarget::Y2 => AxisWhich::Y2,
    }
}

fn axis_which_key(which: AxisWhich) -> &'static str {
    match which {
        AxisWhich::X => "x_axis",
        AxisWhich::Y => "y_axis",
        AxisWhich::Y2 => "y2_axis",
    }
}

fn read_live_axis_spec(gs: &GraphSheet, which: AxisWhich) -> AxisSpec {
    match which {
        AxisWhich::X => gs.spec.x_axis.clone(),
        AxisWhich::Y => gs.spec.y_axis.clone(),
        AxisWhich::Y2 => gs.spec.y2_axis.clone().unwrap_or_else(|| gs.spec.y_axis.clone()),
    }
}

fn axis_primary_seg(axis: &AxisSpec) -> AxisSegment {
    axis.segments.first().cloned().unwrap_or_default()
}

fn clear_axis_input_buffers(app: &mut UiApp) {
    app.interaction.text_field_buffers.remove(AXIS_MIN_INPUT_ID);
    app.interaction.text_field_buffers.remove(AXIS_MAX_INPUT_ID);
    app.interaction.text_field_buffers.remove(AXIS_INTERVAL_INPUT_ID);
}

fn build_segments_for_count(axis: &AxisSpec, count: usize) -> Vec<AxisSegment> {
    let base = axis_primary_seg(axis);
    let lo = base.min.unwrap_or(0.0);
    let hi = base.max.unwrap_or(100.0);
    let (a, b) = if hi > lo { (lo, hi) } else { (0.0, 100.0) };
    let span = b - a;
    if count <= 1 {
        return vec![AxisSegment {
            min: base.min,
            max: base.max,
            scale: base.scale,
            major_tick_interval: base.major_tick_interval,
            minor_ticks_per_interval: base.minor_ticks_per_interval,
            length_share: 1.0,
        }];
    }
    if count == 2 {
        vec![
            AxisSegment {
                min: Some(a),
                max: Some(a + span * 0.35),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.48,
            },
            AxisSegment {
                min: Some(a + span * 0.65),
                max: Some(b),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.48,
            },
        ]
    } else {
        vec![
            AxisSegment {
                min: Some(a),
                max: Some(a + span * 0.25),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
            AxisSegment {
                min: Some(a + span * 0.40),
                max: Some(a + span * 0.62),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
            AxisSegment {
                min: Some(a + span * 0.78),
                max: Some(b),
                scale: base.scale,
                major_tick_interval: None,
                minor_ticks_per_interval: base.minor_ticks_per_interval,
                length_share: 0.32,
            },
        ]
    }
}

fn commit_axis_update(app: &mut UiApp, gs: &GraphSheet, which: AxisWhich, next_axis: AxisSpec) -> Result<(), String> {
    let mut payload = json!({ "sheet_id": gs.id.as_str() });
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(String::from(axis_which_key(which)), json!(next_axis));
    }
    app.execute_command("graph.setAxes", payload).map_err(|e| e.to_string())?;
    clear_axis_input_buffers(app);
    Ok(())
}

fn dispatch_axes_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    use crate::dialogs_canvas::AxisTarget;

    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());

    if let Some(target) = sub.strip_prefix("target:") {
        let next = match target {
            "x" => AxisTarget::X,
            "y" => AxisTarget::Y,
            "y2" => AxisTarget::Y2,
            _ => return Ok(UiEventOutcome::default()),
        };
        app.canvas_dialogs.format_axes.target_axis = next;
        app.canvas_dialogs.format_axes.sync_from_graph(&gs, next);
        clear_axis_input_buffers(app);
        app.interaction.text_field_buffers.remove(AXIS_TITLE_INPUT_ID);
        state.open_popup = None;
        return Ok(outcome_ok(sub, None));
    }

    let which = current_axis_which(app);
    let mut axis = read_live_axis_spec(&gs, which);
    let mut seg = axis_primary_seg(&axis);

    if sub == "apply_title" {
        if let Some(buf) = app.interaction.text_field_buffers.get(AXIS_TITLE_INPUT_ID) {
            axis.title = buf.text.trim().to_owned();
        }
        commit_axis_update(app, &gs, which, axis)?;
        app.interaction.text_field_buffers.remove(AXIS_TITLE_INPUT_ID);
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "y2_toggle" {
        let y2_val = if gs.spec.y2_axis.is_some() { Value::Null } else { json!(gs.spec.y_axis) };
        app.execute_command("graph.setAxes", json!({ "sheet_id": gs.id.as_str(), "y2_axis": y2_val })).map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(scale_raw) = sub.strip_prefix("scale:") {
        let scale = match scale_raw {
            "Linear" => AxisScale::Linear,
            "Log10" => AxisScale::Log10,
            "Log2" => AxisScale::Log2,
            "Ln" => AxisScale::Ln,
            "Logit" => AxisScale::Logit(LogitDomain::Percent),
            "Probit" => AxisScale::Probit,
            "Reciprocal" => AxisScale::Reciprocal,
            "Nonlinear" => match seg.scale {
                AxisScale::Ln => AxisScale::Logit(LogitDomain::Percent),
                AxisScale::Logit(_) => AxisScale::Probit,
                AxisScale::Probit => AxisScale::Reciprocal,
                _ => AxisScale::Ln,
            },
            _ => return Ok(UiEventOutcome::default()),
        };
        if axis.segments.is_empty() {
            axis.segments.push(seg.clone());
        }
        for s in &mut axis.segments {
            s.scale = scale;
        }
        commit_axis_update(app, &gs, which, axis)?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(cnt_raw) = sub.strip_prefix("breaks:") {
        let count = cnt_raw.parse::<usize>().unwrap_or(1).clamp(1, 3);
        axis.segments = build_segments_for_count(&axis, count);
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if let Some(orn_raw) = sub.strip_prefix("ornament:") {
        axis.break_ornament = match orn_raw {
            "DoubleSlash" => BreakOrnament::DoubleSlash,
            "Zigzag" => BreakOrnament::Zigzag,
            "BlankGap" => BreakOrnament::BlankGap,
            _ => return Ok(UiEventOutcome::default()),
        };
        if axis.segments.len() < 2 {
            axis.segments = build_segments_for_count(&axis, 2);
        }
        commit_axis_update(app, &gs, which, axis)?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "auto_range" {
        seg.min = None;
        seg.max = None;
        seg.major_tick_interval = None;
        axis.segments = vec![seg];
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    if sub == "apply_range" {
        let parse_buf = |wid_str: &str, fallback: Option<f64>| -> Option<f64> {
            if let Some(buf) = app.interaction.text_field_buffers.get(wid_str) {
                let trimmed = buf.text.trim();
                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    crate::grid_view::parse_localized_number_input(trimmed, &app.locale_preset)
                        .or_else(|| trimmed.parse::<f64>().ok())
                        .filter(|v| v.is_finite())
                        .or(fallback)
                }
            } else {
                fallback
            }
        };
        let next_min = parse_buf(AXIS_MIN_INPUT_ID, seg.min);
        let next_max = parse_buf(AXIS_MAX_INPUT_ID, seg.max);
        let next_iv = parse_buf(AXIS_INTERVAL_INPUT_ID, seg.major_tick_interval).filter(|v| *v > 0.0);
        if let (Some(lo), Some(hi)) = (next_min, next_max)
            && hi <= lo
        {
            return Ok(UiEventOutcome::default());
        }
        seg.min = next_min;
        seg.max = next_max;
        seg.major_tick_interval = next_iv;
        if axis.segments.is_empty() {
            axis.segments.push(seg);
        } else {
            axis.segments[0] = seg;
        }
        commit_axis_update(app, &gs, which, axis)?;
        return Ok(outcome_ok(sub, Some("graph.setAxes")));
    }

    let step = {
        let lo = seg.min.unwrap_or(0.0);
        let hi = seg.max.unwrap_or(100.0);
        let span = (hi - lo).abs();
        if span > 0.0 { (span / 5.0).max(1.0) } else { 5.0 }
    };

    match sub {
        "min:dec" => {
            seg.min = Some(seg.min.unwrap_or(0.0) - step);
        }
        "min:inc" => {
            let next = seg.min.unwrap_or(0.0) + step;
            if seg.max.is_none_or(|m| next < m) {
                seg.min = Some(next);
            }
        }
        "max:dec" => {
            let next = seg.max.unwrap_or(100.0) - step;
            if seg.min.is_none_or(|m| next > m) {
                seg.max = Some(next);
            }
        }
        "max:inc" => {
            seg.max = Some(seg.max.unwrap_or(100.0) + step);
        }
        "interval:dec" => {
            let cur = seg.major_tick_interval.unwrap_or(step);
            seg.major_tick_interval = Some((cur * 0.5).max(0.5));
        }
        "interval:inc" => {
            let cur = seg.major_tick_interval.unwrap_or(step);
            seg.major_tick_interval = Some((cur * 2.0).max(1.0));
        }
        _ => return Ok(UiEventOutcome::default()),
    }
    if axis.segments.is_empty() {
        axis.segments.push(seg);
    } else {
        axis.segments[0] = seg;
    }
    commit_axis_update(app, &gs, which, axis)?;
    Ok(outcome_ok(sub, Some("graph.setAxes")))
}

fn source_columns_for_graph(doc: &ProjectDocument, gs: &GraphSheet) -> Vec<(usize, String)> {
    let Some(ds) = doc.data_sheet(&gs.spec.primary_source_sheet_id) else {
        return Vec::new();
    };
    let count = ds.archetype.column_count();
    (0..count)
        .map(|idx| {
            let title = data_column_title(ds, idx).map(str::to_owned).unwrap_or_else(|| format!("Col {}", idx + 1));
            (idx, title)
        })
        .collect()
}

fn current_mv_encoding(doc: &ProjectDocument, gs: &GraphSheet) -> MultivariateGraphEncoding {
    if let Some(enc) = &gs.spec.multivariate {
        return enc.clone();
    }
    let cols = source_columns_for_graph(doc, gs);
    let x_column = 0usize;
    let y_column = cols.get(1).or_else(|| cols.first()).map_or(0, |(c, _)| *c);
    MultivariateGraphEncoding {
        plot_mode: MultivariatePlotMode::BubbleScatter,
        x_column,
        y_column,
        color: ColorChannelMapping::Constant(gs.spec.palette.color_at(0)),
        size: SizeChannelMapping::Constant { radius_pt: 4.5 },
        label_column: None,
        x_level_order_override: None,
        show_convex_hulls: false,
        show_confidence_ellipses: false,
    }
}

fn mv_summary_measure(enc: &MultivariateGraphEncoding) -> Option<SummaryMeasure> {
    match &enc.plot_mode {
        MultivariatePlotMode::BubbleScatter => None,
        MultivariatePlotMode::CategoricalSummary { gdo, .. } => match gdo {
            GraphicalDataObject::BarWithReplicates { summary, .. }
            | GraphicalDataObject::AlignedDotPlot { summary, .. }
            | GraphicalDataObject::FloatingBar { summary: Some(summary), .. } => Some(*summary),
            _ => Some(SummaryMeasure::Mean),
        },
    }
}

fn default_mv_categorical_summary(gs: &GraphSheet, summary: SummaryMeasure) -> MultivariatePlotMode {
    MultivariatePlotMode::CategoricalSummary {
        gdo: GraphicalDataObject::BarWithReplicates {
            summary,
            fill_color: gs.spec.palette.color_at(0),
            border_color: Rgba8::BLACK,
            border_width_pt: 0.75,
            replicate_overlay: None,
        },
        y_error_bars: Some(ErrorBarSpec::default()),
    }
}

fn dispatch_multivariable_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut enc = current_mv_encoding(app.document(), &gs);

    if let Some(mode) = sub.strip_prefix("mode:") {
        match mode {
            "bubble" => enc.plot_mode = MultivariatePlotMode::BubbleScatter,
            "cat_summary" => {
                if matches!(enc.plot_mode, MultivariatePlotMode::BubbleScatter) {
                    enc.plot_mode = default_mv_categorical_summary(&gs, SummaryMeasure::Mean);
                    if let Some(TableArchetype::MultipleVariables(mv)) =
                        app.document().data_sheet(&gs.spec.primary_source_sheet_id).map(|ds| &ds.archetype)
                    {
                        let x_is_cat = mv.variables.get(enc.x_column).is_some_and(|v| v.role == VariableRole::Categorical);
                        if !x_is_cat
                            && let (Some(cat_col), Some(cont_col)) = (
                                mv.variables.iter().position(|v| v.role == VariableRole::Categorical),
                                mv.variables
                                    .iter()
                                    .position(|v| v.role == VariableRole::Continuous)
                                    .or_else(|| mv.variables.iter().position(|v| v.role == VariableRole::Calculated)),
                            )
                        {
                            enc.x_column = cat_col;
                            enc.y_column = cont_col;
                        }
                    }
                }
            }
            _ => return Ok(UiEventOutcome::default()),
        }
    } else if let Some(m_raw) = sub.strip_prefix("metric:") {
        let next_m = match m_raw {
            "Mean" => SummaryMeasure::Mean,
            "Median" => SummaryMeasure::Median,
            "Count" => SummaryMeasure::GeometricMean,
            _ => return Ok(UiEventOutcome::default()),
        };
        match &mut enc.plot_mode {
            MultivariatePlotMode::BubbleScatter => enc.plot_mode = default_mv_categorical_summary(&gs, next_m),
            MultivariatePlotMode::CategoricalSummary { gdo, .. } => match gdo {
                GraphicalDataObject::BarWithReplicates { summary, .. } | GraphicalDataObject::AlignedDotPlot { summary, .. } => *summary = next_m,
                GraphicalDataObject::FloatingBar { summary, .. } => *summary = Some(next_m),
                other => {
                    *other = GraphicalDataObject::BarWithReplicates {
                        summary: next_m,
                        fill_color: gs.spec.palette.color_at(0),
                        border_color: Rgba8::BLACK,
                        border_width_pt: 0.75,
                        replicate_overlay: None,
                    };
                }
            },
        }
    } else if sub == "clamp_apply" {
        let parse_clamp = |wid_str: &str| -> Option<f64> {
            let raw = app.interaction.text_field_buffers.get(wid_str)?.text.trim().to_owned();
            if raw.is_empty() {
                return None;
            }
            crate::grid_view::parse_localized_number_input(&raw, &app.locale_preset).or_else(|| raw.parse::<f64>().ok()).filter(|v| v.is_finite())
        };
        let lo = parse_clamp(MV_CLAMP_MIN_INPUT_ID);
        let hi = parse_clamp(MV_CLAMP_MAX_INPUT_ID);
        let next_clamp = match (lo, hi) {
            (Some(a), Some(b)) if b > a => Some((a, b)),
            (None, None) => None,
            _ => return Ok(UiEventOutcome::default()),
        };
        match &mut enc.color {
            ColorChannelMapping::Continuous { domain_clamp, .. } => *domain_clamp = next_clamp,
            _ => {
                let cur_col = match &enc.color {
                    ColorChannelMapping::Categorical { column, .. } => *column,
                    ColorChannelMapping::Constant(_) | ColorChannelMapping::Continuous { .. } => {
                        source_columns_for_graph(app.document(), &gs).last().map_or(0, |(c, _)| *c)
                    }
                };
                enc.color = ColorChannelMapping::Continuous {
                    column: cur_col,
                    ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
                    domain_clamp: next_clamp,
                };
            }
        }
        app.interaction.text_field_buffers.remove(MV_CLAMP_MIN_INPUT_ID);
        app.interaction.text_field_buffers.remove(MV_CLAMP_MAX_INPUT_ID);
    } else if let Some(raw) = sub.strip_prefix("x:") {
        if let Ok(col) = raw.parse::<usize>() {
            enc.x_column = col;
        }
    } else if let Some(raw) = sub.strip_prefix("y:") {
        if let Ok(col) = raw.parse::<usize>() {
            enc.y_column = col;
        }
    } else if let Some(raw) = sub.strip_prefix("color:") {
        if raw == "none" {
            enc.color = ColorChannelMapping::Constant(gs.spec.palette.color_at(0));
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.color = ColorChannelMapping::Categorical { column: col, palette: gs.spec.palette, level_order_override: None };
        }
    } else if let Some(mode) = sub.strip_prefix("color_mode:") {
        let cur_col = match &enc.color {
            ColorChannelMapping::Categorical { column, .. } | ColorChannelMapping::Continuous { column, .. } => *column,
            ColorChannelMapping::Constant(_) => source_columns_for_graph(app.document(), &gs).last().map_or(0, |(c, _)| *c),
        };
        enc.color = match mode {
            "cat" => ColorChannelMapping::Categorical { column: cur_col, palette: gs.spec.palette, level_order_override: None },
            "cont" => ColorChannelMapping::Continuous {
                column: cur_col,
                ramp: ColorRamp::Named { id: ColormapId::Viridis, reversed: false },
                domain_clamp: None,
            },
            _ => return Ok(UiEventOutcome::default()),
        };
    } else if let Some(raw) = sub.strip_prefix("size:") {
        if raw == "none" {
            enc.size = SizeChannelMapping::Constant { radius_pt: 4.5 };
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.size = SizeChannelMapping::Variable { column: col, min_radius_pt: 2.5, max_radius_pt: 9.5 };
        }
    } else if let Some(raw) = sub.strip_prefix("group:") {
        if raw == "none" {
            enc.label_column = None;
        } else if let Ok(col) = raw.parse::<usize>() {
            enc.label_column = Some(col);
        }
    } else if let Some(rest) = sub.strip_prefix("role:")
        && let Some((raw_col, role)) = rest.split_once(':')
        && let Ok(col) = raw_col.parse::<usize>()
    {
        let src_id = gs.spec.primary_source_sheet_id.clone();
        app.execute_command("table.setVariableRole", json!({ "sheet_id": src_id.as_str(), "column": col, "role": role }))
            .map_err(|e| e.to_string())?;
        if let Some(ds) = app.document().data_sheet(&src_id).cloned()
            && app.grid.variables_dialog.open
        {
            app.grid.variables_dialog.sync_from_sheet(&ds);
        }
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("table.setVariableRole")));
    } else if sub == "ellipse:toggle" {
        enc.show_confidence_ellipses = !enc.show_confidence_ellipses;
    } else if sub == "hull:toggle" {
        enc.show_convex_hulls = !enc.show_convex_hulls;
    } else {
        return Ok(UiEventOutcome::default());
    }

    app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "multivariate": enc })).map_err(|e| e.to_string())?;
    state.open_popup = None;
    Ok(outcome_ok(sub, Some("graph.setSpec")))
}

fn find_pairwise_result_sheet_id(doc: &ProjectDocument, gs: &GraphSheet) -> Option<SheetId> {
    if let Some(cld) = &gs.spec.compact_letter_display
        && doc.result_sheet(&cld.result_sheet_id).is_some()
    {
        return Some(cld.result_sheet_id.clone());
    }
    if let Some(br) = gs.spec.comparisons.first()
        && doc.result_sheet(&br.result_sheet_id).is_some()
    {
        return Some(br.result_sheet_id.clone());
    }
    doc.result_sheets
        .iter()
        .rev()
        .find(|rs| {
            matches!(rs.analysis.kind, AnalysisKind::OneWayAnova(_) | AnalysisKind::FactorialAnova(_) | AnalysisKind::NestedComparison(_))
                || !rs.pairwise_comparisons.is_empty()
        })
        .or_else(|| doc.result_sheets.last())
        .map(|rs| rs.id.clone())
}

fn cld_or_default(gs: &GraphSheet, rid: &SheetId) -> CompactLetterDisplaySpec {
    gs.spec.compact_letter_display.clone().unwrap_or_else(|| CompactLetterDisplaySpec::new(rid.clone()))
}

fn is_cld_active(gs: &GraphSheet) -> bool {
    gs.spec.compact_letter_display.is_some()
        && matches!(
            gs.spec.pairwise_display_mode,
            PairwiseDisplayMode::CompactLetterDisplayOnly | PairwiseDisplayMode::BracketsAndCompactLetterDisplay
        )
}

fn dispatch_pairwise_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut pw = app.canvas_dialogs.pairwise.clone();
    pw.sync_from_graph(&gs, app.document());

    match sub {
        "auto_brackets" => {
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            app.execute_command(
                "graph.addComparisonBrackets",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "result_sheet_id": rid.as_str(),
                    "clear_existing": true,
                    "style": pw.bracket_style,
                    "label_mode": pw.label_mode,
                    "hide_if_non_significant": pw.hide_non_significant,
                    "significant_only": pw.hide_non_significant,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
        }
        "clear_brackets" => {
            app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": { "comparisons": [] } })).map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setSpec")));
        }
        "sig_only:toggle" => {
            pw.hide_non_significant = !pw.hide_non_significant;
            app.canvas_dialogs.pairwise = pw.clone();
            if let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs)
                && !gs.spec.comparisons.is_empty()
            {
                app.execute_command(
                    "graph.addComparisonBrackets",
                    json!({
                        "sheet_id": gs.id.as_str(),
                        "result_sheet_id": rid.as_str(),
                        "clear_existing": true,
                        "style": pw.bracket_style,
                        "label_mode": pw.label_mode,
                        "hide_if_non_significant": pw.hide_non_significant,
                        "significant_only": pw.hide_non_significant,
                    }),
                )
                .map_err(|e| e.to_string())?;
            }
            return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
        }
        "cld_toggle" => {
            let next_enabled = !is_cld_active(&gs);
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            let cld = cld_or_default(&gs, &rid);
            let next_mode = if !next_enabled {
                PairwiseDisplayMode::BracketsOnly
            } else if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
                PairwiseDisplayMode::CompactLetterDisplayOnly
            } else {
                gs.spec.pairwise_display_mode
            };
            app.execute_command(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "enabled": next_enabled,
                    "result_sheet_id": rid.as_str(),
                    "pairwise_display_mode": next_mode,
                    "ordering": cld.ordering,
                    "uppercase": cld.uppercase,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
        }
        "cld_case:toggle" => {
            let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
                return Ok(UiEventOutcome::default());
            };
            let cld = cld_or_default(&gs, &rid);
            let next_mode = if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
                PairwiseDisplayMode::CompactLetterDisplayOnly
            } else {
                gs.spec.pairwise_display_mode
            };
            app.execute_command(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "enabled": true,
                    "result_sheet_id": rid.as_str(),
                    "pairwise_display_mode": next_mode,
                    "ordering": cld.ordering,
                    "uppercase": !cld.uppercase,
                }),
            )
            .map_err(|e| e.to_string())?;
            return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
        }
        _ => {}
    }

    if let Some(lbl_raw) = sub.strip_prefix("label_mode:") {
        let mode = match lbl_raw {
            "stars" => BracketLabelMode::SummaryStars,
            "numeric" => BracketLabelMode::NumericPOrQ,
            _ => return Ok(UiEventOutcome::default()),
        };
        pw.label_mode = mode;
        app.canvas_dialogs.pairwise = pw.clone();
        if let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) {
            app.execute_command(
                "graph.addComparisonBrackets",
                json!({
                    "sheet_id": gs.id.as_str(),
                    "result_sheet_id": rid.as_str(),
                    "clear_existing": true,
                    "style": pw.bracket_style,
                    "label_mode": lbl_raw,
                    "hide_if_non_significant": pw.hide_non_significant,
                    "significant_only": pw.hide_non_significant,
                }),
            )
            .map_err(|e| e.to_string())?;
        }
        return Ok(outcome_ok(sub, Some("graph.addComparisonBrackets")));
    }

    if let Some(mode_raw) = sub.strip_prefix("cld_mode:") {
        let mode = match mode_raw {
            "BracketsOnly" => PairwiseDisplayMode::BracketsOnly,
            "CompactLetterDisplayOnly" => PairwiseDisplayMode::CompactLetterDisplayOnly,
            "BracketsAndCompactLetterDisplay" => PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let cld = cld_or_default(&gs, &rid);
        let enabled = !matches!(mode, PairwiseDisplayMode::BracketsOnly);
        app.execute_command(
            "graph.setCompactLetterDisplay",
            json!({
                "sheet_id": gs.id.as_str(),
                "enabled": enabled,
                "result_sheet_id": rid.as_str(),
                "pairwise_display_mode": mode,
                "ordering": cld.ordering,
                "uppercase": cld.uppercase,
            }),
        )
        .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
    }

    if let Some(ord_raw) = sub.strip_prefix("cld_order:") {
        let ordering = match ord_raw {
            "AscendingMean" => CldOrdering::AscendingMean,
            "DescendingMean" => CldOrdering::DescendingMean,
            _ => return Ok(UiEventOutcome::default()),
        };
        let Some(rid) = find_pairwise_result_sheet_id(app.document(), &gs) else {
            return Ok(UiEventOutcome::default());
        };
        let cld = cld_or_default(&gs, &rid);
        let next_mode = if gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly {
            PairwiseDisplayMode::CompactLetterDisplayOnly
        } else {
            gs.spec.pairwise_display_mode
        };
        app.execute_command(
            "graph.setCompactLetterDisplay",
            json!({
                "sheet_id": gs.id.as_str(),
                "enabled": true,
                "result_sheet_id": rid.as_str(),
                "pairwise_display_mode": next_mode,
                "ordering": ordering,
                "uppercase": cld.uppercase,
            }),
        )
        .map_err(|e| e.to_string())?;
        state.open_popup = None;
        return Ok(outcome_ok(sub, Some("graph.setCompactLetterDisplay")));
    }

    Ok(UiEventOutcome::default())
}

fn mm_to_pt(mm: u16) -> f64 {
    f64::from(mm) * 72.0 / 25.4
}

fn layout_margin_mm(state: &InspectorViewState) -> u16 {
    if state.page_margin_mm == 0 { 12 } else { state.page_margin_mm.clamp(4, 40) }
}

fn layout_scale_bar_mm(state: &InspectorViewState) -> u16 {
    if state.scale_bar_mm == 0 { 10 } else { state.scale_bar_mm }
}

fn has_scale_bar_item(spec: &LayoutSpec) -> bool {
    spec.items.iter().any(|item| matches!(item, LayoutItem::TextCallout { id, .. } if id == SCALE_BAR_ITEM_ID))
}

fn layout_size_pt(spec: &LayoutSpec) -> (f64, f64) {
    let scale = match spec.unit {
        PhysicalUnit::Millimeters => 72.0 / 25.4,
        PhysicalUnit::Inches => 72.0,
        PhysicalUnit::Centimeters => 72.0 / 2.54,
        PhysicalUnit::Points => 1.0,
    };
    (spec.width * scale, spec.height * scale)
}

fn rearrange_layout_grid(spec: &mut LayoutSpec, rows: u8, cols: u8, margin_pt: f64, gutter_pt: f64) {
    let rows = rows.max(1);
    let cols = cols.max(1);
    spec.grid_template = Some((rows, cols));
    let (page_w_pt, page_h_pt) = layout_size_pt(spec);
    let usable_w = (page_w_pt - 2.0 * margin_pt - f64::from(cols.saturating_sub(1)) * gutter_pt).max(36.0);
    let usable_h = (page_h_pt - 2.0 * margin_pt - f64::from(rows.saturating_sub(1)) * gutter_pt).max(36.0);
    let cell_w = usable_w / f64::from(cols);
    let cell_h = usable_h / f64::from(rows);

    let mut panel_idx = 0usize;
    for item in &mut spec.items {
        if let LayoutItem::LiveGraph { bounds_pt, .. } = item {
            let r = (panel_idx / usize::from(cols)) % usize::from(rows);
            let c = panel_idx % usize::from(cols);
            *bounds_pt =
                RectPt::new(margin_pt + (c as f64) * (cell_w + gutter_pt), margin_pt + (r as f64) * (cell_h + gutter_pt), cell_w, cell_h).into();
            panel_idx += 1;
        }
    }
}

fn upsert_scale_bar_item(spec: &mut LayoutSpec, margin_pt: f64, length_mm: u16, enabled: bool) {
    spec.items.retain(|it| !matches!(it, LayoutItem::TextCallout { id, .. } if id == SCALE_BAR_ITEM_ID));
    if !enabled {
        return;
    }
    let (page_w_pt, page_h_pt) = layout_size_pt(spec);
    let bar_w_pt = mm_to_pt(length_mm).clamp(24.0, (page_w_pt * 0.4).max(36.0));
    let bar_h_pt = 18.0;
    let x = (page_w_pt - margin_pt - bar_w_pt).max(margin_pt);
    let y = (page_h_pt - margin_pt - bar_h_pt).max(margin_pt);
    spec.items.push(LayoutItem::TextCallout {
        id: String::from(SCALE_BAR_ITEM_ID),
        bounds_pt: RectPt::new(x, y, bar_w_pt, bar_h_pt).into(),
        markdown: format!("|==== {length_mm} mm ====|"),
        font_size_pt: 8.5,
    });
}

fn commit_layout_spec(app: &mut UiApp, ls: &LayoutSheet, spec: &LayoutSpec) -> Result<(), String> {
    app.execute_command(
        "layout.setSpec",
        json!({
            "sheet_id": ls.id.as_str(),
            "align_plot_origins": spec.align_plot_origins,
            "spec": spec,
        }),
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn dispatch_layout_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(ls) = resolve_layout_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_layout_id = Some(ls.id.as_str().to_string());
    let mut spec = ls.spec.clone();
    let (cur_rows, cur_cols) = spec.grid_template.unwrap_or((1, 2));

    if let Some(grid_raw) = sub.strip_prefix("grid:") {
        let (rows, cols) = match grid_raw {
            "1x1" => (1u8, 1u8),
            "1x2" => (1, 2),
            "2x1" => (2, 1),
            "2x2" => (2, 2),
            "2x3" => (2, 3),
            _ => return Ok(UiEventOutcome::default()),
        };
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        rearrange_layout_grid(&mut spec, rows, cols, margin_pt, 12.0);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if let Some(m_raw) = sub.strip_prefix("margin:") {
        let cur_mm = layout_margin_mm(state);
        let next_mm = match m_raw {
            "dec" => cur_mm.saturating_sub(2).max(4),
            "inc" => cur_mm.saturating_add(2).min(40),
            val => val.parse::<u16>().unwrap_or(cur_mm).clamp(4, 40),
        };
        state.page_margin_mm = next_mm;
        let margin_pt = mm_to_pt(next_mm);
        rearrange_layout_grid(&mut spec, cur_rows, cur_cols, margin_pt, 12.0);
        if has_scale_bar_item(&spec) {
            upsert_scale_bar_item(&mut spec, margin_pt, layout_scale_bar_mm(state), true);
        }
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if sub == "align_origins:toggle" {
        let next_align = !spec.align_plot_origins;
        app.execute_command("layout.setSpec", json!({ "sheet_id": ls.id.as_str(), "align_plot_origins": next_align })).map_err(|e| e.to_string())?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if sub == "scale_bar:toggle" {
        let next_on = !has_scale_bar_item(&spec);
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        upsert_scale_bar_item(&mut spec, margin_pt, layout_scale_bar_mm(state), next_on);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    if let Some(sb_raw) = sub.strip_prefix("scale_bar_mm:") {
        let next_mm = sb_raw.parse::<u16>().unwrap_or(10).clamp(1, 200);
        state.scale_bar_mm = next_mm;
        let margin_pt = mm_to_pt(layout_margin_mm(state));
        upsert_scale_bar_item(&mut spec, margin_pt, next_mm, true);
        commit_layout_spec(app, &ls, &spec)?;
        return Ok(outcome_ok(sub, Some("layout.setSpec")));
    }

    Ok(UiEventOutcome::default())
}

fn resolve_renderable_canvas_target(doc: &ProjectDocument, state: &InspectorViewState) -> Option<(SheetId, bool)> {
    if let Some(active) = &doc.active_sheet_id {
        if doc.layout_sheet(active).is_some() {
            return Some((active.clone(), true));
        }
        if doc.graph_sheet(active).is_some() {
            return Some((active.clone(), false));
        }
    }
    if let Some(ls) = resolve_layout_sheet(doc, state) {
        return Some((ls.id.clone(), true));
    }
    resolve_graph_sheet(doc, state).map(|gs| (gs.id.clone(), false))
}

fn dispatch_canvas_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let apply_preset_for_target = |app: &mut UiApp, state: &mut InspectorViewState, preset: FigureCanvasPreset| -> Result<bool, String> {
        let doc_snap = app.document().clone();
        let Some((sheet_id, is_layout)) = resolve_renderable_canvas_target(&doc_snap, state) else {
            return Ok(false);
        };
        let intent = app.canvas_dialogs.apply_canvas_preset_intent(&sheet_id, is_layout, preset);
        app.execute_command(&intent.command_id, intent.params).map_err(|e| e.to_string())?;
        Ok(true)
    };

    if let Some(preset_raw) = sub.strip_prefix("preset:") {
        let Some(preset) = FigureCanvasPreset::from_name(preset_raw) else {
            return Ok(UiEventOutcome::default());
        };
        state.last_canvas_preset = Some(format!("{preset:?}"));
        let applied = apply_preset_for_target(app, state, preset)?;
        return Ok(outcome_ok(sub, if applied { Some("ui.canvas.applyPreset") } else { None }));
    }

    if let Some(mode_raw) = sub.strip_prefix("resize:") {
        let Some(mode) = CanvasResizeMode::from_name(mode_raw) else {
            return Ok(UiEventOutcome::default());
        };
        app.canvas_dialogs.canvas_resize_mode = mode;
        let preset = app
            .canvas_dialogs
            .active_canvas_preset
            .or_else(|| state.last_canvas_preset.as_deref().and_then(FigureCanvasPreset::from_name))
            .unwrap_or(FigureCanvasPreset::SingleColumn88mm);
        let applied = apply_preset_for_target(app, state, preset)?;
        return Ok(outcome_ok(sub, if applied { Some("ui.canvas.applyPreset") } else { None }));
    }

    Ok(UiEventOutcome::default())
}

fn active_graph_scene_and_points(app: &UiApp, state: &InspectorViewState) -> (Option<GraphSheet>, Vec<curvebook_chart::PlottedPointMeta>) {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return (None, Vec::new());
    };
    let pts = app.session().render_sheet_scene(&gs.id).map(|sc| sc.plotted_points).unwrap_or_default();
    (Some(gs), pts)
}

fn resolve_drill_target(app: &UiApp, state: &InspectorViewState, pts: &[curvebook_chart::PlottedPointMeta]) -> Option<SourceCellId> {
    if let Some(brushed) = &app.brushed_cell
        && app.document().data_sheet(&brushed.sheet_id).is_some()
    {
        return Some(brushed.clone());
    }
    if let Some(pt) = pts.get(state.selected_point_cursor).or_else(|| pts.first()) {
        return Some(pt.source_cell.clone());
    }
    resolve_graph_sheet(app.document(), state).and_then(|gs| {
        app.document().data_sheet(&gs.spec.primary_source_sheet_id).map(|ds| SourceCellId { sheet_id: ds.id.clone(), row: 0, col: 0, subcol: 0 })
    })
}

fn resolve_drill_target_fast(app: &UiApp, gs: Option<&GraphSheet>) -> Option<SourceCellId> {
    if let Some(brushed) = &app.brushed_cell
        && app.document().data_sheet(&brushed.sheet_id).is_some()
    {
        return Some(brushed.clone());
    }
    gs.and_then(|g| {
        app.document().data_sheet(&g.spec.primary_source_sheet_id).map(|ds| SourceCellId { sheet_id: ds.id.clone(), row: 0, col: 0, subcol: 0 })
    })
}

fn dispatch_drill_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let maybe_gs = resolve_graph_sheet(app.document(), state).cloned();
    if let Some(gs) = &maybe_gs {
        state.target_graph_id = Some(gs.id.as_str().to_string());
    }

    if let Some(pick) = sub.strip_prefix("point:") {
        let (_, pts) = active_graph_scene_and_points(app, state);
        if pts.is_empty() {
            return Ok(UiEventOutcome::default());
        }
        let n = pts.len();
        let cur = state.selected_point_cursor.min(n - 1);
        let next = match pick {
            "prev" => (cur + n - 1) % n,
            "next" => (cur + 1) % n,
            raw => raw.parse::<usize>().unwrap_or(cur).min(n - 1),
        };
        state.selected_point_cursor = next;
        let pt = &pts[next];
        state.selected_point_summary = format!("{} ({})", pt.series_label, pt.value_summary);
        app.brushed_cell = Some(pt.source_cell.clone());
        app.brushed_points = vec![pt.source_cell.clone()];
        if let Some(gs) = &maybe_gs
            && let Some(idx) = gs.spec.series.iter().position(|s| s.legend_label.as_deref() == Some(pt.series_label.as_str()))
        {
            app.canvas_dialogs.format_graph.selected_series_index = idx;
        }
        return Ok(outcome_ok(sub, None));
    }

    if sub == "go_to_cell" {
        let target = if let Some(b) = resolve_drill_target_fast(app, maybe_gs.as_ref())
            && app.brushed_cell.is_some()
        {
            b
        } else {
            let (_, pts) = active_graph_scene_and_points(app, state);
            let Some(t) = resolve_drill_target(app, state, &pts) else {
                return Ok(UiEventOutcome::default());
            };
            t
        };
        let coord = GridCellCoord::new(target.row, target.col, target.subcol);
        app.execute_command("sheet.select", json!({ "sheet_id": target.sheet_id.as_str() })).map_err(|e| e.to_string())?;
        app.grid.select_single_sync(coord);
        if let Some(ds) = app.session.document().data_sheet(&target.sheet_id) {
            app.grid.tool.ensure_coord_visible(ds, coord);
        }
        app.grid.refresh_selection_stats(app.session.document());
        app.brushed_cell = Some(target.clone());
        app.brushed_points = vec![target];
        app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
        app.focus_widget_id = None;
        return Ok(outcome_ok(sub, Some("sheet.select")));
    }

    if sub == "return_to_graph" {
        if let Some(gs) = &maybe_gs {
            app.execute_command("sheet.select", json!({ "sheet_id": gs.id.as_str() })).map_err(|e| e.to_string())?;
            app.nav.active_pane = ActiveWorkspacePane::MainCanvasOrGrid;
            return Ok(outcome_ok(sub, Some("sheet.select")));
        }
        return Ok(UiEventOutcome::default());
    }

    Ok(UiEventOutcome::default())
}

/// Dispatch custom `WidgetAction::Custom` events targeting the `inspector` domain.
pub fn handle_custom_action(app: &mut UiApp, state: &mut InspectorViewState, payload: &str) -> Result<UiEventOutcome, String> {
    if let Some(tab) = payload.strip_prefix("tab:") {
        let (next_insp, sec) = match tab {
            "graph" => (ActiveInspector::FormatGraph, SectionKind::FormatGraph),
            "axes" => (ActiveInspector::FormatAxes, SectionKind::FormatAxes),
            "multi" => (ActiveInspector::BubblePlot, SectionKind::Multivariable),
            "pairwise" => (ActiveInspector::PairwiseComparisons, SectionKind::PairwiseCld),
            "layout" => (ActiveInspector::PageLayout, SectionKind::PageLayout),
            _ => return Ok(UiEventOutcome::default()),
        };
        app.canvas_dialogs.active_inspector = next_insp;
        let sec_id = sec.id();
        state.collapsed_sections.retain(|s| s != sec_id);
        state.open_popup = None;
        app.interaction.scroll_offsets.offsets.remove(SCROLL_ID);
        match next_insp {
            ActiveInspector::FormatGraph | ActiveInspector::FormatAxes => {
                let doc_snap = app.document().clone();
                if let Some(gs) = resolve_graph_sheet(&doc_snap, state).cloned() {
                    state.target_graph_id = Some(gs.id.as_str().to_string());
                    if next_insp == ActiveInspector::FormatGraph {
                        app.canvas_dialogs.format_graph.sync_from_graph_with_doc(&gs, Some(&doc_snap));
                    } else {
                        let target = app.canvas_dialogs.format_axes.target_axis;
                        app.canvas_dialogs.format_axes.sync_from_graph(&gs, target);
                    }
                }
            }
            ActiveInspector::BubblePlot | ActiveInspector::PairwiseComparisons => {
                let doc_snap = app.document().clone();
                if let Some(gs) = resolve_graph_sheet(&doc_snap, state) {
                    state.target_graph_id = Some(gs.id.as_str().to_string());
                    if next_insp == ActiveInspector::BubblePlot {
                        app.canvas_dialogs.bubble_plot.sync_from_graph(gs, &doc_snap);
                    } else {
                        app.canvas_dialogs.pairwise.sync_from_graph(gs, &doc_snap);
                    }
                }
            }
            ActiveInspector::PageLayout => {
                if let Some(ls) = resolve_layout_sheet(app.document(), state).cloned() {
                    state.target_layout_id = Some(ls.id.as_str().to_string());
                    app.canvas_dialogs.page_layout.sync_from_layout(&ls);
                }
            }
            ActiveInspector::ResultDetails | ActiveInspector::None => {}
        }
        return Ok(outcome_ok(payload, None));
    }

    if let Some(sec_raw) = payload.strip_prefix("section:") {
        let sec = match sec_raw {
            "drill_back" => SectionKind::DrillBack,
            "format_graph" => SectionKind::FormatGraph,
            "format_axes" => SectionKind::FormatAxes,
            "multivariable" => SectionKind::Multivariable,
            "pairwise" => SectionKind::PairwiseCld,
            "page_layout" => SectionKind::PageLayout,
            _ => return Ok(UiEventOutcome::default()),
        };
        let currently_open = is_section_expanded(app, state, sec);
        let id_str = String::from(sec.id());
        if currently_open {
            state.expanded_sections.retain(|s| s != sec.id());
            if !state.collapsed_sections.contains(&id_str) {
                state.collapsed_sections.push(id_str);
            }
        } else {
            state.collapsed_sections.retain(|s| s != sec.id());
            if !state.expanded_sections.contains(&id_str) {
                state.expanded_sections.push(id_str);
            }
        }
        state.open_popup = None;
        return Ok(outcome_ok(payload, None));
    }

    if let Some(pop_kind) = payload.strip_prefix("popup:") {
        if pop_kind == "close" || state.open_popup.as_deref() == Some(pop_kind) {
            state.open_popup = None;
        } else {
            state.open_popup = Some(String::from(pop_kind));
        }
        return Ok(outcome_ok(payload, None));
    }

    if let Some(sub) = payload.strip_prefix("fg.") {
        return dispatch_format_graph_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("regression.") {
        return dispatch_regression_action(app, state, sub);
    }
    if let Some(filter_raw) = payload.strip_prefix("cvd:") {
        let Some(filter) = CanvasPreviewFilter::from_name(filter_raw) else {
            return Ok(UiEventOutcome::default());
        };
        app.preview_filter = filter;
        return Ok(outcome_ok(payload, Some("ui.previewFilter.set")));
    }
    if let Some(sub) = payload.strip_prefix("axes.") {
        return dispatch_axes_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("mv.") {
        return dispatch_multivariable_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("pw.") {
        return dispatch_pairwise_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("layout.") {
        return dispatch_layout_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("canvas.") {
        return dispatch_canvas_action(app, state, sub);
    }
    if let Some(sub) = payload.strip_prefix("drill.") {
        return dispatch_drill_action(app, state, sub);
    }

    Ok(UiEventOutcome::default())
}

fn draw_section_header(ctx: &mut SectionDrawCtx<'_, '_>, id: &str, title: &str, sec: SectionKind, expanded: bool) {
    let rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(26));
    if ctx.is_row_visible(rect) {
        let prefix = if expanded { "[-] " } else { "[+] " };
        let label = format!("{prefix}{title}");
        ctx.emit_action_btn(id, rect, (&label, title), &format!("section:{}", sec.id()), expanded, true);
    }
    ctx.cursor_y += ctx.si(30);
}

fn draw_drill_back_section(ctx: &mut SectionDrawCtx<'_, '_>, gs: Option<&GraphSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::DrillBack);
    let title = insp_text(app, "inspector.section.drill_back", "Point & Drill-Back");
    draw_section_header(ctx, "inspector.section.drill_back", title, SectionKind::DrillBack, expanded);
    if !expanded {
        return;
    }

    let brushed = resolve_drill_target_fast(app, gs);
    let has_target = brushed.is_some();
    let has_pts = gs.and_then(|g| app.document().data_sheet(&g.spec.primary_source_sheet_id)).is_some_and(|ds| ds.archetype.row_count() > 0);
    let summary = match &brushed {
        Some(sc) if app.brushed_cell.as_ref() == Some(sc) && !state.selected_point_summary.is_empty() => {
            format!("{} · R{} C{} S{}", state.selected_point_summary, sc.row + 1, sc.col + 1, sc.subcol + 1)
        }
        Some(sc) => format!("Cell R{} C{} S{} ({})", sc.row + 1, sc.col + 1, sc.subcol + 1, sc.sheet_id.as_str()),
        None => String::from(insp_text(app, "w6.view.insp.no_plotted_points", "No plotted points on active sheet")),
    };
    ctx.paint_label(&summary, ctx.cursor_y, 16, 10.5, muted, false);
    ctx.cursor_y += ctx.si(18);

    let gap = ctx.su(4);
    let half = (ctx.panel_w.saturating_sub(gap) / 2).max(ctx.su(40));
    let prev_rect = LayoutRectPx::new(ctx.mirrored_x(0, half), ctx.cursor_y, half, ctx.su(22));
    let next_rect = LayoutRectPx::new(ctx.mirrored_x((half as i32) + (gap as i32), half), ctx.cursor_y, half, ctx.su(22));
    let prev_lbl = insp_text(app, "inspector.drill_back.prev_pt", "Prev Pt");
    let next_lbl = insp_text(app, "inspector.drill_back.next_pt", "Next Pt");
    ctx.emit_action_btn("inspector.drill.prev", prev_rect, (prev_lbl, prev_lbl), "drill.point:prev", false, has_pts);
    ctx.emit_action_btn("inspector.drill.next", next_rect, (next_lbl, next_lbl), "drill.point:next", false, has_pts);
    ctx.cursor_y += ctx.si(26);

    let on_data_sheet = app.document().active_sheet_id.as_ref().is_some_and(|id| app.document().data_sheet(id).is_some());
    let go_lbl = insp_text(app, "inspector.drill_back.go", "Go to Data Cell");
    let go_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(24));
    ctx.emit_action_btn("inspector.drill.go_to_cell", go_rect, (go_lbl, "Go to Data Cell / Drill Back"), "drill.go_to_cell", false, has_target);
    ctx.cursor_y += ctx.si(28);

    if on_data_sheet && gs.is_some() {
        let back_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(22));
        let back_lbl = insp_text(app, "w6.view.insp.return_to_graph", "Return to Graph");
        ctx.emit_action_btn("inspector.drill.return_graph", back_rect, (back_lbl, back_lbl), "drill.return_to_graph", false, true);
        ctx.cursor_y += ctx.si(26);
    }
}

fn draw_format_graph_section(ctx: &mut SectionDrawCtx<'_, '_>, gs: Option<&GraphSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::FormatGraph);
    let title = insp_text(app, "inspector.format_graph", "Format Graph");
    draw_section_header(ctx, "inspector.section.format_graph", title, SectionKind::FormatGraph, expanded);
    if !expanded {
        return;
    }

    let Some(gs) = gs else {
        ctx.paint_label(
            insp_text(app, "inspector.no_active_sheet", "Select a graph, layout, or data sheet to inspect."),
            ctx.cursor_y,
            18,
            10.5,
            muted,
            false,
        );
        ctx.cursor_y += ctx.si(22);
        return;
    };

    let sel_idx = resolve_selected_series_index(app, gs);
    let cur_series = gs.spec.series.get(sel_idx);
    let ds_label = cur_series.and_then(|s| s.legend_label.as_deref()).unwrap_or("Series");
    let has_series = !gs.spec.series.is_empty();

    let ds_title = insp_text(app, "inspector.fg.dataset", "Dataset / Series");
    ctx.paint_label(ds_title, ctx.cursor_y, 15, 10.5, muted, false);
    ctx.cursor_y += ctx.si(16);
    let nav_w = ctx.su(24);
    let nav_gap = ctx.su(4);
    let row_h = ctx.su(24);
    let mid_w = ctx.panel_w.saturating_sub(2 * nav_w + 2 * nav_gap).max(ctx.su(50));
    let prev_r = LayoutRectPx::new(ctx.mirrored_x(0, nav_w), ctx.cursor_y, nav_w, row_h);
    let mid_r = LayoutRectPx::new(ctx.mirrored_x((nav_w as i32) + (nav_gap as i32), mid_w), ctx.cursor_y, mid_w, row_h);
    let next_r =
        LayoutRectPx::new(ctx.mirrored_x((nav_w as i32) + (nav_gap as i32) + (mid_w as i32) + (nav_gap as i32), nav_w), ctx.cursor_y, nav_w, row_h);
    ctx.emit_action_btn(
        "inspector.fg.dataset_prev",
        prev_r,
        ("<", insp_text(app, "w6.view.insp.prev_series", "Previous series")),
        "fg.dataset:prev",
        false,
        has_series,
    );
    ctx.emit_action_btn(
        "inspector.fg.dataset_btn",
        mid_r,
        (ds_label, ds_title),
        "popup:dataset",
        state.open_popup.as_deref() == Some("dataset"),
        has_series,
    );
    ctx.emit_action_btn(
        "inspector.fg.dataset_next",
        next_r,
        (">", insp_text(app, "w6.view.insp.next_series", "Next series")),
        "fg.dataset:next",
        false,
        has_series,
    );
    if state.open_popup.as_deref() == Some("dataset") && ctx.is_row_visible(mid_r) && has_series {
        let options = gs
            .spec
            .series
            .iter()
            .enumerate()
            .map(|(i, s)| PopupOptionSpec {
                label: s.legend_label.clone().unwrap_or_else(|| format!("Series {}", i + 1)),
                payload: format!("fg.dataset:{i}"),
                active: i == sel_idx,
            })
            .collect();
        ctx.pending_popup = Some(PendingPopup { kind: String::from("dataset"), anchor: mid_r, options });
    }
    ctx.cursor_y += ctx.si(28);

    let pal_options = vec![
        PopupOptionSpec {
            label: String::from("Okabe-Ito"),
            payload: String::from("fg.palette:OkabeIto"),
            active: gs.spec.palette == PaletteId::OkabeIto,
        },
        PopupOptionSpec {
            label: String::from("Tol Bright"),
            payload: String::from("fg.palette:PaulTolBright"),
            active: gs.spec.palette == PaletteId::PaulTolBright,
        },
        PopupOptionSpec {
            label: String::from("Tol Muted"),
            payload: String::from("fg.palette:PaulTolMuted"),
            active: gs.spec.palette == PaletteId::PaulTolMuted,
        },
        PopupOptionSpec {
            label: String::from("Tol High Contrast"),
            payload: String::from("fg.palette:PaulTolHighContrast"),
            active: gs.spec.palette == PaletteId::PaulTolHighContrast,
        },
        PopupOptionSpec {
            label: String::from("Tol Vibrant"),
            payload: String::from("fg.palette:PaulTolVibrant"),
            active: gs.spec.palette == PaletteId::PaulTolVibrant,
        },
        PopupOptionSpec {
            label: String::from("Monochrome"),
            payload: String::from("fg.palette:MonochromePrint"),
            active: gs.spec.palette == PaletteId::MonochromePrint,
        },
        PopupOptionSpec { label: String::from("Viridis"), payload: String::from("fg.palette:Viridis"), active: false },
    ];
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.fg.palette_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.fg.palette", "Colour Palette"),
        current_val: palette_label(gs.spec.palette),
        popup_kind: "palette",
        open_popup: state.open_popup.as_deref(),
        options: pal_options,
        enabled: has_series,
    });

    ctx.paint_label(insp_text(app, "inspector.fg.color", "Series Colour"), ctx.cursor_y, 15, 10.5, muted, false);
    ctx.cursor_y += ctx.si(16);
    let cur_hex = cur_series.map(|s| rgba_to_hex(series_primary_color(s))).unwrap_or_default();
    let sw_gap = ctx.su(4);
    let sw_cols = 4u32;
    let sw_h = ctx.su(20);
    let sw_step = ctx.su(24);
    let sw_w = (ctx.panel_w.saturating_sub(sw_gap * (sw_cols - 1)) / sw_cols).max(ctx.su(24));
    for (idx, (cname, hex)) in COLOR_SWATCHES.iter().enumerate() {
        let r = (idx as u32) / sw_cols;
        let c = (idx as u32) % sw_cols;
        let sw_rect = LayoutRectPx::new(ctx.mirrored_x((c * (sw_w + sw_gap)) as i32, sw_w), ctx.cursor_y + (r * sw_step) as i32, sw_w, sw_h);
        if ctx.is_row_visible(sw_rect) {
            let rgba = parse_hex_rgba8(hex).unwrap_or(Rgba8::BLACK);
            let active = cur_hex.eq_ignore_ascii_case(hex);
            let id = format!("inspector.fg.swatch_{idx}");
            let a11y_lbl = format!("Series colour {cname}");
            let payload = format!("fg.color:{hex}");
            ctx.emit_action_btn(&id, sw_rect, (cname, &a11y_lbl), &payload, active, has_series);
            let strip = LayoutRectPx::new(sw_rect.x + 2, sw_rect.bottom() - 4, sw_rect.width.saturating_sub(4), 3);
            push_rect_op(ctx.builder, strip, rgba, ZLayer::Workspace, Some(ctx.scroll_clip_id), Some(wid(&id)));
        }
    }
    ctx.cursor_y += ctx.si(50);

    let cur_shape = cur_series.map_or(MarkerShape::Circle, |s| series_marker_style(s).shape);
    let shape_options = all_marker_shapes()
        .iter()
        .map(|s| PopupOptionSpec { label: String::from(marker_shape_label(*s)), payload: format!("fg.shape:{s:?}"), active: *s == cur_shape })
        .collect();
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.fg.shape_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.fg.symbol_shape", "Symbol Shape"),
        current_val: marker_shape_label(cur_shape),
        popup_kind: "symbol_shape",
        open_popup: state.open_popup.as_deref(),
        options: shape_options,
        enabled: has_series,
    });

    let size_txt = cur_series.map_or_else(|| String::from("--"), |s| format!("{:.1} pt", series_marker_style(s).size_pt));
    ctx.cursor_y = ctx.emit_stepper_row(StepperArgs {
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.fg.symbol_size", "Symbol Size"),
        value_display: &size_txt,
        dec_id: "inspector.fg.size_dec",
        dec_payload: "fg.size:dec",
        inc_id: "inspector.fg.size_inc",
        inc_payload: "fg.size:inc",
        enabled: has_series,
    });

    let cur_topo = cur_series.map_or("Bar", |s| topology_label(&s.gdo));
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.fg.topology", "Plot Topology")),
        &[
            ("inspector.fg.topo_bar", "Bar", "fg.topology:bar", cur_topo == "Bar"),
            ("inspector.fg.topo_box", "Box", "fg.topology:box", cur_topo == "Box"),
            ("inspector.fg.topo_violin", "Violin", "fg.topology:violin", cur_topo == "Violin"),
            ("inspector.fg.topo_rain", "Raincloud", "fg.topology:raincloud", cur_topo == "Raincloud"),
            ("inspector.fg.topo_dot", "Dot Plot", "fg.topology:dot", cur_topo == "Dot Plot"),
            ("inspector.fg.topo_scatter", "Scatter", "fg.topology:scatter", cur_topo == "Scatter+Line"),
        ],
        has_series,
    );

    let eb_opt = cur_series.and_then(|s| s.y_error_bars.as_ref());
    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.fg.eb_toggle",
        ctx.cursor_y,
        insp_text(app, "inspector.error_bars", "Error Bars"),
        eb_opt.is_some(),
        "fg.err_toggle",
        has_series,
    );

    let cur_metric = eb_opt.map_or(app.canvas_dialogs.format_graph.error_bar_spec.metric, |e| e.metric);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.fg.err_metric", "Error Bar Metric")),
        &[
            ("inspector.fg.metric_sd", "SD", "fg.err_metric:Sd", cur_metric == ErrorBarMetric::Sd),
            ("inspector.fg.metric_sem", "SEM", "fg.err_metric:Sem", cur_metric == ErrorBarMetric::Sem),
            ("inspector.fg.metric_ci95", "95% CI", "fg.err_metric:ConfidenceInterval95", cur_metric == ErrorBarMetric::ConfidenceInterval95),
            ("inspector.fg.metric_range", "Range", "fg.err_metric:Range", cur_metric == ErrorBarMetric::Range),
        ],
        has_series,
    );

    let cur_dir = eb_opt.map_or(app.canvas_dialogs.format_graph.error_bar_spec.direction, |e| e.direction);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.fg.err_dir", "Error Direction")),
        &[
            ("inspector.fg.dir_both", "Both", "fg.err_dir:Both", cur_dir == ErrorBarDirection::Both),
            ("inspector.fg.dir_above", "Above", "fg.err_dir:UpperOnly", cur_dir == ErrorBarDirection::UpperOnly),
            ("inspector.fg.dir_below", "Below", "fg.err_dir:LowerOnly", cur_dir == ErrorBarDirection::LowerOnly),
        ],
        has_series,
    );

    if matches!(gs.spec.kind, GraphKind::Grouped | GraphKind::Contingency) {
        ctx.cursor_y = ctx.emit_chip_row(
            ctx.cursor_y,
            Some("Grouped Arrangement"),
            &[
                (
                    "inspector.fg.arr_interleaved",
                    "Interleaved",
                    "fg.arrangement:Interleaved",
                    gs.spec.grouped_arrangement == GroupedArrangement::Interleaved,
                ),
                ("inspector.fg.arr_stacked", "Stacked", "fg.arrangement:Stacked", gs.spec.grouped_arrangement == GroupedArrangement::Stacked),
                (
                    "inspector.fg.arr_superimposed",
                    "Superimposed",
                    "fg.arrangement:Superimposed",
                    gs.spec.grouped_arrangement == GroupedArrangement::Superimposed,
                ),
            ],
            true,
        );
    }

    let style_lbl = insp_text(ctx.builder.app, "cmd.ui.applyStyleFrom.open", "Apply Style From...");
    let style_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(22));
    ctx.emit_action_btn("inspector.fg.apply_style_from", style_rect, (style_lbl, style_lbl), "fg.apply_style_from", false, true);
    ctx.cursor_y += ctx.si(25);

    let reg_candidates: Vec<(String, String)> = ctx
        .builder
        .app
        .document()
        .result_sheets
        .iter()
        .filter(|rs| !rs.fitted_curves.is_empty())
        .map(|rs| (rs.id.as_str().to_string(), rs.title().to_owned()))
        .collect();
    let cur_ov = cur_series.and_then(|s| s.regression_overlay.as_ref()).or_else(|| gs.spec.series.iter().find_map(|s| s.regression_overlay.as_ref()));
    if !reg_candidates.is_empty() || cur_ov.is_some() {
        let state_snap = &ctx.builder.app.view_shards.inspector;
        let active_res_id = cur_ov.map(|o| o.result_sheet_id.as_str().to_string()).or_else(|| state_snap.regression_result_sheet_id.clone());
        let reg_all_series = if cur_ov.is_some() && !gs.spec.series.is_empty() {
            gs.spec.series.iter().all(|s| s.regression_overlay.is_some()) || state_snap.regression_all_series
        } else {
            state_snap.regression_all_series
        };
        let reg_conf_band = cur_ov.map_or(state_snap.regression_confidence_band, |o| o.show_confidence_band);
        let reg_shaded_ribbon = cur_ov.map_or(state_snap.regression_shaded_ribbon, |o| o.shaded_ribbon);
        let reg_res_label = match &active_res_id {
            Some(rid) => reg_candidates.iter().find(|(id, _)| id == rid).map(|(_, t)| t.clone()).unwrap_or_else(|| rid.clone()),
            None => String::from("None (Select Fit)"),
        };
        let reg_muted = ctx.builder.app.theme.text_muted;
        ctx.paint_label("Regression Overlay", ctx.cursor_y, 15, 10.5, reg_muted, false);
        ctx.cursor_y += ctx.si(16);
        let reg_btn_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(24));
        ctx.emit_action_btn(
            "inspector.regression.result_sheet",
            reg_btn_rect,
            (&reg_res_label, "Regression Result Sheet"),
            "regression.cycle_result",
            active_res_id.is_some(),
            has_series,
        );
        ctx.cursor_y += ctx.si(28);
        ctx.cursor_y = ctx.emit_checkbox_row(
            "inspector.regression.all_series",
            ctx.cursor_y,
            "Apply to All Series",
            reg_all_series,
            "regression.all_series",
            has_series,
        );
        ctx.cursor_y = ctx.emit_checkbox_row(
            "inspector.regression.confidence_band",
            ctx.cursor_y,
            "95% Confidence Band",
            reg_conf_band,
            "regression.confidence_band",
            has_series,
        );
        ctx.cursor_y = ctx.emit_checkbox_row(
            "inspector.regression.shaded_ribbon",
            ctx.cursor_y,
            "Shaded Ribbon",
            reg_shaded_ribbon,
            "regression.shaded_ribbon",
            has_series,
        );
        let reg_apply_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, ctx.su(22));
        ctx.emit_action_btn(
            "inspector.regression.apply",
            reg_apply_rect,
            ("Apply Overlay", "Apply Regression Overlay"),
            "regression.apply",
            false,
            has_series,
        );
        ctx.cursor_y += ctx.si(28);
    }

    let cur_cvd = ctx.builder.app.preview_filter;
    ctx.cursor_y = ctx.emit_chip_row_capped(
        ctx.cursor_y,
        Some("CVD Preview"),
        &[
            ("inspector.cvd.normal", "Norm", "cvd:normal", cur_cvd == CanvasPreviewFilter::Normal),
            ("inspector.cvd.protanopia", "Prot", "cvd:protanopia", cur_cvd == CanvasPreviewFilter::Protanopia),
            ("inspector.cvd.deuteranopia", "Deut", "cvd:deuteranopia", cur_cvd == CanvasPreviewFilter::Deuteranopia),
            ("inspector.cvd.tritanopia", "Trit", "cvd:tritanopia", cur_cvd == CanvasPreviewFilter::Tritanopia),
            ("inspector.cvd.grayscale", "Gray", "cvd:grayscale", cur_cvd == CanvasPreviewFilter::GrayscalePrint),
        ],
        5,
        true,
    );

    if let Some(lvl_target) = resolve_mv_level_target(ctx.builder.app.document(), gs) {
        let loc = ctx.builder.tree.locale.clone();
        let muted = ctx.builder.app.theme.text_muted;
        let sel_name = ctx.builder.app.view_shards.inspector.selected_fg_level_name.clone();
        let sel_idx = resolve_selected_level_idx(&lvl_target.levels, sel_name.as_deref());
        let heading = if lvl_target.is_x {
            crate::i18n::tr(&loc, "w7.mv.level_ordering_section")
        } else {
            crate::i18n::tr(&loc, "w7.mv.color_ordering_section")
        };
        ctx.paint_label(heading, ctx.cursor_y, 16, 10.5, muted, false);
        ctx.cursor_y += ctx.si(18);

        let gap = ctx.su(4);
        let half = (ctx.panel_w.saturating_sub(gap) / 2).max(ctx.su(44));
        let row_h = ctx.su(22);
        let can_up = sel_idx.is_some_and(|i| i > 0);
        let can_dn = sel_idx.is_some_and(|i| i + 1 < lvl_target.levels.len());
        let top_lbl = crate::i18n::tr(&loc, "w7.mv.move_to_top");
        let up_lbl = crate::i18n::tr(&loc, "w7.mv.move_up");
        let dn_lbl = crate::i18n::tr(&loc, "w7.mv.move_down");
        let end_lbl = crate::i18n::tr(&loc, "w7.mv.move_to_end");
        ctx.emit_action_btn(
            "inspector.fg.level_top",
            LayoutRectPx::new(ctx.mirrored_x(0, half), ctx.cursor_y, half, row_h),
            (top_lbl, top_lbl),
            "fg.level:move:top",
            false,
            can_up,
        );
        ctx.emit_action_btn(
            "inspector.fg.level_up",
            LayoutRectPx::new(ctx.mirrored_x((half as i32) + (gap as i32), half), ctx.cursor_y, half, row_h),
            (up_lbl, up_lbl),
            "fg.level:move:up",
            false,
            can_up,
        );
        ctx.cursor_y += ctx.si(24);
        ctx.emit_action_btn(
            "inspector.fg.level_dn",
            LayoutRectPx::new(ctx.mirrored_x(0, half), ctx.cursor_y, half, row_h),
            (dn_lbl, dn_lbl),
            "fg.level:move:down",
            false,
            can_dn,
        );
        ctx.emit_action_btn(
            "inspector.fg.level_end",
            LayoutRectPx::new(ctx.mirrored_x((half as i32) + (gap as i32), half), ctx.cursor_y, half, row_h),
            (end_lbl, end_lbl),
            "fg.level:move:end",
            false,
            can_dn,
        );
        ctx.cursor_y += ctx.si(26);

        for (idx, lvl) in lvl_target.levels.iter().enumerate() {
            let wid = format!("inspector.fg.level.{idx}");
            let payload = format!("fg.level:select:{idx}");
            let r = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, row_h);
            ctx.emit_action_btn(&wid, r, (lvl, lvl), &payload, sel_idx == Some(idx), true);
            ctx.cursor_y += ctx.si(24);
        }
    }
}

struct AxisInputArgs<'a> {
    input_id: &'a str,
    label: &'a str,
    live_text: &'a str,
    dec_id: &'a str,
    dec_payload: &'a str,
    inc_id: &'a str,
    inc_payload: &'a str,
}

fn draw_axis_input_row(ctx: &mut SectionDrawCtx<'_, '_>, args: AxisInputArgs<'_>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let fallback_clean = if args.live_text.eq_ignore_ascii_case("auto") { "" } else { args.live_text };
    let display_val = app.interaction.text_field_buffers.get(args.input_id).map_or_else(|| String::from(fallback_clean), |st| st.text.clone());
    let label_w = if ctx.panel_w < ctx.su(210) { ctx.su(52) } else { ctx.su(68) };
    let btn_w = ctx.su(25);
    let btn_gap = ctx.su(2);
    let stepper_w = btn_w.saturating_mul(2).saturating_add(btn_gap).saturating_add(ctx.su(4));
    let row_h = ctx.su(22);
    let gap = ctx.su(4);
    let field_w = ctx.panel_w.saturating_sub(label_w + stepper_w + gap * 2).max(ctx.su(44));

    let lbl_rect = LayoutRectPx::new(ctx.mirrored_x(0, label_w), ctx.cursor_y + ctx.si(3), label_w, ctx.su(18));
    if ctx.is_row_visible(lbl_rect) {
        let lbl_style = UiTextStyle::ui_sans(ctx.sp(10.5));
        paint_text_fitted(
            ctx.builder,
            ctx.parent_wid,
            FittedTextSpec {
                rect: lbl_rect,
                text: args.label,
                style: &lbl_style,
                pad_x: 2,
                color: muted,
                z_layer: ZLayer::Workspace,
                scroll_container: Some(SCROLL_ID),
            },
        );
    }

    let field_rel = (label_w as i32) + (gap as i32);
    let field_rect = LayoutRectPx::new(ctx.mirrored_x(field_rel, field_w), ctx.cursor_y, field_w, row_h);
    if ctx.is_row_visible(field_rect) {
        let kbd_path = format!("Inspector > {}", args.label);
        emit_text_field(
            ctx.builder,
            TextFieldSpec {
                id: args.input_id,
                rect: field_rect,
                label: args.label,
                keyboard_path: &kbd_path,
                fallback_text: &display_val,
                multiline: false,
                parent: Some(ctx.parent_wid.clone()),
                focus_scope: FocusScope::Workspace,
                z_layer: ZLayer::Workspace,
                tab_order: Some(0),
                enablement: ControlEnablement::enabled(),
            },
        );
    }

    let dec_rel = field_rel + (field_w as i32) + (gap as i32);
    let inc_rel = dec_rel + i32::try_from(btn_w + btn_gap).unwrap_or(27);
    let dec_r = LayoutRectPx::new(ctx.mirrored_x(dec_rel, btn_w), ctx.cursor_y, btn_w, row_h);
    let inc_r = LayoutRectPx::new(ctx.mirrored_x(inc_rel, btn_w), ctx.cursor_y, btn_w, row_h);
    ctx.emit_action_btn(args.dec_id, dec_r, ("-", &format!("Decrease {}", args.label)), args.dec_payload, false, true);
    ctx.emit_action_btn(args.inc_id, inc_r, ("+", &format!("Increase {}", args.label)), args.inc_payload, false, true);

    ctx.cursor_y += ctx.si(26);
}

fn draw_format_axes_section(ctx: &mut SectionDrawCtx<'_, '_>, gs: Option<&GraphSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::FormatAxes);
    let title = insp_text(app, "inspector.format_axes", "Format Axes");
    draw_section_header(ctx, "inspector.section.format_axes", title, SectionKind::FormatAxes, expanded);
    if !expanded {
        return;
    }

    let Some(gs) = gs else {
        ctx.paint_label(
            insp_text(app, "inspector.no_active_sheet", "Select a graph, layout, or data sheet to inspect."),
            ctx.cursor_y,
            18,
            10.5,
            muted,
            false,
        );
        ctx.cursor_y += ctx.si(22);
        return;
    };

    let which = current_axis_which(app);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.axes.target", "Target Axis")),
        &[
            ("inspector.axes.target_x", "X Axis", "axes.target:x", which == AxisWhich::X),
            ("inspector.axes.target_y", "Y Axis", "axes.target:y", which == AxisWhich::Y),
            ("inspector.axes.target_y2", "Y2 Axis", "axes.target:y2", which == AxisWhich::Y2),
        ],
        true,
    );

    if which == AxisWhich::Y2 {
        ctx.cursor_y = ctx.emit_checkbox_row(
            "inspector.axes.y2_enable",
            ctx.cursor_y,
            insp_text(app, "w6.view.insp.enable_y2", "Enable Right Y2 Axis"),
            gs.spec.y2_axis.is_some(),
            "axes.y2_toggle",
            true,
        );
    }

    let axis = read_live_axis_spec(gs, which);
    {
        let app_ref = ctx.builder.app;
        let muted_c = app_ref.theme.text_muted;
        let title_val = app_ref.interaction.text_field_buffers.get(AXIS_TITLE_INPUT_ID).map_or_else(|| axis.title.clone(), |st| st.text.clone());
        let lbl_w = if ctx.panel_w < ctx.su(210) { ctx.su(40) } else { ctx.su(50) };
        let btn_w = ctx.su(48);
        let gap = ctx.su(4);
        let field_w = ctx.panel_w.saturating_sub(lbl_w + btn_w + gap * 2).max(ctx.su(52));
        let row_h = ctx.su(22);
        let lbl_rect = LayoutRectPx::new(ctx.mirrored_x(0, lbl_w), ctx.cursor_y + ctx.si(3), lbl_w, ctx.su(18));
        if ctx.is_row_visible(lbl_rect) {
            let lbl_style = UiTextStyle::ui_sans(ctx.sp(10.5));
            paint_text_fitted(
                ctx.builder,
                ctx.parent_wid,
                FittedTextSpec {
                    rect: lbl_rect,
                    text: "Title",
                    style: &lbl_style,
                    pad_x: 2,
                    color: muted_c,
                    z_layer: ZLayer::Workspace,
                    scroll_container: Some(SCROLL_ID),
                },
            );
        }
        let field_rel = (lbl_w as i32) + (gap as i32);
        let field_rect = LayoutRectPx::new(ctx.mirrored_x(field_rel, field_w), ctx.cursor_y, field_w, row_h);
        if ctx.is_row_visible(field_rect) {
            emit_text_field(
                ctx.builder,
                TextFieldSpec {
                    id: AXIS_TITLE_INPUT_ID,
                    rect: field_rect,
                    label: "Axis Title",
                    keyboard_path: "Inspector > Axis Title",
                    fallback_text: &title_val,
                    multiline: false,
                    parent: Some(ctx.parent_wid.clone()),
                    focus_scope: FocusScope::Workspace,
                    z_layer: ZLayer::Workspace,
                    tab_order: Some(0),
                    enablement: ControlEnablement::enabled(),
                },
            );
        }
        let btn_rel = field_rel + (field_w as i32) + (gap as i32);
        let btn_rect = LayoutRectPx::new(ctx.mirrored_x(btn_rel, btn_w), ctx.cursor_y, btn_w, row_h);
        ctx.emit_action_btn("inspector.axes.title_apply", btn_rect, ("Apply", "Apply Axis Title"), "axes.apply_title", false, true);
        ctx.cursor_y += ctx.si(26);
    }
    let seg = axis_primary_seg(&axis);
    let app = ctx.builder.app;
    let state = &app.view_shards.inspector;
    let scale_options = vec![
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.linear", "Linear")),
            payload: String::from("axes.scale:Linear"),
            active: seg.scale == AxisScale::Linear,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.log10", "Log10")),
            payload: String::from("axes.scale:Log10"),
            active: seg.scale == AxisScale::Log10,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.log2", "Log2")),
            payload: String::from("axes.scale:Log2"),
            active: seg.scale == AxisScale::Log2,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.ln", "Ln (Natural)")),
            payload: String::from("axes.scale:Ln"),
            active: seg.scale == AxisScale::Ln,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.logit", "Logit")),
            payload: String::from("axes.scale:Logit"),
            active: matches!(seg.scale, AxisScale::Logit(_)),
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.probit", "Probit")),
            payload: String::from("axes.scale:Probit"),
            active: seg.scale == AxisScale::Probit,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "inspector.scale.reciprocal", "Reciprocal 1/Y")),
            payload: String::from("axes.scale:Reciprocal"),
            active: seg.scale == AxisScale::Reciprocal,
        },
    ];
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.axes.scale_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.axes.scale", "Axis Scale"),
        current_val: axis_scale_label(seg.scale),
        popup_kind: "axis_scale",
        open_popup: state.open_popup.as_deref(),
        options: scale_options,
        enabled: true,
    });

    let is_nonlinear = matches!(seg.scale, AxisScale::Ln | AxisScale::Logit(_) | AxisScale::Probit | AxisScale::Reciprocal);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        None,
        &[
            ("inspector.axes.scale_linear", insp_text(app, "inspector.scale.linear", "Linear"), "axes.scale:Linear", seg.scale == AxisScale::Linear),
            ("inspector.axes.scale_log10", insp_text(app, "inspector.scale.log10", "Log10"), "axes.scale:Log10", seg.scale == AxisScale::Log10),
            ("inspector.axes.scale_log2", insp_text(app, "inspector.scale.log2", "Log2"), "axes.scale:Log2", seg.scale == AxisScale::Log2),
            ("inspector.axes.scale_nonlin", "Nonlin", "axes.scale:Nonlinear", is_nonlinear),
        ],
        true,
    );

    let min_txt = format_opt_num(seg.min);
    draw_axis_input_row(
        ctx,
        AxisInputArgs {
            input_id: AXIS_MIN_INPUT_ID,
            label: insp_text(app, "inspector.axes.min", "Min"),
            live_text: &min_txt,
            dec_id: "inspector.axes.min_dec",
            dec_payload: "axes.min:dec",
            inc_id: "inspector.axes.min_inc",
            inc_payload: "axes.min:inc",
        },
    );
    let max_txt = format_opt_num(seg.max);
    draw_axis_input_row(
        ctx,
        AxisInputArgs {
            input_id: AXIS_MAX_INPUT_ID,
            label: insp_text(app, "inspector.axes.max", "Max"),
            live_text: &max_txt,
            dec_id: "inspector.axes.max_dec",
            dec_payload: "axes.max:dec",
            inc_id: "inspector.axes.max_inc",
            inc_payload: "axes.max:inc",
        },
    );
    let iv_txt = format_opt_num(seg.major_tick_interval);
    draw_axis_input_row(
        ctx,
        AxisInputArgs {
            input_id: AXIS_INTERVAL_INPUT_ID,
            label: insp_text(app, "inspector.axes.interval", "Interval"),
            live_text: &iv_txt,
            dec_id: "inspector.axes.iv_dec",
            dec_payload: "axes.interval:dec",
            inc_id: "inspector.axes.iv_inc",
            inc_payload: "axes.interval:inc",
        },
    );

    let gap = ctx.su(4);
    let half = (ctx.panel_w.saturating_sub(gap) / 2).max(ctx.su(40));
    let apply_r = LayoutRectPx::new(ctx.mirrored_x(0, half), ctx.cursor_y, half, ctx.su(22));
    let auto_r = LayoutRectPx::new(ctx.mirrored_x((half as i32) + (gap as i32), half), ctx.cursor_y, half, ctx.su(22));
    let apply_lbl = insp_text(app, "w6.view.insp.apply_range", "Apply Range");
    let auto_lbl = insp_text(app, "w6.view.insp.auto_range", "Auto Range");
    ctx.emit_action_btn("inspector.axes.apply_range", apply_r, (apply_lbl, apply_lbl), "axes.apply_range", false, true);
    ctx.emit_action_btn("inspector.axes.auto_range", auto_r, (auto_lbl, auto_lbl), "axes.auto_range", false, true);
    ctx.cursor_y += ctx.si(26);

    let seg_cnt = axis.segments.len().max(1);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.axes.breaks", "Axis Breaks")),
        &[
            ("inspector.axes.breaks_1", insp_text(app, "w6.view.insp.breaks_none", "1 (None)"), "axes.breaks:1", seg_cnt <= 1),
            ("inspector.axes.breaks_2", insp_text(app, "w6.view.insp.breaks_two", "2 Segs"), "axes.breaks:2", seg_cnt == 2),
            ("inspector.axes.breaks_3", insp_text(app, "w6.view.insp.breaks_three", "3 Segs"), "axes.breaks:3", seg_cnt >= 3),
        ],
        true,
    );

    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.axes.ornament", "Break Style")),
        &[
            (
                "inspector.axes.orn_double",
                insp_text(app, "w6.view.insp.orn_slash", "Slash"),
                "axes.ornament:DoubleSlash",
                axis.break_ornament == BreakOrnament::DoubleSlash,
            ),
            (
                "inspector.axes.orn_zigzag",
                insp_text(app, "w6.view.insp.orn_zigzag", "Zigzag"),
                "axes.ornament:Zigzag",
                axis.break_ornament == BreakOrnament::Zigzag,
            ),
            (
                "inspector.axes.orn_gap",
                insp_text(app, "w6.view.insp.orn_gap", "Gap"),
                "axes.ornament:BlankGap",
                axis.break_ornament == BreakOrnament::BlankGap,
            ),
        ],
        true,
    );
}

fn draw_multivariable_section(ctx: &mut SectionDrawCtx<'_, '_>, gs: Option<&GraphSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::Multivariable);
    let title = insp_text(app, "inspector.section.multivariate", "Multivariable Channels");
    draw_section_header(ctx, "inspector.section.multivariate", title, SectionKind::Multivariable, expanded);
    if !expanded {
        return;
    }

    let Some(gs) = gs else {
        ctx.paint_label(
            insp_text(app, "inspector.no_active_sheet", "Select a graph, layout, or data sheet to inspect."),
            ctx.cursor_y,
            18,
            10.5,
            muted,
            false,
        );
        ctx.cursor_y += ctx.si(22);
        return;
    };

    let cols = source_columns_for_graph(app.document(), gs);
    let has_cols = !cols.is_empty();
    let enc = current_mv_encoding(app.document(), gs);
    let col_name = |c: usize| -> String { cols.iter().find(|(idx, _)| *idx == c).map_or_else(|| format!("Col {}", c + 1), |(_, t)| t.clone()) };
    let is_cat_summary = matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. });
    let active_metric = mv_summary_measure(&enc);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some("Plot Mode"),
        &[
            ("inspector.mv.mode_bubble", "Bubble", "mv.mode:bubble", !is_cat_summary),
            ("inspector.mv.mode_cat_summary", "Cat Summary", "mv.mode:cat_summary", is_cat_summary),
        ],
        true,
    );
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some("Summary Metric"),
        &[
            ("inspector.mv.metric_mean", "Mean", "mv.metric:Mean", active_metric == Some(SummaryMeasure::Mean)),
            ("inspector.mv.metric_median", "Median", "mv.metric:Median", active_metric == Some(SummaryMeasure::Median)),
            ("inspector.mv.metric_count", "Count", "mv.metric:Count", active_metric == Some(SummaryMeasure::GeometricMean)),
        ],
        true,
    );
    let app = ctx.builder.app;
    let state = &app.view_shards.inspector;

    let x_opts: Vec<PopupOptionSpec> =
        cols.iter().map(|(c, t)| PopupOptionSpec { label: t.clone(), payload: format!("mv.x:{c}"), active: *c == enc.x_column }).collect();
    let x_name = col_name(enc.x_column);
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.mv.x_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.var_x", "X Variable"),
        current_val: &x_name,
        popup_kind: "mv_x",
        open_popup: state.open_popup.as_deref(),
        options: x_opts,
        enabled: has_cols,
    });

    let y_opts: Vec<PopupOptionSpec> =
        cols.iter().map(|(c, t)| PopupOptionSpec { label: t.clone(), payload: format!("mv.y:{c}"), active: *c == enc.y_column }).collect();
    let y_name = col_name(enc.y_column);
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.mv.y_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.var_y", "Y Variable"),
        current_val: &y_name,
        popup_kind: "mv_y",
        open_popup: state.open_popup.as_deref(),
        options: y_opts,
        enabled: has_cols,
    });

    let (color_val, active_color_col, is_cont) = match &enc.color {
        ColorChannelMapping::Constant(_) => (String::from("None"), None, false),
        ColorChannelMapping::Categorical { column, .. } => (format!("{} (Cat)", col_name(*column)), Some(*column), false),
        ColorChannelMapping::Continuous { column, .. } => (format!("{} (Viridis)", col_name(*column)), Some(*column), true),
    };
    let mut c_opts =
        vec![PopupOptionSpec { label: String::from("None"), payload: String::from("mv.color:none"), active: active_color_col.is_none() }];
    c_opts.extend(cols.iter().map(|(c, t)| PopupOptionSpec {
        label: t.clone(),
        payload: format!("mv.color:{c}"),
        active: active_color_col == Some(*c),
    }));
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.mv.color_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.mv.color_var", "Colour Channel"),
        current_val: &color_val,
        popup_kind: "mv_color",
        open_popup: state.open_popup.as_deref(),
        options: c_opts,
        enabled: has_cols,
    });

    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        None,
        &[
            ("inspector.mv.color_cat", "Categorical", "mv.color_mode:cat", active_color_col.is_some() && !is_cont),
            ("inspector.mv.color_cont", "Viridis", "mv.color_mode:cont", is_cont),
        ],
        has_cols,
    );

    {
        let app_ref = ctx.builder.app;
        let muted_c = app_ref.theme.text_muted;
        let clamp_pair = match &enc.color {
            ColorChannelMapping::Continuous { domain_clamp, .. } => *domain_clamp,
            _ => None,
        };
        let min_fallback = clamp_pair.map(|(lo, _)| format!("{lo}")).unwrap_or_default();
        let max_fallback = clamp_pair.map(|(_, hi)| format!("{hi}")).unwrap_or_default();
        let min_val = app_ref.interaction.text_field_buffers.get(MV_CLAMP_MIN_INPUT_ID).map_or(min_fallback, |st| st.text.clone());
        let max_val = app_ref.interaction.text_field_buffers.get(MV_CLAMP_MAX_INPUT_ID).map_or(max_fallback, |st| st.text.clone());
        ctx.paint_label("Color Clamp (Min / Max)", ctx.cursor_y, 15, 10.5, muted_c, false);
        ctx.cursor_y += ctx.si(16);
        let gap = ctx.su(4);
        let row_h = ctx.su(22);
        let apply_w = ctx.su(44);
        let fields_avail = ctx.panel_w.saturating_sub(apply_w + gap * 2);
        let field_w = (fields_avail / 2).max(ctx.su(38));
        let apply_actual_w = ctx.panel_w.saturating_sub(field_w * 2 + gap * 2).max(ctx.su(36));
        let min_r = LayoutRectPx::new(ctx.mirrored_x(0, field_w), ctx.cursor_y, field_w, row_h);
        let max_rel = (field_w as i32) + (gap as i32);
        let max_r = LayoutRectPx::new(ctx.mirrored_x(max_rel, field_w), ctx.cursor_y, field_w, row_h);
        let app_rel = max_rel + (field_w as i32) + (gap as i32);
        let app_r = LayoutRectPx::new(ctx.mirrored_x(app_rel, apply_actual_w), ctx.cursor_y, apply_actual_w, row_h);
        if ctx.is_row_visible(min_r) {
            emit_text_field(
                ctx.builder,
                TextFieldSpec {
                    id: MV_CLAMP_MIN_INPUT_ID,
                    rect: min_r,
                    label: "Color Clamp Min",
                    keyboard_path: "Inspector > Color Clamp Min",
                    fallback_text: &min_val,
                    multiline: false,
                    parent: Some(ctx.parent_wid.clone()),
                    focus_scope: FocusScope::Workspace,
                    z_layer: ZLayer::Workspace,
                    tab_order: Some(0),
                    enablement: ControlEnablement::enabled(),
                },
            );
        }
        if ctx.is_row_visible(max_r) {
            emit_text_field(
                ctx.builder,
                TextFieldSpec {
                    id: MV_CLAMP_MAX_INPUT_ID,
                    rect: max_r,
                    label: "Color Clamp Max",
                    keyboard_path: "Inspector > Color Clamp Max",
                    fallback_text: &max_val,
                    multiline: false,
                    parent: Some(ctx.parent_wid.clone()),
                    focus_scope: FocusScope::Workspace,
                    z_layer: ZLayer::Workspace,
                    tab_order: Some(0),
                    enablement: ControlEnablement::enabled(),
                },
            );
        }
        ctx.emit_action_btn("inspector.mv.color_clamp_apply", app_r, ("Apply", "Apply Color Clamp"), "mv.clamp_apply", false, true);
        ctx.cursor_y += ctx.si(26);
    }
    let app = ctx.builder.app;
    let state = &app.view_shards.inspector;

    let (size_val, active_size_col) = match &enc.size {
        SizeChannelMapping::Constant { .. } => (String::from("None"), None),
        SizeChannelMapping::Variable { column, .. } => (col_name(*column), Some(*column)),
    };
    let mut s_opts = vec![PopupOptionSpec { label: String::from("None"), payload: String::from("mv.size:none"), active: active_size_col.is_none() }];
    s_opts.extend(cols.iter().map(|(c, t)| PopupOptionSpec {
        label: t.clone(),
        payload: format!("mv.size:{c}"),
        active: active_size_col == Some(*c),
    }));
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.mv.size_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.mv.size_var", "Size Channel"),
        current_val: &size_val,
        popup_kind: "mv_size",
        open_popup: state.open_popup.as_deref(),
        options: s_opts,
        enabled: has_cols,
    });

    let active_group_col = enc.label_column;
    let group_val = active_group_col.map_or_else(|| String::from("None"), col_name);
    let mut g_opts =
        vec![PopupOptionSpec { label: String::from("None"), payload: String::from("mv.group:none"), active: active_group_col.is_none() }];
    g_opts.extend(cols.iter().map(|(c, t)| PopupOptionSpec {
        label: t.clone(),
        payload: format!("mv.group:{c}"),
        active: active_group_col == Some(*c),
    }));
    let group_lbl = crate::i18n::tr(&ctx.builder.tree.locale, "w7.mv.grouping_label");
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.mv.group_btn",
        y: ctx.cursor_y,
        title: group_lbl,
        current_val: &group_val,
        popup_kind: "mv_group",
        open_popup: state.open_popup.as_deref(),
        options: g_opts,
        enabled: has_cols,
    });

    let app = ctx.builder.app;
    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.mv.ellipse_toggle",
        ctx.cursor_y,
        insp_text(app, "inspector.mv.ellipse", "95% Confidence Ellipse"),
        enc.show_confidence_ellipses,
        "mv.ellipse:toggle",
        true,
    );
    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.mv.hull_toggle",
        ctx.cursor_y,
        insp_text(app, "inspector.mv.hull", "Convex Hull"),
        enc.show_convex_hulls,
        "mv.hull:toggle",
        true,
    );

    let loc = ctx.builder.tree.locale.clone();
    let muted = ctx.builder.app.theme.text_muted;
    ctx.paint_label(crate::i18n::tr(&loc, "w7.mv.var_definitions"), ctx.cursor_y, 16, 10.5, muted, false);
    ctx.cursor_y += ctx.si(18);
    let channel_defs: [(&str, &str, &str, Option<usize>); 5] = [
        ("inspector.mv.def_x_btn", "mv_def_x", "X", Some(enc.x_column)),
        ("inspector.mv.def_y_btn", "mv_def_y", "Y", Some(enc.y_column)),
        ("inspector.mv.def_color_btn", "mv_def_color", "Colour", active_color_col),
        ("inspector.mv.def_size_btn", "mv_def_size", "Size", active_size_col),
        ("inspector.mv.def_group_btn", "mv_def_group", "Group", active_group_col),
    ];
    for (btn_id, popup_kind, ch_tag, col_opt) in channel_defs {
        let app_ref = ctx.builder.app;
        let open_popup = app_ref.view_shards.inspector.open_popup.as_deref();
        let (title, val, opts, en) = match col_opt.and_then(|c| mv_column_role_info(app_ref.document(), gs, c).map(|info| (c, info))) {
            Some((c, (role, locked_or_frozen))) => {
                let role_opts = vec![
                    PopupOptionSpec {
                        label: crate::i18n::tr(&loc, "w7.mv.var_role_continuous").to_owned(),
                        payload: format!("mv.role:{c}:continuous"),
                        active: role == VariableRole::Continuous,
                    },
                    PopupOptionSpec {
                        label: crate::i18n::tr(&loc, "w7.mv.var_role_categorical").to_owned(),
                        payload: format!("mv.role:{c}:categorical"),
                        active: role == VariableRole::Categorical,
                    },
                    PopupOptionSpec {
                        label: crate::i18n::tr(&loc, "w7.mv.var_role_label").to_owned(),
                        payload: format!("mv.role:{c}:label"),
                        active: role == VariableRole::Label,
                    },
                    PopupOptionSpec { label: String::from("Auto"), payload: format!("mv.role:{c}:auto"), active: false },
                ];
                (format!("{ch_tag}: {}", col_name(c)), String::from(role_display_name(&loc, role)), role_opts, !locked_or_frozen)
            }
            None => (format!("{ch_tag}: None"), String::from("Unbound"), Vec::new(), false),
        };
        ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
            id: btn_id,
            y: ctx.cursor_y,
            title: &title,
            current_val: &val,
            popup_kind,
            open_popup,
            options: opts,
            enabled: en,
        });
    }
}

fn cld_mode_label(app: &UiApp, mode: PairwiseDisplayMode) -> &'static str {
    match mode {
        PairwiseDisplayMode::BracketsOnly => insp_text(app, "w6.view.insp.brackets_only", "Brackets Only"),
        PairwiseDisplayMode::CompactLetterDisplayOnly => insp_text(app, "w6.view.insp.cld_letters", "CLD Letters"),
        PairwiseDisplayMode::BracketsAndCompactLetterDisplay => insp_text(app, "w6.view.insp.brackets_and_cld", "Brackets + CLD"),
    }
}

fn draw_pairwise_section(ctx: &mut SectionDrawCtx<'_, '_>, gs: Option<&GraphSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::PairwiseCld);
    let title = insp_text(app, "inspector.section.pairwise_cld", "Pairwise & CLD");
    draw_section_header(ctx, "inspector.section.pairwise_cld", title, SectionKind::PairwiseCld, expanded);
    if !expanded {
        return;
    }

    let Some(gs) = gs else {
        ctx.paint_label(
            insp_text(app, "inspector.no_active_sheet", "Select a graph, layout, or data sheet to inspect."),
            ctx.cursor_y,
            18,
            10.5,
            muted,
            false,
        );
        ctx.cursor_y += ctx.si(22);
        return;
    };

    let maybe_rid = find_pairwise_result_sheet_id(app.document(), gs);
    let has_result = maybe_rid.is_some();
    let gap = ctx.su(4);
    let half = (ctx.panel_w.saturating_sub(gap) / 2).max(ctx.su(40));
    let row_h = ctx.su(24);
    let auto_r = LayoutRectPx::new(ctx.mirrored_x(0, half), ctx.cursor_y, half, row_h);
    let clr_r = LayoutRectPx::new(ctx.mirrored_x((half as i32) + (gap as i32), half), ctx.cursor_y, half, row_h);
    let auto_lbl = insp_text(app, "inspector.pw.auto_brackets", "Auto Brackets");
    let clr_lbl = insp_text(app, "inspector.pw.clear_brackets", "Clear");
    ctx.emit_action_btn("inspector.pw.auto_brackets", auto_r, (auto_lbl, auto_lbl), "pw.auto_brackets", false, has_result);
    ctx.emit_action_btn(
        "inspector.pw.clear_brackets",
        clr_r,
        (clr_lbl, "Clear Brackets"),
        "pw.clear_brackets",
        false,
        !gs.spec.comparisons.is_empty(),
    );
    ctx.cursor_y += ctx.si(28);

    let is_stars = matches!(app.canvas_dialogs.pairwise.label_mode, BracketLabelMode::SummaryStars);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some("Bracket Labels"),
        &[
            ("inspector.pw.lbl_stars", "Stars (*)", "pw.label_mode:stars", is_stars),
            ("inspector.pw.lbl_p", "Exact p", "pw.label_mode:numeric", !is_stars),
        ],
        has_result,
    );

    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.pw.sig_only",
        ctx.cursor_y,
        "Significant Pairs Only",
        app.canvas_dialogs.pairwise.hide_non_significant,
        "pw.sig_only:toggle",
        has_result,
    );

    let cld_spec = maybe_rid.as_ref().map(|r| cld_or_default(gs, r));
    let cld_enabled = is_cld_active(gs);
    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.pw.cld_toggle",
        ctx.cursor_y,
        insp_text(app, "inspector.pw.cld_toggle", "Compact Letter Display (CLD)"),
        cld_enabled,
        "pw.cld_toggle",
        has_result,
    );

    let mode_opts = vec![
        PopupOptionSpec {
            label: String::from(insp_text(app, "w6.view.insp.brackets_only", "Brackets Only")),
            payload: String::from("pw.cld_mode:BracketsOnly"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "w6.view.insp.cld_letters", "CLD Letters")),
            payload: String::from("pw.cld_mode:CompactLetterDisplayOnly"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::CompactLetterDisplayOnly,
        },
        PopupOptionSpec {
            label: String::from(insp_text(app, "w6.view.insp.brackets_and_cld", "Brackets + CLD")),
            payload: String::from("pw.cld_mode:BracketsAndCompactLetterDisplay"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
        },
    ];
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.pw.cld_mode_btn",
        y: ctx.cursor_y,
        title: insp_text(app, "w6.view.insp.pairwise_mode", "Pairwise Mode"),
        current_val: cld_mode_label(app, gs.spec.pairwise_display_mode),
        popup_kind: "cld_mode",
        open_popup: state.open_popup.as_deref(),
        options: mode_opts,
        enabled: has_result,
    });

    let ordering = cld_spec.as_ref().map_or(CldOrdering::AscendingMean, |c| c.ordering);
    let uppercase = cld_spec.as_ref().is_none_or(|c| c.uppercase);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.pw.cld_order", "CLD Ordering")),
        &[
            ("inspector.pw.order_asc", "Ascending", "pw.cld_order:AscendingMean", ordering == CldOrdering::AscendingMean),
            ("inspector.pw.order_desc", "Descending", "pw.cld_order:DescendingMean", ordering == CldOrdering::DescendingMean),
        ],
        has_result,
    );

    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.pw.cld_case",
        ctx.cursor_y,
        insp_text(app, "inspector.pw.cld_case", "Uppercase Letters (A, B)"),
        uppercase,
        "pw.cld_case:toggle",
        has_result,
    );

    let state_word = if cld_enabled { insp_text(app, "w6.view.insp.on", "On") } else { insp_text(app, "w6.view.insp.off", "Off") };
    let info_line = format!("Brackets: {} · CLD: {state_word}", gs.spec.comparisons.len());
    ctx.paint_label(&info_line, ctx.cursor_y, 16, 10.5, muted, false);
    ctx.cursor_y += ctx.si(20);
}

fn draw_page_layout_section(ctx: &mut SectionDrawCtx<'_, '_>, ls: Option<&LayoutSheet>) {
    let app = ctx.builder.app;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::PageLayout);
    let title = insp_text(app, "inspector.page_layout", "Page Layout");
    draw_section_header(ctx, "inspector.section.page_layout", title, SectionKind::PageLayout, expanded);
    if !expanded {
        return;
    }

    let active_preset = ctx.builder.app.canvas_dialogs.active_canvas_preset;
    let resize_mode = ctx.builder.app.canvas_dialogs.canvas_resize_mode;
    let has_canvas_target = resolve_renderable_canvas_target(ctx.builder.app.document(), &ctx.builder.app.view_shards.inspector).is_some();
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some("Canvas Presets"),
        &[
            (
                "inspector.canvas.preset_single_88mm",
                "88mm 1-Col",
                "canvas.preset:SingleColumn88mm",
                active_preset == Some(FigureCanvasPreset::SingleColumn88mm),
            ),
            (
                "inspector.canvas.preset_double_180mm",
                "180mm 2-Col",
                "canvas.preset:DoubleColumn180mm",
                active_preset == Some(FigureCanvasPreset::DoubleColumn180mm),
            ),
            ("inspector.canvas.preset_slide_16x9", "Slide 16:9", "canvas.preset:Slide16x9", active_preset == Some(FigureCanvasPreset::Slide16x9)),
        ],
        has_canvas_target,
    );
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some("Resize Mode"),
        &[
            ("inspector.canvas.resize_keep_pt", "Keep pt", "canvas.resize:KeepTypographyPt", resize_mode == CanvasResizeMode::KeepTypographyPt),
            ("inspector.canvas.resize_scale", "Scale", "canvas.resize:ProportionalScale", resize_mode == CanvasResizeMode::ProportionalScale),
        ],
        has_canvas_target,
    );

    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let Some(ls) = ls else {
        ctx.paint_label(insp_text(app, "w6.view.insp.no_page_layout", "No Page Layout sheet in project yet."), ctx.cursor_y, 18, 10.5, muted, false);
        ctx.cursor_y += ctx.si(22);
        return;
    };

    let (rows, cols) = ls.spec.grid_template.unwrap_or((1, 2));
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.layout.grid", "Grid Arrangement")),
        &[
            ("inspector.layout.grid_1x1", "1x1", "layout.grid:1x1", (rows, cols) == (1, 1)),
            ("inspector.layout.grid_1x2", "1x2", "layout.grid:1x2", (rows, cols) == (1, 2)),
            ("inspector.layout.grid_2x1", "2x1", "layout.grid:2x1", (rows, cols) == (2, 1)),
            ("inspector.layout.grid_2x2", "2x2", "layout.grid:2x2", (rows, cols) == (2, 2)),
            ("inspector.layout.grid_2x3", "2x3", "layout.grid:2x3", (rows, cols) == (2, 3)),
        ],
        true,
    );

    let margin_mm = layout_margin_mm(state);
    ctx.cursor_y = ctx.emit_stepper_row(StepperArgs {
        y: ctx.cursor_y,
        title: insp_text(app, "inspector.layout.margins", "Page Margins"),
        value_display: &format!("{margin_mm} mm"),
        dec_id: "inspector.layout.margin_dec",
        dec_payload: "layout.margin:dec",
        inc_id: "inspector.layout.margin_inc",
        inc_payload: "layout.margin:inc",
        enabled: true,
    });

    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.layout.align_origins",
        ctx.cursor_y,
        insp_text(app, "inspector.layout.align_origins", "Align Plot Origins"),
        ls.spec.align_plot_origins,
        "layout.align_origins:toggle",
        true,
    );

    let sb_on = has_scale_bar_item(&ls.spec);
    ctx.cursor_y = ctx.emit_checkbox_row(
        "inspector.layout.scale_bar_toggle",
        ctx.cursor_y,
        insp_text(app, "inspector.layout.scale_bar", "Scale Bar"),
        sb_on,
        "layout.scale_bar:toggle",
        true,
    );

    let sb_mm = layout_scale_bar_mm(state);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        None,
        &[
            ("inspector.layout.sb_5", "5 mm", "layout.scale_bar_mm:5", sb_on && sb_mm == SCALE_BAR_STEPS_MM[0]),
            ("inspector.layout.sb_10", "10 mm", "layout.scale_bar_mm:10", sb_on && sb_mm == SCALE_BAR_STEPS_MM[1]),
            ("inspector.layout.sb_25", "25 mm", "layout.scale_bar_mm:25", sb_on && sb_mm == SCALE_BAR_STEPS_MM[2]),
            ("inspector.layout.sb_50", "50 mm", "layout.scale_bar_mm:50", sb_on && sb_mm == SCALE_BAR_STEPS_MM[3]),
        ],
        true,
    );
}

fn push_rect_op(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx, color: Rgba8, z_layer: ZLayer, clip: Option<ClipId>, owner: Option<WidgetId>) {
    builder.tree.paint_ops.push(PaintOp { z_layer, paint_order: 0, clip, owner_widget_id: owner, kind: PaintOpKind::FillRect { rect, color } });
}

fn push_border_op(
    builder: &mut LayoutBuilder<'_>,
    rect: LayoutRectPx,
    color: Rgba8,
    thickness: u32,
    z_layer: ZLayer,
    clip: Option<ClipId>,
    owner: Option<WidgetId>,
) {
    builder.tree.paint_ops.push(PaintOp {
        z_layer,
        paint_order: 0,
        clip,
        owner_widget_id: owner,
        kind: PaintOpKind::StrokeRect { rect, color, thickness },
    });
}

fn draw_popup_overlay(builder: &mut LayoutBuilder<'_>, popup: &PendingPopup, parent_panel: &WidgetId, vp: LayoutRectPx) {
    let scale = builder.scale();
    let surface = builder.app.theme.surface;
    let accent = builder.app.theme.accent;
    let item_h = insp_u32(24, scale);
    let pad = insp_i32(4, scale);
    let gap = insp_u32(2, scale);
    let visible_max = ((vp.height.saturating_sub(insp_u32(16, scale))) / item_h).max(7) as usize;
    let visible_n = popup.options.len().min(visible_max) as u32;
    let pop_w = popup.anchor.width.max(insp_u32(150, scale)).min(vp.width.saturating_sub(insp_u32(16, scale)).max(insp_u32(120, scale)));
    let pop_h = visible_n * item_h + (2 * pad) as u32;
    let min_x = vp.x + pad;
    let max_x = (vp.right() - (pop_w as i32) - pad).max(min_x);
    let pop_x = popup.anchor.x.clamp(min_x, max_x);
    let pop_y = if popup.anchor.bottom() + (gap as i32) + (pop_h as i32) <= vp.bottom() - pad {
        popup.anchor.bottom() + (gap as i32)
    } else {
        (popup.anchor.y - (pop_h as i32) - (gap as i32)).max(vp.y + pad)
    };
    let pop_rect = LayoutRectPx::new(pop_x, pop_y, pop_w, pop_h);
    let pop_id_str = format!("inspector.popup.{}", popup.kind);
    let pop_wid = wid(&pop_id_str);

    push_rect_op(builder, pop_rect, surface, ZLayer::DropdownMenu, None, Some(pop_wid.clone()));
    push_border_op(builder, pop_rect, accent, 1, ZLayer::DropdownMenu, None, Some(pop_wid.clone()));

    builder.push_node(NodeSpec {
        id: pop_wid.clone(),
        rect_px: pop_rect,
        parent: Some(parent_panel.clone()),
        z_layer: ZLayer::DropdownMenu,
        action: WidgetAction::NoOp,
        a11y: AccessibilityMeta {
            keyboard_path: Some(String::from("Inspector > Menu")),
            ..AccessibilityMeta::new(WidgetRole::ContextMenu, "Inspector menu", FocusScope::Workspace)
        },
        state: WidgetVisualState::default(),
    });

    for (idx, opt) in popup.options.iter().take(visible_max).enumerate() {
        let ir = LayoutRectPx::new(
            pop_x + pad,
            pop_y + pad + (idx as i32) * (item_h as i32),
            pop_w.saturating_sub((pad as u32) * 2),
            item_h.saturating_sub(gap),
        );
        let item_id = format!("inspector.popup.{}.item_{idx}", popup.kind);
        push_btn_widget(
            builder,
            BtnArgs {
                id: &item_id,
                rect: ir,
                label: &opt.label,
                a11y_label: &opt.label,
                action: WidgetAction::Custom { domain: String::from("inspector"), payload: opt.payload.clone() },
                parent: Some(pop_wid.clone()),
                z_layer: ZLayer::DropdownMenu,
                selected: opt.active,
                enabled: true,
                as_tab: false,
            },
        );
    }
}

/// Build the docked right-hand inspector panel inside `rect`.
pub fn build_inspector_slot(builder: &mut LayoutBuilder<'_>, rect: LayoutRectPx) {
    let scale = builder.scale();
    let bg = builder.app.theme.bg;
    let border = builder.app.theme.border;
    let fg = builder.app.theme.text_primary;
    let muted = builder.app.theme.text_muted;
    let inspector_wid = wid("inspector.panel");
    push_rect_op(builder, rect, bg, ZLayer::Workspace, None, Some(inspector_wid.clone()));
    push_border_op(builder, rect, border, 1, ZLayer::Workspace, None, Some(inspector_wid.clone()));

    builder.push_node(NodeSpec {
        id: inspector_wid.clone(),
        rect_px: rect,
        parent: None,
        z_layer: ZLayer::Workspace,
        action: WidgetAction::NoOp,
        a11y: AccessibilityMeta::new(WidgetRole::Label, insp_text(builder.app, "w6.view.insp.panel_title", "Inspector"), FocusScope::Workspace),
        state: WidgetVisualState::default(),
    });

    let app = builder.app;
    let pad = insp_i32(8, scale);
    let inner_w = rect.width.saturating_sub((pad as u32) * 2).max(insp_u32(40, scale));
    let header_rect = LayoutRectPx::new(rect.x + pad, rect.y + pad, inner_w, insp_u32(20, scale));
    let title_str = insp_text(app, "inspector.live_subtitle", "Live engine controls (undoable)");
    let active_insp = app.canvas_dialogs.active_inspector;
    let (hdr_key, hdr_fb) = match active_insp {
        ActiveInspector::FormatGraph => ("inspector.format_graph", active_insp.label()),
        ActiveInspector::FormatAxes => ("inspector.format_axes", active_insp.label()),
        ActiveInspector::BubblePlot => ("inspector.section.multivariate", active_insp.label()),
        ActiveInspector::PairwiseComparisons => ("inspector.section.pairwise_cld", active_insp.label()),
        ActiveInspector::PageLayout => ("inspector.page_layout", active_insp.label()),
        ActiveInspector::ResultDetails => ("w6.view.insp.analysis_checklist", active_insp.label()),
        ActiveInspector::None => ("w6.view.insp.panel_title", "Inspector"),
    };
    let header_title = String::from(insp_text(app, hdr_key, hdr_fb));
    let hdr_style = UiTextStyle::ui_bold(insp_pt(12.5, scale));
    paint_text_fitted(
        builder,
        &inspector_wid,
        FittedTextSpec {
            rect: header_rect,
            text: &header_title,
            style: &hdr_style,
            pad_x: 0,
            color: fg,
            z_layer: ZLayer::Workspace,
            scroll_container: None,
        },
    );

    let sub_rect = LayoutRectPx::new(rect.x + pad, header_rect.bottom() + insp_i32(2, scale), inner_w, insp_u32(15, scale));
    let sub_style = UiTextStyle::ui_sans(insp_pt(10.0, scale));
    paint_text_fitted(
        builder,
        &inspector_wid,
        FittedTextSpec {
            rect: sub_rect,
            text: title_str,
            style: &sub_style,
            pad_x: 0,
            color: muted,
            z_layer: ZLayer::Workspace,
            scroll_container: None,
        },
    );

    let active_sec = section_for_active_inspector(app, &app.view_shards.inspector);
    let tab_defs = [
        ("inspector.tab.graph", insp_text(app, "inspector.tab.graph", "Graph"), "tab:graph", active_sec == SectionKind::FormatGraph),
        ("inspector.tab.axes", insp_text(app, "inspector.tab.axes", "Axes"), "tab:axes", active_sec == SectionKind::FormatAxes),
        ("inspector.tab.multi", insp_text(app, "inspector.tab.multi", "Multi"), "tab:multi", active_sec == SectionKind::Multivariable),
        ("inspector.tab.pairwise", insp_text(app, "inspector.tab.pairwise", "P-Val"), "tab:pairwise", active_sec == SectionKind::PairwiseCld),
        ("inspector.tab.layout", insp_text(app, "inspector.tab.layout", "Layout"), "tab:layout", active_sec == SectionKind::PageLayout),
    ];

    let is_rtl = builder.tree.is_rtl;
    let tab_top = sub_rect.bottom() + insp_i32(6, scale);
    let tab_gap = insp_u32(3, scale);
    let two_rows = rect.width < insp_u32(236, scale);
    let tab_h = insp_u32(24, scale);
    let min_tab_w = insp_u32(30, scale);
    let tabs_bottom = if two_rows {
        let w1 = (inner_w.saturating_sub(2 * tab_gap) / 3).max(min_tab_w);
        for (i, (id, lbl, payload, selected)) in tab_defs[0..3].iter().enumerate() {
            let tx = rtl_row_x(rect.x + pad, inner_w, (i as u32 * (w1 + tab_gap)) as i32, w1, is_rtl);
            let tr_rect = LayoutRectPx::new(tx, tab_top, w1, tab_h);
            push_btn_widget(
                builder,
                BtnArgs {
                    id,
                    rect: tr_rect,
                    label: lbl,
                    a11y_label: lbl,
                    action: WidgetAction::Custom { domain: String::from("inspector"), payload: String::from(*payload) },
                    parent: Some(inspector_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    selected: *selected,
                    enabled: true,
                    as_tab: true,
                },
            );
        }
        let w2 = (inner_w.saturating_sub(tab_gap) / 2).max(min_tab_w);
        for (i, (id, lbl, payload, selected)) in tab_defs[3..5].iter().enumerate() {
            let tx = rtl_row_x(rect.x + pad, inner_w, (i as u32 * (w2 + tab_gap)) as i32, w2, is_rtl);
            let tr_rect = LayoutRectPx::new(tx, tab_top + (tab_h as i32) + (tab_gap as i32), w2, tab_h);
            push_btn_widget(
                builder,
                BtnArgs {
                    id,
                    rect: tr_rect,
                    label: lbl,
                    a11y_label: lbl,
                    action: WidgetAction::Custom { domain: String::from("inspector"), payload: String::from(*payload) },
                    parent: Some(inspector_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    selected: *selected,
                    enabled: true,
                    as_tab: true,
                },
            );
        }
        tab_top + (tab_h as i32) * 2 + (tab_gap as i32)
    } else {
        let tw = (inner_w.saturating_sub(4 * tab_gap) / 5).max(min_tab_w);
        for (i, (id, lbl, payload, selected)) in tab_defs.iter().enumerate() {
            let tx = rtl_row_x(rect.x + pad, inner_w, (i as u32 * (tw + tab_gap)) as i32, tw, is_rtl);
            let tr_rect = LayoutRectPx::new(tx, tab_top, tw, tab_h);
            push_btn_widget(
                builder,
                BtnArgs {
                    id,
                    rect: tr_rect,
                    label: lbl,
                    a11y_label: lbl,
                    action: WidgetAction::Custom { domain: String::from("inspector"), payload: String::from(*payload) },
                    parent: Some(inspector_wid.clone()),
                    z_layer: ZLayer::Workspace,
                    selected: *selected,
                    enabled: true,
                    as_tab: true,
                },
            );
        }
        tab_top + (tab_h as i32)
    };

    let sb_w = insp_u32(10, scale).max(crate::layout::scroll::scrollbar_thickness_px(scale));
    let scroll_top = tabs_bottom + insp_i32(6, scale);
    let scroll_h = rect.bottom().saturating_sub(scroll_top + pad).max(insp_i32(40, scale)) as u32;
    let scroll_w = rect.width.saturating_sub((pad as u32) * 2 + sb_w + 2).max(insp_u32(80, scale));
    let scroll_clip = LayoutRectPx::new(rect.x + pad, scroll_top, scroll_w, scroll_h);
    let scroll_off = builder.app.interaction.scroll_offsets.offsets.get(SCROLL_ID).copied().unwrap_or([0.0, 0.0]);
    let scroll_y = scroll_off[1].max(0.0) as i32;

    let state_snap = builder.app.view_shards.inspector.clone();
    let maybe_gs = resolve_graph_sheet(builder.app.document(), &state_snap).cloned();
    let maybe_ls = resolve_layout_sheet(builder.app.document(), &state_snap).cloned();

    let scroll_wid = wid(SCROLL_ID);
    let scroll_node_idx = builder.tree.nodes.len();
    builder.push_node(NodeSpec {
        id: scroll_wid.clone(),
        rect_px: scroll_clip,
        parent: Some(inspector_wid.clone()),
        z_layer: ZLayer::Workspace,
        action: WidgetAction::NoOp,
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Inspector scroll", FocusScope::Workspace),
        state: WidgetVisualState::default(),
    });
    let scroll_clip_id = builder.push_clip(scroll_clip);

    let mut ctx = SectionDrawCtx {
        builder,
        scroll_clip,
        scroll_clip_id,
        parent_wid: &scroll_wid,
        panel_x: scroll_clip.x,
        panel_w: scroll_clip.width,
        cursor_y: scroll_clip.y - scroll_y,
        pending_popup: None,
    };

    draw_drill_back_section(&mut ctx, maybe_gs.as_ref());
    draw_format_graph_section(&mut ctx, maybe_gs.as_ref());
    draw_format_axes_section(&mut ctx, maybe_gs.as_ref());
    draw_multivariable_section(&mut ctx, maybe_gs.as_ref());
    draw_pairwise_section(&mut ctx, maybe_gs.as_ref());
    draw_page_layout_section(&mut ctx, maybe_ls.as_ref());

    let content_h = ((ctx.cursor_y + scroll_y - scroll_clip.y) + 8).max(scroll_clip.height as i32) as u32;
    let pending_popup = ctx.pending_popup;

    builder.pop_clip();

    let max_scroll = (content_h as f64 - scroll_clip.height as f64).max(0.0);
    let meta = ScrollMeta {
        container_id: WidgetId(String::from(SCROLL_ID)),
        viewport_px: scroll_clip,
        content_size_px: [scroll_clip.width, content_h],
        scroll_offset_px: [0.0, scroll_y as f64],
        max_scroll_px: [0.0, max_scroll],
        axes: ScrollAxes::Vertical,
    };
    if let Some(node) = builder.tree.nodes.get_mut(scroll_node_idx) {
        node.scroll = Some(meta.clone());
    }
    emit_scrollbars_for_meta(builder, ZLayer::Workspace, &meta, Some(inspector_wid.clone()));

    if let Some(popup) = pending_popup {
        draw_popup_overlay(builder, &popup, &inspector_wid, rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewportMetrics;
    use crate::dialogs_canvas::ActiveDialog;
    use crate::human_driver::HumanPathDriver;
    use crate::interaction::KeyModifiers;
    use curvebook_doc::{Cell, CellCoord, DataSheet, GroupedTable, MultipleVariablesTable, SubcolumnMode, TableArchetype, VariableRole, XyTable};
    use curvebook_engine::Session;

    fn scroll_click(driver: &mut HumanPathDriver, id: &str) -> Result<(), String> {
        for _ in 0..16 {
            if driver.find_widget(id).is_some() {
                return driver.click_widget(id).map(|_| ());
            }
            driver.wheel_widget("scroll.inspector", 0.0, 90.0, KeyModifiers::default())?;
        }
        driver.click_widget(id).map(|_| ())
    }

    fn mv_driver() -> Result<(HumanPathDriver, SheetId), String> {
        let mut mv = MultipleVariablesTable::new(3, 4).map_err(|e| format!("{e}"))?;
        mv.variables[0].title = String::from("Group");
        mv.variables[0].role = VariableRole::Categorical;
        mv.variables[1].title = String::from("Dose");
        mv.variables[1].role = VariableRole::Continuous;
        mv.variables[2].title = String::from("Signal");
        mv.variables[2].role = VariableRole::Continuous;
        let ds = DataSheet::new("ds-mv", "MV", TableArchetype::MultipleVariables(mv)).map_err(|e| format!("{e}"))?;
        let mut doc = ProjectDocument::new("MV Test").map_err(|e| format!("{e}"))?;
        doc.add_data_sheet(ds).map_err(|e| format!("{e}"))?;
        let mut session = Session::from_document(doc);
        session
            .execute("graph.create", json!({ "sheet_id": "gr-mv", "data_sheet_id": "ds-mv", "kind": "multiple_variables" }))
            .map_err(|e| format!("{e}"))?;
        session.execute("sheet.select", json!({ "sheet_id": "gr-mv" })).map_err(|e| format!("{e}"))?;
        let mut app = UiApp::from_session(session);
        app.nav.inspector_open = true;
        Ok((HumanPathDriver::new(app, ViewportMetrics::new(1280.0, 800.0, 1.0)), SheetId::new("gr-mv")))
    }

    #[test]
    fn inspector_w6_controls_and_rtl_tabs_behave_cleanly() -> Result<(), String> {
        let (mut driver, gr_mv) = mv_driver()?;
        driver.click_widget("inspector.tab.multi")?;
        driver.click_widget("inspector.mv.mode_cat_summary")?;
        driver.click_widget("inspector.mv.metric_median")?;
        let gs = driver.app().document().graph_sheet(&gr_mv).ok_or("missing gr-mv")?;
        let enc = gs.spec.multivariate.as_ref().ok_or("missing multivariate")?;
        assert!(matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        assert_eq!(mv_summary_measure(enc), Some(SummaryMeasure::Median));

        driver.click_widget("inspector.mv.color_clamp_min.input")?;
        driver.type_text("0")?;
        driver.click_widget("inspector.mv.color_clamp_max.input")?;
        driver.type_text("100")?;
        driver.click_widget("inspector.mv.color_clamp_apply")?;
        let gs = driver.app().document().graph_sheet(&gr_mv).ok_or("missing gr-mv")?;
        assert!(matches!(
            gs.spec.multivariate.as_ref().and_then(|m| match &m.color {
                ColorChannelMapping::Continuous { domain_clamp, .. } => *domain_clamp,
                _ => None,
            }),
            Some((lo, hi)) if (lo - 0.0).abs() < 1e-9 && (hi - 100.0).abs() < 1e-9
        ));

        // Canvas preset + resize works cleanly even when no LayoutSheet exists yet.
        driver.click_widget("inspector.tab.layout")?;
        scroll_click(&mut driver, "inspector.canvas.preset_single_88mm")?;
        scroll_click(&mut driver, "inspector.canvas.resize_keep_pt")?;
        assert_eq!(driver.app().canvas_dialogs.active_canvas_preset, Some(FigureCanvasPreset::SingleColumn88mm));
        assert_eq!(driver.app().canvas_dialogs.canvas_resize_mode, CanvasResizeMode::KeepTypographyPt);

        // CVD filter + Apply Style From in Format Graph + RTL tab mirroring.
        driver.click_widget("inspector.tab.graph")?;
        scroll_click(&mut driver, "inspector.cvd.deuteranopia")?;
        assert_eq!(driver.app().preview_filter, CanvasPreviewFilter::Deuteranopia);
        scroll_click(&mut driver, "inspector.fg.apply_style_from")?;
        assert!(matches!(driver.app().canvas_dialogs.active_dialog, ActiveDialog::ApplyStyleFrom { .. }));
        driver.click_widget("modal.cancel")?;

        driver.click_widget("inspector.tab.axes")?;
        driver.click_widget("inspector.axes.title.input")?;
        driver.type_text("Concentration")?;
        driver.click_widget("inspector.axes.title_apply")?;
        let gs = driver.app().document().graph_sheet(&gr_mv).ok_or("missing gr-mv")?;
        assert!(gs.spec.y_axis.title.contains("Concentration"));

        driver.app_mut().locale = String::from("ar");
        let rtl_tree = driver.layout_tree();
        assert!(rtl_tree.is_rtl);
        let tg = rtl_tree.node("inspector.tab.graph").ok_or("missing tab.graph")?;
        let tl = rtl_tree.node("inspector.tab.layout").ok_or("missing tab.layout")?;
        assert!(tg.bounds.rect_px.x > tl.bounds.rect_px.x);
        Ok(())
    }

    #[test]
    fn grouped_arrangement_and_regression_overlay_dispatch_cleanly() -> Result<(), String> {
        let mut grp = GroupedTable::new(SubcolumnMode::RawReplicates(2), 2, 2).map_err(|e| format!("{e}"))?;
        grp.data_sets[0].title = String::from("A");
        grp.data_sets[1].title = String::from("B");
        let ds_g = DataSheet::new("ds-g", "Grp", TableArchetype::Grouped(grp)).map_err(|e| format!("{e}"))?;

        let xy = XyTable::new(SubcolumnMode::Single, 1, 4).map_err(|e| format!("{e}"))?;
        let mut ds_xy = DataSheet::new("ds-xy", "XY", TableArchetype::Xy(xy)).map_err(|e| format!("{e}"))?;
        for r in 0..4 {
            let x = (r + 1) as f64;
            let y = 1.0 + 2.0 * x;
            ds_xy.set_x_or_time_cell(r, 0, Cell::number(x)).map_err(|e| format!("{e}"))?;
            ds_xy.set_cell(CellCoord::new(r, 0, 0), Cell::number(y)).map_err(|e| format!("{e}"))?;
        }
        let mut doc = ProjectDocument::new("GrpReg").map_err(|e| format!("{e}"))?;
        doc.add_data_sheet(ds_g).map_err(|e| format!("{e}"))?;
        doc.add_data_sheet(ds_xy).map_err(|e| format!("{e}"))?;

        let mut session = Session::from_document(doc);
        session
            .execute("analysis.run", json!({ "sheet_id": "res-lin", "source_sheet": "ds-xy", "kind": "linear_regression" }))
            .map_err(|e| format!("{e}"))?;
        session.execute("graph.create", json!({ "sheet_id": "gr-grp", "data_sheet_id": "ds-g", "kind": "grouped" })).map_err(|e| format!("{e}"))?;
        session.execute("sheet.select", json!({ "sheet_id": "gr-grp" })).map_err(|e| format!("{e}"))?;
        let mut app = UiApp::from_session(session);
        app.nav.inspector_open = true;
        let mut driver = HumanPathDriver::new(app, ViewportMetrics::new(1280.0, 800.0, 1.0));

        scroll_click(&mut driver, "inspector.fg.arr_superimposed")?;
        assert_eq!(
            driver.app().document().graph_sheet(&SheetId::new("gr-grp")).map(|g| g.spec.grouped_arrangement),
            Some(GroupedArrangement::Superimposed)
        );

        scroll_click(&mut driver, "inspector.regression.result_sheet")?;
        scroll_click(&mut driver, "inspector.regression.all_series")?;
        scroll_click(&mut driver, "inspector.regression.confidence_band")?;
        scroll_click(&mut driver, "inspector.regression.shaded_ribbon")?;
        scroll_click(&mut driver, "inspector.regression.apply")?;
        let g = driver.app().document().graph_sheet(&SheetId::new("gr-grp")).ok_or("missing gr-grp")?;
        assert!(g.spec.series.iter().all(|s| s.regression_overlay.as_ref().is_some_and(|o| o.show_confidence_band && o.shaded_ribbon)));
        Ok(())
    }
}
