//! Graph overlay & style inspectors: Pairwise Comparisons, Bubble Plot channels, and Apply Style From.

use curvebook_doc::{
    BracketLabelMode, BracketStyle, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec, GraphKind, GraphSheet, GraphSpec,
    MultivariateGraphEncoding, MultivariatePlotMode, PairwiseBracketSpec, PairwiseDisplayMode, ProjectDocument, SheetId, SizeChannelMapping,
};
use curvebook_palette::{ColorRamp, ColormapId, PaletteId, Rgba8};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::dialogs_r9::{cld_ordering_str, pairwise_display_mode_str};

use super::analyze_dialog::{VariableCandidate, collect_variable_candidates};

/// Live Pairwise Comparison Bracket & Compact Letter Display inspector (`Auto-add from analysis`, `R10.11`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseComparisonsState {
    pub graph_sheet_id: Option<SheetId>,
    pub selected_result_sheet_id: Option<SheetId>,
    pub available_result_ids: Vec<SheetId>,
    pub bracket_style: BracketStyle,
    pub label_mode: BracketLabelMode,
    pub hide_non_significant: bool,
    pub brackets: Vec<PairwiseBracketSpec>,
    #[serde(default)]
    pub brackets_dirty: bool,
    #[serde(default)]
    pub had_brackets_on_graph: bool,
    #[serde(default)]
    pub synced_brackets: Vec<PairwiseBracketSpec>,
    #[serde(default)]
    pub had_cld_on_graph: bool,
    #[serde(default)]
    pub display_mode: PairwiseDisplayMode,
    #[serde(default)]
    pub cld_enabled: bool,
    #[serde(default)]
    pub cld_ordering: CldOrdering,
    #[serde(default)]
    pub cld_uppercase: bool,
    #[serde(default)]
    pub cld_alpha_override: Option<f64>,
    #[serde(default)]
    pub cld_font_size_pt: f64,
    #[serde(default)]
    pub cld_offset_pt: f64,
    #[serde(default)]
    pub cld_color: Rgba8,
}

impl Default for PairwiseComparisonsState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            selected_result_sheet_id: None,
            available_result_ids: Vec::new(),
            bracket_style: BracketStyle::Square,
            label_mode: BracketLabelMode::SummaryStars,
            hide_non_significant: true,
            brackets: Vec::new(),
            brackets_dirty: false,
            had_brackets_on_graph: false,
            synced_brackets: Vec::new(),
            had_cld_on_graph: false,
            display_mode: PairwiseDisplayMode::BracketsOnly,
            cld_enabled: false,
            cld_ordering: CldOrdering::DescendingMean,
            cld_uppercase: false,
            cld_alpha_override: None,
            cld_font_size_pt: 9.5,
            cld_offset_pt: 6.0,
            cld_color: Rgba8::BLACK,
        }
    }
}

impl PairwiseComparisonsState {
    /// Synchronize brackets, CLD settings, and discover upstream [`curvebook_doc::ResultSheet`]s carrying pairwise comparisons.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.brackets = graph.spec.comparisons.clone();
        self.brackets_dirty = false;
        self.had_brackets_on_graph = !graph.spec.comparisons.is_empty();
        self.synced_brackets = graph.spec.comparisons.clone();
        self.available_result_ids = doc.result_sheets.iter().filter(|rs| !rs.pairwise_comparisons.is_empty()).map(|rs| rs.id.clone()).collect();
        self.display_mode = graph.spec.pairwise_display_mode;
        if let Some(cld) = &graph.spec.compact_letter_display {
            self.had_cld_on_graph = true;
            self.cld_enabled = true;
            self.cld_ordering = cld.ordering;
            self.cld_uppercase = cld.uppercase;
            self.cld_alpha_override = cld.alpha_override;
            self.cld_font_size_pt = cld.font_size_pt;
            self.cld_offset_pt = cld.offset_pt;
            self.cld_color = cld.color;
            if self.available_result_ids.contains(&cld.result_sheet_id) {
                self.selected_result_sheet_id = Some(cld.result_sheet_id.clone());
            }
        } else {
            self.had_cld_on_graph = false;
            self.cld_enabled = false;
            self.cld_ordering = CldOrdering::DescendingMean;
            self.cld_uppercase = false;
            self.cld_alpha_override = None;
            self.cld_font_size_pt = 9.5;
            self.cld_offset_pt = 6.0;
            self.cld_color = Rgba8::BLACK;
            if let Some(first_bracket_res) = self.brackets.first().map(|b| &b.result_sheet_id)
                && self.available_result_ids.contains(first_bracket_res)
            {
                self.selected_result_sheet_id = Some(first_bracket_res.clone());
            }
        }
        if self.selected_result_sheet_id.as_ref().is_none_or(|id| !self.available_result_ids.contains(id)) {
            self.selected_result_sheet_id = self.available_result_ids.first().cloned();
        }
    }

    /// Populate brackets automatically from the selected [`curvebook_doc::ResultSheet`]'s `pairwise_comparisons`.
    pub fn auto_add_from_analysis(&mut self, doc: &ProjectDocument) -> usize {
        let Some(res_id) = &self.selected_result_sheet_id else {
            return 0;
        };
        let Some(res) = doc.result_sheet(res_id) else {
            return 0;
        };
        let mut added = 0usize;
        for cmp in &res.pairwise_comparisons {
            if self.hide_non_significant && !cmp.significant {
                continue;
            }
            let duplicate = self.brackets.iter().any(|b| {
                &b.result_sheet_id == res_id
                    && ((b.group_a == cmp.group_a && b.group_b == cmp.group_b) || (b.group_a == cmp.group_b && b.group_b == cmp.group_a))
            });
            if duplicate {
                continue;
            }
            self.brackets.push(PairwiseBracketSpec {
                result_sheet_id: res_id.clone(),
                group_a: cmp.group_a,
                group_b: cmp.group_b,
                style: self.bracket_style,
                label_mode: self.label_mode.clone(),
                hide_if_non_significant: self.hide_non_significant,
                stroke_width_pt: 1.0,
                font_size_pt: 9.0,
                bracket_step_pt: 12.0,
            });
            added += 1;
        }
        if added > 0 {
            self.brackets_dirty = true;
        }
        added
    }

    /// Toggle `cld_enabled` and synchronize `display_mode` cleanly when still in default mode.
    pub fn toggle_cld_enabled(&mut self) {
        self.cld_enabled = !self.cld_enabled;
        if self.cld_enabled {
            if self.display_mode == PairwiseDisplayMode::BracketsOnly {
                self.display_mode = if self.brackets.is_empty() {
                    PairwiseDisplayMode::CompactLetterDisplayOnly
                } else {
                    PairwiseDisplayMode::BracketsAndCompactLetterDisplay
                };
            }
        } else {
            self.display_mode = PairwiseDisplayMode::BracketsOnly;
        }
    }

    /// Cycle `pairwise_display_mode` across brackets-only, CLD-only, and both.
    pub fn cycle_display_mode(&mut self) {
        self.display_mode = match self.display_mode {
            PairwiseDisplayMode::BracketsOnly => PairwiseDisplayMode::CompactLetterDisplayOnly,
            PairwiseDisplayMode::CompactLetterDisplayOnly => PairwiseDisplayMode::BracketsAndCompactLetterDisplay,
            PairwiseDisplayMode::BracketsAndCompactLetterDisplay => PairwiseDisplayMode::BracketsOnly,
        };
        self.cld_enabled = self.display_mode != PairwiseDisplayMode::BracketsOnly;
    }

    /// Apply current bracket and CLD settings back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.comparisons = self.brackets.clone();
        spec.pairwise_display_mode = self.display_mode;
        spec.compact_letter_display = if self.cld_enabled {
            self.selected_result_sheet_id.as_ref().map(|res_id| CompactLetterDisplaySpec {
                result_sheet_id: res_id.clone(),
                alpha_override: self.cld_alpha_override,
                ordering: self.cld_ordering,
                uppercase: self.cld_uppercase,
                font_size_pt: self.cld_font_size_pt,
                offset_pt: self.cld_offset_pt,
                color: self.cld_color,
            })
        } else {
            None
        };
    }

    fn effective_brackets(&self) -> Vec<PairwiseBracketSpec> {
        let style_changed = self.synced_brackets.first().is_some_and(|first| {
            first.style != self.bracket_style || first.label_mode != self.label_mode || first.hide_if_non_significant != self.hide_non_significant
        });
        let mut out = self.brackets.clone();
        if style_changed || self.synced_brackets.is_empty() {
            for b in &mut out {
                b.style = self.bracket_style;
                b.label_mode = self.label_mode.clone();
                b.hide_if_non_significant = self.hide_non_significant;
            }
        }
        out
    }

    #[must_use]
    fn brackets_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let effective = self.effective_brackets();
        if effective.is_empty() {
            if self.had_brackets_on_graph || !self.synced_brackets.is_empty() {
                return Some(ToolCommandIntent::new(
                    "graph.setSpec",
                    json!({
                        "sheet_id": id.as_str(),
                        "comparisons": [],
                    }),
                ));
            }
            return None;
        }
        if effective.len() == 1
            && self.synced_brackets.is_empty()
            && !self.had_brackets_on_graph
            && let Some(res_id) = self.selected_result_sheet_id.as_ref()
            && let [first] = effective.as_slice()
            && first.result_sheet_id == *res_id
        {
            let style_str = match self.bracket_style {
                BracketStyle::Square => "square",
                BracketStyle::HalfTick => "half_tick",
                BracketStyle::FlatLine => "flat_line",
                BracketStyle::CurlyBrace => "curly_brace",
            };
            let mut payload = json!({
                "sheet_id": id.as_str(),
                "result_sheet_id": res_id.as_str(),
                "clear_existing": true,
                "style": style_str,
                "label_mode": self.label_mode,
                "hide_if_non_significant": self.hide_non_significant,
                "significant_only": self.hide_non_significant,
                "stroke_width_pt": first.stroke_width_pt,
                "font_size_pt": first.font_size_pt,
                "bracket_step_pt": first.bracket_step_pt,
            });
            if let Some(map) = payload.as_object_mut() {
                map.insert(String::from("group_a"), json!(first.group_a));
                map.insert(String::from("group_b"), json!(first.group_b));
            }
            return Some(ToolCommandIntent::new("graph.addComparisonBrackets", payload));
        }
        Some(ToolCommandIntent::new(
            "graph.setSpec",
            json!({
                "sheet_id": id.as_str(),
                "comparisons": effective,
            }),
        ))
    }

    /// Emit `graph.setCompactLetterDisplay` intent when CLD or pairwise display mode should be updated.
    #[must_use]
    pub fn cld_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        if !self.cld_enabled && !self.had_cld_on_graph && self.display_mode == PairwiseDisplayMode::BracketsOnly {
            return None;
        }
        let mode_str = pairwise_display_mode_str(self.display_mode);
        if !self.cld_enabled {
            return Some(ToolCommandIntent::new(
                "graph.setCompactLetterDisplay",
                json!({
                    "sheet_id": id.as_str(),
                    "enabled": false,
                    "pairwise_display_mode": mode_str,
                }),
            ));
        }
        let res_id = self.selected_result_sheet_id.as_ref()?;
        let mut payload = json!({
            "sheet_id": id.as_str(),
            "enabled": true,
            "result_sheet_id": res_id.as_str(),
            "pairwise_display_mode": mode_str,
            "ordering": cld_ordering_str(self.cld_ordering),
            "uppercase": self.cld_uppercase,
            "alpha_override": self.cld_alpha_override,
            "font_size_pt": self.cld_font_size_pt,
            "offset_pt": self.cld_offset_pt,
        });
        if self.cld_color != Rgba8::BLACK
            && let Some(map) = payload.as_object_mut()
        {
            map.insert(String::from("color"), json!([self.cld_color.r, self.cld_color.g, self.cld_color.b, self.cld_color.a]));
        }
        Some(ToolCommandIntent::new("graph.setCompactLetterDisplay", payload))
    }

    /// Emit primary intent (`graph.addComparisonBrackets` when brackets exist, else `graph.setCompactLetterDisplay`).
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        self.brackets_commit_intent().or_else(|| self.cld_commit_intent())
    }

    /// Emit all applicable pairwise intents (`graph.addComparisonBrackets` and/or `graph.setCompactLetterDisplay`).
    #[must_use]
    pub fn commit_intents(&self) -> Vec<ToolCommandIntent> {
        let mut intents = Vec::new();
        if self.brackets_dirty
            && let Some(brackets_intent) = self.brackets_commit_intent()
        {
            intents.push(brackets_intent);
        }
        if let Some(cld_intent) = self.cld_commit_intent() {
            intents.push(cld_intent);
        }
        intents
    }
}

/// 5-channel `MultipleVariables` Bubble Plot inspector (`X`, `Y`, `Color`, `Size`, `Label`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BubblePlotInspectorState {
    pub graph_sheet_id: Option<SheetId>,
    pub plot_mode: MultivariatePlotMode,
    pub x_column: usize,
    pub y_column: usize,
    pub color_channel: ColorChannelMapping,
    pub size_channel: SizeChannelMapping,
    pub label_column: Option<usize>,
    pub show_convex_hulls: bool,
    pub show_confidence_ellipses: bool,
    pub available_columns: Vec<VariableCandidate>,
    #[serde(default)]
    pub x_level_order_override: Option<curvebook_doc::LevelOrderSpec>,
}

impl Default for BubblePlotInspectorState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            plot_mode: MultivariatePlotMode::BubbleScatter,
            x_column: 0,
            y_column: 1,
            color_channel: ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0)),
            size_channel: SizeChannelMapping::Constant { radius_pt: 5.0 },
            label_column: None,
            show_convex_hulls: false,
            show_confidence_ellipses: false,
            available_columns: Vec::new(),
            x_level_order_override: None,
        }
    }
}

impl BubblePlotInspectorState {
    /// Load 5-channel encodings and typed source columns for `graph`.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, doc: &ProjectDocument) {
        self.graph_sheet_id = Some(graph.id.clone());
        self.available_columns = doc.data_sheet(&graph.spec.primary_source_sheet_id).map_or_else(Vec::new, collect_variable_candidates);
        if let Some(mv) = &graph.spec.multivariate {
            self.plot_mode = mv.plot_mode.clone();
            self.x_column = mv.x_column;
            self.y_column = mv.y_column;
            self.color_channel = mv.color.clone();
            self.size_channel = mv.size;
            self.label_column = mv.label_column;
            self.show_convex_hulls = mv.show_convex_hulls;
            self.show_confidence_ellipses = mv.show_confidence_ellipses;
            self.x_level_order_override = mv.x_level_order_override.clone();
        } else {
            self.x_level_order_override = None;
        }
    }

    /// Assign categorical column color channel.
    pub fn set_color_categorical(&mut self, column: usize, palette: PaletteId) {
        self.color_channel = ColorChannelMapping::Categorical { column, palette, level_order_override: None };
    }

    /// Assign continuous colormap ramp color channel.
    pub fn set_color_continuous(&mut self, column: usize, colormap: ColormapId, domain_clamp: Option<(f64, f64)>) {
        self.color_channel = ColorChannelMapping::Continuous { column, ramp: ColorRamp::Named { id: colormap, reversed: false }, domain_clamp };
    }

    /// Assign variable bubble radius channel.
    pub fn set_size_variable(&mut self, column: usize, min_radius_pt: f64, max_radius_pt: f64) {
        let min_r = if min_radius_pt.is_finite() { min_radius_pt.clamp(1.0, 24.0) } else { 2.5 };
        let max_r = if max_radius_pt.is_finite() { max_radius_pt.clamp(min_r, 36.0) } else { 14.0 };
        self.size_channel = SizeChannelMapping::Variable { column, min_radius_pt: min_r, max_radius_pt: max_r };
    }

    /// Materialize the 5-channel [`MultivariateGraphEncoding`].
    #[must_use]
    pub fn to_encoding(&self) -> MultivariateGraphEncoding {
        MultivariateGraphEncoding {
            plot_mode: self.plot_mode.clone(),
            x_column: self.x_column,
            y_column: self.y_column,
            color: self.color_channel.clone(),
            size: self.size_channel,
            label_column: self.label_column,
            x_level_order_override: self.x_level_order_override.clone(),
            show_convex_hulls: self.show_convex_hulls,
            show_confidence_ellipses: self.show_confidence_ellipses,
        }
    }

    /// Apply 5-channel encodings to `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) {
        spec.kind = GraphKind::MultipleVariables;
        spec.multivariate = Some(self.to_encoding());
    }

    /// Emit `graph.setSpec` updating multivariate 5-channel encodings.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        Some(ToolCommandIntent::new(
            "graph.setSpec",
            json!({
                "sheet_id": id.as_str(),
                "kind": GraphKind::MultipleVariables,
                "multivariate": self.to_encoding(),
            }),
        ))
    }
}

/// `Apply Style From...` cross-graph style cloning dialog (`graph.applyStyleFrom`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ApplyStyleFromState {
    pub target_graph_id: Option<SheetId>,
    #[serde(default)]
    pub target_graph_ids: Vec<SheetId>,
    #[serde(default)]
    pub target_candidates: Vec<(SheetId, String)>,
    pub donor_graph_id: Option<SheetId>,
    pub donor_candidates: Vec<(SheetId, String)>,
    pub clone_palette: bool,
    pub clone_frame_and_axes: bool,
    pub clone_error_bars_and_legend: bool,
}

impl ApplyStyleFromState {
    /// Discover candidate donor & target graph sheets in `doc` for single target `target_graph_id`.
    pub fn open_for_graph(&mut self, target_graph_id: &SheetId, doc: &ProjectDocument) {
        self.open_for_graphs(std::slice::from_ref(target_graph_id), doc);
    }

    /// Discover candidate donor & target graph sheets in `doc` for one or more targets.
    pub fn open_for_graphs(&mut self, targets: &[SheetId], doc: &ProjectDocument) {
        self.target_candidates = doc.graph_sheets.iter().map(|g| (g.id.clone(), g.title.clone())).collect();
        let mut selected: Vec<SheetId> = Vec::new();
        for tid in targets {
            if doc.graph_sheet(tid).is_some() && !selected.contains(tid) {
                selected.push(tid.clone());
            }
        }
        if selected.is_empty()
            && let Some(first) = doc.graph_sheets.first()
        {
            selected.push(first.id.clone());
        }
        self.target_graph_id = selected.first().cloned();
        self.target_graph_ids = selected;
        self.clone_palette = true;
        self.clone_frame_and_axes = true;
        self.clone_error_bars_and_legend = true;

        let primary_single = (self.target_graph_ids.len() == 1).then(|| self.target_graph_ids[0].clone());
        let non_target_donors: Vec<(SheetId, String)> =
            doc.graph_sheets.iter().filter(|g| !self.target_graph_ids.contains(&g.id)).map(|g| (g.id.clone(), g.title.clone())).collect();
        self.donor_candidates = if let Some(ref single) = primary_single {
            doc.graph_sheets.iter().filter(|g| &g.id != single).map(|g| (g.id.clone(), g.title.clone())).collect()
        } else if !non_target_donors.is_empty() {
            let mut list = non_target_donors;
            for g in &doc.graph_sheets {
                if !list.iter().any(|(id, _)| id == &g.id) {
                    list.push((g.id.clone(), g.title.clone()));
                }
            }
            list
        } else {
            doc.graph_sheets.iter().map(|g| (g.id.clone(), g.title.clone())).collect()
        };
        self.donor_graph_id = self.donor_candidates.first().map(|(id, _)| id.clone());
    }

    /// Preview the styled [`GraphSpec`] without mutating target data bindings.
    #[must_use]
    pub fn preview_applied_spec(&self, doc: &ProjectDocument) -> Option<GraphSpec> {
        let target_id = self.target_graph_id.as_ref()?;
        let donor_id = self.donor_graph_id.as_ref()?;
        let target = doc.graph_sheet(target_id)?;
        let donor = doc.graph_sheet(donor_id)?;
        let mut preview = target.spec.clone();
        preview.apply_style_from(&donor.spec);
        if !self.clone_palette {
            preview.palette = target.spec.palette;
        }
        if !self.clone_frame_and_axes {
            preview.frame_preset = target.spec.frame_preset;
            preview.x_axis.stroke_width_pt = target.spec.x_axis.stroke_width_pt;
            preview.y_axis.stroke_width_pt = target.spec.y_axis.stroke_width_pt;
        }
        if !self.clone_error_bars_and_legend {
            preview.legend = target.spec.legend;
        }
        Some(preview)
    }

    /// Emit `graph.applyStyleFrom` command intent.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let donor = self.donor_graph_id.as_ref()?;
        if self.target_graph_ids.len() > 1 {
            let target_ids: Vec<&str> = self.target_graph_ids.iter().map(SheetId::as_str).collect();
            return Some(ToolCommandIntent::new(
                "graph.applyStyleFrom",
                json!({
                    "target_sheet_ids": target_ids,
                    "donor_sheet_id": donor.as_str(),
                    "clone_palette": self.clone_palette,
                    "clone_frame_and_axes": self.clone_frame_and_axes,
                    "clone_error_bars_and_legend": self.clone_error_bars_and_legend,
                }),
            ));
        }
        let target = self.target_graph_ids.first().or(self.target_graph_id.as_ref())?;
        Some(ToolCommandIntent::new(
            "graph.applyStyleFrom",
            json!({
                "sheet_id": target.as_str(),
                "donor_sheet_id": donor.as_str(),
                "clone_palette": self.clone_palette,
                "clone_frame_and_axes": self.clone_frame_and_axes,
                "clone_error_bars_and_legend": self.clone_error_bars_and_legend,
            }),
        ))
    }
}
