//! Live right-hand inspector view builder (`R10.18`).
//!
//! Renders 5 responsive sub-tabs (`Format Graph`, `Format Axes`, `Multivariable Channels`,
//! `Pairwise & CLD`, `Page Layout`) and interactive `Point & Drill-Back` controls,
//! dispatching instant undoable engine commands and reflecting live state on undo/redo.

#[path = "../i18n/w5_inspector.rs"]
mod w5_i18n;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use curvebook_chart::{RectPt, SourceCellId};
use curvebook_doc::{
    AnalysisKind, AxisScale, AxisSegment, AxisSpec, BracketLabelMode, BreakOrnament, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec,
    ConnectingLineStyle, ErrorBarDirection, ErrorBarMetric, GraphSheet, GraphicalDataObject, LayoutItem, LayoutSheet, LayoutSpec, LineConnectionKind,
    MultivariateGraphEncoding, MultivariatePlotMode, PairwiseDisplayMode, PhysicalUnit, ProjectDocument, ReplicateOverlayStyle, SeriesSpec, SheetId,
    SizeChannelMapping, SummaryMeasure, WhiskerMethod,
};
use curvebook_math::LogitDomain;
use curvebook_palette::{ColorRamp, ColormapId, LineDashStyle, MarkerShape, MarkerStyle, PaletteId, Rgba8};
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

impl<'a, 'b> SectionDrawCtx<'a, 'b> {
    fn is_row_visible(&self, rect: LayoutRectPx) -> bool {
        rect.height > 0 && rect.width > 0 && rect.y >= self.scroll_clip.y && rect.bottom() <= self.scroll_clip.bottom()
    }

    fn paint_label(&mut self, text: &str, y: i32, h: u32, size_px: f64, color: Rgba8, bold: bool) {
        let rect = LayoutRectPx::new(self.panel_x, y, self.panel_w, h);
        if !self.is_row_visible(rect) {
            return;
        }
        let style = if bold { UiTextStyle::ui_bold(size_px) } else { UiTextStyle::ui_sans(size_px) };
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
        let rect = LayoutRectPx::new(self.panel_x, y, self.panel_w, 24);
        if self.is_row_visible(rect) {
            let box_text = if checked { format!("[x] {label}") } else { format!("[ ] {label}") };
            self.emit_action_btn(id, rect, (&box_text, label), payload, checked, enabled);
        }
        y + 28
    }

    fn emit_dropdown_trigger(&mut self, args: DropdownArgs<'_>) -> i32 {
        let btn_rect = LayoutRectPx::new(self.panel_x, args.y + 16, self.panel_w, 24);
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
        args.y + 44
    }

    fn emit_chip_row(&mut self, y: i32, title: Option<&str>, items: &[(&str, &str, &str, bool)], enabled: bool) -> i32 {
        let mut cy = y;
        if let Some(t) = title {
            let muted = self.builder.app.theme.text_muted;
            self.paint_label(t, cy, 15, 10.5, muted, false);
            cy += 16;
        }
        let n = items.len().max(1);
        let max_per_row = if self.panel_w < 210 {
            2
        } else if n > 4 {
            3
        } else {
            n
        };
        let gap = 4u32;
        let rows = n.div_ceil(max_per_row);
        for row_idx in 0..rows {
            let start = row_idx * max_per_row;
            let end = (start + max_per_row).min(items.len());
            let slice = &items[start..end];
            let cols = slice.len().max(1) as u32;
            let total_gaps = gap * (cols.saturating_sub(1));
            let btn_w = (self.panel_w.saturating_sub(total_gaps) / cols).max(24);
            for (idx, (id, lbl, payload, active)) in slice.iter().enumerate() {
                let bx = self.panel_x + ((idx as u32) * (btn_w + gap)) as i32;
                let rect = LayoutRectPx::new(bx, cy, btn_w, 22);
                self.emit_action_btn(id, rect, (lbl, lbl), payload, *active, enabled);
            }
            cy += 26;
        }
        cy
    }

    fn emit_stepper_row(&mut self, args: StepperArgs<'_>) -> i32 {
        let fg = self.builder.app.theme.text_primary;
        let lbl_w = self.panel_w.saturating_sub(62).max(60);
        let lbl_rect = LayoutRectPx::new(self.panel_x, args.y + 3, lbl_w, 18);
        if self.is_row_visible(lbl_rect) {
            let txt = format!("{}: {}", args.title, args.value_display);
            let txt_style = UiTextStyle::ui_sans(10.5);
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
        let right_x = self.panel_x + (self.panel_w as i32);
        let dec_rect = LayoutRectPx::new(right_x - 58, args.y, 27, 22);
        let inc_rect = LayoutRectPx::new(right_x - 27, args.y, 27, 22);
        self.emit_action_btn(args.dec_id, dec_rect, ("-", &format!("Decrease {}", args.title)), args.dec_payload, false, args.enabled);
        self.emit_action_btn(args.inc_id, inc_rect, ("+", &format!("Increase {}", args.title)), args.inc_payload, false, args.enabled);
        args.y + 26
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
        app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": spec })).map_err(|e| e.to_string())?;
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

    Ok(UiEventOutcome::default())
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
        state.open_popup = None;
        return Ok(outcome_ok(sub, None));
    }

    let which = current_axis_which(app);
    let mut axis = read_live_axis_spec(&gs, which);
    let mut seg = axis_primary_seg(&axis);

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

fn dispatch_multivariable_action(app: &mut UiApp, state: &mut InspectorViewState, sub: &str) -> Result<UiEventOutcome, String> {
    let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() else {
        return Ok(UiEventOutcome::default());
    };
    state.target_graph_id = Some(gs.id.as_str().to_string());
    let mut enc = current_mv_encoding(app.document(), &gs);

    if let Some(raw) = sub.strip_prefix("x:") {
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
            let mut spec = gs.spec.clone();
            spec.comparisons.clear();
            app.execute_command("graph.setSpec", json!({ "sheet_id": gs.id.as_str(), "spec": spec })).map_err(|e| e.to_string())?;
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
                if let Some(gs) = resolve_graph_sheet(app.document(), state).cloned() {
                    state.target_graph_id = Some(gs.id.as_str().to_string());
                    if next_insp == ActiveInspector::FormatGraph {
                        app.canvas_dialogs.format_graph.sync_from_graph(&gs);
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
    if let Some(sub) = payload.strip_prefix("drill.") {
        return dispatch_drill_action(app, state, sub);
    }

    Ok(UiEventOutcome::default())
}

fn draw_section_header(ctx: &mut SectionDrawCtx<'_, '_>, id: &str, title: &str, sec: SectionKind, expanded: bool) {
    let rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, 26);
    if ctx.is_row_visible(rect) {
        let prefix = if expanded { "[-] " } else { "[+] " };
        let label = format!("{prefix}{title}");
        ctx.emit_action_btn(id, rect, (&label, title), &format!("section:{}", sec.id()), expanded, true);
    }
    ctx.cursor_y += 30;
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
        None => String::from("No plotted points on active sheet"),
    };
    ctx.paint_label(&summary, ctx.cursor_y, 16, 10.5, muted, false);
    ctx.cursor_y += 18;

    let half = (ctx.panel_w.saturating_sub(4) / 2).max(40);
    let prev_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, half, 22);
    let next_rect = LayoutRectPx::new(ctx.panel_x + (half as i32) + 4, ctx.cursor_y, half, 22);
    let prev_lbl = insp_text(app, "inspector.drill_back.prev_pt", "Prev Pt");
    let next_lbl = insp_text(app, "inspector.drill_back.next_pt", "Next Pt");
    ctx.emit_action_btn("inspector.drill.prev", prev_rect, (prev_lbl, prev_lbl), "drill.point:prev", false, has_pts);
    ctx.emit_action_btn("inspector.drill.next", next_rect, (next_lbl, next_lbl), "drill.point:next", false, has_pts);
    ctx.cursor_y += 26;

    let on_data_sheet = app.document().active_sheet_id.as_ref().is_some_and(|id| app.document().data_sheet(id).is_some());
    let go_lbl = insp_text(app, "inspector.drill_back.go", "Go to Data Cell");
    let go_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, 24);
    ctx.emit_action_btn("inspector.drill.go_to_cell", go_rect, (go_lbl, "Go to Data Cell / Drill Back"), "drill.go_to_cell", false, has_target);
    ctx.cursor_y += 28;

    if on_data_sheet && gs.is_some() {
        let back_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, ctx.panel_w, 22);
        ctx.emit_action_btn("inspector.drill.return_graph", back_rect, ("Return to Graph", "Return to Graph"), "drill.return_to_graph", false, true);
        ctx.cursor_y += 26;
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
        ctx.cursor_y += 22;
        return;
    };

    let sel_idx = resolve_selected_series_index(app, gs);
    let cur_series = gs.spec.series.get(sel_idx);
    let ds_label = cur_series.and_then(|s| s.legend_label.as_deref()).unwrap_or("Series");
    let has_series = !gs.spec.series.is_empty();

    let ds_title = insp_text(app, "inspector.fg.dataset", "Dataset / Series");
    ctx.paint_label(ds_title, ctx.cursor_y, 15, 10.5, muted, false);
    ctx.cursor_y += 16;
    let nav_w = 24u32;
    let mid_w = ctx.panel_w.saturating_sub(2 * nav_w + 8).max(50);
    let prev_r = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, nav_w, 24);
    let mid_r = LayoutRectPx::new(ctx.panel_x + (nav_w as i32) + 4, ctx.cursor_y, mid_w, 24);
    let next_r = LayoutRectPx::new(ctx.panel_x + (nav_w as i32) + 4 + (mid_w as i32) + 4, ctx.cursor_y, nav_w, 24);
    ctx.emit_action_btn("inspector.fg.dataset_prev", prev_r, ("<", "Previous series"), "fg.dataset:prev", false, has_series);
    ctx.emit_action_btn(
        "inspector.fg.dataset_btn",
        mid_r,
        (ds_label, ds_title),
        "popup:dataset",
        state.open_popup.as_deref() == Some("dataset"),
        has_series,
    );
    ctx.emit_action_btn("inspector.fg.dataset_next", next_r, (">", "Next series"), "fg.dataset:next", false, has_series);
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
    ctx.cursor_y += 28;

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
    ctx.cursor_y += 16;
    let cur_hex = cur_series.map(|s| rgba_to_hex(series_primary_color(s))).unwrap_or_default();
    let sw_gap = 4u32;
    let sw_cols = 4u32;
    let sw_w = (ctx.panel_w.saturating_sub(sw_gap * (sw_cols - 1)) / sw_cols).max(24);
    for (idx, (cname, hex)) in COLOR_SWATCHES.iter().enumerate() {
        let r = (idx as u32) / sw_cols;
        let c = (idx as u32) % sw_cols;
        let sw_rect = LayoutRectPx::new(ctx.panel_x + (c * (sw_w + sw_gap)) as i32, ctx.cursor_y + (r * 24) as i32, sw_w, 20);
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
    ctx.cursor_y += 50;

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
    let label_w = if ctx.panel_w < 210 { 52u32 } else { 68u32 };
    let stepper_w = 56u32;
    let field_w = ctx.panel_w.saturating_sub(label_w + stepper_w + 10).max(44);

    let lbl_rect = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y + 3, label_w, 18);
    if ctx.is_row_visible(lbl_rect) {
        let lbl_style = UiTextStyle::ui_sans(10.5);
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

    let field_rect = LayoutRectPx::new(ctx.panel_x + (label_w as i32) + 4, ctx.cursor_y, field_w, 22);
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

    let dec_x = ctx.panel_x + (label_w as i32) + 4 + (field_w as i32) + 4;
    let dec_r = LayoutRectPx::new(dec_x, ctx.cursor_y, 25, 22);
    let inc_r = LayoutRectPx::new(dec_x + 27, ctx.cursor_y, 25, 22);
    ctx.emit_action_btn(args.dec_id, dec_r, ("-", &format!("Decrease {}", args.label)), args.dec_payload, false, true);
    ctx.emit_action_btn(args.inc_id, inc_r, ("+", &format!("Increase {}", args.label)), args.inc_payload, false, true);

    ctx.cursor_y += 26;
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
        ctx.cursor_y += 22;
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
            "Enable Right Y2 Axis",
            gs.spec.y2_axis.is_some(),
            "axes.y2_toggle",
            true,
        );
    }

    let axis = read_live_axis_spec(gs, which);
    let seg = axis_primary_seg(&axis);
    let scale_options = vec![
        PopupOptionSpec { label: String::from("Linear"), payload: String::from("axes.scale:Linear"), active: seg.scale == AxisScale::Linear },
        PopupOptionSpec { label: String::from("Log10"), payload: String::from("axes.scale:Log10"), active: seg.scale == AxisScale::Log10 },
        PopupOptionSpec { label: String::from("Log2"), payload: String::from("axes.scale:Log2"), active: seg.scale == AxisScale::Log2 },
        PopupOptionSpec { label: String::from("Ln (Natural)"), payload: String::from("axes.scale:Ln"), active: seg.scale == AxisScale::Ln },
        PopupOptionSpec { label: String::from("Logit"), payload: String::from("axes.scale:Logit"), active: matches!(seg.scale, AxisScale::Logit(_)) },
        PopupOptionSpec { label: String::from("Probit"), payload: String::from("axes.scale:Probit"), active: seg.scale == AxisScale::Probit },
        PopupOptionSpec {
            label: String::from("Reciprocal 1/Y"),
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
            ("inspector.axes.scale_linear", "Linear", "axes.scale:Linear", seg.scale == AxisScale::Linear),
            ("inspector.axes.scale_log10", "Log10", "axes.scale:Log10", seg.scale == AxisScale::Log10),
            ("inspector.axes.scale_log2", "Log2", "axes.scale:Log2", seg.scale == AxisScale::Log2),
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

    let half = (ctx.panel_w.saturating_sub(4) / 2).max(40);
    let apply_r = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, half, 22);
    let auto_r = LayoutRectPx::new(ctx.panel_x + (half as i32) + 4, ctx.cursor_y, half, 22);
    ctx.emit_action_btn("inspector.axes.apply_range", apply_r, ("Apply Range", "Apply Range"), "axes.apply_range", false, true);
    ctx.emit_action_btn("inspector.axes.auto_range", auto_r, ("Auto Range", "Auto Range"), "axes.auto_range", false, true);
    ctx.cursor_y += 26;

    let seg_cnt = axis.segments.len().max(1);
    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.axes.breaks", "Axis Breaks")),
        &[
            ("inspector.axes.breaks_1", "1 (None)", "axes.breaks:1", seg_cnt <= 1),
            ("inspector.axes.breaks_2", "2 Segs", "axes.breaks:2", seg_cnt == 2),
            ("inspector.axes.breaks_3", "3 Segs", "axes.breaks:3", seg_cnt >= 3),
        ],
        true,
    );

    ctx.cursor_y = ctx.emit_chip_row(
        ctx.cursor_y,
        Some(insp_text(app, "inspector.axes.ornament", "Break Style")),
        &[
            ("inspector.axes.orn_double", "Slash", "axes.ornament:DoubleSlash", axis.break_ornament == BreakOrnament::DoubleSlash),
            ("inspector.axes.orn_zigzag", "Zigzag", "axes.ornament:Zigzag", axis.break_ornament == BreakOrnament::Zigzag),
            ("inspector.axes.orn_gap", "Gap", "axes.ornament:BlankGap", axis.break_ornament == BreakOrnament::BlankGap),
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
        ctx.cursor_y += 22;
        return;
    };

    let cols = source_columns_for_graph(app.document(), gs);
    let has_cols = !cols.is_empty();
    let enc = current_mv_encoding(app.document(), gs);
    let col_name = |c: usize| -> String { cols.iter().find(|(idx, _)| *idx == c).map_or_else(|| format!("Col {}", c + 1), |(_, t)| t.clone()) };

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
}

fn cld_mode_label(mode: PairwiseDisplayMode) -> &'static str {
    match mode {
        PairwiseDisplayMode::BracketsOnly => "Brackets Only",
        PairwiseDisplayMode::CompactLetterDisplayOnly => "CLD Letters",
        PairwiseDisplayMode::BracketsAndCompactLetterDisplay => "Brackets + CLD",
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
        ctx.cursor_y += 22;
        return;
    };

    let maybe_rid = find_pairwise_result_sheet_id(app.document(), gs);
    let has_result = maybe_rid.is_some();
    let half = (ctx.panel_w.saturating_sub(4) / 2).max(40);
    let auto_r = LayoutRectPx::new(ctx.panel_x, ctx.cursor_y, half, 24);
    let clr_r = LayoutRectPx::new(ctx.panel_x + (half as i32) + 4, ctx.cursor_y, half, 24);
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
    ctx.cursor_y += 28;

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
            label: String::from("Brackets Only"),
            payload: String::from("pw.cld_mode:BracketsOnly"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsOnly,
        },
        PopupOptionSpec {
            label: String::from("CLD Letters"),
            payload: String::from("pw.cld_mode:CompactLetterDisplayOnly"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::CompactLetterDisplayOnly,
        },
        PopupOptionSpec {
            label: String::from("Brackets + CLD"),
            payload: String::from("pw.cld_mode:BracketsAndCompactLetterDisplay"),
            active: gs.spec.pairwise_display_mode == PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
        },
    ];
    ctx.cursor_y = ctx.emit_dropdown_trigger(DropdownArgs {
        id: "inspector.pw.cld_mode_btn",
        y: ctx.cursor_y,
        title: "Pairwise Mode",
        current_val: cld_mode_label(gs.spec.pairwise_display_mode),
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

    let info_line = format!("Brackets: {} · CLD: {}", gs.spec.comparisons.len(), if cld_enabled { "On" } else { "Off" });
    ctx.paint_label(&info_line, ctx.cursor_y, 16, 10.5, muted, false);
    ctx.cursor_y += 20;
}

fn draw_page_layout_section(ctx: &mut SectionDrawCtx<'_, '_>, ls: Option<&LayoutSheet>) {
    let app = ctx.builder.app;
    let muted = app.theme.text_muted;
    let state = &app.view_shards.inspector;
    let expanded = is_section_expanded(app, state, SectionKind::PageLayout);
    let title = insp_text(app, "inspector.page_layout", "Page Layout");
    draw_section_header(ctx, "inspector.section.page_layout", title, SectionKind::PageLayout, expanded);
    if !expanded {
        return;
    }

    let Some(ls) = ls else {
        ctx.paint_label("No Page Layout sheet in project yet.", ctx.cursor_y, 18, 10.5, muted, false);
        ctx.cursor_y += 22;
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
    let surface = builder.app.theme.surface;
    let accent = builder.app.theme.accent;
    let item_h = 24u32;
    let pad = 4i32;
    let visible_n = popup.options.len().min(7) as u32;
    let pop_w = popup.anchor.width.max(150).min(vp.width.saturating_sub(16).max(120));
    let pop_h = visible_n * item_h + (2 * pad) as u32;
    let min_x = vp.x + 4;
    let max_x = (vp.right() - (pop_w as i32) - 4).max(min_x);
    let pop_x = popup.anchor.x.clamp(min_x, max_x);
    let pop_y = if popup.anchor.bottom() + 2 + (pop_h as i32) <= vp.bottom() - 4 {
        popup.anchor.bottom() + 2
    } else {
        (popup.anchor.y - (pop_h as i32) - 2).max(vp.y + 4)
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

    for (idx, opt) in popup.options.iter().take(7).enumerate() {
        let ir = LayoutRectPx::new(pop_x + pad, pop_y + pad + (idx as i32) * (item_h as i32), pop_w.saturating_sub(8), item_h - 2);
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
        a11y: AccessibilityMeta::new(WidgetRole::Label, "Inspector", FocusScope::Workspace),
        state: WidgetVisualState::default(),
    });

    let app = builder.app;
    let pad = 8i32;
    let inner_w = rect.width.saturating_sub(16).max(40);
    let header_rect = LayoutRectPx::new(rect.x + pad, rect.y + pad, inner_w, 20);
    let title_str = insp_text(app, "inspector.live_subtitle", "Live engine controls (undoable)");
    let active_insp = app.canvas_dialogs.active_inspector;
    let header_title = if active_insp != ActiveInspector::None { String::from(active_insp.label()) } else { String::from("Inspector") };
    let hdr_style = UiTextStyle::ui_bold(12.5);
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

    let sub_rect = LayoutRectPx::new(rect.x + pad, header_rect.bottom() + 2, inner_w, 15);
    let sub_style = UiTextStyle::ui_sans(10.0);
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

    let tab_top = sub_rect.bottom() + 6;
    let tab_gap = 3u32;
    let two_rows = rect.width < 236;
    let tab_h = 24u32;
    let tabs_bottom = if two_rows {
        let w1 = (inner_w.saturating_sub(2 * tab_gap) / 3).max(32);
        for (i, (id, lbl, payload, selected)) in tab_defs[0..3].iter().enumerate() {
            let tr_rect = LayoutRectPx::new(rect.x + pad + (i as u32 * (w1 + tab_gap)) as i32, tab_top, w1, tab_h);
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
        let w2 = (inner_w.saturating_sub(tab_gap) / 2).max(32);
        for (i, (id, lbl, payload, selected)) in tab_defs[3..5].iter().enumerate() {
            let tr_rect = LayoutRectPx::new(rect.x + pad + (i as u32 * (w2 + tab_gap)) as i32, tab_top + (tab_h as i32) + 3, w2, tab_h);
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
        tab_top + (tab_h as i32) * 2 + 3
    } else {
        let tw = (inner_w.saturating_sub(4 * tab_gap) / 5).max(30);
        for (i, (id, lbl, payload, selected)) in tab_defs.iter().enumerate() {
            let tr_rect = LayoutRectPx::new(rect.x + pad + (i as u32 * (tw + tab_gap)) as i32, tab_top, tw, tab_h);
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

    let sb_w = 10u32;
    let scroll_top = tabs_bottom + 6;
    let scroll_h = rect.bottom().saturating_sub(scroll_top + pad).max(40) as u32;
    let scroll_w = rect.width.saturating_sub(16 + sb_w + 2).max(80);
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
        action: WidgetAction::FocusWidget(String::from(SCROLL_ID)),
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
