//! Public state and schema types for graph formatting & configuration modals.

use std::collections::BTreeMap;

use curvebook_chart::ChartScene;
use curvebook_doc::{
    DiagnosticPlotKind, ErrorBarMetric, ErrorBarSpec, FramePreset, GraphAxisScale, GraphKind, GraphSpec, RegressionOverlaySpec, ReplicateLineMode,
    SheetId, SummaryMeasure, SurvivalDisplayMetric, TickLabelFormat,
};
use curvebook_palette::{ColormapId, LineDashStyle, MarkerFillMode};
use egui::Rect;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AxisTarget, FormatGraphState, GraphTypeOption};

use super::super::unit_spinners::{ScrubUnitFieldState, ScrubUnitKind};

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
    pub(super) const fn widget_id(self) -> &'static str {
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
    pub(super) const fn label_key(self) -> &'static str {
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
    pub(super) const fn widget_id(self) -> &'static str {
        match self {
            Self::FrameAndOrigin => "edit.tab.axes.frame_origin",
            Self::XAxis => "edit.tab.axes.x",
            Self::LeftYAxis => "edit.tab.axes.y",
            Self::RightYAxis => "edit.tab.axes.y2",
            Self::TitlesAndTypography => "edit.tab.axes.titles",
        }
    }

    #[must_use]
    pub(super) const fn label_key(self) -> &'static str {
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
    pub(super) const fn slug(self) -> &'static str {
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
    pub(super) const fn label(self) -> &'static str {
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
    pub(super) const fn matches(self, opt: &GraphTypeOption) -> bool {
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
    pub value_draft: String,
    pub end_value_draft: String,
    pub label_draft: String,
    pub show_line: bool,
    pub shaded_zone: bool,
}

/// Snapshot of heatmap color mapping drafts used to detect whether heatmap color settings changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeatmapColorSnapshot {
    pub(super) colormap: ColormapId,
    pub(super) reversed: bool,
    pub(super) auto_range: bool,
    pub(super) min_draft: String,
    pub(super) max_draft: String,
}

/// Cached per-`(axis, segment)` text drafts and validation errors inside `Format Axes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AxisDraftCache {
    pub(super) min_draft: String,
    pub(super) max_draft: String,
    pub(super) bounds_error: Option<String>,
    pub(super) major_interval_draft: String,
    pub(super) major_interval_error: Option<String>,
    pub(super) minor_ticks_draft: u8,
    pub(super) minor_ticks_error: Option<String>,
    pub(super) length_share_draft: String,
    pub(super) length_share_error: Option<String>,
}

/// Staged per-series edits inside `Format Graph` so switching series preserves unsaved changes (`AUD-10-40`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StagedSeriesEdit {
    pub(super) error_bar_enabled: bool,
    pub(super) error_bar_spec: ErrorBarSpec,
    pub(super) marker_size_pt: f64,
    pub(super) marker_fill_mode: MarkerFillMode,
    pub(super) line_dash: LineDashStyle,
    pub(super) half_violin_with_points: bool,
    pub(super) initial_violin_options: Option<(bool, bool, bool)>,
    pub(super) series_alpha_pct: u8,
    pub(super) alpha_dirty: bool,
    pub(super) show_raw_data: bool,
    pub(super) show_fitted_curve: bool,
    pub(super) cached_regression_overlay: Option<RegressionOverlaySpec>,
    pub(super) show_symbols: bool,
    pub(super) show_symbols_dirty: bool,
    pub(super) replicate_lines: ReplicateLineMode,
    pub(super) replicate_lines_dirty: bool,
    pub(super) bind_to_right_y2: bool,
    pub(super) bind_to_right_y2_dirty: bool,
    pub(super) offset_x_data: f64,
    pub(super) offset_x_data_dirty: bool,
    pub(super) bar_border_width_pt: f64,
    pub(super) bar_border_width_dirty: bool,
    pub(super) pending_overlay_result_id: Option<Option<SheetId>>,
    pub(super) series_gdo_bar: bool,
    pub(super) series_gdo_dirty: bool,
}

impl StagedSeriesEdit {
    pub(super) fn capture(fg: &FormatGraphState) -> Self {
        Self {
            error_bar_enabled: fg.error_bar_enabled,
            error_bar_spec: fg.error_bar_spec,
            marker_size_pt: fg.marker_size_pt,
            marker_fill_mode: fg.marker_fill_mode,
            line_dash: fg.line_dash,
            half_violin_with_points: fg.half_violin_with_points,
            initial_violin_options: fg.initial_violin_options,
            series_alpha_pct: fg.series_alpha_pct,
            alpha_dirty: fg.alpha_dirty,
            show_raw_data: fg.show_raw_data,
            show_fitted_curve: fg.show_fitted_curve,
            cached_regression_overlay: fg.cached_regression_overlay.clone(),
            show_symbols: fg.show_symbols,
            show_symbols_dirty: fg.show_symbols_dirty,
            replicate_lines: fg.replicate_lines,
            replicate_lines_dirty: fg.replicate_lines_dirty,
            bind_to_right_y2: fg.bind_to_right_y2,
            bind_to_right_y2_dirty: fg.bind_to_right_y2_dirty,
            offset_x_data: fg.offset_x_data,
            offset_x_data_dirty: fg.offset_x_data_dirty,
            bar_border_width_pt: fg.bar_border_width_pt,
            bar_border_width_dirty: fg.bar_border_width_dirty,
            pending_overlay_result_id: fg.pending_overlay_result_id.clone(),
            series_gdo_bar: fg.series_gdo_bar,
            series_gdo_dirty: fg.series_gdo_dirty,
        }
    }

    pub(super) fn from_spec(spec: &GraphSpec, series_index: usize) -> Self {
        let mut baseline = FormatGraphState { selected_series_index: series_index, ..FormatGraphState::default() };
        baseline.sync_selected_series_from_spec(spec);
        Self::capture(&baseline)
    }

    pub(super) fn differs_from_spec(&self, spec: &GraphSpec, series_index: usize) -> bool {
        if self.alpha_dirty
            || self.show_symbols_dirty
            || self.replicate_lines_dirty
            || self.bind_to_right_y2_dirty
            || self.offset_x_data_dirty
            || self.bar_border_width_dirty
            || self.series_gdo_dirty
            || self.pending_overlay_result_id.is_some()
        {
            return true;
        }
        let baseline = Self::from_spec(spec, series_index);
        self != &baseline
    }

    pub(super) fn apply_to(&self, fg: &mut FormatGraphState) {
        fg.error_bar_enabled = self.error_bar_enabled;
        fg.error_bar_spec = self.error_bar_spec;
        fg.marker_size_pt = self.marker_size_pt;
        fg.marker_fill_mode = self.marker_fill_mode;
        fg.line_dash = self.line_dash;
        fg.half_violin_with_points = self.half_violin_with_points;
        fg.initial_violin_options = self.initial_violin_options;
        fg.series_alpha_pct = self.series_alpha_pct;
        fg.alpha_dirty = self.alpha_dirty;
        fg.show_raw_data = self.show_raw_data;
        fg.show_fitted_curve = self.show_fitted_curve;
        fg.cached_regression_overlay.clone_from(&self.cached_regression_overlay);
        fg.show_symbols = self.show_symbols;
        fg.show_symbols_dirty = self.show_symbols_dirty;
        fg.replicate_lines = self.replicate_lines;
        fg.replicate_lines_dirty = self.replicate_lines_dirty;
        fg.bind_to_right_y2 = self.bind_to_right_y2;
        fg.bind_to_right_y2_dirty = self.bind_to_right_y2_dirty;
        fg.offset_x_data = self.offset_x_data;
        fg.offset_x_data_dirty = self.offset_x_data_dirty;
        fg.bar_border_width_pt = self.bar_border_width_pt;
        fg.bar_border_width_dirty = self.bar_border_width_dirty;
        fg.pending_overlay_result_id.clone_from(&self.pending_overlay_result_id);
        fg.series_gdo_bar = self.series_gdo_bar;
        fg.series_gdo_dirty = self.series_gdo_dirty;
    }
}

/// Memoized live preview `ChartScene` inside `Change Graph Type` (`AUD-10-54`).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphTypePreviewCache {
    pub(super) graph_sheet_id: SheetId,
    pub(super) selected_option_id: String,
    pub(super) summary_measure: SummaryMeasure,
    pub(super) error_bar_metric: ErrorBarMetric,
    pub(super) survival_metric: SurvivalDisplayMetric,
    pub(super) survival_staircase: bool,
    pub(super) survival_error_display: Option<ErrorBarMetric>,
    pub(super) source_spec: GraphSpec,
    pub(super) undo_depth: usize,
    pub(super) redo_depth: usize,
    pub(super) sheet_counts: (usize, usize, usize),
    pub(super) compiled: Result<ChartScene, String>,
}

pub(super) const fn axis_target_slot(target: AxisTarget) -> usize {
    match target {
        AxisTarget::X => 0,
        AxisTarget::Y => 1,
        AxisTarget::Y2 => 2,
    }
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
    pub line_thickness_spinner: ScrubUnitFieldState,
    pub(super) bar_border_spinner: ScrubUnitFieldState,
    pub(super) series_offset_x_spinner: ScrubUnitFieldState,
    pub graph_type_family: GraphTypeFamily,
    pub graph_type_show_all: bool,
    pub(super) change_graph_type_last_sheet_id: Option<String>,
    pub(super) graph_type_preview_cache: Option<GraphTypePreviewCache>,
    pub heatmap_colormap: ColormapId,
    pub heatmap_reversed: bool,
    pub heatmap_auto_range: bool,
    pub heatmap_min_draft: String,
    pub heatmap_max_draft: String,
    pub heatmap_range_error: Option<String>,
    pub heatmap_tick_angle_deg: f64,
    pub heatmap_color_dirty: bool,
    pub heatmap_tick_angle_dirty: bool,
    pub(super) heatmap_synced_color_snapshot: Option<HeatmapColorSnapshot>,
    pub(super) heatmap_synced_tick_angle_deg: f64,
    pub heatmap_gap_spinner: ScrubUnitFieldState,
    pub mv_min_radius_spinner: ScrubUnitFieldState,
    pub mv_max_radius_spinner: ScrubUnitFieldState,
    pub mv_out_of_range: MvOutOfRangePolicy,
    pub format_graph_session_synced: bool,
    pub format_graph_last_sheet_id: Option<String>,
    pub(super) format_graph_synced_spec: Option<GraphSpec>,
    pub(super) staged_series_edits: BTreeMap<usize, StagedSeriesEdit>,
    pub mv_clamp_min_draft: String,
    pub mv_clamp_max_draft: String,
    pub mv_clamp_error: Option<String>,
    pub format_axes_session_synced: bool,
    pub format_axes_synced_generation: Option<u64>,
    pub format_axes_last_sheet_id: Option<String>,
    pub format_axes_last_target: Option<AxisTarget>,
    pub format_axes_selected_segment: usize,
    pub(super) format_axes_last_segment: usize,
    pub(super) format_axes_pending_tab: Option<FormatAxesTab>,
    pub(super) format_axes_per_axis_cache: [[Option<AxisDraftCache>; 3]; 3],
    pub format_axes_frame_preset: FramePreset,
    pub format_axes_frame_preset_dirty: bool,
    pub format_axes_title_draft: String,
    pub axis_min_draft: String,
    pub axis_max_draft: String,
    pub axis_bounds_error: Option<String>,
    pub axis_major_interval_draft: String,
    pub axis_major_interval_error: Option<String>,
    pub axis_minor_ticks_draft: u8,
    pub axis_minor_ticks_error: Option<String>,
    pub axis_length_share_draft: String,
    pub axis_length_share_error: Option<String>,
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
    pub(super) page_layout_last_sheet_id: Option<String>,
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
            line_thickness_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            bar_border_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            series_offset_x_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Generic),
            graph_type_family: GraphTypeFamily::Recommended,
            graph_type_show_all: false,
            change_graph_type_last_sheet_id: None,
            graph_type_preview_cache: None,
            heatmap_colormap: ColormapId::Viridis,
            heatmap_reversed: false,
            heatmap_auto_range: true,
            heatmap_min_draft: String::from("-1.0"),
            heatmap_max_draft: String::from("1.0"),
            heatmap_range_error: None,
            heatmap_tick_angle_deg: 0.0,
            heatmap_color_dirty: false,
            heatmap_tick_angle_dirty: false,
            heatmap_synced_color_snapshot: None,
            heatmap_synced_tick_angle_deg: 0.0,
            heatmap_gap_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_min_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_max_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_out_of_range: MvOutOfRangePolicy::ClampToBounds,
            format_graph_session_synced: false,
            format_graph_last_sheet_id: None,
            format_graph_synced_spec: None,
            staged_series_edits: BTreeMap::new(),
            mv_clamp_min_draft: String::new(),
            mv_clamp_max_draft: String::new(),
            mv_clamp_error: None,
            format_axes_session_synced: false,
            format_axes_synced_generation: None,
            format_axes_last_sheet_id: None,
            format_axes_last_target: None,
            format_axes_selected_segment: 0,
            format_axes_last_segment: 0,
            format_axes_pending_tab: None,
            format_axes_per_axis_cache: Default::default(),
            format_axes_frame_preset: FramePreset::OpenL,
            format_axes_frame_preset_dirty: false,
            format_axes_title_draft: String::new(),
            axis_min_draft: String::new(),
            axis_max_draft: String::new(),
            axis_bounds_error: None,
            axis_major_interval_draft: String::new(),
            axis_major_interval_error: None,
            axis_minor_ticks_draft: 0,
            axis_minor_ticks_error: None,
            axis_length_share_draft: String::from("1"),
            axis_length_share_error: None,
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
            page_layout_last_sheet_id: None,
        }
    }
}

fn reset_scrub_spinner(spinner: &mut ScrubUnitFieldState) {
    spinner.editing = false;
    spinner.dragging = false;
    spinner.draft_text.clear();
}

impl GraphModalEguiState {
    #[must_use]
    pub(super) fn first_axis_error(&self) -> Option<String> {
        if let Some(err) = &self.axis_bounds_error {
            return Some(err.clone());
        }
        if let Some(err) = &self.axis_major_interval_error {
            return Some(err.clone());
        }
        if let Some(err) = &self.axis_minor_ticks_error {
            return Some(err.clone());
        }
        if let Some(err) = &self.axis_length_share_error {
            return Some(err.clone());
        }
        for axis_slots in &self.format_axes_per_axis_cache {
            for cached in axis_slots.iter().flatten() {
                if let Some(err) = &cached.bounds_error {
                    return Some(err.clone());
                }
                if let Some(err) = &cached.major_interval_error {
                    return Some(err.clone());
                }
                if let Some(err) = &cached.minor_ticks_error {
                    return Some(err.clone());
                }
                if let Some(err) = &cached.length_share_error {
                    return Some(err.clone());
                }
            }
        }
        None
    }

    #[must_use]
    pub(super) fn has_any_axis_errors(&self) -> bool {
        self.first_axis_error().is_some()
    }

    #[must_use]
    pub(super) const fn has_pending_categorical_guard(&self) -> bool {
        self.categorical_guard_pending_axis.is_some()
            || self.format_axes_guard_pending_scale.is_some()
            || self.format_axes_guard_pending_tick_format.is_some()
    }

    pub(super) fn clear_categorical_guard(&mut self) {
        self.categorical_guard_confirmed = false;
        self.categorical_guard_pending_axis = None;
        self.format_axes_guard_pending_scale = None;
        self.format_axes_guard_pending_tick_format = None;
    }

    #[must_use]
    pub(super) const fn any_spinner_editing(&self) -> bool {
        self.symbol_size_spinner.editing
            || self.line_thickness_spinner.editing
            || self.bar_border_spinner.editing
            || self.series_offset_x_spinner.editing
            || self.heatmap_gap_spinner.editing
            || self.mv_min_radius_spinner.editing
            || self.mv_max_radius_spinner.editing
            || self.axis_gap_spinner.editing
            || self.axis_stroke_spinner.editing
            || self.cld_font_spinner.editing
            || self.cld_offset_spinner.editing
            || self.eq_font_spinner.editing
            || self.pts_size_spinner.editing
            || self.pts_border_spinner.editing
    }

    pub(super) fn reset_all_spinners(&mut self) {
        for spinner in [
            &mut self.symbol_size_spinner,
            &mut self.line_thickness_spinner,
            &mut self.bar_border_spinner,
            &mut self.series_offset_x_spinner,
            &mut self.heatmap_gap_spinner,
            &mut self.mv_min_radius_spinner,
            &mut self.mv_max_radius_spinner,
            &mut self.axis_gap_spinner,
            &mut self.axis_stroke_spinner,
            &mut self.cld_font_spinner,
            &mut self.cld_offset_spinner,
            &mut self.eq_font_spinner,
            &mut self.pts_size_spinner,
            &mut self.pts_border_spinner,
        ] {
            reset_scrub_spinner(spinner);
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
            | ActiveDialog::CreateGraph { .. }
            | ActiveDialog::EqualizeScaling { .. }
            | ActiveDialog::FormatEmbeddedTable { .. }
            | ActiveDialog::FormatImage { .. }
    )
}
