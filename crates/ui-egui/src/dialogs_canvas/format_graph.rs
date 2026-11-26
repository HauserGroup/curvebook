//! Non-modal `Format Graph` inspector state machine (palette, CVD preview, frame, error bars, markers).

use curvebook_doc::{
    CartesianOrientation, ColorChannelMapping, ConnectingLineStyle, EquationDisplayMode, ErrorBarSpec, FramePreset, GraphAnnotation, GraphKind,
    GraphSheet, GraphSpec, GraphicalDataObject, GroupedArrangement, HeatmapLabelSpec, LegendSpec, LevelOrderKind, LevelOrderSpec, LineConnectionKind,
    MultivariateGraphEncoding, MultivariatePlotMode, ProjectDocument, RegressionOverlaySpec, ReplicateLineMode, SheetId, SummaryMeasure,
    TableArchetype, VariableRole,
};
use curvebook_palette::{CvdKind, LineDashStyle, MarkerFillMode, MarkerStyle, PaletteId, Rgba8, simulate_cvd};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::edit_params::default_true_flag;

const fn default_alpha_pct_100() -> u8 {
    100
}

const fn default_white_paper_opt() -> Option<Rgba8> {
    Some(Rgba8::WHITE)
}

/// Non-modal `Format Graph` inspector state (`palette`, `CVD` preview, frame, error bars, markers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatGraphState {
    pub graph_sheet_id: Option<SheetId>,
    pub palette: PaletteId,
    pub cvd_preview: Option<CvdKind>,
    pub frame_preset: FramePreset,
    pub grouped_arrangement: GroupedArrangement,
    #[serde(default)]
    pub graph_kind: Option<GraphKind>,
    pub selected_series_index: usize,
    pub error_bar_enabled: bool,
    pub error_bar_spec: ErrorBarSpec,
    pub marker_size_pt: f64,
    pub marker_fill_mode: MarkerFillMode,
    pub line_dash: LineDashStyle,
    pub show_legend: bool,
    pub legend_bordered: bool,
    #[serde(default)]
    pub half_violin_with_points: bool,
    #[serde(default)]
    pub initial_violin_options: Option<(bool, bool, bool)>,
    #[serde(default)]
    pub model_equation_mode: Option<EquationDisplayMode>,
    #[serde(default)]
    pub model_equation_annotations: Vec<GraphAnnotation>,
    #[serde(default)]
    pub annotations_dirty: bool,
    #[serde(default)]
    pub mv_encoding: Option<MultivariateGraphEncoding>,
    #[serde(default)]
    pub mv_levels: Vec<String>,
    #[serde(default)]
    pub mv_level_is_x: bool,
    #[serde(default)]
    pub mv_selected_level: Option<String>,
    #[serde(default)]
    pub mv_levels_dirty: bool,
    #[serde(default = "default_alpha_pct_100")]
    pub series_alpha_pct: u8,
    #[serde(default)]
    pub alpha_dirty: bool,
    #[serde(default = "default_true_flag")]
    pub show_raw_data: bool,
    #[serde(default)]
    pub show_fitted_curve: bool,
    #[serde(default)]
    pub cached_regression_overlay: Option<RegressionOverlaySpec>,
    #[serde(default)]
    pub apply_all_data_sets: bool,
    #[serde(default = "default_white_paper_opt")]
    pub paper_fill: Option<Rgba8>,
    #[serde(default)]
    pub paper_fill_dirty: bool,
    #[serde(default)]
    pub heatmap_labels: HeatmapLabelSpec,
    #[serde(default)]
    pub heatmap_labels_dirty: bool,
    #[serde(default)]
    pub orientation: CartesianOrientation,
    #[serde(default)]
    pub orientation_dirty: bool,
    #[serde(default = "default_true_flag")]
    pub show_symbols: bool,
    #[serde(default)]
    pub show_symbols_dirty: bool,
    #[serde(default)]
    pub replicate_lines: ReplicateLineMode,
    #[serde(default)]
    pub replicate_lines_dirty: bool,
    #[serde(default)]
    pub bind_to_right_y2: bool,
    #[serde(default)]
    pub bind_to_right_y2_dirty: bool,
    #[serde(default)]
    pub offset_x_data: f64,
    #[serde(default)]
    pub offset_x_data_dirty: bool,
    #[serde(default = "default_bar_border_width_pt")]
    pub bar_border_width_pt: f64,
    #[serde(default)]
    pub bar_border_width_dirty: bool,
    #[serde(default)]
    pub forest_show_pooled_diamond: bool,
    #[serde(default)]
    pub forest_show_pooled_diamond_dirty: bool,
    #[serde(default)]
    pub pending_overlay_result_id: Option<Option<SheetId>>,
    #[serde(default)]
    pub series_gdo_bar: bool,
    #[serde(default)]
    pub series_gdo_dirty: bool,
    #[serde(default)]
    pub show_connecting_line: bool,
    #[serde(default)]
    pub line_dash_dirty: bool,
    #[serde(default)]
    pub marker_dirty: bool,
    #[serde(default)]
    pub synced_marker_size_pt: Option<f64>,
    #[serde(default)]
    pub synced_marker_fill_mode: Option<MarkerFillMode>,
    #[serde(default)]
    pub synced_line_dash: Option<LineDashStyle>,
    #[serde(default)]
    pub synced_series_count: Option<usize>,
}

impl Default for FormatGraphState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            palette: PaletteId::OkabeIto,
            cvd_preview: None,
            frame_preset: FramePreset::OpenL,
            grouped_arrangement: GroupedArrangement::Interleaved,
            graph_kind: None,
            selected_series_index: 0,
            error_bar_enabled: true,
            error_bar_spec: ErrorBarSpec::default(),
            marker_size_pt: 5.0,
            marker_fill_mode: MarkerFillMode::Filled,
            line_dash: LineDashStyle::Solid,
            show_legend: true,
            legend_bordered: false,
            half_violin_with_points: false,
            initial_violin_options: None,
            model_equation_mode: None,
            model_equation_annotations: Vec::new(),
            annotations_dirty: false,
            mv_encoding: None,
            mv_levels: Vec::new(),
            mv_level_is_x: true,
            mv_selected_level: None,
            mv_levels_dirty: false,
            series_alpha_pct: 100,
            alpha_dirty: false,
            show_raw_data: true,
            show_fitted_curve: false,
            cached_regression_overlay: None,
            apply_all_data_sets: false,
            paper_fill: Some(Rgba8::WHITE),
            paper_fill_dirty: false,
            heatmap_labels: HeatmapLabelSpec::default(),
            heatmap_labels_dirty: false,
            orientation: CartesianOrientation::default(),
            orientation_dirty: false,
            show_symbols: true,
            show_symbols_dirty: false,
            replicate_lines: ReplicateLineMode::default(),
            replicate_lines_dirty: false,
            bind_to_right_y2: false,
            bind_to_right_y2_dirty: false,
            offset_x_data: 0.0,
            offset_x_data_dirty: false,
            bar_border_width_pt: 1.0,
            bar_border_width_dirty: false,
            forest_show_pooled_diamond: false,
            forest_show_pooled_diamond_dirty: false,
            pending_overlay_result_id: None,
            series_gdo_bar: false,
            series_gdo_dirty: false,
            show_connecting_line: false,
            line_dash_dirty: false,
            marker_dirty: false,
            synced_marker_size_pt: None,
            synced_marker_fill_mode: None,
            synced_line_dash: None,
            synced_series_count: None,
        }
    }
}

impl FormatGraphState {
    /// Populate draft controls from an existing [`GraphSheet`], clearing MV level state unless populated by [`Self::sync_from_graph_with_doc`].
    pub fn sync_from_graph(&mut self, graph: &GraphSheet) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.palette = graph.spec.palette;
        self.frame_preset = graph.spec.frame_preset;
        self.grouped_arrangement = graph.spec.grouped_arrangement;
        self.graph_kind = Some(graph.spec.kind);
        self.orientation = graph.spec.orientation;
        self.orientation_dirty = false;
        self.forest_show_pooled_diamond = graph.spec.forest_show_pooled_diamond;
        self.forest_show_pooled_diamond_dirty = false;
        self.show_legend = graph.spec.legend.visible;
        self.legend_bordered = graph.spec.legend.bordered;
        self.model_equation_annotations.clone_from(&graph.spec.annotations);
        self.model_equation_mode = graph.spec.annotations.iter().find_map(|ann| match ann {
            GraphAnnotation::ModelEquation { mode, .. } => Some(*mode),
            _ => None,
        });
        self.annotations_dirty = false;
        self.mv_encoding.clone_from(&graph.spec.multivariate);
        self.mv_levels.clear();
        self.mv_level_is_x = true;
        self.mv_selected_level = None;
        self.mv_levels_dirty = false;
        let series_idx = self.selected_series_index.min(graph.spec.series.len().saturating_sub(1));
        self.selected_series_index = series_idx;
        self.apply_all_data_sets = false;
        self.alpha_dirty = false;
        self.paper_fill = graph.spec.paper_fill;
        self.paper_fill_dirty = false;
        self.heatmap_labels = graph.spec.heatmap_labels;
        self.heatmap_labels_dirty = false;
        self.sync_selected_series_from_spec(&graph.spec);
    }

    /// Refresh series-scoped formatting controls from `spec.series[self.selected_series_index]`.
    pub fn sync_selected_series_from_spec(&mut self, spec: &GraphSpec) {
        self.synced_series_count = Some(spec.series.len());
        let series_idx = self.selected_series_index.min(spec.series.len().saturating_sub(1));
        self.selected_series_index = series_idx;
        self.pending_overlay_result_id = None;
        self.series_gdo_dirty = false;
        self.alpha_dirty = false;
        if let Some(series) = spec.series.get(series_idx) {
            self.error_bar_enabled = series.y_error_bars.is_some();
            self.error_bar_spec = series.y_error_bars.unwrap_or_default();
            if let GraphicalDataObject::Violin { clip_to_observed_range, show_median_and_quartiles, half_violin_with_points, .. } = series.gdo {
                self.half_violin_with_points = half_violin_with_points;
                self.initial_violin_options = Some((clip_to_observed_range, show_median_and_quartiles, half_violin_with_points));
            } else {
                self.half_violin_with_points = false;
                self.initial_violin_options = None;
            }
            self.series_gdo_bar = matches!(series.gdo, GraphicalDataObject::BarWithReplicates { .. });
            self.show_raw_data = series.show_raw_data;
            self.cached_regression_overlay.clone_from(&series.regression_overlay);
            self.show_fitted_curve = series.regression_overlay.is_some();
            self.bind_to_right_y2 = series.bind_to_right_y2;
            self.bind_to_right_y2_dirty = false;
            self.offset_x_data = series.offset.x_data;
            self.offset_x_data_dirty = false;
            self.show_symbols = match &series.gdo {
                GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } => marker.is_some(),
                _ => true,
            };
            self.show_symbols_dirty = false;
            self.show_connecting_line = matches!(&series.gdo, GraphicalDataObject::ScatterAndLine { line: Some(_), .. });
            self.replicate_lines = match &series.gdo {
                GraphicalDataObject::ScatterAndLine { replicate_lines, .. } => *replicate_lines,
                _ => ReplicateLineMode::default(),
            };
            self.replicate_lines_dirty = false;
            self.bar_border_width_pt = match &series.gdo {
                GraphicalDataObject::BarWithReplicates { border_width_pt, .. } | GraphicalDataObject::FloatingBar { border_width_pt, .. } => {
                    *border_width_pt
                }
                _ => 1.0,
            };
            self.bar_border_width_dirty = false;
            let alpha_u8 = extract_gdo_alpha_byte(&series.gdo);
            self.series_alpha_pct = ((f64::from(alpha_u8) / 255.0) * 100.0).round().clamp(0.0, 100.0) as u8;
            self.marker_size_pt = 5.0;
            self.marker_fill_mode = MarkerFillMode::Filled;
            self.line_dash = LineDashStyle::Solid;
            self.extract_series_marker_and_dash(&series.gdo);
            self.marker_dirty = false;
            self.line_dash_dirty = false;
            self.synced_marker_size_pt = Some(self.marker_size_pt);
            self.synced_marker_fill_mode = Some(self.marker_fill_mode);
            self.synced_line_dash = Some(self.line_dash);
        } else {
            self.error_bar_enabled = false;
            self.error_bar_spec = ErrorBarSpec::default();
            self.half_violin_with_points = false;
            self.initial_violin_options = None;
            self.series_gdo_bar = false;
            self.show_raw_data = true;
            self.cached_regression_overlay = None;
            self.show_fitted_curve = false;
            self.bind_to_right_y2 = false;
            self.bind_to_right_y2_dirty = false;
            self.offset_x_data = 0.0;
            self.offset_x_data_dirty = false;
            self.show_symbols = true;
            self.show_symbols_dirty = false;
            self.show_connecting_line = false;
            self.marker_size_pt = 5.0;
            self.marker_fill_mode = MarkerFillMode::Filled;
            self.marker_dirty = false;
            self.line_dash = LineDashStyle::Solid;
            self.line_dash_dirty = false;
            self.synced_marker_size_pt = None;
            self.synced_marker_fill_mode = None;
            self.synced_line_dash = None;
            self.replicate_lines = ReplicateLineMode::default();
            self.replicate_lines_dirty = false;
            self.bar_border_width_pt = 1.0;
            self.bar_border_width_dirty = false;
            self.series_alpha_pct = 100;
        }
    }

    /// Populate draft controls plus any categorical `MultipleVariables` level ordering from `doc`.
    pub fn sync_from_graph_with_doc(&mut self, graph: &GraphSheet, doc: Option<&ProjectDocument>) {
        let prev_selected = (self.graph_sheet_id.as_ref() == Some(&graph.id)).then(|| self.mv_selected_level.clone()).flatten();
        self.sync_from_graph(graph);
        let Some(doc) = doc else {
            return;
        };
        let Some(enc) = &graph.spec.multivariate else {
            return;
        };
        let Some(ds) = doc.data_sheet(&graph.spec.primary_source_sheet_id) else {
            return;
        };
        let TableArchetype::MultipleVariables(mv) = &ds.archetype else {
            return;
        };
        let x_cat = mv
            .variables
            .get(enc.x_column)
            .filter(|col| col.role == VariableRole::Categorical || matches!(enc.plot_mode, MultivariatePlotMode::CategoricalSummary { .. }));
        if let Some(col) = x_cat {
            let lvls: Vec<String> =
                col.ordered_levels_with(enc.x_level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
            if !lvls.is_empty() {
                self.mv_levels = lvls;
                self.mv_level_is_x = true;
                self.mv_selected_level =
                    resolve_selected_level_idx(&self.mv_levels, prev_selected.as_deref()).and_then(|i| self.mv_levels.get(i).cloned());
                return;
            }
        }
        if let ColorChannelMapping::Categorical { column, level_order_override, .. } = &enc.color
            && let Some(col) = mv.variables.get(*column)
        {
            let lvls: Vec<String> =
                col.ordered_levels_with(level_order_override.as_ref().unwrap_or(&col.level_order)).into_iter().map(|l| l.level).collect();
            if !lvls.is_empty() {
                self.mv_levels = lvls;
                self.mv_level_is_x = false;
                self.mv_selected_level =
                    resolve_selected_level_idx(&self.mv_levels, prev_selected.as_deref()).and_then(|i| self.mv_levels.get(i).cloned());
            }
        }
    }

    /// Cycle selected categorical level inside the `Format Graph` dialog.
    pub fn cycle_selected_mv_level(&mut self) {
        if self.mv_levels.is_empty() {
            return;
        }
        let cur = resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref()).unwrap_or(0);
        let next = (cur + 1) % self.mv_levels.len();
        self.mv_selected_level = self.mv_levels.get(next).cloned();
    }

    /// Stepwise reorder the selected MV level (`"top"`, `"up"`, `"down"`, `"end"`) into `Explicit` order.
    pub fn reorder_mv_level(&mut self, op_str: &str) -> Result<bool, String> {
        if self.mv_levels.is_empty() {
            return Err(String::from("Graph has no categorical levels to reorder."));
        }
        let op = match op_str {
            "top" | "move_to_top" => LevelMoveOp::Top,
            "up" | "move_up" => LevelMoveOp::Up,
            "down" | "move_down" => LevelMoveOp::Down,
            "end" | "move_to_end" => LevelMoveOp::End,
            other => return Err(format!("unknown level move op `{other}`")),
        };
        let moved = apply_explicit_level_move(&mut self.mv_levels, &mut self.mv_selected_level, op);
        let explicit_spec = LevelOrderSpec { order: LevelOrderKind::Explicit(self.mv_levels.clone()), reversed: false };
        if let Some(enc) = &mut self.mv_encoding {
            if self.mv_level_is_x {
                enc.x_level_order_override = Some(explicit_spec);
            } else if let ColorChannelMapping::Categorical { level_order_override, .. } = &mut enc.color {
                *level_order_override = Some(explicit_spec);
            }
            self.mv_levels_dirty = true;
        }
        Ok(moved)
    }

    fn extract_series_marker_and_dash(&mut self, gdo: &GraphicalDataObject) {
        match gdo {
            GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
                if let Some(m) = marker {
                    self.marker_size_pt = m.size_pt;
                    self.marker_fill_mode = m.fill_mode;
                }
                if let Some(l) = line {
                    self.line_dash = l.dash_style;
                }
            }
            GraphicalDataObject::AlignedDotPlot { marker, .. } => {
                self.marker_size_pt = marker.size_pt;
                self.marker_fill_mode = marker.fill_mode;
            }
            GraphicalDataObject::BeforeAfterLines { marker: Some(m), .. } => {
                self.marker_size_pt = m.size_pt;
                self.marker_fill_mode = m.fill_mode;
            }
            _ => {}
        }
    }

    /// Swatch for series `index`, optionally passed through CVD simulation (`Deuteranopia`, etc.).
    #[must_use]
    pub fn preview_swatch(&self, index: usize) -> Rgba8 {
        let base = self.palette.color_at(index);
        match self.cvd_preview {
            Some(kind) => simulate_cvd(base, kind, 1.0),
            None => base,
        }
    }

    /// Apply current inspector state onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.palette = self.palette;
        spec.frame_preset = self.frame_preset;
        spec.grouped_arrangement = self.grouped_arrangement;
        if self.orientation_dirty {
            spec.orientation = self.orientation;
        }
        if self.forest_show_pooled_diamond_dirty {
            spec.forest_show_pooled_diamond = self.forest_show_pooled_diamond;
        }
        if let Some(kind) = self.graph_kind {
            spec.kind = kind;
        }
        spec.legend = LegendSpec { visible: self.show_legend, bordered: self.legend_bordered, ..spec.legend };
        if self.annotations_dirty {
            spec.annotations.clone_from(&self.model_equation_annotations);
        }
        if self.mv_levels_dirty
            && let Some(enc) = &self.mv_encoding
        {
            spec.multivariate = Some(enc.clone());
        }
        let target_indices: Vec<usize> = if self.apply_all_data_sets { (0..spec.series.len()).collect() } else { vec![self.selected_series_index] };
        for idx in target_indices {
            if let Some(series) = spec.series.get_mut(idx) {
                series.y_error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
                let swatch = self.palette.color_at(idx);
                if self.series_gdo_dirty {
                    series.gdo = self.build_switched_series_gdo(swatch);
                }
                if self.pending_overlay_result_id.is_some() || self.cached_regression_overlay.is_some() {
                    series.regression_overlay = if self.show_fitted_curve { self.cached_regression_overlay.clone() } else { None };
                }
                if let GraphicalDataObject::Violin { half_violin_with_points, .. } = &mut series.gdo {
                    *half_violin_with_points = self.half_violin_with_points;
                }
                if !self.show_symbols {
                    if let GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } = &mut series.gdo
                    {
                        *marker = None;
                    }
                } else if self.show_symbols_dirty
                    && let GraphicalDataObject::ScatterAndLine { marker, .. } | GraphicalDataObject::BeforeAfterLines { marker, .. } = &mut series.gdo
                    && marker.is_none()
                {
                    *marker = Some(MarkerStyle::default());
                }
                if self.replicate_lines_dirty
                    && let GraphicalDataObject::ScatterAndLine { replicate_lines, .. } = &mut series.gdo
                {
                    *replicate_lines = self.replicate_lines;
                }
                if self.bind_to_right_y2_dirty {
                    series.bind_to_right_y2 = self.bind_to_right_y2;
                }
                if self.offset_x_data_dirty {
                    series.offset.x_data = self.offset_x_data;
                }
                if self.bar_border_width_dirty
                    && let GraphicalDataObject::BarWithReplicates { border_width_pt, .. } | GraphicalDataObject::FloatingBar { border_width_pt, .. } =
                        &mut series.gdo
                {
                    *border_width_pt = self.bar_border_width_pt;
                }
                apply_marker_and_dash(&mut series.gdo, swatch, self.marker_size_pt, self.marker_fill_mode, self.line_dash);
            }
        }
    }

    /// Switch all [`GraphAnnotation::ModelEquation`] annotations on this graph to `mode`.
    pub fn set_model_equation_mode(&mut self, next_mode: EquationDisplayMode) {
        self.model_equation_mode = Some(next_mode);
        for ann in &mut self.model_equation_annotations {
            if let GraphAnnotation::ModelEquation { mode, .. } = ann {
                *mode = next_mode;
                self.annotations_dirty = true;
            }
        }
    }

    /// Emit declarative `graph.setSpec` and `graph.setSeries` engine command intents.
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let Some(id) = &self.graph_sheet_id else {
            return Vec::new();
        };
        let error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
        let mut spec_patch = serde_json::Map::new();
        spec_patch.insert(String::from("sheet_id"), json!(id.as_str()));
        spec_patch.insert(String::from("palette"), json!(self.palette));
        spec_patch.insert(String::from("frame_preset"), json!(self.frame_preset));
        spec_patch.insert(String::from("grouped_arrangement"), json!(self.grouped_arrangement));
        if let Some(kind) = self.graph_kind {
            spec_patch.insert(String::from("kind"), json!(kind));
        }
        if self.annotations_dirty {
            spec_patch.insert(String::from("annotations"), json!(self.model_equation_annotations));
        }
        if self.mv_levels_dirty
            && let Some(enc) = &self.mv_encoding
        {
            spec_patch.insert(String::from("multivariate"), json!(enc));
        }
        if self.paper_fill_dirty {
            spec_patch.insert(String::from("paper_fill"), json!(self.paper_fill));
        }
        if self.heatmap_labels_dirty {
            spec_patch.insert(String::from("heatmap_labels"), json!(self.heatmap_labels));
        }
        if self.orientation_dirty {
            let orient_str = match self.orientation {
                CartesianOrientation::Vertical => "vertical",
                CartesianOrientation::Horizontal => "horizontal",
            };
            spec_patch.insert(String::from("orientation"), json!(orient_str));
        }
        if self.forest_show_pooled_diamond_dirty {
            spec_patch.insert(String::from("forest_show_pooled_diamond"), json!(self.forest_show_pooled_diamond));
        }
        spec_patch.insert(String::from("legend"), json!({ "visible": self.show_legend, "bordered": self.legend_bordered }));
        let mut intents = vec![ToolCommandIntent::new("graph.setSpec", Value::Object(spec_patch))];
        let skip_series = matches!(self.graph_kind, Some(GraphKind::MultipleVariables)) || self.synced_series_count == Some(0);
        if !skip_series {
            let mut base_series_patch = serde_json::Map::new();
            base_series_patch.insert(String::from("sheet_id"), json!(id.as_str()));
            base_series_patch.insert(String::from("y_error_bars"), json!(error_bars));
            base_series_patch.insert(String::from("show_raw_data"), json!(self.show_raw_data));

            let base_marker_size = self.synced_marker_size_pt.unwrap_or(5.0);
            let base_marker_fill = self.synced_marker_fill_mode.unwrap_or(MarkerFillMode::Filled);
            let base_line_dash = self.synced_line_dash.unwrap_or(LineDashStyle::Solid);
            let marker_modified = self.marker_dirty
                || self.show_symbols_dirty
                || (base_marker_size - self.marker_size_pt).abs() > 1e-12
                || base_marker_fill != self.marker_fill_mode;
            let line_modified = self.line_dash_dirty || base_line_dash != self.line_dash;
            if marker_modified {
                let marker_val = if self.show_symbols {
                    json!({
                        "size_pt": self.marker_size_pt,
                        "fill_mode": self.marker_fill_mode,
                    })
                } else {
                    Value::Null
                };
                base_series_patch.insert(String::from("marker"), marker_val);
            }
            if line_modified {
                base_series_patch.insert(
                    String::from("connecting_line"),
                    json!({
                        "dash_style": self.line_dash,
                    }),
                );
            }
            if self.replicate_lines_dirty {
                base_series_patch.insert(String::from("replicate_lines"), json!(self.replicate_lines));
            }
            if self.bind_to_right_y2_dirty {
                base_series_patch.insert(String::from("bind_to_right_y2"), json!(self.bind_to_right_y2));
            }
            if self.offset_x_data_dirty {
                base_series_patch.insert(String::from("offset"), json!({ "x_data": self.offset_x_data }));
            }
            if self.bar_border_width_dirty {
                base_series_patch.insert(String::from("border_width_pt"), json!(self.bar_border_width_pt));
            }
            if self.alpha_dirty {
                let alpha_u8 = ((f64::from(self.series_alpha_pct) / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
                base_series_patch.insert(String::from("alpha"), json!(alpha_u8));
            }
            if self.pending_overlay_result_id.is_none() && self.cached_regression_overlay.is_some() {
                base_series_patch.insert(
                    String::from("regression_overlay"),
                    if self.show_fitted_curve { json!(self.cached_regression_overlay) } else { Value::Null },
                );
            }

            let replacing_violin =
                !self.series_gdo_dirty && (self.half_violin_with_points || self.initial_violin_options.is_some_and(|(_, _, was_half)| was_half));
            let replacing_gdo = self.series_gdo_dirty || replacing_violin;
            let build_gdo_for_idx = |idx: usize| -> GraphicalDataObject {
                let swatch = self.palette.color_at(idx);
                if self.series_gdo_dirty {
                    self.build_switched_series_gdo(swatch)
                } else {
                    let (clip_to_observed_range, show_median_and_quartiles, _) = self.initial_violin_options.unwrap_or((true, true, false));
                    GraphicalDataObject::Violin {
                        fill_color: swatch.with_alpha(190),
                        stroke_color: swatch,
                        clip_to_observed_range,
                        show_median_and_quartiles,
                        half_violin_with_points: self.half_violin_with_points,
                    }
                }
            };

            if self.apply_all_data_sets && replacing_gdo {
                let count = self.synced_series_count.unwrap_or(1);
                for idx in 0..count {
                    let mut per_series = base_series_patch.clone();
                    per_series.insert(String::from("series_index"), json!(idx));
                    per_series.insert(String::from("gdo"), json!(build_gdo_for_idx(idx)));
                    intents.push(ToolCommandIntent::new("graph.setSeries", Value::Object(per_series)));
                }
            } else {
                if self.apply_all_data_sets {
                    base_series_patch.insert(String::from("all_series"), json!(true));
                } else {
                    base_series_patch.insert(String::from("series_index"), json!(self.selected_series_index));
                }
                if replacing_gdo {
                    base_series_patch.insert(String::from("gdo"), json!(build_gdo_for_idx(self.selected_series_index)));
                }
                intents.push(ToolCommandIntent::new("graph.setSeries", Value::Object(base_series_patch)));
            }

            if let Some(overlay_target) = &self.pending_overlay_result_id {
                let mut ov_params = serde_json::Map::new();
                ov_params.insert(String::from("sheet_id"), json!(id.as_str()));
                if self.apply_all_data_sets {
                    ov_params.insert(String::from("all_series"), json!(true));
                } else {
                    ov_params.insert(String::from("series_index"), json!(self.selected_series_index));
                }
                match overlay_target {
                    Some(rs_id) if self.show_fitted_curve => {
                        ov_params.insert(String::from("result_sheet_id"), json!(rs_id.as_str()));
                        ov_params.insert(String::from("show_confidence_band"), json!(true));
                        ov_params.insert(String::from("show_prediction_band"), json!(false));
                    }
                    _ => {
                        ov_params.insert(String::from("clear"), json!(true));
                    }
                }
                intents.push(ToolCommandIntent::new("graph.setRegressionOverlay", Value::Object(ov_params)));
            }
        }
        intents
    }

    fn build_switched_series_gdo(&self, swatch: Rgba8) -> GraphicalDataObject {
        let alpha_u8 = ((f64::from(self.series_alpha_pct) / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
        let col = swatch.with_alpha(alpha_u8);
        if self.series_gdo_bar {
            GraphicalDataObject::BarWithReplicates {
                summary: SummaryMeasure::Mean,
                fill_color: col,
                border_color: Rgba8::BLACK,
                border_width_pt: self.bar_border_width_pt.max(0.5),
                replicate_overlay: None,
            }
        } else {
            let clamped_size = if self.marker_size_pt.is_finite() { self.marker_size_pt.clamp(1.0, 24.0) } else { 5.0 };
            GraphicalDataObject::ScatterAndLine {
                marker: self.show_symbols.then(|| MarkerStyle {
                    size_pt: clamped_size,
                    fill_mode: self.marker_fill_mode,
                    fill_color: col,
                    stroke_color: swatch,
                    ..MarkerStyle::default()
                }),
                line: Some(ConnectingLineStyle {
                    kind: LineConnectionKind::PointToPoint,
                    dash_style: self.line_dash,
                    stroke_width_pt: 1.5,
                    color: col,
                }),
                area_fill: None,
                replicate_lines: self.replicate_lines,
            }
        }
    }
}

fn apply_marker_and_dash(gdo: &mut GraphicalDataObject, swatch: Rgba8, size_pt: f64, fill_mode: MarkerFillMode, dash: LineDashStyle) {
    let clamped_size = if size_pt.is_finite() { size_pt.clamp(1.0, 24.0) } else { 5.0 };
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, .. } => {
            if let Some(m) = marker {
                m.size_pt = clamped_size;
                m.fill_mode = fill_mode;
                m.fill_color = swatch;
                m.stroke_color = swatch;
            }
            if let Some(l) = line {
                l.dash_style = dash;
                l.color = swatch;
            }
        }
        GraphicalDataObject::AlignedDotPlot { marker, .. } => {
            marker.size_pt = clamped_size;
            marker.fill_mode = fill_mode;
            marker.fill_color = swatch;
            marker.stroke_color = swatch;
        }
        GraphicalDataObject::BeforeAfterLines { marker: Some(m), line_color, .. } => {
            *line_color = swatch;
            m.size_pt = clamped_size;
            m.fill_mode = fill_mode;
        }
        _ => {}
    }
}

const fn extract_gdo_alpha_byte(gdo: &GraphicalDataObject) -> u8 {
    match gdo {
        GraphicalDataObject::ScatterAndLine { marker, line, area_fill, .. } => {
            if let Some(m) = marker {
                m.fill_color.a
            } else if let Some(l) = line {
                l.color.a
            } else if let Some(af) = area_fill {
                af.fill_color.a
            } else {
                255
            }
        }
        GraphicalDataObject::BarWithReplicates { fill_color, .. }
        | GraphicalDataObject::BoxAndWhisker { fill_color, .. }
        | GraphicalDataObject::Violin { fill_color, .. }
        | GraphicalDataObject::FloatingBar { fill_color, .. } => fill_color.a,
        GraphicalDataObject::AlignedDotPlot { marker, .. } => marker.fill_color.a,
        GraphicalDataObject::BeforeAfterLines { line_color, .. } | GraphicalDataObject::KaplanMeierStep { line_color, .. } => line_color.a,
        GraphicalDataObject::PartsOfWhole { .. } => 255,
    }
}

const fn default_bar_border_width_pt() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LevelMoveOp {
    Top,
    Up,
    Down,
    End,
}

/// Resolve the selected index in `levels` matching `selected`, defaulting to `0` when non-empty.
pub(crate) fn resolve_selected_level_idx(levels: &[String], selected: Option<&str>) -> Option<usize> {
    if levels.is_empty() {
        return None;
    }
    selected.and_then(|name| levels.iter().position(|lvl| lvl == name)).or(Some(0))
}

/// Stepwise reorder the selected level in `levels`, updating `selected` to follow the moved item.
fn apply_explicit_level_move(levels: &mut Vec<String>, selected: &mut Option<String>, op: LevelMoveOp) -> bool {
    let Some(idx) = resolve_selected_level_idx(levels, selected.as_deref()) else {
        return false;
    };
    let Some(moved_name) = levels.get(idx).cloned() else {
        return false;
    };
    let changed = match op {
        LevelMoveOp::Top if idx > 0 => {
            let item = levels.remove(idx);
            levels.insert(0, item);
            true
        }
        LevelMoveOp::Up if idx > 0 => {
            levels.swap(idx, idx - 1);
            true
        }
        LevelMoveOp::Down if idx + 1 < levels.len() => {
            levels.swap(idx, idx + 1);
            true
        }
        LevelMoveOp::End if idx + 1 < levels.len() => {
            let item = levels.remove(idx);
            levels.push(item);
            true
        }
        _ => false,
    };
    *selected = Some(moved_name);
    changed
}
