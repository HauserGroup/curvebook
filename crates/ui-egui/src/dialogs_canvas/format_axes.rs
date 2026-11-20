//! `Format Axes` inspector/dialog state machine (axis scale, range, ticks, segments, thresholds).

use curvebook_doc::{
    AdditionalTick, AxisSegment, AxisSpec, AxisThreshold, FramePreset, GraphAxisScale, GraphSheet, GraphSpec, ProjectDocument, SheetId, TickDirection,
};
use curvebook_palette::{LineDashStyle, Rgba8};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dialogs_r9::{norm_key, parse_axis_scale_str, parse_break_ornament_str, parse_finite_f64_with_locale, parse_opt_finite_f64_with_locale};
use crate::i18n::LocalePreset;

/// Which axis (`X`, `Y`, or optional right `Y2`) is being edited in `Format Axes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum AxisTarget {
    X,
    #[default]
    Y,
    Y2,
}

/// Non-modal / dialog state for `Format Axes` (scales, ranges, 1-3 discontinuous `AxisSegment`s).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormatAxesState {
    pub graph_sheet_id: Option<SheetId>,
    pub target_axis: AxisTarget,
    pub enable_y2: bool,
    pub axis_draft: AxisSpec,
    #[serde(default)]
    pub open_generation: u64,
    #[serde(default)]
    pub base_x_thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub base_y_thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub pending_x_thresholds: Option<Vec<AxisThreshold>>,
    #[serde(default)]
    pub pending_y_thresholds: Option<Vec<AxisThreshold>>,
    #[serde(default)]
    pub pending_x_additional_ticks: Option<Vec<AdditionalTick>>,
    #[serde(default)]
    pub pending_y_additional_ticks: Option<Vec<AdditionalTick>>,
    #[serde(default)]
    pub pending_y2_additional_ticks: Option<Vec<AdditionalTick>>,
}

impl Default for FormatAxesState {
    fn default() -> Self {
        Self {
            graph_sheet_id: None,
            target_axis: AxisTarget::Y,
            enable_y2: false,
            axis_draft: AxisSpec::linear("Y"),
            open_generation: 0,
            base_x_thresholds: Vec::new(),
            base_y_thresholds: Vec::new(),
            pending_x_thresholds: None,
            pending_y_thresholds: None,
            pending_x_additional_ticks: None,
            pending_y_additional_ticks: None,
            pending_y2_additional_ticks: None,
        }
    }
}

impl FormatAxesState {
    /// Load axis `target` (`X`, `Y`, or `Y2`) from `graph`.
    pub fn sync_from_graph(&mut self, graph: &GraphSheet, target: AxisTarget) {
        self.open_generation = self.open_generation.wrapping_add(1);
        self.graph_sheet_id = Some(graph.id.clone());
        self.target_axis = target;
        self.enable_y2 = graph.spec.y2_axis.is_some();
        self.base_x_thresholds = graph.spec.x_axis.thresholds.clone();
        self.base_y_thresholds = graph.spec.y_axis.thresholds.clone();
        self.pending_x_thresholds = None;
        self.pending_y_thresholds = None;
        self.pending_x_additional_ticks = None;
        self.pending_y_additional_ticks = None;
        self.pending_y2_additional_ticks = None;
        self.axis_draft = match target {
            AxisTarget::X => graph.spec.x_axis.clone(),
            AxisTarget::Y => graph.spec.y_axis.clone(),
            AxisTarget::Y2 => graph.spec.y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2")),
        };
    }

    /// Stage the active axis's additional ticks so they survive axis-tab switches until OK/Apply.
    pub fn stage_active_additional_ticks(&mut self) {
        let ticks = self.axis_draft.additional_ticks.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_additional_ticks = Some(ticks),
            AxisTarget::Y => self.pending_y_additional_ticks = Some(ticks),
            AxisTarget::Y2 => self.pending_y2_additional_ticks = Some(ticks),
        }
    }

    /// Stage the active axis's thresholds so they survive axis-tab switches until OK/Apply.
    pub fn stage_active_thresholds(&mut self) {
        let thresholds = self.axis_draft.thresholds.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_thresholds = Some(thresholds),
            AxisTarget::Y => self.pending_y_thresholds = Some(thresholds),
            AxisTarget::Y2 => {}
        }
    }

    /// Return the effective X-axis reference thresholds including un-committed inspector edits.
    #[must_use]
    pub fn effective_x_thresholds(&self) -> &[AxisThreshold] {
        if self.target_axis == AxisTarget::X {
            &self.axis_draft.thresholds
        } else if let Some(ref xs) = self.pending_x_thresholds {
            xs
        } else {
            &self.base_x_thresholds
        }
    }

    /// Return the effective Y-axis reference thresholds including un-committed inspector edits.
    #[must_use]
    pub fn effective_y_thresholds(&self) -> &[AxisThreshold] {
        if self.target_axis == AxisTarget::Y {
            &self.axis_draft.thresholds
        } else if let Some(ref ys) = self.pending_y_thresholds {
            ys
        } else {
            &self.base_y_thresholds
        }
    }

    fn set_x_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::X {
            self.axis_draft.thresholds = thresholds.clone();
        }
        self.pending_x_thresholds = Some(thresholds);
    }

    fn set_y_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::Y {
            self.axis_draft.thresholds = thresholds.clone();
        }
        self.pending_y_thresholds = Some(thresholds);
    }

    fn set_active_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        self.axis_draft.thresholds = thresholds.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_thresholds = Some(thresholds),
            AxisTarget::Y => self.pending_y_thresholds = Some(thresholds),
            AxisTarget::Y2 => {}
        }
    }

    fn append_thresholds_for_target(&mut self, target: AxisTarget, mut items: Vec<AxisThreshold>) {
        match target {
            AxisTarget::X => {
                let mut next = self.effective_x_thresholds().to_vec();
                next.append(&mut items);
                self.set_x_thresholds(next);
            }
            AxisTarget::Y => {
                let mut next = self.effective_y_thresholds().to_vec();
                next.append(&mut items);
                self.set_y_thresholds(next);
            }
            AxisTarget::Y2 => {
                self.axis_draft.thresholds.append(&mut items);
            }
        }
    }

    fn set_x_additional_ticks(&mut self, ticks: Vec<AdditionalTick>) {
        if self.target_axis == AxisTarget::X {
            self.axis_draft.additional_ticks = ticks.clone();
        }
        self.pending_x_additional_ticks = Some(ticks);
    }

    fn set_y_additional_ticks(&mut self, ticks: Vec<AdditionalTick>) {
        if self.target_axis == AxisTarget::Y {
            self.axis_draft.additional_ticks = ticks.clone();
        }
        self.pending_y_additional_ticks = Some(ticks);
    }

    fn set_y2_additional_ticks(&mut self, ticks: Vec<AdditionalTick>) {
        if self.target_axis == AxisTarget::Y2 {
            self.axis_draft.additional_ticks = ticks.clone();
        }
        self.pending_y2_additional_ticks = Some(ticks);
    }

    fn switch_target_axis_preserving_thresholds(&mut self, next: AxisTarget, doc: Option<&ProjectDocument>) {
        let preserved_generation = self.open_generation;
        let saved_x = (self.target_axis == AxisTarget::X).then(|| self.axis_draft.thresholds.clone()).or_else(|| self.pending_x_thresholds.clone());
        let saved_y = (self.target_axis == AxisTarget::Y).then(|| self.axis_draft.thresholds.clone()).or_else(|| self.pending_y_thresholds.clone());
        let saved_x_ticks =
            (self.target_axis == AxisTarget::X).then(|| self.axis_draft.additional_ticks.clone()).or_else(|| self.pending_x_additional_ticks.clone());
        let saved_y_ticks =
            (self.target_axis == AxisTarget::Y).then(|| self.axis_draft.additional_ticks.clone()).or_else(|| self.pending_y_additional_ticks.clone());
        let saved_y2_ticks = (self.target_axis == AxisTarget::Y2)
            .then(|| self.axis_draft.additional_ticks.clone())
            .or_else(|| self.pending_y2_additional_ticks.clone());
        if let (Some(d), Some(gid)) = (doc, &self.graph_sheet_id)
            && let Some(gs) = d.graph_sheet(gid)
        {
            self.sync_from_graph(gs, next);
        } else {
            self.target_axis = next;
        }
        self.open_generation = preserved_generation;
        if let Some(xs) = saved_x {
            self.set_x_thresholds(xs);
        }
        if let Some(ys) = saved_y {
            self.set_y_thresholds(ys);
        }
        if let Some(xt) = saved_x_ticks {
            self.set_x_additional_ticks(xt);
        }
        if let Some(yt) = saved_y_ticks {
            self.set_y_additional_ticks(yt);
        }
        if let Some(y2t) = saved_y2_ticks {
            self.set_y2_additional_ticks(y2t);
        }
    }

    /// Add a discontinuous [`AxisSegment`], enforcing `1..=3` bounds. Returns `false` if already at `3`.
    pub fn add_segment(&mut self) -> bool {
        if self.axis_draft.segments.len() >= 3 {
            return false;
        }
        self.axis_draft.segments.push(AxisSegment::default());
        true
    }

    /// Remove segment `idx`, enforcing at least `1` segment.
    pub fn remove_segment(&mut self, idx: usize) -> bool {
        if self.axis_draft.segments.len() <= 1 || idx >= self.axis_draft.segments.len() {
            return false;
        }
        self.axis_draft.segments.remove(idx);
        true
    }

    /// Update scale bijection (`Linear`, `Log10`, `Log2`, `Ln`, `Logit`, `Probit`, `Reciprocal`) on segment `idx`.
    pub fn set_segment_scale(&mut self, idx: usize, scale: GraphAxisScale) -> bool {
        let Some(seg) = self.axis_draft.segments.get_mut(idx) else {
            return false;
        };
        seg.scale = scale;
        true
    }

    /// Update min/max bounds on segment `idx`, rejecting non-finite or inverted `[min >= max]` ranges.
    pub fn set_segment_bounds(&mut self, idx: usize, min: Option<f64>, max: Option<f64>) -> Result<(), String> {
        if min.is_some_and(|v| !v.is_finite()) || max.is_some_and(|v| !v.is_finite()) {
            return Err(String::from("axis bounds must be finite numbers"));
        }
        if let (Some(lo), Some(hi)) = (min, max)
            && lo >= hi
        {
            return Err(format!("axis minimum ({lo}) must be strictly less than maximum ({hi})"));
        }
        let Some(seg) = self.axis_draft.segments.get_mut(idx) else {
            return Err(format!("segment index {idx} out of range"));
        };
        seg.min = min;
        seg.max = max;
        Ok(())
    }

    /// Add a horizontal/vertical threshold reference line or shaded band.
    pub fn add_threshold(&mut self, value: f64, shade_to: Option<f64>, label: Option<String>, color: Rgba8) -> bool {
        if !value.is_finite() || shade_to.is_some_and(|v| !v.is_finite()) {
            return false;
        }
        self.axis_draft.thresholds.push(AxisThreshold { value, shade_to, color, dash_style: LineDashStyle::Dashed, stroke_width_pt: 1.0, label });
        true
    }

    /// Write the validated axis draft back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) -> Result<(), String> {
        self.axis_draft.validate().map_err(|e| e.to_string())?;
        match self.target_axis {
            AxisTarget::X => spec.x_axis = self.axis_draft.clone(),
            AxisTarget::Y => spec.y_axis = self.axis_draft.clone(),
            AxisTarget::Y2 => {
                spec.y2_axis = self.enable_y2.then(|| self.axis_draft.clone());
                if self.enable_y2 {
                    spec.frame_preset = FramePreset::DualYSecondaryRight;
                }
            }
        }
        if self.target_axis != AxisTarget::X
            && let Some(ref xs) = self.pending_x_thresholds
        {
            spec.x_axis.thresholds = xs.clone();
        }
        if self.target_axis != AxisTarget::Y
            && let Some(ref ys) = self.pending_y_thresholds
        {
            spec.y_axis.thresholds = ys.clone();
        }
        if self.target_axis != AxisTarget::X
            && let Some(ref xt) = self.pending_x_additional_ticks
        {
            spec.x_axis.additional_ticks = xt.clone();
        }
        if self.target_axis != AxisTarget::Y
            && let Some(ref yt) = self.pending_y_additional_ticks
        {
            spec.y_axis.additional_ticks = yt.clone();
        }
        if self.target_axis != AxisTarget::Y2
            && let Some(ref y2t) = self.pending_y2_additional_ticks
            && let Some(ref mut y2) = spec.y2_axis
        {
            y2.additional_ticks = y2t.clone();
        }
        Ok(())
    }

    /// Emit `graph.setAxes` intent carrying axis target, validated segments, break ornament, thresholds, and staged additional ticks.
    #[must_use]
    pub fn commit_intent(&self) -> Option<ToolCommandIntent> {
        let id = self.graph_sheet_id.as_ref()?;
        if self.axis_draft.validate().is_err() {
            return None;
        }
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("sheet_id"), Value::String(id.as_str().to_owned()));
        match self.target_axis {
            AxisTarget::X => {
                payload.insert(String::from("x_axis"), serde_json::to_value(&self.axis_draft).ok()?);
            }
            AxisTarget::Y => {
                payload.insert(String::from("y_axis"), serde_json::to_value(&self.axis_draft).ok()?);
            }
            AxisTarget::Y2 => {
                if self.enable_y2 {
                    payload.insert(String::from("frame_preset"), serde_json::to_value(FramePreset::DualYSecondaryRight).ok()?);
                    payload.insert(String::from("y2_axis"), serde_json::to_value(&self.axis_draft).ok()?);
                } else {
                    payload.insert(String::from("y2_axis"), Value::Null);
                }
            }
        }
        if self.target_axis != AxisTarget::X
            && let Some(obj) = build_pending_axis_patch(self.pending_x_thresholds.as_deref(), self.pending_x_additional_ticks.as_deref())
        {
            payload.insert(String::from("x_axis"), Value::Object(obj));
        }
        if self.target_axis != AxisTarget::Y
            && let Some(obj) = build_pending_axis_patch(self.pending_y_thresholds.as_deref(), self.pending_y_additional_ticks.as_deref())
        {
            payload.insert(String::from("y_axis"), Value::Object(obj));
        }
        if self.target_axis != AxisTarget::Y2
            && self.enable_y2
            && let Some(obj) = build_pending_axis_patch(None, self.pending_y2_additional_ticks.as_deref())
        {
            payload.insert(String::from("y2_axis"), Value::Object(obj));
        }
        Some(ToolCommandIntent::new("graph.setAxes", Value::Object(payload)))
    }

    /// Apply modal/inspector field updates using regional numeric parsing (`preset`).
    pub fn apply_field_patch(&mut self, fields: &serde_json::Map<String, Value>, doc: Option<&ProjectDocument>) -> Result<(), String> {
        self.apply_field_patch_localized(fields, doc, &LocalePreset::period_comma("en-US"))
    }

    /// Apply modal/inspector field updates to the axis draft using regional numeric parsing (`preset`).
    pub fn apply_field_patch_localized(
        &mut self,
        fields: &serde_json::Map<String, Value>,
        doc: Option<&ProjectDocument>,
        preset: &LocalePreset,
    ) -> Result<(), String> {
        if let Some(target_val) = fields.get("target_axis").or_else(|| fields.get("axes.target_axis"))
            && let Some(raw) = target_val.as_str()
        {
            let next = parse_axis_target_str(raw)?;
            self.switch_target_axis_preserving_thresholds(next, doc);
        }
        let mut pending_min = self.axis_draft.segments.first().and_then(|s| s.min);
        let mut pending_max = self.axis_draft.segments.first().and_then(|s| s.max);
        let mut bounds_changed = false;
        for (key, val) in fields {
            match key.as_str() {
                "enable_y2" | "axes.enable_y2" => {
                    self.enable_y2 = val.as_bool().ok_or_else(|| String::from("`enable_y2` must be boolean"))?;
                }
                "title" | "axis_title" | "axes.title" => {
                    self.axis_draft.title = val.as_str().ok_or_else(|| String::from("`title` must be a string"))?.to_owned();
                }
                "scale" | "axes.scale" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`scale` must be a string"))?;
                    let scale = parse_axis_scale_str(raw)?;
                    let _ = self.set_segment_scale(0, scale);
                }
                "min" | "axis_min" | "axes.min" => {
                    pending_min = parse_opt_finite_f64_with_locale(val, key, preset)?;
                    bounds_changed = true;
                }
                "max" | "axis_max" | "axes.max" => {
                    pending_max = parse_opt_finite_f64_with_locale(val, key, preset)?;
                    bounds_changed = true;
                }
                "break_ornament" | "axes.break_ornament" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`break_ornament` must be a string"))?;
                    self.axis_draft.break_ornament = parse_break_ornament_str(raw)?;
                }
                "gap_width_pt" | "axes.gap_width_pt" => {
                    self.axis_draft.gap_width_pt = parse_finite_f64_with_locale(val, key, preset)?.clamp(1.0, 48.0);
                }
                "stroke_width_pt" | "axes.stroke_width_pt" => {
                    self.axis_draft.stroke_width_pt = parse_finite_f64_with_locale(val, key, preset)?.clamp(0.2, 8.0);
                }
                "tick_direction" | "axes.tick_direction" => {
                    let raw = val.as_str().ok_or_else(|| String::from("`tick_direction` must be a string"))?;
                    self.axis_draft.tick_direction = match norm_key(raw).as_str() {
                        "outside" => TickDirection::Outside,
                        "inside" => TickDirection::Inside,
                        "both" => TickDirection::Both,
                        "none" | "off" => TickDirection::None,
                        other => return Err(format!("unknown `tick_direction` `{other}`")),
                    };
                }
                "show_tick_labels" | "axes.show_tick_labels" => {
                    self.axis_draft.show_tick_labels = val.as_bool().ok_or_else(|| String::from("`show_tick_labels` must be boolean"))?;
                }
                "add_segment" | "axes.add_segment" if val.as_bool() == Some(true) => {
                    let _ = self.add_segment();
                }
                "remove_segment" | "axes.remove_segment" => {
                    if let Some(idx) = val.as_u64().map(|n| n as usize) {
                        let _ = self.remove_segment(idx);
                    }
                }
                "thresholds" | "axes.thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_active_thresholds(items);
                }
                "x_thresholds" | "axes.x_thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_x_thresholds(items);
                }
                "y_thresholds" | "axes.y_thresholds" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.set_y_thresholds(items);
                }
                "add_threshold" | "axes.add_threshold" => {
                    let axis = match val.get("axis") {
                        None | Some(Value::Null) => self.target_axis,
                        Some(other) => {
                            let s = other.as_str().ok_or_else(|| String::from("`axis` must be a string"))?;
                            parse_axis_target_str(s)?
                        }
                    };
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(axis, items);
                }
                "add_x_threshold" | "axes.add_x_threshold" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(AxisTarget::X, items);
                }
                "add_y_threshold" | "axes.add_y_threshold" => {
                    let items = parse_axis_thresholds_value(val, key, preset)?;
                    self.append_thresholds_for_target(AxisTarget::Y, items);
                }
                "clear_thresholds" | "axes.clear_thresholds" if val.as_bool() == Some(true) => {
                    self.set_active_thresholds(Vec::new());
                }
                "target_axis" | "axes.target_axis" | "add_segment" | "axes.add_segment" | "clear_thresholds" | "axes.clear_thresholds" => {}
                other => return Err(format!("unknown Format Axes field `{other}`")),
            }
        }
        if bounds_changed {
            self.set_segment_bounds(0, pending_min, pending_max)?;
        }
        Ok(())
    }
}

fn build_pending_axis_patch(
    thresholds: Option<&[AxisThreshold]>,
    additional_ticks: Option<&[AdditionalTick]>,
) -> Option<serde_json::Map<String, Value>> {
    if thresholds.is_none() && additional_ticks.is_none() {
        return None;
    }
    let mut obj = serde_json::Map::new();
    if let Some(ts) = thresholds {
        obj.insert(String::from("thresholds"), serde_json::to_value(ts).ok()?);
    }
    if let Some(ticks) = additional_ticks {
        obj.insert(String::from("additional_ticks"), serde_json::to_value(ticks).ok()?);
    }
    Some(obj)
}

fn parse_axis_target_str(raw: &str) -> Result<AxisTarget, String> {
    match norm_key(raw).as_str() {
        "x" => Ok(AxisTarget::X),
        "y" => Ok(AxisTarget::Y),
        "y2" => Ok(AxisTarget::Y2),
        _ => Err(format!("unknown target_axis `{raw}`")),
    }
}

fn parse_axis_thresholds_value(val: &Value, key: &str, preset: &LocalePreset) -> Result<Vec<AxisThreshold>, String> {
    match val {
        Value::Null => Ok(Vec::new()),
        Value::Array(arr) => arr.iter().map(|item| parse_single_axis_threshold(item, key, preset)).collect(),
        _ => Ok(vec![parse_single_axis_threshold(val, key, preset)?]),
    }
}

fn parse_single_axis_threshold(val: &Value, key: &str, preset: &LocalePreset) -> Result<AxisThreshold, String> {
    if val.is_number() || val.is_string() {
        let num = parse_finite_f64_with_locale(val, key, preset)?;
        return Ok(AxisThreshold {
            value: num,
            shade_to: None,
            color: Rgba8::rgb(120, 120, 120),
            dash_style: LineDashStyle::Dashed,
            stroke_width_pt: 1.0,
            label: None,
        });
    }
    if let Ok(mut parsed) = serde_json::from_value::<AxisThreshold>(val.clone()) {
        if !parsed.value.is_finite() || parsed.shade_to.is_some_and(|v| !v.is_finite()) {
            return Err(format!("`{key}` threshold bounds must be finite"));
        }
        parsed.stroke_width_pt = parsed.stroke_width_pt.clamp(0.2, 8.0);
        return Ok(parsed);
    }
    let obj = val.as_object().ok_or_else(|| format!("`{key}` entries must be objects or numbers"))?;
    let val_num = obj.get("value").ok_or_else(|| format!("`{key}` threshold missing `value`"))?;
    let value = parse_finite_f64_with_locale(val_num, key, preset)?;
    let shade_to = obj.get("shade_to").map(|v| parse_opt_finite_f64_with_locale(v, "shade_to", preset)).transpose()?.flatten();
    let label = match obj.get("label") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_str().ok_or_else(|| format!("`{key}` `label` must be a string"))?.to_owned()),
    };
    let color = match obj.get("color") {
        None | Some(Value::Null) => Rgba8::rgb(120, 120, 120),
        Some(v) => parse_threshold_color(v, key)?,
    };
    let dash_style = match obj.get("dash_style") {
        None | Some(Value::Null) => LineDashStyle::Dashed,
        Some(v) => parse_threshold_dash_style(v, key)?,
    };
    let stroke_width_pt = match obj.get("stroke_width_pt") {
        None | Some(Value::Null) => 1.0,
        Some(v) => parse_finite_f64_with_locale(v, "stroke_width_pt", preset)?.clamp(0.2, 8.0),
    };
    Ok(AxisThreshold { value, shade_to, color, dash_style, stroke_width_pt, label })
}

pub(super) fn parse_threshold_color(val: &Value, key: &str) -> Result<Rgba8, String> {
    if let Ok(c) = serde_json::from_value::<Rgba8>(val.clone()) {
        return Ok(c);
    }
    if let Some(hex) = val.as_str() {
        let s = hex.trim().trim_start_matches('#');
        if s.is_ascii() && (s.len() == 6 || s.len() == 8) {
            let r = u8::from_str_radix(&s[0..2], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let g = u8::from_str_radix(&s[2..4], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let b = u8::from_str_radix(&s[4..6], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))?;
            let a = if s.len() == 8 { u8::from_str_radix(&s[6..8], 16).map_err(|_| format!("invalid `{key}` hex color `{hex}`"))? } else { 255 };
            return Ok(Rgba8::rgba(r, g, b, a));
        }
    }
    Err(format!("invalid `{key}` color value"))
}

pub(super) fn parse_threshold_dash_style(val: &Value, key: &str) -> Result<LineDashStyle, String> {
    if let Ok(d) = serde_json::from_value::<LineDashStyle>(val.clone()) {
        return Ok(d);
    }
    if let Some(raw) = val.as_str() {
        return match norm_key(raw).as_str() {
            "solid" => Ok(LineDashStyle::Solid),
            "dashed" | "dash" => Ok(LineDashStyle::Dashed),
            "dotted" | "dot" => Ok(LineDashStyle::Dotted),
            "dashdot" | "dash_dot" => Ok(LineDashStyle::DashDot),
            _ => Err(format!("unknown `{key}` dash_style `{raw}`")),
        };
    }
    Err(format!("`{key}` dash_style must be a string"))
}
