//! Graph formatting & configuration modals in `egui 0.37` (`R15.4`, `gui-layout-spec.md`
//! §8.11B–D): `Change Graph Type`, 3-schema `Format Graph`, `Format Axes` (+ `Categorical Axis
//! Guard` and `Additional Ticks / Thresholds` sub-modal), `Pairwise Comparisons`, `Bubble Plot
//! Channels`, `Apply Style From`, `Format Selected Points`, `Insert Model Equation`, and
//! `Page Layout Composer`.

use curvebook_chart::{ChartScene, PathCmd, PointPt, ScenePrimitive, compile_graph};
use curvebook_doc::{
    AdditionalTick, AxisThreshold, BracketLabelMode, BracketStyle, BreakOrnament, CategoricalTickSource, CldOrdering, ColorChannelMapping,
    ColumnDesignHint, DiagnosticPlotKind, EquationDisplayMode, ErrorBarDirection, ErrorBarMetric, FramePreset, GraphAxisScale, GraphKind, GraphSheet,
    GroupedArrangement, JournalWidthPreset, LayoutItem, LogitDomain, MultivariateGraphEncoding, PageOrientation, PairwiseDisplayMode,
    PanelBadgeStyle, ProjectDocument, RectPt, SheetId, SizeChannelMapping, SummaryMeasure, TableArchetype, TableKind, TickLabelFormat,
};
use curvebook_palette::{ColorRamp, ColormapId, CvdKind, LineDashStyle, MarkerFillMode, MarkerShape, PaletteId, Rgba8, simulate_cvd};
use egui::scroll_area::ScrollBarVisibility;
use egui::{Align2, Color32, Context, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use serde_json::{Map, Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AxisTarget, GRAPH_TYPE_OPTIONS, GraphTypeOption, ThumbnailGlyphKind};
use crate::i18n::{normalize_locale, tr};
use crate::layout::ZLayer;

use super::inspect_bridge::{ModalCaptureSpec, finalize_egui_modal_response};
use super::unit_spinners::{ScrubUnitFieldConfig, ScrubUnitFieldState, ScrubUnitKind, render_scrub_unit_field};
use super::{FrameInspectEntryInput, FrameInspectMeta, PendingUiIntents, Tokens};

/// Top-level tab selection inside 3-schema `Format Graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormatGraphTab {
    #[default]
    Appearance,
    SeriesAndArrangement,
    GraphSettings,
    ValueAnnotations,
    AxisVariables,
    DataObjects,
    Legends,
    ColorMapping,
    HeatmapLabels,
    HeatmapBorders,
}

impl FormatGraphTab {
    #[must_use]
    const fn widget_id(self) -> &'static str {
        match self {
            Self::Appearance => "edit.tab.format_graph.appearance",
            Self::SeriesAndArrangement => "edit.tab.format_graph.series",
            Self::GraphSettings => "edit.tab.format_graph.settings",
            Self::ValueAnnotations => "edit.tab.format_graph.annotations",
            Self::AxisVariables => "edit.tab.format_graph.axis_variables",
            Self::DataObjects => "edit.tab.format_graph.data_objects",
            Self::Legends => "edit.tab.format_graph.legends",
            Self::ColorMapping => "edit.tab.format_graph.color_mapping",
            Self::HeatmapLabels => "edit.tab.format_graph.heatmap_labels",
            Self::HeatmapBorders => "edit.tab.format_graph.heatmap_borders",
        }
    }

    #[must_use]
    const fn label_key(self) -> &'static str {
        match self {
            Self::Appearance => "fg.tab.appearance",
            Self::SeriesAndArrangement => "fg.tab.series",
            Self::GraphSettings => "fg.tab.settings",
            Self::ValueAnnotations => "fg.tab.annotations",
            Self::AxisVariables => "fg.tab.axis_variables",
            Self::DataObjects => "fg.tab.data_objects",
            Self::Legends => "fg.tab.legends",
            Self::ColorMapping => "fg.tab.color_mapping",
            Self::HeatmapLabels => "fg.tab.heatmap_labels",
            Self::HeatmapBorders => "fg.tab.heatmap_borders",
        }
    }
}

/// Active context-switched `Format Graph` inspector schema (`gui-layout-spec.md` §8.11C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormatGraphSchema {
    #[default]
    SeriesBased,
    MvChannelLayer,
    Heatmap,
}

impl FormatGraphSchema {
    /// Resolve schema from active `GraphKind` and `FormatGraphState`.
    #[must_use]
    pub fn detect(app: &UiApp) -> Self {
        let gid = app.canvas_dialogs.format_graph.graph_sheet_id.as_ref();
        let kind = gid.and_then(|id| app.document().graph_sheet(id)).map(|gs| gs.spec.kind).or(app.canvas_dialogs.format_graph.graph_kind);
        match kind {
            Some(GraphKind::Diagnostic(DiagnosticPlotKind::Heatmap)) => Self::Heatmap,
            Some(GraphKind::MultipleVariables) => Self::MvChannelLayer,
            _ => Self::SeriesBased,
        }
    }

    /// Ordered tabs belonging to this schema.
    #[must_use]
    pub const fn tabs(self) -> &'static [FormatGraphTab] {
        match self {
            Self::SeriesBased => {
                &[FormatGraphTab::Appearance, FormatGraphTab::SeriesAndArrangement, FormatGraphTab::GraphSettings, FormatGraphTab::ValueAnnotations]
            }
            Self::MvChannelLayer => &[FormatGraphTab::AxisVariables, FormatGraphTab::DataObjects, FormatGraphTab::Legends],
            Self::Heatmap => &[FormatGraphTab::ColorMapping, FormatGraphTab::HeatmapLabels, FormatGraphTab::HeatmapBorders],
        }
    }

    /// Ensure `tab` belongs to this schema, returning the first valid tab when mismatched.
    #[must_use]
    pub fn clamp_tab(self, tab: FormatGraphTab) -> FormatGraphTab {
        let valid = self.tabs();
        if valid.contains(&tab) { tab } else { valid[0] }
    }
}

/// Top-level tab selection inside `Format Axes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FormatAxesTab {
    #[default]
    FrameAndOrigin,
    XAxis,
    LeftYAxis,
    RightYAxis,
    TitlesAndTypography,
}

impl FormatAxesTab {
    pub const ALL: [Self; 5] = [Self::FrameAndOrigin, Self::XAxis, Self::LeftYAxis, Self::RightYAxis, Self::TitlesAndTypography];

    #[must_use]
    const fn widget_id(self) -> &'static str {
        match self {
            Self::FrameAndOrigin => "edit.tab.axes.frame_origin",
            Self::XAxis => "edit.tab.axes.x",
            Self::LeftYAxis => "edit.tab.axes.y",
            Self::RightYAxis => "edit.tab.axes.y2",
            Self::TitlesAndTypography => "edit.tab.axes.titles",
        }
    }

    #[must_use]
    const fn label_key(self) -> &'static str {
        match self {
            Self::FrameAndOrigin => "axes.tab.frame",
            Self::XAxis => "axes.tab.x",
            Self::LeftYAxis => "axes.tab.y",
            Self::RightYAxis => "axes.tab.y2",
            Self::TitlesAndTypography => "axes.tab.titles",
        }
    }
}

/// Family filter pill inside `Change Graph Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GraphTypeFamily {
    #[default]
    Recommended,
    All,
    Xy,
    ColumnAndGrouped,
    SurvivalAndParts,
    MultivariateAndDiag,
}

impl GraphTypeFamily {
    pub const ALL: [Self; 6] = [Self::Recommended, Self::All, Self::Xy, Self::ColumnAndGrouped, Self::SurvivalAndParts, Self::MultivariateAndDiag];

    #[must_use]
    const fn slug(self) -> &'static str {
        match self {
            Self::Recommended => "recommended",
            Self::All => "all",
            Self::Xy => "xy",
            Self::ColumnAndGrouped => "column_grouped",
            Self::SurvivalAndParts => "survival_parts",
            Self::MultivariateAndDiag => "mv_diag",
        }
    }

    #[must_use]
    const fn label(self) -> &'static str {
        match self {
            Self::Recommended => "Recommended",
            Self::All => "All",
            Self::Xy => "XY",
            Self::ColumnAndGrouped => "Column / Grouped",
            Self::SurvivalAndParts => "Survival / Parts",
            Self::MultivariateAndDiag => "MV / Diagnostic",
        }
    }

    #[must_use]
    const fn matches(self, opt: &GraphTypeOption) -> bool {
        match self {
            Self::Recommended | Self::All => true,
            Self::Xy => matches!(opt.graph_kind, GraphKind::Xy),
            Self::ColumnAndGrouped => matches!(opt.graph_kind, GraphKind::Column | GraphKind::Grouped | GraphKind::Contingency),
            Self::SurvivalAndParts => matches!(opt.graph_kind, GraphKind::Survival | GraphKind::PartsOfWhole),
            Self::MultivariateAndDiag => matches!(opt.graph_kind, GraphKind::MultipleVariables | GraphKind::Diagnostic(_)),
        }
    }
}

/// Out-of-range bubble size handling in MV Schema 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MvOutOfRangePolicy {
    #[default]
    ClampToBounds,
    Omit,
}

/// Sub-modal draft for adding/editing an `Additional Ticks / Thresholds` line or shaded zone.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AdditionalTickDraft {
    pub open: bool,
    pub selected_index: Option<usize>,
    pub value_draft: String,
    pub end_value_draft: String,
    pub label_draft: String,
    pub show_line: bool,
    pub shaded_zone: bool,
}

/// Persistent transient UI state for `graph_modals`.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphModalEguiState {
    pub format_graph_tab: FormatGraphTab,
    pub format_axes_tab: FormatAxesTab,
    pub additional_ticks: AdditionalTickDraft,
    pub categorical_guard_confirmed: bool,
    pub categorical_guard_pending_axis: Option<String>,
    pub symbol_size_spinner: ScrubUnitFieldState,
    pub axis_min_spinner: ScrubUnitFieldState,
    pub axis_max_spinner: ScrubUnitFieldState,
    pub line_thickness_spinner: ScrubUnitFieldState,
    pub graph_type_family: GraphTypeFamily,
    pub graph_type_show_all: bool,
    pub format_graph_apply_globally: bool,
    pub format_graph_alpha_pct: f64,
    pub heatmap_colormap: ColormapId,
    pub heatmap_reversed: bool,
    pub heatmap_auto_range: bool,
    pub heatmap_min_draft: String,
    pub heatmap_max_draft: String,
    pub heatmap_annotate_cells: bool,
    pub heatmap_cell_gap_pt: f64,
    pub heatmap_tick_angle_deg: f64,
    pub heatmap_gap_spinner: ScrubUnitFieldState,
    pub mv_min_radius_spinner: ScrubUnitFieldState,
    pub mv_max_radius_spinner: ScrubUnitFieldState,
    pub mv_out_of_range: MvOutOfRangePolicy,
    pub format_axes_last_sheet_id: Option<String>,
    pub format_axes_last_target: Option<AxisTarget>,
    pub format_axes_frame_preset: FramePreset,
    pub format_axes_title_draft: String,
    pub format_axes_guard_pending_scale: Option<GraphAxisScale>,
    pub format_axes_guard_pending_tick_format: Option<TickLabelFormat>,
    pub additional_tick_draw_gridline: bool,
    pub additional_tick_status: Option<String>,
    pub axis_gap_spinner: ScrubUnitFieldState,
    pub axis_stroke_spinner: ScrubUnitFieldState,
    pub cld_font_spinner: ScrubUnitFieldState,
    pub cld_offset_spinner: ScrubUnitFieldState,
    pub eq_font_spinner: ScrubUnitFieldState,
    pub pts_size_spinner: ScrubUnitFieldState,
    pub pts_border_spinner: ScrubUnitFieldState,
    pub page_callout_draft: String,
}

impl Default for GraphModalEguiState {
    fn default() -> Self {
        Self {
            format_graph_tab: FormatGraphTab::default(),
            format_axes_tab: FormatAxesTab::default(),
            additional_ticks: AdditionalTickDraft::default(),
            categorical_guard_confirmed: false,
            categorical_guard_pending_axis: None,
            symbol_size_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            axis_min_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            axis_max_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            line_thickness_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            graph_type_family: GraphTypeFamily::Recommended,
            graph_type_show_all: false,
            format_graph_apply_globally: false,
            format_graph_alpha_pct: 100.0,
            heatmap_colormap: ColormapId::Viridis,
            heatmap_reversed: false,
            heatmap_auto_range: true,
            heatmap_min_draft: String::from("-1.0"),
            heatmap_max_draft: String::from("1.0"),
            heatmap_annotate_cells: true,
            heatmap_cell_gap_pt: 1.5,
            heatmap_tick_angle_deg: 0.0,
            heatmap_gap_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_min_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_max_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_out_of_range: MvOutOfRangePolicy::ClampToBounds,
            format_axes_last_sheet_id: None,
            format_axes_last_target: None,
            format_axes_frame_preset: FramePreset::OpenL,
            format_axes_title_draft: String::new(),
            format_axes_guard_pending_scale: None,
            format_axes_guard_pending_tick_format: None,
            additional_tick_draw_gridline: true,
            additional_tick_status: None,
            axis_gap_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            axis_stroke_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            cld_font_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            cld_offset_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            eq_font_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            pts_size_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            pts_border_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            page_callout_draft: String::new(),
        }
    }
}

/// Frame geometry and modal capture report emitted by `graph_modals`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphModalFrameReport {
    pub modal_open: bool,
    pub dialog_rect_pt: Option<Rect>,
    pub outside_click_captured: bool,
    pub outside_wheel_captured: bool,
}

/// Return `true` when `app.canvas_dialogs.active_dialog` belongs to `graph_modals`.
#[must_use]
pub const fn is_graph_modal_active(dialog: &ActiveDialog) -> bool {
    matches!(
        dialog,
        ActiveDialog::ChangeGraphType { .. }
            | ActiveDialog::FormatGraph { .. }
            | ActiveDialog::FormatAxes { .. }
            | ActiveDialog::PairwiseComparisons { .. }
            | ActiveDialog::BubblePlotInspector { .. }
            | ActiveDialog::ApplyStyleFrom { .. }
            | ActiveDialog::FormatSelectedPoints { .. }
            | ActiveDialog::InsertModelEquation { .. }
            | ActiveDialog::PageLayoutComposer { .. }
    )
}

struct GraphModalCtx<'a> {
    app: &'a UiApp,
    tokens: &'a Tokens,
    locale: &'a str,
    meta: &'a mut FrameInspectMeta,
    intents: &'a mut PendingUiIntents,
    body_w: f32,
}

struct ButtonEntry<'a> {
    widget_id: &'a str,
    label: &'a str,
    command_id: Option<&'a str>,
    selected: bool,
    enabled: bool,
    disabled_reason: Option<&'a str>,
}

/// Render the active graph or page-layout configuration modal when active.
pub fn render_graph_modal(
    ctx: &Context,
    app: &UiApp,
    state: &mut GraphModalEguiState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
    intents: &mut PendingUiIntents,
) -> GraphModalFrameReport {
    let locale = normalize_locale(&app.locale);
    let (widget_id, title, preferred_w) = match &app.canvas_dialogs.active_dialog {
        ActiveDialog::ChangeGraphType { .. } => ("dialog.change_graph_type", tr(locale, "cmd.graph.setKind"), 760.0_f32),
        ActiveDialog::FormatGraph { .. } => ("dialog.format_graph", tr(locale, "inspector.format_graph"), 680.0_f32),
        ActiveDialog::FormatAxes { .. } => ("dialog.format_axes", tr(locale, "inspector.format_axes"), 680.0_f32),
        ActiveDialog::PairwiseComparisons { .. } => ("dialog.pairwise_comparisons", tr(locale, "inspector.pairwise_comparisons"), 600.0_f32),
        ActiveDialog::BubblePlotInspector { .. } => ("dialog.bubble_plot", tr(locale, "w6.view.modal.bubble_plot_channels"), 580.0_f32),
        ActiveDialog::ApplyStyleFrom { .. } => ("dialog.apply_style_from", tr(locale, "btn.apply_style_from"), 520.0_f32),
        ActiveDialog::FormatSelectedPoints { .. } => ("dialog.format_selected_points", tr(locale, "cmd.graph.formatSelectedPoints"), 540.0_f32),
        ActiveDialog::InsertModelEquation { .. } => ("dialog.insert_model_equation", tr(locale, "w6.view.modal.insert_model_equation"), 540.0_f32),
        ActiveDialog::PageLayoutComposer { .. } => ("dialog.page_layout_composer", tr(locale, "w6.view.modal.page_layout_composer"), 620.0_f32),
        _ => return GraphModalFrameReport::default(),
    };

    let screen = ctx.content_rect();
    let max_w = (screen.width() - 28.0).clamp(260.0, 920.0);
    let max_h = (screen.height() - 28.0).clamp(220.0, 660.0);
    let inner_w = max_w.min(preferred_w).max(260.0);
    let body_w = (inner_w - 28.0).max(200.0);

    let modal_resp = egui::Modal::new(egui::Id::unique(("egui_shell_graph_modal", widget_id))).backdrop_color(tokens.scrim).show(ctx, |ui| {
        ui.set_min_width(inner_w);
        ui.set_max_width(inner_w);
        ui.set_max_height(max_h);
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();

        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(inner_w);
            ui.heading(title);
        });
        ui.add_space(4.0);

        let mut gctx = GraphModalCtx { app, tokens, locale, meta, intents, body_w };
        let header_tabs_h = render_dialog_top_strip(ui, state, &mut gctx);
        let footer_h = if inner_w < 460.0 { 68.0 } else { 42.0 };
        let body_h = (max_h - 46.0 - header_tabs_h - footer_h).max(110.0);

        egui::ScrollArea::vertical()
            .id_salt(("graph_modal.scroll", widget_id))
            .max_height(body_h)
            .auto_shrink([false, true])
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                ui.set_width(body_w);
                ui.set_max_width(body_w);
                render_dialog_scroll_body(ui, state, &mut gctx);
            });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        render_dialog_footer(ui, state, &mut gctx);
    });

    modal_resp.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Dialog, true, title));
    let capture = finalize_egui_modal_response(ctx, &modal_resp, ModalCaptureSpec { widget_id, title, z_dialog: ZLayer::ModalDialog }, meta);
    GraphModalFrameReport {
        modal_open: true,
        dialog_rect_pt: capture.dialog_rect_pt,
        outside_click_captured: capture.outside_click_captured,
        outside_wheel_captured: capture.outside_wheel_captured,
    }
}

fn render_dialog_top_strip(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) -> f32 {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => {
            let schema = FormatGraphSchema::detect(gctx.app);
            let active_tab = schema.clamp_tab(state.format_graph_tab);
            state.format_graph_tab = active_tab;
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let cgt_lbl = gtr(gctx.locale, "fg.jump.change_graph_type");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "format_graph.open_change_graph_type",
                        label: cgt_lbl,
                        command_id: Some("ui.changeGraphType.open"),
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    gctx.intents.push_ui("ui.changeGraphType.open", json!({}));
                }
                let axes_lbl = gtr(gctx.locale, "fg.jump.format_axes");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "format_graph.open_format_axes",
                        label: axes_lbl,
                        command_id: Some("ui.graph.axes.open"),
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    gctx.intents.push_ui("ui.graph.axes.open", json!({}));
                }
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in schema.tabs() {
                    let lbl = gtr(gctx.locale, tab.label_key());
                    if record_tab(ui, gctx.meta, tab.widget_id(), lbl, tab == active_tab, gctx.tokens).clicked() {
                        state.format_graph_tab = tab;
                    }
                }
            });
            ui.add_space(4.0);
            64.0
        }
        ActiveDialog::FormatAxes { .. } => {
            if state.additional_ticks.open {
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(gctx.body_w);
                    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.add_tick.submodal_title")).strong());
                });
                ui.add_space(4.0);
                return 28.0;
            }
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                for &tab in &FormatAxesTab::ALL {
                    let lbl = gtr(gctx.locale, tab.label_key());
                    if record_tab(ui, gctx.meta, tab.widget_id(), lbl, tab == state.format_axes_tab, gctx.tokens).clicked() {
                        state.format_axes_tab = tab;
                        let target_slug = match tab {
                            FormatAxesTab::XAxis => Some("x"),
                            FormatAxesTab::LeftYAxis => Some("y"),
                            FormatAxesTab::RightYAxis => Some("y2"),
                            FormatAxesTab::FrameAndOrigin | FormatAxesTab::TitlesAndTypography => None,
                        };
                        if let Some(slug) = target_slug {
                            let mut patch = Map::new();
                            patch.insert(String::from("axes.target_axis"), Value::String(slug.to_owned()));
                            gctx.intents.push_patch_active_dialog(patch);
                        }
                    }
                }
            });
            ui.add_space(4.0);
            34.0
        }
        _ => 0.0,
    }
}

fn render_dialog_scroll_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::ChangeGraphType { .. } => render_change_graph_type_body(ui, state, gctx),
        ActiveDialog::FormatGraph { .. } => render_format_graph_body(ui, state, gctx),
        ActiveDialog::FormatAxes { .. } => render_format_axes_body(ui, state, gctx),
        ActiveDialog::PairwiseComparisons { .. } => render_pairwise_body(ui, state, gctx),
        ActiveDialog::BubblePlotInspector { .. } => render_bubble_plot_body(ui, state, gctx),
        ActiveDialog::ApplyStyleFrom { .. } => render_apply_style_body(ui, gctx),
        ActiveDialog::FormatSelectedPoints { .. } => render_format_selected_points_body(ui, state, gctx),
        ActiveDialog::InsertModelEquation { .. } => render_insert_equation_body(ui, state, gctx),
        ActiveDialog::PageLayoutComposer { .. } => render_page_layout_body(ui, state, gctx),
        _ => {}
    }
}

fn render_dialog_footer(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let active = &gctx.app.canvas_dialogs.active_dialog;
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if matches!(active, ActiveDialog::FormatAxes { .. }) && state.additional_ticks.open {
            let close_lbl = gtr(gctx.locale, "axes.add_tick.done");
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: "axes.add_tick.close",
                    label: close_lbl,
                    command_id: None,
                    selected: false,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.additional_ticks.open = false;
            }
            return;
        }

        let cancel_lbl = tr(gctx.locale, "btn.cancel");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry { widget_id: "modal.cancel", label: cancel_lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
            gctx.tokens,
        )
        .clicked()
        {
            gctx.intents.push_dialog_button("cancel");
        }

        let supports_apply = matches!(
            active,
            ActiveDialog::ChangeGraphType { .. }
                | ActiveDialog::FormatGraph { .. }
                | ActiveDialog::FormatAxes { .. }
                | ActiveDialog::PairwiseComparisons { .. }
        );
        if supports_apply {
            let apply_lbl = gtr(gctx.locale, "modal.apply");
            let apply_wid = if matches!(active, ActiveDialog::ChangeGraphType { .. }) { "graph_type.apply" } else { "modal.apply" };
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry { widget_id: apply_wid, label: apply_lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
                gctx.tokens,
            )
            .clicked()
            {
                commit_graph_modal_action(gctx, state, "apply");
            }
        }

        let ok_lbl = tr(gctx.locale, "btn.ok");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry { widget_id: "modal.ok", label: ok_lbl, command_id: None, selected: true, enabled: true, disabled_reason: None },
            gctx.tokens,
        )
        .clicked()
        {
            commit_graph_modal_action(gctx, state, "ok");
        }
    });
}

fn commit_graph_modal_action(gctx: &mut GraphModalCtx<'_>, state: &mut GraphModalEguiState, action: &str) {
    match &gctx.app.canvas_dialogs.active_dialog {
        ActiveDialog::FormatGraph { .. } => {
            let fg = &gctx.app.canvas_dialogs.format_graph;
            let schema = FormatGraphSchema::detect(gctx.app);
            if let Some(gid) = &fg.graph_sheet_id {
                if schema == FormatGraphSchema::SeriesBased && state.format_graph_apply_globally {
                    if let Some(gs) = gctx.app.document().graph_sheet(gid) {
                        for idx in 0..gs.spec.series.len() {
                            gctx.intents.push_engine(
                                "graph.setSeries",
                                json!({
                                    "sheet_id": gid.as_str(),
                                    "series_index": idx,
                                    "y_error_bars": fg.error_bar_enabled.then_some(fg.error_bar_spec),
                                    "marker": { "size_pt": fg.marker_size_pt, "fill_mode": fg.marker_fill_mode },
                                    "connecting_line": { "dash_style": fg.line_dash },
                                }),
                            );
                        }
                    }
                } else if schema == FormatGraphSchema::MvChannelLayer
                    && let Some(enc) = &fg.mv_encoding
                {
                    gctx.intents.push_engine(
                        "graph.setSpec",
                        json!({
                            "sheet_id": gid.as_str(),
                            "palette": fg.palette,
                            "multivariate": enc,
                            "legend": { "visible": fg.show_legend, "bordered": fg.legend_bordered },
                        }),
                    );
                } else if schema == FormatGraphSchema::Heatmap {
                    let clamp = if state.heatmap_auto_range {
                        None
                    } else {
                        match (state.heatmap_min_draft.trim().parse::<f64>(), state.heatmap_max_draft.trim().parse::<f64>()) {
                            (Ok(lo), Ok(hi)) if lo.is_finite() && hi.is_finite() && lo < hi => Some((lo, hi)),
                            _ => None,
                        }
                    };
                    if let Some(mut enc) = fg.mv_encoding.clone() {
                        let col = match enc.color {
                            ColorChannelMapping::Continuous { column, .. } | ColorChannelMapping::Categorical { column, .. } => column,
                            ColorChannelMapping::Constant(_) => enc.y_column,
                        };
                        enc.color = ColorChannelMapping::Continuous {
                            column: col,
                            ramp: ColorRamp::Named { id: state.heatmap_colormap, reversed: state.heatmap_reversed },
                            domain_clamp: clamp,
                        };
                        gctx.intents.push_engine(
                            "graph.setSpec",
                            json!({
                                "sheet_id": gid.as_str(),
                                "multivariate": enc,
                            }),
                        );
                    }
                    gctx.intents.push_engine(
                        "graph.setAxes",
                        json!({
                            "sheet_id": gid.as_str(),
                            "axis": "x",
                            "tick_angle_deg": state.heatmap_tick_angle_deg,
                        }),
                    );
                }
            }
            gctx.intents.push_dialog_button(action);
        }
        ActiveDialog::FormatAxes { .. } => {
            let fa = &gctx.app.canvas_dialogs.format_axes;
            if let Some(gid) = &fa.graph_sheet_id {
                gctx.intents.push_engine(
                    "graph.setAxes",
                    json!({
                        "sheet_id": gid.as_str(),
                        "frame_preset": state.format_axes_frame_preset,
                    }),
                );
            }
            if action == "apply" {
                if let Some(intent) = fa.commit_intent() {
                    gctx.intents.push_engine(intent.command_id, intent.params);
                }
            } else {
                gctx.intents.push_dialog_button(action);
            }
        }
        _ => {
            gctx.intents.push_dialog_button(action);
        }
    }
}

// ============================================================================
// 1. Change Graph Type Modal (§8.11B)
// ============================================================================

fn render_change_graph_type_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let cgt = &gctx.app.canvas_dialogs.change_graph_type;
    let doc = gctx.app.document();

    // Family filter strip & Show All toggle.
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &fam in &GraphTypeFamily::ALL {
            let selected = fam == state.graph_type_family && !(fam == GraphTypeFamily::All && !state.graph_type_show_all);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("graph_type.family.{}", fam.slug()),
                    label: fam.label(),
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.graph_type_family = fam;
                state.graph_type_show_all = fam == GraphTypeFamily::All;
            }
        }
        let mut show_all = state.graph_type_show_all;
        let show_all_lbl = gtr(gctx.locale, "cgt.show_all");
        let chk = ui.checkbox(&mut show_all, show_all_lbl);
        record_checkbox(ui, &chk, gctx.meta, "graph_type.show_all", show_all_lbl, show_all);
        if chk.changed() {
            state.graph_type_show_all = show_all;
            state.graph_type_family = if show_all { GraphTypeFamily::All } else { GraphTypeFamily::Recommended };
        }
    });
    ui.add_space(6.0);

    // Build visible options cleanly from GRAPH_TYPE_OPTIONS.
    let base_options: Vec<&'static GraphTypeOption> = if state.graph_type_show_all || state.graph_type_family != GraphTypeFamily::Recommended {
        GRAPH_TYPE_OPTIONS.iter().filter(|opt| state.graph_type_family.matches(opt)).collect()
    } else {
        cgt.recommended_options()
    };

    let cols = if gctx.body_w >= 540.0 { 2 } else { 1 };
    let card_w = ((gctx.body_w - 8.0 * ((cols - 1) as f32)) / (cols as f32)).max(180.0);

    for row_chunk in base_options.chunks(cols) {
        ui.horizontal(|ui| {
            for &opt in row_chunk {
                let selected = cgt.selected_option_id == opt.id;
                let compatible = cgt.source_table_kind.is_none_or(|k| opt.compatible_tables.contains(&k));
                let resp = render_graph_type_card(ui, gctx, opt, card_w, selected, compatible);
                if resp.clicked() {
                    let mut next = cgt.clone();
                    next.selected_option_id = opt.id.to_owned();
                    gctx.intents.push_update_change_graph_type_state(next);
                }
            }
        });
        ui.add_space(4.0);
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    // Summary & Error Bar adaptive controls + compatibility hint.
    let active_opt = GRAPH_TYPE_OPTIONS.iter().find(|o| o.id == cgt.selected_option_id).copied();
    let supports_summary = active_opt.is_some_and(|o| matches!(o.graph_kind, GraphKind::Column | GraphKind::Grouped | GraphKind::Xy));
    if supports_summary {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.summary")).strong());
            for (slug, lbl, val) in [
                ("mean", "Mean", SummaryMeasure::Mean),
                ("median", "Median", SummaryMeasure::Median),
                ("geomean", "Geom. Mean", SummaryMeasure::GeometricMean),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("graph_type.sum.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: cgt.summary_measure == val,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = cgt.clone();
                    next.summary_measure = val;
                    gctx.intents.push_update_change_graph_type_state(next);
                }
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.error_bars")).strong());
            for (slug, lbl, metric) in [
                ("sd", "SD", ErrorBarMetric::Sd),
                ("sem", "SEM", ErrorBarMetric::Sem),
                ("ci95", "95% CI", ErrorBarMetric::ConfidenceInterval95),
                ("iqr", "IQR", ErrorBarMetric::InterquartileRange),
                ("range", "Range", ErrorBarMetric::Range),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("graph_type.eb.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: cgt.error_bar_metric == metric,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = cgt.clone();
                    next.error_bar_metric = metric;
                    gctx.intents.push_update_change_graph_type_state(next);
                }
            }
        });
        ui.add_space(6.0);
    }

    // Compatibility Hint Banner.
    let hint_text = build_compatibility_hint(cgt.source_table_kind, cgt.column_design_hint, active_opt.as_ref(), gctx.locale);
    egui::Frame::group(ui.style()).fill(gctx.tokens.panel_bg).stroke(Stroke::new(1.0, gctx.tokens.border)).corner_radius(CornerRadius::same(4)).show(
        ui,
        |ui| {
            ui.set_max_width((gctx.body_w - 14.0).max(160.0));
            let lbl_resp = ui.add(egui::Label::new(egui::RichText::new(&hint_text).small()).wrap());
            record_label(ui, &lbl_resp, gctx.meta, "graph_type.compatibility_hint", &hint_text);
        },
    );

    ui.add_space(6.0);
    render_live_dataset_preview(ui, gctx, cgt.graph_sheet_id.as_ref(), doc);
}

fn build_compatibility_hint(
    src_kind: Option<TableKind>,
    design_hint: Option<ColumnDesignHint>,
    opt: Option<&GraphTypeOption>,
    locale: &str,
) -> String {
    if let Some(o) = opt
        && let Some(kind) = src_kind
        && !o.compatible_tables.contains(&kind)
    {
        return format!("{} ({:?} -> {})", gtr(locale, "cgt.hint.cross_table"), kind, o.label);
    }
    if design_hint == Some(ColumnDesignHint::PairedRepeatedMeasures) {
        return gtr(locale, "cgt.hint.paired").to_owned();
    }
    gtr(locale, "cgt.hint.compatible").to_owned()
}

fn render_graph_type_card(
    ui: &mut Ui,
    gctx: &mut GraphModalCtx<'_>,
    opt: &GraphTypeOption,
    card_w: f32,
    selected: bool,
    compatible: bool,
) -> Response {
    let card_h = 56.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(card_w, card_h), Sense::click());
    let stroke_color = if selected { gctx.tokens.accent_strong } else { gctx.tokens.border };
    let fill_color = if selected {
        gctx.tokens.active_bg
    } else if resp.hovered() {
        gctx.tokens.panel_bg
    } else {
        gctx.tokens.surface_bg
    };
    ui.painter().rect(rect, CornerRadius::same(6), fill_color, Stroke::new(if selected { 1.5 } else { 1.0 }, stroke_color), StrokeKind::Inside);

    let thumb_rect = Rect::from_min_size(rect.min + Vec2::new(8.0, 8.0), Vec2::new(40.0, 40.0));
    paint_thumbnail_glyph(ui.painter(), thumb_rect, opt.thumbnail, gctx.tokens);

    let text_x = thumb_rect.right() + 8.0;
    let text_w = (rect.right() - text_x - 8.0).max(40.0);
    let (title_galley, elided) = layout_elided(ui.painter(), opt.label, text_w, gctx.tokens.text_primary, 12.0);
    ui.painter().galley(Pos2::new(text_x, rect.top() + 8.0), title_galley, gctx.tokens.text_primary);

    let tag = if compatible { opt.subtitle } else { "Cross-table format hint" };
    let (sub_galley, _) = layout_elided(ui.painter(), tag, text_w, gctx.tokens.text_muted, 10.5);
    ui.painter().galley(Pos2::new(text_x, rect.top() + 28.0), sub_galley, gctx.tokens.text_muted);

    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, selected, opt.label));
    if elided || resp.hovered() {
        resp.clone().on_hover_text(format!("{}\n{}", opt.label, opt.subtitle));
    }
    gctx.meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: &format!("graph_type.card.{}", opt.id),
            role: "option",
            label: opt.label,
            command_id: None,
            value: Some(json!({ "selected": selected, "compatible": compatible })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: elided,
            full_label: elided.then_some(opt.label),
        },
    );
    resp
}

fn paint_thumbnail_glyph(painter: &egui::Painter, rect: Rect, kind: ThumbnailGlyphKind, tokens: &Tokens) {
    painter.rect(rect, CornerRadius::same(4), tokens.panel_bg, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let c0 = tokens.accent_strong;
    let c1 = tokens.text_primary;
    let st = Stroke::new(1.5, c0);
    let axis_st = Stroke::new(1.0, tokens.text_muted);
    painter.line_segment([Pos2::new(inner.left(), inner.top()), Pos2::new(inner.left(), inner.bottom())], axis_st);
    painter.line_segment([Pos2::new(inner.left(), inner.bottom()), Pos2::new(inner.right(), inner.bottom())], axis_st);

    match kind {
        ThumbnailGlyphKind::Scatter | ThumbnailGlyphKind::Volcano | ThumbnailGlyphKind::BlandAltman | ThumbnailGlyphKind::ResidualPlot => {
            for (fx, fy) in [(0.25, 0.72), (0.45, 0.45), (0.65, 0.58), (0.82, 0.25)] {
                painter.circle_filled(lerp_rect(inner, fx, fy), 2.3, c0);
            }
        }
        ThumbnailGlyphKind::ScatterLine | ThumbnailGlyphKind::RocCurve | ThumbnailGlyphKind::NormalQqPlot => {
            let pts = [lerp_rect(inner, 0.15, 0.8), lerp_rect(inner, 0.4, 0.55), lerp_rect(inner, 0.65, 0.35), lerp_rect(inner, 0.9, 0.18)];
            painter.add(egui::Shape::line(pts.to_vec(), st));
            for p in pts {
                painter.circle_filled(p, 2.0, c1);
            }
        }
        ThumbnailGlyphKind::BarReplicates | ThumbnailGlyphKind::GroupedInterleaved | ThumbnailGlyphKind::StackedBar => {
            for (idx, h) in [0.45, 0.75, 0.6].iter().enumerate() {
                let x = inner.left() + 4.0 + (idx as f32) * (inner.width() / 3.2);
                let top = inner.bottom() - inner.height() * *h;
                painter.rect_filled(Rect::from_min_max(Pos2::new(x, top), Pos2::new(x + 6.0, inner.bottom())), CornerRadius::ZERO, c0);
            }
        }
        ThumbnailGlyphKind::BeforeAfter => {
            for (y0, y1) in [(0.7, 0.3), (0.5, 0.2), (0.8, 0.55)] {
                let p0 = lerp_rect(inner, 0.25, y0);
                let p1 = lerp_rect(inner, 0.75, y1);
                painter.line_segment([p0, p1], st);
                painter.circle_filled(p0, 2.0, c1);
                painter.circle_filled(p1, 2.0, c0);
            }
        }
        ThumbnailGlyphKind::BoxWhisker | ThumbnailGlyphKind::ForestPlot => {
            for (fx, top, bot) in [(0.35, 0.35, 0.65), (0.7, 0.2, 0.55)] {
                let cx = inner.left() + inner.width() * fx;
                painter.line_segment([Pos2::new(cx, inner.top() + 3.0), Pos2::new(cx, inner.bottom() - 3.0)], Stroke::new(1.0, c1));
                painter.rect(
                    Rect::from_min_max(
                        Pos2::new(cx - 4.0, inner.top() + inner.height() * top),
                        Pos2::new(cx + 4.0, inner.top() + inner.height() * bot),
                    ),
                    CornerRadius::ZERO,
                    tokens.active_bg,
                    st,
                    StrokeKind::Inside,
                );
            }
        }
        ThumbnailGlyphKind::Violin | ThumbnailGlyphKind::AlignedDot => {
            let pts = [lerp_rect(inner, 0.5, 0.15), lerp_rect(inner, 0.75, 0.45), lerp_rect(inner, 0.5, 0.85), lerp_rect(inner, 0.25, 0.45)];
            painter.add(egui::Shape::convex_polygon(pts.to_vec(), tokens.active_bg, st));
        }
        ThumbnailGlyphKind::KaplanMeier => {
            let pts = [
                lerp_rect(inner, 0.1, 0.2),
                lerp_rect(inner, 0.4, 0.2),
                lerp_rect(inner, 0.4, 0.5),
                lerp_rect(inner, 0.75, 0.5),
                lerp_rect(inner, 0.75, 0.8),
                lerp_rect(inner, 0.95, 0.8),
            ];
            painter.add(egui::Shape::line(pts.to_vec(), st));
        }
        ThumbnailGlyphKind::Donut => {
            let c = inner.center();
            let r = inner.width().min(inner.height()) * 0.42;
            painter.circle_stroke(c, r, Stroke::new(4.0, c0));
        }
        ThumbnailGlyphKind::BubbleScatter => {
            painter.circle_filled(lerp_rect(inner, 0.3, 0.6), 4.0, c0);
            painter.circle_filled(lerp_rect(inner, 0.65, 0.35), 6.0, tokens.active_bg);
            painter.circle_stroke(lerp_rect(inner, 0.65, 0.35), 6.0, st);
        }
        ThumbnailGlyphKind::Heatmap => {
            let cw = inner.width() / 3.0;
            let ch = inner.height() / 3.0;
            for r in 0..3 {
                for c in 0..3 {
                    let shade = ((r * 3 + c) * 24) as u8;
                    let cell_col = Color32::from_rgb(40 + shade / 2, 110, 210 - shade / 3);
                    let cell =
                        Rect::from_min_size(Pos2::new(inner.left() + (c as f32) * cw, inner.top() + (r as f32) * ch), Vec2::new(cw - 1.0, ch - 1.0));
                    painter.rect_filled(cell, CornerRadius::ZERO, cell_col);
                }
            }
        }
    }
}

fn lerp_rect(rect: Rect, fx: f32, fy: f32) -> Pos2 {
    Pos2::new(rect.left() + rect.width() * fx, rect.top() + rect.height() * fy)
}

fn render_live_dataset_preview(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, graph_id: Option<&SheetId>, doc: &ProjectDocument) {
    ui.label(egui::RichText::new(tr(gctx.locale, "w5.graph_type.preview")).strong());
    ui.add_space(3.0);
    let preview_w = gctx.body_w;
    let preview_h = 112.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(preview_w, preview_h), Sense::hover());
    ui.painter().rect(rect, CornerRadius::same(6), Color32::WHITE, Stroke::new(1.0, gctx.tokens.border), StrokeKind::Inside);

    let compiled: Option<Result<ChartScene, String>> = graph_id.and_then(|id| doc.graph_sheet(id)).map(|gs| {
        let mut temp: GraphSheet = gs.clone();
        let _ = gctx.app.canvas_dialogs.change_graph_type.apply_to_spec(&mut temp.spec);
        compile_graph(&temp, doc).map_err(|e| e.to_string())
    });

    match compiled {
        Some(Ok(scene)) if !scene.primitives.is_empty() => {
            paint_chart_scene_compact(ui.painter(), &scene, rect.shrink(6.0));
        }
        Some(Err(err)) => {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, &err, FontId::proportional(11.0), gctx.tokens.text_muted);
        }
        _ => {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                gtr(gctx.locale, "cgt.preview.empty"),
                FontId::proportional(11.0),
                gctx.tokens.text_muted,
            );
        }
    }

    gctx.meta.record_response(
        ui,
        &resp,
        FrameInspectEntryInput {
            widget_id: "graph_type.preview_pane",
            role: "img",
            label: tr(gctx.locale, "w5.graph_type.preview"),
            command_id: None,
            value: Some(json!({ "selected_option": gctx.app.canvas_dialogs.change_graph_type.selected_option_id })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn paint_chart_scene_compact(painter: &egui::Painter, scene: &ChartScene, paper: Rect) {
    let sx = paper.width() / (scene.width_pt.max(1.0) as f32);
    let sy = paper.height() / (scene.height_pt.max(1.0) as f32);
    let map_pt = |pt: PointPt| Pos2::new(paper.left() + (pt.x as f32) * sx, paper.top() + (pt.y as f32) * sy);
    let clipped = painter.with_clip_rect(paper);

    for prim in &scene.primitives {
        if let ScenePrimitive::Path { path, fill, stroke, .. } = prim {
            let mut pts = Vec::new();
            let mut closed = false;
            for cmd in &path.commands {
                match *cmd {
                    PathCmd::MoveTo(p) => {
                        flush_preview_poly(&clipped, &pts, closed, fill.as_ref(), stroke.as_ref(), sx);
                        pts.clear();
                        closed = false;
                        pts.push(map_pt(p));
                    }
                    PathCmd::LineTo(p) | PathCmd::QuadTo { to: p, .. } | PathCmd::CubicTo { to: p, .. } => {
                        pts.push(map_pt(p));
                    }
                    PathCmd::Close => closed = true,
                }
            }
            flush_preview_poly(&clipped, &pts, closed, fill.as_ref(), stroke.as_ref(), sx);
        }
    }
}

fn flush_preview_poly(
    painter: &egui::Painter,
    pts: &[Pos2],
    closed: bool,
    fill: Option<&curvebook_chart::FillPaint>,
    stroke: Option<&curvebook_chart::StrokePaint>,
    scale: f32,
) {
    if pts.len() < 2 {
        return;
    }
    if closed && let Some(f) = fill {
        painter.add(egui::Shape::convex_polygon(pts.to_vec(), rgba_to_color32(f.color), Stroke::NONE));
    }
    if let Some(s) = stroke {
        let st = Stroke::new(((s.width_pt as f32) * scale).max(0.75), rgba_to_color32(s.color));
        if closed {
            painter.add(egui::Shape::closed_line(pts.to_vec(), st));
        } else {
            painter.add(egui::Shape::line(pts.to_vec(), st));
        }
    }
}

// ============================================================================
// 2. 3-Schema Format Graph Modal (§8.11C)
// ============================================================================

fn render_format_graph_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let schema = FormatGraphSchema::detect(gctx.app);
    let tab = schema.clamp_tab(state.format_graph_tab);
    state.format_graph_tab = tab;

    match (schema, tab) {
        (FormatGraphSchema::SeriesBased, FormatGraphTab::Appearance) => render_fg_series_appearance(ui, state, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::SeriesAndArrangement) => render_fg_series_arrangement(ui, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::GraphSettings) => render_fg_settings(ui, gctx),
        (FormatGraphSchema::SeriesBased, FormatGraphTab::ValueAnnotations) => render_fg_annotations(ui, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::AxisVariables) => render_fg_mv_axis_variables(ui, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::DataObjects) => render_fg_mv_data_objects(ui, state, gctx),
        (FormatGraphSchema::MvChannelLayer, FormatGraphTab::Legends) => render_fg_legends(ui, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::ColorMapping) => render_fg_heatmap_color_mapping(ui, state, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapLabels) => render_fg_heatmap_labels(ui, state, gctx),
        (FormatGraphSchema::Heatmap, FormatGraphTab::HeatmapBorders) => render_fg_heatmap_borders(ui, state, gctx),
        _ => render_fg_series_appearance(ui, state, gctx),
    }
}

fn render_fg_series_appearance(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let series_count = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).map_or(1, |gs| gs.spec.series.len().max(1));

    // Dataset Scope Header Bar + Apply Globally.
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(egui::RichText::new(gtr(gctx.locale, "fg.scope.series")).strong());
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.series_prev",
                label: "<",
                command_id: None,
                selected: false,
                enabled: fg.selected_series_index > 0,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && fg.selected_series_index > 0
        {
            let mut next = fg.clone();
            next.selected_series_index -= 1;
            gctx.intents.push_update_format_graph_state(next);
        }
        ui.label(format!("{} / {}", fg.selected_series_index + 1, series_count));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.series_next",
                label: ">",
                command_id: None,
                selected: false,
                enabled: fg.selected_series_index + 1 < series_count,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && fg.selected_series_index + 1 < series_count
        {
            let mut next = fg.clone();
            next.selected_series_index += 1;
            gctx.intents.push_update_format_graph_state(next);
        }

        let mut apply_all = state.format_graph_apply_globally;
        let all_lbl = gtr(gctx.locale, "fg.apply_globally");
        let chk = ui.checkbox(&mut apply_all, all_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.apply_globally", all_lbl, apply_all);
        if chk.changed() {
            state.format_graph_apply_globally = apply_all;
        }
    });

    ui.add_space(6.0);
    // Symbol Size via `unit_spinners` (switchable pt/mm/in, single-commit).
    if let Some(committed) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "format_graph.marker_size_pt",
            label: gtr(gctx.locale, "fg.symbol_size"),
            value_pt: fg.marker_size_pt,
            min_pt: 1.0,
            max_pt: 24.0,
            step_pt: 0.5,
            enabled: true,
            disabled_reason: None,
        },
        &mut state.symbol_size_spinner,
        gctx.tokens,
        gctx.meta,
    ) {
        let mut next = fg.clone();
        next.marker_size_pt = committed;
        gctx.intents.push_update_format_graph_state(next);
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(gtr(gctx.locale, "fg.symbol_fill"));
        for (slug, lbl, mode) in [
            ("filled", "Filled", MarkerFillMode::Filled),
            ("open", "Open", MarkerFillMode::Open),
            ("semi_top", "Semi-Top", MarkerFillMode::SemiFilledTop),
            ("semi_bottom", "Semi-Bottom", MarkerFillMode::SemiFilledBottom),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.fill.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.marker_fill_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.marker_fill_mode = mode;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(gtr(gctx.locale, "fg.line_dash"));
        for (slug, lbl, dash) in [
            ("solid", "Solid", LineDashStyle::Solid),
            ("dashed", "Dashed", LineDashStyle::Dashed),
            ("dotted", "Dotted", LineDashStyle::Dotted),
            ("long_dash", "Long Dash", LineDashStyle::LongDash),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.line_dash.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.line_dash == dash,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.line_dash = dash;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(4.0);

    // Error bars section.
    let mut eb_enabled = fg.error_bar_enabled;
    let eb_lbl = tr(gctx.locale, "inspector.error_bars");
    let eb_chk = ui.checkbox(&mut eb_enabled, eb_lbl);
    record_checkbox(ui, &eb_chk, gctx.meta, "format_graph.error_bar_enabled", eb_lbl, eb_enabled);
    if eb_chk.changed() {
        let mut next = fg.clone();
        next.error_bar_enabled = eb_enabled;
        gctx.intents.push_update_format_graph_state(next);
    }

    if eb_enabled {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, metric) in [
                ("sd", "SD", ErrorBarMetric::Sd),
                ("sem", "SEM", ErrorBarMetric::Sem),
                ("ci95", "95% CI", ErrorBarMetric::ConfidenceInterval95),
                ("iqr", "IQR", ErrorBarMetric::InterquartileRange),
                ("range", "Range", ErrorBarMetric::Range),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.error_bar_metric.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.error_bar_spec.metric == metric,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.error_bar_spec.metric = metric;
                    gctx.intents.push_update_format_graph_state(next);
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (slug, lbl, dir) in [
                ("both", "Both", ErrorBarDirection::Both),
                ("upper", "Upper", ErrorBarDirection::UpperOnly),
                ("lower", "Lower", ErrorBarDirection::LowerOnly),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.error_bar_dir.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.error_bar_spec.direction == dir,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.error_bar_spec.direction = dir;
                    gctx.intents.push_update_format_graph_state(next);
                }
            }
        });

        let stroke_cfg = ScrubUnitFieldConfig {
            step_per_pt: 0.25,
            decimals: 2,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("format_graph.error_bar_stroke", gtr(gctx.locale, "fg.error_stroke"), 0.25, 6.0)
        };
        let stroke_out =
            render_scrub_unit_field(ui, fg.error_bar_spec.stroke_width_pt, &mut state.line_thickness_spinner, stroke_cfg, gctx.tokens, gctx.meta);
        if let Some(committed) = stroke_out.committed_value {
            let mut next = fg.clone();
            next.error_bar_spec.stroke_width_pt = committed.clamp(0.25, 6.0);
            gctx.intents.push_update_format_graph_state(next);
        }
    }

    if fg.initial_violin_options.is_some() {
        ui.add_space(4.0);
        let mut half = fg.half_violin_with_points;
        let half_lbl = "Half-Violin + Points (Raincloud)";
        let chk = ui.checkbox(&mut half, half_lbl);
        record_checkbox(ui, &chk, gctx.meta, "format_graph.half_violin_with_points", half_lbl, half);
        if chk.changed() {
            let mut next = fg;
            next.half_violin_with_points = half;
            gctx.intents.push_update_format_graph_state(next);
        }
    }
}

fn render_fg_series_arrangement(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(gtr(gctx.locale, "fg.grouped_arrangement")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, arr) in [
            ("interleaved", "Interleaved", GroupedArrangement::Interleaved),
            ("separated", "Separated", GroupedArrangement::Separated),
            ("stacked", "Stacked", GroupedArrangement::Stacked),
            ("superimposed", "Superimposed", GroupedArrangement::Superimposed),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.arrangement.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.grouped_arrangement == arr,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.grouped_arrangement = arr;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(8.0);
    ui.label(egui::RichText::new(gtr(gctx.locale, "fg.active_series_list")).strong());
    if let Some(gs) = fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)) {
        for (idx, series) in gs.spec.series.iter().enumerate() {
            let swatch = fg.preview_swatch(idx);
            ui.horizontal(|ui| {
                let (s_rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                ui.painter().rect_filled(s_rect, CornerRadius::same(3), rgba_to_color32(swatch));
                let title = series.legend_label.clone().unwrap_or_else(|| format!("Series {}", idx + 1));
                let axis_lbl = if series.bind_to_right_y2 { "Right Y2" } else { "Left Y" };
                ui.label(format!("#{}: {} ({axis_lbl})", idx + 1, title));
            });
        }
    }
}

fn render_fg_settings(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(gtr(gctx.locale, "fg.palette")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &pal in &PaletteId::ALL {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.palette.{pal:?}"),
                    label: pal.label(),
                    command_id: None,
                    selected: fg.palette == pal,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.palette = pal;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        for idx in 0..fg.palette.swatches().len().min(8) {
            let c = fg.preview_swatch(idx);
            let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(2), rgba_to_color32(c));
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("CVD Accessibility Preview").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, cvd) in [
            ("none", "Normal Vision", None),
            ("protan", "Protanopia", Some(CvdKind::Protanopia)),
            ("deutan", "Deuteranopia", Some(CvdKind::Deuteranopia)),
            ("tritan", "Tritanopia", Some(CvdKind::Tritanopia)),
            ("achroma", "Grayscale", Some(CvdKind::Achromatopsia)),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.cvd.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fg.cvd_preview == cvd,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fg.clone();
                next.cvd_preview = cvd;
                gctx.intents.push_update_format_graph_state(next);
            }
        }
    });

    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

fn render_fg_annotations(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    ui.label(egui::RichText::new(gtr(gctx.locale, "fg.annotations.heading")).strong());
    ui.add_space(4.0);
    let has_eq = fg.model_equation_mode.is_some();
    if has_eq {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label(gtr(gctx.locale, "fg.equation_mode"));
            for (slug, lbl, mode) in
                [("substituted", "Substituted", EquationDisplayMode::Substituted), ("symbolic", "Symbolic", EquationDisplayMode::Symbolic)]
            {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("format_graph.equation.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fg.model_equation_mode == Some(mode),
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    next.set_model_equation_mode(mode);
                    gctx.intents.push_update_format_graph_state(next);
                }
            }
        });
    } else {
        ui.label(gtr(gctx.locale, "fg.annotations.empty"));
    }
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

fn render_fg_mv_axis_variables(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let Some(mut enc) = fg
        .mv_encoding
        .clone()
        .or_else(|| fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate encoding attached to this graph.");
        return;
    };

    let var_names = collect_mv_variable_names(gctx.app, fg.graph_sheet_id.as_ref());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("X Variable: col {} ({})", enc.x_column + 1, var_name(&var_names, enc.x_column)));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.x_next",
                label: "Next X Col",
                command_id: None,
                selected: false,
                enabled: !var_names.is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && !var_names.is_empty()
        {
            enc.x_column = (enc.x_column + 1) % var_names.len();
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        ui.label(format!("Y Variable: col {} ({})", enc.y_column + 1, var_name(&var_names, enc.y_column)));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.y_next",
                label: "Next Y Col",
                command_id: None,
                selected: false,
                enabled: !var_names.is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
            && !var_names.is_empty()
        {
            enc.y_column = (enc.y_column + 1) % var_names.len();
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Color Channel Mapping").strong());
    let is_cat = matches!(enc.color, ColorChannelMapping::Categorical { .. });
    let is_cont = matches!(enc.color, ColorChannelMapping::Continuous { .. });
    let is_const = matches!(enc.color, ColorChannelMapping::Constant(_));
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_constant",
                label: "Uniform",
                command_id: None,
                selected: is_const,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.color = ColorChannelMapping::Constant(fg.palette.color_at(0));
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_categorical",
                label: "Categorical",
                command_id: None,
                selected: is_cat,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let col = enc.x_column;
            enc.color = ColorChannelMapping::Categorical { column: col, palette: fg.palette, level_order_override: None };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.color_continuous",
                label: "Continuous Ramp",
                command_id: None,
                selected: is_cont,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let col = enc.y_column;
            enc.color = ColorChannelMapping::Continuous { column: col, ramp: ColorRamp::default(), domain_clamp: None };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    if !fg.mv_levels.is_empty() {
        ui.add_space(6.0);
        ui.label(format!("Categorical Levels ({}): {}", if fg.mv_level_is_x { "X Axis" } else { "Color" }, fg.mv_levels.join(", ")));
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (wid, op, lbl) in [
                ("format_graph.mv.level_cycle", "cycle", "Next Level"),
                ("format_graph.mv.level_top", "top", "Top"),
                ("format_graph.mv.level_up", "up", "Move Up"),
                ("format_graph.mv.level_down", "down", "Move Down"),
                ("format_graph.mv.level_end", "end", "End"),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry { widget_id: wid, label: lbl, command_id: None, selected: false, enabled: true, disabled_reason: None },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fg.clone();
                    if op == "cycle" {
                        next.cycle_selected_mv_level();
                    } else {
                        let _ = next.reorder_mv_level(op);
                    }
                    gctx.intents.push_update_format_graph_state(next);
                }
            }
        });
    }
}

fn render_fg_mv_data_objects(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let Some(mut enc) = fg
        .mv_encoding
        .clone()
        .or_else(|| fg.graph_sheet_id.as_ref().and_then(|id| gctx.app.document().graph_sheet(id)).and_then(|gs| gs.spec.multivariate.clone()))
    else {
        ui.label("No Multivariate data layers attached.");
        return;
    };

    ui.label(egui::RichText::new("Layer Overlays").strong());
    let mut ellipses = enc.show_confidence_ellipses;
    let ell_chk = ui.checkbox(&mut ellipses, "Confidence Ellipses (95%)");
    record_checkbox(ui, &ell_chk, gctx.meta, "format_graph.mv.confidence_ellipses", "Confidence Ellipses (95%)", ellipses);
    if ell_chk.changed() {
        enc.show_confidence_ellipses = ellipses;
        push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
    }

    let mut hulls = enc.show_convex_hulls;
    let hull_chk = ui.checkbox(&mut hulls, "Convex Hulls");
    record_checkbox(ui, &hull_chk, gctx.meta, "format_graph.mv.convex_hulls", "Convex Hulls", hulls);
    if hull_chk.changed() {
        enc.show_convex_hulls = hulls;
        push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Symbol Sizing Mode").strong());
    let is_var = matches!(enc.size, SizeChannelMapping::Variable { .. });
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.size_uniform",
                label: "Uniform Diameter",
                command_id: None,
                selected: !is_var,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.size = SizeChannelMapping::Constant { radius_pt: 5.0 };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.size_variable",
                label: "Continuous Variable",
                command_id: None,
                selected: is_var,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            enc.size = SizeChannelMapping::Variable { column: enc.y_column, min_radius_pt: 2.5, max_radius_pt: 14.0 };
            push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
        }
    });

    ui.add_space(4.0);
    match enc.size {
        SizeChannelMapping::Constant { radius_pt } => {
            let cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.constant_radius", "Symbol Radius", 1.0, 24.0)
            };
            let out = render_scrub_unit_field(ui, radius_pt, &mut state.mv_min_radius_spinner, cfg, gctx.tokens, gctx.meta);
            if let Some(val) = out.committed_value {
                enc.size = SizeChannelMapping::Constant { radius_pt: val.clamp(1.0, 24.0) };
                push_mv_encoding(gctx.intents, fg, enc);
            }
        }
        SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } => {
            let min_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.min_radius", "Min Radius", 1.0, 24.0)
            };
            let min_out = render_scrub_unit_field(ui, min_radius_pt, &mut state.mv_min_radius_spinner, min_cfg, gctx.tokens, gctx.meta);
            if let Some(min_val) = min_out.committed_value {
                let lo = min_val.clamp(1.0, 24.0);
                enc.size = SizeChannelMapping::Variable { column, min_radius_pt: lo, max_radius_pt: max_radius_pt.max(lo) };
                push_mv_encoding(gctx.intents, fg.clone(), enc.clone());
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("format_graph.mv.max_radius", "Max Radius", 1.0, 36.0)
            };
            let max_out = render_scrub_unit_field(ui, max_radius_pt, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(max_val) = max_out.committed_value {
                let hi = max_val.clamp(min_radius_pt, 36.0);
                enc.size = SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt: hi };
                push_mv_encoding(gctx.intents, fg, enc);
            }
        }
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Out-of-Range Policy:");
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_clamp",
                label: "Clamp to Bounds",
                command_id: None,
                selected: state.mv_out_of_range == MvOutOfRangePolicy::ClampToBounds,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.mv_out_of_range = MvOutOfRangePolicy::ClampToBounds;
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "format_graph.mv.oor_omit",
                label: "Omit Outliers",
                command_id: None,
                selected: state.mv_out_of_range == MvOutOfRangePolicy::Omit,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.mv_out_of_range = MvOutOfRangePolicy::Omit;
        }
    });
}

fn render_fg_legends(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    render_fg_settings(ui, gctx);
    ui.add_space(4.0);
    let _ = fg;
}

fn render_fg_heatmap_color_mapping(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = &gctx.app.canvas_dialogs.format_graph;
    ui.label(egui::RichText::new("Sequential & Diverging Colormaps").strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for &cmap in &ColormapId::ALL {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.cmap.{cmap:?}"),
                    label: cmap.label(),
                    command_id: None,
                    selected: state.heatmap_colormap == cmap,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.heatmap_colormap = cmap;
            }
        }
    });

    ui.add_space(6.0);
    // Live gradient ramp strip (with optional CVD simulation).
    let (ramp_rect, _) = ui.allocate_exact_size(Vec2::new(gctx.body_w.min(360.0), 18.0), Sense::hover());
    let samples = state.heatmap_colormap.sample_discrete(24);
    let n = samples.len().max(1);
    let sw = ramp_rect.width() / (n as f32);
    for (i, base_c) in samples.iter().enumerate() {
        let idx = if state.heatmap_reversed { n - 1 - i } else { i };
        let c = samples.get(idx).copied().unwrap_or(*base_c);
        let preview_c = match fg.cvd_preview {
            Some(kind) => simulate_cvd(c, kind, 1.0),
            None => c,
        };
        let cell = Rect::from_min_size(Pos2::new(ramp_rect.left() + (i as f32) * sw, ramp_rect.top()), Vec2::new(sw + 0.5, ramp_rect.height()));
        ui.painter().rect_filled(cell, CornerRadius::ZERO, rgba_to_color32(preview_c));
    }
    ui.painter().rect_stroke(ramp_rect, CornerRadius::same(2), Stroke::new(1.0, gctx.tokens.border), StrokeKind::Inside);

    ui.add_space(6.0);
    let rev_chk = ui.checkbox(&mut state.heatmap_reversed, "Reverse Colormap");
    record_checkbox(ui, &rev_chk, gctx.meta, "format_graph.heatmap.reversed", "Reverse Colormap", state.heatmap_reversed);
    let auto_chk = ui.checkbox(&mut state.heatmap_auto_range, "Automatic Domain Bounds");
    record_checkbox(ui, &auto_chk, gctx.meta, "format_graph.heatmap.auto_range", "Automatic Domain Bounds", state.heatmap_auto_range);

    if !state.heatmap_auto_range {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Min:");
            let min_resp = ui.add(egui::TextEdit::singleline(&mut state.heatmap_min_draft).desired_width(72.0));
            record_input(ui, &min_resp, gctx.meta, "format_graph.heatmap.min", "Min", &state.heatmap_min_draft, true);
            ui.label("Max:");
            let max_resp = ui.add(egui::TextEdit::singleline(&mut state.heatmap_max_draft).desired_width(72.0));
            record_input(ui, &max_resp, gctx.meta, "format_graph.heatmap.max", "Max", &state.heatmap_max_draft, true);
        });
    }
}

fn render_fg_heatmap_labels(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let ann_chk = ui.checkbox(&mut state.heatmap_annotate_cells, "Show Numeric Cell Annotations");
    record_checkbox(ui, &ann_chk, gctx.meta, "format_graph.heatmap.annotate_cells", "Show Numeric Cell Annotations", state.heatmap_annotate_cells);

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Tick Angle:");
        for (lbl, deg) in [("0 deg", 0.0_f64), ("45 deg", 45.0_f64), ("90 deg", 90.0_f64)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("format_graph.heatmap.angle.{}", deg as i32),
                    label: lbl,
                    command_id: None,
                    selected: (state.heatmap_tick_angle_deg - deg).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.heatmap_tick_angle_deg = deg;
            }
        }
    });
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

fn render_fg_heatmap_borders(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fg = gctx.app.canvas_dialogs.format_graph.clone();
    let gap_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.5,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("format_graph.heatmap.cell_gap", "Tile Gap", 0.0, 12.0)
    };
    let out = render_scrub_unit_field(ui, state.heatmap_cell_gap_pt, &mut state.heatmap_gap_spinner, gap_cfg, gctx.tokens, gctx.meta);
    if let Some(v) = out.committed_value {
        state.heatmap_cell_gap_pt = v.clamp(0.0, 12.0);
    }
    ui.add_space(6.0);
    render_legend_toggles(ui, gctx, &fg);
}

fn render_legend_toggles(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>, fg: &crate::dialogs_canvas::FormatGraphState) {
    let mut show_legend = fg.show_legend;
    let vis_lbl = gtr(gctx.locale, "fg.legend");
    let vis_chk = ui.checkbox(&mut show_legend, vis_lbl);
    record_checkbox(ui, &vis_chk, gctx.meta, "format_graph.show_legend", vis_lbl, show_legend);
    if vis_chk.changed() {
        let mut next = fg.clone();
        next.show_legend = show_legend;
        gctx.intents.push_update_format_graph_state(next);
    }

    let mut bordered = fg.legend_bordered;
    let brd_lbl = "Bordered Legend Box";
    let brd_chk = ui.checkbox(&mut bordered, brd_lbl);
    record_checkbox(ui, &brd_chk, gctx.meta, "format_graph.legend_bordered", brd_lbl, bordered);
    if brd_chk.changed() {
        let mut next = fg.clone();
        next.legend_bordered = bordered;
        gctx.intents.push_update_format_graph_state(next);
    }
}

fn collect_mv_variable_names(app: &UiApp, gid: Option<&SheetId>) -> Vec<String> {
    let Some(id) = gid else {
        return Vec::new();
    };
    let Some(gs) = app.document().graph_sheet(id) else {
        return Vec::new();
    };
    let Some(ds) = app.document().data_sheet(&gs.spec.primary_source_sheet_id) else {
        return Vec::new();
    };
    match &ds.archetype {
        TableArchetype::MultipleVariables(mv) => mv.variables.iter().map(|v| v.title.clone()).collect(),
        _ => Vec::new(),
    }
}

fn var_name(names: &[String], idx: usize) -> &str {
    names.get(idx).map_or("Var", String::as_str)
}

fn push_mv_encoding(intents: &mut PendingUiIntents, mut fg: crate::dialogs_canvas::FormatGraphState, enc: MultivariateGraphEncoding) {
    fg.mv_encoding = Some(enc);
    fg.mv_levels_dirty = true;
    intents.push_update_format_graph_state(fg);
}

// ============================================================================
// 3. Format Axes (+ Categorical Axis Guard & Additional Ticks Sub-Modal) (§8.11D)
// ============================================================================

fn render_format_axes_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    sync_format_axes_local_drafts(gctx.app, state);
    if state.additional_ticks.open {
        render_additional_ticks_submodal(ui, state, gctx);
        return;
    }

    match state.format_axes_tab {
        FormatAxesTab::FrameAndOrigin => render_axes_frame_origin_tab(ui, state, gctx),
        FormatAxesTab::XAxis | FormatAxesTab::LeftYAxis | FormatAxesTab::RightYAxis => render_axes_single_axis_tab(ui, state, gctx),
        FormatAxesTab::TitlesAndTypography => render_axes_titles_tab(ui, state, gctx),
    }
}

fn sync_format_axes_local_drafts(app: &UiApp, state: &mut GraphModalEguiState) {
    let fa = &app.canvas_dialogs.format_axes;
    let sheet_key = fa.graph_sheet_id.as_ref().map(|s| s.as_str().to_owned());
    let changed = state.format_axes_last_sheet_id != sheet_key || state.format_axes_last_target != Some(fa.target_axis);
    if !changed {
        return;
    }
    state.format_axes_last_sheet_id = sheet_key;
    state.format_axes_last_target = Some(fa.target_axis);
    state.format_axes_title_draft = fa.axis_draft.title.clone();
    if let Some(gs) = fa.graph_sheet_id.as_ref().and_then(|id| app.document().graph_sheet(id)) {
        state.format_axes_frame_preset = gs.spec.frame_preset;
    }
    state.axis_min_spinner.draft_text.clear();
    state.axis_min_spinner.editing = false;
    state.axis_max_spinner.draft_text.clear();
    state.axis_max_spinner.editing = false;
    state.axis_gap_spinner.draft_text.clear();
    state.axis_gap_spinner.editing = false;
    state.axis_stroke_spinner.draft_text.clear();
    state.axis_stroke_spinner.editing = false;
    state.categorical_guard_confirmed = false;
    state.categorical_guard_pending_axis = None;
    state.format_axes_guard_pending_scale = None;
    state.format_axes_guard_pending_tick_format = None;
}

fn render_axes_frame_origin_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.frame.heading")).strong());
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, preset) in [
            ("open_l", "Open L-Frame", FramePreset::OpenL),
            ("plain_boxed", "Plain Boxed", FramePreset::PlainBoxed),
            ("tufte", "Offset Range (Tufte)", FramePreset::OffsetRangeTufte),
            ("dual_y", "Dual Y (Left + Right Y2)", FramePreset::DualYSecondaryRight),
            ("origin", "Origin Crossing (0, 0)", FramePreset::OriginCrossing),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.frame.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: state.format_axes_frame_preset == preset,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                state.format_axes_frame_preset = preset;
                if preset == FramePreset::DualYSecondaryRight && !fa.enable_y2 {
                    let mut next = fa.clone();
                    next.enable_y2 = true;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(8.0);
    let mut enable_y2 = fa.enable_y2;
    let y2_lbl = "Enable Secondary Right Y2 Axis";
    let chk = ui.checkbox(&mut enable_y2, y2_lbl);
    record_checkbox(ui, &chk, gctx.meta, "axes.enable_y2", y2_lbl, enable_y2);
    if chk.changed() {
        let mut next = fa;
        next.enable_y2 = enable_y2;
        if enable_y2 {
            state.format_axes_frame_preset = FramePreset::DualYSecondaryRight;
        }
        gctx.intents.push_update_format_axes_state(next);
    }
}

fn render_axes_single_axis_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    if fa.target_axis == AxisTarget::Y2 && !fa.enable_y2 {
        ui.label(egui::RichText::new("Secondary Right Y2 axis is currently disabled.").color(gctx.tokens.text_muted));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.enable_y2",
                label: "Enable Right Y2 Axis",
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa;
            next.enable_y2 = true;
            state.format_axes_frame_preset = FramePreset::DualYSecondaryRight;
            gctx.intents.push_update_format_axes_state(next);
        }
        return;
    }

    let is_categorical = matches!(fa.axis_draft.tick_format, TickLabelFormat::Categorical(_));
    if is_categorical {
        render_categorical_tick_controls(ui, state, gctx, &fa);
        ui.add_space(6.0);
    }

    if state.format_axes_guard_pending_scale.is_some() || state.format_axes_guard_pending_tick_format.is_some() {
        render_categorical_guard_banner(ui, state, gctx, &fa);
        ui.add_space(6.0);
    }

    // Scale selector with Categorical Guard intercept.
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.scale.heading")).strong());
    let cur_scale = fa.axis_draft.segments.first().map_or(GraphAxisScale::Linear, |s| s.scale);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, scale) in [
            ("linear", "Linear", GraphAxisScale::Linear),
            ("log10", "Log10", GraphAxisScale::Log10),
            ("log2", "Log2", GraphAxisScale::Log2),
            ("ln", "Ln", GraphAxisScale::Ln),
            ("logit", "Logit", GraphAxisScale::Logit(LogitDomain::Percent)),
            ("probit", "Probit", GraphAxisScale::Probit),
            ("reciprocal", "Reciprocal", GraphAxisScale::Reciprocal),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.scale.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: cur_scale == scale,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                if is_categorical {
                    state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
                    state.format_axes_guard_pending_scale = Some(scale);
                    state.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
                    state.categorical_guard_confirmed = false;
                } else {
                    let mut next = fa.clone();
                    let _ = next.set_segment_scale(0, scale);
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(6.0);
    // Range & Discontinuous Segments (1..=3).
    render_axis_segments_section(ui, state, gctx, &fa);

    ui.add_space(6.0);
    // Numbering / Ticks + Additional Ticks Sub-Modal button.
    render_axis_ticks_section(ui, state, gctx, &fa, is_categorical);
}

fn render_categorical_tick_controls(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let cur_source = match fa.axis_draft.tick_format {
        TickLabelFormat::Categorical(src) => src,
        _ => CategoricalTickSource::RowTitles,
    };
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.categorical.heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, src) in [
            ("row_titles", "Row Titles", CategoricalTickSource::RowTitles),
            ("column_titles", "Column Titles", CategoricalTickSource::ColumnTitles),
            ("row_numbers", "Row Numbers", CategoricalTickSource::RowNumbers),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_source.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: cur_source == src,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fa.clone();
                next.axis_draft.tick_format = TickLabelFormat::Categorical(src);
                gctx.intents.push_update_format_axes_state(next);
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.switch_to_continuous",
                label: gtr(gctx.locale, "axes.categorical.switch_btn"),
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
            state.format_axes_guard_pending_scale = Some(GraphAxisScale::Linear);
            state.format_axes_guard_pending_tick_format = Some(TickLabelFormat::Decimal);
            state.categorical_guard_confirmed = false;
        }
    });
}

fn render_categorical_guard_banner(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let warn_text = gtr(gctx.locale, "axes.guard.warning");
    egui::Frame::group(ui.style())
        .fill(gctx.tokens.header_bg)
        .stroke(Stroke::new(1.5, gctx.tokens.accent_strong))
        .corner_radius(CornerRadius::same(6))
        .show(ui, |ui| {
            ui.set_max_width((gctx.body_w - 16.0).max(180.0));
            let warn_resp = ui.add(egui::Label::new(egui::RichText::new(warn_text).color(gctx.tokens.link_red).strong()).wrap());
            record_label(ui, &warn_resp, gctx.meta, "axes.categorical_guard.warning", warn_text);

            let confirm_chk_lbl = gtr(gctx.locale, "axes.guard.confirm_chk");
            let chk = ui.checkbox(&mut state.categorical_guard_confirmed, confirm_chk_lbl);
            record_checkbox(ui, &chk, gctx.meta, "axes.categorical_guard.confirm_checkbox", confirm_chk_lbl, state.categorical_guard_confirmed);

            ui.horizontal_wrapped(|ui| {
                let can_confirm = state.categorical_guard_confirmed;
                let proceed_lbl = gtr(gctx.locale, "axes.guard.confirm_btn");
                let disabled_reason = (!can_confirm).then_some("Check confirmation above to switch categorical axis to numeric scale");
                let proceed = record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "axes.categorical_guard.confirm_btn",
                        label: proceed_lbl,
                        command_id: None,
                        selected: can_confirm,
                        enabled: can_confirm,
                        disabled_reason,
                    },
                    gctx.tokens,
                );
                if proceed.clicked() && can_confirm {
                    let mut next = fa.clone();
                    next.axis_draft.tick_format = state.format_axes_guard_pending_tick_format.unwrap_or(TickLabelFormat::Decimal);
                    if let Some(scale) = state.format_axes_guard_pending_scale {
                        let _ = next.set_segment_scale(0, scale);
                    }
                    gctx.intents.push_update_format_axes_state(next);
                    state.categorical_guard_confirmed = false;
                    state.categorical_guard_pending_axis = None;
                    state.format_axes_guard_pending_scale = None;
                    state.format_axes_guard_pending_tick_format = None;
                }

                let abort_lbl = gtr(gctx.locale, "axes.guard.cancel_btn");
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: "axes.categorical_guard.cancel_btn",
                        label: abort_lbl,
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    state.categorical_guard_confirmed = false;
                    state.categorical_guard_pending_axis = None;
                    state.format_axes_guard_pending_scale = None;
                    state.format_axes_guard_pending_tick_format = None;
                }
            });
        });
}

fn render_axis_segments_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
) {
    let seg0 = fa.axis_draft.segments.first().cloned().unwrap_or_default();
    let cur_min = seg0.min.unwrap_or(0.0);
    let cur_max = seg0.max.unwrap_or(100.0);

    let min_cfg = ScrubUnitFieldConfig {
        step_per_pt: 1.0,
        decimals: 3,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("axes.seg.0.min", gtr(gctx.locale, "axes.min"), -1.0e9, 1.0e9)
    };
    let min_out = render_scrub_unit_field(ui, cur_min, &mut state.axis_min_spinner, min_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = min_out.committed_value {
        let mut next = fa.clone();
        if val < cur_max {
            let _ = next.set_segment_bounds(0, Some(val), seg0.max);
            gctx.intents.push_update_format_axes_state(next);
        }
    }

    let max_cfg = ScrubUnitFieldConfig {
        step_per_pt: 1.0,
        decimals: 3,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("axes.seg.0.max", gtr(gctx.locale, "axes.max"), -1.0e9, 1.0e9)
    };
    let max_out = render_scrub_unit_field(ui, cur_max, &mut state.axis_max_spinner, max_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = max_out.committed_value {
        let mut next = fa.clone();
        if cur_min < val {
            let _ = next.set_segment_bounds(0, seg0.min, Some(val));
            gctx.intents.push_update_format_axes_state(next);
        }
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Segments: {}", fa.axis_draft.segments.len()));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.add",
                label: "+ Add Break Segment",
                command_id: None,
                selected: false,
                enabled: fa.axis_draft.segments.len() < 3,
                disabled_reason: (fa.axis_draft.segments.len() >= 3).then_some("Maximum 3 axis segments supported"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa.clone();
            if next.add_segment() {
                gctx.intents.push_update_format_axes_state(next);
            }
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "axes.segment.remove",
                label: "- Remove Segment",
                command_id: None,
                selected: false,
                enabled: fa.axis_draft.segments.len() > 1,
                disabled_reason: (fa.axis_draft.segments.len() <= 1).then_some("At least 1 segment required"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = fa.clone();
            let last = next.axis_draft.segments.len().saturating_sub(1);
            if next.remove_segment(last) {
                gctx.intents.push_update_format_axes_state(next);
            }
        }
    });

    if fa.axis_draft.segments.len() >= 2 {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Break Style:");
            for (slug, lbl, orn) in [
                ("slash", "// Double Slash", BreakOrnament::DoubleSlash),
                ("zigzag", "Zigzag", BreakOrnament::Zigzag),
                ("blank", "Blank Gap", BreakOrnament::BlankGap),
            ] {
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.break_ornament.{slug}"),
                        label: lbl,
                        command_id: None,
                        selected: fa.axis_draft.break_ornament == orn,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.break_ornament = orn;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        });
        if let Some(val) = render_length_spinner_pt(
            ui,
            LengthSpinnerSpec {
                widget_id: "axes.gap_width_pt",
                label: "Break Gap Width",
                value_pt: fa.axis_draft.gap_width_pt,
                min_pt: 2.0,
                max_pt: 48.0,
                step_pt: 0.5,
                enabled: true,
                disabled_reason: None,
            },
            &mut state.axis_gap_spinner,
            gctx.tokens,
            gctx.meta,
        ) {
            let mut next = fa.clone();
            next.axis_draft.gap_width_pt = val;
            gctx.intents.push_update_format_axes_state(next);
        }
    }
}

fn render_axis_ticks_section(
    ui: &mut Ui,
    state: &mut GraphModalEguiState,
    gctx: &mut GraphModalCtx<'_>,
    fa: &crate::dialogs_canvas::FormatAxesState,
    is_categorical: bool,
) {
    ui.label(egui::RichText::new(gtr(gctx.locale, "axes.ticks.heading")).strong());
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        for (slug, lbl, fmt) in [
            ("decimal", "Decimal", TickLabelFormat::Decimal),
            ("power10", "10^x", TickLabelFormat::PowerOfTenSuperscript),
            ("sci_e", "1e-3", TickLabelFormat::ScientificE),
            ("antilog", "Antilog", TickLabelFormat::Antilog),
            ("si", "SI Prefix", TickLabelFormat::EngineeringSi),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_format.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: fa.axis_draft.tick_format == fmt,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                if is_categorical {
                    state.categorical_guard_pending_axis = Some(format!("{:?}", fa.target_axis));
                    state.format_axes_guard_pending_scale = Some(GraphAxisScale::Linear);
                    state.format_axes_guard_pending_tick_format = Some(fmt);
                    state.categorical_guard_confirmed = false;
                } else {
                    let mut next = fa.clone();
                    next.axis_draft.tick_format = fmt;
                    gctx.intents.push_update_format_axes_state(next);
                }
            }
        }
    });

    ui.add_space(6.0);
    let tick_count = fa.axis_draft.additional_ticks.len();
    let thresh_count = fa.axis_draft.thresholds.len();
    let add_btn_lbl = format!("{} ({} / {})", gtr(gctx.locale, "axes.add_tick.open"), tick_count, thresh_count);
    if record_button(
        ui,
        gctx.meta,
        ButtonEntry {
            widget_id: "axes.additional_ticks.open",
            label: &add_btn_lbl,
            command_id: None,
            selected: false,
            enabled: true,
            disabled_reason: None,
        },
        gctx.tokens,
    )
    .clicked()
    {
        state.additional_ticks.open = true;
        state.additional_tick_status = None;
    }
}

fn render_axes_titles_tab(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let title_lbl = gtr(gctx.locale, "axes.title_label");
        ui.label(title_lbl);
        let title_edit = ui.add(egui::TextEdit::singleline(&mut state.format_axes_title_draft).desired_width((gctx.body_w - 120.0).max(120.0)));
        record_input(ui, &title_edit, gctx.meta, "axes.title", title_lbl, &state.format_axes_title_draft, true);
        if title_edit.lost_focus() || title_edit.changed() {
            let mut next = fa.clone();
            next.axis_draft.title = state.format_axes_title_draft.clone();
            gctx.intents.push_update_format_axes_state(next);
        }
    });

    ui.add_space(6.0);
    if let Some(val) = render_length_spinner_pt(
        ui,
        LengthSpinnerSpec {
            widget_id: "axes.stroke_width_pt",
            label: "Axis Spine Width",
            value_pt: fa.axis_draft.stroke_width_pt,
            min_pt: 0.25,
            max_pt: 6.0,
            step_pt: 0.25,
            enabled: true,
            disabled_reason: None,
        },
        &mut state.axis_stroke_spinner,
        gctx.tokens,
        gctx.meta,
    ) {
        let mut next = fa.clone();
        next.axis_draft.stroke_width_pt = val;
        gctx.intents.push_update_format_axes_state(next);
    }

    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Tick Angle:");
        for deg in [0.0_f64, -45.0_f64, 45.0_f64, 90.0_f64] {
            let lbl = format!("{} deg", deg as i32);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("axes.tick_angle.{}", deg as i32),
                    label: &lbl,
                    command_id: None,
                    selected: (fa.axis_draft.tick_angle_deg - deg).abs() < 0.1,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = fa.clone();
                next.axis_draft.tick_angle_deg = deg;
                gctx.intents.push_update_format_axes_state(next);
            }
        }
    });
}

fn render_additional_ticks_submodal(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fa = gctx.app.canvas_dialogs.format_axes.clone();
    ui.label(egui::RichText::new("Custom Ticks & Reference Thresholds").strong());
    ui.add_space(4.0);

    // Existing custom ticks table.
    if fa.axis_draft.additional_ticks.is_empty() {
        ui.label(egui::RichText::new("No additional ticks defined on this axis.").color(gctx.tokens.text_muted));
    } else {
        for (idx, tick) in fa.axis_draft.additional_ticks.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let label_part = tick.label.as_deref().unwrap_or("(auto)");
                let grid_part = if tick.draw_gridline { " + grid" } else { "" };
                ui.label(format!("Tick #{}: {:.4} [{}]{}", idx + 1, tick.value, label_part, grid_part));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.add_tick.delete.{idx}"),
                        label: "Delete",
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.additional_ticks.remove(idx);
                    push_additional_ticks_update(gctx.intents, &next);
                    gctx.intents.push_update_format_axes_state(next);
                }
            });
        }
    }

    if !fa.axis_draft.thresholds.is_empty() {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Thresholds & Shaded Zones:").strong());
        for (idx, th) in fa.axis_draft.thresholds.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(gctx.body_w);
                let span = match th.shade_to {
                    Some(end) => format!("{:.3}..{:.3} (shaded)", th.value, end),
                    None => format!("{:.3} (line)", th.value),
                };
                ui.label(format!("Threshold #{}: {span} {}", idx + 1, th.label.as_deref().unwrap_or("")));
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("axes.threshold.delete.{idx}"),
                        label: "Delete",
                        command_id: None,
                        selected: false,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut next = fa.clone();
                    next.axis_draft.thresholds.remove(idx);
                    gctx.intents.push_update_format_axes_state(next);
                }
            });
        }
    }

    ui.add_space(6.0);
    ui.separator();
    ui.add_space(6.0);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Value:");
        let v_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.value_draft).desired_width(86.0));
        record_input(ui, &v_resp, gctx.meta, "axes.add_tick.value", "Value", &state.additional_ticks.value_draft, true);
        ui.label("Label:");
        let l_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.label_draft).desired_width(120.0));
        record_input(ui, &l_resp, gctx.meta, "axes.add_tick.label", "Label", &state.additional_ticks.label_draft, true);
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let grid_chk = ui.checkbox(&mut state.additional_tick_draw_gridline, "Draw Gridline");
        record_checkbox(ui, &grid_chk, gctx.meta, "axes.add_tick.gridline", "Draw Gridline", state.additional_tick_draw_gridline);

        let line_chk = ui.checkbox(&mut state.additional_ticks.show_line, "Show Threshold Rule");
        record_checkbox(ui, &line_chk, gctx.meta, "axes.add_tick.show_line", "Show Threshold Rule", state.additional_ticks.show_line);

        let shade_chk = ui.checkbox(&mut state.additional_ticks.shaded_zone, "Shaded Zone Band");
        record_checkbox(ui, &shade_chk, gctx.meta, "axes.add_tick.shaded_zone", "Shaded Zone Band", state.additional_ticks.shaded_zone);
    });

    if state.additional_ticks.shaded_zone {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("End Value (Shade To):");
            let end_resp = ui.add(egui::TextEdit::singleline(&mut state.additional_ticks.end_value_draft).desired_width(86.0));
            record_input(ui, &end_resp, gctx.meta, "axes.add_tick.end_value", "End Value", &state.additional_ticks.end_value_draft, true);
        });
    }

    if let Some(err) = &state.additional_tick_status {
        ui.colored_label(gctx.tokens.link_red, err);
    }

    ui.add_space(6.0);
    if record_button(
        ui,
        gctx.meta,
        ButtonEntry {
            widget_id: "axes.add_tick.save",
            label: gtr(gctx.locale, "axes.add_tick.save"),
            command_id: None,
            selected: true,
            enabled: true,
            disabled_reason: None,
        },
        gctx.tokens,
    )
    .clicked()
    {
        match state.additional_ticks.value_draft.trim().parse::<f64>() {
            Ok(val) if val.is_finite() => {
                let label_opt = (!state.additional_ticks.label_draft.trim().is_empty()).then(|| state.additional_ticks.label_draft.trim().to_owned());
                let mut next = fa.clone();
                if state.additional_ticks.show_line || state.additional_ticks.shaded_zone {
                    let shade_to = if state.additional_ticks.shaded_zone {
                        match state.additional_ticks.end_value_draft.trim().parse::<f64>() {
                            Ok(end) if end.is_finite() => Some(end),
                            _ => {
                                state.additional_tick_status = Some(String::from("Enter a valid finite End Value for Shaded Zone"));
                                return;
                            }
                        }
                    } else {
                        None
                    };
                    next.axis_draft.thresholds.push(AxisThreshold {
                        value: val,
                        shade_to,
                        color: curvebook_palette::Rgba8::BLACK,
                        dash_style: LineDashStyle::Dashed,
                        stroke_width_pt: 1.0,
                        label: label_opt.clone(),
                    });
                }
                if !state.additional_ticks.shaded_zone {
                    next.axis_draft.additional_ticks.push(AdditionalTick {
                        value: val,
                        label: label_opt,
                        draw_gridline: state.additional_tick_draw_gridline,
                    });
                    push_additional_ticks_update(gctx.intents, &next);
                }
                gctx.intents.push_update_format_axes_state(next);
                state.additional_tick_status = None;
                state.additional_ticks.value_draft.clear();
                state.additional_ticks.end_value_draft.clear();
                state.additional_ticks.label_draft.clear();
            }
            _ => {
                state.additional_tick_status = Some(String::from("Enter a valid finite numeric value"));
            }
        }
    }
}

fn push_additional_ticks_update(intents: &mut PendingUiIntents, fa: &crate::dialogs_canvas::FormatAxesState) {
    let Some(gid) = &fa.graph_sheet_id else {
        return;
    };
    let axis_name = match fa.target_axis {
        AxisTarget::X => "x",
        AxisTarget::Y => "y",
        AxisTarget::Y2 => "y2",
    };
    intents.push_engine(
        "graph.setAdditionalTicks",
        json!({
            "sheet_id": gid.as_str(),
            "axis": axis_name,
            "ticks": fa.axis_draft.additional_ticks,
        }),
    );
}

// ============================================================================
// 4. Secondary Graph & Page Layout Modals (§8.11 & §8.12)
// ============================================================================

fn render_pairwise_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let pw = gctx.app.canvas_dialogs.pairwise.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "pairwise.auto_add",
                label: "Auto-Add Comparisons",
                command_id: None,
                selected: false,
                enabled: !pw.available_result_ids.is_empty(),
                disabled_reason: pw.available_result_ids.is_empty().then_some("Run pairwise statistical comparison first"),
            },
            gctx.tokens,
        )
        .clicked()
        {
            gctx.intents.push_dialog_widget_click("pairwise.auto_add");
        }
        let mut hide_ns = pw.hide_non_significant;
        let chk = ui.checkbox(&mut hide_ns, "Hide Non-Significant");
        record_checkbox(ui, &chk, gctx.meta, "pairwise.hide_non_significant", "Hide Non-Significant", hide_ns);
        if chk.changed() {
            let mut next = pw.clone();
            next.hide_non_significant = hide_ns;
            next.brackets_dirty = true;
            gctx.intents.push_update_pairwise_state(next);
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Display Mode:");
        for (lbl, mode) in [
            ("Brackets Only", PairwiseDisplayMode::BracketsOnly),
            ("CLD Only", PairwiseDisplayMode::CompactLetterDisplayOnly),
            ("Brackets + CLD", PairwiseDisplayMode::BracketsAndCompactLetterDisplay),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.mode.{mode:?}"),
                    label: lbl,
                    command_id: None,
                    selected: pw.display_mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.display_mode = mode;
                next.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
                gctx.intents.push_update_pairwise_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Bracket Style:");
        for (lbl, st) in [
            ("Square", BracketStyle::Square),
            ("Half Tick", BracketStyle::HalfTick),
            ("Flat Line", BracketStyle::FlatLine),
            ("Curly", BracketStyle::CurlyBrace),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.bracket.{st:?}"),
                    label: lbl,
                    command_id: None,
                    selected: pw.bracket_style == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.bracket_style = st;
                next.brackets_dirty = true;
                gctx.intents.push_update_pairwise_state(next);
            }
        }
        ui.label("Labels:");
        for (lbl, lm) in [("Stars", BracketLabelMode::SummaryStars), ("Numeric p/q", BracketLabelMode::NumericPOrQ)] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("pairwise.label.{lm:?}"),
                    label: lbl,
                    command_id: None,
                    selected: pw.label_mode == lm,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pw.clone();
                next.label_mode = lm;
                next.brackets_dirty = true;
                gctx.intents.push_update_pairwise_state(next);
            }
        }
    });

    if pw.cld_enabled {
        ui.add_space(4.0);
        let font_cfg = ScrubUnitFieldConfig {
            step_per_pt: 0.5,
            decimals: 1,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_font_size", "CLD Font Size", 6.0, 24.0)
        };
        let f_out = render_scrub_unit_field(ui, pw.cld_font_size_pt, &mut state.cld_font_spinner, font_cfg, gctx.tokens, gctx.meta);
        if let Some(v) = f_out.committed_value {
            let mut next = pw.clone();
            next.cld_font_size_pt = v.clamp(6.0, 24.0);
            gctx.intents.push_update_pairwise_state(next);
        }

        let off_cfg = ScrubUnitFieldConfig {
            step_per_pt: 1.0,
            decimals: 1,
            allow_unit_switch: false,
            ..ScrubUnitFieldConfig::new("pairwise.cld_offset", "CLD Offset", 0.0, 48.0)
        };
        let o_out = render_scrub_unit_field(ui, pw.cld_offset_pt, &mut state.cld_offset_spinner, off_cfg, gctx.tokens, gctx.meta);
        if let Some(v) = o_out.committed_value {
            let mut next = pw.clone();
            next.cld_offset_pt = v.clamp(0.0, 48.0);
            gctx.intents.push_update_pairwise_state(next);
        }

        let mut upper = pw.cld_uppercase;
        let chk = ui.checkbox(&mut upper, "Uppercase CLD Letters (A, AB)");
        record_checkbox(ui, &chk, gctx.meta, "pairwise.cld_uppercase", "Uppercase CLD Letters", upper);
        if chk.changed() {
            let mut next = pw.clone();
            next.cld_uppercase = upper;
            gctx.intents.push_update_pairwise_state(next);
        }
        let desc = pw.cld_ordering == CldOrdering::DescendingMean;
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "pairwise.cld_ordering",
                label: if desc { "Order: Descending Mean" } else { "Order: Ascending Mean" },
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            gctx.intents.push_dialog_widget_click("pairwise.cld_ordering");
        }
    }
    ui.add_space(4.0);
    ui.label(format!("Active brackets: {}", pw.brackets.len()));
}

fn render_bubble_plot_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let bp = gctx.app.canvas_dialogs.bubble_plot.clone();
    let n_cols = bp.available_columns.len().max(1);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("X Column: #{} | Y Column: #{}", bp.x_column + 1, bp.y_column + 1));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.x_next",
                label: "Cycle X",
                command_id: None,
                selected: false,
                enabled: n_cols > 1,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            next.x_column = (next.x_column + 1) % n_cols;
            gctx.intents.push_update_bubble_plot_state(next);
        }
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "bubble.y_next",
                label: "Cycle Y",
                command_id: None,
                selected: false,
                enabled: n_cols > 1,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = bp.clone();
            next.y_column = (next.y_column + 1) % n_cols;
            gctx.intents.push_update_bubble_plot_state(next);
        }
    });

    ui.add_space(4.0);
    match bp.size_channel {
        SizeChannelMapping::Constant { radius_pt } => {
            let cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.size_pt", "Bubble Radius", 1.0, 24.0)
            };
            let out = render_scrub_unit_field(ui, radius_pt, &mut state.mv_min_radius_spinner, cfg, gctx.tokens, gctx.meta);
            if let Some(v) = out.committed_value {
                let mut next = bp.clone();
                next.size_channel = SizeChannelMapping::Constant { radius_pt: v.clamp(1.0, 24.0) };
                gctx.intents.push_update_bubble_plot_state(next);
            }
        }
        SizeChannelMapping::Variable { column, min_radius_pt, max_radius_pt } => {
            let min_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.min_radius", "Min Radius", 1.0, 24.0)
            };
            let min_out = render_scrub_unit_field(ui, min_radius_pt, &mut state.mv_min_radius_spinner, min_cfg, gctx.tokens, gctx.meta);
            if let Some(v) = min_out.committed_value {
                let mut next = bp.clone();
                next.set_size_variable(column, v, max_radius_pt);
                gctx.intents.push_update_bubble_plot_state(next);
            }

            let max_cfg = ScrubUnitFieldConfig {
                step_per_pt: 0.5,
                decimals: 1,
                allow_unit_switch: false,
                ..ScrubUnitFieldConfig::new("bubble.max_radius", "Max Radius", 1.0, 36.0)
            };
            let max_out = render_scrub_unit_field(ui, max_radius_pt, &mut state.mv_max_radius_spinner, max_cfg, gctx.tokens, gctx.meta);
            if let Some(v) = max_out.committed_value {
                let mut next = bp.clone();
                next.set_size_variable(column, min_radius_pt, v);
                gctx.intents.push_update_bubble_plot_state(next);
            }
        }
    }

    ui.add_space(4.0);
    let mut hulls = bp.show_convex_hulls;
    let h_chk = ui.checkbox(&mut hulls, "Show Convex Hulls");
    record_checkbox(ui, &h_chk, gctx.meta, "bubble.show_convex_hulls", "Show Convex Hulls", hulls);
    if h_chk.changed() {
        gctx.intents.push_dialog_widget_click("bubble.show_convex_hulls");
    }

    let mut ellipses = bp.show_confidence_ellipses;
    let e_chk = ui.checkbox(&mut ellipses, "Show Confidence Ellipses");
    record_checkbox(ui, &e_chk, gctx.meta, "bubble.show_confidence_ellipses", "Show Confidence Ellipses", ellipses);
    if e_chk.changed() {
        gctx.intents.push_dialog_widget_click("bubble.show_confidence_ellipses");
    }
}

fn render_apply_style_body(ui: &mut Ui, gctx: &mut GraphModalCtx<'_>) {
    let asf = &gctx.app.canvas_dialogs.apply_style;
    ui.label(egui::RichText::new("Source Graph Style Donor").strong());
    if asf.donor_candidates.is_empty() {
        ui.label(egui::RichText::new("Create another graph sheet to clone styling from.").color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            for (idx, (id, title)) in asf.donor_candidates.iter().enumerate() {
                let selected = asf.donor_graph_id.as_ref() == Some(id);
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("apply_style.donor.{idx}"),
                        label: title,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut patch = Map::new();
                    patch.insert(String::from("donor_graph_id"), Value::String(id.as_str().to_owned()));
                    gctx.intents.push_patch_active_dialog(patch);
                }
            }
        });
    }

    ui.add_space(6.0);
    for (wid, lbl, val) in [
        ("apply_style.clone_palette", "Clone Palette & Swatches", asf.clone_palette),
        ("apply_style.clone_frame_and_axes", "Clone Frame & Axis Styling", asf.clone_frame_and_axes),
        ("apply_style.clone_error_bars_and_legend", "Clone Error Bars & Legend", asf.clone_error_bars_and_legend),
    ] {
        let mut cur = val;
        let chk = ui.checkbox(&mut cur, lbl);
        record_checkbox(ui, &chk, gctx.meta, wid, lbl, cur);
        if chk.changed() {
            gctx.intents.push_dialog_widget_click(wid);
        }
    }
}

fn render_format_selected_points_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let fp = &gctx.app.canvas_dialogs.format_points;
    ui.label(format!("Selected points: {}", fp.selected_points.len()));
    ui.add_space(4.0);

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Color:");
        for (name, hex) in
            [("Black", "#111827"), ("Red", "#DC2626"), ("Blue", "#2563EB"), ("Green", "#16A34A"), ("Orange", "#EA580C"), ("Purple", "#7C3AED")]
        {
            let selected = fp.color.as_deref() == Some(hex);
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("fmt_pts.col.{name}"),
                    label: name,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("color"), Value::String(hex.to_owned()));
                patch.insert(String::from("fill_color"), Value::String(hex.to_owned()));
                gctx.intents.push_patch_active_dialog(patch);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Symbol:");
        for (lbl, shape) in [
            ("Circle", MarkerShape::Circle),
            ("Square", MarkerShape::Square),
            ("Triangle", MarkerShape::TriangleUp),
            ("Diamond", MarkerShape::Diamond),
        ] {
            let slug = format!("{shape:?}");
            let selected = fp.symbol.as_deref() == Some(slug.as_str());
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("fmt_pts.sym.{lbl}"),
                    label: lbl,
                    command_id: None,
                    selected,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("symbol"), Value::String(slug));
                gctx.intents.push_patch_active_dialog(patch);
            }
        }
    });

    ui.add_space(4.0);
    let size_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.5,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("fmt_pts.size", "Point Size", 2.0, 24.0)
    };
    let size_out = render_scrub_unit_field(ui, fp.size_pt.unwrap_or(5.0), &mut state.pts_size_spinner, size_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = size_out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("size_pt"), json!(val.clamp(2.0, 24.0)));
        gctx.intents.push_patch_active_dialog(patch);
    }

    let bw_cfg = ScrubUnitFieldConfig {
        step_per_pt: 0.25,
        decimals: 2,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("fmt_pts.border", "Border Width", 0.5, 6.0)
    };
    let bw_out = render_scrub_unit_field(ui, fp.border_width_pt.unwrap_or(1.0), &mut state.pts_border_spinner, bw_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = bw_out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("border_width_pt"), json!(val.clamp(0.5, 6.0)));
        gctx.intents.push_patch_active_dialog(patch);
    }

    ui.add_space(4.0);
    let mut hidden = fp.hidden.unwrap_or(false);
    let h_chk = ui.checkbox(&mut hidden, "Hide Selected Points");
    record_checkbox(ui, &h_chk, gctx.meta, "fmt_pts.hidden", "Hide Selected Points", hidden);
    if h_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("hidden"), Value::Bool(hidden));
        gctx.intents.push_patch_active_dialog(patch);
    }

    let mut clear = fp.clear_overrides;
    let c_chk = ui.checkbox(&mut clear, "Clear Point Overrides");
    record_checkbox(ui, &c_chk, gctx.meta, "fmt_pts.clear", "Clear Point Overrides", clear);
    if c_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("clear_overrides"), Value::Bool(clear));
        gctx.intents.push_patch_active_dialog(patch);
    }
}

fn render_insert_equation_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let eq = &gctx.app.canvas_dialogs.insert_model_equation;
    if eq.available_result_ids.is_empty() {
        ui.label(egui::RichText::new("No regression ResultSheet available to link.").color(gctx.tokens.text_muted));
    } else {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(gctx.body_w);
            ui.label("Regression Result:");
            for (idx, rid) in eq.available_result_ids.iter().enumerate() {
                let selected = eq.result_sheet_id.as_ref() == Some(rid);
                let title = gctx.app.document().result_sheet(rid).map_or_else(|| rid.as_str().to_owned(), |r| r.title().to_owned());
                if record_button(
                    ui,
                    gctx.meta,
                    ButtonEntry {
                        widget_id: &format!("eq.result.{idx}"),
                        label: &title,
                        command_id: None,
                        selected,
                        enabled: true,
                        disabled_reason: None,
                    },
                    gctx.tokens,
                )
                .clicked()
                {
                    let mut patch = Map::new();
                    patch.insert(String::from("result_sheet_id"), Value::String(rid.as_str().to_owned()));
                    gctx.intents.push_patch_active_dialog(patch);
                }
            }
        });
    }

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Display Mode:");
        for (lbl, mode, slug) in
            [("Substituted Values", EquationDisplayMode::Substituted, "substituted"), ("Symbolic Formula", EquationDisplayMode::Symbolic, "symbolic")]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("eq.mode.{slug}"),
                    label: lbl,
                    command_id: None,
                    selected: eq.mode == mode,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut patch = Map::new();
                patch.insert(String::from("mode"), Value::String(slug.to_owned()));
                gctx.intents.push_patch_active_dialog(patch);
            }
        }
    });

    ui.add_space(4.0);
    let font_cfg = ScrubUnitFieldConfig {
        step_per_pt: 1.0,
        decimals: 1,
        allow_unit_switch: false,
        ..ScrubUnitFieldConfig::new("eq.font_size", "Equation Font Size", 6.0, 36.0)
    };
    let out = render_scrub_unit_field(ui, eq.font_size_pt, &mut state.eq_font_spinner, font_cfg, gctx.tokens, gctx.meta);
    if let Some(val) = out.committed_value {
        let mut patch = Map::new();
        patch.insert(String::from("font_size_pt"), json!(val.clamp(6.0, 36.0)));
        gctx.intents.push_patch_active_dialog(patch);
    }

    let mut bordered = eq.bordered;
    let brd_chk = ui.checkbox(&mut bordered, "Draw Bordered Callout Box");
    record_checkbox(ui, &brd_chk, gctx.meta, "eq.bordered", "Draw Bordered Callout Box", bordered);
    if brd_chk.changed() {
        let mut patch = Map::new();
        patch.insert(String::from("bordered"), Value::Bool(bordered));
        gctx.intents.push_patch_active_dialog(patch);
    }
}

fn render_page_layout_body(ui: &mut Ui, state: &mut GraphModalEguiState, gctx: &mut GraphModalCtx<'_>) {
    let pl = gctx.app.canvas_dialogs.page_layout.clone();
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Journal Width Preset:");
        for (lbl, preset) in [
            ("85 mm", JournalWidthPreset::SingleColumn85Mm),
            ("120 mm", JournalWidthPreset::OneAndHalfColumn120Mm),
            ("174 mm", JournalWidthPreset::DoubleColumn174Mm),
            ("A4 Portrait", JournalWidthPreset::A4Portrait),
            ("A4 Landscape", JournalWidthPreset::A4Landscape),
        ] {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("layout.preset.{preset:?}"),
                    label: lbl,
                    command_id: None,
                    selected: pl.preset == preset,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pl.clone();
                next.apply_preset(preset);
                gctx.intents.push_update_page_layout_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label(format!("Grid: {} x {}", pl.rows, pl.cols));
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.arrange",
                label: "Auto-Arrange Graphs",
                command_id: None,
                selected: false,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = pl.clone();
            let gids: Vec<SheetId> = gctx.app.document().graph_sheets.iter().map(|g| g.id.clone()).collect();
            next.arrange_grid(&gids, 24.0, 12.0);
            gctx.intents.push_update_page_layout_state(next);
        }
        let orient_lbl = match pl.orientation {
            PageOrientation::Portrait => "Portrait",
            PageOrientation::Landscape => "Landscape",
        };
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.orientation",
                label: orient_lbl,
                command_id: None,
                selected: true,
                enabled: true,
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = pl.clone();
            next.orientation = match next.orientation {
                PageOrientation::Portrait => PageOrientation::Landscape,
                PageOrientation::Landscape => PageOrientation::Portrait,
            };
            gctx.intents.push_update_page_layout_state(next);
        }
    });

    ui.add_space(4.0);
    let mut align_origins = pl.align_plot_origins;
    let chk = ui.checkbox(&mut align_origins, "Align Plot Origins Across Grid");
    record_checkbox(ui, &chk, gctx.meta, "layout.align_plot_origins", "Align Plot Origins Across Grid", align_origins);
    if chk.changed() {
        let mut next = pl.clone();
        next.align_plot_origins = align_origins;
        gctx.intents.push_update_page_layout_state(next);
    }

    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        ui.label("Panel Badges:");
        for (lbl, st) in
            [("A, B, C", PanelBadgeStyle::UpperCaseLetters), ("a, b, c", PanelBadgeStyle::LowerCaseLetters), ("None", PanelBadgeStyle::None)]
        {
            if record_button(
                ui,
                gctx.meta,
                ButtonEntry {
                    widget_id: &format!("layout.badge.{st:?}"),
                    label: lbl,
                    command_id: None,
                    selected: pl.panel_badges == st,
                    enabled: true,
                    disabled_reason: None,
                },
                gctx.tokens,
            )
            .clicked()
            {
                let mut next = pl.clone();
                next.panel_badges = st;
                gctx.intents.push_update_page_layout_state(next);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(gctx.body_w);
        let edit = ui.add(egui::TextEdit::singleline(&mut state.page_callout_draft).desired_width((gctx.body_w - 130.0).max(110.0)));
        record_input(ui, &edit, gctx.meta, "layout.callout_input", "Callout text", &state.page_callout_draft, true);
        if record_button(
            ui,
            gctx.meta,
            ButtonEntry {
                widget_id: "layout.add_callout",
                label: "Add Callout",
                command_id: None,
                selected: false,
                enabled: !state.page_callout_draft.trim().is_empty(),
                disabled_reason: None,
            },
            gctx.tokens,
        )
        .clicked()
        {
            let mut next = pl.clone();
            next.items.push(LayoutItem::TextCallout {
                id: format!("callout-{}", next.items.len() + 1),
                markdown: state.page_callout_draft.trim().to_owned(),
                bounds_pt: RectPt::new(24.0, 24.0, 180.0, 28.0),
                font_size_pt: 11.0,
            });
            state.page_callout_draft.clear();
            gctx.intents.push_update_page_layout_state(next);
        }
    });
    ui.label(format!("Layout items: {}", pl.items.len()));
}

// ============================================================================
// Shared UI Helpers, Recording & 14-Locale Localized Labels
// ============================================================================

fn rgba_to_color32(c: Rgba8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

fn layout_elided(painter: &egui::Painter, text: &str, max_width: f32, color: Color32, font_size: f32) -> (std::sync::Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::single_section(text.to_owned(), egui::TextFormat::simple(FontId::proportional(font_size), color));
    job.wrap = egui::text::TextWrapping { max_width: max_width.max(8.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let galley = painter.layout_job(job);
    let elided = galley.elided;
    (galley, elided)
}

fn record_if_visible(ui: &Ui, resp: &Response, meta: &mut FrameInspectMeta, input: FrameInspectEntryInput<'_>) {
    let clipped = resp.interact_rect.intersect(ui.clip_rect());
    if clipped.width() > 0.0 && clipped.height() > 0.0 {
        meta.record_response(ui, resp, input);
    }
}

fn record_button(ui: &mut Ui, meta: &mut FrameInspectMeta, entry: ButtonEntry<'_>, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(entry.label)
        .fill(if entry.selected { tokens.active_bg } else { tokens.panel_bg })
        .stroke(Stroke::new(1.0, if entry.selected { tokens.accent_strong } else { tokens.border }));
    let mut resp = ui.add_enabled(entry.enabled, btn);
    if let Some(reason) = entry.disabled_reason {
        resp = resp.on_disabled_hover_text(reason);
    }
    record_if_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id: entry.widget_id,
            role: "button",
            label: entry.label,
            command_id: entry.command_id,
            value: Some(json!({ "selected": entry.selected })),
            enabled: entry.enabled,
            disabled_reason: entry.disabled_reason,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn record_tab(ui: &mut Ui, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, selected: bool, tokens: &Tokens) -> Response {
    let btn = egui::Button::new(label)
        .fill(if selected { tokens.active_bg } else { tokens.surface_bg })
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { tokens.accent_strong } else { tokens.border }));
    let resp = ui.add(btn);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    record_if_visible(
        ui,
        &resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "tab",
            label,
            command_id: None,
            value: Some(json!({ "selected": selected })),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
    resp
}

fn record_checkbox(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, checked: bool) {
    record_if_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "checkbox",
            label,
            command_id: None,
            value: Some(Value::Bool(checked)),
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn record_input(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, label: &str, text: &str, enabled: bool) {
    record_if_visible(
        ui,
        resp,
        meta,
        FrameInspectEntryInput {
            widget_id,
            role: "textbox",
            label,
            command_id: None,
            value: Some(Value::String(text.to_owned())),
            enabled,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

pub(super) struct LengthSpinnerSpec<'a> {
    pub widget_id: &'a str,
    pub label: &'a str,
    pub value_pt: f64,
    pub min_pt: f64,
    pub max_pt: f64,
    pub step_pt: f64,
    pub enabled: bool,
    pub disabled_reason: Option<&'a str>,
}

pub(super) fn render_length_spinner_pt(
    ui: &mut Ui,
    spec: LengthSpinnerSpec<'_>,
    state: &mut ScrubUnitFieldState,
    tokens: &Tokens,
    meta: &mut FrameInspectMeta,
) -> Option<f64> {
    let unit = if state.unit.is_length() { state.unit } else { ScrubUnitKind::Points };
    let disp_in = unit.from_points(spec.value_pt);
    let disp_min = unit.from_points(spec.min_pt);
    let disp_max = unit.from_points(spec.max_pt);
    let step_disp = unit.from_points(spec.step_pt).max(0.001);
    let dec = match unit {
        ScrubUnitKind::Millimeters => 2,
        ScrubUnitKind::Inches => 3,
        _ => 1,
    };
    let cfg = ScrubUnitFieldConfig {
        step_per_pt: step_disp,
        decimals: dec,
        enabled: spec.enabled,
        disabled_reason: spec.disabled_reason,
        allow_unit_switch: true,
        ..ScrubUnitFieldConfig::new(spec.widget_id, spec.label, disp_min, disp_max)
    };
    let out = render_scrub_unit_field(ui, disp_in, state, cfg, tokens, meta);
    out.committed_value.map(|v| out.display_unit.to_points(v).clamp(spec.min_pt, spec.max_pt))
}

fn record_label(ui: &mut Ui, resp: &Response, meta: &mut FrameInspectMeta, widget_id: &str, text: &str) {
    meta.record_response(
        ui,
        resp,
        FrameInspectEntryInput {
            widget_id,
            role: "text",
            label: text,
            command_id: None,
            value: None,
            enabled: true,
            disabled_reason: None,
            z_layer: ZLayer::ModalDialog,
            ellipsized: false,
            full_label: None,
        },
    );
}

fn gtr(locale: &str, key: &str) -> &'static str {
    match (key, locale) {
        ("modal.apply", "de") => "Anwenden",
        ("modal.apply", "es") => "Aplicar",
        ("modal.apply", "fr") => "Appliquer",
        ("modal.apply", "ja") => "適用",
        ("modal.apply", "ko") => "적용",
        ("modal.apply", "pt-BR") => "Aplicar",
        ("modal.apply", "zh-CN") => "应用",
        ("modal.apply", "zh-TW") => "套用",
        ("modal.apply", "it") => "Applica",
        ("modal.apply", "nl") => "Toepassen",
        ("modal.apply", "pl") => "Zastosuj",
        ("modal.apply", "ru") => "Применить",
        ("modal.apply", "tr") => "Uygula",
        ("modal.apply", _) => "Apply",

        ("cgt.show_all", "de") => "Alle Typen anzeigen",
        ("cgt.show_all", "es") => "Mostrar todos",
        ("cgt.show_all", "fr") => "Tout afficher",
        ("cgt.show_all", "ja") => "すべて表示",
        ("cgt.show_all", "ko") => "모두 표시",
        ("cgt.show_all", "pt-BR") => "Mostrar todos",
        ("cgt.show_all", "zh-CN") => "显示全部",
        ("cgt.show_all", "zh-TW") => "顯示全部",
        ("cgt.show_all", _) => "Show All Types",

        ("cgt.hint.compatible", "de") => "Kompatibel mit aktiver Datentabelle.",
        ("cgt.hint.compatible", "ja") => "選択したデータテーブルと互換性があります。",
        ("cgt.hint.compatible", _) => "Compatible with active dataset table structure.",

        ("cgt.hint.paired", _) => "Paired/Repeated Measures detected: Before-After Paired Lines prioritized.",
        ("cgt.hint.cross_table", _) => "Cross-table option: check variable roles before applying",
        ("cgt.preview.empty", _) => "Live preview renders once dataset contains finite points.",

        ("fg.jump.change_graph_type", "de") => "Diagrammtyp ändern…",
        ("fg.jump.change_graph_type", "ja") => "グラフ種類の変更…",
        ("fg.jump.change_graph_type", _) => "Change Graph Type…",

        ("fg.jump.format_axes", "de") => "Achsen formatieren…",
        ("fg.jump.format_axes", "ja") => "軸の書式設定…",
        ("fg.jump.format_axes", _) => "Format Axes…",

        ("fg.tab.appearance", "de") => "Darstellung",
        ("fg.tab.appearance", "ja") => "外観",
        ("fg.tab.appearance", _) => "Appearance",
        ("fg.tab.series", _) => "Series & Arrangement",
        ("fg.tab.settings", _) => "Graph Settings",
        ("fg.tab.annotations", _) => "Value Annotations",
        ("fg.tab.axis_variables", _) => "Axis Variables",
        ("fg.tab.data_objects", _) => "Data Objects",
        ("fg.tab.legends", _) => "Legends",
        ("fg.tab.color_mapping", _) => "Color Mapping",
        ("fg.tab.heatmap_labels", _) => "Labels & Ticks",
        ("fg.tab.heatmap_borders", _) => "Gaps & Borders",

        ("fg.scope.series", _) => "Active Series:",
        ("fg.apply_globally", _) => "Apply Globally",
        ("fg.symbol_size", _) => "Symbol Size",
        ("fg.symbol_fill", _) => "Fill Mode:",
        ("fg.line_dash", _) => "Connecting Line:",
        ("fg.error_stroke", _) => "Error Bar Stroke",
        ("fg.grouped_arrangement", _) => "Grouped Arrangement",
        ("fg.active_series_list", _) => "Plotted Series",
        ("fg.annotations.heading", _) => "Annotations & Model Equations",
        ("fg.equation_mode", _) => "Equation Mode:",
        ("fg.annotations.empty", _) => "No model equations on this graph yet (Insert via Equation tool).",

        ("axes.tab.frame", _) => "Frame & Origin",
        ("axes.tab.x", _) => "X Axis",
        ("axes.tab.y", _) => "Left Y Axis",
        ("axes.tab.y2", _) => "Right Y2 Axis",
        ("axes.tab.titles", _) => "Titles & Typography",
        ("axes.frame.heading", _) => "Frame Preset",
        ("axes.scale.heading", _) => "Scale Bijection",
        ("axes.categorical.heading", _) => "Categorical Ticks",
        ("axes.categorical.switch_btn", _) => "Switch to Continuous Scale…",
        ("axes.guard.warning", _) => {
            "Categorical Axis Guard: Switching a categorical axis to a continuous/logarithmic scale replaces discrete group labels with numeric tick coordinates."
        }
        ("axes.guard.confirm_chk", _) => "I confirm converting this categorical axis to numeric coordinates",
        ("axes.guard.confirm_btn", _) => "Confirm Switch",
        ("axes.guard.cancel_btn", _) => "Keep Categorical",
        ("axes.min", _) => "Min",
        ("axes.max", _) => "Max",
        ("axes.ticks.heading", _) => "Tick Numbering & Additional Ticks",
        ("axes.add_tick.open", _) => "Additional Ticks / Thresholds…",
        ("axes.add_tick.submodal_title", _) => "Additional Ticks & Reference Thresholds",
        ("axes.add_tick.save", _) => "Add / Save Tick or Threshold",
        ("axes.add_tick.done", _) => "Done",

        ("fg.palette", "de") => "Farbpalette",
        ("fg.palette", "ja") => "カラーパレット",
        ("fg.palette", _) => "Palette",
        ("fg.legend", "de") => "Legende anzeigen",
        ("fg.legend", "ja") => "凡例を表示",
        ("fg.legend", _) => "Show Legend",
        ("axes.title_label", "de") => "Achsentitel:",
        ("axes.title_label", "ja") => "軸タイトル:",
        ("axes.title_label", _) => "Title:",

        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::{AdditionalTickDraft, FormatAxesTab, FormatGraphSchema, FormatGraphTab, GraphModalEguiState};

    #[test]
    fn format_graph_schema_clamps_tabs_cleanly() {
        assert_eq!(FormatGraphSchema::Heatmap.clamp_tab(FormatGraphTab::Appearance), FormatGraphTab::ColorMapping);
        assert_eq!(FormatGraphSchema::MvChannelLayer.clamp_tab(FormatGraphTab::ColorMapping), FormatGraphTab::AxisVariables);
        assert_eq!(FormatGraphSchema::SeriesBased.clamp_tab(FormatGraphTab::DataObjects), FormatGraphTab::Appearance);
        assert_eq!(FormatAxesTab::ALL.len(), 5);
        let st = GraphModalEguiState::default();
        assert!(!st.categorical_guard_confirmed);
        assert!(!AdditionalTickDraft::default().open);
    }
}
