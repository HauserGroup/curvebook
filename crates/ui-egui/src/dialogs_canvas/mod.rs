//! Graph/layout canvas interaction controller, scientific analysis/formatting dialog state
//! machines, Command Palette (`Cmd+K`) fuzzy matching, and deterministic software frame
//! compositing (`RasterImage` / PNG) for headless screenshots and visual regression tests.

mod analysis_catalog;
mod analyze_dialog;
mod command_palette;
mod edit_params;
mod format_axes;
mod format_graph;
mod graph_overlays;
mod graph_type;
mod layout_export;
mod print_dialog;
mod r13_dialogs;
mod results_hooks;
mod workspace_dialogs;

#[cfg(test)]
mod tests;

use curvebook_chart::{ChartScene, PreflightIssue};
use curvebook_doc::{CanvasResizeMode, FigureCanvasPreset, GraphKind, ProjectDocument, SheetId};
use curvebook_engine::{EngineError, Session};
use curvebook_tools::{CanvasToolState, GridCellCoord, PanZoomInertiaState, PhysicalUnit, ToolCommandIntent};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub use crate::dialogs_r9::*;
use crate::power_wizard::PowerWizardState;

pub use analysis_catalog::{ANALYSIS_CATALOG, AnalysisCatalogItem, AnalysisCategory, default_analysis_kind_for_id};
pub use analyze_dialog::{
    AnalyzeDialogState, AnalyzeDuplicateChoice, DuplicateChoice, VariableCandidate, VariableRoleFilter, variable_role_badge, variable_role_badge_fg,
    variable_role_color,
};
pub use command_palette::{CommandOrigin, CommandPaletteEntry, CommandPaletteState};
pub use edit_params::{EditAnalysisParametersState, TwoGroupSubmethodMemory};
pub(crate) use edit_params::{clear_fit_parameter_inputs, sync_fit_parameter_inputs};
pub use format_axes::{AxisTarget, FormatAxesState};
pub use format_graph::FormatGraphState;
pub use graph_overlays::{ApplyStyleFromState, BubblePlotInspectorState, PairwiseComparisonsState};
pub use graph_type::{ChangeGraphTypeState, GRAPH_TYPE_OPTIONS, GraphTypeOption, ThumbnailGlyphKind};
pub use layout_export::{CsvExportOptionsState, CsvExportTargetKind, ExportFigureFormat, ExportFigureState, PageLayoutComposerState};
pub use print_dialog::PrintDialogState;
pub use r13_dialogs::{CreateGraphDialogState, EqualizeScalingDialogState, FormatEmbeddedTableDialogState, FormatImageDialogState};
pub(crate) use results_hooks::data_column_title;
pub use results_hooks::{
    AvailableHookOption, ChangeDataAnalyzedState, ChecklistSeverityFilter, ConstantHookPickerState, HookTargetKind, ResultSheetViewState,
    RetargetCandidate, copy_result_table_to_clipboard,
};
pub use workspace_dialogs::{
    AssumptionDiagnosticBanner, ConvertArchetypeDialogState, ExcludeWithReasonDialogState, FormatSelectedPointsState, RemediationKind,
    SplitWorkspaceState,
};

// ============================================================================
// Dialog & Inspector Discriminators
// ============================================================================

/// Active modal dialog open in the front-end shell.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ActiveDialog {
    #[default]
    None,
    FormatAxes {
        sheet_id: SheetId,
    },
    AnalyzeSheet {
        source_sheet_id: SheetId,
    },
    DuplicateAnalysisPrompt {
        source_sheet_id: SheetId,
        existing_result_ids: Vec<SheetId>,
        pending_entry_id: String,
    },
    EditAnalysisParameters {
        result_sheet_id: SheetId,
    },
    ChangeDataAnalyzed {
        result_sheet_id: SheetId,
    },
    ResultViewer {
        result_sheet_id: SheetId,
    },
    ConstantHookPicker {
        target_sheet_id: SheetId,
    },
    ChangeGraphType {
        graph_sheet_id: SheetId,
    },
    FormatGraph {
        graph_sheet_id: SheetId,
    },
    PairwiseComparisons {
        graph_sheet_id: SheetId,
    },
    BubblePlotInspector {
        graph_sheet_id: SheetId,
    },
    ApplyStyleFrom {
        target_graph_id: SheetId,
    },
    PageLayoutComposer {
        layout_sheet_id: SheetId,
    },
    ExportFigure {
        sheet_id: SheetId,
    },
    ConvertArchetype {
        sheet_id: SheetId,
    },
    ImportReshape,
    ExcludeWithReason {
        sheet_id: SheetId,
    },
    FormatSelectedPoints {
        graph_sheet_id: SheetId,
    },
    PowerAndSampleSize,
    Print {
        sheet_id: Option<SheetId>,
    },
    RandomizeGroups,
    RandomSampleRows {
        source_sheet_id: Option<SheetId>,
    },
    SimulateDistributions,
    InsertModelEquation {
        graph_sheet_id: SheetId,
    },
    CreateGraph {
        source_sheet_id: Option<SheetId>,
    },
    EqualizeScaling {
        layout_sheet_id: SheetId,
    },
    FormatEmbeddedTable {
        sheet_id: SheetId,
        target_index: usize,
        is_layout: bool,
    },
    FormatImage {
        sheet_id: SheetId,
        target_index: usize,
        is_layout: bool,
    },
    CommandPalette,
    Checkpoints,
    About,
    Feedback,
    ImportWarnings,
    ImportProjectMode {
        command_id: String,
        path: String,
        has_unsaved_changes: bool,
    },
    CsvExportOptions(CsvExportTargetKind),
    /// Export All Data Tables found files with the planned names already in the chosen folder
    /// and is waiting for the user to replace them, keep both, or cancel (`R10.29`).
    ExportAllTablesExistingFiles {
        /// Folder the user picked.
        dir: String,
        /// File names in `dir` that the export would replace.
        existing_files: Vec<String>,
    },
}

/// Editable draft state for the `Checkpoints` modal (`R15-GUI-31`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CheckpointsDialogState {
    pub name_draft: String,
    pub note_draft: String,
}

/// Active non-modal inspector docked on the right side of the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ActiveInspector {
    #[default]
    None,
    FormatGraph,
    FormatAxes,
    PairwiseComparisons,
    BubblePlot,
    PageLayout,
    ResultDetails,
}

impl ActiveInspector {
    /// Human-readable header label for the right inspector panel.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "Inspector",
            Self::FormatGraph => "Format Graph",
            Self::FormatAxes => "Format Axes",
            Self::PairwiseComparisons => "Pairwise Comparisons",
            Self::BubblePlot => "Bubble Plot Channels",
            Self::PageLayout => "Page Layout",
            Self::ResultDetails => "Analysis & Checklist",
        }
    }
}

const fn default_grid_overlay_spacing_pt() -> f64 {
    18.0
}

/// Interactive state for the graph/layout canvas, non-modal inspectors, and parameter dialogs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanvasDialogState {
    pub canvas: CanvasToolState,
    pub active_dialog: ActiveDialog,
    pub status_banner: Option<String>,
    pub active_inspector: ActiveInspector,
    pub analyze: AnalyzeDialogState,
    pub edit_params: EditAnalysisParametersState,
    pub change_data: ChangeDataAnalyzedState,
    pub result_view: ResultSheetViewState,
    pub hook_picker: ConstantHookPickerState,
    pub change_graph_type: ChangeGraphTypeState,
    pub format_graph: FormatGraphState,
    pub format_axes: FormatAxesState,
    pub pairwise: PairwiseComparisonsState,
    pub bubble_plot: BubblePlotInspectorState,
    pub apply_style: ApplyStyleFromState,
    pub page_layout: PageLayoutComposerState,
    pub export_figure: ExportFigureState,
    pub command_palette: CommandPaletteState,
    #[serde(default)]
    pub convert_archetype: ConvertArchetypeDialogState,
    #[serde(default)]
    pub exclude_with_reason: ExcludeWithReasonDialogState,
    #[serde(default)]
    pub active_canvas_preset: Option<FigureCanvasPreset>,
    #[serde(default)]
    pub canvas_resize_mode: CanvasResizeMode,
    #[serde(default)]
    pub preflight_issues: Vec<PreflightIssue>,
    #[serde(default)]
    pub rulers_visible: bool,
    #[serde(default)]
    pub ruler_unit: PhysicalUnit,
    #[serde(default)]
    pub grid_overlay_visible: bool,
    #[serde(default = "default_grid_overlay_spacing_pt")]
    pub grid_overlay_spacing_pt: f64,
    #[serde(default)]
    pub journal_preset: Option<curvebook_tools::JournalWidthPreset>,
    #[serde(default)]
    pub pan_zoom: PanZoomInertiaState,
    #[serde(default)]
    pub print: PrintDialogState,
    #[serde(default)]
    pub power_wizard: PowerWizardState,
    #[serde(default)]
    pub format_points: FormatSelectedPointsState,
    #[serde(default)]
    pub randomize_groups: RandomizeGroupsDialogState,
    #[serde(default)]
    pub random_sample_rows: RandomSampleRowsDialogState,
    #[serde(default)]
    pub simulate_distributions: SimulateDistributionsDialogState,
    #[serde(default)]
    pub insert_model_equation: InsertModelEquationDialogState,
    #[serde(default)]
    pub feedback: crate::feedback::FeedbackDraft,
    #[serde(default)]
    pub create_graph: CreateGraphDialogState,
    #[serde(default)]
    pub equalize_scaling: EqualizeScalingDialogState,
    #[serde(default)]
    pub format_embedded_table: FormatEmbeddedTableDialogState,
    #[serde(default)]
    pub format_image: FormatImageDialogState,
    #[serde(default)]
    pub checkpoints: CheckpointsDialogState,
    #[serde(default)]
    pub csv_export: CsvExportOptionsState,
    #[serde(skip)]
    pub(crate) suppress_auto_open_fit_params: bool,
}

impl Default for CanvasDialogState {
    fn default() -> Self {
        Self {
            canvas: CanvasToolState::default(),
            active_dialog: ActiveDialog::default(),
            status_banner: None,
            active_inspector: ActiveInspector::default(),
            analyze: AnalyzeDialogState::default(),
            edit_params: EditAnalysisParametersState::default(),
            change_data: ChangeDataAnalyzedState::default(),
            result_view: ResultSheetViewState::default(),
            hook_picker: ConstantHookPickerState::default(),
            change_graph_type: ChangeGraphTypeState::default(),
            format_graph: FormatGraphState::default(),
            format_axes: FormatAxesState::default(),
            pairwise: PairwiseComparisonsState::default(),
            bubble_plot: BubblePlotInspectorState::default(),
            apply_style: ApplyStyleFromState::default(),
            page_layout: PageLayoutComposerState::default(),
            export_figure: ExportFigureState::default(),
            command_palette: CommandPaletteState::default(),
            convert_archetype: ConvertArchetypeDialogState::default(),
            exclude_with_reason: ExcludeWithReasonDialogState::default(),
            active_canvas_preset: None,
            canvas_resize_mode: CanvasResizeMode::default(),
            preflight_issues: Vec::new(),
            rulers_visible: false,
            ruler_unit: PhysicalUnit::default(),
            grid_overlay_visible: false,
            grid_overlay_spacing_pt: default_grid_overlay_spacing_pt(),
            journal_preset: None,
            pan_zoom: PanZoomInertiaState::default(),
            print: PrintDialogState::default(),
            power_wizard: PowerWizardState::default(),
            format_points: FormatSelectedPointsState::default(),
            randomize_groups: RandomizeGroupsDialogState::default(),
            random_sample_rows: RandomSampleRowsDialogState::default(),
            simulate_distributions: SimulateDistributionsDialogState::default(),
            insert_model_equation: InsertModelEquationDialogState::default(),
            feedback: crate::feedback::FeedbackDraft::default(),
            create_graph: CreateGraphDialogState::default(),
            equalize_scaling: EqualizeScalingDialogState::default(),
            format_embedded_table: FormatEmbeddedTableDialogState::default(),
            format_image: FormatImageDialogState::default(),
            checkpoints: CheckpointsDialogState::default(),
            csv_export: CsvExportOptionsState::default(),
            suppress_auto_open_fit_params: false,
        }
    }
}

impl CanvasDialogState {
    /// Compile the currently active graph or layout sheet into a [`ChartScene`], if applicable.
    pub fn active_scene(&self, session: &Session) -> Result<Option<ChartScene>, EngineError> {
        let Some(active_id) = &session.document().active_sheet_id else {
            return Ok(None);
        };
        if session.document().graph_sheet(active_id).is_none() && session.document().layout_sheet(active_id).is_none() {
            return Ok(None);
        }
        Ok(Some(session.render_sheet_scene(active_id)?))
    }

    /// Reset canvas zoom to fit the sheet cleanly (`ui.zoom.fitSheet`).
    pub fn reset_zoom_to_fit(&mut self) {
        self.canvas.set_zoom(1.0);
        self.canvas.pan_pt.x = 0.0;
        self.canvas.pan_pt.y = 0.0;
        self.pan_zoom = PanZoomInertiaState::default();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn effective_zoom_and_pan(&self) -> (f64, [f64; 2]) {
        let zoom = if (self.pan_zoom.zoom - 1.0).abs() > 1e-9 { self.pan_zoom.zoom } else { self.canvas.zoom };
        let pan_x = if self.pan_zoom.pan_pt[0].abs() > 1e-9 { self.pan_zoom.pan_pt[0] } else { self.canvas.pan_pt.x };
        let pan_y = if self.pan_zoom.pan_pt[1].abs() > 1e-9 { self.pan_zoom.pan_pt[1] } else { self.canvas.pan_pt.y };
        (zoom.max(0.05), [pan_x, pan_y])
    }

    /// Open `Convert Table Archetype...` modal dialog (`QOL-1`).
    pub fn open_convert_archetype(&mut self, doc: &ProjectDocument, sheet_id: &SheetId) -> bool {
        let Some(ds) = doc.data_sheet(sheet_id) else {
            return false;
        };
        self.convert_archetype.open_for_sheet(ds);
        self.active_dialog = ActiveDialog::ConvertArchetype { sheet_id: sheet_id.clone() };
        true
    }

    /// Open `Import Reshape (Plate Map / Tidy Long)` modal dialog (`QOL-2`).
    pub fn open_import_reshape(&mut self) {
        self.active_dialog = ActiveDialog::ImportReshape;
    }

    /// Open `Exclude Selection with Reason...` (`Cmd+Shift+E`) modal dialog (`QOL-3`).
    pub fn open_exclude_with_reason(&mut self, sheet_id: SheetId, coords: Vec<GridCellCoord>) {
        self.exclude_with_reason.open_for_coords(sheet_id.clone(), coords);
        self.active_dialog = ActiveDialog::ExcludeWithReason { sheet_id };
    }

    /// Build `graph.applyCanvasPreset` or `layout.applyCanvasPreset` intent (`QOL-8`).
    #[must_use]
    pub fn apply_canvas_preset_intent(&mut self, sheet_id: &SheetId, is_layout: bool, preset: FigureCanvasPreset) -> ToolCommandIntent {
        self.active_canvas_preset = Some(preset);
        let cmd = if is_layout { "layout.applyCanvasPreset" } else { "graph.applyCanvasPreset" };
        ToolCommandIntent::new(
            cmd,
            json!({
                "sheet_id": sheet_id.as_str(),
                "preset": format!("{preset:?}"),
                "resize_mode": format!("{:?}", self.canvas_resize_mode),
            }),
        )
    }

    /// Refresh [`Self::preflight_issues`] directly from a compiled [`ChartScene`] (`QOL-8`).
    pub fn refresh_preflight(&mut self, scene: &ChartScene) {
        self.preflight_issues = curvebook_chart::preflight_check_scene(scene);
    }

    /// Synchronize inspector/dialog state when navigating to `sheet_id`.
    ///
    /// First-time visits to an uninitialized [`curvebook_doc::GraphSheet`] automatically present
    /// [`ActiveDialog::ChangeGraphType`].
    pub fn on_sheet_selected(&mut self, doc: &ProjectDocument, sheet_id: &SheetId) {
        self.on_sheet_selected_with_auto_modal(doc, sheet_id, true);
    }

    /// Synchronize inspector state when selecting `sheet_id`, optionally auto-opening
    /// [`ActiveDialog::ChangeGraphType`] on first-time visits to uninitialized graphs.
    pub fn on_sheet_selected_with_auto_modal(&mut self, doc: &ProjectDocument, sheet_id: &SheetId, open_uninitialized_graph_chooser: bool) {
        if let Some(gs) = doc.graph_sheet(sheet_id) {
            self.format_graph.sync_from_graph(gs);
            self.format_axes.sync_from_graph(gs, AxisTarget::Y);
            self.pairwise.sync_from_graph(gs, doc);
            self.bubble_plot.sync_from_graph(gs, doc);
            if open_uninitialized_graph_chooser && !gs.initialized {
                self.change_graph_type.open_for_graph(gs, doc);
                self.active_dialog = ActiveDialog::ChangeGraphType { graph_sheet_id: gs.id.clone() };
            }
            self.active_inspector =
                if matches!(gs.spec.kind, GraphKind::MultipleVariables) { ActiveInspector::BubblePlot } else { ActiveInspector::FormatGraph };
        } else if let Some(ls) = doc.layout_sheet(sheet_id) {
            self.page_layout.sync_from_layout(ls);
            self.active_inspector = ActiveInspector::PageLayout;
        } else if let Some(rs) = doc.result_sheet(sheet_id) {
            self.result_view.sync_with_result(rs);
            self.active_inspector = ActiveInspector::ResultDetails;
        } else if let Some(ds) = doc.data_sheet(sheet_id) {
            self.analyze.open_for_data_sheet(ds);
            self.active_inspector = ActiveInspector::None;
        }
    }

    /// Open standalone 4-stage Power & Sample Size Wizard (`WF-15`, `R8.7`).
    pub fn open_power_wizard(&mut self) {
        let _ = self.power_wizard.recalculate();
        self.active_dialog = ActiveDialog::PowerAndSampleSize;
    }

    /// Open `Format Selected Points...` dialog targeting `graph_sheet_id` and `points` (`WF-22`).
    pub fn open_format_selected_points(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId, points: Vec<(usize, usize)>) -> bool {
        if doc.graph_sheet(graph_sheet_id).is_none() {
            return false;
        }
        self.format_points.open_for_graph(graph_sheet_id.clone(), points);
        self.active_dialog = ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_sheet_id.clone() };
        true
    }

    /// Open `Format Selected Points...` dialog preserving optional replicate `subcol` per point (`R2-S08-03`).
    pub fn open_format_selected_points_with_subcols(
        &mut self,
        doc: &ProjectDocument,
        graph_sheet_id: &SheetId,
        targets: Vec<(usize, usize, Option<usize>)>,
    ) -> bool {
        if doc.graph_sheet(graph_sheet_id).is_none() {
            return false;
        }
        self.format_points.open_for_graph_with_subcols(graph_sheet_id.clone(), targets);
        self.active_dialog = ActiveDialog::FormatSelectedPoints { graph_sheet_id: graph_sheet_id.clone() };
        true
    }

    /// Open `Randomize Subjects to Groups...` dialog (`R9.14`, `table.randomizeGroups`).
    pub fn open_randomize_groups(&mut self, doc: &ProjectDocument) {
        self.randomize_groups.open_for_document(doc);
        self.active_dialog = ActiveDialog::RandomizeGroups;
    }

    /// Open `Random Sample Rows...` dialog (`R9.14`, `table.randomSampleRows`).
    pub fn open_random_sample_rows(&mut self, doc: &ProjectDocument, source_sheet_id: Option<&SheetId>) -> bool {
        let target_id = source_sheet_id.cloned().or_else(|| doc.data_sheets.first().map(|d| d.id.clone()));
        let Some(id) = target_id else {
            return false;
        };
        if !self.random_sample_rows.open_for_sheet(doc, &id) {
            return false;
        }
        self.active_dialog = ActiveDialog::RandomSampleRows { source_sheet_id: Some(id) };
        true
    }

    /// Open `Simulate Distributions...` dialog (`R9.14`, `table.simulateDistributions`).
    pub fn open_simulate_distributions(&mut self) {
        self.active_dialog = ActiveDialog::SimulateDistributions;
    }

    /// Open `Insert Model Equation...` dialog targeting `graph_sheet_id` (`R9.16`, strictly equations only).
    pub fn open_insert_model_equation(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.insert_model_equation.open_for_graph(gs, doc);
        self.active_dialog = ActiveDialog::InsertModelEquation { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Bubble Plot Channels` dialog & inspector targeting `graph_sheet_id` (`WF-44`).
    pub fn open_bubble_plot(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.bubble_plot.sync_from_graph(gs, doc);
        self.active_inspector = ActiveInspector::BubblePlot;
        self.active_dialog = ActiveDialog::BubblePlotInspector { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Format Graph` dialog & inspector targeting `graph_sheet_id` (`WF-45`).
    pub fn open_format_graph(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.format_graph.sync_from_graph_with_doc(gs, Some(doc));
        self.active_inspector = ActiveInspector::FormatGraph;
        self.active_dialog = ActiveDialog::FormatGraph { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Pairwise Comparisons` dialog & inspector targeting `graph_sheet_id` (`WF-46`).
    pub fn open_pairwise_comparisons(&mut self, doc: &ProjectDocument, graph_sheet_id: &SheetId) -> bool {
        let Some(gs) = doc.graph_sheet(graph_sheet_id) else {
            return false;
        };
        self.pairwise.sync_from_graph(gs, doc);
        self.active_inspector = ActiveInspector::PairwiseComparisons;
        self.active_dialog = ActiveDialog::PairwiseComparisons { graph_sheet_id: gs.id.clone() };
        true
    }

    /// Open `Page Layout Composer` dialog & inspector targeting `layout_sheet_id` (`WF-48`).
    pub fn open_page_layout_composer(&mut self, doc: &ProjectDocument, layout_sheet_id: &SheetId) -> bool {
        let Some(ls) = doc.layout_sheet(layout_sheet_id) else {
            return false;
        };
        self.page_layout.sync_from_layout(ls);
        self.active_inspector = ActiveInspector::PageLayout;
        self.active_dialog = ActiveDialog::PageLayoutComposer { layout_sheet_id: ls.id.clone() };
        true
    }

    /// Open `Export Figure...` dialog targeting `sheet_id` (`WF-48`).
    pub fn open_export_figure(&mut self, sheet_id: &SheetId) {
        self.active_dialog = ActiveDialog::ExportFigure { sheet_id: sheet_id.clone() };
    }

    /// Open `Analyze...` for `source_sheet_id` (`DataSheet`, analyzable `ResultSheet`, or tabular `ResultSheet`).
    pub fn open_analyze_dialog(&mut self, doc: &ProjectDocument, source_sheet_id: &SheetId) -> bool {
        if let Some(ds) = doc.data_sheet(source_sheet_id) {
            self.analyze.open_for_data_sheet(ds);
            self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: ds.id.clone() };
            return true;
        }
        if let Some(rs) = doc.result_sheet(source_sheet_id) {
            if let Some(analyzable) = &rs.analyzable_data {
                self.analyze.open_for_data_sheet(analyzable);
                self.analyze.source_sheet_id = Some(rs.id.clone());
                self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: rs.id.clone() };
                return true;
            }
            let active_tab = self
                .result_view
                .active_tab(rs)
                .filter(|t| !t.column_headers.is_empty() && !t.rows.is_empty())
                .or_else(|| rs.tabs.iter().find(|t| !t.column_headers.is_empty() && !t.rows.is_empty()));
            if let Some(tab) = active_tab {
                self.analyze.open_for_result_tab(rs, tab);
                self.active_dialog = ActiveDialog::AnalyzeSheet { source_sheet_id: rs.id.clone() };
                return true;
            }
        }
        false
    }

    /// Open parameter re-entry (`Cmd+T`) for `result_sheet_id`.
    pub fn open_edit_parameters(&mut self, doc: &ProjectDocument, result_sheet_id: &SheetId) -> bool {
        let Some(rs) = doc.result_sheet(result_sheet_id) else {
            return false;
        };
        self.edit_params.open_for_result(rs);
        if let curvebook_doc::AnalysisKind::PowerAnalysis(ref pa) = rs.analysis.kind {
            self.power_wizard.sync_from_doc_spec(pa);
        }
        self.active_dialog = ActiveDialog::EditAnalysisParameters { result_sheet_id: rs.id.clone() };
        true
    }

    /// Open `Change Data Analyzed` cross-table retargeting for `result_sheet_id`.
    pub fn open_change_data_analyzed(&mut self, doc: &ProjectDocument, result_sheet_id: &SheetId) -> bool {
        let Some(rs) = doc.result_sheet(result_sheet_id) else {
            return false;
        };
        self.change_data.open_for_result(rs, doc);
        self.active_dialog = ActiveDialog::ChangeDataAnalyzed { result_sheet_id: rs.id.clone() };
        true
    }

    /// Open the constant hook picker targeting `target_sheet_id`.
    pub fn open_hook_picker(&mut self, doc: &ProjectDocument, target_sheet_id: &SheetId, param_name: Option<String>) {
        self.hook_picker.open_for_document(target_sheet_id.clone(), param_name, doc);
        self.active_dialog = ActiveDialog::ConstantHookPicker { target_sheet_id: target_sheet_id.clone() };
    }

    /// Open `Apply Style From...` for `target_graph_id`.
    pub fn open_apply_style_from(&mut self, doc: &ProjectDocument, target_graph_id: &SheetId) -> bool {
        if doc.graph_sheet(target_graph_id).is_none() {
            return false;
        }
        self.apply_style.open_for_graph(target_graph_id, doc);
        self.active_dialog = ActiveDialog::ApplyStyleFrom { target_graph_id: target_graph_id.clone() };
        true
    }

    /// Open `Apply Style From...` targeting multiple selected graphs (`WF-56` / `R13.3`).
    pub fn open_apply_style_from_targets(&mut self, doc: &ProjectDocument, targets: &[SheetId]) -> bool {
        let valid: Vec<SheetId> = targets.iter().filter(|id| doc.graph_sheet(id).is_some()).cloned().collect();
        let Some(first) = valid.first().cloned() else {
            return false;
        };
        self.apply_style.open_for_graphs(&valid, doc);
        self.active_dialog = ActiveDialog::ApplyStyleFrom { target_graph_id: first };
        true
    }

    /// Open `Create New Graph...` dialog (`WF-56` / `R13.3`).
    pub fn open_create_graph(&mut self, doc: &ProjectDocument, preferred_source: Option<&SheetId>) -> bool {
        self.create_graph.open_for_document(doc, preferred_source);
        let Some(src) = self.create_graph.source_sheet_id.clone() else {
            return false;
        };
        self.active_dialog = ActiveDialog::CreateGraph { source_sheet_id: Some(src) };
        true
    }

    /// Open `Equalize Scaling...` dialog targeting `layout_id` (`WF-63` / `R13.5`).
    pub fn open_equalize_scaling(&mut self, doc: &ProjectDocument, layout_id: &SheetId) -> bool {
        let Some(ls) = doc.layout_sheet(layout_id) else {
            return false;
        };
        self.equalize_scaling.open_for_layout(doc, ls);
        self.active_dialog = ActiveDialog::EqualizeScaling { layout_sheet_id: ls.id.clone() };
        true
    }

    /// Open `Format Embedded Table...` targeting annotation/panel `target_index` on `sheet_id` (`WF-58` / `R13.4`).
    pub fn open_format_embedded_table(&mut self, doc: &ProjectDocument, sheet_id: &SheetId, target_index: usize) -> bool {
        if let Some(gs) = doc.graph_sheet(sheet_id)
            && self.format_embedded_table.open_for_graph(doc, gs, target_index)
        {
            self.active_dialog = ActiveDialog::FormatEmbeddedTable { sheet_id: gs.id.clone(), target_index, is_layout: false };
            return true;
        }
        if let Some(ls) = doc.layout_sheet(sheet_id)
            && self.format_embedded_table.open_for_layout(doc, ls, target_index)
        {
            self.active_dialog = ActiveDialog::FormatEmbeddedTable { sheet_id: ls.id.clone(), target_index, is_layout: true };
            return true;
        }
        false
    }

    /// Open `Format Image...` targeting embedded image `target_index` on `sheet_id` (`WF-57` / `R13.7`).
    pub fn open_format_image(&mut self, doc: &ProjectDocument, sheet_id: &SheetId, target_index: usize) -> bool {
        if let Some(gs) = doc.graph_sheet(sheet_id)
            && self.format_image.open_for_graph(doc, gs, target_index)
        {
            self.active_dialog = ActiveDialog::FormatImage { sheet_id: gs.id.clone(), target_index, is_layout: false };
            return true;
        }
        if let Some(ls) = doc.layout_sheet(sheet_id)
            && self.format_image.open_for_layout(doc, ls, target_index)
        {
            self.active_dialog = ActiveDialog::FormatImage { sheet_id: ls.id.clone(), target_index, is_layout: true };
            return true;
        }
        false
    }

    /// Open `Cmd+K` Command Palette.
    pub fn open_command_palette(&mut self) {
        self.command_palette.open();
        self.active_dialog = ActiveDialog::CommandPalette;
    }

    /// Open `Checkpoints` manager modal (`R15-GUI-31`).
    pub fn open_checkpoints(&mut self, doc: &ProjectDocument) {
        self.checkpoints.name_draft = format!("Checkpoint {}", doc.checkpoints.len() + 1);
        self.checkpoints.note_draft.clear();
        self.active_dialog = ActiveDialog::Checkpoints;
    }

    /// Open `Print...` dialog for `sheet_id` (or first printable sheet in `doc`).
    pub fn open_print_dialog(&mut self, doc: &ProjectDocument, sheet_id: Option<&SheetId>, discovered_printers: Vec<String>) {
        self.print.open_for_document(doc, sheet_id, discovered_printers);
        let target = self.print.job.sheet_ids.first().cloned().or_else(|| sheet_id.cloned());
        self.active_dialog = ActiveDialog::Print { sheet_id: target };
    }

    /// Confirm `Analyze...`, opening [`ActiveDialog::DuplicateAnalysisPrompt`] if a matching
    /// analysis already exists on the source sheet.
    pub fn submit_analyze_or_prompt(&mut self, doc: &ProjectDocument) -> Option<ToolCommandIntent> {
        match self.analyze.try_build_run_intent(doc) {
            Ok(intent) => {
                self.active_dialog = ActiveDialog::None;
                Some(intent)
            }
            Err(existing_result_ids) if !existing_result_ids.is_empty() => {
                if let (Some(src), Some(entry_id)) = (&self.analyze.source_sheet_id, &self.analyze.selected_entry_id) {
                    self.active_dialog = ActiveDialog::DuplicateAnalysisPrompt {
                        source_sheet_id: src.clone(),
                        existing_result_ids,
                        pending_entry_id: entry_id.clone(),
                    };
                }
                None
            }
            Err(_) => None,
        }
    }

    /// Dismiss any open modal dialog.
    pub fn close_dialog(&mut self) {
        self.active_dialog = ActiveDialog::None;
    }
}
