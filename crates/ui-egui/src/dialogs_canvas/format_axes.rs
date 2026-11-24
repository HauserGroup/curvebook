//! `Format Axes` inspector/dialog state machine (axis scale, range, ticks, segments, thresholds).

use curvebook_doc::{
    AdditionalTick, AxisSegment, AxisSpec, AxisThreshold, FramePreset, GraphAxisScale, GraphSheet, GraphSpec, ProjectDocument, SheetId,
};
use curvebook_palette::{LineDashStyle, Rgba8};

use curvebook_tools::ToolCommandIntent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    pub pending_y2_thresholds: Option<Vec<AxisThreshold>>,
    #[serde(default)]
    pub pending_x_additional_ticks: Option<Vec<AdditionalTick>>,
    #[serde(default)]
    pub pending_y_additional_ticks: Option<Vec<AdditionalTick>>,
    #[serde(default)]
    pub pending_y2_additional_ticks: Option<Vec<AdditionalTick>>,
    #[serde(default)]
    pub base_y2_thresholds: Vec<AxisThreshold>,
    #[serde(default)]
    pub staged_x_axis: Option<AxisSpec>,
    #[serde(default)]
    pub staged_y_axis: Option<AxisSpec>,
    #[serde(default)]
    pub staged_y2_axis: Option<AxisSpec>,
    #[serde(default)]
    pub baseline_x_axis: Option<AxisSpec>,
    #[serde(default)]
    pub baseline_y_axis: Option<AxisSpec>,
    #[serde(default)]
    pub baseline_y2_axis: Option<AxisSpec>,
    #[serde(default)]
    pub initial_enable_y2: bool,
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
            pending_y2_thresholds: None,
            pending_x_additional_ticks: None,
            pending_y_additional_ticks: None,
            pending_y2_additional_ticks: None,
            base_y2_thresholds: Vec::new(),
            staged_x_axis: None,
            staged_y_axis: None,
            staged_y2_axis: None,
            baseline_x_axis: None,
            baseline_y_axis: None,
            baseline_y2_axis: None,
            initial_enable_y2: false,
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
        self.initial_enable_y2 = self.enable_y2;
        self.base_x_thresholds = graph.spec.x_axis.thresholds.clone();
        self.base_y_thresholds = graph.spec.y_axis.thresholds.clone();
        self.base_y2_thresholds = graph.spec.y2_axis.as_ref().map_or_else(Vec::new, |y2| y2.thresholds.clone());
        self.pending_x_thresholds = None;
        self.pending_y_thresholds = None;
        self.pending_y2_thresholds = None;
        self.pending_x_additional_ticks = None;
        self.pending_y_additional_ticks = None;
        self.pending_y2_additional_ticks = None;
        self.staged_x_axis = None;
        self.staged_y_axis = None;
        self.staged_y2_axis = None;
        self.baseline_x_axis = Some(graph.spec.x_axis.clone());
        self.baseline_y_axis = Some(graph.spec.y_axis.clone());
        self.baseline_y2_axis = graph.spec.y2_axis.clone();
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
            AxisTarget::Y2 => self.pending_y2_thresholds = Some(thresholds),
        }
    }

    /// Return the effective X-axis reference thresholds including un-committed inspector edits.
    #[must_use]
    pub fn effective_x_thresholds(&self) -> &[AxisThreshold] {
        if self.target_axis == AxisTarget::X {
            &self.axis_draft.thresholds
        } else if let Some(ref xs) = self.pending_x_thresholds {
            xs
        } else if let Some(ref staged) = self.staged_x_axis {
            &staged.thresholds
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
        } else if let Some(ref staged) = self.staged_y_axis {
            &staged.thresholds
        } else {
            &self.base_y_thresholds
        }
    }

    pub(super) fn set_x_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::X {
            self.axis_draft.thresholds = thresholds.clone();
        }
        if let Some(ref mut staged) = self.staged_x_axis {
            staged.thresholds = thresholds.clone();
        }
        self.pending_x_thresholds = Some(thresholds);
    }

    pub(super) fn set_y_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::Y {
            self.axis_draft.thresholds = thresholds.clone();
        }
        if let Some(ref mut staged) = self.staged_y_axis {
            staged.thresholds = thresholds.clone();
        }
        self.pending_y_thresholds = Some(thresholds);
    }

    fn set_y2_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        if self.target_axis == AxisTarget::Y2 {
            self.axis_draft.thresholds = thresholds.clone();
        }
        if let Some(ref mut staged) = self.staged_y2_axis {
            staged.thresholds = thresholds.clone();
        }
        self.pending_y2_thresholds = Some(thresholds);
    }

    pub(super) fn set_active_thresholds(&mut self, thresholds: Vec<AxisThreshold>) {
        self.axis_draft.thresholds = thresholds.clone();
        match self.target_axis {
            AxisTarget::X => self.pending_x_thresholds = Some(thresholds),
            AxisTarget::Y => self.pending_y_thresholds = Some(thresholds),
            AxisTarget::Y2 => self.pending_y2_thresholds = Some(thresholds),
        }
    }

    pub(super) fn append_thresholds_for_target(&mut self, target: AxisTarget, mut items: Vec<AxisThreshold>) {
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
                let mut next = if self.target_axis == AxisTarget::Y2 {
                    self.axis_draft.thresholds.clone()
                } else if let Some(ref y2s) = self.pending_y2_thresholds {
                    y2s.clone()
                } else if let Some(ref staged) = self.staged_y2_axis {
                    staged.thresholds.clone()
                } else {
                    self.base_y2_thresholds.clone()
                };
                next.append(&mut items);
                self.set_y2_thresholds(next);
            }
        }
    }

    fn stash_active_axis_draft(&mut self) {
        let draft = self.axis_draft.clone();
        match self.target_axis {
            AxisTarget::X => {
                self.pending_x_thresholds = Some(draft.thresholds.clone());
                self.pending_x_additional_ticks = Some(draft.additional_ticks.clone());
                self.staged_x_axis = Some(draft);
            }
            AxisTarget::Y => {
                self.pending_y_thresholds = Some(draft.thresholds.clone());
                self.pending_y_additional_ticks = Some(draft.additional_ticks.clone());
                self.staged_y_axis = Some(draft);
            }
            AxisTarget::Y2 => {
                self.pending_y2_thresholds = Some(draft.thresholds.clone());
                self.pending_y2_additional_ticks = Some(draft.additional_ticks.clone());
                self.staged_y2_axis = Some(draft);
            }
        }
    }

    fn resolve_axis_from_sources(&self, axis: AxisTarget, doc: Option<&ProjectDocument>) -> Option<AxisSpec> {
        let mut resolved = match axis {
            AxisTarget::X => self
                .staged_x_axis
                .clone()
                .or_else(|| doc.and_then(|d| self.graph_sheet_id.as_ref().and_then(|gid| d.graph_sheet(gid))).map(|gs| gs.spec.x_axis.clone()))
                .or_else(|| self.baseline_x_axis.clone()),
            AxisTarget::Y => self
                .staged_y_axis
                .clone()
                .or_else(|| doc.and_then(|d| self.graph_sheet_id.as_ref().and_then(|gid| d.graph_sheet(gid))).map(|gs| gs.spec.y_axis.clone()))
                .or_else(|| self.baseline_y_axis.clone()),
            AxisTarget::Y2 => self
                .staged_y2_axis
                .clone()
                .or_else(|| doc.and_then(|d| self.graph_sheet_id.as_ref().and_then(|gid| d.graph_sheet(gid))).and_then(|gs| gs.spec.y2_axis.clone()))
                .or_else(|| self.baseline_y2_axis.clone()),
        };
        if resolved.is_none() && axis == AxisTarget::Y2 {
            resolved = Some(AxisSpec::linear("Y2"));
        }
        if let Some(ref mut ax) = resolved {
            match axis {
                AxisTarget::X => {
                    if let Some(ref xs) = self.pending_x_thresholds {
                        ax.thresholds = xs.clone();
                    }
                    if let Some(ref xt) = self.pending_x_additional_ticks {
                        ax.additional_ticks = xt.clone();
                    }
                }
                AxisTarget::Y => {
                    if let Some(ref ys) = self.pending_y_thresholds {
                        ax.thresholds = ys.clone();
                    }
                    if let Some(ref yt) = self.pending_y_additional_ticks {
                        ax.additional_ticks = yt.clone();
                    }
                }
                AxisTarget::Y2 => {
                    if let Some(ref y2s) = self.pending_y2_thresholds {
                        ax.thresholds = y2s.clone();
                    }
                    if let Some(ref y2t) = self.pending_y2_additional_ticks {
                        ax.additional_ticks = y2t.clone();
                    }
                }
            }
        }
        resolved
    }

    pub(super) fn switch_target_axis_preserving_thresholds(&mut self, next: AxisTarget, doc: Option<&ProjectDocument>) {
        if next == self.target_axis {
            return;
        }
        self.stash_active_axis_draft();
        self.target_axis = next;
        if let Some(next_draft) = self.resolve_axis_from_sources(next, doc) {
            self.axis_draft = next_draft;
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

    /// Write the validated axis drafts back onto `spec`.
    pub fn apply_to_spec(&self, spec: &mut GraphSpec) -> Result<(), String> {
        self.axis_draft.validate().map_err(|e| e.to_string())?;
        if let Some(ref sx) = self.staged_x_axis
            && self.target_axis != AxisTarget::X
        {
            sx.validate().map_err(|e| e.to_string())?;
            spec.x_axis = sx.clone();
        }
        if let Some(ref sy) = self.staged_y_axis
            && self.target_axis != AxisTarget::Y
        {
            sy.validate().map_err(|e| e.to_string())?;
            spec.y_axis = sy.clone();
        }
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
        if self.target_axis != AxisTarget::Y2 {
            if self.enable_y2 {
                if let Some(ref sy2) = self.staged_y2_axis {
                    sy2.validate().map_err(|e| e.to_string())?;
                    spec.y2_axis = Some(sy2.clone());
                } else if spec.y2_axis.is_none() {
                    spec.y2_axis = Some(self.baseline_y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2")));
                }
                if !self.initial_enable_y2 {
                    spec.frame_preset = FramePreset::DualYSecondaryRight;
                }
            } else {
                spec.y2_axis = None;
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
        if self.target_axis != AxisTarget::Y2
            && let Some(ref y2s) = self.pending_y2_thresholds
            && let Some(ref mut y2) = spec.y2_axis
        {
            y2.thresholds = y2s.clone();
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
        if self.target_axis != AxisTarget::X {
            insert_secondary_axis_payload(
                &mut payload,
                "x_axis",
                self.staged_x_axis.as_ref(),
                self.pending_x_thresholds.as_deref(),
                self.pending_x_additional_ticks.as_deref(),
            )?;
        }
        if self.target_axis != AxisTarget::Y {
            insert_secondary_axis_payload(
                &mut payload,
                "y_axis",
                self.staged_y_axis.as_ref(),
                self.pending_y_thresholds.as_deref(),
                self.pending_y_additional_ticks.as_deref(),
            )?;
        }
        if self.target_axis != AxisTarget::Y2 {
            self.insert_secondary_y2_payload(&mut payload)?;
        }
        Some(ToolCommandIntent::new("graph.setAxes", Value::Object(payload)))
    }

    fn insert_secondary_y2_payload(&self, payload: &mut serde_json::Map<String, Value>) -> Option<()> {
        if !self.enable_y2 {
            if self.initial_enable_y2 || self.staged_y2_axis.is_some() || self.baseline_y2_axis.is_some() {
                payload.insert(String::from("y2_axis"), Value::Null);
            }
            return Some(());
        }
        if !self.initial_enable_y2 {
            payload.insert(String::from("frame_preset"), serde_json::to_value(FramePreset::DualYSecondaryRight).ok()?);
        }
        let fallback_y2;
        let staged = match self.staged_y2_axis.as_ref() {
            Some(ax) => Some(ax),
            None if !self.initial_enable_y2 => {
                fallback_y2 = self.baseline_y2_axis.clone().unwrap_or_else(|| AxisSpec::linear("Y2"));
                Some(&fallback_y2)
            }
            None => None,
        };
        insert_secondary_axis_payload(payload, "y2_axis", staged, self.pending_y2_thresholds.as_deref(), self.pending_y2_additional_ticks.as_deref())
    }
}

fn insert_secondary_axis_payload(
    payload: &mut serde_json::Map<String, Value>,
    field_key: &str,
    staged: Option<&AxisSpec>,
    thresholds: Option<&[AxisThreshold]>,
    additional_ticks: Option<&[AdditionalTick]>,
) -> Option<()> {
    if let Some(base) = staged {
        let mut ax = base.clone();
        if let Some(ts) = thresholds {
            ax.thresholds = ts.to_vec();
        }
        if let Some(ticks) = additional_ticks {
            ax.additional_ticks = ticks.to_vec();
        }
        if ax.validate().is_err() {
            return None;
        }
        payload.insert(String::from(field_key), serde_json::to_value(&ax).ok()?);
    } else if let Some(obj) = build_pending_axis_patch(thresholds, additional_ticks) {
        payload.insert(String::from(field_key), Value::Object(obj));
    }
    Some(())
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
