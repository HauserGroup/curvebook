//! Single-commit `egui_shell` intent definitions, multi-pass merge queue, and settlement committers.

mod executors;
mod stat_params;

use curvebook_doc::{ConfidenceLevel, LevelOrderSpec, PValueStackMethod, SheetId, SheetSection, TableKind};
use curvebook_engine::{CommandOutcome, EngineError, Session};
use curvebook_palette::CanvasPreviewFilter;

use super::layout::WorkspacePreset;
use super::{app_modals, transform_modals};
use crate::UiApp;
use crate::dialogs_canvas::{
    ActiveInspector, ApplyStyleFromState, AxisTarget, BubblePlotInspectorState, ChangeGraphTypeState, CreateGraphDialogState, DuplicateChoice,
    EqualizeScalingDialogState, ExportFigureState, FormatAxesState, FormatEmbeddedTableDialogState, FormatGraphState, FormatImageDialogState,
    FormatSelectedPointsState, PageLayoutComposerState, PairwiseComparisonsState, PrintDialogState, VariableRoleFilter,
};
use crate::dialogs_r9::{InsertModelEquationDialogState, RandomSampleRowsDialogState, RandomizeGroupsDialogState, SimulateDistributionsDialogState};
use crate::grid_view::{ImportOriginKind, ImportPasteWizardState, ImportReshapeState, MoveDirection};
use crate::power_wizard::PowerWizardState;
use crate::theme::UiThemeMode;
use crate::workspace_nav::{ActiveWorkspacePane, GallerySortOrder, NavigatorNestingMode};

/// Target CI method cycled inside the Contingency `EditAnalysisParameters` modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContingencyCiTarget {
    OddsRatio,
    RelativeRisk,
    AttributableRisk,
    Diagnostic,
}

/// Diagnostic preview checkbox toggled inside `EditAnalysisParameters`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagToggleFlag {
    QqPlot,
    Residuals,
    Homoscedasticity,
    Checklist,
    Influence,
}

/// Typed `Analyze...` picker and duplicate-analysis disambiguation actions.
#[derive(Debug, Clone, PartialEq)]
pub enum AnalyzePickerIntent {
    ClearSearch,
    ToggleCategory { slug: String },
    SelectCatalogEntry { entry_id: String },
    TogglePcaPredictor { col: usize },
    SetPcaPredictors { cols: Vec<usize> },
    SetPcrOutcome { col: usize },
    SetMvRoleFilter { filter: VariableRoleFilter },
    ToggleMvVar { col: usize },
    SetOutlierMethod { method: String },
    StepOutlierParam { delta: f64 },
    ToggleOutlierIterative,
    SetPValueStackMethod { method: PValueStackMethod },
    SetPValueStackThreshold { threshold: f64 },
    SetPValueStackColumn { column: Option<usize> },
    SetDuplicateChoice { choice: DuplicateChoice },
    SetDuplicateTarget { target_id: String },
}

/// Typed `EditAnalysisParameters` statistical modal actions (with `R1`–`R9` predicate gating).
#[derive(Debug, Clone, PartialEq)]
pub enum StatParamsIntent {
    SetCustomTitle { title: String },
    SetOneWayMethod { method: String },
    SelectAllDatasets,
    ToggleDataset { idx: usize },
    ToggleSchematic,
    ToggleGeisserGreenhouse,
    SetResidualCovariance { cov: String },
    SetMultiplicityMethod { slug: String },
    StepMultiplicityThreshold { delta: f64 },
    SetCompareAgainstControl { enabled: bool },
    CycleControlColumn,
    SetCompareSelectedPairs { enabled: bool },
    ToggleSelectedPair { a: usize, b: usize },
    ToggleMultiplicityReportCi,
    ToggleEffectSizes,
    SetPValueFormat { slug: String },
    StepPValueDigits { delta: i32 },
    StepTwoGroupHypothetical { delta: f64 },
    CycleContingencyCi { which: ContingencyCiTarget },
    ToggleCorrAllPairs,
    ToggleCorrHeatmap,
    ToggleDiag { flag: DiagToggleFlag },
}

/// User intent captured during an `egui` pass and deferred until `!ctx.will_discard()` settles.
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntent {
    EngineCommand {
        command_id: String,
        params: serde_json::Value,
    },
    EngineCommandGroup {
        commands: Vec<(String, serde_json::Value)>,
    },
    UiCommand {
        command_id: String,
        params: serde_json::Value,
    },
    SetTheme(UiThemeMode),
    SetLocale(String),
    SetUiScale(f32),
    SelectGridCell {
        sheet_id: Option<SheetId>,
        coord: curvebook_tools::GridCellCoord,
    },
    SelectXCell {
        row: usize,
        subcol: usize,
    },
    ExtendGridSelection {
        sheet_id: Option<SheetId>,
        coord: curvebook_tools::GridCellCoord,
    },
    ToggleGridCell {
        sheet_id: Option<SheetId>,
        coord: curvebook_tools::GridCellCoord,
    },
    GridNavigate {
        dir: MoveDirection,
        extend: bool,
        jump: bool,
    },
    SetRowTitlesCollapsed {
        collapsed: bool,
    },
    DialogWidgetClick {
        widget_id: String,
    },
    DialogButtonAction {
        action: String,
    },
    SetTextFieldBuffer {
        widget_id: String,
        text: String,
    },
    OpenProjectFile,
    CopyResultTable {
        sheet_id: SheetId,
        force_full: bool,
    },
    SetFloatingNotePosition {
        sheet_id: SheetId,
        note_id: String,
        pos_pt: [f64; 2],
    },
    ToggleFloatingNoteCollapsed {
        sheet_id: SheetId,
        note_id: String,
    },
    ClearHookPickerSearch,
    AnalyzePicker(AnalyzePickerIntent),
    StatParams(StatParamsIntent),
    SelectResultTab {
        index: usize,
        sheet_id: Option<SheetId>,
    },
    SetResultReportOptions {
        format: String,
        scope: String,
    },
    SetActiveInspector {
        inspector: ActiveInspector,
    },
    SetFocusPane {
        pane: ActiveWorkspacePane,
    },
    SetCanvasZoom {
        zoom: f64,
    },
    ToggleNavigatorSection {
        section: SheetSection,
    },
    SetNavigatorSearch {
        query: String,
    },
    SetNavigatorNesting {
        mode: NavigatorNestingMode,
    },
    SetSplitRatio {
        ratio: f64,
    },
    ToggleInspectorPin,
    SwitchInspectorTarget {
        sheet_id: SheetId,
    },
    SetPreviewFilter {
        filter: CanvasPreviewFilter,
    },
    SetPaletteOpen {
        open: bool,
    },
    OpenCheckpointsDialog,
    SetLauncherOpen {
        open: bool,
    },
    ApplyWorkspacePreset {
        preset: WorkspacePreset,
    },
    SyncLeadingWidth {
        width_pt: f32,
    },
    SyncTrailingWidth {
        width_pt: f32,
    },
    ToggleLeadingCollapsedRail,
    ToggleTrailingCollapsedRail,
    HistoryBack,
    HistoryForward,
    DismissDiagnosticBanner {
        sheet_id: SheetId,
    },
    /// Hide the figure-export warning notice in the status footer (`R0.36`).
    DismissFigureExportWarning,
    /// Hide the shell status-banner notice outside the Welcome hub (`R0.36`).
    DismissStatusBanner,
    /// Open CurveBook's allowlisted citation URL (DOI or repository) in the OS browser (`R14.7`).
    OpenCitationUrl,
    OpenRetargetDialog {
        result_sheet_id: SheetId,
        target_data_sheet_id: Option<SheetId>,
    },
    BrushPoint {
        sheet_id: SheetId,
        coord: curvebook_tools::GridCellCoord,
    },
    AddBrushedPoint {
        sheet_id: SheetId,
        coord: curvebook_tools::GridCellCoord,
    },
    ClearBrush,
    CommitDialogAction {
        action: String,
    },
    SetRetargetSelectedDatasets {
        cols: Vec<usize>,
    },
    UpdateImportReshapeState(Box<ImportReshapeState>),
    SetImportReshapePlateValuesText {
        text: String,
        ensure_default_roles: bool,
    },
    SetConvertArchetypeTarget {
        target_kind: TableKind,
    },
    SetExcludeWithReason {
        reason_tag: Option<String>,
        note: Option<String>,
    },
    UpdateFormatSelectedPointsState(Box<FormatSelectedPointsState>),
    UpdateInsertModelEquationState(Box<InsertModelEquationDialogState>),
    SwitchFormatAxesTarget {
        target: AxisTarget,
    },
    SetCommandPaletteQuery {
        query: String,
    },
    SetHookPickerSearch {
        query: String,
    },
    SetCheckpointsDraft {
        name: Option<String>,
        note: Option<String>,
    },
    CloseAllModals,
    OpenManageVariables {
        sheet_id: SheetId,
        column: Option<usize>,
    },
    SelectManageVarColumn {
        sheet_id: SheetId,
        column: usize,
    },
    StageManageVarRecode {
        from: String,
        to: String,
    },
    ClearManageVarRecodes,
    ApplyManageVarRecodes {
        sheet_id: SheetId,
    },
    ApplyManageVarOrder {
        sheet_id: SheetId,
        column: usize,
        order: LevelOrderSpec,
        reference_level: Option<String>,
    },
    CloseManageVariables,
    CancelManageVariables {
        revert_steps: usize,
    },
    OpenImportWizard {
        origin: ImportOriginKind,
        is_live_link: bool,
        target_sheet_id: Option<SheetId>,
    },
    UpdateImportWizard(Box<ImportPasteWizardState>),
    PasteClipboardIntoImportReshape,
    CommitImportWizard {
        close_after: bool,
    },
    CloseImportWizard,
    SavePlateMapFile {
        path: String,
    },
    LoadPlateMapFile {
        path: String,
    },
    LoadPlateLayoutFromSheet {
        layout_sheet_id: SheetId,
        x_grid_sheet_id: Option<SheetId>,
        blank_labels: Vec<String>,
        positive_control_labels: Vec<String>,
    },
    ApplyPlateGridsText {
        label_grid_text: String,
        x_grid_text: String,
        blank_labels: Vec<String>,
        positive_control_labels: Vec<String>,
    },
    OpenTransformModal {
        sheet_id: SheetId,
        kind: transform_modals::TransformModalKind,
    },
    CloseTransformModal,
    GridCopySelection,
    GridCutSelection,
    GridPasteFromClipboard {
        start_coord: Option<curvebook_tools::GridCellCoord>,
    },
    PasteXCellText {
        sheet_id: SheetId,
        x_row: usize,
        x_subcol: usize,
        text: String,
    },
    WorkspaceCopy {
        dpi: u16,
    },
    WorkspaceCopyForSlides {
        dpi: u16,
    },
    SyncResultSelection {
        sheet_id: SheetId,
        tab_index: usize,
        anchor: (usize, usize),
        focus: (usize, usize),
    },
    CanvasPaste {
        target_sheet_id: SheetId,
        anchor_pt: Option<[f64; 2]>,
    },
    PasteImageFromClipboard {
        target_sheet_id: SheetId,
        anchor_pt: Option<[f64; 2]>,
    },
    SetAnalyzeQuery {
        query: String,
    },
    SelectAnalyzeEntry {
        entry_id: String,
        clear_duplicate_prompt: bool,
    },
    ToggleAnalyzeDataset {
        col_index: usize,
    },
    SetAnalyzeSelectedDatasets {
        cols: Vec<usize>,
    },
    SetEditParamsDraft {
        spec: Box<curvebook_doc::AnalysisSpec>,
    },
    SetEditParamsActiveTab {
        tab: String,
    },
    SetEditParamsPairedOrMatched {
        paired: bool,
    },
    SetEditParamsConfidence {
        level: ConfidenceLevel,
    },
    SaveEditParamsAsDefault,
    SelectRetargetSource {
        target_data_sheet_id: SheetId,
    },
    ResolveDuplicateAnalysis {
        modify_existing_id: Option<SheetId>,
    },
    SelectHookConstant {
        index: Option<usize>,
    },
    UnlinkHookConstant,
    UpdatePowerWizardState(Box<PowerWizardState>),
    CommitPowerWizard {
        action: String,
    },
    UpdateChangeGraphTypeState(Box<ChangeGraphTypeState>),
    UpdateFormatGraphState(Box<FormatGraphState>),
    UpdateFormatAxesState(Box<FormatAxesState>),
    UpdateBubblePlotState(Box<BubblePlotInspectorState>),
    UpdatePairwiseState(Box<PairwiseComparisonsState>),
    UpdatePageLayoutState(Box<PageLayoutComposerState>),
    UpdateExportFigureState(Box<ExportFigureState>),
    UpdatePrintDialogState(Box<PrintDialogState>),
    UpdateCreateGraphState(Box<CreateGraphDialogState>),
    UpdateEqualizeScalingState(Box<EqualizeScalingDialogState>),
    UpdateFormatEmbeddedTableState(Box<FormatEmbeddedTableDialogState>),
    UpdateFormatImageState(Box<FormatImageDialogState>),
    UpdateApplyStyleState(Box<ApplyStyleFromState>),
    UpdateRandomizeGroupsState(Box<RandomizeGroupsDialogState>),
    UpdateRandomSampleRowsState(Box<RandomSampleRowsDialogState>),
    UpdateSimulateDistributionsState(Box<SimulateDistributionsDialogState>),
    SetGallerySelection {
        sheet_ids: Vec<SheetId>,
    },
    SetGalleryViewOptions {
        search_query: Option<String>,
        sort_order: Option<GallerySortOrder>,
        current_family_only: Option<bool>,
    },
    OpenExportFigure {
        sheet_id: SheetId,
        target_selected: bool,
    },
    OpenPrintForSheets {
        sheet_ids: Vec<SheetId>,
    },
    OpenCsvExportForSheets {
        sheet_ids: Vec<SheetId>,
    },
    PlaceSelectedOntoNewLayout {
        graph_ids: Vec<SheetId>,
    },
    OpenPreferences {
        category: Option<app_modals::PreferencesCategory>,
    },
    ClosePreferences {
        save: bool,
    },
    SetShortcutBinding {
        command_id: String,
        combo: String,
    },
    ResetShortcutBinding {
        command_id: Option<String>,
    },
    UpdatePreferencesDraft(Box<app_modals::PreferencesModalState>),
    ResolveUnsavedChanges {
        action: app_modals::UnsavedCloseAction,
    },
    OpenFindReplace {
        initial_query: Option<String>,
    },
    CloseFindReplace,
    ExecuteFindReplace {
        replace_all: bool,
        find_query: String,
        replace_with: String,
        match_case: bool,
        match_entire_cell: bool,
    },
    CopyImportWarnings,
    UpdateCsvExportOptions(Box<crate::dialogs_canvas::CsvExportOptionsState>),
    SetFormulaBarInput {
        text: String,
    },
    CommitFormulaBar,
    CancelFormulaBar,
    AcceptFormulaBarCompletion,
    CanvasPinchZoom {
        scale_delta: f64,
        anchor_pt: [f64; 2],
    },
    CanvasPanDelta {
        delta_x: f64,
        delta_y: f64,
    },
    OpenPaletteWithQuery {
        query: String,
    },
    OpenAnalyzeWithCatalog {
        catalog_id: String,
    },
    OpenFitWithEquation {
        equation_id: String,
    },
    JumpToDataColumn {
        sheet_id: SheetId,
        col: usize,
    },
    OpenSheetFind,
    CloseSheetFind,
    RecordSearchRecent(Box<crate::search_engine::UnifiedSearchItem>),
}

/// Outcome produced when committing a buffered [`UiIntent`].
#[derive(Debug, Clone, PartialEq)]
pub enum UiIntentOutcome {
    Engine { command_id: String, outcome: CommandOutcome },
    Ui { command_id: String, result: Option<serde_json::Value> },
    ShellUpdated,
}

/// Per-intent batch commit report returned by [`PendingUiIntents::commit_all_if_settled`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiIntentBatchResult {
    pub results: Vec<Result<UiIntentOutcome, EngineError>>,
}

impl UiIntentBatchResult {
    /// Collect all successful [`UiIntentOutcome`] entries in FIFO order.
    #[must_use]
    pub fn outcomes(&self) -> Vec<UiIntentOutcome> {
        self.results.iter().filter_map(|res| res.as_ref().ok().cloned()).collect()
    }

    /// Return the first [`EngineError`] encountered during batch execution, if any.
    #[must_use]
    pub fn first_error(&self) -> Option<&EngineError> {
        self.results.iter().find_map(|res| res.as_ref().err())
    }

    /// Return the first command ID that succeeded in this batch, useful for recovery journal labels.
    #[must_use]
    pub fn first_succeeded_command_id(&self) -> Option<&str> {
        self.results.iter().find_map(|res| match res {
            Ok(UiIntentOutcome::Engine { command_id, .. } | UiIntentOutcome::Ui { command_id, .. }) => Some(command_id.as_str()),
            _ => None,
        })
    }

    /// Convert to `Result<Vec<UiIntentOutcome>, EngineError>`, preserving the first error when any
    /// intent in the batch failed.
    ///
    /// # Errors
    /// Returns the first [`EngineError`] encountered in the batch.
    pub fn into_first_error_result(self) -> Result<Vec<UiIntentOutcome>, EngineError> {
        let mut outcomes = Vec::with_capacity(self.results.len());
        let mut first_err: Option<EngineError> = None;
        for res in self.results {
            match res {
                Ok(out) => outcomes.push(out),
                Err(err) => {
                    if first_err.is_none() {
                        first_err = Some(err);
                    }
                }
            }
        }
        if let Some(err) = first_err { Err(err) } else { Ok(outcomes) }
    }
}

/// Single-commit FIFO queue buffering GUI intents across discardable `egui` layout passes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PendingUiIntents {
    queue: Vec<UiIntent>,
}

impl PendingUiIntents {
    /// Construct an empty intent queue.
    #[must_use]
    pub const fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Discard all buffered intents without executing them.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Number of queued intents awaiting frame settlement.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.queue.len()
    }

    /// Returns `true` when no intents are buffered.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Inspect queued intents in FIFO order.
    #[must_use]
    pub fn intents(&self) -> &[UiIntent] {
        &self.queue
    }

    /// Enqueue a typed [`UiIntent`].
    pub fn push(&mut self, intent: UiIntent) {
        self.queue.push(intent);
    }

    /// Merge intents captured in a subsequent layout pass of the same frame without duplicating
    /// intents already buffered from an earlier discarded pass (`R15-GUI-05`).
    ///
    /// Preserves intra-pass multiplicity: if the earlier pass buffered `k` copies of an intent and
    /// `next_pass` buffers `m` copies, the merged queue retains `max(k, m)` copies in FIFO order.
    pub fn merge_pass(&mut self, next_pass: Self) {
        if self.queue.is_empty() {
            self.queue = next_pass.queue;
            return;
        }
        if next_pass.queue.is_empty() {
            return;
        }
        let mut unmatched_existing = vec![true; self.queue.len()];
        for intent in next_pass.queue {
            if let Some(pos) =
                self.queue.iter().enumerate().position(|(idx, existing)| unmatched_existing.get(idx).copied().unwrap_or(false) && *existing == intent)
            {
                if let Some(slot) = unmatched_existing.get_mut(pos) {
                    *slot = false;
                }
            } else {
                self.queue.push(intent);
            }
        }
    }

    /// Enqueue an engine command to execute once via `Caller::Ui` after layout settles.
    pub fn push_engine(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::EngineCommand { command_id: command_id.into(), params });
    }

    /// Enqueue a front-end `ui.*` / `app.*` shell command to execute once after layout settles.
    pub fn push_ui(&mut self, command_id: impl Into<String>, params: serde_json::Value) {
        self.queue.push(UiIntent::UiCommand { command_id: command_id.into(), params });
    }

    /// Commit every buffered intent one by one via `Session::execute_as_ui` (`Caller::Ui`) when
    /// `!will_discard`, collecting per-intent results instead of aborting on the first failure.
    ///
    /// When `will_discard` is `true`, leaves the buffered intents intact without executing any
    /// mutations so first-pass input intents survive `egui` layout discard retries and commit once
    /// the frame settles (`R15-GUI-05`).
    ///
    /// Always runs `app.sync_views()` and `sync_history_with_document()` after processing a
    /// non-empty batch so earlier/later mutations never leave views or history stale, and mirrors
    /// the first batch error into `app.canvas_dialogs.status_banner`.
    pub fn commit_all_if_settled(&mut self, app: &mut UiApp, will_discard: bool) -> UiIntentBatchResult {
        if will_discard || self.queue.is_empty() {
            return UiIntentBatchResult::default();
        }
        let pending = std::mem::take(&mut self.queue);
        let mut results = Vec::with_capacity(pending.len());
        let mut first_err_msg: Option<String> = None;
        let mut had_error = false;
        for intent in pending {
            // Preserve active modal state when a preceding commit intent in the same batch fails
            // (e.g. `ExportFigure` / `CsvExportOptions` queuing `[EngineCommand, CloseAllModals]`; `AUD-10-22`).
            if matches!(intent, UiIntent::CloseAllModals) && had_error {
                continue;
            }
            let res = executors::apply_single_ui_intent(app, intent);
            if let Err(ref err) = res {
                had_error = true;
                if first_err_msg.is_none() {
                    first_err_msg = Some(err.to_string());
                }
            }
            results.push(res);
        }
        if let Some(msg) = first_err_msg {
            app.canvas_dialogs.status_banner = Some(msg);
        }
        app.sync_views();
        app.egui_views.sync_history_with_document(app.session.document());
        UiIntentBatchResult { results }
    }

    /// Commit buffered intents once via `Session::execute_as_ui` (`Caller::Ui`) when `!will_discard`.
    ///
    /// Applies every queued intent one by one (without dropping later intents on an earlier error),
    /// preserves queued intents while `will_discard` is `true`, always syncs views and navigation
    /// history, and returns the first [`EngineError`] if any failed.
    ///
    /// # Errors
    /// Returns the first [`EngineError`] if any queued engine or UI command fails validation or preconditions.
    pub fn commit_if_settled(&mut self, app: &mut UiApp, will_discard: bool) -> Result<Vec<UiIntentOutcome>, EngineError> {
        self.commit_all_if_settled(app, will_discard).into_first_error_result()
    }

    /// Commit only [`UiIntent::EngineCommand`] items directly on `session` (`Caller::Ui`) when `!will_discard`.
    ///
    /// # Errors
    /// Returns the first [`EngineError`] if any queued command fails.
    pub fn commit_engine_intents_on_session(&mut self, session: &mut Session, will_discard: bool) -> Result<Vec<CommandOutcome>, EngineError> {
        if will_discard || self.queue.is_empty() {
            return Ok(Vec::new());
        }
        let pending = std::mem::take(&mut self.queue);
        let mut outcomes = Vec::new();
        let mut first_err: Option<EngineError> = None;
        for intent in pending {
            if let UiIntent::EngineCommand { command_id, params } = intent {
                match session.execute_as_ui(&command_id, params) {
                    Ok(out) => outcomes.push(out),
                    Err(err) => {
                        if first_err.is_none() {
                            first_err = Some(err);
                        }
                    }
                }
            }
        }
        if let Some(err) = first_err { Err(err) } else { Ok(outcomes) }
    }
}

#[cfg(test)]
mod tests {
    use super::{PendingUiIntents, UiIntent};
    use crate::UiApp;
    use serde_json::json;

    #[test]
    fn pending_intents_discard_on_multi_pass_and_commit_once_when_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        let create_params = json!({"sheet_id": "d-1", "title": "Committed Test", "kind": "column", "auto_graph": false});
        intents.push_engine("sheet.createData", create_params.clone());
        let discarded = intents.commit_if_settled(&mut app, true)?;
        assert!(discarded.is_empty());
        // Pass-0 intents stay buffered across discarded passes until the frame settles (`R15-GUI-05`).
        assert_eq!(intents.len(), 1);
        assert_eq!(app.document().total_sheet_count(), 0);

        // Re-running on pass 1 and merging the identical intent deduplicates cleanly.
        let mut pass1 = PendingUiIntents::new();
        pass1.push_engine("sheet.createData", create_params);
        intents.merge_pass(pass1);
        assert_eq!(intents.len(), 1);

        let committed = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(committed.len(), 1);
        assert!(intents.is_empty());
        assert_eq!(app.document().total_sheet_count(), 1);
        assert_eq!(app.session().undo_depth(), 1);
        Ok(())
    }

    #[test]
    fn pending_intents_route_grid_and_dialog_actions_once_settled() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = UiApp::new();
        let mut intents = PendingUiIntents::new();
        intents.push(UiIntent::SelectGridCell { sheet_id: None, coord: curvebook_tools::GridCellCoord::new(2, 1, 0) });
        intents.push(UiIntent::SetRowTitlesCollapsed { collapsed: true });
        let outcomes = intents.commit_if_settled(&mut app, false)?;
        assert_eq!(outcomes.len(), 2);
        assert_eq!(app.grid.tool.active, Some(curvebook_tools::GridCellCoord::new(2, 1, 0)));
        assert_eq!(app.focus_widget_id.as_deref(), Some("grid.cell.2.1.0"));
        assert_eq!(app.grid.row_titles_collapsed_override, Some(true));
        Ok(())
    }
}
