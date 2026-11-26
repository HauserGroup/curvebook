//! QOL workspace & point-selection dialogs: Convert Archetype, Exclude With Reason, Split Workspace, Assumption Banner, Format Selected Points.

use curvebook_chart::ChartScene;
use curvebook_doc::{ChecklistStatus, DataSheet, ProjectDocument, ResultSheet, ResultTabKind, SheetId, TableArchetype, TableKind};

use curvebook_tools::{GridCellCoord, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::dialogs_r9::{is_inconclusive_info_item, sanitize_diagnostics_only_text};

// ============================================================================
// QOL-1 / QOL-3 / QOL-4 / QOL-7 / QOL-8 Dialog & Workspace Models
// ============================================================================

/// Non-destructive table archetype conversion modal state (`QOL-1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertArchetypeDialogState {
    pub sheet_id: Option<SheetId>,
    pub source_kind: Option<TableKind>,
    pub target_kind: TableKind,
    pub allow_lossy: bool,
    #[serde(default)]
    pub preview_only: bool,
    pub preview_ready: bool,
    pub preview_lossy: bool,
    pub preserved_cells: usize,
    pub dropped_cells: usize,
    pub preserved_exclusions: usize,
    pub preview_warnings: Vec<String>,
}

impl Default for ConvertArchetypeDialogState {
    fn default() -> Self {
        Self {
            sheet_id: None,
            source_kind: None,
            target_kind: TableKind::Column,
            allow_lossy: false,
            preview_only: false,
            preview_ready: false,
            preview_lossy: false,
            preserved_cells: 0,
            dropped_cells: 0,
            preserved_exclusions: 0,
            preview_warnings: Vec::new(),
        }
    }
}

impl ConvertArchetypeDialogState {
    /// Supported conversion destination archetypes for `table.convertArchetype`.
    #[must_use]
    pub const fn supported_targets() -> &'static [TableKind] {
        &[TableKind::Column, TableKind::Grouped, TableKind::Xy, TableKind::MultipleVariables, TableKind::Contingency]
    }

    /// Initialize conversion dialog state for `sheet`.
    pub fn open_for_sheet(&mut self, sheet: &DataSheet) {
        let current = sheet.kind();
        self.sheet_id = Some(sheet.id.clone());
        self.source_kind = Some(current);
        self.target_kind = match current {
            TableKind::Column => TableKind::Grouped,
            TableKind::Grouped => TableKind::Column,
            TableKind::Xy => TableKind::MultipleVariables,
            _ => TableKind::Column,
        };
        self.allow_lossy = false;
        self.preview_only = false;
        self.preview_ready = false;
        self.preview_lossy = false;
        self.preserved_cells = 0;
        self.dropped_cells = 0;
        self.preserved_exclusions = 0;
        self.preview_warnings.clear();
    }

    /// Set a new target archetype, resetting stale preview state.
    pub fn set_target_kind(&mut self, target: TableKind) {
        if self.target_kind != target {
            self.target_kind = target;
            self.preview_ready = false;
        }
    }

    /// Build a read-only `table.convertArchetype` (`preview_only: true`) intent.
    #[must_use]
    pub fn preview_intent(&self) -> Option<ToolCommandIntent> {
        let sheet_id = self.sheet_id.as_ref()?;
        Some(ToolCommandIntent::new(
            "table.convertArchetype",
            json!({
                "sheet_id": sheet_id.as_str(),
                "target_kind": convert_target_wire_name(self.target_kind),
                "preview_only": true,
            }),
        ))
    }

    /// Populate preview impact fields from engine `table.convertArchetype` preview JSON output.
    pub fn apply_preview_payload(&mut self, payload: &Value) {
        let p = payload.get("preview").unwrap_or(payload);
        self.preview_lossy = p.get("lossy").and_then(Value::as_bool).unwrap_or(false);
        self.preserved_cells = p.get("preserved_cells").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.dropped_cells = p.get("dropped_cells").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.preserved_exclusions = p.get("preserved_exclusions").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).unwrap_or(0);
        self.preview_warnings = p
            .get("warnings")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|w| w.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        self.preview_ready = true;
    }

    /// Build the mutating `table.convertArchetype` intent, enforcing explicit `allow_lossy` when required.
    ///
    /// # Errors
    /// Returns an error when no sheet is bound or when a lossy conversion awaits explicit confirmation.
    pub fn commit_intent(&self) -> Result<ToolCommandIntent, String> {
        let sheet_id = self.sheet_id.as_ref().ok_or_else(|| String::from("no data sheet selected for archetype conversion"))?;
        if self.preview_lossy && !self.allow_lossy {
            return Err(format!(
                "lossy conversion to {} drops {} cell(s); enable allow_lossy to proceed",
                self.target_kind.label(),
                self.dropped_cells
            ));
        }
        Ok(ToolCommandIntent::new(
            "table.convertArchetype",
            json!({
                "sheet_id": sheet_id.as_str(),
                "target_kind": convert_target_wire_name(self.target_kind),
                "allow_lossy": self.allow_lossy,
                "preview_only": false,
            }),
        ))
    }
}

fn convert_target_wire_name(kind: TableKind) -> &'static str {
    match kind {
        TableKind::Column => "Column",
        TableKind::Grouped => "Grouped",
        TableKind::Xy => "Xy",
        TableKind::MultipleVariables => "MultipleVariables",
        TableKind::Contingency => "Contingency",
        _ => "Column",
    }
}

/// Modal dialog state for auditable exclusion-with-reason (`QOL-3`, `Cmd+Shift+E`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludeWithReasonDialogState {
    pub sheet_id: Option<SheetId>,
    pub coords: Vec<GridCellCoord>,
    pub reason_tag: String,
    pub note: String,
    pub timestamp_utc: Option<String>,
}

impl Default for ExcludeWithReasonDialogState {
    fn default() -> Self {
        Self { sheet_id: None, coords: Vec::new(), reason_tag: String::from("Outlier"), note: String::new(), timestamp_utc: None }
    }
}

impl ExcludeWithReasonDialogState {
    /// Preset reason tags offered in the exclusion audit dialog.
    #[must_use]
    pub const fn preset_reason_tags() -> &'static [&'static str] {
        &["Outlier", "Contamination", "Instrument Fault", "Bubble", "Missing Replicate"]
    }

    /// Initialize dialog state targeting `coords` on `sheet_id`.
    pub fn open_for_coords(&mut self, sheet_id: SheetId, coords: Vec<GridCellCoord>) {
        *self = Self::default();
        self.sheet_id = Some(sheet_id);
        self.coords = coords;
    }

    /// Build the `table.excludeCellsWithReason` command intent.
    ///
    /// # Errors
    /// Returns an error if no cells are selected or `reason_tag` is empty.
    pub fn commit_intent(&self) -> Result<ToolCommandIntent, String> {
        let sheet_id = self.sheet_id.as_ref().ok_or_else(|| String::from("no data sheet selected for exclusion"))?;
        if self.coords.is_empty() {
            return Err(String::from("at least one grid cell must be selected before excluding with reason"));
        }
        let tag = self.reason_tag.trim();
        if tag.is_empty() {
            return Err(String::from("exclusion reason_tag cannot be empty"));
        }
        let coords_json: Vec<Value> = self.coords.iter().map(|c| json!({ "row": c.row, "col": c.col, "subcol": c.subcol })).collect();
        let mut params = json!({
            "sheet_id": sheet_id.as_str(),
            "coords": coords_json,
            "reason_tag": tag,
            "note": self.note,
        });
        if let Some(ts) = &self.timestamp_utc
            && !ts.trim().is_empty()
            && let Some(obj) = params.as_object_mut()
        {
            obj.insert(String::from("timestamp_utc"), Value::String(ts.trim().to_owned()));
        }
        Ok(ToolCommandIntent::new("table.excludeCellsWithReason", params))
    }
}

/// Simultaneous side-by-side Data/Result + live Graph split-pane workspace configuration (`QOL-4`, `R20.3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitWorkspaceState {
    pub enabled: bool,
    /// Fraction of center canvas width allocated to the primary pane in `[0.15, 0.85]`.
    pub ratio: f64,
    pub primary_sheet_id: Option<SheetId>,
    pub secondary_sheet_id: Option<SheetId>,
    #[serde(default)]
    pub dragging_divider: bool,
    /// Ephemeral Inspector target sheet ID (`gui-layout-spec.md` §2.4; never persisted).
    #[serde(skip)]
    pub inspector_target_sheet_id: Option<SheetId>,
    /// Whether the Trailing Studio Inspector stays pinned to [`Self::inspector_target_sheet_id`]
    /// while Split Comparison is active (`gui-layout-spec.md` §2.4; never persisted).
    #[serde(skip)]
    pub inspector_pinned: bool,
}

impl Default for SplitWorkspaceState {
    fn default() -> Self {
        Self {
            enabled: false,
            ratio: 0.5,
            primary_sheet_id: None,
            secondary_sheet_id: None,
            dragging_divider: false,
            inspector_target_sheet_id: None,
            inspector_pinned: false,
        }
    }
}

impl SplitWorkspaceState {
    /// Default primary/secondary split ratio for `Data + Graph` (`40/60`, `gui-layout-spec.md` §2.3).
    pub const DATA_GRAPH_DEFAULT_RATIO: f64 = 0.40;

    /// Clamp split divider fraction into `[0.15, 0.85]`.
    pub fn set_ratio(&mut self, ratio: f64) {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(0.15, 0.85);
        }
    }

    /// Resolve concrete `(primary_sheet_id, secondary_sheet_id)` pair against `doc`.
    #[must_use]
    pub fn resolve_sheets(&self, doc: &ProjectDocument) -> (Option<SheetId>, Option<SheetId>) {
        let active = doc.active_sheet_id.clone();
        let primary = self
            .primary_sheet_id
            .clone()
            .filter(|id| doc.sheet_section(id).is_some())
            .or_else(|| {
                active.as_ref().and_then(|id| {
                    if doc.graph_sheet(id).is_some() {
                        doc.graph_sheet(id)
                            .map(|gs| gs.spec.primary_source_sheet_id.clone())
                            .filter(|src| doc.data_sheet(src).is_some())
                            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()))
                    } else {
                        Some(id.clone())
                    }
                })
            })
            .or_else(|| doc.data_sheets.first().map(|d| d.id.clone()));

        let secondary = self.secondary_sheet_id.clone().filter(|id| doc.sheet_section(id).is_some()).or_else(|| {
            if let Some(act) = &active
                && (doc.graph_sheet(act).is_some() || doc.layout_sheet(act).is_some())
            {
                return Some(act.clone());
            }
            if let Some(prim) = &primary {
                if let Some(gs) = doc.graph_sheets.iter().find(|g| &g.spec.primary_source_sheet_id == prim) {
                    return Some(gs.id.clone());
                }
                if let Some(rs) = doc.result_sheet(prim)
                    && let Some(gs) = doc.graph_sheets.iter().find(|g| g.spec.primary_source_sheet_id == rs.analysis.source_sheet_id)
                {
                    return Some(gs.id.clone());
                }
            }
            doc.graph_sheets.first().map(|g| g.id.clone()).or_else(|| doc.layout_sheets.first().map(|l| l.id.clone()))
        });

        (primary, secondary)
    }

    /// Materialize concrete [`Self::primary_sheet_id`] and [`Self::secondary_sheet_id`] against `doc`
    /// so split pane assignments never drift with `doc.active_sheet_id` (`gui-layout-spec.md` §2.4).
    pub fn materialize_sheets(&mut self, doc: &ProjectDocument) -> (Option<SheetId>, Option<SheetId>) {
        let (primary, secondary) = self.resolve_sheets(doc);
        self.primary_sheet_id = primary.clone();
        self.secondary_sheet_id = secondary.clone();
        (primary, secondary)
    }

    /// Return `true` when `sheet_id` is currently displayed in either materialized split pane.
    #[must_use]
    pub fn contains_sheet(&self, sheet_id: &SheetId) -> bool {
        self.primary_sheet_id.as_ref() == Some(sheet_id) || self.secondary_sheet_id.as_ref() == Some(sheet_id)
    }

    /// Resolve the explicit sheet ID targeted by Trailing Studio Inspector controls (`gui-layout-spec.md` §2.4).
    #[must_use]
    pub fn effective_inspector_target<'a>(&'a self, doc: &'a ProjectDocument) -> Option<&'a SheetId> {
        if self.enabled
            && self.inspector_pinned
            && let Some(target) = self.inspector_target_sheet_id.as_ref()
            && doc.sheet_section(target).is_some()
            && self.contains_sheet(target)
        {
            return Some(target);
        }
        doc.active_sheet_id.as_ref()
    }

    /// Return `true` when Trailing Studio Inspector targets a sheet hosted in the secondary split pane (`GAP-07-01`).
    ///
    /// When the target sheet is displayed only in the secondary pane this returns `true`; when
    /// displayed only in the primary pane this returns `false`; when both panes display the same
    /// sheet (or neither matches), `focused_secondary` breaks the tie.
    #[must_use]
    pub fn inspector_targets_secondary_pane(&self, doc: &ProjectDocument, focused_secondary: bool) -> bool {
        if !self.enabled {
            return false;
        }
        let (primary, secondary) = self.resolve_sheets(doc);
        if let Some(target) = self.effective_inspector_target(doc) {
            let in_primary = primary.as_ref() == Some(target);
            let in_secondary = secondary.as_ref() == Some(target);
            if in_secondary && !in_primary {
                return true;
            }
            if in_primary && !in_secondary {
                return false;
            }
        }
        focused_secondary
    }

    /// Enforce three-way split pin lifecycle invariants (`gui-layout-spec.md` §2.4):
    /// auto-unpin whenever Split closes, the pinned sheet is deleted, or it leaves the split pair.
    pub fn sync_pin_invariants(&mut self, doc: &ProjectDocument) {
        if !self.enabled {
            self.inspector_pinned = false;
            self.inspector_target_sheet_id = doc.active_sheet_id.clone();
            return;
        }
        if self.primary_sheet_id.as_ref().is_none_or(|id| doc.sheet_section(id).is_none())
            || self.secondary_sheet_id.as_ref().is_none_or(|id| doc.sheet_section(id).is_none())
        {
            let _ = self.materialize_sheets(doc);
        }
        if self.inspector_pinned {
            let valid = self.inspector_target_sheet_id.as_ref().is_some_and(|id| doc.sheet_section(id).is_some() && self.contains_sheet(id));
            if !valid {
                self.inspector_pinned = false;
                self.inspector_target_sheet_id = doc.active_sheet_id.clone();
            }
        } else {
            self.inspector_target_sheet_id = doc.active_sheet_id.clone();
        }
    }

    /// Apply §2.4 Navigator/Footer selection rules while Split Comparison is active.
    pub fn on_navigator_select(&mut self, doc: &ProjectDocument, prev_active: Option<&SheetId>, new_active: &SheetId, last_focused_secondary: bool) {
        if !self.enabled {
            self.sync_pin_invariants(doc);
            return;
        }
        if !self.contains_sheet(new_active) {
            let replace_secondary = if prev_active.is_some() && self.secondary_sheet_id.as_ref() == prev_active {
                true
            } else if prev_active.is_some() && self.primary_sheet_id.as_ref() == prev_active {
                false
            } else {
                last_focused_secondary
            };
            if replace_secondary {
                self.secondary_sheet_id = Some(new_active.clone());
            } else {
                self.primary_sheet_id = Some(new_active.clone());
            }
        }
        self.sync_pin_invariants(doc);
    }
}

pub use curvebook_doc::RemediationKind;

/// Non-prescriptive assumption diagnostic strip displayed above a [`ResultSheet`] (`QOL-7`).
///
/// Surfaces checked assumptions, numeric diagnostic metrics, statistical/sample-size limitations,
/// and detail-tab navigation links on the *same* [`ResultSheet`] without prescribing alternative
/// tests, data transforms, or outlier exclusion, and never creates sibling sheets automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssumptionDiagnosticBanner {
    pub source_result_id: SheetId,
    pub severity: ChecklistStatus,
    pub summary: String,
    pub diagnostic_metric: Option<String>,
    pub limitation_note: String,
    pub detail_tab_ids: Vec<String>,
}

impl AssumptionDiagnosticBanner {
    /// Derive a strictly diagnostics-only banner from `rs.checklist` (`Violated`, `Warn`, or
    /// inconclusive/small-sample `Info`) and collect same-sheet detail tab IDs from `rs.tabs`.
    #[must_use]
    pub fn from_result(rs: &ResultSheet) -> Option<Self> {
        let rank = |status: ChecklistStatus| -> u8 {
            match status {
                ChecklistStatus::Violated => 3,
                ChecklistStatus::Warn => 2,
                ChecklistStatus::Info => 1,
                ChecklistStatus::Pass => 0,
            }
        };
        let flagged = rs
            .checklist
            .iter()
            .filter(|c| matches!(c.status, ChecklistStatus::Violated | ChecklistStatus::Warn) || is_inconclusive_info_item(c))
            .max_by_key(|c| rank(c.status))?;

        let mut detail_tab_ids: Vec<String> = rs
            .tabs
            .iter()
            .filter(|tab| {
                matches!(
                    tab.kind,
                    ResultTabKind::Diagnostics
                        | ResultTabKind::NormalQq
                        | ResultTabKind::Residuals
                        | ResultTabKind::CovarianceMatrix
                        | ResultTabKind::LogMinusLog
                )
            })
            .map(|tab| tab.id.clone())
            .collect();
        if detail_tab_ids.is_empty() {
            detail_tab_ids = rs.tabs.iter().map(|tab| tab.id.clone()).collect();
        }

        let status_label = match flagged.status {
            ChecklistStatus::Violated => "assumption violated",
            ChecklistStatus::Warn => "assumption warning",
            ChecklistStatus::Info => "inconclusive / sample-size limitation",
            ChecklistStatus::Pass => "checked",
        };
        let checked_count = rs
            .checklist
            .iter()
            .filter(|c| matches!(c.status, ChecklistStatus::Violated | ChecklistStatus::Warn) || is_inconclusive_info_item(c))
            .count();
        let summary = if checked_count > 1 {
            format!("{} ({status_label}; {checked_count} checks flagged)", flagged.assumption)
        } else {
            format!("{} ({status_label})", flagged.assumption)
        };

        Some(Self {
            source_result_id: rs.id.clone(),
            severity: flagged.status,
            summary,
            diagnostic_metric: flagged.diagnostic_metric.clone(),
            limitation_note: sanitize_diagnostics_only_text(flagged),
            detail_tab_ids,
        })
    }

    /// Preferred same-sheet diagnostic detail tab ID (`Diagnostics`, `NormalQq`, `Residuals`, ...).
    #[must_use]
    pub fn preferred_diagnostic_tab_id(&self) -> Option<&str> {
        self.detail_tab_ids.first().map(String::as_str)
    }

    /// Single-line diagnostics-only banner text rendered above the result sheet grid.
    #[must_use]
    pub fn format_banner_line(&self) -> String {
        match &self.diagnostic_metric {
            Some(metric) => format!("! {} [{}]: {} — Inspect Diagnostics", self.summary, metric, self.limitation_note),
            None => format!("! {}: {} — Inspect Diagnostics", self.summary, self.limitation_note),
        }
    }
}

// ============================================================================
// Unified CanvasDialogState
// ============================================================================

/// Modal dialog state for styling selected graph points (`Format > Format Selected Points...`, `WF-22`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FormatSelectedPointsState {
    pub graph_sheet_id: Option<SheetId>,
    pub selected_points: Vec<(usize, usize)>,
    #[serde(default)]
    pub selected_subcols: Vec<Option<usize>>,
    pub color: Option<String>,
    pub fill_color: Option<String>,
    pub symbol: Option<String>,
    pub size_pt: Option<f64>,
    pub border_width_pt: Option<f64>,
    pub fill_mode: Option<String>,
    pub hidden: Option<bool>,
    pub show_row_labels: bool,
    pub clear_overrides: bool,
}

impl FormatSelectedPointsState {
    /// Open dialog state targeting `graph_sheet_id` and `points` `(series_index, point_index)`.
    pub fn open_for_graph(&mut self, graph_sheet_id: SheetId, points: Vec<(usize, usize)>) {
        *self = Self::default();
        self.graph_sheet_id = Some(graph_sheet_id);
        self.selected_points = points;
    }

    /// Open dialog state targeting `graph_sheet_id` and `(series_index, point_index, subcol)` tuples.
    pub fn open_for_graph_with_subcols(&mut self, graph_sheet_id: SheetId, targets: Vec<(usize, usize, Option<usize>)>) {
        *self = Self::default();
        self.graph_sheet_id = Some(graph_sheet_id);
        self.set_selected_targets(targets);
    }

    /// Replace selected targets with explicit `(series_index, point_index, subcol)` tuples.
    pub fn set_selected_targets(&mut self, targets: Vec<(usize, usize, Option<usize>)>) {
        self.selected_points = targets.iter().map(|&(s, p, _)| (s, p)).collect();
        self.selected_subcols = targets.into_iter().map(|(_, _, sc)| sc).collect();
    }

    /// Build `graph.formatSelectedPoints` command intent across all selected points.
    #[must_use]
    pub fn build_commit_intent(&self) -> Option<ToolCommandIntent> {
        let sheet_id = self.graph_sheet_id.as_ref()?;
        if self.selected_points.is_empty() {
            return None;
        }
        let points_json: Vec<Value> = self
            .selected_points
            .iter()
            .enumerate()
            .map(|(idx, &(series_index, point_index))| {
                let mut entry = json!({ "series_index": series_index, "point_index": point_index });
                if let Some(Some(sc)) = self.selected_subcols.get(idx)
                    && let Some(obj) = entry.as_object_mut()
                {
                    obj.insert(String::from("subcol"), json!(sc));
                }
                entry
            })
            .collect();
        let mut payload = serde_json::Map::new();
        payload.insert(String::from("sheet_id"), json!(sheet_id.as_str()));
        payload.insert(String::from("points"), Value::Array(points_json));
        if self.clear_overrides {
            payload.insert(String::from("mode"), json!("clear"));
            return Some(ToolCommandIntent::new("graph.formatSelectedPoints", Value::Object(payload)));
        }

        let mut has_patch = false;
        if let Some(ref c) = self.color {
            payload.insert(String::from("color"), json!(c));
            has_patch = true;
        }
        if let Some(ref fc) = self.fill_color {
            payload.insert(String::from("fill_color"), json!(fc));
            has_patch = true;
        }
        if let Some(ref sym) = self.symbol {
            payload.insert(String::from("symbol"), json!(sym));
            has_patch = true;
        }
        if let Some(sz) = self.size_pt {
            payload.insert(String::from("size_pt"), json!(sz));
            has_patch = true;
        }
        if let Some(bw) = self.border_width_pt {
            payload.insert(String::from("border_width_pt"), json!(bw));
            has_patch = true;
        }
        if let Some(ref fm) = self.fill_mode {
            payload.insert(String::from("fill_mode"), json!(fm));
            has_patch = true;
        }
        if let Some(h) = self.hidden {
            payload.insert(String::from("hidden"), json!(h));
            has_patch = true;
        }
        if !has_patch && !self.show_row_labels {
            return None;
        }
        if has_patch {
            payload.insert(String::from("mode"), json!("merge"));
            Some(ToolCommandIntent::new("graph.formatSelectedPoints", Value::Object(payload)))
        } else {
            None
        }
    }

    /// Build optional `graph.addAnnotation` intents displaying row labels when `show_row_labels` is true.
    #[must_use]
    pub fn build_row_label_intents(&self, doc: &ProjectDocument) -> Vec<ToolCommandIntent> {
        self.build_row_label_intents_for_scene(doc, None)
    }

    /// Build `graph.addAnnotation` row-label intents placed near each point's plotted coordinates
    /// (with deterministic collision-staggered fallback offsets when a point is off-scene).
    #[must_use]
    pub fn build_row_label_intents_for_scene(&self, doc: &ProjectDocument, scene: Option<&ChartScene>) -> Vec<ToolCommandIntent> {
        if !self.show_row_labels {
            return Vec::new();
        }
        let Some(ref graph_id) = self.graph_sheet_id else {
            return Vec::new();
        };
        let Some(gs) = doc.graph_sheet(graph_id) else {
            return Vec::new();
        };
        let ds = doc
            .data_sheet(&gs.spec.primary_source_sheet_id)
            .or_else(|| doc.result_sheet(&gs.spec.primary_source_sheet_id).and_then(|rs| rs.analyzable_data.as_ref()));
        let mut intents = Vec::new();
        let mut placed_positions: Vec<(f64, f64)> = Vec::new();
        for (idx, &(series_idx, point_idx)) in self.selected_points.iter().enumerate() {
            let subcol_filter = self.selected_subcols.get(idx).copied().flatten();
            let label =
                ds.and_then(|d| row_title_at(&d.archetype, point_idx)).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Row {}", point_idx + 1));
            let plotted = scene.and_then(|sc| {
                sc.plotted_points
                    .iter()
                    .find(|pt| {
                        pt.source_cell.row == point_idx
                            && pt.source_cell.col == series_idx
                            && subcol_filter.is_none_or(|s| pt.source_cell.subcol == s)
                    })
                    .or_else(|| sc.plotted_points.iter().find(|pt| pt.source_cell.row == point_idx))
            });
            let (mut x_pt, mut y_pt) = if let Some(pt) = plotted {
                (pt.center_pt.x + 8.0, (pt.center_pt.y - 18.0).max(4.0))
            } else {
                let col_slot = (idx % 3) as f64;
                let row_slot = (idx / 3) as f64;
                (72.0 + col_slot * 124.0, 48.0 + row_slot * 30.0)
            };
            while placed_positions.iter().any(|&(px, py)| (px - x_pt).abs() < 48.0 && (py - y_pt).abs() < 18.0) {
                y_pt += 20.0;
                if y_pt > 260.0 {
                    y_pt = 24.0;
                    x_pt += 88.0;
                }
            }
            placed_positions.push((x_pt, y_pt));
            intents.push(ToolCommandIntent::new(
                "graph.addAnnotation",
                json!({
                    "sheet_id": graph_id.as_str(),
                    "kind": "text",
                    "text": label,
                    "x_pt": x_pt,
                    "y_pt": y_pt,
                }),
            ));
        }
        intents
    }
}

fn row_title_at(arch: &TableArchetype, row: usize) -> Option<String> {
    match arch {
        TableArchetype::Xy(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Column(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Grouped(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Contingency(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Survival(t) => t.row_titles.get(row).cloned(),
        TableArchetype::PartsOfWhole(t) => t.slice_labels.get(row).cloned(),
        TableArchetype::MultipleVariables(t) => t.row_titles.get(row).cloned(),
        TableArchetype::Nested(t) => t.row_titles.get(row).cloned(),
    }
}
