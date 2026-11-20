//! Deterministic headless & desktop UI accessibility/inspection tree (`ui.inspect`, `R8.7`, §2).
//!
//! Produces [`UiTreeSnapshot`] from the live [`UiApp`] without requiring an OS window, attaching
//! declarative enablement (`enabled`, `disabled_reason`) from [`crate::modal_predicates`] and
//! deterministic `[x, y, width, height]` screen bounds from the settled `egui_shell::inspect_bridge` pass.

use curvebook_doc::{
    AnalysisKind, ColorChannelMapping, ContingencyMethod, MultivariateSpec, OneWayMethod, PValueFormat, ProjectDocument, RepeatedMeasuresDesign,
    SheetId, SizeChannelMapping, SurvivalAnalysisSpec, TableArchetype, TransformAnalysisSpec, TwoGroupMethod, VariableSource, XColumnMode,
};
use curvebook_tools::GridCellCoord;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::UiApp;
use crate::dialogs_canvas::{
    ActiveDialog, AssumptionDiagnosticBanner, cld_ordering_str, equation_display_mode_str, format_compact_f64, pairwise_display_mode_str,
};
use crate::grid_view::format_cell_display;
use crate::interaction::ViewportMetrics;
use crate::menus::tooltips::{menu_bar_tooltips, toolbar_tooltips};
use crate::modal_predicates::{
    ControlEnablement, evaluate_dialog_button, evaluate_dialog_widget, evaluate_edit_params_tab, is_reml_mixed_effects_active, r_km_trend_test,
    r1_paired_or_matched_allowed, r1_sphericity_or_reml_control, r1b_post_hoc_option_enabled, r1b_sphericity_correction, r2_assume_gaussian_allowed,
    r3_control_column_picker, r4_two_group_diagnostics, r4_two_group_graph_differences, r4_two_group_nonparametric_options,
    r4_two_group_welch_correction, r5_contingency_2x2_effect_sizes, r5_contingency_cochran_armitage_trend, r6_contingency_ci_selector,
    r7_correlation_heatmap_toggle, r8_subcolumn_aggregation, r9_pvalue_precision_spinner, r10_pcr_outcome_column, source_table_shape,
    survival_group_count,
};
use crate::power_wizard::PowerWizardStage;
use curvebook_tools::SelectionQuickStats;

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

/// Full deterministic workspace UI tree snapshot returned by `ui.inspect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiTreeSnapshot {
    pub active_sheet_id: Option<String>,
    pub active_sheet_kind: Option<String>,
    pub active_dialog: String,
    pub focus_widget_id: Option<String>,
    pub zoom: f64,
    pub gui_only: bool,
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

const MAX_INSPECT_SELECTED_COORDS: usize = 500;

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
#[must_use]
pub fn inspect_ui_tree(app: &UiApp, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    inspect_ui_tree_for_metrics(app, ViewportMetrics::default(), scope, include_bounds)
}

/// Build a canonical [`UiTreeSnapshot`] under caller-supplied [`ViewportMetrics`], backed by the
/// live `egui_shell::inspect_bridge` pass on native desktop hosts and the semantic state tree on `wasm32`.
#[must_use]
pub fn inspect_ui_tree_for_metrics(app: &UiApp, metrics: ViewportMetrics, scope: Option<&str>, include_bounds: bool) -> UiTreeSnapshot {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::egui_shell::inspect_bridge::inspect_live_egui_snapshot(app, metrics, scope, include_bounds)
    }
    #[cfg(target_arch = "wasm32")]
    {
        inspect_semantic_snapshot(app, metrics, scope, include_bounds)
    }
}

/// Build the pure structural/semantic [`UiTreeSnapshot`] baseline from `app` and `ProjectDocument`
/// (`bounds: None`; overlaid with live `egui` geometry on desktop hosts).
#[must_use]
pub(crate) fn inspect_semantic_snapshot(app: &UiApp, _metrics: ViewportMetrics, scope: Option<&str>, _include_bounds: bool) -> UiTreeSnapshot {
    let doc = app.session().document();
    let active_id = doc.active_sheet_id.as_ref();
    let active_kind = active_id.and_then(|id| doc.sheet_section(id)).map(sheet_section_name);
    let active_dialog_name = active_dialog_slug(&app.canvas_dialogs.active_dialog);

    let scope_norm = scope.map(|s| s.trim().to_ascii_lowercase());
    let want = |name: &str| match scope_norm.as_deref() {
        None | Some("" | "all" | "workspace") => true,
        Some(target) => target == name,
    };

    let grid_sel = if want("grid") {
        build_grid_selection_snapshot(app, doc)
    } else {
        GridSelectionSnapshot {
            active_cell: app.grid.tool.active.map(GridCoordSnapshot::from),
            anchor_cell: app.grid.tool.anchor.map(GridCoordSnapshot::from),
            selected_rows: Vec::new(),
            selected_coords: Vec::new(),
            selected_cells_count: app.grid.quick_stats.as_ref().map_or(app.grid.selected_coords.len(), |qs| qs.selected_cells),
            quick_stats: app.grid.quick_stats,
        }
    };
    let brushed_points = if want("canvas") || want("grid") { collect_brushed_points(app) } else { Vec::new() };
    let result_diagnostic_banner = active_id.and_then(|id| doc.result_sheet(id)).and_then(AssumptionDiagnosticBanner::from_result);

    UiTreeSnapshot {
        active_sheet_id: active_id.map(|id| id.as_str().to_owned()),
        active_sheet_kind: active_kind,
        active_dialog: active_dialog_name,
        focus_widget_id: app.focus_widget_id.clone(),
        zoom: app.canvas_dialogs.effective_zoom_and_pan().0,
        gui_only: app.is_gui_only(),
        undo_depth: app.session().undo_depth(),
        redo_depth: app.session().redo_depth(),
        dirty: app.session().is_dirty(),
        cursor_readout: app.interaction.last_logical_pos,
        grid_selection: grid_sel,
        brushed_points,
        status_banner: app.canvas_dialogs.status_banner.clone(),
        context_menu_target: app.interaction.context_menu_target.clone(),
        result_diagnostic_banner,
        menus: want("menus").then(|| build_menus_snapshot(app)),
        toolbar: want("toolbar").then(|| build_toolbar_snapshot(app)),
        navigator_sheets: want("navigator").then(|| build_navigator_snapshot(app, doc)),
        tabs: want("tabs").then(|| build_tabs_snapshot(app, doc)),
        canvas_elements: want("canvas").then(|| build_canvas_snapshot(app, doc)),
        grid_cells: want("grid").then(|| build_grid_cells_snapshot(app, doc)),
        dialog: want("dialog").then(|| build_dialog_snapshot(app, doc)).flatten(),
    }
}

fn build_grid_selection_snapshot(app: &UiApp, doc: &ProjectDocument) -> GridSelectionSnapshot {
    let active_ds = doc.active_sheet_id.as_ref().and_then(|id| doc.data_sheet(id));
    let coords = active_ds.map_or_else(|| app.grid.selected_coords.clone(), |ds| app.grid.all_selected_coords(ds));
    let mut selected_rows = app.grid.selected_rows.clone();
    if selected_rows.is_empty() && !coords.is_empty() {
        selected_rows = coords.iter().map(|c| c.row).collect();
        selected_rows.sort_unstable();
        selected_rows.dedup();
    }
    GridSelectionSnapshot {
        active_cell: app.grid.tool.active.map(GridCoordSnapshot::from),
        anchor_cell: app.grid.tool.anchor.map(GridCoordSnapshot::from),
        selected_rows: selected_rows.into_iter().take(MAX_INSPECT_SELECTED_COORDS).collect(),
        selected_coords: coords.iter().take(MAX_INSPECT_SELECTED_COORDS).copied().map(GridCoordSnapshot::from).collect(),
        selected_cells_count: app.grid.quick_stats.as_ref().map_or(coords.len(), |qs| qs.selected_cells),
        quick_stats: app.grid.quick_stats,
    }
}

fn collect_brushed_points(app: &UiApp) -> Vec<BrushedPointSnapshot> {
    if !app.brushed_points.is_empty() {
        return app
            .brushed_points
            .iter()
            .map(|p| BrushedPointSnapshot { sheet_id: p.sheet_id.as_str().to_owned(), row: p.row, col: p.col, subcol: p.subcol })
            .collect();
    }
    app.brushed_cell
        .iter()
        .map(|p| BrushedPointSnapshot { sheet_id: p.sheet_id.as_str().to_owned(), row: p.row, col: p.col, subcol: p.subcol })
        .collect()
}

fn build_menus_snapshot(app: &UiApp) -> Vec<UiMenuSnapshot> {
    menu_bar_tooltips(app)
        .into_iter()
        .map(|sec| {
            let items = sec
                .items
                .into_iter()
                .map(|tip| {
                    let widget_id = format!("menu.{}", tip.command_id);
                    UiWidgetNode {
                        widget_id,
                        role: String::from("menuitem"),
                        label: tip.localized_label.to_owned(),
                        command_id: Some(tip.command_id.to_owned()),
                        value: None,
                        enabled: tip.enabled,
                        disabled_reason: (!tip.enabled).then(|| format!("{} is currently unavailable in this context.", tip.label)),
                        bounds: None,
                    }
                })
                .collect();
            UiMenuSnapshot { menu_key: sec.menu_key.to_owned(), title: sec.title.to_owned(), bounds: None, items }
        })
        .collect()
}

fn build_toolbar_snapshot(app: &UiApp) -> Vec<UiWidgetNode> {
    toolbar_tooltips(app)
        .into_iter()
        .map(|entry| UiWidgetNode {
            widget_id: entry.button.id.to_owned(),
            role: String::from("button"),
            label: entry.caption.to_owned(),
            command_id: Some(entry.button.command_id.to_owned()),
            value: None,
            enabled: entry.tooltip.enabled,
            disabled_reason: (!entry.tooltip.enabled).then(|| format!("{} requires a compatible active sheet.", entry.caption)),
            bounds: None,
        })
        .collect()
}

fn build_navigator_snapshot(app: &UiApp, doc: &ProjectDocument) -> Vec<UiNavigatorSheetNode> {
    let active_id = doc.active_sheet_id.as_ref();
    let mut out = Vec::new();
    for (folder, sheet_ids) in [
        ("data", doc.data_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect::<Vec<_>>()),
        ("info", doc.info_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
        ("results", doc.result_sheets.iter().map(|s| (&s.id, s.title())).collect()),
        ("graphs", doc.graph_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
        ("layouts", doc.layout_sheets.iter().map(|s| (&s.id, s.title.as_str())).collect()),
    ] {
        for (id, title) in sheet_ids {
            let widget_id = format!("nav.sheet.{}", id.as_str());
            out.push(UiNavigatorSheetNode {
                widget_id,
                sheet_id: id.as_str().to_owned(),
                section: folder.to_owned(),
                title: title.to_owned(),
                selected: active_id == Some(id),
                role: String::from("treeitem"),
                label: title.to_owned(),
                enabled: app.nav.search_query.is_empty() || title.to_ascii_lowercase().contains(&app.nav.search_query.to_ascii_lowercase()),
                bounds: None,
            });
        }
    }
    out
}

fn build_tabs_snapshot(app: &UiApp, doc: &ProjectDocument) -> Vec<UiWidgetNode> {
    let mut tabs = Vec::new();
    if let Some(rs_id) = doc.active_sheet_id.as_ref()
        && let Some(rs) = doc.result_sheet(rs_id)
    {
        for (idx, tab) in rs.tabs.iter().enumerate() {
            let active = idx == app.canvas_dialogs.result_view.active_tab_index;
            let widget_id = format!("result.tab.{}", tab.id);
            tabs.push(UiWidgetNode {
                widget_id,
                role: String::from("tab"),
                label: tab.title.clone(),
                command_id: None,
                value: Some(json!({ "selected": active, "index": idx, "tab_id": tab.id })),
                enabled: true,
                disabled_reason: None,
                bounds: None,
            });
        }
    }
    if let Some(dlg) = build_dialog_snapshot(app, doc) {
        tabs.extend(dlg.tabs);
    }
    tabs
}

fn build_canvas_snapshot(app: &UiApp, doc: &ProjectDocument) -> Vec<UiWidgetNode> {
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    if (doc.graph_sheet(active_id).is_some() || doc.layout_sheet(active_id).is_some())
        && let Ok(scene) = app.session().render_sheet_scene(active_id)
    {
        for (idx, pt) in scene.plotted_points.iter().enumerate() {
            nodes.push(UiWidgetNode {
                widget_id: format!("canvas.point.{}.{}.{}", pt.source_cell.row, pt.source_cell.col, idx),
                role: String::from("datapoint"),
                label: format!("Row {} Col {}", pt.source_cell.row + 1, pt.source_cell.col + 1),
                command_id: None,
                value: Some(json!({
                    "series_index": pt.source_cell.col,
                    "point_index": pt.source_cell.row,
                    "subcol": pt.source_cell.subcol,
                })),
                enabled: true,
                disabled_reason: None,
                bounds: None,
            });
        }
    }
    if let Some(rs) = doc.result_sheet(active_id)
        && let Some(banner) = AssumptionDiagnosticBanner::from_result(rs)
    {
        nodes.push(UiWidgetNode {
            widget_id: String::from("result.diagnostic_banner"),
            role: String::from("banner"),
            label: banner.summary.clone(),
            command_id: Some(String::from("ui.result.inspectDiagnostics")),
            value: Some(json!({
                "source_result_id": banner.source_result_id.as_str(),
                "severity": format!("{:?}", banner.severity).to_ascii_lowercase(),
                "summary": banner.summary,
                "diagnostic_metric": banner.diagnostic_metric,
                "limitation_note": banner.limitation_note,
                "detail_tab_ids": banner.detail_tab_ids,
            })),
            enabled: !banner.detail_tab_ids.is_empty(),
            disabled_reason: banner.detail_tab_ids.is_empty().then(|| String::from("No detail tabs available.")),
            bounds: None,
        });
        for tab_id in &banner.detail_tab_ids {
            let tab_label = rs.tabs.iter().find(|t| &t.id == tab_id).map_or_else(|| tab_id.clone(), |t| t.title.clone());
            let widget_id = format!("result.diagnostic_banner.tab.{tab_id}");
            nodes.push(UiWidgetNode {
                widget_id,
                role: String::from("link"),
                label: tab_label,
                command_id: Some(String::from("ui.result.inspectDiagnostics")),
                value: Some(json!({
                    "source_result_id": banner.source_result_id.as_str(),
                    "tab_id": tab_id,
                })),
                enabled: true,
                disabled_reason: None,
                bounds: None,
            });
        }
    }
    nodes
}

fn build_grid_cells_snapshot(app: &UiApp, doc: &ProjectDocument) -> Vec<UiWidgetNode> {
    let Some(active_id) = doc.active_sheet_id.as_ref() else {
        return Vec::new();
    };
    let Some(ds) = doc.data_sheet(active_id) else {
        return Vec::new();
    };
    let frame = app.grid.tool.format_visible_window(ds);
    let win_start_row = frame.window.start_row;
    let win_end_row = frame.window.end_row;
    let discontig: Option<std::collections::HashSet<GridCellCoord>> = (app.grid.selected_coords.len() > 1
        || (!app.grid.selected_coords.is_empty() && app.grid.tool.anchor == app.grid.tool.active))
        .then(|| app.grid.selected_coords.iter().copied().collect());
    let mut nodes: Vec<UiWidgetNode> = frame
        .cells
        .into_iter()
        .map(|cell| {
            let widget_id = format!("grid.cell.{}.{}.{}", cell.coord.row, cell.coord.col, cell.coord.subcol);
            let is_calculated = matches!(
                &ds.archetype,
                TableArchetype::MultipleVariables(mv)
                    if mv.variables.get(cell.coord.col).is_some_and(|v| matches!(v.source, VariableSource::Calculated { .. }))
            );
            let enabled = !ds.frozen && !is_calculated;
            let disabled_reason = if ds.frozen {
                Some(String::from("Data sheet is frozen (read-only)."))
            } else if is_calculated {
                Some(String::from("Calculated variable cells are formula-derived (read-only)."))
            } else {
                None
            };
            let selected = discontig.as_ref().map_or(cell.is_selected, |set| set.contains(&cell.coord));
            UiWidgetNode {
                widget_id,
                role: String::from("gridcell"),
                label: format!("R{}C{}", cell.coord.row + 1, cell.coord.col + 1),
                command_id: None,
                value: Some(json!({
                    "row": cell.coord.row,
                    "col": cell.coord.col,
                    "subcol": cell.coord.subcol,
                    "text": cell.display_text,
                    "excluded": cell.excluded,
                    "selected": selected,
                })),
                enabled,
                disabled_reason,
                bounds: None,
            }
        })
        .collect();

    let (x_subcols, is_generated_x) = match &ds.archetype {
        TableArchetype::Xy(xy) => (xy.x_mode.subcolumn_count().max(1), matches!(xy.x_mode, XColumnMode::GeneratedSequence { .. })),
        TableArchetype::Survival(_) => (1, false),
        _ => (0, false),
    };
    if x_subcols > 0 {
        for row in win_start_row..win_end_row {
            for subcol in 0..x_subcols {
                let widget_id = format!("grid.xcell.{row}.{subcol}");
                let (text, excluded) = if is_generated_x {
                    let generated = match &ds.archetype {
                        TableArchetype::Xy(xy) => xy.x_value_at(row).map_or_else(String::new, format_compact_f64),
                        _ => String::new(),
                    };
                    (generated, false)
                } else if let Some(cell) = ds.x_or_time_cell(row, subcol) {
                    (format_cell_display(cell, &app.locale_preset), cell.excluded)
                } else {
                    (String::new(), false)
                };
                let enabled = !ds.frozen && !is_generated_x;
                let disabled_reason = if ds.frozen {
                    Some(String::from("Data sheet is frozen (read-only)."))
                } else if is_generated_x {
                    Some(String::from("X column is a generated sequence (read-only)."))
                } else {
                    None
                };
                nodes.push(UiWidgetNode {
                    widget_id,
                    role: String::from("gridcell"),
                    label: format!("R{}X{}", row + 1, subcol + 1),
                    command_id: None,
                    value: Some(json!({
                        "row": row,
                        "subcol": subcol,
                        "text": text,
                        "excluded": excluded,
                        "selected": app.grid.active_x_cell == Some((row, subcol)),
                    })),
                    enabled,
                    disabled_reason,
                    bounds: None,
                });
            }
        }
    }
    if let TableArchetype::MultipleVariables(mv) = &ds.archetype {
        for (col_idx, var) in mv.variables.iter().enumerate() {
            let widget_id = format!("formula_bar.open_col.{col_idx}");
            nodes.push(UiWidgetNode {
                widget_id,
                role: String::from("button"),
                label: format!("Formula for {}", var.title),
                command_id: Some(String::from("ui.table.openFormulaBar")),
                value: Some(json!({
                    "column": col_idx,
                    "title": var.title,
                    "calculated": matches!(var.source, VariableSource::Calculated { .. }),
                })),
                enabled: !ds.frozen,
                disabled_reason: ds.frozen.then(|| String::from("Data sheet is frozen (read-only).")),
                bounds: None,
            });
        }
        let fb = &app.grid.formula_bar;
        nodes.push(UiWidgetNode {
            widget_id: String::from("formula_bar.input"),
            role: String::from("textbox"),
            label: String::from("Formula bar"),
            command_id: None,
            value: Some(json!({
                "active": fb.active,
                "target_column": fb.target_column,
                "target_title": fb.target_title,
                "input": fb.input,
                "is_valid": fb.is_valid,
                "parse_error": fb.parse_error,
            })),
            enabled: !ds.frozen,
            disabled_reason: ds.frozen.then(|| String::from("Data sheet is frozen (read-only).")),
            bounds: None,
        });
        let has_target = fb.target_column.is_some() || !fb.target_title.trim().is_empty();
        let commit_enabled = !ds.frozen && fb.active && has_target && fb.is_valid && fb.parse_error.is_none();
        let commit_disabled_reason = if ds.frozen {
            Some(String::from("Data sheet is frozen (read-only)."))
        } else if !fb.active || !has_target {
            Some(String::from("Formula bar is not active."))
        } else if let Some(err) = &fb.parse_error {
            Some(err.clone())
        } else if !fb.is_valid {
            Some(String::from("Formula cannot be empty or invalid."))
        } else {
            None
        };
        nodes.push(UiWidgetNode {
            widget_id: String::from("formula_bar.commit"),
            role: String::from("button"),
            label: String::from("Apply formula"),
            command_id: Some(String::from("table.setCalculatedVariable")),
            value: Some(json!({
                "target_column": fb.target_column,
                "is_valid": fb.is_valid,
                "parse_error": fb.parse_error,
            })),
            enabled: commit_enabled,
            disabled_reason: commit_disabled_reason,
            bounds: None,
        });
        nodes.push(UiWidgetNode {
            widget_id: String::from("formula_bar.cancel"),
            role: String::from("button"),
            label: String::from("Cancel formula"),
            command_id: None,
            value: Some(json!({ "active": fb.active })),
            enabled: fb.active,
            disabled_reason: (!fb.active).then(|| String::from("Formula bar is not active.")),
            bounds: None,
        });
    }
    nodes
}

pub(crate) fn dialog_widget_descriptors(app: &UiApp) -> Option<DialogBuildParts> {
    let dialogs = &app.canvas_dialogs;
    if dialogs.active_dialog == ActiveDialog::None {
        return None;
    }
    let doc = app.session().document();
    Some(match &dialogs.active_dialog {
        ActiveDialog::PowerAndSampleSize => build_power_dialog_nodes(app, 0.0, 0.0, false),
        ActiveDialog::FormatSelectedPoints { graph_sheet_id } => build_format_points_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::AnalyzeSheet { source_sheet_id } => build_analyze_dialog_nodes(app, doc, source_sheet_id, 0.0, 0.0, false),
        ActiveDialog::EditAnalysisParameters { result_sheet_id } => build_edit_params_dialog_nodes(app, doc, result_sheet_id, 0.0, 0.0, false),
        ActiveDialog::ImportReshape => build_import_reshape_nodes(app, 0.0, 0.0, false),
        ActiveDialog::ConvertArchetype { sheet_id } => build_convert_archetype_nodes(app, sheet_id, 0.0, 0.0, false),
        ActiveDialog::ChangeGraphType { graph_sheet_id } => build_change_graph_type_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::FormatGraph { graph_sheet_id } => build_format_graph_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::FormatAxes { sheet_id } => build_format_axes_nodes(app, sheet_id, 0.0, 0.0, false),
        ActiveDialog::PairwiseComparisons { graph_sheet_id } => build_pairwise_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::BubblePlotInspector { graph_sheet_id } => build_bubble_plot_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::ApplyStyleFrom { target_graph_id } => build_apply_style_nodes(app, target_graph_id, 0.0, 0.0, false),
        ActiveDialog::PageLayoutComposer { layout_sheet_id } => build_page_layout_nodes(app, layout_sheet_id, 0.0, 0.0, false),
        ActiveDialog::ExportFigure { sheet_id } => build_export_figure_nodes(app, sheet_id, 0.0, 0.0, false),
        ActiveDialog::RandomizeGroups => build_randomize_groups_nodes(app, 0.0, 0.0, false),
        ActiveDialog::RandomSampleRows { .. } => build_random_sample_rows_nodes(app, 0.0, 0.0, false),
        ActiveDialog::SimulateDistributions => build_simulate_distributions_nodes(app, 0.0, 0.0, false),
        ActiveDialog::InsertModelEquation { graph_sheet_id } => build_insert_model_equation_nodes(app, graph_sheet_id, 0.0, 0.0, false),
        ActiveDialog::Print { sheet_id } => build_print_dialog_nodes(app, sheet_id.as_ref(), 0.0, 0.0, false),
        other => build_generic_dialog_nodes(app, other, 0.0, 0.0, false),
    })
}

fn build_dialog_snapshot(app: &UiApp, doc: &ProjectDocument) -> Option<UiDialogSnapshot> {
    let dialogs = &app.canvas_dialogs;
    let (title, target_sheet_id, stage, tabs, controls, buttons_list) = dialog_widget_descriptors(app)?;

    let buttons = buttons_list
        .into_iter()
        .map(|(id, label)| {
            let pred = evaluate_dialog_button(dialogs, doc, id).unwrap_or_else(ControlEnablement::enabled);
            UiWidgetNode {
                widget_id: format!("dialog.button.{id}"),
                role: String::from("button"),
                label: label.to_owned(),
                command_id: None,
                value: None,
                enabled: pred.enabled,
                disabled_reason: pred.disabled_reason,
                bounds: None,
            }
        })
        .collect();

    Some(UiDialogSnapshot {
        dialog_kind: active_dialog_slug(&dialogs.active_dialog),
        title: title.to_owned(),
        target_sheet_id,
        stage,
        bounds: None,
        tabs,
        controls,
        buttons,
    })
}

fn node(
    widget_id: impl Into<String>,
    role: &str,
    label: impl Into<String>,
    value: Value,
    enable: ControlEnablement,
    bounds: Option<[f64; 4]>,
) -> UiWidgetNode {
    UiWidgetNode {
        widget_id: widget_id.into(),
        role: role.to_owned(),
        label: label.into(),
        command_id: None,
        value: Some(value),
        enabled: enable.enabled,
        disabled_reason: enable.disabled_reason,
        bounds,
    }
}

fn insp_tr(app: &UiApp, key: &str, fallback: &'static str) -> &'static str {
    let norm = crate::i18n::normalize_locale(&app.locale);
    crate::i18n::try_tr(norm, key).unwrap_or(fallback)
}

type DialogBuildParts = (&'static str, Option<String>, Option<String>, Vec<UiWidgetNode>, Vec<UiWidgetNode>, Vec<(&'static str, &'static str)>);

fn build_power_dialog_nodes(app: &UiApp, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let pw = &app.canvas_dialogs.power_wizard;
    let pos = |row: usize| include_bounds.then_some([bx, by + (row as f64) * 28.0, 220.0, 22.0]);
    let one_sided_enable = if pw.family.supports_one_sided() {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(format!("{} supports two-tailed significance testing only.", pw.family.label()))
    };
    let tabs = PowerWizardStage::ALL
        .iter()
        .enumerate()
        .map(|(i, st)| {
            node(
                format!("power.stage.{}", i + 1),
                "tab",
                st.label(),
                json!(pw.stage == *st),
                ControlEnablement::enabled(),
                include_bounds.then_some([bx + (i as f64) * 110.0, by, 104.0, 22.0]),
            )
        })
        .collect();

    let mut controls = vec![
        node("power.family", "combobox", "Test family", json!(pw.family.label()), ControlEnablement::enabled(), pos(1)),
        node("power.solve_target", "combobox", "Solve for", json!(format!("{:?}", pw.solve_target)), ControlEnablement::enabled(), pos(2)),
        node("power.effect_mode", "combobox", "Effect size mode", json!(format!("{:?}", pw.effect_mode)), ControlEnablement::enabled(), pos(3)),
        node("power.standardized_effect", "textbox", "Standardized effect", json!(pw.standardized_effect), ControlEnablement::enabled(), pos(4)),
        node(
            "power.within_pair_correlation",
            "textbox",
            "Paired correlation r",
            json!(pw.within_pair_correlation),
            ControlEnablement::enabled(),
            pos(5),
        ),
        node("power.alpha", "textbox", "Alpha", json!(pw.alpha), ControlEnablement::enabled(), pos(6)),
        node("power.target_power", "textbox", "Target power", json!(pw.target_power), ControlEnablement::enabled(), pos(7)),
        node("power.two_tailed", "checkbox", "Two-tailed", json!(pw.two_tailed), ControlEnablement::enabled(), pos(8)),
        node("power.one_sided", "radio", "One-tailed", json!(!pw.two_tailed), one_sided_enable, pos(9)),
        node(
            "power.curve_hover",
            "chart",
            "Power vs N Curve",
            json!({
                "hover_n": pw.hover_n,
                "hover_power": pw.hover_power,
                "curve_len": pw.result.as_ref().map_or(0, |r| r.power_curve.len()),
                "sensitivity_len": pw.sensitivity_cells().len(),
            }),
            ControlEnablement::enabled(),
            include_bounds.then_some([bx, by + 260.0, 340.0, 96.0]),
        ),
    ];
    if let Some(sigma_d) = pw.computed_paired_sigma_d() {
        controls.push(node("power.paired_sigma_d", "readonly", "Paired sigma_d", json!(sigma_d), ControlEnablement::enabled(), pos(10)));
    }
    if let Some(ref res) = pw.result {
        controls.push(node(
            "power.result_summary",
            "status",
            "Solved power result",
            json!({
                "n_per_group": res.n_per_group,
                "n_group2": res.n_group2,
                "total_n": res.total_n,
                "achieved_power": res.achieved_power,
                "effect_size": res.effect_size,
            }),
            ControlEnablement::enabled(),
            pos(11),
        ));
    }

    let btns = vec![
        ("create", insp_tr(app, "w6.view.btn.create", "Create")),
        ("next", "Next"),
        ("back", "Back"),
        ("cancel", insp_tr(app, "btn.cancel", "Cancel")),
    ];
    (insp_tr(app, "w6.view.modal.power_and_sample_size", "Power & Sample Size"), None, Some(pw.stage.label().to_owned()), tabs, controls, btns)
}

fn build_format_points_nodes(app: &UiApp, graph_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let fp = &app.canvas_dialogs.format_points;
    let pos = |row: usize| include_bounds.then_some([bx, by + (row as f64) * 28.0, 220.0, 22.0]);
    let controls = vec![
        node("format_points.count", "readonly", "Selected points", json!(fp.selected_points.len()), ControlEnablement::enabled(), pos(0)),
        node("format_points.color", "textbox", "Symbol color", json!(fp.color), ControlEnablement::enabled(), pos(1)),
        node("format_points.symbol", "combobox", "Symbol shape", json!(fp.symbol), ControlEnablement::enabled(), pos(2)),
        node("format_points.size_pt", "textbox", "Symbol size (pt)", json!(fp.size_pt), ControlEnablement::enabled(), pos(3)),
        node("format_points.show_row_labels", "checkbox", "Show row labels", json!(fp.show_row_labels), ControlEnablement::enabled(), pos(4)),
        node("format_points.clear", "checkbox", "Clear overrides", json!(fp.clear_overrides), ControlEnablement::enabled(), pos(5)),
    ];
    (
        insp_tr(app, "cmd.graph.formatSelectedPoints", "Format Selected Points"),
        Some(graph_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_analyze_dialog_nodes(
    app: &UiApp,
    doc: &ProjectDocument,
    source_sheet_id: &SheetId,
    bx: f64,
    by: f64,
    include_bounds: bool,
) -> DialogBuildParts {
    let st = &app.canvas_dialogs.analyze;
    let pos = |row: usize| include_bounds.then_some([bx, by + (row as f64) * 28.0, 240.0, 22.0]);
    let (_rows, _cols, max_sub) = source_table_shape(doc, Some(source_sheet_id));
    let mut controls = vec![
        node("analyze.search", "textbox", "Search analyses", json!(st.search_query), ControlEnablement::enabled(), pos(0)),
        node("analyze.entry", "listbox", "Selected analysis", json!(st.selected_entry_id), ControlEnablement::enabled(), pos(1)),
        node(
            "analyze.correlation_all_pairs",
            "checkbox",
            "All-pairs Correlation matrix",
            json!(st.correlation_all_pairs),
            ControlEnablement::enabled(),
            pos(2),
        ),
        node(
            "analyze.correlation.heatmap",
            "checkbox",
            "Create correlation heatmap",
            json!(st.correlation_heatmap),
            r7_correlation_heatmap_toggle(st.correlation_all_pairs),
            pos(3),
        ),
        node("analyze.descriptive.subcolumn_agg", "radiogroup", "Subcolumn aggregation", json!(max_sub), r8_subcolumn_aggregation(max_sub), pos(4)),
    ];
    for (idx, cand) in st.variable_candidates.iter().take(12).enumerate() {
        let pcr_gate = r10_pcr_outcome_column(cand.col_index, &st.selected_data_sets);
        controls.push(node(
            format!("analyze.pcr.outcome.{}", cand.col_index),
            "comboboxitem",
            format!("PCR Outcome: {}", cand.title),
            json!(st.pcr_outcome_column == Some(cand.col_index)),
            pcr_gate,
            pos(5 + idx),
        ));
    }
    if matches!(st.selected_entry_id.as_deref(), Some("transform.math" | "transform.select")) {
        let base_row = 5 + st.variable_candidates.len().min(12);
        controls.push(node(
            "analyze.transform.x_transform",
            "combobox",
            "X Transform",
            json!(st.transform_x),
            ControlEnablement::enabled(),
            pos(base_row),
        ));
        controls.push(node(
            "analyze.transform.y_transform",
            "combobox",
            "Y Transform",
            json!(st.transform_y),
            ControlEnablement::enabled(),
            pos(base_row + 1),
        ));
        controls.push(node(
            "analyze.transform.error_propagation",
            "combobox",
            "Error propagation",
            json!(st.transform_error_propagation),
            ControlEnablement::enabled(),
            pos(base_row + 2),
        ));
        controls.push(node(
            "analyze.transform.formula",
            "textbox",
            "Row filter formula",
            json!(st.transform_formula),
            ControlEnablement::enabled(),
            pos(base_row + 3),
        ));
    }
    if st.selected_entry_id.as_deref() == Some("transform.select") {
        let base_row = controls.len();
        controls.push(node(
            "analyze.transform.filter_formula",
            "textbox",
            "Filter formula",
            json!(st.transform_formula),
            ControlEnablement::enabled(),
            pos(base_row),
        ));
        controls.push(node(
            "analyze.transform.target_archetype",
            "combobox",
            "Target table archetype",
            json!(format!("{:?}", st.transform_target_archetype)),
            ControlEnablement::enabled(),
            pos(base_row + 1),
        ));
        controls.push(node(
            "analyze.transform.selected_data_sets",
            "textbox",
            "Selected data sets",
            json!(st.selected_data_sets),
            ControlEnablement::enabled(),
            pos(base_row + 2),
        ));
    }
    if st.selected_entry_id.as_deref() == Some("transform.baseline") {
        let base_row = controls.len();
        controls.push(node(
            "analyze.transform.baseline_columns",
            "textbox",
            "Baseline columns",
            json!(st.baseline_columns),
            ControlEnablement::enabled(),
            pos(base_row),
        ));
        controls.push(node(
            "analyze.transform.baseline_operation",
            "combobox",
            "Baseline operation",
            json!(format!("{:?}", st.baseline_operation)),
            ControlEnablement::enabled(),
            pos(base_row + 1),
        ));
        controls.push(node(
            "analyze.transform.selected_data_sets",
            "textbox",
            "Selected data sets",
            json!(st.selected_data_sets),
            ControlEnablement::enabled(),
            pos(base_row + 2),
        ));
    }
    push_r9_analyze_controls(st, doc, source_sheet_id, &mut controls, &pos);
    (
        insp_tr(app, "w6.view.modal.analyze_data", "Analyze Data"),
        Some(source_sheet_id.as_str().to_owned()),
        st.extracted_result_tab_id.clone(),
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn push_r9_analyze_controls(
    st: &crate::dialogs_canvas::AnalyzeDialogState,
    doc: &ProjectDocument,
    source_sheet_id: &SheetId,
    controls: &mut Vec<UiWidgetNode>,
    pos: &impl Fn(usize) -> Option<[f64; 4]>,
) {
    let mut row = controls.len();
    let mut push = |id: &str, role: &str, label: &str, value: Value, gate: ControlEnablement| {
        controls.push(node(id, role, label, value, gate, pos(row)));
        row += 1;
    };
    push(
        "analyze.assume_gaussian",
        "checkbox",
        "Assume Gaussian",
        json!(!matches!(
            st.two_group_method,
            Some(
                TwoGroupMethod::MannWhitneyU
                    | TwoGroupMethod::WilcoxonSignedRank
                    | TwoGroupMethod::PairedSignTest
                    | TwoGroupMethod::KolmogorovSmirnov
            )
        )),
        r2_assume_gaussian_allowed(doc, Some(source_sheet_id)),
    );
    push(
        "analyze.paired_or_matched",
        "checkbox",
        "Paired / Matched",
        json!(matches!(st.two_group_method, Some(TwoGroupMethod::PairedT | TwoGroupMethod::WilcoxonSignedRank | TwoGroupMethod::PairedSignTest))),
        r1_paired_or_matched_allowed(doc, Some(source_sheet_id)),
    );
    push(
        "analyze.two_group_method",
        "combobox",
        "Two-group method",
        json!(st.two_group_method.map(|m| format!("{m:?}"))),
        ControlEnablement::enabled(),
    );
    push(
        "analyze.contingency_method",
        "combobox",
        "Contingency / Agreement method",
        json!(st.contingency_method.as_ref().map(|m| format!("{m:?}"))),
        ControlEnablement::enabled(),
    );
    push("analyze.null_proportion", "spinbutton", "Binomial null proportion", json!(st.contingency_null_proportion), ControlEnablement::enabled());
    push("analyze.expected_proportions", "textbox", "Expected proportions", json!(st.contingency_expected_proportions), ControlEnablement::enabled());
    push("analyze.factorial.three_way", "checkbox", "Three-way ANOVA", json!(st.factorial_three_way), ControlEnablement::enabled());
    push(
        "analyze.factorial.repeated_measures",
        "combobox",
        "Repeated measures factor",
        json!(format!("{:?}", st.factorial_repeated_measures)),
        ControlEnablement::enabled(),
    );
    push(
        "analyze.factorial.residual_covariance",
        "combobox",
        "Residual covariance",
        json!(format!("{:?}", st.factorial_residual_covariance)),
        r1_sphericity_or_reml_control(st.factorial_repeated_measures != RepeatedMeasuresDesign::None),
    );
    push(
        "analyze.factorial.multiplicity",
        "combobox",
        "Factorial multiplicity",
        json!(format!("{:?}", st.factorial_multiplicity)),
        ControlEnablement::enabled(),
    );
    push("analyze.cluster.distance", "combobox", "Cluster distance", json!(format!("{:?}", st.cluster_distance)), ControlEnablement::enabled());
    push("analyze.cluster.linkage", "combobox", "Cluster linkage", json!(format!("{:?}", st.cluster_linkage)), ControlEnablement::enabled());
    push("analyze.cluster.k_means_k", "spinbutton", "K-means k", json!(st.cluster_k_means_k), ControlEnablement::enabled());
    push("analyze.cluster.k_sweep_max", "spinbutton", "K-means sweep max", json!(st.cluster_k_sweep_max), ControlEnablement::enabled());
    push("analyze.pca.standardize", "checkbox", "Standardize PCA variables", json!(st.pca_standardize), ControlEnablement::enabled());
    push("analyze.correlation.spearman_rank", "checkbox", "Spearman rank correlation", json!(st.corr_spearman_rank), ControlEnablement::enabled());
    push("analyze.imputation.m", "spinbutton", "Imputed datasets (m)", json!(st.imputation_m), ControlEnablement::enabled());
    push("analyze.glm.family", "combobox", "GLM family", json!(format!("{:?}", st.glm_family)), ControlEnablement::enabled());
    push("analyze.glm.compute_vif", "checkbox", "Compute VIF", json!(st.glm_compute_vif), ControlEnablement::enabled());
    push("analyze.cox.tie_method", "combobox", "Cox tie method", json!(format!("{:?}", st.cox_tie_method)), ControlEnablement::enabled());
    push("analyze.km.mv_time_column", "combobox", "Time variable column", json!(st.km_mv_time_column), ControlEnablement::enabled());
    push("analyze.km.mv_status_column", "combobox", "Event / status column", json!(st.km_mv_status_column), ControlEnablement::enabled());
    push("analyze.km.mv_group_column", "combobox", "Group / strata column", json!(st.mv_group_column), ControlEnablement::enabled());
    push("analyze.km.log_rank_mantel_cox", "checkbox", "Log-rank (Mantel-Cox)", json!(st.km_log_rank_mantel_cox), ControlEnablement::enabled());
    push(
        "analyze.km.gehan_breslow_wilcoxon",
        "checkbox",
        "Gehan-Breslow-Wilcoxon",
        json!(st.km_gehan_breslow_wilcoxon),
        ControlEnablement::enabled(),
    );
    let km_trend_gate = r_km_trend_test(survival_group_count(doc, Some(source_sheet_id), st.mv_group_column));
    push("analyze.km.trend_test", "checkbox", "Log-rank trend test", json!(st.km_trend_test), km_trend_gate.clone());
    push("analyze.km.log_rank_for_trend", "checkbox", "Log-rank test for trend", json!(st.km_trend_test), km_trend_gate);
    push(
        "analyze.km.multiplicity_method",
        "combobox",
        "Multiplicity correction",
        json!(format!("{:?}", st.km_multiplicity_method)),
        ControlEnablement::enabled(),
    );
    push("analyze.fit.equation_id", "textbox", "Model equation ID", json!(st.equation_id), ControlEnablement::enabled());
    push("analyze.fit.method", "combobox", "Fit method", json!(format!("{:?}", st.fit_method)), ControlEnablement::enabled());
    push("analyze.fit.weighting", "combobox", "Weighting scheme", json!(format!("{:?}", st.fit_weighting)), ControlEnablement::enabled());
    push("analyze.fit.custom_dsl", "textbox", "Custom DSL equation", json!(st.custom_dsl), ControlEnablement::enabled());
    if st.selected_entry_id.as_deref() == Some("transform.random_sample") {
        push("analyze.sample.sample_size", "spinbutton", "Sample size", json!(st.sample_size), ControlEnablement::enabled());
        push("analyze.sample.seed", "spinbutton", "RNG seed", json!(st.sample_seed), ControlEnablement::enabled());
        push("analyze.sample.stratify_column", "combobox", "Stratify column", json!(st.sample_stratify_column), ControlEnablement::enabled());
        push(
            "analyze.sample.proportional_allocation",
            "checkbox",
            "Proportional allocation",
            json!(st.sample_proportional),
            ControlEnablement::enabled(),
        );
    }
    if st.selected_entry_id.as_deref() == Some("stats.pvalue_stack") {
        push(
            "analyze.pvalue_stack.method",
            "combobox",
            "Multiplicity procedure",
            json!(format!("{:?}", st.pvalue_stack_method)),
            ControlEnablement::enabled(),
        );
        push(
            "analyze.pvalue_stack.threshold",
            "spinbutton",
            "Alpha / FDR Q threshold",
            json!(st.pvalue_stack_threshold),
            ControlEnablement::enabled(),
        );
        push("analyze.pvalue_stack.column", "combobox", "P-value column", json!(st.pvalue_stack_column), ControlEnablement::enabled());
    }
}

fn build_edit_params_dialog_nodes(
    app: &UiApp,
    doc: &ProjectDocument,
    result_sheet_id: &SheetId,
    bx: f64,
    by: f64,
    include_bounds: bool,
) -> DialogBuildParts {
    let ep = &app.canvas_dialogs.edit_params;
    let (rows, cols, max_sub) = source_table_shape(doc, Some(result_sheet_id));
    let pos = |row: usize| include_bounds.then_some([bx, by + (row as f64) * 26.0, 240.0, 20.0]);
    let tab_specs = [
        ("experimental_design", "Experimental Design"),
        ("repeated_measures", "Repeated Measures"),
        ("multiple_comparisons", "Multiple Comparisons"),
        ("options", "Options"),
        ("diagnostics", "Diagnostics / Residuals"),
    ];
    let tabs = tab_specs
        .iter()
        .enumerate()
        .map(|(i, (tid, tlabel))| {
            node(
                format!("edit.tab.{tid}"),
                "tab",
                *tlabel,
                json!(ep.active_tab == *tid),
                evaluate_edit_params_tab(ep, tid),
                include_bounds.then_some([bx + (i as f64) * 114.0, by, 108.0, 22.0]),
            )
        })
        .collect();

    let (method, pv_fmt, cont_spec, two_method) = extract_edit_params_details(ep);
    let source_sheet_id = ep.draft.as_ref().map(|d| &d.source_sheet_id);
    let reml_active = is_reml_mixed_effects_active(ep, doc);
    let gg_checked = ep.draft.as_ref().is_none_or(|d| {
        !matches!(
            d.kind,
            AnalysisKind::OneWayAnova(ref a) if matches!(a.method, OneWayMethod::RepeatedMeasures { geisser_greenhouse: false })
        )
    });
    let mut controls = vec![
        node(
            "edit.assume_gaussian",
            "checkbox",
            "Assume Gaussian",
            json!(ep.assume_gaussian),
            r2_assume_gaussian_allowed(doc, source_sheet_id),
            pos(1),
        ),
        node(
            "edit.paired_or_matched",
            "checkbox",
            "Matched / Repeated Measures",
            json!(ep.paired_or_matched),
            r1_paired_or_matched_allowed(doc, source_sheet_id),
            pos(2),
        ),
        node(
            "edit.sphericity.geisser_greenhouse",
            "checkbox",
            "Geisser-Greenhouse sphericity",
            json!(gg_checked),
            r1b_sphericity_correction(ep.paired_or_matched, reml_active),
            pos(3),
        ),
        node(
            "edit.post_hoc.tukey",
            "radio",
            "Tukey post-hoc",
            json!(matches!(method, curvebook_doc::MultiplicityMethod::TukeyHsd)),
            r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, curvebook_doc::MultiplicityMethod::TukeyHsd),
            pos(4),
        ),
        node(
            "edit.post_hoc.dunn",
            "radio",
            "Dunn's rank comparisons",
            json!(matches!(method, curvebook_doc::MultiplicityMethod::DunnRankSum)),
            r1b_post_hoc_option_enabled(ep.assume_gaussian, reml_active, curvebook_doc::MultiplicityMethod::DunnRankSum),
            pos(5),
        ),
        node(
            "edit.diag.residuals_table",
            "checkbox",
            "Emit Residuals & Standardized Residuals",
            json!(app.view_shards.stat_params.diag_residuals_plot),
            two_method.map_or_else(ControlEnablement::enabled, r4_two_group_diagnostics),
            pos(19),
        ),
        node(
            "edit.control_column",
            "combobox",
            "Control column",
            json!(ep.control_column),
            r3_control_column_picker(ep.compare_against_control, method),
            pos(6),
        ),
        node(
            "edit.two_group.graph_differences",
            "checkbox",
            "Graph differences",
            json!(ep.graph_differences),
            two_method.map_or_else(ControlEnablement::enabled, r4_two_group_graph_differences),
            pos(7),
        ),
        node(
            "edit.two_group.graph_ranks",
            "checkbox",
            "Graph ranks",
            json!(ep.graph_ranks),
            two_method.map_or_else(ControlEnablement::enabled, r4_two_group_nonparametric_options),
            pos(8),
        ),
        node(
            "edit.two_group.hodges_lehmann",
            "checkbox",
            "Hodges-Lehmann CI",
            json!(ep.hodges_lehmann_ci),
            two_method.map_or_else(ControlEnablement::enabled, r4_two_group_nonparametric_options),
            pos(9),
        ),
        node(
            "edit.two_group.welch",
            "checkbox",
            "Welch's correction",
            json!(ep.welch_correction),
            two_method.map_or_else(ControlEnablement::enabled, r4_two_group_welch_correction),
            pos(10),
        ),
        node(
            "edit.contingency.odds_ratio",
            "checkbox",
            "Odds ratio",
            json!(ep.contingency_odds_ratio_checked || cont_spec.compute_odds_and_risk_ratios),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(11),
        ),
        node(
            "edit.contingency.attributable_risk",
            "checkbox",
            "Attributable risk",
            json!(ep.contingency_attributable_risk_checked),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(12),
        ),
        node(
            "edit.contingency.sensitivity_specificity",
            "checkbox",
            "Sensitivity & Specificity",
            json!(cont_spec.compute_sensitivity_specificity),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(13),
        ),
        node(
            "edit.contingency.fisher",
            "radio",
            "Fisher's exact test",
            json!(cont_spec.method == curvebook_doc::ContingencyMethod::FisherExact),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(14),
        ),
        node(
            "edit.contingency.mcnemar",
            "radio",
            "McNemar's test",
            json!(cont_spec.method == curvebook_doc::ContingencyMethod::McNemarPaired),
            r5_contingency_2x2_effect_sizes(rows, cols),
            pos(15),
        ),
        node(
            "edit.contingency.cochran_armitage",
            "radio",
            "Cochran-Armitage trend test",
            json!(cont_spec.method == curvebook_doc::ContingencyMethod::CochranArmitageTrend),
            r5_contingency_cochran_armitage_trend(rows, cols),
            pos(16),
        ),
        node(
            "edit.contingency.ci.baptista_pike",
            "combobox",
            "OR CI (Baptista-Pike/Woolf)",
            json!(ep.contingency_or_ci_method),
            r6_contingency_ci_selector(rows, cols, ep.contingency_odds_ratio_checked || cont_spec.compute_odds_and_risk_ratios, "Baptista-Pike"),
            pos(17),
        ),
        node(
            "edit.contingency.ci.koopman",
            "combobox",
            "RR CI (Koopman)",
            json!(ep.contingency_rr_ci_method),
            r6_contingency_ci_selector(rows, cols, ep.contingency_relative_risk_checked || cont_spec.compute_odds_and_risk_ratios, "Koopman"),
            pos(18),
        ),
        node(
            "edit.contingency.ci.newcombe_wilson",
            "combobox",
            "AR CI (Newcombe-Wilson)",
            json!(ep.contingency_arr_ci_method),
            r6_contingency_ci_selector(rows, cols, ep.contingency_attributable_risk_checked, "Newcombe-Wilson"),
            pos(19),
        ),
        node(
            "edit.contingency.ci.wilson_brown",
            "combobox",
            "Sens/Spec CI (Wilson-Brown)",
            json!(ep.contingency_diag_ci_method),
            r6_contingency_ci_selector(rows, cols, cont_spec.compute_sensitivity_specificity, "Wilson-Brown"),
            pos(20),
        ),
        node(
            "edit.correlation.heatmap",
            "checkbox",
            "Create correlation heatmap",
            json!(ep.correlation_heatmap),
            r7_correlation_heatmap_toggle(ep.correlation_all_pairs),
            pos(21),
        ),
        node(
            "edit.descriptive.subcolumn_agg",
            "radiogroup",
            "Replicate subcolumn aggregation",
            json!(max_sub),
            r8_subcolumn_aggregation(max_sub),
            pos(22),
        ),
        node(
            "edit.pvalue.digits",
            "spinbutton",
            "Custom P-value decimals",
            json!(pvalue_digits_val(&pv_fmt)),
            r9_pvalue_precision_spinner(&pv_fmt),
            pos(23),
        ),
    ];
    if let Some(AnalysisKind::Transform(t)) = ep.draft.as_ref().map(|d| &d.kind) {
        match t {
            TransformAnalysisSpec::Math { x_transform, y_transform, error_propagation } => {
                controls.push(node(
                    "edit.transform.x_transform",
                    "combobox",
                    "X Transform",
                    json!(x_transform),
                    ControlEnablement::enabled(),
                    pos(24),
                ));
                controls.push(node(
                    "edit.transform.y_transform",
                    "combobox",
                    "Y Transform",
                    json!(y_transform),
                    ControlEnablement::enabled(),
                    pos(25),
                ));
                controls.push(node(
                    "edit.transform.error_propagation",
                    "combobox",
                    "Error propagation",
                    json!(error_propagation),
                    ControlEnablement::enabled(),
                    pos(26),
                ));
            }
            TransformAnalysisSpec::SelectAndTransform { filter_formula, target_archetype, .. } => {
                controls.push(node(
                    "edit.transform.formula",
                    "textbox",
                    "Row filter formula",
                    json!(filter_formula),
                    ControlEnablement::enabled(),
                    pos(24),
                ));
                controls.push(node(
                    "edit.transform.filter_formula",
                    "textbox",
                    "Filter formula",
                    json!(filter_formula),
                    ControlEnablement::enabled(),
                    pos(25),
                ));
                controls.push(node(
                    "edit.transform.target_archetype",
                    "combobox",
                    "Target archetype",
                    json!(target_archetype),
                    ControlEnablement::enabled(),
                    pos(26),
                ));
                controls.push(node(
                    "edit.transform.selected_data_sets",
                    "textbox",
                    "Selected data sets",
                    json!(ep.draft.as_ref().map(|d| &d.selected_data_sets)),
                    ControlEnablement::enabled(),
                    pos(27),
                ));
            }
            TransformAnalysisSpec::RemoveBaseline { baseline_columns, operation } => {
                controls.push(node(
                    "edit.transform.baseline_columns",
                    "textbox",
                    "Baseline columns",
                    json!(baseline_columns),
                    ControlEnablement::enabled(),
                    pos(24),
                ));
                controls.push(node(
                    "edit.transform.baseline_operation",
                    "combobox",
                    "Baseline operation",
                    json!(format!("{operation:?}")),
                    ControlEnablement::enabled(),
                    pos(25),
                ));
                controls.push(node(
                    "edit.transform.selected_data_sets",
                    "textbox",
                    "Selected data sets",
                    json!(ep.draft.as_ref().map(|d| &d.selected_data_sets)),
                    ControlEnablement::enabled(),
                    pos(26),
                ));
            }
            _ => {}
        }
    }
    push_r9_edit_params_controls(app, doc, &mut controls, &pos);

    (
        insp_tr(app, "w6.view.modal.edit_analysis_params", "Edit Analysis Parameters"),
        Some(result_sheet_id.as_str().to_owned()),
        Some(ep.active_tab.clone()),
        tabs,
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("apply", insp_tr(app, "btn.apply", "Apply")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn push_r9_edit_params_controls(app: &UiApp, doc: &ProjectDocument, controls: &mut Vec<UiWidgetNode>, pos: &impl Fn(usize) -> Option<[f64; 4]>) {
    let ep = &app.canvas_dialogs.edit_params;
    let Some(spec) = ep.draft.as_ref() else {
        return;
    };
    let mut row = controls.len() + 2;
    let mut push = |id: &str, role: &str, label: &str, value: Value| {
        let gate = if matches!(id, "edit.km.trend_test" | "edit.km.log_rank_for_trend") {
            evaluate_dialog_widget(&app.canvas_dialogs, doc, id)
        } else {
            ControlEnablement::enabled()
        };
        controls.push(node(id, role, label, value, gate, pos(row)));
        row += 1;
    };
    match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => {
            push("edit.two_group.paired_sign_test", "radio", "Paired Sign test", json!(t.method == TwoGroupMethod::PairedSignTest));
        }
        AnalysisKind::Contingency(c) => {
            let kappa_weight = match c.method {
                ContingencyMethod::CohenKappa { weighting } => Some(format!("{weighting:?}").to_ascii_lowercase()),
                _ => None,
            };
            push("edit.contingency.kappa_weighting", "combobox", "Cohen's Kappa weighting", json!(kappa_weight));
            push("edit.contingency.fleiss_kappa", "radio", "Fleiss's Kappa", json!(c.method == ContingencyMethod::FleissKappa));
            push("edit.contingency.exact_binomial", "radio", "Exact binomial", json!(c.method == ContingencyMethod::ExactBinomial));
            push("edit.contingency.barnard_exact", "radio", "Barnard's exact test", json!(c.method == ContingencyMethod::BarnardExact));
            push("edit.contingency.likelihood_g2", "radio", "Likelihood-ratio G2", json!(c.method == ContingencyMethod::LikelihoodRatioG2));
            push("edit.contingency.null_proportion", "spinbutton", "Null proportion", json!(c.null_proportion));
            push("edit.contingency.expected_proportions", "textbox", "Expected proportions", json!(c.expected_proportions));
        }
        AnalysisKind::FactorialAnova(f) => {
            push("edit.factorial.three_way", "checkbox", "Three-way ANOVA", json!(f.three_way));
            push("edit.factorial.repeated_measures", "combobox", "Repeated measures design", json!(format!("{:?}", f.repeated_measures)));
            push("edit.factorial.ss_type", "combobox", "Sum of squares", json!(format!("{:?}", f.ss_type)));
            push("edit.factorial.multiplicity", "combobox", "Factorial multiplicity", json!(format!("{:?}", f.multiplicity.method)));
            controls.push(node(
                "edit.factorial.residual_covariance",
                "combobox",
                "Residual covariance",
                json!(format!("{:?}", f.residual_covariance)),
                r1_sphericity_or_reml_control(f.repeated_measures != RepeatedMeasuresDesign::None),
                pos(row),
            ));
        }
        AnalysisKind::Multivariate(m) => match m {
            MultivariateSpec::Clustering { distance, linkage, k_means_k, k_sweep_max, .. } => {
                push("edit.cluster.distance", "combobox", "Cluster distance", json!(format!("{distance:?}")));
                push("edit.cluster.linkage", "combobox", "Cluster linkage", json!(format!("{linkage:?}")));
                push("edit.cluster.k_means_k", "spinbutton", "K-means k", json!(k_means_k));
                push("edit.cluster.k_sweep_max", "spinbutton", "K-means sweep max", json!(k_sweep_max));
            }
            MultivariateSpec::Pca { standardize, .. } => {
                push("edit.pca.standardize", "checkbox", "Standardize variables", json!(standardize));
            }
            MultivariateSpec::CorrelationMatrix { spearman_rank, .. } => {
                push("edit.correlation.spearman_rank", "checkbox", "Spearman rank correlation", json!(spearman_rank));
            }
            MultivariateSpec::MultipleImputation { imputations_m, .. } => {
                push("edit.imputation.m", "spinbutton", "Imputed datasets (m)", json!(imputations_m));
            }
        },
        AnalysisKind::GeneralizedLinearModel(g) => {
            push("edit.glm.family", "combobox", "GLM family", json!(format!("{:?}", g.family)));
            push("edit.glm.response_column", "combobox", "GLM response column", json!(g.response_column));
            push("edit.glm.predictor_columns", "textbox", "GLM predictor columns", json!(g.predictor_columns));
            push("edit.glm.compute_vif", "checkbox", "Compute VIF", json!(g.compute_vif));
        }
        AnalysisKind::Survival(s) => match s {
            SurvivalAnalysisSpec::CoxProportionalHazards { time_column, status_column, predictor_columns, tie_method, .. } => {
                push("edit.cox.time_column", "combobox", "Time column", json!(time_column));
                push("edit.cox.status_column", "combobox", "Status column", json!(status_column));
                push("edit.cox.predictor_columns", "textbox", "Predictor columns", json!(predictor_columns));
                push("edit.cox.tie_method", "combobox", "Cox tie method", json!(format!("{tie_method:?}")));
            }
            SurvivalAnalysisSpec::KaplanMeier {
                log_rank_mantel_cox,
                gehan_breslow_wilcoxon,
                trend_test,
                multiplicity,
                mv_time_column,
                mv_status_column,
                mv_group_column,
                ..
            } => {
                push("edit.km.log_rank_mantel_cox", "checkbox", "Log-rank (Mantel-Cox)", json!(log_rank_mantel_cox));
                push("edit.km.gehan_breslow_wilcoxon", "checkbox", "Gehan-Breslow-Wilcoxon", json!(gehan_breslow_wilcoxon));
                push("edit.km.trend_test", "checkbox", "Log-rank trend test", json!(trend_test));
                push("edit.km.log_rank_for_trend", "checkbox", "Log-rank test for trend", json!(trend_test));
                push("edit.km.multiplicity_method", "combobox", "Multiplicity correction", json!(format!("{:?}", multiplicity.method)));
                push("edit.km.mv_time_column", "combobox", "Time variable column", json!(mv_time_column));
                push("edit.km.mv_status_column", "combobox", "Event / status column", json!(mv_status_column));
                push("edit.km.mv_group_column", "combobox", "Group / strata column", json!(mv_group_column));
            }
        },
        AnalysisKind::NonlinearRegression(nl) => {
            push("edit.fit.model_id", "textbox", "Model equation ID", json!(nl.equation_id));
            push("edit.fit.method", "combobox", "Fit method", json!(format!("{:?}", nl.method)));
            push("edit.fit.weighting", "combobox", "Weighting scheme", json!(format!("{:?}", nl.weighting)));
            push("edit.fit.custom_dsl", "textbox", "Custom DSL equation", json!(nl.custom_dsl));
            push("edit.fit.parameter_rules_count", "readonly", "Parameter rules", json!(nl.parameter_rules.len()));
        }
        AnalysisKind::Transform(TransformAnalysisSpec::RandomSample { seed, sample_size, stratify_column, proportional_allocation }) => {
            push("edit.sample.sample_size", "spinbutton", "Sample size", json!(sample_size));
            push("edit.sample.seed", "spinbutton", "RNG seed", json!(seed));
            push("edit.sample.stratify_column", "combobox", "Stratify column", json!(stratify_column));
            push("edit.sample.proportional_allocation", "checkbox", "Proportional allocation", json!(proportional_allocation));
        }
        AnalysisKind::PValueStack(ps) => {
            push("edit.pvalue_stack.method", "combobox", "Multiplicity procedure", json!(format!("{:?}", ps.method)));
            push("edit.pvalue_stack.threshold", "spinbutton", "Alpha / FDR Q threshold", json!(ps.threshold));
            push("edit.pvalue_stack.column", "combobox", "P-value column", json!(ps.column));
        }
        AnalysisKind::PowerAnalysis(pa) => {
            push("edit.power.family", "combobox", "Power test family", json!(format!("{:?}", pa.family)));
            push("edit.power.solve_target", "combobox", "Solve target", json!(format!("{:?}", pa.solve_target)));
            push("edit.power.alpha", "spinbutton", "Alpha", json!(pa.alpha));
            push("edit.power.target_power", "spinbutton", "Target power", json!(pa.target_power));
            push("edit.power.sample_size", "spinbutton", "Sample size (n1)", json!(pa.sample_size));
            push("edit.power.sample_size_group2", "spinbutton", "Sample size (n2)", json!(pa.sample_size_group2));
            push("edit.power.allocation_ratio", "spinbutton", "Allocation ratio", json!(pa.allocation_ratio));
            push("edit.power.two_tailed", "checkbox", "Two-tailed", json!(matches!(pa.tail_mode, curvebook_doc::PowerTailMode::TwoSided)));
            push("edit.power.continuity_correction", "checkbox", "Continuity correction", json!(pa.continuity_correction));
            push("edit.power.chi_square_df", "spinbutton", "Chi-square df", json!(pa.chi_square_df));
        }
        _ => {}
    }
}

fn extract_edit_params_details(
    ep: &crate::dialogs_canvas::EditAnalysisParametersState,
) -> (curvebook_doc::MultiplicityMethod, PValueFormat, curvebook_doc::ContingencySpec, Option<curvebook_doc::TwoGroupMethod>) {
    let Some(spec) = ep.draft.as_ref() else {
        return (curvebook_doc::MultiplicityMethod::None, PValueFormat::FourDecimal, curvebook_doc::ContingencySpec::default(), None);
    };
    let mult = match &spec.kind {
        AnalysisKind::OneWayAnova(a) => a.multiplicity.method,
        AnalysisKind::FactorialAnova(f) => f.multiplicity.method,
        _ => curvebook_doc::MultiplicityMethod::None,
    };
    let cont = match &spec.kind {
        AnalysisKind::Contingency(c) => c.clone(),
        _ => curvebook_doc::ContingencySpec::default(),
    };
    let two = match &spec.kind {
        AnalysisKind::TwoGroupComparison(t) => Some(t.method),
        _ => None,
    };
    (mult, spec.p_value_format, cont, two)
}

const fn pvalue_digits_val(fmt: &PValueFormat) -> u8 {
    match fmt {
        PValueFormat::FourDecimal => 4,
        PValueFormat::Apa | PValueFormat::Nejm => 3,
        PValueFormat::Custom { decimals, .. } => *decimals,
    }
}

fn dialog_ctrl_bounds(_bx: f64, _by: f64, _row: usize, _include_bounds: bool) -> Option<[f64; 4]> {
    None
}

fn build_import_reshape_nodes(app: &UiApp, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.grid.import_reshape;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("import.mode", "combobox", "Import mode", json!(format!("{:?}", st.mode)), ControlEnablement::enabled(), pos(0)),
        node("import.title", "textbox", "Sheet title", json!(st.title), ControlEnablement::enabled(), pos(1)),
        node("import.target_kind", "combobox", "Target table kind", json!(format!("{:?}", st.target_kind)), ControlEnablement::enabled(), pos(2)),
        node("import.x_title", "textbox", "X title", json!(st.x_title), ControlEnablement::enabled(), pos(3)),
        node("import.subtract_blank_mean", "checkbox", "Subtract blank mean", json!(st.subtract_blank_mean), ControlEnablement::enabled(), pos(4)),
        node("import.plate_values_count", "spinbutton", "Plate values count", json!(st.plate_values.len()), ControlEnablement::enabled(), pos(5)),
        node("import.tidy_csv", "textbox", "Tidy CSV", json!(st.tidy_csv), ControlEnablement::enabled(), pos(6)),
        node("import.group_col", "textbox", "Group column", json!(st.group_col), ControlEnablement::enabled(), pos(7)),
        node("import.value_col", "textbox", "Value column", json!(st.value_col), ControlEnablement::enabled(), pos(8)),
        node("import.x_col", "textbox", "X column", json!(st.x_col), ControlEnablement::enabled(), pos(9)),
        node("import.replicate_col", "textbox", "Replicate column", json!(st.replicate_col), ControlEnablement::enabled(), pos(10)),
    ];
    (
        insp_tr(app, "w6.view.modal.import_reshape", "Import Reshape"),
        None,
        None,
        Vec::new(),
        controls,
        vec![("ok", "Import"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_convert_archetype_nodes(app: &UiApp, sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.convert_archetype;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("convert.target_kind", "combobox", "Target archetype", json!(format!("{:?}", st.target_kind)), ControlEnablement::enabled(), pos(0)),
        node("convert.allow_lossy", "checkbox", "Allow lossy conversion", json!(st.allow_lossy), ControlEnablement::enabled(), pos(1)),
        node("convert.preview_only", "checkbox", "Preview only", json!(st.preview_only), ControlEnablement::enabled(), pos(2)),
        node("convert.preview_ready", "label", "Preview ready", json!(st.preview_ready), ControlEnablement::enabled(), pos(3)),
        node("convert.preview_lossy", "label", "Preview is lossy", json!(st.preview_lossy), ControlEnablement::enabled(), pos(4)),
        node("convert.preserved_cells", "label", "Preserved cells", json!(st.preserved_cells), ControlEnablement::enabled(), pos(5)),
        node("convert.dropped_cells", "label", "Dropped cells", json!(st.dropped_cells), ControlEnablement::enabled(), pos(6)),
        node("convert.preserved_exclusions", "label", "Preserved exclusions", json!(st.preserved_exclusions), ControlEnablement::enabled(), pos(7)),
        node("convert.preview_warnings", "label", "Preview warnings", json!(st.preview_warnings), ControlEnablement::enabled(), pos(8)),
    ];
    (
        insp_tr(app, "cmd.table.convertArchetype", "Convert Table Archetype"),
        Some(sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", "Convert"), ("preview", "Preview"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_change_graph_type_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.change_graph_type;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("graph_type.selected", "listbox", "Selected graph type", json!(st.selected_option_id), ControlEnablement::enabled(), pos(0)),
        node("graph_type.summary_measure", "combobox", "Summary measure", json!(st.summary_measure), ControlEnablement::enabled(), pos(1)),
        node("graph_type.error_bar_metric", "combobox", "Error bar metric", json!(st.error_bar_metric), ControlEnablement::enabled(), pos(2)),
    ];
    (
        insp_tr(app, "cmd.graph.setKind", "Change Graph Type"),
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_format_graph_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.format_graph;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let eq_gate = if st.model_equation_mode.is_some() {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled("Graph has no ModelEquation annotation.")
    };
    let mv_cat_gate = if !st.mv_levels.is_empty() {
        ControlEnablement::enabled()
    } else {
        ControlEnablement::disabled(crate::i18n::tr(&app.locale, "w7.mv.format_graph_no_cat_levels"))
    };
    let sel_idx = crate::views::mv_formula::resolve_selected_level_idx(&st.mv_levels, st.mv_selected_level.as_deref());
    let sel_lvl = sel_idx.and_then(|i| st.mv_levels.get(i)).cloned().unwrap_or_default();
    let up_gate = if sel_idx.is_some_and(|i| i > 0) {
        ControlEnablement::enabled()
    } else if st.mv_levels.is_empty() {
        mv_cat_gate.clone()
    } else {
        ControlEnablement::disabled("Selected level is already at the top.")
    };
    let down_gate = if sel_idx.is_some_and(|i| i + 1 < st.mv_levels.len()) {
        ControlEnablement::enabled()
    } else if st.mv_levels.is_empty() {
        mv_cat_gate.clone()
    } else {
        ControlEnablement::disabled("Selected level is already at the end.")
    };
    let controls = vec![
        node("format_graph.graph_kind", "combobox", "Graph kind", json!(st.graph_kind), ControlEnablement::enabled(), pos(0)),
        node(
            "format_graph.grouped_arrangement",
            "combobox",
            "Grouped arrangement",
            json!(format!("{:?}", st.grouped_arrangement).to_ascii_lowercase()),
            ControlEnablement::enabled(),
            pos(1),
        ),
        node("format_graph.palette", "combobox", "Palette", json!(st.palette), ControlEnablement::enabled(), pos(2)),
        node("format_graph.frame_preset", "combobox", "Frame preset", json!(st.frame_preset), ControlEnablement::enabled(), pos(3)),
        node("format_graph.show_legend", "checkbox", "Show legend", json!(st.show_legend), ControlEnablement::enabled(), pos(4)),
        node("format_graph.legend_bordered", "checkbox", "Legend border", json!(st.legend_bordered), ControlEnablement::enabled(), pos(5)),
        node("format_graph.error_bar_enabled", "checkbox", "Show error bars", json!(st.error_bar_enabled), ControlEnablement::enabled(), pos(6)),
        node("format_graph.marker_size_pt", "spinbutton", "Marker size (pt)", json!(st.marker_size_pt), ControlEnablement::enabled(), pos(7)),
        node(
            "format_graph.half_violin_with_points",
            "checkbox",
            "Raincloud half-violin + points",
            json!(st.half_violin_with_points),
            ControlEnablement::enabled(),
            pos(8),
        ),
        node(
            "format_graph.model_equation_mode",
            "combobox",
            "Model equation mode",
            json!(st.model_equation_mode.map(equation_display_mode_str)),
            eq_gate.clone(),
            pos(9),
        ),
        node(
            "format_graph.toggle_model_equation_mode",
            "button",
            "Toggle equation display mode",
            json!(st.model_equation_mode.map(equation_display_mode_str)),
            eq_gate,
            pos(10),
        ),
        node("format_graph.level_order", "readonly", "Explicit level order", json!(st.mv_levels), mv_cat_gate.clone(), pos(11)),
        node("format_graph.selected_level", "combobox", "Selected categorical level", json!(sel_lvl), mv_cat_gate, pos(12)),
        node("format_graph.move_to_top", "button", crate::i18n::tr(&app.locale, "w7.mv.move_to_top"), json!(sel_lvl), up_gate.clone(), pos(13)),
        node("format_graph.move_up", "button", crate::i18n::tr(&app.locale, "w7.mv.move_up"), json!(sel_lvl), up_gate, pos(14)),
        node("format_graph.move_down", "button", crate::i18n::tr(&app.locale, "w7.mv.move_down"), json!(sel_lvl), down_gate.clone(), pos(15)),
        node("format_graph.move_to_end", "button", crate::i18n::tr(&app.locale, "w7.mv.move_to_end"), json!(sel_lvl), down_gate, pos(16)),
    ];
    (
        insp_tr(app, "inspector.format_graph", "Format Graph"),
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("apply", "Apply"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_format_axes_nodes(app: &UiApp, sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.format_axes;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let primary_seg = st.axis_draft.segments.first();
    let controls = vec![
        node("axes.target_axis", "combobox", "Target axis", json!(format!("{:?}", st.target_axis)), ControlEnablement::enabled(), pos(0)),
        node("axes.enable_y2", "checkbox", "Enable secondary Y2 axis", json!(st.enable_y2), ControlEnablement::enabled(), pos(1)),
        node("axes.title", "textbox", "Axis title", json!(st.axis_draft.title), ControlEnablement::enabled(), pos(2)),
        node("axes.scale", "combobox", "Axis scale", json!(primary_seg.map(|s| format!("{:?}", s.scale))), ControlEnablement::enabled(), pos(3)),
        node("axes.min", "spinbutton", "Minimum", json!(primary_seg.and_then(|s| s.min)), ControlEnablement::enabled(), pos(4)),
        node("axes.max", "spinbutton", "Maximum", json!(primary_seg.and_then(|s| s.max)), ControlEnablement::enabled(), pos(5)),
        node("axes.segment_count", "readonly", "Axis segments", json!(st.axis_draft.segments.len()), ControlEnablement::enabled(), pos(6)),
        node("axes.thresholds", "textbox", "Active axis thresholds", json!(st.axis_draft.thresholds), ControlEnablement::enabled(), pos(7)),
        node("axes.x_thresholds", "textbox", "X axis thresholds", json!(st.effective_x_thresholds()), ControlEnablement::enabled(), pos(8)),
        node("axes.y_thresholds", "textbox", "Y axis thresholds", json!(st.effective_y_thresholds()), ControlEnablement::enabled(), pos(9)),
    ];
    (
        insp_tr(app, "inspector.format_axes", "Format Axes"),
        Some(sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_pairwise_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.pairwise;
    let doc = app.document();
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let gate = |wid: &str| evaluate_dialog_widget(&app.canvas_dialogs, doc, wid);
    let controls = vec![
        node(
            "pairwise.result_sheet_id",
            "combobox",
            "Source ResultSheet",
            json!(st.selected_result_sheet_id.as_ref().map(SheetId::as_str)),
            ControlEnablement::enabled(),
            pos(0),
        ),
        node("pairwise.bracket_style", "combobox", "Bracket style", json!(format!("{:?}", st.bracket_style)), ControlEnablement::enabled(), pos(1)),
        node("pairwise.label_mode", "combobox", "Label mode", json!(format!("{:?}", st.label_mode)), ControlEnablement::enabled(), pos(2)),
        node(
            "pairwise.hide_non_significant",
            "checkbox",
            "Hide non-significant comparisons",
            json!(st.hide_non_significant),
            ControlEnablement::enabled(),
            pos(3),
        ),
        node("pairwise.bracket_count", "readonly", "Comparison brackets", json!(st.brackets.len()), ControlEnablement::enabled(), pos(4)),
        node("pairwise.auto_add", "button", "Auto-add brackets from analysis", json!(st.brackets.len()), ControlEnablement::enabled(), pos(5)),
        node(
            "pairwise.display_mode",
            "combobox",
            "Pairwise display mode",
            json!(pairwise_display_mode_str(st.display_mode)),
            ControlEnablement::enabled(),
            pos(6),
        ),
        node(
            "pairwise.cld_enabled",
            "checkbox",
            "Show Compact Letter Display (Piepho 2004)",
            json!(st.cld_enabled),
            ControlEnablement::enabled(),
            pos(7),
        ),
        node(
            "pairwise.cld_ordering",
            "combobox",
            "CLD letter ordering",
            json!(cld_ordering_str(st.cld_ordering)),
            gate("pairwise.cld_ordering"),
            pos(8),
        ),
        node("pairwise.cld_uppercase", "checkbox", "Uppercase CLD letters", json!(st.cld_uppercase), gate("pairwise.cld_uppercase"), pos(9)),
        node(
            "pairwise.cld_alpha_override",
            "spinbutton",
            "CLD alpha override",
            json!(st.cld_alpha_override),
            gate("pairwise.cld_alpha_override"),
            pos(10),
        ),
        node("pairwise.cld_font_size_pt", "spinbutton", "CLD font size (pt)", json!(st.cld_font_size_pt), gate("pairwise.cld_font_size_pt"), pos(11)),
        node("pairwise.cld_offset_pt", "spinbutton", "CLD vertical offset (pt)", json!(st.cld_offset_pt), gate("pairwise.cld_offset_pt"), pos(12)),
    ];
    (
        insp_tr(app, "inspector.pairwise_comparisons", "Pairwise Comparisons"),
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("apply", "Apply"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_bubble_plot_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.bubble_plot;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let (color_mode, color_col) = match &st.color_channel {
        ColorChannelMapping::Constant(_) => ("Constant", None),
        ColorChannelMapping::Categorical { column, .. } => ("Categorical", Some(*column)),
        ColorChannelMapping::Continuous { column, .. } => ("Continuous", Some(*column)),
    };
    let size_col = match st.size_channel {
        SizeChannelMapping::Constant { .. } => None,
        SizeChannelMapping::Variable { column, .. } => Some(column),
    };
    let avail: Vec<Value> = st.available_columns.iter().map(|c| json!({ "index": c.col_index, "title": c.title, "role": c.role })).collect();
    let controls = vec![
        node("bubble.plot_mode", "combobox", "Plot mode", json!(format!("{:?}", st.plot_mode)), ControlEnablement::enabled(), pos(0)),
        node("bubble.x_column", "combobox", "X / Category column", json!(st.x_column), ControlEnablement::enabled(), pos(1)),
        node("bubble.y_column", "combobox", "Y column", json!(st.y_column), ControlEnablement::enabled(), pos(2)),
        node("bubble.color_channel", "combobox", "Color channel", json!(st.color_channel), ControlEnablement::enabled(), pos(3)),
        node("bubble.size_channel", "combobox", "Size channel", json!(st.size_channel), ControlEnablement::enabled(), pos(4)),
        node("bubble.label_column", "combobox", "Label / Group column", json!(st.label_column), ControlEnablement::enabled(), pos(5)),
        node("bubble.show_convex_hulls", "checkbox", "Show convex hulls", json!(st.show_convex_hulls), ControlEnablement::enabled(), pos(6)),
        node(
            "bubble.show_confidence_ellipses",
            "checkbox",
            "Show confidence ellipses",
            json!(st.show_confidence_ellipses),
            ControlEnablement::enabled(),
            pos(7),
        ),
        node("bubble.color_mode", "combobox", "Color channel mode", json!(color_mode), ControlEnablement::enabled(), pos(8)),
        node("bubble.color_column", "combobox", "Color column", json!(color_col), ControlEnablement::enabled(), pos(9)),
        node("bubble.size_column", "combobox", "Size column", json!(size_col), ControlEnablement::enabled(), pos(10)),
        node("bubble.available_columns", "readonly", "Available columns", json!(avail), ControlEnablement::enabled(), pos(11)),
    ];
    (
        insp_tr(app, "w6.view.modal.bubble_plot_channels", "Bubble Plot Channels"),
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_apply_style_nodes(app: &UiApp, target_graph_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.apply_style;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node(
            "apply_style.donor_graph_id",
            "combobox",
            "Donor graph",
            json!(st.donor_graph_id.as_ref().map(SheetId::as_str)),
            ControlEnablement::enabled(),
            pos(0),
        ),
        node("apply_style.clone_palette", "checkbox", "Clone palette", json!(st.clone_palette), ControlEnablement::enabled(), pos(1)),
        node(
            "apply_style.clone_frame_and_axes",
            "checkbox",
            "Clone frame & axes",
            json!(st.clone_frame_and_axes),
            ControlEnablement::enabled(),
            pos(2),
        ),
        node(
            "apply_style.clone_error_bars_and_legend",
            "checkbox",
            "Clone error bars & legend",
            json!(st.clone_error_bars_and_legend),
            ControlEnablement::enabled(),
            pos(3),
        ),
    ];
    (
        insp_tr(app, "cmd.ui.applyStyleFrom.open", "Apply Style From..."),
        Some(target_graph_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_page_layout_nodes(app: &UiApp, layout_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.page_layout;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("layout.preset", "combobox", "Journal width preset", json!(format!("{:?}", st.preset)), ControlEnablement::enabled(), pos(0)),
        node("layout.orientation", "combobox", "Orientation", json!(format!("{:?}", st.orientation)), ControlEnablement::enabled(), pos(1)),
        node("layout.rows", "spinbutton", "Grid rows", json!(st.rows), ControlEnablement::enabled(), pos(2)),
        node("layout.cols", "spinbutton", "Grid columns", json!(st.cols), ControlEnablement::enabled(), pos(3)),
        node("layout.align_plot_origins", "checkbox", "Align plot origins", json!(st.align_plot_origins), ControlEnablement::enabled(), pos(4)),
        node("layout.panel_badges", "combobox", "Panel badges", json!(format!("{:?}", st.panel_badges)), ControlEnablement::enabled(), pos(5)),
        node("layout.item_count", "readonly", "Placed items", json!(st.items.len()), ControlEnablement::enabled(), pos(6)),
    ];
    (
        insp_tr(app, "w6.view.modal.page_layout_composer", "Page Layout Composer"),
        Some(layout_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_export_figure_nodes(app: &UiApp, sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.export_figure;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node("export.format", "combobox", "Format", json!(format!("{:?}", st.format).to_ascii_lowercase()), ControlEnablement::enabled(), pos(0)),
        node("export.width_px", "spinbutton", "Width (px)", json!(st.width_px), ControlEnablement::enabled(), pos(1)),
        node("export.height_px", "spinbutton", "Height (px)", json!(st.height_px), ControlEnablement::enabled(), pos(2)),
        node("export.dpi", "spinbutton", "DPI", json!(st.dpi), ControlEnablement::enabled(), pos(3)),
    ];
    (
        insp_tr(app, "w6.view.modal.export_publication_figure", "Export Publication Figure"),
        Some(sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.export", "Export")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_randomize_groups_nodes(app: &UiApp, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.randomize_groups;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let groups_text: Vec<String> = st.groups.iter().map(|(label, w)| if *w == 1 { label.clone() } else { format!("{label}:{w}") }).collect();
    let controls = vec![
        node("randomize.groups", "textbox", "Treatment arms", json!(groups_text.join(", ")), ControlEnablement::enabled(), pos(0)),
        node("randomize.subject_count", "spinbutton", "Subject count", json!(st.subject_count), ControlEnablement::enabled(), pos(1)),
        node("randomize.scheme", "combobox", "Randomization scheme", json!(format!("{:?}", st.scheme)), ControlEnablement::enabled(), pos(2)),
        node("randomize.block_sizes", "textbox", "Block sizes", json!(st.block_sizes), ControlEnablement::enabled(), pos(3)),
        node("randomize.seed", "spinbutton", "RNG seed", json!(st.seed), ControlEnablement::enabled(), pos(4)),
        node("randomize.title", "textbox", "Sheet title", json!(st.title), ControlEnablement::enabled(), pos(5)),
        node(
            "randomize.info_sheet_id",
            "combobox",
            "ARRIVE Info sheet",
            json!(st.target_info_sheet_id.as_ref().map(SheetId::as_str)),
            ControlEnablement::enabled(),
            pos(6),
        ),
    ];
    (
        insp_tr(app, "w6.view.modal.randomize_groups", "Randomize Groups"),
        None,
        None,
        Vec::new(),
        controls,
        vec![("ok", "Generate"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_random_sample_rows_nodes(app: &UiApp, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.random_sample_rows;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node(
            "sample.source_sheet_id",
            "combobox",
            "Source sheet",
            json!(st.source_sheet_id.as_ref().map(SheetId::as_str)),
            ControlEnablement::enabled(),
            pos(0),
        ),
        node("sample.sample_size", "spinbutton", "Sample size", json!(st.sample_size), ControlEnablement::enabled(), pos(1)),
        node("sample.seed", "spinbutton", "RNG seed", json!(st.seed), ControlEnablement::enabled(), pos(2)),
        node("sample.stratify_column", "combobox", "Stratify column", json!(st.stratify_column), ControlEnablement::enabled(), pos(3)),
        node(
            "sample.proportional_allocation",
            "checkbox",
            "Proportional allocation",
            json!(st.proportional_allocation),
            ControlEnablement::enabled(),
            pos(4),
        ),
        node("sample.title", "textbox", "Sample sheet title", json!(st.title), ControlEnablement::enabled(), pos(5)),
    ];
    (
        insp_tr(app, "cmd.table.randomSampleRows", "Random Sample Rows"),
        st.source_sheet_id.as_ref().map(|id| id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", "Sample"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_simulate_distributions_nodes(app: &UiApp, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.simulate_distributions;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let first = st.columns.first();
    let controls = vec![
        node("simulate.n_rows", "spinbutton", "Rows (N)", json!(st.n_rows), ControlEnablement::enabled(), pos(0)),
        node("simulate.seed", "spinbutton", "RNG seed", json!(st.seed), ControlEnablement::enabled(), pos(1)),
        node(
            "simulate.use_multiple_variables",
            "checkbox",
            "Create MultipleVariables table",
            json!(st.use_multiple_variables),
            ControlEnablement::enabled(),
            pos(2),
        ),
        node("simulate.title", "textbox", "Sheet title", json!(st.title), ControlEnablement::enabled(), pos(3)),
        node(
            "simulate.distribution",
            "combobox",
            "Distribution",
            json!(first.map(|c| c.distribution.as_str())),
            ControlEnablement::enabled(),
            pos(4),
        ),
        node("simulate.column_name", "textbox", "Primary column name", json!(first.map(|c| c.name.as_str())), ControlEnablement::enabled(), pos(5)),
        node("simulate.columns", "readonly", "Configured columns", json!(st.columns.len()), ControlEnablement::enabled(), pos(6)),
    ];
    (
        insp_tr(app, "cmd.table.simulateDistributions", "Simulate Distributions"),
        None,
        None,
        Vec::new(),
        controls,
        vec![("ok", "Simulate"), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_insert_model_equation_nodes(app: &UiApp, graph_sheet_id: &SheetId, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.insert_model_equation;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let controls = vec![
        node(
            "equation.result_sheet_id",
            "combobox",
            "Regression ResultSheet",
            json!(st.result_sheet_id.as_ref().map(SheetId::as_str)),
            ControlEnablement::enabled(),
            pos(0),
        ),
        node("equation.data_set_index", "spinbutton", "Data set index", json!(st.data_set_index), ControlEnablement::enabled(), pos(1)),
        node("equation.mode", "combobox", "Equation display mode", json!(equation_display_mode_str(st.mode)), ControlEnablement::enabled(), pos(2)),
        node(
            "equation.toggle_mode",
            "button",
            "Toggle Symbolic / Substituted",
            json!(equation_display_mode_str(st.mode)),
            ControlEnablement::enabled(),
            pos(3),
        ),
        node("equation.font_size_pt", "spinbutton", "Font size (pt)", json!(st.font_size_pt), ControlEnablement::enabled(), pos(4)),
        node("equation.bordered", "checkbox", "Draw border", json!(st.bordered), ControlEnablement::enabled(), pos(5)),
        node("equation.update_existing", "checkbox", "Update existing equation", json!(st.update_existing), ControlEnablement::enabled(), pos(6)),
    ];
    (
        insp_tr(app, "w6.view.modal.insert_model_equation", "Insert Model Equation"),
        Some(graph_sheet_id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "w6.view.btn.insert", "Insert")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_print_dialog_nodes(app: &UiApp, sheet_id: Option<&SheetId>, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let st = &app.canvas_dialogs.print;
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let submit_gate = if app.is_gui_only() {
        ControlEnablement::disabled("Submitting print jobs to the OS queue is blocked under --gui-only.")
    } else {
        ControlEnablement::enabled()
    };
    let controls = vec![
        node("print.crop_marks", "checkbox", "Crop marks", json!(st.job.crop_marks), ControlEnablement::enabled(), pos(0)),
        node("print.bleed_pt", "spinbutton", "Bleed (pt)", json!(st.job.bleed_pt), ControlEnablement::enabled(), pos(1)),
        node("print.copies", "spinbutton", "Copies", json!(st.copies), ControlEnablement::enabled(), pos(2)),
        node("print.scale_percent", "spinbutton", "Scale (%)", json!(st.job.scale_percent), ControlEnablement::enabled(), pos(3)),
        node("print.selected_printer", "combobox", "Printer", json!(st.selected_printer), ControlEnablement::enabled(), pos(4)),
        node("print.submit_to_os", "checkbox", "Submit to OS print queue", json!(st.submit_to_os), submit_gate, pos(5)),
    ];
    (
        insp_tr(app, "toolbar.print", "Print"),
        sheet_id.map(|id| id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "toolbar.print", "Print")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn build_generic_dialog_nodes(app: &UiApp, dialog: &ActiveDialog, bx: f64, by: f64, include_bounds: bool) -> DialogBuildParts {
    let pos = |row: usize| dialog_ctrl_bounds(bx, by, row, include_bounds);
    let mut controls = Vec::new();
    let (title, target) = match dialog {
        ActiveDialog::DuplicateAnalysisPrompt { source_sheet_id, .. } => {
            controls.push(node("duplicate.choice", "combobox", "Duplicate handling", json!("create_new"), ControlEnablement::enabled(), pos(0)));
            (insp_tr(app, "w6.view.modal.analysis_already_exists", "Analysis Already Exists"), Some(source_sheet_id))
        }
        ActiveDialog::ChangeDataAnalyzed { result_sheet_id } => {
            let st = &app.canvas_dialogs.change_data;
            controls.push(node(
                "change_data.target_sheet_id",
                "combobox",
                "Target Data Sheet",
                json!(st.selected_target_id.as_ref().map(SheetId::as_str)),
                ControlEnablement::enabled(),
                pos(0),
            ));
            controls.push(node(
                "change_data.selected_data_sets",
                "textbox",
                "Selected Data Sets",
                json!(st.selected_data_sets),
                ControlEnablement::enabled(),
                pos(1),
            ));
            (insp_tr(app, "btn.change_data_analyzed", "Change Data Analyzed"), Some(result_sheet_id))
        }
        ActiveDialog::ResultViewer { result_sheet_id } => (insp_tr(app, "w6.view.modal.result_viewer", "Result Viewer"), Some(result_sheet_id)),
        ActiveDialog::ConstantHookPicker { target_sheet_id } => {
            let st = &app.canvas_dialogs.hook_picker;
            controls.push(node(
                "hook.search_query",
                "textbox",
                "Search Constants / Results",
                json!(st.search_query),
                ControlEnablement::enabled(),
                pos(0),
            ));
            controls.push(node(
                "hook.selected_index",
                "spinbutton",
                "Selected Hook Index",
                json!(st.selected_index),
                ControlEnablement::enabled(),
                pos(1),
            ));
            (insp_tr(app, "w6.view.modal.hook_constant", "Hook Constant"), Some(target_sheet_id))
        }
        ActiveDialog::ChangeGraphType { graph_sheet_id } => (insp_tr(app, "cmd.graph.setKind", "Change Graph Type"), Some(graph_sheet_id)),
        ActiveDialog::FormatGraph { graph_sheet_id } => (insp_tr(app, "inspector.format_graph", "Format Graph"), Some(graph_sheet_id)),
        ActiveDialog::FormatAxes { sheet_id } => (insp_tr(app, "inspector.format_axes", "Format Axes"), Some(sheet_id)),
        ActiveDialog::PairwiseComparisons { graph_sheet_id } => {
            (insp_tr(app, "inspector.pairwise_comparisons", "Pairwise Comparisons"), Some(graph_sheet_id))
        }
        ActiveDialog::BubblePlotInspector { graph_sheet_id } => {
            (insp_tr(app, "w6.view.modal.bubble_plot_channels", "Bubble Plot Channels"), Some(graph_sheet_id))
        }
        ActiveDialog::ApplyStyleFrom { target_graph_id } => {
            (insp_tr(app, "cmd.ui.applyStyleFrom.open", "Apply Style From..."), Some(target_graph_id))
        }
        ActiveDialog::PageLayoutComposer { layout_sheet_id } => {
            (insp_tr(app, "w6.view.modal.page_layout_composer", "Page Layout Composer"), Some(layout_sheet_id))
        }
        ActiveDialog::ExportFigure { sheet_id } => {
            (insp_tr(app, "w6.view.modal.export_publication_figure", "Export Publication Figure"), Some(sheet_id))
        }
        ActiveDialog::ConvertArchetype { sheet_id } => (insp_tr(app, "cmd.table.convertArchetype", "Convert Table Archetype"), Some(sheet_id)),
        ActiveDialog::ImportReshape => (insp_tr(app, "w6.view.modal.import_reshape", "Import Reshape"), None),
        ActiveDialog::ExcludeWithReason { sheet_id } => {
            let st = &app.canvas_dialogs.exclude_with_reason;
            controls.push(node("exclude.reason_tag", "combobox", "Exclusion Reason", json!(st.reason_tag), ControlEnablement::enabled(), pos(0)));
            controls.push(node("exclude.note", "textbox", "Audit Note", json!(st.note), ControlEnablement::enabled(), pos(1)));
            (insp_tr(app, "w6.view.modal.exclude_with_reason", "Exclude with Reason"), Some(sheet_id))
        }
        ActiveDialog::RandomizeGroups => (insp_tr(app, "w6.view.modal.randomize_groups", "Randomize Groups"), None),
        ActiveDialog::RandomSampleRows { .. } => (insp_tr(app, "cmd.table.randomSampleRows", "Random Sample Rows"), None),
        ActiveDialog::SimulateDistributions => (insp_tr(app, "cmd.table.simulateDistributions", "Simulate Distributions"), None),
        ActiveDialog::InsertModelEquation { graph_sheet_id } => {
            (insp_tr(app, "w6.view.modal.insert_model_equation", "Insert Model Equation"), Some(graph_sheet_id))
        }
        ActiveDialog::Print { sheet_id } => (insp_tr(app, "toolbar.print", "Print"), sheet_id.as_ref()),
        ActiveDialog::CommandPalette => {
            let st = &app.canvas_dialogs.command_palette;
            controls.push(node("palette.query", "textbox", "Search Commands", json!(st.query), ControlEnablement::enabled(), pos(0)));
            controls.push(node(
                "palette.selected_index",
                "spinbutton",
                "Selected Command Index",
                json!(st.selected_index),
                ControlEnablement::enabled(),
                pos(1),
            ));
            (insp_tr(app, "inspector.command_palette", "Command Palette (Cmd+K)"), None)
        }
        ActiveDialog::About => (insp_tr(app, "menu.help.about", "About CurveBook"), None),
        _ => ("", None),
    };
    (
        title,
        target.map(|id| id.as_str().to_owned()),
        None,
        Vec::new(),
        controls,
        vec![("ok", insp_tr(app, "btn.ok", "OK")), ("cancel", insp_tr(app, "btn.cancel", "Cancel"))],
    )
}

fn sheet_section_name(sec: curvebook_doc::SheetSection) -> String {
    match sec {
        curvebook_doc::SheetSection::Data => String::from("data"),
        curvebook_doc::SheetSection::Info => String::from("info"),
        curvebook_doc::SheetSection::Analyses => String::from("results"),
        curvebook_doc::SheetSection::Graphs => String::from("graphs"),
        curvebook_doc::SheetSection::Layouts => String::from("layouts"),
    }
}

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
        ActiveDialog::Print { .. } => String::from("print"),
        ActiveDialog::CommandPalette => String::from("command_palette"),
        ActiveDialog::About => String::from("about"),
    }
}

/// Inspect active grid table dimensions helper for quick external checks.
#[must_use]
pub fn table_dimensions_summary(arch: &TableArchetype) -> (usize, usize) {
    (arch.row_count(), arch.column_count())
}
