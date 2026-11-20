//! Graph overlay & style inspectors: Pairwise Comparisons, Bubble Plot channels, and Apply Style From.

use curvebook_doc::{
    BracketLabelMode, BracketStyle, CldOrdering, ColorChannelMapping, CompactLetterDisplaySpec, GraphKind, GraphSheet, GraphSpec,
    MultivariateGraphEncoding, MultivariatePlotMode, PairwiseBracketSpec, PairwiseDisplayMode, ProjectDocument, SheetId, SizeChannelMapping,
    SummaryMeasure,
};
use curvebook_palette::{ColorRamp, ColormapId, MarkerStyle, PaletteId, Rgba8};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_r9::{
    cld_ordering_str, norm_key, pairwise_display_mode_str, parse_bracket_label_mode, parse_bracket_style_str, parse_cld_ordering_str,
    parse_colormap_id_str, parse_finite_f64_with_locale, parse_opt_finite_f64_with_locale, parse_pairwise_display_mode_str, parse_palette_str,
    parse_usize_min,
};
use crate::i18n::LocalePreset;

use super::analyze_dialog::{VariableCandidate, collect_variable_candidates};
use super::graph_type::build_gdo_for_option;
use super::parse_helpers::parse_dialog_num_with_locale;

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

    #[must_use]
    fn brackets_commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        let res_id = self.selected_result_sheet_id.as_ref()?;
        if self.brackets.is_empty() {
            return None;
        }
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
            "stroke_width_pt": 1.0,
            "font_size_pt": 9.0,
            "bracket_step_pt": 12.0,
        });
        if let ([first], Some(map)) = (self.brackets.as_slice(), payload.as_object_mut()) {
            map.insert(String::from("group_a"), json!(first.group_a));
            map.insert(String::from("group_b"), json!(first.group_b));
        }
        Some(ToolCommandIntent::new("graph.addComparisonBrackets", payload))
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
            && !self.brackets.is_empty()
            && let Some(brackets_intent) = self.brackets_commit_intent()
        {
            intents.push(brackets_intent);
        }
        if let Some(cld_intent) = self.cld_commit_intent() {
            intents.push(cld_intent);
        }
        intents
    }

    /// Apply field patches for `Pairwise Comparisons` brackets and Compact Letter Display (`WF-46`, `WF-50`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, doc, &LocalePreset::period_comma("en-US"))
    }

    /// Apply field patches using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(
        &mut self,
        fields: &serde_json::Map<String, Value>,
        doc: Option<&ProjectDocument>,
        preset: &LocalePreset,
    ) -> Result<(), String> {
        let mut auto_add = false;
        let mut explicit_mode = None;
        let mut explicit_enabled = None;
        for (key, val) in fields {
            match key.as_str() {
                "result_sheet_id" | "selected_result_sheet_id" | "pairwise.result_sheet_id" => {
                    let s = val.as_str().ok_or_else(|| String::from("`result_sheet_id` must be a string"))?.trim();
                    if !s.is_empty() {
                        let sid = SheetId::try_new(s).map_err(|e| e.to_string())?;
                        self.selected_result_sheet_id = Some(sid);
                    }
                }
                "bracket_style" | "style" | "pairwise.bracket_style" | "pairwise.style" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`bracket_style` must be a string"))?;
                    self.bracket_style = parse_bracket_style_str(raw)?;
                    self.brackets_dirty = true;
                }
                "label_mode" | "pairwise.label_mode" => {
                    self.label_mode = parse_bracket_label_mode(val)?;
                    self.brackets_dirty = true;
                }
                "hide_non_significant" | "hide_if_non_significant" | "pairwise.hide_non_significant" => {
                    self.hide_non_significant = val.as_bool().ok_or_else(|| String::from("`hide_non_significant` must be boolean"))?;
                    self.brackets_dirty = true;
                }
                "auto_add" | "pairwise.auto_add" if val.as_bool() == Some(true) => {
                    auto_add = true;
                }
                "display_mode" | "pairwise_display_mode" | "pairwise.display_mode" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`display_mode` must be a string"))?;
                    let mode = parse_pairwise_display_mode_str(raw)?;
                    self.display_mode = mode;
                    explicit_mode = Some(mode);
                }
                "cld_enabled" | "enabled" | "compact_letter_display" | "pairwise.cld_enabled" => {
                    let en = val.as_bool().ok_or_else(|| String::from("`cld_enabled` must be boolean"))?;
                    self.cld_enabled = en;
                    explicit_enabled = Some(en);
                }
                "cld_ordering" | "ordering" | "pairwise.cld_ordering" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`cld_ordering` must be a string"))?;
                    self.cld_ordering = parse_cld_ordering_str(raw)?;
                }
                "cld_uppercase" | "uppercase" | "pairwise.cld_uppercase" => {
                    self.cld_uppercase = val.as_bool().ok_or_else(|| String::from("`cld_uppercase` must be boolean"))?;
                }
                "cld_alpha_override" | "alpha_override" | "pairwise.cld_alpha_override" => {
                    let alpha = parse_opt_finite_f64_with_locale(val, "cld_alpha_override", preset)?;
                    if let Some(a) = alpha
                        && (a <= 0.0 || a >= 1.0)
                    {
                        return Err(format!("`cld_alpha_override` must lie in (0, 1), got {a}"));
                    }
                    self.cld_alpha_override = alpha;
                }
                "cld_font_size_pt" | "font_size_pt" | "pairwise.cld_font_size_pt" => {
                    let fs = parse_finite_f64_with_locale(val, "cld_font_size_pt", preset)?;
                    if fs <= 0.0 || fs > 144.0 {
                        return Err(format!("`cld_font_size_pt` must lie in (0, 144], got {fs}"));
                    }
                    self.cld_font_size_pt = fs;
                }
                "cld_offset_pt" | "offset_pt" | "pairwise.cld_offset_pt" => {
                    let off = parse_dialog_num_with_locale(val, "cld_offset_pt", preset)?;
                    if !(0.0..=200.0).contains(&off) {
                        return Err(format!("`cld_offset_pt` must lie in [0, 200], got {off}"));
                    }
                    self.cld_offset_pt = off;
                }
                "auto_add" | "pairwise.auto_add" => {}
                other => return Err(format!("unknown Pairwise Comparisons field `{other}`")),
            }
        }
        if let Some(en) = explicit_enabled
            && explicit_mode.is_none()
        {
            if en {
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
        } else if let Some(mode) = explicit_mode
            && explicit_enabled.is_none()
        {
            self.cld_enabled = mode != PairwiseDisplayMode::BracketsOnly;
        }
        for b in &mut self.brackets {
            b.style = self.bracket_style;
            b.label_mode = self.label_mode.clone();
            b.hide_if_non_significant = self.hide_non_significant;
        }
        if auto_add && let Some(d) = doc {
            let _ = self.auto_add_from_analysis(d);
            self.brackets_dirty = true;
        }
        Ok(())
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
            x_level_order_override: None,
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

    /// Apply field patches for 5-channel `MultipleVariables` bubble & categorical summary plots (`WF-44`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        let mut color_mode: Option<String> = None;
        let mut color_col: Option<usize> = None;
        let mut palette_override: Option<PaletteId> = None;
        let mut colormap_override: Option<ColormapId> = None;
        for (key, val) in fields {
            match key.as_str() {
                "x_column" | "bubble.x_column" => self.x_column = parse_usize_min(val, key, 0)?,
                "y_column" | "bubble.y_column" => self.y_column = parse_usize_min(val, key, 0)?,
                "size_column" | "bubble.size_column" => {
                    if val.is_null() {
                        self.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
                    } else {
                        self.set_size_variable(parse_usize_min(val, key, 0)?, 2.5, 14.0);
                    }
                }
                "label_column" | "group_column" | "bubble.label_column" => {
                    self.label_column = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                }
                "color_mode" | "bubble.color_mode" => {
                    color_mode = val.as_str().map(norm_key);
                }
                "color_column" | "bubble.color_column" => {
                    color_col = if val.is_null() { None } else { Some(parse_usize_min(val, key, 0)?) };
                }
                "palette" | "bubble.palette" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`palette` must be a string"))?;
                    palette_override = Some(parse_palette_str(raw)?);
                }
                "colormap" | "bubble.colormap" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`colormap` must be a string"))?;
                    colormap_override = Some(parse_colormap_id_str(raw)?);
                }
                "show_convex_hulls" | "bubble.show_convex_hulls" => {
                    self.show_convex_hulls = val.as_bool().ok_or_else(|| String::from("`show_convex_hulls` must be boolean"))?;
                }
                "show_confidence_ellipses" | "bubble.show_confidence_ellipses" => {
                    self.show_confidence_ellipses = val.as_bool().ok_or_else(|| String::from("`show_confidence_ellipses` must be boolean"))?;
                }
                "plot_mode" | "categorical_summary" | "bubble.plot_mode" => {
                    let is_cat = val.as_bool().unwrap_or_else(|| val.as_str().is_some_and(|s| norm_key(s).contains("categorical")));
                    self.plot_mode = if is_cat {
                        MultivariatePlotMode::CategoricalSummary {
                            gdo: build_gdo_for_option("col.box_whisker", self.palette_fallback(), MarkerStyle::default(), SummaryMeasure::Mean),
                            y_error_bars: None,
                        }
                    } else {
                        MultivariatePlotMode::BubbleScatter
                    };
                }
                "bubble.color_channel" | "color_channel" => {
                    if let Ok(ch) = serde_json::from_value::<ColorChannelMapping>(val.clone()) {
                        self.color_channel = ch;
                    } else if val.is_null() {
                        self.color_channel = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0));
                    } else if let Some(s) = val.as_str() {
                        let norm = norm_key(s);
                        if norm.contains("constant") {
                            color_mode = Some(String::from("constant"));
                        } else if norm.contains("continuous") {
                            color_mode = Some(String::from("continuous"));
                        } else if norm.contains("categorical") {
                            color_mode = Some(String::from("categorical"));
                        } else {
                            return Err(format!("unsupported `{key}` value `{s}`"));
                        }
                    } else {
                        return Err(format!("`{key}` must be a ColorChannelMapping object, mode string, or null"));
                    }
                }
                "bubble.size_channel" | "size_channel" => {
                    if let Ok(sz) = serde_json::from_value::<SizeChannelMapping>(val.clone()) {
                        self.size_channel = sz;
                    } else if val.is_null() {
                        self.size_channel = SizeChannelMapping::Constant { radius_pt: 5.0 };
                    } else if let Ok(col) = parse_usize_min(val, key, 0) {
                        self.set_size_variable(col, 2.5, 14.0);
                    } else {
                        return Err(format!("`{key}` must be a SizeChannelMapping object, column index, or null"));
                    }
                }
                other => return Err(format!("unknown Bubble Plot field `{other}`")),
            }
        }
        if color_mode.is_some() || color_col.is_some() || palette_override.is_some() || colormap_override.is_some() {
            match color_mode.as_deref() {
                Some("constant") => self.color_channel = ColorChannelMapping::Constant(PaletteId::OkabeIto.color_at(0)),
                Some("continuous") => {
                    let col = color_col.unwrap_or(self.y_column);
                    self.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
                }
                Some("categorical") => {
                    let col = color_col.unwrap_or(self.label_column.unwrap_or(0));
                    self.set_color_categorical(col, palette_override.unwrap_or(PaletteId::OkabeIto));
                }
                _ if colormap_override.is_some() => {
                    let col = color_col.unwrap_or(self.y_column);
                    self.set_color_continuous(col, colormap_override.unwrap_or(ColormapId::Viridis), None);
                }
                _ if color_col.is_some() => {
                    self.set_color_categorical(color_col.unwrap_or(0), palette_override.unwrap_or(PaletteId::OkabeIto));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn palette_fallback(&self) -> Rgba8 {
        PaletteId::OkabeIto.color_at(0)
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

    /// Apply modal field patches for `Apply Style From...` (`WF-48`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>) -> Result<(), String> {
        for (key, val) in fields {
            match key.as_str() {
                "donor_graph_id" | "donor_sheet_id" | "style.donor_graph_id" | "apply_style.donor_graph_id" => {
                    let s = val.as_str().ok_or_else(|| String::from("`donor_graph_id` must be a string"))?.trim();
                    if !s.is_empty() {
                        self.donor_graph_id = Some(SheetId::from(s));
                    }
                }
                "target_sheet_ids" | "target_graph_ids" | "apply_style.target_sheet_ids" => {
                    let arr = val.as_array().ok_or_else(|| String::from("`target_sheet_ids` must be an array"))?;
                    let mut ids = Vec::with_capacity(arr.len());
                    for entry in arr {
                        let s = entry.as_str().ok_or_else(|| String::from("target sheet ID must be a string"))?;
                        let sid = SheetId::from(s);
                        if !ids.contains(&sid) {
                            ids.push(sid);
                        }
                    }
                    self.target_graph_id = ids.first().cloned();
                    self.target_graph_ids = ids;
                }
                "clone_palette" | "style.clone_palette" | "apply_style.clone_palette" => {
                    self.clone_palette = val.as_bool().ok_or_else(|| String::from("`clone_palette` must be boolean"))?;
                }
                "clone_frame_and_axes" | "style.clone_frame_and_axes" | "apply_style.clone_frame_and_axes" => {
                    self.clone_frame_and_axes = val.as_bool().ok_or_else(|| String::from("`clone_frame_and_axes` must be boolean"))?;
                }
                "clone_error_bars_and_legend" | "style.clone_error_bars_and_legend" | "apply_style.clone_error_bars_and_legend" => {
                    self.clone_error_bars_and_legend = val.as_bool().ok_or_else(|| String::from("`clone_error_bars_and_legend` must be boolean"))?;
                }
                other => return Err(format!("unknown Apply Style From field `{other}`")),
            }
        }
        Ok(())
    }
}
