//! Non-modal `Format Graph` inspector state machine (palette, CVD preview, frame, error bars, markers).

use curvebook_doc::{
    ColorChannelMapping, EquationDisplayMode, ErrorBarSpec, FramePreset, GraphAnnotation, GraphKind, GraphSheet, GraphSpec, GraphicalDataObject,
    GroupedArrangement, HeatmapLabelSpec, LegendSpec, LevelOrderKind, LevelOrderSpec, MultivariateGraphEncoding, MultivariatePlotMode,
    ProjectDocument, RegressionOverlaySpec, SheetId, TableArchetype, VariableRole,
};
use curvebook_palette::{CvdKind, LineDashStyle, MarkerFillMode, PaletteId, Rgba8, simulate_cvd};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_r9::parse_equation_display_mode_str;
use crate::i18n::LocalePreset;

use super::edit_params::default_true_flag;
use super::parse_helpers::{
    parse_dialog_num_with_locale, parse_frame_preset_str, parse_graph_kind_str, parse_grouped_arrangement_str, parse_palette_id_str,
};

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
        let series_idx = self.selected_series_index.min(spec.series.len().saturating_sub(1));
        self.selected_series_index = series_idx;
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
            self.show_raw_data = series.show_raw_data;
            self.cached_regression_overlay.clone_from(&series.regression_overlay);
            self.show_fitted_curve = series.regression_overlay.is_some();
            let alpha_u8 = extract_gdo_alpha_byte(&series.gdo);
            self.series_alpha_pct = ((f64::from(alpha_u8) / 255.0) * 100.0).round().clamp(0.0, 100.0) as u8;
            self.extract_series_marker_and_dash(&series.gdo);
        }
    }

    /// Populate draft controls plus any categorical `MultipleVariables` level ordering from `doc`.
    pub fn sync_from_graph_with_doc(&mut self, graph: &GraphSheet, doc: Option<&ProjectDocument>) {
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
                self.mv_selected_level = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref())
                    .and_then(|i| self.mv_levels.get(i).cloned());
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
                self.mv_selected_level = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref())
                    .and_then(|i| self.mv_levels.get(i).cloned());
            }
        }
    }

    /// Cycle selected categorical level inside the `Format Graph` dialog.
    pub fn cycle_selected_mv_level(&mut self) {
        if self.mv_levels.is_empty() {
            return;
        }
        let cur = crate::views::mv_formula::resolve_selected_level_idx(&self.mv_levels, self.mv_selected_level.as_deref()).unwrap_or(0);
        let next = (cur + 1) % self.mv_levels.len();
        self.mv_selected_level = self.mv_levels.get(next).cloned();
    }

    /// Stepwise reorder the selected MV level (`"top"`, `"up"`, `"down"`, `"end"`) into `Explicit` order.
    pub fn reorder_mv_level(&mut self, op_str: &str) -> Result<bool, String> {
        use crate::views::mv_formula::{LevelMoveOp, apply_explicit_level_move};
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
        if let Some(series) = spec.series.get_mut(self.selected_series_index) {
            series.y_error_bars = self.error_bar_enabled.then_some(self.error_bar_spec);
            let swatch = self.palette.color_at(self.selected_series_index);
            if let GraphicalDataObject::Violin { half_violin_with_points, .. } = &mut series.gdo {
                *half_violin_with_points = self.half_violin_with_points;
            }
            apply_marker_and_dash(&mut series.gdo, swatch, self.marker_size_pt, self.marker_fill_mode, self.line_dash);
        }
    }

    /// Patch `FormatGraph` fields from scripted GUI actions / inspector text inputs.
    ///
    /// # Errors
    /// Returns an error if a supplied enum variant or numeric value is invalid.
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, &LocalePreset::period_comma("en-US"))
    }

    /// Apply field patches using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(&mut self, fields: &serde_json::Map<String, Value>, preset: &LocalePreset) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "show_legend" | "format_graph.show_legend" => {
                    self.show_legend = val.as_bool().ok_or_else(|| String::from("`show_legend` must be boolean"))?;
                }
                "legend_bordered" | "format_graph.legend_bordered" => {
                    self.legend_bordered = val.as_bool().ok_or_else(|| String::from("`legend_bordered` must be boolean"))?;
                }
                "error_bar_enabled" | "format_graph.error_bar_enabled" => {
                    self.error_bar_enabled = val.as_bool().ok_or_else(|| String::from("`error_bar_enabled` must be boolean"))?;
                }
                "marker_size_pt" | "format_graph.marker_size_pt" => {
                    let sz = parse_dialog_num_with_locale(val, key, preset)?;
                    if sz <= 0.0 {
                        return Err(String::from("`marker_size_pt` must be a finite positive number"));
                    }
                    self.marker_size_pt = sz.clamp(1.0, 24.0);
                }
                "selected_series_index" | "series_index" | "format_graph.selected_series_index" => {
                    let idx = val
                        .as_u64()
                        .or_else(|| val.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
                        .ok_or_else(|| String::from("`selected_series_index` must be a non-negative integer"))?;
                    self.selected_series_index = usize::try_from(idx).unwrap_or(usize::MAX);
                }
                "grouped_arrangement" | "format_graph.grouped_arrangement" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`grouped_arrangement` must be a string"))?;
                    self.grouped_arrangement = parse_grouped_arrangement_str(raw)?;
                }
                "graph_kind" | "kind" | "format_graph.graph_kind" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`graph_kind` must be a string"))?;
                    self.graph_kind = Some(parse_graph_kind_str(raw)?);
                }
                "palette" | "format_graph.palette" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`palette` must be a string"))?;
                    self.palette = parse_palette_id_str(raw)?;
                }
                "frame_preset" | "format_graph.frame_preset" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`frame_preset` must be a string"))?;
                    self.frame_preset = parse_frame_preset_str(raw)?;
                }
                "half_violin_with_points" | "raincloud" | "format_graph.half_violin_with_points" => {
                    self.half_violin_with_points = val.as_bool().ok_or_else(|| String::from("`half_violin_with_points` must be boolean"))?;
                }
                "model_equation_mode" | "equation_mode" | "format_graph.model_equation_mode" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`model_equation_mode` must be `substituted` or `symbolic`"))?;
                    let mode = parse_equation_display_mode_str(raw)?;
                    self.set_model_equation_mode(mode);
                }
                "toggle_model_equation_mode" | "format_graph.toggle_model_equation_mode" => {
                    let next = match self.model_equation_mode.unwrap_or_default() {
                        EquationDisplayMode::Substituted => EquationDisplayMode::Symbolic,
                        EquationDisplayMode::Symbolic => EquationDisplayMode::Substituted,
                    };
                    self.set_model_equation_mode(next);
                }
                "selected_level" | "format_graph.selected_level" => {
                    let s = val.as_str().ok_or_else(|| String::from("`selected_level` must be a string"))?;
                    self.mv_selected_level = Some(s.to_owned());
                }
                "move_to_top" | "format_graph.move_to_top" => {
                    let _ = self.reorder_mv_level("top")?;
                }
                "move_up" | "format_graph.move_up" => {
                    let _ = self.reorder_mv_level("up")?;
                }
                "move_down" | "format_graph.move_down" => {
                    let _ = self.reorder_mv_level("down")?;
                }
                "move_to_end" | "format_graph.move_to_end" => {
                    let _ = self.reorder_mv_level("end")?;
                }
                _ => self.apply_extended_format_graph_field(key, val, preset)?,
            }
        }
        Ok(())
    }

    fn apply_extended_format_graph_field(&mut self, key: &str, val: &Value, preset: &LocalePreset) -> Result<(), String> {
        match key {
            "series_alpha_pct" | "alpha_pct" | "alpha" | "format_graph.alpha" => {
                let raw = parse_dialog_num_with_locale(val, key, preset)?;
                let pct = if raw <= 1.0 && !matches!(key, "series_alpha_pct" | "alpha_pct") { raw * 100.0 } else { raw };
                self.series_alpha_pct = pct.round().clamp(0.0, 100.0) as u8;
                self.alpha_dirty = true;
            }
            "show_raw_data" | "format_graph.show_raw_data" => {
                self.show_raw_data = val.as_bool().ok_or_else(|| String::from("`show_raw_data` must be boolean"))?;
            }
            "show_fitted_curve" | "format_graph.show_fitted_curve" => {
                self.show_fitted_curve = val.as_bool().ok_or_else(|| String::from("`show_fitted_curve` must be boolean"))?;
            }
            "apply_all_data_sets" | "apply_all" | "format_graph.apply_all_data_sets" | "format_graph.apply_globally" => {
                self.apply_all_data_sets = val.as_bool().ok_or_else(|| String::from("`apply_all_data_sets` must be boolean"))?;
            }
            "paper_fill" | "format_graph.paper_fill" => {
                self.paper_fill = parse_paper_fill_patch(val)?;
                self.paper_fill_dirty = true;
            }
            "heatmap_labels" | "format_graph.heatmap_labels" => {
                self.heatmap_labels =
                    serde_json::from_value::<HeatmapLabelSpec>(val.clone()).map_err(|err| format!("invalid `heatmap_labels`: {err}"))?;
                self.heatmap_labels_dirty = true;
            }
            other => return Err(format!("unknown Format Graph field `{other}`")),
        }
        Ok(())
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
        spec_patch.insert(String::from("legend"), json!({ "visible": self.show_legend, "bordered": self.legend_bordered }));
        let mut series_patch = serde_json::Map::new();
        series_patch.insert(String::from("sheet_id"), json!(id.as_str()));
        if self.apply_all_data_sets {
            series_patch.insert(String::from("all_series"), json!(true));
        } else {
            series_patch.insert(String::from("series_index"), json!(self.selected_series_index));
        }
        series_patch.insert(String::from("y_error_bars"), json!(error_bars));
        series_patch.insert(String::from("show_raw_data"), json!(self.show_raw_data));
        series_patch.insert(
            String::from("marker"),
            json!({
                "size_pt": self.marker_size_pt,
                "fill_mode": self.marker_fill_mode,
            }),
        );
        series_patch.insert(
            String::from("connecting_line"),
            json!({
                "dash_style": self.line_dash,
            }),
        );
        if self.alpha_dirty {
            let alpha_u8 = ((f64::from(self.series_alpha_pct) / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
            series_patch.insert(String::from("alpha"), json!(alpha_u8));
        }
        if self.cached_regression_overlay.is_some() {
            series_patch
                .insert(String::from("regression_overlay"), if self.show_fitted_curve { json!(self.cached_regression_overlay) } else { Value::Null });
        }
        if self.half_violin_with_points || self.initial_violin_options.is_some_and(|(_, _, was_half)| was_half) {
            let swatch = self.palette.color_at(self.selected_series_index);
            let (clip_to_observed_range, show_median_and_quartiles, _) = self.initial_violin_options.unwrap_or((true, true, false));
            series_patch.insert(
                String::from("gdo"),
                json!(GraphicalDataObject::Violin {
                    fill_color: swatch.with_alpha(190),
                    stroke_color: swatch,
                    clip_to_observed_range,
                    show_median_and_quartiles,
                    half_violin_with_points: self.half_violin_with_points,
                }),
            );
        }
        let series_obj = Value::Object(series_patch);
        let mut intents = vec![ToolCommandIntent::new("graph.setSpec", Value::Object(spec_patch))];
        if !matches!(self.graph_kind, Some(GraphKind::MultipleVariables)) {
            intents.push(ToolCommandIntent::new("graph.setSeries", series_obj));
        }
        intents
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

fn parse_paper_fill_patch(val: &Value) -> Result<Option<Rgba8>, String> {
    if val.is_null() {
        return Ok(None);
    }
    if let Some(s) = val.as_str() {
        return match s.trim().to_ascii_lowercase().as_str() {
            "none" | "transparent" => Ok(None),
            "white" => Ok(Some(Rgba8::WHITE)),
            "cream" => Ok(Some(Rgba8::rgb(255, 251, 235))),
            "slate" => Ok(Some(Rgba8::rgb(241, 245, 249))),
            other => Rgba8::from_hex(other).map(Some).ok_or_else(|| format!("invalid paper fill `{other}`")),
        };
    }
    serde_json::from_value::<Rgba8>(val.clone()).map(Some).map_err(|err| format!("invalid `paper_fill`: {err}"))
}
