//! Public state and schema types for graph formatting & configuration modals.

use curvebook_doc::{DiagnosticPlotKind, FramePreset, GraphAxisScale, GraphKind, TickLabelFormat};
use curvebook_palette::ColormapId;
use egui::Rect;

use crate::UiApp;
use crate::dialogs_canvas::{ActiveDialog, AxisTarget, GraphTypeOption};

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
    pub heatmap_range_error: Option<String>,
    pub heatmap_annotate_cells: bool,
    pub heatmap_cell_gap_pt: f64,
    pub heatmap_tick_angle_deg: f64,
    pub heatmap_gap_spinner: ScrubUnitFieldState,
    pub mv_min_radius_spinner: ScrubUnitFieldState,
    pub mv_max_radius_spinner: ScrubUnitFieldState,
    pub mv_out_of_range: MvOutOfRangePolicy,
    pub format_axes_session_synced: bool,
    pub format_axes_synced_generation: Option<u64>,
    pub format_axes_last_sheet_id: Option<String>,
    pub format_axes_last_target: Option<AxisTarget>,
    pub format_axes_frame_preset: FramePreset,
    pub format_axes_frame_preset_dirty: bool,
    pub format_axes_title_draft: String,
    pub axis_min_draft: String,
    pub axis_max_draft: String,
    pub axis_bounds_error: Option<String>,
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
            heatmap_range_error: None,
            heatmap_annotate_cells: true,
            heatmap_cell_gap_pt: 1.5,
            heatmap_tick_angle_deg: 0.0,
            heatmap_gap_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_min_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_max_radius_spinner: ScrubUnitFieldState::new(ScrubUnitKind::Points),
            mv_out_of_range: MvOutOfRangePolicy::ClampToBounds,
            format_axes_session_synced: false,
            format_axes_synced_generation: None,
            format_axes_last_sheet_id: None,
            format_axes_last_target: None,
            format_axes_frame_preset: FramePreset::OpenL,
            format_axes_frame_preset_dirty: false,
            format_axes_title_draft: String::new(),
            axis_min_draft: String::new(),
            axis_max_draft: String::new(),
            axis_bounds_error: None,
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
            | ActiveDialog::CreateGraph { .. }
            | ActiveDialog::EqualizeScaling { .. }
            | ActiveDialog::FormatEmbeddedTable { .. }
            | ActiveDialog::FormatImage { .. }
    )
}
