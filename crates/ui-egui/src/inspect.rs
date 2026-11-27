//! Shared UI accessibility/inspection tree DTOs (`ui.inspect`, `R8.7`, §2) and desktop
//! `egui_shell::inspect_bridge` entry points.

use curvebook_doc::TableArchetype;
use curvebook_tools::{GridCellCoord, SelectionQuickStats};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(not(target_arch = "wasm32"))]
use crate::UiApp;
#[cfg(not(target_arch = "wasm32"))]
use crate::dialogs_canvas::ActiveDialog;
use crate::dialogs_canvas::AssumptionDiagnosticBanner;
#[cfg(not(target_arch = "wasm32"))]
use crate::interaction::ViewportMetrics;

/// Maximum number of individual selected cell coordinates included in a single `ui.inspect` snapshot.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_INSPECT_SELECTED_COORDS: usize = 500;

/// Uniform accessible node describing any interactive UI menu item, toolbar button, tab, canvas
/// element, grid cell, modal control, or action button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiWidgetNode {
    pub widget_id: String,
    pub role: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
}

/// Top-level menu section exposed by `ui.inspect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiMenuSnapshot {
    pub menu_key: String,
    pub title: String,
    pub bounds: Option<[f64; 4]>,
    pub items: Vec<UiWidgetNode>,
}

/// Navigator tree sheet entry exposed by `ui.inspect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNavigatorSheetNode {
    pub widget_id: String,
    pub sheet_id: String,
    pub section: String,
    pub title: String,
    pub selected: bool,
    pub role: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
}

/// Grid cell / row selection coordinate snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridCoordSnapshot {
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

impl From<GridCellCoord> for GridCoordSnapshot {
    fn from(value: GridCellCoord) -> Self {
        Self { row: value.row, col: value.col, subcol: value.subcol }
    }
}

/// Grid selection & quick-statistics snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSelectionSnapshot {
    pub active_cell: Option<GridCoordSnapshot>,
    pub anchor_cell: Option<GridCoordSnapshot>,
    pub selected_rows: Vec<usize>,
    pub selected_coords: Vec<GridCoordSnapshot>,
    pub selected_cells_count: usize,
    pub quick_stats: Option<SelectionQuickStats>,
}

/// Linked graph/grid brushed data point snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrushedPointSnapshot {
    pub sheet_id: String,
    pub row: usize,
    pub col: usize,
    pub subcol: usize,
}

/// Modal dialog inspection subtree (`dialog`) with tabs, interactive controls, and footer buttons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDialogSnapshot {
    pub dialog_kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_sheet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<[f64; 4]>,
    pub tabs: Vec<UiWidgetNode>,
    pub controls: Vec<UiWidgetNode>,
    pub buttons: Vec<UiWidgetNode>,
}

/// Non-fatal import warnings state exposed by `ui.inspect` (`R10.29`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportWarningsSnapshot {
    pub open: bool,
    pub command_id: String,
    pub source_label: String,
    pub warnings: Vec<crate::ImportWarningItem>,
}

/// Full deterministic workspace UI tree snapshot returned by `ui.inspect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiTreeSnapshot {
    pub active_sheet_id: Option<String>,
    pub active_sheet_kind: Option<String>,
    pub active_dialog: String,
    pub focus_widget_id: Option<String>,
    pub zoom: f64,
    pub undo_depth: usize,
    pub redo_depth: usize,
    pub dirty: bool,
    pub cursor_readout: Option<[f64; 2]>,
    pub grid_selection: GridSelectionSnapshot,
    pub brushed_points: Vec<BrushedPointSnapshot>,
    pub status_banner: Option<String>,
    pub context_menu_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_diagnostic_banner: Option<AssumptionDiagnosticBanner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_warnings: Option<ImportWarningsSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menus: Option<Vec<UiMenuSnapshot>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolbar: Option<Vec<UiWidgetNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub navigator_sheets: Option<Vec<UiNavigatorSheetNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tabs: Option<Vec<UiWidgetNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas_elements: Option<Vec<UiWidgetNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_cells: Option<Vec<UiWidgetNode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog: Option<UiDialogSnapshot>,
}

impl UiTreeSnapshot {
    /// Locate a widget node across all populated snapshot subtrees by `widget_id`.
    #[must_use]
    pub fn find_widget(&self, widget_id: &str) -> Option<UiWidgetNode> {
        if let Some(tb) = &self.toolbar
            && let Some(hit) = tb.iter().find(|n| n.widget_id == widget_id)
        {
            return Some(hit.clone());
        }
        if let Some(tabs) = &self.tabs
            && let Some(hit) = tabs.iter().find(|n| n.widget_id == widget_id)
        {
            return Some(hit.clone());
        }
        if let Some(dlg) = &self.dialog
            && let Some(hit) = dlg.tabs.iter().chain(&dlg.controls).chain(&dlg.buttons).find(|n| n.widget_id == widget_id)
        {
            return Some(hit.clone());
        }
        if let Some(canvas) = &self.canvas_elements
            && let Some(hit) = canvas.iter().find(|n| n.widget_id == widget_id)
        {
            return Some(hit.clone());
        }
        if let Some(grid) = &self.grid_cells
            && let Some(hit) = grid.iter().find(|n| n.widget_id == widget_id)
        {
            return Some(hit.clone());
        }
        if let Some(menus) = &self.menus {
            for m in menus {
                if let Some(hit) = m.items.iter().find(|n| n.widget_id == widget_id) {
                    return Some(hit.clone());
                }
            }
        }
        if let Some(nav) = &self.navigator_sheets
            && let Some(hit) = nav.iter().find(|n| n.widget_id == widget_id || n.sheet_id == widget_id)
        {
            return Some(UiWidgetNode {
                widget_id: hit.widget_id.clone(),
                role: hit.role.clone(),
                label: hit.label.clone(),
                command_id: Some(String::from("sheet.select")),
                value: Some(json!(hit.sheet_id)),
                enabled: hit.enabled,
                disabled_reason: None,
                bounds: hit.bounds,
            });
        }
        None
    }

    /// Resolve center `[x, y]` logical coordinates for `widget_id` if present.
    #[must_use]
    pub fn widget_center(&self, widget_id: &str) -> Option<[f64; 2]> {
        let b = self.find_widget(widget_id)?.bounds?;
        Some([b[0] + b[2] * 0.5, b[1] + b[3] * 0.5])
    }
}

/// Deterministic paint/hit-test Z-layer ordering (`Workspace` < ... < `ModalBackdrop` < `ModalDialog` < `Tooltip`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ZLayer {
    Workspace = 0,
    FloatingOverlay = 10,
    SplitHandle = 20,
    DropdownMenu = 30,
    ModalBackdrop = 40,
    ModalDialog = 50,
    ModalDropdown = 60,
    ContextMenu = 70,
    Tooltip = 80,
}

/// Single failed UI invariant detected by driver parity / layout verification gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParityViolation {
    pub widget_id: String,
    pub check: String,
    pub detail: String,
}

/// Result summary emitted by UI parity / hit-box / accessibility verification passes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ParityReport {
    pub checked_nodes: usize,
    pub checked_text_runs: usize,
    pub violations: Vec<ParityViolation>,
}

impl ParityReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Screenshot crop target supported by `ui.screenshot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ScreenshotRegionTarget {
    #[default]
    FullWindow,
    Canvas,
    Dialog,
}

/// Build a canonical [`UiTreeSnapshot`] from `app` filtered by `scope` (`all`, `menus`,
/// `toolbar`, `navigator`, `tabs`, `canvas`, `grid`, `dialog`) under default viewport metrics.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn inspect_ui_tree(app: &UiApp, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    inspect_ui_tree_for_metrics(app, ViewportMetrics::default(), scope, include_bounds)
}

/// Build a canonical [`UiTreeSnapshot`] under caller-supplied [`ViewportMetrics`], backed by the
/// live `egui_shell::inspect_bridge` pass on native desktop hosts.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn inspect_ui_tree_for_metrics(app: &UiApp, metrics: ViewportMetrics, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    crate::egui_shell::inspect_bridge::inspect_live_egui_snapshot(app, metrics, scope, include_bounds)
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn active_dialog_slug(dialog: &ActiveDialog) -> String {
    match dialog {
        ActiveDialog::None => String::from("none"),
        ActiveDialog::FormatAxes { .. } => String::from("format_axes"),
        ActiveDialog::AnalyzeSheet { .. } => String::from("analyze_sheet"),
        ActiveDialog::DuplicateAnalysisPrompt { .. } => String::from("duplicate_analysis_prompt"),
        ActiveDialog::EditAnalysisParameters { .. } => String::from("edit_analysis_parameters"),
        ActiveDialog::ChangeDataAnalyzed { .. } => String::from("change_data_analyzed"),
        ActiveDialog::ResultViewer { .. } => String::from("result_viewer"),
        ActiveDialog::ConstantHookPicker { .. } => String::from("constant_hook_picker"),
        ActiveDialog::ChangeGraphType { .. } => String::from("change_graph_type"),
        ActiveDialog::FormatGraph { .. } => String::from("format_graph"),
        ActiveDialog::PairwiseComparisons { .. } => String::from("pairwise_comparisons"),
        ActiveDialog::BubblePlotInspector { .. } => String::from("bubble_plot_inspector"),
        ActiveDialog::ApplyStyleFrom { .. } => String::from("apply_style_from"),
        ActiveDialog::PageLayoutComposer { .. } => String::from("page_layout_composer"),
        ActiveDialog::ExportFigure { .. } => String::from("export_figure"),
        ActiveDialog::ConvertArchetype { .. } => String::from("convert_archetype"),
        ActiveDialog::ImportReshape => String::from("import_reshape"),
        ActiveDialog::ExcludeWithReason { .. } => String::from("exclude_with_reason"),
        ActiveDialog::FormatSelectedPoints { .. } => String::from("format_selected_points"),
        ActiveDialog::PowerAndSampleSize => String::from("power_and_sample_size"),
        ActiveDialog::RandomizeGroups => String::from("randomize_groups"),
        ActiveDialog::RandomSampleRows { .. } => String::from("random_sample_rows"),
        ActiveDialog::SimulateDistributions => String::from("simulate_distributions"),
        ActiveDialog::InsertModelEquation { .. } => String::from("insert_model_equation"),
        ActiveDialog::CreateGraph { .. } => String::from("create_graph"),
        ActiveDialog::EqualizeScaling { .. } => String::from("equalize_scaling"),
        ActiveDialog::FormatEmbeddedTable { .. } => String::from("format_embedded_table"),
        ActiveDialog::FormatImage { .. } => String::from("format_image"),
        ActiveDialog::Print { .. } => String::from("print"),
        ActiveDialog::CommandPalette => String::from("command_palette"),
        ActiveDialog::Checkpoints => String::from("checkpoints"),
        ActiveDialog::About => String::from("about"),
        ActiveDialog::Cite => String::from("cite"),
        ActiveDialog::ImportWarnings => String::from("import_warnings"),
        ActiveDialog::ImportProjectMode { .. } => String::from("import_project_mode"),
        ActiveDialog::CsvExportOptions(_) => String::from("csv_export_options"),
        ActiveDialog::ExportAllTablesExistingFiles { .. } => String::from("export_all_tables_existing_files"),
        ActiveDialog::PzfxExportConfirm { .. } => String::from("pzfx_export_confirm"),
    }
}

/// Inspect active grid table dimensions helper for quick external checks.
#[must_use]
pub fn table_dimensions_summary(arch: &TableArchetype) -> (usize, usize) {
    (arch.row_count(), arch.column_count())
}
